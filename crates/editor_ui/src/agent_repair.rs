//! 自动修复闭环的**纯状态机**：错误分类、轮次记录、上限与取消。
//!
//! # 闭环长什么样
//!
//! ```text
//! 用户要求构建/测试/修复
//!   → 真实 cargo build/test（退出码 + 结构化诊断）
//!   → 成功：结束（显示真实退出码）
//!   → 环境问题（工具链缺失/依赖下载失败）：结束，不误判成代码错误
//!   → 编译错误/测试失败：把诊断交给 Agent（第 1..=3 轮）
//!   → Agent 改完文件 → 再真实构建/测试
//!   → 直到成功、达到上限、环境阻塞或用户取消
//! ```
//!
//! # 为什么状态机单独成模块
//!
//! "重试到第几轮""遇到环境问题要不要继续"这类判断必须能脱离进程单测。
//! 真实构建在 `editor_app::build`，真实 Agent 在 `editor_app::agent_dsh`，
//! 这里只回答"下一步该做什么"。

use crate::agent::现在秒;

/// 验证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum 验证方式 {
    /// `cargo build`。
    #[default]
    构建,
    /// `cargo test`。
    测试,
}

impl 验证方式 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            验证方式::构建 => "agent_verify_build",
            验证方式::测试 => "agent_verify_test",
        }
    }

    /// 传给 cargo 的子命令。
    pub fn 子命令(&self) -> &'static str {
        match self {
            验证方式::构建 => "build",
            验证方式::测试 => "test",
        }
    }

    /// 中文名称（写进提示词与日志，不参与界面翻译）。
    pub fn 名称(&self) -> &'static str {
        match self {
            验证方式::构建 => "构建",
            验证方式::测试 => "测试",
        }
    }
}

/// 失败类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum 错误类别 {
    /// 没有错误（真实成功）。
    #[default]
    无错误,
    /// 编译错误（rustc 诊断）。
    编译错误,
    /// 测试失败（断言/panic）。
    测试失败,
    /// 工具链缺失（cargo/rustc/链接器找不到）。
    工具链缺失,
    /// 依赖下载失败（离线/网络/registry）。
    依赖下载失败,
    /// 其他环境问题（文件锁、磁盘、权限）。
    环境其他,
    /// 无法判断。
    未知,
}

impl 错误类别 {
    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            错误类别::无错误 => "agent_error_none",
            错误类别::编译错误 => "agent_error_compile",
            错误类别::测试失败 => "agent_error_test",
            错误类别::工具链缺失 => "agent_error_toolchain",
            错误类别::依赖下载失败 => "agent_error_download",
            错误类别::环境其他 => "agent_error_env",
            错误类别::未知 => "agent_error_unknown",
        }
    }

    /// 是否属于环境问题（不应继续让 Agent 改代码）。
    pub fn 是环境问题(&self) -> bool {
        matches!(
            self,
            错误类别::工具链缺失 | 错误类别::依赖下载失败 | 错误类别::环境其他
        )
    }

    /// 是否值得让 Agent 尝试修复。
    pub fn 可自动修复(&self) -> bool {
        matches!(self, 错误类别::编译错误 | 错误类别::测试失败)
    }
}

/// 从真实诊断文本与退出码判断失败类别。
///
/// 优先级刻意是"环境 → 测试 → 编译 → 未知"：离线时 cargo 也会打印
/// `error:` 字样，先判环境才不会把"下载失败"误报成代码错误。
pub fn 分类错误(诊断: &str, 退出码: Option<i32>, 是测试: bool) -> 错误类别 {
    if 退出码 == Some(0) && 诊断.trim().is_empty() {
        return 错误类别::无错误;
    }
    let 小写 = 诊断.to_lowercase();
    let 含 = |模式: &str| 小写.contains(模式);
    if 含("could not execute process")
        || 含("failed to run custom build command")
        || 含("linker `")
        || 含("linker '")
        || 含("no such file or directory: `cargo`")
        || 含("no such file or directory: `rustc`")
        || 含("cargo: not found")
        || 含("rustc: not found")
        || 含("rustup")
        || 含("toolchain") && 含("not installed")
        || 含("is not installed")
        || 含("program not found")
    {
        return 错误类别::工具链缺失;
    }
    if 含("failed to download")
        || 含("unable to get packages")
        || 含("failed to get")
        || 含("network failure")
        || 含("registry index")
        || 含("could not connect")
        || 含("connection timed out")
        || 含("operation timed out")
        || 含("no matching package")
        || 含("failed to load source")
        || 含("certificate")
        || 含("ssl")
        || 含("offline")
        || 含("blocking waiting for file lock")
    {
        return 错误类别::依赖下载失败;
    }
    if 含("error: could not find `cargo.toml`")
        || 含("could not find `cargo.toml`")
        || 含("没有 cargo.toml")
        || 含("无法启动 cargo")
        || 含("permission denied")
        || 含("os error 5")
        || 含("os error 112")
        || 含("no space left")
    {
        return 错误类别::环境其他;
    }
    if 是测试
        && (含("test result: failed")
            || 含("failures:")
            || 含("panicked at")
            || 含("assertion failed")
            || 含("assertion `left"))
    {
        return 错误类别::测试失败;
    }
    if 含("error[e")
        || 含("error: could not compile")
        || 含("could not compile")
        || 含("aborting due to")
        || 含("error:")
        || 含("mismatched types")
        || 含("cannot find")
        || 含("unresolved import")
        || 含("expected")
    {
        return 错误类别::编译错误;
    }
    if 退出码.is_some_and(|码| 码 != 0) {
        return 错误类别::未知;
    }
    错误类别::无错误
}

/// 一轮修复的记录（真实数据，不做美化）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct 修复轮次 {
    /// 轮次：0 表示首次验证，1.. 表示第 N 轮 Agent 修改后的验证。
    pub 轮数: u32,
    /// 输入给 Agent 的诊断摘要。
    pub 输入诊断: String,
    /// 失败类别。
    pub 错误类别: 错误类别,
    /// 构建/测试退出码。
    pub 退出码: Option<i32>,
    /// 是否成功。
    pub 成功: bool,
    /// 本轮看到的变更文件。
    pub 变更文件: Vec<String>,
    /// 时间。
    pub 时间: u64,
}

impl Default for 修复轮次 {
    fn default() -> Self {
        Self {
            轮数: 0,
            输入诊断: String::new(),
            错误类别: 错误类别::无错误,
            退出码: None,
            成功: false,
            变更文件: Vec::new(),
            时间: 0,
        }
    }
}

/// 闭环阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum 修复阶段 {
    /// 没在修复。
    #[default]
    空闲,
    /// 等待真实构建/测试结束。
    等待验证,
    /// 等待 Agent 修改完成。
    等待Agent,
    /// 已成功（真实退出码为 0）。
    已成功,
    /// 已失败（重试耗尽或不可修复）。
    已失败,
    /// 环境阻塞（工具链/依赖问题，不重试）。
    环境阻塞,
    /// 用户取消。
    已取消,
}

impl 修复阶段 {
    /// 是否仍在推进。
    pub fn 进行中(&self) -> bool {
        matches!(self, 修复阶段::等待验证 | 修复阶段::等待Agent)
    }

    /// 词条键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            修复阶段::空闲 => "agent_repair_idle",
            修复阶段::等待验证 => "agent_repair_verifying",
            修复阶段::等待Agent => "agent_repair_agent",
            修复阶段::已成功 => "agent_repair_ok",
            修复阶段::已失败 => "agent_repair_failed",
            修复阶段::环境阻塞 => "agent_repair_env",
            修复阶段::已取消 => "agent_repair_canceled",
        }
    }
}

/// 用户发起的修复请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 修复请求 {
    /// 用户原始要求（写进提示词）。
    pub 指令: String,
    /// 验证方式。
    pub 验证: 验证方式,
    /// 最大自动修复轮数（首次验证不计入）。
    pub 最大轮数: u32,
}

impl Default for 修复请求 {
    fn default() -> Self {
        Self {
            指令: "构建并修复错误".to_string(),
            验证: 验证方式::构建,
            最大轮数: 3,
        }
    }
}

impl 修复请求 {
    /// 新建请求，把轮数限制在 1..=5。
    pub fn 新建(指令: impl Into<String>, 验证: 验证方式, 最大轮数: u32) -> Self {
        Self {
            指令: 指令.into(),
            验证,
            最大轮数: 最大轮数.clamp(1, 5),
        }
    }
}

/// 构建结束后的下一步。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 下一步 {
    /// 成功，闭环结束。
    完成,
    /// 环境问题，闭环结束（不重试）。
    环境阻塞(错误类别),
    /// 交给 Agent 修复（附带第几轮）。
    启动Agent(u32),
    /// 达到重试上限。
    达到上限,
    /// 已经不在进行中，什么都不做。
    无动作,
}

/// 自动修复闭环状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct 修复循环 {
    /// 阶段。
    pub 阶段: 修复阶段,
    /// 已执行的 Agent 修复轮数。
    pub 已执行轮数: u32,
    /// 上限。
    pub 上限: u32,
    /// 轮次记录（真实构建结果）。
    pub 记录: Vec<修复轮次>,
    /// 验证方式。
    pub 验证: 验证方式,
    /// 用户原始指令。
    pub 指令: String,
    /// 最近一次真实退出码。
    pub 最近退出码: Option<i32>,
    /// 结束原因（真实文本）。
    pub 结束原因: Option<String>,
}

impl Default for 修复循环 {
    fn default() -> Self {
        Self {
            阶段: 修复阶段::空闲,
            已执行轮数: 0,
            上限: 3,
            记录: Vec::new(),
            验证: 验证方式::构建,
            指令: String::new(),
            最近退出码: None,
            结束原因: None,
        }
    }
}

impl 修复循环 {
    /// 开始新一轮闭环。
    pub fn 开始(&mut self, 请求: &修复请求) {
        self.阶段 = 修复阶段::等待验证;
        self.已执行轮数 = 0;
        self.上限 = 请求.最大轮数;
        self.记录.clear();
        self.验证 = 请求.验证;
        self.指令 = 请求.指令.clone();
        self.最近退出码 = None;
        self.结束原因 = None;
    }

    /// 首次验证结束。
    ///
    /// `诊断` 必须是真实 cargo 输出（结构化诊断拼接或 stderr 摘要），
    /// `变更` 是当前真实工作区变更路径。
    pub fn 首次验证结束(
        &mut self,
        诊断: &str,
        退出码: Option<i32>,
        变更: Vec<String>,
    ) -> 下一步 {
        self.记录验证(0, 诊断, 退出码, 变更)
    }

    /// Agent 完成一轮修改后，记录轮数并进入"等待验证"。
    pub fn 智能体完成(&mut self, 摘要: &str) {
        if self.阶段 != 修复阶段::等待Agent {
            return;
        }
        self.已执行轮数 = self.已执行轮数.saturating_add(1);
        self.阶段 = 修复阶段::等待验证;
        // 把 Agent 摘要留到下一次验证记录里，避免丢失"这一轮改了什么"。
        self.结束原因 = Some(摘要.to_string());
    }

    fn 记录验证(
        &mut self,
        轮数: u32,
        诊断: &str,
        退出码: Option<i32>,
        变更: Vec<String>,
    ) -> 下一步 {
        let 成功 = 退出码 == Some(0);
        let 类别 = if 成功 {
            错误类别::无错误
        } else {
            分类错误(诊断, 退出码, self.验证 == 验证方式::测试)
        };
        self.最近退出码 = 退出码;
        self.记录.push(修复轮次 {
            轮数,
            输入诊断: crate::agent_store::截断文本(诊断, 4000),
            错误类别: 类别,
            退出码,
            成功,
            变更文件: 变更,
            时间: 现在秒(),
        });
        if 成功 {
            self.阶段 = 修复阶段::已成功;
            self.结束原因 = Some(format!("{}通过（退出码 0）", self.验证.名称()));
            return 下一步::完成;
        }
        if 类别.是环境问题() {
            self.阶段 = 修复阶段::环境阻塞;
            self.结束原因 = Some(format!("环境问题（{:?}）：没有让 Agent 反复改代码", 类别));
            return 下一步::环境阻塞(类别);
        }
        if !类别.可自动修复() {
            self.阶段 = 修复阶段::已失败;
            self.结束原因 = Some("失败原因无法可靠分类，已停止自动修复".to_string());
            return 下一步::达到上限;
        }
        if self.已执行轮数 >= self.上限 {
            self.阶段 = 修复阶段::已失败;
            self.结束原因 = Some(format!(
                "已达到最大自动修复轮数（{} 轮），最后一次退出码 {:?}",
                self.上限, 退出码
            ));
            return 下一步::达到上限;
        }
        self.阶段 = 修复阶段::等待Agent;
        self.结束原因 = Some(format!(
            "第 {} 轮验证失败（{:?}），把诊断交给 Agent",
            self.已执行轮数 + 1,
            类别
        ));
        下一步::启动Agent(self.已执行轮数 + 1)
    }

    /// Agent 轮次之后的验证结束。
    pub fn 后续验证结束(
        &mut self,
        诊断: &str,
        退出码: Option<i32>,
        变更: Vec<String>,
    ) -> 下一步 {
        self.记录验证(self.已执行轮数, 诊断, 退出码, 变更)
    }

    /// 用户取消。取消不等于回滚：副作用仍需核对。
    pub fn 取消(&mut self, 原因: &str) {
        if self.阶段.进行中() {
            self.阶段 = 修复阶段::已取消;
            self.结束原因 = Some(format!("{原因}（取消不等于回滚：需核对文件差异）"));
        }
    }

    /// 用一句话描述当前状态（真实数据拼接）。
    pub fn 摘要(&self) -> String {
        match self.阶段 {
            修复阶段::空闲 => "未在自动修复".to_string(),
            修复阶段::等待验证 => format!(
                "等待真实{}结果（第 {} 轮，上限 {}）",
                self.验证.名称(),
                self.已执行轮数,
                self.上限
            ),
            修复阶段::等待Agent => format!(
                "等待 Agent 修改（第 {} 轮，上限 {}）",
                self.已执行轮数 + 1,
                self.上限
            ),
            其他 => self.结束原因.clone().unwrap_or_else(|| format!("{其他:?}")),
        }
    }
}

/// 构造交给 Agent 的修复提示。
///
/// 明确写出"不要用粗暴 git 回滚""只改必要文件""不要改无关改动"，并把真实
/// 诊断原文放进去。这不是安全边界（模型仍可能不遵守），但能让默认行为正确。
pub fn 构造修复提示(
    原指令: &str,
    验证: 验证方式,
    诊断: &str,
    轮数: u32,
    上限: u32,
) -> String {
    format!(
        "用户要求：{原指令}\n\n\
         这是第 {轮数}/{上限} 轮自动修复。刚才的 `cargo {子命令}` 真实失败，诊断如下：\n\
         ```\n{诊断}\n```\n\n\
         请定位并修复导致失败的最小代码改动，然后结束。要求：\n\
         1. 先用只读工具确认相关文件与行号；2. 只改必要文件；\n\
         3. 不要使用 git reset --hard、git clean、git restore、git checkout -- 等\
         丢弃工作区改动的命令；4. 不要修改与本诊断无关的既有改动；\n\
         5. 不要声称构建通过——验证由微风在修改后真实执行。",
        子命令 = 验证.子命令()
    )
}

/// 诊断文本的短摘要（界面与提示词用）。
pub fn 诊断摘要(文本: &str, 行数: usize) -> String {
    文本
        .lines()
        .filter(|行| {
            let 行 = 行.trim();
            !行.is_empty()
        })
        .take(行数)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 成功时闭环立即结束() {
        let mut 循环 = 修复循环::default();
        循环.开始(&修复请求::新建("构建", 验证方式::构建, 3));
        assert_eq!(循环.首次验证结束("", Some(0), vec![]), 下一步::完成);
        assert_eq!(循环.阶段, 修复阶段::已成功);
        assert_eq!(循环.记录.len(), 1);
        assert!(循环.记录[0].成功);
    }

    #[test]
    fn 编译错误会启动第一轮修复() {
        let mut 循环 = 修复循环::default();
        循环.开始(&修复请求::新建("构建", 验证方式::构建, 3));
        let 下一步 = 循环.首次验证结束(
            "error[E0308]: mismatched types",
            Some(101),
            vec!["src/main.rs".to_string()],
        );
        assert_eq!(下一步, 下一步::启动Agent(1));
        assert_eq!(循环.阶段, 修复阶段::等待Agent);
        assert_eq!(循环.记录[0].错误类别, 错误类别::编译错误);
    }

    #[test]
    fn 环境问题不启动修复() {
        let mut 循环 = 修复循环::default();
        循环.开始(&修复请求::新建("构建", 验证方式::构建, 3));
        let 下一步 = 循环.首次验证结束(
            "error: failed to download from registry: network failure",
            Some(101),
            vec![],
        );
        assert_eq!(下一步, 下一步::环境阻塞(错误类别::依赖下载失败));
        assert_eq!(循环.阶段, 修复阶段::环境阻塞);
        assert!(循环.记录[0].错误类别.是环境问题());
    }

    #[test]
    fn 达到上限后停止重试() {
        let mut 循环 = 修复循环::default();
        循环.开始(&修复请求::新建("构建", 验证方式::构建, 1));
        assert_eq!(
            循环.首次验证结束("error: cannot find value", Some(101), vec![]),
            下一步::启动Agent(1)
        );
        循环.智能体完成("改了一行");
        assert_eq!(循环.已执行轮数, 1);
        assert_eq!(
            循环.后续验证结束("error: expected `;`", Some(101), vec![]),
            下一步::达到上限
        );
        assert_eq!(循环.阶段, 修复阶段::已失败);
        assert!(循环.结束原因.unwrap().contains("最大自动修复轮数"));
    }

    #[test]
    fn 测试失败被分类为测试失败而不是编译错误() {
        assert_eq!(
            分类错误("test result: FAILED. 1 passed; 1 failed", Some(101), true),
            错误类别::测试失败
        );
        assert_eq!(
            分类错误("assertion `left == right` failed", Some(101), true),
            错误类别::测试失败
        );
    }

    #[test]
    fn 离线下载失败不会被误判成编译错误() {
        let 类别 = 分类错误(
            "error: failed to get `bevy` as a dependency ... network failure",
            Some(101),
            false,
        );
        assert_eq!(类别, 错误类别::依赖下载失败);
        assert!(!类别.可自动修复());
    }

    #[test]
    fn 工具链缺失被单独分类() {
        assert_eq!(
            分类错误("error: could not execute process `rustc`", Some(101), false),
            错误类别::工具链缺失
        );
    }

    #[test]
    fn 取消会保留记录并说明不回滚() {
        let mut 循环 = 修复循环::default();
        循环.开始(&修复请求::新建("构建", 验证方式::构建, 3));
        循环.首次验证结束("error[E0425]", Some(101), vec![]);
        循环.取消("用户点了取消");
        assert_eq!(循环.阶段, 修复阶段::已取消);
        assert!(循环.结束原因.unwrap().contains("取消不等于回滚"));
        assert_eq!(循环.记录.len(), 1, "取消前的结果必须留着核对");
    }

    #[test]
    fn 修复提示包含真实诊断与禁止粗暴回滚() {
        let 提示 = 构造修复提示(
            "修好构建",
            验证方式::构建,
            "error[E0308]: mismatched types",
            2,
            3,
        );
        assert!(提示.contains("error[E0308]"));
        assert!(提示.contains("第 2/3 轮"));
        assert!(提示.contains("git reset --hard"));
        assert!(提示.contains("不要声称构建通过"));
    }
}
