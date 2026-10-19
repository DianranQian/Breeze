//! AI Agent 工作区的三个面板：对话、会话、审查。
//!
//! # 职责边界（避免同一份数据在多处重复展示）
//!
//! | 面板 | 负责 | 不负责 |
//! | --- | --- | --- |
//! | 对话 | 消息流、任务状态、工具调用时间线、输入 | 会话管理、差异详情 |
//! | 会话 | 会话列表与增删改、DSH 连接状态与版本 | 消息正文 |
//! | 审查 | 附加的上下文、真实文件变更与差异、验证状态 | 对话 |
//!
//! # 所有内容都来自真实状态
//!
//! 面板不生成任何"看起来像执行过"的数据：消息由 DSH 事件写入，
//! 变更由 `git status`/`git diff` 写入，连接状态由探测写入。
//! 面板只做展示与推动作。

use egui::{RichText, Ui};

use crate::agent::{任务状态, 消息角色};
use crate::state::{UiAction, 绘制上下文};

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

    // 顶部：任务状态条（真实状态，不是模型措辞）
    let (状态键, 有运行中) = {
        let 会话 = 上下文.界面.智能体.当前().expect("上面已判空");
        match 会话.任务.first() {
            Some(任务) => (任务.状态.词条键(), 任务.状态 == 任务状态::执行中),
            None => ("agent_task_idle", false),
        }
    };
    ui.horizontal_wrapped(|ui| {
        let 颜色 = if 有运行中 {
            配色.警告
        } else {
            配色.次要文字
        };
        ui.label(RichText::new(语言.取(状态键)).small().color(颜色));
        if 有运行中 {
            ui.spinner();
            if ui.button(语言.取("agent_cancel")).clicked() {
                上下文.动作.推(UiAction::智能体取消);
            }
        }
        if let Some(提示) = 上下文.界面.智能体.提示.clone() {
            ui.label(RichText::new(提示).small().color(配色.次要文字));
        }
    });
    ui.separator();

    // 消息流 + 工具调用时间线
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

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height(ui.available_height() - 84.0)
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
                let (标题, 条数, 有变更) = {
                    let 会话 = &上下文.界面.智能体.会话列表[下标];
                    (会话.标题.clone(), 会话.消息.len(), 会话.有变更())
                };
                let 选中 = 上下文.界面.智能体.当前会话 == Some(下标);
                let 文本 = format!("{标题}  ({条数})");
                let 响应 = ui.selectable_label(
                    选中,
                    RichText::new(文本).color(if 有变更 {
                        配色.警告
                    } else {
                        配色.文字
                    }),
                );
                if 响应.clicked() && !选中 {
                    上下文.动作.推(UiAction::智能体选择会话(下标));
                }
                if 有变更 {
                    响应.on_hover_text(语言.取("agent_session_has_changes"));
                }
            }
        });
}

/// 绘制审查面板：上下文 + 真实文件变更 + 差异 + 验证状态。
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

    // ---- 上下文 ----
    ui.horizontal(|ui| {
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
    });
    let 上下文列表 = 上下文
        .界面
        .智能体
        .当前()
        .map(|会话| 会话.上下文.clone())
        .unwrap_or_default();
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
    // ---- 文件变更 ----
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(语言.取("agent_changes"))
                .strong()
                .color(配色.文字),
        );
        if ui.button(语言.取("agent_refresh_changes")).clicked() {
            上下文.动作.推(UiAction::智能体刷新变更);
        }
    });
    let 变更 = 上下文
        .界面
        .智能体
        .当前()
        .map(|会话| 会话.变更.clone())
        .unwrap_or_default();
    if 变更.is_empty() {
        ui.label(
            RichText::new(语言.取("agent_changes_empty"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for 项 in &变更 {
                let 颜色 = if 项.已验证 {
                    配色.成功
                } else {
                    配色.警告
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("{} {}", 项.状态.trim(), 项.路径))
                            .small()
                            .monospace()
                            .color(颜色),
                    );
                    ui.label(
                        RichText::new(if 项.已验证 {
                            语言.取("agent_change_verified")
                        } else {
                            语言.取("agent_change_unverified")
                        })
                        .small()
                        .color(颜色),
                    );
                });
                if !项.差异.is_empty() {
                    let mut 差异 = 项.差异.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut 差异)
                            .desired_width(f32::INFINITY)
                            .desired_rows(6)
                            .interactive(false)
                            .font(egui::TextStyle::Monospace),
                    );
                }
            }
        });
}
