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

#[test]
fn 实例化场景会插入实体并且是一个撤销步() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    // 造一个"子场景"，放进项目。
    let 源路径 = 项目相对路径::解析("scenes/宝箱.scene.ron").unwrap();
    let mut 源文档 = scene_doc::document::场景文档::新建("宝箱");
    let 箱体 = 源文档.生成实体("宝箱", None);
    源文档.生成实体("盖子", Some(箱体));
    编辑器(&世界)
        .项目
        .as_ref()
        .unwrap()
        .保存场景(&源路径, &源文档)
        .expect("保存源场景");

    let 原数量 = 编辑(&世界).会话.文档.实体数量();
    推动作(&mut 世界, vec![UiAction::实例化场景(源路径.clone())]);
    执行动作(&mut 世界);

    let 新数量 = 编辑(&世界).会话.文档.实体数量();
    assert_eq!(新数量, 原数量 + 2, "实例应当插入源场景的两个实体");
    // 实例根带来源标记，Inspector 里能看到来自哪个场景。
    let 实例根 = 编辑(&世界).选择.主选中().expect("实例化后应当选中实例根");
    let 实体 = 编辑(&世界).会话.文档.取(实例根).expect("实例根应当存在");
    assert_eq!(
        editor_core::instance::实例来源(实体),
        Some("scenes/宝箱.scene.ron")
    );

    // 一个撤销步回到实例化之前。
    推动作(&mut 世界, vec![UiAction::撤销]);
    执行动作(&mut 世界);
    assert_eq!(
        编辑(&世界).会话.文档.实体数量(),
        原数量,
        "撤销必须把整次实例化退回去"
    );
    let _ = 根;
}

fn 编辑器(世界: &World) -> &editor_core::state::编辑器状态 {
    &世界.resource::<EditorStateHolder>().状态
}

#[test]
fn 导出子场景后可以再实例化回来() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    // 当前场景：容器 + 两个子节点（其中一个带组件）。
    let (容器, 子节点数) = {
        let mut 持有 = 世界.resource_mut::<EditorStateHolder>();
        let 文档 = &mut 持有.状态.会话.文档;
        let 容器 = 文档.生成实体("宝箱", None);
        let 盖 = 文档.生成实体("盖子", Some(容器));
        文档.生成实体("锁", Some(容器));
        if let Some(实体) = 文档.取可变(盖) {
            实体.加组件(scene_doc::document::组件数据::标记("Sprite"));
        }
        (容器, 3)
    };

    // 导出子树。
    let 子场景路径 = 项目相对路径::解析("scenes/宝箱.scene.ron").unwrap();
    推动作(
        &mut 世界,
        vec![UiAction::导出子场景 {
            根: 容器,
            路径: 子场景路径.clone(),
        }],
    );
    执行动作(&mut 世界);

    let 文件 = 根.join("scenes").join("宝箱.scene.ron");
    assert!(文件.is_file(), "子场景文件应当真的写到磁盘");
    let 读回 = 编辑(&世界)
        .项目
        .as_ref()
        .unwrap()
        .读取场景(&子场景路径)
        .expect("子场景应当能被读回");
    assert_eq!(读回.文档.实体数量(), 子节点数, "导出的实体数应当与子树一致");

    // 再实例化回当前场景：实体总数增加同样的数量。
    let 实例化前 = 编辑(&世界).会话.文档.实体数量();
    推动作(&mut 世界, vec![UiAction::实例化场景(子场景路径)]);
    执行动作(&mut 世界);
    assert_eq!(
        编辑(&世界).会话.文档.实体数量(),
        实例化前 + 子节点数,
        "实例化子场景应当插入它包含的全部实体"
    );

    // 一次撤销把实例化退回去。
    推动作(&mut 世界, vec![UiAction::撤销]);
    执行动作(&mut 世界);
    assert_eq!(编辑(&世界).会话.文档.实体数量(), 实例化前);
}

#[test]
fn 试听请求只会为音频文件排队() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    // 音频：排进试听请求。
    let 音频 = 项目相对路径::解析("assets/audio/beep.wav").unwrap();
    推动作(&mut 世界, vec![UiAction::试听资源(音频.clone())]);
    执行动作(&mut 世界);
    assert_eq!(界面(&世界).试听请求.要播放.as_ref(), Some(&音频));
    assert!(!界面(&世界).试听请求.要停止);

    // 非音频：请求保持为空，并在日志里说明原因（不静默失败）。
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);
    推动作(
        &mut 世界,
        vec![UiAction::试听资源(
            项目相对路径::解析("assets/tex/a.png").unwrap(),
        )],
    );
    执行动作(&mut 世界);
    assert!(
        界面(&世界).试听请求 == editor_ui::state::试听请求::default(),
        "非音频文件不该进入试听队列"
    );
    assert!(
        编辑(&世界)
            .日志列表()
            .iter()
            .any(|条目| 条目.渲染(editor_ui::语言::中文).contains("png")),
        "必须留下一条说明为什么没试听"
    );
}

#[test]
fn 停止试听会排入停止请求() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);
    推动作(&mut 世界, vec![UiAction::停止试听]);
    执行动作(&mut 世界);
    assert!(界面(&世界).试听请求.要停止);
    assert!(界面(&世界).试听请求.要播放.is_none());
}

#[test]
fn 试听选中实体的音频组件() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);
    let 音频 = 项目相对路径::解析("assets/audio/beep.wav").unwrap();

    let 实体 = {
        let mut 持有 = 世界.resource_mut::<EditorStateHolder>();
        let 文档 = &mut 持有.状态.会话.文档;
        let 标识 = 文档.生成实体("喇叭", None);
        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(scene_doc::document::组件数据::新建(
                "音频",
                [
                    (
                        "音频".to_string(),
                        scene_doc::property::属性值::资源(
                            scene_doc::property::资源引用值::按路径(音频.clone()),
                        ),
                    ),
                    ("音量".to_string(), scene_doc::property::属性值::浮点(0.7)),
                ]
                .into_iter()
                .collect(),
            ));
        }
        标识
    };
    {
        let mut 持有 = 世界.resource_mut::<EditorStateHolder>();
        持有.状态.选择.单选(实体);
    }

    推动作(&mut 世界, vec![UiAction::试听选中实体音频]);
    执行动作(&mut 世界);
    assert_eq!(界面(&世界).试听请求.要播放.as_ref(), Some(&音频));
}

#[test]
fn 设计分辨率可保存并拒绝非法值() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    let mut 项目 = 项目描述::打开(&根).expect("打开项目");
    let mut 设置 = 项目.设置视图();
    设置.设计分辨率 = (1280, 720);
    项目.应用设置(&设置).expect("应当能保存");
    项目.保存清单().expect("写清单");

    let 重开 = 项目描述::打开(&根).expect("重新打开项目");
    assert_eq!(重开.清单().设计分辨率, (1280, 720), "设计分辨率必须落盘");

    // 0 尺寸必须被拒绝，且清单不变。
    let mut 项目 = 重开;
    let mut 坏设置 = 项目.设置视图();
    坏设置.设计分辨率 = (0, 720);
    assert!(项目.应用设置(&坏设置).is_err(), "0 宽必须被拒绝");
    assert_eq!(项目.清单().设计分辨率, (1280, 720), "校验失败不能改清单");

    // 过大同样拒绝。
    坏设置.设计分辨率 = (100_000, 720);
    assert!(项目.应用设置(&坏设置).is_err(), "过大的分辨率必须被拒绝");
}

#[test]
fn 旧项目清单缺少设计分辨率时用默认值() {
    // 手写一份"旧格式"清单（没有 `设计分辨率` 字段），打开后应当拿到默认值。
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);
    let 清单路径 = 根.join("project.ron");
    let 旧内容 = std::fs::read_to_string(&清单路径).expect("读清单");
    // 把新字段整行删掉，模拟旧文件。
    let 旧内容: String = 旧内容
        .lines()
        .filter(|行| !行.contains("设计分辨率"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&清单路径, 旧内容).expect("写旧清单");

    let 项目 = 项目描述::打开(&根).expect("旧清单应当能打开");
    assert_eq!(
        项目.清单().设计分辨率,
        editor_core::project::默认设计分辨率(),
        "缺字段时必须回落到默认设计分辨率"
    );
}

#[test]
fn 轨道插值模式会写进动画文件() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    // 建一个实体并给它一条轨道。
    let 实体 = {
        let mut 持有 = 世界.resource_mut::<EditorStateHolder>();
        let 文档 = &mut 持有.状态.会话.文档;
        let 标识 = 文档.生成实体("方块", None);
        持有.状态.选择.单选(标识);
        持有.状态.动画 = Some(editor_core::animation::动画会话::新建("测试动画"));
        // 指定保存路径：没有路径时「保存动画」只会弹对话框，不会直接写盘。
        持有.状态.动画.as_mut().unwrap().路径 =
            Some(项目相对路径::解析("anims/test.anim.ron").unwrap());
        标识
    };
    推动作(
        &mut 世界,
        vec![UiAction::录制动画关键帧 {
            属性: vec![editor_core::animation::动画属性::平移],
        }],
    );
    执行动作(&mut 世界);
    推动作(
        &mut 世界,
        vec![UiAction::设置轨道插值 {
            实体,
            属性: editor_core::animation::动画属性::平移,
            插值: editor_core::animation::插值模式::步进,
        }],
    );
    执行动作(&mut 世界);

    let 轨道 = 编辑(&世界)
        .动画
        .as_ref()
        .expect("应当有动画会话")
        .文档
        .轨道
        .iter()
        .find(|轨道| 轨道.实体 == 实体)
        .expect("轨道应当存在")
        .clone();
    assert_eq!(轨道.插值, editor_core::animation::插值模式::步进);

    // 落盘：插值是数据，必须出现在文件里。
    let 路径 = 项目相对路径::解析("anims/test.anim.ron").unwrap();
    推动作(&mut 世界, vec![UiAction::保存动画]);
    执行动作(&mut 世界);
    let 文本 = std::fs::read_to_string(根.join(路径.文本())).expect("动画文件应当存在");
    // 用解码往返断言，而不是在文本里找 "Step"：RON 里的枚举写法属于
    // 序列化细节（中文变体名），测试不该把它钉死；行为等价才是要验的东西。
    let 读回 = editor_core::animation::动画文档::解码(&文本, &根.join(路径.文本()))
        .expect("动画文件应当能解码");
    let 读回轨道 = 读回
        .轨道
        .iter()
        .find(|轨道| 轨道.实体 == 实体)
        .expect("读回后轨道应当存在");
    assert_eq!(
        读回轨道.插值,
        editor_core::animation::插值模式::步进,
        "插值模式必须往返：{文本}"
    );
}

#[test]
fn 复制粘贴可以跨场景搬实体() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);

    // 在场景 A 里造一棵子树并复制。
    let 根实体 = {
        let mut 持有 = 世界.resource_mut::<EditorStateHolder>();
        let 文档 = &mut 持有.状态.会话.文档;
        let 根 = 文档.生成实体("宝箱", None);
        文档.生成实体("盖子", Some(根));
        持有.状态.选择.单选(根);
        根
    };
    推动作(&mut 世界, vec![UiAction::复制选中子树]);
    执行动作(&mut 世界);
    let 剪贴板数 = 编辑(&世界)
        .剪贴板
        .as_ref()
        .expect("剪贴板应当有内容")
        .实体数量();
    assert_eq!(剪贴板数, 2, "应当复制整棵子树");

    // 换一个场景（空文档），粘贴。
    {
        let mut 持有 = 世界.resource_mut::<EditorStateHolder>();
        持有.状态.会话 = editor_core::state::场景会话::新建("另一个场景");
        持有.状态.选择.清空();
    }
    推动作(&mut 世界, vec![UiAction::粘贴剪贴板]);
    执行动作(&mut 世界);

    let 文档 = &编辑(&世界).会话.文档;
    assert_eq!(文档.实体数量(), 2, "粘贴应当插入两个实体");
    // 身份必须是新的：否则跨场景会出现"同一个实体"。
    assert!(!文档.存在(根实体));
    // 剪贴板没有来源场景，因此不该留下虚假的场景引用。
    assert!(
        文档.资源表.is_empty(),
        "剪贴板粘贴不该登记来源场景：{:?}",
        文档.资源表
    );
    // 一次撤销退回。
    推动作(&mut 世界, vec![UiAction::撤销]);
    执行动作(&mut 世界);
    assert_eq!(编辑(&世界).会话.文档.实体数量(), 0);
}

#[test]
fn 空剪贴板粘贴只留一条提示() {
    let mut 世界 = 建测试世界();
    let (_目录, _根) = 建项目(&mut 世界);
    推动作(&mut 世界, vec![UiAction::粘贴剪贴板]);
    执行动作(&mut 世界);
    assert_eq!(
        编辑(&世界).会话.文档.实体数量(),
        0,
        "空剪贴板不该插入任何实体"
    );
    assert!(
        编辑(&世界)
            .日志列表()
            .iter()
            .any(|条目| 条目.渲染(editor_ui::语言::中文).contains("剪贴板")),
        "必须说明剪贴板是空的"
    );
}

#[test]
fn 打包配置可保存且拒绝越界输出目录() {
    let mut 世界 = 建测试世界();
    let (_目录, 根) = 建项目(&mut 世界);

    let mut 项目 = 项目描述::打开(&根).expect("打开项目");
    let mut 设置 = 项目.设置视图();
    设置.打包.输出目录 = "build/release".to_string();
    设置.打包.包含全部场景 = true;
    项目.应用设置(&设置).expect("合法输出目录应当能保存");
    项目.保存清单().expect("写清单");

    let 重开 = 项目描述::打开(&根).expect("重新打开");
    let 打包 = 重开.清单().打包.clone().expect("打包配置应当落盘");
    assert_eq!(打包.输出目录, "build/release");
    assert!(打包.包含全部场景);

    // 越界输出目录必须被拒绝：打包会先清空它，写错就是不可挽回的破坏。
    let mut 项目 = 重开;
    let mut 坏设置 = 项目.设置视图();
    坏设置.打包.输出目录 = "C:/Windows".to_string();
    assert!(项目.应用设置(&坏设置).is_err(), "绝对路径必须被拒绝");
    坏设置.打包.输出目录 = "../外面".to_string();
    assert!(项目.应用设置(&坏设置).is_err(), "越界路径必须被拒绝");
    assert_eq!(
        项目.清单().打包.as_ref().map(|配置| 配置.输出目录.clone()),
        Some("build/release".to_string()),
        "校验失败不能改清单"
    );
}
