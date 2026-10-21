//! 三维视口：把 Bevy 的透视渲染结果与 egui 叠加层（网格/坐标轴/选中框）对齐。
//!
//! # 与 2D 视口的关系
//!
//! 渲染由 Bevy 完成（`editor_app::preview` 在 3D 模式下换成 [`Camera3d`]），
//! 叠加层由本模块绘制。两者共用**同一个** [`三维相机`] 参数：
//!
//! * Bevy 侧：相机位置由 `位置()` 决定，朝向看向 `目标`；
//! * egui 侧：`投影()` 用同一组参数做标准透视投影。
//!
//! 因此"网格线落在实体脚下"不是靠两边各自小心地对齐，而是按构造一致。
//!
//! # 交互
//!
//! | 操作 | 效果 |
//! | --- | --- |
//! | 右键拖动 / 中键拖动 | 轨道旋转 |
//! | Shift + 拖动 | 平移轨道中心 |
//! | 滚轮 | 拉近/推远 |
//! | 左键点击 | 选中屏幕投影最近的实体 |
//!
//! 左键拾取用**屏幕空间最近点**而不是射线求交：场景里的实体是二维精灵
//! （带 z 偏移），用屏幕距离拾取与用户看到的位置一致，也不会因为
//! 精灵没有厚度而选不中。这是有意的取舍，写在文档里而不是假装是精确射线拾取。

use egui::{Pos2, Rect, RichText, Sense, Stroke, Ui};

use crate::dev_modes::开发模式;
use crate::state::{UiAction, 三维相机, 绘制上下文, 选择意图};

/// 网格半边长（世界单位）。
const 网格半径: i32 = 24;
/// 网格步长。
const 网格步长: i32 = 2;
/// 每几格画一条主线。
const 主网格间隔: i32 = 5;

/// 绘制三维视口。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 内容矩形: Rect) {
    let 尺寸 = 内容矩形.size();
    if 尺寸.x < 2.0 || 尺寸.y < 2.0 {
        return;
    }

    ui.painter()
        .rect_filled(内容矩形, 0.0, 上下文.配色.视口背景);

    // ① 渲染结果（Bevy 的 Camera3d 输出到同一张纹理）。
    if let Some(结果) = 上下文.视口纹理 {
        let 图片 = egui::Image::new(egui::load::SizedTexture::new(
            结果.纹理,
            egui::vec2(结果.像素尺寸.0 as f32, 结果.像素尺寸.1 as f32),
        ));
        let mut 画布 = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(内容矩形)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        画布.add(图片.fit_to_exact_size(尺寸));
    } else {
        ui.painter().text(
            内容矩形.center(),
            egui::Align2::CENTER_CENTER,
            上下文.语言.取("preview_not_ready"),
            egui::FontId::proportional(13.0),
            上下文.配色.警告,
        );
    }

    // ② 交互（先处理输入再画叠加层，保证本帧的相机变化立刻体现在画面上）。
    let 响应 = ui.interact(
        内容矩形,
        ui.id().with("三维视口交互"),
        Sense::click_and_drag(),
    );
    let 相机 = 上下文.界面.三维相机;
    let mut 新相机 = 相机;
    let 修饰键 = ui.input(|输入| 输入.modifiers);
    let 拖拽 = ui.input(|输入| 输入.pointer.delta());

    // ---- 操纵器：先算布局与悬停，再决定相机与拾取是否让路 ----
    //
    // 顺序很重要：如果先让相机吃掉这次左键拖动，再想画操纵器，
    // 用户就会发现"抓住 X 轴却把视角转了"。
    let 主键刚按下 = ui.input(|输入| 输入.pointer.button_pressed(egui::PointerButton::Primary));
    let 主键按下中 = ui.input(|输入| 输入.pointer.button_down(egui::PointerButton::Primary));
    let 取消键 = ui.input(|输入| 输入.key_pressed(egui::Key::Escape));
    let 局部指针 = 响应
        .hover_pos()
        .or_else(|| ui.input(|输入| 输入.pointer.interact_pos()))
        .map(|位置| egui::vec2(位置.x - 内容矩形.min.x, 位置.y - 内容矩形.min.y));

    let 工具 = 上下文.界面.变换工具;
    let 中心世界 = crate::gizmo::操纵器原点(上下文.编辑器).map(|(_, 变换)| 变换.平移);

    // 先按当前相机建立布局（用于悬停判定）。
    let 初始布局 =
        中心世界.and_then(|中心| crate::gizmo3d::布局(工具, &相机, 尺寸, 内容矩形.min, 中心));
    let 初始悬停 = 初始布局
        .as_ref()
        .zip(局部指针)
        .filter(|(_, 局部)| {
            // 与 2D 同一条裁剪规则：绘制被裁在视口内，命中就必须一起裁。
            // 否则视口外（例如检查器上方）的落点也能命中操纵器。
            局部.x >= 0.0 && 局部.y >= 0.0 && 局部.x <= 尺寸.x && 局部.y <= 尺寸.y
        })
        .and_then(|(布, 局部)| {
            布.命中(Pos2::new(内容矩形.min.x + 局部.x, 内容矩形.min.y + 局部.y))
        });
    let 操纵器占用 = 上下文.界面.操纵器拖拽.is_some() || 初始悬停.is_some();

    // 相机交互：操纵器被悬停/拖拽时一律让路。
    if 响应.dragged() && !操纵器占用 {
        let 是平移 = 修饰键.shift;
        let 是轨道 = ui.input(|输入| 输入.pointer.button_down(egui::PointerButton::Secondary))
            || ui.input(|输入| 输入.pointer.button_down(egui::PointerButton::Middle))
            || !是平移;
        if 是平移 {
            新相机.平移(拖拽);
        } else if 是轨道 {
            新相机.旋转(拖拽);
        }
    }
    let 滚轮 = ui.input(|输入| 输入.smooth_scroll_delta.y);
    if 响应.hovered() && 滚轮.abs() > 0.01 && !操纵器占用 {
        新相机.缩放((-滚轮 * 0.0015).exp());
    }

    // ③ 模式工具栏发来的一次性命令。
    let 命令 = 上下文.界面.视口命令.取出();
    if 命令.重置轨道 || 命令.重置视图 {
        新相机.重置();
    }
    if 命令.适应窗口 {
        适应场景(上下文, &mut 新相机, 尺寸);
    }
    if 命令.聚焦选中
        && let Some(标识) = 上下文.编辑器.选择.主选中()
        && let Some(实体) = 上下文.编辑器.会话.文档.取(标识)
        && let Some(变换) = 实体.变换
    {
        新相机.目标 = 变换.平移;
        新相机.距离 = 新相机.距离.clamp(2.0, 40.0);
    }

    // 相机可能已经变了：用**最终**相机重算布局，保证绘制与证据同步。
    let 最终布局 =
        中心世界.and_then(|中心| crate::gizmo3d::布局(工具, &新相机, 尺寸, 内容矩形.min, 中心));
    上下文.界面.操纵器部件几何 = 最终布局
        .as_ref()
        .map(|布| 布.部件.clone())
        .unwrap_or_default();

    // ---- 操纵器交互（生命周期与 2D 完全一致，只有求解器不同）----
    驱动操纵器(
        ui,
        上下文,
        内容矩形,
        尺寸,
        局部指针,
        主键刚按下,
        主键按下中,
        取消键,
        &新相机,
        中心世界,
        初始悬停,
    );

    // ④ 左键点击：选中屏幕上最近的实体（屏幕空间拾取）。
    //
    // 操纵器被悬停/拖拽时不让路的话，"抓住手柄"会顺带把手柄后面的实体选中。
    if 响应.clicked()
        && !操纵器占用
        && 上下文.界面.操纵器拖拽.is_none()
        && let Some(指针) = 响应.interact_pointer_pos()
    {
        let 局部 = egui::vec2(指针.x - 内容矩形.min.x, 指针.y - 内容矩形.min.y);
        if let Some(标识) = 最近实体(上下文, &新相机, 局部, 尺寸, 14.0) {
            // 修饰键与 2D 视口一致：Shift 追加、Ctrl 切换。
            // 3D 此前只能单选，多选只能回到场景树里 Ctrl 点击；
            // 在一个"只能看、不能批量操作"的三维视口里，这是很实际的工作流缺口。
            let 修饰 = ui.input(|输入| 输入.modifiers);
            let 意图 = match crate::picking3d::拾取模式由修饰键(修饰.shift, 修饰.command) {
                crate::picking3d::拾取模式::替换 => 选择意图::单选(标识),
                crate::picking3d::拾取模式::追加 => 选择意图::追加(标识),
                crate::picking3d::拾取模式::切换 => 选择意图::切换(标识),
            };
            上下文.动作.推(UiAction::选择(意图));
        }
    }
    上下文.界面.三维相机 = 新相机;

    // ⑤ 叠加层：网格 → 坐标轴 → 实体 → 操纵器（操纵器在最上面）。
    let 画布 = ui.painter_at(内容矩形);
    let 原点 = 内容矩形.min;
    画网格(&画布, 原点, &新相机, 尺寸, 上下文);
    画坐标轴(&画布, 原点, &新相机, 尺寸, 上下文);
    画实体(&画布, 原点, &新相机, 尺寸, 上下文);
    绘制操纵器(&画布, 上下文);

    // ⑥ 工具快捷键（Q/W/E/R）与 Escape 取消，与 2D 视口同一套规则。
    //
    // 3D 视口此前完全没有键盘处理：切了工具之后只能靠工具栏切回去。
    let _ = crate::gizmo::处理工具快捷键(ui, 上下文);

    // ⑥ 左下角提示（与 2D 视口一致的输入说明）。
    let 提示位置 = Pos2::new(内容矩形.min.x + 8.0, 内容矩形.max.y - 24.0);
    let mut 提示 = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(
                提示位置,
                egui::vec2(内容矩形.width() - 16.0, 18.0),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    egui::Frame::NONE
        .fill(上下文.配色.叠加背景)
        .stroke(Stroke::new(1.0, 上下文.配色.叠加边框))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .corner_radius(3)
        .show(&mut 提示, |ui| {
            ui.label(
                RichText::new(上下文.语言.取("viewport3d_input_hint"))
                    .small()
                    .color(上下文.配色.次要文字),
            );
        });
}

/// 驱动 3D 操纵器：悬停 → 按下 → 拖拽（预览）→ 释放提交 / Escape 取消。
///
/// 生命周期与 2D 完全一致（见 `viewport_panel::驱动操纵器`），
/// 只有求解器换成 [`crate::gizmo3d`]（射线求交）这一处不同。
#[allow(clippy::too_many_arguments)]
fn 驱动操纵器(
    _ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    _内容矩形: Rect,
    尺寸: egui::Vec2,
    局部指针: Option<egui::Vec2>,
    主键刚按下: bool,
    主键按下中: bool,
    取消键: bool,
    相机: &三维相机,
    中心世界: Option<(f32, f32, f32)>,
    初始悬停: Option<crate::gizmo::操纵器部件>,
) {
    use crate::gizmo;

    let 工具 = 上下文.界面.变换工具;
    let mut 悬停 = 初始悬停;

    // ---- 1. 已有会话 ----
    if let Some(mut 会话) = 上下文.界面.操纵器拖拽.take() {
        let 目标有效 = 会话
            .起始
            .iter()
            .all(|(标识, _)| 上下文.编辑器.会话.文档.存在(*标识));
        let 主选中 = gizmo::操纵器原点(上下文.编辑器);
        let 中心 = 主选中.map(|(_, 变换)| 变换.平移);

        if 取消键 || !目标有效 {
            gizmo::收尾(上下文, 会话, false);
        } else if let (Some(局部), Some(中心)) = (局部指针, 中心) {
            if 主键按下中 {
                if let Some(新) = crate::gizmo3d::求解(
                    会话.部件,
                    &mut 会话.基准,
                    中心,
                    局部,
                    相机,
                    尺寸,
                    &会话.起始,
                ) {
                    会话.最新 = 新.clone();
                    gizmo::推送预览(上下文, 新);
                }
                悬停 = Some(会话.部件);
                上下文.界面.操纵器拖拽 = Some(会话);
            } else {
                gizmo::收尾(上下文, 会话, true);
            }
        } else if 主键按下中 {
            悬停 = Some(会话.部件);
            上下文.界面.操纵器拖拽 = Some(会话);
        } else {
            gizmo::收尾(上下文, 会话, true);
        }
    }

    // ---- 2. 按下：命中手柄则建立会话 ----
    if 主键刚按下
        && 上下文.界面.操纵器拖拽.is_none()
        && 局部指针
            .map(|局部| 局部.x >= 0.0 && 局部.y >= 0.0 && 局部.x <= 尺寸.x && 局部.y <= 尺寸.y)
            .unwrap_or(false)
        && let (Some(部件), Some(局部), Some(中心)) = (悬停, 局部指针, 中心世界)
        && let Some(基准) = crate::gizmo3d::建立基准(部件, 中心, 局部, 相机, 尺寸)
    {
        上下文.界面.操纵器拖拽 = Some(gizmo::拖拽会话::新建(
            工具,
            部件,
            gizmo::收集选中变换(上下文.编辑器),
            基准,
        ));
        悬停 = Some(部件);
    }

    上下文.界面.操纵器悬停 = 悬停;
}

/// 绘制 3D 操纵器（图元与命中几何是同一份）。
fn 绘制操纵器(画布: &egui::Painter, 上下文: &绘制上下文<'_>) {
    if 上下文.界面.变换工具 == crate::gizmo::变换工具::选择 {
        return;
    }
    let 拖拽部件 = 上下文.界面.操纵器拖拽.as_ref().map(|会话| 会话.部件);
    for 项 in &上下文.界面.操纵器部件几何 {
        let 色 = crate::gizmo::部件颜色(
            &上下文.配色,
            项.部件.轴(),
            上下文.界面.操纵器悬停,
            拖拽部件,
            项.部件,
        );
        let 线宽 = crate::gizmo::部件强调(上下文.界面.操纵器悬停, 拖拽部件, 项.部件);
        crate::gizmo::绘制部件(画布, 项, 色, 线宽, &上下文.配色);
    }
}

/// 把相机对准整个场景。
fn 适应场景(上下文: &绘制上下文<'_>, 相机: &mut 三维相机, 尺寸: egui::Vec2) {
    let _ = 尺寸;
    let mut 最小 = (f32::MAX, f32::MAX, f32::MAX);
    let mut 最大 = (f32::MIN, f32::MIN, f32::MIN);
    let mut 有内容 = false;
    for 实体 in 上下文.编辑器.会话.文档.实体表.values() {
        if let Some(变换) = 实体.变换 {
            有内容 = true;
            最小.0 = 最小.0.min(变换.平移.0);
            最小.1 = 最小.1.min(变换.平移.1);
            最小.2 = 最小.2.min(变换.平移.2);
            最大.0 = 最大.0.max(变换.平移.0);
            最大.1 = 最大.1.max(变换.平移.1);
            最大.2 = 最大.2.max(变换.平移.2);
        }
    }
    if !有内容 {
        *相机 = 三维相机::default();
        return;
    }
    相机.目标 = (
        (最小.0 + 最大.0) * 0.5,
        (最小.1 + 最大.1) * 0.5,
        (最小.2 + 最大.2) * 0.5,
    );
    let 跨度 = ((最大.0 - 最小.0).powi(2) + (最大.1 - 最小.1).powi(2) + (最大.2 - 最小.2).powi(2))
        .sqrt()
        .max(1.0);
    相机.距离 = (跨度 * 1.6).clamp(三维相机::最小距离, 三维相机::最大距离);
}

/// 屏幕空间最近的实体。
fn 最近实体(
    上下文: &绘制上下文<'_>,
    相机: &三维相机,
    局部: egui::Vec2,
    尺寸: egui::Vec2,
    阈值: f32,
) -> Option<scene_doc::document::实体标识> {
    // 决策规则集中在 `picking3d`：屏幕距离优先，距离接近时用相机深度兜底。
    // 这里只负责把文档投影成候选，不做判断 —— 那样才有测试可写。
    let 相机位置 = 相机.位置();
    let mut 候选 = Vec::new();
    for 实体 in 上下文.编辑器.会话.文档.实体表.values() {
        let Some(变换) = 实体.变换 else {
            continue;
        };
        let Some(屏幕) = 相机.投影(变换.平移, 尺寸) else {
            continue;
        };
        let 屏幕距离 = ((屏幕.x - 局部.x).powi(2) + (屏幕.y - 局部.y).powi(2)).sqrt();
        if 屏幕距离 > 阈值 {
            continue;
        }
        候选.push(crate::picking3d::候选(
            实体.标识,
            屏幕距离,
            相机位置,
            变换.平移,
        ));
    }
    crate::picking3d::最佳拾取(&候选, 阈值)
}

/// 世界点 → 屏幕像素。
fn 投影(
    相机: &三维相机, 原点: Pos2, 世界: (f32, f32, f32), 尺寸: egui::Vec2
) -> Option<Pos2> {
    相机
        .投影(世界, 尺寸)
        .map(|局部| Pos2::new(原点.x + 局部.x, 原点.y + 局部.y))
}

/// 地面网格。
fn 画网格(
    画布: &egui::Painter,
    原点: Pos2,
    相机: &三维相机,
    尺寸: egui::Vec2,
    上下文: &绘制上下文<'_>,
) {
    let 半 = 网格半径;
    let 步 = 网格步长;
    let mut i = -半;
    while i <= 半 {
        let 是主 = i % (步 * 主网格间隔) == 0;
        let 色 = if 是主 {
            上下文.配色.网格主
        } else {
            上下文.配色.网格次
        };
        let 线宽 = if 是主 { 1.0 } else { 0.5 };
        let 描边 = Stroke::new(线宽, 色);
        if let (Some(a), Some(b)) = (
            投影(相机, 原点, (i as f32, 0.0, -半 as f32), 尺寸),
            投影(相机, 原点, (i as f32, 0.0, 半 as f32), 尺寸),
        ) {
            画布.line_segment([a, b], 描边);
        }
        if let (Some(a), Some(b)) = (
            投影(相机, 原点, (-半 as f32, 0.0, i as f32), 尺寸),
            投影(相机, 原点, (半 as f32, 0.0, i as f32), 尺寸),
        ) {
            画布.line_segment([a, b], 描边);
        }
        i += 步;
    }
}

/// 坐标轴：X 红、Y 绿、Z 蓝。
fn 画坐标轴(
    画布: &egui::Painter,
    原点: Pos2,
    相机: &三维相机,
    尺寸: egui::Vec2,
    上下文: &绘制上下文<'_>,
) {
    let 零 = (0.0, 0.0, 0.0);
    let 轴长 = 6.0;
    for (终点, 色) in [
        ((轴长, 0.0, 0.0), 上下文.配色.轴_x),
        ((0.0, 轴长, 0.0), 上下文.配色.轴_y),
        ((0.0, 0.0, 轴长), 上下文.配色.信息),
    ] {
        if let (Some(a), Some(b)) = (投影(相机, 原点, 零, 尺寸), 投影(相机, 原点, 终点, 尺寸))
        {
            画布.line_segment([a, b], Stroke::new(1.5, 色));
        }
    }
    if let Some(原点位置) = 投影(相机, 原点, 零, 尺寸) {
        画布.circle_filled(原点位置, 3.0, 上下文.配色.文字);
    }
}

/// 实体锚点与选中框。
fn 画实体(
    画布: &egui::Painter,
    原点: Pos2,
    相机: &三维相机,
    尺寸: egui::Vec2,
    上下文: &绘制上下文<'_>,
) {
    for 实体 in 上下文.编辑器.会话.文档.实体表.values() {
        let Some(变换) = 实体.变换 else {
            continue;
        };
        let Some(屏幕) = 投影(相机, 原点, 变换.平移, 尺寸) else {
            continue;
        };
        let 选中 = 上下文.编辑器.选择.包含(实体.标识);
        let 色 = if 选中 {
            上下文.配色.选择框
        } else {
            上下文.配色.实体框
        };
        let 半 = if 选中 { 7.0 } else { 4.0 };
        画布.rect_stroke(
            Rect::from_center_size(屏幕, egui::vec2(半 * 2.0, 半 * 2.0)),
            2.0,
            Stroke::new(if 选中 { 1.5 } else { 1.0 }, 色),
            egui::StrokeKind::Middle,
        );
        if 选中 {
            画布.text(
                Pos2::new(屏幕.x + 10.0, 屏幕.y - 10.0),
                egui::Align2::LEFT_BOTTOM,
                实体.名称.clone(),
                egui::FontId::proportional(11.0),
                上下文.配色.文字,
            );
        }
    }
}

/// 当前是否处于三维模式。
pub fn 活动(上下文: &绘制上下文<'_>) -> bool {
    上下文.界面.模式 == 开发模式::三维
}

/// 把三维模式的相机重置（供测试/动作复用）。
pub fn 重置相机(相机: &mut 三维相机) {
    相机.重置();
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 投影把目标点放在视口中心() {
        let 相机 = 三维相机::default();
        let 尺寸 = egui::vec2(800.0, 600.0);
        let 屏幕 = 相机.投影(相机.目标, 尺寸).expect("目标点应当在相机前方");
        assert!((屏幕.x - 400.0).abs() < 0.5, "目标点应在水平中心：{屏幕:?}");
        assert!((屏幕.y - 300.0).abs() < 0.5, "目标点应在垂直中心：{屏幕:?}");
    }

    #[test]
    fn 相机背后的点被剔除() {
        let 相机 = 三维相机::default();
        let 位置 = 相机.位置();
        // 从相机位置继续往外移一个距离，必然在背后。
        let 背后 = (位置.0 * 2.0, 位置.1 * 2.0, 位置.2 * 2.0);
        assert!(相机.投影(背后, egui::vec2(800.0, 600.0)).is_none());
    }

    #[test]
    fn 距离与俯仰都被夹住() {
        let mut 相机 = 三维相机::default();
        相机.缩放(0.0001);
        assert!(相机.距离 >= 三维相机::最小距离);
        for _ in 0..100 {
            相机.旋转(egui::vec2(0.0, 100.0));
        }
        assert!(相机.俯仰 <= 三维相机::最大俯仰 + 1e-5);
        for _ in 0..200 {
            相机.旋转(egui::vec2(0.0, -100.0));
        }
        assert!(相机.俯仰 >= -三维相机::最大俯仰 - 1e-5);
    }

    #[test]
    fn 重置回到默认视角() {
        let mut 相机 = 三维相机::default();
        相机.缩放(4.0);
        相机.旋转(egui::vec2(120.0, 30.0));
        相机.重置();
        assert_eq!(相机, 三维相机::default());
    }

    /// 世界上方向必须投影到屏幕上方。
    ///
    /// 这是修复"叠加层上下反了"的核心断言：用同一套投影算一个比目标高
    /// 一个单位的点，它的屏幕 y 必须**更小**（egui 的 y 轴向下）。
    #[test]
    fn 世界上方向投影到屏幕上方() {
        let 相机 = 三维相机::default();
        let 尺寸 = egui::vec2(800.0, 600.0);
        let 原点 = 相机.投影(相机.目标, 尺寸).unwrap();
        let 上方 = 相机
            .投影((相机.目标.0, 相机.目标.1 + 1.0, 相机.目标.2), 尺寸)
            .expect("目标上方一点应当在视锥内");
        assert!(
            上方.y < 原点.y - 1.0,
            "世界上方向应当投影到屏幕上方：目标 {原点:?}，上方点 {上方:?}"
        );
    }

    /// 默认视角（相机在 +X/+Y/+Z 一侧看向原点）下，世界 +X 必须落在屏幕右侧。
    ///
    /// 这条断言能直接抓住"右向量符号写反"导致的左右镜像。
    #[test]
    fn 世界x轴在默认视角下投影到屏幕右侧() {
        let 相机 = 三维相机::default();
        let 尺寸 = egui::vec2(800.0, 600.0);
        let 原点 = 相机.投影(相机.目标, 尺寸).unwrap();
        let 正x = 相机
            .投影((相机.目标.0 + 4.0, 相机.目标.1, 相机.目标.2), 尺寸)
            .expect("+X 方向应当在视锥内");
        assert!(
            正x.x > 原点.x + 1.0,
            "世界 +X 应当投影到屏幕右侧：原点 {原点:?}，+X {正x:?}"
        );
    }

    /// 默认视角下世界 +Z 必须落在屏幕左侧（相机在 +Z 一侧）。
    ///
    /// 与上一条一起，两条轴的方向就把"左右镜像"钉死了。
    #[test]
    fn 世界z轴在默认视角下投影到屏幕左侧() {
        let 相机 = 三维相机::default();
        let 尺寸 = egui::vec2(800.0, 600.0);
        let 原点 = 相机.投影(相机.目标, 尺寸).unwrap();
        let 正z = 相机
            .投影((相机.目标.0, 相机.目标.1, 相机.目标.2 + 4.0), 尺寸)
            .expect("+Z 方向应当在视锥内");
        assert!(
            正z.x < 原点.x - 1.0,
            "世界 +Z 应当投影到屏幕左侧：原点 {原点:?}，+Z {正z:?}"
        );
    }

    /// 屏幕射线必须穿过它对应的屏幕点：投影 → 反投影 → 再投影应回到原处。
    ///
    /// 非对称点才有判别力：如果某个基向量符号反了，
    /// 原点与中心点仍然对得上，但偏心的点会偏到另一侧。
    #[test]
    fn 屏幕射线与投影在偏心点上也一致() {
        let 相机 = 三维相机::default();
        let 尺寸 = egui::vec2(800.0, 600.0);
        let 目标世界 = (2.0, 1.0, -3.0);
        let 屏幕 = 相机.投影(目标世界, 尺寸).expect("应当在视锥内");
        let (起点, 方向) = 相机.屏幕射线(屏幕, 尺寸);
        // 沿射线走到与目标世界深度相同的位置
        let 相机位置 = 相机.位置();
        let 视深 = ((目标世界.0 - 相机位置.0).powi(2)
            + (目标世界.1 - 相机位置.1).powi(2)
            + (目标世界.2 - 相机位置.2).powi(2))
        .sqrt();
        let 沿途 = (
            起点.0 + 方向.0 * 视深,
            起点.1 + 方向.1 * 视深,
            起点.2 + 方向.2 * 视深,
        );
        let 重量 = 相机.投影(沿途, 尺寸).expect("沿途点应当在前方");
        assert!(
            (重量.x - 屏幕.x).abs() < 2.0 && (重量.y - 屏幕.y).abs() < 2.0,
            "反投影再投影应当回到原点：屏幕 {屏幕:?}，重量 {重量:?}"
        );
    }

    /// 相机永远在目标上方（俯仰为正时），不会钻到地面下。
    #[test]
    fn 默认相机位于目标上方() {
        let 相机 = 三维相机::default();
        let 位置 = 相机.位置();
        assert!(
            位置.1 > 相机.目标.1,
            "默认视角的相机应当在目标上方：位置 {位置:?}"
        );
    }

    #[test]
    fn 屏幕射线与投影互为逆() {
        let 相机 = 三维相机::default();
        let 尺寸 = egui::vec2(800.0, 600.0);
        let (起点, 方向) = 相机.屏幕射线(egui::vec2(400.0, 300.0), 尺寸);
        let 中点 = (
            起点.0 + 方向.0 * 相机.距离,
            起点.1 + 方向.1 * 相机.距离,
            起点.2 + 方向.2 * 相机.距离,
        );
        let 屏幕 = 相机.投影(中点, 尺寸).expect("中点应当在相机前方");
        assert!((屏幕.x - 400.0).abs() < 1.0);
        assert!((屏幕.y - 300.0).abs() < 1.0);
    }
}
