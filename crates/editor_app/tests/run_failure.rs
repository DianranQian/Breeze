//! 中文名：运行失败的降级路径
//! 「运行」按钮失败时必须**只**在控制台里说清楚，不进入运行态、不 panic、不弹窗。
//!
//! # 为什么这组测试存在
//!
//! 运行失败有两条完全不同的原因，处理方式必须相反：
//!
//! 1. **场景里没有实体**：构建出的空世界与"运行成功但什么都没画"在界面上
//!    无法区分，所以必须拒绝启动并提示，否则用户只会看到一个空视口；
//! 2. **数据不完整**（组件类型未注册、父实体悬空）：必须**降级启动**，
//!    把缺失项逐条写进控制台，让用户能进游戏模式自己看问题，而不是堵在门口。
//!
//! 两条路径都不能 panic：panic 在 GUI 进程里意味着窗口消失、用户手里
//! 没保存的工程一起丢掉。因此这里的断言是"状态没变 / 状态变了但日志说清了 /
//! 一个文件对话框都没弹"。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{布局待保存, 编辑器配置资源};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框请求, 文件对话框通道, 缩略图缓存,
    诊断资源,
};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::{运行时宿主, 运行状态};
use editor_core::state::{场景会话, 日志内容, 编辑器状态};
use editor_ui::state::UiAction;
use scene_doc::document::{场景文档, 组件数据};
use std::collections::BTreeMap;

/// 建一个只含动作执行所需资源的世界，并返回**请求接收端**。
///
/// 接收端留在测试手里，才能断言"这条路径一个文件对话框都没弹" ——
/// 原生对话框是另一个进程窗口，一旦弹出就打断用户手头的操作。
fn 建测试世界() -> (World, crossbeam_channel::Receiver<文件对话框请求>) {
    let (请求发送端, 请求接收端) = crossbeam_channel::unbounded();
    let (结果发送端, 结果接收端) = crossbeam_channel::unbounded();
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(物理运行时状态::default());
    世界.insert_resource(文件对话框通道 {
        请求发送端,
        结果接收端,
        处理中: false,
    });
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(布局待保存::default());
    世界.insert_resource(项目运行管理器::default());
    世界.insert_resource(构建宿主::新建());
    世界.insert_resource(诊断中心::default());
    世界.insert_resource(编辑器配置资源::default());
    世界.insert_resource(导入快照资源::default());
    let _ = 结果发送端;
    (世界, 请求接收端)
}

fn 执行(世界: &mut World, 动作: UiAction) {
    世界.resource_mut::<UiStateHolder>().动作.推(动作);
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列不应 panic");
}

fn 编辑器(世界: &World) -> &编辑器状态 {
    &世界.resource::<EditorStateHolder>().状态
}

fn 运行时(世界: &World) -> &运行时宿主 {
    世界.resource::<运行时宿主>()
}

/// 数某条结构化标识在日志里出现了几次。
fn 日志条数(世界: &World, 期望标识: &str) -> usize {
    编辑器(世界)
        .日志列表()
        .iter()
        .filter(|条目| matches!(&条目.内容, 日志内容::结构化 { 标识, .. } if *标识 == 期望标识))
        .count()
}

/// 日志里是否有某段自由文本（未迁移的警告都走这一形态）。
fn 有自由文本(世界: &World, 片段: &str) -> bool {
    编辑器(世界).日志列表().iter().any(|条目| match &条目.内容 {
        日志内容::自由文本(文本) => 文本.contains(片段),
        日志内容::结构化 { .. } => false,
    })
}

#[test]
fn 空场景点运行只提示不进入运行态也不弹窗() {
    let (mut 世界, 对话框请求) = 建测试世界();
    世界.resource_mut::<EditorStateHolder>().状态.会话 = 场景会话::新建("空场景");
    let 模式 = 世界.resource::<UiStateHolder>().状态.模式;

    执行(&mut 世界, UiAction::运行场景);

    assert_eq!(运行时(&世界).状态, 运行状态::已停止, "空场景不应进入运行态");
    assert!(运行时(&世界).世界.is_none(), "不应该留下半建好的世界");
    assert_eq!(
        日志条数(&世界, "log_run_scene_empty"),
        1,
        "必须给出可翻译的提示"
    );
    assert_eq!(
        世界.resource::<UiStateHolder>().状态.模式,
        模式,
        "失败后必须停在原来的编辑模式，用户可以补数据后直接重试"
    );
    assert!(
        对话框请求.try_recv().is_err(),
        "运行失败不许弹任何文件对话框"
    );
}

#[test]
fn 空场景失败后补一个实体就能重试成功() {
    let (mut 世界, _对话框请求) = 建测试世界();
    世界.resource_mut::<EditorStateHolder>().状态.会话 = 场景会话::新建("空场景");

    执行(&mut 世界, UiAction::运行场景);
    assert_eq!(运行时(&世界).状态, 运行状态::已停止);

    世界
        .resource_mut::<EditorStateHolder>()
        .状态
        .会话
        .文档
        .生成实体("玩家", None);

    执行(&mut 世界, UiAction::运行场景);

    assert_eq!(运行时(&世界).状态, 运行状态::运行中);
    assert_eq!(运行时(&世界).上次统计.实体数, 1);
}

#[test]
fn 组件类型没注册时降级运行并把缺失项写进日志() {
    let (mut 世界, _对话框请求) = 建测试世界();
    let 会话 = &mut 世界.resource_mut::<EditorStateHolder>().状态.会话;
    会话.文档 = 场景文档::新建("残缺场景");
    let 标识 = 会话.文档.生成实体("破碎实体", None);
    会话
        .文档
        .实体表
        .get_mut(&标识)
        .expect("实体存在")
        .加组件(组件数据::新建("并不存在的组件类型", BTreeMap::new()));

    执行(&mut 世界, UiAction::运行场景);

    assert_eq!(
        运行时(&世界).状态,
        运行状态::运行中,
        "数据不完整必须降级启动，而不是拒绝运行"
    );
    assert_eq!(
        运行时(&世界).上次统计.跳过类型,
        vec!["并不存在的组件类型".to_string()]
    );
    assert!(
        有自由文本(&世界, "并不存在的组件类型"),
        "缺失的类型名必须出现在控制台里，用户才知道该去修什么"
    );
}

#[test]
fn 崩溃报告写进文件而不是弹窗() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 路径 = editor_app::panic_report::写崩溃报告(
        目录.path(),
        "test panic",
        "src/main.rs:1:1",
        "0: backtrace",
    )
    .expect("写报告");

    let 文本 = std::fs::read_to_string(&路径).expect("读回报告");
    assert!(文本.contains("test panic"));
    assert!(文本.contains("src/main.rs:1:1"));
    assert!(文本.contains("0: backtrace"));
    assert_eq!(
        路径.parent().expect("父目录"),
        目录.path(),
        "报告必须落在给定目录里，而不是某个隐式位置"
    );
}
