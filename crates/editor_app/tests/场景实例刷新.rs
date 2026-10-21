//! 场景实例刷新的端到端验证（走动作队列）。
//!
//! ```text
//! 源场景 宝箱 → 实例化到 关卡 → 用户改名/摆位置
//! 源场景新增一个子实体
//! 动作「刷新场景实例」→ 子树被替换、名称/位置/父级保留、一步撤销
//! ```

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{编辑器配置资源, 布局待保存};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_core::project::项目描述;
use editor_ui::state::{UiAction, 选择意图};
use scene_doc::document::{场景文档, 变换数据};
use scene_doc::path::项目相对路径;

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

fn 编辑(世界: &World) -> &editor_core::state::编辑器状态 {
    &世界.resource::<EditorStateHolder>().状态
}

fn 路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("路径")
}

/// 建项目：源场景「宝箱」（根 + 盖子），关卡里有一个容器与一个实例。
fn 建项目(世界: &mut World) -> (tempfile::TempDir, scene_doc::document::实体标识) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");

    let mut 源 = 场景文档::新建("宝箱");
    let 箱 = 源.生成实体("宝箱", None);
    源.生成实体("盖子", Some(箱));
    源.同步资源表();
    let 源路径 = 路径("scenes/宝箱.scene.ron");
    项目.保存场景(&源路径, &源).expect("保存源");

    let mut 关卡 = 场景文档::新建("关卡");
    let 容器 = 关卡.生成实体("容器", None);
    let 实例 = editor_core::instance::实例化为(
        &mut 关卡,
        &源,
        Some(&源路径),
        Some(容器),
    );
    let 旧根 = 实例.根实体.expect("实例根");
    {
        let 实体 = 关卡.取可变(旧根).expect("旧根");
        实体.名称 = "门口宝箱".to_string();
        实体.变换 = Some(变换数据 {
            平移: (5.0, 6.0, 0.0),
            ..变换数据::单位()
        });
    }
    项目.保存场景(&路径("scenes/关卡.scene.ron"), &关卡).expect("保存关卡");

    // 源场景后来多了一个子实体（模拟"改源场景"）。
    源.生成实体("锁", Some(箱));
    项目.保存场景(&源路径, &源).expect("更新源");

    let 持有 = &mut 世界.resource_mut::<EditorStateHolder>().状态;
    持有.项目 = Some(项目);
    持有.应用已载入场景(关卡, Some(路径("scenes/关卡.scene.ron")));
    (目录, 旧根)
}

#[test]
fn 刷新实例会带入源场景的新内容并保留位置() {
    let mut 世界 = 建测试世界();
    let (_目录, 旧根) = 建项目(&mut 世界);
    let 之前数量 = 编辑(&世界).会话.文档.实体数量();
    推动作(&mut 世界, UiAction::选择(选择意图::单选(旧根)));
    执行动作(&mut 世界);

    推动作(&mut 世界, UiAction::刷新场景实例);
    执行动作(&mut 世界);

    let 文档 = &编辑(&世界).会话.文档;
    let 新根 = 编辑(&世界).选择.主选中().expect("刷新后应选中新根");
    let 实体 = 文档.取(新根).expect("新根");
    assert_eq!(实体.名称, "门口宝箱", "名称必须保留");
    let 变换 = 实体.变换.expect("变换");
    assert_eq!((变换.平移.0, 变换.平移.1), (5.0, 6.0), "位置必须保留");
    assert!(
        文档.实体数量() > 之前数量,
        "源场景新增的子实体应当被带进来：{} → {}",
        之前数量,
        文档.实体数量()
    );
}

#[test]
fn 刷新实例是一步可撤销的操作() {
    let mut 世界 = 建测试世界();
    let (_目录, 旧根) = 建项目(&mut 世界);
    推动作(&mut 世界, UiAction::选择(选择意图::单选(旧根)));
    执行动作(&mut 世界);
    推动作(&mut 世界, UiAction::刷新场景实例);
    执行动作(&mut 世界);
    let 刷新后数量 = 编辑(&世界).会话.文档.实体数量();

    推动作(&mut 世界, UiAction::撤销);
    执行动作(&mut 世界);

    assert_eq!(
        编辑(&世界).会话.文档.实体数量(),
        刷新后数量 - 1,
        "一次撤销应当回到刷新前的子树规模"
    );
    assert!(编辑(&世界).会话.文档.存在(旧根), "旧根应当回来");
}

#[test]
fn 选中普通实体时拒绝刷新并告警() {
    let mut 世界 = 建测试世界();
    let (_目录, _旧根) = 建项目(&mut 世界);
    let 容器 = 编辑(&世界)
        .会话
        .文档
        .深度优先遍历()
        .into_iter()
        .find(|(标识, _)| {
            编辑(&世界)
                .会话
                .文档
                .取(*标识)
                .is_some_and(|实体| 实体.名称 == "容器")
        })
        .map(|(标识, _)| 标识)
        .expect("容器");
    推动作(&mut 世界, UiAction::选择(选择意图::单选(容器)));
    执行动作(&mut 世界);
    let 之前 = 编辑(&世界).会话.文档.实体数量();

    推动作(&mut 世界, UiAction::刷新场景实例);
    执行动作(&mut 世界);

    assert_eq!(编辑(&世界).会话.文档.实体数量(), 之前);
    assert!(
        编辑(&世界)
            .日志列表()
            .iter()
            .any(|条目| 条目.渲染(editor_ui::i18n::语言::中文).contains("不是场景实例")),
        "必须说明为什么没刷新"
    );
}
