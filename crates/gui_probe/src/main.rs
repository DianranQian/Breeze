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
    let mut 期望状态变化: Vec<String> = Vec::new();
    let mut 用例2: Option<String> = None;
    let mut 用例2延迟毫秒: u64 = 1500;
    let mut 期望还原 = false;
    // 只验滚轮是否真的改变了滚动位置（不要求区域/状态发生变化）。
    let mut 期望滚动 = false;
    // 插槽 1 里的滚轮：在场景树等可滚动区域上真实发滚轮事件。
    let mut 滚轮格数: i32 = 0;
    let mut 滚轮区域: Option<String> = None;
    // 插槽 2 里的点击（可多个，逗号分隔）：坐标取自**第二帧证据**，
    // 因此适用于"滚动之后"的几何。
    let mut 用例3: Vec<String> = Vec::new();
    let mut 第三帧帧号: Option<u32> = None;

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
    if 从.is_none() && 分割线.is_none() && 用例.is_none() && 用例3.is_empty() && 滚轮格数 == 0
    {
        eprintln!(
            "drag 需要 --from/--to、--splitter v|h、--case <name>、--case3 <name[,name]> 或 --scroll <n>"
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
    } // if !需要第一插槽点击

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
            let Some((从3, _到3)) = 用例坐标(名, &后证据) else {
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
            eprintln!("插槽 2 单击 `{名}`：逻辑 {从3:?}，屏幕 {屏3:?}");
            插槽2点击.push(serde_json::json!({
                "case": 名,
                // 逻辑坐标 = 第二帧证据里该控件矩形的中心
                "logical": [从3.0, 从3.1],
                "screen": [屏3.0, 屏3.1],
                // 这份几何来自第二帧（滚动后、点击前）
                "geometry_from": "after",
            }));
            let _ = input::执行单击(屏3);
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
        &期望区域,
        &期望状态变化,
        期望不变,
        期望还原,
        期望滚动,
        滚轮格数,
        滚动结果,
        &插槽2点击,
    );

    // 判定：布局区域变化**或**结构化状态变化都算"点击产生了效果"。
    //
    // 只比区域矩形是不够的：点「折叠箭头」不会移动任何面板，
    // 点「运行项目」也不会。真正的事实是场景树可见行/折叠计数、
    // 运行状态与 PID 的变化 —— 它们才是这两个功能的验收依据。
    let 满足 = if 期望不变 {
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
    名.starts_with("arrow:") || 名.starts_with("arrow_name:") || 名.starts_with("row_name:")
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
    期望区域: &Option<String>,
    期望状态: &[String],
    期望不变: bool,
    期望还原: bool,
    期望滚动: bool,
    滚轮格数: i32,
    滚动结果: (bool, usize),
    插槽2点击: &[serde_json::Value],
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
    });
    let _ = std::fs::write(
        输出目录.join(format!("{名称}.drag.json")),
        serde_json::to_string_pretty(&报告).unwrap_or_default(),
    );
    let 行: Vec<String> = 变化.iter().map(|项| format!("- `{项}`")).collect();
    let 状态行: Vec<String> = 状态变化.iter().map(|项| format!("- `{项}`")).collect();
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
         - 插槽 2 点击（坐标取自第二帧 = 滚动后）：{}\n",
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
        serde_json::to_string(插槽2点击).unwrap_or_default()
    );
    let _ = std::fs::write(输出目录.join(format!("{名称}.drag.md")), 文本);
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

/// 预置用例：把"要拖哪里"写成可复用的坐标推导。
///
/// 坐标全部相对客户区逻辑像素；标签条上的落点取面板左上角附近，
/// egui_dock 的标签命中区从面板顶边开始约 24 逻辑像素高。
fn 用例坐标(名: &str, 证据: &serde_json::Value) -> Option<((f32, f32), (f32, f32))> {
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
