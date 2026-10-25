//! 构建打包域动作（构建配置 / 启停构建 / 构建诊断 / 构建并运行 / 打包）。
//!
//! 从 `actions/mod.rs` 拆出；分派入口仍是 `super::执行单个动作`。

use super::*;

/// 构建打包域动作（构建配置 / 启停构建 / 构建诊断 / 构建并运行 / 打包）。
///
/// # 为什么构建与打包放在一起
///
/// 两者都只跟 `构建宿主` 与项目清单打交道：打包必须先有构建产物，
/// 报错口径（"没有项目""没有产物"）也共用同一套日志键。
/// 运行与诊断（`选择运行入口` / 清空日志）仍留在总表，避免把
/// 子进程运行状态混进构建域。
///
/// 返回 `Err(动作)` 表示这条动作不属于构建打包域，原样交回调用者；
/// 总表仍保留这些变体的哨兵分支，穷尽性检查不会因此放宽。
pub(super) fn 处理构建打包动作(
    动作: UiAction,
    上下文: &mut 动作上下文<'_>,
) -> Result<(), UiAction> {
    let 编辑器 = &mut *上下文.编辑器;
    let 界面状态 = &mut *上下文.界面状态;
    let 构建 = &mut *上下文.构建;

    match 动作 {
        UiAction::设置构建配置(配置) => {
            if let Some(项目) = 编辑器.项目.as_mut() {
                项目.清单可变().构建 = Some(配置);
                // 只改内存里的清单；真正落盘发生在保存清单或开始构建时。
                // 每敲一个字就写一次磁盘会把小改动变成高频 IO。
                //
                // 原生 Cargo 工程的清单是从 `Cargo.toml` 推导出来的只读快照：
                // 这里**不写盘**，否则每敲一个字都会在控制台刷一条"原工程只读"
                // 错误。构建配置本来就随项目打开重新推导，不需要持久化。
                if 项目.类型().清单可写()
                    && let Err(错误) = 项目.保存清单()
                {
                    编辑器.错误(format!("构建配置写入项目清单失败：{错误}"));
                }
            } else {
                编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
            }
            Ok(())
        }

        UiAction::开始构建 => {
            // 先把"要构建什么、在哪构建"从项目里取出来，再结束对项目的借用，
            // 这样后面写日志（需要 &mut 编辑器）不会与它冲突。
            let Some((根, 配置, 清单错误)) = 编辑器.项目.as_mut().map(|项目| {
                let 配置 = 项目.清单().构建.clone().unwrap_or_default();
                // 先把配置落盘：构建命令要能在项目被外部工具重建时复现。
                // 原生工程的清单只读，跳过落盘（工作目录仍是 Cargo.toml 所在根）。
                let 清单错误 = if 项目.类型().清单可写() {
                    项目.保存清单().err().map(|错误| 错误.to_string())
                } else {
                    None
                };
                (项目.构建根目录(), 配置, 清单错误)
            }) else {
                编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
                return Ok(());
            };
            if let Some(错误) = 清单错误 {
                编辑器.警告(format!("保存项目清单失败（继续构建）：{错误}"));
            }
            match 构建.启动(&根, &配置) {
                Ok(()) => {
                    编辑器.结构化信息(
                        "log_build_started",
                        vec![日志参数::用户数据(配置.可读命令行())],
                    );
                    界面状态.聚焦面板(editor_ui::Tab::构建);
                }
                Err(原因) => {
                    编辑器.结构化错误("log_build_failed", vec![日志参数::用户数据(原因)]);
                }
            }
            Ok(())
        }

        UiAction::停止构建 => {
            if 构建.在运行() {
                构建.停止();
                编辑器.结构化信息("log_build_stopped", Vec::new());
            }
            Ok(())
        }

        UiAction::打开构建诊断 { 文件, 行 } => {
            打开源码位置(编辑器, 界面状态, 文件.as_deref(), 行);
            Ok(())
        }

        UiAction::构建并运行 => {
            // 先真实构建；构建期间由用户在面板里看到进度，
            // 构建成功后由 `构建完成后启动运行` 系统启动游戏。
            // 这里只置位一次"构建成功后要启动"的意图。
            界面状态.构建后启动 = true;
            if !构建.在运行() {
                let Some((根, 配置构建)) = 编辑器.项目.as_mut().map(|项目| {
                    let 配置 = 项目.清单().构建.clone().unwrap_or_default();
                    (项目.构建根目录(), 配置)
                }) else {
                    编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
                    界面状态.构建后启动 = false;
                    return Ok(());
                };
                match 构建.启动(&根, &配置构建) {
                    Ok(()) => 编辑器.结构化信息(
                        "log_build_started",
                        vec![日志参数::用户数据(配置构建.可读命令行())],
                    ),
                    Err(原因) => {
                        界面状态.构建后启动 = false;
                        编辑器.结构化错误("log_build_failed", vec![日志参数::用户数据(原因)]);
                    }
                }
            }
            界面状态.聚焦面板(editor_ui::Tab::运行);
            Ok(())
        }

        UiAction::设置打包配置(配置) => {
            if let Some(项目) = 编辑器.项目.as_mut() {
                项目.清单可变().打包 = Some(配置);
                if let Err(错误) = 项目.保存清单() {
                    编辑器.错误(format!("打包配置写入项目清单失败：{错误}"));
                }
            }
            Ok(())
        }

        UiAction::打包项目 => {
            let Some(项目) = 编辑器.项目.as_ref() else {
                编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
                return Ok(());
            };
            let 配置 = 项目.清单().打包.clone().unwrap_or_default();
            let Some(产物) = 构建.报告().主产物().cloned() else {
                // 没有产物时打出来的包是"没有游戏的资源包"，必须在入口挡住。
                编辑器.结构化警告("log_package_needs_build", Vec::new());
                界面状态.打包结果 = Some(editor_ui::state::打包结果视图 {
                    成功: false,
                    缺失依赖: Vec::new(),
                    失败项: Vec::new(),
                    ..Default::default()
                });
                return Ok(());
            };

            let 计划 = editor_core::package::生成打包计划(项目, &配置, Some(&产物));
            if !计划.可以打包() {
                界面状态.打包结果 = Some(editor_ui::state::打包结果视图 {
                    成功: false,
                    输出目录: String::new(),
                    文件数: 0,
                    总字节: 0,
                    失败项: Vec::new(),
                    缺失依赖: 计划.缺失依赖.clone(),
                });
                编辑器.结构化警告(
                    "log_package_blocked",
                    vec![日志参数::用户数据(计划.缺失依赖.len().to_string())],
                );
                for 缺失 in &计划.缺失依赖 {
                    编辑器.警告(format!("打包被阻止：{缺失}"));
                }
                return Ok(());
            }

            let 报告 = editor_core::package::执行打包(项目, &配置, &计划);
            界面状态.打包结果 = Some(editor_ui::state::打包结果视图 {
                成功: 报告.成功,
                输出目录: 报告.输出目录.display().to_string(),
                文件数: 报告.文件数,
                总字节: 报告.总字节,
                失败项: 报告.失败项.clone(),
                缺失依赖: Vec::new(),
            });
            if 报告.成功 {
                编辑器.结构化信息(
                    "log_package_done",
                    vec![
                        日志参数::用户数据(报告.文件数.to_string()),
                        日志参数::用户数据(editor_core::package::人类可读字节(报告.总字节)),
                        日志参数::用户数据(报告.输出目录.display().to_string()),
                    ],
                );
            } else {
                编辑器.结构化错误(
                    "log_package_failed",
                    vec![日志参数::用户数据(报告.失败项.len().to_string())],
                );
                for (文件, 原因) in &报告.失败项 {
                    编辑器.错误(format!("打包失败：{文件}：{原因}"));
                }
            }
            Ok(())
        }

        UiAction::打开打包目录 => {
            let Some(项目) = 编辑器.项目.as_ref() else {
                编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
                return Ok(());
            };
            let 配置 = 项目.清单().打包.clone().unwrap_or_default();
            let 目录 = 项目.根目录().join(配置.输出目录路径());
            if !目录.is_dir() {
                编辑器.结构化警告(
                    "log_package_dir_missing",
                    vec![日志参数::用户数据(目录.display().to_string())],
                );
                return Ok(());
            }
            if let Err(错误) = opener::open(&目录) {
                编辑器.错误(format!("打开打包目录失败：{错误}"));
            }
            Ok(())
        }

        其他 => Err(其他),
    }
}
