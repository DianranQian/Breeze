//! 终止子进程（Windows 上连同进程树）。
//!
//! `Child::kill` 只结束直接子进程。DSH 会再起工具子进程、cargo 会再起
//! `rustc`，只杀父进程可能留下继续写文件的孙进程。用户点"取消"时，
//! 尽可能把整棵树都停掉，比只发一个信号更接近"真的停了"。
//!
//! 这仍然不是保证：进程可能已经脱离父子关系、或被系统保护。因此调用方
//! **仍然必须**把"副作用需核对"如实告诉用户，不能因为杀了进程就声称回滚。

use std::process::Child;

/// 终止子进程；Windows 上使用 `taskkill /T /F` 结束整棵树。
pub fn 终止进程树(子进程: &mut Child) {
    #[cfg(windows)]
    {
        let 进程号 = 子进程.id().to_string();
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &进程号, "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    // 兜底：进程可能已经退出，或 taskkill 不可用。
    let _ = 子进程.kill();
}

#[cfg(test)]
mod 测试 {
    use std::process::{Command, Stdio};

    #[test]
    fn 终止已经退出的进程是安全的() {
        let mut 子进程 = Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                vec!["/C", "exit 0"]
            } else {
                vec![]
            })
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("应当能启动子进程");
        let _ = 子进程.wait();
        // 已经退出后再终止：不应 panic，也不应改变退出状态。
        super::终止进程树(&mut 子进程);
    }
}
