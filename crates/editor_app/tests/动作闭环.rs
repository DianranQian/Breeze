//! 编辑器应用层的端到端集成测试。
//!
//! # 这个测试为什么重要
//!
//! `editor_core` 与 `editor_ui` 已经各自有测试，但它们验证的是"API 能工作"。
//! 这里验证的是**用户动作真的能走通整条链路**：
//!
//! ```text
//! UiAction  →  执行动作队列  →  EditorStateHolder  →  保存到磁盘  →  重新打开
//! ```
//!
//! 与"直接调用 editor_core 的 API"相比，这条路多出了三个真实的失败点：
//!
//! 1. **动作分派**：`执行动作队列` 里的 match 分支是否真的接到了这条动作；
//! 2. **上下文参数**：`Local<拖动快照>`、`ResMut` 等参数是否被正确传递；
//! 3. **资源变更可见性**：系统改完之后，从外部读到的资源是否已经是新值。
//!
//! 跑法：用 `World::run_system_once` 在**没有窗口、没有 GPU**的环境里
//! 直接驱动真实的系统函数。
//!
//! # 为什么需要 `--test-threads=1`
//!
//! 这些系统内部有 `Local` 状态（拖动快照），而 `run_system_once`
//! 会为每个 `World` 维护一份局部状态；多个测试并发时若共享同一个 `World`
//! 就会互相干扰。为此每个测试都自建 `World`，并且命令里显式串行执行，
//! 保证结果稳定可复现。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 文件对话框通道, 缩略图缓存, 诊断资源
};
use editor_app::scene_bridge::运行时宿主;
use editor_core::project::项目描述;
use editor_ui::state::{UiAction, 选择意图};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 颜色值};

/// 组装一个只含"动作执行"所需资源的世界。
///
/// 刻意**不**加 `EguiPlugin`：那会要求窗口与渲染上下文，
/// 而本测试要验证的是动作执行，与绘制无关。把绘制排除在外，
/// 测试才能在无 GPU 的 CI 上跑。
fn 建测试世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    // `执行动作队列` 现在也处理"只改界面状态"的动作（恢复默认布局、
    // 聚焦面板、显示关于），因此它需要布局脏标记资源。
    世界.insert_resource(editor_app::config::布局待保存::default());
    // `运行项目 / 停止项目 / 重新运行项目` 由项目运行管理器执行。
    // 早期这条链只存在于"运行模式"的工具栏里，本测试文件没跟着补资源，
    // 于是整个动作闭环测试静默失效了一整轮 —— 现在必须显式装配它。
    世界.insert_resource(editor_app::run::项目运行管理器::default());
    // 同理：`开始构建 / 停止构建` 需要一个真实的构建宿主。
    // 测试里不会真的触发构建，但系统参数必须能解析 ——
    // 缺资源的表现是**所有**动作一起失效，而不是只有构建动作失效。
    世界.insert_resource(editor_app::build::构建宿主::新建());
    世界
}

/// 往动作队列里推入若干动作。
fn 推动作(世界: &mut World, 动作列表: Vec<UiAction>) {
    let mut 界面 = 世界.resource_mut::<UiStateHolder>();
    for 动作 in 动作列表 {
        界面.动作.推(动作);
    }
}

/// 执行已经排好的动作。
fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列应当能运行");
}

/// 读取编辑器状态。
fn 取编辑器(世界: &World) -> &EditorStateHolder {
    世界.resource::<EditorStateHolder>()
}

/// 建一个临时项目并打开它。
fn 建项目(世界: &mut World, 名称: &str) -> tempfile::TempDir {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join(名称);

    let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
    编辑器.状态.创建项目(&根, 名称).expect("创建项目应当成功");
    目录
}

/// 取第一个实体的标识。
fn 唯一实体(世界: &World) -> scene_doc::document::实体标识 {
    取编辑器(世界)
        .状态
        .会话
        .文档
        .实体表
        .keys()
        .next()
        .copied()
        .expect("文档里应当至少有一个实体")
}

// ======================================================================
// 编辑动作
// ======================================================================

#[test]
fn 通过动作队列可以新建实体并被自动选中() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "动作新建");

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "玩家".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);

    let 编辑器 = 取编辑器(&世界);
    assert_eq!(编辑器.状态.会话.文档.实体数量(), 1);
    let 实体 = 唯一实体(&世界);
    assert_eq!(编辑器.状态.会话.文档.取(实体).unwrap().名称, "玩家");
    assert_eq!(
        编辑器.状态.选择.主选中(),
        Some(实体),
        "新建实体应当被自动选中"
    );
}

#[test]
fn 通过动作队列可以修改属性并立刻反映到文档() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "动作改属性");

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "精灵".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 实体 = 唯一实体(&世界);

    推动作(
        &mut 世界,
        vec![UiAction::添加组件 {
            实体,
            类型名: "Sprite".to_string(),
        }],
    );
    执行动作(&mut 世界);

    // 添加组件必须带上注册表里的默认值，而不是一个空属性表
    {
        let 编辑器 = 取编辑器(&世界);
        let 组件 = 编辑器
            .状态
            .会话
            .文档
            .取(实体)
            .unwrap()
            .取组件("Sprite")
            .expect("Sprite 组件应当已被添加");
        assert!(
            !组件.属性.is_empty(),
            "新添加的组件应当带有注册表声明的默认值，而不是空属性表"
        );
        assert!(组件.属性.contains_key("颜色"));
    }

    // 改颜色与位置
    推动作(
        &mut 世界,
        vec![
            UiAction::修改属性(editor_ui::state::属性编辑 {
                实体,
                组件: "Sprite".to_string(),
                属性: "颜色".to_string(),
                新值: 属性值::颜色(颜色值::srgb(1.0, 0.0, 0.0, 1.0)),
            }),
            UiAction::执行命令(editor_core::command::编辑器命令::设变换 {
                标识: 实体,
                变换: Some(scene_doc::document::变换数据 {
                    平移: (5.0, 7.0, 0.0),
                    ..scene_doc::document::变换数据::单位()
                }),
            }),
        ],
    );
    执行动作(&mut 世界);

    let 编辑器 = 取编辑器(&世界);
    let 文档实体 = 编辑器.状态.会话.文档.取(实体).unwrap();
    let 颜色 = 文档实体
        .取组件("Sprite")
        .unwrap()
        .属性
        .get("颜色")
        .unwrap()
        .取颜色()
        .unwrap();
    assert_eq!(颜色.红, 1.0);
    assert_eq!(颜色.绿, 0.0);
    let 变换 = 文档实体.变换.unwrap();
    assert_eq!((变换.平移.0, 变换.平移.1), (5.0, 7.0));
}

#[test]
fn 通过动作队列可以撤销重做且标识保持不变() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "动作撤销");

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "甲".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 实体 = 唯一实体(&世界);

    // 改三次
    for x in [1.0f32, 2.0, 3.0] {
        推动作(
            &mut 世界,
            vec![UiAction::执行命令(
                editor_core::command::编辑器命令::设变换 {
                    标识: 实体,
                    变换: Some(scene_doc::document::变换数据 {
                        平移: (x, 0.0, 0.0),
                        ..scene_doc::document::变换数据::单位()
                    }),
                },
            )],
        );
        执行动作(&mut 世界);
    }
    assert_eq!(
        取编辑器(&世界)
            .状态
            .会话
            .文档
            .取(实体)
            .unwrap()
            .变换
            .unwrap()
            .平移
            .0,
        3.0
    );

    // 撤销两次
    推动作(&mut 世界, vec![UiAction::撤销, UiAction::撤销]);
    执行动作(&mut 世界);
    assert_eq!(
        取编辑器(&世界)
            .状态
            .会话
            .文档
            .取(实体)
            .unwrap()
            .变换
            .unwrap()
            .平移
            .0,
        1.0
    );

    // 重做一次
    推动作(&mut 世界, vec![UiAction::重做]);
    执行动作(&mut 世界);
    assert_eq!(
        取编辑器(&世界)
            .状态
            .会话
            .文档
            .取(实体)
            .unwrap()
            .变换
            .unwrap()
            .平移
            .0,
        2.0
    );

    // 一路撤销到底：实体应当消失，且标识不再存在
    推动作(
        &mut 世界,
        vec![UiAction::撤销, UiAction::撤销, UiAction::撤销],
    );
    执行动作(&mut 世界);
    assert!(
        !取编辑器(&世界).状态.会话.文档.存在(实体),
        "撤销掉新建实体之后，文档里不应再有该标识"
    );

    // 重做必须把**同一个**标识找回来
    推动作(&mut 世界, vec![UiAction::重做]);
    执行动作(&mut 世界);
    assert!(
        取编辑器(&世界).状态.会话.文档.存在(实体),
        "重做之后实体标识必须与原来完全相同，否则选择集与引用会全部失效"
    );
}

#[test]
fn 通过动作队列可以删除实体() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "动作删除");

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "待删除".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 实体 = 唯一实体(&世界);

    推动作(&mut 世界, vec![UiAction::删除实体(实体)]);
    执行动作(&mut 世界);

    let 编辑器 = 取编辑器(&世界);
    assert_eq!(编辑器.状态.会话.文档.实体数量(), 0);
    assert_eq!(
        编辑器.状态.选择.数量(),
        0,
        "删除后选择集必须被清理，不能留下幽灵选中"
    );
}

#[test]
fn 选择动作会正确改变选择集() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "动作选择");

    推动作(
        &mut 世界,
        vec![
            UiAction::执行命令(editor_core::command::编辑器命令::新建实体 {
                名称: "甲".to_string(),
                父实体: None,
                指定标识: None,
            }),
            UiAction::执行命令(editor_core::command::编辑器命令::新建实体 {
                名称: "乙".to_string(),
                父实体: None,
                指定标识: None,
            }),
        ],
    );
    执行动作(&mut 世界);

    let 标识列表: Vec<_> = 取编辑器(&世界)
        .状态
        .会话
        .文档
        .实体表
        .keys()
        .copied()
        .collect();
    assert_eq!(标识列表.len(), 2);

    推动作(
        &mut 世界,
        vec![UiAction::选择(选择意图::设为(标识列表.clone()))],
    );
    执行动作(&mut 世界);
    assert_eq!(取编辑器(&世界).状态.选择.数量(), 2);

    推动作(&mut 世界, vec![UiAction::选择(选择意图::清空)]);
    执行动作(&mut 世界);
    assert_eq!(取编辑器(&世界).状态.选择.数量(), 0);

    推动作(&mut 世界, vec![UiAction::选择(选择意图::单选(标识列表[0]))]);
    执行动作(&mut 世界);
    assert_eq!(取编辑器(&世界).状态.选择.主选中(), Some(标识列表[0]));
}

// ======================================================================
// 完整闭环：动作 → 磁盘 → 重新打开
// ======================================================================

#[test]
fn 完整闭环_通过动作搭场景保存到磁盘再重新打开() {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join("闭环项目");

    let mut 世界 = 建测试世界();
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.创建项目(&根, "闭环项目").expect("创建项目");
    }

    // ---- 1. 通过动作搭一个三层场景 ----
    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "关卡".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 关卡 = 取编辑器(&世界)
        .状态
        .会话
        .文档
        .实体表
        .values()
        .find(|实体| 实体.名称 == "关卡")
        .unwrap()
        .标识;

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "玩家".to_string(),
                父实体: Some(关卡),
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 玩家 = 取编辑器(&世界)
        .状态
        .会话
        .文档
        .实体表
        .values()
        .find(|实体| 实体.名称 == "玩家")
        .unwrap()
        .标识;

    // 给玩家加 Sprite，改颜色，设位置
    推动作(
        &mut 世界,
        vec![
            UiAction::添加组件 {
                实体: 玩家,
                类型名: "Sprite".to_string(),
            },
            UiAction::修改属性(editor_ui::state::属性编辑 {
                实体: 玩家,
                组件: "Sprite".to_string(),
                属性: "颜色".to_string(),
                新值: 属性值::颜色(颜色值::srgb(0.25, 0.5, 0.75, 1.0)),
            }),
            UiAction::修改属性(editor_ui::state::属性编辑 {
                实体: 玩家,
                组件: "Sprite".to_string(),
                属性: "水平翻转".to_string(),
                新值: 属性值::布尔(true),
            }),
            UiAction::执行命令(editor_core::command::编辑器命令::设变换 {
                标识: 玩家,
                变换: Some(scene_doc::document::变换数据 {
                    平移: (100.0, 50.0, 0.0),
                    旋转: (0.0, 0.0, 0.0, 1.0),
                    缩放: (2.0, 2.0, 1.0),
                }),
            }),
        ],
    );
    执行动作(&mut 世界);

    // ---- 2. 另存为 ----
    let 场景路径 = 项目相对路径::解析("scenes/闭环关卡.scene.ron").unwrap();
    推动作(&mut 世界, vec![UiAction::场景另存为(场景路径.clone())]);
    执行动作(&mut 世界);

    let 绝对 = 根.join("scenes").join("闭环关卡.scene.ron");
    assert!(
        绝对.is_file(),
        "场景文件应当被真实写入磁盘：{}",
        绝对.display()
    );
    assert!(
        !取编辑器(&世界).状态.有未保存修改(),
        "保存成功后应当没有未保存修改"
    );

    // 记下保存前的关键数据，供重新打开后比对
    let 保存前文档 = 取编辑器(&世界).状态.会话.文档.clone();
    assert_eq!(保存前文档.实体数量(), 2);

    // ---- 3. 模拟关掉编辑器再打开 ----
    let mut 新世界 = 建测试世界();
    {
        let mut 编辑器 = 新世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.打开项目(&根).expect("重新打开项目");
    }
    推动作(&mut 新世界, vec![UiAction::打开场景(场景路径.clone())]);
    执行动作(&mut 新世界);

    let 新编辑器 = 取编辑器(&新世界);
    let 重开文档 = &新编辑器.状态.会话.文档;

    // ---- 4. 逐项核对 ----
    assert_eq!(重开文档.实体数量(), 2, "重新打开后实体数量必须一致");
    assert_eq!(重开文档.顶层顺序, 保存前文档.顶层顺序);
    assert_eq!(
        重开文档.实体表, 保存前文档.实体表,
        "重新打开后整份实体表必须与保存前逐字段一致"
    );
    assert_eq!(重开文档.资源表, 保存前文档.资源表);

    // 层级必须保持
    let 重开玩家 = 重开文档.取(玩家).expect("重新打开后玩家标识必须保持不变");
    assert_eq!(重开玩家.父实体, Some(关卡), "父子关系必须被保存与恢复");
    assert_eq!(重开玩家.名称, "玩家");

    // 组件属性必须保持
    let 精灵 = 重开玩家.取组件("Sprite").expect("Sprite 组件必须被恢复");
    assert_eq!(
        精灵.属性.get("水平翻转").unwrap().取布尔(),
        Some(true),
        "布尔属性必须被持久化"
    );
    let 颜色 = 精灵.属性.get("颜色").unwrap().取颜色().unwrap();
    assert!(
        (颜色.红 - 0.25).abs() < 1e-5 && (颜色.蓝 - 0.75).abs() < 1e-5,
        "颜色属性必须被持久化，实际 {颜色:?}"
    );

    // 变换必须保持
    let 变换 = 重开玩家.变换.unwrap();
    assert_eq!((变换.平移.0, 变换.平移.1), (100.0, 50.0));
    assert_eq!((变换.缩放.0, 变换.缩放.1), (2.0, 2.0));

    // 重新打开的场景应当是干净的
    assert!(
        !新编辑器.状态.有未保存修改(),
        "刚打开的场景不应显示为已修改"
    );

    // ---- 5. 项目清单也应当能被独立打开 ----
    let 项目 = 项目描述::打开(&根).expect("项目清单应当可独立打开");
    assert_eq!(项目.名称(), "闭环项目");
    assert_eq!(项目.场景列表().len(), 1);
}

// ======================================================================
// 运行时隔离（走真实的启动/停止动作）
// ======================================================================

#[test]
fn 运行与停止不会改动编辑器文档() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "运行时隔离动作");

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "玩家".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 实体 = 唯一实体(&世界);
    let 运行前文档 = 取编辑器(&世界).状态.会话.文档.clone();

    // 运行
    推动作(&mut 世界, vec![UiAction::运行场景]);
    执行动作(&mut 世界);
    assert_eq!(
        世界.resource::<运行时宿主>().状态,
        editor_app::scene_bridge::运行状态::运行中,
        "运行动作之后必须处于运行中状态"
    );
    assert_eq!(
        世界.resource::<运行时宿主>().运行时实体数(),
        1,
        "运行时世界应当包含场景里的那 1 个实体"
    );

    // 暂停 → 继续
    推动作(&mut 世界, vec![UiAction::暂停运行]);
    执行动作(&mut 世界);
    assert_eq!(
        世界.resource::<运行时宿主>().状态,
        editor_app::scene_bridge::运行状态::已暂停
    );
    推动作(&mut 世界, vec![UiAction::暂停运行]);
    执行动作(&mut 世界);
    assert_eq!(
        世界.resource::<运行时宿主>().状态,
        editor_app::scene_bridge::运行状态::运行中
    );

    // 停止
    推动作(&mut 世界, vec![UiAction::停止运行]);
    执行动作(&mut 世界);
    assert!(
        !世界.resource::<运行时宿主>().状态.在运行(),
        "停止之后不应再处于运行状态"
    );
    assert!(
        世界.resource::<运行时宿主>().世界.is_none(),
        "停止之后运行时世界必须被整体丢弃"
    );

    // 编辑文档必须一字未改
    assert_eq!(
        取编辑器(&世界).状态.会话.文档,
        运行前文档,
        "运行、暂停、停止都不能改动编辑器文档"
    );
    assert!(取编辑器(&世界).状态.会话.文档.存在(实体));
}

// ======================================================================
// 合同：动作执行器不得静默失败
// ======================================================================

#[test]
fn 未打开项目时保存动作会记录错误而不是崩溃() {
    let mut 世界 = 建测试世界();
    // 刻意不建项目

    推动作(&mut 世界, vec![UiAction::保存场景]);
    执行动作(&mut 世界);

    let 编辑器 = 取编辑器(&世界);
    let 有错误 = 编辑器
        .状态
        .日志列表()
        .iter()
        .any(|条目| 条目.级别 == editor_core::state::日志级别::错误);
    assert!(
        有错误,
        "没有项目时保存必须记录一条错误日志，而不是静默什么都不做"
    );
}

#[test]
fn 添加未注册组件会记录错误而不是假装成功() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "未注册组件");

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "甲".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 实体 = 唯一实体(&世界);

    推动作(
        &mut 世界,
        vec![UiAction::添加组件 {
            实体,
            类型名: "根本不存在的组件".to_string(),
        }],
    );
    执行动作(&mut 世界);

    let 编辑器 = 取编辑器(&世界);
    assert!(
        !编辑器
            .状态
            .会话
            .文档
            .取(实体)
            .unwrap()
            .有组件("根本不存在的组件"),
        "未注册的组件不能被写进文档"
    );
    assert!(
        编辑器
            .状态
            .日志列表()
            .iter()
            .any(|条目| 条目.级别 == editor_core::state::日志级别::错误),
        "未注册组件必须记录错误日志"
    );
}

#[test]
fn 文件对话框请求会被投递到通道() {
    let mut 世界 = 建测试世界();

    // 无项目时点"打开项目"应当发出一条对话框请求
    推动作(&mut 世界, vec![UiAction::请求打开项目]);
    执行动作(&mut 世界);

    assert!(
        世界.resource::<文件对话框通道>().处理中,
        "请求打开项目之后应当标记为有对话框在处理中"
    );

    // 重复请求应当被拒绝，避免同时弹出多个原生对话框
    推动作(&mut 世界, vec![UiAction::请求打开项目]);
    执行动作(&mut 世界);
    let 有警告 = 取编辑器(&世界)
        .状态
        .日志列表()
        .iter()
        .any(|条目| 条目.级别 == editor_core::state::日志级别::警告);
    assert!(有警告, "重复弹出对话框应当给出警告");
}

// ======================================================================
// 拖动动作
// ======================================================================

#[test]
fn 拖动会以按下位置为基准移动实体() {
    let mut 世界 = 建测试世界();
    let _目录 = 建项目(&mut 世界, "拖动");

    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "可拖动".to_string(),
                父实体: None,
                指定标识: None,
            },
        )],
    );
    执行动作(&mut 世界);
    let 实体 = 唯一实体(&世界);

    // 起始位置设为 (10, 10)
    推动作(
        &mut 世界,
        vec![UiAction::执行命令(
            editor_core::command::编辑器命令::设变换 {
                标识: 实体,
                变换: Some(scene_doc::document::变换数据 {
                    平移: (10.0, 10.0, 0.0),
                    ..scene_doc::document::变换数据::单位()
                }),
            },
        )],
    );
    执行动作(&mut 世界);

    // 开始拖动，然后连续两次报告偏移
    推动作(
        &mut 世界,
        vec![
            UiAction::开始拖动实体 {
                实体,
                追加选择: false,
                按下世界: (10.0, 10.0),
            },
            UiAction::拖动实体到 {
                世界偏移: (5.0, 0.0),
            },
            // 这一条应当覆盖上一条（同一帧内的拖动动作会被合并）
            UiAction::拖动实体到 {
                世界偏移: (7.0, 3.0),
            },
        ],
    );
    执行动作(&mut 世界);

    let 变换 = 取编辑器(&世界)
        .状态
        .会话
        .文档
        .取(实体)
        .unwrap()
        .变换
        .unwrap();
    assert_eq!(
        (变换.平移.0, 变换.平移.1),
        (17.0, 13.0),
        "拖动应当以按下位置 (10,10) 加上最终偏移 (7,3) 计算，\
         而不是把两次偏移累加（那会得到 22,13）"
    );
}
