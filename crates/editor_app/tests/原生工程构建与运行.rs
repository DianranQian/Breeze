//! 原生 Cargo 工程的**真实构建与真实运行入口**验收。
//!
//! # 为什么必须真跑 cargo
//!
//! "打开项目后能构建"如果只断言"命令行字符串拼对了"，那么工作目录错了、
//! 包名选错了、cargo 根本不在 PATH 上 —— 这些统统测不出来，而它们恰好是
//! 用户点下「构建」时唯一能感知到的东西。因此这里生成一个真实的（无依赖、
//! 因此无需联网）最小 crate，让 `构建宿主` 真的把它编译出来，再让
//! `项目运行管理器` 通过真实的 `cargo run` 启动它并看到它正常退出。
//!
//! 选无依赖的 crate 是刻意的：测试不该依赖网络与 crates.io 的可用性，
//! 也不该为了验证"构建链路通不通"去编译 Bevy（那要几分钟）。

use std::path::Path;
use std::time::{Duration, Instant};

use editor_app::build::构建宿主;
use editor_app::run::{
    解析运行命令, 运行目标类型, 项目运行状态, 项目运行管理器
};
use editor_core::project::{项目描述, 项目类型};
use editor_core::state::编辑器状态;

fn 写(路径: &Path, 文本: &str) {
    if let Some(父) = 路径.parent() {
        std::fs::create_dir_all(父).unwrap();
    }
    std::fs::write(路径, 文本).unwrap();
}

/// 造一个无依赖的最小可执行 crate。
fn 造可执行工程(根: &Path, 名称: &str, 输出: &str) {
    写(
        &根.join("Cargo.toml"),
        &format!(
            r#"[package]
name = "{名称}"
version = "0.1.0"
edition = "2024"
"#
        ),
    );
    写(
        &根.join("src/main.rs"),
        &format!("fn main() {{\n    println!(\"{输出}\");\n}}\n"),
    );
}

/// 轮询等待某个条件成立（最多等 `超时`）。
fn 等到(超时: Duration, mut 条件: impl FnMut() -> bool) -> bool {
    let 截止 = Instant::now() + 超时;
    while Instant::now() < 截止 {
        if 条件() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    条件()
}

#[test]
fn 原生工程能通过真实cargo构建出可执行产物() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("native_game");
    造可执行工程(&根, "native_game_fixture", "built");

    let 项目 = 项目描述::打开(&根).expect("原生工程应当能打开");
    assert_eq!(项目.类型(), 项目类型::原生Cargo);
    let 配置 = 项目.清单().构建.clone().unwrap_or_default();
    assert_eq!(
        配置.包名.as_deref(),
        Some("native_game_fixture"),
        "构建必须锁定被识别出的那个包"
    );

    let mut 宿主 = 构建宿主::新建();
    // 构建工作目录必须就是项目根（构造参数只有一个来源）。
    宿主
        .启动(项目.构建根目录().as_path(), &配置)
        .expect("启动构建");

    let 结束 = 等到(Duration::from_secs(180), || {
        let _ = 宿主.轮询();
        !宿主.在运行()
    });
    assert!(结束, "构建在 180 秒内没有结束（cargo 可能不可用）");

    let 报告 = 宿主.报告();
    assert!(
        报告.状态 == editor_core::build::构建状态::成功,
        "真实 cargo 构建必须成功，实际状态 {:?}，摘要 {:?}",
        报告.状态,
        报告.摘要文本()
    );
    let 产物 = 报告.主产物().expect("构建成功必须记录产物路径");
    assert!(产物.is_file(), "产物应当真实存在：{}", 产物.display());
}

#[test]
fn 原生工程的运行入口是真实cargo目标() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("native_run");
    造可执行工程(&根, "native_run_fixture", "ran");

    let 项目 = 项目描述::打开(&根).unwrap();
    let 清单 = 项目.清单().clone();
    let 命令 = 解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(&清单),
        项目.类型(),
    )
    .expect("原生工程必须解析出运行命令");

    assert_eq!(
        命令.目标类型,
        运行目标类型::Cargo可执行目标,
        "原生工程的运行目标是 Cargo 目标，不是「再开一个编辑器」"
    );
    assert!(!命令.目标类型.是预览进程());
    assert_eq!(
        命令.工作目录,
        项目.根目录(),
        "运行的工作目录必须与项目根一致"
    );
    assert_eq!(
        命令.参数,
        vec![
            "run",
            "-p",
            "native_run_fixture",
            "--bin",
            "native_run_fixture"
        ]
    );
    assert!(
        命令
            .程序
            .to_string_lossy()
            .to_lowercase()
            .ends_with("cargo.exe")
            || 命令
                .程序
                .to_string_lossy()
                .to_lowercase()
                .ends_with("cargo"),
        "应当解析到真实的 cargo 可执行文件：{}",
        命令.程序.display()
    );
}

#[test]
fn 只有库的原生工程运行入口给出准确诊断() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("native_lib");
    写(
        &根.join("Cargo.toml"),
        r#"[package]
name = "native_lib_fixture"
version = "0.1.0"
edition = "2024"
"#,
    );
    写(&根.join("src/lib.rs"), "pub fn answer() -> u32 { 42 }\n");

    let 项目 = 项目描述::打开(&根).unwrap();
    let 清单 = 项目.清单().clone();
    let 错误 = 解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(&清单),
        项目.类型(),
    )
    .expect_err("只有库的工程不该有运行入口");
    assert!(
        错误.contains("可执行目标"),
        "诊断必须说明缺少可执行目标，而不是含糊地失败：{错误}"
    );
}

#[test]
fn 编辑器状态里打开的原生工程能直接跑完整链路() {
    // 这条把"打开 → 取构建配置 → 取运行命令"串起来，防止三处各自正确、
    // 组合起来工作目录或包名对不上。
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("e2e");
    造可执行工程(&根, "e2e_fixture", "ok");

    let mut 状态 = 编辑器状态::新建();
    状态.打开项目(&根).unwrap();
    let 项目 = 状态.项目.as_ref().unwrap();
    assert_eq!(项目.构建根目录(), 根, "构建根目录必须等于被打开的根");
    let 配置 = 项目.清单().构建.clone().unwrap_or_default();
    assert_eq!(配置.包名.as_deref(), Some("e2e_fixture"));

    let 命令 = 解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(项目.清单()),
        项目.类型(),
    )
    .unwrap();
    let mut 管理器 = 项目运行管理器::default();
    管理器.启动(命令).expect("启动运行进程");

    let 结束 = 等到(Duration::from_secs(180), || {
        let _ = 管理器.轮询();
        !管理器.状态.在运行()
    });
    assert!(结束, "运行进程在 180 秒内没有结束");
    assert_eq!(
        管理器.状态,
        项目运行状态::已停止,
        "程序应当正常退出（日志：{:?}）",
        管理器.日志
    );
}
