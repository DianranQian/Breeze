//! 编译期 BSN 场景模板。
//!
//! # 这一层解决什么问题
//!
//! 编辑器里有一批"开发内置、结构固定"的场景：关卡骨架、示例项目、新建场景的默认内容。
//! 它们的特点是**形状在编译期就完全确定**（几个实体、谁是谁的子级、挂哪些组件），
//! 只有标量数值可能来自用户输入。这正是 `bsn!` 擅长的形态。
//!
//! 于是把它们写成 `bsn!`，而不是手写一堆 `文档.生成实体(...)` +
//! `实体.加组件(...)` 的样板代码。好处是层级与组件在源码里**一眼可读**，
//! 而且层级、命名、补丁语义都由官方实现保证，不用自己维护。
//!
//! # 必须说清的边界
//!
//! `bsn!` 是 proc macro，**结构**由源码字面决定：
//!
//! * 能变：字段值、构造函数的参数 —— 也就是 [`带参数模板`] 展示的那种；
//! * 不能变：实体个数、层级形状、组件的**种类集合**。
//!
//! 想按运行时数据决定"生成 3 个还是 5 个子节点"，只能写
//! `bsn_list!` + 迭代器 + 场景表达式，或者干脆回到 `scene_doc` 的
//! `生成实体` 接口。这一点在 [`crate::capability`] 的
//! `runtime_string_parsing` 条目里作为"官方未提供"记录，不在本项目伪造。
//!
//! # 这些模板不直接产出编辑器状态
//!
//! 每个模板只负责"把实体实例化进一个真实的 `World`"。要变成编辑器可编辑的场景，
//! 必须再走 [`crate::extract`] 提取成 [`scene_doc::场景文档`]，
//! 由编辑器统一管理撤销重做、身份与保存。**没有任何模板可以绕过文档层**。

use bevy::prelude::*;

use crate::error::BSN错误;
use crate::host::在世界上实例化;

/// 一个编译期确定、可实例化的 BSN 场景模板。
///
/// 实现者只暴露"标识 + 名称键 + 实例化"，不暴露内部实体结构 ——
/// 调用方要检查结构，应该看**实例化之后的世界**或**提取出的文档**，
/// 而不是试图反射模板本身。
pub trait BSN场景模板 {
    /// 稳定标识（英文小写下划线），用于日志、测试与未来的模板菜单键。
    fn 标识(&self) -> &'static str;

    /// 人类可读名称的**词条键**。翻译由界面层按当前语言解析，
    /// 本 crate 不依赖 i18n 表。
    fn 名称键(&self) -> &'static str;

    /// 把模板实例化到给定世界，返回根实体。
    ///
    /// 世界必须已经装配 `AssetServer` 与 `Assets<ScenePatch>`
    /// （[`crate::BSN宿主`] 默认满足）。缺少时返回错误，不会 panic。
    fn 实例化(&self, 世界: &mut World) -> Result<Entity, BSN错误>;
}

/// 模板的静态元信息。用于"模板目录"、测试断言与文档生成。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct 模板说明 {
    /// 稳定标识。
    pub 标识: &'static str,
    /// 名称词条键。
    pub 名称键: &'static str,
    /// 预期产生的实体个数（含根）。
    pub 预期实体数: usize,
    /// 预期层级最大深度（只有根时为 0）。
    pub 预期最大深度: usize,
}

/// 基础关卡骨架：根 + 两个子节点，都是纯 Bevy 组件。
///
/// 刻意只用 `Name` / `Transform` / `Children` 三种官方能力 ——
/// 它是"官方 BSN 能做到什么"的最小可验证样本，
/// 不依赖本项目任何自定义组件，因此可以当作升级 Bevy 时的冒烟测试。
pub struct 基础关卡模板;

impl BSN场景模板 for 基础关卡模板 {
    fn 标识(&self) -> &'static str {
        "basic_level"
    }

    fn 名称键(&self) -> &'static str {
        "bsn_template_basic_level"
    }

    fn 实例化(&self, 世界: &mut World) -> Result<Entity, BSN错误> {
        在世界上实例化(
            世界,
            bsn! {
                #关卡根
                Transform { translation: { Vec3::new(0.0, 0.0, 0.0) } }
                Children [
                    #地面
                    Transform {
                        translation: { Vec3::new(0.0, -120.0, 0.0) },
                        scale: { Vec3::new(8.0, 1.0, 1.0) }
                    }
                    --
                    #玩家起点
                    Transform {
                        translation: { Vec3::new(-160.0, 0.0, 0.0) },
                        rotation: { Quat::from_rotation_z(std::f32::consts::FRAC_PI_4) },
                        scale: { Vec3::new(1.5, 1.5, 1.5) }
                    }
                ]
            },
        )
    }
}

/// 三层嵌套模板：验证官方 BSN 支持**多层**关系，而不只是两层。
///
/// 结构：
/// ```text
/// 世界根
///   ├─ 区域甲
///   │    ├─ 灯
///   │    └─ 箱子
///   └─ 区域乙
/// ```
pub struct 三层嵌套模板;

impl BSN场景模板 for 三层嵌套模板 {
    fn 标识(&self) -> &'static str {
        "nested_three_levels"
    }

    fn 名称键(&self) -> &'static str {
        "bsn_template_nested_three_levels"
    }

    fn 实例化(&self, 世界: &mut World) -> Result<Entity, BSN错误> {
        在世界上实例化(
            世界,
            bsn! {
                #世界根
                Children [
                    #区域甲
                    Transform { translation: { Vec3::new(100.0, 0.0, 0.0) } }
                    Children [
                        #灯
                        Transform { translation: { Vec3::new(0.0, 40.0, 0.0) } }
                        --
                        #箱子
                        Transform { translation: { Vec3::new(20.0, 0.0, 0.0) } }
                    ]
                    --
                    #区域乙
                    Transform { translation: { Vec3::new(-100.0, 0.0, 0.0) } }
                ]
            },
        )
    }
}

/// 带运行时参数的模板：证明 `bsn!` 的值位置可以是运行时数据。
///
/// 这是本模块对"编译期 vs 运行时"边界最直接的证据：
///
/// * **形状**（根 + 一个子节点、挂 `Transform` + `Children`）在编译期固定；
/// * **数值**（名称、平移、缩放）来自调用方传入的运行时数据。
///
/// 所以"用户编辑的数据 → BSN"并不是完全不可能 —— 不可能的是
/// "用户编辑的数据 → 任意形状的 BSN"。这里的模板是**预先写好的形状**，
/// 用来承载运行时数值。这一点必须区分清楚，否则就会写出"编辑器能把
/// 任意场景转成 BSN"这种不成立的宣称。
#[derive(Debug, Clone)]
pub struct 带参数模板 {
    /// 根实体名称。
    pub 根名称: String,
    /// 根实体平移。
    pub 根平移: (f32, f32, f32),
    /// 子实体名称。
    pub 子名称: String,
    /// 子实体的统一缩放。
    pub 子缩放: f32,
}

impl Default for 带参数模板 {
    fn default() -> Self {
        Self {
            根名称: "参数根".to_string(),
            根平移: (0.0, 0.0, 0.0),
            子名称: "参数子".to_string(),
            子缩放: 1.0,
        }
    }
}

impl BSN场景模板 for 带参数模板 {
    fn 标识(&self) -> &'static str {
        "parameterized"
    }

    fn 名称键(&self) -> &'static str {
        "bsn_template_parameterized"
    }

    fn 实例化(&self, 世界: &mut World) -> Result<Entity, BSN错误> {
        // `#名字` 只能写字面量，所以这里用 `#Root` 占位来获得名字作用域，
        // 再紧跟一条 `Name(...)` 组件把名称补丁成运行时字符串 ——
        // 这是官方文档明确给出的写法（“Dynamic Name Values and Entity References”）。
        let 根名称 = self.根名称.clone();
        let 根平移 = Vec3::new(self.根平移.0, self.根平移.1, self.根平移.2);
        let 子名称 = self.子名称.clone();
        let 子缩放 = Vec3::new(self.子缩放, self.子缩放, self.子缩放);
        在世界上实例化(
            世界,
            bsn! {
                #Root
                Name(根名称)
                Transform { translation: { 根平移 } }
                Children [
                    #Child
                    Name(子名称)
                    Transform { scale: { 子缩放 } }
                ]
            },
        )
    }
}

/// 内置模板的静态元信息表。
///
/// 与 [`内置模板列表`] 必须一一对应，由测试守住。
pub const 内置模板说明: &[模板说明] = &[
    模板说明 {
        标识: "basic_level",
        名称键: "bsn_template_basic_level",
        预期实体数: 3,
        预期最大深度: 1,
    },
    模板说明 {
        标识: "nested_three_levels",
        名称键: "bsn_template_nested_three_levels",
        预期实体数: 5,
        预期最大深度: 2,
    },
    模板说明 {
        标识: "parameterized",
        名称键: "bsn_template_parameterized",
        预期实体数: 2,
        预期最大深度: 1,
    },
];

/// 全部内置模板的实例（每次调用都新建，模板本身无可变状态）。
pub fn 内置模板列表() -> Vec<Box<dyn BSN场景模板>> {
    vec![
        Box::new(基础关卡模板),
        Box::new(三层嵌套模板),
        Box::new(带参数模板::default()),
    ]
}

/// 按标识查静态元信息。
pub fn 查模板说明(标识: &str) -> Option<&'static 模板说明> {
    内置模板说明.iter().find(|说明| 说明.标识 == 标识)
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::host::BSN宿主;

    #[test]
    fn 模板列表与说明表一一对应() {
        let 模板 = 内置模板列表();
        assert_eq!(模板.len(), 内置模板说明.len());
        for (项, 说明) in 模板.iter().zip(内置模板说明.iter()) {
            assert_eq!(项.标识(), 说明.标识);
            assert_eq!(项.名称键(), 说明.名称键);
        }
    }

    #[test]
    fn 模板与说明的标识唯一() {
        let mut 已见 = std::collections::BTreeSet::new();
        for 说明 in 内置模板说明 {
            assert!(已见.insert(说明.标识), "模板标识 `{}` 重复", 说明.标识);
        }
    }

    #[test]
    fn 没有依赖资源时实例化返回错误而不是崩溃() {
        // 裸世界缺少 AssetServer，必须得到错误。
        let mut 世界 = World::new();
        match 基础关卡模板.实例化(&mut 世界) {
            Err(BSN错误::世界缺少资源 { .. }) => {}
            其他 => panic!("期望缺少资源错误，实际：{其他:?}"),
        }
    }

    #[test]
    fn 基础模板实例化后可读到名称与变换() {
        let mut 宿主 = BSN宿主::新建();
        let 根 = 基础关卡模板.实例化(宿主.世界可变()).unwrap();

        assert_eq!(宿主.名称(根).as_deref(), Some("关卡根"));
        assert_eq!(宿主.变换(根).map(|值| 值.translation), Some(Vec3::ZERO));

        let 孩子 = 宿主.子实体(根);
        assert_eq!(孩子.len(), 2, "根下应有两个子实体");
        // Children 是有序的，顺序必须与 BSN 里写的顺序一致。
        assert_eq!(宿主.名称(孩子[0]).as_deref(), Some("地面"));
        assert_eq!(宿主.名称(孩子[1]).as_deref(), Some("玩家起点"));

        // 子实体的 ChildOf 必须指回根。
        assert_eq!(宿主.父实体(孩子[0]), Some(根));
        assert_eq!(宿主.父实体(孩子[1]), Some(根));
    }

    #[test]
    fn 参数模板接受运行时数值() {
        let mut 宿主 = BSN宿主::新建();
        let 模板 = 带参数模板 {
            根名称: "关卡甲".to_string(),
            根平移: (12.0, -34.0, 5.5),
            子名称: "守卫".to_string(),
            子缩放: 2.5,
        };
        let 根 = 模板.实例化(宿主.世界可变()).unwrap();

        assert_eq!(宿主.名称(根).as_deref(), Some("关卡甲"));
        let 变换 = 宿主.变换(根).unwrap();
        assert_eq!(变换.translation, Vec3::new(12.0, -34.0, 5.5));

        let 孩子 = 宿主.子实体(根);
        assert_eq!(孩子.len(), 1);
        assert_eq!(宿主.名称(孩子[0]).as_deref(), Some("守卫"));
        assert_eq!(宿主.变换(孩子[0]).unwrap().scale, Vec3::splat(2.5));
    }

    /// 说明表里的"预期实体数 / 预期最大深度"必须与**真实实例化并提取**的结果一致。
    ///
    /// 这条测试很重要：它让 [`内置模板说明`] 不再是"手写的注释"，
    /// 而是被真实运行校验过的数据。改模板结构却忘了改说明会立刻失败。
    #[test]
    fn 说明表的规模与真实提取结果一致() {
        use crate::extract::{提取文档, 提取配置};
        use scene_doc::registry::组件注册表;

        let 注册表 = 组件注册表::新建();
        for 模板 in 内置模板列表() {
            let 说明 = 查模板说明(模板.标识())
                .unwrap_or_else(|| panic!("模板 `{}` 缺少说明条目", 模板.标识()));
            let mut 宿主 = BSN宿主::新建();
            let 根 = 模板
                .实例化(宿主.世界可变())
                .unwrap_or_else(|错误| panic!("模板 `{}` 实例化失败：{错误}", 模板.标识()));
            let 配置 = 提取配置::新建(&注册表, vec![]);
            let 产物 = 提取文档(宿主.世界(), 根, 模板.标识(), &配置)
                .unwrap_or_else(|错误| panic!("模板 `{}` 提取失败：{错误}", 模板.标识()));

            assert_eq!(
                产物.报告.实体数,
                说明.预期实体数,
                "模板 `{}` 的实际实体数与说明不符",
                模板.标识()
            );
            assert_eq!(
                产物.报告.最大深度,
                说明.预期最大深度,
                "模板 `{}` 的实际层级深度与说明不符",
                模板.标识()
            );
        }
    }
}
