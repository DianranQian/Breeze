//! 中文名：代码编辑器多标签与保存
//! 多标签打开 / 切换 / 关闭确认与保存落盘的集成测试。
//!
//! # 这里钉住的是四条用户可见的性质
//!
//! 1. **打开是追加，不是替换**：先后打开两个文件得到两个标签，各自保有自己的
//!    查找词、命中与滚动位置；切回去时这些状态原样还在。
//! 2. **重复打开只聚焦**：同一路径第二次打开不能再长出一条同名标签 ——
//!    否则关闭时用户分不清关的是哪一条。
//! 3. **标签间切换不骚扰**：缓冲区互不覆盖，因此切标签不弹确认；只有
//!    「关闭脏标签」与「有脏标签时打开别的文件」才需要确认。
//! 4. **星号只跟随真实落盘**：保存成功后磁盘内容等于缓冲区且星号清掉；
//!    写失败不关标签、不清星号。
//!
//! 确认浮层的按钮点击属于纯 GUI 交互，本文件只验证它们背后的动作语义；
//! 浮层本身是否被绘制正确需要在运行中的编辑器里肉眼确认。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::code_loader::{代码读取通道, 调度代码读取, 轮询代码读取};
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 文件对话框通道, 缩略图缓存, 诊断资源
};
use editor_app::scene_bridge::运行时宿主;
use editor_ui::script::脚本待确认;
use editor_ui::state::UiAction;

// ======================================================================
// 测试脚手架
// ======================================================================

/// 组装"动作执行 + 代码装载"所需的最小世界。
///
/// 资源清单与 `project_tree.rs` 保持一致：漏掉任何一个，
/// `执行动作队列` 的 SystemParam 校验会让所有用例一起失败。
fn 建测试世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(editor_app::physics_runtime::物理运行时状态::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(editor_app::config::布局待保存::default());
    世界.insert_resource(editor_app::run::项目运行管理器::default());
    世界.insert_resource(editor_app::build::构建宿主::新建());
    世界.insert_resource(editor_app::diagnostics::诊断中心::default());
    世界.insert_resource(editor_app::config::编辑器配置资源::default());
    世界.insert_resource(editor_app::plugin::导入快照资源::default());
    世界.insert_resource(代码读取通道::default());
    世界
}

/// 建一个临时项目并打开它，返回临时目录（保持存活到用例结束）。
fn 建项目(世界: &mut World, 名称: &str) -> tempfile::TempDir {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join(名称);
    let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
    编辑器.状态.创建项目(&根, 名称).expect("创建项目应当成功");
    目录
}

/// 往动作队列里推动作。
fn 推动作(世界: &mut World, 动作: UiAction) {
    世界.resource_mut::<UiStateHolder>().动作.推(动作);
}

/// 执行已排好的动作。
fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列应当能运行");
}

/// 把"待读取"意图交给后台线程。
fn 调度(世界: &mut World) {
    世界
        .run_system_once(调度代码读取)
        .expect("调度代码读取应当能运行");
}

/// 轮询到"读取结束"（成功装载或错误清空都算结束）。
fn 等待读取结束(世界: &mut World) -> bool {
    let 截止 = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        世界
            .run_system_once(轮询代码读取)
            .expect("轮询代码读取应当能运行");
        if !世界.resource::<UiStateHolder>().状态.脚本.正在读取() {
            return true;
        }
        if std::time::Instant::now() >= 截止 {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// 走完整链路打开一个文件：动作 → 后台线程 → 轮询认领。
fn 打开文件(世界: &mut World, 相对路径: &str) {
    let 路径 = 项目相对路径(相对路径);
    推动作(世界, UiAction::打开脚本(路径));
    执行动作(世界);
    调度(世界);
    assert!(等待读取结束(世界), "后台读取必须在上限内完成");
}

/// 构造项目相对路径。
fn 项目相对路径(文本: &str) -> scene_doc::path::项目相对路径 {
    scene_doc::path::项目相对路径::解析(文本).expect("测试路径必须合法")
}

/// 当前活动文档的文件名。
fn 当前文件名(世界: &World) -> String {
    世界
        .resource::<UiStateHolder>()
        .状态
        .脚本
        .当前文档()
        .expect("应当有活动文档")
        .路径
        .as_ref()
        .expect("活动文档应当有路径")
        .文件名()
        .to_string()
}

/// 修改活动标签的文本（模拟用户在编辑框里打字）。
fn 改活动文本(世界: &mut World, 追加: &str) {
    let mut 界面 = 世界.resource_mut::<UiStateHolder>();
    let 标签 = 界面.状态.脚本.当前标签可变().expect("应当有活动标签");
    标签.文档.文本.push_str(追加);
}

// ======================================================================
// 用例：多标签
// ======================================================================

#[test]
fn 打开两个文件得到两个标签并各自保留查找与滚动状态() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "多标签");
    std::fs::create_dir_all(目录.path().join("多标签/src")).unwrap();
    std::fs::write(目录.path().join("多标签/src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(目录.path().join("多标签/src/b.rs"), "fn b() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    打开文件(&mut 世界, "src/b.rs");

    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        assert_eq!(界面.脚本.标签.len(), 2, "先后打开两个文件应当得到两个标签");
        assert_eq!(界面.脚本.活动, 1, "新打开的标签应当成为活动标签");
        assert_eq!(当前文件名(&世界), "b.rs");
    }

    {
        let mut 界面 = 世界.resource_mut::<UiStateHolder>();
        let 甲 = &mut 界面.状态.脚本.标签[0];
        甲.查找 = "fn a".to_string();
        甲.重新查找();
        甲.滚动偏移 = 320.0;
    }

    推动作(&mut 世界, UiAction::切换脚本标签(0));
    执行动作(&mut 世界);

    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        assert_eq!(界面.脚本.活动, 0, "切换动作应当改变活动下标");
        assert_eq!(当前文件名(&世界), "a.rs");
        let 甲 = &界面.脚本.标签[0];
        assert_eq!(甲.查找, "fn a", "查找词应当跟着标签走");
        assert_eq!(甲.命中行, vec![0], "命中行应当由标签自己的文本算出");
        assert_eq!(
            甲.待恢复滚动,
            Some(320.0),
            "切回标签时要恢复离开时的滚动位置"
        );
        assert_eq!(界面.脚本.标签[1].滚动偏移, 0.0, "另一个标签的状态不受影响");
    }
}

#[test]
fn 重复打开同一路径只聚焦已有标签() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "重复打开");
    std::fs::create_dir_all(目录.path().join("重复打开/src")).unwrap();
    std::fs::write(目录.path().join("重复打开/src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(目录.path().join("重复打开/src/b.rs"), "fn b() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    打开文件(&mut 世界, "src/b.rs");
    打开文件(&mut 世界, "src/a.rs");

    let 界面 = &世界.resource::<UiStateHolder>().状态;
    assert_eq!(界面.脚本.标签.len(), 2, "同一路径不该出现第二条标签");
    assert_eq!(界面.脚本.活动, 0, "应当聚焦到已打开的那一条");
    assert_eq!(当前文件名(&世界), "a.rs");
}

#[test]
fn 切换标签不弹确认且脏缓冲区原样保留() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "切换无确认");
    std::fs::create_dir_all(目录.path().join("切换无确认/src")).unwrap();
    std::fs::write(目录.path().join("切换无确认/src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(目录.path().join("切换无确认/src/b.rs"), "fn b() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    打开文件(&mut 世界, "src/b.rs");
    改活动文本(&mut 世界, "// 未保存\n");

    推动作(&mut 世界, UiAction::切换脚本标签(0));
    执行动作(&mut 世界);

    let 界面 = &世界.resource::<UiStateHolder>().状态;
    assert!(
        界面.脚本.待确认.is_none(),
        "标签之间切换不丢缓冲区，不该打扰用户"
    );
    assert_eq!(界面.脚本.标签.len(), 2);
    assert!(
        界面.脚本.标签[1].文档.有未保存修改(),
        "切走之后另一个标签的修改必须还在"
    );
}

// ======================================================================
// 用例：保存与未保存确认
// ======================================================================

#[test]
fn 保存脚本把缓冲区写进磁盘并清掉未保存标记() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "保存落盘");
    let 绝对 = 目录.path().join("保存落盘/src/a.rs");
    std::fs::create_dir_all(绝对.parent().unwrap()).unwrap();
    std::fs::write(&绝对, "fn a() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    改活动文本(&mut 世界, "// 改过\n");
    assert!(
        世界
            .resource::<UiStateHolder>()
            .状态
            .脚本
            .当前标签有未保存修改(),
        "改过文本后必须有未保存标记"
    );

    推动作(&mut 世界, UiAction::保存脚本);
    执行动作(&mut 世界);

    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        let 标签 = 界面.脚本.当前标签().unwrap();
        assert!(!标签.文档.有未保存修改(), "写盘成功后星号必须清掉");
        assert_eq!(
            标签.文档.已保存文本, 标签.文档.文本,
            "保存基线应当更新为当前文本"
        );
    }
    let 磁盘 = std::fs::read_to_string(&绝对).expect("应当能读回文件");
    assert_eq!(磁盘, "fn a() {}\n// 改过\n", "磁盘内容必须是缓冲区全文");
}

#[test]
fn 关闭脏标签先确认且保存成功后关闭() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "关闭确认");
    let 绝对 = 目录.path().join("关闭确认/src/a.rs");
    std::fs::create_dir_all(绝对.parent().unwrap()).unwrap();
    std::fs::write(&绝对, "fn a() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    改活动文本(&mut 世界, "// 改过\n");

    推动作(&mut 世界, UiAction::关闭脚本标签(0));
    执行动作(&mut 世界);

    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        assert!(
            界面.脚本.待确认 == Some(脚本待确认::关闭 { 索引: 0 }),
            "关闭脏标签必须先询问"
        );
        assert_eq!(界面.脚本.标签.len(), 1, "询问期间标签不能提前消失");
    }
    assert_eq!(
        std::fs::read_to_string(&绝对).unwrap(),
        "fn a() {}\n",
        "询问期间不该写盘"
    );

    推动作(&mut 世界, UiAction::脚本确认保存);
    执行动作(&mut 世界);

    let 界面 = &世界.resource::<UiStateHolder>().状态;
    assert!(界面.脚本.标签.is_empty(), "保存成功后应当关闭标签");
    assert!(界面.脚本.待确认.is_none(), "确认完成后浮层必须收起");
    assert_eq!(
        std::fs::read_to_string(&绝对).unwrap(),
        "fn a() {}\n// 改过\n",
        "确认保存必须先写盘再关闭"
    );
}

#[test]
fn 放弃修改关闭标签不写盘() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "放弃关闭");
    let 绝对 = 目录.path().join("放弃关闭/src/a.rs");
    std::fs::create_dir_all(绝对.parent().unwrap()).unwrap();
    std::fs::write(&绝对, "fn a() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    改活动文本(&mut 世界, "// 不要了\n");

    推动作(&mut 世界, UiAction::关闭脚本标签(0));
    执行动作(&mut 世界);
    推动作(&mut 世界, UiAction::脚本放弃修改);
    执行动作(&mut 世界);

    let 界面 = &世界.resource::<UiStateHolder>().状态;
    assert!(界面.脚本.标签.is_empty(), "放弃修改后标签应当关闭");
    assert_eq!(
        std::fs::read_to_string(&绝对).unwrap(),
        "fn a() {}\n",
        "放弃修改绝不能写盘"
    );
}

#[test]
fn 打开另一个文件时确认保存再装载新文件() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "打开确认保存");
    let 甲 = 目录.path().join("打开确认保存/src/a.rs");
    std::fs::create_dir_all(甲.parent().unwrap()).unwrap();
    std::fs::write(&甲, "fn a() {}\n").unwrap();
    std::fs::write(目录.path().join("打开确认保存/src/b.rs"), "fn b() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    改活动文本(&mut 世界, "// 改过\n");

    推动作(&mut 世界, UiAction::打开脚本(项目相对路径("src/b.rs")));
    执行动作(&mut 世界);

    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        assert!(
            界面.脚本.待确认
                == Some(脚本待确认::打开 {
                    路径: 项目相对路径("src/b.rs")
                }),
            "当前文件脏时打开别的文件必须先询问"
        );
        assert!(界面.脚本.待读取.is_none(), "确认之前不能开始读新文件");
    }

    推动作(&mut 世界, UiAction::脚本确认保存);
    执行动作(&mut 世界);

    assert_eq!(
        std::fs::read_to_string(&甲).unwrap(),
        "fn a() {}\n// 改过\n",
        "确认保存必须先写盘"
    );
    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        assert!(!界面.脚本.标签[0].文档.有未保存修改(), "保存后星号要清掉");
        assert_eq!(
            界面.脚本.待读取.as_ref(),
            Some(&项目相对路径("src/b.rs")),
            "确认保存后应当继续读取新文件"
        );
    }

    调度(&mut 世界);
    assert!(等待读取结束(&mut 世界), "后台读取必须在上限内完成");
    let 界面 = &世界.resource::<UiStateHolder>().状态;
    assert_eq!(界面.脚本.标签.len(), 2, "新文件应当作为第二个标签打开");
    assert_eq!(界面.脚本.活动, 1);
    assert_eq!(当前文件名(&世界), "b.rs");
}

#[test]
fn 打开另一个文件时放弃修改会把缓冲区退回磁盘版本() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "打开确认放弃");
    let 甲 = 目录.path().join("打开确认放弃/src/a.rs");
    std::fs::create_dir_all(甲.parent().unwrap()).unwrap();
    std::fs::write(&甲, "fn a() {}\n").unwrap();
    std::fs::write(目录.path().join("打开确认放弃/src/b.rs"), "fn b() {}\n").unwrap();

    打开文件(&mut 世界, "src/a.rs");
    改活动文本(&mut 世界, "// 不要了\n");

    推动作(&mut 世界, UiAction::打开脚本(项目相对路径("src/b.rs")));
    执行动作(&mut 世界);
    推动作(&mut 世界, UiAction::脚本放弃修改);
    执行动作(&mut 世界);

    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        let 甲标签 = &界面.脚本.标签[0];
        assert_eq!(
            甲标签.文档.文本, "fn a() {}\n",
            "放弃修改的语义是退回磁盘版本，而不是留一份脏缓冲区"
        );
        assert!(!甲标签.文档.有未保存修改());
        assert_eq!(
            界面.脚本.待读取.as_ref(),
            Some(&项目相对路径("src/b.rs")),
            "放弃后应当继续读取新文件"
        );
    }

    调度(&mut 世界);
    assert!(等待读取结束(&mut 世界), "后台读取必须在上限内完成");
    let 界面 = &世界.resource::<UiStateHolder>().状态;
    assert_eq!(界面.脚本.标签.len(), 2);
    assert_eq!(当前文件名(&世界), "b.rs");
    assert_eq!(
        std::fs::read_to_string(&甲).unwrap(),
        "fn a() {}\n",
        "放弃修改绝不能写盘"
    );
}
