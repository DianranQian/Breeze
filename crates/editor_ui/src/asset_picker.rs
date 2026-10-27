//! 资源选择器：为 Inspector 的资源字段从资源注册表里挑一个文件。
//!
//! # 为什么不是系统文件对话框
//!
//! 资源字段只能引用项目 `assets/` 里的东西。系统对话框选中的绝对路径
//! 可能在工作区之外，导入、稳定标识与打包都不认它。这里读的是资源注册表
//! 快照，与资源面板看到的是同一份数据；文件在外部被移动后，按 UID 回连
//! 的路径也仍然由注册表统一给出。
//!
//! # 选中之后不在这里写文档
//!
//! 本模块只返回 [`UiAction::设置资源引用`] 或 [`UiAction::设置材质资源引用`]，
//! 由动作层反查路径并走各自的写回通道。组件字段走编辑命令，撤销、脏标记与
//! 预览刷新因此与 Inspector 的普通编辑共用同一条通道，不会出现"选择器改的
//! 属性撤销不掉"这种两套历史；材质字段落进材质草稿，由保存材质那一步统一写盘。
//!
//! # 类别筛选不是一个可选装饰
//!
//! 资源字段声明的类别是数据契约：把模型塞进贴图槽，预览与运行时会一起
//! 失效。因此默认只显示字段要求的类别；切到「全部」时，类别不匹配的条目
//! 仍然可见但不可选，并给出原因，而不是直接从列表里消失让用户以为文件没了。
//!
//! # 两种视图：列表认名字，网格认样子
//!
//! 网格卡片直接显示应用层解码好的缩略图。缓存还没填上（首次打开、图片很大、
//! 解码失败，或者这个类别本来就没有缩略图）时**退化**成大号类别图标 + 文件名，
//! 而不是一块空洞 —— 用户仍然要能看出这里有个资源、它是什么类别、能不能点。
//! 因此本模块不自己读盘解码：缩略图由 `editor_app` 的缓存填 [`绘制`] 的参数，
//! 界面层再一次解码同一张 PNG 只会多出第二套缓存与第二份解码开销。

use std::collections::BTreeMap;

use egui::{RichText, Ui};

use editor_core::asset_registry::注册条目;
use editor_core::state::编辑器状态;
use scene_doc::property::{属性值, 资源引用值, 资源类别};
use uuid::Uuid;

use crate::agent_store::截断文本;
use crate::i18n;
use crate::state::{
    UiAction, 属性编辑, 界面状态, 资源落点, 资源选择器状态, 选择器卡片几何, 选择器视图,
};
use crate::theme::编辑器配色;

/// 卡片宽度（逻辑点）：够放截断到 13 个字符的文件名。
const 卡片宽: f32 = 104.0;
/// 卡片高度：图片区 + 名字行 + 内边距。
const 卡片高: f32 = 76.0;

/// 绘制资源选择器；返回需要排进动作队列的意图。
pub fn 绘制(
    egui上下文: &egui::Context,
    编辑器: &编辑器状态,
    界面: &mut 界面状态,
    缩略图: &BTreeMap<String, egui::TextureHandle>,
    配色: 编辑器配色,
    语言: i18n::语言,
) -> Option<UiAction> {
    let 草稿 = 界面.资源选择器.clone()?;
    let mut 搜索 = 草稿.搜索.clone();
    let mut 筛选 = 草稿.类别筛选;
    let mut 视图 = 草稿.视图;
    let 当前 = 当前引用(编辑器, 界面, &草稿);
    let mut 要选 = None;
    let mut 要清空 = false;
    let mut 要关闭 = false;

    // 卡片几何是「本帧事实」：这一帧画了几张就记几张。在这里清零而不是只依赖
    // 界面主循环清一次，是为了让浮层被反复调用（单元测试、单帧重绘）时也不会
    // 累积出上一帧的残留卡片。
    界面.选择器卡片几何.clear();

    let 窗口响应 = egui::Window::new(语言.取("asset_picker_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .show(egui上下文, |ui: &mut Ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(语言.取("asset_picker_search")).color(配色.次要文字));
                ui.add(
                    egui::TextEdit::singleline(&mut 搜索)
                        .hint_text(语言.取("asset_picker_search_hint"))
                        .desired_width(220.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(
                            视图 == 选择器视图::网格,
                            语言.取("asset_picker_view_grid"),
                        )
                        .clicked()
                    {
                        视图 = 选择器视图::网格;
                    }
                    if ui
                        .selectable_label(
                            视图 == 选择器视图::列表,
                            语言.取("asset_picker_view_list"),
                        )
                        .clicked()
                    {
                        视图 = 选择器视图::列表;
                    }
                });
            });

            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(筛选.is_none(), 语言.取("asset_picker_all"))
                    .clicked()
                {
                    筛选 = None;
                }
                for 类别 in 全部类别() {
                    if ui
                        .selectable_label(筛选 == Some(类别), 语言.取(类别.词条键()))
                        .clicked()
                    {
                        筛选 = Some(类别);
                    }
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(语言.取("asset_picker_current"))
                        .small()
                        .color(配色.次要文字),
                );
                match &当前 {
                    Some(引用) => {
                        let 存在 = 编辑器.资源注册表.取(&引用.路径).is_some();
                        if !存在 {
                            ui.label(RichText::new("⚠").color(配色.错误))
                                .on_hover_text(语言.取("asset_picker_missing"));
                        }
                        ui.label(
                            RichText::new(引用.路径.文本())
                                .small()
                                .monospace()
                                .color(if 存在 { 配色.文字 } else { 配色.错误 }),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new(语言.取("not_set"))
                                .small()
                                .color(配色.次要文字),
                        );
                    }
                }
            });
            ui.separator();

            let 条目: Vec<&注册条目> = 编辑器
                .资源注册表
                .条目()
                .iter()
                .filter(|条目| 筛选.map_or(true, |类别| 条目.类别 == 类别))
                .filter(|条目| 模糊匹配(条目.路径.文件名(), &搜索))
                .collect();

            ui.label(
                RichText::new(语言.填充("asset_picker_count", &[&条目.len()]))
                    .small()
                    .color(配色.次要文字),
            );

            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    if 条目.is_empty() {
                        ui.label(RichText::new(语言.取("asset_picker_empty")).color(配色.次要文字));
                    }
                    match 视图 {
                        选择器视图::列表 => {
                            for 条目 in 条目 {
                                if let Some(标识) = 选择行(ui, 条目, &草稿, 缩略图, 配色, 语言)
                                {
                                    要选 = Some(标识);
                                }
                            }
                        }
                        选择器视图::网格 => {
                            ui.horizontal_wrapped(|ui| {
                                for 条目 in 条目 {
                                    if let Some(标识) = 网格卡片(
                                        ui,
                                        条目,
                                        &草稿,
                                        缩略图,
                                        配色,
                                        语言,
                                        &mut 界面.选择器卡片几何,
                                    ) {
                                        要选 = Some(标识);
                                    }
                                }
                            });
                        }
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(语言.取("clear_2")).clicked() {
                    要清空 = true;
                }
                if ui.button(语言.取("close")).clicked() {
                    要关闭 = true;
                }
            });
        });

    crate::记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_asset_picker",
        窗口响应.map(|响应| 响应.response.rect),
    );

    let 结束 = 要关闭 || 要清空 || 要选.is_some();
    界面.资源选择器 = if 结束 {
        None
    } else {
        Some(资源选择器状态 {
            落点: 草稿.落点.clone(),
            属性: 草稿.属性.clone(),
            类别: 草稿.类别,
            搜索,
            类别筛选: 筛选,
            视图,
        })
    };

    if let Some(标识) = 要选 {
        return Some(match &草稿.落点 {
            资源落点::组件字段 { 实体, 组件 } => UiAction::设置资源引用 {
                实体: *实体,
                组件: 组件.clone(),
                属性: 草稿.属性.clone(),
                资源标识: 标识,
            },
            资源落点::材质字段 { 路径 } => UiAction::设置材质资源引用 {
                材质路径: 路径.clone(),
                属性: 草稿.属性.clone(),
                资源标识: Some(标识),
            },
        });
    }
    if 要清空 {
        return Some(match &草稿.落点 {
            资源落点::组件字段 { 实体, 组件 } => UiAction::修改属性(属性编辑 {
                实体: *实体,
                组件: 组件.clone(),
                属性: 草稿.属性.clone(),
                新值: 属性值::空,
            }),
            资源落点::材质字段 { 路径 } => UiAction::设置材质资源引用 {
                材质路径: 路径.clone(),
                属性: 草稿.属性.clone(),
                资源标识: None,
            },
        });
    }
    None
}

/// 绘制一行资源；可选中时返回它的稳定标识。
fn 选择行(
    ui: &mut Ui,
    条目: &注册条目,
    草稿: &资源选择器状态,
    缩略图: &BTreeMap<String, egui::TextureHandle>,
    配色: 编辑器配色,
    语言: i18n::语言,
) -> Option<Uuid> {
    let 类别匹配 = 条目.类别 == 草稿.类别;
    let (可选, 标识) = 卡片可选(条目, 草稿);
    let mut 结果 = None;

    ui.horizontal(|ui| {
        if 条目.类别 == 资源类别::图片 {
            if let Some(纹理) = 缩略图.get(条目.路径.文本()) {
                ui.add(egui::Image::new(纹理).fit_to_exact_size(egui::vec2(16.0, 16.0)));
            } else {
                ui.label(RichText::new("🖼").small());
            }
        } else {
            ui.label(RichText::new(类别图标(条目.类别)).small());
        }

        let 文字 = if 可选 {
            RichText::new(条目.路径.文件名()).color(配色.文字)
        } else {
            RichText::new(条目.路径.文件名()).color(配色.禁用文字)
        };
        let 响应 = ui.add_enabled(可选, egui::Button::new(文字).frame(false));
        if !类别匹配 {
            响应.on_hover_text(语言.填充(
                "asset_picker_category_mismatch",
                &[&语言.取(草稿.类别.词条键())],
            ));
        } else if 标识.is_none() {
            响应.on_hover_text(语言.取("asset_picker_no_uid"));
        } else if 响应.on_hover_text(条目.路径.文本().to_string()).clicked() {
            结果 = 标识;
        }
    });

    结果
}

/**
 * 这张资源能不能被选中，以及选中后回写哪个稳定标识。
 *
 * 抽成不碰 egui 的纯函数有两个理由：
 * 1. 视图有两种（列表与网格），判定必须同源，否则会出现「列表里能点、
 *    网格里同一个文件是灰的」这种两侧各执一词的分裂；
 * 2. 无鼠标测试要能直接断言规则，不必先造一遍 egui 事件流。
 *
 * 两条拒绝理由各自独立：类别不匹配是数据契约（把模型塞进贴图槽会让预览与
 * 运行时一起失效），缺稳定标识是引用可靠性（没有标识的引用在文件改名后接不回来）。
 */
fn 卡片可选(条目: &注册条目, 草稿: &资源选择器状态) -> (bool, Option<Uuid>) {
    if 条目.类别 != 草稿.类别 {
        return (false, None);
    }
    match 条目.唯一标识 {
        Some(标识) => (true, Some(标识)),
        None => (false, None),
    }
}

/**
 * 绘制网格视图里的一张卡片；可选中时返回它的稳定标识。
 *
 * 卡片自己占一个 `Sense::click` 的矩形，背板与悬停边框由 painter 画；
 * 里面的缩略图与文字一律**不吃点击**（标签 `selectable(false)`、图片只有悬停
 * 感知），否则 egui 的文字选择控件会挡在卡片前面，点上去没反应。
 *
 * 缩略图未命中缓存时退化成放大的类别图标，卡片不留空洞：用户仍可凭名字与
 * 角标判断这是哪个资源。几何一并记进 `界面.选择器卡片几何`，让自动验收按
 * 真实矩形点，而不是按「第 n 张卡」猜坐标。
 */
fn 网格卡片(
    ui: &mut Ui,
    条目: &注册条目,
    草稿: &资源选择器状态,
    缩略图: &BTreeMap<String, egui::TextureHandle>,
    配色: 编辑器配色,
    语言: i18n::语言,
    几何: &mut Vec<选择器卡片几何>,
) -> Option<Uuid> {
    let 类别匹配 = 条目.类别 == 草稿.类别;
    let (可选, 标识) = 卡片可选(条目, 草稿);
    let (矩形, 响应) = ui.allocate_exact_size(egui::vec2(卡片宽, 卡片高), egui::Sense::click());
    几何.push(选择器卡片几何 {
        路径: 条目.路径.文本().to_string(),
        卡片矩形: 矩形,
        可选,
    });

    let 填充 = if !可选 {
        配色.面板次背景
    } else if 响应.hovered() {
        配色.悬停背景
    } else {
        配色.输入框背景
    };
    let 圆角 = egui::CornerRadius::same(6);
    ui.painter().rect_filled(矩形, 圆角, 填充);
    if 可选 && 响应.hovered() {
        ui.painter().rect_stroke(
            矩形,
            圆角,
            egui::Stroke::new(1.0, 配色.边框强),
            egui::StrokeKind::Inside,
        );
    }

    let 文字色 = if 可选 {
        配色.文字
    } else {
        配色.禁用文字
    };
    let 图区 = egui::Rect::from_min_size(
        矩形.min + egui::vec2(6.0, 8.0),
        egui::vec2(卡片宽 - 12.0, 34.0),
    );
    match 缩略图.get(条目.路径.文本()) {
        Some(纹理) => {
            egui::Image::new(纹理)
                .fit_to_exact_size(图区.size())
                .paint_at(ui, 图区);
        }
        None => {
            ui.put(
                图区,
                egui::Label::new(RichText::new(类别图标(条目.类别)).size(26.0).color(文字色))
                    .halign(egui::Align::Center)
                    .selectable(false),
            );
        }
    }

    // 名字单独一行；超长文件名按字符截断，完整路径放在悬停提示里。
    let 名字区 = egui::Rect::from_min_size(
        egui::pos2(矩形.min.x + 4.0, 图区.max.y + 4.0),
        egui::vec2(卡片宽 - 8.0, 16.0),
    );
    ui.put(
        名字区,
        egui::Label::new(
            RichText::new(截断文本(条目.路径.文件名(), 13))
                .small()
                .color(文字色),
        )
        .halign(egui::Align::Center)
        .selectable(false),
    );

    // 右上角类别角标：贴图与材质在缩略图还没出来时，靠它区分得最快。
    ui.put(
        egui::Rect::from_min_size(
            egui::pos2(矩形.max.x - 20.0, 矩形.min.y + 3.0),
            egui::vec2(18.0, 14.0),
        ),
        egui::Label::new(
            RichText::new(类别图标(条目.类别))
                .small()
                .color(配色.次要文字),
        )
        .selectable(false),
    );

    if !类别匹配 {
        响应.on_hover_text(语言.填充(
            "asset_picker_category_mismatch",
            &[&语言.取(草稿.类别.词条键())],
        ));
    } else if 标识.is_none() {
        响应.on_hover_text(语言.取("asset_picker_no_uid"));
    } else if 响应.on_hover_text(条目.路径.文本().to_string()).clicked() {
        return 标识;
    }
    None
}

/// 读取字段当前引用的资源。
///
/// 两类落点的"当前值"来自不同的地方：组件字段在场景文档里，材质字段在
/// `界面.材质编辑` 的草稿里。草稿里没有这个键时返回 `None` —— 那代表
/// 这个槽还没被碰过，显示成「未设置」比显示一个默认引用更诚实。
fn 当前引用(
    编辑器: &编辑器状态,
    界面: &界面状态,
    草稿: &资源选择器状态,
) -> Option<资源引用值> {
    match &草稿.落点 {
        资源落点::组件字段 { 实体, 组件 } => 编辑器
            .会话
            .文档
            .取(*实体)?
            .取组件(组件)?
            .属性
            .get(&草稿.属性)?
            .取资源()
            .cloned(),
        资源落点::材质字段 { 路径 } => 界面
            .材质编辑
            .as_ref()
            .filter(|会话| 会话.路径 == *路径)?
            .属性
            .get(&草稿.属性)?
            .取资源()
            .cloned(),
    }
}

/// 页签顺序：字段最常用的三类排在最前，其余按固定顺序展开。
fn 全部类别() -> [资源类别; 8] {
    [
        资源类别::图片,
        资源类别::模型,
        资源类别::材质,
        资源类别::音频,
        资源类别::字体,
        资源类别::场景,
        资源类别::着色器,
        资源类别::其他,
    ]
}

/// 类别角标。图片在列表里优先显示真实缩略图，这里只是兜底。
fn 类别图标(类别: 资源类别) -> &'static str {
    match 类别 {
        资源类别::图片 => "🖼",
        资源类别::音频 => "🔊",
        资源类别::字体 => "🔤",
        资源类别::模型 => "🧊",
        资源类别::材质 => "🎨",
        资源类别::场景 => "🎬",
        资源类别::着色器 => "✨",
        资源类别::其他 => "📄",
    }
}

/// 文件名模糊匹配：忽略大小写，按字符顺序允许跳过。
///
/// 用子序列而不是 `contains`：用户记得"屋顶"和"png"，但中间还夹着
/// 目录或编号时，`contains` 会直接判不匹配。空查询匹配全部。
fn 模糊匹配(文本: &str, 查询: &str) -> bool {
    let 文本: Vec<char> = 文本.to_lowercase().chars().collect();
    let mut 游标 = 0;
    for 字符 in 查询.to_lowercase().chars() {
        if 字符.is_whitespace() {
            continue;
        }
        let Some(偏移) = 文本[游标..].iter().position(|候选| *候选 == 字符) else {
            return false;
        };
        游标 += 偏移 + 1;
    }
    true
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::实体标识;
    use scene_doc::path::项目相对路径;

    #[test]
    fn 模糊匹配忽略大小写并允许跳过字符() {
        assert!(模糊匹配("Rooftop.PNG", "roof"));
        assert!(模糊匹配("Rooftop.PNG", "rftp"));
        assert!(模糊匹配("屋顶贴图.png", "屋图"));
        assert!(模糊匹配("屋顶贴图.png", ""));
    }

    #[test]
    fn 顺序不符或字符缺失时不匹配() {
        assert!(!模糊匹配("屋顶贴图.png", "图屋"));
        assert!(!模糊匹配("屋顶贴图.png", "屋顶墙"));
    }

    fn 条目(类别: 资源类别, 标识: Option<Uuid>) -> 注册条目 {
        注册条目 {
            路径: 项目相对路径::解析("assets/materials/石板.material.ron").expect("路径"),
            类别,
            大小: 12,
            修改时间: None,
            唯一标识: 标识,
        }
    }

    fn 草稿(类别: 资源类别) -> 资源选择器状态 {
        资源选择器状态::新建(
            实体标识(Uuid::nil()),
            "Mesh3d".to_string(),
            "材质".to_string(),
            类别,
        )
    }

    #[test]
    fn 类别相符且有标识的卡片才可选() {
        let 标识 = Uuid::from_u128(7);
        assert_eq!(
            卡片可选(&条目(资源类别::材质, Some(标识)), &草稿(资源类别::材质)),
            (true, Some(标识))
        );
    }

    #[test]
    fn 类别不符或缺标识时不可选也不回写标识() {
        let 标识 = Uuid::from_u128(8);
        // 材质字段塞图片会让预览与运行时一起失效：类别是数据契约，不是样式。
        assert_eq!(
            卡片可选(&条目(资源类别::图片, Some(标识)), &草稿(资源类别::材质)),
            (false, None)
        );
        // 类别相符但还没进标识表：选进去的引用在文件改名后接不回来。
        assert_eq!(
            卡片可选(&条目(资源类别::材质, None), &草稿(资源类别::材质)),
            (false, None)
        );
    }
}
