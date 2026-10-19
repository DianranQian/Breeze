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
use std::path::PathBuf;
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
            let 摘要 = {
                let 摘要 = 取串("summary");
                if 摘要.is_empty() {
                    取串("input")
                } else {
                    摘要
                }
            };
            let 写操作 = 取串("writes").contains("true")
                || 取串("write").contains("true")
                || 名称.contains("write")
                || 名称.contains("edit")
                || 名称.contains("apply");
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
    let (发送, 接收) = channel();
    let 待探测 = 可执行文件.clone();
    std::thread::spawn(move || {
        let 结果 = Command::new(&待探测)
            .arg("--version")
            .stdin(Stdio::null())
            .output();
        let _ = 发送.send(结果);
    });
    match 接收.recv_timeout(超时) {
        Ok(Ok(输出)) => {
            let 标准输出 = String::from_utf8_lossy(&输出.stdout).to_string();
            let 标准错误 = String::from_utf8_lossy(&输出.stderr).to_string();
            if 输出.status.success() {
                match 解析版本(&标准输出) {
                    Some(版本) => 探测结果 {
                        连接: Dsh连接::就绪 {
                            版本,
                            路径: 可执行文件,
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
        let mut 命令 = Command::new(&可执行文件);
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
            .map_err(|错误| format!("启动 {可执行文件} 失败：{错误}"))?;

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
        let 结果 = self.子进程.kill().is_ok();
        let _ = self.子进程.wait();
        self.已结束 = true;
        self.退出码 = None;
        结果
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
    fn 能解析真实抓取的DSH事件() {
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
    fn 空行与非JSON行被忽略() {
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
}
