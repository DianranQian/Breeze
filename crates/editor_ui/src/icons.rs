//! 图标资源：把烘焙好的 PNG 编进二进制，并按主题色染色显示。
//!
//! # 资产从哪来
//!
//! [Tabler Icons](https://github.com/tabler/tabler-icons) v3.49.0，MIT 许可，
//! 源文件（SVG）、许可证与说明都在 `crates/editor_ui/assets/icons/` 下。
//! PNG 由 `cargo run -p icon_bake` 从 SVG 烘焙得到（32×32，白色描边），
//! 详见该目录的 README。
//!
//! # 为什么用 `include_bytes!` 而不是运行时读文件
//!
//! 与内置中文字体同样的理由：图标是**程序的一部分**。
//! 运行时读文件要考虑工作目录、安装目录、资源被移动等情况，
//! 每一种都会让图标变成空白方块。编进二进制之后，
//! **干净构建与发布构建的加载路径完全一致**，也不存在"忘了带 assets 目录"。
//!
//! # 为什么是白色 PNG + tint
//!
//! 深色主题要浅色图标，浅色主题要深色图标。如果按主题各存一套，
//! 图标数量 × 主题数量会变成维护灾难。白色描边 + `Image::tint(主题色)`
//! 的等价效果是"乘以主题色"：白色不受影响，其它颜色被正确染色，
//! 于是**一套文件覆盖所有主题**。
//!
//! # DPI 与尺寸
//!
//! PNG 是 32×32，逻辑显示尺寸由主题的 `图标尺寸`（默认 16）决定。
//! 2 倍余量让 150%/200% 缩放下依然清晰；线性过滤保证非整数缩放不发毛。

use std::collections::BTreeMap;

use egui::{Color32, ColorImage, Image, TextureHandle, TextureOptions, Ui};

/// 编辑器里用到的全部图标。
///
/// 用枚举而不是字符串：漏掉一个图标时编译器会提醒，
/// 而不是在运行时显示成空白。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum 图标 {
    /// 运行。
    运行,
    /// 暂停。
    暂停,
    /// 单步。
    单步,
    /// 停止。
    停止,
    /// 撤销。
    撤销,
    /// 重做。
    重做,
    /// 保存。
    保存,
    /// 新建。
    新建,
    /// 删除。
    删除,
    /// 复制。
    复制,
    /// 重命名。
    重命名,
    /// 刷新。
    刷新,
    /// 更多。
    更多,
    /// 场景树。
    场景树,
    /// 检查器。
    检查器,
    /// 视口。
    视口,
    /// 资源浏览器。
    资源,
    /// 控制台。
    控制台,
    /// 历史。
    历史,
    /// 运行时 ECS。
    运行时ecs,
    /// 性能。
    性能,
    /// 图片资源。
    图片,
    /// 音频资源。
    音频,
    /// 代码/脚本资源。
    代码,
    /// 文本资源。
    文本,
    /// 未知类型资源。
    未知文件,
    /// 主题。
    主题,
    /// 成功。
    成功,
    /// 警告。
    警告,
    /// 信息。
    信息,
    /// 关闭。
    关闭,
    /// 工作区/布局。
    工作区,
    /// 仪表盘/统计。
    仪表盘,
    /// 太阳（浅色主题）。
    太阳,
    /// 月亮（深色主题）。
    月亮,
    /// 对比度（高对比度主题）。
    对比度,
    /// 缺陷/调试。
    缺陷,
    /// 直方图。
    直方图,
    /// 监控/性能面板。
    监控,
    /// 打开文件夹。
    打开文件夹,
    /// 世界/场景。
    世界,
    /// 放大。
    放大,
    /// 缩小。
    缩小,
    /// 最大化。
    最大化,
    /// 网格。
    网格,
    /// 文本输入。
    文本输入,
    /// 锁定。
    锁定,
    /// 可见。
    眼睛,

    // ---------------------------------------------------------------------
    // 场景树专用（节点类型图标 + 折叠指示器 + 状态开关）
    //
    // 这批图标取代了原先用字符表示的折叠箭头与 emoji 状态开关。
    // 原因见 `crate::node_icons` 的模块文档：内置中文字体子集里没有
    // `▸` / `▾` / `👁` / `🔒` 这些字形，于是它们在界面上被渲染成
    // **空心方框（tofu）** —— 用户看到的"所有节点都是空心正方形"就是这么来的。
    // ---------------------------------------------------------------------
    /// 右向箭头 → 场景树折叠指示器（节点有子实体且已折叠）。
    折叠收起,
    /// 下向箭头 → 场景树折叠指示器（节点有子实体且已展开）。
    折叠展开,
    /// 摄像机 → `Camera2d` 组件节点。
    摄像机,
    /// 地图 → `TileTerrain` 组件节点。
    地形,
    /// 立方体 → `Primitive3d` 组件节点（三维网格/模型）。
    立方体,
    /// 单点 → 没有任何可识别渲染组件的普通实体。
    实体点,
    /// 层级 → 有子实体但自身没有可识别组件的容器/分组节点。
    层级容器,
    /// 隐藏（眼睛加斜杠）→ 实体当前在编辑器中隐藏。
    隐藏,
    /// 未上锁 → 实体未锁定。
    未锁定,

    // ---- 变换工具（视口工具条）----
    /// 指针 → 选择工具。
    指针,
    /// 四向箭头 → 移动工具。
    移动,
    /// 顺时针圆弧 → 旋转工具。
    旋转,
    /// 角点缩放 → 缩放工具。
    缩放,
}

impl 图标 {
    /// 全部图标（测试用）。
    pub fn 全部() -> &'static [图标] {
        &[
            图标::运行,
            图标::暂停,
            图标::单步,
            图标::停止,
            图标::撤销,
            图标::重做,
            图标::保存,
            图标::新建,
            图标::删除,
            图标::复制,
            图标::重命名,
            图标::刷新,
            图标::更多,
            图标::场景树,
            图标::检查器,
            图标::视口,
            图标::资源,
            图标::控制台,
            图标::历史,
            图标::运行时ecs,
            图标::性能,
            图标::图片,
            图标::音频,
            图标::代码,
            图标::文本,
            图标::未知文件,
            图标::主题,
            图标::成功,
            图标::警告,
            图标::信息,
            图标::关闭,
            图标::工作区,
            图标::仪表盘,
            图标::太阳,
            图标::月亮,
            图标::对比度,
            图标::缺陷,
            图标::直方图,
            图标::监控,
            图标::打开文件夹,
            图标::世界,
            图标::放大,
            图标::缩小,
            图标::最大化,
            图标::网格,
            图标::文本输入,
            图标::锁定,
            图标::眼睛,
            图标::折叠收起,
            图标::折叠展开,
            图标::摄像机,
            图标::地形,
            图标::立方体,
            图标::实体点,
            图标::层级容器,
            图标::隐藏,
            图标::未锁定,
            图标::指针,
            图标::移动,
            图标::旋转,
            图标::缩放,
        ]
    }

    /// 稳定的资源名（同时也用于工具提示与测试断言）。
    pub fn 名称(&self) -> &'static str {
        match self {
            Self::运行 => "player-play",
            Self::暂停 => "player-pause",
            Self::单步 => "player-skip-forward",
            Self::停止 => "player-stop",
            Self::撤销 => "arrow-back-up",
            Self::重做 => "arrow-forward-up",
            Self::保存 => "device-floppy",
            Self::新建 => "plus",
            Self::删除 => "trash",
            Self::复制 => "copy",
            Self::重命名 => "pencil",
            Self::刷新 => "refresh",
            Self::更多 => "dots",
            Self::场景树 => "sitemap",
            Self::检查器 => "adjustments-horizontal",
            Self::视口 => "box",
            Self::资源 => "folder",
            Self::控制台 => "terminal-2",
            Self::历史 => "history",
            Self::运行时ecs => "cpu",
            Self::性能 => "chart-line",
            Self::图片 => "photo",
            Self::音频 => "file-music",
            Self::代码 => "file-code",
            Self::文本 => "file-text",
            Self::未知文件 => "file-unknown",
            Self::主题 => "palette",
            Self::成功 => "circle-check",
            Self::警告 => "alert-triangle",
            Self::信息 => "info-circle",
            Self::关闭 => "x",
            Self::工作区 => "layout-grid",
            Self::仪表盘 => "layout-dashboard",
            Self::太阳 => "sun",
            Self::月亮 => "moon",
            Self::对比度 => "contrast",
            Self::缺陷 => "bug",
            Self::直方图 => "chart-histogram",
            Self::监控 => "device-desktop-analytics",
            Self::打开文件夹 => "folder-open",
            Self::世界 => "world",
            Self::放大 => "zoom-in",
            Self::缩小 => "zoom-out",
            Self::最大化 => "maximize",
            Self::网格 => "grid-dots",
            Self::文本输入 => "cursor-text",
            Self::锁定 => "lock",
            Self::眼睛 => "eye",
            Self::折叠收起 => "chevron-right",
            Self::折叠展开 => "chevron-down",
            Self::摄像机 => "camera",
            Self::地形 => "map",
            Self::立方体 => "cube",
            Self::实体点 => "point",
            Self::层级容器 => "hierarchy",
            Self::隐藏 => "eye-off",
            Self::未锁定 => "lock-open",
            Self::指针 => "pointer",
            Self::移动 => "arrows-move",
            Self::旋转 => "rotate-clockwise",
            Self::缩放 => "resize",
        }
    }

    /// 编进二进制的 PNG 字节。
    pub fn 字节(&self) -> &'static [u8] {
        match self {
            Self::运行 => include_bytes!("../assets/icons/png/player-play.png"),
            Self::暂停 => include_bytes!("../assets/icons/png/player-pause.png"),
            Self::单步 => include_bytes!("../assets/icons/png/player-skip-forward.png"),
            Self::停止 => include_bytes!("../assets/icons/png/player-stop.png"),
            Self::撤销 => include_bytes!("../assets/icons/png/arrow-back-up.png"),
            Self::重做 => include_bytes!("../assets/icons/png/arrow-forward-up.png"),
            Self::保存 => include_bytes!("../assets/icons/png/device-floppy.png"),
            Self::新建 => include_bytes!("../assets/icons/png/plus.png"),
            Self::删除 => include_bytes!("../assets/icons/png/trash.png"),
            Self::复制 => include_bytes!("../assets/icons/png/copy.png"),
            Self::重命名 => include_bytes!("../assets/icons/png/pencil.png"),
            Self::刷新 => include_bytes!("../assets/icons/png/refresh.png"),
            Self::更多 => include_bytes!("../assets/icons/png/dots.png"),
            Self::场景树 => include_bytes!("../assets/icons/png/sitemap.png"),
            Self::检查器 => include_bytes!("../assets/icons/png/adjustments-horizontal.png"),
            Self::视口 => include_bytes!("../assets/icons/png/box.png"),
            Self::资源 => include_bytes!("../assets/icons/png/folder.png"),
            Self::控制台 => include_bytes!("../assets/icons/png/terminal-2.png"),
            Self::历史 => include_bytes!("../assets/icons/png/history.png"),
            Self::运行时ecs => include_bytes!("../assets/icons/png/cpu.png"),
            Self::性能 => include_bytes!("../assets/icons/png/chart-line.png"),
            Self::图片 => include_bytes!("../assets/icons/png/photo.png"),
            Self::音频 => include_bytes!("../assets/icons/png/file-music.png"),
            Self::代码 => include_bytes!("../assets/icons/png/file-code.png"),
            Self::文本 => include_bytes!("../assets/icons/png/file-text.png"),
            Self::未知文件 => include_bytes!("../assets/icons/png/file-unknown.png"),
            Self::主题 => include_bytes!("../assets/icons/png/palette.png"),
            Self::成功 => include_bytes!("../assets/icons/png/circle-check.png"),
            Self::警告 => include_bytes!("../assets/icons/png/alert-triangle.png"),
            Self::信息 => include_bytes!("../assets/icons/png/info-circle.png"),
            Self::关闭 => include_bytes!("../assets/icons/png/x.png"),
            Self::工作区 => include_bytes!("../assets/icons/png/layout-grid.png"),
            Self::仪表盘 => include_bytes!("../assets/icons/png/layout-dashboard.png"),
            Self::太阳 => include_bytes!("../assets/icons/png/sun.png"),
            Self::月亮 => include_bytes!("../assets/icons/png/moon.png"),
            Self::对比度 => include_bytes!("../assets/icons/png/contrast.png"),
            Self::缺陷 => include_bytes!("../assets/icons/png/bug.png"),
            Self::直方图 => include_bytes!("../assets/icons/png/chart-histogram.png"),
            Self::监控 => include_bytes!("../assets/icons/png/device-desktop-analytics.png"),
            Self::打开文件夹 => include_bytes!("../assets/icons/png/folder-open.png"),
            Self::世界 => include_bytes!("../assets/icons/png/world.png"),
            Self::放大 => include_bytes!("../assets/icons/png/zoom-in.png"),
            Self::缩小 => include_bytes!("../assets/icons/png/zoom-out.png"),
            Self::最大化 => include_bytes!("../assets/icons/png/maximize.png"),
            Self::网格 => include_bytes!("../assets/icons/png/grid-dots.png"),
            Self::文本输入 => include_bytes!("../assets/icons/png/cursor-text.png"),
            Self::锁定 => include_bytes!("../assets/icons/png/lock.png"),
            Self::眼睛 => include_bytes!("../assets/icons/png/eye.png"),
            Self::折叠收起 => include_bytes!("../assets/icons/png/chevron-right.png"),
            Self::折叠展开 => include_bytes!("../assets/icons/png/chevron-down.png"),
            Self::摄像机 => include_bytes!("../assets/icons/png/camera.png"),
            Self::地形 => include_bytes!("../assets/icons/png/map.png"),
            Self::立方体 => include_bytes!("../assets/icons/png/cube.png"),
            Self::实体点 => include_bytes!("../assets/icons/png/point.png"),
            Self::层级容器 => include_bytes!("../assets/icons/png/hierarchy.png"),
            Self::隐藏 => include_bytes!("../assets/icons/png/eye-off.png"),
            Self::未锁定 => include_bytes!("../assets/icons/png/lock-open.png"),
            Self::指针 => include_bytes!("../assets/icons/png/pointer.png"),
            Self::移动 => include_bytes!("../assets/icons/png/arrows-move.png"),
            Self::旋转 => include_bytes!("../assets/icons/png/rotate-clockwise.png"),
            Self::缩放 => include_bytes!("../assets/icons/png/resize.png"),
        }
    }

    /// 按扩展名推断资源图标。
    pub fn 按扩展名(扩展名: &str) -> Self {
        match 扩展名.to_ascii_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" => Self::图片,
            "ogg" | "wav" | "mp3" | "flac" => Self::音频,
            "rs" | "lua" | "js" | "ts" | "gd" | "shader" | "wgsl" | "glsl" => Self::代码,
            "ron" | "json" | "toml" | "txt" | "md" | "scene" => Self::文本,
            _ => Self::未知文件,
        }
    }
}

/// 图标纹理缓存。
///
/// # 为什么要缓存
///
/// `load_texture` 每次都会往 egui 的纹理图集里加一份新纹理。
/// 工具栏每帧要画十几个图标，不缓存的话一秒钟就会加进去上千张，
/// 显存与 draw call 都会失控。
///
/// 缓存用 `BTreeMap`（而不是 HashMap）：图标数量很小、遍历顺序稳定，
/// 调试输出与截图都更容易复现。
#[derive(Default)]
pub struct 图标库 {
    纹理: BTreeMap<图标, TextureHandle>,
    /// 上传失败的图标（正常情况为空）。
    失败: Vec<图标>,
    /// 已上传的纹理数量（验收证据用）。
    已上传: usize,
}

impl std::fmt::Debug for 图标库 {
    fn fmt(&self, 格式化器: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        格式化器
            .debug_struct("图标库")
            .field("已上传", &self.已上传)
            .field("失败", &self.失败)
            .finish()
    }
}

impl 图标库 {
    /// 新建空缓存。
    pub fn 新() -> Self {
        Self::default()
    }

    /// 已上传的纹理数量。
    pub fn 已上传数量(&self) -> usize {
        self.已上传
    }

    /// 上传失败的图标列表。
    pub fn 失败列表(&self) -> &[图标] {
        &self.失败
    }

    /// 取（必要时上传）图标纹理。
    pub fn 纹理(&mut self, 上下文: &egui::Context, 图标: 图标) -> Option<&TextureHandle> {
        // 用 `Entry` 保持单次查找：图标缓存每帧都会被查，
        // 虽然开销很小，但这个模式更容易读。
        if let std::collections::btree_map::Entry::Vacant(空位) = self.纹理.entry(图标) {
            match 解码(图标) {
                Ok(颜色图) => {
                    let 纹理 = 上下文.load_texture(
                        format!("bevy_studio_icon_{}", 图标.名称()),
                        颜色图,
                        // 线性过滤：图标在 125%/150% 缩放下会被非整数缩放，
                        // 最近邻会变成锯齿，线性过滤至少是"轻微模糊"而不是破图。
                        TextureOptions::LINEAR,
                    );
                    空位.insert(纹理);
                    self.已上传 = self.纹理.len();
                }
                Err(错误) => {
                    // 缺失/损坏的图标不应该让界面崩溃：记下来，退回文字标签。
                    eprintln!("图标 `{}` 加载失败：{错误}", 图标.名称());
                    if !self.失败.contains(&图标) {
                        self.失败.push(图标);
                    }
                    return None;
                }
            }
        }
        self.纹理.get(&图标)
    }

    /// 画一个图标。返回是否画出来了。
    ///
    /// 缺失图标时画一个占位方块并给出工具提示，
    /// 这样"资源没打进二进制"会立刻在界面上暴露，而不是静默消失。
    pub fn 绘制(
        &mut self, ui: &mut Ui, 图标: 图标, 尺寸: f32, 色调: Color32
    ) -> egui::Response {
        let Some(纹理) = self.纹理(ui.ctx(), 图标) else {
            return ui
                .label(egui::RichText::new("?").color(色调))
                .on_hover_text(format!("图标资源缺失：{}", 图标.名称()));
        };
        let 图片 = Image::new(egui::load::SizedTexture::new(
            纹理.id(),
            egui::vec2(尺寸, 尺寸),
        ))
        .tint(色调);
        ui.add(图片)
    }

    /// 图标按钮：图标 + 可选文字。
    ///
    /// 用 `Button::image_and_text` 而不是自己拼 layout：
    /// 这样悬停/按下/禁用状态、键盘焦点、点击判定全部由 egui 处理，
    /// 视觉与行为跟普通按钮完全一致。
    pub fn 图标按钮(
        &mut self,
        ui: &mut Ui,
        图标: 图标,
        文字: Option<&str>,
        尺寸: f32,
        色调: Color32,
        启用: bool,
    ) -> egui::Response {
        // 纹理缺失（资源没打进二进制）时退回纯文字按钮：
        // 宁可少一个图标，也不要画一个空白的不可点按钮。
        let Some(纹理) = self.纹理(ui.ctx(), 图标) else {
            let 文字 = 文字.unwrap_or(图标.名称());
            return ui
                .add_enabled(启用, egui::Button::new(文字))
                .on_hover_text(format!("图标资源缺失：{}", 图标.名称()));
        };
        let 图片 = egui::Image::new(egui::load::SizedTexture::new(
            纹理.id(),
            egui::vec2(尺寸, 尺寸),
        ))
        .tint(色调);
        let 按钮 = match 文字 {
            Some(文字) => {
                egui::Button::image_and_text(图片, egui::RichText::new(文字).size(12.0))
            }
            None => egui::Button::image(图片),
        };
        ui.add_enabled(启用, 按钮)
    }
}

/// 把一个图标的 PNG 解成 `ColorImage`。
///
/// 解码只做一次（结果进纹理缓存）。PNG 很小（约 1 KB），
/// 全部 49 个图标解码后约 49 × 32 × 32 × 4 ≈ 200 KB。
pub fn 解码(图标: 图标) -> Result<ColorImage, String> {
    let 图片 =
        image::load_from_memory(图标.字节()).map_err(|错误| format!("PNG 解码失败：{错误}"))?;
    let rgba = 图片.to_rgba8();
    let (宽, 高) = rgba.dimensions();
    if 宽 == 0 || 高 == 0 {
        return Err("图标尺寸为 0".to_string());
    }
    Ok(ColorImage::from_rgba_unmultiplied(
        [宽 as usize, 高 as usize],
        rgba.as_raw(),
    ))
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 每个图标都能解码出可见像素() {
        for 图标 in 图标::全部() {
            let 颜色图 = 解码(*图标)
                .unwrap_or_else(|错误| panic!("图标 `{}` 解码失败：{错误}", 图标.名称()));
            assert_eq!(颜色图.size, [32, 32], "图标 `{}` 应当是 32×32", 图标.名称());
            // 至少有一部分像素不透明，否则说明烘焙出来是空图。
            let 不透明 = 颜色图.pixels.iter().filter(|像素| 像素.a() > 0).count();
            assert!(
                不透明 > 20,
                "图标 `{}` 几乎没有可见像素（{不透明} 个）：烘焙参数可能错了",
                图标.名称()
            );
        }
    }

    #[test]
    fn 图标名称唯一且资源真实存在() {
        let mut 名称: Vec<&str> = 图标::全部().iter().map(|图标| 图标.名称()).collect();
        let 原数量 = 名称.len();
        名称.sort_unstable();
        名称.dedup();
        assert_eq!(名称.len(), 原数量, "图标资源名必须唯一");

        // 每个图标的 `名称()` 都必须对应一个真实存在的 SVG 源文件。
        //
        // # 为什么不再断言"数量 == 某个硬编码常量"
        //
        // 上一版写的是 `assert_eq!(图标::全部().len(), 48)`，注释却说源文件有 49 个
        // —— 那条断言既挡住了"新增图标忘记加进清单"，也把真实基线记错了，
        // 于是它只能靠人手同步，且在真正需要扩展时先失败。
        //
        // 现在改成检查**真实的文件名对应关系**：新增图标忘记放进 `全部()`
        // 由 `icon_bake --check` 与下面的"未使用源文件"提示暴露；
        // 而写错文件名（`名称()` 指向不存在的 SVG）会立刻在这里失败。
        let svg目录 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("icons")
            .join("svg");
        let mut 源文件: Vec<String> = std::fs::read_dir(&svg目录)
            .unwrap_or_else(|错误| panic!("读不到图标源目录 {}：{错误}", svg目录.display()))
            .filter_map(|项| 项.ok())
            .filter_map(|项| {
                let 路径 = 项.path();
                if 路径.extension().is_some_and(|扩展| 扩展 == "svg") {
                    路径.file_stem().map(|名| 名.to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect();
        源文件.sort();
        assert!(
            !源文件.is_empty(),
            "图标源目录是空的：{}",
            svg目录.display()
        );

        for 图标 in 图标::全部() {
            let 名 = 图标.名称();
            assert!(
                源文件.iter().any(|源| 源 == 名),
                "图标 `{名}` 缺少 SVG 源文件（{}）；可用的有：{源文件:?}",
                svg目录.join(format!("{名}.svg")).display()
            );
        }

        // 没有被任何图标使用的源文件：不算错误（可能是为将来准备的）。
        // 与 `tools/gen_messages.py` 对"僵尸词条"的处理保持一致。
        let 未使用: Vec<&str> = 源文件
            .iter()
            .map(String::as_str)
            .filter(|源| !名称.contains(源))
            .collect();
        if !未使用.is_empty() {
            println!(
                "提示：{} 个 SVG 源文件未被任何图标使用：{未使用:?}",
                未使用.len()
            );
        }
    }

    #[test]
    fn 图片文件走白色描边以便染色() {
        // 烘焙时把 currentColor 换成白色；解码后最亮的像素应当接近纯白。
        let 颜色图 = 解码(图标::运行).expect("应当能解码");
        let 最亮 = 颜色图
            .pixels
            .iter()
            .filter(|像素| 像素.a() > 128)
            .map(|像素| 像素.r() as u16 + 像素.g() as u16 + 像素.b() as u16)
            .max()
            .unwrap_or(0);
        assert!(
            最亮 > 700,
            "运行图标的亮度和是 {最亮}，说明描边不是白色，tint 染色会偏色"
        );
    }

    #[test]
    fn 按扩展名推断图标() {
        assert_eq!(图标::按扩展名("PNG"), 图标::图片);
        assert_eq!(图标::按扩展名("ogg"), 图标::音频);
        assert_eq!(图标::按扩展名("rs"), 图标::代码);
        assert_eq!(图标::按扩展名("scene.ron"), 图标::未知文件);
        assert_eq!(图标::按扩展名("ron"), 图标::文本);
        assert_eq!(图标::按扩展名("xyz"), 图标::未知文件);
    }

    #[test]
    fn 图标库初始为空() {
        let 库 = 图标库::新();
        assert_eq!(库.已上传数量(), 0);
        assert!(库.失败列表().is_empty());
    }
}
