//! 预览视口：把场景文档渲染成纹理，供界面显示。
//!
//! # 为什么预览实体住在**主世界**里
//!
//! 有两种可能的方案：
//!
//! | | 独立预览 `World` | 主世界 + 预览标记（选用） |
//! | --- | --- | --- |
//! | 渲染 | 需要自己搭第二套 extract/render 管线 | 直接用 Bevy 现成管线 |
//! | 复杂度 | 高（要复制 AssetServer、渲染图、相机栈） | 低 |
//! | 隔离风险 | 天然隔离 | 需要靠标记与"文档才是真源"保证 |
//!
//! 选了后者，因为**隔离性已经由现有架构保证了**：
//!
//! * 保存场景时 `项目描述::保存场景` 只序列化 `编辑器状态.会话.文档`，
//!   它**从不读取 `World`**；
//! * 因此世界里有再多预览实体，也不可能进到场景文件里。
//!
//! 这不是"相信我们记得清理"，而是**保存路径根本不接触世界**。
//! 测试 `预览实体不会进入场景保存结果` 把这个性质固定下来。
//!
//! 而且预览实体本身也不是垃圾：它们是"文档在渲染侧的投影"，
//! 该存在，不是需要清理的临时物。停止运行时被丢弃的是
//! [`crate::scene_bridge::运行时宿主`] 的那个 `World`，与预览无关。
//!
//! # 与运行时世界的区别
//!
//! | | 预览世界（本模块） | 运行时世界（`scene_bridge`） |
//! | --- | --- | --- |
//! | 目的 | 让编辑器看到场景长什么样 | 让游戏跑起来 |
//! | 位置 | 主世界，带 [`预览实体`] 标记 | 独立 `World` |
//! | 生命周期 | 与文档同步，一直存在 | 运行/停止 |
//! | 是否可写 | 只读投影：文档 → 预览，**单向** | 游戏逻辑可任意改 |
//!
//! 两者都是"文档的消费者"，互不干扰。

use bevy::camera::{RenderTarget, ScalingMode, visibility::RenderLayers};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::AlphaMode;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use scene_doc::document::{变换数据, 场景文档, 实体标识};
use scene_doc::property::{属性值, 颜色值};

use crate::plugin::EditorStateHolder;

/// 预览实体标记。
///
/// 所有由场景文档投影出来的渲染实体都带这个组件。
/// 它的作用不是"方便清理"，而是让"哪些实体来自文档"这件事可查询、
/// 可测试 —— 例如断言"预览实体数量 == 文档里有变换的实体数量"。
#[derive(Component, Debug, Clone, Copy)]
pub struct 预览实体;

/// 预览实体与文档标识的对应关系，挂在预览实体自己身上。
///
/// 用组件而不是旁挂一张表：Bevy 的实体本身就会被销毁，
/// 组件随实体一起消失，不可能出现"表里还留着已消失实体的条目"这种
/// 两份数据不同步的问题。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct 预览来源(pub 实体标识);

/// 预览相机的标记。
#[derive(Component, Debug, Clone, Copy)]
pub struct 预览相机;

/// 三维预览层：与 2D 预览层分开。
///
/// 2D 相机只画精灵层、3D 相机只画网格层。否则 2D 的正交相机也会把
/// 三维网格当成"扁平的方块"画出来，看起来像多了一堆莫名其妙的矩形。
pub const 三维渲染层: usize = 预览渲染层 + 1;

/// 三维预览用的共享网格（1×1 四边形，按实体尺寸缩放）。
#[derive(Resource)]
pub struct 三维预览资源 {
    /// 四边形网格句柄。
    pub 四边形: Handle<Mesh>,
}

/// 三维预览的当前状态。
#[derive(Resource, Default)]
pub struct 三维预览状态 {
    /// 预览相机当前是否是三维相机。
    pub 已三维: bool,
}

/// 编辑器界面相机标记。
///
/// 单独一台相机专门用于承载 egui 的 primary context。
/// 必须存在且必须与预览相机分开，原因见
/// [`初始化预览资源`] 的说明。
#[derive(Component, Debug, Clone, Copy)]
pub struct 编辑器界面相机;

/// 预览使用的渲染层。
///
/// 预览实体与预览相机都在这一层，主相机（默认第 0 层）看不到它们。
/// 这样预览内容不会意外出现在其他地方，也不会被别的相机重复渲染。
pub const 预览渲染层: usize = 1;

/// 预览渲染目标的大小上限与下限。
///
/// 上限是必要的：视口面板被拉到 4K 宽时，渲染目标会吃掉大量显存，
/// 而编辑器视口并不需要那么高的分辨率。超过上限就按上限渲染，
/// egui 会把它拉伸显示 —— 略微变糊，但不会把显存吃光。
pub const 预览尺寸下限: u32 = 64;
/// 渲染目标边长上限（像素）。
pub const 预览尺寸上限: u32 = 2048;

/// 把期望的视口像素尺寸收敛到允许的渲染目标尺寸。
///
/// 抽成纯函数是为了能直接测试边界，而不必真的去创建纹理。
pub fn 收敛渲染尺寸(宽: f32, 高: f32) -> (u32, u32) {
    let 取整 = |值: f32| -> u32 {
        if !值.is_finite() || 值 <= 0.0 {
            return 预览尺寸下限;
        }
        (值.round() as i64).clamp(预览尺寸下限 as i64, 预览尺寸上限 as i64) as u32
    };
    (取整(宽), 取整(高))
}

/// 预览渲染目标。
#[derive(Resource, Debug, Clone)]
pub struct 预览渲染目标 {
    /// 作为相机渲染目标、同时交给 egui 显示的图片。
    pub 图片: Handle<Image>,
    /// 当前纹理的像素尺寸。
    pub 尺寸: (u32, u32),
    /// egui 纹理句柄。注册成功后才有值。
    pub 纹理: Option<egui::TextureId>,
}

impl 预览渲染目标 {
    /// 交给界面显示的渲染结果。未注册纹理时返回 `None`。
    pub fn 渲染结果(&self) -> Option<editor_ui::视口渲染结果> {
        Some(editor_ui::视口渲染结果 {
            纹理: self.纹理?,
            像素尺寸: self.尺寸,
        })
    }
}

/// 预览相机状态：由界面的视口相机驱动。
#[derive(Resource, Debug, Clone, Copy)]
pub struct 预览相机状态 {
    /// 视口中心对应的世界坐标。
    pub 中心: (f32, f32),
    /// 每个世界单位对应多少像素。
    pub 缩放: f32,
    /// 视口背景色（用于把渲染目标清成该颜色或透明）。
    pub 背景: 颜色值,
    /// 相机实体。初始化后填上。
    pub 相机实体: Option<Entity>,
}

impl Default for 预览相机状态 {
    fn default() -> Self {
        Self {
            中心: (0.0, 0.0),
            缩放: 48.0,
            背景: 颜色值::srgb(0.09, 0.09, 0.10, 1.0),
            相机实体: None,
        }
    }
}

/// 文档投影状态：记录上次同步到哪个修订号。
#[derive(Resource, Debug, Default)]
pub struct 预览同步状态 {
    /// 上次同步时的文档修订号。
    pub 已同步修订: Option<u64>,
    /// 上次同步建立的预览实体数（用于日志与诊断）。
    pub 预览实体数: usize,
}

/// 纹理缓存：资源路径 → 已加载的图片句柄。
///
/// 只属于渲染，不进场景文档（句柄与帧绑定，存进文件没有意义）。
#[derive(Resource, Default)]
pub struct 贴图缓存 {
    /// 已请求过的资源。值 `None` 表示请求过但当时未打开项目 / 加载失败。
    条目: std::collections::HashMap<String, Option<Handle<Image>>>,
}

impl 贴图缓存 {
    /// 查询是否已请求过某个路径。
    pub fn 已请求(&self, 路径: &str) -> bool {
        self.条目.contains_key(路径)
    }

    /// 取已缓存的句柄。
    pub fn 取(&self, 路径: &str) -> Option<&Handle<Image>> {
        self.条目.get(路径).and_then(|项| 项.as_ref())
    }

    /// 记录一次请求结果。
    pub fn 记录(&mut self, 路径: impl Into<String>, 句柄: Option<Handle<Image>>) {
        self.条目.insert(路径.into(), 句柄);
    }

    /// 已缓存的条目数。
    pub fn 数量(&self) -> usize {
        self.条目.len()
    }
}

/// 从场景文档的变换算出预览实体的 `Transform`。
///
/// 与 [`crate::scene_bridge::变换数据转_bevy`] 复用同一段逻辑：
/// 预览与运行时**必须**对同一份文档做出同样的解释，
/// 否则会出现"编辑器里看着在左边、跑起来在右边"这种最令人困惑的 bug。
pub fn 变换转预览(变换: 变换数据) -> Transform {
    crate::scene_bridge::变换数据转_bevy(变换)
}

/// 从 Sprite 组件的属性里取出视觉参数。
///
/// 返回 `(贴图路径, 颜色, 自定义尺寸, 水平翻转, 垂直翻转)`。
///
/// # 默认尺寸的处理
///
/// `自定义尺寸` 为空时返回 `None`，由调用方决定用什么默认值。
/// 用 `None` 而不是在这里编一个数字，是因为"默认尺寸"这件事
/// 应该与 `editor_ui` 的视口包围框**用同一个约定** ——
/// 否则选取范围与实际渲染会不一致。调用方统一走
/// [`默认预览尺寸`]。
pub fn 解析精灵属性(
    属性: &std::collections::BTreeMap<String, 属性值>
) -> 精灵视觉参数 {
    let 贴图 = 属性
        .get("贴图")
        .and_then(属性值::取资源)
        .map(|引用| 引用.路径.文本().to_string());

    let 颜色 = 属性
        .get("颜色")
        .and_then(属性值::取颜色)
        .unwrap_or(颜色值::白());

    // 空值表示"用贴图原始尺寸"，取不到具体值时返回 None。
    let 自定义尺寸 = 属性.get("自定义尺寸").and_then(属性值::取二维);

    let 水平翻转 = 属性
        .get("水平翻转")
        .and_then(属性值::取布尔)
        .unwrap_or(false);
    let 垂直翻转 = 属性
        .get("垂直翻转")
        .and_then(属性值::取布尔)
        .unwrap_or(false);

    精灵视觉参数 {
        贴图路径: 贴图,
        颜色,
        自定义尺寸,
        水平翻转,
        垂直翻转,
    }
}

/// 解析出的精灵视觉参数。
#[derive(Debug, Clone, PartialEq)]
pub struct 精灵视觉参数 {
    /// 贴图的项目相对路径。
    pub 贴图路径: Option<String>,
    /// 着色。
    pub 颜色: 颜色值,
    /// 自定义尺寸。`None` 表示未设置。
    pub 自定义尺寸: Option<(f32, f32)>,
    /// 水平翻转。
    pub 水平翻转: bool,
    /// 垂直翻转。
    pub 垂直翻转: bool,
}

/// 没有贴图、也没有自定义尺寸时的默认预览尺寸（世界单位）。
///
/// 这个值必须与 `editor_ui::viewport_panel` 计算包围框时用的
/// 默认值**保持一致**，否则"点得到"与"看得到"会对不上。
/// 测试 `默认预览尺寸与视口包围框约定一致` 守住这条。
pub const 默认精灵尺寸: f32 = 64.0;

/// 计算预览实体的最终显示尺寸（世界单位）。
pub fn 预览显示尺寸(参数: &精灵视觉参数) -> (f32, f32) {
    match 参数.自定义尺寸 {
        Some((宽, 高)) if 宽.abs() > f32::EPSILON && 高.abs() > f32::EPSILON => {
            (宽.abs(), 高.abs())
        }
        _ => (默认精灵尺寸, 默认精灵尺寸),
    }
}

/// 一个文档实体应当产生的预览渲染项。
#[derive(Debug, Clone, PartialEq)]
pub struct 预览渲染项 {
    /// 对应的文档实体标识。
    pub 来源: 实体标识,
    /// 该实体的变换（已经过与运行时一致的转换）。
    pub 变换: Transform,
    /// 精灵视觉参数。
    pub 视觉: 精灵视觉参数,
    /// 最终显示尺寸（世界单位）。
    pub 显示尺寸: (f32, f32),
}

/// 从场景文档投影出"应当渲染哪些实体"。
///
/// # 这是一个纯函数
///
/// 把"该渲染什么"的判断从 ECS 操作里抽出来，好处有三：
///
/// 1. **可测**：不需要 `AssetServer`、纹理、相机或任何 Bevy 资源
///    就能完整验证投影规则（可见性、组件要求、尺寸、变换）；
/// 2. **可读**：`同步预览实体` 只剩机械的 spawn，
///    所有"业务判断"集中在这一个函数里；
/// 3. **可复用**：将来若要做"导出场景预览图"，可以直接复用这个列表。
///
/// # 投影规则
///
/// 一个实体进入预览，必须同时满足：
///
/// * 有**变换**（没有变换的是纯逻辑节点，没有位置可渲染）；
/// * **没有**被编辑器隐藏（否则"隐藏"按钮在视口里看不出效果）；
/// * 有 **`Sprite` 组件**（本阶段只渲染精灵；
///   非精灵实体的位置由视口的包围框叠加层表示）。
///
/// 顺序与 `深度优先遍历` 一致，便于与场景树对照排查。
pub fn 投影预览项(文档: &场景文档) -> Vec<预览渲染项> {
    let mut 结果 = Vec::new();

    for (标识, _深度) in 文档.深度优先遍历() {
        let Some(实体) = 文档.取(标识) else {
            continue;
        };
        let Some(变换数据) = 实体.变换 else {
            continue;
        };
        if 实体.编辑器隐藏 {
            continue;
        }
        let Some(精灵组件) = 实体.取组件("Sprite") else {
            continue;
        };

        let 视觉 = 解析精灵属性(&精灵组件.属性);
        let 显示尺寸 = 预览显示尺寸(&视觉);
        结果.push(预览渲染项 {
            来源: 标识,
            变换: 变换转预览(变换数据),
            视觉,
            显示尺寸,
        });
    }

    结果
}

/// 把颜色值转成 Bevy 颜色。
pub fn 颜色转_bevy(颜色: 颜色值) -> Color {
    if 颜色.是_srgb {
        Color::srgba(颜色.红, 颜色.绿, 颜色.蓝, 颜色.透明度)
    } else {
        Color::linear_rgba(颜色.红, 颜色.绿, 颜色.蓝, 颜色.透明度)
    }
}

// ======================================================================
// 系统
// ======================================================================

/// 初始化预览：创建渲染目标纹理、预览相机、界面相机。
///
/// # 为什么必须显式管理 egui 的 primary context
///
/// `bevy_egui` 的默认行为是"把 context 挂到应用创建的**第一台**相机上"
/// （见 `EguiGlobalSettings::auto_create_primary_context`）。
///
/// 本编辑器有两台相机：一台渲染场景到纹理，一台用于窗口。
/// 如果放任默认行为，context 很可能被挂到**预览相机**上 ——
/// 后果是整份编辑器界面被画进那张纹理里，窗口上一片空白，
/// 而纹理里显示的又是界面自己，形成一个非常难排查的现象。
///
/// 因此这里：
/// 1. 关掉 `auto_create_primary_context`；
/// 2. 显式创建一台窗口相机并标上 `PrimaryEguiContext`；
/// 3. 预览相机用 `RenderLayers` 与它隔离。
pub fn 初始化预览资源(
    mut 命令: Commands,
    mut 网格资源: ResMut<Assets<Mesh>>,
    mut 图片资源: ResMut<Assets<Image>>,
    mut egui纹理表: ResMut<EguiUserTextures>,
    mut egui全局设置: ResMut<bevy_egui::EguiGlobalSettings>,
    mut 状态: ResMut<预览相机状态>,
) {
    // ① 交给 egui 显示、同时作为相机渲染目标的纹理。
    let 尺寸 = Extent3d {
        width: 512,
        height: 512,
        ..default()
    };
    let mut 图片 = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("bevy_studio_预览渲染目标"),
            size: 尺寸,
            dimension: TextureDimension::D2,
            // 用 `Rgba8UnormSrgb` 而不是 `Bgra8UnormSrgb`：
            // `bevy_egui` 交给 egui 的纹理一律按 **RGBA** 处理
            // （它自己的字体图集也是 `Rgba8UnormSrgb`，见 bevy_egui 的
            // `render/mod.rs`），而且它在 bindless 模式下会把用户纹理与
            // 字体图集放进同一个纹理数组 —— 与图集保持同一种格式最稳妥。
            //
            // 注：曾经把画面上的红蓝对调误判成这里的格式问题，
            // 其实那是抓图脚本把 32 位 DIB 当 RGBA 解码导致的
            // （见 `.verify/抓图.ps1`）。这个格式选择本身是对的，
            // 但它并不是那个现象的成因。
            format: TextureFormat::Rgba8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    // 必须先分配像素缓冲，否则渲染后端拿不到可写的纹理。
    图片.resize(尺寸);
    let 图片句柄 = 图片资源.add(图片);

    let 纹理 = egui纹理表.add_image(EguiTextureHandle::Strong(图片句柄.clone()));

    // ② 预览相机：渲染到纹理，只看预览层。
    let 预览层 = RenderLayers::layer(预览渲染层);
    let 相机实体 = 命令
        .spawn((
            预览相机,
            Camera2d,
            Camera {
                // 在窗口相机之前渲染。顺序不是正确性要求，但能让
                // 一帧内纹理先就绪、界面后绘制，减少一帧延迟。
                order: -1,
                // 关键：清成**全透明**，让 egui 画的网格能从下面透过来。
                // 详见 `editor_ui::viewport_panel::绘制内容` 的绘制顺序说明。
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                // 明确指定可见世界区域而不是依赖 WindowSize + scale：
                // 相机的世界范围直接由 `预览相机状态` 驱动，
                // 与 `视口相机::可见世界矩形` 用同一个矩形，
                // 因此渲染结果与 egui 叠加层按构造对齐。
                scaling_mode: ScalingMode::Fixed {
                    width: 512.0,
                    height: 512.0,
                },
                ..OrthographicProjection::default_2d()
            }),
            RenderTarget::Image(图片句柄.clone().into()),
            预览层,
            Transform::default(),
        ))
        .id();

    状态.相机实体 = Some(相机实体);

    // ③ 界面相机：承载 egui，渲染到窗口。
    命令.spawn((
        编辑器界面相机,
        Camera2d,
        Camera {
            order: 0,
            ..default()
        },
        bevy_egui::PrimaryEguiContext,
    ));

    // ④ 关掉自动挂载，避免 context 被挂到预览相机上。
    egui全局设置.auto_create_primary_context = false;

    命令.insert_resource(预览渲染目标 {
        图片: 图片句柄,
        尺寸: (512, 512),
        纹理: Some(纹理),
    });
    命令.insert_resource(贴图缓存::default());
    命令.insert_resource(预览同步状态::default());
    // 三维实体用同一个 1×1 四边形，按实体显示尺寸缩放：
    // 一个网格句柄服务全部实体，比每个实体建一个网格省得多。
    命令.insert_resource(三维预览资源 {
        四边形: 网格资源.add(Rectangle::new(1.0, 1.0)),
    });
    命令.insert_resource(三维预览状态::default());

    info!("预览视口已初始化：512×512 渲染目标 + 预览相机 + 界面相机");
}

/// 视口尺寸变化时重建渲染目标。
///
/// 只在尺寸**真的变了**的时候才重建：重建一张纹理要重新分配显存、
/// 并让绑定到旧纹理的 egui 句柄失效，绝不能每帧做。
pub fn 同步渲染尺寸(
    mut 渲染目标: ResMut<预览渲染目标>,
    mut 图片资源: ResMut<Assets<Image>>,
    mut egui纹理表: ResMut<EguiUserTextures>,
    界面: Res<crate::plugin::UiStateHolder>,
) {
    let 期望 = 收敛渲染尺寸(界面.状态.视口尺寸.x, 界面.状态.视口尺寸.y);
    if 期望 == 渲染目标.尺寸 {
        return;
    }

    let Some(mut 图片) = 图片资源.get_mut(&渲染目标.图片) else {
        return;
    };
    let 尺寸 = Extent3d {
        width: 期望.0,
        height: 期望.1,
        ..default()
    };
    图片.resize(尺寸);

    // 已经注册过的句柄继续有效（同一 AssetId），不需要重新 add_image，
    // 但需要把尺寸记下来供界面按正确比例显示。
    渲染目标.尺寸 = 期望;

    // 防御性：如果纹理句柄意外丢失，这里补注册一次。
    if 渲染目标.纹理.is_none() {
        渲染目标.纹理 =
            Some(egui纹理表.add_image(EguiTextureHandle::Strong(渲染目标.图片.clone())));
    }
}

/// 把界面的视口相机状态同步到预览相机。
///
/// 转换关系（这是全部坐标一致性的所在）：
///
/// ```text
/// 可见世界矩形 = 视口相机.可见世界矩形(视口像素尺寸)
/// 相机平移     = 矩形中心
/// 投影面积     = 矩形宽 × 矩形高    （ScalingMode::Fixed）
/// ```
///
/// 于是 Bevy 把该矩形铺满整张纹理，而 egui 叠加层把同一矩形铺满
/// 同一个视口区域 —— 两者按构造一致，不需要两边各自"小心地算对"。
pub fn 同步预览相机(
    mut 相机状态: ResMut<预览相机状态>,
    mut 相机查询: Query<(&mut Transform, &mut Projection), With<预览相机>>,
    界面: Res<crate::plugin::UiStateHolder>,
) {
    // 三维模式下相机由 `同步三维相机` 驱动，这里必须让位：
    // 否则 2D 的正交投影会把三维相机覆盖成"看不见东西"的状态。
    if 界面.状态.模式.是三维() {
        return;
    }
    let 视口相机 = 界面.状态.相机;
    let 视口尺寸 = 界面.状态.视口尺寸;
    if 视口尺寸.x <= 0.0 || 视口尺寸.y <= 0.0 {
        return;
    }

    let (最小, 最大) = 视口相机.可见世界矩形(视口尺寸);
    let 中心 = ((最小.0 + 最大.0) * 0.5, (最小.1 + 最大.1) * 0.5);
    let 宽 = (最大.0 - 最小.0).abs().max(f32::EPSILON);
    let 高 = (最大.1 - 最小.1).abs().max(f32::EPSILON);

    相机状态.中心 = 中心;
    相机状态.缩放 = 视口相机.缩放;

    for (mut 变换, mut 投影) in &mut 相机查询 {
        // z 保持为正，确保相机在 2D 内容前方（精灵 z 通常为 0）。
        变换.translation = Vec3::new(中心.0, 中心.1, 999.0);
        变换.rotation = Quat::IDENTITY;
        变换.scale = Vec3::ONE;

        if let Projection::Orthographic(正交) = 投影.as_mut() {
            正交.scaling_mode = ScalingMode::Fixed {
                width: 宽,
                height: 高,
            };
        }
    }
}

/// 文档 → 预览实体的增量同步。
///
/// # 为什么用修订号而不是每帧全量重比
///
/// 全量重比是 O(实体数)，每帧做一次等于持续消耗帧时间；
/// 而文档绝大多数帧根本没变。用 [`编辑器状态::文档修订`] 做 O(1) 判断，
/// 修订号没变就整个跳过。
///
/// 修订号由 `editor_core` 里的**唯一修改入口**维护，
/// 因此不可能出现"文档变了但修订号没变"导致预览不更新的情况
/// （`editor_core` 有专门的测试守着这条）。
///
/// # 同步策略：整体重建
///
/// 修订号变化时**整体重建**预览实体，而不是做逐实体 diff。
/// 理由：一次编辑通常只影响少数实体，但 diff 需要双向比对
/// （新增/删除/改组件/改父子），代码量与出错面远大于重建；
/// 而重建只是 despawn 几个实体再 spawn 几个，在编辑器量级
/// （数百到数千实体）下开销可以接受。
///
/// 这是刻意的取舍，不是偷懒：**正确性优先于这点性能**。
/// 若将来大场景实测成为瓶颈，再引入 diff。
#[allow(clippy::too_many_arguments)]
pub fn 同步预览实体(
    mut 命令: Commands,
    编辑器: Res<EditorStateHolder>,
    mut 同步状态: ResMut<预览同步状态>,
    已有: Query<(Entity, &预览来源), With<预览实体>>,
    mut 贴图缓存: ResMut<贴图缓存>,
    资源服务器: Res<AssetServer>,
    三维资源: Res<三维预览资源>,
    mut 材质表: ResMut<Assets<StandardMaterial>>,
) {
    let 修订 = 编辑器.状态.文档修订();
    if 同步状态.已同步修订 == Some(修订) {
        return;
    }

    // 先清掉旧的预览实体。
    for (实体, _) in &已有 {
        命令.entity(实体).despawn();
    }

    let 文档 = &编辑器.状态.会话.文档;
    let 项目根 = 编辑器
        .状态
        .项目
        .as_ref()
        .map(|项目| 项目.根目录().to_path_buf());

    // 投影判断全部在纯函数里完成，这里只负责把结果变成 ECS 实体。
    let 待渲染 = 投影预览项(文档);

    for 项 in &待渲染 {
        let 图片 = 解析贴图句柄(
            项.视觉.贴图路径.as_deref(),
            项目根.as_deref(),
            &mut 贴图缓存,
            &资源服务器,
        );

        命令.spawn((
            预览实体,
            预览来源(项.来源),
            Sprite {
                image: 图片.clone(),
                color: 颜色转_bevy(项.视觉.颜色),
                flip_x: 项.视觉.水平翻转,
                flip_y: 项.视觉.垂直翻转,
                custom_size: Some(Vec2::new(项.显示尺寸.0, 项.显示尺寸.1)),
                ..default()
            },
            项.变换,
            RenderLayers::layer(预览渲染层),
        ));

        // ---- 三维对应物 ----
        //
        // 精灵在 3D 相机下不可见（Bevy 的精灵是 2D 管线），因此每个实体
        // 额外生成一个"贴了同一张图的四边形"，放在独立渲染层上由 Camera3d 渲染。
        // 它复用实体的完整三维变换（平移/旋转/缩放），所以 3D 视图里的
        // 位置就是文档里的真实坐标 —— 不是另算一份。
        let mut 材质 = StandardMaterial {
            base_color: 颜色转_bevy(项.视觉.颜色),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            // 双面可见：编辑器经常从背面看一个精灵。
            cull_mode: None,
            ..default()
        };
        if !图片.is_strong() {
            // 没有贴图时保持纯色方块，而不是显示一张 1×1 的占位图。
        } else {
            材质.base_color_texture = Some(图片.clone());
        }
        let 材质句柄 = 材质表.add(材质);
        let mut 三维变换 = 项.变换;
        三维变换.scale.x *= 项.显示尺寸.0;
        三维变换.scale.y *= 项.显示尺寸.1;
        命令.spawn((
            预览实体,
            预览来源(项.来源),
            Mesh3d(三维资源.四边形.clone()),
            MeshMaterial3d(材质句柄),
            三维变换,
            RenderLayers::layer(三维渲染层),
        ));
    }

    同步状态.已同步修订 = Some(修订);
    同步状态.预览实体数 = 待渲染.len();
}

/// 三维模式下驱动预览相机：换成 `Camera3d`、跟随轨道相机。
///
/// # 为什么"换相机"而不是一直留两台
///
/// 留两台相机（一台 2D、一台 3D）同时渲染到同一张纹理，结果是两张图
/// 叠加在一起、并且每帧多跑一遍完整渲染。模式切换是低频操作
/// （人不会一秒切十次），despawn + spawn 一台相机的代价完全可以接受，
/// 换来的是"任何时候只有一条渲染路径"的简单性。
pub fn 同步三维相机(
    mut 命令: Commands,
    mut 相机状态: ResMut<预览相机状态>,
    mut 三维状态: ResMut<三维预览状态>,
    渲染目标: Res<预览渲染目标>,
    界面: Res<crate::plugin::UiStateHolder>,
    mut 相机查询: Query<&mut Transform, With<预览相机>>,
) {
    let 想要三维 = 界面.状态.模式.是三维();

    if 想要三维 != 三维状态.已三维 {
        if let Some(旧) = 相机状态.相机实体.take() {
            命令.entity(旧).despawn();
        }
        let 实体 = if 想要三维 {
            命令
                .spawn((
                    预览相机,
                    Camera3d::default(),
                    Camera {
                        order: -1,
                        // 三维视图也要让 egui 叠加层透过来：清成透明。
                        clear_color: ClearColorConfig::Custom(Color::NONE),
                        ..default()
                    },
                    Projection::Perspective(PerspectiveProjection {
                        fov: 界面.状态.三维相机.视场角,
                        ..default()
                    }),
                    RenderTarget::Image(渲染目标.图片.clone().into()),
                    RenderLayers::layer(三维渲染层),
                    Transform::default(),
                ))
                .id()
        } else {
            命令
                .spawn((
                    预览相机,
                    Camera2d,
                    Camera {
                        order: -1,
                        clear_color: ClearColorConfig::Custom(Color::NONE),
                        ..default()
                    },
                    Projection::Orthographic(OrthographicProjection {
                        scaling_mode: ScalingMode::Fixed {
                            width: 512.0,
                            height: 512.0,
                        },
                        ..OrthographicProjection::default_2d()
                    }),
                    RenderTarget::Image(渲染目标.图片.clone().into()),
                    RenderLayers::layer(预览渲染层),
                    Transform::default(),
                ))
                .id()
        };
        相机状态.相机实体 = Some(实体);
        三维状态.已三维 = 想要三维;
        info!(
            "预览相机已切换为{}",
            if 想要三维 {
                "三维透视"
            } else {
                "二维正交"
            }
        );
        return;
    }

    if !想要三维 {
        return;
    }

    // 跟随轨道相机：位置由同一组参数推导，与 egui 叠加层的投影一致。
    let 相机 = 界面.状态.三维相机;
    let 位置 = 相机.位置();
    if let Some(实体) = 相机状态.相机实体
        && let Ok(mut 变换) = 相机查询.get_mut(实体)
    {
        变换.translation = Vec3::new(位置.0, 位置.1, 位置.2);
        变换.look_at(Vec3::new(相机.目标.0, 相机.目标.1, 相机.目标.2), Vec3::Y);
    }
}

/// 解析贴图句柄，带缓存。
///
/// 加载失败或尚未打开项目时返回 [`Handle::default()`]（一张 1×1 的占位图）。
/// 这样精灵仍会以纯色方块显示 —— 用户能看到"这里有东西、尺寸多大"，
/// 而不是什么都不显示而误以为实体丢了。
fn 解析贴图句柄(
    路径: Option<&str>,
    项目根: Option<&std::path::Path>,
    缓存: &mut 贴图缓存,
    资源服务器: &AssetServer,
) -> Handle<Image> {
    let (Some(路径), Some(根)) = (路径, 项目根) else {
        return Handle::default();
    };

    if let Some(句柄) = 缓存.取(路径) {
        return 句柄.clone();
    }

    // 场景里的路径是项目相对的（例如 `assets/textures/hero.png`）。
    // Bevy 的资源根目录在启动时就固定了，无法在打开项目时改写，
    // 因此这里用**绝对路径**加载：Bevy 的文件读取器会
    // `root.join(路径)`，而 `Path::join` 遇到绝对路径时会直接替换，
    // 于是绝对路径能正确绕过资源根目录的限制。
    let 绝对 = 根.join(路径.replace('/', std::path::MAIN_SEPARATOR_STR));
    let 句柄 = if 绝对.is_file() {
        Some(资源服务器.load::<Image>(bevy::asset::AssetPath::from_path_buf(绝对)))
    } else {
        None
    };
    缓存.记录(路径.to_string(), 句柄.clone());
    句柄.unwrap_or_default()
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::{场景文档, 组件数据};

    #[test]
    fn 渲染尺寸被收敛到合法区间() {
        // 太小 → 抬到下限
        assert_eq!(收敛渲染尺寸(0.0, 0.0), (预览尺寸下限, 预览尺寸下限));
        assert_eq!(收敛渲染尺寸(-100.0, 10.0), (预览尺寸下限, 预览尺寸下限));
        assert_eq!(收敛渲染尺寸(1.0, 1.0), (预览尺寸下限, 预览尺寸下限));

        // 正常值原样通过（四舍五入到整数）
        assert_eq!(收敛渲染尺寸(800.0, 600.0), (800, 600));
        assert_eq!(收敛渲染尺寸(800.4, 600.6), (800, 601));

        // 太大 → 压到上限
        assert_eq!(收敛渲染尺寸(8000.0, 5000.0), (预览尺寸上限, 预览尺寸上限));

        // 非有限值不能产生 0 尺寸纹理（那会让渲染后端报错）
        let (宽, 高) = 收敛渲染尺寸(f32::NAN, f32::INFINITY);
        assert!(宽 >= 预览尺寸下限 && 高 >= 预览尺寸下限);
    }

    #[test]
    fn 解析精灵属性会取出全部字段() {
        let mut 属性 = std::collections::BTreeMap::new();
        属性.insert(
            "贴图".to_string(),
            属性值::资源(scene_doc::property::资源引用值::按路径(
                scene_doc::path::项目相对路径::解析("assets/textures/hero.png").unwrap(),
            )),
        );
        属性.insert(
            "颜色".to_string(),
            属性值::颜色(颜色值::srgb(0.25, 0.5, 0.75, 1.0)),
        );
        属性.insert("自定义尺寸".to_string(), 属性值::二维(32.0, 48.0));
        属性.insert("水平翻转".to_string(), 属性值::布尔(true));

        let 参数 = 解析精灵属性(&属性);
        assert_eq!(参数.贴图路径.as_deref(), Some("assets/textures/hero.png"));
        assert_eq!(参数.颜色.红, 0.25);
        assert_eq!(参数.自定义尺寸, Some((32.0, 48.0)));
        assert!(参数.水平翻转);
        assert!(!参数.垂直翻转, "未设置的翻转字段应默认为 false");
    }

    #[test]
    fn 解析精灵属性对缺失字段有合理默认() {
        let 空 = std::collections::BTreeMap::new();
        let 参数 = 解析精灵属性(&空);
        assert_eq!(参数.贴图路径, None);
        assert_eq!(参数.颜色, 颜色值::白(), "无颜色时应默认白色而不是透明");
        assert_eq!(参数.自定义尺寸, None);
        assert!(!参数.水平翻转 && !参数.垂直翻转);
    }

    #[test]
    fn 显示尺寸使用自定义尺寸否则用默认() {
        let 有尺寸 = 精灵视觉参数 {
            贴图路径: None,
            颜色: 颜色值::白(),
            自定义尺寸: Some((120.0, 80.0)),
            水平翻转: false,
            垂直翻转: false,
        };
        assert_eq!(预览显示尺寸(&有尺寸), (120.0, 80.0));

        let 无尺寸 = 精灵视觉参数 {
            自定义尺寸: None,
            ..有尺寸.clone()
        };
        assert_eq!(预览显示尺寸(&无尺寸), (默认精灵尺寸, 默认精灵尺寸));

        // 零尺寸会让精灵不可见，应回退到默认值而不是渲染出 0 宽的东西
        let 零尺寸 = 精灵视觉参数 {
            自定义尺寸: Some((0.0, 0.0)),
            ..有尺寸.clone()
        };
        assert_eq!(预览显示尺寸(&零尺寸), (默认精灵尺寸, 默认精灵尺寸));

        // 负尺寸取绝对值（镜像场景）
        let 负尺寸 = 精灵视觉参数 {
            自定义尺寸: Some((-64.0, -32.0)),
            ..有尺寸
        };
        assert_eq!(预览显示尺寸(&负尺寸), (64.0, 32.0));
    }

    /// 默认预览尺寸必须与视口的包围框约定一致。
    ///
    /// 如果这里用一个值、视口用另一个值，就会出现"点得到但看不见"
    /// 或"看得见但点不到" —— 这是 2D 编辑器最影响手感的一类 bug。
    #[test]
    fn 默认预览尺寸与视口包围框约定一致() {
        // 视口侧对带 Sprite 的实体的默认半尺寸是 32.0，即直径 64.0。
        const 视口侧默认直径: f32 = 64.0;
        assert_eq!(
            默认精灵尺寸, 视口侧默认直径,
            "预览渲染的默认尺寸必须与 editor_ui 视口包围框的默认尺寸一致，\
             否则选取范围与实际显示位置会对不上"
        );
    }

    #[test]
    fn 颜色转_bevy_区分色彩空间() {
        // 必须用中间值来验证色彩空间标记生效。
        //
        // 端点值（0.0 与 1.0）在线性与 sRGB 下映射结果相同 ——
        // 用 1.0 写这条测试会永远通过，却什么也没验证到
        // （第一版就是这么写的，被实测纠正了）。
        let srgb = bevy::color::Srgba::from(颜色转_bevy(颜色值::srgb(0.5, 0.5, 0.5, 1.0)));
        let 线性 = bevy::color::Srgba::from(颜色转_bevy(颜色值::线性(0.5, 0.5, 0.5, 1.0)));

        assert!(
            (srgb.red - 0.5).abs() < 1e-4,
            "标记为 sRGB 的分量应当原样保留，实际 {}",
            srgb.red
        );
        assert!(
            线性.red > srgb.red + 0.1,
            "线性空间的 0.5 转到 sRGB 后应明显大于 0.5（约 0.735），\
             否则说明色彩空间标记被忽略了：srgb={} 线性={}",
            srgb.red,
            线性.red
        );
    }

    #[test]
    fn 变换转换与运行时桥接保持一致() {
        // 预览与运行时必须对同一份文档做出同样的解释。
        let 变换 = 变换数据 {
            平移: (12.0, -34.0, 0.0),
            旋转: (0.0, 0.0, 0.0, 1.0),
            缩放: (2.0, 3.0, 1.0),
        };
        let 预览 = 变换转预览(变换);
        let 运行时 = crate::scene_bridge::变换数据转_bevy(变换);
        assert_eq!(预览.translation, 运行时.translation);
        assert_eq!(预览.scale, 运行时.scale);
        assert_eq!(预览.rotation, 运行时.rotation);
    }

    #[test]
    fn 贴图缓存记录并复用请求() {
        let mut 缓存 = 贴图缓存::default();
        assert!(!缓存.已请求("a.png"));
        assert!(缓存.取("a.png").is_none());

        缓存.记录("a.png", None);
        assert!(缓存.已请求("a.png"), "请求过的路径应当被记住");
        assert!(缓存.取("a.png").is_none(), "加载失败的记录不应给出句柄");
        assert_eq!(缓存.数量(), 1);

        // 重复记录同一路径不应让缓存增长
        缓存.记录("a.png", None);
        assert_eq!(缓存.数量(), 1, "同一路径不应重复占用缓存条目");
    }

    #[test]
    fn 预览渲染目标在无纹理时不给渲染结果() {
        let mut 目标 = 预览渲染目标 {
            图片: Handle::default(),
            尺寸: (800, 600),
            纹理: None,
        };
        assert!(
            目标.渲染结果().is_none(),
            "纹理尚未注册时不应把无效句柄交给界面"
        );

        目标.纹理 = Some(egui::TextureId::User(7));
        let 结果 = 目标.渲染结果().expect("有纹理时应给出渲染结果");
        assert_eq!(结果.像素尺寸, (800, 600));
    }

    /// 只有 Sprite 组件会被渲染成可见精灵。
    #[test]
    fn 只有带精灵组件的实体产生可见渲染() {
        let mut 文档 = 场景文档::新建("预览投影");
        let 带精灵 = 文档.生成实体("带精灵", None);
        let 纯节点 = 文档.生成实体("纯节点", None);
        let 无变换 = 文档.生成实体("无变换", None);

        let mut 属性 = std::collections::BTreeMap::new();
        属性.insert("自定义尺寸".to_string(), 属性值::二维(64.0, 64.0));
        if let Some(实体) = 文档.取可变(带精灵) {
            实体.加组件(组件数据::新建("Sprite", 属性));
        }
        if let Some(实体) = 文档.取可变(无变换) {
            实体.变换 = None;
        }

        let 项 = 投影预览项(&文档);
        let 来源: Vec<实体标识> = 项.iter().map(|项| 项.来源).collect();

        assert_eq!(来源, vec![带精灵], "只有带 Sprite 且有变换的实体才应渲染");
        assert!(!来源.contains(&纯节点));
        assert!(!来源.contains(&无变换));
    }

    /// 隐藏的实体不应产生渲染实体。
    #[test]
    fn 隐藏实体不产生预览渲染() {
        let mut 文档 = 场景文档::新建("隐藏投影");
        let 可见 = 文档.生成实体("可见", None);
        let 隐藏 = 文档.生成实体("被隐藏的", None);
        for 标识 in [可见, 隐藏] {
            if let Some(实体) = 文档.取可变(标识) {
                实体.加组件(组件数据::新建("Sprite", Default::default()));
            }
        }
        if let Some(实体) = 文档.取可变(隐藏) {
            实体.编辑器隐藏 = true;
        }

        let 项 = 投影预览项(&文档);
        assert_eq!(项.len(), 1, "隐藏实体不应进入预览投影");
        assert_eq!(项[0].来源, 可见);
    }

    /// 投影顺序必须与深度优先遍历一致，便于与场景树对照排查。
    #[test]
    fn 投影顺序与场景树一致() {
        let mut 文档 = 场景文档::新建("顺序");
        let 根 = 文档.生成实体("根", None);
        let 子 = 文档.生成实体("子", Some(根));
        for 标识 in [根, 子] {
            if let Some(实体) = 文档.取可变(标识) {
                实体.加组件(组件数据::新建("Sprite", Default::default()));
            }
        }

        let 来源: Vec<实体标识> = 投影预览项(&文档).iter().map(|项| 项.来源).collect();
        let 树顺序: Vec<实体标识> = 文档
            .深度优先遍历()
            .into_iter()
            .map(|(标识, _)| 标识)
            .collect();
        assert_eq!(来源, 树顺序);
    }

    /// 投影项必须携带文档里的真实变换与尺寸，不能是默认值。
    #[test]
    fn 投影携带真实变换与尺寸() {
        let mut 文档 = 场景文档::新建("变换投影");
        let 标识 = 文档.生成实体("精灵", None);

        let mut 属性 = std::collections::BTreeMap::new();
        属性.insert("自定义尺寸".to_string(), 属性值::二维(120.0, 80.0));
        属性.insert(
            "颜色".to_string(),
            属性值::颜色(颜色值::srgb(0.2, 0.4, 0.6, 1.0)),
        );
        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(组件数据::新建("Sprite", 属性));
            实体.变换 = Some(变换数据 {
                平移: (150.0, -75.0, 0.0),
                旋转: (0.0, 0.0, 0.0, 1.0),
                缩放: (3.0, 3.0, 1.0),
            });
        }

        let 项 = 投影预览项(&文档);
        assert_eq!(项.len(), 1);
        let 项 = &项[0];
        assert_eq!(
            项.变换.translation,
            Vec3::new(150.0, -75.0, 0.0),
            "预览变换必须来自文档而不是默认值"
        );
        assert_eq!(
            项.变换.scale,
            Vec3::new(3.0, 3.0, 1.0),
            "缩放要原样搬过来；Z 轴是文档里的 1.0，不应被 splat 成 3.0"
        );
        assert_eq!(项.显示尺寸, (120.0, 80.0));
        assert!((项.视觉.颜色.蓝 - 0.6).abs() < 1e-5);
    }

    /// 空文档与全无精灵的文档都投影出空列表，不应 panic。
    #[test]
    fn 无精灵文档投影为空() {
        let 空 = 场景文档::新建("空");
        assert!(投影预览项(&空).is_empty());

        let mut 无精灵 = 场景文档::新建("无精灵");
        无精灵.生成实体("纯节点", None);
        assert!(投影预览项(&无精灵).is_empty());
    }

    /// 投影是**只读**的：调用它不会给文档增加任何"预览辅助实体"。
    ///
    /// 这条直接对应需求里的"视口辅助实体不进入场景保存结果"：
    /// 预览实体只存在于渲染世界，文档里从头到尾没有它们的位置。
    #[test]
    fn 投影不会往文档里塞辅助实体() {
        let mut 文档 = 场景文档::新建("纯净性");
        let 标识 = 文档.生成实体("精灵", None);
        if let Some(实体) = 文档.取可变(标识) {
            实体.加组件(组件数据::新建("Sprite", Default::default()));
        }
        let 投影前 = 文档.clone();

        let 项 = 投影预览项(&文档);
        assert_eq!(项.len(), 1, "应当投影出一个渲染项");

        assert_eq!(
            文档, 投影前,
            "投影是只读操作，不得改动文档（例如为了渲染而偷偷加辅助实体）"
        );
        assert_eq!(文档.实体数量(), 1, "文档里始终只有用户创建的那一个实体");
    }

    /// 预览辅助的名字与标记都不应成为文档的一部分。
    ///
    /// 用一个具体的"辅助实体命名约定"来钉住这条约定：
    /// 如果将来有人真的把预览实体写进文档，名字里通常会带这些词，
    /// 这条测试会立刻失败。
    #[test]
    fn 文档中不存在任何预览辅助命名() {
        let mut 文档 = 场景文档::新建("命名检查");
        for 名称 in ["精灵甲", "精灵乙", "相机", "网格"] {
            let 标识 = 文档.生成实体(名称, None);
            if let Some(实体) = 文档.取可变(标识) {
                实体.加组件(组件数据::新建("Sprite", Default::default()));
            }
        }
        let _ = 投影预览项(&文档);

        // 文档里的实体名必须全部是**用户起的名字**，
        // 不存在渲染内部产生的名字。
        let 允许的名字 = ["精灵甲", "精灵乙", "相机", "网格"];
        for 实体 in 文档.实体表.values() {
            assert!(
                允许的名字.contains(&实体.名称.as_str()),
                "文档里出现了非用户创建的实体 `{}` —— \
                 预览/网格/相机等辅助对象绝不能进入场景文档",
                实体.名称
            );
        }
        assert_eq!(文档.实体数量(), 4);
    }
}
