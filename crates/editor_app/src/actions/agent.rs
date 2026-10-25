//! 智能体域动作（会话管理 / 任务发送与取消 / 上下文附加 / 变更审批 / 自动修复）。
//!
//! 从 `actions/mod.rs` 拆出；分派入口仍是 `super::执行单个动作`。

use super::*;

/// 智能体域动作（会话管理 / 任务发送与取消 / 上下文附加 / 变更审批 / 自动修复）。
///
/// # 为什么这里只管界面状态
///
/// 真正启动/终止 DSH 子进程、推进任务状态由 plugin 的
/// `推进智能体任务` 系统负责；本域只改会话数据、置位"待发送/待取消"，
/// 这样动作分派不会阻塞主线程，也不会和 Bevy 的资源借用冲突。
///
/// 返回 `Err(动作)` 表示这条动作不属于智能体域，原样交回调用者；
/// 总表仍保留这些变体的哨兵分支，穷尽性检查不会因此放宽。
pub(super) fn 处理智能体动作(
    动作: UiAction,
    上下文: &mut 动作上下文<'_>,
) -> Result<(), UiAction> {
    let 编辑器 = &mut *上下文.编辑器;
    let 界面状态 = &mut *上下文.界面状态;

    match 动作 {
        UiAction::智能体新建会话 => {
            let 项目路径 = 编辑器
                .项目
                .as_ref()
                .map(|项目| editor_ui::agent::规范化项目路径(项目.根目录()))
                .unwrap_or_default();
            if 项目路径.is_empty() {
                编辑器.警告("请先打开项目：AI 会话必须绑定到具体项目".to_string());
            } else {
                let 标识 = format!("local-{}", uuid::Uuid::new_v4());
                let 下标 = 界面状态.智能体.新建会话(&项目路径, 标识);
                if let Some(项目) = 编辑器.项目.as_ref()
                    && let Some(会话) = 界面状态.智能体.会话列表.get_mut(下标)
                {
                    会话.设置项目根(项目.根目录());
                }
                界面状态.智能体.待保存 = true;
                界面状态.聚焦面板(editor_ui::Tab::智能体对话);
            }
            Ok(())
        }

        UiAction::智能体选择会话(下标) => {
            if 界面状态.智能体.选择会话(下标) {
                界面状态.智能体.待保存 = true;
            }
            Ok(())
        }

        UiAction::智能体开始重命名 => {
            if let Some(会话) = 界面状态.智能体.当前() {
                界面状态.智能体.重命名草稿 = Some(会话.标题.clone());
            }
            Ok(())
        }

        UiAction::智能体确认重命名(名称) => {
            let 名称 = 名称.trim().to_string();
            if !名称.is_empty()
                && let Some(会话) = 界面状态.智能体.当前可变()
            {
                会话.标题 = 名称;
                界面状态.智能体.待保存 = true;
            }
            界面状态.智能体.重命名草稿 = None;
            Ok(())
        }

        UiAction::智能体取消重命名 => {
            界面状态.智能体.重命名草稿 = None;
            Ok(())
        }

        UiAction::智能体删除会话 => {
            if 界面状态.智能体.运行中 {
                编辑器.警告("任务仍在运行：请先取消任务，再删除会话".to_string());
            } else if 界面状态.智能体.删除当前会话() {
                编辑器.信息("已删除 AI 会话");
            }
            Ok(())
        }

        UiAction::智能体发送 => {
            let 输入 = 界面状态.智能体.输入.trim().to_string();
            if 输入.is_empty() {
                return Ok(());
            }
            if 界面状态.智能体.当前().is_none() {
                编辑器.警告("没有可用会话：请先新建 AI 会话".to_string());
                return Ok(());
            }
            if !界面状态.智能体.连接.可用() {
                编辑器.警告(界面状态.智能体.连接.摘要().to_string());
                return Ok(());
            }
            let 上下文提示 = 界面状态.智能体.上下文提示();
            let 完整指令 = if 上下文提示.is_empty() {
                输入.clone()
            } else {
                format!("{上下文提示}\n\n## 任务\n{输入}")
            };
            // 高风险命令模式：不直接执行，先排队等用户明确批准。
            let 风险 = editor_ui::agent_permission::危险命令检测(&完整指令);
            if !风险.is_empty() {
                界面状态.智能体.待保存 = true;
                let 动作 = editor_ui::agent_permission::待批动作::执行任务(完整指令);
                let 标识 =
                    editor_ui::agent_permission::排队审批(&mut 界面状态.智能体.审批, 动作);
                界面状态.智能体.提示 = Some(format!(
                    "检测到高风险命令，任务尚未执行；请在审查面板批准（{}）",
                    标识
                ));
                编辑器.警告(format!(
                    "任务包含高风险命令模式：{}。已阻止执行，等待批准",
                    风险.join("、")
                ));
                return Ok(());
            }
            发起对话任务(界面状态, 完整指令, &输入);
            Ok(())
        }

        UiAction::智能体取消 => {
            界面状态.智能体.待取消 = true;
            Ok(())
        }

        UiAction::智能体重试 => {
            if 界面状态.智能体.运行中 {
                编辑器.警告("任务仍在运行：请先取消".to_string());
                return Ok(());
            }
            let 指令 = 界面状态
                .智能体
                .当前()
                .and_then(|会话| 会话.当前任务())
                .map(|任务| 任务.指令.clone());
            if let Some(指令) = 指令 {
                发起对话任务(界面状态, 指令.clone(), "重新执行上一次任务");
                // 续用同一个 DSH 会话：把上一次任务的 session id 继承给新任务。
                if let Some(会话) = 界面状态.智能体.当前可变() {
                    let 上一次会话 = 会话.任务.get(1).and_then(|任务| 任务.会话标识.clone());
                    if let Some(标识) = 上一次会话
                        && let Some(任务) = 会话.当前任务可变()
                    {
                        任务.会话标识 = Some(标识);
                    }
                }
            }
            Ok(())
        }

        UiAction::智能体继续 => {
            if 界面状态.智能体.运行中 {
                编辑器.警告("任务仍在运行：请先取消".to_string());
                return Ok(());
            }
            if 界面状态.智能体.当前().is_none() {
                编辑器.警告("没有可用会话：请先新建 AI 会话".to_string());
                return Ok(());
            }
            if !界面状态.智能体.连接.可用() {
                编辑器.警告(界面状态.智能体.连接.摘要().to_string());
                return Ok(());
            }
            发起对话任务(
                界面状态,
                "继续上一次任务，从上次结束的地方继续完成，不要从头重做。".to_string(),
                "继续",
            );
            Ok(())
        }

        UiAction::智能体探测Dsh => {
            界面状态.智能体.待探测 = true;
            Ok(())
        }

        UiAction::智能体自动修复构建 => {
            界面状态.智能体.待修复 = Some(editor_ui::agent_repair::修复请求::新建(
                "构建并修复错误",
                editor_ui::agent_repair::验证方式::构建,
                3,
            ));
            Ok(())
        }

        UiAction::智能体自动修复测试 => {
            界面状态.智能体.待修复 = Some(editor_ui::agent_repair::修复请求::新建(
                "运行测试并修复失败",
                editor_ui::agent_repair::验证方式::测试,
                3,
            ));
            Ok(())
        }

        UiAction::智能体批准变更(路径) => {
            界面状态.智能体.待批准变更 = Some(路径);
            Ok(())
        }

        UiAction::智能体拒绝变更(路径) => {
            界面状态.智能体.待拒绝变更 = Some(路径);
            Ok(())
        }

        UiAction::智能体批准审批(标识) => {
            let 动作 =
                editor_ui::agent_permission::决定审批(&mut 界面状态.智能体.审批, &标识, true);
            match 动作 {
                Some(editor_ui::agent_permission::待批动作::执行任务(指令)) => {
                    发起对话任务(界面状态, 指令.clone(), &指令);
                    编辑器.信息("已批准：任务开始执行".to_string());
                }
                Some(editor_ui::agent_permission::待批动作::回退变更(路径)) => {
                    界面状态.智能体.待拒绝变更 = Some(路径);
                }
                Some(其他) => {
                    编辑器.信息(format!("已批准：{}（该动作由对应面板执行）", 其他.描述()));
                }
                None => {
                    编辑器.警告("审批请求不存在或已经处理过".to_string());
                }
            }
            界面状态.智能体.待保存 = true;
            Ok(())
        }

        UiAction::智能体拒绝审批(标识) => {
            let 动作 =
                editor_ui::agent_permission::决定审批(&mut 界面状态.智能体.审批, &标识, false);
            if 动作.is_none() {
                编辑器.警告("审批请求不存在或已经处理过".to_string());
            } else {
                编辑器.信息("已拒绝：动作不会执行".to_string());
            }
            界面状态.智能体.待保存 = true;
            Ok(())
        }

        UiAction::智能体附加实体上下文 => {
            if let Some(实体) = 编辑器
                .选择
                .主选中()
                .and_then(|标识| 编辑器.会话.文档.取(标识))
            {
                let 正文 = crate::agent_context::实体摘要(实体);
                界面状态.智能体.附加上下文(editor_ui::agent::上下文项 {
                    种类: editor_ui::agent::上下文种类::实体,
                    名称: 实体.名称.clone(),
                    正文,
                });
            } else {
                编辑器.警告("没有选中实体".to_string());
            }
            Ok(())
        }

        UiAction::智能体附加场景上下文 => {
            let 正文 = crate::agent_context::场景摘要(&编辑器.会话);
            let 名称 = 编辑器.会话.文档.元数据.场景名.clone();
            界面状态.智能体.附加上下文(editor_ui::agent::上下文项 {
                种类: editor_ui::agent::上下文种类::场景,
                名称,
                正文,
            });
            Ok(())
        }

        UiAction::智能体附加项目上下文 => {
            match crate::agent_context::项目摘要(编辑器.项目.as_ref()) {
                Ok(正文) => {
                    let 名称 = 编辑器
                        .项目
                        .as_ref()
                        .map(|项目| 项目.名称().to_string())
                        .unwrap_or_else(|| "项目".to_string());
                    界面状态.智能体.附加上下文(editor_ui::agent::上下文项 {
                        种类: editor_ui::agent::上下文种类::项目,
                        名称,
                        正文,
                    });
                }
                Err(错误) => 编辑器.警告(错误),
            }
            Ok(())
        }

        UiAction::智能体移除上下文(下标) => {
            界面状态.智能体.移除上下文(下标);
            Ok(())
        }

        UiAction::智能体附加原生工程上下文 => {
            let 脚本 = 界面状态.脚本.文档.as_ref();
            let 项列表 = crate::agent_context::原生上下文项(编辑器, 脚本, 120);
            if 项列表.is_empty() {
                编辑器.警告("请先打开项目：原生工程上下文需要真实 Cargo 工程".to_string());
            } else {
                let 数量 = 项列表.len();
                for 项 in 项列表 {
                    界面状态.智能体.附加上下文(项);
                }
                编辑器.信息(format!("已附加 {数量} 个原生工程上下文项"));
            }
            Ok(())
        }

        UiAction::智能体刷新变更 => {
            界面状态.智能体.待刷新变更 = true;
            Ok(())
        }

        其他 => Err(其他),
    }
}
