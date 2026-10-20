//! 会话存档的应用层接线：启动载入、运行期防抖保存、退出前强制保存。
//!
//! 纯存储逻辑在 [`editor_ui::agent_store`]；这里只负责"什么时候读写"和
//! "项目切换后选哪个会话"。存档路径与编辑器其它配置同目录
//! （`BEVY_STUDIO_CONFIG_DIR` 可覆盖），与会话里的项目路径完全无关。
//!
//! # 为什么需要防抖 + 退出强制保存
//!
//! DSH 的文本事件是逐段到达的，每来一段就写一次盘既浪费又会放大崩溃窗口。
//! 因此运行期最多每 600ms 写一次；任务结束时 `运行中 == false` 立即写；
//! 收到 `AppExit` 时无论是否到间隔都强制写一次。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::prelude::*;

use editor_ui::agent::会话文件路径;
use editor_ui::agent_store::{原子保存, 存档来源, 载入结果};
use editor_ui::state::界面状态;

use crate::plugin::{EditorStateHolder, UiStateHolder};

/// 保存防抖间隔。
const 保存间隔: Duration = Duration::from_millis(600);

/// 会话存档的运行时状态。
#[derive(Resource)]
pub struct 会话存档状态 {
    /// 存档文件路径。
    pub 路径: PathBuf,
    /// 是否已经尝试过载入（避免每帧重复读）。
    pub 已载入: bool,
    /// 禁止覆盖（版本不兼容/读取失败时为真）。
    pub 禁止覆盖: bool,
    /// 载入来源。
    pub 载入来源: Option<存档来源>,
    /// 载入警告（真实文本）。
    pub 警告: Vec<String>,
    /// 最近一次成功保存的时间（UNIX 秒）。
    pub 最近保存: Option<u64>,
    /// 最近一次保存的字节数。
    pub 最近保存字节: usize,
    /// 最近一次保存错误。
    pub 错误: Option<String>,
    /// 上次尝试保存的时刻（防抖用）。
    pub 上次尝试: Option<Instant>,
}

impl Default for 会话存档状态 {
    fn default() -> Self {
        Self {
            路径: 会话文件路径(&crate::config::配置目录()),
            已载入: false,
            禁止覆盖: false,
            载入来源: None,
            警告: Vec::new(),
            最近保存: None,
            最近保存字节: 0,
            错误: None,
            上次尝试: None,
        }
    }
}

/// 启动时载入存档，并选中当前项目最近的会话。
pub fn 载入会话存档(
    mut 界面: ResMut<UiStateHolder>,
    mut 编辑器: ResMut<EditorStateHolder>,
    mut 存档: ResMut<会话存档状态>,
) {
    if 存档.已载入 {
        return;
    }
    存档.已载入 = true;
    // 启动即探测一次 DSH：用户不必先点「检测」才能发送任务。
    界面.状态.智能体.待探测 = true;
    let 结果: 载入结果 = editor_ui::agent_store::载入文件(&存档.路径);
    存档.载入来源 = Some(结果.来源);
    存档.禁止覆盖 = 结果.禁止覆盖;
    存档.警告 = 结果.警告.clone();
    let 数量 = 结果.会话.len();
    界面.状态.智能体.会话列表 = 结果.会话;
    let 项目根 = 编辑器
        .状态
        .项目
        .as_ref()
        .map(|项目| 项目.根目录().to_path_buf());
    let 选中 = editor_ui::agent_store::选择项目会话(&mut 界面.状态.智能体, 项目根.as_deref());
    界面.状态.智能体.载入提示 = if 结果.警告.is_empty() {
        if 数量 > 0 {
            Some(format!("已恢复 {数量} 个历史会话"))
        } else {
            None
        }
    } else {
        Some(结果.警告.join("；"))
    };
    match 结果.来源 {
        存档来源::文件 => 编辑器.状态.信息(format!(
            "已从 {} 恢复 {数量} 个 AI 会话{}",
            存档.路径.display(),
            选中
                .map(|下标| format!("，选中「{}」", 界面.状态.智能体.会话列表[下标].标题))
                .unwrap_or_default()
        )),
        存档来源::文件缺失 => {}
        存档来源::已损坏 => 编辑器
            .状态
            .警告("AI 会话存档损坏：已备份原文件，本次从空列表开始".to_string()),
        存档来源::版本不兼容 => 编辑器
            .状态
            .错误("AI 会话存档版本高于本程序：未载入，也不会覆盖原文件".to_string()),
        存档来源::读取失败 => 编辑器
            .状态
            .错误("AI 会话存档读取失败：本次不会覆盖原文件".to_string()),
    }
    for 警告 in &结果.警告 {
        编辑器.状态.警告(format!("AI 会话：{警告}"));
    }
}

/// 运行期保存（防抖）。
pub fn 保存会话存档(
    mut 界面: ResMut<UiStateHolder>, mut 存档: ResMut<会话存档状态>
) {
    if !界面.状态.智能体.待保存 {
        return;
    }
    if 存档.禁止覆盖 {
        if 存档.错误.is_none() {
            存档.错误 =
                Some("存档版本不兼容或读取失败：已停止自动保存，避免覆盖原文件".to_string());
            界面.状态.智能体.提示 = 存档.错误.clone();
        }
        return;
    }
    // 任务结束后立刻落盘；运行中最多每 600ms 一次。
    let 可以写 = !界面.状态.智能体.运行中
        || 存档
            .上次尝试
            .map(|时刻| 时刻.elapsed() >= 保存间隔)
            .unwrap_or(true);
    if !可以写 {
        return;
    }
    存档.上次尝试 = Some(Instant::now());
    let 会话 = 界面.状态.智能体.会话列表.clone();
    match 原子保存(&存档.路径, &会话) {
        Ok(结果) => {
            界面.状态.智能体.待保存 = false;
            存档.最近保存 = Some(editor_ui::agent::现在秒());
            存档.最近保存字节 = 结果.字节;
            存档.错误 = None;
            if 结果.脱敏数 > 0 {
                界面.状态.智能体.提示 =
                    Some(format!("保存会话时已脱敏 {} 处疑似凭证", 结果.脱敏数));
            }
        }
        Err(错误) => {
            // 保留待保存标记，下一帧继续尝试；错误只记录一次。
            if 存档.错误.as_deref() != Some(错误.as_str()) {
                界面.状态.智能体.提示 = Some(format!("保存会话失败：{错误}"));
            }
            存档.错误 = Some(错误);
        }
    }
}

/// 退出前强制保存（无论防抖间隔）。
pub fn 退出时保存会话(
    mut 退出: MessageReader<AppExit>,
    mut 界面: ResMut<UiStateHolder>,
    mut 存档: ResMut<会话存档状态>,
) {
    if 退出.read().count() == 0 && !界面.状态.智能体.待保存 {
        return;
    }
    if 存档.禁止覆盖 {
        eprintln!(
            "AI 会话未保存：存档版本不兼容或读取失败（{}）",
            存档.路径.display()
        );
        return;
    }
    if !界面.状态.智能体.待保存 {
        return;
    }
    let 会话 = 界面.状态.智能体.会话列表.clone();
    match 原子保存(&存档.路径, &会话) {
        Ok(结果) => {
            界面.状态.智能体.待保存 = false;
            eprintln!(
                "AI 会话已保存：{} 个会话，{} 字节，脱敏 {} 处",
                会话.len(),
                结果.字节,
                结果.脱敏数
            );
        }
        Err(错误) => {
            eprintln!("AI 会话保存失败：{错误}");
            存档.错误 = Some(错误);
        }
    }
}

/// 给动作层用：会话有任何变化就标记待保存。
pub fn 标记待保存(界面: &mut 界面状态) {
    界面.智能体.待保存 = true;
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 默认存档路径在配置目录下() {
        let 状态 = 会话存档状态::default();
        assert_eq!(
            状态.路径.file_name().and_then(|名| 名.to_str()),
            Some(editor_ui::agent_store::存档文件名)
        );
        assert!(!状态.已载入);
        assert!(!状态.禁止覆盖);
    }
}
