//! 运行入口与运行历史：把"跑的是哪个 Cargo 目标""这次跑成什么样"变成数据。
//!
//! # 为什么运行入口必须是一个显式值，而不是每次现场拼命令
//!
//! 一个 `cargo` 工程里同时存在多个可运行目标是完全正常的：
//!
//! ```text
//! my_game
//!   ├── src/main.rs        → bin my_game
//!   ├── src/bin/server.rs  → bin server
//!   └── [lib]              → 库（不能直接运行）
//! ```
//!
//! "运行"按钮如果只记住一个布尔量，用户就没法表达"这次我要跑 `server`"。
//! 因此运行入口是一个可序列化的值（包名 + 目标名 + profile + 参数），
//! 由界面提供选择，并**持久化到用户偏好**（见 `editor_app::config`）。
//!
//! # 失效入口怎么处理
//!
//! 用户把 `[[bin]] server` 删掉之后，上次记住的入口就失效了。
//! 这里不静默替换成别的目标（那会让用户以为自己在跑 `server`，
//! 实际跑的是 `my_game`），而是返回 [`入口失效原因`]，
//! 由调用方明确告知用户并回退到默认目标。
//!
//! # 运行记录为什么要单独存
//!
//! "构建成功"不等于"游戏启动成功"，"进程还在跑"也不等于"运行成功"。
//! 记录里同时保存**开始/结束时间、真实退出码、结束原因**，
//! 界面才能如实回答"上次那次到底怎么了"。

use serde::{Deserialize, Serialize};

use crate::build::构建配置类型;
use crate::cargo_project::Cargo工程;

/// 运行目标类型。
///
/// # 为什么这条信息必须进界面
///
/// 把"又开了一个编辑器窗口"当成"跑游戏"是产品层面最严重的误导。
/// 因此类型不是内部实现细节，而是要在状态栏/日志里写清楚的事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 运行目标类型 {
    /// 独立预览进程：再次启动微风并打开同一项目。
    ///
    /// 这是默认值：**没声明运行目标的项目不会被当成有游戏运行时**。
    #[default]
    独立预览进程,
    /// 项目声明的真实可执行程序。
    项目可执行程序,
    /// 原生 Cargo 工程里的可执行目标：通过 `cargo run -p <包> --bin <目标>` 启动。
    Cargo可执行目标,
}

impl 运行目标类型 {
    /// 证据 JSON 里的稳定代号（不随语言变化）。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::独立预览进程 => "preview-process",
            Self::项目可执行程序 => "game",
            Self::Cargo可执行目标 => "cargo-target",
        }
    }

    /// 是否**不**是真正的游戏运行时。
    pub fn 是预览进程(self) -> bool {
        matches!(self, Self::独立预览进程)
    }
}

/// 运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 项目运行状态 {
    /// 没有在运行。
    #[default]
    空闲,
    /// 正在启动（进程还没起来）。
    启动中,
    /// 子进程在运行。
    运行中,
    /// 已停止（主动停止或自然退出）。
    已停止,
    /// 启动失败或异常退出。
    失败,
}

impl 项目运行状态 {
    /// 是否在运行（含启动中）。
    pub fn 在运行(&self) -> bool {
        matches!(self, Self::启动中 | Self::运行中)
    }

    /// 中文状态文案（写进日志，不参与界面翻译）。
    pub fn 文案(&self) -> &'static str {
        match self {
            Self::空闲 => "空闲",
            Self::启动中 => "启动中",
            Self::运行中 => "运行中",
            Self::已停止 => "已停止",
            Self::失败 => "失败",
        }
    }

    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            Self::空闲 => "run_state_idle",
            Self::启动中 => "run_state_starting",
            Self::运行中 => "run_state_running",
            Self::已停止 => "run_state_stopped",
            Self::失败 => "run_state_failed",
        }
    }

    /// 稳定代号（证据 JSON 用）。
    pub fn 代号(&self) -> &'static str {
        match self {
            Self::空闲 => "idle",
            Self::启动中 => "starting",
            Self::运行中 => "running",
            Self::已停止 => "stopped",
            Self::失败 => "failed",
        }
    }
}

/// 界面每帧读取的运行快照。
///
/// 放在核心层而不是应用层，是因为**界面层也要读它**，
/// 而 `editor_ui` 不能依赖 `editor_app`（依赖方向是单向的）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 运行快照 {
    /// 状态。
    pub 状态: 项目运行状态,
    /// 目标类型。
    pub 目标类型: 运行目标类型,
    /// 当前（或最近）入口。
    pub 入口: Option<运行入口>,
    /// 命令行。
    pub 命令行: String,
    /// 进程号。
    pub 进程号: Option<u32>,
    /// 本次运行实例标识（调试会话关联用）。
    pub 运行实例标识: Option<String>,
    /// 真实退出码。
    pub 退出码: Option<i32>,
    /// 结束原因。
    pub 结束原因: 结束原因,
    /// 用时（毫秒）。
    pub 用时毫秒: u64,
    /// 最近错误。
    pub 最近错误: Option<String>,
    /// 本次运行的输出行。
    pub 日志: Vec<String>,
    /// 最近运行记录。
    pub 历史: Vec<运行记录>,
    /// 目标类型说明（本地化短语，明确"是不是游戏运行时"）。
    pub 目标说明: String,
}

impl 运行快照 {
    /// 是否在运行。
    pub fn 在运行(&self) -> bool {
        self.状态.在运行()
    }

    /// 最近一次已结束记录的结果文本。
    pub fn 结果文本(&self) -> String {
        match self.历史.iter().rev().find(|记录| 记录.已结束()) {
            Some(记录) => 记录.结果文本(),
            None => String::new(),
        }
    }
}

/// 一次运行的入口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 运行入口 {
    /// 所属包名（`cargo run -p <包名>`）。
    pub 包名: String,
    /// 目标名（`cargo run --bin <目标名>`）。
    pub 目标名: String,
    /// profile。
    #[serde(default)]
    pub 配置类型: 构建配置类型,
    /// 传给**程序本身**的参数（放在 `--` 之后）。
    #[serde(default)]
    pub 参数: Vec<String>,
}

impl Default for 运行入口 {
    fn default() -> Self {
        Self {
            包名: String::new(),
            目标名: String::new(),
            配置类型: 构建配置类型::Debug,
            参数: Vec::new(),
        }
    }
}

impl 运行入口 {
    /// 新建。
    pub fn 新建(包名: impl Into<String>, 目标名: impl Into<String>) -> Self {
        Self {
            包名: 包名.into(),
            目标名: 目标名.into(),
            ..Self::default()
        }
    }

    /// 供界面显示的一行标识：`包名::目标名`。
    pub fn 标识(&self) -> String {
        format!("{}::{}", self.包名, self.目标名)
    }

    /// 完整的 `cargo` 参数（不含 `cargo` 本身）。
    ///
    /// 顺序刻意固定：`run` → 包 → 目标 → profile → `--` → 程序参数。
    /// `--` 必须在最前之后的位置：放在 profile 之前会让 cargo 把
    /// `--release` 当成程序参数传给游戏。
    pub fn 命令行参数(&self) -> Vec<String> {
        let mut 参数 = vec![
            "run".to_string(),
            "-p".to_string(),
            self.包名.clone(),
            "--bin".to_string(),
            self.目标名.clone(),
        ];
        if let Some(profile) = self.配置类型.命令行参数() {
            参数.push(profile.to_string());
        }
        let 额外: Vec<String> = self
            .参数
            .iter()
            .map(|项| 项.trim())
            .filter(|项| !项.is_empty())
            .map(str::to_string)
            .collect();
        if !额外.is_empty() {
            参数.push("--".to_string());
            参数.extend(额外);
        }
        参数
    }

    /// 可读命令行。
    pub fn 可读命令行(&self) -> String {
        let mut 文本 = String::from("cargo");
        for 参数 in self.命令行参数() {
            文本.push(' ');
            文本.push_str(&参数);
        }
        文本
    }

    /// 校验这个入口在当前工程里是否仍然有效。
    pub fn 校验(&self, 工程: &Cargo工程) -> Result<(), 入口失效原因> {
        if self.包名.trim().is_empty() {
            return Err(入口失效原因::包名为空);
        }
        let Some(包) = 工程.包.iter().find(|包| 包.名称 == self.包名) else {
            return Err(入口失效原因::包不存在(self.包名.clone()));
        };
        if 包.可执行目标.is_empty() {
            return Err(入口失效原因::包没有可执行目标(
                包.名称.clone(),
            ));
        }
        if !包.可执行目标.iter().any(|目标| 目标.名称 == self.目标名) {
            return Err(入口失效原因::目标不存在 {
                包名: 包.名称.clone(),
                目标名: self.目标名.clone(),
            });
        }
        Ok(())
    }

    /// 从工程的默认目标推导入口。
    pub fn 默认(工程: &Cargo工程) -> Option<Self> {
        let 目标 = 工程.默认可执行目标()?;
        Some(Self::新建(目标.包名.clone(), 目标.名称.clone()))
    }

    /// 序列化成偏好字符串（失败时返回 `None`，调用方应忽略而不是报错）。
    pub fn 序列化(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    /// 从偏好字符串解析。
    pub fn 反序列化(文本: &str) -> Option<Self> {
        serde_json::from_str(文本).ok()
    }
}

/// 入口失效的原因。
///
/// 实现 `Display` + `Error` 是为了让它能作为 [`crate::project::项目错误`]
/// 的 `#[from]` 来源：调用方用 `?` 直接把失效原因变成项目错误，
/// 不必在每个调用点手工包装一遍。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 入口失效原因 {
    /// 包名为空（偏好损坏或手工编辑过）。
    包名为空,
    /// 包已不存在。
    包不存在(String),
    /// 包还在，但已经没有可执行目标（改成纯库了）。
    包没有可执行目标(String),
    /// 目标已不存在。
    目标不存在 {
        /// 包名。
        包名: String,
        /// 目标名。
        目标名: String,
    },
}

impl 入口失效原因 {
    /// 稳定代号。
    pub fn 代号(&self) -> &'static str {
        match self {
            Self::包名为空 => "empty-package",
            Self::包不存在(_) => "package-missing",
            Self::包没有可执行目标(_) => "package-lib-only",
            Self::目标不存在 { .. } => "target-missing",
        }
    }

    /// 面向用户的说明。**必须说明具体原因**，
    /// 只写"入口失效"等于什么都没说。
    ///
    /// 注意它**不**声称"已回到默认目标"：拒绝一个非法入口时并没有回退，
    /// 只有恢复旧入口失败时才有回退。那种情形用 [`Self::回退说明`]。
    pub fn 说明(&self) -> String {
        match self {
            Self::包名为空 => "运行入口为空：缺少包名或目标名".to_string(),
            Self::包不存在(包) => format!("包 `{包}` 不在 Cargo 工程里（可能被改名或删除）"),
            Self::包没有可执行目标(包) => {
                format!("包 `{包}` 只有库目标，不能作为运行入口（库不能直接运行）")
            }
            Self::目标不存在 { 包名, 目标名 } => {
                format!("包 `{包名}` 里没有名为 `{目标名}` 的可执行目标")
            }
        }
    }

    /// 回退说明：原因 + 已回到默认目标。
    ///
    /// 只在**真的发生了回退**的路径上使用（恢复旧入口失败、重新识别后发现
    /// 入口失效）。把它与 [`Self::说明`] 分开，是为了不在一句错误文案里
    /// 声称一件没发生的事。
    pub fn 回退说明(&self) -> String {
        format!("{}，已回到默认目标", self.说明())
    }
}

impl std::fmt::Display for 入口失效原因 {
    fn fmt(&self, 格式化器: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `说明` 已经是人读文本；这里不能再包一层"入口失效："，
        // 否则项目错误的 Display 会出现两层前缀。
        格式化器.write_str(&self.说明())
    }
}

impl std::error::Error for 入口失效原因 {}

/// 一次运行是怎么结束的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 结束原因 {
    /// 还没结束（正在启动或正在运行）。
    #[default]
    进行中,
    /// 程序自己以退出码 0 结束。
    正常退出,
    /// 程序自己以非零退出码结束（或崩溃）。
    异常退出,
    /// 用户点了「停止」。
    用户停止,
    /// 进程根本没起来（程序不存在、cargo 不在 PATH）。
    启动失败,
    /// 进程被系统信号终止，拿不到退出码。
    被终止,
}

impl 结束原因 {
    /// 是否已经结束。
    pub fn 已结束(self) -> bool {
        !matches!(self, Self::进行中)
    }

    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::进行中 => "running",
            Self::正常退出 => "exit-ok",
            Self::异常退出 => "exit-failed",
            Self::用户停止 => "stopped",
            Self::启动失败 => "spawn-failed",
            Self::被终止 => "terminated",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::进行中 => "run_end_running",
            Self::正常退出 => "run_end_ok",
            Self::异常退出 => "run_end_failed",
            Self::用户停止 => "run_end_stopped",
            Self::启动失败 => "run_end_spawn_failed",
            Self::被终止 => "run_end_terminated",
        }
    }
}

/// 一条运行记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 运行记录 {
    /// 序号（本次编辑器会话内从 1 开始）。
    pub 序号: u64,
    /// 运行入口（能确定时）。
    pub 入口: Option<运行入口>,
    /// 实际执行的命令行（真实字符串，不是重拼的）。
    pub 命令行: String,
    /// 开始时间（UNIX 秒）。
    pub 开始时间: u64,
    /// 结束时间（UNIX 秒，未结束为 `None`）。
    pub 结束时间: Option<u64>,
    /// 真实退出码。**被杀/拿不到时为 `None`**，绝不伪造成 0 或 1。
    pub 退出码: Option<i32>,
    /// 结束原因。
    pub 结束原因: 结束原因,
    /// 进程号。
    pub 进程号: Option<u32>,
    /// 本次运行实例标识。调试会话用它把"哪个运行"与"哪个调试连接"绑在一起。
    pub 运行实例标识: Option<String>,
    /// 保留的日志行数。
    pub 日志行数: usize,
    /// 结束时的说明（失败原因、退出码来源等）。
    pub 说明: Option<String>,
}

impl Default for 运行记录 {
    fn default() -> Self {
        Self {
            序号: 0,
            入口: None,
            命令行: String::new(),
            开始时间: 0,
            结束时间: None,
            退出码: None,
            结束原因: 结束原因::进行中,
            进程号: None,
            运行实例标识: None,
            日志行数: 0,
            说明: None,
        }
    }
}

impl 运行记录 {
    /// 是否还在进行。
    pub fn 进行中(&self) -> bool {
        !self.结束原因.已结束()
    }

    /// 是否已经结束。
    pub fn 已结束(&self) -> bool {
        self.结束原因.已结束()
    }

    /// 用时（秒）。未结束时用 `现在` 计算。
    pub fn 用时秒(&self, 现在: u64) -> u64 {
        let 结束 = self.结束时间.unwrap_or(现在);
        结束.saturating_sub(self.开始时间)
    }

    /// 一句话结果（真实数据拼接）。
    pub fn 结果文本(&self) -> String {
        match self.结束原因 {
            结束原因::进行中 => "运行中".to_string(),
            结束原因::正常退出 => "正常退出（退出码 0）".to_string(),
            结束原因::异常退出 => match self.退出码 {
                Some(码) => format!("异常退出（退出码 {码}）"),
                None => "异常退出（没有拿到退出码）".to_string(),
            },
            结束原因::用户停止 => "用户停止（不是程序自己退出）".to_string(),
            结束原因::启动失败 => match &self.说明 {
                Some(说明) => format!("启动失败：{说明}"),
                None => "启动失败".to_string(),
            },
            结束原因::被终止 => "进程被系统终止（无退出码）".to_string(),
        }
    }
}

/// 最近运行记录。
///
/// 有上限：一次开发可能运行几十上百次，全留着既没用也会让面板变得很长。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 运行历史 {
    记录: Vec<运行记录>,
    上限: usize,
}

/// 默认保留的运行记录条数。
pub const 默认运行历史上限: usize = 20;

impl Default for 运行历史 {
    fn default() -> Self {
        Self::新建(默认运行历史上限)
    }
}

impl 运行历史 {
    /// 以指定上限新建。
    pub fn 新建(上限: usize) -> Self {
        Self {
            记录: Vec::new(),
            上限: 上限.max(1),
        }
    }

    /// 全部记录（最旧在前）。
    pub fn 记录(&self) -> &[运行记录] {
        &self.记录
    }

    /// 最近一条记录。
    pub fn 最近(&self) -> Option<&运行记录> {
        self.记录.last()
    }

    /// 最近的、已经结束的一条记录。
    pub fn 最近结束(&self) -> Option<&运行记录> {
        self.记录.iter().rev().find(|记录| 记录.结束原因.已结束())
    }

    /// 开始一条新记录，返回它的序号。
    pub fn 开始(
        &mut self,
        命令行: impl Into<String>,
        入口: Option<运行入口>,
        进程号: Option<u32>,
        运行实例标识: Option<String>,
        开始时间: u64,
    ) -> u64 {
        let 序号 = self.记录.last().map(|记录| 记录.序号).unwrap_or(0) + 1;
        self.记录.push(运行记录 {
            序号,
            入口,
            命令行: 命令行.into(),
            开始时间,
            结束时间: None,
            退出码: None,
            结束原因: 结束原因::进行中,
            进程号,
            运行实例标识,
            日志行数: 0,
            说明: None,
        });
        while self.记录.len() > self.上限 {
            self.记录.remove(0);
        }
        序号
    }

    /// 取得正在进行的记录（可变）。
    pub fn 当前可变(&mut self) -> Option<&mut 运行记录> {
        self.记录.iter_mut().rev().find(|记录| 记录.进行中())
    }

    /// 结束当前记录。
    pub fn 结束当前(
        &mut self,
        原因: 结束原因,
        退出码: Option<i32>,
        结束时间: u64,
        说明: Option<String>,
        日志行数: usize,
    ) -> bool {
        match self.当前可变() {
            Some(记录) => {
                记录.结束原因 = 原因;
                记录.退出码 = 退出码;
                记录.结束时间 = Some(结束时间);
                记录.说明 = 说明;
                记录.日志行数 = 日志行数;
                true
            }
            None => false,
        }
    }

    /// 清空（换项目时调用）。
    pub fn 清空(&mut self) {
        self.记录.clear();
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 工程() -> Cargo工程 {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        std::fs::create_dir_all(根.join("src/bin")).unwrap();
        std::fs::write(
            根.join("Cargo.toml"),
            "[package]\nname = \"entry_probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(根.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(根.join("src/bin/server.rs"), "fn main() {}\n").unwrap();
        // 目录在函数结束时被删掉；工程只保存路径，之后不再读磁盘。
        Cargo工程::读取(根).unwrap()
    }

    #[test]
    fn 命令行参数顺序正确() {
        let mut 入口 = 运行入口::新建("my_game", "server");
        入口.配置类型 = 构建配置类型::Release;
        入口.参数 = vec!["--level".into(), "2".into()];
        assert_eq!(
            入口.命令行参数(),
            vec![
                "run",
                "-p",
                "my_game",
                "--bin",
                "server",
                "--release",
                "--",
                "--level",
                "2"
            ]
        );
        assert!(入口.可读命令行().starts_with("cargo run -p my_game"));
    }

    #[test]
    fn 空参数不会留下孤立的双横线() {
        let 入口 = 运行入口::新建("pkg", "bin");
        assert!(!入口.命令行参数().iter().any(|项| 项 == "--"));
    }

    #[test]
    fn 校验能发现失效目标() {
        let 工程 = 工程();
        let 有效 = 运行入口::新建("entry_probe", "server");
        assert!(有效.校验(&工程).is_ok());

        let 缺目标 = 运行入口::新建("entry_probe", "nope");
        assert_eq!(
            缺目标.校验(&工程),
            Err(入口失效原因::目标不存在 {
                包名: "entry_probe".into(),
                目标名: "nope".into()
            })
        );

        let 缺包 = 运行入口::新建("ghost", "server");
        assert_eq!(
            缺包.校验(&工程),
            Err(入口失效原因::包不存在("ghost".into()))
        );
        assert!(
            缺包
                .校验(&工程)
                .unwrap_err()
                .回退说明()
                .contains("已回到默认目标")
        );
        assert!(
            !缺包
                .校验(&工程)
                .unwrap_err()
                .说明()
                .contains("已回到默认目标")
        );
    }

    #[test]
    fn 默认入口取主程序() {
        let 工程 = 工程();
        let 入口 = 运行入口::默认(&工程).unwrap();
        assert_eq!(入口.目标名, "entry_probe");
    }

    #[test]
    fn 序列化往返() {
        let mut 入口 = 运行入口::新建("pkg", "bin");
        入口.参数 = vec!["--x".into()];
        入口.配置类型 = 构建配置类型::Release;
        let 文本 = 入口.序列化().unwrap();
        assert_eq!(运行入口::反序列化(&文本), Some(入口));
        assert_eq!(运行入口::反序列化("这不是 json"), None);
    }

    #[test]
    fn 历史保留上限且记录真实退出码() {
        let mut 历史 = 运行历史::新建(2);
        for 序号 in 0..3u64 {
            历史.开始(
                format!("cargo run #{序号}"),
                None,
                Some(100 + 序号 as u32),
                Some(format!("run-{序号}")),
                10,
            );
            历史.结束当前(结束原因::正常退出, Some(0), 12, None, 5);
        }
        assert_eq!(历史.记录().len(), 2, "超过上限的记录必须被丢弃");
        assert_eq!(历史.最近().unwrap().序号, 3, "序号仍然单调递增");
        assert_eq!(历史.最近().unwrap().退出码, Some(0));
        assert_eq!(历史.最近().unwrap().用时秒(100), 2);
    }

    #[test]
    fn 用户停止不伪造退出码() {
        let mut 历史 = 运行历史::default();
        历史.开始("cargo run", None, Some(1), None, 10);
        历史.结束当前(结束原因::用户停止, None, 15, Some("用户点了停止".into()), 3);
        let 记录 = 历史.最近().unwrap();
        assert_eq!(记录.结束原因, 结束原因::用户停止);
        assert_eq!(记录.退出码, None, "被杀进程不得有伪造退出码");
        assert!(记录.结果文本().contains("不是程序自己退出"));
    }

    #[test]
    fn 结束原因词条键唯一() {
        let mut 键 = Vec::new();
        for 原因 in [
            结束原因::进行中,
            结束原因::正常退出,
            结束原因::异常退出,
            结束原因::用户停止,
            结束原因::启动失败,
            结束原因::被终止,
        ] {
            键.push(原因.词条键());
        }
        let 原始 = 键.len();
        键.sort_unstable();
        键.dedup();
        assert_eq!(键.len(), 原始);
    }
}
