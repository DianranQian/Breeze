//! 项目管理与资源导入两条工作流的端到端验证。
//!
//! 覆盖：
//! * **项目设置**：对话框草稿 → 动作层校验 → 写回 `project.ron` → 重开读回；
//! * **最近项目**：打开项目自动记入列表、失效目录被清理、清除列表；
//! * **资源导入**：扫描 → 写 `imports.ron` → 状态从"新增"变"最新"，
//!   以及依赖检查报告缺失引用。
//!
//! 这些全部通过**动作队列**驱动（与用户点击走同一条路），
//! 而不是直接调 `编辑器状态` 的 API —— 后者会漏掉"动作有没有接线"。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::编辑器配置资源;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::scene_bridge::运行时宿主;
use editor_core::project::{导入状态, 项目描述};
use editor_ui::state::UiAction;
use scene_doc::path::项目相对路径;

fn 建测试世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(editor_app::config::布局待保存::default());
    世界.insert_resource(editor_app::run::项目运行管理器::default());
    世界.insert_resource(构建宿主::新建());
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

/// 建一个带场景与资源的临时项目，并返回 (临时目录, 项目根)。
fn 建项目(世界: &mut World) -> (tempfile::TempDir, std::path::PathBuf) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    // 一个场景 + 一个资源文件。
    let mut 文档 = scene_doc::document::场景文档::新建("主场景");
    文档.生成实体("根", None);
    项目
        .保存场景(&项目相对路径::解析("scenes/main.scene.ron").unwrap(), &文档)
        .expect("保存场景");
    std::fs::write(根.join("assets").join("hero.png"), b"png-bytes").expect("写资源");
    世界.resource_mut::<EditorStateHolder>().状态.项目 = Some(项目);
    (目录, 根)
}

#[test]
fn 打开项目会记入最近项目列表() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);
    世界.resource_mut::<EditorStateHolder>().状态.项目 = None;

    推动作(&mut 世界, vec![UiAction::打开项目(根.clone())]);
    执行动作(&mut 世界);

    assert_eq!(
        界面(&世界).最近项目,
        vec![根.clone()],
        "打开项目后应当出现在最近项目里"
    );
    let 配置 = &世界.resource::<编辑器配置资源>().0;
    assert_eq!(配置.最近项目列表, vec![根], "配置里也要记下来（它会写盘）");
}

#[test]
fn 打开失效的最近项目会被清理并给出警告() {
    let mut 世界 = 建测试世界();
    let 幽灵 = std::path::PathBuf::from("C:/这个目录不存在/项目");
    let 列表 = {
        let mut 配置 = 世界.resource_mut::<编辑器配置资源>();
        配置.0.记录最近项目(&幽灵);
        配置.0.最近项目列表.clone()
    };
    世界.resource_mut::<UiStateHolder>().状态.最近项目 = 列表;

    推动作(&mut 世界, vec![UiAction::打开最近项目(幽灵.clone())]);
    执行动作(&mut 世界);

    assert!(
        !界面(&世界).最近项目.contains(&幽灵),
        "失效目录必须从列表移除，否则用户会一直点到它"
    );
    assert!(
        编辑(&世界)
            .日志列表()
            .iter()
            .any(|条目| 条目.渲染(editor_ui::语言::中文).contains("不存在")),
        "应当有一条警告说明目录不存在"
    );
}

#[test]
fn 清除最近项目会同时清空配置与界面() {
    let mut 世界 = 建测试世界();
    let 列表 = {
        let mut 配置 = 世界.resource_mut::<编辑器配置资源>();
        配置.0.记录最近项目(std::path::Path::new("C:/a"));
        配置.0.记录最近项目(std::path::Path::new("C:/b"));
        配置.0.最近项目列表.clone()
    };
    世界.resource_mut::<UiStateHolder>().状态.最近项目 = 列表;
    推动作(&mut 世界, vec![UiAction::清除最近项目]);
    执行动作(&mut 世界);

    assert!(界面(&世界).最近项目.is_empty());
    assert!(世界.resource::<编辑器配置资源>().0.最近项目列表.is_empty());
}

#[test]
fn 项目设置经由动作写盘并可读回() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    // 打开设置对话框：草稿来自清单。
    推动作(&mut 世界, vec![UiAction::显示项目设置(true)]);
    执行动作(&mut 世界);
    let 草稿 = 界面(&世界)
        .项目设置草稿
        .clone()
        .expect("打开对话框后应当有草稿");
    assert_eq!(草稿.名称, "项目");
    assert!(界面(&世界).显示项目设置);

    // 改草稿并通过动作保存。
    let mut 新设置 = 草稿.clone();
    新设置.名称 = "改过的名字".into();
    新设置.说明 = "这是说明".into();
    新设置.启动场景 = Some("scenes/main.scene.ron".into());
    新设置.构建.包名 = Some("game".into());
    推动作(&mut 世界, vec![UiAction::保存项目设置(新设置)]);
    执行动作(&mut 世界);

    assert!(
        !界面(&世界).显示项目设置,
        "保存成功后对话框应当关闭；错误：{:?}",
        界面(&世界).项目设置错误
    );
    assert!(界面(&世界).项目设置错误.is_none());

    // 从磁盘重新打开，确认真的写进了 project.ron。
    let 重开 = 项目描述::打开(&根).expect("重新打开项目");
    assert_eq!(重开.清单().名称, "改过的名字");
    assert_eq!(重开.清单().说明, "这是说明");
    assert_eq!(
        重开.清单().启动场景.as_ref().map(|路径| 路径.文本()),
        Some("scenes/main.scene.ron")
    );
    assert_eq!(
        重开.清单().构建.as_ref().and_then(|构建| 构建.包名.clone()),
        Some("game".to_string())
    );
}

#[test]
fn 非法的项目设置会留在对话框并给出错误() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    {
        let mut 设置 = 编辑(&世界).项目.as_ref().unwrap().设置视图();
        设置.启动场景 = Some("scenes/没有这个场景.scene.ron".into());
        推动作(&mut 世界, vec![UiAction::保存项目设置(设置)]);
    }
    执行动作(&mut 世界);

    assert!(界面(&世界).显示项目设置, "校验失败时对话框必须保持打开");
    assert!(
        界面(&世界)
            .项目设置错误
            .as_deref()
            .is_some_and(|错误| 错误.contains("启动场景")),
        "错误信息应当说明是哪一项不合法"
    );
    // 清单不能被改动。
    assert!(编辑(&世界).项目.as_ref().unwrap().清单().启动场景.is_none());
}

#[test]
fn 重新导入资源会写缓存并让状态变为最新() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    // 首次：资源是"新增"。
    世界
        .run_system_once(actions::同步导入快照)
        .expect("同步导入快照应当能运行");
    let 状态表 = &世界.resource::<导入快照资源>().状态表;
    assert_eq!(
        状态表.get("assets/hero.png"),
        Some(&导入状态::新增),
        "还没导入过的资源应当是新增：{状态表:?}"
    );

    推动作(&mut 世界, vec![UiAction::重新导入资源]);
    执行动作(&mut 世界);

    // imports.ron 真的写到了磁盘。
    assert!(
        根.join("imports.ron").is_file(),
        "导入后必须留下 imports.ron"
    );

    // 刷新快照：状态应当变成最新。
    世界.resource_mut::<导入快照资源>().需刷新 = true;
    世界
        .run_system_once(actions::同步导入快照)
        .expect("同步导入快照应当能运行");
    let 状态表 = &世界.resource::<导入快照资源>().状态表;
    assert_eq!(
        状态表.get("assets/hero.png"),
        Some(&导入状态::最新),
        "导入过的资源应当是最新：{状态表:?}"
    );
}

#[test]
fn 依赖检查会报告缺失的资源引用() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    // 在场景里塞一个指向不存在文件的资源引用。
    let 场景路径 = 项目相对路径::解析("scenes/main.scene.ron").unwrap();
    let 项目 = 编辑(&世界).项目.as_ref().unwrap();
    let mut 读取 = 项目.读取场景(&场景路径).expect("读场景");
    let 缺失路径 = 项目相对路径::解析("assets/丢了.png").unwrap();
    读取.文档.资源表.insert(
        缺失路径.clone(),
        scene_doc::document::资源引用 {
            路径: 缺失路径.clone(),
            唯一标识: None,
        },
    );
    std::fs::write(
        根.join("scenes").join("main.scene.ron"),
        scene_doc::storage::编码场景(&读取.文档).expect("编码"),
    )
    .expect("写场景");

    推动作(&mut 世界, vec![UiAction::检查资源依赖]);
    执行动作(&mut 世界);

    let 有缺失日志 = 编辑(&世界).日志列表().iter().any(|条目| {
        let 文本 = 条目.渲染(editor_ui::语言::中文);
        文本.contains("丢了.png") && 文本.contains("main.scene.ron")
    });
    assert!(有缺失日志, "依赖检查必须指出缺的是哪个场景引用的哪个资源");
}

#[test]
fn 没有构建产物时打包会被阻止并说明原因() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    推动作(&mut 世界, vec![UiAction::打包项目]);
    执行动作(&mut 世界);

    let 结果 = 界面(&世界)
        .打包结果
        .clone()
        .expect("打包动作必须留下结果（哪怕是失败）");
    assert!(!结果.成功);
    assert!(
        编辑(&世界).日志列表().iter().any(|条目| {
            let 文本 = 条目.渲染(editor_ui::语言::中文);
            文本.contains("构建")
        }),
        "必须明确告诉用户要先构建，而不是静默什么都不发生"
    );
}

#[test]
fn 打包配置会写进项目清单() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    let 配置 = editor_core::package::打包配置 {
        输出目录: "build/release".into(),
        包含全部场景: true,
        写入清单: false,
    };
    推动作(&mut 世界, vec![UiAction::设置打包配置(配置.clone())]);
    执行动作(&mut 世界);

    let 重开 = 项目描述::打开(&根).expect("重新打开项目");
    assert_eq!(重开.清单().打包.as_ref(), Some(&配置));
}
