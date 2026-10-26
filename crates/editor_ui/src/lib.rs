//! Bevy Studio 编辑器界面层。
//!
//! # 这一层的职责边界
//!
//! * **只读** [`editor_core::state::编辑器状态`]；
//! * 把所有用户意图翻译成 [`state::UiAction`] 排进队列；
//! * 自己的临时状态（视口相机、面板布局、搜索框）存在
//!   [`state::界面状态`] 里，**绝不写进场景文档**。
//!
//! 这样做的结果是：整个界面层可以在没有 Bevy、没有窗口的环境下编译，
//! 面板里的纯逻辑（场景树摊平、资源树构建、菜单可用性、历史列表排序）
//! 都可以直接用单元测试验证。

pub mod agent;
pub mod agent_panel;
pub mod agent_permission;
pub mod agent_repair;
pub mod agent_review;
pub mod agent_store;
pub mod arrange_bar;
pub mod asset_preview;
pub mod asset_refs_panel;
pub mod assets_panel;
pub mod build_panel;
pub mod collision_overlay;
pub mod console_panel;
pub mod dev_mode_bar;
pub mod dev_modes;
pub mod docks;
pub mod fonts;
pub mod gizmo;
pub mod gizmo2d;
pub mod gizmo3d;
pub mod help;
pub mod hierarchy_panel;
pub mod history_panel;
pub mod icons;
pub mod inspector_panel;
pub mod menu_bar;
pub mod node_icons;
pub mod picking3d;
pub mod prefab_panel;
pub mod recovery_panel;
pub mod runtime_panel;
pub mod savegame_panel;
pub mod script;
pub mod state;
pub mod theme;
pub mod theme_editor;
pub mod tilemap_panel;
pub mod timeline_panel;
pub mod viewport3d;
pub mod viewport_panel;
pub mod visual_spec;
pub mod workbench_panel;
pub mod workspace_bar;
pub mod workspaces;

// 多语言入口定义在 `editor_core`（它需要给命令显示名做本地化），
// 这里把模块整个再导出，于是界面代码仍可写 `crate::i18n::语言`。
pub use editor_core::i18n;
// 常用类型的便捷再导出，避免调用方到处写 `i18n::`。
pub use dev_modes::开发模式;
pub use docks::Tab;
pub use editor_core::i18n::语言;
pub use fonts::{
    内置字体, 内置字体大小, 内置字体字节, 后备字体环境变量, 字体安装结果, 安装中文字体,
    构造字体定义, 界面代表性中文,
};
pub use icons::{图标, 图标库};
pub use node_icons::{折叠指示, 节点类型};
pub use state::{
    UiAction, 动作队列, 动画关键帧引用, 区域快照, 图片预览信息, 场景行几何, 场景预览信息,
    场景预览实体, 字体预览信息, 属性编辑, 控制台过滤, 文本编辑目标, 文本预览信息, 模型预览信息,
    界面状态, 绘制上下文, 视口拖拽, 视口渲染结果, 视口相机, 资源删除草稿, 资源排序, 资源行几何,
    资源重命名草稿, 资源预览快照, 运行时诊断, 选择意图, 音频预览信息, 预览画面, 预览请求, 预览阶段,
};
pub use theme::{主题, 主题库, 外观, 明暗, 编辑器配色};

use egui::{Ui, WidgetText};

/// 绘制整个编辑器界面。
///
/// 调用方（`editor_app`）负责在绘制前准备好 [`绘制上下文`]，
/// 绘制后用 [`动作队列::取出`] 拿到全部意图并执行。
///
/// # egui 0.36 的面板用法（与旧版本差异较大）
///
/// 这个版本做了两处影响结构的改动：
///
/// 1. `TopBottomPanel` / `SidePanel` 被统一成了 [`egui::Panel`]
///    （`Panel::top` / `bottom` / `left` / `right`），尺寸设置也从
///    `exact_height` 改成了 `exact_size` / `default_size`；
/// 2. 面板的 `show` 不再接收 `&Context`，而是接收**父级的 `&mut Ui`**：
///    面板从父 `Ui` 里扣除自己占据的空间，父 `Ui` 的剩余区域才是
///    下一个面板可用的区域。
///
/// 因此这里三层嵌套：根 `Ui` → 三个边栏 → 中央的可停靠区域。
/// 顺序（上、上、下、中央）会影响实际布局，不能随意调换。
pub fn 绘制(egui上下文: &egui::Context, 上下文: &mut 绘制上下文<'_>) {
    // 主题先落地：样式、缩放、字号、圆角、停靠样式都在这里一次性设置好，
    // 之后所有绘制只读 `上下文.配色` 的语义槽位。
    上下文.主题.应用(egui上下文);
    let 配色 = 上下文.配色;

    // 浮层（对齐条）先于 DockArea 绘制，拿不到本帧视口矩形；
    // 在清空之前把它存进界面状态，供浮层用上一帧的位置。
    上下文.界面.上一帧视口矩形 = 上下文
        .界面
        .布局快照
        .iter()
        .find(|区域| 区域.标识 == "viewport")
        .map(|区域| 区域.矩形);

    // 每帧重新测量。清空而不是复用：区域数量会随面板开关变化，
    // 复用会让已经关闭的面板以旧矩形残留在证据文件里。
    上下文.界面.布局快照.clear();

    // 场景树的四类"本帧事实"同理，而且**必须在这里**清空。
    //
    // 它们由场景树面板在绘制时填写。如果只在面板内部清空，那么面板一旦
    // 被关掉（或所在工作区不显示它），就再也没有代码去清空它们 ——
    // 证据会继续声称"这些行还在 / 右键菜单还开着 / 滚动视口还在"，
    // 而那正是最需要看清真相的时刻。
    上下文.界面.场景行几何.clear();
    // 资源行几何同理：资源面板没画（面板被关掉/换了工作区）时，
    // 证据里不能继续声称"这些行还在、还能点"。
    上下文.界面.资源行几何.clear();
    // 预制件行几何同理：预制件面板被关掉时，证据里不能留下"还能点"的行。
    上下文.界面.预制件行几何.clear();
    上下文.界面.场景右键菜单 = None;
    上下文.界面.场景空白矩形 = None;
    上下文.界面.场景视口矩形 = None;
    // 操纵器的悬停与部件几何同样是"本帧事实"：视口没画它，就不该继续留在
    // 证据里。拖拽会话**不**在这里清 —— 它必须跨帧存活到松手或取消。
    上下文.界面.操纵器悬停 = None;
    上下文.界面.操纵器部件几何.clear();

    let 视口矩形 = egui上下文.viewport_rect();
    let mut 根界面 = Ui::new(
        egui上下文.clone(),
        egui::Id::new("bevy_studio_根界面"),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(视口矩形),
    );

    // 把整个绘制上下文临时搬进 `Option`，这样闭包里可以用 `take()`
    // 一次性取得所有权，构造查看器后再可变借用它。
    //
    // 直接 `&mut *上下文` 是不行的：`CentralPanel::show` 的闭包即使只需要
    // `FnOnce`，闭包捕获的 `&mut` 也只能被使用一次，
    // 而下面要连续调用 4 次绘制方法。用 `take()` 把"借用"变成"移出"，
    // 借用检查器就没有可抱怨的地方了。
    let mut 待用 = Some(上下文);

    egui::CentralPanel::no_frame()
        .frame(egui::Frame::NONE.fill(配色.面板背景))
        .show(&mut 根界面, |界面ui| {
            let Some(绘制上下文) = 待用.take() else {
                return;
            };

            // 取出可停靠布局，交给 `DockArea`。绘制结束后放回去。
            let mut 停靠 = std::mem::replace(
                绘制上下文.界面.停靠_mut(),
                egui_dock::DockState::new(Vec::new()),
            );

            // 停靠样式必须由主题生成：否则用户在主题里改的标签色、
            // 分隔条颜色不会出现在面板上（那是"主题只改了一半"的典型表现）。
            let 停靠样式 = 绘制上下文.主题.dock样式(界面ui.style().as_ref());

            let mut 查看器 = 面板查看器 {
                上下文: 绘制上下文
            };

            // 顶部 chrome 的视觉规则：
            //
            // * 菜单栏与模式栏**共用一层底色**，看起来是一条完整的标题区，
            //   而不是两条各自带分割线的横条；
            // * 工作区栏退到次一级底色，它管的是"面板怎么摆"，比模式更次要；
            // * 工具栏回到面板底色，并用一条 1px 细线收边 —— 工具按钮自己
            //   有描边，工具栏再套一层边框就成了"框里套框"。
            //
            // 四块区域各自保留稳定标识（menubar / modebar / workspacebar /
            // toolbar）：自动验收按这些标识取矩形并断言垂直顺序，
            // 合并行会直接让那几条断言失效。
            let 标题区底色 = 配色.面板次背景;
            let 顶部框 = |底色: egui::Color32| {
                egui::Frame::NONE
                    .fill(底色)
                    .inner_margin(egui::Margin::ZERO)
            };

            // 顶部：菜单栏
            egui::Panel::top("bevy_studio_菜单栏")
                .exact_size(crate::visual_spec::高度::菜单栏)
                .frame(顶部框(标题区底色))
                .show_separator_line(false)
                .show(界面ui, |ui| {
                    查看器.记录区域("menubar", ui.max_rect());
                    查看器.绘制菜单栏(ui);
                });

            // 顶部：导航行（左=编辑模式，右=工作区）
            //
            // 模式与工作区合并成一行：它们都是"当前上下文"，分开占两行
            // 会让顶部堆成四层横条。两个区域仍然各自记录矩形
            // （`modebar` / `workspacebar`），自动验收的垂直顺序断言照旧成立。
            egui::Panel::top("bevy_studio_mode_bar")
                .exact_size(crate::visual_spec::高度::模式栏)
                .frame(顶部框(标题区底色))
                .show_separator_line(false)
                .show(界面ui, |ui| {
                    查看器.记录区域("modebar", ui.max_rect());
                    查看器.绘制模式栏(ui);
                });

            // 顶部：工具栏
            egui::Panel::top("bevy_studio_工具栏")
                .exact_size(crate::visual_spec::高度::工具栏)
                .frame(顶部框(配色.面板背景))
                .show(界面ui, |ui| {
                    查看器.记录区域("toolbar", ui.max_rect());
                    查看器.绘制工具栏(ui);
                });

            // 底部：状态栏
            egui::Panel::bottom("bevy_studio_状态栏")
                .exact_size(crate::visual_spec::高度::状态栏)
                .frame(顶部框(配色.面板次背景))
                .show_separator_line(false)
                .show(界面ui, |ui| {
                    查看器.记录区域("statusbar", ui.max_rect());
                    查看器.绘制状态栏(ui);
                });

            // 浮层：两个对话框。放在这里是为了能在同一帧内得到确认结果，
            // 且不会与外层 `绘制` 的借用冲突。
            查看器.绘制浮层(egui上下文);

            // 其余区域：可停靠面板
            egui_dock::DockArea::new(&mut 停靠)
                .style(停靠样式)
                // 逐标签的关闭按钮**关掉**：每个标签常亮一个 ✕，紧挨着
                // 面板右上角的 ✕，窄窗口下两枚按钮几乎重叠，误关整个面板的
                // 概率明显高于收益。关闭入口保留：面板级 ✕（egui_dock 自带）
                // 与「窗口」菜单都能收起面板。
                //
                // 代价：标签栏右侧少了一个按钮，标签因此更宽 —— 这正好缓解
                // 右侧面板标签溢出（5 个标签在 2400 宽下本来会把「性能」挤掉）。
                .show_close_buttons(false)
                // 折叠按钮：把整个面板折成一条标题栏，腾出屏幕空间做编辑。
                // 这是"面板可以折叠"这条要求的实现。
                .show_leaf_collapse_buttons(true)
                // 标签可拖动、可重新停靠、可拖成独立窗口。
                .draggable_tabs(true)
                // 隐藏标签栏：给视口这类"单独占一个面板"的标签腾出高度。
                .hidable_tab_bars(true)
                // 不加号按钮：新面板统一从「窗口」菜单打开，
                // 避免用户在一个面板组里堆出十几个标签。
                .show_add_buttons(false)
                .show_inside(界面ui, &mut 查看器);

            // 放回可停靠布局。
            查看器.上下文.界面.设定当前布局(停靠);
        });

    // 处理"新建项目"对话框的确认结果。
    if let Some(绘制上下文) = 待用.as_mut()
        && 绘制上下文.界面.显示新建项目确认
    {
        绘制上下文.界面.显示新建项目确认 = false;
        let 名称 = 绘制上下文.界面.新建项目名.trim().to_string();
        let 名称 = if 名称.is_empty() {
            绘制上下文.语言.取("my_game").to_string()
        } else {
            名称
        };
        绘制上下文.动作.推(UiAction::新建项目 { 名称 });
        绘制上下文.界面.显示新建项目 = false;
    }
}

/// 场景的显示名：从未保存过时按界面语言显示"未命名"占位。
///
/// # 为什么要单独一个函数
///
/// 场景名属于**文档数据**，默认值在 `editor_core` 里（与界面语言无关）；
/// 而"显示成什么"属于界面层。这个函数是两者之间唯一的转换点，
/// 菜单栏、场景树、关于窗口都调用它，因此不会再出现
/// "某处英文界面残留中文"的情况。
pub fn 场景占位名(编辑器: &editor_core::state::编辑器状态, 语言: 语言) -> String {
    if 编辑器.场景名是占位() {
        语言.取("untitled_scene").to_string()
    } else {
        编辑器.场景名().to_string()
    }
}

/// 状态栏条目的语义色调。
///
/// 不直接存 `Color32`：状态栏条目要先被算出来（便于自动验收与单元测试
/// 断言"英文模式下这里没有中文"），再由绘制函数按当前主题上色。
/// 若在这里固化成具体颜色，换主题时就得重新计算一遍条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 条目色调 {
    /// 主文字。
    正常,
    /// 次要文字（计数、路径）。
    次要,
    /// 提示/警告。
    警告,
    /// 错误。
    错误,
    /// 成功。
    成功,
}

/// 状态栏的一个条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 状态栏条目 {
    /// 显示文本。
    pub 文本: String,
    /// 语义色调。
    pub 色调: 条目色调,
}

/// 状态栏的两侧内容。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 状态栏内容 {
    /// 左侧条目（保存状态、计数、场景路径）。
    pub 左侧: Vec<状态栏条目>,
    /// 右侧条目（问题数、帧率、缩放）。
    pub 右侧: Vec<状态栏条目>,
}

/// 计算当前状态栏应显示的内容。
///
/// 抽成纯函数的直接收益：自动验收工具可以把这些文本写进证据文件，
/// 从而在**不看图**的前提下断言"英文界面里没有中文残留""问题数为 0 时
/// 不显示问题条目"。绘制与验收读的是同一份计算，不会各说各话。
pub fn 计算状态栏内容(
    编辑器: &editor_core::state::编辑器状态,
    语言: 语言,
    正在运行: bool,
    运行时: &crate::state::运行时诊断,
    缩放百分比: f32,
) -> 状态栏内容 {
    let 脏 = 编辑器.有未保存修改();
    // 路径条目在"尚未关联文件"时退化为一句提示，这与改造前一致：
    // 状态栏永远显示四段信息，用户不会看到某一格突然消失。
    let 左侧 = vec![
        状态栏条目 {
            文本: 语言.取(if 脏 { "unsaved" } else { "saved" }).to_string(),
            色调: if 脏 {
                条目色调::警告
            } else {
                条目色调::次要
            },
        },
        状态栏条目 {
            文本: 语言.填充("entities", &[&编辑器.会话.文档.实体数量()]),
            色调: 条目色调::次要,
        },
        状态栏条目 {
            文本: 语言.填充("selected", &[&编辑器.选择.数量()]),
            色调: 条目色调::次要,
        },
        状态栏条目 {
            文本: 编辑器
                .会话
                .路径
                .as_ref()
                .map(|路径| 路径.文本().to_string())
                .unwrap_or_else(|| 语言.取("not_saved_to_a_file_yet").to_string()),
            色调: 条目色调::次要,
        },
    ];

    // 「已保存 / 未保存」是同一个语义位，只留一条。
    // 上面刻意用 if 表达式保证只有一条。
    let mut 右侧 = Vec::new();
    if 正在运行 {
        右侧.push(状态栏条目 {
            文本: format!("{:.0} FPS · {:.1} ms", 运行时.帧率, 运行时.帧耗时毫秒),
            色调: 条目色调::次要,
        });
    }
    let 问题数 = 编辑器.问题数量();
    if 问题数 > 0 {
        右侧.push(状态栏条目 {
            文本: 语言.填充("problems", &[&问题数]),
            色调: 条目色调::警告,
        });
    }
    右侧.push(状态栏条目 {
        文本: format!("{:.0}%", 缩放百分比),
        色调: 条目色调::次要,
    });

    状态栏内容 { 左侧, 右侧 }
}

/// 底部状态栏。
fn 绘制_状态栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 内容 = 计算状态栏内容(
        上下文.编辑器,
        上下文.语言,
        上下文.正在运行,
        上下文.运行时信息,
        上下文.界面.相机.缩放 * 100.0,
    );

    let 取色 = |色调: 条目色调| match 色调 {
        条目色调::正常 => 上下文.配色.文字,
        条目色调::次要 => 上下文.配色.次要文字,
        条目色调::警告 => 上下文.配色.警告,
        条目色调::错误 => 上下文.配色.错误,
        条目色调::成功 => 上下文.配色.成功,
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // 保存状态用图标先给一个"形状级"的提示：扫一眼就知道有没有未保存改动。
        let (保存图标, 保存色) = if 上下文.编辑器.有未保存修改() {
            (图标::保存, 上下文.配色.警告)
        } else {
            (图标::成功, 上下文.配色.成功)
        };
        上下文
            .图标
            .绘制(ui, 保存图标, 上下文.外观.图标尺寸 * 0.85, 保存色);
        for (下标, 条目) in 内容.左侧.iter().enumerate() {
            if 下标 > 0 {
                ui.separator();
            }
            ui.label(
                egui::RichText::new(&条目.文本)
                    .small()
                    .color(取色(条目.色调)),
            );
        }

        // 右侧：从右往左排，与 egui 的布局方向一致。
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (下标, 条目) in 内容.右侧.iter().enumerate() {
                if 下标 > 0 {
                    ui.separator();
                }
                ui.label(
                    egui::RichText::new(&条目.文本)
                        .small()
                        .color(取色(条目.色调)),
                );
            }
        });
    });
}

/// 浮层区域 id 的前缀（与 `gui_probe` 的约定）。
///
/// # 为什么需要这个前缀
///
/// 自动验收有一条"每块区域都画出了内容"的断言。浮窗（主题编辑器、关于、
/// 新建项目）会**盖住**它下面的面板与工具栏按钮：被盖住的区域在截图里
/// 是一块单色，于是被判定为"控件没画出来"。
///
/// 那是一条**假阴性**：它把"窗口挡住了"报成"渲染坏了"，既让人去修一个
/// 不存在的缺陷，又会把真正的渲染缺陷淹没在同一份清单里。
///
/// 用前缀把浮层区域单独标出来，验收工具就能把"被盖住的区域"如实列出来并
/// 跳过内容判定，而不是把它算成失败。
pub const 浮层区域前缀: &str = "float_";

/// 上报一块**浮层区域**（只服务自动验收，不参与任何布局决策）。
///
/// 矩形先与视口求交：浮窗可以被拖到窗口外面，而"区域越出截图"是另一条
/// 独立断言，不该因为一个被拖到边上的窗口而误报。
pub(crate) fn 记录浮层区域(
    界面: &mut 界面状态,
    视口: egui::Rect,
    标识: &'static str,
    矩形: Option<egui::Rect>,
) {
    let Some(矩形) = 矩形 else {
        return;
    };
    let 裁剪 = 矩形.intersect(视口);
    if !裁剪.is_positive() {
        return;
    }
    界面.布局快照.push(crate::state::区域快照 {
        标识, 矩形: 裁剪
    });
}

/// 新建项目对话框。
///
/// 只借用 [`界面状态`] 与配色：它不需要读场景数据，
/// 也没有必要拿到 `&mut 动作队列` —— 确认动作由调用方在绘制后统一排出，
/// 这样对话框本身是纯展示 + 取输入的，易于推理。
fn 绘制_新建项目对话框(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    语言: crate::i18n::语言,
) {
    if !界面.显示新建项目 {
        return;
    }
    // 关闭状态用 `Rc<Cell<bool>>` 共享：`Window::open` 需要一个
    // `&mut bool` 且借用会持续到 `show` 返回，而窗口内容闭包又需要能改它。
    // 共享所有权让两处各自拿到一份，互不冲突。
    let 打开 = std::rc::Rc::new(std::cell::Cell::new(true));
    let 打开给关闭按钮 = 打开.clone();
    let mut 打开值 = 打开.get();
    let mut 确认 = false;
    // 项目名先借出来编辑：`Window` 的闭包需要 `&mut String`，
    // 直接借用 `界面.新建项目名` 会与下面读 `界面.显示新建项目` 冲突。
    let mut 编辑中名称 = 界面.新建项目名.clone();
    let 窗口响应 = egui::Window::new(语言.取("new_project"))
        .collapsible(false)
        .resizable(false)
        .open(&mut 打开值)
        .show(egui上下文, |ui| {
            ui.label(语言.取("project_name"));
            ui.add(egui::TextEdit::singleline(&mut 编辑中名称).desired_width(240.0));
            ui.label(
                egui::RichText::new(语言.取("new_project_target_dir_hint"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(语言.取("choose_directory_and_create")).clicked() {
                    确认 = true;
                }
                if ui.button(语言.取("cancel")).clicked() {
                    打开给关闭按钮.set(false);
                }
            });
        });

    // `open` 的返回值（标题栏关闭按钮）优先，其次是窗口内的取消按钮。
    let 仍然打开 = 打开值 && 打开.get();
    界面.新建项目名 = 编辑中名称;
    记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_new_project",
        窗口响应.map(|响应| 响应.response.rect),
    );

    if 确认 {
        界面.显示新建项目确认 = true;
        return;
    }
    if !仍然打开 {
        界面.显示新建项目 = false;
    }
}

/// 「项目设置」对话框。
///
/// # 为什么对话框改的是草稿而不是清单
///
/// 用户可能改了一半就取消。若直接改项目清单，取消就要靠"先备份再还原"，
/// 字段一多必然漏。这里编辑的是 [`界面状态::项目设置草稿`]：
/// 只有点「保存」才把整份草稿交给动作层校验并写回清单，
/// 校验失败时错误信息回到对话框里（而不是只写进控制台）。
///
/// # 返回值
///
/// 返回 `Some(动作)` 表示用户点了保存；调用方负责排进动作队列。
fn 绘制_项目设置对话框(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    项目根目录: Option<&std::path::Path>,
    配色: 编辑器配色,
    语言: crate::i18n::语言,
) -> Option<UiAction> {
    if !界面.显示项目设置 {
        return None;
    }
    let mut 打开 = true;
    let mut 要保存 = false;
    let mut 要取消 = false;
    let mut 草稿 = 界面.项目设置草稿.clone().unwrap_or_default();
    let 错误 = 界面.项目设置错误.clone();

    let 窗口响应 = egui::Window::new(语言.取("project_settings"))
        .collapsible(false)
        .resizable(true)
        .default_width(460.0)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if let Some(根) = 项目根目录 {
                ui.label(
                    egui::RichText::new(根.display().to_string())
                        .small()
                        .monospace()
                        .color(配色.次要文字),
                );
            }
            ui.separator();

            egui::Grid::new("project_settings_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(语言.取("project_name"));
                    ui.add(egui::TextEdit::singleline(&mut 草稿.名称).desired_width(300.0));
                    ui.end_row();

                    ui.label(语言.取("project_description"));
                    ui.add(
                        egui::TextEdit::multiline(&mut 草稿.说明)
                            .desired_rows(2)
                            .desired_width(300.0),
                    );
                    ui.end_row();

                    ui.label(语言.取("start_scene"));
                    let mut 启动场景 = 草稿.启动场景.clone().unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut 启动场景)
                                .hint_text("scenes/main.scene.ron")
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        草稿.启动场景 = Some(启动场景).filter(|文本| !文本.trim().is_empty());
                    }
                    ui.end_row();
                });

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(语言.取("run_target_title"))
                    .strong()
                    .color(配色.文字),
            );
            egui::Grid::new("project_settings_run_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(语言.取("run_target_program"));
                    ui.add(
                        egui::TextEdit::singleline(&mut 草稿.运行程序)
                            .hint_text(语言.取("run_target_program_hint"))
                            .desired_width(300.0),
                    );
                    ui.end_row();

                    ui.label(语言.取("run_target_args"));
                    let mut 参数文本 = 草稿.运行参数.join("\n");
                    if ui
                        .add(
                            egui::TextEdit::multiline(&mut 参数文本)
                                .hint_text(语言.取("run_target_args_hint"))
                                .desired_rows(2)
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        草稿.运行参数 = 参数文本
                            .lines()
                            .map(str::trim)
                            .filter(|行| !行.is_empty())
                            .map(str::to_string)
                            .collect();
                    }
                    ui.end_row();

                    ui.label(语言.取("run_target_working_dir"));
                    let mut 工作目录 = 草稿.运行工作目录.clone().unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut 工作目录)
                                .hint_text(语言.取("build_target_hint"))
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        草稿.运行工作目录 = Some(工作目录).filter(|文本| !文本.trim().is_empty());
                    }
                    ui.end_row();
                });

            ui.add_space(4.0);
            // 设计分辨率放在"显示"分组、构建分组之前：
            // 它决定 UI 的布局空间，是比"怎么构建"更靠前的项目契约。
            ui.label(
                egui::RichText::new(语言.取("display"))
                    .strong()
                    .color(配色.文字),
            );
            egui::Grid::new("project_settings_display_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(语言.取("design_resolution"))
                        .on_hover_text(语言.取("design_resolution_hint"));
                    ui.horizontal(|ui| {
                        let mut 宽 = 草稿.设计分辨率.0;
                        let mut 高 = 草稿.设计分辨率.1;
                        // 两个数字框都允许直接键入；任一改动都要写回草稿，
                        // 因此用一个 `mut` 标记而不是链式 `|`（`Response` 不支持）。
                        let mut 改变 = false;
                        改变 |= ui
                            .add(egui::DragValue::new(&mut 宽).range(1..=16384).speed(1.0))
                            .changed();
                        ui.label("×");
                        改变 |= ui
                            .add(egui::DragValue::new(&mut 高).range(1..=16384).speed(1.0))
                            .changed();
                        if 改变 {
                            草稿.设计分辨率 = (宽, 高);
                        }

                        // 常用预设：手打四个数字容易写错（例如把 1080 打成 1800），
                        // 常见尺寸直接选更快。当前值不在预设里时显示"自定义"。
                        const 预设: [(u32, u32); 4] =
                            [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)];
                        let 当前文本 = match 预设
                            .iter()
                            .find(|(宽, 高)| *宽 == 草稿.设计分辨率.0 && *高 == 草稿.设计分辨率.1)
                        {
                            Some((宽, 高)) => format!("{宽}×{高}"),
                            None => 语言.取("resolution_preset_custom").to_string(),
                        };
                        egui::ComboBox::from_id_salt("project_settings_resolution_preset")
                            .selected_text(当前文本)
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for (宽, 高) in 预设 {
                                    if ui
                                        .selectable_label(
                                            草稿.设计分辨率 == (宽, 高),
                                            format!("{宽}×{高}"),
                                        )
                                        .clicked()
                                    {
                                        草稿.设计分辨率 = (宽, 高);
                                    }
                                }
                            });
                    });
                    ui.end_row();
                });

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(语言.取("build"))
                    .strong()
                    .color(配色.文字),
            );
            egui::Grid::new("project_settings_build_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(语言.取("build_profile"));
                    let 当前 = match 草稿.构建.配置类型 {
                        editor_core::build::构建配置类型::Debug => {
                            语言.取("build_profile_debug")
                        }
                        editor_core::build::构建配置类型::Release => {
                            语言.取("build_profile_release")
                        }
                    };
                    egui::ComboBox::from_id_salt("project_settings_profile")
                        .selected_text(当前)
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            for 类型 in editor_core::build::构建配置类型::全部 {
                                let 名称 = match 类型 {
                                    editor_core::build::构建配置类型::Debug => {
                                        语言.取("build_profile_debug")
                                    }
                                    editor_core::build::构建配置类型::Release => {
                                        语言.取("build_profile_release")
                                    }
                                };
                                if ui
                                    .selectable_label(草稿.构建.配置类型 == 类型, 名称)
                                    .clicked()
                                {
                                    草稿.构建.配置类型 = 类型;
                                }
                            }
                        });
                    ui.end_row();

                    ui.label(语言.取("build_package"));
                    let mut 包名 = 草稿.构建.包名.clone().unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut 包名)
                                .hint_text(语言.取("build_package_hint"))
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        草稿.构建.包名 = Some(包名).filter(|值| !值.trim().is_empty());
                    }
                    ui.end_row();

                    ui.label(语言.取("build_features"));
                    let mut 特性 = 草稿.构建.特征.join(",");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut 特性)
                                .hint_text(语言.取("build_features_hint"))
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        草稿.构建.特征 = 特性
                            .split(',')
                            .map(str::trim)
                            .filter(|项| !项.is_empty())
                            .map(str::to_string)
                            .collect();
                    }
                    ui.end_row();

                    ui.label(语言.取("build_target"));
                    let mut 目标 = 草稿.构建.目标平台.clone().unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut 目标)
                                .hint_text(语言.取("build_target_hint"))
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        草稿.构建.目标平台 = Some(目标).filter(|值| !值.trim().is_empty());
                    }
                    ui.end_row();
                });

            ui.add_space(4.0);
            // 打包配置放在项目设置里：它是"这个项目怎么交付"的一部分，
            // 与构建配置同级；构建面板里那份是执行入口，这里是契约。
            ui.label(
                egui::RichText::new(语言.取("package_section"))
                    .strong()
                    .color(配色.文字),
            );
            egui::Grid::new("project_settings_package_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(语言.取("package_output_dir"))
                        .on_hover_text(语言.取("package_output_dir_hint"));
                    let mut 目录 = 草稿.打包.输出目录.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut 目录)
                                .hint_text(editor_core::package::默认输出目录)
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        草稿.打包.输出目录 = 目录;
                    }
                    ui.end_row();

                    ui.label(语言.取("package_all_scenes"));
                    ui.checkbox(
                        &mut 草稿.打包.包含全部场景,
                        语言.取("package_all_scenes_hint"),
                    );
                    ui.end_row();
                });

            if let Some(错误) = 错误.as_deref() {
                ui.separator();
                ui.label(egui::RichText::new(错误).small().color(配色.错误));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(语言.取("save")).clicked() {
                    要保存 = true;
                }
                if ui.button(语言.取("cancel")).clicked() {
                    要取消 = true;
                }
            });
        });

    界面.项目设置草稿 = Some(草稿.clone());
    记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_project_settings",
        窗口响应.map(|响应| 响应.response.rect),
    );

    if 要保存 {
        界面.显示项目设置 = false;
        界面.项目设置错误 = None;
        return Some(UiAction::保存项目设置(草稿));
    }
    if 要取消 || !打开 {
        界面.显示项目设置 = false;
        界面.项目设置错误 = None;
        界面.项目设置草稿 = None;
    }
    None
}

/// 资源重命名/移动对话框。
///
/// 输入的是**完整项目相对路径**，所以同一个框既能改名也能换目录。
/// 确认时不在界面里碰文件系统：只把意图排进动作队列，
/// 由动作层统一校验、移动文件、改写所有场景引用，失败时把原因写回草稿。
fn 绘制_资源重命名对话框(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    语言: crate::i18n::语言,
) -> Option<UiAction> {
    let mut 草稿 = 界面.资源重命名.clone()?;
    let mut 打开 = true;
    let mut 确认 = false;
    let mut 取消 = false;

    let 窗口响应 = egui::Window::new(语言.取("asset_rename_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(480.0)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(
                egui::RichText::new(草稿.旧路径.文本())
                    .small()
                    .monospace()
                    .color(配色.次要文字),
            );
            ui.separator();
            ui.label(语言.取("asset_rename_new_path"));
            ui.add(
                egui::TextEdit::singleline(&mut 草稿.新路径)
                    .desired_width(f32::INFINITY)
                    .hint_text("assets/new_name.png"),
            );
            ui.label(
                egui::RichText::new(语言.取("asset_rename_hint"))
                    .small()
                    .color(配色.次要文字),
            );
            if let Some(错误) = 草稿.错误.as_deref() {
                ui.separator();
                ui.label(egui::RichText::new(错误).small().color(配色.错误));
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(语言.取("confirm")).clicked() {
                    确认 = true;
                }
                if ui.button(语言.取("cancel")).clicked() {
                    取消 = true;
                }
            });
        });

    // 先写回草稿，再决定动作：确认时动作层要用最新的输入。
    界面.资源重命名 = Some(草稿.clone());
    记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_asset_rename",
        窗口响应.map(|响应| 响应.response.rect),
    );

    if 确认 {
        // 空输入或解析失败时不排动作，直接把错误留在对话框里。
        match scene_doc::path::项目相对路径::解析(&草稿.新路径) {
            Ok(新路径) => {
                return Some(UiAction::确认重命名资源 {
                    旧路径: 草稿.旧路径.clone(),
                    新路径,
                });
            }
            Err(错误) => {
                let 说明 = format!("{}", 错误);
                界面.资源重命名 = Some(state::资源重命名草稿 {
                    旧路径: 草稿.旧路径,
                    新路径: 草稿.新路径,
                    错误: Some(说明),
                });
            }
        }
    }
    if 取消 || !打开 {
        界面.资源重命名 = None;
        return Some(UiAction::关闭重命名资源);
    }
    None
}

/// 资源删除确认对话框。
///
/// 引用列表与阻止理由都由动作层在打开时算好，这里只展示。
/// 被引用时删除按钮**不出现**，只留"取消"—— 让危险操作没有可点的入口，
/// 比"点了再被拒绝"更清楚。
fn 绘制_资源删除对话框(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    语言: crate::i18n::语言,
) -> Option<UiAction> {
    let 草稿 = 界面.资源删除.clone()?;
    let mut 打开 = true;
    let mut 确认 = false;
    let mut 取消 = false;

    let 窗口响应 = egui::Window::new(语言.取("asset_delete_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            ui.label(
                egui::RichText::new(草稿.路径.文本())
                    .monospace()
                    .color(配色.文字),
            );
            ui.separator();

            match 草稿.阻止原因.as_deref() {
                Some(原因) => {
                    ui.label(
                        egui::RichText::new(语言.取("asset_delete_blocked"))
                            .strong()
                            .color(配色.错误),
                    );
                    ui.label(egui::RichText::new(原因).small().color(配色.文字));
                }
                None => {
                    ui.label(语言.取("asset_delete_confirm"));
                    if 草稿.引用.is_empty() {
                        ui.label(
                            egui::RichText::new(语言.取("asset_delete_no_refs"))
                                .small()
                                .color(配色.次要文字),
                        );
                    }
                }
            }

            if !草稿.引用.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for 行 in &草稿.引用 {
                            ui.label(
                                egui::RichText::new(行)
                                    .small()
                                    .monospace()
                                    .color(配色.次要文字),
                            );
                        }
                    });
            }

            if let Some(错误) = 草稿.错误.as_deref() {
                ui.separator();
                ui.label(egui::RichText::new(错误).small().color(配色.错误));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if 草稿.可删除() && ui.button(语言.取("delete")).clicked() {
                    确认 = true;
                }
                if ui.button(语言.取("cancel")).clicked() {
                    取消 = true;
                }
            });
        });

    记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_asset_delete",
        窗口响应.map(|响应| 响应.response.rect),
    );

    if 确认 {
        return Some(UiAction::确认删除资源(草稿.路径));
    }
    if 取消 || !打开 {
        界面.资源删除 = None;
        return Some(UiAction::关闭删除资源);
    }
    None
}

/// "关于"窗口。
///
/// 内容分三层：**这是什么**（名称、版本、一句话定位）、
/// **现在能做什么**（工作模式与主要能力）、
/// **边界与现场**（存储格式、仍属原型的部分、当前场景）。
///
/// 能力列表刻意保持"每一项都能在代码或测试里找到对应实现"。
/// 这里写下的每一句都是用户会当真去试的承诺，因此宁可少写，
/// 也不把计划中的功能写成已实现。
fn 绘制_关于窗口(
    egui上下文: &egui::Context,
    界面: &mut 界面状态,
    配色: 编辑器配色,
    场景名: &str,
    语言: crate::i18n::语言,
) {
    if !界面.显示关于 {
        return;
    }
    let mut 打开 = true;
    let 窗口响应 = egui::Window::new(语言.取("about_bevy_studio"))
        .collapsible(false)
        .resizable(true)
        .default_width(460.0)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            // 能力清单比早期版本长得多，窗口高度必须可滚动：
            // 内容被裁掉时用户会以为"关于里没写这个"，
            // 实际上只是看不见。
            egui::ScrollArea::vertical()
                .max_height(460.0)
                .show(ui, |ui| {
                    ui.heading(语言.取("app_title"));
                    ui.label(语言.填充("version", &[&env!("CARGO_PKG_VERSION")]));
                    ui.label(
                        egui::RichText::new(help::技术栈说明())
                            .small()
                            .color(配色.次要文字),
                    );
                    ui.separator();
                    ui.label(语言.取("about_tagline"));

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(语言.取("about_modes"))
                            .strong()
                            .color(配色.文字),
                    );
                    // 模式列表由模式注册表驱动，不在此处另抄一份：
                    // 新增或删除模式时，关于窗口自动跟着变。
                    for 模式 in 开发模式::全部() {
                        ui.label(
                            egui::RichText::new(format!(
                                "· {} — {}",
                                语言.取(模式.名称键()),
                                语言.取(模式.说明键())
                            ))
                            .small()
                            .color(配色.文字),
                        );
                    }

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(语言.取("about_capabilities"))
                            .strong()
                            .color(配色.文字),
                    );
                    ui.label(
                        egui::RichText::new(语言.取("about_implemented_features"))
                            .small()
                            .color(配色.文字),
                    );
                    ui.label(
                        egui::RichText::new(语言.取("about_unimplemented_note"))
                            .small()
                            .color(配色.次要文字),
                    );

                    ui.separator();
                    ui.label(
                        egui::RichText::new(语言.取("tech_stack"))
                            .strong()
                            .color(配色.文字),
                    );
                    ui.label(语言.取("about_ui_stack"));
                    ui.label(语言.取("about_scene_format"));
                    ui.separator();
                    ui.label(
                        egui::RichText::new(语言.填充("current_scene", &[&场景名]))
                            .small()
                            .color(配色.次要文字),
                    );
                });
        });
    if !打开 {
        界面.显示关于 = false;
    }
    记录浮层区域(
        界面,
        egui上下文.viewport_rect(),
        "float_about",
        窗口响应.map(|响应| 响应.response.rect),
    );
}

/// `egui_dock` 的标签查看器。
///
/// 它**拥有**一个 [`绘制上下文`]。之所以是拥有而不是借用：
/// `DockArea::new` 需要 `&mut 停靠布局`，而停靠布局就在
/// [`界面状态`] 内部；若查看器只是借用了 `绘制上下文`，
/// 就无法同时把停靠布局从里面取出来交给 `DockArea`。
/// 拥有它则可以在构造查看器之前先把停靠布局单独取出。
///
/// # 为什么是两个生命周期参数
///
/// `'借用` 是持有 `绘制上下文` 这一引用本身的寿命，
/// `'数据` 是 `绘制上下文` 内部各字段引用所指向数据的寿命。
/// 二者必须分开：把 `&'借用 mut 绘制上下文<'数据>` 写成同一个生命周期
/// （即 `&'a mut 绘制上下文<'a>`）会要求引用本身活得和它指向的数据一样久，
/// 而这个引用只是函数内的局部变量，编译会直接报 "lifetime may not live long enough"。
struct 面板查看器<'借用, '数据> {
    上下文: &'借用 mut 绘制上下文<'数据>,
}

impl 面板查看器<'_, '_> {
    /// 为一次面板绘制构造借用视图。
    ///
    /// 每次调用都重新借用字段，因此不会长时间占用 `界面状态`，
    /// 也就不会与 `DockArea` 对停靠布局的借用冲突。
    fn 视图(&mut self) -> 绘制上下文<'_> {
        // 每帧克隆一份主题：主题只有几百字节，而这样可以让面板同时拿到
        // `&mut 界面状态` 与"当前主题"，不必把状态拆成两半（见 `绘制上下文`）。
        let 主题 = self.上下文.主题.clone();
        let 配色 = 主题.配色;
        let 外观 = 主题.外观.收敛();
        绘制上下文 {
            编辑器: self.上下文.编辑器,
            语言: self.上下文.语言,
            界面: self.上下文.界面,
            动作: self.上下文.动作,
            主题,
            配色,
            外观,
            图标: self.上下文.图标,
            缩略图: self.上下文.缩略图,
            瓦片图集: self.上下文.瓦片图集,
            视口纹理: self.上下文.视口纹理,
            正在运行: self.上下文.正在运行,
            已暂停: self.上下文.已暂停,
            运行时信息: self.上下文.运行时信息,
            构建: self.上下文.构建,
            运行: self.上下文.运行,
            调试: self.上下文.调试,
            调试历史: self.上下文.调试历史,
            统一诊断: self.上下文.统一诊断,
            导入状态: self.上下文.导入状态,
            配置目录: self.上下文.配置目录,
        }
    }

    fn 绘制菜单栏(&mut self, ui: &mut Ui) {
        menu_bar::绘制(ui, &mut self.视图());
    }

    /// 记录一块区域的真实矩形（供自动验收与调试）。
    fn 记录区域(&mut self, 标识: &'static str, 矩形: egui::Rect) {
        self.上下文
            .界面
            .布局快照
            .push(crate::state::区域快照 { 标识, 矩形 });
    }

    fn 绘制工具栏(&mut self, ui: &mut Ui) {
        menu_bar::绘制工具栏(ui, &mut self.视图());
    }

    fn 绘制模式栏(&mut self, ui: &mut Ui) {
        let mut 视图 = self.视图();
        let mut 工作区矩形 = None;
        dev_mode_bar::绘制(ui, &mut 视图, |ui, 视图| {
            // 同一行右侧：工作区切换。它自己的矩形也进证据文件
            // （保留 `workspacebar` 标识，验收按它取几何）。
            工作区矩形 = Some(workspace_bar::行内绘制(ui, 视图));
        });
        if let Some(矩形) = 工作区矩形 {
            self.上下文.界面.布局快照.push(crate::state::区域快照 {
                标识: "workspacebar",
                矩形,
            });
        }
    }

    fn 绘制状态栏(&mut self, ui: &mut Ui) {
        let mut 视图 = self.视图();
        绘制_状态栏(ui, &mut 视图);
    }

    /// 绘制浮层窗口。
    fn 绘制浮层(&mut self, egui上下文: &egui::Context) {
        let 配色 = self.上下文.配色;
        let 语言 = self.上下文.语言;
        绘制_新建项目对话框(egui上下文, self.上下文.界面, 配色, 语言);
        绘制_关于窗口(
            egui上下文,
            self.上下文.界面,
            配色,
            self.上下文.编辑器.场景名(),
            语言,
        );
        // 工作区名称对话框：确认后把动作排进队列，由执行器统一处理。
        if let Some(动作) = workspace_bar::绘制对话框(egui上下文, self.上下文.界面, 配色, 语言)
        {
            self.上下文.动作.推(动作);
        }
        // 项目设置对话框：草稿改在界面状态里，只有点保存才整体写回清单。
        if let Some(动作) = 绘制_项目设置对话框(
            egui上下文,
            self.上下文.界面,
            self.上下文.编辑器.项目.as_ref().map(|项目| 项目.根目录()),
            配色,
            语言,
        ) {
            self.上下文.动作.推(动作);
        }
        // 多选对齐/分布：浮在视口右上角，不占布局。
        //
        // 放在浮层而不是视口绘制里，有两个原因：视口只负责画场景；
        // 而对齐条需要"视口矩形"，那正是布局快照里已经记下来的东西。
        let 视口矩形 = self.上下文.界面.上一帧视口矩形;
        if let Some(视口矩形) = 视口矩形
            && let Some(动作) = arrange_bar::绘制(egui上下文, self.上下文, 视口矩形, 配色, 语言)
        {
            self.上下文.动作.推(动作);
        }
        // 游戏存档槽位：只读浏览项目 `saves/`，不替玩家删改。
        if let Some(动作) = savegame_panel::绘制(egui上下文, self.上下文.界面, 配色, 语言)
        {
            self.上下文.动作.推(动作);
        }
        // 查找引用：资源被哪些场景/实体/字段引用，并可直接定位。
        if let Some(动作) = asset_refs_panel::绘制(egui上下文, self.上下文.界面, 配色, 语言)
        {
            self.上下文.动作.推(动作);
        }
        // 恢复自动保存：启动时发现的恢复点，只有用户点"恢复"才读内容。
        if let Some(动作) = recovery_panel::绘制(egui上下文, self.上下文.界面, 配色, 语言)
        {
            self.上下文.动作.推(动作);
        }
        // 资源重命名/删除：两者都以"草稿 + 动作"工作，界面不碰文件系统。
        if let Some(动作) =
            绘制_资源重命名对话框(egui上下文, self.上下文.界面, 配色, 语言)
        {
            self.上下文.动作.推(动作);
        }
        if let Some(动作) = 绘制_资源删除对话框(egui上下文, self.上下文.界面, 配色, 语言)
        {
            self.上下文.动作.推(动作);
        }
        // 帮助窗口：快捷键、路径与诊断入口。
        if let Some(动作) = help::绘制帮助窗口(
            egui上下文,
            self.上下文.界面,
            配色,
            语言,
            self.上下文.配置目录,
            self.上下文.编辑器.项目.as_ref().map(|项目| 项目.根目录()),
        ) {
            self.上下文.动作.推(动作);
        }
        // 主题编辑器：读写草稿，并把保存/取消等动作排进队列。
        theme_editor::绘制(
            egui上下文,
            self.上下文.界面,
            配色,
            语言,
            self.上下文.图标,
            self.上下文.外观.图标尺寸,
            self.上下文.动作,
        );
    }
}

impl egui_dock::TabViewer for 面板查看器<'_, '_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> WidgetText {
        // 控制台标签上带问题计数角标，让用户在别的面板时也能察觉错误。
        if *tab == Tab::控制台 {
            let 问题数 = self.上下文.编辑器.问题数量();
            if 问题数 > 0 {
                return WidgetText::from(
                    egui::RichText::new(self.上下文.语言.填充("console_2", &[&问题数]))
                        .color(self.上下文.配色.警告),
                );
            }
        }
        WidgetText::from(tab.标题(self.上下文.语言))
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        // 记录每个叶子实际拿到的矩形，并与面板身份绑定。
        // 自动验收工具用它把截图里的像素区域与面板对上号；
        // 它同时也是"fraction 方向写反"这类缺陷的可观测输出。
        self.记录区域(tab.菜单键(), ui.max_rect());

        let mut 上下文 = self.视图();

        // 视口是唯一"内容要贴边"的面板：它画的是场景，四周留白只会让
        // 可用面积变小。其余面板统一套一层内边距，让所有面板的文字左边缘
        // 落在同一条竖线上 —— 这是"界面是一整套"最容易被看出来的地方。
        let 是视口 = *tab == Tab::视口;
        if 是视口 {
            // 提示条改成浮层（见 `viewport_panel::绘制提示`），
            // 因此视口拿到的是整个面板矩形，不再被顶部提示行截去一行。
            let 可用 = ui.available_rect_before_wrap();
            viewport_panel::绘制内容(ui, &mut 上下文, 可用);
            // 提示最后画，压在场景之上（它是浮层，不占布局空间）。
            // 三维视口自带一套"轨道相机"的操作说明（见 `viewport3d`），
            // 这里再画 2D 的说明就会出现两条不一样的提示，反而让人不知道该读哪条。
            if !viewport3d::活动(&上下文) {
                viewport_panel::绘制提示(ui, &上下文);
            }
        } else {
            egui::Frame::NONE
                .inner_margin(crate::visual_spec::面板内边距())
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    match tab {
                        Tab::场景树 => hierarchy_panel::绘制(ui, &mut 上下文),
                        Tab::检查器 => inspector_panel::绘制(ui, &mut 上下文),
                        Tab::资源浏览器 => assets_panel::绘制(ui, &mut 上下文),
                        Tab::预制件 => prefab_panel::绘制(ui, &mut 上下文),
                        Tab::控制台 => console_panel::绘制(ui, &mut 上下文),
                        Tab::历史 => history_panel::绘制历史(ui, &mut 上下文),
                        Tab::运行时ecs => runtime_panel::绘制(ui, &mut 上下文),
                        Tab::性能 => history_panel::绘制性能(ui, &mut 上下文),
                        Tab::脚本 => script::绘制(ui, &mut 上下文),
                        Tab::构建 => build_panel::绘制(ui, &mut 上下文),
                        Tab::运行 => workbench_panel::绘制(ui, &mut 上下文),
                        Tab::时间轴 => timeline_panel::绘制(ui, &mut 上下文),
                        Tab::瓦片地图 => tilemap_panel::绘制(ui, &mut 上下文),
                        Tab::智能体对话 => agent_panel::绘制对话(ui, &mut 上下文),
                        Tab::智能体会话 => agent_panel::绘制会话(ui, &mut 上下文),
                        Tab::智能体审查 => agent_panel::绘制审查(ui, &mut 上下文),
                        Tab::视口 => {
                            // 上面已按 `是视口` 分支处理，这里不可达。
                            // 保留分支只为让 `match` 穷尽，不画任何东西。
                        }
                    }
                });
        }
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        tab.可关闭()
    }

    fn allowed_in_windows(&self, tab: &mut Self::Tab) -> bool {
        // 视口被拖成独立窗口后会失去"中央区域"的语义，
        // 而且视口绘制依赖所在面板的实际矩形，浮动窗口下表现会不一致。
        // 因此只允许可关闭的辅助面板浮动。
        tab.可关闭()
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> egui_dock::widgets::tab_viewer::OnCloseResponse {
        use egui_dock::widgets::tab_viewer::OnCloseResponse;
        // 关闭不可关闭的面板时保持打开，避免用户误操作把视口关掉。
        if tab.可关闭() {
            OnCloseResponse::Close
        } else {
            OnCloseResponse::Ignore
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 界面状态与默认布局可以构造，且面板数量正确。
    #[test]
    fn 界面状态可构造且默认布局完整() {
        let 界面 = 界面状态::新建();
        for 面板 in Tab::全部() {
            assert!(
                界面.面板可见(*面板),
                "默认布局应当包含面板 `{}`",
                面板.标题(crate::i18n::语言::中文)
            );
        }
    }

    #[test]
    fn 恢复默认布局能修复被改乱的布局() {
        let mut 界面 = 界面状态::新建();
        // 制造一个只有视口的布局
        界面.设定当前布局(egui_dock::DockState::new(vec![Tab::视口]));
        assert!(!界面.面板可见(Tab::控制台));

        界面.恢复默认布局();
        assert!(界面.面板可见(Tab::控制台), "恢复默认布局后应当有控制台");
        assert!(界面.面板可见(Tab::场景树));
    }

    #[test]
    fn 聚焦未打开的面板会把它加进布局() {
        let mut 界面 = 界面状态::新建();
        界面.设定当前布局(egui_dock::DockState::new(vec![Tab::视口]));
        assert!(!界面.面板可见(Tab::性能));

        界面.聚焦面板(Tab::性能);
        assert!(界面.面板可见(Tab::性能), "聚焦后该面板应当被加入布局");
    }

    /// 切到 AI Agent 模式必须**真的**装上智能体专用布局。
    ///
    /// 这是 `BEVY_STUDIO_MODE=agent` 启动后界面仍是通用布局的回归闸门：
    /// 曾经的 `修复()` 硬性要求"布局必须含视口"，把智能体预设当场判成
    /// 损坏并换成 2D 通用布局 —— 模式切了、布局没切。
    #[test]
    fn 切到智能体模式会装上智能体布局() {
        let mut 界面 = 界面状态::新建();
        assert_eq!(界面.模式, crate::开发模式::二维);

        界面.切换模式(crate::开发模式::智能体).unwrap();
        assert_eq!(界面.模式, crate::开发模式::智能体);
        for 面板 in [Tab::智能体对话, Tab::智能体会话, Tab::智能体审查] {
            assert!(
                界面.面板可见(面板),
                "AI Agent 模式必须显示专用面板 `{}`",
                面板.标题(crate::i18n::语言::中文)
            );
        }
        assert!(
            !界面.面板可见(Tab::视口),
            "智能体预设本来就不该带场景视口（它不是被误删，而是设计如此）"
        );
    }

    /// 模式往返切换后，各模式的布局必须各自恢复，互不污染。
    #[test]
    fn 模式往返切换各自保留布局() {
        let mut 界面 = 界面状态::新建();
        // 2D 起步：有视口、没有智能体面板。
        assert!(界面.面板可见(Tab::视口));
        assert!(!界面.面板可见(Tab::智能体对话));

        界面.切换模式(crate::开发模式::智能体).unwrap();
        assert!(界面.面板可见(Tab::智能体对话));
        assert!(!界面.面板可见(Tab::视口));

        界面.切换模式(crate::开发模式::三维).unwrap();
        assert!(界面.面板可见(Tab::视口), "3D 模式必须有视口");
        assert!(
            !界面.面板可见(Tab::智能体对话),
            "普通模式不得混入 AI 专用面板"
        );

        界面.切换模式(crate::开发模式::脚本).unwrap();
        assert!(界面.面板可见(Tab::脚本));

        // 回到智能体：原来的智能体布局应当被恢复（而不是重新造一份通用布局）。
        界面.切换模式(crate::开发模式::智能体).unwrap();
        assert!(界面.面板可见(Tab::智能体对话));
        assert!(界面.面板可见(Tab::智能体会话));
        assert!(界面.面板可见(Tab::智能体审查));

        // 再回 2D：布局仍然是 2D 那份。
        界面.切换模式(crate::开发模式::二维).unwrap();
        assert!(界面.面板可见(Tab::视口));
        assert!(!界面.面板可见(Tab::智能体对话));
    }

    /// 「恢复默认布局」只能恢复**当前模式**的基线，不能塞进无关的 AI 面板。
    #[test]
    fn 在普通模式恢复默认布局不会带入智能体面板() {
        let mut 界面 = 界面状态::新建();
        界面.设定当前布局(egui_dock::DockState::new(vec![Tab::视口]));
        界面.恢复默认布局();
        assert!(界面.面板可见(Tab::场景树));
        for 面板 in [Tab::智能体对话, Tab::智能体会话, Tab::智能体审查] {
            assert!(
                !界面.面板可见(面板),
                "2D 模式的默认布局不该含 AI 专用面板 `{}`",
                面板.标题(crate::i18n::语言::中文)
            );
        }
    }

    #[test]
    fn 动作队列按顺序取出并清空() {
        let mut 队列 = 动作队列::新建();
        assert!(队列.为空());
        队列.推(UiAction::撤销);
        队列.推(UiAction::重做);
        assert_eq!(队列.数量(), 2);

        let 取出 = 队列.取出();
        assert_eq!(取出, vec![UiAction::撤销, UiAction::重做]);
        assert!(队列.为空(), "取出后队列必须被清空");
    }
}
