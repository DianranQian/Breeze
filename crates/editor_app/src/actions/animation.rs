//! 动画域动作（动画文件生命周期、播放控制、轨道 / 关键帧 / 事件编辑）。
//!
//! 从 `actions.rs` 拆出；分派入口仍是 `super::执行单个动作`。

use super::*;

pub(super) fn 处理动画动作(
    动作: UiAction,
    上下文: &mut 动作上下文<'_>,
) -> Result<(), UiAction> {
    let 编辑器 = &mut *上下文.编辑器;
    let 界面状态 = &mut *上下文.界面状态;
    let 对话框 = &mut *上下文.对话框;

    match 动作 {
        UiAction::新建动画 => {
            let 名称 = 编辑器
                .默认动画路径()
                .as_ref()
                .and_then(|路径| 路径.文件名().strip_suffix(".anim.ron").map(str::to_string))
                .unwrap_or_else(|| 编辑器.场景名().to_string());
            编辑器.新建动画(名称);
            界面状态.时间轴选中关键帧 = None;
            界面状态.聚焦面板(editor_ui::Tab::时间轴);
            Ok(())
        }

        UiAction::请求打开动画 => {
            if 编辑器.项目.is_none() {
                编辑器.警告(界面状态.语言.取("log_build_no_project").to_string());
            } else {
                请求对话框(对话框, 编辑器, 文件对话框请求::打开动画文件);
            }
            Ok(())
        }

        UiAction::打开动画(路径) => {
            match 编辑器.打开动画(&路径) {
                Ok(()) => {
                    界面状态.时间轴选中关键帧 = None;
                    界面状态.聚焦面板(editor_ui::Tab::时间轴);
                }
                Err(错误) => 编辑器.错误(format!("打开动画失败：{错误}")),
            }
            Ok(())
        }

        UiAction::保存动画 => {
            // 没有关联路径时退化为"另存为"，与场景保存的交互一致。
            let 已有路径 = 编辑器
                .动画
                .as_ref()
                .and_then(|会话| 会话.路径.clone())
                .is_some();
            if !已有路径 {
                请求对话框(对话框, 编辑器, 文件对话框请求::动画另存为);
            } else {
                match 编辑器.保存动画() {
                    Ok(()) => 界面状态.时间轴选中关键帧 = None,
                    Err(错误) => 编辑器.错误(format!("保存动画失败：{错误}")),
                }
            }
            Ok(())
        }

        UiAction::请求动画另存为 => {
            if 编辑器.动画.is_some() {
                请求对话框(对话框, 编辑器, 文件对话框请求::动画另存为);
            } else {
                编辑器.警告(界面状态.语言.取("anim_no_animation").to_string());
            }
            Ok(())
        }

        UiAction::关闭动画 => {
            编辑器.关闭动画();
            界面状态.时间轴选中关键帧 = None;
            Ok(())
        }

        UiAction::播放动画 => {
            if 编辑器.动画.is_none() {
                编辑器.警告(界面状态.语言.取("anim_no_animation").to_string());
            } else {
                编辑器.动画播放.播放中 = true;
            }
            Ok(())
        }

        UiAction::暂停动画 => {
            编辑器.动画播放.播放中 = false;
            Ok(())
        }

        UiAction::停止动画 => {
            编辑器.动画播放.播放中 = false;
            编辑器.动画播放.时间 = 0.0;
            Ok(())
        }

        UiAction::动画定位(时间) => {
            编辑器.动画定位(时间);
            // 定位是"看某一时刻"，不应顺手开始播放。
            编辑器.动画播放.播放中 = false;
            Ok(())
        }

        UiAction::设置动画时长(时长) => {
            if let Some(会话) = 编辑器.动画.as_mut() {
                会话.文档.时长 = 时长.clamp(
                    editor_core::animation::最小时长,
                    editor_core::animation::最大时长,
                );
                let 上限 = 会话.文档.时长;
                if 编辑器.动画播放.时间 > 上限 {
                    编辑器.动画播放.时间 = 上限;
                }
            }
            Ok(())
        }

        UiAction::设置动画循环(循环) => {
            if let Some(会话) = 编辑器.动画.as_mut() {
                会话.文档.循环 = 循环;
            }
            Ok(())
        }

        UiAction::设置动画速度(速度) => {
            编辑器.动画播放.速度 = 速度.clamp(0.1, 8.0);
            Ok(())
        }

        UiAction::录制动画关键帧 { 属性 } => {
            let 数量 = 编辑器.录制选中实体关键帧(&属性);
            if 数量 == 0 {
                编辑器.结构化警告("log_animation_nothing_recorded", Vec::new());
            } else {
                编辑器.结构化信息(
                    "log_animation_recorded",
                    vec![日志参数::用户数据(数量.to_string())],
                );
            }
            Ok(())
        }

        UiAction::设置关键帧切线 {
            实体,
            属性,
            时间,
            出,
            入,
        } => {
            // 先确认目标帧存在，再记一步、再修改：
            // "不存在的帧"不该在历史里留下一条空步骤。
            let 目标存在 = 编辑器
                .动画
                .as_ref()
                .and_then(|会话| 会话.文档.查找轨道(实体, 属性))
                .is_some_and(|轨道| {
                    轨道
                        .关键帧
                        .iter()
                        .any(|帧| (帧.时间 - 时间).abs() <= editor_core::animation::时间容差)
                });
            if 目标存在 && let Some(会话) = 编辑器.动画.as_mut() {
                会话.记一步();
            }
            if let Some(会话) = 编辑器.动画.as_mut()
                && let Some(轨道) = 会话
                    .文档
                    .轨道
                    .iter_mut()
                    .find(|轨道| 轨道.实体 == 实体 && 轨道.属性 == 属性)
                && let Some(帧) = 轨道
                    .关键帧
                    .iter_mut()
                    .find(|帧| (帧.时间 - 时间).abs() <= editor_core::animation::时间容差)
            {
                帧.出切线 = 出;
                帧.入切线 = 入;
            }
            Ok(())
        }

        UiAction::撤销动画 => {
            if let Some(会话) = 编辑器.动画.as_mut() {
                if 会话.撤销() {
                    界面状态.时间轴选中关键帧 = None;
                    编辑器.信息("已撤销一步动画改动");
                } else {
                    编辑器.信息("动画没有可撤销的改动");
                }
            }
            Ok(())
        }

        UiAction::重做动画 => {
            if let Some(会话) = 编辑器.动画.as_mut() {
                if 会话.重做() {
                    界面状态.时间轴选中关键帧 = None;
                    编辑器.信息("已重做一步动画改动");
                } else {
                    编辑器.信息("动画没有可重做的改动");
                }
            }
            Ok(())
        }

        UiAction::设置轨道插值 {
            实体, 属性, 插值
        } => {
            // 脏状态由"当前文本 vs 上次保存文本"派生，因此改完文档自然就脏了，
            // 不需要（也没有）一个显式的"标记脏"接口。
            if let Some(会话) = 编辑器.动画.as_mut()
                && let Some(轨道) = 会话
                    .文档
                    .轨道
                    .iter_mut()
                    .find(|轨道| 轨道.实体 == 实体 && 轨道.属性 == 属性)
            {
                轨道.插值 = 插值;
            }
            Ok(())
        }

        UiAction::删除动画轨道 { 实体, 属性 } => {
            if let Some(会话) = 编辑器.动画.as_mut()
                && 会话.文档.删除轨道(实体, 属性)
            {
                界面状态.时间轴选中关键帧 = None;
            }
            Ok(())
        }

        UiAction::添加动画事件 { 时间, 类型 } => {
            // 名称优先取项目里第一个音频资源（音效事件它就是真实资源）；
            // 没有音频时用占位名，用户后面可以改。
            let 默认名称 = match 类型 {
                editor_core::animation::事件类型::音效 => 编辑器
                    .项目
                    .as_ref()
                    .and_then(|项目| {
                        项目
                            .资源列表()
                            .into_iter()
                            .find(|条目| {
                                !条目.是目录
                                    && 条目.类别 == scene_doc::property::资源类别::音频
                            })
                            .map(|条目| 条目.路径.文本().to_string())
                    })
                    .unwrap_or_else(|| "assets/audio/未指定.ogg".to_string()),
                _ => "未命名事件".to_string(),
            };
            let Some(会话) = 编辑器.动画.as_mut() else {
                编辑器.警告("没有打开动画".to_string());
                return Ok(());
            };
            会话.记一步();
            let 事件 = editor_core::animation::动画事件::新建(时间, 类型, 默认名称);
            if !会话.文档.插入事件(事件.clone()) {
                编辑器.警告("事件名称为空，未添加".to_string());
                return Ok(());
            }
            编辑器.信息(format!("已添加动画事件：{}", 事件.摘要(界面状态.语言)));
            Ok(())
        }

        UiAction::选择动画事件(下标) => {
            界面状态.时间轴选中事件 = 下标;
            Ok(())
        }

        UiAction::更新动画事件 { 下标, 事件 } => {
            let Some(会话) = 编辑器.动画.as_mut() else {
                return Ok(());
            };
            if 下标 >= 会话.文档.事件.len() {
                编辑器.警告("要更新的事件已不存在".to_string());
                return Ok(());
            }
            // 名称不能为空：空名称的事件永远不会被运行时消费。
            if 事件.名称.trim().is_empty() {
                编辑器.警告("事件名称不能为空".to_string());
                return Ok(());
            }
            if 会话.文档.事件[下标] == 事件 {
                return Ok(());
            }
            会话.记一步();
            会话.文档.事件[下标] = 事件.clone();
            // 时间可能变了，重新排序；排序后下标会变，清掉选中避免编辑错行。
            会话.文档.事件.sort_by(|甲, 乙| 甲.时间.total_cmp(&乙.时间));
            界面状态.时间轴选中事件 = None;
            Ok(())
        }

        UiAction::删除动画事件 { 时间 } => {
            let Some(会话) = 编辑器.动画.as_mut() else {
                return Ok(());
            };
            // 先确认确实有可删的事件，再记历史：否则会留下一条
            // "撤销后什么都没变"的空步骤。
            let 有目标 = 会话.文档.事件.iter().any(|事件| {
                (事件.时间 - 时间).abs() <= editor_core::animation::动画文档::事件删除容差
            });
            if !有目标 {
                编辑器.警告("该时刻附近没有可删除的事件".to_string());
                return Ok(());
            }
            会话.记一步();
            会话.文档.删除事件最近(时间);
            Ok(())
        }

        UiAction::删除动画关键帧 {
            实体, 属性, 时间
        } => {
            if let Some(会话) = 编辑器.动画.as_mut() {
                let mut 轨道已空 = false;
                if let Some(轨道) = 会话.文档.查找轨道可变(实体, 属性) {
                    editor_core::animation::动画轨道::删除(轨道, 时间);
                    轨道已空 = 轨道.关键帧.is_empty();
                }
                // 删空的轨道一并移除：留下空轨道在时间轴上只是一行噪声。
                if 轨道已空 {
                    会话.文档.删除轨道(实体, 属性);
                }
            }
            界面状态.时间轴选中关键帧 = None;
            Ok(())
        }

        其他 => Err(其他),
    }
}
