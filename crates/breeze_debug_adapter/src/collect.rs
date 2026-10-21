//! 真实 ECS 采集：只读地遍历 `World`，把结果映射成协议模型。
//!
//! 这里没有"示例实体"或"占位组件"：每个返回值都来自当前 `World`。
//! 实体被销毁、组件被移除、资源不存在时，结果会变空/返回明确错误，
//! 而不是继续沿用上一帧的缓存。

use std::any::TypeId;
use std::collections::VecDeque;
use std::time::Instant;

use bevy::prelude::*;
use runtime_protocol::{
    世界快照数据, 世界快照请求, 世界概览, 资产统计, 夹紧每页, 字段值, 实体列表请求, 实体列表页, 实体摘要,
    实体详情, 属性修改结果, 属性修改请求, 应用元信息, 操作错误, 操作错误代码, 检查支持级别,
    游标编码, 游标解析, 调试响应, 调试请求, 资源摘要, 资源详情,
};

use crate::config::{支持的BEVY版本, 现在毫秒, 调试配置};
use crate::network::调试连接;
use crate::registry::{写入结果, 截断字段, 调试注册};

/// 一次实体列表扫描的硬上限。
///
/// 超过这个数量时不再继续收集候选集，并在响应里标记 `已截断`。
/// 这样即使一个游戏有上百万实体，一次查询也不会把主线程卡住几秒。
pub const 最大扫描实体数: usize = 50_000;

/// 实体列表默认每页数量。
pub const 默认每页实体数: u32 = 50;

/// 采集运行状态（Bevy 资源）。
#[derive(Resource, Default)]
pub struct 调试状态 {
    /// 自适配器启动以来处理的帧数。
    pub 帧序号: u64,
    /// 快照序号（每次采集 +1）。
    pub 快照序号: u64,
    /// 最近一次握手得到的会话标识。
    pub 会话标识: Option<String>,
    /// 是否已连接（由事件驱动）。
    pub 已连接: bool,
    /// 最近一次采集时间（毫秒）。
    pub 最近采集时间毫秒: u64,
    /// 写入结果序号。
    pub 写入序号: u64,
    /// 最近写入的幂等键与结果（有界，用于超时重试重放）。
    pub 最近写入: VecDeque<(String, 属性修改结果)>,
}

/// 单次写入记录上限。
pub const 写入记录上限: usize = 64;
/// 单次快照最多返回的实体数。
pub const 快照最大实体数: u32 = 1000;

/// 处理后台线程事件并采集响应。
///
/// # 为什么是 exclusive system
///
/// 采集需要读整个 `World`（任意实体、任意已注册组件、任意已注册资源），
/// 用普通 `Query` 参数无法表达"按 TypeId 动态调用用户的提取函数"。
/// 每次最多处理 [`默认最大每帧请求`](crate::config::默认最大每帧请求) 条，
/// 因此不会出现"请求洪水把 schedule 拖死"。
pub fn 处理调试连接(world: &mut World) {
    let Some(连接) = world.get_resource::<调试连接>().cloned() else {
        return;
    };
    if !连接.已启用() {
        return;
    }
    let Some(注册) = world.get_resource::<调试注册>().cloned() else {
        return;
    };
    let 配置 = 连接.配置().cloned().unwrap_or_else(|| {
        调试配置::新建(
            "127.0.0.1:0".parse().expect("固定地址总是合法"),
            "0000000000000000",
            "未知项目",
            "未知运行",
        )
    });
    let mut 状态 = world.remove_resource::<调试状态>().unwrap_or_default();
    let 事件 = 连接.取事件();
    let mut 已处理请求 = 0usize;
    for 项 in 事件 {
        match 项 {
            crate::network::网络事件::已连接(问候) => {
                状态.已连接 = true;
                状态.会话标识 = Some(问候.会话标识);
                info!(
                    "微风调试：已与编辑器握手（会话 {}，上限 {} 字节）",
                    状态.会话标识.as_deref().unwrap_or(""),
                    问候.最大消息字节
                );
            }
            crate::network::网络事件::断开(原因) => {
                状态.已连接 = false;
                warn!("微风调试：连接断开：{原因}");
            }
            crate::network::网络事件::协议错误(错误) => {
                warn!(
                    "微风调试：编辑器报告协议错误 {}：{}",
                    错误.代码.代号(),
                    错误.消息
                );
            }
            crate::network::网络事件::心跳回应 { .. } => {}
            crate::network::网络事件::请求 { 编号, 请求 } => {
                if !状态.已连接 {
                    continue;
                }
                // 每帧硬上限：即使客户端洪水式发请求，也不会让游戏主线程
                // 无限期停在采集逻辑里。超出的请求得到明确失败响应。
                if 已处理请求 >= 配置.最大每帧请求 {
                    连接.发送响应(
                        编号,
                        调试响应::失败(操作错误 {
                            代码: 操作错误代码::查询被取消,
                            消息: format!(
                                "游戏端每帧最多处理 {} 个查询，请降低刷新频率",
                                配置.最大每帧请求
                            ),
                        }),
                    );
                    continue;
                }
                已处理请求 += 1;
                let 响应 =
                    match 请求 {
                        调试请求::修改属性(请求) => {
                            调试响应::属性修改(执行属性修改(world, &注册, &mut 状态, 请求))
                        }
                        调试请求::世界快照(请求) => 调试响应::世界快照(
                            采集世界快照(world, &注册, &配置, 请求, &mut 状态),
                        ),
                        _ => 处理请求(world, &注册, &配置, &mut 状态, 请求),
                    };
                连接.发送响应(编号, 响应);
            }
        }
    }
    状态.帧序号 = 状态.帧序号.wrapping_add(1);
    world.insert_resource(状态);
}

/// 一个请求 → 一个响应。所有分支都只读 `World`。
pub fn 处理请求(
    世界: &World,
    注册: &调试注册,
    配置: &调试配置,
    状态: &mut 调试状态,
    请求: 调试请求,
) -> 调试响应 {
    状态.快照序号 = 状态.快照序号.wrapping_add(1);
    let 开始 = Instant::now();
    let 快照序号 = 状态.快照序号;
    let 结果 = match 请求 {
        调试请求::世界概览 => {
            调试响应::世界概览(采集概览(世界, 注册, 配置, 状态, 快照序号, 开始))
        }
        调试请求::实体列表(请求) => {
            调试响应::实体列表(采集实体列表(世界, 注册, 配置, 请求, 快照序号, 开始))
        }
        调试请求::实体详情 { 实体 } => {
            match 采集实体详情(世界, 注册, 配置, 实体, 快照序号, 开始) {
                Ok(详情) => 调试响应::实体详情(详情),
                Err(错误) => 调试响应::失败(错误),
            }
        }
        调试请求::资源列表 => 调试响应::资源列表(采集资源列表(世界, 注册)),
        调试请求::资源详情 { 名称 } => {
            match 采集资源详情(世界, 注册, &名称, 快照序号, 开始) {
                Some(详情) => 调试响应::资源详情(详情),
                None => 调试响应::失败(操作错误 {
                    代码: 操作错误代码::资源不存在,
                    消息: format!("资源 `{名称}` 未注册或当前世界中不存在"),
                }),
            }
        }
        调试请求::修改属性(_) => 调试响应::失败(操作错误 {
            代码: 操作错误代码::内部错误,
            消息: "修改请求必须走受控写入路径".to_string(),
        }),
        调试请求::世界快照(_) => 调试响应::失败(操作错误 {
            代码: 操作错误代码::内部错误,
            消息: "快照请求必须走快照采集路径".to_string(),
        }),
    };
    状态.最近采集时间毫秒 = 现在毫秒();
    结果
}

fn 采集世界快照(
    世界: &World,
    注册: &调试注册,
    配置: &调试配置,
    请求: 世界快照请求,
    状态: &mut 调试状态,
) -> 世界快照数据 {
    状态.快照序号 = 状态.快照序号.wrapping_add(1);
    let 开始 = Instant::now();
    let 快照序号 = 状态.快照序号;
    let 世界实体总数 = 世界.iter_entities().count() as u64;
    let 上限 = 请求
        .最大实体
        .clamp(1, 快照最大实体数)
        .min(配置.最大扫描实体数 as u32) as usize;
    let mut 实体 = Vec::new();
    let mut 有失败 = false;
    for (下标, 项) in 世界.iter_entities().enumerate() {
        if 下标 >= 上限 {
            break;
        }
        match 采集实体详情(世界, 注册, 配置, 项.id().to_bits(), 快照序号, 开始) {
            Ok(详情) => 实体.push(详情),
            Err(_) => 有失败 = true,
        }
    }
    let 资源 = if 请求.包含资源 {
        注册
            .资源()
            .iter()
            .filter_map(|检查器| {
                (检查器.提取)(世界).map(|字段| 资源详情 {
                    名称: 检查器.名称.clone(),
                    类型名: 检查器.类型名.clone(),
                    字段,
                    说明: None,
                    快照序号,
                    采集时间毫秒: 现在毫秒(),
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    世界快照数据 {
        实体,
        资源,
        已截断: 有失败 || 世界实体总数 > 上限 as u64,
        世界实体总数,
        快照序号,
        采集时间毫秒: 现在毫秒(),
        采集耗时微秒: 开始.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
    }
}

/// 受控写入：唯一会修改游戏世界的入口。
fn 执行属性修改(
    世界: &mut World,
    注册: &调试注册,
    状态: &mut 调试状态,
    请求: 属性修改请求,
) -> 属性修改结果 {
    let 开始 = Instant::now();
    if 请求.请求标识.trim().is_empty() || 请求.请求标识.chars().count() > 128 {
        return 修改结果(
            状态,
            开始,
            false,
            false,
            None,
            None,
            Some(操作错误 {
                代码: 操作错误代码::值无效,
                消息: "请求标识为空或过长".to_string(),
            }),
            false,
            false,
            false,
        );
    }
    if 请求.组件.chars().count() > 128 || 请求.字段.chars().count() > 128 {
        return 修改结果(
            状态,
            开始,
            false,
            false,
            None,
            None,
            Some(操作错误 {
                代码: 操作错误代码::值无效,
                消息: "组件名或字段名过长".to_string(),
            }),
            false,
            false,
            false,
        );
    }
    if let Some((_, 已有)) = 状态.最近写入.iter().find(|(键, _)| 键 == &请求.请求标识)
    {
        let mut 重放 = 已有.clone();
        重放.重复请求 = true;
        return 重放;
    }

    let 实体 = Entity::from_bits(请求.实体);
    let Some(检查器) = 注册.按组件名称(&请求.组件) else {
        return 修改结果(
            状态,
            开始,
            false,
            false,
            None,
            None,
            Some(操作错误 {
                代码: 操作错误代码::组件不存在,
                消息: format!("组件 `{}` 未注册检查器", 请求.组件),
            }),
            false,
            false,
            false,
        );
    };
    let Some(声明) = 检查器.可写字段.iter().find(|声明| 声明.字段 == 请求.字段)
    else {
        return 修改结果(
            状态,
            开始,
            false,
            false,
            None,
            None,
            Some(操作错误 {
                代码: 操作错误代码::字段不可写,
                消息: format!("字段 `{}` 不在写入白名单里", 请求.字段),
            }),
            false,
            false,
            false,
        );
    };
    if 请求.索引.is_some() && !声明.允许索引 {
        return 修改结果(
            状态,
            开始,
            false,
            false,
            None,
            None,
            Some(操作错误 {
                代码: 操作错误代码::值无效,
                消息: "该字段只允许整体写入".to_string(),
            }),
            false,
            false,
            false,
        );
    }
    let Some(写入器) = &检查器.写入 else {
        return 修改结果(
            状态,
            开始,
            false,
            false,
            None,
            None,
            Some(操作错误 {
                代码: 操作错误代码::字段不可写,
                消息: "该组件没有注册写入器".to_string(),
            }),
            false,
            false,
            false,
        );
    };
    let Some(预期原值) = &请求.预期原值 else {
        return 修改结果(
            状态,
            开始,
            false,
            false,
            None,
            None,
            Some(操作错误 {
                代码: 操作错误代码::值无效,
                消息: "必须提供预期原值，拒绝盲写".to_string(),
            }),
            false,
            false,
            false,
        );
    };

    let 当前值 = {
        let Ok(实体引用) = 世界.get_entity(实体) else {
            return 修改结果(
                状态,
                开始,
                false,
                false,
                None,
                None,
                Some(操作错误 {
                    代码: 操作错误代码::实体不存在,
                    消息: "实体已被销毁".to_string(),
                }),
                false,
                false,
                false,
            );
        };
        let Some(字段列表) = (检查器.提取)(实体引用) else {
            return 修改结果(
                状态,
                开始,
                false,
                false,
                None,
                None,
                Some(操作错误 {
                    代码: 操作错误代码::组件不存在,
                    消息: "组件已经从实体上移除".to_string(),
                }),
                true,
                false,
                false,
            );
        };
        let Some(字段) = 字段列表.into_iter().find(|项| 项.名称 == 请求.字段) else {
            return 修改结果(
                状态,
                开始,
                false,
                false,
                None,
                None,
                Some(操作错误 {
                    代码: 操作错误代码::字段不存在,
                    消息: format!("组件上不存在字段 `{}`", 请求.字段),
                }),
                true,
                true,
                false,
            );
        };
        字段.值
    };
    if &当前值 != 预期原值 {
        return 修改结果(
            状态,
            开始,
            false,
            true,
            Some(当前值),
            None,
            Some(操作错误 {
                代码: 操作错误代码::冲突,
                消息: "预期原值与游戏端当前值不一致（可能已被其他来源修改）".to_string(),
            }),
            true,
            true,
            false,
        );
    }

    let 执行 = 写入器(世界, 实体, &请求.字段, 请求.索引, &请求.目标值);
    let 实体仍存在 = 世界.get_entity(实体).is_ok();
    let 组件仍存在 = 实体有类型(世界, 实体, 检查器.类型标识);
    let 结果 = match 执行 {
        Ok(写入结果 {
            修改前值, 最终值
        }) => 修改结果(
            状态,
            开始,
            true,
            false,
            Some(修改前值),
            Some(最终值),
            None,
            实体仍存在,
            组件仍存在,
            false,
        ),
        Err(错误) => 修改结果(
            状态,
            开始,
            false,
            false,
            Some(当前值),
            None,
            Some(错误),
            实体仍存在,
            组件仍存在,
            false,
        ),
    };
    状态.最近写入.push_back((请求.请求标识, 结果.clone()));
    while 状态.最近写入.len() > 写入记录上限 {
        状态.最近写入.pop_front();
    }
    结果
}

#[allow(clippy::too_many_arguments)]
fn 修改结果(
    状态: &mut 调试状态,
    开始: Instant,
    成功: bool,
    冲突: bool,
    修改前值: Option<字段值>,
    最终值: Option<字段值>,
    错误: Option<操作错误>,
    实体仍存在: bool,
    组件仍存在: bool,
    重复请求: bool,
) -> 属性修改结果 {
    状态.写入序号 = 状态.写入序号.wrapping_add(1);
    属性修改结果 {
        成功,
        冲突,
        修改前值,
        最终值,
        错误,
        实体仍存在,
        组件仍存在,
        结果序号: 状态.写入序号,
        执行时间毫秒: 现在毫秒(),
        执行耗时微秒: 开始.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        重复请求,
    }
}

fn 实体有类型(世界: &World, 实体: Entity, 类型: TypeId) -> bool {
    let Ok(实体引用) = 世界.get_entity(实体) else {
        return false;
    };
    实体引用
        .archetype()
        .components()
        .iter()
        .filter_map(|标识| 世界.components().get_info(*标识))
        .any(|信息| 信息.type_id() == Some(类型))
}

fn 采集概览(
    世界: &World,
    注册: &调试注册,
    配置: &调试配置,
    状态: &调试状态,
    快照序号: u64,
    开始: Instant,
) -> 世界概览 {
    世界概览 {
        实体总数: 世界.iter_entities().count() as u64,
        组件类型数: 世界.components().len().min(u32::MAX as usize) as u32,
        已注册组件检查器: 注册.组件().len().min(u32::MAX as usize) as u32,
        已注册资源检查器: 注册.资源().len().min(u32::MAX as usize) as u32,
        快照序号,
        采集时间毫秒: 现在毫秒(),
        采集耗时微秒: 开始.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        帧耗时微秒: 世界
            .get_resource::<Time>()
            .map(|时间| {
                (时间.delta_secs() as f64 * 1_000_000.0)
                    .round()
                    .clamp(0.0, u64::MAX as f64) as u64
            })
            .unwrap_or(0),
        资产: 采集资产统计(世界),
        应用: 应用元信息 {
            应用名称: 配置.应用名称.clone(),
            bevy版本: 支持的BEVY版本.to_string(),
            项目标识: 配置.项目标识.clone(),
            运行实例标识: 配置.运行实例标识.clone(),
            启动时间毫秒: 配置.启动时间毫秒,
            帧序号: 状态.帧序号,
        },
    }
}

/// 采集资产规模。
///
/// 所有数字都来自 Bevy `Assets<T>` 的真实条目：
/// `Mesh` 的顶点/索引数、`Image` 的像素数、`AudioSource` 的字节数。
/// 拿不到某类资产资源（例如游戏没注册 `AudioPlugin`）时记 0 并**不报错**——
/// 那是正常的"没有这一类资产"，不是采集失败。
fn 采集资产统计(世界: &World) -> 资产统计 {
    let mut 统计 = 资产统计::default();

    if let Some(网格) = 世界.get_resource::<Assets<Mesh>>() {
        统计.网格数 = 网格.len().min(u32::MAX as usize) as u32;
        for (_, 资产) in 网格.iter() {
            统计.网格顶点数 = 统计
                .网格顶点数
                .saturating_add(资产.count_vertices() as u64);
            统计.网格索引数 = 统计
                .网格索引数
                .saturating_add(资产.indices().map_or(0, |索引| 索引.len()) as u64);
        }
    }

    if let Some(图片) = 世界.get_resource::<Assets<Image>>() {
        统计.图片数 = 图片.len().min(u32::MAX as usize) as u32;
        for (_, 资产) in 图片.iter() {
            let 尺寸 = 资产.texture_descriptor.size;
            统计.图片像素数 = 统计
                .图片像素数
                .saturating_add(u64::from(尺寸.width).saturating_mul(u64::from(尺寸.height)));
        }
    }

    if let Some(音频) = 世界.get_resource::<Assets<AudioSource>>() {
        统计.音频数 = 音频.len().min(u32::MAX as usize) as u32;
        for (_, 资产) in 音频.iter() {
            统计.音频字节数 = 统计
                .音频字节数
                .saturating_add(资产.bytes.len() as u64);
        }
    }

    if let Some(字体) = 世界.get_resource::<Assets<Font>>() {
        统计.字体数 = 字体.len().min(u32::MAX as usize) as u32;
    }

    统计
}

fn 采集实体列表(
    世界: &World,
    注册: &调试注册,
    配置: &调试配置,
    请求: 实体列表请求,
    快照序号: u64,
    开始: Instant,
) -> 实体列表页 {
    let 每页 = 夹紧每页(请求.每页, 默认每页实体数);
    let 偏移 = 游标解析(请求.游标.as_deref());
    let 文本 = 请求.文本.as_deref().map(|项| 项.trim().to_lowercase());
    let 组件过滤 = 请求.组件.as_deref().map(|项| 项.trim().to_lowercase());
    let 世界实体总数 = 世界.iter_entities().count() as u64;
    let mut 命中: Vec<实体摘要> = Vec::new();
    let mut 已截断 = false;

    for (已扫描, 实体) in 世界.iter_entities().enumerate() {
        if 已扫描 >= 配置.最大扫描实体数.max(1) {
            已截断 = true;
            break;
        }
        let 标识 = 实体.id().to_bits();
        let 名称 = 实体.get::<Name>().map(|项| 项.as_str().to_string());
        let 组件 = 组件类型(世界, 实体.archetype().components());
        if let Some(过滤) = &组件过滤
            && !组件.iter().any(|(类型名, _)| {
                类型名.to_lowercase().contains(过滤) || 短类型名(类型名).to_lowercase() == *过滤
            })
        {
            continue;
        }
        if let Some(过滤) = &文本 {
            let 标识文本 = 标识.to_string();
            let 标识十六进制 = format!("{标识:x}");
            let 命中标识 = 标识文本.contains(过滤) || 标识十六进制.contains(过滤);
            let 命中名称 = 名称
                .as_deref()
                .is_some_and(|项| 项.to_lowercase().contains(过滤));
            if !命中标识 && !命中名称 {
                continue;
            }
        }
        let 可检查组件数 = 组件
            .iter()
            .filter(|(_, 类型)| 类型.is_some_and(|类型| 注册.有组件(类型)))
            .count();
        命中.push(实体摘要 {
            标识,
            名称,
            组件数: 组件.len().min(u32::MAX as usize) as u32,
            可检查组件数: 可检查组件数.min(u32::MAX as usize) as u32,
            摘要: 组件
                .iter()
                .map(|(类型名, _)| 短类型名(类型名).to_string())
                .take(6)
                .collect(),
        });
    }
    命中.sort_by_key(|项| 项.标识);
    let 总数 = 命中.len() as u64;
    let 起点 = usize::try_from(偏移).unwrap_or(usize::MAX).min(命中.len());
    let 终点 = 起点.saturating_add(每页 as usize).min(命中.len());
    let 本页 = 命中[起点..终点].to_vec();
    let 下一页游标 = if 终点 < 命中.len() {
        Some(游标编码(终点 as u64))
    } else {
        None
    };
    实体列表页 {
        实体: 本页,
        下一页游标,
        总数,
        世界实体总数,
        已截断,
        快照序号,
        采集时间毫秒: 现在毫秒(),
        采集耗时微秒: 开始.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
    }
}

fn 采集实体详情(
    世界: &World,
    注册: &调试注册,
    配置: &调试配置,
    标识: u64,
    快照序号: u64,
    开始: Instant,
) -> Result<实体详情, 操作错误> {
    let 实体 = Entity::from_bits(标识);
    let 实体引用 = 世界.get_entity(实体).map_err(|_| 操作错误 {
        代码: 操作错误代码::实体不存在,
        消息: format!("实体 {标识} 已被销毁或不属于当前世界"),
    })?;
    let 名称 = 实体引用.get::<Name>().map(|项| 项.as_str().to_string());
    let mut 组件 = Vec::new();
    for 组件标识 in 实体引用.archetype().components() {
        let Some(信息) = 世界.components().get_info(*组件标识) else {
            continue;
        };
        let 类型名 = 信息.name().to_string();
        let 检查器 = 信息.type_id().and_then(|类型| 注册.按类型(类型));
        match 检查器 {
            Some(检查器) => match (检查器.提取)(实体引用) {
                Some(mut 字段) => {
                    crate::registry::标记可写字段(检查器, &mut 字段);
                    组件.push(runtime_protocol::组件视图 {
                        类型名,
                        支持: 检查支持级别::可检查,
                        字段: 截断字段(字段, 配置.最大字段数),
                        说明: None,
                    })
                }
                None => 组件.push(runtime_protocol::组件视图 {
                    类型名,
                    支持: 检查支持级别::部分支持,
                    字段: Vec::new(),
                    说明: Some("检查器没有返回字段（组件可能已变化）".to_string()),
                }),
            },
            None => 组件.push(runtime_protocol::组件视图 {
                类型名,
                支持: 检查支持级别::不支持,
                字段: Vec::new(),
                说明: Some(
                    "该组件没有注册检查器：只显示类型名，不伪造属性。\
                     要检查它，请在游戏端调用 `Breeze调试插件::注册组件`。"
                        .to_string(),
                ),
            }),
        }
    }
    组件.sort_by(|甲, 乙| 甲.类型名.cmp(&乙.类型名));
    Ok(实体详情 {
        标识,
        名称,
        组件总数: 组件.len().min(u32::MAX as usize) as u32,
        组件,
        快照序号,
        采集时间毫秒: 现在毫秒(),
        采集耗时微秒: 开始.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
    })
}

fn 采集资源列表(世界: &World, 注册: &调试注册) -> Vec<资源摘要> {
    注册
        .资源()
        .iter()
        .map(|检查器| {
            let 存在 = (检查器.提取)(世界).is_some();
            资源摘要 {
                名称: 检查器.名称.clone(),
                类型名: 检查器.类型名.clone(),
                存在,
                可检查: 存在,
                说明: (!存在).then(|| "当前世界中不存在该资源".to_string()),
            }
        })
        .collect()
}

fn 采集资源详情(
    世界: &World,
    注册: &调试注册,
    名称: &str,
    快照序号: u64,
    _开始: Instant,
) -> Option<资源详情> {
    let 检查器 = 注册.按名称(名称)?;
    let 字段 = (检查器.提取)(世界)?;
    Some(资源详情 {
        名称: 检查器.名称.clone(),
        类型名: 检查器.类型名.clone(),
        字段,
        说明: None,
        快照序号,
        采集时间毫秒: 现在毫秒(),
    })
}

/// 收集组件类型名与可选 TypeId（按世界注册顺序）。
fn 组件类型(
    世界: &World,
    组件标识: &[bevy::ecs::component::ComponentId],
) -> Vec<(String, Option<TypeId>)> {
    组件标识
        .iter()
        .filter_map(|标识| 世界.components().get_info(*标识))
        .map(|信息| (信息.name().to_string(), 信息.type_id()))
        .collect()
}

/// 短类型名：去掉模块路径，保留最后一段。
fn 短类型名(完整: &str) -> &str {
    完整.rsplit("::").next().unwrap_or(完整)
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::registry::调试注册构建器;
    use runtime_protocol::字段视图;

    #[derive(Component)]
    struct 测试组件 {
        数值: i32,
    }

    #[derive(Resource)]
    struct 测试资源 {
        名称: String,
    }

    fn 测试世界() -> World {
        let mut 世界 = World::new();
        世界.init_resource::<Time>();
        世界.insert_resource(测试资源 {
            名称: "真实资源".to_string(),
        });
        世界.spawn((
            Name::new("玩家"),
            Transform::from_xyz(1.0, 2.0, 3.0),
            测试组件 { 数值: 7 },
        ));
        世界.spawn((Name::new("敌人"), Transform::default()));
        世界
    }

    fn 配置() -> 调试配置 {
        let mut 配置 = 调试配置::新建(
            "127.0.0.1:9999".parse().unwrap(),
            "0123456789abcdef",
            "项目",
            "运行",
        );
        配置.应用名称 = "测试游戏".to_string();
        配置
    }

    #[test]
    fn 世界概览来自真实世界() {
        let 世界 = 测试世界();
        let 注册 = 调试注册构建器::default().构建();
        let mut 状态 = 调试状态::default();
        let 响应 = 处理请求(&世界, &注册, &配置(), &mut 状态, 调试请求::世界概览);
        let 调试响应::世界概览(概览) = 响应 else {
            panic!("必须是世界概览");
        };
        assert_eq!(概览.实体总数, 世界.iter_entities().count() as u64);
        assert_eq!(概览.应用.应用名称, "测试游戏");
        assert_eq!(概览.应用.bevy版本, "0.20");
        assert!(概览.组件类型数 >= 3);
    }

    #[test]
    fn 实体列表分页与过滤都来自真实实体() {
        let 世界 = 测试世界();
        let 注册 = 调试注册构建器::default().构建();
        let mut 状态 = 调试状态::default();
        let mut 列表 = |请求| {
            let 调试响应::实体列表(页) =
                处理请求(&世界, &注册, &配置(), &mut 状态, 调试请求::实体列表(请求))
            else {
                panic!("必须是列表");
            };
            页
        };
        let 第一页 = 列表(实体列表请求 {
            每页: 2,
            ..Default::default()
        });
        assert_eq!(第一页.总数, 世界.iter_entities().count() as u64);
        assert_eq!(第一页.实体.len(), 2);
        assert!(第一页.下一页游标.is_some());
        let 第二页 = 列表(实体列表请求 {
            每页: 2,
            游标: 第一页.下一页游标.clone(),
            ..Default::default()
        });
        assert_eq!(第二页.实体.len(), 2);
        assert!(第二页.下一页游标.is_some());
        assert_ne!(第一页.实体[0].标识, 第二页.实体[0].标识);
        let 末页 = 列表(实体列表请求 {
            每页: 32,
            游标: Some("0".to_string()),
            ..Default::default()
        });
        assert!(末页.下一页游标.is_none());

        let 过滤 = 列表(实体列表请求 {
            文本: Some("玩家".to_string()),
            ..Default::default()
        });
        assert_eq!(过滤.总数, 1);
        assert_eq!(过滤.实体[0].名称.as_deref(), Some("玩家"));
    }

    #[test]
    fn 实体详情显示已注册字段与不支持组件() {
        let 世界 = 测试世界();
        let mut 构建器 = 调试注册构建器::default();
        构建器.注册组件::<测试组件, _>("测试组件", |实体| {
            实体
                .get::<测试组件>()
                .map(|项| vec![字段视图::文本("数值", 项.数值.to_string())])
                .unwrap_or_default()
        });
        let 注册 = 构建器.构建();
        let mut 状态 = 调试状态::default();
        let 标识 = 世界
            .iter_entities()
            .find(|实体| {
                实体
                    .get::<Name>()
                    .is_some_and(|名称| 名称.as_str() == "玩家")
            })
            .unwrap()
            .id()
            .to_bits();
        let 调试响应::实体详情(详情) = 处理请求(
            &世界,
            &注册,
            &配置(),
            &mut 状态,
            调试请求::实体详情 { 实体: 标识 },
        ) else {
            panic!("必须是详情");
        };
        assert!(
            详情.组件.iter().any(|项| {
                项.类型名.ends_with("Transform") && 项.支持 == 检查支持级别::可检查
            }),
            "组件清单：{:?}",
            详情
                .组件
                .iter()
                .map(|项| (&项.类型名, 项.支持))
                .collect::<Vec<_>>()
        );
        assert!(详情.组件.iter().any(|项| {
            项.类型名.ends_with("测试组件") && 项.支持 == 检查支持级别::可检查
        }));
        let 消息 = 详情.组件.iter().find(|项| 项.类型名.ends_with("Name"));
        // Name 是内置检查器。
        assert!(消息.is_some_and(|项| 项.支持 == 检查支持级别::可检查));
    }

    #[test]
    fn 已销毁实体返回明确的不存在错误() {
        let 世界 = 测试世界();
        let 注册 = 调试注册构建器::default().构建();
        let mut 状态 = 调试状态::default();
        let 响应 = 处理请求(
            &世界,
            &注册,
            &配置(),
            &mut 状态,
            调试请求::实体详情 { 实体: u64::MAX },
        );
        assert!(matches!(
            响应,
            调试响应::失败(错误) if 错误.代码 == 操作错误代码::实体不存在
        ));
    }

    #[test]
    fn 资源列表只列出显式注册项() {
        let 世界 = 测试世界();
        let mut 构建器 = 调试注册构建器::default();
        构建器.注册资源::<测试资源, _>("测试资源", |世界| {
            世界
                .get_resource::<测试资源>()
                .map(|资源| vec![字段视图::文本("名称", 资源.名称.clone())])
        });
        let 注册 = 构建器.构建();
        let mut 状态 = 调试状态::default();
        let 调试响应::资源列表(列表) =
            处理请求(&世界, &注册, &配置(), &mut 状态, 调试请求::资源列表)
        else {
            panic!("必须是资源列表");
        };
        assert!(
            列表
                .iter()
                .any(|项| 项.名称 == "测试资源" && 项.存在 && 项.可检查)
        );
        assert!(列表.iter().any(|项| 项.名称 == "Time"));
        assert!(!列表.iter().any(|项| 项.名称 == "测试资源不存在"));
    }

    #[test]
    fn 白名单写入真实改变transform并拒绝冲突与非法值() {
        let mut 世界 = 测试世界();
        let 注册 = 调试注册构建器::default().构建();
        let mut 状态 = 调试状态::default();
        let 实体 = 世界
            .iter_entities()
            .find(|项| 项.get::<Name>().is_some_and(|名称| 名称.as_str() == "玩家"))
            .unwrap()
            .id();

        let 请求 = |目标: Vec<f64>, 预期: Vec<f64>, 键: &str| 属性修改请求 {
            实体: 实体.to_bits(),
            组件: "Transform".to_string(),
            字段: "translation".to_string(),
            索引: None,
            目标值: 字段值::向量(目标),
            预期原值: Some(字段值::向量(预期)),
            请求标识: 键.to_string(),
            请求时间毫秒: 0,
        };
        let 成功 = 执行属性修改(
            &mut 世界,
            &注册,
            &mut 状态,
            请求(vec![4.0, 5.0, 6.0], vec![1.0, 2.0, 3.0], "write-1"),
        );
        assert!(成功.成功, "合法白名单写入必须成功：{成功:?}");
        assert_eq!(
            世界.get::<Transform>(实体).unwrap().translation,
            Vec3::new(4.0, 5.0, 6.0)
        );
        assert_eq!(成功.最终值, Some(字段值::向量(vec![4.0, 5.0, 6.0])));

        let 冲突 = 执行属性修改(
            &mut 世界,
            &注册,
            &mut 状态,
            请求(vec![9.0, 9.0, 9.0], vec![0.0, 0.0, 0.0], "write-2"),
        );
        assert!(!冲突.成功 && 冲突.冲突);
        assert_eq!(
            世界.get::<Transform>(实体).unwrap().translation,
            Vec3::new(4.0, 5.0, 6.0),
            "冲突不能改变世界"
        );

        let 非法 = 执行属性修改(
            &mut 世界,
            &注册,
            &mut 状态,
            请求(vec![f64::NAN, 1.0, 1.0], vec![4.0, 5.0, 6.0], "write-3"),
        );
        assert!(!非法.成功);
        assert_eq!(
            非法.错误.as_ref().map(|错误| 错误.代码),
            Some(操作错误代码::值无效)
        );
        assert_eq!(
            世界.get::<Transform>(实体).unwrap().translation,
            Vec3::new(4.0, 5.0, 6.0)
        );

        let 重放 = 执行属性修改(
            &mut 世界,
            &注册,
            &mut 状态,
            请求(vec![4.0, 5.0, 6.0], vec![1.0, 2.0, 3.0], "write-1"),
        );
        assert!(重放.成功 && 重放.重复请求);
    }

    #[test]
    fn 快照采集包含真实实体资源并支持截断标记() {
        let 世界 = 测试世界();
        let 注册 = 调试注册构建器::default().构建();
        let mut 状态 = 调试状态::default();
        let 快照 = 采集世界快照(
            &世界,
            &注册,
            &配置(),
            世界快照请求 {
                最大实体: 1,
                包含资源: true,
            },
            &mut 状态,
        );
        assert_eq!(快照.实体.len(), 1);
        assert!(快照.已截断, "世界有更多实体时必须标记截断");
        assert!(快照.世界实体总数 >= 2);
        assert!(快照.资源.iter().any(|项| 项.名称 == "Time"));
        assert_eq!(快照.快照序号, 1);
    }

    #[test]
    fn 已销毁实体与未注册字段都拒绝写入() {
        let mut 世界 = 测试世界();
        let 注册 = 调试注册构建器::default().构建();
        let mut 状态 = 调试状态::default();
        let 不存在 = 执行属性修改(
            &mut 世界,
            &注册,
            &mut 状态,
            属性修改请求 {
                实体: u64::MAX,
                组件: "Transform".to_string(),
                字段: "translation".to_string(),
                索引: None,
                目标值: 字段值::向量(vec![1.0, 1.0, 1.0]),
                预期原值: Some(字段值::向量(vec![1.0, 1.0, 1.0])),
                请求标识: "missing".to_string(),
                请求时间毫秒: 0,
            },
        );
        assert!(!不存在.成功);
        assert_eq!(
            不存在.错误.as_ref().map(|错误| 错误.代码),
            Some(操作错误代码::实体不存在)
        );

        let 实体 = 世界
            .iter_entities()
            .find(|项| 项.get::<Name>().is_some_and(|名称| 名称.as_str() == "玩家"))
            .unwrap()
            .id();
        let 未注册 = 执行属性修改(
            &mut 世界,
            &注册,
            &mut 状态,
            属性修改请求 {
                实体: 实体.to_bits(),
                组件: "Transform".to_string(),
                字段: "not_registered".to_string(),
                索引: None,
                目标值: 字段值::浮点(1.0),
                预期原值: Some(字段值::浮点(1.0)),
                请求标识: "unknown-field".to_string(),
                请求时间毫秒: 0,
            },
        );
        assert!(!未注册.成功);
        assert_eq!(
            未注册.错误.as_ref().map(|错误| 错误.代码),
            Some(操作错误代码::字段不可写)
        );
    }
}

#[cfg(test)]
mod 资产统计测试 {
    use super::*;

    #[test]
    fn 没有资产资源时统计全为零且不报错() {
        let 世界 = World::new();
        let 统计 = 采集资产统计(&世界);
        assert!(统计.为空());
        assert_eq!(统计.网格数, 0);
        assert_eq!(统计.图片数, 0);
    }

    #[test]
    fn 网格与图片按真实条目统计() {
        let mut 世界 = World::new();
        世界.init_resource::<Assets<Mesh>>();
        世界.init_resource::<Assets<Image>>();
        {
            let mut 网格 = 世界.resource_mut::<Assets<Mesh>>();
            let 一个 = 网格.add(Mesh::from(bevy::prelude::Cuboid::new(1.0, 1.0, 1.0)));
            let _ = 一个;
            let _ = 网格.len();
            assert_eq!(网格.len(), 1, "夹具应当真的插入了网格");
        }
        {
            let mut 图片 = 世界.resource_mut::<Assets<Image>>();
            let 图 = Image::new_fill(
                bevy::render::render_resource::Extent3d {
                    width: 4,
                    height: 2,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                &[255, 0, 0, 255],
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            );
            let _ = 图片.add(图);
        }

        let 统计 = 采集资产统计(&世界);
        assert_eq!(统计.网格数, 1, "统计：{统计:?}");
        assert!(统计.网格顶点数 > 0, "网格顶点应当被计入");
        assert_eq!(统计.图片数, 1);
        assert_eq!(统计.图片像素数, 8, "4×2 像素");
    }

    #[test]
    fn 部分资产类型缺失时其余仍然统计() {
        let mut 世界 = World::new();
        世界.init_resource::<Assets<Mesh>>();
        {
            let mut 网格 = 世界.resource_mut::<Assets<Mesh>>();
            let _ = 网格.add(Mesh::from(bevy::prelude::Cuboid::new(2.0, 2.0, 2.0)));
        }
        let 统计 = 采集资产统计(&世界);
        assert_eq!(统计.网格数, 1);
        assert_eq!(统计.图片数, 0, "没有图片资源就记 0，不是失败");
        assert!(!统计.为空());
    }
}
