//! 中文名：动作可达性
//! 动作可达性：每个 `UiAction` 变体都必须有**构造点**，不能只有处理分支。
//!
//! # 为什么需要这条测试
//!
//! `UiAction` 是"界面 → 逻辑"的唯一入口。历史上三次出现同一种缺陷：
//! 动作在 `actions.rs` 里实现了处理逻辑，但**界面上没有任何地方推它**——
//! 于是它对用户完全不存在，而且不会报错、不会告警，只能靠人读代码发现：
//!
//! * `试听选中实体音频`（round 4 实现，round 14 才有入口）
//! * `移除最近项目`（round 2 实现，round 15 才有入口）
//! * `删除动画轨道`（round 5 实现，round 24 才有入口）
//!
//! 这条测试把"每个变体至少有一个**构造点**"变成硬约束。注意不能用
//! "出现次数 > 1" 来判断：动作域拆分后，一个被拆走的变体同时出现在
//! 处理函数和分组哨兵里，即使界面上没有任何入口，计数也会大于 1。
//! 因此这里按函数边界剔除分派函数（`执行单个动作`、`处理*动作`）与
//! `动作摘要`，并跳过注释和 `matches!` 模式，只统计真正的构造点；
//! 计数为 0 即判为不可达。
//!
//! # 为什么用源码扫描而不是反射
//!
//! Rust 没有稳定的枚举变体反射。这里直接读源码文本：解析 `state.rs` 里的
//! `pub enum UiAction` 块得到变体名，再在 `editor_ui` 与 `editor_app` 的
//! 全部 `.rs` 里数 `UiAction::<变体>` 的出现次数。解析刻意保持简单
//! （每行一个变体、以标识符开头），改动枚举格式时测试会直接失败提醒更新。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 仓库根（测试文件位于 `crates/editor_app/tests/`）。
fn 仓库根() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("从 crates/editor_app 上溯两级应当得到仓库根")
        .to_path_buf()
}

/// 解析 `UiAction` 的全部变体名。
fn 解析动作变体(源码: &str) -> Vec<String> {
    let mut 变体 = Vec::new();
    let mut 在块内 = false;
    // 结构体变体（`变体 { 字段: T }`）的字段行不能被当成变体名，
    // 因此用花括号深度把变体体内部整段跳过。
    let mut 变体体内 = 0usize;
    for 行 in 源码.lines() {
        if !在块内 {
            if 行.trim_start().starts_with("pub enum UiAction") {
                在块内 = true;
            }
            continue;
        }
        let trimmed = 行.trim();
        if 变体体内 > 0 {
            变体体内 += trimmed.matches('{').count();
            变体体内 -= trimmed.matches('}').count();
            continue;
        }
        // 枚举块结束。
        if trimmed == "}" {
            break;
        }
        let 去空白 = 行.trim_start();
        let 首字符 = 去空白.chars().next();
        let 是变体 = matches!(首字符, Some(c) if c.is_alphabetic() || c == '_');
        if !是变体 {
            continue;
        }
        let 名字: String = 去空白
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !名字.is_empty() && 名字 != "pub" {
            变体.push(名字);
        }
        // 单行内开的结构体变体：记录剩余深度。
        let 开 = 去空白.matches('{').count();
        let 闭 = 去空白.matches('}').count();
        if 开 > 闭 {
            变体体内 = 开 - 闭;
        }
    }
    变体
}

/// 分派函数把 `UiAction::变体` 写在**匹配模式**里，不是构造点。
///
/// 按名字识别而不是按文件：动作域拆分后处理函数会搬进独立文件，
/// 但"这些函数是分派器"这一事实不变。
fn 是分派函数(名字: &str) -> bool {
    名字 == "执行单个动作"
        || 名字 == "动作摘要"
        || (名字.starts_with("处理") && 名字.ends_with("动作"))
}

/// 解析行首定义的函数名（支持 `fn` / `pub fn` / `pub(crate) fn` / `pub(super) fn`）。
fn 顶层函数名(行: &str) -> Option<String> {
    if 行.starts_with(' ') || 行.starts_with('\t') {
        return None;
    }
    let 余下 = 行
        .strip_prefix("pub(crate) fn ")
        .or_else(|| 行.strip_prefix("pub(super) fn "))
        .or_else(|| 行.strip_prefix("pub fn "))
        .or_else(|| 行.strip_prefix("fn "))?;
    let 名字: String = 余下
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!名字.is_empty()).then_some(名字)
}

/// 整段剔除顶层的分派函数；顶层函数的收尾大括号在行首。
fn 剔除分派函数(源码: &str) -> String {
    let mut 结果 = String::new();
    let mut 在分派函数内 = false;
    for 行 in 源码.lines() {
        if 在分派函数内 {
            if 行 == "}" {
                在分派函数内 = false;
            }
            continue;
        }
        if let Some(名字) = 顶层函数名(行)
            && 是分派函数(&名字)
        {
            在分派函数内 = true;
            continue;
        }
        结果.push_str(行);
        结果.push('\n');
    }
    结果
}

/// 只保留可能包含**构造点**的源码行。
fn 可用于计数的源码(源码: &str) -> String {
    剔除分派函数(源码)
        .lines()
        .filter(|行| {
            // 注释里的示例与 `matches!` 里的模式都不算用户入口。
            !行.trim_start().starts_with("//") && !行.contains("matches!(")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 递归收集目录下的 `.rs` 文件内容。
fn 收集源码(目录: &Path, 结果: &mut Vec<String>) {
    let Ok(条目) = std::fs::read_dir(目录) else {
        return;
    };
    for 项 in 条目.flatten() {
        let 路径 = 项.path();
        if 路径.is_dir() {
            收集源码(&路径, 结果);
        } else if 路径.extension().is_some_and(|扩展| 扩展 == "rs")
            && let Ok(文本) = std::fs::read_to_string(&路径)
        {
            结果.push(可用于计数的源码(&文本));
        }
    }
}

#[test]
fn 每个动作变体都有构造点() {
    let 根 = 仓库根();
    let 状态文件 = 根.join("crates/editor_ui/src/state.rs");
    let 源码 = std::fs::read_to_string(&状态文件).expect("应当能读到 state.rs");
    let 变体 = 解析动作变体(&源码);
    assert!(
        变体.len() > 50,
        "变体解析失败了？只解析到 {} 个（UiAction 应当有上百个变体）",
        变体.len()
    );

    let mut 全部 = Vec::new();
    收集源码(&根.join("crates/editor_ui/src"), &mut 全部);
    收集源码(&根.join("crates/editor_app/src"), &mut 全部);
    let 全部 = 全部.join("\n");

    let mut 计数: BTreeMap<&str, usize> = BTreeMap::new();
    for 名字 in &变体 {
        let 模式 = format!("UiAction::{名字}");
        // 用简单的前缀计数：命中处必须紧跟非标识符字符，避免
        // `UiAction::播放` 把 `UiAction::播放动画` 也算进去。
        let mut 次数 = 0;
        let mut 起点 = 0;
        while let Some(偏移) = 全部[起点..].find(&模式) {
            let 结束 = 起点 + 偏移 + 模式.len();
            起点 = 结束;
            let 下一字符 = 全部[结束..].chars().next();
            let 是同一个词 = matches!(下一字符, Some(c) if c.is_alphanumeric() || c == '_');
            if !是同一个词 {
                次数 += 1;
            }
        }
        计数.insert(名字.as_str(), 次数);
    }

    // 0 次 = 分派与摘要之外没有任何构造点 = 用户无法触发。
    let 不可达: Vec<&str> = 计数
        .iter()
        .filter(|(_, 次数)| **次数 == 0)
        .map(|(名字, _)| *名字)
        .collect();
    assert!(
        不可达.is_empty(),
        "以下 UiAction 变体没有任何界面构造点（用户无法触发）：\n{不可达:#?}\n\
         修法：在界面里接上入口，或删掉这个变体。历史上这三种都犯过：\
         试听实体音频 / 移除最近项目 / 删除动画轨道。"
    );
}
