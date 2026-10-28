//! 三维物理组件的**数据**：刚体与碰撞体。
//!
//! # 与二维模块的分工
//!
//! `physics_components` 定义的是 `碰撞体2d` / `刚体2d`，只服务二维视口的
//! 轮廓叠加与二维物理；这里是三维版本，形状、尺寸与物理材质各自独立。
//! 两者共用 [`刚体类型`]（静态 / 动态 / 运动学）与它的词条键：三种
//! 语义在两个维度上完全一致，分成两套枚举只会让 Inspector 多一份翻译。
//!
//! # 为什么字段拆在两个组件里
//!
//! 质量属于**刚体**（惯性由质量决定），摩擦与弹性属于**碰撞面**
//! （同一刚体的不同碰撞面可以有不同材质）。这与 Rapier 的
//! `RigidBody` / `Collider` 划分一致，也与 Godot 把 `PhysicsMaterial`
//! 挂在碰撞形状上的做法一致。
//!
//! 一个实体可以只有碰撞体（当作静态障碍或触发器），也可以两者都有。
//! 只有碰撞体时物理侧按静态刚体处理（见 `physics3d_runtime`）。
//!
//! # 单位
//!
//! 场景约定 1 世界单位 = 1 米，重力按 -9.81 m/s² 计算。宽 / 高 / 深是
//! **外接尺寸**（与 `Primitive3d` 的口径一致），半高用于胶囊体与圆柱体。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use scene_doc::property::{属性值, 枚举值};
use scene_doc::registry::{属性描述, 属性种类, 组件类型};

use crate::physics_components::刚体类型;

/// 三维碰撞体组件类型名。
pub const 碰撞体3D组件名: &str = "碰撞体3d";
/// 三维刚体组件类型名。
pub const 刚体3D组件名: &str = "刚体3d";

/// 三维碰撞体形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 碰撞形状3d {
    /// 长方体（立方体、地面薄板）。
    立方体,
    /// 球体。
    球体,
    /// 胶囊体（竖直段 + 两端半球）。
    胶囊体,
    /// 圆柱体。
    圆柱体,
}

impl 碰撞形状3d {
    /// 全部可选形状（顺序即下拉顺序）。
    pub const 全部: [碰撞形状3d; 4] = [
        碰撞形状3d::立方体,
        碰撞形状3d::球体,
        碰撞形状3d::胶囊体,
        碰撞形状3d::圆柱体,
    ];

    /// 在场景文件里使用的变体名。
    pub fn 变体名(&self) -> &'static str {
        match self {
            碰撞形状3d::立方体 => "Box",
            碰撞形状3d::球体 => "Sphere",
            碰撞形状3d::胶囊体 => "Capsule",
            碰撞形状3d::圆柱体 => "Cylinder",
        }
    }

    /// 下拉序号。
    pub fn 序号(&self) -> u32 {
        match self {
            碰撞形状3d::立方体 => 0,
            碰撞形状3d::球体 => 1,
            碰撞形状3d::胶囊体 => 2,
            碰撞形状3d::圆柱体 => 3,
        }
    }

    /// 由场景文件里的变体名与序号还原。
    ///
    /// 未知名字按序号兜底，保证旧文件或手改文件不会让组件整体失效。
    pub fn 由值(变体名: &str, 序号: u32) -> Self {
        match 变体名 {
            "Box" | "Cuboid" | "立方体" => 碰撞形状3d::立方体,
            "Sphere" | "球体" => 碰撞形状3d::球体,
            "Capsule" | "胶囊体" => 碰撞形状3d::胶囊体,
            "Cylinder" | "圆柱体" => 碰撞形状3d::圆柱体,
            _ => match 序号 {
                3 => 碰撞形状3d::圆柱体,
                2 => 碰撞形状3d::胶囊体,
                1 => 碰撞形状3d::球体,
                _ => 碰撞形状3d::立方体,
            },
        }
    }

    /// 显示名词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            碰撞形状3d::立方体 => "shape_box",
            碰撞形状3d::球体 => "shape_sphere",
            碰撞形状3d::胶囊体 => "shape_capsule",
            碰撞形状3d::圆柱体 => "shape_cylinder",
        }
    }
}

/// 三维碰撞体属性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct 碰撞体3d属性 {
    /// 形状。
    pub 形状: 碰撞形状3d,
    /// 宽度（立方体用，X 轴外接尺寸）。
    pub 宽: f32,
    /// 高度（立方体用，Y 轴外接尺寸）。
    pub 高: f32,
    /// 深度（立方体用，Z 轴外接尺寸）。
    pub 深: f32,
    /// 半径（球体 / 胶囊体 / 圆柱体用）。
    pub 半径: f32,
    /// 半高（胶囊体与圆柱体的竖直段一半）。
    pub 半高: f32,
    /// 摩擦系数（0 = 无摩擦，1 = 常见固体表面）。
    pub 摩擦: f32,
    /// 弹性系数（0 = 不反弹，1 = 完全弹性）。
    pub 弹性: f32,
    /// 是否只是触发器（不阻挡移动，只上报重叠）。
    pub 触发器: bool,
}

impl Default for 碰撞体3d属性 {
    fn default() -> Self {
        Self {
            形状: 碰撞形状3d::立方体,
            宽: 1.0,
            高: 1.0,
            深: 1.0,
            半径: 0.5,
            半高: 0.5,
            摩擦: 0.5,
            弹性: 0.0,
            触发器: false,
        }
    }
}

/// 三维刚体属性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct 刚体3d属性 {
    /// 类型。
    pub 类型: 刚体类型,
    /// 质量（动态刚体用）。
    pub 质量: f32,
    /// 重力缩放（0 表示失重）。
    pub 重力缩放: f32,
}

impl Default for 刚体3d属性 {
    fn default() -> Self {
        Self {
            类型: 刚体类型::动态,
            质量: 1.0,
            重力缩放: 1.0,
        }
    }
}

/// 三维碰撞体组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 碰撞体3d组件类型;

impl 组件类型 for 碰撞体3d组件类型 {
    fn 组件名(&self) -> &'static str {
        碰撞体3D组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(碰撞体3d属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        碰撞体3d字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<碰撞体3d属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert(
            "形状".to_string(),
            属性值::枚举(枚举值::新建(属性.形状.变体名(), 属性.形状.序号())),
        );
        结果.insert("宽".to_string(), 属性值::浮点(属性.宽));
        结果.insert("高".to_string(), 属性值::浮点(属性.高));
        结果.insert("深".to_string(), 属性值::浮点(属性.深));
        结果.insert("半径".to_string(), 属性值::浮点(属性.半径));
        结果.insert("半高".to_string(), 属性值::浮点(属性.半高));
        结果.insert("摩擦".to_string(), 属性值::浮点(属性.摩擦));
        结果.insert("弹性".to_string(), 属性值::浮点(属性.弹性));
        结果.insert("触发器".to_string(), 属性值::布尔(属性.触发器));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<碰撞体3d属性>() else {
            return false;
        };
        if let Some(属性值::枚举(枚举)) = 数据.get("形状") {
            属性.形状 = 碰撞形状3d::由值(&枚举.变体名, 枚举.序号);
        }
        for (键, 目标) in [
            ("宽", &mut 属性.宽),
            ("高", &mut 属性.高),
            ("深", &mut 属性.深),
            ("半径", &mut 属性.半径),
            ("半高", &mut 属性.半高),
        ] {
            if let Some(值) = 数据.get(键).and_then(属性值::取浮点) {
                *目标 = 值.max(0.001);
            }
        }
        if let Some(值) = 数据.get("摩擦").and_then(属性值::取浮点) {
            属性.摩擦 = 值.clamp(0.0, 2.0);
        }
        if let Some(值) = 数据.get("弹性").and_then(属性值::取浮点) {
            属性.弹性 = 值.clamp(0.0, 1.0);
        }
        if let Some(值) = 数据.get("触发器").and_then(属性值::取布尔) {
            属性.触发器 = 值;
        }
        true
    }
}

/// 三维刚体组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 刚体3d组件类型;

impl 组件类型 for 刚体3d组件类型 {
    fn 组件名(&self) -> &'static str {
        刚体3D组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(刚体3d属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        刚体3d字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<刚体3d属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert(
            "类型".to_string(),
            属性值::枚举(枚举值::新建(属性.类型.变体名(), 属性.类型.序号())),
        );
        结果.insert("质量".to_string(), 属性值::浮点(属性.质量));
        结果.insert("重力缩放".to_string(), 属性值::浮点(属性.重力缩放));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<刚体3d属性>() else {
            return false;
        };
        if let Some(属性值::枚举(枚举)) = 数据.get("类型") {
            属性.类型 = 刚体类型::由值(&枚举.变体名, 枚举.序号);
        }
        if let Some(值) = 数据.get("质量").and_then(属性值::取浮点) {
            属性.质量 = 值.max(0.001);
        }
        if let Some(值) = 数据.get("重力缩放").and_then(属性值::取浮点) {
            属性.重力缩放 = 值.clamp(-10.0, 10.0);
        }
        true
    }
}

fn 碰撞体3d字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        let 选项: Vec<(String, String, u32)> = 碰撞形状3d::全部
            .iter()
            .map(|形状| {
                (
                    形状.词条键().to_string(),
                    形状.变体名().to_string(),
                    形状.序号(),
                )
            })
            .collect();
        vec![
            属性描述::新建(
                "形状",
                "physics3d_field_shape",
                属性种类::枚举 { 选项 },
                属性值::枚举(枚举值::新建(碰撞形状3d::立方体.变体名(), 0)),
            ),
            属性描述::新建(
                "宽",
                "collision_field_width",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(1.0),
            )
            .带提示("physics3d_field_size_tooltip"),
            属性描述::新建(
                "高",
                "collision_field_height",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(1.0),
            )
            .带提示("physics3d_field_size_tooltip"),
            属性描述::新建(
                "深",
                "collision_field_depth",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(1.0),
            )
            .带提示("physics3d_field_size_tooltip"),
            属性描述::新建(
                "半径",
                "collision_field_radius",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(0.5),
            )
            .带提示("physics3d_field_curve_tooltip"),
            属性描述::新建(
                "半高",
                "collision_field_half_height",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(0.5),
            )
            .带提示("physics3d_field_curve_tooltip"),
            属性描述::新建(
                "摩擦",
                "physics3d_field_friction",
                属性种类::浮点范围(0.0, 2.0, 0.01),
                属性值::浮点(0.5),
            )
            .带提示("physics3d_field_friction_tooltip"),
            属性描述::新建(
                "弹性",
                "physics3d_field_restitution",
                属性种类::浮点范围(0.0, 1.0, 0.01),
                属性值::浮点(0.0),
            )
            .带提示("physics3d_field_restitution_tooltip"),
            属性描述::新建(
                "触发器",
                "collision_field_trigger",
                属性种类::布尔,
                属性值::布尔(false),
            )
            .带提示("collision_field_trigger_tooltip"),
        ]
    })
}

fn 刚体3d字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        let 选项: Vec<(String, String, u32)> = 刚体类型::全部
            .iter()
            .map(|类型| {
                (
                    类型.词条键().to_string(),
                    类型.变体名().to_string(),
                    类型.序号(),
                )
            })
            .collect();
        vec![
            属性描述::新建(
                "类型",
                "rigid_body_field_type",
                属性种类::枚举 { 选项 },
                属性值::枚举(枚举值::新建(刚体类型::动态.变体名(), 1)),
            ),
            属性描述::新建(
                "质量",
                "rigid_body_field_mass",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(1.0),
            ),
            属性描述::新建(
                "重力缩放",
                "rigid_body_field_gravity_scale",
                属性种类::浮点范围(-10.0, 10.0, 0.1),
                属性值::浮点(1.0),
            )
            .带提示("rigid_body_field_gravity_scale_tooltip"),
        ]
    })
}

/// 三维物理组件名列表。
pub fn 物理3d组件名列表() -> Vec<&'static str> {
    vec![碰撞体3D组件名, 刚体3D组件名]
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 三维物理组件已注册且默认值自洽() {
        let 注册表 = crate::scene_bridge::全部内置组件注册表();
        for 名字 in 物理3d组件名列表() {
            assert!(注册表.已注册(名字));
            let 类型 = 注册表.要求(名字).expect("应当能查到");
            for 描述 in 类型.字段描述() {
                assert!(
                    描述.种类.接受(&描述.默认值),
                    "{名字}.{} 的默认值类型不对",
                    描述.字段名
                );
            }
        }
    }

    #[test]
    fn 碰撞体形状名字与序号往返一致() {
        for (形状, 名字, 序号) in [
            (碰撞形状3d::立方体, "Box", 0),
            (碰撞形状3d::球体, "Sphere", 1),
            (碰撞形状3d::胶囊体, "Capsule", 2),
            (碰撞形状3d::圆柱体, "Cylinder", 3),
        ] {
            assert_eq!(形状.变体名(), 名字);
            assert_eq!(形状.序号(), 序号);
            assert_eq!(碰撞形状3d::由值(名字, 99), 形状);
        }
        assert_eq!(碰撞形状3d::由值("不存在的形状", 2), 碰撞形状3d::胶囊体);
    }

    #[test]
    fn 碰撞体往返并夹取尺寸与材质() {
        let 类型 = 碰撞体3d组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(
            "形状".into(),
            属性值::枚举(枚举值::新建(碰撞形状3d::胶囊体.变体名(), 2)),
        );
        数据.insert("半径".into(), 属性值::浮点(0.0));
        数据.insert("半高".into(), 属性值::浮点(2.0));
        数据.insert("摩擦".into(), 属性值::浮点(9.0));
        数据.insert("弹性".into(), 属性值::浮点(-1.0));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        match 导出.get("形状") {
            Some(属性值::枚举(枚举)) => {
                assert_eq!(枚举.变体名, "Capsule");
                assert_eq!(枚举.序号, 2);
            }
            _ => panic!("形状应当被导出"),
        }
        let 取 = |键: &str| 导出.get(键).and_then(属性值::取浮点).unwrap();
        assert!((取("半径") - 0.001).abs() < 1e-6, "零半径必须被夹到正数");
        assert!((取("半高") - 2.0).abs() < 1e-6);
        assert!((取("摩擦") - 2.0).abs() < 1e-6, "摩擦要夹到 0..2");
        assert_eq!(取("弹性"), 0.0, "弹性要夹到 0..1");
    }

    #[test]
    fn 刚体往返并夹取质量与重力缩放() {
        let 类型 = 刚体3d组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(
            "类型".into(),
            属性值::枚举(枚举值::新建(刚体类型::运动学.变体名(), 2)),
        );
        数据.insert("质量".into(), 属性值::浮点(0.0));
        数据.insert("重力缩放".into(), 属性值::浮点(-99.0));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        match 导出.get("类型") {
            Some(属性值::枚举(枚举)) => assert_eq!(枚举.变体名, "Kinematic"),
            _ => panic!("类型应当被导出"),
        }
        let 质量 = 导出.get("质量").and_then(属性值::取浮点).unwrap();
        assert!(质量 >= 0.001);
        let 缩放 = 导出.get("重力缩放").and_then(属性值::取浮点).unwrap();
        assert!((缩放 + 10.0).abs() < 1e-6, "重力缩放要夹到 -10..10");
    }
}
