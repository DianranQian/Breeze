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
    if 响应.dragged() {
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
    if 响应.hovered() && 滚轮.abs() > 0.01 {
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

    // ④ 左键点击：选中屏幕上最近的实体（屏幕空间拾取）。
    if 响应.clicked()
        && let Some(指针) = 响应.interact_pointer_pos()
    {
        let 局部 = egui::vec2(指针.x - 内容矩形.min.x, 指针.y - 内容矩形.min.y);
        if let Some(标识) = 最近实体(上下文, &新相机, 局部, 尺寸, 14.0) {
            上下文.动作.推(UiAction::选择(选择意图::单选(标识)));
        }
    }
    上下文.界面.三维相机 = 新相机;

    // ⑤ 叠加层：网格 → 坐标轴 → 实体 → 选中框。
    let 画布 = ui.painter_at(内容矩形);
    let 原点 = 内容矩形.min;
    画网格(&画布, 原点, &新相机, 尺寸, 上下文);
    画坐标轴(&画布, 原点, &新相机, 尺寸, 上下文);
    画实体(&画布, 原点, &新相机, 尺寸, 上下文);

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
    let mut 最佳: Option<(f32, scene_doc::document::实体标识)> = None;
    for 实体 in 上下文.编辑器.会话.文档.实体表.values() {
        let Some(变换) = 实体.变换 else {
            continue;
        };
        let Some(屏幕) = 相机.投影(变换.平移, 尺寸) else {
            continue;
        };
        let 距离 = ((屏幕.x - 局部.x).powi(2) + (屏幕.y - 局部.y).powi(2)).sqrt();
        if 距离 <= 阈值 && 最佳.as_ref().map(|(最, _)| 距离 < *最).unwrap_or(true) {
            最佳 = Some((距离, 实体.标识));
        }
    }
    最佳.map(|(_, 标识)| 标识)
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
