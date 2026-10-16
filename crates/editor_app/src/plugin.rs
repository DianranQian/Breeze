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
    state::UiAction, 动作队列, 界面状态, 绘制上下文, 编辑器配色, 视口渲染结果, 运行时诊断,
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
        状态.信息("Bevy Studio 已启动。请新建或打开一个项目。");
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

/// 配色资源。
#[derive(Resource, Default)]
pub struct 配色资源(pub 编辑器配色);

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
}

/// 文件对话框结果。
#[derive(Debug, Clone)]
pub enum 文件对话框结果 {
    /// 选择了一个目录。
    选定目录(PathBuf),
    /// 选择了一个文件。
    选定文件(PathBuf),
    /// 新建项目的父目录 + 项目名。
    新建项目 {
        /// 项目名。
        名称: String,
        /// 父目录。
        父目录: PathBuf,
    },
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
            .set_title("选择 Bevy Studio 项目目录")
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
            .add_filter("Bevy Studio 场景", &["ron"])
            .set_file_name("新场景.scene.ron")
            .save_file()
        {
            Some(文件) => 文件对话框结果::选定文件(文件),
            None => 文件对话框结果::取消,
        },

        文件对话框请求::打开场景文件 => match rfd::FileDialog::new()
            .set_title("打开场景")
            .add_filter("Bevy Studio 场景", &["ron"])
            .pick_file()
        {
            Some(文件) => 文件对话框结果::选定文件(文件),
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
            .init_resource::<配色资源>()
            .init_resource::<缩略图缓存>()
            .init_resource::<诊断资源>()
            .init_resource::<运行时宿主>()
            .init_resource::<文件对话框通道>()
            // ---- 配置与布局持久化 ----
            //
            // 启动时读一次；布局写盘由 `actions::保存窗口布局` 定期完成。
            // 配置文件放在用户配置目录，与项目/场景文件完全无关。
            .insert_resource(编辑器配置资源(编辑器配置::载入(&配置文件路径())))
            .init_resource::<布局待保存>()
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
                    恢复窗口布局,
                    打印启动信息,
                )
                    .chain(),
            )
            // ---- Update：只改数据，不碰 egui ----
            //
            // 顺序即数据流，见模块头部说明。
            .add_systems(
                Update,
                (
                    actions::处理快捷键,
                    actions::处理文件对话框结果,
                    actions::执行动作队列,
                    // 紧跟动作执行：本帧产生的日志（含错误）立刻出现在进程日志里。
                    actions::转发控制台日志到标准错误,
                    actions::读取键盘导航,
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
                        preview::同步预览实体,
                        preview::同步预览相机,
                        preview::同步渲染尺寸,
                    )
                        .chain(),
                    (actions::推进运行时, actions::收集运行时诊断).chain(),
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
}

/// 启动时恢复上次的窗口布局。
fn 恢复窗口布局(mut 界面: ResMut<UiStateHolder>, 配置: Res<编辑器配置资源>) {
    // 语言先恢复：它不依赖布局，而且越早生效越好 ——
    // 晚一帧恢复的话，首帧会先按中文画一次，然后闪一下变成英文。
    if let Some(代号) = 配置.0.偏好.get("语言") {
        界面.状态.语言 = editor_ui::语言::从代号(代号);
        info!("已恢复界面语言：{}", 界面.状态.语言.自称());
    }

    // 布局恢复失败**不能**影响语言：两者互相独立，
    // 用 `return` 会把语言的恢复一起取消掉。
    let Some(布局) = 配置.0.布局.as_ref() else {
        return;
    };
    match 界面.状态.从序列化恢复布局(布局) {
        Ok(()) => info!("已恢复上次的窗口布局"),
        Err(错误) => warn!("窗口布局无法恢复，改用默认布局：{错误}"),
    }
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
    /// 配色。
    pub 配色: 编辑器配色,
    /// 资源缩略图纹理表。
    pub 缩略图: &'平台 BTreeMap<String, egui::TextureHandle>,
    /// 视口的 Bevy 渲染结果。`None` 表示预览尚未就绪。
    pub 视口纹理: Option<视口渲染结果>,
    /// 运行时宿主。
    pub 运行时: &'平台 运行时宿主,
    /// 运行时诊断信息。
    pub 诊断: &'平台 运行时诊断,
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
    let mut 绘制 = 绘制上下文 {
        编辑器: 环境.编辑器,
        // 语言是界面状态的一部分（用户可在「窗口 → 界面语言」里切换），
        // 这里读出来放进上下文，各面板才能用 `上下文.语言.取(..)`。
        语言: 界面.语言,
        界面,
        动作,
        配色: 环境.配色,
        缩略图: 环境.缩略图,
        视口纹理: 环境.视口纹理,
        正在运行: 环境.运行时.状态.在运行(),
        已暂停: 环境.运行时.状态 == 运行状态::已暂停,
        运行时信息: 环境.诊断,
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
        UiAction::执行命令(命令) => format!("执行命令({})", 命令.显示名()),
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
        UiAction::设置语言(语言) => format!("设置语言({})", 语言.自称()),
        UiAction::清空控制台 => "清空控制台".to_string(),
        UiAction::显示关于 => "显示关于".to_string(),
    }
}
