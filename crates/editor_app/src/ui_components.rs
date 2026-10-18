//! 游戏界面（UI）组件：让"游戏里的按钮与文字"成为可编辑的场景数据。
//!
//! # 参考 Godot 的 Control 体系
//!
//! Godot 的 UI 是 `Control` 节点的世界：位置/尺寸用锚点（anchor）与偏移
//! 表达，背景由 `Panel`/`TextureRect` 画，文字由 `Label`/`Button` 画。
//! 这里取它的**最小可用子集**，并显式说明取舍：
//!
//! | Godot | 微风 | 取舍原因 |
//! | --- | --- | --- |
//! | `Control.position` + `size` | `UiNode.偏移X/偏移Y/宽度/高度`（屏幕像素） | 锚点系统要一整套布局解算与可视化编辑器，先做绝对定位 |
//! | `Label.text` | `UiText.文本/字号/颜色` | 一致 |
//! | `Button.text` + 样式 | `UiButton.文本/背景色/文字颜色` | 先不做交互状态（hover/pressed）样式 |
//!
//! 锚点、九宫格、主题样式属于后续扩展点：组件字段加出来即可，
//! 不需要改 UI 框架（Inspector 由注册表驱动）。
//!
//! # 为什么坐标用"屏幕像素"而不是世界坐标
//!
//! 游戏 UI 的权威坐标系是**视口像素**：1920×1080 设计稿在缩放后仍然按像素
//! 对齐。世界坐标会随相机缩放漂移，那是精灵/网格的规则，不是 UI 的规则。
//! 因此这里的数值直接对应 Bevy UI 的 `Val::Px`。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use scene_doc::property::{属性值, 颜色值};
use scene_doc::registry::{属性描述, 属性种类, 组件注册表, 组件类型};

/// 界面面板组件的类型名。
pub const 界面面板组件名: &str = "UiNode";
/// 界面文本组件的类型名。
pub const 界面文本组件名: &str = "UiText";
/// 界面按钮组件的类型名。
pub const 界面按钮组件名: &str = "UiButton";

/// 界面面板属性。
#[derive(Debug, Clone, PartialEq)]
pub struct 界面面板属性 {
    /// 锚点预设：决定盒子贴在视口的哪条边上。
    pub 锚点: editor_core::ui_layout::界面锚点,
    /// 宽度（屏幕像素）。
    pub 宽度: f32,
    /// 高度（屏幕像素）。
    pub 高度: f32,
    /// 左上角横坐标（屏幕像素）。
    pub 偏移x: f32,
    /// 左上角纵坐标（屏幕像素）。
    pub 偏移y: f32,
    /// 背景色。
    pub 背景色: 颜色值,
}

impl Default for 界面面板属性 {
    fn default() -> Self {
        Self {
            锚点: editor_core::ui_layout::界面锚点::左上,
            宽度: 320.0,
            高度: 180.0,
            偏移x: 40.0,
            偏移y: 40.0,
            背景色: 颜色值::srgb(0.10, 0.12, 0.16, 0.88),
        }
    }
}

/// 界面文本属性。
#[derive(Debug, Clone, PartialEq)]
pub struct 界面文本属性 {
    /// 文本内容。
    pub 文本: String,
    /// 字号（像素）。
    pub 字号: f32,
    /// 文字颜色。
    pub 颜色: 颜色值,
}

impl Default for 界面文本属性 {
    fn default() -> Self {
        Self {
            文本: "文字".to_string(),
            字号: 24.0,
            颜色: 颜色值::白(),
        }
    }
}

/// 界面按钮属性。
#[derive(Debug, Clone, PartialEq)]
pub struct 界面按钮属性 {
    /// 按钮上的文字。
    pub 文本: String,
    /// 字号（像素）。
    pub 字号: f32,
    /// 背景色。
    pub 背景色: 颜色值,
    /// 文字颜色。
    pub 文字颜色: 颜色值,
}

impl Default for 界面按钮属性 {
    fn default() -> Self {
        Self {
            文本: "按钮".to_string(),
            字号: 22.0,
            背景色: 颜色值::srgb(0.20, 0.48, 0.85, 1.0),
            文字颜色: 颜色值::白(),
        }
    }
}

/// 界面面板组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 界面面板组件类型;

impl 组件类型 for 界面面板组件类型 {
    fn 组件名(&self) -> &'static str {
        界面面板组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(界面面板属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        面板字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<界面面板属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert(
            "锚点".to_string(),
            属性值::枚举(scene_doc::property::枚举值::新建(
                属性.锚点.变体名(),
                属性.锚点.序号(),
            )),
        );
        结果.insert("宽度".to_string(), 属性值::浮点(属性.宽度));
        结果.insert("高度".to_string(), 属性值::浮点(属性.高度));
        结果.insert("偏移X".to_string(), 属性值::浮点(属性.偏移x));
        结果.insert("偏移Y".to_string(), 属性值::浮点(属性.偏移y));
        结果.insert("背景色".to_string(), 属性值::颜色(属性.背景色));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<界面面板属性>() else {
            return false;
        };
        if let Some(属性值::枚举(枚举)) = 数据.get("锚点") {
            属性.锚点 = editor_core::ui_layout::界面锚点::由值(&枚举.变体名, 枚举.序号);
        }
        if let Some(值) = 数据.get("宽度").and_then(属性值::取浮点) {
            属性.宽度 = 值.max(0.0);
        }
        if let Some(值) = 数据.get("高度").and_then(属性值::取浮点) {
            属性.高度 = 值.max(0.0);
        }
        if let Some(值) = 数据.get("偏移X").and_then(属性值::取浮点) {
            属性.偏移x = 值;
        }
        if let Some(值) = 数据.get("偏移Y").and_then(属性值::取浮点) {
            属性.偏移y = 值;
        }
        if let Some(值) = 数据.get("背景色").and_then(属性值::取颜色) {
            属性.背景色 = 值;
        }
        true
    }
}

/// 界面文本组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 界面文本组件类型;

impl 组件类型 for 界面文本组件类型 {
    fn 组件名(&self) -> &'static str {
        界面文本组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(界面文本属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        文本字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<界面文本属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert("文本".to_string(), 属性值::文本(属性.文本.clone()));
        结果.insert("字号".to_string(), 属性值::浮点(属性.字号));
        结果.insert("颜色".to_string(), 属性值::颜色(属性.颜色));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<界面文本属性>() else {
            return false;
        };
        if let Some(属性值::文本(文本)) = 数据.get("文本") {
            属性.文本 = 文本.clone();
        }
        if let Some(值) = 数据.get("字号").and_then(属性值::取浮点) {
            属性.字号 = 值.clamp(1.0, 512.0);
        }
        if let Some(值) = 数据.get("颜色").and_then(属性值::取颜色) {
            属性.颜色 = 值;
        }
        true
    }
}

/// 界面按钮组件类型。
#[derive(Debug, Default, Clone, Copy)]
pub struct 界面按钮组件类型;

impl 组件类型 for 界面按钮组件类型 {
    fn 组件名(&self) -> &'static str {
        界面按钮组件名
    }

    fn 默认实例(&self) -> Box<dyn std::any::Any> {
        Box::new(界面按钮属性::default())
    }

    fn 字段描述(&self) -> &[属性描述] {
        按钮字段()
    }

    fn 导出(&self, 实例: &dyn std::any::Any) -> Option<BTreeMap<String, 属性值>> {
        let 属性 = 实例.downcast_ref::<界面按钮属性>()?;
        let mut 结果 = BTreeMap::new();
        结果.insert("文本".to_string(), 属性值::文本(属性.文本.clone()));
        结果.insert("字号".to_string(), 属性值::浮点(属性.字号));
        结果.insert("背景色".to_string(), 属性值::颜色(属性.背景色));
        结果.insert("文字颜色".to_string(), 属性值::颜色(属性.文字颜色));
        Some(结果)
    }

    fn 实例化(
        &self, 实例: &mut dyn std::any::Any, 数据: &BTreeMap<String, 属性值>
    ) -> bool {
        let Some(属性) = 实例.downcast_mut::<界面按钮属性>() else {
            return false;
        };
        if let Some(属性值::文本(文本)) = 数据.get("文本") {
            属性.文本 = 文本.clone();
        }
        if let Some(值) = 数据.get("字号").and_then(属性值::取浮点) {
            属性.字号 = 值.clamp(1.0, 512.0);
        }
        if let Some(值) = 数据.get("背景色").and_then(属性值::取颜色) {
            属性.背景色 = 值;
        }
        if let Some(值) = 数据.get("文字颜色").and_then(属性值::取颜色) {
            属性.文字颜色 = 值;
        }
        true
    }
}

fn 面板字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        // 锚点选项：(显示名词条键, 场景文件变体名, 序号)。
        let 锚点选项: Vec<(String, String, u32)> = editor_core::ui_layout::界面锚点::全部
            .iter()
            .map(|锚点| {
                (
                    锚点.词条键().to_string(),
                    锚点.变体名().to_string(),
                    锚点.序号(),
                )
            })
            .collect();
        vec![
            属性描述::新建(
                "锚点",
                "ui_field_anchor",
                属性种类::枚举 {
                    选项: 锚点选项
                },
                属性值::枚举(scene_doc::property::枚举值::新建("TopLeft", 0)),
            )
            .带提示("ui_field_anchor_tooltip"),
            属性描述::新建(
                "宽度",
                "ui_field_width",
                属性种类::浮点范围(0.0, 4096.0, 1.0),
                属性值::浮点(320.0),
            )
            .带提示("ui_field_size_tooltip"),
            属性描述::新建(
                "高度",
                "ui_field_height",
                属性种类::浮点范围(0.0, 4096.0, 1.0),
                属性值::浮点(180.0),
            )
            .带提示("ui_field_size_tooltip"),
            属性描述::新建(
                "偏移X",
                "ui_field_offset_x",
                属性种类::浮点范围(-4096.0, 4096.0, 1.0),
                属性值::浮点(40.0),
            )
            .带提示("ui_field_offset_tooltip"),
            属性描述::新建(
                "偏移Y",
                "ui_field_offset_y",
                属性种类::浮点范围(-4096.0, 4096.0, 1.0),
                属性值::浮点(40.0),
            )
            .带提示("ui_field_offset_tooltip"),
            属性描述::新建(
                "背景色",
                "ui_field_background",
                属性种类::颜色,
                属性值::颜色(颜色值::srgb(0.10, 0.12, 0.16, 0.88)),
            ),
        ]
    })
}

fn 文本字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "文本",
                "ui_field_text",
                属性种类::文本,
                属性值::文本("文字".to_string()),
            ),
            属性描述::新建(
                "字号",
                "ui_field_font_size",
                属性种类::浮点范围(1.0, 512.0, 0.5),
                属性值::浮点(24.0),
            ),
            属性描述::新建(
                "颜色",
                "ui_field_text_color",
                属性种类::颜色,
                属性值::颜色(颜色值::白()),
            ),
        ]
    })
}

fn 按钮字段() -> &'static [属性描述] {
    static 列表: OnceLock<Vec<属性描述>> = OnceLock::new();
    列表.get_or_init(|| {
        vec![
            属性描述::新建(
                "文本",
                "ui_field_text",
                属性种类::文本,
                属性值::文本("按钮".to_string()),
            ),
            属性描述::新建(
                "字号",
                "ui_field_font_size",
                属性种类::浮点范围(1.0, 512.0, 0.5),
                属性值::浮点(22.0),
            ),
            属性描述::新建(
                "背景色",
                "ui_field_background",
                属性种类::颜色,
                属性值::颜色(颜色值::srgb(0.20, 0.48, 0.85, 1.0)),
            ),
            属性描述::新建(
                "文字颜色",
                "ui_field_text_color",
                属性种类::颜色,
                属性值::颜色(颜色值::白()),
            ),
        ]
    })
}

/// 把界面组件注册进注册表。
pub fn 扩展注册表(mut 基础: 组件注册表) -> 组件注册表 {
    for 类型 in [
        Box::new(界面面板组件类型) as Box<dyn 组件类型>,
        Box::new(界面文本组件类型),
        Box::new(界面按钮组件类型),
    ] {
        基础
            .注册(类型)
            .expect("界面组件的字段描述与默认值必须自洽；失败说明代码有错");
    }
    基础
}

/// 界面组件名列表（供测试与界面提示）。
pub fn 界面组件名列表() -> Vec<&'static str> {
    vec![界面面板组件名, 界面文本组件名, 界面按钮组件名]
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 扩展注册表包含全部界面组件() {
        let 注册表 = 扩展注册表(组件注册表::新建());
        for 名字 in 界面组件名列表() {
            assert!(注册表.已注册(名字), "{名字} 应当被注册");
        }
    }

    #[test]
    fn 界面组件默认值符合声明种类() {
        let 注册表 = 扩展注册表(组件注册表::新建());
        for 名字 in 界面组件名列表() {
            let 类型 = 注册表.要求(名字).expect("应当能查到");
            for 描述 in 类型.字段描述() {
                assert!(
                    描述.种类.接受(&描述.默认值),
                    "{名字}.{} 的默认值类型不对",
                    描述.字段名
                );
            }
        }
    }

    #[test]
    fn 面板导出实例化往返() {
        let 类型 = 界面面板组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert("宽度".into(), 属性值::浮点(640.0));
        数据.insert("高度".into(), 属性值::浮点(-10.0));
        数据.insert("偏移X".into(), 属性值::浮点(-20.0));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        assert_eq!(导出.get("宽度"), Some(&属性值::浮点(640.0)));
        assert_eq!(
            导出.get("高度"),
            Some(&属性值::浮点(0.0)),
            "负尺寸必须夹到 0"
        );
        assert_eq!(导出.get("偏移X"), Some(&属性值::浮点(-20.0)));
    }

    #[test]
    fn 按钮文本与颜色往返() {
        let 类型 = 界面按钮组件类型;
        let mut 实例 = 类型.默认实例();
        let mut 数据 = BTreeMap::new();
        数据.insert("文本".into(), 属性值::文本("开始游戏".into()));
        数据.insert("字号".into(), 属性值::浮点(32.0));
        类型.实例化(&mut *实例, &数据);
        let 导出 = 类型.导出(&*实例).unwrap();
        assert_eq!(导出.get("文本"), Some(&属性值::文本("开始游戏".into())));
        assert_eq!(导出.get("字号"), Some(&属性值::浮点(32.0)));
    }
}
