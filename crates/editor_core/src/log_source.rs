//! 日志来源分类。
//!
//! # 为什么用"键前缀"而不是给每条日志加字段
//!
//! 编辑器里有**三百多处**日志调用（`结构化信息` / `警告` / `错误`）。
//! 给 `日志条目` 加一个"来源"参数意味着改动全部调用点，几乎必然漏改，
//! 而漏改的那几处会静默归到错误来源里 —— 比没有分类更糟。
//!
//! 现有日志的键有稳定的命名约定（`log_build_*`、`log_physics_*`、
//! `log_import_*` …），因此这里按前缀分类：**零调用点改动**，规则集中在一处，
//! 有单元测试守着每个前缀的归属。将来真要精确到子系统，再改成显式字段即可
//! （届时这个模块就是唯一需要改的地方）。

use crate::state::{日志内容, 日志条目};

/// 日志来源分类。
///
/// 与 Godot 的 Output 面板把"构建输出 / 调试输出"分开是同一个目的：
/// 用户排查问题时先按"是谁在说话"缩小范围，比按级别更有效 ——
/// 一次构建失败的十几行都来自构建，而编辑器的提示混在其中只是噪声。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum 日志来源 {
    /// 编辑器自身（打开/保存/选择/界面操作等）。
    编辑器,
    /// 构建与打包（cargo 输出、诊断、产物）。
    构建,
    /// 运行时（运行/停止/物理/碰撞/脚本输出）。
    运行,
    /// 资源与导入（资源缺失、导入状态、依赖检查）。
    资源,
}

impl 日志来源 {
    /// 用于显示的语言键。
    pub fn 词条键(&self) -> &'static str {
        match self {
            // 复用既有词条，避免为分类再造一套同义词。
            日志来源::编辑器 => "log_source_editor",
            日志来源::构建 => "log_source_build",
            日志来源::运行 => "log_source_runtime",
            日志来源::资源 => "log_source_assets",
        }
    }

    /// 全部来源（顺序即界面上的显示顺序）。
    pub fn 全部() -> [日志来源; 4] {
        [
            日志来源::编辑器,
            日志来源::构建,
            日志来源::运行,
            日志来源::资源,
        ]
    }

    /// 由词条键推断来源。
    pub fn 由键(键: &str) -> 日志来源 {
        // 顺序重要：先匹配更具体的前缀。
        const 构建前缀: [&str; 4] = ["log_build", "log_package", "log_diagnostic", "log_cargo"];
        const 运行前缀: [&str; 6] = [
            "log_physics",
            "log_run",
            "log_project_run",
            "log_scene_autoload",
            "log_runtime",
            "log_collision",
        ];
        const 资源前缀: [&str; 5] = [
            "log_import",
            "log_asset",
            "log_resource",
            "log_dependency",
            "log_audio",
        ];
        if 构建前缀.iter().any(|前缀| 键.starts_with(前缀)) {
            return 日志来源::构建;
        }
        if 运行前缀.iter().any(|前缀| 键.starts_with(前缀)) {
            return 日志来源::运行;
        }
        if 资源前缀.iter().any(|前缀| 键.starts_with(前缀)) {
            return 日志来源::资源;
        }
        日志来源::编辑器
    }

    /// 由一条日志推断来源。自由文本一律算编辑器（它来自编辑器自己的 `info!`）。
    pub fn 由条目(条目: &日志条目) -> 日志来源 {
        match &条目.内容 {
            日志内容::结构化 { 标识, .. } => 日志来源::由键(标识),
            日志内容::自由文本(_) => 日志来源::编辑器,
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::state::日志级别;

    #[test]
    fn 键前缀映射到正确来源() {
        assert_eq!(日志来源::由键("log_build_started"), 日志来源::构建);
        assert_eq!(日志来源::由键("log_package_done"), 日志来源::构建);
        assert_eq!(
            日志来源::由键("log_physics_collision_started"),
            日志来源::运行
        );
        assert_eq!(日志来源::由键("log_run_failed"), 日志来源::运行);
        assert_eq!(日志来源::由键("log_import_refreshed"), 日志来源::资源);
        assert_eq!(日志来源::由键("log_asset_missing"), 日志来源::资源);
        assert_eq!(日志来源::由键("log_scene_saved"), 日志来源::编辑器);
        assert_eq!(日志来源::由键("log_project_opened"), 日志来源::编辑器);
    }

    #[test]
    fn 自由文本算编辑器来源() {
        let 条目 = 日志条目 {
            级别: 日志级别::信息,
            内容: 日志内容::自由文本("hello".to_string()),
        };
        assert_eq!(日志来源::由条目(&条目), 日志来源::编辑器);
    }

    #[test]
    fn 结构化条目的来源由标识决定() {
        let 条目 = 日志条目 {
            级别: 日志级别::警告,
            内容: 日志内容::结构化 {
                标识: "log_build_failed",
                参数: Vec::new(),
            },
        };
        assert_eq!(日志来源::由条目(&条目), 日志来源::构建);
    }

    #[test]
    fn 全部来源都有词条键且互不重复() {
        let 键: Vec<&str> = 日志来源::全部().iter().map(|来源| 来源.词条键()).collect();
        let 去重后: std::collections::BTreeSet<&str> = 键.iter().copied().collect();
        assert_eq!(键.len(), 去重后.len(), "词条键不该重复：{键:?}");
        assert_eq!(键.len(), 4);
    }
}
