//! 资源稳定标识（UID）：让引用不再只依赖文件路径。
//!
//! # 为什么需要它
//!
//! 微风之前的资源引用只有路径。路径会变：用户重命名、换目录，或者干脆在
//! 系统文件管理器里拖一下。编辑器内的重命名已经能同步改写引用（见
//! `editor_app::asset_ops`），但**编辑器之外**的变化无法被路径匹配识别 ——
//! 下次打开场景就是一堆断链。
//!
//! 成熟引擎（Godot 的 `.godot/imported` + UID、Unity 的 `.meta` GUID）
//! 都用"给资源一个稳定标识，路径只是它当前所在位置"来解决这件事。
//! 本模块只做数据与持久化，不碰场景文档：
//!
//! ```text
//! .breeze/asset_uids.ron          （项目内，进版本控制）
//!   assets/房屋.png  -> 4f2c...
//!   assets/树.png    -> 9a01...
//! ```
//!
//! # 为什么单独一个文件，而不是塞进 `imports.ron`
//!
//! `imports.ron` 是**导入缓存**：`更新导入清单` 会按磁盘现状整体重建，
//! 它回答"这个文件需不需要重新导入"。标识表回答的是"这个资源是谁"，
//! 两者生命周期不同 —— 把身份塞进缓存，任何一次重建都可能把它抹掉。
//!
//! # 原生 Cargo 工程
//!
//! 原生工程没有微风的项目清单，也可能不想多一个 `.breeze/` 目录。
//! 因此标识表只在**微风项目**里落盘；原生工程仍然可以在内存里拥有标识，
//! 但不会写文件。这个限制是明确的，不假装"原生工程也有 UID"。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::项目错误;
use scene_doc::path::项目相对路径;

/// 标识表格式版本。
pub const 标识格式版本: u32 = 1;

/// 标识表在项目内的相对路径。
pub const 标识表相对路径: &str = ".breeze/asset_uids.ron";

fn 默认格式版本() -> u32 {
    标识格式版本
}

/// 资源标识表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 资源标识表 {
    /// 格式版本。
    #[serde(default = "默认格式版本")]
    pub 格式版本: u32,
    /// 项目相对路径文本 → 稳定标识。
    ///
    /// 用文本做键而不是 `项目相对路径`：RON 里直接读字符串更稳，
    /// 也避免路径类型将来增加校验时把旧文件读崩。
    #[serde(default)]
    pub 标识: BTreeMap<String, Uuid>,
}

impl Default for 资源标识表 {
    fn default() -> Self {
        Self {
            格式版本: 标识格式版本,
            标识: BTreeMap::new(),
        }
    }
}

impl 资源标识表 {
    /// 从磁盘读取；文件不存在时返回空表（首次打开不是错误）。
    ///
    /// 文件存在但解析失败时**返回错误**，不是静默清空 ——
    /// 清空会让所有场景的 UID 无法回连，用户还以为项目正常。
    pub fn 读取(路径: &Path) -> Result<Self, 项目错误> {
        if !路径.is_file() {
            return Ok(Self::default());
        }
        let 文本 =
            std::fs::read_to_string(路径).map_err(|错误| 项目错误::文件错误(路径, 错误))?;
        // 先探测版本，与项目清单/场景同样的策略：版本过新要明确报错。
        let 解析: Self = ron::from_str(&文本).map_err(|来源| 项目错误::清单解析 {
            路径: 路径.to_path_buf(),
            来源: Box::new(来源),
        })?;
        if 解析.格式版本 > 标识格式版本 {
            return Err(项目错误::版本过新 {
                文件版本: 解析.格式版本,
                支持版本: 标识格式版本,
            });
        }
        Ok(解析)
    }

    /// 原子写盘：先写临时文件再 rename，避免中途崩溃留下半个文件。
    pub fn 保存(&self, 路径: &Path) -> Result<(), 项目错误> {
        if let Some(父) = 路径.parent() {
            std::fs::create_dir_all(父).map_err(|错误| 项目错误::文件错误(父, 错误))?;
        }
        格式版本检查(self.格式版本)?;
        let 文本 = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|来源| {
                // 序列化失败不可能是"文件读不出来"，用 io 错误承载更准确。
                项目错误::文件错误(
                    路径,
                    std::io::Error::new(std::io::ErrorKind::InvalidData, 来源.to_string()),
                )
            })?;
        let 临时 = 路径.with_extension("ron.tmp");
        std::fs::write(&临时, 文本).map_err(|错误| 项目错误::文件错误(&临时, 错误))?;
        std::fs::rename(&临时, 路径).map_err(|错误| 项目错误::文件错误(路径, 错误))?;
        Ok(())
    }

    /// 取某路径的标识。
    pub fn 取(&self, 路径: &项目相对路径) -> Option<Uuid> {
        self.标识.get(路径.文本()).copied()
    }

    /// 保证某路径有标识；没有就生成并返回"是否新建"。
    pub fn 确保(&mut self, 路径: &项目相对路径) -> (Uuid, bool) {
        if let Some(已有) = self.取(路径) {
            return (已有, false);
        }
        let 标识 = Uuid::new_v4();
        self.标识.insert(路径.文本().to_string(), 标识);
        (标识, true)
    }

    /// 文件改名/移动后同步标识；返回是否真的迁移了一条。
    pub fn 重命名(&mut self, 旧路径: &项目相对路径, 新路径: &项目相对路径) -> bool {
        let Some(标识) = self.标识.remove(旧路径.文本()) else {
            return false;
        };
        self.标识.insert(新路径.文本().to_string(), 标识);
        true
    }

    /// 资源文件被删除后移除标识；返回是否存在过。
    pub fn 移除(&mut self, 路径: &项目相对路径) -> bool {
        self.标识.remove(路径.文本()).is_some()
    }

    /// 按标识反查当前路径。
    ///
    /// 同一标识出现在多条路径时返回 `None` —— 那是**冲突**，
    /// 不能替用户猜哪一条才是真的。冲突列表见 [`Self::冲突标识`]。
    pub fn 按标识查路径(&self, 标识: &Uuid) -> Option<&str> {
        let mut 命中 = self
            .标识
            .iter()
            .filter(|(_, 值)| *值 == 标识)
            .map(|(路径, _)| 路径.as_str());
        let 首 = 命中.next()?;
        if 命中.next().is_some() {
            return None;
        }
        Some(首)
    }

    /// 全部重复标识（同一 UID 被多条路径使用）。
    pub fn 冲突标识(&self) -> BTreeMap<Uuid, Vec<String>> {
        let mut 分组: BTreeMap<Uuid, Vec<String>> = BTreeMap::new();
        for (路径, 标识) in &self.标识 {
            分组.entry(*标识).or_default().push(路径.clone());
        }
        分组.retain(|_, 路径| 路径.len() > 1);
        分组
    }

    /// 与磁盘现状同步：给新文件分配标识、移除已不存在文件的标识。
    ///
    /// 返回 `(新增, 移除)` 计数。**不碰**仍然存在的文件，因此 UID 稳定。
    pub fn 同步(&mut self, 存在的资源: &[项目相对路径]) -> (usize, usize) {
        let 现存: BTreeSet<String> = 存在的资源
            .iter()
            .map(|路径| 路径.文本().to_string())
            .collect();
        let 新增 = 存在的资源
            .iter()
            .filter(|路径| !self.标识.contains_key(路径.文本()))
            .count();
        for 路径 in 存在的资源 {
            if !self.标识.contains_key(路径.文本()) {
                self.标识
                    .insert(路径.文本().to_string(), Uuid::new_v4());
            }
        }
        let 旧数量 = self.标识.len();
        self.标识.retain(|路径, _| 现存.contains(路径));
        (新增, 旧数量 - self.标识.len())
    }
}

/// 按 UID 修复文档的结果。
///
/// 分开记录四类结果，是为了让调用方能如实告诉用户：
/// 哪些已经修好、哪些因为没有标识而根本无从修、哪些是 UID 冲突、
/// 哪些是文件真的没了。把它们混成"修复了 N 处"会掩盖后面的问题。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 修复报告 {
    /// 真的改过来的引用：`(旧路径, 新路径)`。
    pub 已修复: Vec<(项目相对路径, 项目相对路径)>,
    /// 路径失效、且文档里没有记录 UID 的引用。
    pub 无标识缺失: Vec<项目相对路径>,
    /// UID 在标识表里指向多个路径（冲突），不猜测。
    pub 冲突: Vec<项目相对路径>,
    /// UID 在表里没有对应现存文件。
    pub 无法找回: Vec<项目相对路径>,
}

impl 修复报告 {
    /// 是否有任何需要用户注意的情况。
    pub fn 有问题(&self) -> bool {
        !self.无标识缺失.is_empty() || !self.冲突.is_empty() || !self.无法找回.is_empty()
    }

    /// 是否什么都不用做。
    pub fn 为空(&self) -> bool {
        self.已修复.is_empty() && !self.有问题()
    }
}

fn 格式版本检查(版本: u32) -> Result<(), 项目错误> {
    if 版本 > 标识格式版本 {
        return Err(项目错误::版本过新 {
            文件版本: 版本,
            支持版本: 标识格式版本,
        });
    }
    Ok(())
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 路径(文本: &str) -> 项目相对路径 {
        项目相对路径::解析(文本).expect("路径")
    }

    #[test]
    fn 保存并读取后标识完全一致() {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 文件 = 目录.path().join("asset_uids.ron");
        let mut 表 = 资源标识表::default();
        let (甲, _) = 表.确保(&路径("assets/房屋.png"));
        let (乙, _) = 表.确保(&路径("assets/树.png"));

        表.保存(&文件).expect("保存");
        let 读回 = 资源标识表::读取(&文件).expect("读取");
        assert_eq!(读回.取(&路径("assets/房屋.png")), Some(甲));
        assert_eq!(读回.取(&路径("assets/树.png")), Some(乙));
        assert_eq!(读回.格式版本, 标识格式版本);
    }

    #[test]
    fn 同一个路径多次确保只生成一次标识() {
        let mut 表 = 资源标识表::default();
        let (甲, 新建) = 表.确保(&路径("assets/a.png"));
        assert!(新建);
        let (乙, 新建) = 表.确保(&路径("assets/a.png"));
        assert!(!新建);
        assert_eq!(甲, 乙);
    }

    #[test]
    fn 重命名保留标识且旧路径不再存在() {
        let mut 表 = 资源标识表::default();
        let (标识, _) = 表.确保(&路径("assets/旧.png"));
        assert!(表.重命名(&路径("assets/旧.png"), &路径("assets/新.png")));
        assert_eq!(表.取(&路径("assets/新.png")), Some(标识));
        assert!(表.取(&路径("assets/旧.png")).is_none());
    }

    #[test]
    fn 同步会新增与移除但不动稳定项() {
        let mut 表 = 资源标识表::default();
        let (稳定, _) = 表.确保(&路径("assets/稳定.png"));
        let (消失, _) = 表.确保(&路径("assets/消失.png"));

        let (新增, 移除) = 表.同步(&[路径("assets/稳定.png"), 路径("assets/新.png")]);
        assert_eq!(新增, 1);
        assert_eq!(移除, 1);
        assert_eq!(表.取(&路径("assets/稳定.png")), Some(稳定));
        assert!(表.取(&路径("assets/消失.png")).is_none());
        assert_ne!(表.取(&路径("assets/新.png")), Some(消失));
    }

    #[test]
    fn 重复标识会被报为冲突而不是随便选一条() {
        let mut 表 = 资源标识表::default();
        let 标识 = Uuid::new_v4();
        表.标识.insert("assets/a.png".into(), 标识);
        表.标识.insert("assets/b.png".into(), 标识);
        assert!(表.按标识查路径(&标识).is_none(), "冲突时不能猜");
        let 冲突 = 表.冲突标识();
        assert_eq!(冲突.len(), 1);
        assert_eq!(冲突[&标识].len(), 2);
    }

    #[test]
    fn 版本过新的标识表会被拒绝() {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 文件 = 目录.path().join("asset_uids.ron");
        std::fs::write(&文件, "(格式版本: 99, 标识: {})").expect("写文件");
        let 错误 = 资源标识表::读取(&文件).expect_err("必须拒绝");
        assert!(matches!(错误, 项目错误::版本过新 { .. }), "错误：{错误:?}");
    }

    #[test]
    fn 文件不存在时返回空表而不是错误() {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 表 = 资源标识表::读取(&目录.path().join("不存在.ron")).expect("空表");
        assert_eq!(表, 资源标识表::default());
    }
}
