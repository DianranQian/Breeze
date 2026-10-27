//! 中文名：材质投影
//! `.material.ron` → `StandardMaterial` 的投影、代次缓存与降级。
//!
//! # 为什么这组测试存在
//!
//! 材质是**项目资源**，内容不在场景文档里。渲染层要的是"读文件 → 转成
//! `StandardMaterial` → 挂到网格上"这一整条链，而这条链上有三处各自会坏：
//!
//! 1. **字段映射**：粗糙度落到哪个 Bevy 字段、`unlit` 归谁管、自发光全黑要不要发光；
//! 2. **缓存**：代次没变就复用（否则每次重建都要重读几百次盘、塞进几百份同样的材质），
//!    代次一变就要失效（否则材质改了视口不动）；
//! 3. **降级**：材质文件缺失或写坏了，实体必须照样渲染，而不是变黑或消失。
//!
//! 前两条能靠纯函数断言，第三条必须真的跑一遍同步系统才知道"渲染层拿到的那份材质
//! 是不是回退后的那份"，因此这里既跑纯函数，也跑带 `Assets` 的最小 `App`。
//!
//! # 为什么断言到材质内容为止，而不是像素
//!
//! 起整套渲染管线（窗口、GPU、渲染图）来做单元测试不现实，也不稳定。
//! 这里验证的是"渲染层拿到的输入是对的"这一步；最终画面由 GUI 验收负责。

use bevy::asset::AssetPlugin;
use bevy::math::Affine2;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use editor_app::plugin::EditorStateHolder;
use editor_app::preview::{
    三维预览资源, 同步预览实体, 投影几何项, 投影预览项, 材质缓存, 材质转标准材质, 自发光亮度系数,
    贴图缓存, 预览同步状态, 预览来源,
};
use editor_app::tile_terrain::地形缓存;
use editor_core::material::材质资源;
use editor_core::project::项目描述;
use scene_doc::document::{变换数据, 场景文档, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 资源引用值, 颜色值};
use std::collections::BTreeMap;

const 材质路径文本: &str = "assets/materials/石板.material.ron";
const 贴图路径文本: &str = "assets/贴图/石板.png";

/// 一份"每个字段都不同于默认值"的材质。
///
/// 默认值与期望值重合时，"这个字段根本没接线"也能通过断言；
/// 全部取非默认值才能证明每一格都真的落到了 `StandardMaterial` 上。
fn 材质样本() -> 材质资源 {
    材质资源 {
        格式版本: 材质资源::新建("石板路").格式版本,
        名称: "石板路".to_string(),
        着色颜色: 颜色值::srgb(0.2, 0.4, 0.6, 1.0),
        贴图: None,
        粗糙度: 0.25,
        金属度: 0.8,
        自发光: 颜色值::srgb(0.5, 0.0, 0.0, 1.0),
        双面: false,
    }
}

/// 单位变换：投影规则要求实体必须有变换，否则不进预览。
fn 单位变换() -> 变换数据 {
    变换数据 {
        平移: (0.0, 0.0, 0.0),
        旋转: (0.0, 0.0, 0.0, 1.0),
        缩放: (1.0, 1.0, 1.0),
    }
}

/// 一个资源引用属性值（与 Inspector 写进文档的形态一致）。
fn 引用(路径: &str) -> 属性值 {
    属性值::资源(资源引用值::按路径(
        项目相对路径::解析(路径).expect("测试里的路径必须合法"),
    ))
}

/// 造一个带变换与单个组件的实体。
fn 造实体(
    文档: &mut 场景文档,
    名: &str,
    组件名: &str,
    属性: BTreeMap<String, 属性值>,
) -> 实体标识 {
    let 标识 = 文档.生成实体(名, None);
    let 实体 = 文档.取可变(标识).expect("刚生成的实体一定在");
    实体.变换 = Some(单位变换());
    实体.加组件(组件数据::新建(组件名, 属性));
    标识
}

/// 一份 `Primitive3d` 属性：受光立方体，UV 重复 2×2，可选地引用材质。
fn 几何体属性(引用材质: bool) -> BTreeMap<String, 属性值> {
    let mut 属性 = BTreeMap::new();
    属性.insert(
        "形状".to_string(),
        属性值::枚举(scene_doc::property::枚举值::新建("Cuboid", 0)),
    );
    属性.insert("尺寸".to_string(), 属性值::三维(4.0, 1.0, 4.0));
    属性.insert("颜色".to_string(), 属性值::颜色(颜色值::白()));
    属性.insert("贴图重复".to_string(), 属性值::二维(2.0, 2.0));
    属性.insert("受光".to_string(), 属性值::布尔(true));
    属性.insert("双面".to_string(), 属性值::布尔(false));
    if 引用材质 {
        属性.insert("材质".to_string(), 引用(材质路径文本));
    }
    属性
}

/// 把材质写进项目并返回该项目（材质是项目资源，必须先落盘）。
fn 带材质的项目(根: &std::path::Path, 名: &str, 材质: &材质资源) -> 项目描述 {
    let 项目 = 项目描述::创建(根.join(名), 名).expect("创建项目");
    项目
        .保存材质(&项目相对路径::解析(材质路径文本).unwrap(), 材质)
        .expect("保存材质");
    项目
}

/// 在项目里写一张 `宽×高` 的纯色 PNG，路径为项目相对的 `相对路径`。
///
/// 贴图槽的归属只有在**真能解码出图**时才观察得到：文件不存在时
/// `解析贴图句柄` 返回 1×1 的无效占位句柄，材质上根本挂不出贴图，
/// 断言就退化成"有没有贴图"而看不出"用的是哪一张"。
/// 因此两张图取不同尺寸，读回来的 `Image::size()` 直接指明是哪一张。
fn 写入贴图(项目: &项目描述, 相对路径: &str, 宽: u32, 高: u32) {
    let 绝对 = 项目
        .根目录()
        .join(相对路径.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(父目录) = 绝对.parent() {
        std::fs::create_dir_all(父目录).expect("建贴图目录");
    }
    image::RgbaImage::from_pixel(宽, 高, image::Rgba([9, 200, 9, 255]))
        .save(&绝对)
        .expect("写 PNG");
}

/// 材质资源，默认不引用项目（无项目时也要能构造）。
fn 材质资源带贴图(贴图: &str) -> 材质资源 {
    材质资源 {
        贴图: Some(资源引用值::按路径(
            项目相对路径::解析(贴图).expect("测试里的贴图路径必须合法"),
        )),
        ..材质样本()
    }
}

// ---------------------------------------------------------------------
// 字段映射（纯函数）
// ---------------------------------------------------------------------

/// 材质的表面参数逐项落到 `StandardMaterial` 上，渲染模式仍归实体。
#[test]
fn 材质字段逐项投影到标准材质() {
    let 结果 = 材质转标准材质(
        &材质样本(),
        Handle::default(),
        true,
        false,
        false,
        Some((2.0, 4.0)),
    );

    assert_eq!(结果.base_color, Color::srgba(0.2, 0.4, 0.6, 1.0));
    assert!(
        (结果.metallic - 0.8).abs() < 1e-6,
        "金属度要接到 metallic，实际 {}",
        结果.metallic
    );
    assert!(
        (结果.perceptual_roughness - 0.25).abs() < 1e-6,
        "粗糙度要接到 perceptual_roughness，实际 {}",
        结果.perceptual_roughness
    );
    assert!(
        结果.base_color_texture.is_none(),
        "材质与实体都没有贴图时不挂 1×1 占位图：纯色方块才是可辨认的降级"
    );
    assert!(!结果.unlit, "受光的实体挂上材质后仍然受光");
    assert_eq!(
        结果.alpha_mode,
        AlphaMode::Opaque,
        "不透明几何体不该被材质改成 Blend"
    );
    assert_eq!(
        结果.cull_mode,
        Some(Face::Back),
        "实体与材质都要求单面时保留背面剔除"
    );
    assert!(
        结果.emissive.red > 0.0 && 结果.emissive.green == 0.0,
        "自发光要落进 emissive 的对应通道：{:?}",
        结果.emissive
    );
    assert_eq!(
        结果.uv_transform,
        Affine2::from_scale(Vec2::new(2.0, 4.0)),
        "UV 重复是实体的输入，投影要原样带上"
    );
}

/// 全黑自发光必须真的不发光：材质文档的默认值就是全黑。
///
/// 同一条里带一个反向对照：非零自发光在同一通道上确实亮起来了。
/// 只有全黑那一半，断言可能只是"把整个通道写死成 0"蒙过去的。
#[test]
fn 全黑自发光不产生任何亮度() {
    let 不发光 = 材质资源 {
        自发光: 颜色值::srgb(0.0, 0.0, 0.0, 1.0),
        ..材质样本()
    };
    let 结果 = 材质转标准材质(&不发光, Handle::default(), true, false, false, None);
    assert_eq!(
        (结果.emissive.red, 结果.emissive.green, 结果.emissive.blue),
        (0.0, 0.0, 0.0),
        "自发光是 0 nit 才对，否则每个默认材质都会在黑暗里发光"
    );

    let 发光 = 材质转标准材质(&材质样本(), Handle::default(), true, false, false, None);
    assert!(
        发光.emissive.red > 自发光亮度系数 * 0.1,
        "自发光颜色要乘亮度系数后才进 HDR 域，实际 {}",
        发光.emissive.red
    );
}

/// 精灵四边形保持 `unlit + Blend + 不剔除`：材质改不动它的渲染模式。
///
/// 这是既有的"三维对应物"约定 —— 3D 视图里的精灵必须与 2D 视图长得一样。
/// 哪天材质把精灵改成受光，两个视图就会不一致，那是最难查的一类 bug。
#[test]
fn 精灵四边形上材质只改表面不改渲染模式() {
    let 结果 = 材质转标准材质(&材质样本(), Handle::default(), false, true, true, None);
    assert!(结果.unlit, "精灵四边形恒为 unlit");
    assert_eq!(结果.alpha_mode, AlphaMode::Blend);
    assert_eq!(
        结果.cull_mode, None,
        "编辑器经常从背面看精灵，材质不能把它剔掉"
    );
    assert!(
        (结果.metallic - 0.8).abs() < 1e-6 && (结果.perceptual_roughness - 0.25).abs() < 1e-6,
        "表面参数仍然写进去了：这条线将来改成受光时不需要再补投影"
    );
}

/// 材质与实体任一侧要求双面，就不做背面剔除。
#[test]
fn 任一侧要求双面就不剔除背面() {
    let 材质要求双面 = 材质资源 {
        双面: true,
        ..材质样本()
    };
    assert_eq!(
        材质转标准材质(&材质要求双面, Handle::default(), true, false, false, None).cull_mode,
        None,
        "材质说双面就得不剔除"
    );
    assert_eq!(
        材质转标准材质(&材质样本(), Handle::default(), true, true, false, None).cull_mode,
        None,
        "实体说双面同样不剔除：材质不该把实体故意打开的可见性关掉"
    );
}

// ---------------------------------------------------------------------
// 投影：文档字段 → 渲染项
// ---------------------------------------------------------------------

/// `Primitive3d` 与 `Sprite` 的 `材质` 字段都要被投影带出来。
#[test]
fn 投影把材质引用带进渲染项() {
    let mut 文档 = 场景文档::新建("材质引用");
    let 三维 = 造实体(&mut 文档, "地面", "Primitive3d", 几何体属性(true));
    let 二维 = {
        let mut 属性 = BTreeMap::new();
        属性.insert("贴图".to_string(), 引用(贴图路径文本));
        属性.insert("材质".to_string(), 引用(材质路径文本));
        造实体(&mut 文档, "屋顶", "Sprite", 属性)
    };

    let 几何项 = 投影几何项(&文档);
    assert_eq!(几何项.len(), 1, "只有一条 Primitive3d");
    assert_eq!(几何项[0].来源, 三维, "进几何预览的应该是那个立方体");
    assert_eq!(
        几何项[0].材质路径.as_deref(),
        Some(材质路径文本),
        "三维几何体的材质引用必须进渲染项，否则同步阶段无从读起"
    );

    let 预览项 = 投影预览项(&文档);
    assert_eq!(预览项.len(), 1, "没有 Sprite 组件的几何体不进精灵预览");
    assert_eq!(预览项[0].来源, 二维, "进精灵预览的应该是那个 Sprite 实体");
    assert_eq!(
        预览项[0].视觉.材质路径.as_deref(),
        Some(材质路径文本),
        "二维精灵也要带出材质引用：它的三维对应物要靠这一格换材质"
    );
}

// ---------------------------------------------------------------------
// 缓存键与回收
// ---------------------------------------------------------------------

/// 键必须把每一个参与转换的输入都登记上。
///
/// 少登记一项 = 同一份材质在两种实体上共用同一个 `StandardMaterial`，
/// 表现为"改了一个实体的 UV 重复，另一个也跟着变"这种跨实体的幽灵耦合。
#[test]
fn 缓存键区分代次与每一项实体输入() {
    let 基准 = 材质缓存::键(
        材质路径文本,
        1,
        true,
        false,
        false,
        "无贴图",
        Some((2.0, 2.0)),
    );
    let 变化们 = [
        材质缓存::键(
            材质路径文本,
            2,
            true,
            false,
            false,
            "无贴图",
            Some((2.0, 2.0)),
        ),
        材质缓存::键(
            "assets/materials/其他.material.ron",
            1,
            true,
            false,
            false,
            "无贴图",
            Some((2.0, 2.0)),
        ),
        材质缓存::键(
            材质路径文本,
            1,
            false,
            false,
            false,
            "无贴图",
            Some((2.0, 2.0)),
        ),
        材质缓存::键(
            材质路径文本,
            1,
            true,
            true,
            false,
            "无贴图",
            Some((2.0, 2.0)),
        ),
        材质缓存::键(
            材质路径文本,
            1,
            true,
            false,
            true,
            "无贴图",
            Some((2.0, 2.0)),
        ),
        材质缓存::键(
            材质路径文本,
            1,
            true,
            false,
            false,
            贴图路径文本,
            Some((2.0, 2.0)),
        ),
        材质缓存::键(
            材质路径文本,
            1,
            true,
            false,
            false,
            "无贴图",
            Some((2.0, 3.0)),
        ),
        材质缓存::键(材质路径文本, 1, true, false, false, "无贴图", None),
    ];
    for 变化 in 变化们 {
        assert_ne!(基准, 变化, "每一项输入都要反映在键里：{变化}");
    }
    assert_eq!(
        基准,
        材质缓存::键(
            材质路径文本,
            1,
            true,
            false,
            false,
            "无贴图",
            Some((2.0, 2.0))
        ),
        "同样的输入必须得到同样的键，否则缓存永远不命中"
    );
}

/// 缓存条目只保留本轮被引用的键：删实体、换路径、代次推进都不留强句柄。
///
/// 走两轮：第一轮转换并收尾，条目要留下；第二轮只有新代次的键被引用，
/// 旧键必须被丢掉。只跑一轮的话，"记录即登记"与"收尾即回收"分不开，
/// 看不出回收到底有没有生效。
#[test]
fn 清理本轮丢弃未被引用的条目() {
    let mut 缓存 = 材质缓存::default();
    let 旧键 = 材质缓存::键(材质路径文本, 1, true, false, false, "无贴图", None);
    let 新键 = 材质缓存::键(材质路径文本, 2, true, false, false, "无贴图", None);
    let 句柄 = Handle::<StandardMaterial>::default();

    缓存.记录(旧键.clone(), Some(句柄.clone()));
    缓存.清理本轮();
    assert_eq!(缓存.数量(), 1, "本轮刚转换出来的条目必须留下");

    缓存.记录(新键.clone(), Some(句柄));
    assert!(缓存.取(&新键).is_some(), "取到即登记为本轮在用");
    缓存.清理本轮();

    assert_eq!(缓存.数量(), 1, "本轮用过的键必须留下");
    assert!(
        缓存.取(&旧键).is_none(),
        "旧代次的键要被丢掉：强句柄不放掉，材质资产就永远留在资产表里"
    );
}

/// 试过但失败的材质要被记住，而不是每次重建都重新撞一次磁盘。
#[test]
fn 失败结果也进缓存但不再登记为可用() {
    let mut 缓存 = 材质缓存::default();
    let 键 = 材质缓存::键(材质路径文本, 1, true, false, false, "无贴图", None);
    assert!(缓存.取(&键).is_none(), "没试过的时候必须让调用方去读盘");

    缓存.记录(键.clone(), None);
    assert_eq!(缓存.数量(), 1);
    assert_eq!(
        缓存.取(&键),
        Some(None),
        "外层 Some = 试过，内层 None = 失败；两者合起来才能既降级又不重复读盘"
    );
}

// ---------------------------------------------------------------------
// 跑真实同步系统：渲染层拿到的那份材质
// ---------------------------------------------------------------------

/// 只装同步所需资源的最小应用。
///
/// 不挂渲染插件：这里要断言的是"渲染层拿到的输入对不对"，不是最终像素。
///
/// 构造完就刷新一次资源注册表：材质缓存的键里那格代次来自注册表，
/// "打开项目"这一步在真实编辑器里同样是靠它推进代次的。
fn 预览应用(项目: Option<项目描述>, 文档: 场景文档) -> App {
    let mut 应用 = App::new();
    应用.add_plugins((MinimalPlugins, AssetPlugin::default()));
    应用.init_resource::<Assets<StandardMaterial>>();
    应用.init_resource::<Assets<Image>>();
    应用.init_resource::<Assets<Mesh>>();

    let mut 持有 = EditorStateHolder::default();
    if let Some(项目) = 项目 {
        持有.状态.项目 = Some(项目);
    }
    持有.状态.会话.文档 = 文档;
    持有.状态.刷新资源注册表();
    应用.insert_resource(持有);
    应用.insert_resource(贴图缓存::default());
    应用.insert_resource(地形缓存::default());
    应用.insert_resource(材质缓存::default());
    应用.insert_resource(预览同步状态::default());
    应用.insert_resource(三维预览资源 {
        四边形: Handle::default(),
    });
    应用.add_systems(Update, 同步预览实体);
    应用
}

/// 某个文档实体在三维渲染层上拿到的材质句柄。
///
/// 只查 `MeshMaterial3d`：一个实体在三维层恰好一份材质
/// （精灵四边形或几何体网格），二维精灵不挂材质。
/// 用 `iter_entities` 而不是 `QueryState`：后者要 `&mut World`，
/// 而测试里只想读，不想为了读一次材质就把整个世界搭出去。
fn 取材质句柄(世界: &World, 来源: 实体标识) -> Handle<StandardMaterial> {
    世界
        .iter_entities()
        .filter_map(|实体| {
            let 预览 = 实体.get::<预览来源>()?;
            if 预览.0 != 来源 {
                return None;
            }
            Some(实体.get::<MeshMaterial3d<StandardMaterial>>()?.0.clone())
        })
        .next()
        .unwrap_or_else(|| panic!("实体 {来源:?} 没有拿到三维材质"))
}

/// 同上，直接读到材质内容。
fn 取材质(应用: &App, 来源: 实体标识) -> StandardMaterial {
    let 世界 = 应用.world();
    let 句柄 = 取材质句柄(世界, 来源);
    世界
        .resource::<Assets<StandardMaterial>>()
        .get(&句柄)
        .cloned()
        .expect("句柄必须指向一份真实存在的材质")
}

/// 把同步状态视为已过期，强制下一帧重建。
fn 强制同步(应用: &mut App) {
    应用.world_mut().resource_mut::<预览同步状态>().已同步修订 = None;
    应用.update();
}

/// 带材质引用的实体渲染出材质属性。
///
/// 这份材质引用了一张贴图，因此磁盘上必须真有那张 PNG —— 文件不存在时
/// `解析贴图句柄` 给的是无效的占位句柄，材质上什么都挂不上。
#[test]
fn 带材质引用的实体渲染出材质属性() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 项目 = 带材质的项目(目录.path(), "材质投影", &材质资源带贴图(贴图路径文本));
    写入贴图(&项目, 贴图路径文本, 2, 2);

    let mut 文档 = 场景文档::新建("材质投影");
    let 地面 = 造实体(&mut 文档, "地面", "Primitive3d", 几何体属性(true));
    let mut 应用 = 预览应用(Some(项目), 文档);
    应用.update();

    let 实际 = 取材质(&应用, 地面);
    assert!(
        (实际.metallic - 0.8).abs() < 1e-6 && (实际.perceptual_roughness - 0.25).abs() < 1e-6,
        "同步出来的网格必须带上材质的 PBR 参数，实际 metallic={} roughness={}",
        实际.metallic,
        实际.perceptual_roughness
    );
    assert_eq!(实际.base_color, Color::srgba(0.2, 0.4, 0.6, 1.0));
    assert_eq!(
        实际.cull_mode,
        Some(Face::Back),
        "实体单面、材质也单面时保留背面剔除"
    );
    assert_eq!(
        实际.uv_transform,
        Affine2::from_scale(Vec2::new(2.0, 2.0)),
        "UV 重复归实体，材质不得覆盖"
    );
    assert!(
        实际.base_color_texture.is_some(),
        "材质自带贴图时要用材质的贴图，不能因为实体没填贴图就没有贴图"
    );
}

/// 材质自带贴图时盖掉实体的贴图，材质没带贴图时沿用实体的。
///
/// 这条归属规则只住在 `preview::解析材质句柄` 里 —— 它要先读完成材质
/// 才知道材质带不带贴图，纯函数断言够不着。所以这里真的落两张**不同尺寸**的
/// PNG，再读回材质看挂上的是哪一张：尺寸是唯一不会被色彩空间转换改掉的证据。
#[test]
fn 贴图槽按材质优先实体兜底() {
    const 材质贴图: &str = "assets/贴图/材质自带.png";
    const 实体贴图: &str = "assets/贴图/实体自带.png";

    let 目录 = tempfile::tempdir().expect("临时目录");
    let 项目 = 项目描述::创建(目录.path().join("贴图归属"), "贴图归属").expect("创建项目");
    写入贴图(&项目, 材质贴图, 2, 2);
    写入贴图(&项目, 实体贴图, 8, 3);
    let 材质文档路径 = 项目相对路径::解析(材质路径文本).unwrap();
    项目
        .保存材质(&材质文档路径, &材质资源带贴图(材质贴图))
        .expect("保存带贴图的材质");

    let mut 属性 = 几何体属性(true);
    属性.insert("贴图".to_string(), 引用(实体贴图));
    let mut 文档 = 场景文档::新建("贴图归属");
    let 地面 = 造实体(&mut 文档, "地面", "Primitive3d", 属性);
    let mut 应用 = 预览应用(Some(项目), 文档);
    应用.update();

    assert_eq!(
        已挂贴图尺寸(&应用, 地面),
        UVec2::new(2, 2),
        "材质自带贴图时盖掉实体那一格：否则一张共享贴图会被后来者改掉"
    );

    // 材质改成不带贴图 → 代次推进 → 回到实体自己填的贴图。
    应用
        .world()
        .resource::<EditorStateHolder>()
        .状态
        .项目
        .as_ref()
        .expect("已打开项目")
        .保存材质(
            &材质文档路径,
            &材质资源 {
                贴图: None,
                ..材质样本()
            },
        )
        .expect("保存无贴图材质");
    应用
        .world_mut()
        .resource_mut::<EditorStateHolder>()
        .状态
        .刷新资源注册表();
    强制同步(&mut 应用);

    assert_eq!(
        已挂贴图尺寸(&应用, 地面),
        UVec2::new(8, 3),
        "材质没带贴图时沿用实体的贴图槽：实体自己填的那格不该被清空"
    );
}

/// 该实体最终挂上的那张贴图的尺寸；没挂上任何图就失败。
fn 已挂贴图尺寸(应用: &App, 来源: 实体标识) -> UVec2 {
    let 世界 = 应用.world();
    let 材质句柄 = 取材质句柄(世界, 来源);
    let 图片句柄 = 世界
        .resource::<Assets<StandardMaterial>>()
        .get(&材质句柄)
        .and_then(|材质| 材质.base_color_texture.clone())
        .expect("材质必须挂上一张真实解码出来的贴图");
    世界
        .resource::<Assets<Image>>()
        .get(&图片句柄)
        .expect("贴图句柄要指向一份真实存在的图片")
        .size()
}

/// 同一份材质挂在两个实体上只做一次转换；代次推进后重新读盘并回收旧条目。
#[test]
fn 同一代次复用转换结果且代次推进后重取() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 项目 = 带材质的项目(目录.path(), "材质缓存", &材质样本());
    let 材质文档路径 = 项目相对路径::解析(材质路径文本).unwrap();

    let mut 文档 = 场景文档::新建("材质缓存");
    let 甲 = 造实体(&mut 文档, "甲", "Primitive3d", 几何体属性(true));
    let 乙 = 造实体(&mut 文档, "乙", "Primitive3d", 几何体属性(true));
    let mut 应用 = 预览应用(Some(项目), 文档);
    应用.update();

    assert_eq!(
        应用.world().resource::<材质缓存>().数量(),
        1,
        "两个实体共用同一次转换"
    );
    assert_eq!(
        应用.world().resource::<Assets<StandardMaterial>>().len(),
        1,
        "材质资产表里只该有一份石板材质"
    );
    assert_eq!(
        取材质句柄(应用.world(), 甲),
        取材质句柄(应用.world(), 乙),
        "同键必须复用同一个句柄"
    );
    assert!(
        取材质句柄(应用.world(), 甲).is_strong(),
        "缓存要持强句柄，否则材质资产会在这一帧之后被回收"
    );

    // 外部改过材质文件 → 注册表刷新推进代次 → 必须重新读盘。
    应用
        .world()
        .resource::<EditorStateHolder>()
        .状态
        .项目
        .as_ref()
        .expect("已打开项目")
        .保存材质(
            &材质文档路径,
            &材质资源 {
                金属度: 0.3,
                ..材质样本()
            },
        )
        .expect("改材质");
    应用
        .world_mut()
        .resource_mut::<EditorStateHolder>()
        .状态
        .刷新资源注册表();
    强制同步(&mut 应用);

    let 第二次 = 取材质(&应用, 甲);
    assert!(
        (第二次.metallic - 0.3).abs() < 1e-6,
        "代次推进后必须重新读材质文件，实际 metallic={}",
        第二次.metallic
    );
    assert_eq!(
        应用.world().resource::<材质缓存>().数量(),
        1,
        "旧代次的条目要被回收"
    );
}

/// 材质文件不存在：实体照样渲染，用的是没有材质时的那份基线。
#[test]
fn 材质读不到时安全降级为无材质渲染() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    // 项目建好了但**不写**这个材质文件：文档引用了一个不存在的材质。
    let 项目 = 项目描述::创建(目录.path().join("材质降级"), "材质降级").expect("创建项目");

    let mut 属性 = 几何体属性(true);
    属性.insert(
        "材质".to_string(),
        引用("assets/materials/不存在.material.ron"),
    );
    let mut 文档 = 场景文档::新建("材质降级");
    let 地面 = 造实体(&mut 文档, "地面", "Primitive3d", 属性);
    let mut 应用 = 预览应用(Some(项目), 文档);
    应用.update();

    let 实际 = 取材质(&应用, 地面);
    assert_eq!(
        实际.base_color,
        Color::srgba(1.0, 1.0, 1.0, 1.0),
        "降级就是回到实体自己的颜色"
    );
    assert!(
        (实际.perceptual_roughness - 0.9).abs() < 1e-6,
        "降级要回到引入材质前的基线粗糙度，实际 {}",
        实际.perceptual_roughness
    );
    assert!(!实际.unlit, "实体受光 → 降级后仍受光");
    assert_eq!(实际.cull_mode, Some(Face::Back));
    assert_eq!(实际.metallic, 0.0, "坏材质不能凭空带来金属感");

    // 失败也被记住：挂同一个坏路径的实体、每次重建都不该再撞一次磁盘。
    强制同步(&mut 应用);
    assert_eq!(应用.world().resource::<材质缓存>().数量(), 1);
}
