//! 编辑器界面主题：颜色、间距与 egui 样式设置。
//!
//! # 为什么单独抽一层主题
//!
//! 面板要画"选中高亮""隐藏""锁定""网格""坐标轴"这些语义颜色，
//! 如果每个面板各写各的常量，很快就会出现"场景树里的选中蓝
//! 和视口里的选中蓝不是同一种蓝"这类不一致。集中在一处，
//! 改一次全局生效，也让将来做主题切换只需要动这个文件。

use egui::{Color32, CornerRadius, Stroke};

/// 编辑器配色。
#[derive(Debug, Clone, Copy)]
pub struct 编辑器配色 {
    /// 面板背景。
    pub 面板背景: Color32,
    /// 稍深的分隔背景。
    pub 面板次背景: Color32,
    /// 主文字。
    pub 文字: Color32,
    /// 次要文字（提示、类型名）。
    pub 次要文字: Color32,
    /// 强调色（选中、焦点）。
    pub 强调: Color32,
    /// 强调色的淡版本，用于填充选中背景。
    pub 强调淡: Color32,
    /// 警告。
    pub 警告: Color32,
    /// 错误。
    pub 错误: Color32,
    /// 成功/正常。
    pub 成功: Color32,
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
}

impl Default for 编辑器配色 {
    fn default() -> Self {
        Self {
            面板背景: Color32::from_rgb(30, 31, 34),
            面板次背景: Color32::from_rgb(24, 25, 28),
            文字: Color32::from_rgb(222, 226, 230),
            次要文字: Color32::from_rgb(140, 148, 158),
            强调: Color32::from_rgb(86, 156, 214),
            强调淡: Color32::from_rgba_premultiplied(40, 70, 100, 120),
            警告: Color32::from_rgb(220, 180, 80),
            错误: Color32::from_rgb(232, 106, 106),
            成功: Color32::from_rgb(110, 190, 120),
            视口背景: Color32::from_rgb(22, 23, 26),
            网格主: Color32::from_rgb(70, 74, 80),
            网格次: Color32::from_rgb(44, 47, 52),
            轴_x: Color32::from_rgb(200, 80, 80),
            轴_y: Color32::from_rgb(90, 180, 90),
            实体框: Color32::from_rgb(120, 130, 145),
        }
    }
}

impl 编辑器配色 {
    /// 把配色应用到 egui 上下文。
    ///
    /// 用 `all_styles_mut` 而不是 `set_style`：egui 0.36 把样式拆成了
    /// 亮色与暗色两份（`Theme::Dark` / `Theme::Light`），而
    /// `Context::set_style` 已被移除。这里两份都设成同一套深色配色，
    /// 保证无论系统主题如何，编辑器外观都稳定一致 ——
    /// 一个跟随系统变色的编辑器会让用户截的图与文档对不上。
    pub fn 应用(&self, 上下文: &egui::Context) {
        上下文.all_styles_mut(|样式| {
            样式.visuals.dark_mode = true;
            样式.visuals.panel_fill = self.面板背景;
            样式.visuals.window_fill = self.面板背景;
            样式.visuals.extreme_bg_color = self.面板次背景;
            样式.visuals.override_text_color = Some(self.文字);
            样式.visuals.selection.bg_fill = self.强调;
            样式.visuals.selection.stroke = Stroke::new(1.0, self.文字);
            样式.visuals.hyperlink_color = self.强调;
            样式.visuals.window_corner_radius = CornerRadius::same(4);
            样式.visuals.widgets.noninteractive.bg_stroke =
                Stroke::new(1.0, Color32::from_rgb(52, 55, 60));
            样式.visuals.widgets.inactive.corner_radius = CornerRadius::same(3);
            样式.visuals.widgets.hovered.corner_radius = CornerRadius::same(3);
            样式.visuals.widgets.active.corner_radius = CornerRadius::same(3);

            // 稍微紧凑一些：编辑器一屏要放很多控件，默认间距偏松。
            样式.spacing.item_spacing = egui::vec2(6.0, 4.0);
            样式.spacing.button_padding = egui::vec2(8.0, 3.0);
            样式.spacing.indent = 14.0;
            样式.spacing.interact_size.y = 20.0;
        });
    }

    /// 选中状态的填充色。
    pub fn 选中填充(&self, 是否主选中: bool) -> Color32 {
        if 是否主选中 {
            self.强调淡
        } else {
            Color32::from_rgba_premultiplied(40, 70, 100, 60)
        }
    }
}
