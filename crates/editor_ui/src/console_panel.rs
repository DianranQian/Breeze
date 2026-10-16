//! 控制台面板：日志、警告与错误。

use editor_core::state::{日志级别, 编辑器状态};
use egui::{RichText, Ui};

use crate::state::{UiAction, 绘制上下文};

/// 一条用于显示的日志行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 显示日志 {
    /// 级别。
    pub 级别: 日志级别,
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
        // 搜索在**渲染之后**的文本上做：用户看到什么就搜什么。
        // 因此要在当前语言下先渲染再匹配，而不是拿存储内容去比。
        .map(|条目| 显示日志 {
            级别: 条目.级别,
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
    fn 可以隐藏某一级别() {
        let 过滤 = 控制台过滤 {
            显示信息: false,
            显示警告: true,
            显示错误: true,
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
