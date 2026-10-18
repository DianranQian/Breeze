//! 构建面板：配置 → 构建 → 诊断 → 跳转源码。
//!
//! # 面板画的是一条真实链路，不是一排按钮
//!
//! ```text
//!  配置（存在 project.ron）        ← 用户改，动作写回项目清单
//!         │
//!  开始构建 → 子进程 cargo build   ← editor_app::build
//!         │
//!  结构化诊断（错误/警告/提示）     ← editor_core::build 解析
//!         │
//!  点诊断 → 打开脚本面板并滚到该行  ← UiAction::打开构建诊断
//! ```
//!
//! 因此这个面板里没有"看起来能点但其实没接线"的按钮：
//! 每个控件都对应一条已经实现的动作，构建结果来自真实的子进程。

use editor_core::build::{构建状态, 构建配置, 构建配置类型, 诊断级别};
use editor_core::package::打包配置;
use egui::Ui;

use crate::state::{UiAction, 绘制上下文};

/// 绘制构建面板。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 快照 = 上下文.构建;
    let 报告 = &快照.报告;
    let 在运行 = 报告.状态.在运行();

    // ---- 配置 ----
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(语言.取("build_profile"));

        let mut 配置 = 快照.配置.clone();
        let 旧 = 配置.clone();

        let 当前名 = match 配置.配置类型 {
            构建配置类型::Debug => 语言.取("build_profile_debug"),
            构建配置类型::Release => 语言.取("build_profile_release"),
        };
        egui::ComboBox::from_id_salt("build_profile")
            .selected_text(当前名)
            .width(120.0)
            .show_ui(ui, |ui| {
                for 类型 in 构建配置类型::全部 {
                    let 名称 = match 类型 {
                        构建配置类型::Debug => 语言.取("build_profile_debug"),
                        构建配置类型::Release => 语言.取("build_profile_release"),
                    };
                    let 选中 = 配置.配置类型 == 类型;
                    if ui.selectable_label(选中, 名称).clicked() {
                        配置.配置类型 = 类型;
                    }
                }
            });

        ui.label(语言.取("build_target"));
        let mut 目标 = 配置.目标平台.clone().unwrap_or_default();
        let 响应 = ui.add(
            egui::TextEdit::singleline(&mut 目标)
                .hint_text(语言.取("build_target_hint"))
                .desired_width(150.0),
        );
        if 响应.changed() {
            配置.目标平台 = Some(目标).filter(|值| !值.trim().is_empty());
        }

        ui.label(语言.取("build_package"));
        let mut 包名 = 配置.包名.clone().unwrap_or_default();
        let 响应 = ui.add(
            egui::TextEdit::singleline(&mut 包名)
                .hint_text(语言.取("build_package_hint"))
                .desired_width(110.0),
        );
        if 响应.changed() {
            配置.包名 = Some(包名).filter(|值| !值.trim().is_empty());
        }

        ui.label(语言.取("build_features"));
        let mut 特征 = 配置.特征.join(",");
        let 响应 = ui.add(
            egui::TextEdit::singleline(&mut 特征)
                .hint_text(语言.取("build_features_hint"))
                .desired_width(140.0),
        );
        if 响应.changed() {
            配置.特征 = 特征
                .split(',')
                .map(str::trim)
                .filter(|项| !项.is_empty())
                .map(str::to_string)
                .collect();
        }

        if 配置 != 旧 {
            上下文.动作.推(UiAction::设置构建配置(配置));
        }
    });

    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let 有项目 = 上下文.编辑器.项目.is_some();
        ui.add_enabled_ui(有项目 && !在运行, |ui| {
            if ui.button(语言.取("build_start")).clicked() {
                上下文.动作.推(UiAction::开始构建);
            }
        });
        ui.add_enabled_ui(在运行, |ui| {
            if ui.button(语言.取("build_stop")).clicked() {
                上下文.动作.推(UiAction::停止构建);
            }
        });

        // 状态徽标
        let (状态色, 状态文本) = match 报告.状态 {
            构建状态::空闲 => (配色.次要文字, 语言.取("build_state_idle").to_string()),
            构建状态::运行中 => (配色.警告, 语言.取("build_state_running").to_string()),
            构建状态::成功 => (配色.成功, 语言.取("build_state_succeeded").to_string()),
            构建状态::失败 => (配色.错误, 语言.取("build_state_failed").to_string()),
            构建状态::已取消 => {
                (配色.次要文字, 语言.取("build_state_cancelled").to_string())
            }
        };
        ui.label(egui::RichText::new(状态文本).strong().color(状态色));

        if 报告.状态.已结束() {
            ui.label(
                egui::RichText::new(语言.填充(
                    "build_duration",
                    &[&format!("{:.1}", 报告.耗时毫秒 as f32 / 1000.0)],
                ))
                .small()
                .color(配色.次要文字),
            );
            if !报告.无问题() {
                ui.label(
                    egui::RichText::new(
                        语言.填充("build_counts", &[&报告.错误数(), &报告.警告数()]),
                    )
                    .small()
                    .color(if 报告.错误数() > 0 {
                        配色.错误
                    } else {
                        配色.警告
                    }),
                );
            }
        }

        if let Some(产物) = 报告.主产物() {
            // 产物路径可复制：用户常要拿它去手动跑一遍。
            ui.label(
                egui::RichText::new(语言.填充("build_artifact", &[&产物.display().to_string()]))
                    .small()
                    .color(配色.次要文字),
            );
        }
    });

    if !报告.命令行.is_empty() {
        ui.label(
            egui::RichText::new(format!("$ {}", 报告.命令行))
                .small()
                .monospace()
                .color(配色.次要文字),
        );
    }

    ui.separator();

    // ---- 打包 ----
    //
    // 放在构建正下方：用户的动作顺序是"先编过，再打包"，
    // 两块信息在同一屏里才不用来回切面板。
    绘制打包区(ui, 上下文);

    ui.separator();

    // ---- 诊断 ----
    if 上下文.编辑器.项目.is_none() {
        ui.label(
            egui::RichText::new(语言.取("build_no_project_hint"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }

    if 报告.诊断.is_empty() {
        // 失败且无诊断时，摘要就是唯一线索（例如 cargo 根本没跑起来）。
        if let Some(摘要) = 报告.摘要文本() {
            egui::ScrollArea::vertical()
                .id_salt("build_summary")
                .max_height(180.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(语言.取("build_output_summary"))
                            .strong()
                            .color(配色.错误),
                    );
                    ui.label(
                        egui::RichText::new(摘要)
                            .monospace()
                            .small()
                            .color(配色.文字),
                    );
                });
        } else {
            ui.label(
                egui::RichText::new(语言.取("build_empty_hint"))
                    .small()
                    .color(配色.次要文字),
            );
        }
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("build_diagnostics")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (序号, 诊断) in 报告.诊断.iter().enumerate() {
                let (级别文本, 级别色) = match 诊断.级别 {
                    诊断级别::错误 => (语言.取("build_level_error"), 配色.错误),
                    诊断级别::警告 => (语言.取("build_level_warning"), 配色.警告),
                    诊断级别::提示 => (语言.取("build_level_note"), 配色.次要文字),
                    诊断级别::备注 => (语言.取("build_level_help"), 配色.成功),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new(级别文本).small().color(级别色));
                    if let Some(位置) = 诊断.位置.as_ref().and_then(|位置| 位置.短标签())
                    {
                        if 诊断.可跳转() {
                            let 响应 = ui.add(
                                egui::Button::new(
                                    egui::RichText::new(位置)
                                        .small()
                                        .color(配色.成功)
                                        .underline(),
                                )
                                .frame(false),
                            );
                            if 响应.clicked() {
                                上下文.动作.推(UiAction::打开构建诊断 {
                                    文件: 诊断.位置.as_ref().and_then(|位置| 位置.文件.clone()),
                                    行: 诊断.位置.as_ref().map(|位置| 位置.行).unwrap_or(0),
                                });
                            }
                            if 响应.on_hover_text(语言.取("build_jump_hint")).clicked() {
                                // 已经处理，这里只是让提示与点击共用一次命中。
                            }
                        } else {
                            ui.label(egui::RichText::new(位置).small().color(配色.次要文字));
                        }
                    }
                    ui.label(egui::RichText::new(诊断.单行文本()).small().color(
                        if 诊断.级别 == 诊断级别::错误 {
                            配色.文字
                        } else {
                            配色.次要文字
                        },
                    ));
                });
                // 诊断之间用极淡的分隔：几十条警告时仍然能一行一行地读。
                if 序号 + 1 < 报告.诊断.len() {
                    ui.separator();
                }
            }
        });
}

/// 便捷函数：把构建配置从项目清单里取出（没有时用默认值）。
pub fn 取构建配置(编辑器: &editor_core::state::编辑器状态) -> 构建配置 {
    编辑器
        .项目
        .as_ref()
        .and_then(|项目| 项目.清单().构建.clone())
        .unwrap_or_default()
}

/// 便捷函数：把打包配置从项目清单里取出（没有时用默认值）。
pub fn 取打包配置(编辑器: &editor_core::state::编辑器状态) -> 打包配置 {
    编辑器
        .项目
        .as_ref()
        .and_then(|项目| 项目.清单().打包.clone())
        .unwrap_or_default()
}

/// 打包区：输出目录配置 + 打包按钮 + 结果。
fn 绘制打包区(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let mut 配置 = 取打包配置(上下文.编辑器);
    let 旧配置 = 配置.clone();
    let 构建成功 = 上下文.构建.报告.状态 == editor_core::build::构建状态::成功;
    let 有产物 = 上下文.构建.报告.主产物().is_some();

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(语言.取("package_section"));
        let mut 目录 = 配置.输出目录.clone();
        if ui
            .add(
                egui::TextEdit::singleline(&mut 目录)
                    .hint_text(editor_core::package::默认输出目录)
                    .desired_width(120.0),
            )
            .changed()
        {
            配置.输出目录 = 目录;
        }
        ui.checkbox(&mut 配置.包含全部场景, 语言.取("package_all_scenes"))
            .on_hover_text(语言.取("package_all_scenes_hint"));
        if 配置 != 旧配置 {
            上下文.动作.推(UiAction::设置打包配置(配置.clone()));
        }

        // 打包要求"构建成功且有产物"：否则会打出一个没有游戏可执行文件的包。
        let 可打包 = 上下文.编辑器.项目.is_some() && 构建成功 && 有产物;
        if ui
            .add_enabled(可打包, egui::Button::new(语言.取("package_button")))
            .on_hover_text(if 可打包 {
                语言.取("package_button_hint")
            } else {
                语言.取("package_needs_build")
            })
            .clicked()
        {
            上下文.动作.推(UiAction::打包项目);
        }
        let 有结果 = 上下文.界面.打包结果.is_some();
        if ui
            .add_enabled(有结果, egui::Button::new(语言.取("package_open_dir")))
            .clicked()
        {
            上下文.动作.推(UiAction::打开打包目录);
        }
    });

    if let Some(结果) = 上下文.界面.打包结果.as_ref() {
        let 颜色 = if 结果.成功 {
            配色.成功
        } else {
            配色.错误
        };
        let 文本 = if 结果.成功 {
            语言.填充(
                "package_done",
                &[
                    &结果.文件数,
                    &editor_core::package::人类可读字节(结果.总字节),
                    &结果.输出目录,
                ],
            )
        } else {
            语言.填充("package_failed", &[&结果.失败项.len()])
        };
        ui.label(egui::RichText::new(文本).small().color(颜色));
        for (文件, 原因) in &结果.失败项 {
            ui.label(
                egui::RichText::new(format!("{文件}：{原因}"))
                    .small()
                    .color(配色.错误),
            );
        }
        if !结果.缺失依赖.is_empty() {
            ui.label(
                egui::RichText::new(语言.填充("package_missing_deps", &[&结果.缺失依赖.len()]))
                    .small()
                    .color(配色.警告),
            );
            for 缺失 in 结果.缺失依赖.iter().take(6) {
                ui.label(
                    egui::RichText::new(缺失)
                        .small()
                        .monospace()
                        .color(配色.次要文字),
                );
            }
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 没有项目时构建配置是默认值() {
        let 编辑器 = editor_core::state::编辑器状态::新建();
        let 配置 = 取构建配置(&编辑器);
        assert_eq!(配置, 构建配置::default());
    }
}
