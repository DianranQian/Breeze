//! 中文名：项目文件树面板
//!
//! 把项目里的源码摊成一棵可折叠的树，双击文件直接送进代码面板。
//!
//! # 为什么单独一个面板，而不是塞进资源浏览器
//!
//! 资源浏览器是**资产**的入口（`assets/`）：图片、音频、场景、材质，
//! 每个条目都带缩略图与导入状态。源码是另一种东西 —— 它没有缩略图、
//! 不参与导入清单、数量却可能是资产的十倍。混在一个面板里，用户找
//! `main.rs` 要在一堆图片里翻，找贴图又要在 `src/` 的目录里翻。
//!
//! 更根本的是**工作流不同**：资产是"拖进视口"，源码是"双击打开、
//! 改完保存"。两条动线放进同一个列表，必然有一半交互是多余的。
//!
//! # 折叠状态为什么在界面状态里
//!
//! 折叠纯属视图偏好：它不进场景、不进项目清单，也不该让扫描结果变得
//! "有状态"。因此节点树由 [`editor_core::project_tree`] 每次给出完整
//! 结果，折叠集合由 [`界面状态`] 单独记 —— 重扫之后折叠状态自然还在，
//! 只有显式刷新会清空（目录可能已经被外部工具删掉了）。
//!
//! [`界面状态`]: crate::state::界面状态
//!
//! # 双击而不是单击
//!
//! 单击是列表的通用语义，用户用它"看一眼这是什么"。打开文件是**有代价
//! 的操作**（切模式、聚焦面板、替换当前文档），必须是一个明确意图 ——
//! 双击是各平台文件管理器共有的"打开"手势。

use egui::{RichText, Ui};

use editor_core::project_tree::{代码语言, 文件树节点};

use crate::icons::图标;
use crate::state::{UiAction, 绘制上下文};

/// 每一层缩进的像素宽度。
const 层级缩进: f32 = 12.0;

/// 目录折叠箭头的宽度（固定，保证同级名称左对齐）。
const 箭头宽: f32 = 16.0;

/// 文件/目录图标的绘制尺寸。
const 图标尺寸: f32 = 14.0;

/// 绘制项目文件树面板。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言表 = 上下文.语言;
    let 配色 = 上下文.配色;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let 响应 = 上下文.图标.图标按钮(
            ui,
            图标::刷新,
            Some(语言表.取("project_tree_refresh")),
            16.0,
            配色.文字,
            true,
        );
        if 响应
            .on_hover_text(语言表.取("project_tree_refresh_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::刷新项目文件树);
        }
        ui.label(
            RichText::new(语言表.取("project_tree_refresh"))
                .small()
                .color(配色.次要文字),
        );
    });

    let Some(项目) = 上下文.编辑器.项目.as_ref() else {
        ui.add_space(12.0);
        ui.label(RichText::new(语言表.取("project_tree_no_project")).color(配色.次要文字));
        return;
    };

    let 树 = 上下文.界面.文件树(Some(项目));
    ui.label(
        RichText::new(语言表.填充("project_tree_file_count", &[&树.文件数()]))
            .small()
            .color(配色.次要文字),
    );
    ui.separator();

    if 树.为空() {
        ui.add_space(12.0);
        ui.label(RichText::new(语言表.取("project_tree_empty")).color(配色.次要文字));
        return;
    }

    let 当前路径 = 上下文
        .界面
        .脚本
        .当前文档()
        .and_then(|文档| 文档.路径.as_ref())
        .map(|路径| 路径.文本().to_string());

    egui::ScrollArea::vertical()
        .id_salt("project_tree_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            绘制节点列表(ui, 上下文, &树.根项, 0, 当前路径.as_deref());
        });
}

/// 绘制一层节点（根层或某个目录的子项）。
fn 绘制节点列表(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    节点们: &[文件树节点],
    深度: usize,
    当前路径: Option<&str>,
) {
    for 节点 in 节点们 {
        match 节点 {
            文件树节点::目录 {
                名称, 路径, 子项
            } => {
                绘制目录(ui, 上下文, 名称, 路径, 子项, 深度, 当前路径);
            }
            文件树节点::文件 {
                名称, 路径, 语言
            } => {
                绘制文件(ui, 上下文, 名称, 路径, *语言, 深度, 当前路径);
            }
        }
    }
}

/// 绘制一个目录行；展开时递归绘制子项。
fn 绘制目录(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    名称: &str,
    路径: &str,
    子项: &[文件树节点],
    深度: usize,
    当前路径: Option<&str>,
) {
    let 配色 = 上下文.配色;
    let 折叠 = 上下文.界面.文件树已折叠(路径);

    ui.horizontal(|ui| {
        ui.add_space(深度 as f32 * 层级缩进);
        let 箭头 = if 折叠 { "▸" } else { "▾" };
        let 箭头响应 = ui.add(
            egui::Button::new(RichText::new(箭头).small().color(配色.次要文字))
                .frame(false)
                .min_size(egui::Vec2::new(箭头宽, 0.0)),
        );
        上下文
            .图标
            .绘制(ui, 图标::打开文件夹, 图标尺寸, 配色.次要文字);
        let 名称响应 = ui.selectable_label(false, RichText::new(名称).small().color(配色.文字));
        let 数量 = 子项.iter().map(|节点| 节点.文件数()).sum::<usize>();
        if 数量 > 0 {
            ui.label(
                RichText::new(上下文.语言.填充("project_tree_dir_count", &[&数量]))
                    .small()
                    .color(配色.禁用文字),
            );
        }
        if 箭头响应.clicked() || 名称响应.clicked() {
            上下文.界面.切换文件树折叠(路径);
        }
    });

    if !折叠 {
        绘制节点列表(ui, 上下文, 子项, 深度 + 1, 当前路径);
    }
}

/// 绘制一个文件行；双击把它送进代码面板。
fn 绘制文件(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    名称: &str,
    路径: &str,
    语言类型: 代码语言,
    深度: usize,
    当前路径: Option<&str>,
) {
    let 配色 = 上下文.配色;
    let 选中 = 当前路径 == Some(路径);
    ui.horizontal(|ui| {
        ui.add_space(深度 as f32 * 层级缩进 + 箭头宽);
        上下文
            .图标
            .绘制(ui, 文件图标(语言类型), 图标尺寸, 配色.次要文字);
        let 文本 =
            RichText::new(名称)
                .small()
                .color(if 选中 { 配色.强调 } else { 配色.文字 });
        let 响应 = ui.selectable_label(选中, 文本);
        if 响应.double_clicked() {
            上下文.动作.推(打开文件动作(路径));
        }
        // 路径足够长时标签会被截断成同名文件看不出区别，悬停给出全路径。
        响应.on_hover_text(路径);
    });
}

/// 把项目相对路径文本转成打开动作。
///
/// 路径来自扫描结果（`/` 分隔、以项目根为基准），解析失败说明树里混进了
/// 非法路径 —— 那是扫描器的缺陷。这里直接构造而不做"解析失败就忽略"的
/// 兜底：宁可让测试失败，也不让"点了没反应"悄悄上线。
fn 打开文件动作(路径: &str) -> UiAction {
    let 相对 = scene_doc::path::项目相对路径::解析(路径)
        .expect("文件树里的路径一定来自扫描器，必须是合法的项目相对路径");
    UiAction::打开脚本(相对)
}

/// 语言 → 图标。
fn 文件图标(语言: 代码语言) -> 图标 {
    match 语言 {
        代码语言::Rust | 代码语言::着色器 => 图标::代码,
        代码语言::Ron | 代码语言::Toml | 代码语言::Json => 图标::文本,
        代码语言::Markdown | 代码语言::其他 => 图标::文本,
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// `目录/文件` 的路径文本必须能原样换成打开动作。
    #[test]
    fn 树里的路径能直接转成打开动作() {
        assert_eq!(
            打开文件动作("src/main.rs"),
            UiAction::打开脚本(
                scene_doc::path::项目相对路径::解析("src/main.rs").expect("合法路径")
            )
        );
    }

    /// 语言到图标的映射：Rust 与着色器给"代码"，其余给"文本"。
    #[test]
    fn 文件图标按语言区分() {
        assert_eq!(文件图标(代码语言::Rust), 图标::代码);
        assert_eq!(文件图标(代码语言::着色器), 图标::代码);
        assert_eq!(文件图标(代码语言::Markdown), 图标::文本);
        assert_eq!(文件图标(代码语言::Toml), 图标::文本);
    }
}
