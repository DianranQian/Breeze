//! 构建进程管理：真正启动 `cargo build`、把输出变成诊断。
//!
//! # 为什么必须是真的子进程
//!
//! 编辑器里"构建"这个按钮的价值是**报出真实的编译错误**。
//! 用一个假的进度条或者调用 `cargo check` 的库接口再拼一段文案，
//! 都无法代替"用户写的 Rust 代码到底能不能编过"这一事实。
//! 因此这里起一个真实子进程，解析它的
//! `--message-format=json-render-diagnostics` 输出。
//!
//! # 线程模型
//!
//! ```text
//!  主线程（Bevy Update）              两个读线程
//!  ─────────────────────             ────────────
//!  轮询：try_recv（非阻塞）  ←──────  stdout 逐行 ─┐
//!                                   stderr 逐行 ─┴→ 同一个 channel
//!  子进程退出 → 收尾、算耗时
//! ```
//!
//! **绝不能在主线程里 `wait()`**：一次 release 构建可能几分钟，
//! 那会让整个编辑器（包括用户正在看的视口）冻结成一个白窗。
//! 读线程 + 非阻塞 `try_recv` 保证编辑器始终可交互，
//! 构建状态由 [`构建宿主::轮询`] 每帧推进一小步。

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use editor_core::build::{
    构建报告, 构建状态, 构建配置, 编译事件, 解析编译事件, 诊断级别
};

/// 子进程输出的一行。
enum 输出行 {
    /// stdout：cargo 的结构化 JSON。
    标准输出(String),
    /// stderr：人读的进度与诊断（编译器的彩色输出）。
    标准错误(String),
}

/// 调试构建允许保留的 stderr 行数（用于"没有任何诊断"时给出失败原因）。
const 标准错误保留行数: usize = 40;

/// 构建任务的宿主资源。
#[derive(bevy::prelude::Resource)]
pub struct 构建宿主 {
    子进程: Option<Child>,
    接收: Option<Receiver<输出行>>,
    读线程: Vec<std::thread::JoinHandle<()>>,
    报告: 构建报告,
    开始时刻: Option<Instant>,
    取消标记: Arc<AtomicBool>,
    /// 保留的 stderr 尾部（失败时当作摘要）。
    stderr尾部: Vec<String>,
    /// cargo 是否报告成功（`build-finished`）。
    cargo报告成功: Option<bool>,
    /// 报告修订号：每次报告内容变化就 +1。
    ///
    /// 界面每帧要读报告。没有修订号就只能每帧克隆整份诊断列表 ——
    /// 几十条带字符串的诊断在 60 FPS 下会变成每秒几 MB 的无谓分配。
    /// 有了它，同步只在真正变化时发生。
    修订: u64,
}

impl Default for 构建宿主 {
    fn default() -> Self {
        Self::新建()
    }
}

impl 构建宿主 {
    /// 新建空闲宿主。
    pub fn 新建() -> Self {
        Self {
            子进程: None,
            接收: None,
            读线程: Vec::new(),
            报告: 构建报告::default(),
            开始时刻: None,
            取消标记: Arc::new(AtomicBool::new(false)),
            stderr尾部: Vec::new(),
            cargo报告成功: None,
            修订: 0,
        }
    }

    /// 报告修订号。界面据此判断是否需要重新读一份快照。
    pub fn 修订(&self) -> u64 {
        self.修订
    }

    /// 标记报告已变化。
    fn 标记报告变化(&mut self) {
        self.修订 = self.修订.wrapping_add(1);
    }

    /// 最近一次构建报告。
    pub fn 报告(&self) -> &构建报告 {
        &self.报告
    }

    /// 是否有构建正在进行。
    pub fn 在运行(&self) -> bool {
        self.报告.状态.在运行()
    }

    /// 启动一次构建。
    ///
    /// 失败（找不到 cargo、项目没有 `Cargo.toml`）**不会** panic：
    /// 它把失败原因写进报告，界面照常显示"构建失败 + 原因"。
    /// 这比弹一个崩溃对话框有用得多。
    pub fn 启动(&mut self, 项目根: &Path, 配置: &构建配置) -> Result<(), String> {
        if self.在运行() {
            return Err("已有构建正在进行".to_string());
        }
        let 清单 = 项目根.join("Cargo.toml");
        if !清单.is_file() {
            let mut 报告 = 构建报告::开始(配置.可读命令行());
            报告.摘要 = Some(format!(
                "项目根目录下没有 Cargo.toml（已查找 {}），无法构建",
                清单.display()
            ));
            报告.完成(false, 0);
            self.报告 = 报告;
            self.标记报告变化();
            return Err(self.报告.摘要.clone().unwrap_or_default());
        }

        let 参数 = 配置.命令行();
        let 报告 = 构建报告::开始(配置.可读命令行());
        self.报告 = 报告;
        self.标记报告变化();
        self.开始时刻 = Some(Instant::now());
        self.stderr尾部.clear();
        self.cargo报告成功 = None;
        self.取消标记 = Arc::new(AtomicBool::new(false));

        let mut 命令 = Command::new("cargo");
        命令
            .args(&参数)
            .current_dir(项目根)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 关掉颜色：ANSI 转义序列进了日志面板只是一堆 `[1m`。
            .env("CARGO_TERM_COLOR", "never")
            .env("CARGO_TERM_PROGRESS_WHEN", "never");

        let mut 子进程 = match 命令.spawn() {
            Ok(子进程) => 子进程,
            Err(错误) => {
                let mut 报告 = self.报告.clone();
                报告.摘要 = Some(format!(
                    "无法启动 cargo：{错误}。请确认 Rust 工具链已安装并在 PATH 上"
                ));
                报告.完成(false, 0);
                self.报告 = 报告;
                self.标记报告变化();
                return Err("无法启动 cargo".to_string());
            }
        };

        let (发送, 接收) = crossbeam_channel::unbounded();
        let mut 线程 = Vec::new();
        if let Some(stdout) = 子进程.stdout.take() {
            let 发送: Sender<输出行> = 发送.clone();
            线程.push(std::thread::spawn(move || {
                读流(stdout, 发送, true);
            }));
        }
        if let Some(stderr) = 子进程.stderr.take() {
            let 发送: Sender<输出行> = 发送.clone();
            线程.push(std::thread::spawn(move || {
                读流(stderr, 发送, false);
            }));
        }
        drop(发送);

        self.子进程 = Some(子进程);
        self.接收 = Some(接收);
        self.读线程 = 线程;
        Ok(())
    }

    /// 停止（取消）当前构建。
    ///
    /// 杀之前先置取消标记：读线程据此停止把后续输出当成诊断，
    /// 否则用户会看到"已取消"的构建又冒出一堆错误。
    pub fn 停止(&mut self) {
        self.取消标记.store(true, Ordering::SeqCst);
        if let Some(mut 子进程) = self.子进程.take() {
            let _ = 子进程.kill();
            let _ = 子进程.wait();
            let 耗时 = self
                .开始时刻
                .take()
                .map(|时刻| 时刻.elapsed().as_millis() as u64)
                .unwrap_or(0);
            self.报告.取消(耗时);
            self.标记报告变化();
        }
        self.接收 = None;
        // 读线程会因为管道关闭而自行退出；join 它们避免线程泄漏。
        for 线程 in self.读线程.drain(..) {
            let _ = 线程.join();
        }
    }

    /// 推进构建状态。返回本次新增的**可读日志行**（供控制台）。
    ///
    /// 每帧调用一次，只做非阻塞读取，绝不会卡住编辑器。
    pub fn 轮询(&mut self) -> Vec<String> {
        let mut 日志 = Vec::new();
        let mut 通道关闭 = false;

        // 先把接收端**克隆**出来再读：`self.接收.as_ref()` 会不可变借用
        // `self`，而循环体里要调用 `&mut self` 的处理函数。
        // `crossbeam_channel::Receiver` 的克隆共享同一个队列，因此克隆不丢消息。
        let 接收 = self.接收.clone();
        if let Some(接收) = 接收 {
            loop {
                match 接收.try_recv() {
                    Ok(输出行::标准输出(行)) => {
                        self.处理标准输出(&行, &mut 日志);
                    }
                    Ok(输出行::标准错误(行)) => {
                        self.处理标准错误(&行, &mut 日志);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        通道关闭 = true;
                        break;
                    }
                }
            }
        } else {
            return 日志;
        }

        if 通道关闭 {
            self.收尾(&mut 日志);
        }
        日志
    }

    /// 处理一行 cargo 的 JSON 输出。
    fn 处理标准输出(&mut self, 行: &str, 日志: &mut Vec<String>) {
        if self.取消标记.load(Ordering::SeqCst) {
            return;
        }
        match 解析编译事件(行) {
            编译事件::诊断(诊断) => {
                let 级别 = 诊断.级别;
                let 文本 = 诊断.单行文本();
                let 位置 = 诊断
                    .位置
                    .as_ref()
                    .and_then(|位置| 位置.短标签())
                    .unwrap_or_default();
                self.报告.追加诊断(诊断);
                self.标记报告变化();
                if 级别 == 诊断级别::错误 || 级别 == 诊断级别::警告 {
                    if 位置.is_empty() {
                        日志.push(文本.clone());
                    } else {
                        日志.push(format!("{位置}: {文本}"));
                    }
                }
            }
            编译事件::产物 { 路径 } => {
                日志.push(format!("产物：{}", 路径.display()));
                self.报告.记录产物(路径);
                self.标记报告变化();
            }
            编译事件::结束 { 成功 } => {
                self.cargo报告成功 = Some(成功);
            }
            编译事件::无关 => {}
        }
    }

    /// 处理一行 stderr（cargo 的进度与编译器人读输出）。
    fn 处理标准错误(&mut self, 行: &str, 日志: &mut Vec<String>) {
        if self.取消标记.load(Ordering::SeqCst) {
            return;
        }
        let 行 = 行.trim_end();
        if 行.is_empty() {
            return;
        }
        // `Compiling`/`Finished` 这类进度行进日志但不进诊断。
        if 行.starts_with("   Compiling")
            || 行.starts_with("    Finished")
            || 行.starts_with("warning: unused")
        {
            日志.push(行.to_string());
        }
        self.stderr尾部.push(行.to_string());
        if self.stderr尾部.len() > 标准错误保留行数 {
            self.stderr尾部.remove(0);
        }
    }

    /// 子进程结束：结算状态与耗时。
    fn 收尾(&mut self, 日志: &mut Vec<String>) {
        let 耗时 = self
            .开始时刻
            .take()
            .map(|时刻| 时刻.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let mut 成功 = self.cargo报告成功.unwrap_or(false);
        if let Some(mut 子进程) = self.子进程.take()
            && let Ok(状态) = 子进程.wait()
        {
            // 退出码是最终裁决：`build-finished` 缺失时也据此判断。
            成功 = 状态.success();
        }
        if self.取消标记.load(Ordering::SeqCst) {
            self.报告.取消(耗时);
            self.标记报告变化();
        } else {
            self.报告.完成(成功, 耗时);
            self.标记报告变化();
            if !成功 && self.报告.诊断.is_empty() && !self.stderr尾部.is_empty() {
                // 没有结构化诊断但有 stderr：把最后几行作为原因。
                // 典型场景是链接器错误、找不到 Cargo.toml 的 workspace 根。
                self.报告.摘要 = Some(self.stderr尾部.join("\n"));
            }
        }
        self.接收 = None;
        for 线程 in self.读线程.drain(..) {
            let _ = 线程.join();
        }
        if self.报告.状态 == 构建状态::成功 {
            日志.push(format!("构建成功，用时 {:.1} 秒", 耗时 as f32 / 1000.0));
        } else if self.报告.状态 == 构建状态::失败 {
            日志.push(format!(
                "构建失败：{} 个错误、{} 个警告",
                self.报告.错误数(),
                self.报告.警告数()
            ));
        }
    }
}

/// 逐行读一个管道并送入 channel。
///
/// 读失败（进程被杀、管道断开）时直接返回：这不是错误，
/// 主线程会通过 `Disconnected` 得知流已结束。
fn 读流<R: std::io::Read>(流: R, 发送: Sender<输出行>, 是标准输出: bool) {
    let 读 = BufReader::new(流);
    for 行 in 读.lines() {
        let Ok(行) = 行 else {
            break;
        };
        let 消息 = if 是标准输出 {
            输出行::标准输出(行)
        } else {
            输出行::标准错误(行)
        };
        if 发送.send(消息).is_err() {
            break;
        }
    }
}

/// 退出前确保子进程被杀掉。
///
/// 不清理的后果很实际：编辑器关掉了，`cargo` 还在后台跑，
/// 用户下次构建时会撞上 `target` 目录的文件锁。
pub fn 退出时停止构建(mut 宿主: bevy::prelude::ResMut<构建宿主>) {
    if 宿主.在运行() {
        宿主.停止();
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 空闲宿主状态正确() {
        let 宿主 = 构建宿主::新建();
        assert!(!宿主.在运行());
        assert_eq!(宿主.报告().状态, 构建状态::空闲);
        assert!(宿主.报告().诊断.is_empty());
    }

    #[test]
    fn 没有清单时报告失败而不是panic() {
        let 目录 = tempfile::tempdir().unwrap();
        let mut 宿主 = 构建宿主::新建();
        let 结果 = 宿主.启动(目录.path(), &构建配置::default());
        assert!(结果.is_err());
        assert!(!宿主.在运行(), "启动失败后不应停留在运行中");
        assert_eq!(宿主.报告().状态, 构建状态::失败);
        assert!(
            宿主
                .报告()
                .摘要文本()
                .unwrap_or_default()
                .contains("Cargo.toml"),
            "摘要应当说明缺少 Cargo.toml"
        );
    }

    #[test]
    fn 停止空闲宿主是安全的() {
        let mut 宿主 = 构建宿主::新建();
        宿主.停止();
        assert!(!宿主.在运行());
    }
}
