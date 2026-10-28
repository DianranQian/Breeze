//! 粒子发射器组件：把「这里该冒什么烟」变成可编辑的场景数据。
//!
//! # 为什么没有引入 `bevy_hanabi`
//!
//! `bevy_hanabi` 的 **0.19.0**（crates.io 最新发布版，2026-06-27）依赖
//! `bevy ^0.19`，与本仓库的 Bevy 0.20 不兼容；它的 git 主分支虽然已经
//! 升到 0.20，但没有发布版本，跟着主分支走等于把编译稳定性押在
//! 上游每次提交上。按「开源优先」的次序记录冲突之后，这里选择
//! **极简自定义粒子系统**：数据只存编辑意图，模拟在
//! [`crate::particle_runtime`] 里用确定性的 CPU 步进完成。
//!
//! # 只存编辑意图
//!
//! 场景文件里没有"某一帧的粒子位置"：那是运行时求值出来的中间状态，
//! 和动画的中间帧一样不该落盘。组件只回答四个问题：
//! 每秒冒多少个、每个活多久、初速度多大、长什么样。
//!
//! # 坐标与方向约定
//!
//! 粒子从实体**自身原点**发射，主方向是实体的本地 +Y（与 Godot 的
//! `GPUParticles3D` 默认向 +Y 一致），并带一个随机锥形散开。
//! 因此旋转父节点就能改变喷射方向，不需要额外的方向字段。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bevy::prelude::Component;
use scene_doc::property::{属性值, 颜色值};
use scene_doc::registry::{属性描述, 属性种类, 组件类型};

/// 粒子发射器组件的类型名。
pub const 粒子发射器组件名: &str = "粒子发射器";

/// 单个发射器的粒子数量上限。
///
/// 上限保护的是编辑器帧时间：预览与运行时都是 CPU 步进，一个手滑把
/// 发射速率填成 500、生命周期填成 20 秒，就会稳定维持一万个粒子。
/// 超出上限时停止发射而不是淘汰旧粒子 —— 旧粒子按寿命自然消亡，
/// 画面不会突然"断流"。
pub const 最大粒子数: usize = 512;

/// 发射速率的绝对值上限（个/秒）。
pub const 最大发射速率: f32 = 200.0;

/// 粒子生命周期的上限（秒）。
pub const 最大生命周期: f32 = 20.0;

/// 初始速度的上限（世界单位/秒）。
pub const 最大初始速度: f32 = 30.0;

/// 粒子直径的上限（世界单位）。
pub const 最大粒子大小: f32 = 10.0;

/// 粒子发射器的文档属性。
#[derive(Component, Debug, Clone, PartialEq)]
pub struct 粒子发射器属性 {
    /// 发射速率（个/秒）。
    pub 发射速率: f32,
    /// 单个粒子的生命周期（秒）。
    pub 生命周期: f32,
    /// 初始速度（世界单位/秒），沿本地 +Y 的锥形方向。
    pub 初始速度: f32,
    /// 粒子颜色（透明度会直接进材质的混合模式）。
    pub 颜色: 颜色值,
    /// 粒子直径（世界单位）。
    pub 粒子大小: f32,
}

impl Default for 粒子发射器属性 {
    fn default() -> Self {
        Self {
            发射速率: 20.0,
            生命周期: 1.5,
            初始速度: 3.0,
            颜色: 颜色值::srgb(1.0, 0.62, 0.20, 1.0),
            粒子大小: 0.20,
        }
    }
}

/// 字段名：发射速率。
pub const 发射速率字段: &str = "发射速率";
/// 字段名：粒子生命周期。
pub const 生命周期字段: &str = "生命周期";
/// 字段名：初始速度。
pub const 初始速度字段: &str = "初始速度";
/// 字段名：粒子颜色。
pub const 颜色字段: &str = "颜色";
/// 字段名：粒子大小。
pub const 粒子大小字段: &str = "粒子大小";

/// 从属性表解析粒子发射器参数。
///
/// # 容错规则
///
/// 1. 四个浮点字段必须是有限数并夹在各自合法区间内；非有限值或类型
///    不匹配时回落到默认值（保存文件被外部改成 `nan` 也不会炸预览）；
/// 2. 颜色缺失时用默认暖橙色 —— 它比纯白更像"火/烟"，也更容易看出
///    材质是否真的应用了颜色；
/// 3. 所有回落口径与 [`粒子发射器组件类型`] 的 `实例化` 完全一致，
///    避免"Inspector 里看到的值"和"运行时用的值"分叉。
pub fn 解析粒子发射器(属性: &BTreeMap<String, 属性值>) -> 粒子发射器属性 {
    let 默认 = 粒子发射器属性::default();

    粒子发射器属性 {
        发射速率: 属性
            .get(发射速率字段)
            .and_then(属性值::取浮点)
            .filter(|值| 值.is_finite())
            .map(合法发射速率)
            .unwrap_or(默认.发射速率),
        生命周期: 属性
            .get(生命周期字段)
            .and_then(属性值::取浮点)
            .filter(|值| 值.is_finite())
            .map(合法生命周期)
            .unwrap_or(默认.生命周期),
        初始速度: 属性
            .get(初始速度字段)
            .and_then(属性值::取浮点)
            .filter(|值| 值.is_finite())
            .map(合法初始速度)
            .unwrap_or(默认.初始速度),
        颜色: 属性
            .get(颜色字段)
            .and_then(属性值::取颜色)
            .unwrap_or(默认.颜色),
        粒子大小: 属性
            .get(粒子大小字段)
            .and_then(属性值::取浮点)
            .filter(|值| 值.is_finite())
            .map(合法粒子大小)
            .unwrap_or(默认.粒子大小),
    }
}

/// 把发射速率夹在合法区间内；非有限值回落到默认值。
pub fn 合法发射速率(速率: f32) -> f32 {
    if 速率.is_finite() {
        速率.clamp(0.0, 最大发射速率)
    } else {
        粒子发射器属性::default().发射速率
    }
}

/// 把生命周期夹在合法区间内；非有限值回落到默认值。
///
/// 下限取 `0.1` 而不是 `0.0`：寿命为 0 的粒子在同一帧生成又消亡，
/// 用户只会看到"发射速率填了却没效果"。
pub fn 合法生命周期(寿命: f32) -> f32 {
    if 寿命.is_finite() {
        寿命.clamp(0.1, 最大生命周期)
    } else {
        粒子发射器属性::default().生命周期
    }
}

/// 把初始速度夹在合法区间内；非有限值回落到默认值。
pub fn 合法初始速度(速度: f32) -> f32 {
    if 速度.is_finite() {
        速度.clamp(0.0, 最大初始速度)
    } else {
        粒子发射器属性::default().初始速度
    }
}

/// 把粒子直径夹在合法区间内；非有限值回落到默认值。
///
/// 下限取 `0.01`：0 尺寸的网格在 Bevy 里会退化成不可见，
/// 与"删掉发射器"没有区别，但用户会以为是渲染坏了。
pub fn 合法粒子大小(大小: f32) -> f32 {
    if 大小.is_finite() {
        大小.clamp(0.01, 最大粒子大小)
    } else {
        粒子发射器属性::default().粒子大小
    }
}

/// 粒子发射器组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 粒子发射器组件类型;

impl 组件类型 for 粒子发射器组件类型 {
    fn 组件名(&self) -> &'static str {
        粒子发射器组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(粒子发射器属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        粒子发射器字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<粒子发射器属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert(发射速率字段.to_string(), 属性值::浮点(属性.发射速率));
        结果.insert(生命周期字段.to_string(), 属性值::浮点(属性.生命周期));
        结果.insert(初始速度字段.to_string(), 属性值::浮点(属性.初始速度));
        结果.insert(颜色字段.to_string(), 属性值::颜色(属性.颜色));
        结果.insert(粒子大小字段.to_string(), 属性值::浮点(属性.粒子大小));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<粒子发射器属性>() else {
            return false;
        };
        if let Some(值) = 数据.get(发射速率字段).and_then(属性值::取浮点)
            && 值.is_finite()
        {
            属性.发射速率 = 合法发射速率(值);
        }
        if let Some(值) = 数据.get(生命周期字段).and_then(属性值::取浮点)
            && 值.is_finite()
        {
            属性.生命周期 = 合法生命周期(值);
        }
        if let Some(值) = 数据.get(初始速度字段).and_then(属性值::取浮点)
            && 值.is_finite()
        {
            属性.初始速度 = 合法初始速度(值);
        }
        if let Some(值) = 数据.get(颜色字段).and_then(属性值::取颜色) {
            属性.颜色 = 值;
        }
        if let Some(值) = 数据.get(粒子大小字段).and_then(属性值::取浮点)
            && 值.is_finite()
        {
            属性.粒子大小 = 合法粒子大小(值);
        }
        true
    }
}

/// 粒子发射器字段描述。
fn 粒子发射器字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                发射速率字段,
                "fx_field_rate",
                属性种类::浮点范围(0.0, 最大发射速率, 0.5),
                属性值::浮点(20.0),
            )
            .带提示("fx_field_rate_tooltip"),
            属性描述::新建(
                生命周期字段,
                "fx_field_lifetime",
                属性种类::浮点范围(0.1, 最大生命周期, 0.1),
                属性值::浮点(1.5),
            )
            .带提示("fx_field_lifetime_tooltip"),
            属性描述::新建(
                初始速度字段,
                "fx_field_speed",
                属性种类::浮点范围(0.0, 最大初始速度, 0.1),
                属性值::浮点(3.0),
            )
            .带提示("fx_field_speed_tooltip"),
            属性描述::新建(
                颜色字段,
                "fx_field_color",
                属性种类::颜色,
                属性值::颜色(颜色值::srgb(1.0, 0.62, 0.20, 1.0)),
            )
            .带提示("fx_field_color_tooltip"),
            属性描述::新建(
                粒子大小字段,
                "fx_field_size",
                属性种类::浮点范围(0.01, 最大粒子大小, 0.01),
                属性值::浮点(0.20),
            )
            .带提示("fx_field_size_tooltip"),
        ]
    })
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 字段导出实例化往返() {
        let 类型 = 粒子发射器组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(发射速率字段.to_string(), 属性值::浮点(45.0));
        数据.insert(生命周期字段.to_string(), 属性值::浮点(2.5));
        数据.insert(初始速度字段.to_string(), 属性值::浮点(7.0));
        数据.insert(
            颜色字段.to_string(),
            属性值::颜色(颜色值::srgb(0.2, 0.4, 1.0, 0.5)),
        );
        数据.insert(粒子大小字段.to_string(), 属性值::浮点(0.35));
        assert!(类型.实例化(实例.as_mut(), &数据));

        let 属性 = 实例.downcast_ref::<粒子发射器属性>().unwrap();
        assert_eq!(属性.发射速率, 45.0);
        assert_eq!(属性.生命周期, 2.5);
        assert_eq!(属性.初始速度, 7.0);
        assert_eq!(属性.粒子大小, 0.35);
        assert_eq!(属性.颜色, 颜色值::srgb(0.2, 0.4, 1.0, 0.5));

        let 导出 = 类型.导出(实例.as_ref()).unwrap();
        assert_eq!(导出.get(发射速率字段).and_then(属性值::取浮点), Some(45.0));
        assert_eq!(
            导出.get(颜色字段).and_then(属性值::取颜色),
            Some(颜色值::srgb(0.2, 0.4, 1.0, 0.5))
        );
    }

    #[test]
    fn 非法字段保留原值() {
        let 类型 = 粒子发射器组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(发射速率字段.to_string(), 属性值::浮点(f32::NAN));
        数据.insert(生命周期字段.to_string(), 属性值::浮点(f32::INFINITY));
        数据.insert(颜色字段.to_string(), 属性值::文本("不是颜色".to_string()));
        assert!(类型.实例化(实例.as_mut(), &数据));

        let 属性 = 实例.downcast_ref::<粒子发射器属性>().unwrap();
        let 默认 = 粒子发射器属性::default();
        assert_eq!(属性.发射速率, 默认.发射速率, "非有限速率不能写进组件");
        assert_eq!(属性.生命周期, 默认.生命周期);
        assert_eq!(属性.颜色, 默认.颜色, "类型不匹配时保留原值");
    }

    #[test]
    fn 数值被夹在合法范围() {
        assert_eq!(合法发射速率(10_000.0), 最大发射速率);
        assert_eq!(合法发射速率(-1.0), 0.0);
        assert_eq!(合法生命周期(0.0), 0.1);
        assert_eq!(合法生命周期(1_000.0), 最大生命周期);
        assert_eq!(合法初始速度(100.0), 最大初始速度);
        assert_eq!(合法粒子大小(0.0), 0.01);
        assert_eq!(合法粒子大小(50.0), 最大粒子大小);
    }

    #[test]
    fn 解析属性与实例化口径一致() {
        let mut 数据 = BTreeMap::new();
        数据.insert(发射速率字段.to_string(), 属性值::浮点(999.0));
        数据.insert(生命周期字段.to_string(), 属性值::浮点(0.0));
        let 解析 = 解析粒子发射器(&数据);
        assert_eq!(解析.发射速率, 最大发射速率);
        assert_eq!(解析.生命周期, 0.1);
        assert_eq!(解析.初始速度, 粒子发射器属性::default().初始速度);

        let 类型 = 粒子发射器组件类型;
        let mut 实例 = 类型.默认实例();
        assert!(类型.实例化(实例.as_mut(), &数据));
        let 实例化后 = 实例.downcast_ref::<粒子发射器属性>().unwrap();
        assert_eq!(解析, *实例化后, "两条入口必须给出同一份参数");
    }

    #[test]
    fn 完整注册表包含粒子发射器() {
        let 注册表 = crate::scene_bridge::全部内置组件注册表();
        assert!(
            注册表.已注册(粒子发射器组件名),
            "{粒子发射器组件名} 必须出现在内置组件注册表里"
        );
    }
}
