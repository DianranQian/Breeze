//! Play-in-Editor 最小实时渲染原型（R13 第一阶段）。
//!
//! # 这个文件要回答的问题
//!
//! **微风能不能以可维护的方式，把一个独立运行的 Bevy 游戏世界，实时渲染到
//! 编辑器内部？** 而不是"能不能再开一个窗口"。
//!
//! # 为什么是这个架构
//!
//! 原型**没有**新建第二个 `App`。原因是实测确认过的三条约束：
//!
//! 1. 一个进程里起第二个 `App` 会再起一个 winit 事件循环；
//! 2. Bevy 的渲染提取（`RenderApp`）只抽主 `World` 里的实体——
//!    另建一个 `World` 塞相机与精灵，不会被渲染，画面永远是空的；
//! 3. 编辑器已经有成熟的"渲染到纹理 + egui 显示"链路（`preview.rs`），
//!    证明这条路在本仓库里是可用的。
//!
//! 因此原型的做法是：**运行时实体就活在编辑器 App 的 `World` 里**，
//! 但在可见性上完全隔离：
//!
//! ```text
//!   运行时实体（PIE层） ──▶ PIE 相机（RenderTarget::Image）
//!                              │
//!                              ▼  一张独立的 Image 资产
//!                          egui 窗口（SizedTexture）
//! ```
//!
//! 隔离手段是 `RenderLayers`：PIE 相机只看 `运行层`，编辑器预览相机只看
//! `预览渲染层`，界面相机只看默认层。三方互不干扰。
//!
//! # 与"编辑态场景文档"的隔离
//!
//! 这一条是**结构保证**，不靠自觉：编辑器的场景是 `场景文档`（RON 数据），
//! 运行时是 ECS 实体，两者之间只有 `scene_bridge::构建世界` 单向转换。
//! 原型里没有任何代码回写文档；集成测试用"运行前后文档快照相等"锁住它。
//!
//! # 这个原型**不**包含什么
//!
//! 没有暂停、没有鼠标捕获、没有输入焦点管理、没有接运行时调试协议。
//! 它只证明"实时画面真的在编辑器里、真的在动、真的能被输入驱动"。

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy_egui::{EguiContexts, EguiTextureHandle, EguiUserTextures};

/// 原型验收用的环境变量：设置后启动即自动开跑。
pub const 原型启动环境变量: &str = "BEVY_STUDIO_PIE";

/// 验收脚本：`restart` 表示第 120 帧停止、第 150 帧重启。
///
/// 用环境变量而不是"测试直接调函数"：`启动_实现` / `停止_实现` 是同一份
/// 代码，环境变量只是触发时机不同。这样"停止后重启仍可用"是被真实
/// 进程验证的，而不是被单元测试模拟的。
pub const 原型场景环境变量: &str = "BEVY_STUDIO_PIE_SCENARIO";

/// 运行时画面所在的渲染层。
///
/// 取 7 而不是 1：`preview.rs` 用的是 `预览渲染层`（0/1 一类的低位），
/// PIE 用独立高位层，将来再加"运行时 3D"层也不会撞。
pub const 运行层: usize = 7;

/// 渲染目标尺寸。
pub const 画面尺寸: (u32, u32) = (480, 270);

/// 方块的移动速度（世界单位/秒）。
const 方块速度: f32 = 9.0;

/// 方块来回的左右边界。
const 左边界: f32 = -6.0;
const 右边界: f32 = 6.0;

/// PIE 相机标记。
#[derive(Component)]
pub struct Pie相机;

/// PIE 运行世界里的方块（原型里的"游戏对象"）。
#[derive(Component)]
pub struct Pie方块;

/// PIE 运行世界里的地面（静态，用来证明画面不是纯色背景）。
#[derive(Component)]
pub struct Pie地面;

/// 原型演示用的输入：**真实键盘**（WASD/方向键）。
///
/// 不是"模拟输入"：读的是 `ButtonInput<KeyCode>`，与任何 Bevy 游戏读输入的
/// 方式相同。验收时用系统级按键注入驱动它。
#[derive(Resource, Default)]
pub struct Pie输入 {
    /// 本帧是否按住左移。
    pub 左: bool,
    /// 本帧是否按住右移。
    pub 右: bool,
    /// 用方向键加速的倍率（按住 Shift）。
    pub 加速: bool,
    /// 鼠标在 PIE 画面里的水平位置（-1 左、+1 右）。
    ///
    /// `None` 表示指针不在画面上。按住左键时方块会朝这个位置移动 ——
    /// 这是原型里第二条输入链路，用来被**系统级鼠标注入**驱动
    /// （键盘注入在当前验收工具里不可用，见模块文档）。
    pub 指针目标: Option<f32>,
}

/// 原型状态。
#[derive(Resource)]
pub struct Pie原型状态 {
    /// 是否正在运行。
    pub 运行中: bool,
    /// 已推进的帧数。
    pub 帧: u64,
    /// 方块当前世界 x。
    pub 方块x: f32,
    /// 方块当前移动方向（+1 向右，-1 向左）。
    pub 方块方向: f32,
    /// 本次运行的启动时刻。
    pub 启动时刻: Option<std::time::Instant>,
    /// 最近一次推进的耗时（毫秒，用于如实报告帧率）。
    pub 上一帧毫秒: f32,
    /// 最近一次启动耗时（毫秒）。
    pub 上次启动毫秒: f32,
    /// 被指针输入驱动的帧数（证明输入链路真的在工作，而不是只有一帧）。
    pub 指针驱动帧数: u64,
    /// 应用帧计数（**停止时也继续增长**）。
    ///
    /// 验收脚本不能按"运行帧"触发停止/重启：停止后运行帧不再增长，
    /// 重启分支就永远等不到自己的帧号。
    pub 场景帧: u64,
    /// 启动次数（重启后 +1，用于证明"再次启动可用"）。
    pub 启动次数: u32,
    /// 渲染目标图片句柄。
    pub 图片: Option<Handle<Image>>,
    /// 注册到 egui 的纹理 id。
    pub 纹理: Option<egui::TextureId>,
    /// 是否在 egui 里显示原型窗口。
    pub 显示窗口: bool,
    /// 是否还需要做一次性初始化（见 `自动启动` 的注释）。
    pub 待初始化: bool,
    /// 上一次记录的帧号（trace 用，避免每帧刷日志）。
    pub 上次记录帧: u64,
}

impl Default for Pie原型状态 {
    fn default() -> Self {
        Self {
            运行中: false,
            帧: 0,
            方块x: 0.0,
            方块方向: 1.0,
            启动时刻: None,
            上一帧毫秒: 0.0,
            上次启动毫秒: 0.0,
            指针驱动帧数: 0,
            场景帧: 0,
            启动次数: 0,
            图片: None,
            纹理: None,
            显示窗口: false,
            待初始化: false,
            上次记录帧: 0,
        }
    }
}

/// 原型插件。
pub struct Pie原型插件;

impl Plugin for Pie原型插件 {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pie原型状态>()
            .init_resource::<Pie输入>()
            // 纹理注册在 Startup：与 `preview.rs` 创建预览纹理同批，
            // `EguiUserTextures::add_image` 只登记资源与 id，不碰字体。
            .add_systems(Startup, 自动启动)
            .add_systems(
                Update,
                (同步输入, 推进原型, 按场景停止并重启, 记录连续帧证据).chain(),
            )
            // 原型窗口必须在 egui 的 context pass 里画：那里 `Context`
            // 已经跑过、字体已就绪。放在 `Update` 里会 panic：
            // "No fonts available until first call to Context::run()"。
            .add_systems(bevy_egui::EguiPrimaryContextPass, 绘制原型窗口);
    }
}

/// 启动原型：创建渲染目标与 egui 纹理，并（按环境变量）立刻开跑。
///
/// # 为什么可以放在 `Startup`
///
/// `EguiUserTextures::add_image` 只是往一个 `HashMap` 里登记
/// `AssetId → egui::TextureId`，**不碰字体**，因此在
/// `预览渲染目标` 的同一批 Startup 系统里调用是安全的。
///
/// 真正会 panic 的是"在 egui 的 `Context::run()` 之前画 UI"——
/// 那条路径见 `绘制原型窗口` 的调度（`EguiPrimaryContextPass`）。
fn 自动启动(
    mut 状态: ResMut<Pie原型状态>,
    mut 图片资源: ResMut<Assets<Image>>,
    mut egui纹理: ResMut<EguiUserTextures>,
    mut 命令: Commands,
) {
    // 纹理与"这次运行"无关：创建一次、重启复用。反复申请/释放 GPU 纹理
    // 只会把显存碎片化。
    let 图片 = 新建渲染目标图片();
    let 句柄 = 图片资源.add(图片);
    let 纹理 = egui纹理.add_image(EguiTextureHandle::Strong(句柄.clone()));
    状态.图片 = Some(句柄);
    状态.纹理 = Some(纹理);
    状态.显示窗口 = std::env::var(原型启动环境变量).is_ok();

    if 状态.显示窗口 {
        启动_实现(&mut 命令, &mut 状态, None);
    }
}

/// 创建渲染目标图片（`Rgba8UnormSrgb`，与 `preview.rs` 一致，理由见那里）。
fn 新建渲染目标图片() -> Image {
    let 尺寸 = Extent3d {
        width: 画面尺寸.0,
        height: 画面尺寸.1,
        ..default()
    };
    let mut 图片 = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("bevy_studio_pie_渲染目标"),
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
    图片
}

/// 启动（或重启）一次运行。
///
/// `旧根` 为 `Some` 时是**重启**：先清掉上一次运行的实体与相机，再重新生成。
pub fn 启动_实现(命令: &mut Commands, 状态: &mut Pie原型状态, 旧根: Option<Entity>) {
    let 开始 = std::time::Instant::now();
    if let Some(根) = 旧根 {
        命令.entity(根).despawn();
    }

    let 层 = RenderLayers::layer(运行层);
    let 图片 = 状态.图片.clone().expect("渲染目标必须已创建");

    // ① 运行时相机：只看运行层，渲染到独立纹理。
    命令.spawn((
        Pie相机,
        Camera2d,
        Camera {
            // 在 `preview.rs` 的预览相机（order -1）与界面相机（order 0）
            // 之后渲染：PIE 画面不参与编辑态视口，顺序只要确定即可。
            order: 1,
            // 清成不透明深色：原型要能证明"画面是这张纹理渲染出来的"，
            // 透明背景会让它看起来像叠加层。
            clear_color: ClearColorConfig::Custom(Color::srgb(0.06, 0.07, 0.09)),
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: 16.0,
                height: 9.0,
            },
            ..OrthographicProjection::default_2d()
        }),
        RenderTarget::Image(图片.into()),
        层.clone(),
    ));

    // ② 地面：一条宽而扁的静态矩形（证明画面里有非方块内容）。
    命令.spawn((
        Pie地面,
        Sprite::from_color(Color::srgb(0.16, 0.20, 0.26), Vec2::new(18.0, 0.35)),
        Transform::from_xyz(0.0, -3.0, 0.0),
        层.clone(),
    ));

    // ③ 移动方块：原型里的"游戏对象"。
    命令.spawn((
        Pie方块,
        Sprite::from_color(Color::srgb(0.95, 0.55, 0.20), Vec2::new(1.0, 1.0)),
        Transform::from_xyz(-6.0, -2.2, 1.0),
        层,
    ));

    状态.运行中 = true;
    状态.帧 = 0;
    状态.方块x = -6.0;
    状态.方块方向 = 1.0;
    状态.指针驱动帧数 = 0;
    状态.启动时刻 = Some(开始);
    状态.启动次数 += 1;
    状态.上次启动毫秒 = 开始.elapsed().as_secs_f32() * 1000.0;
    info!(
        "PIE 原型已启动（第 {} 次）：{}×{} 渲染目标，层 {}",
        状态.启动次数, 画面尺寸.0, 画面尺寸.1, 运行层
    );
}

/// 停止运行：销毁本次运行生成的全部实体（相机、地面、方块）。
///
/// 纹理**不**销毁：它属于原型本身，不随一次运行创建/销毁，
/// 反复申请释放 GPU 纹理没有意义（见 `自动启动` 的注释）。
pub fn 停止_实现(
    命令: &mut Commands,
    状态: &mut Pie原型状态,
    相机: Option<Entity>,
    地面: Option<Entity>,
    方块: Option<Entity>,
) {
    for 实体 in [相机, 地面, 方块].into_iter().flatten() {
        命令.entity(实体).despawn();
    }
    状态.运行中 = false;
    状态.启动时刻 = None;
    info!("PIE 原型已停止（本次运行 {} 帧）", 状态.帧);
}

/// 验收脚本：按帧号停止并重启，验证"停止 → 再次启动"链路。
///
/// 只有在 `BEVY_STUDIO_PIE_SCENARIO=restart` 时才动作。
fn 按场景停止并重启(
    mut 状态: ResMut<Pie原型状态>,
    mut 命令: Commands,
    相机: Query<Entity, With<Pie相机>>,
    地面: Query<Entity, With<Pie地面>>,
    方块: Query<Entity, With<Pie方块>>,
) {
    if std::env::var(原型场景环境变量).as_deref() != Ok("restart") {
        return;
    }
    状态.场景帧 += 1;
    match 状态.场景帧 {
        120 => {
            停止_实现(
                &mut 命令,
                &mut 状态,
                相机.iter().next(),
                地面.iter().next(),
                方块.iter().next(),
            );
        }
        150 => {
            启动_实现(&mut 命令, &mut 状态, None);
            info!("PIE 原型已重启（重启后第 {} 次启动）", 状态.启动次数);
        }
        _ => {}
    }
}

/// 读取真实键盘输入。
fn 同步输入(
    键盘: Res<ButtonInput<KeyCode>>,
    mut 输入: ResMut<Pie输入>,
    状态: Res<Pie原型状态>,
) {
    if !状态.运行中 {
        输入.左 = false;
        输入.右 = false;
        输入.加速 = false;
        输入.指针目标 = None;
        return;
    }
    输入.左 = 键盘.pressed(KeyCode::KeyA) || 键盘.pressed(KeyCode::ArrowLeft);
    输入.右 = 键盘.pressed(KeyCode::KeyD) || 键盘.pressed(KeyCode::ArrowRight);
    输入.加速 = 键盘.pressed(KeyCode::ShiftLeft) || 键盘.pressed(KeyCode::ShiftRight);
}

/// 推进一步原型世界（纯逻辑，不碰 ECS）。
///
/// 抽出来是为了**可测**：`推进原型` 只是"读 Time → 调本函数 → 写 Transform"，
/// 而真正的运动规则在这里，能用单元测试覆盖到边界与输入优先级。
pub fn 步进(状态: &mut Pie原型状态, 输入: &Pie输入, dt: f32) {
    状态.帧 += 1;
    // 夹住 dt：窗口被拖动/断点暂停后 dt 会很大，不夹会让方块瞬移出画面。
    let dt = dt.clamp(0.0, 0.1);
    let 倍率 = if 输入.加速 { 2.5 } else { 1.0 };

    // 输入链路 2：鼠标按住时朝指针位置移动。
    //
    // 用"直接跟随"而不是"施加力"：原型要证明的是**输入能改变画面**，
    // 而不是物理手感。目标是世界坐标里的固定点，按住时方块会明显偏离
    // 自动来回走的轨迹，日志里一眼能看出来。
    if let Some(目标) = 输入.指针目标 {
        状态.指针驱动帧数 += 1;
        let 目标x = 目标.clamp(-1.0, 1.0) * 右边界;
        let 差 = 目标x - 状态.方块x;
        if 差.abs() > 0.02 {
            状态.方块x += 差.signum() * (方块速度 * 倍率 * dt).min(差.abs());
            状态.方块方向 = 差.signum();
        }
        return;
    }

    // 输入链路 1：键盘。没有按键时沿当前方向继续，撞到边界就掉头。
    let 方向 = if 输入.左 {
        -1.0
    } else if 输入.右 {
        1.0
    } else {
        状态.方块方向
    };
    let mut 新x = 状态.方块x + 方向 * 方块速度 * 倍率 * dt;
    let mut 新方向 = 方向;
    if 新x >= 右边界 {
        新x = 右边界;
        新方向 = -1.0;
    } else if 新x <= 左边界 {
        新x = 左边界;
        新方向 = 1.0;
    }
    状态.方块x = 新x;
    状态.方块方向 = if 输入.左 || 输入.右 {
        方向
    } else {
        新方向
    };
}

/// 推进原型世界：把 `步进` 的结果写进 ECS。
fn 推进原型(
    mut 状态: ResMut<Pie原型状态>,
    输入: Res<Pie输入>,
    时间: Res<Time>,
    mut 查询: Query<&mut Transform, With<Pie方块>>,
) {
    if !状态.运行中 {
        return;
    }
    let 开始 = std::time::Instant::now();
    步进(&mut 状态, &输入, 时间.delta_secs());
    for mut 变换 in &mut 查询 {
        变换.translation.x = 状态.方块x;
    }
    状态.上一帧毫秒 = 开始.elapsed().as_secs_f32() * 1000.0;
}

/// 连续帧证据：每 30 帧写一条带帧号与方块位置的信息级日志。
///
/// # 为什么日志能证明"实时"
///
/// 截图只能证明某一时刻。日志里出现 `帧 30 x=-2.13`、`帧 60 x=1.77`…
/// 是同一个进程里连续推进的序列，配合"同一会话内两帧截图不同"，
/// 才构成"画面在动"的证据链。
fn 记录连续帧证据(状态: Res<Pie原型状态>, 输入: Res<Pie输入>) {
    if !状态.运行中 {
        return;
    }
    if !状态.帧.is_multiple_of(30) || 状态.帧 == 状态.上次记录帧 {
        return;
    }
    info!(
        "PIE 帧 {} · 方块 x={:.3} · 指针 {:?} · 输入驱动 {} 帧 · 推进 {:.3} ms",
        状态.帧, 状态.方块x, 输入.指针目标, 状态.指针驱动帧数, 状态.上一帧毫秒
    );
}

/// 原型窗口：把渲染目标当成普通 egui 纹理显示，并提供启动/停止/重启。
fn 绘制原型窗口(
    mut 上下文: EguiContexts,
    mut 状态: ResMut<Pie原型状态>,
    mut 输入: ResMut<Pie输入>,
    mut 命令: Commands,
    相机: Query<Entity, With<Pie相机>>,
    地面: Query<Entity, With<Pie地面>>,
    方块: Query<Entity, With<Pie方块>>,
) {
    if !状态.显示窗口 {
        return;
    }
    let Ok(egui上下文) = 上下文.ctx_mut() else {
        return;
    };
    let mut 打开 = true;
    let 运行中 = 状态.运行中;
    let mut 请求启动 = false;
    let mut 请求停止 = false;
    let mut 请求重启 = false;
    let 帧 = 状态.帧;
    let 方块x = 状态.方块x;
    let 启动次数 = 状态.启动次数;
    let 上一帧毫秒 = 状态.上一帧毫秒;
    let 纹理 = 状态.纹理;

    egui::Window::new("PIE 原型（实时渲染实验）")
        .open(&mut 打开)
        .default_pos([640.0, 210.0])
        .default_width(520.0)
        .resizable(true)
        .show(egui上下文, |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(!运行中, egui::Button::new("启动")).clicked() {
                    请求启动 = true;
                }
                if ui.add_enabled(运行中, egui::Button::new("停止")).clicked() {
                    请求停止 = true;
                }
                if ui.add_enabled(运行中, egui::Button::new("重启")).clicked() {
                    请求重启 = true;
                }
                ui.label(format!("帧 {帧} · 方块 x={方块x:.3} · 启动 {启动次数} 次"));
            });
            ui.label(
                egui::RichText::new(format!(
                    "推进 {上一帧毫秒:.3} ms · A/D 或 ←/→ 改变方向（需窗口聚焦）"
                ))
                .small(),
            );
            ui.separator();
            match 纹理 {
                Some(纹理) => {
                    let 尺寸 = egui::vec2(画面尺寸.0 as f32, 画面尺寸.1 as f32);
                    let 响应 = ui.add(
                        egui::Image::new(egui::load::SizedTexture::new(纹理, 尺寸))
                            .fit_to_exact_size(尺寸)
                            .sense(egui::Sense::click_and_drag()),
                    );
                    // 第二条输入链路：按住左键时，方块朝指针在画面里的
                    // 水平位置移动。`gui_probe drag` 能注入真实鼠标事件，
                    // 因此这条链路是可被系统级验证的。
                    let 指针 = 响应.interact_pointer_pos();
                    let 按住 = 响应.dragged() || 响应.clicked();
                    if let (true, Some(位置)) = (按住, 指针) {
                        let t = ((位置.x - 响应.rect.left()) / 响应.rect.width()) * 2.0 - 1.0;
                        输入.指针目标 = Some(t.clamp(-1.0, 1.0));
                    } else {
                        输入.指针目标 = None;
                    }
                }
                None => {
                    ui.label("纹理尚未注册");
                }
            }
        });

    if !打开 {
        状态.显示窗口 = false;
    }
    if 请求停止 || 请求重启 {
        停止_实现(
            &mut 命令,
            &mut 状态,
            相机.iter().next(),
            地面.iter().next(),
            方块.iter().next(),
        );
    }
    if 请求启动 || 请求重启 {
        启动_实现(&mut 命令, &mut 状态, None);
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 状态() -> Pie原型状态 {
        Pie原型状态 {
            运行中: true,
            方块方向: 1.0,
            ..Pie原型状态::default()
        }
    }

    #[test]
    fn 自动移动会撞到右边界后掉头() {
        let mut 状态 = 状态();
        let 输入 = Pie输入::default();
        // 9 单位/秒、1 秒一步：从 0 走到 6 需要约 0.67 秒。
        for _ in 0..60 {
            步进(&mut 状态, &输入, 1.0 / 60.0);
        }
        assert!(状态.方块x > 0.0, "默认应当向右移动：{}", 状态.方块x);
        // 再走 3 秒，必然撞到右边界后面朝左。
        for _ in 0..180 {
            步进(&mut 状态, &输入, 1.0 / 60.0);
        }
        assert!(状态.方块x <= 右边界 + 1e-3);
        assert_eq!(状态.方块方向, -1.0, "撞到右边界后应当掉头");
    }

    #[test]
    fn 指针目标优先于自动移动() {
        let mut 状态 = 状态();
        let 输入 = Pie输入 {
            指针目标: Some(-1.0),
            ..Pie输入::default()
        };
        步进(&mut 状态, &输入, 0.5);
        assert!(状态.方块x < 0.0, "指针在左侧时方块应当向左：{}", 状态.方块x);
        assert_eq!(状态.指针驱动帧数, 1);
    }

    #[test]
    fn 键盘可以覆盖自动方向() {
        let mut 状态 = 状态();
        let 输入 = Pie输入 {
            左: true,
            ..Pie输入::default()
        };
        步进(&mut 状态, &输入, 0.5);
        assert!(状态.方块x < 0.0, "按住左键应当向左：{}", 状态.方块x);
        assert_eq!(状态.帧, 1, "每步推进一帧");
    }

    #[test]
    fn 巨大步长被夹住不会让方块瞬移出画面() {
        let mut 状态 = 状态();
        let 输入 = Pie输入::default();
        // 模拟断点/拖动窗口后的巨大 dt。
        步进(&mut 状态, &输入, 30.0);
        assert!(
            (左边界..=右边界).contains(&状态.方块x),
            "dt 未被夹住会让方块离开边界：{}",
            状态.方块x
        );
    }

    #[test]
    fn 加速键提高移动速度() {
        let mut 甲 = 状态();
        let mut 乙 = 状态();
        let 普通 = Pie输入 {
            右: true,
            ..Pie输入::default()
        };
        let 加速 = Pie输入 {
            右: true,
            加速: true,
            ..Pie输入::default()
        };
        步进(&mut 甲, &普通, 0.1);
        步进(&mut 乙, &加速, 0.1);
        assert!(
            乙.方块x > 甲.方块x,
            "加速应当更快：{} vs {}",
            乙.方块x,
            甲.方块x
        );
    }
}
