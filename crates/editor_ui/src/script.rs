//! 脚本编辑：代码文件、行号、Rust 语法高亮、查找与保存。
//!
//! # 为什么自己写而不是引入代码编辑器组件
//!
//! 评估过 `egui_code_edit` / `egui_extras` / `syntect` 这几条路：
//!
//! | 方案 | 代价 |
//! | --- | --- |
//! | `syntect` + `egui` 自定义 layouter | 多一棵依赖树（onig 可选）+ 主题映射层 |
//! | 第三方 egui 代码编辑器 crate | 版本与 egui 0.36 不一定对齐，风险不可控 |
//! | **egui `TextEdit::layouter` + 自制词法** | 零新依赖，约 200 行，够用 |
//!
//! 编辑器只需要"看得懂的 Rust 着色"：关键字、字符串、注释、数字、类型名。
//! 真正的语义分析、补全、LSP 属于后续能力，本模块**不声称**具备。
//!
//! # 为什么高亮是"每行"而不是"整篇"
//!
//! `TextEdit` 的 layouter 会按可见区域逐行调用。按行做词法在跨行状态下
//! 会不准（块注释、多行字符串），因此这里显式只支持**行内**语法元素，
//! 并接受块注释/多行字符串会被当成普通文本 —— 这是有意的取舍：
//! 用 200 行换来零依赖与稳定对齐，而不是半吊子 LSP。

use std::ops::Range;

use egui::{Color32, FontId, RichText, TextFormat, TextStyle, Ui};

use crate::i18n::语言;
use crate::icons::图标;
use crate::state::{UiAction, 绘制上下文};
use crate::theme::编辑器配色;

/// 支持的脚本语言（按扩展名判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 脚本语言 {
    /// Rust。
    Rust,
    /// 未知类型：只显示行号，不做着色。
    纯文本,
}

impl 脚本语言 {
    /// 按文件扩展名判断。
    pub fn 按扩展名(扩展名: &str) -> Self {
        match 扩展名.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            _ => Self::纯文本,
        }
    }

    /// 是否支持语法着色。
    pub fn 支持着色(self) -> bool {
        matches!(self, Self::Rust)
    }
}

/// 一段着色词元的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 词元角色 {
    /// 普通文本。
    普通,
    /// 关键字。
    关键字,
    /// 字符串/字符字面量。
    字符串,
    /// 注释。
    注释,
    /// 数字。
    数字,
    /// 类型名（大写开头的标识符）。
    类型,
    /// 宏（`foo!`）。
    宏,
}

/// 一行里的一段词元：字节范围 + 角色。
pub type 词元 = (Range<usize>, 词元角色);

const RUST关键字: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

/// 把一行 Rust 代码切成词元。
///
/// # 覆盖范围（如实说明）
///
/// * `//` 行注释（含 `///` 文档注释）；
/// * `"…"` / `'…'` 字面量（含简单转义）；
/// * 数字（含小数与下划线）；
/// * 关键字、大写开头的类型名、`ident!` 形式的宏；
/// * **不处理**块注释 `/* */` 的跨行状态与多行字符串：它们会被当作普通文本。
pub fn 切词(行: &str) -> Vec<词元> {
    let 字节 = 行.as_bytes();
    let 长度 = 字节.len();
    let mut 结果: Vec<词元> = Vec::new();
    let mut 起 = 0usize;
    let mut i = 0usize;

    let 收尾 = |结果: &mut Vec<词元>, 起: usize, 止: usize, 角色: 词元角色| {
        if 止 > 起 {
            结果.push((起..止, 角色));
        }
    };

    while i < 长度 {
        let 字节值 = 字节[i];

        // 行注释：到行尾全部是注释。
        if 字节值 == b'/' && i + 1 < 长度 && 字节[i + 1] == b'/' {
            收尾(&mut 结果, 起, i, 词元角色::普通);
            结果.push((i..长度, 词元角色::注释));
            return 结果;
        }

        // 字符串 / 字符字面量。
        if 字节值 == b'"' || (字节值 == b'\'' && i + 1 < 长度 && !是标识符起(字节[i + 1]))
        {
            let 引号 = 字节值;
            收尾(&mut 结果, 起, i, 词元角色::普通);
            let 始 = i;
            i += 1;
            while i < 长度 {
                if 字节[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if 字节[i] == 引号 {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let 止 = i.min(长度);
            结果.push((始..止, 词元角色::字符串));
            起 = 止;
            continue;
        }

        // 数字。
        if 字节值.is_ascii_digit() {
            收尾(&mut 结果, 起, i, 词元角色::普通);
            let 始 = i;
            while i < 长度
                && (字节[i].is_ascii_alphanumeric() || 字节[i] == b'.' || 字节[i] == b'_')
            {
                // 避免把 `1..2` 的范围运算符吃掉。
                if 字节[i] == b'.' && i + 1 < 长度 && 字节[i + 1] == b'.' {
                    break;
                }
                i += 1;
            }
            结果.push((始..i, 词元角色::数字));
            起 = i;
            continue;
        }

        // 标识符。
        if 是标识符起(字节值) {
            let 始 = i;
            while i < 长度 && 是标识符续(字节[i]) {
                i += 1;
            }
            let 词 = &行[始..i];
            let 角色 = if RUST关键字.contains(&词) {
                词元角色::关键字
            } else if i < 长度 && 字节[i] == b'!' {
                词元角色::宏
            } else if 词.chars().next().is_some_and(|字| 字.is_uppercase()) {
                词元角色::类型
            } else {
                词元角色::普通
            };
            收尾(&mut 结果, 起, 始, 词元角色::普通);
            结果.push((始..i, 角色));
            起 = i;
            continue;
        }

        i += 1;
    }
    收尾(&mut 结果, 起, 长度, 词元角色::普通);
    结果
}

fn 是标识符起(字节: u8) -> bool {
    字节.is_ascii_alphabetic() || 字节 == b'_' || 字节 >= 0x80
}

fn 是标识符续(字节: u8) -> bool {
    u8::is_ascii_alphanumeric(&字节) || 字节 == b'_' || 字节 >= 0x80
}

/// 角色 → 颜色（全部来自主题语义槽位）。
fn 角色颜色(角色: 词元角色, 配色: &编辑器配色) -> Color32 {
    match 角色 {
        词元角色::普通 => 配色.文字,
        词元角色::关键字 => 配色.强调,
        词元角色::字符串 => 配色.成功,
        词元角色::注释 => 配色.次要文字,
        词元角色::数字 => 配色.信息,
        词元角色::类型 => 配色.链接,
        词元角色::宏 => 配色.警告,
    }
}

/// 当前打开的脚本文档。
#[derive(Debug, Clone, PartialEq)]
pub struct 脚本文档 {
    /// 项目相对路径。`None` 表示尚未保存过。
    pub 路径: Option<scene_doc::path::项目相对路径>,
    /// 编辑中的文本。
    pub 文本: String,
    /// 最后一次从磁盘读到/写入的文本，用于判断未保存修改。
    pub 已保存文本: String,
    /// 语言。
    pub 语言: 脚本语言,
}

impl 脚本文档 {
    /// 从磁盘内容创建。
    pub fn 新建(路径: Option<scene_doc::path::项目相对路径>, 文本: String) -> Self {
        let 语言 = 路径
            .as_ref()
            .and_then(|路径| 路径.扩展名())
            .map(|扩展| 脚本语言::按扩展名(&扩展))
            .unwrap_or(脚本语言::纯文本);
        Self {
            路径,
            已保存文本: 文本.clone(),
            文本,
            语言,
        }
    }

    /// 是否有未保存修改。
    pub fn 有未保存修改(&self) -> bool {
        self.文本 != self.已保存文本
    }

    /// 显示名（文件名或"未命名"）。
    pub fn 显示名(&self, 语言: 语言) -> String {
        self.路径
            .as_ref()
            .map(|路径| 路径.文件名().to_string())
            .unwrap_or_else(|| 语言.取("script_untitled").to_string())
    }

    /// 行数。
    pub fn 行数(&self) -> usize {
        self.文本.lines().count().max(1)
    }
}

/// 脚本编辑器的界面状态。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct 脚本状态 {
    /// 当前文档。
    pub 文档: Option<脚本文档>,
    /// 是否展开查找栏。
    pub 显示查找: bool,
    /// 查找关键字。
    pub 查找: String,
    /// 查找命中的行号（0 基）。
    pub 命中行: Vec<usize>,
    /// 当前命中序号。
    pub 当前命中: usize,
    /// 一次性提示（例如"已保存"）。
    pub 提示: Option<String>,
    /// 请求滚动到某一行（0 基）。
    pub 滚动到行: Option<usize>,
}

impl 脚本状态 {
    /// 打开一份文档。
    pub fn 打开(&mut self, 文档: 脚本文档) {
        self.文档 = Some(文档);
        self.命中行.clear();
        self.当前命中 = 0;
        self.滚动到行 = Some(0);
    }

    /// 标记已保存。
    pub fn 标记已保存(&mut self, 路径: Option<scene_doc::path::项目相对路径>) {
        if let Some(文档) = self.文档.as_mut() {
            if let Some(新路径) = 路径 {
                文档.路径 = Some(新路径);
            }
            文档.已保存文本 = 文档.文本.clone();
        }
    }

    /// 重新计算查找命中。
    pub fn 重新查找(&mut self) {
        self.命中行.clear();
        self.当前命中 = 0;
        if self.查找.is_empty() {
            return;
        }
        let Some(文档) = self.文档.as_ref() else {
            return;
        };
        let 关键字 = self.查找.to_lowercase();
        for (下标, 行) in 文档.文本.lines().enumerate() {
            if 行.to_lowercase().contains(&关键字) {
                self.命中行.push(下标);
            }
        }
        if let Some(第一) = self.命中行.first() {
            self.滚动到行 = Some(*第一);
        }
    }

    /// 跳到下一个命中。
    pub fn 下一个命中(&mut self) {
        if self.命中行.is_empty() {
            return;
        }
        self.当前命中 = (self.当前命中 + 1) % self.命中行.len();
        self.滚动到行 = Some(self.命中行[self.当前命中]);
    }

    /// 跳到上一个命中。
    pub fn 上一个命中(&mut self) {
        if self.命中行.is_empty() {
            return;
        }
        self.当前命中 = (self.当前命中 + self.命中行.len() - 1) % self.命中行.len();
        self.滚动到行 = Some(self.命中行[self.当前命中]);
    }
}

/// 绘制脚本面板。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;

    // ---- 顶部工具栏 ----
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let 有文档 = 上下文.界面.脚本.文档.is_some();
        let 有修改 = 上下文
            .界面
            .脚本
            .文档
            .as_ref()
            .is_some_and(|文档| 文档.有未保存修改());

        if 上下文
            .图标
            .图标按钮(
                ui,
                图标::保存,
                Some(语言.取("script_save")),
                16.0,
                配色.文字,
                有文档,
            )
            .on_hover_text(语言.取("script_save_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::保存脚本);
        }
        if 上下文
            .图标
            .图标按钮(
                ui,
                图标::复制,
                Some(语言.取("script_save_as")),
                16.0,
                配色.文字,
                有文档,
            )
            .clicked()
        {
            上下文.动作.推(UiAction::脚本另存为);
        }
        if 上下文
            .图标
            .图标按钮(
                ui,
                图标::放大,
                Some(语言.取("script_find")),
                16.0,
                配色.文字,
                有文档,
            )
            .clicked()
        {
            上下文.界面.脚本.显示查找 = !上下文.界面.脚本.显示查找;
        }

        ui.separator();
        match 上下文.界面.脚本.文档.as_ref() {
            Some(文档) => {
                let 名 = 文档.显示名(语言);
                let 文本 = if 文档.有未保存修改() {
                    format!("{名} •")
                } else {
                    名
                };
                ui.label(
                    RichText::new(文本)
                        .color(if 文档.有未保存修改() {
                            配色.警告
                        } else {
                            配色.文字
                        })
                        .small(),
                );
                ui.label(
                    RichText::new(语言.填充("script_line_count", &[&文档.行数()]))
                        .small()
                        .color(配色.次要文字),
                );
            }
            None => {
                ui.label(
                    RichText::new(语言.取("script_no_file"))
                        .small()
                        .color(配色.次要文字),
                );
            }
        }
        if 有修改 {
            ui.label(
                RichText::new(语言.取("script_unsaved"))
                    .small()
                    .color(配色.警告),
            );
        }
    });

    // ---- 查找栏 ----
    if 上下文.界面.脚本.显示查找 {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(语言.取("script_find"))
                    .small()
                    .color(配色.次要文字),
            );
            let 响应 = ui.add(
                egui::TextEdit::singleline(&mut 上下文.界面.脚本.查找)
                    .desired_width(180.0)
                    .hint_text(语言.取("script_find_hint")),
            );
            if 响应.changed() {
                上下文.界面.脚本.重新查找();
            }
            if ui.small_button("◀").clicked() {
                上下文.界面.脚本.上一个命中();
            }
            if ui.small_button("▶").clicked() {
                上下文.界面.脚本.下一个命中();
            }
            let 命中数 = 上下文.界面.脚本.命中行.len();
            ui.label(
                RichText::new(if 命中数 == 0 {
                    语言.取("script_no_match").to_string()
                } else {
                    语言.填充("script_match_count", &[&命中数])
                })
                .small()
                .color(配色.次要文字),
            );
        });
    }

    ui.separator();

    // ---- 编辑区 ----
    let Some(文档) = 上下文.界面.脚本.文档.as_mut() else {
        ui.add_space(12.0);
        ui.label(RichText::new(语言.取("script_empty_hint")).color(配色.次要文字));
        ui.add_space(6.0);
        ui.label(
            RichText::new(语言.取("script_empty_hint_2"))
                .small()
                .color(配色.次要文字),
        );
        return;
    };

    let 语言类型 = 文档.语言;
    let 高亮配色 = 配色;
    let 字号 = 上下文.外观.基础字号;
    let 查找词 = 上下文.界面.脚本.查找.clone();
    let mut 文本 = 文档.文本.clone();

    let mut 布局任务 = |ui: &Ui, 缓冲: &dyn egui::TextBuffer, _宽度: f32| {
        let 字体 = FontId::monospace(字号);
        let mut 作业 = egui::text::LayoutJob::default();
        let 文本 = 缓冲.as_str();
        for 行 in 文本.split('\n') {
            let 词元 = if 语言类型.支持着色() {
                切词(行)
            } else {
                vec![(0..行.len(), 词元角色::普通)]
            };
            for (范围, 角色) in 词元 {
                let 片段 = &行[范围.clone()];
                if 片段.is_empty() {
                    continue;
                }
                let 强调 =
                    !查找词.is_empty() && 片段.to_lowercase().contains(&查找词.to_lowercase());
                作业.append(
                    片段,
                    0.0,
                    TextFormat {
                        font_id: 字体.clone(),
                        color: 角色颜色(角色, &高亮配色),
                        background: if 强调 {
                            高亮配色.选中背景
                        } else {
                            Color32::TRANSPARENT
                        },
                        ..Default::default()
                    },
                );
            }
            作业.append("\n", 0.0, TextFormat::simple(字体.clone(), 高亮配色.文字));
        }
        ui.painter().layout_job(作业)
    };

    // 行号栏 + 编辑区并排。
    let 行数 = 文本.lines().count().max(1);
    ui.horizontal_top(|ui| {
        // 行号：用不可交互的等宽文本列。
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for 行 in 1..=行数 {
                ui.label(
                    RichText::new(format!("{行:>4}"))
                        .monospace()
                        .size(字号)
                        .color(配色.禁用文字),
                );
            }
        });
        ui.separator();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let 编辑器 = egui::TextEdit::multiline(&mut 文本)
                    .font(TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(30)
                    .code_editor()
                    .layouter(&mut 布局任务);
                ui.add(编辑器);
            });
    });

    if 文本 != 文档.文本 {
        文档.文本 = 文本;
        if !查找词.is_empty() {
            上下文.界面.脚本.重新查找();
        }
    }

    // 滚动请求：把命中行带进视野（简化实现：滚到顶部附近）。
    if let Some(行) = 上下文.界面.脚本.滚动到行.take() {
        let _ = 行;
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 角色们(行: &str) -> Vec<词元角色> {
        切词(行).into_iter().map(|(_, 角色)| 角色).collect()
    }

    #[test]
    fn 关键字与标识符被区分() {
        let 行 = "let mut 值 = 1;";
        let 词元 = 切词(行);
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| &行[范围.clone()] == "let" && *角色 == 词元角色::关键字)
        );
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| &行[范围.clone()] == "mut" && *角色 == 词元角色::关键字)
        );
    }

    #[test]
    fn 字符串与注释不会被当成关键字() {
        let 行 = r#"let s = "let fn"; // let fn"#;
        let 词元 = 切词(行);
        // 字面量整体是字符串角色
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| &行[范围.clone()] == r#""let fn""#
                    && *角色 == 词元角色::字符串)
        );
        // 行注释整体是注释角色，并且到行尾
        let (注释范围, 注释角色) = 词元
            .iter()
            .find(|(_, 角色)| *角色 == 词元角色::注释)
            .expect("应当有注释词元");
        assert_eq!(*注释角色, 词元角色::注释);
        assert!(注释范围.end == 行.len());
    }

    #[test]
    fn 数字与范围运算符区分() {
        let 行 = "for i in 0..10 {";
        let 词元 = 切词(行);
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| &行[范围.clone()] == "0" && *角色 == 词元角色::数字)
        );
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| &行[范围.clone()] == "10" && *角色 == 词元角色::数字)
        );
    }

    #[test]
    fn 类型名与宏有独立角色() {
        let 行 = "let v: Vec<String> = vec![];";
        let 词元 = 切词(行);
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| &行[范围.clone()] == "Vec" && *角色 == 词元角色::类型)
        );
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| &行[范围.clone()] == "vec" && *角色 == 词元角色::宏)
        );
    }

    #[test]
    fn 空行不产生词元也不崩溃() {
        assert!(切词("").is_empty());
        assert_eq!(角色们("   "), vec![词元角色::普通]);
    }

    #[test]
    fn 未闭合字符串不会越界() {
        // 行内未闭合的字符串应当一路吃到行尾，而不是 panic。
        let 词元 = 切词(r#"let s = "abc"#);
        assert!(词元.iter().any(|(_, 角色)| *角色 == 词元角色::字符串));
    }

    #[test]
    fn 脚本文档的未保存判定() {
        let mut 文档 = 脚本文档::新建(None, "fn main() {}".to_string());
        assert!(!文档.有未保存修改());
        文档.文本.push_str("\n// 改了一行");
        assert!(文档.有未保存修改());
        文档.已保存文本 = 文档.文本.clone();
        assert!(!文档.有未保存修改());
    }

    #[test]
    fn 查找与命中跳转() {
        let mut 状态 = 脚本状态::default();
        状态.打开(脚本文档::新建(
            None,
            "alpha\nbeta\nAlpha again\n".to_string(),
        ));
        状态.查找 = "alpha".to_string();
        状态.重新查找();
        assert_eq!(状态.命中行, vec![0, 2], "查找应当不区分大小写");
        assert_eq!(状态.滚动到行, Some(0));
        状态.下一个命中();
        assert_eq!(状态.滚动到行, Some(2));
        状态.下一个命中();
        assert_eq!(状态.滚动到行, Some(0), "命中应当循环");
        状态.上一个命中();
        assert_eq!(状态.滚动到行, Some(2));
    }

    #[test]
    fn 空查找不产生命中() {
        let mut 状态 = 脚本状态::default();
        状态.打开(脚本文档::新建(None, "abc".to_string()));
        状态.查找 = String::new();
        状态.重新查找();
        assert!(状态.命中行.is_empty());
        // 没有命中时跳转是安全的空操作。
        状态.下一个命中();
        状态.上一个命中();
    }

    #[test]
    fn 语言按扩展名识别() {
        assert_eq!(脚本语言::按扩展名("rs"), 脚本语言::Rust);
        assert_eq!(脚本语言::按扩展名("RS"), 脚本语言::Rust);
        assert_eq!(脚本语言::按扩展名("txt"), 脚本语言::纯文本);
        assert!(脚本语言::Rust.支持着色());
        assert!(!脚本语言::纯文本.支持着色());
    }
}
