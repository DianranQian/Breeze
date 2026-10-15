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
pub mod hierarchy_panel;
pub mod history_panel;
pub mod inspector_panel;
pub mod menu_bar;
pub mod state;
pub mod theme;
pub mod viewport_panel;

pub use docks::Tab;
pub use state::{
    动作队列, 属性编辑, 文本编辑目标, 界面状态, 绘制上下文, 控制台过滤, 运行时诊断,
    视口相机, 视口拖拽, 资源排序, 选择意图, UiAction,
};
pub use theme::编辑器配色;

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
    上下文.配色.应用(egui上下文);
    let 配色 = 上下文.配色;

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
                &mut 绘制上下文.界面.停靠,
                egui_dock::DockState::new(Vec::new()),
            );

            let mut 查看器 = 面板查看器 { 上下文: 绘制上下文 };

            // 顶部：菜单栏
            egui::Panel::top("bevy_studio_菜单栏")
                .exact_size(24.0)
                .show(界面ui, |ui| {
                    查看器.绘制菜单栏(ui);
                });

            // 顶部：工具栏
            egui::Panel::top("bevy_studio_工具栏")
                .exact_size(28.0)
                .show(界面ui, |ui| {
                    查看器.绘制工具栏(ui);
                });

            // 底部：状态栏
            egui::Panel::bottom("bevy_studio_状态栏")
                .exact_size(22.0)
                .show(界面ui, |ui| {
                    查看器.绘制状态栏(ui);
                });

            // 浮层：两个对话框。放在这里是为了能在同一帧内得到确认结果，
            // 且不会与外层 `绘制` 的借用冲突。
            查看器.绘制浮层(egui上下文);

            // 其余区域：可停靠面板
            egui_dock::DockArea::new(&mut 停靠)
                .style(egui_dock::Style::from_egui(界面ui.style().as_ref()))
                .show_close_buttons(true)
                .draggable_tabs(true)
                .show_add_buttons(false)
                .show_inside(界面ui, &mut 查看器);

            // 放回可停靠布局。
            查看器.上下文.界面.停靠 = 停靠;
        });

    // 处理"新建项目"对话框的确认结果。
    if let Some(绘制上下文) = 待用.as_mut() {
        if 绘制上下文.界面.显示新建项目确认 {
            绘制上下文.界面.显示新建项目确认 = false;
            let 名称 = 绘制上下文.界面.新建项目名.trim().to_string();
            let 名称 = if 名称.is_empty() {
                "我的游戏".to_string()
            } else {
                名称
            };
            绘制上下文.动作.推(UiAction::新建项目 { 名称 });
            绘制上下文.界面.显示新建项目 = false;
        }
    }
}

/// 底部状态栏。
fn 绘制_状态栏(ui: &mut Ui, 上下文: &绘制上下文<'_>) {
    let 编辑器 = 上下文.编辑器;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(if 编辑器.有未保存修改() {
                "● 未保存"
            } else {
                "✓ 已保存"
            })
            .small()
            .color(if 编辑器.有未保存修改() {
                上下文.配色.警告
            } else {
                上下文.配色.次要文字
            }),
        );

        ui.separator();
        ui.label(
            egui::RichText::new(format!("实体 {}", 编辑器.会话.文档.实体数量()))
                .small()
                .color(上下文.配色.次要文字),
        );
        ui.label(
            egui::RichText::new(format!("已选 {}", 编辑器.选择.数量()))
                .small()
                .color(上下文.配色.次要文字),
        );

        ui.separator();
        let 场景路径 = 编辑器
            .会话
            .路径
            .as_ref()
            .map(|路径| 路径.文本().to_string())
            .unwrap_or_else(|| "（尚未保存到文件）".to_string());
        ui.label(
            egui::RichText::new(场景路径)
                .small()
                .color(上下文.配色.次要文字),
        );

        // 右侧：问题计数与运行时帧率
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let 问题数 = 编辑器.问题数量();
            if 问题数 > 0 {
                ui.label(
                    egui::RichText::new(format!("⚠ {问题数} 条问题"))
                        .small()
                        .color(上下文.配色.警告),
                );
            }
            if 上下文.正在运行 {
                ui.label(
                    egui::RichText::new(format!(
                        "{:.0} FPS · {:.1} ms",
                        上下文.运行时信息.帧率, 上下文.运行时信息.帧耗时毫秒
                    ))
                    .small()
                    .color(上下文.配色.次要文字),
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
    egui::Window::new("新建项目")
        .collapsible(false)
        .resizable(false)
        .open(&mut 打开值)
        .show(egui上下文, |ui| {
            ui.label("项目名称");
            ui.add(egui::TextEdit::singleline(&mut 编辑中名称).desired_width(240.0));
            ui.label(
                egui::RichText::new(
                    "项目会创建在接下来选择的目标目录中，且该目录必须是空的。",
                )
                .small()
                .color(配色.次要文字),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("选择目录并创建").clicked() {
                    确认 = true;
                }
                if ui.button("取消").clicked() {
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
) {
    if !界面.显示关于 {
        return;
    }
    let mut 打开 = true;
    egui::Window::new("关于 Bevy Studio")
        .collapsible(false)
        .resizable(false)
        .open(&mut 打开)
        .show(egui上下文, |ui| {
            ui.heading("Bevy Studio");
            ui.label(format!("版本 {}", env!("CARGO_PKG_VERSION")));
            ui.separator();
            ui.label("一个以 Godot Editor 为工作流参考、建立在 Bevy 之上的可视化游戏编辑器。");
            ui.add_space(6.0);
            ui.label(egui::RichText::new("技术栈").strong().color(配色.文字));
            ui.label("Bevy 0.20 / bevy_egui 0.43");
            ui.label("egui 0.36 + egui_dock 0.21（界面与可停靠布局）");
            ui.label("RON（场景与项目清单的序列化格式）");
            ui.separator();
            ui.label(
                egui::RichText::new(format!("当前场景：{场景名}"))
                    .small()
                    .color(配色.次要文字),
            );
            ui.label(
                egui::RichText::new(
                    "本版本已实现：项目建立/打开、场景编辑与保存、场景树、检查器、\
                     2D 视口、资源浏览、撤销重做、控制台。\n\
                     尚未实现的功能在菜单里显示为禁用，不会假装可用。",
                )
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
        绘制上下文 {
            编辑器: self.上下文.编辑器,
            界面: self.上下文.界面,
            动作: self.上下文.动作,
            配色: self.上下文.配色,
            缩略图: self.上下文.缩略图,
            正在运行: self.上下文.正在运行,
            已暂停: self.上下文.已暂停,
            运行时信息: self.上下文.运行时信息,
        }
    }

    fn 绘制菜单栏(&mut self, ui: &mut Ui) {
        menu_bar::绘制(ui, &mut self.视图());
    }

    fn 绘制工具栏(&mut self, ui: &mut Ui) {
        menu_bar::绘制工具栏(ui, &mut self.视图());
    }

    fn 绘制状态栏(&mut self, ui: &mut Ui) {
        绘制_状态栏(ui, &self.视图());
    }

    /// 绘制两个浮层窗口。
    fn 绘制浮层(&mut self, egui上下文: &egui::Context) {
        let 配色 = self.上下文.配色;
        绘制_新建项目对话框(egui上下文, self.上下文.界面, 配色);
        绘制_关于窗口(
            egui上下文,
            self.上下文.界面,
            配色,
            self.上下文.编辑器.场景名(),
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
                    egui::RichText::new(format!("控制台 ({问题数})"))
                        .color(self.上下文.配色.警告),
                );
            }
        }
        WidgetText::from(tab.标题())
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
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

    fn on_close(
        &mut self,
        tab: &mut Self::Tab,
    ) -> egui_dock::widgets::tab_viewer::OnCloseResponse {
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
                面板.标题()
            );
        }
    }

    #[test]
    fn 恢复默认布局能修复被改乱的布局() {
        let mut 界面 = 界面状态::新建();
        // 制造一个只有视口的布局
        界面.停靠 = egui_dock::DockState::new(vec![Tab::视口]);
        assert!(!界面.面板可见(Tab::控制台));

        界面.恢复默认布局();
        assert!(界面.面板可见(Tab::控制台), "恢复默认布局后应当有控制台");
        assert!(界面.面板可见(Tab::场景树));
    }

    #[test]
    fn 聚焦未打开的面板会把它加进布局() {
        let mut 界面 = 界面状态::新建();
        界面.停靠 = egui_dock::DockState::new(vec![Tab::视口]);
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
