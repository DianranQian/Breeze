//! 顶部主开发模式导航（2D / 3D / 脚本 / 运行 / 资源）与模式专用工具栏。
//!
//! # 信息架构（两级）
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────────┐
//! │ 文件 编辑 场景 …                                    菜单栏    │
//! ├───────────────────────────────────────────────────────────────┤
//! │ [2D] [3D] [脚本] [运行] [资源]                    主开发模式  │  ← 本模块
//! ├───────────────────────────────────────────────────────────────┤
//! │ 布局: [2D 默认] [通用] [场景] [资源] [调试] + ⧉ ✎ ✖ ⟲ ⋯      │  ← 布局（工作区）
//! ├───────────────────────────────────────────────────────────────┤
//! │ 模式专用工具 | 通用工具（运行/撤销/保存/实体/面板）           │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! 一级是"我在做什么任务"，二级是"面板怎么摆"。把它们分开的直接好处：
//! 用户可以在 2D 模式里用"调试布局"，也可以在脚本模式里保留自己的布局，
//! 两层互不覆盖。
//!
//! # 模式切换必须"有实感"
//!
//! 只改高亮是不够的。每个模式都实际改变：
//!
//! * **视口类型**：3D 用透视轨道相机，2D 用正交网格视口（脚本/运行/资源
//!   按布局各自保留视口标签）；
//! * **专用工具栏**：本模块 [`绘制模式工具栏`] 按模式给不同工具；
//! * **默认布局**：每个模式一套（见 [`crate::dev_modes::开发模式::默认布局`]）；
//! * **活动编辑内容**：进入脚本模式时会强调脚本面板；运行模式强调运行时观察。
//!
//! 文档、选择、撤销栈、主题、语言在模式之间**共享**，切模式不会丢任何数据。

use egui::{Color32, CornerRadius, RichText, Ui};

use crate::dev_modes::开发模式;
use crate::icons::图标;
use crate::state::{UiAction, 绘制上下文};

/// 模式栏高度（逻辑像素）。
pub const 模式栏高度: f32 = 30.0;

/// 绘制主开发模式导航。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 当前 = 上下文.界面.模式;
    let 图标尺寸 = (上下文.外观.图标尺寸 * 1.05).max(14.0);

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
        ui.add_space(6.0);

        for 模式 in 开发模式::全部() {
            let 激活 = *模式 == 当前;
            let 名称 = 语言.取(模式.名称键());
            let 色 = if 激活 {
                配色.文字
            } else {
                配色.次要文字
            };
            let 文字 = RichText::new(名称)
                .size((上下文.外观.基础字号 + 1.0).max(12.0))
                .color(色);
            let 文字 = if 激活 { 文字.strong() } else { 文字 };

            // 先画图标再画文字：`Button::image_and_text` 会接管悬停/按下状态。
            let 图标项 = 模式.图标();
            let 有纹理 = 上下文.图标.纹理(ui.ctx(), 图标项).is_some();
            let mut 响应 = if 有纹理 {
                let 纹理 = 上下文
                    .图标
                    .纹理(ui.ctx(), 图标项)
                    .expect("上一行已确认纹理存在")
                    .id();
                let 图片 = egui::Image::new(egui::load::SizedTexture::new(
                    纹理,
                    egui::vec2(图标尺寸, 图标尺寸),
                ))
                .tint(if 激活 { 配色.强调 } else { 色 });
                ui.add(
                    egui::Button::image_and_text(图片, 文字)
                        .frame(false)
                        .fill(if 激活 {
                            配色.强调淡
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(if 激活 {
                            egui::Stroke::new(1.0, 配色.强调)
                        } else {
                            egui::Stroke::NONE
                        })
                        .corner_radius(CornerRadius::same(4))
                        .min_size(egui::vec2(0.0, 22.0)),
                )
            } else {
                ui.add(
                    egui::Button::new(文字)
                        .frame(false)
                        .fill(if 激活 {
                            配色.强调淡
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(CornerRadius::same(4)),
                )
            };
            响应 = 响应.on_hover_text(语言.取(模式.说明键()));
            if 响应.clicked() && !激活 {
                上下文.动作.推(UiAction::切换模式(*模式));
            }
        }

        // 右侧：当前模式的身份提示（小字，不抢视觉重心）。
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new(语言.取(当前.说明键()))
                    .small()
                    .color(配色.次要文字),
            );
        });
    });
}

/// 绘制模式专用工具栏。返回是否画了任何控件。
///
/// 只放**真实可用**的操作：每个按钮要么改界面状态、要么排一个动作、
/// 要么切换一个真实存在的显示选项。没有实现的工具不会出现在这里。
pub fn 绘制模式工具栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) -> bool {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 图标尺寸 = 上下文.外观.图标尺寸;
    let 模式 = 上下文.界面.模式;
    #[allow(unused_assignments)]
    let mut 画了 = false;

    match 模式 {
        开发模式::二维 => {
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::最大化,
                    Some(语言.取("fit_window")),
                    图标尺寸,
                    配色.文字,
                    true,
                )
                .on_hover_text(语言.取("fit_window_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::视口适应窗口);
            }
            if 上下文
                .图标
                .图标按钮(ui, 图标::刷新, None, 图标尺寸, 配色.文字, true)
                .on_hover_text(语言.取("reset_view_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::视口重置视图);
            }
            // 显示开关是**界面状态**，直接改，不需要绕动作队列。
            let mut 相机 = 上下文.界面.相机;
            if ui
                .selectable_label(相机.显示网格, 语言.取("grid"))
                .on_hover_text(语言.取("tooltip_toggle_grid"))
                .clicked()
            {
                相机.显示网格 = !相机.显示网格;
            }
            if ui
                .selectable_label(相机.显示坐标轴, 语言.取("axes"))
                .clicked()
            {
                相机.显示坐标轴 = !相机.显示坐标轴;
            }
            if ui
                .selectable_label(相机.显示包围框, 语言.取("bounds"))
                .clicked()
            {
                相机.显示包围框 = !相机.显示包围框;
            }
            上下文.界面.相机 = 相机;
            画了 = true;
        }
        开发模式::三维 => {
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::刷新,
                    Some(语言.取("orbit_reset")),
                    图标尺寸,
                    配色.文字,
                    true,
                )
                .on_hover_text(语言.取("orbit_reset_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::三维重置轨道);
            }
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::最大化,
                    Some(语言.取("fit_window")),
                    图标尺寸,
                    配色.文字,
                    true,
                )
                .clicked()
            {
                上下文.动作.推(UiAction::视口适应窗口);
            }
            if ui
                .add_enabled(
                    !上下文.编辑器.选择.为空(),
                    egui::Button::new(语言.取("focus_selection")).small(),
                )
                .on_hover_text(语言.取("focus_selection_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::聚焦选中实体);
            }
            // 三维相机的实时读数：让用户确认自己在什么视角。
            let 相机 = &上下文.界面.三维相机;
            ui.label(
                RichText::new(format!(
                    "{} {:.0}° {:.0}° {:.0}",
                    语言.取("camera"),
                    相机.偏航.to_degrees(),
                    相机.俯仰.to_degrees(),
                    相机.距离
                ))
                .small()
                .color(配色.次要文字),
            );
            画了 = true;
        }
        开发模式::脚本 => {
            let 有文档 = 上下文.界面.脚本.文档.is_some();
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::保存,
                    Some(语言.取("script_save")),
                    图标尺寸,
                    配色.文字,
                    有文档,
                )
                .on_hover_text(语言.取("script_save_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::保存脚本);
            }
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::放大,
                    Some(语言.取("script_find")),
                    图标尺寸,
                    配色.文字,
                    有文档,
                )
                .clicked()
            {
                上下文.界面.脚本.显示查找 = true;
            }
            match 上下文.界面.脚本.文档.as_ref() {
                Some(文档) => {
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            文档.显示名(语言),
                            if 文档.有未保存修改() {
                                language_unsaved(语言)
                            } else {
                                ""
                            }
                        ))
                        .small()
                        .color(if 文档.有未保存修改() {
                            配色.警告
                        } else {
                            配色.次要文字
                        }),
                    );
                }
                None => {
                    ui.label(
                        RichText::new(语言.取("script_no_file"))
                            .small()
                            .color(配色.次要文字),
                    );
                }
            }
            画了 = true;
        }
        开发模式::运行 => {
            // 项目运行（独立进程）——与下方的编辑器内运行时统计是两套东西。
            let 在跑 = 上下文.界面.项目运行中;
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::运行,
                    Some(语言.取("run")),
                    图标尺寸,
                    配色.文字,
                    !在跑,
                )
                .clicked()
            {
                上下文.动作.推(UiAction::运行项目);
            }
            if 上下文
                .图标
                .图标按钮(ui, 图标::刷新, None, 图标尺寸, 配色.文字, true)
                .clicked()
            {
                上下文.动作.推(UiAction::重新运行项目);
            }
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::停止,
                    Some(语言.取("stop")),
                    图标尺寸,
                    配色.文字,
                    在跑,
                )
                .clicked()
            {
                上下文.动作.推(UiAction::停止项目);
            }
            let 摘要 = 上下文.界面.项目运行摘要.clone();
            if !摘要.is_empty() {
                ui.label(RichText::new(摘要).small().color(if 在跑 {
                    配色.成功
                } else {
                    配色.次要文字
                }));
            }
            ui.separator();

            let 正在运行 = 上下文.正在运行;
            let 已暂停 = 上下文.已暂停;
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::运行,
                    Some(语言.取("run")),
                    图标尺寸,
                    配色.文字,
                    !正在运行,
                )
                .clicked()
            {
                上下文.动作.推(UiAction::运行场景);
            }
            if 上下文
                .图标
                .图标按钮(ui, 图标::暂停, None, 图标尺寸, 配色.文字, 正在运行)
                .clicked()
            {
                上下文.动作.推(UiAction::暂停运行);
            }
            if 上下文
                .图标
                .图标按钮(ui, 图标::停止, None, 图标尺寸, 配色.文字, 正在运行)
                .clicked()
            {
                上下文.动作.推(UiAction::停止运行);
            }
            if 上下文
                .图标
                .图标按钮(ui, 图标::单步, None, 图标尺寸, 配色.文字, 已暂停)
                .clicked()
            {
                上下文.动作.推(UiAction::单步一帧);
            }
            ui.separator();
            for (面板, 文字键) in [
                (crate::docks::Tab::控制台, "console"),
                (crate::docks::Tab::运行时ecs, "runtime_ecs"),
                (crate::docks::Tab::性能, "performance"),
            ] {
                if ui.small_button(语言.取(文字键)).clicked() {
                    上下文.动作.推(UiAction::聚焦面板(面板));
                }
            }
            // 真实运行状态，不是静态标记。
            let 状态文本 = if 已暂停 {
                语言.取("paused")
            } else if 正在运行 {
                语言.取("running")
            } else {
                语言.取("stopped")
            };
            ui.label(RichText::new(状态文本).small().color(if 正在运行 {
                配色.成功
            } else {
                配色.次要文字
            }));
            画了 = true;
        }
        开发模式::资源 => {
            if 上下文
                .图标
                .图标按钮(
                    ui,
                    图标::打开文件夹,
                    Some(语言.取("open_assets_folder")),
                    图标尺寸,
                    配色.文字,
                    上下文.编辑器.项目.is_some(),
                )
                .clicked()
                && let Ok(路径) = scene_doc::path::项目相对路径::解析("assets")
            {
                上下文.动作.推(UiAction::在资源管理器中显示(路径));
            }
            if ui
                .add_enabled(
                    上下文.编辑器.项目.is_some(),
                    egui::Button::new(语言.取("back_to_2d")).small(),
                )
                .on_hover_text(语言.取("back_to_2d_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::切换模式(开发模式::二维));
            }
            ui.label(
                RichText::new(语言.取("asset_mode_note"))
                    .small()
                    .color(配色.次要文字),
            );
            画了 = true;
        }
    }
    画了
}

fn language_unsaved(语言: crate::i18n::语言) -> &'static str {
    语言.取("script_unsaved")
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 模式导航顺序稳定() {
        let ids: Vec<&str> = 开发模式::全部().iter().map(|模式| 模式.id()).collect();
        assert_eq!(ids, vec!["2d", "3d", "script", "run", "assets"]);
    }

    #[test]
    fn 每个模式都有名称与说明键() {
        for 模式 in 开发模式::全部() {
            assert!(!模式.名称键().is_empty());
            assert!(!模式.说明键().is_empty());
            assert_ne!(模式.名称键(), 模式.说明键());
        }
    }
}
