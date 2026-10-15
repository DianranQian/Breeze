//! 项目模型：项目的创建、打开、目录布局与资源/场景清单。
//!
//! # 目录布局的取舍
//!
//! Godot 使用**单一 `res://` 根 + 全部文件混放**的模型。这个模型对用户简单，
//! 但把"资源"和"场景"混在一棵目录里，会让"哪些文件是编辑器资产"这个问题
//! 只能靠扩展名猜。本编辑器改用**按用途分区**的布局：
//!
//! ```text
//! 我的游戏/
//! ├── project.ron          项目清单（名称、格式版本、路径约定）
//! ├── assets/              原始资源：图片、音频、字体、模型
//! │   ├── textures/
//! │   ├── audio/
//! │   └── models/
//! ├── scenes/              场景文档（编辑器的主要产物）
//! └── src/                  可选的 Rust 源码目录（游戏逻辑）
//! ```
//!
//! 这样做的直接好处：项目根目录下任何**不在**上述分区内的文件
//! （`.git`、`target`、构建产物）都不会被资源浏览器当成资产，
//! 不需要维护一份脆弱的忽略规则。
//!
//! 清单文件用 RON 而不是自定义文本格式：它可读、可进 Git、可被
//! 未来版本用 `serde(default)` 平滑扩展。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use scene_doc::error::{场景错误, 场景结果};
use scene_doc::path::{项目相对路径, 路径错误};
use scene_doc::property::资源类别;
use scene_doc::storage::{场景存储, 文件系统场景存储};

/// 项目清单文件名。
pub const 项目清单文件名: &str = "project.ron";

/// 项目清单的当前格式版本。
pub const 项目格式版本: u32 = 1;

/// 项目内约定的子目录名。
pub mod 目录名 {
    /// 原始资源目录。
    pub const 资源: &str = "assets";
    /// 场景文档目录。
    pub const 场景: &str = "scenes";
    /// 游戏源码目录。
    pub const 源码: &str = "src";
}

/// 项目清单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 项目清单 {
    /// 清单格式版本。
    pub 格式版本: u32,
    /// 项目名称。
    pub 名称: String,
    /// 项目说明。
    #[serde(default)]
    pub 说明: String,
    /// 启动场景的项目相对路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 启动场景: Option<项目相对路径>,
    /// 项目自定义配置，供游戏代码与插件读取。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub 配置: BTreeMap<String, String>,
}

impl 项目清单 {
    /// 以项目名创建一个新清单。
    pub fn 新建(名称: impl Into<String>) -> Self {
        Self {
            格式版本: 项目格式版本,
            名称: 名称.into(),
            说明: String::new(),
            启动场景: None,
            配置: BTreeMap::new(),
        }
    }
}

/// 资源浏览器里的一个条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 资源条目 {
    /// 项目相对路径。
    pub 路径: 项目相对路径,
    /// 推断出的资源类别。
    pub 类别: 资源类别,
    /// 字节大小。
    pub 大小: u64,
    /// 是否是目录。
    pub 是目录: bool,
}

/// 项目错误。
#[derive(Debug, thiserror::Error)]
pub enum 项目错误 {
    /// 目录非空，拒绝在其中创建项目。
    #[error("目录 `{0}` 非空，为避免覆盖已有文件，请在空目录中创建项目")]
    目录非空(PathBuf),

    /// 目标目录已存在同名项目。
    #[error("目录 `{0}` 中已存在 project.ron，无法重复创建项目")]
    项目已存在(PathBuf),

    /// 找不到项目清单。
    #[error("`{0}` 不是 Bevy Studio 项目：未找到 project.ron")]
    未找到项目(PathBuf),

    /// 项目清单的格式版本高于本编辑器。
    #[error("项目清单格式版本 {文件版本} 高于本编辑器支持的 {支持版本}，请升级 Bevy Studio")]
    版本过新 {
        /// 文件里的版本号。
        文件版本: u32,
        /// 本编辑器支持的版本号。
        支持版本: u32,
    },

    /// 项目清单解析失败。
    #[error("项目清单解析失败：{} —— {来源}", 路径.display())]
    清单解析 {
        /// 清单路径。
        路径: PathBuf,
        /// 底层 RON 错误。
        #[source]
        来源: Box<ron::error::SpannedError>,
    },

    /// 路径非法。
    #[error(transparent)]
    路径(#[from] 路径错误),

    /// 场景层错误。
    #[error(transparent)]
    场景(#[from] 场景错误),

    /// 文件系统错误。
    #[error("文件操作失败：{} —— {来源}", 路径.display())]
    文件 {
        /// 出错路径。
        路径: PathBuf,
        /// 底层错误。
        #[source]
        来源: std::io::Error,
    },
}

impl 项目错误 {
    /// 构造一个文件错误。
    fn 文件错误(路径: impl Into<PathBuf>, 来源: std::io::Error) -> Self {
        Self::文件 {
            路径: 路径.into(),
            来源,
        }
    }
}

/// 项目操作结果。
pub type 项目结果<T> = Result<T, 项目错误>;

/// 一个已打开的项目。
#[derive(Debug, Clone)]
pub struct 项目描述 {
    /// 项目根目录（绝对路径）。
    根目录: PathBuf,
    /// 已解析的清单。
    清单: 项目清单,
}

impl 项目描述 {
    /// 在 `根目录` 中**创建**一个新项目。
    ///
    /// 安全约束：只允许在**空目录**或**不存在**的目录里创建。
    /// 这样即使调用方传错路径，也不会把用户的既有文件覆盖掉。
    pub fn 创建(根目录: impl AsRef<Path>, 名称: impl Into<String>) -> 项目结果<Self> {
        let 根 = 根目录.as_ref().to_path_buf();

        if 根.exists() {
            if !根.is_dir() {
                return Err(项目错误::文件错误(
                    &根,
                    std::io::Error::new(std::io::ErrorKind::AlreadyExists, "路径不是目录"),
                ));
            }
            let 非空 = std::fs::read_dir(&根)
                .map_err(|错误| 项目错误::文件错误(&根, 错误))?
                .next()
                .is_some();
            if 非空 {
                if 根.join(项目清单文件名).is_file() {
                    return Err(项目错误::项目已存在(根));
                }
                return Err(项目错误::目录非空(根));
            }
        }

        let 名称 = 名称.into();
        let 描述 = Self {
            根目录: 根.clone(),
            清单: 项目清单::新建(名称),
        };

        // 建目录
        for 子目录 in [目录名::资源, 目录名::场景, 目录名::源码] {
            描述.建目录(子目录)?;
        }
        // 在 assets 下预置常用分类，减少用户第一次使用时的空目录感。
        for 子目录 in [
            format!("{}/textures", 目录名::资源),
            format!("{}/audio", 目录名::资源),
            format!("{}/fonts", 目录名::资源),
            format!("{}/models", 目录名::资源),
        ] {
            描述.建目录(&子目录)?;
        }

        描述.保存清单()?;
        Ok(描述)
    }

    /// 打开一个已有项目。
    pub fn 打开(根目录: impl AsRef<Path>) -> 项目结果<Self> {
        let 根 = 根目录.as_ref().to_path_buf();
        let 清单路径 = 根.join(项目清单文件名);

        if !清单路径.is_file() {
            return Err(项目错误::未找到项目(根));
        }

        let 文本 = std::fs::read_to_string(&清单路径)
            .map_err(|错误| 项目错误::文件错误(&清单路径, 错误))?;

        // 先探测版本，再完整解析 —— 与场景文档层同样的策略：
        // "版本过新"比"解析失败"对用户有用得多。
        if let Some(版本) = 探测清单版本(&文本)
            && 版本 > 项目格式版本 {
                return Err(项目错误::版本过新 {
                    文件版本: 版本,
                    支持版本: 项目格式版本,
                });
            }

        let 清单: 项目清单 = ron::from_str(&文本).map_err(|来源| 项目错误::清单解析 {
            路径: 清单路径.clone(),
            来源: Box::new(来源),
        })?;

        if 清单.格式版本 > 项目格式版本 {
            return Err(项目错误::版本过新 {
                文件版本: 清单.格式版本,
                支持版本: 项目格式版本,
            });
        }

        Ok(Self {
            根目录: 根,
            清单,
        })
    }

    /// 项目根目录。
    pub fn 根目录(&self) -> &Path {
        &self.根目录
    }

    /// 项目清单。
    pub fn 清单(&self) -> &项目清单 {
        &self.清单
    }

    /// 项目清单（可变）。修改后需调用 [`项目描述::保存清单`] 落盘。
    pub fn 清单可变(&mut self) -> &mut 项目清单 {
        &mut self.清单
    }

    /// 项目名。
    pub fn 名称(&self) -> &str {
        &self.清单.名称
    }

    /// 清单文件路径。
    pub fn 清单路径(&self) -> PathBuf {
        self.根目录.join(项目清单文件名)
    }

    /// 某个分区目录的绝对路径。
    pub fn 分区目录(&self, 子目录: &str) -> PathBuf {
        self.根目录.join(子目录)
    }

    /// 把项目相对路径解析为绝对路径。
    pub fn 绝对路径(&self, 相对: &项目相对路径) -> PathBuf {
        相对.拼接(&self.根目录)
    }

    /// 把一个绝对路径转换为项目相对路径。
    ///
    /// 若路径不在项目根目录内，返回 `None` —— 这是"绝对路径污染"
    /// 的拦截点：项目外的文件不允许进入场景引用。
    pub fn 相对路径(&self, 绝对: &Path) -> Option<项目相对路径> {
        let 相对部分 = 绝对.strip_prefix(&self.根目录).ok()?;
        let 文本 = 相对部分.to_string_lossy().replace('\\', "/");
        if 文本.is_empty() {
            return None;
        }
        项目相对路径::解析(&文本).ok()
    }

    /// 创建一个目录（相对项目根）。
    pub fn 建目录(&self, 相对: &str) -> 项目结果<PathBuf> {
        let 绝对 = self.根目录.join(相对.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))?;
        Ok(绝对)
    }

    /// 保存项目清单。
    pub fn 保存清单(&self) -> 项目结果<()> {
        let 清单路径 = self.清单路径();
        let 文本 = ron::ser::to_string_pretty(
            &self.清单,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| 项目错误::文件错误(&清单路径, std::io::Error::other(错误)))?;

        std::fs::create_dir_all(&self.根目录)
            .map_err(|错误| 项目错误::文件错误(&self.根目录, 错误))?;
        std::fs::write(&清单路径, 文本)
            .map_err(|错误| 项目错误::文件错误(&清单路径, 错误))?;
        Ok(())
    }

    /// 切换项目名并落盘。
    pub fn 设名称(&mut self, 名称: impl Into<String>) -> 项目结果<()> {
        self.清单.名称 = 名称.into();
        self.保存清单()
    }

    /// 场景存储。
    pub fn 场景存储(&self) -> 文件系统场景存储 {
        文件系统场景存储::新建()
    }

    /// 一个新的场景文件路径（位于 `scenes/` 下，扩展名为 `.scene.ron`）。
    pub fn 场景路径(&self, 场景名: &str) -> 项目结果<项目相对路径> {
        let 清理后 = 清理文件名(场景名);
        let 路径 = format!("{}/{清理后}.scene.ron", 目录名::场景);
        Ok(项目相对路径::解析(路径)?)
    }

    /// 扫描场景目录，返回所有场景文件的项目相对路径（按路径排序）。
    pub fn 场景列表(&self) -> Vec<项目相对路径> {
        self.扫描目录(目录名::场景)
            .into_iter()
            .map(|条目| 条目.路径)
            .filter(|路径| {
                路径
                    .扩展名()
                    .is_some_and(|扩展| 扩展 == "ron" || 扩展 == "scene")
            })
            .collect()
    }

    /// 扫描资源目录，返回所有资源条目（按路径排序）。
    pub fn 资源列表(&self) -> Vec<资源条目> {
        self.扫描目录(目录名::资源)
    }

    /// 递归扫描项目内某个分区目录。
    ///
    /// 实现要点：
    /// * 使用显式栈而不是递归，避免深层目录导致栈溢出；
    /// * 遇到无法读取的子目录时跳过而不是整体失败 ——
    ///   资源浏览器不该因为一个被占用的目录就整个打不开；
    /// * 结果按路径排序，保证 UI 顺序稳定（不随文件系统枚举顺序抖动）。
    pub fn 扫描目录(&self, 分区: &str) -> Vec<资源条目> {
        let 起点 = self.根目录.join(分区);
        let mut 结果 = Vec::new();
        if !起点.is_dir() {
            return 结果;
        }

        let mut 栈 = vec![起点];
        while let Some(当前) = 栈.pop() {
            let Ok(读取) = std::fs::read_dir(&当前) else {
                continue;
            };
            for 项 in 读取.flatten() {
                let 路径 = 项.path();
                let Ok(元数据) = 项.metadata() else { continue };

                if 元数据.is_dir() {
                    栈.push(路径);
                    continue;
                }
                if !元数据.is_file() {
                    continue;
                }

                let Some(相对) = self.相对路径(&路径) else {
                    continue;
                };
                let 类别 = 相对
                    .扩展名()
                    .map(|扩展| 资源类别::由扩展名推断(&扩展))
                    .unwrap_or(资源类别::其他);
                结果.push(资源条目 {
                    路径: 相对,
                    类别,
                    大小: 元数据.len(),
                    是目录: false,
                });
            }
        }

        结果.sort_by(|左, 右| 左.路径.cmp(&右.路径));
        结果
    }

    /// 在资源目录中新建一个文件夹。
    pub fn 新建资源文件夹(&self, 相对: &项目相对路径) -> 项目结果<()> {
        let 绝对 = self.绝对路径(相对);
        // 安全检查：解析后的路径必须仍在项目内。
        if !绝对.starts_with(&self.根目录) {
            return Err(项目错误::路径(路径错误::目录穿越(
                相对.文本().to_string(),
            )));
        }
        std::fs::create_dir_all(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))?;
        Ok(())
    }

    /// 重命名或移动一个资源，并返回**所有需要同步更新的引用路径**。
    ///
    /// 本方法只做文件系统操作，**不**改场景。
    /// 调用方拿到 [`资源移动结果`] 后必须据此更新打开中的场景文档，
    /// 否则场景里的引用就会断掉 —— 这正是需求里
    /// "不得随意移动或重命名文件后破坏场景中的资源引用"的实现方式：
    /// 把"移动文件"和"更新引用"绑成一个不可分割的操作，由上层一起提交。
    pub fn 移动资源(
        &self,
        旧路径: &项目相对路径,
        新路径: &项目相对路径,
    ) -> 项目结果<资源移动结果> {
        let 旧绝对 = self.绝对路径(旧路径);
        let 新绝对 = self.绝对路径(新路径);

        if !旧绝对.exists() {
            return Err(项目错误::文件错误(
                &旧绝对,
                std::io::Error::new(std::io::ErrorKind::NotFound, "源文件不存在"),
            ));
        }
        if 新绝对.exists() {
            return Err(项目错误::文件错误(
                &新绝对,
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, "目标路径已存在"),
            ));
        }
        if !新绝对.starts_with(&self.根目录) {
            return Err(项目错误::路径(路径错误::目录穿越(
                新路径.文本().to_string(),
            )));
        }

        if let Some(父目录) = 新绝对.parent() {
            std::fs::create_dir_all(父目录)
                .map_err(|错误| 项目错误::文件错误(父目录, 错误))?;
        }

        std::fs::rename(&旧绝对, &新绝对)
            .map_err(|错误| 项目错误::文件错误(&旧绝对, 错误))?;

        Ok(资源移动结果 {
            旧路径: 旧路径.clone(),
            新路径: 新路径.clone(),
        })
    }

    /// 删除一个资源文件。
    pub fn 删除资源(&self, 相对: &项目相对路径) -> 项目结果<()> {
        let 绝对 = self.绝对路径(相对);
        if !绝对.starts_with(&self.根目录) {
            return Err(项目错误::路径(路径错误::目录穿越(
                相对.文本().to_string(),
            )));
        }
        std::fs::remove_file(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))?;
        Ok(())
    }

    /// 读取一个资源文件的原始字节（供导入器使用）。
    pub fn 读取资源(&self, 相对: &项目相对路径) -> 项目结果<Vec<u8>> {
        let 绝对 = self.绝对路径(相对);
        std::fs::read(&绝对).map_err(|错误| 项目错误::文件错误(&绝对, 错误))
    }

    /// 把一份场景写入项目。
    pub fn 保存场景(
        &self,
        场景路径: &项目相对路径,
        文档: &scene_doc::document::场景文档,
    ) -> 场景结果<()> {
        let 绝对 = self.绝对路径(场景路径);
        self.场景存储().写入(&绝对, 文档)?;
        Ok(())
    }

    /// 从项目读入一份场景。
    pub fn 读取场景(
        &self,
        场景路径: &项目相对路径,
    ) -> 场景结果<scene_doc::storage::读取结果> {
        let 绝对 = self.绝对路径(场景路径);
        self.场景存储().读取(&绝对)
    }
}

/// 资源移动的结果，供上层同步更新引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 资源移动结果 {
    /// 移动前的路径。
    pub 旧路径: 项目相对路径,
    /// 移动后的路径。
    pub 新路径: 项目相对路径,
}

/// 在不完整解析 RON 的前提下探测清单的 `格式版本`。
fn 探测清单版本(文本: &str) -> Option<u32> {
    let 位置 = 文本.find("格式版本")?;
    let 后半 = &文本[位置..];
    let 冒号 = 后半.find(':')?;
    let 剩余 = 后半[冒号 + 1..].trim_start();
    let 数字: String = 剩余.chars().take_while(char::is_ascii_digit).collect();
    数字.parse().ok()
}

/// 把用户输入的任意名字清理成安全的文件名。
///
/// 去掉路径分隔符、Windows 保留字符与控制字符，并防止结果为
/// `.` / `..`（那会让场景文件落在目录本身的位置）。
pub fn 清理文件名(输入: &str) -> String {
    let 清理: String = 输入
        .trim()
        .chars()
        .map(|字符| match 字符 {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            字符 if 字符.is_control() => '_',
            字符 => 字符,
        })
        .collect();
    let 清理 = 清理.trim_matches('.').trim().to_string();
    if 清理.is_empty() {
        "未命名".to_string()
    } else {
        清理
    }
}
