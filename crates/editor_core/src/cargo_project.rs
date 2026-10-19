//! 原生 Cargo / Bevy 工程识别。
//!
//! # 为什么需要这一层
//!
//! 微风最初只认识自己的项目格式：根目录下有 `project.ron`。但**原生 Bevy 项目**
//! 的标准形态就是一个普通 Rust/Cargo 工程 —— 没有 `project.ron`、
//! 没有 `scenes/`、也没有微风的资源导入清单。用户拿一个真实的 Bevy 工程
//! 去「打开项目」，旧实现直接报"未找到 project.ron"，整个工程打不开。
//!
//! 这一层的职责只有一个：**只读地**从真实的 Cargo 结构里回答"这是什么工程、
//! 有哪些包、哪些可执行目标"。它绝不写用户的 `Cargo.toml`，也不假设目录名。
//!
//! # 识别的依据是 Cargo 契约，不是目录名
//!
//! | 依据 | 含义 |
//! | --- | --- |
//! | `[package]` | 这个清单本身是一个包 |
//! | `[workspace]` + `members` | 这是工作区，游戏目标可能在某个成员里 |
//! | `[[bin]]` / `src/main.rs` / `src/bin/*.rs` | 真正的可执行目标 |
//! | `src/lib.rs` | 只有库目标（**不能**当成可运行游戏） |
//! | `[dependencies].bevy`（含 `workspace = true`） | 是不是 Bevy 工程 |
//!
//! # 为什么静态解析而不是 `cargo metadata`
//!
//! `cargo metadata` 更权威，但它要启动子进程、依赖工具链、在大型工作区上要
//! 几百毫秒甚至更久，而且**打开项目**是第一交互路径。静态解析只读几个
//! `Cargo.toml`，成本可以忽略；识别错了也只是"默认目标选得不够好"，
//! 用户仍可手动选择，不会阻塞打开。真正的构建/运行仍然交给 cargo 自己。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Cargo 清单文件名。
pub const 原生清单文件名: &str = "Cargo.toml";

/// Cargo 工程识别失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum Cargo识别错误 {
    /// 目录下没有 Cargo.toml。
    #[error("`{0}` 下没有 Cargo.toml，无法作为 Cargo 工程打开")]
    缺少清单(PathBuf),

    /// 清单读取失败。
    #[error("读取 Cargo.toml 失败：{} —— {来源}", 路径.display())]
    读取失败 {
        /// 出错的清单路径。
        路径: PathBuf,
        /// 底层 IO 错误。
        #[source]
        来源: std::io::Error,
    },

    /// 清单无法解析为 TOML。
    #[error("解析 Cargo.toml 失败：{} —— {来源}", 路径.display())]
    解析失败 {
        /// 出错的清单路径。
        路径: PathBuf,
        /// 底层 TOML 错误。
        #[source]
        来源: Box<toml::de::Error>,
    },

    /// `[package]` 缺少 `name`。
    #[error("Cargo.toml 的 [package] 缺少 name：{}", .0.display())]
    缺少包名(PathBuf),

    /// 工作区没有解析出任何成员包。
    #[error("`{0}` 是 Cargo 工作区，但没有解析出任何成员包（检查 [workspace] members）")]
    无成员(PathBuf),
}

/// Cargo 识别结果。
pub type Cargo结果<T> = Result<T, Cargo识别错误>;

/// 一个可执行目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cargo可执行目标 {
    /// 所属包名。
    pub 包名: String,
    /// 目标名（`cargo run --bin <名称>` 用的就是它）。
    pub 名称: String,
    /// 所属包根目录。
    pub 包根: PathBuf,
    /// 目标源文件的绝对路径（能确定时）。构建诊断可以据此关联到真实文件。
    pub 源文件: Option<PathBuf>,
}

impl Cargo可执行目标 {
    /// 供界面显示的一行标识：`包名::目标名`。
    pub fn 标识(&self) -> String {
        format!("{}::{}", self.包名, self.名称)
    }
}

/// 工作区里的一个包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cargo包 {
    /// 包名（`[package] name`）。
    pub 名称: String,
    /// 版本（缺省时为空）。
    pub 版本: String,
    /// 包根目录（绝对路径）。
    pub 根目录: PathBuf,
    /// 包清单路径（绝对路径）。
    pub 清单路径: PathBuf,
    /// 这个包是不是工作区根目录里的 `[package]`。
    pub 是工作区根: bool,
    /// 是否依赖 bevy（含 `workspace = true` 继承）。
    pub 依赖贝维: bool,
    /// 库目标名（有 `src/lib.rs` 或 `[lib]` 时）。
    pub 库名: Option<String>,
    /// 可执行目标（按"最可能是游戏主程序"的顺序排）。
    pub 可执行目标: Vec<Cargo可执行目标>,
}

/// 一个已识别的 Cargo 工程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cargo工程 {
    /// 工程根目录（绝对路径）。
    pub 根目录: PathBuf,
    /// 根清单路径。
    pub 清单路径: PathBuf,
    /// 根清单里是否声明了 `[workspace]`。
    pub 是工作区: bool,
    /// 解析出的包（根包在前，成员按声明顺序）。
    pub 包: Vec<Cargo包>,
    /// 声明了但找不到清单的成员（诊断用，不影响打开）。
    pub 缺失成员: Vec<String>,
}

impl Cargo工程 {
    /// 从工程根目录识别。**只读**：不会写任何文件。
    pub fn 读取(根目录: impl AsRef<Path>) -> Cargo结果<Self> {
        let 根目录 = 根目录.as_ref().to_path_buf();
        let 清单路径 = 根目录.join(原生清单文件名);
        if !清单路径.is_file() {
            return Err(Cargo识别错误::缺少清单(根目录));
        }

        let 根表 = 读清单(&清单路径)?;
        let 工作区表 = 根表.get("workspace").and_then(|值| 值.as_table());
        let 是工作区 = 工作区表.is_some();
        // `bevy.workspace = true` 的版本由根的 `[workspace.dependencies]` 决定。
        let 工作区依赖 = 工作区表
            .and_then(|表| 表.get("dependencies"))
            .and_then(|值| 值.as_table())
            .cloned()
            .unwrap_or_default();

        let mut 包 = Vec::new();
        let mut 缺失成员 = Vec::new();

        // 根清单自己可能就是包（workspace + package 的常见形态）。
        if 根表.get("package").is_some() {
            包.push(解析包(&根目录, &根表, &工作区依赖, true)?);
        }

        if let Some(成员列表) = 工作区表
            .and_then(|表| 表.get("members"))
            .and_then(|值| 值.as_array())
        {
            for 项 in 成员列表 {
                let Some(模式) = 项.as_str() else {
                    continue;
                };
                let 命中 = 展开成员(&根目录, 模式);
                if 命中.is_empty() {
                    缺失成员.push(模式.to_string());
                    continue;
                }
                for 成员根 in 命中 {
                    if 成员根 == 根目录 {
                        // 根包上面已经收过了。
                        continue;
                    }
                    let 成员清单 = 成员根.join(原生清单文件名);
                    if !成员清单.is_file() {
                        缺失成员.push(模式.to_string());
                        continue;
                    }
                    let 表 = 读清单(&成员清单)?;
                    if 表.get("package").is_none() {
                        // 成员清单本身又是个虚拟工作区：跳过（不递归，
                        // 嵌套工作区不是 Cargo 支持的常规形态）。
                        continue;
                    }
                    包.push(解析包(&成员根, &表, &工作区依赖, false)?);
                }
            }
        }

        if 包.is_empty() {
            return Err(Cargo识别错误::无成员(根目录));
        }

        Ok(Self {
            根目录,
            清单路径,
            是工作区,
            包,
            缺失成员,
        })
    }

    /// 全部可执行目标。
    pub fn 全部可执行目标(&self) -> Vec<&Cargo可执行目标> {
        self.包.iter().flat_map(|包| 包.可执行目标.iter()).collect()
    }

    /// 是不是 Bevy 工程（任一包依赖 bevy）。
    pub fn 是否贝维工程(&self) -> bool {
        self.包.iter().any(|包| 包.依赖贝维)
    }

    /// 默认包：优先"根包 + 有可执行目标"，其次"依赖 bevy + 有可执行目标"，
    /// 最后才是第一个有可执行目标的成员。
    ///
    /// 这条顺序对应真实的工程形态：单包工程取自己；`workspace + package`
    /// 的工程取根包；纯虚拟工作区取第一个能跑的成员。**只有库目标的包不会
    /// 被选中** —— 库不是游戏，把它当成运行目标只会给出一个必然失败的入口。
    pub fn 默认包(&self) -> Option<&Cargo包> {
        let 可运行 = |包: &&Cargo包| !包.可执行目标.is_empty();
        self.包
            .iter()
            .filter(可运行)
            .find(|包| 包.是工作区根)
            .or_else(|| self.包.iter().filter(可运行).find(|包| 包.依赖贝维))
            .or_else(|| self.包.iter().find(可运行))
    }

    /// 默认可执行目标。
    pub fn 默认可执行目标(&self) -> Option<&Cargo可执行目标> {
        self.默认包().and_then(|包| 包.可执行目标.first())
    }

    /// 是否有任何可运行目标。
    pub fn 有可运行目标(&self) -> bool {
        self.默认可执行目标().is_some()
    }

    /// 一份人类可读的摘要（用于控制台/日志）。
    pub fn 摘要(&self) -> String {
        let 目标数 = self.全部可执行目标().len();
        if self.是工作区 {
            format!(
                "Cargo 工作区：{} 个成员包，{} 个可执行目标",
                self.包.len(),
                目标数
            )
        } else {
            format!("Cargo 包：{} 个可执行目标", 目标数)
        }
    }
}

/// 读取并解析一个 Cargo 清单。
fn 读清单(路径: &Path) -> Cargo结果<toml::Table> {
    let 文本 = std::fs::read_to_string(路径).map_err(|来源| Cargo识别错误::读取失败 {
        路径: 路径.to_path_buf(),
        来源,
    })?;
    toml::from_str::<toml::Table>(&文本).map_err(|来源| Cargo识别错误::解析失败 {
        路径: 路径.to_path_buf(),
        来源: Box::new(来源),
    })
}

/// 解析一个包的清单。
fn 解析包(
    包根: &Path,
    表: &toml::Table,
    工作区依赖: &toml::Table,
    是工作区根: bool,
) -> Cargo结果<Cargo包> {
    let 清单路径 = 包根.join(原生清单文件名);
    let 包表 = 表.get("package").and_then(|值| 值.as_table());
    let 名称 = 包表
        .and_then(|表| 表.get("name"))
        .and_then(|值| 值.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if 名称.is_empty() {
        return Err(Cargo识别错误::缺少包名(清单路径));
    }

    // 版本可以继承工作区（`version.workspace = true`，值是一个内联表）。
    let 版本 = 包表
        .and_then(|表| 表.get("version"))
        .and_then(|值| 值.as_str())
        .unwrap_or("")
        .to_string();

    let 依赖贝维 = 是否依赖贝维(表, 工作区依赖);

    let 默认库名 = 名称.replace('-', "_");
    let 库名 = if let Some(库表) = 表.get("lib").and_then(|值| 值.as_table()) {
        Some(
            库表
                .get("name")
                .and_then(|值| 值.as_str())
                .unwrap_or(&默认库名)
                .to_string(),
        )
    } else if 包根.join("src").join("lib.rs").is_file() {
        Some(默认库名)
    } else {
        None
    };

    let 可执行目标 = 收集可执行目标(包根, &名称, 表, 包表);

    Ok(Cargo包 {
        名称,
        版本,
        根目录: 包根.to_path_buf(),
        清单路径,
        是工作区根,
        依赖贝维,
        库名,
        可执行目标,
    })
}

/// 收集一个包的可执行目标。
///
/// 顺序刻意安排：`src/main.rs`（包的主程序）在**最前**，
/// 然后是显式 `[[bin]]`，最后才是 `src/bin/*.rs` 的自动发现。
/// `默认运行目标` 取第一个，因此单包工程天然落在主程序上。
fn 收集可执行目标(
    包根: &Path,
    包名: &str,
    表: &toml::Table,
    包表: Option<&toml::Table>,
) -> Vec<Cargo可执行目标> {
    let mut 目标 = Vec::new();
    let mut 已见: BTreeSet<String> = BTreeSet::new();
    // `autobins = false` 时 Cargo 不再自动发现 `src/main.rs` / `src/bin/*`。
    let 自动发现 = 包表
        .and_then(|表| 表.get("autobins"))
        .and_then(|值| 值.as_bool())
        .unwrap_or(true);

    let 源文件 = 包根.join("src").join("main.rs");
    if 自动发现 && 源文件.is_file() {
        已见.insert(包名.to_string());
        目标.push(Cargo可执行目标 {
            包名: 包名.to_string(),
            名称: 包名.to_string(),
            包根: 包根.to_path_buf(),
            源文件: Some(源文件),
        });
    }

    if let Some(数组) = 表.get("bin").and_then(|值| 值.as_array()) {
        for 项 in 数组 {
            let 名称 = 项
                .get("name")
                .and_then(|值| 值.as_str())
                .unwrap_or(包名)
                .trim()
                .to_string();
            if 名称.is_empty() || !已见.insert(名称.clone()) {
                continue;
            }
            let 源文件 = 项
                .get("path")
                .and_then(|值| 值.as_str())
                .map(|相对| 包根.join(相对))
                .filter(|路径| 路径.is_file())
                .or_else(|| {
                    // Cargo 的默认推断：`src/bin/<name>.rs` 或 `src/main.rs`。
                    let 候选 = 包根.join("src").join("bin").join(format!("{名称}.rs"));
                    候选.is_file().then_some(候选)
                });
            目标.push(Cargo可执行目标 {
                包名: 包名.to_string(),
                名称,
                包根: 包根.to_path_buf(),
                源文件,
            });
        }
    }

    if 自动发现 {
        let bin目录 = 包根.join("src").join("bin");
        if let Ok(读取) = std::fs::read_dir(&bin目录) {
            let mut 候选: Vec<PathBuf> = 读取
                .flatten()
                .map(|项| 项.path())
                .filter(|路径| 路径.extension().is_some_and(|扩展| 扩展 == "rs"))
                .collect();
            候选.sort();
            for 文件 in 候选 {
                let Some(名称) = 文件
                    .file_stem()
                    .and_then(|名| 名.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                if !已见.insert(名称.clone()) {
                    continue;
                }
                目标.push(Cargo可执行目标 {
                    包名: 包名.to_string(),
                    名称,
                    包根: 包根.to_path_buf(),
                    源文件: Some(文件),
                });
            }
        }
    }

    目标
}

/// 判断包的清单是否依赖 bevy。
///
/// 覆盖三种真实写法：
/// * `bevy = "0.20"` / `bevy = { version = ... }`；
/// * `[dependencies.bevy]`（等价写法，TOML 层面同一个键）；
/// * `bevy = { workspace = true }` → 查根的 `[workspace.dependencies]`。
fn 是否依赖贝维(表: &toml::Table, 工作区依赖: &toml::Table) -> bool {
    for 段 in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(依赖表) = 表.get(段).and_then(|值| 值.as_table()) else {
            continue;
        };
        let Some(贝维) = 依赖表.get("bevy") else {
            continue;
        };
        let 继承工作区 = 贝维
            .as_table()
            .and_then(|表| 表.get("workspace"))
            .and_then(|值| 值.as_bool())
            .unwrap_or(false);
        if 继承工作区 {
            if 工作区依赖.get("bevy").is_some() {
                return true;
            }
            continue;
        }
        return true;
    }
    // `[target.'cfg(...)'.dependencies]` 里声明 bevy 也应当算。
    if let Some(目标表) = 表.get("target").and_then(|值| 值.as_table()) {
        for (_, 配置) in 目标表.iter() {
            let Some(依赖表) = 配置.get("dependencies").and_then(|值| 值.as_table()) else {
                continue;
            };
            if 依赖表.get("bevy").is_some() {
                return true;
            }
        }
    }
    false
}

/// 展开 `[workspace] members` 里的一条声明。
///
/// 支持 Cargo 允许的目录通配（`crates/*`、`games/*/engine`）。只处理
/// 单层 `*`：`**` 在实践中极罕见，遇到时退化成"不展开"而不是报错。
fn 展开成员(根: &Path, 模式: &str) -> Vec<PathBuf> {
    let 归一 = 模式.replace('\\', "/");
    let 段: Vec<&str> = 归一
        .split('/')
        .filter(|段| !段.is_empty() && *段 != ".")
        .collect();
    if 段.is_empty() {
        return Vec::new();
    }

    let mut 当前 = vec![根.to_path_buf()];
    for 段 in 段 {
        let mut 下一批 = Vec::new();
        for 目录 in &当前 {
            if !段.contains('*') {
                let 候选 = 目录.join(段);
                if 候选.is_dir() {
                    下一批.push(候选);
                }
                continue;
            }
            let Ok(读取) = std::fs::read_dir(目录) else {
                continue;
            };
            let mut 名字: Vec<String> = 读取
                .flatten()
                .filter(|项| 项.path().is_dir())
                .filter_map(|项| 项.file_name().into_string().ok())
                .collect();
            // 排序保证成员顺序稳定（不随文件系统枚举顺序抖动）。
            名字.sort();
            for 名 in 名字 {
                if 通配匹配(&名, 段) {
                    下一批.push(目录.join(名));
                }
            }
        }
        当前 = 下一批;
        if 当前.is_empty() {
            break;
        }
    }
    当前
}

/// 单层路径段通配匹配（支持 `*`、`foo*`、`*bar`、`foo*bar`）。
fn 通配匹配(名: &str, 模式: &str) -> bool {
    if !模式.contains('*') {
        return 名 == 模式;
    }
    let 片: Vec<&str> = 模式.split('*').collect();
    let 首 = 片[0];
    let 尾 = 片[片.len() - 1];
    if !名.starts_with(首) || !名.ends_with(尾) {
        return false;
    }
    if 名.len() < 首.len() + 尾.len() {
        // 前后缀重叠时，只有在没有中间片段的情况下才可能成立。
        return 片.len() <= 2 && 名.len() >= 首.len().max(尾.len());
    }
    let mut 余 = &名[首.len()..名.len() - 尾.len()];
    for 中间 in &片[1..片.len().saturating_sub(1)] {
        if 中间.is_empty() {
            continue;
        }
        match 余.find(中间) {
            Some(位置) => 余 = &余[位置 + 中间.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 写(路径: &Path, 文本: &str) {
        if let Some(父) = 路径.parent() {
            std::fs::create_dir_all(父).unwrap();
        }
        std::fs::write(路径, 文本).unwrap();
    }

    #[test]
    fn 单包工程识别主程序() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            r#"
[package]
name = "my_game"
version = "0.1.0"

[dependencies]
bevy = "0.20"
"#,
        );
        写(&根.join("src/main.rs"), "fn main() {}");

        let 工程 = Cargo工程::读取(根).unwrap();
        assert!(!工程.是工作区);
        assert!(工程.是否贝维工程());
        let 目标 = 工程.默认可执行目标().unwrap();
        assert_eq!(目标.包名, "my_game");
        assert_eq!(目标.名称, "my_game");
        assert!(目标.源文件.as_ref().unwrap().ends_with("main.rs"));
    }

    #[test]
    fn 只有库目标时没有可运行目标() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            r#"
[package]
name = "my_lib"
version = "0.1.0"
"#,
        );
        写(&根.join("src/lib.rs"), "");

        let 工程 = Cargo工程::读取(根).unwrap();
        assert!(工程.默认包().is_none(), "库不是可运行游戏");
        assert!(!工程.有可运行目标());
        assert_eq!(工程.包[0].库名.as_deref(), Some("my_lib"));
    }

    #[test]
    fn 多包工作区按成员展开并能选出可运行目标() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/*"]

[workspace.dependencies]
bevy = "0.20"
"#,
        );
        // 库成员：不可运行。
        写(
            &根.join("crates/引擎/Cargo.toml"),
            r#"
[package]
name = "engine"
version = "0.1.0"
"#,
        );
        写(&根.join("crates/引擎/src/lib.rs"), "");
        // 游戏成员：可运行，且通过 workspace 继承 bevy。
        写(
            &根.join("crates/游戏/Cargo.toml"),
            r#"
[package]
name = "the_game"
version = "0.1.0"

[dependencies]
bevy = { workspace = true }
"#,
        );
        写(&根.join("crates/游戏/src/main.rs"), "fn main() {}");

        let 工程 = Cargo工程::读取(根).unwrap();
        assert!(工程.是工作区);
        assert_eq!(工程.包.len(), 2);
        assert!(工程.是否贝维工程());
        let 目标 = 工程.默认可执行目标().unwrap();
        assert_eq!(目标.包名, "the_game");
    }

    #[test]
    fn 工作区根包自己的可执行目标优先() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            r#"
[package]
name = "root_game"
version = "0.1.0"

[workspace]
members = ["crates/tool"]
"#,
        );
        写(&根.join("src/main.rs"), "fn main() {}");
        写(
            &根.join("crates/tool/Cargo.toml"),
            r#"
[package]
name = "tool"
version = "0.1.0"
"#,
        );
        写(&根.join("crates/tool/src/main.rs"), "fn main() {}");

        let 工程 = Cargo工程::读取(根).unwrap();
        assert_eq!(工程.包.len(), 2);
        assert_eq!(工程.默认可执行目标().unwrap().包名, "root_game");
    }

    #[test]
    fn 多个可执行目标都被列出且主程序在前() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            r#"
[package]
name = "multi"
version = "0.1.0"

[[bin]]
name = "server"
path = "src/server.rs"
"#,
        );
        写(&根.join("src/main.rs"), "fn main() {}");
        写(&根.join("src/server.rs"), "fn main() {}");

        let 工程 = Cargo工程::读取(根).unwrap();
        let 名称: Vec<&str> = 工程
            .全部可执行目标()
            .iter()
            .map(|目标| 目标.名称.as_str())
            .collect();
        assert_eq!(名称, vec!["multi", "server"]);
    }

    #[test]
    fn src_bin下的目标被自动发现() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            r#"
[package]
name = "toolz"
version = "0.1.0"
"#,
        );
        写(&根.join("src/bin/导出.rs"), "fn main() {}");

        let 工程 = Cargo工程::读取(根).unwrap();
        let 名称: Vec<&str> = 工程
            .全部可执行目标()
            .iter()
            .map(|目标| 目标.名称.as_str())
            .collect();
        assert_eq!(名称, vec!["导出"]);
    }

    #[test]
    fn 无效的清单给出解析错误而不是panic() {
        let 目录 = tempfile::tempdir().unwrap();
        写(&目录.path().join("Cargo.toml"), "这不是 TOML [");
        match Cargo工程::读取(目录.path()) {
            Err(Cargo识别错误::解析失败 { .. }) => {}
            其他 => panic!("应当是解析失败，实际：{其他:?}"),
        }
    }

    #[test]
    fn 缺少清单时明确说明() {
        let 目录 = tempfile::tempdir().unwrap();
        assert!(matches!(
            Cargo工程::读取(目录.path()),
            Err(Cargo识别错误::缺少清单(_))
        ));
    }

    #[test]
    fn 通配匹配覆盖常见形态() {
        assert!(通配匹配("crates", "*"));
        assert!(通配匹配("abc", "a*c"));
        assert!(!通配匹配("abc", "b*"));
        assert!(通配匹配("engine_core", "engine*"));
        assert!(!通配匹配("engine_core", "*engine"));
    }
}
