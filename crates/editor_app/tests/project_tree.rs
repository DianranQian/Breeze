//! 中文名：项目文件树与代码装载
//! 文件树扫描、刷新动作与"后台读盘"的集成测试。
//!
//! # 这里验证的不是 API，而是三条用户可见的性质
//!
//! 1. **扫描的边界**：`src/` 与根目录核心文件要收进来，`target/` 这类
//!    构建产物绝不能出现在树里 —— 否则用户在文件树里点开的会是旧的
//!    编译产物，而不是自己写的源码；
//! 2. **打开文件不占主线程**：`UiAction::打开脚本` 执行完就应当返回，
//!    文档为空、状态是"正在读取"；文本由后台线程读回、由轮询系统认领；
//! 3. **读不到要说清楚**：错误进日志，界面不能永远停在"正在读取"。
//!
//! 全部用例走 `World::run_system_once`，不需要窗口与 GPU。
//! 后台线程的真实性靠回执里的 `读取线程` 与主线程比较来钉住 ——
//! 这比计时要稳定，不随机器负载抖动。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::code_loader::{代码读取通道, 调度代码读取, 轮询代码读取};
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 文件对话框通道, 缩略图缓存, 诊断资源
};
use editor_app::scene_bridge::运行时宿主;
use editor_core::state::日志级别;
use editor_ui::state::UiAction;
use editor_ui::开发模式;

// ======================================================================
// 测试脚手架
// ======================================================================

/// 组装"动作执行 + 代码装载"所需的最小世界。
///
/// 与 `action_loop.rs` 的资源清单保持一致：漏掉任何一个资源，
/// `执行动作队列` 的 SystemParam 校验会让**所有**用例一起失败。
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

/// 建一个临时项目并打开它。
fn 建项目(世界: &mut World, 名称: &str) -> tempfile::TempDir {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join(名称);
    let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
    编辑器.状态.创建项目(&根, 名称).expect("创建项目应当成功");
    目录
}

/// 写一个项目内的文件（自动创建父目录）。
fn 写(路径: &std::path::Path, 内容: &str) {
    if let Some(父) = 路径.parent() {
        std::fs::create_dir_all(父).expect("应当能创建父目录");
    }
    std::fs::write(路径, 内容).expect("应当能写测试文件");
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

// ======================================================================
// 用例
// ======================================================================

#[test]
fn 刷新文件树只收代码与核心文件并忽略构建产物() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "树扫描");
    let 根 = 目录.path().join("树扫描");
    写(&根.join("src/main.rs"), "fn main() {}\n");
    写(&根.join("src/子目录/工具.rs"), "pub fn 工具() {}\n");
    写(&根.join("README.md"), "# 说明\n");
    写(
        &根.join("Cargo.toml"),
        "[package]\nname = \"树扫描\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    // 这些都不该进树。
    写(&根.join("target/debug/旧产物.rs"), "fn 旧() {}\n");
    写(&根.join("node_modules/包/index.js"), "// js\n");

    推动作(&mut 世界, UiAction::刷新项目文件树);
    执行动作(&mut 世界);

    let 项目 = 世界.resource::<EditorStateHolder>().状态.项目.clone();
    let 树 = 世界
        .resource_mut::<UiStateHolder>()
        .状态
        .文件树(项目.as_ref());
    let 路径们 = 树.全部路径();

    assert!(路径们.contains(&"src/main.rs".to_string()), "{路径们:?}");
    assert!(
        路径们.iter().any(|路径| 路径.ends_with("工具.rs")),
        "子目录里的源码也要收：{路径们:?}"
    );
    assert!(
        路径们.iter().any(|路径| 路径 == "Cargo.toml"),
        "根目录核心文件要收：{路径们:?}"
    );
    assert!(
        !路径们.iter().any(|路径| 路径.contains("target")),
        "构建产物绝不能进文件树：{路径们:?}"
    );
    assert!(
        !路径们.iter().any(|路径| 路径.contains("node_modules")),
        "依赖目录绝不能进文件树：{路径们:?}"
    );
}

#[test]
fn 打开脚本立即返回并由后台线程装载() {
    let mut 世界 = 建测试世界();
    let 目录 = 建项目(&mut 世界, "后台读取");
    let 根 = 目录.path().join("后台读取");
    写(&根.join("src/main.rs"), "fn main() {\n    let 值 = 1;\n}\n");
    let 路径: scene_doc::path::项目相对路径 =
        scene_doc::path::项目相对路径::解析("src/main.rs").expect("合法路径");

    推动作(&mut 世界, UiAction::打开脚本(路径.clone()));
    执行动作(&mut 世界);

    {
        let 界面 = &世界.resource::<UiStateHolder>().状态;
        assert!(
            界面.脚本.文档.is_none(),
            "动作执行完时文本还没读到：读盘必须发生在后台线程"
        );
        assert!(界面.脚本.正在读取(), "界面要能显示正在读取");
        assert_eq!(界面.脚本.待读取.as_ref(), Some(&路径));
        assert_eq!(界面.模式, 开发模式::脚本, "打开脚本应当切到脚本模式");
    }

    调度(&mut 世界);
    assert!(
        世界.resource::<UiStateHolder>().状态.脚本.待读取.is_none(),
        "意图交给后台线程后应当被取走"
    );

    assert!(等待读取结束(&mut 世界), "后台读取必须在上限内完成");
    let 界面 = &世界.resource::<UiStateHolder>().状态;
    let 文档 = 界面.脚本.文档.as_ref().expect("读取成功后应当有文档");
    assert!(
        文档.文本.contains("let 值"),
        "装载的必须是真实文件内容：{}",
        文档.文本
    );
    assert_eq!(文档.路径.as_ref(), Some(&路径));
    assert!(!界面.脚本.正在读取(), "装载完成后不应停在正在读取");

    let 读取线程 = 世界
        .resource::<代码读取通道>()
        .最近读取线程
        .expect("应当记录执行读盘的线程");
    assert_ne!(
        读取线程,
        std::thread::current().id(),
        "读盘必须发生在后台线程：与主线程相同就说明主线程在同步读盘"
    );
    assert!(
        世界
            .resource::<EditorStateHolder>()
            .状态
            .日志列表()
            .iter()
            .any(|条目| matches!(条目.内容, editor_core::state::日志内容::结构化 { 标识, .. } if 标识 == "log_script_opened")),
        "成功打开应当留下结构化日志"
    );
}

#[test]
fn 读不到的文件报错并清掉正在读取() {
    let mut 世界 = 建测试世界();
    建项目(&mut 世界, "缺失文件");
    let 路径 = scene_doc::path::项目相对路径::解析("src/不存在.rs").expect("合法路径");

    推动作(&mut 世界, UiAction::打开脚本(路径));
    执行动作(&mut 世界);
    调度(&mut 世界);

    assert!(等待读取结束(&mut 世界), "错误回执也要结束正在读取状态");
    let 界面 = &世界.resource::<UiStateHolder>().状态;
    assert!(界面.脚本.文档.is_none(), "读不到时不能留下半份文档");
    assert!(!界面.脚本.正在读取());

    let 编辑器 = &世界.resource::<EditorStateHolder>().状态;
    assert!(
        编辑器
            .日志列表()
            .iter()
            .any(|条目| 条目.级别 == 日志级别::错误),
        "读不到文件必须报错，而不是静默失败"
    );
}
