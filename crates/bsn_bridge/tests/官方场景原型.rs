//! 官方 BSN 场景测试：验证 Bevy 0.20.0 的 `bsn!` 在真实世界里的产出。
//!
//! 这一组测试**只依赖官方能力**，不涉及本项目的转换层。
//! 它的作用是给出"官方 BSN 到底能做到什么"的可执行证据：
//! 谁能构建场景、谁能实例化、变换与层级是否真的正确、组件补丁语义是否如文档所说。
//!
//! 如果哪天升级 Bevy 后这些测试失败，说明官方 API 变了 ——
//! 那时应该先去读新版本源码，而不是改测试去迁就实现。

use bevy::prelude::*;

use bsn_bridge::{BSN宿主, BSN错误};

/// 一个用于验证"组件补丁"语义的自定义组件。
///
/// # 为什么类型名是 ASCII
///
/// `bsn!` 用首字符是否大写来区分"类型"与"值"，而汉字没有大小写，
/// 中文类型名会被判成函数值。详见 `tests/标识符约束.rs`。
/// 字段名不受影响，所以这里保留项目惯用的中文字段名。
///
/// BSN 要求 `Default + Clone` 才能直接用类型名写入场景，
/// 这两个 derive 就是官方文档所说的"最简单的路径"。
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
struct HealthBar {
    当前: i64,
    最大: i64,
}

/// 带非零默认值的组件，用来验证"未提到的字段保留默认值"。
///
/// 刻意**手写** `Default` 而不是派生：派生的默认值是 0，
/// 一旦补丁把未提到的字段清零，断言就分不清"保留默认"和"被覆盖成 0"。
#[derive(Component, Clone, Debug, PartialEq, Eq)]
struct Armor {
    值: i64,
    等级: i64,
}

impl Default for Armor {
    fn default() -> Self {
        Self { 值: 10, 等级: 3 }
    }
}

#[test]
fn bsn_宏能构建并实例化多个实体() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            #营地
            Transform { translation: { Vec3::new(1.0, 2.0, 3.0) } }
            Children [
                #篝火 HealthBar { 当前: 30 }
                --
                #哨塔 HealthBar
            ]
        })
        .expect("官方 BSN 场景应立即实例化成功");

    assert!(宿主.存在(根));
    assert_eq!(宿主.名称(根).as_deref(), Some("营地"));

    let 孩子 = 宿主.子实体(根);
    assert_eq!(孩子.len(), 2, "`Children [ a -- b ]` 应产生两个子实体");
    assert_eq!(宿主.名称(孩子[0]).as_deref(), Some("篝火"));
    assert_eq!(宿主.名称(孩子[1]).as_deref(), Some("哨塔"));

    // 组件真的进了世界，且字段值来自 BSN。
    assert_eq!(
        宿主.取组件::<HealthBar>(孩子[0]),
        Some(&HealthBar {
            当前: 30, 最大: 0
        }),
        "`HealthBar {{ 当前: 30 }}` 应把当前值设为 30，未提到的字段取 Default"
    );
    assert_eq!(
        宿主.取组件::<HealthBar>(孩子[1]),
        Some(&HealthBar::default())
    );
}

#[test]
fn 基础变换在实例化后逐分量正确() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            #带变换的实体
            Transform {
                translation: { Vec3::new(-7.5, 12.25, 0.5) },
                rotation: { Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) },
                scale: { Vec3::new(2.0, 3.0, 4.0) }
            }
        })
        .expect("实例化");

    let 变换 = 宿主.变换(根).expect("实体应当有 Transform");
    assert_eq!(变换.translation, Vec3::new(-7.5, 12.25, 0.5));
    assert_eq!(变换.scale, Vec3::new(2.0, 3.0, 4.0));
    // 绕 y 轴 90°：四元数为 (0, sin45°, 0, cos45°)。
    let 期望半角 = std::f32::consts::FRAC_PI_4;
    assert!((变换.rotation.y - 期望半角.sin()).abs() < 1e-6);
    assert!((变换.rotation.w - 期望半角.cos()).abs() < 1e-6);
    assert!(变换.rotation.x.abs() < 1e-6);
    assert!(变换.rotation.z.abs() < 1e-6);
}

#[test]
fn 组件补丁是合并不是整体替换() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            #补丁演示
            Armor
            Armor { 值: 99 }
        })
        .expect("实例化");

    let 组件 = 宿主.取组件::<Armor>(根).expect("应有 Armor 组件");
    assert_eq!(组件.值, 99, "后一条补丁应覆盖 `值`");
    assert_eq!(组件.等级, 3, "未提到的 `等级` 应保留默认值 3，而不是被清零");
}

#[test]
fn 层级关系双向成立且子级有序() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            #根
            Children [
                #甲
                --
                #乙
                --
                #丙
            ]
        })
        .expect("实例化");

    let 孩子 = 宿主.子实体(根);
    let 名称: Vec<String> = 孩子.iter().filter_map(|实体| 宿主.名称(*实体)).collect();
    assert_eq!(
        名称,
        vec!["甲", "乙", "丙"],
        "Children 的顺序必须与 BSN 写法一致"
    );

    // 每个子实体都必须有指回根的 ChildOf。
    for 子 in &孩子 {
        assert_eq!(宿主.父实体(*子), Some(根), "子实体应通过 ChildOf 指回根");
    }
    // 根自己没有父实体。
    assert_eq!(宿主.父实体(根), None);
}

#[test]
fn 多层嵌套层级正确() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            #世界根
            Children [
                #区域
                Children [
                    #道具
                    Transform { translation: { Vec3::new(5.0, 0.0, 0.0) } }
                    Children [
                        #道具的装饰
                    ]
                ]
            ]
        })
        .expect("实例化");

    let 区域 = 宿主.子实体(根);
    assert_eq!(区域.len(), 1);
    let 道具 = 宿主.子实体(区域[0]);
    assert_eq!(道具.len(), 1);
    let 装饰 = 宿主.子实体(道具[0]);
    assert_eq!(装饰.len(), 1);

    assert_eq!(宿主.名称(装饰[0]).as_deref(), Some("道具的装饰"));
    assert_eq!(宿主.父实体(装饰[0]), Some(道具[0]));
    assert_eq!(宿主.父实体(道具[0]), Some(区域[0]));
}

#[test]
fn bsn_list_产生多个根实体() {
    let mut 宿主 = BSN宿主::新建();
    let 根列表 = 宿主
        .实例化列表(bsn_list! {
            #左墙
            Transform { translation: { Vec3::new(-10.0, 0.0, 0.0) } }
            --
            #右墙
            Transform { translation: { Vec3::new(10.0, 0.0, 0.0) } }
        })
        .expect("场景列表应实例化成功");

    assert_eq!(根列表.len(), 2, "bsn_list! 的每个 `--` 段产生一个根实体");
    assert_eq!(宿主.名称(根列表[0]).as_deref(), Some("左墙"));
    assert_eq!(宿主.名称(根列表[1]).as_deref(), Some("右墙"));
    // 两个都是根：没有父实体。
    assert_eq!(宿主.父实体(根列表[0]), None);
    assert_eq!(宿主.父实体(根列表[1]), None);
    assert_eq!(
        宿主.变换(根列表[0]).map(|值| 值.translation),
        Some(Vec3::new(-10.0, 0.0, 0.0))
    );
}

#[test]
fn 必需组件被引擎自动补齐() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            #有变换的
            Transform { translation: { Vec3::new(1.0, 1.0, 1.0) } }
        })
        .expect("实例化");

    // Transform 的必需组件 GlobalTransform 由引擎自动加上。
    assert!(
        宿主.含组件::<GlobalTransform>(根),
        "插入 Transform 后 GlobalTransform 应由必需组件机制补齐"
    );
}

#[test]
fn 没有井号名字时不产生名称组件() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            Transform { translation: { Vec3::new(0.0, 0.0, 0.0) } }
        })
        .expect("实例化");

    assert_eq!(
        宿主.名称(根),
        None,
        "BSN 里没写 `#名字` 时不应凭空产生 Name 组件"
    );
}

#[test]
fn 裸世界实例化返回错误而不是崩溃() {
    // 裸世界缺少 AssetServer：适配层必须返回错误。
    let mut 世界 = World::new();
    let 结果 = bsn_bridge::在世界上实例化(
        &mut 世界,
        bsn! {
            #孤立
        },
    );
    match 结果 {
        Err(BSN错误::世界缺少资源 { 缺少 }) => assert_eq!(缺少, "AssetServer"),
        其他 => panic!("期望缺少资源的错误，实际：{其他:?}"),
    }
}
