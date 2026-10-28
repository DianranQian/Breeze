//! glTF 动画播放组件：把「这个模型要播哪一段」变成可编辑的场景数据。
//!
//! # 为什么不用 Bevy 的 `AnimationPlayer` 当文档组件
//!
//! 场景文档只保存**编辑意图**：片段名、速度、是否循环。Bevy 的
//! `AnimationPlayer` 持有活动动画的运行时状态与资源句柄，每帧都在变，
//! 既不能序列化也不能参与撤销。两者的生命周期完全不同，因此文档里
//! 用独立的数据组件，运行时再翻译成真实播放器。
//!
//! 组件名取中文「动画播放器」而不是 `AnimationPlayer`，是为了在代码与
//! 场景文件里都能一眼区分「编辑意图」与「Bevy 运行时状态」；
//! 桥接逻辑集中在 [`crate::animation_runtime`]。
//!
//! # 片段名从哪来
//!
//! 片段名对应 glTF 文件里的 `animations[].name`（导出时通常是
//! `Idle` / `Run` / `Walk`）。名字不写进模型路径，是因为同一个模型
//! 挂在不同实体上可以播不同片段；片段不存在时模型仍按静态姿势显示，
//! 只在预览里告警一次（见 `animation_runtime::装配模型动画`）。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bevy::prelude::Component;
use scene_doc::property::属性值;
use scene_doc::registry::{属性描述, 属性种类, 组件类型};

/// 动画播放器组件的类型名。
pub const 动画播放器组件名: &str = "动画播放器";

/// 播放速度的绝对值上限。
///
/// 允许负数（倒放），但不允许离谱的倍率：100 倍速在 60 帧下每帧跳过
/// 大半个片段，视觉上与"乱跳"没有区别，也容易掩盖真正的配置错误。
pub const 最大播放速度: f32 = 10.0;

/// 动画播放器的文档属性。
#[derive(Component, Debug, Clone, PartialEq)]
pub struct 动画播放器属性 {
    /// glTF 动画片段名（`animations[].name`）。
    pub 片段: String,
    /// 播放速度，1.0 为原速；负数倒放。
    pub 播放速度: f32,
    /// 是否循环播放。
    pub 循环: bool,
}

impl Default for 动画播放器属性 {
    fn default() -> Self {
        Self {
            片段: "Idle".to_string(),
            播放速度: 1.0,
            循环: true,
        }
    }
}

/// 字段名：动画片段。
pub const 片段字段: &str = "片段";
/// 字段名：播放速度。
pub const 速度字段: &str = "播放速度";
/// 字段名：是否循环。
pub const 循环字段: &str = "循环";

/// 从属性表解析动画播放器参数。
///
/// # 容错规则
///
/// 1. 片段名去首尾空白；空串视为未配置，回落到 `Idle`；
/// 2. 速度必须是有限数，并夹在 `±最大播放速度` 之间；
/// 3. 循环缺省为 `true` —— 走路、待机这类循环动画占绝大多数，
///    缺省不循环会让用户在 Inspector 里反复手动打勾。
pub fn 解析动画播放器(属性: &BTreeMap<String, 属性值>) -> 动画播放器属性 {
    let 默认 = 动画播放器属性::default();

    let 片段 = 属性
        .get(片段字段)
        .and_then(属性值::取文本)
        .map(str::trim)
        .filter(|文本| !文本.is_empty())
        .map(str::to_string)
        .unwrap_or(默认.片段);

    let 播放速度 = 属性
        .get(速度字段)
        .and_then(属性值::取浮点)
        .filter(|值| 值.is_finite())
        .map(合法速度)
        .unwrap_or(默认.播放速度);

    let 循环 = 属性
        .get(循环字段)
        .and_then(属性值::取布尔)
        .unwrap_or(默认.循环);

    动画播放器属性 {
        片段,
        播放速度,
        循环,
    }
}

/// 把速度夹在合法区间内；非有限值回落到原速。
pub fn 合法速度(速度: f32) -> f32 {
    if 速度.is_finite() {
        速度.clamp(-最大播放速度, 最大播放速度)
    } else {
        1.0
    }
}

/// 动画播放器组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 动画播放器组件类型;

impl 组件类型 for 动画播放器组件类型 {
    fn 组件名(&self) -> &'static str {
        动画播放器组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(动画播放器属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        动画播放器字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<动画播放器属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert(片段字段.to_string(), 属性值::文本(属性.片段.clone()));
        结果.insert(速度字段.to_string(), 属性值::浮点(属性.播放速度));
        结果.insert(循环字段.to_string(), 属性值::布尔(属性.循环));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<动画播放器属性>() else {
            return false;
        };
        if let Some(值) = 数据.get(片段字段).and_then(属性值::取文本) {
            let 片段 = 值.trim();
            if !片段.is_empty() {
                属性.片段 = 片段.to_string();
            }
        }
        if let Some(值) = 数据.get(速度字段).and_then(属性值::取浮点)
            && 值.is_finite()
        {
            属性.播放速度 = 合法速度(值);
        }
        if let Some(值) = 数据.get(循环字段).and_then(属性值::取布尔) {
            属性.循环 = 值;
        }
        true
    }
}

/// 动画播放器字段描述。
fn 动画播放器字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                片段字段,
                "anim_field_clip",
                属性种类::文本,
                属性值::文本("Idle".to_string()),
            )
            .带提示("anim_field_clip_tooltip"),
            属性描述::新建(
                速度字段,
                "anim_field_speed",
                属性种类::浮点范围(-最大播放速度, 最大播放速度, 0.1),
                属性值::浮点(1.0),
            )
            .带提示("anim_field_speed_tooltip"),
            属性描述::新建(
                循环字段,
                "anim_field_loop",
                属性种类::布尔,
                属性值::布尔(true),
            )
            .带提示("anim_field_loop_tooltip"),
        ]
    })
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 字段导出实例化往返() {
        let 类型 = 动画播放器组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(片段字段.to_string(), 属性值::文本(" Run ".to_string()));
        数据.insert(速度字段.to_string(), 属性值::浮点(2.5));
        数据.insert(循环字段.to_string(), 属性值::布尔(false));
        assert!(类型.实例化(实例.as_mut(), &数据));

        let 属性 = 实例.downcast_ref::<动画播放器属性>().unwrap();
        assert_eq!(属性.片段, "Run", "片段名应当去掉首尾空白");
        assert_eq!(属性.播放速度, 2.5);
        assert!(!属性.循环);

        let 导出 = 类型.导出(实例.as_ref()).unwrap();
        assert_eq!(导出.get(片段字段).and_then(属性值::取文本), Some("Run"));
        assert_eq!(导出.get(速度字段).and_then(属性值::取浮点), Some(2.5));
        assert_eq!(导出.get(循环字段).and_then(属性值::取布尔), Some(false));
    }

    #[test]
    fn 非法字段保留原值() {
        let 类型 = 动画播放器组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(片段字段.to_string(), 属性值::文本("   ".to_string()));
        数据.insert(速度字段.to_string(), 属性值::浮点(f32::NAN));
        数据.insert(循环字段.to_string(), 属性值::文本("不是布尔".to_string()));
        assert!(类型.实例化(实例.as_mut(), &数据));

        let 属性 = 实例.downcast_ref::<动画播放器属性>().unwrap();
        assert_eq!(属性.片段, "Idle", "空白片段名不能覆盖已有值");
        assert_eq!(属性.播放速度, 1.0, "非有限速度不能写进组件");
        assert!(属性.循环, "类型不匹配时保留原值");
    }

    #[test]
    fn 速度被夹在合法范围() {
        assert_eq!(合法速度(100.0), 最大播放速度);
        assert_eq!(合法速度(-100.0), -最大播放速度);
        assert_eq!(合法速度(-1.5), -1.5, "负数是合法的倒放");
        assert_eq!(合法速度(f32::INFINITY), 1.0);
    }

    #[test]
    fn 解析属性与实例化口径一致() {
        let mut 数据 = BTreeMap::new();
        数据.insert(片段字段.to_string(), 属性值::文本("Walk".to_string()));
        数据.insert(速度字段.to_string(), 属性值::浮点(30.0));
        let 解析 = 解析动画播放器(&数据);
        assert_eq!(解析.片段, "Walk");
        assert_eq!(解析.播放速度, 最大播放速度);
        assert!(解析.循环, "缺省循环");
    }

    #[test]
    fn 完整注册表包含动画播放器() {
        let 注册表 = crate::scene_bridge::全部内置组件注册表();
        assert!(
            注册表.已注册(动画播放器组件名),
            "{动画播放器组件名} 必须出现在内置组件注册表里"
        );
    }
}
