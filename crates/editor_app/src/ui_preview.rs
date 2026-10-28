//! 把场景里的界面组件投影成 Bevy UI 节点，画进预览视口。
//!
//! # 为什么单独一个模块
//!
//! 界面元素和 2D/3D 物体是两条渲染管线：精灵/网格走世界相机，
//! UI 走 Bevy 的 UI 通道（`Node` 树 + 目标相机）。因此这里独立做：
//!
//! 1. 从文档里取出界面实体（`UiNode` / `UiText` / `UiButton` / `UiImage`）；
//! 2. 生成 `Node` 树，并用 `UiTargetCamera(预览相机)` **锁定**到预览相机 ——
//!    不锁定的话 UI 会画到编辑器主界面上去，把整个编辑器盖住；
//! 3. 文字用内置中文字体（Bevy 默认字体没有汉字，会渲染成方框）；
//! 4. 图片走与精灵相同的贴图缓存（项目相对路径 → 解码后的 `Handle<Image>`）。
//!
//! 几何与内容的解释在 [`crate::ui_data`] 里只有一份，运行时投影
//! （[`crate::ui_runtime`]）复用同一份结果，预览与运行不会各算各的。
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
//! 这里只负责**编辑器预览**。点「运行」后，运行时世界里的界面树由
//! `ui_runtime` 按同一份投影重新生成；编辑器不替游戏运行 UI 逻辑
//! （点击、动画），因此本模块不注册任何交互系统。

use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::ui::widget::{ImageNode, NodeImageMode};

use scene_doc::document::场景文档;

use crate::plugin::EditorStateHolder;
use crate::preview::预览相机;
use crate::ui_components::文本对齐;

pub use crate::ui_data::{收集界面项, 界面内容, 界面项, 颜色转_bevy};

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
///
/// # 参数
///
/// * `贴图缓存` / `图片资源`：图片内容的解码结果与精灵共用同一份缓存，
///   同一张图不会因为"界面上又用了一次"而重复解码。
#[allow(clippy::too_many_arguments)]
pub fn 同步界面元素(
    mut 命令: Commands,
    编辑器: Res<EditorStateHolder>,
    mut 状态: ResMut<界面同步状态>,
    mut 字体资源: ResMut<Assets<Font>>,
    mut 贴图缓存: ResMut<crate::preview::贴图缓存>,
    mut 图片资源: ResMut<Assets<Image>>,
    已有: Query<Entity, With<界面预览实体>>,
    相机: Query<(Entity, &Camera), With<预览相机>>,
) {
    let 修订 = 编辑器.状态.文档修订();
    if 状态.已同步修订 == Some(修订) {
        return;
    }
    // 没有预览相机就没有可锁定的目标：把旧实体清掉后什么都不画，
    // 而不是让 UI 落到编辑器主界面上。
    let Ok((相机实体, 相机数据)) = 相机.single() else {
        for 实体 in &已有 {
            命令.entity(实体).despawn();
        }
        状态.上次数量 = 0;
        状态.已同步修订 = Some(修订);
        return;
    };
    // 锚点换算需要视口尺寸。取相机的逻辑视口；拿不到时退回 1280×720
    // （宁可位置差一点，也不要整棵树不出现）。
    let 视口 = 相机数据
        .logical_viewport_size()
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let 视口 = (视口.x, 视口.y);

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

    // 布局空间 = 项目的**设计分辨率**，而不是渲染目标尺寸。
    //
    // 设计分辨率回答"界面按多大的画布排版"（Godot 的
    // `display/window/size/viewport_width/height`）；渲染目标只是看它的窗口。
    // 两者不同时按等比缩放 + 居中留边（letterbox），这样 1920×1080 设计稿
    // 在 512×512 预览里仍然完整可见，而不是只剩左上角一块。
    let (设计宽, 设计高) = 编辑器
        .状态
        .项目
        .as_ref()
        .map(|项目| 项目.清单().设计分辨率)
        .unwrap_or(editor_core::project::默认设计分辨率());
    let 适配 = editor_core::ui_layout::适配缩放(设计宽 as f32, 设计高 as f32, 视口.0, 视口.1);
    let 项目根 = 编辑器
        .状态
        .项目
        .as_ref()
        .map(|项目| 项目.根目录().to_path_buf());

    // 首次同步时打一行诊断：布局空间（设计分辨率）与渲染目标尺寸不一定一致，
    // 位置算错时没有这行数据只能靠猜。仅首次输出，不刷屏。
    if 状态.上次数量 == 0 {
        info!(
            "界面预览布局：渲染目标 {}×{}，设计分辨率 {}×{}，适配缩放 {:.3}，共 {} 个界面元素",
            视口.0,
            视口.1,
            设计宽,
            设计高,
            适配.缩放,
            项.len()
        );
    }

    let 字体 = 确保字体(&mut 状态, &mut 字体资源);
    for (序号, 项) in 项.iter().enumerate() {
        // 锚点 + 偏移 + 尺寸 → 像素矩形。换算逻辑在 `editor_core::ui_layout`：
        // 纯函数、有测试，"差一个半宽"这类错误不会靠看截图才发现。
        let 矩形 = crate::ui_data::计算矩形(项, 设计宽 as f32, 设计高 as f32);
        let mut 实体命令 = 命令.spawn((
            界面预览实体,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(矩形.左 * 适配.缩放 + 适配.偏移x),
                top: Val::Px(矩形.上 * 适配.缩放 + 适配.偏移y),
                width: if 矩形.宽 > 0.0 {
                    Val::Px(矩形.宽 * 适配.缩放)
                } else {
                    Val::Auto
                },
                height: if 矩形.高 > 0.0 {
                    Val::Px(矩形.高 * 适配.缩放)
                } else {
                    Val::Auto
                },
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(颜色转_bevy(项.背景色)),
            ZIndex(序号 as i32),
            UiTargetCamera(相机实体),
        ));
        // 边框颜色跟随背景色：Bevy 在没有 `BorderColor` 时会画默认黑边，
        // 让"无边框的面板"看起来像被一圈黑线框住。
        实体命令.insert(BorderColor::all(颜色转_bevy(项.背景色)));
        match 项.内容.as_ref() {
            Some(界面内容::文案(文案)) => {
                实体命令.with_children(|父| {
                    父.spawn((
                        Text(文案.文本.clone()),
                        TextFont {
                            font: bevy::text::FontSource::Handle(字体.clone()),
                            // 字号也要跟着适配缩放：布局缩了而字号没缩，
                            // 文字会撑出盒子（实测就是面板变成一条细带、文字外溢）。
                            font_size: FontSize::Px(文案.字号 * 适配.缩放),
                            ..default()
                        },
                        TextColor(颜色转_bevy(文案.颜色)),
                        // 对齐与换行交给 Bevy 的文本布局：只有节点有明确宽度时
                        // 才看得出对齐效果（与 Godot 的 Label 一致）。
                        bevy::text::TextLayout {
                            justify: match 文案.对齐 {
                                文本对齐::左 => bevy::text::Justify::Left,
                                文本对齐::居中 => bevy::text::Justify::Center,
                                文本对齐::右 => bevy::text::Justify::Right,
                            },
                            linebreak: bevy::text::LineBreak::WordBoundary,
                        },
                        Node {
                            margin: UiRect::all(Val::Px(8.0 * 适配.缩放)),
                            ..default()
                        },
                    ));
                });
            }
            Some(界面内容::图片(图片)) => {
                let 句柄 = crate::preview::解析贴图句柄(
                    图片.贴图路径.as_ref().map(|路径| 路径.文本()),
                    项目根.as_deref(),
                    &mut 贴图缓存,
                    图片资源.as_mut(),
                );
                // 空句柄 = 没贴图/没打开项目：退回纯色块，让用户看到"这里有东西"。
                let 图片节点 = if 句柄 == Handle::<Image>::default() {
                    ImageNode::solid_color(颜色转_bevy(图片.着色))
                } else {
                    ImageNode::new(句柄).with_color(颜色转_bevy(图片.着色))
                };
                实体命令.insert(图片节点.with_mode(NodeImageMode::Stretch));
            }
            None => {}
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
    use scene_doc::document::{场景文档, 组件数据};
    use scene_doc::property::属性值;

    use crate::ui_components::{
        界面图片组件名, 界面按钮组件名, 界面文本组件名, 界面面板组件名
    };

    fn 挂组件(
        文档: &mut 场景文档, 名称: &str, 组件名: &str, 字段: &[(&str, 属性值)]
    ) {
        let 标识 = 文档.生成实体(名称, None);
        let 属性: std::collections::BTreeMap<String, 属性值> = 字段
            .iter()
            .map(|(键, 值)| ((*键).to_string(), 值.clone()))
            .collect();
        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(组件数据::新建(组件名, 属性));
        }
    }

    fn 建世界(文档: 场景文档) -> World {
        let mut 状态 = editor_core::state::编辑器状态::新建();
        状态.会话.文档 = 文档;

        let mut 世界 = World::new();
        世界.insert_resource(EditorStateHolder { 状态 });
        世界.init_resource::<界面同步状态>();
        世界.insert_resource(Assets::<Font>::default());
        世界.insert_resource(Assets::<Image>::default());
        世界.init_resource::<crate::preview::贴图缓存>();
        世界
    }

    #[test]
    fn 收集面板与按钮并读取字段() {
        let mut 文档 = 场景文档::新建("界面");
        挂组件(
            &mut 文档,
            "面板",
            界面面板组件名,
            &[
                ("宽度", 属性值::浮点(800.0)),
                ("高度", 属性值::浮点(600.0)),
                ("偏移X", 属性值::浮点(10.0)),
                ("偏移Y", 属性值::浮点(20.0)),
            ],
        );
        挂组件(
            &mut 文档,
            "开始",
            界面按钮组件名,
            &[
                ("文本", 属性值::文本("开始游戏".into())),
                ("字号", 属性值::浮点(30.0)),
            ],
        );

        let 项 = 收集界面项(&文档);
        assert_eq!(项.len(), 2);
        assert_eq!(项[0].尺寸, (800.0, 600.0));
        assert_eq!(项[0].偏移, (10.0, 20.0));
        assert!(项[0].内容.is_none());
        // 只有按钮没有面板：给默认尺寸，且带文案。
        assert_eq!(项[1].尺寸, (240.0, 64.0));
        let Some(界面内容::文案(文案)) = 项[1].内容.as_ref() else {
            panic!("按钮必须带文案");
        };
        assert_eq!(文案.文本, "开始游戏");
        assert_eq!(文案.字号, 30.0);
    }

    #[test]
    fn 文本对齐会传给文本布局() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::text::TextLayout;

        let mut 文档 = 场景文档::新建("界面");
        挂组件(
            &mut 文档,
            "说明",
            界面文本组件名,
            &[
                ("文本", 属性值::文本("居中的说明".into())),
                (
                    "对齐",
                    属性值::枚举(scene_doc::property::枚举值::新建("Center", 1)),
                ),
            ],
        );
        let mut 世界 = 建世界(文档);
        世界.spawn((Camera2d, 预览相机));

        世界
            .run_system_once(同步界面元素)
            .expect("同步界面元素应当能运行");
        世界.flush();

        let 子节点 = 世界
            .query_filtered::<Entity, (With<Text>, With<TextLayout>)>()
            .iter(&世界)
            .next()
            .expect("文本节点应当带 TextLayout");
        assert_eq!(
            世界.get::<TextLayout>(子节点).unwrap().justify,
            bevy::text::Justify::Center,
            "对齐字段必须传到文本布局上"
        );
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
        let Some(界面内容::文案(文案)) = 项[0].内容.as_ref() else {
            panic!("必须带文案");
        };
        assert_eq!(文案.文本, "按钮");
    }

    #[test]
    fn 同步系统生成带目标相机的节点树() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::text::TextFont;

        let mut 文档 = 场景文档::新建("界面");
        挂组件(
            &mut 文档,
            "开始",
            界面按钮组件名,
            &[("文本", 属性值::文本("开始".into()))],
        );
        let mut 世界 = 建世界(文档);
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
        // 无面板按钮必须有可见背景。
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

    /// 图片内容必须落成 `ImageNode`：没打开项目时用纯色块兜底，
    /// 着色仍然生效，节点尺寸来自 `UiImage` 自己的字段。
    #[test]
    fn 图片同步成着色的图片节点() {
        use bevy::ecs::system::RunSystemOnce;

        let mut 文档 = 场景文档::新建("界面");
        挂组件(
            &mut 文档,
            "徽标",
            界面图片组件名,
            &[
                ("宽度", 属性值::浮点(120.0)),
                ("高度", 属性值::浮点(60.0)),
                (
                    "贴图",
                    属性值::资源(scene_doc::property::资源引用值::按路径(
                        scene_doc::path::项目相对路径::解析("assets/ui/logo.png").unwrap(),
                    )),
                ),
                (
                    "着色",
                    属性值::颜色(scene_doc::property::颜色值::srgb(1.0, 0.0, 0.0, 1.0)),
                ),
            ],
        );
        let mut 世界 = 建世界(文档);
        世界.spawn((Camera2d, 预览相机));

        世界
            .run_system_once(同步界面元素)
            .expect("同步界面元素应当能运行");
        世界.flush();

        let 节点 = 世界
            .query_filtered::<Entity, (With<界面预览实体>, With<ImageNode>)>()
            .iter(&世界)
            .next()
            .expect("图片内容应当生成 ImageNode");
        let 图片 = 世界.get::<ImageNode>(节点).unwrap();
        assert_eq!(
            图片.image,
            Handle::<Image>::default(),
            "没打开项目时应当退回占位图"
        );
        assert!(
            (图片.color.to_srgba().red - 1.0).abs() < 1e-6 && 图片.color.to_srgba().green < 1e-6,
            "着色必须写到 ImageNode 上，实际 {:?}",
            图片.color
        );
        let 布局 = 世界.get::<Node>(节点).unwrap();
        let (宽, 高) = match (布局.width, 布局.height) {
            (Val::Px(宽), Val::Px(高)) => (宽, 高),
            other => panic!("图片节点尺寸应当是像素，实际 {other:?}"),
        };
        assert!((宽 / 高 - 2.0).abs() < 1e-3, "120×60 的盒子比例必须保持");
    }
}
