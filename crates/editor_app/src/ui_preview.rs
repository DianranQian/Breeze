//! 把场景里的界面组件投影成 Bevy UI 节点，画进预览视口。
//!
//! # 为什么单独一个模块
//!
//! 界面元素和 2D/3D 物体是两条渲染管线：精灵/网格走世界相机，
//! UI 走 Bevy 的 UI 通道（`Node` 树 + 目标相机）。因此这里独立做：
//!
//! 1. 从文档里取出界面实体（`UiNode` / `UiText` / `UiButton`）；
//! 2. 生成 `Node` 树，并用 `UiTargetCamera(预览相机)` **锁定**到预览相机 ——
//!    不锁定的话 UI 会画到编辑器主界面上去，把整个编辑器盖住；
//! 3. 文字用内置中文字体（Bevy 默认字体没有汉字，会渲染成方框）。
//!
//! # 坐标系
//!
//! 界面坐标是**视口像素**，左上角为原点，与 Godot 的 `Control` 屏幕坐标一致。
//! 预览视口的渲染目标固定 512×512 起（见 `预览渲染目标`），
//! 因此"按钮放在 (40, 40)、宽 320"在视口里就是那个位置，
//! 与窗口尺寸无关 —— 这正是游戏 UI 该有的行为。
//!
//! # 与运行时的关系
//!
//! 这里只负责**编辑器预览**。真正的游戏界面由项目自己的 Bevy 代码渲染，
//! 编辑器不替它运行 UI 逻辑（点击、动画）。因此本模块不注册任何交互系统。

use bevy::prelude::*;
use bevy::text::FontSize;

use scene_doc::document::{场景文档, 实体标识, 组件数据};
use scene_doc::property::{属性值, 颜色值};

use crate::plugin::EditorStateHolder;
use crate::preview::预览相机;
use crate::ui_components::{界面按钮组件名, 界面文本组件名, 界面面板组件名};

/// 界面预览实体标记。
#[derive(Component)]
pub struct 界面预览实体;

/// 界面同步状态。
#[derive(Resource, Default)]
pub struct 界面同步状态 {
    /// 上次同步的文档修订号。
    pub 已同步修订: Option<u64>,
    /// 中文字体句柄。首次同步时从内置字体创建。
    pub 字体: Option<Handle<Font>>,
    /// 上次同步生成的界面实体数（诊断用）。
    pub 上次数量: usize,
}

/// 面板参数（纯数据）。
#[derive(Debug, Clone, PartialEq)]
pub struct 面板项 {
    /// 来源实体。
    pub 来源: 实体标识,
    /// 宽、高（像素）。
    pub 尺寸: (f32, f32),
    /// 左上角偏移（像素）。
    pub 偏移: (f32, f32),
    /// 背景色。
    pub 背景色: 颜色值,
    /// 文案（按钮/文本时存在）。
    pub 文案: Option<文案项>,
}

/// 文案参数。
#[derive(Debug, Clone, PartialEq)]
pub struct 文案项 {
    /// 文本。
    pub 文本: String,
    /// 字号。
    pub 字号: f32,
    /// 颜色。
    pub 颜色: 颜色值,
}

/// 把颜色值转成 Bevy 颜色。
pub fn 颜色转_bevy(颜色: 颜色值) -> Color {
    Color::srgba(
        颜色.红.clamp(0.0, 1.0),
        颜色.绿.clamp(0.0, 1.0),
        颜色.蓝.clamp(0.0, 1.0),
        颜色.透明度.clamp(0.0, 1.0),
    )
}

fn 取浮点(组件: &组件数据, 字段: &str, 默认: f32) -> f32 {
    组件.属性.get(字段).and_then(属性值::取浮点).unwrap_or(默认)
}

fn 取颜色(组件: &组件数据, 字段: &str, 默认: 颜色值) -> 颜色值 {
    组件.属性.get(字段).and_then(属性值::取颜色).unwrap_or(默认)
}

fn 取文本(组件: &组件数据, 字段: &str, 默认: &str) -> String {
    match 组件.属性.get(字段) {
        Some(属性值::文本(文本)) => 文本.clone(),
        _ => 默认.to_string(),
    }
}

/// 从文档里收集界面元素（纯函数，便于单测）。
///
/// 规则：
/// * 带 `UiNode` 的实体成为面板；
/// * 面板上再挂 `UiButton`/`UiText` 时，面板成为容器并带上文案；
/// * 只有 `UiButton`/`UiText` 而没有 `UiNode` 时，给它一个默认尺寸与位置，
///   这样"直接加一个按钮"也能立刻看到东西，而不是"加了但什么也没有"。
pub fn 收集界面项(文档: &场景文档) -> Vec<面板项> {
    let mut 结果 = Vec::new();
    for (标识, _) in 文档.深度优先遍历() {
        let Some(实体) = 文档.取(标识) else {
            continue;
        };
        let 面板 = 实体.取组件(界面面板组件名);
        let 按钮 = 实体.取组件(界面按钮组件名);
        let 文本 = 实体.取组件(界面文本组件名);
        if 面板.is_none() && 按钮.is_none() && 文本.is_none() {
            continue;
        }

        let (尺寸, 偏移, 背景色) = match 面板 {
            Some(组件) => (
                (
                    取浮点(组件, "宽度", 320.0).max(0.0),
                    取浮点(组件, "高度", 180.0).max(0.0),
                ),
                (取浮点(组件, "偏移X", 40.0), 取浮点(组件, "偏移Y", 40.0)),
                取颜色(组件, "背景色", 颜色值::srgb(0.10, 0.12, 0.16, 0.88)),
            ),
            // 没有面板组件：给按钮一个合适的默认尺寸，纯文本则自发尺寸。
            None => (
                if 按钮.is_some() {
                    (240.0, 64.0)
                } else {
                    (0.0, 0.0)
                },
                (40.0, 40.0),
                颜色值::srgb(0.0, 0.0, 0.0, 0.0),
            ),
        };

        // 文案优先级：按钮 > 文本。一个实体同时挂两个时按按钮显示，
        // 因为按钮本身就要文字，再叠一段文本只会重影。
        let 文案 = if let Some(组件) = 按钮 {
            Some(文案项 {
                文本: 取文本(组件, "文本", "按钮"),
                字号: 取浮点(组件, "字号", 22.0).clamp(1.0, 512.0),
                颜色: 取颜色(组件, "文字颜色", 颜色值::白()),
            })
        } else {
            文本.map(|组件| 文案项 {
                文本: 取文本(组件, "文本", "文字"),
                字号: 取浮点(组件, "字号", 24.0).clamp(1.0, 512.0),
                颜色: 取颜色(组件, "颜色", 颜色值::白()),
            })
        };

        结果.push(面板项 {
            来源: 标识,
            尺寸,
            偏移,
            背景色,
            文案,
        });
    }
    结果
}

/// 生成字体句柄（内置中文字体）。
///
/// # 为什么不用 Bevy 默认字体
///
/// Bevy 自带的 `FiraMono` 子集**没有汉字**，中文文案会渲染成一排方框。
/// 编辑器里已经有内置的 Noto Sans SC 子集（egui 用的那份），
/// 直接复用同一份字节，不额外增加二进制体积。
pub fn 确保字体(
    状态: &mut 界面同步状态, 字体资源: &mut Assets<Font>
) -> Handle<Font> {
    if let Some(句柄) = 状态.字体.clone() {
        return 句柄;
    }
    let 句柄 = 字体资源.add(Font::from_bytes(editor_ui::内置字体字节().to_vec()));
    状态.字体 = Some(句柄.clone());
    句柄
}

/// 同步界面元素到预览世界。
pub fn 同步界面元素(
    mut 命令: Commands,
    编辑器: Res<EditorStateHolder>,
    mut 状态: ResMut<界面同步状态>,
    mut 字体资源: ResMut<Assets<Font>>,
    已有: Query<Entity, With<界面预览实体>>,
    相机: Query<Entity, With<预览相机>>,
) {
    let 修订 = 编辑器.状态.文档修订();
    if 状态.已同步修订 == Some(修订) {
        return;
    }
    // 没有预览相机就没有可锁定的目标：把旧实体清掉后什么都不画，
    // 而不是让 UI 落到编辑器主界面上。
    let Ok(相机实体) = 相机.single() else {
        for 实体 in &已有 {
            命令.entity(实体).despawn();
        }
        状态.上次数量 = 0;
        状态.已同步修订 = Some(修订);
        return;
    };

    let 项 = 收集界面项(&编辑器.状态.会话.文档);
    if 项.is_empty() && 状态.已同步修订 == Some(修订) {
        return;
    }
    for 实体 in &已有 {
        命令.entity(实体).despawn();
    }
    if 项.is_empty() {
        状态.上次数量 = 0;
        状态.已同步修订 = Some(修订);
        return;
    }

    let 字体 = 确保字体(&mut 状态, &mut 字体资源);
    for (序号, 面板) in 项.iter().enumerate() {
        let mut 实体命令 = 命令.spawn((
            界面预览实体,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(面板.偏移.0),
                top: Val::Px(面板.偏移.1),
                width: if 面板.尺寸.0 > 0.0 {
                    Val::Px(面板.尺寸.0)
                } else {
                    Val::Auto
                },
                height: if 面板.尺寸.1 > 0.0 {
                    Val::Px(面板.尺寸.1)
                } else {
                    Val::Auto
                },
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(颜色转_bevy(面板.背景色)),
            ZIndex(序号 as i32),
            UiTargetCamera(相机实体),
        ));
        // 边框颜色跟随背景色：Bevy 在没有 `BorderColor` 时会画默认黑边，
        // 让"无边框的面板"看起来像被一圈黑线框住。
        实体命令.insert(BorderColor::all(颜色转_bevy(面板.背景色)));
        if let Some(文案) = 面板.文案.as_ref() {
            实体命令.with_children(|父| {
                父.spawn((
                    Text(文案.文本.clone()),
                    TextFont {
                        font: bevy::text::FontSource::Handle(字体.clone()),
                        font_size: FontSize::Px(文案.字号),
                        ..default()
                    },
                    TextColor(颜色转_bevy(文案.颜色)),
                    Node {
                        margin: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                ));
            });
        }
    }
    状态.上次数量 = 项.len();
    状态.已同步修订 = Some(修订);
}

/// 文档里界面元素的数量（供诊断与测试）。
pub fn 界面元素数量(文档: &场景文档) -> usize {
    收集界面项(文档).len()
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::组件数据;

    #[test]
    fn 收集面板与按钮并读取字段() {
        let mut 文档 = 场景文档::新建("界面");
        let 面板 = 文档.生成实体("面板", None);
        if let Some(实体) = 文档.取可变(面板) {
            实体.加组件(组件数据::新建(
                界面面板组件名,
                [
                    ("宽度".to_string(), 属性值::浮点(800.0)),
                    ("高度".to_string(), 属性值::浮点(600.0)),
                    ("偏移X".to_string(), 属性值::浮点(10.0)),
                    ("偏移Y".to_string(), 属性值::浮点(20.0)),
                ]
                .into_iter()
                .collect(),
            ));
        }
        let 按钮 = 文档.生成实体("开始", None);
        if let Some(实体) = 文档.取可变(按钮) {
            实体.加组件(组件数据::新建(
                界面按钮组件名,
                [
                    ("文本".to_string(), 属性值::文本("开始游戏".into())),
                    ("字号".to_string(), 属性值::浮点(30.0)),
                ]
                .into_iter()
                .collect(),
            ));
        }

        let 项 = 收集界面项(&文档);
        assert_eq!(项.len(), 2);
        assert_eq!(项[0].尺寸, (800.0, 600.0));
        assert_eq!(项[0].偏移, (10.0, 20.0));
        assert!(项[0].文案.is_none());
        // 只有按钮没有面板：给默认尺寸，且带文案。
        assert_eq!(项[1].尺寸, (240.0, 64.0));
        let 文案 = 项[1].文案.as_ref().expect("按钮必须带文案");
        assert_eq!(文案.文本, "开始游戏");
        assert_eq!(文案.字号, 30.0);
    }

    #[test]
    fn 非界面实体不计入() {
        let mut 文档 = 场景文档::新建("界面");
        文档.生成实体("箱子", None);
        assert_eq!(界面元素数量(&文档), 0);
        assert!(收集界面项(&文档).is_empty());
    }

    #[test]
    fn 按钮优先于文本() {
        let mut 文档 = 场景文档::新建("界面");
        let 标识 = 文档.生成实体("混合", None);
        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(组件数据::新建(界面按钮组件名, Default::default()));
            实体.加组件(组件数据::新建(界面文本组件名, Default::default()));
        }
        let 项 = 收集界面项(&文档);
        assert_eq!(项[0].文案.as_ref().unwrap().文本, "按钮");
    }

    #[test]
    fn 同步系统生成带目标相机的节点树() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::text::TextFont;

        let mut 文档 = 场景文档::新建("界面");
        let 按钮 = 文档.生成实体("开始", None);
        if let Some(实体) = 文档.取可变(按钮) {
            实体.加组件(组件数据::新建(
                界面按钮组件名,
                [("文本".to_string(), 属性值::文本("开始".into()))]
                    .into_iter()
                    .collect(),
            ));
        }
        let mut 状态 = editor_core::state::编辑器状态::新建();
        状态.会话.文档 = 文档;

        let mut 世界 = World::new();
        世界.insert_resource(EditorStateHolder { 状态 });
        世界.init_resource::<界面同步状态>();
        世界.insert_resource(Assets::<Font>::default());
        // 预览相机（只要有标记即可）。
        let 相机 = 世界.spawn((Camera2d, 预览相机)).id();

        世界
            .run_system_once(同步界面元素)
            .expect("同步界面元素应当能运行");
        世界.flush();

        let 节点: Vec<_> = 世界
            .query_filtered::<Entity, (With<界面预览实体>, With<Node>)>()
            .iter(&世界)
            .collect();
        assert_eq!(节点.len(), 1, "一个按钮应当生成一个界面节点");
        // 目标相机必须锁定到预览相机，否则 UI 会画到编辑器主界面上。
        let 目标 = 世界
            .get::<UiTargetCamera>(节点[0])
            .expect("界面节点必须有目标相机");
        assert_eq!(目标.0, 相机);
        // 背景色取自按钮默认值（非透明）。
        let 背景 = 世界.get::<BackgroundColor>(节点[0]).unwrap();
        assert_ne!(背景.0, Color::NONE);
        // 子节点上有文字与字体。
        let 子节点: Vec<_> = 世界
            .query_filtered::<Entity, (With<Text>, With<TextFont>)>()
            .iter(&世界)
            .collect();
        assert_eq!(子节点.len(), 1, "按钮应当有一个文本子节点");
        assert_eq!(世界.get::<Text>(子节点[0]).unwrap().0, "开始");
    }
}
