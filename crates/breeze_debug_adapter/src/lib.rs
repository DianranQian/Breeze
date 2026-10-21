//! 微风运行时调试的游戏端 Bevy 适配器。
//!
//! # 怎么接入（可选、默认不影响游戏）
//!
//! ```ignore
//! use bevy::prelude::*;
//! use breeze_debug_adapter::Breeze调试插件;
//!
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     // 自动读取编辑器注入的 BREEZE_DEBUG_* 环境变量。
//!     // 没有这些变量时什么也不做：不建线程、不监听端口。
//!     .add_plugins(Breeze调试插件::自动())
//!     .run();
//! ```
//!
//! 需要检查自定义组件/资源时，显式注册：
//!
//! ```ignore
//! #[derive(Component)]
//! struct 生命值 { 当前: f32, 上限: f32 }
//!
//! Breeze调试插件::自动()
//!     .注册组件::<生命值, _>("生命值", |实体| {
//!         let Some(生命值) = 实体.get::<生命值>() else {
//!             return Vec::new();
//!         };
//!         vec![
//!             runtime_protocol::字段视图::浮点("当前", 生命值.当前.into()),
//!             runtime_protocol::字段视图::浮点("上限", 生命值.上限.into()),
//!         ]
//!     })
//! ```
//!
//! # 兼容性
//!
//! 本 crate **只支持 Bevy 0.20**。0.19 需要独立的适配器 crate/分支，
//! 因为 `EntityRef`、消息系统、`Time` 等 API 在 0.19→0.20 之间并非全部兼容。
//! 协议 crate `runtime_protocol` 不依赖 Bevy，可以被两个适配器同时使用。
//!
//! # 安全
//!
//! * 只连接 loopback；
//! * 只执行白名单检查器；
//! * 只读，不修改游戏世界；
//! * 不执行任意代码/脚本/文件 IO。

mod collect;
mod config;
mod network;
mod registry;

pub use collect::{处理调试连接, 最大扫描实体数, 调试状态, 默认每页实体数};
pub use config::{支持的BEVY版本, 调试配置, 默认最大每帧请求};
pub use network::{出站, 网络事件, 调试连接};
pub use registry::{
    写入结果, 可写字段声明, 最大字段数, 组件检查器, 调试注册, 调试注册构建器, 资源检查器,
};

use bevy::prelude::*;
use runtime_protocol::调试通知;

/// 游戏端调试插件。
///
/// 构建顺序很重要：用户先 `.注册组件`，最后把插件交给 `App::add_plugins`。
/// 插件在 `build` 时把注册表冻结成资源，并启动唯一的网络线程。
pub struct Breeze调试插件 {
    配置: Option<调试配置>,
    注册: 调试注册构建器,
    /// 自动模式下的诊断信息（例如环境变量写错了）。
    配置错误: Option<String>,
}

impl Default for Breeze调试插件 {
    fn default() -> Self {
        Self::自动()
    }
}

impl Breeze调试插件 {
    /// 从环境变量自动配置。没有 `BREEZE_DEBUG_PORT` 时插件完全停用。
    pub fn 自动() -> Self {
        match 调试配置::从环境变量() {
            Ok(配置) => Self {
                配置,
                注册: 调试注册构建器::default(),
                配置错误: None,
            },
            Err(错误) => Self {
                配置: None,
                注册: 调试注册构建器::default(),
                配置错误: Some(错误),
            },
        }
    }

    /// 显式配置（测试或嵌入场景）。地址必须通过 [`调试配置::校验`]。
    pub fn 新建(配置: 调试配置) -> Self {
        Self {
            配置: Some(配置),
            注册: 调试注册构建器::default(),
            配置错误: None,
        }
    }

    /// 明确禁用（即使环境变量存在）。
    pub fn 禁用() -> Self {
        Self {
            配置: None,
            注册: 调试注册构建器::default(),
            配置错误: None,
        }
    }

    /// 注册一个组件检查器。
    pub fn 注册组件<T, F>(mut self, 名称: impl Into<String>, 提取: F) -> Self
    where
        T: Component,
        F: for<'w> Fn(EntityRef<'w>) -> Vec<runtime_protocol::字段视图> + Send + Sync + 'static,
    {
        self.注册.注册组件::<T, F>(名称, 提取);
        self
    }

    /// 注册一个带白名单写入器的组件。
    ///
    /// 这是本适配器**唯一**允许修改游戏世界的入口。写入器在游戏主线程
    /// 的调试系统里执行，并且只接受声明过的字段。
    pub fn 注册可写组件<T, F, W>(
        mut self,
        名称: impl Into<String>,
        提取: F,
        写入: W,
        可写字段: Vec<crate::registry::可写字段声明>,
    ) -> Self
    where
        T: Component<Mutability = bevy::ecs::component::Mutable>,
        F: for<'w> Fn(EntityRef<'w>) -> Vec<runtime_protocol::字段视图> + Send + Sync + 'static,
        W: Fn(
                &mut T,
                &str,
                Option<u32>,
                &runtime_protocol::字段值,
            ) -> Result<crate::registry::写入结果, runtime_protocol::操作错误>
            + Send
            + Sync
            + 'static,
    {
        self.注册
            .注册可写组件::<T, F, W>(名称, 提取, 写入, 可写字段);
        self
    }

    /// 注册一个资源检查器。
    pub fn 注册资源<R, F>(mut self, 名称: impl Into<String>, 提取: F) -> Self
    where
        R: Resource,
        F: Fn(&World) -> Option<Vec<runtime_protocol::字段视图>> + Send + Sync + 'static,
    {
        self.注册.注册资源::<R, F>(名称, 提取);
        self
    }

    /// 不注册内置检查器（只保留用户白名单）。
    pub fn 不注册内置(mut self) -> Self {
        self.注册.不注册内置();
        self
    }
}

impl Plugin for Breeze调试插件 {
    fn build(&self, app: &mut App) {
        if let Some(错误) = &self.配置错误 {
            warn!("微风调试未启用：{错误}");
        }
        let 注册 = self.注册.构建();
        let 连接 = match &self.配置 {
            Some(配置) => match 配置.校验() {
                Ok(()) => 调试连接::启动(配置.clone()),
                Err(错误) => {
                    warn!("微风调试未启用：{错误}");
                    调试连接::default()
                }
            },
            None => 调试连接::default(),
        };
        app.insert_resource(注册)
            .init_resource::<调试状态>()
            .insert_resource(连接)
            // PostUpdate：游戏逻辑已经跑完，采集到的是本帧结束时的真实世界。
            .add_systems(PostUpdate, 处理调试连接)
            // Last：AppExit 在游戏结束时发一条"即将退出"通知。
            .add_systems(Last, 转发退出通知);
    }
}

/// 游戏退出时通知编辑器。
///
/// 这条通知只是"尽力而为"：进程被强杀时 socket 会直接关闭，编辑器
/// 同样会把会话标记为断开。因此**不能**把清理逻辑只放在这里。
fn 转发退出通知(mut 消息: MessageReader<AppExit>, 连接: Res<调试连接>) {
    for 退出 in 消息.read() {
        连接.发送通知(调试通知::游戏即将退出 {
            原因: format!("{退出:?}"),
        });
        连接.关闭();
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 禁用插件不启动线程且可正常装配() {
        let mut 应用 = App::new();
        应用
            .add_plugins(MinimalPlugins)
            .add_plugins(Breeze调试插件::禁用());
        let 连接 = 应用.world().resource::<调试连接>();
        assert!(!连接.已启用());
        assert!(!连接.已连接());
        assert!(应用.world().get_resource::<调试注册>().is_some());
    }

    #[test]
    fn 自动模式在缺少环境变量时停用() {
        // 这个测试进程通常没有 BREEZE_DEBUG_PORT；即使有，也只验证
        // "构造不 panic" 的契约。
        let 插件 = Breeze调试插件::自动();
        let mut 应用 = App::new();
        应用.add_plugins(MinimalPlugins).add_plugins(插件);
        assert!(!应用.world().resource::<调试连接>().已连接());
    }
}
