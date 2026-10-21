//! 远程 ECS 检查器面板：只画**真实协议快照**。
//!
//! # 这个面板不做什么
//!
//! * 不构造示例实体、不缓存"上次看起来不错"的数据冒充实时；
//! * 不在本地编辑态 ECS 里查数据；
//! * 不因为连接断了就清空历史 —— 但会把历史明确标成"最近快照（不是实时）"。
//!
//! # 交互闭环
//!
//! ```text
//! 查询（分页/搜索/组件过滤） → 编辑器发协议请求 → 游戏端读真实 World
//!        → 响应按请求编号关联 → 快照更新 → 面板重绘
//! ```

use editor_core::runtime_debug::{
    调试连接状态, 运行时数据状态, 运行时调试快照
};
use egui::{RichText, Ui};
use runtime_protocol::{
    字段值, 字段视图, 实体详情, 检查支持级别, 组件视图, 资源详情
};

use crate::state::{UiAction, 绘制上下文};

/// 绘制远程 ECS 检查器。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    // 先克隆一份快照：面板既要可变借用 `界面状态`，又要读快照。
    // 一次连接快照通常只有几十 KB，每帧克隆的代价远低于把状态拆成两半。
    let 调试 = 上下文.调试.clone();
    绘制状态栏(ui, 上下文, &调试);
    ui.separator();

    match 调试.数据状态() {
        运行时数据状态::未接入 => 绘制接入说明(ui, 上下文, &调试),
        运行时数据状态::不兼容 => 绘制不可用(ui, 上下文, &调试),
        运行时数据状态::加载中 => {
            绘制等待(ui, 上下文, &调试);
        }
        运行时数据状态::不可用 => {
            绘制不可用(ui, 上下文, &调试);
            if 有历史(&调试) {
                绘制数据区(ui, 上下文, &调试);
            }
        }
        运行时数据状态::最近快照 | 运行时数据状态::实时 => {
            if 调试.数据状态() == 运行时数据状态::最近快照 {
                ui.label(
                    RichText::new(上下文.语言.取("runtime_debug_stale_warning"))
                        .small()
                        .color(上下文.配色.警告),
                );
            }
            绘制数据区(ui, 上下文, &调试);
        }
    }
}

fn 有历史(调试: &运行时调试快照) -> bool {
    调试.有历史数据()
}

fn 绘制状态栏(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.horizontal_wrapped(|ui| {
        let 颜色 = match 调试.数据状态() {
            运行时数据状态::实时 => 配色.成功,
            运行时数据状态::最近快照 => 配色.警告,
            运行时数据状态::不兼容 | 运行时数据状态::不可用 => 配色.错误,
            运行时数据状态::未接入 | 运行时数据状态::加载中 => 配色.次要文字,
        };
        ui.label(
            RichText::new(语言.取(调试.状态.词条键()))
                .strong()
                .color(颜色),
        );
        ui.label(
            RichText::new(语言.取(调试.数据状态().词条键()))
                .small()
                .color(颜色),
        );
        if 调试.是否实时() {
            ui.label(RichText::new("●").color(配色.成功))
                .on_hover_text(语言.取("runtime_debug_live_hint"));
        }

        ui.add_space(8.0);
        if ui.button(语言.取("runtime_debug_refresh")).clicked() {
            上下文.动作.推(UiAction::刷新运行时);
        }
        if 调试.状态 == 调试连接状态::已连接
            && ui.button(语言.取("runtime_debug_disconnect")).clicked()
        {
            上下文.动作.推(UiAction::断开运行时调试);
        }
        if matches!(
            调试.状态,
            调试连接状态::已断开 | 调试连接状态::失败 | 调试连接状态::不兼容
        ) && ui.button(语言.取("runtime_debug_reconnect")).clicked()
        {
            上下文.动作.推(UiAction::重连运行时调试);
        }
        if 调试.实体详情.is_some()
            && ui
                .button(语言.取("runtime_debug_send_to_agent"))
                .on_hover_text(语言.取("runtime_debug_send_to_agent_hint"))
                .clicked()
        {
            上下文.动作.推(UiAction::把运行时上下文发送给智能体);
        }
    });

    ui.horizontal_wrapped(|ui| {
        if let Some(应用) = &调试.应用名称 {
            ui.label(
                RichText::new(format!("{}：{应用}", 语言.取("runtime_debug_app")))
                    .small()
                    .color(配色.次要文字),
            );
        }
        if let Some(版本) = &调试.bevy版本 {
            ui.label(
                RichText::new(format!("Bevy {版本}"))
                    .small()
                    .color(配色.次要文字),
            );
        }
        if let Some(地址) = &调试.监听地址 {
            ui.label(
                RichText::new(format!("{}：{地址}", 语言.取("runtime_debug_loopback")))
                    .small()
                    .color(配色.次要文字),
            );
        }
        if let Some(会话) = &调试.会话标识 {
            ui.label(
                RichText::new(format!(
                    "{}：{}",
                    语言.取("runtime_debug_session"),
                    短标识(会话)
                ))
                .small()
                .color(配色.次要文字),
            );
            ui.ctx().copy_text(会话.clone());
        }
        if let Some(运行) = &调试.运行实例标识 {
            ui.label(
                RichText::new(format!(
                    "{}：{}",
                    语言.取("runtime_debug_run"),
                    短标识(运行)
                ))
                .small()
                .color(配色.次要文字),
            );
        }
    });

    if !调试.状态说明.is_empty() && 调试.状态 != 调试连接状态::已连接 {
        ui.label(RichText::new(&调试.状态说明).small().color(配色.次要文字));
    }
    if let Some(错误) = &调试.最近错误
        && !错误.is_empty()
    {
        ui.label(RichText::new(错误).small().color(配色.错误));
    }
}

fn 绘制接入说明(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.add_space(6.0);
    ui.label(
        RichText::new(语言.取("runtime_debug_not_attached_title"))
            .strong()
            .color(配色.警告),
    );
    ui.label(
        RichText::new(语言.取("runtime_debug_not_attached_body"))
            .small()
            .color(配色.次要文字),
    );
    ui.add_space(4.0);
    ui.code("Breeze调试插件::自动()");
    ui.label(
        RichText::new(语言.取("runtime_debug_not_attached_steps"))
            .small()
            .color(配色.次要文字),
    );
    ui.add_space(4.0);
    if !调试.状态说明.is_empty() {
        ui.label(RichText::new(&调试.状态说明).small().color(配色.次要文字));
    }
    ui.label(
        RichText::new(语言.取("runtime_debug_security_note"))
            .small()
            .color(配色.次要文字),
    );
}

fn 绘制等待(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(RichText::new(语言.取("runtime_debug_waiting")).color(配色.文字));
    });
    if let Some(地址) = &调试.监听地址 {
        ui.label(
            RichText::new(format!("{}：{地址}", 语言.取("runtime_debug_loopback")))
                .small()
                .color(配色.次要文字),
        );
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(语言.取("runtime_debug_waiting_hint"))
            .small()
            .color(配色.次要文字),
    );
    ui.code("Breeze调试插件::自动()");
    ui.label(
        RichText::new(语言.取("runtime_debug_waiting_not_failure"))
            .small()
            .color(配色.警告),
    );
}

fn 绘制不可用(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.label(
        RichText::new(语言.取(match 调试.状态 {
            调试连接状态::不兼容 => "runtime_debug_incompatible_title",
            _ => "runtime_debug_failed_title",
        }))
        .strong()
        .color(配色.错误),
    );
    if !调试.状态说明.is_empty() {
        ui.label(RichText::new(&调试.状态说明).small().color(配色.错误));
    }
    ui.label(
        RichText::new(语言.取("runtime_debug_unavailable_hint"))
            .small()
            .color(配色.次要文字),
    );
}

fn 绘制数据区(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;

    if let Some(概览) = &调试.世界概览 {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    语言.取("runtime_debug_entities"),
                    概览.实体总数
                ))
                .monospace(),
            );
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    语言.取("runtime_debug_component_types"),
                    概览.组件类型数
                ))
                .monospace(),
            );
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    语言.取("runtime_debug_registered_checks"),
                    概览.已注册组件检查器
                ))
                .small()
                .color(配色.次要文字),
            );
            if let Some(时间) = 调试.最近采集时间毫秒 {
                ui.label(
                    RichText::new(format!(
                        "{} #{} · {}",
                        语言.取("runtime_debug_snapshot"),
                        概览.快照序号,
                        时间标签(时间)
                    ))
                    .small()
                    .color(配色.次要文字),
                );
            }
            if 概览.帧耗时微秒 > 0 {
                ui.label(
                    RichText::new(format!(
                        "{} {:.2} ms",
                        语言.取("runtime_debug_frame_time"),
                        概览.帧耗时微秒 as f64 / 1000.0
                    ))
                    .small()
                    .color(配色.次要文字),
                );
            }
            if !概览.资产.为空() {
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        语言.取("runtime_debug_asset_scale"),
                        概览.资产.摘要()
                    ))
                    .small()
                    .color(配色.次要文字),
                );
                // 规模细节：只说测得出来的东西，不写"内存占用"。
                let mut 细节 = Vec::new();
                if 概览.资产.网格顶点数 > 0 {
                    细节.push(format!(
                        "{} {}",
                        语言.取("runtime_debug_mesh_vertices"),
                        概览.资产.网格顶点数
                    ));
                }
                if 概览.资产.图片像素数 > 0 {
                    细节.push(format!(
                        "{} {}",
                        语言.取("runtime_debug_image_pixels"),
                        概览.资产.图片像素数
                    ));
                }
                if 概览.资产.音频字节数 > 0 {
                    细节.push(format!(
                        "{} {}",
                        语言.取("runtime_debug_audio_bytes"),
                        editor_core::package::人类可读字节(概览.资产.音频字节数)
                    ));
                }
                if !细节.is_empty() {
                    ui.label(
                        RichText::new(细节.join(" · "))
                            .small()
                            .monospace()
                            .color(配色.次要文字),
                    );
                }
            }
            if let Some(耗时) = 调试.最近请求耗时微秒 {
                ui.label(
                    RichText::new(format!(
                        "{} {:.2} ms",
                        语言.取("runtime_debug_request_latency"),
                        耗时 as f64 / 1000.0
                    ))
                    .small()
                    .color(配色.次要文字),
                );
            }
            if 调试.断开次数 > 0 {
                ui.label(
                    RichText::new(format!(
                        "{} {} / {}",
                        语言.取("runtime_debug_connection_counts"),
                        调试.断开次数,
                        调试.重连次数
                    ))
                    .small()
                    .color(配色.警告),
                );
            }
        });
    }

    ui.separator();
    绘制查询条(ui, 上下文, 调试);
    绘制实体列表(ui, 上下文, 调试);
    ui.separator();
    绘制实体详情(ui, 上下文, 调试);
    绘制关联区(ui, 上下文, 调试);
    绘制快照区(ui, 上下文, 调试);
    绘制修改记录(ui, 上下文, 调试);
    ui.separator();
    绘制资源区(ui, 上下文, 调试);
}

fn 绘制查询条(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    ui.horizontal_wrapped(|ui| {
        ui.label(语言.取("runtime_debug_search"));
        ui.add(
            egui::TextEdit::singleline(&mut 上下文.界面.运行时搜索)
                .desired_width(150.0)
                .hint_text(语言.取("runtime_debug_search_hint")),
        );
        ui.label(语言.取("runtime_debug_component_filter"));
        ui.add(
            egui::TextEdit::singleline(&mut 上下文.界面.运行时组件过滤)
                .desired_width(120.0)
                .hint_text("Transform"),
        );
        if ui.button(语言.取("runtime_debug_query")).clicked() {
            上下文.界面.运行时页 = 0;
            查询(上下文);
        }
        if ui.button(语言.取("runtime_debug_clear_filter")).clicked() {
            上下文.界面.运行时搜索.clear();
            上下文.界面.运行时组件过滤.clear();
            上下文.界面.运行时页 = 0;
            查询(上下文);
        }
    });

    let (总数, 已截断) = 调试
        .实体页
        .as_ref()
        .map(|页| (页.总数, 页.已截断))
        .unwrap_or((0, false));
    let 页数 = ((总数 + u64::from(上下文.界面.运行时每页) - 1)
        / u64::from(上下文.界面.运行时每页.max(1)))
    .max(1);
    let 当前 = u64::from(上下文.界面.运行时页) + 1;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(上下文.界面.运行时页 > 0, egui::Button::new("◀"))
            .on_hover_text(语言.取("runtime_debug_prev_page"))
            .clicked()
        {
            上下文.界面.运行时页 = 上下文.界面.运行时页.saturating_sub(1);
            查询(上下文);
        }
        ui.label(
            RichText::new(format!("{当前} / {页数}"))
                .small()
                .color(上下文.配色.次要文字),
        );
        if ui
            .add_enabled(当前 < 页数, egui::Button::new("▶"))
            .on_hover_text(语言.取("runtime_debug_next_page"))
            .clicked()
        {
            上下文.界面.运行时页 += 1;
            查询(上下文);
        }
        ui.label(
            RichText::new(format!(
                "{} {总数}",
                语言.取("runtime_debug_filtered_total")
            ))
            .small()
            .color(上下文.配色.次要文字),
        );
        if 已截断 {
            ui.label(
                RichText::new(语言.取("runtime_debug_truncated"))
                    .small()
                    .color(上下文.配色.警告),
            );
        }
        let 显示不支持标签 = 上下文.语言.取("runtime_debug_show_unsupported");
        ui.checkbox(&mut 上下文.界面.运行时显示不支持, 显示不支持标签);
    });
}

fn 查询(上下文: &mut 绘制上下文<'_>) {
    上下文.动作.推(UiAction::查询运行时实体 {
        页: 上下文.界面.运行时页,
        每页: 上下文.界面.运行时每页,
        搜索: 上下文.界面.运行时搜索.clone(),
        组件: 上下文.界面.运行时组件过滤.clone(),
    });
}

fn 绘制实体列表(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let Some(页) = &调试.实体页 else {
        ui.label(
            RichText::new(语言.取("runtime_debug_no_entity_page"))
                .small()
                .color(配色.次要文字),
        );
        return;
    };
    if 页.实体.is_empty() {
        ui.label(
            RichText::new(语言.取("runtime_debug_no_entities"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }
    let 选中 = 调试.实体详情.as_ref().map(|详情| 详情.标识);
    let mut 待选中 = None;
    egui::ScrollArea::vertical()
        .id_salt("runtime_debug_entity_list")
        .max_height(220.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for 实体 in &页.实体 {
                let 标签 = match &实体.名称 {
                    Some(名称) => format!("{}  {}", 短标识十进制(实体.标识), 名称),
                    None => 短标识十进制(实体.标识),
                };
                let 摘要 = if 实体.摘要.is_empty() {
                    String::new()
                } else {
                    format!("  ·  {}", 实体.摘要.join(", "))
                };
                let 是选中 = 选中 == Some(实体.标识);
                if ui
                    .selectable_label(是选中, format!("{标签}{摘要}"))
                    .on_hover_text(format!(
                        "{} {}\n{} {}",
                        语言.取("runtime_debug_id"),
                        实体.标识,
                        语言.取("runtime_debug_components"),
                        实体.组件数
                    ))
                    .clicked()
                {
                    待选中 = Some(实体.标识);
                }
            }
        });
    if let Some(实体) = 待选中 {
        上下文.动作.推(UiAction::选中运行时实体 { 实体 });
    }
    ui.label(
        RichText::new(format!(
            "{} {} / {}",
            语言.取("runtime_debug_show_range"),
            (页.实体.len() as u64).min(页.总数),
            页.总数
        ))
        .small()
        .color(配色.次要文字),
    );
}

fn 绘制实体详情(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.label(
        RichText::new(语言.取("runtime_debug_entity_detail"))
            .strong()
            .color(配色.文字),
    );
    let Some(详情) = &调试.实体详情 else {
        ui.label(
            RichText::new(if 调试.查询中 {
                语言.取("runtime_debug_loading_detail")
            } else {
                语言.取("runtime_debug_select_entity")
            })
            .small()
            .color(配色.次要文字),
        );
        return;
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("{} {}", 语言.取("runtime_debug_id"), 详情.标识)).monospace(),
        );
        if let Some(名称) = &详情.名称 {
            ui.label(RichText::new(名称).strong());
        }
        let (可检查, 部分, 不支持) = 调试.组件统计();
        ui.label(
            RichText::new(format!(
                "{} {} · {} {} · {} {}",
                语言.取("runtime_debug_supported"),
                可检查,
                语言.取("runtime_debug_partial"),
                部分,
                语言.取("runtime_debug_unsupported"),
                不支持
            ))
            .small()
            .color(配色.次要文字),
        );
        if ui.button(语言.取("runtime_debug_copy_entity")).clicked() {
            ui.ctx().copy_text(实体摘要文本(语言, 详情));
        }
        if ui.button(语言.取("runtime_debug_copy_detail")).clicked() {
            ui.ctx().copy_text(实体详情文本(详情));
        }
        ui.label(
            RichText::new(format!(
                "{} #{}",
                语言.取("runtime_debug_snapshot"),
                详情.快照序号
            ))
            .small()
            .color(配色.次要文字),
        );
    });

    let 显示不支持 = 上下文.界面.运行时显示不支持;
    let mut 展开过 = 0usize;
    for 组件 in &详情.组件 {
        if 组件.支持 == 检查支持级别::不支持 && !显示不支持 {
            continue;
        }
        let 标题 = format!("{}  [{}]", 组件.类型名, 语言.取(组件.支持.词条键()));
        let 默认展开 = 组件.支持 == 检查支持级别::可检查 && 展开过 < 4;
        if 默认展开 {
            展开过 += 1;
        }
        egui::CollapsingHeader::new(标题)
            .id_salt(("runtime_debug_component", &组件.类型名))
            .default_open(默认展开)
            .show(ui, |ui| {
                if 组件.字段.is_empty() {
                    if let Some(说明) = &组件.说明 {
                        ui.label(RichText::new(说明).small().color(配色.警告));
                    } else {
                        ui.label(
                            RichText::new(语言.取("runtime_debug_no_fields"))
                                .small()
                                .color(配色.次要文字),
                        );
                    }
                    return;
                }
                for 字段 in &组件.字段 {
                    绘制字段行(
                        ui,
                        上下文,
                        详情.标识,
                        &组件.类型名,
                        字段,
                        调试.是否实时(),
                        调试.修改中,
                    );
                }
                if let Some(说明) = &组件.说明 {
                    ui.label(RichText::new(说明).small().color(配色.次要文字));
                }
            });
    }
}

/// 绘制一行字段：可写字段提供编辑框与明确的应用/取消，只读字段保持只读。
fn 绘制字段行(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    实体: u64,
    组件: &str,
    字段: &字段视图,
    实时: bool,
    修改中: bool,
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 键 = 草稿键(实体, 组件, &字段.名称, None);
    ui.horizontal_wrapped(|ui| {
        let 标题 = if 字段.可写 {
            format!("✎ {}", 字段.名称)
        } else {
            字段.名称.clone()
        };
        ui.label(RichText::new(标题).small().color(if 字段.可写 {
            配色.链接
        } else {
            配色.次要文字
        }));
        if !字段.可写 {
            ui.label(
                RichText::new(字段.值.显示文本())
                    .monospace()
                    .color(配色.文字),
            );
            return;
        }

        let 可编辑 = 实时 && !修改中;
        let 当前 = 字段.值.显示文本();
        let mut 草稿 = 上下文
            .界面
            .运行时草稿
            .get(&键)
            .cloned()
            .unwrap_or_else(|| 当前.clone());
        let 响应 = ui.add_enabled(
            可编辑,
            egui::TextEdit::singleline(&mut 草稿)
                .desired_width(180.0)
                .hint_text(当前),
        );
        let 文字 = 语言.取("runtime_debug_edit_apply");
        let 应用 = ui.add_enabled(可编辑, egui::Button::new(文字)).clicked();
        let 取消 = ui.button(语言.取("runtime_debug_edit_cancel")).clicked();
        if 取消 {
            上下文.界面.运行时草稿.remove(&键);
            上下文.界面.运行时草稿错误.remove(&键);
            return;
        }
        if 响应.changed() {
            上下文.界面.运行时草稿.insert(键.clone(), 草稿.clone());
        }
        if 应用 {
            match 解析目标值(&字段.值, &草稿) {
                Ok(目标值) => {
                    上下文.界面.运行时草稿.remove(&键);
                    上下文.界面.运行时草稿错误.remove(&键);
                    上下文.动作.推(UiAction::提交运行时属性 {
                        实体,
                        组件: 组件.to_string(),
                        字段: 字段.名称.clone(),
                        索引: None,
                        目标值,
                        预期原值: 字段.值.clone(),
                    });
                }
                Err(错误) => {
                    上下文.界面.运行时草稿错误.insert(键.clone(), 错误);
                }
            }
        }
        if let Some(范围) = 字段.范围 {
            ui.label(
                RichText::new(format!(
                    "{} [{}, {}]",
                    语言.取("runtime_debug_edit_range"),
                    范围.最小,
                    范围.最大
                ))
                .small()
                .color(配色.次要文字),
            );
        }
        if !实时 {
            ui.label(
                RichText::new(语言.取("runtime_debug_edit_disabled"))
                    .small()
                    .color(配色.警告),
            );
        }
        if 修改中 {
            ui.label(
                RichText::new(语言.取("runtime_debug_edit_waiting"))
                    .small()
                    .color(配色.警告),
            );
        }
    });
    if let Some(错误) = 上下文.界面.运行时草稿错误.get(&键) {
        ui.label(RichText::new(错误).small().color(配色.错误));
    }
}

fn 草稿键(实体: u64, 组件: &str, 字段: &str, 索引: Option<u32>) -> String {
    format!(
        "{实体}|{组件}|{字段}|{}",
        索引
            .map(|下标| 下标.to_string())
            .unwrap_or_else(|| "-".to_string())
    )
}

/// 解析用户输入；向量必须保持原维度，且所有分量有限/在范围内。
fn 解析目标值(当前: &字段值, 文本: &str) -> Result<字段值, String> {
    match 当前 {
        字段值::浮点(_) => {
            let 值: f64 = 文本
                .trim()
                .parse()
                .map_err(|_| "请输入一个有限浮点数".to_string())?;
            if !值.is_finite() {
                return Err("数值必须是有限数（不能是 NaN/Infinity）".to_string());
            }
            Ok(字段值::浮点(值))
        }
        字段值::向量(当前值) => {
            let 清理 = 文本.trim().trim_start_matches('[').trim_end_matches(']');
            let 部分: Vec<&str> = 清理
                .split([',', ' ', '，'])
                .map(str::trim)
                .filter(|项| !项.is_empty())
                .collect();
            if 部分.len() != 当前值.len() {
                return Err(format!(
                    "需要 {} 个分量，实际输入 {} 个",
                    当前值.len(),
                    部分.len()
                ));
            }
            let mut 值 = Vec::with_capacity(部分.len());
            for 项 in 部分 {
                let 数: f64 = 项.parse().map_err(|_| format!("`{项}` 不是有限浮点数"))?;
                if !数.is_finite() {
                    return Err("向量分量不能是 NaN/Infinity".to_string());
                }
                值.push(数);
            }
            Ok(字段值::向量(值))
        }
        字段值::整数(_) => {
            let 值: i64 = 文本.trim().parse().map_err(|_| "请输入整数".to_string())?;
            Ok(字段值::整数(值))
        }
        字段值::文本(_) => Ok(字段值::文本(文本.to_string())),
        _ => Err("该字段类型当前不支持编辑".to_string()),
    }
}

fn 绘制快照区(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.label(
        RichText::new(语言.取("runtime_debug_snapshots"))
            .strong()
            .color(配色.文字),
    );
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                调试.是否实时(),
                egui::Button::new(语言.取("runtime_debug_take_snapshot")),
            )
            .clicked()
        {
            上下文.动作.推(UiAction::采集运行时快照);
        }
        if ui
            .add_enabled(
                调试.是否实时() && 上下文.调试历史.快照().len() >= 2,
                egui::Button::new(语言.取("runtime_debug_compare_snapshots")),
            )
            .clicked()
        {
            上下文.动作.推(UiAction::比较最近运行时快照);
        }
        if let Some(最近) = 上下文.调试历史.快照().last() {
            ui.label(
                RichText::new(format!(
                    "{} #{} · {} · {} {}",
                    语言.取("runtime_debug_latest_snapshot"),
                    最近.序号,
                    时间标签(最近.采集时间毫秒),
                    最近.实体.len(),
                    语言.取("runtime_debug_entities")
                ))
                .small()
                .color(配色.次要文字),
            );
            if 最近.已截断 {
                ui.label(
                    RichText::new(语言.取("runtime_debug_snapshot_truncated"))
                        .small()
                        .color(配色.警告),
                );
            }
        }
    });
    if 上下文.调试历史.快照().is_empty() {
        ui.label(
            RichText::new(语言.取("runtime_debug_no_snapshots"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }
    let mut 待选中 = None;
    egui::ScrollArea::vertical()
        .id_salt("runtime_debug_snapshot_history")
        .max_height(120.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for 快照 in 上下文.调试历史.快照().iter().rev() {
                let 标记 = if 快照.完整() { "" } else { " ⚠" };
                if ui
                    .selectable_label(
                        false,
                        format!(
                            "#{} · {} · {} entities · {} B{}",
                            快照.序号,
                            时间标签(快照.采集时间毫秒),
                            快照.实体.len(),
                            快照.估算字节,
                            标记
                        ),
                    )
                    .on_hover_text(format!(
                        "协议快照 #{}；运行实例 {}",
                        快照.协议快照序号,
                        快照.运行实例标识.as_deref().unwrap_or("未知")
                    ))
                    .clicked()
                {
                    // 点击历史里的实体不可直接定位；差异列表负责跳转。
                }
            }
        });
    if let Some(差异) = &上下文.调试历史.最近差异 {
        ui.label(RichText::new(差异.摘要文本()).small().color(if 差异.可比 {
            配色.文字
        } else {
            配色.错误
        }));
        for 变化 in 差异.字段变化.iter().take(20) {
            let 标签 = format!(
                "实体 {} · {} . {}：{} → {}",
                变化.实体,
                变化.组件.rsplit("::").next().unwrap_or(&变化.组件),
                变化.字段,
                变化.旧值.显示文本(),
                变化.新值.显示文本()
            );
            if ui.selectable_label(false, 标签).clicked() {
                待选中 = Some(变化.实体);
            }
        }
        for (实体, 组件) in 差异.新增组件.iter().take(10) {
            ui.label(
                RichText::new(format!("+ 实体 {实体} · {组件}"))
                    .small()
                    .color(配色.成功),
            );
        }
        for (实体, 组件) in 差异.移除组件.iter().take(10) {
            ui.label(
                RichText::new(format!("- 实体 {实体} · {组件}"))
                    .small()
                    .color(配色.警告),
            );
        }
        for 项 in 差异.无法比较.iter().take(6) {
            ui.label(RichText::new(项).small().color(配色.警告));
        }
        if !差异.字段变化.is_empty() && ui.button(语言.取("runtime_debug_copy_diff")).clicked()
        {
            ui.ctx().copy_text(差异文本(差异));
        }
    }
    if let Some(实体) = 待选中 {
        上下文.动作.推(UiAction::选中运行时实体 { 实体 });
    }
}

fn 绘制关联区(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.separator();
    ui.label(
        RichText::new(语言.取("runtime_debug_link_title"))
            .strong()
            .color(配色.文字),
    );
    let 场景选中 = 上下文.主选中().map(|标识| 标识.to_string());
    if let Some(关联) = &调试.关联 {
        ui.label(
            RichText::new(format!(
                "{} {} · {} · {}",
                语言.取("runtime_debug_link_scene_entity"),
                关联.场景实体,
                关联.来源,
                关联.置信度
            ))
            .small()
            .color(配色.文字),
        );
        if ui.button(语言.取("runtime_debug_link_clear")).clicked() {
            上下文.动作.推(UiAction::清除运行时关联);
        }
    } else {
        ui.label(
            RichText::new(语言.取("runtime_debug_link_none"))
                .small()
                .color(配色.次要文字),
        );
        match &场景选中 {
            Some(场景) => {
                if ui
                    .add_enabled(
                        调试.是否实时() && 调试.实体详情.is_some(),
                        egui::Button::new(语言.取("runtime_debug_link_to_selected")),
                    )
                    .clicked()
                {
                    上下文.动作.推(UiAction::关联运行时实体 {
                        场景实体: 场景.clone(),
                    });
                }
            }
            None => {
                ui.label(
                    RichText::new(语言.取("runtime_debug_link_select_scene"))
                        .small()
                        .color(配色.警告),
                );
            }
        }
    }
    ui.label(
        RichText::new(语言.取("runtime_debug_link_note"))
            .small()
            .color(配色.次要文字),
    );
}

fn 绘制修改记录(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.separator();
    ui.label(
        RichText::new(语言.取("runtime_debug_mutations"))
            .strong()
            .color(配色.文字),
    );
    if 调试.修改中 {
        ui.label(
            RichText::new(语言.取("runtime_debug_edit_waiting"))
                .small()
                .color(配色.警告),
        );
    }
    let 记录 = 上下文.调试历史.修改();
    if 记录.is_empty() {
        ui.label(
            RichText::new(语言.取("runtime_debug_no_mutations"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }
    for 项 in 记录.iter().rev().take(8) {
        let 颜色 = if !项.已确认 {
            配色.警告
        } else if 项.结果.成功 {
            配色.成功
        } else {
            配色.错误
        };
        ui.label(
            RichText::new(format!(
                "#{} {} . {}：{}",
                项.序号,
                项.组件,
                项.字段,
                项.结果说明()
            ))
            .small()
            .color(颜色),
        );
    }
    if ui.button(语言.取("runtime_debug_copy_mutations")).clicked() {
        let 文本 = 记录
            .iter()
            .rev()
            .map(|项| {
                format!(
                    "#{} {} . {} 目标={}；{}",
                    项.序号,
                    项.组件,
                    项.字段,
                    项.目标值.显示文本(),
                    项.结果说明()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        ui.ctx().copy_text(文本);
    }
}

fn 差异文本(差异: &editor_core::运行时快照差异) -> String {
    let mut 文本 = 差异.摘要文本();
    文本.push('\n');
    for 变化 in &差异.字段变化 {
        文本.push_str(&format!(
            "实体 {} · {} . {}：{} → {}\n",
            变化.实体,
            变化.组件,
            变化.字段,
            变化.旧值.显示文本(),
            变化.新值.显示文本()
        ));
    }
    文本
}

fn 绘制资源区(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 调试: &运行时调试快照
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    ui.label(
        RichText::new(语言.取("runtime_debug_resources"))
            .strong()
            .color(配色.文字),
    );
    if 调试.资源列表.is_empty() {
        ui.label(
            RichText::new(语言.取("runtime_debug_no_resources"))
                .small()
                .color(配色.次要文字),
        );
        return;
    }
    let mut 待选中 = None;
    ui.horizontal_wrapped(|ui| {
        for 资源 in &调试.资源列表 {
            let 选中 = 调试
                .资源详情
                .as_ref()
                .is_some_and(|详情| 详情.名称 == 资源.名称);
            let 标签 = format!(
                "{}（{}）",
                资源.名称,
                资源.类型名.rsplit("::").next().unwrap_or(&资源.类型名)
            );
            let 按钮 = ui.selectable_label(选中, 标签);
            let 按钮 = if 资源.存在 {
                按钮
            } else {
                按钮.on_hover_text(语言.取("runtime_debug_resource_absent"))
            };
            if 按钮.clicked() {
                待选中 = Some(资源.名称.clone());
            }
        }
    });
    if let Some(名称) = 待选中 {
        上下文.动作.推(UiAction::选中运行时资源 { 名称 });
    }
    if let Some(详情) = &调试.资源详情 {
        绘制资源详情(ui, 上下文, 详情);
    } else if let Some(名称) = &上下文.界面.运行时资源 {
        ui.label(
            RichText::new(format!(
                "{}：{名称}",
                语言.取("runtime_debug_resource_pending")
            ))
            .small()
            .color(配色.次要文字),
        );
    }
}

fn 绘制资源详情(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 详情: &资源详情) {
    let 配色 = 上下文.配色;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&详情.名称).strong());
        ui.label(
            RichText::new(&详情.类型名)
                .small()
                .monospace()
                .color(配色.次要文字),
        );
        if ui
            .button(上下文.语言.取("runtime_debug_copy_resource"))
            .clicked()
        {
            ui.ctx().copy_text(资源详情文本(详情));
        }
    });
    egui::Grid::new(("runtime_debug_resource_fields", &详情.名称))
        .num_columns(2)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            for 字段 in &详情.字段 {
                ui.label(RichText::new(&字段.名称).small().color(配色.次要文字));
                ui.label(RichText::new(字段.值.显示文本()).monospace());
                ui.end_row();
            }
        });
}

fn 实体摘要文本(语言: crate::i18n::语言, 详情: &实体详情) -> String {
    let mut 文本 = format!("{} {}\n", 语言.取("runtime_debug_id"), 详情.标识);
    if let Some(名称) = &详情.名称 {
        文本.push_str(&format!("{}: {名称}\n", 语言.取("runtime_debug_name")));
    }
    文本.push_str(&format!(
        "{}: {}\n",
        语言.取("runtime_debug_components"),
        详情.组件总数
    ));
    for 组件 in &详情.组件 {
        文本.push_str(&format!("- {} [{}]\n", 组件.类型名, 组件.支持.代号()));
    }
    文本
}

fn 实体详情文本(详情: &实体详情) -> String {
    let mut 文本 = format!("entity={} name={:?}\n", 详情.标识, 详情.名称);
    for 组件 in &详情.组件 {
        文本.push_str(&format!("\n{} [{}]\n", 组件.类型名, 组件.支持.代号()));
        for 字段 in &组件.字段 {
            文本.push_str(&format!("  {} = {}\n", 字段.名称, 字段.值.显示文本()));
        }
        if let Some(说明) = &组件.说明 {
            文本.push_str(&format!("  note: {说明}\n"));
        }
    }
    文本
}

fn 资源详情文本(详情: &资源详情) -> String {
    let mut 文本 = format!("resource={} type={}\n", 详情.名称, 详情.类型名);
    for 字段 in &详情.字段 {
        文本.push_str(&format!("  {} = {}\n", 字段.名称, 字段.值.显示文本()));
    }
    文本
}

/// 短标识：保留前 8 位，避免面板被长 UUID 撑爆。
fn 短标识(文本: &str) -> String {
    let 部分: String = 文本.chars().take(8).collect();
    if 文本.chars().count() > 8 {
        format!("{部分}…")
    } else {
        部分
    }
}

/// 实体 ID 显示：十进制为主，短十六进制便于对照游戏日志。
fn 短标识十进制(标识: u64) -> String {
    format!("{标识}  0x{标识:08x}")
}

fn 时间标签(毫秒: u64) -> String {
    // 这里不引第三方时间库；显示为 UNIX 毫秒的本地可读时间由调用方决定。
    // 面板只显示"多新鲜"的秒数没有可靠时钟，因此保留原始时间戳。
    format!("{毫秒} ms")
}

/// 供测试断言的纯函数：支持级别是否可展开。
pub fn 可展开(组件: &组件视图) -> bool {
    组件.支持 != 检查支持级别::不支持 && !组件.字段.is_empty()
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 不支持检查的组件不能伪造字段() {
        let 组件 = 组件视图 {
            类型名: "Secret".to_string(),
            支持: 检查支持级别::不支持,
            字段: Vec::new(),
            说明: Some("未注册".to_string()),
        };
        assert!(!可展开(&组件));
    }

    #[test]
    fn 短标识不切坏中文() {
        assert_eq!(短标识("短"), "短");
        assert_eq!(短标识("123456789"), "12345678…");
    }
}
