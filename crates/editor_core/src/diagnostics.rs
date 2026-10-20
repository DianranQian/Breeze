//! 统一诊断中心：把构建、检查、测试、运行日志、panic、资源加载失败与
//! Agent 结果都表达成**同一种**可筛选、可跳转、可归属任务的数据。
//!
//! # 为什么需要"统一"而不是各面板自己攒一份
//!
//! 之前构建面板有自己的 `构建诊断`，控制台有自己的日志条目，运行日志只是
//! 一堆字符串。三种表示带来三个具体问题：
//!
//! | 问题 | 后果 |
//! | --- | --- |
//! | 没有任务归属 | 上一次构建的错误与这一次混在一起，用户以为改完还报错 |
//! | 三套位置字段 | 只有构建面板能点着跳源码，运行期的 panic 只能人眼找行号 |
//! | 层层转发各建一条 | 同一条错误在列表里出现两三次 |
//!
//! 因此这里定义**一份**数据模型，并明确规定两条纪律：
//!
//! 1. **每条诊断都属于某个任务**（[`任务标识`]）。构建/运行各自开启任务，
//!    界面默认只看当前任务的结果，历史任务可翻但不是"当前结果"。
//! 2. **同一条诊断只建一次**（[`诊断集合::追加`] 按内容去重）。
//!
//! # 什么算诊断，什么不算
//!
//! 诊断是"值得用户处理的一条问题"，不是"一行日志"。普通 `stderr` 输出
//! （cargo 的 `Compiling`、Bevy 的普通 info 日志）**不算诊断** ——
//! 把它们全算成问题会让列表里 99% 是噪声，用户从此不再看诊断面板。
//! 原始日志由控制台与运行日志视图保留，一条都不丢（见 [`诊断条目::原始日志`]）。
//!
//! # 解析的优先级
//!
//! * 编译诊断：来自 `cargo --message-format=json` 的 `compiler-message`，
//!   **结构化字段**（[`crate::build::解析编译事件`]），不用文本正则；
//! * 运行期：只能从文本识别（rustc/链接器/运行时都只会打印文本），
//!   因此规则刻意保守，并且**保留原文**，宁可漏判也不制造假错误。

use std::path::PathBuf;

use crate::build::诊断级别;

/// 诊断的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum 诊断来源 {
    /// `cargo build` 的编译诊断。
    构建,
    /// `cargo check` 的诊断。
    检查,
    /// `cargo test` 的失败与诊断。
    测试,
    /// 游戏进程的输出（panic、错误日志、非零退出）。
    运行,
    /// 资源加载失败（Bevy 的 asset 错误）。
    资源,
    /// AI Agent 的任务与修复结果。
    智能体,
}

impl 诊断来源 {
    /// 全部来源（界面顺序）。
    pub const 全部: [Self; 6] = [
        Self::构建,
        Self::检查,
        Self::测试,
        Self::运行,
        Self::资源,
        Self::智能体,
    ];

    /// 稳定代号（进日志与证据 JSON，不随语言变化）。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::构建 => "build",
            Self::检查 => "check",
            Self::测试 => "test",
            Self::运行 => "run",
            Self::资源 => "asset",
            Self::智能体 => "agent",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::构建 => "diag_source_build",
            Self::检查 => "diag_source_check",
            Self::测试 => "diag_source_test",
            Self::运行 => "diag_source_run",
            Self::资源 => "diag_source_asset",
            Self::智能体 => "diag_source_agent",
        }
    }

    /// 中文名（写进给 Agent 的提示词，不参与界面翻译）。
    pub fn 中文名(self) -> &'static str {
        match self {
            Self::构建 => "构建",
            Self::检查 => "检查",
            Self::测试 => "测试",
            Self::运行 => "运行",
            Self::资源 => "资源加载",
            Self::智能体 => "智能体",
        }
    }

    /// 由 [`任务类别`] 推出默认来源。
    pub fn 由任务(类别: 任务类别) -> Self {
        match 类别 {
            任务类别::构建 => Self::构建,
            任务类别::运行 => Self::运行,
            任务类别::智能体 => Self::智能体,
        }
    }
}

/// 诊断严重级别。
///
/// 与 [`诊断级别`] 的区别：这里是**统一模型**的级别，只分三档。
/// rustc 的 `note`/`help` 归到 [`诊断严重级别::信息`] —— 它们不是问题，
/// 是在解释问题，界面不该把它们和警告混在一列。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum 诊断严重级别 {
    /// 错误：阻断构建或导致运行失败。
    错误,
    /// 警告：不阻断，但需要处理。
    警告,
    /// 信息：解释性内容（rustc 的 note/help）。
    信息,
}

impl 诊断严重级别 {
    /// 由 rustc/构建诊断级别转换。
    pub fn 从构建级别(级别: 诊断级别) -> Self {
        match 级别 {
            诊断级别::错误 => Self::错误,
            诊断级别::警告 => Self::警告,
            诊断级别::提示 | 诊断级别::备注 => Self::信息,
        }
    }

    /// 是否阻断（只有错误阻断）。
    pub fn 有阻塞性(self) -> bool {
        matches!(self, Self::错误)
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::错误 => "diag_level_error",
            Self::警告 => "diag_level_warning",
            Self::信息 => "diag_level_info",
        }
    }

    /// 单字符标记（列表左侧的徽标文字，不翻译）。
    pub fn 标记(self) -> &'static str {
        match self {
            Self::错误 => "E",
            Self::警告 => "W",
            Self::信息 => "I",
        }
    }
}

/// 任务类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum 任务类别 {
    /// 一次构建/检查/测试。
    构建,
    /// 一次游戏运行。
    运行,
    /// 一次 Agent 任务。
    智能体,
}

impl 任务类别 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::构建 => "build",
            Self::运行 => "run",
            Self::智能体 => "agent",
        }
    }

    /// 中文名（写进提示词）。
    pub fn 中文名(self) -> &'static str {
        match self {
            Self::构建 => "构建",
            Self::运行 => "运行",
            Self::智能体 => "智能体任务",
        }
    }
}

/// 一条诊断归属的任务。
///
/// `序号` 是按类别单调递增的真实序号（从 1 开始），因此 `构建 #3`
/// 就是本次编辑器会话里的第三次构建。它让"这条错误是哪一次跑出来的"
/// 不再靠时间戳猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct 任务标识 {
    /// 类别。
    pub 类别: 任务类别,
    /// 类别内序号（从 1 开始）。
    pub 序号: u64,
}

impl 任务标识 {
    /// 新建。
    pub fn 新(类别: 任务类别, 序号: u64) -> Self {
        Self { 类别, 序号 }
    }

    /// 显示文本（`构建 #3`）。写进日志与提示词，不参与界面翻译。
    pub fn 显示(self) -> String {
        format!("{} #{}", self.类别.中文名(), self.序号)
    }
}

/// 一条诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 诊断条目 {
    /// 集合内唯一编号（单调递增，从 1 开始）。
    pub 编号: u64,
    /// 来源。
    pub 来源: 诊断来源,
    /// 级别。
    pub 级别: 诊断严重级别,
    /// 错误代码（`E0308`、`panic`、`asset-load` 等）。没有就是 `None`。
    pub 代码: Option<String>,
    /// 首行消息。
    pub 消息: String,
    /// 源码文件（相对项目根的路径或绝对路径，取决于 Cargo 的输出）。
    pub 文件: Option<PathBuf>,
    /// 行号（1 基，0 表示未知）。
    pub 行: u32,
    /// 列号（1 基，0 表示未知）。
    pub 列: u32,
    /// 所属 package（能确定时）。
    pub 包: Option<String>,
    /// 所属 target（能确定时）。
    pub 目标: Option<String>,
    /// 发生时间（UNIX 秒）。
    pub 发生时间: u64,
    /// 归属任务。
    pub 任务: Option<任务标识>,
    /// 无法结构化解析时保留的原始日志（一行）。
    pub 原始日志: Option<String>,
}

impl 诊断条目 {
    /// 新建一条（编号由集合分配，这里先置 0）。
    pub fn 新建(
        来源: 诊断来源, 级别: 诊断严重级别, 消息: impl Into<String>
    ) -> Self {
        Self {
            编号: 0,
            来源,
            级别,
            代码: None,
            消息: 消息.into(),
            文件: None,
            行: 0,
            列: 0,
            包: None,
            目标: None,
            发生时间: 0,
            任务: None,
            原始日志: None,
        }
    }

    /// 是否可跳到源码（有文件且有行号）。
    pub fn 可跳转(&self) -> bool {
        self.文件.is_some() && self.行 > 0
    }

    /// 一行显示文本：`消息` + 可选错误码。
    pub fn 单行文本(&self) -> String {
        match &self.代码 {
            Some(码) => format!("{} [{码}]", self.消息),
            None => self.消息.clone(),
        }
    }

    /// `文件名:行` 形式的短标签。
    pub fn 位置标签(&self) -> Option<String> {
        let 文件 = self.文件.as_ref()?;
        let 名 = 文件.file_name()?.to_string_lossy().to_string();
        if self.行 == 0 {
            Some(名)
        } else if self.列 == 0 {
            Some(format!("{名}:{}", self.行))
        } else {
            Some(format!("{名}:{}:{}", self.行, self.列))
        }
    }

    /// `路径:行:列` 形式的完整位置。
    ///
    /// 与 [`诊断条目::位置标签`] 的分工：标签给界面列表省宽度，
    /// 完整位置给**给 Agent 的上下文**与复制 —— 只有 `main.rs:7`
    /// 是没法让 Agent 定位到是哪个 crate 的。
    pub fn 完整位置(&self) -> Option<String> {
        let 文件 = self.文件.as_ref()?;
        let 文本 = 文件.display().to_string();
        if self.行 == 0 {
            Some(文本)
        } else if self.列 == 0 {
            Some(format!("{文本}:{}", self.行))
        } else {
            Some(format!("{文本}:{}:{}", self.行, self.列))
        }
    }

    /// 去重键。
    ///
    /// 刻意**不含**消息里的路径前缀与时间：同一处错误被多层转发时，
    /// 每次转发的原文可能带不同的前缀，但它们必须在列表里只出现一次。
    pub fn 去重键(
        &self,
    ) -> (
        诊断来源,
        任务类别序号,
        诊断严重级别,
        Option<String>,
        String,
        Option<PathBuf>,
        u32,
    ) {
        (
            self.来源,
            self.任务.map(|任务| (任务.类别, 任务.序号)),
            self.级别,
            self.代码.clone(),
            self.消息.trim().to_string(),
            self.文件.clone(),
            self.行,
        )
    }
}

/// 去重键里"任务"那一段的类型别名（可读性用）。
type 任务类别序号 = Option<(任务类别, u64)>;

/// 统一诊断集合。
///
/// 持有上限，超出时丢最旧的：运行期日志可以无限刷，
/// 而界面只需要"当前任务的结果 + 最近的历史"。
#[derive(Debug, Clone)]
pub struct 诊断集合 {
    /// 全部诊断（按加入顺序）。
    条目: Vec<诊断条目>,
    /// 下一个编号。
    下一个编号: u64,
    /// 保留上限。
    上限: usize,
    /// 各类别的任务序号计数器。
    任务计数: [u64; 3],
    /// 构建历史（新的在前）。
    构建记录: Vec<构建记录>,
}

/// 默认保留上限。
///
/// 2000 条 × 每条几十字节 ≈ 百 KB 量级；一次编译几百条诊断、
/// 一次运行几百条 panic/资源失败都装得下。
pub const 默认诊断上限: usize = 2000;

/// 构建历史保留条数。
pub const 保留构建记录: usize = 10;

impl Default for 诊断集合 {
    fn default() -> Self {
        Self::新建(默认诊断上限)
    }
}

impl 诊断集合 {
    /// 以指定上限新建。
    pub fn 新建(上限: usize) -> Self {
        Self {
            条目: Vec::new(),
            下一个编号: 1,
            上限: 上限.max(1),
            任务计数: [0; 3],
            构建记录: Vec::new(),
        }
    }

    /// 全部条目（历史顺序）。
    pub fn 条目(&self) -> &[诊断条目] {
        &self.条目
    }

    /// 条目数量。
    pub fn 长度(&self) -> usize {
        self.条目.len()
    }

    /// 是否为空。
    pub fn 为空(&self) -> bool {
        self.条目.is_empty()
    }

    /// 清空（换项目时调用：旧项目的诊断不属于新项目）。
    pub fn 清空(&mut self) {
        self.条目.clear();
        self.下一个编号 = 1;
        self.任务计数 = [0; 3];
        self.构建记录.clear();
    }

    /// 开启一个新任务，返回它的标识。
    pub fn 开启任务(&mut self, 类别: 任务类别) -> 任务标识 {
        let 下标 = match 类别 {
            任务类别::构建 => 0,
            任务类别::运行 => 1,
            任务类别::智能体 => 2,
        };
        self.任务计数[下标] = self.任务计数[下标].saturating_add(1);
        任务标识::新(类别, self.任务计数[下标])
    }

    /// 记录一次构建/检查/测试的结果。
    ///
    /// 只保留最近 [`保留构建记录`] 条：构建历史是排查线索，不是审计日志。
    pub fn 记录构建(&mut self, 记录: 构建记录) {
        self.构建记录.insert(0, 记录);
        self.构建记录.truncate(保留构建记录);
        self.下一个编号 = self.下一个编号.max(1);
    }

    /// 构建历史（新的在前）。
    pub fn 构建历史(&self) -> &[构建记录] {
        &self.构建记录
    }

    /// 给某条诊断追加一段消息（用于跨行的 panic 文本）。
    ///
    /// # 为什么需要这个
    ///
    /// Rust 的 panic 输出是**两行**的：
    ///
    /// ```text
    /// thread 'main' panicked at src/main.rs:54:9:
    /// fixture-panic-probe：第 5 帧崩溃
    /// ```
    ///
    /// 第一行只有位置，真正的原因在下一行。若只解析第一行，诊断里就只剩
    /// 位置信息 —— 用户看到"某行 panic 了"却不知道原因。
    pub fn 追加消息(&mut self, 编号: u64, 追加: &str) -> bool {
        let Some(条目) = self.条目.iter_mut().find(|条目| 条目.编号 == 编号) else {
            return false;
        };
        let 追加 = 追加.trim();
        if 追加.is_empty() {
            return false;
        }
        if !条目.消息.trim().is_empty() {
            条目.消息.push(' ');
        }
        条目.消息.push_str(追加);
        true
    }

    /// 按编号取一条诊断。
    pub fn 取(&self, 编号: u64) -> Option<&诊断条目> {
        self.条目.iter().find(|条目| 条目.编号 == 编号)
    }

    /// 追加一条诊断。返回是否真的新增（被去重时返回 `false`）。
    pub fn 追加(&mut self, mut 条目: 诊断条目) -> bool {
        let 键 = 条目.去重键();
        if self.条目.iter().any(|已有| 已有.去重键() == 键) {
            return false;
        }
        条目.编号 = self.下一个编号;
        self.下一个编号 = self.下一个编号.saturating_add(1);
        self.条目.push(条目);
        // 超上限时丢最旧的。按"加入顺序"丢而不是按级别丢：
        // 保留最近发生的事比保留"级别高的旧事"更符合排查直觉。
        if self.条目.len() > self.上限 {
            let 溢出 = self.条目.len() - self.上限;
            self.条目.drain(0..溢出);
        }
        true
    }

    /// 批量追加，返回新增条数。
    pub fn 批量追加(&mut self, 列表: impl IntoIterator<Item = 诊断条目>) -> usize {
        列表
            .into_iter()
            .filter(|条目| self.追加(条目.clone()))
            .count()
    }

    /// 清掉某个任务的全部诊断（重跑同一任务前调用，避免新旧混在一起）。
    pub fn 移除任务(&mut self, 任务: 任务标识) -> usize {
        let 前 = self.条目.len();
        self.条目.retain(|条目| 条目.任务 != Some(任务));
        前 - self.条目.len()
    }

    /// 某个任务的全部诊断。
    pub fn 按任务(&self, 任务: 任务标识) -> Vec<&诊断条目> {
        self.条目
            .iter()
            .filter(|条目| 条目.任务 == Some(任务))
            .collect()
    }

    /// 某个类别最近一次任务的标识。
    pub fn 最新任务(&self, 类别: 任务类别) -> Option<任务标识> {
        let 序号 = match 类别 {
            任务类别::构建 => self.任务计数[0],
            任务类别::运行 => self.任务计数[1],
            任务类别::智能体 => self.任务计数[2],
        };
        (序号 > 0).then(|| 任务标识::新(类别, 序号))
    }

    /// 当前结果视图：每个类别**最近一次任务**的诊断，按级别排序。
    ///
    /// 这是界面列表默认显示的东西 —— 历史任务的诊断不属于"当前结果"。
    pub fn 当前视图(&self) -> Vec<&诊断条目> {
        let 当前任务: Vec<任务标识> = [任务类别::构建, 任务类别::运行, 任务类别::智能体]
            .into_iter()
            .filter_map(|类别| self.最新任务(类别))
            .collect();
        let mut 结果: Vec<&诊断条目> = self
            .条目
            .iter()
            .filter(|条目| match 条目.任务 {
                Some(任务) => 当前任务.contains(&任务),
                // 没有任务归属的诊断（例如"根本无法启动 cargo"这类
                // 环境错误）始终属于当前结果：它们不是某次任务产生的。
                None => true,
            })
            .collect();
        结果.sort_by_key(|条目| (条目.级别, 条目.编号));
        结果
    }

    /// 按来源与级别筛选（在给定切片上过滤）。
    ///
    /// 接收一个已经算好的视图（通常是 [`诊断集合::当前视图`]），
    /// 这样"当前结果"与"筛选"两件事可以分别测试。
    pub fn 筛选<'a>(
        &self, 源: &'a [&'a 诊断条目], 显示: &筛选条件
    ) -> Vec<&'a 诊断条目> {
        源.iter().copied().filter(|条目| 显示.接受(条目)).collect()
    }

    /// 错误条数（当前结果视图）。
    pub fn 当前错误数(&self) -> usize {
        self.当前视图()
            .into_iter()
            .filter(|条目| 条目.级别 == 诊断严重级别::错误)
            .count()
    }

    /// 警告条数（当前结果视图）。
    pub fn 当前警告数(&self) -> usize {
        self.当前视图()
            .into_iter()
            .filter(|条目| 条目.级别 == 诊断严重级别::警告)
            .count()
    }

    /// 把当前结果拼成一段**真实**文本，供 Agent 上下文与复制使用。
    ///
    /// 分段明确写出"构建错误 / 运行错误 / 环境错误"，这样 Agent
    /// 不会把"工具链没装"当成用户代码的 bug 去改。
    pub fn 上下文文本(&self, 最多条: usize) -> String {
        let 视图 = self.当前视图();
        if 视图.is_empty() {
            return String::new();
        }
        let mut 文本 = String::new();
        for 分组 in [
            (诊断来源::构建, "构建错误"),
            (诊断来源::检查, "检查错误"),
            (诊断来源::测试, "测试失败"),
            (诊断来源::运行, "运行时错误"),
            (诊断来源::资源, "资源加载失败"),
            (诊断来源::智能体, "智能体任务"),
        ] {
            let 项: Vec<&&诊断条目> = 视图
                .iter()
                .filter(|条目| 条目.来源 == 分组.0 && 条目.级别 == 诊断严重级别::错误)
                .take(最多条)
                .collect();
            if 项.is_empty() {
                continue;
            }
            文本.push_str(&format!("\n### {}\n", 分组.1));
            for 条目 in 项 {
                let 位置 = 条目
                    .完整位置()
                    .map(|标签| format!("（{标签}）"))
                    .unwrap_or_default();
                let 任务 = 条目
                    .任务
                    .map(|任务| format!("[{}] ", 任务.显示()))
                    .unwrap_or_default();
                文本.push_str(&format!("- {任务}{}{位置}\n", 条目.单行文本()));
            }
        }
        // 警告单独一节：Agent 需要知道它们，但它们不是失败原因。
        let 警告: Vec<&&诊断条目> = 视图
            .iter()
            .filter(|条目| 条目.级别 == 诊断严重级别::警告)
            .take(最多条)
            .collect();
        if !警告.is_empty() {
            文本.push_str("\n### 警告\n");
            for 条目 in 警告 {
                let 位置 = 条目
                    .完整位置()
                    .map(|标签| format!("（{标签}）"))
                    .unwrap_or_default();
                文本.push_str(&format!("- {}{位置}\n", 条目.单行文本()));
            }
        }
        文本
    }

    /// 生成界面快照。
    pub fn 生成快照(&self) -> 诊断快照 {
        诊断快照 {
            条目: self.当前视图().into_iter().cloned().collect(),
            全部条目: self.条目.clone(),
            错误数: self.当前错误数(),
            警告数: self.当前警告数(),
            构建任务: self.最新任务(任务类别::构建),
            运行任务: self.最新任务(任务类别::运行),
            智能体任务: self.最新任务(任务类别::智能体),
            构建历史: self.构建历史().to_vec(),
        }
    }

    /// 单条诊断的原文摘要（供"把这条发给 Agent"）。
    pub fn 单条上下文(&self, 编号: u64) -> Option<String> {
        let 条目 = self.条目.iter().find(|条目| 条目.编号 == 编号)?;
        let mut 文本 = String::new();
        文本.push_str(&format!(
            "来源：{} · 级别：{:?}\n",
            条目.来源.中文名(),
            条目.级别
        ));
        if let Some(任务) = 条目.任务 {
            文本.push_str(&format!("任务：{}\n", 任务.显示()));
        }
        if let Some(位置) = 条目.完整位置() {
            文本.push_str(&format!("位置：{}\n", 位置));
        }
        if let Some(码) = &条目.代码 {
            文本.push_str(&format!("代码：{码}\n"));
        }
        文本.push_str(&format!("消息：{}\n", 条目.消息));
        if let Some(原文) = &条目.原始日志
            && 原文.trim() != 条目.消息.trim()
        {
            文本.push_str(&format!("原始日志：{原文}\n"));
        }
        文本.push_str(
            "\n要求：先用只读工具确认相关文件与行号，再给出最小修复；\
             不要使用 git reset --hard / git clean / git restore 等丢弃工作区改动的命令；\
             不要修改与本诊断无关的既有改动；\
             修完后由微风重新执行真实构建验证，不要自行声称构建通过。",
        );
        Some(文本)
    }
}

/// 一条构建记录（历史，不是"当前报告"）。
///
/// 构建面板只显示**最近一次**报告；用户排查"刚才那次为什么失败、再上一次
/// 成功了吗"需要一条历史。这里只记摘要，不保存整份诊断 —— 诊断本身在
/// [`诊断集合`] 里按任务归属保存。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 构建记录 {
    /// 归属任务。
    pub 任务: 任务标识,
    /// 实际执行的命令行。
    pub 命令行: String,
    /// 是否成功（真实退出码 0 才算）。
    pub 成功: bool,
    /// 耗时（毫秒）。
    pub 耗时毫秒: u64,
    /// 错误条数。
    pub 错误数: usize,
    /// 警告条数。
    pub 警告数: usize,
    /// 发生时间（UNIX 秒）。
    pub 时间: u64,
}

impl 构建记录 {
    /// 一句话结果。
    pub fn 结果文本(&self) -> String {
        if self.成功 {
            format!("成功（{} 个警告）", self.警告数)
        } else if self.错误数 > 0 {
            format!("失败（{} 个错误）", self.错误数)
        } else {
            "失败（没有结构化诊断）".to_string()
        }
    }
}

/// 界面读的诊断快照。
///
/// 放在核心层是因为**界面层也要读它**，而 `editor_ui` 不依赖 `editor_app`。
/// 它只是数据：没有任何进程、线程或 egui 类型。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 诊断快照 {
    /// 当前结果视图（每个类别最近一次任务的诊断，按级别排序）。
    pub 条目: Vec<诊断条目>,
    /// 全部诊断（含历史任务，供"显示历史"与按编号打开源码）。
    pub 全部条目: Vec<诊断条目>,
    /// 当前错误数。
    pub 错误数: usize,
    /// 当前警告数。
    pub 警告数: usize,
    /// 最近一次构建任务。
    pub 构建任务: Option<任务标识>,
    /// 最近一次运行任务。
    pub 运行任务: Option<任务标识>,
    /// 最近一次智能体任务。
    pub 智能体任务: Option<任务标识>,
    /// 最近的构建/检查/测试记录（新的在前）。
    ///
    /// 运行结果不在这里：它由 `editor_core::run_entry::运行快照` 提供，
    /// 同一份数据不存两遍。
    pub 构建历史: Vec<构建记录>,
}

impl 诊断快照 {
    /// 按编号找一条诊断（在**全部**条目里找，历史任务也能打开源码）。
    pub fn 按编号(&self, 编号: u64) -> Option<&诊断条目> {
        self.全部条目.iter().find(|条目| 条目.编号 == 编号)
    }

    /// 是否没有任何诊断。
    pub fn 为空(&self) -> bool {
        self.条目.is_empty()
    }
}

/// 诊断列表的筛选条件（界面状态用，纯数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 筛选条件 {
    /// 只保留这些来源（空表示全部）。
    pub 来源: Vec<诊断来源>,
    /// 是否显示错误。
    pub 显示错误: bool,
    /// 是否显示警告。
    pub 显示警告: bool,
    /// 是否显示信息。
    pub 显示信息: bool,
    /// 关键字（空表示不过滤；匹配消息与文件）。
    pub 关键字: String,
}

impl Default for 筛选条件 {
    fn default() -> Self {
        Self {
            来源: Vec::new(),
            显示错误: true,
            显示警告: true,
            显示信息: false,
            关键字: String::new(),
        }
    }
}

impl 筛选条件 {
    /// 一条诊断是否通过筛选。
    pub fn 接受(&self, 条目: &诊断条目) -> bool {
        let 级别通过 = match 条目.级别 {
            诊断严重级别::错误 => self.显示错误,
            诊断严重级别::警告 => self.显示警告,
            诊断严重级别::信息 => self.显示信息,
        };
        if !级别通过 {
            return false;
        }
        if !self.来源.is_empty() && !self.来源.contains(&条目.来源) {
            return false;
        }
        let 关键字 = self.关键字.trim();
        if 关键字.is_empty() {
            return true;
        }
        条目.消息.contains(关键字)
            || 条目
                .文件
                .as_ref()
                .map(|文件| 文件.to_string_lossy().contains(关键字))
                .unwrap_or(false)
    }
}

/// 由一条构建诊断构造诊断条目。
///
/// `包` / `目标` 来自 `cargo metadata` 或构建配置：构建面板要能回答
/// "这个错误是哪个 crate 的"，workspace 里尤其重要。
pub fn 从构建诊断(
    来源: 诊断来源,
    诊断: &crate::build::构建诊断,
    任务: 任务标识,
    包: Option<String>,
    目标: Option<String>,
    发生时间: u64,
) -> 诊断条目 {
    let mut 条目 =
        诊断条目::新建(来源, 诊断严重级别::从构建级别(诊断.级别), 诊断.消息.clone());
    条目.代码 = 诊断.代码.clone();
    条目.文件 = 诊断.位置.as_ref().and_then(|位置| 位置.文件.clone());
    条目.行 = 诊断.位置.as_ref().map(|位置| 位置.行).unwrap_or(0);
    条目.列 = 诊断.位置.as_ref().map(|位置| 位置.列).unwrap_or(0);
    条目.包 = 包;
    条目.目标 = 目标;
    条目.任务 = Some(任务);
    条目.发生时间 = 发生时间;
    条目
}

/// 解析一行运行日志，判断它是不是一条诊断。
///
/// # 规则刻意保守
///
/// 只有**明确带错误标记**的行才变成诊断：
///
/// | 形态 | 结果 |
/// | --- | --- |
/// | `thread 'main' panicked at src/main.rs:12:5: message` | 错误，带位置，代码 `panic` |
/// | `2025-01-01T00:00:00Z ERROR bevy_asset::server: ...` | 按 tracing 级别，错误/警告 |
/// | `error[E0308]: ...` | 错误，带错误码 |
/// | `error: ...` | 错误 |
/// | `Failed to load asset ...` | 资源错误，代码 `asset-load` |
/// | 其他（含普通 stderr） | **不是诊断** |
///
/// 无法定位到文件时也保留原文（[`诊断条目::原始日志`]），
/// 用户至少能在诊断详情里看到它到底说了什么。
pub fn 解析运行日志(
    行: &str,
    来源: 诊断来源,
    任务: 任务标识,
    发生时间: u64,
) -> Option<诊断条目> {
    let 原文 = 行.trim_end();
    // 去掉运行管理器加的流前缀（`[stderr] ` / `[stdout] `）。
    //
    // 它是**传输层标记**，用来在日志视图里区分两个流；放进诊断消息只会让
    // 用户看到 `[stderr] thread 'main' panicked at ...` 这种噪声。
    // 原始日志（`条目.原始日志`）仍保留带前缀的原样，排查时不丢信息。
    let 修剪 = 原文
        .trim()
        .strip_prefix("[stderr]")
        .or_else(|| 原文.trim().strip_prefix("[stdout]"))
        .unwrap_or_else(|| 原文.trim())
        .trim();
    if 修剪.is_empty() {
        return None;
    }
    // ---- 1. panic（最明确，且带位置）----
    if let Some(位置) = 修剪.find("panicked at ") {
        let 之后 = &修剪[位置 + "panicked at ".len()..];
        let (文件, 行号, 列号, 消息) = 解析位置与消息(之后);
        let mut 条目 = 诊断条目::新建(来源, 诊断严重级别::错误, {
            if 消息.trim().is_empty() {
                修剪.to_string()
            } else {
                消息.trim().to_string()
            }
        });
        条目.代码 = Some("panic".to_string());
        条目.文件 = 文件;
        条目.行 = 行号;
        条目.列 = 列号;
        条目.任务 = Some(任务);
        条目.发生时间 = 发生时间;
        条目.原始日志 = Some(原文.to_string());
        return Some(条目);
    }

    // ---- 2. tracing / log 风格的级别标记 ----
    //
    // Bevy 0.20 的默认格式是 `2025-01-01T00:00:00.123456Z  INFO bevy_ecs::world: ...`
    // （级别前后都有空格），也有 `ERROR bevy_asset::server:` 这种行首形态。
    if let Some((级别, 位置)) = 找日志级别(修剪) {
        let 消息 = 修剪[位置..].trim().to_string();
        let 是资源 = 看起来像资源失败(&消息);
        let mut 条目 = 诊断条目::新建(
            if 是资源 {
                诊断来源::资源
            } else {
                来源
            },
            级别,
            消息.clone(),
        );
        条目.代码 = 是资源.then(|| "asset-load".to_string());
        条目.任务 = Some(任务);
        条目.发生时间 = 发生时间;
        条目.原始日志 = Some(原文.to_string());
        return Some(条目);
    }

    // ---- 3. rustc / 链接器风格的结构化错误 ----
    if let Some((码, 消息)) = 解析错误码(修剪) {
        let mut 条目 = 诊断条目::新建(来源, 诊断严重级别::错误, 消息);
        条目.代码 = 码;
        条目.任务 = Some(任务);
        条目.发生时间 = 发生时间;
        条目.原始日志 = Some(原文.to_string());
        return Some(条目);
    }

    // ---- 4. 资源加载失败的文本 ----
    if 看起来像资源失败(修剪) {
        let mut 条目 = 诊断条目::新建(诊断来源::资源, 诊断严重级别::错误, 修剪.to_string());
        条目.代码 = Some("asset-load".to_string());
        条目.任务 = Some(任务);
        条目.发生时间 = 发生时间;
        条目.原始日志 = Some(原文.to_string());
        return Some(条目);
    }

    None
}

/// 从 panic 行里解析 `路径:行:列: 消息`。
///
/// 三种真实形态都要处理：
/// * `src/main.rs:12:5: message`（新版本 rust 的 panic 位置格式）；
/// * `src/main.rs:12:5`（没有消息）；
/// * `message`（没有位置，`RUST_BACKTRACE` 被裁掉时）。
fn 解析位置与消息(文本: &str) -> (Option<PathBuf>, u32, u32, String) {
    let 文本 = 文本.trim();
    // 形如 `path:行:列: rest` / `path:行: rest` / `path:行:列`。
    // 路径里可能含 `:`（Windows 盘符），因此从右往左找数字段。
    let 字节 = 文本.as_bytes();
    let mut 末尾数字组: Vec<(usize, usize)> = Vec::new();
    let mut 下标 = 0;
    while 下标 < 字节.len() {
        if 字节[下标] == b':' {
            let 起 = 下标 + 1;
            let mut 止 = 起;
            while 止 < 字节.len() && 字节[止].is_ascii_digit() {
                止 += 1;
            }
            if 止 > 起 {
                末尾数字组.push((起, 止));
                下标 = 止;
                continue;
            }
        }
        下标 += 1;
    }
    if 末尾数字组.len() >= 2 {
        let (行起, 行止) = 末尾数字组[末尾数字组.len() - 2];
        let (列起, 列止) = 末尾数字组[末尾数字组.len() - 1];
        let 行号 = 文本[行起..行止].parse::<u32>().unwrap_or(0);
        let 列号 = 文本[列起..列止].parse::<u32>().unwrap_or(0);
        // 文件路径是第一个数字组之前的全部内容，去掉结尾的 `:`。
        let 文件止 = 文本[..行起].rfind(':').unwrap_or(0);
        let 文件 = 文本[..文件止].trim();
        if !文件.is_empty() {
            let 消息 = 文本[列止..].trim_start_matches(':').trim().to_string();
            return (Some(PathBuf::from(文件)), 行号, 列号, 消息);
        }
    }
    if 末尾数字组.len() == 1 {
        let (行起, 行止) = 末尾数字组[0];
        let 行号 = 文本[行起..行止].parse::<u32>().unwrap_or(0);
        let 文件止 = 文本[..行起].rfind(':').unwrap_or(0);
        let 文件 = 文本[..文件止].trim();
        if !文件.is_empty() {
            let 消息 = 文本[行止..].trim_start_matches(':').trim().to_string();
            return (Some(PathBuf::from(文件)), 行号, 0, 消息);
        }
    }
    (None, 0, 0, 文本.to_string())
}

/// 找行内第一个日志级别标记，返回级别与**消息起点**的字节位置。
///
/// 只认大写形态（`ERROR` / `WARN` / `INFO`）：小写 `error` 会撞上
/// 普通文本里对"错误"一词的引用，那会把日志误判成诊断。
fn 找日志级别(文本: &str) -> Option<(诊断严重级别, usize)> {
    for (标记, 级别) in [("ERROR", 诊断严重级别::错误), ("WARN", 诊断严重级别::警告)]
    {
        // 行首标记（`ERROR bevy_asset: ...`）。
        if let Some(余) = 文本.strip_prefix(标记)
            && (余.starts_with(' ') || 余.starts_with(':'))
        {
            let 起点 = 标记.len();
            return Some((级别, 起点));
        }
        // 时间戳之后的标记（`2025-...Z ERROR bevy_asset: ...`）。
        if let Some(位置) = 文本.find(&format!(" {标记} ")) {
            return Some((级别, 位置 + 标记.len() + 2));
        }
        if let Some(位置) = 文本.find(&format!(" {标记}:")) {
            return Some((级别, 位置 + 标记.len() + 2));
        }
    }
    None
}

/// 解析 `error[E0308]: ...` / `error: ...`。
fn 解析错误码(文本: &str) -> Option<(Option<String>, String)> {
    // 行首可能有 `[stderr] ` 之类的前缀，因此在任意位置找 `error[E....]:`。
    let 小写 = 文本.to_ascii_lowercase();
    let Some(起点) = 小写.find("error[") else {
        return 小写
            .find("error: ")
            .map(|位置| (None, 文本[位置 + "error: ".len()..].trim().to_string()));
    };
    let 余 = &文本[起点 + "error[".len()..];
    let 闭合 = 余.find(']')?;
    let 码 = 余[..闭合].trim().to_string();
    if 码.is_empty() {
        return None;
    }
    let 消息 = 余[闭合 + 1..]
        .trim_start()
        .trim_start_matches(':')
        .trim()
        .to_string();
    Some((Some(码), 消息))
}

/// 这一行是否像"资源加载失败"。
fn 看起来像资源失败(文本: &str) -> bool {
    let 小写 = 文本.to_lowercase();
    小写.contains("failed to load asset")
        || 小写.contains("unable to load asset")
        || 小写.contains("failed to load `")
        || 小写.contains("asset loader")
        || (小写.contains("asset") && 小写.contains("not found"))
        || 小写.contains("missing asset")
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 任务() -> 任务标识 {
        任务标识::新(任务类别::构建, 1)
    }

    #[test]
    fn 任务序号按类别独立递增() {
        let mut 集合 = 诊断集合::default();
        let 构建1 = 集合.开启任务(任务类别::构建);
        let 构建2 = 集合.开启任务(任务类别::构建);
        let 运行1 = 集合.开启任务(任务类别::运行);
        assert_eq!((构建1.序号, 构建2.序号, 运行1.序号), (1, 2, 1));
        assert_eq!(集合.最新任务(任务类别::构建), Some(构建2));
        assert_eq!(集合.最新任务(任务类别::智能体), None);
    }

    #[test]
    fn 同一条诊断只建一次() {
        let mut 集合 = 诊断集合::default();
        let 任务 = 集合.开启任务(任务类别::构建);
        let mut 条目 = 诊断条目::新建(诊断来源::构建, 诊断严重级别::错误, "mismatched types");
        条目.代码 = Some("E0308".into());
        条目.文件 = Some(PathBuf::from("src/main.rs"));
        条目.行 = 7;
        条目.任务 = Some(任务);
        assert!(集合.追加(条目.clone()));
        assert!(!集合.追加(条目.clone()), "重复转发必须被去重");
        assert_eq!(集合.长度(), 1);
    }

    #[test]
    fn 旧任务诊断不进入当前结果() {
        let mut 集合 = 诊断集合::default();
        let 旧 = 集合.开启任务(任务类别::构建);
        let mut 条目 = 诊断条目::新建(诊断来源::构建, 诊断严重级别::错误, "旧错误");
        条目.任务 = Some(旧);
        集合.追加(条目);

        let 新 = 集合.开启任务(任务类别::构建);
        assert!(
            集合.当前视图().is_empty(),
            "新任务还没产生诊断时当前结果必须为空"
        );
        assert_eq!(集合.按任务(旧).len(), 1, "历史任务的诊断仍然保留");

        let mut 新条目 = 诊断条目::新建(诊断来源::构建, 诊断严重级别::警告, "新警告");
        新条目.任务 = Some(新);
        集合.追加(新条目);
        let 视图 = 集合.当前视图();
        assert_eq!(视图.len(), 1);
        assert_eq!(视图[0].消息, "新警告");
    }

    #[test]
    fn 没有任务归属的诊断始终属于当前结果() {
        let mut 集合 = 诊断集合::default();
        let _ = 集合.开启任务(任务类别::构建);
        let mut 环境 = 诊断条目::新建(
            诊断来源::构建,
            诊断严重级别::错误,
            "无法启动 cargo：找不到程序",
        );
        环境.任务 = None;
        集合.追加(环境);
        assert_eq!(集合.当前视图().len(), 1);
        assert_eq!(集合.当前错误数(), 1);
    }

    #[test]
    fn 超上限时丢最旧的() {
        let mut 集合 = 诊断集合::新建(3);
        for 序号 in 0..6 {
            let mut 条目 =
                诊断条目::新建(诊断来源::运行, 诊断严重级别::错误, format!("第{序号}条"));
            条目.任务 = Some(任务());
            集合.追加(条目);
        }
        assert_eq!(集合.长度(), 3);
        assert_eq!(集合.条目()[0].消息, "第3条");
    }

    #[test]
    fn 解析panci带位置() {
        let 行 = "thread 'main' panicked at src/main.rs:12:5: index out of bounds";
        let 条目 = 解析运行日志(行, 诊断来源::运行, 任务(), 100).expect("应当识别");
        assert_eq!(条目.级别, 诊断严重级别::错误);
        assert_eq!(条目.代码.as_deref(), Some("panic"));
        assert_eq!(条目.文件, Some(PathBuf::from("src/main.rs")));
        assert_eq!((条目.行, 条目.列), (12, 5));
        assert_eq!(条目.消息, "index out of bounds");
        assert_eq!(条目.任务, Some(任务()));
        assert_eq!(条目.发生时间, 100);
        assert!(条目.可跳转());
    }

    #[test]
    fn 解析panci在windows路径下也能定位() {
        let 行 = "thread 'main' panicked at C:\\proj\\src\\main.rs:42:1: boom";
        let 条目 = 解析运行日志(行, 诊断来源::运行, 任务(), 0).unwrap();
        assert_eq!(条目.行, 42);
        assert_eq!(
            条目.文件.as_ref().unwrap().to_string_lossy(),
            "C:\\proj\\src\\main.rs"
        );
    }

    #[test]
    fn 流前缀不会进入诊断消息() {
        // 运行管理器给 stderr 加了 `[stderr] ` 前缀用于区分流；
        // 诊断消息里不该出现它（原始日志里保留）。
        let 条目 = 解析运行日志(
            "[stderr] thread 'main' panicked at src/main.rs:3:1: boom",
            诊断来源::运行,
            任务(),
            0,
        )
        .expect("应当识别");
        assert!(!条目.消息.contains("[stderr]"), "消息：{}", 条目.消息);
        assert_eq!(条目.消息, "boom");
        assert!(
            条目
                .原始日志
                .as_deref()
                .is_some_and(|原文| 原文.contains("[stderr]")),
            "原始日志必须保留原样"
        );
    }

    #[test]
    fn 普通stderr不算诊断() {
        for 行 in [
            "[stderr]    Compiling my_game v0.1.0",
            "Finished dev profile",
            "hello from the game",
            "warning: unused variable",
        ] {
            assert!(
                解析运行日志(行, 诊断来源::运行, 任务(), 0).is_none(),
                "普通输出不该被当成诊断：{行:?}"
            );
        }
    }

    #[test]
    fn 解析tracing级别的错误与警告() {
        let 错误行 = "2025-01-01T00:00:00.000000Z ERROR bevy_asset::server: Failed to load asset";
        let 条目 = 解析运行日志(错误行, 诊断来源::运行, 任务(), 0).unwrap();
        assert_eq!(条目.级别, 诊断严重级别::错误);
        assert_eq!(条目.来源, 诊断来源::资源);
        assert_eq!(条目.代码.as_deref(), Some("asset-load"));

        let 警告行 = "2025-01-01T00:00:00.000000Z  WARN bevy_render: adapter limit";
        let 条目 = 解析运行日志(警告行, 诊断来源::运行, 任务(), 0).unwrap();
        assert_eq!(条目.级别, 诊断严重级别::警告);
        assert!(
            !条目.消息.starts_with("2025"),
            "消息里不该带时间戳：{}",
            条目.消息
        );
    }

    #[test]
    fn 解析编译器风格错误码() {
        let 条目 = 解析运行日志(
            "[stderr] error[E0308]: mismatched types",
            诊断来源::运行,
            任务(),
            0,
        )
        .unwrap();
        assert_eq!(条目.代码.as_deref(), Some("E0308"));
        assert_eq!(条目.消息, "mismatched types");
    }

    #[test]
    fn 资源加载失败被归到资源来源() {
        let 条目 = 解析运行日志(
            "Unable to load asset 'textures/hero.png'",
            诊断来源::运行,
            任务(),
            0,
        )
        .unwrap();
        assert_eq!(条目.来源, 诊断来源::资源);
        assert_eq!(条目.代码.as_deref(), Some("asset-load"));
    }

    #[test]
    fn 筛选条件按级别与来源生效() {
        let mut 集合 = 诊断集合::default();
        let 任务 = 集合.开启任务(任务类别::构建);
        for (级别, 消息) in [
            (诊断严重级别::错误, "错误一"),
            (诊断严重级别::警告, "警告一"),
            (诊断严重级别::信息, "信息一"),
        ] {
            let mut 条目 = 诊断条目::新建(诊断来源::构建, 级别, 消息);
            条目.任务 = Some(任务);
            集合.追加(条目);
        }
        let 全部 = 集合.当前视图();
        let 条件 = 筛选条件::default();
        let 结果 = 集合.筛选(&全部, &条件);
        assert_eq!(结果.len(), 2, "默认显示错误与警告，不显示信息");

        let 只要警告 = 筛选条件 {
            显示错误: false,
            ..筛选条件::default()
        };
        let 结果 = 集合.筛选(&全部, &只要警告);
        assert_eq!(结果.len(), 1);
        assert_eq!(结果[0].消息, "警告一");

        let 只要运行 = 筛选条件 {
            来源: vec![诊断来源::运行],
            ..筛选条件::default()
        };
        assert!(集合.筛选(&全部, &只要运行).is_empty());
    }

    #[test]
    fn 上下文文本区分构建与运行时错误() {
        let mut 集合 = 诊断集合::default();
        let 构建任务 = 集合.开启任务(任务类别::构建);
        let 运行任务 = 集合.开启任务(任务类别::运行);
        let mut 构建条目 =
            诊断条目::新建(诊断来源::构建, 诊断严重级别::错误, "mismatched types");
        构建条目.代码 = Some("E0308".into());
        构建条目.任务 = Some(构建任务);
        构建条目.文件 = Some(PathBuf::from("src/main.rs"));
        构建条目.行 = 7;
        集合.追加(构建条目);
        let mut 运行条目 = 诊断条目::新建(诊断来源::运行, 诊断严重级别::错误, "panicked");
        运行条目.任务 = Some(运行任务);
        集合.追加(运行条目);

        let 文本 = 集合.上下文文本(10);
        assert!(文本.contains("### 构建错误"), "{文本}");
        assert!(文本.contains("### 运行时错误"), "{文本}");
        assert!(文本.contains("src/main.rs:7"), "{文本}");

        // 提示词里给 Agent 的纪律写在**单条**上下文里：
        // 整体摘要只负责如实列出事实。
        let 单条 = 集合.单条上下文(集合.条目()[0].编号).unwrap();
        assert!(单条.contains("E0308"));
        assert!(单条.contains("不要使用 git reset --hard"));
        assert!(单条.contains("不要自行声称构建通过"));
    }

    #[test]
    fn 空集合的上下文文本是空串() {
        let 集合 = 诊断集合::default();
        assert!(集合.上下文文本(5).is_empty());
        assert_eq!(集合.当前错误数(), 0);
    }
}
