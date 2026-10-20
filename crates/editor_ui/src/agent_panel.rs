//! AI Agent 工作区的三个面板：对话、会话、审查。
//!
//! # 职责边界（避免同一份数据在多处重复展示）
//!
//! | 面板 | 负责 | 不负责 |
//! | --- | --- | --- |
//! | 对话 | 消息流、任务状态、自动修复进度、工具调用时间线、输入 | 会话管理、差异详情 |
//! | 会话 | 会话列表与增删改、DSH 连接状态、存档载入提示 | 消息正文 |
//! | 审查 | 上下文、权限与审批、真实文件变更/差异/内容、验证状态、批准与拒绝 | 对话 |
//!
//! # 所有内容都来自真实状态
//!
//! 面板不生成任何"看起来像执行过"的数据：消息由 DSH 事件写入，
//! 变更由 `git status`/`git diff` 写入，验证状态由真实退出码写入，
//! 审批请求由真实的危险模式检测或权限策略产生。面板只做展示与推动作。

use egui::{RichText, Ui};

use crate::agent::{任务状态, 变更来源, 审查状态, 消息角色};
use crate::agent_review::统计;
use crate::state::{UiAction, 绘制上下文};

/// 只读地读取工作区里的文件内容（最多 `行数` 行）。
///
/// 拒绝绝对路径与任何 `..` 组件：审查面板只允许看工作区内的文件。
fn 读取文件内容(根: &std::path::Path, 相对路径: &str, 行数: usize) -> Option<String> {
    let 相对 = std::path::Path::new(相对路径);
    if 相对.as_os_str().is_empty() || 相对.is_absolute() {
        return None;
    }
    if 相对
        .components()
        .any(|组件| !matches!(组件, std::path::Component::Normal(_)))
    {
        return None;
    }
    let 文本 = std::fs::read_to_string(根.join(相对)).ok()?;
    Some(文本.lines().take(行数).collect::<Vec<_>>().join("\n"))
}

/// 绘制对话面板。
pub fn 绘制对话(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 有会话 = 上下文.界面.智能体.当前().is_some();
    if !有会话 {
        ui.label(
            RichText::new(语言.取("agent_no_session"))
                .small()
                .color(配色.次要文字),
        );
        if ui.button(语言.取("agent_new_session")).clicked() {
            上下文.动作.推(UiAction::智能体新建会话);
        }
        return;
    }

    let (状态键, 有运行中, 有任务) = {
        let 会话 = 上下文.界面.智能体.当前().expect("上面已判空");
        match 会话.任务.first() {
            Some(任务) => (
                任务.状态.词条键(),
                任务.状态 == 任务状态::执行中 || 上下文.界面.智能体.运行中,
                true,
            ),
            None => ("agent_task_idle", 上下文.界面.智能体.运行中, false),
        }
    };
    let 修复阶段 = 上下文.界面.智能体.修复.阶段;
    let 修复记录数 = 上下文.界面.智能体.修复.记录.len();

    ui.horizontal_wrapped(|ui| {
        let 颜色 = if 有运行中 {
            配色.警告
        } else {
            配色.次要文字
        };
        ui.label(RichText::new(语言.取(状态键)).small().color(颜色));
        if 修复阶段.进行中() || 修复记录数 > 0 {
            ui.label(
                RichText::new(语言.取(修复阶段.词条键()))
                    .small()
                    .color(配色.信息),
            );
            ui.label(
                RichText::new(语言.填充(
                    "agent_repair_progress",
                    &[
                        &上下文.界面.智能体.修复.已执行轮数,
                        &上下文.界面.智能体.修复.上限,
                    ],
                ))
                .small()
                .color(配色.次要文字),
            );
        }
        if 有运行中 {
            ui.spinner();
            if ui.button(语言.取("agent_cancel")).clicked() {
                上下文.动作.推(UiAction::智能体取消);
            }
        }
        if 有任务 && !有运行中 {
            if ui.button(语言.取("agent_retry")).clicked() {
                上下文.动作.推(UiAction::智能体重试);
            }
            if 上下文.界面.智能体.连接.可用() && ui.button(语言.取("agent_continue")).clicked()
            {
                上下文.动作.推(UiAction::智能体继续);
            }
        }
    });
    if let Some(提示) = 上下文.界面.智能体.提示.clone() {
        ui.label(RichText::new(提示).small().color(配色.次要文字));
    }
    ui.separator();

    let 消息 = 上下文
        .界面
        .智能体
        .当前()
        .map(|会话| 会话.消息.clone())
        .unwrap_or_default();
    let 工具 = 上下文
        .界面
        .智能体
        .当前()
        .and_then(|会话| 会话.任务.first())
        .map(|任务| 任务.工具调用.clone())
        .unwrap_or_default();

    let 高度 = (ui.available_height() - 110.0).max(80.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height(高度)
        .show(ui, |ui| {
            for 条目 in &消息 {
                let (前缀键, 颜色) = match 条目.角色 {
                    消息角色::用户 => ("agent_role_user", 配色.强调),
                    消息角色::智能体 => ("agent_role_agent", 配色.文字),
                    消息角色::系统 => ("agent_role_system", 配色.警告),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(语言.取(前缀键)).small().color(颜色));
                });
                // 用只读多行文本显示：可选中复制，保留换行与代码缩进。
                let mut 正文 = 条目.内容.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut 正文)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .interactive(false)
                        .font(egui::TextStyle::Monospace),
                );
                ui.separator();
            }
            if !工具.is_empty() {
                ui.label(
                    RichText::new(语言.取("agent_tool_calls"))
                        .small()
                        .color(配色.次要文字),
                );
                for 调用 in &工具 {
                    let 颜色 = if 调用.写操作 {
                        配色.警告
                    } else {
                        配色.次要文字
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!("{} · {}", 调用.名称, 调用.阶段))
                                .small()
                                .monospace()
                                .color(颜色),
                        );
                        if !调用.摘要.is_empty() {
                            ui.label(RichText::new(&调用.摘要).small().color(配色.文字));
                        }
                    });
                }
            }
        });

    // 输入区
    ui.separator();
    ui.horizontal(|ui| {
        let mut 输入 = std::mem::take(&mut 上下文.界面.智能体.输入);
        let 响应 = ui.add(
            egui::TextEdit::multiline(&mut 输入)
                .hint_text(语言.取("agent_input_hint"))
                .desired_width(ui.available_width() - 180.0)
                .desired_rows(3),
        );
        let 回车发送 = 响应.has_focus()
            && ui.input(|输入状态| {
                输入状态.key_pressed(egui::Key::Enter) && !输入状态.modifiers.shift
            });
        上下文.界面.智能体.输入 = 输入;
        let 可发送 = !上下文.界面.智能体.输入.trim().is_empty()
            && !上下文.界面.智能体.运行中
            && 上下文.界面.智能体.连接.可用();
        if ui
            .add_enabled(可发送, egui::Button::new(语言.取("agent_send")))
            .clicked()
            || (可发送 && 回车发送)
        {
            上下文.动作.推(UiAction::智能体发送);
        }
        if !上下文.界面.智能体.连接.可用() {
            ui.label(
                RichText::new(语言.取("agent_dsh_unavailable"))
                    .small()
                    .color(配色.错误),
            );
        }
    });
}

/// 绘制会话面板：会话列表 + DSH 连接状态 + 新建/重命名/删除。
pub fn 绘制会话(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;

    // DSH 状态（真实探测结果）
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(语言.取("agent_dsh_status"))
                .small()
                .color(配色.次要文字),
        );
        let (文本, 颜色) = match &上下文.界面.智能体.连接 {
            crate::agent::Dsh连接::就绪 { 版本, .. } => (版本.clone(), 配色.成功),
            crate::agent::Dsh连接::未安装 { 原因 } => (
                format!("{}：{原因}", 语言.取("agent_dsh_missing")),
                配色.错误,
            ),
            crate::agent::Dsh连接::失败 { 错误 } => (
                format!("{}：{错误}", 语言.取("agent_dsh_failed")),
                配色.错误,
            ),
            crate::agent::Dsh连接::未探测 => {
                (语言.取("agent_dsh_unknown").to_string(), 配色.次要文字)
            }
        };
        ui.label(RichText::new(文本).small().color(颜色));
        if ui.small_button(语言.取("agent_dsh_detect")).clicked() {
            上下文.动作.推(UiAction::智能体探测Dsh);
        }
    });
    if let crate::agent::Dsh连接::未安装 { .. } = 上下文.界面.智能体.连接 {
        ui.label(
            RichText::new(语言.取("agent_dsh_install_hint"))
                .small()
                .color(配色.次要文字),
        );
    }
    if let Some(提示) = 上下文.界面.智能体.载入提示.clone() {
        ui.label(RichText::new(提示).small().color(配色.次要文字));
    }
    ui.separator();

    let 项目路径 = 上下文
        .编辑器
        .项目
        .as_ref()
        .map(|项目| crate::agent::规范化项目路径(项目.根目录()))
        .unwrap_or_default();
    let 下标列表: Vec<usize> = if 项目路径.is_empty() {
        Vec::new()
    } else {
        上下文.界面.智能体.项目会话(&项目路径)
    };

    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !项目路径.is_empty(),
                egui::Button::new(语言.取("agent_new_session")),
            )
            .clicked()
        {
            上下文.动作.推(UiAction::智能体新建会话);
        }
        if ui
            .add_enabled(
                上下文.界面.智能体.当前().is_some(),
                egui::Button::new(语言.取("rename")),
            )
            .clicked()
        {
            上下文.动作.推(UiAction::智能体开始重命名);
        }
        if ui
            .add_enabled(
                上下文.界面.智能体.当前().is_some(),
                egui::Button::new(语言.取("delete")),
            )
            .clicked()
        {
            上下文.动作.推(UiAction::智能体删除会话);
        }
    });

    // 重命名对话框（用行内编辑，避免再造一个全局对话框）
    if let Some(mut 草稿) = 上下文.界面.智能体.重命名草稿.clone() {
        ui.horizontal(|ui| {
            let 响应 = ui.add(
                egui::TextEdit::singleline(&mut 草稿)
                    .hint_text(语言.取("agent_session_title_hint")),
            );
            上下文.界面.智能体.重命名草稿 = Some(草稿.clone());
            if ui.button(语言.取("confirm")).clicked() || 响应.lost_focus() {
                上下文.动作.推(UiAction::智能体确认重命名(草稿));
            }
            if ui.button(语言.取("cancel")).clicked() {
                上下文.动作.推(UiAction::智能体取消重命名);
            }
        });
    }

    ui.separator();
    if 下标列表.is_empty() {
        ui.label(
            RichText::new(语言.取("agent_no_sessions_for_project"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for 下标 in 下标列表 {
                let (标题, 条数, 有变更, 有未审查, 项目可用, 状态说明) = {
                    let 会话 = &上下文.界面.智能体.会话列表[下标];
                    (
                        会话.标题.clone(),
                        会话.消息.len(),
                        会话.有变更(),
                        会话.有未审查变更(),
                        会话.项目可用,
                        会话.项目状态说明.clone(),
                    )
                };
                let 选中 = 上下文.界面.智能体.当前会话 == Some(下标);
                let 文本 = format!("{标题}  ({条数})");
                let 颜色 = if !项目可用 {
                    配色.错误
                } else if 有未审查 {
                    配色.警告
                } else {
                    配色.文字
                };
                let 响应 = ui.selectable_label(选中, RichText::new(文本).color(颜色));
                if 响应.clicked() && !选中 {
                    上下文.动作.推(UiAction::智能体选择会话(下标));
                }
                if !项目可用 {
                    响应.on_hover_text(语言.取("agent_project_unavailable"));
                    ui.label(
                        RichText::new(format!(
                            "  {}",
                            状态说明.unwrap_or_else(|| {
                                语言.取("agent_project_unavailable").to_string()
                            })
                        ))
                        .small()
                        .color(配色.错误),
                    );
                } else if 有未审查 || 有变更 {
                    响应.on_hover_text(语言.取("agent_session_has_changes"));
                }
            }
        });
}

/// 绘制审查面板：上下文、权限审批、自动修复、真实文件变更与差异。
pub fn 绘制审查(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    if 上下文.界面.智能体.当前().is_none() {
        ui.label(
            RichText::new(语言.取("agent_no_session"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }

    // 先克隆界面数据：下面的闭包会持续 `&mut 上下文` 推动作。
    let 上下文列表 = 上下文
        .界面
        .智能体
        .当前()
        .map(|会话| 会话.上下文.clone())
        .unwrap_or_default();
    let 变更 = 上下文
        .界面
        .智能体
        .当前()
        .map(|会话| 会话.变更.clone())
        .unwrap_or_default();
    let 审批 = 上下文.界面.智能体.审批.clone();
    let 修复 = 上下文.界面.智能体.修复.clone();
    let 运行中 = 上下文.界面.智能体.运行中;
    let 查看内容 = 上下文.界面.智能体.查看内容.clone();
    let 策略 = 上下文.界面.智能体.权限.clone();
    let 项目根 = 上下文
        .编辑器
        .项目
        .as_ref()
        .map(|项目| 项目.根目录().to_path_buf());

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // ---- 上下文 ----
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(语言.取("agent_context"))
                        .strong()
                        .color(配色.文字),
                );
                let 有选中 = 上下文.编辑器.选择.主选中().is_some();
                if ui
                    .add_enabled(有选中, egui::Button::new(语言.取("agent_attach_entity")))
                    .clicked()
                {
                    上下文.动作.推(UiAction::智能体附加实体上下文);
                }
                if ui.button(语言.取("agent_attach_scene")).clicked() {
                    上下文.动作.推(UiAction::智能体附加场景上下文);
                }
                if ui.button(语言.取("agent_attach_project")).clicked() {
                    上下文.动作.推(UiAction::智能体附加项目上下文);
                }
                if ui.button(语言.取("agent_attach_native")).clicked() {
                    上下文.动作.推(UiAction::智能体附加原生工程上下文);
                }
            });
            if 上下文列表.is_empty() {
                ui.label(
                    RichText::new(语言.取("agent_context_empty"))
                        .small()
                        .color(配色.次要文字),
                );
            }
            for (下标, 项) in 上下文列表.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(语言.取(项.种类键())).small().color(配色.强调));
                    ui.label(RichText::new(&项.名称).small().color(配色.文字));
                    ui.label(
                        RichText::new(format!("{} 字符", 项.正文.chars().count()))
                            .small()
                            .color(配色.次要文字),
                    );
                    if ui.small_button(语言.取("remove")).clicked() {
                        上下文.动作.推(UiAction::智能体移除上下文(下标));
                    }
                });
            }

            ui.separator();
            // ---- 权限与审批 ----
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(语言.取("agent_permission"))
                        .strong()
                        .color(配色.文字),
                );
                ui.label(
                    RichText::new(format!(
                        "{}：{}",
                        语言.取("agent_permission_policy_auto"),
                        语言.取(策略.自动执行上限.词条键())
                    ))
                    .small()
                    .color(配色.次要文字),
                );
            });
            ui.label(
                RichText::new(语言.取("agent_permission_note"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.label(
                RichText::new(语言.取("agent_approvals"))
                    .strong()
                    .color(配色.文字),
            );
            if 审批.is_empty() {
                ui.label(
                    RichText::new(语言.取("agent_no_approvals"))
                        .small()
                        .color(配色.次要文字),
                );
            }
            for 请求 in &审批 {
                if !请求.待决() {
                    continue;
                }
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(语言.取(请求.风险.词条键()))
                                .small()
                                .color(配色.警告),
                        );
                        ui.label(RichText::new(&请求.描述).small().color(配色.文字));
                    });
                    if !请求.证据.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "{}：{}",
                                语言.取("agent_approval_evidence"),
                                请求.证据.join("、")
                            ))
                            .small()
                            .color(配色.错误),
                        );
                    }
                    ui.label(
                        RichText::new(语言.取("agent_approval_pending"))
                            .small()
                            .color(配色.次要文字),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(语言.取("agent_approval_allow")).clicked() {
                            上下文.动作.推(UiAction::智能体批准审批(请求.标识.clone()));
                        }
                        if ui.button(语言.取("agent_approval_deny")).clicked() {
                            上下文.动作.推(UiAction::智能体拒绝审批(请求.标识.clone()));
                        }
                    });
                });
            }

            ui.separator();
            // ---- 自动修复 ----
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(语言.取("agent_repair"))
                        .strong()
                        .color(配色.文字),
                );
                if ui
                    .add_enabled(!运行中, egui::Button::new(语言.取("agent_repair_build")))
                    .clicked()
                {
                    上下文.动作.推(UiAction::智能体自动修复构建);
                }
                if ui
                    .add_enabled(!运行中, egui::Button::new(语言.取("agent_repair_test")))
                    .clicked()
                {
                    上下文.动作.推(UiAction::智能体自动修复测试);
                }
            });
            ui.label(
                RichText::new(语言.取("agent_repair_need_dsh"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.horizontal_wrapped(|ui| {
                let 颜色 = match 修复.阶段 {
                    crate::agent_repair::修复阶段::已成功 => 配色.成功,
                    crate::agent_repair::修复阶段::已失败
                    | crate::agent_repair::修复阶段::环境阻塞
                    | crate::agent_repair::修复阶段::已取消 => 配色.错误,
                    crate::agent_repair::修复阶段::等待验证
                    | crate::agent_repair::修复阶段::等待Agent => 配色.警告,
                    crate::agent_repair::修复阶段::空闲 => 配色.次要文字,
                };
                ui.label(
                    RichText::new(语言.取(修复.阶段.词条键()))
                        .small()
                        .color(颜色),
                );
                if let Some(原因) = &修复.结束原因 {
                    ui.label(RichText::new(原因).small().color(配色.次要文字));
                }
            });
            if !修复.记录.is_empty() {
                ui.label(
                    RichText::new(语言.取("agent_repair_rounds"))
                        .small()
                        .color(配色.次要文字),
                );
                for 记录 in &修复.记录 {
                    let 退出码 = 记录
                        .退出码
                        .map(|码| 码.to_string())
                        .unwrap_or_else(|| "—".to_string());
                    let 类别 = 语言.取(记录.错误类别.词条键());
                    ui.label(
                        RichText::new(语言.填充(
                            "agent_repair_round_line",
                            &[&记录.轮数, &退出码, &类别, &记录.变更文件.len()],
                        ))
                        .small()
                        .monospace()
                        .color(if 记录.成功 {
                            配色.成功
                        } else {
                            配色.错误
                        }),
                    );
                }
            } else {
                ui.label(
                    RichText::new(语言.取("agent_no_rounds"))
                        .small()
                        .color(配色.次要文字),
                );
            }

            ui.separator();
            // ---- 文件变更 ----
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(语言.取("agent_changes"))
                        .strong()
                        .color(配色.文字),
                );
                if ui.button(语言.取("agent_refresh_changes")).clicked() {
                    上下文.动作.推(UiAction::智能体刷新变更);
                }
            });
            if 变更.is_empty() {
                ui.label(
                    RichText::new(语言.取("agent_changes_empty"))
                        .small()
                        .color(配色.次要文字),
                );
                return;
            }
            let 统计 = 统计(&变更);
            ui.label(
                RichText::new(format!(
                    "{}：{} · {} · {} · {} · {}",
                    语言.取("agent_changes_stats"),
                    语言.填充("agent_stat_added", &[&统计.新增]),
                    语言.填充("agent_stat_modified", &[&统计.修改]),
                    语言.填充("agent_stat_deleted", &[&统计.删除]),
                    语言.填充("agent_stat_pending", &[&统计.待审查]),
                    语言.填充("agent_stat_verified", &[&统计.已验证]),
                ))
                .small()
                .color(配色.次要文字),
            );

            for 项 in &变更 {
                let 标题 = format!(
                    "{} {} · {} · {} · {}",
                    语言.取(项.种类.词条键()),
                    项.路径,
                    语言.取(项.来源.词条键()),
                    语言.取(项.审查.词条键()),
                    语言.取(项.验证.词条键()),
                );
                let 颜色 = if 项.来源 == 变更来源::任务前已有 {
                    配色.次要文字
                } else if 项.审查 == 审查状态::冲突 || 项.审查 == 审查状态::无法撤销
                {
                    配色.错误
                } else if 项.验证.通过() {
                    配色.成功
                } else {
                    配色.警告
                };
                egui::CollapsingHeader::new(RichText::new(标题).color(颜色))
                    .id_salt(format!("agent-change-{}", 项.路径))
                    .default_open(项.审查 == 审查状态::待审查 && 项.来源 == 变更来源::本任务)
                    .show(ui, |ui| {
                        if let Some(原路径) = &项.原路径 {
                            ui.label(
                                RichText::new(format!("{} → {原路径}", 项.路径))
                                    .small()
                                    .monospace()
                                    .color(配色.次要文字),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{}：{}",
                                    语言.取("agent_baseline_fingerprint"),
                                    crate::agent_review::短指纹(项.基线指纹.as_deref())
                                ))
                                .small()
                                .color(配色.次要文字),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{}：{}",
                                    语言.取("agent_current_fingerprint"),
                                    crate::agent_review::短指纹(项.当前指纹.as_deref())
                                ))
                                .small()
                                .color(配色.次要文字),
                            );
                        });
                        if let Some(备注) = &项.备注 {
                            ui.label(
                                RichText::new(format!("{}：{备注}", 语言.取("agent_note")))
                                    .small()
                                    .color(配色.次要文字),
                            );
                        }
                        if let Some(任务) = &项.任务标识 {
                            ui.label(
                                RichText::new(format!("{}：{任务}", 语言.取("agent_related_task")))
                                    .small()
                                    .monospace()
                                    .color(配色.次要文字),
                            );
                        }
                        if !项.验证.通过() {
                            ui.label(
                                RichText::new(语言.取("agent_review_unverified_hint"))
                                    .small()
                                    .color(配色.警告),
                            );
                        }

                        ui.label(
                            RichText::new(语言.取("agent_diff"))
                                .small()
                                .color(配色.次要文字),
                        );
                        let mut 差异 = if 项.差异.is_empty() {
                            语言.取("agent_changes_empty").to_string()
                        } else {
                            项.差异.clone()
                        };
                        egui::ScrollArea::vertical()
                            .id_salt(format!("agent-diff-{}", 项.路径))
                            .max_height(240.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut 差异)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(8)
                                        .interactive(false)
                                        .font(egui::TextStyle::Monospace),
                                );
                            });

                        // 文件内容（真实磁盘读取，只读展示）。
                        let 正在查看 = 查看内容.as_deref() == Some(项.路径.as_str());
                        ui.horizontal(|ui| {
                            if ui
                                .button(if 正在查看 {
                                    语言.取("agent_hide_content")
                                } else {
                                    语言.取("agent_view_content")
                                })
                                .clicked()
                            {
                                let 新值 = if 正在查看 {
                                    None
                                } else {
                                    Some(项.路径.clone())
                                };
                                上下文.界面.智能体.查看内容 = 新值;
                            }
                        });
                        if 正在查看 {
                            let 正文 = 项目根
                                .as_ref()
                                .and_then(|根| 读取文件内容(根, &项.路径, 400));
                            match 正文 {
                                Some(内容) => {
                                    ui.label(
                                        RichText::new(语言.取("agent_content_truncated"))
                                            .small()
                                            .color(配色.次要文字),
                                    );
                                    let mut 内容 = 内容;
                                    egui::ScrollArea::vertical()
                                        .id_salt(format!("agent-content-{}", 项.路径))
                                        .max_height(240.0)
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::TextEdit::multiline(&mut 内容)
                                                    .desired_width(f32::INFINITY)
                                                    .desired_rows(10)
                                                    .interactive(false)
                                                    .font(egui::TextStyle::Monospace),
                                            );
                                        });
                                }
                                None => {
                                    ui.label(
                                        RichText::new(语言.填充("agent_read_failed", &[&项.路径]))
                                            .small()
                                            .color(配色.错误),
                                    );
                                }
                            }
                        }

                        // 批准 / 拒绝：真实动作，不是标签。
                        let 可批准 =
                            项.审查 == 审查状态::待审查 && 项.来源 != 变更来源::任务前已有;
                        let 可拒绝 = 项.可回退 && 项.审查 == 审查状态::待审查;
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(可批准, egui::Button::new(语言.取("agent_approve")))
                                .clicked()
                            {
                                上下文.动作.推(UiAction::智能体批准变更(项.路径.clone()));
                            }
                            if ui
                                .add_enabled(可拒绝, egui::Button::new(语言.取("agent_reject")))
                                .on_disabled_hover_text(语言.取("agent_reject_unavailable"))
                                .clicked()
                            {
                                上下文.动作.推(UiAction::智能体拒绝变更(项.路径.clone()));
                            }
                        });
                    });
            }
        });
}
