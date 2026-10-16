//! 历史面板（撤销栈查看）与运行时诊断面板。
//!
//! 这两块放在同一个文件里，是因为它们都是"只读地展示编辑器的内部状态"，
//! 没有自己的交互状态，体量也都很小。分开成两个文件会让
//! "面板实现"目录里多出两个十几行的碎片文件。

use egui::{RichText, Ui};

use crate::state::{UiAction, 绘制上下文};

/// 撤销栈里的一条历史记录（用于显示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 历史项 {
    /// 操作名。
    pub 显示名: String,
    /// 是否已经处于"已应用"状态（否则是"可重做"状态）。
    pub 已应用: bool,
    /// 是否是当前位置的下一步（即点击按钮会执行的那一步）。
    pub 是下一位置: bool,
}

/// 构建历史显示列表：已应用的（旧→新）在前，可重做的（近→远）在后。
///
/// 顺序刻意如此：上半部分是"做过的事"，按时间正序排列，
/// 用户从下往上读就是"我刚刚干了什么"；下半部分是"撤销掉的事"，
/// 按时间倒序（最近撤销的在最上面），也就是重做会先执行它。
pub fn 构建历史列表(
    撤销名列表: &[String], 可重做名列表: &[String]
) -> Vec<历史项> {
    let mut 结果 = Vec::new();
    let 已应用总数 = 撤销名列表.len();
    for (下标, 名称) in 撤销名列表.iter().enumerate() {
        结果.push(历史项 {
            显示名: 名称.clone(),
            已应用: true,
            是下一位置: 下标 + 1 == 已应用总数,
        });
    }
    let 可重做总数 = 可重做名列表.len();
    for (下标, 名称) in 可重做名列表.iter().rev().enumerate() {
        结果.push(历史项 {
            显示名: 名称.clone(),
            已应用: false,
            是下一位置: 下标 + 1 == 可重做总数,
        });
    }
    结果
}

/// 绘制历史面板。
pub fn 绘制历史(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 撤销 = &上下文.编辑器.撤销;

    let 可撤销名: Vec<String> = 撤销
        .下一次撤销名()
        .map(|名字| vec![名字.to_string()])
        .unwrap_or_default();
    let 可重做名: Vec<String> = 撤销
        .下一次重做名()
        .map(|名字| vec![名字.to_string()])
        .unwrap_or_default();

    ui.horizontal(|ui| {
        if ui
            .add_enabled(撤销.可撤销步数() > 0, egui::Button::new("撤销"))
            .clicked()
        {
            上下文.动作.推(UiAction::撤销);
        }
        if ui
            .add_enabled(撤销.可重做步数() > 0, egui::Button::new("重做"))
            .clicked()
        {
            上下文.动作.推(UiAction::重做);
        }
    });

    ui.label(
        RichText::new(format!(
            "可撤销 {} 步 · 可重做 {} 步",
            撤销.可撤销步数(),
            撤销.可重做步数()
        ))
        .small()
        .color(上下文.配色.次要文字),
    );

    if 上下文.编辑器.有未保存修改() {
        ui.label(
            RichText::new(上下文.语言.取("● 相对上次保存已有修改"))
                .small()
                .color(上下文.配色.警告),
        );
    } else {
        ui.label(
            RichText::new(上下文.语言.取("✓ 与上次保存一致"))
                .small()
                .color(上下文.配色.次要文字),
        );
    }

    ui.separator();

    if 撤销.可撤销步数() == 0 && 撤销.可重做步数() == 0 {
        ui.label(
            RichText::new(上下文.语言.取("还没有任何可撤销的操作。")).color(上下文.配色.次要文字),
        );
        return;
    }

    // 撤销栈只对外暴露"下一步的名称"，无法列出完整历史。
    // 这里如实展示：只有完整历史列表才能提供，而完整列表需要
    // 撤销栈暴露条目的显示名（见路线图"历史面板显示完整列表"）。
    ui.label(
        RichText::new(
            上下文
                .语言
                .取("当前撤销栈不支持列出完整历史，只显示下一步操作："),
        )
        .small()
        .color(上下文.配色.次要文字),
    );
    if let Some(名字) = 可撤销名.first() {
        ui.label(RichText::new(format!("下一撤销：{名字}")).color(上下文.配色.文字));
    }
    if let Some(名字) = 可重做名.first() {
        ui.label(RichText::new(format!("下一重做：{名字}")).color(上下文.配色.次要文字));
    }
}

/// 绘制运行时 ECS 诊断面板。
pub fn 绘制运行时(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 诊断 = 上下文.运行时信息;

    if !上下文.正在运行 {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "游戏未在运行。\n点击工具栏的「运行」按钮启动当前场景，\
                 这里会显示运行时世界的实体与组件统计。",
            )
            .color(上下文.配色.次要文字),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "说明：运行时世界与编辑器文档是两份独立数据。\
                 运行时对实体和组件的改动不会写回场景文件。",
            )
            .small()
            .color(上下文.配色.次要文字),
        );
        return;
    }

    表格行(
        ui,
        上下文,
        "状态",
        if 上下文.已暂停 {
            "已暂停"
        } else {
            "运行中"
        },
    );
    表格行(ui, 上下文, "已运行", &format!("{:.1} 秒", 诊断.已运行秒数));
    表格行(ui, 上下文, "实体总数", &诊断.实体总数.to_string());
    表格行(ui, 上下文, "组件类型数", &诊断.组件类型数.to_string());
    表格行(ui, 上下文, "已加载资源数", &诊断.资源数.to_string());

    ui.separator();
    ui.label(
        RichText::new(上下文.语言.取("运行时世界（只读）"))
            .strong()
            .color(上下文.配色.文字),
    );
    ui.label(
        RichText::new(
            "本面板展示的是**游戏运行时**的 ECS 统计，不是编辑器文档的统计。\
             编辑器文档里的实体数量见「场景树」面板。",
        )
        .small()
        .color(上下文.配色.次要文字),
    );
}

/// 绘制性能面板。
pub fn 绘制性能(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 诊断 = 上下文.运行时信息;

    if !上下文.正在运行 {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "游戏未运行，没有性能数据。\n\
                 编辑器自身的界面帧时间由 egui 统计，可在「性能」里查看编辑器开销。",
            )
            .color(上下文.配色.次要文字),
        );
    } else {
        表格行(ui, 上下文, "帧率", &format!("{:.1} FPS", 诊断.帧率));
        表格行(ui, 上下文, "帧耗时", &format!("{:.2} ms", 诊断.帧耗时毫秒));
        if 诊断.帧耗时毫秒 > 0.0 {
            表格行(
                ui,
                上下文,
                "理论上限",
                &format!("{:.0} FPS", 1000.0 / 诊断.帧耗时毫秒),
            );
        }
        // 简单的帧时间条形图，比数字更直观地看出抖动。
        let 占比 = (诊断.帧耗时毫秒 / 16.67).clamp(0.0, 2.0) / 2.0;
        let 颜色 = if 诊断.帧耗时毫秒 > 16.67 {
            上下文.配色.警告
        } else {
            上下文.配色.强调
        };
        ui.add(
            egui::ProgressBar::new(占比)
                .desired_width(180.0)
                .fill(颜色)
                .text(format!("{:.2} ms / 16.67 ms", 诊断.帧耗时毫秒)),
        );
    }

    ui.separator();
    ui.label(
        RichText::new(上下文.语言.取("编辑器自身开销"))
            .strong()
            .color(上下文.配色.文字),
    );
    let 场景实体 = 上下文.编辑器.会话.文档.实体数量();
    表格行(ui, 上下文, "编辑器中实体数", &场景实体.to_string());
    表格行(
        ui,
        上下文,
        "撤销栈估算占用",
        &crate::assets_panel::人类可读大小(上下文.编辑器.撤销.估算字节() as u64),
    );
    表格行(
        ui,
        上下文,
        "已注册组件类型",
        &上下文.编辑器.注册表.数量().to_string(),
    );
}

/// 绘制一行"标签 + 值"。
fn 表格行(ui: &mut Ui, 上下文: &绘制上下文<'_>, 标签: &str, 值: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(标签).color(上下文.配色.次要文字));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(值).monospace().color(上下文.配色.文字));
        });
    });
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 历史列表把已应用与可重做分开且顺序正确() {
        let 已应用 = vec!["新建实体".to_string(), "修改变换".to_string()];
        let 可重做 = vec!["重命名实体".to_string(), "删除组件".to_string()];

        let 列表 = 构建历史列表(&已应用, &可重做);

        // 前两项是已应用，按时间正序
        assert_eq!(列表[0].显示名, "新建实体");
        assert!(列表[0].已应用);
        assert_eq!(列表[1].显示名, "修改变换");
        assert!(列表[1].已应用);
        assert!(列表[1].是下一位置, "最后一个已应用项是当前位置");

        // 后两项是可重做，最近撤销的排在最前
        assert_eq!(列表[2].显示名, "删除组件");
        assert!(!列表[2].已应用);
        assert_eq!(列表[3].显示名, "重命名实体");
        assert!(列表[3].是下一位置, "重做会先执行它，因此标记为下一位置");
    }

    #[test]
    fn 空历史产生空列表() {
        assert!(构建历史列表(&[], &[]).is_empty());
    }

    #[test]
    fn 只有可重做时列表正确() {
        let 列表 = 构建历史列表(&[], &["甲".to_string()]);
        assert_eq!(列表.len(), 1);
        assert!(!列表[0].已应用);
        assert!(列表[0].是下一位置);
    }
}
