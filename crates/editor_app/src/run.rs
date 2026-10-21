//! 项目运行管理：启动/停止/重跑当前项目，并把输出接到控制台。
//!
//! # 三种完全不同的"运行"（这是本模块最重要的一段说明）
//!
//! | 名称 | 在哪里跑 | 是什么 | 本模块管不管 |
//! | --- | --- | --- | --- |
//! | **编辑器内预览** | 编辑器进程自己的 `World` | 推进当前场景、出运行时统计 | 不管（见 `actions::推进运行时`） |
//! | **独立预览进程** | 单独的微风子进程 | 再次启动编辑器并打开同一项目 | 管 |
//! | **真实游戏运行时** | 项目自己声明的可执行程序 | 项目编译出的游戏 | 管 |
//!
//! 后两者由 [`运行目标类型`] 区分，目标来自 `project.ron` 的可选 `运行` 段
//! （[`editor_core::project::运行目标清单`]）：
//!
//! * **没有声明** → 默认是「独立预览进程」，**不是游戏运行时**；
//! * **声明了** → 按声明启动真实程序；程序不存在时报明确错误，**不退回**预览进程
//!   （否则用户会以为在跑游戏）。
//!
//! # 运行状态不是 `is_running` 布尔量
//!
//! 一次运行的真实情况由四样东西共同描述：**状态、真实退出码、结束原因、
//! 起止时间**。只留一个布尔量会立刻产生三个具体错误：
//!
//! | 错误 | 后果 |
//! | --- | --- |
//! | 构建成功当成运行成功 | 编译过了但游戏启动崩溃，界面显示"成功" |
//! | 进程被杀当成正常退出 | 用户点停止后看到"退出码 0"，以为程序自己跑完了 |
//! | 进程还在跑却报成功 | 界面说已结束，实际还有一个窗口在跑 |
//!
//! 因此这里保留每次运行的 [`运行记录`]（见 [`editor_core::run_entry`]），
//! 并且明确：**被杀掉的进程没有退出码**，绝不补一个 0 或 1。
//!
//! # 线程模型
//!
//! 子进程通过 `std::process` + 后台读线程实现，不阻塞 Bevy 主线程；
//! 读线程只在管道关闭时结束，避免子进程因管道写满而挂起。
//! 日志有上限（[`日志行上限`]），不会无限积累。

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::prelude::*;
use editor_core::run_entry::{结束原因, 运行入口, 运行历史};

use crate::plugin::EditorStateHolder;

/// 运行日志保留上限（行）。
///
/// 2000 行 × 平均 100 字节 ≈ 200 KB。一次长时间的运行可能刷出几万行，
/// 无上限积累会让内存随运行时长线性增长，而用户真正会去翻的只有尾部。
pub const 日志行上限: usize = 2000;

/// 一次运行请求解析出的命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 运行命令 {
    /// 可执行文件。
    pub 程序: PathBuf,
    /// 参数。
    pub 参数: Vec<String>,
    /// 工作目录。
    pub 工作目录: PathBuf,
    /// 注入的环境变量。
    pub 环境: Vec<(String, String)>,
    /// 这次运行的**目标类型**。界面上必须把它写清楚，不能含糊。
    pub 目标类型: 运行目标类型,
    /// 这次运行对应的 Cargo 运行入口（只有 [`运行目标类型::Cargo可执行目标`] 有）。
    pub 入口: Option<运行入口>,
}

pub use editor_core::run_entry::{运行快照, 运行目标类型, 项目运行状态};

impl 运行命令 {
    /// 供界面/日志显示的一行命令。
    pub fn 显示(&self) -> String {
        let mut 文本 = self.程序.display().to_string();
        for 项 in &self.参数 {
            文本.push(' ');
            文本.push_str(项);
        }
        文本
    }
}

/// 运行管理器（Bevy 资源）。
///
/// # 它为什么不是一堆布尔量
///
/// 除了"在不在跑"，这里还保留**真实退出码、结束原因、起止时间与运行历史**。
/// 只存 `is_running` 会让下面三件事无法回答，而它们恰好是排查时最需要的：
/// 程序是自己崩溃还是被用户停的？上次那次退出码是多少？跑的是哪个入口？
#[derive(Resource, Default)]
pub struct 项目运行管理器 {
    /// 当前状态。
    pub 状态: 项目运行状态,
    /// 最近一次命令。
    pub 命令: Option<运行命令>,
    /// 当前运行的**目标类型**（未运行时沿用上次的值，便于界面解释"上次跑的是什么"）。
    pub 运行目标类型: 运行目标类型,
    /// 当前（或最近一次）运行的 Cargo 入口。
    pub 入口: Option<运行入口>,
    /// 子进程句柄。
    子进程: Option<Child>,
    /// 子进程 PID。
    pub 进程号: Option<u32>,
    /// 本次运行实例标识：每次启动都换新的，运行记录与调试会话都带上它。
    pub 运行实例标识: Option<String>,
    /// 本次运行的输出（最多保留 [`日志行上限`] 行）。
    ///
    /// 每次新运行**清空**：上一次运行的日志混进来会让"这次到底为什么失败"
    /// 变得不可判断。
    pub 日志: VecDeque<String>,
    /// 后台读线程的输出接收端。
    接收端: Option<Mutex<Receiver<String>>>,
    /// 最近一次错误。
    pub 最近错误: Option<String>,
    /// 最近一次运行的真实退出码。**被杀/没拿到时为 `None`**。
    pub 退出码: Option<i32>,
    /// 最近一次运行的结束原因。
    pub 结束原因: 结束原因,
    /// 最近一次运行的用时（毫秒）。
    pub 用时毫秒: u64,
    /// 最近一次运行的开始时间（UNIX 秒）。
    pub 开始时间: Option<u64>,
    /// 最近运行记录（有上限）。
    pub 历史: 运行历史,
    /// 界面同步用的修订号：任何可见变化都 +1。
    修订: u64,
}

/// 运行快照资源（放在 `plugin` 里注册）。
#[derive(Resource, Default)]
pub struct 运行快照资源 {
    /// 快照。
    pub 快照: 运行快照,
    /// 上次同步时的修订号。
    pub 修订: u64,
}

impl 项目运行管理器 {
    /// 记录一行输出（同时写入编辑器控制台由调用方负责）。
    pub fn 记日志(&mut self, 行: String) {
        while self.日志.len() >= 日志行上限 {
            self.日志.pop_front();
        }
        self.日志.push_back(行);
        self.标记变化();
    }

    /// 修订号（界面据此判断是否需要重新读一份快照）。
    pub fn 修订(&self) -> u64 {
        self.修订
    }

    fn 标记变化(&mut self) {
        self.修订 = self.修订.wrapping_add(1);
    }

    /// 清空本次运行日志并保留一次修订（换项目、重新运行前用）。
    pub fn 清空日志(&mut self) {
        self.日志.clear();
        self.标记变化();
    }

    /// 状态文案 + 最近一行输出，供工具栏显示。
    pub fn 摘要(&self) -> String {
        match self.日志.back() {
            Some(行) => format!("{} · {}", self.状态.文案(), 行),
            None => self.状态.文案().to_string(),
        }
    }

    /// 目标类型的本地化短语（用于状态摘要与日志，避免"跑了什么"含糊）。
    pub fn 目标说明(&self) -> String {
        match self.运行目标类型 {
            // 这句必须明确写出"不是游戏运行时"：它是产品诚实性的落点。
            运行目标类型::独立预览进程 => "独立预览进程（非游戏运行时）".to_string(),
            运行目标类型::项目可执行程序 => "项目可执行程序".to_string(),
            运行目标类型::Cargo可执行目标 => match self.命令.as_ref() {
                // 把真实的包/目标名带上：workspace 里"跑的是哪个成员"是
                // 用户最需要确认的一件事。
                Some(命令) => {
                    let 参数 = 命令.参数.join(" ");
                    format!("Cargo 可执行目标（cargo {参数}）")
                }
                None => "Cargo 可执行目标".to_string(),
            },
        }
    }

    /// 当前（或最近）运行的结果文本（真实退出码/结束原因）。
    pub fn 结果文本(&self) -> String {
        match self.历史.最近() {
            Some(记录) => 记录.结果文本(),
            None => "还没有运行记录".to_string(),
        }
    }

    /// 生成界面快照。
    pub fn 快照(&self) -> 运行快照 {
        运行快照 {
            状态: self.状态,
            目标类型: self.运行目标类型,
            入口: self.入口.clone(),
            命令行: self.命令.as_ref().map(运行命令::显示).unwrap_or_default(),
            进程号: self.进程号,
            运行实例标识: self.运行实例标识.clone(),
            退出码: self.退出码,
            结束原因: self.结束原因,
            用时毫秒: self.用时毫秒,
            最近错误: self.最近错误.clone(),
            日志: self.日志.iter().cloned().collect(),
            历史: self.历史.记录().to_vec(),
            目标说明: self.目标说明(),
        }
    }

    /// 启动命令：拉起子进程并开始读输出。
    ///
    /// # 重复启动的策略：先停旧实例
    ///
    /// 同一个项目重复点「运行」时，旧实例会先被停止。这样做的理由不是
    /// "省事"，而是**不受管理的进程是最坏的结果**：用户看到两个游戏窗口、
    /// 关掉编辑器后还有一个在跑、下次构建撞上文件锁 —— 全部由此而来。
    /// 这条策略是确定的、可预期的，而且会写进日志。
    pub fn 启动(&mut self, 命令: 运行命令) -> Result<(), String> {
        let 标识 = format!("run-{}", uuid::Uuid::new_v4().simple());
        self.启动带标识(命令, Some(标识))
    }

    /// 启动并显式指定运行实例标识。
    ///
    /// 调试桥接在启动**之前**就生成了同一个标识并注入环境变量，
    /// 因此这里必须接受它；否则运行记录与调试会话会出现两个不同的 id，
    /// "哪个连接属于哪次运行"就永远对不上。
    pub fn 启动带标识(
        &mut self,
        命令: 运行命令,
        运行实例标识: Option<String>,
    ) -> Result<(), String> {
        if self.子进程.is_some() {
            self.记日志("已有运行实例：先停止旧实例再启动".to_string());
            self.停止();
        }
        self.运行实例标识 = 运行实例标识;
        let 开始时间 = 现在秒();
        self.状态 = 项目运行状态::启动中;
        self.最近错误 = None;
        self.退出码 = None;
        self.结束原因 = 结束原因::进行中;
        self.用时毫秒 = 0;
        self.开始时间 = Some(开始时间);
        self.运行目标类型 = 命令.目标类型;
        self.入口 = 命令.入口.clone();
        self.命令 = Some(命令.clone());
        // 旧日志不属于这次运行。
        self.日志.clear();
        self.标记变化();

        let mut 进程 = Command::new(&命令.程序);
        进程
            .args(&命令.参数)
            .current_dir(&命令.工作目录)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // cargo 的彩色输出会变成一堆 `[1m` 转义序列，
            // 对"读日志找原因"毫无帮助。
            .env("CARGO_TERM_COLOR", "never")
            .env("CARGO_TERM_PROGRESS_WHEN", "never");
        for (键, 值) in &命令.环境 {
            进程.env(键, 值);
        }
        let mut 子进程 = match 进程.spawn() {
            Ok(子进程) => 子进程,
            Err(错误) => {
                let 文本 = format!("启动失败：{}（{}）", 错误, 命令.显示());
                self.状态 = 项目运行状态::失败;
                self.结束原因 = 结束原因::启动失败;
                self.最近错误 = Some(文本.clone());
                // 启动失败也是一次真实结果，必须进历史：
                // 否则"上次运行"会显示成上一次成功的记录。
                self.历史.开始(
                    命令.显示(),
                    命令.入口.clone(),
                    None,
                    self.运行实例标识.clone(),
                    开始时间,
                );
                self.历史
                    .结束当前(结束原因::启动失败, None, 开始时间, Some(文本.clone()), 0);
                self.记日志(文本.clone());
                self.标记变化();
                return Err(文本);
            }
        };
        let 号 = 子进程.id();
        self.进程号 = Some(号);

        let (发送端, 接收端) = channel();
        if let Some(输出) = 子进程.stdout.take() {
            转发输出(输出, 发送端.clone(), "out");
        }
        if let Some(错误输出) = 子进程.stderr.take() {
            转发输出(错误输出, 发送端, "err");
        }
        self.子进程 = Some(子进程);
        self.接收端 = Some(Mutex::new(接收端));
        self.状态 = 项目运行状态::运行中;
        self.历史.开始(
            命令.显示(),
            命令.入口.clone(),
            Some(号),
            self.运行实例标识.clone(),
            开始时间,
        );
        self.记日志(format!(
            "已启动（PID {号}，目标：{}）：{}",
            self.目标说明(),
            命令.显示()
        ));
        Ok(())
    }

    /// 停止：结束子进程（Windows 上连同子进程树）。
    ///
    /// 用户主动停止**不是**程序自己退出：结束原因记成
    /// [`结束原因::用户停止`]，退出码保持 `None`。
    pub fn 停止(&mut self) {
        let 结束时间 = 现在秒();
        if let Some(mut 子进程) = self.子进程.take() {
            let 号 = 子进程.id();
            // Windows 下 cargo/脚本会再拉子进程，单 kill 会留下孤儿。
            // 这里用 taskkill /T；非 Windows 回退到 child.kill()。
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/PID", &号.to_string(), "/T", "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = 子进程.kill();
            let _ = 子进程.wait();
            self.记日志(format!("已停止进程 {号}"));
            self.结束原因 = 结束原因::用户停止;
            self.退出码 = None;
            self.用时毫秒 = self
                .开始时间
                .map(|开始| 结束时间.saturating_sub(开始) * 1000)
                .unwrap_or(0);
            self.历史.结束当前(
                结束原因::用户停止,
                None,
                结束时间,
                Some(format!("用户停止（PID {号}）")),
                self.日志.len(),
            );
        }
        if self.状态.在运行() {
            self.状态 = 项目运行状态::已停止;
        }
        self.进程号 = None;
        self.接收端 = None;
        self.标记变化();
    }

    /// 轮询：读输出、检测自然退出。
    pub fn 轮询(&mut self) -> Vec<String> {
        let mut 新行 = Vec::new();
        let mut 已收到 = Vec::new();
        if let Some(接收端) = &self.接收端 {
            let 接收端 = 接收端.lock().expect("运行输出接收端不应中毒");
            while let Ok(行) = 接收端.try_recv() {
                已收到.push(行);
            }
        }
        for 行 in 已收到 {
            self.记日志(行.clone());
            新行.push(行);
        }
        if let Some(子进程) = self.子进程.as_mut() {
            match 子进程.try_wait() {
                Ok(Some(退出)) => {
                    let 结束时间 = 现在秒();
                    let 码 = 退出.code();
                    self.记日志(format!(
                        "进程已退出：{}",
                        match 码 {
                            Some(码) => format!("退出码 {码}"),
                            None => "无退出码（被系统终止）".to_string(),
                        }
                    ));
                    let 原因 = if 退出.success() {
                        结束原因::正常退出
                    } else if 码.is_none() {
                        结束原因::被终止
                    } else {
                        结束原因::异常退出
                    };
                    self.状态 = if 退出.success() {
                        项目运行状态::已停止
                    } else {
                        项目运行状态::失败
                    };
                    self.退出码 = 码;
                    self.结束原因 = 原因;
                    self.用时毫秒 = self
                        .开始时间
                        .map(|开始| 结束时间.saturating_sub(开始) * 1000)
                        .unwrap_or(0);
                    let 说明 = 码
                        .map(|码| format!("退出码 {码}"))
                        .unwrap_or_else(|| "没有拿到退出码（可能被系统终止）".to_string());
                    self.历史
                        .结束当前(原因, 码, 结束时间, Some(说明), self.日志.len());
                    self.子进程 = None;
                    self.进程号 = None;
                    self.标记变化();
                }
                Ok(None) => {}
                Err(错误) => {
                    self.状态 = 项目运行状态::失败;
                    self.结束原因 = 结束原因::被终止;
                    self.最近错误 = Some(format!("等待进程失败：{错误}"));
                    self.子进程 = None;
                    self.进程号 = None;
                    self.历史.结束当前(
                        结束原因::被终止,
                        None,
                        现在秒(),
                        Some(format!("等待进程失败：{错误}")),
                        self.日志.len(),
                    );
                    self.标记变化();
                }
            }
        }
        新行
    }

    /// 换项目时重置：停止进程、清掉日志、清掉历史与入口。
    pub fn 重置(&mut self) {
        if self.子进程.is_some() {
            self.停止();
        }
        self.日志.clear();
        self.历史.清空();
        self.状态 = 项目运行状态::空闲;
        self.命令 = None;
        self.入口 = None;
        self.进程号 = None;
        self.退出码 = None;
        self.结束原因 = 结束原因::进行中;
        self.最近错误 = None;
        self.用时毫秒 = 0;
        self.开始时间 = None;
        self.标记变化();
    }
}

/// 当前 UNIX 秒。
fn 现在秒() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_secs())
        .unwrap_or(0)
}

/// 后台线程逐行转发子进程输出（stdout/stderr 都转发）。
fn 转发输出(
    流: impl std::io::Read + Send + 'static,
    发送端: Sender<String>,
    前缀: &'static str,
) {
    std::thread::spawn(move || {
        let 读 = BufReader::new(流);
        for 行 in 读.lines() {
            let Ok(行) = 行 else { break };
            let 文本 = if 前缀 == "err" {
                format!("[stderr] {行}")
            } else {
                行
            };
            if 发送端.send(文本).is_err() {
                break;
            }
        }
    });
}

/// 解析当前项目的运行命令。
///
/// # 目标解析顺序（决定"运行"到底跑什么）
///
/// 1. 原生 Cargo 工程 + 选中的运行入口 → `cargo run -p <包> --bin <目标>`
///    （[`运行目标类型::Cargo可执行目标`]）。包名/目标名来自
///    `cargo metadata` 的真实结果，**不是**按目录名猜的。
///    找不到 `cargo` 时明确报错。
/// 2. 项目清单声明了 `运行.程序` → 用**它**（微风项目声明具体程序，
///    [`运行目标类型::项目可执行程序`]）。程序找不到时返回错误，
///    **不退回**预览进程：用户以为在跑游戏、实际只是又开了一个编辑器，
///    是最坏的误导。
/// 3. 原生 Cargo 工程但**没有**可执行目标（只有库/没有 bin）→ 明确报错。
/// 4. 微风项目且没声明 → 用微风自身再开一个进程打开该项目
///    （[`运行目标类型::独立预览进程`]）。这条链路是真实的（独立进程、
///    真实 stdout/stderr、可停止），但**不是游戏运行时**。
///
/// `可执行文件` 是编辑器自身的路径，只有第 4 种情况用得上。
pub fn 解析运行命令(
    项目根: &Path,
    当前模式: &str,
    可执行文件: &Path,
    清单: Option<&editor_core::project::项目清单>,
    类型: editor_core::project::项目类型,
    入口: Option<&运行入口>,
) -> Result<运行命令, String> {
    if !项目根.is_dir() {
        return Err(format!("项目目录不存在：{}", 项目根.display()));
    }
    let 是微风工程 = 项目根.join("project.ron").is_file();
    let 是cargo工程 = 项目根.join("Cargo.toml").is_file();
    if !是微风工程 && !是cargo工程 {
        return Err(format!(
            "项目根目录下既没有 project.ron（微风项目）也没有 Cargo.toml（Cargo 工程）：{}",
            项目根.display()
        ));
    }

    // ---- 情况 1：原生 Cargo 工程 + 选中的运行入口 ----
    if 类型 == editor_core::project::项目类型::原生Cargo
        && let Some(入口) = 入口
    {
        if 入口.包名.trim().is_empty() || 入口.目标名.trim().is_empty() {
            return Err("运行入口不完整：缺少包名或目标名".to_string());
        }
        // `cargo` 必须在 PATH 上；这里提前查而不是等 spawn 失败，
        // 因为"找不到 cargo"和"游戏启动崩溃"是完全不同的两件事。
        let 程序 = 解析程序路径(项目根, "cargo").map_err(|_| {
            "在 PATH 上找不到 `cargo`：无法构建或运行 Cargo 目标。\
             请确认 Rust 工具链已安装并在 PATH 上"
                .to_string()
        })?;
        return Ok(运行命令 {
            程序,
            参数: 入口.命令行参数(),
            工作目录: 项目根.to_path_buf(),
            环境: Vec::new(),
            目标类型: 运行目标类型::Cargo可执行目标,
            入口: Some(入口.clone()),
        });
    }

    // ---- 情况 2：项目显式声明了运行目标 ----
    if let Some(运行) = 清单.and_then(|清单| 清单.运行.as_ref())
        && 运行.已声明()
    {
        let 程序 = 解析程序路径(项目根, 运行.程序.trim())?;
        let 工作目录 = match 运行.工作目录.as_deref() {
            Some(相对) if !相对.trim().is_empty() => {
                let 目录 = 项目根.join(相对);
                if !目录.is_dir() {
                    return Err(format!(
                        "运行目标的工作目录不存在：{}（声明为 `{相对}`）",
                        目录.display()
                    ));
                }
                目录
            }
            _ => 项目根.to_path_buf(),
        };
        return Ok(运行命令 {
            程序,
            参数: 运行.参数.clone(),
            工作目录,
            环境: 运行
                .环境
                .iter()
                .map(|(键, 值)| (键.clone(), 值.clone()))
                .collect(),
            目标类型: if 类型 == editor_core::project::项目类型::原生Cargo {
                运行目标类型::Cargo可执行目标
            } else {
                运行目标类型::项目可执行程序
            },
            入口: None,
        });
    }

    // ---- 情况 3：原生 Cargo 工程没有可执行目标 ----
    //
    // 这里**必须**报错而不是退回预览进程：只有库的工程没有任何游戏可跑，
    // 再开一个编辑器窗口只会让用户以为"运行成功"。
    if 类型 == editor_core::project::项目类型::原生Cargo {
        return Err(format!(
            "该 Cargo 工程没有可执行目标（只有库，或 [workspace] 里没有 bin）：无法启动游戏。\
             项目根：{}；请检查 Cargo.toml 的 [[bin]] 或 src/main.rs。\
             你仍然可以在这个工程上执行构建与测试",
            项目根.display()
        ));
    }

    // ---- 情况 4：微风项目的默认独立预览进程 ----
    Ok(运行命令 {
        程序: 可执行文件.to_path_buf(),
        参数: Vec::new(),
        工作目录: 项目根.to_path_buf(),
        环境: vec![
            (
                "BEVY_STUDIO_OPEN_PROJECT".to_string(),
                项目根.display().to_string(),
            ),
            ("BEVY_STUDIO_MODE".to_string(), 当前模式.to_string()),
        ],
        目标类型: 运行目标类型::独立预览进程,
        入口: None,
    })
}

/// 把清单里的 `程序` 解析成可执行路径。
///
/// * 绝对路径 → 直接校验存在性；
/// * 含分隔符的相对路径 → 相对项目根解析（这样项目可以自带运行时）；
/// * 裸名字 → 在 `PATH` 上查找。
fn 解析程序路径(项目根: &Path, 写法: &str) -> Result<PathBuf, String> {
    if 写法.is_empty() {
        return Err("运行目标声明为空".to_string());
    }
    let 候选 = Path::new(写法);
    if 候选.is_absolute() {
        return if 候选.is_file() {
            Ok(候选.to_path_buf())
        } else {
            Err(format!("运行目标不存在：{}", 候选.display()))
        };
    }
    // 含分隔符：相对项目根。
    if 写法.contains('/') || 写法.contains('\\') {
        let 全 = 项目根.join(候选);
        return if 全.is_file() {
            Ok(全)
        } else {
            Err(format!(
                "运行目标不存在：{}（相对项目根解析）",
                全.display()
            ))
        };
    }
    // 裸名字：在 PATH 上找。
    let 路径 = std::env::var_os("PATH").unwrap_or_default();
    for 目录 in std::env::split_paths(&路径) {
        let 全 = 目录.join(候选);
        if 全.is_file() {
            return Ok(全);
        }
        // Windows 上裸名字常常省略 .exe。
        #[cfg(windows)]
        {
            let 加后缀 = 目录.join(format!("{写法}.exe"));
            if 加后缀.is_file() {
                return Ok(加后缀);
            }
        }
    }
    Err(format!("在 PATH 上找不到运行目标 `{写法}`"))
}

/// Bevy 系统：每帧轮询运行输出，把新行写进编辑器控制台，并采集诊断。
///
/// # 为什么诊断采集在这里而不是另起一个系统
///
/// `轮询` 会**消费**通道里的行。如果两个系统各自轮询一次，
/// 后执行的那个永远收不到任何东西（而且谁先执行取决于调度顺序，
/// 表现为"有时能采到、有时采不到"的偶发缺陷）。
/// 因此"取一次、分发给两个消费者"必须发生在同一个系统里。
pub fn 轮询项目运行(
    mut 管理器: ResMut<项目运行管理器>,
    mut 编辑器: ResMut<EditorStateHolder>,
    mut 诊断: ResMut<crate::diagnostics::诊断中心>,
) {
    let 运行序号 = 管理器.历史.最近().map(|记录| 记录.序号);
    for 行 in 管理器.轮询() {
        诊断.采集运行日志(&行, 运行序号);
        if 行.starts_with("[stderr]") {
            编辑器.状态.警告(行);
        } else {
            编辑器.状态.信息(行);
        }
    }
    if let Some(错误) = 管理器.最近错误.take() {
        编辑器.状态.错误(错误);
    }
}

/// 把运行管理器的当前状态同步成界面可读的快照。
///
/// 修订号没变就不克隆：运行日志可能有两千行，每帧克隆会变成稳定的
/// 每秒几百 KB 分配。
pub fn 同步运行快照(
    管理器: Res<项目运行管理器>, mut 快照: ResMut<运行快照资源>
) {
    if 快照.修订 == 管理器.修订() {
        return;
    }
    快照.快照 = 管理器.快照();
    快照.修订 = 管理器.修订();
}

/// 编辑器退出时停止本次运行，避免留下孤儿进程。
pub fn 退出时停止项目运行(
    mut 退出: MessageReader<AppExit>,
    mut 管理器: ResMut<项目运行管理器>,
) {
    if 退出.read().next().is_some() && 管理器.状态.在运行() {
        管理器.停止();
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_core::project::{运行目标清单, 项目清单, 项目类型};

    #[test]
    fn 缺少项目清单时给出明确错误() {
        let 目录 = std::env::temp_dir().join("breeze_run_missing_project");
        let _ = std::fs::create_dir_all(&目录);
        let 错误 = 解析运行命令(
            &目录,
            "2d",
            Path::new("bevy-studio.exe"),
            None,
            项目类型::微风,
            None,
        )
        .unwrap_err();
        assert!(
            错误.contains("project.ron"),
            "错误应指出缺少项目清单：{错误}"
        );
    }

    #[test]
    fn 未声明运行目标时退回独立预览进程() {
        let 目录 = std::env::temp_dir().join("breeze_run_ok_project");
        let _ = std::fs::create_dir_all(&目录);
        std::fs::write(目录.join("project.ron"), "()").unwrap();
        let 命令 = 解析运行命令(
            &目录,
            "3d",
            Path::new("bevy-studio.exe"),
            None,
            项目类型::微风,
            None,
        )
        .unwrap();
        assert_eq!(命令.工作目录, 目录);
        assert_eq!(
            命令.目标类型,
            运行目标类型::独立预览进程,
            "没声明运行目标的项目只能以独立预览进程运行"
        );
        assert!(命令.目标类型.是预览进程());
        assert!(
            命令
                .环境
                .iter()
                .any(|(键, 值)| 键 == "BEVY_STUDIO_OPEN_PROJECT"
                    && 值.ends_with("breeze_run_ok_project"))
        );
        assert!(
            命令
                .环境
                .iter()
                .any(|(键, 值)| 键 == "BEVY_STUDIO_MODE" && 值 == "3d")
        );
    }

    /// 项目声明的运行目标必须被**如实**采用，并且工作目录是项目根。
    #[test]
    fn 声明的运行目标被采用且工作目录为项目根() {
        let 目录 = std::env::temp_dir().join("breeze_run_declared_target");
        let _ = std::fs::remove_dir_all(&目录);
        std::fs::create_dir_all(&目录).unwrap();
        std::fs::write(目录.join("project.ron"), "()").unwrap();
        // 造一个真实存在的"程序"：项目根下的 `bin/` 子目录里（验证相对项目根解析）。
        let 程序名 = if cfg!(windows) {
            "runtime.exe"
        } else {
            "runtime"
        };
        std::fs::create_dir_all(目录.join("bin")).unwrap();
        std::fs::write(目录.join("bin").join(程序名), b"stub").unwrap();

        let mut 清单 = 项目清单::新建("测试");
        清单.运行 = Some(运行目标清单 {
            程序: format!("bin/{程序名}"),
            参数: vec!["--demo".to_string()],
            环境: [("K".to_string(), "V".to_string())].into_iter().collect(),
            工作目录: None,
        });
        let 命令 = 解析运行命令(
            &目录,
            "2d",
            Path::new("bevy-studio.exe"),
            Some(&清单),
            项目类型::微风,
            None,
        )
        .expect("应当能解析");
        assert_eq!(
            命令.目标类型,
            运行目标类型::项目可执行程序,
            "声明了运行目标就不该退回预览进程"
        );
        assert_eq!(命令.程序, 目录.join("bin").join(程序名));
        assert_eq!(命令.参数, vec!["--demo".to_string()]);
        assert_eq!(命令.环境, vec![("K".to_string(), "V".to_string())]);
        assert_eq!(命令.工作目录, 目录);
    }

    /// 相对项目根解析：项目可以自带运行时，不必依赖编辑器所在目录。
    #[test]
    fn 运行目标可以声明项目内相对路径与工作目录() {
        let 目录 = std::env::temp_dir().join("breeze_run_relative_target");
        let _ = std::fs::remove_dir_all(&目录);
        std::fs::create_dir_all(目录.join("bin")).unwrap();
        std::fs::create_dir_all(目录.join("run_dir")).unwrap();
        std::fs::write(目录.join("project.ron"), "()").unwrap();
        let 程序名 = if cfg!(windows) { "game.exe" } else { "game" };
        std::fs::write(目录.join("bin").join(程序名), b"stub").unwrap();

        let mut 清单 = 项目清单::新建("测试");
        清单.运行 = Some(运行目标清单 {
            程序: format!("bin/{程序名}"),
            参数: Vec::new(),
            环境: Default::default(),
            工作目录: Some("run_dir".to_string()),
        });
        let 命令 = 解析运行命令(
            &目录,
            "3d",
            Path::new("bevy-studio.exe"),
            Some(&清单),
            项目类型::微风,
            None,
        )
        .expect("应当能解析");
        assert_eq!(命令.程序, 目录.join("bin").join(程序名));
        assert_eq!(命令.工作目录, 目录.join("run_dir"));
    }

    /// 声明的程序不存在时必须报错，**不得**退回到独立预览进程。
    ///
    /// 这条测试守的是产品诚实性：用户以为在跑游戏，实际只是又开了一个编辑器，
    /// 比"运行失败"更糟。
    #[test]
    fn 声明的运行目标不存在时报错而不是退回预览进程() {
        let 目录 = std::env::temp_dir().join("breeze_run_missing_target");
        let _ = std::fs::remove_dir_all(&目录);
        std::fs::create_dir_all(&目录).unwrap();
        std::fs::write(目录.join("project.ron"), "()").unwrap();

        let mut 清单 = 项目清单::新建("测试");
        清单.运行 = Some(运行目标清单 {
            程序: "根本不存在/runtime.exe".to_string(),
            参数: Vec::new(),
            环境: Default::default(),
            工作目录: None,
        });
        let 错误 = 解析运行命令(
            &目录,
            "2d",
            Path::new("bevy-studio.exe"),
            Some(&清单),
            项目类型::微风,
            None,
        )
        .expect_err("必须报错");
        assert!(
            错误.contains("运行目标不存在"),
            "错误应明确指出运行目标不存在：{错误}"
        );
    }

    #[test]
    fn 空白的运行目标声明视为未声明() {
        let 清单 = 运行目标清单 {
            程序: "   ".to_string(),
            参数: Vec::new(),
            环境: Default::default(),
            工作目录: None,
        };
        assert!(!清单.已声明());
    }

    #[test]
    fn 目标类型有稳定的证据代号且能区分预览() {
        assert_eq!(运行目标类型::独立预览进程.代号(), "preview-process");
        assert_eq!(运行目标类型::项目可执行程序.代号(), "game");
        assert!(运行目标类型::独立预览进程.是预览进程());
        assert!(!运行目标类型::项目可执行程序.是预览进程());
    }

    #[test]
    fn 状态机只在运行中才算运行() {
        assert!(项目运行状态::启动中.在运行());
        assert!(项目运行状态::运行中.在运行());
        assert!(!项目运行状态::空闲.在运行());
        assert!(!项目运行状态::已停止.在运行());
        assert!(!项目运行状态::失败.在运行());
    }

    #[cfg(windows)]
    #[test]
    fn 启动真实命令并捕获输出与退出状态() {
        let 临时 = std::env::temp_dir();
        let mut 管理器 = 项目运行管理器::default();
        管理器
            .启动(运行命令 {
                程序: "cmd".into(),
                参数: vec!["/C".into(), "echo breeze-run-ok".into()],
                工作目录: 临时,
                环境: Vec::new(),
                目标类型: 运行目标类型::独立预览进程,
                入口: None,
            })
            .expect("应当能启动 cmd");
        // 等子进程输出并退出（最多 5 秒）。
        let mut 全部 = Vec::new();
        for _ in 0..100 {
            全部.extend(管理器.轮询());
            if 管理器.状态 == 项目运行状态::已停止 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            全部.iter().any(|行| 行.contains("breeze-run-ok")),
            "应当捕获到子进程输出：{全部:?}"
        );
        assert_eq!(管理器.状态, 项目运行状态::已停止);
        assert!(管理器.进程号.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn 停止会结束正在运行的子进程() {
        let mut 管理器 = 项目运行管理器::default();
        管理器
            .启动(运行命令 {
                程序: "cmd".into(),
                参数: vec!["/C".into(), "ping -n 30 127.0.0.1 > NUL".into()],
                工作目录: std::env::temp_dir(),
                环境: Vec::new(),
                目标类型: 运行目标类型::独立预览进程,
                入口: None,
            })
            .expect("应当能启动 cmd");
        assert!(管理器.状态.在运行());
        let 号 = 管理器.进程号.expect("应当有 PID");
        管理器.停止();
        assert_eq!(管理器.状态, 项目运行状态::已停止);
        assert!(管理器.进程号.is_none());
        // 再确认该 PID 已不存在（taskkill + kill 双保险）。
        let 输出 = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {号}"), "/NH"])
            .output()
            .expect("tasklist 应当可运行");
        let 文本 = String::from_utf8_lossy(&输出.stdout);
        assert!(
            !文本.contains(&号.to_string()),
            "进程 {号} 仍在运行：{文本}"
        );
    }

    /// stderr 必须被单独标记后转发，否则用户无法区分"子进程出错"与普通输出。
    #[cfg(windows)]
    #[test]
    fn stderr_被标记后进入日志() {
        let mut 管理器 = 项目运行管理器::default();
        管理器
            .启动(运行命令 {
                程序: "cmd".into(),
                参数: vec!["/C".into(), "echo breeze-stderr-1>&2".into()],
                工作目录: std::env::temp_dir(),
                环境: Vec::new(),
                目标类型: 运行目标类型::项目可执行程序,
                入口: None,
            })
            .expect("应当能启动 cmd");
        assert_eq!(管理器.运行目标类型, 运行目标类型::项目可执行程序);
        let mut 全部 = Vec::new();
        for _ in 0..100 {
            全部.extend(管理器.轮询());
            if 管理器.状态 == 项目运行状态::已停止 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            全部
                .iter()
                .any(|行| 行.starts_with("[stderr]") && 行.contains("breeze-stderr")),
            "stderr 行必须带 `[stderr]` 前缀：{全部:?}"
        );
    }

    #[test]
    fn 日志按上限滚动保留最近的() {
        let mut 管理器 = 项目运行管理器::default();
        let 总数 = 日志行上限 + 500;
        for 序号 in 0..总数 {
            管理器.记日志(format!("第{序号}行"));
        }
        // 上限是**有界**的：无限积累会让内存随运行时长线性增长。
        assert_eq!(管理器.日志.len(), 日志行上限);
        assert_eq!(
            管理器.日志.back().map(String::as_str),
            Some(format!("第{}行", 总数 - 1).as_str())
        );
        assert_eq!(管理器.日志.front().map(String::as_str), Some("第500行"));
    }

    #[test]
    fn 启动失败也留下一条真实记录() {
        let mut 管理器 = 项目运行管理器::default();
        let 错误 = 管理器
            .启动(运行命令 {
                程序: "绝对不存在的程序-breeze-probe".into(),
                参数: Vec::new(),
                工作目录: std::env::temp_dir(),
                环境: Vec::new(),
                目标类型: 运行目标类型::项目可执行程序,
                入口: None,
            })
            .expect_err("不存在的程序必须启动失败");
        assert!(错误.contains("启动失败"), "{错误}");
        assert_eq!(管理器.状态, 项目运行状态::失败);
        assert_eq!(管理器.结束原因, 结束原因::启动失败);
        let 记录 = 管理器.历史.最近().expect("启动失败也要有记录");
        assert_eq!(记录.结束原因, 结束原因::启动失败);
        assert_eq!(记录.退出码, None, "启动失败没有退出码，不得编造");
    }

    #[cfg(windows)]
    #[test]
    fn 正常退出会记录真实退出码与结束原因() {
        let mut 管理器 = 项目运行管理器::default();
        管理器
            .启动(运行命令 {
                程序: "cmd".into(),
                参数: vec!["/C".into(), "exit 0".into()],
                工作目录: std::env::temp_dir(),
                环境: Vec::new(),
                目标类型: 运行目标类型::项目可执行程序,
                入口: None,
            })
            .expect("应当能启动 cmd");
        for _ in 0..100 {
            let _ = 管理器.轮询();
            if !管理器.状态.在运行() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(管理器.结束原因, 结束原因::正常退出);
        assert_eq!(管理器.退出码, Some(0));
        let 记录 = 管理器.历史.最近().unwrap();
        assert_eq!(记录.退出码, Some(0));
        assert!(记录.结束时间.is_some(), "结束时间必须被记录");
        assert!(记录.结果文本().contains("正常退出"));
    }

    #[cfg(windows)]
    #[test]
    fn 非零退出码被如实记录成异常退出() {
        let mut 管理器 = 项目运行管理器::default();
        管理器
            .启动(运行命令 {
                程序: "cmd".into(),
                参数: vec!["/C".into(), "exit 7".into()],
                工作目录: std::env::temp_dir(),
                环境: Vec::new(),
                目标类型: 运行目标类型::项目可执行程序,
                入口: None,
            })
            .expect("应当能启动 cmd");
        for _ in 0..100 {
            let _ = 管理器.轮询();
            if !管理器.状态.在运行() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(管理器.状态, 项目运行状态::失败);
        assert_eq!(管理器.结束原因, 结束原因::异常退出);
        assert_eq!(管理器.退出码, Some(7), "退出码必须是真实的 7");
    }

    #[cfg(windows)]
    #[test]
    fn 用户停止不会被当成正常退出() {
        let mut 管理器 = 项目运行管理器::default();
        管理器
            .启动(运行命令 {
                程序: "cmd".into(),
                参数: vec!["/C".into(), "ping -n 30 127.0.0.1 > NUL".into()],
                工作目录: std::env::temp_dir(),
                环境: Vec::new(),
                目标类型: 运行目标类型::项目可执行程序,
                入口: None,
            })
            .expect("应当能启动 cmd");
        管理器.停止();
        assert_eq!(管理器.结束原因, 结束原因::用户停止);
        assert_eq!(管理器.退出码, None, "被杀进程不得伪造退出码");
        let 记录 = 管理器.历史.最近().unwrap();
        assert_eq!(记录.结束原因, 结束原因::用户停止);
        assert!(记录.结果文本().contains("不是程序自己退出"));
    }

    #[test]
    fn 重新启动会先停止旧实例() {
        #[cfg(windows)]
        {
            let mut 管理器 = 项目运行管理器::default();
            let 命令 = || 运行命令 {
                程序: "cmd".into(),
                参数: vec!["/C".into(), "ping -n 30 127.0.0.1 > NUL".into()],
                工作目录: std::env::temp_dir(),
                环境: Vec::new(),
                目标类型: 运行目标类型::项目可执行程序,
                入口: None,
            };
            管理器.启动(命令()).unwrap();
            let 旧进程 = 管理器.进程号.expect("应当有 PID");
            管理器.启动(命令()).unwrap();
            assert_ne!(管理器.进程号, Some(旧进程), "第二个实例必须是新进程");
            // 旧实例必须已经被停止（否则会出现不受管理的重复进程）。
            let 输出 = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {旧进程}"), "/NH"])
                .output()
                .expect("tasklist 应当可运行");
            let 文本 = String::from_utf8_lossy(&输出.stdout);
            assert!(!文本.contains(&旧进程.to_string()), "旧进程仍在：{文本}");
            assert_eq!(管理器.历史.记录().len(), 2, "两次启动都要留下记录");
            管理器.停止();
        }
    }

    #[test]
    fn 重置会清掉日志历史与入口() {
        let mut 管理器 = 项目运行管理器::default();
        管理器.记日志("一行".to_string());
        管理器.历史.开始("cargo run", None, None, None, 0);
        管理器.重置();
        assert!(管理器.日志.is_empty());
        assert!(管理器.历史.记录().is_empty());
        assert!(管理器.入口.is_none());
        assert_eq!(管理器.状态, 项目运行状态::空闲);
    }
}
