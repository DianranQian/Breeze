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
//! | **egui `TextEdit::layouter` + 自制词法** | 零新依赖，约 300 行，够用 |
//!
//! 编辑器只需要"看得懂的 Rust 着色"：关键字、字符串、注释、数字、类型名、
//! 宏与属性。真正的语义分析、补全、LSP 属于后续能力，本模块**不声称**具备。
//!
//! # 跨行词法：状态顺着行往下传
//!
//! layouter 每次拿到的是**整篇**文本，因此可以在一次调用里逐行推进一个
//! [`词法状态`]：块注释（可嵌套）、原始字符串 `r#"…"#` 跨行时，下一行
//! 开头就处在同样的状态里。早期版本只做行内词法，跨行块注释会被后面
//! 的关键字重新着色 —— 用户看到的是"注释里一半是语法色"，比不着色更糟。
//!
//! 仍未覆盖的：属性内部含 `]` / `"` 的病态写法（如 `#[doc = "]"]`），
//! 以及字符字面量里的转义边界 —— 两者都不影响常见代码的着色。
//!
//! # 行号栏：painted gutter，而不是并排的一列 Label
//!
//! 早期实现把行号放在 `ScrollArea` **外面**的一列 `Label` 里，于是代码
//! 滚动时行号不动 —— 行号与代码必然错位。现在行号由同一个
//! `TextEditOutput` 的 `galley_pos` 反推：每个行号画在它对应那一行的
//! 纵坐标上，滚动多少就跟多少；横向滚动时行号槽固定在左侧不跟着跑。
//! 只画可见行，长文件也不会因为"几千个 Label"拖慢一帧。
//!
//! # 横向滚动为什么要先撑宽子 `Ui`
//!
//! `ScrollArea` 给内容区的最大宽度等于视口宽，而 `TextEdit` 会把自身宽度
//! 夹到可用宽度 —— 直接放进去永远没有可横向滚动的余量。这里按上一帧量到的
//! galley 宽度撑开一个更宽的子 `Ui`，滚动条才有东西可滚（差一帧，肉眼不可见）。

use std::ops::Range;

use egui::{Color32, FontId, RichText, TextFormat, Ui};

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
    /// 属性段（`#[…]` / `#![…]`）。
    属性,
}

/// 一行里的一段词元：字节范围 + 角色。
pub type 词元 = (Range<usize>, 词元角色);

const RUST关键字: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

/// 跨行词法状态：块注释嵌套层数与未闭合的原始字符串。
///
/// layouter 按行推进这个状态；行与行之间唯一的联系就是它，因此它必须
/// 能被廉价地复制、比较，且默认值等价于"处在普通代码里"。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct 词法状态 {
    /// 当前嵌套的块注释层数。Rust 的块注释可以嵌套，所以计数而不是布尔。
    块注释深度: usize,
    /// 未闭合原始字符串的引号后井号数（`r#"…"#` 里是 `1`）。
    原始字符串井号: Option<usize>,
}

impl 词法状态 {
    /// 行首状态：普通代码。
    pub fn 新() -> Self {
        Self::default()
    }

    /// 这一行结束时是否仍处在多行元素里（块注释或原始字符串）。
    pub fn 跨行中(&self) -> bool {
        self.块注释深度 > 0 || self.原始字符串井号.is_some()
    }
}

/// 把一行 Rust 代码切成词元，并推进跨行状态。
///
/// # 覆盖范围（如实说明）
///
/// * `//` 行注释（含 `///` 文档注释）；
/// * `/* … */` 块注释，支持嵌套，且可以跨行（状态经 [`词法状态`] 传递）；
/// * `"…"` / `'…'` 字面量与 `r"…"` / `r#"…"#` 原始字符串（可跨行）；
/// * 数字（含小数与下划线，不吃掉 `..` 范围运算符）；
/// * 关键字、大写开头的类型名、`ident!` 形式的宏；
/// * `#[…]` / `#![…]` 属性段（按方括号配对；跨行属性只着色首行，常见
///   写法都是单行）。
pub fn 切词带状态(行: &str, 状态: &mut 词法状态) -> Vec<词元> {
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
        // ---- 续接上一种跨行元素 ----
        if 状态.块注释深度 > 0 {
            if 字节[i] == b'/' && i + 1 < 长度 && 字节[i + 1] == b'*' {
                状态.块注释深度 += 1;
                i += 2;
            } else if 字节[i] == b'*' && i + 1 < 长度 && 字节[i + 1] == b'/' {
                状态.块注释深度 -= 1;
                i += 2;
                if 状态.块注释深度 == 0 {
                    收尾(&mut 结果, 起, i, 词元角色::注释);
                    起 = i;
                }
            } else {
                i += 1;
            }
            continue;
        }
        if let Some(井号数) = 状态.原始字符串井号 {
            if 字节[i] == b'"'
                && i + 1 + 井号数 <= 长度
                && 字节[i + 1..i + 1 + 井号数].iter().all(|值| *值 == b'#')
            {
                i += 1 + 井号数;
                收尾(&mut 结果, 起, i, 词元角色::字符串);
                起 = i;
                状态.原始字符串井号 = None;
            } else {
                i += 1;
            }
            continue;
        }

        // ---- 普通代码 ----
        let 字节值 = 字节[i];

        // 行注释：到行尾全部是注释。
        if 字节值 == b'/' && i + 1 < 长度 && 字节[i + 1] == b'/' {
            收尾(&mut 结果, 起, i, 词元角色::普通);
            结果.push((i..长度, 词元角色::注释));
            return 结果;
        }

        // 块注释开始：从这里起整段是注释，直到配对闭合（可以跨行）。
        if 字节值 == b'/' && i + 1 < 长度 && 字节[i + 1] == b'*' {
            收尾(&mut 结果, 起, i, 词元角色::普通);
            起 = i;
            状态.块注释深度 = 1;
            i += 2;
            continue;
        }

        // 属性：`#[…]` / `#![…]`，整段一个角色。
        if 字节值 == b'#'
            && i + 1 < 长度
            && (字节[i + 1] == b'[' || (字节[i + 1] == b'!' && i + 2 < 长度 && 字节[i + 2] == b'['))
        {
            收尾(&mut 结果, 起, i, 词元角色::普通);
            let 始 = i;
            i += if 字节[i + 1] == b'!' { 3 } else { 2 };
            let mut 深度 = 1usize;
            while i < 长度 {
                if 字节[i] == b'[' {
                    深度 += 1;
                } else if 字节[i] == b']' {
                    深度 -= 1;
                    i += 1;
                    if 深度 == 0 {
                        break;
                    }
                    continue;
                }
                i += 1;
            }
            结果.push((始..i.min(长度), 词元角色::属性));
            起 = i.min(长度);
            continue;
        }

        // 原始字符串开始：`r"…"` / `r#"…"#`（`r#foo` 是原始标识符，不算）。
        if let Some((井号数, 引号后)) = 原始字符串开头(字节, i) {
            收尾(&mut 结果, 起, i, 词元角色::普通);
            起 = i;
            状态.原始字符串井号 = Some(井号数);
            i = 引号后;
            continue;
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
    let 尾角色 = if 状态.块注释深度 > 0 {
        词元角色::注释
    } else if 状态.原始字符串井号.is_some() {
        词元角色::字符串
    } else {
        词元角色::普通
    };
    收尾(&mut 结果, 起, 长度, 尾角色);
    结果
}

/// 把单独一行切成词元（不跨行：每次从普通代码状态开始）。
///
/// 跨行元素在这里会被截断成"到行尾为止" —— 单行工具（例如查找高亮）
/// 需要的就是这个语义；整篇着色请用 [`切词带状态`] 自己推进状态。
pub fn 切词(行: &str) -> Vec<词元> {
    let mut 状态 = 词法状态::新();
    切词带状态(行, &mut 状态)
}

/// 识别原始字符串的开头，返回 `(井号数, 开头引号之后的下标)`。
///
/// 必须排除"前面紧挨着标识符字符"的情况：`bar"…"` 里的 `r` 不是前缀，
/// 真正的前缀必须处在标识符边界上。
fn 原始字符串开头(字节: &[u8], 起: usize) -> Option<(usize, usize)> {
    if 起 > 0 && 是标识符续(字节[起 - 1]) {
        return None;
    }
    if 字节.get(起) != Some(&b'r') {
        return None;
    }
    let mut i = 起 + 1;
    let mut 井号数 = 0;
    while 字节.get(i) == Some(&b'#') {
        井号数 += 1;
        i += 1;
    }
    if 字节.get(i) == Some(&b'"') {
        Some((井号数, i + 1))
    } else {
        None
    }
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
        词元角色::属性 => 配色.警告,
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
    /// 最近一次编辑光标所在行（1 起）。没有编辑过时为 `None`。
    ///
    /// AI 上下文用它告诉 Agent"用户正看着哪一行"，而不是猜。
    pub 光标行: Option<usize>,
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
            光标行: None,
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

/// 一个打开的代码标签页。
///
/// # 为什么查找与滚动挂在标签上而不是面板上
///
/// 同一份代码可能同时看着两处：用户在 A 里查找 "spawn" 跳到第 80 行，
/// 切到 B 看了几眼再切回来 —— 如果这些状态是面板级的，回来时命中华、
/// 命中序号与滚动位置全都回到了 A 的顶部，等于"刚才的查找白做了"。
/// 因此每个标签自带一份查找词、命中列表、当前命中与纵向滚动偏移，
/// 切换标签只是换一份状态，不是重置。
#[derive(Debug, Clone, PartialEq)]
pub struct 脚本标签 {
    /// 文档内容与保存基线。
    pub 文档: 脚本文档,
    /// 查找关键字。
    pub 查找: String,
    /// 查找命中的行号（0 基）。
    pub 命中行: Vec<usize>,
    /// 当前命中序号。
    pub 当前命中: usize,
    /// 上一次绘制时记下的纵向滚动偏移。
    pub 滚动偏移: f32,
    /// 切回本标签时要一次性恢复的纵向偏移（`None` 表示沿用滚动区自身状态）。
    pub 待恢复滚动: Option<f32>,
    /// 请求滚动到某一行（0 基）。
    pub 滚动到行: Option<usize>,
}

impl 脚本标签 {
    /// 从文档创建标签。
    pub fn 新建(文档: 脚本文档) -> Self {
        Self {
            文档,
            查找: String::new(),
            命中行: Vec::new(),
            当前命中: 0,
            滚动偏移: 0.0,
            待恢复滚动: Some(0.0),
            滚动到行: None,
        }
    }

    /// 重新计算查找命中。
    pub fn 重新查找(&mut self) {
        self.命中行.clear();
        self.当前命中 = 0;
        if self.查找.is_empty() {
            return;
        }
        let 关键字 = self.查找.to_lowercase();
        for (下标, 行) in self.文档.文本.lines().enumerate() {
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

/// 需要用户确认的脚本操作。
///
/// # 为什么"切换标签"不确认，而"打开另一个文件"要确认
///
/// 标签之间的切换**不丢任何东西**：每个标签各自持有一份缓冲区，切过去
/// 再切回来，编辑内容原样还在。为这种无损操作弹窗只会训练用户无脑点
/// "放弃"。真正会让人丢失劳动的是"离开当前文件"：打开另一个文件要把
/// 焦点让出去，关闭标签要把缓冲区销毁 —— 这两种才需要一次确认。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 脚本待确认 {
    /// 关闭一个带未保存修改的标签。
    关闭 {
        /// 标签下标。
        索引: usize,
    },
    /// 打开另一个文件，但当前活动标签有未保存修改。
    打开 {
        /// 要打开的项目相对路径。
        路径: scene_doc::path::项目相对路径,
    },
}

/// 脚本编辑器的界面状态。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct 脚本状态 {
    /// 已打开的标签（按打开顺序）。
    pub 标签: Vec<脚本标签>,
    /// 活动标签下标。没有标签时无意义（恒为 0）。
    pub 活动: usize,
    /// 是否展开查找栏。
    pub 显示查找: bool,
    /// 一次性提示（例如"已保存"）。
    pub 提示: Option<String>,
    /// 正在后台读取的路径。
    ///
    /// 由 [`脚本状态::请求打开`] 置位、由后台回执清空。它存在的意义只是
    /// "让界面能显示正在读取" —— 读盘本身不在主线程上，见 `code_loader`。
    pub 加载中: Option<scene_doc::path::项目相对路径>,
    /// 已声明但尚未交给后台线程的读取意图。
    pub 待读取: Option<scene_doc::path::项目相对路径>,
    /// 读取代次：每次请求递增，界面只接受与当前代次相同的回执。
    pub 加载代次: u64,
    /// 需要用户确认的关闭 / 打开。
    pub 待确认: Option<脚本待确认>,
    /// "另存为"完成后应当标记已保存的标签下标。
    ///
    /// 对话框是异步的，回执回来时用户可能已经切到别的标签；没有这个字段，
    /// 保存成功会清掉**别人**的星号。
    pub 另存目标: Option<usize>,
    /// 最近一次绘制时代码编辑框的 egui 元素 id。
    ///
    /// Ctrl+S 据此判断"焦点是否在代码面板里"：egui 的焦点是**元素级**状态，
    /// 面板自己维护一个布尔量会在面板没有绘制的那一帧残留旧值（例如用户
    /// 切到别的停靠标签后再按 Ctrl+S，会去保存一个根本没在看的文件）。
    pub 代码焦点: Option<egui::Id>,
}

impl 脚本状态 {
    /// 路径对应的标签下标。
    pub fn 标签索引(&self, 路径: &scene_doc::path::项目相对路径) -> Option<usize> {
        self.标签
            .iter()
            .position(|标签| 标签.文档.路径.as_ref() == Some(路径))
    }

    /// 活动标签。
    pub fn 当前标签(&self) -> Option<&脚本标签> {
        self.标签.get(self.活动)
    }

    /// 活动标签（可变）。
    pub fn 当前标签可变(&mut self) -> Option<&mut 脚本标签> {
        self.标签.get_mut(self.活动)
    }

    /// 活动文档。
    pub fn 当前文档(&self) -> Option<&脚本文档> {
        self.当前标签().map(|标签| &标签.文档)
    }

    /// 活动文档（可变）。
    pub fn 当前文档可变(&mut self) -> Option<&mut 脚本文档> {
        self.当前标签可变().map(|标签| &mut 标签.文档)
    }

    /// 活动标签是否有未保存修改。
    pub fn 当前标签有未保存修改(&self) -> bool {
        self.当前文档().is_some_and(脚本文档::有未保存修改)
    }

    /// 打开一份文档。
    ///
    /// 同一路径已经打开时**聚焦已有标签**，不再新建：否则在文件树里双击
    /// 同一个文件会出现两条同名标签，关闭其中一条时用户根本分不清关的是哪个。
    /// 已有标签带未保存修改时保留用户内容（磁盘版本只补在没有修改的标签上），
    /// 磁盘已经读到的内容不会覆盖正在编辑的文本。
    pub fn 打开(&mut self, 文档: 脚本文档) {
        self.加载中 = None;
        self.待读取 = None;
        if let Some(路径) = 文档.路径.clone()
            && let Some(索引) = self.标签索引(&路径)
        {
            let 标签 = &mut self.标签[索引];
            if !标签.文档.有未保存修改() {
                标签.文档 = 文档;
            }
            self.切换(索引);
            return;
        }
        let 新下标 = self.标签.len();
        self.标签.push(脚本标签::新建(文档));
        self.切换(新下标);
    }

    /// 切换到指定标签，并安排恢复它的滚动位置。
    pub fn 切换(&mut self, 索引: usize) {
        if 索引 >= self.标签.len() {
            return;
        }
        self.活动 = 索引;
        self.待确认 = None;
        let 标签 = &mut self.标签[索引];
        标签.待恢复滚动 = Some(标签.滚动偏移);
    }

    /// 移除一个标签。
    ///
    /// 移除活动标签后焦点落到它后面那个；移除的是最后一个就往前退一格。
    /// 这个顺序与浏览器的标签行为一致，用户不会看到"关掉右边的标签，
    /// 焦点却跳到了最左边"。
    pub fn 移除(&mut self, 索引: usize) {
        if 索引 >= self.标签.len() {
            return;
        }
        self.标签.remove(索引);
        self.待确认 = None;
        self.另存目标 = None;
        if self.标签.is_empty() {
            self.活动 = 0;
            return;
        }
        if 索引 < self.活动 {
            self.活动 -= 1;
        }
        self.活动 = self.活动.min(self.标签.len() - 1);
        let 标签 = &mut self.标签[self.活动];
        标签.待恢复滚动 = Some(标签.滚动偏移);
    }

    /// 关闭全部标签（切换项目时用）。
    pub fn 清空(&mut self) {
        self.标签.clear();
        self.活动 = 0;
        self.加载中 = None;
        self.待读取 = None;
        self.待确认 = None;
        self.另存目标 = None;
        self.提示 = None;
        self.代码焦点 = None;
    }

    /// 声明"要打开这个文件"的意图，把读盘交给后台线程。
    ///
    /// 这里**不**清空旧文档：读盘成功前用户仍能看到并继续编辑旧文件，
    /// 失败时也不会突然变空白。代次递增与意图成对出现，保证回执能被
    /// 正确认领或丢弃。
    pub fn 请求打开(&mut self, 路径: scene_doc::path::项目相对路径) {
        self.加载代次 = self.加载代次.wrapping_add(1);
        self.加载中 = Some(路径.clone());
        self.待读取 = Some(路径);
        self.待确认 = None;
    }

    /// 是否有文件正在读取。
    pub fn 正在读取(&self) -> bool {
        self.加载中.is_some()
    }

    /// 把某个标签标记为已保存。
    ///
    /// `路径` 用于"另存为"：文档原来没有路径，保存成功后补上。调用方只在
    /// **写盘成功**后调用，因此星号清掉就一定意味着磁盘上是最新内容。
    pub fn 标记已保存(
        &mut self, 索引: usize, 路径: Option<scene_doc::path::项目相对路径>
    ) {
        let Some(标签) = self.标签.get_mut(索引) else {
            return;
        };
        if let Some(新路径) = 路径 {
            标签.文档.路径 = Some(新路径);
        }
        标签.文档.已保存文本 = 标签.文档.文本.clone();
    }

    /// 重新计算活动标签的查找命中。
    pub fn 重新查找(&mut self) {
        if let Some(标签) = self.当前标签可变() {
            标签.重新查找();
        }
    }

    /// 跳到活动标签的下一个命中。
    pub fn 下一个命中(&mut self) {
        if let Some(标签) = self.当前标签可变() {
            标签.下一个命中();
        }
    }

    /// 跳到活动标签的上一个命中。
    pub fn 上一个命中(&mut self) {
        if let Some(标签) = self.当前标签可变() {
            标签.上一个命中();
        }
    }
}

/// 绘制脚本面板。
///
/// 自上而下：标签栏 → 工具栏 → 查找栏 → 编辑区 → 确认浮层。确认浮层必须
/// 最后画，否则会被编辑区覆盖（`egui` 按调用顺序叠放）。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    if !上下文.界面.脚本.标签.is_empty() {
        绘制标签栏(ui, 上下文);
        ui.separator();
    }
    绘制工具栏(ui, 上下文);
    绘制查找栏(ui, 上下文);
    ui.separator();
    绘制编辑区(ui, 上下文);
    绘制确认浮层(ui, 上下文);
}

/// 绘制顶部工具栏。
///
/// 顺序：保存 / 另存为 / 查找三个图标按钮 → 源文件下拉 → 当前文档名与行数 →
/// 后台读盘中的转圈提示 → 未保存角标。
///
/// 源文件下拉是必要入口：资源浏览器只扫 `assets/`，而 Rust 源码在 `src/`
/// （原生 Cargo 工程还可能是多个成员包的 `src/`）。没有它，用户只能靠
/// 「点构建诊断跳转」才能打开源文件 —— 那不叫浏览。
///
/// 三个按钮的启用条件都是「有活动文档」，与是否有未保存修改无关：对干净
/// 文档再保存一次只是重复写盘，无害。
fn 绘制工具栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let 有文档 = 上下文.界面.脚本.当前文档().is_some();
        let 有修改 = 上下文.界面.脚本.当前标签有未保存修改();

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
        let 源文件 = 上下文.界面.源文件(上下文.编辑器.项目.as_ref());
        let 当前路径 = 上下文
            .界面
            .脚本
            .当前文档()
            .and_then(|文档| 文档.路径.clone());
        let 显示文本 = 当前路径
            .as_ref()
            .map(|路径| 路径.文本().to_string())
            .unwrap_or_else(|| 语言.取("source_files").to_string());
        egui::ComboBox::from_id_salt("script_source_files")
            .selected_text(egui::RichText::new(显示文本).small())
            .width(220.0)
            .show_ui(ui, |ui| {
                if 源文件.is_empty() {
                    ui.label(
                        egui::RichText::new(语言.取("source_files_empty"))
                            .small()
                            .color(配色.次要文字),
                    );
                    return;
                }
                for 路径 in &源文件 {
                    let 选中 = 当前路径.as_ref() == Some(路径);
                    if ui.selectable_label(选中, 路径.文本()).clicked() {
                        上下文.动作.推(UiAction::打开脚本(路径.clone()));
                    }
                }
            });

        ui.separator();
        match 上下文.界面.脚本.当前文档() {
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
        if let Some(路径) = 上下文.界面.脚本.加载中.as_ref() {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(
                RichText::new(format!("{} {}", 语言.取("script_loading"), 路径.文本()))
                    .small()
                    .color(配色.次要文字),
            );
        }
        if 有修改 {
            ui.label(
                RichText::new(语言.取("script_unsaved"))
                    .small()
                    .color(配色.警告),
            );
        }
    });
}

/// 绘制查找栏（仅在「查找」打开时）。
///
/// 查找词、命中列表与当前序号都挂在活动标签上：切到别的文件再切回来，
/// 高亮与「第几个命中」还在原处（见 [`脚本标签`] 的说明）。
fn 绘制查找栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    if 上下文.界面.脚本.显示查找
        && let Some(标签) = 上下文.界面.脚本.当前标签可变()
    {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(语言.取("script_find"))
                    .small()
                    .color(配色.次要文字),
            );
            let 响应 = ui.add(
                egui::TextEdit::singleline(&mut 标签.查找)
                    .id_salt("script_find_input")
                    .desired_width(180.0)
                    .hint_text(语言.取("script_find_hint")),
            );
            if 响应.changed() {
                标签.重新查找();
            }
            if ui.small_button("◀").clicked() {
                标签.上一个命中();
            }
            if ui.small_button("▶").clicked() {
                标签.下一个命中();
            }
            let 命中数 = 标签.命中行.len();
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
}

/// 绘制代码编辑区（行号栏 + 滚动区 + 编辑框）。
///
/// 步骤：
/// 1. 没有标签时只画空态提示，并清空 `代码焦点`（否则关掉最后一个标签后
///    Ctrl+S 还会以为焦点在代码里）；`ui.separator()` 由调用方在查找栏之后画。
/// 2. 字号下限 13：更小的等宽字形会挤在一起，行号也难认。
/// 3. 行号槽先按最大行号位数占位，数字等滚动之后才画 —— 需要先拿到
///    galley 的落点。
/// 4. 滚动区与编辑框的 egui id 都按「标签键」区分：路径文本在项目内唯一，
///    未命名文档退回下标；共用 id 会让滚动位置与光标跟着「当前谁在画」
///    漂移，切标签看起来像随机跳了一下。
/// 5. 着色整篇一次推进词法状态，块注释与原始字符串因此可以跨行；命中词用
///    选中背景强调。
/// 6. 滚动目标优先于待恢复偏移：前者来自「跳到第 N 行」（把目标行放在视口
///    上部约三分之一处，不贴顶），后者来自切回标签。两者都只设一次，之后
///    交给滚动区自己维护 —— 每帧强设偏移会顶掉用户的滚轮。
/// 7. 内容宽度跨帧记忆：`TextEdit` 的宽度被可用宽度夹住，撑宽子 `Ui` 需要
///    上一帧量到的 galley 宽度，否则第一帧永远只有视口宽。
/// 8. 光标行号由 `CCursor::index`（字符下标）前的换行数加一得到。
/// 9. 行号栏只画可见行，且铺不透明底：横向滚动时代码会从槽右侧滑过，
///    透底会串字。
/// 10. 记录代码框的 egui id 到 `代码焦点`（`Ctrl+S` 靠它判断焦点是否在
///     代码面板）与滚动偏移；文本有变化时写回标签并重算查找命中。
fn 绘制编辑区(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    if 上下文.界面.脚本.标签.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new(语言.取("script_empty_hint")).color(配色.次要文字));
        ui.add_space(6.0);
        ui.label(
            RichText::new(语言.取("script_empty_hint_2"))
                .small()
                .color(配色.次要文字),
        );
        上下文.界面.脚本.代码焦点 = None;
        return;
    }

    let 字号 = 上下文.外观.基础字号.max(13.0);
    let 活动 = 上下文.界面.脚本.活动;
    let 高亮配色 = 配色;
    let 字体 = FontId::monospace(字号);
    let 行高 =
        ui.fonts_mut(|字体库| 字体库.row_height(&字体)) + ui.spacing().extra_text_line_spacing;
    let mut 代码框_id = None;

    {
        let Some(标签) = 上下文.界面.脚本.当前标签可变() else {
            return;
        };
        let 语言类型 = 标签.文档.语言;
        let 查找词 = 标签.查找.clone();
        let mut 文本 = 标签.文档.文本.clone();
        let 行数 = 文本.split('\n').count().max(1);
        let 滚动目标 = 标签.滚动到行.take();
        let 待恢复滚动 = 标签.待恢复滚动.take();
        let 标签键 = 标签
            .文档
            .路径
            .as_ref()
            .map(|路径| 路径.文本().to_string())
            .unwrap_or_else(|| format!("#{活动}"));
        let 内容宽键 = egui::Id::new(("script_code_content_width", 标签键.as_str()));

        let mut 布局任务 = |ui: &Ui, 缓冲: &dyn egui::TextBuffer, _宽度: f32| {
            let mut 作业 = egui::text::LayoutJob::default();
            let mut 状态 = 词法状态::新();
            for 行 in 缓冲.as_str().split('\n') {
                let 词元 = if 语言类型.支持着色() {
                    切词带状态(行, &mut 状态)
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

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;

            let 位数 = 行数.to_string().len().max(2);
            let 数字宽 = ui.fonts_mut(|字体库| {
                字体库
                    .layout_no_wrap("0".repeat(位数), 字体.clone(), 配色.禁用文字)
                    .size()
                    .x
            });
            let 槽宽 = 数字宽 + 12.0;
            let 槽 = ui
                .allocate_exact_size(
                    egui::vec2(槽宽, ui.available_height()),
                    egui::Sense::hover(),
                )
                .0;

            let 可用宽 = ui.available_width().max(1.0);
            let 上一帧宽 = ui
                .data(|数据| 数据.get_temp::<f32>(内容宽键))
                .unwrap_or(0.0);
            let 内容宽 = 上一帧宽.max(可用宽);

            let mut 滚动 = egui::ScrollArea::both()
                .id_salt(("script_code_scroll", 标签键.as_str()))
                .auto_shrink([false, false]);
            if let Some(行) = 滚动目标 {
                let 偏移 = (行.min(行数.saturating_sub(1)) as f32) * 行高 - 槽.height() * 0.3;
                滚动 = 滚动.vertical_scroll_offset(偏移.max(0.0));
            } else if let Some(偏移) = 待恢复滚动 {
                滚动 = 滚动.vertical_scroll_offset(偏移.max(0.0));
            }

            let 滚动输出 = 滚动.show(ui, |ui| {
                let 子尺寸 = egui::vec2(内容宽, (行数 as f32 * 行高).max(ui.available_height()));
                ui.allocate_ui_with_layout(
                    子尺寸,
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let 编辑器 = egui::TextEdit::multiline(&mut 文本)
                            .id_salt(("script_code_edit", 标签键.as_str()))
                            .code_editor()
                            .font(字体.clone())
                            .desired_rows(30)
                            .layouter(&mut 布局任务);
                        let 输出 = 编辑器.show(ui);
                        if let Some(范围) = 输出.cursor_range {
                            let 字符下标: usize = 范围.primary.index.into();
                            let 前缀: String = 文本.chars().take(字符下标).collect();
                            标签.文档.光标行 = Some(前缀.matches('\n').count() + 1);
                        }
                        输出
                    },
                )
                .inner
            });
            let 滚动偏移 = 滚动输出.state.offset.y;
            let 输出 = 滚动输出.inner;
            标签.滚动偏移 = 滚动偏移;
            代码框_id = Some(输出.response.id);
            let 内容宽实测 = 输出.galley.rect.width().max(可用宽);
            ui.data_mut(|数据| 数据.insert_temp(内容宽键, 内容宽实测));

            let 可画 = 槽.intersect(ui.clip_rect());
            if 可画.is_positive() {
                let 画布 = ui.painter().with_clip_rect(可画);
                画布.rect_filled(槽, 0.0, 配色.面板背景);
                for 行 in 可见行区间(槽.y_range(), 输出.galley_pos.y, 行高, 行数) {
                    let 纵 = 输出.galley_pos.y + 行 as f32 * 行高;
                    画布.text(
                        egui::pos2(槽.right() - 6.0, 纵),
                        egui::Align2::RIGHT_TOP,
                        format!("{:>位数$}", 行 + 1),
                        字体.clone(),
                        配色.禁用文字,
                    );
                }
                画布.line_segment(
                    [槽.right_top(), 槽.right_bottom()],
                    egui::Stroke::new(1.0, 配色.分割线),
                );
            }
        });

        if 文本 != 标签.文档.文本 {
            标签.文档.文本 = 文本;
            if !查找词.is_empty() {
                标签.重新查找();
            }
        }
    }

    上下文.界面.脚本.代码焦点 = 代码框_id;
}

/// 绘制标签栏。
///
/// 每个标签 = 可点选的标题 + 独立的关闭按钮。标题后的 `*` 表示有未保存
/// 修改 —— 用星号而不是只靠颜色，因为颜色在深色 / 浅色主题下还要与
/// "选中""警告"区分，而 `*` 在任何主题里都只有一个意思。
///
/// 每个 `×` 都挂自己的下标，而不是「关当前标签」：用户点哪一条就关哪一条。
///
/// 先把标题快照出来再画：绘制过程中既要遍历 `界面.脚本.标签`，又要往
/// `动作` 队列里推；快照只有标题字符串与布尔量，代价可以忽略，却让
/// "遍历状态"与"推动作"不再同时借用同一个结构。
fn 绘制标签栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 活动 = 上下文.界面.脚本.活动;
    let 快照: Vec<(usize, String, bool)> = 上下文
        .界面
        .脚本
        .标签
        .iter()
        .enumerate()
        .map(|(下标, 标签)| (下标, 标签.文档.显示名(语言), 标签.文档.有未保存修改()))
        .collect();

    let mut 要切换 = None;
    let mut 要关闭 = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (下标, 名, 有修改) in &快照 {
            let 标题 = if *有修改 {
                format!("{名}*")
            } else {
                名.clone()
            };
            let 响应 = ui.selectable_label(
                *下标 == 活动,
                RichText::new(标题).small().color(if *有修改 {
                    配色.警告
                } else {
                    配色.文字
                }),
            );
            if 响应.clicked() {
                要切换 = Some(*下标);
            }
            if ui
                .small_button("×")
                .on_hover_text(语言.取("script_tab_close"))
                .clicked()
            {
                要关闭 = Some(*下标);
            }
        }
    });
    if let Some(下标) = 要切换 {
        上下文.动作.推(UiAction::切换脚本标签(下标));
    }
    if let Some(下标) = 要关闭 {
        上下文.动作.推(UiAction::关闭脚本标签(下标));
    }
}

/// 绘制未保存确认浮层。
///
/// 用 `egui::Window` 而不是模态框：脚本面板可能停靠在浮窗里，模态会连
/// 主编辑区一起锁住，用户想先看看别处的代码再决定都做不到。点右上角
/// 关闭等同"取消"—— 把关闭按钮当成"放弃修改"会让一次误点毁掉编辑。
fn 绘制确认浮层(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let Some(待确认) = 上下文.界面.脚本.待确认.clone() else {
        return;
    };
    let 正文 = match &待确认 {
        脚本待确认::关闭 { 索引 } => {
            let 名 = 上下文
                .界面
                .脚本
                .标签
                .get(*索引)
                .map(|标签| 标签.文档.显示名(语言))
                .unwrap_or_default();
            语言.填充("script_confirm_close", &[&名])
        }
        脚本待确认::打开 { 路径 } => 语言.填充("script_confirm_switch", &[&路径.文本()]),
    };

    let mut 打开 = true;
    let mut 要保存 = false;
    let mut 要放弃 = false;
    let mut 要取消 = false;
    egui::Window::new(语言.取("script_confirm_title"))
        .id(egui::Id::new("script_confirm_overlay"))
        .collapsible(false)
        .resizable(false)
        .open(&mut 打开)
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new(正文).color(配色.文字));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(语言.取("script_confirm_save")).clicked() {
                    要保存 = true;
                }
                if ui
                    .button(RichText::new(语言.取("script_confirm_discard")).color(配色.警告))
                    .clicked()
                {
                    要放弃 = true;
                }
                if ui.button(语言.取("cancel")).clicked() {
                    要取消 = true;
                }
            });
        });

    if 要保存 {
        上下文.动作.推(UiAction::脚本确认保存);
    } else if 要放弃 {
        上下文.动作.推(UiAction::脚本放弃修改);
    } else if 要取消 || !打开 {
        上下文.动作.推(UiAction::脚本取消确认);
    }
}

/// 行号槽里应当画出的行下标区间（半开）。
///
/// 代码首行的屏幕纵坐标由 `TextEditOutput::galley_pos` 给出，滚动之后它
/// 会跟着变，因此"哪些行可见"必须每帧重新算：只画可见行，长文件才不会
/// 因为几千个行号而拖慢一帧。
fn 可见行区间(
    槽: egui::Rangef,
    首行顶: f32,
    行高: f32,
    行数: usize,
) -> std::ops::Range<usize> {
    if 行数 == 0 || 行高 <= 0.0 || !首行顶.is_finite() {
        return 0..0;
    }
    let 首 = (((槽.min - 首行顶) / 行高).floor().max(0.0) as usize).min(行数);
    let 末 = (((槽.max - 首行顶) / 行高).ceil().max(0.0) as usize).min(行数);
    首..末.max(首)
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
        let 标签 = 状态.当前标签可变().expect("应当有活动标签");
        标签.查找 = "alpha".to_string();
        标签.重新查找();
        assert_eq!(标签.命中行, vec![0, 2], "查找应当不区分大小写");
        assert_eq!(标签.滚动到行, Some(0));
        标签.下一个命中();
        assert_eq!(标签.滚动到行, Some(2));
        标签.下一个命中();
        assert_eq!(标签.滚动到行, Some(0), "命中应当循环");
        标签.上一个命中();
        assert_eq!(标签.滚动到行, Some(2));
    }

    #[test]
    fn 空查找不产生命中() {
        let mut 状态 = 脚本状态::default();
        状态.打开(脚本文档::新建(None, "abc".to_string()));
        let 标签 = 状态.当前标签可变().expect("应当有活动标签");
        标签.查找 = String::new();
        标签.重新查找();
        assert!(标签.命中行.is_empty());
        // 没有命中时跳转是安全的空操作。
        标签.下一个命中();
        标签.上一个命中();
    }

    #[test]
    fn 语言按扩展名识别() {
        assert_eq!(脚本语言::按扩展名("rs"), 脚本语言::Rust);
        assert_eq!(脚本语言::按扩展名("RS"), 脚本语言::Rust);
        assert_eq!(脚本语言::按扩展名("txt"), 脚本语言::纯文本);
        assert!(脚本语言::Rust.支持着色());
        assert!(!脚本语言::纯文本.支持着色());
    }

    #[test]
    fn 块注释跨行仍然是注释() {
        let mut 状态 = 词法状态::新();
        let 第一行 = "let 甲 = 1; /* 注释开始";
        let 词元 = 切词带状态(第一行, &mut 状态);
        assert_eq!(词元.last().expect("应当有词元").1, 词元角色::注释);
        assert!(状态.跨行中());

        let 词元 = 切词带状态("这一整行都在注释里", &mut 状态);
        assert!(
            词元.iter().all(|(_, 角色)| *角色 == 词元角色::注释),
            "跨行块注释的第二行必须整行是注释"
        );
        assert!(状态.跨行中());

        let 第三行 = "结束 */ let 乙 = 2;";
        let 词元 = 切词带状态(第三行, &mut 状态);
        assert!(!状态.跨行中());
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| *角色 == 词元角色::关键字 && &第三行[范围.clone()] == "let"),
            "注释结束后的代码要恢复正常着色"
        );
    }

    #[test]
    fn 块注释可以嵌套() {
        let mut 状态 = 词法状态::新();
        切词带状态("/* 外层 /* 内层 */ 仍在外层", &mut 状态);
        assert!(状态.跨行中(), "嵌套块注释应当在第一行结束时仍未闭合");
        切词带状态("外层结束 */", &mut 状态);
        assert!(!状态.跨行中());
    }

    #[test]
    fn 原始字符串跨行() {
        let mut 状态 = 词法状态::新();
        let 第一行 = r####"let 文本 = r#"第一段"####;
        let 词元 = 切词带状态(第一行, &mut 状态);
        assert_eq!(词元.last().expect("应当有词元").1, 词元角色::字符串);
        assert!(状态.跨行中());

        let 第二行 = r####"第二段"#; let 数 = 1;"####;
        let 词元 = 切词带状态(第二行, &mut 状态);
        assert!(!状态.跨行中());
        assert!(
            词元
                .iter()
                .any(|(范围, 角色)| *角色 == 词元角色::关键字 && &第二行[范围.clone()] == "let"),
            "原始字符串闭合后的代码要恢复正常着色"
        );
    }

    #[test]
    fn 原始标识符不是原始字符串() {
        let 行 = "let r#类型 = 1;";
        let 词元 = 切词(行);
        assert!(
            词元.iter().all(|(_, 角色)| *角色 != 词元角色::字符串),
            "`r#标识符` 是原始标识符，不是字符串"
        );
    }

    #[test]
    fn 属性段整体按属性着色() {
        let 行 = "#[derive(Debug, Clone)]";
        let 词元 = 切词(行);
        assert_eq!(词元.len(), 1);
        assert_eq!(词元[0].1, 词元角色::属性);
        assert_eq!(词元[0].0, 0..行.len());

        let 行 = "#![allow(dead_code)]";
        let 词元 = 切词(行);
        assert_eq!(词元.last().expect("应当有词元").1, 词元角色::属性);
    }

    #[test]
    fn 可见行区间跟着滚动走() {
        // 视口 0..100、行高 10、首行顶 0：可见 10 行。
        assert_eq!(
            可见行区间(egui::Rangef::new(0.0, 100.0), 0.0, 10.0, 30),
            0..10
        );
        // 滚下去 50：可见行从第 5 行开始。
        assert_eq!(
            可见行区间(egui::Rangef::new(0.0, 100.0), -50.0, 10.0, 30),
            5..15
        );
        // 文档只有 3 行：不越界。
        assert_eq!(
            可见行区间(egui::Rangef::new(0.0, 100.0), -50.0, 10.0, 3),
            3..3
        );
        // 退化输入不 panic。
        assert_eq!(
            可见行区间(egui::Rangef::new(0.0, 100.0), 0.0, 0.0, 10),
            0..0
        );
    }

    #[test]
    fn 打开文件会清掉读取意图而请求打开会置位() {
        let mut 状态 = 脚本状态::default();
        let 路径 = scene_doc::path::项目相对路径::解析("src/main.rs").expect("合法路径");
        状态.请求打开(路径.clone());
        assert!(状态.正在读取());
        assert_eq!(状态.加载代次, 1);
        assert_eq!(状态.待读取.as_ref(), Some(&路径));

        状态.打开(脚本文档::新建(Some(路径), "fn main() {}".to_string()));
        assert!(!状态.正在读取(), "装载完成后不应再显示正在读取");
        assert!(状态.待读取.is_none());
    }

    #[test]
    fn 多标签打开与切换保留各自状态() {
        let mut 状态 = 脚本状态::default();
        let 甲 = scene_doc::path::项目相对路径::解析("src/甲.rs").expect("合法路径");
        let 乙 = scene_doc::path::项目相对路径::解析("src/乙.rs").expect("合法路径");
        状态.打开(脚本文档::新建(
            Some(甲.clone()),
            "fn 甲() {}".to_string(),
        ));
        状态.打开(脚本文档::新建(
            Some(乙.clone()),
            "fn 乙() {}".to_string(),
        ));
        assert_eq!(状态.标签.len(), 2);
        assert_eq!(状态.活动, 1, "新打开的文件应当成为活动标签");

        {
            let 标签 = 状态.当前标签可变().expect("应当有活动标签");
            标签.查找 = "乙".to_string();
            标签.重新查找();
            标签.滚动偏移 = 240.0;
            标签.文档.文本.push_str("\n// 未保存");
        }
        assert!(状态.当前标签有未保存修改());

        状态.切换(0);
        assert_eq!(状态.当前文档().unwrap().路径.as_ref(), Some(&甲));
        let 甲标签 = 状态.当前标签().unwrap();
        assert!(甲标签.查找.is_empty(), "甲标签的查找词不该被乙污染");
        assert_eq!(甲标签.待恢复滚动, Some(0.0));
        assert!(!状态.当前标签有未保存修改());

        状态.切换(1);
        let 乙标签 = 状态.当前标签().unwrap();
        assert_eq!(乙标签.查找, "乙");
        assert_eq!(
            乙标签.待恢复滚动,
            Some(240.0),
            "切回来要恢复离开时的滚动位置"
        );
        assert!(乙标签.文档.有未保存修改(), "切换标签不能丢未保存修改");
    }

    #[test]
    fn 重复打开同一路径聚焦已有标签() {
        let mut 状态 = 脚本状态::default();
        let 甲 = scene_doc::path::项目相对路径::解析("src/甲.rs").expect("合法路径");
        let 乙 = scene_doc::path::项目相对路径::解析("src/乙.rs").expect("合法路径");
        状态.打开(脚本文档::新建(
            Some(甲.clone()),
            "fn 甲() {}".to_string(),
        ));
        状态.打开(脚本文档::新建(Some(乙), "fn 乙() {}".to_string()));
        assert_eq!(状态.活动, 1);

        状态.打开(脚本文档::新建(
            Some(甲.clone()),
            "fn 甲() {}\n// 磁盘版本".to_string(),
        ));
        assert_eq!(状态.标签.len(), 2, "同一路径不能开出两条标签");
        assert_eq!(状态.活动, 0, "重复打开应当聚焦已有标签");
        assert!(
            状态.当前文档().unwrap().文本.contains("磁盘版本"),
            "没有未保存修改时应当取磁盘内容"
        );

        // 已有未保存修改时，磁盘内容不能把用户写的东西盖掉。
        状态.当前文档可变().unwrap().文本 = "用户改过的内容".to_string();
        状态.打开(脚本文档::新建(Some(甲), "磁盘内容".to_string()));
        assert_eq!(状态.当前文档().unwrap().文本, "用户改过的内容");
    }

    #[test]
    fn 关闭标签后活动下标前后对齐() {
        let mut 状态 = 脚本状态::default();
        let 路径们: Vec<_> = ["甲", "乙", "丙"]
            .iter()
            .map(|名| {
                scene_doc::path::项目相对路径::解析(&format!("src/{名}.rs")).expect("合法路径")
            })
            .collect();
        for 路径 in &路径们 {
            状态.打开(脚本文档::新建(Some(路径.clone()), String::new()));
        }
        assert_eq!(状态.标签.len(), 3);

        状态.切换(1);
        状态.移除(1);
        assert_eq!(状态.标签.len(), 2);
        assert_eq!(状态.活动, 1, "关掉中间的标签，焦点落到后面那个");
        assert_eq!(状态.当前文档().unwrap().路径.as_ref(), Some(&路径们[2]));

        状态.移除(1);
        assert_eq!(状态.活动, 0, "关掉最后一个标签应当往前退一格");

        状态.移除(0);
        assert!(状态.标签.is_empty());
        assert_eq!(状态.活动, 0, "没有标签时下标恒为 0");
        assert!(状态.当前文档().is_none());
    }

    #[test]
    fn 标记已保存只清除目标标签的星号() {
        let mut 状态 = 脚本状态::default();
        let 甲 = scene_doc::path::项目相对路径::解析("src/甲.rs").expect("合法路径");
        let 乙 = scene_doc::path::项目相对路径::解析("src/乙.rs").expect("合法路径");
        状态.打开(脚本文档::新建(Some(甲), "甲".to_string()));
        状态.打开(脚本文档::新建(Some(乙), "乙".to_string()));
        for 下标 in 0..2 {
            状态.标签[下标].文档.文本.push('!');
        }
        assert!(状态.标签.iter().all(|标签| 标签.文档.有未保存修改()));

        状态.标记已保存(0, None);
        assert!(!状态.标签[0].文档.有未保存修改());
        assert!(
            状态.标签[1].文档.有未保存修改(),
            "保存甲不能顺手把乙的星号也清掉"
        );
    }
}
