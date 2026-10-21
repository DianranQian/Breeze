//! 多选对齐/分布工具条。
//!
//! # 为什么是浮动工具条而不是菜单项
//!
//! 对齐是**连续动作**：用户框选一排房子后，往往会连续试"左对齐 → 垂直分布 →
//! 水平居中"。放进菜单意味着每次都要展开两层菜单、移回视口看结果，
//! 节奏完全断掉。浮动条贴着视口显示，点一下立刻看到结果，再点另一个。
//!
//! # 为什么只在选中 ≥2 时出现
//!
//! 单个物体对齐到自己没有意义。显示一个永远没有效果的按钮，
//! 比不显示更糟 —— 用户会怀疑自己点错了。
//!
//! # 坐标系
//!
//! 用 2D 视口的**世界坐标**框（`viewport_panel::计算实体框`）。
//! 与渲染/选择共用同一套尺寸推断，所以"看起来对齐了"就是真的对齐了。

use crate::state::{UiAction, 绘制上下文};
use crate::theme::编辑器配色;

/// 工具条高度。
pub const 工具条高度: f32 = 26.0;

/// 绘制对齐工具条（浮在视口右上角）。
///
/// # 为什么用 `egui::Area` 而不是在视口面板里插一行
///
/// 对齐条是**浮层**：它不参与布局，也不该挤占视口。用 Area 固定到
/// 视口矩形右上角，既不影响视口拿到的可用区域，也不会随面板内边距变化
/// 而偏移。调用点在「绘制浮层」而不是视口绘制函数里：
/// 这样视口面板不需要认识对齐功能，避免把 UI 职责散到每个面板。
///
/// 返回 `None` 表示不必显示；点击产生的意图直接进动作队列。
pub fn 绘制(
    egui上下文: &egui::Context,
    上下文: &mut 绘制上下文<'_>,
    视口矩形: egui::Rect,
    配色: 编辑器配色,
    语言: crate::i18n::语言,
) -> Option<UiAction> {
    // 3D 的对齐语义尚未实现（三维 AABB 需要真实网格尺寸），
    // 因此只在 2D 出现：宁可不显示，也不给一个位置错误的按钮。
    if crate::viewport3d::活动(上下文) {
        return None;
    }
    if 上下文.编辑器.选择.数量() < 2 {
        return None;
    }

    let 位置 = egui::pos2(视口矩形.max.x - 268.0 - 8.0, 视口矩形.min.y + 40.0);
    let mut 动作 = None;
    egui::Area::new(egui::Id::new("arrange_bar"))
        .order(egui::Order::Foreground)
        .fixed_pos(位置)
        .show(egui上下文, |ui| {
            egui::Frame::NONE
                .fill(配色.叠加背景)
                .stroke(egui::Stroke::new(1.0, 配色.叠加边框))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(4, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.label(
                            egui::RichText::new(语言.取("arrange_title"))
                                .small()
                                .color(配色.次要文字),
                        );
                        for 边 in editor_core::arrange::对齐边::全部 {
                            let 文本 = match 边 {
                                editor_core::arrange::对齐边::左 => "⇤",
                                editor_core::arrange::对齐边::水平居中 => "↔",
                                editor_core::arrange::对齐边::右 => "⇥",
                                editor_core::arrange::对齐边::上 => "⤒",
                                editor_core::arrange::对齐边::垂直居中 => "↕",
                                editor_core::arrange::对齐边::下 => "⤓",
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(文本).size(13.0),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(语言.取(边.词条键()))
                                .clicked()
                            {
                                动作 = Some(UiAction::对齐选中 { 边 });
                            }
                        }
                        // 分隔线只画不点：一个点了没反应的按钮比没有按钮更让人困惑。
                        let (分隔矩形, _) = ui.allocate_exact_size(
                            egui::vec2(7.0, 工具条高度),
                            egui::Sense::hover(),
                        );
                        ui.painter().line_segment(
                            [
                                egui::pos2(分隔矩形.center().x, 分隔矩形.top() + 6.0),
                                egui::pos2(分隔矩形.center().x, 分隔矩形.bottom() - 6.0),
                            ],
                            egui::Stroke::new(1.0, 配色.叠加边框),
                        );
                        for 轴 in [
                            editor_core::arrange::分布轴::水平,
                            editor_core::arrange::分布轴::垂直,
                        ] {
                            let 文本 = match 轴 {
                                editor_core::arrange::分布轴::水平 => "⋯",
                                editor_core::arrange::分布轴::垂直 => "⋮",
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(文本).size(13.0),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(语言.取(轴.词条键()))
                                .clicked()
                            {
                                动作 = Some(UiAction::分布选中 { 轴 });
                            }
                        }
                    });
                });
        });
    动作
}
