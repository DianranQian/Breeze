// SPDX-License-Identifier: MIT OR Apache-2.0
//! 真实、可重复运行的原生 Bevy 调试示例。
//!
//! 它创建真实的实体、组件、`Transform` 与资源，并每帧推进玩家位置。
//! 微风启动它之后会通过 `Breeze调试插件` 读取真实 ECS 世界。
//!
//! ```text
//! cargo run --release            # 手动运行（没有 BREEZE_DEBUG_* 时插件停用）
//! cargo run -- --duration-ms 8000
//! ```
//!
//! 插件在**没有**编辑器注入的 `BREEZE_DEBUG_*` 环境变量时完全停用：
//! 不监听端口、不建线程、不影响游戏。

use bevy::prelude::*;
use bevy_breeze_debug::{Breeze调试插件, 写入结果, 可写字段声明};
use runtime_protocol::{字段值, 字段视图, 操作错误, 操作错误代码, 数值范围};

/// 玩家属性：演示自定义组件的结构化检查。
#[derive(Component, Debug)]
struct 玩家属性 {
    生命值: f32,
    移动速度: f32,
}

/// 敌人标记：确保销毁系统不会把玩家当成敌人。
#[derive(Component)]
struct 敌人标记;

/// 运行时动态添加的组件（未注册检查器，用于验证组件差异）。
#[derive(Component)]
struct 护盾 {
    #[allow(dead_code)]
    强度: f32,
}

/// 一个刻意**不**注册检查器的组件：界面必须显示"不支持检查"，不能伪造字段。
#[derive(Component)]
#[allow(dead_code)]
struct 私有数据 {
    令牌: String,
}

/// 自定义资源。
#[derive(Resource)]
struct 调试示例状态 {
    回合: u32,
    场景: String,
}

/// 运行时参数。
#[derive(Resource)]
struct 运行参数 {
    剩余: f32,
    销毁敌人帧: u32,
    加组件帧: u32,
    移除组件帧: u32,
}

fn main() {
    let 持续毫秒 = 参数值("--duration-ms", 60_000);
    println!("breeze-debug-demo 启动：duration_ms={持续毫秒}");

    App::new()
        .add_plugins(MinimalPlugins)
        .insert_resource(调试示例状态 {
            回合: 1,
            场景: "训练场".to_string(),
        })
        .insert_resource(运行参数 {
            剩余: 持续毫秒 as f32 / 1000.0,
            销毁敌人帧: 参数值("--destroy-at", 5) as u32,
            加组件帧: 参数值("--add-at", 6) as u32,
            移除组件帧: 参数值("--remove-at", 7) as u32,
        })
        .add_plugins(
            Breeze调试插件::自动()
                .注册可写组件::<玩家属性, _, _>(
                    "玩家属性",
                    |实体| {
                        let Some(属性) = 实体.get::<玩家属性>() else {
                            return Vec::new();
                        };
                        vec![
                            字段视图::浮点("生命值", f64::from(属性.生命值)),
                            字段视图::浮点("移动速度", f64::from(属性.移动速度)),
                        ]
                    },
                    |属性, 字段, 索引, 目标| {
                        if 索引.is_some() {
                            return Err(操作错误 {
                                代码: 操作错误代码::值无效,
                                消息: "玩家属性字段只能整体写入".to_string(),
                            });
                        }
                        let 取范围 = |值: &f64| {
                            if 值.is_finite() && (0.0..=1000.0).contains(值) {
                                Ok(*值 as f32)
                            } else {
                                Err(操作错误 {
                                    代码: 操作错误代码::值无效,
                                    消息: "数值必须是 0..=1000 的有限数".to_string(),
                                })
                            }
                        };
                        match 字段 {
                            "生命值" => {
                                let 新值 = match 目标 {
                                    字段值::浮点(值) => 取范围(值)?,
                                    _ => {
                                        return Err(操作错误 {
                                            代码: 操作错误代码::值无效,
                                            消息: "生命值需要单个浮点数".to_string(),
                                        });
                                    }
                                };
                                let 前 = 属性.生命值;
                                属性.生命值 = 新值;
                                Ok(写入结果 {
                                    修改前值: 字段值::浮点(f64::from(前)),
                                    最终值: 字段值::浮点(f64::from(属性.生命值)),
                                })
                            }
                            "移动速度" => {
                                let 新值 = match 目标 {
                                    字段值::浮点(值) => 取范围(值)?,
                                    _ => {
                                        return Err(操作错误 {
                                            代码: 操作错误代码::值无效,
                                            消息: "移动速度需要单个浮点数".to_string(),
                                        });
                                    }
                                };
                                let 前 = 属性.移动速度;
                                属性.移动速度 = 新值;
                                Ok(写入结果 {
                                    修改前值: 字段值::浮点(f64::from(前)),
                                    最终值: 字段值::浮点(f64::from(属性.移动速度)),
                                })
                            }
                            _ => Err(操作错误 {
                                代码: 操作错误代码::字段不可写,
                                消息: format!("玩家属性.{字段} 不在写入白名单里"),
                            }),
                        }
                    },
                    vec![
                        可写字段声明::新建(
                            "生命值",
                            Some(数值范围 {
                                最小: 0.0,
                                最大: 1000.0,
                            }),
                        )
                        .只允许整体()
                        .说明("0..=1000 的有限数"),
                        可写字段声明::新建(
                            "移动速度",
                            Some(数值范围 {
                                最小: 0.0,
                                最大: 1000.0,
                            }),
                        )
                        .只允许整体()
                        .说明("0..=1000 的有限数"),
                    ],
                )
                .注册资源::<调试示例状态, _>("调试示例状态", |世界| {
                    世界.get_resource::<调试示例状态>().map(|状态| {
                        vec![
                            字段视图::文本("场景", 状态.场景.clone()),
                            字段视图::文本("回合", 状态.回合.to_string()),
                        ]
                    })
                }),
        )
        .add_systems(Startup, 生成世界)
        .add_systems(Update, (推进玩家, 推进回合, 退出计时))
        .run();
}

fn 参数值(键: &str, 默认: u64) -> u64 {
    let 参数: Vec<String> = std::env::args().collect();
    for 窗口 in 参数.windows(2) {
        if 窗口[0] == 键 {
            return 窗口[1].parse().unwrap_or(默认);
        }
    }
    默认
}

fn 生成世界(mut 命令: Commands) {
    命令.spawn((
        Name::new("玩家"),
        Transform::from_xyz(1.0, 2.0, 3.0),
        玩家属性 {
            生命值: 100.0,
            移动速度: 2.5,
        },
        私有数据 {
            令牌: "demo-secret-token".to_string(),
        },
    ));
    for 序号 in 0..8 {
        命令.spawn((
            Name::new(format!("敌人{序号}")),
            敌人标记,
            Transform::from_xyz(序号 as f32 * 3.0, 0.0, -5.0),
        ));
    }
    println!("breeze-debug-demo 已生成 9 个实体");
}

fn 推进玩家(时间: Res<Time>, mut 查询: Query<&mut Transform, With<玩家属性>>) {
    for mut 变换 in &mut 查询 {
        变换.translation.x += 时间.delta_secs() * 0.5;
    }
}

fn 推进回合(
    时间: Res<Time>,
    mut 状态: ResMut<调试示例状态>,
    参数: Res<运行参数>,
    mut 计时: Local<f32>,
    mut 命令: Commands,
    敌人: Query<Entity, With<敌人标记>>,
    玩家: Query<Entity, With<玩家属性>>,
) {
    *计时 += 时间.delta_secs();
    if *计时 < 1.0 {
        return;
    }
    *计时 = 0.0;
    状态.回合 += 1;
    // 第 90 帧左右销毁一个敌人：检查器必须反映真实实体被销毁，
    // 而不是继续显示上一帧的列表。
    if 状态.回合 == 参数.销毁敌人帧 {
        if let Some(实体) = 敌人.iter().next() {
            命令.entity(实体).despawn();
            println!("breeze-debug-demo 已销毁一个敌人");
        }
    }
    if 状态.回合 == 参数.加组件帧
        && let Some(实体) = 玩家.iter().next()
    {
        命令.entity(实体).insert(护盾 { 强度: 42.0 });
        println!("breeze-debug-demo 已给玩家添加护盾组件");
    }
    if 状态.回合 == 参数.移除组件帧
        && let Some(实体) = 玩家.iter().next()
    {
        命令.entity(实体).remove::<私有数据>();
        println!("breeze-debug-demo 已移除玩家的私有数据组件");
    }
}

fn 退出计时(
    时间: Res<Time>, mut 参数: ResMut<运行参数>, mut 退出: MessageWriter<AppExit>
) {
    参数.剩余 -= 时间.delta_secs();
    if 参数.剩余 <= 0.0 {
        println!("breeze-debug-demo 计时结束，正常退出");
        退出.write(AppExit::Success);
    }
}
