//! 项目运行管理：启动/停止/重跑当前项目，并把输出接到控制台。
//!
//! # 当前能力边界（不要误解）
//!
//! 微风目前只有**编辑器内运行时宿主**（统计用的独立 World），没有独立的游戏运行时，
//! 也没有把项目编译成可执行文件的能力。因此这里的"运行项目"是：
//!
//! * 为**当前打开的项目**启动一个独立子进程（默认为微风自身，带上
//!   `BEVY_STUDIO_OPEN_PROJECT` / `BEVY_STUDIO_MODE`，即"另一个进程打开同一项目"）；
//! * 子进程的 stdout/stderr 逐行进入 Console；
//! * 停止 = 结束该进程及其子进程树；
//! * 状态机覆盖：空闲 / 启动中 / 运行中 / 已停止 / 失败。
//!
//! 这解决的是"Run 模式没有真实调用链、按钮空转"的问题，**不等于**已经能构建并运行
//! 独立游戏。真正的游戏运行入口（项目自带 Cargo bin）是后续能力，见工作日志。
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 项目运行状态 {
    /// 没有在运行。
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

impl Default for 项目运行状态 {
    fn default() -> Self {
        Self::空闲
    }
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

    /// 启动命令：拉起子进程并开始读输出。
    ///
    /// 已经在跑时**先停止旧实例**，避免出现不受管理的重复进程。
    pub fn 启动(&mut self, 命令: 运行命令) -> Result<(), String> {
        if self.子进程.is_some() {
            self.停止();
        }
        self.状态 = 项目运行状态::启动中;
        self.最近错误 = None;
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
        self.记日志(format!("已启动（PID {号}）：{}", 命令.显示()));
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
/// 目前没有项目级运行配置，默认是"用微风自身再开一个进程打开该项目"：
/// 这样运行链路是真实的（独立进程、真实日志、可停止），但**不是游戏运行时**。
pub fn 解析运行命令(
    项目根: &Path,
    当前模式: &str,
    可执行文件: &Path,
) -> Result<运行命令, String> {
    if !项目根.is_dir() {
        return Err(format!("项目目录不存在：{}", 项目根.display()));
    }
    if !项目根.join("project.ron").is_file() {
        return Err(format!(
            "不是微风项目（缺少 project.ron）：{}",
            项目根.display()
        ));
    }
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
    })
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

    #[test]
    fn 缺少项目清单时给出明确错误() {
        let 目录 = std::env::temp_dir().join("breeze_run_missing_project");
        let _ = std::fs::create_dir_all(&目录);
        let 错误 = 解析运行命令(&目录, "2d", Path::new("bevy-studio.exe")).unwrap_err();
        assert!(
            错误.contains("project.ron"),
            "错误应指出缺少项目清单：{错误}"
        );
    }

    #[test]
    fn 解析出的命令带项目路径与模式() {
        let 目录 = std::env::temp_dir().join("breeze_run_ok_project");
        let _ = std::fs::create_dir_all(&目录);
        std::fs::write(目录.join("project.ron"), "()").unwrap();
        let 命令 = 解析运行命令(&目录, "3d", Path::new("bevy-studio.exe")).unwrap();
        assert_eq!(命令.工作目录, 目录);
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
