//! 构建与动画两条新工作流的端到端验证。
//!
//! # 这里验证的是"真的能跑通"，不是"接口存在"
//!
//! * **构建**：在临时目录里放一个真实的最小 Cargo 工程，启动真实 `cargo build`，
//!   等它跑完，检查诊断里的文件/行号、成败判定与产物路径。
//!   故意写错的用例保证"编译错误真的被解析出来"，
//!   而不是靠构造一段假 JSON 自欺。
//! * **动画**：建项目 → 建实体 → 录关键帧 → 存盘 → 重新打开 → 求值。
//!   同时验证播放**不修改场景文档**（这是编辑态/运行态隔离的核心约束）。
//!
//! 跑法：`cargo test -p editor_app --test 构建与动画 -- --test-threads=1`
//! （真实子进程与临时目录需要串行，避免 cargo 的 target 目录互相干扰）。

use std::path::Path;
use std::time::{Duration, Instant};

use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::plugin::{EditorStateHolder, UiStateHolder};
use editor_core::animation::动画属性;
use editor_core::build::{构建状态, 构建配置, 诊断级别};
use editor_core::project::项目描述;
use editor_core::state::编辑器状态;
use scene_doc::path::项目相对路径;

/// 在临时目录里建一个最小 Cargo 工程，返回 (临时目录, 工程根)。
///
/// 工程没有任何第三方依赖：测试不会联网，也不会因为版本漂移失败。
fn 建临时工程(main_rs: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join("mini_game");
    std::fs::create_dir_all(根.join("src")).expect("应当能创建 src");
    std::fs::write(
        根.join("Cargo.toml"),
        r#"[package]
name = "mini_game"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "mini_game"
path = "src/main.rs"
"#,
    )
    .expect("应当能写入 Cargo.toml");
    std::fs::write(根.join("src").join("main.rs"), main_rs).expect("应当能写入 main.rs");
    (目录, 根)
}

/// 跑完一次构建并返回宿主（阻塞直到构建结束或超时）。
fn 跑完构建(根: &Path, 配置: &构建配置) -> 构建宿主 {
    let mut 宿主 = 构建宿主::新建();
    宿主.启动(根, 配置).expect("构建应当能启动");
    let 起始 = Instant::now();
    while 宿主.在运行() {
        let _ = 宿主.轮询();
        if 起始.elapsed() > Duration::from_secs(180) {
            panic!("构建超时（180 秒）：cargo 可能卡住了");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // 通道可能还有最后几行没读完。
    let _ = 宿主.轮询();
    宿主
}

#[test]
fn 构建成功时报告产物路径() {
    let (_目录, 根) = 建临时工程("fn main() {\n    println!(\"hi\");\n}\n");
    let 宿主 = 跑完构建(&根, &构建配置::default());
    let 报告 = 宿主.报告();

    assert_eq!(
        报告.状态,
        构建状态::成功,
        "最小工程应当构建成功；诊断：{:?}，摘要：{:?}",
        报告.诊断,
        报告.摘要文本()
    );
    assert_eq!(报告.错误数(), 0);
    let 产物 = 报告.主产物().expect("成功构建应当记录可执行产物");
    assert!(产物.exists(), "产物路径必须真实存在：{}", 产物.display());
    let 名 = 产物.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        名.starts_with("mini_game"),
        "产物应当是本包的二进制，实际 {名}"
    );
    // 产物目录遵循 Cargo 约定。
    assert!(报告.状态.已结束());
}

#[test]
fn 编译错误被解析成可跳转诊断() {
    let (_目录, 根) = 建临时工程(
        "fn main() {\n    // 下面这行是故意的类型错误\n    let 数: i32 = \"不是数字\";\n    println!(\"{}\", 数);\n}\n",
    );
    let 宿主 = 跑完构建(&根, &构建配置::default());
    let 报告 = 宿主.报告();

    assert_eq!(报告.状态, 构建状态::失败, "类型错误必须让构建失败");
    assert!(报告.错误数() >= 1, "应当至少解析出一条错误诊断");
    let 错误 = 报告
        .诊断
        .iter()
        .find(|诊断| 诊断.级别 == 诊断级别::错误)
        .expect("应当有错误诊断");
    assert!(错误.可跳转(), "错误应当带可跳转位置：{错误:?}");
    let 位置 = 错误.位置.as_ref().unwrap();
    assert_eq!(
        位置.行, 3,
        "错误行号应当是第 3 行（let 数: i32 = ...），实际 {位置:?}"
    );
    assert!(
        位置
            .文件
            .as_ref()
            .is_some_and(|文件| 文件.display().to_string().contains("main.rs")),
        "错误文件应当是 main.rs，实际 {位置:?}"
    );
    // 首个错误位置就是面板"跳到第一个错误"用的那个。
    assert!(报告.首个错误位置().is_some());
}

#[test]
fn 没有清单时构建如实失败而不是假装成功() {
    let 目录 = tempfile::tempdir().unwrap();
    let mut 宿主 = 构建宿主::新建();
    assert!(宿主.启动(目录.path(), &构建配置::default()).is_err());
    assert_eq!(宿主.报告().状态, 构建状态::失败);
    assert!(
        宿主
            .报告()
            .摘要文本()
            .unwrap_or_default()
            .contains("Cargo.toml"),
        "摘要应当指明缺少 Cargo.toml"
    );
}

/// 建一个已打开的项目。
fn 建项目() -> (tempfile::TempDir, 编辑器状态) {
    let 目录 = tempfile::tempdir().expect("应当能创建临时目录");
    let 根 = 目录.path().join("动画工程");
    let 项目 = 项目描述::创建(&根, "动画工程").expect("创建项目应当成功");
    let mut 编辑器 = 编辑器状态::新建();
    编辑器.项目 = Some(项目);
    (目录, 编辑器)
}

#[test]
fn 动画保存后重新打开关键帧仍在() {
    let (_目录, mut 编辑器) = 建项目();
    let 实体 = 编辑器.新建实体("主角", None).expect("应当能建实体");
    // 让实体有一个非零位置，录下来的关键帧才有辨识度。
    if let Some(项) = 编辑器.会话.文档.取可变(实体)
        && let Some(变换) = 项.变换.as_mut()
    {
        变换.平移 = (5.0, 0.0, 0.0);
    }
    编辑器.新建动画("跳跃");
    编辑器.动画播放.时间 = 0.0;
    assert_eq!(
        编辑器.录制选中实体关键帧(&[动画属性::平移]),
        1,
        "选中实体后应当在播放头处录到一帧"
    );
    // 移到 1 秒再录一帧。
    编辑器.动画播放.时间 = 1.0;
    if let Some(项) = 编辑器.会话.文档.取可变(实体)
        && let Some(变换) = 项.变换.as_mut()
    {
        变换.平移 = (9.0, 2.0, 0.0);
    }
    assert_eq!(编辑器.录制选中实体关键帧(&[动画属性::平移]), 1);

    let 路径 = 编辑器.默认动画路径().expect("应当能推出默认动画路径");
    assert!(
        路径.文本().starts_with("animations/"),
        "默认路径应当在 animations/ 下，实际 {}",
        路径.文本()
    );
    编辑器.保存动画到(&路径).expect("保存动画应当成功");
    assert!(!编辑器.动画有未保存修改(), "保存后不该还有脏标记");

    // 场景也必须落盘：动画按实体标识引用实体，没有场景就没有可求值的对象。
    // 这正是真实工作流（保存场景 + 保存动画 → 重新打开项目与场景）。
    let 场景路径 = 项目相对路径::解析("scenes/主角.scene.ron").unwrap();
    编辑器.保存场景到(&场景路径).expect("保存场景应当成功");

    // 换一份全新的编辑器状态，从磁盘读回来 —— 这才是"重新打开"。
    let 原始项目 = 编辑器.项目.clone().expect("项目应当在");
    let mut 新编辑器 = 编辑器状态::新建();
    新编辑器.项目 = Some(原始项目);
    新编辑器.打开场景(&场景路径).expect("重新打开场景应当成功");
    新编辑器.打开动画(&路径).expect("重新打开动画应当成功");

    let 会话 = 新编辑器.动画.as_ref().expect("应当有动画");
    assert_eq!(会话.文档.轨道数量(), 1);
    assert_eq!(会话.文档.关键帧总数(), 2);

    // 求值：把播放头放到 0.5 秒，应当落在两帧中间。
    新编辑器.动画定位(0.5);
    let 结果 = 新编辑器.动画求值();
    assert_eq!(结果.len(), 1, "只有一条轨道被求值");
    assert!(
        (结果[0].1.平移.0 - 7.0).abs() < 1e-3,
        "0.5 秒处 (5 → 9) 应当是 7，实际 {}",
        结果[0].1.平移.0
    );
}

#[test]
fn 动画求值在中间时刻插值且不写场景文档() {
    let (_目录, mut 编辑器) = 建项目();
    let 实体 = 编辑器.新建实体("方块", None).expect("应当能建实体");
    编辑器.新建动画("移动");
    // 0 秒：x = 0
    if let Some(项) = 编辑器.会话.文档.取可变(实体)
        && let Some(变换) = 项.变换.as_mut()
    {
        变换.平移 = (0.0, 0.0, 0.0);
    }
    编辑器.动画播放.时间 = 0.0;
    编辑器.录制选中实体关键帧(&[动画属性::平移]);
    // 2 秒：x = 10
    if let Some(项) = 编辑器.会话.文档.取可变(实体)
        && let Some(变换) = 项.变换.as_mut()
    {
        变换.平移 = (10.0, 0.0, 0.0);
    }
    编辑器.动画播放.时间 = 2.0;
    编辑器.录制选中实体关键帧(&[动画属性::平移]);

    // 播放头在 1 秒：求值应给出 x = 5。
    编辑器.动画播放.时间 = 1.0;
    let 结果 = 编辑器.动画求值();
    let (结果实体, 变换) = 结果[0];
    assert_eq!(结果实体, 实体);
    assert!(
        (变换.平移.0 - 5.0).abs() < 1e-4,
        "1 秒处应当是插值中点 5，实际 {}",
        变换.平移.0
    );

    // 关键：文档里的位置仍是最后一次编辑的 10，不是求值结果。
    let 文档变换 = 编辑器.会话.文档.取(实体).unwrap().变换.unwrap();
    assert_eq!(文档变换.平移.0, 10.0, "动画求值绝不能写回场景文档");
}

#[test]
fn 删除实体后动画轨道被如实报告为悬空() {
    let (_目录, mut 编辑器) = 建项目();
    let 实体 = 编辑器.新建实体("会被删掉", None).expect("应当能建实体");
    编辑器.新建动画("a");
    编辑器.动画播放.时间 = 0.0;
    编辑器.录制选中实体关键帧(&[动画属性::平移]);

    // 从文档里删掉实体，动画文档里仍保留轨道。
    编辑器.会话.文档.删除实体(实体);
    let 会话 = 编辑器.动画.as_ref().unwrap();
    let 问题 = 会话.文档.校验(&编辑器.会话.文档);
    assert!(
        问题.iter().any(|问题| 问题.contains("已不在场景里")),
        "应当报告悬空轨道，实际：{问题:?}"
    );
    // 求值不会凭空复活已删除的实体。
    assert!(编辑器.动画求值().is_empty());
}

#[test]
fn 打开诊断会把源码装进脚本面板并定位到行() {
    let (_目录, mut 编辑器) = 建项目();
    // 项目里写一个真实的源文件。
    let 根 = 编辑器.项目.as_ref().unwrap().根目录().to_path_buf();
    std::fs::create_dir_all(根.join("src")).unwrap();
    let 源文件 = 根.join("src").join("game.rs");
    std::fs::write(&源文件, "fn main() {\n    let a = 1;\n    let b = 2;\n}\n").unwrap();

    let mut 界面 = editor_ui::界面状态::新建();
    // 诊断里的路径是**相对项目根**的（rustc 就这么给）。
    actions::打开源码位置(&mut 编辑器, &mut 界面, Some(Path::new("src/game.rs")), 3);

    界面.脚本.文档.as_ref().expect("脚本面板应当装入了文件");
    assert_eq!(界面.脚本.滚动到行, Some(2), "行号应当转成 0 基");
    let 文本 = &界面.脚本.文档.as_ref().unwrap().文本;
    assert!(文本.contains("let b = 2;"), "读到的内容应当是真实文件");
    assert!(
        界面.面板可见(editor_ui::Tab::脚本),
        "定位后应当让脚本面板可见"
    );
}

#[test]
fn 构建配置写进项目清单并往返() {
    let (_目录, mut 编辑器) = 建项目();
    let 配置 = 构建配置 {
        配置类型: editor_core::build::构建配置类型::Release,
        目标平台: Some("x86_64-pc-windows-msvc".into()),
        包名: Some("mini_game".into()),
        特征: vec!["a".into(), "b".into()],
        额外参数: vec![],
        二进制名: None,
    };
    let 项目 = 编辑器.项目.as_mut().unwrap();
    项目.清单可变().构建 = Some(配置.clone());
    项目.保存清单().expect("保存清单应当成功");

    // 从磁盘重读：不清空项目也不换目录，直接重新打开同一个根。
    let 根 = 项目.根目录().to_path_buf();
    let 重开 = 项目描述::打开(&根).expect("重新打开项目应当成功");
    assert_eq!(
        重开.清单().构建.as_ref(),
        Some(&配置),
        "构建配置必须能从 project.ron 读回来"
    );
}

/// 动作队列层面的动画闭环：录帧 → 播放 → 停止。
///
/// 与上面直接调 API 的用例互补：这条路径多验证了"动作分派真的接上了"。
#[test]
fn 动作队列能驱动动画录制与播放() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use editor_ui::state::{UiAction, 选择意图};

    let 目录 = tempfile::tempdir().unwrap();
    let 根 = 目录.path().join("动作动画");
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(editor_app::scene_bridge::运行时宿主::default());
    世界.insert_resource(editor_app::plugin::文件对话框通道::default());
    世界.insert_resource(editor_app::plugin::诊断资源::default());
    世界.insert_resource(editor_app::plugin::缩略图缓存::default());
    世界.insert_resource(editor_app::config::布局待保存::default());
    世界.insert_resource(editor_app::run::项目运行管理器::default());
    世界.insert_resource(构建宿主::新建());
    世界.insert_resource(editor_app::config::编辑器配置资源::default());
    世界.insert_resource(editor_app::plugin::导入快照资源::default());

    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.创建项目(&根, "动作动画").expect("建项目");
    }

    // 新建实体并选中，然后走动作：新建动画 → 录关键帧。
    let 实体 = {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        编辑器.状态.新建实体("甲", None).expect("建实体")
    };
    let 动作列表 = vec![
        UiAction::选择(选择意图::单选(实体)),
        UiAction::新建动画,
        UiAction::录制动画关键帧 {
            属性: vec![动画属性::全部[0]],
        },
    ];
    {
        let mut 界面 = 世界.resource_mut::<UiStateHolder>();
        for 动作 in 动作列表 {
            界面.动作.推(动作);
        }
    }
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列应当能运行");

    {
        let 编辑器 = 世界.resource::<EditorStateHolder>();
        let 会话 = 编辑器.状态.动画.as_ref().expect("应当有新动画");
        assert_eq!(会话.文档.关键帧总数(), 1, "应当录到一帧");
    }

    // 播放 → 推进时间 → 停止回到 0。
    for 动作 in [UiAction::播放动画, UiAction::设置动画时长(1.0)] {
        let mut 界面 = 世界.resource_mut::<UiStateHolder>();
        界面.动作.推(动作);
    }
    世界.run_system_once(actions::执行动作队列).unwrap();
    {
        let mut 编辑器 = 世界.resource_mut::<EditorStateHolder>();
        assert!(编辑器.状态.动画播放.播放中, "播放动作应当置位播放状态");
        编辑器.状态.动画播放.时间 = 0.5;
    }
    {
        let mut 界面 = 世界.resource_mut::<UiStateHolder>();
        界面.动作.推(UiAction::停止动画);
    }
    世界.run_system_once(actions::执行动作队列).unwrap();
    let 编辑器 = 世界.resource::<EditorStateHolder>();
    assert!(!编辑器.状态.动画播放.播放中);
    assert_eq!(编辑器.状态.动画播放.时间, 0.0, "停止应当回到 0 秒");
}

/// 默认动画路径必须落在项目内且带 `.anim.ron` 后缀。
#[test]
fn 默认动画路径规范() {
    let (_目录, mut 编辑器) = 建项目();
    编辑器.新建动画("x");
    let 路径: 项目相对路径 = 编辑器.默认动画路径().expect("应当有默认路径");
    assert!(路径.文本().ends_with(".anim.ron"), "实际 {}", 路径.文本());
    assert!(!路径.文本().contains('\\'), "项目相对路径用 / 分隔");
}

/// 播放中的动画必须通过"覆盖预览实体的 Transform"体现出来，
/// 而**不能**改动场景文档。
///
/// # 这条测试钉住的是什么
///
/// `应用动画预览` 是"时间轴 → 画面"的唯一通路。它一旦漏接（没注册、
/// 或者被后来的预览同步系统覆盖掉），界面上就是"时间在走、物体不动"——
/// 而这在截图里很难和"根本没播放"区分开。这里直接读 ECS 组件断言。
#[test]
fn 动画播放覆盖预览变换而不改文档() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use editor_app::preview::{预览实体, 预览来源};

    let (_目录, mut 编辑器) = 建项目();
    let 实体 = 编辑器.新建实体("方块", None).expect("建实体");
    编辑器.新建动画("移动");
    // 0 秒 → x=0，1 秒 → x=10。
    let mut 设置平移 = |x: f32, 时间: f32| {
        if let Some(项) = 编辑器.会话.文档.取可变(实体)
            && let Some(变换) = 项.变换.as_mut()
        {
            变换.平移 = (x, 0.0, 0.0);
        }
        编辑器.动画播放.时间 = 时间;
        编辑器.录制选中实体关键帧(&[动画属性::平移]);
    };
    设置平移(0.0, 0.0);
    设置平移(10.0, 1.0);
    // 文档里的姿态停在 10；播放头放到 0.5 秒。
    if let Some(项) = 编辑器.会话.文档.取可变(实体)
        && let Some(变换) = 项.变换.as_mut()
    {
        变换.平移 = (10.0, 0.0, 0.0);
    }
    编辑器.动画播放.时间 = 0.5;
    编辑器.动画播放.播放中 = true;

    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder { 状态: 编辑器 });
    let 预览 = 世界
        .spawn((
            预览实体,
            预览来源(实体),
            Transform::from_xyz(10.0, 0.0, 0.0),
        ))
        .id();

    世界
        .run_system_once(actions::应用动画预览)
        .expect("应用动画预览应当能运行");

    let 变换 = 世界.get::<Transform>(预览).expect("预览实体应当还在");
    assert!(
        (变换.translation.x - 5.0).abs() < 1e-3,
        "0.5 秒处预览应当插值到 x=5，实际 {}",
        变换.translation.x
    );

    // 文档必须保持原样：动画不能写回场景。
    let 文档平移 = 世界
        .resource::<EditorStateHolder>()
        .状态
        .会话
        .文档
        .取(实体)
        .unwrap()
        .变换
        .unwrap()
        .平移
        .0;
    assert_eq!(文档平移, 10.0, "播放动画绝不能修改场景文档");
}
