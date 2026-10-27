//! 场景文档 ⇄ Bevy ECS 的桥接层。
//!
//! # 这一层要解决的核心矛盾
//!
//! 需求里有一条硬约束：
//!
//! > 编辑器文档数据与运行时 ECS 世界不能简单地混为一体。
//!
//! 但编辑器又必须能把场景**实际内容**反映到运行中的游戏里。这两件事
//! 看起来冲突，实际的解法是把方向分清楚：
//!
//! ```text
//! 场景文档  --[构建世界]-->  Bevy World（运行时快照）
//!     ^                            |
//!     |                            |  只读地取统计数据、组件清单
//!     +-------- 绝不自动回写 -------+
//! ```
//!
//! * **文档 → 世界**：只在"开始运行"时发生一次（[`构建运行时世界`]）。
//! * **世界 → 文档**：**默认不存在**。运行时对实体和组件的任何修改都
//!   停留在那个 `World` 里，停止运行时整个 `World` 被丢弃。
//!   想写回必须由用户显式触发（本期未提供该入口，见功能路线图）。
//!
//! 这样"运行时乱改不污染编辑器"不是靠约定或小心，而是靠**数据被物理隔离**
//! ——两个世界，两份数据，中间只有一条单向通道。

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use scene_doc::document::{变换数据, 场景文档, 实体标识};
use scene_doc::property::{属性值, 颜色值};
use scene_doc::registry::{属性描述, 属性种类, 组件注册表, 组件类型};

/// Bevy 世界的 `Entity` 与场景实体标识之间的映射。
///
/// 编辑器需要在两边来回定位（例如"选中场景树里的某项，高亮运行时对应的实体"），
/// 因此这份映射是桥接层的核心产物。
#[derive(Resource, Debug, Default)]
pub struct 实体映射表 {
    /// 场景标识 → 运行时实体。
    到运行时: HashMap<实体标识, Entity>,
    /// 运行时实体 → 场景标识。
    到文档: HashMap<Entity, 实体标识>,
}

impl 实体映射表 {
    /// 新建空映射表。
    pub fn 新建() -> Self {
        Self::default()
    }

    /// 记录一条映射。
    pub fn 插入(&mut self, 文档标识: 实体标识, 运行时实体: Entity) {
        self.到运行时.insert(文档标识, 运行时实体);
        self.到文档.insert(运行时实体, 文档标识);
    }

    /// 由文档标识查运行时实体。
    pub fn 取运行时(&self, 文档标识: 实体标识) -> Option<Entity> {
        self.到运行时.get(&文档标识).copied()
    }

    /// 由运行时实体查文档标识。
    pub fn 取文档(&self, 运行时实体: Entity) -> Option<实体标识> {
        self.到文档.get(&运行时实体).copied()
    }

    /// 映射数量。
    pub fn 数量(&self) -> usize {
        self.到运行时.len()
    }

    /// 清空。
    pub fn 清空(&mut self) {
        self.到运行时.clear();
        self.到文档.clear();
    }
}

/// 构建运行时世界的统计结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct 构建统计 {
    /// 创建的实体数。
    pub 实体数: usize,
    /// 成功应用的组件数。
    pub 组件数: usize,
    /// 因未注册组件类型而跳过的组件数。
    pub 跳过组件数: usize,
    /// 被跳过的组件类型名（去重后按名排序）。
    pub 跳过类型: Vec<String>,
    /// 建立父子关系的数量。
    pub 父子关系数: usize,
}

/// 运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 运行状态 {
    /// 未运行，处于编辑状态。
    #[default]
    已停止,
    /// 正在运行。
    运行中,
    /// 已暂停。
    已暂停,
}

impl 运行状态 {
    /// 是否处于运行中（含暂停）。
    pub fn 在运行(&self) -> bool {
        !matches!(self, 运行状态::已停止)
    }
}

/// 运行无法开始的原因。
///
/// # 为什么这里只有一个变体
///
/// `构建世界` 对"数据不完整"是**降级**而不是失败：未注册的组件类型跳过并计数、
/// 悬空父实体保持为顶层、缺失的变换按单位处理。这些情况都应当照常进入运行态，
/// 让用户能在游戏里看出问题出在哪（`运行场景` 会把跳过的类型逐条写进控制台）。
///
/// 真正**不该**进入运行态的情形只有一种：场景里一个实体都没有。
/// 那时构建出的空世界与"运行成功但什么都没画"在界面上无法区分。
/// 因此把它建模成错误，由调用方回复提示，而不是让用户对着空视口猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 运行时错误 {
    /// 场景里没有任何实体。
    场景为空,
}

impl 运行时错误 {
    /// 面向用户的词条键；参数由调用方补充。
    pub fn 词条键(&self) -> &'static str {
        match self {
            Self::场景为空 => "log_run_scene_empty",
        }
    }
}

/// 编辑器持有的运行时宿主。
///
/// # 为什么它持有一个独立的 `World`
///
/// 把运行时世界放在**自己**的 `World` 里，而不是往编辑器主世界里塞实体，
/// 是这套隔离方案的关键。主世界始终只包含编辑器界面相关的东西；
/// 运行时世界里发生什么都不会碰到主世界的实体表，
/// 反过来也一样。停止运行时把 `世界` 置空，运行时数据即刻整体消失。
#[derive(Resource)]
pub struct 运行时宿主 {
    /// 当前运行状态。
    pub 状态: 运行状态,
    /// 运行时世界。停止时为 `None`。
    pub 世界: Option<World>,
    /// 实体映射表。
    pub 映射: 实体映射表,
    /// 本次运行启动时的场景文档快照（用于"重新运行"与调试对比）。
    pub 启动快照: Option<场景文档>,
    /// 上次构建统计。
    pub 上次统计: 构建统计,
    /// 本次运行开始的时刻。
    pub 启动时刻: Option<std::time::Instant>,
    /// 已推进的帧数。
    pub 已推进帧数: u64,
    /// 上一帧耗时（毫秒）。
    pub 上一帧毫秒: f32,
}

impl Default for 运行时宿主 {
    fn default() -> Self {
        Self {
            状态: 运行状态::已停止,
            世界: None,
            映射: 实体映射表::新建(),
            启动快照: None,
            上次统计: 构建统计::default(),
            启动时刻: None,
            已推进帧数: 0,
            上一帧毫秒: 0.0,
        }
    }
}

impl 运行时宿主 {
    /// 从场景文档启动运行时世界。
    ///
    /// # 失败时不改动任何状态
    ///
    /// 返回 `Err` 时 `世界`/`映射`/`启动快照`/`状态` 全部保持调用前的值。
    /// 调用方（`UiAction::运行场景`）据此决定提示什么；编辑器继续停在
    /// 原来的模式，用户可以补数据后直接重试，不需要先"停止运行"。
    pub fn 启动(
        &mut self, 文档: &场景文档, 注册表: &组件注册表
    ) -> Result<(), 运行时错误> {
        if 文档.实体表.is_empty() {
            return Err(运行时错误::场景为空);
        }
        let mut 世界 = World::new();
        世界.insert_resource(Time::<()>::default());

        let (统计, 映射) = 构建世界(&mut 世界, 文档, 注册表);

        self.世界 = Some(世界);
        self.映射 = 映射;
        self.启动快照 = Some(文档.clone());
        self.上次统计 = 统计;
        self.状态 = 运行状态::运行中;
        self.启动时刻 = Some(std::time::Instant::now());
        self.已推进帧数 = 0;
        Ok(())
    }

    /// 暂停。
    pub fn 暂停(&mut self) {
        if self.状态 == 运行状态::运行中 {
            self.状态 = 运行状态::已暂停;
        }
    }

    /// 继续。
    pub fn 继续(&mut self) {
        if self.状态 == 运行状态::已暂停 {
            self.状态 = 运行状态::运行中;
        }
    }

    /// 停止并丢弃运行时世界。
    ///
    /// 丢弃是整个隔离方案的最后一步：运行时的所有修改随 `World` 一起消失，
    /// 不可能有机会回写编辑器文档。
    pub fn 停止(&mut self) {
        self.世界 = None;
        self.映射.清空();
        self.状态 = 运行状态::已停止;
        self.启动时刻 = None;
        self.已推进帧数 = 0;
    }

    /// 已运行秒数。
    pub fn 已运行秒数(&self) -> f32 {
        self.启动时刻
            .map(|时刻| 时刻.elapsed().as_secs_f32())
            .unwrap_or(0.0)
    }

    /// 运行时世界里的**场景实体**数量。
    ///
    /// 这里刻意不直接返回 `entities().count_spawned()`。实测表明
    /// `World::new()` 本身就会创建 2 个不带任何组件的内部实体
    /// （索引 0 与 1，没有 `Name`），它们属于引擎基础设施而不是场景内容。
    /// 把它们算进来会让诊断面板显示"空场景也有 2 个实体"，
    /// 也会让"场景里有 N 个实体，运行时就有 N 个"这条断言失去意义。
    ///
    /// 因此以"是否带 `Name`"作为判据：`构建世界` 会给**每一个**
    /// 由场景文档创建出来的实体都插入 `Name`，所以这个判据与
    /// 场景实体的定义严格一致。
    pub fn 运行时实体数(&self) -> usize {
        self.世界
            .as_ref()
            .map(|世界| {
                世界
                    .iter_entities()
                    .filter(|引用| 引用.contains::<Name>())
                    .count()
            })
            .unwrap_or(0)
    }

    /// 运行时世界里的组件类型数量。
    pub fn 运行时组件类型数(&self) -> usize {
        self.世界
            .as_ref()
            .map(|世界| 世界.components().len())
            .unwrap_or(0)
    }
}

/// 把一个场景文档构建进给定世界。
///
/// 返回 `(统计, 映射表)`。这是一个**纯函数式**的操作：只读文档，
/// 只写传入的世界，不触碰编辑器状态，因此可以脱离 Bevy App 单独测试
/// （只需构造一个 `World`）。
///
/// 实现分两趟：
///
/// 1. 第一趟创建所有实体并插入 `Transform`，同时记录映射；
/// 2. 第二趟按文档里的父关系建立 `ChildOf`。
///
/// 分两趟是必须的：建立父子关系时父实体必须已经存在，
/// 而文档里的实体顺序不保证父在子之前（`BTreeMap` 按 UUID 排序）。
pub fn 构建世界(
    世界: &mut World,
    文档: &场景文档,
    注册表: &组件注册表,
) -> (构建统计, 实体映射表) {
    let mut 统计 = 构建统计::default();
    let mut 映射 = 实体映射表::新建();
    let mut 待建父子: Vec<(实体标识, 实体标识)> = Vec::new();

    // ---- 第一趟：实体与组件 ----
    // 按场景树的顺序创建，让运行时世界的实体顺序与编辑器里看到的一致，
    // 调试时更容易对上。
    let 顺序: Vec<实体标识> = 文档.文档遍历();
    // 深度优先遍历会跳过损坏文档中的孤立实体（父实体不存在、又不在顶层顺序里），
    // 这里补齐，保证"文档里有多少实体，运行时就有多少实体"。
    let mut 全部标识 = 顺序.clone();
    for 标识 in 文档.实体表.keys() {
        if !全部标识.contains(标识) {
            全部标识.push(*标识);
        }
    }

    let mut 跳过的类型: std::collections::BTreeSet<String> = Default::default();

    for 标识 in &全部标识 {
        let Some(实体数据) = 文档.取(*标识) else {
            continue;
        };

        let 运行时实体 = 世界.spawn_empty().id();
        映射.插入(*标识, 运行时实体);
        统计.实体数 += 1;

        // Name：让调试器与日志里能看到有意义的实体名
        世界
            .entity_mut(运行时实体)
            .insert(Name::new(实体数据.名称.clone()));

        // Transform：文档里的变换直接映射到 Bevy 的 Transform
        if let Some(变换) = 实体数据.变换 {
            世界.entity_mut(运行时实体).insert(变换数据转_bevy(变换));
        }

        // 其余组件按注册表逐个应用
        for 组件 in &实体数据.组件 {
            let Some(类型) = 注册表.查询(&组件.类型名) else {
                统计.跳过组件数 += 1;
                跳过的类型.insert(组件.类型名.clone());
                continue;
            };
            match 实例化并插入(世界, 运行时实体, 类型, &组件.属性) {
                true => 统计.组件数 += 1,
                false => {
                    统计.跳过组件数 += 1;
                    跳过的类型.insert(组件.类型名.clone());
                }
            }
        }

        // 记录父子关系，第二趟再建立
        if let Some(父) = 实体数据.父实体 {
            待建父子.push((*标识, 父));
        }
    }

    // ---- 第二趟：父子关系 ----
    for (子, 父) in 待建父子 {
        let (Some(父实体), Some(子实体)) = (映射.取运行时(父), 映射.取运行时(子))
        else {
            // 父实体不在映射表里，说明文档里有悬空父实体。
            // 这种情况下保持子实体为顶层实体，而不是丢弃它 ——
            // 丢掉会让"场景里有 10 个实体，运行时只有 9 个"难以解释。
            continue;
        };
        世界.entity_mut(子实体).insert(ChildOf(父实体));
        统计.父子关系数 += 1;
    }

    统计.跳过类型 = 跳过的类型.into_iter().collect();
    (统计, 映射)
}

/// 把属性表实例化成具体组件并插入实体。
///
/// 返回是否成功。失败（类型不匹配或字段非法）时会返回 `false`
/// 并由调用方计入统计，而不是 panic —— 一个字段写错不应该让整个游戏起不来。
fn 实例化并插入(
    世界: &mut World,
    运行时实体: Entity,
    类型: &dyn 组件类型,
    属性: &std::collections::BTreeMap<String, 属性值>,
) -> bool {
    let mut 实例 = 类型.默认实例();
    if !类型.实例化(实例.as_mut(), 属性) {
        return false;
    }
    // 目前注册的组件类型都是 `Component`，用 downcast 取回具体类型。
    // 这里对每种受支持的组件逐个尝试，是"类型擦除后还原"的必要代价；
    // 新增组件时在这里加一行即可（`注册并支持` 保证不会漏）。
    插入已知组件(世界, 运行时实体, 实例.as_ref())
}

/// 把装箱的组件实例插入实体。
///
/// # 为什么必须覆盖注册表里的**每一个**类型
///
/// [`组件类型::实例化`] 负责把属性表写进那个擦除类型的实例，
/// 而这里负责把实例真正插进世界。两者必须一一对应：
/// 如果注册表里有某个类型而这里漏了分支，结果不是编译错误，
/// 而是"注册表明明有、属性也写进去了、但组件根本没进世界"——
/// 统计上它会被算作"跳过"，运行时的表现是这个组件彻底无效。
///
/// 底部的测试 `注册表与插入分支一一对应` 专门守住这条不变量。
fn 插入已知组件(
    世界: &mut World, 运行时实体: Entity, 实例: &dyn std::any::Any
) -> bool {
    // 变换
    if let Some(值) = 实例.downcast_ref::<Transform>() {
        世界.entity_mut(运行时实体).insert(*值);
        return true;
    }
    // 可见性：文档里存的是简化后的布尔，这里映射回 Bevy 的枚举。
    //
    // `true → Inherited`（跟随父级，Bevy 的常规默认），`false → Hidden`。
    // 这个映射是**有损**的：原始为 `Visible` 的会变成 `Inherited`。
    // 选择有损而不是在文档里引入三态，是因为对绝大多数 2D/3D 游戏
    // 而言二者等价，而三态会让用户每次都要停下来想"该选哪个"。
    // 这一限制写在字段提示里，用户能看到。
    if let Some(值) = 实例.downcast_ref::<可见性属性>() {
        世界.entity_mut(运行时实体).insert(if 值.可见 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        return true;
    }
    // 自定义：Sprite 属性载体
    if let Some(值) = 实例.downcast_ref::<Sprite属性>() {
        世界.entity_mut(运行时实体).insert(值.clone());
        return true;
    }
    // 自定义：2D 相机
    if let Some(值) = 实例.downcast_ref::<编辑器相机2d>() {
        世界.entity_mut(运行时实体).insert(*值);
        return true;
    }
    // 自定义：程序化瓦片地形
    if let Some(值) = 实例.downcast_ref::<瓦片地形属性>() {
        世界.entity_mut(运行时实体).insert(值.clone());
        return true;
    }
    // 自定义：程序化三维几何体
    if let Some(值) = 实例.downcast_ref::<几何体属性>() {
        世界.entity_mut(运行时实体).insert(值.clone());
        return true;
    }
    false
}

/// 场景文档里的 [`变换数据`] 转 Bevy 的 `Transform`。
pub fn 变换数据转_bevy(变换: 变换数据) -> Transform {
    Transform {
        translation: Vec3::new(变换.平移.0, 变换.平移.1, 变换.平移.2),
        // 文档里存的是 (x, y, z, w)，Bevy 的 Quat::from_xyzw 也是同样顺序。
        // 长度可能因外部编辑而偏离 1，这里归一化一次。
        rotation: {
            let (x, y, z, w) = 变换.旋转;
            let 四元数 = Quat::from_xyzw(x, y, z, w);
            if 四元数.length_squared() > f32::EPSILON {
                四元数.normalize()
            } else {
                Quat::IDENTITY
            }
        },
        scale: Vec3::new(变换.缩放.0, 变换.缩放.1, 变换.缩放.2),
    }
}

/// Bevy 的 `Transform` 转场景文档的 [`变换数据`]。
pub fn bevy转_变换数据(变换: &Transform) -> 变换数据 {
    let 旋转 = 变换.rotation.normalize();
    变换数据 {
        平移: (变换.translation.x, 变换.translation.y, 变换.translation.z),
        旋转: (旋转.x, 旋转.y, 旋转.z, 旋转.w),
        缩放: (变换.scale.x, 变换.scale.y, 变换.scale.z),
    }
}

// ======================================================================
// 编辑器内置组件类型
// ======================================================================
//
// 这些实现是"注册机制可用"的证明：编辑器不认识任何组件的类型，
// 全靠下面这些描述 + 转换。Bevy 自带的组件（Transform、Visibility）
// 与项目自定义组件（Sprite属性、编辑器相机2d）走的是**完全相同**的路径。

/// 属性描述构造辅助：浮点。
fn 浮点(
    字段: &str,
    显示: &'static str,
    默认: f32,
    最小: f32,
    最大: f32,
    步进: f32,
) -> 属性描述 {
    属性描述::新建(
        字段,
        显示,
        属性种类::浮点 {
            最小: Some(最小),
            最大: Some(最大),
            步进,
        },
        属性值::浮点(默认),
    )
}

/// 属性描述构造辅助：布尔。
fn 布尔(字段: &str, 显示: &'static str, 默认: bool) -> 属性描述 {
    属性描述::新建(字段, 显示, 属性种类::布尔, 属性值::布尔(默认))
}

/// 属性描述构造辅助：二维向量。
fn 二维(字段: &str, 显示: &'static str, 默认: (f32, f32)) -> 属性描述 {
    属性描述::新建(
        字段,
        显示,
        属性种类::二维 { 步进: 0.5 },
        属性值::二维(默认.0, 默认.1),
    )
}

/// 属性描述构造辅助：颜色。
fn 颜色(字段: &str, 显示: &'static str, 默认: 颜色值) -> 属性描述 {
    属性描述::新建(字段, 显示, 属性种类::颜色, 属性值::颜色(默认))
}

/// 属性描述构造辅助：资源。
fn 资源(
    字段: &str, 显示: &'static str, 类别: scene_doc::property::资源类别
) -> 属性描述 {
    属性描述::新建(字段, 显示, 属性种类::资源 { 类别 }, 属性值::空)
}

/// 属性描述构造辅助：字符串。当前内置组件尚未用到，保留供后续组件使用。
#[allow(dead_code)]
fn 文本(字段: &str, 显示: &'static str, 默认: &str) -> 属性描述 {
    属性描述::新建(字段, 显示, 属性种类::文本, 属性值::文本(默认.to_string()))
}

/// Bevy 的 `Visibility` 在编辑器里的表示。
///
/// Bevy 的 `Visibility` 是枚举（`Inherited` / `Hidden` / `Visible`）。
/// 编辑器把它简化为一个布尔"可见"，因为对绝大多数 2D/3D 游戏来说
/// "继承"与"可见"的差别没有实际意义，而三个选项会让用户困惑。
/// 导回 Bevy 时 `true → Inherited`、`false → Hidden`，
/// 这是**有损**的：原始是 `Visible` 的会变成 `Inherited`。
/// 这一点在组件提示里写明了。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct 可见性属性 {
    /// 是否可见。
    pub 可见: bool,
}

impl Default for 可见性属性 {
    fn default() -> Self {
        Self { 可见: true }
    }
}

/// 组件的 `Sprite` 在编辑器文档里的表示。
///
/// 不直接序列化 Bevy 的 `Sprite`：它的字段包含 `Handle<Image>`、
/// `Anchor`、`ImageScaleMode` 等，既有资源句柄（不能进文档）又有
/// 版本差异大的内部结构。这里只保留**编辑器真正要编辑**的部分，
/// 其余交给运行时用默认值。这正体现了"文档格式独立于引擎类型"的价值。
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Sprite属性 {
    /// 贴图资源引用（可能是相对路径文本）。
    pub 贴图路径: Option<scene_doc::path::项目相对路径>,
    /// 着色。
    pub 颜色: 颜色值,
    /// 是否水平翻转。
    pub 水平翻转: bool,
    /// 是否垂直翻转。
    pub 垂直翻转: bool,
    /// 自定义尺寸。`None` 表示用贴图原始尺寸。
    pub 自定义尺寸: Option<(f32, f32)>,
    /// glTF/GLB 模型的项目相对路径。
    ///
    /// 与 `贴图路径` 互斥：有模型时预览按模型渲染（真实网格），
    /// 否则按贴图精灵渲染。放在同一个组件里而不是新造一个组件，
    /// 是因为两者在编辑器里就是"这个实体长什么样"的两种取值。
    pub 模型路径: Option<scene_doc::path::项目相对路径>,
    /// 材质文档（`.material.ron`）的项目相对路径。
    ///
    /// 未引用时实体外观与本字段引入前完全一致，因此旧场景不需要迁移。
    /// 引用了材质不等于放弃自己的贴图：材质的表面参数（粗糙度、金属度、
    /// 自发光、双面）叠在实体上，实体自己的贴图在材质没带贴图时生效。
    /// 归属规则见 [`crate::preview::材质转标准材质`]。
    pub 材质路径: Option<scene_doc::path::项目相对路径>,
}

impl Default for Sprite属性 {
    fn default() -> Self {
        Self {
            贴图路径: None,
            颜色: 颜色值::白(),
            水平翻转: false,
            垂直翻转: false,
            自定义尺寸: None,
            模型路径: None,
            材质路径: None,
        }
    }
}

/// 2D 编辑器相机标记。
///
/// 目前只记录"这个实体是一台相机"，用于在运行时世界里标记相机实体。
/// 真正的相机参数（正交尺寸、视口）尚未纳入文档格式，属于路线图项。
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct 编辑器相机2d {
    /// 正交投影的垂直半高。
    pub 正交半高: f32,
}

impl Default for 编辑器相机2d {
    fn default() -> Self {
        Self {
            正交半高: 360.0
        }
    }
}

/// 程序化瓦片地形在编辑器文档里的表示。
///
/// # 为什么不是一张烘焙好的 PNG
///
/// 原项目的 2D 地面是**双网格自动瓦片**：逻辑格只存地形，视觉瓦片按四角
/// 掩码去 `透明到X.png` 图集里取 16px 瓦片，再按材质分层合成。它不是一个
/// 美术预先拼好的整图 —— 同一份数据换一套图集就是另一套画面。
///
/// 如果把整张地图烘焙成 PNG 再塞进场景，丢掉的正是这套系统：
/// 改一格地形、换一套图集、调整变体概率都做不到了，而且图会随分辨率变糊。
/// 所以这里存的是**参数与数据路径**，由预览在加载时按原项目规格程序化合成。
#[derive(Component, Debug, Clone, PartialEq)]
pub struct 瓦片地形属性 {
    /// 地形数据文件（项目相对路径，RON）。字段见 `assets/.../*.terrain.ron`。
    pub 地形文件: String,
    /// 瓦片图集目录（项目相对路径）。目录内按固定文件名放 10 张 64×64 图集
    /// 与 1 张 112×96 变体表，文件名见 [`crate::tile_terrain`]。
    pub 图集目录: String,
    /// 通用瓦片地图文件（`.tilemap.ron`，项目相对路径）。
    ///
    /// 非空时走**资产驱动**渲染：瓦片集由开发者自己定义，不再依赖固定文件名。
    /// 为空时回退到 `地形文件` + `图集目录` 的旧路径。
    pub 地图文件: String,
    /// 通用瓦片集文件（`.tileset.ron`，项目相对路径）。
    ///
    /// 为空时使用地图文件里记录的 `瓦片集` 路径。
    pub 瓦片集: String,
    /// 地形在世界里的显示尺寸（像素）。渲染结果按这个尺寸铺开，
    /// 视口包围框也用它 —— 改这里等于整体缩放地形。
    pub 显示尺寸: (f32, f32),
    /// 每个逻辑格烘焙成多少像素。16 = 原生尺寸，64 = 默认（清晰）。
    pub 每格像素: u32,
    /// 整格瓦片抽到手绘变体的概率（原项目 0.35）。
    pub 变体概率: f32,
}

impl Default for 瓦片地形属性 {
    fn default() -> Self {
        Self {
            地形文件: String::new(),
            图集目录: "assets/terrain/tiles".to_string(),
            地图文件: String::new(),
            瓦片集: String::new(),
            显示尺寸: (1024.0, 1024.0),
            每格像素: 64,
            变体概率: 0.35,
        }
    }
}

/// 三维程序化几何体的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 几何体形状 {
    /// 长方体（地面、围墙、箱体）。
    立方体,
    /// 水平面（法线 +Y，贴花、地面薄片）。
    平面,
}

impl 几何体形状 {
    /// 场景文件里保存的稳定变体名。
    pub fn 变体名(self) -> &'static str {
        match self {
            Self::立方体 => "Cuboid",
            Self::平面 => "Plane",
        }
    }

    /// 变体序号（名字失配时的兜底）。
    pub fn 序号(self) -> u32 {
        match self {
            Self::立方体 => 0,
            Self::平面 => 1,
        }
    }

    /// 从名字 + 序号恢复。未知名字按序号兜底；两者都未知时退化为立方体。
    pub fn 由值(名: &str, 序号: u32) -> Self {
        match 名 {
            "Cuboid" => Self::立方体,
            "Plane" => Self::平面,
            _ if 序号 == 1 => Self::平面,
            _ => Self::立方体,
        }
    }
}

/// 程序化三维几何体在编辑器文档里的表示。
///
/// # 为什么需要它
///
/// 原 3D 项目的竞技场不是"几张平面贴图"：地面是 48×48×0.6 的长方体，
/// 四面围墙是 48×8×0.6 的实心墙，材质是受光的 PBR，纹理按 UV 重复。
/// 编辑器的 `Sprite` 只能画一个 unlit 四边形，做不出厚度、重复纹理、
/// 受光与阴影 —— 3D 示例于是必然看起来像"纸片搭的舞台"。
///
/// 这个组件把原项目的几何/材质描述搬进文档：形状、尺寸、贴图、UV 重复、
/// 是否受光、是否双面。预览据此生成真正的 `Mesh3d` + `StandardMaterial`。
#[derive(Component, Debug, Clone, PartialEq)]
pub struct 几何体属性 {
    /// 几何形状。
    pub 形状: 几何体形状,
    /// 世界尺寸 `(x, y, z)`。立方体是全长，平面用 `(x, z)`。
    pub 尺寸: (f32, f32, f32),
    /// 贴图资源引用。
    pub 贴图路径: Option<scene_doc::path::项目相对路径>,
    /// 基础色/着色。
    pub 颜色: 颜色值,
    /// 纹理在 UV 上的重复次数 `(u, v)`。
    pub 贴图重复: (f32, f32),
    /// 是否受光。地面/墙面应为 true，UI 类薄片可关掉。
    pub 受光: bool,
    /// 是否双面可见。薄墙、单面贴花用得到。
    pub 双面: bool,
    /// 材质文档（`.material.ron`）的项目相对路径。
    ///
    /// 表面参数（粗糙度、金属度、自发光、双面、贴图）由材质提供；
    /// 实体自己保留形状、尺寸、`受光` 与 UV 重复。归属规则见
    /// [`crate::preview::材质转标准材质`]。
    pub 材质路径: Option<scene_doc::path::项目相对路径>,
}

impl Default for 几何体属性 {
    fn default() -> Self {
        Self {
            形状: 几何体形状::立方体,
            尺寸: (1.0, 1.0, 1.0),
            贴图路径: None,
            颜色: 颜色值::白(),
            贴图重复: (1.0, 1.0),
            受光: true,
            双面: false,
            材质路径: None,
        }
    }
}

/// `Transform` 的编辑器适配。
struct 变换组件类型;

impl 组件类型 for 变换组件类型 {
    fn 组件名(&self) -> &'static str {
        "Transform"
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(Transform::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        变换字段描述()
    }

    fn 导出(
        &self,
        实例: &dyn std::any::Any,
    ) -> Option<std::collections::BTreeMap<String, 属性值>> {
        let 变换 = 实例.downcast_ref::<Transform>()?;
        let 数据 = bevy转_变换数据(变换);
        let mut 结果 = std::collections::BTreeMap::new();
        结果.insert(
            "平移".to_string(),
            属性值::三维(数据.平移.0, 数据.平移.1, 数据.平移.2),
        );
        结果.insert(
            "旋转".to_string(),
            属性值::三维(数据.旋转.0, 数据.旋转.1, 数据.旋转.2),
        );
        结果.insert(
            "缩放".to_string(),
            属性值::三维(数据.缩放.0, 数据.缩放.1, 数据.缩放.2),
        );
        Some(结果)
    }

    fn 实例化(
        &self,
        实例: &mut dyn std::any::Any,
        数据: &std::collections::BTreeMap<String, 属性值>,
    ) -> bool {
        let Some(变换) = 实例.downcast_mut::<Transform>() else {
            return false;
        };
        if let Some((x, y, z)) = 数据.get("平移").and_then(属性值::取三维) {
            变换.translation = Vec3::new(x, y, z);
        }
        // 旋转字段在文档里以四元数分量存储（见 变换字段描述）。
        // 它被标记为只读，因此正常编辑路径不会走到这里；
        // 但从文件加载原始数据时仍需要正确还原。
        if let Some((x, y, z, w)) = 取四维(数据.get("旋转")) {
            let 候选 = Quat::from_xyzw(x, y, z, w);
            if 候选.length_squared() > f32::EPSILON {
                变换.rotation = 候选.normalize();
            }
        }
        if let Some((x, y, z)) = 数据.get("缩放").and_then(属性值::取三维) {
            变换.scale = Vec3::new(x, y, z);
        }
        true
    }
}

/// 从属性值里取四维分量。
///
/// 文档里的旋转以 `属性值::四维` 存储；但手写场景文件时用户可能写成
/// 三维（省略 w）或者把 w 写成 1。两种情况都要能读，否则一个笔误
/// 就会让整份场景加载失败。
fn 取四维(值: Option<&属性值>) -> Option<(f32, f32, f32, f32)> {
    match 值? {
        属性值::四维(x, y, z, w) => Some((*x, *y, *z, *w)),
        属性值::三维(x, y, z) => Some((*x, *y, *z, 1.0)),
        _ => None,
    }
}

/// 变换字段描述。
fn 变换字段描述() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "平移",
                "transform_field_translation",
                属性种类::三维默认(),
                属性值::三维(0.0, 0.0, 0.0),
            )
            .带提示("transform_translation_tooltip"),
            属性描述::新建(
                "旋转",
                "transform_field_rotation_quat",
                属性种类::四维 { 步进: 0.01 },
                属性值::四维(0.0, 0.0, 0.0, 1.0),
            )
            .带提示("transform_rotation_tooltip")
            .设只读(),
            属性描述::新建(
                "缩放",
                "transform_field_scale",
                属性种类::三维默认(),
                属性值::三维(1.0, 1.0, 1.0),
            )
            .带提示("transform_scale_tooltip"),
        ]
    })
}

/// `Visibility` 的编辑器适配。
struct 可见性组件类型;

impl 组件类型 for 可见性组件类型 {
    fn 组件名(&self) -> &'static str {
        "Visibility"
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(可见性属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        可见性字段描述()
    }

    fn 导出(
        &self,
        实例: &dyn std::any::Any,
    ) -> Option<std::collections::BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<可见性属性>()?;
        let mut 结果 = std::collections::BTreeMap::new();
        结果.insert("可见".to_string(), 属性值::布尔(属性.可见));
        Some(结果)
    }

    fn 实例化(
        &self,
        实例: &mut dyn std::any::Any,
        数据: &std::collections::BTreeMap<String, 属性值>,
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<可见性属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("可见").and_then(属性值::取布尔) {
            属性.可见 = 值;
        }
        true
    }
}

/// 可见性字段描述。
fn 可见性字段描述() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![布尔("可见", "visibility_field_visible", true).带提示("visibility_field_tooltip")]
    })
}

/// `Sprite属性` 的编辑器适配。
struct Sprite组件类型;

impl 组件类型 for Sprite组件类型 {
    fn 组件名(&self) -> &'static str {
        "Sprite"
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(Sprite属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        sprite字段描述()
    }

    fn 导出(
        &self,
        实例: &dyn std::any::Any,
    ) -> Option<std::collections::BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<Sprite属性>()?;
        let mut 结果 = std::collections::BTreeMap::new();
        结果.insert(
            "贴图".to_string(),
            match &属性.贴图路径 {
                Some(路径) => 属性值::资源(scene_doc::property::资源引用值::按路径(
                    路径.clone(),
                )),
                None => 属性值::空,
            },
        );
        结果.insert("颜色".to_string(), 属性值::颜色(属性.颜色));
        结果.insert("水平翻转".to_string(), 属性值::布尔(属性.水平翻转));
        结果.insert("垂直翻转".to_string(), 属性值::布尔(属性.垂直翻转));
        结果.insert(
            "自定义尺寸".to_string(),
            match 属性.自定义尺寸 {
                Some((宽, 高)) => 属性值::二维(宽, 高),
                None => 属性值::空,
            },
        );
        结果.insert(
            "模型".to_string(),
            match &属性.模型路径 {
                Some(路径) => 属性值::资源(scene_doc::property::资源引用值::按路径(
                    路径.clone(),
                )),
                None => 属性值::空,
            },
        );
        结果.insert(
            "材质".to_string(),
            match &属性.材质路径 {
                Some(路径) => 属性值::资源(scene_doc::property::资源引用值::按路径(
                    路径.clone(),
                )),
                None => 属性值::空,
            },
        );
        Some(结果)
    }

    fn 实例化(
        &self,
        实例: &mut dyn std::any::Any,
        数据: &std::collections::BTreeMap<String, 属性值>,
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<Sprite属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("贴图") {
            属性.贴图路径 = 值.取资源().map(|引用| 引用.路径.clone());
        }
        if let Some(值) = 数据.get("颜色").and_then(属性值::取颜色) {
            属性.颜色 = 值;
        }
        if let Some(值) = 数据.get("水平翻转").and_then(属性值::取布尔) {
            属性.水平翻转 = 值;
        }
        if let Some(值) = 数据.get("垂直翻转").and_then(属性值::取布尔) {
            属性.垂直翻转 = 值;
        }
        if let Some(值) = 数据.get("自定义尺寸") {
            属性.自定义尺寸 = 值.取二维();
        }
        if let Some(值) = 数据.get("模型") {
            属性.模型路径 = 值.取资源().map(|引用| 引用.路径.clone());
        }
        if let Some(值) = 数据.get("材质") {
            属性.材质路径 = 值.取资源().map(|引用| 引用.路径.clone());
        }
        true
    }
}

/// Sprite 字段描述。
///
/// `自定义尺寸` 的字段名被 [`editor_ui::viewport_panel`] 读取用于计算视口包围框，
/// 因此这里的名字改动会同时影响视口显示 —— 两端都引用同一个常量会更安全，
/// 但跨 crate 常量会引入依赖，这里用注释标记约束，并在视口侧写了测试。
fn sprite字段描述() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            资源(
                "贴图",
                "sprite_field_texture",
                scene_doc::property::资源类别::图片,
            )
            .带提示("sprite_field_texture_tooltip"),
            颜色("颜色", "sprite_field_tint", 颜色值::白()),
            布尔("水平翻转", "sprite_field_flip_horizontal", false),
            布尔("垂直翻转", "sprite_field_flip_vertical", false),
            二维("自定义尺寸", "sprite_field_custom_size", (64.0, 64.0))
                .带提示("sprite_field_custom_size_tooltip"),
            资源(
                "模型",
                "sprite_field_model",
                scene_doc::property::资源类别::模型,
            )
            .带提示("sprite_field_model_tooltip"),
            资源(
                "材质",
                "sprite_field_material",
                scene_doc::property::资源类别::材质,
            )
            .带提示("sprite_field_material_tooltip"),
        ]
    })
}

/// `编辑器相机2d` 的编辑器适配。
struct 相机组件类型;

impl 组件类型 for 相机组件类型 {
    fn 组件名(&self) -> &'static str {
        "Camera2d"
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(编辑器相机2d::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        相机字段描述()
    }

    fn 导出(
        &self,
        实例: &dyn std::any::Any,
    ) -> Option<std::collections::BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<编辑器相机2d>()?;
        let mut 结果 = std::collections::BTreeMap::new();
        结果.insert("正交半高".to_string(), 属性值::浮点(属性.正交半高));
        Some(结果)
    }

    fn 实例化(
        &self,
        实例: &mut dyn std::any::Any,
        数据: &std::collections::BTreeMap<String, 属性值>,
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<编辑器相机2d>() else {
            return false;
        };
        if let Some(值) = 数据.get("正交半高").and_then(属性值::取浮点) {
            // 正交半高为 0 或负数会让投影矩阵退化，直接拒绝。
            if 值 > 0.0 {
                属性.正交半高 = 值;
            }
        }
        true
    }
}

/// 相机字段描述。
fn 相机字段描述() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            浮点(
                "正交半高",
                "camera_field_ortho_half_height",
                360.0,
                1.0,
                100000.0,
                1.0,
            )
            .带提示("camera_ortho_half_height_tooltip"),
        ]
    })
}

/// 整数属性描述构造辅助。
fn 整数(
    字段: &str, 显示: &'static str, 默认: i64, 最小: i64, 最大: i64
) -> 属性描述 {
    属性描述::新建(
        字段,
        显示,
        属性种类::整数 {
            最小: Some(最小),
            最大: Some(最大),
        },
        属性值::整数(默认),
    )
}

/// `瓦片地形` 的编辑器适配。
struct 瓦片地形组件类型;

impl 组件类型 for 瓦片地形组件类型 {
    fn 组件名(&self) -> &'static str {
        "TileTerrain"
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(瓦片地形属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        瓦片地形字段描述()
    }

    fn 导出(
        &self,
        实例: &dyn std::any::Any,
    ) -> Option<std::collections::BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<瓦片地形属性>()?;
        let mut 结果 = std::collections::BTreeMap::new();
        结果.insert("地形文件".to_string(), 属性值::文本(属性.地形文件.clone()));
        结果.insert("图集目录".to_string(), 属性值::文本(属性.图集目录.clone()));
        结果.insert("地图文件".to_string(), 属性值::文本(属性.地图文件.clone()));
        结果.insert("瓦片集".to_string(), 属性值::文本(属性.瓦片集.clone()));
        结果.insert(
            "显示尺寸".to_string(),
            属性值::二维(属性.显示尺寸.0, 属性.显示尺寸.1),
        );
        结果.insert("每格像素".to_string(), 属性值::整数(属性.每格像素 as i64));
        结果.insert("变体概率".to_string(), 属性值::浮点(属性.变体概率));
        Some(结果)
    }

    fn 实例化(
        &self,
        实例: &mut dyn std::any::Any,
        数据: &std::collections::BTreeMap<String, 属性值>,
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<瓦片地形属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("地形文件").and_then(属性值::取文本) {
            属性.地形文件 = 值.to_string();
        }
        if let Some(值) = 数据.get("图集目录").and_then(属性值::取文本) {
            属性.图集目录 = 值.to_string();
        }
        if let Some(值) = 数据.get("地图文件").and_then(属性值::取文本) {
            属性.地图文件 = 值.to_string();
        }
        if let Some(值) = 数据.get("瓦片集").and_then(属性值::取文本) {
            属性.瓦片集 = 值.to_string();
        }
        if let Some(值) = 数据.get("显示尺寸").and_then(属性值::取二维) {
            属性.显示尺寸 = (值.0.abs().max(1.0), 值.1.abs().max(1.0));
        }
        if let Some(值) = 数据.get("每格像素").and_then(属性值::取整数) {
            // 上限与纹理尺寸一起约束：48 格 × 128px 已经是 6K。
            属性.每格像素 = 值.clamp(8, 128) as u32;
        }
        if let Some(值) = 数据.get("变体概率").and_then(属性值::取浮点) {
            属性.变体概率 = 值.clamp(0.0, 1.0);
        }
        true
    }
}

/// `TileTerrain` 字段描述。
fn 瓦片地形字段描述() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            文本(
                "地形文件",
                "tile_terrain_field_file",
                "assets/terrain/example.terrain.ron",
            )
            .带提示("tile_terrain_field_file_tooltip"),
            文本(
                "图集目录",
                "tile_terrain_field_atlas",
                "assets/terrain/tiles",
            )
            .带提示("tile_terrain_field_atlas_tooltip"),
            文本(
                "地图文件",
                "tile_terrain_field_map",
                "assets/terrain/example.tilemap.ron",
            )
            .带提示("tile_terrain_field_map_tooltip"),
            文本(
                "瓦片集",
                "tile_terrain_field_tileset",
                "assets/terrain/example.tileset.ron",
            )
            .带提示("tile_terrain_field_tileset_tooltip"),
            二维(
                "显示尺寸",
                "tile_terrain_field_display_size",
                (1024.0, 1024.0),
            )
            .带提示("tile_terrain_field_display_size_tooltip"),
            整数("每格像素", "tile_terrain_field_cell_pixels", 64, 8, 128)
                .带提示("tile_terrain_field_cell_pixels_tooltip"),
            浮点(
                "变体概率",
                "tile_terrain_field_variant_chance",
                0.35,
                0.0,
                1.0,
                0.01,
            )
            .带提示("tile_terrain_field_variant_chance_tooltip"),
        ]
    })
}

/// `几何体` 的编辑器适配。
struct 几何体组件类型;

impl 组件类型 for 几何体组件类型 {
    fn 组件名(&self) -> &'static str {
        "Primitive3d"
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(几何体属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        几何体字段描述()
    }

    fn 导出(
        &self,
        实例: &dyn std::any::Any,
    ) -> Option<std::collections::BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<几何体属性>()?;
        let mut 结果 = std::collections::BTreeMap::new();
        结果.insert(
            "形状".to_string(),
            属性值::枚举(scene_doc::property::枚举值::新建(
                属性.形状.变体名(),
                属性.形状.序号(),
            )),
        );
        结果.insert(
            "尺寸".to_string(),
            属性值::三维(属性.尺寸.0, 属性.尺寸.1, 属性.尺寸.2),
        );
        结果.insert(
            "贴图".to_string(),
            match &属性.贴图路径 {
                Some(路径) => 属性值::资源(scene_doc::property::资源引用值::按路径(
                    路径.clone(),
                )),
                None => 属性值::空,
            },
        );
        结果.insert("颜色".to_string(), 属性值::颜色(属性.颜色));
        结果.insert(
            "贴图重复".to_string(),
            属性值::二维(属性.贴图重复.0, 属性.贴图重复.1),
        );
        结果.insert("受光".to_string(), 属性值::布尔(属性.受光));
        结果.insert("双面".to_string(), 属性值::布尔(属性.双面));
        结果.insert(
            "材质".to_string(),
            match &属性.材质路径 {
                Some(路径) => 属性值::资源(scene_doc::property::资源引用值::按路径(
                    路径.clone(),
                )),
                None => 属性值::空,
            },
        );
        Some(结果)
    }

    fn 实例化(
        &self,
        实例: &mut dyn std::any::Any,
        数据: &std::collections::BTreeMap<String, 属性值>,
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<几何体属性>() else {
            return false;
        };
        if let Some(属性值::枚举(枚举)) = 数据.get("形状") {
            属性.形状 = 几何体形状::由值(&枚举.变体名, 枚举.序号);
        }
        if let Some(值) = 数据.get("尺寸").and_then(属性值::取三维) {
            属性.尺寸 = (
                值.0.abs().max(0.001),
                值.1.abs().max(0.001),
                值.2.abs().max(0.001),
            );
        }
        if let Some(值) = 数据.get("贴图") {
            属性.贴图路径 = 值.取资源().map(|引用| 引用.路径.clone());
        }
        if let Some(值) = 数据.get("颜色").and_then(属性值::取颜色) {
            属性.颜色 = 值;
        }
        if let Some(值) = 数据.get("贴图重复").and_then(属性值::取二维) {
            属性.贴图重复 = (值.0.abs().max(0.0), 值.1.abs().max(0.0));
        }
        if let Some(值) = 数据.get("受光").and_then(属性值::取布尔) {
            属性.受光 = 值;
        }
        if let Some(值) = 数据.get("双面").and_then(属性值::取布尔) {
            属性.双面 = 值;
        }
        if let Some(值) = 数据.get("材质") {
            属性.材质路径 = 值.取资源().map(|引用| 引用.路径.clone());
        }
        true
    }
}

/// `Primitive3d` 字段描述。
fn 几何体字段描述() -> &'static [属性描述] {
    use std::sync::OnceLock;
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "形状",
                "primitive_field_shape",
                属性种类::枚举 {
                    选项: vec![
                        ("Cuboid".to_string(), "Cuboid".to_string(), 0),
                        ("Plane".to_string(), "Plane".to_string(), 1),
                    ],
                },
                属性值::枚举(scene_doc::property::枚举值::新建("Cuboid", 0)),
            )
            .带提示("primitive_field_shape_tooltip"),
            三维("尺寸", "primitive_field_size", (1.0, 1.0, 1.0))
                .带提示("primitive_field_size_tooltip"),
            资源(
                "贴图",
                "primitive_field_texture",
                scene_doc::property::资源类别::图片,
            )
            .带提示("primitive_field_texture_tooltip"),
            颜色("颜色", "primitive_field_tint", 颜色值::白()),
            二维("贴图重复", "primitive_field_uv_repeat", (1.0, 1.0))
                .带提示("primitive_field_uv_repeat_tooltip"),
            布尔("受光", "primitive_field_lit", true).带提示("primitive_field_lit_tooltip"),
            布尔("双面", "primitive_field_double_sided", false)
                .带提示("primitive_field_double_sided_tooltip"),
            资源(
                "材质",
                "primitive_field_material",
                scene_doc::property::资源类别::材质,
            )
            .带提示("primitive_field_material_tooltip"),
        ]
    })
}

/// 三维向量属性描述构造辅助（供 `Primitive3d` 使用）。
fn 三维(字段: &str, 显示: &'static str, 默认: (f32, f32, f32)) -> 属性描述 {
    属性描述::新建(
        字段,
        显示,
        属性种类::三维 { 步进: 0.1 },
        属性值::三维(默认.0, 默认.1, 默认.2),
    )
}

/// 组件构造器类型。
type 组件构造 = fn() -> Box<dyn 组件类型>;

/// 基础组件数量，即构造表中前 6 项。
const 基础组件数量: usize = 6;

/// 内置组件的注册顺序：**唯一**的组件注册来源。
///
/// 表里存构造器而不是 `Box<dyn 组件类型>`：注册表按值接管 `Box`，
/// 而静态值不能被多次移动；每次构建注册表都必须重新构造实例。
///
/// 顺序与集中前完全一致：基础 6 项 → 三维灯光 → 界面 → 音频 → 物理。
/// 这个顺序是既有行为的基线（组件名冲突的暴露顺序依赖它），
/// 测试 `组件构造顺序与集中前一致` 守住它。
static 内置组件构造表: &[组件构造] = &[
    // 基础组件：`默认组件注册表` 只取这一段，BSN 提取与既有契约依赖这个边界。
    || Box::new(变换组件类型),
    || Box::new(可见性组件类型),
    || Box::new(Sprite组件类型),
    || Box::new(相机组件类型),
    || Box::new(瓦片地形组件类型),
    || Box::new(几何体组件类型),
    // 扩展组件：启动时由 `全部内置组件注册表` 一并注册。
    || Box::new(crate::components3d::方向光组件类型),
    || Box::new(crate::components3d::点光组件类型),
    || Box::new(crate::ui_components::界面面板组件类型),
    || Box::new(crate::ui_components::界面文本组件类型),
    || Box::new(crate::ui_components::界面按钮组件类型),
    || Box::new(crate::audio_components::音频组件类型),
    || Box::new(crate::physics_components::碰撞体组件类型),
    || Box::new(crate::physics_components::刚体组件类型),
];

/// 按构造表构建注册表；这是全项目唯一的 `.注册` 调用点。
fn 按表构建注册表(构造表: &[组件构造]) -> 组件注册表 {
    let mut 注册表 = 组件注册表::新建();
    for 构造 in 构造表 {
        注册表
            .注册(构造())
            .expect("内置组件的描述与默认值必须自洽；注册失败说明代码有错");
    }
    注册表
}

/// 构建编辑器的基础组件注册表（仅前 6 个内置组件）。
///
/// 注册失败会 panic：这些是编辑器自带的组件，若它们的描述与默认值
/// 都不自洽，那是代码错误而不是用户错误，应当立刻在启动时暴露，
/// 而不是等到用户点开 Inspector 才发现某个字段是坏的。
pub fn 默认组件注册表() -> 组件注册表 {
    按表构建注册表(&内置组件构造表[..基础组件数量])
}

/// 构建编辑器实际使用的完整注册表（基础组件 + 全部扩展组件）。
pub fn 全部内置组件注册表() -> 组件注册表 {
    按表构建注册表(内置组件构造表)
}

/// 编辑器支持的组件类型名列表（供 UI 的"添加组件"菜单与测试使用）。
pub fn 内置组件名列表() -> Vec<String> {
    默认组件注册表().组件名列表()
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::组件数据;

    fn 建带组件的实体(文档: &mut 场景文档, 名称: &str) -> 实体标识 {
        文档.生成实体(名称, None)
    }

    #[test]
    fn 默认注册表包含全部内置组件() {
        let 注册表 = 默认组件注册表();
        for 名字 in [
            "Transform",
            "Visibility",
            "Sprite",
            "Camera2d",
            "TileTerrain",
            "Primitive3d",
        ] {
            assert!(注册表.已注册(名字), "内置组件 `{名字}` 应当被注册");
        }
        assert_eq!(注册表.数量(), 6);
    }

    #[test]
    fn 组件构造顺序与集中前一致() {
        let 顺序: Vec<&'static str> = 内置组件构造表.iter().map(|构造| 构造().组件名()).collect();
        assert_eq!(
            顺序,
            vec![
                "Transform",
                "Visibility",
                "Sprite",
                "Camera2d",
                "TileTerrain",
                "Primitive3d",
                "方向光",
                "点光",
                "UiNode",
                "UiText",
                "UiButton",
                "音频",
                "碰撞体2d",
                "刚体2d",
            ],
            "注册顺序是集中前的基线，不得改变"
        );
    }

    #[test]
    fn 完整注册表包含全部内置组件() {
        let 注册表 = 全部内置组件注册表();
        assert_eq!(注册表.数量(), 内置组件构造表.len());
        for 名字 in 内置组件构造表.iter().map(|构造| 构造().组件名()) {
            assert!(注册表.已注册(名字), "内置组件 `{名字}` 应当被注册");
        }
    }

    #[test]
    fn 内置组件的默认值都符合各自声明的种类() {
        // 注册表在注册时已经校验过一遍，这里再显式断言一次，
        // 让"默认值必须合法"这条约束在测试名里可见。
        let 注册表 = 默认组件注册表();
        for 名字 in 注册表.组件名列表() {
            let 类型 = 注册表.要求(&名字).unwrap();
            for 描述 in 类型.字段描述() {
                assert!(
                    描述.种类.接受(&描述.默认值),
                    "组件 `{名字}` 的字段 `{}` 默认值非法",
                    描述.字段名
                );
            }
        }
    }

    #[test]
    fn 变换数据与_bevy_transform_往返一致() {
        let 原 = 变换数据 {
            平移: (1.5, -2.5, 3.0),
            旋转: (0.0, 0.0, 0.0, 1.0),
            缩放: (2.0, 2.0, 1.0),
        };
        let 还原 = bevy转_变换数据(&变换数据转_bevy(原));
        assert!((还原.平移.0 - 原.平移.0).abs() < 1e-6);
        assert!((还原.平移.1 - 原.平移.1).abs() < 1e-6);
        assert!((还原.平移.2 - 原.平移.2).abs() < 1e-6);
        assert!((还原.缩放.0 - 原.缩放.0).abs() < 1e-6);
        assert!((还原.旋转.3 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn 退化四元数被替换为单位四元数() {
        // 损坏的文件里可能出现零四元数，转换时必须兜住，
        // 否则 Bevy 的 Transform 会得到 NaN，整个渲染崩掉。
        let mut 原 = 变换数据::单位();
        原.旋转 = (0.0, 0.0, 0.0, 0.0);
        let 结果 = 变换数据转_bevy(原);
        assert!(结果.rotation.is_finite());
        assert!((结果.rotation.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn 构建世界会创建实体与变换() {
        let mut 文档 = 场景文档::新建("桥接测试");
        let 甲 = 文档.生成实体("甲", None);
        let 乙 = 文档.生成实体("乙", Some(甲));
        if let Some(实体) = 文档.取可变(乙) {
            实体.变换 = Some(变换数据 {
                平移: (10.0, 20.0, 0.0),
                ..变换数据::单位()
            });
        }

        let 注册表 = 默认组件注册表();
        let mut 世界 = World::new();
        let (统计, 映射) = 构建世界(&mut 世界, &文档, &注册表);

        assert_eq!(统计.实体数, 2);
        assert_eq!(统计.父子关系数, 1);
        assert_eq!(映射.数量(), 2);

        // 乙的变换必须被正确搬运
        let 乙运行时 = 映射.取运行时(乙).unwrap();
        let 变换 = 世界.get::<Transform>(乙运行时).unwrap();
        assert!((变换.translation.x - 10.0).abs() < 1e-6);
        assert!((变换.translation.y - 20.0).abs() < 1e-6);

        // 父子关系必须建立
        let 甲运行时 = 映射.取运行时(甲).unwrap();
        assert_eq!(
            世界.get::<ChildOf>(乙运行时).map(|关系| 关系.parent()),
            Some(甲运行时)
        );
    }

    #[test]
    fn 构建世界会应用已注册组件() {
        let mut 文档 = 场景文档::新建("组件桥接");
        let 标识 = 建带组件的实体(&mut 文档, "带精灵的实体");

        let mut 属性 = std::collections::BTreeMap::new();
        属性.insert(
            "颜色".to_string(),
            属性值::颜色(颜色值::srgb(1.0, 0.0, 0.0, 1.0)),
        );
        属性.insert("水平翻转".to_string(), 属性值::布尔(true));
        属性.insert("自定义尺寸".to_string(), 属性值::二维(32.0, 48.0));
        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(组件数据::新建("Sprite", 属性));
        }

        let 注册表 = 默认组件注册表();
        let mut 世界 = World::new();
        let (统计, 映射) = 构建世界(&mut 世界, &文档, &注册表);

        assert_eq!(统计.组件数, 1);
        assert_eq!(统计.跳过组件数, 0);

        let 运行时 = 映射.取运行时(标识).unwrap();
        let 精灵 = 世界.get::<Sprite属性>(运行时).unwrap();
        assert_eq!(精灵.颜色.红, 1.0);
        assert!(精灵.水平翻转);
        assert_eq!(精灵.自定义尺寸, Some((32.0, 48.0)));
    }

    #[test]
    fn 未注册组件被跳过但不影响其他组件() {
        let mut 文档 = 场景文档::新建("含未知组件");
        let 标识 = 文档.生成实体("甲", None);

        let mut 未知属性 = std::collections::BTreeMap::new();
        未知属性.insert("未来字段".to_string(), 属性值::整数(42));
        let mut 已知属性 = std::collections::BTreeMap::new();
        已知属性.insert("可见".to_string(), 属性值::布尔(false));

        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(组件数据::新建("来自未来的组件", 未知属性));
            实体.加组件(组件数据::新建("Visibility", 已知属性));
        }

        let 注册表 = 默认组件注册表();
        let mut 世界 = World::new();
        let (统计, 映射) = 构建世界(&mut 世界, &文档, &注册表);

        assert_eq!(统计.跳过组件数, 1, "未知组件应当被跳过并计入统计");
        assert_eq!(统计.跳过类型, vec!["来自未来的组件".to_string()]);
        assert_eq!(统计.组件数, 1, "已知组件仍应被正常应用");

        // 关键：未知组件不能让整个实体丢失
        let 运行时 = 映射.取运行时(标识).unwrap();
        let 可见性 = 世界.get::<Visibility>(运行时).unwrap();
        assert!(!matches!(可见性, Visibility::Inherited), "应当被应用为隐藏");
    }

    #[test]
    fn 悬空父实体不会导致子实体丢失() {
        let mut 文档 = 场景文档::新建("悬空父");
        let 孤儿 = 文档.生成实体("孤儿", None);
        // 人为制造悬空父实体
        if let Some(实体) = 文档.取可变(孤儿) {
            实体.父实体 = Some(实体标识::新建());
        }

        let 注册表 = 默认组件注册表();
        let mut 世界 = World::new();
        let (统计, 映射) = 构建世界(&mut 世界, &文档, &注册表);

        assert_eq!(统计.实体数, 1, "实体不能因为父实体不存在而消失");
        assert_eq!(统计.父子关系数, 0);
        assert!(映射.取运行时(孤儿).is_some());
    }

    #[test]
    fn 运行时宿主启动停止后世界被彻底丢弃() {
        let 文档 = {
            let mut 文档 = 场景文档::新建("宿主测试");
            文档.生成实体("甲", None);
            文档.生成实体("乙", None);
            文档
        };
        let 注册表 = 默认组件注册表();
        let mut 宿主 = 运行时宿主::default();

        assert_eq!(宿主.状态, 运行状态::已停止);
        assert!(!宿主.状态.在运行());
        assert_eq!(宿主.运行时实体数(), 0);

        宿主.启动(&文档, &注册表).expect("有实体的场景必须能启动");
        assert_eq!(宿主.状态, 运行状态::运行中);
        assert!(宿主.状态.在运行());
        {
            let 世界 = 宿主.世界.as_ref().unwrap();
            let 存活: Vec<_> = 世界
                .iter_entities()
                .map(|引用| {
                    (
                        引用.id(),
                        引用.get::<Name>().map(|名| 名.as_str().to_string()),
                    )
                })
                .collect();
            // 记录实测结果：`World::new()` 自带的两个无名内部实体
            // 必须被排除在"场景实体数"之外，否则诊断面板会误导用户。
            eprintln!(
                "DEBUG 全部存活={} 其中带名字的={} 实体列表={:?}",
                世界.entities().count_spawned(),
                存活.iter().filter(|(_, 名)| 名.is_some()).count(),
                存活
            );
        }
        assert_eq!(宿主.运行时实体数(), 2);
        assert_eq!(宿主.映射.数量(), 2);
        assert!(宿主.启动快照.is_some());

        // 暂停与继续
        宿主.暂停();
        assert_eq!(宿主.状态, 运行状态::已暂停);
        宿主.继续();
        assert_eq!(宿主.状态, 运行状态::运行中);

        // 停止后运行时数据必须整体消失
        宿主.停止();
        assert_eq!(宿主.状态, 运行状态::已停止);
        assert!(宿主.世界.is_none(), "停止后运行时世界必须被丢弃");
        assert_eq!(宿主.映射.数量(), 0);
        assert_eq!(宿主.运行时实体数(), 0);
    }

    #[test]
    fn 运行时的修改不会影响文档快照() {
        // 这是"数据隔离"的核心断言：改动运行时世界里的实体，
        // 启动时用的文档必须完全不受影响。
        let mut 文档 = 场景文档::新建("隔离测试");
        let 标识 = 文档.生成实体("玩家", None);
        let 文档备份 = 文档.clone();

        let 注册表 = 默认组件注册表();
        let mut 宿主 = 运行时宿主::default();
        宿主.启动(&文档, &注册表).expect("有实体的场景必须能启动");

        // 在运行时世界里修改实体
        let 运行时实体 = 宿主.映射.取运行时(标识).unwrap();
        let 世界 = 宿主.世界.as_mut().unwrap();
        世界.entity_mut(运行时实体).insert(Transform {
            translation: Vec3::new(999.0, 999.0, 999.0),
            ..default()
        });
        世界.despawn(运行时实体);

        // 运行时世界里确实少了这个实体（否则下面的隔离断言就没有说服力：
        // 如果运行时根本没被改动，"文档没变"是理所当然的）
        assert_eq!(
            宿主.运行时实体数(),
            0,
            "运行时实体已被销毁，运行时实体数应当变为 0"
        );

        // 文档必须一字未改
        assert_eq!(文档, 文档备份, "运行时修改绝不能写回场景文档");
        assert!(文档.存在(标识));
        assert_eq!(文档.取(标识).unwrap().名称, "玩家");
    }

    #[test]
    fn 实体映射表双向一致() {
        let mut 映射 = 实体映射表::新建();
        let mut 世界 = World::new();
        let 文档标识 = 实体标识::新建();
        let 运行时 = 世界.spawn_empty().id();
        映射.插入(文档标识, 运行时);

        assert_eq!(映射.取运行时(文档标识), Some(运行时));
        assert_eq!(映射.取文档(运行时), Some(文档标识));
        assert_eq!(映射.数量(), 1);

        // 未知标识
        let 别的 = 实体标识::新建();
        assert_eq!(映射.取运行时(别的), None);

        映射.清空();
        assert_eq!(映射.数量(), 0);
    }

    #[test]
    fn 空场景能正常构建() {
        let 文档 = 场景文档::新建("空场景");
        let 注册表 = 默认组件注册表();
        let mut 世界 = World::new();
        let (统计, 映射) = 构建世界(&mut 世界, &文档, &注册表);
        assert_eq!(统计.实体数, 0);
        assert_eq!(映射.数量(), 0);
    }

    #[test]
    fn 内置组件名列表与注册表一致() {
        let 列表 = 内置组件名列表();
        assert_eq!(列表.len(), 6, "注册表新增组件时要同步更新这条断言");
        assert!(列表.contains(&"Transform".to_string()));
        assert!(列表.contains(&"TileTerrain".to_string()));
        assert!(列表.contains(&"Primitive3d".to_string()));
    }

    /// 守住"注册表里的每个类型都必须有插入分支"这条不变量。
    ///
    /// 这条不变量无法用类型系统表达（`组件类型` 是类型擦除的），
    /// 而漏掉一个分支的后果很隐蔽：注册表里有、属性也写进去了、
    /// 运行时统计却把它算成"跳过"，组件静默失效。
    /// 因此用测试把每个已注册类型都实际走一遍插入路径。
    #[test]
    fn 注册表与插入分支一一对应() {
        let 注册表 = 默认组件注册表();

        for 类型名 in 注册表.组件名列表() {
            let 类型 = 注册表.要求(&类型名).unwrap();
            let mut 实例 = 类型.默认实例();

            // 用声明出的默认值构造一份合法属性表
            let mut 属性 = std::collections::BTreeMap::new();
            for 描述 in 类型.字段描述() {
                属性.insert(描述.字段名.clone(), 描述.默认值.clone());
            }
            assert!(
                类型.实例化(实例.as_mut(), &属性),
                "组件 `{类型名}` 的实例化失败了"
            );

            // 关键断言：必须能被插入世界。返回 false 说明
            // `插入已知组件` 缺少对应的 downcast 分支。
            let mut 世界 = World::new();
            let 实体 = 世界.spawn_empty().id();
            assert!(
                插入已知组件(&mut 世界, 实体, 实例.as_ref()),
                "组件 `{类型名}` 在 `插入已知组件` 里没有对应的分支；\
                 注册表与插入逻辑必须保持一一对应"
            );
        }
    }

    #[test]
    fn 相机正交半高拒绝非法值() {
        // 正交半高为 0 会让投影矩阵退化，必须被拒绝而不是写进去。
        let 类型 = 相机组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = std::collections::BTreeMap::new();
        数据.insert("正交半高".to_string(), 属性值::浮点(0.0));
        assert!(类型.实例化(实例.as_mut(), &数据));
        let 结果 = 实例.downcast_ref::<编辑器相机2d>().unwrap();
        assert_eq!(结果.正交半高, 360.0, "非法值应当被忽略，保留原值");

        数据.insert("正交半高".to_string(), 属性值::浮点(720.0));
        assert!(类型.实例化(实例.as_mut(), &数据));
        assert_eq!(实例.downcast_ref::<编辑器相机2d>().unwrap().正交半高, 720.0);
    }

    #[test]
    fn 组件导出与实例化往返一致() {
        let 注册表 = 默认组件注册表();
        let 类型 = 注册表.要求("Sprite").unwrap();

        let mut 实例 = 类型.默认实例();
        let mut 数据 = std::collections::BTreeMap::new();
        数据.insert(
            "颜色".to_string(),
            属性值::颜色(颜色值::srgb(0.2, 0.4, 0.6, 1.0)),
        );
        数据.insert("垂直翻转".to_string(), 属性值::布尔(true));
        数据.insert("自定义尺寸".to_string(), 属性值::二维(100.0, 200.0));
        数据.insert(
            "贴图".to_string(),
            属性值::资源(scene_doc::property::资源引用值::按路径(
                scene_doc::path::项目相对路径::解析("assets/textures/a.png").unwrap(),
            )),
        );

        assert!(类型.实例化(实例.as_mut(), &数据));

        // 导出回来必须一致
        let 导出 = 类型.导出(实例.as_ref()).unwrap();
        assert_eq!(导出.get("颜色").unwrap().取颜色().unwrap().绿, 0.4);
        assert_eq!(导出.get("垂直翻转").unwrap().取布尔(), Some(true));
        assert_eq!(
            导出.get("自定义尺寸").unwrap().取二维(),
            Some((100.0, 200.0))
        );
        assert_eq!(
            导出.get("贴图").unwrap().取资源().unwrap().路径.文本(),
            "assets/textures/a.png"
        );
    }
}
