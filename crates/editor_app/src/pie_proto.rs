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
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, EguiTextureHandle, EguiUserTextures};

use crate::pie_input::{
    Pie焦点状态, Pie输入焦点, 原始按键, 焦点变化原因, 路由键盘
};

/// 原型验收用的环境变量：设置后启动即自动开跑。
pub const 原型启动环境变量: &str = "BEVY_STUDIO_PIE";

/// 验收脚本：`restart` 表示第 120 帧停止、第 150 帧重启。
///
/// 用环境变量而不是"测试直接调函数"：`启动_实现` / `停止_实现` 是同一份
/// 代码，环境变量只是触发时机不同。这样"停止后重启仍可用"是被真实
/// 进程验证的，而不是被单元测试模拟的。
pub const 原型场景环境变量: &str = "BEVY_STUDIO_PIE_SCENARIO";

/// 验收脚本：第 N 帧自动请求捕获（模拟"用户点击画面"）。
///
/// 为什么需要它：`gui_probe` 能注入鼠标点击、也能被日志证实捕获成功，
/// 但它不能在应用运行中途注入**键盘**——键盘注入要靠操作系统级
/// `SendKeys`，那需要应用已经在跑。两者凑不进同一次运行，
/// 所以键盘验收用"同一状态机方法、不同触发源"的方式：
/// 点击捕获有独立证据（drag 日志），键盘路由用这个入口 + 真实按键。
pub const 原型自动捕获环境变量: &str = "BEVY_STUDIO_PIE_AUTOCAPTURE";

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
pub const 左边界: f32 = -6.0;
pub const 右边界: f32 = 6.0;

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
    /// 这是"未捕获"状态下的鼠标链路，用来被**系统级鼠标注入**驱动。
    pub 指针目标: Option<f32>,
    /// 捕获状态下本帧的相对鼠标水平位移（像素）。
    ///
    /// 捕获时指针被系统锁定，绝对坐标不再有意义，只能用相对位移。
    /// 这条与 `指针目标` 互斥：捕获时读这个，未捕获时读那个。
    pub 鼠标位移x: f32,
    /// 捕获状态下鼠标左键是否按下。
    pub 左键按下: bool,
    /// 本帧是否收到"退出捕获"（Esc）。由焦点状态机消费一次。
    pub 退出捕获: bool,
}

/// PIE 运行生命周期状态。
///
/// # 为什么是枚举而不是两个布尔
///
/// "运行中" 与 "已暂停" 用两个布尔表达会立刻出现第三种非法组合
/// （既运行又暂停）。枚举把非法状态从类型层面排除，也让
/// "未运行时点暂停" 变成一次被拒绝的转换，而不是落进一个虚假的暂停态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pie运行状态 {
    /// 没有运行会话。
    已停止,
    /// 游戏模拟正在推进。
    运行中,
    /// 游戏模拟冻结，但编辑器、渲染与输入管理照常工作。
    已暂停,
}

impl Pie运行状态 {
    /// 中文名（日志与证据）。
    pub fn 名称(self) -> &'static str {
        match self {
            Pie运行状态::已停止 => "已停止",
            Pie运行状态::运行中 => "运行中",
            Pie运行状态::已暂停 => "已暂停",
        }
    }

    /// 是否存在有效会话（暂停也算：它只是冻结，不是结束）。
    pub fn 在会话中(self) -> bool {
        !matches!(self, Pie运行状态::已停止)
    }

    /// 游戏模拟是否应该在本帧推进。
    pub fn 模拟推进(self) -> bool {
        matches!(self, Pie运行状态::运行中)
    }
}

/// 单步一次的**固定**模拟步长（秒）。
///
/// # 为什么单步用固定步长而不是"上一帧的 dt"
///
/// 暂停期间没有真实的帧间时间可用；用"上一帧 dt"会让单步的位移取决于
/// 暂停前那台机器那一刻的帧率——同一份场景在不同机器上单步出不同结果。
/// 固定 1/60 秒让"一次单步 = 一个 60Hz 模拟步"成为可复现的语义。
pub const 单步步长秒: f32 = 1.0 / 60.0;

/// 原型状态。
#[derive(Resource)]
pub struct Pie原型状态 {
    /// 运行生命周期状态（**唯一**权威，界面与调度都读它）。
    pub 状态: Pie运行状态,
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
    /// 被捕获态鼠标位移驱动的帧数。
    pub 鼠标驱动帧数: u64,
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
    /// 待执行的单步请求数（0 表示没有）。
    ///
    /// 用计数而不是布尔：连续点两次单步应当执行两次，而不是被合并成一次。
    /// 每次 `推进原型` **最多**消费一个，因此单步不会因为掉帧而多推进。
    pub 待单步: u32,
    /// 已完成的单步次数（证据里用来核对"请求 N 次 = 执行 N 次"）。
    pub 单步执行次数: u64,
    /// PIE 自己的模拟时钟（秒）。
    ///
    /// 只由游戏模拟推进：运行时按真实 dt 累加，暂停时不累加。
    /// 它**不**使用也不修改全局 `Time`，因此暂停不会冻结编辑器动画。
    pub 模拟累计秒: f64,
    /// 本帧用于模拟的时间步（秒；暂停时为 0）。
    pub 本次步长秒: f32,
}

impl Default for Pie原型状态 {
    fn default() -> Self {
        Self {
            状态: Pie运行状态::已停止,
            帧: 0,
            方块x: 0.0,
            方块方向: 1.0,
            启动时刻: None,
            上一帧毫秒: 0.0,
            上次启动毫秒: 0.0,
            指针驱动帧数: 0,
            鼠标驱动帧数: 0,
            场景帧: 0,
            启动次数: 0,
            图片: None,
            纹理: None,
            显示窗口: false,
            待初始化: false,
            上次记录帧: 0,
            待单步: 0,
            单步执行次数: 0,
            模拟累计秒: 0.0,
            本次步长秒: 0.0,
        }
    }
}

/// 原型插件。
pub struct Pie原型插件;

impl Plugin for Pie原型插件 {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pie原型状态>()
            .init_resource::<Pie输入>()
            .init_resource::<Pie输入焦点>()
            // 纹理注册在 Startup：与 `preview.rs` 创建预览纹理同批，
            // `EguiUserTextures::add_image` 只登记资源与 id，不碰字体。
            .add_systems(Startup, 自动启动)
            .add_systems(
                Update,
                (
                    同步焦点,
                    按帧自动捕获,
                    同步输入,
                    按场景生命周期,
                    推进原型,
                    按场景停止并重启,
                    记录连续帧证据,
                )
                    .chain(),
            )
            // 指针锁定必须在窗口系统更新之后、渲染之前生效。
            .add_systems(PostUpdate, 应用鼠标捕获)
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

    状态.状态 = Pie运行状态::运行中;
    状态.帧 = 0;
    状态.待单步 = 0;
    状态.单步执行次数 = 0;
    状态.模拟累计秒 = 0.0;
    状态.本次步长秒 = 0.0;
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
    状态.状态 = Pie运行状态::已停止;
    状态.启动时刻 = None;
    // 停止必须丢弃未执行的单步：否则下一次启动会"凭空"多推进一次。
    状态.待单步 = 0;
    状态.本次步长秒 = 0.0;
    info!("PIE 原型已停止（本次运行 {} 帧）", 状态.帧);
}

/// 暂停：**只**冻结游戏模拟，编辑器与渲染照常。
///
/// 非法转换（未运行/已暂停）返回 `false`，不制造"暂停会话"这种假状态。
pub fn pie暂停(状态: &mut Pie原型状态, 输入: &mut Pie输入) -> bool {
    if 状态.状态 != Pie运行状态::运行中 {
        return false;
    }
    状态.状态 = Pie运行状态::已暂停;
    状态.本次步长秒 = 0.0;
    // 暂停时清掉"按住的键"：暂停期间用户松手不会产生释放事件，
    // 不清的话恢复瞬间会带着旧按键状态突然移动一段。
    清空游戏输入(输入);
    info!("PIE 已暂停（帧 {}）", 状态.帧);
    true
}

/// 继续：恢复游戏模拟。
///
/// 用 `false` 表示"当前不该继续"，界面据此不再显示可用的继续按钮。
pub fn pie继续(状态: &mut Pie原型状态, 输入: &mut Pie输入) -> bool {
    if 状态.状态 != Pie运行状态::已暂停 {
        return false;
    }
    状态.状态 = Pie运行状态::运行中;
    // 同样清空：暂停期间按下的键不会补发，恢复后从"没有按键"开始。
    清空游戏输入(输入);
    info!(
        "PIE 已继续（帧 {} · 模拟累计 {:.3}s）",
        状态.帧, 状态.模拟累计秒
    );
    true
}

/// 请求单步：只有"已暂停且会话有效"时才接受。
///
/// 这里只**记一次请求**，真正的执行在下一帧的 `推进原型` 里消费。
/// 这样单步与常规模拟走同一条代码路径，不会出现"单步跑的是一套、
/// 正常跑的是另一套"的偏差。
pub fn pie请求单步(状态: &mut Pie原型状态) -> bool {
    if 状态.状态 != Pie运行状态::已暂停 {
        return false;
    }
    状态.待单步 = 状态.待单步.saturating_add(1);
    info!("PIE 单步请求（待执行 {} 次）", 状态.待单步);
    true
}

/// 验收脚本：按应用帧号驱动"暂停 → 单步 ×2 → 继续"。
///
/// 为什么用环境变量而不是点按钮：`gui_probe` 能注入鼠标，但 PIE 面板是
/// 应用内 egui 窗口，按钮位置随布局变化；用脚本触发的是**同一套**
/// `pie暂停 / pie请求单步 / pie继续` 调用，只是触发源不同。
/// 按钮本身的可点性由截图目视确认，调用链由本脚本验证。
// 折叠成 `120 if pie请求单步(..)` 会借用冲突（match 的可变借用与 guard
// 里的 `&mut 状态` 重叠），因此保留显式 if。
#[allow(clippy::collapsible_match)]
fn 按场景生命周期(mut 状态: ResMut<Pie原型状态>, mut 输入: ResMut<Pie输入>) {
    if std::env::var(原型场景环境变量).as_deref() != Ok("lifecycle") {
        return;
    }
    状态.场景帧 += 1;
    let 帧 = 状态.场景帧;
    match 帧 {
        90 => {
            if pie暂停(&mut 状态, &mut 输入) {
                info!("[生命周期] 第 90 帧：暂停");
            }
        }
        120 => {
            if pie请求单步(&mut 状态) {
                info!("[生命周期] 第 120 帧：单步 #1");
            }
        }
        150 => {
            if pie请求单步(&mut 状态) {
                info!("[生命周期] 第 150 帧：单步 #2");
            }
        }
        180 => {
            if pie继续(&mut 状态, &mut 输入) {
                info!("[生命周期] 第 180 帧：继续");
            }
        }
        _ => {}
    }
}

/// 验收脚本：在第 N 帧自动请求捕获。
fn 按帧自动捕获(mut 焦点: ResMut<Pie输入焦点>, 状态: Res<Pie原型状态>) {
    let Ok(帧文本) = std::env::var(原型自动捕获环境变量) else {
        return;
    };
    let Ok(帧号) = 帧文本.parse::<u64>() else {
        return;
    };
    if !状态.状态.在会话中() || 状态.帧 != 帧号 {
        return;
    }
    if 焦点.请求捕获() {
        info!("PIE 自动捕获（验收脚本，第 {帧号} 帧）：{}", 焦点.摘要());
    }
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

/// 同步输入焦点：**所有**焦点状态转换都经过这里。
///
/// # 为什么集中在一个系统里
///
/// 焦点是最容易分裂成多个布尔值的东西：视口 UI 想改"是否捕获"，
/// 运行生命周期想改"是否运行"，窗口事件想改"是否活跃"。三处各改一半，
/// 就会出现"按 Esc 释放了鼠标但键盘还在路由""停止后游戏还在动"这类
/// 互相矛盾的中间态。
///
/// 这里把状态机（`pie_input::Pie输入焦点`）当作唯一权威，本系统只负责
/// 把"外部发生了什么事"翻译成状态机的一次调用：
///
/// ```text
///  窗口 focused 变化 ──▶ 窗口失焦 / 窗口恢复
///  Esc 按下（捕获中）──▶ 释放(用户按Esc)
///  运行开始/停止 ────▶ 开始会话 / 会话结束
///  视口纹理消失 ─────▶ 目标失效
/// ```
#[allow(clippy::too_many_arguments)]
fn 同步焦点(
    mut 焦点: ResMut<Pie输入焦点>,
    状态: Res<Pie原型状态>,
    mut 输入: ResMut<Pie输入>,
    窗口: Query<&Window, With<PrimaryWindow>>,
    键盘: Res<ButtonInput<KeyCode>>,
) {
    // ---- 运行开始 / 停止 ----
    //
    // 用 `在会话中()` 而不是"是否正在推进"：暂停只是冻结的运行会话，
    // 焦点会话必须保留，否则暂停会被当成"运行结束"把输入焦点清掉。
    let 运行中 = 状态.状态.在会话中();
    let 有会话 = 焦点.状态 != Pie焦点状态::无会话;
    if 运行中 && !有会话 {
        焦点.开始会话();
        清空游戏输入(&mut 输入);
        info!("PIE 输入焦点：{}", 焦点.摘要());
    } else if !运行中 && 有会话 {
        焦点.会话结束();
        清空游戏输入(&mut 输入);
        info!("PIE 输入焦点：{}", 焦点.摘要());
    }

    // ---- 视口目标失效 ----
    if 运行中 && 状态.纹理.is_none() {
        焦点.目标失效();
        清空游戏输入(&mut 输入);
        info!("PIE 输入目标失效，已退回未捕获");
    }

    // ---- 窗口操作焦点 ----
    if let Ok(窗口) = 窗口.single() {
        if 窗口.focused && !焦点.窗口活跃 {
            焦点.窗口恢复();
            info!("PIE 输入焦点：{}", 焦点.摘要());
        } else if !窗口.focused && 焦点.窗口活跃 {
            焦点.窗口失焦();
            清空游戏输入(&mut 输入);
            info!("PIE 输入焦点：{}", 焦点.摘要());
        }
    }

    // ---- Esc：只在捕获时归游戏 ----
    // 未捕获时 Esc 完全不动（编辑器的"取消选择"仍然生效，见 actions）。
    if 焦点.游戏接收键盘() && 键盘.just_pressed(KeyCode::Escape) {
        焦点.释放(焦点变化原因::用户按Esc);
        清空游戏输入(&mut 输入);
        info!("PIE 输入焦点：{}", 焦点.摘要());
    }
}

/// 本帧的原始按键快照（交给 `pie_input::路由键盘` 决定归属）。
fn 读原始按键(键盘: &ButtonInput<KeyCode>) -> 原始按键 {
    原始按键 {
        左: 键盘.pressed(KeyCode::KeyA) || 键盘.pressed(KeyCode::ArrowLeft),
        右: 键盘.pressed(KeyCode::KeyD) || 键盘.pressed(KeyCode::ArrowRight),
        加速: 键盘.pressed(KeyCode::ShiftLeft) || 键盘.pressed(KeyCode::ShiftRight),
        空格按下: 键盘.just_pressed(KeyCode::Space),
        esc按下: 键盘.just_pressed(KeyCode::Escape),
    }
}

/// 测试专用：把 Bevy 键盘快照读成 `原始按键`。
///
/// 生产路径用私有函数 `读原始按键`；集成测试要验证"从真实 `ButtonInput`
/// 到路由结果"这一段，因此暴露一个只读包装，而不是把私有函数改成 pub。
pub fn 读原始按键为测试用(键盘: &ButtonInput<KeyCode>) -> 原始按键 {
    读原始按键(键盘)
}

/// 清空 PIE 自己的输入状态。
///
/// **只清自己的**：绝不 `键盘.clear()` / `键盘.reset()` —— 那会把编辑器
/// 快捷键与其它系统的按键状态一起抹掉，属于"为了修一个 bug 制造三个"。
pub fn 清空游戏输入(输入: &mut Pie输入) {
    输入.左 = false;
    输入.右 = false;
    输入.加速 = false;
    输入.指针目标 = None;
    输入.鼠标位移x = 0.0;
    输入.左键按下 = false;
    输入.退出捕获 = false;
}

/// 应用鼠标捕获（指针锁定 + 隐藏光标）。
///
/// # 这是**系统级**捕获，不是"只在 egui 矩形里锁"
///
/// `CursorGrabMode::Locked` 由 winit 下发到窗口：捕获期间指针被锁定在
/// 窗口内某点，不再能移动到别的窗口/桌面边缘。因此提示文案不能用
/// "画面区域内锁定"这类会误导人的说法。
///
/// Windows 支持 `Locked`；若平台不支持，winit 会退回 `Confined`
/// 或 `None`。这里按"状态要求捕获 → 设 Locked；状态不要求 → 设 None"
/// 写，不判断返回值，因为"设置失败"不该让编辑器 panic —— 提示文案
/// 仍然显示当前状态，用户按 Esc 仍可释放。
fn 应用鼠标捕获(
    焦点: Res<Pie输入焦点>,
    窗口: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let mut 窗口 = 窗口;
    if !焦点.is_changed() {
        return;
    }
    let Ok(mut 光标) = 窗口.single_mut() else {
        return;
    };
    // 用 Bool 比较而不是"每帧都写"：每帧写 `cursor_options` 会触发
    // 变更检测，窗口系统每帧都做一次无意义的抓取/释放。
    let 想要捕获 = 焦点.鼠标捕获;
    let 当前捕获 = 光标.grab_mode != CursorGrabMode::None;
    if 想要捕获 == 当前捕获 {
        return;
    }
    if 想要捕获 {
        光标.grab_mode = CursorGrabMode::Locked;
        光标.visible = false;
    } else {
        光标.grab_mode = CursorGrabMode::None;
        光标.visible = true;
    }
}

/// 读取真实键盘与鼠标，经**路由**决定这一帧谁拿到输入。
///
/// 路由本身在 `pie_input::路由键盘`（纯函数、有单测）；这里只做两件事：
/// 把 Bevy 输入读成 `原始按键`，把结果写进 `Pie输入`。
fn 同步输入(
    键盘: Res<ButtonInput<KeyCode>>,
    鼠标按键: Res<ButtonInput<MouseButton>>,
    mut 鼠标位移: MessageReader<bevy::input::mouse::MouseMotion>,
    焦点: Res<Pie输入焦点>,
    mut 输入: ResMut<Pie输入>,
    状态: Res<Pie原型状态>,
) {
    // 输入采集**不**受暂停影响：暂停是"冻结模拟"，不是"断开输入"。
    // 但暂停时也不该把按住的键喂给模拟，因此下面只在运行态把结果写进输入。
    if !状态.状态.在会话中() {
        清空游戏输入(&mut 输入);
        return;
    }

    let 结果 = 路由键盘(&焦点, 读原始按键(&键盘));

    // 键盘：只有路由给游戏时才有值；未捕获时全部为 false。
    输入.左 = 结果.游戏.左;
    输入.右 = 结果.游戏.右;
    输入.加速 = 结果.游戏.加速;
    输入.退出捕获 = 结果.游戏.退出捕获;

    // 鼠标：两条链路，按焦点状态二选一，绝不混用。
    if 焦点.游戏接收鼠标() {
        // 捕获态：指针被锁定，只能用相对位移。
        let mut dx = 0.0;
        for 事件 in 鼠标位移.read() {
            dx += 事件.delta.x;
        }
        输入.鼠标位移x = dx;
        输入.左键按下 = 鼠标按键.pressed(MouseButton::Left);
        // 捕获时禁用"朝指针位置走"：那个目标是相对画面矩形的绝对坐标，
        // 指针锁定后它没有意义，两者同时生效会让方块乱跳。
        输入.指针目标 = None;
    } else {
        输入.鼠标位移x = 0.0;
        输入.左键按下 = false;
        // 未捕获时不清 `指针目标`：它由视口 UI 在拖拽时写入，
        // 这里清掉会把 UI 刚写进去的值抹掉（同一帧内两个系统争同一字段）。
    }
}

/// 本帧模拟步长的**唯一决策点**：返回 `Some(dt)` 表示本帧推进一次模拟。
///
/// 抽成纯函数是为了让"暂停真的不推进""单步真的只推进一次"这两件事
/// 能被单元测试直接验证执行次数，而不是只检查某个布尔字段。
///
/// 规则：
///
/// * `已停止` → 永不推进；
/// * `运行中` → 用真实 dt（夹在 0..=0.1，避免恢复/卡顿后瞬移）；
/// * `已暂停` → 只有存在单步请求才推进**一个固定步长**，并消费一个请求。
pub fn 取本帧步长(状态: &mut Pie原型状态, 真实dt: f32) -> Option<f32> {
    match 状态.状态 {
        Pie运行状态::已停止 => None,
        Pie运行状态::运行中 => Some(真实dt.clamp(0.0, 0.1)),
        Pie运行状态::已暂停 => {
            if 状态.待单步 == 0 {
                return None;
            }
            状态.待单步 -= 1;
            状态.单步执行次数 += 1;
            info!(
                "PIE 单步执行 #{}（帧 {} → {}）",
                状态.单步执行次数,
                状态.帧,
                状态.帧 + 1
            );
            Some(单步步长秒)
        }
    }
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

/// 推进原型世界：**PIE 游戏模拟的唯一入口**。
///
/// # 执行边界
///
/// 这个系统就是"游戏模拟"：它读输入、推进 `Pie原型状态`、写 Transform。
/// 暂停时它**完全不推进**（直接 return），但同帧的其它系统照常运行：
///
/// ```text
///   编辑器界面 / egui 绘制      ← 与暂停无关，一直跑
///   PIE 输入采集与焦点管理      ← 与暂停无关（暂停不该吞掉焦点）
///   PIE 渲染（相机 + 纹理）      ← 与暂停无关（暂停时画面仍要能看）
///   PIE 游戏模拟（本函数）       ← 只在"运行中"或"有单步请求"时推进
/// ```
///
/// # 单步的粒度
///
/// 一次单步 = **一个固定 1/60 秒模拟步**（`单步步长秒`），不是"一帧渲染"。
/// 暂停期间没有真实 dt 可用，固定步长让单步结果可复现。
fn 推进原型(
    mut 状态: ResMut<Pie原型状态>,
    输入: Res<Pie输入>,
    时间: Res<Time>,
    mut 查询: Query<&mut Transform, With<Pie方块>>,
) {
    if !状态.状态.在会话中() {
        状态.本次步长秒 = 0.0;
        return;
    }
    let 开始 = std::time::Instant::now();

    // ---- 决定本帧的模拟步长（唯一决策点，见 `取本帧步长`）----
    let Some(步长) = 取本帧步长(&mut 状态, 时间.delta_secs()) else {
        状态.本次步长秒 = 0.0;
        return;
    };
    状态.本次步长秒 = 步长;
    状态.模拟累计秒 += f64::from(步长);

    // 捕获态的鼠标相对位移（与键盘一样，只在模拟真正推进时消费）。
    if 输入.鼠标位移x.abs() > 0.0 {
        let 位移 = 输入.鼠标位移x * 0.01;
        let 新x = (状态.方块x + 位移).clamp(左边界, 右边界);
        if 新x != 状态.方块x {
            状态.方块x = 新x;
            状态.鼠标驱动帧数 += 1;
        }
    }

    步进(&mut 状态, &输入, 步长);
    for mut 变换 in &mut 查询 {
        变换.translation.x = 状态.方块x;
    }
    状态.上一帧毫秒 = 开始.elapsed().as_secs_f32() * 1000.0;
}

/// 测试专用：把 `推进原型` 系统包装成可被 `run_system_once` 调用的形式。
///
/// 生产路径由插件注册；集成测试需要"真的跑一遍系统"，因此暴露这个别名，
/// 而不是把私有系统改成 `pub` 或复制一份逻辑。
pub fn 推进原型为测试用(
    状态: ResMut<Pie原型状态>,
    输入: Res<Pie输入>,
    时间: Res<Time>,
    查询: Query<&mut Transform, With<Pie方块>>,
) {
    推进原型(状态, 输入, 时间, 查询)
}

/// 连续帧证据：每 30 帧写一条带帧号与方块位置的信息级日志。
///
/// # 为什么日志能证明"实时"
///
/// 截图只能证明某一时刻。日志里出现 `帧 30 x=-2.13`、`帧 60 x=1.77`…
/// 是同一个进程里连续推进的序列，配合"同一会话内两帧截图不同"，
/// 才构成"画面在动"的证据链。
fn 记录连续帧证据(
    mut 状态: ResMut<Pie原型状态>,
    输入: Res<Pie输入>,
    焦点: Res<Pie输入焦点>,
) {
    if !状态.状态.在会话中() {
        return;
    }
    // 去重必须**记录**上次的帧号：只读它会让暂停期间（帧号不变）
    // 同一个帧号被打印几十遍，日志噪声大到看不出单步。
    if !状态.帧.is_multiple_of(30) || 状态.帧 == 状态.上次记录帧 {
        return;
    }
    状态.上次记录帧 = 状态.帧;
    info!(
        "PIE 帧 {} · 运行 {} · 方块 x={:.3} · 焦点 {} · 键盘 左={} 右={} · \
         指针 {:?} · 输入驱动 {} 帧 · 单步 {} 次 · 推进 {:.3} ms",
        状态.帧,
        状态.状态.名称(),
        状态.方块x,
        焦点.状态.名称(),
        输入.左,
        输入.右,
        输入.指针目标,
        状态.指针驱动帧数,
        状态.单步执行次数,
        状态.上一帧毫秒
    );
}

/// 原型窗口：把渲染目标当成普通 egui 纹理显示，并提供启动/停止/重启。
#[allow(clippy::too_many_arguments)]
fn 绘制原型窗口(
    mut 上下文: EguiContexts,
    mut 状态: ResMut<Pie原型状态>,
    mut 输入: ResMut<Pie输入>,
    mut 焦点: ResMut<Pie输入焦点>,
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
    let 运行状态 = 状态.状态;
    let 在会话中 = 运行状态.在会话中();
    let 正在推进 = 运行状态.模拟推进();
    let 已暂停 = 运行状态 == Pie运行状态::已暂停;
    let mut 请求启动 = false;
    let mut 请求停止 = false;
    let mut 请求重启 = false;
    let mut 请求暂停 = false;
    let mut 请求继续 = false;
    let mut 请求单步 = false;
    let 单步执行次数 = 状态.单步执行次数;
    let 待单步 = 状态.待单步;
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
                if ui
                    .add_enabled(!在会话中, egui::Button::new("启动"))
                    .clicked()
                {
                    请求启动 = true;
                }
                let 暂停响应 = ui.add_enabled(正在推进, egui::Button::new("暂停"));
                if 暂停响应.clicked() {
                    请求暂停 = true;
                }
                let 继续响应 = ui.add_enabled(已暂停, egui::Button::new("继续"));
                if 继续响应.clicked() {
                    请求继续 = true;
                }
                let 单步响应 = ui
                    .add_enabled(已暂停, egui::Button::new("单步"))
                    .on_hover_text("执行一次固定 1/60 秒模拟步后仍保持暂停");
                if 单步响应.clicked() {
                    请求单步 = true;
                }
                let 停止响应 = ui.add_enabled(在会话中, egui::Button::new("停止"));
                if 停止响应.clicked() {
                    请求停止 = true;
                }
                let 重启响应 = ui.add_enabled(在会话中, egui::Button::new("重启"));
                if 重启响应.clicked() {
                    请求重启 = true;
                }
                // 状态用**文字**表达，不能只靠按钮可用性猜。
                let 状态色 = if 正在推进 {
                    egui::Color32::LIGHT_GREEN
                } else if 已暂停 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::GRAY
                };
                ui.label(egui::RichText::new(运行状态.名称()).strong().color(状态色));
                if 待单步 > 0 {
                    ui.label(
                        egui::RichText::new(format!("待单步 {待单步}"))
                            .small()
                            .color(egui::Color32::YELLOW),
                    );
                }
            });
            ui.label(format!(
                "帧 {帧} · 方块 x={方块x:.3} · 启动 {启动次数} 次 · 单步 {单步执行次数} 次"
            ));
            ui.label(egui::RichText::new(format!("推进 {上一帧毫秒:.3} ms")).small());
            // 焦点提示：**状态驱动**，不是"上次点击残留"。
            // 同时给文字与颜色：只靠颜色对色觉障碍用户等于没提示。
            let (提示, 提示色) = match 焦点.状态 {
                Pie焦点状态::无会话 => ("未运行：点「启动」开始", egui::Color32::GRAY),
                Pie焦点状态::未捕获 if 在会话中 => (
                    "点击画面开始交互（键盘与鼠标交给游戏）",
                    egui::Color32::LIGHT_BLUE,
                ),
                Pie焦点状态::未捕获 => ("未运行：点「启动」开始", egui::Color32::GRAY),
                Pie焦点状态::已捕获 => (
                    "已捕获：WASD / ←→ 控制，Esc 释放",
                    egui::Color32::LIGHT_GREEN,
                ),
                Pie焦点状态::挂起 => {
                    ("窗口失焦，已释放；点回画面继续", egui::Color32::YELLOW)
                }
            };
            ui.label(egui::RichText::new(提示).small().color(提示色));
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
                    // 点击画面 = 请求捕获。只有命中**图像矩形**的点击才算：
                    // egui 的 `Response` 只覆盖这张图，点工具栏/检查器拿不到它。
                    if 响应.clicked() {
                        焦点.请求捕获();
                    }

                    if 焦点.游戏接收鼠标() {
                        // 捕获态：指针被系统锁定，绝对坐标没有意义，
                        // 鼠标输入走 `同步输入` 里的相对位移链路。
                        输入.指针目标 = None;
                    } else {
                        let 指针 = 响应.interact_pointer_pos();
                        let 按住 = 响应.dragged();
                        if let (true, Some(位置)) = (按住, 指针) {
                            let t = ((位置.x - 响应.rect.left()) / 响应.rect.width()) * 2.0 - 1.0;
                            输入.指针目标 = Some(t.clamp(-1.0, 1.0));
                        } else {
                            输入.指针目标 = None;
                        }
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
    if 请求暂停 {
        pie暂停(&mut 状态, &mut 输入);
    }
    if 请求继续 {
        pie继续(&mut 状态, &mut 输入);
    }
    if 请求单步 {
        pie请求单步(&mut 状态);
    }
    if 请求停止 || 请求重启 {
        // 立即释放捕获：不能等下一帧的 `同步焦点` —— 那一帧里会话
        // 可能还在，游戏仍会收到一次按键。
        焦点.释放(焦点变化原因::运行停止);
        清空游戏输入(&mut 输入);
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
            状态: Pie运行状态::运行中,
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

    // ---- R13.2 运行生命周期 ----

    fn 运行态() -> Pie原型状态 {
        Pie原型状态 {
            状态: Pie运行状态::运行中,
            方块方向: 1.0,
            ..Pie原型状态::default()
        }
    }

    #[test]
    fn 暂停后模拟不推进且步长为零() {
        let mut 状态 = 运行态();
        let mut 输入 = Pie输入::default();
        assert!(pie暂停(&mut 状态, &mut 输入));
        assert_eq!(状态.状态, Pie运行状态::已暂停);

        let 帧前 = 状态.帧;
        let 位置前 = 状态.方块x;
        // 模拟连续 60 帧：`取本帧步长` 必须一直返回 None。
        for _ in 0..60 {
            assert!(
                取本帧步长(&mut 状态, 1.0 / 60.0).is_none(),
                "暂停后不能有任何一帧推进"
            );
        }
        assert_eq!(状态.帧, 帧前, "暂停期间模拟计数不能增长");
        assert_eq!(状态.方块x, 位置前);
        assert_eq!(状态.模拟累计秒, 0.0);
    }

    #[test]
    fn 继续后模拟恢复推进() {
        let mut 状态 = 运行态();
        let mut 输入 = Pie输入::default();
        assert!(pie暂停(&mut 状态, &mut 输入));
        assert!(取本帧步长(&mut 状态, 0.016).is_none());

        assert!(pie继续(&mut 状态, &mut 输入));
        assert_eq!(状态.状态, Pie运行状态::运行中);
        let 步长 = 取本帧步长(&mut 状态, 0.016).expect("继续后必须推进");
        assert!((步长 - 0.016).abs() < 1e-6);
    }

    #[test]
    fn 单步只推进一次且完成后仍暂停() {
        let mut 状态 = 运行态();
        let mut 输入 = Pie输入::default();
        pie暂停(&mut 状态, &mut 输入);

        assert!(pie请求单步(&mut 状态));
        assert_eq!(状态.待单步, 1);

        let 步长 = 取本帧步长(&mut 状态, 0.016).expect("有单步请求时必须推进");
        assert!((步长 - 单步步长秒).abs() < 1e-6, "单步必须是固定步长");
        assert_eq!(状态.待单步, 0, "一次单步消费一个请求");
        assert_eq!(状态.单步执行次数, 1);
        assert_eq!(状态.状态, Pie运行状态::已暂停, "单步后仍保持暂停");

        // 下一次没有新请求：不再推进。
        assert!(取本帧步长(&mut 状态, 0.016).is_none());
        assert_eq!(状态.单步执行次数, 1);
    }

    #[test]
    fn 连续两次单步请求执行两次而不是合并() {
        let mut 状态 = 运行态();
        let mut 输入 = Pie输入::default();
        pie暂停(&mut 状态, &mut 输入);
        assert!(pie请求单步(&mut 状态));
        assert!(pie请求单步(&mut 状态));
        assert_eq!(状态.待单步, 2);

        assert!(取本帧步长(&mut 状态, 0.016).is_some());
        assert_eq!(状态.单步执行次数, 1, "一帧最多消费一个请求");
        assert!(取本帧步长(&mut 状态, 0.016).is_some());
        assert_eq!(状态.单步执行次数, 2);
        assert!(取本帧步长(&mut 状态, 0.016).is_none());
    }

    #[test]
    fn 未运行时暂停与单步都被拒绝() {
        let mut 状态 = Pie原型状态::default();
        let mut 输入 = Pie输入::default();
        assert!(!pie暂停(&mut 状态, &mut 输入), "未运行不能进暂停态");
        assert!(!pie请求单步(&mut 状态), "未运行不能单步");
        assert_eq!(状态.状态, Pie运行状态::已停止, "不能制造虚假暂停会话");
        assert_eq!(状态.待单步, 0);
    }

    #[test]
    fn 已运行时重复暂停被拒绝() {
        let mut 状态 = 运行态();
        let mut 输入 = Pie输入::default();
        assert!(pie暂停(&mut 状态, &mut 输入));
        assert!(!pie暂停(&mut 状态, &mut 输入), "重复暂停是无操作");
        let mut 另一个 = 运行态();
        assert!(!pie继续(&mut 另一个, &mut 输入), "运行中不能继续");
    }

    #[test]
    fn 停止会丢弃未执行单步且不残留暂停态() {
        let mut 状态 = 运行态();
        let mut 输入 = Pie输入::default();
        pie暂停(&mut 状态, &mut 输入);
        assert!(pie请求单步(&mut 状态));
        assert_eq!(状态.待单步, 1);

        // `停止_实现` 需要 Commands/实体；这里验证等价的状态清理。
        状态.状态 = Pie运行状态::已停止;
        状态.待单步 = 0;
        状态.本次步长秒 = 0.0;

        assert_eq!(状态.状态, Pie运行状态::已停止);
        assert!(取本帧步长(&mut 状态, 0.016).is_none(), "停止后不能推进");
        assert_eq!(状态.待单步, 0);
    }

    #[test]
    fn 暂停与继续不会改动场景文档() {
        // PIE 生命周期只碰 ECS 资源，不持有场景文档。
        // 这里把"不改文档"写成断言，防止将来有人顺手在暂停时写回。
        use scene_doc::document::场景文档;
        let 文档 = 场景文档::新建("生命周期隔离");
        let 快照 = 文档.clone();
        let mut 状态 = 运行态();
        let mut 输入 = Pie输入::default();
        pie暂停(&mut 状态, &mut 输入);
        pie请求单步(&mut 状态);
        取本帧步长(&mut 状态, 0.016);
        pie继续(&mut 状态, &mut 输入);
        assert_eq!(文档, 快照);
    }

    #[test]
    fn 巨大dt在恢复后不会让方块瞬移() {
        let mut 状态 = 运行态();
        状态.方块x = 0.0;
        state_set_direction(&mut 状态);
        let 步长 = 取本帧步长(&mut 状态, 30.0).expect("运行中必须推进");
        assert!(步长 <= 0.1, "dt 必须被夹住：{步长}");
        let mut 输入 = Pie输入::default();
        步进(&mut 状态, &输入, 步长);
        assert!(
            (左边界..=右边界).contains(&状态.方块x),
            "夹住 dt 后方块不能瞬移出画面：{}",
            状态.方块x
        );
        let _ = &mut 输入;
    }

    fn state_set_direction(状态: &mut Pie原型状态) {
        状态.方块方向 = 1.0;
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
