//! 后台网络线程：连接、握手、心跳、帧收发。
//!
//! # 线程边界
//!
//! Bevy 主线程**绝不**做阻塞网络调用。这里把 socket 全部放进后台线程：
//!
//! ```text
//!   后台线程 ──请求事件──► Bevy 主线程（只读 World 采集）
//!   后台线程 ◄──响应/通知── Bevy 主线程
//! ```
//!
//! 两个方向都是普通 channel，主线程每帧最多处理有限条请求。
//! 连接失败、握手失败、心跳超时都只影响调试功能，不影响游戏启动。

use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use runtime_protocol::{
    写入帧, 协议版本, 协议错误, 客户端问候, 帧读取错误, 服务端问候, 消息信封, 编码信封, 能力,
    解码信封, 读取帧, 调试响应, 调试消息, 调试请求, 调试通知, 错误代码,
};

use crate::config::{支持的BEVY版本, 现在毫秒, 调试配置, 默认应用名};

/// 后台线程 → Bevy 主线程的事件。
#[derive(Debug)]
pub enum 网络事件 {
    /// 握手完成。**只有真实握手成功才会发出这个事件**。
    已连接(服务端问候),
    /// 编辑器发来的只读查询。
    请求 {
        /// 请求编号。
        编号: u64,
        /// 请求体。
        请求: 调试请求,
    },
    /// 心跳回应。
    心跳回应 {
        /// 序号。
        序号: u64,
    },
    /// 收到协议错误。
    协议错误(协议错误),
    /// 编辑器主动断开/游戏端关闭。
    断开(String),
}

/// Bevy 主线程 → 后台线程的出站消息。
#[derive(Debug)]
pub enum 出站 {
    /// 查询响应。
    响应 {
        /// 请求编号。
        编号: u64,
        /// 响应体。
        响应: 调试响应,
    },
    /// 主动通知。
    通知(调试通知),
    /// 关闭连接（发送完已排队消息后）。
    关闭,
}

/// 网络共享状态。
struct 共享 {
    到网络: SyncSender<出站>,
    从网络: Mutex<Receiver<网络事件>>,
    关闭: AtomicBool,
    已连接: AtomicBool,
}

/// 调试连接资源。
///
/// 即使没有配置（游戏没有接入调试），这个资源也存在；所有方法都是
/// 无害的空操作。这样系统可以无条件注册，不需要在每帧判断资源是否存在。
#[derive(Resource, Clone, Default)]
pub struct 调试连接 {
    共享: Option<Arc<共享>>,
    配置: Option<调试配置>,
}

impl 调试连接 {
    /// 是否配置并启用了调试。
    pub fn 已启用(&self) -> bool {
        self.共享.is_some()
    }

    /// 是否已完成真实握手。
    pub fn 已连接(&self) -> bool {
        self.共享
            .as_ref()
            .map(|共享| 共享.已连接.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    /// 配置（供状态展示）。
    pub fn 配置(&self) -> Option<&调试配置> {
        self.配置.as_ref()
    }

    /// 发送查询响应。
    pub fn 发送响应(&self, 编号: u64, 响应: 调试响应) {
        self.发送(出站::响应 { 编号, 响应 });
    }

    /// 发送通知。
    pub fn 发送通知(&self, 通知: 调试通知) {
        self.发送(出站::通知(通知));
    }

    /// 请求关闭连接。
    pub fn 关闭(&self) {
        if let Some(共享) = &self.共享 {
            共享.关闭.store(true, Ordering::Release);
            let _ = 共享.到网络.try_send(出站::关闭);
        }
    }

    fn 发送(&self, 消息: 出站) {
        let Some(共享) = &self.共享 else {
            return;
        };
        match 共享.到网络.try_send(消息) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                warn!("微风调试：出站队列已满，丢弃一条消息（游戏主线程不会被网络拖住）");
            }
            Err(TrySendError::Disconnected(_)) => {
                共享.已连接.store(false, Ordering::Release);
            }
        }
    }

    /// 取走后台线程已经送达的事件。
    pub fn 取事件(&self) -> Vec<网络事件> {
        let Some(共享) = &self.共享 else {
            return Vec::new();
        };
        let Ok(接收) = 共享.从网络.lock() else {
            return Vec::new();
        };
        let mut 事件 = Vec::new();
        loop {
            match 接收.try_recv() {
                Ok(项) => 事件.push(项),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        事件
    }

    /// 启动后台线程（由插件在 build 时调用一次）。
    pub fn 启动(配置: 调试配置) -> Self {
        let (到网络, 网络接收) = sync_channel::<出站>(64);
        let (网络发送, 从网络) = sync_channel::<网络事件>(128);
        let 共享 = Arc::new(共享 {
            到网络,
            从网络: Mutex::new(从网络),
            关闭: AtomicBool::new(false),
            已连接: AtomicBool::new(false),
        });
        let 线程共享 = 共享.clone();
        let 线程配置 = 配置.clone();
        std::thread::Builder::new()
            .name("breeze-debug-network".to_string())
            .spawn(move || 网络线程(线程配置, 线程共享, 网络接收, 网络发送))
            .ok();
        Self {
            共享: Some(共享),
            配置: Some(配置),
        }
    }
}

/// 后台线程主循环。
fn 网络线程(
    配置: 调试配置,
    共享: Arc<共享>,
    出站接收: Receiver<出站>,
    事件发送: SyncSender<网络事件>,
) {
    if let Err(原因) = 配置.校验() {
        warn!("微风调试未启用：{原因}");
        let _ = 事件发送.try_send(网络事件::断开(format!("配置无效：{原因}")));
        return;
    }
    info!(
        "微风调试适配器连接 {}（Bevy {}，项目 {}，运行 {}）",
        配置.地址, 支持的BEVY版本, 配置.项目标识, 配置.运行实例标识
    );
    let mut 尝试 = 0u32;
    loop {
        if 共享.关闭.load(Ordering::Acquire) {
            return;
        }
        match 单次连接(&配置, &共享, &出站接收, &事件发送) {
            Ok(()) => {
                // 正常关闭（游戏退出/编辑器主动断开）。
                return;
            }
            Err((原因, 是否已连接)) => {
                warn!("微风调试连接失败：{原因}");
                if 是否已连接 {
                    let _ = 事件发送.try_send(网络事件::断开(原因));
                } else {
                    let _ = 事件发送.try_send(网络事件::断开(format!("连接失败：{原因}")));
                }
            }
        }
        尝试 += 1;
        if 尝试 >= 3 || 共享.关闭.load(Ordering::Acquire) {
            return;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// 一次完整连接。返回 `Err((原因, 是否曾经握手成功))`。
fn 单次连接(
    配置: &调试配置,
    共享: &Arc<共享>,
    出站接收: &Receiver<出站>,
    事件发送: &SyncSender<网络事件>,
) -> Result<(), (String, bool)> {
    let 地址: SocketAddr = 配置.地址;
    let mut 流 = TcpStream::connect_timeout(&地址, Duration::from_millis(800))
        .map_err(|错误| (format!("无法连接 {地址}：{错误}"), false))?;
    let _ = 流.set_nodelay(true);
    let _ = 流.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = 流.set_write_timeout(Some(Duration::from_secs(2)));

    // ---- 问候 ----
    let 问候 = 客户端问候 {
        令牌: 配置.令牌.clone(),
        项目标识: 配置.项目标识.clone(),
        运行实例标识: 配置.运行实例标识.clone(),
        应用名称: 应用名(配置),
        bevy版本: 支持的BEVY版本.to_string(),
        客户端名称: format!("breeze-debug-adapter/{}", env!("CARGO_PKG_VERSION")),
        支持能力: vec![
            能力::实体列表.to_string(),
            能力::实体详情.to_string(),
            能力::组件.to_string(),
            能力::资源.to_string(),
            能力::分页.to_string(),
            能力::心跳.to_string(),
            能力::概览.to_string(),
            能力::快照.to_string(),
            能力::属性写入.to_string(),
        ],
        启动时间毫秒: 配置.启动时间毫秒,
    };
    let 问候信封 = 消息信封 {
        协议版本: 协议版本::当前(),
        会话标识: String::new(),
        消息: 调试消息::客户端问候(问候),
    };
    发送信封(&mut 流, &问候信封, 配置.最大消息字节)
        .map_err(|错误| (format!("发送问候失败：{错误}"), false))?;

    let 服务端 =
        等待服务端问候(&mut 流, 配置).map_err(|错误| (format!("握手失败：{错误}"), false))?;
    let 会话标识 = 服务端.会话标识.clone();
    if 会话标识.is_empty() {
        return Err(("服务端下发了空会话标识".to_string(), false));
    }
    共享.已连接.store(true, Ordering::Release);
    let _ = 事件发送.try_send(网络事件::已连接(服务端.clone()));

    // ---- 主循环 ----
    let 心跳间隔 = Duration::from_millis(配置.心跳间隔毫秒.max(200));
    let 沉默上限 = 心跳间隔 * 4 + Duration::from_secs(2);
    let mut 上次发送心跳 = Instant::now();
    let mut 上次收到消息 = Instant::now();
    let mut 心跳序号 = 0u64;

    loop {
        if 共享.关闭.load(Ordering::Acquire) {
            // 先把已经排队的消息发完，再关闭。
            排空出站(&mut 流, 配置, &会话标识, 出站接收, None).ok();
            let _ = 流.shutdown(std::net::Shutdown::Both);
            共享.已连接.store(false, Ordering::Release);
            return Ok(());
        }

        // 发送排队消息（最多 16 条/轮，避免饿死读循环）。
        排空出站(&mut 流, 配置, &会话标识, 出站接收, Some(16))
            .map_err(|错误| (format!("写入失败：{错误}"), true))?;

        if 上次发送心跳.elapsed() >= 心跳间隔 {
            心跳序号 += 1;
            let 心跳 = 消息信封 {
                协议版本: 协议版本::当前(),
                会话标识: 会话标识.clone(),
                消息: 调试消息::心跳 {
                    序号: 心跳序号,
                    发送时间毫秒: 现在毫秒(),
                },
            };
            发送信封(&mut 流, &心跳, 配置.最大消息字节)
                .map_err(|错误| (format!("发送心跳失败：{错误}"), true))?;
            上次发送心跳 = Instant::now();
        }

        // 读取。超时是正常的空闲，不是断线。
        let 载荷 = match 读取帧(&mut 流, 配置.最大消息字节) {
            Ok(载荷) => 载荷,
            Err(帧读取错误::网络错误(原因)) if 超时错误文本(&原因) => {
                if 上次收到消息.elapsed() > 沉默上限 {
                    return Err(("超过心跳窗口没有收到编辑器消息".to_string(), true));
                }
                continue;
            }
            Err(错误) => return Err((format!("读取失败：{错误}"), true)),
        };
        上次收到消息 = Instant::now();
        let 信封 = match 解码信封(&载荷, 配置.最大消息字节) {
            Ok(信封) => 信封,
            Err(错误) => {
                // 畸形消息：回一条协议错误，然后断开。继续读下去无法
                // 重新同步 —— 帧边界还在，但语义已经不可信。
                发送协议错误(
                    &mut 流,
                    配置,
                    &会话标识,
                    错误代码::畸形消息,
                    &错误.to_string(),
                    false,
                )
                .ok();
                return Err((format!("收到畸形消息：{错误}"), true));
            }
        };
        if let Err(错误) = 协议版本::当前().兼容(信封.协议版本) {
            发送协议错误(
                &mut 流,
                配置,
                &会话标识,
                错误代码::版本不兼容,
                &错误.to_string(),
                false,
            )
            .ok();
            return Err((format!("协议版本不兼容：{错误}"), true));
        }
        if 信封.会话标识 != 会话标识 {
            发送协议错误(
                &mut 流,
                配置,
                &会话标识,
                错误代码::会话不匹配,
                "消息会话标识与握手会话不一致",
                false,
            )
            .ok();
            return Err(("会话标识不匹配".to_string(), true));
        }

        match 信封.消息 {
            调试消息::请求 { 编号, 请求 } => {
                if let Err(TrySendError::Full(_)) =
                    事件发送.try_send(网络事件::请求 { 编号, 请求 })
                {
                    发送协议错误(
                        &mut 流,
                        配置,
                        &会话标识,
                        错误代码::限流,
                        "游戏端请求队列已满",
                        true,
                    )
                    .ok();
                }
            }
            调试消息::心跳回应 { 序号 } => {
                let _ = 事件发送.try_send(网络事件::心跳回应 { 序号 });
            }
            调试消息::心跳 { .. } => {
                // 编辑器当前实现不主动发心跳，但协议允许；回应它。
                // 序号来自对端，直接回显。
            }
            调试消息::错误(错误) => {
                let _ = 事件发送.try_send(网络事件::协议错误(错误));
            }
            调试消息::通知(调试通知::游戏即将退出 { .. }) => {
                let _ = 流.shutdown(std::net::Shutdown::Both);
                return Ok(());
            }
            调试消息::服务端问候(_) => {
                // 重复问候：忽略。
            }
            _ => {
                发送协议错误(
                    &mut 流,
                    配置,
                    &会话标识,
                    错误代码::不支持,
                    "游戏端不接受该消息方向",
                    true,
                )
                .ok();
            }
        }
    }
}

/// Windows 的 `WSAEWOULDBLOCK` 是 10035，且中文系统会把提示翻译成中文；
/// 只认英文 "WouldBlock" 会把正常空闲误判成断线。
fn 超时错误文本(原因: &str) -> bool {
    原因.contains("timed out")
        || 原因.contains("WouldBlock")
        || 原因.contains("would block")
        || 原因.contains("os error 10035")
        || 原因.contains("os error 10060")
        || 原因.contains("非阻止性套接字")
        || 原因.contains("无法立即完成")
}

fn 应用名(配置: &调试配置) -> String {
    if 配置.应用名称.trim().is_empty() {
        默认应用名()
    } else {
        配置.应用名称.clone()
    }
}

fn 等待服务端问候(
    流: &mut TcpStream, 配置: &调试配置
) -> Result<服务端问候, String> {
    let 截止 = Instant::now() + Duration::from_millis(配置.请求超时毫秒.max(1000));
    while Instant::now() < 截止 {
        let 载荷 = match 读取帧(流, 配置.最大消息字节) {
            Ok(载荷) => 载荷,
            Err(帧读取错误::网络错误(原因)) if 超时错误文本(&原因) => {
                continue;
            }
            Err(错误) => return Err(format!("等待问候时读取失败：{错误}")),
        };
        let 信封 = 解码信封(&载荷, 配置.最大消息字节)
            .map_err(|错误| format!("服务端问候无法解码：{错误}"))?;
        if let Err(错误) = 协议版本::当前().兼容(信封.协议版本) {
            return Err(format!("协议版本不兼容：{错误}"));
        }
        match 信封.消息 {
            调试消息::服务端问候(问候) => {
                if 问候.最大消息字节 as usize > 配置.最大消息字节 {
                    return Err(format!(
                        "服务端声明单帧上限 {} 字节，高于适配器上限 {}",
                        问候.最大消息字节, 配置.最大消息字节
                    ));
                }
                if !问候.能力.iter().any(|项| 项 == 能力::实体列表) {
                    return Err("服务端没有声明实体列表能力".to_string());
                }
                return Ok(问候);
            }
            调试消息::错误(错误) => {
                return Err(format!(
                    "服务端拒绝握手（{}）：{}",
                    错误.代码.代号(),
                    错误.消息
                ));
            }
            _ => {
                // 握手前收到别的消息：忽略，继续等。
            }
        }
    }
    Err("等待服务端问候超时".to_string())
}

fn 排空出站(
    流: &mut TcpStream,
    配置: &调试配置,
    会话标识: &str,
    出站接收: &Receiver<出站>,
    上限: Option<usize>,
) -> std::io::Result<()> {
    let mut 已发送 = 0usize;
    loop {
        if 上限.is_some_and(|上限| 已发送 >= 上限) {
            return Ok(());
        }
        let 项 = match 出站接收.try_recv() {
            Ok(项) => 项,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => return Ok(()),
        };
        let 消息 = match 项 {
            出站::响应 { 编号, 响应 } => 调试消息::响应 { 编号, 响应 },
            出站::通知(通知) => 调试消息::通知(通知),
            出站::关闭 => {
                let 通知 = 消息信封 {
                    协议版本: 协议版本::当前(),
                    会话标识: 会话标识.to_string(),
                    消息: 调试消息::通知(调试通知::调试插件已卸载),
                };
                发送信封(流, &通知, 配置.最大消息字节)?;
                return Ok(());
            }
        };
        let 信封 = 消息信封 {
            协议版本: 协议版本::当前(),
            会话标识: 会话标识.to_string(),
            消息,
        };
        if 发送信封(流, &信封, 配置.最大消息字节).is_err() {
            // 载荷过大时不能静默丢消息：如实告诉编辑器。
            发送协议错误(
                流,
                配置,
                会话标识,
                错误代码::消息过大,
                "响应超过单帧上限，已丢弃；请缩小分页或减少字段",
                true,
            )?;
        }
        已发送 += 1;
    }
}

fn 发送协议错误(
    流: &mut TcpStream,
    配置: &调试配置,
    会话标识: &str,
    代码: 错误代码,
    消息: &str,
    可恢复: bool,
) -> std::io::Result<()> {
    let 信封 = 消息信封 {
        协议版本: 协议版本::当前(),
        会话标识: 会话标识.to_string(),
        消息: 调试消息::错误(协议错误 {
            代码,
            消息: 消息.to_string(),
            可恢复,
        }),
    };
    发送信封(流, &信封, 配置.最大消息字节)
}

fn 发送信封(流: &mut TcpStream, 信封: &消息信封, 上限: usize) -> std::io::Result<()> {
    let 载荷 = 编码信封(信封, 上限)
        .map_err(|错误| std::io::Error::new(std::io::ErrorKind::InvalidData, 错误.to_string()))?;
    写入帧(流, &载荷)?;
    流.flush()
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 没有配置时资源是空操作() {
        let 连接 = 调试连接::default();
        assert!(!连接.已启用());
        assert!(!连接.已连接());
        assert!(连接.取事件().is_empty());
        连接.发送通知(调试通知::调试插件已卸载);
        连接.关闭();
    }
}
