//! 真实工作区操作：快照、指纹、差异、精确回退。
//!
//! # 为什么不用 `git restore` / `git checkout`
//!
//! 那两个命令的作用域是"整个路径或整个工作区"，很容易把用户在任务期间
//! 新写的改动一起冲掉。这里只做两件可核对的事：
//!
//! 1. **读** `git show HEAD:<path>` 拿到基线内容；
//! 2. **写** 指定文件（同目录临时文件 + 原子替换）。
//!
//! 写之前必须通过 [`editor_ui::agent_review::审查决策`]：文件在记录之后
//! 又被改动、或任务开始前就脏，都会被拒绝，保留现状。
//!
//! # 指纹
//!
//! 用 FNV-1a 64 位内容哈希（稳定、跨进程一致、无需额外依赖）。
//! 它的用途是**冲突检测**，不是密码学安全，因此注释里不把它说成 SHA。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use editor_ui::agent::{任务基线, 文件变更};
use editor_ui::agent_review::{
    填入差异, 审查决策, 审查动作, 建立基线, 归类变更, 快照条目
};

/// 一次快照的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct 快照 {
    /// `HEAD` 提交（空仓库为空串）。
    pub 头提交: String,
    /// 当前所有脏文件（含未跟踪）。
    pub 条目: Vec<快照条目>,
}

/// 采集当前工作区快照。不是 git 仓库时返回真实错误。
pub fn 采集快照(根: &Path) -> Result<快照, String> {
    let 输出 = 运行git(
        根,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let 文本 = String::from_utf8_lossy(&输出.stdout);
    let 条目 = 解析状态输出(&文本, 根);
    let 头提交 = 运行git(根, &["rev-parse", "HEAD"])
        .map(|输出| String::from_utf8_lossy(&输出.stdout).trim().to_string())
        .unwrap_or_default();
    Ok(快照 { 头提交, 条目 })
}

/// 采集任务基线。
pub fn 采集基线(根: &Path, 任务标识: &str) -> Result<任务基线, String> {
    let 快照 = 采集快照(根)?;
    Ok(建立基线(任务标识, 快照.头提交, &快照.条目))
}

/// 解析 `git status --porcelain=v1 -z` 的输出。
///
/// `-z` 格式里重命名是 `XY 新路径\0原路径\0`（已用真实 git 核对）。
pub fn 解析状态输出(文本: &str, 根: &Path) -> Vec<快照条目> {
    let mut 结果 = Vec::new();
    let mut 部分 = 文本.split('\0');
    while let Some(项) = 部分.next() {
        if 项.len() < 4 {
            continue;
        }
        let 状态 = 项[..2].to_string();
        let 路径 = 项[3..].to_string();
        if 路径.is_empty() {
            continue;
        }
        let 是重命名 = 状态.contains('R') || 状态.contains('C');
        let 原路径 = if 是重命名 {
            部分.next().map(|原| 原.to_string())
        } else {
            None
        };
        let 指纹 = 文件指纹(根, &路径).unwrap_or_default();
        结果.push(快照条目 {
            路径,
            原路径,
            状态,
            指纹,
        });
        if 结果.len() >= 4000 {
            break;
        }
    }
    结果
}

/// 单个文件的内容指纹。文件不存在时返回 `None`。
pub fn 文件指纹(根: &Path, 相对路径: &str) -> Option<String> {
    let 路径 = 安全拼接(根, 相对路径)?;
    let 元数据 = std::fs::metadata(&路径).ok()?;
    if !元数据.is_file() {
        return None;
    }
    // 超大文件只哈希前 4 MiB，并在指纹里带上原始长度：足以检测"是否变了"，
    // 又不会在误把 target/ 之类的目录纳入时卡住界面。
    const 上限: u64 = 4 * 1024 * 1024;
    let 读取长度 = 元数据.len().min(上限) as usize;
    let bytes = std::fs::read(&路径).ok()?;
    let 切片 = &bytes[..bytes.len().min(读取长度)];
    Some(format!("fnv1a:{:016x}:{}", fnv1a(切片), 元数据.len()))
}

fn fnv1a(字节: &[u8]) -> u64 {
    let mut 值: u64 = 0xcbf29ce484222325;
    for 字节 in 字节 {
        值 ^= u64::from(*字节);
        值 = 值.wrapping_mul(0x100000001b3);
    }
    值
}

/// 内存内容的指纹，与 [`文件指纹`] 的格式一致（用于校验写回结果）。
fn 内容指纹(内容: &str) -> Option<String> {
    let 字节 = 内容.as_bytes();
    Some(format!("fnv1a:{:016x}:{}", fnv1a(字节), 字节.len()))
}

/// 把项目相对路径安全地拼到根目录下。
///
/// 拒绝绝对路径、盘符、`..` 与空路径 —— 审查操作只能碰工作区内文件。
pub fn 安全拼接(根: &Path, 相对路径: &str) -> Option<PathBuf> {
    let 相对 = Path::new(相对路径);
    if 相对.as_os_str().is_empty() || 相对.is_absolute() {
        return None;
    }
    for 组件 in 相对.components() {
        match 组件 {
            Component::Normal(_) => {}
            _ => return None,
        }
    }
    Some(根.join(相对))
}

/// 读取一个变更的真实差异。
///
/// * 已跟踪文件：`git diff HEAD -- <path>`（同时覆盖暂存与未暂存）；
/// * 未跟踪新文件：按真实内容生成 `+` 行；
/// * 删除：从 `HEAD` 读出内容生成 `-` 行；
/// * 重命名：把原路径与新路径一起交给 git（`-M`）。
pub fn 读取差异(根: &Path, 条目: &文件变更) -> String {
    let 差异 = if let Some(原路径) = &条目.原路径 {
        运行git(
            根,
            &[
                "diff",
                "HEAD",
                "-M",
                "--no-color",
                "--no-ext-diff",
                "--",
                原路径,
                &条目.路径,
            ],
        )
    } else if 条目.状态.trim() == "??" {
        Err("未跟踪文件没有 git diff".to_string())
    } else {
        运行git(
            根,
            &[
                "diff",
                "HEAD",
                "--no-color",
                "--no-ext-diff",
                "--",
                &条目.路径,
            ],
        )
    };
    let 文本 = match 差异 {
        Ok(输出) => String::from_utf8_lossy(&输出.stdout).to_string(),
        Err(_) => String::new(),
    };
    if !文本.trim().is_empty() {
        return editor_ui::agent_store::截断文本(&文本, 20000);
    }
    // 没有 diff：新增文件给真实内容，删除文件给 HEAD 内容。
    if (条目.状态.trim() == "??" || 条目.种类 == editor_ui::agent::变更种类::新增)
        && let Some(内容) = 读取工作区文件(根, &条目.路径, 400)
    {
        return format!(
            "(未跟踪/新增文件，以下为真实内容前 400 行)\n{}",
            合成差异(&内容, '+')
        );
    }
    if 条目.种类 == editor_ui::agent::变更种类::删除
        && let Some(内容) = 读取提交文件行数(根, &条目.路径, 400)
    {
        return format!(
            "(文件已删除，以下为 HEAD 中的真实内容)\n{}",
            合成差异(&内容, '-')
        );
    }
    "(没有可显示的差异)".to_string()
}

/// 读取工作区文件（最多 `行数` 行）。
pub fn 读取工作区文件(根: &Path, 相对路径: &str, 行数: usize) -> Option<String> {
    let 路径 = 安全拼接(根, 相对路径)?;
    let 文本 = std::fs::read_to_string(路径).ok()?;
    Some(截断行(&文本, 行数))
}

/// 读取 `HEAD` 中的文件内容。
pub fn 读取提交文件(根: &Path, 相对路径: &str) -> Option<String> {
    读取提交文件行数(根, 相对路径, usize::MAX)
}

fn 读取提交文件行数(根: &Path, 相对路径: &str, 行数: usize) -> Option<String> {
    let 规格 = format!("HEAD:{相对路径}");
    let 输出 = 运行git(根, &["show", &规格]).ok()?;
    if !输出.status.success() {
        return None;
    }
    let 文本 = String::from_utf8_lossy(&输出.stdout).to_string();
    if 行数 == usize::MAX {
        // 完整读取：必须保留原始换行与结尾，否则回退会改变文件字节。
        return Some(文本);
    }
    Some(截断行(&文本, 行数))
}

fn 截断行(文本: &str, 行数: usize) -> String {
    文本.lines().take(行数).collect::<Vec<_>>().join("\n")
}

fn 合成差异(内容: &str, 前缀: char) -> String {
    内容
        .lines()
        .map(|行| format!("{前缀}{行}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 刷新审查列表：真实快照 → 归属 → 差异。
pub fn 刷新变更(
    根: &Path,
    基线: Option<&任务基线>,
    写路径: &[String],
    任务标识: &str,
    旧: &[文件变更],
) -> Result<Vec<文件变更>, String> {
    let 快照 = 采集快照(根)?;
    let mut 变更 = 归类变更(基线, &快照.条目, 写路径, 任务标识, 旧);
    填入差异(&mut 变更, |项| 读取差异(根, 项));
    Ok(变更)
}

/// 验证时的文件指纹表（用于把验证结论固定到具体内容）。
pub fn 指纹表(根: &Path, 变更: &[文件变更]) -> BTreeMap<String, String> {
    let mut 表 = BTreeMap::new();
    for 项 in 变更 {
        if let Some(指纹) = 文件指纹(根, &项.路径) {
            表.insert(项.路径.clone(), 指纹);
        }
    }
    表
}

/// 回退结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 回退结果 {
    /// 实际执行的动作描述。
    pub 动作: String,
    /// 被恢复/删除的路径。
    pub 路径: String,
    /// 真实说明（含校验结果）。
    pub 说明: String,
}

/// 精确回退一个变更。
///
/// 安全检查顺序：审查决策 → 当前指纹复核 → 写盘 → 回读校验。
/// 任何一步失败都不会留下"半恢复"的假象：调用方把真实错误写进备注。
pub fn 回退变更(根: &Path, 变更: &文件变更) -> Result<回退结果, String> {
    let 当前指纹 = 文件指纹(根, &变更.路径);
    let 动作 = 审查决策(变更, 当前指纹.as_deref())?;
    match 动作 {
        审查动作::保留 => Err("审查动作是保留，不需要回退".to_string()),
        审查动作::删除新增文件 => {
            if 已被跟踪(根, &变更.路径) {
                return Err(
                    "该路径已被 git 跟踪，拒绝删除（只删除本任务新建且未跟踪的文件）".to_string(),
                );
            }
            let 路径 = 安全拼接(根, &变更.路径).ok_or_else(|| "非法路径".to_string())?;
            std::fs::remove_file(&路径)
                .map_err(|错误| format!("删除 {} 失败：{错误}", 变更.路径))?;
            Ok(回退结果 {
                动作: "删除新增文件".to_string(),
                路径: 变更.路径.clone(),
                说明: "文件是本任务新建且未被 git 跟踪，已删除".to_string(),
            })
        }
        审查动作::恢复文件 => {
            let 内容 = 读取提交文件(根, &变更.路径).ok_or_else(|| {
                format!(
                    "HEAD 中没有 {}：无法恢复到任务前状态（可能从未提交）",
                    变更.路径
                )
            })?;
            let 路径 = 安全拼接(根, &变更.路径).ok_or_else(|| "非法路径".to_string())?;
            原子写文件(&路径, 内容.as_bytes())?;
            let 新指纹 = 文件指纹(根, &变更.路径);
            // 基线干净时没有基线指纹：用"刚写回的 HEAD 内容"的指纹作为期望值，
            // 至少能证明磁盘内容确实等于 HEAD，而不是半截写入。
            let 期望 = 变更.基线指纹.clone().or_else(|| 内容指纹(&内容));
            if let Some(期望) = 期望
                && 新指纹.as_deref() != Some(期望.as_str())
            {
                return Err(format!(
                    "已写回 HEAD 内容，但回读指纹与基线不一致（期望 {}，实际 {}）",
                    期望,
                    新指纹.unwrap_or_else(|| "文件不存在".to_string())
                ));
            }
            Ok(回退结果 {
                动作: "从 HEAD 恢复".to_string(),
                路径: 变更.路径.clone(),
                说明: format!(
                    "已用 HEAD 内容原子写回；回读指纹 {}",
                    新指纹.unwrap_or_else(|| "无".to_string())
                ),
            })
        }
        审查动作::恢复重命名 { 原路径 } => {
            // 先删除新路径（只在未被跟踪时），再恢复原路径。
            if !已被跟踪(根, &变更.路径) {
                if let Some(新路径) = 安全拼接(根, &变更.路径)
                    && 新路径.exists()
                {
                    std::fs::remove_file(&新路径)
                        .map_err(|错误| format!("删除新路径失败：{错误}"))?;
                }
            } else {
                return Err("新路径已被 git 跟踪，拒绝删除".to_string());
            }
            let 内容 = 读取提交文件(根, &原路径)
                .ok_or_else(|| format!("HEAD 中没有原路径 {原路径}，无法恢复重命名"))?;
            let 目标 = 安全拼接(根, &原路径).ok_or_else(|| "原路径非法".to_string())?;
            原子写文件(&目标, 内容.as_bytes())?;
            Ok(回退结果 {
                动作: "恢复重命名".to_string(),
                路径: 变更.路径.clone(),
                说明: format!("已删除新路径并恢复原路径 {原路径}"),
            })
        }
    }
}

/// 路径是否被 git 跟踪（用于拒绝删除已跟踪文件）。
pub fn 已被跟踪(根: &Path, 相对路径: &str) -> bool {
    运行git(根, &["ls-files", "--error-unmatch", "--", 相对路径])
        .map(|输出| 输出.status.success())
        .unwrap_or(false)
}

/// 同目录临时文件 + `sync_all` + 原子替换。
pub fn 原子写文件(路径: &Path, 内容: &[u8]) -> Result<(), String> {
    if let Some(父) = 路径.parent() {
        std::fs::create_dir_all(父).map_err(|错误| format!("创建目录失败：{错误}"))?;
    }
    static 序号: AtomicU64 = AtomicU64::new(0);
    let 临时 = 路径.with_file_name(format!(
        ".breeze-write-{}-{}.tmp",
        std::process::id(),
        序号.fetch_add(1, Ordering::Relaxed)
    ));
    let 写结果 = (|| -> Result<(), String> {
        let mut 文件 = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&临时)
            .map_err(|错误| format!("创建临时文件失败：{错误}"))?;
        文件
            .write_all(内容)
            .map_err(|错误| format!("写入失败：{错误}"))?;
        文件
            .sync_all()
            .map_err(|错误| format!("同步失败：{错误}"))?;
        Ok(())
    })();
    if let Err(错误) = 写结果 {
        let _ = std::fs::remove_file(&临时);
        return Err(错误);
    }
    if let Err(错误) = std::fs::rename(&临时, 路径) {
        let _ = std::fs::remove_file(&临时);
        return Err(format!("原子替换失败：{错误}"));
    }
    Ok(())
}

fn 运行git(根: &Path, 参数: &[&str]) -> Result<std::process::Output, String> {
    let 输出 = std::process::Command::new("git")
        .args(参数)
        .current_dir(根)
        // 只读命令不要抢 index.lock，避免和用户自己的 git 操作打架。
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|错误| format!("git 启动失败（是否安装 Git？）：{错误}"))?;
    if !输出.status.success() {
        let 错误 = String::from_utf8_lossy(&输出.stderr);
        return Err(format!(
            "git {:?} 退出码 {:?}：{}",
            参数,
            输出.status.code(),
            editor_ui::agent_store::截断文本(错误.trim(), 400)
        ));
    }
    Ok(输出)
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_ui::agent::{变更来源, 变更种类, 审查状态, 文件变更};

    fn 临时仓库() -> tempfile::TempDir {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(根)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(根)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(根)
            .output()
            .unwrap();
        std::fs::create_dir_all(根.join("src")).unwrap();
        std::fs::write(根.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(根)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-qm", "init"])
            .current_dir(根)
            .output()
            .unwrap();
        目录
    }

    fn 变更项(路径: &str) -> 文件变更 {
        文件变更 {
            路径: 路径.to_string(),
            状态: " M".to_string(),
            种类: 变更种类::修改,
            来源: 变更来源::本任务,
            审查: 审查状态::待审查,
            可回退: true,
            ..Default::default()
        }
    }

    #[test]
    fn 快照能识别用户已有改动与未跟踪文件() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        std::fs::write(根.join("src/main.rs"), "fn main() { /* user */ }\n").unwrap();
        std::fs::write(根.join("new.txt"), "new\n").unwrap();
        let 快照 = 采集快照(根).unwrap();
        assert_eq!(快照.条目.len(), 2);
        assert!(快照.条目.iter().any(|项| 项.路径 == "src/main.rs"));
        assert!(
            快照
                .条目
                .iter()
                .any(|项| 项.路径 == "new.txt" && 项.状态 == "??")
        );
        assert!(!快照.头提交.is_empty());
    }

    #[test]
    fn 任务前已有改动不会被回退函数接受() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        std::fs::write(根.join("src/main.rs"), "fn main() { /* user */ }\n").unwrap();
        let 变更 = 变更项("src/main.rs");
        let 结果 = 回退变更(根, &变更);
        assert!(结果.is_err(), "没有基线指纹/可回退标记时不得动文件");
        assert_eq!(
            std::fs::read_to_string(根.join("src/main.rs")).unwrap(),
            "fn main() { /* user */ }\n",
            "拒绝回退时文件必须保持原样"
        );
    }

    #[test]
    fn 恢复文件只用提交内容且不动其他改动() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        // 用户旧改动
        std::fs::write(根.join("src/user.rs"), "用户自己写的\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "src/user.rs"])
            .current_dir(根)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-qm", "user"])
            .current_dir(根)
            .output()
            .unwrap();
        std::fs::write(根.join("src/user.rs"), "用户未提交的改动\n").unwrap();
        // Agent 改动 main.rs
        std::fs::write(根.join("src/main.rs"), "fn main() { broken }\n").unwrap();
        let mut 变更 = 变更项("src/main.rs");
        变更.当前指纹 = 文件指纹(根, "src/main.rs");
        let 结果 = 回退变更(根, &变更).unwrap();
        assert_eq!(
            std::fs::read_to_string(根.join("src/main.rs")).unwrap(),
            "fn main() {}\n",
            "本任务的改动应恢复为 HEAD 内容"
        );
        assert_eq!(
            std::fs::read_to_string(根.join("src/user.rs")).unwrap(),
            "用户未提交的改动\n",
            "用户已有改动不能被碰"
        );
        assert!(结果.说明.contains("回读指纹"));
    }

    #[test]
    fn 删除新增文件拒绝碰已跟踪路径() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        let mut 变更 = 变更项("src/main.rs");
        变更.种类 = 变更种类::新增;
        变更.状态 = "A ".to_string();
        变更.当前指纹 = 文件指纹(根, "src/main.rs");
        let 结果 = 回退变更(根, &变更);
        assert!(结果.is_err());
        assert!(根.join("src/main.rs").exists());
    }

    #[test]
    fn 未跟踪新文件可以被安全删除() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        std::fs::write(根.join("new.txt"), "temp\n").unwrap();
        let mut 变更 = 变更项("new.txt");
        变更.种类 = 变更种类::新增;
        变更.状态 = "??".to_string();
        变更.当前指纹 = 文件指纹(根, "new.txt");
        let 结果 = 回退变更(根, &变更).unwrap();
        assert_eq!(结果.动作, "删除新增文件");
        assert!(!根.join("new.txt").exists());
    }

    #[test]
    fn 差异读取给出真实内容() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        std::fs::write(根.join("src/main.rs"), "fn main() { changed }\n").unwrap();
        let 变更 = 变更项("src/main.rs");
        let 差异 = 读取差异(根, &变更);
        assert!(差异.contains("-fn main() {}"));
        assert!(差异.contains("+fn main() { changed }"));

        std::fs::write(根.join("brand_new.rs"), "fn new() {}\n").unwrap();
        let mut 新 = 变更项("brand_new.rs");
        新.种类 = 变更种类::新增;
        新.状态 = "??".to_string();
        let 差异 = 读取差异(根, &新);
        assert!(差异.contains("+fn new() {}"));
    }

    #[test]
    fn 路径安全拼接拒绝越界() {
        let 根 = Path::new("C:/proj");
        assert!(安全拼接(根, "../etc/passwd").is_none());
        assert!(安全拼接(根, "C:/Windows/system32").is_none());
        assert!(安全拼接(根, "").is_none());
        assert_eq!(
            安全拼接(根, "src/main.rs").unwrap(),
            PathBuf::from("C:/proj/src/main.rs")
        );
    }

    #[test]
    fn 指纹变化能被检测到() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        let 甲 = 文件指纹(根, "src/main.rs").unwrap();
        std::fs::write(根.join("src/main.rs"), "fn main() { 1 }\n").unwrap();
        let 乙 = 文件指纹(根, "src/main.rs").unwrap();
        assert_ne!(甲, 乙);
        assert!(甲.starts_with("fnv1a:"));
        assert_eq!(文件指纹(根, "不存在.rs"), None);
    }

    #[test]
    fn 回退后冲突检测仍然有效() {
        let 仓库 = 临时仓库();
        let 根 = 仓库.path();
        std::fs::write(根.join("src/main.rs"), "fn main() { one }\n").unwrap();
        let mut 变更 = 变更项("src/main.rs");
        变更.当前指纹 = 文件指纹(根, "src/main.rs");
        // 记录之后用户又改了一次
        std::fs::write(根.join("src/main.rs"), "fn main() { two }\n").unwrap();
        let 结果 = 回退变更(根, &变更);
        assert!(结果.is_err());
        assert!(结果.unwrap_err().contains("又被修改"));
        assert_eq!(
            std::fs::read_to_string(根.join("src/main.rs")).unwrap(),
            "fn main() { two }\n"
        );
    }
}
