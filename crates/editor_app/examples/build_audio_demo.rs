//! 中文名：构建音频演示项目
//! 生成一个"带音频"的演示项目，供 GUI 验收与手工试用。
//!
//! 运行：
//!
//! ```powershell
//! cargo run -p editor_app --example build_audio_demo
//! ```
//!
//! 生成的项目包含 `assets/audio/beep.wav`（440Hz 正弦，6 秒（足够长，验收截图时它还在播））
//! 与一个挂了 `音频` 组件的实体。右键资源树里的 wav 就能试听。
//!
//! # 为什么要自己写 WAV 字节
//!
//! 试听功能必须有一份**真实可解码**的音频文件才能验证（假的空文件会让
//! `AssetServer` 报错，反而掩盖问题）。WAV 的 RIFF 头只有 44 字节，
//! 用代码生成比往仓库里塞二进制更好：体积、时长、频率都写在代码里，
//! 谁都能看懂它是什么。

use editor_core::project::项目描述;
use scene_doc::document::{变换数据, 场景文档, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 枚举值, 资源引用值, 颜色值};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let 目标 = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".verify/runs/audio_demo".to_string());
    let 根 = std::path::PathBuf::from(&目标);
    if 根.exists() {
        std::fs::remove_dir_all(&根)?;
    }
    if let Some(父) = 根.parent() {
        std::fs::create_dir_all(父)?;
    }
    let mut 项目 = 项目描述::创建(&根, "音频演示")?;

    // 1) 真实音频文件。
    let 音频路径 = 项目相对路径::解析("assets/audio/beep.wav")?;
    let 音频绝对 = 根.join(音频路径.文本());
    if let Some(父) = 音频绝对.parent() {
        std::fs::create_dir_all(父)?;
    }
    std::fs::write(&音频绝对, 正弦波_wav(440.0, 6.0))?;

    // 2) 场景：一个可见的箱子 + 一个挂音频的实体。
    let mut 文档 = 场景文档::新建("audio");
    let 箱子 = 文档.生成实体("箱子", None);
    if let Some(实体) = 文档.取可变(箱子) {
        实体.变换 = Some(变换数据::单位());
        实体.加组件(组件数据::新建(
            "Primitive3d",
            [
                ("形状".to_string(), 属性值::枚举(枚举值::新建("Cuboid", 0))),
                ("尺寸".to_string(), 属性值::三维(1.5, 1.5, 1.5)),
                (
                    "颜色".to_string(),
                    属性值::颜色(颜色值::srgb(0.85, 0.7, 0.45, 1.0)),
                ),
                ("受光".to_string(), 属性值::布尔(true)),
                ("双面".to_string(), 属性值::布尔(false)),
                ("贴图重复".to_string(), 属性值::二维(1.0, 1.0)),
            ]
            .into_iter()
            .collect(),
        ));
    }
    let 喇叭 = 文档.生成实体("喇叭", None);
    if let Some(实体) = 文档.取可变(喇叭) {
        实体.变换 = Some(变换数据 {
            平移: (2.0, 0.0, 0.0),
            ..变换数据::单位()
        });
        实体.加组件(组件数据::新建(
            "音频",
            [
                (
                    "音频".to_string(),
                    属性值::资源(资源引用值::按路径(音频路径.clone())),
                ),
                ("音量".to_string(), 属性值::浮点(0.8)),
                ("自动播放".to_string(), 属性值::布尔(false)),
                ("空间化".to_string(), 属性值::布尔(true)),
            ]
            .into_iter()
            .collect(),
        ));
    }

    // 把组件里的资源引用同步进资源表：否则打开场景会报
    // "1 个资源引用未登记"（依赖检查与打包也会漏掉这个音频）。
    文档.同步资源表();

    let 场景路径 = 项目相对路径::解析("scenes/audio.scene.ron")?;
    项目.保存场景(&场景路径, &文档)?;
    项目.清单可变().启动场景 = Some(场景路径);
    项目.保存清单()?;

    println!("已生成音频演示项目：{}", 根.display());
    println!(
        "打开方式：$env:BEVY_STUDIO_OPEN_PROJECT=\"{}\"; $env:BEVY_STUDIO_AUDITION=\"assets/audio/beep.wav\"",
        根.display()
    );
    Ok(())
}

/// 生成一段单声道 16 位 PCM 的正弦波 WAV。
///
/// 采样率 44100；时长 = 秒数；振幅取半量程，避免削波。
fn 正弦波_wav(频率: f32, 时长秒: f32) -> Vec<u8> {
    let 采样率 = 44_100u32;
    let 样本数 = (采样率 as f32 * 时长秒) as u32;
    let 数据长度 = 样本数 * 2;
    let mut 字节 = Vec::with_capacity(44 + 数据长度 as usize);

    // RIFF 头。
    字节.extend_from_slice(b"RIFF");
    字节.extend_from_slice(&(36 + 数据长度).to_le_bytes());
    字节.extend_from_slice(b"WAVE");
    // fmt 块。
    字节.extend_from_slice(b"fmt ");
    字节.extend_from_slice(&16u32.to_le_bytes());
    字节.extend_from_slice(&1u16.to_le_bytes()); // PCM
    字节.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    字节.extend_from_slice(&采样率.to_le_bytes());
    字节.extend_from_slice(&(采样率 * 2).to_le_bytes()); // 字节率
    字节.extend_from_slice(&2u16.to_le_bytes()); // 块对齐
    字节.extend_from_slice(&16u16.to_le_bytes()); // 位深
    // data 块。
    字节.extend_from_slice(b"data");
    字节.extend_from_slice(&数据长度.to_le_bytes());
    for 序号 in 0..样本数 {
        let 相位 = 2.0 * std::f32::consts::PI * 频率 * 序号 as f32 / 采样率 as f32;
        // 两端各做 20ms 的淡入淡出：直接截断会在开始/结束处产生"啪"声。
        let 时间 = 序号 as f32 / 采样率 as f32;
        let 包络 = (时间 / 0.02).min(1.0) * ((时长秒 - 时间) / 0.02).clamp(0.0, 1.0);
        let 样本 = (相位.sin() * 0.5 * 包络 * i16::MAX as f32) as i16;
        字节.extend_from_slice(&样本.to_le_bytes());
    }
    字节
}
