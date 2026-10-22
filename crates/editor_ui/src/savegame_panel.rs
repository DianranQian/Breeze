//! 「游戏存档槽位」浏览器。
//!
//! # 只读，是刻意的
//!
//! 存档属于**玩家**。编辑器替玩家删除或覆盖存档，是越过游戏运行时职责的
//! 危险操作：游戏可能自己维护"自动存档 / 手动存档"语义、云同步、防作弊校验。
//! 因此这里只做三件事：列出槽位、显示元数据、标出损坏项。
//!
//! # 为什么读 `saves/` 而不是问游戏进程
//!
//! 存档是**游戏写、游戏读**的文件，编辑器只负责让人看见它们。
//! 通过文件系统读取的好处是：即使游戏没在运行，也能检查自己的存档。
//! 这与"运行时 ECS 检查器"是两条不同的信息来源，互不替代。

use egui::Ui;

use crate::i18n;
use crate::state::{UiAction, 界面状态};
use crate::theme::编辑器配色;

/// 绘制存档槽位对话框。
pub fn 绘制(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    语言: i18n::语言,
) -> Option<UiAction> {
    if !界面.显示存档槽位 {
        return None;
    }
    let mut 打开 = true;
    let mut 要刷新 = false;
    let 槽位 = 界面.存档槽位.clone();
    let 损坏 = 界面.存档损坏.clone();
    let 目录 = 界面.存档目录.clone();

    let 窗口响应 = egui::Window::new(语言.取("savegame_browser_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .open(&mut 打开)
        .show(egui上下文, |ui: &mut Ui| {
            match 目录.as_deref() {
                Some(目录) => {
                    ui.label(
                        egui::RichText::new(目录)
                            .small()
                            .monospace()
                            .color(配色.次要文字),
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new(语言.取("savegame_no_project"))
                            .small()
                            .color(配色.警告),
                    );
                }
            }
            ui.label(
                egui::RichText::new(语言.取("savegame_readonly_hint"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.separator();

            if 槽位.is_empty() {
                ui.label(
                    egui::RichText::new(语言.取("savegame_empty"))
                        .small()
                        .color(配色.次要文字),
                );
            }
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for 项 in &槽位 {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(&项.slot).monospace().color(配色.文字));
                            let 预览 = if 项.preview.trim().is_empty() {
                                语言.取("savegame_no_preview")
                            } else {
                                项.preview.as_str()
                            };
                            ui.label(egui::RichText::new(预览).small().color(配色.次要文字));
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} · {} · {}s",
                                    语言.填充(
                                        "savegame_meta",
                                        &[&项.format_version, &项.game_version]
                                    ),
                                    语言.填充("savegame_saved_at", &[&项.saved_at_unix]),
                                    项.play_time_seconds.round() as i64
                                ))
                                .small()
                                .monospace()
                                .color(配色.次要文字),
                            );
                        });
                    }
                });

            if !损坏.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(语言.填充("savegame_corrupt_count", &[&损坏.len()]))
                        .small()
                        .color(配色.错误),
                );
                for (槽位名, 原因) in &损坏 {
                    ui.label(
                        egui::RichText::new(format!("{槽位名}：{原因}"))
                            .small()
                            .monospace()
                            .color(配色.错误),
                    );
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(语言.取("refresh")).clicked() {
                    要刷新 = true;
                }
            });
        });

    crate::记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_savegame",
        窗口响应.map(|响应| 响应.response.rect),
    );

    if 要刷新 {
        return Some(UiAction::刷新存档槽位);
    }
    if !打开 {
        界面.显示存档槽位 = false;
    }
    None
}

/// 扫描一个存档目录，返回 `(槽位元数据, 损坏项)`。
///
/// 动作层调用它，界面层调用它，两边因此不会对"什么算损坏"产生分歧。
pub fn 扫描(目录: &std::path::Path) -> (Vec<savegame::存档元数据>, Vec<(String, String)>) {
    savegame::存档仓库::新建(目录).列表()
}
