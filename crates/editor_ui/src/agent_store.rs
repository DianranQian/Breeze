//! AI 会话的本地持久化：版本化格式、原子写入、损坏与中断恢复。
//!
//! # 为什么不用 `serde_json::to_writer` 直接覆盖
//!
//! 会话文件是**用户资产**：中途崩溃写坏一次，历史对话、任务记录和审查结论
//! 就全丢了。这里采用"同目录临时文件 + `sync_all` + 原子替换 + 保留上一份
//! 备份"的顺序，保证任何时刻磁盘上都有一份完整存档。
//!
//! # 版本升级
//!
//! 存档外层带 `格式版本`。读取时：
//!
//! * 版本高于本程序 → **拒绝载入且禁止覆盖**（旧程序不能理解新字段，
//!   覆盖会静默丢数据）；
//! * 版本低于本程序 → 按 serde 默认值补齐字段后迁移（缺 `创建时间`、
//!   `项目根`、基线等老字段都能读）；
//! * 文件损坏 → 先改名备份成 `.corrupt-<毫秒>.bak`，再返回空集合并如实报告。
//!
//! # 敏感数据
//!
//! 存档模型里**没有**凭证字段。用户如果把 API Key 粘进对话，保存前会经过
//! [`脱敏文本`] 处理：常见凭证模式替换成 `[已脱敏]`，并返回脱敏计数。
//! 这是"尽力而为"的保护，不是密码学保证 —— 所以界面上也会如实说明。
//!
//! # 项目失效
//!
//! 载入时只做 `is_dir()` 元数据检查，**不读项目内容、不自动打开**。
//! 项目被删除或移动后，历史仍然可读，界面明确标注"项目不可用"。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::agent::{智能体会话, 现在秒};

/// 当前存档格式版本。
///
/// * 1：最初版本（无创建时间/项目根/基线字段）；
/// * 2：增加会话创建时间、原始项目根、任务种类、基线、审查与验证状态。
pub const 会话格式版本: u32 = 2;

/// 存档文件名。
pub const 存档文件名: &str = "agent_sessions.json";

/// 磁盘上的存档外层。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 会话存档 {
    /// 格式版本。
    #[serde(default)]
    pub 格式版本: u32,
    /// 写入时间（UNIX 秒）。
    #[serde(default)]
    pub 写入时间: u64,
    /// 会话列表。
    #[serde(default)]
    pub 会话: Vec<智能体会话>,
}

/// 载入结果的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 存档来源 {
    /// 正常读取。
    文件,
    /// 文件不存在（首次使用）。
    文件缺失,
    /// 文件损坏，已备份。
    已损坏,
    /// 版本高于本程序，拒绝载入。
    版本不兼容,
    /// IO 失败。
    读取失败,
}

/// 载入结果。**所有字段都是真实结果**，没有占位。
#[derive(Debug, Clone, PartialEq)]
pub struct 载入结果 {
    /// 读到的会话。
    pub 会话: Vec<智能体会话>,
    /// 从哪个旧版本迁移而来。
    pub 迁移自: Option<u32>,
    /// 真实警告（损坏、路径失效、版本迁移等）。
    pub 警告: Vec<String>,
    /// 被标记为中断的任务数。
    pub 修正中断任务: usize,
    /// 是否禁止覆盖原文件（版本不兼容或读取失败时必须为 `true`）。
    pub 禁止覆盖: bool,
    /// 来源。
    pub 来源: 存档来源,
}

impl 载入结果 {
    /// 空结果（文件缺失/损坏时使用）。
    pub fn 空(来源: 存档来源) -> Self {
        Self {
            会话: Vec::new(),
            迁移自: None,
            警告: Vec::new(),
            修正中断任务: 0,
            禁止覆盖: matches!(来源, 存档来源::版本不兼容 | 存档来源::读取失败),
            来源,
        }
    }
}

/// 序列化会话（**不脱敏**，仅供测试与内部对照使用）。
pub fn 序列化(会话: &[智能体会话]) -> Result<String, String> {
    let 存档 = 会话存档 {
        格式版本: 会话格式版本,
        写入时间: 现在秒(),
        会话: 会话.to_vec(),
    };
    serde_json::to_string_pretty(&存档).map_err(|错误| format!("序列化会话失败：{错误}"))
}

/// 解析存档文本。不触碰文件系统。
pub fn 解析存档(文本: &str) -> 载入结果 {
    let 值: serde_json::Value = match serde_json::from_str(文本) {
        Ok(值) => 值,
        Err(错误) => {
            let mut 结果 = 载入结果::空(存档来源::已损坏);
            结果.警告.push(format!("存档不是合法 JSON：{错误}"));
            return 结果;
        }
    };
    let 版本 = 值
        .get("格式版本")
        .or_else(|| 值.get("version"))
        .and_then(|值| 值.as_u64())
        .unwrap_or(1) as u32;
    if 版本 > 会话格式版本 {
        let mut 结果 = 载入结果::空(存档来源::版本不兼容);
        结果.警告.push(format!(
            "存档版本 {版本} 高于本程序支持的 {会话格式版本}：未载入，且不会覆盖该文件"
        ));
        return 结果;
    }
    let mut 存档: 会话存档 = match serde_json::from_value(值) {
        Ok(存档) => 存档,
        Err(错误) => {
            let mut 结果 = 载入结果::空(存档来源::已损坏);
            结果.警告.push(format!("存档字段无法解析：{错误}"));
            return 结果;
        }
    };
    let 迁移自 = if 版本 < 会话格式版本 {
        Some(版本)
    } else {
        None
    };
    存档.格式版本 = 会话格式版本;
    let mut 警告 = Vec::new();
    if let Some(旧) = 迁移自 {
        警告.push(format!(
            "存档从版本 {旧} 迁移到 {会话格式版本}（缺失字段按默认值补齐）"
        ));
    }
    for 会话 in &mut 存档.会话 {
        迁移会话(会话);
    }
    // 中断任务必须标成"状态未知"，不能假装成功，也不能自动恢复。
    let mut 临时 = crate::agent::智能体状态 {
        会话列表: std::mem::take(&mut 存档.会话),
        ..Default::default()
    };
    let 修正中断任务 = 临时.修正中断任务();
    if 修正中断任务 > 0 {
        警告.push(format!(
            "{修正中断任务} 个任务在上次退出时仍在运行：已标记为状态未知（副作用需核对）"
        ));
    }
    let mut 会话 = 临时.会话列表;
    for 会话 in &mut 会话 {
        刷新项目可用性(会话, &mut 警告);
    }
    载入结果 {
        会话,
        迁移自,
        警告,
        修正中断任务,
        禁止覆盖: false,
        来源: 存档来源::文件,
    }
}

/// 补齐字段（迁移用）。
fn 迁移会话(会话: &mut 智能体会话) {
    if 会话.项目根.trim().is_empty() {
        会话.项目根 = 会话.项目路径.clone();
    }
    if 会话.创建时间 == 0 {
        会话.创建时间 = 会话.更新时间;
    }
    if 会话.创建时间 == 0 {
        会话.创建时间 = 现在秒();
    }
    // 旧任务没有结束时间：已结束的用开始时间兜底，进行中的由中断修正覆盖。
    for 任务 in &mut 会话.任务 {
        if 任务.结束时间.is_none() && 任务.状态.已结束() {
            任务.结束时间 = Some(任务.开始时间);
        }
    }
}

/// 检查项目是否仍然可用。只做目录存在性检查，不读取项目内容。
fn 刷新项目可用性(会话: &mut 智能体会话, 警告: &mut Vec<String>) {
    let 根 = if 会话.项目根.trim().is_empty() {
        &会话.项目路径
    } else {
        &会话.项目根
    };
    if 根.trim().is_empty() {
        会话.项目可用 = false;
        会话.项目状态说明 = Some("会话没有记录项目路径".to_string());
        return;
    }
    let 路径 = Path::new(根);
    会话.项目可用 = 路径.is_dir();
    if 会话.项目可用 {
        会话.项目状态说明 = None;
    } else {
        会话.项目状态说明 = Some(format!("项目目录不存在或已移动：{根}"));
        警告.push(format!(
            "会话「{}」的项目不可用：{根}（历史保留，但不会自动访问失效路径）",
            会话.标题
        ));
    }
}

/// 载入存档文件。
pub fn 载入文件(路径: &Path) -> 载入结果 {
    清理临时文件(路径);
    if !路径.exists() {
        return 载入结果::空(存档来源::文件缺失);
    }
    let 文本 = match std::fs::read_to_string(路径) {
        Ok(文本) => 文本,
        Err(错误) => {
            let mut 结果 = 载入结果::空(存档来源::读取失败);
            结果
                .警告
                .push(format!("读取会话存档失败：{} —— {错误}", 路径.display()));
            return 结果;
        }
    };
    let mut 结果 = 解析存档(&文本);
    if 结果.来源 == 存档来源::已损坏 {
        match 备份损坏文件(路径) {
            Ok(备份) => {
                结果
                    .警告
                    .push(format!("损坏的存档已备份到：{}", 备份.display()));
                // 有了备份才允许覆盖；备份失败时宁可让保存失败也不静默丢数据。
                结果.禁止覆盖 = false;
            }
            Err(错误) => {
                结果.警告.push(format!("备份损坏存档失败：{错误}"));
                结果.禁止覆盖 = true;
            }
        }
    }
    结果
}

/// 把损坏的存档改名备份（不删除）。
fn 备份损坏文件(路径: &Path) -> Result<PathBuf, String> {
    let 毫秒 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_millis())
        .unwrap_or(0);
    let 文件名 = 路径
        .file_name()
        .map(|名| 名.to_string_lossy().to_string())
        .unwrap_or_else(|| 存档文件名.to_string());
    let 备份 = 路径.with_file_name(format!("{文件名}.corrupt-{毫秒}.bak"));
    std::fs::rename(路径, &备份).map_err(|错误| format!("改名失败：{错误}"))?;
    Ok(备份)
}

/// 保存结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct 保存结果 {
    /// 脱敏替换次数。
    pub 脱敏数: usize,
    /// 写入字节数。
    pub 字节: usize,
}

/// 原子保存会话。
///
/// 步骤：同目录临时文件 → `sync_all` → 备份旧存档 → 原子替换。
/// 任一步失败都会删掉临时文件并返回真实错误。
pub fn 原子保存(路径: &Path, 会话: &[智能体会话]) -> Result<保存结果, String> {
    if let Some(父) = 路径.parent() {
        std::fs::create_dir_all(父)
            .map_err(|错误| format!("无法创建配置目录 {}：{错误}", 父.display()))?;
    }
    let (脱敏会话, 脱敏数) = 脱敏全部(会话);
    let 文本 = {
        let 存档 = 会话存档 {
            格式版本: 会话格式版本,
            写入时间: 现在秒(),
            会话: 脱敏会话,
        };
        serde_json::to_string_pretty(&存档).map_err(|错误| format!("序列化会话失败：{错误}"))?
    };
    let 临时 = 临时路径(路径);
    let 写结果 = (|| -> Result<(), String> {
        let mut 文件 = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&临时)
            .map_err(|错误| format!("无法创建临时存档 {}：{错误}", 临时.display()))?;
        文件
            .write_all(文本.as_bytes())
            .map_err(|错误| format!("写入临时存档失败：{错误}"))?;
        文件
            .sync_all()
            .map_err(|错误| format!("同步临时存档失败：{错误}"))?;
        Ok(())
    })();
    if let Err(错误) = 写结果 {
        let _ = std::fs::remove_file(&临时);
        return Err(错误);
    }
    // 备份上一份完整存档（尽力而为：备份失败不阻止本次保存，但会在错误里报告）。
    if 路径.exists() {
        let 备份 = 路径.with_extension("json.bak");
        let _ = std::fs::copy(路径, 备份);
    }
    if let Err(错误) = std::fs::rename(&临时, 路径) {
        let _ = std::fs::remove_file(&临时);
        return Err(format!(
            "原子替换存档失败：{} —— {错误}（原文件未改动）",
            路径.display()
        ));
    }
    Ok(保存结果 {
        脱敏数,
        字节: 文本.len(),
    })
}

static 临时序号: AtomicU64 = AtomicU64::new(0);

fn 临时路径(路径: &Path) -> PathBuf {
    let 序号 = 临时序号.fetch_add(1, Ordering::Relaxed);
    let 文件名 = 路径
        .file_name()
        .map(|名| 名.to_string_lossy().to_string())
        .unwrap_or_else(|| 存档文件名.to_string());
    let 临时名 = format!("{文件名}.tmp-{}-{序号}", std::process::id());
    路径.with_file_name(临时名)
}

/// 清理上次异常退出留下的临时文件。只删自己的命名模式。
pub fn 清理临时文件(路径: &Path) {
    let Some(父) = 路径.parent() else {
        return;
    };
    let 前缀 = format!("{存档文件名}.tmp-");
    let Ok(条目) = std::fs::read_dir(父) else {
        return;
    };
    for 项 in 条目.flatten() {
        let 名 = 项.file_name().to_string_lossy().to_string();
        if 名.starts_with(&前缀) {
            let _ = std::fs::remove_file(项.path());
        }
    }
}

/// 保存前的脱敏统计。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct 脱敏统计 {
    /// 替换次数。
    pub 数量: usize,
    /// 命中的模式（去重，仅用于日志；不包含被替换的原文）。
    pub 命中: Vec<String>,
}

/// 克隆会话并脱敏所有可能含凭证的文本字段。
pub fn 脱敏全部(会话: &[智能体会话]) -> (Vec<智能体会话>, usize) {
    let mut 结果 = 会话.to_vec();
    let mut 总数 = 0;
    for 会话 in &mut 结果 {
        for 消息 in &mut 会话.消息 {
            let (新, 数) = 脱敏文本(&消息.内容);
            消息.内容 = 新;
            总数 += 数;
        }
        for 任务 in &mut 会话.任务 {
            for 字段 in [&mut 任务.指令, &mut 任务.摘要, &mut 任务.诊断输入] {
                let (新, 数) = 脱敏文本(字段);
                *字段 = 新;
                总数 += 数;
            }
            for 调用 in &mut 任务.工具调用 {
                let (新, 数) = 脱敏文本(&调用.摘要);
                调用.摘要 = 新;
                总数 += 数;
            }
        }
        for 项 in &mut 会话.上下文 {
            let (新, 数) = 脱敏文本(&项.正文);
            项.正文 = 新;
            总数 += 数;
        }
        for 变更 in &mut 会话.变更 {
            let (新, 数) = 脱敏文本(&变更.差异);
            变更.差异 = 新;
            总数 += 数;
            if let Some(备注) = &mut 变更.备注 {
                let (新, 数) = 脱敏文本(备注);
                *备注 = 新;
                总数 += 数;
            }
        }
    }
    (结果, 总数)
}

/// 常见凭证模式。
///
/// 每一项是 `(前缀, 最小长度)`：命中前缀后继续吃掉 `[A-Za-z0-9_\-\.]`，
/// 长度达到下限才替换。前缀本身也区分大小写，避免误伤普通英文。
const 凭证前缀: &[(&str, usize)] = &[
    ("sk-", 16),
    ("sk_", 16),
    ("ghp_", 16),
    ("gho_", 16),
    ("github_pat_", 16),
    ("xoxb-", 12),
    ("xoxp-", 12),
    ("AKIA", 16),
    ("ASIA", 16),
    ("AIza", 20),
];

/// 键值形式的凭证字段名。
const 凭证键名: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "secret_key",
    "client_secret",
    "password",
    "passwd",
    "authorization",
];

/// 脱敏后的占位文本。
pub const 脱敏占位: &str = "[已脱敏]";

/// 把文本里的常见凭证替换成 [`脱敏占位`]，返回 `(新文本, 替换次数)`。
pub fn 脱敏文本(文本: &str) -> (String, usize) {
    let 字符: Vec<char> = 文本.chars().collect();
    let mut 结果 = String::with_capacity(文本.len());
    let mut 数量 = 0;
    let mut 位置 = 0;
    while 位置 < 字符.len() {
        // 1) `Bearer <token>`。
        if let Some(吃掉) = 匹配前缀(&字符, 位置, "Bearer ", 8) {
            结果.push_str(脱敏占位);
            数量 += 1;
            位置 += 吃掉;
            continue;
        }
        // 2) 知名前缀。
        let mut 命中 = false;
        for (前缀, 最小) in 凭证前缀 {
            if let Some(吃掉) = 匹配前缀(&字符, 位置, 前缀, *最小) {
                结果.push_str(脱敏占位);
                数量 += 1;
                位置 += 吃掉;
                命中 = true;
                break;
            }
        }
        if 命中 {
            continue;
        }
        // 3) `password=...` / `"api_key": "..."`。
        let mut 键命中 = false;
        for 键 in 凭证键名 {
            if 从位置匹配忽略大小写(&字符, 位置, 键) {
                let mut 扫描 = 位置 + 键.chars().count();
                while 扫描 < 字符.len() && matches!(字符[扫描], ' ' | '\t' | '"' | '\'') {
                    扫描 += 1;
                }
                if 扫描 < 字符.len() && matches!(字符[扫描], '=' | ':') {
                    扫描 += 1;
                    while 扫描 < 字符.len() && matches!(字符[扫描], ' ' | '\t' | '"' | '\'')
                    {
                        扫描 += 1;
                    }
                    let 起点 = 扫描;
                    while 扫描 < 字符.len()
                        && !matches!(
                            字符[扫描],
                            ' ' | '\t' | '\n' | '\r' | '"' | '\'' | ',' | ';' | '}' | ']'
                        )
                    {
                        扫描 += 1;
                    }
                    if 扫描 > 起点 && !字符[起点..扫描].iter().collect::<String>().is_empty()
                    {
                        结果.push_str(脱敏占位);
                        数量 += 1;
                        位置 = 扫描;
                        键命中 = true;
                        break;
                    }
                }
            }
        }
        if 键命中 {
            continue;
        }
        // 4) PEM 私钥整段。
        if let Some(吃掉) = 匹配私钥(&字符, 位置) {
            结果.push_str("[已脱敏私钥]");
            数量 += 1;
            位置 += 吃掉;
            continue;
        }
        结果.push(字符[位置]);
        位置 += 1;
    }
    (结果, 数量)
}

/// 检查文本中是否还有可疑凭证（保存/展示前的自检）。
pub fn 存档安全审查(文本: &str) -> Vec<String> {
    let (_, 数量) = 脱敏文本(文本);
    if 数量 > 0 {
        vec![format!("检测到 {数量} 处疑似凭证")]
    } else {
        Vec::new()
    }
}

fn 匹配前缀(字符: &[char], 位置: usize, 前缀: &str, 最小: usize) -> Option<usize> {
    if !从位置匹配忽略大小写(字符, 位置, 前缀) {
        return None;
    }
    let 前缀长 = 前缀.chars().count();
    let mut 扫描 = 位置 + 前缀长;
    while 扫描 < 字符.len() && 是凭证字符(字符[扫描]) {
        扫描 += 1;
    }
    let 后段 = 扫描 - (位置 + 前缀长);
    if 后段 >= 最小 {
        Some(扫描 - 位置)
    } else {
        None
    }
}

fn 是凭证字符(字符: char) -> bool {
    字符.is_ascii_alphanumeric() || matches!(字符, '_' | '-' | '.' | '+' | '/' | '=')
}

fn 从位置匹配忽略大小写(字符: &[char], 位置: usize, 模式: &str) -> bool {
    let 模式字符: Vec<char> = 模式.chars().collect();
    if 位置 + 模式字符.len() > 字符.len() {
        return false;
    }
    for (偏移, 期望) in 模式字符.iter().enumerate() {
        if !字符[位置 + 偏移].eq_ignore_ascii_case(期望) {
            return false;
        }
    }
    true
}

fn 匹配私钥(字符: &[char], 位置: usize) -> Option<usize> {
    let 文本: String = 字符[位置..].iter().take(64).collect();
    if !文本.starts_with("-----BEGIN") || !文本.contains("PRIVATE KEY") {
        return None;
    }
    let 剩余: String = 字符[位置..].iter().collect();
    let 结尾 = "-----END";
    let 结束 = 剩余.find(结尾)?;
    let 尾段 = &剩余[结束..];
    let 结束行 = 尾段.find("-----").map(|偏移| 偏移 + 5)?;
    let 吃掉: usize = 剩余[..结束 + 结束行].chars().count();
    Some(吃掉.min(字符.len() - 位置))
}

/// 按字符截断（避免切断多字节字符）。
pub fn 截断文本(文本: &str, 上限: usize) -> String {
    if 文本.chars().count() <= 上限 {
        return 文本.to_string();
    }
    文本.chars().take(上限).collect::<String>() + "…"
}

/// 按项目分组（界面"两个项目不串线"的纯逻辑）。
pub fn 按项目分组(会话: &[智能体会话]) -> BTreeMap<String, Vec<usize>> {
    let mut 结果: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (下标, 会话) in 会话.iter().enumerate() {
        结果.entry(会话.项目路径.clone()).or_default().push(下标);
    }
    结果
}

/// 切换项目后重新选中属于该项目的最近会话。
///
/// 返回选中的下标。没有项目时清空选择；项目没有任何会话时也清空选择
/// —— **绝不**保留上一个项目的会话（那会串线）。
pub fn 选择项目会话(
    状态: &mut crate::agent::智能体状态,
    项目根: Option<&Path>,
) -> Option<usize> {
    let Some(根) = 项目根 else {
        状态.当前会话 = None;
        return None;
    };
    let 键 = crate::agent::规范化项目路径(根);
    for 会话 in &mut 状态.会话列表 {
        if 会话.项目路径 == 键 {
            会话.项目根 = 根.to_string_lossy().to_string();
            会话.项目可用 = 根.is_dir();
            会话.项目状态说明 = if 会话.项目可用 {
                None
            } else {
                Some(format!("项目目录不存在：{}", 根.display()))
            };
        }
    }
    let 选中 = 状态.项目会话(&键).first().copied();
    状态.当前会话 = 选中;
    选中
}

/// 迁移后的会话是否仍带着"运行中"任务（供调用方断言的辅助函数）。
pub fn 有未结束任务(会话: &智能体会话) -> bool {
    会话.任务.iter().any(|任务| !任务.状态.已结束())
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::agent::{
        任务状态, 文件变更, 智能体任务, 智能体消息, 消息角色
    };

    fn 测试会话(标识: &str, 项目: &str) -> 智能体会话 {
        let mut 会话 = 智能体会话::新建(标识, 项目);
        会话.追加消息(消息角色::用户, "帮我修构建");
        会话.追加消息(消息角色::智能体, "好的");
        会话.任务.push(智能体任务 {
            标识: "t1".to_string(),
            指令: "修构建".to_string(),
            状态: 任务状态::成功,
            结束时间: Some(1),
            退出码: Some(0),
            ..Default::default()
        });
        会话
    }

    #[test]
    fn 保存载入往返保留消息任务与创建时间() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join(存档文件名);
        let 会话 = 测试会话("s1", 目录.path().to_string_lossy().as_ref());
        let 保存 = 原子保存(&路径, std::slice::from_ref(&会话)).unwrap();
        assert!(保存.字节 > 0);
        let 载入 = 载入文件(&路径);
        assert_eq!(载入.来源, 存档来源::文件);
        assert_eq!(载入.会话.len(), 1);
        assert_eq!(载入.会话[0].消息.len(), 2);
        assert_eq!(载入.会话[0].任务[0].状态, 任务状态::成功);
        assert_eq!(载入.会话[0].任务[0].退出码, Some(0));
        assert_eq!(载入.会话[0].创建时间, 会话.创建时间);
        assert!(载入.会话[0].项目可用);
    }

    #[test]
    fn 原子保存不留下临时文件() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join(存档文件名);
        原子保存(&路径, &[测试会话("s1", "C:/p")]).unwrap();
        原子保存(&路径, &[测试会话("s2", "C:/p")]).unwrap();
        let 残留: Vec<String> = std::fs::read_dir(目录.path())
            .unwrap()
            .flatten()
            .map(|项| 项.file_name().to_string_lossy().to_string())
            .filter(|名| 名.contains(".tmp-"))
            .collect();
        assert!(残留.is_empty(), "不应留下临时文件：{残留:?}");
        assert!(路径.with_extension("json.bak").exists(), "应保留上一份备份");
        let 载入 = 载入文件(&路径);
        assert_eq!(载入.会话[0].标识, "s2");
    }

    #[test]
    fn 损坏存档会备份并返回空结果() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join(存档文件名);
        std::fs::write(&路径, "{ 这不是 json").unwrap();
        let 载入 = 载入文件(&路径);
        assert_eq!(载入.来源, 存档来源::已损坏);
        assert!(载入.会话.is_empty());
        assert!(!载入.禁止覆盖, "有备份后允许重新保存");
        let 备份数 = std::fs::read_dir(目录.path())
            .unwrap()
            .flatten()
            .filter(|项| 项.file_name().to_string_lossy().contains(".corrupt-"))
            .count();
        assert_eq!(备份数, 1, "损坏文件必须被保留为备份，而不是删除");
    }

    #[test]
    fn 未来版本拒绝载入且禁止覆盖() {
        let 文本 = r#"{"格式版本":999,"写入时间":0,"会话":[]}"#;
        let 载入 = 解析存档(文本);
        assert_eq!(载入.来源, 存档来源::版本不兼容);
        assert!(载入.禁止覆盖, "不能覆盖旧程序看不懂的存档");
        assert!(载入.警告[0].contains("999"));
    }

    #[test]
    fn 旧版本会迁移并补齐字段() {
        let 文本 = r#"{
            "格式版本": 1,
            "写入时间": 0,
            "会话": [{
                "标识": "old",
                "标题": "旧会话",
                "项目路径": "C:/p",
                "消息": [{"角色": "用户", "内容": "你好"}],
                "任务": [{"标识": "t", "指令": "构建", "状态": "执行中"}],
                "上下文": [],
                "变更": [],
                "更新时间": 10
            }]
        }"#;
        let 载入 = 解析存档(文本);
        assert_eq!(载入.迁移自, Some(1));
        assert_eq!(载入.会话[0].创建时间, 10);
        assert_eq!(载入.会话[0].项目根, "C:/p");
        assert_eq!(载入.修正中断任务, 1);
        assert_eq!(载入.会话[0].任务[0].状态, 任务状态::状态未知);
        assert!(载入.会话[0].任务[0].摘要.contains("中断"));
    }

    #[test]
    fn 项目失效时保留历史但不自动访问() {
        let 目录 = tempfile::tempdir().unwrap();
        let 不存在 = 目录.path().join("已经被删除的项目");
        let mut 会话 = 测试会话("s1", 不存在.to_string_lossy().as_ref());
        会话.项目根 = 不存在.to_string_lossy().to_string();
        会话.项目路径 = crate::agent::规范化项目路径(&不存在);
        let 路径 = 目录.path().join(存档文件名);
        原子保存(&路径, &[会话]).unwrap();
        let 载入 = 载入文件(&路径);
        assert_eq!(载入.会话.len(), 1, "历史消息必须保留");
        assert!(!载入.会话[0].项目可用);
        assert!(
            载入.会话[0]
                .项目状态说明
                .as_deref()
                .unwrap_or("")
                .contains("已经被删除的项目")
        );
        assert!(载入.警告.iter().any(|警告| 警告.contains("项目不可用")));
    }

    #[test]
    fn 保存会脱敏常见凭证() {
        let mut 会话 = 智能体会话::新建("s1", "C:/p");
        会话.追加消息(
            消息角色::用户,
            "我的 key 是 sk-abcdefghijklmnopqrstuvwx，别写进文件",
        );
        会话.任务.push(智能体任务 {
            指令: "api_key=supersecretvalue123".to_string(),
            ..Default::default()
        });
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join(存档文件名);
        原子保存(&路径, std::slice::from_ref(&会话)).unwrap();
        let 文本 = std::fs::read_to_string(&路径).unwrap();
        assert!(!文本.contains("sk-abcdefghijklmnopqrstuvwx"));
        assert!(!文本.contains("supersecretvalue123"));
        assert!(文本.contains(脱敏占位));
    }

    #[test]
    fn 存档模型没有凭证字段() {
        let 文本 = 序列化(&[测试会话("s1", "C:/p")]).unwrap();
        for 禁止 in ["api_key", "password", "credential", "client_secret"] {
            assert!(
                !文本.contains(禁止),
                "普通会话不应出现凭证字段 {禁止}：{文本}"
            );
        }
    }

    #[test]
    fn 脱敏不误伤普通文本() {
        let 原文 = "把 src/main.rs 第 12 行的变量名改一下，然后运行 cargo test";
        let (新, 数) = 脱敏文本(原文);
        assert_eq!(新, 原文);
        assert_eq!(数, 0);
    }

    #[test]
    fn 脱敏覆盖多种凭证形式() {
        assert!(脱敏文本("Bearer abcdefgh12345").0.contains(脱敏占位));
        assert!(
            脱敏文本("password = hunter2").0.contains(脱敏占位),
            "键值形式必须脱敏"
        );
        assert!(
            脱敏文本("ghp_1234567890abcdef").0.contains(脱敏占位),
            "GitHub token 必须脱敏"
        );
    }

    #[test]
    fn 截断按字符() {
        assert_eq!(截断文本("中文中文", 2), "中文…");
        assert_eq!(截断文本("ab", 5), "ab");
    }

    #[test]
    fn 按项目分组不会串线() {
        let 会话 = vec![
            测试会话("a1", "c:/proj/a"),
            测试会话("b1", "c:/proj/b"),
            测试会话("a2", "c:/proj/a"),
        ];
        let 分组 = 按项目分组(&会话);
        assert_eq!(分组["c:/proj/a"], vec![0, 2]);
        assert_eq!(分组["c:/proj/b"], vec![1]);
    }

    #[test]
    fn 有未结束任务的辅助函数() {
        let 会话 = 测试会话("s1", "C:/p");
        assert!(!有未结束任务(&会话));
        let 文本 = r#"{"格式版本":2,"会话":[{"标识":"s","标题":"t","项目路径":"C:/p","项目根":"C:/p","消息":[],"任务":[{"标识":"t","状态":"执行中"}],"上下文":[],"变更":[],"创建时间":1,"更新时间":1}]}"#;
        let 载入 = 解析存档(文本);
        assert!(!有未结束任务(&载入.会话[0]), "载入后已被修正为状态未知");
    }

    #[test]
    fn 读取失败时禁止覆盖() {
        let 目录 = tempfile::tempdir().unwrap();
        let 路径 = 目录.path().join(存档文件名);
        std::fs::create_dir(&路径).unwrap(); // 同名目录：读文件必然失败
        let 载入 = 载入文件(&路径);
        assert_eq!(载入.来源, 存档来源::读取失败);
        assert!(载入.禁止覆盖);
    }

    #[test]
    fn 文件缺失是正常首次使用() {
        let 目录 = tempfile::tempdir().unwrap();
        let 载入 = 载入文件(&目录.path().join(存档文件名));
        assert_eq!(载入.来源, 存档来源::文件缺失);
        assert!(!载入.禁止覆盖);
        assert!(载入.会话.is_empty());
        assert!(载入.警告.is_empty());
    }

    #[test]
    fn 切换项目会选中该项目自己的会话() {
        let 目录 = tempfile::tempdir().unwrap();
        let 甲根 = 目录.path().join("甲");
        let 乙根 = 目录.path().join("乙");
        std::fs::create_dir_all(&甲根).unwrap();
        std::fs::create_dir_all(&乙根).unwrap();
        let mut 状态 = crate::agent::智能体状态::default();
        let 甲 = 状态.新建会话(&crate::agent::规范化项目路径(&甲根), "甲会话");
        let 乙 = 状态.新建会话(&crate::agent::规范化项目路径(&乙根), "乙会话");
        状态.会话列表[乙].更新时间 += 10;

        assert_eq!(选择项目会话(&mut 状态, Some(&甲根)), Some(甲));
        assert_eq!(状态.当前().unwrap().标识, "甲会话");
        assert_eq!(状态.当前().unwrap().项目根, 甲根.to_string_lossy());

        assert_eq!(选择项目会话(&mut 状态, Some(&乙根)), Some(乙));
        assert_eq!(状态.当前().unwrap().标识, "乙会话");
        assert_eq!(
            选择项目会话(&mut 状态, None),
            None,
            "没有项目时不能留着旧会话"
        );
        assert_eq!(状态.当前会话, None);
    }

    #[test]
    fn 未使用的消息类型不会导致编译告警() {
        let _ = 智能体消息 {
            角色: 消息角色::系统,
            内容: String::new(),
            时间: 0,
        };
        let _ = 文件变更::default();
    }
}
