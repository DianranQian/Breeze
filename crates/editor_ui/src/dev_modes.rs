//! 主开发模式：2D / 3D / 脚本 / 运行 / 资源。
//!
//! # 与"布局预设"的区别（这是本模块存在的理由）
//!
//! 上一轮把 `general / scene / assets / debug` 直接当成一级"工作区"。
//! 但从产品角度它们是**两种不同的东西**：
//!
//! | | 主开发模式 | 布局预设 |
//! | --- | --- | --- |
//! | 回答的问题 | 我现在在**做什么**任务 | 面板**怎么摆** |
//! | 数量 | 固定五个（2D/3D/脚本/运行/资源） | 内置四个 + 用户任意个 |
//! | 影响 | 视口类型、专用工具栏、快捷键、活动编辑内容 | 只影响停靠布局 |
//! | 用户可否删除 | 否 | 用户预设可以 |
//!
//! Godot 的主屏幕是"2D / 3D / Script / AssetLib"，Blender 的顶部是
//! "Layout / Modeling / …" 这类**任务级**入口，二者都不是"布局"。
//! 因此这里把模式做成独立的枚举，每个模式拥有自己的一套布局集
//! （[`crate::workspaces::工作区集`]），切换模式时整体交换，
//! 用户在 2D 里整理的布局不会影响脚本模式。
//!
//! # 模式之间共享什么
//!
//! 场景文档、选择、撤销栈、主题、语言全部**共享**：
//! 切模式不会丢实体、不会清选择、不会改主题。模式只切换
//! "看这份数据的角度"与"可用的工具"。

use serde::{Deserialize, Serialize};

use crate::docks::Tab;
use crate::icons::图标;

/// 布局存档的格式版本（模式 + 每模式工作区集）。
pub const 模式存档版本: u32 = 3;

/// 主开发模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum 开发模式 {
    /// 2D 场景编辑：二维视口、网格、精灵。
    二维,
    /// 3D 场景编辑：透视视口、轨道相机、空间坐标。
    三维,
    /// 脚本编辑：代码文件、查找、诊断。
    脚本,
    /// 运行与观察：播放控制、Console、Runtime ECS、Performance。
    运行,
    /// 资源管理：项目内资产浏览、搜索、预览。
    资源,
}

impl 开发模式 {
    /// 全部模式（顺序即顶部导航顺序）。
    pub fn 全部() -> &'static [开发模式] {
        &[
            开发模式::二维,
            开发模式::三维,
            开发模式::脚本,
            开发模式::运行,
            开发模式::资源,
        ]
    }

    /// 稳定 id。**进配置文件，永不随语言变化。**
    pub fn id(self) -> &'static str {
        match self {
            Self::二维 => "2d",
            Self::三维 => "3d",
            Self::脚本 => "script",
            Self::运行 => "run",
            Self::资源 => "assets",
        }
    }

    /// 名称键。
    pub fn 名称键(self) -> &'static str {
        match self {
            Self::二维 => "mode_2d",
            Self::三维 => "mode_3d",
            Self::脚本 => "mode_script",
            Self::运行 => "mode_run",
            Self::资源 => "mode_assets",
        }
    }

    /// 该模式的说明键（顶部导航的工具提示）。
    pub fn 说明键(self) -> &'static str {
        match self {
            Self::二维 => "mode_2d_hint",
            Self::三维 => "mode_3d_hint",
            Self::脚本 => "mode_script_hint",
            Self::运行 => "mode_run_hint",
            Self::资源 => "mode_assets_hint",
        }
    }

    /// 顶部导航图标。
    pub fn 图标(self) -> 图标 {
        match self {
            Self::二维 => 图标::网格,
            Self::三维 => 图标::世界,
            Self::脚本 => 图标::代码,
            Self::运行 => 图标::运行,
            Self::资源 => 图标::资源,
        }
    }

    /// 从 id 反查。
    pub fn 从id(id: &str) -> Option<Self> {
        Self::全部().iter().copied().find(|模式| 模式.id() == id)
    }

    /// 该模式默认激活的面板（新建该模式布局时用）。
    pub fn 默认面板(self) -> Tab {
        match self {
            Self::二维 | Self::三维 => Tab::视口,
            Self::脚本 => Tab::脚本,
            Self::运行 => Tab::控制台,
            Self::资源 => Tab::资源浏览器,
        }
    }

    /// 是否使用三维视口。
    pub fn 是三维(self) -> bool {
        self == Self::三维
    }

    /// 该模式的默认布局。
    ///
    /// # 为什么每个模式给一份独立布局，而不是共用一份再"按模式隐藏面板"
    ///
    /// 隐藏面板会让布局里存在大量"看不见的标签"，用户拖动分隔条时
    /// 会发现明明只有两块却分出来五块。独立布局则让每个模式都是一份
    /// 干净、可解释的停靠树。
    pub fn 默认布局(self) -> egui_dock::DockState<Tab> {
        match self {
            Self::二维 => crate::docks::默认布局(),
            Self::三维 => 三维预设(),
            Self::脚本 => 脚本预设(),
            Self::运行 => 运行预设(),
            Self::资源 => 资源预设(),
        }
    }

    /// 该模式的默认工作区名称键（布局栏里显示的第一个布局）。
    pub fn 默认布局名称键(self) -> &'static str {
        match self {
            Self::二维 => "layout_2d_default",
            Self::三维 => "layout_3d_default",
            Self::脚本 => "layout_script_default",
            Self::运行 => "layout_run_default",
            Self::资源 => "layout_assets_default",
        }
    }
}

/// 3D 模式预设：视口优先，右列检查器/历史，底部控制台/运行时。
///
/// ```text
/// Horizontal(主区 78% | 右列 22%)          ← 检查器/历史/性能
///   └ Vertical(上方 76% | 底部 24%)        ← 控制台/运行时 ECS/资源
///       └ Horizontal(视口 82% | 场景树 18%)
/// ```
fn 三维预设() -> egui_dock::DockState<Tab> {
    let mut 布局 = egui_dock::DockState::new(vec![Tab::视口]);
    let 主 = 布局.main_surface_mut();
    let [主区, _右列] = 主.split_right(
        egui_dock::NodeIndex::root(),
        0.78,
        vec![Tab::检查器, Tab::历史, Tab::性能],
    );
    let [上方, _底部] = 主.split_below(
        主区,
        0.76,
        vec![Tab::控制台, Tab::运行时ecs, Tab::资源浏览器, Tab::脚本],
    );
    let [_视口, _场景树] = 主.split_left(上方, 0.18, vec![Tab::场景树]);
    布局
}

/// 脚本模式预设：编辑区最大，左侧场景/资源，右侧检查器，底部控制台。
///
/// ```text
/// Horizontal(左列 20% | 主区 80%)
///   主区 └ Horizontal(脚本 76% | 右列 24%)
///            右列 = 检查器/历史
///            底部 = 控制台/性能
/// ```
fn 脚本预设() -> egui_dock::DockState<Tab> {
    let mut 布局 = egui_dock::DockState::new(vec![Tab::脚本]);
    let 主 = 布局.main_surface_mut();
    let [主区, _右列] = 主.split_right(
        egui_dock::NodeIndex::root(),
        0.76,
        vec![Tab::检查器, Tab::历史],
    );
    let [上方, _底部] = 主.split_below(主区, 0.72, vec![Tab::控制台, Tab::运行时ecs, Tab::性能]);
    let [_脚本, _左列] = 主.split_left(上方, 0.20, vec![Tab::资源浏览器, Tab::场景树, Tab::视口]);
    布局
}

/// 运行模式预设：底部观察区占 45%，右侧性能/历史，视口仍在。
fn 运行预设() -> egui_dock::DockState<Tab> {
    let mut 布局 = egui_dock::DockState::new(vec![Tab::视口]);
    let 主 = 布局.main_surface_mut();
    let [主区, _右列] = 主.split_right(
        egui_dock::NodeIndex::root(),
        0.74,
        vec![Tab::性能, Tab::历史, Tab::检查器, Tab::脚本],
    );
    let [上方, _底部] = 主.split_below(主区, 0.55, vec![Tab::控制台, Tab::运行时ecs, Tab::场景树]);
    let _ = &上方;
    let [_视口, _左列] = 主.split_left(上方, 0.18, vec![Tab::资源浏览器]);
    布局
}

/// 资源模式预设：资源浏览器占中央主体。
fn 资源预设() -> egui_dock::DockState<Tab> {
    let mut 布局 = egui_dock::DockState::new(vec![Tab::资源浏览器, Tab::控制台]);
    let 主 = 布局.main_surface_mut();
    let [中区, 右列] = 主.split_right(egui_dock::NodeIndex::root(), 0.70, vec![Tab::视口]);
    let [_右上半, _右下半] = 主.split_below(右列, 0.60, vec![Tab::检查器, Tab::历史, Tab::性能]);
    let [_资源, _左列] = 主.split_left(中区, 0.30, vec![Tab::场景树, Tab::运行时ecs, Tab::脚本]);
    布局
}

/// 内置示例项目。
///
/// 与"新建项目"不同：示例是**随仓库分发**的现成项目（含 CC0 素材），
/// 用来体验完整编辑流程与作为 GUI 验收的固定夹具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 示例项目 {
    /// 二维示例：林间营地。
    二维,
    /// 三维示例：浮空圣所。
    三维,
}

impl 示例项目 {
    /// 全部示例。
    pub fn 全部() -> &'static [示例项目] {
        &[示例项目::二维, 示例项目::三维]
    }

    /// 仓库内目录名。
    pub fn 目录名(self) -> &'static str {
        match self {
            Self::二维 => "2d_demo",
            Self::三维 => "3d_demo",
        }
    }

    /// 菜单词条键。
    pub fn 名称键(self) -> &'static str {
        match self {
            Self::二维 => "open_2d_demo",
            Self::三维 => "open_3d_demo",
        }
    }

    /// 打开后进入的模式。
    pub fn 默认模式(self) -> 开发模式 {
        match self {
            Self::二维 => 开发模式::二维,
            Self::三维 => 开发模式::三维,
        }
    }
}

/// 单个模式的存档。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct 模式项 {
    /// 模式 id。
    模式: String,
    /// 该模式的工作区集文本（[`crate::workspaces::工作区集::序列化为文本`]）。
    工作区: String,
}

/// 布局存档（格式版本 3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct 布局存档 {
    格式版本: u32,
    当前模式: String,
    模式: Vec<模式项>,
}

/// 反序列化结果：模式 → 工作区集。
pub type 模式布局表 = std::collections::BTreeMap<开发模式, crate::workspaces::工作区集>;

/// 读取布局存档。
///
/// # 三级兼容策略（这是"不得删除旧配置"的落点）
///
/// 1. **版本 3**：模式 + 每模式工作区集；
/// 2. **版本 2**：上一轮的单个工作区集 → 放进 2D 模式，其他模式给出厂布局；
/// 3. **版本 1**：更早的单份 DockState → 交给
///    [`crate::workspaces::工作区集::从旧布局迁移`]，放进 2D 模式。
///
/// 任何一级失败都不会整体失败：坏掉的那部分回退到出厂布局，
/// 并把说明放进返回值里供日志展示。
pub fn 读取布局存档(
    文本: &str,
    语言: crate::i18n::语言,
) -> Result<(开发模式, 模式布局表, Vec<String>), String> {
    let mut 备注 = Vec::new();

    // ---- 版本 3 ----
    if let Ok(存档) = ron::from_str::<布局存档>(文本) {
        if 存档.格式版本 > 模式存档版本 {
            备注.push(format!(
                "布局存档版本 {} 高于本版本 {}，已按兼容模式读取",
                存档.格式版本, 模式存档版本
            ));
        }
        let mut 表 = 模式布局表::new();
        for 项 in &存档.模式 {
            let Some(模式) = 开发模式::从id(&项.模式) else {
                备注.push(format!("忽略未知模式 `{}`", 项.模式));
                continue;
            };
            match crate::workspaces::工作区集::从文本恢复(&项.工作区) {
                Ok(集) => {
                    表.insert(模式, 集);
                }
                Err(错误) => {
                    备注.push(format!(
                        "模式 `{}` 的布局不可用，已回退出厂布局：{}",
                        项.模式,
                        错误.文案(语言)
                    ));
                }
            }
        }
        let 当前 = 开发模式::从id(&存档.当前模式).unwrap_or_else(|| {
            备注.push(format!("未知的当前模式 `{}`，已回退 2D", 存档.当前模式));
            开发模式::二维
        });
        return Ok((当前, 补全模式(表, 语言), 备注));
    }

    // ---- 版本 2 ----
    if let Ok(集) = crate::workspaces::工作区集::从文本恢复(文本) {
        备注.push("已把上一轮的单一工作区集迁移到 2D 模式".to_string());
        let mut 表 = 模式布局表::new();
        表.insert(开发模式::二维, 集);
        return Ok((开发模式::二维, 补全模式(表, 语言), 备注));
    }

    // ---- 版本 1 ----
    if let Ok(集) = crate::workspaces::工作区集::从旧布局迁移(文本, 语言) {
        备注.push("已把最早版本的单份停靠布局迁移到 2D 模式".to_string());
        let mut 表 = 模式布局表::new();
        表.insert(开发模式::二维, 集);
        return Ok((开发模式::二维, 补全模式(表, 语言), 备注));
    }

    Err("布局存档无法识别（不是版本 3/2/1 的任意一种）".to_string())
}

/// 给缺失的模式补上出厂布局。
pub fn 补全模式(mut 表: 模式布局表, 语言: crate::i18n::语言) -> 模式布局表 {
    for 模式 in 开发模式::全部() {
        if 表.contains_key(模式) {
            continue;
        }
        表.insert(*模式, 出厂工作区集(*模式, 语言));
    }
    // 每个模式至少要有视口或自己的主面板；坏掉的用出厂布局顶替。
    for (模式, 集) in 表.iter_mut() {
        let 主面板 = 模式.默认面板();
        if !集.面板可见(主面板) && !集.面板可见(Tab::视口) {
            *集 = 出厂工作区集(*模式, 语言);
        }
    }
    表
}

/// 某个模式的出厂工作区集：一个以模式名命名的布局 + 内置预设。
pub fn 出厂工作区集(
    模式: 开发模式,
    语言: crate::i18n::语言,
) -> crate::workspaces::工作区集 {
    let mut 集 = crate::workspaces::工作区集::新建();
    // 把"通用"之外的三个内置预设保留，把通用换成该模式的默认布局，
    // 并让当前布局 = 模式默认布局。这样布局栏里立刻就能看到
    // "2D 默认 / 通用 / 场景 / 资源 / 调试"。
    let 名称 = 语言.取(模式.默认布局名称键()).to_string();
    let _ = 集.承认内建布局(模式.默认布局(), &名称);
    集
}

/// 序列化存档。
pub fn 写出布局存档(
    当前: 开发模式,
    表: &模式布局表,
    语言: crate::i18n::语言,
) -> Result<String, String> {
    let mut 项 = Vec::new();
    for 模式 in 开发模式::全部() {
        let Some(集) = 表.get(模式) else {
            continue;
        };
        let 文本 = 集.序列化为文本().map_err(|错误| 错误.文案(语言))?;
        项.push(模式项 {
            模式: 模式.id().to_string(),
            工作区: 文本,
        });
    }
    let 存档 = 布局存档 {
        格式版本: 模式存档版本,
        当前模式: 当前.id().to_string(),
        模式: 项,
    };
    ron::ser::to_string_pretty(
        &存档,
        ron::ser::PrettyConfig::default().new_line("\n".to_string()),
    )
    .map_err(|错误| format!("布局存档序列化失败：{错误}"))
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 五个模式齐全且id唯一() {
        assert_eq!(开发模式::全部().len(), 5);
        let mut ids: Vec<&str> = 开发模式::全部().iter().map(|模式| 模式.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5);
        for 模式 in 开发模式::全部() {
            assert_eq!(开发模式::从id(模式.id()), Some(*模式));
            assert!(模式.默认布局().find_tab(&模式.默认面板()).is_some());
        }
    }

    #[test]
    fn 每个模式默认布局都可用且彼此不同() {
        let mut 文本: Vec<String> = Vec::new();
        for 模式 in 开发模式::全部() {
            let 布局 = 模式.默认布局();
            // 视口是编辑器主体，除脚本模式外都应当有。
            if *模式 != 开发模式::脚本 {
                assert!(
                    布局.find_tab(&Tab::视口).is_some(),
                    "模式 {} 的默认布局缺少视口",
                    模式.id()
                );
            }
            assert!(
                布局.find_tab(&模式.默认面板()).is_some(),
                "模式 {} 的默认布局缺少自己的主面板",
                模式.id()
            );
            文本.push(ron::ser::to_string(&布局).unwrap_or_default());
        }
        let 原数量 = 文本.len();
        文本.sort();
        文本.dedup();
        assert_eq!(文本.len(), 原数量, "五个模式的默认布局必须彼此不同");
    }

    #[test]
    fn 存档往返保留当前模式与各模式布局() {
        let 语言 = crate::i18n::语言::中文;
        let mut 表 = 模式布局表::new();
        for 模式 in 开发模式::全部() {
            表.insert(*模式, 出厂工作区集(*模式, 语言));
        }
        let 文本 = 写出布局存档(开发模式::脚本, &表, 语言).expect("序列化应当成功");
        let (当前, 读回, 备注) = 读取布局存档(&文本, 语言).expect("读取应当成功");
        assert_eq!(当前, 开发模式::脚本);
        assert_eq!(读回.len(), 5);
        assert!(备注.is_empty(), "正常存档不应产生修复备注：{备注:?}");
        assert!(读回[&开发模式::脚本].面板可见(Tab::脚本));
    }

    #[test]
    fn 版本2存档被迁移到2d模式() {
        let 语言 = crate::i18n::语言::中文;
        let 集 = 出厂工作区集(开发模式::二维, 语言);
        let 旧文本 = 集.序列化为文本().unwrap();
        let (当前, 表, 备注) = 读取布局存档(&旧文本, 语言).expect("版本 2 应当能迁移");
        assert_eq!(当前, 开发模式::二维);
        assert_eq!(表.len(), 5, "其余模式应当补齐出厂布局");
        assert!(备注.iter().any(|说明| 说明.contains("2D")), "{备注:?}");
    }

    #[test]
    fn 损坏的单模式布局只回退该模式() {
        let 语言 = crate::i18n::语言::中文;
        let 好 = 出厂工作区集(开发模式::二维, 语言).序列化为文本().unwrap();
        let 文本 = format!(
            r#"(
    格式版本: 3,
    当前模式: "2d",
    模式: [
        (模式: "2d", 工作区: {好:?}),
        (模式: "3d", 工作区: "(((("),
    ],
)"#
        );
        let (当前, 表, 备注) = 读取布局存档(&文本, 语言).expect("单条损坏不应整体失败");
        assert_eq!(当前, 开发模式::二维);
        assert_eq!(表.len(), 5);
        assert!(
            备注.iter().any(|说明| 说明.contains("3d")),
            "必须记录哪一条坏了：{备注:?}"
        );
        assert!(
            表[&开发模式::三维].面板可见(Tab::视口),
            "坏的应回退出厂布局"
        );
    }

    #[test]
    fn 完全无法识别的存档返回错误() {
        let 语言 = crate::i18n::语言::中文;
        assert!(读取布局存档("这不是 RON", 语言).is_err());
        assert!(读取布局存档("(随便: 1)", 语言).is_err());
    }

    #[test]
    fn 未知当前模式回退到2d并留下说明() {
        let 语言 = crate::i18n::语言::中文;
        let 文本 = r#"(格式版本: 3, 当前模式: "voxel", 模式: [])"#;
        let (当前, 表, 备注) = 读取布局存档(文本, 语言).unwrap();
        assert_eq!(当前, 开发模式::二维);
        assert_eq!(表.len(), 5);
        assert!(备注.iter().any(|说明| 说明.contains("voxel")));
    }
}
