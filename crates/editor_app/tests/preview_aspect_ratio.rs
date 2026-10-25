//! 中文名：预览宽高比
//! 预览分辨率与宽高比：真实系统级验收。
//!
//! # 为什么这组测试存在
//!
//! "预览画面不能拉伸"是一个**看起来正确、实际很容易错**的性质：
//! 渲染目标尺寸、正交投影、letterbox 显示矩形、叠加层坐标换算四处只要有一处
//! 比例不一致，画面就会被压扁或拉长，而单元测试各测一块是发现不了的。
//!
//! 这里直接跑真实系统：
//! * `preview::同步渲染尺寸` —— 渲染目标是否按设计比例分配；
//! * `视口相机::显示矩形` / `世界转屏幕` —— 显示矩形是否按设计比例 letterbox，
//!   且世界原点与画面中心重合（叠加层与纹理对齐的前提）。
//!
//! 覆盖三种典型设计分辨率：1280×720、1920×1080、1080×1920（竖屏）。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::plugin::{EditorStateHolder, UiStateHolder};
use editor_app::preview::{同步渲染尺寸, 预览渲染目标};
use editor_core::project::项目描述;
use editor_ui::state::视口相机;

/// 造一个"已打开指定设计分辨率的项目"的世界。
///
/// # 临时目录必须**每个用例独占**
///
/// 早先目录名只由设计分辨率决定，而 `横屏_1920x1080_不拉伸`、
/// `面板变窄变宽都不改变画面比例`、`零尺寸与极小面板…` 都用 1920×1080：
/// 并行跑测试时它们互相删对方的目录，于是随机出现
/// "项目已存在 / 目录非空"。用 `用例` 名分开后每个用例一份沙箱。
fn 造世界(用例: &str, 设计: (u32, u32), 面板: (f32, f32)) -> World {
    let 根 = std::env::temp_dir().join(format!(
        "breeze_preview_ratio_{}_{}_{}x{}",
        std::process::id(),
        用例,
        设计.0,
        设计.1
    ));
    let _ = std::fs::remove_dir_all(&根);
    let mut 项目 = 项目描述::创建(&根, "比例测试").expect("创建项目");
    项目.清单可变().设计分辨率 = 设计;
    项目.保存清单().expect("写清单");

    let mut 编辑器 = editor_core::state::编辑器状态::新建();
    编辑器.打开项目(&根).expect("打开项目");

    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder { 状态: 编辑器 });
    let mut 界面状态 = editor_ui::state::界面状态::default();
    界面状态.视口尺寸 = egui::vec2(面板.0, 面板.1);
    世界.insert_resource(UiStateHolder {
        状态: 界面状态,
        动作: Default::default(),
    });
    let mut 图片资源 = Assets::<Image>::default();
    let 图片 = 图片资源.add(Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: 512,
            height: 512,
            ..default()
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[0, 0, 0, 0],
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    ));
    世界.insert_resource(图片资源);
    // egui 纹理表：同步渲染尺寸 会在必要时向它补注册句柄。
    世界.insert_resource(bevy_egui::EguiUserTextures::default());
    世界.insert_resource(预览渲染目标 {
        图片,
        尺寸: (512, 512),
        纹理: None,
    });
    let _ = 根;
    世界
}

/// 渲染目标与显示矩形都必须保持设计分辨率的长宽比。
fn 断言比例(用例: &str, 设计: (u32, u32), 面板: (f32, f32)) {
    let mut 世界 = 造世界(用例, 设计, 面板);
    世界.run_system_once(同步渲染尺寸).expect("同步渲染尺寸");

    let (宽, 高) = 世界.resource::<预览渲染目标>().尺寸;
    let 设计比 = 设计.0 as f32 / 设计.1 as f32;
    assert!(
        (宽 as f32 / 高 as f32 - 设计比).abs() < 0.02,
        "{设计:?} 设计稿在 {面板:?} 面板上的渲染目标比例不对：{宽}×{高}"
    );
    assert!(宽 <= 2048 && 高 <= 2048, "渲染目标不得超过上限");

    // 显示矩形（letterbox 后画面实际占据的区域）比例必须一致。
    let 相机 = 视口相机 {
        设计尺寸: Some((设计.0 as f32, 设计.1 as f32)),
        ..视口相机::default()
    };
    let 面板尺寸 = egui::vec2(面板.0, 面板.1);
    let 显示 = 相机.显示矩形(面板尺寸);
    assert!(
        (显示.width() / 显示.height() - 设计比).abs() < 0.02,
        "{设计:?} 的显示矩形比例不对：{}×{}",
        显示.width(),
        显示.height()
    );
    // 不越界：留边而不是溢出。
    assert!(显示.min.x >= -0.5 && 显示.min.y >= -0.5);
    assert!(显示.max.x <= 面板.0 + 0.5 && 显示.max.y <= 面板.1 + 0.5);
    // 世界原点必须落在显示矩形中心（叠加层与纹理对齐的前提）。
    let 原点屏幕 = 相机.世界转屏幕((0.0, 0.0), 面板尺寸);
    assert!((原点屏幕.x - 显示.center().x).abs() < 0.5);
    assert!((原点屏幕.y - 显示.center().y).abs() < 0.5);
}

#[test]
fn 横屏_1280x720_不拉伸() {
    断言比例("横屏720", (1280, 720), (910.0, 531.0));
}

#[test]
fn 横屏_1920x1080_不拉伸() {
    断言比例("横屏1080", (1920, 1080), (910.0, 531.0));
}

#[test]
fn 竖屏_1080x1920_不拉伸() {
    断言比例("竖屏", (1080, 1920), (910.0, 531.0));
}

#[test]
fn 面板变窄变宽都不改变画面比例() {
    for 面板 in [(420.0, 531.0), (1400.0, 300.0), (640.0, 640.0)] {
        断言比例("变宽变窄", (1920, 1080), 面板);
    }
}

#[test]
fn 零尺寸与极小面板不会产生非法纹理尺寸() {
    for 面板 in [(0.0, 0.0), (1.0, 1.0), (12.0, 800.0)] {
        let mut 世界 = 造世界("极端面板", (1920, 1080), 面板);
        世界
            .run_system_once(同步渲染尺寸)
            .expect("同步渲染尺寸应当能容忍极端面板");
        let (宽, 高) = 世界.resource::<预览渲染目标>().尺寸;
        assert!(宽 >= 64 && 高 >= 64, "过小面板也要给正数尺寸：{宽}×{高}");
    }
}
