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
            "--dry-run" => 空跑 = true,
            "--case" => 用例 = 取值(&mut 下标),
            "--window" => 窗口尺寸 = 取值(&mut 下标),
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
    if 从.is_none() && 分割线.is_none() && 用例.is_none() {
        eprintln!("drag 需要 --from/--to、--splitter v|h 或 --case <name>");
        return 4;
    }
    if let Err(错误) = std::fs::create_dir_all(&输出目录) {
        eprintln!("无法创建输出目录：{错误}");
        return 4;
    }
    let 第二帧 = 第二帧.unwrap_or(第一帧 + 120);
    let 前图 = 输出目录.join(format!("{名称}_before.png"));
    let 前证 = 输出目录.join(format!("{名称}_before.json"));
    let 后图 = 输出目录.join(format!("{名称}_after.png"));
    let 后证 = 输出目录.join(format!("{名称}_after.json"));
    let 日志 = 输出目录.join(format!("{名称}.drag.log"));
    for 路径 in [&前图, &前证, &后图, &后证] {
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
        let _ = 等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第一帧证据不可读", 3);
    };
    if !第一帧就绪 {
        let _ = 等待退出(&mut 子进程, Duration::from_secs(5));
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第一帧采集超时", 3);
    }
    let Some(窗口) = 窗口 else {
        let _ = 等待退出(&mut 子进程, Duration::from_secs(5));
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
                let _ = 等待退出(&mut 子进程, Duration::from_secs(5));
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
                let _ = 等待退出(&mut 子进程, Duration::from_secs(5));
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

    // 聚焦目标窗口：不聚焦时 SendInput 的点击会落到别的窗口上。
    //
    // `SetForegroundWindow` 在 Windows 上可能被前台锁拒绝（返回 0）。
    // 这时点一下**标题栏**：它不进入客户区，不会误触 UI，但足以把窗口激活。
    确保前台(&窗口);
    std::thread::sleep(Duration::from_millis(250));

    let 起点屏幕 = 转屏幕(从逻辑);
    let 终点屏幕 = 转屏幕(到逻辑);
    eprintln!(
        "注入：逻辑 {:?}→{:?}，屏幕 {:?}→{:?}（ppp={ppp:.2}，客户区原点={原点:?}）",
        从逻辑, 到逻辑, 起点屏幕, 终点屏幕
    );
    let 注入 = if 空跑 {
        // 负向对照：完全不动鼠标，用来证明工具会把"没有变化"判为输入未生效，
        // 而不是把 Dock 判成坏的。
        input::注入结果 {
            移动成功: true,
            按下成功: true,
            抬起成功: true,
            移动步数: 0,
        }
    } else {
        input::执行拖拽(input::拖拽 {
            从: 起点屏幕,
            到: 终点屏幕,
            步数: 24,
            步间隔: Duration::from_millis(28),
            预热: !空跑,
        })
    };

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
    等待退出(&mut 子进程, Duration::from_secs(10));
    let 日志尾部 = 读取日志尾部(&日志, 80);

    if !第二帧就绪 {
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第二帧采集失败", 3);
    }
    let Ok(后证据) = 读取证据(&后证) else {
        return 拖拽失败报告(&输出目录, &名称, "capture_failed", "第二帧证据不可读", 3);
    };
    if !注入.完整() {
        return 拖拽失败报告(
            &输出目录,
            &名称,
            "input_failed",
            &format!("SendInput 阶段失败：{}", 注入.摘要()),
            3,
        );
    }

    let 变化 = 比较区域(&前证据, &后证据);
    let 收到指针 = 日志尾部.contains("[输入诊断]");

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
        &期望区域,
        期望不变,
    );

    let 满足 = if 期望不变 {
        变化.is_empty()
    } else {
        !变化.is_empty()
    };
    let 期望命中 = 期望区域
        .as_deref()
        .map(|区域| 变化.iter().any(|项| 项 == 区域))
        .unwrap_or(true);

    if !满足 {
        eprintln!("输入未产生布局变化（received_pointer={收到指针}）");
        // 指针事件确实到达 egui 但布局完全没变 → 这是真实 UI 缺陷；
        // 否则只能判定为输入/环境失败，不能冤枉 Dock。
        return if 收到指针 { 2 } else { 3 };
    }
    if !期望命中 {
        eprintln!("布局发生了变化，但期望区域 `{期望区域:?}` 未变化");
        return 2;
    }
    0
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
                    let h = 项.get("h")?.as_f64()? as f32 / ppp;
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
    期望区域: &Option<String>,
    期望不变: bool,
) {
    let 报告 = serde_json::json!({
        "schema": 1,
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
        "changed_regions": 变化,
        "expect_region": 期望区域,
        "expect_no_change": 期望不变,
    });
    let _ = std::fs::write(
        输出目录.join(format!("{名称}.drag.json")),
        serde_json::to_string_pretty(&报告).unwrap_or_default(),
    );
    let 行: Vec<String> = 变化.iter().map(|项| format!("- `{项}`")).collect();
    let 文本 = format!(
        "# 真实鼠标拖拽：{名称}\n\n\
         - 窗口：{}（{}），DPI {}\n\
         - 客户区：{}×{}，原点 {:?}\n\
         - pixels_per_point：{ppp:.3}\n\
         - 逻辑坐标：{:?} → {:?}\n\
         - 屏幕坐标：{:?} → {:?}\n\
         - 注入：{}\n\
         - 应用收到指针事件：{}\n\
         - 变化区域（{}）：\n{}\n",
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
        }
    );
    let _ = std::fs::write(输出目录.join(format!("{名称}.drag.md")), 文本);
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

/// 预置用例：把"要拖哪里"写成可复用的坐标推导。
///
/// 坐标全部相对客户区逻辑像素；标签条上的落点取面板左上角附近，
/// egui_dock 的标签命中区从面板顶边开始约 24 逻辑像素高。
fn 用例坐标(名: &str, 证据: &serde_json::Value) -> Option<((f32, f32), (f32, f32))> {
    let 表 = 区域表(证据);
    let 取 = |id: &str| 表.get(id).copied();
    let 标签 = |id: &str| 取(id).map(|(x, y, _, _)| (x + 40.0, y - 14.0));
    match 名 {
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
            // egui_dock 在指针离开 DockArea 时把拖动中的标签变成浮动表面。
            // 菜单栏会被菜单按钮占用，工具栏中段是空白区，更适合作为落点。
            let 起 = 标签("asset_browser")?;
            let (_, _, _, 状态栏y) = 取("statusbar")?;
            let _ = 状态栏y;
            let (_, 工具栏y, _, 工具栏h) = 取("toolbar")?;
            Some((起, (900.0, 工具栏y + 工具栏h * 0.5)))
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
