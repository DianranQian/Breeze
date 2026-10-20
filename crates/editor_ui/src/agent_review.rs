//! 文件变更审查的**纯逻辑**：归属判断、审查决策、状态合并。
//!
//! # 为什么不能把整份 `git diff` 当成 Agent 的成果
//!
//! 真实工作区里常常同时存在四类改动：
//!
//! 1. 任务开始前用户就已经改过的文件；
//! 2. 当前 Agent 任务产生的改动；
//! 3. 其他工作线（另一个编辑器窗口、脚本、构建工具）并发产生的改动；
//! 4. 用户在任务执行期间手动改的文件。
//!
//! 单看 `git diff` 无法区分它们。本模块用**任务开始时的基线**（脏文件集合 +
//! 内容指纹）回答"改动是不是在任务期间出现的"，再用工具调用里的写操作路径
//! 回答"是不是这个任务写的"。两者都没有证据时，一律标成
//! [`变更来源::来源未知`]，绝不冒认。
//!
//! # 拒绝为什么必须能失败
//!
//! 拒绝不是改一个标签：它要把文件恢复到任务开始前的状态。只有同时满足
//! "任务开始时文件是干净的"与"记录之后文件没有被再次改动"才允许恢复；
//! 否则必须报冲突并**保留现状**。判断在这里，真实 git 操作在
//! `editor_app::agent_worktree`。

use std::collections::BTreeMap;

use crate::agent::{
    任务基线, 变更来源, 变更种类, 基线文件, 审查状态, 文件变更, 现在秒, 验证状态
};

/// 一次工作区快照里的单个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 快照条目 {
    /// 项目相对路径。
    pub 路径: String,
    /// 重命名时的原路径。
    pub 原路径: Option<String>,
    /// `git status --porcelain` 的两字符状态。
    pub 状态: String,
    /// 内容指纹（文件不存在时为空串）。
    pub 指纹: String,
}

/// 由 git 状态码判断变更种类。
///
/// `XY` 两字符：`??` 未跟踪、`A` 新增、`D` 删除、`R` 重命名、`T` 类型变化，
/// 其余（`M`、`MM`、`AM`…）按修改处理。
pub fn 判断种类(状态: &str, 有原路径: bool) -> 变更种类 {
    let 状态 = 状态.trim();
    if 状态.is_empty() {
        return 变更种类::未知;
    }
    if 有原路径 || 状态.starts_with('R') || 状态.contains('R') {
        return 变更种类::重命名;
    }
    if 状态 == "??" || 状态.starts_with("A") || 状态.contains('A') {
        return 变更种类::新增;
    }
    if 状态.starts_with('D') || 状态.contains('D') {
        return 变更种类::删除;
    }
    if 状态.starts_with('T') || 状态.contains('T') {
        return 变更种类::类型变更;
    }
    if 状态.starts_with('M') || 状态.contains('M') {
        return 变更种类::修改;
    }
    变更种类::未知
}

/// 把基线、当前快照与工具写路径合并成审查列表。
///
/// `旧` 用于保留"指纹没变"的审查/验证结论：一次刷新不应把用户刚批准的文件
/// 又变回待审查。
pub fn 归类变更(
    基线: Option<&任务基线>,
    当前: &[快照条目],
    写路径: &[String],
    任务标识: &str,
    旧: &[文件变更],
) -> Vec<文件变更> {
    let 空基线;
    let 基线 = match 基线 {
        Some(基线) => 基线,
        None => {
            空基线 = 任务基线::default();
            &空基线
        }
    };
    let 时间 = 现在秒();
    let mut 结果 = Vec::new();
    for 条目 in 当前 {
        let 写命中 = 写路径
            .iter()
            .any(|路径| 路径 == &条目.路径 || 条目.原路径.as_deref().is_some_and(|原| 路径 == 原));
        let 基准 = 基线
            .文件
            .get(&条目.路径)
            .or_else(|| 条目.原路径.as_deref().and_then(|原| 基线.文件.get(原)));
        let 任务前已有 = 基准.is_some_and(|文件| 文件.指纹 == 条目.指纹);
        let (来源, 基线指纹, 可回退) = if 任务前已有 {
            // 内容与任务开始时一致：这是用户/其它工作线的旧改动，不属于本任务。
            (
                变更来源::任务前已有,
                基准.map(|文件| 文件.指纹.clone()),
                false,
            )
        } else if let Some(基准) = 基准 {
            // 任务开始时就是脏的，任务期间又被改了：无法安全恢复原内容。
            let 来源 = if 写命中 {
                变更来源::本任务
            } else {
                变更来源::任务期间非本任务
            };
            (来源, Some(基准.指纹.clone()), false)
        } else if 写命中 {
            // 任务开始时干净（不在脏文件集合里），且有写工具证据。
            (变更来源::本任务, None, true)
        } else {
            // 任务期间出现，但没有写工具证据：可能是用户手动或其它工作线。
            (变更来源::来源未知, None, false)
        };

        let 种类 = 判断种类(&条目.状态, 条目.原路径.is_some());
        // 已暂存的重命名（`R `）不能只靠工作区操作安全回退：那会需要改
        // 用户自己的 git 暂存区。明确标成"不可回退"，而不是冒险动索引。
        let 暂存重命名 = 种类 == 变更种类::重命名 && 条目.状态.starts_with('R');
        let 可回退 = 可回退 && !暂存重命名;
        let 旧项 = 旧
            .iter()
            .find(|项| 项.路径 == 条目.路径 && 项.当前指纹.as_deref() == Some(条目.指纹.as_str()));
        let (审查, 验证, 验证时间, 已验证指纹) = match 旧项 {
            Some(旧项) => (旧项.审查, 旧项.验证, 旧项.验证时间, 旧项.已验证指纹.clone()),
            None => (审查状态::待审查, 验证状态::未验证, None, None),
        };
        let 指纹 = if 条目.指纹.is_empty() {
            None
        } else {
            Some(条目.指纹.clone())
        };
        结果.push(文件变更 {
            路径: 条目.路径.clone(),
            原路径: 条目.原路径.clone(),
            状态: 条目.状态.clone(),
            种类,
            差异: String::new(),
            来源,
            任务标识: if 来源 == 变更来源::任务前已有 {
                None
            } else {
                Some(任务标识.to_string())
            },
            审查: if 来源 == 变更来源::任务前已有 {
                审查状态::已批准
            } else {
                审查
            },
            验证,
            基线指纹,
            当前指纹: 指纹,
            可回退,
            记录时间: 时间,
            验证时间,
            已验证指纹,
            备注: if 暂存重命名 {
                Some(
                    "重命名已在 git 索引中暂存：为避免改动用户暂存区，拒绝不会自动回退".to_string(),
                )
            } else if 来源 == 变更来源::任务期间非本任务 {
                Some("任务期间出现，但没有写工具记录：可能来自用户手动修改或其他工作线".to_string())
            } else if 来源 == 变更来源::来源未知 {
                Some("无法可靠归属：缺少任务前基线或写操作证据".to_string())
            } else if 来源 == 变更来源::本任务 && !可回退 {
                Some("任务开始时该文件已有改动：可以批准，但不能安全回退".to_string())
            } else {
                None
            },
        });
    }
    结果
}

/// 拒绝（回退）时要执行的真实动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 审查动作 {
    /// 保留现状（批准）。
    保留,
    /// 用 `HEAD` 内容写回文件。
    恢复文件,
    /// 删除本任务新建的文件。
    删除新增文件,
    /// 重命名回退：删除新路径并用 `HEAD` 恢复原路径。
    恢复重命名 {
        /// 原路径。
        原路径: String,
    },
}

/// 决定"拒绝"能否安全执行。
///
/// 返回 `Err` 时**不得改动文件**：调用方把变更标成
/// [`审查状态::冲突`] 或 [`审查状态::无法撤销`]，并保留现状。
pub fn 审查决策(
    变更: &文件变更, 当前指纹: Option<&str>
) -> Result<审查动作, String> {
    match 变更.审查 {
        审查状态::已批准 => return Err("该变更已批准，不能再次拒绝".to_string()),
        审查状态::已拒绝 => return Err("该变更已拒绝".to_string()),
        _ => {}
    }
    if 变更.来源 == 变更来源::任务前已有 {
        return Err("这是任务开始前就存在的改动，不属于本任务，拒绝不会动它".to_string());
    }
    if !变更.可回退 {
        return Err(变更
            .备注
            .clone()
            .unwrap_or_else(|| "缺少可安全恢复的基线，不能拒绝".to_string()));
    }
    let 记录 = 变更.当前指纹.as_deref();
    match (记录, 当前指纹) {
        (Some(记录), Some(当前)) if 记录 == 当前 => {}
        // 删除状态本身就是"文件不存在"：记录与现状都是 None 才算没被再次改动。
        (None, None) if 变更.种类 == 变更种类::删除 => {}
        (None, Some(_)) => {
            return Err("文件在记录之后被重新创建：拒绝会覆盖新的内容，已保留现状".to_string());
        }
        (Some(_), None) => {
            return Err("文件在记录之后被删除：拒绝不会恢复未确认的内容，已保留现状".to_string());
        }
        _ => {
            return Err("文件在记录之后又被修改过：拒绝会覆盖新的改动，已保留现状".to_string());
        }
    }
    if let Some(原路径) = &变更.原路径 {
        return Ok(审查动作::恢复重命名 {
            原路径: 原路径.clone(),
        });
    }
    if 变更.种类 == 变更种类::新增 {
        // 任务开始时干净且是新增：恢复 = 删除新建文件（不碰 HEAD）。
        return Ok(审查动作::删除新增文件);
    }
    Ok(审查动作::恢复文件)
}

/// 变更统计（真实计数，供界面摘要）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct 变更统计 {
    /// 新增数。
    pub 新增: usize,
    /// 修改数。
    pub 修改: usize,
    /// 删除数。
    pub 删除: usize,
    /// 重命名数。
    pub 重命名: usize,
    /// 待审查数（不含任务前已有）。
    pub 待审查: usize,
    /// 已验证数。
    pub 已验证: usize,
}

/// 统计一份变更列表。
pub fn 统计(变更: &[文件变更]) -> 变更统计 {
    let mut 结果 = 变更统计::default();
    for 项 in 变更 {
        match 项.种类 {
            变更种类::新增 => 结果.新增 += 1,
            变更种类::修改 => 结果.修改 += 1,
            变更种类::删除 => 结果.删除 += 1,
            变更种类::重命名 => 结果.重命名 += 1,
            _ => {}
        }
        if 项.审查 == 审查状态::待审查 && 项.来源 != 变更来源::任务前已有 {
            结果.待审查 += 1;
        }
        if 项.验证.通过() {
            结果.已验证 += 1;
        }
    }
    结果
}

/// 用真实验证结果更新变更列表。
///
/// 只有"验证时文件指纹 == 当前指纹"的变更才算已验证；之后文件再被改，
/// 验证结论自动失效（退回未验证）。
pub fn 应用验证结果(
    变更: &mut [文件变更],
    状态: 验证状态,
    验证时指纹: &BTreeMap<String, String>,
) {
    let 时间 = 现在秒();
    for 项 in 变更.iter_mut() {
        let 当前 = 验证时指纹.get(&项.路径);
        if 当前.is_some() && 当前 == 项.当前指纹.as_ref() {
            项.验证 = 状态;
            项.验证时间 = Some(时间);
            项.已验证指纹 = 项.当前指纹.clone();
        } else if 项.验证.通过() {
            // 验证时没看到这个文件，或内容已经不同：之前的"已验证"不再成立。
            项.验证 = 验证状态::未验证;
            项.验证时间 = None;
            项.已验证指纹 = None;
        }
    }
}

/// 刷新变更列表里的差异文本（保留已有审查/验证结论）。
pub fn 填入差异(变更: &mut [文件变更], 差异: impl Fn(&文件变更) -> String) {
    for 项 in 变更.iter_mut() {
        if 项.差异.is_empty() {
            项.差异 = 差异(项);
        }
    }
}

/// 由快照构造基线。
pub fn 建立基线(
    任务标识: impl Into<String>,
    头提交: impl Into<String>,
    快照: &[快照条目],
) -> 任务基线 {
    let mut 文件 = BTreeMap::new();
    for 条目 in 快照 {
        文件.insert(
            条目.路径.clone(),
            基线文件 {
                状态: 条目.状态.clone(),
                指纹: 条目.指纹.clone(),
            },
        );
    }
    任务基线 {
        任务标识: 任务标识.into(),
        采集时间: 现在秒(),
        头提交: 头提交.into(),
        文件,
    }
}

/// 指纹的短形式（界面显示用）。
pub fn 短指纹(指纹: Option<&str>) -> String {
    match 指纹 {
        Some(值) if 值.len() > 10 => 值.chars().take(10).collect(),
        Some(值) => 值.to_string(),
        None => "无".to_string(),
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 条目(路径: &str, 状态: &str, 指纹: &str) -> 快照条目 {
        快照条目 {
            路径: 路径.to_string(),
            原路径: None,
            状态: 状态.to_string(),
            指纹: 指纹.to_string(),
        }
    }

    fn 基线(条目: &[快照条目]) -> 任务基线 {
        建立基线("t1", "abc", 条目)
    }

    #[test]
    fn 任务前已有改动不会被认成智能体成果() {
        let 旧 = 条目("src/user.rs", " M", "hash-user");
        let 基线 = 基线(std::slice::from_ref(&旧));
        let 变更 = 归类变更(Some(&基线), &[旧], &[], "t1", &[]);
        assert_eq!(变更[0].来源, 变更来源::任务前已有);
        assert_eq!(变更[0].审查, 审查状态::已批准, "旧改动默认保持不动");
        assert!(!变更[0].可回退);
    }

    #[test]
    fn 任务期间新增且有写证据归本任务() {
        let 基线 = 基线(&[]);
        let 当前 = 条目("src/main.rs", " M", "hash-new");
        let 写 = vec!["src/main.rs".to_string()];
        let 变更 = 归类变更(Some(&基线), &[当前], &写, "t1", &[]);
        assert_eq!(变更[0].来源, 变更来源::本任务);
        assert!(变更[0].可回退, "任务前干净的文件可以恢复到 HEAD");
        assert_eq!(变更[0].种类, 变更种类::修改);
    }

    #[test]
    fn 没有写证据的并发改动标成来源未知() {
        let 基线 = 基线(&[]);
        let 变更 = 归类变更(
            Some(&基线),
            &[条目("src/other.rs", " M", "hash-x")],
            &[],
            "t1",
            &[],
        );
        assert_eq!(变更[0].来源, 变更来源::来源未知);
        assert!(!变更[0].可回退);
        assert!(
            变更[0]
                .备注
                .as_deref()
                .unwrap_or("")
                .contains("无法可靠归属")
        );
    }

    #[test]
    fn 基线脏文件在任务期间再改归属但不允许回退() {
        let 基线 = 基线(&[条目("src/lib.rs", " M", "old")]);
        let 变更 = 归类变更(
            Some(&基线),
            &[条目("src/lib.rs", " M", "new")],
            &["src/lib.rs".to_string()],
            "t1",
            &[],
        );
        assert_eq!(变更[0].来源, 变更来源::本任务);
        assert!(!变更[0].可回退, "没有保存任务前内容，不能安全回退");
        assert!(审查决策(&变更[0], Some("new")).is_err());
    }

    #[test]
    fn 拒绝在指纹变化时报冲突而不是覆盖() {
        let 基线 = 基线(&[]);
        let 写 = vec!["a.rs".to_string()];
        let 变更 = 归类变更(Some(&基线), &[条目("a.rs", " M", "hash-1")], &写, "t1", &[]);
        let 动作 = 审查决策(&变更[0], Some("hash-2"));
        assert!(动作.is_err(), "文件在记录后又被改，必须拒绝回退");
        assert!(动作.unwrap_err().contains("又被修改"));
    }

    #[test]
    fn 新增文件拒绝时删除而不是还原() {
        let 基线 = 基线(&[]);
        let 写 = vec!["new.rs".to_string()];
        let 变更 = 归类变更(
            Some(&基线),
            &[条目("new.rs", "??", "hash-new")],
            &写,
            "t1",
            &[],
        );
        assert_eq!(变更[0].种类, 变更种类::新增);
        assert_eq!(
            审查决策(&变更[0], Some("hash-new")),
            Ok(审查动作::删除新增文件)
        );
    }

    #[test]
    fn 审查结论在内容不变时保留() {
        let 基线 = 基线(&[]);
        let 写 = vec!["a.rs".to_string()];
        let mut 第一次 = 归类变更(Some(&基线), &[条目("a.rs", " M", "hash-1")], &写, "t1", &[]);
        第一次[0].审查 = 审查状态::已批准;
        第一次[0].验证 = 验证状态::构建通过;
        第一次[0].验证时间 = Some(1);
        第一次[0].已验证指纹 = Some("hash-1".to_string());
        let 第二次 = 归类变更(
            Some(&基线),
            &[条目("a.rs", " M", "hash-1")],
            &写,
            "t1",
            &第一次,
        );
        assert_eq!(第二次[0].审查, 审查状态::已批准);
        assert_eq!(第二次[0].验证, 验证状态::构建通过);

        let 第三次 = 归类变更(
            Some(&基线),
            &[条目("a.rs", " M", "hash-2")],
            &写,
            "t1",
            &第二次,
        );
        assert_eq!(第三次[0].审查, 审查状态::待审查, "内容变了要重新审查");
        assert_eq!(第三次[0].验证, 验证状态::未验证, "旧验证结论必须失效");
    }

    #[test]
    fn 验证结果只覆盖指纹一致的文件() {
        let 基线 = 基线(&[]);
        let 写 = vec!["a.rs".to_string(), "b.rs".to_string()];
        let mut 变更 = 归类变更(
            Some(&基线),
            &[条目("a.rs", " M", "hash-a"), 条目("b.rs", " M", "hash-b")],
            &写,
            "t1",
            &[],
        );
        let mut 指纹 = BTreeMap::new();
        指纹.insert("a.rs".to_string(), "hash-a".to_string());
        指纹.insert("b.rs".to_string(), "hash-other".to_string());
        应用验证结果(&mut 变更, 验证状态::构建通过, &指纹);
        assert_eq!(变更[0].验证, 验证状态::构建通过);
        assert_eq!(变更[1].验证, 验证状态::未验证);
    }

    #[test]
    fn 统计反映真实计数() {
        let 基线 = 基线(&[条目("old.rs", " M", "h0")]);
        let mut 变更 = 归类变更(
            Some(&基线),
            &[
                条目("old.rs", " M", "h0"),
                条目("new.rs", "??", "h1"),
                条目("gone.rs", " D", "h2"),
            ],
            &["new.rs".to_string(), "gone.rs".to_string()],
            "t1",
            &[],
        );
        变更[1].验证 = 验证状态::构建通过;
        let 统计 = 统计(&变更);
        assert_eq!(统计.新增, 1);
        assert_eq!(统计.删除, 1);
        assert_eq!(统计.待审查, 2, "任务前已有的旧改动不算待审查");
        assert_eq!(统计.已验证, 1);
    }

    #[test]
    fn 种类判断覆盖常见状态码() {
        assert_eq!(判断种类("??", false), 变更种类::新增);
        assert_eq!(判断种类("A ", false), 变更种类::新增);
        assert_eq!(判断种类(" M", false), 变更种类::修改);
        assert_eq!(判断种类("MM", false), 变更种类::修改);
        assert_eq!(判断种类(" D", false), 变更种类::删除);
        assert_eq!(判断种类("R ", true), 变更种类::重命名);
        assert_eq!(判断种类("T ", false), 变更种类::类型变更);
        assert_eq!(判断种类("", false), 变更种类::未知);
    }
}
