//! 资源预览：把选中资源的**真实内容**准备好，交给界面显示。
//!
//! # 分工
//!
//! | 层 | 职责 |
//! | --- | --- |
//! | `editor_ui::assets_panel` | 点哪个资源（选中 + 预览请求） |
//! | `editor_ui::asset_preview` | 怎么画（纯 egui，不读盘） |
//! | 本模块 | 读盘、解码、解析、渲染，把结果写成一份 `资源预览快照` |
//!
//! # 为什么只在"选中变化"时做一次
//!
//! 预览面板每帧都画，但**资源内容不会每帧都变**。解码一张 4K PNG
//! 或解析一个 2 MB 的 glTF 是毫秒到几十毫秒级的操作，放进每帧绘制里
//! 会把整个编辑器拖垮。因此这里用"请求代次"驱动：
//!
//! ```text
//! 单击资源 → 界面状态.预览请求{目标, 代次+1}
//!          → 本系统发现代次未处理 → 读盘/解码/解析 → 写回快照{同一个代次}
//!          → 界面只采用"代次等于当前请求"的快照
//! ```
//!
//! 代次同时解决了**旧请求覆盖新选择**的竞态：用户快速点了 A → B 时，
//! 迟到的 A 结果代次更小，写回后界面也不会显示它。
//!
//! # 3D 模型为什么要独立相机
//!
//! 模型预览必须"看到真实网格"，而场景视口的相机看的是**当前编辑的场景**。
//! 若把模型塞进场景视口的渲染层，用户会在编辑器视口里看到预览模型混在
//! 自己的场景里 —— 那既不是预览，也会让人以为场景被改了。
//!
//! 因此模型预览有独立渲染层（[`模型预览层`]）、独立相机、独立渲染目标。
//! glTF 的子实体会被**递归**打上这个渲染层（Bevy 不会让子实体继承
//! `RenderLayers`，这是踩过的坑，见 `preview.rs` 的注释），
//! 而编辑态场景的相机看不见这一层。
//!
//! # 不写源文件
//!
//! 本模块**只读**项目文件。预览失败时只报告原因，绝不"修复"或重写资源。

use std::collections::HashMap;
use std::path::Path;

use bevy::camera::RenderTarget;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy_egui::{EguiContexts, EguiTextureHandle};
use editor_ui::state::{
    图片预览信息, 场景预览信息, 场景预览实体, 字体预览信息, 文本预览信息, 模型预览信息,
    资源预览快照, 音频预览信息, 预览画面, 预览阶段,
};
use scene_doc::document::场景文档;
use scene_doc::path::项目相对路径;
use scene_doc::property::资源类别;

use crate::plugin::{EditorStateHolder, UiStateHolder};

/// 模型预览的渲染层。
///
/// 与场景预览层（1）和三维视图层（2）都不同：三层互不可见，
/// 因此"预览模型"不会出现在视口里，也不会被视口相机拍到。
pub const 模型预览层: usize = 3;

/// 模型预览渲染目标的边长（像素）。
const 预览边长: u32 = 512;
/// 文本预览最多读多少字节。
const 文本最大字节: u64 = 512 * 1024;
/// 文本预览最多显示多少行。
const 文本最大行: usize = 400;
/// 场景预览最多列多少实体。
const 场景最大实体: usize = 200;
/// 字体预览的 egui 家族键。
pub const 字体家族键: &str = "asset_preview_font";

/// 一张已解码并注册给 egui 的图片。
struct 纹理条目 {
    /// 保留 Bevy 图像句柄：句柄一旦释放，注册给 egui 的纹理也会失效。
    _句柄: Handle<Image>,
    /// egui 纹理。
    纹理: egui::TextureId,
    /// 像素尺寸。
    尺寸: (u32, u32),
    /// 文件格式标识。
    格式: String,
    /// 是否带透明通道。
    有透明: bool,
    /// 文件修改时间（Unix 秒）。
    修改时间: Option<u64>,
    /// 文件长度。
    长度: u64,
}

/// 模型预览的运行时状态。
#[derive(Default)]
pub struct 模型预览运行时 {
    /// 相机与渲染目标是否已建立。
    已建立: bool,
    /// 预览根实体（glTF 场景挂在这里）。
    根: Option<Entity>,
    /// 渲染目标图像。
    目标图片: Option<Handle<Image>>,
    /// 注册给 egui 的纹理。
    纹理: Option<egui::TextureId>,
    /// 相机实体。
    相机: Option<Entity>,
    /// 当前预览的模型路径。
    当前路径: Option<String>,
    /// 已统计到的网格数。
    网格数: usize,
    /// 已统计到的材质数。
    材质数: usize,
    /// 世界空间包围盒（中心、半径）。
    包围盒: Option<(Vec3, f32)>,
}

/// 资源预览缓存。
#[derive(Resource, Default)]
pub struct 资源预览缓存 {
    /// 已解码的图片（键 = `路径#修改时间#长度`）。
    图片: HashMap<String, 纹理条目>,
    /// 已处理的请求代次。
    已处理代次: Option<u64>,
    /// 模型预览状态。
    模型: 模型预览运行时,
    /// 已注册的字体家族来自哪个路径。
    字体来源: Option<String>,
}

/// 模型预览相机。
#[derive(Component)]
pub struct 模型预览相机;

/// 模型预览根实体标记。
#[derive(Component)]
pub struct 模型预览根;

/// 资源预览插件。
pub struct 资源预览插件;

impl Plugin for 资源预览插件 {
    fn build(&self, app: &mut App) {
        app.init_resource::<资源预览缓存>().add_systems(
            Update,
            (处理预览请求, 同步模型预览, 清理失效选择, 监视选中资源变化).chain(),
        );
    }
}

/// 处理预览请求：把"想看什么"变成一份真实的快照。
#[allow(clippy::too_many_arguments)]
/// 一次预览计算的结果：快照 + 需要系统侧落地 GPU/egui 资源的原始数据。
pub struct 预览产物 {
    /// 写回界面状态的快照（此时还没有纹理，`画面` 字段留空）。
    pub 快照: 资源预览快照,
    /// 解码后的 RGBA 像素与新图片的尺寸（图片预览用）。
    pub 图片: Option<(Vec<u8>, u32, u32)>,
    /// 字体文件字节（字体预览要注册家族）。
    pub 字体字节: Option<Vec<u8>>,
    /// 模型绝对路径（模型预览要交给 AssetServer）。
    pub 模型路径: Option<std::path::PathBuf>,
}

/// 纯计算一次资源预览：读盘、解析、解码，**不碰 GPU 与 egui**。
///
/// # 为什么要把它从系统里拆出来
///
/// 系统函数需要 `Assets<Image>` / `EguiUserTextures` / `AssetServer`，
/// 于是"选中的资源解析成什么"这件事在测试里就完全不可达 ——
/// 只能靠真窗口验收。拆出纯函数后，同名文件、缺失文件、不支持格式、
/// 图片尺寸、场景层级这些**判断**都能被直接测试。
///
/// 它仍然读盘（那正是预览要做的事），但只读当前这一个文件。
pub fn 计算预览(
    项目: &editor_core::project::项目描述,
    路径: &项目相对路径,
    代次: u64,
    语言: editor_core::i18n::语言,
) -> 预览产物 {
    let 绝对 = 项目.绝对路径(路径);
    let mut 产物 = 预览产物 {
        快照: 资源预览快照 {
            代次,
            路径: Some(路径.文本().to_string()),
            类别: 判断类别(项目, 路径),
            阶段: 预览阶段::就绪,
            ..Default::default()
        },
        图片: None,
        字体字节: None,
        模型路径: None,
    };

    let 元数据 = match std::fs::metadata(&绝对) {
        Ok(元数据) => 元数据,
        Err(错误) => {
            产物.快照.阶段 = 预览阶段::失败;
            产物.快照.说明 = Some(
                语言
                    .填充(
                        "asset_preview_missing",
                        &[&format!("{}（{错误}）", 路径.文本())],
                    )
                    .to_string(),
            );
            return 产物;
        }
    };
    产物.快照.大小 = 元数据.len();
    产物.快照.修改时间 = 元数据.modified().ok().and_then(time_to_unix);

    let 类别 = 产物.快照.类别;
    let 扩展 = 路径.扩展名().unwrap_or_default();

    if 类别 == 资源类别::场景 {
        match 项目.读取场景(路径) {
            Ok(读取) => 产物.快照.场景 = Some(构建场景预览(&读取.文档)),
            Err(错误) => {
                产物.快照.阶段 = 预览阶段::失败;
                产物.快照.说明 = Some(format!("{}：{错误}", 语言.取("asset_preview_scene_failed")));
            }
        }
        return 产物;
    }

    if 类别 == 资源类别::图片 {
        let 字节 = match std::fs::read(&绝对) {
            Ok(字节) => 字节,
            Err(错误) => {
                产物.快照.阶段 = 预览阶段::失败;
                产物.快照.说明 = Some(
                    语言
                        .填充(
                            "asset_preview_read_failed",
                            &[&format!("{}：{错误}", 路径.文本())],
                        )
                        .to_string(),
                );
                return 产物;
            }
        };
        // 从**内容**猜格式，而不是只看扩展名：扩展名写错的文件不应被当成
        // "解码失败"，反过来也不该因为扩展名对就假装解析成功。
        let 读取器 =
            match image::ImageReader::new(std::io::Cursor::new(&字节)).with_guessed_format() {
                Ok(读取器) => 读取器,
                Err(错误) => {
                    产物.快照.阶段 = 预览阶段::失败;
                    产物.快照.说明 = Some(
                        语言
                            .填充(
                                "asset_preview_decode_failed",
                                &[&format!("{}：{错误}", 路径.文本())],
                            )
                            .to_string(),
                    );
                    return 产物;
                }
            };
        let 格式 = 读取器
            .format()
            .map(|格式| format!("{格式:?}").to_ascii_lowercase())
            .unwrap_or_default();
        match 读取器.decode() {
            Ok(图片) => {
                let 有透明 = 图片.color().has_alpha();
                let rgba = 图片.to_rgba8();
                let 尺寸 = (rgba.width(), rgba.height());
                产物.快照.图片 = Some(图片预览信息 {
                    宽: 尺寸.0,
                    高: 尺寸.1,
                    格式,
                    有透明,
                });
                产物.图片 = Some((rgba.into_raw(), 尺寸.0, 尺寸.1));
            }
            Err(错误) => {
                产物.快照.阶段 = 预览阶段::失败;
                产物.快照.说明 = Some(
                    语言
                        .填充(
                            "asset_preview_decode_failed",
                            &[&format!("{}：{错误}", 路径.文本())],
                        )
                        .to_string(),
                );
            }
        }
        return 产物;
    }

    if 类别 == 资源类别::模型 {
        if !matches!(扩展.as_str(), "gltf" | "glb") {
            产物.快照.阶段 = 预览阶段::不支持;
            产物.快照.说明 = Some(
                语言
                    .填充(
                        "asset_preview_unsupported_type",
                        &[&扩展.to_uppercase() as &dyn std::fmt::Display],
                    )
                    .to_string(),
            );
            return 产物;
        }
        // 是否真的加载成功由 AssetServer 决定；在那一刻之前如实标"准备中"，
        // 不拿一个默认方块冒充模型。
        产物.快照.模型 = Some(模型预览信息 {
            网格数: 0,
            材质数: 0,
            尺寸: None,
            等待渲染: true,
        });
        产物.模型路径 = Some(绝对);
        return 产物;
    }

    if 类别 == 资源类别::音频 {
        产物.快照.音频 = Some(读取音频信息(&绝对, 路径));
        return 产物;
    }

    if 类别 == 资源类别::字体 {
        let 字节 = match std::fs::read(&绝对) {
            Ok(字节) => 字节,
            Err(错误) => {
                产物.快照.阶段 = 预览阶段::失败;
                产物.快照.说明 = Some(
                    语言
                        .填充(
                            "asset_preview_read_failed",
                            &[&format!("{}：{错误}", 路径.文本())],
                        )
                        .to_string(),
                );
                return 产物;
            }
        };
        let 字体 = match ttf_parser::Face::parse(&字节, 0) {
            Ok(字体) => 字体,
            Err(_) => {
                产物.快照.阶段 = 预览阶段::失败;
                产物.快照.说明 = Some(语言.取("asset_preview_font_failed").to_string());
                return 产物;
            }
        };
        let 家族名 = 字体
            .names()
            .into_iter()
            .find(|名| 名.name_id == ttf_parser::name_id::FAMILY)
            .and_then(|名| 名.to_string())
            .unwrap_or_default();
        产物.快照.字体 = Some(字体预览信息 {
            家族键: 字体家族键.to_string(),
            家族名,
            字形数: Some(字体.number_of_glyphs() as u32),
        });
        产物.字体字节 = Some(字节);
        return 产物;
    }

    if let Some(文本) = 读取文本(&绝对, &扩展) {
        产物.快照.文本 = Some(文本);
        return 产物;
    }

    if matches!(类别, 资源类别::材质 | 资源类别::着色器) {
        match std::fs::read_to_string(&绝对) {
            Ok(内容) => {
                let 总行数 = 内容.lines().count();
                产物.快照.文本 = Some(文本预览信息 {
                    行: 内容.lines().take(文本最大行).map(str::to_string).collect(),
                    总行数,
                    语言: 扩展.clone(),
                    已截断: 总行数 > 文本最大行,
                });
            }
            Err(错误) => {
                产物.快照.阶段 = 预览阶段::失败;
                产物.快照.说明 = Some(
                    语言
                        .填充(
                            "asset_preview_read_failed",
                            &[&format!("{}：{错误}", 路径.文本())],
                        )
                        .to_string(),
                );
            }
        }
        return 产物;
    }

    产物.快照.阶段 = 预览阶段::不支持;
    产物.快照.说明 = Some(
        语言
            .填充(
                "asset_preview_unsupported_type",
                &[&扩展.to_uppercase() as &dyn std::fmt::Display],
            )
            .to_string(),
    );
    产物
}

/// 判断一个路径的类别。
///
/// 场景是特例：`scenes/*.scene.ron` 的扩展名是 `ron`，按扩展名推断只会得到
/// "其他"。这里先问项目自己的场景列表，再退回扩展名推断。
pub fn 判断类别(
    项目: &editor_core::project::项目描述, 路径: &项目相对路径
) -> 资源类别 {
    if 项目.场景列表().iter().any(|场景| 场景 == 路径) {
        return 资源类别::场景;
    }
    路径
        .扩展名()
        .map(|扩展| 资源类别::由扩展名推断(&扩展))
        .unwrap_or(资源类别::其他)
}

/// 图片纹理的缓存键。
///
/// 键里带 mtime 与长度：文件在编辑器外被覆盖后，缓存自然失效，
/// 不会继续显示旧图；同一文件反复选中时直接命中，不重复解码。
fn 图片缓存键(路径: &项目相对路径, 绝对: &Path) -> String {
    let (修改时间, 长度) = std::fs::metadata(绝对)
        .map(|元数据| {
            (
                元数据.modified().ok().and_then(time_to_unix).unwrap_or(0),
                元数据.len(),
            )
        })
        .unwrap_or((0, 0));
    format!("{}#{}#{}", 路径.文本(), 修改时间, 长度)
}

/// 处理预览请求：把"想看什么"变成一份真实的快照。
#[allow(clippy::too_many_arguments)]
fn 处理预览请求(
    mut 命令: Commands,
    编辑器: Res<EditorStateHolder>,
    mut 界面: ResMut<UiStateHolder>,
    mut 缓存: ResMut<资源预览缓存>,
    mut 图片资源: ResMut<Assets<Image>>,
    mut 上下文: EguiContexts,
    资源服务器: Res<AssetServer>,
) {
    let 请求 = 界面.状态.预览请求.clone();
    if 缓存.已处理代次 == Some(请求.代次) {
        return;
    }
    缓存.已处理代次 = Some(请求.代次);

    let 语言 = 界面.状态.语言;
    let Some(路径) = 请求.目标.clone() else {
        // 没有目标 = 清空预览（关闭项目）。
        界面.状态.资源预览 = 资源预览快照 {
            代次: 请求.代次,
            ..Default::default()
        };
        return;
    };

    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        界面.状态.资源预览 = 资源预览快照 {
            代次: 请求.代次,
            路径: Some(路径.文本().to_string()),
            阶段: 预览阶段::失败,
            说明: Some(语言.取("log_no_project").to_string()),
            ..Default::default()
        };
        return;
    };

    let 绝对 = 项目.绝对路径(&路径);
    let 键 = 图片缓存键(&路径, &绝对);

    // 图片命中缓存：不重新解码，也不重新注册纹理。
    if let Some(条目) = 缓存.图片.get(&键) {
        界面.状态.资源预览 = 资源预览快照 {
            代次: 请求.代次,
            路径: Some(路径.文本().to_string()),
            类别: 判断类别(项目, &路径),
            阶段: 预览阶段::就绪,
            大小: 条目.长度,
            修改时间: 条目.修改时间,
            图片: Some(图片预览信息 {
                宽: 条目.尺寸.0,
                高: 条目.尺寸.1,
                格式: 条目.格式.clone(),
                有透明: 条目.有透明,
            }),
            画面: Some(预览画面 {
                纹理: 条目.纹理,
                像素尺寸: 条目.尺寸,
            }),
            ..Default::default()
        };
        return;
    }

    let 产物 = 计算预览(项目, &路径, 请求.代次, 语言);

    // 图片：把解码结果注册成 egui 纹理。
    if let Some((rgba, 宽, 高)) = &产物.图片 {
        let 图像 = Image::new(
            Extent3d {
                width: *宽,
                height: *高,
                ..default()
            },
            TextureDimension::D2,
            rgba.clone(),
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        );
        let 句柄 = 图片资源.add(图像);
        let 纹理 = 上下文.add_image(EguiTextureHandle::Strong(句柄.clone()));
        缓存.图片.insert(
            键.clone(),
            纹理条目 {
                _句柄: 句柄,
                纹理,
                尺寸: (*宽, *高),
                格式: 产物
                    .快照
                    .图片
                    .as_ref()
                    .map(|图片| 图片.格式.clone())
                    .unwrap_or_default(),
                有透明: 产物
                    .快照
                    .图片
                    .as_ref()
                    .map(|图片| 图片.有透明)
                    .unwrap_or(false),
                修改时间: 产物.快照.修改时间,
                长度: 产物.快照.大小,
            },
        );
    }

    let mut 快照 = 产物.快照;

    // 图片：补上纹理句柄。
    if let Some(条目) = 缓存.图片.get(&键) {
        快照.画面 = Some(预览画面 {
            纹理: 条目.纹理,
            像素尺寸: 条目.尺寸,
        });
    }

    // 字体：注册家族（保留默认家族，避免界面变方框），并检查缺字。
    if let Some(字节) = &产物.字体字节 {
        if 缓存.字体来源.as_deref() != Some(路径.文本())
            && let Ok(ctx) = 上下文.ctx_mut()
        {
            let mut 定义 = editor_ui::构造字体定义(editor_ui::fonts::读取后备字体());
            定义.font_data.insert(
                字体家族键.to_string(),
                std::sync::Arc::new(egui::epaint::text::FontData {
                    font: std::borrow::Cow::Owned(字节.clone()),
                    index: 0,
                    tweak: Default::default(),
                }),
            );
            定义.families.insert(
                egui::FontFamily::Name(字体家族键.into()),
                vec![字体家族键.to_string()],
            );
            ctx.set_fonts(定义);
            缓存.字体来源 = Some(路径.文本().to_string());
            ctx.request_repaint();
        }
        if let Ok(字体) = ttf_parser::Face::parse(字节, 0) {
            let 缺失: Vec<char> = 界面
                .状态
                .字体样例
                .chars()
                .filter(|字符| *字符 != ' ' && 字体.glyph_index(*字符).is_none())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if !缺失.is_empty() {
                快照.说明 = Some(
                    语言
                        .填充(
                            "asset_preview_font_missing",
                            &[&缺失.iter().collect::<String>()],
                        )
                        .to_string(),
                );
            }
        }
    }

    // 模型：建立独立相机与渲染目标，再交给 AssetServer 加载。
    if let Some(模型绝对) = &产物.模型路径 {
        if let Some(旧) = 缓存.模型.根.take() {
            命令.entity(旧).despawn();
        }
        if !缓存.模型.已建立 {
            建立模型预览基础设施(&mut 命令, &mut 缓存, &mut 图片资源, &mut 上下文);
        }
        let 场景 = 资源服务器.load::<bevy::world_serialization::WorldAsset>(
            bevy::asset::AssetPath::from_path_buf(模型绝对.clone()).with_label("Scene0"),
        );
        let 根 = 命令
            .spawn((
                模型预览根,
                bevy::world_serialization::WorldAssetRoot(场景),
                // 先隐藏：glTF 的子实体第一帧还没打上预览层，若立即可见，
                // 它们会以默认层出现在主视口里一闪而过。
                Visibility::Hidden,
                RenderLayers::layer(模型预览层),
                Transform::default(),
            ))
            .id();
        缓存.模型.根 = Some(根);
        缓存.模型.当前路径 = Some(路径.文本().to_string());
        缓存.模型.网格数 = 0;
        缓存.模型.材质数 = 0;
        缓存.模型.包围盒 = None;
    }

    界面.状态.资源预览 = 快照;
}

/// 建立模型预览用的渲染目标与相机（只做一次）。
fn 建立模型预览基础设施(
    命令: &mut Commands,
    缓存: &mut 资源预览缓存,
    图片资源: &mut Assets<Image>,
    上下文: &mut EguiContexts,
) {
    let 尺寸 = Extent3d {
        width: 预览边长,
        height: 预览边长,
        ..default()
    };
    let mut 图片 = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("bevy_studio_资源预览渲染目标"),
            size: 尺寸,
            dimension: TextureDimension::D2,
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
    图片.resize(尺寸);
    let 图片句柄 = 图片资源.add(图片);
    let 纹理 = 上下文.add_image(EguiTextureHandle::Strong(图片句柄.clone()));
    let 相机 = 命令
        .spawn((
            模型预览相机,
            Camera3d::default(),
            Camera {
                // 在主界面之前渲染：这一帧的纹理在界面绘制时已经就绪。
                order: -2,
                clear_color: ClearColorConfig::Custom(Color::srgb(0.10, 0.10, 0.12)),
                ..default()
            },
            Projection::Perspective(PerspectiveProjection {
                fov: 45f32.to_radians(),
                ..default()
            }),
            RenderTarget::Image(图片句柄.clone().into()),
            RenderLayers::layer(模型预览层),
            Transform::from_xyz(0.0, 1.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    缓存.模型.已建立 = true;
    缓存.模型.相机 = Some(相机);
    缓存.模型.目标图片 = Some(图片句柄);
    缓存.模型.纹理 = Some(纹理);
}

/// 读文本类资源。
fn 读取文本(绝对: &Path, 扩展: &str) -> Option<文本预览信息> {
    let 语言 = match 扩展 {
        "rs" => "rust",
        "wgsl" | "wesl" => "wgsl",
        "ron" => "ron",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "txt" | "csv" | "tsv" | "ini" | "cfg" | "log" => "text",
        "scene" | "bsn" => "ron",
        _ => return None,
    };
    let 元数据 = std::fs::metadata(绝对).ok()?;
    if 元数据.len() > 文本最大字节 {
        return Some(文本预览信息 {
            行: Vec::new(),
            总行数: 0,
            语言: 语言.to_string(),
            已截断: true,
        });
    }
    let 内容 = std::fs::read_to_string(绝对).ok()?;
    let 总行数 = 内容.lines().count();
    Some(文本预览信息 {
        行: 内容.lines().take(文本最大行).map(str::to_string).collect(),
        总行数,
        语言: 语言.to_string(),
        已截断: 总行数 > 文本最大行,
    })
}

/// 场景层级预览。
pub fn 构建场景预览(文档: &场景文档) -> 场景预览信息 {
    let 遍历 = 文档.深度优先遍历();
    let mut 实体 = Vec::new();
    for (序号, (标识, 深度)) in 遍历.iter().enumerate() {
        if 序号 >= 场景最大实体 {
            break;
        }
        if let Some(场景实体) = 文档.取(*标识) {
            实体.push(场景预览实体 {
                深度: *深度,
                名称: 场景实体.名称.clone(),
                组件数: 场景实体.组件.len(),
            });
        }
    }
    场景预览信息 {
        名称: 文档.元数据.场景名.clone(),
        实体总数: 文档.实体数量(),
        根实体数: 文档.顶层顺序.len(),
        实体,
        已截断: 遍历.len() > 场景最大实体,
    }
}

/// 音频：只报告**真实可读**的元数据。
fn 读取音频信息(绝对: &Path, 路径: &项目相对路径) -> 音频预览信息 {
    let mut 信息 = 音频预览信息 {
        可试听: crate::audio_preview::可试听(路径),
        ..Default::default()
    };
    let Ok(字节) = std::fs::read(绝对) else {
        return 信息;
    };
    if let Some((采样率, 声道, 位深, 时长)) = 解析_wav头(&字节) {
        信息.采样率 = Some(采样率);
        信息.声道数 = Some(声道);
        信息.位深 = Some(位深);
        信息.时长秒 = Some(时长);
    }
    信息
}

/// 解析 WAV 头，返回 `(采样率, 声道, 位深, 时长秒)`。
///
/// 只处理 PCM/浮点 WAV 的 `fmt ` + `data` 块；压缩编码的 WAV
/// （ADPCM 等）返回 `None`，因为按 PCM 算出来的时长会是错的。
pub fn 解析_wav头(字节: &[u8]) -> Option<(u32, u16, u16, f32)> {
    if 字节.len() < 44 || &字节[0..4] != b"RIFF" || &字节[8..12] != b"WAVE" {
        return None;
    }
    let mut 位置 = 12usize;
    let mut 声道 = None;
    let mut 采样率 = None;
    let mut 位深 = None;
    let mut 格式 = None;
    let mut 数据长度 = None;
    while 位置 + 8 <= 字节.len() {
        let 块标识 = &字节[位置..位置 + 4];
        let 块长度 = u32::from_le_bytes(字节[位置 + 4..位置 + 8].try_into().ok()?) as usize;
        let 数据起点 = 位置 + 8;
        match 块标识 {
            b"fmt " => {
                if 数据起点 + 16 > 字节.len() {
                    return None;
                }
                格式 = Some(u16::from_le_bytes(
                    字节[数据起点..数据起点 + 2].try_into().ok()?,
                ));
                声道 = Some(u16::from_le_bytes(
                    字节[数据起点 + 2..数据起点 + 4].try_into().ok()?,
                ));
                采样率 = Some(u32::from_le_bytes(
                    字节[数据起点 + 4..数据起点 + 8].try_into().ok()?,
                ));
                位深 = Some(u16::from_le_bytes(
                    字节[数据起点 + 14..数据起点 + 16].try_into().ok()?,
                ));
            }
            b"data" => {
                数据长度 = Some(块长度.min(字节.len().saturating_sub(数据起点)));
            }
            _ => {}
        }
        位置 = 数据起点.saturating_add(块长度);
        // 块按偶数对齐。
        if 块长度 % 2 == 1 {
            位置 += 1;
        }
    }
    let (Some(格式码), Some(声道), Some(采样率), Some(位深), Some(数据长度)) =
        (格式, 声道, 采样率, 位深, 数据长度)
    else {
        return None;
    };
    // 1 = PCM，3 = IEEE float；其余编码无法按此推算时长。
    if !matches!(格式码, 1 | 3) || 声道 == 0 || 采样率 == 0 || 位深 == 0 {
        return None;
    }
    let 每秒字节 = 采样率 as f32 * 声道 as f32 * (位深 as f32 / 8.0);
    if 每秒字节 <= 0.0 {
        return None;
    }
    Some((采样率, 声道, 位深, 数据长度 as f32 / 每秒字节))
}

/// 选中被清空时，把预览与模型资源一起收掉。
fn 清理失效选择(
    mut 界面: ResMut<UiStateHolder>,
    mut 缓存: ResMut<资源预览缓存>,
    mut 命令: Commands,
) {
    if 界面.状态.资源选中.is_some() {
        return;
    }
    if let Some(根) = 缓存.模型.根.take() {
        命令.entity(根).despawn();
    }
    缓存.模型.当前路径 = None;
    if 界面.状态.资源预览.阶段 != 预览阶段::空闲 {
        界面.状态.清空资源预览();
    }
}

/// 监视选中的文件是否在磁盘上被改名/删除/覆盖。
///
/// 资源被重命名或删除后，预览不能继续显示旧内容：用户会以为"文件还在"。
/// 这里按 mtime + 长度判断，变了就重新走一次预览请求（请求会如实报告
/// 文件缺失），而不是自动做任何"修复"。
fn 监视选中资源变化(mut 界面: ResMut<UiStateHolder>, 编辑器: Res<EditorStateHolder>) {
    let Some(路径) = 界面.状态.资源选中.clone() else {
        return;
    };
    if 界面.状态.资源预览.阶段 != 预览阶段::就绪 {
        return;
    }
    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        return;
    };
    let 绝对 = 项目.绝对路径(&路径);
    let Ok(元数据) = std::fs::metadata(&绝对) else {
        // 文件没了：让加载逻辑给出"文件不存在"的明确说明。
        界面.状态.重新加载预览();
        return;
    };
    let 修改时间 = 元数据.modified().ok().and_then(time_to_unix);
    if 界面.状态.资源预览.修改时间 != 修改时间 || 界面.状态.资源预览.大小 != 元数据.len()
    {
        界面.状态.重新加载预览();
    }
}

fn time_to_unix(时间: std::time::SystemTime) -> Option<u64> {
    时间
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|时长| 时长.as_secs())
}

/// 模型预览：递归打渲染层、统计网格/材质、取景、驱动相机。
#[allow(clippy::too_many_arguments)]
fn 同步模型预览(
    mut 命令: Commands,
    mut 界面: ResMut<UiStateHolder>,
    mut 缓存: ResMut<资源预览缓存>,
    mut 相机查询: Query<&mut Transform, With<模型预览相机>>,
    根查询: Query<(Entity, Option<&Children>), With<模型预览根>>,
    子查询: Query<&Children>,
    层查询: Query<&RenderLayers>,
    网格查询: Query<(&Mesh3d, Option<&Aabb>, &GlobalTransform)>,
    材质查询: Query<&MeshMaterial3d<StandardMaterial>>,
) {
    let Some(根实体) = 缓存.模型.根 else {
        return;
    };
    if 根查询.get(根实体).is_err() {
        缓存.模型.根 = None;
        return;
    }

    // 1. 递归给后代打上预览层。glTF 子实体**不继承** `RenderLayers`。
    let 后代 = 收集后代(根实体, &子查询);
    for 实体 in &后代 {
        let 已有 = 层查询
            .get(*实体)
            .map(|层| 层.intersects(&RenderLayers::layer(模型预览层)))
            .unwrap_or(false);
        if !已有 {
            命令.entity(*实体).insert(RenderLayers::layer(模型预览层));
        }
    }

    // 2. 只统计**属于这个预览根**的网格与材质。
    let 网格: Vec<(Aabb, GlobalTransform)> = 后代
        .iter()
        .filter_map(|实体| 网格查询.get(*实体).ok())
        .filter_map(|(_, aabb, 变换)| aabb.map(|aabb| (*aabb, *变换)))
        .collect();
    let 网格数 = 后代
        .iter()
        .filter(|实体| 网格查询.get(**实体).is_ok())
        .count();
    let 材质数 = 后代
        .iter()
        .filter(|实体| 材质查询.get(**实体).is_ok())
        .count();

    if 网格数 == 0 {
        // 还没加载完（或加载失败）：保持隐藏，界面显示"正在准备"。
        return;
    }

    // 3. 第一次统计到网格：解除隐藏并计算包围盒。
    if 缓存.模型.网格数 == 0 {
        命令.entity(根实体).insert(Visibility::Inherited);
    }
    缓存.模型.网格数 = 网格数;
    缓存.模型.材质数 = 材质数;

    let mut 最小: Option<Vec3> = None;
    let mut 最大: Option<Vec3> = None;
    for (aabb, 变换) in &网格 {
        let 仿射 = 变换.affine();
        // 8 个角点都要变换：只变换中心会低估旋转后的包围盒。
        for 序号 in 0..8 {
            let 符号 = Vec3::new(
                if 序号 & 1 == 0 { -1.0 } else { 1.0 },
                if 序号 & 2 == 0 { -1.0 } else { 1.0 },
                if 序号 & 4 == 0 { -1.0 } else { 1.0 },
            );
            let 角 = Vec3::from(aabb.center) + Vec3::from(aabb.half_extents) * 符号;
            let 世界 = 仿射.transform_point3(角);
            最小 = Some(最小.map(|值| 值.min(世界)).unwrap_or(世界));
            最大 = Some(最大.map(|值| 值.max(世界)).unwrap_or(世界));
        }
    }
    let 包围盒 = match (最小, 最大) {
        (Some(最小), Some(最大)) => {
            let 中心 = (最小 + 最大) * 0.5;
            let 半径 = ((最大 - 最小).length() * 0.5).max(0.05);
            Some((中心, 半径))
        }
        _ => None,
    };
    if let Some(值) = 包围盒 {
        缓存.模型.包围盒 = Some(值);
    }

    // 4. 相机：按界面里的旋转/缩放与模型包围盒取景。
    let (中心, 半径) = 缓存.模型.包围盒.unwrap_or((Vec3::ZERO, 1.0));
    if let Some(相机) = 缓存.模型.相机
        && let Ok(mut 变换) = 相机查询.get_mut(相机)
    {
        let (偏航, 俯仰) = 界面.状态.模型旋转;
        let 缩放 = 界面.状态.模型缩放.max(0.05);
        // 让包围球正好装进 45° 视场：距离 = r / sin(fov/2)，
        // 再留一点余量、叠加用户缩放。
        let 距离 = 半径 / (45f32.to_radians() * 0.5).sin() * 1.15 / 缩放;
        let 偏航 = 偏航.to_radians();
        let 俯仰 = 俯仰.to_radians().clamp(-1.5, 1.5);
        let 方向 = Vec3::new(偏航.sin() * 俯仰.cos(), 俯仰.sin(), 偏航.cos() * 俯仰.cos());
        变换.translation = 中心 + 方向 * 距离.max(0.2);
        变换.look_at(中心, Vec3::Y);
    }

    // 5. 把画面与统计写回快照。
    if let Some(纹理) = 缓存.模型.纹理 {
        界面.状态.资源预览.画面 = Some(预览画面 {
            纹理,
            像素尺寸: (预览边长, 预览边长),
        });
        if let Some(模型) = 界面.状态.资源预览.模型.as_mut() {
            模型.网格数 = 网格数;
            模型.材质数 = 材质数;
            模型.等待渲染 = false;
            模型.尺寸 = 包围盒.map(|(_, 半径)| (半径 * 2.0, 半径 * 2.0, 半径 * 2.0));
        }
    }
}

/// 收集一个实体的全部后代（不含自己）。
fn 收集后代(根: Entity, 子查询: &Query<&Children>) -> Vec<Entity> {
    let mut 结果 = Vec::new();
    let mut 栈 = vec![根];
    while let Some(当前) = 栈.pop() {
        if let Ok(子列表) = 子查询.get(当前) {
            for 子 in 子列表.iter() {
                结果.push(子);
                栈.push(子);
            }
        }
    }
    结果
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 一个最小合法 WAV：单声道 / 8000 Hz / 16 bit / 0.5 秒静音。
    fn 造_wav(采样率: u32, 声道: u16, 位深: u16, 数据字节: u32) -> Vec<u8> {
        let mut 字节 = Vec::new();
        字节.extend_from_slice(b"RIFF");
        字节.extend_from_slice(&(36 + 数据字节).to_le_bytes());
        字节.extend_from_slice(b"WAVE");
        字节.extend_from_slice(b"fmt ");
        字节.extend_from_slice(&16u32.to_le_bytes());
        字节.extend_from_slice(&1u16.to_le_bytes());
        字节.extend_from_slice(&声道.to_le_bytes());
        字节.extend_from_slice(&采样率.to_le_bytes());
        let 每秒 = 采样率 * 声道 as u32 * (位深 as u32 / 8);
        字节.extend_from_slice(&每秒.to_le_bytes());
        字节.extend_from_slice(&(声道 * 位深 / 8).to_le_bytes());
        字节.extend_from_slice(&位深.to_le_bytes());
        字节.extend_from_slice(b"data");
        字节.extend_from_slice(&数据字节.to_le_bytes());
        字节.extend(std::iter::repeat_n(0u8, 数据字节 as usize));
        字节
    }

    #[test]
    fn wav头能读出真实元数据() {
        let wav = 造_wav(8_000, 1, 16, 8_000);
        let (采样率, 声道, 位深, 时长) = 解析_wav头(&wav).expect("应当解析成功");
        assert_eq!(采样率, 8_000);
        assert_eq!(声道, 1);
        assert_eq!(位深, 16);
        assert!((时长 - 0.5).abs() < 0.01, "实际时长 {时长}");
    }

    #[test]
    fn 非wav文件不会被当成wav() {
        let 字节 = b"OggS\0\0\0\0not a wav file";
        assert!(解析_wav头(字节).is_none());
    }

    #[test]
    fn 损坏的wav头返回none() {
        let mut wav = 造_wav(44_100, 2, 16, 100);
        // 把 fmt 块的数据起点截断。
        wav.truncate(20);
        assert!(解析_wav头(&wav).is_none());
    }
}
