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
//! # 能力边界（不要误解）
//!
//! 微风**不**会编译项目，也不内置 Bevy 游戏运行时；"独立预览进程"只是
//! 另一个编辑器窗口。真正的游戏运行入口靠项目清单声明。
//!
//! 子进程通过 `std::process` + 后台读线程实现，不阻塞 Bevy 主线程；
//! 读线程只在管道关闭时结束，避免子进程因管道写满而挂起。

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::prelude::*;

use crate::plugin::EditorStateHolder;

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
}

/// 运行目标类型。
///
/// 区分它是本模块的核心职责：把"又开了一个编辑器窗口"当成"跑游戏"
/// 是产品层面最严重的误导。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 运行目标类型 {
    /// 独立预览进程：再次启动微风并打开同一项目。
    ///
    /// 这是默认值：**没声明运行目标的项目不会被当成有游戏运行时**。
    #[default]
    独立预览进程,
    /// 项目声明的真实可执行程序。
    项目可执行程序,
    /// 原生 Cargo 工程里的可执行目标：通过 `cargo run -p <包> --bin <目标>` 启动。
    ///
    /// # 为什么和 [`Self::项目可执行程序`] 分开
    ///
    /// 它不是"某个固定路径的程序"，而是"这个工程的某个 Cargo 目标"：
    /// 目标未构建时由 cargo 先构建，包/目标名来自 `Cargo.toml`，
    /// 失败原因是编译错误而不是"程序不存在"。这三种差别都必须在
    /// 界面上说清楚，否则用户会把编译失败当成运行器坏了。
    Cargo可执行目标,
}

impl 运行目标类型 {
    /// 证据 JSON 里的稳定代号（不随语言变化）。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::独立预览进程 => "preview-process",
            Self::项目可执行程序 => "game",
            Self::Cargo可执行目标 => "cargo-target",
        }
    }

    /// 是否**不**是真正的游戏运行时。
    pub fn 是预览进程(self) -> bool {
        matches!(self, Self::独立预览进程)
    }
}

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

/// 运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum 项目运行状态 {
    /// 没有在运行。
    #[default]
    空闲,
    /// 正在启动（进程还没起来）。
    启动中,
    /// 子进程在运行。
    运行中,
    /// 已停止（主动停止或自然退出）。
    已停止,
    /// 启动失败。
    失败,
}

impl 项目运行状态 {
    /// 是否在运行（含启动中）。
    pub fn 在运行(&self) -> bool {
        matches!(self, Self::启动中 | Self::运行中)
    }

    /// 中文状态文案。
    pub fn 文案(&self) -> &'static str {
        match self {
            Self::空闲 => "空闲",
            Self::启动中 => "启动中",
            Self::运行中 => "运行中",
            Self::已停止 => "已停止",
            Self::失败 => "失败",
        }
    }
}

/// 运行管理器（Bevy 资源）。
#[derive(Resource, Default)]
pub struct 项目运行管理器 {
    /// 当前状态。
    pub 状态: 项目运行状态,
    /// 最近一次命令。
    pub 命令: Option<运行命令>,
    /// 当前运行的**目标类型**（未运行时沿用上次的值，便于界面解释"上次跑的是什么"）。
    pub 运行目标类型: 运行目标类型,
    /// 子进程句柄。
    子进程: Option<Child>,
    /// 子进程 PID。
    pub 进程号: Option<u32>,
    /// 最近输出（最多保留 200 行）。
    pub 日志: VecDeque<String>,
    /// 后台读线程的输出接收端。
    接收端: Option<Mutex<Receiver<String>>>,
    /// 最近一次错误。
    pub 最近错误: Option<String>,
}

impl 项目运行管理器 {
    /// 记录一行输出（同时写入编辑器控制台由调用方负责）。
    pub fn 记日志(&mut self, 行: String) {
        if self.日志.len() >= 200 {
            self.日志.pop_front();
        }
        self.日志.push_back(行);
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

    /// 启动命令：拉起子进程并开始读输出。
    ///
    /// 已经在跑时**先停止旧实例**，避免出现不受管理的重复进程。
    pub fn 启动(&mut self, 命令: 运行命令) -> Result<(), String> {
        if self.子进程.is_some() {
            self.停止();
        }
        self.状态 = 项目运行状态::启动中;
        self.最近错误 = None;
        self.运行目标类型 = 命令.目标类型;
        self.命令 = Some(命令.clone());

        let mut 进程 = Command::new(&命令.程序);
        进程
            .args(&命令.参数)
            .current_dir(&命令.工作目录)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (键, 值) in &命令.环境 {
            进程.env(键, 值);
        }
        let mut 子进程 = 进程.spawn().map_err(|错误| {
            let 文本 = format!("启动失败：{}（{}）", 错误, 命令.显示());
            self.状态 = 项目运行状态::失败;
            self.最近错误 = Some(文本.clone());
            文本
        })?;
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
        self.记日志(format!(
            "已启动（PID {号}，目标：{}）：{}",
            self.目标说明(),
            命令.显示()
        ));
        Ok(())
    }

    /// 停止：结束子进程（Windows 上连同子进程树）。
    pub fn 停止(&mut self) {
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
        }
        if self.状态.在运行() {
            self.状态 = 项目运行状态::已停止;
        }
        self.进程号 = None;
        self.接收端 = None;
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
                    self.记日志(format!("进程已退出：{退出}"));
                    self.状态 = if 退出.success() {
                        项目运行状态::已停止
                    } else {
                        项目运行状态::失败
                    };
                    self.子进程 = None;
                    self.进程号 = None;
                }
                Ok(None) => {}
                Err(错误) => {
                    self.状态 = 项目运行状态::失败;
                    self.最近错误 = Some(format!("等待进程失败：{错误}"));
                    self.子进程 = None;
                    self.进程号 = None;
                }
            }
        }
        新行
    }
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
/// 1. 项目清单声明了 `运行.程序` → 用**它**。原生 Cargo 工程的清单是
///    打开时从 `Cargo.toml` 推导的，默认目标即
///    `cargo run -p <包> --bin <目标>`（[`运行目标类型::Cargo可执行目标`]）；
///    微风项目声明的是具体程序（[`运行目标类型::项目可执行程序`]）。
///    程序找不到时返回错误，**不退回**预览进程：用户以为在跑游戏、
///    实际只是又开了一个编辑器，是最坏的误导。
/// 2. 原生 Cargo 工程但**没有**可执行目标（只有库/没有 bin）→ 明确报错。
/// 3. 微风项目且没声明 → 用微风自身再开一个进程打开该项目
///    （[`运行目标类型::独立预览进程`]）。这条链路是真实的（独立进程、
///    真实 stdout/stderr、可停止），但**不是游戏运行时**。
///
/// `可执行文件` 是编辑器自身的路径，只有第 3 种情况用得上。
pub fn 解析运行命令(
    项目根: &Path,
    当前模式: &str,
    可执行文件: &Path,
    清单: Option<&editor_core::project::项目清单>,
    类型: editor_core::project::项目类型,
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

    // ---- 情况 1：项目显式声明了运行目标 ----
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
        });
    }

    // ---- 情况 2：原生 Cargo 工程没有可执行目标 ----
    //
    // 这里**必须**报错而不是退回预览进程：只有库的工程没有任何游戏可跑，
    // 再开一个编辑器窗口只会让用户以为"运行成功"。
    if 类型 == editor_core::project::项目类型::原生Cargo {
        return Err(format!(
            "该 Cargo 工程没有可执行目标（只有库，或 [workspace] 里没有 bin）：无法启动游戏。\
             项目根：{}；请检查 Cargo.toml 的 [[bin]] 或 src/main.rs",
            项目根.display()
        ));
    }

    // ---- 情况 3：微风项目的默认独立预览进程 ----
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

/// Bevy 系统：每帧轮询运行输出，把新行写进编辑器控制台。
pub fn 轮询项目运行(
    mut 管理器: ResMut<项目运行管理器>,
    mut 编辑器: ResMut<EditorStateHolder>,
) {
    for 行 in 管理器.轮询() {
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
    fn 日志只保留最近两百行() {
        let mut 管理器 = 项目运行管理器::default();
        for 序号 in 0..250 {
            管理器.记日志(format!("第{序号}行"));
        }
        assert_eq!(管理器.日志.len(), 200);
        assert_eq!(管理器.日志.back().map(String::as_str), Some("第249行"));
    }
}
