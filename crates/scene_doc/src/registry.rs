//! 组件注册表：把"这个组件有哪些可编辑字段"变成**数据**而不是硬编码的 UI 代码。
//!
//! # 这是本项目最关键的可扩展性设计
//!
//! 如果 Inspector 里写的是
//! `if 组件名 == "Sprite" { 画一个颜色选择器，改 sprite.color }`，
//! 那么编辑器每支持一个新组件都要改 UI 代码，第三方无法扩展，
//! 而且"哪些组件可编辑"这件事会散落在几十个 `match` 分支里。
//!
//! 这里把它倒过来：[`组件描述`] 声明字段与类型，[`组件类型`] 负责
//! `属性值 <-> Rust 值` 的双向转换，Inspector 只消费描述来生成 UI。
//! 于是：
//!
//! * 支持一个新组件 = 注册一条描述 + 一个转换实现，**不改任何 UI 代码**；
//! * 描述本身可校验（默认值类型必须与声明类型一致），错误在注册期暴露；
//! * 插件系统将来要扩展组件，只需往注册表里加条目。

use std::collections::BTreeMap;
use std::fmt;

use crate::property::{属性值, 资源类别};

/// 属性的编辑语义。
///
/// 它决定 Inspector 生成哪种控件，以及数值的软性范围。
/// 注意：这只是**提示**，不是强制约束 —— 注册表不阻止把 `位置` 声明成角度。
#[derive(Debug, Clone, PartialEq)]
pub enum 属性种类 {
    /// 布尔开关。
    布尔,
    /// 整数输入框。
    整数 {
        /// 建议最小值。
        最小: Option<i64>,
        /// 建议最大值。
        最大: Option<i64>,
    },
    /// 浮点输入框。
    浮点 {
        /// 建议最小值。
        最小: Option<f32>,
        /// 建议最大值。
        最大: Option<f32>,
        /// 拖动时的步进量。
        步进: f32,
    },
    /// 单行文本。
    文本,
    /// 二维向量。
    二维 {
        /// 拖动时的步进量。
        步进: f32,
    },
    /// 三维向量。
    三维 {
        /// 拖动时的步进量。
        步进: f32,
    },
    /// 以欧拉角（度）编辑的三维向量。
    角度三维,
    /// 四维向量。
    四维 {
        /// 拖动时的步进量。
        步进: f32,
    },
    /// 颜色选择器。
    颜色,
    /// 下拉枚举。`选项` 是 `(显示名, 变体名, 序号)`。
    枚举 {
        /// 可选项列表。
        选项: Vec<(String, String, u32)>,
    },
    /// 资源选择器。
    资源 {
        /// 期望的资源类别，决定弹出的选择器类型。
        类别: 资源类别,
    },
    /// 同构列表。
    列表 {
        /// 每个元素的种类。
        元素: Box<属性种类>,
    },
    /// 子结构。
    结构 {
        /// 子字段列表。
        字段: Vec<属性描述>,
    },
}

impl 属性种类 {
    /// 浮点字段的简写构造。
    pub fn 浮点范围(最小: f32, 最大: f32, 步进: f32) -> Self {
        属性种类::浮点 {
            最小: Some(最小),
            最大: Some(最大),
            步进,
        }
    }

    /// 无范围限制的浮点。
    pub fn 自由浮点() -> Self {
        属性种类::浮点 {
            最小: None,
            最大: None,
            步进: 0.01,
        }
    }

    /// 无范围的整数。
    pub fn 自由整数() -> Self {
        属性种类::整数 {
            最小: None,
            最大: None,
        }
    }

    /// 默认步进的二维向量。
    pub fn 二维默认() -> Self {
        属性种类::二维 { 步进: 0.01 }
    }

    /// 默认步进的三维向量。
    pub fn 三维默认() -> Self {
        属性种类::三维 { 步进: 0.01 }
    }

    /// 判断一个属性值是否符合本种类。
    ///
    /// 校验是**容忍整数/浮点互换**的，因为手写场景文件里 `1` 与 `1.0`
    /// 都应该被接受；但不会容忍把字符串塞进向量字段。
    pub fn 接受(&self, 值: &属性值) -> bool {
        match (self, 值) {
            (属性种类::布尔, 属性值::布尔(_)) => true,
            (属性种类::整数 { .. }, 属性值::整数(_)) => true,
            (属性种类::整数 { .. }, 属性值::浮点(浮点)) => 浮点.fract() == 0.0,
            (属性种类::浮点 { .. }, 属性值::浮点(_) | 属性值::整数(_)) => true,
            (属性种类::文本, 属性值::文本(_)) => true,
            (属性种类::二维 { .. }, 属性值::二维(..)) => true,
            (属性种类::三维 { .. } | 属性种类::角度三维, 属性值::三维(..)) => true,
            (属性种类::四维 { .. }, 属性值::四维(..)) => true,
            (属性种类::颜色, 属性值::颜色(_)) => true,
            (属性种类::枚举 { 选项 }, 属性值::枚举(枚举)) => 选项
                .iter()
                .any(|(_, 变体名, 序号)| 变体名 == &枚举.变体名 || *序号 == 枚举.序号),
            (属性种类::资源 { .. }, 属性值::资源(_)) => true,
            (属性种类::列表 { 元素 }, 属性值::列表(列表)) => {
                列表.iter().all(|项| 元素.接受(项))
            }
            (属性种类::结构 { 字段 }, 属性值::结构(结构)) => 结构.iter().all(|(名字, 项)| {
                字段
                    .iter()
                    .find(|描述| &描述.字段名 == 名字)
                    .is_some_and(|描述| 描述.种类.接受(项))
            }),
            // 空值对所有非布尔的种类都表示"未设置"，一律接受。
            (属性种类::布尔, 属性值::空) => false,
            (_, 属性值::空) => true,
            _ => false,
        }
    }

    /// 供 UI 显示的简要类型名。
    pub fn 类型名(&self) -> &'static str {
        match self {
            属性种类::布尔 => "bool",
            属性种类::整数 { .. } => "i64",
            属性种类::浮点 { .. } => "f32",
            属性种类::文本 => "String",
            属性种类::二维 { .. } => "Vec2",
            属性种类::三维 { .. } => "Vec3",
            属性种类::角度三维 => "Euler(度)",
            属性种类::四维 { .. } => "Vec4",
            属性种类::颜色 => "Color",
            属性种类::枚举 { .. } => "enum",
            属性种类::资源 { .. } => "Handle",
            属性种类::列表 { .. } => "Vec",
            属性种类::结构 { .. } => "struct",
        }
    }
}

/// 单个属性的描述。
#[derive(Debug, Clone, PartialEq)]
pub struct 属性描述 {
    /// 字段名。与 Rust 结构体的字段标识符保持一致，作为序列化键。
    pub 字段名: String,
    /// 在 Inspector 里显示的名字（中文）。
    pub 显示名: String,
    /// 编辑语义。
    pub 种类: 属性种类,
    /// 默认值。注册时校验必须符合 `种类`。
    pub 默认值: 属性值,
    /// 鼠标悬停提示。
    pub 提示: String,
    /// 是否只读。只读字段在 Inspector 里显示但不可编辑。
    pub 只读: bool,
}

impl 属性描述 {
    /// 创建一个可编辑属性描述。
    pub fn 新建(
        字段名: impl Into<String>,
        显示名: impl Into<String>,
        种类: 属性种类,
        默认值: 属性值,
    ) -> Self {
        Self {
            字段名: 字段名.into(),
            显示名: 显示名.into(),
            种类,
            默认值,
            提示: String::new(),
            只读: false,
        }
    }

    /// 附加悬停提示。
    pub fn 带提示(mut self, 提示: impl Into<String>) -> Self {
        self.提示 = 提示.into();
        self
    }

    /// 标记为只读。
    pub fn 设只读(mut self) -> Self {
        self.只读 = true;
        self
    }
}

/// 从组件实例导出属性值，或把属性值写回组件实例。
///
/// # 为什么用 `&mut dyn Any` 而不是泛型关联类型
///
/// 注册表需要把**异构**的组件放进同一个容器，因此必须擦除类型。
/// 每个实现者用 `downcast_mut` 恢复具体类型。如果类型不匹配，
/// 说明注册表被错误地填入了不匹配的条目 —— 这是编程错误而非用户错误，
/// 因此在 `实例化` 里返回 `false` 而不是静默忽略。
pub trait 组件类型: Send + Sync + 'static {
    /// 组件在场景文件里使用的稳定名称（英文，与 Bevy 组件名一致）。
    fn 组件名(&self) -> &'static str;

    /// 返回一个该组件的默认实例（装箱的 `Any`）。
    fn 默认实例(&self) -> Box<dyn std::any::Any>;

    /// 该组件暴露给编辑器的字段描述。
    fn 字段描述(&self) -> &[属性描述];

    /// 把组件实例里的数据导出成属性值。
    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>>;

    /// 把属性值写回组件实例。返回是否成功。
    ///
    /// 缺失的字段应保留实例中的原值，而不是重置为默认 ——
    /// 这样旧版本场景文件缺少新字段时不会把用户数据抹掉。
    fn 实例化(&self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>) -> bool;
}

/// 组件注册表错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum 注册表错误 {
    /// 同名组件被注册了两次。
    #[error("组件 `{0}` 已被注册，不能重复注册")]
    重复注册(String),

    /// 查询了未注册的组件。
    #[error("组件 `{0}` 未在注册表中，无法编辑")]
    未注册组件(String),

    /// 属性描述的默认值与其声明的种类不匹配。
    #[error("组件 `{组件}` 的字段 `{字段}` 默认值种类不匹配：声明为 {种类}，默认值是 {实际}")]
    默认值不匹配 {
        /// 组件名。
        组件: String,
        /// 字段名。
        字段: String,
        /// 声明的种类名。
        种类: String,
        /// 实际默认值的种类名。
        实际: String,
    },
}

/// 组件注册表。
///
/// 使用 `BTreeMap` 而非 `HashMap`：场景文件里的组件顺序应当稳定，
/// 否则每次保存都会产生无意义的文本 diff。
pub struct 组件注册表 {
    条目: BTreeMap<String, Box<dyn 组件类型>>,
}

impl Default for 组件注册表 {
    fn default() -> Self {
        Self::新建()
    }
}

impl fmt::Debug for 组件注册表 {
    fn fmt(&self, 格式: &mut fmt::Formatter<'_>) -> fmt::Result {
        格式
            .debug_struct("组件注册表")
            .field("已注册组件", &self.条目.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl 组件注册表 {
    /// 创建一个空注册表。
    pub fn 新建() -> Self {
        Self {
            条目: BTreeMap::new(),
        }
    }

    /// 注册一个组件类型。
    ///
    /// 注册时会校验每个属性的默认值是否符合声明的种类，
    /// 让配置错误在**程序启动时**暴露，而不是等用户点到那个字段才发现。
    pub fn 注册(&mut self, 类型: Box<dyn 组件类型>) -> Result<(), 注册表错误> {
        let 组件名 = 类型.组件名().to_string();
        for 描述 in 类型.字段描述() {
            if !描述.种类.接受(&描述.默认值) {
                return Err(注册表错误::默认值不匹配 {
                    组件: 组件名.clone(),
                    字段: 描述.字段名.clone(),
                    种类: 描述.种类.类型名().to_string(),
                    实际: 描述.默认值.种类名().to_string(),
                });
            }
        }
        if self.条目.contains_key(&组件名) {
            return Err(注册表错误::重复注册(组件名));
        }
        self.条目.insert(组件名, 类型);
        Ok(())
    }

    /// 查询一个组件类型。
    pub fn 查询(&self, 组件名: &str) -> Option<&dyn 组件类型> {
        self.条目.get(组件名).map(|装箱| 装箱.as_ref())
    }

    /// 要求某个组件必须存在，否则返回错误。
    pub fn 要求(&self, 组件名: &str) -> Result<&dyn 组件类型, 注册表错误> {
        self.查询(组件名)
            .ok_or_else(|| 注册表错误::未注册组件(组件名.to_string()))
    }

    /// 是否已注册该组件。
    pub fn 已注册(&self, 组件名: &str) -> bool {
        self.条目.contains_key(组件名)
    }

    /// 所有已注册组件名（字典序）。
    pub fn 组件名列表(&self) -> Vec<String> {
        self.条目.keys().cloned().collect()
    }

    /// 已注册组件数量。
    pub fn 数量(&self) -> usize {
        self.条目.len()
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use std::any::Any;

    /// 测试用的最小组件：一个 `f32`。
    struct 测试组件;

    #[derive(Debug, Default)]
    struct 测试数据 {
        速度: f32,
        启用: bool,
    }

    impl 组件类型 for 测试组件 {
        fn 组件名(&self) -> &'static str {
            "测试组件"
        }

        fn 默认实例(&self) -> Box<dyn Any> {
            Box::new(测试数据::default())
        }

        fn 字段描述(&self) -> &[属性描述] {
            测试组件描述()
        }

        fn 导出(&self, 实例: &dyn Any) -> Option<BTreeMap<String, 属性值>> {
            let 数据 = 实例.downcast_ref::<测试数据>()?;
            let mut 结果 = BTreeMap::new();
            结果.insert("速度".into(), 属性值::浮点(数据.速度));
            结果.insert("启用".into(), 属性值::布尔(数据.启用));
            Some(结果)
        }

        fn 实例化(
            &self,
            实例: &mut dyn Any,
            数据: &BTreeMap<String, 属性值>,
        ) -> bool {
            let Some(数据实例) = 实例.downcast_mut::<测试数据>() else {
                return false;
            };
            if let Some(值) = 数据.get("速度").and_then(属性值::取浮点) {
                数据实例.速度 = 值;
            }
            if let Some(值) = 数据.get("启用").and_then(属性值::取布尔) {
                数据实例.启用 = 值;
            }
            true
        }
    }

    /// 测试组件的字段描述。用 `OnceLock` 惰性构造，
    /// 因为 `属性描述` 含有 `String` 与 `Vec`，无法放进 `const`。
    fn 测试组件描述() -> &'static [属性描述] {
        use std::sync::OnceLock;
        static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
        列表.get_or_init(|| {
            vec![
                属性描述::新建("速度", "速度", 属性种类::自由浮点(), 属性值::浮点(0.0)),
                属性描述::新建("启用", "启用", 属性种类::布尔, 属性值::布尔(false)),
            ]
        })
    }

    #[test]
    fn 注册与查询() {
        let mut 注册表 = 组件注册表::新建();
        注册表.注册(Box::new(测试组件)).unwrap();
        assert!(注册表.已注册("测试组件"));
        assert_eq!(注册表.数量(), 1);
        assert!(注册表.查询("不存在").is_none());
    }

    #[test]
    fn 拒绝重复注册() {
        let mut 注册表 = 组件注册表::新建();
        注册表.注册(Box::new(测试组件)).unwrap();
        let 结果 = 注册表.注册(Box::new(测试组件));
        assert!(matches!(结果, Err(注册表错误::重复注册(_))));
    }

    /// 默认值与声明种类不符时必须在注册期就报错。
    struct 坏组件;

    /// 坏组件的字段描述：声明为布尔，默认值却是文本。
    fn 坏组件描述() -> &'static [属性描述] {
        use std::sync::OnceLock;
        static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
        列表.get_or_init(|| {
            vec![属性描述::新建(
                "开关",
                "开关",
                属性种类::布尔,
                属性值::文本("这不是布尔".into()),
            )]
        })
    }

    impl 组件类型 for 坏组件 {
        fn 组件名(&self) -> &'static str {
            "坏组件"
        }
        fn 默认实例(&self) -> Box<dyn Any> {
            Box::new(())
        }
        fn 字段描述(&self) -> &[属性描述] {
            坏组件描述()
        }
        fn 导出(&self, _: &dyn Any) -> Option<BTreeMap<String, 属性值>> {
            None
        }
        fn 实例化(&self, _: &mut dyn Any, _: &BTreeMap<String, 属性值>) -> bool {
            false
        }
    }

    #[test]
    fn 注册期校验默认值种类() {
        let mut 注册表 = 组件注册表::新建();
        let 结果 = 注册表.注册(Box::new(坏组件));
        match 结果 {
            Err(注册表错误::默认值不匹配 { 字段, .. }) => assert_eq!(字段, "开关"),
            其他 => panic!("期望默认值不匹配错误，实际：{其他:?}"),
        }
        // 校验失败不应污染注册表。
        assert_eq!(注册表.数量(), 0);
    }

    #[test]
    fn 接受规则容忍数值互换但拒绝跨类() {
        let 浮点种类 = 属性种类::自由浮点();
        assert!(浮点种类.接受(&属性值::整数(1)));
        assert!(浮点种类.接受(&属性值::浮点(1.5)));
        assert!(!浮点种类.接受(&属性值::文本("1".into())));

        let 整数种类 = 属性种类::自由整数();
        assert!(整数种类.接受(&属性值::浮点(2.0)));
        assert!(!整数种类.接受(&属性值::浮点(2.5)));
    }
}
