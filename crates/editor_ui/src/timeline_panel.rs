//! 时间轴面板：动画的播放头、轨道与关键帧。
//!
//! # 参考的 Godot 时间轴
//!
//! Godot 的动画编辑器（`editor/animation/animation_track_editor.cpp`）
//! 布局是：顶部播放控制 + 时间标尺，左侧轨道列表，右侧每条轨道的关键帧轨道。
//! 交互上它有两条**必须遵守**的规则，本面板照搬：
//!
//! 1. **播放头是拖动而不是输入**：用户要"拖着看"某一时刻的姿态，
//!    因此标尺区域支持按住拖动连续定位；
//! 2. **关键帧是点出来的**：在某条轨道上双击 = 在该时刻用实体**当前**变换
//!    录一帧。"先在属性面板改数值、再录一帧"是最常用的动画手势。
//!
//! # 为什么关键帧直接画在自绘矩形里
//!
//! 关键帧是"时间 → 位置"的映射，用 egui 的普通控件排不出来（不可拖动、
//! 无法按时间比例定位）。这里用 `allocate_exact_size` 拿一块矩形，
//! 用 `painter` 画标尺与菱形，命中测试自己算 —— 与视口 gizmo 同一套做法。

use editor_core::animation::{动画属性, 时间容差};
use egui::Ui;

use crate::state::{UiAction, 动画关键帧引用, 绘制上下文};

/// 轨道行的固定高度。
const 轨道行高: f32 = 22.0;
/// 左侧轨道名称列的宽度。
const 名称列宽: f32 = 190.0;
/// 标尺高度。
const 标尺高度: f32 = 20.0;
/// 关键帧菱形的半径。
const 菱形半径: f32 = 4.5;
/// 轨道区域的横向内边距（右侧留一点，避免最后一帧贴边）。
const 轨道右内边距: f32 = 8.0;

/// 绘制时间轴面板。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;

    // 先把本帧要用的动画事实一次取齐：
    // 其余绘制代码只读这些局部量，不再反复穿透 `编辑器.动画.as_ref()`。
    let (有动画, 播放中, 时间, 时长, 循环, 速度, 脏) = match 上下文.编辑器.动画.as_ref() {
        Some(会话) => (
            true,
            上下文.编辑器.动画播放.播放中,
            上下文.编辑器.动画播放.时间,
            会话.文档.时长,
            会话.文档.循环,
            上下文.编辑器.动画播放.速度,
            会话.有未保存修改(),
        ),
        None => (false, false, 0.0, 0.0, false, 1.0, false),
    };

    // ---- 顶部：文件与播放控制 ----
    //
    // # 千万别在 `horizontal_wrapped` 里用 `add_enabled_ui`
    //
    // `add_enabled_ui` 会创建**子 Ui**，而在可换行的横向布局里，子 Ui 的
    // 可用矩形是父 Ui 的整个剩余区域，于是它汇报的尺寸把这一行撑到
    // 150 多像素高：表现为工具栏下面出现一大片空白，后面所有内容被推到
    // 面板外面裁掉（时间轴第一版就是这样"标尺和轨道全都不见了"）。
    //
    // 正确做法是给**每个控件**用 `add_enabled`，它只画控件、不建子 Ui。
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui.button(语言.取("anim_new")).clicked() {
            上下文.动作.推(UiAction::新建动画);
        }
        let 有项目 = 上下文.编辑器.项目.is_some();
        if ui
            .add_enabled(有项目, egui::Button::new(语言.取("anim_open")))
            .clicked()
        {
            上下文.动作.推(UiAction::请求打开动画);
        }

        let (播放文本, 播放提示) = if 播放中 {
            (语言.取("anim_pause"), 语言.取("anim_pause_hint"))
        } else {
            (语言.取("anim_play"), 语言.取("anim_play_hint"))
        };
        if ui
            .add_enabled(有动画, egui::Button::new(播放文本))
            .on_hover_text(播放提示)
            .clicked()
        {
            上下文.动作.推(if 播放中 {
                UiAction::暂停动画
            } else {
                UiAction::播放动画
            });
        }
        if ui
            .add_enabled(有动画, egui::Button::new(语言.取("anim_stop")))
            .on_hover_text(语言.取("anim_stop_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::停止动画);
        }

        let mut 循环值 = 循环;
        if ui
            .add_enabled(
                有动画,
                egui::Checkbox::new(&mut 循环值, 语言.取("anim_loop")),
            )
            .on_hover_text(语言.取("anim_loop_hint"))
            .changed()
        {
            上下文.动作.推(UiAction::设置动画循环(循环值));
        }

        ui.label(语言.取("anim_speed"));
        let mut 速度值 = 速度;
        if ui
            .add_enabled(
                有动画,
                egui::DragValue::new(&mut 速度值)
                    .speed(0.05)
                    .range(0.1..=8.0),
            )
            .changed()
        {
            上下文.动作.推(UiAction::设置动画速度(速度值));
        }

        竖分隔(ui, 配色.分割线);

        if ui
            .add_enabled(
                有动画 && 有项目 && 脏,
                egui::Button::new(语言.取("anim_save")),
            )
            .on_hover_text(语言.取("anim_save_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::保存动画);
        }
        if ui
            .add_enabled(有动画 && 有项目, egui::Button::new(语言.取("anim_save_as")))
            .clicked()
        {
            上下文.动作.推(UiAction::请求动画另存为);
        }
        if ui
            .add_enabled(有动画, egui::Button::new(语言.取("anim_close")))
            .clicked()
        {
            上下文.动作.推(UiAction::关闭动画);
        }

        // 动画有自己的撤销历史（不共用场景撤销栈），入口也放在时间轴里，
        // 避免用户以为 Ctrl+Z 能撤销关键帧改动。
        竖分隔(ui, 配色.分割线);
        let (可撤销, 可重做) = 上下文
            .编辑器
            .动画
            .as_ref()
            .map(|会话| (会话.可撤销步数(), 会话.可重做步数()))
            .unwrap_or((0, 0));
        if ui
            .add_enabled(可撤销 > 0, egui::Button::new(语言.取("anim_undo")))
            .on_hover_text(语言.取("anim_undo_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::撤销动画);
        }
        if ui
            .add_enabled(可重做 > 0, egui::Button::new(语言.取("anim_redo")))
            .on_hover_text(语言.取("anim_redo_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::重做动画);
        }
    });

    if !有动画 {
        ui.separator();
        ui.label(
            egui::RichText::new(语言.取("anim_no_animation"))
                .small()
                .color(配色.次要文字),
        );
        // 上一轮动画可能还留在预览里；按钮仍可用，只是没有数据。
        return;
    }

    // 动画名 + 时长 + 播放头读数
    let (名称, 轨道列表) = match 上下文.编辑器.动画.as_ref() {
        Some(会话) => (会话.文档.显示名(语言), 会话.文档.轨道.clone()),
        None => return,
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(egui::RichText::new(名称).strong().color(配色.文字));
        if 脏 {
            ui.label(
                egui::RichText::new(语言.取("anim_unsaved"))
                    .small()
                    .color(配色.警告),
            );
        }
        竖分隔(ui, 配色.分割线);
        ui.label(语言.取("anim_duration"));
        let mut 时长值 = 时长;
        if ui
            .add(
                egui::DragValue::new(&mut 时长值)
                    .speed(0.05)
                    .range(editor_core::animation::最小时长..=editor_core::animation::最大时长)
                    .suffix(" s"),
            )
            .changed()
        {
            上下文.动作.推(UiAction::设置动画时长(时长值));
        }
        竖分隔(ui, 配色.分割线);
        // 播放头读数：同时显示"当前 / 总长"，避免用户不知道末尾在哪。
        ui.label(
            egui::RichText::new(语言.填充(
                "anim_time_of_total",
                &[&format!("{时间:.2}"), &format!("{时长:.2}")],
            ))
            .monospace()
            .small()
            .color(配色.次要文字),
        );
        ui.label(
            egui::RichText::new(语言.填充(
                "anim_keyframe_count",
                &[&轨道列表.iter().map(|轨道| 轨道.关键帧.len()).sum::<usize>()],
            ))
            .small()
            .color(配色.次要文字),
        );
    });

    // ---- 录制按钮 ----
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let 有选中 = 上下文.编辑器.选择.数量() > 0;
        // 同样避开 `add_enabled_ui`：见工具栏处的注释。
        if ui
            .add_enabled(
                有选中,
                egui::Button::new(语言.取("anim_record_translation")),
            )
            .on_hover_text(语言.取("anim_record_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::录制动画关键帧 {
                属性: vec![动画属性::平移],
            });
        }
        if ui
            .add_enabled(有选中, egui::Button::new(语言.取("anim_record_all")))
            .clicked()
        {
            上下文.动作.推(UiAction::录制动画关键帧 {
                属性: 动画属性::全部.to_vec(),
            });
        }
        if !有选中 {
            ui.label(
                egui::RichText::new(语言.取("anim_no_selection"))
                    .small()
                    .color(配色.次要文字),
            );
        }
    });

    ui.separator();

    // ---- 标尺 ----
    //
    // # 坐标系：所有绘制坐标都是**屏幕绝对坐标**
    //
    // egui 的 `Painter` 用的是屏幕坐标，不是面板内坐标。第一版把
    // `名称列宽 + 比例*轨道宽` 当成绝对 x 直接画，结果所有刻度都落在
    // 面板左侧几百像素之外、被裁剪矩形整片裁掉 —— 面板看起来"什么都没画"，
    // 实际上每条线、每个标签都确实被调用了。
    // 现在统一以**标尺/行自己的左边界**为基准，不会再漂。
    let 可用宽 = ui.available_width().max(名称列宽 + 80.0);
    let 轨道宽 = (可用宽 - 名称列宽 - 轨道右内边距).max(40.0);
    let 比例 = |时间: f32| -> f32 {
        if 时长 > f32::EPSILON {
            (时间 / 时长).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let 轨道x =
        |基准左: f32, 时间: f32| -> f32 { 基准左 + 名称列宽 + 比例(时间) * 轨道宽 };
    let 反解时间 = |基准左: f32, x: f32| -> f32 {
        let 比例 = ((x - 基准左 - 名称列宽) / 轨道宽).clamp(0.0, 1.0);
        比例 * 时长
    };

    // 标尺：刻度 + 播放头。
    let (标尺矩形, 标尺响应) =
        ui.allocate_exact_size(egui::vec2(可用宽, 标尺高度), egui::Sense::click_and_drag());
    let 绘制器 = ui.painter_at(标尺矩形);
    let 标尺左 = 标尺矩形.left();
    let 刻度数 = 刻度数量(时长);
    for 序号 in 0..=刻度数 {
        let 刻度时间 = 时长 * 序号 as f32 / 刻度数 as f32;
        let x = 轨道x(标尺左, 刻度时间);
        绘制器.line_segment(
            [
                egui::pos2(x, 标尺矩形.bottom() - 6.0),
                egui::pos2(x, 标尺矩形.bottom()),
            ],
            egui::Stroke::new(1.0, 配色.次要文字.gamma_multiply(0.6)),
        );
        绘制器.text(
            egui::pos2(x + 2.0, 标尺矩形.top() + 1.0),
            egui::Align2::LEFT_TOP,
            format!("{刻度时间:.1}"),
            egui::FontId::proportional(9.0),
            配色.次要文字,
        );
    }
    let 播放头x = 轨道x(标尺左, 时间);
    绘制器.line_segment(
        [
            egui::pos2(播放头x, 标尺矩形.top()),
            egui::pos2(播放头x, 标尺矩形.bottom()),
        ],
        egui::Stroke::new(2.0, 配色.警告),
    );

    if (标尺响应.dragged() || 标尺响应.clicked())
        && let Some(位置) = 标尺响应.interact_pointer_pos()
    {
        上下文.动作.推(UiAction::动画定位(反解时间(标尺左, 位置.x)));
    }

    // ---- 轨道列表 ----
    egui::ScrollArea::vertical()
        .id_salt("timeline_tracks")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // ---- 事件行 ----
            //
            // 事件不是"某个实体的某个属性"，因此不占轨道列表的一行，
            // 而是固定在轨道上方单独一行。双击添加、右键选类型，
            // 与关键帧的交互保持同一套（双击 = 在指针时刻创建）。
            let (事件行, 事件响应) = ui
                .allocate_exact_size(egui::vec2(可用宽, 轨道行高), egui::Sense::click());
            let 事件绘制 = ui.painter_at(事件行);
            let 事件左 = 事件行.left();
            let 事件轨道矩形 = egui::Rect::from_min_max(
                egui::pos2(事件左 + 名称列宽, 事件行.top() + 2.0),
                egui::pos2(事件左 + 名称列宽 + 轨道宽, 事件行.bottom() - 2.0),
            );
            事件绘制.rect_filled(事件轨道矩形, 2.0, 配色.面板次背景);
            事件绘制.text(
                egui::pos2(事件左 + 4.0, 事件行.center().y),
                egui::Align2::LEFT_CENTER,
                语言.取("anim_events"),
                egui::FontId::proportional(11.0),
                配色.文字,
            );
            let 事件列表 = 上下文
                .编辑器
                .动画
                .as_ref()
                .map(|会话| 会话.文档.事件.clone())
                .unwrap_or_default();
            let mut 命中事件: Option<f32> = None;
            for 事件 in &事件列表 {
                let x = 轨道x(事件左, 事件.时间);
                let 中心 = egui::pos2(x, 事件轨道矩形.center().y);
                // 事件用三角形，与关键帧的菱形区分开：一眼能看出这行是事件。
                let 三角 = vec![
                    中心 + egui::vec2(0.0, -菱形半径 - 1.0),
                    中心 + egui::vec2(菱形半径 + 1.0, 菱形半径),
                    中心 + egui::vec2(-菱形半径 - 1.0, 菱形半径),
                ];
                let 颜色 = match 事件.类型 {
                    editor_core::animation::事件类型::音效 => 配色.成功,
                    editor_core::animation::事件类型::逻辑 => 配色.强调,
                    editor_core::animation::事件类型::自定义 => 配色.次要文字,
                };
                事件绘制.add(egui::Shape::convex_polygon(三角, 颜色, egui::Stroke::NONE));
                if let Some(位置) = 事件响应.interact_pointer_pos()
                    && 事件响应.clicked()
                    && (位置.x - x).abs() <= 菱形半径 + 4.0
                {
                    命中事件 = Some(事件.时间);
                }
            }
            if let Some(时间) = 命中事件 {
                上下文.动作.推(UiAction::动画定位(时间));
            }
            // 双击事件行：在该时刻添加逻辑事件（最常用），类型可在右键里换。
            if 事件响应.double_clicked()
                && let Some(位置) = 事件响应.interact_pointer_pos()
                && 位置.x >= 事件左 + 名称列宽
            {
                let 时间 = 反解时间(事件左, 位置.x);
                上下文.动作.推(UiAction::动画定位(时间));
                上下文.动作.推(UiAction::添加动画事件 {
                    时间,
                    类型: editor_core::animation::事件类型::逻辑,
                });
            }
            事件响应.context_menu(|ui| {
                ui.label(语言.取("anim_events"));
                for 类型 in editor_core::animation::事件类型::全部 {
                    if ui.button(语言.取(类型.词条键())).clicked() {
                        let 时间 = 上下文.编辑器.动画播放.时间;
                        上下文.动作.推(UiAction::添加动画事件 { 时间, 类型 });
                        ui.close();
                    }
                }
                ui.separator();
                if ui
                    .button(egui::RichText::new(语言.取("anim_event_delete")).color(配色.错误))
                    .clicked()
                {
                    let 时间 = 上下文.编辑器.动画播放.时间;
                    上下文.动作.推(UiAction::删除动画事件 { 时间 });
                    ui.close();
                }
            });

            if 轨道列表.is_empty() {
                ui.label(
                    egui::RichText::new(语言.取("anim_no_tracks"))
                        .small()
                        .color(配色.次要文字),
                );
                return;
            }
            let 文档 = &上下文.编辑器.会话.文档;
            for 轨道 in &轨道列表 {
                let 实体名 = 文档
                    .取(轨道.实体)
                    .map(|实体| 实体.名称.clone())
                    .unwrap_or_else(|| 语言.取("anim_missing_entity").to_string());
                let 属性名 = 语言.取(轨道.属性.词条键());

                let (行矩形, 行响应) =
                    ui.allocate_exact_size(egui::vec2(可用宽, 轨道行高), egui::Sense::click());
                let 行绘制 = ui.painter_at(行矩形);
                let 行左 = 行矩形.left();
                let 轨道矩形 = egui::Rect::from_min_max(
                    egui::pos2(行左 + 名称列宽, 行矩形.top() + 2.0),
                    egui::pos2(行左 + 名称列宽 + 轨道宽, 行矩形.bottom() - 2.0),
                );
                行绘制.rect_filled(轨道矩形, 2.0, 配色.面板背景.gamma_multiply(1.15));
                let 行播放头x = 轨道x(行左, 时间);
                行绘制.line_segment(
                    [
                        egui::pos2(行播放头x, 轨道矩形.top()),
                        egui::pos2(行播放头x, 轨道矩形.bottom()),
                    ],
                    egui::Stroke::new(1.0, 配色.警告.gamma_multiply(0.7)),
                );
                行绘制.text(
                    egui::pos2(行左 + 4.0, 行矩形.center().y),
                    egui::Align2::LEFT_CENTER,
                    // 非默认插值在名字后面加标记（用词条名，不要写死中文）：
                    // 一眼能区分"卡顿"是数据（步进）还是掉帧。
                    if 轨道.插值 == editor_core::animation::插值模式::线性 {
                        format!("{实体名} · {属性名}")
                    } else {
                        format!("{实体名} · {属性名}（{}）", 语言.取(轨道.插值.词条键()))
                    },
                    egui::FontId::proportional(11.0),
                    配色.文字,
                );

                // 右键切换插值：线性 ↔ 步进。
                行响应.context_menu(|ui| {
                    ui.label(语言.取("interp_mode"));
                    for 模式 in editor_core::animation::插值模式::全部 {
                        let 选中 = 轨道.插值 == 模式;
                        if ui.selectable_label(选中, 语言.取(模式.词条键())).clicked() {
                            上下文.动作.推(UiAction::设置轨道插值 {
                                实体: 轨道.实体,
                                属性: 轨道.属性,
                                插值: 模式,
                            });
                            ui.close();
                        }
                    }
                    // 删除整条轨道：`UiAction::删除动画轨道` 早就实现了，却一直没有
                    // 界面入口（与"试听实体音频""移除最近项目"属于同一类问题）。
                    // 用分隔线隔开，避免和"切换插值"混成一片。
                    ui.separator();
                    if ui
                        .button(egui::RichText::new(语言.取("anim_delete_track")).color(配色.错误))
                        .clicked()
                    {
                        上下文.动作.推(UiAction::删除动画轨道 {
                            实体: 轨道.实体,
                            属性: 轨道.属性,
                        });
                        ui.close();
                    }
                });

                // 关键帧
                let mut 命中关键帧: Option<f32> = None;
                for 帧 in &轨道.关键帧 {
                    let x = 轨道x(行左, 帧.时间);
                    let 中心 = egui::pos2(x, 轨道矩形.center().y);
                    let 选中 = 上下文.界面.时间轴选中关键帧.is_some_and(|引用| {
                        引用.实体 == 轨道.实体
                            && 引用.属性 == 轨道.属性
                            && (引用.时间 - 帧.时间).abs() <= 时间容差
                    });
                    let 大小 = if 选中 {
                        菱形半径 + 1.5
                    } else {
                        菱形半径
                    };
                    let 菱形 = vec![
                        中心 + egui::vec2(0.0, -大小),
                        中心 + egui::vec2(大小, 0.0),
                        中心 + egui::vec2(0.0, 大小),
                        中心 + egui::vec2(-大小, 0.0),
                    ];
                    行绘制.add(egui::Shape::convex_polygon(
                        菱形,
                        if 选中 { 配色.警告 } else { 配色.成功 },
                        egui::Stroke::NONE,
                    ));
                    // 命中测试用"距离 + 时间"两个条件：只按屏幕距离会让
                    // 相邻很近的两帧抢同一个点。
                    if let Some(位置) = 行响应.interact_pointer_pos()
                        && 行响应.clicked()
                        && (位置.x - x).abs() <= 菱形半径 + 3.0
                        && 位置.y >= 轨道矩形.top()
                        && 位置.y <= 轨道矩形.bottom()
                    {
                        命中关键帧 = Some(帧.时间);
                    }
                }

                // 空白处点击 = 定位播放头；点在菱形上 = 选中该帧。
                if 行响应.clicked()
                    && let Some(位置) = 行响应.interact_pointer_pos()
                    && 位置.x >= 行左 + 名称列宽
                {
                    match 命中关键帧 {
                        Some(帧时间) => {
                            上下文.界面.时间轴选中关键帧 = Some(动画关键帧引用 {
                                实体: 轨道.实体,
                                属性: 轨道.属性,
                                时间: 帧时间,
                            });
                            上下文.动作.推(UiAction::动画定位(帧时间));
                        }
                        None => {
                            上下文.界面.时间轴选中关键帧 = None;
                            上下文.动作.推(UiAction::动画定位(反解时间(行左, 位置.x)));
                        }
                    }
                }

                // 双击：在该时刻用实体当前变换录一帧。
                if 行响应.double_clicked()
                    && let Some(位置) = 行响应.interact_pointer_pos()
                    && 位置.x >= 行左 + 名称列宽
                {
                    上下文.动作.推(UiAction::动画定位(反解时间(行左, 位置.x)));
                    上下文.动作.推(UiAction::录制动画关键帧 {
                        属性: vec![轨道.属性],
                    });
                    // 点名的实体可能与当前选择不同：把它选上，
                    // 否则"录一帧"会录到别的实体上，用户完全预期不到。
                    上下文.动作.推(UiAction::选择(crate::state::选择意图::单选(
                        轨道.实体,
                    )));
                }

                行响应.context_menu(|ui| {
                    if ui.button(语言.取("anim_delete_keyframe")).clicked() {
                        if let Some(选中) = 上下文.界面.时间轴选中关键帧 {
                            上下文.动作.推(UiAction::删除动画关键帧 {
                                实体: 选中.实体,
                                属性: 选中.属性,
                                时间: 选中.时间,
                            });
                            上下文.界面.时间轴选中关键帧 = None;
                        }
                        ui.close();
                    }
                    if ui.button(语言.取("anim_delete_track")).clicked() {
                        上下文.动作.推(UiAction::删除动画轨道 {
                            实体: 轨道.实体,
                            属性: 轨道.属性,
                        });
                        ui.close();
                    }
                });
            }
        });

    // ---- 底部：选中关键帧的详情 ----
    if let Some(选中) = 上下文.界面.时间轴选中关键帧
        && let Some(轨道) = 上下文
            .编辑器
            .动画
            .as_ref()
            .and_then(|会话| 会话.文档.查找轨道(选中.实体, 选中.属性))
        && let Some((_, 帧)) = 轨道
            .关键帧
            .iter()
            .enumerate()
            .find(|(_, 帧)| (帧.时间 - 选中.时间).abs() <= 时间容差)
    {
        ui.separator();
        let 值文本 = match 帧.值 {
            editor_core::animation::关键帧值::向量3(v) => {
                format!("({:.3}, {:.3}, {:.3})", v[0], v[1], v[2])
            }
            editor_core::animation::关键帧值::四元数(q) => {
                format!("quat({:.3}, {:.3}, {:.3}, {:.3})", q[0], q[1], q[2], q[3])
            }
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(语言.填充(
                    "anim_keyframe_detail",
                    &[&format!("{:.3}", 帧.时间), &值文本],
                ))
                .small()
                .monospace()
                .color(配色.次要文字),
            );
        });

        // 三次曲线才需要切线；其它插值下切线不参与求值，但会被保存
        // （用户切到三次时不必重填），因此这里只在三次时提供编辑入口。
        if 轨道.插值 == editor_core::animation::插值模式::三次 {
            if matches!(帧.值, editor_core::animation::关键帧值::向量3(_)) {
                绘制切线编辑(ui, 上下文, 选中, 帧);
            } else {
                ui.label(
                    egui::RichText::new(语言.取("anim_cubic_rotation_fallback"))
                        .small()
                        .color(配色.警告),
                );
            }
        }
    }
}

/// 绘制选中关键帧的入/出切线编辑器。
///
/// # 交互设计
///
/// * 每侧三个 `DragValue`（x/y/z 的速度），缺失切线时按 0 显示；
/// * "自动"按钮把该侧切线清空 —— 交给 Catmull-Rom 推算；
/// * "静止"按钮把该侧切线设为 0 —— 等价于 smoothstep 的端点行为。
///
/// 每次拖动都会推一条 `设置关键帧切线`，动作层会先记一步历史，
/// 因此拖动后按「撤销」能回到上一步，而不是一次性丢掉全部调整。
fn 绘制切线编辑(
    ui: &mut egui::Ui,
    上下文: &mut 绘制上下文<'_>,
    选中: crate::state::动画关键帧引用,
    帧: &editor_core::animation::关键帧,
) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    for (标题键, 当前, 是入) in [
        ("anim_tangent_in", 帧.入切线, true),
        ("anim_tangent_out", 帧.出切线, false),
    ] {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(语言.取(标题键))
                    .small()
                    .color(配色.次要文字),
            );
            let mut 值 = 当前.and_then(|切线| 切线.取向量3()).unwrap_or([0.0; 3]);
            let mut 改动 = false;
            for 分量 in &mut 值 {
                改动 |= ui
                    .add(
                        egui::DragValue::new(分量)
                            .speed(0.1)
                            .max_decimals(3)
                            .prefix(""),
                    )
                    .changed();
            }
            if 改动 {
                上下文.动作.推(UiAction::设置关键帧切线 {
                    实体: 选中.实体,
                    属性: 选中.属性,
                    时间: 选中.时间,
                    出: if 是入 {
                        帧.出切线
                    } else {
                        Some(editor_core::animation::切线::向量3(值))
                    },
                    入: if 是入 {
                        Some(editor_core::animation::切线::向量3(值))
                    } else {
                        帧.入切线
                    },
                });
            }
            if ui.small_button(语言.取("anim_tangent_auto")).clicked() {
                上下文.动作.推(UiAction::设置关键帧切线 {
                    实体: 选中.实体,
                    属性: 选中.属性,
                    时间: 选中.时间,
                    出: if 是入 { 帧.出切线 } else { None },
                    入: if 是入 { None } else { 帧.入切线 },
                });
            }
            if ui.small_button(语言.取("anim_tangent_flat")).clicked() {
                上下文.动作.推(UiAction::设置关键帧切线 {
                    实体: 选中.实体,
                    属性: 选中.属性,
                    时间: 选中.时间,
                    出: if 是入 {
                        帧.出切线
                    } else {
                        Some(editor_core::animation::切线::静止())
                    },
                    入: if 是入 {
                        Some(editor_core::animation::切线::静止())
                    } else {
                        帧.入切线
                    },
                });
            }
            if 当前.is_none() {
                ui.label(
                    egui::RichText::new(语言.取("anim_tangent_auto_label"))
                        .small()
                        .color(配色.次要文字),
                );
            }
        });
    }
}

/// 在横向排布里画一条**固定高度**的竖分隔线。
///
/// # 为什么不能用 `ui.separator()`
///
/// egui 的 `Separator` 在横向布局里会尝试撑满**父 Ui 的剩余高度**。
/// 在高度固定的顶部工具栏里这没问题，但在一个普通面板里，
/// 一次 `ui.separator()` 就会把这一行的高度撑到几百像素：
/// 表现是面板顶部出现一大片空白，紧接着的所有内容被推到下面并裁剪掉 ——
/// 时间轴最初就是这样"标尺和轨道全都不见了"，而代码里它们明明被调用了。
/// 自绘一条 16px 高的线没有这个副作用。
fn 竖分隔(ui: &mut Ui, 颜色: egui::Color32) {
    let (矩形, _) = ui.allocate_exact_size(egui::vec2(7.0, 16.0), egui::Sense::hover());
    let x = 矩形.center().x;
    ui.painter().line_segment(
        [
            egui::pos2(x, 矩形.top() + 2.0),
            egui::pos2(x, 矩形.bottom() - 2.0),
        ],
        egui::Stroke::new(1.0, 颜色),
    );
}

/// 标尺刻度数量：让刻度间距落在合理区间。
/// 固定 10 段会让 0.5 秒的动画每格 0.05 秒（标签挤成一团），
/// 3600 秒的动画反过来只有 10 个标签。这里按"每格至少 0.1 秒、
/// 至多 1 秒"选一个 1/2/5 × 10^n 的步长。
fn 刻度数量(时长: f32) -> usize {
    if 时长 <= f32::EPSILON {
        return 1;
    }
    const 目标格数: f32 = 8.0;
    let 粗略步长 = 时长 / 目标格数;
    let mut 步长 = 1.0f32;
    for 候选 in [
        0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0, 1800.0,
    ] {
        if 粗略步长 <= 候选 {
            步长 = 候选;
            break;
        }
        步长 = 候选;
    }
    ((时长 / 步长).round() as usize).clamp(1, 60)
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 刻度数量随时长增长而不会爆炸() {
        for 时长 in [0.1f32, 0.5, 2.0, 10.0, 60.0, 600.0, 3600.0] {
            let 数量 = 刻度数量(时长);
            assert!((1..=60).contains(&数量), "时长 {时长} 得到 {数量} 个刻度");
        }
        assert_eq!(刻度数量(0.0), 1, "零时长也要有一个刻度");
    }
}
