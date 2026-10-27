//! 项目模型：项目的创建、打开、目录布局与资源/场景清单。
//!
//! # 目录布局的取舍
//!
//! Godot 使用**单一 `res://` 根 + 全部文件混放**的模型。这个模型对用户简单，
//! 但把"资源"和"场景"混在一棵目录里，会让"哪些文件是编辑器资产"这个问题
//! 只能靠扩展名猜。本编辑器改用**按用途分区**的布局：
//!
//! ```text
//! 我的游戏/
//! ├── project.ron          项目清单（名称、格式版本、路径约定）
//! ├── assets/              原始资源：图片、音频、字体、模型
//! │   ├── textures/
//! │   ├── audio/
//! │   └── models/
//! ├── scenes/              场景文档（编辑器的主要产物）
//! └── src/                  可选的 Rust 源码目录（游戏逻辑）
//! ```
//!
//! 这样做的直接好处：项目根目录下任何**不在**上述分区内的文件
//! （`.git`、`target`、构建产物）都不会被资源浏览器当成资产，
//! 不需要维护一份脆弱的忽略规则。
//!
//! 清单文件用 RON 而不是自定义文本格式：它可读、可进 Git、可被
//! 未来版本用 `serde(default)` 平滑扩展。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use scene_doc::error::{场景结果, 场景错误};
use scene_doc::path::{路径错误, 项目相对路径};
use scene_doc::property::资源类别;
use scene_doc::storage::{场景存储, 文件系统场景存储};

use crate::asset_uid::{标识表相对路径 as 标识表路径, 资源标识表};
use crate::cargo_project::{Cargo工程, Cargo识别错误};

/// 项目清单文件名。
pub const 项目清单文件名: &str = "project.ron";

/// 项目的来源类型。
///
/// # 为什么需要这一层（而不是"没有 project.ron 就失败"）
///
/// 一个原生 Bevy 项目的标准形态就是**普通 Cargo 工程**：没有 `project.ron`、
/// 没有 `scenes/`、场景由 Rust 代码构建。产品要求它必须能被正常打开，
/// 但**不能**因此把两种项目混为一谈 —— 很多行为恰好相反：
///
/// | 行为 | 微风项目 | 原生 Cargo 工程 |
/// | --- | --- | --- |
/// | 项目清单 | `project.ron` 是权威数据 | 由 `Cargo.toml` **只读**推导 |
/// | 保存设置 | 写回 `project.ron` | **不得**往用户工程里写任何东西 |
/// | 场景目录 | `scenes/` 里的场景文件 | 可能一个都没有（代码构建） |
/// | 运行目标 | 清单声明的程序 | Cargo 可执行目标（`cargo run`） |
///
/// 因此这里显式区分类型，而不是让"清单里恰好是空的"去暗示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 项目类型 {
    /// 微风自有格式：根目录有 `project.ron`。
    微风,
    /// 原生 Cargo / Bevy 工程：靠 `Cargo.toml` 识别，清单只存在于内存里。
    原生Cargo,
}

impl 项目类型 {
    /// 稳定 id（进日志与证据 JSON，不随语言变化）。
    pub fn id(self) -> &'static str {
        match self {
            Self::微风 => "breeze",
            Self::原生Cargo => "cargo",
        }
    }

    /// 名称词条键。
    pub fn 名称键(self) -> &'static str {
        match self {
            Self::微风 => "project_type_breeze",
            Self::原生Cargo => "project_type_cargo",
        }
    }

    /// 这个类型的项目清单是不是磁盘上的权威数据。
    ///
    /// 原生工程的清单是**每次打开时从 Cargo.toml 推导出来的快照**，
    /// 写回去就等于篡改用户的构建配置，因此必须为 `false`。
    pub fn 清单可写(self) -> bool {
        matches!(self, Self::微风)
    }
}

/// 项目清单的当前格式版本。
pub const 项目格式版本: u32 = 1;

/// 项目内约定的子目录名。
pub mod 目录名 {
    /// 原始资源目录。
    pub const 资源: &str = "assets";
    /// 场景文档目录。
    pub const 场景: &str = "scenes";
    /// 预制件模板目录。
    pub const 预制件: &str = "prefabs";
    /// 游戏源码目录。
    pub const 源码: &str = "src";
    /// 动画文档目录。
    pub const 动画: &str = "animations";
    /// 构建产物目录（相对项目根，供界面展示）。
    pub const 产物: &str = "target";
}

/// 项目的运行目标声明（`project.ron` 里的 `运行` 字段）。
///
/// # 为什么运行目标必须是项目契约，而不是编辑器的固定行为
///
/// 「运行项目」有两种完全不同的含义，编辑器**不可以**替用户猜：
///
/// | 目标 | 是什么 | 由谁决定 |
/// | --- | --- | --- |
/// | 独立预览进程 | 再启动一个微风实例打开同一项目 | 编辑器默认行为 |
/// | 真实游戏可执行程序 | 项目自己编译出的游戏 | **项目清单** |
///
/// 后者必须显式声明，否则编辑器会在用户以为"在跑游戏"的时候
/// 只是又开了一个编辑器窗口。因此清单里有一个可选的 `运行` 段：
/// 声明了就按声明执行，没声明就走默认的独立预览进程，
/// 并在界面上把目标类型写清楚（见 `editor_app::run::运行目标类型`）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct 运行目标清单 {
    /// 可执行程序。
    ///
    /// 三种写法都支持：
    /// * 绝对路径 —— 直接使用；
    /// * 含分隔符的相对路径 —— 相对**项目根目录**解析；
    /// * 不含分隔符的裸名字 —— 在 `PATH` 上查找。
    pub 程序: String,
    /// 命令行参数。
    #[serde(default)]
    pub 参数: Vec<String>,
    /// 追加给子进程的环境变量。
    #[serde(default)]
    pub 环境: BTreeMap<String, String>,
    /// 工作目录（项目相对路径）。留空表示项目根目录。
    #[serde(default)]
    pub 工作目录: Option<String>,
}

impl 运行目标清单 {
    /// 是否声明了运行目标。
    pub fn 已声明(&self) -> bool {
        !self.程序.trim().is_empty()
    }
}

/// 项目清单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 项目清单 {
    /// 清单格式版本。
    pub 格式版本: u32,
    /// 项目名称。
    pub 名称: String,
    /// 项目说明。
    #[serde(default)]
    pub 说明: String,
    /// 启动场景的项目相对路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 启动场景: Option<项目相对路径>,
    /// 项目自定义配置，供游戏代码与插件读取。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub 配置: BTreeMap<String, String>,
    /// 运行目标声明。`None` 表示该项目只能以"独立预览进程"方式运行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 运行: Option<运行目标清单>,
    /// 设计分辨率（游戏 UI 的布局空间，也是导出窗口的基准尺寸）。
    ///
    /// 与 Godot 的 `display/window/size/viewport_width/height` 同一个概念：
    /// 界面按这个尺寸布局，实际窗口与预览纹理的缩放只是显示缩放。
    /// 旧项目没有这一项时用 1920×1080（最常见的设计稿尺寸）。
    #[serde(default = "默认设计分辨率")]
    pub 设计分辨率: (u32, u32),
    /// 构建配置。`None` 表示用默认配置（`cargo build`）。
    ///
    /// 与 [`Self::运行`] 同一条原则：**构建怎么打是项目契约**，
    /// 不是编辑器全局偏好。同一个编辑器会打开多个项目，
    /// 每个项目的包名、特性、目标平台都可能不同。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 构建: Option<crate::build::构建配置>,
    /// 打包配置。`None` 表示用默认配置（输出到 `dist/`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 打包: Option<crate::package::打包配置>,
}

/// 默认设计分辨率。
///
/// 1080p 是最常见的设计稿尺寸。
pub const fn 默认设计分辨率() -> (u32, u32) {
    (1920, 1080)
}

impl 项目清单 {
    /// 以项目名创建一个新清单。
    pub fn 新建(名称: impl Into<String>) -> Self {
        Self {
            格式版本: 项目格式版本,
            名称: 名称.into(),
            说明: String::new(),
            启动场景: None,
            配置: BTreeMap::new(),
            运行: None,
            设计分辨率: 默认设计分辨率(),
            构建: None,
            打包: None,
        }
    }
}

/// 资源浏览器里的一个条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 资源条目 {
    /// 项目相对路径。
    pub 路径: 项目相对路径,
    /// 推断出的资源类别。
    pub 类别: 资源类别,
    /// 字节大小。
    pub 大小: u64,
    /// 文件最后修改时间；文件系统不提供时为空。
    ///
    /// 注册表用它判断"两次扫描之间文件是否变过"，界面不直接展示。
    pub 修改时间: Option<std::time::SystemTime>,
    /// 是否是目录。
    pub 是目录: bool,
}

/// 项目错误。
#[derive(Debug, thiserror::Error)]
pub enum 项目错误 {
    /// 目录非空，拒绝在其中创建项目。
    #[error("目录 `{0}` 非空，为避免覆盖已有文件，请在空目录中创建项目")]
    目录非空(PathBuf),

    /// 目标目录已存在同名项目。
    #[error("目录 `{0}` 中已存在 project.ron，无法重复创建项目")]
    项目已存在(PathBuf),

    /// 找不到项目清单。
    #[error("`{0}` 不是可识别的项目：既没有微风的 project.ron，也没有 Cargo.toml")]
    未找到项目(PathBuf),

    /// 项目清单的格式版本高于本编辑器。
    #[error("项目清单格式版本 {文件版本} 高于本编辑器支持的 {支持版本}，请升级微风")]
    版本过新 {
        /// 文件里的版本号。
        文件版本: u32,
        /// 本编辑器支持的版本号。
        支持版本: u32,
    },

    /// Cargo 工程识别失败（原生工程路径）。
    #[error(transparent)]
    Cargo工程(#[from] Cargo识别错误),

    /// 试图改写一个原生工程的清单（必须被拒绝）。
    #[error("`{0}` 是原生 Cargo 工程，微风不会写入它的 Cargo.toml 或创建 project.ron")]
    原生工程只读(PathBuf),

    /// 项目清单解析失败。
    #[error("项目清单解析失败：{} —— {来源}", 路径.display())]
    清单解析 {
        /// 清单路径。
        路径: PathBuf,
        /// 底层 RON 错误。
        #[source]
        来源: Box<ron::error::SpannedError>,
    },

    /// 路径非法。
    #[error(transparent)]
    路径(#[from] 路径错误),

    /// 场景层错误。
    #[error(transparent)]
    场景(#[from] 场景错误),

    /// 文件系统错误。
    #[error("文件操作失败：{} —— {来源}", 路径.display())]
    文件 {
        /// 出错路径。
        路径: PathBuf,
        /// 底层错误。
        #[source]
        来源: std::io::Error,
    },

    /// 项目设置不合法（保存时被拒绝，清单未被修改）。
    #[error("项目设置不合法：{0}")]
    设置非法(String),

    /// 运行入口失效（包/目标已不存在，或该包只有库目标）。
    #[error("运行入口不可用：{0}")]
    运行入口失效(#[from] crate::run_entry::入口失效原因),
}

impl 项目错误 {
    /// 构造一个文件错误。
    pub(crate) fn 文件错误(路径: impl Into<PathBuf>, 来源: std::io::Error) -> Self {
        Self::文件 {
            路径: 路径.into(),
            来源,
        }
    }
}

/// 项目操作结果。
pub type 项目结果<T> = Result<T, 项目错误>;

/// 一个已打开的项目。
#[derive(Debug, Clone)]
pub struct 项目描述 {
    /// 项目根目录（绝对路径）。
    根目录: PathBuf,
    /// 已解析的清单。
    清单: 项目清单,
    /// 项目来源类型。
    类型: 项目类型,
    /// 原生 Cargo 工程的识别结果（`类型 == 微风` 时为 `None`）。
    cargo: Option<Cargo工程>,
    /// 原生工程的识别来源（元数据 / 静态兜底）。微风项目为 `None`。
    识别来源: Option<crate::cargo_metadata::识别来源>,
    /// 元数据失败、回退静态解析时的**真实原因**（必须显示给用户）。
    识别提示: Option<String>,
    /// 当前选中的运行入口（原生工程）。
    ///
    /// 对微风项目没有意义（那种项目的运行目标写在 `project.ron` 里，
    /// 由 [`项目清单::运行`] 权威表达），因此恒为 `None`。
    运行入口: Option<crate::run_entry::运行入口>,
    /// 资源稳定标识表（路径 → UID）。
    ///
    /// 只在**微风项目**里落盘到 `.breeze/asset_uids.ron`；
    /// 原生 Cargo 工程可以在内存里维护，但不写文件（原因见
    /// [`crate::asset_uid`] 的模块说明）。
    资源标识: crate::asset_uid::资源标识表,
}

impl 项目描述 {
    /// 在 `根目录` 中**创建**一个新项目。
    ///
    /// 安全约束：只允许在**空目录**或**不存在**的目录里创建。
    /// 这样即使调用方传错路径，也不会把用户的既有文件覆盖掉。
    pub fn 创建(根目录: impl AsRef<Path>, 名称: impl Into<String>) -> 项目结果<Self> {
        let 根 = 绝对化(根目录.as_ref());

        if 根.exists() {
            if !根.is_dir() {
                return Err(项目错误::文件错误(
                    &根,
                    std::io::Error::new(std::io::ErrorKind::AlreadyExists, "路径不是目录"),
                ));
            }
            let 非空 = std::fs::read_dir(&根)
                .map_err(|错误| 项目错误::文件错误(&根, 错误))?
                .next()
                .is_some();
            if 非空 {
                if 根.join(项目清单文件名).is_file() {
                    return Err(项目错误::项目已存在(根));
                }
                return Err(项目错误::目录非空(根));
            }
        }

        let 名称 = 名称.into();
        let 描述 = Self {
            根目录: 根.clone(),
            清单: 项目清单::新建(名称),
            类型: 项目类型::微风,
            cargo: None,
            识别来源: None,
            识别提示: None,
            运行入口: None,
            资源标识: crate::asset_uid::资源标识表::default(),
        };

        // 建目录
        for 子目录 in [目录名::资源, 目录名::场景, 目录名::源码] {
            描述.建目录(子目录)?;
        }
        // 在 assets 下预置常用分类，减少用户第一次使用时的空目录感。
        for 子目录 in [
            format!("{}/textures", 目录名::资源),
            format!("{}/audio", 目录名::资源),
            format!("{}/fonts", 目录名::资源),
            format!("{}/models", 目录名::资源),
        ] {
            描述.建目录(&子目录)?;
        }

        描述.保存清单()?;
        Ok(描述)
    }

    /// 打开一个已有项目。
    ///
    /// # 两种项目，一条入口
    ///
    /// 产品要求"原生 Bevy 项目必须能直接打开"，但**不能**为此复制一套
    /// 互不兼容的项目管理。因此这里按真实工程结构分派：
    ///
    /// 1. 有 `project.ron` → 微风项目（格式版本校验、解析失败都会明确报错）；
    /// 2. 没有 `project.ron` 但有 `Cargo.toml` → 原生 Cargo / Bevy 工程，
    ///    清单由 Cargo 结构**只读推导**（见 [`crate::cargo_project`]）；
    /// 3. 两者都没有 → [`项目错误::未找到项目`]。
    ///
    /// 根目录一律**绝对化**：项目路径与构建工作目录必须是同一个字符串来源，
    /// 否则日志里的路径、cargo 的 `current_dir`、最近项目记录会三处不一致。
    pub fn 打开(根目录: impl AsRef<Path>) -> 项目结果<Self> {
        let 根 = 绝对化(根目录.as_ref());
        if 根.join(项目清单文件名).is_file() {
            return Self::打开微风(根);
        }
        if 根.join(crate::cargo_project::原生清单文件名).is_file() {
            return Self::打开原生工程(根);
        }
        Err(项目错误::未找到项目(根))
    }

    /// 打开一个微风项目（`project.ron` 是权威清单）。
    fn 打开微风(根: PathBuf) -> 项目结果<Self> {
        let 清单路径 = 根.join(项目清单文件名);

        let 文本 = std::fs::read_to_string(&清单路径)
            .map_err(|错误| 项目错误::文件错误(&清单路径, 错误))?;

        // 先探测版本，再完整解析 —— 与场景文档层同样的策略：
        // "版本过新"比"解析失败"对用户有用得多。
        if let Some(版本) = 探测清单版本(&文本)
            && 版本 > 项目格式版本
        {
            return Err(项目错误::版本过新 {
                文件版本: 版本,
                支持版本: 项目格式版本,
            });
        }

        let 清单: 项目清单 = ron::from_str(&文本).map_err(|来源| 项目错误::清单解析 {
            路径: 清单路径.clone(),
            来源: Box::new(来源),
        })?;

        if 清单.格式版本 > 项目格式版本 {
            return Err(项目错误::版本过新 {
                文件版本: 清单.格式版本,
                支持版本: 项目格式版本,
            });
        }

        // 标识表损坏时**不阻止打开项目**：场景与资源仍然可用，
        // 标识只影响"外部移动后的重连"。把错误留给调用方显示，
        // 而不是让用户对着一个打不开的工程。
        let 资源标识 = match crate::asset_uid::资源标识表::读取(&根.join(标识表路径)) {
            Ok(表) => 表,
            Err(错误) => {
                let 空表 = crate::asset_uid::资源标识表::default();
                // 用 eprintln 而不是日志框架：这一层不依赖日志设施，
                // 且这是必须在第一时间看见的告警。
                eprintln!("资源标识表读取失败，已按空表继续：{错误}");
                空表
            }
        };

        Ok(Self {
            根目录: 根,
            清单,
            类型: 项目类型::微风,
            cargo: None,
            识别来源: None,
            识别提示: None,
            运行入口: None,
            资源标识,
        })
    }

    /// 打开一个原生 Cargo / Bevy 工程。
    ///
    /// # 这里**不**写任何文件
    ///
    /// 不创建 `project.ron`、不改 `Cargo.toml`、不移动源码与资源。
    /// 清单只是一份内存快照，用来让构建、运行、项目名这些公共链路
    /// 不必为"两种项目"分叉。
    ///
    /// # 清单里放什么
    ///
    /// * 名称：根包名；虚拟工作区没有根包时用目录名；
    /// * 运行目标：默认 Cargo 可执行目标 → `cargo run -p <包> --bin <目标>`；
    ///   只有库目标时**不**声明运行目标（库不是游戏，给一个必然失败的入口
    ///   比不给更糟）；
    /// * 构建配置：固定包名，保证 workspace 里构建的是同一个包；
    /// * 启动场景：`None`（原生工程通常没有微风的场景文件）。
    fn 打开原生工程(根: PathBuf) -> 项目结果<Self> {
        // 元数据优先、静态解析兜底；失败原因留在 `识别提示` 里。
        let 结论 = crate::cargo_metadata::识别工程(&根)?;
        let 工程 = 结论.工程;
        let 名称 = 工程
            .默认包()
            .map(|包| 包.名称.clone())
            .or_else(|| {
                // 纯虚拟工作区（没有根包、成员全是库）也要有个可显示的名字。
                根.file_name()
                    .and_then(|名| 名.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Cargo 工程".to_string());

        let mut 清单 = 项目清单::新建(名称);
        清单.说明 = 工程.摘要();

        let 运行入口 = crate::run_entry::运行入口::默认(&工程);
        if let Some(目标) = 工程.默认可执行目标() {
            // 用 `cargo run` 而不是直接指向 `target/debug/xxx.exe`：
            // 后者在"还没构建过"时必然不存在，而用户点「运行项目」的语义
            // 是"跑我的游戏"，不是"跑上次的构建产物"。
            清单.运行 = Some(运行目标清单 {
                程序: "cargo".to_string(),
                参数: vec![
                    "run".to_string(),
                    "-p".to_string(),
                    目标.包名.clone(),
                    "--bin".to_string(),
                    目标.名称.clone(),
                ],
                环境: BTreeMap::new(),
                工作目录: None,
            });
            清单.构建 = Some(crate::build::构建配置 {
                包名: Some(目标.包名.clone()),
                二进制名: Some(目标.名称.clone()),
                ..crate::build::构建配置::default()
            });
        }

        Ok(Self {
            根目录: 根,
            清单,
            类型: 项目类型::原生Cargo,
            cargo: Some(工程),
            识别来源: Some(结论.来源),
            识别提示: 结论
                .回退原因
                .as_ref()
                .map(|错误| format!("{}：{}", 错误.代号(), 错误.说明())),
            运行入口,
            // 原生工程不写 `.breeze/`：它没有微风清单，也不该被塞一个。
            资源标识: crate::asset_uid::资源标识表::default(),
        })
    }

    /// 原生工程的识别来源（微风项目为 `None`）。
    pub fn 识别来源(&self) -> Option<crate::cargo_metadata::识别来源> {
        self.识别来源
    }

    /// 元数据失败、回退静态解析时的真实原因（没有回退时为 `None`）。
    pub fn 识别提示(&self) -> Option<&str> {
        self.识别提示.as_deref()
    }

    /// 当前运行入口（只有原生 Cargo 工程有）。
    pub fn 运行入口(&self) -> Option<&crate::run_entry::运行入口> {
        self.运行入口.as_ref()
    }

    /// 选择一个运行入口。
    ///
    /// # 校验在前，写入在后
    ///
    /// 传入的入口必须真的存在于当前 `Cargo.toml` 里，否则返回
    /// [`项目错误::运行入口失效`]。这里**不做静默回退**：把 `server`
    /// 悄悄换成 `my_game` 会让用户对着一个跑错的程序排查半天。
    ///
    /// 写入只改**内存里的清单快照**（`运行` + `构建` 两个段），
    /// 原生工程的 `Cargo.toml` 一个字节都不动。
    pub fn 设置运行入口(
        &mut self, 入口: crate::run_entry::运行入口
    ) -> 项目结果<()> {
        if !self.类型.清单可写() {
            let 工程 = self
                .cargo
                .as_ref()
                .ok_or_else(|| 项目错误::原生工程只读(self.根目录.clone()))?;
            入口.校验(工程)?;
        }
        self.清单.运行 = Some(运行目标清单 {
            程序: "cargo".to_string(),
            参数: 入口.命令行参数(),
            环境: BTreeMap::new(),
            工作目录: None,
        });
        let 原构建 = self.清单.构建.clone().unwrap_or_default();
        self.清单.构建 = Some(crate::build::构建配置 {
            包名: Some(入口.包名.clone()),
            二进制名: Some(入口.目标名.clone()),
            配置类型: 入口.配置类型,
            子命令: 原构建.子命令,
            目标平台: 原构建.目标平台,
            特征: 原构建.特征,
            额外参数: 原构建.额外参数,
        });
        self.运行入口 = Some(入口);
        // 原生工程的清单只读：需要落盘时调用方必须先判断类型。
        if self.类型.清单可写() {
            self.保存清单()?;
        }
        Ok(())
    }

    /// 重新识别原生 Cargo 工程（`Cargo.toml` 变化后用）。
    ///
    /// 返回当前运行入口的失效原因（如果有）。失效时入口会回退到默认目标，
    /// 但**必须**把原因告诉用户 —— 见 [`crate::run_entry::入口失效原因::说明`]。
    pub fn 重新识别(&mut self) -> 项目结果<Option<crate::run_entry::入口失效原因>> {
        if self.类型 != 项目类型::原生Cargo {
            return Ok(None);
        }
        // 缓存由指纹驱动；显式失效一次，避免 mtime 精度导致读到旧结果。
        crate::cargo_metadata::清除工程缓存();
        let 结论 = crate::cargo_metadata::识别工程(&self.根目录)?;
        self.cargo = Some(结论.工程);
        self.识别来源 = Some(结论.来源);
        self.识别提示 = 结论
            .回退原因
            .as_ref()
            .map(|错误| format!("{}：{}", 错误.代号(), 错误.说明()));

        let 工程 = self.cargo.as_ref().expect("刚刚写入");
        let 失效 = match &self.运行入口 {
            Some(入口) => 入口.校验(工程).err(),
            None => None,
        };
        if 失效.is_some() || self.运行入口.is_none() {
            let 新入口 = crate::run_entry::运行入口::默认(工程);
            self.运行入口 = 新入口.clone();
            match 新入口 {
                Some(入口) => {
                    self.清单.运行 = Some(运行目标清单 {
                        程序: "cargo".to_string(),
                        参数: 入口.命令行参数(),
                        环境: BTreeMap::new(),
                        工作目录: None,
                    });
                    let 原构建 = self.清单.构建.clone().unwrap_or_default();
                    self.清单.构建 = Some(crate::build::构建配置 {
                        包名: Some(入口.包名.clone()),
                        二进制名: Some(入口.目标名.clone()),
                        配置类型: 入口.配置类型,
                        ..原构建
                    });
                }
                None => {
                    // 工程已经没有任何可运行目标（改成纯库了）：
                    // 必须撤掉运行声明，而不是留一个必然失败的入口。
                    self.清单.运行 = None;
                }
            }
        }
        Ok(失效)
    }

    /// 项目来源类型。
    pub fn 类型(&self) -> 项目类型 {
        self.类型
    }

    /// 是否微风自有格式的项目。
    pub fn 是微风项目(&self) -> bool {
        self.类型 == 项目类型::微风
    }

    /// 原生 Cargo 工程的识别结果（微风项目为 `None`）。
    pub fn cargo_工程(&self) -> Option<&Cargo工程> {
        self.cargo.as_ref()
    }

    /// 构建时必须使用的 `Cargo.toml` 所在目录。
    ///
    /// 与 [`Self::根目录`] **始终一致**：原生工程里根目录就是 workspace 根，
    /// 成员包的选择通过 `-p` 完成，而不是换工作目录 —— 换工作目录会让
    /// `target/`、相对资源路径、诊断里的文件路径全部漂移。
    pub fn 构建根目录(&self) -> PathBuf {
        self.根目录.clone()
    }

    /// 项目根目录。
    pub fn 根目录(&self) -> &Path {
        &self.根目录
    }

    /// 项目清单。
    pub fn 清单(&self) -> &项目清单 {
        &self.清单
    }

    /// 项目清单（可变）。修改后需调用 [`项目描述::保存清单`] 落盘。
    pub fn 清单可变(&mut self) -> &mut 项目清单 {
        &mut self.清单
    }

    /// 项目名。
    pub fn 名称(&self) -> &str {
        &self.清单.名称
    }

    /// 清单文件路径。
    pub fn 清单路径(&self) -> PathBuf {
        self.根目录.join(项目清单文件名)
    }

    /// 某个分区目录的绝对路径。
    pub fn 分区目录(&self, 子目录: &str) -> PathBuf {
        self.根目录.join(子目录)
    }

    /// 把项目相对路径解析为绝对路径。
    pub fn 绝对路径(&self, 相对: &项目相对路径) -> PathBuf {
        相对.拼接(&self.根目录)
    }

    /// 把一个绝对路径转换为项目相对路径。
    ///
    /// 若路径不在项目根目录内，返回 `None` —— 这是"绝对路径污染"
    /// 的拦截点：项目外的文件不允许进入场景引用。
    pub fn 相对路径(&self, 绝对: &Path) -> Option<项目相对路径> {
        let 相对部分 = 绝对.strip_prefix(&self.根目录).ok()?;
        let 文本 = 相对部分.to_string_lossy().replace('\\', "/");
        if 文本.is_empty() {
            return None;
        }
        项目相对路径::解析(&文本).ok()
    }

    /// 创建一个目录（相对项目根）。
    pub fn 建目录(&self, 相对: &str) -> 项目结果<PathBuf> {
        let 绝对 = self
            .根目录
            .join(相对.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))?;
        Ok(绝对)
    }

    /// 保存项目清单。
    pub fn 保存清单(&self) -> 项目结果<()> {
        // 原生工程：清单是从 `Cargo.toml` 推导出来的**只读**快照。
        // 写它就等于替用户改构建配置（还会凭空造出一个 project.ron，
        // 下次打开时项目类型就会从"原生"变成"微风"）。这里必须拒绝，
        // 而不是"悄悄不写" —— 静默失败会让用户以为自己改的设置生效了。
        if !self.类型.清单可写() {
            return Err(项目错误::原生工程只读(self.根目录.clone()));
        }
        let 清单路径 = self.清单路径();
        let 文本 = ron::ser::to_string_pretty(
            &self.清单,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| 项目错误::文件错误(&清单路径, std::io::Error::other(错误)))?;

        std::fs::create_dir_all(&self.根目录)
            .map_err(|错误| 项目错误::文件错误(&self.根目录, 错误))?;
        std::fs::write(&清单路径, 文本)
            .map_err(|错误| 项目错误::文件错误(&清单路径, 错误))?;
        Ok(())
    }

    /// 切换项目名并落盘。
    ///
    /// 原生 Cargo 工程的名称来自 `Cargo.toml`，改名要改的是那份清单 ——
    /// 微风不会替用户改，因此这里会返回 [`项目错误::原生工程只读`]。
    pub fn 设名称(&mut self, 名称: impl Into<String>) -> 项目结果<()> {
        if !self.类型.清单可写() {
            return Err(项目错误::原生工程只读(self.根目录.clone()));
        }
        self.清单.名称 = 名称.into();
        self.保存清单()
    }

    /// 场景存储。
    pub fn 场景存储(&self) -> 文件系统场景存储 {
        文件系统场景存储::新建()
    }

    /// 一个新的场景文件路径（位于 `scenes/` 下，扩展名为 `.scene.ron`）。
    pub fn 场景路径(&self, 场景名: &str) -> 项目结果<项目相对路径> {
        let 清理后 = 清理文件名(场景名);
        let 路径 = format!("{}/{清理后}.scene.ron", 目录名::场景);
        Ok(项目相对路径::解析(路径)?)
    }

    /// 扫描场景目录，返回所有场景文件的项目相对路径（按路径排序）。
    pub fn 场景列表(&self) -> Vec<项目相对路径> {
        self.扫描目录(目录名::场景)
            .into_iter()
            .map(|条目| 条目.路径)
            .filter(|路径| {
                路径
                    .扩展名()
                    .is_some_and(|扩展| 扩展 == "ron" || 扩展 == "scene")
            })
            .collect()
    }

    /// 扫描资源目录，返回所有资源条目（按路径排序）。
    pub fn 资源列表(&self) -> Vec<资源条目> {
        self.扫描目录(目录名::资源)
    }

    /**
     * 扫描预制件目录，返回所有 `.ron` 模板的项目相对路径（按路径排序）。
     *
     * 只按扩展名过滤，不按文件名里的 `.prefab` 后缀：目录本身就是
     * `prefabs/`，再要求文件名格式只会无端拒绝用户自建的文件。
     */
    pub fn 预制件列表(&self) -> Vec<项目相对路径> {
        self.扫描目录(目录名::预制件)
            .into_iter()
            .map(|条目| 条目.路径)
            .filter(|路径| 路径.扩展名().is_some_and(|扩展| 扩展 == "ron"))
            .collect()
    }

    /// 项目里的 Rust 源文件（项目相对路径，按路径排序）。
    ///
    /// # 为什么单独一个方法，而不是塞进资源列表
    ///
    /// `assets/` 与 `src/` 是**两种不同的东西**：前者进资源浏览器与导入清单，
    /// 后者进代码编辑器。混成一个列表，资源浏览器里会冒出 `.rs` 文件
    /// （类别只能标成"其他"），导入清单也会试图"导入"源码。
    ///
    /// 原生 Cargo 工程的源码可能分布在多个成员包里，因此这里同时扫描
    /// 根目录的 `src/` 与每个成员包的 `src/`（按项目相对路径去重）。
    pub fn 源文件列表(&self) -> Vec<项目相对路径> {
        let mut 起点: Vec<PathBuf> = vec![self.根目录.join(目录名::源码)];
        if let Some(工程) = &self.cargo {
            for 包 in &工程.包 {
                let 源码目录 = 包.根目录.join(目录名::源码);
                if !起点.contains(&源码目录) {
                    起点.push(源码目录);
                }
            }
        }

        let mut 结果: Vec<项目相对路径> = Vec::new();
        for 目录 in 起点 {
            if !目录.is_dir() {
                continue;
            }
            let mut 栈 = vec![目录];
            while let Some(当前) = 栈.pop() {
                let Ok(读取) = std::fs::read_dir(&当前) else {
                    continue;
                };
                for 项 in 读取.flatten() {
                    let 路径 = 项.path();
                    let Ok(元数据) = 项.metadata() else {
                        continue;
                    };
                    if 元数据.is_dir() {
                        栈.push(路径);
                        continue;
                    }
                    if 路径.extension().is_none_or(|扩展| 扩展 != "rs") {
                        continue;
                    }
                    let Some(相对) = self.相对路径(&路径) else {
                        continue;
                    };
                    if !结果.contains(&相对) {
                        结果.push(相对);
                    }
                }
            }
        }
        结果.sort();
        结果
    }

    /// 递归扫描项目内某个分区目录。
    ///
    /// 实现要点：
    /// * 使用显式栈而不是递归，避免深层目录导致栈溢出；
    /// * 遇到无法读取的子目录时跳过而不是整体失败 ——
    ///   资源浏览器不该因为一个被占用的目录就整个打不开；
    /// * 结果按路径排序，保证 UI 顺序稳定（不随文件系统枚举顺序抖动）。
    pub fn 扫描目录(&self, 分区: &str) -> Vec<资源条目> {
        let 起点 = self.根目录.join(分区);
        let mut 结果 = Vec::new();
        if !起点.is_dir() {
            return 结果;
        }

        let mut 栈 = vec![起点];
        while let Some(当前) = 栈.pop() {
            let Ok(读取) = std::fs::read_dir(&当前) else {
                continue;
            };
            for 项 in 读取.flatten() {
                let 路径 = 项.path();
                let Ok(元数据) = 项.metadata() else {
                    continue;
                };

                if 元数据.is_dir() {
                    栈.push(路径);
                    continue;
                }
                if !元数据.is_file() {
                    continue;
                }

                let Some(相对) = self.相对路径(&路径) else {
                    continue;
                };
                let 类别 = 相对
                    .扩展名()
                    .map(|扩展| 资源类别::由扩展名推断(&扩展))
                    .unwrap_or(资源类别::其他);
                结果.push(资源条目 {
                    路径: 相对,
                    类别,
                    大小: 元数据.len(),
                    修改时间: 元数据.modified().ok(),
                    是目录: false,
                });
            }
        }

        结果.sort_by(|左, 右| 左.路径.cmp(&右.路径));
        结果
    }

    /// 在资源目录中新建一个文件夹。
    pub fn 新建资源文件夹(&self, 相对: &项目相对路径) -> 项目结果<()> {
        let 绝对 = self.绝对路径(相对);
        // 安全检查：解析后的路径必须仍在项目内。
        if !绝对.starts_with(&self.根目录) {
            return Err(项目错误::路径(路径错误::目录穿越(
                相对.文本().to_string(),
            )));
        }
        std::fs::create_dir_all(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))?;
        Ok(())
    }

    /// 重命名或移动一个资源，并返回**所有需要同步更新的引用路径**。
    ///
    /// 本方法只做文件系统操作，**不**改场景。
    /// 调用方拿到 [`资源移动结果`] 后必须据此更新打开中的场景文档，
    /// 否则场景里的引用就会断掉 —— 这正是需求里
    /// "不得随意移动或重命名文件后破坏场景中的资源引用"的实现方式：
    /// 把"移动文件"和"更新引用"绑成一个不可分割的操作，由上层一起提交。
    pub fn 移动资源(
        &self,
        旧路径: &项目相对路径,
        新路径: &项目相对路径,
    ) -> 项目结果<资源移动结果> {
        let 旧绝对 = self.绝对路径(旧路径);
        let 新绝对 = self.绝对路径(新路径);

        if !旧绝对.exists() {
            return Err(项目错误::文件错误(
                &旧绝对,
                std::io::Error::new(std::io::ErrorKind::NotFound, "源文件不存在"),
            ));
        }
        if 新绝对.exists() {
            return Err(项目错误::文件错误(
                &新绝对,
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, "目标路径已存在"),
            ));
        }
        if !新绝对.starts_with(&self.根目录) {
            return Err(项目错误::路径(路径错误::目录穿越(
                新路径.文本().to_string(),
            )));
        }

        if let Some(父目录) = 新绝对.parent() {
            std::fs::create_dir_all(父目录)
                .map_err(|错误| 项目错误::文件错误(父目录, 错误))?;
        }

        std::fs::rename(&旧绝对, &新绝对)
            .map_err(|错误| 项目错误::文件错误(&旧绝对, 错误))?;

        Ok(资源移动结果 {
            旧路径: 旧路径.clone(),
            新路径: 新路径.clone(),
        })
    }

    /// 删除一个资源文件。
    pub fn 删除资源(&self, 相对: &项目相对路径) -> 项目结果<()> {
        let 绝对 = self.绝对路径(相对);
        if !绝对.starts_with(&self.根目录) {
            return Err(项目错误::路径(路径错误::目录穿越(
                相对.文本().to_string(),
            )));
        }
        std::fs::remove_file(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))?;
        Ok(())
    }

    /// 读取一个资源文件的原始字节（供导入器使用）。
    pub fn 读取资源(&self, 相对: &项目相对路径) -> 项目结果<Vec<u8>> {
        let 绝对 = self.绝对路径(相对);
        std::fs::read(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))
    }

    /// 把一份场景写入项目。
    pub fn 保存场景(
        &self,
        场景路径: &项目相对路径,
        文档: &scene_doc::document::场景文档,
    ) -> 场景结果<()> {
        let 绝对 = self.绝对路径(场景路径);
        self.场景存储().写入(&绝对, 文档)?;
        Ok(())
    }

    /// 从项目读入一份场景。
    pub fn 读取场景(
        &self,
        场景路径: &项目相对路径,
    ) -> 场景结果<scene_doc::storage::读取结果> {
        let 绝对 = self.绝对路径(场景路径);
        self.场景存储().读取(&绝对)
    }

    /**
     * 从项目读入一份预制件模板。
     *
     * 读取失败与解析失败都保持原始错误类型：模板目录的加载器需要区分
     * "文件读不了"和"文件读到了但内容坏了"，前者可以提示权限/占用，
     * 后者才是用户需要修模板内容的信号。
     */
    pub fn 读取预制件(
        &self,
        路径: &项目相对路径,
    ) -> 场景结果<scene_doc::prefab::预制件文档> {
        let 绝对 = self.绝对路径(路径);
        let 文本 = std::fs::read_to_string(&绝对).map_err(|来源| 场景错误::文件读写 {
            路径: 绝对.clone(),
            来源,
        })?;
        scene_doc::storage::解码预制件(&文本, &绝对)
    }

    /**
     * 把一份预制件模板写入项目。
     *
     * 与 [`项目描述::保存场景`] 同一形状：编码统一走 `scene_doc::storage`，
     * 保证模板文件与场景文件用同一套美化配置（要进 Git，一行一字段）。
     * 只负责"落盘"这一个动作：版本推进与指纹重算由调用方在传入之前
     * 完成，本函数不替调用方决定模板语义。
     */
    pub fn 保存预制件(
        &self,
        路径: &项目相对路径,
        模板: &scene_doc::prefab::预制件文档,
    ) -> 场景结果<()> {
        let 绝对 = self.绝对路径(路径);
        let 文本 = scene_doc::storage::编码预制件(模板)?;
        std::fs::write(&绝对, 文本).map_err(|来源| 场景错误::文件读写 {
            路径: 绝对.clone(),
            来源,
        })?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // 资源导入状态与依赖检查
    // ------------------------------------------------------------------

    /// 导入清单的绝对路径。
    pub fn 导入清单路径(&self) -> PathBuf {
        self.根目录.join(导入清单文件名)
    }

    // ------------------------------------------------------------------
    // 资源稳定标识（UID）
    // ------------------------------------------------------------------

    /// 标识表在磁盘上的绝对路径。
    pub fn 资源标识路径(&self) -> PathBuf {
        self.根目录.join(标识表路径)
    }

    /// 只读的标识表。
    pub fn 资源标识(&self) -> &资源标识表 {
        &self.资源标识
    }

    /// 可变的标识表（调用方改完要自己调用 [`项目描述::保存资源标识`]）。
    pub fn 资源标识可变(&mut self) -> &mut 资源标识表 {
        &mut self.资源标识
    }

    /// 标识表是否可以落盘。
    ///
    /// 只有微风项目写 `.breeze/asset_uids.ron`；原生 Cargo 工程不写，
    /// 因为那会给一个本来干净的原生工程凭空造出编辑器私有目录。
    pub fn 资源标识可落盘(&self) -> bool {
        self.类型 == 项目类型::微风
    }

    /// 保存标识表。原生工程直接返回 `Ok(())`（不写文件）。
    pub fn 保存资源标识(&self) -> 项目结果<()> {
        if !self.资源标识可落盘() {
            return Ok(());
        }
        self.资源标识.保存(&self.资源标识路径())
    }

    /// 与磁盘上的资源文件同步标识表，并在可落盘时写回。
    ///
    /// 返回 `(新增, 移除)`。这是"新导入的文件自动获得 UID、
    /// 已删除文件的 UID 被清理"的唯一入口。
    pub fn 同步资源标识(&mut self) -> 项目结果<(usize, usize)> {
        let 资源: Vec<项目相对路径> = self
            .资源列表()
            .into_iter()
            .filter(|条目| !条目.是目录)
            .map(|条目| 条目.路径)
            .collect();
        let 变化 = self.资源标识.同步(&资源);
        self.保存资源标识()?;
        Ok(变化)
    }

    /// 把文档里记录的标识补进标识表，并把表里的标识写回文档。
    ///
    /// # 为什么要双向
    ///
    /// * 文档 → 表：旧场景里已经记录过 `唯一标识` 的资源，
    ///   不该在第一次同步时被换成新 UUID；
    /// * 表 → 文档：保存场景时把标识真正写进文件，
    ///   下次在文件管理器里移动资源时才有东西可以按 UID 回连。
    ///
    /// 返回补进表的条数。调用方负责随后保存表与场景。
    pub fn 对齐文档标识(&mut self, 文档: &mut scene_doc::document::场景文档) -> usize {
        let mut 补入 = 0;
        for (路径, 引用) in 文档.资源表.iter_mut() {
            match 引用.唯一标识 {
                Some(已有) => {
                    let 文本 = 路径.文本().to_string();
                    if let std::collections::btree_map::Entry::Vacant(空位) =
                        self.资源标识.标识.entry(文本)
                    {
                        空位.insert(已有.0);
                        补入 += 1;
                    }
                }
                None => {
                    let (标识, _新建) = self.资源标识.确保(路径);
                    引用.唯一标识 = Some(scene_doc::path::资源唯一标识(标识));
                    // 计数要覆盖**两个方向**：
                    // * 表里新建了条目（`新建` 为真）；
                    // * 或者表里早就有、只是文档里没写（例如打开项目时
                    //   同步已经给磁盘上的每个资源发了 UID）。
                    // 只数前者会让"文档补齐了 UID"这种情况返回 0，
                    // 于是打开场景时既不提示、也不标脏 —— 补了等于没补。
                    补入 += 1;
                }
            }
        }
        补入
    }

    /// 按 UID 修复一份文档里"路径已失效、但资源还在别处"的引用。
    ///
    /// 这是"用户在系统文件管理器里移动了资源"的补救路径：
    /// 路径对不上时，用文档里记录的 UID 去标识表里找它现在在哪。
    ///
    /// 只在**路径确实不存在**且 **UID 能唯一确定一个现存路径**时才改；
    /// 冲突（同一 UID 多个路径）或找不到时原样保留，由调用方报告。
    pub fn 按标识修复文档(
        &self,
        文档: &mut scene_doc::document::场景文档,
    ) -> crate::asset_uid::修复报告 {
        let mut 报告 = crate::asset_uid::修复报告::default();
        // 先收集要改的，再统一走 `改资源路径`：后者会同时改资源表与
        // 组件里内嵌的引用值，避免出现"表改了、Sprite 还指向旧路径"。
        let 冲突 = self.资源标识.冲突标识();
        let mut 待改: Vec<(项目相对路径, 项目相对路径)> = Vec::new();
        for (路径, 引用) in &文档.资源表 {
            if self.绝对路径(路径).is_file() {
                continue;
            }
            let Some(标识) = 引用.唯一标识 else {
                报告.无标识缺失.push(路径.clone());
                continue;
            };
            let Some(现状) = self.资源标识.按标识查路径(&标识.0) else {
                if 冲突.contains_key(&标识.0) {
                    报告.冲突.push(路径.clone());
                } else {
                    报告.无法找回.push(路径.clone());
                }
                continue;
            };
            let Ok(新路径) = 项目相对路径::解析(现状) else {
                报告.无法找回.push(路径.clone());
                continue;
            };
            if !self.绝对路径(&新路径).is_file() {
                报告.无法找回.push(路径.clone());
                continue;
            }
            待改.push((路径.clone(), 新路径));
        }
        for (旧, 新) in 待改 {
            文档.改资源路径(&旧, &新);
            报告.已修复.push((旧, 新));
        }
        报告
    }

    /// 读取导入清单。文件不存在或损坏时返回空清单。
    ///
    /// 与场景文件相反：导入清单**损坏不报错**。它只是缓存索引，
    /// 丢掉最多重扫一遍；让一个坏缓存挡住整个项目是本末倒置。
    /// 版本过新时同样退回空清单（并保留原文件不覆盖，等用户升级）。
    pub fn 读取导入清单(&self) -> 资源导入清单 {
        let 路径 = self.导入清单路径();
        let Ok(文本) = std::fs::read_to_string(&路径) else {
            return 资源导入清单::default();
        };
        match ron::from_str::<资源导入清单>(&文本) {
            Ok(清单) if 清单.格式版本 <= 导入格式版本 => 清单,
            Ok(清单) => {
                // 版本过新：不覆盖别人的文件，只是本次不认它。
                let _ = 清单;
                资源导入清单::default()
            }
            Err(_) => 资源导入清单::default(),
        }
    }

    /// 保存导入清单。
    pub fn 保存导入清单(&self, 清单: &资源导入清单) -> 项目结果<()> {
        let 路径 = self.导入清单路径();
        let 文本 = ron::ser::to_string_pretty(
            清单,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| 项目错误::文件错误(&路径, std::io::Error::other(错误)))?;
        // 原子写：导入清单被踩坏会让所有资源显示成"新增"，
        // 虽然不致命，但用户会以为项目坏了。
        let 临时 = 路径.with_extension("ron.tmp");
        std::fs::write(&临时, 文本).map_err(|错误| 项目错误::文件错误(&临时, 错误))?;
        std::fs::rename(&临时, &路径).map_err(|错误| 项目错误::文件错误(&路径, 错误))?;
        Ok(())
    }

    /// 扫描资源目录并与导入清单对比，产出报告（**不写盘**）。
    ///
    /// 这是"资源浏览器里的状态圆点"与"导入按钮是否可用"的唯一数据源：
    /// 扫描一次、对比一次，UI 与动作读同一份结论，不会各说各话。
    pub fn 扫描导入(&self, 清单: &资源导入清单) -> 导入报告 {
        let mut 项 = Vec::new();
        let 磁盘列表 = self.资源列表();
        for 资源 in &磁盘列表 {
            if 资源.是目录 {
                continue;
            }
            let 记录 = 导入记录::从文件(资源.路径.clone(), &资源.路径.拼接(&self.根目录));
            let 状态 = match 清单.取(&资源.路径) {
                Some(旧) if 旧.指纹 == 记录.指纹 => 导入状态::最新,
                Some(_) => 导入状态::已变更,
                None => 导入状态::新增,
            };
            项.push(导入项 {
                路径: 资源.路径.clone(),
                类别: 资源.类别,
                状态,
                大小: 资源.大小,
            });
        }
        // 清单里有、磁盘上没有 = 被删掉的资源。
        for (键, 旧) in &清单.记录 {
            if 磁盘列表.iter().any(|资源| 资源.路径.文本() == 键) {
                continue;
            }
            // 目录也可能在清单里出现过（早期版本），只在文件确实不存在时报告。
            if self.绝对路径(&旧.路径).exists() {
                continue;
            }
            项.push(导入项 {
                路径: 旧.路径.clone(),
                类别: 资源类别::其他,
                状态: 导入状态::缺失,
                大小: 旧.大小,
            });
        }
        项.sort_by(|甲, 乙| 甲.路径.文本().cmp(乙.路径.文本()));
        导入报告 { 项 }
    }

    /// 把报告里的"新增/最新/已变更"写进清单，并**移除缺失项**。
    ///
    /// 缺失项被移除是刻意的：清单只记录"当前项目里真实存在的资源"。
    /// 保留缺失项会让"这个资源被删了"永远显示成错误状态，
    /// 用户删完资源后每次扫描都看到旧噪声。
    pub fn 更新导入清单(&self, 清单: &mut 资源导入清单, 报告: &导入报告) {
        for 项 in &报告.项 {
            match 项.状态 {
                导入状态::缺失 => {
                    清单.记录.remove(项.路径.文本());
                }
                状态 => {
                    let _ = 状态;
                    let 记录 =
                        导入记录::从文件(项.路径.clone(), &项.路径.拼接(&self.根目录));
                    清单.记录.insert(项.路径.文本().to_string(), 记录);
                }
            }
        }
    }

    /// 检查所有场景文件里的资源引用是否都能在磁盘上找到。
    ///
    /// # 为什么扫全部场景而不是只扫当前场景
    ///
    /// 用户准备打包时才关心"有没有丢资源"，那时他并不一定打开着出问题的场景。
    /// 逐场景读文件的开销很小（RON 解析几百 KB 级），换来一次点击就能
    /// 列出全项目的断链引用 —— 这是 Godot 的"导出前依赖检查"同一用途。
    ///
    /// 读不出来的场景会被跳过并计数，由调用方决定怎么提示；
    /// 这里不把它们算成"缺失引用"，因为那是另一种问题。
    pub fn 检查资源引用(&self) -> Vec<缺失引用> {
        let 存储 = self.场景存储();
        let mut 缺失 = Vec::new();
        for 场景路径 in self.场景列表() {
            let 绝对 = self.绝对路径(&场景路径);
            let Ok(读取) = 存储.读取(&绝对) else {
                continue;
            };
            let 文档 = &读取.文档;
            for 路径 in 文档.资源表.keys() {
                let 资源绝对 = self.绝对路径(路径);
                if !资源绝对.is_file() {
                    缺失.push(缺失引用 {
                        场景: 场景路径.clone(),
                        资源: 路径.clone(),
                    });
                }
            }
        }
        缺失.sort_by(|甲, 乙| {
            甲.场景
                .文本()
                .cmp(乙.场景.文本())
                .then_with(|| 甲.资源.文本().cmp(乙.资源.文本()))
        });
        缺失.dedup();
        缺失
    }

    // ------------------------------------------------------------------
    // 项目设置（Project Settings）
    // ------------------------------------------------------------------

    /// 把当前清单投影成"可编辑设置视图"。
    ///
    /// # 为什么要一个独立的视图类型
    ///
    /// 设置对话框需要**可变的文本字段**（`String`），而且用户可能中途取消；
    /// 若对话框直接改清单，取消就得靠"先备份再还原"，很容易漏字段。
    /// 视图是纯数据：对话框随便改，只有点"保存"时整体校验后写回清单。
    /// 与 Godot 的 `ProjectSettings` 对话框同一思路（改动到最后才落盘）。
    pub fn 设置视图(&self) -> 项目设置 {
        let 运行 = self.清单.运行.clone().unwrap_or_default();
        项目设置 {
            名称: self.清单.名称.clone(),
            说明: self.清单.说明.clone(),
            启动场景: self
                .清单
                .启动场景
                .as_ref()
                .map(|路径| 路径.文本().to_string()),
            运行程序: 运行.程序.clone(),
            运行参数: 运行.参数.clone(),
            运行工作目录: 运行.工作目录.clone(),
            构建: self.清单.构建.clone().unwrap_or_default(),
            设计分辨率: self.清单.设计分辨率,
            打包: self.清单.打包.clone().unwrap_or_default(),
        }
    }

    /// 校验并写回设置。**任何一项校验失败都不改清单**。
    ///
    /// 校验规则：
    /// * 项目名不能为空（它同时是窗口标题与目录提示里的名字）；
    /// * 启动场景必须能在项目内找到 —— 一个指向不存在文件的启动配置
    ///   会让"打开项目"每次都失败，必须在保存时就挡住；
    /// * 运行程序留空 = 移除运行目标声明（退回独立预览进程）；
    /// * 运行程序若是**含分隔符的相对路径**，必须在项目内存在。
    ///   裸名字（如 `cargo`）按 PATH 查找，无法在保存时校验，放行。
    pub fn 应用设置(&mut self, 设置: &项目设置) -> 项目结果<()> {
        // 原生工程的可编辑设置来自 `Cargo.toml`。这里的校验即使全过，
        // 也**没有地方落盘**（不允许写 project.ron），所以必须在改动内存
        // 清单之前就拒绝，否则用户会看到一个"保存成功过一帧又变回去"的
        // 编辑体验。
        if !self.类型.清单可写() {
            return Err(项目错误::原生工程只读(self.根目录.clone()));
        }
        let 名称 = 设置.名称.trim();
        if 名称.is_empty() {
            return Err(项目错误::设置非法("项目名不能为空".to_string()));
        }

        let 启动场景 = match 设置
            .启动场景
            .as_deref()
            .map(str::trim)
            .filter(|文本| !文本.is_empty())
        {
            Some(文本) => {
                let 路径 = 项目相对路径::解析(文本)?;
                let 绝对 = self.绝对路径(&路径);
                if !绝对.is_file() {
                    return Err(项目错误::设置非法(format!(
                        "启动场景不存在：{}",
                        路径.文本()
                    )));
                }
                Some(路径)
            }
            None => None,
        };

        let 运行程序 = 设置.运行程序.trim();
        let 运行 = if 运行程序.is_empty() {
            None
        } else {
            if 运行程序.contains('/') || 运行程序.contains('\\') {
                let 绝对 = self.根目录.join(运行程序);
                if !绝对.is_file() {
                    return Err(项目错误::设置非法(format!(
                        "运行目标不存在：{运行程序}（若它是 PATH 上的程序，请只写程序名）"
                    )));
                }
            }
            Some(运行目标清单 {
                程序: 运行程序.to_string(),
                参数: 设置
                    .运行参数
                    .iter()
                    .map(|项| 项.trim().to_string())
                    .filter(|项| !项.is_empty())
                    .collect(),
                环境: BTreeMap::new(),
                工作目录: 设置
                    .运行工作目录
                    .as_deref()
                    .map(str::trim)
                    .filter(|文本| !文本.is_empty())
                    .map(str::to_string),
            })
        };

        self.清单.名称 = 名称.to_string();
        self.清单.说明 = 设置.说明.trim().to_string();
        self.清单.启动场景 = 启动场景;
        self.清单.运行 = 运行;
        // 设计分辨率必须为正：0 宽的布局空间会让所有百分比锚点退化到 0。
        if 设置.设计分辨率.0 == 0 || 设置.设计分辨率.1 == 0 {
            return Err(项目错误::设置非法("设计分辨率不能为 0".to_string()));
        }
        if 设置.设计分辨率.0 > 16384 || 设置.设计分辨率.1 > 16384 {
            return Err(项目错误::设置非法(
                "设计分辨率过大（上限 16384）".to_string(),
            ));
        }
        self.清单.设计分辨率 = 设置.设计分辨率;
        // 打包输出目录会被清空后重写：必须先确认它落在项目内。
        设置.打包.输出目录合法().map_err(项目错误::设置非法)?;
        self.清单.打包 = Some(设置.打包.clone());
        self.清单.构建 = Some(设置.构建.clone());
        Ok(())
    }
}

/// 项目设置的可编辑视图。
///
/// 与 [`项目清单`] 的区别：清单是**持久化格式**（含格式版本、配置表、运行环境变量），
/// 视图只包含对话框里能编辑的字段，并且全部是"界面友好"的类型（路径用文本）。
#[derive(Debug, Clone, PartialEq)]
pub struct 项目设置 {
    /// 项目名。
    pub 名称: String,
    /// 项目说明。
    pub 说明: String,
    /// 启动场景（项目相对路径文本）。空 = 不设启动场景。
    pub 启动场景: Option<String>,
    /// 运行目标程序。空 = 独立预览进程。
    pub 运行程序: String,
    /// 运行参数（每行一个）。
    pub 运行参数: Vec<String>,
    /// 运行工作目录（项目相对路径文本）。
    pub 运行工作目录: Option<String>,
    /// 构建配置。
    pub 构建: crate::build::构建配置,
    /// 设计分辨率（UI 布局空间）。
    pub 设计分辨率: (u32, u32),
    /// 打包配置。
    pub 打包: crate::package::打包配置,
}

impl Default for 项目设置 {
    fn default() -> Self {
        Self {
            名称: String::new(),
            说明: String::new(),
            启动场景: None,
            运行程序: String::new(),
            运行参数: Vec::new(),
            运行工作目录: None,
            构建: crate::build::构建配置::default(),
            设计分辨率: 默认设计分辨率(),
            打包: crate::package::打包配置::default(),
        }
    }
}

impl 项目设置 {
    /// 运行目标是否已声明。
    pub fn 有运行目标(&self) -> bool {
        !self.运行程序.trim().is_empty()
    }
}

// ---------------------------------------------------------------------------
// 资源导入状态
// ---------------------------------------------------------------------------

/// 导入清单文件名（项目根下，进版本控制）。
///
/// # 为什么导入状态要放在项目里而不是用户配置里
///
/// 导入状态描述的是"**这份资源的内容**对应哪个派生结果"，
/// 它属于项目：换台机器、换个人，都应该得到同一份导入结论。
/// 这与"窗口布局属于这台机器"正好相反（见 `editor_app::config`）。
///
/// 它也进版本控制：这样 CI 里可以直接判断"是否需要重新导入"，
/// 而不必先跑一遍完整扫描再对比。
pub const 导入清单文件名: &str = "imports.ron";

/// 导入清单的格式版本。
pub const 导入格式版本: u32 = 1;

/// 一条资源的导入记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 导入记录 {
    /// 项目相对路径。
    pub 路径: 项目相对路径,
    /// 字节大小。
    #[serde(default)]
    pub 大小: u64,
    /// 修改时间（Unix 秒）。文件系统不支持时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 修改时间秒: Option<u64>,
    /// 内容指纹。由"大小 + 修改时间"算出，用于判断是否变化。
    #[serde(default)]
    pub 指纹: u64,
}

impl 导入记录 {
    /// 由磁盘上的文件信息构造记录。
    pub fn 从文件(路径: 项目相对路径, 绝对: &Path) -> Self {
        let 元数据 = std::fs::metadata(绝对).ok();
        let 大小 = 元数据.as_ref().map(|元| 元.len()).unwrap_or(0);
        let 修改时间秒 = 元数据
            .as_ref()
            .and_then(|元| 元.modified().ok())
            .and_then(|时间| 时间.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|时长| 时长.as_secs());
        let 指纹 = 计算指纹(大小, 修改时间秒);
        Self {
            路径,
            大小,
            修改时间秒,
            指纹,
        }
    }
}

/// 由大小与修改时间算出稳定指纹。
///
/// # 为什么不做内容哈希
///
/// 内容哈希要读整个文件：一个 200MB 的模型会让"扫描项目"变成几十秒。
/// 编辑器需要回答的是"这个文件**可能**变了没"，所以用
/// `(大小, 修改时间)` 这组廉价信号；真正的导入器仍会读内容，
/// 它可以用更精确的哈希去重（那是派生缓存的事，不是这里的事）。
fn 计算指纹(大小: u64, 修改时间秒: Option<u64>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut 哈希 = std::collections::hash_map::DefaultHasher::new();
    大小.hash(&mut 哈希);
    修改时间秒.hash(&mut 哈希);
    哈希.finish()
}

/// 资源导入清单（持久化格式）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 资源导入清单 {
    /// 格式版本。
    #[serde(default = "默认导入格式版本")]
    pub 格式版本: u32,
    /// 记录表：键是项目相对路径文本，保证保存顺序稳定。
    #[serde(default)]
    pub 记录: BTreeMap<String, 导入记录>,
}

fn 默认导入格式版本() -> u32 {
    导入格式版本
}

impl Default for 资源导入清单 {
    fn default() -> Self {
        Self {
            格式版本: 导入格式版本,
            记录: BTreeMap::new(),
        }
    }
}

impl 资源导入清单 {
    /// 记录数。
    pub fn 数量(&self) -> usize {
        self.记录.len()
    }

    /// 取一条记录。
    pub fn 取(&self, 路径: &项目相对路径) -> Option<&导入记录> {
        self.记录.get(路径.文本())
    }
}

/// 单个资源的导入状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 导入状态 {
    /// 磁盘上有、清单里没有：需要首次导入。
    新增,
    /// 指纹变化：需要重新导入。
    已变更,
    /// 与清单一致：无需处理。
    最新,
    /// 清单里有、磁盘上没有：资源被删了。
    缺失,
}

impl 导入状态 {
    /// 是否需要执行导入动作。
    pub fn 需要处理(self) -> bool {
        matches!(self, Self::新增 | Self::已变更)
    }

    /// 语义色调名（供界面取色）。
    pub fn 语义(self) -> &'static str {
        match self {
            Self::新增 => "成功",
            Self::已变更 => "警告",
            Self::最新 => "次要",
            Self::缺失 => "错误",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::新增 => "import_state_new",
            Self::已变更 => "import_state_changed",
            Self::最新 => "import_state_uptodate",
            Self::缺失 => "import_state_missing",
        }
    }
}

/// 一个资源的导入状态项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 导入项 {
    /// 项目相对路径。
    pub 路径: 项目相对路径,
    /// 资源类别。
    pub 类别: 资源类别,
    /// 状态。
    pub 状态: 导入状态,
    /// 字节大小。
    pub 大小: u64,
}

/// 一次扫描的汇总。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct 导入报告 {
    /// 全部条目（按路径排序）。
    pub 项: Vec<导入项>,
}

impl 导入报告 {
    /// 按状态计数。
    pub fn 计数(&self, 状态: 导入状态) -> usize {
        self.项.iter().filter(|项| 项.状态 == 状态).count()
    }

    /// 是否需要执行导入。
    pub fn 需要导入(&self) -> bool {
        self.项.iter().any(|项| 项.状态.需要处理())
    }

    /// 一句话摘要（供控制台/状态栏）。
    pub fn 摘要(&self) -> String {
        format!(
            "{} 个资源：新增 {}、变更 {}、最新 {}、缺失 {}",
            self.项.len(),
            自我计数(self, 导入状态::新增),
            自我计数(self, 导入状态::已变更),
            自我计数(self, 导入状态::最新),
            自我计数(self, 导入状态::缺失),
        )
    }
}

fn 自我计数(报告: &导入报告, 状态: 导入状态) -> usize {
    报告.计数(状态)
}

/// 一处缺失的资源引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 缺失引用 {
    /// 引用所在场景（项目相对路径）。
    pub 场景: 项目相对路径,
    /// 引用指向的资源路径。
    pub 资源: 项目相对路径,
}

impl fmt::Display for 缺失引用 {
    fn fmt(&self, 格式: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            格式,
            "{} 引用了不存在的资源 {}",
            self.场景.文本(),
            self.资源.文本()
        )
    }
}

/// 资源移动的结果，供上层同步更新引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 资源移动结果 {
    /// 移动前的路径。
    pub 旧路径: 项目相对路径,
    /// 移动后的路径。
    pub 新路径: 项目相对路径,
}

/// 把用户选的目录变成**绝对路径**（保留符号链接，不解析它）。
///
/// # 为什么不用 `canonicalize`
///
/// Windows 上 `canonicalize` 返回 `\\?\C:\...` 的 verbatim 前缀：它不能直接
/// 显示给用户，也不能直接交给某些子进程当工作目录。`std::path::absolute`
/// 只做"补全 + 归一化 `..`"，得到的仍是普通绝对路径，正好是
/// "项目路径与构建工作目录一致"需要的东西。
///
/// 已经是绝对路径时原样返回（失败时也不 panic —— 打开项目不该因为
/// 拿不到当前目录而整体失败）。
fn 绝对化(路径: &Path) -> PathBuf {
    std::path::absolute(路径).unwrap_or_else(|_| 路径.to_path_buf())
}

/// 在不完整解析 RON 的前提下探测清单的 `格式版本`。
fn 探测清单版本(文本: &str) -> Option<u32> {
    let 位置 = 文本.find("格式版本")?;
    let 后半 = &文本[位置..];
    let 冒号 = 后半.find(':')?;
    let 剩余 = 后半[冒号 + 1..].trim_start();
    let 数字: String = 剩余.chars().take_while(char::is_ascii_digit).collect();
    数字.parse().ok()
}

/// 把用户输入的任意名字清理成安全的文件名。
///
/// 去掉路径分隔符、Windows 保留字符与控制字符，并防止结果为
/// `.` / `..`（那会让场景文件落在目录本身的位置）。
pub fn 清理文件名(输入: &str) -> String {
    let 清理: String = 输入
        .trim()
        .chars()
        .map(|字符| match 字符 {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            字符 if 字符.is_control() => '_',
            字符 => 字符,
        })
        .collect();
    let 清理 = 清理.trim_matches('.').trim().to_string();
    if 清理.is_empty() {
        "未命名".to_string()
    } else {
        清理
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    use scene_doc::document::场景文档;
    use scene_doc::property::属性值;

    /// 建一个临时项目，并在其中写一个资源文件与一个场景文件。
    fn 建项目() -> (tempfile::TempDir, 项目描述) {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 根 = 目录.path().join("设置工程");
        let 项目 = 项目描述::创建(&根, "设置工程").expect("创建项目");
        (目录, 项目)
    }

    fn 写场景(项目: &项目描述, 名称: &str) -> 项目相对路径 {
        let 路径 = 项目相对路径::解析(format!("scenes/{名称}.scene.ron")).unwrap();
        let mut 文档 = scene_doc::document::场景文档::新建(名称);
        文档.生成实体("根", None);
        项目.保存场景(&路径, &文档).expect("保存场景");
        路径
    }

    #[test]
    fn 项目设置能读回清单里的字段() {
        let (_目录, mut 项目) = 建项目();
        let 场景 = 写场景(&项目, "主关卡");
        // 运行目标若是含分隔符的相对路径，保存时会校验存在性；
        // 这里放一个占位文件，模拟"项目已经构建过一次"。
        let 产物目录 = 项目.根目录().join("target").join("debug");
        std::fs::create_dir_all(&产物目录).expect("建产物目录");
        std::fs::write(产物目录.join("game.exe"), b"stub").expect("写占位产物");

        let mut 设置 = 项目.设置视图();
        设置.名称 = "新名字".into();
        设置.说明 = "说明文字".into();
        设置.启动场景 = Some(场景.文本().to_string());
        设置.运行程序 = "target/debug/game.exe".into();
        项目.应用设置(&设置).expect("应用设置");

        let 读回 = 项目.设置视图();
        assert_eq!(读回.名称, "新名字");
        assert_eq!(读回.说明, "说明文字");
        assert_eq!(读回.启动场景.as_deref(), Some(场景.文本()));
        assert_eq!(读回.运行程序, "target/debug/game.exe");
        assert!(读回.有运行目标());

        // 落盘后重新打开仍然一致。
        项目.保存清单().expect("保存清单");
        let 重开 = 项目描述::打开(项目.根目录()).expect("重新打开");
        assert_eq!(重开.清单().名称, "新名字");
        assert_eq!(重开.清单().说明, "说明文字");
        assert!(重开.清单().启动场景.is_some());
        assert!(重开.清单().运行.is_some());
    }

    #[test]
    fn 空名称与不存在的启动场景会被拒绝且不改清单() {
        let (_目录, mut 项目) = 建项目();
        let 原名 = 项目.清单().名称.to_string();

        let mut 设置 = 项目.设置视图();
        设置.名称 = "   ".into();
        let 错误 = 项目.应用设置(&设置).expect_err("空名称应当被拒绝");
        assert!(matches!(错误, 项目错误::设置非法(_)), "错误：{错误:?}");
        assert_eq!(项目.清单().名称, 原名, "校验失败不能改清单");

        let mut 设置 = 项目.设置视图();
        设置.启动场景 = Some("scenes/不存在.scene.ron".into());
        let 错误 = 项目
            .应用设置(&设置)
            .expect_err("不存在的启动场景应当被拒绝");
        assert!(matches!(错误, 项目错误::设置非法(_)));
        assert!(项目.清单().启动场景.is_none());
    }

    #[test]
    fn 清空运行程序会移除运行声明() {
        let (_目录, mut 项目) = 建项目();
        let mut 设置 = 项目.设置视图();
        设置.运行程序 = "cargo".into();
        设置.运行参数 = vec![" run".into(), " ".into(), "--release".into()];
        项目.应用设置(&设置).expect("应用设置");
        let 运行 = 项目.清单().运行.clone().expect("应当有运行目标");
        assert_eq!(运行.参数, vec!["run".to_string(), "--release".to_string()]);

        设置.运行程序 = "   ".into();
        项目.应用设置(&设置).expect("清空运行程序");
        assert!(项目.清单().运行.is_none(), "留空应当移除运行声明");
    }

    #[test]
    fn 导入状态经历新增到最新再到变更与缺失() {
        let (_目录, 项目) = 建项目();
        let 资源路径 = 项目相对路径::解析("assets/textures/hero.png").unwrap();
        std::fs::write(项目.绝对路径(&资源路径), b"v1").expect("写资源");

        let mut 清单 = 资源导入清单::default();
        let 报告 = 项目.扫描导入(&清单);
        assert_eq!(报告.计数(导入状态::新增), 1);
        assert!(报告.需要导入());

        项目.更新导入清单(&mut 清单, &报告);
        项目.保存导入清单(&清单).expect("保存导入清单");
        assert_eq!(清单.数量(), 1);

        // 再扫一次：最新。
        let 报告 = 项目.扫描导入(&清单);
        assert_eq!(报告.计数(导入状态::最新), 1);
        assert!(!报告.需要导入());

        // 改内容（大小也变，保证指纹变化，不依赖文件系统时间精度）。
        std::fs::write(项目.绝对路径(&资源路径), b"version-2-longer").expect("写资源");
        let 报告 = 项目.扫描导入(&清单);
        assert_eq!(报告.计数(导入状态::已变更), 1, "改内容后应当显示已变更");

        // 删掉资源：清单里仍有记录 → 缺失。
        std::fs::remove_file(项目.绝对路径(&资源路径)).expect("删资源");
        let 报告 = 项目.扫描导入(&清单);
        assert_eq!(报告.计数(导入状态::缺失), 1, "被删的资源应当报告缺失");
        assert!(报告.摘要().contains("缺失"), "摘要：{}", 报告.摘要());
    }

    #[test]
    fn 依赖检查报告缺失的资源引用() {
        let (_目录, 项目) = 建项目();
        let 场景路径 = 写场景(&项目, "检查");
        // 在场景里登记一个不存在的资源引用，再存回去。
        let 绝对 = 项目.绝对路径(&场景路径);
        let mut 读取 = 项目.读取场景(&场景路径).expect("读场景");
        读取.文档.资源表.insert(
            项目相对路径::解析("assets/textures/丢失.png").unwrap(),
            scene_doc::document::资源引用 {
                路径: 项目相对路径::解析("assets/textures/丢失.png").unwrap(),
                唯一标识: None,
            },
        );
        std::fs::write(
            &绝对,
            scene_doc::storage::编码场景(&读取.文档).expect("编码"),
        )
        .expect("写场景");

        let 缺失 = 项目.检查资源引用();
        assert_eq!(缺失.len(), 1, "应当报告一条缺失引用：{缺失:?}");
        assert_eq!(缺失[0].场景, 场景路径);
        assert!(缺失[0].资源.文本().contains("丢失.png"));
    }

    #[test]
    fn 同步资源标识会写盘并给新文件分配标识() {
        let (_目录, mut 项目) = 建项目();
        std::fs::write(项目.根目录().join("assets").join("新图.png"), b"a").expect("写资源");
        let (新增, 移除) = 项目.同步资源标识().expect("同步");
        assert_eq!(新增, 1);
        assert_eq!(移除, 0);
        assert!(项目.资源标识路径().is_file(), "微风项目必须落盘");

        let 重开 = 项目描述::打开(项目.根目录()).expect("重开");
        assert_eq!(
            重开
                .资源标识()
                .取(&项目相对路径::解析("assets/新图.png").unwrap()),
            项目
                .资源标识()
                .取(&项目相对路径::解析("assets/新图.png").unwrap()),
            "重新打开后标识必须一致"
        );
    }

    #[test]
    fn 保存场景时把标识写进文件并补进表() {
        let (_目录, mut 项目) = 建项目();
        let mut 文档 = 场景文档::新建("带引用");
        文档.资源表.insert(
            项目相对路径::解析("assets/引用.png").unwrap(),
            scene_doc::document::资源引用 {
                路径: 项目相对路径::解析("assets/引用.png").unwrap(),
                唯一标识: None,
            },
        );
        let 补入 = 项目.对齐文档标识(&mut 文档);
        assert_eq!(补入, 1, "表里没有的引用应当生成标识并补入");
        assert!(文档.资源表.values().all(|引用| 引用.唯一标识.is_some()));

        // 再对齐一次不应产生新条目。
        let 再次 = 项目.对齐文档标识(&mut 文档);
        assert_eq!(再次, 0, "标识必须稳定");
    }

    #[test]
    fn 外部移动文件后能按标识修复引用() {
        let (_目录, mut 项目) = 建项目();
        let 旧 = 项目相对路径::解析("assets/搬家.png").unwrap();
        let 新 = 项目相对路径::解析("assets/textures/搬家.png").unwrap();
        std::fs::write(项目.绝对路径(&旧), b"x").expect("写资源");
        项目.同步资源标识().expect("同步");
        let 标识 = 项目.资源标识().取(&旧).expect("应当有标识");

        // 场景记录旧路径 + 标识。
        let mut 文档 = 场景文档::新建("搬迁");
        let 实体 = 文档.生成实体("房屋", None);
        let mut 属性 = BTreeMap::new();
        属性.insert(
            "texture".to_string(),
            属性值::资源(scene_doc::property::资源引用值::完整(
                旧.clone(),
                标识,
            )),
        );
        文档
            .取可变(实体)
            .expect("实体")
            .加组件(scene_doc::document::组件数据::新建("Sprite", 属性));
        文档.同步资源表();

        // 模拟用户绕过编辑器移动文件：表里的路径同步更新。
        std::fs::create_dir_all(项目.根目录().join("assets").join("textures")).expect("建目录");
        std::fs::rename(项目.绝对路径(&旧), 项目.绝对路径(&新)).expect("移动文件");
        项目.资源标识可变().重命名(&旧, &新);
        项目.保存资源标识().expect("保存表");

        let 报告 = 项目.按标识修复文档(&mut 文档);
        assert_eq!(报告.已修复.len(), 1, "应当修复一处：{报告:?}");
        assert!(文档.资源表.contains_key(&新));
        assert!(!文档.资源表.contains_key(&旧));
        assert!(文档.校验().无问题(), "修复后文档应当自洽");
    }

    #[test]
    fn 标识冲突时不猜路径() {
        let (_目录, mut 项目) = 建项目();
        let 甲 = 项目相对路径::解析("assets/甲.png").unwrap();
        let 乙 = 项目相对路径::解析("assets/乙.png").unwrap();
        std::fs::write(项目.绝对路径(&甲), b"a").expect("写资源");
        std::fs::write(项目.绝对路径(&乙), b"b").expect("写资源");
        项目.同步资源标识().expect("同步");

        // 人为制造冲突：两条路径共用同一标识。
        let 标识 = 项目.资源标识().取(&甲).expect("标识");
        项目.资源标识可变().标识.insert(乙.文本().to_string(), 标识);

        let mut 文档 = 场景文档::新建("冲突");
        文档.资源表.insert(
            项目相对路径::解析("assets/丢失.png").unwrap(),
            scene_doc::document::资源引用 {
                路径: 项目相对路径::解析("assets/丢失.png").unwrap(),
                唯一标识: Some(scene_doc::path::资源唯一标识::从(标识)),
            },
        );
        let 报告 = 项目.按标识修复文档(&mut 文档);
        assert_eq!(报告.冲突.len(), 1, "冲突必须被报告：{报告:?}");
        assert!(报告.已修复.is_empty(), "不能替用户猜");
    }
}
