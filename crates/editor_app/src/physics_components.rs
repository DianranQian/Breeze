//! 二维物理组件的**数据**：碰撞体与刚体。
//!
//! # 现状与边界
//!
//! 这里只定义 `碰撞体2d` / `刚体2d` 的场景数据与 Inspector 字段；真正的
//! 模拟在 [`crate::physics_runtime`]，且**只在点击「运行」之后**发生。
//! 编辑期能做的仍然是：加组件、改尺寸与参数、在视口里看碰撞轮廓
//! （见 `editor_ui::collision_overlay`）、随场景保存。
//!
//! 三维版本（`碰撞体3d` / `刚体3d`）在
//! [`crate::physics3d_components`]，模拟在 [`crate::physics3d_runtime`]。
//! 两者共用 [`刚体类型`]，因为它们对"静态 / 动态 / 运动学"的语义完全相同。
//!
//! 把数据与模拟分开的好处是：场景格式只依赖"形状 + 尺寸 + 质量 +
//! 摩擦/弹性"这组公共交集，换成 avian / rapier 都不需要改文件格式；
//! 而"能看见碰撞体在哪"本身就是排错时最有用的一半。
//!
//! # 参考 Godot
//!
//! Godot 的 `CollisionShape2D`（形状资源）+ `RigidBody2D`（mode: static/
//! dynamic/kinematic）是分开的两个节点，编辑期用形状的可视化轮廓调试
//! （`--debug-collisions`）。这里同样把"形状"和"刚体"分成两个组件，
//! 一个实体可以只有碰撞体（触发器/静态障碍），也可以两者都有。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use scene_doc::property::{属性值, 枚举值};
use scene_doc::registry::{属性描述, 属性种类, 组件类型};

/// 碰撞体组件类型名。
pub const 碰撞体组件名: &str = "碰撞体2d";
/// 刚体组件类型名。
pub const 刚体组件名: &str = "刚体2d";

/// 碰撞体形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 碰撞形状 {
    /// 矩形。
    矩形,
    /// 圆形。
    圆形,
}

impl 碰撞形状 {
    /// 全部可选形状（顺序即下拉顺序）。
    pub const 全部: [碰撞形状; 2] = [碰撞形状::矩形, 碰撞形状::圆形];

    /// 在场景文件里使用的变体名。
    pub fn 变体名(&self) -> &'static str {
        match self {
            // 用英文：场景格式要向运行时组件名看齐，中文名只出现在界面上。
            碰撞形状::矩形 => "Rectangle",
            碰撞形状::圆形 => "Circle",
        }
    }

    /// 下拉序号。
    pub fn 序号(&self) -> u32 {
        match self {
            碰撞形状::矩形 => 0,
            碰撞形状::圆形 => 1,
        }
    }

    /// 由场景文件里的变体名与序号还原。未知名字按序号兜底，
    /// 保证旧文件或手改文件不会让组件整体失效。
    pub fn 由值(变体名: &str, 序号: u32) -> Self {
        match 变体名 {
            "Circle" | "圆形" => 碰撞形状::圆形,
            "Rectangle" | "矩形" => 碰撞形状::矩形,
            _ => match 序号 {
                1 => 碰撞形状::圆形,
                _ => 碰撞形状::矩形,
            },
        }
    }

    /// 显示名词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            // 复用形状名（与几何体形状下拉里的"矩形/圆形"同义）。
            碰撞形状::矩形 => "shape_rectangle",
            碰撞形状::圆形 => "shape_circle",
        }
    }
}

/// 碰撞体属性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct 碰撞体属性 {
    /// 形状。
    pub 形状: 碰撞形状,
    /// 宽度（米/世界单位，矩形用）。
    pub 宽度: f32,
    /// 高度（矩形用）。
    pub 高度: f32,
    /// 半径（圆形用）。
    pub 半径: f32,
    /// 是否只是触发器（不阻挡移动，只上报重叠）。
    pub 触发器: bool,
}

impl Default for 碰撞体属性 {
    fn default() -> Self {
        Self {
            形状: 碰撞形状::矩形,
            宽度: 1.0,
            高度: 1.0,
            半径: 0.5,
            触发器: false,
        }
    }
}

/// 刚体类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 刚体类型 {
    /// 静态：不动，碰撞时把别的刚体推开。
    静态,
    /// 动态：受重力与力影响。
    动态,
    /// 运动学：由代码驱动，不与其它刚体交换冲量。
    运动学,
}

impl 刚体类型 {
    /// 全部可选类型。
    pub const 全部: [刚体类型; 3] = [刚体类型::静态, 刚体类型::动态, 刚体类型::运动学];

    /// 场景文件里的变体名。
    pub fn 变体名(&self) -> &'static str {
        match self {
            刚体类型::静态 => "Static",
            刚体类型::动态 => "Dynamic",
            刚体类型::运动学 => "Kinematic",
        }
    }

    /// 下拉序号。
    pub fn 序号(&self) -> u32 {
        match self {
            刚体类型::静态 => 0,
            刚体类型::动态 => 1,
            刚体类型::运动学 => 2,
        }
    }

    /// 由变体名与序号还原。
    pub fn 由值(变体名: &str, 序号: u32) -> Self {
        match 变体名 {
            "Static" | "静态" => 刚体类型::静态,
            "Dynamic" | "动态" => 刚体类型::动态,
            "Kinematic" | "运动学" => 刚体类型::运动学,
            _ => match 序号 {
                2 => 刚体类型::运动学,
                1 => 刚体类型::动态,
                _ => 刚体类型::静态,
            },
        }
    }

    /// 显示名词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            刚体类型::静态 => "rigid_body_static",
            刚体类型::动态 => "rigid_body_dynamic",
            刚体类型::运动学 => "rigid_body_kinematic",
        }
    }
}

/// 刚体属性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct 刚体属性 {
    /// 类型。
    pub 类型: 刚体类型,
    /// 质量（动态刚体用）。
    pub 质量: f32,
    /// 重力缩放（0 表示失重）。
    pub 重力缩放: f32,
}

impl Default for 刚体属性 {
    fn default() -> Self {
        Self {
            类型: 刚体类型::动态,
            质量: 1.0,
            重力缩放: 1.0,
        }
    }
}

/// 碰撞体组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 碰撞体组件类型;

impl 组件类型 for 碰撞体组件类型 {
    fn 组件名(&self) -> &'static str {
        碰撞体组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(碰撞体属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        碰撞体字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<碰撞体属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert(
            "形状".to_string(),
            属性值::枚举(枚举值::新建(属性.形状.变体名(), 属性.形状.序号())),
        );
        结果.insert("宽度".to_string(), 属性值::浮点(属性.宽度));
        结果.insert("高度".to_string(), 属性值::浮点(属性.高度));
        结果.insert("半径".to_string(), 属性值::浮点(属性.半径));
        结果.insert("触发器".to_string(), 属性值::布尔(属性.触发器));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<碰撞体属性>() else {
            return false;
        };
        if let Some(属性值::枚举(枚举)) = 数据.get("形状") {
            属性.形状 = 碰撞形状::由值(&枚举.变体名, 枚举.序号);
        }
        // 尺寸下一律夹到正数：0 或负尺寸的碰撞体会让视口画出一条看不见的线，
        // 用户会以为"碰撞体没生效"。
        if let Some(值) = 数据.get("宽度").and_then(属性值::取浮点) {
            属性.宽度 = 值.max(0.001);
        }
        if let Some(值) = 数据.get("高度").and_then(属性值::取浮点) {
            属性.高度 = 值.max(0.001);
        }
        if let Some(值) = 数据.get("半径").and_then(属性值::取浮点) {
            属性.半径 = 值.max(0.001);
        }
        if let Some(值) = 数据.get("触发器").and_then(属性值::取布尔) {
            属性.触发器 = 值;
        }
        true
    }
}

/// 刚体组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 刚体组件类型;

impl 组件类型 for 刚体组件类型 {
    fn 组件名(&self) -> &'static str {
        刚体组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(刚体属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        刚体字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<刚体属性>()?;
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
        let Some(属性) = 实例.downcast_mut::<刚体属性>() else {
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

fn 碰撞体字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        // 选项三元组是 (显示名词条键, 场景文件里的变体名, 序号)：
        // 变体名用英文（场景格式向运行时看齐），显示名走词条表。
        let 选项: Vec<(String, String, u32)> = 碰撞形状::全部
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
                "collision_field_shape",
                属性种类::枚举 { 选项 },
                属性值::枚举(枚举值::新建(碰撞形状::矩形.变体名(), 0)),
            ),
            属性描述::新建(
                "宽度",
                "collision_field_width",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(1.0),
            )
            .带提示("collision_field_size_tooltip"),
            属性描述::新建(
                "高度",
                "collision_field_height",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(1.0),
            )
            .带提示("collision_field_size_tooltip"),
            属性描述::新建(
                "半径",
                "collision_field_radius",
                属性种类::浮点范围(0.001, 1000.0, 0.01),
                属性值::浮点(0.5),
            )
            .带提示("collision_field_size_tooltip"),
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

fn 刚体字段() -> &'static [属性描述] {
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

/// 物理组件名列表。
pub fn 物理组件名列表() -> Vec<&'static str> {
    vec![碰撞体组件名, 刚体组件名]
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 完整注册表包含物理组件且默认值自洽() {
        let 注册表 = crate::scene_bridge::全部内置组件注册表();
        for 名字 in 物理组件名列表() {
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
    fn 碰撞体往返并夹取尺寸() {
        let 类型 = 碰撞体组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(
            "形状".into(),
            属性值::枚举(枚举值::新建(碰撞形状::圆形.变体名(), 1)),
        );
        数据.insert("半径".into(), 属性值::浮点(0.0));
        数据.insert("触发器".into(), 属性值::布尔(true));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        match 导出.get("形状") {
            Some(属性值::枚举(枚举)) => {
                assert_eq!(枚举.变体名, "Circle");
                assert_eq!(碰撞形状::由值(&枚举.变体名, 枚举.序号), 碰撞形状::圆形);
            }
            _ => panic!("形状应当被导出"),
        }
        let 半径 = 导出.get("半径").and_then(属性值::取浮点).unwrap();
        assert!(半径 >= 0.001, "零半径必须被夹到正数");
        assert_eq!(导出.get("触发器"), Some(&属性值::布尔(true)));
    }

    #[test]
    fn 未知形状名按序号兜底() {
        assert_eq!(碰撞形状::由值("不知道", 0), 碰撞形状::矩形);
        assert_eq!(碰撞形状::由值("不知道", 1), 碰撞形状::圆形);
        assert_eq!(刚体类型::由值("不知道", 2), 刚体类型::运动学);
    }

    #[test]
    fn 刚体往返() {
        let 类型 = 刚体组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(
            "类型".into(),
            属性值::枚举(枚举值::新建(刚体类型::运动学.变体名(), 2)),
        );
        数据.insert("重力缩放".into(), 属性值::浮点(-99.0));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        match 导出.get("类型") {
            Some(属性值::枚举(枚举)) => assert_eq!(枚举.变体名, "Kinematic"),
            _ => panic!("类型应当被导出"),
        }
        let 缩放 = 导出.get("重力缩放").and_then(属性值::取浮点).unwrap();
        assert!((缩放 + 10.0).abs() < 1e-6, "重力缩放要夹到 -10..10");
    }
}
