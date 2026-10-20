//! DSH（DeepSeek Harness）集成：进程、协议与会话。
//!
//! # 真实接口（本机核对，DSH 0.2.0-rc.2）
//!
//! ```text
//! dsh --version                          → 版本号
//! dsh headless "<task>"                  → 回答写到 stdout，诊断写到 stderr
//! dsh headless --json "<task>"           → 换行分隔的 run events（NDJSON）
//! dsh headless --session-id <id> "<task>"→ 续用已持久化的 Session
//! ```
//!
//! `--json` 的真实事件（实跑抓取，见 `解析事件` 的测试）：
//!
//! ```text
//! {"type":"session","sessionId":"session-…","cwd":"C:\\…"}
//! {"type":"status","phase":"turn_start","turn":1}
//! {"type":"status","phase":"step_start","turn":1,"step":1}
//! {"type":"thinking","text":""}
//! {"type":"text","text":"ok"}
//! {"type":"status","phase":"step_end","turn":1,"step":1,"usage":{…}}
//! {"type":"status","phase":"turn_end","turn":1,"reason":{"kind":"completed"}}
//! {"type":"final","text":"ok"}
//! ```
//!
//! headless **完成一次任务就退出**，因此微风把它当作"一次任务执行"：
//! 会话连续性通过 `--session-id` 续用 DSH 自己的 Session 实现，
//! 而不是维持一个长连接协议（那样需要维护 ACP 的读写线程与心跳，
//! 在编辑器里收益不明确，风险更大）。
//!
//! # 为什么进程放在独立线程
//!
//! DSH 一次任务可能跑几十秒到几分钟。主线程（egui 绘制）必须保持响应，
//! 因此：启动 = 起子进程 + 两个读线程（stdout/stderr）→ 通过 channel 回传；
//! 主线程每帧非阻塞地收事件。取消 = kill 子进程（并如实报告"已取消"，
//! 不假装撤销了副作用）。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use editor_ui::agent::{Dsh连接, 任务状态, 工具调用};

/// 一次任务里从 DSH 收到的事件（已解析）。
#[derive(Debug, Clone, PartialEq)]
pub enum Dsh事件 {
    /// 会话建立：带 DSH 的 Session ID 与工作目录。
    会话 {
        会话标识: String, 工作目录: String
    },
    /// 阶段变化（turn/step）。
    阶段 {
        阶段: String,
        轮次: Option<i64>,
        步: Option<i64>,
    },
    /// 思考片段（可能为空）。
    思考 { 文本: String },
    /// 助手文本片段。
    文本 { 文本: String },
    /// 工具调用事件（名称 + 阶段 + 摘要）。
    工具 {
        名称: String,
        阶段: String,
        摘要: String,
        写操作: bool,
    },
    /// 最终回答。
    最终 { 文本: String },
    /// 无法识别的行（原样保留，便于排查协议变化）。
    未知 { 原文: String },
    /// 读取线程报的 IO 错误。
    读取错误 { 错误: String },
}

/// 把一行 `--json` 输出解析成事件。
///
/// 解析刻意宽松：字段缺失时给出空串，而不是丢事件 —— 协议新增字段时
/// 旧代码仍能显示主要内容。
pub fn 解析事件(行: &str) -> Option<Dsh事件> {
    let 行 = 行.trim();
    if 行.is_empty() || !行.starts_with('{') {
        return None;
    }
    let 值: serde_json::Value = serde_json::from_str(行).ok()?;
    let 类型 = 值.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let 取串 = |键: &str| {
        值.get(键)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    Some(match 类型 {
        "session" => Dsh事件::会话 {
            会话标识: 取串("sessionId"),
            工作目录: 取串("cwd"),
        },
        "status" => Dsh事件::阶段 {
            阶段: 取串("phase"),
            轮次: 值.get("turn").and_then(|v| v.as_i64()),
            步: 值.get("step").and_then(|v| v.as_i64()),
        },
        "thinking" => Dsh事件::思考 {
            文本: 取串("text")
        },
        "text" => Dsh事件::文本 {
            文本: 取串("text")
        },
        "final" => Dsh事件::最终 {
            文本: 取串("text")
        },
        "tool" | "tool_call" | "tool_result" | "tool_use" => {
            let 名称 = {
                let 名 = 取串("name");
                if 名.is_empty() { 取串("tool") } else { 名 }
            };
            let 阶段 = {
                let 阶段 = 取串("phase");
                if 阶段.is_empty() {
                    类型.to_string()
                } else {
                    阶段
                }
            };
            // 真实 DSH 的 `tool_call` 形如
            // `{"type":"tool_call","tool":"edit","input":{"file_path":"C:\\…"}}`；
            // 路径在 `input.file_path` 里，不在 `summary`。`tool_result` 的
            // `result` 是文件内容/命令输出，不能当成路径证据。
            let 摘要 = if 类型 == "tool_result" {
                取串("status")
            } else {
                let 摘要 = 取串("summary");
                if !摘要.is_empty() {
                    摘要
                } else if let Some(输入) = 值.get("input") {
                    let 字段 = ["file_path", "path", "command", "pattern", "url"]
                        .iter()
                        .find_map(|键| 输入.get(键).and_then(|值| 值.as_str()));
                    match 字段 {
                        Some(字段) => 字段.to_string(),
                        None => 截断(&输入.to_string(), 200),
                    }
                } else {
                    取串("input")
                }
            };
            let 写操作 = 取串("writes").contains("true")
                || 取串("write").contains("true")
                || [
                    "write", "edit", "apply", "patch", "create", "delete", "rename", "move",
                ]
                .iter()
                .any(|关键字| 名称.to_ascii_lowercase().contains(关键字));
            Dsh事件::工具 {
                名称,
                阶段,
                摘要: 截断(&摘要, 200),
                写操作,
            }
        }
        _ => Dsh事件::未知 {
            原文: 截断(行, 400),
        },
    })
}

/// 从写工具的事件摘要里提取项目相对路径。
///
/// DSH 的摘要格式不固定：真实 `tool_call` 把绝对路径放在 `input.file_path`，
/// 也有工具直接给裸相对路径。这里只接受能落到**当前项目内**的片段：
///
/// * 绝对路径必须位于 `项目根` 下（`strip_prefix` 失败就拒绝）；
/// * 拒绝 `..`、URL、以 `-` 开头的参数、无法识别的噪声；
/// * 接受 `src/main.rs`、`crates/game/Cargo.toml` 这类带分隔符或扩展名的路径。
///
/// 提取不到就返回 `None` —— 审查层会把这类改动标成"来源未知"，而不是猜。
pub fn 提取写路径(摘要: &str, 项目根: Option<&Path>) -> Option<String> {
    let 文本 = 摘要.trim();
    if 文本.is_empty() {
        return None;
    }
    // 优先解析 JSON 的 `"path": "..."` / `"file_path": "..."`。
    for 键 in ["\"file_path\"", "\"path\""] {
        if let Some(索引) = 文本.find(键) {
            let 剩余 = &文本[索引 + 键.len()..];
            if let Some(冒号) = 剩余.find(':') {
                let 值 = 剩余[冒号 + 1..].trim_start();
                if let Some(引号) = 值.find('"')
                    && let Some(结束) = 值[引号 + 1..].find('"')
                    && let Some(路径) = 规范化写路径(&值[引号 + 1..引号 + 1 + 结束], 项目根)
                {
                    return Some(路径);
                }
            }
        }
    }
    for 片段 in 文本.split(|字符: char| {
        字符.is_whitespace()
            || matches!(
                字符,
                ',' | ';' | '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}'
            )
    }) {
        if let Some(路径) = 规范化写路径(片段, 项目根) {
            return Some(路径);
        }
    }
    None
}

fn 规范化写路径(原始: &str, 项目根: Option<&Path>) -> Option<String> {
    let 去键值 = match 原始.find('=') {
        Some(位置) => &原始[位置 + 1..],
        None => 原始,
    };
    let 原文 = 去键值
        .trim()
        .trim_matches(|字符: char| matches!(字符, '"' | '\'' | '`' | ':' | ',' | ';'));
    let 原文路径 = Path::new(原文);
    let 字节 = 原文.as_bytes();
    if 原文路径.is_absolute() || (字节.len() >= 2 && 字节[1] == b':') {
        // 真实 DSH 给的是绝对路径：只有落在当前项目内才算证据。
        let 根 = 项目根?;
        let 相对 = 原文路径.strip_prefix(根).ok()?;
        let 文本 = 相对.to_string_lossy().replace('\\', "/");
        return 校验相对路径(&文本);
    }
    let 文本 = 原文.trim_start_matches("./").replace('\\', "/");
    // 连续分隔符归一化：JSON 里的 `src\\main.rs` 会变成 `src//main.rs`。
    let 文本 = 文本
        .split('/')
        .filter(|段| !段.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    校验相对路径(&文本)
}

fn 校验相对路径(文本: &str) -> Option<String> {
    if 文本.is_empty() || 文本.len() > 400 {
        return None;
    }
    let 字节 = 文本.as_bytes();
    if 文本.starts_with('/') || (字节.len() >= 2 && 字节[1] == b':') {
        return None;
    }
    if 文本.contains("..") || 文本.starts_with('-') {
        return None;
    }
    if 文本.starts_with("http:") || 文本.starts_with("https:") {
        return None;
    }
    let 有扩展 = 文本
        .rsplit('/')
        .next()
        .and_then(|名| 名.rsplit_once('.'))
        .is_some_and(|(_, 扩展)| {
            !扩展.is_empty()
                && 扩展.len() <= 8
                && 扩展.chars().all(|字符| 字符.is_ascii_alphanumeric())
        });
    if !文本.contains('/') && !有扩展 {
        return None;
    }
    Some(文本.to_string())
}

/// 按字符截断（避免把多字节字符切断）。
pub fn 截断(文本: &str, 上限: usize) -> String {
    let 字符数 = 文本.chars().count();
    if 字符数 <= 上限 {
        return 文本.to_string();
    }
    let 前缀: String = 文本.chars().take(上限).collect();
    format!("{前缀}…")
}

/// DSH 探测结果。
#[derive(Debug, Clone, PartialEq)]
pub struct 探测结果 {
    /// 连接状态（直接写进界面状态）。
    pub 连接: Dsh连接,
    /// 原始 stderr（失败时用于排查；不写入日志以免泄漏凭证）。
    pub 诊断: String,
}

/// 解析 `dsh --version` 的输出。
///
/// 真实输出就是一行版本号，但某些包装器会附加提示文本，因此取第一行非空内容。
pub fn 解析版本(输出: &str) -> Option<String> {
    输出
        .lines()
        .map(str::trim)
        .find(|行| !行.is_empty())
        .map(|行| 行.to_string())
}

/// 启动一个可执行文件所需的真实程序与前置参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 启动规格 {
    /// 真正交给 `CreateProcess` 的程序。
    pub 程序: String,
    /// 前置参数（`.cmd`/`.bat` 需要 `cmd /C`）。
    pub 前置参数: Vec<String>,
    /// 供界面/错误信息展示的真实路径。
    pub 展示: String,
}

/// 解析可执行文件，处理 Windows 上 `.cmd` / `.bat` 包装器。
///
/// # 为什么必须有这一层
///
/// 本机 DSH 安装成 `dsh.cmd`（Electron 的启动垫片）。Rust 的
/// `Command::new("dsh")` 只按 `.exe` 找，不会应用 `PATHEXT`，实测报
/// `program not found`；而直接 `Command::new("dsh.cmd")` 也不是合法
/// Win32 可执行文件。正确做法是找到 `.cmd` 后用 `cmd /C` 启动它。
pub fn 解析启动规格(输入: &str) -> 启动规格 {
    let 输入 = 输入.trim().trim_matches('"');
    if 输入.is_empty() {
        return 启动规格 {
            程序: "dsh".to_string(),
            前置参数: Vec::new(),
            展示: "dsh".to_string(),
        };
    }
    #[cfg(windows)]
    {
        let 有分隔符 = 输入.contains('\\') || 输入.contains('/');
        let 扩展 = std::path::Path::new(输入)
            .extension()
            .and_then(|扩展| 扩展.to_str())
            .map(|扩展| 扩展.to_ascii_lowercase());
        let 路径 = if 有分隔符 || 扩展.is_some() {
            Some(PathBuf::from(输入))
        } else {
            ["exe", "cmd", "bat"]
                .iter()
                .find_map(|扩展| 在路径中查找(输入, 扩展))
        };
        if let Some(路径) = 路径 {
            let 展示 = 路径.to_string_lossy().to_string();
            let 扩展 = 路径
                .extension()
                .and_then(|扩展| 扩展.to_str())
                .map(|扩展| 扩展.to_ascii_lowercase());
            if matches!(扩展.as_deref(), Some("cmd") | Some("bat")) {
                return 启动规格 {
                    程序: "cmd".to_string(),
                    前置参数: vec!["/C".to_string(), "call".to_string(), 展示.clone()],
                    展示,
                };
            }
            return 启动规格 {
                程序: 展示.clone(),
                前置参数: Vec::new(),
                展示,
            };
        }
    }
    启动规格 {
        程序: 输入.to_string(),
        前置参数: Vec::new(),
        展示: 输入.to_string(),
    }
}

fn 在路径中查找(名: &str, 扩展: &str) -> Option<PathBuf> {
    let 路径变量 = std::env::var_os("PATH")?;
    for 目录 in std::env::split_paths(&路径变量) {
        let 候选 = 目录.join(format!("{名}.{扩展}"));
        if 候选.is_file() {
            return Some(候选);
        }
    }
    None
}

/// 探测 DSH 是否可用。
///
/// 在**独立线程**里执行并带超时：`--version` 正常是毫秒级，但可执行文件损坏
/// 或卡在杀毒扫描时可能很久，绝不能阻塞编辑器启动。
pub fn 探测(可执行文件: &str, 超时: std::time::Duration) -> 探测结果 {
    let 可执行文件 = if 可执行文件.trim().is_empty() {
        "dsh".to_string()
    } else {
        可执行文件.trim().to_string()
    };
    let 规格 = 解析启动规格(&可执行文件);
    let (发送, 接收) = channel();
    let 待探测 = 规格.clone();
    std::thread::spawn(move || {
        let mut 命令 = Command::new(&待探测.程序);
        命令.args(&待探测.前置参数).arg("--version");
        let 结果 = 命令.stdin(Stdio::null()).output();
        let _ = 发送.send(结果);
    });
    let 展示路径 = 规格.展示.clone();
    match 接收.recv_timeout(超时) {
        Ok(Ok(输出)) => {
            let 标准输出 = String::from_utf8_lossy(&输出.stdout).to_string();
            let 标准错误 = String::from_utf8_lossy(&输出.stderr).to_string();
            if 输出.status.success() {
                match 解析版本(&标准输出) {
                    Some(版本) => 探测结果 {
                        连接: Dsh连接::就绪 {
                            版本,
                            路径: 展示路径.clone(),
                        },
                        诊断: 标准错误,
                    },
                    None => 探测结果 {
                        连接: Dsh连接::失败 {
                            错误: "版本输出为空".to_string(),
                        },
                        诊断: 标准错误,
                    },
                }
            } else {
                探测结果 {
                    连接: Dsh连接::失败 {
                        错误: format!("退出码 {}", 输出.status.code().unwrap_or(-1)),
                    },
                    诊断: 标准错误,
                }
            }
        }
        Ok(Err(错误)) => 探测结果 {
            连接: Dsh连接::未安装 {
                原因: format!("{错误}"),
            },
            诊断: String::new(),
        },
        Err(_) => 探测结果 {
            连接: Dsh连接::失败 {
                错误: "探测超时".to_string(),
            },
            诊断: String::new(),
        },
    }
}

/// 一次运行中的任务：子进程 + 事件接收端 + 退出状态。
pub struct 任务运行器 {
    子进程: Child,
    // mpsc::Receiver 不是 Sync，而 Bevy 的 Resource 要求 Send + Sync。
    // 用 Mutex 包一层即可：同一时刻只有一个线程收事件（主线程 轮询）。
    事件接收: std::sync::Mutex<Receiver<Dsh事件>>,
    已结束: bool,
    /// 退出码（进程结束后才有）。
    退出码: Option<i32>,
}

impl std::fmt::Debug for 任务运行器 {
    fn fmt(&self, 格式: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        格式
            .debug_struct("任务运行器")
            .field("pid", &self.子进程.id())
            .field("已结束", &self.已结束)
            .field("退出码", &self.退出码)
            .finish()
    }
}

impl 任务运行器 {
    /// 启动一次 headless 任务。
    ///
    /// * `工作目录` = 项目根目录：DSH 的工具看到的是**这个项目**，
    ///   因此切换项目不会让旧任务的执行结果混进来。
    /// * `会话标识` 非空时用 `--session-id` 续用 DSH 的持久化会话。
    pub fn 启动(
        可执行文件: &str,
        任务: &str,
        工作目录: &std::path::Path,
        会话标识: Option<&str>,
    ) -> Result<Self, String> {
        let 可执行文件 = if 可执行文件.trim().is_empty() {
            "dsh".to_string()
        } else {
            可执行文件.trim().to_string()
        };
        let 规格 = 解析启动规格(&可执行文件);
        let mut 命令 = Command::new(&规格.程序);
        命令.args(&规格.前置参数);
        命令.arg("headless").arg("--json");
        if let Some(标识) = 会话标识.map(str::trim).filter(|值| !值.is_empty()) {
            命令.arg("--session-id").arg(标识);
        }
        命令.arg(任务);
        命令
            .current_dir(工作目录)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut 子进程 = 命令
            .spawn()
            .map_err(|错误| format!("启动 {} 失败：{错误}", 规格.展示))?;

        let (发送, 接收) = channel();
        if let Some(标准输出) = 子进程.stdout.take() {
            let 发送 = 发送.clone();
            std::thread::spawn(move || {
                let 读 = BufReader::new(标准输出);
                for 行 in 读.lines() {
                    match 行 {
                        Ok(行) => {
                            if let Some(事件) = 解析事件(&行) {
                                let _ = 发送.send(事件);
                            }
                        }
                        Err(错误) => {
                            let _ = 发送.send(Dsh事件::读取错误 {
                                错误: 错误.to_string(),
                            });
                            return;
                        }
                    }
                }
            });
        }
        if let Some(标准错误) = 子进程.stderr.take() {
            let 发送 = 发送.clone();
            std::thread::spawn(move || {
                let 读 = BufReader::new(标准错误);
                for 行 in 读.lines().map_while(Result::ok) {
                    if !行.trim().is_empty() {
                        let _ = 发送.send(Dsh事件::未知 {
                            原文: 截断(&行, 400),
                        });
                    }
                }
            });
        }

        Ok(Self {
            子进程,
            事件接收: std::sync::Mutex::new(接收),
            已结束: false,
            退出码: None,
        })
    }

    /// 非阻塞地收一帧内到达的事件。
    pub fn 轮询(&mut self) -> Vec<Dsh事件> {
        let mut 结果 = Vec::new();
        let 接收端 = match self.事件接收.lock() {
            Ok(锁) => 锁,
            Err(_) => return 结果,
        };
        loop {
            match 接收端.try_recv() {
                Ok(事件) => 结果.push(事件),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        结果
    }

    /// 是否仍在运行（会顺带检查进程是否已退出）。
    pub fn 运行中(&mut self) -> bool {
        if self.已结束 {
            return false;
        }
        match self.子进程.try_wait() {
            Ok(Some(状态)) => {
                self.退出码 = 状态.code();
                self.已结束 = true;
                false
            }
            Ok(None) => true,
            Err(_) => {
                self.已结束 = true;
                false
            }
        }
    }

    /// 退出码。
    pub fn 退出码(&self) -> Option<i32> {
        self.退出码
    }

    /// 是否进程已结束（且没有拿到退出码 —— 即状态未知）。
    pub fn 状态未知(&mut self) -> bool {
        !self.运行中() && self.退出码.is_none()
    }

    /// 取消：真实终止子进程。
    ///
    /// 返回 `true` 表示终止动作已发出。**不代表**副作用被撤销 ——
    /// 调用方必须如实告诉用户"已取消"，而不是"已回滚"。
    pub fn 取消(&mut self) -> bool {
        if self.已结束 {
            return false;
        }
        // Windows 上连同进程树一起结束：DSH 可能已经起了工具子进程。
        crate::process_tree::终止进程树(&mut self.子进程);
        let _ = self.子进程.wait();
        self.已结束 = true;
        self.退出码 = None;
        true
    }
}

/// 依据进程结果判定任务状态。
///
/// 真实规则：
/// * 收到 `final` 且退出码 0 → 成功；
/// * 退出码非 0 → 失败；
/// * 进程结束但没有退出码 → 状态未知（不能假装成功）。
pub fn 判定状态(收到最终结果: bool, 退出码: Option<i32>) -> 任务状态 {
    match (收到最终结果, 退出码) {
        (true, Some(0)) => 任务状态::成功,
        (_, Some(0)) => 任务状态::失败,
        (_, Some(_)) => 任务状态::失败,
        (_, None) => 任务状态::状态未知,
    }
}

/// 把工具事件整理成界面用的记录。
pub fn 记录工具调用(事件: &Dsh事件) -> Option<工具调用> {
    match 事件 {
        Dsh事件::工具 {
            名称,
            阶段,
            摘要,
            写操作,
        } => Some(工具调用 {
            名称: 名称.clone(),
            阶段: 阶段.clone(),
            摘要: 摘要.clone(),
            写操作: *写操作,
        }),
        _ => None,
    }
}

/// 默认配置目录下的会话文件路径（与编辑器其它配置同目录）。
pub fn 会话存储路径(配置目录: &std::path::Path) -> PathBuf {
    editor_ui::agent::会话文件路径(配置目录)
}

/// 项目根目录探测（用于把任务的工作目录固定到当前项目）。
pub fn 项目根(项目: Option<&editor_core::project::项目描述>) -> Option<PathBuf> {
    项目.map(|项目| 项目.根目录().to_path_buf())
}

#[cfg(test)]
mod 测试 {
    use super::*;

    // 下面这些 JSON 是**实跑 `dsh headless --json` 抓下来的原文**，
    // 不是按文档臆造的样例。
    const 真实事件: [&str; 8] = [
        r#"{"type":"session","sessionId":"session-62c1bfa3-6521-46b0-93eb-caeb6f908270","cwd":"C:\\Users\\DianRan\\AppData\\Local\\Temp\\dsh_probe"}"#,
        r#"{"type":"status","phase":"turn_start","turn":1}"#,
        r#"{"type":"status","phase":"step_start","turn":1,"step":1}"#,
        r#"{"type":"thinking","text":""}"#,
        r#"{"type":"text","text":"ok"}"#,
        r#"{"type":"status","phase":"step_end","turn":1,"step":1,"usage":{"inputTokens":8799,"outputTokens":2,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":8801}}"#,
        r#"{"type":"status","phase":"turn_end","turn":1,"reason":{"kind":"completed"}}"#,
        r#"{"type":"final","text":"ok"}"#,
    ];

    #[test]
    fn 能解析真实抓取的会话事件() {
        let 事件: Vec<Dsh事件> = 真实事件.iter().filter_map(|行| 解析事件(行)).collect();
        assert_eq!(事件.len(), 8, "8 行真实输出都应当被识别");
        assert_eq!(
            事件[0],
            Dsh事件::会话 {
                会话标识: "session-62c1bfa3-6521-46b0-93eb-caeb6f908270".to_string(),
                工作目录: r"C:\Users\DianRan\AppData\Local\Temp\dsh_probe".to_string(),
            }
        );
        assert_eq!(
            事件[5],
            Dsh事件::阶段 {
                阶段: "step_end".to_string(),
                轮次: Some(1),
                步: Some(1),
            }
        );
        assert_eq!(
            事件[7],
            Dsh事件::最终 {
                文本: "ok".to_string()
            }
        );
    }

    #[test]
    fn 空行与非结构化行被忽略() {
        assert!(解析事件("").is_none());
        assert!(解析事件("   ").is_none());
        assert!(解析事件("not json").is_none());
    }

    #[test]
    fn 未知类型保留原文() {
        let 事件 = 解析事件(r#"{"type":"mystery","x":1}"#).unwrap();
        assert!(matches!(事件, Dsh事件::未知 { .. }));
    }

    #[test]
    fn 工具事件能识别写操作() {
        let 事件 = 解析事件(
            r#"{"type":"tool_call","name":"write_file","phase":"start","summary":"src/main.rs"}"#,
        )
        .unwrap();
        let 记录 = 记录工具调用(&事件).expect("应当转成工具调用记录");
        assert_eq!(记录.名称, "write_file");
        assert!(记录.写操作, "写文件的工具必须被标记为写操作");
        assert_eq!(记录.摘要, "src/main.rs");
    }

    #[test]
    fn 状态判定只依据真实退出码与最终事件() {
        assert_eq!(判定状态(true, Some(0)), 任务状态::成功);
        assert_eq!(
            判定状态(false, Some(0)),
            任务状态::失败,
            "没有 final 不算成功"
        );
        assert_eq!(判定状态(true, Some(101)), 任务状态::失败);
        assert_eq!(判定状态(false, None), 任务状态::状态未知);
    }

    #[test]
    fn 版本解析取第一行非空() {
        assert_eq!(解析版本("0.2.0-rc.2\n").as_deref(), Some("0.2.0-rc.2"));
        assert_eq!(解析版本("\n\n 1.0.0 \n").as_deref(), Some("1.0.0"));
        assert_eq!(解析版本("   "), None);
    }

    #[test]
    fn 截断按字符而不是字节() {
        assert_eq!(截断("中文中文中文", 3), "中文中…");
        assert_eq!(截断("abc", 3), "abc");
    }

    #[test]
    fn 探测不存在的可执行文件报未安装() {
        let 结果 = 探测(
            "definitely-not-a-real-dsh-binary-xyz",
            std::time::Duration::from_secs(5),
        );
        assert!(matches!(结果.连接, Dsh连接::未安装 { .. }));
    }

    #[test]
    fn 写路径能从常见摘要里提取() {
        assert_eq!(
            提取写路径("src/main.rs", None).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            提取写路径("write_file: crates/game/src/lib.rs", None).as_deref(),
            Some("crates/game/src/lib.rs")
        );
        assert_eq!(
            提取写路径(r#"{"path":"src\\main.rs","writes":true}"#, None).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            提取写路径("./Cargo.toml", None).as_deref(),
            Some("Cargo.toml")
        );
    }

    #[test]
    fn 写路径拒绝越界与噪声() {
        assert_eq!(提取写路径("../outside.rs", None), None);
        assert_eq!(提取写路径("C:\\Windows\\system32\\x.rs", None), None);
        assert_eq!(提取写路径("https://example.com/a.rs", None), None);
        assert_eq!(提取写路径("writes=true", None), None);
        assert_eq!(提取写路径("", None), None);
        assert_eq!(提取写路径("edit", None), None);
    }

    #[test]
    fn cmd包装器会通过命令解释器启动() {
        let 目录 = tempfile::tempdir().unwrap();
        let 垫片 = 目录.path().join("probe.cmd");
        std::fs::write(&垫片, "@echo off\r\n").unwrap();
        let 规格 = 解析启动规格(垫片.to_str().unwrap());
        if cfg!(windows) {
            assert_eq!(规格.程序, "cmd", "必须用 cmd /C 启动 .cmd 垫片");
            assert_eq!(规格.前置参数.first().map(String::as_str), Some("/C"));
            assert!(规格.展示.ends_with("probe.cmd"));
        } else {
            assert_eq!(规格.程序, 垫片.to_string_lossy());
        }
    }

    #[test]
    fn 裸命令名会解析成可用的启动规格() {
        let 规格 = 解析启动规格("dsh");
        assert!(!规格.展示.is_empty());
        // 这台机器上 DSH 是 .cmd 垫片；解析后程序应是 cmd，而不是裸 "dsh"。
        if cfg!(windows) && 规格.展示.to_ascii_lowercase().ends_with(".cmd") {
            assert_eq!(规格.程序, "cmd");
        }
    }

    #[test]
    fn 真实tool_call的绝对路径能落到项目内() {
        // 实跑抓取的真实事件（路径换成示例根目录）。
        let 行 = r#"{"type":"tool_call","callId":"call_00_x","tool":"edit","input":{"file_path":"C:\\proj\\src\\main.rs","old_string":"a","new_string":"b"}}"#;
        let 事件 = 解析事件(行).expect("应解析 tool_call");
        let 调用 = 记录工具调用(&事件).expect("应转成工具调用");
        assert_eq!(调用.名称, "edit");
        assert!(调用.写操作, "edit 必须标记为写操作");
        assert_eq!(
            提取写路径(&调用.摘要, Some(std::path::Path::new("C:\\proj"))).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            提取写路径(&调用.摘要, Some(std::path::Path::new("C:\\other"))),
            None,
            "项目外的绝对路径不能算成本项目证据"
        );
    }

    #[test]
    fn tool_result不会把文件内容当成路径() {
        let 行 = r#"{"type":"tool_result","callId":"call_00_x","status":"completed","result":"The file C:\\proj\\src\\main.rs has been updated"}"#;
        let 事件 = 解析事件(行).expect("应解析 tool_result");
        let 调用 = 记录工具调用(&事件).expect("应转成工具调用");
        assert_eq!(调用.摘要, "completed");
        assert!(!调用.写操作, "tool_result 本身不是写操作");
    }
}
