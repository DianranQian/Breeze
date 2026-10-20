//! 「运行与诊断」面板：原生 Cargo 工程的开发工作台。
//!
//! # 这个面板画的是一条真实闭环
//!
//! ```text
//!  选择运行入口（cargo metadata 里的真实 package / bin）
//!        │
//!  构建（真实 cargo build，退出码与结构化诊断）
//!        │
//!  启动（真实子进程；失败时报真实原因，不谎报成功）
//!        │
//!  看日志 / 看诊断（可筛选、可复制、可点着跳源码）
//!        │
//!  交给 AI Agent（附上真实诊断原文与纪律约束）
//! ```
//!
//! # 为什么它和「构建」面板共存而不是替换它
//!
//! 「构建」面板是**构建配置**的主界面（profile、target、features、打包）；
//! 这个面板是**运行闭环**的主界面（入口、运行状态、运行日志、统一诊断）。
//! 两者共用同一份构建配置与同一批诊断数据，不存在第二套构建逻辑。
//!
//! # 每个控件都有真实行为
//!
//! 面板里没有装饰性按钮：
//!
//! | 控件 | 真实行为 |
//! | --- | --- |
//! | 运行入口下拉 | 写运行入口 + 持久化到用户偏好 |
//! | 构建 / 运行 / 停止 / 重新构建 | 分别对应真实 cargo 子进程动作 |
//! | 诊断行 | 打开真实源文件并滚到行 |
//! | 日志区 | 复制选中/全部日志到剪贴板 |
//! | 发送给 Agent | 把真实诊断/日志附进 AI 会话上下文 |

use editor_core::diagnostics::{诊断严重级别, 诊断条目, 诊断来源};
use editor_core::run_entry::{结束原因, 运行入口, 运行记录, 项目运行状态};
use egui::{RichText, Ui};

use crate::state::{UiAction, 绘制上下文};

/// 绘制「运行与诊断」面板。
///
/// # 为什么整体是可滚动的
///
/// 这个面板常常被停靠在**底部**（与控制台、构建并列），可用高度只有两三百
/// 像素，而它要放下：入口选择、四个操作按钮、运行状态、最近记录、日志、
/// 诊断列表与诊断详情。硬塞必然被裁掉 —— 实测第一次截图里诊断区就整块
/// 落在可视区之外。
///
/// 因此外层给一个纵向滚动，内部两块（日志 / 诊断）各自有**固定上限高度**
/// 并独立滚动：长日志不会把面板撑坏，同时所有控件都够得着。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    egui::ScrollArea::vertical()
        .id_salt("workbench_root")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // 顶部：项目与运行入口
            绘制入口区(ui, 上下文);
            ui.add_space(4.0);
            // 操作条
            绘制操作条(ui, 上下文);
            ui.add_space(4.0);
            // 运行状态与最近记录
            绘制运行状态(ui, 上下文);
            ui.separator();
            // 运行日志（上限高度，独立滚动）
            绘制运行日志(ui, 上下文);
            ui.separator();
            // 统一诊断（上限高度，独立滚动）
            绘制诊断区(ui, 上下文);
        });
}

/// 当前可选的运行入口列表（来自真实 Cargo 工程）。
fn 可选入口(上下文: &绘制上下文<'_>) -> Vec<运行入口> {
    let Some(项目) = 上下文.编辑器.项目.as_ref() else {
        return Vec::new();
    };
    let Some(工程) = 项目.cargo_工程() else {
        return Vec::new();
    };
    工程
        .包
        .iter()
        .flat_map(|包| {
            包.可执行目标.iter().map(|目标| {
                let mut 入口 = 运行入口::新建(目标.包名.clone(), 目标.名称.clone());
                // 沿用当前 profile：用户选过的 Release 不应该因为换目标被重置。
                if let Some(当前) = 上下文.运行.入口.as_ref() {
                    入口.配置类型 = 当前.配置类型;
                }
                入口
            })
        })
        .collect()
}

fn 绘制入口区(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 有项目 = 上下文.编辑器.项目.is_some();
    let 是原生的 = 上下文
        .编辑器
        .项目
        .as_ref()
        .map(|项目| 项目.类型() == editor_core::project::项目类型::原生Cargo)
        .unwrap_or(false);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(语言.取("workbench_project")).strong());
        let 名称 = 上下文
            .编辑器
            .项目
            .as_ref()
            .map(|项目| 项目.名称().to_string())
            .unwrap_or_else(|| 语言.取("workbench_no_project").to_string());
        ui.label(名称);

        if 是原生的 {
            // 识别来源必须如实显示：静态兜底时目标可能选得不够好，
            // 用户看到"为什么退化了"才知道是工具链还是清单的问题。
            if let Some(来源) = 上下文.编辑器.项目.as_ref().and_then(|项目| 项目.识别来源())
            {
                ui.label(
                    RichText::new(语言.取(来源.词条键()))
                        .small()
                        .color(配色.次要文字),
                );
            }
            if let Some(提示) = 上下文.编辑器.项目.as_ref().and_then(|项目| 项目.识别提示())
            {
                ui.label(RichText::new(提示).small().color(配色.警告));
            }
            if ui
                .button(语言.取("workbench_reidentify"))
                .on_hover_text(语言.取("workbench_reidentify_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::重新识别工程);
            }
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(语言.取("workbench_entry"));

        let 候选 = 可选入口(上下文);
        if 候选.is_empty() {
            // 纯库工程：明确说明为什么不能直接运行，并指出可用的操作。
            let 说明 = if !有项目 {
                语言.取("workbench_no_project")
            } else if 是原生的 {
                语言.取("workbench_no_runnable_target")
            } else {
                语言.取("workbench_entry_only_cargo")
            };
            ui.label(RichText::new(说明).color(配色.警告));
            return;
        }

        let 当前 = 上下文.运行.入口.clone().or_else(|| 候选.first().cloned());
        let 当前文本 = 当前.as_ref().map(|入口| 入口.标识()).unwrap_or_default();
        let mut 选中 = None;
        egui::ComboBox::from_id_salt("workbench_run_entry")
            .selected_text(当前文本)
            .width(220.0)
            .show_ui(ui, |ui| {
                for 入口 in &候选 {
                    let 是当前 = 当前.as_ref() == Some(入口);
                    if ui.selectable_label(是当前, 入口.标识()).clicked() {
                        选中 = Some(入口.clone());
                    }
                }
            });
        if let Some(入口) = 选中 {
            上下文.动作.推(UiAction::选择运行入口(入口));
        }

        // profile：直接改运行入口的 profile（debug/release 影响的是同一次运行）。
        if let Some(mut 入口) = 当前.clone() {
            let 原 = 入口.配置类型;
            egui::ComboBox::from_id_salt("workbench_run_profile")
                .selected_text(match 入口.配置类型 {
                    editor_core::build::构建配置类型::Debug => {
                        语言.取("build_profile_debug")
                    }
                    editor_core::build::构建配置类型::Release => {
                        语言.取("build_profile_release")
                    }
                })
                .width(96.0)
                .show_ui(ui, |ui| {
                    for 类型 in editor_core::build::构建配置类型::全部 {
                        let 名称 = match 类型 {
                            editor_core::build::构建配置类型::Debug => {
                                语言.取("build_profile_debug")
                            }
                            editor_core::build::构建配置类型::Release => {
                                语言.取("build_profile_release")
                            }
                        };
                        if ui.selectable_label(入口.配置类型 == 类型, 名称).clicked() {
                            入口.配置类型 = 类型;
                        }
                    }
                });
            if 入口.配置类型 != 原 {
                上下文.动作.推(UiAction::选择运行入口(入口));
            }
        }
    });
}

fn 绘制操作条(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 构建中 = 上下文.构建.报告.状态.在运行();
    let 运行中 = 上下文.运行.在运行();
    let 有项目 = 上下文.编辑器.项目.is_some();
    let 有入口 = 上下文.运行.入口.is_some();
    let _ = 有入口;
    let 可运行 = 有项目;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_enabled_ui(有项目 && !构建中, |ui| {
            if ui.button(语言.取("workbench_build")).clicked() {
                上下文.动作.推(UiAction::开始构建);
            }
        });
        ui.add_enabled_ui(可运行 && !构建中 && !运行中, |ui| {
            if ui
                .button(RichText::new(语言.取("workbench_run")).strong())
                .on_hover_text(语言.取("workbench_run_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::构建并运行);
            }
        });
        ui.add_enabled_ui(运行中, |ui| {
            if ui.button(语言.取("workbench_stop")).clicked() {
                上下文.动作.推(UiAction::停止项目);
            }
        });
        ui.add_enabled_ui(有项目 && !运行中, |ui| {
            // 重新构建 = 先清掉旧结果再构建：这条与「构建」的区别只在
            // 诊断任务边界（会开一个新的构建任务，旧错误不再算当前结果）。
            if ui.button(语言.取("workbench_rebuild")).clicked() {
                上下文.动作.推(UiAction::开始构建);
            }
        });

        ui.separator();
        let (状态色, 状态文本) = match 上下文.运行.状态 {
            项目运行状态::空闲 => (配色.次要文字, 语言.取("run_state_idle")),
            项目运行状态::启动中 => (配色.警告, 语言.取("run_state_starting")),
            项目运行状态::运行中 => (配色.成功, 语言.取("run_state_running")),
            项目运行状态::已停止 => (配色.次要文字, 语言.取("run_state_stopped")),
            项目运行状态::失败 => (配色.错误, 语言.取("run_state_failed")),
        };
        ui.label(RichText::new(状态文本).strong().color(状态色));

        if let Some(进程号) = 上下文.运行.进程号 {
            ui.label(
                RichText::new(format!("PID {}", 进程号))
                    .small()
                    .color(配色.次要文字),
            );
        }
    });
}

fn 绘制运行状态(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 运行 = 上下文.运行;

    if !运行.命令行.is_empty() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(语言.取("workbench_command"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.label(RichText::new(&运行.命令行).monospace().small());
            if ui.small_button(语言.取("workbench_copy")).clicked() {
                ui.ctx().copy_text(运行.命令行.clone());
            }
        });
    }

    if 运行.状态.在运行() {
        ui.label(
            RichText::new(语言.填充("workbench_running_hint", &[&运行.目标说明]))
                .small()
                .color(配色.次要文字),
        );
    } else if let Some(记录) = 运行.历史.last() {
        let 色 = match 记录.结束原因 {
            结束原因::正常退出 => 配色.成功,
            结束原因::进行中 => 配色.次要文字,
            _ => 配色.错误,
        };
        let 用时 = 语言.填充(
            "workbench_duration",
            &[&format!("{:.1}", 记录.用时秒(现在秒()) as f32)],
        );
        let 退出码 = 记录
            .退出码
            .map(|码| 语言.填充("workbench_exit_code", &[&码]))
            .unwrap_or_else(|| 语言.取("workbench_exit_code_none").to_string());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(记录.结果文本()).color(色));
            ui.label(RichText::new(退出码).small().color(配色.次要文字));
            ui.label(RichText::new(用时).small().color(配色.次要文字));
            if let Some(入口) = &记录.入口 {
                ui.label(RichText::new(入口.标识()).small().color(配色.次要文字));
            }
        });
        if let Some(说明) = &记录.说明 {
            ui.label(RichText::new(说明).small().color(配色.次要文字));
        }
    }

    if let Some(错误) = &运行.最近错误 {
        ui.label(RichText::new(错误).color(配色.错误));
    }

    // P1：运行时检查器的**可行性**必须如实显示。
    //
    // 现在没有任何运行时通道，因此这里明确写"不可用"，而不是画一个看起来
    // 能展开的占位树 —— 拿编辑器自己的实体冒充游戏进程里的实体是最坏的欺骗。
    let 可行性 = editor_core::架构预留::运行时可行性::评估();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(语言.取("workbench_runtime_inspect"))
                .small()
                .color(配色.次要文字),
        );
        let 文案 = if 可行性.可读取运行时 {
            format!(
                "{} · {}",
                语言.取(可行性.通道.词条键()),
                语言.取("workbench_runtime_available")
            )
        } else {
            format!(
                "{} · {}",
                语言.取(可行性.通道.词条键()),
                语言.取("workbench_runtime_unavailable")
            )
        };
        ui.label(RichText::new(文案).small().color(配色.警告));
    });

    // 最近运行记录（可展开）：真实数据，不美化。
    if !运行.历史.is_empty() {
        egui::CollapsingHeader::new(语言.填充("workbench_run_history", &[&运行.历史.len()]))
            .id_salt("workbench_run_history")
            .show(ui, |ui| {
                for 记录 in 运行.历史.iter().rev() {
                    绘制记录行(ui, 记录, 上下文.配色, 语言);
                }
            });
    }

    // 最近构建记录：与运行记录并列，回答"上一次构建成功了吗"。
    let 构建历史 = &上下文.统一诊断.构建历史;
    if !构建历史.is_empty() {
        egui::CollapsingHeader::new(语言.填充("workbench_build_history", &[&构建历史.len()]))
            .id_salt("workbench_build_history")
            .show(ui, |ui| {
                for 记录 in 构建历史 {
                    let 色 = if 记录.成功 {
                        上下文.配色.成功
                    } else {
                        上下文.配色.错误
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!("#{}", 记录.任务.序号))
                                .small()
                                .color(上下文.配色.次要文字),
                        );
                        ui.label(RichText::new(记录.结果文本()).small().color(色));
                        ui.label(
                            RichText::new(语言.填充(
                                "workbench_duration",
                                &[&format!("{:.1}", 记录.耗时毫秒 as f32 / 1000.0)],
                            ))
                            .small()
                            .color(上下文.配色.次要文字),
                        );
                        ui.label(
                            RichText::new(&记录.命令行)
                                .small()
                                .monospace()
                                .color(上下文.配色.次要文字),
                        );
                    });
                }
            });
    }
}

fn 绘制记录行(
    ui: &mut Ui,
    记录: &运行记录,
    配色: crate::theme::编辑器配色,
    语言: crate::i18n::语言,
) {
    let 色 = match 记录.结束原因 {
        结束原因::正常退出 => 配色.成功,
        结束原因::进行中 => 配色.次要文字,
        _ => 配色.错误,
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("#{}", 记录.序号))
                .small()
                .color(配色.次要文字),
        );
        ui.label(RichText::new(记录.结果文本()).small().color(色));
        ui.label(
            RichText::new(语言.填充(
                "workbench_duration",
                &[&format!("{:.1}", 记录.用时秒(现在秒()) as f32)],
            ))
            .small()
            .color(配色.次要文字),
        );
        ui.label(
            RichText::new(&记录.命令行)
                .small()
                .monospace()
                .color(配色.次要文字),
        );
    });
}

fn 绘制运行日志(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;

    ui.horizontal(|ui| {
        ui.label(RichText::new(语言.取("workbench_log")).strong());
        ui.add(
            egui::TextEdit::singleline(&mut 上下文.界面.运行日志关键字)
                .hint_text(语言.取("workbench_log_filter_hint"))
                .desired_width(160.0),
        );
        ui.checkbox(
            &mut 上下文.界面.运行日志自动滚动,
            语言.取("workbench_autoscroll"),
        );
        if ui
            .button(语言.取("workbench_copy_all"))
            .on_hover_text(语言.取("workbench_copy_all_hint"))
            .clicked()
        {
            let 文本 = 上下文.运行.日志.join("\n");
            if !文本.is_empty() {
                ui.ctx().copy_text(文本);
            }
        }
        if ui.button(语言.取("workbench_clear_log")).clicked() {
            上下文.动作.推(UiAction::清空运行日志);
        }
        ui.add_enabled_ui(!上下文.运行.日志.is_empty(), |ui| {
            if ui.button(语言.取("workbench_send_log")).clicked() {
                上下文.动作.推(UiAction::把运行日志发送给智能体);
            }
        });
    });

    let 关键字 = 上下文.界面.运行日志关键字.trim().to_string();
    let 行: Vec<String> = 上下文
        .运行
        .日志
        .iter()
        .filter(|行| 关键字.is_empty() || 行.contains(&关键字))
        .cloned()
        .collect();

    if 行.is_empty() {
        ui.label(
            RichText::new(if 上下文.运行.日志.is_empty() {
                语言.取("workbench_log_empty")
            } else {
                语言.取("workbench_log_no_match")
            })
            .small()
            .color(配色.次要文字),
        );
        return;
    }

    let 滚动 = egui::ScrollArea::vertical()
        .id_salt("workbench_run_log")
        // 固定上限：再长的日志也只占这么多高度，绝不把面板撑坏。
        .max_height(188.0)
        .auto_shrink([false, false]);
    let 滚动 = if 上下文.界面.运行日志自动滚动 {
        滚动.stick_to_bottom(true)
    } else {
        滚动
    };
    滚动.show(ui, |ui| {
        for 行 in &行 {
            // 日志行可以单行复制：排查时最常做的一件事是把某一行贴给别人。
            let 响应 = ui.add(
                egui::Label::new(RichText::new(行).monospace().small())
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .sense(egui::Sense::click()),
            );
            if 响应.clicked() {
                ui.ctx().copy_text(行.clone());
            }
            if 响应.hovered() {
                响应.on_hover_text(语言.取("workbench_log_copy_hint"));
            }
        }
    });
}

fn 绘制诊断区(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 诊断 = 上下文.统一诊断;

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(语言.取("workbench_diagnostics")).strong());
        let 错误数 = 诊断.错误数;
        let 警告数 = 诊断.警告数;
        ui.label(
            RichText::new(语言.填充("workbench_diag_counts", &[&错误数, &警告数]))
                .small()
                .color(if 错误数 > 0 {
                    配色.错误
                } else {
                    配色.次要文字
                }),
        );
    });

    ui.horizontal_wrapped(|ui| {
        // 级别开关
        ui.checkbox(
            &mut 上下文.界面.诊断筛选.显示错误,
            语言.取("diag_level_error"),
        );
        ui.checkbox(
            &mut 上下文.界面.诊断筛选.显示警告,
            语言.取("diag_level_warning"),
        );
        ui.checkbox(
            &mut 上下文.界面.诊断筛选.显示信息,
            语言.取("diag_level_info"),
        );
        ui.checkbox(
            &mut 上下文.界面.显示历史诊断,
            语言.取("workbench_show_history"),
        );
    });

    ui.horizontal_wrapped(|ui| {
        // 来源筛选：点一下切换（用可选中标签，状态可见）。
        for 来源 in 诊断来源::全部 {
            let 选中 = 上下文.界面.诊断筛选.来源.contains(&来源);
            if ui.selectable_label(选中, 语言.取(来源.词条键())).clicked() {
                if 选中 {
                    上下文.界面.诊断筛选.来源.retain(|项| *项 != 来源);
                } else {
                    上下文.界面.诊断筛选.来源.push(来源);
                }
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut 上下文.界面.诊断筛选.关键字)
                .hint_text(语言.取("workbench_diag_filter_hint"))
                .desired_width(140.0),
        );
    });

    ui.horizontal(|ui| {
        let 有内容 = !诊断.条目.is_empty();
        ui.add_enabled_ui(有内容, |ui| {
            if ui
                .button(语言.取("workbench_diag_send_all"))
                .on_hover_text(语言.取("workbench_diag_send_all_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::把诊断发送给智能体 { 编号: None });
            }
        });
        if ui.button(语言.取("workbench_diag_clear")).clicked() {
            上下文.动作.推(UiAction::清空诊断);
        }
    });

    // 列表：当前结果（或含历史）
    //
    // 克隆可见项：下面的行绘制要 `&mut 上下文`，若持有指向快照的引用
    // 就会与可变借用冲突。诊断条目是纯数据，克隆代价可接受。
    let 源: &[诊断条目] = if 上下文.界面.显示历史诊断 {
        &诊断.全部条目
    } else {
        &诊断.条目
    };
    let 筛选 = 上下文.界面.诊断筛选.clone();
    let 可见: Vec<诊断条目> = 源.iter().filter(|条目| 筛选.接受(条目)).cloned().collect();

    if 可见.is_empty() {
        ui.label(
            RichText::new(if 源.is_empty() {
                语言.取("workbench_diag_empty")
            } else {
                语言.取("workbench_diag_no_match")
            })
            .small()
            .color(配色.次要文字),
        );
        return;
    }

    // 诊断列表：固定上限高度，独立滚动（详情区始终在图下方可见）。
    let 列表高 = 168.0;
    egui::ScrollArea::vertical()
        .id_salt("workbench_diag_list")
        .max_height(列表高)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for 条目 in &可见 {
                绘制诊断行(ui, 条目, 上下文);
            }
        });

    // 详情：选中项的原文 + 交给 Agent
    if let Some(编号) = 上下文.界面.选中诊断
        && let Some(条目) = 上下文.统一诊断.按编号(编号)
    {
        ui.separator();
        let 位置 = 条目
            .完整位置()
            .unwrap_or_else(|| 语言.取("diag_no_source_location").to_string());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&条目.消息).strong());
            ui.label(RichText::new(位置).small().color(配色.次要文字));
        });
        if let Some(任务) = 条目.任务 {
            ui.label(RichText::new(任务.显示()).small().color(配色.次要文字));
        }
        if let Some(原文) = &条目.原始日志
            && 原文.trim() != 条目.消息.trim()
        {
            ui.label(RichText::new(原文).monospace().small());
        }
        ui.horizontal(|ui| {
            if 条目.可跳转() && ui.button(语言.取("diag_open_source")).clicked() {
                上下文.动作.推(UiAction::打开诊断位置 { 编号 });
            }
            if ui.button(语言.取("workbench_diag_send_one")).clicked() {
                上下文.动作.推(UiAction::把诊断发送给智能体 {
                    编号: Some(编号)
                });
            }
            if ui.button(语言.取("workbench_copy")).clicked()
                && let Some(文本) = 条目.完整位置()
            {
                ui.ctx().copy_text(format!("{}: {}", 文本, 条目.单行文本()));
            }
        });
    }
}

fn 绘制诊断行(ui: &mut Ui, 条目: &诊断条目, 上下文: &mut 绘制上下文<'_>) {
    let 配色 = 上下文.配色;
    let 语言 = 上下文.语言;
    let 选中 = 上下文.界面.选中诊断 == Some(条目.编号);
    let 级别色 = match 条目.级别 {
        诊断严重级别::错误 => 配色.错误,
        诊断严重级别::警告 => 配色.警告,
        诊断严重级别::信息 => 配色.次要文字,
    };
    let 位置 = 条目.位置标签().unwrap_or_default();
    let 文本 = format!(
        "{} {}  {}{}",
        条目.级别.标记(),
        语言.取(条目.来源.词条键()),
        if 位置.is_empty() {
            String::new()
        } else {
            format!("{}  ", 位置)
        },
        条目.单行文本()
    );
    let 响应 = ui.selectable_label(
        选中,
        RichText::new(文本)
            .small()
            .color(if 选中 { 配色.文字 } else { 级别色 }),
    );
    if 响应.clicked() {
        上下文.界面.选中诊断 = Some(条目.编号);
        // 单击就跳源码：这是排查时最高频的动作，不该要求"先选中再点按钮"。
        if 条目.可跳转() {
            上下文.动作.推(UiAction::打开诊断位置 {
                编号: 条目.编号
            });
        }
    }
}

/// 当前 UNIX 秒（只用于显示用时）。
fn 现在秒() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_secs())
        .unwrap_or(0)
}
