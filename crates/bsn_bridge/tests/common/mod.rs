//! 集成测试共用的测试夹具。
//!
//! 这里模拟"编辑器侧"的最小配置：两个受支持的组件 + 一个组件注册表 +
//! 一份提取配置。放在 `common` 里是为了让多个测试文件共用同一份定义，
//! 避免每个文件各写一遍 `组件类型` 实现而逐渐走形。
//!
//! **这些类型只存在于测试里**，不进入生产代码 —— 真实注册表在
//! `editor_app::scene_bridge`，由那里的集成测试覆盖。

#![allow(dead_code)]

use std::any::Any;
use std::collections::BTreeMap;

use bevy::prelude::*;

use bsn_bridge::{提取配置, 组件提取器};
use scene_doc::property::属性值;
use scene_doc::registry::{属性描述, 属性种类, 组件注册表, 组件类型};

/// 受支持组件之一：血条。字段用中文，验证中文字段名与 BSN 语法共存。
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
pub struct HealthBar {
    pub 当前: i64,
    pub 最大: i64,
}

/// 受支持组件之二：护甲。带非零默认值，用于校验"未提到的字段取默认"。
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Armor {
    pub 值: i64,
    pub 等级: i64,
}

impl Default for Armor {
    fn default() -> Self {
        Self { 值: 10, 等级: 3 }
    }
}

/// 完全没有提取器的组件，用于验证"未转换组件必须导致严格转换失败"。
#[derive(Component, Default, Clone, Debug)]
pub struct Unregistered {
    pub 载荷: i64,
}

/// 文档侧的可见性表示。
///
/// 模拟 `editor_app::scene_bridge::可见性属性`：把 Bevy 的三态 `Visibility`
/// 简化成一个布尔。它**不是** `Component` —— 它只活在文档适配器里，
/// 这正是"世界类型 ≠ 文档类型"的典型场景。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct 可见性属性 {
    pub 可见: bool,
}

impl Default for 可见性属性 {
    fn default() -> Self {
        Self { 可见: true }
    }
}

/// 世界侧 `Visibility` → 文档侧 `可见性属性`。
///
/// 与 `editor_app::scene_bridge` 的映射保持一致：
/// 只有 `Visibility::Hidden` 算不可见，`Inherited` / `Visible` 都算可见。
pub fn 转换可见性(可见性: &Visibility) -> 可见性属性 {
    可见性属性 {
        可见: *可见性 != Visibility::Hidden,
    }
}

// ----------------------------------------------------------------------
// 编辑器侧的组件适配（组件类型）
// ----------------------------------------------------------------------

fn 整数(字段: &str, 默认: i64) -> 属性描述 {
    属性描述::新建(字段, "属性", 属性种类::自由整数(), 属性值::整数(默认))
}

fn 血条字段() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| vec![整数("当前", 0), 整数("最大", 0)])
}

fn 护甲字段() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| vec![整数("值", 10), 整数("等级", 3)])
}

fn 导出整数<T>(
    实例: &dyn Any,
    读取: impl Fn(&T) -> Vec<(&'static str, i64)>,
) -> Option<BTreeMap<String, 属性值>>
where
    T: 'static,
{
    let 数据 = 实例.downcast_ref::<T>()?;
    let mut 结果 = BTreeMap::new();
    for (名字, 值) in 读取(数据) {
        结果.insert(名字.to_string(), 属性值::整数(值));
    }
    Some(结果)
}

fn 写入整数<T>(
    实例: &mut dyn Any,
    数据: &BTreeMap<String, 属性值>,
    设置: impl Fn(&mut T, &str, i64),
) -> bool
where
    T: 'static,
{
    let Some(目标) = 实例.downcast_mut::<T>() else {
        return false;
    };
    for (名字, 值) in 数据 {
        if let Some(数字) = 值.取整数() {
            设置(目标, 名字, 数字);
        }
    }
    true
}

/// 血条的组件类型。
pub struct 血条类型;

impl 组件类型 for 血条类型 {
    fn 组件名(&self) -> &'static str {
        "HealthBar"
    }
    fn 默认实例(&self) -> Box<dyn Any> {
        Box::new(HealthBar::default())
    }
    fn 字段描述(&self) -> &[属性描述] {
        血条字段()
    }
    fn 导出(&self, 实例: &dyn Any) -> Option<BTreeMap<String, 属性值>> {
        导出整数::<HealthBar>(实例, |数据| {
            vec![("当前", 数据.当前), ("最大", 数据.最大)]
        })
    }
    fn 实例化(&self, 实例: &mut dyn Any, 数据: &BTreeMap<String, 属性值>) -> bool {
        写入整数::<HealthBar>(实例, 数据, |目标, 名字, 值| match 名字 {
            "当前" => 目标.当前 = 值,
            "最大" => 目标.最大 = 值,
            _ => {}
        })
    }
}

/// 护甲的组件类型。
pub struct 护甲类型;

impl 组件类型 for 护甲类型 {
    fn 组件名(&self) -> &'static str {
        "Armor"
    }
    fn 默认实例(&self) -> Box<dyn Any> {
        Box::new(Armor::default())
    }
    fn 字段描述(&self) -> &[属性描述] {
        护甲字段()
    }
    fn 导出(&self, 实例: &dyn Any) -> Option<BTreeMap<String, 属性值>> {
        导出整数::<Armor>(实例, |数据| vec![("值", 数据.值), ("等级", 数据.等级)])
    }
    fn 实例化(&self, 实例: &mut dyn Any, 数据: &BTreeMap<String, 属性值>) -> bool {
        写入整数::<Armor>(实例, 数据, |目标, 名字, 值| match 名字 {
            "值" => 目标.值 = 值,
            "等级" => 目标.等级 = 值,
            _ => {}
        })
    }
}

/// 可见性的组件类型适配器（文档侧类型是 [`可见性属性`]，不是 `Visibility`）。
pub struct 可见性类型;

fn 可见性字段() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![属性描述::新建(
            "可见",
            "可见",
            属性种类::布尔,
            属性值::布尔(true),
        )]
    })
}

impl 组件类型 for 可见性类型 {
    fn 组件名(&self) -> &'static str {
        "Visibility"
    }
    fn 默认实例(&self) -> Box<dyn Any> {
        Box::new(可见性属性::default())
    }
    fn 字段描述(&self) -> &[属性描述] {
        可见性字段()
    }
    fn 导出(&self, 实例: &dyn Any) -> Option<BTreeMap<String, 属性值>> {
        let 数据 = 实例.downcast_ref::<可见性属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert("可见".to_string(), 属性值::布尔(数据.可见));
        Some(结果)
    }
    fn 实例化(&self, 实例: &mut dyn Any, 数据: &BTreeMap<String, 属性值>) -> bool {
        let Some(目标) = 实例.downcast_mut::<可见性属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("可见").and_then(属性值::取布尔) {
            目标.可见 = 值;
        }
        true
    }
}

/// 测试用组件注册表。
pub fn 测试注册表() -> 组件注册表 {
    let mut 注册表 = 组件注册表::新建();
    注册表.注册(Box::new(血条类型)).expect("注册血条");
    注册表.注册(Box::new(护甲类型)).expect("注册护甲");
    注册表.注册(Box::new(可见性类型)).expect("注册可见性");
    注册表
}

/// 测试用提取配置。
///
/// `Visibility` 用 [`组件提取器::带转换`]：世界侧是 Bevy 的枚举，
/// 文档侧是简化的布尔包装，两者类型不同。
pub fn 测试提取配置(注册表: &组件注册表) -> 提取配置<'_> {
    提取配置::新建(
        注册表,
        vec![
            组件提取器::新建::<HealthBar>("HealthBar"),
            组件提取器::新建::<Armor>("Armor"),
            组件提取器::带转换::<Visibility, 可见性属性>("Visibility", 转换可见性),
        ],
    )
}
