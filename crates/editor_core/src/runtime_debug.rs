//! 运行时调试的编辑器侧只读快照。
//!
//! # 为什么这些类型在 `editor_core`
//!
//! 界面层（`editor_ui`）要画检查器，但不能依赖 `editor_app`（依赖方向是
//! 单向的）。因此"连接状态 + 最近一次协议数据 + 数据是否实时"必须放在
//! 更底层的 `editor_core`，由 `editor_app` 写入、`editor_ui` 只读。
//!
//! # 真实性与过期规则
//!
//! 这个模块的核心不是字段，而是**状态语义**：
//!
//! | 状态 | 能否当成实时数据 | 典型原因 |
//! | --- | --- | --- |
//! | 已连接 | 可以 | 握手完成、心跳正常 |
//! | 已断开 | **不可以** | 游戏退出、连接丢失 |
//! | 不兼容 | **不可以** | 协议版本不匹配 |
//! | 未接入 | **不可以** | 游戏没有调试插件 |
//!
//! 断开后旧数据**保留**（用户还要看"最后一次快照"），但 [`运行时调试快照::是否实时`]
//! 必须变成 `false`。任何把旧快照继续标成实时数据的行为都是产品级错误。

use std::collections::BTreeMap;

use runtime_protocol::{
    世界快照数据, 世界概览, 字段值, 实体列表页, 实体详情, 属性修改结果, 检查支持级别, 调试响应,
    资源摘要, 资源详情,
};

/// 调试连接状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 调试连接状态 {
    /// 没有配置调试（例如当前运行目标不是游戏）。
    #[default]
    未启用,
    /// 已准备监听，等待游戏连接。
    等待连接,
    /// 已接受 TCP，正在握手。
    握手中,
    /// 握手完成、心跳正常。
    已连接,
    /// 曾经连接，现在已断开。
    已断开,
    /// 协议版本不兼容。
    不兼容,
    /// 明确失败（认证、畸形消息、监听失败等）。
    失败,
}

impl 调试连接状态 {
    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            调试连接状态::未启用 => "runtime_debug_state_disabled",
            调试连接状态::等待连接 => "runtime_debug_state_waiting",
            调试连接状态::握手中 => "runtime_debug_state_handshake",
            调试连接状态::已连接 => "runtime_debug_state_connected",
            调试连接状态::已断开 => "runtime_debug_state_disconnected",
            调试连接状态::不兼容 => "runtime_debug_state_incompatible",
            调试连接状态::失败 => "runtime_debug_state_failed",
        }
    }

    /// 稳定代号（证据 JSON）。
    pub fn 代号(self) -> &'static str {
        match self {
            调试连接状态::未启用 => "disabled",
            调试连接状态::等待连接 => "waiting",
            调试连接状态::握手中 => "handshaking",
            调试连接状态::已连接 => "connected",
            调试连接状态::已断开 => "disconnected",
            调试连接状态::不兼容 => "incompatible",
            调试连接状态::失败 => "failed",
        }
    }

    /// 这个状态下是否允许把数据显示为实时。
    pub fn 允许实时(self) -> bool {
        matches!(self, 调试连接状态::已连接)
    }

    /// 是否已经建立过会话（断开后用于区分"从没连上"与"连过又断了"）。
    pub fn 曾经连接(self) -> bool {
        matches!(
            self,
            调试连接状态::已连接 | 调试连接状态::已断开 | 调试连接状态::不兼容
        )
    }
}

/// 数据展示可信度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 运行时数据状态 {
    /// 当前实时数据。
    实时,
    /// 最近一次成功快照，但连接已断开。
    最近快照,
    /// 协议不兼容，数据不可用。
    不兼容,
    /// 游戏没有接入调试插件。
    未接入,
    /// 查询失败或连接失败。
    不可用,
    /// 正在等待/加载。
    加载中,
}

impl 运行时数据状态 {
    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            运行时数据状态::实时 => "runtime_debug_data_live",
            运行时数据状态::最近快照 => "runtime_debug_data_last",
            运行时数据状态::不兼容 => "runtime_debug_data_incompatible",
            运行时数据状态::未接入 => "runtime_debug_data_not_attached",
            运行时数据状态::不可用 => "runtime_debug_data_unavailable",
            运行时数据状态::加载中 => "runtime_debug_data_loading",
        }
    }
}

/// 编辑器侧运行时调试快照。
///
/// 这是一个**纯数据**结构：不持有 socket、线程或游戏端类型。
#[derive(Debug, Clone, Default)]
pub struct 运行时调试快照 {
    /// 连接状态。
    pub 状态: 调试连接状态,
    /// 状态说明（真实错误文本；不是泛泛的"失败"）。
    pub 状态说明: String,
    /// 调试会话标识（由编辑器生成）。
    pub 会话标识: Option<String>,
    /// 运行实例标识（与运行记录关联）。
    pub 运行实例标识: Option<String>,
    /// 项目标识。
    pub 项目标识: Option<String>,
    /// 应用名。
    pub 应用名称: Option<String>,
    /// 游戏端声明的 Bevy 版本。
    pub bevy版本: Option<String>,
    /// 游戏启动时间（UNIX 毫秒）。
    pub 游戏启动时间毫秒: Option<u64>,
    /// 编辑器实际监听地址（只可能是 loopback）。
    pub 监听地址: Option<String>,
    /// 协议版本文本。
    pub 协议版本: Option<String>,
    /// 握手完成时间（UNIX 毫秒）。
    pub 连接时间毫秒: Option<u64>,
    /// 最近一次收到心跳回应的时间。
    pub 最近心跳毫秒: Option<u64>,
    /// 最近一次成功采集时间。
    pub 最近采集时间毫秒: Option<u64>,
    /// 当前世界概览。
    pub 世界概览: Option<世界概览>,
    /// 最近一页实体。
    pub 实体页: Option<实体列表页>,
    /// 选中实体详情。
    pub 实体详情: Option<实体详情>,
    /// 已注册资源列表。
    pub 资源列表: Vec<资源摘要>,
    /// 选中资源详情。
    pub 资源详情: Option<资源详情>,
    /// 最近错误。
    pub 最近错误: Option<String>,
    /// 是否有请求在途。
    pub 查询中: bool,
    /// 快照是否已经过期（断开、重启、协议错误）。
    pub 已过期: bool,
    /// 过期原因。
    pub 过期原因: Option<String>,
    /// 旧会话数据不能被新运行复用。
    pub 代次: u64,
    /// 是否有属性修改正在等待游戏端结果。
    pub 修改中: bool,
    /// 最近一次请求往返耗时（微秒）。
    pub 最近请求耗时微秒: Option<u64>,
    /// 本次运行累计断开次数。
    pub 断开次数: u32,
    /// 本次运行累计重连次数。
    pub 重连次数: u32,
    /// 当前运行时实体与微风场景实体的**用户指定**关联。
    pub 关联: Option<运行时关联>,
}

/// 运行时实体与编辑态场景实体的关联。
///
/// 只有用户显式指定才算可靠；名称/Transform 相似不会自动建立关联。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 运行时关联 {
    /// 微风场景实体标识（字符串形式）。
    pub 场景实体: String,
    /// 关联来源。
    pub 来源: String,
    /// 置信度。
    pub 置信度: String,
    /// 备注。
    pub 说明: String,
    /// 建立关联时的运行实例标识。
    pub 运行实例标识: Option<String>,
    /// 建立关联时的运行时实体标识。
    pub 运行时实体: u64,
}

impl 运行时关联 {
    /// 词条键。
    pub fn 来源词条键(&self) -> &'static str {
        "runtime_debug_link_user"
    }
}

impl 运行时调试快照 {
    /// 是否可以把界面数据标成实时。
    pub fn 是否实时(&self) -> bool {
        self.状态.允许实时() && !self.已过期
    }

    /// 面向界面的数据可信度。
    pub fn 数据状态(&self) -> 运行时数据状态 {
        if self.是否实时() {
            return 运行时数据状态::实时;
        }
        match self.状态 {
            调试连接状态::不兼容 => 运行时数据状态::不兼容,
            调试连接状态::未启用 => 运行时数据状态::未接入,
            调试连接状态::等待连接 | 调试连接状态::握手中 => {
                if self.有历史数据() {
                    运行时数据状态::最近快照
                } else {
                    运行时数据状态::加载中
                }
            }
            调试连接状态::已断开 => {
                if self.有历史数据() {
                    运行时数据状态::最近快照
                } else {
                    运行时数据状态::不可用
                }
            }
            调试连接状态::失败 => 运行时数据状态::不可用,
            调试连接状态::已连接 => 运行时数据状态::加载中,
        }
    }

    /// 是否有任何曾经采集到的数据。
    pub fn 有历史数据(&self) -> bool {
        self.世界概览.is_some() || self.实体页.is_some() || self.实体详情.is_some()
    }

    /// 世界实体数（没有概览时为 `None`，绝不填 0 假装知道）。
    pub fn 实体总数(&self) -> Option<u64> {
        self.世界概览.as_ref().map(|概览| 概览.实体总数)
    }

    /// 数据是否来自当前连接（否则是历史快照）。
    pub fn 数据来源说明(&self) -> &'static str {
        match self.数据状态() {
            运行时数据状态::实时 => "实时（当前游戏进程）",
            运行时数据状态::最近快照 => "最近一次快照（不是实时）",
            运行时数据状态::不兼容 => "协议不兼容（无可用数据）",
            运行时数据状态::未接入 => "游戏未接入调试插件",
            运行时数据状态::不可用 => "数据不可用",
            运行时数据状态::加载中 => "等待游戏端响应",
        }
    }

    /// 会话开始时重置：旧会话数据必须消失，不能冒充新会话。
    pub fn 开始新会话(&mut self, 代次: u64, 运行实例标识: Option<String>) {
        self.代次 = 代次;
        self.状态 = 调试连接状态::等待连接;
        self.状态说明 = String::new();
        self.会话标识 = None;
        self.运行实例标识 = 运行实例标识;
        self.项目标识 = None;
        self.应用名称 = None;
        self.bevy版本 = None;
        self.游戏启动时间毫秒 = None;
        self.监听地址 = None;
        self.协议版本 = None;
        self.连接时间毫秒 = None;
        self.最近心跳毫秒 = None;
        self.最近采集时间毫秒 = None;
        self.世界概览 = None;
        self.实体页 = None;
        self.实体详情 = None;
        self.资源列表.clear();
        self.资源详情 = None;
        self.最近错误 = None;
        self.查询中 = false;
        self.已过期 = true;
        self.过期原因 = Some("等待新的调试会话".to_string());
        self.修改中 = false;
        self.最近请求耗时微秒 = None;
        self.断开次数 = 0;
        self.重连次数 = 0;
        self.关联 = None;
    }

    /// 标记断开并保留历史快照。
    pub fn 标记断开(&mut self, 原因: impl Into<String>) {
        let 原因 = 原因.into();
        self.状态 = 调试连接状态::已断开;
        self.状态说明 = 原因.clone();
        self.最近错误 = Some(原因.clone());
        self.已过期 = true;
        self.过期原因 = Some(原因);
        self.查询中 = false;
    }

    /// 标记协议不兼容。
    pub fn 标记不兼容(&mut self, 原因: impl Into<String>) {
        let 原因 = 原因.into();
        self.状态 = 调试连接状态::不兼容;
        self.状态说明 = 原因.clone();
        self.最近错误 = Some(原因.clone());
        self.已过期 = true;
        self.过期原因 = Some(原因);
        self.查询中 = false;
    }

    /// 标记失败。
    pub fn 标记失败(&mut self, 原因: impl Into<String>) {
        let 原因 = 原因.into();
        self.状态 = 调试连接状态::失败;
        self.状态说明 = 原因.clone();
        self.最近错误 = Some(原因.clone());
        self.已过期 = true;
        self.过期原因 = Some(原因);
        self.查询中 = false;
    }

    /// 应用一条查询响应。
    ///
    /// 返回 `false` 表示响应的请求已过期（例如详情请求发出后用户换了选中项），
    /// 调用方应丢弃它而不是覆盖当前状态。
    pub fn 应用响应(&mut self, 请求: 请求用途, 响应: 调试响应) -> bool {
        self.查询中 = false;
        match 响应 {
            调试响应::世界概览(概览) => {
                self.最近采集时间毫秒 = Some(概览.采集时间毫秒);
                self.世界概览 = Some(概览);
                true
            }
            调试响应::实体列表(页) => {
                if 请求 != 请求用途::实体列表 {
                    return false;
                }
                self.最近采集时间毫秒 = Some(页.采集时间毫秒);
                self.实体页 = Some(页);
                true
            }
            调试响应::实体详情(详情) => {
                if 请求 != 请求用途::实体详情 {
                    return false;
                }
                self.最近采集时间毫秒 = Some(详情.采集时间毫秒);
                self.实体详情 = Some(详情);
                true
            }
            调试响应::资源列表(列表) => {
                self.资源列表 = 列表;
                true
            }
            调试响应::资源详情(详情) => {
                if 请求 != 请求用途::资源详情 {
                    return false;
                }
                self.资源详情 = Some(详情);
                true
            }
            调试响应::失败(错误) => {
                self.最近错误 = Some(错误.消息);
                false
            }
            // 快照与属性修改由管理器单独关联，不进入实体详情快照。
            调试响应::世界快照(_) | 调试响应::属性修改(_) => false,
        }
    }

    /// 选中实体后，旧详情必须立刻清空（避免把上一个实体的属性显示在新选中项下）。
    pub fn 选中实体(&mut self, 标识: u64) {
        if self.实体详情.as_ref().map(|项| 项.标识) != Some(标识) {
            self.实体详情 = None;
        }
        self.查询中 = true;
    }

    /// 选中资源。
    pub fn 选中资源(&mut self, 名称: &str) {
        if self.资源详情.as_ref().map(|项| 项.名称.as_str()) != Some(名称) {
            self.资源详情 = None;
        }
        self.查询中 = true;
    }

    /// 组件支持级别统计（界面摘要用）。
    pub fn 组件统计(&self) -> (usize, usize, usize) {
        let 组件 = self
            .实体详情
            .as_ref()
            .map(|详情| 详情.组件.as_slice())
            .unwrap_or_default();
        let 可检查 = 组件
            .iter()
            .filter(|项| 项.支持 == 检查支持级别::可检查)
            .count();
        let 部分 = 组件
            .iter()
            .filter(|项| 项.支持 == 检查支持级别::部分支持)
            .count();
        let 不支持 = 组件
            .iter()
            .filter(|项| 项.支持 == 检查支持级别::不支持)
            .count();
        (可检查, 部分, 不支持)
    }
}

/// 一次属性修改的完整记录。
#[derive(Debug, Clone, PartialEq)]
pub struct 属性修改记录 {
    /// 编辑器内记录序号。
    pub 序号: u64,
    /// 记录时间（UNIX 毫秒）。
    pub 时间毫秒: u64,
    /// 运行实例标识。
    pub 运行实例标识: Option<String>,
    /// 调试会话标识。
    pub 会话标识: Option<String>,
    /// 实体标识。
    pub 实体: u64,
    /// 组件注册名。
    pub 组件: String,
    /// 字段名。
    pub 字段: String,
    /// 分量下标。
    pub 索引: Option<u32>,
    /// 用户提交的目标值。
    pub 目标值: 字段值,
    /// 游戏端返回结果。
    pub 结果: 属性修改结果,
    /// 结果是否已经确认（超时/断线时为 `false`，表示"未知"而不是失败）。
    pub 已确认: bool,
}

impl 属性修改记录 {
    /// 用于界面/AI 的一句话结果。
    pub fn 结果说明(&self) -> String {
        if !self.已确认 {
            return "结果未知：连接在收到游戏端结果前中断，不能当作成功或失败".to_string();
        }
        if self.结果.成功 {
            let 值 = self
                .结果
                .最终值
                .as_ref()
                .map(字段值::显示文本)
                .unwrap_or_else(|| "（无值）".to_string());
            format!("成功，游戏端最终值：{值}")
        } else if self.结果.冲突 {
            format!(
                "冲突：{}",
                self.结果
                    .错误
                    .as_ref()
                    .map(|错误| 错误.消息.as_str())
                    .unwrap_or("预期原值不一致")
            )
        } else {
            format!(
                "失败：{}",
                self.结果
                    .错误
                    .as_ref()
                    .map(|错误| 错误.消息.as_str())
                    .unwrap_or("游戏端拒绝")
            )
        }
    }
}

/// 一份有界运行时快照。
#[derive(Debug, Clone, PartialEq)]
pub struct 运行时快照记录 {
    /// 编辑器内快照序号（从 1 开始）。
    pub 序号: u64,
    /// 游戏端协议快照序号。
    pub 协议快照序号: u64,
    /// 采集时间（UNIX 毫秒）。
    pub 采集时间毫秒: u64,
    /// 项目标识。
    pub 项目标识: Option<String>,
    /// 运行实例标识。
    pub 运行实例标识: Option<String>,
    /// 调试会话标识。
    pub 会话标识: Option<String>,
    /// 协议版本文本。
    pub 协议版本: Option<String>,
    /// 游戏应用名。
    pub 应用名称: Option<String>,
    /// Bevy 版本。
    pub bevy版本: Option<String>,
    /// 实体详情。
    pub 实体: Vec<实体详情>,
    /// 资源详情。
    pub 资源: Vec<资源详情>,
    /// 世界实体总数。
    pub 世界实体总数: u64,
    /// 是否被截断/采集失败。
    pub 已截断: bool,
    /// 估算 JSON 大小（字节）。
    pub 估算字节: u64,
    /// 数据来源说明。
    pub 数据来源: String,
}

impl 运行时快照记录 {
    /// 从协议快照数据生成记录。
    pub fn 从协议(
        序号: u64,
        快照: 世界快照数据,
        项目标识: Option<String>,
        运行实例标识: Option<String>,
        会话标识: Option<String>,
        协议版本: Option<String>,
        应用名称: Option<String>,
        bevy版本: Option<String>,
    ) -> Self {
        let 估算字节 = serde_json::to_vec(&快照)
            .map(|字节| 字节.len() as u64)
            .unwrap_or(0);
        Self {
            序号,
            协议快照序号: 快照.快照序号,
            采集时间毫秒: 快照.采集时间毫秒,
            项目标识,
            运行实例标识,
            会话标识,
            协议版本,
            应用名称,
            bevy版本,
            实体: 快照.实体,
            资源: 快照.资源,
            世界实体总数: 快照.世界实体总数,
            已截断: 快照.已截断,
            估算字节,
            数据来源: "游戏进程真实 ECS 快照".to_string(),
        }
    }

    /// 快照是否完整。
    pub fn 完整(&self) -> bool {
        !self.已截断
    }
}

/// 字段级差异。
#[derive(Debug, Clone, PartialEq)]
pub struct 字段差异 {
    /// 实体标识。
    pub 实体: u64,
    /// 组件类型名。
    pub 组件: String,
    /// 字段名。
    pub 字段: String,
    /// 旧值。
    pub 旧值: 字段值,
    /// 新值。
    pub 新值: 字段值,
}

/// Resource 字段差异。
#[derive(Debug, Clone, PartialEq)]
pub struct 资源差异 {
    /// 资源名。
    pub 资源: String,
    /// 字段名。
    pub 字段: String,
    /// 旧值。
    pub 旧值: 字段值,
    /// 新值。
    pub 新值: 字段值,
}

/// 两份快照的差异。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct 运行时快照差异 {
    /// 基线快照序号。
    pub 基线序号: u64,
    /// 目标快照序号。
    pub 目标序号: u64,
    /// 是否可比。
    pub 可比: bool,
    /// 不可比原因。
    pub 不可比原因: Option<String>,
    /// 快照是否被截断（差异不完整）。
    pub 已截断: bool,
    /// 新增实体。
    pub 新增实体: Vec<u64>,
    /// 消失实体。
    pub 消失实体: Vec<u64>,
    /// 新增组件（实体, 组件名）。
    pub 新增组件: Vec<(u64, String)>,
    /// 移除组件（实体, 组件名）。
    pub 移除组件: Vec<(u64, String)>,
    /// 字段变化。
    pub 字段变化: Vec<字段差异>,
    /// 资源变化。
    pub 资源变化: Vec<资源差异>,
    /// 无法比较的项与原因。
    pub 无法比较: Vec<String>,
}

impl 运行时快照差异 {
    /// 是否有任何可见变化。
    pub fn 有变化(&self) -> bool {
        !self.新增实体.is_empty()
            || !self.消失实体.is_empty()
            || !self.新增组件.is_empty()
            || !self.移除组件.is_empty()
            || !self.字段变化.is_empty()
            || !self.资源变化.is_empty()
    }

    /// 面向用户/AI 的摘要。
    pub fn 摘要文本(&self) -> String {
        if !self.可比 {
            return format!(
                "不可比较：{}",
                self.不可比原因
                    .as_deref()
                    .unwrap_or("快照不属于同一运行实例")
            );
        }
        format!(
            "实体 +{} / -{}；组件 +{} / -{}；字段变化 {}；资源变化 {}{}",
            self.新增实体.len(),
            self.消失实体.len(),
            self.新增组件.len(),
            self.移除组件.len(),
            self.字段变化.len(),
            self.资源变化.len(),
            if self.已截断 {
                "；数据被截断，差异不完整"
            } else {
                ""
            }
        )
    }
}

/// 比较两份快照。只有同一项目、同一运行实例才允许比较。
pub fn 比较快照(
    基线: &运行时快照记录, 目标: &运行时快照记录
) -> 运行时快照差异 {
    let mut 差异 = 运行时快照差异 {
        基线序号: 基线.序号,
        目标序号: 目标.序号,
        可比: false,
        不可比原因: None,
        已截断: 基线.已截断 || 目标.已截断,
        ..Default::default()
    };
    if 基线.项目标识 != 目标.项目标识
        || 基线.运行实例标识 != 目标.运行实例标识
        || 基线.运行实例标识.is_none()
    {
        差异.不可比原因 =
            Some("快照不属于同一个项目/运行实例；游戏重启后的世界不能直接混比".to_string());
        return 差异;
    }
    差异.可比 = true;

    let 基实体: BTreeMap<u64, &实体详情> = 基线.实体.iter().map(|项| (项.标识, 项)).collect();
    let 目实体: BTreeMap<u64, &实体详情> = 目标.实体.iter().map(|项| (项.标识, 项)).collect();
    for 标识 in 目实体.keys() {
        if !基实体.contains_key(标识) {
            差异.新增实体.push(*标识);
        }
    }
    for 标识 in 基实体.keys() {
        if !目实体.contains_key(标识) {
            差异.消失实体.push(*标识);
        }
    }
    for (标识, 目标详情) in &目实体 {
        let Some(基详情) = 基实体.get(标识) else {
            continue;
        };
        let 基组件: BTreeMap<&str, &runtime_protocol::组件视图> = 基详情
            .组件
            .iter()
            .map(|项| (项.类型名.as_str(), 项))
            .collect();
        let 目组件: BTreeMap<&str, &runtime_protocol::组件视图> = 目标详情
            .组件
            .iter()
            .map(|项| (项.类型名.as_str(), 项))
            .collect();
        for 名称 in 目组件.keys() {
            if !基组件.contains_key(名称) {
                差异.新增组件.push((*标识, (*名称).to_string()));
            }
        }
        for 名称 in 基组件.keys() {
            if !目组件.contains_key(名称) {
                差异.移除组件.push((*标识, (*名称).to_string()));
            }
        }
        for (名称, 目标组件) in &目组件 {
            let Some(基组件) = 基组件.get(名称) else {
                continue;
            };
            if 基组件.支持 != 目标组件.支持 {
                差异.无法比较.push(format!(
                    "实体 {标识} 的组件 {名称} 支持级别变化：{:?} → {:?}",
                    基组件.支持, 目标组件.支持
                ));
                continue;
            }
            if 目标组件.支持 == 检查支持级别::不支持 {
                continue;
            }
            let 基字段: BTreeMap<&str, &runtime_protocol::字段视图> = 基组件
                .字段
                .iter()
                .map(|项| (项.名称.as_str(), 项))
                .collect();
            let 目字段: BTreeMap<&str, &runtime_protocol::字段视图> = 目标组件
                .字段
                .iter()
                .map(|项| (项.名称.as_str(), 项))
                .collect();
            for (字段, 目标字段) in &目字段 {
                match 基字段.get(字段) {
                    Some(基字段) if 基字段.值 != 目标字段.值 => {
                        差异.字段变化.push(字段差异 {
                            实体: *标识,
                            组件: (*名称).to_string(),
                            字段: (*字段).to_string(),
                            旧值: 基字段.值.clone(),
                            新值: 目标字段.值.clone(),
                        });
                    }
                    Some(_) => {}
                    None => 差异.无法比较.push(format!(
                        "实体 {标识} 的组件 {名称} 新增字段 {字段}，无法确定旧值"
                    )),
                }
            }
            for 字段 in 基字段.keys() {
                if !目字段.contains_key(字段) {
                    差异.无法比较.push(format!(
                        "实体 {标识} 的组件 {名称} 缺少字段 {字段}，无法比较"
                    ));
                }
            }
        }
    }

    let 基资源: BTreeMap<&str, &资源详情> =
        基线.资源.iter().map(|项| (项.名称.as_str(), 项)).collect();
    let 目资源: BTreeMap<&str, &资源详情> =
        目标.资源.iter().map(|项| (项.名称.as_str(), 项)).collect();
    for (名称, 目标资源) in &目资源 {
        let Some(基资源) = 基资源.get(名称) else {
            差异.无法比较.push(format!("资源 {名称} 只存在于目标快照"));
            continue;
        };
        let 基字段: BTreeMap<&str, &runtime_protocol::字段视图> = 基资源
            .字段
            .iter()
            .map(|项| (项.名称.as_str(), 项))
            .collect();
        for 目标字段 in &目标资源.字段 {
            if let Some(基字段) = 基字段.get(目标字段.名称.as_str())
                && 基字段.值 != 目标字段.值
            {
                差异.资源变化.push(资源差异 {
                    资源: (*名称).to_string(),
                    字段: 目标字段.名称.clone(),
                    旧值: 基字段.值.clone(),
                    新值: 目标字段.值.clone(),
                });
            }
        }
    }
    for 名称 in 基资源.keys() {
        if !目资源.contains_key(名称) {
            差异.无法比较.push(format!("资源 {名称} 只存在于基线快照"));
        }
    }
    差异
}

/// 有界的运行时快照/修改历史。
#[derive(Debug, Clone)]
pub struct 运行时调试历史 {
    快照: Vec<运行时快照记录>,
    修改: Vec<属性修改记录>,
    /// 最近一次差异。
    pub 最近差异: Option<运行时快照差异>,
    /// 下一快照序号。
    下一快照序号: u64,
    /// 下一修改记录序号。
    下一修改序号: u64,
}

/// 历史快照数量上限。
pub const 最大历史快照: usize = 12;
/// 历史快照总估算字节上限。
pub const 最大历史快照字节: u64 = 8 * 1024 * 1024;
/// 修改记录数量上限。
pub const 最大修改记录: usize = 64;

impl Default for 运行时调试历史 {
    fn default() -> Self {
        Self {
            快照: Vec::new(),
            修改: Vec::new(),
            最近差异: None,
            下一快照序号: 1,
            下一修改序号: 1,
        }
    }
}

impl 运行时调试历史 {
    /// 全部快照（旧→新）。
    pub fn 快照(&self) -> &[运行时快照记录] {
        &self.快照
    }

    /// 全部修改记录（旧→新）。
    pub fn 修改(&self) -> &[属性修改记录] {
        &self.修改
    }

    /// 追加一份快照，自动执行数量/内存上限清理。
    pub fn 追加快照(&mut self, 快照: 世界快照数据) -> u64 {
        let 序号 = self.下一快照序号;
        self.下一快照序号 = self.下一快照序号.wrapping_add(1);
        let 记录 = 运行时快照记录::从协议(序号, 快照, None, None, None, None, None, None);
        self.快照.push(记录);
        self.清理快照();
        序号
    }

    /// 追加快照记录（保留来源）。
    pub fn 追加记录(&mut self, 快照: 运行时快照记录) -> u64 {
        let 序号 = self.下一快照序号;
        self.下一快照序号 = self.下一快照序号.wrapping_add(1);
        let mut 记录 = 快照;
        记录.序号 = 序号;
        self.快照.push(记录);
        self.清理快照();
        序号
    }

    /// 记录一次属性修改。
    pub fn 追加修改(
        &mut self,
        运行实例标识: Option<String>,
        会话标识: Option<String>,
        请求: &runtime_protocol::属性修改请求,
        结果: 属性修改结果,
        已确认: bool,
    ) -> u64 {
        let 序号 = self.下一修改序号;
        self.下一修改序号 = self.下一修改序号.wrapping_add(1);
        self.修改.push(属性修改记录 {
            序号,
            时间毫秒: 现在毫秒(),
            运行实例标识,
            会话标识,
            实体: 请求.实体,
            组件: 请求.组件.clone(),
            字段: 请求.字段.clone(),
            索引: 请求.索引,
            目标值: 请求.目标值.clone(),
            结果,
            已确认,
        });
        while self.修改.len() > 最大修改记录 {
            self.修改.remove(0);
        }
        序号
    }

    /// 取最近两份快照做比较。
    pub fn 比较最近两份(&mut self) -> Option<运行时快照差异> {
        if self.快照.len() < 2 {
            return None;
        }
        let 基线 = &self.快照[self.快照.len() - 2];
        let 目标 = &self.快照[self.快照.len() - 1];
        let 差异 = 比较快照(基线, 目标);
        self.最近差异 = Some(差异.clone());
        Some(差异)
    }

    /// 指定两份快照比较。
    pub fn 比较(
        &mut self, 基线序号: u64, 目标序号: u64
    ) -> Option<运行时快照差异> {
        let 基线 = self.快照.iter().find(|项| 项.序号 == 基线序号)?;
        let 目标 = self.快照.iter().find(|项| 项.序号 == 目标序号)?;
        let 差异 = 比较快照(基线, 目标);
        self.最近差异 = Some(差异.clone());
        Some(差异)
    }

    /// 清空历史；进行中的修改记录不受影响（它们不在这里）。
    pub fn 清空(&mut self) {
        self.快照.clear();
        self.修改.clear();
        self.最近差异 = None;
        self.下一快照序号 = 1;
        self.下一修改序号 = 1;
    }

    fn 清理快照(&mut self) {
        while self.快照.len() > 最大历史快照 {
            self.快照.remove(0);
        }
        let mut 总量: u64 = self.快照.iter().map(|项| 项.估算字节).sum();
        while 总量 > 最大历史快照字节 && self.快照.len() > 1 {
            let 移除 = self.快照.remove(0);
            总量 = 总量.saturating_sub(移除.估算字节);
        }
    }
}

/// 请求用途，用于丢弃乱序/过期响应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 请求用途 {
    /// 世界概览。
    世界概览,
    /// 实体列表。
    实体列表,
    /// 实体详情。
    实体详情,
    /// 资源列表。
    资源列表,
    /// 资源详情。
    资源详情,
    /// 运行时快照。
    快照,
    /// 属性修改。
    属性修改,
}

impl 请求用途 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            请求用途::世界概览 => "overview",
            请求用途::实体列表 => "entity_list",
            请求用途::实体详情 => "entity_detail",
            请求用途::资源列表 => "resource_list",
            请求用途::资源详情 => "resource_detail",
            请求用途::快照 => "snapshot",
            请求用途::属性修改 => "mutation",
        }
    }
}

fn 现在毫秒() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// 把运行时数据整理成一段**带来源与可信度**的文本，供 AI Agent 使用。
///
/// 这里的措辞是产品约束的一部分：不能把"实体 42"说成"源码里的玩家"，
/// 也不能把历史快照说成实时数据。
pub fn 智能体上下文文本(快照: &运行时调试快照) -> String {
    let mut 文本 = String::new();
    文本.push_str("【运行时数据来源】\n");
    文本.push_str(&format!("- 可信度：{}\n", 快照.数据来源说明()));
    文本.push_str(&format!("- 连接状态：{}\n", 快照.状态.代号()));
    if let Some(会话) = &快照.会话标识 {
        文本.push_str(&format!("- 调试会话：{会话}\n"));
    }
    if let Some(运行) = &快照.运行实例标识 {
        文本.push_str(&format!("- 运行实例：{运行}\n"));
    }
    if let Some(应用) = &快照.应用名称 {
        文本.push_str(&format!("- 应用：{应用}\n"));
    }
    if let Some(版本) = &快照.bevy版本 {
        文本.push_str(&format!("- Bevy 版本：{版本}\n"));
    }
    if let Some(时间) = 快照.最近采集时间毫秒 {
        文本.push_str(&format!("- 最近采集（UNIX 毫秒）：{时间}\n"));
    }
    if let Some(概览) = &快照.世界概览 {
        文本.push_str(&format!(
            "- 实体总数：{}；组件类型数：{}\n",
            概览.实体总数, 概览.组件类型数
        ));
    }
    if let Some(详情) = &快照.实体详情 {
        文本.push_str("\n【选中实体】\n");
        文本.push_str(&format!("- 运行时实体标识：{}\n", 详情.标识));
        if let Some(名称) = &详情.名称 {
            文本.push_str(&format!("- 运行时名称：{名称}\n"));
        }
        文本.push_str(&format!("- 组件数：{}\n", 详情.组件总数));
        for 组件 in &详情.组件 {
            文本.push_str(&format!(
                "\n组件 `{}`（{}）\n",
                组件.类型名,
                组件.支持.代号()
            ));
            for 字段 in &组件.字段 {
                文本.push_str(&format!("  - {} = {}\n", 字段.名称, 字段.值.显示文本()));
            }
            if let Some(说明) = &组件.说明 {
                文本.push_str(&format!("  - 说明：{说明}\n"));
            }
        }
    }
    if !快照.资源列表.is_empty() {
        文本.push_str("\n【已注册资源】\n");
        for 资源 in &快照.资源列表 {
            文本.push_str(&format!(
                "- {}（{}）：{}\n",
                资源.名称,
                资源.类型名,
                if 资源.存在 {
                    "存在"
                } else {
                    "当前世界不存在"
                }
            ));
        }
    }
    文本.push_str(
        "\n【纪律】\n\
         - 以上只是运行时 ECS 快照，不是源码语义分析。\n\
         - 不得仅凭实体标识猜测源码文件；源码位置未知时明确写未知。\n\
         - 修改代码必须走现有差异审查与真实构建/测试流程。\n",
    );
    文本
}

/// 带快照历史、差异与修改记录的 AI 上下文。
///
/// 明确区分"当前实时值""历史快照""待提交值"和"未确认结果"，
/// 防止 Agent 把失败/未确认的目标值当成游戏当前值。
pub fn 智能体上下文文本扩展(
    快照: &运行时调试快照, 历史: &运行时调试历史
) -> String {
    let mut 文本 = 智能体上下文文本(快照);
    if let Some(关联) = &快照.关联 {
        文本.push_str("\n【编辑态关联】\n");
        文本.push_str(&format!(
            "- 场景实体：{}；来源：{}；置信度：{}；说明：{}\n",
            关联.场景实体, 关联.来源, 关联.置信度, 关联.说明
        ));
    } else {
        文本.push_str("\n【编辑态关联】\n- 未关联：没有可靠的场景实体映射，禁止猜测。\n");
    }
    if 快照.修改中 {
        文本.push_str("\n【待提交/等待结果】\n- 有属性修改已发出，但尚未收到游戏端明确结果；不能当作已生效。\n");
    }
    文本.push_str("\n【运行时快照历史】\n");
    if 历史.快照().is_empty() {
        文本.push_str("- 尚未采集快照。\n");
    } else {
        for 项 in 历史.快照().iter().rev().take(6) {
            文本.push_str(&format!(
                "- 快照 #{}：{} 个实体，协议序号 {}，{}，来源：{}{}\n",
                项.序号,
                项.实体.len(),
                项.协议快照序号,
                项.采集时间毫秒,
                项.数据来源,
                if 项.已截断 {
                    "（已截断，不完整）"
                } else {
                    ""
                }
            ));
        }
    }
    if let Some(差异) = &历史.最近差异 {
        文本.push_str("\n【最近快照差异】\n");
        文本.push_str(&format!("- {}\n", 差异.摘要文本()));
        for 变化 in 差异.字段变化.iter().take(20) {
            文本.push_str(&format!(
                "- 实体 {} 的 {} . {}：{} → {}\n",
                变化.实体,
                变化.组件,
                变化.字段,
                变化.旧值.显示文本(),
                变化.新值.显示文本()
            ));
        }
        for 项 in 差异.无法比较.iter().take(10) {
            文本.push_str(&format!("- 无法比较：{项}\n"));
        }
    }
    文本.push_str("\n【最近属性修改】\n");
    if 历史.修改().is_empty() {
        文本.push_str("- 还没有属性修改记录。\n");
    } else {
        for 项 in 历史.修改().iter().rev().take(8) {
            文本.push_str(&format!(
                "- #{} 实体 {} 的 {} . {}：{}；目标值 {}；{}\n",
                项.序号,
                项.实体,
                项.组件,
                项.字段,
                项.时间毫秒,
                项.目标值.显示文本(),
                项.结果说明()
            ));
        }
    }
    文本.push_str(
        "\n【运行时编辑纪律】\n\
         - 当前实时值只来自最近一次读取；历史快照不是实时值。\n\
         - 未收到游戏端成功结果的修改，不得描述为已生效。\n\
         - 不得把用户目标值写成游戏当前值。\n\
         - 关联信息只有“用户指定”时才可信；未关联时不得猜源码。\n",
    );
    文本
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use runtime_protocol::{
        世界概览, 字段视图, 实体列表页, 实体摘要, 实体详情, 应用元信息, 检查支持级别, 组件视图,
    };

    fn 快照() -> 运行时调试快照 {
        let mut 快照 = 运行时调试快照::default();
        快照.开始新会话(1, Some("run-1".to_string()));
        快照.状态 = 调试连接状态::已连接;
        快照.已过期 = false;
        快照.世界概览 = Some(世界概览 {
            实体总数: 3,
            组件类型数: 5,
            已注册组件检查器: 2,
            已注册资源检查器: 1,
            快照序号: 1,
            采集时间毫秒: 100,
            采集耗时微秒: 10,
            帧耗时微秒: 16_000,
            资产: runtime_protocol::资产统计 {
                网格数: 1,
                网格顶点数: 24,
                ..Default::default()
            },
            应用: 应用元信息 {
                应用名称: "测试".to_string(),
                bevy版本: "0.20".to_string(),
                项目标识: "p".to_string(),
                运行实例标识: "run-1".to_string(),
                启动时间毫秒: 1,
                帧序号: 2,
            },
        });
        快照
    }

    #[test]
    fn 会话开始时旧数据必须消失() {
        let mut 快照 = 快照();
        快照.实体页 = Some(实体列表页 {
            实体: vec![实体摘要 {
                标识: 1,
                名称: None,
                组件数: 1,
                可检查组件数: 1,
                摘要: vec![],
            }],
            下一页游标: None,
            总数: 1,
            世界实体总数: 1,
            已截断: false,
            快照序号: 1,
            采集时间毫秒: 1,
            采集耗时微秒: 1,
        });
        快照.开始新会话(2, Some("run-2".to_string()));
        assert!(快照.实体页.is_none());
        assert!(快照.世界概览.is_none());
        assert!(快照.已过期);
        assert!(!快照.是否实时());
    }

    #[test]
    fn 断开后保留历史但绝不标成实时() {
        let mut 快照 = 快照();
        assert!(快照.是否实时());
        快照.标记断开("进程退出");
        assert!(!快照.是否实时());
        assert_eq!(快照.数据状态(), 运行时数据状态::最近快照);
        assert!(快照.世界概览.is_some(), "历史快照应保留");
        assert!(快照.数据来源说明().contains("不是实时"));
    }

    #[test]
    fn 不兼容与未接入给出各自状态() {
        let mut 快照 = 运行时调试快照::default();
        assert_eq!(快照.数据状态(), 运行时数据状态::未接入);
        快照.标记不兼容("主版本不同");
        assert_eq!(快照.数据状态(), 运行时数据状态::不兼容);
    }

    #[test]
    fn 过期响应不能覆盖当前选中项() {
        let mut 快照 = 快照();
        let 详情 = 实体详情 {
            标识: 7,
            名称: None,
            组件: vec![组件视图 {
                类型名: "Transform".to_string(),
                支持: 检查支持级别::可检查,
                字段: vec![字段视图::浮点("x", 1.0)],
                说明: None,
            }],
            组件总数: 1,
            快照序号: 1,
            采集时间毫秒: 1,
            采集耗时微秒: 1,
        };
        assert!(!快照.应用响应(请求用途::实体列表, 调试响应::实体详情(详情.clone())));
        assert!(快照.实体详情.is_none());
        assert!(快照.应用响应(请求用途::实体详情, 调试响应::实体详情(详情)));
        assert_eq!(快照.组件统计(), (1, 0, 0));
    }

    #[test]
    fn 智能体上下文明确标注来源与纪律() {
        let 文本 = 智能体上下文文本(&快照());
        assert!(文本.contains("实时"));
        assert!(文本.contains("不是源码语义分析"));
        assert!(文本.contains("不得仅凭实体标识猜测源码文件"));
    }

    fn 快照实体(标识: u64, 组件: Vec<组件视图>) -> 实体详情 {
        实体详情 {
            标识,
            名称: Some(format!("实体{标识}")),
            组件总数: 组件.len() as u32,
            组件,
            快照序号: 1,
            采集时间毫秒: 1,
            采集耗时微秒: 1,
        }
    }

    fn 变换组件(x: f64) -> 组件视图 {
        组件视图 {
            类型名: "bevy_transform::components::transform::Transform".to_string(),
            支持: 检查支持级别::可检查,
            字段: vec![字段视图::向量("translation", vec![x, 0.0, 0.0])],
            说明: None,
        }
    }

    fn 快照记录(
        运行实例: &str, 实体: Vec<实体详情>, 截断: bool
    ) -> 运行时快照记录 {
        运行时快照记录::从协议(
            1,
            世界快照数据 {
                实体,
                资源: Vec::new(),
                已截断: 截断,
                世界实体总数: 2,
                快照序号: 1,
                采集时间毫秒: 10,
                采集耗时微秒: 2,
            },
            Some("项目".to_string()),
            Some(运行实例.to_string()),
            Some("会话".to_string()),
            Some("1.0".to_string()),
            Some("测试".to_string()),
            Some("0.20".to_string()),
        )
    }

    #[test]
    fn 快照差异识别新增消失组件与字段变化() {
        let 基线 = 快照记录(
            "run-1",
            vec![快照实体(1, vec![变换组件(0.0)]), 快照实体(2, Vec::new())],
            false,
        );
        let 目标 = 快照记录(
            "run-1",
            vec![
                快照实体(
                    1,
                    vec![
                        变换组件(2.5),
                        组件视图 {
                            类型名: "测试::护盾".to_string(),
                            支持: 检查支持级别::不支持,
                            字段: Vec::new(),
                            说明: None,
                        },
                    ],
                ),
                快照实体(3, Vec::new()),
            ],
            true,
        );
        let 差异 = 比较快照(&基线, &目标);
        assert!(差异.可比);
        assert!(差异.已截断, "任一快照截断时差异必须标记不完整");
        assert_eq!(差异.新增实体, vec![3]);
        assert_eq!(差异.消失实体, vec![2]);
        assert!(
            差异
                .新增组件
                .iter()
                .any(|(实体, 名称)| { *实体 == 1 && 名称.contains("护盾") })
        );
        assert_eq!(差异.字段变化.len(), 1);
        assert_eq!(差异.字段变化[0].字段, "translation");
        assert!(差异.摘要文本().contains("不完整"));
    }

    #[test]
    fn 不同运行实例的快照拒绝比较() {
        let 甲 = 快照记录("run-1", vec![快照实体(1, Vec::new())], false);
        let 乙 = 快照记录("run-2", vec![快照实体(1, Vec::new())], false);
        let 差异 = 比较快照(&甲, &乙);
        assert!(!差异.可比);
        assert!(差异.不可比原因.as_deref().unwrap().contains("运行实例"));
    }

    #[test]
    fn 历史快照有上限且修改记录不被普通清理影响() {
        let mut 历史 = 运行时调试历史::default();
        for _ in 0..(最大历史快照 + 3) {
            历史.追加记录(快照记录("run-1", Vec::new(), false));
        }
        assert_eq!(历史.快照().len(), 最大历史快照);
        assert_eq!(历史.快照().last().unwrap().序号, (最大历史快照 + 3) as u64);
        assert!(历史.修改().is_empty());
    }

    #[test]
    fn 扩展上下文区分历史快照与未确认修改() {
        let mut 历史 = 运行时调试历史::default();
        历史.追加记录(快照记录("run-1", Vec::new(), true));
        let mut 快照 = 快照();
        快照.修改中 = true;
        let 文本 = 智能体上下文文本扩展(&快照, &历史);
        assert!(文本.contains("历史快照"));
        assert!(文本.contains("不能当作已生效"));
        assert!(文本.contains("已截断"));
    }
}
