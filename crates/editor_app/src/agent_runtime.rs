//! AI Agent 的进程侧运行时：探测、任务启动、非阻塞事件、取消、真实验证、
//! 文件审查动作与自动修复闭环。
//!
//! # 为什么单独成模块
//!
//! 进程句柄不可序列化，也不该进会话文件；界面层（editor_ui）因此完全不碰
//! 标准库进程 API。这里只依赖 `agent_dsh`（协议）、`agent_context`（上下文）、
//! `agent_worktree`（真实文件）与 `build`（真实 cargo）。
//!
//! # 任务归属
//!
//! 任务启动时记录 `运行项目` 与 `运行会话`。之后即使用户切换会话，事件也只
//! 写回**启动它的那个会话**；项目一换就终止进程，并如实标成"状态未知"。
//! 这就是"两个项目不串线"的执行层保证。
//!
//! # 自动修复
//!
//! 真实构建/测试 → 真实诊断 → Agent 修改 → 再真实构建/测试。
//! 默认最多 3 轮，环境问题不重试，成功只认退出码 0。

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use editor_ui::agent::{
    任务状态, 任务种类, 审查状态, 智能体会话, 消息角色, 现在秒, 验证状态
};
use editor_ui::agent_repair::{
    下一步, 修复请求, 修复阶段, 构造修复提示, 诊断摘要, 验证方式
};
use editor_ui::agent_review::应用验证结果;

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
    /// 已完成的验证（构建/测试）次数。
    pub 验证次数: u64,
    /// 任务启动时的项目根（用于项目切换守卫）。
    pub 运行项目: Option<PathBuf>,
    /// 任务所属的会话标识（事件只写回这个会话）。
    pub 运行会话: Option<String>,
    /// 上一次处理过的构建修订号（避免重复处理同一次构建结果）。
    pub 上次构建修订: u64,
    /// 观察到的真实构建开始时间（UNIX 秒）。
    pub 构建开始秒: Option<u64>,
    /// 任务序号（同一秒内多次任务也不会撞 id）。
    pub 任务序号: u64,
}

impl 智能体运行 {
    /// 生成一个真实唯一（进程内）的任务标识。
    pub fn 新任务标识(&mut self) -> String {
        self.任务序号 = self.任务序号.wrapping_add(1);
        format!("task-{}-{}", 现在秒(), self.任务序号)
    }

    /// 清空全部运行态（任务/修复彻底结束时）。
    fn 清理运行态(&mut self) {
        self.运行器 = None;
        self.运行项目 = None;
        self.运行会话 = None;
        self.收到最终结果 = false;
        self.构建开始秒 = None;
    }

    /// 只结束当前子进程，保留项目/会话归属（自动修复进入下一轮验证时用）。
    fn 清理子进程(&mut self) {
        self.运行器 = None;
        self.收到最终结果 = false;
    }
}

/// 推进 AI Agent：探测、启动任务、非阻塞收事件、取消、构建验证、审查动作。
///
/// 每帧只做与"到达事件"成正比的工作，绝不阻塞主线程。
pub fn 推进智能体(
    mut 界面: ResMut<UiStateHolder>,
    mut 编辑器: ResMut<EditorStateHolder>,
    mut 运行: ResMut<智能体运行>,
    mut 构建: ResMut<crate::build::构建宿主>,
) {
    let 界面 = &mut 界面.状态;
    let 编辑器状态 = &mut 编辑器.状态;

    // ---- 项目切换守卫 ----
    处理项目切换(界面, 编辑器状态, &mut 运行, &mut 构建);

    // ---- 探测 ----
    if 界面.智能体.待探测 {
        界面.智能体.待探测 = false;
        let 结果 =
            crate::agent_dsh::探测(&界面.智能体.可执行文件, std::time::Duration::from_secs(8));
        界面.智能体.连接 = 结果.连接;
        if !结果.诊断.trim().is_empty() {
            编辑器状态.信息(format!(
                "DSH 诊断：{}",
                crate::agent_dsh::截断(&结果.诊断, 200)
            ));
        }
    }

    // ---- 取消 ----
    if 界面.智能体.待取消 {
        界面.智能体.待取消 = false;
        let 已终止 = match 运行.运行器.as_mut() {
            Some(运行器) => 运行器.取消(),
            None => false,
        };
        let 构建被停止 = 构建.在运行();
        if 构建被停止 {
            构建.停止();
        }
        let 修复进行中 = 界面.智能体.修复.阶段.进行中();
        if 修复进行中 {
            界面.智能体.修复.取消("用户取消");
        }
        let 摘要 = format!(
            "已取消：{}；构建{}。取消不等于回滚，请核对下方文件差异",
            if 已终止 {
                "子进程已终止"
            } else if 修复进行中 || 构建被停止 {
                "真实构建已停止"
            } else {
                "取消时进程已结束"
            },
            if 构建被停止 {
                "已停止"
            } else {
                "未在运行"
            }
        );
        结束运行任务(界面, &运行, 任务状态::已取消, 摘要);
        运行.清理运行态();
        界面.智能体.待刷新变更 = true;
        界面.智能体.待保存 = true;
    }

    // ---- 自动修复启动 ----
    if let Some(请求) = 界面.智能体.待修复.take() {
        启动自动修复(界面, 编辑器状态, &mut 运行, &mut 构建, 请求);
    }

    // ---- 普通任务启动 ----
    if let Some(指令) = 界面.智能体.待执行.take() {
        启动任务(界面, 编辑器状态, &mut 运行, 指令);
    }

    // ---- 收 DSH 事件 ----
    轮询任务事件(界面, 编辑器状态, &mut 运行, &mut 构建);

    // ---- 构建/测试结果 ----
    处理构建结果(界面, 编辑器状态, &mut 运行, &mut 构建);

    // ---- 审查动作（批准 / 拒绝）----
    处理审查动作(界面, 编辑器状态);

    // ---- 刷新真实文件变更 ----
    if 界面.智能体.待刷新变更 {
        界面.智能体.待刷新变更 = false;
        刷新当前变更(界面, 编辑器状态);
    }
}

/// 当前项目根目录。
fn 项目根(编辑器: &editor_core::state::编辑器状态) -> Option<PathBuf> {
    编辑器.项目.as_ref().map(|项目| 项目.根目录().to_path_buf())
}

/// 项目切换/关闭时终止旧项目的任务与构建。
fn 处理项目切换(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &mut editor_core::state::编辑器状态,
    运行: &mut 智能体运行,
    构建: &mut crate::build::构建宿主,
) {
    let 当前根 = 项目根(编辑器);
    let Some(旧根) = 运行.运行项目.clone() else {
        return;
    };
    if 当前根.as_ref() == Some(&旧根) {
        return;
    }
    if let Some(运行器) = 运行.运行器.as_mut() {
        运行器.取消();
    }
    if 构建.在运行() {
        构建.停止();
    }
    let 摘要 = format!(
        "项目已切换（{} → {}）：任务被终止，状态未知；副作用需核对",
        旧根.display(),
        当前根
            .as_ref()
            .map(|根| 根.display().to_string())
            .unwrap_or_else(|| "未打开项目".to_string())
    );
    结束运行任务(界面, 运行, 任务状态::状态未知, 摘要.clone());
    if 界面.智能体.修复.阶段.进行中() {
        界面.智能体.修复.取消("项目已切换");
    }
    运行.清理运行态();
    界面.智能体.待刷新变更 = false;
    界面.智能体.待保存 = true;
    编辑器.信息(摘要);
}

/// 启动自动修复：先真实构建/测试，失败后再让 Agent 修改。
fn 启动自动修复(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &mut editor_core::state::编辑器状态,
    运行: &mut 智能体运行,
    构建: &mut crate::build::构建宿主,
    请求: 修复请求,
) {
    let Some(根) = 项目根(编辑器) else {
        界面
            .智能体
            .结束任务(任务状态::失败, "请先打开项目：自动修复需要真实工作目录");
        return;
    };
    if 界面.智能体.当前().is_none() {
        界面
            .智能体
            .结束任务(任务状态::失败, "没有可用会话：请先新建 AI 会话");
        return;
    }
    if 界面.智能体.运行中 {
        界面
            .智能体
            .结束任务(任务状态::失败, "已有任务在运行：不能同时启动自动修复");
        return;
    }
    if 构建.在运行() {
        界面.智能体.结束任务(任务状态::失败, "已有构建正在进行");
        return;
    }

    let 任务标识 = 运行.新任务标识();
    // 基线必须在构建之前采集：构建脚本也可能改文件。
    let 基线说明 = match crate::agent_worktree::采集基线(&根, &任务标识) {
        Ok(基线) => {
            if let Some(会话) = 界面.智能体.当前可变() {
                会话.基线 = Some(基线);
            }
            None
        }
        Err(错误) => Some(format!(
            "无法采集 git 基线（{错误}）：本次改动将标为来源未知，拒绝会自动降级为冲突"
        )),
    };
    let 会话标识 = 界面
        .智能体
        .当前()
        .map(|会话| 会话.标识.clone())
        .unwrap_or_default();
    if let Some(会话) = 界面.智能体.当前可变() {
        会话.任务.insert(
            0,
            editor_ui::agent::智能体任务 {
                标识: 任务标识,
                指令: 请求.指令.clone(),
                状态: 任务状态::执行中,
                开始时间: 现在秒(),
                种类: 任务种类::自动修复,
                轮数: 0,
                摘要: format!("自动修复：先执行真实{}", 请求.验证.名称()),
                ..Default::default()
            },
        );
    }
    界面.智能体.输入.clear();
    界面.智能体.修复.开始(&请求);
    界面.智能体.运行中 = true;
    界面.智能体.提示 = Some(界面.智能体.修复.摘要());
    界面.智能体.待保存 = true;
    // 即使还没起 DSH，也要记住归属：项目切换时才能停止这次真实构建。
    运行.运行项目 = Some(根.clone());
    运行.运行会话 = Some(会话标识);

    match 启动真实验证(界面, 编辑器, 构建, 请求.验证) {
        Ok(()) => {
            运行.上次构建修订 = 构建.修订();
            运行.构建开始秒 = Some(现在秒());
            if let Some(说明) = 基线说明 {
                界面.智能体.追加消息(消息角色::系统, 说明);
            }
        }
        Err(错误) => {
            界面.智能体.修复.阶段 = 修复阶段::已失败;
            界面.智能体.修复.结束原因 = Some(错误.clone());
            结束运行任务(界面, 运行, 任务状态::失败, 错误);
            运行.清理运行态();
        }
    }
}

/// 启动一次真实 `cargo build/test`。
fn 启动真实验证(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &editor_core::state::编辑器状态,
    构建: &mut crate::build::构建宿主,
    验证: 验证方式,
) -> Result<(), String> {
    let 根 = 项目根(编辑器).ok_or_else(|| "没有打开项目".to_string())?;
    let mut 配置 = 编辑器
        .项目
        .as_ref()
        .and_then(|项目| 项目.清单().构建.clone())
        .unwrap_or_default();
    配置.子命令 = match 验证 {
        验证方式::构建 => editor_core::build::构建子命令::构建,
        验证方式::测试 => editor_core::build::构建子命令::测试,
    };
    构建.启动(&根, &配置)?;
    界面.智能体.修复.验证 = 验证;
    Ok(())
}

/// 启动一次 DSH headless 任务。
fn 启动任务(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &mut editor_core::state::编辑器状态,
    运行: &mut 智能体运行,
    指令: String,
) {
    let Some(根) = 项目根(编辑器) else {
        界面
            .智能体
            .结束任务(任务状态::失败, "请先打开项目：Agent 的任务需要真实工作目录");
        return;
    };
    let Some(会话) = 界面.智能体.当前() else {
        界面.智能体.结束任务(任务状态::失败, "没有可用会话");
        return;
    };
    let 会话标识键 = 会话.标识.clone();
    let 任务种类 = 会话
        .当前任务()
        .map(|任务| 任务.种类)
        .unwrap_or(任务种类::对话);
    let 任务标识 = 会话
        .当前任务()
        .map(|任务| 任务.标识.clone())
        .unwrap_or_default();

    // 普通任务：以本次任务为基线；自动修复：复用修复开始时的基线。
    if 任务种类 != 任务种类::自动修复 {
        let 基线说明 = match crate::agent_worktree::采集基线(&根, &任务标识) {
            Ok(基线) => {
                if let Some(会话) = 界面.智能体.当前可变() {
                    会话.基线 = Some(基线);
                }
                None
            }
            Err(错误) => Some(format!(
                "无法采集 git 基线（{错误}）：本次改动将标为来源未知"
            )),
        };
        if let Some(说明) = 基线说明 {
            界面.智能体.追加消息(消息角色::系统, 说明);
        }
    }

    // 续用 DSH 自己的 Session：优先用最近任务里的真实 session id。
    let 续用 = 界面
        .智能体
        .当前()
        .and_then(|会话| 会话.当前任务())
        .and_then(|任务| 任务.会话标识.clone())
        .or_else(|| {
            let 标识 = 界面.智能体.当前().map(|会话| 会话.标识.clone())?;
            if 标识.starts_with("session-") {
                Some(标识)
            } else {
                None
            }
        })
        .filter(|标识| 标识.starts_with("session-"));

    match crate::agent_dsh::任务运行器::启动(
        &界面.智能体.可执行文件,
        &指令,
        &根,
        续用.as_deref(),
    ) {
        Ok(运行器) => {
            运行.运行器 = Some(运行器);
            运行.收到最终结果 = false;
            运行.运行项目 = Some(根.clone());
            运行.运行会话 = Some(会话标识键);
            运行.构建开始秒 = None;
            界面.智能体.运行中 = true;
            界面.智能体.提示 = Some(format!(
                "任务已启动（{}）",
                if 续用.is_some() {
                    "续用 DSH 会话"
                } else {
                    "新 DSH 会话"
                }
            ));
            if let Some(会话) = 界面.智能体.当前可变()
                && let Some(任务) = 会话.当前任务可变()
            {
                任务.状态 = 任务状态::执行中;
                任务.开始时间 = 现在秒();
            }
            界面.智能体.待保存 = true;
        }
        Err(错误) => {
            if 界面.智能体.修复.阶段 == 修复阶段::等待Agent {
                界面.智能体.修复.阶段 = 修复阶段::环境阻塞;
                界面.智能体.修复.结束原因 = Some(format!("DSH 启动失败：{错误}"));
            }
            界面.智能体.运行中 = false;
            结束运行任务(界面, 运行, 任务状态::失败, 错误);
            运行.清理运行态();
        }
    }
}

/// 收事件并写回**启动它的那个会话**。
fn 轮询任务事件(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &mut editor_core::state::编辑器状态,
    运行: &mut 智能体运行,
    构建: &mut crate::build::构建宿主,
) {
    if 运行.运行器.is_none() {
        return;
    }
    let 事件列表 = 运行
        .运行器
        .as_mut()
        .map(|运行器| 运行器.轮询())
        .unwrap_or_default();
    let mut 会话键 = 运行.运行会话.clone();
    for 事件 in 事件列表 {
        match &事件 {
            crate::agent_dsh::Dsh事件::会话 {
                会话标识: 新标识,
            ..
            } => {
                if let Some(会话) = 按标识可变(界面, 会话键.as_deref()) {
                    会话.标识 = 新标识.clone();
                    if let Some(任务) = 会话.当前任务可变() {
                        任务.会话标识 = Some(新标识.clone());
                    }
                }
                // 后续事件必须按新标识找到同一个会话：DSH 在首个事件里
                // 才把本地 id 换成真实 session id。
                会话键 = Some(新标识.clone());
                运行.运行会话 = Some(新标识.clone());
                界面.智能体.待保存 = true;
            }
            crate::agent_dsh::Dsh事件::文本 { 文本 } => {
                追加智能体文本(界面, 会话键.as_deref(), 文本);
                界面.智能体.待保存 = true;
            }
            crate::agent_dsh::Dsh事件::最终 { 文本 } => {
                运行.收到最终结果 = true;
                // DSH 先逐段发 `text`，最后再发一次完整 `final`。如果最后一条
                // Agent 消息已经包含这段文本，就不重复追加。
                let 已包含 = 按标识可变(界面, 会话键.as_deref())
                    .and_then(|会话| 会话.消息.last())
                    .is_some_and(|最后| {
                        最后.角色 == 消息角色::智能体
                            && (最后.内容.ends_with(文本) || 最后.内容 == *文本)
                    });
                if !已包含 {
                    追加智能体文本(界面, 会话键.as_deref(), 文本);
                }
                界面.智能体.待保存 = true;
            }
            crate::agent_dsh::Dsh事件::工具 { .. } => {
                if let Some(调用) = crate::agent_dsh::记录工具调用(&事件) {
                    let 写路径 = if 调用.写操作 {
                        crate::agent_dsh::提取写路径(&调用.摘要, 运行.运行项目.as_deref())
                    } else {
                        None
                    };
                    if let Some(会话) = 按标识可变(界面, 会话键.as_deref())
                        && let Some(任务) = 会话.当前任务可变()
                    {
                        if let Some(路径) = 写路径 {
                            任务.记录写路径(&路径);
                        }
                        任务.工具调用.push(调用);
                    }
                    界面.智能体.待保存 = true;
                }
            }
            crate::agent_dsh::Dsh事件::读取错误 { 错误 } => {
                if let Some(会话) = 按标识可变(界面, 会话键.as_deref()) {
                    会话.追加消息(消息角色::系统, format!("读取输出失败：{错误}"));
                }
            }
            _ => {}
        }
    }

    let 已结束 = 运行
        .运行器
        .as_mut()
        .map(|运行器| !运行器.运行中())
        .unwrap_or(false);
    if !已结束 {
        return;
    }
    let 退出码 = 运行.运行器.as_ref().and_then(|运行器| 运行器.退出码());
    let 状态 = crate::agent_dsh::判定状态(运行.收到最终结果, 退出码);
    let 摘要 = match 状态 {
        任务状态::成功 => "任务完成（退出码 0）".to_string(),
        任务状态::失败 => format!(
            "任务失败（退出码 {}）",
            退出码
                .map(|码| 码.to_string())
                .unwrap_or_else(|| "未知".to_string())
        ),
        任务状态::状态未知 => "进程结束但没有退出码：状态未知".to_string(),
        _ => "任务结束".to_string(),
    };
    if let Some(会话) = 按标识可变(界面, 会话键.as_deref())
        && let Some(任务) = 会话.当前任务可变()
    {
        任务.退出码 = 退出码;
    }
    结束运行任务(界面, 运行, 状态, 摘要.clone());
    let 修复进行中 = 界面.智能体.修复.阶段 == 修复阶段::等待Agent;
    let 可继续修复 = 修复进行中 && matches!(状态, 任务状态::成功 | 任务状态::失败);
    if 修复进行中 {
        if 可继续修复 {
            界面.智能体.修复.智能体完成(&摘要);
        } else {
            界面.智能体.修复.取消("Agent 任务未正常结束");
        }
    }
    if 可继续修复 {
        运行.清理子进程();
    } else {
        运行.清理运行态();
    }
    界面.智能体.待刷新变更 = true;
    界面.智能体.待保存 = true;

    // 自动修复：Agent 改完后立刻做下一轮真实验证。
    if 可继续修复 {
        let 验证 = 界面.智能体.修复.验证;
        match 启动真实验证(界面, 编辑器, 构建, 验证) {
            Ok(()) => {
                运行.上次构建修订 = 构建.修订();
                运行.构建开始秒 = Some(现在秒());
                界面.智能体.提示 = Some(界面.智能体.修复.摘要());
            }
            Err(错误) => {
                界面.智能体.修复.阶段 = 修复阶段::已失败;
                界面.智能体.修复.结束原因 = Some(format!("无法启动验证：{错误}"));
                界面
                    .智能体
                    .追加消息(消息角色::系统, format!("无法启动验证：{错误}"));
            }
        }
    }
}

fn 追加智能体文本(
    界面: &mut editor_ui::state::界面状态, 会话键: Option<&str>, 文本: &str
) {
    let Some(会话) = 按标识可变(界面, 会话键) else {
        return;
    };
    let 追加到上一条 = 会话
        .消息
        .last()
        .is_some_and(|最后| 最后.角色 == 消息角色::智能体);
    if 追加到上一条 {
        if let Some(最后) = 会话.消息.last_mut() {
            最后.内容.push_str(文本);
        }
    } else {
        会话.追加消息(消息角色::智能体, 文本);
    }
}

fn 按标识可变<'a>(
    界面: &'a mut editor_ui::state::界面状态,
    标识: Option<&str>,
) -> Option<&'a mut 智能体会话> {
    match 标识 {
        Some(标识) => 界面
            .智能体
            .会话列表
            .iter_mut()
            .find(|会话| 会话.标识 == 标识),
        None => 界面.智能体.当前可变(),
    }
}

/// 把界面选中项定位到任务启动时记录的会话（自动修复跨轮次时用）。
fn 定位运行会话(界面: &mut editor_ui::state::界面状态, 运行: &智能体运行) {
    let Some(标识) = 运行.运行会话.as_deref() else {
        return;
    };
    if let Some(下标) = 界面
        .智能体
        .会话列表
        .iter()
        .position(|会话| 会话.标识 == 标识)
    {
        界面.智能体.当前会话 = Some(下标);
    }
}

/// 结束"启动它的那个会话"里的当前任务。
fn 结束运行任务(
    界面: &mut editor_ui::state::界面状态,
    运行: &智能体运行,
    状态: 任务状态,
    摘要: impl Into<String>,
) {
    let 摘要 = 摘要.into();
    界面.智能体.运行中 = false;
    let 会话 = match 运行.运行会话.as_deref() {
        Some(标识) => 界面
            .智能体
            .会话列表
            .iter_mut()
            .find(|会话| 会话.标识 == 标识),
        None => 界面.智能体.当前可变(),
    };
    if let Some(会话) = 会话
        && let Some(任务) = 会话.当前任务可变()
    {
        任务.状态 = 状态;
        任务.摘要 = 摘要.clone();
        任务.结束时间 = Some(现在秒());
        if matches!(状态, 任务状态::失败 | 任务状态::已取消 | 任务状态::状态未知)
        {
            会话.追加消息(消息角色::系统, 摘要.clone());
        }
    }
    if !摘要.is_empty() {
        界面.智能体.提示 = Some(摘要);
    }
    界面.智能体.待保存 = true;
}

/// 处理真实构建/测试结束（自动修复闭环的验证步）。
fn 处理构建结果(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &mut editor_core::state::编辑器状态,
    运行: &mut 智能体运行,
    构建: &mut crate::build::构建宿主,
) {
    // 记录真实构建开始时间：用于判断"验证是否发生在修改之后"。
    if 构建.在运行() {
        if 运行.构建开始秒.is_none() {
            运行.构建开始秒 = Some(现在秒());
        }
        return;
    }
    let 修订 = 构建.修订();
    if 修订 == 运行.上次构建修订 {
        return;
    }
    let 修复阶段 = 界面.智能体.修复.阶段;
    if 修复阶段 != 修复阶段::等待验证 {
        // 不是自动修复：只把"在修改之后真实跑过的成功构建"写进验证状态。
        if let Some(根) = 项目根(编辑器)
            && let Some(开始) = 运行.构建开始秒
            && 构建.退出码() == Some(0)
        {
            let 验证 = if 构建.报告().命令行.starts_with("cargo test") {
                验证状态::测试通过
            } else {
                验证状态::构建通过
            };
            标记变更已验证(界面, &根, 验证, 开始);
            运行.验证次数 += 1;
        }
        运行.上次构建修订 = 修订;
        运行.构建开始秒 = None;
        return;
    }

    // 自动修复：消费这一次真实构建结果。
    // 先定位到发起修复的会话：用户可能在等待构建时切换了会话，
    // 但修复结果必须写回原来的会话，不能串线。
    定位运行会话(界面, 运行);
    运行.上次构建修订 = 修订;
    let 退出码 = 构建.退出码();
    let 根 = 项目根(编辑器);
    let (诊断, 变更路径) = match 根.as_deref() {
        Some(根) => (
            构建诊断文本(构建, 根),
            界面
                .智能体
                .当前()
                .map(|会话| 会话.变更.iter().map(|项| 项.路径.clone()).collect())
                .unwrap_or_default(),
        ),
        None => ("没有打开项目".to_string(), Vec::new()),
    };
    let 首次 = 界面.智能体.修复.记录.is_empty();
    let 决策 = if 首次 {
        界面.智能体.修复.首次验证结束(&诊断, 退出码, 变更路径)
    } else {
        界面.智能体.修复.后续验证结束(&诊断, 退出码, 变更路径)
    };

    if let Some(会话) = 界面.智能体.当前可变()
        && let Some(任务) = 会话.当前任务可变()
    {
        任务.已运行验证 = true;
    }
    if 退出码 == Some(0) {
        运行.验证次数 += 1;
        let 验证 = if 界面.智能体.修复.验证 == 验证方式::测试 {
            验证状态::测试通过
        } else {
            验证状态::构建通过
        };
        if let Some(根) = 根.as_deref() {
            标记变更已验证(界面, 根, 验证, 现在秒());
        }
    }

    let 提示 = 界面.智能体.修复.摘要();
    界面.智能体.提示 = Some(提示.clone());
    match 决策 {
        下一步::完成 => {
            结束运行任务(界面, 运行, 任务状态::成功, 提示.clone());
            界面.智能体.追加消息(
                消息角色::系统,
                format!(
                    "自动修复成功：{} 退出码 0（{} 轮真实验证）",
                    界面.智能体.修复.验证.名称(),
                    界面.智能体.修复.记录.len()
                ),
            );
            界面.智能体.修复.阶段 = 修复阶段::已成功;
            运行.清理运行态();
        }
        下一步::环境阻塞(类别) => {
            结束运行任务(
                界面,
                运行,
                任务状态::失败,
                format!("环境问题（{类别:?}）：已停止自动修复"),
            );
            界面.智能体.追加消息(
                消息角色::系统,
                format!(
                    "环境问题：{:?}。这不是代码错误，已停止自动修复（不无限重试）。\n{}",
                    类别,
                    诊断摘要(&诊断, 12)
                ),
            );
            运行.清理运行态();
        }
        下一步::达到上限 => {
            结束运行任务(
                界面,
                运行,
                任务状态::失败,
                "已达到最大自动修复轮数或失败不可自动修复".to_string(),
            );
            界面.智能体.追加消息(
                消息角色::系统,
                format!(
                    "自动修复结束：{}\n最后诊断：\n{}",
                    界面.智能体.修复.摘要(),
                    诊断摘要(&诊断, 12)
                ),
            );
            运行.清理运行态();
        }
        下一步::启动Agent(轮数) => {
            if let Some(会话) = 界面.智能体.当前可变()
                && let Some(任务) = 会话.当前任务可变()
            {
                任务.轮数 = 轮数;
                任务.诊断输入 = 诊断.clone();
            }
            let 指令 = 构造修复提示(
                &界面.智能体.修复.指令,
                界面.智能体.修复.验证,
                &诊断,
                轮数,
                界面.智能体.修复.上限,
            );
            界面.智能体.追加消息(
                消息角色::系统,
                format!(
                    "第 {轮数} 轮：{}失败（退出码 {:?}，{:?}），把诊断交给 Agent",
                    界面.智能体.修复.验证.名称(),
                    退出码,
                    界面.智能体.修复.记录.last().map(|记录| 记录.错误类别)
                ),
            );
            界面.智能体.待执行 = Some(指令);
            界面.智能体.待保存 = true;
        }
        下一步::无动作 => {}
    }
    运行.构建开始秒 = None;
    界面.智能体.待保存 = true;
}

/// 把真实验证结果写到变更列表（指纹一致才算）。
fn 标记变更已验证(
    界面: &mut editor_ui::state::界面状态,
    根: &Path,
    验证: 验证状态,
    验证开始秒: u64,
) {
    let Some(会话) = 界面.智能体.当前可变() else {
        return;
    };
    // 构建开始早于变更记录的，不算"修改之后的验证"。
    if 会话.变更.iter().any(|项| 项.记录时间 > 验证开始秒) {
        return;
    }
    let 指纹 = crate::agent_worktree::指纹表(根, &会话.变更);
    应用验证结果(&mut 会话.变更, 验证, &指纹);
    界面.智能体.待保存 = true;
}

/// 从真实构建报告拼出给 Agent 的诊断文本。
fn 构建诊断文本(构建: &crate::build::构建宿主, 根: &Path) -> String {
    let 报告 = 构建.报告();
    let mut 结果 = String::new();
    for 诊断 in 报告
        .诊断
        .iter()
        .filter(|诊断| {
            matches!(
                诊断.级别,
                editor_core::build::诊断级别::错误 | editor_core::build::诊断级别::警告
            )
        })
        .take(80)
    {
        let 位置 = 诊断
            .位置
            .as_ref()
            .map(|位置| {
                let 文件 = 位置
                    .文件
                    .as_ref()
                    .map(|文件| {
                        文件
                            .strip_prefix(根)
                            .map(|路径| 路径.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_else(|_| 文件.to_string_lossy().to_string())
                    })
                    .unwrap_or_else(|| "?".to_string());
                format!("{文件}:{}:{}: ", 位置.行, 位置.列)
            })
            .unwrap_or_default();
        let 级别 = match 诊断.级别 {
            editor_core::build::诊断级别::错误 => "error",
            editor_core::build::诊断级别::警告 => "warning",
            _ => "note",
        };
        结果.push_str(&format!("{位置}{级别} {}\n", 诊断.单行文本()));
    }
    if 结果.trim().is_empty() {
        let 尾部 = 构建.标准错误尾部();
        if !尾部.trim().is_empty() {
            结果 = 尾部;
        } else if let Some(摘要) = 报告.摘要文本() {
            结果 = 摘要.to_string();
        } else {
            结果 = format!("构建失败但没有可读诊断（退出码 {:?}）", 构建.退出码());
        }
    }
    editor_ui::agent_store::截断文本(&结果, 8000)
}

/// 处理"批准 / 拒绝"真实文件变更。
fn 处理审查动作(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &editor_core::state::编辑器状态,
) {
    if let Some(路径) = 界面.智能体.待批准变更.take() {
        let Some(根) = 项目根(编辑器) else {
            return;
        };
        if let Some(会话) = 界面.智能体.当前可变()
            && let Some(项) = 会话.变更.iter_mut().find(|项| 项.路径 == 路径)
        {
            项.审查 = 审查状态::已批准;
            项.当前指纹 = crate::agent_worktree::文件指纹(&根, &项.路径);
            项.备注 = Some(format!(
                "用户于 {} 批准；保留工作区现状",
                editor_ui::agent::现在秒()
            ));
        }
        界面.智能体.待保存 = true;
        界面.智能体.提示 = Some(format!("已批准：{路径}"));
    }

    if let Some(路径) = 界面.智能体.待拒绝变更.take() {
        let Some(根) = 项目根(编辑器) else {
            return;
        };
        let 变更 = 界面
            .智能体
            .当前()
            .and_then(|会话| 会话.变更.iter().find(|项| 项.路径 == 路径).cloned());
        let Some(变更) = 变更 else {
            return;
        };
        let 结果 = crate::agent_worktree::回退变更(&根, &变更);
        let (状态, 说明) = match 结果 {
            Ok(结果) => {
                界面.智能体.提示 = Some(format!("已拒绝并回退：{}", 结果.路径));
                (审查状态::已拒绝, 结果.说明)
            }
            Err(错误) => {
                let 状态 = if 错误.contains("又被修改") {
                    审查状态::冲突
                } else {
                    审查状态::无法撤销
                };
                界面.智能体.提示 = Some(format!("拒绝失败（{状态:?}）：{错误}"));
                (状态, 错误)
            }
        };
        if let Some(会话) = 界面.智能体.当前可变()
            && let Some(项) = 会话.变更.iter_mut().find(|项| 项.路径 == 路径)
        {
            项.审查 = 状态;
            项.备注 = Some(说明);
            项.当前指纹 = crate::agent_worktree::文件指纹(&根, &路径);
        }
        界面.智能体.待保存 = true;
    }
}

/// 刷新当前会话的真实文件变更（带基线归属）。
fn 刷新当前变更(
    界面: &mut editor_ui::state::界面状态,
    编辑器: &editor_core::state::编辑器状态,
) {
    let Some(根) = 项目根(编辑器) else {
        return;
    };
    let Some(会话) = 界面.智能体.当前() else {
        return;
    };
    let 基线 = 会话.基线.clone();
    let 写路径 = 会话
        .当前任务()
        .map(|任务| 任务.修改文件.clone())
        .unwrap_or_default();
    let 任务标识 = 会话
        .当前任务()
        .map(|任务| 任务.标识.clone())
        .unwrap_or_default();
    let 旧 = 会话.变更.clone();
    match crate::agent_worktree::刷新变更(&根, 基线.as_ref(), &写路径, &任务标识, &旧)
    {
        Ok(变更) => {
            if let Some(会话) = 界面.智能体.当前可变() {
                会话.变更 = 变更;
            }
            界面.智能体.待保存 = true;
        }
        Err(错误) => {
            界面.智能体.提示 = Some(format!("读取文件变更失败：{错误}"));
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_ui::agent::{变更来源, 文件变更};

    #[test]
    fn 运行态清理不会留下旧项目与会话() {
        let mut 运行 = 智能体运行 {
            运行项目: Some(PathBuf::from("C:/a")),
            运行会话: Some("s1".to_string()),
            收到最终结果: true,
            ..Default::default()
        };
        运行.清理运行态();
        assert!(运行.运行项目.is_none());
        assert!(运行.运行会话.is_none());
        assert!(!运行.收到最终结果);
    }

    #[test]
    fn 只清理子进程时保留归属() {
        let mut 运行 = 智能体运行 {
            运行项目: Some(PathBuf::from("C:/a")),
            运行会话: Some("s1".to_string()),
            ..Default::default()
        };
        运行.清理子进程();
        assert_eq!(运行.运行项目, Some(PathBuf::from("C:/a")));
        assert_eq!(运行.运行会话.as_deref(), Some("s1"));
    }

    #[test]
    fn 任务标识在一次运行内唯一() {
        let mut 运行 = 智能体运行::default();
        let a = 运行.新任务标识();
        let b = 运行.新任务标识();
        assert_ne!(a, b);
        assert!(a.starts_with("task-"));
    }

    #[test]
    fn 变更验证只认修改之后的构建() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        std::fs::write(根.join("a.rs"), "fn main() {}\n").unwrap();
        let mut 界面 = editor_ui::state::界面状态::新建();
        界面.智能体.新建会话("c:/p", "s1");
        let 指纹 = crate::agent_worktree::文件指纹(根, "a.rs");
        界面.智能体.当前可变().unwrap().变更 = vec![文件变更 {
            路径: "a.rs".to_string(),
            当前指纹: 指纹,
            记录时间: 200,
            来源: 变更来源::本任务,
            ..Default::default()
        }];
        // 构建开始早于变更记录 → 不算验证
        标记变更已验证(&mut 界面, 根, 验证状态::构建通过, 100);
        assert_eq!(界面.智能体.当前().unwrap().变更[0].验证, 验证状态::未验证);
        // 构建开始晚于变更记录 → 真实验证
        标记变更已验证(&mut 界面, 根, 验证状态::构建通过, 300);
        assert_eq!(界面.智能体.当前().unwrap().变更[0].验证, 验证状态::构建通过);
    }
}
