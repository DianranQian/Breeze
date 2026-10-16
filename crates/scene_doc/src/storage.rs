//! 场景读写与存储抽象。
//!
//! # 为什么存储要抽象成 trait
//!
//! 需求里有几条硬性要求，它们只有在"能把存储替换掉"的前提下才测得到：
//!
//! * **保存失败时保留原始文件** —— 要用一个会失败的存储来验证；
//! * **损坏文件恢复** —— 要能构造出"原文件没了、备份还在"的现场；
//! * **性能：不要每次改属性都重写整个项目** —— 要能数出真实写入次数。
//!
//! 因此 [`场景存储`] 是一个 trait，生产用 [`文件系统场景存储`]，
//! 测试用 [`临时存储`]。
//!
//! # 保存的原子性
//!
//! 直接 `File::create(原文件)` 然后写入是**危险**的：一旦写到一半断电、
//! 磁盘满、或进程被 kill，场景文件就变成半截内容，而这是用户唯一的资产。
//!
//! 本实现采用三段式写盘：
//!
//! ```text
//! 1. 写入  <场景>.tmp      —— 内容不完整也只污染临时文件
//! 2. 把原文件改名为 <场景>.bak —— 保留上一个完好版本
//! 3. 把 .tmp 改名为原文件      —— 原子替换
//! 成功后删除 .bak；任一步失败则回滚
//! ```
//!
//! 读取时按 `原文件 → .bak → .tmp` 的顺序尝试，覆盖"恰好崩溃在第 2、3 步之间"
//! 的现场。这样最坏情况也只是丢失最后一次保存，而不是丢掉整个场景。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::document::{场景文档, 当前格式版本};
use crate::error::{场景结果, 场景错误};

/// 场景文件的默认扩展名。
pub const 默认场景扩展名: &str = "scene.ron";

/// 读取结果。
#[derive(Debug, Clone)]
pub struct 读取结果 {
    /// 解析出的场景文档。
    pub 文档: 场景文档,
    /// 实际读取的文件路径。若回退到了备份，这里会是备份路径。
    pub 来源路径: PathBuf,
    /// 是否是从备份或临时文件恢复的（意味着上次保存可能未完成）。
    pub 从备份恢复: bool,
}

/// 保存结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 保存结果 {
    /// 新建了文件。
    新建,
    /// 覆盖了已有文件。
    覆盖,
}

/// 场景存储抽象。
pub trait 场景存储 {
    /// 读取一个场景。
    fn 读取(&self, 路径: &Path) -> 场景结果<读取结果>;

    /// 写入一个场景。
    fn 写入(&self, 路径: &Path, 文档: &场景文档) -> 场景结果<保存结果>;

    /// 判断场景是否存在。
    fn 存在(&self, 路径: &Path) -> bool;
}

/// 把场景文档编码为文本。
///
/// 用**美化输出**是刻意的选择：场景文件是要进 Git 的，
/// 一行一个字段才能让 `git diff` 和代码评审有意义。
pub fn 编码场景(文档: &场景文档) -> 场景结果<String> {
    let 配置 = ron::ser::PrettyConfig::default()
        .struct_names(false)
        .depth_limit(32)
        .new_line("\n".to_string());
    ron::ser::to_string_pretty(文档, 配置)
        .map_err(|错误| 场景错误::编码失败(错误.to_string()))
}

/// 从文本解码场景文档，并做格式版本检查。
pub fn 解码场景(文本: &str, 路径: &Path) -> 场景结果<场景文档> {
    // 先用一个宽松的结构读出格式版本，这样即使后续字段解析失败，
    // 也能给出"版本过新"这种更有价值的错误，而不是笼统的解析失败。
    if let Some(文件版本) = 探测格式版本(文本)
        && 文件版本 > 当前格式版本
    {
        return Err(场景错误::版本过新 {
            文件版本,
            支持版本: 当前格式版本,
        });
    }

    let 文档: 场景文档 = ron::from_str(文本).map_err(|来源| 场景错误::格式解析 {
        路径: 路径.to_path_buf(),
        来源: Box::new(来源),
    })?;

    if 文档.格式版本 > 当前格式版本 {
        return Err(场景错误::版本过新 {
            文件版本: 文档.格式版本,
            支持版本: 当前格式版本,
        });
    }

    Ok(文档)
}

/// 在不完整解析的前提下探测 `格式版本` 字段。
///
/// 只做文本级的浅层扫描：目标是**尽早**判断"这个文件我读不懂"，
/// 因此不追求完备，探测不到就返回 `None` 交给完整解析去报错。
fn 探测格式版本(文本: &str) -> Option<u32> {
    let 位置 = 文本.find("格式版本")?;
    let 后半 = &文本[位置..];
    let 冒号 = 后半.find(':')?;
    let 剩余 = 后半[冒号 + 1..].trim_start();
    let 数字: String = 剩余.chars().take_while(char::is_ascii_digit).collect();
    数字.parse().ok()
}

/// 基于文件系统的场景存储。
#[derive(Debug, Default, Clone)]
pub struct 文件系统场景存储 {
    /// 是否在成功写入后保留 `.bak` 文件。默认 `false`（写完即删）。
    pub 保留备份: bool,
}

impl 文件系统场景存储 {
    /// 创建默认配置的存储。
    pub fn 新建() -> Self {
        Self {
            保留备份: false
        }
    }

    /// 临时文件路径。
    fn 临时路径(路径: &Path) -> PathBuf {
        加后缀(路径, ".tmp")
    }

    /// 备份文件路径。
    fn 备份路径(路径: &Path) -> PathBuf {
        加后缀(路径, ".bak")
    }
}

/// 在完整文件名后追加后缀：`a.scene.ron` -> `a.scene.ron.tmp`。
fn 加后缀(路径: &Path, 后缀: &str) -> PathBuf {
    let mut 文本 = 路径.as_os_str().to_os_string();
    文本.push(后缀);
    PathBuf::from(文本)
}

impl 场景存储 for 文件系统场景存储 {
    fn 读取(&self, 路径: &Path) -> 场景结果<读取结果> {
        let 候选 = [
            (路径.to_path_buf(), false),
            (Self::备份路径(路径), true),
            (Self::临时路径(路径), true),
        ];

        let 存在列表: Vec<_> = 候选
            .into_iter()
            .filter(|(候选路径, _)| 候选路径.is_file())
            .collect();

        if 存在列表.is_empty() {
            return Err(场景错误::文件(
                路径,
                std::io::Error::new(std::io::ErrorKind::NotFound, "场景文件不存在"),
            ));
        }

        let mut 最后错误 = None;

        for (候选路径, 从备份) in 存在列表 {
            let 文本 = match std::fs::read_to_string(&候选路径) {
                Ok(文本) => 文本,
                Err(错误) => {
                    最后错误 = Some(场景错误::文件(候选路径.clone(), 错误));
                    continue;
                }
            };
            match 解码场景(&文本, &候选路径) {
                Ok(文档) => {
                    return Ok(读取结果 {
                        文档,
                        来源路径: 候选路径,
                        从备份恢复: 从备份,
                    });
                }
                Err(错误) => {
                    最后错误 = Some(错误);
                }
            }
        }

        Err(最后错误.unwrap_or_else(|| {
            场景错误::文件(路径, std::io::Error::other("所有候选文件都无法解析"))
        }))
    }

    fn 写入(&self, 路径: &Path, 文档: &场景文档) -> 场景结果<保存结果> {
        let 已存在 = 路径.is_file();
        let 文本 = 编码场景(文档)?;

        if let Some(父目录) = 路径.parent()
            && !父目录.as_os_str().is_empty()
        {
            std::fs::create_dir_all(父目录).map_err(|错误| 场景错误::文件(父目录, 错误))?;
        }

        let 临时 = Self::临时路径(路径);
        let 备份 = Self::备份路径(路径);

        // 第 1 步：完整写入临时文件并落盘。
        // `sync_all` 是必要的 —— 否则数据可能只在内核页缓存里，
        // 断电后重命名成功但内容是空的。
        {
            use std::io::Write;
            let mut 句柄 = std::fs::File::create(&临时)
                .map_err(|错误| 场景错误::文件(临时.clone(), 错误))?;
            句柄
                .write_all(文本.as_bytes())
                .map_err(|错误| 场景错误::文件(临时.clone(), 错误))?;
            句柄
                .flush()
                .map_err(|错误| 场景错误::文件(临时.clone(), 错误))?;
            句柄
                .sync_all()
                .map_err(|错误| 场景错误::文件(临时.clone(), 错误))?;
        }

        // 第 2 步：把现有文件挪到备份。
        // 若上一步成功后这里失败，原文件仍然完好，直接清理临时文件返回错误即可。
        if 已存在 {
            let _ = std::fs::remove_file(&备份);
            if let Err(错误) = std::fs::rename(路径, &备份) {
                let _ = std::fs::remove_file(&临时);
                return Err(场景错误::保存失败 {
                    原文件: 路径.to_path_buf(),
                    来源: 错误,
                });
            }
        }

        // 第 3 步：临时文件就位。
        if let Err(错误) = std::fs::rename(&临时, 路径) {
            // 回滚：把备份搬回原位，保证用户的原文件不丢。
            if 已存在 {
                let _ = std::fs::rename(&备份, 路径);
            }
            let _ = std::fs::remove_file(&临时);
            return Err(场景错误::保存失败 {
                原文件: 路径.to_path_buf(),
                来源: 错误,
            });
        }

        if !self.保留备份 {
            let _ = std::fs::remove_file(&备份);
        }

        Ok(if 已存在 {
            保存结果::覆盖
        } else {
            保存结果::新建
        })
    }

    fn 存在(&self, 路径: &Path) -> bool {
        路径.is_file() || Self::备份路径(路径).is_file()
    }
}

/// 内存存储，用于测试。
///
/// # 为什么用 `RefCell` 而不是 `&mut self`
///
/// [`场景存储`] 的方法取 `&self`，因为生产实现（文件系统）不需要可变状态。
/// 测试实现却需要累计写入次数、并支持"写入必定失败"的注入。
/// 用内部可变性可以在**不改变 trait 形状**的前提下做到这两点，
/// 避免为了测试便利把 `&self` 改成 `&mut self` 而污染生产接口。
#[derive(Debug, Default)]
pub struct 临时存储 {
    /// 路径到内容的映射。键统一为小写正斜杠形式，
    /// 避免 Windows 上大小写与路径前缀差异导致测试不稳定。
    文件: std::cell::RefCell<BTreeMap<String, String>>,
    /// 设为一个错误信息后，所有写入都会失败。
    写入失败原因: std::cell::RefCell<Option<String>>,
    /// 累计成功写入次数。用于验证"改一个属性不会重写整个场景"。
    写入次数: std::cell::Cell<usize>,
    /// 是否模拟"写入前原文件已经被破坏"，用于测试损坏恢复路径。
    模拟原文件损坏: std::cell::Cell<bool>,
    /// 是否在写入成功后删除 `.bak`。与生产实现保持一致，默认 `true`。
    写入后清理备份: std::cell::Cell<bool>,
}

impl Clone for 临时存储 {
    fn clone(&self) -> Self {
        Self {
            文件: std::cell::RefCell::new(self.文件.borrow().clone()),
            写入失败原因: std::cell::RefCell::new(self.写入失败原因.borrow().clone()),
            写入次数: std::cell::Cell::new(self.写入次数.get()),
            模拟原文件损坏: std::cell::Cell::new(self.模拟原文件损坏.get()),
            写入后清理备份: std::cell::Cell::new(self.写入后清理备份.get()),
        }
    }
}

impl 临时存储 {
    /// 创建一个空存储。
    pub fn 新建() -> Self {
        Self {
            写入后清理备份: std::cell::Cell::new(true),
            ..Default::default()
        }
    }

    /// 让后续所有写入都失败，并返回指定错误信息。
    pub fn 设置写入失败(&self, 原因: impl Into<String>) {
        *self.写入失败原因.borrow_mut() = Some(原因.into());
    }

    /// 取消写入失败注入。
    pub fn 清除写入失败(&self) {
        *self.写入失败原因.borrow_mut() = None;
    }

    /// 设置写入后是否清理 `.bak`。
    pub fn 设置保留备份(&self, 保留: bool) {
        self.写入后清理备份.set(!保留);
    }

    /// 模拟原文件内容损坏（用于测试回退到备份的逻辑）。
    pub fn 损坏原文件(&self, 路径: &Path) {
        self.文件
            .borrow_mut()
            .insert(键(路径), "这不是合法的场景内容 {{{".to_string());
    }

    /// 预置一个文件内容。
    pub fn 放入(&self, 路径: &Path, 内容: impl Into<String>) {
        self.文件.borrow_mut().insert(键(路径), 内容.into());
    }

    /// 读取文件内容。
    pub fn 取出(&self, 路径: &Path) -> Option<String> {
        self.文件.borrow().get(&键(路径)).cloned()
    }

    /// 移除一个文件。
    pub fn 移除(&self, 路径: &Path) -> bool {
        self.文件.borrow_mut().remove(&键(路径)).is_some()
    }

    /// 已存文件数量。
    pub fn 文件数量(&self) -> usize {
        self.文件.borrow().len()
    }

    /// 所有文件路径（已规范化）。
    pub fn 路径列表(&self) -> Vec<String> {
        self.文件.borrow().keys().cloned().collect()
    }

    /// 累计成功写入次数。
    pub fn 写入次数(&self) -> usize {
        self.写入次数.get()
    }
}

/// 把路径规范化为存储键。
fn 键(路径: &Path) -> String {
    路径.to_string_lossy().replace('\\', "/").to_lowercase()
}

impl 场景存储 for 临时存储 {
    fn 读取(&self, 路径: &Path) -> 场景结果<读取结果> {
        // 与原文件系统实现保持同样的回退语义：按 原文件 → 备份 → 临时文件
        // 依次尝试，**每一个候选都要完整解析成功才算数**。
        // 只检查"文件是否存在"是不够的 —— 一个存在但内容损坏的原文件
        // 会让读取提前失败，从而永远用不上完好的备份。
        let 候选 = [
            (路径.to_path_buf(), false),
            (加后缀(路径, ".bak"), true),
            (加后缀(路径, ".tmp"), true),
        ];

        let mut 最后错误 = None;
        for (候选路径, 从备份) in 候选 {
            let Some(文本) = self.取出(&候选路径) else {
                continue;
            };
            match 解码场景(&文本, &候选路径) {
                Ok(文档) => {
                    return Ok(读取结果 {
                        文档,
                        来源路径: 候选路径,
                        从备份恢复: 从备份,
                    });
                }
                Err(错误) => 最后错误 = Some(错误),
            }
        }

        Err(最后错误.unwrap_or_else(|| {
            场景错误::文件(
                路径,
                std::io::Error::new(std::io::ErrorKind::NotFound, "场景文件不存在"),
            )
        }))
    }

    fn 写入(&self, 路径: &Path, 文档: &场景文档) -> 场景结果<保存结果> {
        if let Some(原因) = self.写入失败原因.borrow().clone() {
            return Err(场景错误::保存失败 {
                原文件: 路径.to_path_buf(),
                来源: std::io::Error::other(原因),
            });
        }

        let 文本 = 编码场景(文档)?;
        let 已存在 = self.文件.borrow().contains_key(&键(路径));

        // 与生产实现保持同样的三段式语义：先写临时、再备份、最后就位。
        self.文件
            .borrow_mut()
            .insert(键(&加后缀(路径, ".tmp")), 文本.clone());
        if 已存在 {
            let 原内容 = self
                .文件
                .borrow()
                .get(&键(路径))
                .cloned()
                .unwrap_or_default();
            self.文件
                .borrow_mut()
                .insert(键(&加后缀(路径, ".bak")), 原内容);
        }
        self.文件.borrow_mut().insert(键(路径), 文本);
        self.文件.borrow_mut().remove(&键(&加后缀(路径, ".tmp")));

        if self.写入后清理备份.get() {
            self.文件.borrow_mut().remove(&键(&加后缀(路径, ".bak")));
        }

        if self.模拟原文件损坏.get() {
            // 模拟"备份有效但原文件已被写坏"的现场。
            self.文件
                .borrow_mut()
                .insert(键(路径), "损坏内容".to_string());
        }

        self.写入次数.set(self.写入次数.get() + 1);
        Ok(if 已存在 {
            保存结果::覆盖
        } else {
            保存结果::新建
        })
    }

    fn 存在(&self, 路径: &Path) -> bool {
        self.文件.borrow().contains_key(&键(路径))
    }
}
