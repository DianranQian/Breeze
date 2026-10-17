//! 顶部工作区栏：切换、新建、复制、重命名、删除工作区。
//!
//! # 为什么工作区切换要占一整条栏
//!
//! Blender 的工作区标签之所以有效，是因为它**一直在视野里**：
//! 用户想从建模切到调试，不需要打开菜单两级。Godot 的 2D/3D/Script
//! 切换也是同一思路。把它藏进菜单或配置文件，等于强迫用户记住
//! "原来还有别的布局"，实际使用中几乎不会被发现。
//!
//! 因此这里放一整条 26px 的栏：左边是工作区标签（当前项高亮），
//! 右边是固定宽度的操作组（新建/复制/重命名/删除/重置/更多）。
//! 动作组先占位，标签在剩余空间里排列 —— 这样窗口变窄时**丢掉的是
//! 标签**，而操作按钮永远可点，不会出现"窗口一小就再也切不了工作区"。
//!
//! # 与主题的一致性
//!
//! 所有颜色都取自 [`crate::theme::编辑器配色`] 的语义槽位，不写字面色值；
//! 因此深色/浅色/高对比度主题下这一条栏都会自动跟着变。

use egui::{Button, Color32, CornerRadius, RichText, Ui};

use crate::i18n::语言;
use crate::state::{UiAction, 工作区对话框, 工作区对话框模式, 绘制上下文};
use crate::theme::编辑器配色;

/// 工作区栏高度（逻辑像素）。
pub const 工作区栏高度: f32 = 26.0;

/// 标签名称的最大显示字符数。
///
/// 长名称是真实存在的（用户会起"角色动画 与 UI 布局"这种名字）。
/// 不截断的话一个标签就能把整条栏顶开，动作组被挤到屏幕外。
const 名称最大字符数: usize = 14;

/// 截断过长的名称，并给出是否发生了截断。
fn 截断名称(名称: &str) -> (String, bool) {
    let 字符数 = 名称.chars().count();
    if 字符数 <= 名称最大字符数 {
        return (名称.to_string(), false);
    }
    let 保留: String = 名称.chars().take(名称最大字符数 - 1).collect();
    (format!("{保留}…"), true)
}

/// 绘制工作区栏。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;

    // 先把需要的数据拷出来：下面要可变借用 `上下文.界面`（打开对话框）。
    let 当前id = 上下文.界面.工作区.当前id().to_string();
    let 条目: Vec<(String, String, bool)> = 上下文
        .界面
        .工作区
        .列表()
        .iter()
        .map(|项| (项.id.clone(), 项.显示名(语言), 项.内置))
        .collect();
    let 当前是内置 = 上下文.界面.工作区.当前().内置;

    let mut 待推: Vec<UiAction> = Vec::new();
    let mut 新对话框: Option<工作区对话框> = None;

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
        ui.add_space(4.0);

        // ---- 右侧动作组：先占位，保证窗口变窄时不会被标签挤出去 ----
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // 「更多」菜单：低频但重要的操作。
            ui.menu_button("⋯", |ui| {
                if ui.button(语言.取("restore_default_layout")).clicked() {
                    待推.push(UiAction::重置当前工作区);
                    ui.close();
                }
                if ui.button(语言.取("workspace_reset_all")).clicked() {
                    待推.push(UiAction::重置全部内置工作区);
                    ui.close();
                }
            })
            .response
            .on_hover_text(语言.取("workspace_more"));

            ui.add_space(2.0);

            // 重置当前工作区布局。
            let 重置 = ui
                .add_enabled(
                    true,
                    Button::new("⟲")
                        .frame(false)
                        .min_size(egui::vec2(22.0, 20.0)),
                )
                .on_hover_text(语言.取("workspace_reset_current"));
            if 重置.clicked() {
                待推.push(UiAction::重置当前工作区);
            }

            // 删除：仅用户工作区可用。
            let 删除 = ui
                .add_enabled(
                    !当前是内置,
                    Button::new(RichText::new("✖").color(配色.错误))
                        .frame(false)
                        .min_size(egui::vec2(22.0, 20.0)),
                )
                .on_hover_text(if 当前是内置 {
                    语言.取("workspace_builtin_not_deletable")
                } else {
                    语言.取("workspace_delete")
                });
            if 删除.clicked() {
                待推.push(UiAction::删除工作区(当前id.clone()));
            }

            // 重命名：仅用户工作区可用。
            let 重命名 = ui
                .add_enabled(
                    !当前是内置,
                    Button::new("✎")
                        .frame(false)
                        .min_size(egui::vec2(22.0, 20.0)),
                )
                .on_hover_text(if 当前是内置 {
                    语言.取("workspace_builtin_not_renamable")
                } else {
                    语言.取("workspace_rename")
                });
            if 重命名.clicked() {
                let 当前名称 = 上下文.界面.工作区.当前().显示名(语言);
                新对话框 = Some(工作区对话框::重命名(当前id.clone(), 当前名称));
            }

            // 复制当前工作区。
            let 复制 = ui
                .add_enabled(
                    true,
                    Button::new("⧉")
                        .frame(false)
                        .min_size(egui::vec2(22.0, 20.0)),
                )
                .on_hover_text(语言.取("workspace_duplicate"));
            if 复制.clicked() {
                let 建议 = 上下文.界面.工作区.当前().显示名(语言);
                新对话框 = Some(工作区对话框::新建(
                    true,
                    语言.填充("workspace_copy_name", &[&建议]),
                ));
            }

            // 新建工作区。
            let 新建 = ui
                .add_enabled(
                    true,
                    Button::new("＋")
                        .frame(false)
                        .min_size(egui::vec2(22.0, 20.0)),
                )
                .on_hover_text(语言.取("workspace_new"));
            if 新建.clicked() {
                新对话框 = Some(工作区对话框::新建(
                    false,
                    语言.填充("workspace_new_name", &[&(条目.len() + 1)]),
                ));
            }

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // ---- 左侧：工作区标签 ----
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                ui.add_space(2.0);
                for (id, 名称, 内置) in &条目 {
                    let 激活 = *id == 当前id;
                    let (显示, 截断) = 截断名称(名称);
                    let 文本颜色 = if 激活 {
                        配色.文字
                    } else {
                        配色.次要文字
                    };
                    let mut 文本 = RichText::new(显示).color(文本颜色).size(12.0);
                    if 激活 {
                        文本 = 文本.strong();
                    }
                    let 按钮 = Button::new(文本)
                        .frame(false)
                        .corner_radius(CornerRadius::same(3))
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
                        .min_size(egui::vec2(0.0, 20.0));
                    let 响应 = ui.add(按钮);
                    // 悬停提示始终给出完整名称：长名称在标签里被截断，
                    // 但用户永远能通过悬停看到全名（以及"内置"标记）。
                    let 提示 = if *内置 {
                        语言.填充("workspace_builtin_hint", &[名称])
                    } else {
                        名称.clone()
                    };
                    let 响应 = 响应.on_hover_text(提示);
                    let _ = 截断;
                    if 响应.clicked() && !激活 {
                        待推.push(UiAction::切换工作区(id.clone()));
                    }
                    // 右键菜单：用户工作区可直接重命名/删除（内置项禁用）。
                    响应.context_menu(|ui| {
                        if ui.button(语言.取("workspace_duplicate")).clicked() {
                            新对话框 = Some(工作区对话框::新建(
                                true,
                                语言.填充("workspace_copy_name", &[名称]),
                            ));
                            ui.close();
                        }
                        if ui
                            .add_enabled(!*内置, Button::new(语言.取("workspace_rename")))
                            .clicked()
                        {
                            新对话框 =
                                Some(工作区对话框::重命名(id.clone(), 名称.clone()));
                            ui.close();
                        }
                        if ui
                            .add_enabled(!*内置, Button::new(语言.取("workspace_delete")))
                            .clicked()
                        {
                            待推.push(UiAction::删除工作区(id.clone()));
                            ui.close();
                        }
                    });
                }
            });
        });
    });

    if let Some(对话框) = 新对话框 {
        上下文.界面.工作区对话框 = Some(对话框);
    }
    for 动作 in 待推 {
        上下文.动作.推(动作);
    }
}

/// 绘制名称对话框（新建/复制/重命名共用）。
///
/// 返回需要排进动作队列的动作。确认后对话框**保持打开**，直到动作执行器
/// 成功或报错：成功时执行器清空它，失败时把错误回填到 `错误`，
/// 用户不用重新输入一遍。
pub fn 绘制对话框(
    egui上下文: &egui::Context,
    界面: &mut crate::state::界面状态,
    配色: 编辑器配色,
    语言: 语言,
) -> Option<UiAction> {
    let mut 对话框 = 界面.工作区对话框.take()?;

    let 标题 = match 对话框.模式 {
        工作区对话框模式::新建 { 复制当前: true } => 语言.取("workspace_duplicate"),
        工作区对话框模式::新建 { .. } => 语言.取("workspace_new"),
        工作区对话框模式::重命名 { .. } => 语言.取("workspace_rename"),
    };

    let mut 打开 = true;
    let mut 确认 = false;
    // 「取消」用一个独立标志：`.open(&mut 打开)` 在整个 `show` 调用期间
    // 独占借用 `打开`，闭包里再写它会被借用检查器拒绝。
    let mut 取消 = false;
    egui::Window::new(标题)
        .id(egui::Id::new("bevy_studio_工作区对话框"))
        .collapsible(false)
        .resizable(false)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            ui.label(语言.取("workspace_name"));
            let 响应 = ui.add(
                egui::TextEdit::singleline(&mut 对话框.文本)
                    .desired_width(240.0)
                    .hint_text(语言.取("workspace_name_hint")),
            );
            响应.request_focus();
            if 响应.lost_focus() && ui.input(|输入| 输入.key_pressed(egui::Key::Enter)) {
                确认 = true;
            }
            if let Some(错误) = 对话框.错误.as_deref() {
                ui.label(RichText::new(错误).small().color(配色.错误));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(语言.取("workspace_ok")).clicked() {
                    确认 = true;
                }
                if ui.button(语言.取("cancel")).clicked() {
                    取消 = true;
                }
            });
        });
    if 取消 {
        打开 = false;
    }

    let 动作 = if 确认 {
        let 名称 = 对话框.文本.trim().to_string();
        match &对话框.模式 {
            工作区对话框模式::新建 { 复制当前 } => Some(UiAction::新建工作区 {
                名称,
                复制当前: *复制当前,
            }),
            工作区对话框模式::重命名 { id } => Some(UiAction::重命名工作区 {
                id: id.clone(),
                新名称: 名称,
            }),
        }
    } else {
        None
    };

    if 打开 {
        界面.工作区对话框 = Some(对话框);
    }
    动作
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 长名称会被截断并保留提示() {
        let (显示, 截断) = 截断名称("角色动画 与 UI 布局 的完整工作区");
        assert!(截断, "超长名称必须被截断");
        assert!(显示.ends_with('…'));
        assert_eq!(显示.chars().count(), 名称最大字符数);
    }

    #[test]
    fn 短名称原样显示() {
        let (显示, 截断) = 截断名称("通用");
        assert!(!截断);
        assert_eq!(显示, "通用");
    }

    #[test]
    fn 截断不会在字符中间切断汉字() {
        // 逐字符处理而不是按字节切片：按字节切会在汉字中间断开并 panic。
        let 名称 = "一二三四五六七八九十一二三四五六七八";
        let (显示, _) = 截断名称(名称);
        assert!(显示.chars().count() <= 名称最大字符数);
        assert!(显示.is_char_boundary(显示.len()));
    }
}
