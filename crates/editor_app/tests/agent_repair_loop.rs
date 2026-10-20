//! 构建失败后的自动修复闭环：真实 `cargo build`、真实退出码、真实诊断。
//!
//! # 这里"Agent"是受控替身，不是真实 DSH
//!
//! 本机在 CI/测试进程里不保证有可用的 DSH 模型配置，因此这些用例验证的是
//! **闭环的构建与验证部分**：真实失败 → 真诊断 → 应用修复 → 真实成功。
//! DSH 进程本身的事件解析在 `agent_dsh` 的单测里用真实抓取的 NDJSON 覆盖；
//! 真实 DSH 的端到端验收在交付报告里单独区分，绝不把本文件当成"真实模型跑通"。

use std::path::Path;
use std::time::{Duration, Instant};

use editor_app::build::构建宿主;
use editor_core::build::{构建状态, 构建配置};
use editor_ui::agent_repair::{
    下一步, 修复循环, 修复请求, 修复阶段, 分类错误, 构造修复提示, 错误类别, 验证方式,
};

fn 建工程(根: &Path, 名称: &str, 源码: &str) {
    std::fs::create_dir_all(根.join("src")).unwrap();
    std::fs::write(
        根.join("Cargo.toml"),
        format!("[package]\nname = \"{名称}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    std::fs::write(根.join("src/main.rs"), 源码).unwrap();
}

fn 等待构建(宿主: &mut 构建宿主) -> Option<i32> {
    let 起点 = Instant::now();
    loop {
        let _ = 宿主.轮询();
        if !宿主.在运行() {
            return 宿主.退出码();
        }
        assert!(
            起点.elapsed() < Duration::from_secs(240),
            "真实构建超时（240 秒）"
        );
        std::thread::sleep(Duration::from_millis(30));
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
    if 结果.trim().is_empty() {
        结果 = 宿主.标准错误尾部();
    }
    结果
}

#[test]
fn 真实编译错误修复后重新构建成功() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    建工程(
        根,
        "repair_probe",
        "fn main() {\n    let 值: u32 = \"这不是数字\";\n    println!(\"{值}\");\n}\n",
    );

    let mut 构建 = 构建宿主::新建();
    构建.启动(根, &构建配置::default()).expect("构建应当能启动");
    let 退出码 = 等待构建(&mut 构建);
    assert_eq!(退出码, Some(101), "编译失败必须留下真实非零退出码");
    assert_eq!(构建.报告().状态, 构建状态::失败);
    let 诊断 = 诊断文本(&构建);
    assert!(
        诊断.contains("E0308") || 诊断.contains("mismatched types"),
        "应当拿到结构化编译诊断：{诊断}"
    );
    assert_eq!(
        分类错误(&诊断, 退出码, false),
        错误类别::编译错误,
        "必须是代码错误，而不是环境问题"
    );

    // 自动修复状态机：首次失败 → 第 1 轮 Agent。
    let mut 循环 = 修复循环::default();
    循环.开始(&修复请求::新建("构建并修复", 验证方式::构建, 1));
    let 决策 = 循环.首次验证结束(&诊断, 退出码, vec!["src/main.rs".to_string()]);
    assert_eq!(决策, 下一步::启动Agent(1));
    assert_eq!(循环.阶段, 修复阶段::等待Agent);
    let 提示 = 构造修复提示("构建并修复", 验证方式::构建, &诊断, 1, 1);
    assert!(提示.contains("E0308") || 提示.contains("mismatched types"));

    // 受控替身执行"Agent 的修改"。
    建工程(
        根,
        "repair_probe",
        "fn main() {\n    let 值: u32 = 42;\n    println!(\"{值}\");\n}\n",
    );
    循环.智能体完成("把字符串改成 42");
    构建
        .启动(根, &构建配置::default())
        .expect("第二轮构建应当能启动");
    let 第二次退出码 = 等待构建(&mut 构建);
    assert_eq!(第二次退出码, Some(0), "修复后必须真实构建成功");
    let 第二次诊断 = 诊断文本(&构建);
    let 决策 = 循环.后续验证结束(&第二次诊断, 第二次退出码, vec!["src/main.rs".to_string()]);
    assert_eq!(决策, 下一步::完成);
    assert_eq!(循环.阶段, 修复阶段::已成功);
    assert_eq!(循环.记录.len(), 2, "每轮都要留下真实验证记录");
    assert_eq!(循环.记录[0].退出码, Some(101));
    assert_eq!(循环.记录[1].退出码, Some(0));
    assert!(循环.记录[1].成功);
}

#[test]
fn 环境问题不会被当成代码错误重试() {
    // 真实构建宿主在缺少 Cargo.toml 时的摘要（不是伪造文本）。
    let 目录 = tempfile::tempdir().unwrap();
    let mut 构建 = 构建宿主::新建();
    let 错误 = 构建
        .启动(目录.path(), &构建配置::default())
        .expect_err("没有 Cargo.toml 必须启动失败");
    assert!(错误.contains("Cargo.toml"));
    assert_eq!(构建.退出码(), None, "没有进程就没有退出码");

    let 类别 = 分类错误(&错误, None, false);
    assert!(
        类别.是环境问题(),
        "缺少清单必须是环境问题，不能误判成代码错误：{类别:?}"
    );
}

#[test]
fn 取消真实构建后没有可信退出码() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    建工程(根, "cancel_probe", "fn main() {}\n");
    // 构建脚本故意睡很久：让"停止"发生在构建真正结束之前。
    std::fs::write(
        根.join("build.rs"),
        "fn main() {\n    std::thread::sleep(std::time::Duration::from_secs(60));\n}\n",
    )
    .unwrap();

    let mut 构建 = 构建宿主::新建();
    构建.启动(根, &构建配置::default()).unwrap();
    let 起点 = Instant::now();
    while !构建.在运行() && 起点.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    构建.停止();
    assert!(!构建.在运行());
    assert_eq!(构建.退出码(), None, "被杀的子进程不能给出假退出码");
    assert_eq!(
        构建.报告().状态,
        构建状态::已取消,
        "取消必须如实标记为已取消"
    );
    // 取消后可以再次构建（不会留下锁或残留进程）。
    构建
        .启动(根, &构建配置::default())
        .expect("取消后应能重新启动构建");
    构建.停止();
}
