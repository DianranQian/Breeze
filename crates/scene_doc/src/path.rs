//! 项目相对路径与资源唯一标识。
//!
//! # 为什么不能直接用 `PathBuf` / `String`
//!
//! 场景文件里的资源引用有三种典型故障，全部源于"路径就是一串自由文本"：
//!
//! 1. **平台差异**：Windows 写 `textures\\hero.png`，Linux 读不到。
//! 2. **绝对路径污染**：把 `C:\Users\某人\...` 存进场景，换台机器立刻失效。
//! 3. **目录穿越**：`../../etc/passwd` 这类路径逃出项目根目录。
//!
//! `项目相对路径` 用不可绕过的构造入口（[`项目相对路径::解析`]）在**边界处**一次性
//! 解决这三点：统一用 `/` 分隔、拒绝绝对路径与前导盘符、拒绝 `..` 与 `.` 段。
//! 内部字段私有，解析成功即代表路径合法，后续代码无需重复校验。
//!
//! [`资源唯一标识`] 是资源引用稳定性的第二道保险：即使文件被改名或移动，
//! 只要内容同一，UUID 不变，编辑器就能重建引用（见 `storage` 与资源重导入逻辑）。

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 路径处理错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum 路径错误 {
    /// 路径为空。
    #[error("路径不能为空")]
    空路径,

    /// 提供了绝对路径（含盘符或前导 `/`）。
    #[error("资源路径必须是项目相对路径，但收到了绝对路径：{0}")]
    绝对路径(String),

    /// 路径中包含 `..` 段，可能逃出项目根目录。
    #[error("资源路径不允许包含 `..` 段：{0}")]
    目录穿越(String),

    /// 路径段为空（例如 `a//b` 或结尾是 `/`）。
    #[error("资源路径包含空路径段：{0}")]
    空路径段(String),
}

/// 规范化的项目相对路径。
///
/// 不变式（由构造函数保证，之后永不被破坏）：
/// * 分隔符统一为 `/`；
/// * 非空、不以 `/` 开头、不含盘符；
/// * 不含 `.` 与 `..` 段。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct 项目相对路径(String);

impl 项目相对路径 {
    /// 解析并校验一个项目相对路径。
    ///
    /// `输入` 可以带 `\` 或 `/`，两种分隔符都会被规范化。
    pub fn 解析(输入: impl AsRef<str>) -> Result<Self, 路径错误> {
        let 原文 = 输入.as_ref();
        let 去空白 = 原文.trim();
        if 去空白.is_empty() {
            return Err(路径错误::空路径);
        }

        // 先把反斜杠统一成斜杠，再交给 Path 处理。
        let 统一 = 去空白.replace('\\', "/");

        // 显式拦截盘符与 UNC：`C:/x`、`//server/share`。
        if 统一.starts_with('/') {
            return Err(路径错误::绝对路径(原文.to_string()));
        }
        let bytes = 统一.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' {
            return Err(路径错误::绝对路径(原文.to_string()));
        }

        let mut 段列表 = Vec::new();
        for 段 in 统一.split('/') {
            match 段 {
                "" => {
                    // 允许末尾单个斜杠造成的空段被忽略，中间的空段视为错误。
                    if 段列表.is_empty() && 统一.starts_with('/') {
                        unreachable!("前导斜杠已在上方拦截");
                    }
                    return Err(路径错误::空路径段(原文.to_string()));
                }
                "." => continue,
                ".." => return Err(路径错误::目录穿越(原文.to_string())),
                _ => 段列表.push(段),
            }
        }

        // 用 std::path 再过一遍，兜住 Windows 上的其他绝对路径形态（如 `\\?\`）。
        let 原始 = Path::new(&统一);
        for 组件 in 原始.components() {
            match 组件 {
                Component::Normal(_) => {}
                Component::CurDir => {}
                _ => return Err(路径错误::绝对路径(原文.to_string())),
            }
        }

        if 段列表.is_empty() {
            return Err(路径错误::空路径);
        }

        Ok(Self(段列表.join("/")))
    }

    /// 直接以已知合法的字符串构造，仅供内部与测试使用。
    ///
    /// 调用者必须自行保证 `值` 已规范化。
    pub fn 来自已规范化(值: impl Into<String>) -> Self {
        Self(值.into())
    }

    /// 取得规范化的字符串形式（始终使用 `/`）。
    pub fn 文本(&self) -> &str {
        &self.0
    }

    /// 转成当前平台的原生路径。
    pub fn 转原生(&self) -> PathBuf {
        self.0.split('/').collect()
    }

    /// 相对于某个项目根目录，拼出可用于文件系统访问的绝对路径。
    ///
    /// 由于构造时已排除 `..` 与绝对路径，这里不会逃出 `项目根`。
    pub fn 拼接(&self, 项目根: impl AsRef<Path>) -> PathBuf {
        项目根.as_ref().join(self.转原生())
    }

    /// 取文件名（最后一段）。
    pub fn 文件名(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// 取扩展名（小写，不含点）。无扩展名时返回 `None`。
    pub fn 扩展名(&self) -> Option<String> {
        self.文件名()
            .rsplit_once('.')
            .map(|(_, 扩展)| 扩展.to_ascii_lowercase())
            .filter(|扩展| !扩展.is_empty())
    }

    /// 取父目录路径。顶层文件返回 `None`。
    pub fn 父目录(&self) -> Option<项目相对路径> {
        self.0
            .rsplit_once('/')
            .map(|(父, _)| 项目相对路径(父.to_string()))
    }

    /// 换一个文件名，保留所在目录。
    pub fn 换文件名(&self, 新文件名: impl AsRef<str>) -> Result<项目相对路径, 路径错误> {
        match self.父目录() {
            Some(父) => 项目相对路径::解析(format!("{}/{}", 父.文本(), 新文件名.as_ref())),
            None => 项目相对路径::解析(新文件名),
        }
    }
}

impl fmt::Display for 项目相对路径 {
    fn fmt(&self, 格式: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, 格式)
    }
}

/// 资源唯一标识。
///
/// 与"路径"解耦：路径是**用户可见的组织方式**，UUID 是**引用稳定性的锚点**。
/// 资源被改名时，导入元数据里的 UUID 不变，场景中所有指向它的引用即可自动跟随。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct 资源唯一标识(pub Uuid);

impl 资源唯一标识 {
    /// 生成一个新的随机资源标识。
    pub fn 新建() -> Self {
        Self(Uuid::new_v4())
    }

    /// 由已有的 UUID 构造。
    pub fn 从(值: Uuid) -> Self {
        Self(值)
    }

    /// 取出内部的 UUID。
    pub fn 内部(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for 资源唯一标识 {
    fn fmt(&self, 格式: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, 格式)
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 解析时统一分隔符() {
        let 路径 = 项目相对路径::解析(r"textures\hero.png").unwrap();
        assert_eq!(路径.文本(), "textures/hero.png");
        assert_eq!(路径.文件名(), "hero.png");
        assert_eq!(路径.扩展名().as_deref(), Some("png"));
    }

    #[test]
    fn 拒绝绝对路径() {
        assert!(matches!(
            项目相对路径::解析("/etc/passwd"),
            Err(路径错误::绝对路径(_))
        ));
        assert!(matches!(
            项目相对路径::解析(r"C:\Windows\system32"),
            Err(路径错误::绝对路径(_))
        ));
    }

    #[test]
    fn 拒绝目录穿越() {
        assert!(matches!(
            项目相对路径::解析("../../secrets.txt"),
            Err(路径错误::目录穿越(_))
        ));
        assert!(matches!(
            项目相对路径::解析("assets/../../boom.png"),
            Err(路径错误::目录穿越(_))
        ));
    }

    #[test]
    fn 拒绝空路径与空段() {
        assert!(matches!(项目相对路径::解析(""), Err(路径错误::空路径)));
        assert!(matches!(
            项目相对路径::解析("a//b"),
            Err(路径错误::空路径段(_))
        ));
        assert!(matches!(
            项目相对路径::解析("assets/"),
            Err(路径错误::空路径段(_))
        ));
    }

    #[test]
    fn 忽略当前目录段() {
        let 路径 = 项目相对路径::解析("./assets/./hero.png").unwrap();
        assert_eq!(路径.文本(), "assets/hero.png");
    }

    #[test]
    fn 父目录与换名() {
        let 路径 = 项目相对路径::解析("assets/sprites/hero.png").unwrap();
        assert_eq!(路径.父目录().unwrap().文本(), "assets/sprites");
        assert_eq!(
            路径.换文件名("villain.png").unwrap().文本(),
            "assets/sprites/villain.png"
        );

        let 顶层 = 项目相对路径::解析("hero.png").unwrap();
        assert!(顶层.父目录().is_none());
    }

    #[test]
    fn 拼接不会逃出项目根() {
        let 路径 = 项目相对路径::解析("assets/hero.png").unwrap();
        let 绝对 = 路径.拼接("C:/project");
        assert!(绝对.starts_with("C:/project"));
    }

    #[test]
    fn 资源标识往返序列化() {
        let 标识 = 资源唯一标识::新建();
        let 文本 = serde_json::to_string(&标识).unwrap();
        let 还原: 资源唯一标识 = serde_json::from_str(&文本).unwrap();
        assert_eq!(标识, 还原);
    }
}
