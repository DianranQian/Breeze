//! 主题编辑器：实时预览 + 保存 + 导入导出。
//!
//! # 实时预览是怎么做的
//!
//! 编辑器把改动写进 [`界面状态::主题草稿`]，而 `绘制` 每帧优先使用草稿
//! （见 `editor_app::plugin::绘制一帧`）。因此**改一个颜色，下一帧整界面就变**，
//! 不需要"应用"按钮，也不会污染主题库 —— 取消时丢掉草稿即可，
//! 界面自动回到主题库里的当前主题。
//!
//! # 为什么颜色槽位是表驱动的
//!
//! 32 个语义槽位如果每个都手写三行 `ui.horizontal(...)`，这个文件会变成
//! 四百行重复代码，而且"新增一个颜色"要改四处。这里用一张
//! `(词条键, 取色函数, 设色函数)` 的表，渲染循环统一处理：
//! 加一个颜色只需要在表里加一行 + 在词表里加一条。
//!
//! # 内置主题与用户主题的区别
//!
//! 内置主题可以被编辑，但保存时写进"覆盖层"，出厂定义永远保留；
//! 「恢复内置」只需要丢掉覆盖层。用户主题则直接保存。
//! 内置主题不能删除 —— 删掉之后用户就没有可回退的基准了。

use egui::{Color32, RichText};

use crate::i18n::语言;
use crate::state::{UiAction, 动作队列, 界面状态};
use crate::theme::{内置主题, 编辑器配色};

/// 一个可编辑的颜色槽位。
struct 颜色槽 {
    /// 词条键。
    键: &'static str,
    /// 显示名称键（与 `键` 分开：`color_*` 是显示名，
    /// 将来若要给某个槽位加说明，可以独立演进）。
    取: fn(&编辑器配色) -> Color32,
    /// 设色。
    设: fn(&mut 编辑器配色, Color32),
}

/// 一组颜色槽位。
struct 颜色组 {
    标题键: &'static str,
    槽: Vec<颜色槽>,
}

fn 槽(
    键: &'static str,
    取: fn(&编辑器配色) -> Color32,
    设: fn(&mut 编辑器配色, Color32),
) -> 颜色槽 {
    颜色槽 { 键, 取, 设 }
}

/// 颜色槽位表（按语义分组）。
fn 颜色分组() -> Vec<颜色组> {
    vec![
        颜色组 {
            标题键: "theme_group_background",
            槽: vec![
                槽("color_window_bg", |c| c.窗口背景, |c, v| c.窗口背景 = v),
                槽("color_panel_bg", |c| c.面板背景, |c, v| c.面板背景 = v),
                槽(
                    "color_panel_alt_bg",
                    |c| c.面板次背景,
                    |c, v| c.面板次背景 = v,
                ),
                槽("color_input_bg", |c| c.输入框背景, |c, v| c.输入框背景 = v),
                槽("color_hover_bg", |c| c.悬停背景, |c, v| c.悬停背景 = v),
                槽("color_active_bg", |c| c.激活背景, |c, v| c.激活背景 = v),
                槽("color_overlay_bg", |c| c.叠加背景, |c, v| c.叠加背景 = v),
                槽(
                    "color_overlay_border",
                    |c| c.叠加边框,
                    |c, v| c.叠加边框 = v,
                ),
            ],
        },
        颜色组 {
            标题键: "theme_group_text",
            槽: vec![
                槽("color_text", |c| c.文字, |c, v| c.文字 = v),
                槽(
                    "color_text_secondary",
                    |c| c.次要文字,
                    |c, v| c.次要文字 = v,
                ),
                槽("color_text_disabled", |c| c.禁用文字, |c, v| c.禁用文字 = v),
                槽("color_link", |c| c.链接, |c, v| c.链接 = v),
            ],
        },
        颜色组 {
            标题键: "theme_group_accent",
            槽: vec![
                槽("color_accent", |c| c.强调, |c, v| c.强调 = v),
                槽("color_accent_bright", |c| c.强调亮, |c, v| c.强调亮 = v),
                槽("color_selection", |c| c.选中背景, |c, v| c.选中背景 = v),
            ],
        },
        颜色组 {
            标题键: "theme_group_border",
            槽: vec![
                槽("color_border", |c| c.边框, |c, v| c.边框 = v),
                槽("color_border_strong", |c| c.边框强, |c, v| c.边框强 = v),
                槽("color_separator", |c| c.分割线, |c, v| c.分割线 = v),
            ],
        },
        颜色组 {
            标题键: "theme_group_viewport",
            槽: vec![
                槽("color_viewport_bg", |c| c.视口背景, |c, v| c.视口背景 = v),
                槽("color_grid_major", |c| c.网格主, |c, v| c.网格主 = v),
                槽("color_grid_minor", |c| c.网格次, |c, v| c.网格次 = v),
                槽("color_axis_x", |c| c.轴_x, |c, v| c.轴_x = v),
                槽("color_axis_y", |c| c.轴_y, |c, v| c.轴_y = v),
                槽("color_selection_box", |c| c.选择框, |c, v| c.选择框 = v),
            ],
        },
        颜色组 {
            标题键: "theme_group_status",
            槽: vec![
                槽("color_info", |c| c.信息, |c, v| c.信息 = v),
                槽("color_success", |c| c.成功, |c, v| c.成功 = v),
                槽("color_warning", |c| c.警告, |c, v| c.警告 = v),
                槽("color_error", |c| c.错误, |c, v| c.错误 = v),
            ],
        },
        颜色组 {
            标题键: "theme_group_controls",
            槽: vec![
                槽("color_button_bg", |c| c.按钮背景, |c, v| c.按钮背景 = v),
                槽("color_button_hover", |c| c.按钮悬停, |c, v| c.按钮悬停 = v),
                槽("color_button_active", |c| c.按钮按下, |c, v| c.按钮按下 = v),
            ],
        },
        颜色组 {
            标题键: "theme_group_dock",
            槽: vec![
                槽(
                    "color_tab_bar_bg",
                    |c| c.标签栏背景,
                    |c, v| c.标签栏背景 = v,
                ),
                槽(
                    "color_tab_active_bg",
                    |c| c.标签激活背景,
                    |c, v| c.标签激活背景 = v,
                ),
                槽("color_tab_text", |c| c.标签文字, |c, v| c.标签文字 = v),
            ],
        },
    ]
}

/// 绘制主题编辑器。
///
/// 只在 `界面.显示主题编辑器` 为真时显示。草稿由动作执行器负责创建/销毁，
/// 本函数只读写草稿本身。
pub fn 绘制(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    语言: 语言,
    图标库: &mut crate::icons::图标库,
    图标尺寸: f32,
    动作: &mut 动作队列,
) {
    if !界面.显示主题编辑器 {
        return;
    }
    // 编辑器打开但还没有草稿（例如刚启动时配置里写着"显示"）：补一份。
    if 界面.主题草稿.is_none() {
        界面.主题草稿 = Some(界面.主题库.当前().clone());
    }

    let mut 打开 = true;
    let mut 待推: Vec<UiAction> = Vec::new();

    // 先把只读数据取出来，避免与草稿的可变借用交叉。
    let 当前id = 界面.主题库.当前id().to_string();
    let 用户数量 = 界面.主题库.用户数量();
    let 主题列表: Vec<(String, String, bool)> = 界面
        .主题库
        .全部()
        .iter()
        .map(|主题| (主题.id.clone(), 主题.显示名(语言), 主题.内置))
        .collect();

    let 草稿 = 界面.主题草稿.as_mut().expect("上面已保证草稿存在");

    let 窗口响应 = egui::Window::new(语言.取("theme_editor"))
        .id(egui::Id::new("bevy_studio_主题编辑器"))
        .open(&mut 打开)
        .resizable(true)
        .collapsible(false)
        .default_size(egui::vec2(620.0, 700.0))
        .min_width(520.0)
        // 离开左上角：贴边浮窗会压住菜单条与模式切换，用户第一眼会把它
        // 当成"界面坏了"而不是"一个新窗口"。默认位置落在工作区中间偏左。
        .default_pos(egui::pos2(320.0, 84.0))
        .show(egui上下文, |ui| {
            // ---- 顶部：当前主题 + 切换 ----
            //
            // 主题列表用分段语法而不是普通按钮：它们是**互斥的一选一**。
            // 每个内置主题带自己的身份图标（月亮 / 太阳 / 对比度），
            // 用户不必先读文字才能认出深色与浅色。
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(语言.取("theme_list"))
                        .size(13.0 - 0.5)
                        .strong()
                        .color(配色.文字),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(if 当前id.starts_with("user-") {
                            format!("{} {用户数量}", 语言.取("theme_group_custom"))
                        } else {
                            String::new()
                        })
                        .size(11.0)
                        .color(配色.禁用文字),
                    );
                });
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (id, 名称, 内置) in &主题列表 {
                    let 选中 = *id == 当前id;
                    // 内置主题各有一个"身份图标"：月亮=深色、太阳=浅色、
                    // 对比度=高对比度。用户在列表里不必读文字也能区分。
                    let 图标项 = match 内置主题::从id(id) {
                        Some(内置主题::深色) => crate::icons::图标::月亮,
                        Some(内置主题::浅色) => crate::icons::图标::太阳,
                        Some(内置主题::高对比度) => crate::icons::图标::对比度,
                        None => crate::icons::图标::主题,
                    };
                    let 纹理 = 图标库.纹理(egui上下文, 图标项).map(|纹理| {
                        egui::load::SizedTexture::new(
                            纹理.id(),
                            egui::vec2(图标尺寸 * 0.8, 图标尺寸 * 0.8),
                        )
                    });
                    let 标签 = if *内置 {
                        format!("{}·{}", 名称, 语言.取("theme_builtin_tag"))
                    } else {
                        名称.clone()
                    };
                    if crate::visual_spec::分段标签(
                        ui,
                        配色,
                        13.0,
                        &标签,
                        纹理.as_ref(),
                        选中,
                        true,
                    )
                    .clicked()
                        && !选中
                    {
                        待推.push(UiAction::设置主题(id.clone()));
                    }
                }
            });
            ui.add_space(6.0);

            // ---- 草稿基本信息 ----
            egui::Grid::new("主题基本信息")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new(语言.取("theme_name")).color(配色.次要文字));
                    ui.horizontal(|ui| {
                        let mut 名称 = 草稿.名称.clone().unwrap_or_else(|| 草稿.显示名(语言));
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut 名称)
                                    .desired_width(200.0)
                                    .frame(crate::visual_spec::输入框框架(配色, 草稿.明暗)),
                            )
                            .changed()
                        {
                            草稿.名称 = Some(名称);
                        }
                        // 明暗属性影响 egui 的一些默认绘制，必须可调。
                        let 明暗标签 = match 草稿.明暗 {
                            crate::theme::明暗::深色 => 语言.取("theme_dark"),
                            crate::theme::明暗::浅色 => 语言.取("theme_light"),
                        };
                        ui.label(RichText::new(明暗标签).size(11.0).color(配色.禁用文字));
                    });
                    ui.end_row();
                });

            ui.add_space(4.0);
            // 高度自适应窗口：固定 360 会在默认窗口高度下把
            // "控件/停靠"两组颜色裁掉一半（实测过）。留 80px 给下面的按钮行。
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 80.0).max(120.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for 组 in 颜色分组() {
                        ui.add_space(8.0);
                        // 分组标题：小字 + 一条贯穿整行的细线。分组靠线断开，
                        // 不靠给每组套一个背景块。
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(语言.取(组.标题键))
                                    .size(11.0)
                                    .strong()
                                    .color(配色.次要文字),
                            );
                            let 剩余 = ui.available_width();
                            let (线, _) =
                                ui.allocate_exact_size(egui::vec2(剩余, 1.0), egui::Sense::hover());
                            ui.painter().rect_filled(线, 0.0, 配色.边框);
                        });
                        // 两列网格：每列 = 名称 + 色块。名称列宽固定，
                        // 色块因此垂直对齐；改造前是流式换行，色块参差不齐。
                        egui::Grid::new(("主题颜色", 组.标题键))
                            .num_columns(4)
                            .spacing([8.0, 4.0])
                            .min_col_width(78.0)
                            .show(ui, |ui| {
                                for (下标, 槽位) in 组.槽.iter().enumerate() {
                                    let 值 = (槽位.取)(&草稿.配色);
                                    ui.label(
                                        RichText::new(语言.取(槽位.键))
                                            .size(11.0)
                                            .color(配色.次要文字),
                                    );
                                    let mut 新值 = 值;
                                    if ui
                                        .color_edit_button_srgba(&mut 新值)
                                        .on_hover_text(format!(
                                            "#{:02X}{:02X}{:02X}",
                                            值.r(),
                                            值.g(),
                                            值.b()
                                        ))
                                        .changed()
                                    {
                                        (槽位.设)(&mut 草稿.配色, 新值);
                                    }
                                    // 每行两项，第二项之后换行。
                                    if 下标 % 2 == 1 {
                                        ui.end_row();
                                    }
                                }
                                if 组.槽.len() % 2 == 1 {
                                    ui.label("");
                                    ui.label("");
                                    ui.end_row();
                                }
                            });
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(
                        RichText::new(语言.取("theme_group_appearance"))
                            .strong()
                            .color(配色.文字),
                    );
                    // 外观参数直接改草稿；`外观::收敛` 在应用时会夹住范围，
                    // 因此这里的滑块范围与之一致，避免"拖到极限值才发现被夹"。
                    ui.add(
                        egui::Slider::new(&mut 草稿.外观.缩放, 0.7..=2.0)
                            .text(语言.取("theme_ui_scale")),
                    );
                    ui.add(
                        egui::Slider::new(&mut 草稿.外观.基础字号, 9.0..=22.0)
                            .text(语言.取("theme_font_size")),
                    );
                    ui.add(
                        egui::Slider::new(&mut 草稿.外观.圆角, 0.0..=12.0)
                            .text(语言.取("theme_corner_radius")),
                    );
                    ui.add(
                        egui::Slider::new(&mut 草稿.外观.间距, 2.0..=16.0)
                            .text(语言.取("theme_spacing")),
                    );
                    ui.add(
                        egui::Slider::new(&mut 草稿.外观.图标尺寸, 12.0..=32.0)
                            .text(语言.取("theme_icon_size")),
                    );
                });

            ui.add_space(6.0);
            crate::visual_spec::细线(ui, 配色);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(语言.取("theme_save"))
                                .color(配色.强调亮)
                                .strong(),
                        )
                        .fill(配色.强调淡)
                        .stroke(egui::Stroke::new(1.0, 配色.强调)),
                    )
                    .on_hover_text(语言.取("theme_save_hint"))
                    .clicked()
                {
                    待推.push(UiAction::保存主题草稿);
                }
                if ui
                    .button(语言.取("theme_save_as"))
                    .on_hover_text(语言.取("theme_save_as_hint"))
                    .clicked()
                {
                    待推.push(UiAction::另存为主题 {
                        名称: 语言.填充("theme_new_name", &[&(用户数量 + 1)]),
                    });
                }
                // 危险操作与普通操作分开：删除主题是唯一不可撤销的动作，
                // 它用错误色描边、并与导入/导出之间隔一道分组线。
                // 改造前七个按钮等宽等权排在一条线上，"删除主题"和"取消"
                // 长得一样，误点代价却是丢掉一整套配色。
                let 当前是内置 = 内置主题::从id(&当前id).is_some();
                if ui
                    .add_enabled(
                        !当前是内置,
                        egui::Button::new(RichText::new(语言.取("theme_delete")).color(
                            if 当前是内置 {
                                配色.禁用文字
                            } else {
                                配色.错误
                            },
                        ))
                        .stroke(egui::Stroke::new(
                            1.0,
                            if 当前是内置 {
                                配色.边框
                            } else {
                                配色.错误
                            },
                        )),
                    )
                    .on_hover_text(if 当前是内置 {
                        语言.取("theme_builtin_not_deletable")
                    } else {
                        语言.取("theme_delete")
                    })
                    .clicked()
                {
                    待推.push(UiAction::删除主题(当前id.clone()));
                }
                if ui
                    .add_enabled(
                        当前是内置,
                        egui::Button::new(语言.取("theme_reset_builtin")),
                    )
                    .clicked()
                {
                    待推.push(UiAction::重置内置主题);
                }
                crate::visual_spec::工具栏分组(ui, 配色);
                if ui.button(语言.取("theme_import")).clicked() {
                    待推.push(UiAction::导入主题);
                }
                if ui.button(语言.取("theme_export")).clicked() {
                    待推.push(UiAction::导出主题);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(语言.取("cancel")).clicked() {
                        待推.push(UiAction::关闭主题编辑器);
                    }
                });
            });
            ui.label(
                RichText::new(语言.取("theme_live_preview_hint"))
                    .small()
                    .color(配色.次要文字),
            );
        });

    // 标题栏 × 与窗口内取消按钮等价。
    if !打开 {
        待推.push(UiAction::关闭主题编辑器);
    }
    // 上报浮层区域：验收工具靠它区分"这块按钮被窗口盖住了"与"按钮没画出来"。
    crate::记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_theme_editor",
        窗口响应.map(|响应| 响应.response.rect),
    );
    for 动作项 in 待推 {
        动作.推(动作项);
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 颜色槽位表没有重复键() {
        let 分组 = 颜色分组();
        let mut 键: Vec<&str> = 分组
            .iter()
            .flat_map(|组| 组.槽.iter().map(|槽| 槽.键))
            .collect();
        let 原数量 = 键.len();
        键.sort_unstable();
        键.dedup();
        assert_eq!(键.len(), 原数量, "颜色槽位的词条键必须唯一");
    }

    #[test]
    fn 颜色槽位表覆盖全部必需类别() {
        let 分组 = 颜色分组();
        let 标题: Vec<&str> = 分组.iter().map(|组| 组.标题键).collect();
        for 必需 in [
            "theme_group_background",
            "theme_group_text",
            "theme_group_accent",
            "theme_group_border",
            "theme_group_viewport",
            "theme_group_status",
            "theme_group_controls",
            "theme_group_dock",
        ] {
            assert!(标题.contains(&必需), "缺少颜色分组 `{必需}`");
        }
        // 需求点名的六类颜色都要能编辑。
        let 键: Vec<&str> = 分组
            .iter()
            .flat_map(|组| 组.槽.iter().map(|槽| 槽.键))
            .collect();
        for 必需 in [
            "color_panel_bg",
            "color_text",
            "color_accent",
            "color_selection",
            "color_border",
            "color_grid_major",
            "color_warning",
            "color_error",
        ] {
            assert!(键.contains(&必需), "缺少颜色槽位 `{必需}`");
        }
    }

    #[test]
    fn 每个槽位都能真正改写配色() {
        let mut 配色 = 编辑器配色::default();
        for 组 in 颜色分组() {
            for 槽位 in &组.槽 {
                let 原值 = (槽位.取)(&配色);
                let 新值 = Color32::from_rgb(原值.r().wrapping_add(1), 原值.g(), 原值.b());
                (槽位.设)(&mut 配色, 新值);
                assert_eq!((槽位.取)(&配色), 新值, "槽位 `{}` 的设值没有生效", 槽位.键);
            }
        }
    }
}
