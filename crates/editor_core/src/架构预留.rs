//! P1 预留能力：原生代码 ↔ 微风场景的关联，以及运行时检查器的可行性。
//!
//! # 这个模块的定位：**先有稳定的数据模型，没有不可靠的自动推断**
//!
//! 产品上想要的是"点场景实体 → 跳到构建它的 Rust 代码""看运行中实体的
//! Transform"。这两件事都依赖一个前提：**能可靠地把代码里的东西和场景里的
//! 东西对上**。而这个前提在当前架构下**只部分成立**：
//!
//! | 想做的事 | 能不能可靠做到 | 本模块的处理 |
//! | --- | --- | --- |
//! | 记住某个场景实体对应哪个源文件 | 能（用户显式指定，或从资源引用反推） | 记录为 [`源码锚点`]，带置信度 |
//! | 从 Rust 源码里可靠识别组件/实体 | **不能**（那需要真正的 Rust 语义分析） | 明确标为 [`关联置信度::未知`]，不猜 |
//! | 判断一个字符串字面量是不是资源路径 | 能（行级扫描 + 扩展名白名单） | [`扫描资源引用`]，**并注明是行级扫描** |
//! | 把任意 Bevy ECS 实体映射到微风场景树 | **不能** | 不实现，文档里写明限制 |
//! | 读取运行中游戏的 ECS | 只有游戏主动配合（调试插件/IPC）才可能 | [`运行时数据通道`]，默认为"没有" |
//!
//! # 为什么"看起来能做"的自动映射不做
//!
//! 行级搜索能匹配到 `spawn`、`Transform`、某个字符串，但**匹配不到语义**：
//! 同一个 `Transform` 可能出现在三个函数里，哪一个对应这个实体取决于运行时
//! 实际走了哪条路径。基于文本猜出来的映射一旦错了，用户会花更多时间去
//! 排查一个假线索，比"这里暂时没有关联"更糟。
//!
//! 因此本轮只交付两样东西：
//! 1. 一个**可验证**的数据模型（锚点、置信度、来源种类、资源引用）；
//! 2. 一个**诚实的**运行时通道评估（说清"现在没有通道"以及要加什么才有）。

use std::path::{Path, PathBuf};

/// 源码的种类。
///
/// 三者必须分开，否则界面会把"编辑器自己的场景文档"和"游戏进程里的对象"
/// 混成同一个东西 —— 那是运行时检查器最容易犯的错。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 源码种类 {
    /// 原生 Rust 源码（`.rs`）。
    原生Rust,
    /// 微风自己的场景文档（`.scene.ron`）。
    微风场景,
    /// Bevy 运行时对象（游戏进程里的实体/组件）。
    Bevy运行时对象,
    /// 无法判断。
    未知,
}

impl 源码种类 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::原生Rust => "rust",
            Self::微风场景 => "breeze-scene",
            Self::Bevy运行时对象 => "bevy-runtime",
            Self::未知 => "unknown",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::原生Rust => "link_kind_rust",
            Self::微风场景 => "link_kind_scene",
            Self::Bevy运行时对象 => "link_kind_runtime",
            Self::未知 => "link_kind_unknown",
        }
    }

    /// 由文件扩展名判断。
    pub fn 由路径(路径: &Path) -> Self {
        let 文本 = 路径.to_string_lossy().to_lowercase();
        if 文本.ends_with(".rs") {
            Self::原生Rust
        } else if 文本.ends_with(".scene.ron") || 文本.ends_with(".scene") {
            Self::微风场景
        } else {
            Self::未知
        }
    }
}

/// 关联的置信度。
///
/// **这是本模块最重要的字段**：它让界面可以如实显示"这条关联是用户指定的"
/// 还是"这是从一行字符串猜出来的"。没有它，任何文本匹配都会被当成事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum 关联置信度 {
    /// 未知：没有可靠依据，界面必须显示"未知"。
    未知,
    /// 行级匹配：在源码里找到了同名字符串/资源路径，但不保证语义正确。
    行级匹配,
    /// 显式指定：用户或场景文件自己写下的来源信息。
    显式指定,
}

impl 关联置信度 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::未知 => "unknown",
            Self::行级匹配 => "line-scan",
            Self::显式指定 => "explicit",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::未知 => "link_confidence_unknown",
            Self::行级匹配 => "link_confidence_scan",
            Self::显式指定 => "link_confidence_explicit",
        }
    }

    /// 是否可以直接当事实展示（只有显式指定可以）。
    pub fn 可直接展示(self) -> bool {
        matches!(self, Self::显式指定)
    }
}

/// 一个源码锚点：场景里的东西 ↔ 源码里的一个位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 源码锚点 {
    /// 源码种类。
    pub 种类: 源码种类,
    /// 源码文件（项目相对路径或绝对路径，取决于调用方）。
    pub 文件: Option<PathBuf>,
    /// 行号（1 基，0 表示未知）。
    pub 行: u32,
    /// 关联的场景实体标识（文本形式，避免本模块依赖 `scene_doc`）。
    pub 实体: Option<String>,
    /// 关联的组件类型名（能确定时）。
    pub 组件: Option<String>,
    /// 关联的置信度。
    pub 置信度: 关联置信度,
    /// 为什么是这个置信度（人读说明，必须能解释"未知"）。
    pub 说明: String,
}

impl 源码锚点 {
    /// 构造一个"未知"锚点。
    ///
    /// 这是**默认**结果：没有可靠依据时不要编。
    pub fn 未知(说明: impl Into<String>) -> Self {
        Self {
            种类: 源码种类::未知,
            文件: None,
            行: 0,
            实体: None,
            组件: None,
            置信度: 关联置信度::未知,
            说明: 说明.into(),
        }
    }

    /// 是否可跳到源码。
    pub fn 可跳转(&self) -> bool {
        self.文件.is_some() && self.行 > 0
    }

    /// 一行显示文本（真实数据拼接）。
    pub fn 标签(&self) -> String {
        let 位置 = match &self.文件 {
            Some(文件) => {
                let 名 = 文件
                    .file_name()
                    .map(|名| 名.to_string_lossy().to_string())
                    .unwrap_or_else(|| 文件.display().to_string());
                if self.行 > 0 {
                    format!("{名}:{}", self.行)
                } else {
                    名
                }
            }
            None => "（无源码位置）".to_string(),
        };
        format!("{} · {位置}", self.置信度.代号())
    }
}

/// 场景 ↔ 代码的关联表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 场景关联表 {
    锚点: Vec<源码锚点>,
}

impl 场景关联表 {
    /// 空表。
    pub fn 新建() -> Self {
        Self::default()
    }

    /// 全部锚点。
    pub fn 锚点(&self) -> &[源码锚点] {
        &self.锚点
    }

    /// 追加一个锚点。
    pub fn 追加(&mut self, 锚点: 源码锚点) {
        self.锚点.push(锚点);
    }

    /// 某个实体的全部锚点。
    pub fn 按实体(&self, 实体: &str) -> Vec<&源码锚点> {
        self.锚点
            .iter()
            .filter(|锚点| 锚点.实体.as_deref() == Some(实体))
            .collect()
    }

    /// 是否能对该实体给出"可直接展示"的关联。
    ///
    /// 界面用它决定显示真实路径还是"未知 —— 需要显式指定"。
    pub fn 有可靠关联(&self, 实体: &str) -> bool {
        self.按实体(实体)
            .iter()
            .any(|锚点| 锚点.置信度.可直接展示() && 锚点.可跳转())
    }

    /// 清空（换项目时调用）。
    pub fn 清空(&mut self) {
        self.锚点.clear();
    }
}

/// 一条资源引用（行级扫描的结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 资源引用 {
    /// 源码文件。
    pub 文件: PathBuf,
    /// 行号（1 基）。
    pub 行: u32,
    /// 引用到的资源路径文本（引号内原文）。
    pub 路径: String,
    /// 这个文件在磁盘上是否存在（调用方传入根目录时才会检查）。
    pub 存在: bool,
}

/// 资源类扩展名白名单。
///
/// 只认这些扩展名，是为了让"扫描资源引用"不把任意字符串都当成资源路径。
/// 白名单列的是 Bevy 默认 asset loader 覆盖的常见类型。
pub const 资源扩展名: [&str; 14] = [
    "png", "jpg", "jpeg", "webp", "bmp", "gif", "ktx2", "hdr", "exr", "ogg", "mp3", "wav", "flac",
    "glb",
];

/// 从 Rust 源码文本里扫描资源引用。
///
/// # 这是**行级扫描**，不是语义分析
///
/// 它只做一件事：找引号里、以资源扩展名结尾、且不含明显变量插值的字符串。
/// 因此：
///
/// * 结果可靠地表示"这个文件第 N 行出现了一个像资源路径的字符串"；
/// * 结果**不**表示"这个资源真的会被加载"（那取决于运行时）。
///
/// 路径相对于 `资源根`（通常是 `<项目>/assets`）；`资源根` 为 `None` 时
/// 不做存在性检查，`存在` 一律为 `false`。
pub fn 扫描资源引用(
    文件: &Path, 文本: &str, 资源根: Option<&Path>
) -> Vec<资源引用> {
    let mut 结果 = Vec::new();
    for (下标, 行) in 文本.lines().enumerate() {
        let 行号 = (下标 + 1) as u32;
        // 注释行不扫：注释里的路径不是引用。
        let 修剪 = 行.trim_start();
        if 修剪.starts_with("//") {
            continue;
        }
        for 候选 in 引号里的字符串(行) {
            if !是资源路径(&候选) {
                continue;
            }
            let 存在 = 资源根
                .map(|根| {
                    根.join(候选.replace('/', std::path::MAIN_SEPARATOR_STR))
                        .is_file()
                })
                .unwrap_or(false);
            结果.push(资源引用 {
                文件: 文件.to_path_buf(),
                行: 行号,
                路径: 候选,
                存在,
            });
        }
    }
    结果
}

/// 一行里所有双引号字符串的内容（不含引号）。
///
/// 只处理双引号：Rust 的字符字面量与原始字符串在这里都不需要支持，
/// 把它们当普通文本跳过即可（误判成资源路径的概率极低，
/// 而且白名单扩展名会再挡一次）。
fn 引号里的字符串(行: &str) -> Vec<String> {
    let mut 结果 = Vec::new();
    let mut 迭代 = 行.split('"');
    // `split` 的语义：偶数下标（0,2,4…）是引号外的文本，
    // 奇数下标是引号内的内容。
    let _ = 迭代.next();
    for (序号, 段) in 迭代.enumerate() {
        if 序号 % 2 == 0 {
            结果.push(段.to_string());
        }
    }
    结果
}

/// 这个字符串看起来是不是一个资源路径。
fn 是资源路径(文本: &str) -> bool {
    let 文本 = 文本.trim();
    if 文本.is_empty() || 文本.contains('{') || 文本.contains('\n') {
        return false;
    }
    let Some(扩展) = 文本.rsplit('.').next() else {
        return false;
    };
    资源扩展名.contains(&扩展.to_lowercase().as_str())
}

/// 运行时数据通道的类型。
///
/// **`无` 是默认值，也是当前项目的真实状态。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 运行时数据通道 {
    /// 没有通道：编辑器只能看到进程的 stdout/stderr。
    #[default]
    无,
    /// 游戏里装了可选的调试插件，通过本地 socket/命名管道推送数据。
    调试插件,
    /// 游戏与编辑器约定了一套 IPC 协议（例如 stdin/stdout 上的 JSON 行）。
    Ipc协议,
}

impl 运行时数据通道 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::无 => "none",
            Self::调试插件 => "debug-plugin",
            Self::Ipc协议 => "ipc",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::无 => "runtime_channel_none",
            Self::调试插件 => "runtime_channel_plugin",
            Self::Ipc协议 => "runtime_channel_ipc",
        }
    }

    /// 是否真的能读到运行中的 ECS。
    pub fn 可读取运行时() -> bool {
        false
    }
}

/// 运行时快照的来源。
///
/// 界面必须把这四者分开显示，否则用户会对着"编辑态数据"以为在看游戏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 快照来源 {
    /// 还没采集过。
    #[default]
    未采集,
    /// 编辑器自己的场景文档（编辑态）。
    编辑态,
    /// 游戏进程实时推送的运行态数据。
    运行态,
    /// 上一次运行留下的快照（游戏已经退出）。
    上次运行,
}

impl 快照来源 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::未采集 => "uncollected",
            Self::编辑态 => "edit-time",
            Self::运行态 => "live",
            Self::上次运行 => "last-run",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::未采集 => "runtime_snapshot_none",
            Self::编辑态 => "runtime_snapshot_edit",
            Self::运行态 => "runtime_snapshot_live",
            Self::上次运行 => "runtime_snapshot_last",
        }
    }

    /// 是否是真实的运行态数据（只有实时推送才算）。
    pub fn 是实时(self) -> bool {
        matches!(self, Self::运行态)
    }
}

/// 运行时检查器的可行性结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 运行时可行性 {
    /// 当前可用的通道。
    pub 通道: 运行时数据通道,
    /// 能否读取运行中的实体/组件。
    pub 可读取运行时: bool,
    /// 结论说明（真实理由，不美化）。
    pub 说明: String,
    /// 要支持它需要做什么（下一步）。
    pub 前置条件: Vec<String>,
}

impl 运行时可行性 {
    /// 评估当前架构的可行性。
    ///
    /// 这个函数**不做探测**（探测需要真实运行一个游戏），
    /// 它只根据当前实现返回结论：编辑器自己没有运行游戏的 ECS，
    /// 也没有与游戏进程约定的协议，因此结论是"不可读取"。
    pub fn 评估() -> Self {
        Self {
            通道: 运行时数据通道::无,
            可读取运行时: false,
            说明: "微风自己的运行时世界属于编辑器进程，不是游戏进程；\
                   游戏进程只通过 stdout/stderr 提供文本，没有任何结构化运行时通道。\
                   因此当前**不能**检查运行中的实体、组件、Transform、资源与状态。"
                .to_string(),
            前置条件: vec![
                "在游戏侧提供一个可选调试插件（或让用户工程自己实现）：启动后按约定端口/管道上报实体与组件".to_string(),
                "定义一个版本化的数据结构（实体 id、组件类型名、属性值、帧号）并写明兼容策略".to_string(),
                "编辑器侧实现接收、校验与展示，并明确标注数据来源是『运行态实时推送』".to_string(),
                "不能用编辑器内部的实体冒充游戏进程里的实体 —— 两边的 Entity id 空间完全不同".to_string(),
            ],
        }
    }
}

/// 一份运行时快照。
///
/// 本轮**不会被填充任何真实数据**：没有通道就没有数据。
/// 保留类型是为了让"未采集""编辑态""上次运行"这三种状态
/// 从第一天起就分得清，而不是等有了通道再补。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct 运行时快照 {
    /// 数据来源。
    pub 来源: 快照来源,
    /// 实体数量（未采集时为 `None`）。
    pub 实体数: Option<usize>,
    /// 采集时间（UNIX 秒）。
    pub 采集时间: Option<u64>,
}

impl 运行时快照 {
    /// 未采集的快照。
    pub fn 未采集() -> Self {
        Self::default()
    }

    /// 是否有数据。
    pub fn 有数据(&self) -> bool {
        !matches!(self.来源, 快照来源::未采集)
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 源码种类按扩展名区分() {
        assert_eq!(
            源码种类::由路径(Path::new("src/main.rs")),
            源码种类::原生Rust
        );
        assert_eq!(
            源码种类::由路径(Path::new("scenes/a.scene.ron")),
            源码种类::微风场景
        );
        assert_eq!(源码种类::由路径(Path::new("a.txt")), 源码种类::未知);
        // 运行时对象不是"从文件推出来的东西"：它必须由运行时通道提供。
        assert_eq!(源码种类::Bevy运行时对象.代号(), "bevy-runtime");
    }

    #[test]
    fn 扫描资源引用只认白名单扩展名() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        std::fs::create_dir_all(根.join("textures")).unwrap();
        std::fs::write(根.join("textures/hero.png"), b"x").unwrap();

        let 文本 = r#"
fn main() {
    // "commented/not_a_ref.png"
    let a = "textures/hero.png";
    let b = "textures/missing.png";
    let c = "hello world";
    let d = format!("textures/{}.png", name);
}
"#;
        let 引用 = 扫描资源引用(Path::new("src/main.rs"), 文本, Some(根));
        assert_eq!(引用.len(), 2, "只应命中两条真实资源路径：{引用:?}");
        assert_eq!(引用[0].路径, "textures/hero.png");
        assert!(引用[0].存在);
        assert_eq!(引用[0].行, 4);
        assert_eq!(引用[1].路径, "textures/missing.png");
        assert!(!引用[1].存在, "不存在的资源必须如实标记");
    }

    #[test]
    fn 未知锚点不会伪装成事实() {
        let 锚点 = 源码锚点::未知("没有可靠的代码关联：Rust 源码里无法定位这个实体");
        assert_eq!(锚点.种类, 源码种类::未知);
        assert_eq!(锚点.置信度, 关联置信度::未知);
        assert!(!锚点.可跳转());
        assert!(!锚点.置信度.可直接展示());
        assert!(锚点.标签().contains("unknown"));
    }

    #[test]
    fn 关联表区分可靠与不可靠关联() {
        let mut 表 = 场景关联表::新建();
        表.追加(源码锚点 {
            种类: 源码种类::原生Rust,
            文件: Some(PathBuf::from("src/main.rs")),
            行: 12,
            实体: Some("实体-1".to_string()),
            组件: Some("Transform".to_string()),
            置信度: 关联置信度::行级匹配,
            说明: "行级匹配：同名字符串出现在这里".to_string(),
        });
        assert!(!表.有可靠关联("实体-1"), "行级匹配不能当成事实");
        表.追加(源码锚点 {
            种类: 源码种类::原生Rust,
            文件: Some(PathBuf::from("src/player.rs")),
            行: 30,
            实体: Some("实体-1".to_string()),
            组件: None,
            置信度: 关联置信度::显式指定,
            说明: "用户在检查器里指定".to_string(),
        });
        assert!(表.有可靠关联("实体-1"));
        assert_eq!(表.按实体("实体-1").len(), 2);
    }

    #[test]
    fn 运行时可行性默认不可读取() {
        let 结论 = 运行时可行性::评估();
        assert_eq!(结论.通道, 运行时数据通道::无);
        assert!(!结论.可读取运行时);
        assert!(!运行时数据通道::可读取运行时());
        assert!(结论.说明.contains("不能"));
        assert!(!结论.前置条件.is_empty(), "必须给出要做什么才有通道");
    }

    #[test]
    fn 运行时快照默认是未采集() {
        let 快照 = 运行时快照::未采集();
        assert_eq!(快照.来源, 快照来源::未采集);
        assert!(!快照.有数据());
        assert!(!快照.来源.是实时());
        assert!(快照.实体数.is_none(), "未采集不得编造实体数");
    }
}
