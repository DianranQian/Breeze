//! 运行调试域动作（运行入口偏好 / 工程重新识别 / 诊断面板 / 只读远程 ECS 检查器）。
//!
//! 从 `actions/mod.rs` 拆出；分派入口仍是 `super::执行单个动作`。

use super::*;

/// 处理运行调试域动作：认领属于本域的 `UiAction`，其余原样交回。
///
/// # 为什么这一族要放在一起
///
/// 远程检查器的动作共享同一道门：`调试.快照().是否实时()` 不成立时一律
/// 只记一条警告、不发协议请求。这道门以前散在总表里，各分支自己记得写；
/// 集中到本文件后，新增检查器动作时"忘了挡非实时状态"会被同伴分支显眼地衬托出来。
///
/// 三条"发给智能体"的动作也留在这里而不是智能体域：它们的输入是运行日志、
/// 诊断条目和运行时快照，属于"从调试现场取上下文"，智能体域只管会话本身。
///
/// 返回 `Err(动作)` 表示这条动作不属于运行调试域；
/// 总表仍保留这些变体的哨兵分支，穷尽性检查不会因此放宽。
pub(super) fn 处理运行调试动作(
    动作: UiAction,
    上下文: &mut 动作上下文<'_>,
) -> Result<(), UiAction> {
    let 编辑器 = &mut *上下文.编辑器;
    let 界面状态 = &mut *上下文.界面状态;
    let 布局待保存 = &mut *上下文.布局待保存;
    let 项目运行 = &mut *上下文.项目运行;
    let 配置 = &mut *上下文.配置;
    let 诊断 = &mut *上下文.诊断;
    let 调试 = &mut *上下文.调试;

    match 动作 {
        UiAction::选择运行入口(入口) => {
            let Some(项目) = 编辑器.项目.as_mut() else {
                编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
                return Ok(());
            };
            if 项目.类型() != editor_core::project::项目类型::原生Cargo {
                编辑器.警告(界面状态.语言.取("run_entry_only_cargo").to_string());
                return Ok(());
            }
            match 项目.设置运行入口(入口.clone()) {
                Ok(()) => {
                    // 持久化到**用户偏好**（按项目路径隔离）。
                    // 不写进项目目录：运行入口是"这台机器上这个用户的习惯"，
                    // 不是项目契约（原生工程的 Cargo.toml 更是一个字节都不能改）。
                    let 键 = 运行入口偏好键(项目.根目录());
                    配置.0.偏好.insert(键, 入口.序列化().unwrap_or_default());
                    布局待保存.脏 = true;
                    编辑器.结构化信息(
                        "log_run_entry_selected",
                        vec![日志参数::用户数据(入口.标识())],
                    );
                }
                Err(错误) => {
                    编辑器.警告(format!("{}：{错误}", 界面状态.语言.取("run_entry_invalid")));
                }
            }
            Ok(())
        }

        UiAction::重新识别工程 => {
            // 先在一个短借用里完成识别，拿到结果后再写日志：
            // `结构化信息` 需要 `&mut 编辑器`，与项目借用冲突。
            let 结果 = 编辑器
                .项目
                .as_mut()
                .map(|项目| 项目.重新识别().map(|失效| (项目.名称().to_string(), 失效)));
            match 结果 {
                Some(Ok((名称, 失效))) => {
                    编辑器.结构化信息(
                        "log_project_recognized_again",
                        vec![日志参数::用户数据(名称)],
                    );
                    if let Some(原因) = 失效 {
                        // 这里**真的**发生了回退：新目标已写成默认入口。
                        // 静默替换会让用户以为自己在跑 `server`，实际跑的是
                        // `my_game`，所以必须明说。
                        编辑器.警告(原因.回退说明());
                    }
                }
                Some(Err(错误)) => {
                    编辑器.错误(format!(
                        "{}：{错误}",
                        界面状态.语言.取("run_reidentify_failed")
                    ));
                }
                None => {
                    编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
                }
            }
            Ok(())
        }

        UiAction::清空运行日志 => {
            项目运行.清空日志();
            编辑器.结构化信息("log_run_log_cleared", Vec::new());
            Ok(())
        }

        UiAction::刷新运行时 => {
            if !调试.快照().是否实时() {
                编辑器.警告(界面状态.语言.取("runtime_debug_not_live").to_string());
                return Ok(());
            }
            调试.刷新();
            Ok(())
        }

        UiAction::查询运行时实体 {
            页,
            每页,
            搜索,
            组件,
        } => {
            界面状态.运行时页 = 页;
            界面状态.运行时每页 = 每页;
            界面状态.运行时搜索 = 搜索.clone();
            界面状态.运行时组件过滤 = 组件.clone();
            if !调试.快照().是否实时() {
                编辑器.警告(界面状态.语言.取("runtime_debug_not_live").to_string());
                return Ok(());
            }
            调试.查询实体(页, 每页, 搜索, 组件);
            Ok(())
        }

        UiAction::选中运行时实体 { 实体 } => {
            if !调试.快照().是否实时() {
                编辑器.警告(界面状态.语言.取("runtime_debug_not_live").to_string());
                return Ok(());
            }
            调试.选中实体(实体);
            Ok(())
        }

        UiAction::选中运行时资源 { 名称 } => {
            if !调试.快照().是否实时() {
                编辑器.警告(界面状态.语言.取("runtime_debug_not_live").to_string());
                return Ok(());
            }
            界面状态.运行时资源 = Some(名称.clone());
            调试.选中资源(&名称);
            Ok(())
        }

        UiAction::断开运行时调试 => {
            调试.手动断开();
            编辑器.信息("已断开运行时调试连接（游戏继续运行）".to_string());
            Ok(())
        }

        UiAction::重连运行时调试 => {
            调试.手动重连();
            编辑器.信息("正在等待游戏重新连接调试会话".to_string());
            Ok(())
        }

        UiAction::把运行时上下文发送给智能体 => {
            let 快照 = 调试.快照();
            if !快照.有历史数据() && 调试.历史().快照().is_empty() && 调试.历史().修改().is_empty()
            {
                编辑器.警告(
                    界面状态
                        .语言
                        .取("runtime_debug_nothing_to_send")
                        .to_string(),
                );
                return Ok(());
            }
            let 正文 = editor_core::智能体上下文文本扩展(快照, 调试.历史());
            let 名称 = format!(
                "运行时 ECS 快照（{}，{}）",
                快照.应用名称.as_deref().unwrap_or("未知应用"),
                界面状态.语言.取(快照.数据状态().词条键())
            );
            let 项 = editor_ui::agent::上下文项 {
                种类: editor_ui::agent::上下文种类::运行时,
                名称,
                正文,
            };
            if !界面状态.智能体.附加上下文(项.clone()) {
                let 项目路径 = 编辑器
                    .项目
                    .as_ref()
                    .map(|项目| editor_ui::agent::规范化项目路径(项目.根目录()))
                    .unwrap_or_default();
                let 标识 = format!("运行时调试-{}", editor_ui::agent::现在秒());
                界面状态.智能体.新建会话(&项目路径, 标识);
                界面状态.智能体.附加上下文(项);
            }
            编辑器.结构化信息("runtime_debug_sent_to_agent", Vec::new());
            let _ = 界面状态.切换模式(editor_ui::开发模式::智能体);
            界面状态.聚焦面板(editor_ui::Tab::智能体对话);
            Ok(())
        }

        UiAction::打开诊断位置 { 编号 } => {
            let 条目 = 诊断.集合.条目().iter().find(|条目| 条目.编号 == 编号);
            match 条目 {
                Some(条目) => {
                    if !条目.可跳转() {
                        编辑器.警告(界面状态.语言.取("diag_no_source_location").to_string());
                        return Ok(());
                    }
                    // 复用同一条跳转链路：构建面板能跳，这里也必须能跳。
                    打开源码位置(编辑器, 界面状态, 条目.文件.as_deref(), 条目.行);
                }
                None => 编辑器.警告(界面状态.语言.取("diag_not_found").to_string()),
            }
            Ok(())
        }

        UiAction::把诊断发送给智能体 { 编号 } => {
            let 正文 = match 编号 {
                Some(编号) => 诊断.集合.单条上下文(编号),
                None => {
                    let 文本 = 诊断.集合.上下文文本(30);
                    (!文本.trim().is_empty()).then_some(文本)
                }
            };
            match 正文 {
                Some(正文) => {
                    let 名称 = 编号
                        .and_then(|编号| 诊断.集合.条目().iter().find(|条| 条.编号 == 编号))
                        .map(|条目| format!("诊断 #{} {}", 条目.编号, 条目.单行文本()))
                        .unwrap_or_else(|| "当前诊断结果".to_string());
                    let 项 = editor_ui::agent::上下文项 {
                        种类: editor_ui::agent::上下文种类::诊断,
                        名称,
                        正文,
                    };
                    // 没有会话时先建一个：用户点"发给 Agent"的语义是
                    // "我现在就要问它"，不是"请先自己建会话"。
                    if !界面状态.智能体.附加上下文(项.clone()) {
                        let 项目路径 = 编辑器
                            .项目
                            .as_ref()
                            .map(|项目| editor_ui::agent::规范化项目路径(项目.根目录()))
                            .unwrap_or_default();
                        let 标识 = format!("诊断-{}", editor_ui::agent::现在秒());
                        界面状态.智能体.新建会话(&项目路径, 标识);
                        界面状态.智能体.附加上下文(项);
                    }
                    编辑器.结构化信息("log_diag_sent_to_agent", Vec::new());
                    // 切到 AI 对话模式：用户下一步就是补一句"帮我修"。
                    let _ = 界面状态.切换模式(editor_ui::开发模式::智能体);
                    界面状态.聚焦面板(editor_ui::Tab::智能体对话);
                }
                None => 编辑器.警告(界面状态.语言.取("diag_nothing_to_send").to_string()),
            }
            Ok(())
        }

        UiAction::把运行日志发送给智能体 => {
            let 日志 = &项目运行.日志;
            if 日志.is_empty() {
                编辑器.警告(界面状态.语言.取("run_log_empty").to_string());
                return Ok(());
            }
            // 只取尾部 200 行：一次长时间运行可能有几千行，
            // 全塞进上下文既超预算也淹没了真正有用的那几行。
            let 起点 = 日志.len().saturating_sub(200);
            let 正文 = 日志
                .iter()
                .skip(起点)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let 上下文项 = editor_ui::agent::上下文项 {
                种类: editor_ui::agent::上下文种类::诊断,
                名称: format!(
                    "运行日志（最近 {} 行，入口 {}）",
                    日志.len() - 起点,
                    项目运行
                        .入口
                        .as_ref()
                        .map(|入口| 入口.标识())
                        .unwrap_or_else(|| "未知".to_string())
                ),
                正文: format!(
                    "命令：{}\n结果：{}\n退出码：{}\n\n{}",
                    项目运行
                        .命令
                        .as_ref()
                        .map(|命令| 命令.显示())
                        .unwrap_or_default(),
                    项目运行.结果文本(),
                    项目运行
                        .退出码
                        .map(|码| 码.to_string())
                        .unwrap_or_else(|| "无（进程被停止或未退出）".to_string()),
                    正文
                ),
            };
            if 界面状态.智能体.附加上下文(上下文项.clone()) {
                编辑器.结构化信息("log_run_log_sent_to_agent", Vec::new());
            } else {
                // 没有会话就先建一个：用户点"发给 Agent"的语义是
                // "我现在就要问它"，不是"请先自己建会话"。
                let 项目路径 = 编辑器
                    .项目
                    .as_ref()
                    .map(|项目| editor_ui::agent::规范化项目路径(项目.根目录()))
                    .unwrap_or_default();
                let 标识 = format!("运行日志-{}", editor_ui::agent::现在秒());
                界面状态.智能体.新建会话(&项目路径, 标识);
                界面状态.智能体.附加上下文(上下文项);
                编辑器.结构化信息("log_run_log_sent_to_agent", Vec::new());
            }
            let _ = 界面状态.切换模式(editor_ui::开发模式::智能体);
            界面状态.聚焦面板(editor_ui::Tab::智能体对话);
            Ok(())
        }

        UiAction::清空诊断 => {
            // 只清当前结果视图所依赖的数据：这是用户"清空面板"的语义。
            // 诊断本身是排查历史，全部删掉会让"上一次构建报了什么"永久丢失。
            for 类别 in [
                editor_core::diagnostics::任务类别::构建,
                editor_core::diagnostics::任务类别::运行,
                editor_core::diagnostics::任务类别::智能体,
            ] {
                if let Some(任务) = 诊断.集合.最新任务(类别) {
                    诊断.集合.移除任务(任务);
                }
            }
            诊断.标记界面变化();
            编辑器.结构化信息("log_diagnostics_cleared", Vec::new());
            Ok(())
        }

        UiAction::采集运行时快照 => {
            match 调试.采集快照() {
                Ok(()) => 编辑器.信息("已请求运行时快照，等待游戏端返回".to_string()),
                Err(错误) => 编辑器.警告(错误),
            }
            Ok(())
        }

        UiAction::比较最近运行时快照 => {
            match 调试.比较最近快照() {
                Some(差异) => 编辑器.信息(format!("运行时快照差异：{}", 差异.摘要文本())),
                None => 编辑器.警告("至少需要两份同一运行实例的快照才能比较".to_string()),
            }
            Ok(())
        }

        UiAction::提交运行时属性 {
            实体,
            组件,
            字段,
            索引,
            目标值,
            预期原值,
        } => {
            match 调试.提交属性修改(实体, 组件, 字段, 索引, 目标值, 预期原值)
            {
                Ok(()) => 编辑器.信息("属性修改已提交，等待游戏端真实执行结果".to_string()),
                Err(错误) => 编辑器.警告(错误),
            }
            Ok(())
        }

        UiAction::关联运行时实体 { 场景实体 } => {
            match 调试.关联场景实体(场景实体) {
                Ok(()) => 编辑器.信息("已建立用户指定的运行态/编辑态关联".to_string()),
                Err(错误) => 编辑器.警告(错误),
            }
            Ok(())
        }

        UiAction::清除运行时关联 => {
            调试.清除关联();
            编辑器.信息("已清除运行时关联".to_string());
            Ok(())
        }

        其他 => Err(其他),
    }
}
