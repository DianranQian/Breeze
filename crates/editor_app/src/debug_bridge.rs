//! 编辑器端运行时调试桥：监听、握手、心跳、请求关联与快照状态。
//!
//! # 为什么编辑器是监听方
//!
//! 游戏进程通常只活几十秒到几小时，而编辑器是长期进程。让**编辑器**
//! 在 `127.0.0.1:0` 上监听、把端口和一次性令牌注入游戏环境变量，
//! 比让游戏监听端口有三个直接好处：
//!
//! 1. 游戏不需要打开任何端口，默认安全边界更清晰；
//! 2. 端口由操作系统分配，不会和用户其他程序冲突；
//! 3. 旧游戏进程无法在新会话里"抢连" —— 令牌和运行实例标识都必须匹配。
//!
//! # 数据流
//!
//! ```text
//!   后台线程：accept → 握手 → 心跳/收发帧 → 事件 channel
//!   Bevy：轮询事件 → 更新只读快照 → 界面/AI 读取
//! ```
//!
//! UI 主线程从不做阻塞网络调用。所有请求都带编号，响应按编号与用途
//! 关联；超时、乱序、旧会话消息都会被丢弃或标记为不可用。

use std::collections::BTreeMap;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use editor_core::diagnostics::诊断严重级别;
use editor_core::runtime_debug::{
    请求用途, 调试连接状态, 运行时调试历史, 运行时调试快照
};
use runtime_protocol::{
    世界快照请求, 写入帧, 协议版本, 协议错误, 字段值, 实体列表请求, 客户端问候, 属性修改结果,
    属性修改请求, 帧读取错误, 操作错误, 操作错误代码, 服务端问候, 消息信封, 环境变量, 编码信封,
    能力, 解码信封, 读取帧, 调试响应, 调试消息, 调试请求, 调试通知, 错误代码, 默认心跳间隔毫秒,
    默认最大消息字节,
};

/// 在途请求上限。超过时不再发新请求，避免把编辑器内存和游戏主线程拖垮。
pub const 最大在途请求: usize = 16;

/// 单个请求超时。
pub const 请求超时: Duration = Duration::from_secs(6);

/// 心跳沉默多久判定连接失效。
pub const 心跳沉默上限: Duration = Duration::from_secs(10);

/// 自动重连上限。
pub const 最大重连次数: u32 = 3;

/// 检查器每页默认实体数。
pub const 检查器每页: u32 = 32;

/// 连接期间的自动刷新间隔。
///
/// 运行时实体可能被游戏每帧创建/销毁；只有刷新才能看到变化。
/// 2 秒是"足够及时"与"不把请求洪水打给游戏主线程"之间的折中。
pub const 自动刷新间隔: Duration = Duration::from_secs(2);

/// 一次调试诊断事件（由系统转交统一诊断中心）。
#[derive(Debug, Clone)]
pub struct 调试诊断消息 {
    /// 级别。
    pub 级别: 诊断严重级别,
    /// 文本。
    pub 消息: String,
}

/// 管理器内部命令。
enum 调试指令 {
    请求 { 编号: u64, 请求: 调试请求 },
    重新监听,
    关闭,
}

/// 后台线程事件。
enum 调试事件 {
    监听中(SocketAddr),
    握手中,
    握手成功 {
        客户端: 客户端问候,
        服务端: 服务端问候,
    },
    响应 {
        编号: u64,
        响应: 调试响应,
    },
    心跳,
    通知(调试通知),
    协议错误(协议错误),
    断开(String),
}

struct 调试服务 {
    发送: SyncSender<调试指令>,
    关闭: Arc<AtomicBool>,
    /// 是否允许接受新连接。手动断开后置 false，手动重连后置 true。
    允许连接: Arc<AtomicBool>,
}

/// 运行时调试管理器（Bevy 资源）。
#[derive(Resource)]
pub struct 运行时调试管理器 {
    快照: 运行时调试快照,
    服务: Option<调试服务>,
    事件接收: Option<Mutex<Receiver<调试事件>>>,
    待处理: BTreeMap<u64, (请求用途, Instant)>,
    待写入: BTreeMap<u64, 属性修改请求>,
    历史: 运行时调试历史,
    下一编号: u64,
    选中实体: Option<u64>,
    选中资源: Option<String>,
    当前页: u32,
    每页: u32,
    搜索: String,
    组件过滤: String,
    重连次数: u32,
    游戏运行: bool,
    最近事件时间: Option<Instant>,
    最近自动刷新: Instant,
    诊断: Vec<调试诊断消息>,
    代次: u64,
}

impl Default for 运行时调试管理器 {
    fn default() -> Self {
        Self {
            快照: 运行时调试快照::default(),
            服务: None,
            事件接收: None,
            待处理: BTreeMap::new(),
            待写入: BTreeMap::new(),
            历史: 运行时调试历史::default(),
            下一编号: 1,
            选中实体: None,
            选中资源: None,
            当前页: 0,
            每页: 检查器每页,
            搜索: String::new(),
            组件过滤: String::new(),
            重连次数: 0,
            游戏运行: false,
            最近事件时间: None,
            最近自动刷新: Instant::now(),
            诊断: Vec::new(),
            代次: 0,
        }
    }
}

impl 运行时调试管理器 {
    /// 只读快照。
    pub fn 快照(&self) -> &运行时调试快照 {
        &self.快照
    }

    /// 当前以页为单位的列表状态（供界面显示页码）。
    pub fn 分页(&self) -> (u32, u32) {
        (self.当前页, self.每页)
    }

    /// 当前选中的运行时实体标识。
    pub fn 选中实体标识(&self) -> Option<u64> {
        self.选中实体
    }

    /// 快照/修改历史（只读）。
    pub fn 历史(&self) -> &运行时调试历史 {
        &self.历史
    }

    /// 是否正在监听/连接。
    pub fn 活跃(&self) -> bool {
        self.服务.is_some()
    }

    /// 取走待转交的诊断消息。
    pub fn 取诊断(&mut self) -> Vec<调试诊断消息> {
        std::mem::take(&mut self.诊断)
    }

    /// 游戏进程已经启动。
    pub fn 标记游戏运行(&mut self, 运行: bool) {
        self.游戏运行 = 运行;
        if !运行 {
            self.快照.标记断开("游戏进程已结束");
        }
    }

    /// 为一次新运行准备监听，返回需要注入游戏环境的环境变量。
    ///
    /// 返回的标识会被同一个 `运行实例标识` 写进运行记录。调用方必须把它
    /// 原样交给 [`crate::run::项目运行管理器::启动带标识`]。
    pub fn 准备启动(
        &mut self,
        项目标识: &str,
    ) -> Result<(String, Vec<(String, String)>), String> {
        self.关闭服务();
        let 监听 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|错误| format!("无法在 loopback 上监听调试端口：{错误}"))?;
        let 地址 = 监听
            .local_addr()
            .map_err(|错误| format!("无法取得调试监听地址：{错误}"))?;
        if !地址.ip().is_loopback() {
            return Err(format!("调试监听地址 {地址} 不是 loopback，已拒绝"));
        }
        let 令牌 = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let 运行实例标识 = format!("run-{}", uuid::Uuid::new_v4().simple());
        self.代次 = self.代次.wrapping_add(1);
        let 代次 = self.代次;
        self.快照.开始新会话(代次, Some(运行实例标识.clone()));
        self.快照.状态 = 调试连接状态::等待连接;
        self.快照.监听地址 = Some(地址.to_string());
        self.代次 = 代次;
        self.选中实体 = None;
        self.选中资源 = None;
        self.待处理.clear();
        self.待写入.clear();
        self.历史.清空();
        self.重连次数 = 0;
        self.游戏运行 = true;
        self.最近事件时间 = Some(Instant::now());
        self.最近自动刷新 = Instant::now();

        let (命令发送, 命令接收) = sync_channel::<调试指令>(64);
        let (事件发送, 事件接收) = sync_channel::<调试事件>(256);
        let 关闭 = Arc::new(AtomicBool::new(false));
        let 允许连接 = Arc::new(AtomicBool::new(true));
        监听
            .set_nonblocking(true)
            .map_err(|错误| format!("无法设置调试监听为非阻塞：{错误}"))?;

        let 线程关闭 = 关闭.clone();
        let 线程允许连接 = 允许连接.clone();
        let 线程令牌 = 令牌.clone();
        let 线程项目 = 项目标识.to_string();
        let 线程运行 = 运行实例标识.clone();
        std::thread::Builder::new()
            .name("breeze-debug-server".to_string())
            .spawn(move || {
                服务线程(
                    监听,
                    线程令牌,
                    线程项目,
                    线程运行,
                    命令接收,
                    事件发送,
                    线程关闭,
                    线程允许连接,
                )
            })
            .map_err(|错误| format!("无法启动调试监听线程：{错误}"))?;

        self.事件接收 = Some(Mutex::new(事件接收));
        self.服务 = Some(调试服务 {
            发送: 命令发送,
            关闭,
            允许连接,
        });
        Ok((
            运行实例标识,
            vec![
                (环境变量::端口.to_string(), 地址.port().to_string()),
                (环境变量::令牌.to_string(), 令牌),
                (环境变量::项目标识.to_string(), 项目标识.to_string()),
                (
                    环境变量::运行实例标识.to_string(),
                    self.快照.运行实例标识.clone().unwrap_or_default(),
                ),
            ],
        ))
    }

    /// 游戏启动失败时调用：关闭监听并给出原因。
    pub fn 启动失败(&mut self, 原因: impl Into<String>) {
        self.游戏运行 = false;
        self.快照.标记失败(原因);
        self.关闭服务();
    }

    /// 明确标记"本次运行不提供运行时调试"（例如独立预览进程）。
    pub fn 标记未启用(&mut self, 原因: impl Into<String>) {
        self.游戏运行 = false;
        self.快照.状态 = 调试连接状态::未启用;
        self.快照.状态说明 = 原因.into();
        self.快照.已过期 = true;
        self.快照.查询中 = false;
        self.关闭服务();
    }

    /// 游戏退出/用户停止：关闭连接，保留最近快照并标记过期。
    pub fn 结束运行(&mut self, 原因: impl Into<String>) {
        let 原因 = 原因.into();
        self.游戏运行 = false;
        self.快照.标记断开(原因.clone());
        self.取消待写入(&原因);
        self.待处理.clear();
        self.关闭服务();
    }

    /// 手动断开（不结束游戏）。
    pub fn 手动断开(&mut self) {
        self.快照.标记断开("用户手动断开调试连接");
        self.取消待写入("用户手动断开调试连接");
        self.待处理.clear();
        if let Some(服务) = &self.服务 {
            服务.允许连接.store(false, Ordering::Release);
            let _ = 服务.发送.try_send(调试指令::关闭);
        }
    }

    /// 手动重新连接：关闭当前会话，回到监听状态。
    pub fn 手动重连(&mut self) {
        if self.服务.is_none() {
            self.诊断.push(调试诊断消息 {
                级别: 诊断严重级别::警告,
                消息: "没有可重连的调试监听会话：请重新运行游戏".to_string(),
            });
            return;
        }
        self.快照.状态 = 调试连接状态::等待连接;
        self.快照.状态说明 = "正在等待游戏重新连接".to_string();
        self.快照.已过期 = true;
        self.快照.过期原因 = Some("重新连接中".to_string());
        self.待处理.clear();
        self.重连次数 = 0;
        if let Some(服务) = &self.服务 {
            服务.允许连接.store(true, Ordering::Release);
            let _ = 服务.发送.try_send(调试指令::重新监听);
        }
    }

    /// 设置实体列表的查询条件并请求第一页。
    pub fn 查询实体(&mut self, 页: u32, 每页: u32, 搜索: String, 组件过滤: String) {
        self.当前页 = 页;
        self.每页 = 每页.clamp(1, runtime_protocol::最大每页实体数);
        self.搜索 = 搜索;
        self.组件过滤 = 组件过滤;
        self.请求实体列表();
    }

    /// 刷新当前视图（概览 + 当前页 + 资源）。
    pub fn 刷新(&mut self) {
        self.请求概览();
        self.请求实体列表();
        self.请求资源列表();
        if let Some(标识) = self.选中实体 {
            self.请求实体详情(标识);
        }
        if let Some(名称) = self.选中资源.clone() {
            self.请求资源详情(&名称);
        }
    }

    /// 选中实体并请求详情。
    pub fn 选中实体(&mut self, 标识: u64) {
        self.选中实体 = Some(标识);
        self.快照.选中实体(标识);
        self.请求实体详情(标识);
    }

    /// 选中资源并请求详情。
    pub fn 选中资源(&mut self, 名称: &str) {
        self.选中资源 = Some(名称.to_string());
        self.快照.选中资源(名称);
        self.请求资源详情(名称);
    }

    /// 请求一份有界世界快照。
    pub fn 采集快照(&mut self) -> Result<(), String> {
        if !self.快照.是否实时() {
            return Err("当前没有实时调试连接，不能采集快照".to_string());
        }
        self.发送(
            请求用途::快照,
            调试请求::世界快照(世界快照请求 {
                最大实体: 1000,
                包含资源: true,
            }),
        );
        Ok(())
    }

    /// 比较最近两份快照。
    pub fn 比较最近快照(&mut self) -> Option<editor_core::运行时快照差异> {
        self.历史.比较最近两份()
    }

    /// 提交一次白名单属性修改。只有游戏端明确成功后才更新显示值。
    #[allow(clippy::too_many_arguments)]
    pub fn 提交属性修改(
        &mut self,
        实体: u64,
        组件: String,
        字段: String,
        索引: Option<u32>,
        目标值: 字段值,
        预期原值: 字段值,
    ) -> Result<(), String> {
        if !self.快照.是否实时() {
            return Err("当前没有实时调试连接，写入已禁用".to_string());
        }
        if self.快照.修改中 {
            return Err("已有属性修改在等待游戏端结果".to_string());
        }
        if 组件.chars().count() > 128 || 字段.chars().count() > 128 {
            return Err("组件名或字段名过长".to_string());
        }
        let 请求标识 = format!("write-{}", uuid::Uuid::new_v4().simple());
        let 请求 = 属性修改请求 {
            实体,
            组件,
            字段,
            索引,
            目标值,
            预期原值: Some(预期原值),
            请求标识,
            请求时间毫秒: 现在毫秒(),
        };
        let Some(服务) = &self.服务 else {
            return Err("调试连接已关闭".to_string());
        };
        if self.待处理.len() >= 最大在途请求 {
            return Err("在途请求过多，请稍后重试".to_string());
        }
        let 编号 = self.下一编号;
        self.下一编号 = self.下一编号.wrapping_add(1);
        服务
            .发送
            .try_send(调试指令::请求 {
                编号,
                请求: 调试请求::修改属性(请求.clone()),
            })
            .map_err(|_| "调试连接已关闭，写入未发送".to_string())?;
        self.待处理
            .insert(编号, (请求用途::属性修改, Instant::now()));
        self.待写入.insert(编号, 请求);
        self.快照.修改中 = true;
        Ok(())
    }

    /// 用户显式把当前运行时实体关联到微风场景实体。
    ///
    /// 这是**用户指定**的关联，不是根据名称/Transform 猜测出来的。
    pub fn 关联场景实体(&mut self, 场景实体: String) -> Result<(), String> {
        let Some(运行时) = self.选中实体 else {
            return Err("请先选中一个运行时实体".to_string());
        };
        if !self.快照.是否实时() {
            return Err("当前没有实时调试连接".to_string());
        }
        if 场景实体.trim().is_empty() {
            return Err("场景实体标识为空".to_string());
        }
        self.快照.关联 = Some(editor_core::运行时关联 {
            场景实体,
            来源: "用户指定".to_string(),
            置信度: "用户确认".to_string(),
            说明: "由用户在检查器中显式建立；不是自动推断".to_string(),
            运行实例标识: self.快照.运行实例标识.clone(),
            运行时实体: 运行时,
        });
        Ok(())
    }

    /// 清除关联。
    pub fn 清除关联(&mut self) {
        self.快照.关联 = None;
    }

    /// 轮询后台线程事件。返回时需要转交诊断中心的消息。
    pub fn 轮询(&mut self) -> Vec<调试诊断消息> {
        let 事件: Vec<调试事件> = {
            let Some(接收) = &self.事件接收 else {
                return self.取诊断();
            };
            let Ok(接收) = 接收.lock() else {
                return self.取诊断();
            };
            let mut 列表 = Vec::new();
            loop {
                match 接收.try_recv() {
                    Ok(项) => 列表.push(项),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            列表
        };
        for 项 in 事件 {
            self.最近事件时间 = Some(Instant::now());
            match 项 {
                调试事件::监听中(地址) => {
                    self.快照.状态 = 调试连接状态::等待连接;
                    self.快照.状态说明 = format!(
                        "已监听 {地址}，等待游戏连接。若游戏没有接入调试插件，会一直停在这里。"
                    );
                    self.快照.监听地址 = Some(地址.to_string());
                }
                调试事件::握手中 => {
                    self.快照.状态 = 调试连接状态::握手中;
                    self.快照.状态说明 = "已接受本机连接，正在校验令牌与协议版本".to_string();
                }
                调试事件::握手成功 {
                    客户端, 服务端
                } => {
                    self.重连次数 = 0;
                    self.快照.重连次数 = 0;
                    self.快照.状态 = 调试连接状态::已连接;
                    self.快照.状态说明 = "握手完成，心跳正常".to_string();
                    self.快照.会话标识 = Some(服务端.会话标识.clone());
                    self.快照.应用名称 = Some(客户端.应用名称.clone());
                    self.快照.bevy版本 = Some(客户端.bevy版本.clone());
                    self.快照.游戏启动时间毫秒 = Some(客户端.启动时间毫秒);
                    self.快照.协议版本 = Some(协议版本文本());
                    self.快照.连接时间毫秒 = Some(现在毫秒());
                    self.快照.最近心跳毫秒 = Some(现在毫秒());
                    self.最近自动刷新 = Instant::now();
                    self.快照.已过期 = false;
                    self.快照.过期原因 = None;
                    self.快照.最近错误 = None;
                    self.快照.查询中 = false;
                    self.诊断.push(调试诊断消息 {
                        级别: 诊断严重级别::信息,
                        消息: format!(
                            "调试已连接：{}（Bevy {}，会话 {}）",
                            客户端.应用名称, 客户端.bevy版本, 服务端.会话标识
                        ),
                    });
                    // 握手一完成就拉一次真实世界数据，不让检查器停在空白。
                    self.刷新();
                }
                调试事件::响应 { 编号, 响应 } => {
                    self.处理响应(编号, 响应);
                }
                调试事件::心跳 => {}
                调试事件::通知(通知) => match 通知 {
                    调试通知::游戏即将退出 { 原因 } => {
                        self.游戏运行 = false;
                        self.快照.标记断开(format!("游戏退出：{原因}"));
                        self.取消待写入("游戏退出，写入结果未知");
                        self.待处理.clear();
                        self.诊断.push(调试诊断消息 {
                            级别: 诊断严重级别::信息,
                            消息: format!("游戏退出，调试会话已断开：{原因}"),
                        });
                        self.关闭服务();
                    }
                    调试通知::调试插件已卸载 => {
                        self.快照.标记断开("游戏端调试插件已卸载");
                        self.取消待写入("游戏端调试插件已卸载，写入结果未知");
                    }
                    调试通知::世界已重建 { 快照序号 } => {
                        self.选中实体 = None;
                        self.快照.实体详情 = None;
                        self.快照.实体页 = None;
                        self.快照.关联 = None;
                        self.取消待写入("游戏世界已重建，旧实体上的写入结果未知");
                        self.诊断.push(调试诊断消息 {
                            级别: 诊断严重级别::信息,
                            消息: format!("游戏世界已重建（快照 #{快照序号}），旧实体数据已清空"),
                        });
                    }
                },
                调试事件::协议错误(错误) => {
                    match 错误.代码 {
                        错误代码::版本不兼容 => {
                            self.快照.标记不兼容(错误.消息.clone())
                        }
                        错误代码::认证失败
                        | 错误代码::会话不匹配
                        | 错误代码::畸形消息
                        | 错误代码::消息过大
                        | 错误代码::不支持 => self.快照.标记失败(错误.消息.clone()),
                        _ => {
                            self.快照.最近错误 = Some(错误.消息.clone());
                            self.快照.状态说明 = 错误.消息.clone();
                        }
                    }
                    self.诊断.push(调试诊断消息 {
                        级别: 诊断严重级别::错误,
                        消息: format!("调试协议错误 {}：{}", 错误.代码.代号(), 错误.消息),
                    });
                }
                调试事件::断开(原因) => {
                    // 版本/认证/畸形消息导致的终止状态不能被随后的
                    // socket 关闭覆盖成"等待重连"。
                    if matches!(self.快照.状态, 调试连接状态::不兼容 | 调试连接状态::失败)
                    {
                        continue;
                    }
                    let 以前连接过 = self.快照.状态 == 调试连接状态::已连接;
                    self.快照.标记断开(原因.clone());
                    self.取消待写入(&format!("连接断开：{原因}；写入结果未知"));
                    self.待处理.clear();
                    self.诊断.push(调试诊断消息 {
                        级别: 诊断严重级别::警告,
                        消息: format!("调试连接断开：{原因}"),
                    });
                    if 以前连接过 {
                        self.重连次数 = self.重连次数.saturating_add(1);
                    }
                    self.快照.断开次数 = self.快照.断开次数.saturating_add(1);
                    self.快照.重连次数 = self.重连次数;
                    if self.游戏运行 && self.重连次数 <= 最大重连次数 {
                        // 后台线程仍在监听：下一次游戏重连会重新握手。
                        self.快照.状态 = 调试连接状态::等待连接;
                        self.快照.状态说明 = format!(
                            "连接丢失（{原因}），等待重连（{}/{}）",
                            self.重连次数, 最大重连次数
                        );
                    } else if self.游戏运行 {
                        self.快照
                            .标记失败(format!("连续 {最大重连次数} 次重连失败：{原因}"));
                        // 有上限的重连：到上限就真正关闭监听，避免后台线程
                        // 无限 accept 却再也不会被界面使用。
                        self.关闭服务();
                    }
                }
            }
        }
        self.检查超时();
        // 连接期间定期刷新：游戏里实体被销毁/组件变化时，检查器不会
        // 永远停在打开面板时的那一帧。
        if self.快照.是否实时()
            && self.待处理.len() < 4
            && self.最近自动刷新.elapsed() >= 自动刷新间隔
        {
            self.最近自动刷新 = Instant::now();
            self.刷新();
        }
        self.取诊断()
    }

    /// 请求世界概览。
    pub fn 请求概览(&mut self) {
        self.发送(请求用途::世界概览, 调试请求::世界概览);
    }

    /// 请求实体列表当前页。
    pub fn 请求实体列表(&mut self) {
        self.发送(
            请求用途::实体列表,
            调试请求::实体列表(实体列表请求 {
                游标: Some((self.当前页 * self.每页).to_string()),
                每页: self.每页,
                文本: (!self.搜索.trim().is_empty()).then(|| self.搜索.clone()),
                组件: (!self.组件过滤.trim().is_empty()).then(|| self.组件过滤.clone()),
            }),
        );
    }

    /// 请求实体详情。
    pub fn 请求实体详情(&mut self, 标识: u64) {
        self.发送(请求用途::实体详情, 调试请求::实体详情 { 实体: 标识 });
    }

    /// 请求资源列表。
    pub fn 请求资源列表(&mut self) {
        self.发送(请求用途::资源列表, 调试请求::资源列表);
    }

    /// 请求资源详情。
    pub fn 请求资源详情(&mut self, 名称: &str) {
        self.发送(
            请求用途::资源详情,
            调试请求::资源详情 {
                名称: 名称.to_string(),
            },
        );
    }

    fn 发送(&mut self, 用途: 请求用途, 请求: 调试请求) {
        if !self.快照.是否实时() {
            return;
        }
        let Some(服务) = &self.服务 else {
            return;
        };
        if self.待处理.len() >= 最大在途请求 {
            return;
        }
        let 编号 = self.下一编号;
        self.下一编号 = self.下一编号.wrapping_add(1);
        if 服务.发送.try_send(调试指令::请求 { 编号, 请求 }).is_ok() {
            self.待处理.insert(编号, (用途, Instant::now()));
            self.快照.查询中 = true;
        }
    }

    fn 处理响应(&mut self, 编号: u64, 响应: 调试响应) {
        let Some((用途, 开始)) = self.待处理.remove(&编号) else {
            // 超时后才到的响应，或者旧会话的响应：直接丢弃。
            if matches!(响应, 调试响应::属性修改(_)) {
                self.诊断.push(调试诊断消息 {
                    级别: 诊断严重级别::警告,
                    消息: "收到超时后的属性修改结果，已忽略（不能把迟到结果显示为当前成功）"
                        .to_string(),
                });
            }
            return;
        };
        self.快照.最近请求耗时微秒 =
            Some(开始.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
        if let 调试响应::世界快照(数据) = 响应 {
            if 用途 == 请求用途::快照 && self.快照.是否实时() {
                let 记录 = editor_core::运行时快照记录::从协议(
                    0,
                    数据,
                    self.快照.项目标识.clone(),
                    self.快照.运行实例标识.clone(),
                    self.快照.会话标识.clone(),
                    self.快照.协议版本.clone(),
                    self.快照.应用名称.clone(),
                    self.快照.bevy版本.clone(),
                );
                let 序号 = self.历史.追加记录(记录);
                self.诊断.push(调试诊断消息 {
                    级别: 诊断严重级别::信息,
                    消息: format!("已采集运行时快照 #{序号}"),
                });
            }
            return;
        }
        if let 调试响应::属性修改(结果) = 响应 {
            let Some(请求) = self.待写入.remove(&编号) else {
                self.诊断.push(调试诊断消息 {
                    级别: 诊断严重级别::警告,
                    消息: "收到没有对应请求的属性修改结果，已忽略".to_string(),
                });
                return;
            };
            self.应用修改记录(请求, 结果);
            self.快照.修改中 = !self.待写入.is_empty();
            return;
        }
        let 失败消息 = match &响应 {
            调试响应::失败(错误) => Some(错误.消息.clone()),
            _ => None,
        };
        if !self.快照.应用响应(用途, 响应)
            && let Some(消息) = 失败消息
        {
            if 用途 == 请求用途::实体详情 {
                self.快照.实体详情 = None;
            }
            self.诊断.push(调试诊断消息 {
                级别: 诊断严重级别::警告,
                消息: format!("运行时查询失败：{消息}"),
            });
        }
    }

    fn 应用修改记录(&mut self, 请求: 属性修改请求, 结果: 属性修改结果) {
        let 成功 = 结果.成功;
        let 最终值 = 结果.最终值.clone();
        let 错误 = 结果
            .错误
            .as_ref()
            .map(|错误| 错误.消息.clone())
            .unwrap_or_else(|| "游戏端拒绝写入".to_string());
        self.历史.追加修改(
            self.快照.运行实例标识.clone(),
            self.快照.会话标识.clone(),
            &请求,
            结果,
            true,
        );
        if 成功 {
            if let Some(详情) = self.快照.实体详情.as_mut()
                && 详情.标识 == 请求.实体
                && let Some(组件) = 详情.组件.iter_mut().find(|组件| {
                    组件.类型名 == 请求.组件 || 组件.类型名.ends_with(&format!("::{}", 请求.组件))
                })
                && let Some(字段) = 组件.字段.iter_mut().find(|字段| 字段.名称 == 请求.字段)
                && let Some(值) = 最终值
            {
                字段.值 = 值;
            }
            self.快照.最近错误 = None;
            self.诊断.push(调试诊断消息 {
                级别: 诊断严重级别::信息,
                消息: format!("属性修改成功：{} . {}", 请求.组件, 请求.字段),
            });
        } else {
            self.快照.最近错误 = Some(错误.clone());
            self.诊断.push(调试诊断消息 {
                级别: 诊断严重级别::警告,
                消息: format!("属性修改被拒绝：{错误}"),
            });
        }
    }

    fn 检查超时(&mut self) {
        let 现在 = Instant::now();
        let 过期: Vec<u64> = self
            .待处理
            .iter()
            .filter(|(_, (_, 时间))| 现在.duration_since(*时间) > 请求超时)
            .map(|(编号, _)| *编号)
            .collect();
        for 编号 in 过期 {
            if let Some((用途, _)) = self.待处理.remove(&编号) {
                self.诊断.push(调试诊断消息 {
                    级别: 诊断严重级别::警告,
                    消息: format!("运行时请求超时（{}）", 用途.代号()),
                });
                self.快照.最近错误 = Some(format!("请求超时（{}）", 用途.代号()));
                if 用途 == 请求用途::属性修改
                    && let Some(请求) = self.待写入.remove(&编号)
                {
                    self.历史.追加修改(
                        self.快照.运行实例标识.clone(),
                        self.快照.会话标识.clone(),
                        &请求,
                        超时修改结果(),
                        false,
                    );
                }
            }
        }
        self.快照.查询中 = !self.待处理.is_empty();
        self.快照.修改中 = !self.待写入.is_empty();
        if self.快照.状态 == 调试连接状态::已连接
            && let Some(最近) = self.最近事件时间
            && 现在.duration_since(最近) > 心跳沉默上限
        {
            self.快照.标记断开("超过心跳窗口没有收到游戏消息");
            self.诊断.push(调试诊断消息 {
                级别: 诊断严重级别::警告,
                消息: "调试心跳超时：游戏端可能已失去响应".to_string(),
            });
            if let Some(服务) = &self.服务 {
                let _ = 服务.发送.try_send(调试指令::关闭);
            }
        }
    }

    fn 取消待写入(&mut self, 原因: &str) {
        let 待写入 = std::mem::take(&mut self.待写入);
        for (_, 请求) in 待写入 {
            let mut 结果 = 超时修改结果();
            if let Some(错误) = &mut 结果.错误 {
                错误.消息 = format!("连接结束：{原因}；游戏端是否执行未知");
            }
            self.历史.追加修改(
                self.快照.运行实例标识.clone(),
                self.快照.会话标识.clone(),
                &请求,
                结果,
                false,
            );
        }
        self.快照.修改中 = false;
    }

    fn 关闭服务(&mut self) {
        if let Some(服务) = self.服务.take() {
            服务.关闭.store(true, Ordering::Release);
            let _ = 服务.发送.try_send(调试指令::关闭);
        }
        self.事件接收 = None;
    }
}

impl Drop for 运行时调试管理器 {
    fn drop(&mut self) {
        self.关闭服务();
    }
}

fn 协议版本文本() -> String {
    协议版本::当前().文本()
}

/// 超时/断线时使用的结果：**不是失败，也不是成功**，而是"未确认"。
fn 超时修改结果() -> 属性修改结果 {
    属性修改结果 {
        成功: false,
        冲突: false,
        修改前值: None,
        最终值: None,
        错误: Some(操作错误 {
            代码: 操作错误代码::查询被取消,
            消息: "等待游戏端结果超时，游戏端是否执行未知".to_string(),
        }),
        实体仍存在: false,
        组件仍存在: false,
        结果序号: 0,
        执行时间毫秒: 现在毫秒(),
        执行耗时微秒: 0,
        重复请求: false,
    }
}

fn 现在毫秒() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// 后台监听/会话线程。
#[allow(clippy::too_many_arguments)]
fn 服务线程(
    监听: TcpListener,
    令牌: String,
    项目标识: String,
    运行实例标识: String,
    命令接收: Receiver<调试指令>,
    事件发送: SyncSender<调试事件>,
    关闭: Arc<AtomicBool>,
    允许连接: Arc<AtomicBool>,
) {
    let 地址 = 监听.local_addr().ok();
    if let Some(地址) = 地址 {
        发送事件(&事件发送, 调试事件::监听中(地址));
    }
    loop {
        if 关闭.load(Ordering::Acquire) {
            return;
        }
        if !允许连接.load(Ordering::Acquire) {
            // 手动断开后暂停接受新连接；这样对端自动重试不会立刻
            // 把用户刚断开的会话又接回来。
            while let Ok(命令) = 命令接收.try_recv() {
                if matches!(命令, 调试指令::关闭) {
                    关闭.store(true, Ordering::Release);
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        match 监听.accept() {
            Ok((流, _)) => {
                // Windows 上 accept 出来的 socket 会继承监听 socket 的
                // 非阻塞模式；不显式改回阻塞，后续 read 会立刻返回
                // WSAEWOULDBLOCK(10035)，表现为"刚握手就断开"。
                let _ = 流.set_nonblocking(false);
                发送事件(&事件发送, 调试事件::握手中);
                let 结果 = 处理会话(
                    &流,
                    &令牌,
                    &项目标识,
                    &运行实例标识,
                    &命令接收,
                    &事件发送,
                    &关闭,
                );
                match 结果 {
                    Ok(()) => {}
                    Err(原因) => {
                        发送事件(&事件发送, 调试事件::断开(原因));
                    }
                }
                let _ = 流.shutdown(std::net::Shutdown::Both);
            }
            Err(错误) if 错误.kind() == std::io::ErrorKind::WouldBlock => {
                // 没有连接时也要消费关闭命令。
                while let Ok(命令) = 命令接收.try_recv() {
                    if matches!(命令, 调试指令::关闭) {
                        关闭.store(true, Ordering::Release);
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(错误) => {
                发送事件(
                    &事件发送,
                    调试事件::断开(format!("监听 accept 失败：{错误}")),
                );
                return;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn 处理会话(
    流: &TcpStream,
    令牌: &str,
    项目标识: &str,
    运行实例标识: &str,
    命令接收: &Receiver<调试指令>,
    事件发送: &SyncSender<调试事件>,
    关闭: &Arc<AtomicBool>,
) -> Result<(), String> {
    let _ = 流.set_nodelay(true);
    let _ = 流.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = 流.set_write_timeout(Some(Duration::from_secs(2)));
    // 两个句柄共享同一个 socket：读线程与写线程各持有一个克隆。
    let mut 写 = 流
        .try_clone()
        .map_err(|错误| format!("复制调试连接句柄失败：{错误}"))?;
    let mut 读 = 流
        .try_clone()
        .map_err(|错误| format!("复制调试连接句柄失败：{错误}"))?;

    // ---- 握手 ----
    let 截止 = Instant::now() + Duration::from_secs(8);
    let 客户端 = loop {
        if 关闭.load(Ordering::Acquire) {
            return Ok(());
        }
        if Instant::now() >= 截止 {
            return Err("等待客户端问候超时".to_string());
        }
        let 载荷 = match 读取帧(&mut 读, 默认最大消息字节) {
            Ok(载荷) => 载荷,
            Err(帧读取错误::网络错误(原因)) if 超时错误文本(&原因) => {
                continue;
            }
            Err(错误) => return Err(format!("握手读取失败：{错误}")),
        };
        let 信封 = match 解码信封(&载荷, 默认最大消息字节) {
            Ok(信封) => 信封,
            Err(错误) => {
                发送协议错误信封(&mut 写, "", 错误代码::畸形消息, &错误.to_string(), false).ok();
                报告协议错误(事件发送, 错误代码::畸形消息, &错误.to_string());
                return Err(format!("客户端问候不是合法协议消息：{错误}"));
            }
        };
        if let Err(错误) = 协议版本::当前().兼容(信封.协议版本) {
            发送协议错误信封(&mut 写, "", 错误代码::版本不兼容, &错误.to_string(), false).ok();
            报告协议错误(事件发送, 错误代码::版本不兼容, &错误.to_string());
            return Err(format!("协议版本不兼容：{错误}"));
        }
        match 信封.消息 {
            调试消息::客户端问候(问候) => break 问候,
            调试消息::错误(错误) => {
                return Err(format!(
                    "客户端报告协议错误 {}：{}",
                    错误.代码.代号(),
                    错误.消息
                ));
            }
            _ => continue,
        }
    };

    // 认证与身份校验。令牌比较不追求密码学常数时间 —— 这是本机回环
    // 的一次性令牌，且失败会立即断开；真正的边界是"只监听 loopback"。
    if 客户端.令牌 != 令牌 {
        发送协议错误信封(&mut 写, "", 错误代码::认证失败, "调试令牌不匹配", false).ok();
        报告协议错误(事件发送, 错误代码::认证失败, "调试令牌不匹配");
        return Err("调试令牌不匹配".to_string());
    }
    if 客户端.项目标识 != 项目标识 || 客户端.运行实例标识 != 运行实例标识
    {
        发送协议错误信封(
            &mut 写,
            "",
            错误代码::会话不匹配,
            "项目或运行实例标识不匹配",
            false,
        )
        .ok();
        报告协议错误(事件发送, 错误代码::会话不匹配, "项目或运行实例标识不匹配");
        return Err("项目或运行实例标识不匹配".to_string());
    }
    if 客户端.bevy版本 != "0.20" {
        发送协议错误信封(
            &mut 写,
            "",
            错误代码::不支持,
            "当前适配器只声明支持 Bevy 0.20",
            true,
        )
        .ok();
        报告协议错误(事件发送, 错误代码::不支持, "当前适配器只声明支持 Bevy 0.20");
        return Err(format!(
            "适配器声明的 Bevy 版本 `{}` 不受支持（需要 0.20）",
            客户端.bevy版本
        ));
    }

    let 会话标识 = format!("session-{}", uuid::Uuid::new_v4().simple());
    let 服务端 = 服务端问候 {
        服务端名称: format!("微风运行时调试/{}", env!("CARGO_PKG_VERSION")),
        会话标识: 会话标识.clone(),
        能力: vec![
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
        最大消息字节: 默认最大消息字节 as u32,
        心跳间隔毫秒: 默认心跳间隔毫秒,
        请求超时毫秒: 6000,
    };
    let 问候 = 消息信封 {
        协议版本: 协议版本::当前(),
        会话标识: 会话标识.clone(),
        消息: 调试消息::服务端问候(服务端.clone()),
    };
    发送信封(&mut 写, &问候)?;
    发送事件(
        事件发送,
        调试事件::握手成功 {
            客户端, 服务端
        },
    );

    // ---- 会话 ----
    let 心跳间隔 = Duration::from_millis(默认心跳间隔毫秒.max(300));
    let mut 上次心跳 = Instant::now();
    let mut 上次回应 = Instant::now();
    let mut 心跳序号 = 0u64;
    loop {
        if 关闭.load(Ordering::Acquire) {
            let _ = 写.flush();
            return Ok(());
        }
        // 命令
        loop {
            match 命令接收.try_recv() {
                Ok(调试指令::请求 { 编号, 请求 }) => {
                    let 信封 = 消息信封 {
                        协议版本: 协议版本::当前(),
                        会话标识: 会话标识.clone(),
                        消息: 调试消息::请求 { 编号, 请求 },
                    };
                    if let Err(错误) = 发送信封(&mut 写, &信封) {
                        return Err(format!("发送请求失败：{错误}"));
                    }
                }
                Ok(调试指令::重新监听) => {
                    let _ = 写.shutdown(std::net::Shutdown::Both);
                    return Ok(());
                }
                Ok(调试指令::关闭) => {
                    // 编辑器主动断开不是"游戏退出"：不要在协议里伪造这个
                    // 语义。关闭 socket 就足以让对端走断线处理。
                    let _ = 写.shutdown(std::net::Shutdown::Both);
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if 上次心跳.elapsed() >= 心跳间隔 {
            心跳序号 = 心跳序号.wrapping_add(1);
            let 信封 = 消息信封 {
                协议版本: 协议版本::当前(),
                会话标识: 会话标识.clone(),
                消息: 调试消息::心跳 {
                    序号: 心跳序号,
                    发送时间毫秒: 现在毫秒(),
                },
            };
            发送信封(&mut 写, &信封).map_err(|错误| format!("发送心跳失败：{错误}"))?;
            上次心跳 = Instant::now();
        }
        let 载荷 = match 读取帧(&mut 读, 默认最大消息字节) {
            Ok(载荷) => 载荷,
            Err(帧读取错误::网络错误(原因)) if 超时错误文本(&原因) => {
                if 上次回应.elapsed() > 心跳沉默上限 {
                    return Err("心跳超时：游戏端没有回应".to_string());
                }
                continue;
            }
            Err(错误) => return Err(format!("会话读取失败：{错误}")),
        };
        上次回应 = Instant::now();
        let 信封 = match 解码信封(&载荷, 默认最大消息字节) {
            Ok(信封) => 信封,
            Err(错误) => {
                发送协议错误信封(
                    &mut 写,
                    &会话标识,
                    错误代码::畸形消息,
                    &错误.to_string(),
                    false,
                )
                .ok();
                return Err(format!("游戏端发来畸形消息：{错误}"));
            }
        };
        if let Err(错误) = 协议版本::当前().兼容(信封.协议版本) {
            报告协议错误(事件发送, 错误代码::版本不兼容, &错误.to_string());
            return Err(format!("协议版本不兼容：{错误}"));
        }
        if 信封.会话标识 != 会话标识 {
            报告协议错误(事件发送, 错误代码::会话不匹配, "游戏端消息的会话标识不匹配");
            return Err("游戏端消息的会话标识不匹配".to_string());
        }
        match 信封.消息 {
            调试消息::响应 { 编号, 响应 } => {
                发送事件(事件发送, 调试事件::响应 { 编号, 响应 });
            }
            调试消息::心跳 { 序号, .. } => {
                let 回应 = 消息信封 {
                    协议版本: 协议版本::当前(),
                    会话标识: 会话标识.clone(),
                    消息: 调试消息::心跳回应 { 序号 },
                };
                发送信封(&mut 写, &回应).ok();
            }
            调试消息::心跳回应 { .. } => {
                发送事件(事件发送, 调试事件::心跳);
            }
            调试消息::通知(通知) => {
                发送事件(事件发送, 调试事件::通知(通知));
            }
            调试消息::错误(错误) => {
                发送事件(事件发送, 调试事件::协议错误(错误));
            }
            _ => {}
        }
    }
}

fn 报告协议错误(发送: &SyncSender<调试事件>, 代码: 错误代码, 消息: &str) {
    发送事件(
        发送,
        调试事件::协议错误(协议错误 {
            代码,
            消息: 消息.to_string(),
            可恢复: false,
        }),
    );
}

fn 发送事件(发送: &SyncSender<调试事件>, 事件: 调试事件) {
    match 发送.try_send(事件) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            warn!("微风调试：事件队列已满，丢弃一条后台事件");
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

fn 发送信封(流: &mut TcpStream, 信封: &消息信封) -> Result<(), String> {
    let 载荷 = 编码信封(信封, 默认最大消息字节).map_err(|错误| error_text(&错误))?;
    写入帧(流, &载荷).map_err(|错误| 错误.to_string())?;
    流.flush().map_err(|错误| 错误.to_string())
}

fn 发送协议错误信封(
    流: &mut TcpStream,
    会话标识: &str,
    代码: 错误代码,
    消息: &str,
    可恢复: bool,
) -> Result<(), String> {
    发送信封(
        流,
        &消息信封 {
            协议版本: 协议版本::当前(),
            会话标识: 会话标识.to_string(),
            消息: 调试消息::错误(协议错误 {
                代码,
                消息: 消息.to_string(),
                可恢复,
            }),
        },
    )
}

fn error_text(错误: &runtime_protocol::编解码错误) -> String {
    错误.to_string()
}

/// 判断一个 IO 错误文本是不是"读超时/暂时无数据"。
///
/// Windows 的 `WSAEWOULDBLOCK` 是 10035（不是 10060），且系统语言可能
/// 把它翻译成中文。只匹配英文 "WouldBlock" 会在中文 Windows 上漏判，
/// 进而把一次正常空闲当成断线。
fn 超时错误文本(原因: &str) -> bool {
    原因.contains("timed out")
        || 原因.contains("WouldBlock")
        || 原因.contains("would block")
        || 原因.contains("os error 10035")
        || 原因.contains("os error 10060")
        || 原因.contains("非阻止性套接字")
        || 原因.contains("无法立即完成")
}

/// 每帧推进调试连接：轮询后台事件、关联响应、把错误送进统一诊断中心。
///
/// 这个系统不阻塞：所有 socket 操作都在后台线程。
pub fn 推进调试(
    mut 调试: ResMut<运行时调试管理器>,
    运行: Res<crate::run::项目运行管理器>,
    mut 诊断: ResMut<crate::diagnostics::诊断中心>,
) {
    use editor_core::diagnostics::{任务类别, 诊断条目, 诊断来源};

    // 游戏进程已经结束（自然退出/用户停止/启动失败）：
    // 监听必须关闭，历史快照保留但标记为过期。
    if 调试.活跃() && !运行.状态.在运行() {
        let 原因 = if 运行.结果文本().is_empty() {
            "游戏进程已结束".to_string()
        } else {
            format!("游戏进程已结束：{}", 运行.结果文本())
        };
        调试.结束运行(原因);
    }
    // 新运行没有复用同一个运行实例标识：旧连接不能冒充新运行。
    if 调试.活跃()
        && let Some(当前) = 运行.运行实例标识.as_deref()
        && 调试.快照().运行实例标识.as_deref() != Some(当前)
    {
        调试.结束运行("运行实例已切换，旧调试会话被隔离");
    }

    let 消息 = 调试.轮询();
    if 消息.is_empty() {
        return;
    }
    let 任务 = 诊断.集合.最新任务(任务类别::运行);
    let 现在 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_secs())
        .unwrap_or(0);
    for 项 in 消息 {
        let mut 条目 = 诊断条目::新建(诊断来源::运行, 项.级别, 项.消息);
        条目.任务 = 任务;
        条目.发生时间 = 现在;
        诊断.集合.追加(条目);
    }
    诊断.标记界面变化();
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 默认管理器没有任何伪造数据() {
        let 管理器 = 运行时调试管理器::default();
        assert!(!管理器.活跃());
        assert!(!管理器.快照().是否实时());
        assert_eq!(管理器.快照().实体总数(), None);
    }

    #[test]
    fn 准备启动会监听loopback并注入四个环境变量() {
        let mut 管理器 = 运行时调试管理器::default();
        let (运行, 环境) = 管理器.准备启动("项目-a").unwrap();
        assert!(运行.starts_with("run-"));
        assert_eq!(环境.len(), 4);
        let 端口 = 环境
            .iter()
            .find(|(键, _)| 键 == 环境变量::端口)
            .map(|(_, 值)| 值.parse::<u16>().unwrap())
            .unwrap();
        assert_ne!(端口, 0);
        let 令牌 = 环境
            .iter()
            .find(|(键, _)| 键 == 环境变量::令牌)
            .map(|(_, 值)| 值)
            .unwrap();
        assert!(令牌.len() >= 32);
        let 地址 = 管理器.快照().监听地址.clone().unwrap();
        assert!(地址.starts_with("127.0.0.1:"));
        管理器.结束运行("测试结束");
        assert!(!管理器.快照().是否实时());
    }

    #[test]
    fn 新会话不会保留上一次会话的实时状态() {
        let mut 管理器 = 运行时调试管理器::default();
        let _ = 管理器.准备启动("项目").unwrap();
        // 人为塞入"已连接"与数据，模拟上一次运行。
        管理器.快照.状态 = 调试连接状态::已连接;
        管理器.快照.已过期 = false;
        let _ = 管理器.准备启动("项目").unwrap();
        assert_eq!(管理器.快照().状态, 调试连接状态::等待连接);
        assert!(管理器.快照().已过期);
        assert_eq!(管理器.快照().实体总数(), None);
        管理器.结束运行("测试结束");
    }

    #[test]
    fn 会话代次单调递增() {
        let mut 管理器 = 运行时调试管理器::default();
        let _ = 管理器.准备启动("项目").unwrap();
        let 第一 = 管理器.快照().代次;
        let _ = 管理器.准备启动("项目").unwrap();
        assert!(管理器.快照().代次 > 第一);
        管理器.结束运行("测试结束");
    }

    #[test]
    fn 协议主版本不兼容时客户端收到明确错误且编辑器不谎报已连接() {
        use runtime_protocol::{
            写入帧, 协议版本, 客户端问候, 消息信封, 编码信封, 解码信封, 读取帧, 调试消息,
        };

        let mut 管理器 = 运行时调试管理器::default();
        let (运行, 环境) = 管理器.准备启动("项目-版本").unwrap();
        let 端口: u16 = 环境
            .iter()
            .find(|(键, _)| 键 == 环境变量::端口)
            .map(|(_, 值)| 值.parse().unwrap())
            .unwrap();
        let 令牌 = 环境
            .iter()
            .find(|(键, _)| 键 == 环境变量::令牌)
            .map(|(_, 值)| 值.clone())
            .unwrap();

        let mut 流 = std::net::TcpStream::connect(("127.0.0.1", 端口)).unwrap();
        let _ = 流.set_read_timeout(Some(Duration::from_secs(3)));
        let 问候 = 消息信封 {
            协议版本: 协议版本 { 主: 2, 次: 0 },
            会话标识: String::new(),
            消息: 调试消息::客户端问候(客户端问候 {
                令牌,
                项目标识: "项目-版本".to_string(),
                运行实例标识: 运行,
                应用名称: "测试".to_string(),
                bevy版本: "0.20".to_string(),
                客户端名称: "测试客户端".to_string(),
                支持能力: Vec::new(),
                启动时间毫秒: 0,
            }),
        };
        let 载荷 = 编码信封(&问候, 默认最大消息字节).unwrap();
        写入帧(&mut 流, &载荷).unwrap();
        let 回包 = 读取帧(&mut 流, 默认最大消息字节).unwrap();
        let 回 = 解码信封(&回包, 默认最大消息字节).unwrap();
        assert!(matches!(
            回.消息,
            调试消息::错误(错误) if 错误.代码 == 错误代码::版本不兼容
        ));

        for _ in 0..20 {
            管理器.轮询();
            if 管理器.快照().状态 == 调试连接状态::不兼容 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(管理器.快照().状态, 调试连接状态::不兼容);
        assert!(!管理器.快照().是否实时());
        管理器.结束运行("测试结束");
    }
}
