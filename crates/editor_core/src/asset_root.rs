// SPDX-License-Identifier: MIT OR Apache-2.0
//! 资源根解析：发布版（绿色版）如何找到可执行文件同级的 `assets/`。
//!
//! # 为什么需要这一层
//!
//! Bevy 的 `AssetPlugin` 把 `file_path`（默认 `assets`）拼在"基准目录"上：
//! 先看 `BEVY_ASSET_ROOT`，再看编译期的 `CARGO_MANIFEST_DIR`，最后才是
//! **可执行文件所在目录**。开发期这套规则足够用；发行版却不一样：
//! 从快捷方式、任务计划器或别的宿主启动时，**当前工作目录**可能与 exe
//! 不在同一处，用户看到的现象是"模型和贴图全部加载失败"。
//!
//! 本模块把候选顺序显式化，并补上"当前目录找不到就回退到 exe 同级"：
//!
//! | 优先级 | 候选 | 来源 |
//! | --- | --- | --- |
//! | 1 | `$BEVY_ASSET_ROOT/assets`（变量直接指向 assets 目录时也接受） | 环境变量 |
//! | 2 | `<当前工作目录>/assets` | 当前目录 |
//! | 3 | `<exe 所在目录>/assets` | 可执行文件同级 |
//! | 4 | 第 3 档的路径，但目录尚不存在 | 回退可执行文件同级 |
//!
//! 第 4 档不是猜：它正是打包目录的固定布局（`dist/bevy-studio.exe` 与
//! `dist/assets/` 并排）。即使资源还没拷进去，`AssetPlugin` 也必须拿到
//! 一个**确定**的路径，而不是随启动方式漂移。
//!
//! # 为什么不读 `CARGO_MANIFEST_DIR`
//!
//! 那是编译期常量，只有 `cargo run` 的进程环境里才恰好存在；绿色版没有它。
//! 把它列进候选会让"同一个 exe 换个启动方式就换资源目录"的问题继续存在，
//! 因此这里只看运行期真实可得的信息：环境变量、当前目录、exe 位置。
//!
//! 纯 `std` 实现、不依赖 Bevy，因此可以在没有窗口与 GPU 的测试里覆盖
//! 每一条优先级。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// 资源根覆盖环境变量（与 Bevy 自身读取的是同一个名字）。
pub const 资源根环境变量: &str = "BEVY_ASSET_ROOT";

/// 资源目录名。
pub const 资源目录名: &str = "assets";

/// 候选资源目录是怎么选出来的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 资源根来源 {
    /// `BEVY_ASSET_ROOT` 环境变量显式指定。
    环境变量,
    /// 当前工作目录下的 `assets/`。
    当前目录,
    /// 可执行文件所在目录下的 `assets/`。
    可执行文件同级,
    /// 以上都不存在时的确定性回退（仍是可执行文件同级）。
    回退可执行文件同级,
}

impl 资源根来源 {
    /// 界面、日志与测试断言共用的一句话说明。
    pub fn 说明(self) -> &'static str {
        match self {
            资源根来源::环境变量 => "环境变量 BEVY_ASSET_ROOT",
            资源根来源::当前目录 => "当前工作目录",
            资源根来源::可执行文件同级 => "可执行文件同级",
            资源根来源::回退可执行文件同级 => "回退到可执行文件同级",
        }
    }

    /// 该来源是否来自用户的显式选择（环境变量）。
    pub fn 是显式指定(self) -> bool {
        matches!(self, 资源根来源::环境变量)
    }
}

/// 一次资源根解析的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 资源根解析 {
    /// 资源目录路径（绝对路径，或在拿不到 exe 位置时的相对路径）。
    pub 路径: PathBuf,
    /// 这个路径是按哪条优先级选出来的。
    pub 来源: 资源根来源,
    /// 解析此刻该目录是否真的存在。
    ///
    /// 为 `false` 只代表"资源还没放进来"，不代表解析失败：
    /// 打包目录的固定布局仍然应当被交给资产服务器。
    pub 存在: bool,
}

/// 可执行文件所在目录；拿不到时退回当前目录。
///
/// # 为什么需要这个函数
///
/// `Path::new("bevy-studio").parent()` 返回的是 `Some("")` 而不是 `None`，
/// 直接 `join("assets")` 会得到相对路径 `assets`。测试与嵌入式调用都可能
/// 传入裸文件名，因此这里把"没有有效父目录"统一收敛到当前目录。
pub fn 可执行目录(可执行文件: &Path, 当前目录: &Path) -> PathBuf {
    match 可执行文件.parent() {
        Some(父) if !父.as_os_str().is_empty() => 父.to_path_buf(),
        _ => 当前目录.to_path_buf(),
    }
}

/// 列出**真实存在**的候选资源目录，按优先级排序并去重。
///
/// # 环境变量的两种写法
///
/// `BEVY_ASSET_ROOT` 在 Bevy 里表示"基准目录"，因此优先取
/// `$BEVY_ASSET_ROOT/assets`；如果该子目录不存在但变量本身指向一个目录，
/// 就把它当作资源目录（用户直接指向 `.../assets` 的写法同样有效）。
///
/// 环境变量指向不存在的路径时**静默跳过**：它是覆盖项，不是唯一来源，
/// 项目被移动到别的机器后仍应能靠 exe 同级布局启动。
pub fn 候选资源根(
    环境值: Option<&OsStr>,
    当前目录: &Path,
    可执行文件: &Path,
) -> Vec<(PathBuf, 资源根来源)> {
    let mut 候选: Vec<(PathBuf, 资源根来源)> = Vec::new();

    if let Some(值) = 环境值.filter(|值| !值.is_empty()) {
        let 基准 = PathBuf::from(值);
        let 带子目录 = 基准.join(资源目录名);
        let 环境目录 = if 带子目录.is_dir() {
            带子目录
        } else {
            基准
        };
        if 环境目录.is_dir() {
            候选.push((环境目录, 资源根来源::环境变量));
        }
    }

    let 当前 = 当前目录.join(资源目录名);
    if 当前.is_dir() && !候选.iter().any(|(路径, _)| 路径 == &当前) {
        候选.push((当前, 资源根来源::当前目录));
    }

    let 同级 = 可执行目录(可执行文件, 当前目录).join(资源目录名);
    if 同级.is_dir() && !候选.iter().any(|(路径, _)| 路径 == &同级) {
        候选.push((同级, 资源根来源::可执行文件同级));
    }

    候选
}

/// 从排好序的候选里选第一个；空列表返回 `None`。
pub fn 选定资源根(候选: &[(PathBuf, 资源根来源)]) -> Option<资源根解析> {
    候选.first().map(|(路径, 来源)| 资源根解析 {
        路径: 路径.clone(),
        来源: *来源,
        存在: true,
    })
}

/// 可注入参数的解析入口（测试与嵌入式宿主用）。
///
/// # 返回值
///
/// 1. 有存在的候选 → 选优先级最高的那个，`存在` 为 `true`；
/// 2. 一个都不存在 → 返回 exe 同级的 `assets/`，`存在` 为 `false`，
///    来源标记为回退。调用方据此决定是否提示"资源目录缺失"。
pub fn 解析资源根带参数(
    环境值: Option<&OsStr>,
    当前目录: &Path,
    可执行文件: &Path,
) -> 资源根解析 {
    let 候选 = 候选资源根(环境值, 当前目录, 可执行文件);
    if let Some(命中) = 选定资源根(&候选) {
        return 命中;
    }
    资源根解析 {
        路径: 可执行目录(可执行文件, 当前目录).join(资源目录名),
        来源: 资源根来源::回退可执行文件同级,
        存在: false,
    }
}

/// 读取真实运行环境并解析资源根。
///
/// 这是编辑器入口使用的正式入口；`current_dir` / `current_exe` 失败时
/// 用空目录兜底，保证启动流程不会因为路径查询失败而中断。
pub fn 解析资源根() -> 资源根解析 {
    let 环境值 = std::env::var_os(资源根环境变量);
    let 当前目录 = std::env::current_dir().unwrap_or_default();
    let 可执行文件 = std::env::current_exe().unwrap_or_else(|_| 当前目录.join("bevy-studio"));
    解析资源根带参数(环境值.as_deref(), &当前目录, &可执行文件)
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use std::fs;

    fn 建目录(根: &Path, 相对: &str) -> PathBuf {
        let 路径 = 根.join(相对);
        fs::create_dir_all(&路径).expect("建目录");
        路径
    }

    #[test]
    fn 环境变量优先于当前目录与可执行文件() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 根 = 临时.path();
        let 当前目录 = 建目录(根, "当前");
        let 执行目录 = 建目录(根, "安装");
        let 环境目录 = 建目录(根, "自定义");
        建目录(&当前目录, 资源目录名);
        建目录(&执行目录, 资源目录名);
        let 环境资源 = 建目录(&环境目录, 资源目录名);

        let 结果 = 解析资源根带参数(
            Some(环境目录.as_os_str()),
            &当前目录,
            &执行目录.join("bevy-studio.exe"),
        );
        assert_eq!(结果.路径, 环境资源);
        assert_eq!(结果.来源, 资源根来源::环境变量);
        assert!(结果.存在);
        assert!(结果.来源.是显式指定());
    }

    #[test]
    fn 环境变量可以直接指向资源目录本身() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 根 = 临时.path();
        let 当前目录 = 建目录(根, "当前");
        let 执行目录 = 建目录(根, "安装");
        let 资源本身 = 建目录(根, "assets");

        let 结果 = 解析资源根带参数(
            Some(资源本身.as_os_str()),
            &当前目录,
            &执行目录.join("bevy-studio.exe"),
        );
        assert_eq!(
            结果.路径, 资源本身,
            "变量直接指向 assets 时不应再拼一层 assets"
        );
        assert_eq!(结果.来源, 资源根来源::环境变量);
    }

    #[test]
    fn 无效环境变量被跳过并继续回退() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 根 = 临时.path();
        let 当前目录 = 建目录(根, "当前");
        let 执行目录 = 建目录(根, "安装");
        let 当前资源 = 建目录(&当前目录, 资源目录名);

        let 结果 = 解析资源根带参数(
            Some(OsStr::new("绝对不存在的资源目录")),
            &当前目录,
            &执行目录.join("bevy-studio.exe"),
        );
        assert_eq!(结果.路径, 当前资源);
        assert_eq!(结果.来源, 资源根来源::当前目录);
    }

    #[test]
    fn 当前目录优先于可执行文件同级() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 根 = 临时.path();
        let 当前目录 = 建目录(根, "当前");
        let 执行目录 = 建目录(根, "安装");
        let 当前资源 = 建目录(&当前目录, 资源目录名);
        建目录(&执行目录, 资源目录名);

        let 结果 = 解析资源根带参数(None, &当前目录, &执行目录.join("bevy-studio.exe"));
        assert_eq!(结果.路径, 当前资源);
        assert_eq!(结果.来源, 资源根来源::当前目录);
    }

    #[test]
    fn 当前目录没有时回退到可执行文件同级() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 根 = 临时.path();
        let 当前目录 = 建目录(根, "别的启动目录");
        let 执行目录 = 建目录(根, "安装");
        let 安装资源 = 建目录(&执行目录, 资源目录名);

        let 结果 = 解析资源根带参数(None, &当前目录, &执行目录.join("bevy-studio.exe"));
        assert_eq!(结果.路径, 安装资源);
        assert_eq!(结果.来源, 资源根来源::可执行文件同级);
        assert!(结果.存在);
    }

    #[test]
    fn 都不存在时给出确定的回退路径() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 根 = 临时.path();
        let 当前目录 = 建目录(根, "当前");
        let 执行目录 = 建目录(根, "安装");

        let 结果 = 解析资源根带参数(None, &当前目录, &执行目录.join("bevy-studio.exe"));
        assert_eq!(结果.路径, 执行目录.join(资源目录名));
        assert_eq!(结果.来源, 资源根来源::回退可执行文件同级);
        assert!(
            !结果.存在,
            "目录确实不存在时必须如实标记，调用方才能提示用户"
        );
    }

    #[test]
    fn 裸文件名执行路径退回当前目录() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 当前目录 = 临时.path().join("当前");
        建目录(&当前目录, "占位");

        let 结果 = 解析资源根带参数(None, &当前目录, Path::new("bevy-studio"));
        assert_eq!(结果.路径, 当前目录.join(资源目录名));
        assert_eq!(结果.来源, 资源根来源::回退可执行文件同级);
    }

    #[test]
    fn 同一目录只出现一次() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 根 = 临时.path();
        let 共用 = 建目录(根, "共用");
        建目录(&共用, 资源目录名);

        let 候选 = 候选资源根(None, &共用, &共用.join("bevy-studio.exe"));
        assert_eq!(候选.len(), 1, "当前目录与 exe 同级重合时不应重复");
        assert_eq!(候选[0].1, 资源根来源::当前目录);
    }

    #[test]
    fn 来源说明覆盖全部取值() {
        for 来源 in [
            资源根来源::环境变量,
            资源根来源::当前目录,
            资源根来源::可执行文件同级,
            资源根来源::回退可执行文件同级,
        ] {
            assert!(!来源.说明().is_empty());
        }
        assert!(资源根来源::环境变量.是显式指定());
        assert!(!资源根来源::当前目录.是显式指定());
    }
}
