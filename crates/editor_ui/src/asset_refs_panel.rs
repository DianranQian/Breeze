//! 「查找引用」对话框：一个资源被哪些场景、哪些实体的哪个字段引用。
//!
//! # 为什么这不是"删除守卫"的重复
//!
//! 删除守卫只在删除时回答"能不能删"。但用户更常见的需求是**主动排查**：
//!
//! * 这张图到底还有没有在用？改它之前要先确认影响面；
//! * 同一个资源被两个场景引用，改了一处忘了另一处；
//! * 想从资源直接跳到场景里那个实体，而不是靠场景树里翻。
//!
//! 因此这里不是一个确认框，而是一张可点击的清单：
//! 每一行显示 `场景 / 实体 / 组件.属性`，点「定位」直接打开场景并选中实体。

use egui::Ui;

use crate::i18n;
use crate::state::{UiAction, 界面状态};
use crate::theme::编辑器配色;

/// 绘制查找引用对话框；返回需要排进动作队列的意图。
pub fn 绘制(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    语言: i18n::语言,
) -> Option<UiAction> {
    let 草稿 = 界面.查找引用.clone()?;
    let mut 要跳转 = None;
    let mut 要关闭 = false;

    let 窗口响应 = egui::Window::new(语言.取("asset_find_refs_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(620.0)
        .show(egui上下文, |ui: &mut Ui| {
            ui.label(
                egui::RichText::new(草稿.资源.文本())
                    .monospace()
                    .color(配色.文字),
            );
            ui.label(
                egui::RichText::new(语言.填充(
                    "asset_find_refs_count",
                    &[&草稿.行.len()],
                ))
                .small()
                .color(配色.次要文字),
            );
            ui.separator();

            if 草稿.行.is_empty() {
                ui.label(
                    egui::RichText::new(语言.取("asset_find_refs_none"))
                        .small()
                        .color(配色.次要文字),
                );
            }

            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (下标, 行) in 草稿.行.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{}. {}", 下标 + 1, 行.文本()))
                                    .small()
                                    .monospace()
                                    .color(配色.文字),
                            );
                            if ui.button(语言.取("asset_find_refs_go")).clicked() {
                                要跳转 = Some((行.场景.clone(), 行.实体));
                            }
                        });
                    }
                });

            for 问题 in &草稿.问题 {
                ui.label(egui::RichText::new(问题).small().color(配色.错误));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(语言.取("close")).clicked() {
                    要关闭 = true;
                }
            });
        });

    crate::记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_find_refs",
        窗口响应.map(|响应| 响应.response.rect),
    );

    if let Some((场景, 实体)) = 要跳转 {
        return Some(UiAction::打开引用位置 { 场景, 实体 });
    }
    if 要关闭 {
        return Some(UiAction::关闭查找引用);
    }
    None
}
