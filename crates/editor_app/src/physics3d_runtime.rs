//! 三维物理模拟：让 `碰撞体3d` / `刚体3d` 在**运行态**真正参与模拟。
//!
//! # 引擎选型（先核实事实，再决定）
//!
//! 本项目用 **Bevy 0.20**，两个候选引擎都兼容：
//!
//! | 引擎 | 版本 | 依赖的 Bevy | 形态 |
//! | --- | --- | --- | --- |
//! | Avian 3D | 0.8.0 | `bevy 0.20` | ECS 驱动，`PhysicsPlugins` 插件 |
//! | bevy_rapier3d | 0.37.0 | `bevy 0.20` | `RapierPhysicsPlugin` 插件 |
//!
//! 两者的入口都是 **`App` 插件**：它们把系统排进 `App` 的调度，并依赖
//! `Assets`、`Time` 等应用级资源。而本项目的运行时世界是一个**独立的
//! `World`**（见 [`crate::scene_bridge::运行时宿主`]），没有自己的
//! `App` 与调度。为了挂插件把运行时改造成第二个 `App`，代价远大于收益，
//! 也会破坏"停止运行时整体丢弃世界"的隔离语义。
//!
//! 因此沿用二维模块已经验证过的方案：**直接依赖 `rapier3d 0.36`** ——
//! 它正是 `bevy_rapier3d 0.37` 为 Bevy 0.20 编译的那一份后端。物理世界
//! 在运行态的 `World` 之外单独持有，由固定步长驱动，结果只写回运行时
//! `Transform`。
//!
//! # 隔离边界（最重要的一条）
//!
//! 物理**只在点击「运行」之后**存在：
//!
//! * 编辑器的预览实体从不插入物理组件，也没有任何系统会步进物理；
//! * 打开场景、拖动物体、拖动时间轴都不会让物体自己掉下来；
//! * 运行时姿态只写进 [`crate::scene_bridge::运行时宿主::世界`]，
//!   绝不碰场景文档，所以保存一次不会把"落点"写进关卡。
//!
//! # 已知限制（诚实说明）
//!
//! * 场景格式没有关节 / 弹簧，因此没有关节。
//! * 碰撞事件暂不写进控制台（三维先保证"掉得下来、站得住"）。
//! * 速度不写回文档（与二维一致：运行时姿态不落盘）。

use bevy::prelude::*;
use rapier3d::prelude::*;
use scene_doc::document::{场景文档, 实体标识};
use scene_doc::registry::组件类型;

use crate::physics_components::刚体类型;
use crate::physics3d_components::{
    刚体3D组件名, 刚体3d属性, 刚体3d组件类型, 碰撞体3D组件名, 碰撞体3d属性, 碰撞体3d组件类型,
    碰撞形状3d,
};
use crate::scene_bridge::实体映射表;

/// 重力（米/秒²）。
///
/// 三维场景约定 1 世界单位 = 1 米，因此用地球表面的标准重力。
/// 二维模块用的是 980 px/s²，两者数值差 100 倍是因为单位口径不同，
/// 不是笔误。
pub const 重力加速度: f32 = -9.81;

/// 固定步长（秒）。物理按固定步长推进，渲染帧率变化不改变模拟结果。
pub const 固定步长: f32 = 1.0 / 60.0;

/// 单帧最多补多少步。卡顿后一次补上百步会穿墙并卡死。
pub const 最大补步数: u32 = 4;

/// 构建三维物理世界时的统计与问题列表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 物理3d诊断 {
    /// 建立的刚体数。
    pub 刚体数: usize,
    /// 建立的碰撞体数。
    pub 碰撞体数: usize,
    /// 没有对应运行时实体的实体数（只模拟，不回写）。
    pub 无运行时实体: usize,
    /// 问题说明。
    pub 问题: Vec<String>,
}

/// 一个文档实体在三维物理侧的对应物。
#[derive(Debug, Clone, Copy)]
struct 实体对应 {
    /// 文档标识（用于日志与测试定位）。
    标识: 实体标识,
    /// 运行时实体。`None` 表示这次运行没构建出该实体（只模拟不回写）。
    运行时: Option<Entity>,
    /// 刚体句柄。
    刚体: RigidBodyHandle,
}

/// 运行时三维物理世界。
pub struct 物理3d运行时 {
    世界: PhysicsWorld,
    实体表: Vec<实体对应>,
    累积时间: f32,
    诊断: 物理3d诊断,
}

impl std::fmt::Debug for 物理3d运行时 {
    fn fmt(&self, 格式: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        格式
            .debug_struct("物理3d运行时")
            .field("刚体数", &self.诊断.刚体数)
            .field("碰撞体数", &self.诊断.碰撞体数)
            .field("问题", &self.诊断.问题.len())
            .finish()
    }
}

/// 从组件属性还原碰撞体。
///
/// 复用组件类型的 [`crate::scene_doc::registry::组件类型::实例化`]，
/// 而不是在这里再写一份解析：夹取规则（正尺寸、摩擦 0..2、弹性 0..1）
/// 必须与 Inspector 看到的完全一致，两份解析迟早会漂移。
fn 读碰撞体(文档: &场景文档, 标识: 实体标识) -> Option<碰撞体3d属性> {
    let 组件 = 文档.取(标识)?.取组件(碰撞体3D组件名)?;
    let 类型 = 碰撞体3d组件类型;
    let mut 实例 = 类型.默认实例();
    类型.实例化(实例.as_mut(), &组件.属性);
    实例.downcast::<碰撞体3d属性>().ok().map(|盒| *盒)
}

/// 从组件属性还原刚体。
///
/// 类型缺失时按静态处理：只有碰撞体的实体在 Godot 里也是静态障碍，
/// 这比"忽略该实体"更符合直觉。
fn 读刚体(文档: &场景文档, 标识: 实体标识) -> 刚体3d属性 {
    let Some(组件) = 文档.取(标识).and_then(|实体| 实体.取组件(刚体3D组件名))
    else {
        return 刚体3d属性 {
            类型: 刚体类型::静态,
            ..刚体3d属性::default()
        };
    };
    let 类型 = 刚体3d组件类型;
    let mut 实例 = 类型.默认实例();
    类型.实例化(实例.as_mut(), &组件.属性);
    实例
        .downcast::<刚体3d属性>()
        .map(|盒| *盒)
        .unwrap_or_default()
}

/// 把场景里的四元数 `(x, y, z, w)` 转成 Rapier 的旋转。
///
/// Rapier 0.36 的 `Rotation` 是 `glam::Quat`（parry 0.31 换成 glam 数学层），
/// 分量顺序与场景格式一致，但 `from_xyzw` 不做归一化，因此这里手动归一；
/// 长度异常（手改文件里的零四元数）时退回单位旋转，避免整份世界的位姿
/// 变成 NaN。
fn 四元数转_rapier(四元数: (f32, f32, f32, f32)) -> Rotation {
    let (x, y, z, w) = 四元数;
    let 长度平方 = x * x + y * y + z * z + w * w;
    if !长度平方.is_finite() || 长度平方 <= 1e-12 {
        return Rotation::IDENTITY;
    }
    let 比例 = 1.0 / 长度平方.sqrt();
    Rotation::from_xyzw(x * 比例, y * 比例, z * 比例, w * 比例)
}

impl 物理3d运行时 {
    /// 从场景文档构建三维物理世界。
    ///
    /// `映射` 用来把物理结果写回运行时实体；构建阶段只读文档，
    /// 因此不会污染正在编辑的场景。
    pub fn 从文档构建(文档: &场景文档, 映射: &实体映射表) -> Self {
        let mut 物理 = 物理3d运行时 {
            世界: PhysicsWorld::default(),
            实体表: Vec::new(),
            累积时间: 0.0,
            诊断: 物理3d诊断::default(),
        };
        物理.世界.gravity = Vector::new(0.0, 重力加速度, 0.0);
        物理.世界.integration_parameters.dt = 固定步长;

        // 用实体表的键顺序遍历：BTreeMap 保证同一份文档每次构建的刚体顺序
        // 一致，诊断输出因此可复现。
        for 标识 in 文档.实体表.keys().copied() {
            let Some(碰撞) = 读碰撞体(文档, 标识) else {
                continue;
            };
            let 刚体 = 读刚体(文档, 标识);
            let 实体 = 文档.取(标识);
            let 名字 = 实体.map(|实体| 实体.名称.clone()).unwrap_or_default();
            let 变换 = 实体.and_then(|实体| 实体.变换).unwrap_or_default();

            let 位姿 = Pose::from_parts(
                Vector::new(变换.平移.0, 变换.平移.1, 变换.平移.2),
                四元数转_rapier(变换.旋转),
            );

            let 刚体构造 = match 刚体.类型 {
                刚体类型::静态 => RigidBodyBuilder::fixed(),
                刚体类型::动态 => RigidBodyBuilder::dynamic().additional_mass(刚体.质量),
                刚体类型::运动学 => RigidBodyBuilder::kinematic_position_based(),
            }
            .pose(位姿)
            .gravity_scale(刚体.重力缩放)
            // 轻微阻尼：没有阻尼时"永远滑动或永远旋转的箱子"看起来像 bug。
            .linear_damping(0.05)
            .angular_damping(0.05);

            let 碰撞构造 = match 碰撞.形状 {
                碰撞形状3d::立方体 => ColliderBuilder::cuboid(
                    碰撞.宽.max(0.001) * 0.5,
                    碰撞.高.max(0.001) * 0.5,
                    碰撞.深.max(0.001) * 0.5,
                ),
                碰撞形状3d::球体 => ColliderBuilder::ball(碰撞.半径.max(0.001)),
                碰撞形状3d::胶囊体 => {
                    ColliderBuilder::capsule_y(碰撞.半高.max(0.001), 碰撞.半径.max(0.001))
                }
                碰撞形状3d::圆柱体 => {
                    ColliderBuilder::cylinder(碰撞.半高.max(0.001), 碰撞.半径.max(0.001))
                }
            }
            .sensor(碰撞.触发器)
            .friction(碰撞.摩擦)
            .restitution(碰撞.弹性);

            let (刚体句柄, _) = 物理.世界.insert(刚体构造, 碰撞构造);
            let 运行时 = 映射.取运行时(标识);
            if 运行时.is_none() {
                物理.诊断.无运行时实体 += 1;
                物理
                    .诊断
                    .问题
                    .push(format!("实体「{名字}」没有对应的运行时实体，只模拟不回写"));
            }
            物理.实体表.push(实体对应 {
                标识,
                运行时,
                刚体: 刚体句柄,
            });
            物理.诊断.刚体数 += 1;
            物理.诊断.碰撞体数 += 1;
        }

        物理
    }

    /// 诊断信息。
    pub fn 诊断(&self) -> &物理3d诊断 {
        &self.诊断
    }

    /// 物理侧实体数（有碰撞体的文档实体数）。
    pub fn 实体数(&self) -> usize {
        self.实体表.len()
    }

    /// 推进 `经过秒数`，返回实际步进的步数。
    ///
    /// 固定步长累加器：144Hz 时有的帧一步都不走，30Hz 时一帧走两步 ——
    /// 物理结果因此与帧率无关，测试可以稳定断言"第几秒在哪"。
    pub fn 推进(&mut self, 经过秒数: f32) -> u32 {
        if !经过秒数.is_finite() || 经过秒数 <= 0.0 {
            return 0;
        }
        self.累积时间 += 经过秒数;
        let mut 步数 = 0;
        while self.累积时间 >= 固定步长 && 步数 < 最大补步数 {
            self.世界.step();
            self.累积时间 -= 固定步长;
            步数 += 1;
        }
        // 卡顿后丢弃积压，避免"追帧"越来越慢。
        if self.累积时间 > 固定步长 * 最大补步数 as f32 {
            self.累积时间 = 0.0;
        }
        步数
    }

    /// 把物理结果写回运行时世界的 `Transform`。
    ///
    /// 写平移与完整旋转：三维刚体有偏航/俯仰/翻滚三个自由度，
    /// 只写绕 Y 轴会让翻滚的箱子看起来被钉住。
    pub fn 写回(&self, 世界: &mut World) {
        for 对应 in &self.实体表 {
            let Some(运行时) = 对应.运行时 else {
                continue;
            };
            let Some(刚体) = self.世界.bodies.get(对应.刚体) else {
                continue;
            };
            let 平移 = 刚体.translation();
            let 四元数 = 刚体.rotation();
            let Ok(mut 实体) = 世界.get_entity_mut(运行时) else {
                continue;
            };
            if let Some(mut 变换) = 实体.get_mut::<Transform>() {
                变换.translation = Vec3::new(平移.x, 平移.y, 平移.z);
                变换.rotation = Quat::from_xyzw(四元数.x, 四元数.y, 四元数.z, 四元数.w);
            }
        }
    }

    /// 读某实体的当前物理平移（测试与状态显示用）。
    pub fn 实体平移(&self, 标识: 实体标识) -> Option<(f32, f32, f32)> {
        let 对应 = self.实体表.iter().find(|对应| 对应.标识 == 标识)?;
        let 刚体 = self.世界.bodies.get(对应.刚体)?;
        let 平移 = 刚体.translation();
        Some((平移.x, 平移.y, 平移.z))
    }

    /// 读某实体的当前速度（测试用：证明"确实在动"）。
    pub fn 实体速度(&self, 标识: 实体标识) -> Option<(f32, f32, f32)> {
        let 对应 = self.实体表.iter().find(|对应| 对应.标识 == 标识)?;
        let 刚体 = self.世界.bodies.get(对应.刚体)?;
        let 速度 = 刚体.linvel();
        Some((速度.x, 速度.y, 速度.z))
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::{变换数据, 组件数据};
    use scene_doc::property::{属性值, 枚举值};

    /// 往文档里加一个带三维碰撞体的实体。
    fn 加实体(
        文档: &mut 场景文档,
        名称: &str,
        平移: (f32, f32, f32),
        刚体类型名: Option<(&str, u32)>,
    ) -> 实体标识 {
        let 标识 = 文档.生成实体(名称, None);
        let 实体 = 文档.取可变(标识).expect("刚生成");
        实体.变换 = Some(变换数据 {
            平移,
            ..变换数据::单位()
        });
        实体.组件.push(组件数据::新建(
            碰撞体3D组件名,
            [
                ("形状".to_string(), 属性值::文本("Box".to_string())),
                ("宽".to_string(), 属性值::浮点(1.0)),
                ("高".to_string(), 属性值::浮点(1.0)),
                ("深".to_string(), 属性值::浮点(1.0)),
            ]
            .into_iter()
            .collect(),
        ));
        if let Some((变体名, 序号)) = 刚体类型名 {
            实体.组件.push(组件数据::新建(
                刚体3D组件名,
                [
                    ("类型".to_string(), 属性值::枚举(枚举值::新建(变体名, 序号))),
                    ("质量".to_string(), 属性值::浮点(1.0)),
                ]
                .into_iter()
                .collect(),
            ));
        }
        标识
    }

    #[test]
    fn 纯物理世界重力让动态刚体下落() {
        let mut 文档 = 场景文档::新建("三维重力");
        let 球 = 加实体(&mut 文档, "球", (0.0, 5.0, 0.0), Some(("Dynamic", 1)));
        let 地面 = 加实体(&mut 文档, "地面", (0.0, 0.0, 0.0), Some(("Static", 0)));

        let 映射 = 实体映射表::新建();
        let mut 物理 = 物理3d运行时::从文档构建(&文档, &映射);
        assert_eq!(物理.实体数(), 2);
        assert_eq!(物理.诊断().刚体数, 2);

        let 初始 = 物理.实体平移(球).expect("球应当在物理世界里").1;
        assert!((初始 - 5.0).abs() < 1e-6);

        // 跑满一秒（60 个固定步）。
        for _ in 0..60 {
            物理.推进(固定步长);
        }

        let 之后 = 物理.实体平移(球).expect("球应当在物理世界里").1;
        assert!(
            之后 < 初始 - 3.0,
            "一秒后应当明显下落：初始 {初始}，现在 {之后}"
        );
        let 地面_y = 物理.实体平移(地面).expect("地面应当在物理世界里").1;
        assert!((地面_y - 0.0).abs() < 1e-6, "静态刚体不应移动");
    }

    #[test]
    fn 运动学刚体不受重力影响() {
        let mut 文档 = 场景文档::新建("运动学");
        let 平台 = 加实体(&mut 文档, "平台", (0.0, 4.0, 0.0), Some(("Kinematic", 2)));

        let 映射 = 实体映射表::新建();
        let mut 物理 = 物理3d运行时::从文档构建(&文档, &映射);
        for _ in 0..60 {
            物理.推进(固定步长);
        }
        let y = 物理.实体平移(平台).expect("平台应当在物理世界里").1;
        assert!((y - 4.0).abs() < 1e-4, "运动学刚体不应受重力下落，实际 {y}");
    }
}
