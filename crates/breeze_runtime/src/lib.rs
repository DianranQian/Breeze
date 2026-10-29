// SPDX-License-Identifier: MIT OR Apache-2.0
//! 微风游戏运行时：用户项目 crate 的唯一依赖。
//!
//! # 它解决的具体问题
//!
//! 旧导出把**编辑器自己的 exe** 复制进游戏包，因此包里跑的永远是编辑器的
//! 投影与运行时内核，项目自己的 System / Plugin 一个都不会执行。本 crate 把
//! 那条缝补上：用户项目是一个普通 Rust crate，依赖本 crate，`main` 里调一次
//! [`启动`] 就得到"只有画面、没有界面"的游戏应用，自己的逻辑用
//! [`启动带扩展`] 挂进去。
//!
//! ```text
//! 用户项目 my_game/
//! ├── Cargo.toml   breeze_runtime = { path = "…/crates/breeze_runtime" }
//! ├── src/main.rs  breeze_runtime::启动带扩展(装配我的系统);
//! ├── project.ron  运行: Cargo可执行目标 { 包: "my_game", 目标: "my_game" }
//! └── assets/ scenes/
//! ```
//!
//! # 依赖规则（唯一一条硬约束）
//!
//! 用户项目的 `Cargo.toml` 里**不要再写 `bevy`**。Bevy 必须只有一份：
//! 两份特性并集不同的 Bevy 会实例化出两套同名类型，`Transform`、`Entity`、
//! `App` 彼此不通用，报错现场极难辨认。需要什么类型就 `use` 本 crate 的
//! [`prelude`]，它原样转发 Bevy 的 prelude。
//!
//! # 工程目录从哪来
//!
//! | 情况 | 工程目录 |
//! | --- | --- |
//! | 命令行给了 `--project <目录>` / `--project=<目录>` | 就是它（相对路径按当前目录解析） |
//! | exe 同级有 `game_manifest.ron` | exe 所在目录（发布包的形态） |
//! | 当前工作目录有 `game_manifest.ron` | 当前工作目录（开发期便利路径） |
//! | 都没有 | 报错，并点名上面查过的两个位置 |
//!
//! 与编辑器 exe 的 `--game` 身份不同，这里**没有"退回编辑器"这一档**：
//! 用户项目本来就是游戏，找不到工程只能报错，不能悄悄开出一个编辑器窗口。
//!
//! # 调试参数
//!
//! `--windowed` / `--monitor <n>` / `--bench [秒]` 与编辑器游戏身份完全一致，
//! 解析直接复用 `editor_app::standalone`，不在这里重写第二套。
//!
//! # 诚实边界（薄壳阶段）
//!
//! 本 crate 直接依赖 `editor_app` 的库目标，因此游戏二进制会把编辑器应用的
//! 依赖图一起编进来（egui / 文件对话框 / 文件监视等）。链接器会裁掉未被引用
//! 的代码，但编译时间与产物体积仍高于理论值。把投影与运行时内核拆成独立
//! crate 是后续单独排期的工作，不在薄壳阶段做。

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use editor_app::standalone::{启动请求, 构建游戏应用并扩展, 识别启动, 载入};
use editor_core::game_manifest::{游戏清单, 游戏清单文件名};

pub use bevy;
pub use editor_app;

/// 常用类型的统一入口：与 `bevy::prelude` 完全相同。
///
/// 用户项目只依赖本 crate，因此不能写 `use bevy::prelude::*`；
/// 写 `use breeze_runtime::prelude::*` 拿到的就是同一批名字。
pub mod prelude {
    pub use bevy::prelude::*;
}

/// 命令行里显式指定工程目录的参数名。
pub const 工程参数: &str = "--project";

/// 以游戏身份启动并阻塞到窗口关闭；失败时打印原因并退出进程。
///
/// 适用于"不需要往应用里加任何东西"的项目。要挂自己的 System / Plugin，
/// 用 [`启动带扩展`]。
pub fn 启动() {
    启动带扩展(|_| {});
}

/// 以游戏身份启动，并交给调用方在进入主循环前追加自定义装配。
///
/// # 参数
///
/// * `装配` —— 拿到的是"投影与运行时都已就位"的应用：
///   `editor_app::standalone` 的装配内核已经跑完，`Startup` 里的游戏初始化
///   随后执行。往 `Update` 加系统不会与首帧初始化抢跑。
///
/// # 失败处理
///
/// 工程缺失、清单损坏、主场景读不出来都属于"启动不起来"，此时打印原因并
/// 以退出码 1 结束进程 —— 玩家双击的游戏包没有控制台可以展示半启动状态。
pub fn 启动带扩展<装配>(装配: 装配)
where
    装配: FnOnce(&mut App),
{
    let 参数: Vec<String> = std::env::args().skip(1).collect();
    let 可执行目录 = 可执行目录();
    let 当前目录 = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let 结果 = 解析启动请求(&参数, &可执行目录, &当前目录).and_then(|请求| 载入(&请求));
    match 结果 {
        Ok(配置) => {
            构建游戏应用并扩展(配置, 装配).run();
        }
        Err(错误) => {
            eprintln!("游戏启动失败：{错误}");
            std::process::exit(1);
        }
    }
}

/// 把命令行解析成游戏启动请求（不碰窗口，可在测试里直接调用）。
///
/// # 步骤
///
/// 1. 先认 `--project`：显式给了就用它，不再做清单探测；
/// 2. 否则按"exe 同级 → 当前目录"找 `game_manifest.ron`，两处都没有就报错；
/// 3. 把 `--project` 改写成 `--game` 交给 [`识别启动`]，让全屏 / 显示器 /
///    基准这几个开关只保留一份解析实现。
///
/// `--project` 只影响"工程目录"，其余参数原样透传。
pub fn 解析启动请求(
    参数: &[String],
    可执行目录: &Path,
    当前目录: &Path,
) -> Result<启动请求, String> {
    let 目录 = match 解析工程目录(参数, 当前目录) {
        Some(目录) => 目录,
        None => 默认工程目录(可执行目录, 当前目录)?,
    };
    let 游戏参数 = 转成游戏参数(参数, &目录);
    match 识别启动(&游戏参数, 可执行目录, 当前目录)? {
        Some(请求) => Ok(请求),
        None => Err(format!("无法以游戏身份启动（工程目录 {}）", 目录.display())),
    }
}

/// exe 所在目录；取不到时退回当前目录。
fn 可执行目录() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|路径| 路径.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 取出 `--project 目录` / `--project=目录` 的值。
fn 解析工程目录(参数: &[String], 当前目录: &Path) -> Option<PathBuf> {
    let 前缀 = format!("{工程参数}=");
    let mut 索引 = 0;
    while 索引 < 参数.len() {
        let 当前 = 参数[索引].as_str();
        if let Some(尾) = 当前.strip_prefix(&前缀) {
            let 尾 = 尾.trim();
            return (!尾.is_empty()).then(|| 落盘(尾, 当前目录));
        }
        if 当前 == 工程参数 {
            return 参数
                .get(索引 + 1)
                .map(|值| 值.trim())
                .filter(|值| !值.is_empty() && !值.starts_with('-'))
                .map(|值| 落盘(值, 当前目录));
        }
        索引 += 1;
    }
    None
}

/// 把相对路径落到当前目录下（绝对路径原样保留）。
fn 落盘(文本: &str, 当前目录: &Path) -> PathBuf {
    let 路径 = Path::new(文本);
    if 路径.is_absolute() {
        路径.to_path_buf()
    } else {
        当前目录.join(路径)
    }
}

/// 没有显式工程目录时的探测顺序：exe 同级 → 当前目录。
fn 默认工程目录(可执行目录: &Path, 当前目录: &Path) -> Result<PathBuf, String> {
    for 目录 in [可执行目录, 当前目录] {
        if 游戏清单::定位(目录).is_some() {
            return Ok(目录.to_path_buf());
        }
    }
    Err(format!(
        "在 {} 与 {} 都没有找到 {游戏清单文件名}；发布包里它应当与 exe 同级，\
         开发时用 {工程参数} <工程目录> 指定",
        可执行目录.display(),
        当前目录.display()
    ))
}

/// 去掉 `--project`，并补一条 `--game <工程目录>`。
///
/// `--project` 后面没有取值（或取值以 `-` 开头）时只跳过它自己，
/// 不能把紧跟的开关（例如 `--windowed`）当成取值一起吞掉。
fn 转成游戏参数(参数: &[String], 目录: &Path) -> Vec<String> {
    let 前缀 = format!("{工程参数}=");
    let mut 结果: Vec<String> = Vec::with_capacity(参数.len() + 2);
    let mut 索引 = 0;
    while 索引 < 参数.len() {
        let 当前 = 参数[索引].as_str();
        if 当前.starts_with(&前缀) {
            索引 += 1;
            continue;
        }
        if 当前 == 工程参数 {
            let 有取值 = 参数
                .get(索引 + 1)
                .is_some_and(|值| !值.trim().is_empty() && !值.starts_with('-'));
            索引 += if 有取值 { 2 } else { 1 };
            continue;
        }
        结果.push(参数[索引].clone());
        索引 += 1;
    }
    结果.push("--game".to_string());
    结果.push(目录.display().to_string());
    结果
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_core::game_manifest::游戏清单格式版本;

    fn 参数表(项: &[&str]) -> Vec<String> {
        项.iter().map(|项| 项.to_string()).collect()
    }

    fn 写清单(目录: &Path) {
        std::fs::create_dir_all(目录).expect("建目录");
        let 清单 = 游戏清单 {
            格式版本: 游戏清单格式版本,
            名称: "示例游戏".to_string(),
            主场景: "scenes/主场景.scene.ron".to_string(),
            设计分辨率: (1280, 720),
            全屏: true,
        };
        清单.写入(目录).expect("写清单");
    }

    #[test]
    fn exe同级的清单被认作工程目录() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("Game_Build");
        写清单(&包);
        let 别处 = tempfile::tempdir().expect("临时目录");
        let 请求 = 解析启动请求(&参数表(&[]), &包, 别处.path()).expect("判定");
        assert_eq!(请求.项目目录, 包);
        assert_eq!(请求.清单.expect("有清单").名称, "示例游戏");
    }

    #[test]
    fn 清单只在工作目录时也能启动() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 工程 = 临时.path().join("工程");
        写清单(&工程);
        let 空 = tempfile::tempdir().expect("临时目录");
        let 请求 = 解析启动请求(&参数表(&[]), 空.path(), &工程).expect("判定");
        assert_eq!(请求.项目目录, 工程);
    }

    #[test]
    fn 两处都没有清单时报错并点名两个位置() {
        let 空甲 = tempfile::tempdir().expect("临时目录");
        let 空乙 = tempfile::tempdir().expect("临时目录");
        let 错误 = 解析启动请求(&参数表(&[]), 空甲.path(), 空乙.path()).expect_err("必须报错");
        assert!(错误.contains(游戏清单文件名), "要点名缺的文件：{错误}");
        assert!(
            错误.contains(&空甲.path().display().to_string())
                && 错误.contains(&空乙.path().display().to_string()),
            "要把查过的两个位置都说出来：{错误}"
        );
    }

    #[test]
    fn 工程参数覆盖清单探测() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 指定 = 临时.path().join("指定工程");
        std::fs::create_dir_all(&指定).expect("建目录");

        let 等号 = 解析启动请求(
            &参数表(&[&format!("{工程参数}={}", 指定.display())]),
            临时.path(),
            临时.path(),
        )
        .expect("判定");
        assert_eq!(等号.项目目录, 指定);

        let 空格 = 解析启动请求(
            &参数表(&[工程参数, 指定.to_str().unwrap()]),
            临时.path(),
            临时.path(),
        )
        .expect("判定");
        assert_eq!(空格.项目目录, 指定);
    }

    #[test]
    fn 调试参数原样透传且窗口模式优先() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 工程 = 临时.path().join("工程");
        std::fs::create_dir_all(&工程).expect("建目录");
        let 请求 = 解析启动请求(
            &参数表(&[
                工程参数,
                工程.to_str().unwrap(),
                "--windowed",
                "--monitor",
                "2",
                "--bench=3",
            ]),
            临时.path(),
            临时.path(),
        )
        .expect("判定");
        assert!(!请求.全屏, "--windowed 必须覆盖全屏");
        assert_eq!(请求.显示器, None, "窗口模式下显示器选择不生效");
        assert_eq!(请求.基准秒, Some(3.0));
        assert!(请求.清单.is_none(), "没有清单时由 project.ron 兜底");
    }

    #[test]
    fn 工程参数没有取值时不吞掉后面的开关() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("Game_Build");
        写清单(&包);
        let 请求 =
            解析启动请求(&参数表(&[工程参数, "--windowed"]), &包, 临时.path()).expect("判定");
        assert_eq!(请求.项目目录, 包, "缺值时应退回清单探测");
        assert!(!请求.全屏, "--windowed 不能被当成 --project 的取值吞掉");
    }
}
