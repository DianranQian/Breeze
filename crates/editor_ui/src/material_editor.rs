//! 材质编辑器浮层：把一份 `.material.ron` 摊成一列可编辑的字段。
//!
//! # 为什么复用 Inspector 的字段控件而不是另画一套
//!
//! 材质文档与场景组件在数据层是同一套属性模型（`scene_doc::registry::属性描述`
//! + 「字段名 → `scene_doc::property::属性值`」），`editor_core::material::材质资源`
//! 已经交出这份描述。于是这里唯一的工作是**排一次 Grid、写两个按钮**，
//! 再给资源字段补一个「选择…」入口：
//! 浮点拖动、颜色选择、贴图缩略图、缺失警告这些行为与 Inspector 里逐字一致，
//! 将来控件加个范围提示或改个配色，材质编辑器自动跟着变，不存在第二处要同步的地方。
//! 反过来，为材质新画一套控件只会得到两个会慢慢长歪的 Inspector。
//!
//! # 为什么「选择…」按钮画在这里而不是 Controls 里
//!
//! Inspector 的 [`按描述编辑`] 只在拿到 `(实体, 组件)` 归属时才画选择按钮，
//! 而材质没有实体可以写入 —— 它要给的是另一条动作（写回材质草稿）。在
//! 控件里再开一个"落点"参数会让 Inspector 的每个调用点都被动改动；把按钮
//! 留给知道落点的这一层，控件只负责显示与手工编辑。
//!
//! # 为什么不做成停靠面板
//!
//! `面板注册表` 里每个面板都要占一个 `Tab` 变体、一条工作区预设、一个分发分支，
//! 而材质编辑器是**跟着某个文件打开的临时视图**，不是"总想看的东西"——
//! 与主题编辑器、资源选择器同一类，所以走浮层：`界面状态.材质编辑` 存在即打开。
//!
//! # 为什么这里不碰磁盘
//!
//! 与瓦片地图面板同一分工：本模块只改内存里的草稿，读盘与写盘排成
//! [`UiAction::打开材质编辑器`] / [`UiAction::保存材质编辑器`] 交给动作层。
//! 「代次刷新」「项目相对路径校验」因此只有一处实现，测试也只需在动作层断一次。

use egui::{Grid, Layout, RichText, Window};
use scene_doc::registry::属性种类;

use crate::inspector_panel::{按描述编辑, 格式提示};
use crate::state::{UiAction, 材质编辑会话, 绘制上下文};
use crate::visual_spec;
use editor_core::material::材质资源;

/// 浮层在自动验收快照里的区域标识。
///
/// 前缀 `float_` 是硬要求：验收脚本按它判断"这块区域不属于任何停靠面板"，
/// 少了前缀就会被当成缺失的面板布局。
const 区域标识: &str = "float_material_editor";

/// 绘制材质编辑器浮层；`界面状态.材质编辑` 为 `None` 时整块不画。
///
/// # 每帧的工作
///
/// 1. 克隆会话草稿 —— 立即模式下草稿必须与 `界面状态` 分开持有，
///    否则 `按描述编辑` 借用 `上下文` 时会在同一帧里两次可变借用界面状态；
/// 2. 按 [`材质资源::字段描述`] 逐行画控件，控件报"改了"就把新值写回本地草稿
///    并把会话标脏；
/// 3. 结尾把本地草稿写回 `界面状态`，并按按钮排出保存/关闭动作。
///
/// 第 3 步先写回再排动作，是为了让动作层拿到的一定是本帧最新的那份草稿。
pub(crate) fn 绘制(egui上下文: &egui::Context, 上下文: &mut 绘制上下文<'_>) {
    let Some(会话) = 上下文.界面.材质编辑.clone() else {
        return;
    };
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 标题 = 语言.填充("material_editor_title", &[&会话.文件名()]);
    let 路径 = 会话.路径;
    let mut 属性 = 会话.属性;
    let mut 脏 = 会话.脏;
    let mut 待确认丢弃 = 会话.待确认丢弃;
    let mut 要保存 = false;
    let mut 要关闭 = false;

    let 字段描述 = 材质资源::字段描述();

    let 窗口响应 = Window::new(标题)
        .id(egui::Id::new(区域标识))
        .resizable(true)
        .default_width(360.0)
        .show(egui上下文, |ui| {
            Grid::new("material_editor_fields")
                .num_columns(2)
                .spacing([visual_spec::留白::标签, 4.0])
                .min_col_width(visual_spec::留白::标签列宽)
                .show(ui, |ui| {
                    for 描述 in &字段描述 {
                        let mut 值 = 属性
                            .get(&描述.字段名)
                            .cloned()
                            .unwrap_or_else(|| 描述.默认值.clone());
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(语言.取(描述.显示名)).color(配色.次要文字))
                                .on_hover_text(格式提示(描述, 语言));
                        });
                        ui.horizontal(|ui| {
                            if 按描述编辑(ui, &描述.字段名, &描述.种类, &mut 值, 上下文, None)
                            {
                                值.收拢();
                                脏 = true;
                                待确认丢弃 = false;
                            }
                            // 四个贴图槽都是 `属性种类::资源`，一律给选择入口；
                            // 类别由字段描述决定，选择器只放行图片。
                            if let 属性种类::资源 { 类别 } = &描述.种类
                                && ui.small_button(语言.取("asset_picker_browse")).clicked()
                            {
                                上下文.动作.推(UiAction::打开材质资源选择器 {
                                    材质路径: 路径.clone(),
                                    属性: 描述.字段名.clone(),
                                    类别: *类别,
                                });
                            }
                        });
                        属性.insert(描述.字段名.clone(), 值);
                        ui.end_row();
                    }
                });

            ui.label(
                RichText::new(语言.取("material_editor_texture_note"))
                    .small()
                    .color(配色.次要文字),
            );

            ui.separator();
            ui.horizontal(|ui| {
                if 脏 {
                    ui.label(RichText::new(语言.取("unsaved")).small().color(配色.警告));
                }
                if ui.button(语言.取("save")).clicked() {
                    要保存 = true;
                }
                if 脏 && !待确认丢弃 {
                    if ui.button(语言.取("close")).clicked() {
                        待确认丢弃 = true;
                    }
                } else {
                    if ui
                        .button(语言.取("material_discard"))
                        .on_hover_text(语言.取("material_discard_confirm"))
                        .clicked()
                    {
                        要关闭 = true;
                    }
                }
            });
        });

    crate::记录浮层区域(
        上下文.界面,
        egui上下文.viewport_rect(),
        区域标识,
        窗口响应.map(|响应| 响应.response.rect),
    );

    上下文.界面.材质编辑 = Some(材质编辑会话 {
        路径: 路径.clone(),
        属性: 属性.clone(),
        脏,
        待确认丢弃,
    });

    if 要保存 {
        上下文.动作.推(UiAction::保存材质编辑器 {
            路径,
            材质: 材质资源::从属性(&属性),
        });
    }
    if 要关闭 {
        上下文.动作.推(UiAction::关闭材质编辑器);
    }
}
