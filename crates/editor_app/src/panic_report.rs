// SPDX-License-Identifier: MIT OR Apache-2.0
//! 崩溃报告：panic 只写文件与 stderr，**绝不弹原生窗口**。
//!
//! # 为什么要自己装钩子
//!
//! 编辑器是长驻进程，用户手里的工程可能几小时没保存过。panic 直接退出
//! 除了丢掉窗口，还让用户拿不到任何现场。这里统一做三件事：
//!
//! 1. 把 panic 说明、源码位置、回溯写进配置目录下的报告文件；
//! 2. 往 stderr 打一行"报告在哪"，终端与日志都接得住；
//! 3. 交回默认钩子，保留 Rust 原有的输出 —— 不吞掉任何东西。
//!
//! # 为什么不弹窗
//!
//! 原生消息框会阻塞在崩溃的那条线程上，用户的第一个反应是点掉它，
//! 现场就此丢失；而且 `rfd` 的对话框要靠主线程泵消息，在崩溃时刻并不可靠。
//! 报告文件可以重启后再看，比弹一个"未知错误"有用得多。
//!
//! 报告正文用英文：它要能贴进 issue、被 grep，不该跟着界面语言变。
//!
//! # 钩子本身不许再 panic
//!
//! 崩溃处理里任何 `unwrap` 都会把一次 panic 变成"递归 panic → 进程立刻死亡"。
//! 因此这里所有 IO 失败都只降级成一行 stderr，绝不向上传播。

use std::io::Write;
use std::path::{Path, PathBuf};

/// 崩溃报告目录：配置目录下的 `crash-reports`。
pub fn 崩溃报告目录() -> PathBuf {
    crate::config::配置目录().join("crash-reports")
}

/// 生成报告正文（与写盘分开，便于测试逐行断言）。
///
/// 三段结构固定：说明 → 位置 → 回溯。位置缺失时写 `(unknown location)`，
/// 而不是省略这一节 —— 用户贴报告时不用再解释"这里为什么没有行号"。
pub fn 报告文本(说明: &str, 位置: &str, 回溯: &str) -> String {
    format!(
        "Breeze crash report\n\
         ===================\n\
         message:\n{说明}\n\n\
         location:\n{位置}\n\n\
         backtrace:\n{回溯}\n"
    )
}

/// 把一次 panic 写成报告文件，返回报告路径。
///
/// 目录不存在会自动建齐；同一毫秒内重复写会加 `-1`、`-2` 后缀，
/// 绝不覆盖上一份报告 —— 两次崩溃往往是一串因果，后一份不能踩掉前一份。
pub fn 写崩溃报告(
    目录: &Path,
    说明: &str,
    位置: &str,
    回溯: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(目录)?;
    let 毫秒 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_millis())
        .unwrap_or(0);
    let mut 路径 = 目录.join(format!("crash-{毫秒}.txt"));
    let mut 序号 = 1;
    while 路径.exists() {
        路径 = 目录.join(format!("crash-{毫秒}-{序号}.txt"));
        序号 += 1;
    }
    let mut 文件 = std::fs::File::create(&路径)?;
    文件.write_all(报告文本(说明, 位置, 回溯).as_bytes())?;
    文件.flush()?;
    Ok(路径)
}

/// 安装进程级崩溃钩子。
///
/// 重复调用只生效第一次（`Once`）：多个测试或插件重复初始化时，
/// 不会产生"钩子套钩子 → 同一份报告写两遍"。
///
/// 钩子做的事：读 panic 载荷 → 取源码位置 → 强制捕获回溯 → 写文件 →
/// stderr 打一行 → 调用安装前的钩子（通常是 Rust 默认钩子）。
pub fn 安装崩溃钩子() {
    static 已安装: std::sync::Once = std::sync::Once::new();
    已安装.call_once(|| {
        let 原钩子 = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |信息| {
            let 说明 = match 信息.payload().downcast_ref::<&str>() {
                Some(文本) => (*文本).to_string(),
                None => match 信息.payload().downcast_ref::<String>() {
                    Some(文本) => 文本.clone(),
                    None => "(panic payload is not a string)".to_string(),
                },
            };
            let 位置 = 信息
                .location()
                .map(|位置| 位置.to_string())
                .unwrap_or_else(|| "(unknown location)".to_string());
            let 回溯 = std::backtrace::Backtrace::force_capture().to_string();
            match 写崩溃报告(&崩溃报告目录(), &说明, &位置, &回溯) {
                Ok(路径) => eprintln!("[crash] report written to {}", 路径.display()),
                Err(错误) => eprintln!("[crash] could not write report: {错误}"),
            }
            原钩子(信息);
        }));
    });
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 报告正文按说明位置回溯三段排列() {
        let 文本 = 报告文本("索引越界", "src/main.rs:12:3", "0: backtrace");
        let 说明位置 = 文本.find("索引越界").expect("说明");
        let 位置位置 = 文本.find("src/main.rs:12:3").expect("位置");
        let 回溯位置 = 文本.find("0: backtrace").expect("回溯");
        assert!(说明位置 < 位置位置 && 位置位置 < 回溯位置);
    }

    #[test]
    fn 写崩溃报告会建齐目录且不覆盖上一份() {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 目标 = 目录.path().join("多层/报告");
        let 第一份 = 写崩溃报告(&目标, "第一次", "a.rs:1:1", "bt1").expect("第一份");
        let 第二份 = 写崩溃报告(&目标, "第二次", "a.rs:2:2", "bt2").expect("第二份");
        assert_ne!(第一份, 第二份, "同一毫秒内的两份报告不能互相覆盖");
        let 第一份文本 = std::fs::read_to_string(&第一份).expect("读回第一份");
        assert!(第一份文本.contains("第一次"), "第一份内容必须原样保留");
        let 第二份文本 = std::fs::read_to_string(&第二份).expect("读回第二份");
        assert!(第二份文本.contains("a.rs:2:2"));
    }
}
