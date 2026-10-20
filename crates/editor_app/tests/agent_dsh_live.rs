//! 真实 DSH 集成验收（默认 `#[ignore]`）。
//!
//! 普通 `cargo test` 不会调用模型；需要真实 DSH 模型配置时手动运行：
//!
//! ```text
//! cargo test -p editor_app --test agent_dsh_live -- --ignored --nocapture
//! ```
//!
//! 验证链路：真实编译错误 → 真实 DSH headless 修复 → 真实差异/归属 →
//! 再次真实构建成功。它与 `agent_repair_loop` 的受控替身测试**不是**同一件事：
//! 这里跑的是本机真实的 `dsh headless --json`。

use std::path::Path;
use std::time::{Duration, Instant};

use editor_app::agent_dsh::{
    Dsh事件, 任务运行器, 判定状态, 提取写路径, 记录工具调用
};
use editor_app::agent_worktree::{刷新变更, 采集基线};
use editor_app::build::构建宿主;
use editor_core::build::{构建状态, 构建配置};
use editor_ui::agent::{任务状态, 变更来源};
use editor_ui::agent_repair::{分类错误, 错误类别};

fn git(根: &Path, 参数: &[&str]) {
    let 输出 = std::process::Command::new("git")
        .args(参数)
        .current_dir(根)
        .output()
        .expect("需要 git");
    assert!(
        输出.status.success(),
        "git {:?} 失败：{}",
        参数,
        String::from_utf8_lossy(&输出.stderr)
    );
}

fn 等待构建(宿主: &mut 构建宿主) -> Option<i32> {
    let 起点 = Instant::now();
    loop {
        let _ = 宿主.轮询();
        if !宿主.在运行() {
            return 宿主.退出码();
        }
        assert!(起点.elapsed() < Duration::from_secs(240), "构建超时");
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn 诊断文本(宿主: &构建宿主) -> String {
    let mut 结果 = String::new();
    for 诊断 in &宿主.报告().诊断 {
        if matches!(诊断.级别, editor_core::build::诊断级别::错误) {
            结果.push_str(&诊断.单行文本());
            结果.push('\n');
        }
    }
    结果
}

#[test]
#[ignore = "需要真实 DSH 模型配置；用 --ignored 手动运行"]
fn 真实命令行修复编译错误并重新构建成功() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    std::fs::create_dir_all(根.join("src")).unwrap();
    std::fs::write(
        根.join("Cargo.toml"),
        "[package]\nname = \"dsh_live_probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        根.join("src/main.rs"),
        "fn main() {\n    let 数量: u32 = \"不是数字\";\n    println!(\"{数量}\");\n}\n",
    )
    .unwrap();
    git(根, &["init", "-q"]);
    git(根, &["config", "user.email", "live@example.com"]);
    git(根, &["config", "user.name", "live"]);
    git(根, &["add", "."]);
    git(根, &["commit", "-qm", "broken"]);

    let 基线 = 采集基线(根, "live-1").expect("采集基线");

    let mut 构建 = 构建宿主::新建();
    构建.启动(根, &构建配置::default()).unwrap();
    let 退出码 = 等待构建(&mut 构建);
    assert_eq!(退出码, Some(101));
    let 诊断 = 诊断文本(&构建);
    assert!(诊断.contains("E0308"), "真实诊断：{诊断}");
    assert_eq!(分类错误(&诊断, 退出码, false), 错误类别::编译错误);

    let 指令 = format!(
        "这个 Rust 项目构建失败。请只修改 src/main.rs 修复它，不要动其他文件，\
         不要使用 git reset/clean/restore。诊断如下：\n{诊断}"
    );
    let mut 运行器 = 任务运行器::启动("dsh", &指令, 根, None).expect("真实 DSH 应能启动");
    let 起点 = Instant::now();
    let mut 事件 = Vec::new();
    let mut 收到最终 = false;
    let 退出码 = loop {
        for 项 in 运行器.轮询() {
            if matches!(项, Dsh事件::最终 { .. }) {
                收到最终 = true;
            }
            事件.push(项);
        }
        if !运行器.运行中() {
            break 运行器.退出码();
        }
        assert!(
            起点.elapsed() < Duration::from_secs(420),
            "真实 DSH 任务超时（420 秒）"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    for 项 in 运行器.轮询() {
        事件.push(项);
    }
    assert_eq!(
        判定状态(收到最终, 退出码),
        任务状态::成功,
        "真实 DSH 任务必须成功结束（退出码 {退出码:?}）"
    );

    let 写路径: Vec<String> = 事件
        .iter()
        .filter_map(记录工具调用)
        .filter(|调用| 调用.写操作)
        .filter_map(|调用| 提取写路径(&调用.摘要, Some(根)))
        .collect();
    eprintln!("真实 DSH 写路径：{写路径:?}");

    let 变更 = 刷新变更(根, Some(&基线), &写路径, "live-1-round-1", &[]).unwrap();
    let 主文件 = 变更
        .iter()
        .find(|项| 项.路径 == "src/main.rs")
        .expect("必须看到 main.rs 的变更");
    assert_eq!(
        主文件.来源,
        变更来源::本任务,
        "真实 DSH 的 tool_call 路径必须让改动归属到本任务；写路径：{写路径:?}"
    );
    assert!(
        !主文件.差异.is_empty(),
        "审查必须拿到真实的文件差异：{:?}",
        主文件.差异
    );

    构建.启动(根, &构建配置::default()).unwrap();
    let 第二次退出码 = 等待构建(&mut 构建);
    assert_eq!(
        第二次退出码,
        Some(0),
        "真实 DSH 修复后必须真实构建成功；报告：{:?}",
        构建.报告().状态
    );
    assert_eq!(构建.报告().状态, 构建状态::成功);
}
