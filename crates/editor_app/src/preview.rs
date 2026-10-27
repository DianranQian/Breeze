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

use bevy::camera::primitives::Aabb;
use bevy::camera::{RenderTarget, ScalingMode, visibility::RenderLayers};
use bevy::math::Affine2;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::AlphaMode;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, Face, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use scene_doc::document::{变换数据, 场景文档, 实体标识};
use scene_doc::property::{属性值, 资源引用值, 颜色值};

use crate::plugin::EditorStateHolder;
use crate::scene_bridge::几何体形状;
use crate::tile_terrain::{地形参数, 地形缓存};

/// 预览实体标记。
///
/// 每个文档实体对应一个带此标记的**层级节点**：节点只承载文档变换与
/// 父子关系，精灵/四边形/网格等视觉实体作为它的子实体存在。
/// 这样"哪些实体来自文档"仍然可查询、可测试，
/// 而世界变换只有 `GlobalTransform` 传播这一条来源。
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
    /// 上次同步时的资源注册表代次。
    ///
    /// 材质、贴图这类**项目资源**的内容不在文档里，改它们不会推进文档修订号。
    /// 把代次一起当守卫，"刷新资源/导入/材质面板保存"之后视口才会重新取材质，
    /// 而不需要用户再去动一下场景。
    pub 已同步代次: Option<u64>,
    /// 上次同步建立的预览实体数（用于日志与诊断）。
    pub 预览实体数: usize,
}

/// 预览世界变换快照：文档实体 → 预览节点当前的世界变换。
///
/// 界面层读不到 Bevy 世界，又绝不能把本地变换当世界变换用；
/// 这里在 `TransformSystems::Propagate` 之后把传播结果抄成纯数据，每帧覆盖。
/// 键是文档实体标识而不是 ECS `Entity`：预览重建会换掉底层实体，
/// 但文档标识不变，快照因此在重建的那一帧仍然可用。
///
/// 除分解后的平移/旋转/缩放外，还保存**世界变换的逆线性部分**：
/// 拖动与对齐算出的增量在世界空间，写入文档前必须用父级逆矩阵换算，
/// 否则父级带旋转/缩放时子实体会偏到错误位置。
#[derive(Resource, Default, Debug, Clone)]
pub struct 预览世界变换快照 {
    /// 供界面绘制包围框、操纵器使用的分解结果。
    pub 变换: std::collections::BTreeMap<实体标识, 变换数据>,
    /// 供世界位移换算使用的逆线性矩阵（行主序，键是实体自身）。
    pub 逆矩阵: editor_ui::world_space::世界逆矩阵表,
    /// 供 3D 视口射线拾取使用的世界 AABB。
    ///
    /// 键是文档实体；值是该实体自身与全部后代的网格包围盒并集。
    /// 模型实例的网格是 glTF 子实体、在文档里没有对应标识，
    /// 只靠文档属性算不出真实体积 —— 这一层必须由渲染侧提供。
    pub 包围盒: std::collections::BTreeMap<实体标识, editor_ui::picking3d::世界包围盒>,
}

/// 把预览节点的 `GlobalTransform` 抄成界面可读的快照。
///
/// 必须在变换传播之后运行；否则读到的是上一帧的位置，
/// 拖动或改层级后视口包围框会慢一帧。
///
/// 逆矩阵由 `GlobalTransform::affine().inverse()` 给出，不手写矩阵累加；
/// 退化变换（任一轴缩放为 0）没有逆，直接跳过该实体，换算时退回世界增量。
///
/// 包围盒取 `Aabb`（网格本地盒）× `GlobalTransform` 的 8 角点并集：
/// 只变换中心会低估旋转后的范围，用户点盒子边缘就会选不中。
/// 同一文档标识可能有多个预览实体（二维精灵 + 三维四边形 + 网格），
/// 因此按标识并集而不是覆盖。
pub fn 同步预览世界变换(
    已有: Query<(Entity, &预览来源, &GlobalTransform), With<预览实体>>,
    子查询: Query<&Children>,
    网格: Query<(&Aabb, &GlobalTransform)>,
    mut 快照: ResMut<预览世界变换快照>,
) {
    let 快照 = &mut *快照;
    let 变换表 = &mut 快照.变换;
    let 矩阵表 = &mut 快照.逆矩阵;
    let 包围盒表 = &mut 快照.包围盒;
    变换表.clear();
    矩阵表.clear();
    包围盒表.clear();
    for (根, 来源, 全局) in &已有 {
        let 平移 = 全局.translation();
        let 旋转 = 全局.rotation();
        let 缩放 = 全局.scale();
        变换表.insert(
            来源.0,
            变换数据 {
                平移: (平移.x, 平移.y, 平移.z),
                旋转: (旋转.x, 旋转.y, 旋转.z, 旋转.w),
                缩放: (缩放.x, 缩放.y, 缩放.z),
            },
        );

        let mut 盒: Option<editor_ui::picking3d::世界包围盒> = None;
        并入网格盒(&mut 盒, 根, &网格);
        let mut 栈 = vec![根];
        while let Some(当前) = 栈.pop() {
            let Ok(子列表) = 子查询.get(当前) else {
                continue;
            };
            for 子 in 子列表.iter() {
                栈.push(子);
                并入网格盒(&mut 盒, 子, &网格);
            }
        }
        if let Some(盒) = 盒 {
            包围盒表
                .entry(来源.0)
                .and_modify(|已有| 已有.并(盒))
                .or_insert(盒);
        }

        let 逆 = 全局.affine().inverse();
        if !逆.is_finite() {
            continue;
        }
        let 线性 = 逆.matrix3;
        矩阵表.insert(
            来源.0,
            [
                [线性.x_axis.x, 线性.y_axis.x, 线性.z_axis.x],
                [线性.x_axis.y, 线性.y_axis.y, 线性.z_axis.y],
                [线性.x_axis.z, 线性.y_axis.z, 线性.z_axis.z],
            ],
        );
    }
}

/// 把一个预览实体（含模型根自身的网格）的世界包围盒并入累加器。
fn 并入网格盒(
    累加: &mut Option<editor_ui::picking3d::世界包围盒>,
    实体: Entity,
    网格: &Query<(&Aabb, &GlobalTransform)>,
) {
    let Ok((aabb, 变换)) = 网格.get(实体) else {
        return;
    };
    let 仿射 = 变换.affine();
    let 中心 = Vec3::from(aabb.center);
    let 半 = Vec3::from(aabb.half_extents);
    let mut 最小 = Vec3::splat(f32::MAX);
    let mut 最大 = Vec3::splat(f32::MIN);
    for 序号 in 0..8 {
        let 符号 = Vec3::new(
            if 序号 & 1 == 0 { -1.0 } else { 1.0 },
            if 序号 & 2 == 0 { -1.0 } else { 1.0 },
            if 序号 & 4 == 0 { -1.0 } else { 1.0 },
        );
        let 世界 = 仿射.transform_point3(中心 + 半 * 符号);
        最小 = 最小.min(世界);
        最大 = 最大.max(世界);
    }
    let 盒 = editor_ui::picking3d::世界包围盒::由角点(
        (最小.x, 最小.y, 最小.z),
        (最大.x, 最大.y, 最大.z),
    );
    match 累加 {
        Some(已有) => 已有.并(盒),
        None => *累加 = Some(盒),
    }
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

/// 自发光颜色到 HDR 亮度（nits）的换算系数。
///
/// Bevy 0.20 的 `StandardMaterial::emissive` 用的是物理单位（cd/m²），
/// 而材质文档里的 `自发光` 是 `0.0..=1.0` 的调色颜色：直接把 `1.0` 塞进去
/// 只有 1 nit，在预览那盏 9000 lux 方向光下与不发光看不出区别，
/// 用户会以为这个字段没接线。这个系数把满量程自发光定在明显亮于受光表面、
/// 又不至于整块网格过曝成一团白的位置；纯视觉校准，改动只需看视口。
///
/// 全黑仍然是不发光：`0.0 × 系数 = 0.0`，与 [`材质资源`] 的默认值口径一致。
///
/// [`材质资源`]: editor_core::material::材质资源
pub const 自发光亮度系数: f32 = 2_000.0;

/// 一份材质投影要用的四张贴图句柄。
///
/// # 为什么收成一个结构体
///
/// 基础色以外的三张图都是可选的，语义上同属"材质自带的贴图"。继续往
/// [`材质转标准材质`] 后面排参数，它就会变成九个参数、其中六个是
/// `Option` 与 `bool`，调用点写错顺序编译器也拦不住；收成结构体后每个字段名
/// 自己说明它是哪一张图，测试也只需改一处构造。
///
/// `基础色` 用 [`Handle::default`] 表示"没挂图"：与 [`解析贴图句柄`] 的既有
/// 降级口径一致，无效句柄在投影阶段被过滤掉，不留下 1×1 占位图。
#[derive(Debug, Clone, Default)]
pub struct 材质贴图句柄 {
    /// 基础色（albedo）贴图。
    pub 基础色: Handle<Image>,
    /// 切线空间法线贴图。
    pub 法线: Option<Handle<Image>>,
    /// 金属粗糙度贴图（G 通道 = 粗糙度，B 通道 = 金属度）。
    pub 金属粗糙度: Option<Handle<Image>>,
    /// 环境光遮蔽贴图（R 通道）。
    pub 遮蔽: Option<Handle<Image>>,
}

/// 把一份材质文档投影成 Bevy 的 `StandardMaterial`。
///
/// # 字段归属：材质管表面，实体管渲染模式
///
/// | 来源 | 字段 |
/// | --- | --- |
/// | 材质 | `着色颜色` → `base_color`、`粗糙度` → `perceptual_roughness`、
///   `金属度` → `metallic`、`自发光` → `emissive`（乘 [`自发光亮度系数`]）、
///   `法线贴图` / `金属粗糙度贴图` / `遮蔽贴图` → `normal_map_texture` /
///   `metallic_roughness_texture` / `occlusion_texture` |
/// | 材质或实体 | `base_color_texture`（材质带贴图用材质的，否则用实体的）、
///   背面剔除（任一侧要求双面就不剔除） |
/// | 实体 | `受光` → `unlit`、`alpha_mode`、`uv_transform`、网格本身 |
///
/// 金属粗糙度贴图与 `metallic` / `perceptual_roughness` 是**相乘**关系
/// （Bevy 与 glTF 都如此）：贴图给空间变化，标量给整体偏移，两者一起用不是
/// 二选一，因此这里不做"有贴图就忽略标量"的取舍。
///
/// 材质**不碰**渲染模式：三维精灵四边形一直是 `unlit + Blend`，
/// 因为它必须与 2D 视图长得一样（见 [`同步预览实体`] 的"材质只换表面，不换渲染模式"）；
/// 一个表面参数改不动"这东西到底受不受光"，否则挂上材质就会让精灵在
/// 3D 视图里突然变暗，而 2D 视图毫无变化 —— 那是最难查的一类不一致。
///
/// 这是纯函数：不读盘、不碰 ECS，因此能被直接断言。
/// 读盘与缓存属于 [`解析材质句柄`]。
///
/// [`材质资源`]: editor_core::material::材质资源
pub fn 材质转标准材质(
    材质: &editor_core::material::材质资源,
    贴图: 材质贴图句柄,
    受光: bool,
    双面: bool,
    混合: bool,
    贴图重复: Option<(f32, f32)>,
) -> StandardMaterial {
    let 自发光 = LinearRgba::from(颜色转_bevy(材质.自发光));
    StandardMaterial {
        base_color: 颜色转_bevy(材质.着色颜色),
        base_color_texture: if 贴图.基础色.is_strong() {
            Some(贴图.基础色)
        } else {
            None
        },
        normal_map_texture: 贴图.法线,
        metallic_roughness_texture: 贴图.金属粗糙度,
        occlusion_texture: 贴图.遮蔽,
        metallic: 材质.金属度,
        perceptual_roughness: 材质.粗糙度,
        emissive: LinearRgba {
            red: 自发光.red * 自发光亮度系数,
            green: 自发光.green * 自发光亮度系数,
            blue: 自发光.blue * 自发光亮度系数,
            alpha: 自发光.alpha,
        },
        unlit: !受光,
        alpha_mode: if 混合 {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        cull_mode: if 双面 || 材质.双面 {
            None
        } else {
            Some(Face::Back)
        },
        uv_transform: 贴图重复
            .map(|(u, v)| Affine2::from_scale(Vec2::new(u, v)))
            .unwrap_or_default(),
        ..default()
    }
}

/// 材质转换缓存：材质引用 → 已转换的 `StandardMaterial` 句柄。
///
/// # 为什么必须缓存
///
/// `同步预览实体` 在文档修订号变化时**整体重建**预览。一个 3D 示例场景
/// 有几十到几百个实体，绝大多数共用同一份材质；不缓存就意味着每次拖动
/// 滑条都要重读几十次 `.material.ron`、重新解码 PNG、再往
/// `Assets<StandardMaterial>` 里塞几十份内容相同的材质。
///
/// # 键里为什么带资源注册表代次
///
/// 代次是"磁盘上的资源可能变了"的唯一权威信号：打开项目、导入、显式刷新、
/// 以及材质面板保存都会推进它。把代次写进键，材质文件被外部改动或
/// 在材质编辑器里改过之后，下一次同步自然落到新键上重新转换；
/// 旧键在同一次同步末尾由 [`材质缓存::清理本轮`] 丢弃，因此既不会复用旧内容，
/// 也不会越攒越多。
///
/// # 键里为什么还带实体侧的输入
///
/// `受光`、`双面`、`alpha_mode`、贴图路径、UV 重复都参与生成最终的
/// `StandardMaterial`。少登记任何一项，就会出现"同一份材质挂在两种实体上、
/// 后者静默继承前者的渲染模式"这种只在特定组合下复现的 bug。
/// 键的构造与 [`材质转标准材质`] 的入参由同一个函数负责，两者不会走散。
///
/// 值 `None` 表示"读过但失败了"（文件缺失、RON 坏了、版本过新）。
/// 记下失败与记下成功同样重要：否则每个实体、每次重建都要重新撞一次
/// 同一块坏磁盘，还会把警告刷成噪声。
#[derive(Resource, Default)]
pub struct 材质缓存 {
    条目: std::collections::HashMap<String, Option<Handle<StandardMaterial>>>,
    本轮使用: Vec<String>,
}

impl 材质缓存 {
    /// 一次转换的缓存键。
    ///
    /// 参数顺序与 [`材质转标准材质`] 一致，只是把"材质自带的那几张图"省掉了：
    /// 四个贴图槽引用哪张图都是 `材质路径 + 代次` 的函数，代次一变就会落到
    /// 新键上，因此不需要为它们在键里另开一栏。
    ///
    /// UV 重复是 `f32`，进键时取**位模式**而不是 `{:?}`：调试格式会把同一个值
    /// 打成不同的短字符串，那正好制造出"两个相同的值算出两个键"的缓存击穿。
    #[allow(clippy::too_many_arguments)]
    pub fn 键(
        材质路径: &str,
        代次: u64,
        受光: bool,
        双面: bool,
        混合: bool,
        实体贴图路径: &str,
        贴图重复: Option<(f32, f32)>,
    ) -> String {
        let 重复 = 贴图重复
            .map(|(u, v)| format!("{:x}-{:x}", u.to_bits(), v.to_bits()))
            .unwrap_or_else(|| "无".to_string());
        format!("{材质路径}|{代次}|{受光}|{双面}|{混合}|{实体贴图路径}|{重复}")
    }

    /// 命中时顺手把键登记为"本轮在用"，未命中返回 `None` 让调用方去读盘。
    ///
    /// 双层 `Option`：外层"这个键试过没有"，内层"试过的结果是句柄还是失败"。
    /// 合成一层就把"没试过"和"试过且坏了"混成同一件事，
    /// 于是每次重建都要重新读一次坏文件、重新警告一次。
    pub fn 取(&mut self, 键: &str) -> Option<Option<Handle<StandardMaterial>>> {
        let 已试 = self.条目.get(键).cloned()?;
        self.本轮使用.push(键.to_string());
        Some(已试)
    }

    /// 记录一次转换结果（`None` 表示失败，也要记）。
    pub fn 记录(&mut self, 键: String, 句柄: Option<Handle<StandardMaterial>>) {
        self.本轮使用.push(键.clone());
        self.条目.insert(键, 句柄);
    }

    /// 丢弃本轮没有被引用的条目。
    ///
    /// 在 `同步预览实体` 末尾调用一次。删掉实体、换材质路径、代次推进
    /// 留下的旧条目都在这里回收 —— 强句柄不放掉，材质资产就永远留在
    /// `Assets<StandardMaterial>` 里。
    pub fn 清理本轮(&mut self) {
        let 使用 = std::mem::take(&mut self.本轮使用);
        let 保留: std::collections::HashSet<String> = 使用.into_iter().collect();
        self.条目.retain(|键, _| 保留.contains(键));
    }

    /// 已缓存的条目数（供测试与诊断）。
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

/// 2D 叠放步长：把文档遍历序号折算成精灵局部 z 的微小偏移。
///
/// 正交相机下 z 不改变屏幕位置，只决定精灵的绘制先后。步长远小于任何
/// 可见几何（1000 个实体累计 0.1 世界单位），因此"文档 z 相同时，
/// 场景树里靠后的画在上面"成为确定事实，而不是依赖排序实现的偶然行为。
pub const 二维叠放步长: f32 = 0.0001;

/// 文档遍历序号 → 2D 精灵的局部 z。
pub fn 二维叠放偏移(序号: u32) -> f32 {
    序号 as f32 * 二维叠放步长
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

    let 模型 = 属性
        .get("模型")
        .and_then(属性值::取资源)
        .map(|引用| 引用.路径.文本().to_string());

    let 材质 = 属性
        .get("材质")
        .and_then(属性值::取资源)
        .map(|引用| 引用.路径.文本().to_string());

    精灵视觉参数 {
        贴图路径: 贴图,
        颜色,
        自定义尺寸,
        水平翻转,
        垂直翻转,
        模型路径: 模型,
        材质路径: 材质,
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
    /// glTF/GLB 模型的项目相对路径（与 `贴图路径` 互斥）。
    pub 模型路径: Option<String>,
    /// 材质文档（`.material.ron`）的项目相对路径。
    pub 材质路径: Option<String>,
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

    for 标识 in 文档.文档遍历() {
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

/// 一个 `TileTerrain` 实体应当产生的渲染项。
///
/// 与 [`预览渲染项`] 分开，因为它的"贴图"不是一条资源路径，而是
/// 加载时按地形数据程序化合成的纹理。把合成留给 `同步预览实体`，
/// 投影本身仍然保持纯函数、可单测。
#[derive(Debug, Clone, PartialEq)]
pub struct 地形渲染项 {
    /// 对应的文档实体标识。
    pub 来源: 实体标识,
    /// 该实体的变换。
    pub 变换: Transform,
    /// 地形参数（数据路径、图集目录、烘焙参数）。
    pub 参数: 地形参数,
    /// 世界显示尺寸。
    pub 显示尺寸: (f32, f32),
}

/// 一个 `Primitive3d` 实体应当产生的渲染项。
#[derive(Debug, Clone, PartialEq)]
pub struct 几何渲染项 {
    /// 对应的文档实体标识。
    pub 来源: 实体标识,
    /// 该实体的变换。
    pub 变换: Transform,
    /// 几何形状。
    pub 形状: 几何体形状,
    /// 世界尺寸。
    pub 尺寸: (f32, f32, f32),
    /// 贴图路径（项目相对）。
    pub 贴图路径: Option<String>,
    /// 材质文档路径（项目相对，`.material.ron`）。
    pub 材质路径: Option<String>,
    /// 基础色。
    pub 颜色: 颜色值,
    /// UV 重复。
    pub 贴图重复: (f32, f32),
    /// 是否受光。
    pub 受光: bool,
    /// 是否双面。
    pub 双面: bool,
    /// 二维视口里的代替尺寸（顶视图/正视图）。
    pub 二维尺寸: (f32, f32),
}

/// 从 `TileTerrain` 组件投影地形渲染项。
pub fn 投影地形项(文档: &场景文档) -> Vec<地形渲染项> {
    let mut 结果 = Vec::new();
    for 标识 in 文档.文档遍历() {
        let Some(实体) = 文档.取(标识) else {
            continue;
        };
        if 实体.编辑器隐藏 {
            continue;
        }
        let Some(变换) = 实体.变换 else {
            continue;
        };
        let Some(组件) = 实体.取组件("TileTerrain") else {
            continue;
        };
        let 参数 = 地形参数::解析(&组件.属性);
        let 显示尺寸 = 参数.显示尺寸;
        结果.push(地形渲染项 {
            来源: 标识,
            变换: 变换转预览(变换),
            参数,
            显示尺寸,
        });
    }
    结果
}

/// 从 `Primitive3d` 组件投影几何渲染项。
pub fn 投影几何项(文档: &场景文档) -> Vec<几何渲染项> {
    let mut 结果 = Vec::new();
    for 标识 in 文档.文档遍历() {
        let Some(实体) = 文档.取(标识) else {
            continue;
        };
        if 实体.编辑器隐藏 {
            continue;
        }
        let Some(变换) = 实体.变换 else {
            continue;
        };
        let Some(组件) = 实体.取组件("Primitive3d") else {
            continue;
        };
        let 属性 = &组件.属性;

        let 形状 = match 属性.get("形状") {
            Some(属性值::枚举(枚举)) => 几何体形状::由值(&枚举.变体名, 枚举.序号),
            _ => 几何体形状::立方体,
        };
        let (x, y, z) = 属性
            .get("尺寸")
            .and_then(属性值::取三维)
            .map(|(x, y, z)| (x.abs().max(0.001), y.abs().max(0.001), z.abs().max(0.001)))
            .unwrap_or((1.0, 1.0, 1.0));
        let 贴图路径 = 属性
            .get("贴图")
            .and_then(属性值::取资源)
            .map(|引用| 引用.路径.文本().to_string());
        let 材质路径 = 属性
            .get("材质")
            .and_then(属性值::取资源)
            .map(|引用| 引用.路径.文本().to_string());
        let 颜色 = 属性
            .get("颜色")
            .and_then(属性值::取颜色)
            .unwrap_or(颜色值::白());
        let 贴图重复 = 属性
            .get("贴图重复")
            .and_then(属性值::取二维)
            .map(|(u, v)| (u.abs().max(0.0), v.abs().max(0.0)))
            .unwrap_or((1.0, 1.0));
        let 受光 = 属性.get("受光").and_then(属性值::取布尔).unwrap_or(true);
        let 双面 = 属性.get("双面").and_then(属性值::取布尔).unwrap_or(false);

        // 2D 视口里没有"厚度"概念：地面取 (x,z) 的顶视图，
        // 其余立体形状取 (x,y) 的正视图，保证两种主要用法都能看见。
        let 二维尺寸 = match 形状 {
            几何体形状::平面 => (x, z),
            几何体形状::立方体 | 几何体形状::球体 | 几何体形状::圆柱体 => {
                if z > y {
                    (x, z)
                } else {
                    (x, y)
                }
            }
        };

        结果.push(几何渲染项 {
            来源: 标识,
            变换: 变换转预览(变换),
            形状,
            尺寸: (x, y, z),
            贴图路径,
            材质路径,
            颜色,
            贴图重复,
            受光,
            双面,
            二维尺寸,
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
    命令.insert_resource(地形缓存::default());
    命令.insert_resource(材质缓存::default());
    命令.insert_resource(地形文件监视::default());
    命令.insert_resource(预览同步状态::default());
    // 三维实体用同一个 1×1 四边形，按实体显示尺寸缩放：
    // 一个网格句柄服务全部实体，比每个实体建一个网格省得多。
    命令.insert_resource(三维预览资源 {
        四边形: 网格资源.add(Rectangle::new(1.0, 1.0)),
    });
    命令.insert_resource(三维预览状态::default());

    // 三维预览的照明：模型材质是受光的（与 unlit 的贴图四边形不同），
    // 没有光会渲染成纯黑。方向光负责形体，环境光保证背光面可见。
    //
    // 开启阴影：3D 示例的地面/围墙现在是受光的实体网格，没有阴影时
    // 道具看起来像浮在地上；这是"程序化几何体"与"纸片"最直观的差别之一。
    命令.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(6.0, 12.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    命令.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 600.0,
        ..default()
    });

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
    编辑器: Res<crate::plugin::EditorStateHolder>,
) {
    // 渲染目标的**宽高比必须等于项目的设计分辨率**，尺寸则贴合面板（受上限约束）。
    //
    // 早先这里直接用面板像素尺寸：面板 910×531、设计稿 1920×1080 时，
    // 投影与纹理都按面板比例，"看起来没拉伸"；但设计稿比例一旦与面板不同，
    // 界面就会被拉变形。现在把"设计比例"作为唯一真相：面板只决定渲染多少
    // 像素（清晰度），显示时再按比例 letterbox。
    let (设计宽, 设计高) = 编辑器
        .状态
        .项目
        .as_ref()
        .map(|项目| 项目.清单().设计分辨率)
        .unwrap_or(editor_core::project::默认设计分辨率());
    let 期望 = editor_core::ui_layout::适配渲染尺寸(
        设计宽 as f32,
        设计高 as f32,
        界面.状态.视口尺寸.x,
        界面.状态.视口尺寸.y,
        预览尺寸上限,
    );
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
    // 可见世界矩形由 `视口相机` 内部按**设计帧**换算（见 `视口相机::画布尺寸`），
    // 这里仍然把面板尺寸传进去：面板尺寸只用于 letterbox，不参与世界尺度。
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

/// 为文档里的每个实体建立预览层级节点，并按文档父子关系接线。
///
/// # 为什么每个实体都要一个节点
///
/// 一个文档实体可能同时产生精灵与三维四边形：二者渲染层不同、局部缩放
/// 也不同，不能挂在同一个实体上。若让视觉实体直接互为父子，就必须按
/// "同一渲染层"挑父节点，而父实体没有对应视觉时世界变换会直接丢失。
///
/// 节点只承载文档变换，视觉实体挂在它下面、局部变换只表达自身尺寸差异。
/// 父子传播因此始终由 `GlobalTransform` 一处完成，不需要任何手写矩阵累加。
///
/// 隐藏实体的节点也保留：父实体被隐藏时，子实体仍要落在正确的世界位置。
///
/// 返回 文档实体 → 预览节点。顺序与 [`场景文档::文档遍历`] 一致。
pub fn 构建预览层级(
    命令: &mut Commands,
    文档: &场景文档,
) -> std::collections::HashMap<实体标识, Entity> {
    let 顺序 = 文档.文档遍历();
    let mut 节点表 = std::collections::HashMap::with_capacity(顺序.len());
    for 标识 in &顺序 {
        let 变换 = 文档
            .取(*标识)
            .and_then(|实体| 实体.变换)
            .map(变换转预览)
            .unwrap_or_default();
        let 节点 = 命令.spawn((预览实体, 预览来源(*标识), 变换)).id();
        节点表.insert(*标识, 节点);
    }
    // 第二趟才能接线：父节点必须已经存在。文档顺序本身是父先于子，
    // 但用第二趟表达"父必须已建"这条约束，不依赖遍历实现细节。
    for 标识 in &顺序 {
        let Some(父) = 文档.取(*标识).and_then(|实体| 实体.父实体) else {
            continue;
        };
        let (Some(子节点), Some(父节点)) = (节点表.get(标识), 节点表.get(&父))
        else {
            continue;
        };
        命令.entity(*子节点).insert(ChildOf(*父节点));
    }
    节点表
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
/// # 为什么还要比资源注册表代次
///
/// 材质与贴图是**项目资源**，内容不在文档里：改一份 `.material.ron`
/// 不会推进文档修订号。只比修订号的话，用户改完材质必须再动一下场景
/// 才能看到视口更新 —— 那正是"改了没反应"最难复现的 bug 的形状。
/// 代次在注册表刷新/导入/清空时推进，比较它同样是 O(1)。
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
///
/// # 材质只换表面，不换渲染模式
///
/// 引用了 `.material.ron` 的实体走私有函数 `解析材质句柄`，两条三维分支都换：
///
/// | 分支 | 材质提供的 | 实体保留的 |
/// | --- | --- | --- |
/// | 精灵的三维对应物（四边形） | 着色颜色、贴图、粗糙度、金属度、自发光 | `unlit`、`Blend`、不剔除 |
/// | 程序化几何体（网格） | 同上，外加"材质要求双面就不剔除" | `受光` → `unlit`、UV 重复、网格尺寸 |
///
/// 精灵四边形那三项**不交给材质**：它是"3D 视图里的精灵必须与 2D 视图
/// 长得一样"这条约定的前提，材质把它改成受光就会出现两个视图不一致，
/// 那是最难查的一类 bug。完整的字段归属表见 [`材质转标准材质`]。
#[allow(clippy::too_many_arguments)]
pub fn 同步预览实体(
    mut 命令: Commands,
    编辑器: Res<EditorStateHolder>,
    mut 同步状态: ResMut<预览同步状态>,
    已有: Query<(Entity, Option<&ChildOf>), With<预览实体>>,
    mut 贴图缓存: ResMut<贴图缓存>,
    mut 地形缓存: ResMut<地形缓存>,
    mut 材质缓存: ResMut<材质缓存>,
    三维资源: Res<三维预览资源>,
    mut 材质表: ResMut<Assets<StandardMaterial>>,
    mut 网格资源: ResMut<Assets<Mesh>>,
    mut 图片资源: ResMut<Assets<Image>>,
    资源服务器: Res<AssetServer>,
) {
    let 修订 = 编辑器.状态.文档修订();
    let 代次 = 编辑器.状态.资源注册表.代次();
    if 同步状态.已同步修订 == Some(修订) && 同步状态.已同步代次 == Some(代次)
    {
        return;
    }

    // 先清掉旧的预览实体。只摘根节点：`despawn` 会连同子树一起回收，
    // 再对已经被连带删除的子节点调用一次会打出一串"实体不存在"的警告。
    for (实体, 父) in &已有 {
        if 父.is_none() {
            命令.entity(实体).despawn();
        }
    }

    let 文档 = &编辑器.状态.会话.文档;
    let 项目根 = 编辑器
        .状态
        .项目
        .as_ref()
        .map(|项目| 项目.根目录().to_path_buf());

    // 投影判断全部在纯函数里完成，这里只负责把结果变成 ECS 实体。
    let 待渲染 = 投影预览项(文档);
    let 地形项 = 投影地形项(文档);
    let 几何项 = 投影几何项(文档);

    // 层级节点先建：视觉实体全部挂在节点下面，
    // 世界变换只由 `TransformPlugin` 的 `GlobalTransform` 传播产生。
    let 节点表 = 构建预览层级(&mut 命令, 文档);
    let 叠放序号 = 文档.叠放序号表();

    for 项 in &待渲染 {
        let Some(节点) = 节点表.get(&项.来源).copied() else {
            continue;
        };
        // 模型实体：交给 Bevy 的 glTF 场景加载器，父节点承载文档变换。
        if let Some(模型路径) = 项.视觉.模型路径.as_deref()
            && let Some(根) = 项目根.as_deref()
        {
            let 绝对 = 根.join(模型路径.replace('/', std::path::MAIN_SEPARATOR_STR));
            // Bevy 0.20 的 glTF 场景产物是 `WorldAsset`（`WorldAssetRoot` 挂载），
            // 不再使用旧的 `Scene`/`SceneRoot`。
            // glTF 的首个场景是子资源，标签固定为 `Scene0`；
            // 不带标签会报 "Could not find an asset loader matching"。
            let 场景 = 资源服务器.load::<bevy::world_serialization::WorldAsset>(
                bevy::asset::AssetPath::from_path_buf(绝对).with_label("Scene0"),
            );
            命令
                .spawn((
                    预览来源(项.来源),
                    bevy::world_serialization::WorldAssetRoot(场景),
                    Transform::default(),
                    RenderLayers::layer(三维渲染层),
                ))
                .insert(ChildOf(节点));
            continue;
        }

        let 图片 = 解析贴图句柄(
            项.视觉.贴图路径.as_deref(),
            项目根.as_deref(),
            &mut 贴图缓存,
            &mut 图片资源,
        );

        // 二维精灵只表达尺寸与贴图：位置/旋转/缩放来自父节点。
        // 局部 z 带上文档遍历序号，让同 z 时的叠放顺序等于场景树顺序。
        命令
            .spawn((
                预览来源(项.来源),
                Sprite {
                    image: 图片.clone(),
                    color: 颜色转_bevy(项.视觉.颜色),
                    flip_x: 项.视觉.水平翻转,
                    flip_y: 项.视觉.垂直翻转,
                    custom_size: Some(Vec2::new(项.显示尺寸.0, 项.显示尺寸.1)),
                    ..default()
                },
                Transform::from_translation(Vec3::new(
                    0.0,
                    0.0,
                    叠放序号
                        .get(&项.来源)
                        .copied()
                        .map(二维叠放偏移)
                        .unwrap_or(0.0),
                )),
                RenderLayers::layer(预览渲染层),
            ))
            .insert(ChildOf(节点));

        // ---- 三维对应物 ----
        //
        // 精灵在 3D 相机下不可见（Bevy 的精灵是 2D 管线），因此每个实体
        // 额外生成一个"贴了同一张图的四边形"，放在独立渲染层上由 Camera3d 渲染。
        // 它也挂在同一个节点下，只把显示尺寸乘进局部缩放，
        // 3D 视图里的世界位置因此与 2D 视图完全一致 —— 不是另算一份。
        let 引用材质 = 解析材质句柄(
            项.视觉.材质路径.as_deref(),
            项.视觉.贴图路径.as_deref(),
            false,
            true,
            true,
            None,
            &编辑器.状态,
            &mut 材质缓存,
            &mut 贴图缓存,
            &mut 图片资源,
            &mut 材质表,
        );
        let 材质句柄 = 引用材质.unwrap_or_else(|| {
            let mut 材质 = StandardMaterial {
                base_color: 颜色转_bevy(项.视觉.颜色),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                // 双面可见：编辑器经常从背面看一个精灵。
                cull_mode: None,
                ..default()
            };
            if 图片.is_strong() {
                材质.base_color_texture = Some(图片.clone());
            }
            // 没有贴图时保持纯色方块，而不是显示一张 1×1 的占位图。
            材质表.add(材质)
        });
        命令
            .spawn((
                预览来源(项.来源),
                Mesh3d(三维资源.四边形.clone()),
                MeshMaterial3d(材质句柄),
                Transform::from_scale(Vec3::new(项.显示尺寸.0, 项.显示尺寸.1, 1.0)),
                RenderLayers::layer(三维渲染层),
            ))
            .insert(ChildOf(节点));
    }

    // ---- 程序化瓦片地形 ----
    //
    // 贴图不是项目里的 PNG，而是按 `TileTerrain` 的参数与字符网格在加载时
    // 走双网格合成出来的。缓存键包含文件 mtime，数据或图集一改就重烘。
    if let Some(根) = 项目根.as_deref() {
        let mut 使用键 = Vec::with_capacity(地形项.len());
        for 项 in &地形项 {
            使用键.push(地形缓存::键(根, &项.参数));
            let 图片 = 地形缓存.取或烘焙(根, &项.参数, &mut 图片资源);
            let Some(节点) = 节点表.get(&项.来源).copied() else {
                continue;
            };

            // 二维视口：整张地形当作一个精灵，尺寸就是文档里的显示尺寸。
            命令
                .spawn((
                    预览来源(项.来源),
                    Sprite {
                        image: 图片.clone(),
                        custom_size: Some(Vec2::new(项.显示尺寸.0, 项.显示尺寸.1)),
                        ..default()
                    },
                    Transform::from_translation(Vec3::new(
                        0.0,
                        0.0,
                        叠放序号
                            .get(&项.来源)
                            .copied()
                            .map(二维叠放偏移)
                            .unwrap_or(0.0),
                    )),
                    RenderLayers::layer(预览渲染层),
                ))
                .insert(ChildOf(节点));

            // 三维视口：同一张纹理的四边形（2D 场景在 3D 模式下仍然是平面）。
            let 材质 = StandardMaterial {
                base_color_texture: Some(图片),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                ..default()
            };
            let 材质句柄 = 材质表.add(材质);
            命令
                .spawn((
                    预览来源(项.来源),
                    Mesh3d(三维资源.四边形.clone()),
                    MeshMaterial3d(材质句柄),
                    Transform::from_scale(Vec3::new(项.显示尺寸.0, 项.显示尺寸.1, 1.0)),
                    RenderLayers::layer(三维渲染层),
                ))
                .insert(ChildOf(节点));
        }
        // 删掉/改路径的地形不能把 25MB 纹理永久留在 `Assets<Image>` 里。
        地形缓存.清理(&使用键, &mut 图片资源);
    } else {
        地形缓存.清理(&[], &mut 图片资源);
    }

    // ---- 程序化三维几何体（地面、围墙、箱体）----
    //
    // 这里不再用 unlit 四边形：按文档里的形状/尺寸生成真正的 `Mesh3d`，
    // 材质是受光的 `StandardMaterial`，UV 按 `贴图重复` 缩放。
    // 3D 示例的地面与四面墙就是由这一条路径渲染出来的。
    for 项 in &几何项 {
        let Some(节点) = 节点表.get(&项.来源).copied() else {
            continue;
        };
        let 图片 = 解析贴图句柄(
            项.贴图路径.as_deref(),
            项目根.as_deref(),
            &mut 贴图缓存,
            &mut 图片资源,
        );

        // 二维视口里的代替表示：地面看顶视图、墙看正视图。
        命令
            .spawn((
                预览来源(项.来源),
                Sprite {
                    image: 图片.clone(),
                    color: 颜色转_bevy(项.颜色),
                    custom_size: Some(Vec2::new(项.二维尺寸.0, 项.二维尺寸.1)),
                    ..default()
                },
                Transform::from_translation(Vec3::new(
                    0.0,
                    0.0,
                    叠放序号
                        .get(&项.来源)
                        .copied()
                        .map(二维叠放偏移)
                        .unwrap_or(0.0),
                )),
                RenderLayers::layer(预览渲染层),
            ))
            .insert(ChildOf(节点));

        let 网格句柄 = match 项.形状 {
            几何体形状::立方体 => {
                网格资源.add(Cuboid::new(项.尺寸.0, 项.尺寸.1, 项.尺寸.2))
            }
            几何体形状::平面 => 网格资源.add(
                Plane3d::new(Vec3::Y, Vec2::new(项.尺寸.0 * 0.5, 项.尺寸.2 * 0.5))
                    .mesh()
                    .build(),
            ),
            // 球体与圆柱体的"尺寸"是直径：Inspector 里填 2 表示直径 2，
            // 与立方体的"全长"语义保持一致（都是外接尺寸，不是半径）。
            几何体形状::球体 => {
                let 半径 = 项.尺寸.0.max(项.尺寸.1).max(项.尺寸.2) * 0.5;
                网格资源.add(Sphere::new(半径).mesh().uv(32, 18))
            }
            几何体形状::圆柱体 => {
                网格资源.add(Cylinder::new(项.尺寸.0 * 0.5, 项.尺寸.1).mesh().build())
            }
        };

        let 引用材质 = 解析材质句柄(
            项.材质路径.as_deref(),
            项.贴图路径.as_deref(),
            项.受光,
            项.双面,
            false,
            Some(项.贴图重复),
            &编辑器.状态,
            &mut 材质缓存,
            &mut 贴图缓存,
            &mut 图片资源,
            &mut 材质表,
        );
        let 材质句柄 = 引用材质.unwrap_or_else(|| {
            let mut 材质 = StandardMaterial {
                base_color: 颜色转_bevy(项.颜色),
                unlit: !项.受光,
                alpha_mode: AlphaMode::Opaque,
                cull_mode: if 项.双面 { None } else { Some(Face::Back) },
                uv_transform: Affine2::from_scale(Vec2::new(项.贴图重复.0, 项.贴图重复.1)),
                perceptual_roughness: 0.9,
                ..default()
            };
            if 图片.is_strong() {
                材质.base_color_texture = Some(图片.clone());
            }
            材质表.add(材质)
        });

        命令
            .spawn((
                预览来源(项.来源),
                Mesh3d(网格句柄),
                MeshMaterial3d(材质句柄),
                Transform::default(),
                RenderLayers::layer(三维渲染层),
            ))
            .insert(ChildOf(节点));
    }

    材质缓存.清理本轮();

    同步状态.已同步修订 = Some(修订);
    同步状态.已同步代次 = Some(代次);
    同步状态.预览实体数 = 待渲染.len() + 地形项.len() + 几何项.len();
    // 每次属性修改都会走到这里；用 debug 级别，正常使用不刷屏。
    debug!(
        "预览同步完成：Sprite {} / 地形 {} / 几何体 {}",
        待渲染.len(),
        地形项.len(),
        几何项.len()
    );
}

/// 地形文件监视：外部编辑 `.terrain.ron` 后，文档修订号不会变，
/// 但预览应该刷新。
///
/// # 为什么需要它
///
/// 地形数据是**文本**，工作流是"在编辑器外改字符 → 回编辑器看结果"。
/// 如果只在文档修订号变化时同步，改完文件必须再动一下场景才会重烘。
/// 这里每秒查一次 mtime，变了就把同步状态置为未同步，下一帧按新数据重烘；
/// 纹理缓存自己也会比对 mtime，不会做无用的重复烘焙。
#[derive(Resource)]
pub struct 地形文件监视 {
    记录: std::collections::HashMap<实体标识, Option<std::time::SystemTime>>,
    累计: f32,
    首次: bool,
}

impl Default for 地形文件监视 {
    fn default() -> Self {
        Self {
            记录: std::collections::HashMap::new(),
            累计: 0.0,
            // 首次只建立基线，不因为"记录表是空的"就误报变化。
            首次: true,
        }
    }
}

/// 每秒检查一次地形数据文件是否被外部修改。
pub fn 监视地形文件(
    mut 监视: ResMut<地形文件监视>,
    mut 同步: ResMut<预览同步状态>,
    编辑器: Res<EditorStateHolder>,
    time: Res<Time>,
) {
    监视.累计 += time.delta_secs();
    if 监视.累计 < 1.0 {
        return;
    }
    监视.累计 = 0.0;

    let Some(项目根) = 编辑器
        .状态
        .项目
        .as_ref()
        .map(|项目| 项目.根目录().to_path_buf())
    else {
        return;
    };

    let mut 变化 = false;
    let mut 新记录 = std::collections::HashMap::new();
    for 实体 in 编辑器.状态.会话.文档.实体表.values() {
        let Some(组件) = 实体.取组件("TileTerrain") else {
            continue;
        };
        let 参数 = 地形参数::解析(&组件.属性);
        if 参数.地形文件.trim().is_empty() {
            continue;
        }
        let 路径 = 项目根.join(参数.地形文件.replace('/', std::path::MAIN_SEPARATOR_STR));
        let 时间 = std::fs::metadata(&路径)
            .ok()
            .and_then(|元| 元.modified().ok());
        if !监视.首次 && 监视.记录.get(&实体.标识) != Some(&时间) {
            变化 = true;
            info!("[地形] 检测到 {} 变化，刷新预览", 参数.地形文件);
        }
        新记录.insert(实体.标识, 时间);
    }

    监视.记录 = 新记录;
    if 监视.首次 {
        info!("[地形] 文件监视已建立基线：{} 个地形实体", 监视.记录.len());
    }
    监视.首次 = false;
    if 变化 {
        同步.已同步修订 = None;
    }
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
                    // glTF 场景的子实体不会继承父实体的 `RenderLayers`，
                    // 它们落在默认层 0；因此 3D 相机必须同时看 0 层与三维层，
                    // 否则网格模型永远不显示（而贴图四边形是三维层，能显示）。
                    RenderLayers::from_layers(&[0, 三维渲染层]),
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

/// 模型实例化诊断（仅在 `BEVY_STUDIO_MODEL_DIAG=1` 时输出）。
///
/// 它回答一个具体问题：模型实体到底是"资产没加载完"、"加载了但没实例化"，
/// 还是"实例化成功但画面里看不见"。没有这层观测就只能猜相机和尺度。
pub fn 模型诊断(
    资源服务器: Res<AssetServer>,
    根: Query<(
        Entity,
        &bevy::world_serialization::WorldAssetRoot,
        &Transform,
    )>,
    子: Query<&Children>,
    网格: Query<&Mesh3d>,
) {
    if std::env::var("BEVY_STUDIO_MODEL_DIAG").is_err() {
        return;
    }
    for (实体, 根组件, 变换) in &根 {
        let 状态 = 资源服务器.get_load_state(根组件.0.id());
        let 子数 = 子.get(实体).map(|列表| 列表.len()).unwrap_or(0);
        let 网格数 = 子
            .get(实体)
            .map(|列表| {
                列表
                    .iter()
                    .filter(|子实体| 网格.get(*子实体).is_ok())
                    .count()
            })
            .unwrap_or(0);
        info!(
            "[模型诊断] 路径 {:?} 加载状态 {状态:?} 位置 {:?} 直接子实体 {子数} 网格子实体 {网格数}",
            根组件.0.path(),
            变换.translation
        );
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
    图片资源: &mut Assets<Image>,
) -> Handle<Image> {
    let (Some(路径), Some(根)) = (路径, 项目根) else {
        return Handle::default();
    };

    if let Some(句柄) = 缓存.取(路径) {
        return 句柄.clone();
    }

    // 场景里的路径是项目相对的（例如 `assets/2d_demo/player.png`）。
    //
    // # 为什么不用 `AssetServer::load`
    //
    // Bevy 0.20 的资产服务器会拒绝加载"不在资产根目录下"的绝对路径
    // （日志里是 `is unapproved`），而编辑器的资产根在启动时固定，
    // 不可能随"打开的项目"切换。实测中这表现为**所有贴图都渲染成白方块**。
    //
    // 因此这里自己读文件、自己解码、直接插进 `Assets<Image>`：
    // 路径与项目根都由编辑器掌控，不经过资产服务器的路径审核。
    // 代价是同步 IO —— 贴图都是几 KB 的 PNG，编辑器量级完全可接受。
    let 绝对 = 根.join(路径.replace('/', std::path::MAIN_SEPARATOR_STR));
    let 句柄 = match std::fs::read(&绝对) {
        Ok(字节) => match image::load_from_memory(&字节) {
            Ok(图片) => {
                let rgba = 图片.to_rgba8();
                let 尺寸 = Extent3d {
                    width: rgba.width(),
                    height: rgba.height(),
                    ..default()
                };
                let 图像 = Image::new(
                    尺寸,
                    TextureDimension::D2,
                    rgba.into_raw(),
                    TextureFormat::Rgba8UnormSrgb,
                    bevy::asset::RenderAssetUsages::default(),
                );
                Some(图片资源.add(图像))
            }
            Err(错误) => {
                warn!("贴图解码失败 {}：{错误}", 绝对.display());
                None
            }
        },
        Err(_) => None,
    };
    缓存.记录(路径.to_string(), 句柄.clone());
    句柄.unwrap_or_default()
}

/// 解析可选的多通道贴图；没引用或读不出来都返回 `None`。
///
/// 与基础色不同，这里**不留** 1×1 占位图：[`解析贴图句柄`] 失败时返回
/// [`Handle::default`]，把它当法线贴图会让表面朝一个固定方向偏，当金属粗糙度
/// 贴图会把整块表面变成全粗糙金属 —— 比"这个通道没启用"更糟。降级成不挂该
/// 通道，画面回到没有多通道贴图时的样子。
fn 解析可选贴图(
    引用: Option<&资源引用值>,
    根: &std::path::Path,
    缓存: &mut 贴图缓存,
    图片资源: &mut Assets<Image>,
) -> Option<Handle<Image>> {
    let 路径 = 引用?.路径.文本();
    let 句柄 = 解析贴图句柄(Some(路径), Some(根), 缓存, 图片资源);
    (句柄 != Handle::default()).then_some(句柄)
}

/// 解析材质引用 → 转换好的 `StandardMaterial` 句柄，带代次缓存。
///
/// # 降级契约
///
/// 返回 `None` 只发生在「没打开项目」「路径不合法」「读盘或解码失败」
/// 这三种情况，后两种各留一条 `warn` —— 与 [`crate::tile_terrain::地形缓存`]
/// 同一口径：材质坏了不能让整个实体消失，也不能静默。
/// 调用方拿 `None` 时**沿用没有材质时的那份材质**，也就是引入本函数之前的
/// 渲染结果：用户的场景不会因为一个坏引用而变黑或变白。
///
/// 失败也要写进缓存（[`材质缓存::记录`] 传 `None`）：否则每个挂了这个坏路径的
/// 实体、每次重建都要重新撞一次同一块坏磁盘，并把警告刷成噪声。
///
/// # 材质自己的贴图走同一条贴图通道
///
/// 材质带贴图时盖掉实体的 `贴图` 字段，没带时沿用实体的；法线 / 金属粗糙度 /
/// 遮蔽三张图只认材质自己的（实体没有对应字段可沿用）。四张图都经过
/// [`解析贴图句柄`]，所以材质与实体共用同一份图片缓存 ——
/// 同一张 PNG 挂在二十个实体上仍然只解码一次。
///
/// 三张多通道贴图走 [`解析可选贴图`]：读不出来时该通道不挂，而不是塞一张
/// 1×1 占位图进去（理由见那个函数）。
#[allow(clippy::too_many_arguments)]
fn 解析材质句柄(
    材质路径: Option<&str>,
    实体贴图路径: Option<&str>,
    受光: bool,
    双面: bool,
    混合: bool,
    贴图重复: Option<(f32, f32)>,
    编辑器: &editor_core::state::编辑器状态,
    缓存: &mut 材质缓存,
    贴图缓存: &mut 贴图缓存,
    图片资源: &mut Assets<Image>,
    材质表: &mut Assets<StandardMaterial>,
) -> Option<Handle<StandardMaterial>> {
    let 路径 = 材质路径?;
    let 项目 = 编辑器.项目.as_ref()?;
    let 键 = 材质缓存::键(
        路径,
        编辑器.资源注册表.代次(),
        受光,
        双面,
        混合,
        实体贴图路径.unwrap_or("无贴图"),
        贴图重复,
    );
    if let Some(已试) = 缓存.取(&键) {
        return 已试;
    }

    let 解析后 = match scene_doc::path::项目相对路径::解析(路径) {
        Ok(解析后) => 解析后,
        Err(_) => {
            warn!("[材质] {路径} 不是合法的项目相对路径；该实体按无材质渲染");
            缓存.记录(键, None);
            return None;
        }
    };
    let 材质 = match 项目.读取材质(&解析后) {
        Ok(材质) => 材质,
        Err(错误) => {
            warn!("[材质] 读取 {路径} 失败：{错误}；该实体按无材质渲染");
            缓存.记录(键, None);
            return None;
        }
    };

    let 根 = 项目.根目录();
    let 自带贴图 = 材质.贴图.as_ref().map(|引用| 引用.路径.文本().to_string());
    let 贴图 = 材质贴图句柄 {
        基础色: 解析贴图句柄(
            自带贴图.as_deref().or(实体贴图路径),
            Some(根),
            贴图缓存,
            图片资源,
        ),
        法线: 解析可选贴图(材质.法线贴图.as_ref(), 根, 贴图缓存, 图片资源),
        金属粗糙度: 解析可选贴图(材质.金属粗糙度贴图.as_ref(), 根, 贴图缓存, 图片资源),
        遮蔽: 解析可选贴图(材质.遮蔽贴图.as_ref(), 根, 贴图缓存, 图片资源),
    };
    let 句柄 = 材质表.add(材质转标准材质(
        &材质,
        贴图,
        受光,
        双面,
        混合,
        贴图重复,
    ));
    缓存.记录(键, Some(句柄.clone()));
    Some(句柄)
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
            模型路径: None,
            材质路径: None,
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

    /// 投影顺序必须与规范化遍历一致，便于与场景树对照排查。
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
        assert_eq!(来源, 文档.文档遍历());
    }

    /// 2D 叠放必须等于文档遍历顺序：父在前、兄弟按 `次序`，与生成顺序无关。
    #[test]
    fn 二维叠放顺序与次序投影一致() {
        let mut 文档 = 场景文档::新建("叠放");
        let 甲 = 文档.生成实体("甲", None);
        let 乙 = 文档.生成实体("乙", None);
        let 丙 = 文档.生成实体("丙", None);
        let 子的 = 文档.生成实体("乙的子", Some(乙));
        // 交换乙/丙的次序：叠放必须跟着 `次序` 走，而不是生成顺序。
        for (标识, 次序) in [(乙, 2), (丙, 1)] {
            if let Some(实体) = 文档.取可变(标识) {
                实体.次序 = 次序;
            }
        }
        for 标识 in [甲, 乙, 丙, 子的] {
            if let Some(实体) = 文档.取可变(标识) {
                实体.加组件(组件数据::新建("Sprite", Default::default()));
            }
        }

        let 遍历 = 文档.文档遍历();
        assert_eq!(遍历, vec![甲, 丙, 乙, 子的], "父在前、兄弟按 `次序`");

        let 序号 = 文档.叠放序号表();
        let 偏移: Vec<f32> = 遍历.iter().map(|标识| 二维叠放偏移(序号[标识])).collect();
        for 窗口 in 偏移.windows(2) {
            assert!(
                窗口[0] < 窗口[1],
                "叠放偏移必须随遍历序号严格递增：{偏移:?}"
            );
        }
        assert!(
            *偏移.last().unwrap() < 0.001,
            "叠放偏移不应造成可见位移：{偏移:?}"
        );

        let 投影: Vec<实体标识> = 投影预览项(&文档).iter().map(|项| 项.来源).collect();
        assert_eq!(投影, 遍历, "投影顺序必须与叠放顺序同源");
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

    /// 预览快照必须把网格的**世界**包围盒抄给界面层。
    ///
    /// 3D 视口射线拾取靠这张表判断"点到了什么"：模型实例的网格是 glTF
    /// 子实体、在文档里没有标识，只靠文档属性算不出真实体积。
    /// 这里用"父节点平移 + 子节点缩放"两层钉住两件事：
    /// 中心要跟父节点走，半尺寸要吃掉子节点的缩放。
    #[test]
    fn 预览快照会抄出网格的世界包围盒() {
        let mut 应用 = App::new();
        应用.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin));
        应用.init_resource::<预览世界变换快照>();
        应用.add_systems(
            PostUpdate,
            同步预览世界变换.after(bevy::transform::TransformSystems::Propagate),
        );

        let 标识 = scene_doc::document::实体标识::新建();
        let 根 = 应用
            .world_mut()
            .spawn((预览实体, 预览来源(标识), Transform::from_xyz(5.0, 0.0, 0.0)))
            .id();
        应用.world_mut().spawn((
            Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(0.5)),
            Transform::from_scale(Vec3::splat(2.0)),
            ChildOf(根),
        ));
        应用.update();

        let 快照 = 应用.world().resource::<预览世界变换快照>();
        let 盒 = 快照.包围盒.get(&标识).expect("网格子实体应当贡献包围盒");
        let (中心x, 中心y, 中心z) = 盒.中心();
        assert!(
            (中心x - 5.0).abs() < 1e-4 && 中心y.abs() < 1e-4 && 中心z.abs() < 1e-4,
            "包围盒中心应跟随父节点世界位置，实际 ({中心x}, {中心y}, {中心z})"
        );
        let (半x, 半y, 半z) = 盒.半尺寸();
        assert!(
            (半x - 1.0).abs() < 1e-4 && (半y - 1.0).abs() < 1e-4 && (半z - 1.0).abs() < 1e-4,
            "半尺寸应吃掉子节点的 2 倍缩放，实际 ({半x}, {半y}, {半z})"
        );
    }
}
