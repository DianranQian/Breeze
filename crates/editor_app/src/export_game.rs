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
//! 导出固定使用**当前进程**：脚本构建出的 release exe 再调用自身来导出，
//! 因此包里那份 exe 就是构建产物，不存在"脚本复制了另一份"的歧义。

use std::path::PathBuf;

use editor_core::package::{导出可玩包, 打包报告};

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

/// 识别并执行 `--export-game`；没有这个参数时返回 `None`，
/// 把启动身份让给独立游戏模式与编辑器。
pub fn 从命令行导出(参数: &[String]) -> Option<Result<打包报告, String>> {
    match 解析导出命令(参数) {
        Some(Ok(命令)) => Some(执行导出(&命令)),
        Some(Err(错误)) => Some(Err(错误)),
        None => None,
    }
}

/// 执行一条导出命令，产物固定为当前可执行文件。
pub fn 执行导出(命令: &导出命令) -> Result<打包报告, String> {
    let 产物 = std::env::current_exe().map_err(|错误| format!("找不到当前可执行文件：{错误}"))?;
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
}
