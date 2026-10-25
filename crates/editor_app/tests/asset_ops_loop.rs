//! 中文名：资源操作闭环
//! 资源重命名/删除的端到端验证（走**动作队列**，与用户点击同一条路）。
//!
//! 覆盖的真实链路：
//!
//! ```text
//! 右键"重命名" → 请求重命名资源 → 对话框草稿预填
//!   → 确认重命名资源 → 移动文件 → 改写其它场景 → 当前文档进撤销栈
//! 右键"删除" → 请求删除资源 → 真实引用列表 + 阻止原因
//!   → 确认删除资源（有引用时必须被拒绝，文件仍在）
//! 资源面板"使用报告" → 检查资源使用 → 未使用/缺失分别进控制台
//! ```
//!
//! 为什么必须走动作层：文件移动与场景改写的顺序、失败回滚都在
//! `asset_ops` 里；但"界面上的按钮/菜单是否真的接到了它"只有在动作层
//! 才能被验证。只测 `asset_ops` 会漏掉"零入口"这类缺陷。

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
use scene_doc::document::{场景文档, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 资源引用值};
use std::collections::BTreeMap;

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

fn 推动作(世界: &mut World, 动作列表: Vec<UiAction>) {
    let mut 界面 = 世界.resource_mut::<UiStateHolder>();
    for 动作 in 动作列表 {
        界面.动作.推(动作);
    }
}

fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列应当能运行");
}

fn 编辑(世界: &World) -> &editor_core::state::编辑器状态 {
    &世界.resource::<EditorStateHolder>().状态
}

fn 界面(世界: &World) -> &editor_ui::界面状态 {
    &世界.resource::<UiStateHolder>().状态
}

fn 路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("路径")
}

fn 精灵(资源: 项目相对路径) -> 组件数据 {
    let mut 属性 = BTreeMap::new();
    属性.insert(
        "texture".to_string(),
        属性值::资源(资源引用值::按路径(资源)),
    );
    组件数据::新建("Sprite", 属性)
}

/// 建项目：两张图、两个场景；当前打开的是 `scenes/甲.scene.ron`。
fn 建项目(世界: &mut World) -> (tempfile::TempDir, std::path::PathBuf) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    std::fs::write(根.join("assets").join("房屋.png"), "房屋").expect("写资源");
    std::fs::write(根.join("assets").join("未用.png"), "未用").expect("写资源");

    let mut 甲 = 场景文档::新建("甲");
    let 实体 = 甲.生成实体("房屋", None);
    甲.取可变(实体)
        .expect("实体")
        .加组件(精灵(路径("assets/房屋.png")));
    甲.同步资源表();
    let 甲路径 = 路径("scenes/甲.scene.ron");
    项目.保存场景(&甲路径, &甲).expect("保存甲");

    let mut 乙 = 场景文档::新建("乙");
    let 实体 = 乙.生成实体("另一间房", None);
    乙.取可变(实体)
        .expect("实体")
        .加组件(精灵(路径("assets/房屋.png")));
    乙.同步资源表();
    项目
        .保存场景(&路径("scenes/乙.scene.ron"), &乙)
        .expect("保存乙");

    let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
    持有.项目 = Some(项目);
    持有.会话 = editor_core::state::场景会话 {
        文档: 甲,
        路径: Some(甲路径),
    };
    (目录, 根)
}

#[test]
fn 请求重命名会预填草稿并等待确认() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    推动作(
        &mut 世界,
        vec![UiAction::请求重命名资源(路径("assets/房屋.png"))],
    );
    执行动作(&mut 世界);

    let 草稿 = 界面(&世界)
        .资源重命名
        .as_ref()
        .expect("应当打开重命名对话框");
    assert_eq!(草稿.旧路径, 路径("assets/房屋.png"));
    assert_eq!(草稿.新路径, "assets/房屋.png", "应预填当前路径方便修改");
    // 文件此时一字未动 —— 对话框只是草稿。
    let 项目 = 编辑(&世界).项目.as_ref().unwrap();
    assert!(项目.绝对路径(&路径("assets/房屋.png")).is_file());
}

#[test]
fn 确认重命名会移动文件并同步所有场景引用() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    推动作(
        &mut 世界,
        vec![UiAction::确认重命名资源 {
            旧路径: 路径("assets/房屋.png"),
            新路径: 路径("assets/客厅.png"),
        }],
    );
    执行动作(&mut 世界);

    assert!(界面(&世界).资源重命名.is_none(), "成功后对话框应关闭");
    let 项目 = 编辑(&世界).项目.as_ref().unwrap();
    assert!(项目.绝对路径(&路径("assets/客厅.png")).is_file());
    assert!(!项目.绝对路径(&路径("assets/房屋.png")).exists());

    // 磁盘上的另一个场景也改了。
    let 乙 = 项目
        .读取场景(&路径("scenes/乙.scene.ron"))
        .expect("读乙")
        .文档;
    assert!(乙.资源表.contains_key(&路径("assets/客厅.png")));

    // 当前文档改了，且可以撤销（撤销后引用回到旧路径）。
    assert!(
        编辑(&世界)
            .会话
            .文档
            .资源表
            .contains_key(&路径("assets/客厅.png"))
    );
    push_undo(&mut 世界);
    assert!(
        编辑(&世界)
            .会话
            .文档
            .资源表
            .contains_key(&路径("assets/房屋.png"))
    );
}

/// 通过动作队列执行一次撤销（不直接调 API，确保动作接线存在）。
fn push_undo(世界: &mut World) {
    推动作(世界, vec![UiAction::撤销]);
    执行动作(世界);
}

#[test]
fn 有引用的资源删除被拒绝且文件仍在() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    推动作(
        &mut 世界,
        vec![UiAction::请求删除资源(路径("assets/房屋.png"))],
    );
    执行动作(&mut 世界);

    let 草稿 = 界面(&世界).资源删除.as_ref().expect("应当打开删除对话框");
    assert!(草稿.阻止原因.is_some(), "有引用时必须有阻止原因");
    assert!(
        !草稿.引用.is_empty(),
        "必须列出引用位置，而不是只说'有引用'"
    );
    assert!(!草稿.可删除(), "被引用时删除按钮不应当可用");

    推动作(
        &mut 世界,
        vec![UiAction::确认删除资源(路径("assets/房屋.png"))],
    );
    执行动作(&mut 世界);

    let 项目 = 编辑(&世界).项目.as_ref().unwrap();
    assert!(
        项目.绝对路径(&路径("assets/房屋.png")).is_file(),
        "被拒绝的删除不能动文件"
    );
    // 拒绝原因回到草稿里，用户看得到。
    assert!(
        界面(&世界)
            .资源删除
            .as_ref()
            .is_some_and(|草稿| 草稿.错误.is_some())
    );
}

#[test]
fn 未被引用的资源可以删除() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    推动作(
        &mut 世界,
        vec![UiAction::请求删除资源(路径("assets/未用.png"))],
    );
    执行动作(&mut 世界);
    let 草稿 = 界面(&世界).资源删除.as_ref().expect("对话框");
    assert!(草稿.阻止原因.is_none());
    assert!(草稿.可删除());

    推动作(
        &mut 世界,
        vec![UiAction::确认删除资源(路径("assets/未用.png"))],
    );
    执行动作(&mut 世界);

    assert!(界面(&世界).资源删除.is_none(), "成功后对话框应关闭");
    let 项目 = 编辑(&世界).项目.as_ref().unwrap();
    assert!(!项目.绝对路径(&路径("assets/未用.png")).exists());
}

#[test]
fn 使用报告会列出未使用资源() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    推动作(&mut 世界, vec![UiAction::检查资源使用]);
    执行动作(&mut 世界);

    let 日志 = 编辑(&世界)
        .日志列表()
        .iter()
        .map(|条目| 条目.渲染(editor_ui::i18n::语言::中文))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        日志.contains("assets/未用.png"),
        "应当报告未使用资源：{日志}"
    );
    assert!(
        !日志.contains("assets/房屋.png"),
        "被引用的资源不能出现在未使用列表里：{日志}"
    );
}
