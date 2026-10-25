//! 中文名：存档槽位
//! 游戏存档槽位浏览器的端到端验证。
//!
//! ```text
//! 项目里有 saves/slot_1.sav、slot_2.sav、一个坏文件
//! 动作「显示存档槽位」→ 界面状态拿到 2 个槽位 + 1 条损坏记录
//! 没有项目 → 目录为空、列表清空，不 panic
//! ```
//!
//! 这里**真实写盘**：存档是文件，纯内存夹具证明不了"目录扫描"这条链路。

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
use editor_ui::state::UiAction;
use savegame::{存档, 存档元数据, 存档格式版本};

fn 建测试世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
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
    世界
}

fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 推动作(世界: &mut World, 动作: UiAction) {
    世界.resource_mut::<UiStateHolder>().动作.推(动作);
}

fn 界面(世界: &World) -> &editor_ui::界面状态 {
    &世界.resource::<UiStateHolder>().状态
}

fn 写存档(目录: &std::path::Path, 槽位: &str, 预览: &str, 时间: u64, 等级: u64) {
    let 仓库 = savegame::存档仓库::新建(目录);
    仓库
        .写入(&存档::新建(
            存档元数据 {
                format_version: 存档格式版本,
                game_version: 1,
                slot: 槽位.to_string(),
                saved_at_unix: 时间,
                play_time_seconds: 42.0,
                preview: 预览.to_string(),
            },
            serde_json::json!({ "等级": 等级 }),
        ))
        .expect("写存档");
}

#[test]
fn 扫描项目存档目录并标出损坏项() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    let 存档目录 = 根.join("saves");
    std::fs::create_dir_all(&存档目录).expect("建 saves");
    写存档(&存档目录, "slot_1", "青岚山谷 · 等级 3", 100, 3);
    写存档(&存档目录, "slot_2", "青岚山谷 · 等级 5", 200, 5);
    std::fs::write(存档目录.join("坏.sav"), "不是合法存档").expect("写坏文件");

    let mut 世界 = 建测试世界();
    世界.resource_mut::<EditorStateHolder>().状态.项目 = Some(项目);
    推动作(&mut 世界, UiAction::显示存档槽位(true));
    执行动作(&mut 世界);

    let 状态 = 界面(&世界);
    assert!(状态.显示存档槽位, "对话框应当打开");
    assert_eq!(状态.存档槽位.len(), 2, "两个有效槽位：{:?}", 状态.存档槽位);
    assert_eq!(状态.存档损坏.len(), 1, "坏文件要被列进损坏项");
    assert_eq!(状态.存档槽位[0].slot, "slot_2", "最近的排前面");
    assert!(状态.存档槽位[0].preview.contains("等级 5"));
    assert!(
        状态
            .存档目录
            .as_deref()
            .is_some_and(|路径| 路径.replace('\\', "/").ends_with("saves")),
        "目录应当指向项目的 saves/：{:?}",
        状态.存档目录
    );
}

#[test]
fn 没有项目时清空列表而不报错() {
    let mut 世界 = 建测试世界();
    推动作(&mut 世界, UiAction::显示存档槽位(true));
    执行动作(&mut 世界);

    let 状态 = 界面(&世界);
    assert!(状态.存档槽位.is_empty());
    assert!(状态.存档损坏.is_empty());
    assert!(状态.存档目录.is_none());
}

#[test]
fn 关闭后再打开会重新扫描到新存档() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    let 存档目录 = 根.join("saves");
    std::fs::create_dir_all(&存档目录).expect("建 saves");
    写存档(&存档目录, "slot_1", "第一版", 100, 1);

    let mut 世界 = 建测试世界();
    世界.resource_mut::<EditorStateHolder>().状态.项目 = Some(项目);
    推动作(&mut 世界, UiAction::显示存档槽位(true));
    执行动作(&mut 世界);
    assert_eq!(界面(&世界).存档槽位.len(), 1);

    // 游戏又存了一档：刷新必须看到它，而不是继续显示旧列表。
    写存档(&存档目录, "slot_2", "第二版", 300, 2);
    推动作(&mut 世界, UiAction::刷新存档槽位);
    执行动作(&mut 世界);
    assert_eq!(界面(&世界).存档槽位.len(), 2, "刷新后应当看到新槽位");
}
