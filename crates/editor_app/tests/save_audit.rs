//! 中文名：保存前审计
//! 保存场景前的一致性审计端到端验证（走**动作队列**，与用户按 Ctrl+S 同一条路）。
//!
//! # 为什么这组测试存在
//!
//! 审计本身只是几条 `if`，会静默失效的是它的**两条边界**：
//!
//! 1. 报出问题后顺手"修数据"或**拦住落盘** —— 前者把用户没来得及备份的
//!    孤儿记录删掉，后者让用户按了保存却什么都没写进去，两种都比不审计更糟；
//! 2. 只在某个入口生效 —— 载入查了、保存没查，那么坏数据正是在保存那一刻
//!    被写进 Git 的，用户在控制台里看到的"没问题"来自另一次检查。
//!
//! 因此这里的断言顺序是：**文件真的写了、写进去的内容一条没少、控制台里
//! 有对应条数的结构化告警**，最后再拿干净场景反钉一次"不许产生噪音"。
//! 场景内容用 `项目描述::读取场景` 从磁盘读回，断言的是落盘结果而不是内存状态。

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
use editor_core::state::{场景会话, 日志内容, 日志级别, 编辑器状态};
use editor_ui::state::UiAction;
use scene_doc::document::{场景文档, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::prefab::{节点地址, 覆盖键, 预制件实例记录, 预制件文档};
use scene_doc::property::{属性值, 资源引用值};
use std::collections::BTreeMap;

/// 场景在项目的固定落点：审计挂在"已关联文件"的保存路径上。
const 场景路径文本: &str = "scenes/关卡.scene.ron";

/// 建一个"有一个敌人模板、场景里挂着一份实例记录"的项目与应用。
///
/// 返回应用、临时目录、模板与模板场景里那个真实存在的子节点地址。
/// 模板落盘后**再**打开项目，`打开项目` 会刷新模板目录，实例才比对得上。
///
/// 临时目录由调用方持有：`TempDir` 一析构项目根就没了，落盘断言读不到文件。
fn 建应用(
    用例: &str,
) -> (
    App,
    tempfile::TempDir,
    预制件文档,
    项目相对路径,
    节点地址,
    实体标识,
) {
    let 目录 = tempfile::tempdir().expect("创建临时目录");
    let 根 = 目录.path().join(用例);
    let 项目 = 项目描述::创建(&根, 用例).expect("创建项目");

    let mut 模板场景 = 场景文档::新建("敌人模板");
    let 模板根 = 模板场景.生成实体("敌人", None);
    let 模板子 = 模板场景.生成实体("枪", Some(模板根));
    let 模板 = 预制件文档::新建("敌人", 模板根, 模板场景);
    let 模板路径 = 取路径("prefabs/敌人.prefab.ron");
    项目.建目录("prefabs").expect("建模板目录");
    项目.保存预制件(&模板路径, &模板).expect("写入模板");

    let mut 编辑器 = 编辑器状态::新建();
    编辑器.打开项目(&根).expect("打开项目");
    assert_eq!(编辑器.预制件目录.数量(), 1, "模板应当已被目录读到");

    let mut 文档 = 场景文档::新建("关卡");
    let 实例根 = 文档.生成实体("敌人实例", None);
    let mut 记录 = 预制件实例记录::新建(模板.引用(模板路径.clone()));
    记录.展开版本 = 模板.头.版本;
    文档.预制件实例表.insert(实例根, 记录);
    编辑器.会话 = 场景会话 {
        文档,
        路径: Some(取路径(场景路径文本)),
    };

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
    (
        应用,
        目录,
        模板,
        模板路径,
        节点地址::根().追加(模板子),
        实例根,
    )
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

/// 数某条结构化标识在日志里出现了几次（不区分级别）。
fn 日志条数(应用: &App, 期望标识: &str) -> usize {
    编辑器(应用)
        .日志列表()
        .iter()
        .filter(|条目| {
            matches!(&条目.内容,
                日志内容::结构化 { 标识, .. } if *标识 == 期望标识)
        })
        .count()
}

/// 取汇总那条告警的第一个参数文本，用于断言"汇总数=明细条数"。
fn 汇总数(应用: &App) -> Option<String> {
    编辑器(应用)
        .日志列表()
        .iter()
        .find_map(|条目| match &条目.内容 {
            日志内容::结构化 { 标识, 参数 } if *标识 == "log_audit_summary" => {
                参数.first().map(|值| 值.to_string())
            }
            _ => None,
        })
}

/// 往实例根的记录上写一条覆盖；地址可以是模板里根本不存在的那个。
fn 加覆盖(应用: &mut App, 实例根: 实体标识, 地址: 节点地址) {
    let mut 持有 = 应用.world_mut().resource_mut::<EditorStateHolder>();
    持有
        .状态
        .会话
        .文档
        .预制件实例表
        .get_mut(&实例根)
        .expect("实例记录应当存在")
        .覆盖
        .设值(
            覆盖键::整组件(地址, "Sprite"),
            属性值::文本("红".to_string()),
        );
}

/// 塞进一条键指向不存在实体的实例记录（模拟手改文件 / 旧版本存档）。
fn 加孤儿记录(
    应用: &mut App, 模板: &预制件文档, 模板路径: &项目相对路径
) -> 实体标识 {
    let 孤儿键 = 实体标识::新建();
    let mut 持有 = 应用.world_mut().resource_mut::<EditorStateHolder>();
    持有
        .状态
        .会话
        .文档
        .预制件实例表
        .insert(孤儿键, 预制件实例记录::新建(模板.引用(模板路径.clone())));
    孤儿键
}

/// 从磁盘读回场景，断言的是**落盘结果**而不是内存里的状态。
fn 读回场景(目录: &tempfile::TempDir, 用例: &str) -> 场景文档 {
    let 项目 = 项目描述::打开(目录.path().join(用例)).expect("重开项目");
    项目
        .读取场景(&取路径(场景路径文本))
        .expect("场景应当已落盘")
        .文档
}

#[test]
fn 审计报出问题也照常落盘且不改写任何数据() {
    let (mut 应用, 目录, 模板, 模板路径, _, 实例根) = 建应用("报问题也保存");
    加覆盖(&mut 应用, 实例根, 节点地址::根().追加(实体标识::新建()));
    let 孤儿键 = 加孤儿记录(&mut 应用, &模板, &模板路径);

    执行(&mut 应用, UiAction::保存场景);

    let 落盘 = 读回场景(&目录, "报问题也保存");
    assert_eq!(
        落盘.预制件实例表.len(),
        2,
        "审计只报告，不许顺手删记录：一条孤儿、一条正常都要原样写进文件"
    );
    assert!(
        落盘.预制件实例表.contains_key(&孤儿键),
        "被审计点名的孤儿记录必须还在文件里，由用户自己决定怎么处理"
    );
    assert_eq!(
        落盘.预制件实例表[&实例根].覆盖.条目数(),
        1,
        "失效覆盖同样原样保留 —— 它可能对应还没同步的新版模板"
    );

    assert_eq!(
        日志条数(&应用, "log_audit_prefab"),
        2,
        "孤儿记录 + 失效覆盖各一条"
    );
    assert_eq!(日志条数(&应用, "log_audit_summary"), 1);
    assert_eq!(
        汇总数(&应用).as_deref(),
        Some("2"),
        "汇总条数要与明细条数一致，否则用户不知道漏看了几条"
    );
    assert!(
        编辑器(&应用)
            .日志列表()
            .iter()
            .any(|条目| 条目.级别 == 日志级别::警告),
        "审计必须是警告级别：信息级别会被控制台的默认过滤挡掉"
    );
}

#[test]
fn 干净场景保存不留一条审计日志() {
    let (mut 应用, 目录, _模板, _模板路径, 合法地址, 实例根) = 建应用("干净场景");
    加覆盖(&mut 应用, 实例根, 合法地址);

    执行(&mut 应用, UiAction::保存场景);

    assert_eq!(日志条数(&应用, "log_audit_prefab"), 0);
    assert_eq!(日志条数(&应用, "log_audit_summary"), 0);
    assert!(
        读回场景(&目录, "干净场景")
            .预制件实例表
            .contains_key(&实例根),
        "覆盖落在模板真实节点上时，保存照旧、审计照旧沉默"
    );
}

#[test]
fn 场景载入走的是同一条审计而不是第二份实现() {
    let (mut 应用, _目录, _模板, _模板路径, _, 实例根) = 建应用("载入也查");
    加覆盖(&mut 应用, 实例根, 节点地址::根().追加(实体标识::新建()));
    let 文档 = 编辑器(&应用).会话.文档.clone();

    应用
        .world_mut()
        .resource_mut::<EditorStateHolder>()
        .状态
        .应用已载入场景(文档, Some(取路径(场景路径文本)));

    assert_eq!(
        日志条数(&应用, "log_audit_prefab"),
        1,
        "载入与保存共用一份检查，两处报出的必须是同一条事实"
    );
}

#[test]
fn 另存为同样先审计再落盘() {
    let (mut 应用, 目录, _模板, _模板路径, _, 实例根) = 建应用("另存为");
    加覆盖(&mut 应用, 实例根, 节点地址::根().追加(实体标识::新建()));

    let 副本路径 = 取路径("scenes/副本.scene.ron");
    执行(&mut 应用, UiAction::场景另存为(副本路径.clone()));

    assert_eq!(
        日志条数(&应用, "log_audit_prefab"),
        1,
        "审计挂在写盘那一步，不能只挂在 Ctrl+S 那个分支上"
    );
    assert_eq!(
        编辑器(&应用).会话.路径.as_ref(),
        Some(&副本路径),
        "报了审计也照样完成另存为"
    );
    assert!(
        目录
            .path()
            .join("另存为")
            .join("scenes/副本.scene.ron")
            .is_file(),
        "副本必须真的落盘"
    );
}

/// 往场景里加一个引用 `贴图` 的精灵，并把该资源文件写进项目磁盘。
///
/// 返回资源的绝对路径，方便调用方模拟"文件被删除"。
fn 加带贴图的精灵(应用: &mut App, 贴图: &项目相对路径) -> std::path::PathBuf {
    let 绝对 = 编辑器(应用)
        .项目
        .as_ref()
        .expect("夹具应当打开了项目")
        .绝对路径(贴图);
    std::fs::write(&绝对, b"png").expect("写资源文件");

    let mut 持有 = 应用.world_mut().resource_mut::<EditorStateHolder>();
    let 实体 = 持有.状态.新建实体("精灵", None).expect("新实体");
    let mut 属性 = BTreeMap::new();
    属性.insert(
        "texture".to_string(),
        属性值::资源(资源引用值::按路径(贴图.clone())),
    );
    持有.状态.添加组件(实体, 组件数据::新建("Sprite", 属性));
    绝对
}

/**
 * 资源被删掉、场景还引用着：保存前报出来，但引用与文件都不许被"顺手修"。
 *
 * 这是跨文件那一层的核心断言：审计读的是**磁盘当前状态**，
 * 而修复权在用户手里（放回文件、或改引用）。因此这里同时检查
 * "报了一条"和"落盘内容里那条引用还在"。
 */
#[test]
fn 资源被删后保存会报跨文件缺失但照常落盘() {
    let (mut 应用, 目录, _模板, _模板路径, _, _实例根) = 建应用("资源缺失");
    let 贴图 = 取路径("assets/敌人.png");
    let 绝对 = 加带贴图的精灵(&mut 应用, &贴图);
    std::fs::remove_file(&绝对).expect("删掉资源文件");

    执行(&mut 应用, UiAction::保存场景);

    assert_eq!(
        日志条数(&应用, "log_audit_asset_missing"),
        1,
        "场景引用的资源不存在时必须报一条"
    );
    assert_eq!(日志条数(&应用, "log_audit_summary"), 1);
    assert_eq!(
        汇总数(&应用).as_deref(),
        Some("1"),
        "缺失资源要计入同一条汇总"
    );

    let 落盘 = 读回场景(&目录, "资源缺失");
    let 还引用着 = 落盘.实体表.values().any(|实体| {
        实体.组件.iter().any(|组件| {
            组件
                .属性
                .values()
                .any(|值| matches!(值, 属性值::资源(引用) if 引用.路径 == 贴图))
        })
    });
    assert!(
        还引用着,
        "审计只报告：引用必须原样写回文件，由用户决定是补回资源还是改引用"
    );
}

/// 资源存在时跨文件检查必须沉默 —— 否则每次保存都会刷一条假警报。
#[test]
fn 资源存在时跨文件检查保持沉默() {
    let (mut 应用, _目录, _模板, _模板路径, _, _实例根) = 建应用("资源齐全");
    let 贴图 = 取路径("assets/敌人.png");
    加带贴图的精灵(&mut 应用, &贴图);

    执行(&mut 应用, UiAction::保存场景);

    assert_eq!(日志条数(&应用, "log_audit_asset_missing"), 0);
    assert_eq!(日志条数(&应用, "log_audit_summary"), 0);
}
