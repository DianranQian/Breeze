//! 取消任务的副作用核对：真实子进程、真实文件、真实差异。
//!
//! 用一个**受控的假 DSH 垫片**（`.cmd`）模拟"已经开始写文件、还没结束"的任务：
//! 它输出真实的 NDJSON 事件、写一个 `note.txt`、然后长时间挂起。测试在文件
//! 出现后取消进程，再核对：
//!
//! * 进程确实被终止，且没有可信退出码；
//! * 已经写出的文件仍在工作区里（取消 ≠ 回滚）；
//! * 差异能归属到本任务并给出可审查状态。

#![cfg(windows)]

use std::path::Path;
use std::time::{Duration, Instant};

use editor_app::agent_dsh::{任务运行器, 提取写路径, 记录工具调用};
use editor_app::agent_worktree::{刷新变更, 回退变更, 采集基线};
use editor_ui::agent::{变更来源, 审查状态};

fn git(根: &Path, 参数: &[&str]) {
    let 输出 = std::process::Command::new("git")
        .args(参数)
        .current_dir(根)
        .output()
        .expect("需要 git");
    assert!(
        输出.status.success(),
        "git {参数:?} 失败：{}",
        String::from_utf8_lossy(&输出.stderr)
    );
}

#[test]
fn 取消真实进程后文件差异仍可核对() {
    let 工程 = tempfile::tempdir().unwrap();
    let 根 = 工程.path();
    git(根, &["init", "-q"]);
    git(根, &["config", "user.email", "cancel@example.com"]);
    git(根, &["config", "user.name", "cancel"]);
    std::fs::write(根.join("README.md"), "# cancel probe\n").unwrap();
    git(根, &["add", "."]);
    git(根, &["commit", "-qm", "init"]);
    let 基线 = 采集基线(根, "cancel-task").unwrap();

    // 假 DSH 放在项目外：它自己不该出现在项目变更里。
    let 垫片目录 = tempfile::tempdir().unwrap();
    let 垫片 = 垫片目录.path().join("fake_dsh.cmd");
    std::fs::write(
        &垫片,
        "@echo off\r\n\
         echo {\"type\":\"session\",\"sessionId\":\"session-fake-cancel\",\"cwd\":\".\"}\r\n\
         echo {\"type\":\"tool_call\",\"tool\":\"edit\",\"input\":{\"file_path\":\"note.txt\"}}\r\n\
         echo partial side effect> note.txt\r\n\
         ping -n 30 127.0.0.1 >nul\r\n\
         echo {\"type\":\"final\",\"text\":\"should not reach here\"}\r\n",
    )
    .unwrap();

    let mut 运行器 =
        任务运行器::启动(垫片.to_str().unwrap(), "写一个文件然后挂起", 根, None)
            .expect("假 DSH 应当能启动");

    // 等真实文件出现，证明副作用已经发生，再取消。
    let 起点 = Instant::now();
    let mut 事件 = Vec::new();
    let mut 文件出现 = false;
    while 起点.elapsed() < Duration::from_secs(20) {
        for 项 in 运行器.轮询() {
            事件.push(项);
        }
        if 根.join("note.txt").exists() {
            文件出现 = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(文件出现, "假 DSH 没有写出副作用，测试前提不成立");

    运行器.取消();
    std::thread::sleep(Duration::from_millis(300));
    for 项 in 运行器.轮询() {
        事件.push(项);
    }
    assert_eq!(运行器.退出码(), None, "被取消的进程不能有假退出码");

    let 写路径: Vec<String> = 事件
        .iter()
        .filter_map(记录工具调用)
        .filter(|调用| 调用.写操作)
        .filter_map(|调用| 提取写路径(&调用.摘要, Some(根)))
        .collect();
    assert_eq!(写路径, vec!["note.txt".to_string()]);

    let 变更 = 刷新变更(根, Some(&基线), &写路径, "cancel-task", &[]).unwrap();
    let 文件 = 变更
        .iter()
        .find(|项| 项.路径 == "note.txt")
        .expect("取消后必须仍能看到已经产生的文件");
    assert_eq!(文件.来源, 变更来源::本任务);
    assert_eq!(文件.审查, 审查状态::待审查);
    assert!(文件.可回退, "取消后的新文件仍然可以被审查者拒绝");
    assert_eq!(
        std::fs::read_to_string(根.join("note.txt")).unwrap().trim(),
        "partial side effect",
        "取消不等于回滚：已经写出的内容必须还在"
    );

    // 审查者拒绝后，才真正删除这个可归属的新文件。
    let 结果 = 回退变更(根, 文件).unwrap();
    assert_eq!(结果.动作, "删除新增文件");
    assert!(!根.join("note.txt").exists());
}
