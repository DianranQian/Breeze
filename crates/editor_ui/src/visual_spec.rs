//! 中文名：视觉规范
//! 视觉规范：把"编辑器长什么样"从各面板里抽出来的一处。
//!
//! # 为什么需要这一层
//!
//! 改造前的情况是：顶部四行 chrome 各自写着 `24.0 / 30.0 / 26.0 / 28.0`，
//! 面板里各自 `add_space(4.0)` 或 `8.0`，分隔线有时是 `ui.separator()`、
//! 有时是自己画的矩形。结果是同一个界面里出现了四五种行距、三种分隔线，
//! 用户说不清哪里不对，只觉得"拼起来的"。
//!
//! 这个模块只回答三件事：
//!
//! 1. **尺子**：chrome 高度、面板内边距、行高、控件高度、节标题高度；
//! 2. **层级**：标题 / 正文 / 辅助 / 数值各用哪个文字样式与颜色；
//! 3. **复用件**：节标题、细分隔线、工具栏分组、空状态提示。
//!
//! 颜色与间距的**源头**仍然是 [`crate::编辑器配色`] 与 [`crate::外观`]：
//! 用户改主题时这些尺子跟着字号与间距缩放，而不是写死成另一套常量。
//! 也就是说：这里定义的是"比例与用法"，主题定义的是"数值与颜色"。

use egui::{Color32, RichText, Stroke, TextStyle, Ui};

use crate::state::绘制上下文;

/// 节标题文字样式。
///
/// 用它而不是 `RichText::strong()`：`strong` 只是换颜色，字号仍然是正文，
/// 分组标题与属性名看起来一样大，界面的层级就塌了。
pub fn 节标题文本样式() -> TextStyle {
    TextStyle::Name(std::sync::Arc::from("节标题"))
}

/// 顶部 chrome 的高度（逻辑像素）。
///
/// 菜单栏最矮（它只有文字），模式栏略高（它要容纳分段控件），
/// 工具栏与模式栏同高（同一行里两套按钮不能一高一低）。
pub mod 高度 {
    /// 菜单栏。
    pub const 菜单栏: f32 = 28.0;
    /// 主开发模式栏（2D / 3D / 代码 / AI Agent）。
    pub const 模式栏: f32 = 34.0;
    /// 工作区栏（布局切换）。
    pub const 工作区栏: f32 = 28.0;
    /// 工具栏（模式工具 + 运行 + 编辑 + 实体 + 面板）。
    pub const 工具栏: f32 = 34.0;
    /// 状态栏。
    pub const 状态栏: f32 = 24.0;
}

/// 面板内的留白（逻辑像素）。
pub mod 留白 {
    /// 面板四周（左右）内边距。
    pub const 面板左右: f32 = 10.0;
    /// 面板内容上下内边距。
    pub const 面板上下: f32 = 8.0;
    /// 控件之间的水平间距。
    pub const 控件水平: f32 = 6.0;
    /// 分组之间的垂直间距（比行距大一档，用来"断开"两组内容）。
    pub const 分组: f32 = 10.0;
    /// 属性名与控件之间的间距。
    pub const 标签: f32 = 8.0;
    /// 属性名列的宽度（检查器、对话框共用，保证跨面板对齐）。
    pub const 标签列宽: f32 = 64.0;
    /// 缩进一层的宽度（场景树、节点层级）。
    pub const 缩进: f32 = 14.0;
}

/// 场景树与其他行式列表的行高（逻辑像素）。
pub fn 行高(控件高度: f32) -> f32 {
    (控件高度 - 2.0).max(22.0)
}

/// 一条**安静的**分隔线。
///
/// 与 `ui.separator()` 的区别：它用主题的边框色、只占 1px，
/// 并且在上下各留 4px，不会把面板切得像表格。
pub fn 细线(ui: &mut Ui, 配色: crate::编辑器配色) {
    let 宽 = ui.available_width();
    let (矩形, _) = ui.allocate_exact_size(egui::vec2(宽, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(矩形, 0.0, 配色.边框);
}

/// 在面板顶部画一行"分区标题"。
///
/// 用法：
/// ```ignore
/// crate::visual_spec::节标题(ui, 上下文, 语言.取("transform"), Some("局部"));
/// ```
/// 右侧的 `右注` 用来放该分组的类型名、单位或状态（"局部 / 世界"、
/// "Sprite"、"启用"），字号比标题小一档、颜色退到次要文字 —— 它是注释，
/// 不是第二个标题。
pub fn 节标题(
    ui: &mut Ui,
    上下文: &绘制上下文<'_>,
    标题: impl Into<RichText>,
    右注: Option<&str>,
) {
    let 配色 = 上下文.配色;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 留白::控件水平;
        ui.label(
            标题
                .into()
                .size(上下文.外观.基础字号 - 0.5)
                .strong()
                .color(配色.文字),
        );
        if let Some(注) = 右注 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(注)
                        .size(上下文.外观.基础字号 - 2.0)
                        .color(配色.次要文字),
                );
            });
        }
    });
}

/// 空状态提示：一行标题 + 一行说明，居中偏上，不画边框盒子。
///
/// 编辑器里空状态很常见（没选中实体、没有打开项目、场景为空）。
/// 它们需要的是"告诉我下一步做什么"，不是一个带图标的卡片。
pub fn 空状态(ui: &mut Ui, 上下文: &绘制上下文<'_>, 标题: &str, 说明: &str) {
    ui.add_space(留白::分组 * 2.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(标题)
                .size(上下文.外观.基础字号)
                .color(上下文.配色.次要文字),
        );
        if !说明.is_empty() {
            ui.add_space(2.0);
            ui.label(
                RichText::new(说明)
                    .size(上下文.外观.基础字号 - 2.0)
                    .color(上下文.配色.禁用文字),
            );
        }
    });
}

/// 工具栏里的一组按钮之后的分隔。
///
/// 与 `ui.separator()` 不同：它是一根 1px、高度 16 的竖线，
/// 垂直居中，且在两侧留出固定间距 —— 工具栏分组靠"留白 + 细线"，
/// 不靠给每组套一个框。
pub fn 工具栏分组(ui: &mut Ui, 配色: crate::编辑器配色) {
    let 高 = 16.0;
    let (矩形, _) = ui.allocate_exact_size(egui::vec2(9.0, 高), egui::Sense::hover());
    let 线 = egui::Rect::from_center_size(矩形.center(), egui::vec2(1.0, 高));
    ui.painter().rect_filled(线, 0.0, 配色.边框);
}

/// 分段控件的一个标签。
///
/// 用于模式切换（2D / 3D / 代码 / AI Agent）与视图开关（网格 / 坐标轴）。
/// 视觉规则：**选中态 = 强调色的淡底 + 强调色文字**，未选中态没有底色，
/// 悬停才出现一档浅底。不用整块彩色胶囊，否则一排标签就是一面彩旗。
pub fn 分段标签(
    ui: &mut Ui,
    配色: crate::编辑器配色,
    基础字号: f32,
    文本: &str,
    图标: Option<&egui::load::SizedTexture>,
    选中: bool,
    启用: bool,
) -> egui::Response {
    let 字号 = 基础字号;
    let 高 = (基础字号 + 12.0).max(26.0);
    let (底色, 文字色, 描边) = if !启用 {
        (Color32::TRANSPARENT, 配色.禁用文字, Stroke::NONE)
    } else if 选中 {
        (配色.强调淡, 配色.强调亮, Stroke::new(1.0, 配色.强调))
    } else {
        (Color32::TRANSPARENT, 配色.次要文字, Stroke::NONE)
    };

    let 内边距 = egui::vec2(留白::控件水平 + 4.0, 0.0);
    let 图标宽 = 图标.map_or(0.0, |图| 图.size.x + 6.0);
    let 文字宽 = ui
        .painter()
        .layout_no_wrap(文本.to_string(), egui::FontId::proportional(字号), 文字色)
        .size()
        .x;
    let 宽 = (文字宽 + 内边距.x * 2.0 + 图标宽).max(34.0);

    let (矩形, 响应) = ui.allocate_exact_size(egui::vec2(宽, 高), egui::Sense::click());
    let 可见响应 = 响应.clone();
    if 响应.hovered() && 启用 && !选中 {
        ui.painter()
            .rect_filled(矩形, egui::CornerRadius::same(4), 配色.悬停背景);
    }
    if 底色 != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(矩形, egui::CornerRadius::same(4), 底色);
        ui.painter().rect_stroke(
            矩形,
            egui::CornerRadius::same(4),
            描边,
            egui::StrokeKind::Inside,
        );
    }

    let 内容宽 = 图标宽 + 文字宽;
    let mut 左 = 矩形.center().x - 内容宽 * 0.5;
    if let Some(图) = 图标 {
        let 图标矩形 =
            egui::Rect::from_min_size(egui::pos2(左, 矩形.center().y - 图.size.y * 0.5), 图.size);
        ui.painter().image(
            图.id,
            图标矩形,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            文字色,
        );
        左 += 图标宽;
    }
    ui.painter().text(
        egui::pos2(左, 矩形.center().y),
        egui::Align2::LEFT_CENTER,
        文本,
        egui::FontId::proportional(字号),
        文字色,
    );

    可见响应
}

/// 面板内容的统一内边距（交给 `egui::Frame::inner_margin`）。
///
/// # 为什么是返回 Margin 而不是"在面板里 add_space 一下"
///
/// `add_space` 只能推竖直方向，左右留白还得每处手写；用 `Frame` 的内边距
/// 则让**背景、裁剪与命中测试**都一起内缩 —— 选中行的底色不会贴到面板
/// 边缘，滚动条也不会压在内容上。检查器、场景树、控制台共用同一份。
pub fn 面板内边距() -> egui::Margin {
    egui::Margin {
        left: 留白::面板左右 as i8,
        right: 留白::面板左右 as i8,
        top: 留白::面板上下 as i8,
        bottom: 留白::面板上下 as i8,
    }
}

/// 一个**无边框**的开关标签（用于视口叠加层这类"嵌在画面里"的小控件）。
///
/// # 为什么不用 `selectable_label`
///
/// `selectable_label` 在选中时会画一块浅色底，叠在视口上像贴了一张小卡片。
/// 这里用"文字 + 一个状态点"表达开关：开启时文字用强调色、点时可见；
/// 关闭时整组退到次要文字。它不占背景，因此不会把画布切碎。
pub fn 开关标签(
    ui: &mut Ui,
    配色: crate::编辑器配色,
    字号: f32,
    文本: &str,
    开: bool,
) -> egui::Response {
    let 字体 = egui::FontId::proportional(字号);
    let 文字色 = if 开 {
        配色.强调亮
    } else {
        配色.次要文字
    };
    let 排版 = ui.painter().layout_no_wrap(文本.to_string(), 字体, 文字色);
    let 点宽 = 8.0;
    let 宽 = 排版.size().x + 点宽 + 5.0;
    let 高 = (字号 + 10.0).max(22.0);
    let (矩形, 响应) = ui.allocate_exact_size(egui::vec2(宽, 高), egui::Sense::click());
    if 响应.hovered() {
        ui.painter()
            .rect_filled(矩形, egui::CornerRadius::same(4), 配色.悬停背景);
    }
    let 点 = egui::pos2(矩形.min.x + 3.5, 矩形.center().y);
    if 开 {
        ui.painter().circle_filled(点, 2.5, 配色.强调);
    } else {
        ui.painter()
            .circle_stroke(点, 2.5, egui::Stroke::new(1.0, 配色.禁用文字));
    }
    ui.painter().galley(
        egui::pos2(
            矩形.min.x + 点宽 + 4.0,
            矩形.center().y - 排版.size().y * 0.5,
        ),
        排版,
        文字色,
    );
    响应
}

/// 纯图标的分段按钮（视口工具条用）。
///
/// # 为什么单独一个函数
///
/// 视口工具条要的是"一排图标、选中一项"，和文字分段标签的排版差别很大：
/// 图标按钮是正方形、只靠底色和描边表达选中。godot 的视口工具条就是这种
/// 形态：没有文字，悬停才出现名称与快捷键。
pub fn 图标分段按钮(
    ui: &mut Ui,
    配色: crate::编辑器配色,
    边长: f32,
    图标: &egui::load::SizedTexture,
    选中: bool,
    启用: bool,
) -> egui::Response {
    let 尺寸 = egui::vec2(边长, 边长);
    let (矩形, 响应) = ui.allocate_exact_size(尺寸, egui::Sense::click());
    let 圆角 = egui::CornerRadius::same(5);
    if !启用 {
        // 禁用：不画底、图标本身降透明（由调用方给灰的 tint）。
    } else if 选中 {
        ui.painter().rect_filled(矩形, 圆角, 配色.强调淡);
        ui.painter().rect_stroke(
            矩形,
            圆角,
            egui::Stroke::new(1.0, 配色.强调),
            egui::StrokeKind::Inside,
        );
    } else if 响应.hovered() {
        ui.painter().rect_filled(矩形, 圆角, 配色.悬停背景);
    }
    let 图标矩形 = egui::Rect::from_center_size(矩形.center(), 图标.size);
    ui.painter().image(
        图标.id,
        图标矩形,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        配色.文字,
    );
    响应
}

/// 一排互斥按钮/一个工具组的胶囊底。
///
/// # 为什么需要它
///
/// 纯图标按钮如果各自独立摆放，读起来是"一排漂浮的符号"，看不出它们是一组。
/// Godot 的视口工具条把变换工具收进一个可见的胶囊里，边界本身就说明了
/// "这四个是一组、只能选一个"。这里复用它，并把外层的工具条底色去掉：
/// 画面上只留下两个胶囊（工具 / 开关），而不是一条横贯视口的工具条。
pub fn 分段容器<R>(
    ui: &mut Ui,
    配色: crate::编辑器配色,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut 返回值 = None;
    // 用面板色 + 一道 1px 边框，而不是叠加色的半透明底：
    // 深色下叠加色与视口底色几乎同值，胶囊会"消失"；
    // 浅色下叠加色是近白，压在纸上同样看不见。分组必须真的看得出来。
    egui::Frame::NONE
        .fill(配色.面板背景)
        .stroke(egui::Stroke::new(1.0, 配色.边框))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(3, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            返回值 = Some(add_contents(ui));
        });
    返回值.expect("闭包一定会被调用一次")
}

/// 输入类控件的外框（深色无描边，浅色一道 1px 细边）。
///
/// # 为什么浅色要单独加
///
/// 深色下输入框底色 `#0D0F11` 压在面板 `#16191C` 上已经足够区分；
/// 浅色下输入框是纯白、面板是暖白，两者的明度差很小，
/// 去掉边框之后数值与标签会连成一片。这里只给浅色补一道边，
/// 深色保持无边框 —— 否则深色界面会退回"一排小方格"的观感。
pub fn 输入框外框<R>(
    ui: &mut Ui,
    配色: crate::编辑器配色,
    明暗: crate::theme::明暗,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut 框 = egui::Frame::NONE
        .fill(配色.输入框背景)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(4, 2));
    if 明暗 == crate::theme::明暗::浅色 {
        框 = 框.stroke(egui::Stroke::new(1.0, 配色.边框));
    }
    框.show(ui, |ui| add_contents(ui)).inner
}

/// 输入框自身的 `Frame`（交给 `TextEdit::frame`）。
///
/// 与 [`输入框外框`] 同一规则：深色无描边、浅色一道 1px 细边，
/// 内边距比默认略大一点，让单行输入的高度与 30px 控件对齐。
pub fn 输入框框架(
    配色: crate::编辑器配色, 明暗: crate::theme::明暗
) -> egui::Frame {
    let mut 框 = egui::Frame::NONE
        .fill(配色.输入框背景)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 4));
    if 明暗 == crate::theme::明暗::浅色 {
        框 = 框.stroke(egui::Stroke::new(1.0, 配色.边框));
    }
    框
}
