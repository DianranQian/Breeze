// SPDX-License-Identifier: MIT OR Apache-2.0
//! 图标烘焙工具：Tabler Icons 的 SVG 源图 → 编辑器用的 2x PNG。
//!
//! # 为什么要烘焙，而不是运行时渲染 SVG
//!
//! 对比过两条路：
//!
//! | 方案 | 运行时依赖 | 构建复杂度 | 维护成本 |
//! | --- | --- | --- | --- |
//! | 运行时用 resvg 渲染 | +resvg/usvg/tiny-skia（约 30 个传递依赖） | 每次启动解析 SVG | 高 |
//! | **构建期烘焙成 PNG** | **无** | 本工具一次性运行 | 低 |
//!
//! egui 本身不能把 SVG 当图片加载，运行时要显示矢量图必须引入一个渲染器。
//! 而编辑器图标只有十几种固定尺寸（16 逻辑像素 × DPI 缩放），
//! 预渲染成 32×32 PNG 后由 egui 缩放显示，画质足够、内存更省，
//! 而且**发布构建不依赖任何 SVG 解析代码**。
//!
//! 因此 resvg 只出现在这个构建期工具里，编辑器的运行时依赖没有变化。
//!
//! # 用法
//!
//! ```text
//! cargo run -p icon_bake              # 烘焙全部 SVG 到 PNG
//! cargo run -p icon_bake -- --check   # 只校验 PNG 是否齐全且尺寸正确（CI 用）
//! ```
//!
//! # 为什么源图统一染色成白色
//!
//! Tabler 的描边色是 `currentColor`。烘焙时统一替换成纯白，
//! 运行时再用 `egui::Image::tint(主题色)` 上色：这样一个图标文件
//! 在深色/浅色/高对比度主题下都能用，且不需要为每个主题各存一套。
//! 白色 + `tint` 等价于"多乘一个颜色"，白色不受影响，其他色被正确染色。

use std::path::{Path, PathBuf};

/// 源 SVG 目录（相对仓库根）。
const SVG目录: &str = "crates/editor_ui/assets/icons/svg";
/// 输出 PNG 目录。
const PNG目录: &str = "crates/editor_ui/assets/icons/png";
/// 输出边长（逻辑 16px × 2 倍 DPI）。
const 输出边长: u32 = 32;

fn main() {
    let 参数: Vec<String> = std::env::args().skip(1).collect();
    let 只校验 = 参数.iter().any(|项| 项 == "--check");

    let 根 = 仓库根();
    let svg目录 = 根.join(SVG目录);
    let png目录 = 根.join(PNG目录);

    if !svg目录.is_dir() {
        eprintln!("找不到 SVG 目录：{}", svg目录.display());
        std::process::exit(2);
    }
    if !只校验 && let Err(错误) = std::fs::create_dir_all(&png目录) {
        eprintln!("无法创建 PNG 目录 {}：{错误}", png目录.display());
        std::process::exit(2);
    }

    let mut 源列表: Vec<PathBuf> = match std::fs::read_dir(&svg目录) {
        Ok(读) => 读
            .filter_map(|项| 项.ok())
            .map(|项| 项.path())
            .filter(|路径| 路径.extension().is_some_and(|扩展| 扩展 == "svg"))
            .collect(),
        Err(错误) => {
            eprintln!("读不了 SVG 目录：{错误}");
            std::process::exit(2);
        }
    };
    源列表.sort();
    if 源列表.is_empty() {
        eprintln!("SVG 目录里没有图标：{}", svg目录.display());
        std::process::exit(2);
    }

    let mut 失败 = 0usize;
    let mut 总字节 = 0u64;

    for 源 in &源列表 {
        let 名 = 源
            .file_stem()
            .map(|名| 名.to_string_lossy().to_string())
            .unwrap_or_default();
        let 目标 = png目录.join(format!("{名}.png"));

        if 只校验 {
            match 校验(&目标) {
                Ok(大小) => {
                    总字节 += 大小;
                    println!("✓ {名}.png {输出边长}×{输出边长}（{大小} 字节）");
                }
                Err(错误) => {
                    eprintln!("✗ {名}：{错误}");
                    失败 += 1;
                }
            }
            continue;
        }

        match 烘焙(源, &目标) {
            Ok(大小) => {
                总字节 += 大小;
                println!("{名}.png  {大小} 字节");
            }
            Err(错误) => {
                eprintln!("烘焙失败 {名}：{错误}");
                失败 += 1;
            }
        }
    }

    println!(
        "\n{} 个图标，合计 {:.1} KB，输出 {}×{}",
        源列表.len(),
        总字节 as f64 / 1024.0,
        输出边长,
        输出边长
    );
    if 失败 > 0 {
        eprintln!("{失败} 个图标失败");
        std::process::exit(1);
    }
}

/// 仓库根目录。
///
/// `cargo run -p icon_bake` 的工作目录就是仓库根，因此直接用当前目录；
/// 这样不必去猜可执行文件位置（那在 `cargo install` 之后必然猜错）。
fn 仓库根() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// 烘焙一个 SVG。
fn 烘焙(源: &Path, 目标: &Path) -> Result<u64, String> {
    let 文本 =
        std::fs::read_to_string(源).map_err(|错误| format!("读不到 {}：{错误}", 源.display()))?;
    // Tabler 用 `currentColor` 描边；统一换成白色，运行时靠 tint 上色。
    let 文本 = 文本.replace("currentColor", "#FFFFFF");

    let 选项 = resvg::usvg::Options::default();
    let 树 = resvg::usvg::Tree::from_str(&文本, &选项)
        .map_err(|错误| format!("SVG 解析失败：{错误}"))?;

    let mut 位图 = resvg::tiny_skia::Pixmap::new(输出边长, 输出边长)
        .ok_or_else(|| "无法分配位图".to_string())?;
    let 源边长 = 树.size().width().max(1.0);
    let 缩放 = 输出边长 as f32 / 源边长;
    resvg::render(
        &树,
        resvg::tiny_skia::Transform::from_scale(缩放, 缩放),
        &mut 位图.as_mut(),
    );
    位图
        .save_png(目标)
        .map_err(|错误| format!("写 PNG 失败：{错误}"))?;

    std::fs::metadata(目标)
        .map(|元数据| 元数据.len())
        .map_err(|错误| format!("读不到输出文件：{错误}"))
}

/// 校验一个 PNG 是否存在且尺寸正确。
fn 校验(目标: &Path) -> Result<u64, String> {
    let 文件 = std::fs::File::open(目标).map_err(|错误| {
        format!(
            "缺少 {}（请运行 cargo run -p icon_bake）：{错误}",
            目标.display()
        )
    })?;
    let 解码器 = png::Decoder::new(文件);
    let 读取器 = 解码器
        .read_info()
        .map_err(|错误| format!("PNG 头损坏：{错误}"))?;
    let (宽, 高) = 读取器.info().size();
    if 宽 != 输出边长 || 高 != 输出边长 {
        return Err(format!("尺寸是 {宽}×{高}，应当是 {输出边长}×{输出边长}"));
    }
    std::fs::metadata(目标)
        .map(|元数据| 元数据.len())
        .map_err(|错误| format!("读不到输出文件：{错误}"))
}
