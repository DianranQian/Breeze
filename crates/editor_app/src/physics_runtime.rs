//! 2D 物理模拟：让 `碰撞体2d` / `刚体2d` 在**运行态**真正参与模拟。
//!
//! # 引擎选型（先核实事实，再决定）
//!
//! 本项目用 **Bevy 0.20**。候选引擎在 crates.io 上的实际依赖（2025 年数据）：
//!
//! | 引擎 | 版本 | 依赖的 Bevy | 结论 |
//! | --- | --- | --- | --- |
//! | Avian 2D | 0.7.0 | `bevy ^0.19` | **不兼容 0.20**，排除 |
//! | bevy_rapier2d | 0.37.0 | `bevy ^0.20` | 兼容（Apache-2.0） |
//! | bevy_xpbd_2d | 0.5.0 | `bevy ^0.14` | 太旧，排除 |
//!
//! 首选是 Avian，但它要求 `bevy ^0.19`；本项目不能为了物理升降 Bevy 版本
//! （会牵动 `bevy_egui` 0.43、`egui_dock` 0.21 等一整套依赖）。
//! 因此采用 Rapier：**直接依赖 `rapier2d 0.36`**，它正是 `bevy_rapier2d 0.37`
//! 为 Bevy 0.20 编译的那一份。
//!
//! # 为什么不挂 `bevy_rapier2d` 插件
//!
//! 运行时世界是一个**独立的 `World`**（见 [`crate::scene_bridge::运行时宿主`]），
//! 不是第二个 `App`：没有自己的调度、`Time`、`Assets`。`bevy_rapier2d` 是 `App`
//! 插件，硬挂上去要把运行时改造成第二个 App，代价远大于收益。
//!
//! Rapier 0.36 有 [`PhysicsWorld`] 这个一站式封装（刚体集、碰撞体集、积分参数、
//! 流水线都在里面），直接驱动它反而更简单：
//!
//! 1. **构建**：读场景文档里的 `碰撞体2d`/`刚体2d`/变换 → 生成刚体与碰撞体，
//!    并用 [`实体映射表`] 记住"文档实体 ↔ 运行时实体"；
//! 2. **步进**：固定步长累加器 + `step_with_events`，帧率不影响结果；
//! 3. **写回**：刚体的平移/角度写进**运行时世界**的 `Transform`。
//!
//! 边界很关键：**物理只改运行时世界，绝不碰文档**。所以"打开场景对象自己掉落"
//! 不可能发生，已保存的场景也不会被运行时的姿态污染。
//!
//! # 与三维模块的关系
//!
//! 这个文件只处理 `碰撞体2d` / `刚体2d`。三维版本在
//! [`crate::physics3d_runtime`]：同一套"独立世界 + 固定步长 + 只写回运行时
//! `Transform`"的边界，只是换了 Rapier 的 3D 后端与重力口径（-9.81 m/s²）。
//! 两者由同一个 [`物理运行时状态`] 持有，一起启动、一起停止。
//!
//! # 已知限制（诚实说明）
//!
//! * 场景格式里没有关节/弹簧，因此没有关节。
//! * 速度不写回文档（与 Godot 一致：运行时姿态不落盘）。
//! * 触发器只上报"进入/离开"，不做脚本回调（脚本能力尚未接入）。

use std::collections::BTreeMap;
use std::sync::Mutex;

use bevy::prelude::*;
use rapier2d::prelude::*;
use scene_doc::document::{场景文档, 实体标识};
use scene_doc::property::属性值;

use editor_core::collision::{刚体组件名, 碰撞体, 碰撞体组件名, 碰撞形状};

use crate::scene_bridge::实体映射表;

/// 重力（世界单位/秒²）。
///
/// 场景约定 1 世界单位 = 1 像素，因此按像素口径给一个"看得见的下落"：
/// 980 px/s² 是常见平台跳跃游戏的重力手感。
pub const 重力加速度: f32 = -980.0;

/// 固定步长（秒）。物理按固定步长推进，渲染帧率变化不改变模拟结果。
pub const 固定步长: f32 = 1.0 / 60.0;

/// 单帧最多补多少步。
///
/// 卡顿后一次补上百步会穿墙并卡死；宁可让模拟慢一点，也不追帧到失控。
pub const 最大补步数: u32 = 4;

/// 刚体在物理侧的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 物理刚体类型 {
    /// 静态：不动。
    静态,
    /// 动态：受重力与碰撞影响。
    动态,
    /// 运动学：由变换驱动，不被碰撞推动。
    运动学,
}

impl 物理刚体类型 {
    /// 从场景属性里的枚举名解析（兼容中英文变体名）。
    pub fn 由变体名(名字: &str) -> Self {
        match 名字 {
            "Kinematic" | "运动学" | "运动学刚体" => 物理刚体类型::运动学,
            "Dynamic" | "动态" | "动态刚体" => 物理刚体类型::动态,
            _ => 物理刚体类型::静态,
        }
    }
}

/// 一次物理推进中产生的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 物理事件 {
    /// 两个实体开始接触。`传感器` 为真表示至少一方是触发器。
    开始接触 {
        /// 一方实体名。
        甲: String,
        /// 另一方实体名。
        乙: String,
        /// 是否涉及触发器（触发器不产生碰撞响应，只报告进入/离开）。
        传感器: bool,
    },
    /// 两个实体结束接触。
    结束接触 {
        /// 一方实体名。
        甲: String,
        /// 另一方实体名。
        乙: String,
    },
}

/// 构建物理世界时的统计与问题列表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 物理诊断 {
    /// 建立的刚体数。
    pub 刚体数: usize,
    /// 建立的碰撞体数。
    pub 碰撞体数: usize,
    /// 没有对应运行时实体的实体数（只模拟，不回写）。
    pub 无运行时实体: usize,
    /// 问题说明。
    pub 问题: Vec<String>,
}

/// 碰撞事件收集器：Rapier 的回调是 `&self`，因此用 `Mutex` 做内部可变。
#[derive(Default)]
struct 事件收集 {
    事件: Mutex<Vec<CollisionEvent>>,
}

impl EventHandler for 事件收集 {
    /// 接触力事件：本轮不做力反馈（没有脚本可读），显式忽略而不是留空实现。
    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: f32,
    ) {
    }

    /// 软体撕裂事件：场景格式没有软体，忽略。
    fn handle_soft_body_tear_event(&self, _soft_bodies: &SoftBodySet, _event: &SoftBodyTearEvent) {}

    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        if let Ok(mut 列表) = self.事件.lock() {
            列表.push(event);
        }
    }
}

/// 一个文档实体在物理侧的对应物。
#[derive(Debug, Clone, Copy)]
struct 实体对应 {
    /// 文档标识（用于日志与测试定位）。
    标识: 实体标识,
    /// 运行时实体。`None` 表示这次运行没构建出该实体（只模拟不回写）。
    运行时: Option<Entity>,
    /// 刚体句柄。
    刚体: RigidBodyHandle,
}

/// 运行时 2D 物理世界。
pub struct 物理运行时 {
    世界: PhysicsWorld,
    收集器: 事件收集,
    实体表: Vec<实体对应>,
    碰撞体名字: BTreeMap<u32, String>,
    传感器: BTreeMap<u32, bool>,
    累积时间: f32,
    诊断: 物理诊断,
    事件: Vec<物理事件>,
}

impl std::fmt::Debug for 物理运行时 {
    fn fmt(&self, 格式: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        格式
            .debug_struct("物理运行时")
            .field("刚体数", &self.诊断.刚体数)
            .field("碰撞体数", &self.诊断.碰撞体数)
            .field("问题", &self.诊断.问题.len())
            .finish()
    }
}

/// 从场景文档里读出 `碰撞体2d` 的属性。
fn 读碰撞体(文档: &场景文档, 标识: 实体标识) -> Option<碰撞体> {
    let 实体 = 文档.取(标识)?;
    let 组件 = 实体.取组件(碰撞体组件名)?;
    Some(碰撞体::从属性(&组件.属性))
}

/// 从场景文档里读出 `刚体2d` 的三项关键属性。
///
/// 类型缺失时按静态处理：一个只有碰撞体的实体在 Godot 里也是静态障碍，
/// 这比"忽略该实体"更符合直觉（用户加了碰撞体就期望它挡人）。
fn 读刚体(文档: &场景文档, 标识: 实体标识) -> (物理刚体类型, f32, f32) {
    let Some(实体) = 文档.取(标识) else {
        return (物理刚体类型::静态, 1.0, 1.0);
    };
    let Some(组件) = 实体.取组件(刚体组件名) else {
        return (物理刚体类型::静态, 1.0, 1.0);
    };
    let 类型 = 组件
        .属性
        .get("类型")
        .and_then(|值| match 值 {
            属性值::枚举(枚举) => Some(物理刚体类型::由变体名(&枚举.变体名)),
            _ => None,
        })
        .unwrap_or(物理刚体类型::静态);
    let 质量 = 组件
        .属性
        .get("质量")
        .and_then(属性值::取浮点)
        .unwrap_or(1.0)
        .max(0.0001);
    let 重力缩放 = 组件
        .属性
        .get("重力缩放")
        .and_then(属性值::取浮点)
        .unwrap_or(1.0);
    (类型, 质量, 重力缩放)
}

impl 物理运行时 {
    /// 从场景文档构建物理世界。
    ///
    /// `映射` 用来把文档实体的结果写回运行时实体；构建阶段只读文档，
    /// 因此不会污染正在编辑的场景。
    pub fn 从文档构建(文档: &场景文档, 映射: &实体映射表) -> Self {
        let mut 物理 = 物理运行时 {
            世界: PhysicsWorld::default(),
            收集器: 事件收集::default(),
            实体表: Vec::new(),
            碰撞体名字: BTreeMap::new(),
            传感器: BTreeMap::new(),
            累积时间: 0.0,
            诊断: 物理诊断::default(),
            事件: Vec::new(),
        };
        物理.世界.gravity = Vector::new(0.0, 重力加速度);
        物理.世界.integration_parameters.dt = 固定步长;

        // 用实体表的键顺序遍历：BTreeMap 保证同一份文档每次构建的刚体顺序一致，
        // 事件与诊断的输出因此可复现。
        for 标识 in 文档.实体表.keys().copied() {
            let Some(碰撞) = 读碰撞体(文档, 标识) else {
                continue;
            };
            let (类型, 质量, 重力缩放) = 读刚体(文档, 标识);
            let 实体 = 文档.取(标识);
            let 名字 = 实体.map(|实体| 实体.名称.clone()).unwrap_or_default();
            let 变换 = 实体.and_then(|实体| 实体.变换).unwrap_or_default();
            let 角度 = 四元数转_z_角度(变换.旋转);

            let 刚体构造 = match 类型 {
                物理刚体类型::静态 => RigidBodyBuilder::fixed(),
                物理刚体类型::动态 => RigidBodyBuilder::dynamic().additional_mass(质量),
                物理刚体类型::运动学 => RigidBodyBuilder::kinematic_position_based(),
            }
            .translation(Vector::new(变换.平移.0, 变换.平移.1))
            .rotation(角度)
            .gravity_scale(重力缩放)
            // 轻微阻尼：没有摩擦/阻尼时"永远滑动的箱子"看起来像 bug。
            .linear_damping(0.05)
            .angular_damping(0.05);

            let 碰撞构造 = match 碰撞.形状 {
                碰撞形状::矩形 => {
                    ColliderBuilder::cuboid(碰撞.宽度.max(0.001) * 0.5, 碰撞.高度.max(0.001) * 0.5)
                }
                碰撞形状::圆形 => ColliderBuilder::ball(碰撞.半径.max(0.001)),
            }
            .sensor(碰撞.触发器)
            // 至少一方开启 COLLISION_EVENTS 才会回调事件；触发器也靠它驱动。
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .friction(0.6)
            .restitution(0.0);

            let (刚体句柄, 碰撞句柄) = 物理.世界.insert(刚体构造, 碰撞构造);
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
            物理.碰撞体名字.insert(碰撞句柄.into_raw_parts().0, 名字);
            物理.传感器.insert(碰撞句柄.into_raw_parts().0, 碰撞.触发器);
            物理.诊断.刚体数 += 1;
            物理.诊断.碰撞体数 += 1;
        }

        物理
    }

    /// 诊断信息。
    pub fn 诊断(&self) -> &物理诊断 {
        &self.诊断
    }

    /// 取走本轮产生的全部事件。
    pub fn 取出事件(&mut self) -> Vec<物理事件> {
        std::mem::take(&mut self.事件)
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
            self.走一步();
            self.累积时间 -= 固定步长;
            步数 += 1;
        }
        // 卡顿后丢弃积压，避免"追帧"越来越慢。
        if self.累积时间 > 固定步长 * 最大补步数 as f32 {
            self.累积时间 = 0.0;
        }
        步数
    }

    fn 走一步(&mut self) {
        self.世界.step_with_events(&(), &self.收集器);
        let 原始 = match self.收集器.事件.lock() {
            Ok(mut 列表) => std::mem::take(&mut *列表),
            Err(_) => return,
        };
        for 事件 in 原始 {
            match 事件 {
                CollisionEvent::Started(甲, 乙, _) => {
                    let (甲号, 乙号) = (甲.into_raw_parts().0, 乙.into_raw_parts().0);
                    let 传感器 = self.传感器.get(&甲号).copied().unwrap_or(false)
                        || self.传感器.get(&乙号).copied().unwrap_or(false);
                    let 甲名 = self.碰撞体名字.get(&甲号).cloned().unwrap_or_default();
                    let 乙名 = self.碰撞体名字.get(&乙号).cloned().unwrap_or_default();
                    // 只上报有名字的一侧，避免日志里全是空字符串。
                    if 甲名.is_empty() && 乙名.is_empty() {
                        continue;
                    }
                    self.事件.push(物理事件::开始接触 {
                        甲: 甲名,
                        乙: 乙名,
                        传感器,
                    });
                }
                CollisionEvent::Stopped(甲, 乙, _) => {
                    let (甲号, 乙号) = (甲.into_raw_parts().0, 乙.into_raw_parts().0);
                    let 甲名 = self.碰撞体名字.get(&甲号).cloned().unwrap_or_default();
                    let 乙名 = self.碰撞体名字.get(&乙号).cloned().unwrap_or_default();
                    if 甲名.is_empty() && 乙名.is_empty() {
                        continue;
                    }
                    self.事件.push(物理事件::结束接触 {
                        甲: 甲名, 乙: 乙名
                    });
                }
            }
        }
    }

    /// 把物理结果写回运行时世界的 `Transform`。
    ///
    /// 只写平移与绕 Z 旋转：2D 物理没有别的自由度，写其它字段只会引入噪声。
    pub fn 写回(&self, 世界: &mut World) {
        for 对应 in &self.实体表 {
            let Some(运行时) = 对应.运行时 else {
                continue;
            };
            let Some(刚体) = self.世界.bodies.get(对应.刚体) else {
                continue;
            };
            let 平移 = 刚体.translation();
            let 角度 = 刚体.rotation().angle();
            let Ok(mut 实体) = 世界.get_entity_mut(运行时) else {
                continue;
            };
            if let Some(mut 变换) = 实体.get_mut::<Transform>() {
                变换.translation.x = 平移.x;
                变换.translation.y = 平移.y;
                变换.rotation = Quat::from_rotation_z(角度);
            }
        }
    }

    /// 读某实体的当前物理平移（测试与状态显示用）。
    pub fn 实体平移(&self, 标识: 实体标识) -> Option<(f32, f32)> {
        let 对应 = self.实体表.iter().find(|对应| 对应.标识 == 标识)?;
        let 刚体 = self.世界.bodies.get(对应.刚体)?;
        let 平移 = 刚体.translation();
        Some((平移.x, 平移.y))
    }

    /// 读某实体的当前速度（测试用：证明"确实在动"）。
    pub fn 实体速度(&self, 标识: 实体标识) -> Option<(f32, f32)> {
        let 对应 = self.实体表.iter().find(|对应| 对应.标识 == 标识)?;
        let 刚体 = self.世界.bodies.get(对应.刚体)?;
        let 速度 = 刚体.linvel();
        Some((速度.x, 速度.y))
    }
}

/// 把场景里的四元数 `(x, y, z, w)` 转成 2D 角度（绕 Z）。
fn 四元数转_z_角度(四元数: (f32, f32, f32, f32)) -> f32 {
    let (x, y, z, w) = 四元数;
    // 2D 物理只关心绕 Z 的分量；归一化失败（脏数据）时退回 0。
    let 长度 = (x * x + y * y + z * z + w * w).sqrt();
    if !长度.is_finite() || 长度 <= 1e-6 {
        return 0.0;
    }
    let (x, y, z, w) = (x / 长度, y / 长度, z / 长度, w / 长度);
    let _ = (x, y);
    // 绕 Z 的扭转角：`atan2(2(wz), 1 - 2z²)`。
    (2.0 * w * z).atan2(1.0 - 2.0 * z * z)
}

/// 编辑器持有的物理运行时状态（`None` = 当前没有运行的物理世界）。
///
/// 二维与三维共用这一个资源：它们只在点击「运行」时一起构建、停止时一起
/// 丢弃。分成两个资源会让"忘记重置其中一个"变成一类新的 bug，而两者
/// 的生命周期本来就完全一致。
#[derive(Resource, Debug, Default)]
pub struct 物理运行时状态 {
    /// 运行中的二维物理世界。
    pub 当前: Option<物理运行时>,
    /// 上一次构建的二维实体数（停止后仍可显示，便于排查"为什么没模拟"）。
    pub 上次实体数: usize,
    /// 上一次构建的二维诊断。
    pub 上次诊断: 物理诊断,
    /// 运行中的三维物理世界。
    pub 当前三维: Option<crate::physics3d_runtime::物理3d运行时>,
    /// 上一次构建的三维实体数。
    pub 上次实体数三维: usize,
    /// 上一次构建的三维诊断。
    pub 上次诊断三维: crate::physics3d_runtime::物理3d诊断,
}

impl 物理运行时状态 {
    /// 开始运行：从文档构建二维与三维两个物理世界。
    ///
    /// 两边各自只认领自己的组件（`碰撞体2d` / `碰撞体3d`），因此同一份
    /// 场景里两种物理可以共存，互不干扰。
    pub fn 开始(&mut self, 文档: &场景文档, 映射: &实体映射表) {
        let 物理 = 物理运行时::从文档构建(文档, 映射);
        self.上次实体数 = 物理.实体数();
        self.上次诊断 = 物理.诊断().clone();
        self.当前 = Some(物理);

        let 三维 = crate::physics3d_runtime::物理3d运行时::从文档构建(文档, 映射);
        self.上次实体数三维 = 三维.实体数();
        self.上次诊断三维 = 三维.诊断().clone();
        // 没有任何三维碰撞体时不留空世界：空 Rapier 世界每帧步进一次
        // 没有任何收益，却会让"当前三维"这个字段失去诊断意义。
        self.当前三维 = (三维.实体数() > 0).then_some(三维);
    }

    /// 停止运行：整体丢弃（运行时世界实体也一并消失）。
    pub fn 停止(&mut self) {
        self.当前 = None;
        self.当前三维 = None;
    }
}
