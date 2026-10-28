//! 三维灯光组件：让"场景里的灯"成为可编辑的数据。
//!
//! # 为什么要单独一个模块
//!
//! 三维灯光与 2D/3D 场景组件的**渲染路径不同**：几何体走网格材质，
//! 灯光是给渲染器提供照明的独立组件（`DirectionalLight` / `PointLight`）。
//! 把它们放进独立模块，好处有两个：
//!
//! 1. 组件定义、字段描述、预览同步三处改动都在一个文件里，评审时能一眼看全；
//! 2. 注册集中在 `scene_bridge` 的构造表里，本模块只提供组件类型本身。
//!
//! # 与 Godot 的对应
//!
//! Godot 的 `DirectionalLight3D` / `OmniLight3D` 都是**场景节点**：
//! 灯的位置/朝向来自节点变换，颜色与强度是节点属性，导出时进场景文件。
//! 这里完全同构：灯光的方向来自实体变换（四元数），颜色/强度是组件属性。
//! 因此"拖动灯的旋转"就是"改光照方向"，不需要额外的方向字段。
//!
//! # 注册表
//!
//! 这两个组件由 [`crate::scene_bridge::全部内置组件注册表`] 在启动时统一注册。
//! Inspector 的字段编辑器、添加组件菜单、场景校验**全部由注册表驱动**，
//! 所以只要注册成功，这三处自动就有它们（不需要改任何 UI 代码）。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use scene_doc::property::{属性值, 颜色值};
use scene_doc::registry::{属性描述, 属性种类, 组件类型};

/// 方向光组件在场景文件里的类型名。
pub const 方向光组件名: &str = "方向光";

/// 点光组件在场景文件里的类型名。
pub const 点光组件名: &str = "点光";

/// 聚光灯组件在场景文件里的类型名。
pub const 聚光灯组件名: &str = "聚光灯";

/// 方向光属性。
#[derive(Debug, Clone, PartialEq)]
pub struct 方向光属性 {
    /// 光色。
    pub 颜色: 颜色值,
    /// 照度（lux）。Bevy 的 `DirectionalLight::illuminance` 直接对应。
    pub 照度: f32,
    /// 是否投射阴影。
    pub 投射阴影: bool,
}

impl Default for 方向光属性 {
    fn default() -> Self {
        Self {
            颜色: 颜色值::白(),
            // Bevy 默认 10000 lux 左右接近正午阳光；编辑器里取略亮一点，
            // 因为预览还叠着一层环境光。
            照度: 10_000.0,
            投射阴影: true,
        }
    }
}

/// 点光属性。
#[derive(Debug, Clone, PartialEq)]
pub struct 点光属性 {
    /// 光色。
    pub 颜色: 颜色值,
    /// 强度（流明，Bevy `PointLight::intensity`）。
    pub 强度: f32,
    /// 影响半径（米）。Bevy `PointLight::range`。
    pub 半径: f32,
    /// 是否投射阴影。
    pub 投射阴影: bool,
}

impl Default for 点光属性 {
    fn default() -> Self {
        Self {
            颜色: 颜色值::白(),
            强度: 800.0,
            半径: 20.0,
            投射阴影: true,
        }
    }
}

/// 聚光灯属性。
///
/// 角度以**度**存储与显示，投影到 Bevy 时才换算弧度：场景文件是给人
/// 读的，`30°` 比 `0.5236` 更容易核对。
#[derive(Debug, Clone, PartialEq)]
pub struct 聚光灯属性 {
    /// 光色。
    pub 颜色: 颜色值,
    /// 强度（流明，Bevy `SpotLight::intensity`）。
    pub 强度: f32,
    /// 影响半径（米）。Bevy `SpotLight::range`。
    pub 半径: f32,
    /// 内锥角（度）。光斑中心完全明亮的范围。
    pub 内锥角: f32,
    /// 外锥角（度）。超过该角度不再受光，必须大于内锥角且小于 90°。
    pub 外锥角: f32,
    /// 是否投射阴影。
    pub 投射阴影: bool,
}

impl Default for 聚光灯属性 {
    fn default() -> Self {
        Self {
            颜色: 颜色值::白(),
            强度: 4000.0,
            半径: 20.0,
            内锥角: 30.0,
            外锥角: 45.0,
            投射阴影: true,
        }
    }
}

/// 方向光组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 方向光组件类型;

impl 组件类型 for 方向光组件类型 {
    fn 组件名(&self) -> &'static str {
        方向光组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(方向光属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        方向光字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<方向光属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert("颜色".to_string(), 属性值::颜色(属性.颜色));
        结果.insert("照度".to_string(), 属性值::浮点(属性.照度));
        结果.insert("投射阴影".to_string(), 属性值::布尔(属性.投射阴影));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<方向光属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("颜色").and_then(属性值::取颜色) {
            属性.颜色 = 值;
        }
        if let Some(值) = 数据.get("照度").and_then(属性值::取浮点) {
            // 负照度没有物理意义；夹到非负，避免整片场景变黑却查不出原因。
            属性.照度 = 值.clamp(0.0, 200_000.0);
        }
        if let Some(值) = 数据.get("投射阴影").and_then(属性值::取布尔) {
            属性.投射阴影 = 值;
        }
        true
    }
}

/// 点光组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 点光组件类型;

impl 组件类型 for 点光组件类型 {
    fn 组件名(&self) -> &'static str {
        点光组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(点光属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        点光字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<点光属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert("颜色".to_string(), 属性值::颜色(属性.颜色));
        结果.insert("强度".to_string(), 属性值::浮点(属性.强度));
        结果.insert("半径".to_string(), 属性值::浮点(属性.半径));
        结果.insert("投射阴影".to_string(), 属性值::布尔(属性.投射阴影));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<点光属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("颜色").and_then(属性值::取颜色) {
            属性.颜色 = 值;
        }
        if let Some(值) = 数据.get("强度").and_then(属性值::取浮点) {
            属性.强度 = 值.clamp(0.0, 100_000.0);
        }
        if let Some(值) = 数据.get("半径").and_then(属性值::取浮点) {
            // 半径为 0 的灯等于不亮；给一个极小值让用户至少能看到"灯还在"。
            属性.半径 = 值.clamp(0.01, 1000.0);
        }
        if let Some(值) = 数据.get("投射阴影").and_then(属性值::取布尔) {
            属性.投射阴影 = 值;
        }
        true
    }
}

/// 聚光灯组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 聚光灯组件类型;

impl 组件类型 for 聚光灯组件类型 {
    fn 组件名(&self) -> &'static str {
        聚光灯组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(聚光灯属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        聚光灯字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<聚光灯属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert("颜色".to_string(), 属性值::颜色(属性.颜色));
        结果.insert("强度".to_string(), 属性值::浮点(属性.强度));
        结果.insert("半径".to_string(), 属性值::浮点(属性.半径));
        结果.insert("内锥角".to_string(), 属性值::浮点(属性.内锥角));
        结果.insert("外锥角".to_string(), 属性值::浮点(属性.外锥角));
        结果.insert("投射阴影".to_string(), 属性值::布尔(属性.投射阴影));
        Some(结果)
    }

    /// # 角度约束
    ///
    /// Bevy 要求 `outer_angle < PI/2` 且 `inner_angle <= outer_angle`，
    /// 违反时渲染器会 panic 或画出不可预期的光圈。这里先读外锥角、
    /// 再把内锥角夹到它以下：字段顺序由 `BTreeMap` 决定，不能指望
    /// 用户总是先改外角。
    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<聚光灯属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("颜色").and_then(属性值::取颜色) {
            属性.颜色 = 值;
        }
        if let Some(值) = 数据.get("强度").and_then(属性值::取浮点) {
            属性.强度 = 值.clamp(0.0, 100_000.0);
        }
        if let Some(值) = 数据.get("半径").and_then(属性值::取浮点) {
            属性.半径 = 值.clamp(0.01, 1000.0);
        }
        if let Some(值) = 数据.get("外锥角").and_then(属性值::取浮点) {
            属性.外锥角 = 值.clamp(0.1, 89.9);
        }
        if let Some(值) = 数据.get("内锥角").and_then(属性值::取浮点) {
            属性.内锥角 = 值.clamp(0.0, 属性.外锥角);
        }
        if let Some(值) = 数据.get("投射阴影").and_then(属性值::取布尔) {
            属性.投射阴影 = 值;
        }
        true
    }
}

fn 方向光字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "颜色",
                "light_field_color",
                属性种类::颜色,
                属性值::颜色(颜色值::白()),
            )
            .带提示("light_field_color_tooltip"),
            属性描述::新建(
                "照度",
                "light_field_illuminance",
                属性种类::浮点范围(0.0, 200_000.0, 100.0),
                属性值::浮点(10_000.0),
            )
            .带提示("light_field_illuminance_tooltip"),
            属性描述::新建(
                "投射阴影",
                "light_field_shadows",
                属性种类::布尔,
                属性值::布尔(true),
            )
            .带提示("light_field_shadows_tooltip"),
        ]
    })
}

fn 点光字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "颜色",
                "light_field_color",
                属性种类::颜色,
                属性值::颜色(颜色值::白()),
            )
            .带提示("light_field_color_tooltip"),
            属性描述::新建(
                "强度",
                "light_field_intensity",
                属性种类::浮点范围(0.0, 100_000.0, 50.0),
                属性值::浮点(800.0),
            )
            .带提示("light_field_intensity_tooltip"),
            属性描述::新建(
                "半径",
                "light_field_range",
                属性种类::浮点范围(0.01, 1000.0, 0.5),
                属性值::浮点(20.0),
            )
            .带提示("light_field_range_tooltip"),
            属性描述::新建(
                "投射阴影",
                "light_field_shadows",
                属性种类::布尔,
                属性值::布尔(true),
            )
            .带提示("light_field_shadows_tooltip"),
        ]
    })
}

fn 聚光灯字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "颜色",
                "light_field_color",
                属性种类::颜色,
                属性值::颜色(颜色值::白()),
            )
            .带提示("light_field_color_tooltip"),
            属性描述::新建(
                "强度",
                "light_field_intensity",
                属性种类::浮点范围(0.0, 100_000.0, 100.0),
                属性值::浮点(4000.0),
            )
            .带提示("light_field_intensity_tooltip"),
            属性描述::新建(
                "半径",
                "light_field_range",
                属性种类::浮点范围(0.01, 1000.0, 0.5),
                属性值::浮点(20.0),
            )
            .带提示("light_field_range_tooltip"),
            属性描述::新建(
                "内锥角",
                "light_field_inner_angle",
                属性种类::浮点范围(0.0, 89.9, 0.5),
                属性值::浮点(30.0),
            )
            .带提示("light_field_inner_angle_tooltip"),
            属性描述::新建(
                "外锥角",
                "light_field_outer_angle",
                属性种类::浮点范围(0.1, 89.9, 0.5),
                属性值::浮点(45.0),
            )
            .带提示("light_field_outer_angle_tooltip"),
            属性描述::新建(
                "投射阴影",
                "light_field_shadows",
                属性种类::布尔,
                属性值::布尔(true),
            )
            .带提示("light_field_shadows_tooltip"),
        ]
    })
}

/// 三维灯光组件名列表（供测试与界面提示）。
pub fn 灯光组件名列表() -> Vec<&'static str> {
    vec![方向光组件名, 点光组件名, 聚光灯组件名]
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 完整注册表包含灯光组件() {
        let 注册表 = crate::scene_bridge::全部内置组件注册表();
        for 名字 in 灯光组件名列表() {
            assert!(注册表.已注册(名字), "{名字} 应当被注册");
        }
    }

    #[test]
    fn 灯光组件默认值符合各自声明种类() {
        let 注册表 = crate::scene_bridge::全部内置组件注册表();
        for 名字 in 灯光组件名列表() {
            let 类型 = 注册表.要求(名字).expect("应当能查到");
            for 描述 in 类型.字段描述() {
                assert!(
                    描述.种类.接受(&描述.默认值),
                    "{名字}.{} 的默认值类型不对",
                    描述.字段名
                );
                assert!(!描述.显示名.is_empty(), "字段必须有显示名词条键");
            }
        }
    }

    #[test]
    fn 导出与实例化往返一致() {
        let 类型 = 方向光组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert("照度".into(), 属性值::浮点(2500.0));
        数据.insert("投射阴影".into(), 属性值::布尔(false));
        数据.insert(
            "颜色".into(),
            属性值::颜色(颜色值::srgb(1.0, 0.5, 0.25, 1.0)),
        );
        assert!(类型.实例化(&mut *实例, &数据));
        let 导出 = 类型.导出(&*实例).expect("应当能导出");
        assert_eq!(导出.get("照度"), Some(&属性值::浮点(2500.0)));
        assert_eq!(导出.get("投射阴影"), Some(&属性值::布尔(false)));
        match 导出.get("颜色") {
            Some(属性值::颜色(颜色)) => assert!((颜色.红 - 1.0).abs() < 1e-6),
            _ => panic!("颜色应当被导出"),
        }
    }

    #[test]
    fn 非法数值会被夹取而不是原样保留() {
        let 类型 = 点光组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert("强度".into(), 属性值::浮点(-5.0));
        数据.insert("半径".into(), 属性值::浮点(0.0));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        let 强度 = 导出.get("强度").and_then(属性值::取浮点).unwrap();
        let 半径 = 导出.get("半径").and_then(属性值::取浮点).unwrap();
        assert_eq!(强度, 0.0, "负强度必须夹到 0，否则场景会变黑");
        assert!(半径 >= 0.01, "半径为 0 的灯等于不亮");
    }

    #[test]
    fn 聚光灯导出实例化往返一致且夹角受约束() {
        let 类型 = 聚光灯组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert("强度".into(), 属性值::浮点(2500.0));
        数据.insert("内锥角".into(), 属性值::浮点(20.0));
        数据.insert("外锥角".into(), 属性值::浮点(60.0));
        数据.insert("投射阴影".into(), 属性值::布尔(false));
        assert!(类型.实例化(&mut *实例, &数据));
        let 导出 = 类型.导出(&*实例).expect("应当能导出");
        assert_eq!(导出.get("强度"), Some(&属性值::浮点(2500.0)));
        assert_eq!(导出.get("内锥角"), Some(&属性值::浮点(20.0)));
        assert_eq!(导出.get("外锥角"), Some(&属性值::浮点(60.0)));
        assert_eq!(导出.get("投射阴影"), Some(&属性值::布尔(false)));

        // 内角大于外角：必须被夹回外角，否则 Bevy 的光锥参数非法。
        let mut 越界 = BTreeMap::new();
        越界.insert("内锥角".into(), 属性值::浮点(80.0));
        越界.insert("外锥角".into(), 属性值::浮点(30.0));
        类型.实例化(&mut *实例, &越界);
        let 导出 = 类型.导出(&*实例).unwrap();
        let 内角 = 导出.get("内锥角").and_then(属性值::取浮点).unwrap();
        let 外角 = 导出.get("外锥角").and_then(属性值::取浮点).unwrap();
        assert!(内角 <= 外角, "内锥角必须夹到外锥角以下");
        assert!(外角 < 90.0);
    }

    #[test]
    fn 缺字段时保留原值() {
        let 类型 = 方向光组件类型;
        let mut 实例 = 类型.默认实例();
        let 原始 = 类型.导出(&*实例).unwrap();
        // 只给一个字段：其余必须保持默认，而不是被重置成别的值。
        let mut 数据 = BTreeMap::new();
        数据.insert("照度".into(), 属性值::浮点(1.0));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        assert_eq!(导出.get("投射阴影"), 原始.get("投射阴影"));
    }
}
