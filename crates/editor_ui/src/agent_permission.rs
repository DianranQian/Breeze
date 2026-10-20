//! 操作风险分级与审批。
//!
//! # 边界说明（必须如实）
//!
//! `dsh headless` 的真实命令行只有 `--json` / `--session-id`，**没有**逐次
//! 授权参数；DSH 自己工具执行的沙箱与审批由它的 profile 决定，微风不伪造、
//! 也不绕过。微风这一层能做的是：
//!
//! 1. 对**微风自己发起**的操作（构建、测试、回退文件、删除、打开工作区外
//!    路径、把任务交给 DSH）做风险分级与真实授权门禁 —— 未批准就不执行；
//! 2. 对要交给 DSH 的自然语言任务做危险命令模式检测，命中就**先审批**；
//! 3. 在界面上明确写出"DSH 内部工具执行仍受 DSH 自身配置约束"，
//!    不声称微风提供了强于实际的安全边界。
//!
//! # 为什么不是"全部弹窗"
//!
//! 只读检索和项目内修改是开发循环的常态，逐次弹窗会让 Agent 不可用。
//! 默认策略自动允许到「项目内修改」，但构建/测试、删除/覆盖、工作区外访问、
//! 破坏性命令必须显式批准。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::现在秒;

/// 操作风险等级（从低到高）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum 风险等级 {
    /// 只读检索：读文件、看 git 状态、搜索代码。
    #[default]
    只读检索,
    /// 项目内文件修改。
    项目内修改,
    /// 构建与测试（会执行项目里的构建脚本）。
    构建测试,
    /// 删除或覆盖文件。
    删除覆盖,
    /// 访问工作区外的路径。
    工作区外,
    /// 高风险/破坏性命令。
    破坏性命令,
}

impl 风险等级 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            风险等级::只读检索 => "agent_risk_readonly",
            风险等级::项目内修改 => "agent_risk_project_write",
            风险等级::构建测试 => "agent_risk_build",
            风险等级::删除覆盖 => "agent_risk_delete",
            风险等级::工作区外 => "agent_risk_outside",
            风险等级::破坏性命令 => "agent_risk_destructive",
        }
    }

    /// 全部等级（界面列策略用）。
    pub const 全部: [Self; 6] = [
        Self::只读检索,
        Self::项目内修改,
        Self::构建测试,
        Self::删除覆盖,
        Self::工作区外,
        Self::破坏性命令,
    ];
}

/// 权限策略。
///
/// 默认：自动执行到项目内修改；构建/测试需批准（它会执行项目里的
/// `build.rs` 等构建脚本）；工作区外与破坏性命令永远需要显式批准。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 权限策略 {
    /// 自动执行的风险上限（含）。
    pub 自动执行上限: 风险等级,
    /// 是否允许访问工作区外路径（即使批准也默认不允许）。
    pub 允许工作区外: bool,
    /// 是否允许破坏性命令（默认不允许，批准也不放行）。
    pub 允许破坏性命令: bool,
}

impl Default for 权限策略 {
    fn default() -> Self {
        Self {
            自动执行上限: 风险等级::项目内修改,
            允许工作区外: false,
            允许破坏性命令: false,
        }
    }
}

impl 权限策略 {
    /// 是否必须审批。
    pub fn 需要审批(&self, 风险: 风险等级) -> bool {
        match 风险 {
            风险等级::工作区外 => !self.允许工作区外,
            风险等级::破坏性命令 => !self.允许破坏性命令,
            _ => 风险 > self.自动执行上限,
        }
    }

    /// 即使批准也禁止执行的情形。
    pub fn 永久禁止(&self, 风险: 风险等级) -> bool {
        matches!(风险, 风险等级::破坏性命令) && !self.允许破坏性命令
    }
}

/// 待审批时记住"批准后要做什么"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 待批动作 {
    /// 把任务交给 DSH 执行。
    执行任务(String),
    /// 回退某个文件变更（项目相对路径）。
    回退变更(String),
    /// 启动真实构建。
    构建,
    /// 启动真实测试。
    测试,
    /// 打开工作区外路径。
    打开工作区外(String),
}

impl 待批动作 {
    /// 动作描述（写入审批记录，不美化）。
    pub fn 描述(&self) -> String {
        match self {
            Self::执行任务(指令) => format!("把任务交给 DSH：{指令}"),
            Self::回退变更(路径) => format!("回退文件变更：{路径}"),
            Self::构建 => "执行 cargo build".to_string(),
            Self::测试 => "执行 cargo test".to_string(),
            Self::打开工作区外(路径) => format!("打开工作区外路径：{路径}"),
        }
    }

    /// 风险等级。任务指令按危险命令检测提升等级。
    pub fn 风险(&self) -> 风险等级 {
        match self {
            Self::执行任务(指令) => {
                if 危险命令检测(指令).is_empty() {
                    风险等级::项目内修改
                } else {
                    风险等级::破坏性命令
                }
            }
            Self::回退变更(_) => 风险等级::删除覆盖,
            Self::构建 | Self::测试 => 风险等级::构建测试,
            Self::打开工作区外(_) => 风险等级::工作区外,
        }
    }
}

/// 审批状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 审批状态 {
    /// 等用户决定。
    #[default]
    待批准,
    /// 已允许（动作可以执行）。
    已允许,
    /// 已拒绝（动作不得执行）。
    已拒绝,
    /// 已过期（会话切换等导致动作不再适用）。
    已过期,
}

/// 一条审批请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 审批请求 {
    /// 标识。
    pub 标识: String,
    /// 风险等级。
    pub 风险: 风险等级,
    /// 动作描述。
    pub 描述: String,
    /// 命中危险模式的证据（没有则为空）。
    pub 证据: Vec<String>,
    /// 批准后要执行的动作。
    pub 动作: 待批动作,
    /// 状态。
    pub 状态: 审批状态,
    /// 创建时间。
    pub 创建时间: u64,
}

impl 审批请求 {
    /// 新建一条待批请求。
    pub fn 新建(动作: 待批动作) -> Self {
        let 风险 = 动作.风险();
        let 证据 = match &动作 {
            // 危险命令检测只对"交给 DSH 的自然语言任务"有意义。
            待批动作::执行任务(指令) => 危险命令检测(指令),
            _ => Vec::new(),
        };
        Self {
            标识: uuid::Uuid::new_v4().to_string(),
            风险,
            描述: 动作.描述(),
            证据,
            动作,
            状态: 审批状态::待批准,
            创建时间: 现在秒(),
        }
    }

    /// 是否还在等待决定。
    pub fn 待决(&self) -> bool {
        self.状态 == 审批状态::待批准
    }
}

/// 把请求加入队列；同一动作已经在等待时不重复添加。
pub fn 排队审批(队列: &mut Vec<审批请求>, 动作: 待批动作) -> String {
    if let Some(已有) = 队列.iter().find(|请求| 请求.待决() && 请求.动作 == 动作) {
        return 已有.标识.clone();
    }
    let 请求 = 审批请求::新建(动作);
    let 标识 = 请求.标识.clone();
    队列.push(请求);
    标识
}

/// 找出并标记一条审批决定。返回被允许的动作（拒绝时返回 `None`）。
pub fn 决定审批(
    队列: &mut [审批请求], 标识: &str, 允许: bool
) -> Option<待批动作> {
    let 请求 = 队列.iter_mut().find(|请求| 请求.标识 == 标识)?;
    if !请求.待决() {
        return None;
    }
    请求.状态 = if 允许 {
        审批状态::已允许
    } else {
        审批状态::已拒绝
    };
    if 允许 {
        Some(请求.动作.clone())
    } else {
        None
    }
}

/// 危险命令模式。命中的是"文本片段"，不是完整语义 —— 因此只作为
/// **提高风险等级并要求审批** 的依据，绝不据此声称任务一定危险或安全。
pub const 危险模式: &[(&str, &str)] = &[
    ("rm -rf", "递归强制删除"),
    ("rm -fr", "递归强制删除"),
    ("rmdir /s", "递归删除目录"),
    ("del /s", "递归删除文件"),
    ("del /f", "强制删除文件"),
    ("remove-item -recurse", "递归删除（PowerShell）"),
    ("remove-item -force", "强制删除（PowerShell）"),
    ("clear-content", "清空文件内容"),
    ("format ", "格式化磁盘"),
    ("mkfs", "格式化磁盘"),
    ("dd if=", "裸设备写入"),
    ("git reset --hard", "丢弃工作区改动"),
    ("git clean", "删除未跟踪文件"),
    ("git checkout --", "丢弃文件改动"),
    ("git restore", "丢弃文件改动"),
    ("reg delete", "删除注册表项"),
    ("shutdown", "关机"),
    ("takeown", "夺取文件所有权"),
    ("icacls", "修改文件权限"),
    ("> /dev/sd", "裸设备写入"),
    ("curl | sh", "远程脚本直接执行"),
    ("curl | bash", "远程脚本直接执行"),
    ("iwr | iex", "远程脚本直接执行"),
    ("删除全部", "删除全部内容"),
    ("清空仓库", "清空仓库"),
    ("chmod -r 777", "放开全部权限"),
];

/// 检测文本里的危险命令模式，返回 `模式 → 说明` 的证据。
pub fn 危险命令检测(文本: &str) -> Vec<String> {
    let 小写 = 文本.to_lowercase();
    危险模式
        .iter()
        .filter(|(模式, _)| 小写.contains(模式))
        .map(|(模式, 说明)| format!("{模式}（{说明}）"))
        .collect()
}

/// 路径是否位于工作区内。
///
/// 用"规范化的绝对路径前缀"判断，而不是字符串 `starts_with`：
/// `C:\proj` 与 `C:\proj-evil` 前缀相同但不是同一个目录。
pub fn 在工作区内(根: &Path, 路径: &Path) -> bool {
    let 根 = 规范路径(根);
    let 目标 = 规范路径(路径);
    if cfg!(windows) {
        let 根文本 = 根.to_string_lossy().to_lowercase();
        let 目标文本 = 目标.to_string_lossy().to_lowercase();
        let 根文本 = 根文本.trim_end_matches(['\\', '/']).to_string();
        目标文本 == 根文本 || 目标文本.starts_with(&format!("{根文本}\\"))
    } else {
        目标.starts_with(&根)
    }
}

/// 规范化路径：尽量 `canonicalize`；目标不存在时规范化最近的已存在祖先，
/// 再把剩余部分按词法拼回去。这样"准备新建的文件"也能判断是否在工作区内。
fn 规范路径(路径: &Path) -> PathBuf {
    let mut 现有 = 路径.to_path_buf();
    let mut 后缀: Vec<std::ffi::OsString> = Vec::new();
    for _ in 0..64 {
        if let Ok(规范) = 现有.canonicalize() {
            let mut 结果 = 规范;
            for 部分 in 后缀.iter().rev() {
                结果.push(部分);
            }
            return 结果;
        }
        match 现有.file_name() {
            Some(名) => {
                后缀.push(名.to_os_string());
                现有.pop();
            }
            None => return 路径.to_path_buf(),
        }
    }
    路径.to_path_buf()
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 默认策略只自动允许到项目内修改() {
        let 策略 = 权限策略::default();
        assert!(!策略.需要审批(风险等级::只读检索));
        assert!(!策略.需要审批(风险等级::项目内修改));
        assert!(策略.需要审批(风险等级::构建测试));
        assert!(策略.需要审批(风险等级::删除覆盖));
        assert!(策略.需要审批(风险等级::工作区外));
        assert!(策略.需要审批(风险等级::破坏性命令));
        assert!(策略.永久禁止(风险等级::破坏性命令));
    }

    #[test]
    fn 打开工作区外永远需要批准() {
        let 策略 = 权限策略 {
            允许工作区外: true,
            ..Default::default()
        };
        assert!(!策略.需要审批(风险等级::工作区外));
        assert!(策略.需要审批(风险等级::破坏性命令), "破坏性命令仍受限制");
    }

    #[test]
    fn 危险命令检测命中真实模式() {
        assert!(!危险命令检测("请执行 rm -rf target").is_empty());
        assert!(!危险命令检测("git reset --hard HEAD").is_empty());
        assert!(危险命令检测("把 src/main.rs 里的拼写改一下").is_empty());
        let 证据 = 危险命令检测("Remove-Item -Recurse -Force .");
        assert!(证据.iter().any(|项| 项.contains("递归删除")));
    }

    #[test]
    fn 危险任务动作会被提升等级() {
        let 动作 = 待批动作::执行任务("git reset --hard 然后重来".to_string());
        assert_eq!(动作.风险(), 风险等级::破坏性命令);
        let 普通 = 待批动作::执行任务("修复编译错误".to_string());
        assert_eq!(普通.风险(), 风险等级::项目内修改);
    }

    #[test]
    fn 审批排队去重并按决定返回动作() {
        let mut 队列 = Vec::new();
        let a = 排队审批(&mut 队列, 待批动作::构建);
        let b = 排队审批(&mut 队列, 待批动作::构建);
        assert_eq!(a, b, "同一动作不应重复排队");
        assert_eq!(队列.len(), 1);
        assert_eq!(决定审批(&mut 队列, &a, true), Some(待批动作::构建));
        assert_eq!(队列[0].状态, 审批状态::已允许);
        assert_eq!(决定审批(&mut 队列, &a, true), None, "已决定的不再返回");
    }

    #[test]
    fn 拒绝不会返回动作() {
        let mut 队列 = Vec::new();
        let id = 排队审批(&mut 队列, 待批动作::回退变更("src/a.rs".to_string()));
        assert_eq!(决定审批(&mut 队列, &id, false), None);
        assert_eq!(队列[0].状态, 审批状态::已拒绝);
    }

    #[test]
    fn 工作区判断不是字符串前缀() {
        let 根 = tempfile::tempdir().unwrap();
        let 里面 = 根.path().join("src").join("main.rs");
        assert!(
            在工作区内(根.path(), &里面),
            "不存在的子路径也应视为工作区内"
        );
        let 外面 = std::env::temp_dir();
        assert!(!在工作区内(根.path(), &外面));
    }
}
