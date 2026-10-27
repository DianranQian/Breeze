// SPDX-License-Identifier: MIT OR Apache-2.0
//! 游戏运行时存档：版本化槽位、原子写入、备份回退、损坏检测与迁移。
//!
//! # 这与编辑器的"保存场景"完全不是一回事
//!
//! | 数据 | 谁写 | 存在哪 | 本文档管不管 |
//! | --- | --- | --- | --- |
//! | 场景文档（关卡、实体、组件） | 编辑器 | 项目 `scenes/` | **不管** |
//! | 编辑器自动保存恢复点 | 编辑器 | 项目 `.breeze/recovery/` | **不管** |
//! | 玩家进度（等级、背包、世界状态） | **游戏运行时** | 游戏自己的存档目录 | **管** |
//!
//! 把编辑器场景保存说成"游戏存档"是常见误解：游戏发布后玩家没有编辑器，
//! 存档必须由游戏自己写、自己读、自己迁移。因此本 crate：
//!
//! * **不依赖 Bevy、不依赖编辑器核心**：游戏可以只依赖它；
//! * **不规定玩法字段**：载荷是 `serde_json::Value`，背包/任务/等级都由游戏定义；
//! * **只管"存档怎么可靠地活着"**：槽位、版本、原子性、备份、损坏与迁移。
//!
//! # 数据结构
//!
//! ```text
//! saves/
//!   slot_1.sav        ← 当前存档（RON）
//!   slot_1.sav.bak    ← 上一版（写入前自动保留）
//!   slot_1.sav.tmp    ← 写入中间态（原子替换用）
//! ```
//!
//! 文件内容：
//!
//! ```ron
//! (
//!     format_version: 1,        // 存档**格式**版本（本 crate 负责迁移）
//!     game_version: 3,          // 游戏**数据**版本（游戏自己负责迁移）
//!     slot: "slot_1",
//!     saved_at_unix: 1792600000,
//!     play_time_seconds: 1234.5,
//!     preview: "青岚山谷 · 等级 7",
//!     payload: {...},           // 任意 JSON：由游戏定义
//! )
//! ```
//!
//! # 三条可靠性约束
//!
//! 1. **原子写**：先写 `.tmp`、`fsync`、再把旧文件改名成 `.bak`、最后 rename 就位；
//!    任何一步失败都回退到旧文件，不会留下"半个存档"。
//! 2. **读失败可回退**：主文件解析失败依次尝试 `.bak` 与 `.tmp`，
//!    并把"从备份恢复"如实告诉调用方。
//! 3. **不兼容要说清**：格式版本高于本 crate 支持时返回明确错误，
//!    不静默清空、不覆盖未来版本文件。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 当前支持的最高存档格式版本。
///
/// 只在**格式本身**变化时增加（例如字段语义改变）；游戏数据版本由游戏自己管理，
/// 不要混进来。
pub const 存档格式版本: u32 = 1;

/// 存档目录（相对于游戏的数据目录）。
pub const 存档目录名: &str = "saves";
/// 存档扩展名。
pub const 存档扩展名: &str = "sav";

/// 存档错误。
#[derive(Debug, thiserror::Error)]
pub enum 存档错误 {
    /// 文件系统错误。
    #[error("存档文件操作失败：{路径:?}（{来源}）")]
    文件 {
        /// 路径。
        路径: PathBuf,
        /// 底层错误。
        来源: std::io::Error,
    },
    /// 解析失败（主文件与备份都不可用）。
    #[error("存档无法解析：{0}")]
    解析(String),
    /// 格式版本高于本程序支持。
    #[error("存档格式版本 {文件版本} 高于本程序支持的 {支持版本}，请升级游戏")]
    版本过新 {
        /// 文件版本。
        文件版本: u32,
        /// 支持版本。
        支持版本: u32,
    },
    /// 槽位名不合法。
    #[error("存档槽位名不合法：{0}（只允许字母、数字、下划线、连字符）")]
    槽位非法(String),
    /// 槽位不存在。
    #[error("存档槽位不存在：{0}")]
    槽位不存在(String),
}

/// 存档操作结果。
pub type 存档结果<T> = Result<T, 存档错误>;

/// 一份存档的元数据（不含玩法载荷）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 存档元数据 {
    /// 存档格式版本（本 crate 负责）。
    pub format_version: u32,
    /// 游戏数据版本（游戏自己负责解释与迁移）。
    #[serde(default)]
    pub game_version: u32,
    /// 槽位名。
    pub slot: String,
    /// 保存时间（Unix 秒）。
    #[serde(default)]
    pub saved_at_unix: u64,
    /// 游戏内游玩时长（秒）。
    #[serde(default)]
    pub play_time_seconds: f64,
    /// 一句话预览（游戏可放"关卡名 · 等级 7"，存档列表直接显示）。
    #[serde(default)]
    pub preview: String,
}

/// 一份完整存档。
///
/// # 为什么字段是"平铺"的，而不是嵌套 `元数据`
///
/// 存到磁盘上的东西要经得起人工查看。平铺后文件就是
///
/// ```ron
/// (
///     format_version: 1,
///     slot: "slot_1",
///     payload: {...},
/// )
/// ```
///
/// 而不是 `(元数据: (format_version: 1, ...), payload: ...)` 这种多一层、
/// 手工修起来更容易出错的结构。`存档元数据` 仍然存在，它是从本结构
/// **取出来的视图**（`存档::元数据`），给列表界面用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 存档 {
    /// 存档格式版本（本 crate 负责）。
    pub format_version: u32,
    /// 游戏数据版本（游戏自己负责解释与迁移）。
    #[serde(default)]
    pub game_version: u32,
    /// 槽位名。
    pub slot: String,
    /// 保存时间（Unix 秒）。
    #[serde(default)]
    pub saved_at_unix: u64,
    /// 游戏内游玩时长（秒）。
    #[serde(default)]
    pub play_time_seconds: f64,
    /// 一句话预览（游戏可放"关卡名 · 等级 7"，存档列表直接显示）。
    #[serde(default)]
    pub preview: String,
    /// 玩法载荷：由游戏定义结构，本 crate 只保证它被完整保存与读回。
    ///
    /// 用 `serde_json::Value` 而不是泛型参数：存档槽位目录需要在**不知道**
    /// 游戏类型的情况下列出/删除/迁移（编辑器侧尤其如此）。泛型会把
    /// "列出所有槽位"变成需要游戏类型参数，得不偿失。
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl 存档 {
    /// 用元数据与载荷构造。
    pub fn 新建(元数据: 存档元数据, payload: serde_json::Value) -> Self {
        Self {
            format_version: 元数据.format_version,
            game_version: 元数据.game_version,
            slot: 元数据.slot,
            saved_at_unix: 元数据.saved_at_unix,
            play_time_seconds: 元数据.play_time_seconds,
            preview: 元数据.preview,
            payload,
        }
    }

    /// 取出元数据视图（列表界面用）。
    pub fn 元数据(&self) -> 存档元数据 {
        存档元数据 {
            format_version: self.format_version,
            game_version: self.game_version,
            slot: self.slot.clone(),
            saved_at_unix: self.saved_at_unix,
            play_time_seconds: self.play_time_seconds,
            preview: self.preview.clone(),
        }
    }
}

/// 读取结果。
#[derive(Debug, Clone, PartialEq)]
pub struct 读取结果 {
    /// 读到的存档。
    pub 存档: 存档,
    /// 是否从备份/临时文件恢复（主文件损坏时）。
    pub 从备份恢复: bool,
    /// 实际读取的路径。
    pub 来源路径: PathBuf,
}

/// 存档槽位仓库：管理一个目录下的多个槽位。
#[derive(Debug, Clone)]
pub struct 存档仓库 {
    目录: PathBuf,
    /// 写入失败注入（仅测试用；`None` 表示正常写盘）。
    写入失败原因: Option<String>,
    /// 是否在写入后强制让主文件损坏（仅测试用）。
    写后损坏主文件: bool,
}

impl 存档仓库 {
    /// 用存档目录构造。目录不存在会在首次写入时创建。
    pub fn 新建(目录: impl Into<PathBuf>) -> Self {
        Self {
            目录: 目录.into(),
            写入失败原因: None,
            写后损坏主文件: false,
        }
    }

    /// 存档目录。
    pub fn 目录(&self) -> &Path {
        &self.目录
    }

    /// 槽位文件名。
    pub fn 槽位路径(&self, 槽位: &str) -> 存档结果<PathBuf> {
        校验槽位名(槽位)?;
        Ok(self.目录.join(format!("{槽位}.{存档扩展名}")))
    }

    /// 列出全部槽位（读元数据；损坏的槽位会被跳过并记入返回值）。
    ///
    /// 返回 `(元数据列表, 损坏槽位及原因)`。列表按保存时间倒序，
    /// 最近的存档排在最前 —— 存档选择界面的常见排序。
    pub fn 列表(&self) -> (Vec<存档元数据>, Vec<(String, String)>) {
        let mut 结果 = Vec::new();
        let mut 损坏 = Vec::new();
        let Ok(目录) = std::fs::read_dir(&self.目录) else {
            return (结果, 损坏);
        };
        for 项 in 目录.flatten() {
            let 路径 = 项.path();
            if 路径.extension().and_then(|值| 值.to_str()) != Some(存档扩展名) {
                continue;
            }
            match 读取文件(&路径) {
                Ok(读取) => 结果.push(读取.存档.元数据()),
                Err(错误) => {
                    let 名 = 路径
                        .file_stem()
                        .map(|值| 值.to_string_lossy().to_string())
                        .unwrap_or_default();
                    损坏.push((名, 错误.to_string()));
                }
            }
        }
        结果.sort_by_key(|项| std::cmp::Reverse(项.saved_at_unix));
        (结果, 损坏)
    }

    /// 读取一个槽位。
    pub fn 读取(&self, 槽位: &str) -> 存档结果<读取结果> {
        let 路径 = self.槽位路径(槽位)?;
        读取文件(&路径)
    }

    /// 删除一个槽位（连同备份与临时文件）。
    pub fn 删除(&self, 槽位: &str) -> 存档结果<()> {
        let 路径 = self.槽位路径(槽位)?;
        if !路径.is_file() && !加后缀(&路径, ".bak").is_file() {
            return Err(存档错误::槽位不存在(槽位.to_string()));
        }
        for 候选 in [路径.clone(), 加后缀(&路径, ".bak"), 加后缀(&路径, ".tmp")] {
            if 候选.exists() {
                std::fs::remove_file(&候选).map_err(|错误| 存档错误::文件 {
                    路径: 候选.clone(),
                    来源: 错误,
                })?;
            }
        }
        Ok(())
    }

    /// 写入一个槽位。
    ///
    /// # 原子性
    ///
    /// 1. 写 `.tmp` 并 `sync_all`；
    /// 2. 若主文件存在，改名为 `.bak`（覆盖旧备份）；
    /// 3. `.tmp` 改名为主文件；
    /// 4. 第 3 步失败时把 `.bak` 改回主文件，再返回错误。
    ///
    /// 全程不会出现"主文件被截断"的中间态。
    pub fn 写入(&self, 存档: &存档) -> 存档结果<()> {
        校验槽位名(&存档.slot)?;
        if let Some(原因) = &self.写入失败原因 {
            return Err(存档错误::文件 {
                路径: self.目录.clone(),
                来源: std::io::Error::other(原因.clone()),
            });
        }
        std::fs::create_dir_all(&self.目录).map_err(|错误| 存档错误::文件 {
            路径: self.目录.clone(),
            来源: 错误,
        })?;

        let 主 = self.槽位路径(&存档.slot)?;
        let 临时 = 加后缀(&主, ".tmp");
        let 备份 = 加后缀(&主, ".bak");

        let 文本 = ron::ser::to_string_pretty(存档, ron::ser::PrettyConfig::default())
            .map_err(|错误| 存档错误::解析(format!("存档编码失败：{错误}")))?;
        {
            use std::io::Write;
            let mut 文件 = std::fs::File::create(&临时).map_err(|错误| 存档错误::文件 {
                路径: 临时.clone(),
                来源: 错误,
            })?;
            文件
                .write_all(文本.as_bytes())
                .and_then(|_| 文件.sync_all())
                .map_err(|错误| 存档错误::文件 {
                    路径: 临时.clone(),
                    来源: 错误,
                })?;
        }

        let 有旧文件 = 主.is_file();
        if 有旧文件 {
            if 备份.exists() {
                let _ = std::fs::remove_file(&备份);
            }
            std::fs::rename(&主, &备份).map_err(|错误| 存档错误::文件 {
                路径: 主.clone(),
                来源: 错误,
            })?;
        }

        if let Err(错误) = std::fs::rename(&临时, &主) {
            if 有旧文件 {
                let _ = std::fs::rename(&备份, &主);
            }
            let _ = std::fs::remove_file(&临时);
            return Err(存档错误::文件 {
                路径: 主,
                来源: 错误,
            });
        }

        if self.写后损坏主文件 {
            let _ = std::fs::write(&主, "这不是合法的存档");
        }
        Ok(())
    }

    /// 注入写入失败（测试"保存失败时旧存档不被破坏"）。
    pub fn 设置写入失败(&mut self, 原因: impl Into<String>) {
        self.写入失败原因 = Some(原因.into());
    }

    /// 注入"写完后把主文件写坏"（测试"从备份恢复"）。
    pub fn 设置写后损坏(&mut self, 损坏: bool) {
        self.写后损坏主文件 = 损坏;
    }
}

/// 从文件读取，主文件解析失败时依次回退 `.bak` / `.tmp`。
fn 读取文件(路径: &Path) -> 存档结果<读取结果> {
    let 候选 = [
        (路径.to_path_buf(), false),
        (加后缀(路径, ".bak"), true),
        (加后缀(路径, ".tmp"), true),
    ];
    let 存在: Vec<_> = 候选.into_iter().filter(|(项, _)| 项.is_file()).collect();
    if 存在.is_empty() {
        return Err(存档错误::槽位不存在(
            路径
                .file_stem()
                .map(|值| 值.to_string_lossy().to_string())
                .unwrap_or_default(),
        ));
    }
    let mut 最后错误 = None;
    for (候选路径, 从备份) in 存在 {
        let 文本 = match std::fs::read_to_string(&候选路径) {
            Ok(文本) => 文本,
            Err(错误) => {
                最后错误 = Some(存档错误::文件 {
                    路径: 候选路径,
                    来源: 错误,
                });
                continue;
            }
        };
        match 解码(&文本) {
            Ok(存档) => {
                return Ok(读取结果 {
                    存档,
                    从备份恢复: 从备份,
                    来源路径: 候选路径,
                });
            }
            Err(错误) => 最后错误 = Some(错误),
        }
    }
    Err(最后错误.unwrap_or_else(|| 存档错误::解析("所有候选文件都无法解析".into())))
}

/// 解码一份存档并检查格式版本。
pub fn 解码(文本: &str) -> 存档结果<存档> {
    // 先只读元数据探测版本：版本过新时给出准确错误，而不是让完整解析
    // 因为未知字段先失败（`serde(deny_unknown_fields)` 的未来版本尤其如此）。
    #[derive(Deserialize)]
    struct 版本探测 {
        format_version: u32,
    }
    if let Ok(探测) = ron::from_str::<版本探测>(文本)
        && 探测.format_version > 存档格式版本
    {
        return Err(存档错误::版本过新 {
            文件版本: 探测.format_version,
            支持版本: 存档格式版本,
        });
    }
    let mut 存档: 存档 =
        ron::from_str(文本).map_err(|错误| 存档错误::解析(format!("{错误}")))?;
    if 存档.format_version == 0 {
        存档.format_version = 存档格式版本;
    }
    Ok(存档)
}

/// 编码一份存档（测试与外部工具共用）。
pub fn 编码(存档: &存档) -> 存档结果<String> {
    ron::ser::to_string_pretty(存档, ron::ser::PrettyConfig::default())
        .map_err(|错误| 存档错误::解析(format!("存档编码失败：{错误}")))
}

/// 迁移：把旧格式存档升到当前格式。
///
/// # 为什么迁移在本 crate 而不是游戏里
///
/// **格式**（槽位、元数据、原子性）由本 crate 定义，因此格式升级也由它负责；
/// **玩法数据**（payload 里的字段）由游戏负责，本函数只把 `game_version`
/// 原样保留，不假装能理解游戏字段。
///
/// 目前只有格式 1，因此没有实际迁移步骤；`from_version == 1` 时原样返回。
/// 保留这个函数是给未来的格式 2 一个明确的落点，而不是等到需要时再改 API。
pub fn 迁移(存档: 存档) -> 存档结果<存档> {
    if 存档.format_version > 存档格式版本 {
        return Err(存档错误::版本过新 {
            文件版本: 存档.format_version,
            支持版本: 存档格式版本,
        });
    }
    let mut 存档 = 存档;
    存档.format_version = 存档格式版本;
    Ok(存档)
}

/// 校验槽位名：只允许字母、数字、下划线与连字符。
///
/// 这是**路径安全**要求，不是风格偏好：槽位名直接拼进文件名，
/// 允许 `../` 就等于允许写到存档目录之外。
pub fn 校验槽位名(槽位: &str) -> 存档结果<()> {
    if 槽位.is_empty() || 槽位.len() > 64 {
        return Err(存档错误::槽位非法(槽位.to_string()));
    }
    if !槽位
        .chars()
        .all(|字符| 字符.is_ascii_alphanumeric() || 字符 == '_' || 字符 == '-')
    {
        return Err(存档错误::槽位非法(槽位.to_string()));
    }
    Ok(())
}

fn 加后缀(路径: &Path, 后缀: &str) -> PathBuf {
    let mut 文本 = 路径.as_os_str().to_os_string();
    文本.push(后缀);
    PathBuf::from(文本)
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 存档(槽位: &str, 预览: &str, 等级: u32) -> 存档 {
        存档::新建(
            存档元数据 {
                format_version: 存档格式版本,
                game_version: 1,
                slot: 槽位.to_string(),
                saved_at_unix: 100,
                play_time_seconds: 12.5,
                preview: 预览.to_string(),
            },
            serde_json::json!({ "等级": 等级, "背包": ["药", "剑"] }),
        )
    }

    #[test]
    fn 保存后能原样读回载荷() {
        let 目录 = tempfile::tempdir().unwrap();
        let 仓库 = 存档仓库::新建(目录.path());
        let 原 = 存档("slot_1", "青岚山谷 · 等级 7", 7);
        仓库.写入(&原).expect("写入");

        let 读取 = 仓库.读取("slot_1").expect("读取");
        assert!(!读取.从备份恢复);
        assert_eq!(读取.存档, 原);
        assert!(
            读取.存档.payload["背包"]
                .as_array()
                .is_some_and(|项| 项.len() == 2)
        );
    }

    #[test]
    fn 列表按时间倒序且跳过损坏槽位() {
        let 目录 = tempfile::tempdir().unwrap();
        let 仓库 = 存档仓库::新建(目录.path());
        let mut 甲 = 存档("a", "A", 1);
        甲.saved_at_unix = 100;
        let mut 乙 = 存档("b", "B", 2);
        乙.saved_at_unix = 200;
        仓库.写入(&甲).unwrap();
        仓库.写入(&乙).unwrap();
        std::fs::write(目录.path().join("坏.sav"), "不是 RON").unwrap();

        let (列表, 损坏) = 仓库.列表();
        assert_eq!(列表.len(), 2);
        assert_eq!(列表[0].slot, "b", "最近的排在前面");
        assert_eq!(损坏.len(), 1);
        assert!(损坏[0].1.contains("解析") || !损坏[0].1.is_empty());
    }

    #[test]
    fn 写入失败不会破坏旧存档() {
        let 目录 = tempfile::tempdir().unwrap();
        let 仓库 = 存档仓库::新建(目录.path());
        仓库.写入(&存档("slot_1", "旧", 1)).unwrap();

        let mut 会失败 = 仓库.clone();
        会失败.设置写入失败("磁盘满");
        let 错误 = 会失败.写入(&存档("slot_1", "新", 2)).unwrap_err();
        assert!(错误.to_string().contains("磁盘满"));

        let 读回 = 仓库.读取("slot_1").unwrap();
        assert_eq!(读回.存档.preview, "旧", "失败不能动旧文件");
    }

    #[test]
    fn 主文件损坏时从备份恢复并标记来源() {
        let 目录 = tempfile::tempdir().unwrap();
        let 仓库 = 存档仓库::新建(目录.path());
        仓库.写入(&存档("slot_1", "第一版", 1)).unwrap();
        仓库.写入(&存档("slot_1", "第二版", 2)).unwrap();
        // 手动写坏主文件，模拟断电。
        std::fs::write(目录.path().join("slot_1.sav"), "坏").unwrap();

        let 读取 = 仓库.读取("slot_1").expect("应当从备份恢复");
        assert!(读取.从备份恢复);
        assert_eq!(读取.存档.preview, "第一版");
    }

    #[test]
    fn 版本过新时明确拒绝而不是清空() {
        let 目录 = tempfile::tempdir().unwrap();
        std::fs::write(
            目录.path().join("future.sav"),
            "(format_version: 99, slot: \"future\")",
        )
        .unwrap();
        let 仓库 = 存档仓库::新建(目录.path());
        let 错误 = 仓库.读取("future").unwrap_err();
        assert!(matches!(错误, 存档错误::版本过新 { .. }), "{错误:?}");
        // 原文件必须还在：拒绝读取不代表可以删除。
        assert!(目录.path().join("future.sav").is_file());
    }

    #[test]
    fn 槽位名阻止目录穿越() {
        let 目录 = tempfile::tempdir().unwrap();
        let 仓库 = 存档仓库::新建(目录.path());
        assert!(matches!(
            仓库.槽位路径("../逃逸"),
            Err(存档错误::槽位非法(_))
        ));
        assert!(matches!(仓库.槽位路径("a/b"), Err(存档错误::槽位非法(_))));
        assert!(仓库.槽位路径("slot-1_ok").is_ok());
    }

    #[test]
    fn 删除带走备份与临时文件() {
        let 目录 = tempfile::tempdir().unwrap();
        let 仓库 = 存档仓库::新建(目录.path());
        仓库.写入(&存档("slot_1", "一", 1)).unwrap();
        仓库.写入(&存档("slot_1", "二", 2)).unwrap();
        assert!(目录.path().join("slot_1.sav.bak").is_file());

        仓库.删除("slot_1").expect("删除");
        assert!(!目录.path().join("slot_1.sav").exists());
        assert!(!目录.path().join("slot_1.sav.bak").exists());
        assert!(matches!(仓库.删除("slot_1"), Err(存档错误::槽位不存在(_))));
    }

    #[test]
    fn 迁移保留游戏版本并只改格式版本() {
        let mut 旧 = 存档("slot_1", "旧", 1);
        旧.format_version = 1;
        旧.game_version = 5;
        let 新 = 迁移(旧.clone()).expect("迁移");
        assert_eq!(新.format_version, 存档格式版本);
        assert_eq!(新.game_version, 5, "游戏数据版本由游戏负责");
        assert_eq!(新.payload, 旧.payload);
    }

    #[test]
    fn 编码解码往返一致() {
        let 原 = 存档("slot_1", "往返", 3);
        let 文本 = 编码(&原).unwrap();
        let 读回 = 解码(&文本).unwrap();
        assert_eq!(读回, 原);
    }
}
