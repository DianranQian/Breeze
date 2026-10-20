//! AI Agent 模式的编辑器状态模型。
//!
//! # 为什么单独成型一个模块
//!
//! AI Agent 是**一级模式**（与 2D/3D/代码平级），它的状态不是某个面板的私有
//! 字段，而是"项目 + 会话 + 任务 + 工具调用 + 上下文 + 文件变更"这一整棵树。
//! 放在 `界面状态` 里但单独成模块，是为了让"数据模型"与"绘制代码"分开：
//! 面板只读这里的数据，动作层只改这里的数据。
//!
//! # 与 DSH 的边界
//!
//! 这里**不启动进程、不解析协议**。真正跑 DSH 的是 `editor_app::agent_dsh`
//! （进程 + NDJSON 事件），它把事件写进 [`智能体状态::当前任务`]。
//! 因此界面层可以脱离 DSH 单独测试，DSH 也可以脱离界面单独测试。
//!
//! # 任务状态必须来自执行结果
//!
//! [`任务状态`] 只允许由真实进程事件推进：`等待执行 → 执行中 → 成功/失败/已取消`。
//! 绝不根据模型回复的文字去猜"是不是成功了"。
//!
//! # 持久化
//!
//! 会话模型是可序列化的（见 `agent_store`）。这里只定义**允许落盘**的字段：
//! 不放进程句柄、不放环境变量、不放任何凭证。新增字段一律带
//! `#[serde(default)]`，旧存档才能继续读。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent_permission::{审批请求, 权限策略};
use crate::agent_repair::修复循环;

/// 一个 AI 会话。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 智能体会话 {
    /// 会话标识（DSH 的 Session ID；未连接 DSH 时是本地生成的 id）。
    #[serde(default)]
    pub 标识: String,
    /// 标题（用户可重命名；默认取第一条消息的摘要）。
    #[serde(default)]
    pub 标题: String,
    /// 规范化后的所属项目路径。会话**按项目隔离**，切换项目不会混入。
    #[serde(default)]
    pub 项目路径: String,
    /// 项目根目录的原始文本（保留大小写，用于真实文件访问）。
    ///
    /// `项目路径` 是规范化后的隔离键（Windows 上会小写），不能用于拼路径；
    /// 两者分开存，缺一不可。
    #[serde(default)]
    pub 项目根: String,
    /// 对话消息。
    #[serde(default)]
    pub 消息: Vec<智能体消息>,
    /// 历史任务（最新在前）。
    #[serde(default)]
    pub 任务: Vec<智能体任务>,
    /// 附加的上下文项。
    #[serde(default)]
    pub 上下文: Vec<上下文项>,
    /// 文件变更（来自真实 `git status`/`git diff`）。
    #[serde(default)]
    pub 变更: Vec<文件变更>,
    /// 最近一次任务开始前采集的文件基线（用于变更归属）。
    #[serde(default)]
    pub 基线: Option<任务基线>,
    /// 创建时间（UNIX 秒）。
    #[serde(default)]
    pub 创建时间: u64,
    /// 最后更新时间（UNIX 秒，用于排序与显示）。
    #[serde(default)]
    pub 更新时间: u64,
    /// 载入时会话所属项目是否仍然存在。
    ///
    /// 为 `false` 时界面只显示历史，**绝不**自动访问失效路径。
    #[serde(default = "默认真")]
    pub 项目可用: bool,
    /// 项目不可用时的真实原因（目录不存在 / 不是项目）。
    #[serde(default)]
    pub 项目状态说明: Option<String>,
}

/// serde 默认值：历史存档没有这个字段时按"可用"处理。
fn 默认真() -> bool {
    true
}

impl Default for 智能体会话 {
    fn default() -> Self {
        Self {
            标识: String::new(),
            标题: "新会话".to_string(),
            项目路径: String::new(),
            项目根: String::new(),
            消息: Vec::new(),
            任务: Vec::new(),
            上下文: Vec::new(),
            变更: Vec::new(),
            基线: None,
            创建时间: 0,
            更新时间: 0,
            项目可用: true,
            项目状态说明: None,
        }
    }
}

impl 智能体会话 {
    /// 新建一个属于指定项目的会话。
    pub fn 新建(标识: impl Into<String>, 项目路径: impl Into<String>) -> Self {
        let 项目路径 = 项目路径.into();
        let 现在 = 现在秒();
        Self {
            标识: 标识.into(),
            标题: "新会话".to_string(),
            项目根: 项目路径.clone(),
            项目路径,
            消息: Vec::new(),
            任务: Vec::new(),
            上下文: Vec::new(),
            变更: Vec::new(),
            基线: None,
            创建时间: 现在,
            更新时间: 现在,
            项目可用: true,
            项目状态说明: None,
        }
    }

    /// 设置项目根（原始大小写文本）。隔离键由调用方一并给出。
    pub fn 设置项目根(&mut self, 根: &std::path::Path) {
        self.项目根 = 根.to_string_lossy().to_string();
        self.项目路径 = 规范化项目路径(根);
        self.项目可用 = 根.is_dir();
        self.项目状态说明 = if self.项目可用 {
            None
        } else {
            Some(format!("项目目录不存在：{}", 根.display()))
        };
    }

    /// 追加一条消息。
    pub fn 追加消息(&mut self, 角色: 消息角色, 内容: impl Into<String>) {
        self.消息.push(智能体消息 {
            角色,
            内容: 内容.into(),
            时间: 现在秒(),
        });
        self.更新时间 = 现在秒();
        // 标题留空或是默认值时，用第一条用户消息做标题：用户一眼能认出会话。
        if (self.标题.is_empty() || self.标题 == "新会话")
            && 角色 == 消息角色::用户
            && let Some(首) = self.消息.last()
        {
            self.标题 = 首.内容.chars().take(24).collect::<String>();
        }
    }

    /// 本次会话是否产生了真实文件变更。
    pub fn 有变更(&self) -> bool {
        !self.变更.is_empty()
    }

    /// 是否存在需要用户处理的审查项。
    pub fn 有未审查变更(&self) -> bool {
        self.变更
            .iter()
            .any(|项| 项.审查 == 审查状态::待审查 && 项.来源 != 变更来源::任务前已有)
    }

    /// 当前任务（最新一条）。
    pub fn 当前任务(&self) -> Option<&智能体任务> {
        self.任务.first()
    }

    /// 当前任务（可变）。
    pub fn 当前任务可变(&mut self) -> Option<&mut 智能体任务> {
        self.任务.first_mut()
    }
}

/// 一条对话消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 智能体消息 {
    /// 谁说的。
    pub 角色: 消息角色,
    /// 正文。
    pub 内容: String,
    /// 时间（UNIX 秒）。旧存档没有该字段时按 0 处理。
    #[serde(default)]
    pub 时间: u64,
}

/// 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum 消息角色 {
    /// 用户。
    用户,
    /// Agent。
    智能体,
    /// 系统/诊断（连接失败、任务中断等真实事件，不是模型输出）。
    系统,
}

/// 任务状态。只能由真实执行结果推进。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 任务状态 {
    /// 已排队，尚未开始。
    #[default]
    等待执行,
    /// 正在执行。
    执行中,
    /// 成功完成（退出码 0 且收到 final 事件）。
    成功,
    /// 失败（非零退出码、连接错误、超时）。
    失败,
    /// 用户取消。
    已取消,
    /// 状态未知（进程消失但没有明确退出信息）—— 必须如实告知用户。
    状态未知,
}

impl 任务状态 {
    /// 是否已经结束（不再推进）。
    pub fn 已结束(&self) -> bool {
        !matches!(self, 任务状态::等待执行 | 任务状态::执行中)
    }

    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            任务状态::等待执行 => "agent_task_pending",
            任务状态::执行中 => "agent_task_running",
            任务状态::成功 => "agent_task_ok",
            任务状态::失败 => "agent_task_failed",
            任务状态::已取消 => "agent_task_canceled",
            任务状态::状态未知 => "agent_task_unknown",
        }
    }
}

/// 任务种类。决定执行结束后由谁接管（普通对话 / 自动修复 / 单次验证）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 任务种类 {
    /// 普通对话任务。
    #[default]
    对话,
    /// 自动修复闭环中的一轮 Agent 修改。
    自动修复,
    /// 只做真实构建/测试验证。
    构建验证,
}

/// 一次任务执行（对应一次 DSH 进程）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 智能体任务 {
    /// 任务标识。
    #[serde(default)]
    pub 标识: String,
    /// 用户输入的任务原文。
    #[serde(default)]
    pub 指令: String,
    /// 状态。
    #[serde(default)]
    pub 状态: 任务状态,
    /// DSH 报的 Session ID（有则可继续该会话）。
    #[serde(default)]
    pub 会话标识: Option<String>,
    /// 工具调用记录。
    #[serde(default)]
    pub 工具调用: Vec<工具调用>,
    /// 输出摘要 / 错误信息（真实 stderr 或退出原因）。
    #[serde(default)]
    pub 摘要: String,
    /// 开始时间（UNIX 秒）。
    #[serde(default)]
    pub 开始时间: u64,
    /// 结束时间（UNIX 秒）。
    #[serde(default)]
    pub 结束时间: Option<u64>,
    /// 结束时是否已运行过构建或测试（用于"已验证/未验证"标记）。
    #[serde(default)]
    pub 已运行验证: bool,
    /// 任务种类。
    #[serde(default)]
    pub 种类: 任务种类,
    /// 真实退出码（拿到才有）。
    #[serde(default)]
    pub 退出码: Option<i32>,
    /// 自动修复轮数（第几轮；普通任务为 0）。
    #[serde(default)]
    pub 轮数: u32,
    /// 从工具调用里能可靠看到的写操作路径（相对项目根）。
    #[serde(default)]
    pub 修改文件: Vec<String>,
    /// 输入给 Agent 的诊断摘要（自动修复用）。
    #[serde(default)]
    pub 诊断输入: String,
}

impl Default for 智能体任务 {
    fn default() -> Self {
        Self {
            标识: String::new(),
            指令: String::new(),
            状态: 任务状态::等待执行,
            会话标识: None,
            工具调用: Vec::new(),
            摘要: String::new(),
            开始时间: 0,
            结束时间: None,
            已运行验证: false,
            种类: 任务种类::对话,
            退出码: None,
            轮数: 0,
            修改文件: Vec::new(),
            诊断输入: String::new(),
        }
    }
}

impl 智能体任务 {
    /// 记录一次写操作路径（去重）。只接受项目相对路径。
    pub fn 记录写路径(&mut self, 路径: &str) {
        let 路径 = 路径.trim().trim_start_matches("./").replace('\\', "/");
        if 路径.is_empty() || 路径.starts_with("..") || 路径.len() > 400 {
            return;
        }
        // 拒绝 URL 编码的斜杠/空字节：它们可能绕过"相对路径"的直觉判断。
        let 小写 = 路径.to_ascii_lowercase();
        if 小写.contains("%2f") || 小写.contains("%5c") || 小写.contains("%00") {
            return;
        }
        if !self.修改文件.iter().any(|已有| 已有 == &路径) {
            self.修改文件.push(路径);
        }
    }
}

/// 一次工具调用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 工具调用 {
    /// 工具名（如 `read_file` / `shell`）。
    pub 名称: String,
    /// 状态文案（原始事件里的 phase，不做翻译，便于排查）。
    pub 阶段: String,
    /// 结果摘要（截断到合理长度）。
    pub 摘要: String,
    /// 是否涉及文件写入（影响审查区的高亮）。
    pub 写操作: bool,
}

/// 附加到会话的上下文项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 上下文项 {
    /// 种类（场景/实体/代码/诊断/资源/项目）。
    pub 种类: 上下文种类,
    /// 显示名（路径或实体名）。
    pub 名称: String,
    /// 提供给 Agent 的正文（真实数据，不是占位符）。
    pub 正文: String,
}

impl 上下文项 {
    /// 词条键（用于种类标签）。
    pub fn 种类键(&self) -> &'static str {
        self.种类.词条键()
    }
}

/// 上下文种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum 上下文种类 {
    /// 项目结构摘要。
    项目,
    /// 当前场景。
    场景,
    /// 选中实体。
    实体,
    /// 源代码文件。
    代码,
    /// 构建/运行诊断。
    诊断,
    /// 原生 Cargo/Bevy 工程结构（workspace、crate、入口）。
    Cargo工程,
}

impl 上下文种类 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            // 复用既有的"项目/场景/资源/构建"词条，避免为 AI 再造一套同义词。
            上下文种类::项目 => "project",
            上下文种类::场景 => "scene",
            上下文种类::实体 => "entity",
            上下文种类::代码 => "script",
            上下文种类::诊断 => "diagnostics",
            上下文种类::Cargo工程 => "agent_context_cargo",
        }
    }
}

/// 变更的文件类型（来自 git 状态码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 变更种类 {
    /// 新增（未跟踪或已暂存新增）。
    新增,
    /// 修改。
    修改,
    /// 删除。
    删除,
    /// 重命名。
    重命名,
    /// 类型变化（文件 ↔ 符号链接等）。
    类型变更,
    /// 无法判断。
    #[default]
    未知,
}

impl 变更种类 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            变更种类::新增 => "agent_change_added",
            变更种类::修改 => "agent_change_modified",
            变更种类::删除 => "agent_change_deleted",
            变更种类::重命名 => "agent_change_renamed",
            变更种类::类型变更 => "agent_change_type",
            变更种类::未知 => "agent_change_unknown",
        }
    }
}

/// 变更归属。
///
/// 判断依据是**任务开始前的基线 + 工具写操作记录**，不是"整份 git diff"。
/// 无法可靠归属的一律是 [`变更来源::来源未知`]，绝不冒认成 Agent 的成果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 变更来源 {
    /// 任务开始前就已经存在（用户或其它工作线的旧改动）。
    任务前已有,
    /// 本任务期间出现，且有写工具记录指向该文件。
    本任务,
    /// 任务期间出现，但没有写工具记录 —— 可能是用户手动或其他工作线。
    任务期间非本任务,
    /// 无法判断。
    #[default]
    来源未知,
}

impl 变更来源 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            变更来源::任务前已有 => "agent_source_preexisting",
            变更来源::本任务 => "agent_source_task",
            变更来源::任务期间非本任务 => "agent_source_concurrent",
            变更来源::来源未知 => "agent_source_unknown",
        }
    }

    /// 是否可以按"本任务成果"处理（只有它才允许回退）。
    pub fn 可回退(&self) -> bool {
        matches!(self, 变更来源::本任务)
    }
}

/// 审查状态。批准/拒绝都是**真实动作**，不是标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 审查状态 {
    /// 还没审查。
    #[default]
    待审查,
    /// 已批准：保留工作区现状，并记录批准时的指纹。
    已批准,
    /// 已拒绝：已按基线恢复（或已删除新增文件）。
    已拒绝,
    /// 冲突：文件在记录之后又被改动，拒绝操作被拒绝。
    冲突,
    /// 无法撤销：缺少可安全恢复的基线（例如任务前已修改）。
    无法撤销,
}

impl 审查状态 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            审查状态::待审查 => "agent_review_pending",
            审查状态::已批准 => "agent_review_approved",
            审查状态::已拒绝 => "agent_review_rejected",
            审查状态::冲突 => "agent_review_conflict",
            审查状态::无法撤销 => "agent_review_unrevertable",
        }
    }
}

/// 验证状态：只由真实构建/测试退出码写入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum 验证状态 {
    /// 尚未验证。
    #[default]
    未验证,
    /// 修改之后真实构建通过。
    构建通过,
    /// 修改之后真实构建失败。
    构建失败,
    /// 修改之后真实测试通过。
    测试通过,
    /// 修改之后真实测试失败。
    测试失败,
}

impl 验证状态 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            验证状态::未验证 => "agent_change_unverified",
            验证状态::构建通过 => "agent_verify_build_ok",
            验证状态::构建失败 => "agent_verify_build_failed",
            验证状态::测试通过 => "agent_verify_test_ok",
            验证状态::测试失败 => "agent_verify_test_failed",
        }
    }

    /// 是否是一个"通过"的结果。
    pub fn 通过(&self) -> bool {
        matches!(self, 验证状态::构建通过 | 验证状态::测试通过)
    }
}

/// 一个被修改的文件（来自真实 git 状态与差异）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct 文件变更 {
    /// 项目相对路径。
    pub 路径: String,
    /// 重命名时的原路径。
    #[serde(default)]
    pub 原路径: Option<String>,
    /// `git status --porcelain` 里的两字符状态（如 ` M`、`??`、`A `）。
    pub 状态: String,
    /// 变更种类。
    #[serde(default)]
    pub 种类: 变更种类,
    /// 差异文本（真实 `git diff` 或按真实内容生成的 unified diff）。
    pub 差异: String,
    /// 归属。
    #[serde(default)]
    pub 来源: 变更来源,
    /// 关联任务标识。
    #[serde(default)]
    pub 任务标识: Option<String>,
    /// 审查状态。
    #[serde(default)]
    pub 审查: 审查状态,
    /// 验证状态。
    #[serde(default)]
    pub 验证: 验证状态,
    /// 任务基线里该文件的指纹（没有基线时为 `None`）。
    #[serde(default)]
    pub 基线指纹: Option<String>,
    /// 记录时的文件指纹（用于检测之后是否又被改动）。
    #[serde(default)]
    pub 当前指纹: Option<String>,
    /// 是否具备安全回退条件（基线干净 + 有基线指纹）。
    #[serde(default)]
    pub 可回退: bool,
    /// 变更记录时间（UNIX 秒）。
    #[serde(default)]
    pub 记录时间: u64,
    /// 验证时间（UNIX 秒）。
    #[serde(default)]
    pub 验证时间: Option<u64>,
    /// 验证时的文件指纹。之后文件再变，就退回"未验证"。
    #[serde(default)]
    pub 已验证指纹: Option<String>,
    /// 备注（冲突原因、不可回退原因等真实文本）。
    #[serde(default)]
    pub 备注: Option<String>,
}

/// 任务开始前的文件基线。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct 任务基线 {
    /// 对应的任务标识。
    pub 任务标识: String,
    /// 采集时间（UNIX 秒）。
    pub 采集时间: u64,
    /// 采集时的 `HEAD` 提交（不是仓库时为空）。
    pub 头提交: String,
    /// 采集时已经处于修改状态的文件（含未跟踪）。
    pub 文件: BTreeMap<String, 基线文件>,
}

/// 基线里的单个文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 基线文件 {
    /// `git status --porcelain` 的状态码。
    pub 状态: String,
    /// 内容指纹。
    pub 指纹: String,
}

impl 任务基线 {
    /// 该文件在任务开始时是否是干净的（不在基线脏文件集合里）。
    pub fn 任务前干净(&self, 路径: &str) -> bool {
        !self.文件.contains_key(路径)
    }
}

/// DSH 连接状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Dsh连接 {
    /// 还没探测过。
    #[default]
    未探测,
    /// 探测成功：记录版本与路径。
    就绪 {
        /// 版本号文本（来自 `dsh --version`）。
        版本: String,
        /// 实际使用的可执行文件路径。
        路径: String,
    },
    /// 没找到可执行文件。
    未安装 {
        /// 探测失败的具体原因。
        原因: String,
    },
    /// 找到了但调用失败（权限、损坏等）。
    失败 {
        /// 错误文本。
        错误: String,
    },
}

impl Dsh连接 {
    /// 是否可用。
    pub fn 可用(&self) -> bool {
        matches!(self, Dsh连接::就绪 { .. })
    }

    /// 单行状态摘要（真实数据拼接，不做美化）。
    pub fn 摘要(&self) -> String {
        match self {
            Dsh连接::未探测 => "未探测".to_string(),
            Dsh连接::就绪 { 版本, 路径 } => format!("{版本} · {路径}"),
            Dsh连接::未安装 { 原因 } => format!("未安装：{原因}"),
            Dsh连接::失败 { 错误 } => format!("失败：{错误}"),
        }
    }
}

/// AI Agent 模式的总状态。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct 智能体状态 {
    /// 所有会话（按项目隔离，用 `项目路径` 字段区分）。
    pub 会话列表: Vec<智能体会话>,
    /// 当前会话在列表中的下标。
    pub 当前会话: Option<usize>,
    /// 输入框内容。
    pub 输入: String,
    /// DSH 连接状态。
    pub 连接: Dsh连接,
    /// DSH 可执行文件（可为空，表示用 PATH 里的 `dsh`）。
    pub 可执行文件: String,
    /// 正在运行的任务（与当前会话的最后一条任务对应）。
    pub 运行中: bool,
    /// 最近一次操作的结果提示（真实错误或成功信息）。
    pub 提示: Option<String>,
    /// 新建会话时用户输入的名称（重命名对话框复用）。
    pub 重命名草稿: Option<String>,
    /// 待执行的任务指令（界面 → 应用层的一次性请求）。
    ///
    /// 界面只负责"把要做什么排进队列"，真正起进程的是 `editor_app`
    /// （见 `agent_dsh`）。这样 `执行单个动作` 不必持有进程句柄，
    /// 签名保持不变，界面层也能脱离进程单独测试。
    pub 待执行: Option<String>,
    /// 请求取消当前任务。
    pub 待取消: bool,
    /// 请求刷新文件变更。
    pub 待刷新变更: bool,
    /// 请求探测 DSH。
    pub 待探测: bool,
    /// 请求保存会话存档（有任何变更就置位；落盘在应用层）。
    pub 待保存: bool,
    /// 请求自动修复（用户要求构建/测试并在失败后修复）。
    pub 待修复: Option<crate::agent_repair::修复请求>,
    /// 自动修复闭环状态。
    pub 修复: 修复循环,
    /// 操作权限策略。
    pub 权限: 权限策略,
    /// 等待用户批准的请求。
    pub 审批: Vec<审批请求>,
    /// 请求"批准某个文件变更"（路径）。
    pub 待批准变更: Option<String>,
    /// 请求"拒绝某个文件变更"（路径）。
    pub 待拒绝变更: Option<String>,
    /// 最近一次会话载入的真实结果（供界面显示，不是占位）。
    pub 载入提示: Option<String>,
    /// 审查面板里正在查看文件内容的路径（不持久化）。
    pub 查看内容: Option<String>,
}

impl 智能体状态 {
    /// 当前会话。
    pub fn 当前(&self) -> Option<&智能体会话> {
        self.当前会话.and_then(|下标| self.会话列表.get(下标))
    }

    /// 当前会话（可变）。
    pub fn 当前可变(&mut self) -> Option<&mut 智能体会话> {
        match self.当前会话 {
            Some(下标) => self.会话列表.get_mut(下标),
            None => None,
        }
    }

    /// 找到指定项目的会话下标列表（按最近更新时间倒序）。
    pub fn 项目会话(&self, 项目路径: &str) -> Vec<usize> {
        let mut 下标: Vec<usize> = self
            .会话列表
            .iter()
            .enumerate()
            .filter(|(_, 会话)| 会话.项目路径 == 项目路径)
            .map(|(下标, _)| 下标)
            .collect();
        下标.sort_by_key(|下标| std::cmp::Reverse(self.会话列表[*下标].更新时间));
        下标
    }

    /// 为项目新建会话并选中它。
    pub fn 新建会话(&mut self, 项目路径: &str, 标识: impl Into<String>) -> usize {
        let mut 会话 = 智能体会话::新建(标识, 项目路径);
        会话.标题 = 默认标题(self.项目会话(项目路径).len() + 1);
        self.会话列表.push(会话);
        let 下标 = self.会话列表.len() - 1;
        self.当前会话 = Some(下标);
        self.待保存 = true;
        下标
    }

    /// 删除当前会话。
    pub fn 删除当前会话(&mut self) -> bool {
        let Some(下标) = self.当前会话 else {
            return false;
        };
        if 下标 >= self.会话列表.len() {
            self.当前会话 = None;
            return false;
        }
        self.会话列表.remove(下标);
        self.当前会话 = if self.会话列表.is_empty() {
            None
        } else {
            Some(下标.min(self.会话列表.len() - 1))
        };
        self.待保存 = true;
        true
    }

    /// 选择会话。
    pub fn 选择会话(&mut self, 下标: usize) -> bool {
        if 下标 >= self.会话列表.len() {
            return false;
        }
        self.当前会话 = Some(下标);
        true
    }

    /// 当前会话追加一条消息；没有会话时什么都不做并返回 `false`。
    pub fn 追加消息(&mut self, 角色: 消息角色, 内容: impl Into<String>) -> bool {
        match self.当前可变() {
            Some(会话) => {
                会话.追加消息(角色, 内容);
                self.待保存 = true;
                true
            }
            None => false,
        }
    }

    /// 把上下文项附加到当前会话（同种类同名去重，后加的覆盖前面的）。
    pub fn 附加上下文(&mut self, 项: 上下文项) -> bool {
        let Some(会话) = self.当前可变() else {
            return false;
        };
        match 会话
            .上下文
            .iter_mut()
            .find(|已有| 已有.种类 == 项.种类 && 已有.名称 == 项.名称)
        {
            Some(已有) => *已有 = 项,
            None => 会话.上下文.push(项),
        }
        self.待保存 = true;
        true
    }

    /// 移除一个上下文项。
    pub fn 移除上下文(&mut self, 下标: usize) -> bool {
        let Some(会话) = self.当前可变() else {
            return false;
        };
        if 下标 >= 会话.上下文.len() {
            return false;
        }
        会话.上下文.remove(下标);
        self.待保存 = true;
        true
    }

    /// 把一段上下文拼成 prompt 前缀（供任务执行使用）。
    ///
    /// 只拼**用户明确附加**的项：不把整个仓库偷偷塞进对话。
    pub fn 上下文提示(&self) -> String {
        let Some(会话) = self.当前() else {
            return String::new();
        };
        if 会话.上下文.is_empty() {
            return String::new();
        }
        let mut 结果 = String::from("以下是当前编辑器里的上下文（真实数据）：\n");
        for 项 in &会话.上下文 {
            // 上下文可能来自源码/日志，先做一次凭证脱敏，避免把明显的密钥
            // 直接送进模型上下文。这里不改变编辑器里的原文。
            let (正文, _) = crate::agent_store::脱敏文本(&项.正文);
            结果.push_str(&format!("\n## {} · {}\n{}\n", 项.种类键(), 项.名称, 正文));
        }
        结果
    }

    /// 任务已完成：把状态与摘要写回会话，并结束运行标记。
    pub fn 结束任务(&mut self, 状态: 任务状态, 摘要: impl Into<String>) {
        self.运行中 = false;
        let 摘要 = 摘要.into();
        if let Some(会话) = self.当前可变()
            && let Some(任务) = 会话.任务.first_mut()
        {
            任务.状态 = 状态;
            任务.摘要 = 摘要.clone();
            任务.结束时间 = Some(现在秒());
            // 只有真实失败/取消/未知才作为系统消息显示，成功的结果走 Agent 回复。
            if matches!(状态, 任务状态::失败 | 任务状态::已取消 | 任务状态::状态未知)
            {
                会话.追加消息(消息角色::系统, 摘要.clone());
            }
        }
        if !摘要.is_empty() {
            self.提示 = Some(摘要);
        }
        self.待保存 = true;
    }

    /// 载入后修正：把上次退出时仍在运行的任务标成"状态未知"，
    /// 并清空一次性请求标记。
    pub fn 修正中断任务(&mut self) -> usize {
        let mut 数量 = 0;
        for 会话 in &mut self.会话列表 {
            for 任务 in &mut 会话.任务 {
                if !任务.状态.已结束() {
                    任务.状态 = 任务状态::状态未知;
                    任务.结束时间 = Some(现在秒());
                    let 原摘要 = 任务.摘要.trim().to_string();
                    任务.摘要 = if 原摘要.is_empty() {
                        "上次退出时任务仍在运行：已标记为中断（副作用需核对）".to_string()
                    } else {
                        format!("{原摘要}\n上次退出时任务仍在运行：已标记为中断（副作用需核对）")
                    };
                    数量 += 1;
                }
            }
        }
        self.运行中 = false;
        self.待执行 = None;
        self.待取消 = false;
        self.待刷新变更 = false;
        self.待探测 = false;
        self.待修复 = None;
        self.待批准变更 = None;
        self.待拒绝变更 = None;
        self.修复 = 修复循环::default();
        数量
    }
}

/// 默认会话标题："会话 N"。
fn 默认标题(序号: usize) -> String {
    format!("会话 {序号}")
}

/// 当前 UNIX 秒。
pub fn 现在秒() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_secs())
        .unwrap_or(0)
}

/// 项目路径的规范化形式（用于会话隔离比较）。
///
/// 只做"分隔符统一 + 去尾部分隔符 + 小写（Windows 不区分大小写）"，
/// 不做 `canonicalize`：后者会因为目录暂时不存在而失败，导致会话被误判成
/// 另一个项目。
pub fn 规范化项目路径(路径: &std::path::Path) -> String {
    let 文本 = 路径.to_string_lossy().replace('\\', "/");
    let 文本 = 文本.trim_end_matches('/').to_string();
    if cfg!(windows) {
        文本.to_ascii_lowercase()
    } else {
        文本
    }
}

/// 会话持久化用的绝对路径推导（供 `editor_app` 落盘使用）。
pub fn 会话文件路径(配置目录: &std::path::Path) -> PathBuf {
    配置目录.join("agent_sessions.json")
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 状态() -> 智能体状态 {
        let mut 状态 = 智能体状态::default();
        状态.新建会话("C:/proj/a", "s1");
        状态
    }

    #[test]
    fn 新建会话会选中并用第一条用户消息做标题() {
        let mut 状态 = 状态();
        assert_eq!(状态.当前会话, Some(0));
        assert!(状态.追加消息(消息角色::用户, "帮我加一个跳跃动画"));
        let 当前 = 状态.当前().unwrap();
        // 标题保持"会话 N"：自动改名会把用户手改的标题顶掉，因此只在
        // 标题为空时才用首条消息补齐。
        assert_eq!(当前.标题, "会话 1");
        assert_eq!(当前.消息.len(), 1);
        assert_eq!(当前.创建时间, 当前.更新时间);
    }

    #[test]
    fn 会话按项目隔离() {
        let mut 状态 = 状态();
        状态.新建会话("C:/proj/b", "s2");
        let a = 状态.项目会话("C:/proj/a");
        let b = 状态.项目会话("C:/proj/b");
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_ne!(a[0], b[0], "两个项目的会话必须是不同的会话");
    }

    #[test]
    fn 删除会话会选中相邻会话() {
        let mut 状态 = 状态();
        状态.新建会话("C:/proj/a", "s2");
        状态.当前会话 = Some(1);
        assert!(状态.删除当前会话());
        assert_eq!(状态.会话列表.len(), 1);
        assert_eq!(状态.当前会话, Some(0));
        assert!(状态.删除当前会话());
        assert_eq!(状态.当前会话, None);
        assert!(!状态.删除当前会话(), "没有会话时删除应当返回 false");
    }

    #[test]
    fn 上下文去重并影响提示() {
        let mut 状态 = 状态();
        let 项 = |正文: &str| 上下文项 {
            种类: 上下文种类::实体,
            名称: "箱子".to_string(),
            正文: 正文.to_string(),
        };
        assert!(状态.附加上下文(项("第一次")));
        assert!(状态.附加上下文(项("第二次")));
        assert_eq!(状态.当前().unwrap().上下文.len(), 1, "同种类同名应当覆盖");
        状态.附加上下文(上下文项 {
            种类: 上下文种类::场景,
            名称: "main.scene.ron".to_string(),
            正文: "实体数：3".to_string(),
        });
        let 提示 = 状态.上下文提示();
        assert!(提示.contains("第二次"), "提示里应当是最新一次的内容");
        assert!(提示.contains("main.scene.ron"));
        assert!(状态.移除上下文(0));
        assert!(!状态.上下文提示().contains("第二次"));
    }

    #[test]
    fn 上下文提示会脱敏明显凭证() {
        let mut 状态 = 状态();
        状态.附加上下文(上下文项 {
            种类: 上下文种类::代码,
            名称: "config.rs".to_string(),
            正文: "const KEY: &str = \"sk-abcdefghijklmnopqrstuvwx\";".to_string(),
        });
        let 提示 = 状态.上下文提示();
        assert!(
            !提示.contains("sk-abcdefghijklmnopqrstuvwx"),
            "模型上下文里不能出现明文凭证：{提示}"
        );
        assert!(提示.contains("已脱敏"));
    }

    #[test]
    fn 任务结束会写回状态并结束运行标记() {
        let mut 状态 = 状态();
        状态.运行中 = true;
        {
            let 会话 = 状态.当前可变().unwrap();
            会话.任务.push(智能体任务 {
                标识: "t1".to_string(),
                指令: "构建".to_string(),
                状态: 任务状态::执行中,
                ..Default::default()
            });
        }
        状态.结束任务(任务状态::失败, "退出码 101");
        assert!(!状态.运行中);
        let 会话 = 状态.当前().unwrap();
        assert_eq!(会话.任务[0].状态, 任务状态::失败);
        assert_eq!(会话.任务[0].摘要, "退出码 101");
        assert!(会话.任务[0].结束时间.is_some());
        assert_eq!(会话.消息.len(), 1, "失败应当留下一条系统消息");
        assert_eq!(会话.消息[0].角色, 消息角色::系统);
        assert!(状态.待保存, "任务结束必须触发存档");
    }

    #[test]
    fn 任务状态只有结束后才不再推进() {
        assert!(!任务状态::等待执行.已结束());
        assert!(!任务状态::执行中.已结束());
        for 状态 in [
            任务状态::成功,
            任务状态::失败,
            任务状态::已取消,
            任务状态::状态未知,
        ] {
            assert!(状态.已结束(), "{状态:?} 应当视为已结束");
        }
    }

    #[test]
    fn 项目路径规范化统一分隔符与大小写() {
        let a = 规范化项目路径(std::path::Path::new("C:\\proj\\A\\"));
        let b = 规范化项目路径(std::path::Path::new("c:/proj/a"));
        if cfg!(windows) {
            assert_eq!(a, b, "Windows 上大小写与分隔符不应影响会话归属");
        } else {
            assert_eq!(a, "C:/proj/A");
        }
    }

    #[test]
    fn 连接状态摘要反映真实数据() {
        assert_eq!(Dsh连接::未探测.摘要(), "未探测");
        let 就绪 = Dsh连接::就绪 {
            版本: "0.2.0".to_string(),
            路径: "dsh".to_string(),
        };
        assert!(就绪.可用());
        assert!(就绪.摘要().contains("0.2.0"));
        assert!(
            !Dsh连接::未安装 {
                原因: "找不到".to_string()
            }
            .可用()
        );
    }

    #[test]
    fn 载入修正把运行中任务标成状态未知() {
        let mut 状态 = 状态();
        {
            let 会话 = 状态.当前可变().unwrap();
            会话.任务.push(智能体任务 {
                状态: 任务状态::执行中,
                摘要: "退出码未知".to_string(),
                ..Default::default()
            });
        }
        状态.运行中 = true;
        状态.待执行 = Some("旧任务".to_string());
        assert_eq!(状态.修正中断任务(), 1);
        assert!(!状态.运行中);
        assert!(状态.待执行.is_none());
        let 任务 = 状态.当前().unwrap().当前任务().unwrap();
        assert_eq!(任务.状态, 任务状态::状态未知);
        assert!(任务.摘要.contains("中断"), "必须说明是中断而不是成功");
        assert_eq!(状态.修正中断任务(), 0, "第二次修正不应再改状态");
    }

    #[test]
    fn 写路径记录只接受项目相对路径() {
        let mut 任务 = 智能体任务::default();
        任务.记录写路径("./src\\main.rs");
        任务.记录写路径("src/main.rs");
        任务.记录写路径("%2Fetc%2Fpasswd");
        任务.记录写路径("../outside.rs");
        assert_eq!(任务.修改文件, vec!["src/main.rs".to_string()]);
    }
}
