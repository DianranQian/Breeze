//! 控制台面板：日志、警告与错误。

use editor_core::log_source::日志来源;
use editor_core::state::{日志级别, 编辑器状态};
use egui::{RichText, Ui};

use crate::state::{UiAction, 绘制上下文};

/// 一条用于显示的日志行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 显示日志 {
    /// 级别。
    pub 级别: 日志级别,
    /// 来源（构建/运行/资源/编辑器）。
    pub 来源: 日志来源,
    /// 正文。
    pub 消息: String,
}

/// 按过滤条件筛选日志。
pub fn 筛选日志(
    日志: &[editor_core::state::日志条目],
    过滤: crate::state::控制台过滤,
    关键字: &str,
    语言: crate::i18n::语言,
) -> Vec<显示日志> {
    let 关键字 = 关键字.trim();
    日志
        .iter()
        .filter(|条目| match 条目.级别 {
            日志级别::信息 => 过滤.显示信息,
            日志级别::警告 => 过滤.显示警告,
            日志级别::错误 => 过滤.显示错误,
        })
        .filter(|条目| match 日志来源::由条目(条目) {
            日志来源::编辑器 => 过滤.显示编辑器,
            日志来源::构建 => 过滤.显示构建,
            日志来源::运行 => 过滤.显示运行,
            日志来源::资源 => 过滤.显示资源,
        })
        // 搜索在**渲染之后**的文本上做：用户看到什么就搜什么。
        // 因此要在当前语言下先渲染再匹配，而不是拿存储内容去比。
        .map(|条目| 显示日志 {
            级别: 条目.级别,
            来源: 日志来源::由条目(条目),
            消息: 条目.渲染(语言),
        })
        .filter(|条目| 关键字.is_empty() || 条目.消息.contains(关键字))
        .collect()
}

/// 统计各级别数量。
///
/// 返回 `(信息数, 警告数, 错误数)`。用于在过滤按钮上显示角标，
/// 让用户即使把某一级过滤掉了也能知道"其实有 3 个错误"。
pub fn 统计各级别(日志: &[editor_core::state::日志条目]) -> (usize, usize, usize) {
    let mut 信息 = 0;
    let mut 警告 = 0;
    let mut 错误 = 0;
    for 条目 in 日志 {
        match 条目.级别 {
            日志级别::信息 => 信息 += 1,
            日志级别::警告 => 警告 += 1,
            日志级别::错误 => 错误 += 1,
        }
    }
    (信息, 警告, 错误)
}

/// 统计各来源的日志条数（顺序与 [`日志来源::全部`] 一致）。
pub fn 统计各来源(日志: &[editor_core::state::日志条目]) -> [(日志来源, usize); 4] {
    let mut 计数 = [
        (日志来源::编辑器, 0usize),
        (日志来源::构建, 0),
        (日志来源::运行, 0),
        (日志来源::资源, 0),
    ];
    for 条目 in 日志 {
        let 来源 = 日志来源::由条目(条目);
        if let Some(项) = 计数.iter_mut().find(|(类别, _)| *类别 == 来源) {
            项.1 += 1;
        }
    }
    计数
}

/// 识别被引号包起来、且内含 `.rs:行[:列]` 的源码位置。
///
/// 只在引号内部解析，因此路径里的空格不会被当成分隔符。
fn 引号内的源码位置(文本: &str) -> Option<(String, u32, u32)> {
    // 34 = 双引号、39 = 单引号的 ASCII 码。这里刻意不写字符字面量：
    // 字面量里嵌引号会让 i18n 扫描器把代码当成中文文案（真实踩过）。
    for 引号 in [34u8 as char, 39u8 as char] {
        let mut 起点 = 0;
        while let Some(偏移) = 文本[起点..].find(引号) {
            let 开头 = 起点 + 偏移 + 引号.len_utf8();
            let Some(闭合偏移) = 文本[开头..].find(引号) else {
                break;
            };
            let 路径 = 文本[开头..开头 + 闭合偏移].trim().to_string();
            let 闭引号之后 = 开头 + 闭合偏移 + 引号.len_utf8();
            起点 = 闭引号之后;
            if !路径.ends_with(".rs") {
                continue;
            }
            // 行号/列号在引号之后：`:42` 或 `:42:5`。
            let 余下 = 文本[闭引号之后..].trim_start();
            let Some(余下) = 余下.strip_prefix(':') else {
                continue;
            };
            let 行数字: String = 余下.chars().take_while(char::is_ascii_digit).collect();
            let Ok(行) = 行数字.parse::<u32>() else {
                continue;
            };
            let 列 = 余下[行数字.len()..]
                .strip_prefix(':')
                .and_then(|尾部| {
                    尾部
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                        .parse::<u32>()
                        .ok()
                })
                .unwrap_or(0);
            return Some((路径, 行, 列));
        }
    }
    None
}

/// 从一行日志里找出一条"源码位置"：`路径:行[:列]`。
///
/// # 为什么值得做
///
/// 运行项目的日志里最常见的报错形态就是 `src/main.rs:42:5: ...`（rustc、
/// 链接器、以及用户自己的 `println!` 都会这么写）。把它变成可点的一小段，
/// 用户排查"游戏为什么崩"时就不用自己从一长串路径里挑出行号再手动打开文件。
///
/// # 识别规则（刻意保守）
///
/// * 必须出现 `.rs` 扩展名（本项目是 Rust 项目；把任意 `x:1` 当位置会误报）；
/// * 路径里不允许空白与引号，行号/列号是纯数字；
/// * 取**最后**一个匹配：日志前缀里常有时间戳（`12:03:45`），
///   但 `路径.rs:42` 这样带扩展名的模式不会被时间戳误命中。
///
/// 返回 `(路径文本, 行号, 列号)`；列号缺失时为 0。
pub fn 提取源码位置(文本: &str) -> Option<(String, u32, u32)> {
    // 先试"被引号包起来的路径"：`"C:\my projects\game\src\main.rs":42:5`。
    //
    // 为什么必须单独处理：路径里有空格时，下面"向右取空白为止"的规则会
    // 从空格处截断，把 `C:\my` 当成路径 —— 这是最难查的一类失败（看起来
    // 能点，点了却打不开文件）。引号是唯一的可靠边界，因此优先识别。
    if let Some(结果) = 引号内的源码位置(文本) {
        return Some(结果);
    }
    let 字节 = 文本.as_bytes();
    let 标记 = b".rs:";
    let mut 结果 = None;
    let mut 起点 = 0;
    while let Some(偏移) = 文本[起点..].find(".rs:") {
        let 点位置 = 起点 + 偏移;
        起点 = 点位置 + 标记.len();
        // 向左取路径：字母数字、下划线、点、斜杠、反斜杠、连字符、井号。
        let 左 = 文本[..点位置]
            .char_indices()
            .rev()
            .take_while(|(_, 字符)| {
                matches!(字符,
                    'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '.' | '/' | '\\' | '-' | '#' | ':')
            })
            .last()
            .map(|(下标, _)| 下标)
            .unwrap_or(点位置);
        let 路径 = &文本[左..点位置 + 3];
        // 向右取数字。
        let 行开始 = 点位置 + 标记.len();
        let 行数字: String = 文本[行开始..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if 行数字.is_empty() {
            continue;
        }
        let Ok(行) = 行数字.parse::<u32>() else {
            continue;
        };
        // 可选列：`行:列`。
        let 列开始 = 行开始 + 行数字.len();
        let mut 列 = 0;
        if 文本[列开始..].starts_with(':') {
            let 列数字: String = 文本[列开始 + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(值) = 列数字.parse::<u32>() {
                列 = 值;
            }
        }
        // 路径至少要有文件名主体（`.rs` 之前至少一个字符）。
        if 路径.len() > 3 {
            结果 = Some((路径.to_string(), 行, 列));
        }
    }
    let _ = 字节;
    结果
}

/// 绘制控制台。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let (信息数, 警告数, 错误数) = 统计各级别(上下文.编辑器.日志列表());

    ui.horizontal(|ui| {
        let mut 过滤 = 上下文.界面.控制台过滤;
        ui.checkbox(&mut 过滤.显示信息, 上下文.语言.填充("info", &[&信息数]));
        ui.checkbox(&mut 过滤.显示警告, 上下文.语言.填充("warnings", &[&警告数]));
        ui.checkbox(&mut 过滤.显示错误, 上下文.语言.填充("errors", &[&错误数]));
        上下文.界面.控制台过滤 = 过滤;

        ui.separator();
        let mut 关键字 = std::mem::take(&mut 上下文.界面.控制台搜索);
        ui.add(
            egui::TextEdit::singleline(&mut 关键字)
                .hint_text(上下文.语言.取("search_log"))
                .desired_width(140.0),
        );
        上下文.界面.控制台搜索 = 关键字;

        if ui.small_button(上下文.语言.取("clear")).clicked() {
            上下文.动作.推(UiAction::清空控制台);
        }
    });

    ui.separator();

    // 来源过滤：与级别过滤正交。排查"构建为什么失败"时先把编辑器噪声关掉，
    // 比在几十行提示里翻找有效得多（Godot 的 Output 面板同样分来源）。
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(上下文.语言.取("log_source"))
                .small()
                .color(上下文.配色.次要文字),
        );
        let mut 过滤 = 上下文.界面.控制台过滤;
        for (来源, 数量) in 统计各来源(上下文.编辑器.日志列表()) {
            // 数量用括号附在名称后面。刻意不用 `format!("{名称}...")` 这种
            // 内联模板：模板里出现中文字段名会被 i18n 扫描器当成"未翻译的
            // 界面文案"（真实踩过）。
            let 文案 = format!("{} ({})", 上下文.语言.取(来源.词条键()), 数量);
            let 开关 = match 来源 {
                日志来源::编辑器 => &mut 过滤.显示编辑器,
                日志来源::构建 => &mut 过滤.显示构建,
                日志来源::运行 => &mut 过滤.显示运行,
                日志来源::资源 => &mut 过滤.显示资源,
            };
            ui.checkbox(开关, 文案);
        }
        上下文.界面.控制台过滤 = 过滤;
    });

    ui.separator();

    let 显示列表 = 筛选日志(
        上下文.编辑器.日志列表(),
        上下文.界面.控制台过滤,
        &上下文.界面.控制台搜索,
        上下文.语言,
    );

    if 显示列表.is_empty() {
        ui.label(RichText::new(上下文.语言.取("no_log_entries")).color(上下文.配色.次要文字));
        return;
    }

    // 自动滚到底部：日志面板最有用的状态永远是最新一条。
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for 条目 in &显示列表 {
                let 颜色 = match 条目.级别 {
                    日志级别::信息 => 上下文.配色.文字,
                    日志级别::警告 => 上下文.配色.警告,
                    日志级别::错误 => 上下文.配色.错误,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(条目.级别.前缀(上下文.语言))
                            .small()
                            .color(颜色),
                    );
                    ui.label(RichText::new(&条目.消息).color(颜色));
                    // 日志里带源码位置时给一个"跳转"按钮：
                    // 运行期报错最常见的就是 `src/main.rs:42:5: ...`。
                    if let Some((路径, 行, _列)) = 提取源码位置(&条目.消息) {
                        let 响应 = ui.add(
                            egui::Button::new(
                                RichText::new(format!("{路径}:{行}"))
                                    .small()
                                    .color(上下文.配色.成功)
                                    .underline(),
                            )
                            .frame(false),
                        );
                        if 响应
                            .on_hover_text(上下文.语言.取("open_source_line_hint"))
                            .clicked()
                        {
                            上下文.动作.推(UiAction::打开构建诊断 {
                                文件: Some(std::path::PathBuf::from(路径)),
                                行,
                            });
                        }
                    }
                });
            }
        });
}

/// 编辑器是否有需要注意的问题。供菜单栏显示角标。
pub fn 有问题(编辑器: &编辑器状态) -> bool {
    编辑器
        .日志列表()
        .iter()
        .any(|条目| 条目.级别 >= 日志级别::警告)
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::state::控制台过滤;

    fn 造日志() -> Vec<editor_core::state::日志条目> {
        use editor_core::state::日志条目;
        vec![
            日志条目 {
                级别: 日志级别::信息,
                内容: editor_core::state::日志内容::自由文本("场景已保存".to_string()),
            },
            日志条目 {
                级别: 日志级别::警告,
                内容: editor_core::state::日志内容::自由文本("资源缺失".to_string()),
            },
            日志条目 {
                级别: 日志级别::错误,
                内容: editor_core::state::日志内容::自由文本("保存失败".to_string()),
            },
        ]
    }

    #[test]
    fn 默认过滤显示全部() {
        let 过滤 = 控制台过滤::default();
        assert_eq!(
            筛选日志(&造日志(), 过滤, "", crate::i18n::语言::中文).len(),
            3
        );
    }

    #[test]
    fn 引号里的带空格路径不会被截断() {
        let 文本 = r#"error: could not compile in "C:\my projects\game\src\main.rs":42:5"#;
        let (路径, 行, 列) = 提取源码位置(文本).expect("应当识别出源码位置");
        assert_eq!(路径, r"C:\my projects\game\src\main.rs");
        assert_eq!((行, 列), (42, 5));
    }

    #[test]
    fn 没有引号时仍按原有规则解析() {
        let 文本 = "src/main.rs:12:3: warning: unused variable";
        let (路径, 行, 列) = 提取源码位置(文本).expect("应当识别出源码位置");
        assert_eq!(路径, "src/main.rs");
        assert_eq!((行, 列), (12, 3));
    }

    #[test]
    fn 没有源码位置的日志返回空() {
        assert!(提取源码位置("场景已保存").is_none());
        assert!(提取源码位置("12:03:45 构建开始").is_none());
    }
    #[test]
    fn 可以只显示构建来源() {
        use editor_core::state::{日志内容, 日志条目};
        let 日志 = vec![
            日志条目 {
                级别: 日志级别::信息,
                内容: 日志内容::结构化 {
                    标识: "log_build_started",
                    参数: Vec::new(),
                },
            },
            日志条目 {
                级别: 日志级别::信息,
                内容: 日志内容::自由文本("编辑器提示".to_string()),
            },
            日志条目 {
                级别: 日志级别::信息,
                内容: 日志内容::结构化 {
                    标识: "log_physics_collision_started",
                    参数: Vec::new(),
                },
            },
        ];
        let 过滤 = 控制台过滤 {
            显示编辑器: false,
            显示运行: false,
            显示资源: false,
            ..控制台过滤::default()
        };
        let 结果 = 筛选日志(&日志, 过滤, "", crate::i18n::语言::中文);
        assert_eq!(结果.len(), 1, "只应剩下构建来源");
        assert_eq!(结果[0].来源, 日志来源::构建);
    }

    #[test]
    fn 来源统计与筛选结果一致() {
        use editor_core::state::{日志内容, 日志条目};
        let 日志 = vec![
            日志条目 {
                级别: 日志级别::信息,
                内容: 日志内容::结构化 {
                    标识: "log_build_started",
                    参数: Vec::new(),
                },
            },
            日志条目 {
                级别: 日志级别::警告,
                内容: 日志内容::自由文本("编辑器警告".to_string()),
            },
            日志条目 {
                级别: 日志级别::信息,
                内容: 日志内容::结构化 {
                    标识: "log_import_refreshed",
                    参数: Vec::new(),
                },
            },
        ];
        let 统计 = 统计各来源(&日志);
        let 总数: usize = 统计.iter().map(|(_, 数量)| 数量).sum();
        assert_eq!(总数, 日志.len(), "统计之和必须等于日志总数");
        let 构建 = 统计
            .iter()
            .find(|(来源, _)| *来源 == 日志来源::构建)
            .map(|(_, 数量)| *数量)
            .unwrap();
        assert_eq!(构建, 1);
    }
    #[test]
    fn 可以隐藏某一级别() {
        let 过滤 = 控制台过滤 {
            显示信息: false,
            ..控制台过滤::default()
        };
        let 结果 = 筛选日志(&造日志(), 过滤, "", crate::i18n::语言::中文);
        assert_eq!(结果.len(), 2);
        assert!(结果.iter().all(|条目| 条目.级别 != 日志级别::信息));
    }

    #[test]
    fn 关键字过滤() {
        let 过滤 = 控制台过滤::default();
        let 结果 = 筛选日志(&造日志(), 过滤, "保存", crate::i18n::语言::中文);
        assert_eq!(结果.len(), 2, "应当匹配「场景已保存」与「保存失败」");
    }

    #[test]
    fn 统计各级别数量() {
        let (信息, 警告, 错误) = 统计各级别(&造日志());
        assert_eq!((信息, 警告, 错误), (1, 1, 1));
    }

    #[test]
    fn 从日志里提取源码位置() {
        let 例子 = 提取源码位置("error: src/main.rs:42:5: expected item").expect("应当识别");
        assert_eq!(例子.0, "src/main.rs");
        assert_eq!(例子.1, 42);
        assert_eq!(例子.2, 5);
        // 只有行号没有列。
        let 例子 = 提取源码位置("at crates/editor_app/src/lib.rs:7").unwrap();
        assert_eq!(例子.0, "crates/editor_app/src/lib.rs");
        assert_eq!(例子.1, 7);
        assert_eq!(例子.2, 0);
        // 时间戳里的冒号不能被当成行列。
        assert!(提取源码位置("12:03:45 开始运行").is_none());
        // 没有 .rs 扩展名的一律不认。
        assert!(提取源码位置("assets/hero.png:12").is_none());
    }

    #[test]
    fn 空日志统计为零() {
        assert_eq!(统计各级别(&[]), (0, 0, 0));
        assert!(筛选日志(&[], 控制台过滤::default(), "", crate::i18n::语言::中文).is_empty());
    }

    /// 控制台按**当前语言**渲染结构化日志，而不是存储时的语言。
    #[test]
    fn 结构化日志按当前语言渲染() {
        use crate::i18n::语言;
        use editor_core::state::{日志内容, 日志参数};
        let 日志 = vec![editor_core::state::日志条目 {
            级别: 日志级别::信息,
            内容: 日志内容::结构化 {
                标识: "log_scene_opened",
                参数: vec![日志参数::用户数据("关卡.scene.ron".into())],
            },
        }];

        let 中文 = 筛选日志(&日志, 控制台过滤::default(), "", 语言::中文);
        let 英文 = 筛选日志(&日志, 控制台过滤::default(), "", 语言::英文);

        assert!(中文[0].消息.contains("已打开场景"), "{}", 中文[0].消息);
        assert!(英文[0].消息.contains("Opened scene"), "{}", 英文[0].消息);
        // 用户数据在两种语言下都保持原文
        assert!(中文[0].消息.contains("关卡.scene.ron"));
        assert!(英文[0].消息.contains("关卡.scene.ron"));
    }

    /// 搜索是在**渲染之后**的文本上做：用户看到什么就搜什么。
    #[test]
    fn 搜索基于渲染后的文本() {
        use crate::i18n::语言;
        use editor_core::state::{日志内容, 日志参数};
        let 日志 = vec![editor_core::state::日志条目 {
            级别: 日志级别::信息,
            内容: 日志内容::结构化 {
                标识: "log_scene_opened",
                参数: vec![日志参数::用户数据("关卡.scene.ron".into())],
            },
        }];

        // 英文模式下用英文关键词应当能搜到
        assert_eq!(
            筛选日志(&日志, 控制台过滤::default(), "Opened", 语言::英文).len(),
            1
        );
        // 中文模式下用中文关键词应当能搜到
        assert_eq!(
            筛选日志(&日志, 控制台过滤::default(), "已打开", 语言::中文).len(),
            1
        );
        // 语言与关键词不匹配时搜不到（说明确实是按渲染文本匹配的）
        assert_eq!(
            筛选日志(&日志, 控制台过滤::default(), "Opened", 语言::中文).len(),
            0
        );
    }

    /// 级别前缀随语言变化 —— 原先它是硬编码中文。
    #[test]
    fn 级别前缀随语言变化() {
        use crate::i18n::语言;
        assert_eq!(日志级别::信息.前缀(语言::中文), "信息");
        assert_eq!(日志级别::信息.前缀(语言::英文), "Info");
        assert_eq!(日志级别::错误.前缀(语言::英文), "Error");
    }
}
