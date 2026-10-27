//! 中文名：材质编辑器闭环
//! 双击材质文件 → 草稿 → 保存回 `.material.ron` → 代次推进 → 视口重取材质。
//!
//! # 为什么这组测试存在
//!
//! 材质编辑器这条链上有四种「看着都对，其实没接上」的坏法，全都不崩溃：
//!
//! 1. **打开**读的是草稿而不是磁盘 —— 用户看到的是上一次没落盘的旧值；
//! 2. **保存**写了文件却没推进注册表代次 —— 视口的材质缓存以「路径 + 代次」为键，
//!    代次不动就等于「改了材质但 3D 视口不刷新」，这是最容易被误判成渲染坏了的缺陷；
//! 3. **保存**把草稿写成另一条路径 —— 名字对了，改的却是别的文件；
//! 4. **关闭**静默丢掉未保存的改动 —— 用户调了半天的材质一次点击归零。
//!
//! 因此这里走**真实动作队列**（与用户点击同一条路）、真实临时项目、真实
//! `同步预览实体` 系统：断言的不只是「草稿变了」，而是磁盘内容、注册表代次
//! 与渲染层拿到的那份 `StandardMaterial` 三处同时变了。
//!
//! # 为什么不在这里断言控件长什么样
//!
//! 字段控件由 `材质资源::字段描述()` 驱动、与 Inspector 共用同一个渲染函数，
//! 描述与文档之间的往返由 `editor_core` 的测试钉住；本文件只负责「接线」。

use std::collections::BTreeMap;

use bevy::asset::AssetPlugin;
use bevy::ecs::system::RunSystemOnce;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{布局待保存, 编辑器配置资源};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::preview::{
    三维预览资源, 同步预览实体, 材质缓存, 贴图缓存, 预览同步状态, 预览来源
};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_app::tile_terrain::地形缓存;
use editor_core::material::材质资源;
use editor_core::project::项目描述;
use editor_core::state::{场景会话, 日志内容, 日志级别, 编辑器状态};
use editor_ui::state::{UiAction, 材质编辑会话, 资源落点};
use scene_doc::document::{变换数据, 场景文档, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 枚举值, 资源引用值, 资源类别, 颜色值};

const 材质路径文本: &str = "assets/materials/石板.material.ron";
const 粗糙度字段: &str = "粗糙度";
const 名称字段: &str = "名称";

/// 一份字段取值都不同于默认的材质，作为「磁盘上的初始真相」。
fn 磁盘材质() -> 材质资源 {
    let mut 材质 = 材质资源::新建("石板路");
    材质.着色颜色 = 颜色值::srgb(0.2, 0.4, 0.6, 1.0);
    材质.粗糙度 = 0.25;
    材质.金属度 = 0.8;
    材质
}

fn 材质路径() -> 项目相对路径 {
    项目相对路径::解析(材质路径文本).expect("测试里的路径必须合法")
}

fn 单位变换() -> 变换数据 {
    变换数据 {
        平移: (0.0, 0.0, 0.0),
        旋转: (0.0, 0.0, 0.0, 1.0),
        缩放: (1.0, 1.0, 1.0),
    }
}

/// 一个引用了这份材质的 `Primitive3d` 实体：视口要按它去取材质。
fn 带材质实体的文档() -> (场景文档, 实体标识) {
    let mut 文档 = 场景文档::新建("材质编辑测试");
    let 标识 = 文档.生成实体("石板", None);
    let 实体 = 文档.取可变(标识).expect("刚生成的实体一定在");
    实体.变换 = Some(单位变换());
    let mut 属性 = BTreeMap::new();
    属性.insert("形状".to_string(), 属性值::枚举(枚举值::新建("Cuboid", 0)));
    属性.insert("尺寸".to_string(), 属性值::三维(4.0, 1.0, 4.0));
    属性.insert("受光".to_string(), 属性值::布尔(true));
    属性.insert(
        "材质".to_string(),
        属性值::资源(资源引用值::按路径(材质路径())),
    );
    实体.加组件(组件数据::新建("Primitive3d", 属性));
    (文档, 标识)
}

/// 建一个「真实项目 + 真实材质文件 + 一个引用它的实体」的应用。
///
/// 资源分两套插进同一个 `App`：动作队列那套（与生产同一批资源）与
/// `同步预览实体` 那套。用同一个世界是故意的 —— 「保存后视口刷新」这件事
/// 只有当两边看的是同一个注册表实例时才谈得上成立。
///
/// 临时目录由调用方持有：`TempDir` 一析构项目根就没了。
fn 建应用(用例: &str) -> (App, tempfile::TempDir, 实体标识) {
    let 目录 = tempfile::tempdir().expect("创建临时目录");
    let 根 = 目录.path().join(用例);
    let 项目 = 项目描述::创建(&根, 用例).expect("创建项目");
    项目
        .保存材质(&材质路径(), &磁盘材质())
        .expect("写入初始材质");

    let mut 编辑器 = 编辑器状态::新建();
    编辑器.打开项目(&根).expect("打开项目");
    let (文档, 实体) = 带材质实体的文档();
    编辑器.会话 = 场景会话 {
        文档, 路径: None
    };

    let mut 应用 = App::new();
    应用.add_plugins((MinimalPlugins, AssetPlugin::default()));
    应用.init_resource::<Assets<StandardMaterial>>();
    应用.init_resource::<Assets<Image>>();
    应用.init_resource::<Assets<Mesh>>();
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
    世界.insert_resource(贴图缓存::default());
    世界.insert_resource(地形缓存::default());
    世界.insert_resource(材质缓存::default());
    世界.insert_resource(预览同步状态::default());
    世界.insert_resource(三维预览资源 {
        四边形: Handle::default(),
    });
    应用.add_systems(Update, 同步预览实体);
    (应用, 目录, 实体)
}

/// 把一条动作推进队列并立刻执行 —— 与用户点击走同一条路。
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

fn 编辑器<'a>(应用: &'a App) -> &'a 编辑器状态 {
    &应用.world().resource::<EditorStateHolder>().状态
}

fn 界面<'a>(应用: &'a App) -> &'a editor_ui::界面状态 {
    &应用.world().resource::<UiStateHolder>().状态
}

/// 直接改草稿里的一个字段，等价于用户在浮层里拖动那个控件。
///
/// 浮层的绘制不在这里测（那要 egui 上下文）；这里只关心「草稿变了之后，
/// 保存动作落下去的是什么」。
fn 改草稿(应用: &mut App, 字段: &str, 值: 属性值) {
    let mut 持有 = 应用.world_mut().resource_mut::<UiStateHolder>();
    let 会话 = 持有.状态.材质编辑.as_mut().expect("草稿会话应当已打开");
    会话.属性.insert(字段.to_string(), 值);
    会话.脏 = true;
}

/// 把浮层「保存」按钮排出去的那条动作：路径照抄会话，材质由草稿还原。
fn 排保存(应用: &App) -> UiAction {
    let 会话 = 界面(应用).材质编辑.as_ref().expect("草稿会话应当已打开");
    UiAction::保存材质编辑器 {
        路径: 会话.路径.clone(),
        材质: 会话.取材质(),
    }
}

/// 拼出「临时目录 / 用例项目根 / 项目相对路径」的绝对路径。
///
/// 用例名必须带上：`建应用` 把项目根放在临时目录下的**同名子目录**里，
/// 直接用临时目录拼路径会指到一个不存在的地方。
fn 项目内(目录: &std::path::Path, 用例: &str, 相对: &str) -> std::path::PathBuf {
    目录
        .join(用例)
        .join(相对.replace('/', std::path::MAIN_SEPARATOR_STR))
}

fn 读磁盘材质(应用: &App) -> 材质资源 {
    编辑器(应用)
        .项目
        .as_ref()
        .expect("项目应当已打开")
        .读取材质(&材质路径())
        .expect("读取材质")
}

fn 代次(应用: &App) -> u64 {
    编辑器(应用).资源注册表.代次()
}

/// 视口渲染层实际拿到的那份材质的粗糙度。
///
/// 读的是 `perceptual_roughness` —— Bevy 给「粗糙度」起的名字，`材质资源::粗糙度`
/// 原样落到这个字段（映射由 `preview::材质转标准材质` 负责，那里已有逐字段测试）。
///
/// 走 `iter_entities` 而不是 `QueryState`：后者要 `&mut World`，而这里只想读。
fn 视口粗糙度(应用: &App, 来源: 实体标识) -> Option<f32> {
    let 世界 = 应用.world();
    let 材质表 = 世界.resource::<Assets<StandardMaterial>>();
    let 句柄 = 世界
        .iter_entities()
        .filter_map(|实体| {
            let 预览 = 实体.get::<预览来源>()?;
            if 预览.0 != 来源 {
                return None;
            }
            Some(实体.get::<MeshMaterial3d<StandardMaterial>>()?.0.clone())
        })
        .next()?;
    材质表.get(&句柄).map(|材质| 材质.perceptual_roughness)
}

fn 有结构化日志(应用: &App, 级别: 日志级别, 期望标识: &str) -> bool {
    编辑器(应用).日志列表().iter().any(|条目| {
        条目.级别 == 级别
            && matches!(&条目.内容,
                日志内容::结构化 { 标识, .. } if *标识 == 期望标识)
    })
}

#[test]
fn 打开材质编辑器把磁盘上的材质读成草稿() {
    let (mut 应用, _目录, _实体) = 建应用("打开");

    执行(
        &mut 应用,
        UiAction::打开材质编辑器(材质路径文本.to_string()),
    );

    let 会话 = 界面(&应用).材质编辑.clone().expect("应当留下一份草稿会话");
    assert_eq!(会话.路径, 材质路径文本, "会话必须记住自己对应哪个文件");
    assert!(!会话.脏, "刚从磁盘读出来的草稿与磁盘一致，不该一打开就标脏");
    assert_eq!(会话.属性.get(粗糙度字段), Some(&属性值::浮点(0.25)));
    assert_eq!(
        会话.属性.get(名称字段),
        Some(&属性值::文本("石板路".to_string())),
        "草稿要显示磁盘上的真值，而不是上一次的内存残留"
    );
    assert_eq!(
        会话.取材质(),
        磁盘材质(),
        "草稿 → 材质 的还原必须与读到的那份完全相同，否则第一次保存就是一次无意的改写"
    );
}

#[test]
fn 非法路径与读不到的材质都不打开会话() {
    let (mut 应用, _目录, _实体) = 建应用("拒绝打开");

    执行(
        &mut 应用,
        UiAction::打开材质编辑器("../越界.ron".to_string()),
    );
    assert!(界面(&应用).材质编辑.is_none(), "越界路径不能开出一个空会话");
    assert!(
        有结构化日志(&应用, 日志级别::警告, "log_material_editor_bad_path"),
        "拒绝要有日志：界面什么都没发生，用户只会以为双击没生效"
    );

    执行(
        &mut 应用,
        UiAction::打开材质编辑器("assets/materials/不存在.material.ron".to_string()),
    );
    assert!(
        界面(&应用).材质编辑.is_none(),
        "读不出内容的材质不该留下一个全默认值的会话"
    );
    assert!(
        有结构化日志(&应用, 日志级别::错误, "log_material_editor_read_failed"),
        "文件读不到是错误，不是警告"
    );
}

#[test]
fn 保存把草稿写回文件推进代次并让视口重取材质() {
    let (mut 应用, _目录, 实体) = 建应用("保存");
    let 旧代次 = 代次(&应用);

    应用.update();
    assert!(
        (视口粗糙度(&应用, 实体).expect("实体应当有渲染层材质") - 0.25).abs() < 1e-6,
        "改动之前，视口拿到的应当是磁盘上的旧粗糙度"
    );

    执行(
        &mut 应用,
        UiAction::打开材质编辑器(材质路径文本.to_string()),
    );
    改草稿(&mut 应用, 粗糙度字段, 属性值::浮点(0.9));
    let 动作 = 排保存(&应用);
    执行(&mut 应用, 动作);

    assert!(
        (读磁盘材质(&应用).粗糙度 - 0.9).abs() < 1e-6,
        "草稿里的粗糙度要真的落到 `.material.ron` 上"
    );
    assert!(
        代次(&应用) > 旧代次,
        "保存后必须推进注册表代次：视口的材质缓存按「路径 + 代次」取，代次不动视口就不刷新"
    );
    let 会话 = 界面(&应用).材质编辑.as_ref().expect("保存后浮层保持打开");
    assert!(!会话.脏, "落盘成功就要清掉未保存标记");
    assert_eq!(会话.取材质().粗糙度, 0.9);

    应用.update();
    assert!(
        (视口粗糙度(&应用, 实体).expect("实体应当有渲染层材质") - 0.9).abs() < 1e-6,
        "同一帧之后，3D 视口拿到的 `StandardMaterial` 应当是新粗糙度"
    );
}

#[test]
fn 保存走的是会话里那条路径而不是别的材质() {
    let (mut 应用, 目录, _实体) = 建应用("落点");
    执行(
        &mut 应用,
        UiAction::打开材质编辑器(材质路径文本.to_string()),
    );
    改草稿(&mut 应用, 名称字段, 属性值::文本("改过的名字".to_string()));
    let 动作 = 排保存(&应用);
    执行(&mut 应用, 动作);

    let 另一份 = 项目内(目录.path(), "落点", "assets/materials/另一份.material.ron");
    std::fs::write(&另一份, "(名称: \"别的材质\")").expect("写入另一份材质");
    assert_eq!(读磁盘材质(&应用).名称, "改过的名字");
    assert_eq!(
        材质资源::解码(
            &std::fs::read_to_string(&另一份).expect("读另一份"),
            &另一份
        )
        .expect("解码")
        .名称,
        "别的材质",
        "改一份材质不能顺手碰到同目录下的另一份"
    );
}

#[test]
fn 有未保存改动时打开另一份材质会被拒绝() {
    let (mut 应用, 目录, _实体) = 建应用("抢占");
    执行(
        &mut 应用,
        UiAction::打开材质编辑器(材质路径文本.to_string()),
    );
    改草稿(&mut 应用, 粗糙度字段, 属性值::浮点(0.9));

    let 另一份路径 = 项目内(目录.path(), "抢占", "assets/materials/另一份.material.ron");
    std::fs::write(&另一份路径, "(名称: \"另一份\")").expect("写入另一份材质");

    执行(
        &mut 应用,
        UiAction::打开材质编辑器("assets/materials/另一份.material.ron".to_string()),
    );

    let 会话 = 界面(&应用).材质编辑.as_ref().expect("会话不应被顶掉");
    assert_eq!(会话.路径, 材质路径文本, "拒绝顶掉当前草稿");
    assert!(会话.脏, "被拒绝的打开不能顺手把用户的改动标记成已保存");
    assert_eq!(会话.属性.get(粗糙度字段), Some(&属性值::浮点(0.9)));
    assert!(
        有结构化日志(&应用, 日志级别::警告, "log_material_editor_busy"),
        "拒绝必须说清原因，否则看起来像双击没反应"
    );
}

#[test]
fn 关闭丢弃草稿并留下警告且不改文件() {
    let (mut 应用, _目录, _实体) = 建应用("丢弃");
    执行(
        &mut 应用,
        UiAction::打开材质编辑器(材质路径文本.to_string()),
    );
    改草稿(&mut 应用, 粗糙度字段, 属性值::浮点(0.9));

    执行(&mut 应用, UiAction::关闭材质编辑器);

    assert!(界面(&应用).材质编辑.is_none(), "关闭就是把会话清掉");
    assert!(
        有结构化日志(&应用, 日志级别::警告, "log_material_editor_discarded"),
        "丢掉的是用户改的东西，必须留一条可查的警告"
    );
    assert!(
        (读磁盘材质(&应用).粗糙度 - 0.25).abs() < 1e-6,
        "丢弃只丢内存草稿，文件要保持磁盘上那份"
    );
}

#[test]
fn 贴图槽的选择只改草稿保存后才落盘() {
    const 贴图: &str = "assets/贴图/石板_法线.png";
    let (mut 应用, 目录, _实体) = 建应用("贴图选取");
    执行(
        &mut 应用,
        UiAction::打开材质编辑器(材质路径文本.to_string()),
    );

    let 绝对 = 项目内(目录.path(), "贴图选取", 贴图);
    std::fs::create_dir_all(绝对.parent().expect("贴图必有父目录")).expect("建贴图目录");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 200, 9, 255]))
        .save(&绝对)
        .expect("写 PNG");
    let 贴图路径 = 项目相对路径::解析(贴图).expect("测试里的路径合法");
    {
        let mut 持有 = 应用.world_mut().resource_mut::<EditorStateHolder>();
        持有
            .状态
            .项目
            .as_mut()
            .expect("项目应当已打开")
            .同步资源标识()
            .expect("同步资源标识");
        持有.状态.刷新资源注册表();
    }
    let 标识 = 编辑器(&应用)
        .资源注册表
        .取(&贴图路径)
        .expect("贴图应当被扫描进注册表")
        .唯一标识
        .expect("注册条目应带稳定标识");

    执行(
        &mut 应用,
        UiAction::打开材质资源选择器 {
            材质路径: 材质路径文本.to_string(),
            属性: "法线贴图".to_string(),
            类别: 资源类别::图片,
        },
    );
    let 选择器 = 界面(&应用).资源选择器.clone().expect("选择器应当打开");
    assert_eq!(
        选择器.落点,
        资源落点::材质字段 {
            路径: 材质路径文本.to_string(),
        },
        "选择器要知道结果写回哪份草稿，而不是回落到某个实体字段"
    );
    assert_eq!(选择器.属性, "法线贴图");
    assert_eq!(选择器.类别, 资源类别::图片);

    执行(
        &mut 应用,
        UiAction::设置材质资源引用 {
            材质路径: 材质路径文本.to_string(),
            属性: "法线贴图".to_string(),
            资源标识: Some(标识),
        },
    );

    let 会话 = 界面(&应用).材质编辑.clone().expect("草稿应当还在");
    assert!(会话.脏, "选了贴图就是有未保存改动");
    assert_eq!(
        会话.取材质().法线贴图.map(|引用| 引用.路径),
        Some(贴图路径),
        "草稿里的引用要带路径与标识，保存时才能原样写回文件"
    );
    assert!(界面(&应用).资源选择器.is_none(), "选完要关掉浮层");
    assert!(
        读磁盘材质(&应用).法线贴图.is_none(),
        "选择本身不写盘：写盘只归保存那一步，否则窗口上的「未保存」角标就是谎话"
    );

    let 动作 = 排保存(&应用);
    执行(&mut 应用, 动作);
    assert!(
        读磁盘材质(&应用).法线贴图.is_some(),
        "保存后新贴图槽才写进 `.material.ron`"
    );
}

#[test]
fn 草稿已关闭时选择结果被拒绝并留警告() {
    let (mut 应用, _目录, _实体) = 建应用("选择落空");

    执行(
        &mut 应用,
        UiAction::设置材质资源引用 {
            材质路径: 材质路径文本.to_string(),
            属性: "法线贴图".to_string(),
            资源标识: None,
        },
    );

    assert!(
        有结构化日志(&应用, 日志级别::警告, "log_material_editor_no_draft"),
        "没有草稿时要说明结果没地方写，而不是静默丢掉一次选择"
    );
}

#[test]
fn 没有项目时打开与保存都只报警并保留草稿() {
    let (mut 应用, _目录, _实体) = 建应用("无项目");
    应用
        .world_mut()
        .resource_mut::<EditorStateHolder>()
        .状态
        .项目 = None;

    执行(
        &mut 应用,
        UiAction::打开材质编辑器(材质路径文本.to_string()),
    );
    assert!(界面(&应用).材质编辑.is_none());
    assert!(
        有结构化日志(&应用, 日志级别::警告, "log_material_editor_no_project"),
        "「没有打开项目」要说出来，而不是点了没反应"
    );

    应用
        .world_mut()
        .resource_mut::<UiStateHolder>()
        .状态
        .材质编辑 = Some(材质编辑会话::从材质(
        材质路径文本.to_string(),
        &磁盘材质(),
    ));
    改草稿(&mut 应用, 粗糙度字段, 属性值::浮点(0.9));
    let 动作 = 排保存(&应用);
    执行(&mut 应用, 动作);

    let 会话 = 界面(&应用).材质编辑.as_ref().expect("保存失败要保留草稿");
    assert!(会话.脏, "写盘失败时未保存标记必须留着");
    assert_eq!(会话.取材质().粗糙度, 0.9, "草稿内容不能被失败的动作吃掉");
}
