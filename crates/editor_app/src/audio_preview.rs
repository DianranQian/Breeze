//! 音频试听：把项目里的音频文件在编辑器内真实播放一遍。
//!
//! # 参考 Godot
//!
//! Godot 的 FileSystem 停靠面板对音频文件提供播放/停止按钮
//! （`editor/audio/` + `AudioStreamPreviewGenerator`），美术与策划不用启动游戏
//! 就能确认"这个音效是不是我要的那个"。这是音频工具链里最常用的一环，
//! 因此先做它，而不是先做运行期播放器。
//!
//! # 为什么不在预览里自动播放
//!
//! 编辑器的 `音频` 组件是**数据**：它告诉游戏"这个实体带什么音效"。
//! 自动播放会让一堆音效在打开场景时同时响起来，那不是预览，那是噪音。
//! 因此播放只在用户显式点「试听」时发生。
//!
//! # 状态机
//!
//! 用户请求放在界面状态里（[`试听请求`]），由本模块的系统消费并清空；
//! 播放实体由 [`试听状态`] 记录。播放自然结束时 Bevy 会把实体 despawn 掉
//! （`PlaybackMode::Despawn`），系统据此把"正在试听"的显示清掉 ——
//! 否则按钮会永远停在"停止"状态，用户点它也没反应。

use bevy::audio::{AudioPlayer, PlaybackMode, PlaybackSettings, Volume};
use bevy::prelude::*;

use editor_ui::state::{决定试听动作, 试听动作, 试听请求};
use scene_doc::path::项目相对路径;
use scene_doc::property::属性值;

use crate::plugin::EditorStateHolder;
use crate::plugin::UiStateHolder;

/// 试听实体标记。
#[derive(Component)]
pub struct 试听实体;

/// 试听运行时状态。
#[derive(Resource, Default)]
pub struct 试听状态 {
    /// 当前正在播的实体。
    pub 当前实体: Option<Entity>,
    /// 当前正在播的路径（用于界面显示）。
    pub 当前路径: Option<String>,
}

/// 判断扩展名是否是编辑器能试听的音频格式。
///
/// # 为什么只认两种
///
/// Bevy 0.20 的 `audio` 特性**不带任何解码器**，格式要单独开
/// （`Cargo.toml` 里我们只开了 `wav` 与 `vorbis`）。喂一个没编进来的格式，
/// `AudioSource` 会在音频线程里 panic（`UnrecognizedFormat`）——
/// 实测就是"点一下试听，编辑器整个崩掉"。
/// 因此这里只放行真正能解码的两种，其余格式由界面明确拒绝并写日志。
pub fn 可试听(路径: &项目相对路径) -> bool {
    let 文本 = 路径.文本().to_ascii_lowercase();
    [".wav", ".ogg"].iter().any(|扩展| 文本.ends_with(扩展))
}

/// 把项目相对路径解析成绝对路径。
///
/// # 为什么需要它
///
/// 试听要把文件读进内存（见 [`处理试听请求`] 的注释：`AssetServer` 会把
/// 绝对路径拼到编辑器自己的资源根后面）。读文件必须有绝对路径。
pub fn 音频绝对路径(
    项目根: &std::path::Path, 路径: &项目相对路径
) -> std::path::PathBuf {
    项目根.join(路径.文本())
}

/// 从实体的 `音频` 组件里读出音频路径与音量。
///
/// 返回 `(路径, 音量)`；没有组件或路径字段为空时返回 `None`。
pub fn 实体音频(
    实体: &scene_doc::document::场景实体
) -> Option<(项目相对路径, f32)> {
    let 组件 = 实体.取组件(crate::audio_components::音频组件名)?;
    let 路径 = match 组件.属性.get("音频") {
        Some(属性值::资源(引用)) => Some(引用.路径.clone()),
        _ => None,
    }?;
    let 音量 = 组件
        .属性
        .get("音量")
        .and_then(属性值::取浮点)
        .unwrap_or(1.0)
        .clamp(0.0, 2.0);
    Some((路径, 音量))
}

/// 处理试听请求：把界面状态里的请求变成真实播放。
///
/// 参数里没有 `Query` 之外的借用冲突：界面状态与试听状态是两个独立资源。
pub fn 处理试听请求(
    mut 命令: Commands,
    mut 编辑器: ResMut<EditorStateHolder>,
    mut 界面: ResMut<UiStateHolder>,
    mut 状态: ResMut<试听状态>,
    mut 音频资源: ResMut<Assets<AudioSource>>,
    已有: Query<Entity, With<试听实体>>,
) {
    // 播放自然结束：实体已被 `PlaybackMode::Despawn` 回收，
    // 界面上的"正在试听"必须跟着清掉。
    if let Some(实体) = 状态.当前实体
        && 已有.get(实体).is_err()
    {
        状态.当前实体 = None;
        状态.当前路径 = None;
        界面.状态.试听中 = None;
    }

    let 请求 = 界面.状态.试听请求.clone();
    if 请求 == 试听请求::default() {
        return;
    }
    let 动作 = 决定试听动作(&请求, 状态.当前路径.as_deref());
    界面.状态.试听请求 = 试听请求::default();

    match 动作 {
        试听动作::无 => {}
        试听动作::停止 => {
            for 实体 in &已有 {
                命令.entity(实体).despawn();
            }
            状态.当前实体 = None;
            状态.当前路径 = None;
            界面.状态.试听中 = None;
            编辑器.状态.结构化信息("log_audition_stopped", Vec::new());
        }
        试听动作::播放(路径) => {
            // 直接读字节再插入 `Assets<AudioSource>`，而不是 `AssetServer::load`。
            //
            // 原因（实测踩过）：预览世界的资源根是**编辑器自己的** `assets/`，
            // 用户项目在任意目录。`load(绝对路径)` 会被拼到资源根后面，
            // 日志里是 `.../target/debug/assets/C:/proj/assets/x.wav` ——
            // 加载失败，界面却显示"正在试听"。读字节没有这个歧义。
            let 绝对 = match 编辑器.状态.项目.as_ref() {
                Some(项目) => 音频绝对路径(项目.根目录(), &路径),
                None => {
                    编辑器
                        .状态
                        .警告(界面.状态.语言.取("log_no_project").to_string());
                    return;
                }
            };
            let 字节 = match std::fs::read(&绝对) {
                Ok(字节) => 字节,
                Err(错误) => {
                    // 读不到就不进"正在试听"：宁可什么都不显示，
                    // 也不让界面替一个没播的声音作证。
                    编辑器
                        .状态
                        .错误(format!("读取音频失败：{}：{错误}", 绝对.display()));
                    界面.状态.试听中 = None;
                    return;
                }
            };
            for 实体 in &已有 {
                命令.entity(实体).despawn();
            }
            let 句柄 = 音频资源.add(AudioSource {
                bytes: 字节.into()
            });
            let 实体 = 命令
                .spawn((
                    试听实体,
                    AudioPlayer(句柄),
                    PlaybackSettings {
                        // 播完即回收：否则用户每试听一次都会多一个静默实体。
                        mode: PlaybackMode::Despawn,
                        volume: Volume::Linear(1.0),
                        ..default()
                    },
                ))
                .id();
            状态.当前实体 = Some(实体);
            状态.当前路径 = Some(路径.文本().to_string());
            界面.状态.试听中 = Some(路径.文本().to_string());
            编辑器.状态.结构化信息(
                "log_audition_started",
                vec![editor_core::state::日志参数::用户数据(
                    路径.文本().to_string(),
                )],
            );
        }
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 只识别音频扩展名() {
        for 路径 in ["assets/audio/beep.wav", "assets/audio/bgm.OGG"] {
            assert!(
                可试听(&项目相对路径::解析(路径).unwrap()),
                "{路径} 应当可试听"
            );
        }
        // 没编进来的格式必须被拒绝：放行会让音频线程 panic。
        for 路径 in [
            "assets/audio/readme.txt",
            "assets/tex/a.png",
            "scenes/a.scene.ron",
            "assets/audio/x.flac",
            "assets/audio/y.mp3",
        ] {
            assert!(
                !可试听(&项目相对路径::解析(路径).unwrap()),
                "{路径} 不该可试听"
            );
        }
    }

    #[test]
    fn 绝对路径解析到项目根下() {
        let 根 = std::path::Path::new("C:/proj");
        let 相对 = 项目相对路径::解析("assets/audio/beep.wav").unwrap();
        let 绝对 = 音频绝对路径(根, &相对);
        assert_eq!(绝对, 根.join("assets/audio/beep.wav"));
    }

    #[test]
    fn 可以从实体读取音频组件() {
        use scene_doc::document::组件数据;
        let mut 实体 = scene_doc::document::场景实体::新建("喇叭");
        let mut 属性 = std::collections::BTreeMap::new();
        属性.insert(
            "音频".to_string(),
            属性值::资源(scene_doc::property::资源引用值::按路径(
                项目相对路径::解析("assets/audio/beep.wav").unwrap(),
            )),
        );
        属性.insert("音量".to_string(), 属性值::浮点(0.5));
        实体.加组件(组件数据::新建(
            crate::audio_components::音频组件名,
            属性,
        ));
        let (路径, 音量) = 实体音频(&实体).expect("应当读出音频");
        assert_eq!(路径.文本(), "assets/audio/beep.wav");
        assert!((音量 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn 没有音频组件时读不出来() {
        let 实体 = scene_doc::document::场景实体::新建("箱子");
        assert!(实体音频(&实体).is_none());
    }
}
