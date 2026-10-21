//! 游戏端显式注册表：**只有注册过的组件/资源**才会被采集。
//!
//! # 为什么不自动反射整个世界
//!
//! 三个理由，每一个都足以否决"自动把所有组件序列化"：
//!
//! 1. 不是所有组件都实现 `Reflect`；强行要求会逼用户改游戏代码。
//! 2. 即使实现了 `Reflect`，也不代表适合送到编辑器：里面可能有句柄、
//!    密钥、用户隐私或巨大的二进制缓存。
//! 3. 自动采集会把"哪些数据会离开进程"变成不可审计的事实。
//!
//! 因此适配器采用**白名单式检查器**：用户为每个类型提供一个小提取函数，
//! 只返回愿意展示的结构化字段。未注册的组件仍然会出现在实体详情里，
//! 但只显示类型名与"不支持检查"。

use std::any::TypeId;
use std::sync::Arc;

use bevy::prelude::*;
use runtime_protocol::{字段值, 字段视图, 操作错误, 操作错误代码, 数值范围};

/// 单个组件最多返回的字段数。
pub const 最大字段数: usize = 64;

/// 组件提取器。
pub type 组件提取器 = Arc<dyn for<'w> Fn(EntityRef<'w>) -> Option<Vec<字段视图>> + Send + Sync>;

/// 资源提取器。返回 `None` 表示当前世界里没有这个资源。
pub type 资源提取器 = Arc<dyn Fn(&World) -> Option<Vec<字段视图>> + Send + Sync>;

/// 一次成功写入的前后值。
#[derive(Debug, Clone, PartialEq)]
pub struct 写入结果 {
    /// 修改前值。
    pub 修改前值: 字段值,
    /// 修改后值（以组件真实值为准，例如旋转已归一化）。
    pub 最终值: 字段值,
}

/// 组件写入器。参数：世界、实体、字段、分量下标、目标值。
pub type 组件写入器 = Arc<
    dyn Fn(&mut World, Entity, &str, Option<u32>, &字段值) -> Result<写入结果, 操作错误>
        + Send
        + Sync,
>;

/// 一个可写字段的声明。只有出现在这里、且检查器提供写入器的字段才可写。
#[derive(Debug, Clone, PartialEq)]
pub struct 可写字段声明 {
    /// 字段名。
    pub 字段: String,
    /// 数值范围。
    pub 范围: Option<数值范围>,
    /// 是否允许按分量写入。
    pub 允许索引: bool,
    /// 面向用户的说明。
    pub 说明: String,
}

impl 可写字段声明 {
    /// 新建一个可写字段声明。
    pub fn 新建(字段: impl Into<String>, 范围: Option<数值范围>) -> Self {
        Self {
            字段: 字段.into(),
            范围,
            允许索引: true,
            说明: String::new(),
        }
    }

    /// 禁止按分量写入（例如四元数必须整体归一化）。
    pub fn 只允许整体(mut self) -> Self {
        self.允许索引 = false;
        self
    }

    /// 附加说明。
    pub fn 说明(mut self, 说明: impl Into<String>) -> Self {
        self.说明 = 说明.into();
        self
    }
}

/// 一个组件检查器。
#[derive(Clone)]
pub struct 组件检查器 {
    /// 编辑器里显示的名字。
    pub 名称: String,
    /// Rust 类型名。
    pub 类型名: String,
    /// 类型标识。
    pub 类型标识: TypeId,
    /// 提取函数。
    pub 提取: 组件提取器,
    /// 可选的白名单写入器。
    pub 写入: Option<组件写入器>,
    /// 可写字段声明。
    pub 可写字段: Vec<可写字段声明>,
}

/// 一个资源检查器。
#[derive(Clone)]
pub struct 资源检查器 {
    /// 编辑器里显示的名字。
    pub 名称: String,
    /// Rust 类型名。
    pub 类型名: String,
    /// 提取函数。
    pub 提取: 资源提取器,
}

/// 适配器注册表（Bevy 资源）。
#[derive(Resource, Clone)]
pub struct 调试注册 {
    组件: Arc<Vec<组件检查器>>,
    资源: Arc<Vec<资源检查器>>,
}

impl 调试注册 {
    /// 从构建器生成。
    pub fn 新建(组件: Vec<组件检查器>, 资源: Vec<资源检查器>) -> Self {
        Self {
            组件: Arc::new(组件),
            资源: Arc::new(资源),
        }
    }

    /// 全部组件检查器。
    pub fn 组件(&self) -> &[组件检查器] {
        &self.组件
    }

    /// 全部资源检查器。
    pub fn 资源(&self) -> &[资源检查器] {
        &self.资源
    }

    /// 按类型查找组件检查器。
    pub fn 按类型(&self, 类型: TypeId) -> Option<&组件检查器> {
        self.组件.iter().find(|项| 项.类型标识 == 类型)
    }

    /// 按注册名查找资源检查器。
    pub fn 按名称(&self, 名称: &str) -> Option<&资源检查器> {
        self.资源.iter().find(|项| 项.名称 == 名称)
    }

    /// 按适配器注册名查找组件检查器。
    pub fn 按组件名称(&self, 名称: &str) -> Option<&组件检查器> {
        self.组件.iter().find(|项| 项.名称 == 名称)
    }

    /// 是否有该类型的组件检查器。
    pub fn 有组件(&self, 类型: TypeId) -> bool {
        self.按类型(类型).is_some()
    }
}

/// 注册表构建器。
///
/// 默认包含一组**只读且安全**的常用检查器（Transform / GlobalTransform /
/// Name / Visibility / Time）。用户可以用 [`Self::不注册内置`] 关闭。
#[derive(Clone)]
pub struct 调试注册构建器 {
    组件: Vec<组件检查器>,
    资源: Vec<资源检查器>,
    包含内置: bool,
}

impl Default for 调试注册构建器 {
    fn default() -> Self {
        Self {
            组件: Vec::new(),
            资源: Vec::new(),
            包含内置: true,
        }
    }
}

impl 调试注册构建器 {
    /// 注册一个组件检查器。
    pub fn 注册组件<T, F>(&mut self, 名称: impl Into<String>, 提取: F) -> &mut Self
    where
        T: Component,
        F: for<'w> Fn(EntityRef<'w>) -> Vec<字段视图> + Send + Sync + 'static,
    {
        self.组件.push(组件检查器 {
            名称: 名称.into(),
            类型名: std::any::type_name::<T>().to_string(),
            类型标识: TypeId::of::<T>(),
            提取: Arc::new(move |实体| Some(提取(实体))),
            写入: None,
            可写字段: Vec::new(),
        });
        self
    }

    /// 注册一个带**白名单写入器**的组件检查器。
    ///
    /// 写入器只会在游戏主线程的调试系统里被调用，并且只接受
    /// `可写字段` 中声明的字段。它必须自己完成范围/合法性检查；
    /// 适配器会在调用前再检查预期原值与幂等键。
    pub fn 注册可写组件<T, F, W>(
        &mut self,
        名称: impl Into<String>,
        提取: F,
        写入: W,
        可写字段: Vec<可写字段声明>,
    ) -> &mut Self
    where
        T: Component<Mutability = bevy::ecs::component::Mutable>,
        F: for<'w> Fn(EntityRef<'w>) -> Vec<字段视图> + Send + Sync + 'static,
        W: Fn(&mut T, &str, Option<u32>, &字段值) -> Result<写入结果, 操作错误>
            + Send
            + Sync
            + 'static,
    {
        self.组件.push(组件检查器 {
            名称: 名称.into(),
            类型名: std::any::type_name::<T>().to_string(),
            类型标识: TypeId::of::<T>(),
            提取: Arc::new(move |实体| Some(提取(实体))),
            写入: Some(Arc::new(move |世界, 实体, 字段, 索引, 目标| {
                let Some(mut 组件) = 世界.get_mut::<T>(实体) else {
                    return Err(操作错误 {
                        代码: 操作错误代码::组件不存在,
                        消息: "组件已经被移除".to_string(),
                    });
                };
                写入(&mut 组件, 字段, 索引, 目标)
            })),
            可写字段,
        });
        self
    }

    /// 注册一个资源检查器。`None` 表示资源不存在。
    pub fn 注册资源<R, F>(&mut self, 名称: impl Into<String>, 提取: F) -> &mut Self
    where
        R: Resource,
        F: Fn(&World) -> Option<Vec<字段视图>> + Send + Sync + 'static,
    {
        self.资源.push(资源检查器 {
            名称: 名称.into(),
            类型名: std::any::type_name::<R>().to_string(),
            提取: Arc::new(提取),
        });
        self
    }

    /// 不注册任何内置检查器（完全白名单）。
    pub fn 不注册内置(&mut self) -> &mut Self {
        self.包含内置 = false;
        self
    }

    /// 生成注册表。
    pub fn 构建(&self) -> 调试注册 {
        let mut 组件 = if self.包含内置 {
            内置组件检查器()
        } else {
            Vec::new()
        };
        let mut 资源 = if self.包含内置 {
            内置资源检查器()
        } else {
            Vec::new()
        };
        // 用户注册的同类型检查器覆盖内置项：显式选择优先。
        for 项 in &self.组件 {
            组件.retain(|已有| 已有.类型标识 != 项.类型标识);
            组件.push(项.clone());
        }
        for 项 in &self.资源 {
            资源.retain(|已有| 已有.名称 != 项.名称);
            资源.push(项.clone());
        }
        调试注册::新建(组件, 资源)
    }
}

/// 安全等级：只展示真实字段，不做任何写操作。
pub fn 内置组件检查器() -> Vec<组件检查器> {
    vec![
        组件检查器 {
            名称: "Transform".to_string(),
            类型名: std::any::type_name::<Transform>().to_string(),
            类型标识: TypeId::of::<Transform>(),
            提取: Arc::new(|实体: EntityRef<'_>| 实体.get::<Transform>().map(变换字段)),
            写入: Some(Arc::new(|世界, 实体, 字段, 索引, 目标| {
                let Some(mut 变换) = 世界.get_mut::<Transform>(实体) else {
                    return Err(操作错误 {
                        代码: 操作错误代码::组件不存在,
                        消息: "Transform 已经被移除".to_string(),
                    });
                };
                写变换(&mut 变换, 字段, 索引, 目标)
            })),
            可写字段: vec![
                可写字段声明::新建(
                    "translation",
                    Some(数值范围 {
                        最小: -1_000_000.0,
                        最大: 1_000_000.0,
                    }),
                )
                .说明("世界坐标；可整体或按 x/y/z 分量写入"),
                可写字段声明::新建(
                    "scale",
                    Some(数值范围 {
                        最小: -1_000_000.0,
                        最大: 1_000_000.0,
                    }),
                )
                .说明("缩放；可整体或按 x/y/z 分量写入"),
                可写字段声明::新建("rotation_quat", None)
                    .只允许整体()
                    .说明("四元数必须整体写入，游戏端会归一化；零长度会被拒绝"),
            ],
        },
        组件检查器 {
            名称: "GlobalTransform".to_string(),
            类型名: std::any::type_name::<GlobalTransform>().to_string(),
            类型标识: TypeId::of::<GlobalTransform>(),
            写入: None,
            可写字段: Vec::new(),
            提取: Arc::new(|实体: EntityRef<'_>| {
                实体.get::<GlobalTransform>().map(|变换| {
                    let (横, 纵, 滚) = 变换.rotation().to_euler(EulerRot::XYZ);
                    vec![
                        字段视图::向量("translation", 向量3(变换.translation())),
                        字段视图::向量(
                            "rotation_quat",
                            vec![
                                变换.rotation().x as f64,
                                变换.rotation().y as f64,
                                变换.rotation().z as f64,
                                变换.rotation().w as f64,
                            ],
                        ),
                        字段视图::向量(
                            "rotation_euler_deg",
                            vec![
                                横.to_degrees() as f64,
                                纵.to_degrees() as f64,
                                滚.to_degrees() as f64,
                            ],
                        ),
                        字段视图::向量("scale", 向量3(变换.scale())),
                    ]
                })
            }),
        },
        组件检查器 {
            名称: "Name".to_string(),
            类型名: std::any::type_name::<Name>().to_string(),
            类型标识: TypeId::of::<Name>(),
            写入: None,
            可写字段: Vec::new(),
            提取: Arc::new(|实体: EntityRef<'_>| {
                实体
                    .get::<Name>()
                    .map(|名称| vec![字段视图::文本("name", 名称.as_str())])
            }),
        },
        组件检查器 {
            名称: "Visibility".to_string(),
            类型名: std::any::type_name::<Visibility>().to_string(),
            类型标识: TypeId::of::<Visibility>(),
            写入: None,
            可写字段: Vec::new(),
            提取: Arc::new(|实体: EntityRef<'_>| {
                实体.get::<Visibility>().map(|可见性| {
                    vec![字段视图 {
                        名称: "visibility".to_string(),
                        值: runtime_protocol::字段值::枚举 {
                            类型名: "Visibility".to_string(),
                            变体: format!("{可见性:?}"),
                        },
                        可写: false,
                        范围: None,
                    }]
                })
            }),
        },
    ]
}

/// 内置资源检查器。只包含确定不含敏感数据的通用资源。
pub fn 内置资源检查器() -> Vec<资源检查器> {
    vec![资源检查器 {
        名称: "Time".to_string(),
        类型名: std::any::type_name::<Time>().to_string(),
        提取: Arc::new(|世界: &World| {
            世界.get_resource::<Time>().map(|时间| {
                vec![
                    字段视图::浮点("elapsed_secs", f64::from(时间.elapsed_secs())),
                    字段视图::浮点("delta_secs", f64::from(时间.delta_secs())),
                ]
            })
        }),
    }]
}

/// 把检查器声明的可写字段标记到提取结果上。
pub fn 标记可写字段(检查器: &组件检查器, 字段: &mut [字段视图]) {
    for 项 in 字段 {
        if let Some(声明) = 检查器.可写字段.iter().find(|声明| 声明.字段 == 项.名称)
        {
            项.可写 = true;
            项.范围 = 声明.范围;
        }
    }
}

fn 写变换(
    变换: &mut Transform,
    字段: &str,
    索引: Option<u32>,
    目标: &字段值,
) -> Result<写入结果, 操作错误> {
    let 范围 = 数值范围 {
        最小: -1_000_000.0,
        最大: 1_000_000.0,
    };
    match 字段 {
        "translation" => {
            let (前, 后) = 写向量3(变换.translation, 索引, 目标, Some(范围))?;
            变换.translation = 后;
            Ok(写入结果 {
                修改前值: 字段值::向量(前),
                最终值: 字段值::向量(vec![
                    变换.translation.x as f64,
                    变换.translation.y as f64,
                    变换.translation.z as f64,
                ]),
            })
        }
        "scale" => {
            let (前, 后) = 写向量3(变换.scale, 索引, 目标, Some(范围))?;
            变换.scale = 后;
            Ok(写入结果 {
                修改前值: 字段值::向量(前),
                最终值: 字段值::向量(vec![
                    变换.scale.x as f64,
                    变换.scale.y as f64,
                    变换.scale.z as f64,
                ]),
            })
        }
        "rotation_quat" => {
            if 索引.is_some() {
                return Err(操作错误 {
                    代码: 操作错误代码::值无效,
                    消息: "四元数必须整体写入，不能只写单个分量".to_string(),
                });
            }
            let 目标值 = match 目标 {
                字段值::向量(值) if 值.len() == 4 => 值,
                _ => {
                    return Err(操作错误 {
                        代码: 操作错误代码::值无效,
                        消息: "rotation_quat 需要 4 个有限浮点数".to_string(),
                    });
                }
            };
            if 目标值.iter().any(|值| !值.is_finite()) {
                return Err(操作错误 {
                    代码: 操作错误代码::值无效,
                    消息: "四元数包含 NaN/Infinity".to_string(),
                });
            }
            let 长度 = (目标值[0] * 目标值[0]
                + 目标值[1] * 目标值[1]
                + 目标值[2] * 目标值[2]
                + 目标值[3] * 目标值[3])
                .sqrt();
            if 长度 < 1e-6 {
                return Err(操作错误 {
                    代码: 操作错误代码::值无效,
                    消息: "四元数长度接近 0，无法归一化".to_string(),
                });
            }
            let 四元数 = Quat::from_xyzw(
                (目标值[0] / 长度) as f32,
                (目标值[1] / 长度) as f32,
                (目标值[2] / 长度) as f32,
                (目标值[3] / 长度) as f32,
            );
            if !四元数.is_finite() {
                return Err(操作错误 {
                    代码: 操作错误代码::值无效,
                    消息: "归一化后的四元数不是有限数".to_string(),
                });
            }
            let 前 = 变换.rotation;
            变换.rotation = 四元数;
            Ok(写入结果 {
                修改前值: 字段值::向量(vec![
                    前.x as f64,
                    前.y as f64,
                    前.z as f64,
                    前.w as f64,
                ]),
                最终值: 字段值::向量(vec![
                    变换.rotation.x as f64,
                    变换.rotation.y as f64,
                    变换.rotation.z as f64,
                    变换.rotation.w as f64,
                ]),
            })
        }
        _ => Err(操作错误 {
            代码: 操作错误代码::字段不可写,
            消息: format!("Transform.{字段} 不在写入白名单里"),
        }),
    }
}

fn 写向量3(
    当前: Vec3,
    索引: Option<u32>,
    目标: &字段值,
    范围: Option<数值范围>,
) -> Result<(Vec<f64>, Vec3), 操作错误> {
    let 前 = vec![当前.x as f64, 当前.y as f64, 当前.z as f64];
    let mut 新 = [当前.x as f64, 当前.y as f64, 当前.z as f64];
    match 索引 {
        Some(下标) => {
            let 下标 = 下标 as usize;
            if 下标 >= 3 {
                return Err(操作错误 {
                    代码: 操作错误代码::值无效,
                    消息: "向量分量下标越界".to_string(),
                });
            }
            let 值 = match 目标 {
                字段值::浮点(值) => *值,
                _ => {
                    return Err(操作错误 {
                        代码: 操作错误代码::值无效,
                        消息: "按分量写入需要单个浮点数".to_string(),
                    });
                }
            };
            检查数值(值, 范围)?;
            新[下标] = 值;
        }
        None => {
            let 值 = match 目标 {
                字段值::向量(值) if 值.len() == 3 => 值,
                _ => {
                    return Err(操作错误 {
                        代码: 操作错误代码::值无效,
                        消息: "需要 3 个有限浮点数".to_string(),
                    });
                }
            };
            for (下标, 值) in 值.iter().enumerate() {
                检查数值(*值, 范围)?;
                新[下标] = *值;
            }
        }
    }
    Ok((前, Vec3::new(新[0] as f32, 新[1] as f32, 新[2] as f32)))
}

fn 检查数值(值: f64, 范围: Option<数值范围>) -> Result<(), 操作错误> {
    if !值.is_finite() {
        return Err(操作错误 {
            代码: 操作错误代码::值无效,
            消息: "数值必须是有限数（不能是 NaN/Infinity）".to_string(),
        });
    }
    if let Some(范围) = 范围
        && !范围.包含(值)
    {
        return Err(操作错误 {
            代码: 操作错误代码::值无效,
            消息: format!("数值 {值} 超出允许范围 [{}, {}]", 范围.最小, 范围.最大),
        });
    }
    Ok(())
}

fn 变换字段(变换: &Transform) -> Vec<字段视图> {
    let (横, 纵, 滚) = 变换.rotation.to_euler(EulerRot::XYZ);
    vec![
        字段视图::向量("translation", 向量3(变换.translation)),
        字段视图::向量(
            "rotation_quat",
            vec![
                变换.rotation.x as f64,
                变换.rotation.y as f64,
                变换.rotation.z as f64,
                变换.rotation.w as f64,
            ],
        ),
        字段视图::向量(
            "rotation_euler_deg",
            vec![
                横.to_degrees() as f64,
                纵.to_degrees() as f64,
                滚.to_degrees() as f64,
            ],
        ),
        字段视图::向量("scale", 向量3(变换.scale)),
    ]
}

fn 向量3(值: Vec3) -> Vec<f64> {
    vec![值.x as f64, 值.y as f64, 值.z as f64]
}

/// 截断字段列表，防止某个用户检查器返回上万字段把消息撑爆。
pub fn 截断字段(mut 字段: Vec<字段视图>, 上限: usize) -> Vec<字段视图> {
    字段.truncate(上限);
    字段
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 默认注册表包含常用组件与资源() {
        let 注册 = 调试注册构建器::default().构建();
        assert!(注册.有组件(TypeId::of::<Transform>()));
        assert!(注册.有组件(TypeId::of::<Name>()));
        assert!(注册.按名称("Time").is_some());
    }

    #[test]
    fn 可以完全关闭内置白名单() {
        let mut 构建器 = 调试注册构建器::default();
        构建器.不注册内置();
        let 注册 = 构建器.构建();
        assert!(注册.组件().is_empty());
        assert!(注册.资源().is_empty());
    }

    #[test]
    fn 用户注册覆盖同类型内置检查器() {
        let mut 构建器 = 调试注册构建器::default();
        构建器.注册组件::<Transform, _>("自定义变换", |_| {
            vec![字段视图::文本("说明", "覆盖")]
        });
        let 注册 = 构建器.构建();
        let 项: Vec<_> = 注册
            .组件()
            .iter()
            .filter(|项| 项.类型标识 == TypeId::of::<Transform>())
            .collect();
        assert_eq!(项.len(), 1);
        assert_eq!(项[0].名称, "自定义变换");
    }

    #[test]
    fn 截断字段保留前缀且不越界() {
        let 字段 = vec![字段视图::文本("a", "1"), 字段视图::文本("b", "2")];
        assert_eq!(截断字段(字段.clone(), 1).len(), 1);
        assert_eq!(截断字段(字段, 10).len(), 2);
    }
}
