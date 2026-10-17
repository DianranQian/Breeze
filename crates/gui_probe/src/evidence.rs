//! 采集证据的读取（由 `editor_app::capture` 写出）。
//!
//! 证据格式刻意设计成"只增不改"：验收工具用 `#[serde(default)]` 容忍新字段，
//! 因此编辑器加入主题、图标等新证据项时，旧版工具仍然能读旧字段，
//! 不会因为一个字段缺失就把整次验收判成失败。

use serde::Deserialize;

/// 证据里的尺寸。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct 尺寸 {
    pub width: u32,
    pub height: u32,
}

/// 证据里的工作区摘要。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct 工作区证据 {
    /// 稳定 id。
    #[serde(default)]
    pub id: String,
    /// 当前语言下的显示名。
    #[serde(default)]
    pub name: String,
    /// 是否内置。
    #[serde(default)]
    pub builtin: bool,
    /// 工作区总数。
    #[serde(default)]
    pub count: usize,
}

/// 证据里的主题摘要。
#[derive(Debug, Clone, Deserialize)]
pub struct 主题证据 {
    /// 主题标识（`dark` / `light` / `high_contrast` / 自定义名）。
    #[serde(default)]
    pub id: String,
    /// 面板背景色 `#RRGGBB`。
    #[serde(default)]
    pub panel_bg: String,
    /// 主文字色 `#RRGGBB`。
    #[serde(default)]
    pub text: String,
    /// 强调色 `#RRGGBB`。
    #[serde(default)]
    pub accent: String,
}

impl Default for 主题证据 {
    fn default() -> Self {
        Self {
            id: "unknown".to_string(),
            panel_bg: String::new(),
            text: String::new(),
            accent: String::new(),
        }
    }
}

/// 证据里的一块区域。
#[derive(Debug, Clone, Deserialize)]
pub struct 区域证据 {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 完整证据。
#[derive(Debug, Clone, Deserialize)]
pub struct 证据 {
    #[serde(default)]
    pub schema: u32,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub bevy_version: String,
    /// 语言代号：`zh` / `en`。
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub language_label: String,
    #[serde(default = "默认倍数")]
    pub ui_scale: f32,
    #[serde(default = "默认倍数")]
    pub pixels_per_point: f32,
    pub window: 尺寸,
    #[serde(default)]
    pub workspace: 工作区证据,
    #[serde(default)]
    pub theme: 主题证据,
    #[serde(default)]
    pub regions: Vec<区域证据>,
    #[serde(default)]
    pub status_left: Vec<String>,
    #[serde(default)]
    pub status_right: Vec<String>,
    #[serde(default)]
    pub panel_titles: Vec<String>,
    #[serde(default)]
    pub panel_keys: Vec<String>,
    #[serde(default)]
    pub viewport_scale_percent: f32,
    #[serde(default)]
    pub icon_count: usize,
}

fn 默认倍数() -> f32 {
    1.0
}

impl 证据 {
    /// 读取证据文件。
    pub fn 读取(路径: &std::path::Path) -> Result<Self, String> {
        let 文本 = std::fs::read_to_string(路径)
            .map_err(|错误| format!("读不到证据文件 {}：{错误}", 路径.display()))?;
        serde_json::from_str(&文本)
            .map_err(|错误| format!("证据文件不是合法 JSON（{}）：{错误}", 路径.display()))
    }

    /// 状态栏全部文案（左 + 右）。
    pub fn 状态栏文案(&self) -> String {
        let mut 文本 = self.status_left.join(" | ");
        if !self.status_right.is_empty() {
            文本.push_str(" | ");
            文本.push_str(&self.status_right.join(" | "));
        }
        文本
    }

    /// 面板标题全部文案。
    pub fn 面板标题文案(&self) -> String {
        self.panel_titles.join(" | ")
    }

    /// 把 `#RRGGBB` 解析成 RGB。格式非法时返回 `None`。
    pub fn 解析颜色(文本: &str) -> Option<(u8, u8, u8)> {
        let 文本 = 文本.trim().strip_prefix('#')?;
        if 文本.len() != 6 {
            return None;
        }
        let 值 = u32::from_str_radix(文本, 16).ok()?;
        Some(((值 >> 16) as u8, (值 >> 8) as u8, 值 as u8))
    }
}
