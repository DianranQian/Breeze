//! 中文名：哨兵与域文件变体同步校验
//! 校验 `actions/mod.rs` 里每组 `unreachable!` 哨兵登记的变体清单，与对应域文件
//! `处理X动作` 里实际认领的变体清单**完全一致**。
//!
//! # 为什么需要这条测试
//!
//! `actions/` 按域拆分后，同一个变体名要写在两个地方：
//!
//! 1. 域文件的 `match 动作` 里，真正处理它的臂；
//! 2. `mod.rs` 的分组哨兵里，为了让总表保持穷尽性而保留的 `unreachable!` 臂。
//!
//! 编译器拦不住这两种手滑：
//!
//! * **只加域臂、忘加哨兵** —— 该动作会落到总表里没人认领的位置；若总表恰好没有
//!   兜底分支就编译报错，但一旦有人为了省事加了 `_ => {}`，就变成静默吞掉。
//! * **只改哨兵、忘改域臂** —— 编译通过、测试可能也过，直到用户点那个入口才在
//!   运行时 panic（哨兵里的 `unreachable!` 其实并非不可达）。
//!
//! 所以这里把「两份清单必须逐字相等」变成硬约束，让它在 `cargo test` 阶段失败。
//!
//! # 为什么用源码扫描而不是反射
//!
//! Rust 没有稳定的枚举变体反射，也拿不到「某个 match 臂属于哪个函数」。这里直接读
//! 源码文本：哨兵按 `unreachable!("<域>动作应已由 \`处理X动作\` 处理")` 的固定措辞
//! 识别；域文件按顶层 `match 动作 {` 的花括号深度取出臂的模式行，从而**只看模式、
//! 不看函数体**——函数体里 `UiAction::Y` 出现在构造点（例如设置域重新推
//! `UiAction::打开项目`），把它当认领会假阳性。
//!
//! 解析刻意保持简单（依赖 rustfmt 的「臂收尾 `}` 在行首」等格式约定）。改动 match
//! 的书写格式时若测试失败，请按错误信息更新本测试，而不是放宽断言。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 仓库根（测试文件位于 `crates/editor_app/tests/`）。
fn 仓库根() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("从 crates/editor_app 上溯两级应当得到仓库根")
        .to_path_buf()
}

/// 把一行里**不参与语法结构**的部分去掉：行注释、字符串字面量、字符字面量。
/// 数花括号深度时必须用这个结果，否则 `format!("{}")` 里的括号会算错深度。
fn 结构文本(行: &str) -> String {
    let 字符: Vec<char> = 行.chars().collect();
    let mut 结果 = String::new();
    let mut 下标 = 0usize;
    while 下标 < 字符.len() {
        let 当前 = 字符[下标];
        if 当前 == '/' && 字符.get(下标 + 1) == Some(&'/') {
            break;
        }
        if 当前 == '"' {
            下标 += 1;
            while 下标 < 字符.len() {
                match 字符[下标] {
                    '\\' => 下标 += 2,
                    '"' => {
                        下标 += 1;
                        break;
                    }
                    _ => 下标 += 1,
                }
            }
            continue;
        }
        if 当前 == '\'' {
            // 生命周期标注 `'a` 不是字符字面量；真正的字符字面量形如 `'x'` / `'\n'`。
            let 结束 = 下标
                + if 字符.get(下标 + 1) == Some(&'\\') {
                    4
                } else {
                    3
                };
            if 字符.get(结束) == Some(&'\'') {
                下标 = 结束 + 1;
                continue;
            }
        }
        结果.push(当前);
        下标 += 1;
    }
    结果
}

/// 取出文本中出现的所有 `UiAction::<变体名>`，名字后必须紧跟非标识符字符。
fn 变体名(文本: &str) -> Vec<String> {
    const 前缀: &str = "UiAction::";
    let mut 结果 = Vec::new();
    let mut 起点 = 0usize;
    while let Some(偏移) = 文本[起点..].find(前缀) {
        let 名字起 = 起点 + 偏移 + 前缀.len();
        let 尾部 = &文本[名字起..];
        let 名字: String = 尾部
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let 长度 = 名字.len();
        if 长度 > 0 {
            结果.push(名字);
        }
        起点 = 名字起 + 长度;
    }
    结果
}

/// 从 `unreachable!("<域>动作应已由 \`处理X动作\` 处理")` 里取出反引号中的函数名。
/// 措辞不符（例如总表某个臂的正常 `unreachable!`）返回 `None`。
fn 哨兵指认的函数(行: &str) -> Option<String> {
    let 正文 = 行.strip_prefix("unreachable!(")?.trim();
    let 正文 = 正文.strip_prefix('"')?;
    if !正文.contains("动作应已由") {
        return None;
    }
    let 后引号 = 正文.find('`')?;
    let 余下 = &正文[后引号 + 1..];
    let 函数名 = &余下[..余下.find('`')?];
    (!函数名.is_empty()).then(|| 函数名.to_string())
}

/// 提取 `mod.rs` 里每组哨兵：`(处理函数名, 该组登记的变体集合)`。
///
/// 状态机：`空闲` 见到以 `UiAction::` 开头的行开始收集模式；模式一路累加到出现
/// `=>` 的那一行；紧接着的下一非空行若是指认哨兵，就收组，否则整段作废
/// （那是总表里一条真正处理动作的臂）。
fn 哨兵分组(源码: &str) -> Vec<(String, BTreeSet<String>)> {
    #[derive(PartialEq)]
    enum 状态 {
        空闲,
        收集模式,
        看臂体,
    }
    let mut 结果 = Vec::new();
    let mut 状态 = 状态::空闲;
    let mut 模式 = String::new();
    for 原始行 in 源码.lines() {
        let 行 = 原始行.trim();
        if 行.is_empty() {
            continue;
        }
        match 状态 {
            状态::空闲 => {
                if 行.starts_with("UiAction::") {
                    模式 = 行.to_string();
                    状态 = if 模式.contains("=>") {
                        状态::看臂体
                    } else {
                        状态::收集模式
                    };
                }
            }
            状态::收集模式 => {
                模式.push(' ');
                模式.push_str(行);
                if 行.contains("=>") {
                    状态 = 状态::看臂体;
                }
            }
            状态::看臂体 => {
                if let Some(函数名) = 哨兵指认的函数(行) {
                    结果.push((函数名, 变体名(&模式).into_iter().collect()));
                }
                模式.clear();
                状态 = 状态::空闲;
            }
        }
    }
    结果
}

/// 提取一个域文件里 `处理X动作` 顶层 `match 动作 { … }` 各臂模式中的变体名。
///
/// 只认第一处 `match 动作 {`（即入口那个），靠花括号深度把函数体内的嵌套 match
/// 与臂体整段跳过；返回 `(处理函数名, 认领的变体集合)`。
fn 域内认领(源码: &str, 文件名: &str) -> (String, BTreeSet<String>) {
    let mut 函数名 = String::new();
    let mut 起点 = None;
    for (序号, 原始行) in 源码.lines().enumerate() {
        let 行 = 原始行.trim();
        if let Some(余下) = 行.strip_prefix("pub(super) fn ")
            && 余下.starts_with("处理")
            && 余下.ends_with("动作(")
        {
            函数名 = 余下[..余下.len() - 1].to_string();
            起点 = Some(序号);
            break;
        }
    }
    let 起点 = 起点.unwrap_or_else(|| panic!("{文件名} 里找不到 `pub(super) fn 处理…动作(` 定义"));

    let mut 认领 = BTreeSet::new();
    let mut 深度 = 0i32;
    let mut 已进入 = false;
    for 原始行 in 源码.lines().skip(起点) {
        let 行 = 原始行.trim();
        if !已进入 {
            if 行 == "match 动作 {" {
                已进入 = true;
                深度 = 1;
            }
            continue;
        }
        if 深度 == 1 {
            if 行 == "}" {
                break;
            }
            if 行.starts_with("UiAction::") || 行.starts_with('|') {
                let 模式段 = 行.split("=>").next().unwrap_or(行);
                认领.extend(变体名(模式段));
            }
        }
        let 结构 = 结构文本(行);
        深度 += 结构.matches('{').count() as i32 - 结构.matches('}').count() as i32;
    }
    assert!(已进入, "{文件名} 里找不到入口 `match 动作 {{`");
    (函数名, 认领)
}

#[test]
fn 每个哨兵组的变体清单与域文件处理的完全一致() {
    let 根 = 仓库根();
    let 动作目录 = 根.join("crates/editor_app/src/actions");
    let 分派文件 = 动作目录.join("mod.rs");
    let 分派源码 = std::fs::read_to_string(&分派文件).expect("应当能读到 actions/mod.rs");
    let 分组 = 哨兵分组(&分派源码);
    assert!(
        分组.len() >= 10,
        "哨兵解析失败了？只取到 {} 组（当前每个已拆出的域各有一组）",
        分组.len()
    );

    // 域文件清单：目录里除 mod.rs 外的每个 .rs 都应有一个 `处理X动作`。
    let mut 域文件: Vec<std::path::PathBuf> = std::fs::read_dir(&动作目录)
        .expect("应当能列出 actions/ 目录")
        .flatten()
        .map(|项| 项.path())
        .filter(|路径| 路径.extension().is_some_and(|扩展| 扩展 == "rs"))
        .filter(|路径| 路径.file_name().is_some_and(|名| 名 != "mod.rs"))
        .collect();
    域文件.sort();
    assert!(
        !域文件.is_empty(),
        "actions/ 里除 mod.rs 外没有域文件？拆分被回退了？"
    );

    // 1) 函数名：哨兵指认的函数集合 == 域文件里定义的函数集合。
    let 哨兵函数: BTreeSet<String> = 分组.iter().map(|(名, _)| 名.clone()).collect();
    let 域函数: BTreeSet<String> = 域文件
        .iter()
        .map(|路径| {
            let 名 = 路径.file_name().unwrap().to_string_lossy().to_string();
            域内认领(
                &std::fs::read_to_string(路径).unwrap_or_else(|_| panic!("读不到 {名}")),
                &名,
            )
            .0
        })
        .collect();
    assert_eq!(
        域函数.len(),
        域文件.len(),
        "两个域文件里的处理函数重名：{域函数:?}（域文件 {} 个）",
        域文件.len()
    );
    let 缺哨兵: Vec<&String> = 域函数.iter().filter(|名| !哨兵函数.contains(*名)).collect();
    let 多哨兵: Vec<&String> = 哨兵函数.iter().filter(|名| !域函数.contains(*名)).collect();
    assert!(
        缺哨兵.is_empty() && 多哨兵.is_empty(),
        "哨兵与域文件的对应关系断了。\n\
         有处理函数但 mod.rs 里没有对应哨兵组（该域漏登记哨兵）：{缺哨兵:?}\n\
         哨兵指认了不存在的处理函数（域文件改名/删除，或 unreachable! 措辞被改）：{多哨兵:?}"
    );

    // 2) 变体集合：逐组比对，两侧差集都必须为空。
    let mut 不匹配组 = Vec::new();
    let mut 全部认领: BTreeSet<String> = BTreeSet::new();
    let mut 重复认领 = Vec::new();
    for (函数名, 哨兵集) in &分组 {
        let 路径 = 域文件
            .iter()
            .find(|路径| {
                let 文本 = std::fs::read_to_string(路径).unwrap_or_default();
                域内认领(&文本, &路径.file_name().unwrap().to_string_lossy()).0 == *函数名
            })
            .unwrap_or_else(|| panic!("找不到定义了 {函数名} 的域文件"));
        let 文本 = std::fs::read_to_string(路径).expect("应当能读到域文件");
        let (_, 域集) = 域内认领(&文本, &路径.file_name().unwrap().to_string_lossy());
        assert!(!域集.is_empty(), "{函数名} 一个变体都没认领？解析失效了");

        let 只在哨兵: Vec<&String> = 哨兵集.difference(&域集).collect();
        let 只在域: Vec<&String> = 域集.difference(哨兵集).collect();
        if !只在哨兵.is_empty() || !只在域.is_empty() {
            不匹配组.push(format!(
                "{函数名}（{}）\n  只在哨兵里（域文件没有对应臂，触发即 panic）：{只在哨兵:?}\n  只在域文件里（mod.rs 漏登记哨兵，可能落到总表）：{只在域:?}",
                路径.file_name().unwrap().to_string_lossy()
            ));
        }

        // 3) 一个变体不该出现在两个域：早到的那个域会吃掉它，另一份清单是死字。
        for 名 in &域集 {
            if !全部认领.insert(名.clone()) {
                重复认领.push(format!("{函数名} 也认领了 {名}"));
            }
        }
    }
    assert!(
        不匹配组.is_empty(),
        "以下域的哨兵清单与域文件实际处理的臂不一致：\n{}\n\
         修法：把变体同时加/删到两侧。历史上「处理逻辑写完了、界面上没有入口」\
         已经犯过三次（见 action_reachability.rs 顶部说明）。",
        不匹配组.join("\n")
    );
    assert!(
        重复认领.is_empty(),
        "同一个 UiAction 变体被两个域认领（后者永不执行，清单是死字）：\n{}",
        重复认领.join("\n")
    );

    // 4) 总量兜底：解析规模对得上，防止「两边同时解析失败而互相吻合」。
    assert!(
        全部认领.len() > 100,
        "全域认领的变体总数只有 {}，解析规模异常",
        全部认领.len()
    );
}
