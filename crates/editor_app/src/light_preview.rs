//! 把场景里的灯光组件投影成真正的 Bevy 光源。
//!
//! # 与 `preview` 模块的分工
//!
//! `preview` 负责"3D 几何体/精灵/地形"的投影（那是另一条工作线的文件）。
//! 灯光是**照明**而不是几何：它没有网格、没有材质，只是给渲染器的一份参数。
//! 因此单独一个模块，只做三件事：
//!
//! 1. 文档里有 `方向光` / `点光` 组件的实体 → spawn 对应的 Bevy 光源；
//! 2. 场景里出现第一盏灯时，把编辑器的**填充光**压暗（否则用户加的灯
//!    在原本就很亮的默认光下几乎看不出来，会误以为"加灯没生效"）；
//! 3. 删除全部灯后把填充光恢复原值 —— 不留一个"永久变暗"的后遗症。
//!
//! # 为什么按文档修订号重建
//!
//! 灯光数量少（几盏到几十盏），整体重建的成本远低于维护 diff；
//! 与 `preview::同步预览实体` 用同一个修订号，因此"改一个属性 → 画面
//! 立刻变化"的时序完全一致。

use bevy::prelude::*;

use scene_doc::document::{变换数据, 场景文档, 实体标识};
use scene_doc::property::{属性值, 颜色值};

use crate::components3d::{方向光组件名, 点光组件名};
use crate::plugin::EditorStateHolder;

/// 由灯光组件生成的光源实体标记。
#[derive(Component)]
pub struct 灯光预览实体;

/// 灯光同步状态。
#[derive(Resource, Default)]
pub struct 灯光同步状态 {
    /// 上次同步时的文档修订号。
    pub 已同步修订: Option<u64>,
    /// 编辑器填充光（原值）。`None` 表示还没记录。
    pub 填充光: Option<填充光原值>,
    /// 上一次同步时场景是否含有灯光（用于"变暗/恢复"的状态切换）。
    pub 场景有灯: bool,
}

/// 编辑器默认填充光的原始参数。
#[derive(Debug, Clone, Copy)]
pub struct 填充光原值 {
    /// 光源实体。
    pub 实体: Entity,
    /// 原始照度。
    pub 照度: f32,
    /// 原始是否投影。
    pub 投影: bool,
}

/// 场景里有灯时，填充光降到这个照度。
///
/// 取 200 lux（默认 9000 的 2% 左右）：环境不至于纯黑（否则没有灯照到的
/// 角落一片死黑，用户分不清"没灯"与"渲染坏了"），但场景灯的效果清晰可见。
pub const 填充光照度下限: f32 = 200.0;

/// 把颜色值转成 Bevy 颜色。
pub fn 颜色转_bevy(颜色: 颜色值) -> Color {
    Color::srgba(
        颜色.红.clamp(0.0, 1.0),
        颜色.绿.clamp(0.0, 1.0),
        颜色.蓝.clamp(0.0, 1.0),
        颜色.透明度.clamp(0.0, 1.0),
    )
}

/// 一盏灯的投影结果（纯数据，便于单测）。
#[derive(Debug, Clone, PartialEq)]
pub enum 灯光参数 {
    /// 方向光。
    方向 {
        /// 颜色。
        颜色: Color,
        /// 照度（lux）。
        照度: f32,
        /// 是否投影。
        投影: bool,
    },
    /// 点光。
    点 {
        /// 颜色。
        颜色: Color,
        /// 强度（流明）。
        强度: f32,
        /// 半径（米）。
        半径: f32,
    },
}

/// 从实体的组件数据解析灯光参数。
///
/// 返回 `None` 表示该实体不是灯（或组件数据不完整到无法使用）。
/// 解析时对所有数值做一次夹取：场景文件可以被手改，负强度/零半径
/// 不应该让渲染器收到非法参数。
pub fn 解析灯光(实体: &scene_doc::document::场景实体) -> Option<灯光参数> {
    if let Some(组件) = 实体.取组件(方向光组件名) {
        let 颜色 = 组件
            .属性
            .get("颜色")
            .and_then(属性值::取颜色)
            .map(颜色转_bevy)
            .unwrap_or(Color::WHITE);
        let 照度 = 组件
            .属性
            .get("照度")
            .and_then(属性值::取浮点)
            .unwrap_or(10_000.0)
            .clamp(0.0, 200_000.0);
        let 投影 = 组件
            .属性
            .get("投射阴影")
            .and_then(属性值::取布尔)
            .unwrap_or(true);
        return Some(灯光参数::方向 {
            颜色, 照度, 投影
        });
    }
    if let Some(组件) = 实体.取组件(点光组件名) {
        let 颜色 = 组件
            .属性
            .get("颜色")
            .and_then(属性值::取颜色)
            .map(颜色转_bevy)
            .unwrap_or(Color::WHITE);
        let 强度 = 组件
            .属性
            .get("强度")
            .and_then(属性值::取浮点)
            .unwrap_or(800.0)
            .clamp(0.0, 100_000.0);
        let 半径 = 组件
            .属性
            .get("半径")
            .and_then(属性值::取浮点)
            .unwrap_or(20.0)
            .clamp(0.01, 1000.0);
        return Some(灯光参数::点 {
            颜色, 强度, 半径
        });
    }
    None
}

/// 场景里全部灯光的投影清单。
///
/// 返回 `(来源实体, 参数, 变换)`，按文档顺序稳定排列，
/// 这样每次重建得到的结果顺序一致，便于截图比对。
pub fn 投影灯光项(文档: &场景文档) -> Vec<(实体标识, 灯光参数, 变换数据)> {
    let mut 结果 = Vec::new();
    for 标识 in 文档.文档遍历() {
        let Some(实体) = 文档.取(标识) else {
            continue;
        };
        let Some(参数) = 解析灯光(实体) else {
            continue;
        };
        let 变换 = 实体.变换.unwrap_or_default();
        结果.push((标识, 参数, 变换));
    }
    结果
}

/// 文档里灯光的数量（供诊断与测试）。
pub fn 灯光数量(文档: &场景文档) -> usize {
    投影灯光项(文档).len()
}

/// 同步场景灯光到预览世界。
pub fn 同步场景灯光(
    mut 命令: Commands,
    编辑器: Res<EditorStateHolder>,
    mut 状态: ResMut<灯光同步状态>,
    已有: Query<Entity, With<灯光预览实体>>,
    填充光: Query<(Entity, &DirectionalLight), Without<灯光预览实体>>,
) {
    let 修订 = 编辑器.状态.文档修订();
    if 状态.已同步修订 == Some(修订) {
        return;
    }
    let 文档 = &编辑器.状态.会话.文档;

    for 实体 in &已有 {
        命令.entity(实体).despawn();
    }

    let 项 = 投影灯光项(文档);
    for (_, 参数, 变换) in &项 {
        let 实体变换 = crate::preview::变换转预览(*变换);
        match 参数 {
            // 方向光：方向由实体旋转决定。四元数的 -Z 指向，与
            // Bevy 的 `DirectionalLight` 约定一致（灯沿自身 -Z 照射）。
            灯光参数::方向 {
                颜色, 照度, 投影
            } => {
                命令.spawn((
                    灯光预览实体,
                    DirectionalLight {
                        color: *颜色,
                        illuminance: *照度,
                        shadow_maps_enabled: *投影,
                        ..default()
                    },
                    实体变换,
                ));
            }
            灯光参数::点 {
                颜色, 强度, 半径
            } => {
                命令.spawn((
                    灯光预览实体,
                    PointLight {
                        color: *颜色,
                        intensity: *强度,
                        range: *半径,
                        ..default()
                    },
                    实体变换,
                ));
            }
        }
    }

    // 填充光：有场景灯就压暗，没有就恢复。
    if let Some((实体, 光)) = 填充光.iter().next() {
        let 原值 = *状态.填充光.get_or_insert(填充光原值 {
            实体,
            照度: 光.illuminance,
            投影: 光.shadow_maps_enabled,
        });
        let 有灯 = !项.is_empty();
        if 有灯 != 状态.场景有灯 || 状态.填充光.is_none() {
            命令.entity(原值.实体).insert(DirectionalLight {
                illuminance: if 有灯 {
                    填充光照度下限
                } else {
                    原值.照度
                },
                shadow_maps_enabled: !有灯 && 原值.投影,
                ..default()
            });
            状态.场景有灯 = 有灯;
        }
    }

    状态.已同步修订 = Some(修订);
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::组件数据;

    fn 造灯实体(
        类型名: &str, 属性: &[(&str, 属性值)]
    ) -> scene_doc::document::场景实体 {
        let mut 实体 = scene_doc::document::场景实体::新建("灯");
        let mut 表 = std::collections::BTreeMap::new();
        for (键, 值) in 属性 {
            表.insert((*键).to_string(), 值.clone());
        }
        实体.加组件(组件数据::新建(类型名, 表));
        实体
    }

    #[test]
    fn 解析方向光() {
        let 实体 = 造灯实体(
            方向光组件名,
            &[
                ("照度", 属性值::浮点(3000.0)),
                ("投射阴影", 属性值::布尔(false)),
                ("颜色", 属性值::颜色(颜色值::srgb(1.0, 0.5, 0.25, 1.0))),
            ],
        );
        match 解析灯光(&实体).expect("应当解析出方向光") {
            灯光参数::方向 {
                照度, 投影, 颜色
            } => {
                assert_eq!(照度, 3000.0);
                assert!(!投影);
                assert_eq!(颜色, Color::srgba(1.0, 0.5, 0.25, 1.0));
            }
            _ => panic!("类型错了"),
        }
    }

    #[test]
    fn 解析点光并对非法值夹取() {
        let 实体 = 造灯实体(
            点光组件名,
            &[("强度", 属性值::浮点(-100.0)), ("半径", 属性值::浮点(0.0))],
        );
        match 解析灯光(&实体).expect("应当解析出点光") {
            灯光参数::点 { 强度, 半径, .. } => {
                assert_eq!(强度, 0.0);
                assert!(半径 >= 0.01);
            }
            _ => panic!("类型错了"),
        }
    }

    #[test]
    fn 非灯光实体解析为_none() {
        let 实体 = scene_doc::document::场景实体::新建("箱子");
        assert!(解析灯光(&实体).is_none());
    }

    #[test]
    fn 投影清单按文档顺序列出灯光() {
        let mut 文档 = scene_doc::document::场景文档::新建("灯光");
        let 甲 = 文档.生成实体("箱体", None);
        let 灯 = 文档.生成实体("主光", None);
        if let Some(实体) = 文档.取可变(灯) {
            实体.加组件(组件数据::新建(
                方向光组件名,
                std::collections::BTreeMap::new(),
            ));
        }
        let 项 = 投影灯光项(&文档);
        assert_eq!(项.len(), 1);
        assert_eq!(项[0].0, 灯);
        // 没有灯光组件的实体不出现在清单里。
        assert!(项.iter().all(|(标识, _, _)| *标识 != 甲));
        assert_eq!(灯光数量(&文档), 1);
        // 缺字段时用默认值，而不是丢弃这盏灯。
        match &项[0].1 {
            灯光参数::方向 { 照度, .. } => assert_eq!(*照度, 10_000.0),
            _ => panic!("应当是方向光"),
        }
    }

    #[test]
    fn 同步系统会生成光源实体并在无灯时恢复填充光() {
        use bevy::ecs::system::RunSystemOnce;

        let mut 文档 = scene_doc::document::场景文档::新建("灯光");
        let 灯 = 文档.生成实体("主光", None);
        if let Some(实体) = 文档.取可变(灯)
            && let Some(变换) = 实体.变换.as_mut()
        {
            变换.平移 = (3.0, 5.0, 2.0);
        }
        if let Some(实体) = 文档.取可变(灯) {
            实体.加组件(组件数据::新建(
                点光组件名,
                [("强度".to_string(), 属性值::浮点(1200.0))]
                    .into_iter()
                    .collect(),
            ));
        }
        let mut 状态 = editor_core::state::编辑器状态::新建();
        状态.会话.文档 = 文档;

        let mut 世界 = World::new();
        世界.insert_resource(EditorStateHolder { 状态 });
        世界.init_resource::<灯光同步状态>();
        // 编辑器填充光。
        let 填充 = 世界
            .spawn(DirectionalLight {
                illuminance: 9000.0,
                shadow_maps_enabled: true,
                ..default()
            })
            .id();

        世界
            .run_system_once(同步场景灯光)
            .expect("同步场景灯光应当能运行");
        // Commands 需要显式 flush 才能看到新实体。
        世界.flush();

        let 灯数量 = 世界
            .query::<(&PointLight, &灯光预览实体)>()
            .iter(&世界)
            .count();
        assert_eq!(灯数量, 1, "场景里的一盏点光应当变成一个光源实体");
        let 强度 = 世界
            .query::<&PointLight>()
            .iter(&世界)
            .next()
            .map(|光| 光.intensity)
            .unwrap();
        assert_eq!(强度, 1200.0);

        // 填充光被压暗。
        let 填充照度 = 世界
            .get::<DirectionalLight>(填充)
            .map(|光| 光.illuminance)
            .unwrap();
        assert!(
            (填充照度 - 填充光照度下限).abs() < 1e-3,
            "有场景灯时填充光应当压暗，实际 {填充照度}"
        );
        assert!(世界.resource::<灯光同步状态>().场景有灯);

        // 删掉灯：填充光恢复。
        {
            let mut 持有 = 世界.resource_mut::<EditorStateHolder>();
            持有.状态.会话.文档 = scene_doc::document::场景文档::新建("空");
            // 修订号必须变化，否则同步会短路。
            持有.状态.新建实体("触发修订", None).unwrap();
        }
        世界
            .run_system_once(同步场景灯光)
            .expect("第二次同步应当能运行");
        世界.flush();
        assert_eq!(
            世界.query::<&PointLight>().iter(&世界).count(),
            0,
            "灯被删掉后预览光源也要消失"
        );
        let 恢复照度 = 世界
            .get::<DirectionalLight>(填充)
            .map(|光| 光.illuminance)
            .unwrap();
        assert!(
            (恢复照度 - 9000.0).abs() < 1e-3,
            "没有场景灯时填充光必须恢复原值，实际 {恢复照度}"
        );
    }
}
