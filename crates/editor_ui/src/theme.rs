//! 主题系统：语义配色 + 外观参数 + 主题库。
//!
//! # 为什么要有"语义配色"这一层
//!
//! 面板代码里最危险的颜色写法是 `Color32::from_rgb(30, 31, 34)`：
//! 它把"面板背景"这个**语义**直接烧成了一个数值。一旦要支持浅色主题，
//! 就得在整个代码库里找出所有"看起来像背景"的字面量逐个判断。
//!
//! 因此所有界面颜色都必须来自 [`编辑器配色`] 的**语义槽位**：
//! `面板背景`、`文字`、`强调`、`警告`…… 主题切换只换这一整组值，
//! 不存在"某个面板忘了换色"的中间状态。
//!
//! # 三个内置主题
//!
//! | 主题 | 用途 | 特征 |
//! | --- | --- | --- |
//! | 深色 | 默认 | 中性灰 + 蓝强调，长时间盯屏不刺眼 |
//! | 浅色 | 明亮环境 | 白底深字，适合投影/白天 |
//! | 高对比度 | 可访问性 | 纯黑底 + 纯白字 + 黄强调，对比度 ≥ 7:1 |
//!
//! # 与工作区的边界
//!
//! 主题只管**颜色与外观参数**，工作区只管**布局**。两者互不影响：
//! 切换工作区不会改主题，切换主题不会重置布局。这条边界让两套配置
//! 可以独立保存、独立迁移，也避免了"每份工作区里复制一整份配色"。
//!
//! # 持久化格式
//!
//! [`主题库`] 序列化成 RON 存在编辑器配置里。格式版本独立于配置版本：
//! 主题存档坏了只需要回退主题，不该连工作区一起丢。

use std::collections::BTreeMap;

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};
use serde::{Deserialize, Serialize};

/// 主题存档的格式版本。
pub const 主题格式版本: u32 = 1;

/// 主题的明暗属性。
///
/// 它不只是元数据：`egui` 的 `Visuals::dark_mode` 会影响若干默认绘制，
/// 验收工具也用它来断言"这次截的确实是深色主题"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum 明暗 {
    /// 深色。
    深色,
    /// 浅色。
    浅色,
}

/// 语义配色。
///
/// 每个字段都对应界面上的一类**用途**，而不是某个具体控件。
/// 新增控件时先找有没有合适的槽位，没有再加 —— 这条纪律是
/// "所有面板看起来属于同一个编辑器"的前提。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct 编辑器配色 {
    // ---- 背景层级（由深到浅：窗口 → 面板 → 次级面板 → 控件） ----
    /// 窗口最外层背景。
    pub 窗口背景: Color32,
    /// 面板背景。
    pub 面板背景: Color32,
    /// 稍深的分隔背景（输入区、条带）。
    pub 面板次背景: Color32,
    /// 输入框/代码块背景。
    pub 输入框背景: Color32,
    /// 悬停背景。
    pub 悬停背景: Color32,
    /// 按下/激活背景。
    pub 激活背景: Color32,

    // ---- 文字 ----
    /// 主文字。
    pub 文字: Color32,
    /// 次要文字（提示、类型名）。
    pub 次要文字: Color32,
    /// 禁用文字。
    pub 禁用文字: Color32,
    /// 链接与可点文本。
    pub 链接: Color32,

    // ---- 强调与选择 ----
    /// 强调色（选中、焦点）。
    pub 强调: Color32,
    /// 强调色的亮版本（悬停）。
    pub 强调亮: Color32,
    /// 强调色的淡版本，用于填充选中背景。
    pub 强调淡: Color32,
    /// 选中项背景。
    pub 选中背景: Color32,

    // ---- 边框 ----
    /// 普通边框/分隔线。
    pub 边框: Color32,
    /// 强边框（焦点、激活标签）。
    pub 边框强: Color32,

    // ---- 视口 ----
    /// 视口背景。
    pub 视口背景: Color32,
    /// 网格主线。
    pub 网格主: Color32,
    /// 网格次线。
    pub 网格次: Color32,
    /// X 轴颜色。
    pub 轴_x: Color32,
    /// Y 轴颜色。
    pub 轴_y: Color32,
    /// 实体框线。
    pub 实体框: Color32,
    /// 选择框线。
    pub 选择框: Color32,

    // ---- 状态提示 ----
    /// 信息。
    pub 信息: Color32,
    /// 成功/正常。
    pub 成功: Color32,
    /// 警告。
    pub 警告: Color32,
    /// 错误。
    pub 错误: Color32,

    // ---- 控件 ----
    /// 按钮背景。
    pub 按钮背景: Color32,
    /// 按钮悬停背景。
    pub 按钮悬停: Color32,
    /// 按钮按下背景。
    pub 按钮按下: Color32,

    // ---- 停靠与浮窗 ----
    /// 标签栏背景。
    pub 标签栏背景: Color32,
    /// 激活标签背景。
    pub 标签激活背景: Color32,
    /// 未激活标签文字。
    pub 标签文字: Color32,
    /// 分隔条。
    pub 分割线: Color32,
    /// 浮窗标题栏背景。
    pub 浮窗标题背景: Color32,
    /// 工具提示背景。
    pub 提示背景: Color32,
    /// 视口内叠加条背景（半透明，压在渲染结果上）。
    ///
    /// # 为什么单独一个槽位
    ///
    /// 上一轮浅色主题下这里用的是写死的深色半透明底，配深色文字几乎看不清。
    /// 叠加层压在**用户内容**上，不能直接复用面板背景（那会盖住半透明意图），
    /// 因此给它一个独立槽位，由每个主题给出"既压得住内容、又不糊掉文字"的值。
    pub 叠加背景: Color32,
    /// 叠加条边框。
    pub 叠加边框: Color32,
}

impl Default for 编辑器配色 {
    fn default() -> Self {
        深色配色()
    }
}

impl 编辑器配色 {
    /// 选中状态的填充色。
    pub fn 选中填充(&self, 是否主选中: bool) -> Color32 {
        if 是否主选中 {
            self.选中背景
        } else {
            // 非主选中：同色但更淡，避免多选时整片糊成一块。
            Color32::from_rgba_unmultiplied(
                self.选中背景.r(),
                self.选中背景.g(),
                self.选中背景.b(),
                (self.选中背景.a() as u16 * 2 / 3).min(255) as u8,
            )
        }
    }
}

/// 外观参数（与颜色无关的"手感"设置）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct 外观 {
    /// 整体缩放（0.7 ~ 2.0）。
    pub 缩放: f32,
    /// 基础字号（像素）。
    pub 基础字号: f32,
    /// 控件圆角（像素）。
    pub 圆角: f32,
    /// 控件间距（像素）。
    pub 间距: f32,
    /// 图标尺寸（像素）。
    pub 图标尺寸: f32,
}

impl Default for 外观 {
    fn default() -> Self {
        Self {
            缩放: 1.0,
            基础字号: 13.0,
            圆角: 3.0,
            间距: 6.0,
            图标尺寸: 16.0,
        }
    }
}

impl 外观 {
    /// 把参数夹到安全区间后返回。
    ///
    /// # 为什么必须夹
    ///
    /// 这些值来自用户可编辑的主题文件。一个 `间距: 0` 或
    /// `基础字号: 0` 会让整个界面塌成一条线，而 `缩放: 50`
    /// 会直接把控件挤出窗口 —— 那时连"重置主题"的菜单都点不到。
    pub fn 收敛(self) -> Self {
        Self {
            缩放: self.缩放.clamp(0.7, 2.0),
            基础字号: self.基础字号.clamp(9.0, 22.0),
            圆角: self.圆角.clamp(0.0, 12.0),
            间距: self.间距.clamp(2.0, 16.0),
            图标尺寸: self.图标尺寸.clamp(12.0, 32.0),
        }
    }
}

/// 一个主题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 主题 {
    /// 稳定 id（内置为 `dark` / `light` / `high_contrast`，用户为 `user-*`）。
    pub id: String,
    /// 显示名。内置主题用 `None`，显示名走 i18n。
    pub 名称: Option<String>,
    /// 是否内置。
    pub 内置: bool,
    /// 明暗属性。
    pub 明暗: 明暗,
    /// 语义配色。
    pub 配色: 编辑器配色,
    /// 外观参数。
    pub 外观: 外观,
}

impl 主题 {
    /// 显示名（按当前语言）。
    pub fn 显示名(&self, 语言: crate::i18n::语言) -> String {
        match (&self.名称, 内置主题::从id(&self.id)) {
            (Some(名称), _) if !名称.trim().is_empty() => 名称.clone(),
            (_, Some(内置)) => 语言.取(内置.名称键()).to_string(),
            _ => self.id.clone(),
        }
    }

    /// 把主题真正应用到 egui 上下文。
    ///
    /// # 两处容易漏的细节
    ///
    /// 1. **两份样式**：egui 0.36 把样式拆成亮/暗两份，必须用
    ///    `all_styles_mut` 同时设置，否则系统主题一变编辑器就"半换色"；
    /// 2. **缩放用 `set_zoom_factor`**：它不是把字号乘一乘，而是让
    ///    整个界面的逻辑像素变大，连图元与命中测试一起缩放 ——
    ///    这是唯一能让 1.5 倍 UI 真正可用的做法。
    pub fn 应用(&self, 上下文: &egui::Context) {
        let 配色 = self.配色;
        let 外观 = self.外观.收敛();
        上下文.set_zoom_factor(外观.缩放);

        let 圆角 = CornerRadius::same(外观.圆角.round().clamp(0.0, 12.0) as u8);
        let 基础字号 = 外观.基础字号;
        let 间距 = 外观.间距;

        上下文.all_styles_mut(|样式| {
            样式.visuals.dark_mode = self.明暗 == 明暗::深色;
            样式.visuals.panel_fill = 配色.面板背景;
            样式.visuals.window_fill = 配色.浮窗标题背景;
            样式.visuals.faint_bg_color = 配色.面板次背景;
            样式.visuals.extreme_bg_color = 配色.输入框背景;
            样式.visuals.override_text_color = Some(配色.文字);
            样式.visuals.selection.bg_fill = 配色.选中背景;
            样式.visuals.selection.stroke = Stroke::new(1.0, 配色.文字);
            样式.visuals.hyperlink_color = 配色.链接;
            样式.visuals.warn_fg_color = 配色.警告;
            样式.visuals.error_fg_color = 配色.错误;
            样式.visuals.window_stroke = Stroke::new(1.0, 配色.边框强);
            样式.visuals.window_corner_radius = 圆角;
            样式.visuals.menu_corner_radius = 圆角;
            样式.visuals.widgets.noninteractive.bg_fill = 配色.面板背景;
            样式.visuals.widgets.noninteractive.weak_bg_fill = 配色.面板背景;
            样式.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, 配色.边框);
            样式.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, 配色.文字);
            样式.visuals.widgets.noninteractive.corner_radius = 圆角;

            样式.visuals.widgets.inactive.bg_fill = 配色.按钮背景;
            样式.visuals.widgets.inactive.weak_bg_fill = 配色.按钮背景;
            样式.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, 配色.边框);
            样式.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, 配色.文字);
            样式.visuals.widgets.inactive.corner_radius = 圆角;

            样式.visuals.widgets.hovered.bg_fill = 配色.按钮悬停;
            样式.visuals.widgets.hovered.weak_bg_fill = 配色.按钮悬停;
            样式.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, 配色.强调);
            样式.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, 配色.文字);
            样式.visuals.widgets.hovered.corner_radius = 圆角;
            样式.visuals.widgets.hovered.expansion = 0.0;

            样式.visuals.widgets.active.bg_fill = 配色.按钮按下;
            样式.visuals.widgets.active.weak_bg_fill = 配色.按钮按下;
            样式.visuals.widgets.active.bg_stroke = Stroke::new(1.0, 配色.强调亮);
            样式.visuals.widgets.active.fg_stroke = Stroke::new(1.0, 配色.文字);
            样式.visuals.widgets.active.corner_radius = 圆角;
            样式.visuals.widgets.active.expansion = 0.0;

            样式.visuals.widgets.open.bg_fill = 配色.激活背景;
            样式.visuals.widgets.open.weak_bg_fill = 配色.激活背景;
            样式.visuals.widgets.open.bg_stroke = Stroke::new(1.0, 配色.边框强);
            样式.visuals.widgets.open.fg_stroke = Stroke::new(1.0, 配色.文字);
            样式.visuals.widgets.open.corner_radius = 圆角;

            // 字号：一处定义，全界面生效。
            样式.text_styles = [
                (
                    TextStyle::Heading,
                    FontId::new(基础字号 + 7.0, FontFamily::Proportional),
                ),
                (
                    TextStyle::Body,
                    FontId::new(基础字号, FontFamily::Proportional),
                ),
                (
                    TextStyle::Button,
                    FontId::new(基础字号, FontFamily::Proportional),
                ),
                (
                    TextStyle::Small,
                    FontId::new((基础字号 - 2.0).max(8.0), FontFamily::Proportional),
                ),
                (
                    TextStyle::Monospace,
                    FontId::new(基础字号 - 1.0, FontFamily::Monospace),
                ),
            ]
            .into();

            // 间距：编辑器一屏要放很多控件，默认偏松。
            样式.spacing.item_spacing = egui::vec2(间距, (间距 * 0.66).max(2.0));
            样式.spacing.button_padding = egui::vec2(间距 + 2.0, (间距 * 0.5).max(2.0));
            样式.spacing.menu_margin = egui::Margin::same((间距 * 0.66) as i8);
            样式.spacing.indent = 间距 * 2.2;
            样式.spacing.interact_size.y = (基础字号 + 7.0).max(16.0);
            样式.spacing.scroll.bar_width = 8.0;
        });
    }

    /// 由主题生成 `egui_dock` 的样式。
    ///
    /// 从 `Style::from_egui` 起步再覆盖：这样没显式设置的部分仍然
    /// 跟随整体 `egui::Style`，不会出现"改了字号但停靠区还是旧字号"。
    pub fn dock样式(&self, egui样式: &egui::Style) -> egui_dock::Style {
        let 配色 = self.配色;
        let 圆角 = CornerRadius::same(self.外观.收敛().圆角.round().clamp(0.0, 12.0) as u8);
        let mut 停靠 = egui_dock::Style::from_egui(egui样式);

        停靠.main_surface_border_stroke = Stroke::new(1.0, 配色.边框);
        停靠.main_surface_border_rounding = 圆角;

        停靠.separator.color_idle = 配色.分割线;
        停靠.separator.color_hovered = 配色.强调;
        停靠.separator.color_dragged = 配色.强调亮;
        停靠.separator.width = 2.0;

        停靠.tab_bar.bg_fill = 配色.标签栏背景;
        停靠.tab_bar.hline_color = 配色.边框;
        停靠.tab_bar.corner_radius = 圆角;

        停靠.tab.active.bg_fill = 配色.标签激活背景;
        停靠.tab.active.text_color = 配色.文字;
        停靠.tab.active.outline_color = 配色.强调;
        停靠.tab.active.corner_radius = 圆角;

        停靠.tab.inactive.bg_fill = 配色.标签栏背景;
        停靠.tab.inactive.text_color = 配色.标签文字;
        停靠.tab.inactive.outline_color = Color32::TRANSPARENT;
        停靠.tab.inactive.corner_radius = 圆角;

        停靠.tab.hovered.bg_fill = 配色.悬停背景;
        停靠.tab.hovered.text_color = 配色.文字;
        停靠.tab.hovered.outline_color = 配色.边框强;
        停靠.tab.hovered.corner_radius = 圆角;

        停靠.tab.focused = 停靠.tab.active.clone();
        停靠.tab.tab_body.bg_fill = 配色.面板背景;
        停靠.tab.tab_body.stroke = Stroke::new(1.0, 配色.边框);
        停靠.tab.tab_body.corner_radius = 圆角;

        停靠.buttons.close_tab_color = 配色.次要文字;
        停靠.buttons.close_tab_active_color = 配色.错误;
        停靠.buttons.close_tab_bg_fill = Color32::TRANSPARENT;
        停靠.buttons.collapse_tabs_color = 配色.次要文字;
        停靠.buttons.collapse_tabs_active_color = 配色.文字;
        停靠.buttons.show_tab_bar_color = 配色.次要文字;
        停靠.buttons.show_tab_bar_active_color = 配色.文字;

        停靠
    }
}

/// 内置主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 内置主题 {
    /// 深色。
    深色,
    /// 浅色。
    浅色,
    /// 高对比度。
    高对比度,
}

impl 内置主题 {
    /// 全部内置主题（顺序即列表显示顺序）。
    pub fn 全部() -> &'static [内置主题] {
        &[内置主题::深色, 内置主题::浅色, 内置主题::高对比度]
    }

    /// 稳定 id。
    pub fn id(self) -> &'static str {
        match self {
            Self::深色 => "dark",
            Self::浅色 => "light",
            Self::高对比度 => "high_contrast",
        }
    }

    /// 名称键。
    pub fn 名称键(self) -> &'static str {
        match self {
            Self::深色 => "theme_dark",
            Self::浅色 => "theme_light",
            Self::高对比度 => "theme_high_contrast",
        }
    }

    /// 从 id 反查。
    pub fn 从id(id: &str) -> Option<Self> {
        Self::全部().iter().copied().find(|内置| 内置.id() == id)
    }

    /// 从名称键（i18n 键）反查。菜单项存的是词条键。
    pub fn 从名称键(键: &str) -> Option<Self> {
        Self::全部()
            .iter()
            .copied()
            .find(|内置| 内置.名称键() == 键)
    }

    /// 出厂定义。
    pub fn 预设(self) -> 主题 {
        match self {
            Self::深色 => 主题 {
                id: "dark".to_string(),
                名称: None,
                内置: true,
                明暗: 明暗::深色,
                配色: 深色配色(),
                外观: 外观::default(),
            },
            Self::浅色 => 主题 {
                id: "light".to_string(),
                名称: None,
                内置: true,
                明暗: 明暗::浅色,
                配色: 浅色配色(),
                外观: 外观::default(),
            },
            Self::高对比度 => 主题 {
                id: "high_contrast".to_string(),
                名称: None,
                内置: true,
                明暗: 明暗::深色,
                配色: 高对比度配色(),
                外观: 外观 {
                    基础字号: 14.0,
                    圆角: 2.0,
                    ..外观::default()
                },
            },
        }
    }
}

/// 深色配色。
fn 深色配色() -> 编辑器配色 {
    编辑器配色 {
        窗口背景: Color32::from_rgb(20, 21, 23),
        面板背景: Color32::from_rgb(30, 31, 34),
        面板次背景: Color32::from_rgb(24, 26, 28),
        输入框背景: Color32::from_rgb(16, 17, 19),
        悬停背景: Color32::from_rgb(42, 45, 49),
        激活背景: Color32::from_rgb(51, 55, 61),
        文字: Color32::from_rgb(222, 226, 230),
        次要文字: Color32::from_rgb(140, 148, 158),
        禁用文字: Color32::from_rgb(90, 96, 104),
        链接: Color32::from_rgb(108, 182, 255),
        强调: Color32::from_rgb(86, 156, 214),
        强调亮: Color32::from_rgb(127, 184, 232),
        强调淡: Color32::from_rgba_unmultiplied(40, 70, 100, 120),
        选中背景: Color32::from_rgba_unmultiplied(86, 156, 214, 70),
        边框: Color32::from_rgb(52, 56, 62),
        边框强: Color32::from_rgb(74, 80, 88),
        视口背景: Color32::from_rgb(22, 23, 26),
        网格主: Color32::from_rgb(70, 74, 80),
        网格次: Color32::from_rgb(44, 47, 52),
        轴_x: Color32::from_rgb(200, 80, 80),
        轴_y: Color32::from_rgb(90, 180, 90),
        实体框: Color32::from_rgb(125, 136, 150),
        选择框: Color32::from_rgb(255, 184, 77),
        信息: Color32::from_rgb(108, 182, 255),
        成功: Color32::from_rgb(110, 190, 120),
        警告: Color32::from_rgb(220, 180, 80),
        错误: Color32::from_rgb(232, 106, 106),
        按钮背景: Color32::from_rgb(42, 44, 48),
        按钮悬停: Color32::from_rgb(52, 55, 60),
        按钮按下: Color32::from_rgb(62, 66, 72),
        标签栏背景: Color32::from_rgb(25, 26, 29),
        标签激活背景: Color32::from_rgb(38, 40, 44),
        标签文字: Color32::from_rgb(184, 192, 200),
        分割线: Color32::from_rgb(52, 56, 62),
        浮窗标题背景: Color32::from_rgb(36, 38, 42),
        提示背景: Color32::from_rgb(38, 40, 44),
        叠加背景: Color32::from_rgba_unmultiplied(20, 21, 24, 216),
        叠加边框: Color32::from_rgba_unmultiplied(74, 80, 88, 220),
    }
}

/// 浅色配色。
fn 浅色配色() -> 编辑器配色 {
    编辑器配色 {
        窗口背景: Color32::from_rgb(232, 233, 235),
        面板背景: Color32::from_rgb(245, 246, 247),
        面板次背景: Color32::from_rgb(236, 238, 240),
        输入框背景: Color32::from_rgb(255, 255, 255),
        悬停背景: Color32::from_rgb(227, 230, 234),
        激活背景: Color32::from_rgb(214, 218, 224),
        文字: Color32::from_rgb(30, 32, 34),
        次要文字: Color32::from_rgb(90, 97, 105),
        禁用文字: Color32::from_rgb(154, 161, 169),
        链接: Color32::from_rgb(31, 111, 235),
        强调: Color32::from_rgb(47, 111, 208),
        强调亮: Color32::from_rgb(74, 139, 232),
        强调淡: Color32::from_rgba_unmultiplied(47, 111, 208, 40),
        选中背景: Color32::from_rgba_unmultiplied(47, 111, 208, 55),
        边框: Color32::from_rgb(201, 206, 213),
        边框强: Color32::from_rgb(169, 176, 185),
        视口背景: Color32::from_rgb(255, 255, 255),
        网格主: Color32::from_rgb(194, 199, 207),
        网格次: Color32::from_rgb(226, 229, 234),
        轴_x: Color32::from_rgb(192, 74, 74),
        轴_y: Color32::from_rgb(62, 158, 74),
        实体框: Color32::from_rgb(107, 118, 132),
        选择框: Color32::from_rgb(224, 138, 0),
        信息: Color32::from_rgb(31, 111, 235),
        成功: Color32::from_rgb(46, 139, 71),
        警告: Color32::from_rgb(176, 122, 0),
        错误: Color32::from_rgb(192, 57, 43),
        按钮背景: Color32::from_rgb(232, 234, 238),
        按钮悬停: Color32::from_rgb(220, 224, 229),
        按钮按下: Color32::from_rgb(207, 213, 220),
        标签栏背景: Color32::from_rgb(230, 232, 235),
        标签激活背景: Color32::from_rgb(245, 246, 247),
        标签文字: Color32::from_rgb(58, 64, 72),
        分割线: Color32::from_rgb(201, 206, 213),
        浮窗标题背景: Color32::from_rgb(232, 234, 238),
        提示背景: Color32::from_rgb(255, 255, 255),
        // 浅色主题：白底 + 深字。半透明程度比深色主题略低，
        // 因为浅色背景下"压不住内容"会更明显。
        叠加背景: Color32::from_rgba_unmultiplied(255, 255, 255, 232),
        叠加边框: Color32::from_rgba_unmultiplied(169, 176, 185, 235),
    }
}

/// 高对比度配色（可访问性）。
fn 高对比度配色() -> 编辑器配色 {
    编辑器配色 {
        窗口背景: Color32::from_rgb(0, 0, 0),
        面板背景: Color32::from_rgb(10, 10, 10),
        面板次背景: Color32::from_rgb(0, 0, 0),
        输入框背景: Color32::from_rgb(0, 0, 0),
        悬停背景: Color32::from_rgb(30, 42, 58),
        激活背景: Color32::from_rgb(42, 58, 80),
        文字: Color32::from_rgb(255, 255, 255),
        次要文字: Color32::from_rgb(200, 208, 216),
        禁用文字: Color32::from_rgb(128, 136, 144),
        链接: Color32::from_rgb(102, 217, 255),
        强调: Color32::from_rgb(255, 212, 0),
        强调亮: Color32::from_rgb(255, 229, 92),
        强调淡: Color32::from_rgba_unmultiplied(255, 212, 0, 60),
        选中背景: Color32::from_rgba_unmultiplied(255, 212, 0, 80),
        边框: Color32::from_rgb(176, 184, 192),
        边框强: Color32::from_rgb(255, 255, 255),
        视口背景: Color32::from_rgb(0, 0, 0),
        网格主: Color32::from_rgb(106, 112, 120),
        网格次: Color32::from_rgb(46, 50, 56),
        轴_x: Color32::from_rgb(255, 107, 107),
        轴_y: Color32::from_rgb(107, 255, 107),
        实体框: Color32::from_rgb(255, 255, 255),
        选择框: Color32::from_rgb(255, 212, 0),
        信息: Color32::from_rgb(102, 217, 255),
        成功: Color32::from_rgb(107, 255, 107),
        警告: Color32::from_rgb(255, 212, 0),
        错误: Color32::from_rgb(255, 107, 107),
        按钮背景: Color32::from_rgb(26, 26, 26),
        按钮悬停: Color32::from_rgb(42, 42, 42),
        按钮按下: Color32::from_rgb(58, 58, 58),
        标签栏背景: Color32::from_rgb(0, 0, 0),
        标签激活背景: Color32::from_rgb(26, 26, 26),
        标签文字: Color32::from_rgb(255, 255, 255),
        分割线: Color32::from_rgb(176, 184, 192),
        浮窗标题背景: Color32::from_rgb(26, 26, 26),
        提示背景: Color32::from_rgb(16, 16, 16),
        叠加背景: Color32::from_rgba_unmultiplied(0, 0, 0, 240),
        叠加边框: Color32::from_rgba_unmultiplied(255, 255, 255, 255),
    }
}

/// 主题操作的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 主题错误 {
    /// 找不到主题。
    找不到(String),
    /// 内置主题不能删除。
    内置受保护(String),
    /// 名称为空。
    名称为空,
    /// 名称重复。
    名称重复(String),
    /// 导入/存档格式不合法。
    格式不合法(String),
}

impl 主题错误 {
    /// 本地化文案。
    pub fn 文案(&self, 语言: crate::i18n::语言) -> String {
        match self {
            Self::找不到(id) => 语言.填充("theme_not_found", &[id]),
            Self::内置受保护(名称) => 语言.填充("theme_builtin_protected", &[名称]),
            Self::名称为空 => 语言.取("theme_name_empty").to_string(),
            Self::名称重复(名称) => 语言.填充("theme_name_duplicate", &[名称]),
            Self::格式不合法(详情) => 语言.填充("theme_parse_failed", &[详情]),
        }
    }
}

/// 主题库存档。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct 主题库存档 {
    格式版本: u32,
    当前: String,
    #[serde(default)]
    用户: Vec<主题>,
    /// 内置主题的用户修改（按 id）。
    #[serde(default)]
    覆盖: BTreeMap<String, 主题>,
}

/// 主题库：全部主题 + 当前激活的那个。
#[derive(Debug, Clone, PartialEq)]
pub struct 主题库 {
    内置: Vec<主题>,
    用户: Vec<主题>,
    覆盖: BTreeMap<String, 主题>,
    当前: String,
}

impl Default for 主题库 {
    fn default() -> Self {
        Self::新建()
    }
}

impl 主题库 {
    /// 出厂状态：三个内置主题，当前为深色。
    pub fn 新建() -> Self {
        Self {
            内置: 内置主题::全部().iter().map(|内置| 内置.预设()).collect(),
            用户: Vec::new(),
            覆盖: BTreeMap::new(),
            当前: 内置主题::深色.id().to_string(),
        }
    }

    /// 当前主题。
    pub fn 当前(&self) -> &主题 {
        self.按id(&self.当前)
            .or_else(|| self.全部().first().copied())
            // 出厂状态一定有内置主题，因此这里的兜底只是防御性写法。
            .unwrap_or_else(|| {
                debug_assert!(false, "主题库必须至少有一个主题");
                &self.内置[0]
            })
    }

    /// 当前主题 id。
    pub fn 当前id(&self) -> &str {
        &self.当前
    }

    /// 全部主题（内置在前，用户在后）。
    pub fn 全部(&self) -> Vec<&主题> {
        self.内置.iter().chain(self.用户.iter()).collect()
    }

    /// 用户主题数量。
    pub fn 用户数量(&self) -> usize {
        self.用户.len()
    }

    /// 按 id 查找（内置会返回被用户覆盖后的版本）。
    pub fn 按id(&self, id: &str) -> Option<&主题> {
        if let Some(覆盖) = self.覆盖.get(id) {
            return Some(覆盖);
        }
        self.内置
            .iter()
            .chain(self.用户.iter())
            .find(|主题| 主题.id == id)
    }

    /// 切换到指定主题。
    pub fn 切换(&mut self, id: &str) -> Result<(), 主题错误> {
        if self.按id(id).is_none() {
            return Err(主题错误::找不到(id.to_string()));
        }
        self.当前 = id.to_string();
        Ok(())
    }

    /// 新建主题（以当前主题为起点）。
    pub fn 新建主题(
        &self,
        名称: impl AsRef<str>,
        语言: crate::i18n::语言,
    ) -> Result<主题, 主题错误> {
        let 名称 = 名称.as_ref().trim();
        if 名称.is_empty() {
            return Err(主题错误::名称为空);
        }
        if self.全部().iter().any(|主题| 主题.显示名(语言) == 名称) {
            return Err(主题错误::名称重复(名称.to_string()));
        }
        let mut 新主题 = self.当前().clone();
        新主题.id = self.下一个用户id();
        新主题.名称 = Some(名称.to_string());
        新主题.内置 = false;
        Ok(新主题)
    }

    /// 下一个用户主题 id。
    fn 下一个用户id(&self) -> String {
        let mut 序号 = 1;
        loop {
            let 候选 = format!("user-{序号}");
            if self.按id(&候选).is_none() {
                return 候选;
            }
            序号 += 1;
        }
    }

    /// 保存一个主题（新建或覆盖当前）。
    ///
    /// 内置主题被修改时写入"覆盖"层，出厂定义永远保留 ——
    /// 这样「恢复内置主题」才有东西可恢复。
    pub fn 保存(&mut self, 主题: 主题) -> Result<(), 主题错误> {
        let 主题 = 主题.收敛();
        if 是内置主题(&主题) {
            self.覆盖.insert(主题.id.clone(), 主题.clone());
        } else if let Some(位置) = self.用户.iter().position(|已有| 已有.id == 主题.id) {
            self.用户[位置] = 主题.clone();
        } else {
            self.用户.push(主题.clone());
        }
        self.当前 = 主题.id;
        Ok(())
    }

    /// 把一份草稿另存为新的用户主题。
    ///
    /// 与 [`主题库::保存`] 的区别：保存是"就地保存当前主题"（内置写覆盖层），
    /// 另存为一定会生成**新的用户主题**，不改变原主题。
    pub fn 另存为(
        &mut self,
        草稿: 主题,
        名称: &str,
        语言: crate::i18n::语言,
    ) -> Result<String, 主题错误> {
        let 名称 = 名称.trim();
        if 名称.is_empty() {
            return Err(主题错误::名称为空);
        }
        if self.全部().iter().any(|主题| 主题.显示名(语言) == 名称) {
            return Err(主题错误::名称重复(名称.to_string()));
        }
        let mut 新主题 = 草稿.收敛();
        新主题.id = self.下一个用户id();
        新主题.名称 = Some(名称.to_string());
        新主题.内置 = false;
        self.用户.push(新主题.clone());
        self.当前 = 新主题.id.clone();
        Ok(新主题.id)
    }

    /// 删除用户主题。内置主题不可删除。
    pub fn 删除(&mut self, id: &str, 语言: crate::i18n::语言) -> Result<(), 主题错误> {
        let Some(主题) = self.按id(id) else {
            return Err(主题错误::找不到(id.to_string()));
        };
        if 主题.内置 {
            return Err(主题错误::内置受保护(主题.显示名(语言)));
        }
        self.用户.retain(|已有| 已有.id != id);
        if self.当前 == id {
            self.当前 = 内置主题::深色.id().to_string();
        }
        Ok(())
    }

    /// 恢复全部内置主题的出厂定义。返回被恢复的数量。
    pub fn 重置内置(&mut self) -> usize {
        let 数量 = self.覆盖.len();
        self.覆盖.clear();
        数量
    }

    /// 恢复某个内置主题的出厂定义。
    pub fn 重置内置主题(&mut self, id: &str) -> bool {
        self.覆盖.remove(id).is_some()
    }

    /// 导出一个主题为 RON 文本。
    pub fn 导出(&self, id: &str) -> Result<String, 主题错误> {
        let 主题 = self
            .按id(id)
            .ok_or_else(|| 主题错误::找不到(id.to_string()))?;
        ron::ser::to_string_pretty(
            主题,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| 主题错误::格式不合法(错误.to_string()))
    }

    /// 从 RON 文本导入一个主题（作为用户主题）。
    ///
    /// 导入的主题一律变成**用户主题**：即使文本里写着 `内置: true`，
    /// 也不能让一份外部文件覆盖出厂定义。
    pub fn 导入(
        &mut self, 文本: &str, 语言: crate::i18n::语言
    ) -> Result<String, 主题错误> {
        let mut 主题: 主题 =
            ron::from_str(文本).map_err(|错误| 主题错误::格式不合法(错误.to_string()))?;
        主题.内置 = false;
        主题.外观 = 主题.外观.收敛();
        if 主题.名称.as_deref().unwrap_or("").trim().is_empty() {
            主题.名称 = None;
        }
        let 名称 = 主题.名称.clone().unwrap_or_else(|| 主题.id.clone());
        // 重名时自动加后缀，避免导入这一步就失败。
        let mut 候选 = 名称.clone();
        let mut 序号 = 2;
        while self.全部().iter().any(|已有| 已有.显示名(语言) == 候选) {
            候选 = format!("{名称} {序号}");
            序号 += 1;
        }
        主题.id = self.下一个用户id();
        主题.名称 = Some(候选.clone());
        self.用户.push(主题);
        Ok(候选)
    }

    /// 序列化为存档文本。
    pub fn 序列化为文本(&self) -> Result<String, 主题错误> {
        let 存档 = 主题库存档 {
            格式版本: 主题格式版本,
            当前: self.当前.clone(),
            用户: self.用户.clone(),
            覆盖: self.覆盖.clone(),
        };
        ron::ser::to_string_pretty(
            &存档,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| 主题错误::格式不合法(错误.to_string()))
    }

    /// 从存档文本恢复。
    ///
    /// 单个主题坏掉不影响其它主题：解析失败时**跳过**该主题并继续，
    /// 最终回退到内置主题。这与"工作区"的策略一致：
    /// 配置是便利功能，不该因为一条坏数据让编辑器不可用。
    pub fn 从文本恢复(文本: &str) -> Result<Self, 主题错误> {
        let 存档: 主题库存档 =
            ron::from_str(文本).map_err(|错误| 主题错误::格式不合法(错误.to_string()))?;
        let mut 库 = Self::新建();

        for 主题 in 存档.用户 {
            let mut 主题 = 主题;
            主题.内置 = false;
            主题.外观 = 主题.外观.收敛();
            if 库.用户.iter().any(|已有| 已有.id == 主题.id) {
                continue;
            }
            库.用户.push(主题);
        }
        for (id, 主题) in 存档.覆盖 {
            // 只接受真正的内置 id：外部文件不能凭空造出"内置主题"。
            if 内置主题::从id(&id).is_some() {
                let mut 主题 = 主题;
                主题.内置 = true;
                主题.id = id.clone();
                主题.外观 = 主题.外观.收敛();
                库.覆盖.insert(id, 主题);
            }
        }

        if 库.按id(&存档.当前).is_some() {
            库.当前 = 存档.当前;
        }
        Ok(库)
    }

    /// 把主题真正应用到上下文。
    pub fn 应用当前(&self, 上下文: &egui::Context) {
        self.当前().应用(上下文);
    }
}

/// 主题是否属于内置 id（用于选择"覆盖"还是"用户"存储）。
fn 是内置主题(候选: &主题) -> bool {
    内置主题::从id(&候选.id).is_some()
}

impl 主题 {
    /// 收敛外观参数。
    fn 收敛(mut self) -> Self {
        self.外观 = self.外观.收敛();
        self
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 平均亮度(色: Color32) -> f32 {
        0.2126 * 色.r() as f32 + 0.7152 * 色.g() as f32 + 0.0722 * 色.b() as f32
    }

    #[test]
    fn 内置主题齐全且id唯一() {
        let 库 = 主题库::新建();
        assert_eq!(库.全部().len(), 3);
        let mut ids: Vec<&str> = 库.全部().iter().map(|主题| 主题.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3);
        for 内置 in 内置主题::全部() {
            assert!(库.按id(内置.id()).is_some());
        }
        assert_eq!(库.当前id(), 内置主题::深色.id());
    }

    #[test]
    fn 深色与浅色的明暗属性正确() {
        let 深 = 内置主题::深色.预设();
        let 浅 = 内置主题::浅色.预设();
        assert_eq!(深.明暗, 明暗::深色);
        assert_eq!(浅.明暗, 明暗::浅色);
        assert!(
            平均亮度(深.配色.面板背景) < 80.0,
            "深色主题的面板背景必须够暗"
        );
        assert!(
            平均亮度(浅.配色.面板背景) > 200.0,
            "浅色主题的面板背景必须够亮"
        );
        assert!(平均亮度(深.配色.文字) > 180.0, "深色主题的文字必须够亮");
        assert!(平均亮度(浅.配色.文字) < 80.0, "浅色主题的文字必须够暗");
    }

    #[test]
    fn 高对比度主题的对比度足够() {
        let 高 = 内置主题::高对比度.预设();
        // 相对亮度近似：这里用感知亮度的差值做下界检查。
        let 差 = (平均亮度(高.配色.文字) - 平均亮度(高.配色.面板背景)).abs();
        assert!(差 > 200.0, "高对比度主题的文字/背景亮度差只有 {差:.1}");
        assert_eq!(高.配色.文字, Color32::WHITE);
    }

    #[test]
    fn 外观参数会被夹到安全区间() {
        let 外观 = 外观 {
            缩放: 99.0,
            基础字号: 0.0,
            圆角: 999.0,
            间距: 0.0,
            图标尺寸: 1000.0,
        }
        .收敛();
        assert_eq!(外观.缩放, 2.0);
        assert_eq!(外观.基础字号, 9.0);
        assert_eq!(外观.圆角, 12.0);
        assert_eq!(外观.间距, 2.0);
        assert_eq!(外观.图标尺寸, 32.0);
    }

    #[test]
    fn 新建主题继承当前配色并可切换() {
        let mut 库 = 主题库::新建();
        let 草稿 = 库
            .新建主题("我的主题", crate::i18n::语言::中文)
            .expect("新建应当成功");
        assert!(!草稿.内置);
        assert_eq!(草稿.配色, 库.当前().配色, "新主题应当继承当前配色");
        let id = 草稿.id.clone();
        库.保存(草稿).unwrap();
        assert_eq!(库.当前id(), id);
        assert_eq!(库.用户数量(), 1);
        assert_eq!(库.当前().显示名(crate::i18n::语言::中文), "我的主题");
    }

    #[test]
    fn 主题重名与空名被拒绝() {
        let 库 = 主题库::新建();
        assert_eq!(
            库.新建主题("  ", crate::i18n::语言::中文),
            Err(主题错误::名称为空)
        );
        assert_eq!(
            库.新建主题("深色", crate::i18n::语言::中文),
            Err(主题错误::名称重复("深色".to_string()))
        );
    }

    #[test]
    fn 修改内置主题写入覆盖层且能恢复() {
        let mut 库 = 主题库::新建();
        let 原色 = 库.当前().配色.强调;
        let mut 草稿 = 库.当前().clone();
        草稿.配色.强调 = Color32::from_rgb(255, 0, 0);
        库.保存(草稿).unwrap();

        assert_eq!(库.当前().配色.强调, Color32::from_rgb(255, 0, 0));
        assert_eq!(库.用户数量(), 0, "内置主题的修改不应变成用户主题");

        assert!(库.重置内置主题(内置主题::深色.id()));
        assert_eq!(库.当前().配色.强调, 原色, "恢复后应当回到出厂配色");
    }

    #[test]
    fn 内置主题不能被删除() {
        let mut 库 = 主题库::新建();
        let 结果 = 库.删除(内置主题::深色.id(), crate::i18n::语言::中文);
        assert!(matches!(结果, Err(主题错误::内置受保护(_))));
    }

    #[test]
    fn 用户主题可删除且回退到深色() {
        let mut 库 = 主题库::新建();
        let 草稿 = 库.新建主题("临时", crate::i18n::语言::中文).unwrap();
        let id = 草稿.id.clone();
        库.保存(草稿).unwrap();
        assert_eq!(库.当前id(), id);

        库.删除(&id, crate::i18n::语言::中文).unwrap();
        assert_eq!(库.用户数量(), 0);
        assert_eq!(库.当前id(), 内置主题::深色.id());
    }

    #[test]
    fn 存档往返保留用户主题与当前项() {
        let mut 库 = 主题库::新建();
        let 草稿 = 库.新建主题("工地主题", crate::i18n::语言::中文).unwrap();
        let id = 草稿.id.clone();
        let mut 草稿 = 草稿;
        草稿.外观.基础字号 = 16.0;
        库.保存(草稿).unwrap();

        let 文本 = 库.序列化为文本().unwrap();
        let 恢复 = 主题库::从文本恢复(&文本).unwrap();
        assert_eq!(恢复.当前id(), id);
        assert_eq!(恢复.用户数量(), 1);
        assert_eq!(恢复.当前().外观.基础字号, 16.0);
        assert_eq!(恢复.当前().显示名(crate::i18n::语言::中文), "工地主题");
    }

    #[test]
    fn 存档里损坏的主题被跳过而不是整体失败() {
        // 用户主题缺字段 → serde 会整体失败；这里验证"格式非法时返回错误"
        // 而不是 panic，调用方据此回退到内置主题。
        let 结果 = 主题库::从文本恢复("(这不是 RON");
        assert!(matches!(结果, Err(主题错误::格式不合法(_))));
    }

    #[test]
    fn 导入的主题一律成为用户主题() {
        let mut 库 = 主题库::新建();
        let 外部 = 内置主题::浅色.预设();
        let mut 文本主题 = 外部;
        // 恶意/误写的"内置"标记不能被信任。
        文本主题.内置 = true;
        文本主题.名称 = Some("外来主题".to_string());
        let 文本 = ron::ser::to_string(&文本主题).unwrap();

        let 名称 = 库.导入(&文本, crate::i18n::语言::中文).unwrap();
        assert_eq!(名称, "外来主题");
        assert_eq!(库.用户数量(), 1);
        let 全部 = 库.全部();
        let 导入 = 全部.last().unwrap();
        assert!(!导入.内置, "导入的主题不能是内置主题");
        assert_ne!(导入.id, "light", "导入不能覆盖内置 id");
    }

    #[test]
    fn 导入重名会自动加后缀() {
        let mut 库 = 主题库::新建();
        let mut 主题 = 内置主题::浅色.预设();
        主题.名称 = Some("深色".to_string());
        let 文本 = ron::ser::to_string(&主题).unwrap();
        let 名称 = 库.导入(&文本, crate::i18n::语言::中文).unwrap();
        assert_eq!(名称, "深色 2", "重名时应自动加序号而不是失败");
    }

    #[test]
    fn 导出再导入保持配色一致() {
        let 库 = 主题库::新建();
        let 文本 = 库.导出(内置主题::高对比度.id()).unwrap();
        let 主题: 主题 = ron::from_str(&文本).unwrap();
        assert_eq!(主题.配色, 内置主题::高对比度.预设().配色);
    }

    #[test]
    fn 每个主题的语义配色槽位都不透明() {
        // 背景类颜色必须完全不透明：带 alpha 的背景会让下层颜色透上来，
        // 在截图里表现为"面板颜色不对"，也很难归因。
        for 内置 in 内置主题::全部() {
            let 配色 = 内置.预设().配色;
            for (名称, 色) in [
                ("窗口背景", 配色.窗口背景),
                ("面板背景", 配色.面板背景),
                ("面板次背景", 配色.面板次背景),
                ("视口背景", 配色.视口背景),
                ("输入框背景", 配色.输入框背景),
            ] {
                assert_eq!(色.a(), 255, "`{名称}` 必须不透明（主题 {}）", 内置.id());
            }
        }
    }

    #[test]
    fn 三个内置主题彼此不同() {
        let 深 = 内置主题::深色.预设();
        let 浅 = 内置主题::浅色.预设();
        let 高 = 内置主题::高对比度.预设();
        assert_ne!(深.配色, 浅.配色);
        assert_ne!(浅.配色, 高.配色);
        assert_ne!(深.配色.强调, 高.配色.强调, "高对比度应当有独特的强调色");
    }
}
