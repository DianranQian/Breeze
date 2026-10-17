//! 最小真实运行时样例。
//!
//! # 它是什么，不是什么
//!
//! 它是**一个真实的独立可执行程序**：有自己的进程、自己的固定步长循环、
//! 自己的 stdout / stderr、自己的退出码。编辑器「运行项目」的声明式运行目标
//! （`project.ron` 的 `运行` 段）可以用它端到端验证：启动 → PID → 日志转发 →
//! 停止 → 重跑 → 错误反馈。
//!
//! 它**不是** Bevy 游戏，也不加载场景、不渲染任何东西。它存在的唯一目的是
//! 给"项目声明的真实可执行程序"这条链路提供一个能跑起来、能被停止、
//! 能被断言的真实对象 —— 而不是拿"再开一个编辑器窗口"冒充游戏运行时。
//!
//! # 用法
//!
//! ```text
//! miniruntime [--project <项目目录>] [--ticks <帧数>] [--watch-stdin] [--fail]
//! ```
//!
//! * `--ticks N`：跑够 N 帧后正常退出（默认 0 = 一直跑）。
//! * `--watch-stdin`：读 stdin，遇到 EOF 或读到 `quit` 就退出。
//!   **默认关闭**：编辑器启动子进程时会继承父进程的 stdin，
//!   而在无人值守的验收环境里那个 stdin 往往已经是 EOF ——
//!   若默认监听，进程会在第一帧就退出，看起来像"运行管理器没能保持进程"。
//! * `--fail`：以非零码退出，用于验证"运行失败"的错误反馈。

use std::io::BufRead;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 一帧的时长（毫秒）。与编辑器界面刷新无关，这里是运行时自己的节拍。
const 帧时长毫秒: u64 = 120;

fn main() {
    let 参数: Vec<String> = std::env::args().skip(1).collect();
    let mut 项目目录: Option<PathBuf> = None;
    let mut 目标帧数: u64 = 0;
    let mut 强制失败 = false;
    let mut 监听stdin = false;

    let mut 下标 = 0;
    while 下标 < 参数.len() {
        match 参数[下标].as_str() {
            "--project" => {
                下标 += 1;
                项目目录 = 参数.get(下标).map(PathBuf::from);
            }
            "--ticks" => {
                下标 += 1;
                目标帧数 = 参数
                    .get(下标)
                    .and_then(|值| 值.trim().parse().ok())
                    .unwrap_or(0);
            }
            "--watch-stdin" => 监听stdin = true,
            "--fail" => 强制失败 = true,
            "--help" | "-h" => {
                println!(
                    "miniruntime [--project <目录>] [--ticks <帧数>] [--watch-stdin] [--fail]"
                );
                return;
            }
            _ => {}
        }
        下标 += 1;
    }

    let 项目根 = 项目目录
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let 项目名 = 读项目名(&项目根).unwrap_or_else(|| "<未知项目>".to_string());

    // stdout：正常运行日志。编辑器会把这些行原样转进控制台。
    println!("miniruntime 启动：项目「{项目名}」");
    println!("项目根：{}", 项目根.display());
    println!("节拍：{帧时长毫秒}ms/帧，进程 PID {}", std::process::id());

    // stderr：故意写一行到标准错误，用来验证编辑器的 stderr 转发链路。
    eprintln!("miniruntime: 这是一条写入 stderr 的诊断行（用于验证错误流转发）");
    let _ = std::io::stderr().flush();

    if 强制失败 {
        eprintln!("miniruntime: --fail 被指定，以非零码退出");
        std::process::exit(3);
    }

    // 可选：读 stdin，遇到 EOF 或 `quit` 就退出。
    //
    // 默认**不**监听：编辑器启动子进程时继承父进程 stdin，
    // 无人值守环境下它往往已经是 EOF，默认监听会让进程第一帧就退出。
    if 监听stdin {
        std::thread::spawn(|| {
            let 输入 = std::io::stdin();
            let mut 行 = String::new();
            loop {
                行.clear();
                match 输入.lock().read_line(&mut 行) {
                    Ok(0) | Err(_) => std::process::exit(0),
                    Ok(_) => {
                        if 行.trim() == "quit" {
                            std::process::exit(0);
                        }
                    }
                }
            }
        });
    }

    let mut 帧: u64 = 0;
    loop {
        帧 += 1;
        println!("[tick {帧}] 运行时推进一帧（{帧时长毫秒}ms）");
        let _ = std::io::stdout().flush();
        if 目标帧数 > 0 && 帧 >= 目标帧数 {
            println!("miniruntime 正常退出：已完成 {帧} 帧");
            return;
        }
        std::thread::sleep(Duration::from_millis(帧时长毫秒));
    }
}

/// 从 `project.ron` 里读项目名。
///
/// 刻意只做最小解析而不引入 serde：这个样例工具要保持零依赖，
/// 这样它在任何机器上都能直接 `cargo build` 出可执行文件。
fn 读项目名(项目根: &Path) -> Option<String> {
    let 文本 = std::fs::read_to_string(项目根.join("project.ron")).ok()?;
    let 起始 = 文本.find("名称")?;
    let 余下 = &文本[起始..];
    let 冒号 = 余下.find(':')?;
    let 余下 = &余下[冒号 + 1..];
    let 引号开始 = 余下.find('"')?;
    let 余下 = &余下[引号开始 + 1..];
    let 引号结束 = 余下.find('"')?;
    Some(余下[..引号结束].to_string())
}
