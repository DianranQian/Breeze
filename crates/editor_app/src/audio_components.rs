//! 音频组件：让"这个实体带什么音效"成为可编辑的场景数据。
//!
//! # 与 Godot 的对应
//!
//! Godot 的 `AudioStreamPlayer` / `AudioStreamPlayer3D` 是节点：`stream` 属性
//! 指向音频资源，`volume_db` 控制响度，`autoplay` 决定进入场景是否立刻播放。
//! 这里取同一组语义的最小集合：
//!
//! | Godot | 微风 | 说明 |
//! | --- | --- | --- |
//! | `stream` | `音频`（资源引用） | 进项目资源表，依赖检查与打包能看到 |
//! | `volume_db` | `音量`（0..2 线性） | 线性比 dB 对使用者更直观 |
//! | `autoplay` | `自动播放` | 只是**数据**：运行期由游戏代码决定 |
//! | `AudioStreamPlayer3D` 的衰减 | `空间化` | 勾选表示"按实体位置播放" |
//!
//! # 为什么编辑器里不自动播放
//!
//! `自动播放` 是给**游戏**用的字段，不是给编辑器用的：打开场景就把所有音效
//! 同时播一遍，既吵又掩盖了"到底哪个文件坏了"。编辑器只在用户点「试听」时
//! 播放（见 `audio_preview`），试听走的是同一份路径数据，因此"能编辑的"
//! 和"能听到的"是同一个文件。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use scene_doc::property::{属性值, 资源引用值, 资源类别};
use scene_doc::registry::{属性描述, 属性种类, 组件注册表, 组件类型};

/// 音频组件在场景文件里的类型名。
pub const 音频组件名: &str = "音频";

/// 音频组件属性。
#[derive(Debug, Clone, PartialEq)]
pub struct 音频属性 {
    /// 音频文件（项目相对路径）。
    pub 音频: Option<scene_doc::path::项目相对路径>,
    /// 音量（0..2，1 为原始响度）。
    pub 音量: f32,
    /// 进入游戏时是否自动播放。
    pub 自动播放: bool,
    /// 是否按实体位置做空间化衰减。
    pub 空间化: bool,
}

impl Default for 音频属性 {
    fn default() -> Self {
        Self {
            音频: None,
            音量: 1.0,
            自动播放: false,
            空间化: false,
        }
    }
}

impl 音频属性 {
    /// 是否已经指定了可播放的文件。
    pub fn 有音频(&self) -> bool {
        self.音频.is_some()
    }
}

/// 音频组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 音频组件类型;

impl 组件类型 for 音频组件类型 {
    fn 组件名(&self) -> &'static str {
        音频组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(音频属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        音频字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<音频属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert(
            "音频".to_string(),
            match &属性.音频 {
                Some(路径) => 属性值::资源(资源引用值::按路径(路径.clone())),
                None => 属性值::空,
            },
        );
        结果.insert("音量".to_string(), 属性值::浮点(属性.音量));
        结果.insert("自动播放".to_string(), 属性值::布尔(属性.自动播放));
        结果.insert("空间化".to_string(), 属性值::布尔(属性.空间化));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<音频属性>() else {
            return false;
        };
        if let Some(值) = 数据.get("音频") {
            属性.音频 = 值.取资源().map(|引用| 引用.路径.clone());
        }
        if let Some(值) = 数据.get("音量").and_then(属性值::取浮点) {
            // 上限 2.0：再高只会削波失真，不是"更响"。
            属性.音量 = 值.clamp(0.0, 2.0);
        }
        if let Some(值) = 数据.get("自动播放").and_then(属性值::取布尔) {
            属性.自动播放 = 值;
        }
        if let Some(值) = 数据.get("空间化").and_then(属性值::取布尔) {
            属性.空间化 = 值;
        }
        true
    }
}

fn 音频字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "音频",
                "audio_field_clip",
                属性种类::资源 {
                    类别: 资源类别::音频,
                },
                属性值::空,
            )
            .带提示("audio_field_clip_tooltip"),
            属性描述::新建(
                "音量",
                "audio_field_volume",
                属性种类::浮点范围(0.0, 2.0, 0.01),
                属性值::浮点(1.0),
            )
            .带提示("audio_field_volume_tooltip"),
            属性描述::新建(
                "自动播放",
                "audio_field_autoplay",
                属性种类::布尔,
                属性值::布尔(false),
            )
            .带提示("audio_field_autoplay_tooltip"),
            属性描述::新建(
                "空间化",
                "audio_field_spatial",
                属性种类::布尔,
                属性值::布尔(false),
            )
            .带提示("audio_field_spatial_tooltip"),
        ]
    })
}

/// 把音频组件注册进注册表。
pub fn 扩展注册表(mut 基础: 组件注册表) -> 组件注册表 {
    基础
        .注册(Box::new(音频组件类型))
        .expect("音频组件的字段描述与默认值必须自洽；失败说明代码有错");
    基础
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 扩展注册表包含音频组件() {
        let 注册表 = 扩展注册表(组件注册表::新建());
        assert!(注册表.已注册(音频组件名));
    }

    #[test]
    fn 默认值符合声明种类() {
        let 注册表 = 扩展注册表(组件注册表::新建());
        let 类型 = 注册表.要求(音频组件名).expect("应当能查到");
        for 描述 in 类型.字段描述() {
            assert!(
                描述.种类.接受(&描述.默认值),
                "{} 的默认值类型不对",
                描述.字段名
            );
        }
    }

    #[test]
    fn 导出实例化往返并夹取音量() {
        let 类型 = 音频组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert(
            "音频".into(),
            属性值::资源(资源引用值::按路径(
                scene_doc::path::项目相对路径::解析("assets/audio/beep.wav").unwrap(),
            )),
        );
        数据.insert("音量".into(), 属性值::浮点(5.0));
        数据.insert("自动播放".into(), 属性值::布尔(true));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        let 音量 = 导出.get("音量").and_then(属性值::取浮点).unwrap();
        assert!((音量 - 2.0).abs() < 1e-6, "音量上限必须是 2.0");
        assert_eq!(导出.get("自动播放"), Some(&属性值::布尔(true)));
        match 导出.get("音频") {
            Some(属性值::资源(引用)) => {
                assert_eq!(引用.路径.文本(), "assets/audio/beep.wav")
            }
            _ => panic!("音频路径应当被导出"),
        }
    }

    #[test]
    fn 未指定文件时导出空值() {
        let 类型 = 音频组件类型;
        let 实例 = 类型.默认实例();
        let 导出 = 类型.导出(&*实例).unwrap();
        assert_eq!(导出.get("音频"), Some(&属性值::空));
    }
}
