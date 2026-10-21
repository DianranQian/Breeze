//! 微风运行时调试协议（Breeze runtime debug protocol）。
//!
//! # 这个 crate 为什么**不依赖 Bevy**
//!
//! 协议的两端是**两个进程**：微风编辑器与用户自己的原生 Bevy 游戏。
//! 如果协议类型直接用 `Entity` / `Component` / `World`，就会出现三个问题：
//!
//! 1. 两边的 Bevy 版本必须完全一致，否则类型根本不是同一个类型；
//! 2. 协议无法做版本协商 —— 类型布局变了，连"先说清楚版本"都做不到；
//! 3. 协议无法在没有 Bevy 的环境里做编解码与畸形输入测试。
//!
//! 因此这里只定义**独立、可版本化、可序列化**的数据模型。游戏端由
//! `bevy_breeze_debug` 把真实 ECS 数据映射成这些模型；编辑器端把它们
//! 解码成只读快照。任何"共享 Bevy 内部类型"的做法都会破坏这条边界。
//!
//! # 传输
//!
//! 首个实现只支持 **TCP loopback**（编辑器监听 `127.0.0.1:0`，游戏主动连接）。
//! 每帧格式为：
//!
//! ```text
//!   ┌──────────────┬──────────────────────────┐
//!   │ 4 字节大端长度 │ UTF-8 JSON（≤ 上限）       │
//!   └──────────────┴──────────────────────────┘
//! ```
//!
//! 选 JSON 而不是二进制格式，是因为这一阶段的瓶颈是**协议演进与可诊断性**，
//! 不是吞吐量。一个能被人读、能被测试直接构造的载荷，比省几毫秒更重要。
//! 上限、版本与错误码都在本 crate 内强制检查，传输层不自行解释字节。
//!
//! # 安全边界
//!
//! * 协议里**没有**任意文件读写、任意代码执行、任意脚本求值命令；
//! * 请求只覆盖只读的 ECS 查询；
//! * 认证令牌由编辑器生成，游戏端必须在问候消息里带上；
//! * 消息长度、请求编号、分页、字段数量都有上限。

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// 协议主版本。主版本不同 = **不兼容**。
pub const 协议主版本: u16 = 1;

/// 协议次版本。次版本只允许向后兼容的增量。
pub const 协议次版本: u16 = 0;

/// 单帧默认上限（4 MiB）。
///
/// 这个值足以装下几千个实体的分页摘要，又不会让一次畸形长度前缀
/// 直接把编辑器内存吃光。游戏端和编辑器端必须使用同一个上限。
pub const 默认最大消息字节: usize = 4 * 1024 * 1024;

/// 默认心跳间隔（毫秒）。
pub const 默认心跳间隔毫秒: u64 = 2000;

/// 默认请求超时（毫秒）。
pub const 默认请求超时毫秒: u64 = 5000;

/// 单页实体上限。请求方可以要得更少，但不能要得更多。
pub const 最大每页实体数: u32 = 500;

/// 协议版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct 协议版本 {
    /// 主版本。
    pub 主: u16,
    /// 次版本。
    pub 次: u16,
}

impl 协议版本 {
    /// 当前协议版本。
    pub const fn 当前() -> Self {
        Self {
            主: 协议主版本,
            次: 协议次版本,
        }
    }

    /// 检查对端版本是否兼容。
    ///
    /// 规则刻意保守：主版本必须相等；对端次版本不能高于本端。
    /// "对端更新"不等于"本端能读懂它" —— 把未知字段当成可忽略，
    /// 必须在协议设计时逐字段确认，不能靠 serde 默认忽略未知字段来赌。
    pub fn 兼容(&self, 对端: Self) -> Result<(), 兼容错误> {
        if 对端.主 != self.主 {
            return Err(兼容错误::主版本不兼容 {
                本端: *self, 对端
            });
        }
        if 对端.次 > self.次 {
            return Err(兼容错误::对端次版本过新 {
                本端: *self, 对端
            });
        }
        Ok(())
    }

    /// 面向用户的短文本。
    pub fn 文本(&self) -> String {
        format!("{}.{}", self.主, self.次)
    }
}

impl Default for 协议版本 {
    fn default() -> Self {
        Self::当前()
    }
}

/// 版本兼容检查失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum 兼容错误 {
    /// 主版本不同，协议不兼容。
    #[error("协议主版本不兼容：本端 {本端:?}，对端 {对端:?}")]
    主版本不兼容 {
        /// 本端版本。
        本端: 协议版本,
        /// 对端版本。
        对端: 协议版本,
    },
    /// 对端次版本高于本端，本端读不懂新增语义。
    #[error("对端协议次版本过新：本端 {本端:?}，对端 {对端:?}")]
    对端次版本过新 {
        /// 本端版本。
        本端: 协议版本,
        /// 对端版本。
        对端: 协议版本,
    },
}

/// 消息信封：所有消息都带协议版本与会话标识。
///
/// 会话标识在握手前为空字符串；握手后必须逐条匹配，否则视为
/// "旧游戏的消息污染了新游戏"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 消息信封 {
    /// 发送方实现的协议版本。
    pub 协议版本: 协议版本,
    /// 会话标识（握手前为空）。
    pub 会话标识: String,
    /// 消息体。
    pub 消息: 调试消息,
}

/// 一条协议消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body")]
pub enum 调试消息 {
    /// 游戏端 → 编辑器：请求建立调试会话。
    #[serde(rename = "client_hello")]
    客户端问候(客户端问候),
    /// 编辑器 → 游戏端：接受会话并下发参数。
    #[serde(rename = "server_hello")]
    服务端问候(服务端问候),
    /// 编辑器 → 游戏端：只读查询。
    #[serde(rename = "request")]
    请求 {
        /// 请求编号（编辑器内单调递增）。
        编号: u64,
        /// 请求体。
        请求: 调试请求,
    },
    /// 游戏端 → 编辑器：查询结果。
    #[serde(rename = "response")]
    响应 {
        /// 与请求对应的编号。
        编号: u64,
        /// 响应体。
        响应: 调试响应,
    },
    /// 心跳。
    #[serde(rename = "heartbeat")]
    心跳 {
        /// 序号。
        序号: u64,
        /// 发送时间（UNIX 毫秒）。
        发送时间毫秒: u64,
    },
    /// 心跳回应。
    #[serde(rename = "heartbeat_ack")]
    心跳回应 {
        /// 对应心跳序号。
        序号: u64,
    },
    /// 游戏端主动通知（退出、插件卸载、世界重建）。
    #[serde(rename = "notice")]
    通知(调试通知),
    /// 协议级错误。
    #[serde(rename = "protocol_error")]
    错误(协议错误),
}

/// 游戏端的问候消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 客户端问候 {
    /// 共享令牌。必须与编辑器生成的一致。
    pub 令牌: String,
    /// 项目标识（由编辑器注入，用于防止串项目）。
    pub 项目标识: String,
    /// 本次运行实例标识（由编辑器注入）。
    pub 运行实例标识: String,
    /// 游戏应用名。
    pub 应用名称: String,
    /// 游戏使用的 Bevy 版本（适配器声明的版本，不是猜的）。
    pub bevy版本: String,
    /// 适配器名称。
    pub 客户端名称: String,
    /// 客户端支持的能力代号。
    pub 支持能力: Vec<String>,
    /// 游戏启动时间（UNIX 毫秒）。
    pub 启动时间毫秒: u64,
}

/// 编辑器返回的问候消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 服务端问候 {
    /// 服务端名称。
    pub 服务端名称: String,
    /// 本次调试会话标识。
    pub 会话标识: String,
    /// 编辑器支持的能力代号。
    pub 能力: Vec<String>,
    /// 双方必须遵守的最大消息字节数。
    pub 最大消息字节: u32,
    /// 心跳间隔（毫秒）。
    pub 心跳间隔毫秒: u64,
    /// 请求超时（毫秒）。
    pub 请求超时毫秒: u64,
}

/// 能力代号。
pub mod 能力 {
    /// 实体列表。
    pub const 实体列表: &str = "entities";
    /// 实体详情与组件字段。
    pub const 实体详情: &str = "entity_detail";
    /// 组件检查。
    pub const 组件: &str = "components";
    /// 资源检查。
    pub const 资源: &str = "resources";
    /// 分页。
    pub const 分页: &str = "pagination";
    /// 心跳。
    pub const 心跳: &str = "heartbeat";
    /// 世界概览。
    pub const 概览: &str = "overview";
    /// 世界快照（一次取回有界实体/资源集合）。
    pub const 快照: &str = "snapshot";
    /// 白名单属性写入。
    pub const 属性写入: &str = "mutation";
}

/// 编辑器与游戏端之间的**启动注入**环境变量约定。
///
/// 协议本身不规定进程怎么启动；但双方都读同一组常量，才不会出现
/// "编辑器写 `BREEZE_DEBUG_PORT`、适配器读 `BREEZE_PORT`"这种静默失败。
/// 令牌只通过进程环境传递，不写进命令行（命令行会出现在进程列表里）。
pub mod 环境变量 {
    /// 编辑器监听的 TCP 端口（仅 loopback）。
    pub const 端口: &str = "BREEZE_DEBUG_PORT";
    /// 会话令牌。
    pub const 令牌: &str = "BREEZE_DEBUG_TOKEN";
    /// 项目标识。
    pub const 项目标识: &str = "BREEZE_DEBUG_PROJECT";
    /// 运行实例标识。
    pub const 运行实例标识: &str = "BREEZE_DEBUG_RUN";
    /// 应用显示名（可选；缺省时用可执行文件名）。
    pub const 应用名称: &str = "BREEZE_DEBUG_APP";
}

/// 只读查询请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "query", content = "params")]
pub enum 调试请求 {
    /// 世界概览。
    #[serde(rename = "world_overview")]
    世界概览,
    /// 实体列表（分页）。
    #[serde(rename = "entity_list")]
    实体列表(实体列表请求),
    /// 实体详情。
    #[serde(rename = "entity_detail")]
    实体详情 {
        /// 实体会话内标识（`Entity::to_bits`）。
        实体: u64,
    },
    /// 已注册资源列表。
    #[serde(rename = "resource_list")]
    资源列表,
    /// 单个已注册资源详情。
    #[serde(rename = "resource_detail")]
    资源详情 {
        /// 资源名称（适配器注册名）。
        名称: String,
    },
    /// 世界快照：一次取回有界实体/资源集合，用于编辑器侧快照与差异比较。
    #[serde(rename = "world_snapshot")]
    世界快照(世界快照请求),
    /// 白名单属性写入。
    #[serde(rename = "mutate_field")]
    修改属性(属性修改请求),
}

/// 世界快照请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 世界快照请求 {
    /// 最多返回多少实体（会被夹到 `1..=1000`）。
    pub 最大实体: u32,
    /// 是否包含已注册资源详情。
    pub 包含资源: bool,
}

impl Default for 世界快照请求 {
    fn default() -> Self {
        Self {
            最大实体: 500,
            包含资源: true,
        }
    }
}

/// 白名单属性修改请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 属性修改请求 {
    /// 实体标识。
    pub 实体: u64,
    /// 适配器注册的组件名。
    pub 组件: String,
    /// 可写字段名。
    pub 字段: String,
    /// 向量字段的分量下标；`None` 表示整个字段。
    pub 索引: Option<u32>,
    /// 目标值。
    pub 目标值: 字段值,
    /// 客户端看到并期望的原值。`None` 表示不做冲突检查（不推荐）。
    pub 预期原值: Option<字段值>,
    /// 客户端生成的幂等键，用于超时重试。
    pub 请求标识: String,
    /// 请求发出时间（UNIX 毫秒）。
    pub 请求时间毫秒: u64,
}

/// 白名单属性修改结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 属性修改结果 {
    /// 游戏端是否真的执行成功。
    pub 成功: bool,
    /// 是否因预期原值不一致而拒绝。
    pub 冲突: bool,
    /// 修改前值（能确定时）。
    pub 修改前值: Option<字段值>,
    /// 游戏端最终值（成功时）。
    pub 最终值: Option<字段值>,
    /// 失败原因。
    pub 错误: Option<操作错误>,
    /// 实体是否仍存在。
    pub 实体仍存在: bool,
    /// 组件是否仍存在。
    pub 组件仍存在: bool,
    /// 结果序号（单调递增）。
    pub 结果序号: u64,
    /// 游戏端执行完成时间（UNIX 毫秒）。
    pub 执行时间毫秒: u64,
    /// 执行耗时（微秒）。
    pub 执行耗时微秒: u64,
    /// 是否是幂等键命中后的重放结果（此时没有再次写入）。
    pub 重复请求: bool,
}

/// 世界快照数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 世界快照数据 {
    /// 快照中的实体详情。
    pub 实体: Vec<实体详情>,
    /// 已注册资源详情。
    pub 资源: Vec<资源详情>,
    /// 是否因为上限而截断。
    pub 已截断: bool,
    /// 世界实体总数。
    pub 世界实体总数: u64,
    /// 快照序号。
    pub 快照序号: u64,
    /// 采集时间（UNIX 毫秒）。
    pub 采集时间毫秒: u64,
    /// 采集耗时（微秒）。
    pub 采集耗时微秒: u64,
}

/// 实体列表请求。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct 实体列表请求 {
    /// 上一页返回的游标；`None` 表示第一页。
    pub 游标: Option<String>,
    /// 每页数量（会被夹到 `1..=最大每页实体数`）。
    pub 每页: u32,
    /// 文本过滤：匹配实体标识或名称。
    pub 文本: Option<String>,
    /// 组件过滤：只保留拥有该组件的实体。
    pub 组件: Option<String>,
}

/// 查询响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", content = "data")]
pub enum 调试响应 {
    /// 世界概览。
    #[serde(rename = "world_overview")]
    世界概览(世界概览),
    /// 实体列表。
    #[serde(rename = "entity_list")]
    实体列表(实体列表页),
    /// 实体详情。
    #[serde(rename = "entity_detail")]
    实体详情(实体详情),
    /// 资源列表。
    #[serde(rename = "resource_list")]
    资源列表(Vec<资源摘要>),
    /// 资源详情。
    #[serde(rename = "resource_detail")]
    资源详情(资源详情),
    /// 世界快照。
    #[serde(rename = "world_snapshot")]
    世界快照(世界快照数据),
    /// 属性修改结果。
    #[serde(rename = "mutate_field")]
    属性修改(属性修改结果),
    /// 操作失败（实体不存在、资源不存在、采集失败……）。
    #[serde(rename = "failure")]
    失败(操作错误),
}

/// 世界概览。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 世界概览 {
    /// 实体总数。
    pub 实体总数: u64,
    /// 世界里注册的组件类型数。
    pub 组件类型数: u32,
    /// 适配器注册的组件检查器数量。
    pub 已注册组件检查器: u32,
    /// 适配器注册的资源检查器数量。
    pub 已注册资源检查器: u32,
    /// 快照序号（每次采集 +1）。
    pub 快照序号: u64,
    /// 采集时间（UNIX 毫秒）。
    pub 采集时间毫秒: u64,
    /// 采集耗时（微秒）。
    pub 采集耗时微秒: u64,
    /// 游戏上一帧耗时（微秒）。
    ///
    /// 用整数微秒而不是 `f32`：概览要参与 `Eq`，浮点数不能。
    #[serde(default)]
    pub 帧耗时微秒: u64,
    /// 应用元信息。
    pub 应用: 应用元信息,
}

/// 应用元信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 应用元信息 {
    /// 应用名。
    pub 应用名称: String,
    /// Bevy 版本。
    pub bevy版本: String,
    /// 项目标识。
    pub 项目标识: String,
    /// 运行实例标识。
    pub 运行实例标识: String,
    /// 游戏启动时间（UNIX 毫秒）。
    pub 启动时间毫秒: u64,
    /// 当前帧序号。
    pub 帧序号: u64,
}

/// 一页实体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 实体列表页 {
    /// 本页实体。
    pub 实体: Vec<实体摘要>,
    /// 下一页游标；`None` 表示没有更多。
    pub 下一页游标: Option<String>,
    /// 过滤后的总数。
    pub 总数: u64,
    /// 过滤前的世界实体总数。
    pub 世界实体总数: u64,
    /// 是否因为达到扫描上限而截断了候选集。
    pub 已截断: bool,
    /// 快照序号。
    pub 快照序号: u64,
    /// 采集时间（UNIX 毫秒）。
    pub 采集时间毫秒: u64,
    /// 采集耗时（微秒）。
    pub 采集耗时微秒: u64,
}

/// 实体摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 实体摘要 {
    /// 会话内稳定标识（`Entity::to_bits`）。
    pub 标识: u64,
    /// 名称（注册了 `Name` 检查器时）。
    pub 名称: Option<String>,
    /// 组件数。
    pub 组件数: u32,
    /// 有检查器的组件数。
    pub 可检查组件数: u32,
    /// 供列表展示的短摘要（例如 `Transform`、`Sprite`）。
    pub 摘要: Vec<String>,
}

/// 实体详情。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 实体详情 {
    /// 实体标识。
    pub 标识: u64,
    /// 名称。
    pub 名称: Option<String>,
    /// 组件视图（含"不支持检查"的类型）。
    pub 组件: Vec<组件视图>,
    /// 组件总数。
    pub 组件总数: u32,
    /// 快照序号。
    pub 快照序号: u64,
    /// 采集时间（UNIX 毫秒）。
    pub 采集时间毫秒: u64,
    /// 采集耗时（微秒）。
    pub 采集耗时微秒: u64,
}

/// 组件视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 组件视图 {
    /// 组件类型名。
    pub 类型名: String,
    /// 支持级别。
    pub 支持: 检查支持级别,
    /// 结构化字段（不支持时为空）。
    pub 字段: Vec<字段视图>,
    /// 说明（为什么不可检查、采集失败等）。
    pub 说明: Option<String>,
}

/// 组件/资源字段可检查的程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum 检查支持级别 {
    /// 有注册检查器，字段完整。
    可检查,
    /// 有检查器，但只能展示部分字段。
    部分支持,
    /// 没有检查器：只显示类型名。
    不支持,
}

impl 检查支持级别 {
    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            检查支持级别::可检查 => "runtime_debug_field_supported",
            检查支持级别::部分支持 => "runtime_debug_field_partial",
            检查支持级别::不支持 => "runtime_debug_field_unsupported",
        }
    }

    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            检查支持级别::可检查 => "supported",
            检查支持级别::部分支持 => "partial",
            检查支持级别::不支持 => "unsupported",
        }
    }
}

/// 一个字段的值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum 字段值 {
    /// 文本。
    文本(String),
    /// 布尔。
    布尔(bool),
    /// 整数。
    整数(i64),
    /// 浮点。
    浮点(f64),
    /// 向量（2/3/4 维坐标、四元数等）。
    向量(Vec<f64>),
    /// 枚举变体。
    枚举 {
        /// 枚举类型名。
        类型名: String,
        /// 变体名。
        变体: String,
    },
    /// 无法展示，附带原因。
    不可用(String),
}

impl 字段值 {
    /// 人读文本（界面/复制/Agent 上下文共用）。
    pub fn 显示文本(&self) -> String {
        match self {
            字段值::文本(值) => 值.clone(),
            字段值::布尔(值) => {
                if *值 {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            字段值::整数(值) => 值.to_string(),
            字段值::浮点(值) => format!("{值:.4}"),
            字段值::向量(值) => {
                let 部分: Vec<String> = 值.iter().map(|项| format!("{项:.4}")).collect();
                format!("[{}]", 部分.join(", "))
            }
            字段值::枚举 { 类型名, 变体 } => format!("{类型名}::{变体}"),
            字段值::不可用(原因) => format!("不可用：{原因}"),
        }
    }
}

/// 字段视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 字段视图 {
    /// 字段名。
    pub 名称: String,
    /// 字段值。
    pub 值: 字段值,
    /// 游戏端是否允许通过白名单写入该字段。
    #[serde(default)]
    pub 可写: bool,
    /// 数值范围（仅可写数值字段）。
    #[serde(default)]
    pub 范围: Option<数值范围>,
}

/// 可写数值范围。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct 数值范围 {
    /// 最小值（含）。
    pub 最小: f64,
    /// 最大值（含）。
    pub 最大: f64,
}

impl 数值范围 {
    /// 数值是否在范围内且有限。
    pub fn 包含(&self, 值: f64) -> bool {
        值.is_finite() && 值 >= self.最小 && 值 <= self.最大
    }
}

impl 字段视图 {
    /// 新建一个文本字段。
    pub fn 文本(名称: impl Into<String>, 值: impl Into<String>) -> Self {
        Self {
            名称: 名称.into(),
            值: 字段值::文本(值.into()),
            可写: false,
            范围: None,
        }
    }

    /// 新建一个浮点字段。
    pub fn 浮点(名称: impl Into<String>, 值: f64) -> Self {
        Self {
            名称: 名称.into(),
            值: 字段值::浮点(值),
            可写: false,
            范围: None,
        }
    }

    /// 新建一个向量字段。
    pub fn 向量(名称: impl Into<String>, 值: Vec<f64>) -> Self {
        Self {
            名称: 名称.into(),
            值: 字段值::向量(值),
            可写: false,
            范围: None,
        }
    }
}

/// 已注册资源摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 资源摘要 {
    /// 适配器注册名。
    pub 名称: String,
    /// Rust 类型名。
    pub 类型名: String,
    /// 当前世界里是否存在。
    pub 存在: bool,
    /// 是否可检查。
    pub 可检查: bool,
    /// 不可检查的原因。
    pub 说明: Option<String>,
}

/// 资源详情。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 资源详情 {
    /// 注册名。
    pub 名称: String,
    /// 类型名。
    pub 类型名: String,
    /// 字段。
    pub 字段: Vec<字段视图>,
    /// 说明。
    pub 说明: Option<String>,
    /// 快照序号。
    pub 快照序号: u64,
    /// 采集时间（UNIX 毫秒）。
    pub 采集时间毫秒: u64,
}

/// 游戏端主动通知。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "notice", content = "detail")]
pub enum 调试通知 {
    /// 游戏即将退出。
    #[serde(rename = "game_exiting")]
    游戏即将退出 {
        /// 原因。
        原因: String,
    },
    /// 调试插件已被卸载。
    #[serde(rename = "plugin_unloaded")]
    调试插件已卸载,
    /// 世界被重建（场景切换、清空等）。
    #[serde(rename = "world_rebuilt")]
    世界已重建 {
        /// 新快照序号。
        快照序号: u64,
    },
}

/// 协议级错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 协议错误 {
    /// 错误码。
    pub 代码: 错误代码,
    /// 人读说明。
    pub 消息: String,
    /// 是否还能继续使用这条连接。
    pub 可恢复: bool,
}

/// 协议级错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum 错误代码 {
    /// 协议版本不兼容。
    版本不兼容,
    /// 令牌错误或缺失。
    认证失败,
    /// 会话标识不匹配。
    会话不匹配,
    /// 消息超过长度上限。
    消息过大,
    /// 畸形消息。
    畸形消息,
    /// 请求频率超限。
    限流,
    /// 请求/握手超时。
    超时,
    /// 尚未完成握手。
    未握手,
    /// 不支持的能力或请求。
    不支持,
}

impl 错误代码 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            错误代码::版本不兼容 => "version_mismatch",
            错误代码::认证失败 => "auth_failed",
            错误代码::会话不匹配 => "session_mismatch",
            错误代码::消息过大 => "message_too_large",
            错误代码::畸形消息 => "malformed",
            错误代码::限流 => "rate_limited",
            错误代码::超时 => "timeout",
            错误代码::未握手 => "not_handshaken",
            错误代码::不支持 => "unsupported",
        }
    }
}

/// 查询级错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 操作错误 {
    /// 错误码。
    pub 代码: 操作错误代码,
    /// 人读说明。
    pub 消息: String,
}

/// 查询级错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum 操作错误代码 {
    /// 实体已被销毁。
    实体不存在,
    /// 资源不存在或未注册。
    资源不存在,
    /// 当前适配器不支持。
    不支持,
    /// 采集过程中失败。
    采集失败,
    /// 查询被取消。
    查询被取消,
    /// 字段不存在。
    字段不存在,
    /// 字段存在但不可写。
    字段不可写,
    /// 值无效（NaN/Infinity/越界/长度不对）。
    值无效,
    /// 预期原值与游戏端当前值不一致。
    冲突,
    /// 组件不存在。
    组件不存在,
    /// 请求标识重复（幂等重放，不是二次写入）。
    请求重复,
    /// 内部错误。
    内部错误,
}

impl 操作错误代码 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            操作错误代码::实体不存在 => "entity_missing",
            操作错误代码::资源不存在 => "resource_missing",
            操作错误代码::不支持 => "unsupported",
            操作错误代码::采集失败 => "collect_failed",
            操作错误代码::查询被取消 => "query_cancelled",
            操作错误代码::字段不存在 => "field_missing",
            操作错误代码::字段不可写 => "field_read_only",
            操作错误代码::值无效 => "invalid_value",
            操作错误代码::冲突 => "conflict",
            操作错误代码::组件不存在 => "component_missing",
            操作错误代码::请求重复 => "duplicate_request",
            操作错误代码::内部错误 => "internal",
        }
    }
}

/// 编解码错误。
#[derive(Debug, thiserror::Error)]
pub enum 编解码错误 {
    /// 载荷超过上限。
    #[error("消息 {实际} 字节，超过上限 {上限} 字节")]
    消息过大 {
        /// 实际字节数。
        实际: usize,
        /// 上限。
        上限: usize,
    },
    /// JSON 结构不合法。
    #[error("消息不是合法的协议 JSON：{0}")]
    畸形消息(String),
}

/// 帧读取错误。
#[derive(Debug, thiserror::Error)]
pub enum 帧读取错误 {
    /// 对端正常关闭。
    #[error("对端已关闭连接")]
    对端关闭,
    /// 读到一半连接断开。
    #[error("连接在读取一帧的过程中断开")]
    不完整,
    /// 长度前缀超过上限。
    #[error("帧长度 {长度} 超过上限 {上限}")]
    帧过大 {
        /// 声明长度。
        长度: u32,
        /// 上限。
        上限: usize,
    },
    /// 底层 IO 错误（含超时）。
    #[error("读取帧失败：{0}")]
    网络错误(String),
}

/// 编码一条消息为 JSON 载荷，并检查长度上限。
pub fn 编码信封(信封: &消息信封, 上限: usize) -> Result<Vec<u8>, 编解码错误> {
    let 字节 =
        serde_json::to_vec(信封).map_err(|错误| 编解码错误::畸形消息(错误.to_string()))?;
    if 字节.len() > 上限 {
        return Err(编解码错误::消息过大 {
            实际: 字节.len(),
            上限,
        });
    }
    Ok(字节)
}

/// 从 JSON 载荷解码一条消息，并检查长度上限。
pub fn 解码信封(字节: &[u8], 上限: usize) -> Result<消息信封, 编解码错误> {
    if 字节.len() > 上限 {
        return Err(编解码错误::消息过大 {
            实际: 字节.len(),
            上限,
        });
    }
    serde_json::from_slice(字节).map_err(|错误| 编解码错误::畸形消息(错误.to_string()))
}

/// 写入一帧（4 字节大端长度 + 载荷）。
pub fn 写入帧<W: Write>(写: &mut W, 载荷: &[u8]) -> std::io::Result<()> {
    let 长度 = u32::try_from(载荷.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "单帧超过 u32 上限"))?;
    写.write_all(&长度.to_be_bytes())?;
    写.write_all(载荷)?;
    写.flush()
}

/// 读取一帧，长度上限由调用方给出。
///
/// 超时/`WouldBlock` 会作为 [`帧读取错误::网络错误`] 返回：传输线程据此
/// 继续轮询而不是把一次空闲等待当成断线。
pub fn 读取帧<R: Read>(读: &mut R, 上限: usize) -> Result<Vec<u8>, 帧读取错误> {
    let mut 长度字节 = [0u8; 4];
    match 读.read_exact(&mut 长度字节) {
        Ok(()) => {}
        Err(错误) if 错误.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(帧读取错误::对端关闭);
        }
        Err(错误) => return Err(帧读取错误::网络错误(错误.to_string())),
    }
    let 长度 = u32::from_be_bytes(长度字节);
    if 长度 as usize > 上限 {
        return Err(帧读取错误::帧过大 { 长度, 上限 });
    }
    let mut 载荷 = vec![0u8; 长度 as usize];
    match 读.read_exact(&mut 载荷) {
        Ok(()) => Ok(载荷),
        Err(错误) if 错误.kind() == std::io::ErrorKind::UnexpectedEof => {
            Err(帧读取错误::不完整)
        }
        Err(错误) => Err(帧读取错误::网络错误(错误.to_string())),
    }
}

/// 把请求编号变成稳定游标。
pub fn 游标编码(偏移: u64) -> String {
    偏移.to_string()
}

/// 解析游标；非法游标当作第一页，不把内部错误暴露成协议错误。
pub fn 游标解析(游标: Option<&str>) -> u64 {
    游标.and_then(|文本| 文本.parse::<u64>().ok()).unwrap_or(0)
}

/// 夹紧每页数量。
pub fn 夹紧每页(请求: u32, 默认: u32) -> u32 {
    if 请求 == 0 {
        默认.clamp(1, 最大每页实体数)
    } else {
        请求.clamp(1, 最大每页实体数)
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 信封(消息: 调试消息) -> 消息信封 {
        消息信封 {
            协议版本: 协议版本::当前(),
            会话标识: "会话-1".to_string(),
            消息,
        }
    }

    #[test]
    fn 版本协商拒绝不同主版本与更新的次版本() {
        let 本端 = 协议版本 { 主: 1, 次: 2 };
        assert!(本端.兼容(协议版本 { 主: 1, 次: 1 }).is_ok());
        assert!(本端.兼容(协议版本 { 主: 1, 次: 2 }).is_ok());
        assert!(matches!(
            本端.兼容(协议版本 { 主: 2, 次: 0 }),
            Err(兼容错误::主版本不兼容 { .. })
        ));
        assert!(matches!(
            本端.兼容(协议版本 { 主: 1, 次: 3 }),
            Err(兼容错误::对端次版本过新 { .. })
        ));
    }

    #[test]
    fn 每个消息变体都能往返编解码() {
        let 列表 = 实体列表请求 {
            游标: Some("40".to_string()),
            每页: 20,
            文本: Some("玩家".to_string()),
            组件: Some("Transform".to_string()),
        };
        let 消息列表 = vec![
            调试消息::客户端问候(客户端问候 {
                令牌: "令牌".to_string(),
                项目标识: "项目".to_string(),
                运行实例标识: "运行".to_string(),
                应用名称: "示例".to_string(),
                bevy版本: "0.20.0".to_string(),
                客户端名称: "适配器".to_string(),
                支持能力: vec![能力::实体列表.to_string()],
                启动时间毫秒: 10,
            }),
            调试消息::服务端问候(服务端问候 {
                服务端名称: "微风".to_string(),
                会话标识: "会话-1".to_string(),
                能力: vec![能力::实体列表.to_string()],
                最大消息字节: 1024,
                心跳间隔毫秒: 1000,
                请求超时毫秒: 3000,
            }),
            调试消息::请求 {
                编号: 7,
                请求: 调试请求::实体列表(列表.clone()),
            },
            调试消息::响应 {
                编号: 7,
                响应: 调试响应::实体列表(实体列表页 {
                    实体: vec![实体摘要 {
                        标识: 42,
                        名称: Some("玩家".to_string()),
                        组件数: 3,
                        可检查组件数: 1,
                        摘要: vec!["Transform".to_string()],
                    }],
                    下一页游标: None,
                    总数: 1,
                    世界实体总数: 9,
                    已截断: false,
                    快照序号: 4,
                    采集时间毫秒: 99,
                    采集耗时微秒: 12,
                }),
            },
            调试消息::心跳 {
                序号: 2,
                发送时间毫秒: 123,
            },
            调试消息::心跳回应 { 序号: 2 },
            调试消息::通知(调试通知::游戏即将退出 {
                原因: "正常结束".to_string(),
            }),
            调试消息::错误(协议错误 {
                代码: 错误代码::畸形消息,
                消息: "坏帧".to_string(),
                可恢复: false,
            }),
        ];
        for 消息 in 消息列表 {
            let 原 = 信封(消息);
            let 字节 = 编码信封(&原, 默认最大消息字节).expect("编码");
            let 回 = 解码信封(&字节, 默认最大消息字节).expect("解码");
            assert_eq!(原, 回);
        }
    }

    #[test]
    fn 属性修改与快照消息能往返且保留确认字段() {
        let 修改 = 属性修改请求 {
            实体: 42,
            组件: "Transform".to_string(),
            字段: "translation".to_string(),
            索引: None,
            目标值: 字段值::向量(vec![1.0, 2.0, 3.0]),
            预期原值: Some(字段值::向量(vec![0.0, 0.0, 0.0])),
            请求标识: "write-1".to_string(),
            请求时间毫秒: 10,
        };
        let 结果 = 属性修改结果 {
            成功: true,
            冲突: false,
            修改前值: Some(字段值::向量(vec![0.0, 0.0, 0.0])),
            最终值: Some(字段值::向量(vec![1.0, 2.0, 3.0])),
            错误: None,
            实体仍存在: true,
            组件仍存在: true,
            结果序号: 3,
            执行时间毫秒: 20,
            执行耗时微秒: 30,
            重复请求: false,
        };
        let 快照请求 = 世界快照请求 {
            最大实体: 100,
            包含资源: true,
        };
        let 快照数据 = 世界快照数据 {
            实体: Vec::new(),
            资源: Vec::new(),
            已截断: true,
            世界实体总数: 200,
            快照序号: 9,
            采集时间毫秒: 40,
            采集耗时微秒: 50,
        };
        for 消息 in [
            调试消息::请求 {
                编号: 1,
                请求: 调试请求::修改属性(修改.clone()),
            },
            调试消息::响应 {
                编号: 1,
                响应: 调试响应::属性修改(结果.clone()),
            },
            调试消息::请求 {
                编号: 2,
                请求: 调试请求::世界快照(快照请求),
            },
            调试消息::响应 {
                编号: 2,
                响应: 调试响应::世界快照(快照数据),
            },
        ] {
            let 原 = 信封(消息);
            let 字节 = 编码信封(&原, 默认最大消息字节).unwrap();
            assert_eq!(解码信封(&字节, 默认最大消息字节).unwrap(), 原);
        }
    }

    #[test]
    fn 字段可写元数据默认不可写且范围检查拒绝非有限值() {
        let 字段 = 字段视图::浮点("生命值", 1.0);
        assert!(!字段.可写);
        let 范围 = 数值范围 {
            最小: 0.0,
            最大: 10.0,
        };
        assert!(范围.包含(0.0));
        assert!(范围.包含(10.0));
        assert!(!范围.包含(f64::NAN));
        assert!(!范围.包含(f64::INFINITY));
        assert!(!范围.包含(10.1));
    }

    #[test]
    fn 超过上限的消息在编码解码两侧都被拒绝() {
        let 消息 = 信封(调试消息::通知(调试通知::游戏即将退出 {
            原因: "超长".repeat(100),
        }));
        let 上限 = 64;
        assert!(matches!(
            编码信封(&消息, 上限),
            Err(编解码错误::消息过大 { .. })
        ));
        assert!(matches!(
            解码信封(&[0u8; 100], 上限),
            Err(编解码错误::消息过大 { .. })
        ));
    }

    #[test]
    fn 畸形json返回畸形消息而不是panic() {
        assert!(matches!(
            解码信封(b"{not json", 1024),
            Err(编解码错误::畸形消息(_))
        ));
        // 合法 JSON、错误结构。
        assert!(matches!(
            解码信封(r#"{"协议版本":{"主":1,"次":0}}"#.as_bytes(), 1024),
            Err(编解码错误::畸形消息(_))
        ));
    }

    #[test]
    fn 帧编解码保持长度并拒绝超长前缀() {
        let mut 缓冲 = Vec::new();
        写入帧(&mut 缓冲, b"hello").unwrap();
        assert_eq!(&缓冲[..4], &5u32.to_be_bytes());
        let mut 游标 = std::io::Cursor::new(缓冲);
        assert_eq!(读取帧(&mut 游标, 16).unwrap(), b"hello");

        // 声明 100 字节但上限 16：必须在分配前拒绝。
        let mut 恶意 = Vec::new();
        恶意.extend_from_slice(&100u32.to_be_bytes());
        恶意.extend_from_slice(&[0u8; 8]);
        let mut 游标 = std::io::Cursor::new(恶意);
        assert!(matches!(
            读取帧(&mut 游标, 16),
            Err(帧读取错误::帧过大 { 长度: 100, .. })
        ));
    }

    #[test]
    fn 空流返回对端关闭而截断载荷返回不完整() {
        let mut 空: &[u8] = &[];
        assert!(matches!(读取帧(&mut 空, 16), Err(帧读取错误::对端关闭)));

        let mut 短 = Vec::new();
        短.extend_from_slice(&8u32.to_be_bytes());
        短.extend_from_slice(b"abc");
        let mut 游标 = std::io::Cursor::new(短);
        assert!(matches!(读取帧(&mut 游标, 16), Err(帧读取错误::不完整)));
    }

    #[test]
    fn 游标与分页夹紧是确定的() {
        assert_eq!(游标解析(Some("12")), 12);
        assert_eq!(游标解析(Some("不是数字")), 0);
        assert_eq!(游标解析(None), 0);
        assert_eq!(夹紧每页(0, 50), 50);
        assert_eq!(夹紧每页(1, 50), 1);
        assert_eq!(夹紧每页(u32::MAX, 50), 最大每页实体数);
    }

    #[test]
    fn 字段值显示不伪造精度() {
        assert_eq!(字段值::整数(42).显示文本(), "42");
        assert_eq!(字段值::布尔(true).显示文本(), "true");
        assert_eq!(字段值::向量(vec![1.0, 2.5]).显示文本(), "[1.0000, 2.5000]");
        assert!(
            字段值::不可用("不是 Reflect".to_string())
                .显示文本()
                .contains("不是 Reflect")
        );
    }
}
