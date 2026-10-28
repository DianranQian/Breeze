//! 粒子系统的模拟与两处宿主：运行时世界与预览视口。
//!
//! # 为什么同一个模拟器服务两个宿主
//!
//! 编辑器有两条互不相干的"世界"（见 [`crate::preview`] 与
//! [`crate::scene_bridge`]）：预览要真的画出粒子，运行时只要求
//! "游戏逻辑里真的有粒子在动"。两边如果各写一套积分公式，
//! 一定会出现"编辑器里是这样、跑起来是那样"的经典分叉。
//! 所以积分、发射、寿命、上限全部收在 [`粒子模拟器`] 里，
//! 两个宿主只负责把"编号 → 实体"的映射落到自己的世界。
//!
//! # 两处宿主各自做什么
//!
//! | | 运行时世界 | 预览视口 |
//! | --- | --- | --- |
//! | 位置 | 隔离的裸 `World`（无渲染） | 主世界，预览节点子树 |
//! | 粒子实体 | 只有 `Transform` + [`粒子实例`] | 小球网格 + 材质，三维层 |
//! | 生命周期 | 运行中推进，停止即随世界丢弃 | 一直推进，随预览重建而重置 |
//! | 用途 | 让运行时诊断看到真实的粒子数量 | 让用户看到效果 |
//!
//! # 为什么用确定性伪随机
//!
//! 粒子的散开方向需要随机，但本仓库没有引入 `rand`（Bevy 只保证
//! 内部依赖存在，直接依赖它会绑定到 Bevy 的私有版本）。这里用
//! 一个 32 位线性同余发生器：同一个种子给出同一段轨迹，测试因此
//! 可以断言"第 N 帧粒子在哪"，不需要任何容差魔法。
//!
//! # 物理口径
//!
//! 粒子只受恒定重力（[`粒子重力`]）：没有碰撞、没有风。这是"极简"
//! 的边界，写在这里是为了让后来者知道哪些是刻意省略的，而不是 bug。

use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;

use crate::particle_components::{
    最大粒子数, 粒子发射器属性, 粒子发射器组件名
};

/// 粒子受到的重力加速度（世界单位/秒²）。
pub const 粒子重力: f32 = -9.8;

/// 发射方向的锥形散开半径（与竖直方向的水平比）。
///
/// `0.6` 对应约 31° 的半角：太小像一根水柱，太大像球形爆炸。
pub const 散开半径: f32 = 0.6;

/// 单帧允许的最大时间步（秒）。
///
/// 编辑器卡顿后 `delta` 可能是几秒，直接用会让粒子瞬移一大截，
/// 也会让发射蓄积一次性生成几百个粒子。夹到 100ms 后，
/// 长卡顿表现为"粒子慢了半拍"，而不是"画面炸开"。
pub const 最大步长: f32 = 0.1;

/// 一个存活粒子的模拟状态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct 粒子状态 {
    /// 相对发射器原点的位置（世界单位，本地坐标）。
    pub 位置: Vec3,
    /// 当前速度（世界单位/秒，本地坐标）。
    pub 速度: Vec3,
    /// 已经存在的时间（秒）。
    pub 年龄: f32,
    /// 寿命（秒），单个粒子在基础寿命上有 ±20% 抖动。
    pub 寿命: f32,
}

/// 一次步进产生的三类变化。
///
/// 宿主只需要按这三个列表做增删改，不需要知道模拟器内部怎么存粒子：
/// * `生成` —— 本帧新出现，宿主需要建实体；
/// * `存活` —— 本帧之前就在、本帧仍在，宿主需要同步位置；
/// * `消亡` —— 本帧走完寿命，宿主需要删实体。
///
/// 生成与存活互斥，避免同一条记录被处理两次。
#[derive(Debug, Default, Clone)]
pub struct 步进结果 {
    /// 新生成的粒子。
    pub 生成: Vec<(u64, 粒子状态)>,
    /// 继续存活的旧粒子。
    pub 存活: Vec<(u64, 粒子状态)>,
    /// 已经消亡的粒子编号。
    pub 消亡: Vec<u64>,
}

/// 确定性的粒子模拟器。
///
/// 内部按出现顺序保存 `(编号, 状态)`，编号只增不减；宿主用编号做
/// 实体映射，因此粒子被移除时不会出现"后面所有粒子的实体错位"。
#[derive(Debug, Clone)]
pub struct 粒子模拟器 {
    种子: u32,
    下次编号: u64,
    蓄积: f32,
    粒子: Vec<(u64, 粒子状态)>,
}

impl Default for 粒子模拟器 {
    fn default() -> Self {
        Self::新建(1)
    }
}

impl 粒子模拟器 {
    /// 用给定种子创建模拟器。
    pub fn 新建(种子: u32) -> Self {
        Self {
            种子,
            下次编号: 0,
            蓄积: 0.0,
            粒子: Vec::new(),
        }
    }

    /// 当前存活粒子数。
    pub fn 粒子数(&self) -> usize {
        self.粒子.len()
    }

    /// 当前存活粒子的只读快照（测试与诊断用）。
    pub fn 粒子(&self) -> impl Iterator<Item = (u64, 粒子状态)> + '_ {
        self.粒子.iter().copied()
    }

    /// 推进一帧，返回宿主需要落地的变化。
    ///
    /// # 步骤
    ///
    /// 1. 把时间步夹进 `0..=最大步长`，非有限值按 0 处理；
    /// 2. 先让旧粒子受重力、移动、变老，再把走完寿命的记进 `消亡`；
    /// 3. 按 `发射速率 × 时间步` 累积发射额度，每满 1 个生成一个粒子，
    ///    直到额度不足或到达 [`最大粒子数`]；
    /// 4. 到达上限时丢弃蓄积 —— 上限期间的"欠账"不应该在上限解除后
    ///    化作一次爆发，那会让帧率抖动直接变成视觉抖动。
    pub fn 步进(&mut self, 参数: &粒子发射器属性, 帧间隔: f32) -> 步进结果 {
        let 间隔 = if 帧间隔.is_finite() {
            帧间隔.clamp(0.0, 最大步长)
        } else {
            0.0
        };

        let mut 结果 = 步进结果::default();

        for (编号, 状态) in &mut self.粒子 {
            状态.速度.y += 粒子重力 * 间隔;
            状态.位置 += 状态.速度 * 间隔;
            状态.年龄 += 间隔;
            if 状态.年龄 >= 状态.寿命 {
                结果.消亡.push(*编号);
            } else {
                结果.存活.push((*编号, *状态));
            }
        }
        if !结果.消亡.is_empty() {
            let 消亡 = &结果.消亡;
            self.粒子.retain(|(编号, _)| !消亡.contains(编号));
        }

        self.蓄积 += 参数.发射速率.max(0.0) * 间隔;
        while self.蓄积 >= 1.0 && self.粒子.len() < 最大粒子数 {
            self.蓄积 -= 1.0;
            let 状态 = self.新粒子(参数);
            let 编号 = self.下次编号;
            self.下次编号 += 1;
            self.粒子.push((编号, 状态));
            结果.生成.push((编号, 状态));
        }
        if self.粒子.len() >= 最大粒子数 {
            self.蓄积 = 0.0;
        }

        结果
    }

    /// 生成一个粒子的初始状态。
    ///
    /// # 方向
    ///
    /// 在本地 +Y 周围的锥形内取方向：水平半径由 `sqrt(u)` 取样
    /// （保证圆盘内面积均匀），角度均匀分布。
    fn 新粒子(&mut self, 参数: &粒子发射器属性) -> 粒子状态 {
        let 角度 = self.下一个() * std::f32::consts::TAU;
        let 半径 = 散开半径 * self.下一个().sqrt();
        let 方向 = Vec3::new(角度.cos() * 半径, 1.0, 角度.sin() * 半径).normalize();
        let 抖动 = 0.8 + self.下一个() * 0.4;
        粒子状态 {
            位置: Vec3::ZERO,
            速度: 方向 * 参数.初始速度.max(0.0),
            年龄: 0.0,
            寿命: 参数.生命周期.max(0.1) * 抖动,
        }
    }

    /// 取一个 `0.0..1.0` 的确定性随机数（线性同余）。
    fn 下一个(&mut self) -> f32 {
        self.种子 = self
            .种子
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        // 取高 24 位：低位线性同余的周期太短，直接取低位会出现明显条纹。
        ((self.种子 >> 8) as f32) / ((1u32 << 24) as f32)
    }
}

/// 计算粒子当前显示直径。
///
/// 生命周期最后 30% 收缩到 5% 大小：没有这一手时粒子会"啪"地消失，
/// 收缩至少让它在关闭前有个收尾。`最小比例` 不是 0，因为 0 缩放的
/// 网格在部分平台上会产生退化三角形的警告。
pub fn 粒子尺度(状态: &粒子状态, 基础大小: f32) -> f32 {
    if 状态.寿命 <= f32::EPSILON {
        return 基础大小;
    }
    let 剩余 = (1.0 - 状态.年龄 / 状态.寿命).clamp(0.0, 1.0);
    let 比例 = (剩余 / 0.3).clamp(0.05, 1.0);
    基础大小.max(0.0) * 比例
}

/// 运行时世界里的粒子实例标记。
///
/// 它没有任何字段：运行时世界不渲染粒子，标记只让调试器与统计
/// 能区分"这是粒子"和"这是场景实体"。
#[derive(Component, Debug, Clone, Copy)]
pub struct 粒子实例;

/// 运行时实体上的粒子云：模拟器 + 编号到实体的映射。
#[derive(Component, Debug)]
pub struct 粒子运行时云 {
    /// 这个发射器的模拟器。
    pub 模拟: 粒子模拟器,
    实体: HashMap<u64, Entity>,
}

impl 粒子运行时云 {
    /// 用给定种子创建空云。
    pub fn 新建(种子: u32) -> Self {
        Self {
            模拟: 粒子模拟器::新建(种子),
            实体: HashMap::new(),
        }
    }
}

impl Default for 粒子运行时云 {
    fn default() -> Self {
        Self::新建(1)
    }
}

/// 给运行时实体挂上粒子云。
///
/// 种子取实体位模式：不同实体有个各自稳定的散开形状，
/// 同一实体每次运行又完全一致，便于复现问题。
pub fn 装配运行时粒子(世界: &mut World, 实体: Entity) {
    世界
        .entity_mut(实体)
        .insert(粒子运行时云::新建(实体.to_bits() as u32));
}

/// 推进运行时世界里所有粒子发射器。
///
/// # 步骤
///
/// 1. 先收集所有挂 [`粒子发射器属性`] 的实体（只读查询，避免与生成冲突）；
/// 2. 逐个取出粒子云、步进、按结果增删改粒子实体，再放回；
/// 3. 粒子实体挂在发射器下（`ChildOf`），停止运行时随世界一起丢弃。
pub fn 推进运行时粒子(世界: &mut World, 经过: f32) {
    let 发射器: Vec<(Entity, 粒子发射器属性)> = {
        let mut 查询 = 世界.query::<(Entity, &粒子发射器属性)>();
        查询
            .iter(世界)
            .map(|(实体, 属性)| (实体, 属性.clone()))
            .collect()
    };

    for (实体, 属性) in 发射器 {
        let Some(mut 云) = 世界.entity_mut(实体).take::<粒子运行时云>() else {
            warn_一次(实体);
            continue;
        };
        let 结果 = 云.模拟.步进(&属性, 经过);

        for 编号 in &结果.消亡 {
            if let Some(粒子) = 云.实体.remove(编号) {
                if 世界.entities().contains(粒子) {
                    世界.despawn(粒子);
                }
            }
        }
        for (编号, 状态) in &结果.生成 {
            let 粒子 = 世界
                .spawn((
                    ChildOf(实体),
                    Transform::from_translation(状态.位置),
                    粒子实例,
                ))
                .id();
            云.实体.insert(*编号, 粒子);
        }
        for (编号, 状态) in &结果.存活 {
            let Some(粒子) = 云.实体.get(编号).copied() else {
                continue;
            };
            if let Some(mut 变换) = 世界.get_mut::<Transform>(粒子) {
                变换.translation = 状态.位置;
            }
        }

        世界.entity_mut(实体).insert(云);
    }
}

/// 某个运行时实体缺少粒子云时的告警去重。
///
/// 缺失只可能发生在"组件是外部直接插进世界"的场景；重复告警会
/// 每帧刷屏，所以按实体去重。这里不做修复：补云需要知道发射器种子，
/// 那是装配阶段的知识，本函数只负责把问题说出来。
fn warn_一次(实体: Entity) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static 已警告: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    let 集合 = 已警告.get_or_init(|| Mutex::new(HashSet::new()));
    let 首次 = {
        let mut 守卫 = 集合.lock().unwrap_or_else(|错| 错.into_inner());
        守卫.insert(实体.to_bits())
    };
    if 首次 {
        warn!(
            "实体 {:?} 挂着「{}」但没有粒子云，跳过粒子推进（重新运行即可修复）",
            实体, 粒子发射器组件名
        );
    }
}

/// 预览节点上的粒子云：模拟器 + 编号到粒子实体的映射 + 共享材质。
#[derive(Component, Debug)]
pub struct 预览粒子云 {
    /// 这个发射器的模拟器。
    pub 模拟: 粒子模拟器,
    /// 同一发射器的粒子共用一个材质句柄。
    pub 材质: Handle<StandardMaterial>,
    实体: HashMap<u64, Entity>,
}

impl 预览粒子云 {
    /// 用给定材质与种子创建空云。
    pub fn 新建(材质: Handle<StandardMaterial>, 种子: u32) -> Self {
        Self {
            模拟: 粒子模拟器::新建(种子),
            材质,
            实体: HashMap::new(),
        }
    }
}

/// 每帧推进预览粒子并同步渲染实体。
///
/// # 步骤
///
/// 1. 时间步同样夹在 `0..=最大步长`；
/// 2. 每个发射器按自己的参数步进；
/// 3. 消亡的粒子实体直接回收；
/// 4. 新粒子用共享小球网格 + 发射器材质生成在三维渲染层，
///    并挂到发射器节点下 —— 节点平移/旋转时粒子整体跟走；
/// 5. 存活粒子只改 `Transform`（位置 + 随寿命收缩的缩放）。
pub fn 推进预览粒子(
    mut 命令: Commands,
    三维资源: Res<crate::preview::三维预览资源>,
    时间: Res<Time>,
    发射器: Query<(Entity, &粒子发射器属性, &mut 预览粒子云)>,
) {
    let 间隔 = 时间.delta_secs();
    let 间隔 = if 间隔.is_finite() {
        间隔.clamp(0.0, 最大步长)
    } else {
        0.0
    };

    for (发射实体, 参数, mut 云) in 发射器 {
        let 结果 = 云.模拟.步进(参数, 间隔);

        for 编号 in &结果.消亡 {
            if let Some(粒子) = 云.实体.remove(编号) {
                命令.entity(粒子).despawn();
            }
        }
        for (编号, 状态) in &结果.生成 {
            let 粒子 = 命令
                .spawn((
                    Mesh3d(三维资源.粒子球.clone()),
                    MeshMaterial3d(云.材质.clone()),
                    Transform::from_translation(状态.位置)
                        .with_scale(Vec3::splat(粒子尺度(状态, 参数.粒子大小))),
                    RenderLayers::layer(crate::preview::三维渲染层),
                    ChildOf(发射实体),
                ))
                .id();
            云.实体.insert(*编号, 粒子);
        }
        for (编号, 状态) in &结果.存活 {
            let Some(粒子) = 云.实体.get(编号).copied() else {
                continue;
            };
            命令.entity(粒子).insert(
                Transform::from_translation(状态.位置)
                    .with_scale(Vec3::splat(粒子尺度(状态, 参数.粒子大小))),
            );
        }
    }
}

/// 由颜色生成预览粒子材质：不受光、半透明混合。
///
/// 不受光是刻意的：粒子是"光点/烟雾"，跟着场景光照变暗会让
/// 火焰在阴影里消失；混合模式让透明度字段直接生效。
pub fn 粒子材质(颜色: scene_doc::property::颜色值) -> StandardMaterial {
    StandardMaterial {
        base_color: crate::preview::颜色转_bevy(颜色),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    }
}

/// 预览粒子材质缓存：按颜色位模式复用句柄。
///
/// # 为什么必须缓存
///
/// 预览同步在文档每次修订时整体重建（拖动一个发射器就是每帧一次），
/// 如果每次同步都 `add` 一份材质，那些句柄没有持有者却仍留在
/// `Assets<StandardMaterial>` 里，拖久一点就是成千上万个泄漏的材质。
/// 颜色在拖动位置时不变，按颜色复用之后，一次重建只做查表。
#[derive(Resource, Default)]
pub struct 粒子材质表 {
    表: HashMap<[u32; 4], Handle<StandardMaterial>>,
}

impl 粒子材质表 {
    /// 取颜色对应的材质句柄，没有就创建一份。
    pub fn 取或建(
        &mut self,
        颜色: scene_doc::property::颜色值,
        材质表: &mut Assets<StandardMaterial>,
    ) -> Handle<StandardMaterial> {
        let 键 = [
            颜色.红.to_bits(),
            颜色.绿.to_bits(),
            颜色.蓝.to_bits(),
            颜色.透明度.to_bits(),
        ];
        self.表
            .entry(键)
            .or_insert_with(|| 材质表.add(粒子材质(颜色)))
            .clone()
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::property::颜色值;

    fn 参数() -> 粒子发射器属性 {
        粒子发射器属性 {
            发射速率: 10.0,
            生命周期: 1.0,
            初始速度: 2.0,
            颜色: 颜色值::srgb(1.0, 0.5, 0.2, 1.0),
            粒子大小: 0.2,
        }
    }

    #[test]
    fn 按发射速率累积生成() {
        let mut 模拟 = 粒子模拟器::新建(7);
        let mut 参数 = 参数();
        参数.生命周期 = 20.0;
        let mut 总数 = 0;
        for _ in 0..10 {
            总数 += 模拟.步进(&参数, 0.1).生成.len();
        }
        assert_eq!(总数, 10, "10 个/秒 × 1 秒应当恰好生成 10 个");
        assert_eq!(模拟.粒子数(), 10);
    }

    #[test]
    fn 粒子会走完寿命并消亡() {
        let mut 模拟 = 粒子模拟器::新建(3);
        let mut 参数 = 参数();
        参数.发射速率 = 10.0;
        模拟.步进(&参数, 0.1);
        assert_eq!(模拟.粒子数(), 1);
        参数.发射速率 = 0.0;
        let mut 消亡数 = 0;
        for _ in 0..20 {
            消亡数 += 模拟.步进(&参数, 0.1).消亡.len();
        }
        assert_eq!(消亡数, 1, "寿命约 1 秒的粒子应当在 2 秒内消亡");
        assert_eq!(模拟.粒子数(), 0);
    }

    #[test]
    fn 发射数量有硬上限() {
        let mut 模拟 = 粒子模拟器::新建(11);
        let mut 参数 = 参数();
        参数.发射速率 = 200.0;
        参数.生命周期 = 20.0;
        for _ in 0..100 {
            模拟.步进(&参数, 0.1);
        }
        assert_eq!(模拟.粒子数(), 最大粒子数);
    }

    #[test]
    fn 同一颗种子给出同一条轨迹() {
        let 参数 = 参数();
        let mut 甲 = 粒子模拟器::新建(42);
        let mut 乙 = 粒子模拟器::新建(42);
        let 甲结果 = 甲.步进(&参数, 0.5);
        let 乙结果 = 乙.步进(&参数, 0.5);
        assert_eq!(甲结果.生成, 乙结果.生成, "同种子必须逐位一致");
    }

    #[test]
    fn 重力让粒子先升后落() {
        let mut 模拟 = 粒子模拟器::新建(5);
        let mut 参数 = 参数();
        参数.发射速率 = 10.0;
        参数.生命周期 = 20.0;
        模拟.步进(&参数, 0.1);
        let 编号 = 模拟.粒子().next().unwrap().0;
        let 高度 = |模拟: &粒子模拟器| {
            模拟
                .粒子()
                .find(|(号, _)| *号 == 编号)
                .map(|(_, 状态)| 状态.位置.y)
                .unwrap()
        };
        for _ in 0..5 {
            模拟.步进(&参数, 0.05);
        }
        let 上升 = 高度(&模拟);
        assert!(上升 > 0.0);
        for _ in 0..40 {
            模拟.步进(&参数, 0.05);
        }
        let 后 = 高度(&模拟);
        assert!(后 < 上升, "重力必须把粒子拉下来");
    }

    #[test]
    fn 尺度随寿命收紧且不为零() {
        let 状态 = 粒子状态 {
            位置: Vec3::ZERO,
            速度: Vec3::ZERO,
            年龄: 0.0,
            寿命: 1.0,
        };
        assert_eq!(粒子尺度(&状态, 0.4), 0.4);
        let 将尽 = 粒子状态 {
            年龄: 0.99, ..状态
        };
        let 尺度 = 粒子尺度(&将尽, 0.4);
        assert!(尺度 > 0.0 && 尺度 < 0.4, "应当收缩但不为零");
    }

    #[test]
    fn 粒子材质按颜色复用句柄() {
        let mut 材质表 = Assets::<StandardMaterial>::default();
        let mut 缓存 = 粒子材质表::default();
        let 色 = 颜色值::srgb(1.0, 0.5, 0.2, 1.0);
        let 甲 = 缓存.取或建(色, &mut 材质表);
        let 乙 = 缓存.取或建(色, &mut 材质表);
        assert_eq!(甲, 乙, "同一种颜色必须复用同一个材质句柄");
        assert_eq!(材质表.len(), 1, "缓存命中时不能再 add 一份材质");
    }

    #[test]
    fn 运行时推进生成并回收粒子实体() {
        let mut 世界 = World::new();
        let 实体 = 世界.spawn((参数(), 粒子运行时云::新建(9))).id();
        推进运行时粒子(&mut 世界, 0.1);
        let 粒子数 = 世界.query::<&粒子实例>().iter(&世界).count();
        assert_eq!(粒子数, 1, "10 个/秒 × 0.1 秒应当有 1 个粒子");

        let mut 停止 = 参数();
        停止.发射速率 = 0.0;
        世界.entity_mut(实体).insert(停止);
        for _ in 0..20 {
            推进运行时粒子(&mut 世界, 0.1);
        }
        let 剩余 = 世界.query::<&粒子实例>().iter(&世界).count();
        assert_eq!(剩余, 0, "超过寿命的粒子必须被回收");
        assert!(世界.get::<粒子运行时云>(实体).is_some(), "云要放回实体");
    }
}
