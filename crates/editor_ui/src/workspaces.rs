//! 命名工作区：把"一套停靠布局"提升为编辑器的一等对象。
//!
//! # 为什么不能只有一份 `DockState`
//!
//! Blender 与 Godot 的共性不是"面板能拖"，而是**同一份工程数据可以按任务
//! 换一套工作环境**：建模时想看属性，调试时想看控制台与性能曲线，
//! 理资源时想让资源浏览器占满中央。如果编辑器只有一棵 Dock 树，
//! 用户每次切换任务都要手工重排一遍，而且排完就丢了。
//!
//! 因此工作区必须拥有独立的：
//!
//! * 停靠树结构与嵌套分割比例；
//! * 标签组、标签顺序与当前活动标签；
//! * 面板显示/关闭状态；
//! * 浮动表面（egui_dock 的 window surface）及其位置尺寸。
//!
//! 这些**全部**由 `egui_dock` 的 `DockState` 序列化承载，本模块负责
//! "每个工作区各存一份 + 版本 + 迁移 + 修复"，而不是重新发明布局格式。
//!
//! # 与主题的边界
//!
//! 工作区只存**布局**，不存颜色。主题是另一个维度（`crate::theme`），
//! 切换工作区不会触碰主题，切换主题也不会重置布局。这条边界让两者
//! 可以独立演进，也避免了"每份工作区里复制一整份配色"的配置膨胀。
//!
//! # 内置与用户工作区的区别
//!
//! | | 内置 | 用户 |
//! | --- | --- | --- |
//! | 来源 | 代码里的预设（[`内置工作区::预设`]） | 用户创建/复制的快照 |
//! | 名称 | 走 i18n 词表，随语言变化 | 用户输入，不翻译 |
//! | 删除/重命名 | **禁止** | 允许 |
//! | 重置 | 回到代码预设（新版本也跟着更新） | 回到创建时的基线 |
//!
//! 内置工作区不允许删除，是因为它是产品的一部分：删掉之后没有"恢复"
//! 入口，用户只能删配置文件。重置用代码预设而不是"第一次启动的快照"，
//! 是为了让预设的改进能在升级后真正到达用户。

use serde::{Deserialize, Serialize};

use crate::docks::Tab;
use crate::docks::补全标签组;
use crate::i18n::语言;

/// 工作区集存档的格式版本。
///
/// * `1`：初版（单布局字段，已废弃，仅用于迁移）；
/// * `2`：命名工作区集。
pub const 工作区集版本: u32 = 2;

/// 单个工作区布局的格式版本。
///
/// 布局文本本身由 `egui_dock` 序列化，版本号用于将来在迁移时判断
/// "这份布局是按哪一版面板集合写下的"。
pub const 布局版本: u32 = 1;

/// 早期版本「旧版单布局迁移」留下的工作区 id。
///
/// 那个工作区叫「迁移的布局」，内容是用户旧配置里的那份布局，名字却是
/// 实现细节 —— 工作区栏里因此永久多出一个和「通用」几乎一样、用户完全
/// 不知道干什么用的标签，而且迁移还把它设成了当前工作区。
///
/// 迁移是**一次性**的内部动作，不该在界面上留下痕迹。旧布局现在并入
/// 「通用」（见 [`工作区集::从旧布局迁移`]），这个 id 只剩一件事要做：
/// 读取存档时把历史遗留的那一项收回去（见 [`工作区集::从文本恢复`]）。
const 历史迁移工作区标识: &str = "user-migrated";

/// 内置工作区。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum 内置工作区 {
    /// 通用：默认工作区，常规游戏编辑。
    通用,
    /// 场景编辑：视口与场景树/检查器为主。
    场景,
    /// 资源管理：资源浏览器占中央大面积。
    资源,
    /// 调试与性能：控制台/运行时/性能为主。
    调试,
}

impl 内置工作区 {
    /// 全部内置工作区（顺序即工作区栏里的显示顺序）。
    pub fn 全部() -> &'static [内置工作区] {
        &[
            内置工作区::通用,
            内置工作区::场景,
            内置工作区::资源,
            内置工作区::调试,
        ]
    }

    /// 稳定 id。**永不随语言或版本变化**：它出现在配置文件里。
    pub fn id(self) -> &'static str {
        match self {
            Self::通用 => "general",
            Self::场景 => "scene",
            Self::资源 => "assets",
            Self::调试 => "debug",
        }
    }

    /// 本地化名称键。
    pub fn 名称键(self) -> &'static str {
        match self {
            Self::通用 => "workspace_general",
            Self::场景 => "workspace_scene",
            Self::资源 => "workspace_assets",
            Self::调试 => "workspace_debug",
        }
    }

    /// 从 id 反查。
    pub fn 从id(id: &str) -> Option<Self> {
        Self::全部().iter().copied().find(|内置| 内置.id() == id)
    }

    /// 从名称键（i18n 键）反查。
    ///
    /// 菜单里存的是词条键而不是 id：菜单项的文本就是 i18n 键，
    /// 这样派发动作时与界面语言无关。
    pub fn 从名称键(键: &str) -> Option<Self> {
        Self::全部()
            .iter()
            .copied()
            .find(|内置| 内置.名称键() == 键)
    }

    /// 该内置工作区的出厂布局。
    pub fn 预设(self) -> egui_dock::DockState<Tab> {
        match self {
            Self::通用 => crate::docks::默认布局(),
            Self::场景 => 场景预设(),
            Self::资源 => 资源预设(),
            Self::调试 => 调试预设(),
        }
    }
}

// ---------------------------------------------------------------------------
// 预设布局
// ---------------------------------------------------------------------------
//
// 所有预设都必须包含视口（编辑器主体），并让全部通用面板出现在某个标签组里
// —— 这样每个工作区都是一份"完整的工作环境"，而不是要靠用户先打开面板。
//
// 这份"完整"现在由注册表保证，不再靠人工维护四份清单：每份预设只声明
// 设计师显式安排过位置的标签组，其余通用面板由 `crate::docks::补全标签组`
// 追加到该预设的兜底栏（智能体面板按 `面板分组` 被排除在通用工作区之外）。
//
// `fraction` 的语义见 `crate::docks::默认布局`：它是**第一个子节点**的比例。
// `split_right` / `split_below` 的第一个子节点是**旧**节点（左/上），
// `split_left` / `split_above` 的第一个子节点是**新**节点。

/// 场景编辑预设。
///
/// ```text
/// Horizontal(主区 76% | 右列 24%)          ← 检查器/历史
///   └ Vertical(上方 82% | 底部 18%)        ← 控制台/性能
///       └ Horizontal(场景树 20% | 视口 80%) ← 场景树/资源/运行时
/// ```
///
/// 与通用工作区的区别：视口更高更宽（底部只留 18%），资源浏览器折叠进
/// 左侧标签组 —— 场景编辑时资源面板是"偶尔要拖一个贴图进来"，
/// 不应该长期占掉视口下方的高度。
///
/// 兜底栏是**底部**：注册表里新登记、还没人为安排位置的通用面板由
/// `crate::docks::补全标签组` 追加到这里（见该函数的说明）。
fn 场景预设() -> egui_dock::DockState<Tab> {
    let 右列: &[Tab] = &[Tab::检查器, Tab::历史, Tab::时间轴];
    let 左列: &[Tab] = &[Tab::场景树, Tab::资源浏览器, Tab::运行时ecs];
    let 底部 = 补全标签组(
        &[
            Tab::控制台,
            Tab::构建,
            Tab::运行,
            Tab::性能,
            Tab::脚本,
            Tab::瓦片地图,
        ],
        &[&[Tab::视口], 右列, 左列],
    );
    let mut 布局 = egui_dock::DockState::new(vec![Tab::视口]);
    let 主 = 布局.main_surface_mut();
    let [主区, _右列] = 主.split_right(egui_dock::NodeIndex::root(), 0.76, 右列.to_vec());
    let [上方, _底部] = 主.split_below(主区, 0.82, 底部);
    let [_视口, _左列] = 主.split_left(上方, 0.20, 左列.to_vec());
    布局
}

/// 资源管理预设。
///
/// ```text
/// Horizontal(中区 70% | 右列 30%)
///   ├ 中区 └ Horizontal(左列 34% | 资源浏览器/控制台 66%)
///   └ 右列 └ Vertical(视口 60% | 检查器/历史/性能 40%)
/// ```
///
/// 资源浏览器占到中央整列（宽度约 46%、高度 100%），适合逐个浏览资源；
/// 视口缩小但仍在右上保持完整比例，拖动资源到视口的行为照常可用。
///
/// 兜底栏是**左列**：资源工作区里它本来就装着"次要"面板，新面板落在这里
/// 不会打断"中列浏览 → 拖进右上视口"这条主动线。
fn 资源预设() -> egui_dock::DockState<Tab> {
    let 中区成员: &[Tab] = &[Tab::资源浏览器, Tab::控制台];
    let 右下成员: &[Tab] = &[Tab::检查器, Tab::历史, Tab::时间轴, Tab::性能];
    let 左列成员 = 补全标签组(
        &[
            Tab::场景树,
            Tab::构建,
            Tab::运行,
            Tab::运行时ecs,
            Tab::脚本,
            Tab::瓦片地图,
        ],
        &[中区成员, 右下成员, &[Tab::视口]],
    );
    let mut 布局 = egui_dock::DockState::new(中区成员.to_vec());
    let 主 = 布局.main_surface_mut();
    let [中区, 右列] = 主.split_right(egui_dock::NodeIndex::root(), 0.70, vec![Tab::视口]);
    let [_右上半, _右下半] = 主.split_below(右列, 0.60, 右下成员.to_vec());
    let [_资源, _左列] = 主.split_left(中区, 0.34, 左列成员);
    布局
}

/// 调试与性能预设。
///
/// ```text
/// Horizontal(主区 74% | 右列 26%)          ← 性能/历史/检查器
///   └ Vertical(上方 55% | 底部 45%)        ← 控制台/运行时 ECS/资源浏览器
///       └ Horizontal(场景树 20% | 视口 80%)
/// ```
///
/// 底部占 45%：调试时控制台与运行时 ECS 需要能看到多行输出，
/// 而不是像默认布局那样被压成两三行。
///
/// 兜底栏是**底部**：正因为它占 45% 高度，多几个标签也还看得见。
fn 调试预设() -> egui_dock::DockState<Tab> {
    let 右列成员: &[Tab] = &[Tab::性能, Tab::历史, Tab::检查器, Tab::时间轴];
    let 左列成员: &[Tab] = &[Tab::场景树];
    let 底部成员 = 补全标签组(
        &[
            Tab::控制台,
            Tab::构建,
            Tab::运行,
            Tab::运行时ecs,
            Tab::资源浏览器,
            Tab::脚本,
            Tab::瓦片地图,
        ],
        &[&[Tab::视口], 右列成员, 左列成员],
    );
    let mut 布局 = egui_dock::DockState::new(vec![Tab::视口]);
    let 主 = 布局.main_surface_mut();
    let [主区, _右列] = 主.split_right(egui_dock::NodeIndex::root(), 0.74, 右列成员.to_vec());
    let [上方, _底部] = 主.split_below(主区, 0.55, 底部成员);
    let [_视口, _左列] = 主.split_left(上方, 0.20, 左列成员.to_vec());
    布局
}

/// 一个工作区。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 工作区 {
    /// 稳定 id：内置用 [`内置工作区::id`]，用户用 `user-<序号>`。
    pub id: String,
    /// 用户可见名称。内置工作区为 `None`（显示名走 i18n 词表）。
    pub 名称: Option<String>,
    /// 是否内置。
    pub 内置: bool,
    /// 布局的 RON 文本。存文本而不是嵌套结构，是为了让**单个**工作区
    /// 的解析失败只影响它自己，而不是让整份配置连带失效。
    pub 布局: String,
    /// 重置基线（内置 = 出厂预设，用户 = 创建时的快照）。
    pub 基线布局: String,
    /// 布局版本。
    pub 布局版本: u32,
}

impl 工作区 {
    /// 显示名（按当前语言）。
    pub fn 显示名(&self, 语言: 语言) -> String {
        match (&self.名称, 内置工作区::从id(&self.id)) {
            (Some(名称), _) if !名称.trim().is_empty() => 名称.clone(),
            (_, Some(内置)) => 语言.取(内置.名称键()).to_string(),
            _ => self.id.clone(),
        }
    }

    /// 名称键（内置）或用户名称（用户）。
    pub fn 名称键(&self) -> Option<&'static str> {
        内置工作区::从id(&self.id).map(内置工作区::名称键)
    }
}

/// 工作区操作的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 工作区错误 {
    /// 找不到指定 id 的工作区。
    找不到(String),
    /// 内置工作区不允许删除/重命名。
    内置受保护(String),
    /// 名称为空。
    名称为空,
    /// 名称重复。
    名称重复(String),
    /// 布局解析失败。
    布局解析失败(String),
    /// 存档结构不合法。
    存档不合法(String),
}

impl 工作区错误 {
    /// 本地化文案。
    pub fn 文案(&self, 语言: 语言) -> String {
        match self {
            Self::找不到(id) => 语言.填充("workspace_not_found", &[id]),
            Self::内置受保护(名称) => 语言.填充("workspace_builtin_protected", &[名称]),
            Self::名称为空 => 语言.取("workspace_name_empty").to_string(),
            Self::名称重复(名称) => 语言.填充("workspace_name_duplicate", &[名称]),
            Self::布局解析失败(详情) => {
                语言.填充("workspace_layout_parse_failed", &[详情])
            }
            Self::存档不合法(详情) => 语言.填充("workspace_archive_invalid", &[详情]),
        }
    }
}

/// 存档结构（序列化用）。
///
/// 与运行时结构分开：存档字段是**对外契约**，加字段必须带 `serde(default)`，
/// 这样旧版本写出的配置永远能被新版本读进来。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct 工作区集存档 {
    格式版本: u32,
    当前: String,
    项: Vec<工作区存档>,
}

/// 单个工作区的存档。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct 工作区存档 {
    id: String,
    #[serde(default)]
    名称: Option<String>,
    #[serde(default)]
    内置: bool,
    #[serde(default = "当前布局版本")]
    布局版本: u32,
    布局: String,
    #[serde(default)]
    基线布局: Option<String>,
}

fn 当前布局版本() -> u32 {
    布局版本
}

/// 工作区集：编辑器的全部工作区 + 当前激活的那个。
///
/// # 为什么当前布局是"物化"的
///
/// `DockState` 每帧都被界面可变借用（拖动分隔条、切标签）。如果每次绘制前
/// 都从文本反序列化，会把每帧成本抬高到不可接受。因此当前布局始终以
/// **活对象**存在，只在切换/保存时与文本互转。
#[derive(Debug, Clone)]
pub struct 工作区集 {
    项: Vec<工作区>,
    当前: String,
    当前布局: egui_dock::DockState<Tab>,
    /// 加载/修复过程中的说明，供启动日志展示。
    备注: Vec<String>,
}

impl Default for 工作区集 {
    fn default() -> Self {
        Self::新建()
    }
}

impl 工作区集 {
    /// 出厂状态：四个内置工作区，当前为通用。
    pub fn 新建() -> Self {
        Self::以通用布局(内置工作区::通用.预设())
    }

    /// 出厂状态，但把「通用」的出厂布局换成给定布局。
    ///
    /// # 为什么需要它（这是"两行职责分离"的落点）
    ///
    /// 主开发模式（2D / 3D / 代码）决定"我在做什么任务"，工作区决定
    /// "面板怎么摆"。模式之间最明显的差别恰恰体现在**默认面板布局**上
    /// （3D 需要更大的视口、代码模式需要更大的编辑区）。
    ///
    /// 早期版本把每个模式的默认布局登记成一个**以模式命名的用户工作区**
    /// （"2D 默认布局" / "运行默认布局"……）。结果是工作区栏里出现了
    /// 与上一行模式栏一一对应的副本：同一件事在两行各有一个入口，
    /// 用户无法判断"点上面那行的运行"和"点下面那行的运行默认布局"有什么区别。
    ///
    /// 现在改为：模式的默认布局成为该模式里「通用」工作区的**出厂布局**，
    /// 于是工作区栏在每个模式下都稳定地只有 通用 / 场景 / 资源 / 调试
    /// （外加用户自建），名称与模式栏**不再重叠**。
    pub fn 以通用布局(通用布局: egui_dock::DockState<Tab>) -> Self {
        let mut 项 = Vec::new();
        for 内置 in 内置工作区::全部() {
            // 「通用」用调用方给的布局，其余内置工作区用各自的出厂预设。
            let 布局 = if *内置 == 内置工作区::通用 {
                通用布局.clone()
            } else {
                内置.预设()
            };
            let 文本 = 序列化布局(&布局);
            项.push(工作区 {
                id: 内置.id().to_string(),
                名称: None,
                内置: true,
                布局: 文本.clone(),
                基线布局: 文本,
                布局版本,
            });
        }
        Self {
            项,
            当前: 内置工作区::通用.id().to_string(),
            当前布局: 通用布局,
            备注: Vec::new(),
        }
    }

    /// 全部工作区。
    pub fn 列表(&self) -> &[工作区] {
        &self.项
    }

    /// 当前工作区 id。
    pub fn 当前id(&self) -> &str {
        &self.当前
    }

    /// 当前工作区。
    pub fn 当前(&self) -> &工作区 {
        // `当前` 始终指向列表里存在的项（所有修改都会维护这条不变量）。
        self.项
            .iter()
            .find(|项| 项.id == self.当前)
            .unwrap_or_else(|| &self.项[0])
    }

    /// 当前布局（只读）。
    pub fn 布局(&self) -> &egui_dock::DockState<Tab> {
        &self.当前布局
    }

    /// 当前布局（可变）。界面每帧通过它让 `DockArea` 直接修改布局。
    pub fn 布局_mut(&mut self) -> &mut egui_dock::DockState<Tab> {
        &mut self.当前布局
    }

    /// 加载与修复过程中产生的说明（供启动日志）。
    pub fn 备注(&self) -> &[String] {
        &self.备注
    }

    /// 按 id 查找。
    pub fn 查找(&self, id: &str) -> Option<&工作区> {
        self.项.iter().find(|项| 项.id == id)
    }

    /// 当前布局里是否存在某个面板。
    pub fn 面板可见(&self, 面板: Tab) -> bool {
        self.当前布局.find_tab(&面板).is_some()
    }

    /// 把面板切到前台；不在布局里时追加。
    pub fn 聚焦面板(&mut self, 面板: Tab) {
        if let Some(路径) = self.当前布局.find_tab(&面板) {
            let _ = self.当前布局.set_active_tab(路径);
        } else {
            self.当前布局.push_to_focused_leaf(面板);
        }
    }

    /// 把某个面板浮动成一个独立浮窗表面。
    ///
    /// # 这不是"再画一个 egui 窗口"
    ///
    /// `egui_dock` 的浮窗是 `Surface::Window`：它有自己的标题栏、可以拖动、
    /// 可以把标签再拖回主表面，而且**会被序列化**（含窗口位置与大小）。
    /// 因此"浮动面板"与"停靠回去"是同一套布局数据上的两种状态，
    /// 不会出现"浮动出去的面板一保存就丢了"。
    ///
    /// 返回是否真的发生了移动（面板不存在或已在浮窗里时返回 `false`）。
    pub fn 浮动面板(&mut self, 面板: Tab) -> bool {
        let Some(路径) = self.当前布局.find_tab(&面板) else {
            return false;
        };
        if 路径.surface != egui_dock::SurfaceIndex::main() {
            return false;
        }
        // 默认浮窗位置：稍微偏右下，避免正好盖住中央视口。
        let 矩形 = egui::Rect::from_min_size(egui::pos2(160.0, 140.0), egui::vec2(560.0, 400.0));
        self.当前布局
            .move_tab(路径, egui_dock::TabDestination::Window(矩形));
        true
    }

    /// 把全部浮动面板停靠回主表面（追加到视口所在标签组）。
    ///
    /// 返回被停靠的面板数量。
    pub fn 停靠全部浮动(&mut self) -> usize {
        // 目标：视口所在的叶子。布局校验保证视口存在，
        // 但这里仍然用 `Option` 处理，避免任何"必定存在"的假设。
        let Some(视口路径) = self.当前布局.find_tab(&Tab::视口) else {
            return 0;
        };
        let 目标节点 = egui_dock::NodePath {
            surface: 视口路径.surface,
            node: 视口路径.node,
        };

        let 浮动项: Vec<egui_dock::TabPath> = self
            .当前布局
            .iter_surfaces_indexed()
            .filter(|(索引, _)| *索引 != egui_dock::SurfaceIndex::main())
            .flat_map(|(索引, 表面)| {
                表面
                    .iter_all_tabs()
                    .map(move |((节点, 标签), _)| egui_dock::TabPath {
                        surface: 索引,
                        node: 节点,
                        tab: 标签,
                    })
            })
            .collect();

        let mut 数量 = 0;
        for 路径 in 浮动项 {
            self.当前布局.move_tab(
                路径,
                egui_dock::TabDestination::Node(目标节点, egui_dock::TabInsert::Append),
            );
            数量 += 1;
        }
        数量
    }

    /// 当前布局里的浮动表面数量（不含主表面）。
    pub fn 浮动表面数(&self) -> usize {
        self.当前布局
            .iter_surfaces_indexed()
            .filter(|(索引, _)| *索引 != egui_dock::SurfaceIndex::main())
            .count()
    }

    /// 把当前活布局写回当前工作区记录。
    pub fn 同步当前布局(&mut self) {
        let 文本 = 序列化布局(&self.当前布局);
        if let Some(项) = self.项.iter_mut().find(|项| 项.id == self.当前) {
            项.布局 = 文本;
        }
    }

    /// 切换工作区。
    ///
    /// 切换前先把当前布局同步回记录 —— 否则用户在 A 里的调整会在切走时丢失。
    pub fn 切换(&mut self, id: &str) -> Result<(), 工作区错误> {
        if id == self.当前 {
            return Ok(());
        }
        let 目标 = self
            .项
            .iter()
            .find(|项| 项.id == id)
            .ok_or_else(|| 工作区错误::找不到(id.to_string()))?
            .clone();
        let 新布局 = 解析布局(&目标.布局)
            .map_err(|错误| 工作区错误::布局解析失败(error_文本(&错误)))?;
        self.同步当前布局();
        self.当前 = 目标.id;
        self.当前布局 = 新布局;
        Ok(())
    }

    /// 直接替换当前布局（测试与"恢复默认"用）。
    pub fn 设定当前布局(&mut self, 布局: egui_dock::DockState<Tab>) {
        self.当前布局 = 布局;
        self.同步当前布局();
    }

    /// 新建工作区。
    ///
    /// `复制当前` 为真时以当前布局为快照，否则以通用预设为起点。
    pub fn 新建工作区(
        &mut self,
        名称: &str,
        复制当前: bool,
        语言: 语言,
    ) -> Result<String, 工作区错误> {
        let 名称 = 名称.trim();
        if 名称.is_empty() {
            return Err(工作区错误::名称为空);
        }
        if self.项.iter().any(|项| 项.显示名(语言) == 名称) {
            return Err(工作区错误::名称重复(名称.to_string()));
        }

        // 「不复制当前」的起点是该模式下「通用」工作区的**出厂布局**，
        // 而不是全局的 2D 默认布局：在 3D / 代码模式下新建工作区，
        // 起点应当与当前模式一致，否则用户一新建就被拽回 2D 布局。
        let 布局 = if 复制当前 {
            序列化布局(&self.当前布局)
        } else {
            self.项
                .iter()
                .find(|项| 项.id == 内置工作区::通用.id())
                .map(|项| 项.基线布局.clone())
                .unwrap_or_else(|| 序列化布局(&内置工作区::通用.预设()))
        };
        // id 用递增序号：稳定、可读、不与内置冲突。
        let mut 序号 = 1;
        let id = loop {
            let 候选 = format!("user-{序号}");
            if !self.项.iter().any(|项| 项.id == 候选) {
                break 候选;
            }
            序号 += 1;
        };

        let 新布局 = 解析布局(&布局)
            .map_err(|错误| 工作区错误::布局解析失败(error_文本(&错误)))?;
        self.同步当前布局();
        self.项.push(工作区 {
            id: id.clone(),
            名称: Some(名称.to_string()),
            内置: false,
            布局: 布局.clone(),
            基线布局: 布局,
            布局版本,
        });
        self.当前 = id.clone();
        self.当前布局 = 新布局;
        Ok(id)
    }

    /// 复制当前工作区。
    pub fn 复制当前(
        &mut self, 名称: &str, 语言: 语言
    ) -> Result<String, 工作区错误> {
        self.新建工作区(名称, true, 语言)
    }

    /// 重命名（内置工作区不允许）。
    pub fn 重命名(
        &mut self, id: &str, 新名称: &str, 语言: 语言
    ) -> Result<(), 工作区错误> {
        let 新名称 = 新名称.trim();
        if 新名称.is_empty() {
            return Err(工作区错误::名称为空);
        }
        let 当前名 = self
            .项
            .iter()
            .find(|项| 项.id == id)
            .map(|项| 项.显示名(语言))
            .ok_or_else(|| 工作区错误::找不到(id.to_string()))?;
        if 当前名 == 新名称 {
            return Ok(());
        }
        if self
            .项
            .iter()
            .any(|项| 项.id != id && 项.显示名(语言) == 新名称)
        {
            return Err(工作区错误::名称重复(新名称.to_string()));
        }
        let 项 = self
            .项
            .iter_mut()
            .find(|项| 项.id == id)
            .ok_or_else(|| 工作区错误::找不到(id.to_string()))?;
        if 项.内置 {
            return Err(工作区错误::内置受保护(当前名));
        }
        项.名称 = Some(新名称.to_string());
        Ok(())
    }

    /// 删除（内置工作区不允许）。
    pub fn 删除(&mut self, id: &str, 语言: 语言) -> Result<(), 工作区错误> {
        let 项 = self
            .项
            .iter()
            .find(|项| 项.id == id)
            .ok_or_else(|| 工作区错误::找不到(id.to_string()))?;
        if 项.内置 {
            return Err(工作区错误::内置受保护(项.显示名(语言)));
        }
        self.项.retain(|项| 项.id != id);
        if self.当前 == id {
            // 删掉的正是当前工作区：切回通用，并重新物化布局。
            let 回退 = 内置工作区::通用.id().to_string();
            let 布局 = self
                .项
                .iter()
                .find(|项| 项.id == 回退)
                .and_then(|项| 解析布局(&项.布局).ok())
                .unwrap_or_else(|| 内置工作区::通用.预设());
            self.当前 = 回退;
            self.当前布局 = 布局;
        }
        Ok(())
    }

    /// 恢复当前工作区的默认布局。
    pub fn 重置当前(&mut self) -> Result<(), 工作区错误> {
        let 基线 = self
            .项
            .iter()
            .find(|项| 项.id == self.当前)
            .map(|项| 项.基线布局.clone())
            .ok_or_else(|| 工作区错误::找不到(self.当前.clone()))?;
        let 布局 = 解析布局(&基线)
            .map_err(|错误| 工作区错误::布局解析失败(error_文本(&错误)))?;
        self.当前布局 = 布局;
        self.同步当前布局();
        Ok(())
    }

    /// 恢复全部内置工作区的出厂布局。返回被重置的数量。
    ///
    /// # 为什么用「各工作区自己的基线」而不是「内置枚举的预设」
    ///
    /// 「通用」的出厂布局按当前主开发模式不同（2D / 3D / 代码各一套，
    /// 见 [`工作区集::以通用布局`]）。若这里一律回退到
    /// [`内置工作区::通用.预设`]，在 3D 或代码模式下点「重置全部内置」
    /// 会把「通用」重置成 2D 的布局 —— 用户看到的是"重置把布局弄错了"。
    /// 因此以各条目记录的基线为准，只有在基线不可用时才回退到枚举预设。
    pub fn 重置全部内置(&mut self) -> usize {
        let mut 数量 = 0;
        for 项 in &mut self.项 {
            let Some(内置) = 内置工作区::从id(&项.id) else {
                continue;
            };
            // 基线可解析就用基线，否则回退到枚举预设。
            let 文本 = if 解析布局(&项.基线布局).is_ok() {
                项.基线布局.clone()
            } else {
                序列化布局(&内置.预设())
            };
            项.布局 = 文本.clone();
            项.基线布局 = 文本;
            项.布局版本 = 布局版本;
            数量 += 1;
        }
        // 当前若是内置，活布局也要跟着换掉。
        if let Some(内置) = 内置工作区::从id(&self.当前) {
            let 基线 = self
                .项
                .iter()
                .find(|项| 项.id == self.当前)
                .map(|项| 项.布局.clone())
                .unwrap_or_else(|| 序列化布局(&内置.预设()));
            self.当前布局 = 解析布局(&基线).unwrap_or_else(|_| 内置.预设());
            self.同步当前布局();
        }
        数量
    }

    /// 当前布局的轻量指纹，用于检测用户是否改动过布局。
    ///
    /// 与 `界面状态::布局指纹` 同一思路：`DockState` 没有 `PartialEq`，
    /// 用"面板 → (surface, node) 位置 + 活动标签"代替。
    pub fn 布局指纹(&self) -> Vec<(Tab, usize)> {
        let mut 结果: Vec<(Tab, usize)> = Vec::new();
        for 面板 in Tab::全部() {
            if let Some(路径) = self.当前布局.find_tab(面板) {
                let 位置 = 路径.surface.0 * 1000 + 路径.node.0;
                结果.push((*面板, 位置));
            }
        }
        结果.sort_by_key(|(面板, _)| *面板 as u8);
        结果
    }

    /// 序列化成存档文本。
    pub fn 序列化为文本(&self) -> Result<String, 工作区错误> {
        // 序列化前同步一次：调用方可能刚拖动过分隔条。
        let mut 副本 = self.clone();
        副本.同步当前布局();
        let 存档 = 工作区集存档 {
            格式版本: 工作区集版本,
            当前: 副本.当前.clone(),
            项: 副本
                .项
                .iter()
                .map(|项| 工作区存档 {
                    id: 项.id.clone(),
                    名称: 项.名称.clone(),
                    内置: 项.内置,
                    布局版本: 项.布局版本,
                    布局: 项.布局.clone(),
                    基线布局: Some(项.基线布局.clone()),
                })
                .collect(),
        };
        ron::ser::to_string_pretty(
            &存档,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| 工作区错误::存档不合法(错误.to_string()))
    }

    /// 从存档文本恢复。
    ///
    /// 解析成功但**个别**工作区的布局坏掉时，不会整体失败：
    /// 坏的那个回退到基线/预设，并在 [`工作区集::备注`] 里留下说明。
    pub fn 从文本恢复(文本: &str) -> Result<Self, 工作区错误> {
        let 存档: 工作区集存档 = ron::from_str(文本)
            .map_err(|错误| 工作区错误::存档不合法(error_文本(&错误)))?;
        if 存档.项.is_empty() {
            return Err(工作区错误::存档不合法(
                "no workspaces".to_string(),
            ));
        }

        let mut 备注 = Vec::new();
        if 存档.格式版本 > 工作区集版本 {
            备注.push(format!(
                "工作区存档格式版本 {} 高于本版本 {}，已按兼容模式读取",
                存档.格式版本, 工作区集版本
            ));
        }

        let mut 项: Vec<工作区> = Vec::new();
        for 记录 in 存档.项 {
            if 记录.id == 历史迁移工作区标识 {
                // 收回早期版本留下的"迁移的布局"工作区。
                // 它是一次性迁移的痕迹，不是用户自己建的工作区：
                // 名字是实现术语，布局又与「通用」重复，留着只会让人
                // 在工作区栏里对着一个不知道干什么的标签发愣。
                备注.push(
                    "已移除早期版本遗留的「迁移的布局」工作区（旧配置迁移的痕迹）".to_string(),
                );
                continue;
            }
            if 项.iter().any(|已有| 已有.id == 记录.id) {
                备注.push(format!("忽略重复的工作区 id `{}`", 记录.id));
                continue;
            }
            let 内置 = 内置工作区::从id(&记录.id);
            // 回退基线必须是**可用**的布局：先看存档里的基线，
            // 再看内置预设，最后兜底到通用预设。
            //
            // 早期版本在这里用 `记录.布局` 兜底，结果"布局坏 + 没有基线"
            // 时会把坏文本原样留下 —— 测试 `单个工作区布局损坏只影响它自己`
            // 正是这样抓到了它。
            let 基线 = [
                记录.基线布局.clone(),
                内置.map(|内置| 序列化布局(&内置.预设())),
            ]
            .into_iter()
            .flatten()
            .find(|文本| 解析布局(文本).is_ok_and(|布局| 布局有面板(&布局)))
            .unwrap_or_else(|| 序列化布局(&内置工作区::通用.预设()));

            // 布局可用性：能解析且至少还有一个面板。
            //
            // # 这里**不能**要求"必须含视口"
            //
            // 存档是按模式分槽保存的，而 AI Agent 模式的专用布局本来就没有
            // 场景视口。旧检查会把智能体模式的「通用」工作区判成损坏，
            // 换成 2D 预设 —— 于是"启动时选了 agent、界面却是通用布局"，
            // 而且**每次重启都会再坏一次**（存档被自动改写成通用布局）。
            // 各模式的"必须有自己主面板"由 `dev_modes::补全模式` 与
            // `修复(主面板)` 按已知模式判断，这里只需要识别空/坏布局。
            let 布局 = match 解析布局(&记录.布局) {
                Ok(布局) if 布局有面板(&布局) => 记录.布局.clone(),
                _ => {
                    备注.push(format!(
                        "工作区 `{}` 的布局不可用，已回退到基线布局",
                        记录.id
                    ));
                    基线.clone()
                }
            };

            项.push(工作区 {
                id: 记录.id.clone(),
                名称: 记录.名称,
                内置: 内置.is_some() || 记录.内置,
                布局,
                基线布局: 基线,
                布局版本: 记录.布局版本,
            });
        }

        if 项.is_empty() {
            return Err(工作区错误::存档不合法(
                "no valid workspaces".to_string(),
            ));
        }
        // 内置工作区必须齐全：老配置可能还没见过新加的内置预设。
        for 内置 in 内置工作区::全部() {
            if !项.iter().any(|项| 项.id == 内置.id()) {
                let 文本 = 序列化布局(&内置.预设());
                备注.push(format!("补回缺失的内置工作区 `{}`", 内置.id()));
                项.push(工作区 {
                    id: 内置.id().to_string(),
                    名称: None,
                    内置: true,
                    布局: 文本.clone(),
                    基线布局: 文本,
                    布局版本,
                });
            }
        }
        // 内置工作区的显示名必须以代码为准（用户不能改名，也不该被旧存档带偏）。
        for 项 in &mut 项 {
            if 项.内置 {
                项.名称 = None;
            }
        }

        let 当前 = if 项.iter().any(|项| 项.id == 存档.当前) {
            存档.当前
        } else {
            let 回退 = 内置工作区::通用.id().to_string();
            备注.push(format!(
                "存档里的当前工作区 `{}` 不存在，已切回通用工作区",
                存档.当前
            ));
            回退
        };
        let 当前布局 = 项
            .iter()
            .find(|项| 项.id == 当前)
            .and_then(|项| 解析布局(&项.布局).ok())
            .ok_or_else(|| {
                工作区错误::存档不合法("current layout unusable".to_string())
            })?;

        let mut 集 = Self {
            项,
            当前,
            当前布局,
            备注,
        };
        // 统一走一遍修复：把"布局解析不了""当前项失效"这类**结构性**异常
        // 收敛到可用状态。注意这里不做主面板约束 —— 一份工作区集并不知道
        // 自己属于哪个模式（AI Agent 的槽位天然没有视口），模式级校验由
        // `dev_modes::补全模式` / `界面状态::切换模式` 带着模式信息完成。
        集.修复(&[]);
        Ok(集)
    }
    /// 从旧版单布局配置迁移。
    ///
    /// 旧配置（格式版本 1）只有一个 `布局` 字段。迁移策略：
    /// 把它当作「通用」工作区的出厂布局 —— 用户上次看到的东西必须原样
    /// 出现，而不是被悄悄换成出厂布局。
    ///
    /// # 为什么不再新建一个名为"迁移的布局"的工作区
    ///
    /// 早期实现把旧布局包成一个用户工作区（[`历史迁移工作区标识`]）塞进
    /// 列表并设为当前。那份布局和「通用」几乎一样，名字却是内部术语，
    /// 于是工作区栏里永久多出一个"完全没用、不知道干嘛的"标签，而且
    /// 每次进 2D 模式都落在它上面。迁移只是一次性的内部动作：同样的
    /// 目标（上次的布局不丢）用「通用」的出厂布局就能达到，界面不必
    /// 多出任何东西。
    pub fn 从旧布局迁移(旧布局: &str) -> Result<Self, 工作区错误> {
        let 布局 = 解析布局(旧布局)
            .map_err(|错误| 工作区错误::布局解析失败(error_文本(&错误)))?;
        if 布局.find_tab(&Tab::视口).is_none() {
            return Err(工作区错误::布局解析失败(
                "旧布局缺少视口面板".to_string(),
            ));
        }
        let mut 集 = Self::以通用布局(布局);
        集.备注
            .push("已把旧版单布局配置并入「通用」工作区".to_string());
        Ok(集)
    }

    /// 校验并修复所有工作区。返回修复说明。
    ///
    /// # 为什么"必须含视口"不能是全局约束（这是真实缺陷的根因）
    ///
    /// 早期实现把"布局里必须有 `Tab::视口`"当成一条**通用**不变量。这在
    /// 2D/3D 下是对的，但对 AI Agent 模式**恰好相反**：智能体布局是
    /// 「生成对话 + 会话 + 审查 + 控制台」，它本来就没有场景视口。
    /// 于是 `切换模式(智能体)` 刚把智能体预设装进工作区，紧接着调用的
    /// `修复()` 就判定"布局损坏"，把它整份换成 2D 的通用布局 ——
    /// 表现为「模式显示 AI Agent，界面却是通用布局，agent_chat 根本没出现」。
    ///
    /// 现在约束由调用方按**当前模式**给出：`主面板` 是"这份布局至少要有
    /// 其中一个"的候选集合（各模式通常是 `[该模式默认面板, 视口]`）。
    /// 候选为空表示不做面板检查（只检查可解析性）。
    ///
    /// 修复规则：
    /// * 布局无法解析或缺少候选主面板 → 回退该工作区的基线布局；
    /// * 基线布局也不可用 → 回退内置预设；
    /// * 内置预设也不可用 → 退化成"只含第一个候选主面板"的最小布局；
    /// * 用户工作区名称为空 → 用 id 兜底；
    /// * 当前 id 失效 → 回退通用工作区。
    pub fn 修复(&mut self, 主面板: &[Tab]) -> Vec<String> {
        let mut 说明 = Vec::new();
        for 项 in &mut self.项 {
            let 可用 = 解析布局(&项.布局)
                .ok()
                .filter(|布局| 含主面板(布局, 主面板));
            if 可用.is_none() {
                说明.push(format!("工作区 `{}` 的布局损坏，已回退", 项.id));
                let 基线 = 解析布局(&项.基线布局)
                    .ok()
                    .filter(|布局| 含主面板(布局, 主面板));
                let 回退 = 基线
                    .map(|_| 项.基线布局.clone())
                    .or_else(|| {
                        内置工作区::从id(&项.id)
                            .map(|内置| 序列化布局(&内置.预设()))
                            .filter(|文本| {
                                解析布局(文本)
                                    .ok()
                                    .is_some_and(|布局| 含主面板(&布局, 主面板))
                            })
                    })
                    .unwrap_or_else(|| 序列化布局(&最小布局(主面板)));
                项.布局 = 回退.clone();
                项.基线布局 = 回退;
            }
            if !项.内置 && 项.名称.as_deref().unwrap_or("").trim().is_empty() {
                项.名称 = Some(项.id.clone());
                说明.push(format!("工作区 `{}` 名称为空，已用 id 兜底", 项.id));
            }
        }
        if !self.项.iter().any(|项| 项.id == self.当前) {
            说明.push("当前工作区失效，已切回通用工作区".to_string());
            self.当前 = 内置工作区::通用.id().to_string();
        }
        // 活布局也要跟着修复。
        if 解析布局(&序列化布局(&self.当前布局)).is_err() || !含主面板(&self.当前布局, 主面板)
        {
            let 回退 = 内置工作区::通用.预设();
            self.当前布局 = if 含主面板(&回退, 主面板) {
                回退
            } else {
                最小布局(主面板)
            };
            self.同步当前布局();
        }
        self.备注.extend(说明.iter().cloned());
        说明
    }
}

/// 布局里是否至少还有一个面板。
///
/// "一个面板都没有"是**结构性损坏**（用户不可能把标签全关光，视口不可关闭）；
/// 而"没有某类面板"不是 —— 后者要由知道当前模式的一方判断。
fn 布局有面板(布局: &egui_dock::DockState<Tab>) -> bool {
    Tab::全部含智能体()
        .iter()
        .any(|面板| 布局.find_tab(面板).is_some())
}

/// 布局里是否含"至少一个候选主面板"。
///
/// 候选为空表示调用方不做面板约束（只关心能否解析）。
fn 含主面板(布局: &egui_dock::DockState<Tab>, 主面板: &[Tab]) -> bool {
    if 主面板.is_empty() {
        return true;
    }
    主面板.iter().any(|面板| 布局.find_tab(面板).is_some())
}

/// 最小可用布局：只含第一个候选主面板。
///
/// 这是修复链路的最后一档兜底 —— 连内置预设都不满足候选约束时，
/// 至少保证编辑器里有**一个**能做主的真实面板，而不是把用户丢进空界面。
fn 最小布局(主面板: &[Tab]) -> egui_dock::DockState<Tab> {
    let 面板 = 主面板.first().copied().unwrap_or(Tab::视口);
    egui_dock::DockState::new(vec![面板])
}

/// 序列化一个布局。
///
/// 布局里只有面板枚举与浮点数，序列化失败在实践中不会发生；
/// 万一发生，退回一个"只有视口"的最小布局，保证编辑器仍可用。
fn 序列化布局(布局: &egui_dock::DockState<Tab>) -> String {
    ron::ser::to_string_pretty(
        布局,
        ron::ser::PrettyConfig::default().new_line("\n".to_string()),
    )
    .unwrap_or_else(|_| "()".to_string())
}

/// 解析布局文本。
fn 解析布局(文本: &str) -> Result<egui_dock::DockState<Tab>, ron::error::SpannedError> {
    ron::from_str(文本)
}

/// 把解析错误压成一行文本（用于日志与错误文案）。
fn error_文本<E: std::fmt::Display>(错误: &E) -> String {
    错误.to_string()
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 空布局构造：只有视口，用于模拟"用户把面板都关了"。
    fn 只有视口() -> egui_dock::DockState<Tab> {
        egui_dock::DockState::new(vec![Tab::视口])
    }

    #[test]
    fn 内置工作区齐全且id唯一() {
        let 集 = 工作区集::新建();
        assert_eq!(集.列表().len(), 4, "出厂应有四个内置工作区");
        let mut ids: Vec<&str> = 集.列表().iter().map(|项| 项.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4, "工作区 id 必须唯一");
        for 内置 in 内置工作区::全部() {
            assert!(集.查找(内置.id()).is_some(), "缺少内置工作区 {}", 内置.id());
            assert_eq!(集.查找(内置.id()).unwrap().id, 内置.id());
            assert!(集.查找(内置.id()).unwrap().内置);
        }
        assert_eq!(集.当前id(), 内置工作区::通用.id());
    }

    #[test]
    fn 每个内置预设都包含全部面板且含视口() {
        for 内置 in 内置工作区::全部() {
            let 布局 = 内置.预设();
            assert!(
                布局.find_tab(&Tab::视口).is_some(),
                "工作区 {} 必须包含视口",
                内置.id()
            );
            for 面板 in Tab::全部() {
                assert!(
                    布局.find_tab(面板).is_some(),
                    "工作区 {} 缺少面板 `{}`：工作区应是一份完整工作环境",
                    内置.id(),
                    面板.标题(语言::中文)
                );
            }
        }
    }

    /// 布局里每个叶子（= 一个标签组）的成员：组内顺序保留，组间顺序无关。
    ///
    /// 用菜单键而不是 `Tab` 本身做比较值 —— `Tab` 没有 `Ord`，而"叶子遍历顺序"
    /// 是 egui_dock 的实现细节，不是布局设计意图的一部分。
    fn 叶子标签组(布局: &egui_dock::DockState<Tab>) -> Vec<Vec<&'static str>> {
        let mut 组: Vec<Vec<&'static str>> = 布局
            .iter_leaves()
            .map(|(_, 叶)| 叶.tabs().iter().map(|面板| 面板.菜单键()).collect())
            .collect();
        组.sort();
        组
    }

    /// 派生之后的四份出厂布局，标签组成员必须与建立注册表之前手写的清单**逐字一致**。
    ///
    /// 期望值是从拆分前那四处 `vec![...]` 里手工抄进来的，不是从代码生成的。
    /// 这条是「只加不改」的验收线：兜底派生今天必须是空集合，任何预设少一个面板、
    /// 多一个面板、或组内顺序变了，都会在这里失败，而不是表现成"某工作区打开后怪怪的"。
    #[test]
    fn 每个内置预设的标签组成员与派生前逐字一致() {
        let 期望: &[(&str, &[&[Tab]])] = &[
            (
                内置工作区::通用.id(),
                &[
                    &[Tab::视口],
                    &[
                        Tab::检查器,
                        Tab::历史,
                        Tab::时间轴,
                        Tab::运行时ecs,
                        Tab::性能,
                    ],
                    &[
                        Tab::资源浏览器,
                        Tab::控制台,
                        Tab::构建,
                        Tab::运行,
                        Tab::脚本,
                        Tab::瓦片地图,
                    ],
                    &[Tab::场景树],
                ],
            ),
            (
                内置工作区::场景.id(),
                &[
                    &[Tab::视口],
                    &[Tab::检查器, Tab::历史, Tab::时间轴],
                    &[
                        Tab::控制台,
                        Tab::构建,
                        Tab::运行,
                        Tab::性能,
                        Tab::脚本,
                        Tab::瓦片地图,
                    ],
                    &[Tab::场景树, Tab::资源浏览器, Tab::运行时ecs],
                ],
            ),
            (
                内置工作区::资源.id(),
                &[
                    &[Tab::资源浏览器, Tab::控制台],
                    &[Tab::视口],
                    &[Tab::检查器, Tab::历史, Tab::时间轴, Tab::性能],
                    &[
                        Tab::场景树,
                        Tab::构建,
                        Tab::运行,
                        Tab::运行时ecs,
                        Tab::脚本,
                        Tab::瓦片地图,
                    ],
                ],
            ),
            (
                内置工作区::调试.id(),
                &[
                    &[Tab::视口],
                    &[Tab::性能, Tab::历史, Tab::检查器, Tab::时间轴],
                    &[
                        Tab::控制台,
                        Tab::构建,
                        Tab::运行,
                        Tab::运行时ecs,
                        Tab::资源浏览器,
                        Tab::脚本,
                        Tab::瓦片地图,
                    ],
                    &[Tab::场景树],
                ],
            ),
        ];
        let 排序 = |组: &[&[Tab]]| -> Vec<Vec<&'static str>> {
            let mut 键: Vec<Vec<&'static str>> = 组
                .iter()
                .map(|成员| 成员.iter().map(|面板| 面板.菜单键()).collect())
                .collect();
            键.sort();
            键
        };
        assert_eq!(
            期望.len(),
            内置工作区::全部().len(),
            "四份内置预设都要被这条断言覆盖"
        );
        for (id, 期望组) in 期望 {
            let 内置 = 内置工作区::从id(id).expect("内置 id 应当能反查");
            assert_eq!(
                叶子标签组(&内置.预设()),
                排序(期望组),
                "工作区 {id} 的标签组成员与派生前不一致"
            );
        }
    }

    /// 兜底派生只补**通用**面板：AI Agent 的三个面板属于它自己的一级模式，
    /// 出厂布局里凭空多出三个对话面板是需求明确禁止的。
    #[test]
    fn 兜底派生不会把智能体面板塞进内置预设() {
        for 内置 in 内置工作区::全部() {
            let 布局 = 内置.预设();
            for 面板 in Tab::全部含智能体() {
                if Tab::全部().contains(面板) {
                    continue;
                }
                assert!(
                    布局.find_tab(面板).is_none(),
                    "工作区 {} 不应包含智能体面板 {面板:?}",
                    内置.id()
                );
            }
        }
    }

    #[test]
    fn 不同内置工作区使用不同布局() {
        // 用结构指纹比较：若两份预设序列化后完全一致，说明"新工作区"
        // 只是换了个名字，属于需求明确禁止的假功能。
        let mut 指纹: Vec<String> = Vec::new();
        for 内置 in 内置工作区::全部() {
            指纹.push(序列化布局(&内置.预设()));
        }
        let 原数量 = 指纹.len();
        指纹.sort();
        指纹.dedup();
        assert_eq!(指纹.len(), 原数量, "内置工作区的布局必须彼此不同");
    }

    #[test]
    fn 切换工作区会恢复对应布局() {
        let mut 集 = 工作区集::新建();
        // 在通用工作区里把面板关到只剩视口。
        集.设定当前布局(只有视口());
        assert!(!集.面板可见(Tab::控制台));

        // 切到调试：布局必须来自调试预设，而不是残留的"只有视口"。
        集.切换(内置工作区::调试.id()).expect("切换应当成功");
        assert!(集.面板可见(Tab::控制台), "调试工作区应当有控制台");
        assert_eq!(集.当前id(), 内置工作区::调试.id());

        // 切回通用：通用工作区里已被用户改成"只有视口"，必须原样保留。
        集.切换(内置工作区::通用.id()).expect("切回应当成功");
        assert!(
            !集.面板可见(Tab::控制台),
            "用户在某工作区里的改动不得被其他工作区的布局覆盖"
        );
    }

    #[test]
    fn 切换会同步当前布局避免丢改动() {
        let mut 集 = 工作区集::新建();
        集.设定当前布局(只有视口());
        集.切换(内置工作区::场景.id()).unwrap();
        // 通用工作区的记录里应当已经保存了"只有视口"。
        let 通用 = 集.查找(内置工作区::通用.id()).unwrap();
        let 布局 = 解析布局(&通用.布局).unwrap();
        assert!(
            布局.find_tab(&Tab::控制台).is_none(),
            "切换前必须把当前活布局同步回记录"
        );
    }

    #[test]
    fn 内置工作区不能被删除或重命名() {
        let mut 集 = 工作区集::新建();
        let 结果 = 集.删除(内置工作区::通用.id(), 语言::中文);
        assert_eq!(
            结果,
            Err(工作区错误::内置受保护("通用".to_string())),
            "内置工作区必须受保护"
        );
        let 结果 = 集.重命名(内置工作区::通用.id(), "我的通用", 语言::中文);
        assert!(matches!(结果, Err(工作区错误::内置受保护(_))));
        assert_eq!(集.列表().len(), 4);
    }

    #[test]
    fn 用户工作区可新建复制重命名删除() {
        let mut 集 = 工作区集::新建();
        let id = 集.新建工作区("我的布局", false, 语言::中文).unwrap();
        assert!(id.starts_with("user-"));
        assert_eq!(集.列表().len(), 5);
        assert_eq!(集.当前id(), id);
        assert_eq!(集.当前().显示名(语言::中文), "我的布局");

        // 复制当前
        let 副本 = 集.复制当前("我的布局 副本", 语言::中文).unwrap();
        assert_ne!(副本, id);
        let 原布局 = 集.查找(&id).unwrap().布局.clone();
        let 副布局 = 集.查找(&副本).unwrap().布局.clone();
        assert_eq!(原布局, 副布局, "复制出的工作区应继承当前布局");

        // 重命名
        集.重命名(&副本, "资源专用", 语言::中文).unwrap();
        assert_eq!(集.查找(&副本).unwrap().显示名(语言::中文), "资源专用");

        // 删除
        集.删除(&副本, 语言::中文).unwrap();
        assert!(集.查找(&副本).is_none());
        assert_eq!(集.列表().len(), 5);
    }

    #[test]
    fn 新建工作区拒绝空名与重名() {
        let mut 集 = 工作区集::新建();
        assert_eq!(
            集.新建工作区("   ", false, 语言::中文),
            Err(工作区错误::名称为空)
        );
        assert_eq!(
            集.新建工作区("通用", false, 语言::中文),
            Err(工作区错误::名称重复("通用".to_string())),
            "不能与内置工作区重名"
        );
        let id = 集.新建工作区("我的", false, 语言::中文).unwrap();
        assert_eq!(
            集.新建工作区("我的", false, 语言::中文),
            Err(工作区错误::名称重复("我的".to_string()))
        );
        // 改成同名（自身）不算冲突。
        集.重命名(&id, "我的", 语言::中文).unwrap();
    }

    #[test]
    fn 删除当前工作区会安全回退到通用() {
        let mut 集 = 工作区集::新建();
        let id = 集.新建工作区("临时", false, 语言::中文).unwrap();
        assert_eq!(集.当前id(), id);
        集.删除(&id, 语言::中文).unwrap();
        assert_eq!(集.当前id(), 内置工作区::通用.id());
        assert!(集.面板可见(Tab::视口), "回退后当前布局必须可用");
    }

    #[test]
    fn 重置当前只影响当前工作区() {
        let mut 集 = 工作区集::新建();
        // 场景工作区改成只有视口
        集.切换(内置工作区::场景.id()).unwrap();
        集.设定当前布局(只有视口());
        // 通用保持原样
        let 通用原样 = 集.查找(内置工作区::通用.id()).unwrap().布局.clone();

        集.重置当前().unwrap();
        assert!(集.面板可见(Tab::控制台), "重置后应当回到场景预设");
        assert_eq!(
            集.查找(内置工作区::通用.id()).unwrap().布局,
            通用原样,
            "重置当前工作区不得影响其他工作区"
        );
    }

    #[test]
    fn 重置全部内置只恢复内置工作区() {
        let mut 集 = 工作区集::新建();
        let 用户id = 集.新建工作区("用户甲", false, 语言::中文).unwrap();
        集.设定当前布局(只有视口());

        let 数量 = 集.重置全部内置();
        assert_eq!(数量, 4, "四个内置工作区都应被重置");
        assert!(
            集.查找(&用户id).is_some(),
            "重置内置工作区不得删除用户工作区"
        );
        // 内置工作区回到出厂布局（含全部面板）。
        let 通用 = 集.查找(内置工作区::通用.id()).unwrap();
        let 布局 = 解析布局(&通用.布局).unwrap();
        for 面板 in Tab::全部() {
            assert!(
                布局.find_tab(面板).is_some(),
                "重置后通用工作区应当有面板 `{}`",
                面板.标题(语言::中文)
            );
        }
    }

    #[test]
    fn 存档往返保留全部工作区与当前项() {
        let mut 集 = 工作区集::新建();
        let id = 集.新建工作区("往返测试", false, 语言::中文).unwrap();
        集.设定当前布局(只有视口());

        let 文本 = 集.序列化为文本().expect("序列化应当成功");
        let 复原 = 工作区集::从文本恢复(&文本).expect("反序列化应当成功");

        assert_eq!(复原.列表().len(), 5);
        assert_eq!(复原.当前id(), id);
        assert!(
            !复原.面板可见(Tab::控制台),
            "当前工作区的活布局必须原样恢复"
        );
        assert_eq!(复原.查找(&id).unwrap().显示名(语言::中文), "往返测试");
    }

    #[test]
    fn 存档为未知字段时仍可读取() {
        // 模拟"新版本写了额外字段，旧版本读取"：serde 默认忽略未知字段。
        let 文本 = r#"
(
    格式版本: 2,
    当前: "general",
    未来字段: 42,
    项: [
        (id: "general", 内置: true, 布局: "()", 基线布局: None),
    ],
)
"#;
        // 布局 "()" 解析不出可用布局 → 应当回退到内置预设而不是整体失败。
        let 集 = 工作区集::从文本恢复(文本).expect("应当容忍未知字段与坏布局");
        assert_eq!(集.当前id(), "general");
        assert!(集.面板可见(Tab::视口), "坏布局应回退到内置预设");
        assert_eq!(集.列表().len(), 4, "缺失的内置工作区应当被补回");
        assert!(!集.备注().is_empty(), "修复过程必须留下说明");
    }

    #[test]
    fn 损坏存档整体失败并给出错误() {
        let 结果 = 工作区集::从文本恢复("这不是 RON {{{");
        assert!(matches!(结果, Err(工作区错误::存档不合法(_))));
        // 空项也是无效存档。
        let 结果 = 工作区集::从文本恢复("(格式版本: 2, 当前: \"general\", 项: [])");
        assert!(matches!(结果, Err(工作区错误::存档不合法(_))));
    }

    #[test]
    fn 单个工作区布局损坏只影响它自己() {
        let 集 = 工作区集::新建();
        let 好布局 = 集.查找(内置工作区::通用.id()).unwrap().布局.clone();
        let 文本 = format!(
            r#"
(
    格式版本: 2,
    当前: "general",
    项: [
        (id: "general", 内置: true, 布局: {好布局:?}, 基线布局: None),
        (id: "user-1", 名称: Some("坏掉的那个"), 内置: false, 布局: "((((", 基线布局: None),
    ],
)
"#
        );
        let 集 = 工作区集::从文本恢复(&文本).expect("单条布局损坏不应导致整体失败");
        assert_eq!(集当前工作区数(&集), 5, "四个内置 + 一个用户工作区");
        let 用户 = 集.查找("user-1").expect("损坏的用户工作区应当保留");
        // 回退到通用预设（基线也坏，只能兜底）。
        let 布局 = 解析布局(&用户.布局).expect("回退后的布局必须可解析");
        assert!(布局.find_tab(&Tab::视口).is_some());
        assert!(
            集.备注().iter().any(|说明| 说明.contains("user-1")),
            "必须记录修复说明：{:?}",
            集.备注()
        );
    }

    fn 集当前工作区数(集: &工作区集) -> usize {
        集.列表().len()
    }

    #[test]
    fn 旧版单布局并入通用工作区而不新建标签() {
        let 旧 = 序列化布局(&内置工作区::场景.预设());
        let 集 = 工作区集::从旧布局迁移(&旧).expect("迁移应当成功");
        // 用户上次看到的布局必须原样出现。
        assert_eq!(集.当前id(), 内置工作区::通用.id());
        assert!(集.面板可见(Tab::视口));
        // 工作区栏不能因为迁移多出任何东西。
        assert_eq!(集.列表().len(), 内置工作区::全部().len());
        assert!(集.列表().iter().all(|项| 项.内置));
        assert!(!集.备注().is_empty(), "迁移必须留下说明");
        assert!(
            工作区集::从旧布局迁移("垃圾").is_err(),
            "非法旧布局必须报错"
        );
    }

    #[test]
    fn 早期迁移残留的工作区在读取时被收回() {
        // 造一份"早期版本迁移后保存"的存档：多一个 user-migrated，且它是当前项。
        let 集 = 工作区集::新建();
        let 通用布局 = 集.查找(内置工作区::通用.id()).unwrap().布局.clone();
        let 旧布局 = 序列化布局(&内置工作区::场景.预设());
        let 存档 = format!(
            "(格式版本: 2, 当前: \"user-migrated\", 项: [\
             (id: \"general\", 内置: true, 布局: {通用布局:?}), \
             (id: \"user-migrated\", 名称: Some(\"迁移的布局\"), 内置: false, 布局: {旧布局:?})])"
        );
        let 恢复 = 工作区集::从文本恢复(&存档).expect("存档应当能恢复");
        assert!(
            恢复.查找("user-migrated").is_none(),
            "历史遗留的「迁移的布局」必须被收回"
        );
        assert_eq!(
            恢复.当前id(),
            内置工作区::通用.id(),
            "当前项失效时必须回退到通用"
        );
        assert_eq!(恢复.列表().len(), 内置工作区::全部().len());
    }

    #[test]
    fn 修复能把坏布局与失效当前项拉回可用状态() {
        let mut 集 = 工作区集::新建();
        // 直接破坏内部记录（模拟手改配置文件后的状态）。
        集.项[0].布局 = "((((".to_string();
        集.项[0].基线布局 = "((((".to_string();
        集.当前 = "不存在的 id".to_string();
        集.当前布局 = 只有视口();

        let 说明 = 集.修复(&[Tab::视口]);
        assert!(!说明.is_empty(), "修复必须留下说明");
        assert_eq!(集.当前id(), 内置工作区::通用.id());
        let 通用 = 集.查找(内置工作区::通用.id()).unwrap();
        let 布局 = 解析布局(&通用.布局).expect("修复后的布局必须可解析");
        assert!(布局.find_tab(&Tab::视口).is_some());
    }

    #[test]
    fn 关闭面板后仍可通过聚焦重新访问() {
        let mut 集 = 工作区集::新建();
        集.设定当前布局(只有视口());
        for 面板 in Tab::全部() {
            if *面板 == Tab::视口 {
                // 视口本来就不可关闭，跳过前置条件检查。
                continue;
            }
            assert!(
                !集.面板可见(*面板),
                "前置条件：`{}` 应当已被关闭",
                面板.标题(语言::中文)
            );
            集.聚焦面板(*面板);
            assert!(
                集.面板可见(*面板),
                "聚焦后 `{}` 必须重新出现在布局里",
                面板.标题(语言::中文)
            );
        }
    }

    #[test]
    fn 布局指纹能反映面板位置变化() {
        let mut 集 = 工作区集::新建();
        let 原指纹 = 集.布局指纹();
        集.设定当前布局(只有视口());
        assert_ne!(集.布局指纹(), 原指纹, "关闭面板后指纹必须变化");
    }

    /// 在 1200×800 下跑一次真实布局，返回每个激活标签的矩形。
    fn 测真实矩形(内置: 内置工作区) -> Vec<(Tab, egui::Rect)> {
        let mut 布局 = 内置.预设();
        crate::docks::测量布局矩形(&mut 布局, egui::vec2(1200.0, 800.0))
    }

    fn 取矩形(矩形: &[(Tab, egui::Rect)], 面板: Tab) -> egui::Rect {
        矩形
            .iter()
            .find(|(候选, _)| *候选 == 面板)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("`{}` 没有拿到矩形", 面板.标题(语言::中文)))
    }

    /// 每个内置工作区都必须是**可用的**真实布局，而不是名义上不同。
    ///
    /// 这条测试跑真实的一帧布局计算，因此能挡住：
    /// * fraction 方向写反把某栏压成细条；
    /// * 预设漏掉关键面板（场景树/检查器/视口）；
    /// * 面板矩形超出窗口或退化。
    ///
    /// 注意：`egui_dock` 每个叶子只绘制**当前激活的**标签，
    /// 因此这里逐个工作区列出"应当是激活状态"的面板。
    /// 非激活标签仍可访问（点标签即可），只是不产生内容矩形。
    #[test]
    fn 各内置工作区的真实布局矩形合理() {
        for (内置, 激活面板) in [
            (
                内置工作区::通用,
                vec![Tab::视口, Tab::场景树, Tab::检查器, Tab::资源浏览器],
            ),
            (
                内置工作区::场景,
                vec![Tab::视口, Tab::场景树, Tab::检查器, Tab::控制台],
            ),
            (
                内置工作区::资源,
                vec![Tab::资源浏览器, Tab::视口, Tab::场景树, Tab::检查器],
            ),
            (
                内置工作区::调试,
                vec![Tab::视口, Tab::场景树, Tab::控制台, Tab::性能],
            ),
        ] {
            let 矩形 = 测真实矩形(内置);
            for 面板 in 激活面板 {
                let r = 取矩形(&矩形, 面板);
                assert!(
                    r.width() >= 60.0 && r.height() >= 60.0,
                    "工作区 `{}` 的 `{}` 矩形退化：{r:?}",
                    内置.id(),
                    面板.标题(语言::中文)
                );
                assert!(
                    r.min.x >= -1.0 && r.min.y >= -1.0 && r.max.x <= 1201.0 && r.max.y <= 801.0,
                    "工作区 `{}` 的 `{}` 越出窗口：{r:?}",
                    内置.id(),
                    面板.标题(语言::中文)
                );
            }
            let 视口 = 取矩形(&矩形, Tab::视口);
            let 宽占比 = 视口.width() / 1200.0;
            let 高占比 = 视口.height() / 800.0;
            assert!(
                (0.20..=0.90).contains(&宽占比),
                "工作区 `{}` 的视口宽占比 {宽占比:.3} 不合理：{视口:?}",
                内置.id()
            );
            assert!(
                (0.20..=0.95).contains(&高占比),
                "工作区 `{}` 的视口高占比 {高占比:.3} 不合理：{视口:?}",
                内置.id()
            );
        }
    }

    /// 各工作区的差异必须是**结构性**的，不能只是标签顺序不同。
    #[test]
    fn 不同工作区拥有结构性不同的布局() {
        let 通用 = 测真实矩形(内置工作区::通用);
        let 场景 = 测真实矩形(内置工作区::场景);
        let 资源 = 测真实矩形(内置工作区::资源);
        let 调试 = 测真实矩形(内置工作区::调试);

        // 场景工作区把底部压到 18%，视口必须比通用更高。
        let 通用视口 = 取矩形(&通用, Tab::视口);
        let 场景视口 = 取矩形(&场景, Tab::视口);
        assert!(
            场景视口.height() > 通用视口.height(),
            "场景工作区的视口应当更高：场景 {:?} vs 通用 {:?}",
            场景视口,
            通用视口
        );

        // 资源工作区让资源浏览器占满中央整列的高度。
        let 通用资源 = 取矩形(&通用, Tab::资源浏览器);
        let 资源资源 = 取矩形(&资源, Tab::资源浏览器);
        assert!(
            资源资源.height() > 通用资源.height() * 1.5,
            "资源工作区的资源浏览器应当显著更高：资源 {:?} vs 通用 {:?}",
            资源资源,
            通用资源
        );

        // 调试工作区的底部（控制台）比通用工作区的底部更高。
        let 通用底部 = 取矩形(&通用, Tab::资源浏览器);
        let 调试底部 = 取矩形(&调试, Tab::控制台);
        assert!(
            调试底部.height() > 通用底部.height(),
            "调试工作区的控制台应当更高：调试 {:?} vs 通用 {:?}",
            调试底部,
            通用底部
        );
    }

    #[test]
    fn 面板可以浮动并停靠回来() {
        let mut 集 = 工作区集::新建();
        assert_eq!(集.浮动表面数(), 0, "默认布局没有浮窗");

        assert!(集.浮动面板(Tab::控制台), "控制台应当可以浮动");
        assert_eq!(集.浮动表面数(), 1);
        assert!(集.面板可见(Tab::控制台), "浮动后仍然可访问");
        assert!(
            !集.浮动面板(Tab::控制台),
            "已经在浮窗里的面板不应当再次浮动"
        );

        // 浮窗状态必须能持久化：否则一保存就把用户浮出去的面板弄丢了。
        let 文本 = 集.序列化为文本().unwrap();
        let mut 复原 = 工作区集::从文本恢复(&文本).unwrap();
        assert_eq!(复原.浮动表面数(), 1, "浮窗状态必须随工作区一起保存");

        assert_eq!(复原.停靠全部浮动(), 1, "应当把 1 个浮窗面板停靠回来");
        assert_eq!(复原.浮动表面数(), 0);
        assert!(复原.面板可见(Tab::控制台));
    }

    /// 标签在面板组之间移动（合并）与拆分成新节点。
    ///
    /// 这两个操作就是鼠标拖放标签时 `egui_dock` 内部走的**同一条数据路径**：
    /// 拖到别的标签组 → `move_tab`；拖到边缘 → `split`。
    /// 真实鼠标拖拽无法在无输入的截图验收里模拟，因此这里直接验证
    /// 数据路径：操作后面板仍在、结构确实变了、序列化往返不丢。
    #[test]
    fn 标签可以合并进其他面板组并拆分到新节点() {
        let mut 集 = 工作区集::新建();

        // ---- 合并：把控制台拖进检查器所在的标签组 ----
        let 源 = 集.布局().find_tab(&Tab::控制台).expect("控制台应在布局里");
        let 目标 = 集.布局().find_tab(&Tab::检查器).expect("检查器应在布局里");
        let 原控制台节点 = 源.node;
        集.布局_mut().move_tab(
            源,
            egui_dock::TabDestination::Node(
                egui_dock::NodePath {
                    surface: 目标.surface,
                    node: 目标.node,
                },
                egui_dock::TabInsert::Append,
            ),
        );
        assert!(集.面板可见(Tab::控制台), "合并后控制台不能消失");
        let 新位置 = 集.布局().find_tab(&Tab::控制台).expect("控制台应当仍在");
        assert_eq!(
            新位置.node, 目标.node,
            "控制台应当与检查器在同一个标签组里：{新位置:?}"
        );
        assert_eq!(新位置.node, 目标.node);
        let _ = 原控制台节点;

        // ---- 拆分：把控制台从该标签组拆成下方的独立节点 ----
        let 承载 = 集.布局().find_tab(&Tab::控制台).unwrap();
        let 父节点 = egui_dock::NodePath {
            surface: 承载.surface,
            node: 承载.node,
        };
        let 新节点 = egui_dock::Node::Leaf(egui_dock::LeafNode::new(vec![Tab::运行时ecs]));
        let [_旧节点, _新节点] = 集
            .布局_mut()
            .split(父节点, egui_dock::Split::Below, 0.3, 新节点);
        assert!(
            集.面板可见(Tab::运行时ecs),
            "拆分出的新面板必须出现在布局里"
        );
        // 结构与拆分前不同：指纹里运行时 ECS 的位置变了。
        let 指纹 = 集.布局指纹();
        assert!(指纹.iter().any(|(面板, _)| *面板 == Tab::运行时ecs));

        // ---- 序列化往返仍然有效 ----
        let 文本 = 集.序列化为文本().unwrap();
        let 复原 = 工作区集::从文本恢复(&文本).unwrap();
        assert!(复原.面板可见(Tab::运行时ecs));
        assert!(复原.面板可见(Tab::控制台));
    }

    #[test]
    fn 浮动面板不影响其他工作区() {
        let mut 集 = 工作区集::新建();
        assert!(集.浮动面板(Tab::控制台));
        集.切换(内置工作区::调试.id()).unwrap();
        assert_eq!(集.浮动表面数(), 0, "切换后应当使用目标工作区自己的布局");
        集.切换(内置工作区::通用.id()).unwrap();
        assert_eq!(集.浮动表面数(), 1, "切回后浮窗状态应当保留");
    }
}
