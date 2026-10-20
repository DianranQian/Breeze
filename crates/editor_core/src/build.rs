//! 构建任务：把 Cargo 的编译输出变成编辑器能显示、能跳转、能判断成败的数据。
//!
//! # 参考的 Godot 设计
//!
//! Godot 的构建/导出入口在 `editor/export/`（导出平台与预设）与
//! `editor/debugger/`（编译错误、运行错误与性能）两处，共同的结构是：
//!
//! 1. **配置是项目契约**：导出预设存在项目里，而不是编辑器全局偏好；
//! 2. **任务与界面解耦**：构建/导出在后台推进，界面只读"当前状态 + 事件列表"；
//! 3. **错误带位置**：每条错误都有 `文件:行`，点击能跳到脚本编辑器。
//!
//! 本模块只承担第 1 与第 3 点的**纯数据部分**（可脱离进程与 UI 单测），
//! 进程管理在 `editor_app::build`，面板绘制在 `editor_ui::build_panel`。
//!
//! # 为什么解析 Cargo 的 JSON 而不是正则抓 `error[E0308]`
//!
//! `cargo build --message-format=json` 每行一个 JSON 对象，
//! 其中的 `compiler-message` 携带结构化 span（文件、行、列、级别、错误码）。
//! 抓文本正则会把"多行错误""跨文件 span""笔记(note)"全部弄错，
//! 而且很容易把警告里的 `error:` 字样误判为错误。
//!
//! 注意**不是** `json-render-diagnostics`：那个模式只把诊断渲染到 stderr，
//! stdout 上不再有 `compiler-message`，编辑器会一条也解析不到。
//! 这一点是实测发现的，见 [`构建配置::命令行`] 的注释。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 构建配置类型（对应 Cargo 的 profile）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 构建配置类型 {
    /// 开发构建（`cargo build`）。
    #[default]
    Debug,
    /// 发布构建（`cargo build --release`）。
    Release,
}

impl 构建配置类型 {
    /// 全部可选值，供界面列成下拉项。
    pub const 全部: [Self; 2] = [Self::Debug, Self::Release];

    /// 稳定代号，写进 `project.ron`。不要改。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    /// 由代号解析，未知值回退到 Debug。
    pub fn 从代号(代号: &str) -> Self {
        match 代号 {
            "release" => Self::Release,
            _ => Self::Debug,
        }
    }

    /// 传给 cargo 的 profile 参数（Debug 不需要参数）。
    pub fn 命令行参数(self) -> Option<&'static str> {
        match self {
            Self::Debug => None,
            Self::Release => Some("--release"),
        }
    }

    /// 产物所在子目录名（相对于 Cargo 的 `target/`）。
    ///
    /// 这是 Cargo 的固定约定：release 产物在 `target/release/`，
    /// 其余 profile 在 `target/<profile>/`。自定义 profile 由
    /// [`构建配置::目标目录`] 统一处理，这里只覆盖两种内置情形。
    pub fn 产物目录名(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

/// 构建任务实际执行的 Cargo 子命令。
///
/// 构建与测试共用同一套诊断解析：两者都通过 `--message-format=json` 输出
/// `compiler-message`；测试失败本身不是编译器诊断，因此还要看真实退出码与
/// stderr 尾部（见 [`crate::build::构建报告`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 构建子命令 {
    /// `cargo build`。
    #[default]
    构建,
    /// `cargo test`。
    测试,
    /// `cargo check`（只要诊断，不产出二进制）。
    检查,
}

impl 构建子命令 {
    /// 传给 cargo 的子命令名。
    pub fn 命令名(self) -> &'static str {
        match self {
            构建子命令::构建 => "build",
            构建子命令::测试 => "test",
            构建子命令::检查 => "check",
        }
    }
}

/// 一次构建的配置。
///
/// 它直接存进项目清单（`project.ron` 的可选 `构建` 段），
/// 因此**删字段必须保留 serde 默认值**，否则旧项目会打不开。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 构建配置 {
    /// 执行哪个 Cargo 子命令。
    #[serde(default)]
    pub 子命令: 构建子命令,
    /// profile。
    #[serde(default)]
    pub 配置类型: 构建配置类型,
    /// 目标三元组（`--target`）。留空表示宿主机。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 目标平台: Option<String>,
    /// 包名（`-p`）。多包 workspace 里必须指定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 包名: Option<String>,
    /// 启用的 Cargo 特性（`--features`）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub 特征: Vec<String>,
    /// 追加的原始参数，原样传给 cargo。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub 额外参数: Vec<String>,
    /// 期望产物名（不含扩展名）。留空表示用包名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 二进制名: Option<String>,
}

impl Default for 构建配置 {
    fn default() -> Self {
        Self {
            子命令: 构建子命令::构建,
            配置类型: 构建配置类型::Debug,
            目标平台: None,
            包名: None,
            特征: Vec::new(),
            额外参数: Vec::new(),
            二进制名: None,
        }
    }
}

impl 构建配置 {
    /// 组装传给 cargo 的参数（**不含** `cargo` 本身）。
    ///
    /// 顺序刻意固定：先子命令，再消息格式，再 profile、目标、包、特性，
    /// 最后才是用户额外参数 —— 用户参数放最后才能覆盖前面的同名项。
    pub fn 命令行(&self) -> Vec<String> {
        let mut 参数 = vec![
            self.子命令.命令名().to_string(),
            // # 为什么是 `json` 而不是 `json-render-diagnostics`
            //
            // 这是一个**实测踩出来的坑**：`json-render-diagnostics` 的行为是
            // "把诊断渲染到 stderr 而**不**在 stdout 发 `compiler-message`"。
            // 于是编辑器只能拿到 `build-finished`，一条诊断也解析不到，
            // 面板上表现为"构建失败了，但没有错误列表"。
            //
            // `json` 才是在 stdout 逐行输出 `compiler-message` 的那个模式，
            // 其中 `message.rendered` 还带着人读文本。人读输出我们并不需要
            // （界面自己排版），需要的是**结构化的 span**。
            "--message-format=json".to_string(),
        ];
        if let Some(profile) = self.配置类型.命令行参数() {
            参数.push(profile.to_string());
        }
        if let Some(目标) = self
            .目标平台
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            参数.push("--target".to_string());
            参数.push(目标.to_string());
        }
        if let Some(包) = self
            .包名
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            参数.push("-p".to_string());
            参数.push(包.to_string());
        }
        let 特征: Vec<&str> = self
            .特征
            .iter()
            .map(|项| 项.trim())
            .filter(|项| !项.is_empty())
            .collect();
        if !特征.is_empty() {
            参数.push("--features".to_string());
            参数.push(特征.join(","));
        }
        for 项 in &self.额外参数 {
            let 项 = 项.trim();
            if !项.is_empty() {
                参数.push(项.to_string());
            }
        }
        参数
    }

    /// 供状态栏/日志显示的一行命令。
    pub fn 可读命令行(&self) -> String {
        let mut 文本 = String::from("cargo");
        for 参数 in self.命令行() {
            文本.push(' ');
            文本.push_str(&参数);
        }
        文本
    }

    /// 产物可能所在的目录（相对于项目根）。
    ///
    /// Cargo 在交叉编译时会把产物放到 `target/<三元组>/<profile>/`，
    /// 因此带目标平台时要多插一层，否则界面上给出的产物路径是错的。
    pub fn 目标目录(&self) -> PathBuf {
        let mut 目录 = PathBuf::from("target");
        if let Some(目标) = self
            .目标平台
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            目录.push(目标);
        }
        目录.push(self.配置类型.产物目录名());
        目录
    }
}

/// 诊断级别，与 Cargo/rustc 的 `level` 字段一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum 诊断级别 {
    /// `error` 与 `error[E0308]`。
    错误,
    /// `warning`。
    警告,
    /// `note` / `failure-note`。
    提示,
    /// `help`。
    备注,
}

impl 诊断级别 {
    /// 由 rustc 的级别字符串解析。未知级别按"提示"处理：
    /// 不认识的东西不该被算成错误，那会让构建结果误报失败。
    pub fn 从rustc(级别: &str) -> Self {
        match 级别 {
            "error" | "error: internal compiler error" => Self::错误,
            "warning" => Self::警告,
            "help" => Self::备注,
            _ => Self::提示,
        }
    }

    /// 是否阻断构建（只有错误阻断）。
    pub fn 有阻塞性(self) -> bool {
        matches!(self, Self::错误)
    }

    /// 界面颜色槽位的语义名。
    pub fn 语义(self) -> &'static str {
        match self {
            Self::错误 => "错误",
            Self::警告 => "警告",
            Self::提示 => "次要",
            Self::备注 => "成功",
        }
    }

    /// 词条键，供界面显示。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::错误 => "build_level_error",
            Self::警告 => "build_level_warning",
            Self::提示 => "build_level_note",
            Self::备注 => "build_level_help",
        }
    }
}

/// 诊断指向的源码位置。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct 代码位置 {
    /// 绝对路径。`None` 表示与项目文件无关（例如链接器错误）。
    pub 文件: Option<PathBuf>,
    /// 行号，1 基。0 表示未知。
    pub 行: u32,
    /// 列号，1 基。0 表示未知。
    pub 列: u32,
}

impl 代码位置 {
    /// `文件名:行` 形式的短标签，供诊断列表右侧显示。
    pub fn 短标签(&self) -> Option<String> {
        let 文件 = self.文件.as_ref()?;
        let 名 = 文件.file_name()?.to_string_lossy().to_string();
        if self.行 == 0 {
            Some(名)
        } else {
            Some(format!("{名}:{}", self.行))
        }
    }

    /// 是否可以跳转（有文件且有行号）。
    pub fn 可跳转(&self) -> bool {
        self.文件.is_some() && self.行 > 0
    }
}

/// 一条构建诊断。
#[derive(Debug, Clone, PartialEq)]
pub struct 构建诊断 {
    /// 级别。
    pub 级别: 诊断级别,
    /// 首行消息（rustc 的 `message` 字段，不含子诊断）。
    pub 消息: String,
    /// 错误码，例如 `E0308`。
    pub 代码: Option<String>,
    /// 主 span 的位置。
    pub 位置: Option<代码位置>,
}

impl 构建诊断 {
    /// 用于界面单行显示的文本：`消息` + 可选错误码。
    pub fn 单行文本(&self) -> String {
        match &self.代码 {
            Some(码) => format!("{} [{码}]", self.消息),
            None => self.消息.clone(),
        }
    }

    /// 是否可跳到源码。
    pub fn 可跳转(&self) -> bool {
        self.位置.as_ref().is_some_and(代码位置::可跳转)
    }
}

/// 解析一行 cargo 输出得到的事件。
///
/// 只有三种事件与编辑器有关：诊断、产物、构建结束。
/// 其余（`compiler-artifact` 里没有可执行文件的、`build-script-executed` 等）
/// 一律归为 [`编译事件::无关`]，由调用方原样当日志转出去。
#[derive(Debug, Clone, PartialEq)]
pub enum 编译事件 {
    /// 一条诊断。
    诊断(构建诊断),
    /// 一个可执行产物。
    产物 {
        /// 绝对路径。
        路径: PathBuf,
    },
    /// 构建结束。
    结束 {
        /// 是否成功。
        成功: bool,
    },
    /// 与本编辑器无关的行，原样转发即可。
    无关,
}

/// 解析一行 cargo 的 JSON 输出。
///
/// 不是 JSON 的行（cargo 自己也往 stdout 打进度、彩色诊断）返回
/// [`编译事件::无关`] —— 这不是错误，**不能**因为一行解析失败就判定构建失败。
pub fn 解析编译事件(行: &str) -> 编译事件 {
    let 行 = 行.trim();
    if 行.is_empty() || !行.starts_with('{') {
        return 编译事件::无关;
    }
    let Ok(值) = serde_json::from_str::<serde_json::Value>(行) else {
        return 编译事件::无关;
    };
    match 值.get("reason").and_then(|r| r.as_str()) {
        Some("compiler-message") => match 值.get("message") {
            Some(消息) => match 解析诊断(消息) {
                Some(诊断) => 编译事件::诊断(诊断),
                None => 编译事件::无关,
            },
            None => 编译事件::无关,
        },
        Some("compiler-artifact") => {
            // 只有 `executable` 非空的产物才值得显示；
            // 库的 `.rlib` 对"能不能运行游戏"没有直接帮助。
            match 值.get("executable").and_then(|e| e.as_str()) {
                Some(路径) if !路径.is_empty() => 编译事件::产物 {
                    路径: PathBuf::from(路径),
                },
                _ => 编译事件::无关,
            }
        }
        Some("build-finished") => 编译事件::结束 {
            成功: 值.get("success").and_then(|s| s.as_bool()).unwrap_or(false),
        },
        _ => 编译事件::无关,
    }
}

/// 把一个 `message` JSON 对象解析成诊断。
///
/// 没有 `message` 文本的对象（纯 `failure-note` 容器）返回 `None`。
fn 解析诊断(值: &serde_json::Value) -> Option<构建诊断> {
    let 级别 = 诊断级别::从rustc(值.get("level").and_then(|l| l.as_str()).unwrap_or("note"));
    let 消息 = 值.get("message").and_then(|m| m.as_str())?.to_string();
    if 消息.trim().is_empty() {
        return None;
    }
    let 代码 = 值
        .get("code")
        .and_then(|码| 码.get("code"))
        .and_then(|码| 码.as_str())
        .map(str::to_string);
    // 只取**主 span**（`is_primary == true`）。rustc 会给出多个 span
    // （"这个值在这里被借用""第一次可变借用发生在这里"），
    // 编辑器需要的是一个确定的落点，不是全部。
    let 位置 = 值
        .get("spans")
        .and_then(|spans| spans.as_array())
        .and_then(|spans| {
            spans
                .iter()
                .find(|span| span.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
                .or_else(|| spans.first())
        })
        .map(|span| 代码位置 {
            文件: span
                .get("file_name")
                .and_then(|f| f.as_str())
                .filter(|f| !f.is_empty())
                .map(PathBuf::from),
            行: span.get("line_start").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
            列: span
                .get("column_start")
                .and_then(|c| c.as_u64())
                .unwrap_or(0) as u32,
        });
    Some(构建诊断 {
        级别,
        消息,
        代码,
        位置,
    })
}

/// 构建任务的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 构建状态 {
    /// 还没有构建过。
    #[default]
    空闲,
    /// 正在构建。
    运行中,
    /// 构建成功。
    成功,
    /// 构建失败（含启动失败）。
    失败,
    /// 用户取消了构建。
    已取消,
}

impl 构建状态 {
    /// 是否正在进行。
    pub fn 在运行(self) -> bool {
        matches!(self, Self::运行中)
    }

    /// 是否已经结束（成功/失败/取消）。
    pub fn 已结束(self) -> bool {
        matches!(self, Self::成功 | Self::失败 | Self::已取消)
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::空闲 => "build_state_idle",
            Self::运行中 => "build_state_running",
            Self::成功 => "build_state_succeeded",
            Self::失败 => "build_state_failed",
            Self::已取消 => "build_state_cancelled",
        }
    }
}

/// 一次构建的完整报告。
///
/// 它同时是"界面读的数据"与"测试断言的输出对象"，因此**不持有**
/// 任何进程/线程句柄 —— 那些在 `editor_app::build` 里。
#[derive(Debug, Clone, Default)]
pub struct 构建报告 {
    /// 状态。
    pub 状态: 构建状态,
    /// 诊断，按"错误 → 警告 → 提示 → 备注"稳定排序后再按出现顺序。
    pub 诊断: Vec<构建诊断>,
    /// 可执行产物路径。
    pub 产物: Vec<PathBuf>,
    /// 本次构建耗时（毫秒）。
    pub 耗时毫秒: u64,
    /// 实际执行的命令行。
    pub 命令行: String,
    /// 启动失败或非结构化输出的摘要（失败原因的人读说明）。
    pub 摘要: Option<String>,
}

impl 构建报告 {
    /// 一次新构建的开始状态。
    pub fn 开始(命令行: impl Into<String>) -> Self {
        Self {
            状态: 构建状态::运行中,
            命令行: 命令行.into(),
            ..Self::default()
        }
    }

    /// 记一条诊断，并保持排序。
    pub fn 追加诊断(&mut self, 诊断: 构建诊断) {
        self.诊断.push(诊断);
        // 稳定排序：同级诊断保持 rustc 的输出顺序，
        // 否则同一个文件的错误会在列表里跳来跳去。
        self.诊断.sort_by_key(|诊断| 诊断.级别);
    }

    /// 记一个产物。
    pub fn 记录产物(&mut self, 路径: PathBuf) {
        if !self.产物.contains(&路径) {
            self.产物.push(路径);
        }
    }

    /// 结束构建。
    ///
    /// `成功` 只表示"cargo 退出码为 0"；即使成功，诊断里的警告也保留，
    /// 因为用户需要看到它们。
    pub fn 完成(&mut self, 成功: bool, 耗时毫秒: u64) {
        self.状态 = if 成功 {
            构建状态::成功
        } else {
            构建状态::失败
        };
        self.耗时毫秒 = 耗时毫秒;
        if !成功 && self.摘要.is_none() && self.诊断.is_empty() {
            // 失败但没有任何结构化诊断：通常意味着 cargo 根本没跑起来
            // （找不到 Cargo.toml、命令不在 PATH 上）。给一句可读的解释，
            // 而不是让用户面对一个空列表。
            self.摘要 = Some(String::new());
        }
    }

    /// 标记为已取消。
    pub fn 取消(&mut self, 耗时毫秒: u64) {
        self.状态 = 构建状态::已取消;
        self.耗时毫秒 = 耗时毫秒;
    }

    /// 错误条数。
    pub fn 错误数(&self) -> usize {
        self.诊断
            .iter()
            .filter(|诊断| 诊断.级别 == 诊断级别::错误)
            .count()
    }

    /// 警告条数。
    pub fn 警告数(&self) -> usize {
        self.诊断
            .iter()
            .filter(|诊断| 诊断.级别 == 诊断级别::警告)
            .count()
    }

    /// 是否没有任何问题。
    pub fn 无问题(&self) -> bool {
        self.诊断.is_empty()
    }

    /// 第一个可跳转的错误位置 —— 「点一下跳到第一个错误」用它。
    pub fn 首个错误位置(&self) -> Option<&代码位置> {
        self.诊断
            .iter()
            .find(|诊断| 诊断.级别 == 诊断级别::错误)
            .and_then(|诊断| 诊断.位置.as_ref())
            .filter(|位置| 位置.可跳转())
    }

    /// 主产物（第一个可执行文件）。
    pub fn 主产物(&self) -> Option<&PathBuf> {
        self.产物.first()
    }

    /// 状态摘要文本，供状态栏与面板顶部使用。
    pub fn 摘要文本(&self) -> Option<&str> {
        self.摘要.as_deref().filter(|摘要| !摘要.is_empty())
    }
}

/// 界面每帧读取的构建快照：配置 + 报告。
///
/// 把它们打包成一个值，是为了让 `绘制上下文` 只多一个字段，
/// 而不是"每加一个功能就给上下文加两个字段"。
#[derive(Debug, Clone, Default)]
pub struct 构建快照 {
    /// 当前配置（来自项目清单）。
    pub 配置: 构建配置,
    /// 最近一次构建报告。
    pub 报告: 构建报告,
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 默认配置只构建当前包() {
        let 配置 = 构建配置::default();
        assert_eq!(
            配置.命令行(),
            vec!["build".to_string(), "--message-format=json".to_string()]
        );
        assert_eq!(配置.目标目录(), PathBuf::from("target").join("debug"));
    }

    #[test]
    fn 配置参数按固定顺序组装() {
        let 配置 = 构建配置 {
            子命令: 构建子命令::构建,
            配置类型: 构建配置类型::Release,
            目标平台: Some("x86_64-pc-windows-msvc".into()),
            包名: Some("my_game".into()),
            特征: vec!["debug_ui".into(), " ".into(), " extra ".into()],
            额外参数: vec!["--locked".into()],
            二进制名: None,
        };
        let 参数 = 配置.命令行();
        assert_eq!(参数[0], "build");
        assert_eq!(参数[1], "--message-format=json");
        assert_eq!(参数[2], "--release");
        assert_eq!(参数[3], "--target");
        assert_eq!(参数[4], "x86_64-pc-windows-msvc");
        assert_eq!(参数[5], "-p");
        assert_eq!(参数[6], "my_game");
        assert_eq!(参数[7], "--features");
        // 空白项被丢掉，首尾空格被修剪 —— 用户复制粘贴时常带上空格。
        assert_eq!(参数[8], "debug_ui,extra");
        assert_eq!(参数[9], "--locked");
        // 交叉编译的目标目录必须多一层目标三元组。
        assert_eq!(
            配置.目标目录(),
            PathBuf::from("target")
                .join("x86_64-pc-windows-msvc")
                .join("release")
        );
    }

    #[test]
    fn 解析结构化错误诊断() {
        // 真实 `cargo build --message-format=json` 的一行（截断了无关字段）。
        let 行 = r#"{"reason":"compiler-message","package_id":"path+file:///p#my_game@0.1.0","manifest_path":"C:\\p\\Cargo.toml","target":{"kind":["bin"],"name":"my_game"},"message":{"rendered":"error[E0308]: mismatched types\n","$message_type":"diagnostic","message":"mismatched types","code":{"code":"E0308","explanation":null},"level":"error","spans":[{"file_name":"src/main.rs","byte_start":10,"byte_end":12,"line_start":3,"line_end":3,"column_start":5,"column_end":7,"is_primary":true,"text":[]}],"children":[],"rendered":"error[E0308]"}}"#;
        let 事件 = 解析编译事件(行);
        let 编译事件::诊断(诊断) = 事件 else {
            panic!("应当解析出诊断，实际 {事件:?}");
        };
        assert_eq!(诊断.级别, 诊断级别::错误);
        assert_eq!(诊断.消息, "mismatched types");
        assert_eq!(诊断.代码.as_deref(), Some("E0308"));
        let 位置 = 诊断.位置.as_ref().expect("应当有位置");
        assert_eq!(位置.文件, Some(PathBuf::from("src/main.rs")));
        assert_eq!(位置.行, 3);
        assert_eq!(位置.列, 5);
        assert!(诊断.可跳转());
        assert_eq!(诊断.单行文本(), "mismatched types [E0308]");
    }

    #[test]
    fn 解析产物与结束事件() {
        let 行 = r#"{"reason":"compiler-artifact","package_id":"p","executable":"C:\\p\\target\\debug\\my_game.exe","target":{"kind":["bin"],"name":"my_game"}}"#;
        assert_eq!(
            解析编译事件(行),
            编译事件::产物 {
                路径: PathBuf::from("C:\\p\\target\\debug\\my_game.exe")
            }
        );
        // 没有 executable 的产物（库）不进列表。
        assert_eq!(
            解析编译事件(r#"{"reason":"compiler-artifact","executable":null}"#),
            编译事件::无关
        );
        assert_eq!(
            解析编译事件(r#"{"reason":"build-finished","success":true}"#),
            编译事件::结束 { 成功: true }
        );
        assert_eq!(
            解析编译事件(r#"{"reason":"build-finished","success":false}"#),
            编译事件::结束 { 成功: false }
        );
    }

    #[test]
    fn 非_json_行不算错误() {
        // cargo 自己的进度输出、编译器的彩色诊断都会混在 stdout 里。
        // 把它们当成解析失败会让正常构建被判定为失败。
        for 行 in [
            "   Compiling my_game v0.1.0",
            "error: could not compile `my_game` (bin \"my_game\")",
            "",
            "{ 这不是 json",
        ] {
            assert_eq!(解析编译事件(行), 编译事件::无关, "行：{行:?}");
        }
    }

    #[test]
    fn 报告按级别排序并统计() {
        let mut 报告 = 构建报告::开始("cargo build");
        let 警告 = 构建诊断 {
            级别: 诊断级别::警告,
            消息: "unused".into(),
            代码: None,
            位置: None,
        };
        let 错误 = 构建诊断 {
            级别: 诊断级别::错误,
            消息: "boom".into(),
            代码: Some("E0001".into()),
            位置: Some(代码位置 {
                文件: Some(PathBuf::from("src/lib.rs")),
                行: 7,
                列: 1,
            }),
        };
        报告.追加诊断(警告.clone());
        报告.追加诊断(错误.clone());
        // 错误排在前面：用户打开面板第一眼要看到阻断项。
        assert_eq!(报告.诊断[0].级别, 诊断级别::错误);
        assert_eq!(报告.错误数(), 1);
        assert_eq!(报告.警告数(), 1);
        assert_eq!(
            报告.首个错误位置().map(|位置| 位置.行),
            Some(7),
            "首个错误位置应当可跳转"
        );

        报告.完成(false, 1234);
        assert_eq!(报告.状态, 构建状态::失败);
        assert_eq!(报告.耗时毫秒, 1234);
        // 有诊断时不该再塞一句空摘要。
        assert!(报告.摘要文本().is_none());
    }

    #[test]
    fn 无诊断失败给出摘要而不是空列表() {
        let mut 报告 = 构建报告::开始("cargo build");
        报告.完成(false, 5);
        assert_eq!(报告.状态, 构建状态::失败);
        assert!(报告.诊断.is_empty());
        // 摘要存在（内容由调用方补），界面据此显示"cargo 没有输出结构化诊断"。
        assert!(报告.摘要.is_some());
    }

    #[test]
    fn 状态判定() {
        assert!(构建状态::运行中.在运行());
        assert!(!构建状态::空闲.在运行());
        assert!(构建状态::成功.已结束());
        assert!(构建状态::失败.已结束());
        assert!(构建状态::已取消.已结束());
        assert!(!构建状态::运行中.已结束());
    }

    #[test]
    fn 配置类型代号往返() {
        for 类型 in 构建配置类型::全部 {
            assert_eq!(构建配置类型::从代号(类型.代号()), 类型);
        }
        assert_eq!(构建配置类型::从代号("unknown"), 构建配置类型::Debug);
    }
}
