//! 原生 Cargo / Bevy 工程的打开、识别与"绝不改写用户工程"的回归测试。
//!
//! # 这些测试为什么必须用真实的标准 Cargo 工程
//!
//! 上一轮的缺陷是"原生 Bevy 项目无法打开"，根因是打开链路只认 `project.ron`。
//! 只构造一个假的项目对象来测，永远测不到"真实目录里没有 project.ron 会怎样"。
//! 因此这里每个用例都在临时目录里生成**真实的最小 Cargo 工程**
//! （`Cargo.toml` + `src/main.rs`），再从公开 API 走完整打开流程。
//!
//! # 同时要守住"不越权"
//!
//! 原生工程是用户自己的仓库。这里逐条断言：
//! 不产生 `project.ron`、`Cargo.toml` 一个字节都不变、不移动源码。

use std::path::{Path, PathBuf};

use editor_core::project::{项目描述, 项目清单文件名, 项目类型, 项目错误};
use editor_core::state::编辑器状态;

/// 写一个文件（自动建父目录）。
fn 写(路径: &Path, 文本: &str) {
    if let Some(父) = 路径.parent() {
        std::fs::create_dir_all(父).unwrap();
    }
    std::fs::write(路径, 文本).unwrap();
}

/// 造一个单包 Cargo 工程（有可执行目标、依赖 bevy）。
fn 造单包工程(根: &Path) {
    写(
        &根.join("Cargo.toml"),
        r#"[package]
name = "breeze_native_fixture"
version = "0.1.0"
edition = "2024"

[dependencies]
bevy = "0.20"
"#,
    );
    写(
        &根.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    );
    写(&根.join("assets/textures/hero.png"), "not-a-real-png");
}

/// 造一个虚拟工作区：一个库成员 + 一个游戏成员。
fn 造工作区(根: &Path) {
    写(
        &根.join("Cargo.toml"),
        r#"[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.dependencies]
bevy = "0.20"
"#,
    );
    写(
        &根.join("crates/engine/Cargo.toml"),
        r#"[package]
name = "fixture_engine"
version = "0.1.0"
"#,
    );
    写(&根.join("crates/engine/src/lib.rs"), "pub fn tick() {}\n");
    写(
        &根.join("crates/game/Cargo.toml"),
        r#"[package]
name = "fixture_game"
version = "0.1.0"

[dependencies]
bevy = { workspace = true }
"#,
    );
    写(&根.join("crates/game/src/main.rs"), "fn main() {}\n");
}

#[test]
fn 无微风配置的原生工程可以被打开并使用() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    造单包工程(根);
    // 前提：打开前没有任何微风专用文件。
    assert!(!根.join(项目清单文件名).exists());

    let 项目 = 项目描述::打开(根).expect("原生 Cargo 工程必须能打开");

    assert_eq!(项目.类型(), 项目类型::原生Cargo);
    assert!(项目.根目录().is_absolute(), "项目根目录必须是绝对路径");
    assert_eq!(项目.名称(), "breeze_native_fixture");
    // 构建工作目录必须与项目根完全一致：两处路径漂移会让诊断里的文件路径对不上。
    assert_eq!(项目.构建根目录(), 项目.根目录());
    // 文件树：源码与资源都能被列出来。
    let 源文件: Vec<String> = 项目
        .源文件列表()
        .iter()
        .map(|路径| 路径.文本().to_string())
        .collect();
    assert_eq!(源文件, vec!["src/main.rs"]);
    let 资源: Vec<String> = 项目
        .资源列表()
        .iter()
        .map(|条目| 条目.路径.文本().to_string())
        .collect();
    assert_eq!(资源, vec!["assets/textures/hero.png"]);
    // 没有微风场景不该让打开失败：场景列表为空，但项目是"已打开"的。
    assert!(项目.场景列表().is_empty());

    // 运行目标：真实 Cargo 可执行目标。
    let 运行 = 项目.清单().运行.as_ref().expect("应当推导出运行目标");
    assert_eq!(运行.程序, "cargo");
    assert_eq!(
        运行.参数,
        vec![
            "run",
            "-p",
            "breeze_native_fixture",
            "--bin",
            "breeze_native_fixture"
        ]
    );
    assert_eq!(
        项目
            .清单()
            .构建
            .as_ref()
            .and_then(|配置| 配置.包名.as_deref()),
        Some("breeze_native_fixture")
    );

    // 关键：打开过程不得往用户工程里写任何东西。
    assert!(
        !根.join(项目清单文件名).exists(),
        "打开原生工程不得生成 project.ron"
    );
}

#[test]
fn 打开原生工程不会修改用户清单与源码() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    造单包工程(根);
    let 清单路径 = 根.join("Cargo.toml");
    let 原始清单 = std::fs::read_to_string(&清单路径).unwrap();
    let 原始源码 = std::fs::read_to_string(根.join("src/main.rs")).unwrap();

    let 项目 = 项目描述::打开(根).unwrap();
    // 试图保存设置必须被拒绝，且磁盘上不能多出任何文件。
    assert!(
        matches!(项目.保存清单(), Err(项目错误::原生工程只读(_))),
        "原生工程的清单是只读快照，写它等于篡改用户的构建配置"
    );

    assert_eq!(std::fs::read_to_string(&清单路径).unwrap(), 原始清单);
    assert_eq!(
        std::fs::read_to_string(根.join("src/main.rs")).unwrap(),
        原始源码
    );
    assert!(!根.join(项目清单文件名).exists());
}

#[test]
fn 工作区工程能选出可运行成员且构建根在workspace根() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    造工作区(根);

    let 项目 = 项目描述::打开(根).unwrap();
    assert_eq!(项目.类型(), 项目类型::原生Cargo);
    assert_eq!(
        项目.构建根目录(),
        项目.根目录(),
        "workspace 的构建必须在根目录执行，成员选择靠 -p，而不是换工作目录"
    );

    let 工程 = 项目.cargo_工程().expect("原生工程必须有 Cargo 识别结果");
    assert!(工程.是工作区);
    assert_eq!(工程.包.len(), 2, "两个成员包都要被识别");
    // 只有库的成员不会被当成游戏；有 bin 的成员被选中。
    let 目标 = 工程.默认可执行目标().expect("应当选出可运行目标");
    assert_eq!(目标.包名, "fixture_game");
    assert_eq!(目标.名称, "fixture_game");

    // 多个成员包的源码都要能被浏览到。
    let 源文件: Vec<String> = 项目
        .源文件列表()
        .iter()
        .map(|路径| 路径.文本().to_string())
        .collect();
    assert!(源文件.contains(&"crates/engine/src/lib.rs".to_string()));
    assert!(源文件.contains(&"crates/game/src/main.rs".to_string()));

    // 构建配置锁定了同一个包。
    let 配置 = 项目.清单().构建.as_ref().unwrap();
    assert_eq!(配置.包名.as_deref(), Some("fixture_game"));
}

#[test]
fn 只有库的工程能打开但没有运行目标() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    写(
        &根.join("Cargo.toml"),
        r#"[package]
name = "fixture_lib_only"
version = "0.1.0"
"#,
    );
    写(
        &根.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    );

    let 项目 = 项目描述::打开(根).expect("库工程也是 Cargo 工程，应当能打开");
    assert!(项目.清单().运行.is_none(), "库不是游戏，不能声明运行目标");
    let 工程 = 项目.cargo_工程().unwrap();
    assert!(!工程.有可运行目标());
    assert_eq!(工程.包[0].库名.as_deref(), Some("fixture_lib_only"));
}

#[test]
fn 非工程目录给出明确错误() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    // 一个普通 Rust 脚本目录：既没有 project.ron，也没有 Cargo.toml。
    写(&根.join("main.rs"), "fn main() {}");
    match 项目描述::打开(根) {
        Err(项目错误::未找到项目(路径)) => {
            assert_eq!(路径, 根, "错误里应当带上被检查的目录")
        }
        其他 => panic!("应当报告未找到项目，实际：{其他:?}"),
    }
}

#[test]
fn 损坏的清单给出解析错误而不是打开成功() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path();
    写(&根.join("Cargo.toml"), "[package\nname = 不是合法 TOML\n");
    写(&根.join("src/main.rs"), "fn main() {}");
    match 项目描述::打开(根) {
        Err(项目错误::Cargo工程(_)) => {}
        其他 => panic!("应当报告 Cargo 清单解析失败，实际：{其他:?}"),
    }
}

#[test]
fn 编辑器状态能打开原生工程并给出限制说明() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("游戏");
    造单包工程(&根);

    let mut 状态 = 编辑器状态::新建();
    状态.打开项目(&根).expect("编辑器必须能打开原生工程");
    assert_eq!(状态.项目名(), "breeze_native_fixture");
    // 没有场景文件时仍应有一个可编辑的空场景（而不是把整个项目打开失败）。
    assert!(状态.场景名().contains("主场景") || 状态.场景名().contains("场景"));
    // 控制台必须如实说明工程是怎么被识别的、以及哪些能力暂不适用。
    // 用稳定标识而不是渲染文案：断言不该依赖当前语言。
    let 标识: Vec<&str> = 状态
        .日志列表()
        .iter()
        .filter_map(|条目| 条目.标识())
        .collect();
    assert!(
        标识.contains(&"log_project_cargo_recognized"),
        "必须记录「从 Cargo.toml 识别工程」：{标识:?}"
    );
    assert!(
        标识.contains(&"log_project_native_no_scene"),
        "必须记录「该项目没有微风场景文件」：{标识:?}"
    );
    let 渲染: String = 状态
        .日志列表()
        .iter()
        .map(|条目| 条目.渲染(editor_core::i18n::语言::中文))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(渲染.contains("Cargo"), "控制台文案应当提到 Cargo：{渲染}");
}

#[test]
fn 微风项目仍然按原格式打开() {
    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("微风项目");
    std::fs::create_dir_all(&根).unwrap();
    let mut 项目 = 项目描述::创建(&根, "微风样例").expect("创建微风项目");
    项目.清单可变().启动场景 = None;
    // 顺带写一份 Cargo.toml：微风项目即使带 Cargo.toml 也必须优先按 project.ron 解析。
    写(
        &根.join("Cargo.toml"),
        r#"[package]
name = "should_be_ignored"
version = "0.1.0"
"#,
    );
    let _ = 项目;

    let 重开 = 项目描述::打开(&根).unwrap();
    assert_eq!(重开.类型(), 项目类型::微风);
    assert_eq!(重开.名称(), "微风样例");
    assert!(重开.cargo_工程().is_none());
}

/// 帮编译器明确一下临时目录的生命周期：所有用例都依赖 `tempfile::TempDir`
/// 在作用域结束时删除目录，这里只做一个显式引用，避免将来有人把路径 clone
/// 出去后误以为目录还在。
#[allow(dead_code)]
fn _保持目录类型(路径: PathBuf) -> PathBuf {
    路径
}
