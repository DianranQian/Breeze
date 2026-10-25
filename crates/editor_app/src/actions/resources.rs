//! 资源域动作（重命名 / 删除 / 引用查找 / 导入检查）。
//!
//! 从 `actions.rs` 拆出；分派入口仍是 `super::执行单个动作`。

use super::*;

pub(super) fn 处理资源动作(
    动作: UiAction,
    上下文: &mut 动作上下文<'_>,
) -> Result<(), UiAction> {
    let 编辑器 = &mut *上下文.编辑器;
    let 界面状态 = &mut *上下文.界面状态;
    let 导入 = &mut *上下文.导入;

    match 动作 {
        UiAction::请求重命名资源(路径) => {
            // 打开对话框时预填当前路径：用户多数只想改文件名，
            // 让他从完整路径开始删改，比让他从头输入更少出错。
            界面状态.资源重命名 = Some(editor_ui::state::资源重命名草稿 {
                旧路径: 路径.clone(),
                新路径: 路径.文本().to_string(),
                错误: None,
            });
            Ok(())
        }

        UiAction::确认重命名资源 {
            旧路径, 新路径
        } => {
            match asset_ops::重命名资源(编辑器, &旧路径, &新路径) {
                Ok(报告) => {
                    // 成功才关对话框：失败时用户还要改输入。
                    界面状态.资源重命名 = None;
                    导入.需刷新 = true;
                    // 选中的资源跟着改名走：否则预览会继续引用旧路径，
                    // 下一帧就变成"文件不存在"，看起来像预览坏了。
                    if 界面状态.资源选中.as_ref() == Some(&旧路径) {
                        界面状态.选中资源(新路径.clone());
                    }
                    let 语言 = 界面状态.语言;
                    let 摘要 = 报告.摘要();
                    if 报告.当前文档引用数 + 报告.其它场景引用数 == 0 {
                        编辑器.结构化信息("log_asset_rename_no_refs", Vec::new());
                    }
                    编辑器.结构化信息(
                        "log_asset_rename_done",
                        vec![
                            日志参数::用户数据(旧路径.文本().to_string()),
                            日志参数::用户数据(新路径.文本().to_string()),
                            日志参数::用户数据(摘要),
                        ],
                    );
                    let _ = 语言;
                }
                Err(错误) => {
                    if let Some(草稿) = 界面状态.资源重命名.as_mut() {
                        草稿.错误 = Some(错误.clone());
                    }
                    编辑器.错误(错误);
                }
            }
            Ok(())
        }

        UiAction::关闭重命名资源 => {
            界面状态.资源重命名 = None;
            Ok(())
        }

        UiAction::请求删除资源(路径) => {
            // 打开时就用真实索引算一遍引用：对话框打开期间项目可能被其它
            // 编辑器/工具改动，因此**确认时动作层还会再算一次**，
            // 不能把这里的结论当成授权。
            let (引用, 阻止原因) = match (编辑器.项目.as_ref(), 路径.文本()) {
                (Some(项目), _) => {
                    let 索引 = editor_core::asset_index::项目资源索引::构建(
                        项目,
                        编辑器.会话.路径.as_ref(),
                        Some(&编辑器.会话.文档),
                    );
                    let 引用 = 索引
                        .引用位置(&路径)
                        .iter()
                        .map(|项| {
                            format!(
                                "{} / {} / {}.{}",
                                项.场景.文本(),
                                if 项.实体名称.is_empty() {
                                    "(已删除实体)"
                                } else {
                                    &项.实体名称
                                },
                                项.位置.组件,
                                项.位置.属性路径
                            )
                        })
                        .collect();
                    (引用, 索引.阻止删除原因(&路径))
                }
                (None, _) => (Vec::new(), Some("没有打开的项目".to_string())),
            };
            界面状态.资源删除 = Some(editor_ui::state::资源删除草稿 {
                路径,
                引用,
                阻止原因,
                错误: None,
            });
            Ok(())
        }

        UiAction::确认删除资源(路径) => {
            match asset_ops::删除资源(编辑器, &路径) {
                Ok(_) => {
                    界面状态.资源删除 = None;
                    导入.需刷新 = true;
                    // 删掉的正好是当前预览目标：清空选择，而不是让面板
                    // 继续显示一个已经不存在的文件。
                    if 界面状态.资源选中.as_ref() == Some(&路径) {
                        界面状态.清空资源预览();
                    }
                    let 语言 = 界面状态.语言;
                    编辑器.结构化信息(
                        "log_asset_deleted",
                        vec![
                            日志参数::用户数据(路径.文本().to_string()),
                            日志参数::用户数据(语言.取("assets").to_string()),
                        ],
                    );
                }
                Err(错误) => {
                    if let Some(草稿) = 界面状态.资源删除.as_mut() {
                        草稿.阻止原因 = Some(错误.clone());
                        草稿.错误 = Some(错误.clone());
                    }
                    编辑器.结构化警告(
                        "log_asset_delete_blocked",
                        vec![
                            日志参数::用户数据(路径.文本().to_string()),
                            日志参数::用户数据(错误),
                        ],
                    );
                }
            }
            Ok(())
        }

        UiAction::关闭删除资源 => {
            界面状态.资源删除 = None;
            Ok(())
        }

        UiAction::请求查找引用(资源) => {
            let Some(项目) = 编辑器.项目.clone() else {
                编辑器.警告(界面状态.语言.取("build_no_project_hint").to_string());
                return Ok(());
            };
            let 索引 = editor_core::asset_index::项目资源索引::构建(
                &项目,
                编辑器.会话.路径.as_ref(),
                Some(&编辑器.会话.文档),
            );
            let 行 = 索引
                .引用位置(&资源)
                .iter()
                .map(|项| editor_ui::state::查找引用行 {
                    场景: 项.场景.clone(),
                    实体: 项.位置.实体,
                    实体名称: 项.实体名称.clone(),
                    组件: 项.位置.组件.clone(),
                    属性路径: 项.位置.属性路径.clone(),
                })
                .collect();
            let 问题: Vec<String> = 索引
                .无法读取的场景
                .iter()
                .map(|(场景, 原因)| {
                    界面状态.语言.填充(
                        "asset_find_refs_scene_unreadable",
                        &[&format!("{}（{原因}）", 场景.文本())],
                    )
                })
                .collect();
            界面状态.查找引用 = Some(editor_ui::state::查找引用草稿 {
                资源, 行, 问题
            });
            Ok(())
        }

        UiAction::打开引用位置 { 场景, 实体 } => {
            // 跨场景跳转会丢弃当前未保存修改：必须拒绝并说清，
            // 而不是"帮用户做了决定"。
            let 当前 = 编辑器.会话.路径.clone();
            if 当前.as_ref() != Some(&场景) && 编辑器.有未保存修改() {
                编辑器.警告(界面状态.语言.取("asset_find_refs_unsaved").to_string());
                return Ok(());
            }
            if 当前.as_ref() != Some(&场景) {
                match 编辑器.打开场景(&场景) {
                    Ok(()) => {}
                    Err(错误) => {
                        编辑器.错误(format!("打开场景失败：{错误}"));
                        return Ok(());
                    }
                }
                // 打开场景会清空选择；跳转的意义就是选中那一行对应的实体。
                编辑器.选择.单选(实体);
                编辑器.信息(format!("已跳转到 {}", 场景.文本()));
            } else {
                编辑器.选择.单选(实体);
            }
            界面状态.查找引用 = None;
            Ok(())
        }

        UiAction::关闭查找引用 => {
            界面状态.查找引用 = None;
            Ok(())
        }

        UiAction::检查资源使用 => {
            let Some(项目) = 编辑器.项目.clone() else {
                编辑器.警告(界面状态.语言.取("build_no_project_hint").to_string());
                return Ok(());
            };
            let 索引 = editor_core::asset_index::项目资源索引::构建(
                &项目,
                编辑器.会话.路径.as_ref(),
                Some(&编辑器.会话.文档),
            );
            let 未用 = 索引.未被引用的资源(&项目);
            let 缺失 = 索引.缺失的资源(&项目);

            if !索引.无法读取的场景.is_empty() {
                for (场景, 原因) in &索引.无法读取的场景 {
                    编辑器.结构化警告(
                        "log_dependency_missing",
                        vec![
                            日志参数::用户数据(场景.文本().to_string()),
                            日志参数::用户数据(原因.clone()),
                        ],
                    );
                }
                编辑器.信息("有场景无法读取，资源使用报告可能不完整".to_string());
            }
            for 路径 in &未用 {
                编辑器.结构化信息(
                    "asset_usage_unused",
                    vec![日志参数::用户数据(路径.文本().to_string())],
                );
            }
            for 路径 in &缺失 {
                编辑器.结构化警告(
                    "asset_usage_missing",
                    vec![日志参数::用户数据(路径.文本().to_string())],
                );
            }
            编辑器.结构化信息(
                "asset_usage_checked",
                vec![
                    日志参数::用户数据(未用.len().to_string()),
                    日志参数::用户数据(缺失.len().to_string()),
                ],
            );
            Ok(())
        }

        UiAction::重新导入资源 => {
            let Some(项目) = 编辑器.项目.as_ref() else {
                编辑器.警告(界面状态.语言.取("build_no_project_hint").to_string());
                return Ok(());
            };
            let mut 清单 = 项目.读取导入清单();
            let 报告 = 项目.扫描导入(&清单);
            let 待处理 = 报告.项.iter().filter(|项| 项.状态.需要处理()).count();
            项目.更新导入清单(&mut 清单, &报告);
            match 项目.保存导入清单(&清单) {
                Ok(()) => {
                    编辑器.结构化信息(
                        "log_import_finished",
                        vec![
                            日志参数::用户数据(报告.摘要()),
                            日志参数::用户数据(待处理.to_string()),
                        ],
                    );
                }
                Err(错误) => 编辑器.错误(format!("写入 imports.ron 失败：{错误}")),
            }
            // 让界面下一帧重算状态表（导入后圆点应当变灰）。
            导入.需刷新 = true;
            Ok(())
        }

        UiAction::检查资源依赖 => {
            let Some(项目) = 编辑器.项目.as_ref() else {
                编辑器.警告(界面状态.语言.取("build_no_project_hint").to_string());
                return Ok(());
            };
            let 缺失 = 项目.检查资源引用();
            if 缺失.is_empty() {
                编辑器.结构化信息("log_dependency_ok", Vec::new());
            } else {
                for 项 in &缺失 {
                    编辑器.结构化警告(
                        "log_dependency_missing",
                        vec![
                            日志参数::用户数据(项.场景.文本().to_string()),
                            日志参数::用户数据(项.资源.文本().to_string()),
                        ],
                    );
                }
                编辑器.结构化警告(
                    "log_dependency_summary",
                    vec![日志参数::用户数据(缺失.len().to_string())],
                );
            }
            Ok(())
        }

        其他 => Err(其他),
    }
}
