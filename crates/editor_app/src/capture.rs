//! GUI 采集：把**真实窗口画面**与**真实布局几何**落盘，供自动验收工具核对。
//!
//! # 为什么必须换掉旧的外部抓图
//!
//! 旧流程用 `PrintWindow(PW_RENDERFULLCONTENT)` 从进程外抓窗口。它对本编辑器
//! 这类 wgpu 应用**不可靠**：wgpu 把画面提交给 DWM 合成，窗口的 GDI 表面
//! 通常不含有效内容，`PrintWindow` 于是返回过陈旧帧、纯黑、或部分区域丢失的位图。
//! 之前几轮"截图看起来是坏的，但代码其实没问题"的误判就是这么来的 ——
//! **用不可靠的抓图去改生产代码，比不抓图更糟**。
//!
//! # 采用的方案：进程内渲染目标回读
//!
//! Bevy 的 [`Screenshot`] 直接从**渲染目标**（也就是窗口的交换链内容）
//! 回读像素，走 `wgpu` 的 buffer readback，不经过 GDI，也不依赖窗口是否被遮挡。
//! 它拿到的正是这一帧被合成到窗口上的画面 —— 与 Windows Graphics Capture
//! 在合成器侧抓到的内容同源，但不需要 D3D11 互操作、WinRT 异步帧池，
//! 也不需要额外的系统权限。
//!
//! 这条路径还有一个关键优势：**同帧一致性**。截图请求与布局几何在同一帧产生，
//! 因此"截图里某面板在左边"与"JSON 里某面板的矩形在左边"必然是同一帧的事实，
//! 不会出现外部抓图常见的"截的是上一帧布局"的错位。
//!
//! # 与 `gui_probe` 的分工
//!
//! 本模块只负责**如实地把事实写成文件**，不做任何判定：
//!
//! * PNG：这一帧的窗口画面；
//! * JSON：语言、主题、UI 缩放、窗口尺寸、每块区域的实际矩形、状态栏文案。
//!
//! 判定（尺寸一致性、区域是否有内容、三栏关系、文字是否真的画出来了、
//! 亮色/深色主题是否符合预期）全部由 `crates/gui_probe` 离线完成。
//! 这样"采集失败"与"界面缺陷"能被明确区分：前者是文件缺失或尺寸不符，
//! 后者是文件齐全但像素/几何断言不成立。
//!
//! # 启用方式
//!
//! 只有设置了 [`采集图环境变量`] 才会启用，正常启动完全不受影响
//! （每帧只多一次 `Option` 判断）。

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_egui::EguiContexts;
use serde::Serialize;

use crate::plugin::{EditorStateHolder, UiStateHolder, 诊断资源};
use crate::scene_bridge::运行时宿主;

/// 采集的 PNG 输出路径。设置它才会启用采集。
pub const 采集图环境变量: &str = "BEVY_STUDIO_CAPTURE_PNG";
/// 证据 JSON 的输出路径。留空时与 PNG 同名、扩展名换成 `.json`。
pub const 采集报告环境变量: &str = "BEVY_STUDIO_CAPTURE_JSON";
/// 在哪一帧采集（从 1 开始数，默认 90）。
///
/// 90 帧的依据：启动后需要先载入项目/场景、替换中文字体（会重建字体图集并
/// 重新排版一次）、建立预览渲染目标。实测 90 帧时界面已完全稳定。
pub const 采集帧环境变量: &str = "BEVY_STUDIO_CAPTURE_FRAME";
/// 第二张截图的输出路径（可选）。与 `CAPTURE_PNG` 一起构成"操作前/操作后"双帧证据。
pub const 采集图2环境变量: &str = "BEVY_STUDIO_CAPTURE_PNG2";
/// 第二份证据 JSON 的路径。留空时与第二张截图同名。
pub const 采集报告2环境变量: &str = "BEVY_STUDIO_CAPTURE_JSON2";
/// 第二张截图在哪一帧（默认第一帧 + 120）。
pub const 采集帧2环境变量: &str = "BEVY_STUDIO_CAPTURE_FRAME2";
/// 采集完成后是否自动退出（默认 1）。
pub const 采集中退出环境变量: &str = "BEVY_STUDIO_CAPTURE_EXIT";
/// 启动窗口尺寸 `宽x高`（逻辑像素）。用于验证不同窗口尺寸下的布局。
pub const 窗口尺寸环境变量: &str = "BEVY_STUDIO_WINDOW_SIZE";

/// 等待截图写盘的最大帧数。
///
/// 回读是异步的（渲染线程 → 通道 → 观察者），通常 1~3 帧内完成。
/// 给 600 帧的余量，超时说明截图链路本身坏了，此时必须让验收**失败**，
/// 而不是假装采集成功 —— 这正是旧流程最危险的地方。
const 写盘超时帧数: u32 = 600;

/// 采集配置与进度。
#[derive(Resource, Debug, Clone, Default)]
pub struct 采集配置 {
    /// PNG 输出路径。`None` 表示未启用采集。
    pub 图片: Option<PathBuf>,
    /// JSON 输出路径。
    pub 报告: Option<PathBuf>,
    /// 目标帧。
    pub 目标帧: u32,
    /// 采集后是否退出。
    pub 完成后退出: bool,
    /// 已经过的界面帧数。
    pub 已过帧: u32,
    /// 是否已发起截图。
    pub 已发起: bool,
    /// 发起后等待写盘的帧数。
    pub 等待帧: u32,
    /// 报告是否已写入。
    pub 报告已写: bool,
    /// 采集是否彻底完成（PNG 已落盘）。
    pub 已完成: bool,
    /// 第二张截图路径（`None` 表示只采单帧）。
    pub 图片2: Option<PathBuf>,
    /// 第二份证据路径。
    pub 报告2: Option<PathBuf>,
    /// 第二张截图的帧号（0 表示不采第二帧）。
    pub 目标帧2: u32,
    /// 是否已发起第二帧截图。
    pub 已发起2: bool,
    /// 第二份证据是否已写。
    pub 报告2已写: bool,
}

impl 采集配置 {
    /// 从环境变量读取配置。未设置 PNG 路径时返回未启用状态。
    pub fn 从环境变量() -> Self {
        let Some(图片) = 取环境变量路径(采集图环境变量) else {
            return Self::default();
        };
        let 报告 = 取环境变量路径(采集报告环境变量).unwrap_or_else(|| 图片.with_extension("json"));
        let 目标帧 = std::env::var(采集帧环境变量)
            .ok()
            .and_then(|值| 值.trim().parse::<u32>().ok())
            .filter(|值| *值 > 0)
            .unwrap_or(90);
        let 完成后退出 = std::env::var(采集中退出环境变量)
            .map(|值| !matches!(值.trim(), "0" | "false" | "no"))
            .unwrap_or(true);

        // 双帧协议：调用方设置 PNG2 才会启用第二帧。
        let 图片2 = 取环境变量路径(采集图2环境变量);
        let 报告2 = 图片2.as_ref().map(|路径| {
            取环境变量路径(采集报告2环境变量).unwrap_or_else(|| 路径.with_extension("json"))
        });
        let 目标帧2 = if 图片2.is_some() {
            std::env::var(采集帧2环境变量)
                .ok()
                .and_then(|值| 值.trim().parse::<u32>().ok())
                .filter(|值| *值 > 目标帧)
                .unwrap_or(目标帧 + 120)
        } else {
            0
        };

        Self {
            图片: Some(图片),
            报告: Some(报告),
            目标帧,
            完成后退出,
            图片2,
            报告2,
            目标帧2,
            ..Default::default()
        }
    }

    /// 是否启用了采集。
    pub fn 已启用(&self) -> bool {
        self.图片.is_some()
    }
}

fn 取环境变量路径(名: &str) -> Option<PathBuf> {
    let 值 = std::env::var(名).ok()?;
    let 值 = 值.trim();
    if 值.is_empty() {
        return None;
    }
    Some(PathBuf::from(值))
}

/// 解析 [`窗口尺寸环境变量`]（形如 `1280x800`）。
///
/// 解析失败返回 `None` 并由调用方保留默认尺寸：启动参数写错不应该让
/// 编辑器起不来，只应该在日志里说一声。
pub fn 解析窗口尺寸(值: &str) -> Option<(u32, u32)> {
    let (宽, 高) = 值.trim().split_once(['x', 'X', '×'])?;
    let 宽 = 宽.trim().parse::<u32>().ok()?;
    let 高 = 高.trim().parse::<u32>().ok()?;
    // 下限保护：过窄的窗口连三栏布局都放不下，不是合法的验收目标。
    if 宽 < 320 || 高 < 240 {
        return None;
    }
    Some((宽, 高))
}

/// 一块区域在截图坐标系里的矩形（物理像素，左上角为原点）。
#[derive(Debug, Clone, Copy, Serialize)]
struct 区域记录 {
    id: &'static str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// 窗口尺寸记录。
#[derive(Debug, Clone, Copy, Serialize)]
struct 尺寸记录 {
    width: u32,
    height: u32,
}

/// 主开发模式摘要。
#[derive(Debug, Clone, Serialize)]
struct 模式记录 {
    id: String,
    name: String,
}

/// 工作区摘要：让验收工具能断言"这次跑的确实是目标工作区"。
#[derive(Debug, Clone, Serialize)]
struct 工作区记录 {
    /// 稳定 id（`general` / `scene` / `assets` / `debug` / `user-*`）。
    id: String,
    /// 当前语言下的显示名。
    name: String,
    /// 是否内置。
    builtin: bool,
    /// 工作区总数。
    count: usize,
}

/// 主题摘要：让验收工具能断言"这次截的确实是深色主题"。
#[derive(Debug, Clone, Serialize)]
struct 主题记录 {
    /// 主题标识（`dark` / `light` / `high_contrast` / 用户自定义 id）。
    id: String,
    /// 当前语言下的主题名。
    name: String,
    /// 明暗属性：`dark` / `light`。
    scheme: String,
    /// 面板背景色，`#RRGGBB`。
    panel_bg: String,
    /// 主文字色，`#RRGGBB`。
    text: String,
    /// 强调色，`#RRGGBB`。
    accent: String,
}

/// 采集证据。
#[derive(Debug, Clone, Serialize)]
struct 采集证据 {
    /// 证据格式版本。字段只增不改，验收工具按需读取。
    schema: u32,
    app_version: &'static str,
    bevy_version: &'static str,
    /// 语言代号（`zh` / `en`）。
    language: &'static str,
    /// 语言自称。
    language_label: String,
    /// UI 缩放倍数。
    ui_scale: f32,
    /// 一个逻辑点等于多少物理像素。
    pixels_per_point: f32,
    window: 尺寸记录,
    mode: 模式记录,
    workspace: 工作区记录,
    theme: 主题记录,
    /// 各区域矩形（物理像素）。
    regions: Vec<区域记录>,
    /// 状态栏左侧文案。
    status_left: Vec<String>,
    /// 状态栏右侧文案。
    status_right: Vec<String>,
    /// 当前布局里存在的面板标题（当前语言）。
    panel_titles: Vec<String>,
    /// 当前布局里存在的面板**稳定键**（与语言无关）。
    panel_keys: Vec<String>,
    /// 视口缩放百分比。
    viewport_scale_percent: f32,
    /// 已加载的图标数量（图标系统接入后由 0 变为实际值）。
    icon_count: usize,
}

/// 一次界面绘制结束后写证据并请求截图。
///
/// 必须挂在 `EguiPrimaryContextPass` 且排在界面绘制**之后**：
/// 这样 `布局快照` 里是本帧的真实矩形，而截图请求发生在
/// `ExtractSchedule` 之前，两者对应同一帧。
// Bevy 系统天然需要多个资源参数；这里每一项都是采集证据的必要输入，
// 拆成 SystemParam 只会把同一组依赖藏到另一个类型里，可读性反而更差。
#[allow(clippy::too_many_arguments)]
pub fn 采集一帧(
    mut 采集: ResMut<采集配置>,
    mut 上下文: EguiContexts,
    mut commands: Commands,
    界面: Res<UiStateHolder>,
    编辑器: Res<EditorStateHolder>,
    诊断: Res<诊断资源>,
    运行时: Res<运行时宿主>,
) {
    if !采集.已启用() {
        return;
    }
    let Ok(egui上下文) = 上下文.ctx_mut() else {
        return;
    };
    采集.已过帧 += 1;

    // 第一帧：操作前
    if !采集.已发起 && 采集.已过帧 >= 采集.目标帧 {
        采集.已发起 = true;
        let 图片 = 采集.图片.clone();
        let 报告 = 采集.报告.clone();
        let 帧号 = 采集.已过帧;
        if 发起一次采集(
            &mut commands,
            &egui上下文,
            &界面,
            &编辑器,
            &诊断,
            &运行时,
            图片.as_deref(),
            报告.as_deref(),
            帧号,
        )
        .is_ok()
        {
            采集.报告已写 = 报告.as_deref().map(|路径| 路径.is_file()).unwrap_or(true);
        }
        return;
    }

    // 第二帧：操作后（只有调用方显式设置了 PNG2 才采）
    if 采集.已发起 && !采集.已发起2 && 采集.目标帧2 > 0 && 采集.已过帧 >= 采集.目标帧2
    {
        采集.已发起2 = true;
        let 图片 = 采集.图片2.clone();
        let 报告 = 采集.报告2.clone();
        let 帧号 = 采集.已过帧;
        if 发起一次采集(
            &mut commands,
            &egui上下文,
            &界面,
            &编辑器,
            &诊断,
            &运行时,
            图片.as_deref(),
            报告.as_deref(),
            帧号,
        )
        .is_ok()
        {
            采集.报告2已写 = 报告.as_deref().map(|路径| 路径.is_file()).unwrap_or(true);
        }
    }
}

/// 采集当前帧：写证据 + 请求截图。
///
/// 抽成函数是双帧协议的前提：两帧必须走**完全同一段**证据生成逻辑，
/// 否则"操作前/后"的数据口径会悄悄漂移。
#[allow(clippy::too_many_arguments)]
fn 发起一次采集(
    命令: &mut Commands,
    egui上下文: &egui::Context,
    界面: &UiStateHolder,
    编辑器: &EditorStateHolder,
    诊断: &诊断资源,
    运行时: &运行时宿主,
    图片: Option<&Path>,
    报告: Option<&Path>,
    帧号: u32,
) -> Result<(), String> {
    let 缩放 = egui上下文.zoom_factor();
    let 每点像素 = egui上下文.pixels_per_point();
    let 屏幕矩形 = egui上下文.viewport_rect();
    let 物理 = |值: f32| (值 * 每点像素).round() as i32;
    let 尺寸 = |值: f32| (值 * 每点像素).round().max(1.0) as i32;

    let 区域: Vec<区域记录> = 界面
        .状态
        .布局快照
        .iter()
        .map(|快照| 区域记录 {
            id: 快照.标识,
            x: 物理(快照.矩形.min.x),
            y: 物理(快照.矩形.min.y),
            w: 尺寸(快照.矩形.width()),
            h: 尺寸(快照.矩形.height()),
        })
        .collect();

    let 状态栏 = editor_ui::计算状态栏内容(
        &编辑器.状态,
        界面.状态.语言,
        运行时.状态.在运行(),
        &诊断.0,
        界面.状态.相机.缩放 * 100.0,
    );

    let mut 面板标题 = Vec::new();
    let mut 面板键 = Vec::new();
    for (路径, 面板) in 界面.状态.停靠().iter_all_tabs() {
        // 同一个面板可能只出现一次；这里保留去重后的顺序。
        let _ = 路径;
        if !面板键.contains(&面板.菜单键().to_string()) {
            面板标题.push(面板.标题(界面.状态.语言).to_string());
            面板键.push(面板.菜单键().to_string());
        }
    }

    let 证据 = 采集证据 {
        schema: 1,
        app_version: env!("CARGO_PKG_VERSION"),
        bevy_version: "0.20.0",
        language: 界面.状态.语言.代号(),
        language_label: 界面.状态.语言.自称().to_string(),
        ui_scale: 缩放,
        pixels_per_point: 每点像素,
        window: 尺寸记录 {
            width: 物理(屏幕矩形.width()) as u32,
            height: 物理(屏幕矩形.height()) as u32,
        },
        mode: 模式记录 {
            id: 界面.状态.模式.id().to_string(),
            name: 界面.状态.语言.取(界面.状态.模式.名称键()).to_string(),
        },
        workspace: 工作区记录 {
            id: 界面.状态.工作区.当前id().to_string(),
            name: 界面.状态.工作区.当前().显示名(界面.状态.语言),
            builtin: 界面.状态.工作区.当前().内置,
            count: 界面.状态.工作区.列表().len(),
        },
        theme: 主题记录 {
            id: 界面.状态.主题库.当前id().to_string(),
            name: 界面.状态.主题库.当前().显示名(界面.状态.语言),
            scheme: match 界面.状态.主题库.当前().明暗 {
                editor_ui::明暗::深色 => "dark".to_string(),
                editor_ui::明暗::浅色 => "light".to_string(),
            },
            panel_bg: 十六进制(界面.状态.主题库.当前().配色.面板背景),
            text: 十六进制(界面.状态.主题库.当前().配色.文字),
            accent: 十六进制(界面.状态.主题库.当前().配色.强调),
        },
        regions: 区域,
        status_left: 状态栏.左侧.into_iter().map(|条目| 条目.文本).collect(),
        status_right: 状态栏.右侧.into_iter().map(|条目| 条目.文本).collect(),
        panel_titles: 面板标题,
        panel_keys: 面板键,
        viewport_scale_percent: 界面.状态.相机.缩放 * 100.0,
        icon_count: 0,
    };

    if let Some(报告路径) = 报告.map(Path::to_path_buf) {
        match 写报告(&报告路径, &证据) {
            Ok(()) => {}
            Err(错误) => {
                error!("写入采集证据失败：{错误}");
                // 报告写不出来就没有验收依据，直接失败退出而不是继续跑。
                return Err(format!("写入采集证据失败：{错误}"));
            }
        }
    }

    if let Some(图片路径) = 图片.map(Path::to_path_buf) {
        if let Some(父目录) = 图片路径.parent()
            && let Err(错误) = std::fs::create_dir_all(父目录)
        {
            error!("无法创建截图目录 {}：{错误}", 父目录.display());
        }
        info!(
            "已请求窗口截图：{}（第 {} 帧，{:.0}×{:.0} 逻辑点）",
            图片路径.display(),
            帧号,
            屏幕矩形.width(),
            屏幕矩形.height()
        );
        命令
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(图片路径));
    }
    Ok(())
}

/// 把证据写盘。先写临时文件再改名，避免验收工具读到半截 JSON。
fn 写报告(路径: &Path, 证据: &采集证据) -> Result<(), String> {
    if let Some(父目录) = 路径.parent() {
        std::fs::create_dir_all(父目录)
            .map_err(|错误| format!("无法创建目录 {}：{错误}", 父目录.display()))?;
    }
    let 文本 =
        serde_json::to_string_pretty(证据).map_err(|错误| format!("证据序列化失败：{错误}"))?;
    let 临时 = 路径.with_extension("json.tmp");
    std::fs::write(&临时, 文本).map_err(|错误| format!("写临时证据失败：{错误}"))?;
    std::fs::rename(&临时, 路径).map_err(|错误| {
        let _ = std::fs::remove_file(&临时);
        format!("替换证据文件失败：{错误}")
    })
}

/// `#RRGGBB` 形式的颜色文本。
fn 十六进制(颜色: egui::Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", 颜色.r(), 颜色.g(), 颜色.b())
}

/// 采集完成后退出；超时未拿到 PNG 则**以失败码退出**。
///
/// 退出码约定（与 `gui_probe` 共享）：
///
/// * `0` 成功；
/// * `3` 采集失败（截图链路没有产出文件）。
///
/// 让超时变成非零退出，是为了让"采集失败"在 CI 里不可能被当成成功。
pub fn 采集收尾(mut 采集: ResMut<采集配置>, mut 退出: MessageWriter<AppExit>) {
    if !采集.已启用() || !采集.已发起 || 采集.已完成 {
        return;
    }
    // 双帧协议下必须等第二帧也发起，否则会在两帧之间提前退出。
    let 期望完成 = 采集.目标帧2 == 0 || 采集.已发起2;
    if !期望完成 {
        return;
    }
    采集.等待帧 += 1;

    let 文件就绪 = |路径: &Option<PathBuf>| {
        路径
            .as_ref()
            .map(|路径| path_is_stable_file(路径))
            .unwrap_or(true)
    };
    let 第一帧完成 = 文件就绪(&采集.图片) && 文件就绪(&采集.报告);
    let 第二帧完成 = 采集.目标帧2 == 0 || (文件就绪(&采集.图片2) && 文件就绪(&采集.报告2));

    if 第一帧完成 && 第二帧完成 {
        采集.已完成 = true;
        info!(
            "截图已落盘（等待 {} 帧）：{}",
            采集.等待帧,
            采集
                .图片
                .as_ref()
                .map(|路径| 路径.display().to_string())
                .unwrap_or_default()
        );
        if 采集.完成后退出 {
            退出.write(AppExit::Success);
        }
        return;
    }

    if 采集.等待帧 > 写盘超时帧数 {
        error!("等待截图写盘超时（{} 帧）", 采集.等待帧);
        采集.已完成 = true;
        if 采集.完成后退出 {
            退出.write(AppExit::from_code(3));
        }
    }
}

/// 文件存在且大小稳定（非 0 字节）才算写完。
fn path_is_stable_file(路径: &Path) -> bool {
    路径
        .metadata()
        .map(|元数据| 元数据.is_file() && 元数据.len() > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 窗口尺寸解析接受常见写法() {
        assert_eq!(解析窗口尺寸("1280x800"), Some((1280, 800)));
        assert_eq!(解析窗口尺寸("1600X950"), Some((1600, 950)));
        assert_eq!(解析窗口尺寸(" 1024 × 768 "), Some((1024, 768)));
    }

    #[test]
    fn 窗口尺寸解析拒绝非法输入() {
        // 拒绝而不是 panic：这是来自环境变量的外部输入。
        assert_eq!(解析窗口尺寸(""), None);
        assert_eq!(解析窗口尺寸("abc"), None);
        assert_eq!(解析窗口尺寸("0x0"), None);
        // 过小：三栏布局放不下，不是有效验收目标。
        assert_eq!(解析窗口尺寸("200x200"), None);
        assert_eq!(解析窗口尺寸("1280"), None);
    }

    #[test]
    fn 颜色转十六进制格式正确() {
        assert_eq!(
            十六进制(egui::Color32::from_rgb(30, 31, 34)),
            "#1E1F22".to_string()
        );
        assert_eq!(
            十六进制(egui::Color32::from_rgb(255, 255, 255)),
            "#FFFFFF".to_string()
        );
    }

    #[test]
    fn 未设置环境变量时不启用采集() {
        // 环境变量在测试进程里可能被外部设置，因此这里只断言
        // "没有 PNG 路径就一定不启用"这一条不变量。
        let mut 配置 = 采集配置::default();
        assert!(!配置.已启用());
        配置.图片 = Some(PathBuf::from("x.png"));
        assert!(配置.已启用());
    }
}
