//! 会话持久化的端到端测试：真实磁盘、真实重启、真实项目目录。
//!
//! 这些用例不依赖 Bevy 窗口，但走的是与应用完全相同的
//! `editor_ui::agent_store` 读写路径，因此能证明"退出后重启还能恢复"。

use std::path::Path;

use editor_ui::agent::{
    任务状态, 智能体任务, 智能体会话, 智能体状态, 消息角色, 现在秒
};
use editor_ui::agent_store::{
    原子保存, 存档文件名, 存档来源, 载入文件, 选择项目会话
};

fn 建目录(根: &Path, 名: &str) -> std::path::PathBuf {
    let 路径 = 根.join(名);
    std::fs::create_dir_all(&路径).unwrap();
    路径
}

fn 会话(标识: &str, 根: &Path, 文本: &str) -> 智能体会话 {
    let mut 会话 = 智能体会话::新建(标识, editor_ui::agent::规范化项目路径(根));
    会话.设置项目根(根);
    会话.追加消息(消息角色::用户, 文本);
    会话.追加消息(消息角色::智能体, format!("回复：{文本}"));
    会话.任务.push(智能体任务 {
        标识: format!("task-{标识}"),
        指令: 文本.to_string(),
        状态: 任务状态::成功,
        开始时间: 现在秒(),
        结束时间: Some(现在秒()),
        退出码: Some(0),
        ..Default::default()
    });
    会话
}

#[test]
fn 两个项目重启后各自恢复且不串线() {
    let 目录 = tempfile::tempdir().unwrap();
    let 项目甲 = 建目录(目录.path(), "项目甲");
    let 项目乙 = 建目录(目录.path(), "项目乙");
    let 存档 = 目录.path().join(存档文件名);

    let 甲会话 = 会话("s-甲", &项目甲, "修甲的构建");
    let 乙会话 = 会话("s-乙", &项目乙, "修乙的测试");
    原子保存(&存档, &[甲会话, 乙会话]).unwrap();

    // 模拟关闭微风后重启：只有磁盘上的文件。
    let 载入 = 载入文件(&存档);
    assert_eq!(载入.来源, 存档来源::文件);
    assert_eq!(载入.会话.len(), 2);

    let mut 状态 = 智能体状态 {
        会话列表: 载入.会话,
        ..Default::default()
    };
    assert_eq!(选择项目会话(&mut 状态, Some(&项目甲)), Some(0));
    assert_eq!(状态.当前().unwrap().标识, "s-甲");
    assert!(
        状态
            .当前()
            .unwrap()
            .消息
            .iter()
            .any(|消息| 消息.内容.contains("修甲的构建"))
    );
    assert!(
        !状态
            .当前()
            .unwrap()
            .消息
            .iter()
            .any(|消息| 消息.内容.contains("修乙")),
        "项目甲不能看到项目乙的消息"
    );

    assert_eq!(选择项目会话(&mut 状态, Some(&项目乙)), Some(1));
    assert_eq!(状态.当前().unwrap().标识, "s-乙");
    assert!(
        状态
            .当前()
            .unwrap()
            .任务
            .iter()
            .all(|任务| 任务.指令.contains("修乙"))
    );

    // 切回甲仍然正确。
    assert_eq!(选择项目会话(&mut 状态, Some(&项目甲)), Some(0));
    assert_eq!(状态.当前().unwrap().标识, "s-甲");
}

#[test]
fn 损坏存档会保留备份并从空列表开始() {
    let 目录 = tempfile::tempdir().unwrap();
    let 存档 = 目录.path().join(存档文件名);
    std::fs::write(&存档, "{ 这不是合法 JSON").unwrap();

    let 载入 = 载入文件(&存档);
    assert_eq!(载入.来源, 存档来源::已损坏);
    assert!(载入.会话.is_empty());
    assert!(!载入.禁止覆盖, "有备份后应允许重新保存");

    let 备份: Vec<_> = std::fs::read_dir(目录.path())
        .unwrap()
        .flatten()
        .filter(|项| 项.file_name().to_string_lossy().contains(".corrupt-"))
        .collect();
    assert_eq!(备份.len(), 1, "损坏的原文件必须保留");
    assert!(
        std::fs::read_to_string(备份[0].path())
            .unwrap()
            .contains("不是合法")
    );
}

#[test]
fn 未来版本的存档拒绝载入且禁止覆盖() {
    let 目录 = tempfile::tempdir().unwrap();
    let 存档 = 目录.path().join(存档文件名);
    std::fs::write(&存档, r#"{"格式版本":9999,"写入时间":0,"会话":[]}"#).unwrap();
    let 原文 = std::fs::read_to_string(&存档).unwrap();

    let 载入 = 载入文件(&存档);
    assert_eq!(载入.来源, 存档来源::版本不兼容);
    assert!(载入.禁止覆盖);
    assert_eq!(
        std::fs::read_to_string(&存档).unwrap(),
        原文,
        "不兼容时不能改动原文件"
    );
}

#[test]
fn 中断任务恢复成状态未知而不是成功() {
    let 目录 = tempfile::tempdir().unwrap();
    let 项目 = 建目录(目录.path(), "中断项目");
    let 存档 = 目录.path().join(存档文件名);
    let mut 会话 = 会话("s-中断", &项目, "跑一个长任务");
    会话.任务[0].状态 = 任务状态::执行中;
    会话.任务[0].结束时间 = None;
    会话.任务[0].退出码 = None;
    原子保存(&存档, &[会话]).unwrap();

    let 载入 = 载入文件(&存档);
    assert_eq!(载入.修正中断任务, 1);
    let 会话 = &载入.会话[0];
    assert_eq!(会话.任务[0].状态, 任务状态::状态未知);
    assert!(
        会话.任务[0].摘要.contains("中断"),
        "必须说明是中断：{}",
        会话.任务[0].摘要
    );
    assert!(会话.任务[0].退出码.is_none(), "没有退出码就不能编造");
}

#[test]
fn 项目被移动后历史保留但标记不可用() {
    let 目录 = tempfile::tempdir().unwrap();
    let 项目 = 建目录(目录.path(), "会移动的项目");
    let 存档 = 目录.path().join(存档文件名);
    原子保存(&存档, &[会话("s-移动", &项目, "历史消息")]).unwrap();

    // 真实地移动目录：会话文件里的路径随之失效。
    let 新位置 = 目录.path().join("移动后");
    std::fs::rename(&项目, &新位置).unwrap();

    let 载入 = 载入文件(&存档);
    assert_eq!(载入.会话.len(), 1, "历史必须保留");
    assert!(!载入.会话[0].项目可用);
    assert!(
        载入.会话[0]
            .项目状态说明
            .as_deref()
            .unwrap_or("")
            .contains("会移动的项目")
    );
    assert!(!载入.会话[0].消息.is_empty(), "消息不能被清空");
}

#[test]
fn 保存到磁盘时不写入凭证原文() {
    let 目录 = tempfile::tempdir().unwrap();
    let 项目 = 建目录(目录.path(), "凭证项目");
    let 存档 = 目录.path().join(存档文件名);
    let mut 会话 = 会话("s-凭证", &项目, "普通任务");
    会话.追加消息(
        消息角色::用户,
        "token 是 sk-abcdefghijklmnopqrstuvwxyz，别保存",
    );
    会话.任务[0].指令 = "api_key=supersecretvalue123".to_string();
    原子保存(&存档, &[会话]).unwrap();

    let 文本 = std::fs::read_to_string(&存档).unwrap();
    assert!(!文本.contains("sk-abcdefghijklmnopqrstuvwxyz"));
    assert!(!文本.contains("supersecretvalue123"));
    assert!(文本.contains("已脱敏"));
}

#[test]
fn 原子保存不会留下半截文件() {
    let 目录 = tempfile::tempdir().unwrap();
    let 项目 = 建目录(目录.path(), "原子项目");
    let 存档 = 目录.path().join(存档文件名);
    for i in 0..5 {
        let mut 会话 = 会话("s", &项目, &format!("第 {i} 次"));
        会话.标识 = format!("s-{i}");
        原子保存(&存档, &[会话]).unwrap();
        // 每次保存后都能立刻完整读回。
        let 载入 = 载入文件(&存档);
        assert_eq!(载入.会话[0].标识, format!("s-{i}"));
    }
    let 临时文件: Vec<_> = std::fs::read_dir(目录.path())
        .unwrap()
        .flatten()
        .filter(|项| 项.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(临时文件.is_empty());
}

/// 为 GUI 验收准备一个**由真实代码生成**的会话存档（默认不跑）。
///
/// 它会：
/// 1. 在 `.verify_v3/gui_project` 建真实 git 工程并留下两类改动：
///    用户任务前的旧改动、任务期间由写工具产生的改动；
/// 2. 用 `agent_worktree` 真实采集基线与差异；
/// 3. 把会话写进 `.verify_v3/gui_config/BevyStudio/agent_sessions.json`。
///
/// 这样 GUI 截屏里看到的差异、归属、指纹都不是手写 JSON 占位。
#[test]
#[ignore = "准备 GUI 验收数据，用 --ignored 手动运行"]
fn 生成图形界面验收会话() {
    use editor_app::agent_worktree::{刷新变更, 采集基线};
    use editor_ui::agent::{任务状态, 智能体任务};

    let 工程根 = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(".verify_v3/gui_project");
    let _ = std::fs::remove_dir_all(&工程根);
    std::fs::create_dir_all(工程根.join("src")).unwrap();
    std::fs::write(
        工程根.join("Cargo.toml"),
        "[package]\nname = \"gui_review_probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        工程根.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    std::fs::write(工程根.join("src/user.rs"), "// 用户模块\n").unwrap();
    for 参数 in [
        vec!["init", "-q"],
        vec!["config", "user.email", "gui@example.com"],
        vec!["config", "user.name", "gui"],
        vec!["add", "."],
        vec!["commit", "-qm", "init"],
    ] {
        let 输出 = std::process::Command::new("git")
            .args(&参数)
            .current_dir(&工程根)
            .output()
            .unwrap();
        assert!(
            输出.status.success(),
            "git {参数:?} 失败：{}",
            String::from_utf8_lossy(&输出.stderr)
        );
    }
    // 任务开始前用户已经改过 user.rs。
    std::fs::write(
        工程根.join("src/user.rs"),
        "// 用户未提交的改动（GUI 验收）\n",
    )
    .unwrap();
    let 基线 = 采集基线(&工程根, "gui-task").unwrap();
    // 任务期间 Agent 改了 main.rs。
    std::fs::write(
        工程根.join("src/main.rs"),
        "fn main() {\n    let 数量: u32 = \"这不是数字\";\n    println!(\"{数量}\");\n}\n",
    )
    .unwrap();
    let 变更 = 刷新变更(
        &工程根,
        Some(&基线),
        &["src/main.rs".to_string()],
        "gui-task",
        &[],
    )
    .unwrap();
    assert!(变更.iter().any(|项| 项.路径 == "src/main.rs"));
    assert!(变更.iter().any(|项| 项.路径 == "src/user.rs"));

    let mut 会话 = 会话("session-gui-review", &工程根, "修一下构建错误");
    会话.标题 = "GUI 真实审查会话".to_string();
    会话.基线 = Some(基线);
    会话.变更 = 变更;
    会话.任务.insert(
        0,
        智能体任务 {
            标识: "gui-task".to_string(),
            指令: "修一下构建错误".to_string(),
            状态: 任务状态::成功,
            开始时间: 现在秒(),
            结束时间: Some(现在秒()),
            退出码: Some(0),
            摘要: "任务完成（退出码 0）".to_string(),
            ..Default::default()
        },
    );
    let 配置目录 = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(".verify_v3/gui_config/BevyStudio");
    std::fs::create_dir_all(&配置目录).unwrap();
    原子保存(&配置目录.join(存档文件名), &[会话]).unwrap();
    eprintln!("GUI 验收数据已写入 {}", 配置目录.display());
}
