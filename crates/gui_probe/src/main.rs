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
