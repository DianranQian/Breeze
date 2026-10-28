//! 运行时界面树：点「运行」后把界面组件投影成**真实**的 Bevy UI 节点。
//!
//! # 为什么不是把 `ui_preview` 的实体复制一份
//!
//! 预览实体活在主世界、锁定预览相机；运行实体活在运行时的裸 `World` 里。
//! 两者共享的解释在 [`crate::ui_data`]，这里只做"界面项 → Bevy 组件"的翻译，
//! 因此预览里看到的位置、尺寸、颜色与运行时的数据逐项一致。
//!
//! # 布局空间
//!
//! 运行时没有 letterbox：设计分辨率就是布局空间，相机视口按设计分辨率设置
//! （由具体游戏决定）。因此这里把 `计算矩形` 的结果直接写成像素，
//! 不做 `适配缩放` —— 那是"把设计稿塞进编辑器的小预览窗"才需要的东西。
//!
//! # 图片的降级
//!
//! 运行时世界的构建入口只拿得到文档与注册表，没有项目根，也就不能读盘解码
//! 贴图。因此图片节点先落成 `ImageNode::solid_color(着色)`：
//! 数据上是"这里有一张着色图片"，渲染宿主接上资产后按路径换成真图即可。
//! 编辑器预览里同一张图是**解码后**的，便于用户当场确认素材。
//!
//! # 字体
//!
//! 裸 `World` 里没有 `AssetPlugin`，但 `Assets<Font>` 本身只是资源表；
//! 这里按需插入它并写入内置中文字体，保证 `TextFont` 的句柄在数据上有效。

use bevy::prelude::*;
use bevy::text::{FontSize, Justify, LineBreak, TextLayout};
use bevy::ui::widget::{ImageNode, NodeImageMode};

use scene_doc::document::{场景文档, 实体标识};

use crate::ui_components::文本对齐;
use crate::ui_data::{收集界面项, 界面内容, 颜色转_bevy};

/// 运行时界面树的根节点标记。
#[derive(Component, Debug, Clone, Copy)]
pub struct 运行界面根;

/// 运行界面容器与场景实体的对应关系。
#[derive(Component, Debug, Clone, Copy)]
pub struct 运行界面来源(pub 实体标识);

/// 把文档里的界面组件建成一棵挂在运行时世界里的 UI 树。
///
/// 返回生成的界面元素数量（不含根节点）。文档里没有界面组件时
/// **不创建根节点**：一个空的全屏节点会挡住将来真实画面的输入。
pub fn 构建界面树(
    世界: &mut World, 文档: &场景文档, 设计宽: u32, 设计高: u32
) -> usize {
    let 项 = 收集界面项(文档);
    if 项.is_empty() {
        return 0;
    }

    let 根 = 世界
        .spawn((
            运行界面根,
            Name::new("界面树"),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ))
        .id();

    let 需要字体 = 项
        .iter()
        .any(|项| matches!(项.内容, Some(界面内容::文案(_))));
    let 字体 = 需要字体.then(|| {
        if !世界.contains_resource::<Assets<Font>>() {
            世界.init_resource::<Assets<Font>>();
        }
        世界
            .resource_mut::<Assets<Font>>()
            .add(Font::from_bytes(editor_ui::内置字体字节().to_vec()))
    });

    for (序号, 项) in 项.iter().enumerate() {
        let 矩形 = crate::ui_data::计算矩形(项, 设计宽 as f32, 设计高 as f32);
        let 容器 = 世界
            .spawn((
                运行界面来源(项.来源),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(矩形.左),
                    top: Val::Px(矩形.上),
                    width: if 矩形.宽 > 0.0 {
                        Val::Px(矩形.宽)
                    } else {
                        Val::Auto
                    },
                    height: if 矩形.高 > 0.0 {
                        Val::Px(矩形.高)
                    } else {
                        Val::Auto
                    },
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(颜色转_bevy(项.背景色)),
                BorderColor::all(颜色转_bevy(项.背景色)),
                ZIndex(序号 as i32),
                ChildOf(根),
            ))
            .id();

        match 项.内容.as_ref() {
            Some(界面内容::文案(文案)) => {
                let Some(字体) = 字体.clone() else {
                    continue;
                };
                世界.spawn((
                    Text(文案.文本.clone()),
                    TextFont {
                        font: bevy::text::FontSource::Handle(字体),
                        font_size: FontSize::Px(文案.字号),
                        ..default()
                    },
                    TextColor(颜色转_bevy(文案.颜色)),
                    TextLayout {
                        justify: match 文案.对齐 {
                            文本对齐::左 => Justify::Left,
                            文本对齐::居中 => Justify::Center,
                            文本对齐::右 => Justify::Right,
                        },
                        linebreak: LineBreak::WordBoundary,
                    },
                    Node {
                        margin: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    ChildOf(容器),
                ));
            }
            Some(界面内容::图片(图片)) => {
                // 运行入口没有项目根：落成着色块，渲染宿主接上资产后按路径换真图。
                世界.entity_mut(容器).insert(
                    ImageNode::solid_color(颜色转_bevy(图片.着色))
                        .with_mode(NodeImageMode::Stretch),
                );
            }
            None => {}
        }
    }

    项.len()
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::组件数据;
    use scene_doc::property::{属性值, 颜色值};

    use crate::scene_bridge::全部内置组件注册表;
    use crate::ui_components::{
        界面图片组件名, 界面文本组件名, 界面面板组件名
    };

    fn 挂组件(
        文档: &mut 场景文档,
        名称: &str,
        组件名: &str,
        字段: &[(&str, 属性值)],
    ) -> 实体标识 {
        let 标识 = 文档.生成实体(名称, None);
        let 属性: std::collections::BTreeMap<String, 属性值> = 字段
            .iter()
            .map(|(键, 值)| ((*键).to_string(), 值.clone()))
            .collect();
        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(组件数据::新建(组件名, 属性));
        }
        标识
    }

    #[test]
    fn 界面组件建成挂在根节点下的节点树() {
        let mut 文档 = 场景文档::新建("界面");
        挂组件(
            &mut 文档,
            "面板",
            界面面板组件名,
            &[
                ("宽度", 属性值::浮点(320.0)),
                ("高度", 属性值::浮点(180.0)),
                ("偏移X", 属性值::浮点(40.0)),
                ("偏移Y", 属性值::浮点(40.0)),
            ],
        );
        挂组件(
            &mut 文档,
            "标题",
            界面文本组件名,
            &[("文本", 属性值::文本("你好".into()))],
        );
        挂组件(
            &mut 文档,
            "徽标",
            界面图片组件名,
            &[("着色", 属性值::颜色(颜色值::srgb(0.0, 1.0, 0.0, 1.0)))],
        );

        let mut 世界 = World::new();
        let 数量 = 构建界面树(&mut 世界, &文档, 1920, 1080);
        assert_eq!(数量, 3);

        let 根 = 世界
            .query_filtered::<Entity, With<运行界面根>>()
            .iter(&世界)
            .next()
            .expect("必须有一个界面根节点");
        let 容器: Vec<Entity> = 世界
            .query_filtered::<Entity, With<运行界面来源>>()
            .iter(&世界)
            .collect();
        assert_eq!(容器.len(), 3);
        for 实体 in &容器 {
            assert_eq!(
                世界.get::<ChildOf>(*实体).map(|父| 父.parent()),
                Some(根),
                "界面容器必须挂在根节点下"
            );
        }

        // 文本落在容器子节点上，字体句柄有效。
        let 文本: Vec<Entity> = 世界
            .query_filtered::<Entity, (With<Text>, With<TextFont>)>()
            .iter(&世界)
            .collect();
        assert_eq!(文本.len(), 1);
        assert_eq!(世界.get::<Text>(文本[0]).unwrap().0, "你好");
        let 字体 = match &世界.get::<TextFont>(文本[0]).unwrap().font {
            bevy::text::FontSource::Handle(句柄) => 句柄.clone(),
            other => panic!("内置字体必须以句柄形式挂上，实际 {other:?}"),
        };
        assert!(
            世界.resource::<Assets<Font>>().contains(&字体),
            "文本字体必须真的写进运行时世界的资源表"
        );

        // 图片容器上落成 ImageNode。
        let 图片 = 世界
            .query_filtered::<Entity, With<ImageNode>>()
            .iter(&世界)
            .next()
            .expect("图片内容必须落成 ImageNode");
        assert!(世界.get::<运行界面来源>(图片).is_some());
    }

    #[test]
    fn 矩形按设计分辨率而不是视口留边() {
        let mut 文档 = 场景文档::新建("界面");
        挂组件(
            &mut 文档,
            "右下角",
            界面面板组件名,
            &[
                ("宽度", 属性值::浮点(100.0)),
                ("高度", 属性值::浮点(40.0)),
                ("偏移X", 属性值::浮点(20.0)),
                ("偏移Y", 属性值::浮点(30.0)),
                (
                    "锚点",
                    属性值::枚举(scene_doc::property::枚举值::新建("BottomRight", 3)),
                ),
            ],
        );
        let mut 世界 = World::new();
        构建界面树(&mut 世界, &文档, 800, 600);
        let 容器 = 世界
            .query_filtered::<Entity, With<运行界面来源>>()
            .iter(&世界)
            .next()
            .unwrap();
        let 布局 = 世界.get::<Node>(容器).unwrap();
        assert_eq!(布局.left, Val::Px(800.0 - 100.0 - 20.0));
        assert_eq!(布局.top, Val::Px(600.0 - 40.0 - 30.0));
    }

    #[test]
    fn 没有界面组件时不留空根节点() {
        let mut 文档 = 场景文档::新建("空界面");
        文档.生成实体("箱子", None);
        let mut 世界 = World::new();
        assert_eq!(构建界面树(&mut 世界, &文档, 1920, 1080), 0);
        assert_eq!(
            世界
                .query_filtered::<Entity, With<运行界面根>>()
                .iter(&世界)
                .count(),
            0
        );
    }

    #[test]
    fn 构建世界把界面树一并接上() {
        let mut 文档 = 场景文档::新建("界面");
        挂组件(&mut 文档, "面板", 界面面板组件名, &[]);
        let 注册表 = 全部内置组件注册表();
        let mut 世界 = World::new();
        let (统计, _) = crate::scene_bridge::构建世界以设计分辨率(
            &mut 世界,
            &文档,
            &注册表,
            (1920, 1080),
        );
        assert_eq!(统计.界面节点数, 1, "统计必须报告界面元素数量");
        assert_eq!(
            世界
                .query_filtered::<Entity, With<运行界面根>>()
                .iter(&世界)
                .count(),
            1
        );
    }
}
