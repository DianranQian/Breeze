//! 打包：把"能跑的游戏"收集到一个可交付目录。
//!
//! # 参考的 Godot 设计
//!
//! Godot 的导出（`editor/export/`）分两步，且这一步的顺序很关键：
//!
//! 1. **先解析依赖**：`EditorExportPlatform::export_project_files` 遍历
//!    资源依赖图，得到"这个项目真正需要哪些文件"，缺一个就报错；
//! 2. **再写产物**：把依赖 + 可执行文件 + 项目配置写进目标目录。
//!
//! 本模块只做这两件事的**数据与文件层面**（可脱离 UI 单测）：
//! 生成 [`打包计划`]（依赖闭包 + 目标路径），校验缺失依赖，
//! 然后按计划复制。构建本身在 `editor_app::build` —— 打包消费它的产物。
//!
//! # 为什么不直接"把整个项目目录复制过去"
//!
//! 那会把 `src/`、`target/`、`imports.ron`、示例场景、废弃资源全部带进发行包，
//! 体积翻几倍，还会泄露源码。交付物应当只包含**启动场景可达的资源**
//! 加上可执行文件与项目清单。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{项目描述, 项目清单文件名};

/// 打包输出目录的默认名字（相对项目根）。
pub const 默认输出目录: &str = "dist";

/// 打包配置。
///
/// 与构建配置一样属于**项目契约**：同一个项目在 CI 与本地应当打出同样的包，
/// 因此它进项目清单（`project.ron` 的可选 `打包` 段）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 打包配置 {
    /// 输出目录（项目相对路径）。
    #[serde(default = "默认输出目录字符串")]
    pub 输出目录: String,
    /// 是否把全部场景都带上（默认只带启动场景及其引用）。
    #[serde(default)]
    pub 包含全部场景: bool,
    /// 是否在包内写入一份说明文件（文件清单，便于排查）。
    #[serde(default = "默认真")]
    pub 写入清单: bool,
}

fn 默认输出目录字符串() -> String {
    默认输出目录.to_string()
}

fn 默认真() -> bool {
    true
}

impl Default for 打包配置 {
    fn default() -> Self {
        Self {
            输出目录: 默认输出目录字符串(),
            包含全部场景: false,
            写入清单: true,
        }
    }
}

impl 打包配置 {
    /// 规范化的输出目录（去掉首尾空白与分隔符，空则回退默认值）。
    pub fn 输出目录路径(&self) -> String {
        let 清理 = self.输出目录.trim().replace('\\', "/");
        let 清理 = 清理.trim_matches('/');
        if 清理.is_empty() {
            默认输出目录.to_string()
        } else {
            清理.to_string()
        }
    }

    /// 输出目录是否可以安全使用：必须是项目内的相对路径。
    ///
    /// # 为什么必须校验
    ///
    /// 打包会**先清空输出目录**再写文件。如果用户填了绝对路径（`C:/Windows`）
    /// 或含 `..` 的越界路径，被清空的就不是项目内的目录 —— 这是不可挽回的破坏。
    /// 因此这条校验在设置层就做，而不是等到执行打包时再"尽量处理"。
    pub fn 输出目录合法(&self) -> Result<(), String> {
        let 原文 = self.输出目录.trim().replace('\\', "/");
        // 注意用原文而不是规范化后的路径判断绝对性：规范化会 trim 掉开头的 `/`，
        // 那一步会把 `/etc` 变成 `etc`，从而把绝对路径误判成合法相对路径。
        let 是绝对 = 原文.starts_with('/')
            || 原文.starts_with("//")
            // Windows 盘符（`C:` / `c:/`）。
            || (原文.len() >= 2 && 原文.as_bytes()[1] == b':');
        if 是绝对 {
            return Err(format!(
                "输出目录必须是项目内的相对路径，不能是绝对路径：{原文}"
            ));
        }
        if 原文.split('/').any(|段| 段 == "..") {
            return Err(format!("输出目录不能包含 `..`（会跑到项目外）：{原文}"));
        }
        Ok(())
    }
}

/// 计划里的一个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 打包条目 {
    /// 源绝对路径。
    pub 来源: PathBuf,
    /// 包内相对路径（`/` 分隔）。
    pub 目标: String,
    /// 字节大小。
    pub 大小: u64,
}

/// 打包计划：一份"要复制什么、到哪去"的清单。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct 打包计划 {
    /// 要复制的文件（按目标路径排序）。
    pub 条目: Vec<打包条目>,
    /// 缺失的依赖（场景引用但磁盘上不存在的资源）。
    pub 缺失依赖: Vec<String>,
}

impl 打包计划 {
    /// 总字节数。
    pub fn 总字节(&self) -> u64 {
        self.条目.iter().map(|条目| 条目.大小).sum()
    }

    /// 是否可以打包（缺失依赖会阻止打包）。
    pub fn 可以打包(&self) -> bool {
        self.缺失依赖.is_empty()
    }

    /// 各目标目录的文件数（供界面显示"assets 12 个、scenes 1 个"）。
    pub fn 目录统计(&self) -> BTreeMap<String, usize> {
        let mut 表 = BTreeMap::new();
        for 条目 in &self.条目 {
            let 目录 = 条目
                .目标
                .rsplit_once('/')
                .map(|(目录, _)| 目录.to_string())
                .unwrap_or_else(|| ".".to_string());
            *表.entry(目录).or_insert(0) += 1;
        }
        表
    }
}

/// 打包结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct 打包报告 {
    /// 是否成功。
    pub 成功: bool,
    /// 输出目录（绝对路径）。
    pub 输出目录: PathBuf,
    /// 复制的文件数。
    pub 文件数: usize,
    /// 总字节数。
    pub 总字节: u64,
    /// 失败项（文件 → 原因）。
    pub 失败项: Vec<(String, String)>,
}

impl 打包报告 {
    /// 供界面显示的一行摘要。
    pub fn 摘要(&self) -> String {
        if self.成功 {
            format!(
                "打包完成：{} 个文件 / {}",
                self.文件数,
                人类可读字节(self.总字节)
            )
        } else if !self.失败项.is_empty() {
            format!("打包失败：{} 个文件写入失败", self.失败项.len())
        } else {
            "打包失败".to_string()
        }
    }
}

/// 字节数的人类可读形式。
pub fn 人类可读字节(字节: u64) -> String {
    const 单位: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut 值 = 字节 as f64;
    let mut 下标 = 0;
    while 值 >= 1024.0 && 下标 + 1 < 单位.len() {
        值 /= 1024.0;
        下标 += 1;
    }
    if 下标 == 0 {
        format!("{字节} B")
    } else {
        format!("{:.1} {}", 值, 单位[下标])
    }
}

/// 生成打包计划。
///
/// `产物` 是构建得到的可执行文件绝对路径。若它为 `None`，
/// 计划里就没有可执行文件 —— 调用方应当先构建，而不是打一个"没有游戏"的包。
///
/// # 依赖闭包怎么算
///
/// 1. 起点：启动场景（清单里声明；未声明时取场景列表第一个）；
/// 2. 场景文件本身进包；读取它登记的 `资源表`，把每个资源加入待复制集合；
/// 3. `包含全部场景` 时把所有场景都作为起点。
///
/// 资源表是保存场景时同步维护的（见 `场景文档::同步资源表`），
/// 因此不需要在这里重新解析组件属性找资源路径。
pub fn 生成打包计划(
    项目: &项目描述, 配置: &打包配置, 产物: Option<&Path>
) -> 打包计划 {
    let mut 计划 = 打包计划::default();
    let mut 目标集合: BTreeSet<String> = BTreeSet::new();

    let 场景列表: Vec<scene_doc::path::项目相对路径> = if 配置.包含全部场景 {
        项目.场景列表()
    } else {
        项目
            .清单()
            .启动场景
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
            .into_iter()
            .chain(项目.场景列表().into_iter().take(1))
            .take(1)
            .collect()
    };

    for 场景 in 场景列表 {
        let 绝对 = 项目.绝对路径(&场景);
        if !绝对.is_file() {
            计划
                .缺失依赖
                .push(format!("启动场景不存在：{}", 场景.文本()));
            continue;
        }
        if 目标集合.insert(场景.文本().to_string()) {
            计划.条目.push(打包条目 {
                大小: std::fs::metadata(&绝对).map(|元| 元.len()).unwrap_or(0),
                来源: 绝对,
                目标: 场景.文本().to_string(),
            });
        }
        // 该场景登记的资源引用。
        let Ok(读取) = 项目.读取场景(&场景) else {
            continue;
        };
        for 资源 in 读取.文档.资源表.keys() {
            let 资源绝对 = 项目.绝对路径(资源);
            if !资源绝对.is_file() {
                计划
                    .缺失依赖
                    .push(format!("{} 引用的资源不存在：{}", 场景.文本(), 资源.文本()));
                continue;
            }
            if !目标集合.insert(资源.文本().to_string()) {
                continue;
            }
            计划.条目.push(打包条目 {
                大小: std::fs::metadata(&资源绝对).map(|元| 元.len()).unwrap_or(0),
                来源: 资源绝对,
                目标: 资源.文本().to_string(),
            });
        }
    }

    // 可执行文件放在包根目录（最直观的双击入口）。
    if let Some(产物) = 产物
        && 产物.is_file()
    {
        let 文件名 = 产物
            .file_name()
            .map(|名| 名.to_string_lossy().to_string())
            .unwrap_or_else(|| "game.exe".to_string());
        if 目标集合.insert(文件名.clone()) {
            计划.条目.push(打包条目 {
                大小: std::fs::metadata(产物).map(|元| 元.len()).unwrap_or(0),
                来源: 产物.to_path_buf(),
                目标: 文件名,
            });
        }
    }

    // 项目清单必须进包：没有它，游戏代码读不到启动场景等配置。
    let 清单路径 = 项目.清单路径();
    if 清单路径.is_file() && 目标集合.insert(项目清单文件名.to_string()) {
        计划.条目.push(打包条目 {
            大小: std::fs::metadata(&清单路径).map(|元| 元.len()).unwrap_or(0),
            来源: 清单路径,
            目标: 项目清单文件名.to_string(),
        });
    }

    计划.条目.sort_by(|甲, 乙| 甲.目标.cmp(&乙.目标));
    计划.缺失依赖.sort();
    计划.缺失依赖.dedup();
    计划
}

/// 执行打包：按计划把文件复制到输出目录。
///
/// # 失败处理
///
/// 单个文件复制失败**不会**中断整个过程：剩下的文件继续复制，
/// 失败的记进报告。这样用户能看到"12 个成功、1 个被占用"，
/// 而不是"打包失败"四个字然后不知道哪个文件出了问题。
///
/// 会先清空输出目录：发行包必须是"恰好这些文件"，
/// 残留的上一版资源会让用户以为改动没生效。
pub fn 执行打包(
    项目: &项目描述, 配置: &打包配置, 计划: &打包计划
) -> 打包报告 {
    let 输出目录 = 项目.根目录().join(配置.输出目录路径());
    if 输出目录.exists()
        && let Err(错误) = std::fs::remove_dir_all(&输出目录)
    {
        return 打包报告 {
            成功: false,
            输出目录: 输出目录.clone(),
            失败项: vec![(".".to_string(), format!("清空输出目录失败：{错误}"))],
            ..打包报告::default()
        };
    }

    let mut 报告 = 打包报告 {
        成功: true,
        输出目录: 输出目录.clone(),
        ..打包报告::default()
    };

    for 条目 in &计划.条目 {
        let 目标 = 输出目录.join(条目.目标.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(父) = 目标.parent()
            && let Err(错误) = std::fs::create_dir_all(父)
        {
            报告
                .失败项
                .push((条目.目标.clone(), format!("创建目录失败：{错误}")));
            continue;
        }
        match std::fs::copy(&条目.来源, &目标) {
            Ok(字节) => {
                报告.文件数 += 1;
                报告.总字节 += 字节;
            }
            Err(错误) => {
                报告
                    .失败项
                    .push((条目.目标.clone(), format!("复制失败：{错误}")));
            }
        }
    }

    if 配置.写入清单 {
        let 清单 = 生成清单文本(计划, &报告);
        if let Err(错误) = std::fs::write(输出目录.join("build-info.txt"), 清单) {
            报告
                .失败项
                .push(("build-info.txt".to_string(), format!("写入失败：{错误}")));
        }
    }

    报告.成功 = 报告.失败项.is_empty();
    报告
}

/// 生成包内说明文件（文件清单，便于用户与 CI 排查"包里到底有什么"）。
fn 生成清单文本(计划: &打包计划, 报告: &打包报告) -> String {
    let mut 文本 = String::new();
    文本.push_str("# 微风（Breeze）打包清单\n");
    文本.push_str(&format!(
        "# 共 {} 个文件，{}\n",
        计划.条目.len(),
        人类可读字节(计划.总字节())
    ));
    文本.push_str("# 目标                大小\n");
    for 条目 in &计划.条目 {
        文本.push_str(&format!("{:<28} {}\n", 条目.目标, 人类可读字节(条目.大小)));
    }
    if !报告.失败项.is_empty() {
        文本.push_str("\n# 写入失败：\n");
        for (文件, 原因) in &报告.失败项 {
            文本.push_str(&format!("#   {文件}：{原因}\n"));
        }
    }
    文本
}

/// 供界面显示的分组说明（例如 `assets/tps：7 个`）。
pub fn 分组说明(计划: &打包计划) -> Vec<String> {
    计划
        .目录统计()
        .into_iter()
        .map(|(目录, 数量)| format!("{目录}: {数量}"))
        .collect()
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 建项目() -> (tempfile::TempDir, 项目描述) {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 根 = 目录.path().join("打包工程");
        let 项目 = 项目描述::创建(&根, "打包工程").expect("创建项目");
        (目录, 项目)
    }

    /// 写一个场景，登记它引用的资源。
    fn 写场景(
        项目: &项目描述,
        名称: &str,
        资源: &[&str],
    ) -> scene_doc::path::项目相对路径 {
        let 路径 = scene_doc::path::项目相对路径::解析(format!("scenes/{名称}.scene.ron")).unwrap();
        let mut 文档 = scene_doc::document::场景文档::新建(名称);
        文档.生成实体("根", None);
        for 资源 in 资源 {
            let 资源路径 = scene_doc::path::项目相对路径::解析(资源).unwrap();
            文档.资源表.insert(
                资源路径.clone(),
                scene_doc::document::资源引用 {
                    路径: 资源路径,
                    唯一标识: None,
                },
            );
        }
        项目.保存场景(&路径, &文档).expect("保存场景");
        路径
    }

    fn 写文件(项目: &项目描述, 相对: &str, 内容: &[u8]) -> PathBuf {
        let 路径 = 项目.根目录().join(相对);
        if let Some(父) = 路径.parent() {
            std::fs::create_dir_all(父).expect("建目录");
        }
        std::fs::write(&路径, 内容).expect("写文件");
        路径
    }

    #[test]
    fn 计划只包含启动场景可达的资源() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/hero.png"]);
        写场景(&项目, "废弃场景", &["assets/unused.png"]);
        写文件(&项目, "assets/hero.png", b"hero");
        写文件(&项目, "assets/unused.png", b"unused");
        项目.清单可变().启动场景 = Some(主场景.clone());
        项目.保存清单().expect("保存清单");

        let 计划 = 生成打包计划(&项目, &打包配置::default(), None);
        assert!(计划.可以打包(), "缺失依赖：{:?}", 计划.缺失依赖);
        let 目标: Vec<&str> = 计划.条目.iter().map(|条目| 条目.目标.as_str()).collect();
        assert_eq!(
            目标,
            vec!["assets/hero.png", "project.ron", "scenes/主场景.scene.ron"],
            "包里只应当有启动场景与它引用的资源"
        );
        assert!(
            !目标.iter().any(|目标| 目标.contains("unused")),
            "未被引用的资源不能进包"
        );
    }

    #[test]
    fn 包含全部场景时把其余场景也带上() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &[]);
        写场景(&项目, "第二场景", &[]);
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");

        let 配置 = 打包配置 {
            包含全部场景: true,
            ..打包配置::default()
        };
        let 计划 = 生成打包计划(&项目, &配置, None);
        let 目标: Vec<&str> = 计划.条目.iter().map(|条目| 条目.目标.as_str()).collect();
        assert!(目标.contains(&"scenes/第二场景.scene.ron"));
        assert!(目标.contains(&"scenes/主场景.scene.ron"));
    }

    #[test]
    fn 缺失引用会阻止打包() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/丢失.png"]);
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");

        let 计划 = 生成打包计划(&项目, &打包配置::default(), None);
        assert!(!计划.可以打包());
        assert_eq!(计划.缺失依赖.len(), 1);
        assert!(计划.缺失依赖[0].contains("丢失.png"));
    }

    #[test]
    fn 有产物时把它放在包根目录() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &[]);
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");
        let 产物 = 写文件(&项目, "target/debug/mini_game.exe", b"exe-bytes");

        let 计划 = 生成打包计划(&项目, &打包配置::default(), Some(&产物));
        let 条目 = 计划
            .条目
            .iter()
            .find(|条目| 条目.目标 == "mini_game.exe")
            .expect("产物应当在包根目录");
        assert_eq!(条目.大小, 9);
    }

    #[test]
    fn 执行打包会复制文件并写清单() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/hero.png"]);
        写文件(&项目, "assets/hero.png", b"hero-bytes");
        let 产物 = 写文件(&项目, "target/debug/mini_game.exe", b"EXE");
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");

        let 配置 = 打包配置::default();
        let 计划 = 生成打包计划(&项目, &配置, Some(&产物));
        let 报告 = 执行打包(&项目, &配置, &计划);

        assert!(报告.成功, "失败项：{:?}", 报告.失败项);
        assert_eq!(报告.文件数, 计划.条目.len());
        let 输出 = 项目.根目录().join(默认输出目录);
        assert_eq!(报告.输出目录, 输出);
        for 相对 in [
            "scenes/主场景.scene.ron",
            "assets/hero.png",
            "mini_game.exe",
            "project.ron",
            "build-info.txt",
        ] {
            assert!(
                输出
                    .join(相对.replace('/', std::path::MAIN_SEPARATOR_STR))
                    .is_file(),
                "包里缺少 {相对}"
            );
        }
        let 清单 = std::fs::read_to_string(输出.join("build-info.txt")).expect("读清单");
        assert!(清单.contains("assets/hero.png"));
    }

    #[test]
    fn 再次打包会清空旧产物() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &[]);
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");
        let 输出 = 项目.根目录().join(默认输出目录);
        std::fs::create_dir_all(&输出).unwrap();
        std::fs::write(输出.join("上一版的残留.bin"), b"stale").unwrap();

        let 配置 = 打包配置::default();
        let 计划 = 生成打包计划(&项目, &配置, None);
        let 报告 = 执行打包(&项目, &配置, &计划);
        assert!(报告.成功);
        assert!(
            !输出.join("上一版的残留.bin").exists(),
            "输出目录必须先清空，否则旧文件会让用户以为改动没生效"
        );
    }

    #[test]
    fn 输出目录必须是项目内相对路径() {
        let 造配置 = |目录: &str| 打包配置 {
            输出目录: 目录.to_string(),
            ..打包配置::default()
        };
        // 合法。
        assert!(造配置("dist").输出目录合法().is_ok());
        assert!(造配置("build/package").输出目录合法().is_ok());
        // 绝对路径必须被拒绝：打包会先清空输出目录，这是不可挽回的操作。
        assert!(造配置("C:/Windows").输出目录合法().is_err());
        assert!(造配置("/etc").输出目录合法().is_err());
        assert!(造配置("//server/share").输出目录合法().is_err());
        // 越界路径同样拒绝。
        assert!(造配置("../外面").输出目录合法().is_err());
        assert!(造配置("a/../../b").输出目录合法().is_err());
    }

    #[test]
    fn 输出目录规范化与字节格式化() {
        let 配置 = 打包配置 {
            输出目录: "  \\build/out/  ".into(),
            ..打包配置::default()
        };
        assert_eq!(配置.输出目录路径(), "build/out");
        let 空 = 打包配置 {
            输出目录: "   ".into(),
            ..打包配置::default()
        };
        assert_eq!(空.输出目录路径(), 默认输出目录);
        assert_eq!(人类可读字节(512), "512 B");
        assert_eq!(人类可读字节(2048), "2.0 KB");
    }
}
