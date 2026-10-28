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

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use editor_core::state::{编辑器状态, 预制件身份};
use egui::{Color32, RichText, Ui};
use scene_doc::document::{变换数据, 场景实体, 实体标识};
use scene_doc::prefab::{变换组件名, 实体来源, 覆盖状态};
use scene_doc::prefab_nested::变换覆盖字段;
use scene_doc::property::{属性值, 枚举值, 资源类别, 颜色值};
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
///
/// # 环境设置为什么常驻
///
/// 环境光与背景属于**整个场景**，不挂在任何实体上。若只在选中实体时显示，
/// 用户必须先随便选个东西才能改天空亮度；若不选实体就不显示，又会出现
/// "这个面板到底有没有环境设置"的不确定感。放在顶部常驻，选中与不选中
/// 两种状态都能编辑。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    绘制环境区(ui, 上下文);
    ui.add_space(crate::visual_spec::留白::分组 * 0.5);
    crate::visual_spec::细线(ui, 上下文.配色);
    ui.add_space(crate::visual_spec::留白::分组 * 0.5);

    let 编辑器 = 上下文.编辑器;

    let Some(标识) = 编辑器.选择.主选中() else {
        // 空状态只给两行文字：标题说"现在没有内容"，说明说"下一步做什么"。
        // 不画图标卡片、不加边框 —— 空面板本身已经很安静，再加装饰只会更空。
        let 提示 = 上下文.语言.取("inspector_no_entity_selected_hint");
        let (标题, 说明) = match 提示.split_once('\n') {
            Some((标题, 说明)) => (标题, 说明),
            None => (提示, ""),
        };
        crate::visual_spec::空状态(ui, 上下文, 标题, 说明);
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
    let 预制件 = 编辑器.预制件身份(&标识);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            绘制头部(ui, 上下文, &实体快照, 选择数量, 预制件.as_ref());
            ui.add_space(crate::visual_spec::留白::分组);
            crate::visual_spec::细线(ui, 上下文.配色);
            ui.add_space(crate::visual_spec::留白::分组 * 0.5);

            // ---- 变换 ----
            if let Some(变换) = 实体快照.变换 {
                绘制变换区(ui, 上下文, 标识, 变换);
                ui.add_space(crate::visual_spec::留白::分组 * 0.5);
                crate::visual_spec::细线(ui, 上下文.配色);
                ui.add_space(crate::visual_spec::留白::分组 * 0.5);
            }

            // ---- 组件 ----
            绘制组件列表(ui, 上下文, 标识, &实体快照);
        });
}

/// 绘制全局环境设置：环境光颜色/强度与背景颜色。
///
/// # 交互
///
/// 三个控件都直接改文档里的 [`场景环境`] 副本，一旦有变化就推一条
/// [`UiAction::设置场景环境`]。动作层用连续命令执行，因此拖一次滑块
/// 只产生一步撤销；预览在同一帧的下一次同步里读到新值，实现实时反馈。
///
/// # 背景为什么固定不透明
///
/// 预览相机渲染到界面纹理，透明背景会让画布后面的面板透出来。
/// 用户选的是"场景背景"，只暴露 RGB 三个分量，透明度强制为 1。
fn 绘制环境区(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let mut 环境 = 上下文.编辑器.会话.文档.环境;
    let mut 改了 = false;

    egui::CollapsingHeader::new(RichText::new(语言.取("environment_settings")).color(配色.文字))
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("environment_settings_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(语言.取("environment_ambient_color"));
                    let mut 颜色 = [
                        环境.环境光颜色.红,
                        环境.环境光颜色.绿,
                        环境.环境光颜色.蓝,
                        环境.环境光颜色.透明度,
                    ];
                    if ui.color_edit_button_rgba_unmultiplied(&mut 颜色).changed() {
                        环境.环境光颜色 = 颜色值::srgb(颜色[0], 颜色[1], 颜色[2], 颜色[3]);
                        改了 = true;
                    }
                    ui.end_row();

                    ui.label(语言.取("environment_ambient_intensity"));
                    if ui
                        .add(
                            egui::DragValue::new(&mut 环境.环境光强度)
                                .speed(10.0)
                                .range(0.0..=100_000.0),
                        )
                        .changed()
                    {
                        改了 = true;
                    }
                    ui.end_row();

                    ui.label(语言.取("environment_background_color"));
                    let mut 颜色 = [环境.背景颜色.红, 环境.背景颜色.绿, 环境.背景颜色.蓝];
                    if ui.color_edit_button_rgb(&mut 颜色).changed() {
                        环境.背景颜色 = 颜色值::srgb(颜色[0], 颜色[1], 颜色[2], 1.0);
                        改了 = true;
                    }
                    ui.end_row();
                });
        });

    if 改了 {
        上下文.动作.推(UiAction::设置场景环境(环境));
    }
}

/// 绘制头部：实体名、标识、预制件归属、组件与实体动作。
///
/// # 层级
///
/// 1. 实体名是这一屏的主标题（Heading 档），可以直接编辑；
/// 2. 类型与 id 退成辅助信息（等宽小字），它们用来核对，不用来阅读；
/// 3. 预制件徽标紧跟其后 —— 它回答"这一屏改的东西落到哪儿"，
///    必须在用户开始编辑之前就看见，放在动作行之后等于事后解释；
/// 4. 动作放在最后一行，`添加组件` 是主操作（唯一带强调色的按钮），
///    复制/删除是次要操作。
///
/// 改造前这里是一排"名称: [输入框]"加三个等权按钮，实体名和"添加组件"
/// 看起来一样重要，用户扫一眼不知道自己在编辑谁。
fn 绘制头部(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    实体: &场景实体,
    选择数量: usize,
    预制件: Option<&预制件身份>,
) {
    let 配色 = 上下文.配色;
    let mut 名称 = 实体.名称.clone();
    let 响应 = ui.add(
        egui::TextEdit::singleline(&mut 名称)
            .font(egui::TextStyle::Heading)
            .desired_width(f32::INFINITY)
            // 实体名是这一屏的**标题**，不是表单字段。
            // 给它加输入框底色会让最高层级看起来像一块输入控件，
            // 反而把标题压成了"表单第一行"。保持无框，靠光标表达可编辑。
            .frame(egui::Frame::NONE)
            .hint_text(上下文.语言.取("name")),
    );
    if 响应.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if 响应.lost_focus() && ui.input(|输入| 输入.key_pressed(egui::Key::Enter)) {
        let 新名称 = 名称.trim().to_string();
        if !新名称.is_empty() && 新名称 != 实体.名称 {
            上下文.动作.推(UiAction::重命名实体 {
                标识: 实体.标识,
                新名称,
            });
        }
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        // 只显示 id 的前 8 个字符：完整 uuid 有 36 个字符，会把这一行
        // 撑成一条"看起来像哈希的墙"，而实际需要的是"能区分两个实体"。
        // 完整值放在悬停提示里，复制需求由提示承担。
        let 完整id = 实体.标识.to_string();
        let 短id: String = 完整id.chars().take(8).collect();
        ui.label(
            RichText::new(format!("{}  ·  {}", 上下文.语言.取("id"), 短id))
                .size((上下文.外观.基础字号 - 2.0).max(9.0))
                .monospace()
                .color(配色.禁用文字),
        )
        .on_hover_text(上下文.语言.填充(
            "entity_id_full_tooltip",
            &[&上下文.语言.取("entity_id_stable_tooltip"), &完整id],
        ));
        if 选择数量 > 1 {
            ui.label(
                RichText::new(
                    上下文
                        .语言
                        .填充("inspector_multi_selection_hint", &[&选择数量]),
                )
                .size((上下文.外观.基础字号 - 2.0).max(9.0))
                .color(配色.警告),
            );
        }
    });

    if let Some(身份) = 预制件 {
        绘制预制件徽标(ui, 上下文, 实体, 身份);
    }

    ui.add_space(crate::visual_spec::留白::控件水平);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let 可添加组件 = 上下文.编辑器.注册表.数量() > 0;
        if ui
            .add_enabled(
                可添加组件,
                egui::Button::new(RichText::new(上下文.语言.取("add_component")).color(
                    if 可添加组件 {
                        配色.强调亮
                    } else {
                        配色.禁用文字
                    },
                ))
                .fill(配色.强调淡)
                .stroke(egui::Stroke::new(1.0, 配色.强调)),
            )
            .clicked()
        {
            上下文.界面.打开组件选择器 = !上下文.界面.打开组件选择器;
            上下文.界面.组件目标 = Some(实体.标识);
        }
        let 尺寸 = 上下文.外观.图标尺寸 * 0.9;
        if 上下文
            .图标
            .图标按钮(
                ui,
                crate::icons::图标::复制,
                None,
                尺寸,
                配色.次要文字,
                true,
            )
            .on_hover_text(上下文.语言.取("duplicate_entity"))
            .clicked()
        {
            上下文.动作.推(UiAction::复制实体(实体.标识));
        }
        if 上下文
            .图标
            .图标按钮(
                ui,
                crate::icons::图标::删除,
                None,
                尺寸,
                配色.次要文字,
                true,
            )
            .on_hover_text(上下文.语言.取("delete_entity"))
            .clicked()
        {
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

        egui::Frame::popup(ui.style())
            .inner_margin(crate::visual_spec::面板内边距())
            .show(ui, |ui| {
                if 候选.is_empty() {
                    ui.label(
                        RichText::new(上下文.语言.取("inspector_all_components_added"))
                            .color(配色.次要文字),
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

/// 绘制预制件归属徽标。
///
/// # 为什么界面层要专门说这件事
///
/// 嵌套预制件展开后，场景里的一行可能由**内层记录**管着，也可能由最外层
/// 记录管着。两者都能改，但改动落点不同：写到外层记录里的内层地址永远
/// 匹配不上，下一次同步把用户改的东西原样还原 —— 静默失效，不报错。
/// 命令层已经按地址域换算好了（[`editor_core::state::编辑器状态::预制件身份`]
/// 就是那次换算的结果），界面必须把结论显示出来，否则用户只能靠猜。
///
/// 文案与配色按 `(是不是某层实例根, 落点是不是内层记录)` 四选一：
/// * 外层实例根 / 外层派生节点 —— 次要文字，改动记进本实例的覆盖；
/// * 嵌套实例根 / 内层派生节点 —— 信息色，改动记进那一层的内层记录，
///   同步时随最外层实例一起展开。
///
/// 这一行的文本、语言键、实际配色与矩形会写进
/// [`crate::state::界面状态::预制件徽标`]：徽标不改变任何面板矩形，
/// 没有这份"本帧事实"，验收就分不清「嵌套徽标没渲染」与「渲染成了外层实例的
/// 灰字」，而后者会让用户在错误的地址域上改东西且毫无察觉。
fn 绘制预制件徽标(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    实体: &场景实体,
    身份: &预制件身份,
) {
    let 是实例根 = matches!(
        实体.来源,
        实体来源::预制件实例根 { .. } | 实体来源::预制件嵌套实例根 { .. }
    );
    let (标签键, 提示键, 颜色) = match (是实例根, 身份.是嵌套记录) {
        (true, false) => (
            "prefab_instance",
            "prefab_override_outer_hint",
            上下文.配色.次要文字,
        ),
        (true, true) => (
            "prefab_nested_instance",
            "prefab_override_inner_hint",
            上下文.配色.信息,
        ),
        (false, false) => (
            "prefab_derived_node",
            "prefab_override_outer_hint",
            上下文.配色.次要文字,
        ),
        (false, true) => (
            "prefab_nested_node",
            "prefab_override_inner_hint",
            上下文.配色.信息,
        ),
    };
    ui.horizontal(|ui| {
        let 文本 = format!("{} · {}", 上下文.语言.取(标签键), 身份.模板名称);
        let 响应 = ui.label(
            RichText::new(文本.clone())
                .size((上下文.外观.基础字号 - 2.0).max(9.0))
                .color(颜色),
        );
        上下文.界面.预制件徽标 = Some(crate::state::预制件徽标几何 {
            语言键: 标签键,
            文本,
            颜色,
            矩形: 响应.rect,
            是内层记录: 身份.是嵌套记录,
        });
        响应.on_hover_text(上下文.语言.填充(提示键, &[&身份.模板名称]));
    });
}

/// 组件属性行「恢复默认值」按钮的区域标识（进程级驻留）。
///
/// # 为什么需要驻留
///
/// [`crate::state::区域快照`] 的 `标识` 是 `&'static str`，GUI 探针按它点击。
/// 变换三行可以直接用字面量，组件行的键却来自注册表 —— 组件类型名与字段名
/// 都是运行期 `String`。这里把 `inspector_revert_field.{组件}.{属性}` 驻留进
/// 一张进程级表：同一对键永远返回同一个 `&'static str`，探针手里的标识与
/// 面板每帧推入的标识因此可以直接比较。
///
/// 表只增不减：键的取值域是组件注册表里的「组件 × 字段」，有限且远小于
/// 场景规模，不需要淘汰策略。
fn 组件行区域标识(组件: &str, 属性: &str) -> &'static str {
    static 驻留表: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let 表 = 驻留表.get_or_init(|| Mutex::new(HashMap::new()));
    let 键 = format!("inspector_revert_field.{}.{}", 组件, 属性);
    let mut 表 = 表.lock().expect("组件行区域标识表被毒化");
    if let Some(标识) = 表.get(&键) {
        return *标识;
    }
    let 标识: &'static str = Box::leak(键.clone().into_boxed_str());
    表.insert(键, 标识);
    标识
}

/**
 * 画「恢复该项默认值」按钮：只在**这一项确实存在覆盖**时出现。
 *
 * 参数：
 * * `实体` —— 正在编辑的实体；覆盖落在哪条记录、相对哪个模板根，全部由
 *   [`编辑器状态::预制件字段覆盖状态`] 按它算出来，界面不参与换算；
 * * `组件` / `属性` —— 与写入覆盖时**同一对键**：变换三行用 [`变换组件名`]
 *   与 [`变换覆盖字段`]，普通属性行用注册表里的组件名与字段名；
 * * `区域标识` —— 验收脚本点击用的静态区域名。变换三行用固定字面量（与既有
 *   验收脚本的契约一致）；组件行由 [`组件行区域标识`] 按
 *   `inspector_revert_field.{组件}.{属性}` 驻留出静态名，因此组件字段行的
 *   按钮同样能被 GUI 探针点到，不必靠猜坐标。
 *
 * 状态查询走 [`crate::state::预制件覆盖缓存`]，按 `(实体, 文档修订号)` 失效：
 * 命中时不碰文档，只有选中或内容变化后的第一帧才真正遍历预制件记录。
 *
 * 显示条件按状态收紧到**直接覆盖**：`继承` 没有条目可删；`祖先覆盖` 由整组件
 * 覆盖管着，删字段级条目是空操作（[`scene_doc::prefab::覆盖表::恢复字段`]
 * 刻意不删祖先）；`移除` 是组件级动作，不是「恢复这一项」。三种情况下按钮
 * 点下去要么什么都不做、要么只做一半，画出来就是「按了没反应」。
 *
 * 按钮文字走 i18n 词条 `prefab_revert_field`（「恢复」/「Reset」），不用
 * `⟲` 这类符号：内置中文字体子集里没有 U+27F2，它目前能显示只是因为 egui
 * 自带的 emoji 图标字体恰好收录了它 —— 字形来源与界面字体不同，也绕开了
 * 词条。纯文字由同一套界面字体渲染、可随语言切换，两三个汉字/字母不会把
 * 属性名挤到换行；完整含义由悬停提示补足（`prefab_revert_field_hint`）。
 */
fn 绘制恢复按钮(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    实体: 实体标识,
    组件: &str,
    属性: &str,
    区域标识: Option<&'static str>,
) {
    let 编辑器 = 上下文.编辑器;
    let 缓存 = &mut 上下文.界面.预制件覆盖缓存;
    let 状态 = 缓存.取(实体, 编辑器.文档修订(), 组件, 属性, || {
        编辑器.预制件字段覆盖状态(实体, 组件, 属性)
    });
    if !matches!(状态, Some(覆盖状态::直接覆盖)) {
        return;
    }
    let 响应 = ui
        .push_id(("revert_field", 组件, 属性), |ui| {
            ui.add(
                egui::Button::new(
                    RichText::new(上下文.语言.取("prefab_revert_field"))
                        .size((上下文.外观.基础字号 - 1.0).max(9.0))
                        .color(上下文.配色.信息),
                )
                .fill(Color32::TRANSPARENT)
                .frame(false),
            )
        })
        .inner;
    if let Some(标识) = 区域标识 {
        上下文.界面.布局快照.push(crate::state::区域快照 {
            标识,
            矩形: 响应.rect,
        });
    }
    if 响应
        .on_hover_text(上下文.语言.取("prefab_revert_field_hint"))
        .clicked()
    {
        上下文.动作.推(UiAction::恢复预制件字段默认值 {
            实体,
            组件: 组件.to_string(),
            属性: 属性.to_string(),
        });
    }
}

/// 绘制变换区。
///
/// 三个分量用 `Grid` 排版：属性名列宽固定，数值控件从同一条竖线开始。
/// 改造前每个属性行各自 `horizontal`，标签不等宽，数值框起点参差，
/// 一屏下来像手写的表格。
fn 绘制变换区(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 标识: 实体标识, 变换: 变换数据
) {
    let 可折叠 = egui::CollapsingHeader::new(
        RichText::new(上下文.语言.取("transform"))
            .size(上下文.外观.基础字号 - 0.5)
            .strong()
            .color(上下文.配色.文字),
    )
    .default_open(true)
    .id_salt("变换区");

    可折叠.show(ui, |ui| {
        let mut 新变换 = 变换;
        egui::Grid::new("变换网格")
            .num_columns(2)
            .spacing([crate::visual_spec::留白::标签, 4.0])
            .min_col_width(crate::visual_spec::留白::标签列宽)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    绘制恢复按钮(
                        ui,
                        上下文,
                        标识,
                        变换组件名,
                        变换覆盖字段[0],
                        Some("inspector_revert_pos"),
                    );
                    ui.label(RichText::new(上下文.语言.取("position")).color(上下文.配色.次要文字));
                });
                let 改变 = 三元拖动(ui, 上下文, "位置", &mut 新变换.平移, 0.05);
                ui.end_row();
                if 改变 {
                    提交变换(上下文, 标识, 新变换, true);
                }

                // 旋转以角度编辑，内部仍存四元数。
                let 欧拉 = 四元数转欧拉角(变换.旋转);
                let mut 新欧拉 = 欧拉;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    绘制恢复按钮(
                        ui,
                        上下文,
                        标识,
                        变换组件名,
                        变换覆盖字段[1],
                        Some("inspector_revert_rot"),
                    );
                    ui.label(RichText::new(上下文.语言.取("rotate")).color(上下文.配色.次要文字));
                });
                ui.vertical(|ui| {
                    if 三元拖动(ui, 上下文, "旋转", &mut 新欧拉, 0.5) {
                        新变换.旋转 = 欧拉角转四元数(新欧拉);
                        提交变换(上下文, 标识, 新变换, true);
                    }
                    // 单位紧贴它解释的那一行，而不是排在缩放行之后 ——
                    // 排在下面时，用户读到它已经离开了旋转字段。
                    //
                    // 放在数值框**下方**而不是同一行：检查器最窄只有约 300px，
                    // 三个数值框加单位提示放在一行会把提示挤出面板。
                    ui.label(
                        RichText::new(上下文.语言.取("transform_rotation_units_hint"))
                            .size((上下文.外观.基础字号 - 3.5).max(8.0))
                            .color(上下文.配色.禁用文字),
                    );
                });
                ui.end_row();

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    绘制恢复按钮(
                        ui,
                        上下文,
                        标识,
                        变换组件名,
                        变换覆盖字段[2],
                        Some("inspector_revert_scale"),
                    );
                    ui.label(
                        RichText::new(上下文.语言.取("transform_field_scale"))
                            .color(上下文.配色.次要文字),
                    );
                });
                if 三元拖动(ui, 上下文, "缩放", &mut 新变换.缩放, 0.01) {
                    提交变换(上下文, 标识, 新变换, true);
                }
                ui.end_row();
            });

        // 缩放归零会让网格完全退化，给一个快捷修复。
        if 新变换.缩放.0 == 0.0 || 新变换.缩放.1 == 0.0 || 新变换.缩放.2 == 0.0 {
            ui.label(
                RichText::new(上下文.语言.取("inspector_scale_zero_warning"))
                    .size((上下文.外观.基础字号 - 2.0).max(9.0))
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
        // 一组数值 = 一个"输入组"：三个分量共用一层浅底、圆角 4，
        // 组内用 1px 缝分开。这样一行的读数一眼能看出"这是位置/旋转/缩放"，
        // 又不会像三个独立方块那样把面板切碎。
        crate::visual_spec::输入框外框(ui, 上下文.配色, 上下文.主题.明暗, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (下标, 标签) in ["x", "y", "z"].iter().enumerate() {
                    let 分量 = match 下标 {
                        0 => &mut 值.0,
                        1 => &mut 值.1,
                        _ => &mut 值.2,
                    };
                    // 轴字母用轴色、字号小一档；数值等宽、固定两位。
                    // 拖动时数字不跳宽度，一眼能看出是哪个轴在变。
                    let 轴色 = match 下标 {
                        0 => 上下文.配色.轴_x,
                        1 => 上下文.配色.轴_y,
                        _ => 上下文.配色.信息,
                    };
                    ui.label(
                        RichText::new(*标签)
                            .size((上下文.外观.基础字号 - 3.5).max(8.0))
                            .monospace()
                            .color(轴色),
                    );
                    let 响应 = ui.add_sized(
                        [58.0, (上下文.外观.基础字号 + 8.0).max(22.0)],
                        egui::DragValue::new(分量)
                            .speed(步进)
                            .fixed_decimals(2)
                            .update_while_editing(true),
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
        });
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
    egui::CollapsingHeader::new(
        RichText::new(类型名)
            .size(上下文.外观.基础字号 - 0.5)
            .strong()
            .color(上下文.配色.文字),
    )
    .default_open(true)
    .id_salt(format!("组件-{类型名}"))
    .show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(上下文.语言.取("remove_component_2"))
                            .size((上下文.外观.基础字号 - 2.5).max(9.0))
                            .color(上下文.配色.错误),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .frame(false),
                )
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

        // 网格排版：字段名列宽固定，所有控件从同一条竖线开始。
        // `min_col_width` 用视觉规范里的标签列宽，与变换区共用同一档，
        // 因此同一个面板里不会出现两种标签宽度。
        egui::Grid::new(("组件属性", 类型名))
            .num_columns(2)
            .spacing([crate::visual_spec::留白::标签, 4.0])
            .min_col_width(crate::visual_spec::留白::标签列宽)
            .show(ui, |ui| {
                for 描述 in 字段描述 {
                    let 当前 = 属性
                        .iter()
                        .find(|(名字, _)| 名字 == &描述.字段名)
                        .map(|(_, 值)| 值.clone())
                        .unwrap_or_else(|| 描述.默认值.clone());

                    // 字段名与悬停提示。提示内容由 `格式提示` 统一组装，
                    // 因此这里不需要按"提示是否为空"分叉 —— 之前那样写
                    // 会出现两个完全相同的分支。
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        绘制恢复按钮(
                            ui,
                            上下文,
                            标识,
                            类型名,
                            &描述.字段名,
                            Some(组件行区域标识(类型名, &描述.字段名)),
                        );
                        ui.label(
                            RichText::new(上下文.语言.取(描述.显示名)).color(上下文.配色.次要文字),
                        )
                        .on_hover_text(格式提示(描述, 上下文.语言));
                    });

                    ui.horizontal(|ui| {
                        if 描述.只读 {
                            ui.label(
                                RichText::new(值摘要(&当前, 上下文.语言))
                                    .monospace()
                                    .color(上下文.配色.禁用文字),
                            );
                            return;
                        }
                        let mut 新值 = 当前.clone();
                        if 按描述编辑(
                            ui,
                            &描述.字段名,
                            &描述.种类,
                            &mut 新值,
                            上下文,
                            Some((标识, 类型名)),
                        ) && 新值 != 当前
                        {
                            新值.收拢();
                            提交属性(上下文, 标识, 类型名, &描述.字段名, 新值);
                        }
                    });
                    ui.end_row();
                }
            });
    });
}

/// 组合悬停提示。
///
/// 材质编辑器共用它：提示里带上「字段名 + 类型」的格式在两处必须一致，
/// 否则同一个字段在两个面板里 hover 出两种信息。
pub(crate) fn 格式提示(描述: &属性描述, 语言: crate::i18n::语言) -> String {
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
///
/// 材质编辑器（`material_editor.rs`）复用这个函数渲染 `.material.ron` 的字段，
/// 所以它是 `pub(crate)` 而不是私有：一套属性模型只配有一套字段控件。
///
/// # `归属` 的用途
///
/// 资源字段需要把"用户点了选择"翻译成带落点的 [`UiAction::打开资源选择器`]。
/// 列表/结构的递归字段拿不到可写回的顶层落点，因此传 `None`：
/// 它们照常显示当前值，但不提供选择按钮 —— 给嵌套字段弹选择器却把结果
/// 写到顶层属性，会让整份列表被一个资源值覆盖，是比"没有按钮"更糟的缺陷。
///
/// # 材质字段为什么多一个「编辑」
///
/// 引用本身只是一条路径，而用户在 Inspector 里发现「这个物体的材质不对」时，
/// 真正想调的是那份材质，回头去资源面板翻文件是最长的路。因此类别为材质且
/// 引用存在时，就在清除按钮旁边给出入口；它只排一个
/// [`UiAction::打开材质编辑器`]，读盘仍归动作层。
pub(crate) fn 按描述编辑(
    ui: &mut Ui,
    字段名: &str,
    种类: &属性种类,
    值: &mut 属性值,
    上下文: &mut 绘制上下文<'_>,
    归属: Option<(实体标识, &str)>,
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
            // 引用还在文档里、但注册表里没有对应文件 = 资源缺失。
            // 不从路径直接 `stat` 磁盘：界面层不做文件 IO，注册表就是
            // "资源还在不在"的唯一快照来源。
            let 缺失 = 当前
                .as_ref()
                .is_some_and(|引用| 上下文.编辑器.资源注册表.取(&引用.路径).is_none());
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
                        if 缺失 {
                            ui.label(RichText::new("⚠").color(上下文.配色.错误))
                                .on_hover_text(上下文.语言.取("asset_picker_missing"));
                        }
                        let 路径文本 = 引用.路径.文本().to_string();
                        ui.label(RichText::new(引用.路径.文件名()).small().color(if 缺失 {
                            上下文.配色.错误
                        } else {
                            上下文.配色.文字
                        }))
                        .on_hover_text(路径文本);
                        if ui.small_button(上下文.语言.取("clear_2")).clicked() {
                            *值 = 属性值::空;
                            改变 = true;
                        }
                        if *类别 == 资源类别::材质
                            && ui
                                .small_button(上下文.语言.取("material_edit"))
                                .on_hover_text(上下文.语言.取("material_edit_hint"))
                                .clicked()
                        {
                            上下文.动作.推(UiAction::打开材质编辑器(
                                引用.路径.文本().to_string(),
                            ));
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new(上下文.语言.取("not_set")).color(上下文.配色.次要文字),
                        );
                    }
                }
                if let Some((实体, 组件)) = 归属
                    && ui
                        .small_button(上下文.语言.取("asset_picker_browse"))
                        .clicked()
                {
                    上下文.动作.推(UiAction::打开资源选择器 {
                        实体,
                        组件: 组件.to_string(),
                        属性: 字段名.to_string(),
                        类别: *类别,
                    });
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
                        if 按描述编辑(ui, &format!("{字段名}-{下标}"), 元素, 项, 上下文, None)
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
                        if 按描述编辑(
                            ui,
                            &子描述.字段名,
                            &子描述.种类,
                            &mut 子值,
                            上下文,
                            None,
                        ) {
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

    /// 组件行的区域标识必须稳定、可原样匹配，且不同字段互不顶替。
    ///
    /// 这是 GUI 探针能点到组件字段按钮的前提：探针按字符串找区域，
    /// 标识一旦每次都变（或两个字段撞同一个标识），点击就会落空或点错行。
    #[test]
    fn 组件行区域标识稳定且区分字段() {
        let 颜色 = 组件行区域标识("精灵", "颜色");
        assert_eq!(颜色, "inspector_revert_field.精灵.颜色");
        // 同一对键拿到的是同一个驻留字符串，而不是每次新建。
        assert!(std::ptr::eq(颜色, 组件行区域标识("精灵", "颜色")));
        // 组件或字段任一不同，都必须是另一个标识。
        assert_ne!(颜色, 组件行区域标识("精灵", "缩放"));
        assert_ne!(颜色, 组件行区域标识("碰撞体", "颜色"));
        // 不与变换三行的既有标识冲突（它们共用点击命名空间）。
        assert_ne!(颜色, "inspector_revert_pos");
        assert_ne!(颜色, "inspector_revert_rot");
        assert_ne!(颜色, "inspector_revert_scale");
    }

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
