//! 编辑器自身的配置持久化：窗口布局、最近项目。
//!
//! # 为什么必须与项目/场景文件分开存放
//!
//! 需求里有一条明确的一致性要求：
//!
//! > 窗口布局持久化与游戏场景持久化互不干扰。
//!
//! 这两类数据的性质完全不同：
//!
//! | | 场景文件 | 编辑器配置 |
//! | --- | --- | --- |
//! | 归属 | **属于项目**，要进版本控制 | 属于**这台机器上的这个用户** |
//! | 内容 | 游戏数据 | 窗口布局、最近打开的项目 |
//! | 换台机器 | 必须能原样打开 | 应当重新开始，不该带着别人的窗口尺寸 |
//!
//! 如果把窗口布局写进项目目录（例如 `project.ron` 旁边），会立刻产生
//! 两个问题：
//!
//! 1. 团队协作时每个人提交自己的窗口尺寸，产生无意义的 diff 冲突；
//! 2. 场景文件的"未保存修改"判定会被这些噪声干扰。
//!
//! 因此配置文件放在**用户配置目录**下，与项目目录完全无关。
//! 项目被删掉、被移动，都不影响配置；配置损坏也不影响任何项目。
//!
//! # 配置文件损坏的处理
//!
//! 配置是"方便"而不是"资产"，因此策略与场景文件相反：
//! **损坏就直接用默认值并覆盖重写**，不尝试备份恢复。
//! 让用户因为一个坏的布局文件而打不开编辑器，是本末倒置。

use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// 配置文件的格式版本。
///
/// 与场景文件同样的策略：加字段不升版本（靠 `serde(default)`），
/// 改语义才升版本；读到更高版本就用默认值覆盖，
/// 因为配置丢了没有任何损失。
///
/// 版本历史：
///
/// * `1`：只有一个 `布局` 字段（单 DockState）；
/// * `2`：`工作区` 字段承载命名工作区集，`布局` 降级为迁移用的旧字段。
pub const 配置格式版本: u32 = 2;

/// 配置文件在配置目录下的文件名。
pub const 配置文件名: &str = "config.ron";

/// 编辑器配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 编辑器配置 {
    /// 格式版本。
    pub 格式版本: u32,
    /// 窗口布局的 RON 文本。`None` 表示使用默认布局。
    ///
    /// # 这是**旧字段**，只为迁移而保留
    ///
    /// 格式版本 1 只保存一份布局。版本 2 起改为命名工作区集
    /// （见 [`编辑器配置::工作区`]）。读到非空的 `布局` 而 `工作区` 为空时，
    /// 启动流程会把它迁移成一个"迁移的布局"工作区 —— 用户上次看到的
    /// 布局必须原样出现，不能被悄悄换成出厂布局。
    ///
    /// 存成文本而不是结构化字段，是为了让这个类型**不依赖
    /// `egui_dock` 的布局内部结构** —— 布局格式变化时，
    /// 只会导致这一项解析失败并回退到默认布局，
    /// 不会让整个配置文件连同"最近项目"一起失效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 布局: Option<String>,
    /// 命名工作区集的 RON 文本（格式版本 2 起）。
    ///
    /// 同样存成文本：工作区集的解析失败可以整体回退到内置工作区，
    /// 而不会让配置文件里其它字段跟着作废。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 工作区: Option<String>,
    /// 主题库的 RON 文本（格式版本 2 起）。
    ///
    /// 主题与工作区是两个独立的配置维度：一个存颜色/外观，一个存布局。
    /// 分开存的好处是"删掉主题配置不会丢工作区"、"换主题不会改布局"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 主题: Option<String>,
    /// 最近打开的项目目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 最近项目: Option<PathBuf>,
    /// 界面搜索框等轻量偏好的持久化预留。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub 偏好: std::collections::BTreeMap<String, String>,
}

impl Default for 编辑器配置 {
    fn default() -> Self {
        Self {
            格式版本: 配置格式版本,
            布局: None,
            工作区: None,
            主题: None,
            最近项目: None,
            偏好: std::collections::BTreeMap::new(),
        }
    }
}

impl 编辑器配置 {
    /// 载入配置。文件不存在、损坏、或版本过新时返回默认配置。
    ///
    /// **不返回错误**：配置是便利性功能，读不到就用默认值，
    /// 不该阻止编辑器启动。
    pub fn 载入(路径: &std::path::Path) -> Self {
        let Ok(文本) = std::fs::read_to_string(路径) else {
            return Self::default();
        };

        match ron::from_str::<Self>(&文本) {
            Ok(配置) if 配置.格式版本 <= 配置格式版本 => 配置,
            Ok(配置) => {
                warn!(
                    "编辑器配置格式版本 {} 高于本编辑器支持的 {}，已改用默认配置",
                    配置.格式版本, 配置格式版本
                );
                Self::default()
            }
            Err(错误) => {
                warn!("编辑器配置解析失败，已改用默认配置：{错误}");
                Self::default()
            }
        }
    }

    /// 保存配置。返回是否成功。
    pub fn 保存(&self, 路径: &std::path::Path) -> Result<(), String> {
        if let Some(父目录) = 路径.parent() {
            std::fs::create_dir_all(父目录)
                .map_err(|错误| format!("无法创建配置目录 {}：{错误}", 父目录.display()))?;
        }
        let 文本 = ron::ser::to_string_pretty(
            self,
            ron::ser::PrettyConfig::default().new_line("\n".to_string()),
        )
        .map_err(|错误| format!("配置序列化失败：{错误}"))?;

        // 与场景保存同样的三段式思路，但更轻：配置不值得为它维护备份，
        // 但仍然要避免"写到一半崩溃留下半截文件"导致下次启动读不了。
        let 临时 = 路径.with_extension("ron.tmp");
        std::fs::write(&临时, 文本).map_err(|错误| format!("写入临时配置失败：{错误}"))?;
        std::fs::rename(&临时, 路径).map_err(|错误| {
            let _ = std::fs::remove_file(&临时);
            format!("替换配置文件失败：{错误}")
        })
    }
}

/// 从环境变量读取配置基准目录并追加子路径。
///
/// 抽成辅助函数而不是写三遍 `if let Ok(x) = var(..) { if !x.is_empty() { .. } }`：
/// 三段重复的嵌套判断既啰嗦又容易漏掉"空字符串"这一情况
/// （Windows 上 `APPDATA` 偶尔会是空串）。
fn 取环境目录(变量名: &str, 子路径: &[&str]) -> Option<PathBuf> {
    let 值 = std::env::var(变量名).ok()?;
    if 值.is_empty() {
        return None;
    }
    let mut 路径 = PathBuf::from(值);
    for 段 in 子路径 {
        路径.push(段);
    }
    Some(路径)
}

/// 解析配置目录。
///
/// 优先用操作系统的用户配置目录；取不到时退回进程当前目录下的
/// `.bevy_studio`，保证任何环境下都能工作（哪怕位置不理想）。
pub fn 配置目录() -> PathBuf {
    // Windows: %APPDATA%\BevyStudio
    // 类 Unix: $XDG_CONFIG_HOME/bevy-studio，或 $HOME/.config/bevy-studio
    if let Some(路径) = 取环境目录("APPDATA", &["BevyStudio"]) {
        return 路径;
    }
    if let Some(路径) = 取环境目录("XDG_CONFIG_HOME", &["bevy-studio"]) {
        return 路径;
    }
    if let Some(路径) = 取环境目录("HOME", &[".config", "bevy-studio"]) {
        return 路径;
    }
    // 所有环境变量都拿不到时的兜底：放在进程当前目录下。
    // 位置不理想，但至少配置能存下来，功能不会失效。
    PathBuf::from(".bevy_studio")
}

/// 配置文件的完整路径。
pub fn 配置文件路径() -> PathBuf {
    配置目录().join(配置文件名)
}

/// 编辑器配置资源。
#[derive(Resource, Debug, Default)]
pub struct 编辑器配置资源(pub 编辑器配置);

/// 布局是否有待写盘的改动。
///
/// 用一个脏标记而不是每帧写盘：布局拖动过程中每帧都会变化，
/// 每帧写文件既浪费 IO，也可能在用户还在拖的时候反复覆盖。
#[derive(Resource, Debug, Default)]
pub struct 布局待保存 {
    /// 距上次保存已过的秒数。
    pub 累计秒: f32,
    /// 是否有未写盘的改动。
    pub 脏: bool,
}

/// 布局写盘的最小间隔（秒）。
///
/// 用户拖动分隔条时会连续产生变化，攒够这段时间再写一次。
/// 取 2 秒是"不会明显丢改动"与"不会频繁写盘"之间的折中。
pub const 布局写盘间隔: f32 = 2.0;

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 配置往返一致() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("config.ron");

        let 配置 = 编辑器配置 {
            格式版本: 配置格式版本,
            布局: Some("(布局内容)".to_string()),
            工作区: Some("(工作区内容)".to_string()),
            主题: Some("(主题内容)".to_string()),
            最近项目: Some(PathBuf::from("C:/games/我的游戏")),
            偏好: [("主题".to_string(), "深色".to_string())]
                .into_iter()
                .collect(),
        };
        配置.保存(&路径).expect("保存应当成功");

        let 载入 = 编辑器配置::载入(&路径);
        assert_eq!(载入, 配置);
    }

    #[test]
    fn 缺失文件返回默认配置而不是报错() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("根本不存在.ron");
        let 配置 = 编辑器配置::载入(&路径);
        assert_eq!(配置, 编辑器配置::default());
        assert!(配置.布局.is_none());
    }

    #[test]
    fn 损坏的配置文件回退到默认值() {
        // 配置是"方便"不是"资产"：坏了就用默认值，
        // 绝不能因为一个坏布局文件让编辑器打不开。
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("config.ron");
        std::fs::write(&路径, "这不是合法的 RON {{{").unwrap();

        let 配置 = 编辑器配置::载入(&路径);
        assert_eq!(配置, 编辑器配置::default());
    }

    #[test]
    fn 版本过新的配置回退到默认值() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("config.ron");
        std::fs::write(&路径, "(格式版本: 9999)").unwrap();

        let 配置 = 编辑器配置::载入(&路径);
        assert_eq!(
            配置.格式版本, 配置格式版本,
            "版本过新时应改用默认配置而不是照读"
        );
    }

    #[test]
    fn 缺少可选字段的旧配置仍可加载() {
        // 只有格式版本的最小配置，模拟早期版本或手写的配置。
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("config.ron");
        std::fs::write(&路径, "(格式版本: 1)").unwrap();

        let 配置 = 编辑器配置::载入(&路径);
        assert_eq!(配置.格式版本, 1);
        assert!(配置.布局.is_none());
        assert!(配置.工作区.is_none());
        assert!(配置.最近项目.is_none());
    }

    #[test]
    fn 旧版单布局配置能被识别为待迁移() {
        // 格式版本 1 + 仅有布局字段：启动流程应当把它迁移成工作区。
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("config.ron");
        std::fs::write(&路径, "(格式版本: 1, 布局: Some(\"(x)\"))").unwrap();

        let 配置 = 编辑器配置::载入(&路径);
        assert_eq!(配置.格式版本, 1, "旧配置必须被读进来而不是丢掉");
        assert_eq!(配置.布局.as_deref(), Some("(x)"));
        assert!(配置.工作区.is_none(), "旧配置没有工作区字段");
    }

    #[test]
    fn 保存会创建缺失的父目录() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("深层").join("嵌套").join("config.ron");
        assert!(!路径.parent().unwrap().exists());

        let 配置 = 编辑器配置::default();
        配置.保存(&路径).expect("应当自动创建父目录");
        assert!(路径.is_file());
    }

    #[test]
    fn 保存不留下临时文件() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join("config.ron");
        编辑器配置::default().保存(&路径).unwrap();

        let 残留: Vec<String> = std::fs::read_dir(目录.path())
            .unwrap()
            .filter_map(|项| 项.ok())
            .map(|项| 项.file_name().to_string_lossy().to_string())
            .filter(|名| 名.ends_with(".tmp"))
            .collect();
        assert!(残留.is_empty(), "不应残留临时配置文件：{残留:?}");
    }

    #[test]
    fn 配置目录可从环境变量推导() {
        // 只验证"能解析出一个路径并且带文件名"，
        // 不假设具体平台的环境变量内容。
        let 路径 = 配置文件路径();
        assert_eq!(
            路径.file_name().and_then(|名| 名.to_str()),
            Some(配置文件名)
        );
        assert!(路径.parent().is_some(), "配置路径应当有父目录");
    }
}
