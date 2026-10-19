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

use std::path::PathBuf;

/// 一个 AI 会话。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct 智能体会话 {
    /// 会话标识（DSH 的 Session ID；未连接 DSH 时是本地生成的 id）。
    pub 标识: String,
    /// 标题（用户可重命名；默认取第一条消息的摘要）。
    pub 标题: String,
    /// 所属项目的绝对路径。会话**按项目隔离**，切换项目不会混入。
    pub 项目路径: String,
    /// 对话消息。
    pub 消息: Vec<智能体消息>,
    /// 历史任务（最新在前）。
    pub 任务: Vec<智能体任务>,
    /// 附加的上下文项。
    pub 上下文: Vec<上下文项>,
    /// 文件变更（来自真实 `git status`/`git diff`）。
    pub 变更: Vec<文件变更>,
    /// 最后更新时间（UNIX 秒，用于排序与显示）。
    pub 更新时间: u64,
}

impl 智能体会话 {
    /// 新建一个属于指定项目的会话。
    pub fn 新建(标识: impl Into<String>, 项目路径: impl Into<String>) -> Self {
        Self {
            标识: 标识.into(),
            标题: "新会话".to_string(),
            项目路径: 项目路径.into(),
            消息: Vec::new(),
            任务: Vec::new(),
            上下文: Vec::new(),
            变更: Vec::new(),
            更新时间: 现在秒(),
        }
    }

    /// 追加一条消息。
    pub fn 追加消息(&mut self, 角色: 消息角色, 内容: impl Into<String>) {
        self.消息.push(智能体消息 {
            角色,
            内容: 内容.into(),
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
}

/// 一条对话消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 智能体消息 {
    /// 谁说的。
    pub 角色: 消息角色,
    /// 正文。
    pub 内容: String,
}

/// 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 消息角色 {
    /// 用户。
    用户,
    /// Agent。
    智能体,
    /// 系统/诊断（连接失败、任务中断等真实事件，不是模型输出）。
    系统,
}

/// 任务状态。只能由真实执行结果推进。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

/// 一次任务执行（对应一次 DSH 进程）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct 智能体任务 {
    /// 任务标识。
    pub 标识: String,
    /// 用户输入的任务原文。
    pub 指令: String,
    /// 状态。
    pub 状态: 任务状态,
    /// DSH 报的 Session ID（有则可继续该会话）。
    pub 会话标识: Option<String>,
    /// 工具调用记录。
    pub 工具调用: Vec<工具调用>,
    /// 输出摘要 / 错误信息（真实 stderr 或退出原因）。
    pub 摘要: String,
    /// 开始时间（UNIX 秒）。
    pub 开始时间: u64,
    /// 结束时是否已运行过构建或测试（用于"已验证/未验证"标记）。
    pub 已运行验证: bool,
}

/// 一次工具调用。
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        }
    }
}

/// 一个被修改的文件（来自真实 git 状态与差异）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 文件变更 {
    /// 项目相对路径。
    pub 路径: String,
    /// `git status --porcelain` 里的两字符状态（如 ` M`、`??`、`A `）。
    pub 状态: String,
    /// 差异文本（`git diff`；未跟踪文件用文件头部摘要代替）。
    pub 差异: String,
    /// 是否在改动之后运行过构建/测试（真实执行记录，不是猜测）。
    pub 已验证: bool,
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
        true
    }

    /// 当前会话追加一条消息；没有会话时什么都不做并返回 `false`。
    pub fn 追加消息(&mut self, 角色: 消息角色, 内容: impl Into<String>) -> bool {
        match self.当前可变() {
            Some(会话) => {
                会话.追加消息(角色, 内容);
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
            结果.push_str(&format!(
                "\n## {} · {}\n{}\n",
                项.种类键(),
                项.名称,
                项.正文
            ));
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
            // 只有真实失败/取消/未知才作为系统消息显示，成功的结果走 Agent 回复。
            if matches!(状态, 任务状态::失败 | 任务状态::已取消 | 任务状态::状态未知)
            {
                会话.追加消息(消息角色::系统, 摘要.clone());
            }
        }
        if !摘要.is_empty() {
            self.提示 = Some(摘要);
        }
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
        assert_eq!(会话.消息.len(), 1, "失败应当留下一条系统消息");
        assert_eq!(会话.消息[0].角色, 消息角色::系统);
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
}
