//! 从真实 Bevy ECS 提取出 `scene_doc` 场景文档。
//!
//! # 为什么方向只有一个
//!
//! `bsn!` 是 **proc macro**，在编译期展开成 `impl Scene`。它接受的是 Rust 表达式，
//! 不是字符串。因此：
//!
//! ```text
//! 编译期 BSN 场景描述  ──实例化──>  真实 ECS 实体  ──提取──>  scene_doc 文档
//!                                                              │
//! 运行时 scene_doc 数据  ──✗ 不能反向生成 bsn! ─────────────────┘
//! ```
//!
//! 反向（文档 → `bsn!`）需要"运行时把数据拼成 Rust 源码再编译"，这在本版本
//! 没有任何官方支持，也不是本项目要做的事（见 [`crate::capability`] 的
//! `runtime_string_parsing` 条目）。所以本模块**只实现一个方向**，
//! 并在文档与测试里如实标注。
//!
//! # 转换的三条铁律
//!
//! 1. **不静默丢数据**：实体上出现的每一个组件都必须被归入三类之一 ——
//!    已转换、被明确忽略（引擎派生，可由其他数据重建）、或未转换。
//!    只要存在未转换组件，[`提取并校验`] 就返回错误。
//! 2. **不伪造身份**：文档实体身份是新的 UUID，不复用 Bevy 的 `Entity`
//!    （后者的 index/generation 会在运行时变化，不具备持久语义）。
//!    映射关系由 [`提取产物::实体映射`] 显式给出。
//! 3. **规范化必须写明**：`Transform → 变换`、`Children/ChildOf → 父实体 + 次序`、
//!    缺 `Name` 时自动命名 —— 这些都记进 [`提取报告::规范化`]，不是隐式行为。

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use bevy::transform::components::TransformTreeChanged;

use scene_doc::document::{
    变换数据, 场景实体, 场景文档, 实体标识, 文档校验报告, 组件数据
};
use scene_doc::registry::组件注册表;

use crate::error::BSN错误;

/// 从世界取出某具体类型组件的函数。
///
/// 返回 `Box<dyn Any>` 而不是 `&dyn Any` 是刻意的：有些组件在世界里的类型
/// 与它在文档适配器里的类型**不是同一个**。典型例子是可见性：
/// 世界里是 Bevy 的 `Visibility` 枚举，而编辑器的注册表条目
/// （`editor_app::scene_bridge::可见性属性`）是一个简化的 `bool` 包装。
/// 要复用注册表的 [`scene_doc::registry::组件类型::导出`]，必须先把
/// `Visibility` 转成 `可见性属性`，而那就需要产生一个**新值**。
type 取出函数 = Box<dyn Fn(&World, Entity) -> Option<Box<dyn Any>> + Send + Sync>;

fn 取出组件<T: Component + Clone>(世界: &World, 实体: Entity) -> Option<Box<dyn Any>> {
    世界
        .get::<T>(实体)
        .map(|值| Box::new(值.clone()) as Box<dyn Any>)
}

/// 一个"受支持组件"的声明：它在文档里叫什么，以及怎么从世界取出来。
///
/// # 为什么需要它
///
/// 场景文档与 Bevy 类型之间是**弱耦合**的：文档只认识"类型名 + 属性表"。
/// 要把 ECS 里的具体类型变成属性表，必须先把它类型擦除成 `&dyn Any`，
/// 交给注册表里的 [`scene_doc::registry::组件类型::导出`] 处理。
/// 这一步无法对任意类型自动完成 —— 它必须由知道 `T` 的代码写一行。
pub struct 组件提取器 {
    /// 组件在文档与组件注册表里的名字（例如 `Sprite`）。
    pub 组件名: &'static str,
    世界类型标识: TypeId,
    取出: 取出函数,
}

impl 组件提取器 {
    /// 世界类型与文档适配器类型**相同**时用它：直接把组件克隆出来。
    pub fn 新建<T: Component + Clone>(组件名: &'static str) -> Self {
        Self {
            组件名,
            世界类型标识: TypeId::of::<T>(),
            取出: Box::new(取出组件::<T>),
        }
    }

    /// 世界类型 `W` 与文档侧类型 `D` **不同**时用它。
    ///
    /// `转换` 负责把世界里的值映射成注册表适配器认识的值。
    /// 必须保证这个映射与反方向（`组件类型::实例化` 之后由
    /// `editor_app::scene_bridge` 把它插回世界）**语义一致**，
    /// 否则文档与世界会悄悄跑偏。本项目用集成测试守住这条一致性。
    pub fn 带转换<W, D>(组件名: &'static str, 转换: fn(&W) -> D) -> Self
    where
        W: Component + Clone,
        D: Any + 'static,
    {
        let 取出 = Box::new(move |世界: &World, 实体: Entity| -> Option<Box<dyn Any>> {
            let 世界值 = 世界.get::<W>(实体)?;
            Some(Box::new(转换(世界值)) as Box<dyn Any>)
        });
        Self {
            组件名,
            世界类型标识: TypeId::of::<W>(),
            取出,
        }
    }

    /// 该提取器所匹配的**世界侧**组件 `TypeId`。
    ///
    /// 注意是世界侧：组件分类是针对实体上真实存在的组件做的。
    pub fn 类型标识(&self) -> TypeId {
        self.世界类型标识
    }
}

/// 一个已知类型：名字 + `TypeId`。
///
/// 用于把实体上的 `TypeId` 分类成"已转换 / 有意忽略 / 未转换"，
/// 并在报告里给出可读名字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct 已知类型 {
    /// 可读名字。
    pub 名字: &'static str,
    /// 类型标识。
    pub 类型标识: TypeId,
}

impl 已知类型 {
    /// 为一个实现了 `Component` 的类型建立条目。
    pub fn 新建<T: Component>(名字: &'static str) -> Self {
        Self {
            名字,
            类型标识: TypeId::of::<T>(),
        }
    }
}

/// Bevy 自动补齐、且**可由已有数据重建**的组件。默认忽略，不写入文档。
///
/// 这些不是"转换失败"，而是"转换时有意不搬"：
/// * `GlobalTransform` 是 `Transform` 沿层级累积的结果，运行时由 Bevy 自己维护；
/// * `TransformTreeChanged` 是变换传播用的内部标记；
/// * `InheritedVisibility` / `ViewVisibility` 是 `Visibility` 的必需组件，
///   由可见性与层级共同推导，同样不该进文档。
///
/// 把 `GlobalTransform` 写进文档反而是错的：它会与 `变换` 字段产生两份互相
/// 矛盾的世界坐标，用户改一处就会出现"视口里位置和数字不一致"。
///
/// **注意**：这张表是**白名单性质的**。新增任何条目都必须能说清
/// "它为什么可以从已有数据重建"，否则就是在用忽略掩盖数据丢失。
pub fn 默认引擎派生组件() -> Vec<已知类型> {
    vec![
        已知类型::新建::<GlobalTransform>("GlobalTransform"),
        已知类型::新建::<TransformTreeChanged>("TransformTreeChanged"),
        已知类型::新建::<InheritedVisibility>("InheritedVisibility"),
        已知类型::新建::<ViewVisibility>("ViewVisibility"),
    ]
}

/// 提取配置。
pub struct 提取配置<'a> {
    /// 组件注册表。决定受支持组件的字段描述与 `属性值` 导出方式。
    pub 注册表: &'a 组件注册表,
    /// 受支持的组件提取器。未出现在此列表里的组件一律算"未转换"。
    pub 提取器: Vec<组件提取器>,
    /// 有意忽略的引擎派生组件。
    pub 引擎派生组件: Vec<已知类型>,
}

impl<'a> 提取配置<'a> {
    /// 以注册表与提取器列表建立配置，引擎派生组件取默认值。
    pub fn 新建(注册表: &'a 组件注册表, 提取器: Vec<组件提取器>) -> Self {
        Self {
            注册表,
            提取器,
            引擎派生组件: 默认引擎派生组件(),
        }
    }

    /// TypeId → 可读名字（覆盖提取器与引擎派生组件）。
    fn 名字表(&self) -> BTreeMap<TypeId, &'static str> {
        let mut 表 = BTreeMap::new();
        for 提取器 in &self.提取器 {
            表.insert(提取器.类型标识(), 提取器.组件名);
        }
        for 类型 in &self.引擎派生组件 {
            表.insert(类型.类型标识, 类型.名字);
        }
        表
    }

    /// 属于"已转换"的组件类型集合。
    ///
    /// 包含固定项（`Name` / `Transform` / `Children` / `ChildOf`）
    /// 与全部提取器的类型。这些类型即使不出现在实体上也算已覆盖 ——
    /// 不会出现"实体上没这个组件、却被算成未转换"的荒谬结果。
    fn 已转换类型集合(&self) -> BTreeSet<TypeId> {
        let mut 集合 = BTreeSet::new();
        集合.insert(TypeId::of::<Name>());
        集合.insert(TypeId::of::<Transform>());
        集合.insert(TypeId::of::<Children>());
        集合.insert(TypeId::of::<ChildOf>());
        for 提取器 in &self.提取器 {
            集合.insert(提取器.类型标识());
        }
        集合
    }
}

/// 一个无法转换到文档里的组件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 未转换组件 {
    /// 所属文档实体。
    pub 实体: 实体标识,
    /// 所属实体名称，便于定位。
    pub 实体名称: String,
    /// 组件类型标识。
    ///
    /// `None` 表示该组件**没有对应的 Rust 类型**（`ComponentInfo::type_id()`
    /// 返回 `None`）。这种情况极少见，但一旦出现，就不能假装它是某个类型。
    ///
    /// 为什么是 `TypeId` 而不是名字：`ComponentInfo::name()` 只在
    /// `bevy_utils/debug` 特性开启时返回真实名字，本项目未开启该特性，
    /// 因此**无法**为任意组件取得名字。`TypeId` 至少能让开发者用
    /// `TypeId::of::<可疑类型>()` 对出来。这一点在
    /// [`crate::capability`] 的 `component_name_enumeration` 条目里写明。
    pub 类型标识: Option<TypeId>,
}

impl 未转换组件 {
    /// 组件的诊断文本。
    pub fn 类型文本(&self) -> String {
        match self.类型标识 {
            Some(类型) => format!("{类型:?}"),
            None => "<没有 Rust 类型的组件>".to_string(),
        }
    }
}

/// 一次提取的完整报告。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 提取报告 {
    /// 提取出的实体个数。
    pub 实体数: usize,
    /// 层级最大深度（只有根时为 0）。
    pub 最大深度: usize,
    /// 带来真实 `Name` 组件的实体个数。
    pub 有名称的实体: usize,
    /// 缺失 `Name`、由适配层自动命名的实体。
    pub 自动命名的实体: Vec<实体标识>,
    /// 带来 `Transform` 组件的实体个数。
    pub 有变换的实体: usize,
    /// 已转换组件：组件名 → 次数。
    pub 已转换组件: BTreeMap<String, usize>,
    /// 规范化转换的说明（去重、有序）。
    pub 规范化: BTreeSet<String>,
    /// 被有意忽略的引擎派生组件：名字 → 次数。
    pub 忽略组件: BTreeMap<String, usize>,
    /// 无法转换的组件。非空即代表**有数据损失**。
    pub 未转换组件: Vec<未转换组件>,
    /// 对产出的文档跑一遍 [`场景文档::校验`] 的结果。
    pub 文档校验: 文档校验报告,
}

impl 提取报告 {
    /// 是否存在无法表达的数据。
    pub fn 有数据损失(&self) -> bool {
        !self.未转换组件.is_empty()
    }

    /// 一句话结论。
    pub fn 摘要(&self) -> String {
        let mut 部分 = vec![format!("实体 {} 个", self.实体数)];
        if !self.已转换组件.is_empty() {
            let 组件: Vec<String> = self
                .已转换组件
                .iter()
                .map(|(名字, 次数)| format!("{名字}×{次数}"))
                .collect();
            部分.push(format!("组件 {}", 组件.join("、")));
        }
        if !self.忽略组件.is_empty() {
            let 忽略: Vec<String> = self
                .忽略组件
                .iter()
                .map(|(名字, 次数)| format!("{名字}×{次数}"))
                .collect();
            部分.push(format!("忽略引擎派生 {}", 忽略.join("、")));
        }
        if self.有数据损失() {
            部分.push(format!("⚠ 未转换 {} 个", self.未转换组件.len()));
        } else {
            部分.push("无数据损失".to_string());
        }
        部分.join("；")
    }
}

/// 提取产物：文档 + 报告 + Bevy 实体到文档标识的映射。
#[derive(Debug, Clone)]
pub struct 提取产物 {
    /// 提取出的场景文档。
    pub 文档: 场景文档,
    /// 提取报告。
    pub 报告: 提取报告,
    /// Bevy `Entity` → 文档 `实体标识`。顺序与深度优先遍历一致。
    pub 实体映射: Vec<(Entity, 实体标识)>,
}

impl 提取产物 {
    /// 按 Bevy 实体回查文档标识。
    pub fn 文档标识(&self, 实体: Entity) -> Option<实体标识> {
        self.实体映射
            .iter()
            .find(|(运行时, _)| *运行时 == 实体)
            .map(|(_, 文档)| *文档)
    }
}

/// 把 Bevy 的 `Transform` 转成文档里的 [`变换数据`]。
///
/// 与 `editor_app::scene_bridge::bevy转_变换数据` 做的是同一件事：
/// 归一化四元数后按 `(x, y, z, w)` 存放。之所以在适配层再写一份，
/// 是因为适配层只允许依赖 `scene_doc` 与 Bevy，不能反向依赖 `editor_app`
/// （那会形成环）。两处一致性由 `editor_app` 的集成测试守住。
pub fn bevy变换转文档(变换: &Transform) -> 变换数据 {
    let 旋转 = 变换.rotation.normalize();
    变换数据 {
        平移: (变换.translation.x, 变换.translation.y, 变换.translation.z),
        旋转: (旋转.x, 旋转.y, 旋转.z, 旋转.w),
        缩放: (变换.scale.x, 变换.scale.y, 变换.scale.z),
    }
}

/// 从 `根` 开始，把整个世界里的这棵子树提取成场景文档。
///
/// 这个函数**总是**返回文档（只要根实体存在），即使存在未转换组件 ——
/// 未转换内容会完整列在 [`提取报告::未转换组件`] 里。
/// 想要"有损失就失败"的严格语义，用 [`提取并校验`]。
pub fn 提取文档(
    世界: &World,
    根: Entity,
    场景名: impl Into<String>,
    配置: &提取配置<'_>,
) -> Result<提取产物, BSN错误> {
    if 世界.get_entity(根).is_err() {
        return Err(BSN错误::根实体缺失 {
            根: format!("{根:?}"),
        });
    }

    let 已转换类型 = 配置.已转换类型集合();
    let 名字表 = 配置.名字表();
    let 引擎派生: BTreeSet<TypeId> = 配置.引擎派生组件.iter().map(|类型| 类型.类型标识).collect();

    let mut 文档 = 场景文档::新建(场景名);
    let mut 报告 = 提取报告::default();
    let mut 实体映射: Vec<(Entity, 实体标识)> = Vec::new();

    // 规范化说明：这些是"转换必然带来的表示变化"，必须显式记录。
    报告
        .规范化
        .insert("Transform → 场景实体的 `变换` 字段".to_string());
    报告
        .规范化
        .insert("Children / ChildOf → 场景实体的 `父实体` + `次序`".to_string());

    // 深度优先遍历。栈元素：(Bevy 实体, 父文档标识, 在兄弟中的次序, 深度)。
    // 子实体**逆序压栈**，这样出栈顺序与 `Children` 的声明顺序一致 ——
    // 场景树里的顺序必须和 BSN 里写的顺序相同，否则层级看起来是乱的。
    let mut 待处理: Vec<(Entity, Option<实体标识>, u32, usize)> = vec![(根, None, 0, 0)];
    let mut 自动命名序号 = 0usize;

    while let Some((实体, 父标识, 次序, 深度)) = 待处理.pop() {
        let 标识 = 实体标识::新建();
        实体映射.push((实体, 标识));
        报告.最大深度 = 报告.最大深度.max(深度);

        // ---- 名称 ----
        let 名称 = match 世界.get::<Name>(实体) {
            Some(名字) => {
                报告.有名称的实体 += 1;
                名字.as_str().to_string()
            }
            None => {
                let 名称 = format!("未命名实体{自动命名序号}");
                自动命名序号 += 1;
                报告.自动命名的实体.push(标识);
                报告
                    .规范化
                    .insert("缺少 Name 组件的实体被自动命名为 `未命名实体N`".to_string());
                名称
            }
        };

        // ---- 变换 ----
        let 变换 = 世界.get::<Transform>(实体).map(bevy变换转文档);
        if 变换.is_some() {
            报告.有变换的实体 += 1;
        }

        // ---- 受支持组件 ----
        let mut 组件列表: Vec<组件数据> = Vec::new();
        for 提取器 in &配置.提取器 {
            let Some(实例) = (提取器.取出)(世界, 实体) else {
                continue;
            };
            let 类型 =
                配置
                    .注册表
                    .查询(提取器.组件名)
                    .ok_or_else(|| BSN错误::组件未注册 {
                        实体: 标识,
                        名称: 名称.clone(),
                        组件: 提取器.组件名.to_string(),
                    })?;
            let 属性 = 类型
                .导出(实例.as_ref())
                .ok_or_else(|| BSN错误::组件导出失败 {
                    实体: 标识,
                    名称: 名称.clone(),
                    组件: 提取器.组件名.to_string(),
                })?;
            组件列表.push(组件数据::新建(提取器.组件名, 属性));
            *报告
                .已转换组件
                .entry(提取器.组件名.to_string())
                .or_insert(0) += 1;
        }

        // ---- 组件分类：已转换 / 有意忽略 / 未转换 ----
        if let Ok(迭代) = 世界.inspect_entity(实体) {
            for (_, 信息) in 迭代 {
                let Some(类型标识) = 信息.type_id() else {
                    // 没有对应 Rust 类型的组件（动态组件），无法用 TypeId 归类，
                    // 直接算作未转换，避免它被悄悄跳过。
                    报告.未转换组件.push(未转换组件 {
                        实体: 标识,
                        实体名称: 名称.clone(),
                        类型标识: None,
                    });
                    continue;
                };
                if 已转换类型.contains(&类型标识) {
                    continue;
                }
                if 引擎派生.contains(&类型标识) {
                    let 名字 = 名字表.get(&类型标识).copied().unwrap_or("<引擎派生组件>");
                    *报告.忽略组件.entry(名字.to_string()).or_insert(0) += 1;
                    continue;
                }
                报告.未转换组件.push(未转换组件 {
                    实体: 标识,
                    实体名称: 名称.clone(),
                    类型标识: Some(类型标识),
                });
            }
        }

        // ---- 写入文档 ----
        let mut 实体数据 = 场景实体::新建(名称);
        实体数据.标识 = 标识;
        实体数据.父实体 = 父标识;
        实体数据.次序 = 次序;
        实体数据.变换 = 变换;
        实体数据.组件 = 组件列表;
        文档.插入实体(实体数据);

        // ---- 排入子实体 ----
        let 孩子: Vec<Entity> = 世界
            .get::<Children>(实体)
            .map(|孩子| 孩子.to_vec())
            .unwrap_or_default();
        for (下标, 子) in 孩子.iter().enumerate().rev() {
            待处理.push((*子, Some(标识), 下标 as u32, 深度 + 1));
        }
    }

    报告.实体数 = 文档.实体数量();

    // 组件里可能内嵌资源引用，保存前统一登记，保证资源表完整。
    let 新登记 = 文档.同步资源表();
    if 新登记 > 0 {
        报告
            .规范化
            .insert(format!("从组件属性中登记了 {新登记} 条资源引用"));
    }

    报告.文档校验 = 文档.校验();

    Ok(提取产物 {
        文档,
        报告,
        实体映射,
    })
}

/// 与 [`提取文档`] 相同，但**存在未转换组件时返回错误**。
///
/// 这是给"要把结果当作生产数据"的调用方用的：宁可失败，也不要拿着
/// 一份悄悄少了组件的文档继续走下去。
pub fn 提取并校验(
    世界: &World,
    根: Entity,
    场景名: impl Into<String>,
    配置: &提取配置<'_>,
) -> Result<提取产物, BSN错误> {
    let 产物 = 提取文档(世界, 根, 场景名, 配置)?;
    if 产物.报告.有数据损失() {
        let 示例 = 产物
            .报告
            .未转换组件
            .iter()
            .take(3)
            .map(|项| format!("{} 上的 {}", 项.实体名称, 项.类型文本()))
            .collect::<Vec<_>>()
            .join("；");
        return Err(BSN错误::存在未转换组件 {
            数量: 产物.报告.未转换组件.len(),
            示例,
        });
    }
    Ok(产物)
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::property::属性值;
    use scene_doc::registry::{属性描述, 属性种类, 组件类型};

    // ---- 一个最小的受支持组件，用来验证"注册表驱动提取"这条链路 ----

    /// 编辑器侧的可编辑组件。
    #[derive(Component, Debug, Clone, PartialEq)]
    struct 计分板 {
        分数: i64,
    }

    impl Default for 计分板 {
        fn default() -> Self {
            Self { 分数: 0 }
        }
    }

    /// 该组件的编辑器适配。
    struct 计分板类型;

    fn 计分板字段() -> &'static [属性描述] {
        use std::sync::OnceLock;
        static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
        列表.get_or_init(|| {
            vec![属性描述::新建(
                "分数",
                "分数",
                属性种类::自由整数(),
                属性值::整数(0),
            )]
        })
    }

    impl 组件类型 for 计分板类型 {
        fn 组件名(&self) -> &'static str {
            "计分板"
        }
        fn 默认实例(&self) -> Box<dyn Any> {
            Box::new(计分板::default())
        }
        fn 字段描述(&self) -> &[属性描述] {
            计分板字段()
        }
        fn 导出(&self, 实例: &dyn Any) -> Option<BTreeMap<String, 属性值>> {
            let 数据 = 实例.downcast_ref::<计分板>()?;
            let mut 结果 = BTreeMap::new();
            结果.insert("分数".into(), 属性值::整数(数据.分数));
            Some(结果)
        }
        fn 实例化(&self, 实例: &mut dyn Any, 数据: &BTreeMap<String, 属性值>) -> bool {
            let Some(目标) = 实例.downcast_mut::<计分板>() else {
                return false;
            };
            if let Some(值) = 数据.get("分数").and_then(属性值::取整数) {
                目标.分数 = 值;
            }
            true
        }
    }

    fn 测试注册表() -> 组件注册表 {
        let mut 注册表 = 组件注册表::新建();
        注册表.注册(Box::new(计分板类型)).unwrap();
        注册表
    }

    const 计分板提取器: fn() -> 组件提取器 = || 组件提取器::新建::<计分板>("计分板");

    // ---- 测试 ----

    #[test]
    fn 变换转换保留全部十个分量() {
        let 变换 = Transform {
            translation: Vec3::new(1.0, -2.0, 3.5),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(2.0, 3.0, 4.0),
        };
        let 文档变换 = bevy变换转文档(&变换);
        assert_eq!(文档变换.平移, (1.0, -2.0, 3.5));
        assert_eq!(文档变换.缩放, (2.0, 3.0, 4.0));
        // 旋转必须归一化：长度误差应在极小范围内。
        let (x, y, z, w) = 文档变换.旋转;
        let 模长 = (x * x + y * y + z * z + w * w).sqrt();
        assert!((模长 - 1.0).abs() < 1e-5, "四元数未归一化：模长 {模长}");
        // 绕 z 转 90°，与 Bevy 的结果一致。
        assert!((z - 变换.rotation.z).abs() < 1e-6);
    }

    #[test]
    fn 未转换组件会被记录而不是丢掉() {
        // 一个没有提取器的组件。
        #[derive(Component, Default, Clone)]
        struct 不支持的组件;

        let mut 世界 = World::new();
        let 根 = 世界.spawn((Name::new("甲"), 不支持的组件)).id();

        let 注册表 = 测试注册表();
        let 配置 = 提取配置::新建(&注册表, vec![计分板提取器()]);
        let 产物 = 提取文档(&世界, 根, "测试", &配置).unwrap();

        assert!(产物.报告.有数据损失());
        assert_eq!(产物.报告.未转换组件.len(), 1);
        assert_eq!(
            产物.报告.未转换组件[0].类型标识,
            Some(TypeId::of::<不支持的组件>())
        );
        assert_eq!(产物.报告.未转换组件[0].实体名称, "甲");

        // 严格入口必须拒绝。
        let 错误 = 提取并校验(&世界, 根, "测试", &配置).unwrap_err();
        match 错误 {
            BSN错误::存在未转换组件 { 数量, 示例 } => {
                assert_eq!(数量, 1);
                assert!(示例.contains("甲"), "错误信息应能定位到实体：{示例}");
            }
            其他 => panic!("期望存在未转换组件的错误，实际：{其他:?}"),
        }
    }

    #[test]
    fn 注册表缺组件时拒绝导出() {
        let mut 世界 = World::new();
        let 根 = 世界.spawn(计分板 { 分数: 7 }).id();

        // 注册表为空，但提取器声明了 `计分板`。
        let 注册表 = 组件注册表::新建();
        let 配置 = 提取配置::新建(&注册表, vec![计分板提取器()]);
        match 提取文档(&世界, 根, "测试", &配置) {
            Err(BSN错误::组件未注册 { 组件, .. }) => assert_eq!(组件, "计分板"),
            其他 => panic!("期望组件未注册错误，实际：{其他:?}"),
        }
    }

    #[test]
    fn 缺少名称时自动命名并记录规范化() {
        let mut 世界 = World::new();
        let 根 = 世界.spawn(计分板 { 分数: 1 }).id();

        let 注册表 = 测试注册表();
        let 配置 = 提取配置::新建(&注册表, vec![计分板提取器()]);
        let 产物 = 提取文档(&世界, 根, "测试", &配置).unwrap();

        let 唯一 = 产物.文档.实体表.values().next().unwrap();
        assert_eq!(唯一.名称, "未命名实体0");
        assert_eq!(产物.报告.自动命名的实体.len(), 1);
        assert!(
            产物
                .报告
                .规范化
                .iter()
                .any(|说明| 说明.contains("自动命名")),
            "自动命名必须记进规范化说明"
        );
    }

    #[test]
    fn 根实体不存在时返回错误() {
        let 世界 = World::new();
        let 注册表 = 测试注册表();
        let 配置 = 提取配置::新建(&注册表, vec![]);
        // 空世界里随便取一个实体标识。
        let 幽灵 = Entity::from_raw_u32(12345).unwrap();
        match 提取文档(&世界, 幽灵, "测试", &配置) {
            Err(BSN错误::根实体缺失 { .. }) => {}
            其他 => panic!("期望根实体缺失错误，实际：{其他:?}"),
        }
    }

    #[test]
    fn 引擎派生组件默认被忽略且不算数据损失() {
        let mut 世界 = World::new();
        // 插入 Transform 会自动补齐 GlobalTransform。
        let 根 = 世界.spawn((Name::new("乙"), Transform::default())).id();
        // 手动补上变换传播标记，模拟真实场景。
        世界.entity_mut(根).insert(TransformTreeChanged);

        let 注册表 = 测试注册表();
        let 配置 = 提取配置::新建(&注册表, vec![]);
        let 产物 = 提取文档(&世界, 根, "测试", &配置).unwrap();

        assert!(!产物.报告.有数据损失());
        assert!(产物.报告.忽略组件.contains_key("GlobalTransform"));
        assert!(产物.报告.忽略组件.contains_key("TransformTreeChanged"));
        // 忽略的组件不应出现在文档实体上。
        let 唯一 = 产物.文档.实体表.values().next().unwrap();
        assert!(!唯一.有组件("GlobalTransform"));
        // 但变换字段必须有值。
        assert!(唯一.变换.is_some());
    }

    #[test]
    fn 提取出的文档通过自身校验() {
        let mut 世界 = World::new();
        let 根 = 世界.spawn(Name::new("父")).id();
        let 子 = 世界.spawn(Name::new("子")).id();
        世界.entity_mut(子).insert(ChildOf(根));

        let 注册表 = 测试注册表();
        let 配置 = 提取配置::新建(&注册表, vec![]);
        let 产物 = 提取文档(&世界, 根, "测试", &配置).unwrap();

        assert!(
            产物.报告.文档校验.无问题(),
            "提取出的文档必须自洽：{}",
            产物.报告.文档校验.摘要()
        );
    }
}
