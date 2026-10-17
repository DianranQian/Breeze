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

pub mod assets_panel;
pub mod console_panel;
pub mod docks;
pub mod fonts;
pub mod hierarchy_panel;
pub mod history_panel;
pub mod icons;
pub mod inspector_panel;
pub mod menu_bar;
pub mod state;
pub mod theme;
pub mod theme_editor;
pub mod viewport_panel;
pub mod workspace_bar;
pub mod workspaces;

// 多语言入口定义在 `editor_core`（它需要给命令显示名做本地化），
// 这里把模块整个再导出，于是界面代码仍可写 `crate::i18n::语言`。
pub use editor_core::i18n;
// 常用类型的便捷再导出，避免调用方到处写 `i18n::`。
pub use docks::Tab;
pub use editor_core::i18n::语言;
pub use fonts::{
    内置字体, 内置字体大小, 后备字体环境变量, 字体安装结果, 安装中文字体, 界面代表性中文,
};
pub use icons::{图标, 图标库};
pub use state::{
    UiAction, 动作队列, 区域快照, 属性编辑, 控制台过滤, 文本编辑目标, 界面状态, 绘制上下文,
    视口拖拽, 视口渲染结果, 视口相机, 资源排序, 运行时诊断, 选择意图,
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

    // 每帧重新测量。清空而不是复用：区域数量会随面板开关变化，
    // 复用会让已经关闭的面板以旧矩形残留在证据文件里。
    上下文.界面.布局快照.clear();

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

            // 顶部：菜单栏
            egui::Panel::top("bevy_studio_菜单栏")
                .exact_size(24.0)
                .show(界面ui, |ui| {
                    查看器.记录区域("menubar", ui.max_rect());
                    查看器.绘制菜单栏(ui);
                });

            // 顶部：工作区栏（切换 / 新建 / 复制 / 重命名 / 删除 / 重置）
            egui::Panel::top("bevy_studio_workspace_bar")
                .exact_size(workspace_bar::工作区栏高度)
                .show(界面ui, |ui| {
                    查看器.记录区域("workspacebar", ui.max_rect());
                    查看器.绘制工作区栏(ui);
                });

            // 顶部：工具栏
            egui::Panel::top("bevy_studio_工具栏")
                .exact_size(28.0)
                .show(界面ui, |ui| {
                    查看器.记录区域("toolbar", ui.max_rect());
                    查看器.绘制工具栏(ui);
                });

            // 底部：状态栏
            egui::Panel::bottom("bevy_studio_状态栏")
                .exact_size(22.0)
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
                // 关闭按钮：让用户能把面板收起来，再从「窗口」菜单打开。
                .show_close_buttons(true)
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
    egui::Window::new(语言.取("new_project"))
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

    if 确认 {
        界面.显示新建项目确认 = true;
        return;
    }
    if !仍然打开 {
        界面.显示新建项目 = false;
    }
}

/// "关于"窗口。
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
    egui::Window::new(语言.取("about_bevy_studio"))
        .collapsible(false)
        .resizable(false)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            ui.heading("Bevy Studio");
            ui.label(语言.填充("version", &[&env!("CARGO_PKG_VERSION")]));
            ui.separator();
            ui.label(语言.取("about_tagline"));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(语言.取("tech_stack"))
                    .strong()
                    .color(配色.文字),
            );
            ui.label("Bevy 0.20 / bevy_egui 0.43");
            ui.label(语言.取("about_ui_stack"));
            ui.label(语言.取("about_scene_format"));
            ui.separator();
            ui.label(
                egui::RichText::new(语言.填充("current_scene", &[&场景名]))
                    .small()
                    .color(配色.次要文字),
            );
            ui.label(
                egui::RichText::new(语言.取("about_implemented_features"))
                    .small()
                    .color(配色.次要文字),
            );
        });
    if !打开 {
        界面.显示关于 = false;
    }
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
            视口纹理: self.上下文.视口纹理,
            正在运行: self.上下文.正在运行,
            已暂停: self.上下文.已暂停,
            运行时信息: self.上下文.运行时信息,
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

    fn 绘制工作区栏(&mut self, ui: &mut Ui) {
        workspace_bar::绘制(ui, &mut self.视图());
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
        match tab {
            Tab::场景树 => hierarchy_panel::绘制(ui, &mut 上下文),
            Tab::检查器 => inspector_panel::绘制(ui, &mut 上下文),
            Tab::视口 => {
                viewport_panel::绘制提示(ui, &上下文);
                // 视口需要知道自己能画多大的区域。
                let 可用 = ui.available_rect_before_wrap();
                viewport_panel::绘制内容(ui, &mut 上下文, 可用);
            }
            Tab::资源浏览器 => assets_panel::绘制(ui, &mut 上下文),
            Tab::控制台 => console_panel::绘制(ui, &mut 上下文),
            Tab::历史 => history_panel::绘制历史(ui, &mut 上下文),
            Tab::运行时ecs => history_panel::绘制运行时(ui, &mut 上下文),
            Tab::性能 => history_panel::绘制性能(ui, &mut 上下文),
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
