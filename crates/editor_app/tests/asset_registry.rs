//! 中文名：资源注册表
//! 验证 `assets/` 的扫描缓存与刷新动作。
//!
//! # 为什么这组测试存在
//!
//! 资源注册表把"每次绘制都扫盘"改成"打开项目时扫一次、用户刷新时再扫"。
//! 缓存一旦接错，最典型的症状不是崩溃，而是**列表停在旧内容**：
//! 新放进项目的文件不出现，删掉的文件还在，或者改了大小仍显示旧值。
//!
//! 这里走真实项目与真实动作队列：创建临时项目、写入假文件、打开项目，
//! 然后改动磁盘并触发 `UiAction::刷新资源注册表`，同时断言代次、条目数
//! 与单条内容都跟着变。

use std::path::PathBuf;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{布局待保存, 编辑器配置资源};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_core::project::项目描述;
use editor_core::state::编辑器状态;
use editor_ui::state::UiAction;
use scene_doc::path::项目相对路径;
use scene_doc::property::资源类别;

/// 建一个已写入假资源并打开的项目，返回应用、临时目录与项目根。
///
/// 临时目录必须由调用方持有：`TempDir` 一析构，项目根就没了，
/// 后续动作里的 UID 同步会因为写不进 `.breeze/` 而失败。
fn 建应用(用例: &str) -> (App, tempfile::TempDir, PathBuf) {
    let 目录 = tempfile::tempdir().expect("创建临时目录");
    let 根 = 目录.path().join(用例);
    项目描述::创建(&根, 用例).expect("创建项目");
    std::fs::create_dir_all(根.join("assets/贴图")).expect("创建资源目录");
    std::fs::write(根.join("assets/贴图/屋顶.png"), [1u8; 4]).expect("写入图片");
    std::fs::write(根.join("assets/背景.ogg"), [2u8; 3]).expect("写入音频");

    let mut 编辑器 = 编辑器状态::新建();
    编辑器.打开项目(&根).expect("打开项目");

    let mut 应用 = App::new();
    let 世界 = 应用.world_mut();
    世界.insert_resource(EditorStateHolder { 状态: 编辑器 });
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(物理运行时状态::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(布局待保存::default());
    世界.insert_resource(项目运行管理器::default());
    世界.insert_resource(构建宿主::新建());
    世界.insert_resource(诊断中心::default());
    世界.insert_resource(编辑器配置资源::default());
    世界.insert_resource(导入快照资源::default());
    (应用, 目录, 根)
}

/// 把一条动作推进队列并立刻执行。
fn 执行(应用: &mut App, 动作: UiAction) {
    应用
        .world_mut()
        .resource_mut::<UiStateHolder>()
        .动作
        .推(动作);
    应用
        .world_mut()
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 取路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("合法路径")
}

fn 编辑器<'a>(应用: &'a App) -> &'a 编辑器状态 {
    &应用.world().resource::<EditorStateHolder>().状态
}

#[test]
fn 打开项目会扫描资源并缓存类别大小与标识() {
    let (应用, _目录, _根) = 建应用("打开扫描");
    let 注册表 = &编辑器(&应用).资源注册表;

    assert_eq!(注册表.条目数(), 2, "assets/ 下的两个文件都应被扫描到");
    assert_eq!(注册表.代次(), 1, "打开项目应当完成一次扫描");

    let 图片 = 注册表
        .取(&取路径("assets/贴图/屋顶.png"))
        .expect("图片条目应当存在");
    assert_eq!(图片.类别, 资源类别::图片);
    assert_eq!(图片.大小, 4);
    assert!(图片.修改时间.is_some(), "真实文件应当带有修改时间");
    assert!(图片.唯一标识.is_some(), "打开项目应同步 UID 表");

    let 音频 = 注册表
        .取(&取路径("assets/背景.ogg"))
        .expect("音频条目应当存在");
    assert_eq!(音频.类别, 资源类别::音频);
    assert_eq!(音频.大小, 3);
}

#[test]
fn 刷新动作在文件变化后推进代次并更新条目() {
    let (mut 应用, _目录, 根) = 建应用("刷新动作");
    let 旧代次 = 编辑器(&应用).资源注册表.代次();

    std::fs::write(根.join("assets/贴图/屋顶.png"), [9u8; 12]).expect("改写图片");
    std::fs::write(根.join("assets/新增.txt"), [3u8; 5]).expect("新增文本");
    std::fs::remove_file(根.join("assets/背景.ogg")).expect("删除音频");

    执行(&mut 应用, UiAction::刷新资源注册表);

    let 注册表 = &编辑器(&应用).资源注册表;
    assert_eq!(
        注册表.代次(),
        旧代次 + 1,
        "一次刷新必须只推进一代，界面据此失效缓存"
    );
    assert_eq!(注册表.条目数(), 2, "改写 + 新增 - 删除后应剩两个条目");
    assert!(
        注册表.取(&取路径("assets/背景.ogg")).is_none(),
        "删除的文件不能留在快照里"
    );
    assert_eq!(
        注册表
            .取(&取路径("assets/贴图/屋顶.png"))
            .map(|条目| 条目.大小),
        Some(12),
        "修改过的文件必须更新大小"
    );
    let 新增 = 注册表
        .取(&取路径("assets/新增.txt"))
        .expect("新增文件应当被扫描到");
    assert_eq!(新增.类别, 资源类别::其他);
    assert!(
        新增.唯一标识.is_some(),
        "刷新动作应先同步 UID 表，再让新文件拿到标识"
    );
}

#[test]
fn 关闭项目会清空注册表并推进代次() {
    let (mut 应用, _目录, _根) = 建应用("关闭清空");
    let 旧代次 = 编辑器(&应用).资源注册表.代次();

    {
        let 状态 = &mut 应用.world_mut().resource_mut::<EditorStateHolder>().状态;
        状态.关闭项目();
    }

    let 注册表 = &编辑器(&应用).资源注册表;
    assert_eq!(注册表.条目数(), 0, "关闭项目后不能残留旧项目条目");
    assert_eq!(
        注册表.代次(),
        旧代次 + 1,
        "清空也要推进代次，避免下一个项目撞上旧缓存"
    );
    assert!(注册表.最近项目根().is_none());
}
