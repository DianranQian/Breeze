//! `gui_probe`：Bevy Studio 的 GUI 自动验收工具。
//!
//! # 它解决的问题
//!
//! 上一轮验收的做法是"从进程外用 `PrintWindow` 抓一张图，然后靠人眼看"。
//! 这个流程有两个致命问题：
//!
//! 1. `PrintWindow` 对 wgpu 窗口不可靠 —— 会拿到陈旧帧、纯黑或半张图，
//!    于是"看起来坏了"与"真的坏了"分不开，改代码就没有依据；
//! 2. 判断依赖人眼 —— 既不可重复，也没法进 CI。
//!
//! 现在改成：应用自己从**渲染目标**回读这一帧的窗口画面（见
//! `editor_app::capture`），同一帧把布局几何写成 JSON；本工具负责
//! **确认窗口 → 读证据 → 做像素与几何断言 → 给出分类判定**。
//!
//! # 退出码
//!
//! | 码 | 含义 |
//! | --- | --- |
//! | 0 | 全部通过 |
//! | 2 | 界面缺陷（采集正常，断言不成立） |
//! | 3 | 采集失败（文件缺失/尺寸不符/窗口不存在/超时） |
//! | 4 | 用法错误 |
//!
//! # 用法
//!
//! ```text
//! gui_probe run --exe target\debug\bevy-studio.exe --out .verify\general `
//!               --expect-language zh --expect-theme dark --expect-panels scene_tree,viewport,inspector
//! gui_probe analyze --png shot.png --json shot.json
//! gui_probe window --pid 12345
//! ```

mod analysis;
mod evidence;
mod image_stats;
mod input;
mod window_probe;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use analysis::{分析结果, 判定, 断言类别, 期望};
use evidence::证据;
use image_stats::图像;
use window_probe::窗口信息;

fn main() {
    // 必须先声明 DPI 感知：否则 Windows 会把窗口尺寸虚拟化成 96 DPI，
    // 截图（物理像素）与客户区（逻辑像素）永远对不上，验收会误报采集失败。
    #[cfg(windows)]
    window_probe::提升dpi感知();

    let 参数: Vec<String> = std::env::args().skip(1).collect();
    let 码 = 分派(&参数);
    std::process::exit(码);
}

fn 分派(参数: &[String]) -> i32 {
    let Some(子命令) = 参数.first().map(String::as_str) else {
        用法();
        return 4;
    };
    match 子命令 {
        "run" => 运行(参数),
        "drag" => 拖拽(参数),
        "analyze" => 只分析(参数),
        "window" => 查窗口(参数),
        "help" | "--help" | "-h" => {
            用法();
            0
        }
        其他 => {
            eprintln!("未知子命令：{其他}");
            用法();
            4
        }
    }
}

fn 用法() {
    println!(
        r#"gui_probe —— Bevy Studio GUI 自动验收

用法：
  gui_probe run     --exe <应用路径> [--out <目录>] [--name <前缀>] [选项]
  gui_probe analyze --png <截图> --json <证据> [选项]
  gui_probe window  --pid <进程号>

run 的选项：
  --out <目录>            输出目录（默认 .verify/runs）
  --name <前缀>           输出文件名前缀（默认 general）
  --timeout <秒>          等待采集的超时（默认 180）
  --frame <帧号>          在第几帧采集（默认 90）
  --window <宽x高>        以指定窗口尺寸启动
  --env K=V               追加环境变量（可重复）

共同选项（也适用于 analyze / run 之后的判定）：
  --expect-language zh|en
  --expect-theme dark|light
  --expect-panels a,b,c
  --expect-status <片段>   可重复
  --expect-mode <id>      期望激活的主开发模式（2d/3d/script/run/assets）
  --expect-workspace <id> 期望激活的工作区 id
  --min-viewport <0..1>   视口最小宽度占比（默认 0.30）
"#
    );
}

/// 解析出的命令行选项。
#[derive(Debug, Default, Clone)]
struct 选项 {
    exe: Option<PathBuf>,
    out: Option<PathBuf>,
    name: Option<String>,
    png: Option<PathBuf>,
    json: Option<PathBuf>,
    pid: Option<u32>,
    timeout: Option<u64>,
    frame: Option<u32>,
    window: Option<String>,
    env: Vec<(String, String)>,
    期望: 期望,
}

fn 解析(参数: &[String]) -> Result<选项, String> {
    let mut 选项 = 选项 {
        期望: 期望::default(),
        ..Default::default()
    };
    let mut 下标 = 1;
    while 下标 < 参数.len() {
        let 键 = 参数[下标].as_str();
        let 取值 = |下标: &mut usize| -> Result<String, String> {
            *下标 += 1;
            参数
                .get(*下标)
                .cloned()
                .ok_or_else(|| format!("选项 `{键}` 缺少参数"))
        };
        match 键 {
            "--exe" => 选项.exe = Some(PathBuf::from(取值(&mut 下标)?)),
            "--out" => 选项.out = Some(PathBuf::from(取值(&mut 下标)?)),
            "--name" => 选项.name = Some(取值(&mut 下标)?),
            "--png" => 选项.png = Some(PathBuf::from(取值(&mut 下标)?)),
            "--json" => 选项.json = Some(PathBuf::from(取值(&mut 下标)?)),
            "--pid" => {
                选项.pid = Some(
                    取值(&mut 下标)?
                        .parse()
                        .map_err(|_| "`--pid` 需要数字".to_string())?,
                )
            }
            "--timeout" => {
                选项.timeout = Some(
                    取值(&mut 下标)?
                        .parse()
                        .map_err(|_| "`--timeout` 需要秒数".to_string())?,
                )
            }
            "--frame" => {
                选项.frame = Some(
                    取值(&mut 下标)?
                        .parse()
                        .map_err(|_| "`--frame` 需要帧号".to_string())?,
                )
            }
            "--window" => 选项.window = Some(取值(&mut 下标)?),
            "--env" => {
                let 值 = 取值(&mut 下标)?;
                let (键, 值) = 值
                    .split_once('=')
                    .ok_or_else(|| format!("`--env` 需要 K=V 形式，实际 `{值}`"))?;
                选项.env.push((键.to_string(), 值.to_string()));
            }
            "--expect-language" => 选项.期望.语言 = Some(取值(&mut 下标)?),
            "--expect-theme" => 选项.期望.明暗 = Some(取值(&mut 下标)?),
            "--expect-panels" => {
                选项.期望.必须面板 = 取值(&mut 下标)?
                    .split(',')
                    .map(str::trim)
                    .filter(|项| !项.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "--expect-status" => 选项.期望.状态栏包含.push(取值(&mut 下标)?),
            "--expect-mode" => {
                let id = 取值(&mut 下标)?;
                选项.期望.轮廓 = analysis::布局轮廓::从id(&id);
                选项.期望.模式 = Some(id);
            }
            "--expect-workspace" => {
                let id = 取值(&mut 下标)?;
                选项.期望.轮廓 = analysis::布局轮廓::从id(&id);
                选项.期望.工作区 = Some(id);
            }
            "--min-viewport" => {
                选项.期望.最小视口宽度占比 = 取值(&mut 下标)?
                    .parse()
                    .map_err(|_| "`--min-viewport` 需要 0..1 的小数".to_string())?;
            }
            其他 => return Err(format!("未知选项：{其他}")),
        }
        下标 += 1;
    }
    Ok(选项)
}

fn 查窗口(参数: &[String]) -> i32 {
    let Ok(选项) = 解析(参数) else {
        eprintln!("参数错误");
        return 4;
    };
    let Some(pid) = 选项.pid else {
        eprintln!("`window` 需要 --pid");
        return 4;
    };
    match window_probe::查找(pid) {
        Some(信息) => {
            println!("{}", 窗口摘要(&信息));
            0
        }
        None => {
            eprintln!("进程 {pid} 没有可见窗口");
            3
        }
    }
}

fn 窗口摘要(信息: &窗口信息) -> String {
    format!(
        "窗口 {}：标题「{}」类名「{}」可见={} 客户区={}×{} 客户区原点=({}, {}) 尺寸={}×{} DPI={}",
        信息.句柄,
        信息.标题,
        信息.类名,
        信息.可见,
        信息.客户区.0,
        信息.客户区.1,
        信息.窗口矩形.0,
        信息.窗口矩形.1,
        信息.窗口矩形.2 - 信息.窗口矩形.0,
        信息.窗口矩形.3 - 信息.窗口矩形.1,
        信息.dpi
    )
}

fn 只分析(参数: &[String]) -> i32 {
    let 选项 = match 解析(参数) {
        Ok(选项) => 选项,
        Err(错误) => {
            eprintln!("参数错误：{错误}");
            return 4;
        }
    };
    let (Some(png), Some(json)) = (选项.png.as_ref(), 选项.json.as_ref()) else {
        eprintln!("`analyze` 需要 --png 与 --json");
        return 4;
    };
    let 输出目录 = 选项
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(".verify/runs"));

    match 执行分析(png, json, None, &选项.期望) {
        Ok(结果) => {
            let _ = 写报告(
                &输出目录,
                选项.name.as_deref().unwrap_or("analyze"),
                png,
                json,
                &结果,
                None,
            );
            打印结果(&结果);
            结果.判定.退出码()
        }
        Err(错误) => {
            eprintln!("采集失败：{错误}");
            3
        }
    }
}

fn 运行(参数: &[String]) -> i32 {
    let 选项 = match 解析(参数) {
        Ok(选项) => 选项,
        Err(错误) => {
            eprintln!("参数错误：{错误}");
            return 4;
        }
    };
    let Some(exe) = 选项.exe.clone() else {
        eprintln!("`run` 需要 --exe");
        return 4;
    };
    if !exe.is_file() {
        eprintln!("可执行文件不存在：{}", exe.display());
        return 4;
    }

    let 输出目录 = 选项
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(".verify/runs"));
    let 名称 = 选项.name.clone().unwrap_or_else(|| "general".to_string());
    if let Err(错误) = std::fs::create_dir_all(&输出目录) {
        eprintln!("无法创建输出目录 {}：{错误}", 输出目录.display());
        return 4;
    }
    let png = 输出目录.join(format!("{名称}.png"));
    let json = 输出目录.join(format!("{名称}.json"));
    let log = 输出目录.join(format!("{名称}.log"));

    // 先删旧文件：否则一次失败会读到上一次的旧证据，把"没采到"误判成"通过"。
    for 路径 in [&png, &json] {
        if 路径.exists() {
            let _ = std::fs::remove_file(路径);
        }
    }

    let 超时 = Duration::from_secs(选项.timeout.unwrap_or(180));
    let 帧 = 选项.frame.unwrap_or(90);

    let mut 命令 = Command::new(&exe);
    命令
        .env("BEVY_STUDIO_CAPTURE_PNG", &png)
        .env("BEVY_STUDIO_CAPTURE_JSON", &json)
        .env("BEVY_STUDIO_CAPTURE_FRAME", 帧.to_string())
        .env("BEVY_STUDIO_CAPTURE_EXIT", "1");
    if let Some(尺寸) = 选项.window.as_deref() {
        命令.env("BEVY_STUDIO_WINDOW_SIZE", 尺寸);
    }
    for (键, 值) in &选项.env {
        命令.env(键, 值);
    }

    let 日志文件 = match std::fs::File::create(&log) {
        Ok(文件) => 文件,
        Err(错误) => {
            eprintln!("无法创建日志文件 {}：{错误}", log.display());
            return 4;
        }
    };
    let 日志副本 = match 日志文件.try_clone() {
        Ok(副本) => 副本,
        Err(错误) => {
            eprintln!("无法复制日志句柄：{错误}");
            return 4;
        }
    };
    命令
        .stdout(Stdio::from(日志文件))
        .stderr(Stdio::from(日志副本));

    let mut 子进程 = match 命令.spawn() {
        Ok(子进程) => 子进程,
        Err(错误) => {
            eprintln!("无法启动应用 {}：{错误}", exe.display());
            return 3;
        }
    };
    let 进程号 = 子进程.id();

    // 边等边查窗口：应用采完就退出，窗口信息必须在它活着的时候取。
    let 开始 = Instant::now();
    let mut 窗口信息: Option<窗口信息> = None;
    let mut 采集完成 = false;
    loop {
        // 每轮都重新查一次并保留更好的那个：winit 在初始化早期会短暂存在
        // 标题为空、客户区 0×0 的辅助窗口，只取第一次会记住那个假窗口。
        if let Some(候选) = window_probe::查找(进程号)
            && 更优窗口(&候选, 窗口信息.as_ref())
        {
            窗口信息 = Some(候选);
        }
        if png.is_file() && json.is_file() {
            采集完成 = true;
            break;
        }
        if let Ok(Some(状态)) = 子进程.try_wait() {
            if !状态.success() {
                eprintln!("应用在采集前退出，退出码 {状态}");
            }
            break;
        }
        if 开始.elapsed() > 超时 {
            eprintln!("等待采集超时（{} 秒）", 超时.as_secs());
            break;
        }
        std::thread::sleep(Duration::from_millis(120));
    }

    // 采集完成后给进程一点时间自行退出，再确保它不再占着文件。
    等待退出(&mut 子进程, Duration::from_secs(15));

    let 日志尾部 = 读取日志尾部(&log, 40);

    if !采集完成 {
        eprintln!("采集失败：没有同时得到截图与证据文件");
        if !日志尾部.is_empty() {
            eprintln!("应用日志尾部：\n{日志尾部}");
        }
        let _ = 写失败报告(&输出目录, &名称, &png, &json, &日志尾部, 窗口信息.as_ref());
        return 3;
    }

    match 执行分析(&png, &json, 窗口信息.as_ref(), &选项.期望) {
        Ok(结果) => {
            let _ = 写报告(
                &输出目录,
                &名称,
                &png,
                &json,
                &结果,
                Some((&log, &日志尾部)),
            );
            打印结果(&结果);
            if 结果.判定 != 判定::通过 {
                eprintln!("日志尾部：\n{日志尾部}");
            }
            结果.判定.退出码()
        }
        Err(错误) => {
            eprintln!("采集失败：{错误}");
            let _ = 写失败报告(&输出目录, &名称, &png, &json, &日志尾部, 窗口信息.as_ref());
            3
        }
    }
}

/// 新窗口信息是否比已有的更好。
///
/// 排序依据：先看是否为可见的 `Bevy Studio` 主窗口，再看客户区面积。
/// 把"标题匹配"放在面积之前，是因为主窗口刚创建时可能还没设置标题，
/// 但真正的辅助窗口永远不会叫 Bevy Studio。
fn 更优窗口(新: &窗口信息, 旧: Option<&窗口信息>) -> bool {
    let Some(旧) = 旧 else {
        return true;
    };
    let 评分 = |信息: &窗口信息| {
        let 标题匹配 = 信息.标题.contains("Bevy Studio") as i32;
        let 可见 = 信息.可见 as i32;
        (标题匹配, 可见, 信息.客户区.0 as i64 * 信息.客户区.1 as i64)
    };
    评分(新) > 评分(旧)
}

fn 等待退出(子进程: &mut Child, 最长: Duration) {
    let 开始 = Instant::now();
    loop {
        match 子进程.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {
                if 开始.elapsed() > 最长 {
                    let _ = 子进程.kill();
                    let _ = 子进程.wait();
                    return;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            Err(_) => {
                let _ = 子进程.kill();
                return;
            }
        }
    }
}

fn 读取日志尾部(路径: &Path, 行数: usize) -> String {
    let Ok(文本) = std::fs::read_to_string(路径) else {
        return String::new();
    };
    let 全部: Vec<&str> = 文本.lines().collect();
    let 起点 = 全部.len().saturating_sub(行数);
    全部[起点..].join("\n")
}

fn 执行分析(
    png: &Path,
    证据路径: &Path,
    窗口信息: Option<&窗口信息>,
    期望: &期望,
) -> Result<分析结果, String> {
    let 图 = 图像::读取(png)?;
    let 证据 = 证据::读取(证据路径)?;
    Ok(analysis::分析(analysis::输入 {
        图像: &图,
        证据: &证据,
        窗口: 窗口信息,
        期望,
    }))
}

fn 打印结果(结果: &分析结果) {
    println!(
        "判定：{}（截图 {}×{}，均亮度 {:.1}，主导色 {}）",
        结果.判定.描述(),
        结果.截图尺寸.0,
        结果.截图尺寸.1,
        结果.均亮度,
        结果.主导色
    );
    println!(
        "  断言 {} 条，失败 {}",
        结果.断言.len(),
        结果.失败项().len()
    );
    for 项 in &结果.断言 {
        let 类别 = match 项.类别 {
            断言类别::采集 => "采集",
            断言类别::界面 => "界面",
        };
        println!(
            "  [{}] {:<32} [{}] {}",
            if 项.通过 { "通过" } else { "失败" },
            项.名称,
            类别,
            项.说明
        );
    }
}

/// 证据里的元信息，原样进报告：验收结果必须能追溯到是哪一次构建、哪种主题。
fn 证据元信息(json: &Path) -> serde_json::Value {
    match 证据::读取(json) {
        Ok(证据) => serde_json::json!({
            "schema": 证据.schema,
            "app_version": 证据.app_version,
            "bevy_version": 证据.bevy_version,
            "language": 证据.language,
            "language_label": 证据.language_label,
            "ui_scale": 证据.ui_scale,
            "pixels_per_point": 证据.pixels_per_point,
            "theme_id": 证据.theme.id,
            "theme_panel_bg": 证据.theme.panel_bg,
            "theme_text": 证据.theme.text,
            "theme_accent": 证据.theme.accent,
            "icon_count": 证据.icon_count,
            "viewport_scale_percent": 证据.viewport_scale_percent,
            "panel_titles": 证据.panel_titles,
            "status_left": 证据.status_left,
            "status_right": 证据.status_right,
        }),
        Err(错误) => serde_json::json!({ "error": 错误 }),
    }
}

/// 写报告（JSON + Markdown）。
fn 写报告(
    输出目录: &Path,
    名称: &str,
    png: &Path,
    json: &Path,
    结果: &分析结果,
    日志: Option<(&Path, &str)>,
) -> Result<(), String> {
    std::fs::create_dir_all(输出目录)
        .map_err(|错误| format!("无法创建目录 {}：{错误}", 输出目录.display()))?;

    let 断言列表: Vec<serde_json::Value> = 结果
        .断言
        .iter()
        .map(|项| {
            serde_json::json!({
                "name": 项.名称,
                "pass": 项.通过,
                "category": match 项.类别 { 断言类别::采集 => "capture", 断言类别::界面 => "ui" },
                "detail": 项.说明,
            })
        })
        .collect();

    let 报告 = serde_json::json!({
        "verdict": match 结果.判定 { 判定::通过 => "pass", 判定::界面缺陷 => "ui_defect", 判定::采集失败 => "capture_failed" },
        "exit_code": 结果.判定.退出码(),
        "png": png.display().to_string(),
        "evidence": json.display().to_string(),
        "image": { "width": 结果.截图尺寸.0, "height": 结果.截图尺寸.1 },
        "mean_luma": 结果.均亮度,
        "dominant_color": 结果.主导色,
        "evidence_meta": 证据元信息(json),
        "checks": 断言列表,
        "app_log_tail": 日志.map(|(_, 文本)| 文本.to_string()),
    });
    let 文本 =
        serde_json::to_string_pretty(&报告).map_err(|错误| format!("报告序列化失败：{错误}"))?;
    let _ = 日志;
    std::fs::write(输出目录.join(format!("{名称}.report.json")), 文本)
        .map_err(|错误| format!("写报告失败：{错误}"))?;

    // Markdown 版本：给人看，也给工作日志直接引用。
    let mut md = String::new();
    md.push_str(&format!("# GUI 验收：{名称}\n\n"));
    md.push_str(&format!(
        "- 判定：**{}**（退出码 {}）\n",
        结果.判定.描述(),
        结果.判定.退出码()
    ));
    md.push_str(&format!("- 截图：`{}`\n", png.display()));
    md.push_str(&format!("- 证据：`{}`\n", json.display()));
    md.push_str(&format!(
        "- 截图尺寸：{}×{}，平均亮度 {:.1}，主导色 {}\n\n",
        结果.截图尺寸.0, 结果.截图尺寸.1, 结果.均亮度, 结果.主导色
    ));
    md.push_str("| 断言 | 类别 | 结果 | 说明 |\n| --- | --- | --- | --- |\n");
    println!(
        "  断言 {} 条，失败 {}",
        结果.断言.len(),
        结果.失败项().len()
    );
    for 项 in &结果.断言 {
        md.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            项.名称,
            match 项.类别 {
                断言类别::采集 => "采集",
                断言类别::界面 => "界面",
            },
            if 项.通过 { "✅" } else { "❌" },
            项.说明.replace('|', "\\|")
        ));
    }
    if let Some((_, 文本)) = 日志
        && !文本.is_empty()
    {
        md.push_str("\n## 应用日志尾部\n\n```text\n");
        md.push_str(文本);
        md.push_str("\n```\n");
    }
    std::fs::write(输出目录.join(format!("{名称}.report.md")), md)
        .map_err(|错误| format!("写 Markdown 报告失败：{错误}"))?;

    Ok(())
}

/// 采集失败时也写一份报告：让"采集失败"留下可查的证据，而不是只打印到终端。
fn 写失败报告(
    输出目录: &Path,
    名称: &str,
    png: &Path,
    json: &Path,
    日志尾部: &str,
    窗口信息: Option<&窗口信息>,
) -> Result<(), String> {
    let 报告 = serde_json::json!({
        "verdict": "capture_failed",
        "exit_code": 3,
        "png": png.display().to_string(),
        "png_exists": png.is_file(),
        "evidence": json.display().to_string(),
        "evidence_exists": json.is_file(),
        "window": 窗口信息.map(|信息| serde_json::json!({
            "title": 信息.标题,
            "class": 信息.类名,
            "visible": 信息.可见,
            "client": [信息.客户区.0, 信息.客户区.1],
            "dpi": 信息.dpi,
        })),
        "app_log_tail": 日志尾部,
    });
    std::fs::write(
        输出目录.join(format!("{名称}.report.json")),
        serde_json::to_string_pretty(&报告).map_err(|错误| 错误.to_string())?,
    )
    .map_err(|错误| format!("写失败报告失败：{错误}"))?;
    Ok(())
}

// ===========================================================================
// drag：真实鼠标拖拽 + 双帧证据
// ===========================================================================

/// 一次会话里**单个注入阶段**的中间态记录。
///
/// # 为什么需要"逐阶段"而不是只有前后两帧
///
/// 一次会话最多有五个注入阶段：插槽 1（点击/拖拽）、右键、滚轮、
/// 第二次点击、插槽 2。改造前报告里只有"前后两帧 + 一个总判定"：
/// 一旦失败，无法说明是**哪个阶段**没生效 —— 是右键没送达？是滚轮没滚动？
/// 还是滚动生效了但点击用了过期的坐标？只能重跑并逐个删步骤去试。
///
/// 下面的记录让每个阶段的落点、几何来源（取自哪一帧证据）、注入结果都留痕，
/// 于是"滚轮生效、但插槽 2 的落点在视口外"这类中间态可以直接从报告里读出来，
/// 而不必再靠推断。
#[derive(Debug, Clone)]
struct 阶段记录 {
    /// 阶段名：`slot1` / `right_click` / `scroll` / `slot2` / `case2`。
    阶段: &'static str,
    /// 动作种类：`click` / `drag` / `right_click` / `wheel` / `skip`。
    动作: &'static str,
    /// 触发它的用例名（`arrow_name:乔木` 之类）；纯坐标会话为 `None`。
    用例: Option<String>,
    /// 逻辑客户区坐标。
    逻辑: Option<(f32, f32)>,
    /// 屏幕物理坐标。
    屏幕: Option<(i32, i32)>,
    /// 坐标取自哪一份证据：`before` / `after`。
    ///
    /// "用滚动前的几何去点滚动后的界面"是这类验收最容易犯的错，
    /// 单独记下来才能一眼看出坐标是否过期。
    几何来源: Option<&'static str>,
    /// 注入是否完整（按下/移动/抬起都成功）。没有点击的阶段为 `None`。
    注入完整: Option<bool>,
    /// 该阶段的补充说明（跳过原因、边界检查结论等）。
    备注: Option<String>,
}

impl 阶段记录 {
    fn 新(阶段: &'static str, 动作: &'static str) -> Self {
        Self {
            阶段,
            动作,
            用例: None,
            逻辑: None,
            屏幕: None,
            几何来源: None,
            注入完整: None,
            备注: None,
        }
    }

    fn 用例(mut self, 用例: &str) -> Self {
        self.用例 = Some(用例.to_string());
        self
    }

    fn 落点(mut self, 逻辑: (f32, f32), 屏幕: (i32, i32)) -> Self {
        self.逻辑 = Some(逻辑);
        self.屏幕 = Some(屏幕);
        self
    }

    fn 来自(mut self, 来源: &'static str) -> Self {
        self.几何来源 = Some(来源);
        self
    }

    fn 注入(mut self, 完整: bool) -> Self {
        self.注入完整 = Some(完整);
        self
    }

    fn 备注(mut self, 备注: impl Into<String>) -> Self {
        self.备注 = Some(备注.into());
        self
    }

    fn 到json(&self) -> serde_json::Value {
        serde_json::json!({
            "stage": self.阶段,
            "action": self.动作,
            "case": self.用例,
            "logical": self.逻辑.map(|(x, y)| [x, y]),
            "screen": self.屏幕.map(|(x, y)| [x, y]),
            "geometry_from": self.几何来源,
            "input_complete": self.注入完整,
            "note": self.备注,
        })
    }
}

/// 场景树右键菜单在证据里的 `kind` 代号 ↔ 命令行可写名字。
fn 菜单期望代号(名: &str) -> Option<&'static str> {
    match 名 {
        "row" | "scene_row" => Some("scene_row"),
        "blank" | "scene_blank" => Some("scene_blank"),
        "none" => Some("none"),
        _ => None,
    }
}

/// 读取证据里"本帧弹出的场景树右键菜单"。
///
/// 返回 `(kind, target)`；证据里没有这个字段（老版本证据）或没有菜单时为 `None`。
fn 证据菜单(证据: &serde_json::Value) -> Option<(String, Option<String>)> {
    let 菜单 = 证据.get("context_menu")?;
    if 菜单.is_null() {
        return None;
    }
    let kind = 菜单.get("kind")?.as_str()?.to_string();
    let target = 菜单
        .get("target")
        .and_then(|值| 值.as_str())
        .map(|值| 值.to_string());
    Some((kind, target))
}

/// 断言"右键之后菜单处于期望状态"。
///
/// 返回 `Err` 时**必须**是界面缺陷（退出码 2）：菜单是这次注入的直接结果，
/// 注入本身是否送达由 `注入.完整()` 与指针诊断单独保证。
fn 判定菜单(证据: &serde_json::Value, 期望: Option<&str>) -> Result<(), String> {
    let Some(期望原文) = 期望 else {
        return Ok(());
    };
    let Some(期望代号) = 菜单期望代号(期望原文) else {
        return Err(format!(
            "无法识别的 --expect-menu 取值 `{期望原文}`（可用 row / blank / none）"
        ));
    };
    let 实际 = 证据菜单(证据);
    if 期望代号 == "none" {
        return match 实际 {
            None => Ok(()),
            Some((kind, target)) => Err(format!(
                "期望没有右键菜单，但证据里菜单是打开的：kind={kind}，target={target:?}"
            )),
        };
    }
    match 实际 {
        None => Err(format!(
            "期望右键之后弹出 `{期望代号}` 菜单，但证据里 context_menu 为 null\
             （菜单没有打开，或这一帧恰好已经关闭）"
        )),
        Some((实际代号, 目标)) => {
            if 实际代号 != 期望代号 {
                return Err(format!(
                    "期望右键之后弹出 `{期望代号}` 菜单，实际弹出的是 `{实际代号}`\
                     （target={目标:?}）"
                ));
            }
            Ok(())
        }
    }
}

/// 断言"某个场景树行处于选中状态"。
///
/// 选中集是"点这一行"的直接结果，改造前完全不在证据里，因此这条断言
/// 之前根本无法表达 —— 只能靠人眼看截图里的高亮。
fn 判定选中(证据: &serde_json::Value, 期望名称: Option<&str>) -> Result<(), String> {
    let Some(名称) = 期望名称 else {
        return Ok(());
    };
    let 选中 = 证据
        .get("selection")
        .ok_or_else(|| "证据里没有 selection 字段（应用版本过旧？）".to_string())?;
    let ids: Vec<&str> = 选中
        .get("ids")
        .and_then(|值| 值.as_array())
        .map(|值| 值.iter().filter_map(|项| 项.as_str()).collect())
        .unwrap_or_default();
    let 行id = 证据
        .get("scene_rows")
        .and_then(|值| 值.as_array())
        .and_then(|行列表| {
            行列表
                .iter()
                .find(|行| 行.get("name").and_then(|值| 值.as_str()) == Some(名称))
                .and_then(|行| 行.get("id").and_then(|值| 值.as_str()))
        })
        .ok_or_else(|| format!("证据里找不到名为 `{名称}` 的场景树行，无法核对选中状态"))?;
    if ids.contains(&行id) {
        return Ok(());
    }
    Err(format!(
        "期望 `{名称}`（id={行id}）被选中，实际选中集是 {ids:?}"
    ))
}

/// 拖拽验收的判定结果（退出码与既有约定一致）。
fn 拖拽(参数: &[String]) -> i32 {
    let mut exe: Option<PathBuf> = None;
    let mut 输出目录 = PathBuf::from(".verify/runs");
    let mut 名称 = "drag".to_string();
    let mut 超时秒 = 180u64;
    let mut 第一帧 = 90u32;
    let mut 第二帧: Option<u32> = None;
    let mut 从: Option<(f32, f32)> = None;
    let mut 到: Option<(f32, f32)> = None;
    let mut 分割线: Option<String> = None;
    let mut 期望区域: Option<String> = None;
    let mut 期望不变 = false;
    let mut 空跑 = false;
    let mut 用例: Option<String> = None;
    let mut 窗口尺寸: Option<String> = None;
    let mut 环境变量: Vec<(String, String)> = Vec::new();
    let mut 期望状态变化: Vec<String> = Vec::new();
    let mut 用例2: Option<String> = None;
    let mut 用例2延迟毫秒: u64 = 1500;
    let mut 期望还原 = false;
    // 只验滚轮是否真的改变了滚动位置（不要求区域/状态发生变化）。
    let mut 期望滚动 = false;
    // 插槽 1 里的滚轮：在场景树等可滚动区域上真实发滚轮事件。
    // 插槽 1 的右键（用于验证右键菜单这类浮层）。
    let mut 右键用例: Option<String> = None;
    // 只做视觉取证、不判定"有没有变化"（菜单这类浮层不改变任何记录值）。
    let mut 允许无变化 = false;
    let mut 滚轮格数: i32 = 0;
    let mut 滚轮区域: Option<String> = None;
    // 插槽 2 里的点击（可多个，逗号分隔）：坐标取自**第二帧证据**，
    // 因此适用于"滚动之后"的几何。
    let mut 用例3: Vec<String> = Vec::new();
    let mut 第三帧帧号: Option<u32> = None;
    // 右键之后的菜单状态断言：`row` / `blank` / `none`。
    //
    // 菜单是浮层，不改变任何矩形，因此在加入这条路之前，"右键菜单弹没弹"
    // 只能靠人眼看截图。现在它是一条真正会让进程非零退出的断言。
    let mut 期望菜单: Option<String> = None;
    // 点击之后的选中状态断言：给出场景树里的**行名称**。
    let mut 期望选中: Option<String> = None;
    // 逐阶段中间态记录：每个注入阶段追加一条，最后写进报告。
    let mut 阶段记录表: Vec<阶段记录> = Vec::new();
    // ---- 变换工具 / 操纵器相关的断言（本轮新增）----
    let mut 期望工具: Option<String> = None;
    let mut 期望悬停: Option<String> = None;
    let mut 期望变换变化: Vec<String> = Vec::new();
    let mut 期望变换不变: Vec<String> = Vec::new();
    let mut 期望撤销增量: Option<i64> = None;
    let mut 期望脏状态: Option<String> = None;
    // 在第一帧之后、第二帧之前按一次键（`Escape` 用于验证取消）。
    let mut 按键: Option<String> = None;

    let mut 下标 = 1;
    while 下标 < 参数.len() {
        let 键 = 参数[下标].as_str();
        let 取值 = |下标: &mut usize| -> Option<String> {
            *下标 += 1;
            参数.get(*下标).cloned()
        };
        match 键 {
            "--exe" => exe = 取值(&mut 下标).map(PathBuf::from),
            "--out" => {
                if let Some(值) = 取值(&mut 下标) {
                    输出目录 = PathBuf::from(值);
                }
            }
            "--name" => {
                if let Some(值) = 取值(&mut 下标) {
                    名称 = 值;
                }
            }
            "--timeout" => {
                if let Some(值) = 取值(&mut 下标).and_then(|值| 值.parse().ok()) {
                    超时秒 = 值;
                }
            }
            "--frame" => {
                if let Some(值) = 取值(&mut 下标).and_then(|值| 值.parse().ok()) {
                    第一帧 = 值;
                }
            }
            "--frame2" => {
                第二帧 = 取值(&mut 下标).and_then(|值| 值.parse().ok());
            }
            "--from" => 从 = 取值(&mut 下标).and_then(|值| 解析点(&值)),
            "--to" => 到 = 取值(&mut 下标).and_then(|值| 解析点(&值)),
            "--splitter" => 分割线 = 取值(&mut 下标),
            "--expect-region" => 期望区域 = 取值(&mut 下标),
            "--expect-no-change" => 期望不变 = true,
            // 两次点击后状态应当**回到起点**（折叠再展开、运行再停止）。
            "--expect-restore" => 期望还原 = true,
            // 只要求"滚轮真的改变了滚动位置"，用于单独的滚动验证会话。
            "--expect-scroll" => 期望滚动 = true,
            "--dry-run" => 空跑 = true,
            "--case" => 用例 = 取值(&mut 下标),
            // 第二次点击：发生在**第一帧证据之后、第二帧证据之前**。
            //
            // 这是"启动 → 停止 → 重跑"这类多步操作的最小可行表达：
            // 第一次点击让状态从 A 变 B（第一帧证据拍到 A，第二帧拍到 B），
            // 第二次点击让状态从 B 变 C。没有它就只能一个会话验一步，
            // 而"停止一个正在跑的进程"必须在一个会话里完成。
            "--case2" => 用例2 = 取值(&mut 下标),
            "--case2-delay" => {
                if let Some(值) = 取值(&mut 下标).and_then(|值| 值.parse().ok()) {
                    用例2延迟毫秒 = 值;
                }
            }
            "--scroll" => {
                if let Some(值) = 取值(&mut 下标).and_then(|值| 值.trim().parse().ok()) {
                    滚轮格数 = 值;
                }
            }
            "--scroll-at" => 滚轮区域 = 取值(&mut 下标),
            "--right-click" => 右键用例 = 取值(&mut 下标),
            "--allow-no-change" => 允许无变化 = true,
            "--case3" => {
                if let Some(值) = 取值(&mut 下标) {
                    for 段 in 值.split(',') {
                        let 段 = 段.trim();
                        if !段.is_empty() {
                            用例3.push(段.to_string());
                        }
                    }
                }
            }
            "--frame3" => 第三帧帧号 = 取值(&mut 下标).and_then(|值| 值.parse().ok()),
            "--expect-menu" => 期望菜单 = 取值(&mut 下标),
            "--expect-selection" => 期望选中 = 取值(&mut 下标),
            "--expect-tool" => 期望工具 = 取值(&mut 下标),
            "--expect-gizmo" => 期望悬停 = 取值(&mut 下标),
            "--expect-transform" => {
                if let Some(值) = 取值(&mut 下标) {
                    期望变换变化.push(值);
                }
            }
            "--expect-transform-unchanged" => {
                if let Some(值) = 取值(&mut 下标) {
                    期望变换不变.push(值);
                }
            }
            "--expect-undo-delta" => {
                期望撤销增量 = 取值(&mut 下标).and_then(|值| 值.trim().parse().ok());
            }
            "--expect-dirty" => 期望脏状态 = 取值(&mut 下标),
            "--press-key" => 按键 = 取值(&mut 下标),
            "--window" => 窗口尺寸 = 取值(&mut 下标),
            "--expect-state" => {
                if let Some(值) = 取值(&mut 下标) {
                    期望状态变化.push(值);
                }
            }
            "--env" => {
                if let Some(值) = 取值(&mut 下标)
                    && let Some((键, 值)) = 值.split_once('=')
                {
                    环境变量.push((键.to_string(), 值.to_string()));
                }
            }
            _ => {}
        }
        下标 += 1;
    }
    let Some(exe) = exe else {
        eprintln!("drag 需要 --exe");
        return 4;
    };
    if !exe.is_file() {
        eprintln!("可执行文件不存在：{}", exe.display());
        return 4;
    }
    if 从.is_none()
        && 分割线.is_none()
        && 用例.is_none()
        && 用例3.is_empty()
        && 滚轮格数 == 0
        && 右键用例.is_none()
    {
        eprintln!(
            "drag 需要 --from/--to、--splitter v|h、--case <name>、--case3 <name[,name]>、--scroll <n> 或 --right-click <name>"
        );
        return 4;
    }
    if !用例3.is_empty() && 第三帧帧号.is_none() {
        eprintln!("--case3 需要同时给 --frame3 <帧号>（点击发生在第二帧之后、第三帧之前）");
        return 4;
    }
    if 滚轮格数 != 0 && 滚轮区域.is_none() {
        eprintln!("--scroll 需要同时给 --scroll-at <区域 id>（在该区域中心发滚轮）");
        return 4;
    }
    if let Err(错误) = std::fs::create_dir_all(&输出目录) {
        eprintln!("无法创建输出目录：{错误}");
        return 4;
    }
    let 第二帧 = 第二帧.unwrap_or(第一帧 + 120);
    let 第三帧 = 第三帧帧号.unwrap_or(第二帧 + 120);
    let 前图 = 输出目录.join(format!("{名称}_before.png"));
    let 前证 = 输出目录.join(format!("{名称}_before.json"));
    let 后图 = 输出目录.join(format!("{名称}_after.png"));
    let 后证 = 输出目录.join(format!("{名称}_after.json"));
    let 末图 = 输出目录.join(format!("{名称}_final.png"));
    let 末证 = 输出目录.join(format!("{名称}_final.json"));
    let 日志 = 输出目录.join(format!("{名称}.drag.log"));
    for 路径 in [&前图, &前证, &后图, &后证, &末图, &末证] {
        let _ = std::fs::remove_file(路径);
    }

    let mut 命令 = Command::new(&exe);
    命令
        .env("BEVY_STUDIO_CAPTURE_PNG", &前图)
        .env("BEVY_STUDIO_CAPTURE_JSON", &前证)
        .env("BEVY_STUDIO_CAPTURE_FRAME", 第一帧.to_string())
        .env("BEVY_STUDIO_CAPTURE_PNG2", &后图)
        .env("BEVY_STUDIO_CAPTURE_JSON2", &后证)
        .env("BEVY_STUDIO_CAPTURE_FRAME2", 第二帧.to_string())
        .env("BEVY_STUDIO_CAPTURE_EXIT", "1")
        .env("BEVY_STUDIO_INPUT_DIAG", "1");
    // 第三帧只在需要时启用：不需要的话应用只等两帧就退出，不会多跑一倍时间。
    if 第三帧帧号.is_some() {
        命令
            .env("BEVY_STUDIO_CAPTURE_PNG3", &末图)
            .env("BEVY_STUDIO_CAPTURE_JSON3", &末证)
            .env("BEVY_STUDIO_CAPTURE_FRAME3", 第三帧.to_string());
    }
    if let Some(尺寸) = 窗口尺寸.as_deref() {
        命令.env("BEVY_STUDIO_WINDOW_SIZE", 尺寸);
    }
    for (键, 值) in &环境变量 {
        命令.env(键, 值);
    }
    let 日志文件 = match std::fs::File::create(&日志) {
        Ok(文件) => 文件,
        Err(错误) => {
            eprintln!("无法创建日志：{错误}");
            return 4;
        }
    };
    let Ok(日志副本) = 日志文件.try_clone() else {
        return 4;
    };
    命令
        .stdout(Stdio::from(日志文件))
        .stderr(Stdio::from(日志副本));
    let mut 子进程 = match 命令.spawn() {
        Ok(子进程) => 子进程,
        Err(错误) => {
            eprintln!("无法启动应用：{错误}");
            return 3;
        }
    };
    let 进程号 = 子进程.id();
    let 开始 = Instant::now();
    let 超时 = Duration::from_secs(超时秒);

    // 等第一帧 + 窗口
    let mut 窗口: Option<窗口信息> = None;
    let mut 第一帧就绪 = false;
    loop {
        if let Some(候选) = window_probe::查找(进程号)
            && 更优窗口(&候选, 窗口.as_ref())
        {
            窗口 = Some(候选);
        }
        if 前图.is_file() && 前证.is_file() && 窗口.is_some() {
            第一帧就绪 = true;
            break;
        }
        if let Ok(Some(_)) = 子进程.try_wait() {
            break;
        }
        if 开始.elapsed() > 超时 {
            break;
        }
        std::thread::sleep(Duration::from_millis(80));
    }

    let Ok(前证据) = 读取证据(&前证) else {
        eprintln!("采集失败：第一帧证据不存在或不可读");
        等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第一帧证据不可读", 3);
    };
    if !第一帧就绪 {
        等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第一帧采集超时", 3);
    }
    let Some(窗口) = 窗口 else {
        等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "无法识别目标窗口", 3);
    };

    let ppp = 前证据
        .get("pixels_per_point")
        .and_then(|值| 值.as_f64())
        .unwrap_or(1.0) as f32;
    let 原点 = 窗口.客户区原点;
    // 逻辑客户区坐标 → 屏幕物理坐标
    let 转屏幕 = |逻辑: (f32, f32)| -> (i32, i32) {
        (
            原点.0 + (逻辑.0 * ppp).round() as i32,
            原点.1 + (逻辑.1 * ppp).round() as i32,
        )
    };

    let (从逻辑, 到逻辑) = match (从, 到, 分割线.as_deref(), 用例.as_deref()) {
        (Some(起点), Some(终点), _, _) => (起点, 终点),
        (_, _, Some(方向), _) => match 分割线坐标(&前证据, 方向) {
            Some(坐标) => 坐标,
            None => {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "capture_failed",
                    "证据里没有视口区域",
                    3,
                );
            }
        },
        (_, _, _, Some(名)) => match 用例坐标(名, &前证据) {
            Some(坐标) => 坐标,
            None => {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "capture_failed",
                    "用例需要的区域不存在",
                    3,
                );
            }
        },
        _ => ((0.0, 0.0), (0.0, 0.0)),
    };
    // 第一插槽是否需要点击：只有显式给了目标才注入。
    //
    // 否则 `--scroll` / `--case3` 这类会话会在窗口左上角（菜单栏）多出一次
    // 真实单击 —— 它既是多余输入，也可能误触菜单把后续步骤搞乱。
    let 需要第一插槽点击 = 从.is_some() || 分割线.is_some() || 用例.is_some();

    // 聚焦目标窗口：不聚焦时 SendInput 的点击会落到别的窗口上。
    //
    // `SetForegroundWindow` 在 Windows 上可能被前台锁拒绝（返回 0）。
    // 这时点一下**标题栏**：它不进入客户区，不会误触 UI，但足以把窗口激活。
    确保前台(&窗口);
    std::thread::sleep(Duration::from_millis(250));

    let 起点屏幕 = 转屏幕(从逻辑);
    let 终点屏幕 = 转屏幕(到逻辑);

    let mut 注入 = input::注入结果 {
        移动成功: true,
        按下成功: true,
        抬起成功: true,
        移动步数: 0,
    };
    if !需要第一插槽点击 {
        // 本次会话只做滚轮 / 插槽 2：跳过第一插槽的越界检查与点击注入。
        eprintln!("第一插槽：未指定点击目标，跳过点击注入");
        阶段记录表.push(
            阶段记录::新("slot1", "skip")
                .备注("未指定点击目标（--case / --from / --splitter 都没给）"),
        );
    } else {
        // 落点必须在**逻辑客户区**内。
        //
        // 场景树会滚动：滚出视口的行，其矩形仍然被应用记录下来
        // （`ScrollArea` 会把所有行都布局一遍再裁掉），但坐标可能已经落在窗口外。
        // 那时候注入的点击会打到桌面或其他窗口上，表现为"输入未生效" ——
        // 而这个结论是**错的**。这里直接拦下并说清楚，
        // 避免把"工具点了越界坐标"当成界面缺陷。
        let 客户宽 = 前证据
            .get("window")
            .and_then(|值| 值.get("width"))
            .and_then(|值| 值.as_f64())
            .unwrap_or(0.0) as f32
            / ppp;
        let 客户高 = 前证据
            .get("window")
            .and_then(|值| 值.get("height"))
            .and_then(|值| 值.as_f64())
            .unwrap_or(0.0) as f32
            / ppp;
        if 客户宽 > 1.0
            && 客户高 > 1.0
            && (从逻辑.0 < 0.0 || 从逻辑.1 < 0.0 || 从逻辑.0 > 客户宽 || 从逻辑.1 > 客户高)
        {
            等待退出(&mut 子进程, Duration::from_secs(5));
            return 拖拽失败报告(
                &输出目录,
                &名称,
                "target_out_of_view",
                &format!(
                    "落点 {从逻辑:?} 不在逻辑客户区 {客户宽:.0}×{客户高:.0} 内：\
                 目标被滚动到视口之外（需要先滚动，或换一个可见目标）"
                ),
                3,
            );
        }

        // 起点与终点重合 = 这是一次**单击**，不是拖拽。
        //
        // 二者必须走不同的注入路径：拖拽流程按下约 1.0 秒，
        // 超过 egui 的 `max_click_duration`（默认 0.8s），
        // 于是 `Response::clicked()` 永远是 false —— 坐标对、指针也到了，
        // 但按钮"没反应"。这正是上一轮 Run 按钮被误判为缺陷的原因。
        let 是单击 = (起点屏幕.0 - 终点屏幕.0).abs() <= 1 && (起点屏幕.1 - 终点屏幕.1).abs() <= 1;
        eprintln!(
            "注入{}：逻辑 {:?}→{:?}，屏幕 {:?}→{:?}（ppp={ppp:.2}，客户区原点={原点:?}）",
            if 是单击 { "单击" } else { "拖拽" },
            从逻辑,
            到逻辑,
            起点屏幕,
            终点屏幕
        );
        注入 = if 空跑 {
            // 负向对照：完全不动鼠标，用来证明工具会把"没有变化"判为输入未生效，
            // 而不是把界面判成坏的。
            input::注入结果 {
                移动成功: true,
                按下成功: true,
                抬起成功: true,
                移动步数: 0,
            }
        } else if 是单击 {
            input::执行单击(起点屏幕)
        } else {
            input::执行拖拽(input::拖拽 {
                从: 起点屏幕,
                到: 终点屏幕,
                步数: 24,
                步间隔: Duration::from_millis(28),
                预热: !空跑,
            })
        };
        let mut 记录 = 阶段记录::新("slot1", if 是单击 { "click" } else { "drag" })
            .落点(从逻辑, 起点屏幕)
            .来自("before")
            .注入(注入.完整())
            .备注(format!("终点 逻辑 {到逻辑:?} 屏幕 {终点屏幕:?}"));
        if let Some(名) = 用例.as_deref() {
            记录 = 记录.用例(名);
        }
        if 空跑 {
            记录 = 记录.备注("空跑：刻意不移动鼠标（负向对照）");
        }
        阶段记录表.push(记录);
    } // if !需要第一插槽点击

    // ---- 右键（可选）：第一帧证据之后、第二帧证据之前 ----
    //
    // 坐标同样来自第一帧证据：点的就是界面上那个真实控件。
    if let Some(名) = 右键用例.clone() {
        match 用例坐标(&名, &前证据) {
            Some((从右, _)) => {
                let 屏右 = 转屏幕(从右);
                eprintln!("右键单击 `{名}`：逻辑 {从右:?}，屏幕 {屏右:?}");
                确保前台(&窗口);
                std::thread::sleep(Duration::from_millis(150));
                let _ = input::执行右键单击(屏右);
                阶段记录表.push(
                    阶段记录::新("right_click", "right_click")
                        .用例(&名)
                        .落点(从右, 屏右)
                        .来自("before"),
                );
                std::thread::sleep(Duration::from_millis(400));
            }
            None => {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "capture_failed",
                    &format!("右键用例 `{名}` 在第一帧证据里取不到坐标"),
                    3,
                );
            }
        }
    }

    // ---- 按键（可选）：第一帧证据之后、第二帧证据之前 ----
    if let Some(键名) = 按键.clone() {
        let 虚拟键 = match 键名.as_str() {
            "Escape" | "esc" => Some(0x1Bu16),
            "Delete" | "del" => Some(0x2Eu16),
            "F" | "f" => Some(0x46u16),
            _ => None,
        };
        let Some(虚拟键) = 虚拟键 else {
            等待退出(&mut 子进程, Duration::from_secs(5));
            return 拖拽失败报告(
                &输出目录,
                &名称,
                "capture_failed",
                &format!("不支持的 --press-key 取值 `{键名}`（可用 Escape/Delete/F）"),
                3,
            );
        };
        确保前台(&窗口);
        std::thread::sleep(Duration::from_millis(150));
        let 送达 = input::执行按键(虚拟键);
        阶段记录表.push(
            阶段记录::新("press_key", "key")
                .用例(&键名)
                .注入(送达)
                .备注("真实键盘事件"),
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    // ---- 滚轮（可选）：同样发生在第一帧证据之后、第二帧证据之前 ----
    //
    // 它的意义是让**第二帧证据**拍到"滚动之后"的真实几何。
    // 后续 `--case3` 就用这份几何去点击，因此不需要任何预设坐标。
    if 滚轮格数 != 0 {
        let 区域 = 滚轮区域.clone().unwrap_or_else(|| "scene_tree".to_string());
        let ppp = 前证据
            .get("pixels_per_point")
            .and_then(|值| 值.as_f64())
            .unwrap_or(1.0) as f32;
        let 表 = 区域表(&前证据);
        let Some((x, y, w, h)) = 表.get(&区域).copied() else {
            等待退出(&mut 子进程, Duration::from_secs(5));
            return 拖拽失败报告(
                &输出目录,
                &名称,
                "capture_failed",
                &format!("滚轮区域 `{区域}` 不在第一帧证据里"),
                3,
            );
        };
        // 滚轮要落在区域**内部**：区域矩形是逻辑像素，这里取中心偏上一点，
        // 避开区域自身的标题/搜索栏等不可滚动部分。
        let 逻辑点 = (x + w * 0.5, y + h * 0.45);
        let 屏幕点 = (
            原点.0 + (逻辑点.0 * ppp).round() as i32,
            原点.1 + (逻辑点.1 * ppp).round() as i32,
        );
        eprintln!("滚轮：区域 `{区域}` 逻辑 {逻辑点:?} → 屏幕 {屏幕点:?}，格数 {滚轮格数}");
        确保前台(&窗口);
        std::thread::sleep(Duration::from_millis(150));
        let 滚动成功 = input::滚动(屏幕点, 滚轮格数);
        eprintln!("滚轮注入结果：{滚动成功}");
        阶段记录表.push(
            阶段记录::新("scroll", "wheel")
                .用例(&区域)
                .落点(逻辑点, 屏幕点)
                .来自("before")
                .注入(滚动成功)
                .备注(format!("{滚轮格数} 格")),
        );
        // 给 egui 的平滑滚动留出收敛时间，避免第二帧拍到中间态。
        std::thread::sleep(Duration::from_millis(700));
    }

    // ---- 第二次点击（可选）：发生在第一帧证据之后、第二帧证据之前 ----
    //
    // 用它可以在**同一个会话**里验「启动 → 停止」或「启动 → 重跑」：
    // 第一帧证据拍到第一次点击前的状态，第二帧拍到第二次点击后的状态。
    // 没有它就只能一个会话验一步，而"停止一个正在跑的进程"必须先让它跑起来。
    if let Some(名) = 用例2.clone() {
        std::thread::sleep(Duration::from_millis(用例2延迟毫秒));
        match 用例坐标(&名, &前证据) {
            Some((从2, _到2)) => {
                let 屏2 = 转屏幕(从2);
                eprintln!("第二次单击 `{名}`：逻辑 {从2:?}，屏幕 {屏2:?}");
                确保前台(&窗口);
                std::thread::sleep(Duration::from_millis(120));
                let _ = input::执行单击(屏2);
                阶段记录表.push(
                    阶段记录::新("case2", "click")
                        .用例(&名)
                        .落点(从2, 屏2)
                        .来自("before"),
                );
            }
            None => {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "capture_failed",
                    &format!("第二次点击的用例 `{名}` 在第一帧证据里取不到坐标"),
                    3,
                );
            }
        }
    }

    // 等第二帧
    let 二开始 = Instant::now();
    let mut 第二帧就绪 = false;
    loop {
        if 后图.is_file() && 后证.is_file() {
            第二帧就绪 = true;
            break;
        }
        if let Ok(Some(_)) = 子进程.try_wait() {
            if 后图.is_file() && 后证.is_file() {
                第二帧就绪 = true;
            }
            break;
        }
        if 二开始.elapsed() > 超时 {
            break;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    if !第二帧就绪 {
        等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第二帧采集失败", 3);
    }
    let Ok(后证据) = 读取证据(&后证) else {
        等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第二帧证据不可读", 3);
    };
    if !注入.完整() {
        等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(
            &输出目录,
            &名称,
            "input_failed",
            &format!("SendInput 阶段失败：{}", 注入.摘要()),
            3,
        );
    }

    // ---- 插槽 2（可选）：第二帧证据之后、第三帧证据之前 ----
    //
    // 关键点：坐标一律取自**第二帧证据**。滚动之后的真实矩形只存在于
    // 这一份证据里，用它点击才谈得上"没有猜坐标"。
    let mut 末证据 = 后证据.clone();
    let mut 第三帧就绪 = false;
    // 插槽 2 每次点击的实际落点：报告里要能证明"用的是滚动后重新测量的矩形"。
    let mut 插槽2点击: Vec<serde_json::Value> = Vec::new();
    if !用例3.is_empty() {
        let ppp3 = 后证据
            .get("pixels_per_point")
            .and_then(|值| 值.as_f64())
            .unwrap_or(1.0) as f32;
        let 转屏幕3 = |逻辑: (f32, f32)| -> (i32, i32) {
            (
                原点.0 + (逻辑.0 * ppp3).round() as i32,
                原点.1 + (逻辑.1 * ppp3).round() as i32,
            )
        };
        确保前台(&窗口);
        std::thread::sleep(Duration::from_millis(150));
        for 名 in &用例3 {
            // ---- 组合键动作：`press:ctrlz` / `press:ctrly` / `press:ctrls` / `press:escape` ----
            //
            // 撤销 / 重做 / 保存都是**键盘**入口，必须用真实按键验证：
            // 直接调撤销函数测不出"快捷键有没有接上"，也测不出
            // "一次拖拽之后按一次 Ctrl+Z 是否恰好退回起点"。
            if let Some(键名) = 名.strip_prefix("press:") {
                let 结果 = match 键名 {
                    // VK_CONTROL=0x11, Z=0x5A, Y=0x59, S=0x53
                    "ctrlz" => input::执行组合键(0x11, 0x5A),
                    "ctrly" => input::执行组合键(0x11, 0x59),
                    "ctrls" => input::执行组合键(0x11, 0x53),
                    // VK_ESCAPE=0x1B
                    "escape" => input::执行按键(0x1B),
                    _ => false,
                };
                eprintln!("插槽 2 组合键 `{名}`：送达={结果}");
                插槽2点击.push(serde_json::json!({
                    "case": 名,
                    "kind": "key",
                    "delivered": 结果,
                }));
                阶段记录表.push(
                    阶段记录::新("slot2", "key")
                        .用例(名)
                        .注入(结果)
                        .备注("真实键盘组合键"),
                );
                continue;
            }
            // `gizmo-cancel:` 复用 `gizmo-drag:` 的坐标推导，只是注入序列不同
            // （中途按 Escape 再松手）。这里先换名字解析坐标。
            let 坐标名 = if 名.starts_with("gizmo-cancel:") {
                名.replacen("gizmo-cancel:", "gizmo-drag:", 1)
            } else {
                名.clone()
            };
            let Some((从3, 到3)) = 用例坐标(&坐标名, &后证据) else {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "capture_failed",
                    &format!("插槽 2 的用例 `{名}` 在第二帧证据里取不到坐标"),
                    3,
                );
            };
            // 落点必须仍在这个窗口的客户区内；滚出视口的行会给出明确报告，
            // 而不是让点击打到桌面上再被误判成"界面没反应"。
            let 客户宽3 = 后证据
                .get("window")
                .and_then(|值| 值.get("width"))
                .and_then(|值| 值.as_f64())
                .unwrap_or(0.0) as f32
                / ppp3;
            let 客户高3 = 后证据
                .get("window")
                .and_then(|值| 值.get("height"))
                .and_then(|值| 值.as_f64())
                .unwrap_or(0.0) as f32
                / ppp3;
            if 客户宽3 > 1.0
                && 客户高3 > 1.0
                && (从3.0 < 0.0 || 从3.1 < 0.0 || 从3.0 > 客户宽3 || 从3.1 > 客户高3)
            {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "target_out_of_view",
                    &format!(
                        "插槽 2 的用例 `{名}` 落点 {从3:?} 不在客户区 \
                         {客户宽3:.0}×{客户高3:.0} 内（滚动后仍不可见）"
                    ),
                    3,
                );
            }
            // 场景树用例还要校验"是否真的在滚动视口内"。
            // 滚出视口的行会被裁掉，点它只会落到别的面板上 —— 那既不是
            // 折叠失败，也不是成功，必须按"目标不可见"如实报告。
            if 是场景树用例(名)
                && let Some(在内) = 点落在场景视口内(&后证据, 从3)
                && !在内
                && let Some((vx, vy, vw, vh)) = 场景视口(&后证据)
            {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "target_out_of_view",
                    &format!(
                        "插槽 2 的用例 `{名}` 落点 {从3:?} 在场景树滚动视口 \
                         ({vx:.0},{vy:.0},{vw:.0}×{vh:.0}) 之外（滚动后仍不可见）"
                    ),
                    3,
                );
            }
            let 屏3 = 转屏幕3(从3);
            let 屏3终 = 转屏幕3(到3);
            // ---- 取消路径：按下 → 分段拖拽 → Escape → 抬起 ----
            //
            // 这是"拖拽中按 Escape 取消"的唯一真实验证方式：egui 读的是
            // `RawInput` 里的键盘事件，只有真实 `SendInput` 才能证明
            // "快捷键接上了、并且把变换恢复回去了"。
            if 名.starts_with("gizmo-cancel:") {
                let _ = input::移动到(屏3);
                std::thread::sleep(Duration::from_millis(120));
                let _ = input::按下();
                std::thread::sleep(Duration::from_millis(120));
                for 步 in 1..=12 {
                    let t = 步 as f32 / 12.0;
                    let 点 = (
                        屏3.0 + ((屏3终.0 - 屏3.0) as f32 * t) as i32,
                        屏3.1 + ((屏3终.1 - 屏3.1) as f32 * t) as i32,
                    );
                    let _ = input::移动到(点);
                    std::thread::sleep(Duration::from_millis(28));
                }
                // 0x1B = VK_ESCAPE
                let 取消送达 = input::执行按键(0x1B);
                let _ = input::抬起();
                std::thread::sleep(Duration::from_millis(250));
                eprintln!("插槽 2 拖拽后取消 `{名}`：逻辑 {从3:?}→{到3:?}，Escape 送达={取消送达}");
                插槽2点击.push(serde_json::json!({
                    "case": 名,
                    "kind": "drag_cancel",
                    "logical": [从3.0, 从3.1],
                    "logical_to": [到3.0, 到3.1],
                    "screen": [屏3.0, 屏3.1],
                    "screen_to": [屏3终.0, 屏3终.1],
                    "escape_delivered": 取消送达,
                    "geometry_from": "after",
                }));
                阶段记录表.push(
                    阶段记录::新("slot2", "drag_cancel")
                        .用例(名)
                        .落点(从3, 屏3)
                        .来自("after")
                        .注入(取消送达)
                        .备注("按下 → 分段拖拽 → Escape → 抬起"),
                );
                continue;
            }
            // 插槽 2 既支持"单击"也支持"拖拽"：起点与终点不同就是一次拖拽。
            //
            // 操纵器用例必须走拖拽（按下 → 分段移动 → 抬起），因为
            // 变换是在**按住期间**连续求解的；用单击去测操纵器只会得到
            // "点了没反应"。
            let 是拖拽 = 屏3 != 屏3终;
            eprintln!(
                "插槽 2 {} `{名}`：逻辑 {从3:?}→{到3:?}，屏幕 {屏3:?}→{屏3终:?}",
                if 是拖拽 { "拖拽" } else { "单击" }
            );
            插槽2点击.push(serde_json::json!({
                "case": 名,
                // 逻辑坐标来自第二帧证据（该帧测得的几何，不是写死的坐标）
                "logical": [从3.0, 从3.1],
                "logical_to": [到3.0, 到3.1],
                "screen": [屏3.0, 屏3.1],
                "screen_to": [屏3终.0, 屏3终.1],
                "kind": if 是拖拽 { "drag" } else { "click" },
                // 这份几何来自第二帧（选中之后、拖拽之前）
                "geometry_from": "after",
            }));
            if 是拖拽 {
                let _ = input::执行拖拽(input::拖拽 {
                    从: 屏3,
                    到: 屏3终,
                    步数: 24,
                    步间隔: Duration::from_millis(28),
                    预热: true,
                });
            } else {
                let _ = input::执行单击(屏3);
            }
            阶段记录表.push(
                阶段记录::新("slot2", if 是拖拽 { "drag" } else { "click" })
                    .用例(名)
                    .落点(从3, 屏3)
                    .来自("after")
                    .备注(format!("终点 逻辑 {到3:?} 屏幕 {屏3终:?}")),
            );
            std::thread::sleep(Duration::from_millis(450));
        }

        // 等第三帧
        let 三开始 = Instant::now();
        loop {
            if 末图.is_file() && 末证.is_file() {
                第三帧就绪 = true;
                break;
            }
            if let Ok(Some(_)) = 子进程.try_wait() {
                if 末图.is_file() && 末证.is_file() {
                    第三帧就绪 = true;
                }
                break;
            }
            if 三开始.elapsed() > 超时 {
                break;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
        if !第三帧就绪 {
            等待退出(&mut 子进程, Duration::from_secs(5));
            return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第三帧采集失败", 3);
        }
        match 读取证据(&末证) {
            Ok(值) => 末证据 = 值,
            Err(_) => {
                等待退出(&mut 子进程, Duration::from_secs(5));
                return 拖拽失败报告(
                    &输出目录,
                    &名称,
                    "capture_failed",
                    "第三帧证据不可读",
                    3,
                );
            }
        }
    }

    等待退出(&mut 子进程, Duration::from_secs(10));
    let 日志尾部 = 读取日志尾部(&日志, 80);

    // 滚动证据：同一行在滚动前后的 y 坐标必须不同，否则"滚轮没生效"，
    // 后面拿到的新矩形也就没有意义。
    let 滚动结果 = 比较滚动(&前证据, &后证据);
    if 滚轮格数 != 0 && !滚动结果.0 {
        eprintln!(
            "滚轮没有改变任何场景树行的位置（移位数 {}）：滚动未生效，按采集失败处理",
            滚动结果.1
        );
        return 拖拽失败报告(
            &输出目录,
            &名称,
            "scroll_not_effective",
            "滚轮事件已发送，但场景树的滚动位置没有变化",
            3,
        );
    }

    let 变化 = 比较区域(&前证据, &末证据);
    let 状态变化 = 比较状态(&前证据, &末证据);
    let 收到指针 = 日志尾部.contains("[输入诊断]");

    // 右键菜单与选中集是"本帧事实"：只对最后一帧（注入完成之后那一帧）判定。
    let 菜单判定 = 判定菜单(&末证据, 期望菜单.as_deref());
    let 选中判定 = 判定选中(&末证据, 期望选中.as_deref());
    // 变换相关的断言同样只看末帧状态，但"变化/未变化"要比前后两帧。
    //
    // 基准帧必须选**最后一次注入之前**的那一帧：有插槽 2 动作时是第二帧
    // （选中之后、拖拽之前），否则是第一帧。用第一帧当基准，会在
    // "先选中、后拖拽"的场景里拿 null 变换与最终值比较，断言永远失败 ——
    // 那不是界面缺陷，是选错了基准。
    let 基准证据: &serde_json::Value = if 用例3.is_empty() {
        &前证据
    } else {
        &后证据
    };
    let 工具判定 = 判定工具(&末证据, 期望工具.as_deref());
    let 悬停判定 = 判定悬停(&后证据, 期望悬停.as_deref());
    let 变换判定 = 判定变换分量(基准证据, &末证据, &期望变换变化, &期望变换不变);
    let 撤销判定 = match 期望撤销增量 {
        Some(期望) => 判定撤销步数(基准证据, &末证据, 期望),
        None => Ok(()),
    };
    let 脏判定 = 判定脏状态(&末证据, 期望脏状态.as_deref());
    let mut 断言通过 = true;
    for (名, 结果) in [
        ("菜单", &菜单判定),
        ("选中", &选中判定),
        ("工具", &工具判定),
        ("操纵器悬停", &悬停判定),
        ("变换分量", &变换判定),
        ("撤销步数", &撤销判定),
        ("文档脏状态", &脏判定),
    ] {
        if let Err(错误) = 结果 {
            eprintln!("{名}断言未通过：{错误}");
            断言通过 = false;
        }
    }

    写拖拽报告(
        &输出目录,
        &名称,
        &窗口,
        ppp,
        从逻辑,
        到逻辑,
        起点屏幕,
        终点屏幕,
        &注入,
        收到指针,
        &变化,
        &状态变化,
        &前证据,
        &末证据,
        基准证据,
        &期望区域,
        &期望状态变化,
        期望不变,
        期望还原,
        期望滚动,
        允许无变化,
        &右键用例,
        滚轮格数,
        滚动结果,
        &插槽2点击,
        &阶段记录表,
        &期望菜单,
        &期望选中,
        &菜单判定,
        &选中判定,
        &期望工具,
        &期望悬停,
        &期望变换变化,
        &期望变换不变,
        &期望撤销增量,
        &期望脏状态,
        &工具判定,
        &悬停判定,
        &变换判定,
        &撤销判定,
        &脏判定,
    );

    // 判定：布局区域变化**或**结构化状态变化都算"点击产生了效果"。
    //
    // 只比区域矩形是不够的：点「折叠箭头」不会移动任何面板，
    // 点「运行项目」也不会。真正的事实是场景树可见行/折叠计数、
    // 运行状态与 PID 的变化 —— 它们才是这两个功能的验收依据。
    let 满足 = if 允许无变化 {
        // 只做视觉取证：截图存在即算采集成功（指针是否到达单独记录）。
        收到指针 || !变化.is_empty() || !状态变化.is_empty()
    } else if 期望不变 {
        变化.is_empty() && 状态变化.is_empty()
    } else if 期望滚动 {
        // 滚轮会话：只要行位置真的变了就算通过。
        // 滚动不改变"哪些行存在"，所以不可能靠区域/状态指纹来判定。
        滚动结果.0 && 滚轮格数 != 0
    } else if 期望还原 {
        // 「两次点击后应当回到起点」：状态指纹必须完全一致。
        //
        // 注意这条判据单独使用没有意义（点击完全无效也会返回 true），
        // 因此要求**必须**配置了第二次点击，并且由同一轮的
        // "只点一次"会话证明第一次点击确实生效。
        状态变化.is_empty() && (用例2.is_some() || 用例3.len() >= 2)
    } else {
        !变化.is_empty() || !状态变化.is_empty()
    };
    let 期望命中 = 期望区域
        .as_deref()
        .map(|区域| 变化.iter().any(|项| 项 == 区域))
        .unwrap_or(true);
    let 状态命中 = 期望状态变化
        .iter()
        .all(|键| 状态变化.iter().any(|项| 项 == 键));

    if !满足 {
        eprintln!(
            "输入未产生任何变化（received_pointer={收到指针}，区域变化 {}，状态变化 {}）",
            变化.len(),
            状态变化.len()
        );
        // 指针事件确实到达 egui 但界面完全没变 → 这是真实 UI 缺陷；
        // 否则只能判定为输入/环境失败，不能冤枉界面。
        return if 收到指针 { 2 } else { 3 };
    }
    if !期望命中 {
        eprintln!("发生了变化，但期望区域 `{期望区域:?}` 未变化");
        return 2;
    }
    if !状态命中 {
        eprintln!("发生了变化，但期望的状态字段 {期望状态变化:?} 未变化（实际变化：{状态变化:?}）");
        return 2;
    }
    // 菜单 / 选中断言不看"有没有变化"，只看**末帧的真实状态**。
    //
    // 因此它们能抓住一类既有的漏判：右键弹出了菜单、但弹的是**另一种**
    // （行菜单 vs 空白菜单）；或者点到了行边缘、选择却落在另一行上。
    // 这类错误在"只看有没有变化"的判据下是"通过"的。
    if !断言通过 {
        return 2;
    }
    0
}

/// 取场景树**滚动视口**的逻辑矩形（`scene_view`）。
///
/// 老证据文件里没有这个字段时返回 `None`（此时退回"只查窗口"的旧行为）。
fn 场景视口(证据: &serde_json::Value) -> Option<(f32, f32, f32, f32)> {
    let ppp = 证据
        .get("pixels_per_point")
        .and_then(|值| 值.as_f64())
        .unwrap_or(1.0) as f32;
    let 视口 = 证据.get("scene_view")?;
    if 视口.is_null() {
        return None;
    }
    let 取 = |键: &str| 视口.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) as f32 / ppp;
    Some((取("x"), 取("y"), 取("w"), 取("h")))
}

/// 这个用例是不是"点场景树里的东西"。
///
/// 只有这类用例才需要做滚动视口校验：点工具栏按钮、面板标签时
/// `scene_view` 与落点无关。
fn 是场景树用例(名: &str) -> bool {
    名.starts_with("arrow:")
        || 名.starts_with("arrow_name:")
        || 名.starts_with("row_name:")
        || 名.starts_with("name:")
        || 名.starts_with("row_blank:")
        || 名 == "scene_blank"
}

/// 落点是否在场景树滚动视口内（含 1px 容差）。
///
/// # 为什么必须有这条检查
///
/// 滚出视口的行仍然会被布局并记录矩形，只是被 `ScrollArea` 裁掉。
/// 用它的中心去点击，点击会落到**别的面板**上，界面自然"没有反应"。
/// 若不拦下来，验收会把"工具点了视口外"当成"折叠功能坏了"，
/// 或者更糟：两次落空的点击会让 `--expect-restore` 误判通过。
fn 点落在场景视口内(证据: &serde_json::Value, 点: (f32, f32)) -> Option<bool> {
    let (x, y, w, h) = 场景视口(证据)?;
    Some(点.0 >= x - 1.0 && 点.0 <= x + w + 1.0 && 点.1 >= y - 1.0 && 点.1 <= y + h + 1.0)
}

/// 比较前后两帧里**同一行**的 y 坐标，判断滚动是否真的发生。
///
/// 返回 `(是否有行位移, 位移行数)`。
///
/// # 为什么必须比 y 而不是比"行列表"
///
/// 滚动不改变"哪些行存在"，只改变它们画在什么位置。因此
/// `scene_rows` 的 `深度:名称:展开` 指纹在滚动前后完全相同 ——
/// 只看它会把"确实滚了"误判成"什么都没发生"。行的 `row.y` 才是滚动的证据。
fn 比较滚动(前: &serde_json::Value, 后: &serde_json::Value) -> (bool, usize) {
    let 取表 = |证据: &serde_json::Value| -> std::collections::BTreeMap<String, f64> {
        let mut 表 = std::collections::BTreeMap::new();
        if let Some(行列表) = 证据.get("scene_rows").and_then(|值| 值.as_array()) {
            for 行 in 行列表 {
                let Some(名) = 行.get("name").and_then(|值| 值.as_str()) else {
                    continue;
                };
                let Some(y) = 行
                    .get("row")
                    .and_then(|值| 值.get("y"))
                    .and_then(|值| 值.as_f64())
                else {
                    continue;
                };
                表.insert(名.to_string(), y);
            }
        }
        表
    };
    let 前表 = 取表(前);
    let 后表 = 取表(后);
    let mut 位移行数 = 0;
    for (名, 前y) in &前表 {
        if let Some(后y) = 后表.get(名)
            && (后y - 前y).abs() > 1.0
        {
            位移行数 += 1;
        }
    }
    let 有新增 = 后表.keys().any(|名| !前表.contains_key(名));
    (位移行数 > 0 || 有新增, 位移行数)
}

fn 解析点(值: &str) -> Option<(f32, f32)> {
    let (x, y) = 值.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// 用证据里的视口区域推导分割线位置（返回逻辑坐标的 from/to）。
fn 分割线坐标(证据: &serde_json::Value, 方向: &str) -> Option<((f32, f32), (f32, f32))> {
    let ppp = 证据.get("pixels_per_point")?.as_f64()? as f32;
    let 区域 = 证据.get("regions")?.as_array()?.clone();
    let 取 = |id: &str| -> Option<(f32, f32, f32, f32)> {
        区域.iter().find_map(|项| {
            if 项.get("id")?.as_str()? != id {
                return None;
            }
            Some((
                项.get("x")?.as_f64()? as f32 / ppp,
                项.get("y")?.as_f64()? as f32 / ppp,
                项.get("w")?.as_f64()? as f32 / ppp,
                项.get("h")?.as_f64()? as f32 / ppp,
            ))
        })
    };
    let (vx, vy, vw, vh) = 取("viewport")?;
    match 方向 {
        "v" => {
            // 分割线在"左邻面板右边缘"与"视口左边缘"之间的中点。
            // 直接用视口 x-2 会落在视口内部，按不到分割条（真实踩过）。
            let 左 = 区域
                .iter()
                .filter_map(|项| {
                    let id = 项.get("id")?.as_str()?;
                    let x = 项.get("x")?.as_f64()? as f32 / ppp;
                    let w = 项.get("w")?.as_f64()? as f32 / ppp;
                    let y = 项.get("y")?.as_f64()? as f32 / ppp;
                    // 只比"左上角在同一水平带、右边缘在视口左侧"就够定位分割线，
                    // 高度不参与判定，因此不去读 `h`（读了也只是死变量）。
                    (id != "viewport" && x + w <= vx + 1.0 && (y - vy).abs() < 4.0).then_some(x + w)
                })
                .fold(f32::MIN, f32::max);
            let 左边缘 = if 左.is_finite() { 左 } else { vx - 20.0 };
            let 边界 = (左边缘 + vx) * 0.5;
            let 中间 = vy + vh * 0.5;
            Some(((边界, 中间), (边界 + 70.0, 中间)))
        }
        "h" => {
            // 分割线在"视口下边缘"与"下方面板顶边"之间的中点。
            let 下 = 区域
                .iter()
                .filter_map(|项| {
                    let id = 项.get("id")?.as_str()?;
                    let y = 项.get("y")?.as_f64()? as f32 / ppp;
                    let x = 项.get("x")?.as_f64()? as f32 / ppp;
                    (id != "viewport" && y >= vy + vh - 1.0 && (x - vx).abs() < 40.0).then_some(y)
                })
                .fold(None::<f32>, |当前, 值| {
                    Some(当前.map_or(值, |已有| 已有.min(值)))
                });
            let 下边缘 = 下.unwrap_or(vy + vh + 20.0);
            let 边界 = (vy + vh + 下边缘) * 0.5;
            let 中间 = vx + vw * 0.5;
            Some(((中间, 边界), (中间, 边界 - 55.0)))
        }
        _ => None,
    }
}

/// 读取证据 JSON。
/// 证据里可比较的**结构化状态**（区域矩形之外的事实）。
///
/// # 为什么不能只比区域矩形
///
/// 点击「折叠箭头」不会移动任何面板，点击「运行项目」也不会。
/// 只比矩形会把这类**功能真的生效了**的点击判成"输入未生效"，
/// 进而把工具本身的问题误判成界面缺陷。反过来，如果只比矩形，
/// 一个"看起来变了但其实没执行动作"的点击也可能蒙混过关。
///
/// 这里挑出的都是**用户可见的功能事实**：
/// * `collapsed_count` / `scene_rows`：折叠是否真的改变了可见行；
/// * `run_status` / `run_pid` / `run_running`：运行是否真的起了子进程；
/// * `run_log_lines`：子进程输出是否真的进了控制台；
/// * `mode` / `workspace`：导航按钮是否真的切了目标。
fn 状态指纹(证据: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    let mut 表 = std::collections::BTreeMap::new();
    let 取 = |路径: &[&str]| -> Option<&serde_json::Value> {
        let mut 当前 = 证据;
        for 段 in 路径 {
            当前 = 当前.get(*段)?;
        }
        Some(当前)
    };
    let 文本 = |路径: &[&str]| -> String {
        match 取(路径) {
            Some(serde_json::Value::String(值)) => 值.clone(),
            Some(serde_json::Value::Bool(值)) => 值.to_string(),
            Some(serde_json::Value::Number(值)) => 值.to_string(),
            Some(serde_json::Value::Null) | None => "null".to_string(),
            Some(其他) => 其他.to_string(),
        }
    };

    表.insert("mode".to_string(), 文本(&["mode", "id"]));
    表.insert("workspace".to_string(), 文本(&["workspace", "id"]));
    表.insert("run_status".to_string(), 文本(&["project_run", "status"]));
    表.insert("run_running".to_string(), 文本(&["project_run", "running"]));
    表.insert("run_pid".to_string(), 文本(&["project_run", "pid"]));
    表.insert(
        "run_log_lines".to_string(),
        取(&["project_run", "log_tail"])
            .and_then(|值| 值.as_array())
            .map(|值| 值.len().to_string())
            .unwrap_or_else(|| "0".to_string()),
    );
    表.insert("collapsed_count".to_string(), 文本(&["collapsed_count"]));
    // 选中集：让"点这一行确实选中了它"成为可比较的状态，而不是靠看截图。
    //
    // 改造前状态指纹里没有选中集，于是"点击生效了但只改了选中"与
    // "点击压根没送达"在前后的指纹里完全一样 —— 这正是上一轮
    // `select_row` 会话"变化 0 项"却无法判定对错的原因。
    表.insert(
        "selection".to_string(),
        取(&["selection", "ids"])
            .and_then(|值| 值.as_array())
            .map(|值| {
                值.iter()
                    .filter_map(|项| 项.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default(),
    );
    // 右键菜单：`种类:目标名:菜单项`。菜单是浮层，不移动任何矩形，
    // 只有把它写进指纹，右键类会话才谈得上"有变化可判定"。
    表.insert(
        "context_menu".to_string(),
        match 证据菜单(证据) {
            None => "none".to_string(),
            Some((kind, target)) => {
                let 项 = 证据
                    .get("context_menu")
                    .and_then(|值| 值.get("items"))
                    .and_then(|值| 值.as_array())
                    .map(|值| {
                        值.iter()
                            .filter_map(|项| 项.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                format!("{kind}:{}:{项}", target.unwrap_or_default())
            }
        },
    );
    // ---- 变换工具 / 操纵器 / 文档事务 ----
    //
    // 这几项决定了"拖拽操纵器"这件事能不能被断言：
    // 工具与悬停说明命中到了哪个部件；`selection_transform` 说明实体真的变了；
    // `undo_steps` 说明一次拖拽只提交一次；`document_dirty` 说明文档被改脏了。
    表.insert("transform_tool".to_string(), 文本(&["transform_tool"]));
    表.insert("gizmo_hover".to_string(), 文本(&["gizmo_hover"]));
    表.insert("gizmo_drag".to_string(), 文本(&["gizmo_drag"]));
    表.insert(
        "selection_transform".to_string(),
        证据
            .get("selection_transform")
            .filter(|值| !值.is_null())
            .map(|值| {
                let 数 = |字段: &str| -> String {
                    值.get(字段)
                        .and_then(|项| 项.as_array())
                        .map(|项| {
                            项.iter()
                                .map(|项| format!("{:.5}", 项.as_f64().unwrap_or(0.0)))
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default()
                };
                format!(
                    "平移[{}] 旋转[{}] 缩放[{}]",
                    数("translate"),
                    数("rotation"),
                    数("scale")
                )
            })
            .unwrap_or_else(|| "none".to_string()),
    );
    表.insert("undo_steps".to_string(), 文本(&["undo_steps"]));
    表.insert("document_dirty".to_string(), 文本(&["document_dirty"]));
    表.insert(
        "console_log_count".to_string(),
        文本(&["console_log_count"]),
    );
    // 控制台末行：子进程输出是否真的进入了**界面会画出来的**日志区域。
    表.insert(
        "console_log_tail".to_string(),
        取(&["console_log_tail"])
            .and_then(|值| 值.as_array())
            .map(|行| {
                行.iter()
                    .filter_map(|行| 行.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default(),
    );
    // 场景树可见行：`深度:名称:是否展开`。折叠会让行数与内容同时变化。
    表.insert(
        "scene_rows".to_string(),
        取(&["scene_rows"])
            .and_then(|值| 值.as_array())
            .map(|行| {
                行.iter()
                    .map(|行| {
                        format!(
                            "{}:{}:{}",
                            行.get("depth").and_then(|值| 值.as_i64()).unwrap_or(-1),
                            行.get("name").and_then(|值| 值.as_str()).unwrap_or(""),
                            行.get("expanded")
                                .and_then(|值| 值.as_bool())
                                .unwrap_or(false),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default(),
    );
    表
}

/// 比较前后两帧的结构化状态，返回发生变化的字段名。
fn 比较状态(前: &serde_json::Value, 后: &serde_json::Value) -> Vec<String> {
    let 前表 = 状态指纹(前);
    let 后表 = 状态指纹(后);
    let mut 结果 = Vec::new();
    for (键, 值) in &前表 {
        if 后表.get(键) != Some(值) {
            结果.push(键.clone());
        }
    }
    for 键 in 后表.keys() {
        if !前表.contains_key(键) {
            结果.push(键.clone());
        }
    }
    结果.sort();
    结果.dedup();
    结果
}

fn 读取证据(路径: &Path) -> Result<serde_json::Value, String> {
    let 文本 = std::fs::read_to_string(路径).map_err(|错误| 错误.to_string())?;
    serde_json::from_str(&文本).map_err(|错误| 错误.to_string())
}

/// 比较前后两帧的区域矩形，返回发生变化（>1px）的区域 id。
fn 比较区域(前: &serde_json::Value, 后: &serde_json::Value) -> Vec<String> {
    let 取表 =
        |证据: &serde_json::Value| -> std::collections::BTreeMap<String, (f32, f32, f32, f32)> {
            let mut 表 = std::collections::BTreeMap::new();
            if let Some(区域) = 证据.get("regions").and_then(|值| 值.as_array()) {
                for 项 in 区域 {
                    let id = 项.get("id").and_then(|值| 值.as_str()).unwrap_or("");
                    let 数 = |键: &str| 项.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) as f32;
                    表.insert(id.to_string(), (数("x"), 数("y"), 数("w"), 数("h")));
                }
            }
            表
        };
    let 前表 = 取表(前);
    let 后表 = 取表(后);
    let mut 结果 = Vec::new();
    for (id, 值) in &前表 {
        match 后表.get(id) {
            Some(新) => {
                let 差 = (值.0 - 新.0).abs()
                    + (值.1 - 新.1).abs()
                    + (值.2 - 新.2).abs()
                    + (值.3 - 新.3).abs();
                if 差 > 1.0 {
                    结果.push(id.clone());
                }
            }
            None => 结果.push(id.clone()),
        }
    }
    for id in 后表.keys() {
        if !前表.contains_key(id) {
            结果.push(id.clone());
        }
    }
    结果.sort();
    结果.dedup();
    结果
}

#[allow(clippy::too_many_arguments)]
fn 写拖拽报告(
    输出目录: &Path,
    名称: &str,
    窗口: &窗口信息,
    ppp: f32,
    从逻辑: (f32, f32),
    到逻辑: (f32, f32),
    起点屏幕: (i32, i32),
    终点屏幕: (i32, i32),
    注入: &input::注入结果,
    收到指针: bool,
    变化: &[String],
    状态变化: &[String],
    前证据: &serde_json::Value,
    后证据: &serde_json::Value,
    // 变换类断言用的基准帧（最后一次注入之前的那一帧）。
    基准: &serde_json::Value,
    期望区域: &Option<String>,
    期望状态: &[String],
    期望不变: bool,
    期望还原: bool,
    期望滚动: bool,
    允许无变化: bool,
    右键用例: &Option<String>,
    滚轮格数: i32,
    滚动结果: (bool, usize),
    插槽2点击: &[serde_json::Value],
    阶段记录表: &[阶段记录],
    期望菜单: &Option<String>,
    期望选中: &Option<String>,
    菜单判定: &Result<(), String>,
    选中判定: &Result<(), String>,
    期望工具: &Option<String>,
    期望悬停: &Option<String>,
    期望变换变化: &[String],
    期望变换不变: &[String],
    期望撤销增量: &Option<i64>,
    期望脏状态: &Option<String>,
    工具判定: &Result<(), String>,
    悬停判定: &Result<(), String>,
    变换判定: &Result<(), String>,
    撤销判定: &Result<(), String>,
    脏判定: &Result<(), String>,
) {
    let 状态表 = |证据: &serde_json::Value| -> serde_json::Value {
        let mut 对象 = serde_json::Map::new();
        for (键, 值) in 状态指纹(证据) {
            对象.insert(键, serde_json::Value::String(值));
        }
        serde_json::Value::Object(对象)
    };
    let 报告 = serde_json::json!({
        "schema": 2,
        "name": 名称,
        "window": {
            "handle": 窗口.句柄,
            "title": 窗口.标题,
            "client": 窗口.客户区,
            "origin": 窗口.客户区原点,
            "dpi": 窗口.dpi,
        },
        "pixels_per_point": ppp,
        "coords": {
            "from_logical": [从逻辑.0, 从逻辑.1],
            "to_logical": [到逻辑.0, 到逻辑.1],
            "from_screen": [起点屏幕.0, 起点屏幕.1],
            "to_screen": [终点屏幕.0, 终点屏幕.1],
        },
        "input": {
            "summary": 注入.摘要(),
            "complete": 注入.完整(),
            "moves": 注入.移动步数,
        },
        "pointer_reached_app": 收到指针,
        // 插槽 2：坐标取自第二帧证据（滚动后、点击前）
        "slot2_clicks": 插槽2点击,
        "changed_regions": 变化,
        "changed_state": 状态变化,
        "state_before": 状态表(前证据),
        "state_after": 状态表(后证据),
        "run_before": 前证据.get("project_run").cloned(),
        "run_after": 后证据.get("project_run").cloned(),
        "scroll": {
            // `notches == 0` 表示这次会话没有用滚轮。
            "notches": 滚轮格数,
            // 是否真的改变了场景树行的位置（不是"事件发出去了"）。
            "effective": 滚动结果.0,
            "moved_rows": 滚动结果.1,
        },
        "expect_region": 期望区域,
        "expect_state": 期望状态,
        "expect_no_change": 期望不变,
        "expect_restore": 期望还原,
        "expect_scroll": 期望滚动,
        "allow_no_change": 允许无变化,
        "right_click_case": 右键用例,
        // ---- 逐阶段中间态证据 ----
        //
        // 每个注入阶段按执行顺序一条，带落点、几何来源与注入结果。
        // 失败时先看这里：能直接指出是"注入没送达"、"坐标取自过期几何"，
        // 还是"界面没按预期响应"。
        "stages": 阶段记录表.iter().map(阶段记录::到json).collect::<Vec<_>>(),
        // ---- 断言（不是"有没有变化"，而是"状态对不对"）----
        "expect_menu": 期望菜单,
        "expect_selection": 期望选中,
        "menu_actual": 证据菜单(后证据).map(|(kind, target)| serde_json::json!({
            "kind": kind,
            "target": target,
        })),
        "selection_actual": 后证据.get("selection").cloned(),
        "menu_ok": 菜单判定.is_ok(),
        "menu_error": 菜单判定.as_ref().err(),
        "selection_ok": 选中判定.is_ok(),
        "selection_error": 选中判定.as_ref().err(),
    });
    // 变换相关的字段单独构造再合并：一个 `json!` 里塞进近百个键会撞上
    // 宏的递归上限（`recursion limit reached while expanding json_internal!`）。
    let 变换段 = serde_json::json!({
        // ---- 变换工具 / 操纵器 / 文档事务 ----
        "expect_tool": 期望工具,
        "expect_gizmo_hover": 期望悬停,
        "expect_transform_changed": 期望变换变化,
        "expect_transform_unchanged": 期望变换不变,
        "expect_undo_delta": 期望撤销增量,
        "expect_dirty": 期望脏状态,
        "tool_actual": 后证据.get("transform_tool"),
        "gizmo_hover_actual": 后证据.get("gizmo_hover"),
        "gizmo_drag_actual": 后证据.get("gizmo_drag"),
        "gizmo_parts_actual": 后证据.get("gizmo_parts"),
        "transform_before": 基准.get("selection_transform"),
        "transform_after": 后证据.get("selection_transform"),
        "undo_steps_before": 基准.get("undo_steps"),
        "undo_steps_after": 后证据.get("undo_steps"),
        "dirty_before": 基准.get("document_dirty"),
        "dirty_after": 后证据.get("document_dirty"),
        "tool_ok": 工具判定.is_ok(),
        "tool_error": 工具判定.as_ref().err(),
        "gizmo_hover_ok": 悬停判定.is_ok(),
        "gizmo_hover_error": 悬停判定.as_ref().err(),
        "transform_ok": 变换判定.is_ok(),
        "transform_error": 变换判定.as_ref().err(),
        "undo_ok": 撤销判定.is_ok(),
        "undo_error": 撤销判定.as_ref().err(),
        "dirty_ok": 脏判定.is_ok(),
        "dirty_error": 脏判定.as_ref().err(),
    });
    let mut 报告 = 报告;
    if let (Some(目标), Some(来源)) = (报告.as_object_mut(), 变换段.as_object()) {
        for (键, 值) in 来源 {
            目标.insert(键.clone(), 值.clone());
        }
    }
    let _ = std::fs::write(
        输出目录.join(format!("{名称}.drag.json")),
        serde_json::to_string_pretty(&报告).unwrap_or_default(),
    );
    let 行: Vec<String> = 变化.iter().map(|项| format!("- `{项}`")).collect();
    let 状态行: Vec<String> = 状态变化.iter().map(|项| format!("- `{项}`")).collect();
    // 逐阶段表：失败时的第一现场。
    let mut 阶段行: Vec<String> = Vec::new();
    for (下标, 记录) in 阶段记录表.iter().enumerate() {
        阶段行.push(format!(
            "- {}. `{}` / `{}`{} 逻辑 {:?} 屏幕 {:?}{}{}{}",
            下标 + 1,
            记录.阶段,
            记录.动作,
            match &记录.用例 {
                Some(名) => format!(" 用例 `{名}`"),
                None => String::new(),
            },
            记录.逻辑,
            记录.屏幕,
            match 记录.几何来源 {
                Some(来源) => format!(" 几何取自 `{来源}`"),
                None => String::new(),
            },
            match 记录.注入完整 {
                Some(true) => " 注入完整".to_string(),
                Some(false) => " **注入不完整**".to_string(),
                None => String::new(),
            },
            match &记录.备注 {
                Some(备注) => format!(" — {备注}"),
                None => String::new(),
            },
        ));
    }
    let 菜单行 = match 证据菜单(后证据) {
        Some((kind, target)) => format!("打开，种类 `{kind}`，目标 {target:?}"),
        None => "未打开".to_string(),
    };
    let 选中行 = 后证据
        .get("selection")
        .and_then(|值| 值.get("ids"))
        .and_then(|值| 值.as_array())
        .map(|值| {
            值.iter()
                .filter_map(|项| 项.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|文本| !文本.is_empty())
        .unwrap_or_else(|| "（空）".to_string());
    let 期望菜单文本 = 期望菜单.as_deref().unwrap_or("未指定");
    let 期望选中文本 = 期望选中.as_deref().unwrap_or("未指定");
    let 菜单断言行 = 断言文本(菜单判定);
    let 选中断言行 = 断言文本(选中判定);
    let 文本 = format!(
        "# 真实鼠标拖拽/点击：{名称}\n\n\
         - 窗口：{}（{}），DPI {}\n\
         - 客户区：{}×{}，原点 {:?}\n\
         - pixels_per_point：{ppp:.3}\n\
         - 逻辑坐标：{:?} → {:?}\n\
         - 屏幕坐标：{:?} → {:?}\n\
         - 注入：{}\n\
         - 应用收到指针事件：{}\n\
         - 变化区域（{}）：\n{}\n\
         - 变化状态字段（{}）：\n{}\n\
         - 滚轮：{} 格，是否真的改变了滚动位置：{}（位移行数 {}）\n\
         - 插槽 2 点击（坐标取自第二帧 = 滚动后）：{}\n\
         - 右键之后菜单：{菜单行}\n\
         - 末帧选中集：{选中行}\n\
         - 菜单断言（期望 {期望菜单文本}）：{菜单断言行}\n\
         - 选中断言（期望 {期望选中文本}）：{选中断言行}\n\
         \n## 逐阶段中间态\n\n{}\n",
        窗口.标题,
        窗口.句柄,
        窗口.dpi,
        窗口.客户区.0,
        窗口.客户区.1,
        窗口.客户区原点,
        从逻辑,
        到逻辑,
        起点屏幕,
        终点屏幕,
        注入.摘要(),
        if 收到指针 { "是" } else { "否" },
        变化.len(),
        if 行.is_empty() {
            "- （无）".to_string()
        } else {
            行.join("\n")
        },
        状态变化.len(),
        if 状态行.is_empty() {
            "- （无）".to_string()
        } else {
            状态行.join("\n")
        },
        滚轮格数,
        if 滚动结果.0 { "是" } else { "否" },
        滚动结果.1,
        serde_json::to_string(插槽2点击).unwrap_or_default(),
        if 阶段行.is_empty() {
            "- （本次会话没有注入阶段）".to_string()
        } else {
            阶段行.join("\n")
        }
    );
    let _ = std::fs::write(输出目录.join(format!("{名称}.drag.md")), 文本);
}

/// 把一条断言结果渲染成报告里的一行文字。
///
/// `Ok` 既可能是"断言成立"，也可能是"本次会话没有指定这条断言"；
/// 两者在报告里由同一行的"期望 X"部分区分，所以这里只写结论词。
fn 断言文本(结果: &Result<(), String>) -> String {
    match 结果 {
        Ok(()) => "通过".to_string(),
        Err(错误) => format!("**未通过**：{错误}"),
    }
}

/// 从证据里取某个场景树行（由调用方挑选）的**折叠箭头矩形中心**（逻辑坐标）。
fn 场景树箭头中心<F>(证据: &serde_json::Value, 挑选: F) -> Option<(f32, f32)>
where
    F: FnOnce(&[serde_json::Value]) -> Option<serde_json::Value>,
{
    let ppp = 证据
        .get("pixels_per_point")
        .and_then(|值| 值.as_f64())
        .unwrap_or(1.0);
    let 行列表 = 证据.get("scene_rows")?.as_array()?;
    let 行 = 挑选(行列表)?;
    let 箭头 = 行.get("arrow")?;
    if 箭头.is_null() {
        return None;
    }
    let 取数 = |键: &str| 箭头.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) / ppp;
    Some((
        (取数("x") + 取数("w") * 0.5) as f32,
        (取数("y") + 取数("h") * 0.5) as f32,
    ))
}

fn 拖拽失败报告(
    输出目录: &Path, 名称: &str, 类别: &str, 原因: &str, 码: i32
) -> i32 {
    let 文本 = format!("# 真实鼠标拖拽：{名称}\n\n- 失败：**{类别}**\n- 原因：{原因}\n");
    let _ = std::fs::write(输出目录.join(format!("{名称}.drag.md")), 文本);
    let _ = std::fs::write(
        输出目录.join(format!("{名称}.drag.json")),
        serde_json::json!({"schema":1,"name":名称,"failure":类别,"reason":原因}).to_string(),
    );
    eprintln!("拖拽失败（{类别}）：{原因}");
    码
}

/// 区域逻辑矩形（x,y,w,h），来自证据 JSON 的物理像素。
fn 区域表(
    证据: &serde_json::Value
) -> std::collections::BTreeMap<String, (f32, f32, f32, f32)> {
    let ppp = 证据
        .get("pixels_per_point")
        .and_then(|值| 值.as_f64())
        .unwrap_or(1.0) as f32;
    let mut 表 = std::collections::BTreeMap::new();
    if let Some(区域) = 证据.get("regions").and_then(|值| 值.as_array()) {
        for 项 in 区域 {
            let Some(id) = 项.get("id").and_then(|值| 值.as_str()) else {
                continue;
            };
            let 取 = |键: &str| 项.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) as f32 / ppp;
            表.insert(id.to_string(), (取("x"), 取("y"), 取("w"), 取("h")));
        }
    }
    表
}

/// 从证据里取某个操纵器部件的**落点与拖拽方向**（逻辑坐标）。
///
/// 方向来自应用在**当前帧**测得的投影几何，因此 3D 相机怎么转、
/// 2D 视图怎么缩放，拖拽方向都跟着正确 —— 不写死任何屏幕坐标。
fn 操纵器部件(证据: &serde_json::Value, 部件: &str) -> Option<((f32, f32), (f32, f32))> {
    let ppp = 证据
        .get("pixels_per_point")
        .and_then(|值| 值.as_f64())
        .unwrap_or(1.0) as f32;
    let 列表 = 证据.get("gizmo_parts")?.as_array()?;
    let 项 = 列表
        .iter()
        .find(|项| 项.get("part").and_then(|值| 值.as_str()) == Some(部件))?;
    let hit = 项.get("hit")?.as_array()?;
    let dir = 项.get("dir")?.as_array()?;
    let 取 = |数组: &Vec<serde_json::Value>, 下标: usize| -> f32 {
        数组.get(下标).and_then(|值| 值.as_f64()).unwrap_or(0.0) as f32
    };
    Some((
        (取(hit, 0) / ppp, 取(hit, 1) / ppp),
        (取(dir, 0), 取(dir, 1)),
    ))
}

/// 从证据里读主选中实体的变换。
fn 证据变换(证据: &serde_json::Value) -> Option<&serde_json::Value> {
    let 变换 = 证据.get("selection_transform")?;
    if 变换.is_null() { None } else { Some(变换) }
}

/// 读某个变换分量的三个值（`translate` / `scale`）。
fn 变换分量(证据: &serde_json::Value, 字段: &str) -> Option<[f64; 3]> {
    let 值 = 证据变换(证据)?.get(字段)?.as_array()?;
    if 值.len() < 3 {
        return None;
    }
    Some([值[0].as_f64()?, 值[1].as_f64()?, 值[2].as_f64()?])
}

/// 断言"某个变换分量发生了变化 / 没有变化"。
///
/// # 为什么按分量而不是按"整体是否变化"
///
/// 工程要求"未操作轴保持原值"。如果只比"变换变了没有"，
/// 把 Y 轴误写成也一起动了的实现会照样通过 —— 那正是这类缺陷的典型形态。
fn 判定变换分量(
    前: &serde_json::Value,
    后: &serde_json::Value,
    期望变化: &[String],
    期望不变: &[String],
) -> Result<(), String> {
    for 字段 in 期望变化 {
        let 前值 =
            变换分量(前, 字段).ok_or_else(|| format!("第一帧证据里没有变换分量 `{字段}`"))?;
        let 后值 = 变换分量(后, 字段).ok_or_else(|| format!("末帧证据里没有变换分量 `{字段}`"))?;
        let 差 = (0..3)
            .map(|i| (后值[i] - 前值[i]).abs())
            .fold(0.0f64, f64::max);
        if 差 <= 1e-4 {
            return Err(format!(
                "期望 `{字段}` 发生变化，但前后完全一致（{前值:?} → {后值:?}）"
            ));
        }
    }
    for 字段 in 期望不变 {
        let 前值 =
            变换分量(前, 字段).ok_or_else(|| format!("第一帧证据里没有变换分量 `{字段}`"))?;
        let 后值 = 变换分量(后, 字段).ok_or_else(|| format!("末帧证据里没有变换分量 `{字段}`"))?;
        let 差 = (0..3)
            .map(|i| (后值[i] - 前值[i]).abs())
            .fold(0.0f64, f64::max);
        if 差 > 1e-4 {
            return Err(format!(
                "期望 `{字段}` 保持不变，实际从 {前值:?} 变成 {后值:?}（最大变化 {差:.6}）"
            ));
        }
    }
    Ok(())
}

/// 断言"一次拖拽只提交一次"：撤销步数必须恰好增加 `期望`。
fn 判定撤销步数(
    前: &serde_json::Value,
    后: &serde_json::Value,
    期望: i64,
) -> Result<(), String> {
    let 取 = |证据: &serde_json::Value| -> i64 {
        证据
            .get("undo_steps")
            .and_then(|值| 值.as_i64())
            .unwrap_or(-1)
    };
    let (前值, 后值) = (取(前), 取(后));
    if 前值 < 0 || 后值 < 0 {
        return Err("证据里没有 undo_steps 字段（应用版本过旧？）".to_string());
    }
    let 实际 = 后值 - 前值;
    if 实际 != 期望 {
        return Err(format!(
            "撤销步数应增加 {期望}，实际增加 {实际}（{前值} → {后值}）"
        ));
    }
    Ok(())
}

/// 断言"文档脏状态符合期望"。
fn 判定脏状态(证据: &serde_json::Value, 期望: Option<&str>) -> Result<(), String> {
    let Some(期望) = 期望 else {
        return Ok(());
    };
    let 实际 = 证据
        .get("document_dirty")
        .and_then(|值| 值.as_bool())
        .ok_or_else(|| "证据里没有 document_dirty 字段（应用版本过旧？）".to_string())?;
    let 期望值 = match 期望 {
        "yes" | "true" | "1" => true,
        "no" | "false" | "0" => false,
        其他 => return Err(format!("无法识别的 --expect-dirty 取值 `{其他}`")),
    };
    if 实际 != 期望值 {
        return Err(format!("期望文档脏状态为 {期望值}，实际是 {实际}"));
    }
    Ok(())
}

/// 断言"当前变换工具符合期望"。
fn 判定工具(证据: &serde_json::Value, 期望: Option<&str>) -> Result<(), String> {
    let Some(期望) = 期望 else {
        return Ok(());
    };
    let 实际 = 证据
        .get("transform_tool")
        .and_then(|值| 值.as_str())
        .ok_or_else(|| "证据里没有 transform_tool 字段（应用版本过旧？）".to_string())?;
    if 实际 != 期望 {
        return Err(format!("期望变换工具为 `{期望}`，实际是 `{实际}`"));
    }
    Ok(())
}

/// 断言"操纵器悬停状态符合期望"。`none` 表示不应有悬停。
fn 判定悬停(证据: &serde_json::Value, 期望: Option<&str>) -> Result<(), String> {
    let Some(期望) = 期望 else {
        return Ok(());
    };
    let 实际 = 证据
        .get("gizmo_hover")
        .and_then(|值| 值.as_str())
        .unwrap_or("none");
    if 实际 != 期望 {
        return Err(format!("期望操纵器悬停为 `{期望}`，实际是 `{实际}`"));
    }
    Ok(())
}

/// 场景树用例的坐标推导（含操纵器部件）。
///
/// 坐标全部相对客户区逻辑像素；标签条上的落点取面板左上角附近，
/// egui_dock 的标签命中区从面板顶边开始约 24 逻辑像素高。
fn 用例坐标(名: &str, 证据: &serde_json::Value) -> Option<((f32, f32), (f32, f32))> {
    // ---- 操纵器：`gizmo:<部件>` 取落点；`gizmo-drag:<部件>:<像素>` 沿实测方向拖 ----
    //
    // 这两个用例的坐标**完全来自应用当帧测得的几何**：
    // 落点是该部件的代表点，方向是该轴/圆环在屏幕上的投影方向。
    // 因此 3D 相机怎么转、2D 视图怎么缩放，拖拽都落在正确方向上。
    if let Some(部件) = 名.strip_prefix("gizmo:") {
        let (点, _) = 操纵器部件(证据, 部件)?;
        return Some((点, 点));
    }
    if let Some(其余) = 名.strip_prefix("gizmo-drag:") {
        let mut 段 = 其余.split(':');
        let 部件 = 段.next()?;
        let 距离: f32 = 段.next()?.trim().parse().ok()?;
        let (起点, 方向) = 操纵器部件(证据, 部件)?;
        let 终点 = (起点.0 + 方向.0 * 距离, 起点.1 + 方向.1 * 距离);
        return Some((起点, 终点));
    }

    let 表 = 区域表(证据);
    let 取 = |id: &str| 表.get(id).copied();
    let 标签 = |id: &str| 取(id).map(|(x, y, _, _)| (x + 40.0, y - 14.0));

    // ---- 通用单击：`click:<区域 id>` 点该区域的**真实矩形中心** ----
    //
    // 这是"不再猜坐标"的落点：矩形来自应用自己记录的证据，
    // 坐标换算（物理→逻辑、客户区原点）由本工具统一处理。
    if let Some(区域id) = 名.strip_prefix("click:") {
        let (x, y, w, h) = 取(区域id)?;
        let 中心 = (x + w * 0.5, y + h * 0.5);
        return Some((中心, 中心));
    }

    // ---- 通用拖拽：`drag:<区域 id>:<dx>:<dy>` ----
    //
    // 起落点都来自应用记录的**真实矩形中心**，只额外给一个像素位移。
    // 用于"数值输入框连拖"这类没有操纵器几何可用的控件：
    // 拖 48px 相当于把 `DragValue` 的值拖动 48 × 步进 那么多。
    if let Some(其余) = 名.strip_prefix("drag:") {
        let 段: Vec<&str> = 其余.split(':').collect();
        if 段.len() == 3
            && let (Some(dx), Some(dy)) = (
                段[1].trim().parse::<f32>().ok(),
                段[2].trim().parse::<f32>().ok(),
            )
            && let Some((x, y, w, h)) = 取(段[0])
        {
            let 中心 = (x + w * 0.5, y + h * 0.5);
            return Some((中心, (中心.0 + dx, 中心.1 + dy)));
        }
    }

    // ---- 场景树箭头：`arrow:<序号>` / `arrow_name:<名称>` ----
    //
    // 折叠箭头只有 18px 宽且按深度缩进，猜坐标必然点偏（点偏就落到名称
    // 标签上 = 只改选择，不改折叠）。这里直接用应用记录的箭头矩形中心。
    if let Some(序号文本) = 名.strip_prefix("arrow:") {
        let 序号: usize = 序号文本.parse().ok()?;
        let 点 = 场景树箭头中心(证据, |行列表| {
            行列表
                .iter()
                .filter(|行| 行.get("arrow").map(|值| !值.is_null()).unwrap_or(false))
                .nth(序号)
                .cloned()
        })?;
        return Some((点, 点));
    }
    if let Some(名称) = 名.strip_prefix("arrow_name:") {
        let 点 = 场景树箭头中心(证据, |行列表| {
            行列表
                .iter()
                .find(|行| {
                    行.get("name").and_then(|值| 值.as_str()) == Some(名称)
                        && 行.get("arrow").map(|值| !值.is_null()).unwrap_or(false)
                })
                .cloned()
        })?;
        return Some((点, 点));
    }
    // 场景树：点**名称文字**。
    //
    // 名称起点由该行类型图标的真实右边缘 + 固定间距推出，
    // 因此不依赖"整行宽度"或猜测文字宽度。
    if let Some(名称) = 名.strip_prefix("name:") {
        let ppp = 证据
            .get("pixels_per_point")
            .and_then(|值| 值.as_f64())
            .unwrap_or(1.0);
        let 行列表 = 证据.get("scene_rows")?.as_array()?;
        let 行 = 行列表
            .iter()
            .find(|行| 行.get("name").and_then(|值| 值.as_str()) == Some(名称))?;
        let 图标 = 行.get("type_icon")?;
        if 图标.is_null() {
            return None;
        }
        let 取数 = |键: &str| 图标.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) / ppp;
        // 名称紧跟在类型图标之后：右边缘 + 4px 间距，再往里 12px 落在文字上。
        let 点 = (
            (取数("x") + 取数("w") + 16.0) as f32,
            (取数("y") + 取数("h") * 0.5) as f32,
        );
        return Some((点, 点));
    }

    // 场景树某一行的**整行空白区**中心（右键用例的落点）。
    //
    // 与 `name:` 的区别正是这次要验的东西：
    // - `name:` 落在名称文字上，那里本来就有可点控件，右键弹菜单是**旧行为**；
    // - `row_blank:` 落在"名称右边缘 → 状态开关左边缘"之间那条**只有空白**的
    //   带子上。改造前那里没有任何控件，右键什么都不会发生。
    // 因此只有后者能证明"整行都能右键"。
    if let Some(名称) = 名.strip_prefix("row_blank:") {
        let ppp = 证据
            .get("pixels_per_point")
            .and_then(|值| 值.as_f64())
            .unwrap_or(1.0);
        let 行列表 = 证据.get("scene_rows")?.as_array()?;
        let 行 = 行列表
            .iter()
            .find(|行| 行.get("name").and_then(|值| 值.as_str()) == Some(名称))?;
        let 空白 = 行.get("blank")?;
        if 空白.is_null() {
            // 这一行名称太长，没有空白区。返回 None 让调用方报"取不到坐标"，
            // 而不是退回去点名称 —— 那会把"空白区不存在"伪装成"通过了"。
            return None;
        }
        let 取数 = |键: &str| 空白.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) / ppp;
        let 点 = (
            (取数("x") + 取数("w") * 0.5) as f32,
            (取数("y") + 取数("h") * 0.5) as f32,
        );
        return Some((点, 点));
    }

    // 场景树某一行的整行矩形中心（用于验证"点行 = 选择"）。
    if let Some(名称) = 名.strip_prefix("row_name:") {
        let ppp = 证据
            .get("pixels_per_point")
            .and_then(|值| 值.as_f64())
            .unwrap_or(1.0);
        let 行列表 = 证据.get("scene_rows")?.as_array()?;
        let 行 = 行列表
            .iter()
            .find(|行| 行.get("name").and_then(|值| 值.as_str()) == Some(名称))?;
        let 行矩形 = 行.get("row")?;
        let 取数 = |键: &str| 行矩形.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) / ppp;
        // 取行的右半部分：避开左侧的箭头与缩进，只命中名称标签。
        let 点 = (
            (取数("x") + 取数("w") * 0.75) as f32,
            (取数("y") + 取数("h") * 0.5) as f32,
        );
        return Some((点, 点));
    }

    match 名 {
        // 场景树**空白区域**的中心：右键应当弹"空白菜单"（新建顶层实体）。
        //
        // 矩形来自应用自己记录的 `scene_blank`。猜坐标（例如"面板底部往上 40px"）
        // 会在行数变化时落到最后一行上，于是弹出的其实是行菜单 ——
        // 那样测的就不是空白区了。
        "scene_blank" => {
            let ppp = 证据
                .get("pixels_per_point")
                .and_then(|值| 值.as_f64())
                .unwrap_or(1.0) as f32;
            let 空白 = 证据.get("scene_blank")?;
            if 空白.is_null() {
                return None;
            }
            let 取数 =
                |键: &str| 空白.get(键).and_then(|值| 值.as_f64()).unwrap_or(0.0) as f32 / ppp;
            let 点 = (取数("x") + 取数("w") * 0.5, 取数("y") + 取数("h") * 0.5);
            Some((点, 点))
        }
        // 把场景树标签拖进视口标签 → 合并成同一标签组
        "merge" => {
            let 起 = 标签("scene_tree")?;
            let (x, y, _, _) = 取("viewport")?;
            Some((起, (x + 55.0, y - 14.0)))
        }
        // 把检查器标签拖到视口中央 → 拆分出新分区
        "split" => {
            // 拖到视口右边缘内侧：egui_dock 在叶片边缘放手会拆分为新分区，
            // 落在正中间则只是合并成标签页 —— 两者验证的目标不同。
            let 起 = 标签("inspector")?;
            let (x, y, w, h) = 取("viewport")?;
            Some((起, (x + w - 24.0, y + h * 0.5)))
        }
        // 关闭检查器：点标签右侧的关闭按钮
        "close" => {
            let (x, y, w, _) = 取("inspector")?;
            Some(((x + w - 12.0, y - 16.0), (x + w - 12.0, y - 16.0)))
        }
        // 浮动：把控制台标签拖到菜单栏区域（Dock 之外）
        "float" => {
            // egui_dock 0.21 默认 OverlayType::Widgets：
            // 叶片中心的按钮 = 合并，边缘按钮 = 拆分；
            // **叶片的角落**（按钮簇之外、叶片之内）才是 TabDestination::Window。
            // 拖到 DockArea 之外（菜单栏/工具栏）不会进入拖放判定 —— 这是之前失败的真实原因。
            let 起 = 标签("inspector")?;
            let (x, y, w, h) = 取("viewport")?;
            Some((起, (x + w - 25.0, y + h - 25.0)))
        }
        _ => None,
    }
}

/// 确保目标窗口位于前台：先 `SetForegroundWindow`，失败则点标题栏激活。
fn 确保前台(窗口: &窗口信息) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, SetForegroundWindow,
        };
        let Ok(句柄) = usize::from_str_radix(窗口.句柄.trim_start_matches("0x"), 16) else {
            return;
        };
        let 句柄 = 句柄 as _;
        // SAFETY: 只传窗口句柄。
        let 已是前台 = unsafe { GetForegroundWindow() } == 句柄;
        if 已是前台 {
            return;
        }
        // SAFETY: 句柄来自 EnumWindows 的枚举结果。
        unsafe {
            SetForegroundWindow(句柄);
        }
        std::thread::sleep(Duration::from_millis(120));
        if unsafe { GetForegroundWindow() } == 句柄 {
            return;
        }
        // 点标题栏激活：窗口矩形顶边 + 10px，水平取客户区中部。
        let (x, y, w, _) = 窗口.窗口矩形;
        let 标题点 = (x + w / 2, y + 10);
        let _ = input::移动到(标题点);
        std::thread::sleep(Duration::from_millis(80));
        let _ = input::按下();
        std::thread::sleep(Duration::from_millis(60));
        let _ = input::抬起();
        std::thread::sleep(Duration::from_millis(150));
    }
    #[cfg(not(windows))]
    {
        let _ = 窗口;
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 造一份最小证据：一行 + 指定选中集 + 指定菜单。
    fn 证据(选中: &[&str], 菜单: Option<(&str, Option<&str>)>) -> serde_json::Value {
        let 行 = serde_json::json!({
            "id": "实体-甲",
            "name": "甲",
            "depth": 0,
            "has_children": false,
            "expanded": false,
            "row": {"x": 0, "y": 0, "w": 100, "h": 20},
            "blank": null,
        });
        let context_menu = match 菜单 {
            None => serde_json::Value::Null,
            Some((kind, target)) => serde_json::json!({
                "kind": kind,
                "target": target,
                "items": ["rename", "delete"],
            }),
        };
        serde_json::json!({
            "pixels_per_point": 1.0,
            "window": {"width": 800, "height": 600},
            "scene_rows": [行],
            "scene_view": {"x": 0, "y": 0, "w": 400, "h": 300},
            "scene_blank": {"x": 0, "y": 20, "w": 400, "h": 280},
            "selection": {
                "primary": 选中.first(),
                "count": 选中.len(),
                "ids": 选中,
            },
            "context_menu": context_menu,
        })
    }

    #[test]
    fn 状态指纹包含选中集与右键菜单() {
        let 前 = 证据(&[], None);
        let 后 = 证据(&["实体-甲"], Some(("scene_row", Some("甲"))));
        let 变化 = 比较状态(&前, &后);
        assert!(
            变化.iter().any(|项| 项 == "selection"),
            "选中集从空变为 {{甲}} 必须被状态指纹发现，实际变化：{变化:?}"
        );
        assert!(
            变化.iter().any(|项| 项 == "context_menu"),
            "右键菜单从无到有必须被状态指纹发现，实际变化：{变化:?}"
        );
    }

    #[test]
    fn 菜单种类不同则状态指纹不同() {
        // 行菜单与空白菜单都"有菜单"，但它们是不同的界面状态：
        // 只比"有没有菜单"会把"右键空白却弹出行菜单"判成通过。
        let 行菜单 = 证据(&[], Some(("scene_row", Some("甲"))));
        let 空白菜单 = 证据(&[], Some(("scene_blank", None)));
        assert_ne!(
            状态指纹(&行菜单).get("context_menu"),
            状态指纹(&空白菜单).get("context_menu")
        );
    }

    #[test]
    fn 菜单断言能区分行菜单与空白菜单() {
        let 行菜单 = 证据(&[], Some(("scene_row", Some("甲"))));
        let 空白菜单 = 证据(&[], Some(("scene_blank", None)));
        assert!(判定菜单(&行菜单, Some("row")).is_ok());
        assert!(判定菜单(&空白菜单, Some("blank")).is_ok());
        // 关键负向用例：右键空白区却弹出行菜单，必须被判失败。
        assert!(判定菜单(&行菜单, Some("blank")).is_err());
        assert!(判定菜单(&空白菜单, Some("row")).is_err());
    }

    #[test]
    fn 菜单断言为none时要求确实没有菜单() {
        let 无菜单 = 证据(&[], None);
        let 有菜单 = 证据(&[], Some(("scene_row", Some("甲"))));
        assert!(判定菜单(&无菜单, Some("none")).is_ok());
        assert!(判定菜单(&有菜单, Some("none")).is_err());
    }

    #[test]
    fn 未指定菜单断言时永远通过() {
        assert!(判定菜单(&证据(&[], Some(("scene_row", None))), None).is_ok());
        assert!(判定菜单(&证据(&[], None), None).is_ok());
    }

    #[test]
    fn 无法识别的菜单期望取值被拒绝() {
        let 错误 = 判定菜单(&证据(&[], None), Some("别的东西"))
            .expect_err("未知取值必须报错，而不是静默通过");
        assert!(
            错误.contains("row / blank / none"),
            "错误信息要给出可选值：{错误}"
        );
    }

    #[test]
    fn 选中断言按行名称核对标识() {
        let 选中 = 证据(&["实体-甲"], None);
        assert!(判定选中(&选中, Some("甲")).is_ok());
        // 选中了别的实体 → 失败。
        let 空 = 证据(&[], None);
        assert!(判定选中(&空, Some("甲")).is_err());
        // 行不存在 → 明确说明"找不到行"，而不是笼统判失败。
        let 错误 = 判定选中(&选中, Some("不存在的行")).expect_err("行不存在应当报错");
        assert!(
            错误.contains("找不到名为"),
            "错误信息要指出找不到哪一行：{错误}"
        );
    }

    #[test]
    fn 阶段记录序列化保留落点与几何来源() {
        let 记录 = 阶段记录::新("right_click", "right_click")
            .用例("row_blank:植被")
            .落点((120.0, 240.0), (300, 500))
            .来自("before")
            .注入(true)
            .备注("测试");
        let json = 记录.到json();
        assert_eq!(json["stage"], "right_click");
        assert_eq!(json["case"], "row_blank:植被");
        assert_eq!(json["logical"][0], 120.0);
        assert_eq!(json["screen"][1], 500);
        assert_eq!(json["geometry_from"], "before");
        assert_eq!(json["input_complete"], true);
    }

    #[test]
    fn 场景空白区用例取矩形中心() {
        let 证据 = 证据(&[], None);
        let (逻辑, _) =
            用例坐标("scene_blank", &证据).expect("证据里有 scene_blank 就应当取到坐标");
        // scene_blank = (0, 20, 400, 280) → 中心 (200, 160)
        assert!((逻辑.0 - 200.0).abs() < 1e-3, "实际 {逻辑:?}");
        assert!((逻辑.1 - 160.0).abs() < 1e-3, "实际 {逻辑:?}");
    }

    #[test]
    fn 行空白区缺失时不退化成点名称() {
        // `blank` 为 null（名称太长）时必须返回 None：退回去点名称会把
        // "空白区不存在"伪装成"右键空白区通过了"。
        let 证据 = 证据(&[], None);
        assert_eq!(用例坐标("row_blank:甲", &证据), None);
    }

    #[test]
    fn 场景树用例判定覆盖新增的两种落点() {
        assert!(是场景树用例("row_blank:植被"));
        assert!(是场景树用例("scene_blank"));
        assert!(是场景树用例("arrow_name:乔木"));
        assert!(!是场景树用例("click:viewport"));
    }
    // ---- 操纵器：坐标推导 / 变换分量断言 / 事务断言 ----

    /// 造一份带操纵器部件与变换的证据。
    fn 操纵器证据() -> serde_json::Value {
        serde_json::json!({
            "pixels_per_point": 2.0,
            "window": {"width": 1600, "height": 1000},
            "transform_tool": "move",
            "gizmo_hover": "move_x",
            "gizmo_drag": null,
            "undo_steps": 3,
            "document_dirty": false,
            "gizmo_parts": [
                {
                    "part": "move_x",
                    // 物理像素 (400, 300) → 逻辑 (200, 150)
                    "hit": [400, 300],
                    "dir": [1.0, 0.0],
                    "bbox": {"x": 300, "y": 290, "w": 120, "h": 20},
                },
                {
                    "part": "rotate_z",
                    "hit": [400, 500],
                    "dir": [0.0, -1.0],
                    "bbox": {"x": 300, "y": 400, "w": 200, "h": 200},
                }
            ],
            "selection_transform": {
                "entity": "实体标识(甲)",
                "translate": [10.0, 20.0, 0.0],
                "rotation": [0.0, 0.0, 0.0, 1.0],
                "rotation_deg_z": 0.0,
                "scale": [1.0, 1.0, 1.0],
            },
        })
    }

    #[test]
    fn 操纵器用例按实测几何取落点() {
        let 证据 = 操纵器证据();
        let (起, 终) = 用例坐标("gizmo:move_x", &证据).expect("应当取到落点");
        // 物理 400/2 = 逻辑 200
        assert_eq!(起, (200.0, 150.0));
        assert_eq!(终, 起, "`gizmo:` 是纯落点，终点与起点相同");
    }

    #[test]
    fn 操纵器拖拽沿实测方向延伸() {
        let 证据 = 操纵器证据();
        let (起, 终) = 用例坐标("gizmo-drag:move_x:60", &证据).expect("应当取到拖拽坐标");
        assert_eq!(起, (200.0, 150.0));
        assert!((终.0 - 260.0).abs() < 1e-4, "应沿 +x 拖 60px，实际 {终:?}");
        assert!((终.1 - 150.0).abs() < 1e-4);

        // 3D 里方向来自投影：这里验证"方向是证据给的，而不是写死的右方向"。
        let (起, 终) = 用例坐标("gizmo-drag:rotate_z:30", &证据).expect("应当取到拖拽坐标");
        assert_eq!(起, (200.0, 250.0));
        assert!((终.0 - 200.0).abs() < 1e-4, "方向是 (0,-1)，x 不该变");
        assert!((终.1 - 220.0).abs() < 1e-4, "应沿 -y 拖 30px，实际 {终:?}");
    }

    #[test]
    fn 未知操纵器部件取不到坐标() {
        assert_eq!(用例坐标("gizmo:nope", &操纵器证据()), None);
        assert_eq!(用例坐标("gizmo-drag:move_x:不是数字", &操纵器证据()), None);
    }

    #[test]
    fn 变换分量断言能区分变化与不变() {
        let 前 = 操纵器证据();
        let mut 后 = 操纵器证据();
        后["selection_transform"]["translate"] = serde_json::json!([70.0, 20.0, 0.0]);

        // 平移变了 → 通过
        assert!(判定变换分量(&前, &后, &["translate".into()], &[]).is_ok());
        // 缩放与旋转没变 → 通过
        assert!(判定变换分量(&前, &后, &[], &["scale".into(), "rotation".into()]).is_ok());
        // 断言平移不变 → 必须失败（这就是"未操作轴保持不变"的守门测试）
        let 错误 = 判定变换分量(&前, &后, &[], &["translate".into()])
            .expect_err("平移变了却断言不变，必须失败");
        assert!(错误.contains("保持不变"), "{错误}");
        // 断言缩放变化 → 必须失败
        assert!(判定变换分量(&前, &后, &["scale".into()], &[]).is_err());
    }

    #[test]
    fn 变换分量断言在缺少字段时给出明确原因() {
        let 前 = serde_json::json!({"selection_transform": null});
        let 后 = 操纵器证据();
        let 错误 =
            判定变换分量(&前, &后, &["translate".into()], &[]).expect_err("缺少变换时必须报错");
        assert!(错误.contains("没有变换分量"), "{错误}");
    }

    #[test]
    fn 撤销步数断言按增量核对() {
        let 前 = 操纵器证据();
        let mut 后 = 操纵器证据();
        // 恰好 +1 → 通过（一次拖拽一条记录）
        assert!(判定撤销步数(&前, &后, 0).is_ok());
        后["undo_steps"] = serde_json::json!(4);
        assert!(判定撤销步数(&前, &后, 1).is_ok());
        // 实际 +1 却期望 0 → 失败
        assert!(判定撤销步数(&前, &后, 0).is_err());
        // 实际 +1 却期望 2 → 失败
        assert!(判定撤销步数(&前, &后, 2).is_err());
    }

    #[test]
    fn 脏状态断言接受常见写法() {
        let 证据 = 操纵器证据();
        assert!(判定脏状态(&证据, None).is_ok());
        assert!(判定脏状态(&证据, Some("no")).is_ok());
        assert!(判定脏状态(&证据, Some("yes")).is_err());
        let mut 脏 = 操纵器证据();
        脏["document_dirty"] = serde_json::json!(true);
        assert!(判定脏状态(&脏, Some("yes")).is_ok());
        assert!(判定脏状态(&脏, Some("乱七八糟")).is_err());
    }

    #[test]
    fn 工具与悬停断言核对代号() {
        let 证据 = 操纵器证据();
        assert!(判定工具(&证据, Some("move")).is_ok());
        assert!(判定工具(&证据, Some("rotate")).is_err());
        assert!(判定悬停(&证据, Some("move_x")).is_ok());
        assert!(判定悬停(&证据, Some("none")).is_err());
        // `none` 期望在没有悬停时成立。
        let mut 无悬停 = 操纵器证据();
        无悬停["gizmo_hover"] = serde_json::Value::Null;
        assert!(判定悬停(&无悬停, Some("none")).is_ok());
    }

    #[test]
    fn 状态指纹包含工具操纵器与事务字段() {
        let 指纹 = 状态指纹(&操纵器证据());
        for 键 in [
            "transform_tool",
            "gizmo_hover",
            "gizmo_drag",
            "selection_transform",
            "undo_steps",
            "document_dirty",
        ] {
            assert!(指纹.contains_key(键), "状态指纹缺少 `{键}`");
        }
        // 变换被展开成可读文本，位置变化必须体现出来。
        let mut 后 = 操纵器证据();
        后["selection_transform"]["translate"] = serde_json::json!([11.0, 20.0, 0.0]);
        assert_ne!(
            指纹.get("selection_transform"),
            状态指纹(&后).get("selection_transform")
        );
    }

    #[test]
    fn 拖拽取消用例复用拖拽坐标() {
        let 证据 = 操纵器证据();
        // 取消用例的名字前缀交给调用方改成 drag 再解析，这里验证等价性。
        let 拖拽 = 用例坐标("gizmo-drag:move_x:40", &证据);
        let 取消 = 用例坐标("gizmo-drag:move_x:40", &证据);
        assert_eq!(拖拽, 取消);
    }
}
