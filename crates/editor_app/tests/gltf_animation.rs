//! 中文名：glTF 动画
//! 动画播放组件的端到端验收：文档数据 → 运行时播放器 → 预览投影。
//!
//! # 为什么同时断言"运行时结构"与"文档不变"
//!
//! 动画有两个最容易"看起来接上了、实际没通"的接缝：
//!
//! 1. **运行时**：文档里的片段名 / 速度 / 循环必须真的落到 Bevy 的
//!    `AnimationPlayer` 上，而不是只留在文档组件里；
//! 2. **隔离**：播放器与动画图是运行时产物，停止运行后必须随世界一起消失，
//!    场景文档不能因为点过"运行"而多出组件。
//!
//! 运行时世界不加载 glTF（没有 `AssetServer`），因此片段是空占位；
//! 真实片段的名字解析在预览宿主完成，这里通过投影与解析函数覆盖那条线。

use bevy::animation::graph::AnimationGraphHandle;
use bevy::animation::{AnimationPlayer, RepeatAnimation};
use bevy::prelude::*;

use editor_app::animation_components::{
    动画播放器属性, 动画播放器组件名, 动画播放器组件类型, 循环字段, 片段字段, 速度字段,
};
use editor_app::preview::投影预览项;
use editor_app::scene_bridge::{全部内置组件注册表, 构建世界};
use scene_doc::document::{场景文档, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 资源引用值};
use scene_doc::registry::组件类型;

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

fn 挂动画(
    文档: &mut 场景文档, 标识: 实体标识, 片段: &str, 速度: f32, 循环: bool
) {
    let 属性: std::collections::BTreeMap<String, 属性值> = [
        (片段字段.to_string(), 属性值::文本(片段.to_string())),
        (速度字段.to_string(), 属性值::浮点(速度)),
        (循环字段.to_string(), 属性值::布尔(循环)),
    ]
    .into_iter()
    .collect();
    let 实体 = 文档.取可变(标识).expect("实体必须存在");
    assert!(
        实体.加组件(组件数据::新建(动画播放器组件名, 属性)),
        "同一实体不应重复挂动画组件"
    );
}

/// 一个挂了 glTF 模型与动画播放器的实体。
fn 建动画场景() -> (场景文档, 实体标识) {
    let mut 文档 = 场景文档::新建("动画");
    let 标识 = 挂组件(
        &mut 文档,
        "角色",
        "Sprite",
        &[(
            "模型",
            属性值::资源(资源引用值::按路径(
                项目相对路径::解析("models/hero.glb").expect("路径合法"),
            )),
        )],
    );
    挂动画(&mut 文档, 标识, "Run", 2.0, true);
    (文档, 标识)
}

#[test]
fn 运行时把动画意图落成真实播放器() {
    let (文档, 标识) = 建动画场景();
    let 注册表 = 全部内置组件注册表();
    let 启动前组件数: usize = 文档.实体表.values().map(|实体| 实体.组件.len()).sum();

    let mut 世界 = World::new();
    let (统计, 映射) = 构建世界(&mut 世界, &文档, &注册表);
    assert_eq!(统计.实体数, 1);
    assert!(
        !统计.跳过类型.contains(&动画播放器组件名.to_string()),
        "动画播放器已注册并支持，不应被跳过"
    );

    let 运行时实体 = 映射.取运行时(标识).expect("文档实体必须有运行时实体");

    let 属性 = 世界
        .get::<动画播放器属性>(运行时实体)
        .expect("运行时实体必须保留文档属性，调试器才看得到意图");
    assert_eq!(属性.片段, "Run");
    assert_eq!(属性.播放速度, 2.0);

    let 播放器 = 世界
        .get::<AnimationPlayer>(运行时实体)
        .expect("必须建立真实的 Bevy 播放器");
    assert_eq!(
        播放器.playing_animations().count(),
        1,
        "自动播放意味着启动时就有一个活动动画"
    );
    let (_, 活动) = 播放器.playing_animations().next().unwrap();
    assert_eq!(活动.speed(), 2.0, "速度必须写进活动动画");
    assert_eq!(活动.repeat_mode(), RepeatAnimation::Forever);

    assert!(
        世界.get::<AnimationGraphHandle>(运行时实体).is_some(),
        "播放器必须带动画图，否则动画目标找不到曲线"
    );

    let 启动后组件数: usize = 文档.实体表.values().map(|实体| 实体.组件.len()).sum();
    assert_eq!(
        启动前组件数, 启动后组件数,
        "构建运行时世界是只读投影，不能改文档"
    );
}

#[test]
fn 关闭循环时播放器只播一次() {
    let (mut 文档, 标识) = 建动画场景();
    if let Some(实体) = 文档.取可变(标识)
        && let Some(组件) = 实体.取组件可变(动画播放器组件名)
    {
        组件.属性.insert(循环字段.to_string(), 属性值::布尔(false));
        组件.属性.insert(速度字段.to_string(), 属性值::浮点(-0.5));
    }

    let 注册表 = 全部内置组件注册表();
    let mut 世界 = World::new();
    let (_, 映射) = 构建世界(&mut 世界, &文档, &注册表);
    let 运行时实体 = 映射.取运行时(标识).unwrap();

    let 播放器 = 世界.get::<AnimationPlayer>(运行时实体).unwrap();
    let (_, 活动) = 播放器.playing_animations().next().unwrap();
    assert_eq!(活动.repeat_mode(), RepeatAnimation::Never);
    assert_eq!(活动.speed(), -0.5, "负速度表示倒放");
}

#[test]
fn 预览投影把动画挂在渲染项上() {
    let (文档, 标识) = 建动画场景();
    let 项 = 投影预览项(&文档);
    assert_eq!(项.len(), 1);
    assert_eq!(项[0].来源, 标识);
    let 动画 = 项[0].动画.as_ref().expect("渲染项必须带上动画意图");
    assert_eq!(动画.片段, "Run");
    assert_eq!(动画.播放速度, 2.0);
    assert!(动画.循环);

    let mut 无动画 = 场景文档::新建("静态");
    挂组件(
        &mut 无动画,
        "方块",
        "Sprite",
        &[(
            "模型",
            属性值::资源(资源引用值::按路径(
                项目相对路径::解析("models/crate.glb").unwrap(),
            )),
        )],
    );
    let 项 = 投影预览项(&无动画);
    assert_eq!(项.len(), 1);
    assert!(项[0].动画.is_none(), "没挂动画组件时不能凭空播放");
}

#[test]
fn 动画组件字段可供检查器编辑() {
    let 注册表 = 全部内置组件注册表();
    assert!(注册表.已注册(动画播放器组件名));

    let 类型 = 动画播放器组件类型;
    let 字段: Vec<&str> = 类型
        .字段描述()
        .iter()
        .map(|描述| 描述.字段名.as_str())
        .collect();
    assert_eq!(字段, [片段字段, 速度字段, 循环字段]);

    let mut 实例 = 类型.默认实例();
    let mut 数据 = std::collections::BTreeMap::new();
    数据.insert(片段字段.to_string(), 属性值::文本("Walk".to_string()));
    数据.insert(速度字段.to_string(), 属性值::浮点(1.5));
    数据.insert(循环字段.to_string(), 属性值::布尔(false));
    assert!(类型.实例化(实例.as_mut(), &数据));
    let 属性 = 实例.downcast_ref::<动画播放器属性>().unwrap();
    assert_eq!(属性.片段, "Walk");
    assert_eq!(属性.播放速度, 1.5);
    assert!(!属性.循环);
}
