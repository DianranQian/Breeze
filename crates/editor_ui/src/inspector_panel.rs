//! 属性检查器面板。
//!
//! # 这个面板为什么不认识任何具体组件
//!
//! 本文件里找不到 `Sprite`、`Transform`、`Camera2d` 这类名字。所有字段的
//! 编辑界面都由 [`组件注册表`] 里的 [`属性描述`] 在运行时生成：
//! 描述说"这是个 `0.0..=1.0` 的浮点"，这里就画一个带范围的拖动条；
//! 描述说"这是个颜色"，这里就画颜色选择器。
//!
//! 于是支持一个新组件不需要动这个文件一行代码 —— 这正是需求里
//! "必须设计明确的组件注册机制、属性元数据和编辑器适配接口"的落地方式。
//!
//! [`组件注册表`]: scene_doc::registry::组件注册表
//! [`属性描述`]: scene_doc::registry::属性描述

use editor_core::state::编辑器状态;
use egui::{Color32, RichText, Ui};
use scene_doc::document::{变换数据, 场景实体, 实体标识};
use scene_doc::property::{属性值, 枚举值, 颜色值};
use scene_doc::registry::{属性描述, 属性种类};

use crate::state::{UiAction, 属性编辑, 绘制上下文};

/// 由四元数换算出的欧拉角（度），顺序为 XYZ。
///
/// 文档里存四元数（唯一规范表示），但用户需要用角度编辑，
/// 因此每次绘制时换算一次。这是两种表示之间的唯一桥接点。
fn 四元数转欧拉角(四元数: (f32, f32, f32, f32)) -> (f32, f32, f32) {
    let (x, y, z, w) = 四元数;
    // 归一化，避免累积误差放大成可见的角度抖动。
    let 模长 = (x * x + y * y + z * z + w * w).sqrt();
    if !模长.is_finite() || 模长 < 1e-6 {
        return (0.0, 0.0, 0.0);
    }
    let (x, y, z, w) = (x / 模长, y / 模长, z / 模长, w / 模长);

    let 弧度转角度 = 180.0 / std::f32::consts::PI;

    // 绕 X
    let 奇异 = (2.0 * (x * z - w * y)).abs() > 0.999_9;
    if 奇异 {
        let 俯仰 = -std::f32::consts::FRAC_PI_2 * (2.0 * (x * z - w * y)).signum();
        let 偏航 = (-2.0 * (y * z + w * x)).atan2(1.0 - 2.0 * (x * x + y * y));
        return (俯仰 * 弧度转角度, 0.0, 偏航 * 弧度转角度);
    }

    let 横滚 = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let 俯仰 = (2.0 * (w * y - z * x)).asin();
    let 偏航 = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    (横滚 * 弧度转角度, 俯仰 * 弧度转角度, 偏航 * 弧度转角度)
}

/// 由欧拉角（度，顺序 XYZ）换算四元数。
fn 欧拉角转四元数(角度: (f32, f32, f32)) -> (f32, f32, f32, f32) {
    let 角度转弧度 = std::f32::consts::PI / 180.0;
    let (横滚, 俯仰, 偏航) = (
        角度.0 * 角度转弧度 * 0.5,
        角度.1 * 角度转弧度 * 0.5,
        角度.2 * 角度转弧度 * 0.5,
    );
    let (sr, cr) = 横滚.sin_cos();
    let (sp, cp) = 俯仰.sin_cos();
    let (sy, cy) = 偏航.sin_cos();

    (
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
        cr * cp * cy + sr * sp * sy,
    )
}

/// 绘制检查器。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 编辑器 = 上下文.编辑器;

    let Some(标识) = 编辑器.选择.主选中() else {
        ui.add_space(12.0);
        ui.label(
            RichText::new(上下文.语言.取("inspector_no_entity_selected_hint"))
                .color(上下文.配色.次要文字),
        );
        return;
    };

    let Some(实体) = 编辑器.会话.文档.取(标识) else {
        ui.label(
            RichText::new(上下文.语言.取("the_selected_entity_no_longer_exists"))
                .color(上下文.配色.错误),
        );
        return;
    };

    // 先克隆出绘制所需的数据，避免在闭包里长期持有文档借用。
    let 实体快照: 场景实体 = 实体.clone();
    let 选择数量 = 编辑器.选择.数量();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            绘制头部(ui, 上下文, &实体快照, 选择数量);
            ui.separator();

            // ---- 变换 ----
            if let Some(变换) = 实体快照.变换 {
                绘制变换区(ui, 上下文, 标识, 变换);
                ui.separator();
            }

            // ---- 组件 ----
            绘制组件列表(ui, 上下文, 标识, &实体快照);
        });
}

/// 绘制头部：实体名、标识、组件添加按钮。
fn 绘制头部(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 实体: &场景实体, 选择数量: usize
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(上下文.语言.取("name")).color(上下文.配色.次要文字));
        let mut 名称 = 实体.名称.clone();
        let 响应 = ui.add(egui::TextEdit::singleline(&mut 名称).desired_width(160.0));
        if 响应.lost_focus() && ui.input(|输入| 输入.key_pressed(egui::Key::Enter)) {
            let 新名称 = 名称.trim().to_string();
            if !新名称.is_empty() && 新名称 != 实体.名称 {
                上下文.动作.推(UiAction::重命名实体 {
                    标识: 实体.标识,
                    新名称,
                });
            }
        }
    });

    if 选择数量 > 1 {
        ui.label(
            RichText::new(
                上下文
                    .语言
                    .填充("inspector_multi_selection_hint", &[&选择数量]),
            )
            .small()
            .color(上下文.配色.警告),
        );
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new(上下文.语言.取("id")).color(上下文.配色.次要文字));
        ui.label(
            RichText::new(实体.标识.to_string())
                .small()
                .monospace()
                .color(上下文.配色.次要文字),
        )
        .on_hover_text(上下文.语言.取("entity_id_stable_tooltip"));
    });

    ui.horizontal(|ui| {
        let 可添加组件 = 上下文.编辑器.注册表.数量() > 0;
        if ui
            .add_enabled(
                可添加组件,
                egui::Button::new(上下文.语言.取("add_component")),
            )
            .clicked()
        {
            上下文.界面.打开组件选择器 = !上下文.界面.打开组件选择器;
            上下文.界面.组件目标 = Some(实体.标识);
        }
        if ui.button(上下文.语言.取("duplicate_entity")).clicked() {
            上下文.动作.推(UiAction::复制实体(实体.标识));
        }
        if ui.button(上下文.语言.取("delete_entity")).clicked() {
            上下文.动作.推(UiAction::删除实体(实体.标识));
        }
    });

    // 组件选择器
    if 上下文.界面.打开组件选择器 && 上下文.界面.组件目标 == Some(实体.标识)
    {
        let 已用: Vec<String> = 实体.组件.iter().map(|组件| 组件.类型名.clone()).collect();
        let 候选: Vec<String> = 上下文
            .编辑器
            .注册表
            .组件名列表()
            .into_iter()
            .filter(|名字| !已用.contains(名字))
            .collect();

        egui::Frame::popup(ui.style()).show(ui, |ui| {
            if 候选.is_empty() {
                ui.label(
                    RichText::new(上下文.语言.取("inspector_all_components_added"))
                        .color(上下文.配色.次要文字),
                );
            }
            for 类型名 in 候选 {
                if ui.button(&类型名).clicked() {
                    上下文.动作.推(UiAction::添加组件 {
                        实体: 实体.标识,
                        类型名: 类型名.clone(),
                    });
                    上下文.界面.打开组件选择器 = false;
                }
            }
            if ui.button(上下文.语言.取("close")).clicked() {
                上下文.界面.打开组件选择器 = false;
            }
        });
    }
}

/// 绘制变换区。
fn 绘制变换区(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 标识: 实体标识, 变换: 变换数据
) {
    let 可折叠 = egui::CollapsingHeader::new(
        RichText::new(上下文.语言.取("transform"))
            .strong()
            .color(上下文.配色.文字),
    )
    .default_open(true)
    .id_salt("变换区");

    可折叠.show(ui, |ui| {
        let mut 新变换 = 变换;

        ui.horizontal(|ui| {
            ui.label(上下文.语言.取("position"));
            let 改变 = 三元拖动(ui, 上下文, "位置", &mut 新变换.平移, 0.05);
            if 改变 {
                提交变换(上下文, 标识, 新变换, true);
            }
        });

        // 旋转以角度编辑，内部仍存四元数。
        let 欧拉 = 四元数转欧拉角(变换.旋转);
        let mut 新欧拉 = 欧拉;
        ui.horizontal(|ui| {
            ui.label(上下文.语言.取("rotate"));
            if 三元拖动(ui, 上下文, "旋转", &mut 新欧拉, 0.5) {
                新变换.旋转 = 欧拉角转四元数(新欧拉);
                提交变换(上下文, 标识, 新变换, true);
            }
        });
        ui.label(
            RichText::new(上下文.语言.取("transform_rotation_units_hint"))
                .small()
                .color(上下文.配色.次要文字),
        );

        ui.horizontal(|ui| {
            ui.label(上下文.语言.取("transform_field_scale"));
            if 三元拖动(ui, 上下文, "缩放", &mut 新变换.缩放, 0.01) {
                提交变换(上下文, 标识, 新变换, true);
            }
        });

        // 缩放归零会让网格完全退化，给一个快捷修复。
        if 新变换.缩放.0 == 0.0 || 新变换.缩放.1 == 0.0 || 新变换.缩放.2 == 0.0 {
            ui.label(
                RichText::new(上下文.语言.取("inspector_scale_zero_warning"))
                    .small()
                    .color(上下文.配色.警告),
            );
        }

        if ui
            .button(上下文.语言.取("reset_to_identity_transform"))
            .clicked()
        {
            提交变换(上下文, 标识, 变换数据::单位(), false);
        }
    });
}

/// 三个浮点分量并排拖动。返回是否有改动。
///
/// 顺带把每个数值框的**真实矩形**记进布局快照：自动验收靠它来拖动数值框，
/// 而不是猜坐标。"连拖数值框产生一条撤销记录"这类协议必须用真实拖拽验证。
fn 三元拖动(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    标识前缀: &str,
    值: &mut (f32, f32, f32),
    步进: f32,
) -> bool {
    let mut 改变 = false;
    let 区域表: Option<[&'static str; 3]> = match 标识前缀 {
        "位置" => Some(["inspector_pos_x", "inspector_pos_y", "inspector_pos_z"]),
        "旋转" => Some(["inspector_rot_x", "inspector_rot_y", "inspector_rot_z"]),
        "缩放" => Some([
            "inspector_scale_x",
            "inspector_scale_y",
            "inspector_scale_z",
        ]),
        _ => None,
    };
    ui.push_id(标识前缀, |ui| {
        for (下标, 标签) in ["x", "y", "z"].iter().enumerate() {
            let 分量 = match 下标 {
                0 => &mut 值.0,
                1 => &mut 值.1,
                _ => &mut 值.2,
            };
            let 响应 = ui.add(
                egui::DragValue::new(分量)
                    .speed(步进)
                    .prefix(*标签)
                    .max_decimals(3),
            );
            if let Some(表) = 区域表 {
                上下文.界面.布局快照.push(crate::state::区域快照 {
                    标识: 表[下标],
                    矩形: 响应.rect,
                });
            }
            改变 |= 响应.changed();
        }
    });
    改变
}

/// 提交变换。
///
/// 必须在这里归一化旋转：用户按角度输入、以及四元数乘法累积，
/// 都会让四元数长度偏离 1，长时间编辑后表现为模型被剪切变形。
fn 提交变换(
    上下文: &mut 绘制上下文<'_>, 标识: 实体标识, mut 变换: 变换数据, 连续: bool
) {
    变换.归一化旋转();
    let 命令 = editor_core::command::编辑器命令::设变换 {
        标识,
        变换: Some(变换),
    };
    // 数值框拖拽是"连续编辑"：同一段拖拽内的逐帧改动会在撤销栈里合并成一步。
    // 而"重置为单位变换"这种一次性点击走普通入口，撤销时单独一步。
    if 连续 {
        上下文.动作.推(UiAction::执行命令连续(命令));
    } else {
        上下文.动作.推(UiAction::执行命令(命令));
    }
}

/// 绘制组件列表。
fn 绘制组件列表(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 标识: 实体标识, 实体: &场景实体
) {
    if 实体.组件.is_empty() {
        ui.add_space(6.0);
        ui.label(
            RichText::new(上下文.语言.取("inspector_entity_has_no_components_hint"))
                .color(上下文.配色.次要文字),
        );
        return;
    }

    // 先把"要画的组件"收集成一份值快照。
    // 之所以克隆而不是借用：下面要在闭包里推动作（需要 &mut 上下文），
    // 同时又要读注册表（借用 &上下文.编辑器），
    // 直接借用会让借用检查器无法同时满足两者。
    struct 待绘组件 {
        类型名: String,
        字段描述: Vec<属性描述>,
        属性: Vec<(String, 属性值)>,
        已注册: bool,
    }

    let 待绘: Vec<待绘组件> = 实体
        .组件
        .iter()
        .map(|组件| {
            let 描述 = 上下文.编辑器.注册表.查询(&组件.类型名);
            待绘组件 {
                类型名: 组件.类型名.clone(),
                字段描述: 描述
                    .map(|描述| 描述.字段描述().to_vec())
                    .unwrap_or_default(),
                属性: 组件
                    .属性
                    .iter()
                    .map(|(名字, 值)| (名字.clone(), 值.clone()))
                    .collect(),
                已注册: 描述.is_some(),
            }
        })
        .collect();

    for 组件 in 待绘 {
        绘制单个组件(
            ui,
            上下文,
            标识,
            &组件.类型名,
            &组件.字段描述,
            &组件.属性,
            组件.已注册,
        );
    }
}

/// 绘制一个组件。
#[allow(clippy::too_many_arguments)]
fn 绘制单个组件(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    标识: 实体标识,
    类型名: &str,
    字段描述: &[属性描述],
    属性: &[(String, 属性值)],
    已注册: bool,
) {
    egui::CollapsingHeader::new(RichText::new(类型名).strong().color(上下文.配色.强调))
        .default_open(true)
        .id_salt(format!("组件-{类型名}"))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .small_button(上下文.语言.取("remove_component_2"))
                    .clicked()
                {
                    上下文.动作.推(UiAction::删除组件 {
                        实体: 标识,
                        类型名: 类型名.to_string(),
                    });
                }
                if !已注册 {
                    ui.label(
                        RichText::new(上下文.语言.取("inspector_component_unregistered_hint"))
                            .small()
                            .color(上下文.配色.警告),
                    );
                }
            });

            if 字段描述.is_empty() {
                // 未注册组件：退化成"按值的种类推断控件"的通用编辑。
                // 这样即使用户用新版编辑器打开了含新组件的场景，
                // 也仍能改动它的值，而不是只能干看着。
                for (名字, 值) in 属性 {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(名字).color(上下文.配色.次要文字));
                        let mut 新值 = 值.clone();
                        if 通用值编辑(ui, 名字, &mut 新值, 上下文.语言) {
                            提交属性(上下文, 标识, 类型名, 名字, 新值);
                        }
                    });
                }
                if 属性.is_empty() {
                    ui.label(
                        RichText::new(上下文.语言.取("no_properties")).color(上下文.配色.次要文字),
                    );
                }
                return;
            }

            for 描述 in 字段描述 {
                let 当前 = 属性
                    .iter()
                    .find(|(名字, _)| 名字 == &描述.字段名)
                    .map(|(_, 值)| 值.clone())
                    .unwrap_or_else(|| 描述.默认值.clone());

                ui.horizontal(|ui| {
                    // 字段名与悬停提示。提示内容由 `格式提示` 统一组装，
                    // 因此这里不需要按"提示是否为空"分叉 —— 之前那样写
                    // 会出现两个完全相同的分支。
                    ui.label(RichText::new(上下文.语言.取(描述.显示名)).color(上下文.配色.文字))
                        .on_hover_text(格式提示(描述, 上下文.语言));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if 描述.只读 {
                            ui.label(
                                RichText::new(值摘要(&当前, 上下文.语言))
                                    .monospace()
                                    .color(上下文.配色.次要文字),
                            );
                            return;
                        }
                        let mut 新值 = 当前.clone();
                        if 按描述编辑(ui, &描述.字段名, &描述.种类, &mut 新值, 上下文)
                            && 新值 != 当前
                        {
                            新值.收拢();
                            提交属性(上下文, 标识, 类型名, &描述.字段名, 新值);
                        }
                    });
                });
            }
        });
}

/// 组合悬停提示。
fn 格式提示(描述: &属性描述, 语言: crate::i18n::语言) -> String {
    let 类型 = 描述.种类.类型名();
    if 描述.提示.is_empty() {
        语言.填充("type", &[&描述.字段名, &类型])
    } else {
        语言.填充(
            "tooltip_field_name_and_type",
            &[&语言.取(描述.提示), &描述.字段名, &类型],
        )
    }
}

/// 提交一个属性修改。
fn 提交属性(
    上下文: &mut 绘制上下文<'_>, 实体: 实体标识, 组件: &str, 属性: &str, 新值: 属性值
) {
    上下文.动作.推(UiAction::修改属性(属性编辑 {
        实体,
        组件: 组件.to_string(),
        属性: 属性.to_string(),
        新值,
    }));
}

/// 按属性描述生成对应控件。返回控件是否被改动。
fn 按描述编辑(
    ui: &mut Ui,
    字段名: &str,
    种类: &属性种类,
    值: &mut 属性值,
    上下文: &绘制上下文<'_>,
) -> bool {
    match 种类 {
        属性种类::布尔 => {
            let mut 开关 = 值.取布尔().unwrap_or(false);
            let 改变 = ui.checkbox(&mut 开关, "").changed();
            if 改变 {
                *值 = 属性值::布尔(开关);
            }
            改变
        }

        属性种类::整数 { 最小, 最大 } => {
            let mut 数值 = 值.取整数().unwrap_or(0);
            let mut 控件 = egui::DragValue::new(&mut 数值).speed(1.0);
            if let Some(小) = 最小 {
                控件 = 控件.range(*小..=最大.unwrap_or(i64::MAX));
            } else if let Some(大) = 最大 {
                控件 = 控件.range(i64::MIN..=*大);
            }
            let 改变 = ui.add(控件).changed();
            if 改变 {
                *值 = 属性值::整数(数值);
            }
            改变
        }

        属性种类::浮点 {
            最小, 最大, 步进
        } => {
            let mut 数值 = 值.取浮点().unwrap_or(0.0);
            let mut 控件 = egui::DragValue::new(&mut 数值).speed(*步进).max_decimals(4);
            let 下界 = 最小.unwrap_or(f32::NEG_INFINITY);
            let 上界 = 最大.unwrap_or(f32::INFINITY);
            if 下界.is_finite() || 上界.is_finite() {
                控件 = 控件.range(下界..=上界);
            }
            let 改变 = ui.add(控件).changed();
            if 改变 {
                *值 = 属性值::浮点(数值);
            }
            改变
        }

        属性种类::文本 => {
            let mut 文本 = 值.取文本().unwrap_or_default().to_string();
            let 改变 = ui
                .add(egui::TextEdit::singleline(&mut 文本).desired_width(180.0))
                .changed();
            if 改变 {
                *值 = 属性值::文本(文本);
            }
            改变
        }

        属性种类::二维 { 步进 } => {
            let (mut x, mut y) = 值.取二维().unwrap_or((0.0, 0.0));
            let mut 改变 = false;
            ui.push_id(format!("二维-{字段名}"), |ui| {
                改变 |= ui
                    .add(egui::DragValue::new(&mut x).speed(*步进).prefix("x"))
                    .changed();
                改变 |= ui
                    .add(egui::DragValue::new(&mut y).speed(*步进).prefix("y"))
                    .changed();
            });
            if 改变 {
                *值 = 属性值::二维(x, y);
            }
            改变
        }

        属性种类::三维 { 步进 } => {
            let (mut x, mut y, mut z) = 值.取三维().unwrap_or((0.0, 0.0, 0.0));
            let mut 改变 = false;
            ui.push_id(format!("三维-{字段名}"), |ui| {
                for (标签, 分量) in [("x", &mut x), ("y", &mut y), ("z", &mut z)] {
                    改变 |= ui
                        .add(egui::DragValue::new(分量).speed(*步进).prefix(标签))
                        .changed();
                }
            });
            if 改变 {
                *值 = 属性值::三维(x, y, z);
            }
            改变
        }

        属性种类::角度三维 => {
            // 角度用度作为单位，因此步进固定为 0.5 度而不是沿用浮点步进。
            let (mut x, mut y, mut z) = 值.取三维().unwrap_or((0.0, 0.0, 0.0));
            let mut 改变 = false;
            ui.push_id(format!("角度-{字段名}"), |ui| {
                for (标签, 分量) in [("x", &mut x), ("y", &mut y), ("z", &mut z)] {
                    改变 |= ui
                        .add(
                            egui::DragValue::new(分量)
                                .speed(0.5)
                                .prefix(标签)
                                .suffix("°"),
                        )
                        .changed();
                }
            });
            if 改变 {
                *值 = 属性值::三维(x, y, z);
            }
            改变
        }

        属性种类::四维 { 步进 } => {
            let (mut a, mut b, mut c, mut d) = match 值 {
                属性值::四维(a, b, c, d) => (*a, *b, *c, *d),
                _ => (0.0, 0.0, 0.0, 0.0),
            };
            let mut 改变 = false;
            ui.push_id(format!("四维-{字段名}"), |ui| {
                for (标签, 分量) in [("x", &mut a), ("y", &mut b), ("z", &mut c), ("w", &mut d)]
                {
                    改变 |= ui
                        .add(egui::DragValue::new(分量).speed(*步进).prefix(标签))
                        .changed();
                }
            });
            if 改变 {
                *值 = 属性值::四维(a, b, c, d);
            }
            改变
        }

        属性种类::颜色 => {
            let 颜色 = 值.取颜色().unwrap_or(颜色值::白());
            let mut 数组 = [颜色.红, 颜色.绿, 颜色.蓝, 颜色.透明度];
            let 改变 = ui.color_edit_button_rgba_unmultiplied(&mut 数组).changed();
            if 改变 {
                *值 = 属性值::颜色(颜色值 {
                    红: 数组[0],
                    绿: 数组[1],
                    蓝: 数组[2],
                    透明度: 数组[3],
                    是_srgb: 颜色.是_srgb,
                });
            }
            改变
        }

        属性种类::枚举 { 选项 } => {
            let 当前 = match 值 {
                属性值::枚举(枚举) => 枚举.clone(),
                _ => 枚举值::新建("", 0),
            };
            let 当前文本 = 选项
                .iter()
                .find(|(_, 变体名, 序号)| 变体名 == &当前.变体名 || *序号 == 当前.序号)
                .map(|(显示名, _, _)| 显示名.clone())
                .unwrap_or_else(|| 上下文.语言.填充("unknown", &[&当前.变体名]));

            let mut 改变 = false;
            egui::ComboBox::from_id_salt(format!("枚举-{字段名}"))
                .selected_text(当前文本)
                .show_ui(ui, |ui| {
                    for (显示名, 变体名, 序号) in 选项 {
                        let 选中 = 变体名 == &当前.变体名;
                        if ui.selectable_label(选中, 显示名).clicked() && !选中 {
                            *值 = 属性值::枚举(枚举值::新建(变体名.clone(), *序号));
                            改变 = true;
                        }
                    }
                });
            改变
        }

        属性种类::资源 { 类别 } => {
            let 当前 = 值.取资源().cloned();
            let mut 改变 = false;
            ui.horizontal(|ui| {
                match &当前 {
                    Some(引用) => {
                        // 有缩略图就画缩略图，否则画一个类型角标。
                        if let Some(纹理) = 上下文.缩略图(&引用.路径) {
                            ui.add(
                                egui::Image::new(纹理).fit_to_exact_size(egui::vec2(18.0, 18.0)),
                            );
                        } else {
                            ui.label(
                                RichText::new(上下文.语言.取(类别.词条键()))
                                    .small()
                                    .color(上下文.配色.次要文字),
                            );
                        }
                        ui.label(
                            RichText::new(引用.路径.文件名())
                                .small()
                                .color(上下文.配色.文字),
                        )
                        .on_hover_text(引用.路径.文本().to_string());
                        if ui.small_button(上下文.语言.取("clear_2")).clicked() {
                            *值 = 属性值::空;
                            改变 = true;
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new(上下文.语言.取("not_set")).color(上下文.配色.次要文字),
                        );
                    }
                }
            });
            改变
        }

        属性种类::列表 { 元素 } => {
            let mut 改变 = false;
            let mut 列表 = match 值 {
                属性值::列表(列表) => 列表.clone(),
                _ => Vec::new(),
            };
            let mut 待删除 = None;
            ui.vertical(|ui| {
                for (下标, 项) in 列表.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("[{下标}]"))
                                .small()
                                .color(上下文.配色.次要文字),
                        );
                        if 按描述编辑(ui, &format!("{字段名}-{下标}"), 元素, 项, 上下文)
                        {
                            改变 = true;
                        }
                        if ui.small_button("✕").clicked() {
                            待删除 = Some(下标);
                        }
                    });
                }
                if ui.small_button(上下文.语言.取("add_element")).clicked() {
                    列表.push(元素_default(元素));
                    改变 = true;
                }
            });
            if let Some(下标) = 待删除 {
                列表.remove(下标);
                改变 = true;
            }
            if 改变 {
                *值 = 属性值::列表(列表);
            }
            改变
        }

        属性种类::结构 { 字段 } => {
            let mut 改变 = false;
            let mut 当前字段: Vec<(String, 属性值)> = match 值 {
                属性值::结构(字段) => 字段.clone(),
                _ => Vec::new(),
            };
            ui.vertical(|ui| {
                for 子描述 in 字段 {
                    let 位置 = 当前字段.iter().position(|(名字, _)| 名字 == &子描述.字段名);
                    let mut 子值 = 位置
                        .map(|位置| 当前字段[位置].1.clone())
                        .unwrap_or_else(|| 子描述.默认值.clone());

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(上下文.语言.取(子描述.显示名))
                                .small()
                                .color(上下文.配色.次要文字),
                        );
                        if 按描述编辑(ui, &子描述.字段名, &子描述.种类, &mut 子值, 上下文)
                        {
                            match 位置 {
                                Some(位置) => 当前字段[位置].1 = 子值,
                                None => 当前字段.push((子描述.字段名.clone(), 子值)),
                            }
                            改变 = true;
                        }
                    });
                }
            });
            if 改变 {
                *值 = 属性值::结构(当前字段);
            }
            改变
        }
    }
}

/// 为列表新元素取一个默认值。
fn 元素_default(元素: &属性种类) -> 属性值 {
    match 元素 {
        属性种类::布尔 => 属性值::布尔(false),
        属性种类::整数 { .. } => 属性值::整数(0),
        属性种类::浮点 { .. } => 属性值::浮点(0.0),
        属性种类::文本 => 属性值::文本(String::new()),
        属性种类::二维 { .. } => 属性值::二维(0.0, 0.0),
        属性种类::三维 { .. } | 属性种类::角度三维 => {
            属性值::三维(0.0, 0.0, 0.0)
        }
        属性种类::四维 { .. } => 属性值::四维(0.0, 0.0, 0.0, 0.0),
        属性种类::颜色 => 属性值::颜色(颜色值::白()),
        属性种类::枚举 { 选项 } => 选项
            .first()
            .map(|(_, 变体名, 序号)| 属性值::枚举(枚举值::新建(变体名.clone(), *序号)))
            .unwrap_or(属性值::空),
        属性种类::资源 { .. } => 属性值::空,
        属性种类::列表 { .. } => 属性值::列表(Vec::new()),
        属性种类::结构 { .. } => 属性值::结构(Vec::new()),
    }
}

/// 在没有字段描述时，按值的实际种类推断控件。
fn 通用值编辑(
    ui: &mut Ui, 字段名: &str, 值: &mut 属性值, 语言: crate::i18n::语言
) -> bool {
    match 值 {
        属性值::布尔(开关) => ui.checkbox(开关, "").changed(),
        属性值::整数(数值) => ui.add(egui::DragValue::new(数值)).changed(),
        属性值::浮点(数值) => ui.add(egui::DragValue::new(数值).max_decimals(4)).changed(),
        属性值::文本(文本) => ui
            .add(egui::TextEdit::singleline(文本).desired_width(180.0))
            .changed(),
        属性值::二维(x, y) => {
            let mut 改变 = false;
            ui.push_id(format!("通用二维-{字段名}"), |ui| {
                改变 |= ui.add(egui::DragValue::new(x).prefix("x")).changed();
                改变 |= ui.add(egui::DragValue::new(y).prefix("y")).changed();
            });
            改变
        }
        属性值::三维(x, y, z) => {
            let mut 改变 = false;
            ui.push_id(format!("通用三维-{字段名}"), |ui| {
                改变 |= ui.add(egui::DragValue::new(x).prefix("x")).changed();
                改变 |= ui.add(egui::DragValue::new(y).prefix("y")).changed();
                改变 |= ui.add(egui::DragValue::new(z).prefix("z")).changed();
            });
            改变
        }
        属性值::四维(a, b, c, d) => {
            let mut 改变 = false;
            ui.push_id(format!("通用四维-{字段名}"), |ui| {
                for (标签, 分量) in [
                    ("x", &mut *a),
                    ("y", &mut *b),
                    ("z", &mut *c),
                    ("w", &mut *d),
                ] {
                    改变 |= ui.add(egui::DragValue::new(分量).prefix(标签)).changed();
                }
            });
            改变
        }
        属性值::颜色(颜色) => {
            let mut 数组 = [颜色.红, 颜色.绿, 颜色.蓝, 颜色.透明度];
            let 改变 = ui.color_edit_button_rgba_unmultiplied(&mut 数组).changed();
            if 改变 {
                颜色.红 = 数组[0];
                颜色.绿 = 数组[1];
                颜色.蓝 = 数组[2];
                颜色.透明度 = 数组[3];
            }
            改变
        }
        属性值::枚举(枚举) => {
            let mut 改变 = false;
            ui.horizontal(|ui| {
                if ui
                    .add(egui::DragValue::new(&mut 枚举.序号).prefix("#"))
                    .changed()
                {
                    改变 = true;
                }
                if ui.text_edit_singleline(&mut 枚举.变体名).changed() {
                    改变 = true;
                }
            });
            改变
        }
        属性值::资源(引用) => {
            ui.label(RichText::new(引用.路径.文本()).small().monospace());
            false
        }
        属性值::列表(列表) => {
            let mut 改变 = false;
            ui.vertical(|ui| {
                for (下标, 项) in 列表.iter_mut().enumerate() {
                    改变 |= 通用值编辑(ui, &format!("{字段名}-{下标}"), 项, 语言);
                }
            });
            改变
        }
        属性值::结构(字段) => {
            let mut 改变 = false;
            ui.vertical(|ui| {
                for (名字, 项) in 字段.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(名字.as_str()).small());
                        改变 |= 通用值编辑(ui, 名字, 项, 语言);
                    });
                }
            });
            改变
        }
        属性值::空 => {
            ui.label(RichText::new(语言.取("empty_2")).color(Color32::GRAY));
            false
        }
    }
}

/// 一行摘要文本，用于只读字段。
fn 值摘要(值: &属性值, 语言: crate::i18n::语言) -> String {
    match 值 {
        属性值::布尔(值) => 值.to_string(),
        属性值::整数(值) => 值.to_string(),
        属性值::浮点(值) => format!("{值:.4}"),
        属性值::文本(值) => 值.clone(),
        属性值::二维(x, y) => format!("({x:.3}, {y:.3})"),
        属性值::三维(x, y, z) => format!("({x:.3}, {y:.3}, {z:.3})"),
        属性值::四维(a, b, c, d) => format!("({a:.3}, {b:.3}, {c:.3}, {d:.3})"),
        属性值::颜色(颜色) => format!(
            "rgba({:.2}, {:.2}, {:.2}, {:.2})",
            颜色.红, 颜色.绿, 颜色.蓝, 颜色.透明度
        ),
        属性值::枚举(枚举) => 枚举.变体名.clone(),
        属性值::资源(引用) => 引用.路径.文本().to_string(),
        属性值::列表(列表) => 语言.填充("value_list_items", &[&列表.len()]),
        属性值::结构(字段) => 语言.填充("value_struct_fields", &[&字段.len()]),
        属性值::空 => 语言.取("value_empty").to_string(),
    }
}

/// 检查器使用的编辑器状态（供测试与外部查询）。
pub fn 当前主选中(编辑器: &编辑器状态) -> Option<实体标识> {
    编辑器.选择.主选中()
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 欧拉角与四元数互转必须自洽。
    #[test]
    fn 欧拉角与四元数往返一致() {
        let 用例 = [
            (0.0f32, 0.0f32, 0.0f32),
            (30.0, 0.0, 0.0),
            (0.0, 45.0, 0.0),
            (0.0, 0.0, 90.0),
            (10.0, 20.0, 30.0),
        ];
        for 原角 in 用例 {
            let 四元数 = 欧拉角转四元数(原角);
            // 生成的四元数必须是单位长度，否则会引入缩放
            let 模长 = (四元数.0 * 四元数.0
                + 四元数.1 * 四元数.1
                + 四元数.2 * 四元数.2
                + 四元数.3 * 四元数.3)
                .sqrt();
            assert!(
                (模长 - 1.0).abs() < 1e-4,
                "四元数模长应为 1，实际 {模长}（原角 {原角:?}）"
            );

            let 还原 = 四元数转欧拉角(四元数);
            for (下标, (左, 右)) in [原角.0, 原角.1, 原角.2]
                .iter()
                .zip([还原.0, 还原.1, 还原.2].iter())
                .enumerate()
            {
                assert!(
                    (左 - 右).abs() < 0.05,
                    "第 {下标} 个角度往返后偏差过大：{左} vs {右}（原角 {原角:?}）"
                );
            }
        }
    }

    #[test]
    fn 单位四元数对应零角度() {
        let 角 = 四元数转欧拉角((0.0, 0.0, 0.0, 1.0));
        assert!(角.0.abs() < 1e-4 && 角.1.abs() < 1e-4 && 角.2.abs() < 1e-4);
    }

    #[test]
    fn 退化四元数不会产生_nan() {
        // 零四元数在数学上无意义，但损坏的文件里可能出现。
        // 编辑器绝不能因此产生 NaN 并把它写回文档。
        let 角 = 四元数转欧拉角((0.0, 0.0, 0.0, 0.0));
        assert!(角.0.is_finite() && 角.1.is_finite() && 角.2.is_finite());
        assert_eq!(角, (0.0, 0.0, 0.0));
    }

    #[test]
    fn 列表新元素的默认值符合元素种类() {
        // 这是"合理的默认值"这条要求的直接验证：
        // 往列表里加元素时，默认值必须与声明的元素种类匹配，
        // 否则加进去的项立刻是非法数据。
        let 各种类 = [
            属性种类::布尔,
            属性种类::自由整数(),
            属性种类::自由浮点(),
            属性种类::文本,
            属性种类::二维默认(),
            属性种类::三维默认(),
            属性种类::颜色,
            属性种类::枚举 {
                选项: vec![("甲".into(), "甲".into(), 0)],
            },
            属性种类::资源 {
                类别: scene_doc::property::资源类别::图片,
            },
        ];
        for 种类 in &各种类 {
            let 默认值 = 元素_default(种类);
            assert!(
                种类.接受(&默认值),
                "为 {种类:?} 生成的默认值 {默认值:?} 不符合该种类"
            );
        }
    }

    #[test]
    fn 值摘要覆盖所有种类且不崩溃() {
        let 所有值 = [
            属性值::布尔(true),
            属性值::整数(-3),
            属性值::浮点(1.5),
            属性值::文本("文本".into()),
            属性值::二维(1.0, 2.0),
            属性值::三维(1.0, 2.0, 3.0),
            属性值::四维(1.0, 2.0, 3.0, 4.0),
            属性值::颜色(颜色值::srgb(1.0, 0.0, 0.0, 1.0)),
            属性值::枚举(枚举值::新建("甲", 0)),
            属性值::列表(vec![属性值::整数(1)]),
            属性值::结构(vec![("甲".into(), 属性值::整数(1))]),
            属性值::空,
        ];
        for 值 in &所有值 {
            let 摘要 = 值摘要(值, crate::i18n::语言::中文);
            assert!(!摘要.is_empty(), "{值:?} 的摘要不应为空");
        }
    }
}
