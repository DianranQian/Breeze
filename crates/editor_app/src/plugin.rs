//! Bevy Studio 编辑器插件：把编辑器核心、界面与 Bevy 应用装配起来。
//!
//! # 数据是怎样流动的
//!
//! 每一帧都按同一个顺序走完整个回路，没有例外：
//!
//! ```text
//! ① 读键盘快捷键           → 产生 UiAction
//! ② 处理异步文件对话框结果  → 产生 UiAction
//! ③ 逐条执行 UiAction      → 修改编辑器状态 / 启动停止运行时
//! ④ 推进运行时世界         → 只影响运行时自己的 World
//! ⑤ 绘制界面               → 只读编辑器状态，产生新的 UiAction
//! ⑥ 收集运行时统计         → 供下一帧界面显示
//! ```
//!
//! 注意 ③ 在 ⑤ **之前**：这样用户在这一帧点下的操作，下一帧就能看到结果，
//! 而不是延迟两帧。同时 ⑤ 产生的动作会留到下一帧的 ③ 执行 ——
//! 因此界面永远不会在同一帧里"边读边改"编辑器状态。
//!
//! # 两个世界的边界
//!
//! 编辑器数据住在 [`EditorStateHolder`] 里（一个普通 Rust 结构体，不是 ECS 资源世界）；
//! 游戏运行时数据住在 [`运行时宿主`] 自己的 `World` 里。
//! 两者之间只有一条单向通道（[`crate::scene_bridge::构建世界`]），
//! 没有任何回写路径。

use std::collections::BTreeMap;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use editor_core::state::编辑器状态;
use editor_ui::{
    state::UiAction, 动作队列, 界面状态, 绘制上下文, 视口渲染结果, 运行时诊断
};

use crate::scene_bridge::{运行时宿主, 运行状态, 默认组件注册表};
use crate::{
    actions,
    config::{布局待保存, 编辑器配置, 编辑器配置资源, 配置文件路径},
    preview,
};

/// 编辑器状态容器。
///
/// 刻意用 `Resource` 包住一个普通结构体，而不是把编辑器数据拆成 ECS 组件：
/// 编辑器文档是**树状文档**，不是 ECS 世界。把它硬塞进 ECS 只会得到
/// 一堆带 `Parent` 的贫血实体，然后每个操作都要靠查询拼回来。
#[derive(Resource)]
pub struct EditorStateHolder {
    /// 编辑器权威状态。
    pub 状态: 编辑器状态,
}

impl Default for EditorStateHolder {
    fn default() -> Self {
        let mut 状态 = 编辑器状态::新建();
        状态.注册表 = 默认组件注册表();
        状态.信息("微风已启动。请新建或打开一个项目。");
        Self { 状态 }
    }
}

/// 界面侧的状态容器。
#[derive(Resource, Default)]
pub struct UiStateHolder {
    /// 界面自有状态。
    pub 状态: 界面状态,
    /// 本帧累积的动作队列。
    pub 动作: 动作队列,
}

impl UiStateHolder {
    /// 新建。
    pub fn 新建() -> Self {
        Self {
            状态: 界面状态::新建(),
            动作: 动作队列::新建(),
        }
    }
}

// 配色不再是独立资源：它是**主题**的一部分，住在 界面状态.主题库 里。
// 早期版本把配色单列成资源，导致"主题改了但某个面板还在用旧配色"。

/// 图标纹理缓存（全局一份，跨帧复用）。
#[derive(Resource, Default)]
pub struct 图标资源(pub editor_ui::图标库);

/// 缩略图缓存：资源路径文本 → egui 纹理。
///
/// 纹理只属于界面，**绝不能**进场景文档：纹理句柄与帧绑定，
/// 存进文件既无意义又会立刻失效。
#[derive(Resource, Default)]
pub struct 缩略图缓存 {
    /// 纹理表。
    pub 纹理: BTreeMap<String, egui::TextureHandle>,
    /// 已经尝试加载过的路径（含失败的），避免每帧重复读盘。
    pub 已尝试: std::collections::HashSet<String>,
}

/// 运行时诊断资源，由运行时推进系统写入、界面读取。
#[derive(Resource, Default)]
pub struct 诊断资源(pub 运行时诊断);

/// 构建快照资源：界面读的构建配置 + 最近一次报告。
///
/// `报告修订` 是**缓存指纹**而不是业务数据：只有它变了才重新克隆报告，
/// 否则每帧克隆几十条带字符串的诊断会成为稳定的每秒几 MB 分配。
#[derive(Resource, Default)]
pub struct 构建快照资源 {
    /// 供界面读取的快照。
    pub 快照: editor_core::build::构建快照,
    /// 快照对应的构建报告修订号。
    pub 报告修订: u64,
}

/// 资源导入快照：界面读的"每个资源处于什么导入状态"。
///
/// # 为什么不让面板自己扫描
///
/// `项目.扫描导入()` 要遍历资源目录并读取每个文件的元数据。资源面板每帧都画，
/// 如果每帧扫一次，上千资源的项目会在纯绘制上吃掉可观时间。
/// 这里按"项目变了 / 用户点了导入"才重算一次，面板只读结果。
#[derive(Resource, Default)]
pub struct 导入快照资源 {
    /// 项目相对路径文本 → 导入状态。
    pub 状态表: std::collections::BTreeMap<String, editor_core::project::导入状态>,
    /// 一句话摘要（新增/变更/最新/缺失计数）。
    pub 摘要: String,
    /// 状态表对应的项目根目录。换项目时必须重算。
    pub 最近项目根: Option<std::path::PathBuf>,
    /// 需要重算的标记（导入动作、资源变更会置位）。
    pub 需刷新: bool,
}

/// 文件对话框请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 文件对话框请求 {
    /// 选择"打开项目"的目录。
    选择项目目录,
    /// 选择"新建项目"的父目录。
    选择新建项目目录 {
        /// 项目名。
        名称: String,
    },
    /// 选择"场景另存为"的目标文件。
    场景另存为,
    /// 选择要打开的场景文件。
    打开场景文件,
    /// 选择要实例化到当前场景的场景文件。
    实例化场景文件,
    /// 选择"另存为子场景"的目标文件。
    子场景另存为,
    /// 选择要重命名的资源。
    重命名资源(PathBuf),
    /// 确认删除资源。
    确认删除资源(PathBuf),
    /// 把主题 RON 文本导出到用户选择的文件。
    导出主题(String),
    /// 让用户选择一个主题文件并读入。
    导入主题,
    /// 脚本另存为：携带待写入的文本。
    脚本另存为(String),
    /// 打开一个动画文件（`.anim.ron`）。
    打开动画文件,
    /// 动画另存为。
    动画另存为,
}

/// 文件对话框结果。
#[derive(Debug, Clone)]
pub enum 文件对话框结果 {
    /// 选择了一个目录。
    选定目录(PathBuf),
    /// 选择了一个文件。
    选定文件(PathBuf),
    /// 选择了一个要打开的场景。
    打开场景(PathBuf),
    /// 选择了一个要实例化的场景。
    实例化场景(PathBuf),
    /// 选择了一个"另存为子场景"的目标文件。
    子场景保存到(PathBuf),
    /// 主题已成功导出到该路径。
    主题已导出(PathBuf),
    /// 脚本已保存到该路径。
    脚本已保存(PathBuf),
    /// 读取到的主题文本。
    主题文本 {
        /// 来源路径（用于日志）。
        路径: PathBuf,
        /// 文件内容。
        文本: String,
    },
    /// 新建项目的父目录 + 项目名。
    新建项目 {
        /// 项目名。
        名称: String,
        /// 父目录。
        父目录: PathBuf,
    },
    /// 用户选中的动画文件（打开）。
    打开动画(PathBuf),
    /// 用户为动画选定的保存路径。
    动画保存到(PathBuf),
    /// 用户取消。
    取消,
}

/// 文件对话框通道。
///
/// `rfd` 的原生对话框是**阻塞**的，直接在 Bevy 系统里调用会让编辑器
/// 冻住（窗口不再重绘、鼠标不动）。因此把对话框丢到后台线程，
/// 结果通过 channel 回传，主线程每帧非阻塞地取一次。
#[derive(Resource)]
pub struct 文件对话框通道 {
    /// 发送请求。
    pub 请求发送端: crossbeam_channel::Sender<文件对话框请求>,
    /// 接收结果。
    pub 结果接收端: crossbeam_channel::Receiver<文件对话框结果>,
    /// 是否有请求正在处理（避免同时弹两个对话框）。
    pub 处理中: bool,
}

impl Default for 文件对话框通道 {
    fn default() -> Self {
        let (请求发送端, 请求接收端) = crossbeam_channel::unbounded();
        let (结果发送端, 结果接收端) = crossbeam_channel::unbounded();

        std::thread::Builder::new()
            .name("bevy-studio-文件对话框".to_string())
            .spawn(move || 对话框工作线程(请求接收端, 结果发送端))
            .expect("应当能启动文件对话框线程");

        Self {
            请求发送端,
            结果接收端,
            处理中: false,
        }
    }
}

/// 后台线程：等待请求 → 弹对话框 → 回传结果。
fn 对话框工作线程(
    请求接收端: crossbeam_channel::Receiver<文件对话框请求>,
    结果发送端: crossbeam_channel::Sender<文件对话框结果>,
) {
    while let Ok(请求) = 请求接收端.recv() {
        let 结果 = 处理请求(&请求);
        if 结果发送端.send(结果).is_err() {
            // 主线程已经退出，结束线程。
            break;
        }
    }
}

/// 实际弹出对话框。
fn 处理请求(请求: &文件对话框请求) -> 文件对话框结果 {
    match 请求 {
        文件对话框请求::选择项目目录 => match rfd::FileDialog::new()
            .set_title("选择微风项目目录")
            .pick_folder()
        {
            Some(目录) => 文件对话框结果::选定目录(目录),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::选择新建项目目录 { 名称 } => match rfd::FileDialog::new()
            .set_title("选择用于新建项目的目录（必须为空）")
            .pick_folder()
        {
            Some(父目录) => 文件对话框结果::新建项目 {
                名称: 名称.clone(),
                父目录,
            },
            None => 文件对话框结果::取消,
        },

        文件对话框请求::场景另存为 => match rfd::FileDialog::new()
            .set_title("场景另存为")
            .add_filter("微风场景", &["ron"])
            .set_file_name("新场景.scene.ron")
            .save_file()
        {
            // 用**专用结果**而不是通用的 `选定文件`：
            // 旧实现让"打开场景"和"场景另存为"共用一个结果，
            // 处理时只能靠"文件存不存在"猜意图 —— 结果"打开场景"
            // 实际执行成了"另存为"，是个会覆盖文件的真 bug。
            Some(文件) => 文件对话框结果::选定文件(文件),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::打开场景文件 => match rfd::FileDialog::new()
            .set_title("打开场景")
            .add_filter("微风场景", &["ron"])
            .pick_file()
        {
            Some(文件) => 文件对话框结果::打开场景(文件),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::实例化场景文件 => match rfd::FileDialog::new()
            .set_title("实例化场景")
            .add_filter("微风场景", &["ron"])
            .pick_file()
        {
            Some(文件) => 文件对话框结果::实例化场景(文件),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::子场景另存为 => match rfd::FileDialog::new()
            .set_title("另存为子场景")
            .add_filter("微风场景", &["ron"])
            .set_file_name("子场景.scene.ron")
            .save_file()
        {
            Some(文件) => 文件对话框结果::子场景保存到(文件),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::导出主题(文本) => match rfd::FileDialog::new()
            .set_title("导出主题")
            .add_filter("微风主题", &["ron"])
            .set_file_name("我的主题.theme.ron")
            .save_file()
        {
            Some(路径) => match std::fs::write(&路径, 文本) {
                Ok(()) => 文件对话框结果::主题已导出(路径),
                Err(_) => 文件对话框结果::取消,
            },
            None => 文件对话框结果::取消,
        },

        文件对话框请求::脚本另存为(文本) => match rfd::FileDialog::new()
            .set_title("脚本另存为")
            .add_filter("Rust 源文件", &["rs"])
            .set_file_name("script.rs")
            .save_file()
        {
            Some(路径) => match std::fs::write(&路径, 文本) {
                Ok(()) => 文件对话框结果::脚本已保存(路径),
                Err(_) => 文件对话框结果::取消,
            },
            None => 文件对话框结果::取消,
        },

        文件对话框请求::导入主题 => match rfd::FileDialog::new()
            .set_title("导入主题")
            .add_filter("微风主题", &["ron"])
            .pick_file()
        {
            Some(路径) => match std::fs::read_to_string(&路径) {
                Ok(文本) => 文件对话框结果::主题文本 { 路径, 文本 },
                Err(_) => 文件对话框结果::取消,
            },
            None => 文件对话框结果::取消,
        },

        文件对话框请求::打开动画文件 => match rfd::FileDialog::new()
            .set_title("打开动画")
            .add_filter("微风动画", &["ron"])
            .pick_file()
        {
            Some(路径) => 文件对话框结果::打开动画(路径),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::动画另存为 => match rfd::FileDialog::new()
            .set_title("动画另存为")
            .add_filter("微风动画", &["ron"])
            .set_file_name("animation.anim.ron")
            .save_file()
        {
            Some(路径) => 文件对话框结果::动画保存到(路径),
            None => 文件对话框结果::取消,
        },

        // 重命名与删除需要文本输入与二次确认，本版本尚未提供对应对话框，
        // 因此如实返回"取消"，而不是假装成功。
        文件对话框请求::重命名资源(_) | 文件对话框请求::确认删除资源(_) => {
            文件对话框结果::取消
        }
    }
}

/// 编辑器插件。
pub struct BevyStudio插件;

impl Plugin for BevyStudio插件 {
    fn build(&self, app: &mut App) {
        // egui 的 primary context 由 `preview::初始化预览资源` 显式管理，
        // 因为本编辑器有两台相机（预览 + 界面），必须明确指定哪一台
        // 承载界面。默认行为是"挂到第一台相机上"，那会挂错。
        app.add_plugins(EguiPlugin::default())
            // R13 原型：Play-in-Editor 最小实时渲染实验（见 pie_proto 模块头）。
            .add_plugins(crate::pie_proto::Pie原型插件)
            .init_resource::<EditorStateHolder>()
            .insert_resource(UiStateHolder::新建())
            // 资源预览：单击资源后读盘/解码/解析，把结果写回界面状态。
            // 它的系统放在 Update 里，与"改数据"的其它系统同一条时间线。
            .add_plugins(crate::asset_preview::资源预览插件)
            .init_resource::<缩略图缓存>()
            .init_resource::<crate::tilemap_textures::瓦片图集纹理>()
            .init_resource::<图标资源>()
            .init_resource::<诊断资源>()
            .init_resource::<运行时宿主>()
            // 物理运行时状态：只在运行态有内容，停止后置空。
            .init_resource::<crate::physics_runtime::物理运行时状态>()
            .init_resource::<crate::agent_runtime::智能体运行>()
            // 自动保存：只在有未保存修改时按间隔写恢复点。
            .init_resource::<crate::recovery_autosave::自动保存状态>()
            // AI 会话存档：启动载入、运行期防抖保存、退出前强制保存。
            .init_resource::<crate::agent_persistence::会话存档状态>()
            .init_resource::<crate::run::项目运行管理器>()
            // 运行时调试桥：监听 loopback、握手、心跳与只读 ECS 查询。
            .init_resource::<crate::debug_bridge::运行时调试管理器>()
            // 运行快照：界面读的运行时状态（不含 Child/线程等进程细节）。
            .init_resource::<crate::run::运行快照资源>()
            // 统一诊断中心：构建 / 运行 / Agent 三个来源汇到同一份数据。
            .init_resource::<crate::diagnostics::诊断中心>()
            .init_resource::<crate::diagnostics::诊断快照资源>()
            // 构建任务：真实 cargo 子进程 + 界面读的快照。
            .init_resource::<crate::build::构建宿主>()
            .init_resource::<构建快照资源>()
            // 资源导入状态：按需重算，避免每帧扫资源目录。
            .init_resource::<导入快照资源>()
            // 灯光预览：把场景里的灯光组件投影成 Bevy 光源。
            .init_resource::<crate::light_preview::灯光同步状态>()
            // 界面预览：把场景里的 UI 组件投影成 Bevy UI 节点。
            .init_resource::<crate::ui_preview::界面同步状态>()
            // 音频试听：把「试听」请求变成真实播放。
            .init_resource::<crate::audio_preview::试听状态>()
            .init_resource::<文件对话框通道>()
            // ---- 配置与布局持久化 ----
            //
            // 启动时读一次；布局写盘由 `actions::保存窗口布局` 定期完成。
            // 配置文件放在用户配置目录，与项目/场景文件完全无关。
            .insert_resource(编辑器配置资源(编辑器配置::载入(&配置文件路径())))
            .init_resource::<布局待保存>()
            // ---- GUI 采集（自动验收用，默认不启用） ----
            //
            // 只有设置了 `BEVY_STUDIO_CAPTURE_PNG` 才会真正工作：
            // 它在指定帧把窗口画面（渲染目标回读）与布局证据写入磁盘，
            // 供 `gui_probe` 离线判定。正常使用时每帧只多一次 `Option` 判断。
            .insert_resource(crate::capture::采集配置::从环境变量())
            // ---- 预览视口 ----
            //
            // 必须在界面绘制之前创建相机与渲染目标并完成纹理注册，
            // 否则首帧视口会退化成只有网格。
            .init_resource::<preview::预览相机状态>()
            .add_systems(
                Startup,
                (
                    // 注册表扩展必须**最先**执行：打开项目时会用注册表校验
                    // 场景里的组件，晚一步就会把灯光报成"未注册组件"。
                    安装扩展组件,
                    preview::初始化预览资源,
                    按环境变量打开项目,
                    // 会话存档必须在项目打开之后载入：只有那时才能选中
                    // "当前项目最近的会话"，也才能校验项目是否仍可用。
                    crate::agent_persistence::载入会话存档,
                    按环境变量设置设计分辨率,
                    按环境变量自动运行,
                    // 恢复布局必须在"聚焦面板"**之前**：恢复会用磁盘上的
                    // 布局整体替换当前布局，若顺序反了，环境变量让某个面板
                    // 前台化、紧接着又被旧布局覆盖 —— 表现是"面板明明在，
                    // 截图里却没有"，而日志里两件事都成功。
                    恢复窗口布局,
                    // 放在打开项目与恢复布局之后：那时才有选中实体，
                    // Inspector 才有内容，且聚焦不会被随后的布局恢复抹掉。
                    按环境变量聚焦面板,
                    按环境变量设置语言,
                    // 最近项目：把配置里的列表镜像到界面状态，并把本次启动
                    // 打开的项目记进列表（环境变量打开的项目也要能被"最近"找到）。
                    恢复最近项目,
                    按环境变量试听音频,
                    按环境变量显示碰撞,
                    按环境变量打开项目设置,
                    按环境变量打开帮助,
                    按环境变量设置变换工具,
                    按环境变量打开主题编辑器,
                    按环境变量打开脚本,
                    // 放在最后：它需要一个已打开的项目与已恢复的布局，
                    // 否则「构建并运行」会落在"没有项目"的分支上。
                    按环境变量构建并运行,
                    按环境变量直接运行项目,
                    打印启动信息,
                )
                    .chain(),
            )
            // 资源对话框需要"项目已打开、场景已载入"之后才能算引用，
            // 因此单独放在启动链之后，并显式排在打开项目之后
            // （Bevy 的系统元组上限是 20 个，那条链已经很长了）。
            .add_systems(Startup, 按环境变量打开资源对话框.after(按环境变量打开项目))
            // 恢复点扫描必须在项目打开之后：它要按项目根去找
            // `.breeze/recovery/recovery.ron`。
            .add_systems(
                Startup,
                crate::recovery_autosave::启动扫描恢复点.after(按环境变量打开项目),
            )
            .add_systems(Startup, 按环境变量打开恢复对话框.after(按环境变量打开项目))
            .add_systems(Startup, 按环境变量演示动画事件.after(按环境变量打开项目))
            .add_systems(Startup, 按环境变量多选.after(按环境变量打开项目))
            .add_systems(Startup, 按环境变量刷新实例.after(按环境变量打开项目))
            .add_systems(Startup, 按环境变量打开存档浏览器.after(按环境变量打开项目))
            // ---- Update：只改数据，不碰 egui ----
            //
            // 顺序即数据流，见模块头部说明。
            // ---- Last：退出前把配置落盘 ----
            .add_systems(
                Last,
                (
                    actions::退出时保存配置,
                    // 先强制保存 AI 会话，再保存窗口布局：两者互不依赖，
                    // 但会话是用户资产，退出路径上要给足机会落盘。
                    crate::agent_persistence::退出时保存会话,
                    crate::run::退出时停止项目运行,
                    crate::build::退出时停止构建,
                ),
            )
            // 自动保存必须排在动作执行之后：本帧的编辑要进入本次恢复点。
            .add_systems(
                Update,
                crate::recovery_autosave::自动保存
                    .after(actions::执行动作队列)
                    .after(crate::capture::采集收尾),
            )
            .add_systems(
                Update,
                (
                    actions::处理快捷键,
                    actions::处理文件对话框结果,
                    actions::执行动作队列,
                    // 构建状态推进必须紧跟动作执行：刚点下"开始构建"的那一帧
                    // 之后才轮询，日志与"构建中"状态才能在同一帧反映到界面。
                    actions::轮询构建,
                    actions::同步构建快照,
                    // 导入状态按需重算：项目变了或用户点了导入才扫盘。
                    actions::同步导入快照,
                    // 动画时间在动作之后推进：本帧点了"播放"就立刻开始走时间。
                    actions::推进动画播放,
                    // 紧跟动作执行：本帧产生的日志（含错误）立刻出现在进程日志里。
                    actions::转发控制台日志到标准错误,
                    actions::读取键盘导航,
                    // 采集收尾：等截图落盘后退出。放在这一链的末尾，
                    // 保证"本帧的日志先转发出去"再决定退出。
                    crate::capture::采集收尾,
                    // 会话存档防抖保存：任务结束时立即写，运行中最多每 600ms。
                    crate::agent_persistence::保存会话存档,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    // 先把文档投影成渲染实体，再同步相机与纹理尺寸。
                    // 分成两个三元组再 `chain`：Bevy 的 `chain` 对元组
                    // 元素个数有上限，6 个直接链会编译不过。
                    (
                        // 外部改 `.terrain.ron` 时先让同步状态失效，下一帧重烘。
                        preview::监视地形文件,
                        preview::同步三维相机,
                        preview::同步预览实体,
                        // 灯光落在几何体之后：它只依赖文档，与几何体互不影响，
                        // 但放在同一链里能保证"同一帧的文档版本"看到同样的内容。
                        crate::light_preview::同步场景灯光,
                        // 界面元素同样按修订号重建；它锁定预览相机，
                        // 因此必须排在预览相机同步之后。
                        crate::ui_preview::同步界面元素,
                        // 音频试听是事件驱动的（用户点按钮），不需要按修订号重建。
                        crate::audio_preview::处理试听请求,
                        preview::同步预览相机,
                        // AI Agent：探测/启动/收事件/取消，全部非阻塞。
                        crate::agent_runtime::推进智能体,
                        preview::同步渲染尺寸,
                        preview::模型诊断,
                        crate::run::轮询项目运行,
                        crate::run::同步运行快照,
                        // 调试事件必须紧跟运行状态：游戏退出后立刻把会话标成断开。
                        crate::debug_bridge::推进调试,
                        // GUI 验收专用（没有环境变量时是空操作）：用真实
                        // 管理器方法与真实游戏完成成功/拒绝/快照差异。
                        crate::runtime_acceptance::驱动运行时验收,
                        // 「构建并运行」的一次性意图在这里消费：
                        // 只有真实构建成功（退出码 0）才启动游戏。
                        actions::构建完成后启动运行,
                        crate::diagnostics::同步诊断快照,
                        同步项目运行状态,
                        输入诊断,
                    )
                        .chain(),
                    (
                        actions::推进运行时,
                        actions::收集运行时诊断,
                        actions::应用动画预览,
                    )
                        .chain(),
                )
                    .chain(),
            )
            // ---- EguiPrimaryContextPass：把数据画出来 ----
            //
            // 界面绘制必须放在这个调度里。原因见
            // `actions::绘制编辑器界面` 的文档注释：egui 的字体是在
            // `Context::run()` 里加载的，而本调度正是在 `Context::run()`
            // 的闭包内部被执行。放到 `Update` 会因为字体尚未加载而 panic。
            .add_systems(
                EguiPrimaryContextPass,
                (
                    // 字体必须先于任何文字绘制装好，否则首帧的中文是方框。
                    确保中文字体已安装,
                    // 瓦片图集纹理要先于面板绘制：面板本帧就要用 UV 取瓦片。
                    crate::tilemap_textures::同步瓦片图集纹理,
                    actions::绘制编辑器界面,
                    // 采集必须紧跟界面绘制：它读的是本帧刚测出的布局矩形，
                    // 并在 `ExtractSchedule` 之前请求截图，从而与画面同帧。
                    crate::capture::采集一帧,
                    actions::保存窗口布局,
                )
                    .chain(),
            );
    }
}

/// 确保中文字体已安装（幂等，只做一次）。
///
/// # 为什么放在这里而不是 `Startup`
///
/// `egui::Context` 由 `bevy_egui` 在**它自己的** `PreStartup`
/// （`EguiStartupSet::InitContexts`）里创建，并且创建在我们显式声明
/// `PrimaryEguiContext` 的那台相机上。跨插件的启动系统执行顺序不容易保证，
/// 因此改成：在界面绘制之前检查一次，装完打标记，
/// 之后每帧只判断一个布尔量（代价可以忽略）。
///
/// # 为什么不会失败
///
/// 字体是用 `include_bytes!` 编进二进制的（见 `editor_ui::fonts`），
/// 因此不存在"文件找不到"的情况。唯一可能的问题是内置字体本身
/// 缺少某个字形 —— 那由 `editor_ui` 的
/// `内置字体覆盖界面源码里的全部汉字` 测试在编译前就挡住。
pub fn 确保中文字体已安装(mut 上下文: EguiContexts, mut 已安装: Local<bool>) {
    if *已安装 {
        return;
    }
    let Ok(上下文) = 上下文.ctx_mut() else {
        // 上下文还没建立，下一帧再试。
        return;
    };

    let 结果 = editor_ui::安装中文字体(上下文);
    info!(
        "已安装中文字体（内置子集 {:.1} MB）",
        editor_ui::内置字体大小() as f32 / 1024.0 / 1024.0
    );
    if let Some(路径) = 结果.后备字体 {
        info!(
            "已追加后备字体：{}（仅在内置子集缺字时使用）",
            路径.display()
        );
    }
    *已安装 = true;

    // 换字体会让 egui 重建字体图集并重新排版。
    // 主动请求重绘，避免首帧用旧字体画出的方框被缓存住。
    上下文.request_repaint();
}
/// 允许用环境变量在启动时直接打开一个项目与场景。
///
/// # 为什么需要这个入口
///
/// 正常流程是「菜单 → 打开项目 → 弹系统目录对话框」。那个对话框是原生窗口，
/// 自动化脚本点不了，于是**无法在无人值守的情况下验证"有内容的项目长什么样"**
/// —— 而"视口能不能正确渲染场景里的精灵"恰恰只能这样验证。
///
/// 有了它，验证就变成一条命令：
///
/// ```powershell
/// $env:BEVY_STUDIO_OPEN_PROJECT = "C:\path\to\项目"
/// $env:BEVY_STUDIO_OPEN_SCENE   = "scenes/关卡.scene.ron"   # 可选
/// .\bevy-studio.exe
/// ```
///
/// 它同时也是 CI 截图、以及用户报 bug 时"打开这个项目就能复现"的入口。
pub const 打开项目环境变量: &str = "BEVY_STUDIO_OPEN_PROJECT";
/// 启动时按名称前缀选中一个实体（截图与自动验收用，只改选择集）。
pub const 选中实体环境变量: &str = "BEVY_STUDIO_SELECT";
/// 指定要打开的场景（项目相对路径）。留空则用启动场景或第一个场景。
pub const 打开场景环境变量: &str = "BEVY_STUDIO_OPEN_SCENE";
/// 指定要打开的动画（项目相对路径，如 `animations/关卡.anim.ron`）。
///
/// 动画的打开入口在时间轴面板里，而那个面板需要一个**来自文件对话框**的路径。
/// 与场景同理，自动化脚本点不了原生对话框，因此给一条启动期入口：
/// 有了它就能对着真实动画截时间轴的图，而不是只能截一张空面板。
pub const 打开动画环境变量: &str = "BEVY_STUDIO_OPEN_ANIMATION";

/// 启动时按环境变量打开项目与场景。未设置时什么都不做。
fn 按环境变量打开项目(mut 编辑器: ResMut<EditorStateHolder>) {
    let Ok(项目目录) = std::env::var(打开项目环境变量) else {
        return;
    };
    if 项目目录.trim().is_empty() {
        return;
    }

    match 编辑器.状态.打开项目(&项目目录) {
        Ok(()) => info!("已按环境变量打开项目：{项目目录}"),
        Err(错误) => {
            warn!("按环境变量打开项目失败：{错误}");
            return;
        }
    }

    // 显式指定场景时再打开一次（打开项目时已自动载入了启动场景）。
    if let Ok(场景) = std::env::var(打开场景环境变量)
        && !场景.trim().is_empty()
    {
        match scene_doc::path::项目相对路径::解析(&场景) {
            Ok(路径) => match 编辑器.状态.打开场景(&路径) {
                Ok(()) => info!("已按环境变量打开场景：{场景}"),
                Err(错误) => warn!("按环境变量打开场景失败：{错误}"),
            },
            Err(错误) => warn!("环境变量里的场景路径非法：{错误}"),
        }
    }

    // 截图/验收用：按名称前缀选中一个实体，让检查器有内容可看。
    //
    // 为什么要这个入口：场景树/检查器的"选中态"是这批界面最重要的状态之一，
    // 而在无人值守的截图环境里，注入鼠标点击依赖窗口真的拿到前台焦点
    // （实测会被 Windows 的前台锁拒掉，表现为 pointer 事件根本没到达应用）。
    // 有了它，就能在**不点鼠标**的情况下把界面摆到"选中一个房屋"的状态，
    // 再交给 `gui_probe` 从渲染目标回读。
    //
    // 它只改选择集（视图状态），不动文档、不进撤销栈。
    if let Ok(目标) = std::env::var(选中实体环境变量)
        && !目标.trim().is_empty()
    {
        let 目标 = 目标.trim();
        let 文档 = &编辑器.状态.会话.文档;
        let 候选 = 文档
            .深度优先遍历()
            .into_iter()
            .map(|(标识, _)| 标识)
            .find(|标识| 文档.取(*标识).is_some_and(|实体| 实体.名称.contains(目标)));
        match 候选 {
            Some(标识) => {
                编辑器.状态.选择.单选(标识);
                info!("已按环境变量选中实体：{目标}");
            }
            None => warn!("环境变量 {选中实体环境变量} 没有匹配到任何实体：{目标}"),
        }
    }

    // 动画同理：必须在场景打开之后，动画的轨道要对着场景里的实体才有意义。
    if let Ok(动画) = std::env::var(打开动画环境变量)
        && !动画.trim().is_empty()
    {
        match scene_doc::path::项目相对路径::解析(&动画) {
            Ok(路径) => match 编辑器.状态.打开动画(&路径) {
                Ok(()) => info!("已按环境变量打开动画：{动画}"),
                Err(错误) => warn!("按环境变量打开动画失败：{错误}"),
            },
            Err(错误) => warn!("环境变量里的动画路径非法：{错误}"),
        }
    }
}

/// 指定启动时打开哪个源码文件（项目相对路径，如 `src/main.rs`）。
///
/// # 为什么源码也需要一个环境变量入口
///
/// 原生 Cargo 工程没有微风场景文件，"打开项目后能浏览并打开 Rust 源文件"
/// 是它的主要用途之一。自动验收要证明这条链路，就必须能在**不点鼠标**
/// 的情况下打开一个确定的文件 —— 否则"点开组合框再选中一项"会混进
/// 输入注入的不确定性里，验不清到底坏在哪。
pub const 打开脚本环境变量: &str = "BEVY_STUDIO_OPEN_SCRIPT";

/// 启动时按环境变量请求打开一个源码文件。
///
/// 只排一个 [`UiAction::打开脚本`] 进动作队列，真正的读取/切模式/聚焦
/// 仍然走用户点击时的同一条链路 —— 不另开一条"启动专用"的旁路。
fn 按环境变量打开脚本(
    编辑器: Res<EditorStateHolder>, mut 界面: ResMut<UiStateHolder>
) {
    let Ok(相对文本) = std::env::var(打开脚本环境变量) else {
        return;
    };
    let 相对文本 = 相对文本.trim();
    if 相对文本.is_empty() {
        return;
    }
    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        warn!("环境变量 {打开脚本环境变量} 要求打开脚本，但当前没有打开项目");
        return;
    };
    match scene_doc::path::项目相对路径::解析(相对文本) {
        Ok(路径) => {
            let 绝对 = 项目.绝对路径(&路径);
            if !绝对.is_file() {
                warn!(
                    "环境变量 {打开脚本环境变量} 指定的脚本不存在：{}",
                    绝对.display()
                );
                return;
            }
            界面.动作.推(editor_ui::UiAction::打开脚本(路径));
            info!("已按环境变量请求打开脚本：{相对文本}");
        }
        Err(错误) => warn!("环境变量 {打开脚本环境变量} 的路径不合法：{相对文本}（{错误}）"),
    }
}

/// 启动时同步"最近项目"。
///
/// 两份数据必须一致：配置文件里的是**持久化真相**，界面状态里的是
/// **本帧要显示的那份**。启动时镜像一次，之后由动作层同时更新两者。
fn 恢复最近项目(
    mut 界面: ResMut<UiStateHolder>,
    mut 配置: ResMut<编辑器配置资源>,
    mut 布局待保存: ResMut<布局待保存>,
    编辑器: Res<EditorStateHolder>,
) {
    if let Some(项目) = 编辑器.状态.项目.as_ref()
        && 配置.0.记录最近项目(项目.根目录())
    {
        // 新项目进了列表：让配置在下一个攒批周期落盘。
        布局待保存.脏 = true;
    }
    界面.状态.最近项目 = 配置.0.最近项目列表.clone();
}

/// 启动时装入完整的内置组件注册表。
///
/// `EditorStateHolder::default` 只装基础组件（BSN 提取依赖"恰好 6 个"这个
/// 边界）；完整表由 `scene_bridge::全部内置组件注册表` 统一提供。这里整体
/// 替换。**幂等**，重复执行也不会出现重复注册错误。
fn 安装扩展组件(mut 编辑器: ResMut<EditorStateHolder>) {
    let 注册表 = crate::scene_bridge::全部内置组件注册表();
    let 数量 = 注册表.数量();
    编辑器.状态.注册表 = 注册表;
    info!("已安装扩展组件，注册表中共 {数量} 个组件");
}

/// 指定启动时要切到前台的面板（用「窗口」菜单里的词条键，如 `inspector`）。
pub const 启动聚焦面板环境变量: &str = "BEVY_STUDIO_FOCUS_PANEL";

/// 启动时直接打开「项目设置」对话框（自动验收用）。
///
/// # 为什么需要这条
///
/// 对话框只能从菜单打开，而菜单需要真实点击 —— 截图脚本点不了。
/// 没有这个入口，"项目设置对话框长什么样、有没有真的列出清单字段"
/// 就只能靠人看，和之前 `BEVY_STUDIO_OPEN_THEME_EDITOR` 是同一个理由。
pub const 打开项目设置环境变量: &str = "BEVY_STUDIO_OPEN_PROJECT_SETTINGS";

/// 启动时直接打开资源重命名/删除对话框（自动验收用）。
///
/// 取值 `rename`、`delete` 或 `refs`（查找引用）。只用于截图与回归，不是产品功能；
/// 它打开的是**真实的**对话框状态，引用列表也是真索引算出来的。
pub const 资源对话框环境变量: &str = "BEVY_STUDIO_OPEN_ASSET_DIALOG";

/// 启动时选中前 N 个实体（自动验收用）。
///
/// 3D 多选没有可自动化的"按住 Ctrl 点击"——`gui_probe` 的指针注入目前
/// 不带修饰键。这个入口只把**选择集**摆成多选状态，用来验证"多选在三维
/// 视口里确实被画出来、Inspector 也认得"，不假装验证了点击路径。
pub const 多选验收环境变量: &str = "BEVY_STUDIO_SELECT_COUNT";

/// 启动时选中第一个场景实例并触发「刷新场景实例」（自动验收用）。
///
/// 真实菜单需要展开两层菜单才能点到，`gui_probe` 做不到；这个入口只是
/// 把"用户点击那条菜单项"换成环境变量触发，动作本身完全相同。
pub const 刷新实例验收环境变量: &str = "BEVY_STUDIO_REFRESH_INSTANCE";

/// 启动时打开「游戏存档槽位」浏览器（自动验收用）。
///
/// 菜单需要展开一层才能点到，`gui_probe` 做不到；这个入口把
/// "用户点菜单"换成环境变量，动作本身完全相同。
pub const 存档浏览器验收环境变量: &str = "BEVY_STUDIO_OPEN_SAVES";

/// 启动时生成一条**真实**恢复点并打开恢复对话框（自动验收用）。
///
/// 只用于截图与回归：它用当前场景真实内容写恢复点，再走与启动扫描
/// 完全相同的路径，不构造假数据。
pub const 恢复对话框环境变量: &str = "BEVY_STUDIO_OPEN_RECOVERY";

/// 启动时直接打开帮助窗口（自动验收用）。
pub const 打开帮助环境变量: &str = "BEVY_STUDIO_OPEN_SHORTCUTS";

/// 启动时直接打开「关于」窗口（自动验收用）。
///
/// 与帮助窗口同理：它只能从菜单打开，而截图脚本点不到菜单。
/// 没有这个入口，关于窗口的排版改动就只能靠人眼确认。
pub const 打开关于环境变量: &str = "BEVY_STUDIO_OPEN_ABOUT";

/// 启动时按环境变量打开帮助窗口，以及「关于」窗口。
///
/// 与项目设置同理：这两个窗口只能从菜单/F1 打开，截图脚本按不了键。
/// 两者共用同一个系统，而不是各占一个：Bevy 的系统元组有元数上限，
/// 这里再多挂一个就会编译失败（报的是"trait bounds were not satisfied"，
/// 与真正的原因相距很远）。一个系统看两个环境变量，代价只是多一次 `var`。
fn 按环境变量打开帮助(mut 界面: ResMut<UiStateHolder>) {
    if std::env::var(打开帮助环境变量).is_ok() {
        界面.状态.显示帮助 = true;
        info!("已按环境变量打开帮助窗口");
    }
    if std::env::var(打开关于环境变量).is_ok() {
        界面.状态.显示关于 = true;
        info!("已按环境变量打开关于窗口");
    }
}

/// 启动时显示碰撞体轮廓（自动验收用).
///
/// 与其它环境变量同理：视口工具栏的开关点不到，验收就只能拍一张
/// "看不出碰撞体在哪"的图。
pub const 显示碰撞环境变量: &str = "BEVY_STUDIO_SHOW_COLLISIONS";

/// 按环境变量打开碰撞轮廓显示。
fn 按环境变量显示碰撞(mut 界面: ResMut<UiStateHolder>) {
    if std::env::var(显示碰撞环境变量).is_err() {
        return;
    }
    界面.状态.相机.显示碰撞 = true;
    info!("已按环境变量显示碰撞轮廓");
}

/// 启动时自动执行一次「构建并运行」（自动验收用）。
///
/// # 为什么需要这个入口
///
/// 「构建并运行」是一个图形界面动作。没有它，自动验收只能证明
/// "构建管理器与运行管理器各自能跑"（单元/集成测试已覆盖），
/// 证明不了**面板上那条真实链路**：点按钮 → 真实 cargo build →
/// 成功后启动游戏 → 日志进面板 → 退出码显示在状态区。
///
/// 它只推一条 `UiAction::构建并运行`，走的是与用户点击**完全相同**的路径，
/// 不做任何"验收专用捷径"。
pub const 构建并运行环境变量: &str = "BEVY_STUDIO_BUILD_AND_RUN";

/// 按环境变量执行一次「构建并运行」。
fn 按环境变量构建并运行(mut 界面: ResMut<UiStateHolder>) {
    if std::env::var(构建并运行环境变量).is_err() {
        return;
    }
    界面.状态.聚焦面板(editor_ui::Tab::运行);
    界面.动作.推(UiAction::构建并运行);
    info!("已按环境变量执行「构建并运行」");
}

/// GUI 验收专用：直接运行已构建好的项目（跳过构建管理器）。
///
/// 它与用户点「运行项目」走同一条运行管理器链路；只用于需要稳定帧号
/// 的 GUI 截图验收，避免构建耗时不固定导致截图落在构建阶段。
pub const 直接运行项目环境变量: &str = "BEVY_STUDIO_RUN_PROJECT";

fn 按环境变量直接运行项目(mut 界面: ResMut<UiStateHolder>) {
    if std::env::var(直接运行项目环境变量).is_err() {
        return;
    }
    界面.状态.聚焦面板(editor_ui::Tab::运行);
    界面.动作.推(UiAction::运行项目);
    info!("已按环境变量直接运行项目");
}

/// 启动后自动开始运行（`1` = 是）。仅供自动验收使用。
///
/// 物理只在**运行态**步进，而"运行"是一个图形界面动作。没有这个入口时，
/// 自动验收无法证明"点了运行之后真的在算物理"，只能证明单元测试通过了。
pub const 自动运行环境变量: &str = "BEVY_STUDIO_AUTORUN";

/// 按环境变量自动开始运行场景。
fn 按环境变量自动运行(
    mut 界面: ResMut<UiStateHolder>, 编辑器: Res<EditorStateHolder>
) {
    let Ok(值) = std::env::var(自动运行环境变量) else {
        return;
    };
    if 值 != "1" || 编辑器.状态.项目.is_none() {
        return;
    }
    界面.动作.推(UiAction::运行场景);
    info!("已按环境变量自动开始运行");
}

/// 指定启动时的设计分辨率（`1280x720` 形式）。
///
/// # 为什么需要这个入口
///
/// 预览分辨率与宽高比是"跨分辨率才暴露"的问题，自动验收必须在**同一个项目**
/// 上反复用不同设计分辨率启动。手改 `project.ron` 会让证据依赖仓库里的临时改动，
/// 用环境变量则只影响本次进程（不落盘）。
pub const 设计分辨率环境变量: &str = "BEVY_STUDIO_DESIGN_RESOLUTION";

/// 按环境变量覆盖本次运行的设计分辨率（不写回项目文件）。
fn 按环境变量设置设计分辨率(mut 编辑器: ResMut<EditorStateHolder>) {
    let Ok(文本) = std::env::var(设计分辨率环境变量) else {
        return;
    };
    let Some((宽, 高)) = 文本.trim().split_once(['x', 'X', '*']) else {
        warn!("设计分辨率格式应为 宽x高：{文本}");
        return;
    };
    let (Ok(宽), Ok(高)) = (宽.trim().parse::<u32>(), 高.trim().parse::<u32>()) else {
        warn!("设计分辨率不是合法数字：{文本}");
        return;
    };
    if 宽 == 0 || 高 == 0 {
        warn!("设计分辨率不能为 0：{文本}");
        return;
    }
    if let Some(项目) = 编辑器.状态.项目.as_mut() {
        项目.清单可变().设计分辨率 = (宽, 高);
        info!("已按环境变量临时覆盖设计分辨率：{宽}×{高}");
    }
}

/// 启动时自动试听一个音频文件（项目相对路径）。
///
/// # 为什么需要这个入口
///
/// 「试听」在资源树的右键菜单里，自动验收没有输入注入时打不开菜单。
/// 有了这个环境变量，验收才能确认整条链路真的跑通：项目被打开、
/// `AssetServer` 加载了音频、`AudioPlayer` 实体被创建、界面进入"正在试听"。
/// 否则只能拍到一张"看不出有没有播"的静态截图。
pub const 试听环境变量: &str = "BEVY_STUDIO_AUDITION";

/// 按环境变量自动试听一个音频文件。
fn 按环境变量试听音频(mut 界面: ResMut<UiStateHolder>) {
    let Ok(文本) = std::env::var(试听环境变量) else {
        return;
    };
    let Ok(路径) = scene_doc::path::项目相对路径::解析(文本.trim()) else {
        warn!("音频路径不合法：{文本}");
        return;
    };
    界面.状态.试听请求 = editor_ui::state::试听请求 {
        要播放: Some(路径),
        要停止: false,
    };
    info!("已按环境变量请求试听");
}

/// 启动时按环境变量打开项目设置对话框。
fn 按环境变量打开项目设置(
    mut 界面: ResMut<UiStateHolder>,
    编辑器: Res<EditorStateHolder>,
) {
    if std::env::var(打开项目设置环境变量).is_err() {
        return;
    }
    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        warn!("环境变量要求打开项目设置，但当前没有打开项目");
        return;
    };
    界面.状态.项目设置草稿 = Some(项目.设置视图());
    界面.状态.显示项目设置 = true;
    info!("已按环境变量打开项目设置对话框");
}

/// 启动时按环境变量打开资源重命名/删除对话框。
///
/// # 为什么需要这个入口
///
/// 资源右键菜单里的"重命名/删除"最终都会落到一个模态对话框上，
/// 而自动化脚本点不了右键菜单（真实指针注入只能点已存在的坐标）。
/// 没有这个入口，就只能靠人眼确认对话框长什么样，
/// 而"截图里没看见"恰恰是过去漏掉检查器显示问题的那类原因。
///
/// 取值：`rename` 打开重命名、`delete` 打开删除；其它值忽略。
/// 目标资源取项目资源列表里**第一个非目录文件**，因此它验证的是
/// "对话框 + 真实引用列表"这条链路，而不是构造一份假数据。
fn 按环境变量打开资源对话框(
    mut 界面: ResMut<UiStateHolder>,
    编辑器: Res<EditorStateHolder>,
) {
    let Ok(模式) = std::env::var(资源对话框环境变量) else {
        return;
    };
    let 模式 = 模式.trim().to_ascii_lowercase();
    if 模式 != "rename" && 模式 != "delete" && 模式 != "refs" {
        warn!("{资源对话框环境变量} 只接受 rename / delete / refs，收到：{模式}");
        return;
    }
    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        warn!("环境变量要求打开资源对话框，但当前没有打开项目");
        return;
    };
    let Some(资源) = 项目
        .资源列表()
        .into_iter()
        .find(|条目| !条目.是目录)
        .map(|条目| 条目.路径)
    else {
        warn!("项目里没有任何资源文件，无法打开资源对话框");
        return;
    };

    if 模式 == "refs" {
        let 索引 = editor_core::asset_index::项目资源索引::构建(
            项目,
            编辑器.状态.会话.路径.as_ref(),
            Some(&编辑器.状态.会话.文档),
        );
        // 选一个**真的有引用**的资源，截图才有内容；没有就用第一个文件。
        let 目标 = 索引
            .按资源
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| 资源.clone());
        let 行 = 索引
            .引用位置(&目标)
            .iter()
            .map(|项| editor_ui::state::查找引用行 {
                场景: 项.场景.clone(),
                实体: 项.位置.实体,
                实体名称: 项.实体名称.clone(),
                组件: 项.位置.组件.clone(),
                属性路径: 项.位置.属性路径.clone(),
            })
            .collect();
        界面.状态.查找引用 = Some(editor_ui::state::查找引用草稿 {
            资源: 目标,
            行,
            问题: Vec::new(),
        });
        info!("已按环境变量打开查找引用对话框");
        return;
    }

    if 模式 == "rename" {
        界面.状态.资源重命名 = Some(editor_ui::state::资源重命名草稿 {
            旧路径: 资源.clone(),
            新路径: 资源.文本().to_string(),
            错误: None,
        });
    } else {
        let 索引 = editor_core::asset_index::项目资源索引::构建(
            项目,
            编辑器.状态.会话.路径.as_ref(),
            Some(&编辑器.状态.会话.文档),
        );
        let 引用 = 索引
            .引用位置(&资源)
            .iter()
            .map(|项| {
                format!(
                    "{} / {} / {}.{}",
                    项.场景.文本(),
                    项.实体名称,
                    项.位置.组件,
                    项.位置.属性路径
                )
            })
            .collect();
        界面.状态.资源删除 = Some(editor_ui::state::资源删除草稿 {
            路径: 资源.clone(),
            引用,
            阻止原因: 索引.阻止删除原因(&资源),
            错误: None,
        });
    }
    info!("已按环境变量打开资源对话框：{模式}");
}

/// 启动时按环境变量生成恢复点并打开恢复对话框。
fn 按环境变量打开恢复对话框(
    mut 界面: ResMut<UiStateHolder>,
    编辑器: Res<EditorStateHolder>,
) {
    if std::env::var(恢复对话框环境变量).is_err() {
        return;
    }
    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        warn!("环境变量要求打开恢复对话框，但当前没有打开项目");
        return;
    };
    let 存储 = editor_core::recovery::恢复点存储::新建(项目.根目录());
    let 时间 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_secs())
        .unwrap_or(0);
    match 存储.写入(编辑器.状态.会话.路径.as_ref(), &编辑器.状态.会话.文档, 时间)
    {
        Ok(_) => {
            crate::recovery_autosave::扫描项目恢复点(&mut 界面.状态, 项目);
            info!("已按环境变量生成恢复点并打开恢复对话框");
        }
        Err(错误) => warn!("生成恢复点失败：{错误}"),
    }
}

/// 启动时打开存档浏览器（自动验收用）。
fn 按环境变量打开存档浏览器(mut 界面: ResMut<UiStateHolder>) {
    if std::env::var(存档浏览器验收环境变量).is_err() {
        return;
    }
    界面.动作.推(editor_ui::state::UiAction::显示存档槽位(true));
    info!("已按环境变量请求打开存档槽位浏览器");
}

/// 启动时选中第一个场景实例并排队一条「刷新场景实例」。
fn 按环境变量刷新实例(
    mut 界面: ResMut<UiStateHolder>,
    mut 编辑器: ResMut<EditorStateHolder>,
) {
    if std::env::var(刷新实例验收环境变量).is_err() {
        return;
    }
    let 候选 = 编辑器
        .状态
        .会话
        .文档
        .深度优先遍历()
        .into_iter()
        .find(|(标识, _)| {
            编辑器
                .状态
                .会话
                .文档
                .取(*标识)
                .and_then(editor_core::instance::实例来源)
                .is_some()
        })
        .map(|(标识, _)| 标识);
    let Some(标识) = 候选 else {
        warn!("场景里没有实例，无法演示刷新");
        return;
    };
    编辑器.状态.选择.单选(标识);
    界面.动作.推(editor_ui::state::UiAction::刷新场景实例);
    info!("已按环境变量选中实例并请求刷新");
}

/// 启动时按环境变量选中前 N 个实体（自动验收用）。
fn 按环境变量多选(mut 编辑器: ResMut<EditorStateHolder>) {
    let Ok(数量) = std::env::var(多选验收环境变量) else {
        return;
    };
    let Ok(数量) = 数量.trim().parse::<usize>() else {
        warn!("{多选验收环境变量} 需要一个数字，收到：{数量}");
        return;
    };
    let 标识: Vec<scene_doc::document::实体标识> = 编辑器
        .状态
        .会话
        .文档
        .深度优先遍历()
        .into_iter()
        .map(|(标识, _)| 标识)
        .take(数量.max(1))
        .collect();
    if 标识.is_empty() {
        warn!("场景是空的，无法演示多选");
        return;
    }
    let 个数 = 标识.len();
    编辑器.状态.选择.设为(标识);
    info!("已按环境变量选中 {个数} 个实体");
}

/// 启动时生成一份**带事件**的动画（自动验收用）。
///
/// 走真实数据结构与真实项目资源：事件引用项目里第一个音频文件（没有就
/// 用逻辑事件）。这样截图里的三角标记、事件行与播放触发都是真数据。
pub const 动画事件验收环境变量: &str = "BEVY_STUDIO_ANIMATION_EVENTS";

fn 按环境变量演示动画事件(
    mut 编辑器: ResMut<EditorStateHolder>,
    mut 界面: ResMut<UiStateHolder>,
) {
    if std::env::var(动画事件验收环境变量).is_err() {
        return;
    }
    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        warn!("环境变量要求演示动画事件，但当前没有打开项目");
        return;
    };
    let 音频 = 项目
        .资源列表()
        .into_iter()
        .find(|条目| !条目.是目录 && 条目.类别 == scene_doc::property::资源类别::音频)
        .map(|条目| 条目.路径.文本().to_string());
    let 实体 = 编辑器
        .状态
        .会话
        .文档
        .深度优先遍历()
        .first()
        .map(|(标识, _)| *标识);
    let Some(实体) = 实体 else {
        warn!("场景是空的，无法演示动画事件");
        return;
    };

    let mut 会话 = editor_core::animation::动画会话::新建("事件演示");
    会话.文档.时长 = 3.0;
    let 轨道 = 会话
        .文档
        .轨道路径(实体, editor_core::animation::动画属性::平移);
    轨道.插入(editor_core::animation::关键帧::新建(
        0.0,
        editor_core::animation::关键帧值::向量3([0.0, 0.0, 0.0]),
    ));
    轨道.插入(editor_core::animation::关键帧::新建(
        3.0,
        editor_core::animation::关键帧值::向量3([2.0, 0.0, 0.0]),
    ));
    let 事件 = match 音频 {
        Some(资源) => editor_core::animation::动画事件::新建(
            1.0,
            editor_core::animation::事件类型::音效,
            资源,
        ),
        None => editor_core::animation::动画事件::新建(
            1.0,
            editor_core::animation::事件类型::音效,
            "assets/audio/未指定.ogg",
        ),
    };
    会话.文档.插入事件(事件);
    会话.文档.插入事件(editor_core::animation::动画事件::新建(
        2.0,
        editor_core::animation::事件类型::逻辑,
        "开门",
    ));
    编辑器.状态.动画 = Some(会话);
    // 选中第一个事件，让截图里能看到名称/类型/参数的编辑区 ——
    // 只摆界面状态，等价于用户点了一下事件三角。
    界面.状态.时间轴选中事件 = Some(0);
    界面.状态.聚焦面板(editor_ui::Tab::时间轴);
    info!("已按环境变量生成带事件的动画，并聚焦时间轴");
}

/// 启动时按环境变量把某个面板切到前台。
///
/// # 为什么需要它
///
/// 面板默认是标签页形式，只有被点到才显示。而**抓图脚本没法点击**，
/// 于是"某个面板在英文模式下长什么样"就无法用截图验证 ——
/// 上一轮的 `Inspector` 正是在这种"没看见就以为没问题"的情况下漏掉的：
/// 我声称检查器已覆盖，但截图里它根本没显示出来。
///
/// 有了这个入口，验证某个面板就变成一条命令：
///
/// ```powershell
/// $env:BEVY_STUDIO_OPEN_PROJECT = "…\示例项目"
/// $env:BEVY_STUDIO_FOCUS_PANEL  = "inspector"
/// .\bevy-studio.exe
/// ```
///
/// 取值用「窗口」菜单里的词条键（`scene_tree` / `inspector` / `viewport` /
/// `asset_browser` / `console` / `history` / `runtime_ecs` / `performance`），
/// 认不出来就什么都不做，并给一条带可选值的警告。
fn 按环境变量聚焦面板(mut 界面: ResMut<UiStateHolder>) {
    let Ok(名字) = std::env::var(启动聚焦面板环境变量) else {
        return;
    };
    let 名字 = 名字.trim();
    if 名字.is_empty() {
        return;
    }
    match editor_ui::Tab::全部()
        .iter()
        .find(|面板| 面板.菜单键() == 名字)
    {
        Some(面板) => {
            界面.状态.聚焦面板(*面板);
            info!("已按环境变量把 `{名字}` 面板切到前台");
        }
        None => warn!(
            "环境变量 {启动聚焦面板环境变量} 的值 `{名字}` 不是已知面板；\
             可用值：scene_tree / inspector / viewport / asset_browser / \
             console / history / runtime_ecs / performance / script / build / run / timeline"
        ),
    }
}

/// 启动时恢复上次的工作区集。
///
/// # 恢复顺序（每一步都可能失败，且失败互不牵连）
///
/// 1. 语言：不依赖布局，越早生效越好，否则首帧会先按中文画一次再闪成英文；
/// 2. 新格式工作区集（配置格式版本 2）；
/// 3. 旧格式单布局（格式版本 1）→ 迁移成"迁移的布局"工作区；
/// 4. 校验与修复：单个工作区布局坏掉只回退它自己；
/// 5. 环境变量 `BEVY_STUDIO_WORKSPACE` 覆盖当前工作区（自动验收用）。
fn 恢复窗口布局(
    mut 界面: ResMut<UiStateHolder>,
    配置: Res<编辑器配置资源>,
    mut 布局待保存: ResMut<布局待保存>,
    编辑器: Res<EditorStateHolder>,
) {
    // 语言先恢复：它不依赖布局，而且越早生效越好 ——
    // 晚一帧恢复的话，首帧会先按中文画一次，然后闪一下变成英文。
    if let Some(代号) = 配置.0.偏好.get("语言") {
        界面.状态.语言 = editor_ui::i18n::语言::从代号(代号);
        info!("已恢复界面语言：{}", 界面.状态.语言.自称());
    }

    // 布局恢复失败**不能**影响语言：两者互相独立，
    // 用 `return` 会把语言的恢复一起取消掉。
    // 布局存档：格式版本 3（模式 + 每模式工作区集）优先；
    // 读取失败会依次尝试版本 2（单一工作区集）与版本 1（单份 DockState），
    // 因此旧配置不需要删除。
    let 存档文本 = 配置.0.工作区.as_deref().or(配置.0.布局.as_deref());
    if let Some(文本) = 存档文本 {
        match 界面.状态.从布局存档恢复(文本) {
            Ok(备注) => {
                info!(
                    "已恢复布局：当前模式 `{}`，共 {} 个模式的布局",
                    界面.状态.模式.id(),
                    界面.状态.模式布局().len() + 1
                );
                for 说明 in &备注 {
                    info!("布局迁移：{说明}");
                }
            }
            Err(错误) => warn!("布局存档无法恢复（{错误}），改用出厂布局"),
        }
    }

    // 主题：与工作区完全独立的维度。主题存档坏了只回退主题，
    // 工作区与语言都不受影响。
    if let Some(文本) = 配置.0.主题.as_deref() {
        match editor_ui::主题库::从文本恢复(文本) {
            Ok(库) => {
                info!("已恢复主题库，当前主题为 `{}`", 库.当前id());
                界面.状态.主题库 = 库;
            }
            Err(错误) => warn!(
                "主题存档无法恢复（{}），改用内置主题",
                错误.文案(界面.状态.语言)
            ),
        }
    }
    // 环境变量覆盖：自动验收要能指定"这次跑哪个主题"。
    if let Ok(id) = std::env::var(主题环境变量) {
        let id = id.trim();
        if !id.is_empty() {
            match 界面.状态.主题库.切换(id) {
                Ok(()) => {
                    布局待保存.脏 = true;
                    info!("已按环境变量切换到主题 `{id}`");
                }
                Err(错误) => warn!(
                    "环境变量 {主题环境变量} 指定的主题 `{id}` 无法切换：{}",
                    错误.文案(界面.状态.语言)
                ),
            }
        }
    }

    // 校验与修复：损坏的单个工作区不应让整个编辑器失去布局。
    // 主面板约束按**当前模式**给出（AI Agent 模式没有视口，这是设计而非损坏）。
    let 主面板 = 界面.状态.模式.主面板候选();
    let 修复 = 界面.状态.工作区.修复(&主面板);
    for 说明 in &修复 {
        warn!("工作区已修复：{说明}");
    }
    // 备注里可能还有"补回缺失的内置工作区"这类迁移说明，逐条记录。
    for 备注 in 界面.状态.工作区.备注() {
        if !修复.contains(备注) {
            info!("工作区：{备注}");
        }
    }

    // 环境变量覆盖：自动验收要能指定"这次跑哪个编辑模式"。
    if let Ok(id) = std::env::var(模式环境变量) {
        let id = id.trim();
        if !id.is_empty() {
            match editor_ui::开发模式::兼容旧id(id) {
                editor_ui::dev_modes::旧模式迁移::当前(模式) => {
                    match 界面.状态.切换模式(模式) {
                        Ok(()) => {
                            布局待保存.脏 = true;
                            info!("已按环境变量切换到模式 `{id}`");
                        }
                        Err(错误) => warn!("环境变量 {模式环境变量} 切换模式失败：{错误}"),
                    }
                }
                editor_ui::dev_modes::旧模式迁移::已移除 { id: 旧id, 代替 } => {
                    warn!("环境变量 {模式环境变量} 用的是已移除的模式 `{旧id}`：{代替}")
                }
                editor_ui::dev_modes::旧模式迁移::未知 => warn!(
                    "环境变量 {模式环境变量} 的值 `{id}` 不是已知模式；可用值：2d / 3d / script"
                ),
            }
        }
    }

    // 环境变量覆盖：自动验收要能指定"这次跑哪个工作区"。
    if let Ok(id) = std::env::var(工作区环境变量) {
        let id = id.trim();
        if !id.is_empty() {
            match 界面.状态.工作区.切换(id) {
                Ok(()) => {
                    // 环境变量指定的工作区/主题属于"用户选择"，
                    // 标记为待保存：否则下次启动会回退到旧配置，
                    // 自动验收也就无法验证"重启后恢复"。
                    布局待保存.脏 = true;
                    info!("已按环境变量切换到工作区 `{id}`");
                }
                Err(错误) => warn!(
                    "环境变量 {工作区环境变量} 指定的工作区 `{id}` 无法切换：{}",
                    错误.文案(界面.状态.语言)
                ),
            }
        }
    }
    // 启动时就打开了项目（`BEVY_STUDIO_OPEN_PROJECT`）：
    // 2D 视口自动框住整张地图，首帧就能看到全貌；3D 保持默认近景，
    // 因为把 48×48 竞技场拉成全景会让角色只剩十几像素。
    if 编辑器.状态.项目.is_some() && !界面.状态.模式.是三维() {
        界面.状态.视口命令.适应窗口 = true;
    }
}

/// 指定启动时使用哪个主题（内置 id 或用户主题 id）。
pub const 主题环境变量: &str = "BEVY_STUDIO_THEME";
/// 指定启动时切到哪个工作区（内置 id 或用户工作区 id）。
pub const 工作区环境变量: &str = "BEVY_STUDIO_WORKSPACE";

/// 指定启动时进入哪个编辑模式（2d / 3d / script）。
///
/// 早期版本还有 `run` / `assets` 两个模式，它们已并入工具栏与工作区；
/// 用旧值启动不会静默失败 —— [`editor_ui::开发模式::兼容旧id`] 会给出说明。
pub const 模式环境变量: &str = "BEVY_STUDIO_MODE";

/// 指定启动界面语言（`zh` / `en`）。
pub const 语言环境变量: &str = "BEVY_STUDIO_LANGUAGE";

/// 指定启动时使用哪个变换工具（`select` / `move` / `rotate` / `scale`）。
///
/// # 为什么工具也需要一个环境变量入口
///
/// 自动验收要验证的是**拖拽操纵器**这件事本身。如果每个场景都得先
/// "点一下工具栏按钮"才能进入移动工具，那么工具切换失败与操纵器拖拽
/// 失败就会混在同一条证据里。有了这个入口，工具状态是确定的，
/// 拖拽验证只考察拖拽本身；工具栏切换另有一条专门的场景验证。
pub const 变换工具环境变量: &str = "BEVY_STUDIO_TOOL";

/// 按环境变量设置启动时的变换工具。
fn 按环境变量设置变换工具(mut 界面: ResMut<UiStateHolder>) {
    let Ok(代号) = std::env::var(变换工具环境变量) else {
        return;
    };
    let Some(工具) = editor_ui::gizmo::变换工具::从代号(代号.trim()) else {
        warn!("未知的变换工具代号：{代号}（可用 select/move/rotate/scale）");
        return;
    };
    界面.状态.变换工具 = 工具;
    info!("已按环境变量设置变换工具：{}", 工具.代号());
}

/// 按环境变量覆盖界面语言。
///
/// 语言本来存在配置的 `偏好` 里；自动验收需要一个不依赖配置文件历史的
/// 确定性入口，否则"英文截图"可能因为上一轮配置残留而拍成中文。
fn 按环境变量设置语言(mut 界面: ResMut<UiStateHolder>) {
    let Ok(代号) = std::env::var(语言环境变量) else {
        return;
    };
    let 代号 = 代号.trim();
    if 代号.is_empty() {
        return;
    }
    界面.状态.语言 = editor_ui::i18n::语言::从代号(代号);
    info!("已按环境变量设置界面语言：{}", 界面.状态.语言.自称());
}

/// 让验收脚本可以直接打开主题编辑器（`1` 表示打开）。
pub const 主题编辑器环境变量: &str = "BEVY_STUDIO_OPEN_THEME_EDITOR";

/// 按环境变量打开主题编辑器。
///
/// 主题编辑器是浮窗而不是面板，"打开它"在自动验收里没有鼠标可点。
/// 有了这个入口，截图就能覆盖"主题编辑器真的画出来了、颜色可编辑"这件事。
fn 按环境变量打开主题编辑器(mut 界面: ResMut<UiStateHolder>) {
    let Ok(值) = std::env::var(主题编辑器环境变量) else {
        return;
    };
    if matches!(值.trim(), "0" | "false" | "no" | "") {
        return;
    }
    界面.状态.显示主题编辑器 = true;
    界面.状态.主题草稿 = Some(界面.状态.主题库.当前().clone());
    info!("已按环境变量打开主题编辑器");
}

/// 启动时在控制台写一行说明，便于确认版本与环境。
fn 打印启动信息() {
    info!(
        "Bevy Studio {} 启动：Bevy {} / egui 界面已就绪",
        env!("CARGO_PKG_VERSION"),
        "0.20"
    );
}

/// 一次界面绘制所需的"只读环境"。
///
/// 把这些散在参数列表里的只读输入打成一个结构体，有两个好处：
///
/// 1. 调用点从 8 个位置参数变成"上下文 + 环境 + 两个可变输出"，
///    一眼能看出哪些是输入、哪些是输出；
/// 2. 将来给绘制加一项只读输入时，只需要改这里的字段与构造点，
///    不必四处调整参数列表。
pub struct 绘制环境<'平台> {
    /// 编辑器权威状态。
    pub 编辑器: &'平台 编辑器状态,
    /// 图标纹理缓存。
    pub 图标: &'平台 mut editor_ui::图标库,

    /// 资源缩略图纹理表。
    pub 缩略图: &'平台 BTreeMap<String, egui::TextureHandle>,
    /// 瓦片集图集纹理表。
    pub 瓦片图集: &'平台 BTreeMap<String, egui::TextureHandle>,
    /// 视口的 Bevy 渲染结果。`None` 表示预览尚未就绪。
    pub 视口纹理: Option<视口渲染结果>,
    /// 运行时宿主。
    pub 运行时: &'平台 运行时宿主,
    /// 运行时诊断信息。
    pub 诊断: &'平台 运行时诊断,
    /// 构建配置与最近一次构建报告。
    pub 构建: &'平台 editor_core::build::构建快照,
    /// 运行状态、本次运行日志与最近运行记录。
    pub 运行: &'平台 editor_core::run_entry::运行快照,
    /// 远程调试会话只读快照。
    pub 调试: &'平台 editor_core::runtime_debug::运行时调试快照,
    /// 运行时快照与属性修改历史。
    pub 调试历史: &'平台 editor_core::runtime_debug::运行时调试历史,
    /// 统一诊断（当前结果视图）。
    pub 统一诊断: &'平台 crate::diagnostics::诊断快照,
    /// 资源导入状态表（项目相对路径文本 → 状态）。
    pub 导入状态: &'平台 std::collections::BTreeMap<String, editor_core::project::导入状态>,
    /// 编辑器配置目录（帮助窗口显示路径、提供"打开配置目录"）。
    pub 配置目录: &'平台 std::path::Path,
}

/// 便捷函数：构造一次绘制所需的上下文并调用界面绘制。
///
/// 抽出来是为了让"构造上下文"这件事只有一处实现：
/// 字段一旦增减，只有这里需要改。
pub fn 绘制一帧(
    上下文: &egui::Context,
    环境: 绘制环境<'_>,
    界面: &mut 界面状态,
    动作: &mut 动作队列,
) {
    // 本帧生效的主题：主题编辑器打开时用**草稿**（实时预览），
    // 否则用主题库里的当前主题。
    let 主题 = 界面
        .主题草稿
        .clone()
        .unwrap_or_else(|| 界面.主题库.当前().clone());
    let 配色 = 主题.配色;
    let 外观 = 主题.外观.收敛();
    let mut 绘制 = 绘制上下文 {
        编辑器: 环境.编辑器,
        // 语言是界面状态的一部分（用户可在「窗口 → 界面语言」里切换），
        // 这里读出来放进上下文，各面板才能用 `上下文.语言.取(..)`。
        语言: 界面.语言,
        界面,
        动作,
        主题,
        配色,
        外观,
        图标: 环境.图标,
        缩略图: 环境.缩略图,
        瓦片图集: 环境.瓦片图集,
        视口纹理: 环境.视口纹理,
        正在运行: 环境.运行时.状态.在运行(),
        已暂停: 环境.运行时.状态 == 运行状态::已暂停,
        运行时信息: 环境.诊断,
        构建: 环境.构建,
        运行: 环境.运行,
        调试: 环境.调试,
        调试历史: 环境.调试历史,
        统一诊断: 环境.统一诊断,
        导入状态: 环境.导入状态,
        配置目录: 环境.配置目录,
    };
    editor_ui::绘制(上下文, &mut 绘制);
}

/// 把 [`UiAction`] 的显示名取出来，便于日志与测试断言。
pub fn 动作摘要(动作: &UiAction) -> String {
    match 动作 {
        UiAction::新建项目 { 名称 } => format!("新建项目({名称})"),
        UiAction::请求打开项目 => "请求打开项目".to_string(),
        UiAction::打开项目(路径) => format!("打开项目({})", 路径.display()),
        UiAction::关闭项目 => "关闭项目".to_string(),
        UiAction::新建场景 => "新建场景".to_string(),
        UiAction::打开场景(路径) => format!("打开场景({})", 路径.文本()),
        UiAction::请求场景另存为 => "请求场景另存为".to_string(),
        UiAction::场景另存为(路径) => format!("场景另存为({})", 路径.文本()),
        UiAction::保存场景 => "保存场景".to_string(),
        UiAction::在资源管理器中显示(路径) => {
            format!("在资源管理器中显示({})", 路径.文本())
        }
        // 动作摘要只用于日志与测试断言，固定用中文。
        UiAction::执行命令(命令) => {
            format!("执行命令({})", 命令.显示名(editor_ui::i18n::语言::中文))
        }
        UiAction::执行命令连续(命令) => {
            format!("连续编辑({})", 命令.显示名(editor_ui::i18n::语言::中文))
        }
        UiAction::撤销 => "撤销".to_string(),
        UiAction::重做 => "重做".to_string(),
        UiAction::修改属性(编辑) => {
            format!("修改属性({}.{})", 编辑.组件, 编辑.属性)
        }
        UiAction::添加组件 { 类型名, .. } => format!("添加组件({类型名})"),
        UiAction::删除组件 { 类型名, .. } => format!("删除组件({类型名})"),
        UiAction::删除实体(_) => "删除实体".to_string(),
        UiAction::复制实体(_) => "复制实体".to_string(),
        UiAction::重命名实体 { 新名称, .. } => format!("重命名实体({新名称})"),
        UiAction::选择(_) => "选择".to_string(),
        UiAction::开始拖动实体 { .. } => "开始拖动实体".to_string(),
        UiAction::拖动实体到 { .. } => "拖动实体".to_string(),
        UiAction::结束拖动 => "结束拖动".to_string(),
        UiAction::预览变换 { 项 } => format!("预览变换({} 个实体)", 项.len()),
        UiAction::提交变换 { 最终, 起始 } => {
            // 只报"几个实体变了"，不打印具体数值：这条只用于日志与测试断言。
            let 变化数 = 最终
                .iter()
                .filter(|(标识, 变换)| {
                    起始
                        .iter()
                        .find(|(起标识, _)| 起标识 == 标识)
                        .map(|(_, 起变换)| 起变换 != 变换)
                        .unwrap_or(true)
                })
                .count();
            format!("提交变换({变化数} 个实体发生变化)")
        }
        UiAction::取消变换 { 起始 } => format!("取消变换({} 个实体)", 起始.len()),
        UiAction::从资源创建实体 { 资源, .. } => {
            format!("从资源创建实体({})", 资源.文本())
        }
        UiAction::请求重命名资源(路径) => {
            format!("请求重命名资源({})", 路径.文本())
        }
        UiAction::请求删除资源(路径) => {
            format!("请求删除资源({})", 路径.文本())
        }
        UiAction::确认重命名资源 {
            旧路径, 新路径
        } => {
            format!("确认重命名资源({} → {})", 旧路径.文本(), 新路径.文本())
        }
        UiAction::关闭重命名资源 => "关闭重命名资源".to_string(),
        UiAction::确认删除资源(路径) => {
            format!("确认删除资源({})", 路径.文本())
        }
        UiAction::关闭删除资源 => "关闭删除资源".to_string(),
        UiAction::检查资源使用 => "检查资源使用".to_string(),
        UiAction::请求查找引用(资源) => {
            format!("请求查找引用({})", 资源.文本())
        }
        UiAction::打开引用位置 { 场景, .. } => {
            format!("打开引用位置({})", 场景.文本())
        }
        UiAction::关闭查找引用 => "关闭查找引用".to_string(),
        UiAction::显示存档槽位(显示) => format!("显示存档槽位({显示})"),
        UiAction::刷新存档槽位 => "刷新存档槽位".to_string(),
        UiAction::刷新场景实例 => "刷新场景实例".to_string(),
        UiAction::同步预制件实例 { 实例根 } => format!("同步预制件实例({实例根})"),
        UiAction::同步预制件模板 { 模板 } => format!("同步预制件模板({})", 模板.0),
        UiAction::重新纳入预制件节点 { 记录键, 地址 } => {
            format!("重新纳入预制件节点(记录 {记录键}, {} 段)", 地址.0.len())
        }
        UiAction::刷新预制件目录 => "刷新预制件目录".to_string(),
        UiAction::对齐选中 { 边 } => format!("对齐选中({})", 边.词条键()),
        UiAction::分布选中 { 轴 } => format!("分布选中({})", 轴.词条键()),
        UiAction::添加动画事件 { 类型, .. } => {
            format!("添加动画事件({})", 类型.代号())
        }
        UiAction::删除动画事件 { 时间 } => format!("删除动画事件({时间})"),
        UiAction::更新动画事件 { 下标, .. } => format!("更新动画事件(#{下标})"),
        UiAction::选择动画事件(下标) => match 下标 {
            Some(下标) => format!("选择动画事件(#{下标})"),
            None => "取消选择动画事件".to_string(),
        },
        UiAction::恢复自动保存(元数据) => {
            format!("恢复自动保存({})", 元数据.场景名)
        }
        UiAction::丢弃自动保存(元数据) => {
            format!("丢弃自动保存({})", 元数据.场景名)
        }
        UiAction::关闭恢复提示 => "关闭恢复提示".to_string(),
        UiAction::运行场景 => "运行场景".to_string(),
        UiAction::暂停运行 => "暂停运行".to_string(),
        UiAction::停止运行 => "停止运行".to_string(),
        UiAction::单步一帧 => "单步一帧".to_string(),
        // 用中文标题而不是当前语言：这是给日志与测试断言用的**诊断名**，
        // 应当与界面语言无关 —— 否则测试会因为语言设置而变红，
        // 而日志里换成英文也不便与中文文档对照。
        UiAction::聚焦面板(面板) => format!("聚焦面板({})", 面板.中文标题()),
        UiAction::恢复默认布局 => "恢复默认布局".to_string(),
        UiAction::切换工作区(id) => format!("切换工作区({id})"),
        UiAction::打开工作区对话框 { 复制当前 } => {
            format!("打开工作区对话框(复制当前={复制当前})")
        }
        UiAction::新建工作区 {
            名称, 复制当前
        } => {
            format!("新建工作区({名称}, 复制当前={复制当前})")
        }
        UiAction::重命名工作区 { id, 新名称 } => format!("重命名工作区({id} → {新名称})"),
        UiAction::删除工作区(id) => format!("删除工作区({id})"),
        UiAction::重置当前工作区 => "重置当前工作区".to_string(),
        UiAction::重置全部内置工作区 => "重置全部内置工作区".to_string(),
        UiAction::切换模式(模式) => format!("切换模式({})", 模式.id()),
        UiAction::运行项目 => "运行项目".to_string(),
        UiAction::停止项目 => "停止项目".to_string(),
        UiAction::重新运行项目 => "重新运行项目".to_string(),
        UiAction::打开示例项目(示例) => format!("打开示例项目({})", 示例.目录名()),
        UiAction::打开脚本(路径) => format!("打开脚本({})", 路径.文本()),
        UiAction::保存脚本 => "保存脚本".to_string(),
        UiAction::脚本另存为 => "脚本另存为".to_string(),
        UiAction::视口适应窗口 => "视口适应窗口".to_string(),
        UiAction::视口重置视图 => "视口重置视图".to_string(),
        UiAction::三维重置轨道 => "三维重置轨道".to_string(),
        UiAction::聚焦选中实体 => "聚焦选中实体".to_string(),
        UiAction::浮动面板(面板) => format!("浮动面板({})", 面板.中文标题()),
        UiAction::停靠全部浮动面板 => "停靠全部浮动面板".to_string(),
        UiAction::载入瓦片地图 { 地图文件, .. } => {
            format!("载入瓦片地图({地图文件})")
        }
        UiAction::保存瓦片地图 { 地图文件, .. } => {
            format!("保存瓦片地图({地图文件})")
        }
        UiAction::保存瓦片集 {
            瓦片集文件, ..
        } => {
            format!("保存瓦片集({瓦片集文件})")
        }
        UiAction::设置主题(id) => format!("设置主题({id})"),
        UiAction::显示主题编辑器(显示) => format!("显示主题编辑器({显示})"),
        UiAction::关闭主题编辑器 => "关闭主题编辑器".to_string(),
        UiAction::保存主题草稿 => "保存主题草稿".to_string(),
        UiAction::另存为主题 { 名称 } => format!("另存为主题({名称})"),
        UiAction::删除主题(id) => format!("删除主题({id})"),
        UiAction::重置内置主题 => "重置内置主题".to_string(),
        UiAction::重置全部内置主题 => "重置全部内置主题".to_string(),
        UiAction::导出主题 => "导出主题".to_string(),
        UiAction::导入主题 => "导入主题".to_string(),
        UiAction::设置语言(语言) => format!("设置语言({})", 语言.自称()),
        UiAction::清空控制台 => "清空控制台".to_string(),
        UiAction::显示关于 => "显示关于".to_string(),

        // ---- 构建 ----
        UiAction::开始构建 => "开始构建".to_string(),
        UiAction::停止构建 => "停止构建".to_string(),
        UiAction::设置构建配置(配置) => format!("设置构建配置({})", 配置.可读命令行()),
        UiAction::打开构建诊断 { 文件, 行 } => format!(
            "打开构建诊断({}:{行})",
            文件
                .as_ref()
                .map(|文件| 文件.display().to_string())
                .unwrap_or_else(|| "?".to_string())
        ),

        // ---- 运行与诊断（开发工作台） ----
        UiAction::选择运行入口(入口) => format!("选择运行入口({})", 入口.标识()),
        UiAction::重新识别工程 => "重新识别工程".to_string(),
        UiAction::构建并运行 => "构建并运行".to_string(),
        UiAction::清空运行日志 => "清空运行日志".to_string(),
        UiAction::打开诊断位置 { 编号 } => format!("打开诊断位置(#{编号})"),
        UiAction::把诊断发送给智能体 { 编号 } => match 编号 {
            Some(编号) => format!("把诊断发送给智能体(#{编号})"),
            None => "把当前诊断发送给智能体".to_string(),
        },
        UiAction::把运行日志发送给智能体 => "把运行日志发送给智能体".to_string(),
        UiAction::清空诊断 => "清空诊断".to_string(),

        // ---- 远程 ECS 检查器 ----
        UiAction::刷新运行时 => "刷新运行时".to_string(),
        UiAction::查询运行时实体 { 页, 每页, .. } => {
            format!("查询运行时实体(第 {页} 页，每页 {每页})")
        }
        UiAction::选中运行时实体 { 实体 } => format!("选中运行时实体({实体})"),
        UiAction::选中运行时资源 { 名称 } => format!("选中运行时资源({名称})"),
        UiAction::断开运行时调试 => "断开运行时调试".to_string(),
        UiAction::重连运行时调试 => "重连运行时调试".to_string(),
        UiAction::把运行时上下文发送给智能体 => {
            "把运行时上下文发送给智能体".to_string()
        }
        UiAction::采集运行时快照 => "采集运行时快照".to_string(),
        UiAction::比较最近运行时快照 => "比较最近运行时快照".to_string(),
        UiAction::提交运行时属性 {
            组件, 字段, 实体,
        ..
        } => format!("提交运行时属性({实体} {组件}.{字段})"),
        UiAction::关联运行时实体 { 场景实体 } => {
            format!("关联运行时实体({场景实体})")
        }
        UiAction::清除运行时关联 => "清除运行时关联".to_string(),

        // ---- 动画与时间轴 ----
        UiAction::新建动画 => "新建动画".to_string(),
        UiAction::请求打开动画 => "请求打开动画".to_string(),
        UiAction::打开动画(路径) => format!("打开动画({})", 路径.文本()),
        UiAction::试听资源(路径) => format!("试听资源({})", 路径.文本()),
        UiAction::停止试听 => "停止试听".to_string(),
        UiAction::试听实体音频(标识) => format!("试听实体音频({标识})"),
        UiAction::请求打开场景 => "请求打开场景".to_string(),
        UiAction::请求实例化场景 => "请求实例化场景".to_string(),
        UiAction::实例化场景(路径) => format!("实例化场景({})", 路径.文本()),
        UiAction::复制选中子树 => "复制选中子树".to_string(),
        UiAction::粘贴剪贴板 => "粘贴剪贴板".to_string(),
        UiAction::请求导出子场景 => "请求导出子场景".to_string(),
        UiAction::导出子场景 { 路径, .. } => format!("导出子场景({})", 路径.文本()),
        UiAction::设置关键帧切线 { 时间, .. } => format!("设置关键帧切线({时间:.2}s)"),
        UiAction::撤销动画 => "撤销动画".to_string(),
        // AI Agent 的动作都在 `执行单个动作` 里逐个处理；这里的摘要只用于调试
        // 输出与动作可达性守卫，因此如实给出动作名即可。
        UiAction::智能体新建会话 => "AI 新建会话".to_string(),
        UiAction::智能体选择会话(下标) => format!("AI 选择会话({下标})"),
        UiAction::智能体开始重命名 => "AI 开始重命名".to_string(),
        UiAction::智能体确认重命名(名称) => format!("AI 重命名({名称})"),
        UiAction::智能体取消重命名 => "AI 取消重命名".to_string(),
        UiAction::智能体删除会话 => "AI 删除会话".to_string(),
        UiAction::智能体发送 => "AI 发送".to_string(),
        UiAction::智能体取消 => "AI 取消任务".to_string(),
        UiAction::智能体探测Dsh => "AI 探测 DSH".to_string(),
        UiAction::智能体附加实体上下文 => "AI 附加实体".to_string(),
        UiAction::智能体附加场景上下文 => "AI 附加场景".to_string(),
        UiAction::智能体附加项目上下文 => "AI 附加项目".to_string(),
        UiAction::智能体移除上下文(下标) => format!("AI 移除上下文({下标})"),
        UiAction::智能体刷新变更 => "AI 刷新变更".to_string(),
        UiAction::智能体重试 => "AI 重新执行".to_string(),
        UiAction::智能体继续 => "AI 继续任务".to_string(),
        UiAction::智能体附加原生工程上下文 => "AI 附加原生工程".to_string(),
        UiAction::智能体自动修复构建 => "AI 构建并修复".to_string(),
        UiAction::智能体自动修复测试 => "AI 测试并修复".to_string(),
        UiAction::智能体批准变更(路径) => format!("AI 批准变更({路径})"),
        UiAction::智能体拒绝变更(路径) => format!("AI 拒绝变更({路径})"),
        UiAction::智能体批准审批(标识) => format!("AI 批准审批({标识})"),
        UiAction::智能体拒绝审批(标识) => format!("AI 拒绝审批({标识})"),
        UiAction::重做动画 => "重做动画".to_string(),
        UiAction::设置轨道插值 { 插值, .. } => format!("设置轨道插值({})", 插值.变体名()),
        UiAction::保存动画 => "保存动画".to_string(),
        UiAction::请求动画另存为 => "请求动画另存为".to_string(),
        UiAction::关闭动画 => "关闭动画".to_string(),
        UiAction::播放动画 => "播放动画".to_string(),
        UiAction::暂停动画 => "暂停动画".to_string(),
        UiAction::停止动画 => "停止动画".to_string(),
        UiAction::动画定位(时间) => format!("动画定位({时间:.3})"),
        UiAction::设置动画时长(时长) => format!("设置动画时长({时长:.3})"),
        UiAction::设置动画循环(循环) => format!("设置动画循环({循环})"),
        UiAction::设置动画速度(速度) => format!("设置动画速度({速度:.2})"),
        UiAction::录制动画关键帧 { 属性 } => {
            format!("录制动画关键帧({} 个属性)", 属性.len())
        }
        UiAction::删除动画轨道 { 属性, .. } => format!("删除动画轨道({})", 属性.代号()),
        UiAction::删除动画关键帧 { 时间, .. } => format!("删除动画关键帧({时间:.3})"),

        // ---- 项目管理 ----
        UiAction::显示项目设置(显示) => format!("显示项目设置({显示})"),
        UiAction::保存项目设置(设置) => format!("保存项目设置({})", 设置.名称),
        UiAction::打开最近项目(目录) => {
            format!("打开最近项目({})", 目录.display())
        }
        UiAction::移除最近项目(目录) => {
            format!("移除最近项目({})", 目录.display())
        }
        UiAction::清除最近项目 => "清除最近项目".to_string(),

        // ---- 资源导入 ----
        UiAction::重新导入资源 => "重新导入资源".to_string(),
        UiAction::检查资源依赖 => "检查资源依赖".to_string(),

        // ---- 帮助与设置 ----
        UiAction::显示帮助(显示) => format!("显示帮助({显示})"),
        UiAction::打开配置目录 => "打开配置目录".to_string(),

        // ---- 打包 ----
        UiAction::设置打包配置(配置) => {
            format!("设置打包配置({})", 配置.输出目录路径())
        }
        UiAction::打包项目 => "打包项目".to_string(),
        UiAction::打开打包目录 => "打开打包目录".to_string(),
    }
}

/// 输入诊断：仅在 `BEVY_STUDIO_INPUT_DIAG=1` 时记录指针按下/抬起。
///
/// GUI 验收需要区分"事件没送到"和"送到了但 UI 没响应"。
/// 这行日志就是那条分界线：没有它，任何布局不变都只能猜。
pub fn 输入诊断(
    窗口: Query<&Window, With<bevy::window::PrimaryWindow>>,
    按键: Res<ButtonInput<MouseButton>>,
) {
    if std::env::var("BEVY_STUDIO_INPUT_DIAG").is_err() {
        return;
    }
    let Ok(窗口) = 窗口.single() else {
        return;
    };
    // 三个按钮都记：验收里既有左键（选择/按钮）也有右键（上下文菜单）。
    // 只记左键会让"右键菜单没弹出来"与"右键根本没送达"无法区分。
    for (按钮, 名称) in [
        (MouseButton::Left, "左键"),
        (MouseButton::Right, "右键"),
        (MouseButton::Middle, "中键"),
    ] {
        let 按下 = 按键.just_pressed(按钮);
        let 抬起 = 按键.just_released(按钮);
        if 按下 || 抬起 {
            info!(
                "[输入诊断] {名称}{} 位置 {:?}",
                if 按下 { "按下" } else { "抬起" },
                窗口.cursor_position()
            );
        }
    }
}

/// 把运行管理器状态同步到界面（工具栏摘要/按钮可用性）。
fn 同步项目运行状态(
    管理器: Res<crate::run::项目运行管理器>,
    mut 界面: ResMut<UiStateHolder>,
) {
    界面.状态.项目运行摘要 = 管理器.摘要();
    界面.状态.项目运行中 = 管理器.状态.在运行();
}
