//! AI Agent 的进程侧运行时：探测、启动任务、非阻塞收事件、取消、刷新变更。
//!
//! # 为什么单独成模块
//!
//! 进程句柄不可序列化，也不该进会话文件；界面层（editor_ui）因此完全不碰
//! 标准库进程 API。这里只依赖 `agent_dsh`（协议）与 `agent_context`（上下文）。
//!
//! 每帧工作量与“到达事件数”成正比，主线程不会被 DSH 阻塞。

use bevy::prelude::*;

use crate::plugin::{EditorStateHolder, UiStateHolder};

/// AI Agent 的进程侧状态。
///
/// # 与 `界面状态.智能体` 的分工
///
/// 界面状态只放"用户看得见的数据"（会话、消息、任务记录），进程句柄放在这里：
/// 它不可序列化、也不该写进会话文件；界面层（`editor_ui`）因此不依赖进程 API。
#[derive(bevy::prelude::Resource, Default)]
pub struct 智能体运行 {
    /// 当前任务运行器。
    pub 运行器: Option<crate::agent_dsh::任务运行器>,
    /// 本次任务是否收到了 `final` 事件。
    pub 收到最终结果: bool,
    /// 已完成的验证（构建/测试）次数，用于给文件变更打"已验证"标记。
    pub 验证次数: u64,
}

/// 推进 AI Agent：探测、启动任务、非阻塞收事件、取消、刷新真实文件变更。
///
/// 每帧只做与"到达事件"成正比的工作，绝不阻塞主线程。
pub fn 推进智能体(
    mut 界面: ResMut<UiStateHolder>,
    mut 编辑器: ResMut<EditorStateHolder>,
    mut 运行: ResMut<智能体运行>,
) {
    // ---- 探测 ----
    if 界面.状态.智能体.待探测 {
        界面.状态.智能体.待探测 = false;
        let 结果 = crate::agent_dsh::探测(
            &界面.状态.智能体.可执行文件,
            std::time::Duration::from_secs(8),
        );
        界面.状态.智能体.连接 = 结果.连接;
        if !结果.诊断.trim().is_empty() {
            编辑器.状态.信息(format!(
                "DSH 诊断：{}",
                crate::agent_dsh::截断(&结果.诊断, 200)
            ));
        }
    }

    // ---- 取消 ----
    if 界面.状态.智能体.待取消 {
        界面.状态.智能体.待取消 = false;
        let 已取消 = match 运行.运行器.as_mut() {
            Some(运行器) => 运行器.取消(),
            None => false,
        };
        界面.状态.智能体.结束任务(
            editor_ui::agent::任务状态::已取消,
            if 已取消 {
                "已取消：子进程已终止（副作用是否发生需自行核对改动）".to_string()
            } else {
                "取消时进程已结束".to_string()
            },
        );
        运行.运行器 = None;
    }

    // ---- 启动 ----
    if let Some(指令) = 界面.状态.智能体.待执行.take() {
        let 根 = 编辑器
            .状态
            .项目
            .as_ref()
            .map(|项目| 项目.根目录().to_path_buf());
        match 根 {
            None => 界面.状态.智能体.结束任务(
                editor_ui::agent::任务状态::失败,
                "请先打开项目：Agent 的任务需要真实工作目录".to_string(),
            ),
            Some(根) => {
                let 会话标识 = 界面
                    .状态
                    .智能体
                    .当前()
                    .and_then(|会话| 会话.任务.first())
                    .and_then(|任务| 任务.会话标识.clone())
                    .filter(|标识| !标识.starts_with("local-"));
                match crate::agent_dsh::任务运行器::启动(
                    &界面.状态.智能体.可执行文件,
                    &指令,
                    &根,
                    会话标识.as_deref(),
                ) {
                    Ok(运行器) => {
                        运行.收到最终结果 = false;
                        运行.运行器 = Some(运行器);
                        if let Some(会话) = 界面.状态.智能体.当前可变()
                            && let Some(任务) = 会话.任务.first_mut()
                        {
                            任务.状态 = editor_ui::agent::任务状态::执行中;
                        }
                    }
                    Err(错误) => {
                        界面
                            .状态
                            .智能体
                            .结束任务(editor_ui::agent::任务状态::失败, 错误);
                    }
                }
            }
        }
    }

    // ---- 收事件 ----
    if 运行.运行器.is_some() {
        let 事件列表 = 运行
            .运行器
            .as_mut()
            .map(|运行器| 运行器.轮询())
            .unwrap_or_default();
        for 事件 in 事件列表 {
            match &事件 {
                crate::agent_dsh::Dsh事件::会话 {
                    会话标识: 新标识,
                ..
                } => {
                    if let Some(会话) = 界面.状态.智能体.当前可变() {
                        会话.标识 = 新标识.clone();
                        if let Some(任务) = 会话.任务.first_mut() {
                            任务.会话标识 = Some(新标识.clone());
                        }
                    }
                }
                crate::agent_dsh::Dsh事件::文本 { 文本 } => {
                    // 连续 `text` 事件拼成同一条回复：判据是"上一条是 Agent"。
                    let 追加到上一条 = 界面
                        .状态
                        .智能体
                        .当前()
                        .and_then(|会话| 会话.消息.last())
                        .is_some_and(|最后| 最后.角色 == editor_ui::agent::消息角色::智能体);
                    if !追加到上一条 {
                        界面
                            .状态
                            .智能体
                            .追加消息(editor_ui::agent::消息角色::智能体, 文本.clone());
                    } else if let Some(会话) = 界面.状态.智能体.当前可变()
                        && let Some(最后) = 会话.消息.last_mut()
                    {
                        最后.内容.push_str(文本);
                    }
                }
                crate::agent_dsh::Dsh事件::最终 { 文本 } => {
                    运行.收到最终结果 = true;
                    界面
                        .状态
                        .智能体
                        .追加消息(editor_ui::agent::消息角色::智能体, 文本.clone());
                }
                crate::agent_dsh::Dsh事件::工具 { .. } => {
                    if let Some(调用) = crate::agent_dsh::记录工具调用(&事件)
                        && let Some(会话) = 界面.状态.智能体.当前可变()
                        && let Some(任务) = 会话.任务.first_mut()
                    {
                        任务.工具调用.push(调用);
                    }
                }
                crate::agent_dsh::Dsh事件::读取错误 { 错误 } => {
                    界面.状态.智能体.追加消息(
                        editor_ui::agent::消息角色::系统,
                        format!("读取输出失败：{错误}"),
                    );
                }
                _ => {}
            }
        }

        let 已结束 = 运行
            .运行器
            .as_mut()
            .map(|运行器| !运行器.运行中())
            .unwrap_or(false);
        if 已结束 {
            let 退出码 = 运行.运行器.as_ref().and_then(|运行器| 运行器.退出码());
            运行.运行器 = None;
            let 状态 = crate::agent_dsh::判定状态(运行.收到最终结果, 退出码);
            let 摘要 = match 状态 {
                editor_ui::agent::任务状态::成功 => "任务完成".to_string(),
                editor_ui::agent::任务状态::失败 => format!(
                    "任务失败（退出码 {}）",
                    退出码
                        .map(|码| 码.to_string())
                        .unwrap_or_else(|| "未知".to_string())
                ),
                editor_ui::agent::任务状态::状态未知 => {
                    "进程结束但没有退出码：状态未知".to_string()
                }
                _ => "任务结束".to_string(),
            };
            界面.状态.智能体.结束任务(状态, 摘要);
            界面.状态.智能体.待刷新变更 = true;
        }
    }

    // ---- 刷新文件变更（真实 git 状态）----
    if 界面.状态.智能体.待刷新变更 {
        界面.状态.智能体.待刷新变更 = false;
        if let Some(项目) = 编辑器.状态.项目.as_ref() {
            let 变更 = crate::agent_context::收集变更(项目.根目录(), 运行.验证次数 > 0);
            if let Some(会话) = 界面.状态.智能体.当前可变() {
                会话.变更 = 变更;
            }
        }
    }
}
