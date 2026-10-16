//! 通用属性值模型。
//!
//! # 为什么需要"通用属性值"
//!
//! 本期不使用 `bevy_reflect` 作为 Inspector 的驱动方式。原因不是反射不好，而是
//! **反射还没有覆盖编辑器需要的全部类型**：`Color`、`Handle<T>`、`Sprite` 里的
//! 枚举、以及游戏自己的组件，都还需要显式注册。如果 Inspector 依赖反射，
//! 那么"某个组件能不能编辑"就取决于它是否恰好派生了 `Reflect`，
//! 而编辑器无法对没派生反射的类型给出任何编辑能力。
//!
//! 于是反过来做：**场景文档只存通用属性值**，每个组件的每个字段由注册表声明为
//! 一个 [`属性种类`]，并提供 `导出/实例化` 两个方向的转换。这样：
//!
//! * 序列化格式与 Rust 类型解耦 —— 升级引擎不会破坏旧场景；
//! * 每个类型只需要写一次转换，Inspector 就自动获得编辑 UI；
//! * 没有 `Reflect` 派生要求的黑盒游戏组件也能编辑。
//!
//! 反射仍然有用武之地：`editor_app` 的 ECS 调试器（只读浏览运行时世界）
//! 会直接走 `bevy_reflect`，因为那里不存在"转换回文档"的需求。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::path::项目相对路径;

/// 颜色值。
///
/// 单独用一个结构而不是复用 `bevy::color::Color`，是为了让场景文档层保持
/// 与 Bevy 无关。同时显式记录色彩空间，避免 sRGB / 线性空间来回转换时
/// 出现"颜色越改越亮"的经典问题。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct 颜色值 {
    /// 红分量，范围 `0.0..=1.0`。
    pub 红: f32,
    /// 绿分量，范围 `0.0..=1.0`。
    pub 绿: f32,
    /// 蓝分量，范围 `0.0..=1.0`。
    pub 蓝: f32,
    /// 透明度，范围 `0.0..=1.0`。
    pub 透明度: f32,
    /// 该分量是否处于 sRGB 非线性空间。
    pub 是_srgb: bool,
}

impl 颜色值 {
    /// 由 sRGB 分量构造。
    pub const fn srgb(红: f32, 绿: f32, 蓝: f32, 透明度: f32) -> Self {
        Self {
            红,
            绿,
            蓝,
            透明度,
            是_srgb: true,
        }
    }

    /// 由线性空间分量构造。
    pub const fn 线性(红: f32, 绿: f32, 蓝: f32, 透明度: f32) -> Self {
        Self {
            红,
            绿,
            蓝,
            透明度,
            是_srgb: false,
        }
    }

    /// 全白。
    pub const fn 白() -> Self {
        Self::srgb(1.0, 1.0, 1.0, 1.0)
    }

    /// 把四个分量收拢到 `0.0..=1.0`，避免用户输入把颜色打出有效范围。
    ///
    /// 刻意不叫 `收拢`：那会与 [`属性值::收拢`] 同名，
    /// 在 `属性值::颜色(值) => 值.收拢()` 这样的调用点造成方法解析歧义。
    pub fn 收拢分量(&mut self) {
        for 分量 in [&mut self.红, &mut self.绿, &mut self.蓝, &mut self.透明度] {
            if 分量.is_nan() {
                *分量 = 0.0;
            } else {
                *分量 = 分量.clamp(0.0, 1.0);
            }
        }
    }
}

/// 枚举取值。
///
/// 存"名字 + 序号 + 可选负载"而不是只存序号：序号在枚举变体顺序变化后会
/// 指向错误的变体，而名字可以匹配失败时回退到序号并报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 枚举值 {
    /// 变体名称，与 Rust 里的变体标识符一致。
    pub 变体名: String,
    /// 变体在枚举声明中的序号，作为名字失配时的兜底。
    pub 序号: u32,
    /// 可选负载（目前用于带数据的变体）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 负载: Option<Box<属性值>>,
}

impl 枚举值 {
    /// 由变体名与序号构造一个无负载枚举值。
    pub fn 新建(变体名: impl Into<String>, 序号: u32) -> Self {
        Self {
            变体名: 变体名.into(),
            序号,
            负载: None,
        }
    }
}

/// 资源类型的逻辑分类。
///
/// 用它而不是靠扩展名硬猜，是因为 Inspector 需要知道"这一栏该弹出哪种选择器"，
/// 而导入器需要知道"这个文件该交给谁处理"。二者都必须能按类型分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum 资源类别 {
    /// 图片纹理。
    图片,
    /// 音频。
    音频,
    /// 字体。
    字体,
    /// 3D 模型。
    模型,
    /// 材质。
    材质,
    /// 场景文件。
    场景,
    /// 着色器。
    着色器,
    /// 其他未分类。
    其他,
}

impl 资源类别 {
    /// 根据文件扩展名推断资源类别。
    ///
    /// 这是**猜测**，用于资源浏览器分组与拖放默认行为。真正的导入行为
    /// 以导入器的显式声明为准，扩展名只作为第一层分派。
    pub fn 由扩展名推断(扩展名: &str) -> Self {
        match 扩展名 {
            "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tga" | "ktx2" | "basis" => Self::图片,
            "wav" | "ogg" | "mp3" | "flac" => Self::音频,
            "ttf" | "otf" => Self::字体,
            "gltf" | "glb" | "obj" => Self::模型,
            "material" | "mat" => Self::材质,
            "scene" | "scn" | "bsn" => Self::场景,
            "wgsl" | "wesl" => Self::着色器,
            _ => Self::其他,
        }
    }

    /// 中文显示名。
    pub fn 显示名(&self) -> &'static str {
        match self {
            资源类别::图片 => "图片",
            资源类别::音频 => "音频",
            资源类别::字体 => "字体",
            资源类别::模型 => "模型",
            资源类别::材质 => "材质",
            资源类别::场景 => "场景",
            资源类别::着色器 => "着色器",
            资源类别::其他 => "其他",
        }
    }
}

/// 属性值的具体种类。
///
/// 这些种类构成**完整且封闭的集合**：任何需要在 Inspector 里编辑的数据，
/// 都必须能表达成其中之一。新增一种编辑能力 = 新增一个变体，
/// 编译器会强制所有匹配处处理它。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum 属性值 {
    /// 布尔值。
    布尔(bool),
    /// 64 位有符号整数。
    整数(i64),
    /// 32 位浮点数。
    浮点(f32),
    /// 字符串。
    文本(String),
    /// 二维向量。
    二维(f32, f32),
    /// 三维向量。
    三维(f32, f32, f32),
    /// 四维向量。
    四维(f32, f32, f32, f32),
    /// 颜色。
    颜色(颜色值),
    /// 枚举。
    枚举(枚举值),
    /// 资源引用。
    资源(资源引用值),
    /// 同构列表（例如一组路径点）。
    列表(Vec<属性值>),
    /// 子结构（例如 `Sprite` 里嵌套的 `Rect`）。
    结构(Vec<(String, 属性值)>),
    /// 显式空值。用于表示 `Option::None`，与"字段不存在"区分开。
    空,
}

impl 属性值 {
    /// 种类的稳定名称，用于错误信息与注册表校验。
    pub fn 种类名(&self) -> &'static str {
        match self {
            属性值::布尔(_) => "布尔",
            属性值::整数(_) => "整数",
            属性值::浮点(_) => "浮点",
            属性值::文本(_) => "文本",
            属性值::二维(..) => "二维",
            属性值::三维(..) => "三维",
            属性值::四维(..) => "四维",
            属性值::颜色(_) => "颜色",
            属性值::枚举(_) => "枚举",
            属性值::资源(_) => "资源",
            属性值::列表(_) => "列表",
            属性值::结构(_) => "结构",
            属性值::空 => "空",
        }
    }

    /// 尝试按 f32 读取。
    ///
    /// 整数与浮点之间互相容忍：场景里写 `1` 还是 `1.0` 都能读成 `f32`，
    /// 这样手写场景文件时不会因为少写小数点而加载失败。
    pub fn 取浮点(&self) -> Option<f32> {
        match self {
            属性值::浮点(值) => Some(*值),
            属性值::整数(值) => Some(*值 as f32),
            _ => None,
        }
    }

    /// 尝试按 i64 读取。
    pub fn 取整数(&self) -> Option<i64> {
        match self {
            属性值::整数(值) => Some(*值),
            属性值::浮点(值) if 值.fract() == 0.0 => Some(*值 as i64),
            _ => None,
        }
    }

    /// 尝试按 bool 读取。
    pub fn 取布尔(&self) -> Option<bool> {
        match self {
            属性值::布尔(值) => Some(*值),
            _ => None,
        }
    }

    /// 尝试按 `&str` 读取。
    pub fn 取文本(&self) -> Option<&str> {
        match self {
            属性值::文本(值) => Some(值),
            _ => None,
        }
    }

    /// 尝试按二维向量读取。
    pub fn 取二维(&self) -> Option<(f32, f32)> {
        match self {
            属性值::二维(x, y) => Some((*x, *y)),
            _ => None,
        }
    }

    /// 尝试按三维向量读取。
    pub fn 取三维(&self) -> Option<(f32, f32, f32)> {
        match self {
            属性值::三维(x, y, z) => Some((*x, *y, *z)),
            _ => None,
        }
    }

    /// 尝试按颜色读取。
    pub fn 取颜色(&self) -> Option<颜色值> {
        match self {
            属性值::颜色(值) => Some(*值),
            _ => None,
        }
    }

    /// 尝试按资源引用读取。
    pub fn 取资源(&self) -> Option<&资源引用值> {
        match self {
            属性值::资源(值) => Some(值),
            _ => None,
        }
    }

    /// 把数值分量收拢到合法范围。
    ///
    /// 返回是否发生了修改。Inspector 在用户输入后调用它，
    /// 保证文档里永远不会出现 `NaN` 或超出范围的颜色分量。
    pub fn 收拢(&mut self) -> bool {
        match self {
            属性值::浮点(值) => {
                if 值.is_nan() {
                    *值 = 0.0;
                    return true;
                }
                false
            }
            属性值::颜色(值) => {
                let 原 = *值;
                值.收拢分量();
                *值 != 原
            }
            属性值::二维(x, y) => 清理非有限(x) | 清理非有限(y),
            属性值::三维(x, y, z) => 清理非有限(x) | 清理非有限(y) | 清理非有限(z),
            属性值::四维(a, b, c, d) => {
                清理非有限(a) | 清理非有限(b) | 清理非有限(c) | 清理非有限(d)
            }
            属性值::列表(列表) => 列表.iter_mut().fold(false, |累计, 项| 项.收拢() | 累计),
            属性值::结构(字段) => 字段
                .iter_mut()
                .fold(false, |累计, (_, 项)| 项.收拢() | 累计),
            _ => false,
        }
    }
}

/// 把 `NaN` / 无穷替换为 0，返回是否发生替换。
fn 清理非有限(值: &mut f32) -> bool {
    if 值.is_finite() {
        false
    } else {
        *值 = 0.0;
        true
    }
}

/// 属性值里内嵌的资源引用。
///
/// 同时保存路径与 UUID：
/// * **路径**保证人可读，且在没有导入元数据时依然能用（`AssetServer` 靠路径加载）；
/// * **UUID**保证改名/移动后仍能重建引用。
///
/// 若两者都失效，引用进入"缺失"状态并记录在文档校验报告里，
/// 而不是让场景静默加载出一个空组件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 资源引用值 {
    /// 项目相对路径。
    pub 路径: 项目相对路径,
    /// 资源唯一标识，可能尚未生成（例如手工放进 assets 的文件）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 唯一标识: Option<Uuid>,
}

impl 资源引用值 {
    /// 只按路径构造一个引用。
    pub fn 按路径(路径: 项目相对路径) -> Self {
        Self {
            路径,
            唯一标识: None,
        }
    }

    /// 按路径与 UUID 构造一个引用。
    pub fn 完整(路径: 项目相对路径, 唯一标识: Uuid) -> Self {
        Self {
            路径,
            唯一标识: Some(唯一标识),
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 整数与浮点互相容忍() {
        assert_eq!(属性值::整数(3).取浮点(), Some(3.0));
        assert_eq!(属性值::浮点(3.0).取整数(), Some(3));
        // 有小数部分时不应被静默截断成整数。
        assert_eq!(属性值::浮点(3.5).取整数(), None);
    }

    #[test]
    fn 收拢会清掉_nan() {
        let mut 值 = 属性值::三维(f32::NAN, 1.0, f32::INFINITY);
        assert!(值.收拢());
        assert_eq!(值.取三维(), Some((0.0, 1.0, 0.0)));
        // 已经合法时不应报告修改。
        assert!(!值.收拢());
    }

    #[test]
    fn 颜色收拢到合法区间() {
        let mut 值 = 属性值::颜色(颜色值::srgb(2.0, -1.0, 0.5, 0.5));
        assert!(值.收拢());
        let 颜色 = 值.取颜色().unwrap();
        assert_eq!((颜色.红, 颜色.绿, 颜色.蓝), (1.0, 0.0, 0.5));
    }

    #[test]
    fn 属性值往返序列化() {
        let 原值 = 属性值::结构(vec![
            ("位置".into(), 属性值::三维(1.0, 2.0, 3.0)),
            ("可见".into(), 属性值::布尔(true)),
            ("模式".into(), 属性值::枚举(枚举值::新建("重复", 2))),
        ]);
        let 文本 = serde_json::to_string(&原值).unwrap();
        let 还原: 属性值 = serde_json::from_str(&文本).unwrap();
        assert_eq!(原值, 还原);
    }

    #[test]
    fn 资源类别由扩展名推断() {
        assert_eq!(资源类别::由扩展名推断("png"), 资源类别::图片);
        assert_eq!(资源类别::由扩展名推断("OGG"), 资源类别::其他);
        assert_eq!(资源类别::由扩展名推断("glb"), 资源类别::模型);
    }
}
