// SPDX-License-Identifier: MIT OR Apache-2.0
//! 命令行导出可玩包：`bevy-studio.exe --export-game <工程> <输出> [--windowed]`。
//!
//! # 为什么导出不再由脚本实现
//!
//! 包内结构、清单字段与"哪些资源算可达"由 `editor_core::package` 决定；
//! 让 `build_release.ps1` 再写一遍就是第二份实现 —— 上一版脚本用正则从
//! `project.ron` 抠名称/设计分辨率/启动场景，字段改名或出现转义字符就会
//! 静默导出错入口。现在脚本只负责构建与找路径，导出本身调用本模块，
//! 与编辑器界面的「打包」共用同一套计划、复制与清单逻辑。
//!
//! # 产物是哪个 exe
//!
//! 分两种情况，由项目清单决定，**中间没有静默回退**：
//!
//! | 清单 | 产物 |
//! | --- | --- |
//! | 没有 `运行` 段（纯微风项目） | 当前进程 —— 脚本构建出的 release exe 调用自身来导出 |
//! | 有 `运行` 段（项目自带 Cargo 目标） | `cargo build -p <包> --bin <目标> --release` 的产物 |
//!
//! 第二种是"项目代码真的进包"的唯一路径：包里那份 exe 是**用户项目自己**
//! 编译出来的可执行文件，链接了 `breeze_runtime`，因此它的 System 会执行。
//! 清单声明了运行目标却在 `Cargo.toml` 里找不到对应的包/目标（清单陈旧）
//! 时导出直接报错 —— 悄悄打包编辑器 exe 会让用户对着一个"能开但逻辑不对"
//! 的包排查很久。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use editor_core::build::{构建配置, 编译事件, 解析编译事件};
use editor_core::cargo_project::Cargo工程;
use editor_core::package::{导出可玩包, 打包报告};
use editor_core::project::{运行目标清单, 项目描述};
use editor_core::run_entry::运行入口;

/// 命令行参数名。
pub const 导出参数: &str = "--export-game";

/// 一条导出命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 导出命令 {
    /// 要导出的微风工程根目录。
    pub 工程目录: PathBuf,
    /// 可玩包输出目录（先清空）。
    pub 输出目录: PathBuf,
    /// 清单里的启动模式：`false` 时游戏窗口化启动。
    pub 全屏: bool,
}

/// 导出前要构建的 Cargo 目标（`cargo build -p <包名> --bin <目标名>`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 构建目标 {
    /// 包名。
    pub 包名: String,
    /// 可执行目标名。
    pub 目标名: String,
}

/// 包内那份 exe 从哪来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 产物来源 {
    /// 没有运行目标：沿用当前进程（纯微风项目的既有行为）。
    当前程序,
    /// 项目自带 Cargo 目标：先构建，再用构建产物。
    构建(构建目标),
}

/// 根据清单与 Cargo 工程决定"包里的 exe 从哪来"。
///
/// # 规则
///
/// 1. 清单没有 `运行` 段 → [`产物来源::当前程序`]；
/// 2. 有 `运行` 段但没有 Cargo 工程 → 报错（声明的运行目标不是可构建的东西）；
/// 3. 有 `运行` 段、有 Cargo 工程，但清单里没有包名 → 报错；
/// 4. 包名 / 证据都能对上 `Cargo.toml` → [`产物来源::构建`]；
/// 5. 包或目标在 `Cargo.toml` 里已经不存在 → 报错（清单陈旧）。
///
/// 第 2–5 条**都不回退**到当前程序：导出的产物必须与清单声明一致，
/// 不一致时宁可失败也不交出一个"能开但逻辑不对"的包。
///
/// 本函数不做任何 IO，`工程` 由调用方读取后传入，因而可以直接单测。
pub fn 选择产物来源(
    运行: Option<&运行目标清单>,
    构建: Option<&构建配置>,
    工程: Option<&Cargo工程>,
) -> Result<产物来源, String> {
    if 运行.is_none() {
        return Ok(产物来源::当前程序);
    }
    let Some(工程) = 工程 else {
        return Err(
            "项目声明了运行目标，但目录里没有可识别的 Cargo 工程，导出无法构建它；\
             请把游戏做成 Cargo 工程（`Cargo.toml` + `[[bin]]`），或移除清单里的运行目标"
                .to_string(),
        );
    };
    let Some(配置) = 构建 else {
        return Err(
            "清单声明了运行目标，但没有对应的 Cargo 包/二进制配置（项目不是 Cargo 目标）；\
             导出只能打包可构建的游戏，请在 Cargo.toml 里声明 `[[bin]]` 后重新选择运行入口"
                .to_string(),
        );
    };
    let 包名 = 配置
        .包名
        .as_deref()
        .map(str::trim)
        .filter(|文本| !文本.is_empty())
        .ok_or_else(|| "运行目标的构建配置缺少包名，无法构建".to_string())?;
    let 目标名 = 配置
        .二进制名
        .as_deref()
        .map(str::trim)
        .filter(|文本| !文本.is_empty())
        .unwrap_or(包名);
    let 入口 = 运行入口::新建(包名, 目标名);
    入口.校验(工程).map_err(|原因| {
        format!(
            "运行目标已失效：{}。清单里的包/目标与 `Cargo.toml` 不一致，\
             导出不会悄悄换成编辑器程序",
            原因.说明()
        )
    })?;
    Ok(产物来源::构建(构建目标 {
        包名: 包名.to_string(),
        目标名: 目标名.to_string(),
    }))
}

/// 打开工程并决定产物来源。
///
/// 微风项目以 `project.ron` 为准，`项目描述` 不会顺带识别同目录的
/// `Cargo.toml`；因此当清单声明了运行目标、而项目本身没有 Cargo 工程时，
/// 再按工程根补读一次 `Cargo.toml`（微风项目 + 自带 crate 的组合）。
pub fn 解析产物来源(工程根: &Path) -> Result<产物来源, String> {
    let 项目 = 项目描述::打开(工程根).map_err(|错误| format!("打开项目失败：{错误}"))?;
    let 运行 = 项目.清单().运行.as_ref();
    let 自带工程 = 项目.cargo_工程();
    let 补读工程 = match (运行.is_some(), 自带工程) {
        (true, None) => Cargo工程::读取(工程根).ok(),
        _ => None,
    };
    选择产物来源(
        运行,
        项目.清单().构建.as_ref(),
        自带工程.or(补读工程.as_ref()),
    )
}

/// 识别并执行 `--export-game`；没有这个参数时返回 `None`，
/// 把启动身份让给独立游戏模式与编辑器。
pub fn 从命令行导出(参数: &[String]) -> Option<Result<打包报告, String>> {
    match 解析导出命令(参数) {
        Some(Ok(命令)) => Some(执行导出(&命令)),
        Some(Err(错误)) => Some(Err(错误)),
        None => None,
    }
}

/// 执行一条导出命令：先确定产物来源，再交给共用打包流程。
///
/// # 顺序
///
/// 构建发生在清空输出目录**之前**：编译失败时上一版包原样保留，
/// 不会出现"编译报错 + 旧包已被删"的双重损失。
pub fn 执行导出(命令: &导出命令) -> Result<打包报告, String> {
    let 产物 = match 解析产物来源(&命令.工程目录)? {
        产物来源::当前程序 => {
            std::env::current_exe().map_err(|错误| format!("找不到当前可执行文件：{错误}"))?
        }
        产物来源::构建(目标) => 构建运行目标(&命令.工程目录, &目标)?,
    };
    let 报告 = 导出可玩包(&命令.工程目录, &命令.输出目录, &产物, 命令.全屏)?;
    if !报告.成功 {
        let 明细 = 报告
            .失败项
            .iter()
            .map(|(文件, 原因)| format!("{文件}：{原因}"))
            .collect::<Vec<_>>()
            .join("\n  ");
        return Err(format!(
            "有 {} 个文件写入失败：\n  {明细}",
            报告.失败项.len()
        ));
    }
    Ok(报告)
}

/// 用 `cargo build` 构建项目自己的可执行目标，返回产物路径。
///
/// # 参数
///
/// 固定 `--release`；`--message-format=json` 让 cargo 在 stdout 逐行给出
/// `compiler-artifact`，产物路径从那里取（`target-dir` 被重定向、
/// 或使用交叉编译目标时都能拿到真实路径，不靠猜 `target/release/`）。
///
/// # 错误转述
///
/// stderr 直接继承：cargo 的进度与"could not compile"实时可见。
/// 编译失败时再把 JSON 里的 `message.rendered` 逐条打印 —— 那是 rustc
/// 渲染好的人读文本（含文件、行号、源码片段），与终端里看到的一致。
/// 这里不会把失败压缩成一句"导出失败"。
fn 构建运行目标(工程根: &Path, 目标: &构建目标) -> Result<PathBuf, String> {
    let 输出 = Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .arg("--color=never")
        .arg("--release")
        .arg("-p")
        .arg(&目标.包名)
        .arg("--bin")
        .arg(&目标.目标名)
        .arg("--manifest-path")
        .arg(工程根.join("Cargo.toml"))
        .current_dir(工程根)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|错误| format!("无法启动 cargo（导出机需要安装 Rust 工具链）：{错误}"))?;

    let mut 候选: Vec<PathBuf> = Vec::new();
    let mut 诊断: Vec<String> = Vec::new();
    let 标准输出 = String::from_utf8_lossy(&输出.stdout);
    for 行 in 标准输出.lines() {
        match 解析编译事件(行) {
            编译事件::产物 { 路径 } => 候选.push(路径),
            编译事件::诊断(_) => {
                if let Some(文本) = 提取渲染文本(行) {
                    诊断.push(文本);
                }
            }
            编译事件::结束 { .. } | 编译事件::无关 => {}
        }
    }

    if !输出.status.success() {
        let 正文 = if 诊断.is_empty() {
            "cargo 没有给出可读的诊断，原始输出见上方".to_string()
        } else {
            诊断.join("\n")
        };
        return Err(format!(
            "构建运行目标 {}::{} 失败（cargo 退出码 {}）：\n{正文}",
            目标.包名,
            目标.目标名,
            输出.status.code().unwrap_or(-1)
        ));
    }
    挑选产物(&候选, &目标.目标名)
}

/// 从 cargo 报告的可执行产物里挑出目标那一个。
///
/// 优先按文件名（不含扩展名）精确匹配；只有一个候选时直接采用，
/// 因为 `--bin <目标>` 决定了本次构建只可能产出它。
fn 挑选产物(候选: &[PathBuf], 目标名: &str) -> Result<PathBuf, String> {
    if let Some(路径) = 候选
        .iter()
        .find(|路径| 路径.file_stem().and_then(|名| 名.to_str()) == Some(目标名))
    {
        return Ok(路径.clone());
    }
    match 候选 {
        [] => Err("cargo 报告成功，但没有给出可执行产物路径（本次构建只有库目标？）".to_string()),
        [唯一] => Ok(唯一.clone()),
        多个 => Err(format!(
            "cargo 给出了多个可执行产物，但没有一个叫 `{目标名}`：{}",
            多个
                .iter()
                .map(|路径| 路径.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        )),
    }
}

/// 从一行 `compiler-message` 里取出 rustc 渲染好的人读文本。
fn 提取渲染文本(行: &str) -> Option<String> {
    let 值: serde_json::Value = serde_json::from_str(行.trim()).ok()?;
    let 文本 = 值.get("message")?.get("rendered")?.as_str()?;
    let 文本 = 文本.trim_end();
    (!文本.is_empty()).then(|| 文本.to_string())
}

/// 解析参数；`None` 表示命令行里没有 `--export-game`。
///
/// 位置参数按出现顺序取两个：工程目录、输出目录。`--windowed` 可在任意
/// 位置，含义与独立模式的同名参数一致 —— 清单里写 `全屏: false`。
pub fn 解析导出命令(参数: &[String]) -> Option<Result<导出命令, String>> {
    let 位置 = 参数.iter().position(|项| 项 == 导出参数)?;
    let 全屏 = !参数.iter().any(|项| 项 == "--windowed");
    let mut 位置参数: Vec<&String> = Vec::new();
    for 项 in &参数[位置 + 1..] {
        if 项 == "--windowed" {
            continue;
        }
        if 项.starts_with("--") {
            return Some(Err(format!("不认识的导出参数：{项}")));
        }
        位置参数.push(项);
    }
    if 位置参数.len() != 2 {
        return Some(Err(format!(
            "用法：{导出参数} <工程目录> <输出目录> [--windowed]（当前给了 {} 个位置参数）",
            位置参数.len()
        )));
    }
    Some(Ok(导出命令 {
        工程目录: PathBuf::from(位置参数[0]),
        输出目录: PathBuf::from(位置参数[1]),
        全屏,
    }))
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 参数(原始: &[&str]) -> Vec<String> {
        原始.iter().map(|项| 项.to_string()).collect()
    }

    #[test]
    fn 没有导出参数时返回空() {
        assert!(解析导出命令(&参数(&["--game", "示例"])).is_none());
        assert!(解析导出命令(&[]).is_none());
    }

    #[test]
    fn 解析工程与输出目录() {
        let 命令 = 解析导出命令(&参数(
            &["--export-game", "examples/3d_demo", "Game_Build"],
        ))
        .expect("应当识别")
        .expect("应当合法");
        assert_eq!(命令.工程目录, PathBuf::from("examples/3d_demo"));
        assert_eq!(命令.输出目录, PathBuf::from("Game_Build"));
        assert!(命令.全屏);
    }

    #[test]
    fn 窗口参数可以在任意位置并且只影响全屏() {
        let 命令 = 解析导出命令(&参数(&["--windowed", "--export-game", "工程", "输出"]))
            .expect("应当识别")
            .expect("应当合法");
        assert!(!命令.全屏);
        assert_eq!(命令.工程目录, PathBuf::from("工程"));
    }

    #[test]
    fn 位置参数数量不对时报用法() {
        let 错误 = 解析导出命令(&参数(&["--export-game", "只有工程"]))
            .expect("应当识别")
            .expect_err("缺输出目录必须报错");
        assert!(错误.contains("用法"), "错误原文：{错误}");
    }

    #[test]
    fn 不认识的参数不会被静默忽略() {
        let 错误 = 解析导出命令(&参数(&[
            "--export-game",
            "工程",
            "输出",
            "--输出目录",
            "别的",
        ]))
        .expect("应当识别")
        .expect_err("未知参数必须报错");
        assert!(错误.contains("不认识的导出参数"), "错误原文：{错误}");
    }

    use editor_core::cargo_project::{Cargo包, Cargo可执行目标, Cargo工程};

    fn 造工程(包名: &str, 可执行目标: &[&str]) -> Cargo工程 {
        let 根 = PathBuf::from("C:/工程");
        Cargo工程 {
            根目录: 根.clone(),
            清单路径: 根.join("Cargo.toml"),
            是工作区: false,
            包: vec![Cargo包 {
                名称: 包名.to_string(),
                版本: "0.1.0".to_string(),
                根目录: 根.clone(),
                清单路径: 根.join("Cargo.toml"),
                是工作区根: true,
                依赖贝维: true,
                库名: None,
                可执行目标: 可执行目标
                    .iter()
                    .map(|名| Cargo可执行目标 {
                        包名: 包名.to_string(),
                        名称: (*名).to_string(),
                        包根: 根.clone(),
                        源文件: None,
                        种类: vec!["bin".to_string()],
                    })
                    .collect(),
            }],
            缺失成员: Vec::new(),
        }
    }

    fn 造运行() -> 运行目标清单 {
        运行目标清单 {
            程序: "cargo".to_string(),
            参数: vec!["run".to_string()],
            环境: Default::default(),
            工作目录: None,
        }
    }

    fn 造构建(包名: &str, 目标: Option<&str>) -> 构建配置 {
        构建配置 {
            包名: Some(包名.to_string()),
            二进制名: 目标.map(str::to_string),
            ..构建配置::default()
        }
    }

    #[test]
    fn 没有运行目标时沿用当前程序() {
        assert_eq!(
            选择产物来源(None, None, None).expect("判定"),
            产物来源::当前程序,
            "没有运行段就维持既有导出行为"
        );
        let 工程 = 造工程("my_game", &["my_game"]);
        let 构建 = 造构建("my_game", None);
        assert_eq!(
            选择产物来源(None, Some(&构建), Some(&工程)).expect("判定"),
            产物来源::当前程序,
            "构建配置存在但清单没有运行段，不算自带游戏"
        );
    }

    #[test]
    fn 有运行目标时给出要构建的包与目标() {
        let 运行 = 造运行();
        let 构建 = 造构建("my_game", Some("my_game"));
        let 工程 = 造工程("my_game", &["my_game"]);
        assert_eq!(
            选择产物来源(Some(&运行), Some(&构建), Some(&工程)).expect("判定"),
            产物来源::构建(构建目标 {
                包名: "my_game".to_string(),
                目标名: "my_game".to_string(),
            })
        );
    }

    #[test]
    fn 清单陈旧时明确报错且不回退() {
        let 运行 = 造运行();
        let 构建 = 造构建("my_game", Some("server"));
        let 工程 = 造工程("my_game", &["my_game"]);
        let 错误 = 选择产物来源(Some(&运行), Some(&构建), Some(&工程)).expect_err("必须报错");
        assert!(错误.contains("server"), "要点名失效的目标：{错误}");
        assert!(
            错误.contains("已失效"),
            "要说清是清单与 Cargo.toml 不一致：{错误}"
        );
        assert!(
            错误.contains("不会悄悄换成编辑器程序"),
            "导出宁可失败也不能交出错误产物：{错误}"
        );
    }

    #[test]
    fn 声明了运行目标但工程不可识别时报错() {
        let 运行 = 造运行();
        let 错误 = 选择产物来源(Some(&运行), None, None).expect_err("必须报错");
        assert!(错误.contains("Cargo 工程"), "{错误}");
    }

    #[test]
    fn 有工程但没有构建配置时报错() {
        let 运行 = 造运行();
        let 工程 = 造工程("my_game", &["my_game"]);
        let 错误 = 选择产物来源(Some(&运行), None, Some(&工程)).expect_err("必须报错");
        assert!(错误.contains("没有对应的 Cargo 包"), "{错误}");
    }

    #[test]
    fn 构建配置缺包名时报错() {
        let 运行 = 造运行();
        let mut 构建 = 造构建("my_game", None);
        构建.包名 = None;
        let 工程 = 造工程("my_game", &["my_game"]);
        let 错误 = 选择产物来源(Some(&运行), Some(&构建), Some(&工程)).expect_err("必须报错");
        assert!(错误.contains("缺少包名"), "{错误}");
    }

    #[test]
    fn 挑选产物按目标名匹配() {
        let 甲 = PathBuf::from("target/release/server.exe");
        let 乙 = PathBuf::from("target/release/my_game.exe");
        assert_eq!(
            挑选产物(&[甲.clone(), 乙.clone()], "my_game").expect("命中"),
            乙,
            "多个产物时按目标名挑，而不是取第一个"
        );
        assert_eq!(
            挑选产物(std::slice::from_ref(&甲), "别的名字").expect("单个候选"),
            甲,
            "`--bin` 只可能产出一个可执行文件"
        );
        assert!(挑选产物(&[], "my_game").is_err(), "没有产物要报错");
        let 错误 = 挑选产物(&[甲, 乙], "third").expect_err("都不匹配");
        assert!(错误.contains("third"), "{错误}");
    }

    #[test]
    fn 从编译消息里取出渲染文本() {
        let 行 = r#"{"reason":"compiler-message","message":{"rendered":"error[E0308]: mismatched types\n"}}"#;
        assert_eq!(
            提取渲染文本(行).as_deref(),
            Some("error[E0308]: mismatched types")
        );
        assert!(
            提取渲染文本(r#"{"reason":"build-finished","success":true}"#).is_none(),
            "不是诊断的行不能被当成错误文本"
        );
    }
}
