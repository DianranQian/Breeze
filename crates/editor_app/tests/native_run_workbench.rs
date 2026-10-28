//! 中文名：原生工程运行工作台
//! 原生 Cargo 工程「开发工作台」的端到端验收：入口选择 → 构建 → 运行 →
//! 日志/诊断 → 跳源码 → 交给 Agent。
//!
//! # 为什么这些用例必须跑真实 cargo
//!
//! "构建失败时能列出错误"这种能力，只有真的让 `rustc` 报一次错才能验证：
//! 手写一段假 JSON 只能证明解析器认得那段 JSON，证明不了
//! `--message-format=json` 的输出、工作目录、包选择、退出码这一整条链路。
//! 因此这里每个用例都在临时目录里生成真实的最小 crate（无第三方依赖，
//! 所以不需要联网），跑完就删。

use std::path::Path;
use std::time::{Duration, Instant};

use editor_app::build::构建宿主;
use editor_app::diagnostics::诊断中心;
use editor_app::run::{运行目标类型, 项目运行管理器};
use editor_core::diagnostics::{任务类别, 诊断严重级别, 诊断来源};
use editor_core::project::项目描述;
use editor_core::run_entry::运行入口;
use editor_core::state::编辑器状态;

fn 写(路径: &Path, 文本: &str) {
    if let Some(父) = 路径.parent() {
        std::fs::create_dir_all(父).unwrap();
    }
    std::fs::write(路径, 文本).unwrap();
}

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

/// 造一个多 target 的 crate：`src/main.rs` + `src/bin/server.rs` + 一个库。
fn 造多目标工程(根: &Path) {
    写(
        &根.join("Cargo.toml"),
        r#"[package]
name = "workbench_multi"
version = "0.1.0"
edition = "2024"

[lib]
name = "workbench_multi_lib"
path = "src/lib.rs"
"#,
    );
    写(&根.join("src/lib.rs"), "pub fn shared() -> u32 { 7 }\n");
    写(
        &根.join("src/main.rs"),
        "fn main() {\n    println!(\"main-ok\");\n}\n",
    );
    写(
        &根.join("src/bin/server.rs"),
        "fn main() {\n    println!(\"server-ok\");\n}\n",
    );
}

/// 造一个工作区：库成员 + 两个游戏成员。
fn 造工作区(根: &Path) {
    写(
        &根.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"engine\", \"game_a\", \"game_b\"]\n",
    );
    写(
        &根.join("engine/Cargo.toml"),
        "[package]\nname = \"wb_engine\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    写(&根.join("engine/src/lib.rs"), "pub fn tick() {}\n");
    for (目录, 名称) in [("game_a", "wb_game_a"), ("game_b", "wb_game_b")] {
        写(
            &根.join(format!("{目录}/Cargo.toml")),
            &format!("[package]\nname = \"{名称}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        );
        写(&根.join(format!("{目录}/src/main.rs")), "fn main() {}\n");
    }
}

#[test]
fn 多binary工程能列出并切换所有运行入口() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("multi");
    造多目标工程(&根);

    let mut 项目 = 项目描述::打开(&根).expect("多目标工程必须能打开");
    let 工程 = 项目.cargo_工程().expect("应当识别出 Cargo 工程");
    let 全部: Vec<String> = 工程
        .全部可执行目标()
        .iter()
        .map(|目标| 目标.标识())
        .collect();
    assert!(
        全部.contains(&"workbench_multi::workbench_multi".to_string()),
        "主程序必须在列表里：{全部:?}"
    );
    assert!(
        全部.contains(&"workbench_multi::server".to_string()),
        "`src/bin/server.rs` 必须被识别成独立目标：{全部:?}"
    );
    // 同时有库与二进制时，库名也要被识别出来（库不能运行，但要知道它在）。
    let 包 = 工程
        .包
        .iter()
        .find(|包| 包.名称 == "workbench_multi")
        .unwrap();
    assert_eq!(包.库名.as_deref(), Some("workbench_multi_lib"));

    // 默认入口是主程序。
    assert_eq!(项目.运行入口().unwrap().目标名, "workbench_multi");

    // 切换到 server：真实生效，并且构建配置同步切过去（构建与运行必须同包同目标）。
    项目
        .设置运行入口(运行入口::新建("workbench_multi", "server"))
        .expect("server 是真实存在的目标");
    assert_eq!(项目.运行入口().unwrap().目标名, "server");
    assert_eq!(
        项目.清单().构建.as_ref().unwrap().二进制名.as_deref(),
        Some("server")
    );
    assert_eq!(
        项目.清单().运行.as_ref().unwrap().参数,
        vec!["run", "-p", "workbench_multi", "--bin", "server"]
    );
}

#[test]
fn 工作区能选中任一游戏成员并且失效入口被拒绝() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("ws");
    造工作区(&根);

    let mut 项目 = 项目描述::打开(&根).expect("工作区必须能打开");
    let 工程 = 项目.cargo_工程().unwrap();
    assert!(工程.是工作区);
    assert_eq!(工程.包.len(), 3, "三个成员都要被识别");

    // 切到另一个成员：包名与目标名必须一起变。
    项目
        .设置运行入口(运行入口::新建("wb_game_b", "wb_game_b"))
        .expect("wb_game_b 是真实目标");
    assert_eq!(项目.清单().运行.as_ref().unwrap().参数[2], "wb_game_b");
    assert_eq!(
        项目.清单().构建.as_ref().unwrap().包名.as_deref(),
        Some("wb_game_b"),
        "构建配置必须跟着运行入口走，否则会构建 A 却运行 B"
    );

    // 纯库成员不能作为运行入口。
    let 错误 = 项目
        .设置运行入口(运行入口::新建("wb_engine", "wb_engine"))
        .expect_err("库成员不能作为运行入口");
    assert!(
        format!("{错误}").contains("只有库目标"),
        "错误必须说明该包没有可执行目标：{错误}"
    );

    // 不存在的目标也必须被拒绝（不静默替换成默认目标）。
    let 错误 = 项目
        .设置运行入口(运行入口::新建("wb_game_a", "does_not_exist"))
        .expect_err("不存在的目标不能被接受");
    assert!(format!("{错误}").contains("does_not_exist"), "{错误}");
    // 拒绝之后，原来选中的入口不能被改掉。
    assert_eq!(项目.运行入口().unwrap().包名, "wb_game_b");
}

#[test]
fn 构建失败会产出可跳转的真实诊断并归属构建任务() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("broken");
    写(
        &根.join("Cargo.toml"),
        "[package]\nname = \"workbench_broken\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    // 真实的类型错误：`let x: i32 = "str";`
    写(
        &根.join("src/main.rs"),
        "fn main() {\n    let _x: i32 = \"这不是整数\";\n}\n",
    );

    let 项目 = 项目描述::打开(&根).unwrap();
    let 配置 = 项目.清单().构建.clone().unwrap_or_default();
    let mut 宿主 = 构建宿主::新建();
    宿主.启动(&项目.构建根目录(), &配置).expect("启动构建");

    let mut 中心 = 诊断中心::default();
    let 完成 = 等到(Duration::from_secs(180), || {
        let _ = 宿主.轮询();
        中心.采集构建(宿主.报告(), Some(&配置), 宿主.修订());
        !宿主.在运行()
    });
    assert!(完成, "构建在 180 秒内没有结束（cargo 可能不可用）");
    assert_eq!(
        宿主.报告().状态,
        editor_core::build::构建状态::失败,
        "这段代码必须编译失败"
    );
    assert_eq!(宿主.退出码(), Some(101), "cargo 编译失败的退出码是 101");

    let 快照 = 中心.快照();
    assert!(快照.错误数 > 0, "构建失败必须产出错误诊断");
    let 条目 = 快照
        .条目
        .iter()
        .find(|条目| 条目.级别 == 诊断严重级别::错误)
        .expect("至少有一条错误");
    assert_eq!(条目.来源, 诊断来源::构建);
    assert_eq!(条目.任务.unwrap().类别, 任务类别::构建);
    assert_eq!(条目.代码.as_deref(), Some("E0308"), "应当是类型不匹配");
    assert!(条目.可跳转(), "编译错误必须带源码位置");
    assert!(
        条目
            .文件
            .as_ref()
            .map(|文件| 文件.ends_with("src/main.rs") || 文件.ends_with("src\\main.rs"))
            .unwrap_or(false),
        "诊断应当指向 src/main.rs，实际：{:?}",
        条目.文件
    );
    assert_eq!(条目.行, 2);

    // 构建历史也留下了真实记录。
    assert_eq!(快照.构建历史.len(), 1);
    assert!(!快照.构建历史[0].成功);
    assert!(快照.构建历史[0].错误数 > 0);
}

#[test]
fn 诊断可以跳转到真实源码位置() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("jump");
    写(
        &根.join("Cargo.toml"),
        "[package]\nname = \"workbench_jump\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    写(
        &根.join("src/main.rs"),
        "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = a + b;\n}\n",
    );

    let mut 编辑器 = 编辑器状态::新建();
    编辑器.打开项目(&根).unwrap();
    let mut 界面 = editor_ui::界面状态::default();

    // 相对路径（rustc 对项目内文件给相对路径）必须相对**项目根**解析。
    editor_app::actions::打开源码位置(
        &mut 编辑器,
        &mut 界面,
        Some(Path::new("src/main.rs")),
        3,
    );

    let 标签 = 界面.脚本.当前标签().expect("应当打开脚本面板");
    assert!(
        标签.文档.文本.contains("let b = 2"),
        "打开的内容必须是真实源码：{}",
        标签.文档.文本
    );
    assert_eq!(标签.滚动到行, Some(2), "应当滚到第 3 行（0 基下标 2）");
}

#[test]
fn 缺少源文件的诊断给出可读反馈而不是静默失败() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("missing");
    写(
        &根.join("Cargo.toml"),
        "[package]\nname = \"wb_missing\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    写(&根.join("src/main.rs"), "fn main() {}\n");

    let mut 编辑器 = 编辑器状态::新建();
    编辑器.打开项目(&根).unwrap();
    let mut 界面 = editor_ui::界面状态::default();
    let 之前 = 编辑器.日志列表().len();

    editor_app::actions::打开源码位置(
        &mut 编辑器,
        &mut 界面,
        Some(Path::new("src/根本不存在.rs")),
        10,
    );
    let 新日志: Vec<String> = 编辑器
        .日志列表()
        .iter()
        .skip(之前)
        .map(|条目| 条目.渲染(editor_ui::i18n::语言::中文))
        .collect();
    assert!(
        新日志
            .iter()
            .any(|行| 行.contains("无法打开") && 行.contains("根本不存在")),
        "必须明确告知打不开哪个文件：{新日志:?}"
    );
    assert!(
        界面.脚本.标签.is_empty(),
        "打不开时不该打开一个空脚本"
    );
}

#[test]
fn 真实cargo运行会记录退出码与运行入口() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("run_ok");
    写(
        &根.join("Cargo.toml"),
        "[package]\nname = \"workbench_run_ok\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    写(
        &根.join("src/main.rs"),
        "fn main() {\n    println!(\"workbench-run-ok\");\n    std::process::exit(3);\n}\n",
    );

    let 项目 = 项目描述::打开(&根).unwrap();
    let 入口 = 项目.运行入口().expect("应当有运行入口").clone();
    let 命令 = editor_app::run::解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(项目.清单()),
        项目.类型(),
        Some(&入口),
    )
    .expect("应当解析出 cargo 运行命令");
    assert_eq!(命令.目标类型, 运行目标类型::Cargo可执行目标);

    let mut 管理器 = 项目运行管理器::default();
    管理器.启动(命令).expect("启动真实 cargo run");
    let 完成 = 等到(Duration::from_secs(300), || {
        let _ = 管理器.轮询();
        !管理器.状态.在运行()
    });
    assert!(完成, "cargo run 在 300 秒内没有结束");

    assert!(
        管理器.日志.iter().any(|行| 行.contains("workbench-run-ok")),
        "必须捕获到游戏程序的真实 stdout：{:?}",
        管理器.日志
    );
    // 程序自己 `exit(3)`：既不是 0，也不是"用户停止"。
    assert_eq!(管理器.退出码, Some(3), "必须拿到真实退出码 3");
    assert_eq!(管理器.结束原因, editor_core::run_entry::结束原因::异常退出);
    let 记录 = 管理器.历史.最近().unwrap();
    assert_eq!(
        记录.入口.as_ref().map(|入口| 入口.标识()),
        Some(入口.标识())
    );
    assert!(记录.结束时间.is_some());
}

#[test]
fn 运行日志里的panic进入统一诊断() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("panic");
    写(
        &根.join("Cargo.toml"),
        "[package]\nname = \"workbench_panic\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    写(
        &根.join("src/main.rs"),
        "fn main() {\n    panic!(\"workbench-panic-probe\");\n}\n",
    );

    let 项目 = 项目描述::打开(&根).unwrap();
    let 入口 = 项目.运行入口().unwrap().clone();
    let 命令 = editor_app::run::解析运行命令(
        &项目.构建根目录(),
        "2d",
        Path::new("bevy-studio.exe"),
        Some(项目.清单()),
        项目.类型(),
        Some(&入口),
    )
    .unwrap();
    let mut 管理器 = 项目运行管理器::default();
    管理器.启动(命令).unwrap();

    let mut 中心 = 诊断中心::default();
    let 完成 = 等到(Duration::from_secs(300), || {
        let 序号 = 管理器.历史.最近().map(|记录| 记录.序号);
        for 行 in 管理器.轮询() {
            中心.采集运行日志(&行, 序号);
        }
        !管理器.状态.在运行()
    });
    assert!(完成, "运行没有在 300 秒内结束");

    let 快照 = 中心.快照();
    let panic_条目 = 快照
        .条目
        .iter()
        .find(|条目| 条目.代码.as_deref() == Some("panic"))
        .expect("panic 必须成为一条诊断");
    assert_eq!(panic_条目.来源, 诊断来源::运行);
    assert_eq!(panic_条目.任务.unwrap().类别, 任务类别::运行);
    assert!(panic_条目.可跳转(), "panic 必须带源码位置");
    assert!(
        panic_条目.行 > 0 && panic_条目.文件.is_some(),
        "panic 诊断必须能定位：{panic_条目:?}"
    );
    // 退出码不是 0，结束原因不是"正常退出"。
    assert_ne!(管理器.退出码, Some(0));
    assert_ne!(管理器.结束原因, editor_core::run_entry::结束原因::正常退出);
}

#[cfg(windows)]
#[test]
fn stdout与stderr并发与大日志量都不会丢内容或撑爆内存() {
    use editor_app::run::{日志行上限, 运行命令};

    let mut 管理器 = 项目运行管理器::default();
    // 两个流交替各写 1500 行：总量超过日志上限，同时验证不会死锁。
    let 脚本 = "(for /L %i in (1,1,1500) do @(echo out-%i& echo err-%i 1>&2))";
    管理器
        .启动(运行命令 {
            程序: "cmd".into(),
            参数: vec!["/C".into(), 脚本.into()],
            工作目录: std::env::temp_dir(),
            环境: Vec::new(),
            目标类型: 运行目标类型::项目可执行程序,
            入口: None,
        })
        .expect("启动 cmd");

    let 完成 = 等到(Duration::from_secs(120), || {
        let _ = 管理器.轮询();
        !管理器.状态.在运行()
    });
    assert!(完成, "并发输出没有在 120 秒内结束（可能死锁）");
    assert_eq!(
        管理器.日志.len(),
        日志行上限,
        "日志必须有界，实际 {}",
        管理器.日志.len()
    );
    // 两个流都必须出现在保留窗口里（stdout 与 stderr 都被读到）。
    assert!(
        管理器.日志.iter().any(|行| 行.contains("out-")),
        "stdout 内容丢失"
    );
    assert!(
        管理器
            .日志
            .iter()
            .any(|行| 行.starts_with("[stderr]") && 行.contains("err-")),
        "stderr 内容丢失"
    );
    assert_eq!(管理器.退出码, Some(0));
}

#[test]
fn 旧构建的诊断不会混进新构建的结果() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("isolate");
    写(
        &根.join("Cargo.toml"),
        "[package]\nname = \"workbench_isolate\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    写(&根.join("src/main.rs"), "fn main() {}\n");

    let 项目 = 项目描述::打开(&根).unwrap();
    let 配置 = 项目.清单().构建.clone().unwrap_or_default();
    let mut 宿主 = 构建宿主::新建();
    let mut 中心 = 诊断中心::default();

    // 第一次构建：成功，没有诊断。
    宿主.启动(&项目.构建根目录(), &配置).expect("第一次构建");
    等到(Duration::from_secs(180), || {
        let _ = 宿主.轮询();
        中心.采集构建(宿主.报告(), Some(&配置), 宿主.修订());
        !宿主.在运行()
    });
    let 第一次任务 = 中心.快照().构建任务.expect("第一次构建必须有任务");
    assert_eq!(中心.快照().错误数, 0);

    // 写坏源码再构建：新任务，旧任务的历史仍在但不再是"当前结果"。
    写(
        &根.join("src/main.rs"),
        "fn main() { let _x: i32 = \"坏\"; }\n",
    );
    宿主.启动(&项目.构建根目录(), &配置).expect("第二次构建");
    等到(Duration::from_secs(180), || {
        let _ = 宿主.轮询();
        中心.采集构建(宿主.报告(), Some(&配置), 宿主.修订());
        !宿主.在运行()
    });
    let 快照 = 中心.快照();
    let 第二次任务 = 快照.构建任务.unwrap();
    assert_ne!(第一次任务, 第二次任务, "第二次构建必须是新任务");
    assert!(快照.错误数 > 0, "第二次构建必须报错");
    assert!(
        快照.条目.iter().all(|条目| 条目.任务 == Some(第二次任务)),
        "当前结果里的诊断必须全部属于最新任务"
    );
    assert_eq!(快照.构建历史.len(), 2, "两次构建都要留下历史记录");
}

#[test]
fn 诊断能附进智能体上下文并带上真实内容与纪律() {
    use editor_ui::agent::{上下文种类, 智能体状态, 消息角色};

    let mut 中心 = 诊断中心::default();
    中心.采集运行日志(
        "thread 'main' panicked at src/main.rs:9:5: workbench-ctx-probe",
        Some(1),
    );
    let 快照 = 中心.快照();
    assert_eq!(快照.错误数, 1);
    let 正文 = 中心.集合.单条上下文(快照.条目[0].编号).unwrap();
    assert!(正文.contains("workbench-ctx-probe"), "{正文}");
    assert!(正文.contains("src/main.rs:9"), "{正文}");
    assert!(正文.contains("不要使用 git reset --hard"), "{正文}");
    assert!(正文.contains("不要自行声称构建通过"), "{正文}");

    let mut 状态 = 智能体状态::default();
    状态.新建会话("C:/proj/isolate", "会话");
    assert!(状态.附加上下文(editor_ui::agent::上下文项 {
        种类: 上下文种类::诊断,
        名称: "诊断 #1".into(),
        正文: 正文.clone(),
    }));
    let 提示 = 状态.上下文提示();
    assert!(提示.contains("workbench-ctx-probe"), "{提示}");
    assert!(提示.contains("诊断"), "上下文种类标签必须出现：{提示}");

    // 会话里能查到这条真实消息来源（不是伪造的"已验证"）。
    状态.追加消息(消息角色::用户, "帮我修这个 panic");
    assert_eq!(状态.当前().unwrap().消息.len(), 1);
}
