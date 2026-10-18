//! 帮助窗口：快捷键、路径与诊断入口。
//!
//! # 这张表为什么必须和代码放在一起审阅
//!
//! 快捷键是"看不见的界面"：用户只能通过这份表知道 `Ctrl+D` 是复制实体。
//! 如果表里写了代码没实现的键，用户按下去没反应；如果代码实现了表里没写的键，
//! 用户永远发现不了。因此这里把表集中成一个常量，
//! 并在 `editor_app` 的处理函数旁边留了指向它的注释 ——
//! 加/删快捷键时**两处一起改**是这条链上唯一的人工约束，
//! 与其分散成十几处 `if`，不如让它显式。
//!
//! # 为什么还有一个"诊断入口"
//!
//! 用户报"点了没反应"时，第一手证据在控制台与配置目录。帮助窗口把这两个
//! 入口放在快捷键旁边，比让用户在菜单里翻「窗口 → 控制台」更快。

use crate::state::UiAction;

/// 一条快捷键说明。
pub struct 快捷键条目 {
    /// 键位组合的显示文本（如 `Ctrl+S`）。
    ///
    /// 不翻译：键名是物理按键的标识，翻译成"控制键+S"只会更难认。
    pub 组合: &'static str,
    /// 动作的词条键（显示时按当前语言查表）。
    pub 动作键: &'static str,
}

/// 全部已实现的快捷键。
///
/// 顺序即显示顺序：文件 → 编辑 → 场景 → 运行 → 视口。
pub const 快捷键表: &[快捷键条目] = &[
    快捷键条目 {
        组合: "Ctrl+S",
        动作键: "save_scene",
    },
    快捷键条目 {
        组合: "Ctrl+Z",
        动作键: "undo_2",
    },
    快捷键条目 {
        组合: "Ctrl+Y",
        动作键: "redo_2",
    },
    快捷键条目 {
        组合: "Ctrl+D",
        动作键: "duplicate_entity",
    },
    快捷键条目 {
        组合: "Ctrl+A",
        动作键: "select_all",
    },
    快捷键条目 {
        组合: "Delete",
        动作键: "delete_entity",
    },
    快捷键条目 {
        组合: "F",
        动作键: "focus_selection",
    },
    快捷键条目 {
        组合: "Esc",
        动作键: "shortcut_cancel_selection",
    },
    快捷键条目 {
        组合: "F5",
        动作键: "run_the_current_scene",
    },
    快捷键条目 {
        组合: "F6",
        动作键: "pause_resume",
    },
    快捷键条目 {
        组合: "F7",
        动作键: "stop_2",
    },
];

/// 绘制帮助窗口。
///
/// 返回 `Some(动作)` 表示用户点了某个按钮（打开配置目录 / 聚焦控制台）。
#[allow(clippy::too_many_arguments)]
pub fn 绘制帮助窗口(
    egui上下文: &egui::Context,
    界面: &mut crate::state::界面状态,
    配色: crate::theme::编辑器配色,
    语言: crate::i18n::语言,
    配置目录: &std::path::Path,
    项目根: Option<&std::path::Path>,
) -> Option<UiAction> {
    if !界面.显示帮助 {
        return None;
    }
    let mut 打开 = true;
    let mut 动作 = None;
    let 窗口响应 = egui::Window::new(语言.取("keyboard_shortcuts"))
        .collapsible(false)
        .resizable(true)
        .default_width(420.0)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            ui.label(
                egui::RichText::new(语言.取("shortcuts_intro"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.separator();
            egui::Grid::new("shortcuts_grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for 项 in 快捷键表 {
                        ui.label(egui::RichText::new(项.组合).monospace().color(配色.文字));
                        ui.label(
                            egui::RichText::new(语言.取(项.动作键))
                                .small()
                                .color(配色.文字),
                        );
                        ui.end_row();
                    }
                });

            ui.separator();
            ui.label(
                egui::RichText::new(语言.取("help_paths"))
                    .strong()
                    .color(配色.文字),
            );
            egui::Grid::new("help_paths_grid")
                .num_columns(2)
                .spacing([12.0, 3.0])
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(语言.取("help_config_dir"))
                            .small()
                            .color(配色.次要文字),
                    );
                    ui.label(
                        egui::RichText::new(配置目录.display().to_string())
                            .small()
                            .monospace()
                            .color(配色.次要文字),
                    );
                    ui.end_row();
                    ui.label(
                        egui::RichText::new(语言.取("help_project_dir"))
                            .small()
                            .color(配色.次要文字),
                    );
                    ui.label(
                        egui::RichText::new(
                            项目根
                                .map(|根| 根.display().to_string())
                                .unwrap_or_else(|| 语言.取("no_project_open").to_string()),
                        )
                        .small()
                        .monospace()
                        .color(配色.次要文字),
                    );
                    ui.end_row();
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(语言.取("open_config_dir")).clicked() {
                    动作 = Some(UiAction::打开配置目录);
                }
                if ui.button(语言.取("show_console")).clicked() {
                    动作 = Some(UiAction::聚焦面板(crate::docks::Tab::控制台));
                }
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} · {}",
                    env!("CARGO_PKG_VERSION"),
                    language_neutral_stack()
                ))
                .small()
                .color(配色.次要文字),
            );
        });

    crate::记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_help",
        窗口响应.map(|响应| 响应.response.rect),
    );
    if !打开 {
        界面.显示帮助 = false;
    }
    动作
}

/// 技术栈说明与语言无关，单独提供以免在词表里重复。
fn language_neutral_stack() -> &'static str {
    "Bevy 0.20 / egui 0.36"
}

/// 在帮助窗口里显示一条快捷键（供测试断言表格结构）。
pub fn 找到快捷键(组合: &str) -> Option<&'static 快捷键条目> {
    快捷键表.iter().find(|项| 项.组合 == 组合)
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 快捷键表非空且组合唯一() {
        assert!(!快捷键表.is_empty());
        let mut 组合: Vec<&str> = 快捷键表.iter().map(|项| 项.组合).collect();
        let 数量 = 组合.len();
        组合.sort_unstable();
        组合.dedup();
        assert_eq!(组合.len(), 数量, "快捷键组合不能重复（否则表里会自相矛盾）");
    }

    #[test]
    fn 每条快捷键的动作词条都存在() {
        for 项 in 快捷键表 {
            assert!(
                crate::语言::中文.有键(项.动作键),
                "快捷键 {} 的动作词条 `{}` 不在词表里",
                项.组合,
                项.动作键
            );
            assert!(!项.组合.trim().is_empty());
        }
    }

    #[test]
    fn 关键快捷键在表里() {
        // 这几条是用户最常用的，缺一条都算文档缺陷。
        for 组合 in ["Ctrl+S", "Ctrl+Z", "F5", "Esc"] {
            assert!(找到快捷键(组合).is_some(), "缺少快捷键 {组合}");
        }
    }
}
