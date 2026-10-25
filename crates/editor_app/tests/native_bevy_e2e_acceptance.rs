//! 中文名：原生Bevy工程端到端验收
//! 真实原生 Bevy 工程的**端到端验收**（默认忽略，需显式运行）。
//!
//! ```text
//! cargo test -p editor_app --test native_bevy_e2e_acceptance -- --ignored --nocapture
//! ```
//!
//! # 它验收的是哪条链路
//!
//! ```text
//!  打开真实 Bevy 工程（bevy 依赖、App/ECS、真实 main）
//!    → 识别 Cargo targets（cargo metadata）
//!    → 选择运行入口
//!    → 真实 cargo build（真实退出码）
//!    → 真实启动游戏（真实子进程）
//!    → 捕获真实 stdout
//!    → 进程自然退出（真实退出码 0）
//!    → 再跑一次 --panic 变体：真实 panic 日志 → 统一诊断 → 可跳源码
//! ```
//!
//! # 为什么默认 `#[ignore]`
//!
//! 它会真的编译一个 Bevy 二进制并启动子进程（十几秒到几分钟，取决于
//! 共享 target 目录是否已预热）。让 `cargo test --workspace` 每次都做这件事
//! 会让日常测试变得难以忍受；因此它是一条**显式的验收命令**，
//! 而不是回归测试的一部分。
//!
//! 结果（日志、退出码、结构化证据）写到 `.verify_v4/acceptance/e2e/`。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use editor_app::build::构建宿主;
use editor_app::diagnostics::诊断中心;
use editor_app::run::{运行目标类型, 项目运行管理器};
use editor_core::diagnostics::诊断来源;
use editor_core::project::{项目描述, 项目类型};
use editor_core::run_entry::{结束原因, 运行入口};

fn 夹具目录() -> PathBuf {
    // 测试二进制的工作目录是 crate 根（cargo 的约定）。
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/原生Bevy工程")
}

fn 证据目录() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.verify_v4/acceptance/e2e")
}

fn 写证据(名称: &str, 内容: &str) {
    let 目录 = 证据目录();
    std::fs::create_dir_all(&目录).unwrap();
    std::fs::write(目录.join(名称), 内容).unwrap();
}

fn 等到(超时: Duration, mut 条件: impl FnMut() -> bool) -> bool {
    let 截止 = Instant::now() + 超时;
    while Instant::now() < 截止 {
        if 条件() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    条件()
}

/// JSON 字符串转义（证据文件要机器可读）。
fn 转义(文本: &str) -> String {
    文本
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
        .replace('\t', " ")
}

#[test]
#[ignore = "会真实编译并运行 Bevy 工程：用 --ignored 显式触发"]
fn 原生_bevy工程完成选择构建运行日志停止与诊断的闭环() {
    let 根 = 夹具目录();
    assert!(
        根.join("Cargo.toml").is_file(),
        "验收夹具缺失：{}",
        根.display()
    );

    // ---- 1. 打开工程：识别来源必须是 cargo metadata ----
    let mut 项目 = 项目描述::打开(&根).expect("真实 Bevy 工程必须能打开");
    assert_eq!(项目.类型(), 项目类型::原生Cargo);
    assert_eq!(
        项目.识别来源().map(|来源| 来源.代号()),
        Some("cargo-metadata"),
        "必须优先用 cargo metadata 识别"
    );

    let 工程 = 项目.cargo_工程().expect("应当识别出 Cargo 工程");
    let 目标: Vec<String> = 工程
        .全部可执行目标()
        .iter()
        .map(|目标| 目标.标识())
        .collect();
    assert!(
        目标.contains(&"breeze_native_fixture::breeze_native_fixture".to_string()),
        "必须识别出真实可执行目标：{目标:?}"
    );
    assert!(
        工程.是否贝维工程(),
        "这个工程真的依赖 bevy，识别结果必须如实反映"
    );

    // ---- 2. 选择运行入口 ----
    let 入口 = 运行入口::新建("breeze_native_fixture", "breeze_native_fixture");
    项目.设置运行入口(入口.clone()).expect("设置真实运行入口");
    assert_eq!(项目.运行入口(), Some(&入口));

    // ---- 3. 真实构建 ----
    let 配置 = 项目.清单().构建.clone().unwrap_or_default();
    let mut 构建 = 构建宿主::新建();
    构建
        .启动(&项目.构建根目录(), &配置)
        .expect("启动真实 cargo build");
    let 构建完成 = 等到(Duration::from_secs(1800), || {
        let _ = 构建.轮询();
        !构建.在运行()
    });
    assert!(构建完成, "构建在 30 分钟内没有结束");
    let 报告 = 构建.报告().clone();
    assert_eq!(
        报告.状态,
        editor_core::build::构建状态::成功,
        "真实 Bevy 工程必须构建成功；摘要：{:?}",
        报告.摘要文本()
    );
    assert_eq!(构建.退出码(), Some(0), "构建必须留下真实退出码 0");
    let 产物 = 报告.主产物().cloned().expect("构建必须产出可执行文件");
    assert!(产物.is_file(), "产物必须真实存在：{}", 产物.display());

    let mut 构建日志 = String::new();
    for 行 in &报告.诊断 {
        构建日志.push_str(&format!("[诊断] {}\n", 行.单行文本()));
    }
    构建日志.push_str(&format!(
        "构建成功：退出码 {:?}，产物 {}，用时 {} ms\n",
        构建.退出码(),
        产物.display(),
        报告.耗时毫秒
    ));
    写证据("build.log", &构建日志);

    // ---- 4. 真实启动 → 日志 → 自然退出 ----
    let 命令 = editor_app::run::解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(项目.清单()),
        项目.类型(),
        Some(&入口),
    )
    .expect("必须解析出 cargo run 命令");
    assert_eq!(命令.目标类型, 运行目标类型::Cargo可执行目标);

    let mut 运行 = 项目运行管理器::default();
    let mut 中心 = 诊断中心::default();
    运行.启动(命令).expect("启动真实游戏进程");
    let 运行完成 = 等到(Duration::from_secs(600), || {
        let 序号 = 运行.历史.最近().map(|记录| 记录.序号);
        for 行 in 运行.轮询() {
            中心.采集运行日志(&行, 序号);
        }
        !运行.状态.在运行()
    });
    assert!(运行完成, "游戏进程在 10 分钟内没有结束");

    let 运行日志: Vec<String> = 运行.日志.iter().cloned().collect();
    写证据("run.log", &运行日志.join("\n"));
    assert!(
        运行日志
            .iter()
            .any(|行| 行.contains("fixture-log-ok 完成 5 帧")),
        "必须捕获到游戏自己的真实 stdout：{运行日志:?}"
    );
    assert_eq!(运行.退出码, Some(0), "程序自然退出必须记录真实退出码 0");
    assert_eq!(运行.结束原因, 结束原因::正常退出);
    assert_eq!(中心.快照().错误数, 0, "这次运行没有错误，不该凭空产生诊断");

    // ---- 5. panic 变体：真实运行期错误 → 统一诊断 ----
    let mut 崩溃入口 = 入口.clone();
    崩溃入口.参数 = vec!["--panic".to_string()];
    项目
        .设置运行入口(崩溃入口.clone())
        .expect("带参数的运行入口必须被接受");
    let 崩溃命令 = editor_app::run::解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(项目.清单()),
        项目.类型(),
        Some(&崩溃入口),
    )
    .unwrap();
    assert!(
        崩溃命令.参数.iter().any(|参数| 参数 == "--panic"),
        "程序参数必须放在 `--` 之后传给游戏：{:?}",
        崩溃命令.参数
    );

    let mut 崩溃运行 = 项目运行管理器::default();
    let mut 崩溃诊断 = 诊断中心::default();
    崩溃运行.启动(崩溃命令).expect("启动会崩溃的进程");
    let 崩溃完成 = 等到(Duration::from_secs(600), || {
        let 序号 = 崩溃运行.历史.最近().map(|记录| 记录.序号);
        for 行 in 崩溃运行.轮询() {
            崩溃诊断.采集运行日志(&行, 序号);
        }
        !崩溃运行.状态.在运行()
    });
    assert!(崩溃完成, "崩溃进程在 10 分钟内没有结束");

    let 崩溃日志: Vec<String> = 崩溃运行.日志.iter().cloned().collect();
    写证据("run_panic.log", &崩溃日志.join("\n"));
    let 快照 = 崩溃诊断.快照();
    let panic_条目 = 快照
        .条目
        .iter()
        .find(|条目| 条目.代码.as_deref() == Some("panic"))
        .unwrap_or_else(|| panic!("必须识别出 panic 诊断：{:?}", 快照.条目));
    assert_eq!(panic_条目.来源, 诊断来源::运行);
    assert_eq!(
        panic_条目.级别,
        editor_core::diagnostics::诊断严重级别::错误
    );
    assert!(panic_条目.可跳转(), "panic 必须带真实源码位置");
    assert!(
        panic_条目.消息.contains("fixture-panic-probe"),
        "诊断消息必须是真实 panic 文本：{}",
        panic_条目.消息
    );
    assert_ne!(崩溃运行.退出码, Some(0), "崩溃进程的退出码不能是 0");
    assert_ne!(崩溃运行.结束原因, 结束原因::正常退出);

    // 诊断里的位置必须和磁盘上的真实源码对得上。
    let 文件 = panic_条目.文件.clone().expect("必须有文件");
    let 绝对 = if 文件.is_absolute() {
        文件.clone()
    } else {
        项目.根目录().join(&文件)
    };
    assert!(绝对.is_file(), "诊断指向的文件必须存在：{}", 绝对.display());
    let 行文本 = std::fs::read_to_string(&绝对)
        .unwrap()
        .lines()
        .nth(panic_条目.行.saturating_sub(1) as usize)
        .unwrap_or_default()
        .to_string();
    assert!(
        行文本.contains("panic!"),
        "诊断行号必须落在真实的 panic! 那一行，实际第 {} 行：{行文本}",
        panic_条目.行
    );

    // ---- 6. 用户停止：不是正常退出，也不伪造退出码 ----
    let mut 停止入口 = 入口.clone();
    // `--panic` 会让它 5 帧后崩溃；这里换成让它多跑一会儿的方式：
    // 直接启动 `ping` 更可控，但我们要验证的是**同一个管理器**的停止语义。
    let 停止命令 = editor_app::run::解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(项目.清单()),
        项目.类型(),
        Some(&停止入口),
    )
    .unwrap();
    let _ = &mut 停止入口;
    let mut 停止运行 = 项目运行管理器::default();
    停止运行.启动(停止命令).expect("启动用于停止验证的进程");
    停止运行.停止();
    assert_eq!(停止运行.结束原因, 结束原因::用户停止);
    assert_eq!(停止运行.退出码, None, "被停止的进程不得伪造退出码");
    let 停止记录 = 停止运行.历史.最近().expect("停止也要留下记录");
    assert!(停止记录.结果文本().contains("不是程序自己退出"));

    // ---- 7. 机器可读证据 ----
    let 证据 = format!(
        "{{\n  \"fixture\": \"{}\",\n  \"recognized_via\": \"{}\",\n  \"targets\": [{}],\n  \
         \"entry\": \"{}\",\n  \"build\": {{ \"exit_code\": {}, \"artifact\": \"{}\", \"ms\": {} }},\n  \
         \"run\": {{ \"exit_code\": {}, \"end\": \"{}\", \"log_lines\": {}, \"stdout_ok\": true }},\n  \
         \"panic_run\": {{ \"exit_code\": {}, \"diagnostics\": {}, \"message\": \"{}\", \"file\": \"{}\", \"line\": {}, \"col\": {} }},\n  \
         \"stop\": {{ \"end\": \"{}\", \"exit_code\": null }}\n}}\n",
        转义(&根.display().to_string()),
        "cargo-metadata",
        目标
            .iter()
            .map(|目标| format!("\"{}\"", 转义(目标)))
            .collect::<Vec<_>>()
            .join(", "),
        转义(&入口.标识()),
        构建.退出码().unwrap_or(-1),
        转义(&产物.display().to_string()),
        报告.耗时毫秒,
        运行.退出码.unwrap_or(-1),
        运行.结束原因.代号(),
        运行日志.len(),
        崩溃运行.退出码.unwrap_or(-1),
        快照.错误数,
        转义(&panic_条目.消息),
        转义(&panic_条目.文件.as_ref().unwrap().display().to_string()),
        panic_条目.行,
        panic_条目.列,
        停止运行.结束原因.代号(),
    );
    写证据("acceptance.json", &证据);

    println!("端到端验收通过，证据写入 {}", 证据目录().display());
    println!("{证据}");
}

#[test]
fn 验收夹具是真实走cargo的工程而不是空壳() {
    // 这条**不**忽略：它保证夹具本身没有被"简化"成一个不会用 Bevy 的空壳，
    // 否则上面那条端到端验收就变成了自欺。
    let 根 = 夹具目录();
    let 清单 = std::fs::read_to_string(根.join("Cargo.toml")).expect("夹具必须有 Cargo.toml");
    assert!(清单.contains("bevy"), "夹具必须真的依赖 bevy：{清单}");
    let 源码 = std::fs::read_to_string(根.join("src/main.rs")).expect("夹具必须有 main.rs");
    for 必需 in [
        "use bevy::prelude::*",
        "#[derive(Component)]",
        "#[derive(Resource)]",
        "App::new()",
        ".add_systems(",
        "Commands",
        "Query<",
        "panic!",
    ] {
        assert!(
            源码.contains(必需),
            "夹具必须真的用 Bevy ECS（缺少 `{必需}`）"
        );
    }
}
