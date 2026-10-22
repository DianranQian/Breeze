//! R13.2：PIE 运行生命周期（暂停/继续/单步）在真实 Bevy 世界里的行为。
//!
//! 这里不是"检查某个布尔字段"，而是**真的跑系统**：
//! 用 `World::run_system_once` 调 `推进原型`，然后看模拟帧号与方块 Transform
//! 是否变了。暂停期间调用同一系统必须一次都不推进。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::pie_proto::{
    Pie原型状态, Pie方块, Pie输入, Pie运行状态, pie暂停, pie继续, pie请求单步, 单步步长秒, 右边界,
    左边界, 推进原型为测试用,
};

/// 建一个最小世界：一个方块实体 + 生命周期资源。
fn 建世界(状态: Pie运行状态) -> World {
    let mut 世界 = World::new();
    世界.insert_resource(Time::<()>::default());
    世界.insert_resource(Pie输入::default());
    世界.insert_resource(Pie原型状态 {
        状态,
        方块方向: 1.0,
        ..Pie原型状态::default()
    });
    世界.spawn((Pie方块, Transform::from_xyz(0.0, 0.0, 0.0)));
    世界
}

fn 跑一帧(世界: &mut World) {
    世界
        .run_system_once(推进原型为测试用)
        .expect("推进原型应当可运行");
}

fn 方块x(世界: &mut World) -> f32 {
    世界
        .query_filtered::<&Transform, With<Pie方块>>()
        .iter(世界)
        .next()
        .map(|变换| 变换.translation.x)
        .unwrap_or(f32::NAN)
}

fn 状态(世界: &World) -> &Pie原型状态 {
    世界.resource::<Pie原型状态>()
}

#[test]
fn 暂停期间模拟系统一次都不推进() {
    let mut 世界 = 建世界(Pie运行状态::运行中);
    // 先正常跑 5 帧。
    for _ in 0..5 {
        跑一帧(&mut 世界);
    }
    let 帧前 = 状态(&世界).帧;
    let 位置前 = 方块x(&mut 世界);
    assert!(帧前 >= 5, "运行中每帧都应推进：{帧前}");

    // 暂停：同一个系统再跑 20 帧，模拟必须完全不动。
    {
        let mut 状态 = 世界.resource_mut::<Pie原型状态>();
        let mut 输入 = Pie输入::default();
        pie暂停(&mut 状态, &mut 输入);
    }
    for _ in 0..20 {
        跑一帧(&mut 世界);
    }
    assert_eq!(状态(&世界).帧, 帧前, "暂停期间模拟计数不能增长");
    assert_eq!(方块x(&mut 世界), 位置前, "暂停期间方块不能移动");
    assert_eq!(状态(&世界).本次步长秒, 0.0, "暂停帧的步长必须是 0");
}

#[test]
fn 单步恰好推进一次模拟且仍暂停() {
    let mut 世界 = 建世界(Pie运行状态::运行中);
    跑一帧(&mut 世界);
    {
        let mut 状态 = 世界.resource_mut::<Pie原型状态>();
        let mut 输入 = Pie输入::default();
        pie暂停(&mut 状态, &mut 输入);
        pie请求单步(&mut 状态);
    }
    let 帧前 = 状态(&世界).帧;
    let 位置前 = 方块x(&mut 世界);

    // 有单步请求的那一帧推进一次。
    跑一帧(&mut 世界);
    assert_eq!(状态(&世界).帧, 帧前 + 1, "单步必须恰好推进一帧");
    assert_ne!(方块x(&mut 世界), 位置前, "单步必须真的改动方块");
    assert_eq!(状态(&世界).状态, Pie运行状态::已暂停, "单步后仍暂停");
    assert_eq!(状态(&世界).单步执行次数, 1);
    assert!((状态(&世界).本次步长秒 - 单步步长秒).abs() < 1e-6);

    // 后续帧不再推进。
    for _ in 0..10 {
        跑一帧(&mut 世界);
    }
    assert_eq!(状态(&世界).帧, 帧前 + 1, "单步之后不能再自动推进");
}

#[test]
fn 继续后恢复正常推进() {
    let mut 世界 = 建世界(Pie运行状态::运行中);
    跑一帧(&mut 世界);
    {
        let mut 状态 = 世界.resource_mut::<Pie原型状态>();
        let mut 输入 = Pie输入::default();
        pie暂停(&mut 状态, &mut 输入);
        pie继续(&mut 状态, &mut 输入);
    }
    let 帧前 = 状态(&世界).帧;
    for _ in 0..3 {
        跑一帧(&mut 世界);
    }
    assert_eq!(状态(&世界).帧, 帧前 + 3);
}

#[test]
fn 停止后系统不推进且单步请求被丢弃() {
    let mut 世界 = 建世界(Pie运行状态::已暂停);
    {
        let mut 状态 = 世界.resource_mut::<Pie原型状态>();
        pie请求单步(&mut 状态);
        // 停止等价清理：会话结束 + 丢弃单步。
        状态.状态 = Pie运行状态::已停止;
        状态.待单步 = 0;
    }
    let 帧前 = 状态(&世界).帧;
    for _ in 0..10 {
        跑一帧(&mut 世界);
    }
    assert_eq!(状态(&世界).帧, 帧前, "停止后不能推进");
    assert_eq!(状态(&世界).待单步, 0);
}

#[test]
fn 大dt被夹住且方块留在场地内() {
    let mut 世界 = 建世界(Pie运行状态::运行中);
    // 直接把全局时间拨大：系统内部必须自己夹住。
    {
        let mut 时间 = 世界.resource_mut::<Time>();
        时间.advance_by(std::time::Duration::from_secs(30));
    }
    跑一帧(&mut 世界);
    let x = 方块x(&mut 世界);
    assert!(
        (左边界 - 0.001..=右边界 + 0.001).contains(&x),
        "大 dt 会让方块瞬移出场地：{x}"
    );
}
