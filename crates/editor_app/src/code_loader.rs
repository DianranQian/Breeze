// SPDX-License-Identifier: MIT OR Apache-2.0
//! 中文名：后台读取代码文件
//!
//! 把"打开一个代码文件"拆成两步：动作只声明**意图**，读盘在后台线程完成。
//!
//! # 为什么读盘不能留在动作里
//!
//! `UiAction::打开脚本` 是在主线程的 `Update` 里执行的，而它背后可能是
//! 一个几 MB 的文件、一块被索引服务占着的磁盘、甚至是断开的网络盘符。
//! 同步 `read_to_string` 会让整帧卡住：窗口不重绘、输入堆积、连"正在
//! 打开"的提示都画不出来 —— 用户看到的是**编辑器假死**，而不是"文件
//! 大了一点"。这与 `文件对话框通道` 的结论是同一条：主线程只做调度。
//!
//! # 为什么用代次而不是"只认最后一次路径"
//!
//! 用户连续双击两个文件时，两次读盘的完成顺序**不保证**与请求顺序一致
//! （磁盘缓存、文件大小都会影响）。如果只比较路径，先发后到的 A 会覆盖
//! 后发先到的 B；如果只认"最近一次结果"，同样无法分辨。因此每次请求
//! 带一个单调递增的代次，回执带回同一个代次，界面只接受等于当前代次的
//! 那一份。这与资源预览的请求代次是同一套约定。
//!
//! # 为什么回执里带线程标识
//!
//! "读取不阻塞主线程"这件事，用计时去验证会随机器抖动；而"读盘发生在
//! 哪条线程上"是确定的。回执里记下真正执行 `read_to_string` 的线程，
//! 测试把它与主线程比较即可 —— 这条断言不依赖任何时序假设。

use std::path::PathBuf;

use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use scene_doc::path::项目相对路径;

use editor_core::state::日志参数;
use editor_ui::开发模式;

use crate::config::布局待保存;
use crate::plugin::{EditorStateHolder, UiStateHolder};

/// 一次读盘请求。
#[derive(Debug, Clone)]
pub struct 代码读取请求 {
    /// 请求代次（界面按它丢弃过期回执）。
    pub 代次: u64,
    /// 项目相对路径（回执原样带回）。
    pub 路径: 项目相对路径,
    /// 已解析成绝对路径的落点。
    pub 绝对路径: PathBuf,
}

/// 一次读盘的完成回执。
#[derive(Debug)]
pub struct 代码读取完成 {
    /// 与请求相同的代次。
    pub 代次: u64,
    /// 项目相对路径。
    pub 路径: 项目相对路径,
    /// 读到的文本，或"读不到"的可读原因。
    pub 结果: Result<String, String>,
    /// 真正执行读盘的线程。
    pub 读取线程: std::thread::ThreadId,
}

/// 代码读取的后台通道。
///
/// 线程在第一次构造资源时启动，`World` 析构后请求端断开，工作线程
/// 自然退出 —— 不需要显式的"停止"接口。
#[derive(Resource)]
pub struct 代码读取通道 {
    /// 发送读盘请求。
    pub 请求发送端: Sender<代码读取请求>,
    /// 接收完成回执。
    pub 结果接收端: Receiver<代码读取完成>,
    /// 最近一次完成读盘的线程（诊断与"读取不占主线程"的测试依据）。
    pub 最近读取线程: Option<std::thread::ThreadId>,
}

impl Default for 代码读取通道 {
    fn default() -> Self {
        let (请求发送端, 请求接收端) = unbounded();
        let (结果发送端, 结果接收端) = unbounded();
        std::thread::Builder::new()
            .name("bevy-studio-代码读取".to_string())
            .spawn(move || 读取工作线程(请求接收端, 结果发送端))
            .expect("应当能启动代码读取线程");
        Self {
            请求发送端,
            结果接收端,
            最近读取线程: None,
        }
    }
}

/// 后台线程：等待请求 → 读盘 → 回传结果。
fn 读取工作线程(
    请求接收端: Receiver<代码读取请求>, 结果发送端: Sender<代码读取完成>
) {
    while let Ok(请求) = 请求接收端.recv() {
        let 结果 = std::fs::read_to_string(&请求.绝对路径).map_err(|错误| 错误.to_string());
        let 回执 = 代码读取完成 {
            代次: 请求.代次,
            路径: 请求.路径,
            结果,
            读取线程: std::thread::current().id(),
        };
        if 结果发送端.send(回执).is_err() {
            // 主线程已退出，结束线程。
            break;
        }
    }
}

/// 把界面里"待读取"的意图交给后台线程。
///
/// 排在本帧动作执行之后：用户双击文件产生的意图当帧就发出去，不需要
/// 等到下一帧 —— 否则每次打开都会多一帧的延迟，连续点击时更明显。
pub fn 调度代码读取(
    通道: Res<代码读取通道>,
    mut 界面持有: ResMut<UiStateHolder>,
    编辑器持有: Res<EditorStateHolder>,
) {
    let 界面 = &mut 界面持有.状态;
    let Some(路径) = 界面.脚本.待读取.take() else {
        return;
    };
    let Some(项目) = 编辑器持有.状态.项目.as_ref() else {
        // 动作层已经在没有项目时警告过；这里只保证不会永远显示"正在读取"。
        界面.脚本.加载中 = None;
        return;
    };
    let 请求 = 代码读取请求 {
        代次: 界面.脚本.加载代次,
        路径: 路径.clone(),
        绝对路径: 项目.绝对路径(&路径),
    };
    if 通道.请求发送端.send(请求).is_err() {
        界面.脚本.加载中 = None;
    }
}

/// 收下后台线程的回执，把文本装进代码面板。
///
/// 过期回执（代次不等于当前请求）直接丢弃：用户已经点了别的文件，
/// 把旧文件装进去会让"最后点的那个"反而打不开。
pub fn 轮询代码读取(
    mut 通道: ResMut<代码读取通道>,
    mut 界面持有: ResMut<UiStateHolder>,
    mut 编辑器持有: ResMut<EditorStateHolder>,
    mut 布局待保存: ResMut<布局待保存>,
) {
    let 回执们: Vec<代码读取完成> = 通道.结果接收端.try_iter().collect();
    if 回执们.is_empty() {
        return;
    }
    let 界面 = &mut 界面持有.状态;
    for 回执 in 回执们 {
        通道.最近读取线程 = Some(回执.读取线程);
        if 回执.代次 != 界面.脚本.加载代次 {
            continue;
        }
        界面.脚本.加载中 = None;
        match 回执.结果 {
            Ok(文本) => {
                let 语言 = 界面.语言;
                // 打开新文件会替换掉当前文档；有未保存修改时必须说出来，
                // 而不是让用户的编辑无声消失。
                if 界面
                    .脚本
                    .文档
                    .as_ref()
                    .is_some_and(|旧文档| 旧文档.有未保存修改())
                {
                    编辑器持有
                        .状态
                        .警告(语言.填充("script_discarded", &[&回执.路径.文本()]));
                }
                界面.脚本.打开(editor_ui::script::脚本文档::新建(
                    Some(回执.路径.clone()),
                    文本,
                ));
                // 打开代码 = 进入代码模式并聚焦代码面板，与"点诊断跳转"一致。
                let _ = 界面.切换模式(开发模式::脚本);
                界面.聚焦面板(editor_ui::Tab::脚本);
                布局待保存.脏 = true;
                编辑器持有.状态.结构化信息(
                    "log_script_opened",
                    vec![日志参数::用户数据(回执.路径.文本().to_string())],
                );
            }
            Err(错误) => {
                let 语言 = 界面.语言;
                编辑器持有
                    .状态
                    .错误(语言.填充("script_open_failed", &[&错误]));
            }
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 工作线程必须真的在另一条线程上读盘。
    ///
    /// 这里直接驱动通道（不经 Bevy 世界），把"读盘不在调用线程上"这条
    /// 性质钉在最底层；集成测试再从动作队列那一端验证一次。
    #[test]
    fn 读盘发生在工作线程而不是调用线程() {
        let 通道 = 代码读取通道::default();
        let 目录 = tempfile::tempdir().expect("创建临时目录");
        let 绝对 = 目录.path().join("main.rs");
        std::fs::write(&绝对, "fn main() {}\n").expect("写测试文件");

        通道
            .请求发送端
            .send(代码读取请求 {
                代次: 1,
                路径: 项目相对路径::解析("src/main.rs").expect("合法路径"),
                绝对路径: 绝对,
            })
            .expect("应当能发出请求");

        let 回执 = 通道
            .结果接收端
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("工作线程应当回执");
        assert_eq!(回执.代次, 1);
        assert!(
            回执
                .结果
                .as_ref()
                .expect("读取应当成功")
                .contains("fn main")
        );
        assert_ne!(
            回执.读取线程,
            std::thread::current().id(),
            "读盘必须发生在工作线程：在调用线程上就是用主线程在读"
        );
    }

    /// 读不到的文件回执成可读原因，而不是 panic 或空文本。
    #[test]
    fn 读不到的文件回执错误原因() {
        let 通道 = 代码读取通道::default();
        通道
            .请求发送端
            .send(代码读取请求 {
                代次: 7,
                路径: 项目相对路径::解析("src/不存在.rs").expect("合法路径"),
                绝对路径: PathBuf::from("C:/绝对不存在的目录/不存在.rs"),
            })
            .expect("应当能发出请求");

        let 回执 = 通道
            .结果接收端
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("工作线程应当回执");
        assert_eq!(回执.代次, 7);
        assert!(回执.结果.is_err(), "读不到必须是错误回执");
    }
}
