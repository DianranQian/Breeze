//! 资源预览：把选中资源的**真实内容**画出来。
//!
//! # 为什么单独一个模块
//!
//! 资源浏览器（[`crate::assets_panel`]）负责"有哪些资源"，这里负责
//! "选中的那个资源长什么样"。两者分开是因为预览的内容类型差异很大：
//! 图片是一张位图、模型是一张渲染结果、音频是可试听的波形信息、
//! 字体要用**它自己的**字体家族渲染样例、场景是一棵层级树、文本是只读内容。
//! 把这些塞进同一个绘制函数会让资源浏览器变成第二个"什么都做"的面板。
//!
//! # 数据来源
//!
//! 本模块**不做任何 IO**：它只读 [`crate::state::界面状态::资源预览`]，
//! 那份快照由 `editor_app` 的预览系统按 [`crate::state::预览请求`] 填好。
//! 这样界面层仍然不依赖 Bevy，而"读盘/解码/渲染"这些有副作用的事
//! 全部留在应用层，并且只在**选中变化**时发生一次。
//!
//! # 与"实体预览"的区别
//!
//! 预览显示的资源**不是**场景里已经创建的实体。这里没有任何入口
//! 会把预览内容写进文档；想把它加进场景，仍然要走右键菜单的
//! 「在场景中添加此资源」（那是一次显式、可撤销的编辑）。

use egui::{Color32, RichText, Sense, Ui};
use scene_doc::path::项目相对路径;
use scene_doc::property::资源类别;

use crate::state::{绘制上下文, 资源预览快照, 预览画面, 预览阶段};

/// 右侧预览区的最小宽度。
pub const 最小预览宽度: f32 = 220.0;
/// 右侧预览区的默认宽度。
pub const 默认预览宽度: f32 = 320.0;
/// 底部预览区的默认高度。
pub const 默认预览高度: f32 = 250.0;
/// 面板内可用宽度小于它时改成上下布局。
///
/// 资源浏览器在通用工作区里通常是"底部一条"，宽度够但高度只有 200 多像素，
/// 左右分栏会把两边都挤成细条。因此按**可用宽度**决定排布方向，
/// 而不是写死一种布局。
pub const 侧排最小宽度: f32 = 600.0;

/// 绘制资源预览区域。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 路径: &项目相对路径) {
    绘制标题栏(ui, 上下文);

    上下文.界面.布局快照.push(crate::state::区域快照 {
        标识: "asset_preview",
        矩形: ui.max_rect(),
    });

    let 快照 = 上下文.界面.资源预览.clone();
    // 只显示"属于当前请求"的结果：快速连续切换资源时，旧请求的
    // 结果可能晚一步才写回来，照着它画会让预览闪回上一个资源。
    if !快照.是最新(&上下文.界面.预览请求) {
        绘制加载中(ui, 上下文, Some(路径.文本()));
        return;
    }

    match 快照.阶段 {
        预览阶段::空闲 => 绘制空状态(ui, 上下文),
        预览阶段::加载中 => 绘制加载中(ui, 上下文, 快照.路径.as_deref()),
        预览阶段::失败 | 预览阶段::不支持 => 绘制终态(ui, 上下文, &快照),
        预览阶段::就绪 => {
            绘制元数据(ui, 上下文, &快照);
            ui.separator();
            绘制内容(ui, 上下文, &快照, 路径);
        }
    }
}

/// 标题栏：标题 + 重新加载 / 在文件管理器中显示。
///
/// # 为什么按钮单独占一行
///
/// 预览区在窄布局下只有 200 多逻辑像素宽。"标题 + 右对齐按钮"放在同一行时，
/// 按钮会把标题压成 `资…`（实测如此）。标题是这块区域的身份，
/// 被截断比按钮换行严重得多，因此按钮退到第二行并允许自动换行。
fn 绘制标题栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    ui.label(
        RichText::new(上下文.语言.取("asset_preview_title"))
            .strong()
            .color(上下文.配色.文字),
    );
    ui.horizontal_wrapped(|ui| {
        let 有选中 = 上下文.界面.资源选中.clone();
        let 重载 = ui
            .small_button(上下文.语言.取("asset_preview_reload"))
            .on_hover_text(上下文.语言.取("asset_preview_reload_hint"));
        if 重载.clicked() {
            上下文.界面.重新加载预览();
        }
        if let Some(路径) = 有选中
            && ui
                .small_button(上下文.语言.取("asset_preview_open_location"))
                .clicked()
        {
            上下文
                .动作
                .推(crate::state::UiAction::在资源管理器中显示(路径));
        }
    });
}

/// 未选中任何资源。
fn 绘制空状态(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    ui.add_space(12.0);
    ui.label(
        RichText::new(上下文.语言.取("asset_preview_idle"))
            .strong()
            .color(上下文.配色.次要文字),
    );
    ui.label(RichText::new(上下文.语言.取("asset_preview_idle_hint")).color(上下文.配色.次要文字));
}

/// 加载中。
fn 绘制加载中(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 路径: Option<&str>) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(
            RichText::new(上下文.语言.取("asset_preview_loading")).color(上下文.配色.次要文字),
        );
    });
    if let Some(路径) = 路径 {
        ui.label(RichText::new(路径).small().color(上下文.配色.次要文字));
    }
}

/// 失败 / 不支持：给出**明确**的原因，而不是空白。
fn 绘制终态(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 快照: &资源预览快照) {
    ui.add_space(8.0);
    let 阶段色 = if 快照.阶段 == 预览阶段::失败 {
        上下文.配色.错误
    } else {
        上下文.配色.警告
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("⚠").color(阶段色));
        ui.label(
            RichText::new(上下文.语言.取(快照.阶段.词条键()))
                .strong()
                .color(阶段色),
        );
    });
    if let Some(说明) = 快照.说明.as_deref() {
        ui.label(RichText::new(说明).color(上下文.配色.文字));
    }
    ui.add_space(4.0);
    绘制元数据(ui, 上下文, 快照);
    ui.add_space(6.0);
    if ui.button(上下文.语言.取("asset_preview_reload")).clicked() {
        上下文.界面.重新加载预览();
    }
}

/// 基本信息（路径/大小/修改时间）。
///
/// 信息密度刻意保持在三行：预览面板不是检查器，把几十个字段铺满
/// 会把真正的预览内容挤没。
fn 绘制元数据(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 快照: &资源预览快照) {
    let Some(路径) = 快照.路径.as_deref() else {
        return;
    };
    egui::Grid::new("资源预览元数据")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            ui.label(
                RichText::new(上下文.语言.取("asset_preview_path"))
                    .small()
                    .color(上下文.配色.次要文字),
            );
            ui.label(RichText::new(路径).small().color(上下文.配色.文字))
                .on_hover_text(路径);
            ui.end_row();

            ui.label(
                RichText::new(上下文.语言.取("asset_preview_size"))
                    .small()
                    .color(上下文.配色.次要文字),
            );
            ui.label(
                RichText::new(crate::assets_panel::人类可读大小(快照.大小))
                    .small()
                    .color(上下文.配色.文字),
            );
            ui.end_row();

            if let Some(时间) = 快照.修改时间 {
                ui.label(
                    RichText::new(上下文.语言.取("asset_preview_modified"))
                        .small()
                        .color(上下文.配色.次要文字),
                );
                ui.label(
                    RichText::new(格式化时间(时间))
                        .small()
                        .color(上下文.配色.文字),
                );
                ui.end_row();
            }
        });
}

/// 把 Unix 秒格式化成 `YYYY-MM-DD HH:MM`（UTC）。
///
/// 刻意不引入时间库：这里只需要一个稳定、可比对的显示值，
/// 而本地时区换算会让同一份证据在不同机器上出现不同文本。
fn 格式化时间(秒: u64) -> String {
    let 天 = 秒 / 86_400;
    let 余 = 秒 % 86_400;
    let (时, 分) = (余 / 3_600, (余 % 3_600) / 60);
    // 从 1970-01-01 起的民用历换算（Howard Hinnant 的 days_from_civil 逆运算）。
    let 调整 = 天 as i64 + 719_468;
    let 纪 = 调整.div_euclid(146_097);
    let 纪内 = 调整.rem_euclid(146_097);
    let 年偏移 = (纪内 - 纪内 / 1_460 + 纪内 / 36_524 - 纪内 / 146_096) / 365;
    let 年 = 年偏移 + 纪 * 400;
    let 年内 = 纪内 - (365 * 年偏移 + 年偏移 / 4 - 年偏移 / 100);
    let 月 = (5 * 年内 + 2) / 153;
    let 日 = 年内 - (153 * 月 + 2) / 5 + 1;
    let 月 = if 月 < 10 { 月 + 3 } else { 月 - 9 };
    let 年 = if 月 <= 2 { 年 + 1 } else { 年 };
    format!("{年:04}-{月:02}-{日:02} {时:02}:{分:02} UTC")
}

/// 按资源类型绘制真实预览内容。
fn 绘制内容(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    快照: &资源预览快照,
    路径: &项目相对路径,
) {
    if let Some(图片) = &快照.图片 {
        绘制图片(ui, 上下文, 快照.画面, 图片);
        return;
    }
    if let Some(模型) = &快照.模型 {
        绘制模型(ui, 上下文, 快照.画面, 模型);
        return;
    }
    if let Some(音频) = &快照.音频 {
        绘制音频(ui, 上下文, 音频, 路径);
        return;
    }
    if let Some(字体) = &快照.字体 {
        绘制字体(ui, 上下文, 字体);
        return;
    }
    if let Some(场景) = &快照.场景 {
        绘制场景(ui, 上下文, 场景);
        return;
    }
    if let Some(文本) = &快照.文本 {
        绘制文本(ui, 上下文, 文本);
        return;
    }
    // 到这里说明这一类资源只有元数据可用：明确说出来，不留空白。
    ui.add_space(6.0);
    ui.label(
        RichText::new(上下文.语言.取("asset_preview_metadata_only")).color(上下文.配色.次要文字),
    );
}

/// 图片：按可用区域等比缩放，透明图垫棋盘格。
fn 绘制图片(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    画面: Option<预览画面>,
    信息: &crate::state::图片预览信息,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(
                上下文
                    .语言
                    .填充("asset_preview_image_size", &[&信息.宽, &信息.高]),
            )
            .small()
            .color(上下文.配色.次要文字),
        );
        ui.label(
            RichText::new(信息.格式.to_uppercase())
                .small()
                .color(上下文.配色.次要文字),
        );
        ui.label(
            RichText::new(if 信息.有透明 {
                上下文.语言.取("asset_preview_image_alpha")
            } else {
                上下文.语言.取("asset_preview_image_no_alpha")
            })
            .small()
            .color(上下文.配色.次要文字),
        );
    });

    let Some(画面) = 画面 else {
        // 没有任何真实纹理时**不画占位图**：那会让用户以为预览成功了。
        ui.add_space(8.0);
        ui.label(RichText::new(上下文.语言.取("asset_preview_error")).color(上下文.配色.错误));
        return;
    };

    let 可用 = ui.available_size();
    if 可用.x < 8.0 || 可用.y < 8.0 {
        return;
    }
    let 原尺寸 = egui::vec2(画面.像素尺寸.0.max(1) as f32, 画面.像素尺寸.1.max(1) as f32);
    let 比例 = (可用.x / 原尺寸.x).min(可用.y / 原尺寸.y).min(4.0);
    let 显示 = egui::vec2((原尺寸.x * 比例).max(1.0), (原尺寸.y * 比例).max(1.0));
    let (矩形, _) = ui.allocate_exact_size(显示, Sense::hover());
    绘制棋盘格(ui.painter(), 矩形, 上下文);
    egui::Image::new(egui::load::SizedTexture::new(画面.纹理, 显示))
        .fit_to_exact_size(显示)
        .paint_at(ui, 矩形);
}

/// 模型：渲染结果 + 拖动旋转 + 滚轮缩放。
fn 绘制模型(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    画面: Option<预览画面>,
    信息: &crate::state::模型预览信息,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{} {}",
                上下文.语言.取("asset_preview_model_meshes"),
                信息.网格数
            ))
            .small()
            .color(上下文.配色.次要文字),
        );
        ui.label(
            RichText::new(format!(
                "{} {}",
                上下文.语言.取("asset_preview_model_materials"),
                信息.材质数
            ))
            .small()
            .color(上下文.配色.次要文字),
        );
        if let Some((宽, 高, 深)) = 信息.尺寸 {
            ui.label(
                RichText::new(format!(
                    "{} {:.2} × {:.2} × {:.2}",
                    上下文.语言.取("asset_preview_model_bounds"),
                    宽,
                    高,
                    深
                ))
                .small()
                .color(上下文.配色.次要文字),
            );
        }
        if ui
            .small_button(上下文.语言.取("asset_preview_model_reset"))
            .clicked()
        {
            上下文.界面.模型旋转 = (0.0, 0.0);
            上下文.界面.模型缩放 = 1.0;
        }
    });

    let 可用 = ui.available_size();
    if 可用.x < 8.0 || 可用.y < 8.0 {
        return;
    }
    let 边长 = 可用.x.min(可用.y);
    let (矩形, 响应) = ui.allocate_exact_size(egui::vec2(边长, 边长), Sense::click_and_drag());

    // 拖动改旋转、滚轮改缩放。这里只改**界面状态**，应用层下一帧
    // 据此重排预览相机，不会碰场景文档里的任何实体。
    if 响应.dragged() {
        let 增量 = 响应.drag_delta();
        上下文.界面.模型旋转.0 += 增量.x * 0.6;
        上下文.界面.模型旋转.1 = (上下文.界面.模型旋转.1 + 增量.y * 0.6).clamp(-89.0, 89.0);
    }
    if 响应.hovered() {
        let 滚轮 = ui.input(|输入| 输入.smooth_scroll_delta.y);
        if 滚轮.abs() > 0.0 {
            let 新 = 上下文.界面.模型缩放 * (滚轮 * 0.0015).exp();
            上下文.界面.模型缩放 = 新.clamp(0.3, 5.0);
        }
    }

    match 画面 {
        Some(画面) => {
            绘制棋盘格(ui.painter(), 矩形, 上下文);
            egui::Image::new(egui::load::SizedTexture::new(画面.纹理, 矩形.size()))
                .fit_to_exact_size(矩形.size())
                .paint_at(ui, 矩形);
        }
        None => {
            // 渲染资源还没就绪：如实说明，不画默认方块冒充模型。
            ui.painter()
                .rect_filled(矩形, egui::CornerRadius::same(4), 上下文.配色.输入框背景);
            ui.put(
                矩形,
                egui::Label::new(
                    RichText::new(上下文.语言.取("asset_preview_model_preparing"))
                        .color(上下文.配色.次要文字),
                ),
            );
        }
    }
    ui.label(
        RichText::new(上下文.语言.取("asset_preview_model_hint"))
            .small()
            .color(上下文.配色.次要文字),
    );
}

/// 音频：试听/停止 + 真实可读取的元数据。
fn 绘制音频(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    信息: &crate::state::音频预览信息,
    路径: &项目相对路径,
) {
    let 正在试听 = 上下文.界面.试听中.as_deref() == Some(路径.文本());
    ui.horizontal(|ui| {
        if 正在试听 {
            let 按钮 = ui.button(上下文.语言.取("asset_preview_audio_stop"));
            // 记录按钮矩形：自动验收要真的点它，而不是猜一个坐标。
            上下文.界面.布局快照.push(crate::state::区域快照 {
                标识: "asset_preview_audio",
                矩形: 按钮.rect,
            });
            if 按钮.clicked() {
                上下文.动作.推(crate::state::UiAction::停止试听);
            }
        } else if 信息.可试听 {
            let 按钮 = ui.button(上下文.语言.取("asset_preview_audio_play"));
            上下文.界面.布局快照.push(crate::state::区域快照 {
                标识: "asset_preview_audio",
                矩形: 按钮.rect,
            });
            if 按钮.clicked() {
                上下文
                    .动作
                    .推(crate::state::UiAction::试听资源(路径.clone()));
            }
        } else {
            ui.label(
                RichText::new(上下文.语言.取("asset_preview_audio_unsupported"))
                    .small()
                    .color(上下文.配色.警告),
            );
        }
    });

    let 未知 = || 上下文.语言.取("asset_preview_audio_unknown").to_string();
    egui::Grid::new("资源预览音频元数据")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            for (键, 值) in [
                (
                    "asset_preview_audio_duration",
                    信息
                        .时长秒
                        .map(|秒| format!("{秒:.2} s"))
                        .unwrap_or_else(未知),
                ),
                (
                    "asset_preview_audio_rate",
                    信息
                        .采样率
                        .map(|值| format!("{值} Hz"))
                        .unwrap_or_else(未知),
                ),
                (
                    "asset_preview_audio_channels",
                    信息.声道数.map(|值| 值.to_string()).unwrap_or_else(未知),
                ),
                (
                    "asset_preview_audio_bits",
                    信息.位深.map(|值| format!("{值} bit")).unwrap_or_else(未知),
                ),
            ] {
                ui.label(
                    RichText::new(上下文.语言.取(键))
                        .small()
                        .color(上下文.配色.次要文字),
                );
                ui.label(RichText::new(值).small().color(上下文.配色.文字));
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    ui.label(
        RichText::new(上下文.语言.取("asset_preview_audio_hint"))
            .small()
            .color(上下文.配色.次要文字),
    );
}

/// 字体：用**该字体自己的家族**渲染可编辑样例。
fn 绘制字体(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 信息: &crate::state::字体预览信息
) {
    if !信息.家族名.is_empty() {
        ui.label(
            RichText::new(format!(
                "{}: {}",
                上下文.语言.取("asset_preview_font_family"),
                信息.家族名
            ))
            .small()
            .color(上下文.配色.次要文字),
        );
    }
    if let Some(字形数) = 信息.字形数 {
        ui.label(
            RichText::new(format!(
                "{}: {}",
                上下文.语言.取("asset_preview_font_glyphs"),
                字形数
            ))
            .small()
            .color(上下文.配色.次要文字),
        );
    }

    // 样例文字可编辑：用户要测的是"这个字体渲染我的字是什么样"。
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(上下文.语言.取("asset_preview_font_sample"))
                .small()
                .color(上下文.配色.次要文字),
        );
        let mut 样例 = std::mem::take(&mut 上下文.界面.字体样例);
        let 响应 = ui.add(
            egui::TextEdit::singleline(&mut 样例)
                .desired_width(f32::INFINITY)
                .hint_text("Aa 汉字 0123"),
        );
        上下文.界面.字体样例 = 样例;
        if 响应.changed() {
            // 样例变了就要重新检查缺字：检查由应用层用真实字体文件做，
            // 界面只负责把新样例带过去。
            上下文.界面.重新加载预览();
        }
    });

    // 用真实家族渲染；家族尚未注册时如实退回默认字体，并说明原因。
    let 家族齐全 = !信息.家族键.is_empty();
    let 家族 = egui::FontFamily::Name(信息.家族键.clone().into());
    let 文字 = 上下文.界面.字体样例.clone();
    egui::Frame::NONE
        .fill(上下文.配色.输入框背景)
        .inner_margin(egui::Margin::same(8))
        .corner_radius(egui::CornerRadius::same(上下文.外观.圆角.round() as u8))
        .show(ui, |ui| {
            ui.set_min_height(56.0);
            let 字号 = (上下文.外观.基础字号 * 2.4).clamp(20.0, 44.0);
            if 家族齐全 {
                ui.label(
                    RichText::new(&文字)
                        .font(egui::FontId::new(字号, 家族))
                        .color(上下文.配色.文字),
                );
            } else {
                ui.label(RichText::new(&文字).size(字号).color(上下文.配色.文字));
            }
        });
    if !家族齐全 {
        ui.label(
            RichText::new(上下文.语言.取("asset_preview_font_failed"))
                .small()
                .color(上下文.配色.警告),
        );
    }
}

/// 场景：实体层级概览（预览，不改当前文档）。
fn 绘制场景(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 信息: &crate::state::场景预览信息
) {
    ui.label(
        RichText::new(上下文.语言.取("asset_preview_scene_preview_only"))
            .small()
            .color(上下文.配色.警告),
    );
    ui.horizontal_wrapped(|ui| {
        for (键, 值) in [
            ("asset_preview_scene_entities", 信息.实体总数),
            ("asset_preview_scene_roots", 信息.根实体数),
        ] {
            ui.label(
                RichText::new(format!("{}: {}", 上下文.语言.取(键), 值))
                    .small()
                    .color(上下文.配色.次要文字),
            );
        }
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for 实体 in &信息.实体 {
                ui.horizontal(|ui| {
                    ui.add_space(实体.深度 as f32 * 12.0);
                    ui.label("📄");
                    ui.label(RichText::new(&实体.名称).color(上下文.配色.文字));
                    ui.label(
                        RichText::new(format!("({})", 实体.组件数))
                            .small()
                            .color(上下文.配色.次要文字),
                    );
                });
            }
            if 信息.已截断 {
                ui.label(
                    RichText::new(
                        上下文
                            .语言
                            .填充("asset_preview_scene_truncated", &[&信息.实体.len()]),
                    )
                    .small()
                    .color(上下文.配色.次要文字),
                );
            }
        });
}

/// 文本：只读内容 + 行号。
fn 绘制文本(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 信息: &crate::state::文本预览信息
) {
    ui.label(
        RichText::new(
            上下文
                .语言
                .填充("asset_preview_text_lines", &[&信息.总行数]),
        )
        .small()
        .color(上下文.配色.次要文字),
    );
    if 信息.已截断 {
        ui.label(
            RichText::new(
                上下文
                    .语言
                    .填充("asset_preview_text_truncated", &[&信息.行.len()]),
            )
            .small()
            .color(上下文.配色.警告),
        );
    }
    ui.separator();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (序号, 行) in 信息.行.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:>4}", 序号 + 1))
                            .monospace()
                            .small()
                            .color(上下文.配色.次要文字),
                    );
                    // 只读：用可选中的 Label，而不是 TextEdit。
                    // 预览绝不能给用户"我在这里改了文件"的错觉。
                    ui.add(
                        egui::Label::new(RichText::new(行).monospace().color(上下文.配色.文字))
                            .selectable(true)
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
            }
        });
}

/// 透明背景的棋盘格。
///
/// 只有图片**真的带透明通道**时才需要它；但即便不透明，垫一层棋盘格
/// 也不会改变观感（图片会完全盖住它），因此这里不额外判断。
fn 绘制棋盘格(画布: &egui::Painter, 矩形: egui::Rect, 上下文: &绘制上下文<'_>) {
    let 亮 = if 上下文.主题.明暗 == crate::theme::明暗::深色 {
        Color32::from_rgb(58, 58, 62)
    } else {
        Color32::from_rgb(226, 226, 230)
    };
    let 暗 = if 上下文.主题.明暗 == crate::theme::明暗::深色 {
        Color32::from_rgb(44, 44, 48)
    } else {
        Color32::from_rgb(206, 206, 212)
    };
    画布.rect_filled(矩形, egui::CornerRadius::same(2), 亮);
    const 格: f32 = 10.0;
    let 列 = (矩形.width() / 格).ceil() as i32;
    let 行 = (矩形.height() / 格).ceil() as i32;
    for 行号 in 0..行 {
        for 列号 in 0..列 {
            if (行号 + 列号) % 2 == 1 {
                continue;
            }
            let 最小 = egui::pos2(矩形.min.x + 列号 as f32 * 格, 矩形.min.y + 行号 as f32 * 格);
            let 格矩形 = egui::Rect::from_min_size(最小, egui::vec2(格, 格)).intersect(矩形);
            画布.rect_filled(格矩形, egui::CornerRadius::ZERO, 暗);
        }
    }
}

/// 资源类别在界面上的显示名。
pub fn 类别名(类别: 资源类别, 语言: crate::i18n::语言) -> String {
    语言.取(类别.词条键()).to_string()
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 时间格式化是稳定且可读的() {
        // 1970-01-01 00:00 UTC
        assert_eq!(格式化时间(0), "1970-01-01 00:00 UTC");
        // 2024-02-29 12:34 UTC（闰日）
        assert_eq!(格式化时间(1_709_210_040), "2024-02-29 12:34 UTC");
    }

    #[test]
    fn 侧排阈值高于最小预览宽度() {
        // 否则会出现"左右分栏但右侧只有 200px"的挤压布局。
        // 用变量绕开 clippy 的 "assertion has a constant value"（常量断言
        // 会被优化掉消息，调试时看不出实际值）。
        let 阈值差 = 侧排最小宽度 - 最小预览宽度;
        assert!(阈值差 > 200.0, "阈值差只有 {阈值差}");
    }
}
