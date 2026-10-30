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

use crate::game_manifest::{游戏清单, 游戏清单文件名};
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
    /// 全量复制 `assets/` 时要排除的路径前缀（相对 `assets/` 根，`/` 分隔）。
    ///
    /// 只做**前缀**匹配：`raw_src` 排除 `raw_src/` 下的全部文件，不排除
    /// `raw_src2/`（按目录边界比较，不是按子串）。不支持 glob ——
    /// 通配符的边界语义（是否跨目录、是否匹配隐藏文件）很容易写出一条
    /// "看起来对、实际多排除了东西"的规则，前缀匹配简单且可预测。
    ///
    /// 只在带用户代码的工程（声明了运行或构建配置）里生效：场景闭包模式
    /// 只复制场景引用的资源，排除项无从下手，写了也不会静默改变结果。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub 排除资源: Vec<String>,
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
            排除资源: Vec::new(),
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

    /// 规范化后的排除前缀（相对 `assets/` 根，`/` 分隔，已去重）。
    ///
    /// 规范化只做三件事：去首尾空白、反斜杠统一成 `/`、去掉开头的 `./`
    /// 与结尾的 `/`。**不**把开头的 `/` 去掉 —— 那会把用户误写的绝对路径
    /// 静默变成合法前缀；保留原样它就永远匹配不到，结果与用户设想一致：
    /// 什么都没排除。空项直接丢弃，否则空前缀会匹配全部资源。
    pub fn 排除前缀(&self) -> Vec<String> {
        let mut 结果: Vec<String> = Vec::new();
        for 原文 in &self.排除资源 {
            let 清理 = 原文.trim().replace('\\', "/");
            let 清理 = 清理.trim_start_matches("./").trim_end_matches('/').trim();
            if 清理.is_empty() {
                continue;
            }
            if !结果.iter().any(|已有| 已有 == 清理) {
                结果.push(清理.to_string());
            }
        }
        结果
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
    /// 是否由场景闭包带进来（场景文件本身与它登记的引用都为 `true`）。
    ///
    /// 全量复制补齐的资源、可执行文件与项目清单为 `false`。
    /// 导出摘要据此区分"场景确实在用"与"可能只在代码里用"，
    /// 后者是用户判断排除项该写什么的依据。
    pub 被场景引用: bool,
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
/// # 两种收集模式
///
/// | 工程 | 收集方式 |
/// | --- | --- |
/// | 纯微风（没声明运行/构建） | 场景闭包：只带启动场景（或全部场景）与它引用的资源 |
/// | 带用户代码（声明了运行或构建） | 先算场景闭包，再全量复制 `assets/` |
///
/// 分叉的原因是**资源的发现方式不同**：纯微风工程的资源只能由场景引用，
/// 闭包就是全集；带用户代码的工程用 `asset_server.load("…")` 加载资源，
/// 路径可能来自 `format!`、配置文件或 `load_folder`，静态分析必然漏，
/// 漏掉的资源在玩家机器上表现为贴图缺失 / 模型不显示。
///
/// 全量复制时补进来的条目 `被场景引用` 为 `false`；
/// `配置.排除资源` 的前缀在补文件时生效。
///
/// # 闭包怎么算
///
/// 1. 起点：启动场景（清单里声明；未声明时取场景列表第一个）；
/// 2. 场景文件本身进包；读取它登记的 `资源表`，把每个资源加入待复制集合；
/// 3. `包含全部场景` 时把所有场景都作为起点。
///
/// 资源表是保存场景时同步维护的（见 `场景文档::同步资源表`），
/// 因此不需要在这里重新解析组件属性找资源路径。
///
/// # 排除项与场景引用冲突
///
/// 某个前缀盖住了场景正在引用的资源时，工程配置自相矛盾：照排除打出来的
/// 包会缺资源，照场景打出来的包又违背了排除项。这里不替用户选，而是把
/// **点名到具体路径**的冲突写进 `缺失依赖` 阻止打包 —— 沉默地选一边，
/// 用户只会在运行游戏时才发现资源没了。
///
/// # 扫描安全
///
/// 补文件复用 [`项目描述::资源列表`]：不跟随符号链接、深度上限 20 层
/// （见 `项目描述::扫描目录`），否则一个指回上层目录的链接就能让导出卡死。
pub fn 生成打包计划(
    项目: &项目描述, 配置: &打包配置, 产物: Option<&Path>
) -> 打包计划 {
    let mut 计划 = 打包计划::default();
    let mut 目标集合: BTreeSet<String> = BTreeSet::new();
    let 全量复制 = 带用户代码(项目);
    let 排除前缀 = 配置.排除前缀();

    let 场景列表: Vec<scene_doc::path::项目相对路径> = if 配置.包含全部场景 {
        项目.场景列表()
    } else {
        项目.启动场景().into_iter().collect()
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
                被场景引用: true,
            });
        }
        // 该场景登记的资源引用。
        let Ok(读取) = 项目.读取场景(&场景) else {
            continue;
        };
        for 资源 in 读取.文档.资源表.keys() {
            let 资源文本 = 资源.文本().to_string();
            if 全量复制
                && let Some(相对) = 资产相对路径(&资源文本)
                && let Some(前缀) = 被排除(相对, &排除前缀)
            {
                计划.缺失依赖.push(format!(
                    "场景 {} 引用的资源 {} 被排除项 `{}` 覆盖：排除项不能排除场景正在使用的资源",
                    场景.文本(),
                    资源文本,
                    前缀
                ));
                continue;
            }
            let 资源绝对 = 项目.绝对路径(资源);
            if !资源绝对.is_file() {
                计划
                    .缺失依赖
                    .push(format!("{} 引用的资源不存在：{}", 场景.文本(), 资源文本));
                continue;
            }
            if !目标集合.insert(资源文本.clone()) {
                continue;
            }
            计划.条目.push(打包条目 {
                大小: std::fs::metadata(&资源绝对).map(|元| 元.len()).unwrap_or(0),
                来源: 资源绝对,
                目标: 资源文本,
                被场景引用: true,
            });
        }
    }

    // 带用户代码的工程：代码加载的资源无法静态枚举，整份带上 assets/。
    if 全量复制 {
        for 资源 in 项目.资源列表() {
            let 文本 = 资源.路径.文本().to_string();
            if 目标集合.contains(&文本) {
                continue;
            }
            if let Some(相对) = 资产相对路径(&文本)
                && 被排除(相对, &排除前缀).is_some()
            {
                continue;
            }
            计划.条目.push(打包条目 {
                大小: 资源.大小,
                来源: 项目.绝对路径(&资源.路径),
                目标: 文本.clone(),
                被场景引用: false,
            });
            目标集合.insert(文本);
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
                被场景引用: false,
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
            被场景引用: false,
        });
    }

    计划.条目.sort_by(|甲, 乙| 甲.目标.cmp(&乙.目标));
    计划.缺失依赖.sort();
    计划.缺失依赖.dedup();
    计划
}

/// 这个工程是不是"带用户代码"：声明了运行目标或构建配置。
///
/// 只看**有没有声明**，不看是否构建成功：收集范围由工程契约决定，
/// 不能因为这次构建失败就悄悄退回闭包模式，否则同一份工程在不同机器上
/// 会打出内容不同的包。
fn 带用户代码(项目: &项目描述) -> bool {
    let 清单 = 项目.清单();
    清单.运行.as_ref().is_some_and(|运行| 运行.已声明()) || 清单.构建.is_some()
}

/// 项目相对路径里 `assets/` 之后的部分（排除前缀按资产根匹配）。
fn 资产相对路径(路径: &str) -> Option<&str> {
    路径
        .strip_prefix(crate::project::目录名::资源)
        .and_then(|剩余| 剩余.strip_prefix('/'))
}

/// 命中的排除前缀（按目录边界比较：`tex` 不覆盖 `textures/a.png`）。
fn 被排除<'a>(资产相对: &str, 前缀表: &'a [String]) -> Option<&'a str> {
    前缀表.iter().map(String::as_str).find(|前缀| {
        资产相对 == *前缀
            || 资产相对
                .strip_prefix(前缀)
                .is_some_and(|剩余| 剩余.starts_with('/'))
    })
}

/// 按计划把文件复制到 `输出目录`，返回尚未判定成功的报告。
///
/// # 失败处理
///
/// 单个文件复制失败**不会**中断整个过程：剩下的文件继续复制，
/// 失败的记进报告。这样用户能看到"12 个成功、1 个被占用"，
/// 而不是"打包失败"四个字然后不知道哪个文件出了问题。
///
/// 报告里的 `成功` 先按 `true` 记，最终值由 [`补齐生成物`] 决定：
/// 清单写不进去同样算打包失败，调用方不该在两处各判一次。
fn 复制计划(计划: &打包计划, 输出目录: &Path) -> 打包报告 {
    let mut 报告 = 打包报告 {
        成功: true,
        输出目录: 输出目录.to_path_buf(),
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

    报告
}

/// 复制之后补齐包内生成物：游戏清单与说明文件，并判定最终成功与否。
///
/// # 为什么清单不是计划里的一条
///
/// `game_manifest.ron` 的内容由项目清单推导（见 [`游戏清单::从项目`]），
/// 磁盘上没有对应的源文件可复制，因此它在复制阶段之后单独生成。
/// 项目一个场景都没有时不写清单：没有入口的目录不是可玩的包，
/// 写了清单只会让运行时加载到空入口。
///
/// `全屏` 是清单里的启动窗口模式：编辑器内打包取发布默认值 `true`，
/// 命令行导出可用 `--windowed` 覆盖成 `false`。
fn 补齐生成物(
    项目: &项目描述,
    计划: &打包计划,
    输出目录: &Path,
    全屏: bool,
    写入说明: bool,
    mut 报告: 打包报告,
) -> 打包报告 {
    let mut 附加文件: Vec<(String, u64)> = Vec::new();
    match 游戏清单::从项目(项目) {
        Ok(mut 清单) => {
            清单.全屏 = 全屏;
            match 清单.写入(输出目录) {
                Ok(路径) => {
                    let 字节 = std::fs::metadata(&路径).map(|元| 元.len()).unwrap_or(0);
                    附加文件.push((游戏清单文件名.to_string(), 字节));
                }
                Err(错误) => 报告
                    .失败项
                    .push((游戏清单文件名.to_string(), 错误.to_string())),
            }
        }
        // 空项目：没有主场景就没有可玩的入口，不写清单（见文档说明）。
        Err(错误) => {
            if !matches!(错误, crate::game_manifest::清单错误::没有主场景) {
                报告
                    .失败项
                    .push((游戏清单文件名.to_string(), 错误.to_string()));
            }
        }
    }

    if 写入说明 {
        let 清单 = 生成清单文本(计划, &报告, &附加文件);
        if let Err(错误) = std::fs::write(输出目录.join("build-info.txt"), 清单) {
            报告
                .失败项
                .push(("build-info.txt".to_string(), format!("写入失败：{错误}")));
        }
    }

    报告.成功 = 报告.失败项.is_empty();
    报告
}

/// 执行打包：按计划把文件复制到输出目录，并写游戏运行清单。
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

    let 报告 = 复制计划(计划, &输出目录);
    补齐生成物(项目, 计划, &输出目录, true, 配置.写入清单, 报告)
}

/// 导出一份可玩包到 `输出目录`（任意位置，不要求位于工程内）。
///
/// # 与编辑器内「打包」的关系
///
/// 共用 [`生成打包计划`]、[`复制计划`] 与 [`补齐生成物`]，因此包结构、
/// 依赖闭包、清单格式完全一致。差别只有三点：
///
/// 1. 输出目录由调用方给（脚本传 `-GameOutDir`），不写进 `project.ron`；
/// 2. 带上全部场景 —— 导出的是"这个工程的完整游戏"，而不是启动场景最小包；
/// 3. `全屏` 可由命令行覆盖（编辑器内固定按发布默认全屏）。
///
/// `排除资源` 与 `写入清单` 仍取项目清单里的 `打包` 段：它们是项目契约，
/// 命令行导出只覆盖"输出到哪、带几个场景、窗口模式"，不另立一套配置。
///
/// # 为什么输出目录校验放在这里
///
/// 打包会先清空输出目录：`-GameOutDir` 一旦填成工程目录或它的上级，
/// 清空的就是用户的工作目录。脚本原先用四段字符串比较挡这件事，
/// 现在收回 Rust 侧，工程根与产物都参与判断，脚本不再各写一份。
pub fn 导出可玩包(
    工程根: &Path,
    输出目录: &Path,
    产物: &Path,
    全屏: bool,
) -> Result<打包报告, String> {
    let 项目 = 项目描述::打开(工程根).map_err(|错误| format!("打开工程失败：{错误}"))?;
    if !产物.is_file() {
        return Err(format!("可执行文件不存在：{}", 产物.display()));
    }
    let 输出 = std::path::absolute(输出目录).map_err(|错误| format!("输出目录路径非法：{错误}"))?;
    校验导出路径(项目.根目录(), &输出, 产物)?;
    if 项目.启动场景().is_none() {
        return Err(format!(
            "{} 的 scenes/ 里没有场景；请先在编辑器里保存一个场景并设为启动场景",
            项目.根目录().display()
        ));
    }

    let mut 配置 = 项目.清单().打包.clone().unwrap_or_default();
    配置.包含全部场景 = true;
    let mut 计划 = 生成打包计划(&项目, &配置, Some(产物));
    if !计划.可以打包() {
        return Err(format!(
            "工程有 {} 处缺失依赖，拒绝导出：\n  {}",
            计划.缺失依赖.len(),
            计划.缺失依赖.join("\n  ")
        ));
    }
    追加调试符号(&mut 计划, 产物);

    if 输出.exists()
        && let Err(错误) = std::fs::remove_dir_all(&输出)
    {
        return Err(format!("清空输出目录失败：{错误}"));
    }

    let 报告 = 复制计划(&计划, &输出);
    Ok(补齐生成物(&项目, &计划, &输出, 全屏, true, 报告))
}

/// 产物旁边有 `.pdb` 时一并进包：崩溃报告要符号才能定位，缺了只能看地址。
fn 追加调试符号(计划: &mut 打包计划, 产物: &Path) {
    let 符号 = 产物.with_extension("pdb");
    if !符号.is_file() {
        return;
    }
    let Some(名称) = 符号.file_name().map(|名| 名.to_string_lossy().to_string()) else {
        return;
    };
    if 计划.条目.iter().any(|条目| 条目.目标 == 名称) {
        return;
    }
    计划.条目.push(打包条目 {
        大小: std::fs::metadata(&符号).map(|元| 元.len()).unwrap_or(0),
        来源: 符号,
        目标: 名称,
        被场景引用: false,
    });
    计划.条目.sort_by(|甲, 乙| 甲.目标.cmp(&乙.目标));
}

/// 挡住"清空输出目录"会连带删掉别的东西的三种情况。
///
/// 1. 输出目录就是工程目录 —— 等于删掉工程；
/// 2. 输出目录是工程目录的上级 —— 等于删掉工程所在的整棵树；
/// 3. 可执行文件在输出目录里 —— 清空时先把自己删了，复制必然失败。
///
/// 输出目录允许在工程内（编辑器的 `dist/` 就是这种布局），因此只拦
/// "工程位于输出目录之下"，不拦反过来把输出目录放进工程。
fn 校验导出路径(工程根: &Path, 输出: &Path, 产物: &Path) -> Result<(), String> {
    let 工程 = 比较文本(工程根);
    let 输出文本 = 比较文本(输出);
    if 相同或子路径(&输出文本, &工程) {
        return Err(format!(
            "输出目录不能是工程目录或它的上级（导出会先清空输出目录）：{}",
            输出.display()
        ));
    }
    if 相同或子路径(&输出文本, &比较文本(产物)) {
        return Err(format!(
            "可执行文件在输出目录里，清空输出目录会把它删掉：{}",
            产物.display()
        ));
    }
    Ok(())
}

/// 绝对路径的规范化比较文本：分隔符统一成 `/`、去尾分隔符，
/// Windows 上大小写不敏感（否则 `c:\repo` 能绕过 `C:\repo` 的判断）。
fn 比较文本(路径: &Path) -> String {
    let 绝对 = std::path::absolute(路径).unwrap_or_else(|_| 路径.to_path_buf());
    let 文本 = 绝对.to_string_lossy().replace('\\', "/");
    let 文本 = 文本.trim_end_matches('/').to_string();
    if cfg!(windows) {
        文本.to_lowercase()
    } else {
        文本
    }
}

/// `子` 是否就是 `父`，或位于 `父` 之下。
fn 相同或子路径(父: &str, 子: &str) -> bool {
    子 == 父 || 子.starts_with(&format!("{父}/"))
}

/// 生成包内说明文件（文件清单，便于用户与 CI 排查"包里到底有什么"）。
///
/// `附加` 是复制阶段之外生成的文件（目前只有游戏清单），它们同样要
/// 出现在清单里，否则用户会以为"包里没有 game_manifest.ron"。
fn 生成清单文本(
    计划: &打包计划, 报告: &打包报告, 附加: &[(String, u64)]
) -> String {
    let 附加字节: u64 = 附加.iter().map(|(_, 字节)| 字节).sum();
    let mut 文本 = String::new();
    文本.push_str("# 微风（Breeze）打包清单\n");
    文本.push_str(&format!(
        "# 共 {} 个文件，{}\n",
        计划.条目.len() + 附加.len(),
        人类可读字节(计划.总字节() + 附加字节)
    ));
    文本.push_str("# 目标                大小\n");
    for 条目 in &计划.条目 {
        文本.push_str(&format!("{:<28} {}\n", 条目.目标, 人类可读字节(条目.大小)));
    }
    for (名称, 字节) in 附加 {
        文本.push_str(&format!("{名称:<28} {}\n", 人类可读字节(*字节)));
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

    /// 把工程声明成"带用户代码"：有运行目标。
    fn 声明运行目标(项目: &mut 项目描述) {
        let 运行 = crate::project::运行目标清单 {
            程序: "mini_game.exe".into(),
            ..Default::default()
        };
        项目.清单可变().运行 = Some(运行);
        项目.保存清单().expect("保存清单");
    }

    fn 计划目标(计划: &打包计划) -> Vec<String> {
        计划.条目.iter().map(|条目| 条目.目标.clone()).collect()
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
    fn 带运行目标的工程把代码加载的资源也打进包() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/hero.png"]);
        写文件(&项目, "assets/hero.png", b"hero");
        写文件(&项目, "assets/models/cube.glb", b"glb-bytes");
        写文件(&项目, "assets/raw_src/source.psd", b"psd-bytes");
        项目.清单可变().启动场景 = Some(主场景);
        声明运行目标(&mut 项目);

        let 配置 = 打包配置 {
            排除资源: vec!["raw_src/".into()],
            ..打包配置::default()
        };
        let 计划 = 生成打包计划(&项目, &配置, None);

        assert!(计划.可以打包(), "阻塞项：{:?}", 计划.缺失依赖);
        let 目标 = 计划目标(&计划);
        assert!(
            目标.contains(&"assets/models/cube.glb".to_string()),
            "代码加载、场景没登记的资源必须进包，实际：{目标:?}"
        );
        assert!(
            !目标.contains(&"assets/raw_src/source.psd".to_string()),
            "排除项必须生效，实际：{目标:?}"
        );
        let 立方体 = 计划
            .条目
            .iter()
            .find(|条目| 条目.目标 == "assets/models/cube.glb")
            .expect("立方体在计划里");
        assert!(!立方体.被场景引用, "场景没引用它，摘要要能区分出来");
        let 英雄 = 计划
            .条目
            .iter()
            .find(|条目| 条目.目标 == "assets/hero.png")
            .expect("英雄贴图在计划里");
        assert!(英雄.被场景引用);
    }

    #[test]
    fn 排除项覆盖场景引用的资源时阻止打包并点名路径() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/raw_src/used.png"]);
        写文件(&项目, "assets/raw_src/used.png", b"png");
        项目.清单可变().启动场景 = Some(主场景);
        声明运行目标(&mut 项目);

        let 配置 = 打包配置 {
            排除资源: vec!["raw_src".into()],
            ..打包配置::default()
        };
        let 计划 = 生成打包计划(&项目, &配置, None);

        assert!(!计划.可以打包(), "排除项盖住场景资源时必须阻止打包");
        let 文本 = 计划.缺失依赖.join("\n");
        assert!(
            文本.contains("assets/raw_src/used.png"),
            "必须点名被覆盖的路径：{文本}"
        );
        assert!(文本.contains("`raw_src`"), "必须点名是哪条排除项：{文本}");
    }

    #[test]
    fn 带运行目标但没有资源目录的工程照常打包() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &[]);
        项目.清单可变().启动场景 = Some(主场景);
        声明运行目标(&mut 项目);
        std::fs::remove_dir_all(项目.根目录().join(crate::project::目录名::资源))
            .expect("删资源目录");

        let 计划 = 生成打包计划(&项目, &打包配置::default(), None);

        assert!(计划.可以打包(), "阻塞项：{:?}", 计划.缺失依赖);
        let 目标 = 计划目标(&计划);
        assert!(目标.contains(&"scenes/主场景.scene.ron".to_string()));
        assert!(目标.contains(&"project.ron".to_string()));
    }

    #[test]
    fn 排除项按目录边界匹配而不是子串() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &[]);
        写文件(&项目, "assets/tex/a.png", b"a");
        写文件(&项目, "assets/textures/b.png", b"b");
        写文件(&项目, "assets/tex2/c.png", b"c");
        项目.清单可变().启动场景 = Some(主场景);
        声明运行目标(&mut 项目);

        let 排除后的目标 = |前缀: &str| {
            let 配置 = 打包配置 {
                排除资源: vec![前缀.to_string()],
                ..打包配置::default()
            };
            计划目标(&生成打包计划(&项目, &配置, None))
        };

        let 目标 = 排除后的目标("tex");
        assert!(!目标.contains(&"assets/tex/a.png".to_string()));
        assert!(
            目标.contains(&"assets/textures/b.png".to_string()),
            "`tex` 不能误伤 `textures`：{目标:?}"
        );
        assert!(
            目标.contains(&"assets/tex2/c.png".to_string()),
            "`tex` 不能误伤 `tex2`：{目标:?}"
        );

        let 目标 = 排除后的目标("./tex/");
        assert!(
            !目标.contains(&"assets/tex/a.png".to_string()),
            "规范化要吃掉 `./` 与结尾 `/`：{目标:?}"
        );
    }

    #[test]
    fn 纯微风工程的排除项不生效也不报冲突() {
        let (_目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/raw_src/used.png"]);
        写文件(&项目, "assets/raw_src/used.png", b"png");
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");

        let 配置 = 打包配置 {
            排除资源: vec!["raw_src".into()],
            ..打包配置::default()
        };
        let 计划 = 生成打包计划(&项目, &配置, None);

        assert!(
            计划.可以打包(),
            "场景闭包模式下排除项不参与，不应报冲突：{:?}",
            计划.缺失依赖
        );
        assert!(
            计划目标(&计划).contains(&"assets/raw_src/used.png".to_string()),
            "闭包模式照常带上场景引用的资源"
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
            "game_manifest.ron",
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
        assert!(
            清单.contains("game_manifest.ron"),
            "说明文件必须列出游戏清单"
        );
        let 游戏清单 = 游戏清单::读取(&输出).expect("读取游戏清单");
        assert_eq!(游戏清单.主场景, "scenes/主场景.scene.ron");
    }

    #[test]
    fn 没有主场景的项目不写游戏清单() {
        let (_目录, 项目) = 建项目();
        let 配置 = 打包配置::default();
        let 计划 = 生成打包计划(&项目, &配置, None);
        let 报告 = 执行打包(&项目, &配置, &计划);
        assert!(报告.成功, "失败项：{:?}", 报告.失败项);
        let 输出 = 项目.根目录().join(默认输出目录);
        assert!(
            !输出.join(游戏清单文件名).exists(),
            "没有场景就没有可玩的入口，不应写出空入口的清单"
        );
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

    /// 建一个"能导出"的工程：两个场景、一个资源、一个产物与调试符号。
    fn 准备可导出工程() -> (tempfile::TempDir, 项目描述, PathBuf) {
        let (目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/hero.png"]);
        写场景(&项目, "第二场景", &[]);
        写文件(&项目, "assets/hero.png", b"hero");
        let 产物 = 写文件(&项目, "target/debug/mini_game.exe", b"EXE");
        std::fs::write(产物.with_extension("pdb"), b"PDB").expect("写符号");
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");
        (目录, 项目, 产物)
    }

    #[test]
    fn 导出可玩包带上全部场景与调试符号并覆盖全屏() {
        let (目录, 项目, 产物) = 准备可导出工程();
        let 输出 = 目录.path().join("游戏包");

        let 报告 = 导出可玩包(项目.根目录(), &输出, &产物, false).expect("导出");

        assert!(报告.成功, "失败项：{:?}", 报告.失败项);
        for 相对 in [
            "mini_game.exe",
            "mini_game.pdb",
            "project.ron",
            "game_manifest.ron",
            "build-info.txt",
            "scenes/主场景.scene.ron",
            "scenes/第二场景.scene.ron",
            "assets/hero.png",
        ] {
            assert!(
                输出
                    .join(相对.replace('/', std::path::MAIN_SEPARATOR_STR))
                    .is_file(),
                "包里缺少 {相对}"
            );
        }
        let 清单 = 游戏清单::读取(&输出).expect("读取游戏清单");
        assert!(!清单.全屏, "`--windowed` 必须覆盖清单里的全屏默认值");
    }

    #[test]
    fn 导出可玩包拒绝工程目录与它的上级作为输出() {
        let (目录, 项目, 产物) = 准备可导出工程();
        let 错误 = 导出可玩包(项目.根目录(), 项目.根目录(), &产物, true)
            .expect_err("把工程本身当输出必须拒绝");
        assert!(错误.contains("工程目录"), "错误原文：{错误}");

        let 错误 = 导出可玩包(项目.根目录(), 目录.path(), &产物, true)
            .expect_err("清空上级目录会删掉整个工程，必须拒绝");
        assert!(错误.contains("工程目录"), "错误原文：{错误}");
    }

    #[test]
    fn 导出可玩包拒绝产物位于输出目录内() {
        let (_目录, 项目, 产物) = 准备可导出工程();
        let 输出 = 产物.parent().expect("产物父目录").to_path_buf();
        let 错误 =
            导出可玩包(项目.根目录(), &输出, &产物, true).expect_err("清空输出目录会先删掉产物");
        assert!(错误.contains("可执行文件在输出目录里"), "错误原文：{错误}");
        assert!(产物.is_file(), "被拒绝的导出不能动输出目录");
    }

    #[test]
    fn 导出可玩包列出缺失依赖而不是打出半个包() {
        let (目录, mut 项目) = 建项目();
        let 主场景 = 写场景(&项目, "主场景", &["assets/丢失.png"]);
        项目.清单可变().启动场景 = Some(主场景);
        项目.保存清单().expect("保存清单");
        let 产物 = 写文件(&项目, "target/debug/mini_game.exe", b"EXE");
        let 输出 = 目录.path().join("游戏包");

        let 错误 = 导出可玩包(项目.根目录(), &输出, &产物, true).expect_err("缺失依赖必须拒绝");
        assert!(错误.contains("缺失依赖"), "错误原文：{错误}");
        assert!(错误.contains("丢失.png"), "错误原文：{错误}");
        assert!(!输出.exists(), "拒绝导出时不应创建输出目录");
    }

    #[test]
    fn 导出可玩包在工程没有场景时报错() {
        let (目录, 项目) = 建项目();
        let 产物 = 写文件(&项目, "target/debug/mini_game.exe", b"EXE");
        let 输出 = 目录.path().join("游戏包");
        let 错误 = 导出可玩包(项目.根目录(), &输出, &产物, true).expect_err("没有入口不能导出");
        assert!(错误.contains("没有场景"), "错误原文：{错误}");
    }
}
