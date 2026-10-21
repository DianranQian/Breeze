//! GUI 验收专用：自动驱动真实的运行时检查器操作。
//!
//! 这不是产品功能，只有设置 `BEVY_STUDIO_RUNTIME_ACCEPTANCE=1` 才运行。
//! 它不构造假数据：每一步都调用与用户操作相同的
//! [`crate::debug_bridge::运行时调试管理器`] 方法，等待真实游戏端响应，
//! 最终让检查器停留在"一次成功修改 + 一次被拒绝修改 + 一组快照差异"的状态。
//!
//! 这样 GUI 截图可以证明界面确实显示了这些真实结果，而不是空面板。

use bevy::prelude::*;

use crate::debug_bridge::运行时调试管理器;

/// 启用 GUI 运行时验收的環境变量。
pub const 运行时验收环境变量: &str = "BEVY_STUDIO_RUNTIME_ACCEPTANCE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum 阶段 {
    #[default]
    等待连接,
    等待详情,
    第一次快照,
    成功写入,
    拒绝写入,
    第二次快照,
    完成,
}

/// 自动推进验收场景。必须在 `推进调试` 之后运行，才能看到本帧事件。
pub(crate) fn 驱动运行时验收(
    mut 调试: ResMut<运行时调试管理器>, mut 阶段: Local<阶段>
) {
    if std::env::var(运行时验收环境变量).is_err() {
        return;
    }
    if !调试.快照().是否实时() {
        return;
    }
    match *阶段 {
        阶段::等待连接 => {
            if 调试.快照().实体页.is_none() {
                调试.查询实体(0, 32, "玩家".to_string(), String::new());
                return;
            }
            let Some(玩家) = 调试
                .快照()
                .实体页
                .as_ref()
                .and_then(|页| 页.实体.iter().find(|项| 项.名称.as_deref() == Some("玩家")))
                .map(|项| 项.标识)
            else {
                return;
            };
            调试.选中实体(玩家);
            *阶段 = 阶段::等待详情;
            info!("运行时 GUI 验收：已选中玩家实体 {玩家}");
        }
        阶段::等待详情 => {
            if 调试.快照().实体详情.is_some() {
                调试.采集快照().ok();
                *阶段 = 阶段::第一次快照;
            }
        }
        阶段::第一次快照 => {
            if 调试.历史().快照().len() >= 1 {
                let Some(详情) = 调试.快照().实体详情.clone() else {
                    return;
                };
                let Some(当前缩放) = 取字段(&详情, "::transform::Transform", "scale")
                else {
                    return;
                };
                调试
                    .提交属性修改(
                        详情.标识,
                        "Transform".to_string(),
                        "scale".to_string(),
                        None,
                        runtime_protocol::字段值::向量(vec![2.0, 2.0, 2.0]),
                        当前缩放,
                    )
                    .ok();
                *阶段 = 阶段::成功写入;
            }
        }
        阶段::成功写入 => {
            if 调试.历史().修改().iter().rev().any(|项| 项.结果.成功) {
                let Some(详情) = 调试.快照().实体详情.clone() else {
                    return;
                };
                let Some(当前生命) = 取字段(&详情, "玩家属性", "生命值") else {
                    return;
                };
                调试
                    .提交属性修改(
                        详情.标识,
                        "玩家属性".to_string(),
                        "生命值".to_string(),
                        None,
                        runtime_protocol::字段值::浮点(2000.0),
                        当前生命,
                    )
                    .ok();
                *阶段 = 阶段::拒绝写入;
            }
        }
        阶段::拒绝写入 => {
            if 调试
                .历史()
                .修改()
                .iter()
                .rev()
                .any(|项| 项.已确认 && !项.结果.成功)
            {
                调试.采集快照().ok();
                *阶段 = 阶段::第二次快照;
            }
        }
        阶段::第二次快照 => {
            if 调试.历史().快照().len() >= 2 {
                调试.比较最近快照();
                *阶段 = 阶段::完成;
                info!("运行时 GUI 验收：成功写入、拒绝写入与快照差异均已就绪");
            }
        }
        阶段::完成 => {}
    }
}

fn 取字段(
    详情: &runtime_protocol::实体详情,
    组件后缀: &str,
    字段: &str,
) -> Option<runtime_protocol::字段值> {
    详情
        .组件
        .iter()
        .find(|组件| 组件.类型名.ends_with(组件后缀))
        .and_then(|组件| 组件.字段.iter().find(|项| 项.名称 == 字段))
        .map(|项| 项.值.clone())
}
