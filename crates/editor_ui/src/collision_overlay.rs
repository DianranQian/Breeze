//! 碰撞体轮廓叠加：在 2D 视口里画出实体的碰撞范围。
//!
//! # 为什么在没有物理引擎时也值得做
//!
//! "碰撞体到底在哪、有多大"是物理相关 bug 的主要来源。即使编辑器不跑模拟，
//! 把碰撞范围画出来就已经能让"卡墙里""打不中""触发器不触发"这类问题一眼看出。
//! Godot 的 `--debug-collisions` 做的是同一件事。
//!
//! # 分层
//!
//! 解析在 `editor_core::collision`（无 Bevy、可单测），绘制在 `editor_ui`。
//! 之所以不把解析放在 `editor_app`：UI 层不能依赖应用层（依赖方向是单向的
//! `editor_app → editor_ui → editor_core`）。UI 只从**文档数据**里读组件属性，
//! 与它读其它组件的方式一致。

use egui::{Color32, Painter, Stroke};
use scene_doc::document::场景文档;

use crate::state::视口相机;

/// 碰撞轮廓（世界坐标）。
#[derive(Debug, Clone, PartialEq)]
pub enum 碰撞轮廓 {
    /// 以实体位置为中心的矩形（世界单位）。
    矩形 {
        /// 中心。
        中心: (f32, f32),
        /// 宽。
        宽: f32,
        /// 高。
        高: f32,
    },
    /// 以实体位置为中心的圆（世界单位）。
    圆形 {
        /// 圆心。
        中心: (f32, f32),
        /// 半径。
        半径: f32,
    },
}

/// 从实体里读出碰撞轮廓。
///
/// 返回 `None` 表示没有碰撞体组件（或组件数据不可用）。
pub fn 实体碰撞轮廓(实体: &scene_doc::document::场景实体) -> Option<碰撞轮廓> {
    let 组件 = 实体.取组件(editor_core::collision::碰撞体组件名)?;
    // 解析交给 `editor_core::collision`：缺字段的默认值与非法值夹取都在那里，
    // 运行时将来复用同一份语义，不会出现"编辑器画的和游戏用的不一样"。
    let 碰撞 = editor_core::collision::碰撞体::从属性(&组件.属性);
    let 变换 = 实体.变换.unwrap_or_default();
    // 位置取平移的 x/y；缩放**不**参与：碰撞体尺寸是绝对尺寸，
    // 与精灵贴图缩放语义不同（Godot 也是如此：形状尺寸独立于节点缩放）。
    let 中心 = (变换.平移.0, 变换.平移.1);
    Some(match 碰撞.形状 {
        editor_core::collision::碰撞形状::圆形 => 碰撞轮廓::圆形 {
            中心,
            半径: 碰撞.半径,
        },
        editor_core::collision::碰撞形状::矩形 => 碰撞轮廓::矩形 {
            中心,
            宽: 碰撞.宽度,
            高: 碰撞.高度,
        },
    })
}
/// 文档里全部碰撞轮廓（按文档顺序，保证绘制稳定）。
pub fn 全部碰撞轮廓(
    文档: &场景文档,
) -> Vec<(scene_doc::document::实体标识, 碰撞轮廓)> {
    let mut 结果 = Vec::new();
    for (标识, _) in 文档.深度优先遍历() {
        if let Some(实体) = 文档.取(标识)
            && let Some(轮廓) = 实体碰撞轮廓(实体)
        {
            结果.push((标识, 轮廓));
        }
    }
    结果
}

/// 绘制全部碰撞轮廓。
///
/// `画面原点` 是视口内容矩形的左上角（`Painter` 用屏幕绝对坐标，
/// 而相机换算是相对视口中心的，因此需要这个偏移）。
pub fn 绘制碰撞轮廓(
    画布: &Painter,
    画面原点: egui::Pos2,
    视口尺寸: egui::Vec2,
    相机: &视口相机,
    文档: &场景文档,
    主选中: Option<scene_doc::document::实体标识>,
    配色: &crate::theme::编辑器配色,
) {
    for (标识, 轮廓) in 全部碰撞轮廓(文档) {
        // 选中实体的碰撞体用强调色：多条轮廓重叠时，用户需要一眼找到
        // "我正在编辑的那个"。
        let 颜色 = if 主选中 == Some(标识) {
            配色.强调
        } else {
            // 半透明青色：与实体包围框（灰）区分开，又不抢选中高亮。
            Color32::from_rgba_unmultiplied(80, 220, 200, 160)
        };
        let 线 = Stroke::new(1.5, 颜色);
        match 轮廓 {
            碰撞轮廓::矩形 { 中心, 宽, 高 } => {
                let 左上 = 相机.世界转屏幕((中心.0 - 宽 * 0.5, 中心.1 + 高 * 0.5), 视口尺寸);
                let 右下 = 相机.世界转屏幕((中心.0 + 宽 * 0.5, 中心.1 - 高 * 0.5), 视口尺寸);
                let 矩形 = egui::Rect::from_min_max(画面原点 + 左上, 画面原点 + 右下);
                画布.rect_stroke(矩形, 0.0, 线, egui::StrokeKind::Middle);
            }
            碰撞轮廓::圆形 { 中心, 半径 } => {
                let 屏幕中心 = 相机.世界转屏幕(中心, 视口尺寸);
                let 边缘 = 相机.世界转屏幕((中心.0 + 半径, 中心.1), 视口尺寸);
                let 半径屏幕 = (边缘.x - 屏幕中心.x).abs();
                画布.circle_stroke(画面原点 + 屏幕中心, 半径屏幕, 线);
            }
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::{场景文档, 组件数据};
    use scene_doc::property::{属性值, 枚举值};

    fn 造实体(属性: &[(&str, 属性值)]) -> scene_doc::document::场景实体 {
        let mut 实体 = scene_doc::document::场景实体::新建("碰撞体");
        let mut 表 = std::collections::BTreeMap::new();
        for (键, 值) in 属性 {
            表.insert((*键).to_string(), 值.clone());
        }
        实体.加组件(组件数据::新建(
            editor_core::collision::碰撞体组件名,
            表,
        ));
        实体
    }

    #[test]
    fn 矩形轮廓取宽度与高度() {
        let 实体 = 造实体(&[
            ("形状", 属性值::枚举(枚举值::新建("Rectangle", 0))),
            ("宽度", 属性值::浮点(2.0)),
            ("高度", 属性值::浮点(4.0)),
        ]);
        match 实体碰撞轮廓(&实体).expect("应当有轮廓") {
            碰撞轮廓::矩形 { 宽, 高, .. } => {
                assert_eq!(宽, 2.0);
                assert_eq!(高, 4.0);
            }
            _ => panic!("应当是矩形"),
        }
    }

    #[test]
    fn 圆形轮廓取半径() {
        let 实体 = 造实体(&[
            ("形状", 属性值::枚举(枚举值::新建("Circle", 1))),
            ("半径", 属性值::浮点(0.75)),
        ]);
        match 实体碰撞轮廓(&实体).expect("应当有轮廓") {
            碰撞轮廓::圆形 { 半径, .. } => assert_eq!(半径, 0.75),
            _ => panic!("应当是圆形"),
        }
    }

    #[test]
    fn 没有碰撞体组件时返回_none() {
        let 实体 = scene_doc::document::场景实体::新建("箱子");
        assert!(实体碰撞轮廓(&实体).is_none());
    }

    #[test]
    fn 非法尺寸被夹成正数() {
        let 实体 = 造实体(&[
            ("形状", 属性值::枚举(枚举值::新建("Rectangle", 0))),
            ("宽度", 属性值::浮点(0.0)),
            ("高度", 属性值::浮点(-3.0)),
        ]);
        match 实体碰撞轮廓(&实体).unwrap() {
            碰撞轮廓::矩形 { 宽, 高, .. } => {
                assert!(宽 >= 0.001 && 高 >= 0.001, "尺寸必须为正：{宽}x{高}");
            }
            _ => panic!("应当是矩形"),
        }
    }

    #[test]
    fn 文档收集只包含有碰撞体的实体() {
        let mut 文档 = 场景文档::新建("关卡");
        let 甲 = 文档.生成实体("甲", None);
        let 乙 = 文档.生成实体("乙", None);
        if let Some(实体) = 文档.取可变(乙) {
            实体.加组件(组件数据::新建(
                editor_core::collision::碰撞体组件名,
                [
                    ("形状".to_string(), 属性值::枚举(枚举值::新建("Circle", 1))),
                    ("半径".to_string(), 属性值::浮点(1.0)),
                ]
                .into_iter()
                .collect(),
            ));
        }
        let 轮廓 = 全部碰撞轮廓(&文档);
        assert_eq!(轮廓.len(), 1);
        assert_eq!(轮廓[0].0, 乙);
        assert!(轮廓.iter().all(|(标识, _)| *标识 != 甲));
    }
}
