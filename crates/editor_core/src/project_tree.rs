// SPDX-License-Identifier: MIT OR Apache-2.0
//! 中文名：项目文件树
//!
//! 把项目根目录下的源码摊成一棵可以折叠的树，供「项目文件树」面板渲染。
//!
//! # 为什么单独一个模块，而不是复用资源浏览器
//!
//! 资源浏览器扫的是 `assets/`，它的过滤规则是**按资源类别**：图片、音频、
//! 场景、材质…… 源码 `.rs` 在那里只能标成"其他"。文件树要回答的是另一个
//! 问题 —— "这个项目的代码分布在哪些文件里"，规则是**按扩展名**，而且要
//! 把 `Cargo.toml`、`README.md` 这些"不在 `assets/`、也不是资源"的文件
//! 摆在根上。两套规则混在一个扫描器里，任何一边加白名单都会误伤另一边。
//!
//! # 为什么在 `editor_core` 而不是界面层
//!
//! 扫描是纯文件系统逻辑：输入项目根、输出节点树，不碰 egui、不碰 Bevy。
//! 放这里可以直接用临时目录做单元测试 —— 界面层测这个要给 egui 上下文，
//! 而"`target/` 里的产物有没有被漏进来"这种缺陷与界面无关。
//!
//! # 覆盖范围（如实说明）
//!
//! * 扫描项目根的 `src/`（递归）与根目录的一份固定核心文件清单；
//! * 只收白名单内的扩展名，`target/`、`.git/`、`node_modules/` 等构建目录
//!   与所有 `.` 开头的隐藏目录整棵跳过；
//! * **不扫**工作区成员包的 `src/`（`crates/*/src` 等）。当前项目模板
//!   （`cargo new` 生成的单包工程）不需要它；一旦需要，扩展点是
//!   [`扫描`] 里的起点列表，而不是另写一棵树。
//!
//! # 排序为什么必须稳定
//!
//! 文件系统枚举顺序不保证稳定。若直接按枚举顺序渲染，同一份项目在两次
//! 启动之间可能换位置 —— 用户"记得它在第三个"的肌肉记忆立刻失效，自动化
//! 验收按行号找节点也会随机失败。因此这里显式排序：目录在前、文件在后，
//! 各自按小写名称比较，同名前缀用原文比较兜底。

use std::path::Path;

/// 文件树里的一个节点。
///
/// 路径一律是**项目相对路径文本**（`/` 分隔），与 [`项目相对路径`] 的
/// 文本形式一致 —— 面板拿到它就能直接排 `打开脚本` 动作，不需要再换算。
///
/// [`项目相对路径`]: scene_doc::path::项目相对路径
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 文件树节点 {
    /// 目录。
    目录 {
        /// 显示名（不含路径）。
        名称: String,
        /// 完整项目相对路径文本。
        路径: String,
        /// 子节点（已排序）。
        子项: Vec<文件树节点>,
    },
    /// 文件。
    文件 {
        /// 显示名。
        名称: String,
        /// 完整项目相对路径文本。
        路径: String,
        /// 按扩展名推断的语言，用于选图标与着色。
        语言: 代码语言,
    },
}

impl 文件树节点 {
    /// 节点显示名。
    pub fn 名称(&self) -> &str {
        match self {
            Self::目录 { 名称, .. } | Self::文件 { 名称, .. } => 名称,
        }
    }

    /// 节点项目相对路径文本。
    pub fn 路径(&self) -> &str {
        match self {
            Self::目录 { 路径, .. } | Self::文件 { 路径, .. } => 路径,
        }
    }

    /// 是否为目录。
    pub fn 是目录(&self) -> bool {
        matches!(self, Self::目录 { .. })
    }

    /// 这棵子树里的文件数（目录节点递归统计，文件节点算 1）。
    pub fn 文件数(&self) -> usize {
        match self {
            Self::文件 { .. } => 1,
            Self::目录 { 子项, .. } => 子项.iter().map(自文件数).sum(),
        }
    }
}

fn 自文件数(节点: &文件树节点) -> usize {
    节点.文件数()
}

/// 一份扫描结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 项目文件树 {
    /// 根级节点（目录在前、文件在后）。
    pub 根项: Vec<文件树节点>,
}

impl 项目文件树 {
    /// 全部文件的节点数。
    pub fn 文件数(&self) -> usize {
        self.根项.iter().map(自文件数).sum()
    }

    /// 是否一个节点都没有（没有项目、或项目里既无 `src/` 也无核心文件）。
    pub fn 为空(&self) -> bool {
        self.根项.is_empty()
    }

    /// 全部节点的路径（先序），用于测试与调试。
    pub fn 全部路径(&self) -> Vec<String> {
        fn 收集(节点: &文件树节点, 出: &mut Vec<String>) {
            出.push(节点.路径().to_string());
            if let 文件树节点::目录 { 子项, .. } = 节点 {
                for 子 in 子项 {
                    收集(子, 出);
                }
            }
        }
        let mut 出 = Vec::new();
        for 节点 in &self.根项 {
            收集(节点, &mut 出);
        }
        出
    }
}

/// 代码语言（按扩展名推断，只用于选图标与着色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 代码语言 {
    /// Rust 源码。
    Rust,
    /// RON 数据。
    Ron,
    /// TOML 配置。
    Toml,
    /// Markdown 文档。
    Markdown,
    /// JSON 数据。
    Json,
    /// 着色器（WGSL / GLSL 及常见后缀）。
    着色器,
    /// 白名单内但不属于以上任何一类（`.txt` 等）。
    其他,
}

impl 代码语言 {
    /// 按文件扩展名（不含点）推断语言。
    pub fn 按扩展名(扩展名: &str) -> Self {
        match 扩展名.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "ron" => Self::Ron,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            "json" => Self::Json,
            "wgsl" | "glsl" | "vert" | "frag" | "comp" => Self::着色器,
            _ => Self::其他,
        }
    }
}

/// 源码目录名。
pub const 源码目录: &str = "src";

/// 收进树的文件扩展名白名单。
///
/// 白名单而不是黑名单：文件树每多一样东西，用户就要多扫一眼。二进制、
/// 压缩包、图片即使放错目录也不该出现在代码树里。
pub const 允许扩展: &[&str] = &[
    "rs", "toml", "md", "markdown", "ron", "json", "wgsl", "glsl", "vert", "frag", "comp", "txt",
];

/// 根目录下的核心文件名（存在才收）。
///
/// 这些文件不是 `src/` 的一部分，但改代码时经常要顺手看一眼 ——
/// 依赖清单、构建配置、说明文档。
pub const 根核心文件: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "README.md",
    "LICENSE",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    ".gitignore",
];

/// 整棵跳过的目录名。
///
/// `target/` 是构建产物（动辄几万文件），`.git/` 是版本库内部结构，
/// `node_modules/` 是前端依赖 —— 这三类进树只会把真正的源码挤到视野之外。
pub const 忽略目录: &[&str] = &[
    "target",
    ".git",
    "node_modules",
    ".breeze",
    ".idea",
    ".vscode",
];

/// 递归深度上限。
///
/// 贪心防御：符号链接已经被跳过，但极端项目仍可能构造出很深的目录
/// （生成的代码、嵌套 vendor）。超过上限的分支整棵跳过，而不是让栈爆掉。
pub const 最大深度: usize = 32;

/// 扫描项目根，构建文件树。
///
/// 项目根不存在或不可读时返回空树 —— 文件树是浏览视图，不该因为
/// 一个权限问题就把面板变成错误页。
pub fn 扫描(项目根: &Path) -> 项目文件树 {
    let mut 根项: Vec<文件树节点> = Vec::new();

    let 源码路径 = 项目根.join(源码目录);
    if 源码路径.is_dir() {
        let 子项 = 扫描目录(&源码路径, 源码目录, 0);
        根项.push(文件树节点::目录 {
            名称: 源码目录.to_string(),
            路径: 源码目录.to_string(),
            子项,
        });
    }

    for 名称 in 根核心文件 {
        let 绝对 = 项目根.join(名称);
        if !绝对.is_file() {
            continue;
        }
        根项.push(文件树节点::文件 {
            名称: (*名称).to_string(),
            路径: (*名称).to_string(),
            语言: 代码语言::按扩展名(
                绝对.extension().and_then(|值| 值.to_str()).unwrap_or(""),
            ),
        });
    }

    排序(&mut 根项);
    项目文件树 { 根项 }
}

/// 递归扫描一个目录，返回已排序的子节点。
///
/// `相对前缀` 是该目录相对项目根的路径文本（第一层是 `src`），子节点的
/// 项目相对路径由它拼出，因此整棵树不需要再回头算一次相对路径。
fn 扫描目录(目录: &Path, 相对前缀: &str, 深度: usize) -> Vec<文件树节点> {
    let mut 子项: Vec<文件树节点> = Vec::new();
    if 深度 > 最大深度 {
        return 子项;
    }
    let Ok(读取) = std::fs::read_dir(目录) else {
        return 子项;
    };
    for 条目 in 读取.flatten() {
        let 路径 = 条目.path();
        let Ok(文件类型) = 条目.file_type() else {
            continue;
        };
        if 文件类型.is_symlink() {
            continue;
        }
        let Some(名称) = 路径.file_name().and_then(|值| 值.to_str()) else {
            continue;
        };
        if 文件类型.is_dir() {
            if 名称.starts_with('.') || 忽略目录.contains(&名称) {
                continue;
            }
            let 相对 = format!("{相对前缀}/{名称}");
            子项.push(文件树节点::目录 {
                名称: 名称.to_string(),
                路径: 相对.clone(),
                子项: 扫描目录(&路径, &相对, 深度 + 1),
            });
            continue;
        }
        if !文件类型.is_file() {
            continue;
        }
        let Some(扩展) = 路径.extension().and_then(|值| 值.to_str()) else {
            continue;
        };
        if !允许扩展.contains(&扩展.to_ascii_lowercase().as_str()) {
            continue;
        }
        子项.push(文件树节点::文件 {
            名称: 名称.to_string(),
            路径: format!("{相对前缀}/{名称}"),
            语言: 代码语言::按扩展名(扩展),
        });
    }
    排序(&mut 子项);
    子项
}

/// 就地排序：目录在前、文件在后；同类按小写名称比较，同名再按原文比较。
fn 排序(节点: &mut [文件树节点]) {
    节点.sort_by(|甲, 乙| {
        乙.是目录()
            .cmp(&甲.是目录())
            .then_with(|| 甲.名称().to_lowercase().cmp(&乙.名称().to_lowercase()))
            .then_with(|| 甲.名称().cmp(乙.名称()))
    });
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 写(根: &Path, 相对: &str) {
        let 路径 = 根.join(相对.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(父) = 路径.parent() {
            std::fs::create_dir_all(父).expect("创建父目录");
        }
        std::fs::write(路径, "// 测试文件\n").expect("写文件");
    }

    #[test]
    fn 递归收进源码并忽略构建目录() {
        let 目录 = tempfile::tempdir().expect("创建临时目录");
        let 根 = 目录.path();
        写(根, "src/main.rs");
        写(根, "src/game/mod.rs");
        写(根, "src/game/玩家.rs");
        写(根, "src/target/不该出现.rs");
        写(根, "target/debug/产物.rs");
        写(根, "src/.hidden/隐藏.rs");
        写(根, "src/notes.txt");

        let 树 = 扫描(根);
        let 路径 = 树.全部路径();

        assert!(路径.contains(&"src/main.rs".to_string()));
        assert!(路径.contains(&"src/game/mod.rs".to_string()));
        assert!(路径.contains(&"src/game/玩家.rs".to_string()));
        assert!(路径.contains(&"src/notes.txt".to_string()));
        for 不该有 in ["src/target", "src/.hidden", "target"] {
            assert!(
                !路径.iter().any(|项| 项.starts_with(不该有)),
                "`{不该有}` 不该出现在文件树里：{路径:?}"
            );
        }
    }

    #[test]
    fn 扩展名白名单过滤非代码文件() {
        let 目录 = tempfile::tempdir().expect("创建临时目录");
        let 根 = 目录.path();
        写(根, "src/main.rs");
        写(根, "src/logo.png");
        写(根, "src/blob.bin");
        写(根, "src/data.json");

        let 路径 = 扫描(根).全部路径();
        assert!(路径.contains(&"src/main.rs".to_string()));
        assert!(路径.contains(&"src/data.json".to_string()));
        assert!(!路径.iter().any(|项| 项.ends_with(".png")));
        assert!(!路径.iter().any(|项| 项.ends_with(".bin")));
    }

    #[test]
    fn 根目录核心文件按清单收进() {
        let 目录 = tempfile::tempdir().expect("创建临时目录");
        let 根 = 目录.path();
        写(根, "Cargo.toml");
        写(根, "README.md");
        写(根, "src/main.rs");
        写(根, "random.txt");

        let 树 = 扫描(根);
        let 路径 = 树.全部路径();
        assert!(路径.contains(&"Cargo.toml".to_string()));
        assert!(路径.contains(&"README.md".to_string()));
        assert!(
            !路径.contains(&"random.txt".to_string()),
            "根目录不在核心清单里的文件不该进树"
        );
        assert_eq!(树.文件数(), 3, "两个核心文件 + 一个源文件");
    }

    #[test]
    fn 目录在前文件在后且各自按名称排序() {
        let 目录 = tempfile::tempdir().expect("创建临时目录");
        let 根 = 目录.path();
        写(根, "src/zebra.rs");
        写(根, "src/alpha.rs");
        写(根, "src/子目录/a.rs");

        let 树 = 扫描(根);
        let 顶层 = &树.根项;
        assert!(顶层[0].是目录(), "`src/` 应当排在最前面");
        let (目录项, 文件项): (Vec<&str>, Vec<&str>) = match &顶层[0] {
            文件树节点::目录 { 子项, .. } => 子项.iter().fold(
                (Vec::new(), Vec::new()),
                |(mut 目录们, mut 文件们), 节点| {
                    if 节点.是目录() {
                        目录们.push(节点.名称());
                    } else {
                        文件们.push(节点.名称());
                    }
                    (目录们, 文件们)
                },
            ),
            _ => panic!("根项第一项应当是目录"),
        };
        assert_eq!(目录项, vec!["子目录"]);
        assert_eq!(文件项, vec!["alpha.rs", "zebra.rs"]);
    }

    #[test]
    fn 没有源码目录时只剩核心文件() {
        let 目录 = tempfile::tempdir().expect("创建临时目录");
        写(目录.path(), "Cargo.toml");

        let 树 = 扫描(目录.path());
        assert_eq!(树.根项.len(), 1);
        assert_eq!(树.根项[0].名称(), "Cargo.toml");
        assert!(!树.为空());
    }

    #[test]
    fn 项目根不存在时返回空树() {
        let 树 = 扫描(Path::new("C:/绝对不存在的项目根/没有任何文件"));
        assert!(树.为空());
        assert_eq!(树.文件数(), 0);
    }

    #[test]
    fn 语言按扩展名推断() {
        assert_eq!(代码语言::按扩展名("rs"), 代码语言::Rust);
        assert_eq!(代码语言::按扩展名("RS"), 代码语言::Rust);
        assert_eq!(代码语言::按扩展名("wgsl"), 代码语言::着色器);
        assert_eq!(代码语言::按扩展名("md"), 代码语言::Markdown);
        assert_eq!(代码语言::按扩展名("txt"), 代码语言::其他);
    }

    #[test]
    fn 核心文件的语言按扩展名落定() {
        let 目录 = tempfile::tempdir().expect("创建临时目录");
        写(目录.path(), "Cargo.toml");
        写(目录.path(), "README.md");

        let 树 = 扫描(目录.path());
        let 语言: Vec<代码语言> = 树
            .根项
            .iter()
            .map(|节点| match 节点 {
                文件树节点::文件 { 语言, .. } => *语言,
                _ => 代码语言::其他,
            })
            .collect();
        assert_eq!(语言, vec![代码语言::Toml, 代码语言::Markdown]);
    }
}
