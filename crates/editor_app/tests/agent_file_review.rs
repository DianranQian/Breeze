//! 真实文件审查的端到端测试：真实 git 仓库、真实文件、真实回退。
//!
//! 重点验证**归属**与**冲突保护**：
//! * 任务开始前用户已有的改动不会被冒认成 Agent 成果；
//! * 拒绝只回退有可靠归属的改动；
//! * 文件在记录之后又被改动时，拒绝必须失败并保留现状。

use std::path::{Path, PathBuf};

use editor_app::agent_worktree::{
    刷新变更, 回退变更, 文件指纹, 采集基线, 采集快照
};
use editor_ui::agent::{变更来源, 变更种类, 审查状态, 文件变更};

struct 仓库 {
    _目录: tempfile::TempDir,
    根: PathBuf,
}

impl 仓库 {
    fn 新建() -> Self {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path().to_path_buf();
        git(&根, &["init", "-q"]);
        git(&根, &["config", "user.email", "review@example.com"]);
        git(&根, &["config", "user.name", "review"]);
        std::fs::create_dir_all(根.join("src")).unwrap();
        std::fs::write(根.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(根.join("src/user.rs"), "// 用户文件\n").unwrap();
        git(&根, &["add", "."]);
        git(&根, &["commit", "-qm", "init"]);
        Self {
            _目录: 目录, 根
        }
    }

    fn 写(&self, 相对: &str, 内容: &str) {
        let 路径 = self.根.join(相对);
        if let Some(父) = 路径.parent() {
            std::fs::create_dir_all(父).unwrap();
        }
        std::fs::write(路径, 内容).unwrap();
    }

    fn 读(&self, 相对: &str) -> String {
        std::fs::read_to_string(self.根.join(相对)).unwrap()
    }
}

fn git(根: &Path, 参数: &[&str]) {
    let 输出 = std::process::Command::new("git")
        .args(参数)
        .current_dir(根)
        .output()
        .expect("需要 git 可执行文件");
    assert!(
        输出.status.success(),
        "git {:?} 失败：{}",
        参数,
        String::from_utf8_lossy(&输出.stderr)
    );
}

fn 找<'a>(变更: &'a [文件变更], 路径: &str) -> &'a 文件变更 {
    变更
        .iter()
        .find(|项| 项.路径 == 路径)
        .unwrap_or_else(|| panic!("变更列表里没有 {路径}：{变更:?}"))
}

#[test]
fn 用户已有修改不会被认成智能体成果() {
    let 仓库 = 仓库::新建();
    // 任务开始前用户已经改了 user.rs。
    仓库.写("src/user.rs", "// 用户未提交的改动\n");
    let 基线 = 采集基线(&仓库.根, "t-1").expect("采集基线");

    // Agent 任务：改 main.rs。user.rs 没有被任何写工具记录。
    仓库.写("src/main.rs", "fn main() { broken }\n");
    let 变更 = 刷新变更(
        &仓库.根,
        Some(&基线),
        &["src/main.rs".to_string()],
        "t-1",
        &[],
    )
    .unwrap();

    let 用户的 = 找(&变更, "src/user.rs");
    assert_eq!(用户的.来源, 变更来源::任务前已有);
    assert_eq!(用户的.审查, 审查状态::已批准, "旧改动默认保持不动");
    assert!(!用户的.可回退);

    let 智能体的 = 找(&变更, "src/main.rs");
    assert_eq!(智能体的.来源, 变更来源::本任务);
    assert!(智能体的.可回退);

    let 结果 = 回退变更(&仓库.根, 智能体的).expect("有归有据的改动应能回退");
    assert_eq!(结果.路径, "src/main.rs");
    assert_eq!(
        仓库.读("src/main.rs"),
        "fn main() {}\n",
        "main.rs 必须恢复为 HEAD 内容"
    );
    assert_eq!(
        仓库.读("src/user.rs"),
        "// 用户未提交的改动\n",
        "用户的旧改动一个字都不能动"
    );
}

#[test]
fn 拒绝新建文件会删除而不会碰其他文件() {
    let 仓库 = 仓库::新建();
    let 基线 = 采集基线(&仓库.根, "t-2").unwrap();
    仓库.写("src/new_module.rs", "pub fn 新函数() {}\n");
    let 变更 = 刷新变更(
        &仓库.根,
        Some(&基线),
        &["src/new_module.rs".to_string()],
        "t-2",
        &[],
    )
    .unwrap();
    let 新文件 = 找(&变更, "src/new_module.rs");
    assert_eq!(新文件.种类, 变更种类::新增);
    assert_eq!(新文件.来源, 变更来源::本任务);

    let 结果 = 回退变更(&仓库.根, 新文件).unwrap();
    assert_eq!(结果.动作, "删除新增文件");
    assert!(!仓库.根.join("src/new_module.rs").exists());
    assert!(仓库.根.join("src/main.rs").exists(), "不能误删别的文件");
}

#[test]
fn 记录之后又被修改时拒绝会失败并保留现状() {
    let 仓库 = 仓库::新建();
    let 基线 = 采集基线(&仓库.根, "t-3").unwrap();
    仓库.写("src/main.rs", "fn main() { agent }\n");
    let 变更 = 刷新变更(
        &仓库.根,
        Some(&基线),
        &["src/main.rs".to_string()],
        "t-3",
        &[],
    )
    .unwrap();
    let 智能体的 = 找(&变更, "src/main.rs").clone();

    // 记录之后用户（或其他工作线）又改了同一个文件。
    仓库.写("src/main.rs", "fn main() { user later }\n");
    let 错误 = 回退变更(&仓库.根, &智能体的).expect_err("必须检测冲突");
    assert!(错误.contains("又被修改"));
    assert_eq!(
        仓库.读("src/main.rs"),
        "fn main() { user later }\n",
        "冲突时必须保留最新内容，不能覆盖"
    );
}

#[test]
fn 删除的文件能从提交版本精确恢复() {
    let 仓库 = 仓库::新建();
    let 基线 = 采集基线(&仓库.根, "t-4").unwrap();
    std::fs::remove_file(仓库.根.join("src/main.rs")).unwrap();
    let 变更 = 刷新变更(
        &仓库.根,
        Some(&基线),
        &["src/main.rs".to_string()],
        "t-4",
        &[],
    )
    .unwrap();
    let 删除 = 找(&变更, "src/main.rs");
    assert_eq!(删除.种类, 变更种类::删除);
    assert_eq!(删除.来源, 变更来源::本任务);

    let 结果 = 回退变更(&仓库.根, 删除).unwrap();
    assert_eq!(结果.动作, "从 HEAD 恢复");
    assert_eq!(仓库.读("src/main.rs"), "fn main() {}\n");
}

#[test]
fn 重命名被识别为暂存操作且不会冒险改索引() {
    let 仓库 = 仓库::新建();
    let 基线 = 采集基线(&仓库.根, "t-5").unwrap();
    git(&仓库.根, &["mv", "src/main.rs", "src/game.rs"]);
    let 变更 = 刷新变更(
        &仓库.根,
        Some(&基线),
        &["src/game.rs".to_string()],
        "t-5",
        &[],
    )
    .unwrap();
    let 重命名 = 找(&变更, "src/game.rs");
    assert_eq!(重命名.种类, 变更种类::重命名);
    assert_eq!(重命名.原路径.as_deref(), Some("src/main.rs"));
    assert!(
        !重命名.可回退,
        "git mv 已经改动暂存区：拒绝不能冒险动用户的 index"
    );
    assert!(
        重命名.备注.as_deref().unwrap_or("").contains("暂存区"),
        "必须如实说明限制：{:?}",
        重命名.备注
    );
    assert!(
        重命名.差异.contains("main.rs") && 重命名.差异.contains("game.rs"),
        "重命名的差异必须同时涉及两个路径：{}",
        重命名.差异
    );
    // 拒绝会真实失败并保留现状，而不是改一个标签。
    let 错误 = 回退变更(&仓库.根, 重命名).expect_err("暂存重命名不能自动回退");
    assert!(错误.contains("不能安全恢复") || 错误.contains("暂存区") || 错误.contains("暂存"));
    assert!(仓库.根.join("src/game.rs").exists());
    assert!(!仓库.根.join("src/main.rs").exists());
}

#[test]
fn 没有写证据的并发改动标为来源未知且不可回退() {
    let 仓库 = 仓库::新建();
    let 基线 = 采集基线(&仓库.根, "t-6").unwrap();
    // 模拟"用户在任务期间手改"，但没有任何 Agent 写工具记录。
    仓库.写("src/user.rs", "// 任务期间用户手改\n");
    let 变更 = 刷新变更(&仓库.根, Some(&基线), &[], "t-6", &[]).unwrap();
    let 未知 = 找(&变更, "src/user.rs");
    assert_eq!(未知.来源, 变更来源::来源未知);
    assert!(!未知.可回退);
    assert!(未知.备注.as_deref().unwrap_or("").contains("无法可靠归属"));
    assert!(回退变更(&仓库.根, 未知).is_err());
    assert_eq!(仓库.读("src/user.rs"), "// 任务期间用户手改\n");
}

#[test]
fn 非git项目不会伪造变更列表() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    std::fs::write(根.join("main.rs"), "fn main() {}\n").unwrap();
    assert!(采集快照(根).is_err(), "没有 .git 时不能假装有真实状态");
    assert!(采集基线(根, "t").is_err());
    assert!(文件指纹(根, "main.rs").is_some(), "指纹本身仍可用");
}

#[test]
fn 指纹在内容变化后立即不同() {
    let 仓库 = 仓库::新建();
    let 甲 = 文件指纹(&仓库.根, "src/main.rs").unwrap();
    仓库.写("src/main.rs", "fn main() { 1 }\n");
    let 乙 = 文件指纹(&仓库.根, "src/main.rs").unwrap();
    assert_ne!(甲, 乙);
}

#[test]
fn 基线里任务前干净的文件才有回退资格() {
    let 仓库 = 仓库::新建();
    仓库.写("src/user.rs", "// 任务前已有\n");
    let 基线 = 采集基线(&仓库.根, "t-7").unwrap();
    仓库.写("src/user.rs", "// 任务期间又改\n");
    仓库.写("src/main.rs", "fn main() { task }\n");
    let 变更 = 刷新变更(
        &仓库.根,
        Some(&基线),
        &["src/user.rs".to_string(), "src/main.rs".to_string()],
        "t-7",
        &[],
    )
    .unwrap();
    let 用户 = 找(&变更, "src/user.rs");
    assert_eq!(用户.来源, 变更来源::本任务, "有写证据就算本任务碰过");
    assert!(!用户.可回退, "任务前就脏，没有可安全恢复的内容");
    let 主 = 找(&变更, "src/main.rs");
    assert!(主.可回退, "任务前干净才能回到 HEAD");
}
