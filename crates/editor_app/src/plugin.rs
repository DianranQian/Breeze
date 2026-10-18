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
            Some(文件) => 文件对话框结果::选定文件(文件),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::打开场景文件 => match rfd::FileDialog::new()
            .set_title("打开场景")
            .add_filter("微风场景", &["ron"])
            .pick_file()
        {
            Some(文件) => 文件对话框结果::选定文件(文件),
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
            .init_resource::<EditorStateHolder>()
            .insert_resource(UiStateHolder::新建())
            .init_resource::<缩略图缓存>()
            .init_resource::<图标资源>()
            .init_resource::<诊断资源>()
            .init_resource::<运行时宿主>()
            .init_resource::<crate::run::项目运行管理器>()
            // 构建任务：真实 cargo 子进程 + 界面读的快照。
            .init_resource::<crate::build::构建宿主>()
            .init_resource::<构建快照资源>()
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
                    preview::初始化预览资源,
                    按环境变量打开项目,
                    // 恢复布局必须在"聚焦面板"**之前**：恢复会用磁盘上的
                    // 布局整体替换当前布局，若顺序反了，环境变量让某个面板
                    // 前台化、紧接着又被旧布局覆盖 —— 表现是"面板明明在，
                    // 截图里却没有"，而日志里两件事都成功。
                    恢复窗口布局,
                    // 放在打开项目与恢复布局之后：那时才有选中实体，
                    // Inspector 才有内容，且聚焦不会被随后的布局恢复抹掉。
                    按环境变量聚焦面板,
                    按环境变量设置语言,
                    按环境变量设置变换工具,
                    按环境变量打开主题编辑器,
                    打印启动信息,
                )
                    .chain(),
            )
            // ---- Update：只改数据，不碰 egui ----
            //
            // 顺序即数据流，见模块头部说明。
            // ---- Last：退出前把配置落盘 ----
            .add_systems(
                Last,
                (
                    actions::退出时保存配置,
                    crate::run::退出时停止项目运行,
                    crate::build::退出时停止构建,
                ),
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
                    // 动画时间在动作之后推进：本帧点了"播放"就立刻开始走时间。
                    actions::推进动画播放,
                    // 紧跟动作执行：本帧产生的日志（含错误）立刻出现在进程日志里。
                    actions::转发控制台日志到标准错误,
                    actions::读取键盘导航,
                    // 采集收尾：等截图落盘后退出。放在这一链的末尾，
                    // 保证"本帧的日志先转发出去"再决定退出。
                    crate::capture::采集收尾,
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
                        preview::同步预览相机,
                        preview::同步渲染尺寸,
                        preview::模型诊断,
                        crate::run::轮询项目运行,
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

/// 指定启动时要切到前台的面板（用「窗口」菜单里的词条键，如 `inspector`）。
pub const 启动聚焦面板环境变量: &str = "BEVY_STUDIO_FOCUS_PANEL";

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
             console / history / runtime_ecs / performance / script / build / timeline"
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
    let 修复 = 界面.状态.工作区.修复();
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
    /// 视口的 Bevy 渲染结果。`None` 表示预览尚未就绪。
    pub 视口纹理: Option<视口渲染结果>,
    /// 运行时宿主。
    pub 运行时: &'平台 运行时宿主,
    /// 运行时诊断信息。
    pub 诊断: &'平台 运行时诊断,
    /// 构建配置与最近一次构建报告。
    pub 构建: &'平台 editor_core::build::构建快照,
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
        视口纹理: 环境.视口纹理,
        正在运行: 环境.运行时.状态.在运行(),
        已暂停: 环境.运行时.状态 == 运行状态::已暂停,
        运行时信息: 环境.诊断,
        构建: 环境.构建,
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

        // ---- 动画与时间轴 ----
        UiAction::新建动画 => "新建动画".to_string(),
        UiAction::请求打开动画 => "请求打开动画".to_string(),
        UiAction::打开动画(路径) => format!("打开动画({})", 路径.文本()),
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
