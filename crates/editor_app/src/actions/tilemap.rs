//! 瓦片域动作（瓦片地图 / 瓦片集的载入与保存）。
//!
//! 从 `actions/mod.rs` 拆出；分派入口仍是 `super::执行单个动作`。

use super::*;

/// 瓦片域动作（瓦片地图 / 瓦片集的载入与保存）。
///
/// # 为什么读盘/写盘集中在这里
///
/// 地图与瓦片集是场景之外的文件。读盘/写盘统一走这里，
/// 面板只改内存里的草稿：这样"路径越界检查""父目录不存在"
/// 这类规则只需要实现一次。
///
/// 返回 `Err(动作)` 表示这条动作不属于瓦片域，原样交回调用者；
/// 总表仍保留这些变体的哨兵分支，穷尽性检查不会因此放宽。
pub(super) fn 处理瓦片动作(
    动作: UiAction,
    上下文: &mut 动作上下文<'_>,
) -> Result<(), UiAction> {
    let 编辑器 = &mut *上下文.编辑器;
    let 界面状态 = &mut *上下文.界面状态;

    match 动作 {
        UiAction::载入瓦片地图 {
            实体: _, 地图文件
        } => {
            let 语言 = 界面状态.语言;
            let 结果 = (|| {
                let 项目 = 编辑器
                    .项目
                    .as_ref()
                    .ok_or_else(|| 语言.取("tilemap_no_project").to_string())?;
                let 相对 = scene_doc::path::项目相对路径::解析(&地图文件)
                    .map_err(|错误| format!("地图路径无效：{错误}"))?;
                let 地图路径 = 项目.绝对路径(&相对);
                let 地图文本 = std::fs::read_to_string(&地图路径)
                    .map_err(|错误| format!("读取瓦片地图 {地图文件} 失败：{错误}"))?;
                let 地图 = editor_core::tilemap::瓦片地图::解析(&地图文本)
                    .map_err(|错误| format!("解析瓦片地图 {地图文件} 失败：{错误}"))?;

                let 集相对文本 = if 界面状态.瓦片地图.瓦片集文件.trim().is_empty() {
                    地图.瓦片集.trim().to_string()
                } else {
                    界面状态.瓦片地图.瓦片集文件.trim().to_string()
                };
                let (集, 集文本) = if 集相对文本.is_empty() {
                    (None, String::new())
                } else {
                    let 集相对 = scene_doc::path::项目相对路径::解析(&集相对文本)
                        .map_err(|错误| format!("瓦片集路径无效：{错误}"))?;
                    let 集路径 = 项目.绝对路径(&集相对);
                    let 文本 = std::fs::read_to_string(&集路径)
                        .map_err(|错误| format!("读取瓦片集 {集相对文本} 失败：{错误}"))?;
                    let 集 = editor_core::tileset::瓦片集::解析(&文本)
                        .map_err(|错误| format!("解析瓦片集 {集相对文本} 失败：{错误}"))?;
                    (Some(集), 集相对文本)
                };
                Ok::<_, String>((地图, 集, 集文本))
            })();

            match 结果 {
                Ok((地图, 集, 集文本)) => {
                    let 编辑 = &mut 界面状态.瓦片地图;
                    编辑.待应用尺寸 = (地图.宽, 地图.高);
                    编辑.地图 = Some(地图);
                    编辑.瓦片集 = 集;
                    编辑.瓦片集文件 = 集文本;
                    编辑.地图文件 = 地图文件;
                    编辑.脏 = false;
                    编辑.瓦片集脏 = false;
                    编辑.错误 = None;
                    编辑.请求中 = None;
                    编辑.待确认 = None;
                    编辑.图层 = 0;
                    编辑.笔刷 = None;
                    编辑.撤销栈.clear();
                    编辑.重做栈.clear();
                }
                Err(错误) => {
                    let 编辑 = &mut 界面状态.瓦片地图;
                    编辑.错误 = Some(错误.clone());
                    编辑.请求中 = None;
                    编辑器.警告(错误);
                }
            }
            Ok(())
        }

        UiAction::保存瓦片地图 {
            实体: _,
            地图文件,
            文本,
        } => {
            let 语言 = 界面状态.语言;
            let 结果 = 写入项目文本(编辑器, &地图文件, &文本);
            match 结果 {
                Ok(()) => {
                    let 编辑 = &mut 界面状态.瓦片地图;
                    编辑.脏 = false;
                    编辑.错误 = None;
                    编辑器.结构化信息("log_tilemap_saved", vec![日志参数::用户数据(地图文件)]);
                }
                Err(错误) => {
                    界面状态.瓦片地图.错误 = Some(错误.clone());
                    编辑器.警告(format!("{}：{错误}", 语言.取("tilemap_save_failed")));
                }
            }
            Ok(())
        }

        UiAction::保存瓦片集 {
            瓦片集文件, 文本
        } => {
            let 语言 = 界面状态.语言;
            let 结果 = 写入项目文本(编辑器, &瓦片集文件, &文本);
            match 结果 {
                Ok(()) => {
                    let 编辑 = &mut 界面状态.瓦片地图;
                    编辑.瓦片集脏 = false;
                    编辑.错误 = None;
                    编辑器
                        .结构化信息("log_tileset_saved", vec![日志参数::用户数据(瓦片集文件)]);
                }
                Err(错误) => {
                    界面状态.瓦片地图.错误 = Some(错误.clone());
                    编辑器.警告(format!("{}：{错误}", 语言.取("tilemap_save_failed")));
                }
            }
            Ok(())
        }

        其他 => Err(其他),
    }
}
