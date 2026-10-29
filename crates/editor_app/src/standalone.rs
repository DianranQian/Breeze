// SPDX-License-Identifier: MIT OR Apache-2.0
//! 独立游戏运行模式（Standalone Game Mode）：同一个可执行文件，两种身份。
//!
//! # 它解决的具体问题
//!
//! `dist/` 里那份 exe 启动的是编辑器（带面板、带撤销栈、带文件对话框）。
//! 把游戏交给玩家或测试者时，那些东西一个都不该出现 —— 玩家要的是
//! 「双击就进游戏」。这一模块把"游戏身份"的启动路径补上：
//!
//! ```text
//! main.rs ──► 识别启动() ──┬─► Some(请求) ─► 载入() ─► 运行()   （只有画面，没有界面）
//!                          └─► None      ─► 编辑器照旧启动
//! ```
//!
//! # 判定顺序（三条入口，一条规则）
//!
//! | 情况 | 结果 |
//! | --- | --- |
//! | 命令行有 `--game [目录]` / `--game=目录` | 进游戏，目录缺省用当前工作目录 |
//! | exe 同级存在 `game_manifest.ron` | 进游戏，项目就是 exe 所在目录 |
//! | 当前工作目录存在 `game_manifest.ron` | 进游戏（开发期 `cargo run -- --game ..` 之外的便利路径） |
//! | 以上都没有 | 编辑器 |
//!
//! `--windowed` 只覆盖"全屏"这一件事，不改变是否进游戏。
//!
//! # 为什么复用编辑器投影，而不是另写一套渲染
//!
//! 编辑器已经有一套经过测试的**文档 → 渲染实体**投影（`preview`、
//! `light_preview`、`ui_preview`、`animation_runtime`、`particle_runtime`）
//! 和一套**文档 → 运行时世界**桥接（`scene_bridge` + `physics_runtime`）。
//! 游戏模式要的正是这两套东西，区别只有两处：
//!
//! 1. 相机直接渲染到窗口，不再渲染给 egui 的纹理；
//! 2. 运行时的姿态要写回投影节点，否则物理模拟出来的运动在画面上看不见
//!    （编辑器里物理只在隔离世界推进，视口显示的是文档姿态）。
//!
//! 因此这里**不新增第三套渲染路径**（纪律一）：
//! 画面与编辑器视口所见即同一份投影，"编辑器里长这样、打包后长那样"
//! 这类最难复现的 bug 从构造上就不存在。
//!
//! # 数据流
//!
//! ```text
//! 场景文档（只读）
//!    ├─► 预览投影（主世界实体）──► Bevy 渲染 ──► 窗口
//!    │        ▲
//!    │        └── 同步运行姿态：运行时本地 Transform → 投影节点
//!    └─► 运行时世界（隔离 World）──► 物理步进 / 粒子推进
//! ```
//!
//! # 诚实边界
//!
//! * **没有脚本**：运行时世界不执行用户代码，游戏逻辑仍走
//!   `run.rs` 的子进程路径。模式名字里的"游戏"指"能看能动的场景"。
//! * **3D 需要场景相机节点**：文档里有 `Camera3d` 时用透视相机；
//!   没有时一律 2D 正交 —— 此时三维网格/模型不在渲染层内，画面会是空的，
//!   启动日志会明确告警，而不是让人对着黑屏猜。
//! * **2D 画面按设计分辨率取景**：正交投影用
//!   `ScalingMode::AutoMin{设计宽, 设计高}`，即"1 世界单位 = 1 设计像素"
//!   （与编辑器视口默认缩放 1.0 同一语义）；窗口更宽时看到更多世界，
//!   不拉伸、不裁切。界面（HUD）按设计分辨率等比缩放并居中（letterbox），
//!   沿用 `ui_preview` 的同一套换算。

use std::path::{Path, PathBuf};

use bevy::camera::{ScalingMode, visibility::RenderLayers};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode, WindowResolution};
use editor_core::game_manifest::{游戏清单, 游戏清单文件名};
use editor_core::state::编辑器状态;
use scene_doc::document::{场景文档, 实体标识};

use crate::plugin::EditorStateHolder;
use crate::preview::{
    三维渲染层, 三维预览资源, 材质缓存, 贴图缓存, 预览同步状态, 预览实体, 预览来源, 预览渲染层,
    预览相机,
};
use crate::scene_bridge::{全部内置组件注册表, 编辑器相机3d, 运行时宿主};

/// 命令行里声明"以游戏身份启动"的参数名。
pub const 游戏模式参数: &str = "--game";

/// 命令行里把全屏覆盖成窗口模式的参数名。
///
/// 发布包默认清单里 `全屏: true`；调试时不希望独占显示器就用它。
pub const 窗口模式参数: &str = "--windowed";

/// 三维场景相机的组件名（与 `scene_bridge` 的注册名一致）。
const 三维相机组件名: &str = "Camera3d";

/// 一组三维内容组件名：用于"场景是三维但没有相机"的告警判据。
const 三维内容组件名: &[&str] = &[
    "Primitive3d",
    crate::components3d::方向光组件名,
    crate::components3d::点光组件名,
    crate::components3d::聚光灯组件名,
    crate::physics3d_components::碰撞体3D组件名,
    crate::physics3d_components::刚体3D组件名,
];

/// 判定出来的游戏启动请求。
#[derive(Debug, Clone, PartialEq)]
pub struct 启动请求 {
    /// 项目根目录（清单所在目录，或 `--game` 指定的目录）。
    pub 项目目录: PathBuf,
    /// 读到的清单。`--game` 指着一个没有清单的工程时是 `None`，
    /// 此时主场景与设计分辨率回退到 `project.ron`。
    pub 清单: Option<游戏清单>,
    /// 是否全屏。清单的 `全屏` 与 `--windowed` 合成后的结果。
    pub 全屏: bool,
}

/// 把命令行参数解析成启动请求。
///
/// # 参数
///
/// * `参数` —— 不含程序名的参数表（`std::env::args().skip(1)`）；
/// * `可执行目录` —— exe 所在目录，发布包把清单放在这里；
/// * `当前目录` —— 进程工作目录，`--game 相对路径` 以它为基准。
///
/// # 返回
///
/// * `Ok(None)` —— 不是游戏启动，调用方继续走编辑器；
/// * `Ok(Some)` —— 以游戏身份启动；
/// * `Err` —— 清单存在但读不出来（版本过新、RON 损坏、主场景非法）。
///   这时**不能**悄悄退回编辑器：用户双击的是游戏包，退回编辑器只会让
///   他以为"打包产物坏了"，而真正的原因在清单里。
pub fn 识别启动(
    参数: &[String],
    可执行目录: &Path,
    当前目录: &Path,
) -> Result<Option<启动请求>, String> {
    let 声明游戏 = 参数
        .iter()
        .any(|项| 项 == 游戏模式参数 || 项.starts_with(&format!("{游戏模式参数}=")));
    let 要求窗口 = 参数.iter().any(|项| 项 == 窗口模式参数);
    let 显式目录 = 解析游戏目录(参数, 当前目录);

    let 候选: Vec<PathBuf> = match (声明游戏, 显式目录) {
        (true, Some(目录)) => vec![目录],
        (true, None) => vec![当前目录.to_path_buf()],
        (false, _) => vec![可执行目录.to_path_buf(), 当前目录.to_path_buf()],
    };

    for 目录 in 候选 {
        let 有清单 = 游戏清单::定位(&目录).is_some();
        if !声明游戏 && !有清单 {
            continue;
        }
        let 清单 = match 有清单 {
            true => Some(
                游戏清单::读取(&目录)
                    .map_err(|错误| format!("{}：{错误}", 目录.join(游戏清单文件名).display()))?,
            ),
            false => None,
        };
        let 全屏 = !要求窗口 && 清单.as_ref().map(|清单| 清单.全屏).unwrap_or(true);
        return Ok(Some(启动请求 {
            项目目录: 目录,
            清单,
            全屏,
        }));
    }
    Ok(None)
}

/// 取出 `--game 目录` / `--game=目录` 里显式给出的工程目录。
fn 解析游戏目录(参数: &[String], 当前目录: &Path) -> Option<PathBuf> {
    let mut 索引 = 0;
    while 索引 < 参数.len() {
        let 当前 = 参数[索引].as_str();
        if let Some(尾) = 当前.strip_prefix(&format!("{游戏模式参数}=")) {
            let 尾 = 尾.trim();
            if 尾.is_empty() {
                return None;
            }
            return Some(相对于(尾, 当前目录));
        }
        if 当前 == 游戏模式参数 {
            if let Some(下一) = 参数.get(索引 + 1) {
                if !下一.starts_with('-') && !下一.trim().is_empty() {
                    return Some(相对于(下一.trim(), 当前目录));
                }
            }
            return None;
        }
        索引 += 1;
    }
    None
}

/// 把命令行里的路径落到当前工作目录下（绝对路径原样保留）。
fn 相对于(文本: &str, 当前目录: &Path) -> PathBuf {
    let 路径 = Path::new(文本);
    if 路径.is_absolute() {
        路径.to_path_buf()
    } else {
        当前目录.join(路径)
    }
}

/// 载入完成、可以直接开跑的游戏配置。
#[derive(Debug)]
pub struct 游戏启动配置 {
    /// 窗口标题。
    pub 名称: String,
    /// 是否全屏。
    pub 全屏: bool,
    /// 取景用的设计分辨率。
    pub 设计分辨率: (u32, u32),
    /// 项目根目录（资源根与资产路径的基准）。
    pub 项目目录: PathBuf,
    /// 已载入的项目与场景。与编辑器用的是同一个 `编辑器状态`，
    /// 区别是这里永远不会有人改它。
    pub 状态: 编辑器状态,
    /// 场景相机：文档里的 `Camera3d` 实体与其参数。
    /// `None` 表示按二维正交取景。
    pub 三维相机: Option<(实体标识, 编辑器相机3d)>,
}

/// 打开项目与主场景，产出可直接运行的配置。
///
/// # 步骤
///
/// 1. 装完整内置组件注册表（否则场景里的灯光、物理组件会被当成未注册类型）；
/// 2. `编辑器状态::打开项目` —— 复用编辑器的项目装载（含资源标识表同步、
///    预制件目录刷新），不另写一份只读装载；
/// 3. 主场景：清单声明优先，否则用 `project.ron` 的启动场景 / 场景目录第一项；
/// 4. 把清单里的设计分辨率写进**内存里**的项目清单（不落盘）：
///    `ui_preview` 与渲染尺寸换算读的就是这一份，不覆盖就会出现
///    "清单说 1280×720、界面仍按 1920×1080 排版"的分裂；
/// 5. 拒绝空场景：运行时构建本身就把空场景当成错误，与其让窗口黑着，
///    不如把原因写成一句话还给调用方。
pub fn 载入(请求: &启动请求) -> Result<游戏启动配置, String> {
    let mut 状态 = 编辑器状态::新建();
    状态.注册表 = 全部内置组件注册表();
    状态
        .打开项目(&请求.项目目录)
        .map_err(|错误| format!("打开项目 {} 失败：{错误}", 请求.项目目录.display()))?;
    // 先一次性取出后面还要用的项目信息（都是 owned 值），让项目的不可变
    // 借用在此结束 —— 下面要 `打开场景` 和改设计分辨率，都需要 `&mut 状态`。
    let 回退主场景 = 状态
        .项目
        .as_ref()
        .expect("打开项目成功后项目一定存在")
        .启动场景();
    let 项目名 = 状态.项目.as_ref().expect("同上").名称().to_string();
    let 项目设计分辨率 = 状态.项目.as_ref().expect("同上").清单().设计分辨率;

    let 主场景 = match &请求.清单 {
        Some(清单) => Some(
            清单
                .主场景路径()
                .map_err(|错误| format!("游戏清单里的主场景非法：{错误}"))?,
        ),
        None => 回退主场景,
    };
    let Some(主场景) = 主场景 else {
        return Err(format!(
            "{项目名} 里没有场景文件，无法作为游戏启动（请在编辑器里保存一个场景后重新打包）"
        ));
    };

    状态
        .打开场景(&主场景)
        .map_err(|错误| format!("打开场景「{}」失败：{错误}", 主场景.文本()))?;

    if 状态.会话.文档.实体表.is_empty() {
        return Err(format!(
            "场景「{}」里没有任何实体，独立运行时画面会是空的",
            主场景.文本()
        ));
    }

    let 设计分辨率 = 请求
        .清单
        .as_ref()
        .map(|清单| 清单.设计分辨率)
        .unwrap_or(项目设计分辨率);
    if let Some(项目可变) = 状态.项目.as_mut() {
        项目可变.清单可变().设计分辨率 = 设计分辨率;
    }

    let 名称 = 请求
        .清单
        .as_ref()
        .map(|清单| 清单.名称.clone())
        .filter(|名称| !名称.trim().is_empty())
        .unwrap_or(项目名);

    Ok(游戏启动配置 {
        名称,
        全屏: 请求.全屏,
        设计分辨率,
        项目目录: 请求.项目目录.clone(),
        三维相机: 解析三维相机(&状态),
        状态,
    })
}

/// 从文档里取第一个 `Camera3d` 节点的参数。
///
/// 走注册表的 `实例化` 而不是自己读属性表：视野/裁剪面的合法区间
/// （夹在 1°–179°、远裁剪大于近裁剪）是注册表实现的既有契约，
/// 在第二处重写一遍数值边界，早晚会出现"编辑器里合法、游戏里退化"的分裂。
///
/// 遍历**不能**用 `?` 提前返回：文档里绝大多数实体本来就没有 `Camera3d`，
/// 缺组件、取不到实体都属于"跳过这一个"，只有真的找到相机才返回。
fn 解析三维相机(状态: &编辑器状态) -> Option<(实体标识, 编辑器相机3d)> {
    let 类型 = 状态.注册表.查询(三维相机组件名)?;
    for 标识 in 状态.会话.文档.文档遍历() {
        let Some(实体) = 状态.会话.文档.取(标识) else {
            continue;
        };
        let Some(组件) = 实体.取组件(三维相机组件名) else {
            continue;
        };
        let mut 实例 = 类型.默认实例();
        类型.实例化(&mut *实例, &组件.属性);
        if let Some(参数) = 实例.downcast_ref::<编辑器相机3d>() {
            return Some((标识, *参数));
        }
    }
    None
}

/// 独立游戏的窗口/相机参数（`游戏启动配置` 里去掉文档的那一半，作为 ECS 资源）。
#[derive(Resource)]
struct 游戏运行参数 {
    设计分辨率: (u32, u32),
    三维相机: Option<(实体标识, 编辑器相机3d)>,
}

/// 三维取景相机的绑定：文档里那台 `Camera3d` → 它的投影节点 → 渲染相机。
///
/// `节点` 缓存"文档相机被投影成了哪个实体"。没有这份缓存，每帧都要遍历
/// 全部预览实体去找它，实体数一多就是白花的帧预算；缓存失效（尚未建立、
/// 或投影实体被重建）时由 [`解析相机节点`] 重新解析。
#[derive(Component)]
struct 游戏相机绑定 {
    /// 文档里那台 `Camera3d` 的实体标识。
    文档: 实体标识,
    /// 已解析到的投影节点实体。
    节点: Option<Entity>,
}

/// 按请求载入并阻塞运行；载入失败时把错误交回调用方打印。
///
/// 返回 `None` 表示这不是游戏启动（调用方继续开编辑器）。
pub fn 从命令行启动(参数: &[String]) -> Option<Result<(), String>> {
    let 可执行目录 = std::env::current_exe()
        .ok()
        .and_then(|路径| 路径.parent().map(|目录| 目录.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let 当前目录 = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let 请求 = match 识别启动(参数, &可执行目录, &当前目录) {
        Ok(Some(请求)) => 请求,
        Ok(None) => return None,
        Err(错误) => return Some(Err(错误)),
    };
    Some(载入(&请求).map(运行))
}

/// 拉起一个只有画面的 Bevy 应用并阻塞到窗口关闭。
pub fn 运行(配置: 游戏启动配置) {
    构建游戏应用(配置).run();
}

/// 组装游戏应用（不进入主循环，便于测试与自定义退出）。
///
/// # 为什么不加 `EguiPlugin`
///
/// 加了就必然带出一整套界面状态与停靠布局；即使一个面板都不画，
/// egui 也会占住第一台相机并改变输入路径。游戏模式**从装配层面**就没有
/// 界面，而不是"有界面但藏起来"。
///
/// # 资产根
///
/// `AssetPlugin.file_path` 指向项目里的 `assets/`，与编辑器的做法一致；
/// 投影系统加载贴图/模型用的是项目根拼出来的**绝对路径**，
/// 因此必须保持 `UnapprovedPathMode::Allow`，否则 Bevy 会以
/// "路径在资产根之外"为由拒绝加载，表现为图片全黑。
fn 构建游戏应用(配置: 游戏启动配置) -> App {
    let (设计宽, 设计高) = 配置.设计分辨率;
    let 资源目录 = 配置.项目目录.join(editor_core::project::目录名::资源);
    let 窗口模式 = if 配置.全屏 {
        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
    } else {
        WindowMode::Windowed
    };
    let 名称 = 配置.名称.clone();

    let mut 应用 = App::new();
    应用.add_plugins(
        DefaultPlugins
            .set(bevy::asset::AssetPlugin {
                file_path: 资源目录.display().to_string(),
                unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                ..default()
            })
            // 窗口必须走 `DefaultPlugins.set`：`DefaultPlugins` 里已经带了一份
            // `WindowPlugin`，再 `add_plugins` 一次会在启动瞬间 panic
            // （"plugin was already added in application"）。
            .set(bevy::window::WindowPlugin {
                primary_window: Some(Window {
                    title: 名称,
                    resolution: WindowResolution::new(设计宽, 设计高),
                    mode: 窗口模式,
                    ..default()
                }),
                ..default()
            }),
    );
    装配内核(&mut 应用, 配置);
    应用
}

/// 把"只有画面"的那一份装配进应用：编辑状态、取景参数、投影与运行时插件。
///
/// # 为什么和 `DefaultPlugins` 分开
///
/// `WinitPlugin::build` 在构造 `DefaultPlugins` 时就创建系统事件循环，
/// 而 winit 在非主线程构造事件循环会直接 panic —— 单元测试跑在工作线程上，
/// 所以测试里碰不得 `DefaultPlugins`。把窗口/渲染外壳与游戏内核拆开后，
/// 测试用裸 `App::new()` 装同一份内核，检查的正是产品要求本身：
/// **游戏模式没有装配任何界面层**。
fn 装配内核(应用: &mut App, 配置: 游戏启动配置) {
    let 参数 = 游戏运行参数 {
        设计分辨率: 配置.设计分辨率,
        三维相机: 配置.三维相机,
    };
    应用
        .insert_resource(EditorStateHolder {
            状态: 配置.状态
        })
        .insert_resource(参数)
        .add_plugins(独立游戏插件);
}

/// 独立游戏插件：投影 + 运行时 + 取景，没有任何界面系统。
pub struct 独立游戏插件;

impl Plugin for 独立游戏插件 {
    fn build(&self, app: &mut App) {
        app.init_resource::<运行时宿主>()
            .init_resource::<crate::physics_runtime::物理运行时状态>()
            .init_resource::<crate::light_preview::灯光同步状态>()
            .init_resource::<crate::ui_preview::界面同步状态>()
            .add_systems(Startup, 初始化游戏)
            .add_systems(
                Update,
                (
                    (
                        // 文档 → 投影实体：只在首帧真正干活（运行期没人改文档），
                        // 但守卫逻辑本来就是按修订号写的，这里不改它。
                        crate::preview::同步预览实体,
                        // 紧跟投影：相机要的节点实体这时才存在。
                        解析相机节点,
                        crate::light_preview::同步场景灯光,
                        crate::light_preview::同步场景环境,
                        crate::ui_preview::同步界面元素,
                    )
                        .chain(),
                    (
                        crate::animation_runtime::装配模型动画,
                        crate::particle_runtime::推进预览粒子,
                        // 运行时世界：物理步进 + 运行时粒子，只写运行时的 Transform。
                        crate::actions::推进运行时,
                        // 把运行时姿态搬到投影节点上，画面才会跟着物理走。
                        同步运行姿态,
                        // 必须在姿态同步之后：相机要跟的是传播后的节点位置。
                        同步游戏相机,
                    )
                        .chain(),
                ),
            )
            .add_systems(Update, 处理退出按键);
    }
}

/// 建好投影所需的资源、取景相机与运行时世界。
///
/// # 步骤
///
/// 1. 插入投影侧的缓存与状态资源（与 `preview::初始化预览资源` 同一批，
///    但**不含**渲染目标纹理与 egui 纹理注册 —— 这里直接渲染到窗口）；
/// 2. 补一盏填充平行光与环境光：三维材质是受光的，没有光会渲染成纯黑；
///    场景自带灯光时由 `同步场景灯光` 自动把这盏压暗；
/// 3. 取景相机：文档里有 `Camera3d` 就用透视相机并接管其变换与投影参数，
///    否则用二维正交相机按设计分辨率取景；两种情况都只有一台相机，
///    且都带 `预览相机` 标记 —— `同步场景环境` 与 `同步界面元素`
///    按这一份标记找清屏色与 UI 归属，多一台就会让 `.single()` 查询失败；
/// 4. 启动运行时世界与物理（含三维物理），停止运行时整体丢弃的语义不变。
fn 初始化游戏(
    mut 命令: Commands,
    参数: Res<游戏运行参数>,
    编辑器: Res<EditorStateHolder>,
    mut 运行时: ResMut<运行时宿主>,
    mut 物理: ResMut<crate::physics_runtime::物理运行时状态>,
    mut 网格资源: ResMut<Assets<Mesh>>,
) {
    命令.insert_resource(贴图缓存::default());
    命令.insert_resource(crate::tile_terrain::地形缓存::default());
    命令.insert_resource(材质缓存::default());
    命令.insert_resource(crate::particle_runtime::粒子材质表::default());
    命令.insert_resource(预览同步状态::default());
    命令.insert_resource(三维预览资源 {
        四边形: 网格资源.add(Rectangle::new(1.0, 1.0)),
        粒子球: 网格资源.add(Sphere::new(0.5).mesh().uv(8, 6)),
    });
    命令.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 600.0,
        ..default()
    });

    命令.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(6.0, 12.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    let 文档 = &编辑器.状态.会话.文档;
    let 背景 = crate::preview::颜色转_bevy(文档.环境.背景颜色);
    let (设计宽, 设计高) = 参数.设计分辨率;
    if let Some((标识, 相机参数)) = &参数.三维相机 {
        命令.spawn((
            预览相机,
            游戏相机绑定 {
                文档: *标识,
                节点: None,
            },
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(背景),
                ..default()
            },
            Projection::Perspective(PerspectiveProjection {
                fov: 相机参数.视野.to_radians(),
                near: 相机参数.近裁剪,
                far: 相机参数.远裁剪,
                ..default()
            }),
            // 第 0 层：glTF 加载器放进子实体的网格不带渲染层，
            // 与编辑器三维预览同一个理由。
            RenderLayers::from_layers(&[0, 三维渲染层]),
            Transform::from_xyz(8.0, 8.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    } else {
        命令.spawn((
            预览相机,
            Camera2d,
            Camera {
                clear_color: ClearColorConfig::Custom(背景),
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::AutoMin {
                    min_width: 设计宽.max(1) as f32,
                    min_height: 设计高.max(1) as f32,
                },
                ..OrthographicProjection::default_2d()
            }),
            RenderLayers::layer(预览渲染层),
            Transform::from_xyz(0.0, 0.0, 999.0),
        ));
        if 有三维内容(文档) {
            warn!(
                "独立运行：场景里有三维内容但没有「{三维相机组件名}」节点，按二维取景启动，\
                 三维网格与模型不会出现在画面里；请在编辑器里添加一台三维相机。"
            );
        }
    }

    let 结果 = 运行时.启动以设计分辨率(文档, &编辑器.状态.注册表, 参数.设计分辨率);
    match 结果 {
        Ok(()) => {
            物理.开始(文档, &运行时.映射);
            info!(
                "独立游戏已启动：运行时实体 {} 个，界面元素按设计分辨率 {}×{}，{}",
                运行时.运行时实体数(),
                设计宽,
                设计高,
                if 参数.三维相机.is_some() {
                    "三维取景"
                } else {
                    "二维取景"
                }
            );
        }
        Err(错误) => warn!("独立运行：运行时世界未能启动（{错误:?}），画面只播放文档姿态与粒子"),
    }
}

/// 场景里是否存在只有三维相机才看得见的内容（用于缺相机的告警）。
fn 有三维内容(文档: &场景文档) -> bool {
    文档
        .实体表
        .values()
        .any(|实体| 三维内容组件名.iter().any(|名| 实体.有组件(名)))
}

/// 把运行时世界的本地变换搬进投影节点。
///
/// # 为什么需要这一步
///
/// 编辑器里物理的姿态只写进隔离的运行时世界，视口显示文档姿态 ——
/// 那是编辑器的正确行为（编辑时不该被模拟结果改画面）。游戏模式相反：
/// 玩家看的就是模拟结果。两边都不改文档，改变的只是"谁的画面读运行时"。
///
/// 搬的是**本地** `Transform`：运行时世界与投影世界用同一份文档建立同一棵
/// 父子树，父链一致，本地变换因此可以直接对拷，不需要世界空间换算。
///
/// # 只在值真的变了才写
///
/// `Mut<Transform>` 一旦解引用成可变就会盖上变更标记，而变换传播按这个标记
/// 决定要不要重算子树的 `GlobalTransform`。静止的实体每帧照写一遍，等于让
/// 整棵投影树在没有任何变化的情况下每帧重算 —— 大场景里这是纯粹的浪费。
fn 同步运行姿态(
    运行时: Res<运行时宿主>,
    mut 预览: Query<(&预览来源, &mut Transform), With<预览实体>>,
) {
    if !运行时.状态.在运行() {
        return;
    }
    let Some(世界) = 运行时.世界.as_ref() else {
        return;
    };
    for (来源, mut 变换) in &mut 预览 {
        let Some(实体) = 运行时.映射.取运行时(来源.0) else {
            continue;
        };
        let Some(本地) = 世界.get::<Transform>(实体) else {
            continue;
        };
        if *变换 != *本地 {
            *变换 = *本地;
        }
    }
}

/// 把文档相机对到它的投影节点上（缓存缺失或失效时才扫描）。
///
/// 投影实体由 [`crate::preview::同步预览实体`] 建立；那份系统自己按修订号
/// 判断要不要重建，因此这里的缓存判定只看"实体还在不在"，不看修订号。
fn 解析相机节点(
    预览: Query<(Entity, &预览来源), With<预览实体>>,
    mut 相机: Query<&mut 游戏相机绑定>,
) {
    for mut 绑定 in &mut 相机 {
        if 绑定.节点.is_some_and(|实体| 预览.get(实体).is_ok()) {
            continue;
        }
        let 找到 = 预览
            .iter()
            .find(|(_, 来源)| 来源.0 == 绑定.文档)
            .map(|(实体, _)| 实体);
        if 绑定.节点 != 找到 {
            绑定.节点 = 找到;
        }
    }
}

/// 让取景相机跟随文档里的三维相机节点。
///
/// 每帧从投影节点取 `GlobalTransform`（分解后的世界平移/旋转），
/// 因此相机挂在带旋转/缩放的父节点下时依然正确；缩放对相机无意义，不搬。
///
/// 姿态只在真的变了才写入（理由同 [`同步运行姿态`]）；节点暂时不可用时
/// **保持上一帧姿态**——把相机弹回默认位置只会让画面瞬间跳到别处。
/// 视野与裁剪面也不在这里写：文档在游戏模式下只读，这些参数在生成相机时
/// 就定好了，每帧重写只会白白推进投影的变更标记。
fn 同步游戏相机(
    预览: Query<&GlobalTransform, With<预览实体>>,
    mut 相机: Query<(&mut Transform, &游戏相机绑定), With<预览相机>>,
) {
    for (mut 变换, 绑定) in &mut 相机 {
        let Some(实体) = 绑定.节点 else {
            continue;
        };
        let Ok(全局) = 预览.get(实体) else {
            continue;
        };
        let 目标 = Transform::from_translation(全局.translation()).with_rotation(全局.rotation());
        if *变换 != 目标 {
            *变换 = 目标;
        }
    }
}

/// `Esc` 退出游戏。
///
/// 无边框全屏下没有标题栏的关闭按钮，也没有任务栏右键之外的退路；
/// 不给退出键就等于把玩家锁在画面里。
fn 处理退出按键(键盘: Res<ButtonInput<KeyCode>>, mut 退出: MessageWriter<AppExit>) {
    if 键盘.just_pressed(KeyCode::Escape) {
        退出.write(AppExit::Success);
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_core::game_manifest::游戏清单格式版本;
    use scene_doc::document::{场景实体, 场景文档, 组件数据};

    fn 参数表(项: &[&str]) -> Vec<String> {
        项.iter().map(|项| 项.to_string()).collect()
    }

    /// 建一个"能当游戏启动"的工程目录：有 project.ron 且有非空场景。
    fn 建工程(根: &Path, 相机: bool) {
        let 名称 = 根.file_name().unwrap().to_string_lossy().to_string();
        let mut 项目 = editor_core::project::项目描述::创建(根, 名称).expect("创建项目");
        let mut 文档 = 场景文档::新建("主场景");
        let mut 玩家 = 场景实体::新建("玩家");
        玩家.加组件(组件数据::新建("Sprite", Default::default()));
        文档.插入实体(玩家);
        if 相机 {
            let 属性 = std::collections::BTreeMap::from([(
                "视野".to_string(),
                scene_doc::property::属性值::浮点(200.0),
            )]);
            let mut 相机实体 = 场景实体::新建("主相机");
            相机实体.加组件(组件数据::新建("Camera3d", 属性));
            文档.插入实体(相机实体);
        }
        let 场景 = 项目.场景路径("主场景").expect("场景路径");
        项目.保存场景(&场景, &文档).expect("保存场景");
        项目.清单可变().设计分辨率 = (1024, 768);
        项目.保存清单().expect("保存清单");
    }

    #[test]
    fn 不带参数且没有清单时不是游戏启动() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 结果 = 识别启动(&参数表(&[]), 临时.path(), 临时.path()).expect("判定");
        assert!(结果.is_none(), "普通开发启动不应被判成游戏");
    }

    #[test]
    fn game参数指着的目录可以没有清单() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 工程 = 临时.path().join("游戏工程");
        建工程(&工程, false);
        let 请求 = 识别启动(
            &参数表(&["--game", 工程.to_str().unwrap()]),
            临时.path(),
            临时.path(),
        )
        .expect("判定")
        .expect("应当进游戏");
        assert_eq!(请求.项目目录, 工程);
        assert!(请求.清单.is_none(), "目录里没有清单就是 None");
        assert!(请求.全屏, "默认全屏");

        let 窗口版 = 识别启动(
            &参数表(&["--game", 工程.to_str().unwrap(), "--windowed"]),
            临时.path(),
            临时.path(),
        )
        .expect("判定")
        .expect("应当进游戏");
        assert!(!窗口版.全屏, "--windowed 必须覆盖全屏");
    }

    #[test]
    fn 等号形式的game参数与相对路径() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 请求 = 识别启动(&参数表(&["--game=工程"]), 临时.path(), 临时.path())
            .expect("判定")
            .expect("应当进游戏");
        assert_eq!(请求.项目目录, 临时.path().join("工程"));
    }

    #[test]
    fn exe同级的清单不需要任何参数就进游戏() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("Game_Build");
        std::fs::create_dir_all(&包).expect("建包目录");
        let 清单 = 游戏清单 {
            格式版本: 游戏清单格式版本,
            名称: "示例游戏".to_string(),
            主场景: "scenes/主场景.scene.ron".to_string(),
            设计分辨率: (1280, 720),
            全屏: true,
        };
        清单.写入(&包).expect("写入清单");

        let 其他 = tempfile::tempdir().expect("临时目录");
        let 请求 = 识别启动(&参数表(&[]), &包, 其他.path())
            .expect("判定")
            .expect("应当按清单进游戏");
        assert_eq!(请求.项目目录, 包);
        assert!(请求.全屏);
        assert_eq!(请求.清单.as_ref().expect("有清单").名称, "示例游戏");
    }

    #[test]
    fn 清单损坏时报错而不是退回编辑器() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        std::fs::write(临时.path().join(游戏清单文件名), "这不是 RON").expect("写入");
        let 错误 = 识别启动(&参数表(&[]), 临时.path(), 临时.path()).expect_err("必须报错");
        assert!(
            错误.contains("游戏清单"),
            "错误里要能看出是清单的问题：{错误}"
        );
    }

    #[test]
    fn 载入按清单入口打开场景并套用设计分辨率() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("Game_Build");
        建工程(&包, false);
        let 清单 = 游戏清单 {
            格式版本: 游戏清单格式版本,
            名称: "示例游戏".to_string(),
            主场景: "scenes/主场景.scene.ron".to_string(),
            设计分辨率: (1280, 720),
            全屏: false,
        };
        清单.写入(&包).expect("写入清单");

        let 请求 = 识别启动(&参数表(&[]), &包, 包.as_path())
            .expect("判定")
            .expect("应当进游戏");
        let 配置 = 载入(&请求).expect("载入");
        assert_eq!(配置.名称, "示例游戏");
        assert_eq!(配置.设计分辨率, (1280, 720));
        assert!(!配置.全屏);
        assert_eq!(
            配置.状态.项目.as_ref().expect("项目").清单().设计分辨率,
            (1280, 720),
            "清单的设计分辨率必须覆盖进内存清单，界面排版与取景才有同一份真相"
        );
        assert!(!配置.状态.会话.文档.实体表.is_empty());
        assert!(配置.三维相机.is_none(), "场景里没有 Camera3d 时按二维取景");
    }

    #[test]
    fn 载入按文档相机出三维取景参数并夹住非法视野() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("游戏工程");
        建工程(&包, true);
        let 请求 = 启动请求 {
            项目目录: 包.clone(),
            清单: None,
            全屏: true,
        };
        let 配置 = 载入(&请求).expect("载入");
        let (标识, 参数) = 配置.三维相机.expect("场景相机必须被认领");
        assert_eq!(参数.视野, 179.0, "200° 的透视视野要按注册表的边界夹住");
        assert!(
            配置
                .状态
                .会话
                .文档
                .取(标识)
                .expect("标识仍在文档里")
                .有组件("Camera3d")
        );
    }

    #[test]
    fn 场景文件缺失时给出可读错误() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("游戏工程");
        建工程(&包, false);
        let 请求 = 启动请求 {
            项目目录: 包,
            清单: Some(游戏清单 {
                格式版本: 游戏清单格式版本,
                名称: String::new(),
                主场景: "scenes/不存在的场景.scene.ron".to_string(),
                设计分辨率: (1280, 720),
                全屏: true,
            }),
            全屏: true,
        };
        let 错误 = 载入(&请求).expect_err("缺失主场景必须失败");
        assert!(错误.contains("不存在的场景"), "错误里要点名场景：{错误}");
    }

    #[test]
    fn 工程里没有场景时拒绝启动() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("空工程");
        editor_core::project::项目描述::创建(&包, "空工程").expect("创建项目");
        let 请求 = 启动请求 {
            项目目录: 包,
            清单: None,
            全屏: true,
        };
        let 错误 = 载入(&请求).expect_err("没有入口必须失败");
        assert!(
            错误.contains("没有场景文件"),
            "错误要说明缺的是入口：{错误}"
        );
    }

    /// 游戏身份的装配面：投影 + 运行时 + 取景资源都在，egui 与编辑器插件都不在。
    ///
    /// 用裸 `App::new()` 装内核：`DefaultPlugins` 会在构造时创建 winit 事件循环，
    /// 而测试跑在非主线程，那会 panic（见 [`装配内核`]）。
    #[test]
    fn 游戏应用装配里没有界面层() {
        let 临时 = tempfile::tempdir().expect("临时目录");
        let 包 = 临时.path().join("装配工程");
        建工程(&包, false);
        let 请求 = 启动请求 {
            项目目录: 包,
            清单: None,
            全屏: false,
        };
        let mut 应用 = App::new();
        装配内核(&mut 应用, 载入(&请求).expect("载入"));

        assert!(
            应用.is_plugin_added::<独立游戏插件>(),
            "投影与运行时由本插件装配"
        );
        assert!(
            !应用.is_plugin_added::<bevy_egui::EguiPlugin>(),
            "游戏身份不得带 egui"
        );
        assert!(
            !应用.is_plugin_added::<crate::plugin::BevyStudio插件>(),
            "游戏身份不得带编辑器插件"
        );
        let 世界 = 应用.world();
        assert!(世界.get_resource::<EditorStateHolder>().is_some());
        assert!(世界.get_resource::<游戏运行参数>().is_some());
        assert!(世界.get_resource::<运行时宿主>().is_some());
    }

    /// 这一帧有多少个 `Transform` 被标记过变更（新加实体也算）。
    #[derive(Resource, Default)]
    struct 变换变更次数(usize);

    fn 统计变换变更(
        变化: Query<(), Changed<Transform>>, mut 次数: ResMut<变换变更次数>
    ) {
        次数.0 = 变化.iter().count();
    }

    /// 静止的运行时实体不该每帧重写投影变换。
    ///
    /// 断言用 `Changed<Transform>` 而不是比较数值：`Mut<Transform>` 只要解引用
    /// 成可变就会盖变更标记，而变化标记正是"整棵投影树重算变换传播"的开关。
    /// 数值相同但标记被盖上，照样是每帧白算。
    #[test]
    fn 静止的运行时实体不会每帧重写投影变换() {
        let 标识 = 实体标识::新建();
        let mut 运行时世界 = World::new();
        let 运行时实体 = 运行时世界.spawn(Transform::from_xyz(1.0, 2.0, 3.0)).id();
        let mut 宿主 = 运行时宿主::default();
        宿主.状态 = crate::scene_bridge::运行状态::运行中;
        宿主.映射.插入(标识, 运行时实体);
        宿主.世界 = Some(运行时世界);

        let mut 应用 = App::new();
        应用
            .insert_resource(宿主)
            .init_resource::<变换变更次数>()
            .add_systems(Update, (同步运行姿态, 统计变换变更).chain());
        let 预览节点 = 应用
            .world_mut()
            .spawn((预览实体, 预览来源(标识), Transform::default()))
            .id();

        应用.update();
        assert_eq!(
            应用
                .world()
                .get::<Transform>(预览节点)
                .expect("投影节点")
                .translation,
            Vec3::new(1.0, 2.0, 3.0),
            "运行时姿态要搬到投影节点上"
        );
        assert_eq!(
            应用.world().resource::<变换变更次数>().0,
            1,
            "首帧要真的写过一次"
        );

        应用.update();
        assert_eq!(
            应用.world().resource::<变换变更次数>().0,
            0,
            "值没变就不该写：每次写都会让这棵子树重算变换传播"
        );
    }

    /// 相机跟随文档相机节点；节点消失时保持姿态，节点回来后重新解析。
    #[test]
    fn 游戏相机跟随文档相机节点并保持姿态() {
        let 标识 = 实体标识::新建();
        let mut 应用 = App::new();
        应用.add_systems(Update, (解析相机节点, 同步游戏相机).chain());
        let 相机 = 应用
            .world_mut()
            .spawn((
                预览相机,
                游戏相机绑定 {
                    文档: 标识,
                    节点: None,
                },
                Transform::default(),
            ))
            .id();
        let 节点 = 应用
            .world_mut()
            .spawn((
                预览实体,
                预览来源(标识),
                Transform::from_xyz(3.0, 4.0, 5.0),
                GlobalTransform::from_xyz(3.0, 4.0, 5.0),
            ))
            .id();

        应用.update();
        assert_eq!(
            应用
                .world()
                .entity(相机)
                .get::<游戏相机绑定>()
                .expect("绑定还在")
                .节点,
            Some(节点),
            "解析一次就该记住投影节点，之后不再遍历全部预览实体"
        );
        assert_eq!(
            应用
                .world()
                .get::<Transform>(相机)
                .expect("相机变换")
                .translation,
            Vec3::new(3.0, 4.0, 5.0)
        );

        应用.world_mut().despawn(节点);
        应用.update();
        assert_eq!(
            应用
                .world()
                .entity(相机)
                .get::<游戏相机绑定>()
                .expect("绑定还在")
                .节点,
            None,
            "节点没了，缓存必须失效"
        );
        assert_eq!(
            应用
                .world()
                .get::<Transform>(相机)
                .expect("相机变换")
                .translation,
            Vec3::new(3.0, 4.0, 5.0),
            "节点不可用时保持上一帧姿态，不要弹回原点"
        );
    }
}
