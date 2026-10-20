//! 用 `cargo metadata` 识别原生 Cargo 工程，并缓存结果。
//!
//! # 为什么在静态解析之外再加这一层
//!
//! [`crate::cargo_project`] 里的静态解析只读几个 `Cargo.toml`，成本可以忽略，
//! 但它终究是"猜 Cargo 的规则"：`[workspace] default-members`、`exclude`、
//! 路径依赖、`[[bin]] path` 的各种写法、`autobins` 与 `src/bin/*` 的交互……
//! 每一条都要自己实现一遍，而用户工程里出现例外时，界面给出的运行入口就是错的。
//!
//! `cargo metadata` 是 Cargo **自己**回答这个问题的方式：
//! package、target 名、target 种类（`bin` / `lib` / `example` / `test` …）、
//! manifest 路径、工作区根、以及每个包依赖了谁，全都是结构化字段。
//!
//! 因此这里的顺序是**元数据优先、静态解析兜底**：
//!
//! | 情况 | 行为 |
//! | --- | --- |
//! | `cargo` 可用且清单合法 | 用元数据结果（权威） |
//! | `cargo` 不可用 / 依赖下载失败 / 清单损坏 | 回退静态解析，并把**真实失败原因**留在结论里 |
//! | 两者都失败 | 明确报错，不假装识别成功 |
//!
//! # 缓存
//!
//! 打开项目是第一交互路径，不能每次都起一个 `cargo` 子进程。
//! 缓存按**根目录 + 指纹**索引：指纹是一组 `Cargo.toml` / `Cargo.lock` 的
//! `(路径, mtime, 长度)`。任何一处清单变化（或用户 `cargo add` 之后）
//! 指纹就变，下一次识别自然重新采集 —— 不需要文件监听器，也不会读到陈旧结果。
//!
//! 缓存是**进程级**的（编辑器是单进程应用），对外提供
//! [`清除工程缓存`] 以便测试与"手动刷新"使用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use crate::cargo_project::{
    Cargo包, Cargo可执行目标, Cargo工程, Cargo结果, Cargo识别错误, 原生清单文件名,
};

/// 指纹里最多收集多少个清单文件。
///
/// 一个正常的多 crate 工程远小于这个数；设上限是为了避免有人把仓库根
/// 指向一个包含大量嵌套工程的巨型目录时，指纹计算本身变成瓶颈。
const 指纹文件上限: usize = 512;

/// `cargo metadata` 失败的原因。
///
/// 分类不是为了好看：这四类的**处置方式完全不同**。
/// 工具链缺失要去装 Rust；依赖下载失败要联网或换 registry；
/// 清单错误要改用户代码（而微风不会替用户改）；其余才可能是编辑器的问题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cargo元数据错误 {
    /// `cargo` 本身找不到或无法执行。
    工具链缺失(String),
    /// 依赖解析/下载失败（离线、registry、证书、网络）。
    依赖下载失败(String),
    /// `Cargo.toml` 语法或语义错误。
    清单错误(String),
    /// 其他失败（含 JSON 解析失败）。
    其他(String),
}

impl Cargo元数据错误 {
    /// 稳定代号（进日志与证据 JSON，不随语言变化）。
    pub fn 代号(&self) -> &'static str {
        match self {
            Self::工具链缺失(_) => "toolchain-missing",
            Self::依赖下载失败(_) => "dependency-download",
            Self::清单错误(_) => "manifest-error",
            Self::其他(_) => "metadata-other",
        }
    }

    /// 词条键（界面显示用）。
    pub fn 词条键(&self) -> &'static str {
        match self {
            Self::工具链缺失(_) => "cargo_meta_toolchain",
            Self::依赖下载失败(_) => "cargo_meta_download",
            Self::清单错误(_) => "cargo_meta_manifest",
            Self::其他(_) => "cargo_meta_other",
        }
    }

    /// 真实错误文本（绝不裁剪成一句空话）。
    pub fn 文本(&self) -> &str {
        match self {
            Self::工具链缺失(文本)
            | Self::依赖下载失败(文本)
            | Self::清单错误(文本)
            | Self::其他(文本) => 文本,
        }
    }

    /// 一句话说明（给界面与控制台）。
    pub fn 说明(&self) -> String {
        format!("{}：{}", self.代号(), 截断(self.文本(), 400))
    }
}

/// 工程识别走了哪条路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 识别来源 {
    /// `cargo metadata`（权威）。
    Cargo元数据,
    /// 静态解析 `Cargo.toml`（兜底）。
    静态解析,
}

impl 识别来源 {
    /// 稳定代号。
    pub fn 代号(self) -> &'static str {
        match self {
            Self::Cargo元数据 => "cargo-metadata",
            Self::静态解析 => "static-parse",
        }
    }

    /// 词条键。
    pub fn 词条键(self) -> &'static str {
        match self {
            Self::Cargo元数据 => "cargo_source_metadata",
            Self::静态解析 => "cargo_source_static",
        }
    }
}

/// 一次工程识别的结论。
#[derive(Debug, Clone)]
pub struct Cargo识别结论 {
    /// 识别出的工程。
    pub 工程: Cargo工程,
    /// 走了哪条路径。
    pub 来源: 识别来源,
    /// 元数据失败时的真实原因（`来源 == 静态解析` 时才有值）。
    ///
    /// **必须显示给用户**：静态解析可能把目标选得不够好，
    /// 用户看到"为什么退化了"才知道该去修工具链还是修清单。
    pub 回退原因: Option<Cargo元数据错误>,
    /// 是否直接命中缓存（没有起子进程）。
    pub 命中缓存: bool,
}

/// 一个缓存条目。
#[derive(Debug, Clone)]
struct 缓存条目 {
    指纹: Vec<(PathBuf, u64, u64)>,
    结论: Cargo识别结论,
}

/// 工程识别缓存。
///
/// 与 `HashMap` 相比用 `BTreeMap` 只是为了输出稳定（测试与日志里好比对）。
#[derive(Debug, Default)]
pub struct Cargo识别缓存 {
    条目: BTreeMap<PathBuf, 缓存条目>,
}

impl Cargo识别缓存 {
    /// 缓存里有多少个根目录。
    pub fn 长度(&self) -> usize {
        self.条目.len()
    }

    /// 清空。
    pub fn 清空(&mut self) {
        self.条目.clear();
    }

    /// 让某个根目录的缓存失效（用户点"刷新工程"时用）。
    pub fn 失效(&mut self, 根: &Path) {
        self.条目.remove(&规范化根(根));
    }

    /// 识别一个工程（带缓存）。
    ///
    /// 指纹一致时直接返回上次结论并标记 `命中缓存`。
    ///
    /// # 注意：缓存键规范化，**真实路径不规范化**
    ///
    /// 缓存键会统一分隔符并小写（Windows 路径不区分大小写），
    /// 但传给识别逻辑的必须是调用方给的原始路径：把它替换成小写路径
    /// 会让 `工程.根目录` 与项目根目录在大小写上不一致，
    /// 于是 `strip_prefix` 全部失败、源码列表变空、诊断跳转找不到文件。
    pub fn 识别(&mut self, 根: &Path) -> Cargo结果<Cargo识别结论> {
        let 键 = 规范化根(根);
        let 指纹 = 计算指纹(根);
        if let Some(条目) = self.条目.get(&键)
            && 条目.指纹 == 指纹
        {
            let mut 结论 = 条目.结论.clone();
            结论.命中缓存 = true;
            return Ok(结论);
        }
        let 结论 = 采集识别(根)?;
        self.条目.insert(
            键,
            缓存条目 {
                指纹,
                结论: 结论.clone(),
            },
        );
        Ok(结论)
    }
}

/// 进程级缓存。
fn 全局缓存() -> &'static Mutex<Cargo识别缓存> {
    static 缓存: OnceLock<Mutex<Cargo识别缓存>> = OnceLock::new();
    缓存.get_or_init(|| Mutex::new(Cargo识别缓存::default()))
}

/// 清除进程级工程识别缓存。
///
/// 场景：用户刚在外部改了 `Cargo.toml`（例如 `cargo add bevy`），
/// 但文件 mtime 精度不足或文件系统缓存导致指纹没变。这时由界面提供
/// 一个显式的"重新识别工程"入口，比赌指纹可靠。
pub fn 清除工程缓存() {
    if let Ok(mut 缓存) = 全局缓存().lock() {
        缓存.清空();
    }
}

/// 识别一个工程（使用进程级缓存）。**元数据优先，静态解析兜底**。
pub fn 识别工程(根: &Path) -> Cargo结果<Cargo识别结论> {
    let mut 缓存 = 全局缓存().lock().map_err(|_| {
        Cargo识别错误::元数据与静态解析都失败(
            根.to_path_buf(),
            "识别缓存锁中毒".into(),
        )
    })?;
    缓存.识别(根)
}

/// 不使用缓存地识别一次（测试与"强制刷新"用）。
pub fn 识别工程不带缓存(根: &Path) -> Cargo结果<Cargo识别结论> {
    采集识别(根)
}

/// 采集一次识别结论：先试元数据，失败则静态解析。
fn 采集识别(根: &Path) -> Cargo结果<Cargo识别结论> {
    match 采集元数据(根).and_then(|元数据| 转换为工程(根, 元数据)) {
        Ok(工程) => Ok(Cargo识别结论 {
            工程,
            来源: 识别来源::Cargo元数据,
            回退原因: None,
            命中缓存: false,
        }),
        Err(元数据错误) => match Cargo工程::读取(根) {
            Ok(工程) => Ok(Cargo识别结论 {
                工程,
                来源: 识别来源::静态解析,
                回退原因: Some(元数据错误),
                命中缓存: false,
            }),
            Err(静态错误) => Err(Cargo识别错误::元数据与静态解析都失败(
                根.to_path_buf(),
                format!(
                    "cargo metadata 失败（{}）；静态解析失败（{}）",
                    元数据错误.说明(),
                    静态错误
                ),
            )),
        },
    }
}

/// 解析后的 `cargo metadata` 最小子集。
#[derive(Debug, Clone)]
struct 元数据 {
    工作区根: PathBuf,
    包: Vec<元数据包>,
}

#[derive(Debug, Clone)]
struct 元数据包 {
    名称: String,
    版本: String,
    清单路径: PathBuf,
    依赖: Vec<String>,
    目标: Vec<元数据目标>,
}

#[derive(Debug, Clone)]
struct 元数据目标 {
    名称: String,
    种类: Vec<String>,
    源文件: PathBuf,
}

/// 跑一次 `cargo metadata --no-deps --format-version 1`。
///
/// `--no-deps` 是刻意的：编辑器只关心**本地工作区**的包与目标，
/// 加上依赖会有几千个第三方包，既慢又毫无用处。
fn 采集元数据(根: &Path) -> Result<元数据, Cargo元数据错误> {
    let 清单 = 根.join(原生清单文件名);
    if !清单.is_file() {
        return Err(Cargo元数据错误::清单错误(format!(
            "`{}` 下没有 Cargo.toml",
            根.display()
        )));
    }
    let 输出 = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&清单)
        .current_dir(根)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // 关掉颜色与进度：我们要的是 JSON，不是给人看的渲染结果。
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_TERM_PROGRESS_WHEN", "never")
        .output();

    let 输出 = match 输出 {
        Ok(输出) => 输出,
        Err(错误) => {
            return Err(Cargo元数据错误::工具链缺失(format!(
                "无法执行 `cargo`：{错误}。请确认 Rust 工具链已安装并在 PATH 上"
            )));
        }
    };
    if !输出.status.success() {
        let 标准错误 = String::from_utf8_lossy(&输出.stderr).to_string();
        return Err(分类元数据失败(&标准错误));
    }
    let 值: serde_json::Value = serde_json::from_slice(&输出.stdout).map_err(|错误| {
        Cargo元数据错误::其他(format!(
            "cargo metadata 输出的 JSON 无法解析：{错误}（前 200 字节：{}）",
            截断(&String::from_utf8_lossy(&输出.stdout), 200)
        ))
    })?;
    let 工作区根 = 值
        .get("workspace_root")
        .and_then(|值| 值.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| Cargo元数据错误::其他("cargo metadata 缺少 workspace_root".into()))?;

    let 包列表 = 值
        .get("packages")
        .and_then(|值| 值.as_array())
        .ok_or_else(|| Cargo元数据错误::其他("cargo metadata 缺少 packages".into()))?;

    let mut 包 = Vec::new();
    for 项 in 包列表 {
        let Some(名称) = 项.get("name").and_then(|值| 值.as_str()) else {
            continue;
        };
        let 清单路径 = 项
            .get("manifest_path")
            .and_then(|值| 值.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| 根.join(原生清单文件名));
        let 依赖 = 项
            .get("dependencies")
            .and_then(|值| 值.as_array())
            .map(|数组| {
                数组
                    .iter()
                    .filter_map(|依赖| 依赖.get("name").and_then(|值| 值.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let 目标 = 项
            .get("targets")
            .and_then(|值| 值.as_array())
            .map(|数组| {
                数组
                    .iter()
                    .filter_map(|目标| {
                        let 名称 = 目标.get("name").and_then(|值| 值.as_str())?.to_string();
                        let 种类 = 目标
                            .get("kind")
                            .and_then(|值| 值.as_array())
                            .map(|数组| {
                                数组
                                    .iter()
                                    .filter_map(|值| 值.as_str())
                                    .map(str::to_string)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        let 源文件 = 目标
                            .get("src_path")
                            .and_then(|值| 值.as_str())
                            .map(PathBuf::from)
                            .unwrap_or_default();
                        Some(元数据目标 {
                            名称, 种类, 源文件
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        包.push(元数据包 {
            名称: 名称.to_string(),
            版本: 项
                .get("version")
                .and_then(|值| 值.as_str())
                .unwrap_or("")
                .to_string(),
            清单路径,
            依赖,
            目标,
        });
    }
    if 包.is_empty() {
        return Err(Cargo元数据错误::其他(
            "cargo metadata 没有返回任何本地包".into(),
        ));
    }
    Ok(元数据 { 工作区根, 包 })
}

/// 把 `cargo metadata` 的结果转换成编辑器自己的工程模型。
fn 转换为工程(根: &Path, 元数据: 元数据) -> Result<Cargo工程, Cargo元数据错误> {
    // `是工作区` 仍然读根清单：`cargo metadata` 不直接给出这个布尔量，
    // 而它决定了界面文案（"Cargo 工作区：N 个成员包"）。
    let 是工作区 = 根
        .join(原生清单文件名)
        .is_file()
        .then(|| std::fs::read_to_string(根.join(原生清单文件名)).ok())
        .flatten()
        .and_then(|文本| toml::from_str::<toml::Table>(&文本).ok())
        .map(|表| 表.get("workspace").is_some())
        .unwrap_or(false);

    let mut 包列表: Vec<Cargo包> = 元数据
        .包
        .iter()
        .map(|源| {
            let 包根 = 源
                .清单路径
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| 根.to_path_buf());
            let 清单路径 = if 源.清单路径.is_absolute() {
                源.清单路径.clone()
            } else {
                根.join(&源.清单路径)
            };
            let 包根 = if 包根.is_absolute() {
                包根
            } else {
                根.join(包根)
            };
            let 是工作区根 = 包根 == 元数据.工作区根 || 包根 == 根;
            let 库名 = 源
                .目标
                .iter()
                .find(|目标| 目标.种类.iter().any(|种类| 是库种类(种类)))
                .map(|目标| 目标.名称.clone());
            let 依赖贝维 = 源.依赖.iter().any(|依赖| 依赖 == "bevy");
            let mut 可执行目标: Vec<Cargo可执行目标> = 源
                .目标
                .iter()
                .filter(|目标| 目标.种类.iter().any(|种类| 种类 == "bin"))
                .map(|目标| Cargo可执行目标 {
                    包名: 源.名称.clone(),
                    名称: 目标.名称.clone(),
                    包根: 包根.clone(),
                    源文件: Some(if 目标.源文件.is_absolute() {
                        目标.源文件.clone()
                    } else {
                        包根.join(&目标.源文件)
                    }),
                    种类: 目标.种类.clone(),
                })
                .collect();
            排序可执行目标(&mut 可执行目标, &源.名称);
            Cargo包 {
                名称: 源.名称.clone(),
                版本: 源.版本.clone(),
                根目录: 包根,
                清单路径,
                是工作区根,
                依赖贝维,
                库名,
                可执行目标,
            }
        })
        .collect();

    // 根包在前，其余按名称排序：让"默认包"的选择与显示顺序都稳定，
    // 不随 Cargo 内部的 HashMap 迭代顺序抖动。
    包列表.sort_by(|左, 右| {
        右.是工作区根
            .cmp(&左.是工作区根)
            .then_with(|| 左.名称.cmp(&右.名称))
    });

    Ok(Cargo工程 {
        根目录: 根.to_path_buf(),
        清单路径: 根.join(原生清单文件名),
        是工作区,
        包: 包列表,
        缺失成员: Vec::new(),
    })
}

/// 该 target 种类是不是"库"。
fn 是库种类(种类: &str) -> bool {
    matches!(
        种类,
        "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
    )
}

/// 可执行目标排序：主程序（`src/main.rs` 或与包同名）在前，其余按名字。
pub fn 排序可执行目标(目标: &mut [Cargo可执行目标], 包名: &str) {
    目标.sort_by(|左, 右| {
        let 左主 = 是主程序(左, 包名);
        let 右主 = 是主程序(右, 包名);
        右主.cmp(&左主).then_with(|| 左.名称.cmp(&右.名称))
    });
}

fn 是主程序(目标: &Cargo可执行目标, 包名: &str) -> bool {
    目标
        .源文件
        .as_ref()
        .and_then(|路径| 路径.file_name())
        .map(|名| 名 == "main.rs")
        .unwrap_or(false)
        || 目标.名称 == 包名
}

/// 把 `cargo metadata` 的 stderr 归类。
///
/// 与 `editor_ui::agent_repair::分类错误` 同一套判别顺序（环境 → 清单 → 其他），
/// 但**不能直接复用**：那个函数在 UI 层，而且它的输入是"构建失败的全部诊断"，
/// 这里的输入是 `cargo metadata` 的 stderr。
pub fn 分类元数据失败(标准错误: &str) -> Cargo元数据错误 {
    let 小写 = 标准错误.to_lowercase();
    let 含 = |模式: &str| 小写.contains(模式);
    if 含("failed to download")
        || 含("failed to get")
        || 含("unable to get packages")
        || 含("network failure")
        || 含("registry index")
        || 含("could not connect")
        || 含("connection timed out")
        || 含("operation timed out")
        || 含("no matching package")
        || 含("failed to load source")
        || 含("certificate")
        || 含("offline")
        || 含("blocking waiting for file lock")
    {
        return Cargo元数据错误::依赖下载失败(截断(标准错误, 4000));
    }
    if 含("no such file or directory: `cargo`")
        || 含("cargo: not found")
        || 含("could not execute process")
        || 含("program not found")
        || 含("toolchain") && 含("not installed")
        || 含("is not installed")
    {
        return Cargo元数据错误::工具链缺失(截断(标准错误, 4000));
    }
    if 含("failed to parse manifest")
        || 含("could not find `cargo.toml`")
        || 含("error in")
        || 含("invalid")
        || 含("missing")
        || 含("unused manifest key")
        || 含("virtual manifest")
        || 含("no targets specified")
    {
        return Cargo元数据错误::清单错误(截断(标准错误, 4000));
    }
    Cargo元数据错误::其他(截断(标准错误, 4000))
}

/// 计算根目录的清单指纹。
///
/// 只收集 `Cargo.toml` 与 `Cargo.lock`：这两类文件决定工程结构。
/// 深度限制与跳过目录（`target`/`.git`）让代价保持在一个目录遍历的量级。
pub fn 计算指纹(根: &Path) -> Vec<(PathBuf, u64, u64)> {
    let mut 结果 = Vec::new();
    收集清单(根, 0, &mut 结果);
    // 稳定排序：同一组文件必须得到同一个指纹，否则缓存永远命中不了。
    结果.sort_by(|左, 右| 左.0.cmp(&右.0));
    结果
}

fn 收集清单(目录: &Path, 深度: usize, 结果: &mut Vec<(PathBuf, u64, u64)>) {
    if 结果.len() >= 指纹文件上限 || 深度 > 6 {
        return;
    }
    let Ok(读取) = std::fs::read_dir(目录) else {
        return;
    };
    let mut 子目录 = Vec::new();
    for 项 in 读取.flatten() {
        if 结果.len() >= 指纹文件上限 {
            break;
        }
        let 路径 = 项.path();
        let 名 = 项.file_name().to_string_lossy().to_string();
        if 路径.is_dir() {
            // `target` 是构建产物，`.git` 是版本库，`node_modules` 是前端依赖：
            // 三者都可能包含海量目录，且与 Cargo 工程结构无关。
            if matches!(名.as_str(), "target" | ".git" | "node_modules")
                || (名.starts_with('.') && 深度 == 0 && 名 != ".")
            {
                continue;
            }
            子目录.push(路径);
            continue;
        }
        if (名 == 原生清单文件名 || 名 == "Cargo.lock")
            && let Ok(元数据) = 项.metadata()
        {
            let 修改 = 元数据
                .modified()
                .ok()
                .and_then(|时刻| 时刻.duration_since(UNIX_EPOCH).ok())
                .map(|时长| 时长.as_nanos() as u64)
                .unwrap_or(0);
            结果.push((路径, 修改, 元数据.len()));
        }
    }
    子目录.sort();
    for 子目录 in 子目录 {
        收集清单(&子目录, 深度 + 1, 结果);
    }
}

/// 根目录的规范化键（Windows 不区分大小写，且不允许尾部分隔符造成两次缓存）。
fn 规范化根(根: &Path) -> PathBuf {
    let 文本 = 根.to_string_lossy().replace('\\', "/");
    let 文本 = 文本.trim_end_matches('/').to_string();
    let 文本 = if cfg!(windows) {
        文本.to_ascii_lowercase()
    } else {
        文本
    };
    PathBuf::from(文本)
}

/// 按字符边界截断（避免把多字节字符切断成非法 UTF-8 观感）。
pub fn 截断(文本: &str, 上限: usize) -> String {
    if 文本.chars().count() <= 上限 {
        return 文本.to_string();
    }
    let 截取: String = 文本.chars().take(上限).collect();
    format!("{截取}…")
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
    fn 元数据识别多包工作区与多个二进制目标() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            "[workspace]\nmembers = [\"engine\", \"game\"]\nresolver = \"3\"\n",
        );
        写(
            &根.join("engine/Cargo.toml"),
            "[package]\nname = \"meta_engine\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        写(&根.join("engine/src/lib.rs"), "pub fn tick() {}\n");
        写(
            &根.join("game/Cargo.toml"),
            r#"[package]
name = "meta_game"
version = "0.2.0"
edition = "2021"

[lib]
name = "meta_game_lib"
path = "src/lib.rs"

[[bin]]
name = "server"
path = "src/bin/server.rs"
"#,
        );
        写(&根.join("game/src/lib.rs"), "pub fn shared() {}\n");
        写(&根.join("game/src/main.rs"), "fn main() {}\n");
        写(&根.join("game/src/bin/server.rs"), "fn main() {}\n");

        let 结论 = 识别工程不带缓存(根).expect("应当能识别");
        assert_eq!(结论.来源, 识别来源::Cargo元数据, "有 cargo 时必须走元数据");
        assert!(结论.工程.是工作区);
        assert_eq!(结论.工程.包.len(), 2);
        let 游戏 = 结论
            .工程
            .包
            .iter()
            .find(|包| 包.名称 == "meta_game")
            .expect("应当识别出 game 包");
        // 同时具有 library 与 binary target 的工程：两者都要被识别，
        // 但只有 bin 能作为运行入口。
        assert_eq!(游戏.库名.as_deref(), Some("meta_game_lib"));
        let 目标: Vec<&str> = 游戏
            .可执行目标
            .iter()
            .map(|目标| 目标.名称.as_str())
            .collect();
        assert_eq!(目标, vec!["meta_game", "server"], "主程序必须排在最前");
        assert_eq!(结论.工程.默认可执行目标().unwrap().包名, "meta_game");
    }

    #[test]
    fn 纯库工程没有可运行目标但能识别() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            "[package]\nname = \"meta_lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        写(&根.join("src/lib.rs"), "pub fn x() {}\n");
        let 结论 = 识别工程不带缓存(根).unwrap();
        assert!(!结论.工程.有可运行目标());
        assert_eq!(结论.工程.包[0].库名.as_deref(), Some("meta_lib"));
    }

    #[test]
    fn 损坏的清单同时给出两条失败原因() {
        let 目录 = tempfile::tempdir().unwrap();
        写(&目录.path().join("Cargo.toml"), "[package\nname=\n");
        写(&目录.path().join("src/main.rs"), "fn main() {}");
        match 识别工程不带缓存(目录.path()) {
            Err(Cargo识别错误::元数据与静态解析都失败(_, 说明)) => {
                assert!(说明.contains("cargo metadata"), "{说明}");
                assert!(说明.contains("静态解析"), "{说明}");
            }
            其他 => panic!("应当同时报告两条失败原因，实际：{其他:?}"),
        }
    }

    #[test]
    fn 缓存命中与失效由清单指纹驱动() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(
            &根.join("Cargo.toml"),
            "[package]\nname = \"cache_probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        写(&根.join("src/main.rs"), "fn main() {}\n");

        let mut 缓存 = Cargo识别缓存::default();
        let 第一次 = 缓存.识别(根).unwrap();
        assert!(!第一次.命中缓存);
        let 第二次 = 缓存.识别(根).unwrap();
        assert!(第二次.命中缓存, "清单未变时必须命中缓存");

        // 改清单 → 指纹变化 → 重新采集。
        写(
            &根.join("Cargo.toml"),
            r#"[package]
name = "cache_probe"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "second"
path = "src/second.rs"
"#,
        );
        写(&根.join("src/second.rs"), "fn main() {}\n");
        let 第三次 = 缓存.识别(根).unwrap();
        assert!(!第三次.命中缓存, "清单变化后必须重新采集");
        assert_eq!(
            第三次.工程.全部可执行目标().len(),
            2,
            "改清单后新增的 bin 必须被看到"
        );

        缓存.失效(根);
        assert_eq!(缓存.长度(), 0);
    }

    #[test]
    fn 指纹覆盖成员清单() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        写(&根.join("Cargo.toml"), "[workspace]\nmembers=[\"a\"]\n");
        写(
            &根.join("a/Cargo.toml"),
            "[package]\nname=\"a\"\nversion=\"0.1.0\"\n",
        );
        let 指纹 = 计算指纹(根);
        assert!(
            指纹
                .iter()
                .any(|(路径, _, _)| 路径.ends_with("a/Cargo.toml")
                    || 路径.ends_with("a\\Cargo.toml")),
            "成员清单必须进指纹：{指纹:?}"
        );
    }

    #[test]
    fn stderr分类覆盖真实形态() {
        assert!(matches!(
            分类元数据失败("error: failed to download from `https://index.crates.io`"),
            Cargo元数据错误::依赖下载失败(_)
        ));
        assert!(matches!(
            分类元数据失败("error: failed to parse manifest at `Cargo.toml`"),
            Cargo元数据错误::清单错误(_)
        ));
        assert!(matches!(
            分类元数据失败("thread 'main' panicked"),
            Cargo元数据错误::其他(_)
        ));
    }
}
