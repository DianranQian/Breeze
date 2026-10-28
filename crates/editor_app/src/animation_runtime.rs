//! 文档动画组件落到 Bevy 播放器的桥接层。
//!
//! # 两个宿主，两条路径
//!
//! | 宿主 | 可用资源 | 做法 |
//! | --- | --- | --- |
//! | 运行时世界（点「运行」后的裸 `World`） | 没有 `AssetServer`、不渲染 | 建立真实的 `AnimationGraph` + `AnimationPlayer`，片段是空占位 |
//! | 编辑器预览宿主（主 `App`） | `AssetServer` + `Assets<Gltf>` | 按名字取真实 `AnimationClip`，在模型自带的播放器上播放 |
//!
//! 运行时世界是刻意隔离出来的、**没有渲染与资源加载**的世界
//! （见 `scene_bridge` 的模块文档），因此那边无法读取 glTF 里的曲线数据。
//! 这里仍然建立真实的动画图与播放器，而不是只塞一个属性：调试器能看到
//! 动画节点已经就绪，将来运行世界接上渲染宿主时只需替换片段句柄。
//!
//! 用户实际**看得见**动画的是编辑器预览：模型通过 `WorldAssetRoot`
//! 实例化后，glTF 加载器会在动画根节点上放一个 `AnimationPlayer`。
//! [`装配模型动画`] 找到那个播放器、编译单片段动画图并套用速度与循环。
//!
//! # 为什么片段要按名字查
//!
//! glTF 的动画子资源标签是 `Animation0` / `Animation1`，与美术导出的
//! 名字（`Idle` / `Run`）不同；`Gltf::named_animations` 保留了名字映射，
//! 这是唯一稳定的查找入口。名字不存在时只告警一次，模型保持静态显示。

use std::collections::VecDeque;

use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex};
use bevy::animation::{AnimationClip, AnimationPlayer, RepeatAnimation};
use bevy::gltf::Gltf;
use bevy::prelude::*;

use crate::animation_components::动画播放器属性;

/// 等待被装配动画的模型实体。
///
/// 挂在预览模型根实体上：`WorldAssetRoot` 的实例化是异步的，
/// 动画播放器要等 glTF 场景真正生成后才会出现，因此需要跨帧等待。
#[derive(Component, Debug, Clone)]
pub struct 模型动画装配 {
    /// glTF 资源句柄（未带场景标签的主资产，用来查 `named_animations`）。
    pub 模型: Handle<Gltf>,
    /// 项目相对路径，只用于告警文案。
    pub 模型路径: String,
    /// 文档里的动画意图。
    pub 动画: 动画播放器属性,
}

/// 把循环开关映射成 Bevy 的重复策略。
pub fn 重复策略(循环: bool) -> RepeatAnimation {
    if 循环 {
        RepeatAnimation::Forever
    } else {
        RepeatAnimation::Never
    }
}

/// 按名字从 glTF 的命名动画里取片段句柄。
///
/// 名字按原样精确匹配（glTF 的名字区分大小写），但允许首尾空白，
/// 因为 Inspector 里复制粘贴很容易带进来。
pub fn 按名字取片段<'a>(
    命名: impl IntoIterator<Item = (&'a str, &'a Handle<AnimationClip>)>,
    名字: &str,
) -> Option<Handle<AnimationClip>> {
    let 目标 = 名字.trim();
    if 目标.is_empty() {
        return None;
    }
    命名
        .into_iter()
        .find(|(名称, _)| 名称.trim() == 目标)
        .map(|(_, 句柄)| 句柄.clone())
}

/// 在播放器上开始播放指定节点，并套用文档里的速度与循环。
///
/// 用 `start` 而不是 `play`：重新装配（例如切片段）时必须从头开始，
/// 否则会继承上一段的播放进度。
pub fn 配置播放器(
    播放器: &mut AnimationPlayer,
    节点: AnimationNodeIndex,
    属性: &动画播放器属性,
) {
    let 活动 = 播放器.start(节点);
    活动
        .set_speed(属性.播放速度)
        .set_repeat(重复策略(属性.循环));
}

/// 在运行时世界里为实体装配动画播放器。
///
/// # 步骤
///
/// 1. 确保片段与动画图两张资产表存在（裸 `World` 不会自动初始化）；
/// 2. 建一个空片段与单节点动画图，得到节点序号；
/// 3. 建播放器，按文档属性设置速度与循环；
/// 4. 把图句柄与播放器插到实体上。
///
/// # 为什么片段是空占位
///
/// 运行时世界没有 `AssetServer`，无法读取 glTF 的曲线数据。空片段让
/// 结构与播放状态成立（可调试、可断言），真实片段由预览宿主加载。
pub fn 装配运行时动画(
    世界: &mut World,
    实体: Entity,
    属性: &动画播放器属性,
) -> AnimationNodeIndex {
    世界.init_resource::<Assets<AnimationClip>>();
    世界.init_resource::<Assets<AnimationGraph>>();

    let 片段 = 世界
        .resource_mut::<Assets<AnimationClip>>()
        .add(AnimationClip::default());
    let (图, 节点) = AnimationGraph::from_clip(片段);
    let 图句柄 = 世界.resource_mut::<Assets<AnimationGraph>>().add(图);

    let mut 播放器 = AnimationPlayer::default();
    配置播放器(&mut 播放器, 节点, 属性);

    世界
        .entity_mut(实体)
        .insert((AnimationGraphHandle(图句柄), 播放器));
    节点
}

/// 在预览子树里找 glTF 加载器放下的动画播放器。
///
/// glTF 加载器把 `AnimationPlayer` 放在动画根节点上（通常是模型场景的
/// 根或某个节点根），深度优先找到第一个即可：一个 glTF 场景的动画
/// 通常共享同一个播放器。
pub fn 子树内动画播放器(
    根: Entity,
    子: &Query<&Children>,
    播放器: &Query<Entity, With<AnimationPlayer>>,
) -> Option<Entity> {
    let mut 队列 = VecDeque::from([根]);
    while let Some(当前) = 队列.pop_front() {
        if 播放器.contains(当前) {
            return Some(当前);
        }
        if let Ok(孩子列表) = 子.get(当前) {
            队列.extend(孩子列表.iter());
        }
    }
    None
}

/// 每帧尝试给预览模型装配真实动画。
///
/// # 等待与放弃
///
/// * glTF 主资产或场景尚未加载完成：保留 `模型动画装配`，下一帧再试；
/// * 模型子树已经生成但里面没有 `AnimationPlayer`（模型本来就没有动画）：
///   告警一次并移除标记，避免每帧重复扫描；
/// * 片段名不存在：同样告警一次并移除标记，模型保持静态。
pub fn 装配模型动画(
    mut 命令: Commands,
    gltf资源: Res<Assets<Gltf>>,
    mut 图形资源: ResMut<Assets<AnimationGraph>>,
    待装配: Query<(Entity, &模型动画装配)>,
    子: Query<&Children>,
    播放器实体: Query<Entity, With<AnimationPlayer>>,
    mut 播放器: Query<&mut AnimationPlayer>,
) {
    for (根, 装配) in &待装配 {
        let Some(gltf) = gltf资源.get(&装配.模型) else {
            continue;
        };
        let 片段 = 按名字取片段(
            gltf.named_animations
                .iter()
                .map(|(名称, 句柄)| (名称.as_ref(), 句柄)),
            &装配.动画.片段,
        );
        let Some(片段) = 片段 else {
            warn!(
                "模型动画：{} 里没有名为「{}」的动画；模型按静态姿势显示",
                装配.模型路径, 装配.动画.片段
            );
            命令.entity(根).remove::<模型动画装配>();
            continue;
        };

        let Some(播放器目标) = 子树内动画播放器(根, &子, &播放器实体) else {
            if 子.get(根).map(|孩子| !孩子.is_empty()).unwrap_or(false) {
                warn!(
                    "模型动画：{} 没有可播放的 AnimationPlayer；动画设置被忽略",
                    装配.模型路径
                );
                命令.entity(根).remove::<模型动画装配>();
            }
            continue;
        };

        let Ok(mut 目标播放器) = 播放器.get_mut(播放器目标) else {
            continue;
        };
        let (图, 节点) = AnimationGraph::from_clip(片段);
        let 图句柄 = 图形资源.add(图);
        配置播放器(&mut 目标播放器, 节点, &装配.动画);
        命令.entity(播放器目标).insert(AnimationGraphHandle(图句柄));
        命令.entity(根).remove::<模型动画装配>();
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::animation_components::动画播放器组件名;

    #[test]
    fn 运行时装配出正在播放的播放器() {
        let mut 世界 = World::new();
        let 实体 = 世界.spawn_empty().id();
        let 属性 = 动画播放器属性 {
            片段: "Run".to_string(),
            播放速度: 2.0,
            循环: true,
        };

        let 节点 = 装配运行时动画(&mut 世界, 实体, &属性);

        let 播放器 = 世界.get::<AnimationPlayer>(实体).expect("必须插入播放器");
        assert_eq!(播放器.playing_animations().count(), 1);
        let (活动节点, 活动) = 播放器.playing_animations().next().unwrap();
        assert_eq!(*活动节点, 节点);
        assert_eq!(活动.speed(), 2.0);
        assert_eq!(活动.repeat_mode(), RepeatAnimation::Forever);

        let 图句柄 = 世界
            .get::<AnimationGraphHandle>(实体)
            .expect("播放器必须有动画图");
        assert!(
            世界
                .resource::<Assets<AnimationGraph>>()
                .contains(图句柄.id()),
            "动画图必须真的进了资产表"
        );
        assert!(
            !世界.resource::<Assets<AnimationClip>>().is_empty(),
            "占位片段也要进资产表，否则图节点会悬空"
        );
    }

    #[test]
    fn 不循环时播放一次就停() {
        let mut 世界 = World::new();
        let 实体 = 世界.spawn_empty().id();
        装配运行时动画(
            &mut 世界,
            实体,
            &动画播放器属性 {
                片段: "Open".to_string(),
                播放速度: -1.0,
                循环: false,
            },
        );

        let 播放器 = 世界.get::<AnimationPlayer>(实体).unwrap();
        let (_, 活动) = 播放器.playing_animations().next().unwrap();
        assert_eq!(活动.repeat_mode(), RepeatAnimation::Never);
        assert_eq!(活动.speed(), -1.0, "负数速度表示倒放");
    }

    #[test]
    fn 按名字取片段精确匹配() {
        let mut 片段表 = Assets::<AnimationClip>::default();
        let idle = 片段表.add(AnimationClip::default());
        let run = 片段表.add(AnimationClip::default());
        let 命名 = vec![("Idle", &idle), ("Run", &run)];

        assert_eq!(
            按名字取片段(命名.iter().map(|(名, 句柄)| (*名, *句柄)), "Run"),
            Some(run.clone())
        );
        assert_eq!(
            按名字取片段(命名.iter().map(|(名, 句柄)| (*名, *句柄)), " Run "),
            Some(run.clone()),
            "允许首尾空白，但名字本身区分大小写"
        );
        assert!(
            按名字取片段(命名.iter().map(|(名, 句柄)| (*名, *句柄)), "run").is_none(),
            "大小写不同视为不同片段"
        );
        assert!(
            按名字取片段(命名.iter().map(|(名, 句柄)| (*名, *句柄)), "Walk").is_none(),
            "不存在的片段必须返回 None"
        );
        assert!(
            按名字取片段(命名.iter().map(|(名, 句柄)| (*名, *句柄)), "  ").is_none(),
            "空片段名不能匹配任何东西"
        );
    }

    #[test]
    fn 重复策略映射() {
        assert_eq!(重复策略(true), RepeatAnimation::Forever);
        assert_eq!(重复策略(false), RepeatAnimation::Never);
    }

    #[test]
    fn 重新配置会从头播放() {
        let mut 播放器 = AnimationPlayer::default();
        let 属性 = 动画播放器属性::default();
        let 节点 = AnimationNodeIndex::new(0);
        配置播放器(&mut 播放器, 节点, &属性);
        let 首段 = *播放器
            .playing_animations()
            .next()
            .map(|(序号, _)| 序号)
            .unwrap();
        配置播放器(&mut 播放器, 节点, &属性);
        assert_eq!(
            播放器.playing_animations().count(),
            1,
            "同一节点重复配置不应长出第二个活动动画"
        );
        assert_eq!(首段, 节点);
    }

    #[test]
    fn 动画组件名可在注册表里找到() {
        assert_eq!(动画播放器组件名, "动画播放器");
    }
}
