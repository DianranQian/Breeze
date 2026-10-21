//! 恢复点：把"还没保存的编辑"定期写到独立文件，供异常退出后找回。
//!
//! # 与 `.scene.ron.bak` 的区别（必须分清）
//!
//! 场景存储层已经有一个 `.bak`：它是**保存成功时的上一版**，
//! 用来在文件损坏时退回。它救不了"写了很久但一次都没点保存"的情况——
//! 那正是崩溃/断电最常见的损失。
//!
//! 恢复点是另一条时间线：
//!
//! ```text
//! 用户编辑（未保存） --每 N 秒--> .breeze/recovery/<id>.scene.ron + recovery.ron
//! 用户点保存       --> 删除对应恢复点
//! 异常退出后启动    --> 发现恢复点 → 提示恢复或丢弃
//! ```
//!
//! # 三条硬约束
//!
//! 1. **绝不覆盖用户原文件**：恢复点只写 `.breeze/recovery/`，
//!    恢复动作也只是把内容装进编辑器，是否落盘由用户决定；
//! 2. **恢复点必须是完整可解析的场景**：复用场景存储的编码/解码，
//!    而不是另造格式；
//! 3. **元数据与内容分开**：`recovery.ron` 里记项目根、场景路径、
//!    写入时间与实体数；启动时只读元数据就能判断"这是不是当前项目的、
//!    值不值得提示"，不必解析所有场景。
//!
//! 保存在**项目内**而不是用户配置目录：恢复点属于这个项目的数据，
//! 换机器/换用户时应当跟着项目走；同时天然被项目清理逻辑看见。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use scene_doc::document::场景文档;
use scene_doc::path::项目相对路径;
use scene_doc::storage::{解码场景, 编码场景};

/// 恢复点格式版本。
pub const 恢复点格式版本: u32 = 1;

/// 恢复点目录（项目相对路径）。
pub const 恢复点目录: &str = ".breeze/recovery";

/// 元数据文件名。
pub const 恢复点元数据文件名: &str = "recovery.ron";

/// 单个恢复点的元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 恢复点元数据 {
    /// 格式版本。
    #[serde(default = "默认版本")]
    pub 格式版本: u32,
    /// 恢复点标识（同时是内容文件名的一部分）。
    pub 标识: Uuid,
    /// 写入时刻（Unix 秒）。
    pub 时间: u64,
    /// 场景在项目里的路径；`None` 表示"从未保存过的未命名场景"。
    #[serde(default)]
    pub 场景路径: Option<String>,
    /// 场景显示名（用于提示文案）。
    pub 场景名: String,
    /// 实体数量（用于在提示里给出信息量）。
    pub 实体数: usize,
    /// 内容文件相对项目根的路径。
    pub 内容文件: String,
}

fn 默认版本() -> u32 {
    恢复点格式版本
}

/// 恢复点索引文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 恢复点索引 {
    /// 格式版本。
    #[serde(default = "默认版本")]
    pub 格式版本: u32,
    /// 恢复点列表（按时间排序，最旧在前）。
    #[serde(default)]
    pub 恢复点: Vec<恢复点元数据>,
}

impl Default for 恢复点索引 {
    fn default() -> Self {
        Self {
            格式版本: 恢复点格式版本,
            恢复点: Vec::new(),
        }
    }
}

/// 恢复点存储。所有路径都在项目根之下。
#[derive(Debug, Clone)]
pub struct 恢复点存储 {
    根目录: PathBuf,
    /// 最多保留多少个恢复点（超出时丢最旧的）。
    保留数量: usize,
}

impl 恢复点存储 {
    /// 用项目根目录构造。
    pub fn 新建(根目录: impl Into<PathBuf>) -> Self {
        Self {
            根目录: 根目录.into(),
            保留数量: 默认保留数量,
        }
    }

    /// 指定保留数量（测试与"项目很大，少留几个"的场景用）。
    pub fn 带保留数量(mut self, 数量: usize) -> Self {
        self.保留数量 = 数量.max(1);
        self
    }

    /// 恢复点目录的绝对路径。
    pub fn 目录(&self) -> PathBuf {
        self.根目录.join(恢复点目录.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    /// 索引文件绝对路径。
    pub fn 索引路径(&self) -> PathBuf {
        self.目录().join(恢复点元数据文件名)
    }

    /// 读取索引。文件不存在返回空索引；损坏时返回空索引并给出原因。
    ///
    /// 恢复点损坏**不能阻止编辑器启动**：它只是"可能找回上次的修改"，
    /// 不该让用户连项目都打不开。调用方拿到 `Err` 时应当提示并继续。
    pub fn 读取索引(&self) -> Result<恢复点索引, String> {
        let 路径 = self.索引路径();
        if !路径.is_file() {
            return Ok(恢复点索引::default());
        }
        let 文本 = std::fs::read_to_string(&路径)
            .map_err(|错误| format!("读取恢复点索引失败：{错误}"))?;
        let 索引: 恢复点索引 =
            ron::from_str(&文本).map_err(|错误| format!("恢复点索引解析失败：{错误}"))?;
        if 索引.格式版本 > 恢复点格式版本 {
            return Err(format!(
                "恢复点索引版本 {} 高于本编辑器支持的 {}",
                索引.格式版本, 恢复点格式版本
            ));
        }
        Ok(索引)
    }

    /// 写入一份恢复点，返回它的元数据。
    ///
    /// 编码复用场景存储，保证恢复出来的内容与场景文件同一套格式。
    pub fn 写入(
        &self,
        path: Option<&项目相对路径>,
        文档: &场景文档,
        时间: u64,
    ) -> Result<恢复点元数据, String> {
        std::fs::create_dir_all(self.目录())
            .map_err(|错误| format!("创建恢复点目录失败：{错误}"))?;

        let 标识 = Uuid::new_v4();
        let 内容文件 = format!("{恢复点目录}/{标识}.scene.ron");
        let 内容绝对 = self.根目录.join(内容文件.replace('/', std::path::MAIN_SEPARATOR_STR));
        let 文本 = 编码场景(文档).map_err(|错误| format!("恢复点编码失败：{错误}"))?;
        let 临时 = 内容绝对.with_extension("ron.tmp");
        std::fs::write(&临时, 文本).map_err(|错误| format!("写恢复点失败：{错误}"))?;
        std::fs::rename(&临时, &内容绝对).map_err(|错误| format!("落盘恢复点失败：{错误}"))?;

        let 元数据 = 恢复点元数据 {
            格式版本: 恢复点格式版本,
            标识,
            时间,
            场景路径: path.map(|路径| 路径.文本().to_string()),
            场景名: 文档.元数据.场景名.clone(),
            实体数: 文档.实体数量(),
            内容文件,
        };

        let mut 索引 = self.读取索引().unwrap_or_default();
        // 同一场景只保留**最新**恢复点：旧的点对用户没有额外价值，
        // 还会让提示列表变成一长串同名条目。
        索引.恢复点.retain(|项| 项.场景路径 != 元数据.场景路径);
        索引.恢复点.push(元数据.clone());
        索引.恢复点.sort_by_key(|项| 项.时间);

        // 超出保留数量时丢最旧的，并删掉对应内容文件。
        let mut 被淘汰 = Vec::new();
        while 索引.恢复点.len() > self.保留数量 {
            被淘汰.push(索引.恢复点.remove(0));
        }
        self.保存索引(&索引)?;
        for 旧 in 被淘汰 {
            self.移除内容(&旧);
        }
        Ok(元数据)
    }

    /// 读取某个恢复点的场景内容。
    pub fn 读取内容(&self, 元数据: &恢复点元数据) -> Result<场景文档, String> {
        let 绝对 = self
            .根目录
            .join(元数据.内容文件.replace('/', std::path::MAIN_SEPARATOR_STR));
        let 文本 = std::fs::read_to_string(&绝对)
            .map_err(|错误| format!("读取恢复点内容失败：{错误}"))?;
        解码场景(&文本, &绝对).map_err(|错误| format!("恢复点内容无法解析：{错误}"))
    }

    /// 丢弃一个恢复点（用户选择不恢复，或该场景已经保存）。
    pub fn 丢弃(&self, 元数据: &恢复点元数据) -> Result<(), String> {
        let mut 索引 = self.读取索引().unwrap_or_default();
        索引.恢复点.retain(|项| 项.标识 != 元数据.标识);
        self.保存索引(&索引)?;
        self.移除内容(元数据);
        Ok(())
    }

    /// 丢弃**某个场景路径**对应的恢复点（保存成功后调用）。
    ///
    /// 未命名场景（`None`）也会匹配所有未命名恢复点：
    /// 用户一旦把未命名场景保存到磁盘，旧的无名恢复点就不再有用。
    pub fn 丢弃场景(&self, path: Option<&项目相对路径>) -> Result<usize, String> {
        let 索引 = self.读取索引().unwrap_or_default();
        let 目标: Vec<&恢复点元数据> = 索引
            .恢复点
            .iter()
            .filter(|项| {
                match (path, &项.场景路径) {
                    (Some(路径), Some(文本)) => 路径.文本() == 文本,
                    (None, None) => true,
                    _ => false,
                }
            })
            .collect();
        let 数量 = 目标.len();
        for 项 in 目标 {
            self.丢弃(项)?;
        }
        Ok(数量)
    }

    /// 全部恢复点（供启动提示）。
    pub fn 列表(&self) -> Vec<恢复点元数据> {
        self.读取索引().map(|索引| 索引.恢复点).unwrap_or_default()
    }

    fn 保存索引(&self, 索引: &恢复点索引) -> Result<(), String> {
        std::fs::create_dir_all(self.目录())
            .map_err(|错误| format!("创建恢复点目录失败：{错误}"))?;
        let 文本 = ron::ser::to_string_pretty(索引, ron::ser::PrettyConfig::default())
            .map_err(|错误| format!("恢复点索引编码失败：{错误}"))?;
        let 路径 = self.索引路径();
        let 临时 = 路径.with_extension("ron.tmp");
        std::fs::write(&临时, 文本).map_err(|错误| format!("写恢复点索引失败：{错误}"))?;
        std::fs::rename(&临时, &路径).map_err(|错误| format!("落盘恢复点索引失败：{错误}"))?;
        Ok(())
    }

    fn 移除内容(&self, 元数据: &恢复点元数据) {
        let 绝对 = self
            .根目录
            .join(元数据.内容文件.replace('/', std::path::MAIN_SEPARATOR_STR));
        let _ = std::fs::remove_file(绝对);
    }
}

/// 默认保留的恢复点数量。
///
/// 5 个足够覆盖"开了几个场景轮流编辑"的常见情况；
/// 再多会让 `.breeze/recovery` 在大型项目里变大，
/// 而恢复点的价值集中在**最近一次**。
pub const 默认保留数量: usize = 5;

/// 自动保存的默认间隔（秒）。
pub const 默认自动保存间隔秒: u64 = 30;

/// 恢复点是否比场景文件更新（用于提示"有比磁盘更新的修改"）。
pub fn 比场景更新(恢复点时间: u64, 场景文件: &Path) -> bool {
    let Ok(元数据) = std::fs::metadata(场景文件) else {
        // 场景还没保存过：恢复点当然"更新"。
        return true;
    };
    let Ok(修改时间) = 元数据.modified() else {
        return true;
    };
    let Ok(时长) = 修改时间.duration_since(std::time::UNIX_EPOCH) else {
        return true;
    };
    恢复点时间 > 时长.as_secs()
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::场景文档;

    fn 建项目() -> (tempfile::TempDir, 恢复点存储) {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 根 = 目录.path().join("项目");
        std::fs::create_dir_all(&根).expect("建项目根");
        (目录, 恢复点存储::新建(&根))
    }

    fn 场景(名称: &str, 实体数: usize) -> 场景文档 {
        let mut 文档 = 场景文档::新建(名称);
        for 序号 in 0..实体数 {
            文档.生成实体(format!("实体{序号}"), None);
        }
        文档
    }

    #[test]
    fn 写入后能原样读回内容() {
        let (_目录, 存储) = 建项目();
        let 文档 = 场景("主场景", 3);
        let 元数据 = 存储
            .写入(Some(&项目相对路径::解析("scenes/main.scene.ron").unwrap()), &文档, 100)
            .expect("写入");

        assert_eq!(元数据.实体数, 3);
        assert_eq!(元数据.场景名, "主场景");
        let 读回 = 存储.读取内容(&元数据).expect("读取");
        assert_eq!(读回.实体数量(), 3);
        assert_eq!(读回.元数据.场景名, "主场景");
        assert_eq!(存储.列表().len(), 1);
    }

    #[test]
    fn 同一个场景只保留最新恢复点() {
        let (_目录, 存储) = 建项目();
        let 路径 = 项目相对路径::解析("scenes/main.scene.ron").unwrap();
        存储.写入(Some(&路径), &场景("主场景", 1), 100).expect("写入");
        let 新 = 存储.写入(Some(&路径), &场景("主场景", 5), 200).expect("写入");

        let 列表 = 存储.列表();
        assert_eq!(列表.len(), 1, "同场景不该堆多条");
        assert_eq!(列表[0].标识, 新.标识);
        assert_eq!(列表[0].实体数, 5);
    }

    #[test]
    fn 未命名场景只保留最新一个恢复点() {
        let (_目录, 存储) = 建项目();
        let 甲 = 存储.写入(None, &场景("甲", 1), 100).expect("写入");
        存储.写入(None, &场景("乙", 1), 200).expect("写入");
        存储.写入(None, &场景("丙", 1), 300).expect("写入");
        let 列表 = 存储.列表();
        assert_eq!(列表.len(), 1, "未命名场景共享空路径，只保留最新");
        assert!(!列表.iter().any(|项| 项.标识 == 甲.标识));
    }

    #[test]
    fn 超出保留数量会丢弃最旧的并删内容文件() {
        let (_目录, 基础存储) = 建项目();
        let 存储 = 基础存储.带保留数量(2);
        let 甲 = 存储
            .写入(Some(&项目相对路径::解析("scenes/甲.scene.ron").unwrap()), &场景("甲", 1), 100)
            .expect("写入");
        let 乙 = 存储
            .写入(Some(&项目相对路径::解析("scenes/乙.scene.ron").unwrap()), &场景("乙", 1), 200)
            .expect("写入");
        let 丙 = 存储
            .写入(Some(&项目相对路径::解析("scenes/丙.scene.ron").unwrap()), &场景("丙", 1), 300)
            .expect("写入");
        let 列表 = 存储.列表();
        assert_eq!(列表.len(), 2);
        assert!(!列表.iter().any(|项| 项.标识 == 甲.标识), "最旧的必须被淘汰");
        assert!(列表.iter().any(|项| 项.标识 == 乙.标识));
        assert!(列表.iter().any(|项| 项.标识 == 丙.标识));
    }

    #[test]
    fn 丢弃指定场景会删除内容与索引项() {
        let (_目录, 存储) = 建项目();
        let 路径 = 项目相对路径::解析("scenes/main.scene.ron").unwrap();
        let 元数据 = 存储.写入(Some(&路径), &场景("主场景", 1), 100).expect("写入");
        assert_eq!(存储.丢弃场景(Some(&路径)).expect("丢弃"), 1);
        assert!(存储.列表().is_empty());
        assert!(存储.读取内容(&元数据).is_err(), "内容文件应当被删掉");
    }

    #[test]
    fn 损坏的索引不会让恢复点读取崩溃() {
        let (_目录, 存储) = 建项目();
        std::fs::create_dir_all(存储.目录()).expect("建目录");
        std::fs::write(存储.索引路径(), "{ 这不是 RON").expect("写坏索引");
        assert!(存储.读取索引().is_err());
        assert!(存储.列表().is_empty(), "损坏时列表为空，但不崩溃");
    }

    #[test]
    fn 比场景文件更新能正确判断() {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 文件 = 目录.path().join("main.scene.ron");
        std::fs::write(&文件, "x").expect("写文件");
        // 场景文件刚写，恢复点时间设为远古 → 不更新。
        assert!(!比场景更新(1, &文件));
        // 文件不存在 → 一律认为恢复点更新（场景还没保存过）。
        assert!(比场景更新(1, &目录.path().join("不存在.scene.ron")));
    }
}
