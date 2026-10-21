//! 适配器配置：从哪里连、令牌是什么、上限是多少。
//!
//! 配置默认从环境变量读取。**没有配置就不启用**：一个没有接入调试的
//! 游戏不应该因为本 crate 被链接进来就多出一个后台线程。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::collect::最大扫描实体数;
use crate::registry::最大字段数;
use runtime_protocol::{
    环境变量, 默认心跳间隔毫秒, 默认最大消息字节, 默认请求超时毫秒
};

/// 适配器声明的 Bevy 版本。
///
/// 这不是从 `CARGO_PKG_VERSION` 读出来的 —— 那是**适配器自己**的版本。
/// 游戏用的是哪个 Bevy，由本 crate 的依赖声明决定，并且必须显式写在这里，
/// 让编辑器能如实显示"这个游戏用的是 0.20 适配器"。
pub const 支持的BEVY版本: &str = "0.20";

/// 单帧最多处理多少个请求。
///
/// 一次查询可能触发实体列表 + 详情 + 资源三个请求；上限存在的意义是
/// 防止恶意/故障客户端用请求洪水把游戏主线程钉死在采集逻辑里。
pub const 默认最大每帧请求: usize = 12;

/// 适配器配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 调试配置 {
    /// 编辑器监听地址（必须是 loopback）。
    pub 地址: SocketAddr,
    /// 共享令牌。
    pub 令牌: String,
    /// 项目标识。
    pub 项目标识: String,
    /// 运行实例标识。
    pub 运行实例标识: String,
    /// 应用显示名。
    pub 应用名称: String,
    /// 启动时间（UNIX 毫秒）。
    pub 启动时间毫秒: u64,
    /// 心跳间隔。
    pub 心跳间隔毫秒: u64,
    /// 请求超时。
    pub 请求超时毫秒: u64,
    /// 单条消息上限。
    pub 最大消息字节: usize,
    /// 单帧处理上限。
    pub 最大每帧请求: usize,
    /// 实体扫描上限。
    pub 最大扫描实体数: usize,
    /// 组件单次最多返回字段数。
    pub 最大字段数: usize,
}

impl 调试配置 {
    /// 明确配置一个回环连接（测试与嵌入场景用）。
    pub fn 新建(
        地址: SocketAddr,
        令牌: impl Into<String>,
        项目标识: impl Into<String>,
        运行实例标识: impl Into<String>,
    ) -> Self {
        Self {
            地址,
            令牌: 令牌.into(),
            项目标识: 项目标识.into(),
            运行实例标识: 运行实例标识.into(),
            应用名称: 默认应用名(),
            启动时间毫秒: 现在毫秒(),
            心跳间隔毫秒: 默认心跳间隔毫秒,
            请求超时毫秒: 默认请求超时毫秒,
            最大消息字节: 默认最大消息字节,
            最大每帧请求: 默认最大每帧请求,
            最大扫描实体数,
            最大字段数,
        }
    }

    /// 从环境变量读取；没有端口或令牌时返回 `None`。
    ///
    /// 返回值同时带一条**人读原因**：调用方可以记录"为什么没启用"，
    /// 而不是让用户面对一个静默缺席的调试器。
    pub fn 从环境变量() -> Result<Option<Self>, String> {
        let Some(端口文本) = 读取环境(环境变量::端口) else {
            return Ok(None);
        };
        let 端口: u16 = 端口文本
            .parse()
            .map_err(|_| format!("{} 不是合法端口：{端口文本}", 环境变量::端口))?;
        let 令牌 = 读取环境(环境变量::令牌)
            .ok_or_else(|| format!("缺少 {}：调试服务必须带令牌", 环境变量::令牌))?;
        if 令牌.len() < 16 {
            return Err("调试令牌太短（至少 16 字节）".to_string());
        }
        let 项目标识 = 读取环境(环境变量::项目标识).unwrap_or_else(|| "未知项目".to_string());
        let 运行实例标识 =
            读取环境(环境变量::运行实例标识).unwrap_or_else(|| "未知运行".to_string());
        let 应用名称 = 读取环境(环境变量::应用名称).unwrap_or_else(默认应用名);
        // 只允许 loopback。即使环境变量被伪造，也不能把游戏内部数据
        // 发到局域网/公网地址。
        let 地址 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 端口);
        Ok(Some(Self {
            地址,
            令牌,
            项目标识,
            运行实例标识,
            应用名称,
            启动时间毫秒: 现在毫秒(),
            心跳间隔毫秒: 默认心跳间隔毫秒,
            请求超时毫秒: 默认请求超时毫秒,
            最大消息字节: 默认最大消息字节,
            最大每帧请求: 默认最大每帧请求,
            最大扫描实体数,
            最大字段数,
        }))
    }

    /// 校验安全边界。
    pub fn 校验(&self) -> Result<(), String> {
        if !self.地址.ip().is_loopback() {
            return Err(format!(
                "调试地址 {} 不是 loopback：微风拒绝向远程地址发送运行时数据",
                self.地址
            ));
        }
        if self.令牌.len() < 16 {
            return Err("调试令牌太短（至少 16 字节）".to_string());
        }
        if self.最大消息字节 == 0 {
            return Err("最大消息字节不能为 0".to_string());
        }
        Ok(())
    }
}

fn 读取环境(键: &str) -> Option<String> {
    std::env::var(键)
        .ok()
        .map(|值| 值.trim().to_string())
        .filter(|值| !值.is_empty())
}

/// 可执行文件名（没有时用固定占位）。
pub fn 默认应用名() -> String {
    std::env::args()
        .next()
        .as_deref()
        .map(std::path::Path::new)
        .and_then(std::path::Path::file_stem)
        .and_then(|名| 名.to_str())
        .unwrap_or("bevy-game")
        .to_string()
}

/// 当前 UNIX 毫秒。
pub fn 现在毫秒() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 非回环地址被拒绝() {
        let mut 配置 = 调试配置::新建(
            "0.0.0.0:9000".parse().unwrap(),
            "0123456789abcdef",
            "项目",
            "运行",
        );
        assert!(配置.校验().is_err());
        配置.地址 = "127.0.0.1:9000".parse().unwrap();
        assert!(配置.校验().is_ok());
    }

    #[test]
    fn 短令牌被拒绝() {
        let 配置 = 调试配置::新建("127.0.0.1:9000".parse().unwrap(), "short", "项目", "运行");
        assert!(配置.校验().is_err());
    }

    #[test]
    fn 从环境变量缺端口时不启用() {
        // 不修改进程环境（测试并行会互相污染）；只验证纯函数路径。
        // 真实环境读取由端到端测试以子进程方式覆盖。
        assert!(读取环境("一个肯定不存在的环境变量名_BREEZE").is_none());
    }
}
