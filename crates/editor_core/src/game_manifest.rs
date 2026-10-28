// SPDX-License-Identifier: MIT OR Apache-2.0
//! 独立游戏运行清单（`game_manifest.ron`）。
//!
//! # 它在交付物里的位置
//!
//! 打包产物是"可执行文件 + 项目文件"的一个平铺目录：
//!
//! ```text
//! Game_Build/
//!   bevy-studio.exe
//!   game_manifest.ron     ← 本模块
//!   project.ron
//!   scenes/  assets/  ...
//! ```
//!
//! 运行时入口（`editor_app::standalone`）先找这份清单：有它就以**游戏**身份
//! 启动，没有才回退到编辑器。判定"这是不是一个可玩的包"不依赖命令行参数，
//! 双击 exe 就能进游戏。
//!
//! # 为什么单独一份文件，而不是只读 `project.ron`
//!
//! 两者回答的问题不同：
//!
//! | 文件 | 回答 | 谁写 |
//! | --- | --- | --- |
//! | `project.ron` | 这个工程怎么被编辑、构建、打包 | 用户与编辑器 |
//! | `game_manifest.ron` | 这个**已发布的包**从哪个场景开始跑 | 打包流程 |
//!
//! 把入口写进清单，意味着"发布后的入口"与"编辑时的启动场景"可以分离，
//! 并且发布方（脚本或打包动作）能在不改工程配置的情况下调整入口。
//! 这与 Godot 的 `export_presets`/`project.godot` 分工是同一个思路。
//!
//! # 路径安全
//!
//! 主场景必须是**项目相对路径**。清单来自磁盘，可能被手改，因此读取时
//! 一律经 [`scene_doc::path::项目相对路径::解析`] 校验：绝对路径与 `..`
//! 都会被拒绝，运行时不会去读包外的场景文件。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{项目描述, 默认设计分辨率};
use scene_doc::path::{路径错误, 项目相对路径};

/// 游戏清单文件名（与可执行文件同级）。
pub const 游戏清单文件名: &str = "game_manifest.ron";

/// 当前支持的清单格式版本。
pub const 游戏清单格式版本: u32 = 1;

/// 独立游戏运行清单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 游戏清单 {
    /// 清单格式版本。
    #[serde(default = "默认格式版本")]
    pub 格式版本: u32,
    /// 游戏名称（窗口标题用）。
    #[serde(default)]
    pub 名称: String,
    /// 主场景的项目相对路径。
    pub 主场景: String,
    /// 设计分辨率。
    #[serde(default = "默认设计分辨率")]
    pub 设计分辨率: (u32, u32),
    /// 是否以全屏启动。
    ///
    /// 缺省为 `true`：发布包应该直接进游戏；开发期可以用 `--windowed`
    /// 覆盖成窗口模式（见 `editor_app::standalone`）。
    #[serde(default = "默认真")]
    pub 全屏: bool,
}

fn 默认格式版本() -> u32 {
    游戏清单格式版本
}

fn 默认真() -> bool {
    true
}

impl 游戏清单 {
    /// 按项目当前配置生成清单。
    ///
    /// 主场景取 [`项目描述::启动场景`]：清单声明优先，否则用场景目录里
    /// 的第一个。项目一个场景都没有时返回 [`清单错误::没有主场景`] ——
    /// 没有入口的包不是可玩的包，不应生成清单。
    pub fn 从项目(项目: &项目描述) -> Result<Self, 清单错误> {
        let 主场景 = 项目.启动场景().ok_or(清单错误::没有主场景)?;
        Ok(Self {
            格式版本: 游戏清单格式版本,
            名称: 项目.名称().to_string(),
            主场景: 主场景.文本().to_string(),
            设计分辨率: 项目.清单().设计分辨率,
            全屏: true,
        })
    }

    /// 校验并返回主场景路径。
    pub fn 主场景路径(&self) -> Result<项目相对路径, 路径错误> {
        项目相对路径::解析(&self.主场景)
    }

    /// 写入目录下的 `game_manifest.ron`，返回写入的绝对路径。
    pub fn 写入(&self, 目录: &Path) -> Result<PathBuf, 清单错误> {
        std::fs::create_dir_all(目录).map_err(清单错误::写入失败)?;
        let 文本 = ron::ser::to_string_pretty(
            self,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| 清单错误::序列化失败(错误.to_string()))?;
        let 路径 = 目录.join(游戏清单文件名);
        std::fs::write(&路径, 文本).map_err(清单错误::写入失败)?;
        Ok(路径)
    }

    /// 读取目录下的 `game_manifest.ron`。
    ///
    /// 读取时做两项校验：格式版本不得高于本编辑器支持的版本（否则字段
    /// 语义可能已经变了，硬读会得到"看似成功但入口错误"的清单）；
    /// 主场景必须是合法的项目相对路径。
    pub fn 读取(目录: &Path) -> Result<Self, 清单错误> {
        let 路径 = 目录.join(游戏清单文件名);
        let 文本 = std::fs::read_to_string(&路径).map_err(清单错误::读取失败)?;
        // 跳过开头的 UTF-8 BOM：本模块写出的文本没有 BOM，但清单也会被手改或
        // 被别的工具重写一次，带 BOM 时 RON 词法直接报"格式错误"，
        // 表现出来却是"这个包跑不起来"。
        let 正文 = 文本.strip_prefix('\u{feff}').unwrap_or(&文本);
        let 清单: 游戏清单 = ron::from_str(正文)
            .map_err(|错误| 清单错误::格式错误(format!("{}：{错误}", 路径.display())))?;
        if 清单.格式版本 > 游戏清单格式版本 {
            return Err(清单错误::版本过新 {
                文件版本: 清单.格式版本,
                支持版本: 游戏清单格式版本,
            });
        }
        清单.主场景路径()?;
        Ok(清单)
    }

    /// 目录里是否存在清单文件；存在则返回它的绝对路径。
    pub fn 定位(目录: &Path) -> Option<PathBuf> {
        let 路径 = 目录.join(游戏清单文件名);
        路径.is_file().then_some(路径)
    }
}

/// 游戏清单的读写错误。
#[derive(Debug, thiserror::Error)]
pub enum 清单错误 {
    /// 读文件失败。
    #[error("读取游戏清单失败：{0}")]
    读取失败(std::io::Error),

    /// 写文件失败。
    #[error("写入游戏清单失败：{0}")]
    写入失败(std::io::Error),

    /// RON 文本损坏。
    #[error("游戏清单格式错误：{0}")]
    格式错误(String),

    /// 序列化失败。
    #[error("游戏清单序列化失败：{0}")]
    序列化失败(String),

    /// 清单版本高于本程序支持的版本。
    #[error("游戏清单格式版本 {文件版本} 高于本程序支持的 {支持版本}，请更新游戏程序")]
    版本过新 {
        /// 清单里声明的版本。
        文件版本: u32,
        /// 本程序支持的版本。
        支持版本: u32,
    },

    /// 项目里没有任何场景，无法确定入口。
    #[error("项目里没有场景，无法生成游戏清单")]
    没有主场景,

    /// 主场景路径不合法。
    #[error("游戏清单里的主场景路径非法：{0}")]
    主场景非法(#[from] 路径错误),
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 临时目录() -> tempfile::TempDir {
        tempfile::tempdir().expect("临时目录")
    }

    #[test]
    fn 写入再读取可以往返() {
        let 目录 = 临时目录();
        let 清单 = 游戏清单 {
            格式版本: 游戏清单格式版本,
            名称: "微风示例".to_string(),
            主场景: "scenes/主场景.scene.ron".to_string(),
            设计分辨率: (1280, 720),
            全屏: false,
        };
        let 路径 = 清单.写入(目录.path()).expect("写入清单");
        assert_eq!(路径, 目录.path().join(游戏清单文件名));
        assert!(路径.is_file());

        let 读回 = 游戏清单::读取(目录.path()).expect("读取清单");
        assert_eq!(读回, 清单);
        assert_eq!(读回.主场景路径().unwrap().文本(), "scenes/主场景.scene.ron");
    }

    #[test]
    fn 定位只认已有的清单文件() {
        let 目录 = 临时目录();
        assert!(游戏清单::定位(目录.path()).is_none());
        std::fs::write(目录.path().join(游戏清单文件名), "()").unwrap();
        assert_eq!(
            游戏清单::定位(目录.path()),
            Some(目录.path().join(游戏清单文件名))
        );
    }

    #[test]
    fn 读取时拒绝绝对路径与目录穿越() {
        let 目录 = 临时目录();
        for 主场景 in ["C:/Windows/win.ini", "/etc/passwd", "../外面.scene.ron"] {
            let 清单 = 游戏清单 {
                格式版本: 游戏清单格式版本,
                名称: "坏入口".to_string(),
                主场景: 主场景.to_string(),
                设计分辨率: 默认设计分辨率(),
                全屏: true,
            };
            清单.写入(目录.path()).expect("写入");
            let 错误 = 游戏清单::读取(目录.path()).expect_err("非法主场景必须被拒绝");
            assert!(
                matches!(错误, 清单错误::主场景非法(_)),
                "主场景 {主场景} 应当被路径校验拒绝，实际 {错误}"
            );
        }
    }

    #[test]
    fn 读取时拒绝比本程序更新的版本() {
        let 目录 = 临时目录();
        let 清单 = 游戏清单 {
            格式版本: 游戏清单格式版本 + 1,
            名称: "未来版本".to_string(),
            主场景: "scenes/主场景.scene.ron".to_string(),
            设计分辨率: 默认设计分辨率(),
            全屏: true,
        };
        清单.写入(目录.path()).expect("写入");
        let 错误 = 游戏清单::读取(目录.path()).expect_err("新版本必须被拒绝");
        assert!(matches!(错误, 清单错误::版本过新 { .. }), "实际 {错误}");
    }

    #[test]
    fn 从项目生成清单并取启动场景() {
        let 目录 = 临时目录();
        let 根 = 目录.path().join("游戏工程");
        let mut 项目 = 项目描述::创建(&根, "游戏工程").expect("创建项目");
        let 场景 = 项目.场景路径("主场景").expect("场景路径");
        let 文档 = scene_doc::document::场景文档::新建("主场景");
        项目.保存场景(&场景, &文档).expect("保存场景");
        项目.清单可变().启动场景 = Some(场景.clone());
        项目.清单可变().设计分辨率 = (1280, 720);
        项目.保存清单().expect("保存清单");

        let 清单 = 游戏清单::从项目(&项目).expect("生成清单");
        assert_eq!(清单.名称, "游戏工程");
        assert_eq!(清单.主场景, 场景.文本());
        assert_eq!(清单.设计分辨率, (1280, 720));
        assert!(清单.全屏, "发布包默认全屏启动");
    }

    #[test]
    fn 没有场景的项目不生成清单() {
        let 目录 = 临时目录();
        let 根 = 目录.path().join("空工程");
        let 项目 = 项目描述::创建(&根, "空工程").expect("创建项目");
        assert!(项目.场景列表().is_empty(), "新项目不应自带场景文件");
        assert!(matches!(游戏清单::从项目(&项目), Err(清单错误::没有主场景)));
    }

    /// 打包脚本若用 PowerShell 的 `Set-Content -Encoding UTF8` 写清单，产物是
    /// **带 BOM** 的文本，而 RON 词法不认 BOM —— 那种清单会让游戏在启动瞬间
    /// 报"格式错误"，看起来像"包坏了"。读取端跳过 BOM，脚本端也用无 BOM 写出。
    #[test]
    fn 读取时跳过开头的UTF8BOM() {
        let 目录 = 临时目录();
        let 路径 = 目录.path().join(游戏清单文件名);
        let 文本 = "\u{feff}(
    格式版本: 1,
    名称: \"带签名的包\",
    主场景: \"scenes/主场景.scene.ron\",
    设计分辨率: (1280, 720),
    全屏: true,
)";
        std::fs::write(&路径, 文本).expect("写入");

        let 清单 = 游戏清单::读取(目录.path()).expect("带 BOM 的清单也必须能读");
        assert_eq!(清单.名称, "带签名的包");
        assert_eq!(清单.主场景, "scenes/主场景.scene.ron");
        assert_eq!(清单.设计分辨率, (1280, 720));
    }

    /// `build_release.ps1 -ExportGame` 从 `project.ron` 的文本里取
    /// 名称 / 启动场景 / 设计分辨率（脚本没有 RON 解析器，只能按字段取）。
    /// 这份夹具就是脚本的验收样本：三个字段都取自 serde/RON 的真实写法。
    #[test]
    fn 启动场景与设计分辨率在清单文本里是稳定写法() {
        let 目录 = 临时目录();
        let 根 = 目录.path().join("完整工程");
        let mut 项目 = 项目描述::创建(&根, "完整工程").expect("创建项目");
        let 场景 = 项目.场景路径("主场景").expect("场景路径");
        项目
            .保存场景(&场景, &scene_doc::document::场景文档::新建("主场景"))
            .expect("保存场景");
        项目.清单可变().启动场景 = Some(场景.clone());
        项目.清单可变().设计分辨率 = (1280, 720);
        项目.保存清单().expect("保存清单");

        let 文本 =
            std::fs::read_to_string(根.join(crate::project::项目清单文件名)).expect("读项目清单");
        assert!(
            文本.contains(&format!("启动场景: Some(\"{}\")", 场景.文本())),
            "Option 字段写成 `Some(\"...\")`，脚本的正例要认这个：{文本}"
        );
        assert!(文本.contains("设计分辨率: (1280, 720)"));
        assert!(文本.contains("名称: \"完整工程\""));
    }
}
