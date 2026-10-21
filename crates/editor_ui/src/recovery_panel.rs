//! 「恢复自动保存」对话框：把启动时发现的恢复点如实列出来。
//!
//! # 为什么是列表而不是"直接恢复最新一个"
//!
//! 自动恢复看起来省事，但会带来两个真实问题：
//!
//! 1. 用户在恢复点之后又打开/编辑了别的场景，自动恢复会把它们换掉；
//! 2. 撤销栈已经清空，用户退不回去。
//!
//! 所以这里只做三件事：列出恢复点、让用户选一个恢复、或者丢弃。
//! 恢复也**不写盘**——内容装进编辑器后，用户自己决定保存到哪。

use egui::Ui;

use crate::i18n;
use crate::state::{UiAction, 界面状态};
use crate::theme::编辑器配色;

/// 绘制恢复点对话框；返回需要排进动作队列的意图。
pub fn 绘制(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    语言: i18n::语言,
) -> Option<UiAction> {
    if 界面.恢复点候选.is_empty() && 界面.恢复点错误.is_none() {
        return None;
    }

    let mut 要恢复 = None;
    let mut 要丢弃 = None;
    let mut 要关闭 = false;
    let 候选 = 界面.恢复点候选.clone();
    let 错误 = 界面.恢复点错误.clone();

    let 窗口响应 = egui::Window::new(语言.取("recovery_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .show(egui上下文, |ui: &mut Ui| {
            ui.label(语言.取("recovery_intro"));
            if let Some(错误) = &错误 {
                ui.separator();
                ui.label(egui::RichText::new(错误).small().color(配色.错误));
            }
            ui.separator();
            for 项 in &候选 {
                ui.horizontal_wrapped(|ui| {
                    let 名称 = if 项.场景名.trim().is_empty() {
                        语言.取("recovery_unnamed")
                    } else {
                        项.场景名.as_str()
                    };
                    ui.label(egui::RichText::new(名称).strong().color(配色.文字));
                    let 位置 = 项.场景路径.as_deref().unwrap_or("—");
                    ui.label(
                        egui::RichText::new(位置)
                            .small()
                            .monospace()
                            .color(配色.次要文字),
                    );
                    ui.label(
                        egui::RichText::new(
                            语言.填充("recovery_item_meta", &[&项.实体数, &项.时间]),
                        )
                        .small()
                        .color(配色.次要文字),
                    );
                    if ui.button(语言.取("recovery_restore")).clicked() {
                        要恢复 = Some(项.clone());
                    }
                    if ui.button(语言.取("recovery_discard")).clicked() {
                        要丢弃 = Some(项.clone());
                    }
                });
                ui.separator();
            }
            ui.label(
                egui::RichText::new(语言.取("recovery_note"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.horizontal(|ui| {
                if ui.button(语言.取("recovery_later")).clicked() {
                    要关闭 = true;
                }
            });
        });

    // 几何证据：浮层矩形进验收数据（与项目设置/资源对话框同一机制）。
    crate::记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_recovery",
        窗口响应.map(|响应| 响应.response.rect),
    );

    if let Some(元数据) = 要恢复 {
        return Some(UiAction::恢复自动保存(元数据));
    }
    if let Some(元数据) = 要丢弃 {
        return Some(UiAction::丢弃自动保存(元数据));
    }
    if 要关闭 {
        return Some(UiAction::关闭恢复提示);
    }
    None
}
