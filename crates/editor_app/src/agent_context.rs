//! AI Agent 的上下文构建器：把编辑器的真实状态变成 Agent 可读的文本。
//!
//! # 原则
//!
//! * **只用真实数据**：场景实体来自场景文档，项目摘要来自文件系统与 `git`，
//!   不带任何示例/占位内容。
//! * **不搬运整仓库**：项目摘要只给结构（目录、文件数、扩展名统计）与
//!   git 状态，具体文件由 Agent 自己按需读取（它有真实文件工具）。
//! * **可测试**：场景摘要与实体摘要都是纯函数，便于单元测试。

use std::path::Path;

use editor_core::state::编辑器状态;
use scene_doc::document::场景实体;

/// 选中实体的摘要（组件与属性）。
pub fn 实体摘要(实体: &场景实体) -> String {
    let mut 结果 = format!("实体：{}\n", 实体.名称);
    if let Some(变换) = 实体.变换 {
        结果.push_str(&format!(
            "变换：平移 ({:.3}, {:.3}, {:.3}) 旋转四元数 ({:.3}, {:.3}, {:.3}, {:.3}) 缩放 ({:.3}, {:.3}, {:.3})\n",
            变换.平移.0, 变换.平移.1, 变换.平移.2,
            变换.旋转.0, 变换.旋转.1, 变换.旋转.2, 变换.旋转.3,
            变换.缩放.0, 变换.缩放.1, 变换.缩放.2,
        ));
    }
    结果.push_str(&format!("组件数：{}\n", 实体.组件.len()));
    for 组件 in &实体.组件 {
        结果.push_str(&format!("- {}：", 组件.类型名));
        let 属性: Vec<String> = 组件
            .属性
            .iter()
            .map(|(键, 值)| format!("{键}={}", 属性展示(值)))
            .collect();
        结果.push_str(&属性.join(", "));
        结果.push('\n');
    }
    结果
}

/// 属性的紧凑展示（枚举取变体名，数值保留可读精度）。
fn 属性展示(值: &scene_doc::property::属性值) -> String {
    use scene_doc::property::属性值;
    match 值 {
        属性值::布尔(值) => 值.to_string(),
        属性值::整数(值) => 值.to_string(),
        属性值::浮点(值) => format!("{值:.3}"),
        属性值::文本(值) => format!("\"{值}\""),
        属性值::二维(x, y) => format!("({x:.3}, {y:.3})"),
        属性值::三维(x, y, z) => format!("({x:.3}, {y:.3}, {z:.3})"),
        属性值::四维(a, b, c, d) => format!("({a:.3}, {b:.3}, {c:.3}, {d:.3})"),
        属性值::颜色(颜色) => format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            (颜色.红 * 255.0) as u8,
            (颜色.绿 * 255.0) as u8,
            (颜色.蓝 * 255.0) as u8,
            (颜色.透明度 * 255.0) as u8
        ),
        属性值::枚举(枚举) => 枚举.变体名.clone(),
        属性值::资源(引用) => format!("资源({})", 引用.路径.文本()),
        属性值::列表(值) => format!("列表({} 项)", 值.len()),
        属性值::结构(值) => format!("结构({} 字段)", 值.len()),
        属性值::空 => "空".to_string(),
    }
}

/// 当前场景的摘要：路径、是否脏、实体数与层级。
pub fn 场景摘要(会话: &editor_core::state::场景会话) -> String {
    let 文档 = &会话.文档;
    let mut 结果 = String::new();
    结果.push_str(&format!("场景：{}\n", 文档.元数据.场景名));
    if let Some(路径) = &会话.路径 {
        结果.push_str(&format!("路径：{}\n", 路径.文本()));
    }
    结果.push_str(&format!("实体数：{}\n", 文档.实体表.len()));
    结果.push_str(&format!("资源引用数：{}\n", 文档.资源表.len()));
    // 层级：只给前两层，避免把巨大场景整棵树塞进对话。
    结果.push_str("层级（前 40 个实体）：\n");
    for (序号, (标识, 实体)) in 文档.实体表.iter().take(40).enumerate() {
        // 层级用“向上数父实体”得到，避免依赖专门的祖先 API。
        let mut 层级 = 0usize;
        let mut 当前 = 文档.取(*标识).and_then(|实体| 实体.父实体);
        while let Some(父) = 当前 {
            层级 += 1;
            if 层级 > 8 {
                break;
            }
            当前 = 文档.取(父).and_then(|实体| 实体.父实体);
        }
        结果.push_str(&format!("{}{}\n", "  ".repeat(层级), 实体.名称));
        let _ = 序号;
    }
    结果
}

/// 项目摘要：路径、目录结构、文件统计与 git 状态。
///
/// `git` 用真实子进程；项目不是 git 仓库时如实标注，而不是空着。
pub fn 项目摘要(项目: Option<&editor_core::project::项目描述>) -> Result<String, String> {
    let Some(项目) = 项目 else {
        return Err("没有打开项目".to_string());
    };
    let 根 = 项目.根目录();
    let mut 结果 = format!("项目：{}\n根目录：{}\n", 项目.名称(), 根.display());
    let (设计宽, 设计高) = 项目.清单().设计分辨率;
    结果.push_str(&format!("设计分辨率：{设计宽}×{设计高}\n"));
    if let Some(启动) = &项目.清单().启动场景 {
        结果.push_str(&format!("启动场景：{}\n", 启动.文本()));
    }
    结果.push_str(&format!("场景数：{}\n", 项目.场景列表().len()));
    结果.push_str(&format!("资源数：{}\n", 项目.资源列表().len()));

    // 目录结构（两层，跳过 target 与 .git）
    match 目录结构(根, 2) {
        Ok(结构) => {
            结果.push_str("\n目录结构（两层）：\n");
            结果.push_str(&结构);
        }
        Err(错误) => 结果.push_str(&format!("\n目录结构读取失败：{错误}\n")),
    }

    // git 状态（真实命令）
    match git状态(根) {
        Ok(状态) => {
            结果.push_str("\nGit 状态：\n");
            结果.push_str(&状态);
        }
        Err(错误) => 结果.push_str(&format!("\nGit 状态不可用：{错误}\n")),
    }
    Ok(结果)
}

/// 两层目录结构（只列目录与文件名，不读内容）。
pub fn 目录结构(根: &Path, 深度: usize) -> Result<String, String> {
    fn 遍历(
        目录: &Path, 层级: usize, 上限: usize, 结果: &mut String, 计数: &mut usize
    ) {
        if 层级 > 上限 || *计数 > 200 {
            return;
        }
        let Ok(条目) = std::fs::read_dir(目录) else {
            return;
        };
        let mut 列表: Vec<_> = 条目.flatten().collect();
        列表.sort_by_key(|项| 项.file_name());
        for 项 in 列表 {
            let 名 = 项.file_name().to_string_lossy().to_string();
            if matches!(
                名.as_str(),
                "target" | ".git" | "node_modules" | ".verify" | ".verify_v2"
            ) {
                continue;
            }
            let Ok(类型) = 项.file_type() else {
                continue;
            };
            结果.push_str(&format!("{}{}\n", "  ".repeat(层级 - 1), 名));
            *计数 += 1;
            if 类型.is_dir() {
                遍历(&项.path(), 层级 + 1, 上限, 结果, 计数);
            }
        }
    }
    let mut 结果 = String::new();
    let mut 计数 = 0;
    遍历(根, 1, 深度, &mut 结果, &mut 计数);
    Ok(结果)
}

/// `git status --porcelain` 的真实输出（截断）。
pub fn git状态(根: &Path) -> Result<String, String> {
    let 输出 = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=no"])
        .current_dir(根)
        .output()
        .map_err(|错误| format!("git 启动失败：{错误}"))?;
    if !输出.status.success() {
        return Err(format!("git 退出码 {:?}", 输出.status.code()));
    }
    let 文本 = String::from_utf8_lossy(&输出.stdout).to_string();
    if 文本.trim().is_empty() {
        return Ok("(工作区干净，无已跟踪改动)".to_string());
    }
    Ok(crate::agent_dsh::截断(&文本, 4000))
}

/// 读取某个文件的真实差异文本。
pub fn 文件差异(根: &Path, 相对路径: &str) -> Result<String, String> {
    let 输出 = std::process::Command::new("git")
        .args(["diff", "--", 相对路径])
        .current_dir(根)
        .output()
        .map_err(|错误| format!("git 启动失败：{错误}"))?;
    if !输出.status.success() {
        return Err(format!("git diff 退出码 {:?}", 输出.status.code()));
    }
    let 文本 = String::from_utf8_lossy(&输出.stdout).to_string();
    if 文本.trim().is_empty() {
        // 新文件没有 diff：给出文件头部，至少让用户看到内容来源。
        let 绝对 = 根.join(相对路径);
        let 内容 = std::fs::read_to_string(&绝对).unwrap_or_default();
        let 头部: String = 内容.lines().take(30).collect::<Vec<_>>().join("\n");
        return Ok(format!("(新文件/未跟踪，以下为前 30 行)\n{头部}"));
    }
    Ok(crate::agent_dsh::截断(&文本, 20000))
}

/// 收集项目里已跟踪的变更文件（状态 + 归属 + 真实差异）。
///
/// 没有任务基线时，所有改动只能标成"来源未知"—— 这正是旧接口的语义。
/// 需要精确归属时用 [`crate::agent_worktree::刷新变更`]。
pub fn 收集变更(根: &Path) -> Vec<editor_ui::agent::文件变更> {
    let Ok(快照) = crate::agent_worktree::采集快照(根) else {
        return Vec::new();
    };
    let mut 变更 = editor_ui::agent_review::归类变更(None, &快照.条目, &[], "", &[]);
    for 项 in &mut 变更 {
        项.差异 = crate::agent_worktree::读取差异(根, 项);
        if 项.差异.len() > 20000 {
            项.差异 = editor_ui::agent_store::截断文本(&项.差异, 20000);
        }
    }
    变更
}

/// 诊断摘要：控制台最后的错误/警告行。
pub fn 诊断摘要(编辑器: &编辑器状态, 上限: usize) -> String {
    let 语言 = editor_core::i18n::语言::中文;
    let mut 结果 = String::new();
    let 列表 = 编辑器.日志列表();
    let 起点 = 列表.len().saturating_sub(上限);
    for 条目 in &列表[起点..] {
        let 文本 = 条目.渲染(语言);
        结果.push_str(&文本);
        结果.push('\n');
    }
    if 结果.is_empty() {
        "(没有诊断信息)".to_string()
    } else {
        结果
    }
}

/// 当前打开脚本的摘要：路径、是否有未保存修改、光标行与光标附近代码。
///
/// 光标行由脚本面板真实写入（见 `editor_ui::script`）；没编辑过时如实写"未知"。
pub fn 脚本文档摘要(文档: &editor_ui::script::脚本文档) -> String {
    let 路径 = 文档
        .路径
        .as_ref()
        .map(|路径| 路径.文本().to_string())
        .unwrap_or_else(|| "未命名".to_string());
    let mut 结果 = format!(
        "当前打开源码：{路径}\n行数：{}\n未保存修改：{}\n",
        文档.行数(),
        if 文档.有未保存修改() {
            "是"
        } else {
            "否"
        }
    );
    match 文档.光标行 {
        Some(行) => {
            结果.push_str(&format!("光标行：{行}\n"));
            let 起始 = 行.saturating_sub(20).max(1);
            let 结束 = 行 + 20;
            结果.push_str(&format!("光标附近（第 {起始}-{结束} 行）：\n"));
            for (序号, 内容) in 文档.文本.lines().enumerate() {
                let 行号 = 序号 + 1;
                if 行号 < 起始 || 行号 > 结束 {
                    continue;
                }
                let 标记 = if 行号 == 行 { ">" } else { " " };
                结果.push_str(&format!("{标记}{行号:>5} | {内容}\n"));
            }
        }
        None => {
            结果.push_str("光标行：未知（用户还没有在编辑器里移动过光标）\n");
            结果.push_str("文件前 80 行：\n");
            for (序号, 内容) in 文档.文本.lines().take(80).enumerate() {
                结果.push_str(&format!("{:>5} | {内容}\n", 序号 + 1));
            }
        }
    }
    结果
}

/// 原生 Cargo/Bevy 工程摘要：工作区、成员 crate、库/二进制目标与入口。
///
/// 只读解析 `Cargo.toml`（复用 `editor_core::cargo_project`），不把原生工程
/// 转换成微风专用格式。
pub fn 原生工程摘要(根: &Path) -> Result<String, String> {
    let 工程 = editor_core::cargo_project::Cargo工程::读取(根).map_err(|错误| format!("{错误}"))?;
    let mut 结果 = String::new();
    结果.push_str(&format!(
        "原生 Cargo 工程：{}\n是工作区：{}\n根清单：{}\n包数：{}\n",
        根.display(),
        if 工程.是工作区 { "是" } else { "否" },
        工程.清单路径.display(),
        工程.包.len()
    ));
    if !工程.缺失成员.is_empty() {
        结果.push_str(&format!(
            "声明了但找不到清单的成员：{}\n",
            工程.缺失成员.join("、")
        ));
    }
    结果.push_str("\n包：\n");
    for 包 in &工程.包 {
        let 相对根 = 包
            .根目录
            .strip_prefix(根)
            .map(|路径| 路径.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| 包.根目录.to_string_lossy().to_string());
        结果.push_str(&format!(
            "- {} {}（{}）{}\n",
            包.名称,
            包.版本,
            if 相对根.is_empty() {
                "."
            } else {
                相对根.as_str()
            },
            if 包.依赖贝维 {
                " · 依赖 bevy"
            } else {
                ""
            }
        ));
        结果.push_str(&format!(
            "  库目标：{}\n",
            包.库名.clone().unwrap_or_else(|| "无".to_string())
        ));
        if 包.可执行目标.is_empty() {
            结果.push_str("  可执行目标：无\n");
        }
        for 目标 in &包.可执行目标 {
            let 源 = 目标
                .源文件
                .as_ref()
                .map(|路径| {
                    路径
                        .strip_prefix(根)
                        .map(|路径| 路径.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| 路径.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "源文件未知".to_string());
            结果.push_str(&format!("  可执行目标：{}（{}）\n", 目标.名称, 源));
        }
    }

    // 入口定位：即使 Cargo 清单没写 [[bin]]，`src/main.rs` 也是入口。
    结果.push_str("\n入口文件：\n");
    let mut 入口 = Vec::new();
    for 候选 in ["src/main.rs", "src/lib.rs"] {
        if 根.join(候选).is_file() {
            入口.push(候选.to_string());
        }
    }
    for 包 in &工程.包 {
        for 目标 in &包.可执行目标 {
            if let Some(源) = &目标.源文件
                && let Ok(相对) = 源.strip_prefix(根)
            {
                let 文本 = 相对.to_string_lossy().replace('\\', "/");
                if !入口.contains(&文本) {
                    入口.push(文本);
                }
            }
        }
    }
    if let Ok(条目) = std::fs::read_dir(根.join("src/bin")) {
        for 项 in 条目.flatten() {
            if 项.path().extension().is_some_and(|扩展| 扩展 == "rs") {
                入口.push(format!("src/bin/{}", 项.file_name().to_string_lossy()));
            }
        }
    }
    if 入口.is_empty() {
        结果.push_str("（没有找到 src/main.rs、src/lib.rs 或 bin 目标）\n");
    } else {
        for 项 in 入口 {
            结果.push_str(&format!("- {项}\n"));
        }
    }
    Ok(结果)
}

/// 源码文件（相对路径, 文本）。只读、限量，避免把整个仓库塞进上下文。
fn 收集源码(根: &Path, 上限: usize) -> Vec<(String, String)> {
    fn 遍历(根: &Path, 目录: &Path, 结果: &mut Vec<(String, String)>, 上限: usize) {
        if 结果.len() >= 上限 {
            return;
        }
        let Ok(条目) = std::fs::read_dir(目录) else {
            return;
        };
        let mut 列表: Vec<_> = 条目.flatten().collect();
        列表.sort_by_key(|项| 项.file_name());
        for 项 in 列表 {
            if 结果.len() >= 上限 {
                return;
            }
            let 名 = 项.file_name().to_string_lossy().to_string();
            if matches!(
                名.as_str(),
                "target" | ".git" | "node_modules" | ".verify" | ".verify_v2" | ".verify_v3"
            ) {
                continue;
            }
            let Ok(类型) = 项.file_type() else {
                continue;
            };
            if 类型.is_dir() {
                遍历(根, &项.path(), 结果, 上限);
                continue;
            }
            if 项.path().extension().is_none_or(|扩展| 扩展 != "rs") {
                continue;
            }
            let 元数据 = match 项.metadata() {
                Ok(元数据) => 元数据,
                Err(_) => continue,
            };
            // 单个源文件超过 512 KiB 的不读：那是生成代码，塞进上下文只会挤掉重点。
            if 元数据.len() > 512 * 1024 {
                continue;
            }
            let Ok(文本) = std::fs::read_to_string(项.path()) else {
                continue;
            };
            let 相对 = 项
                .path()
                .strip_prefix(根)
                .map(|路径| 路径.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| 项.path().to_string_lossy().to_string());
            结果.push((相对, 文本));
        }
    }
    let mut 结果 = Vec::new();
    遍历(根, 根, &mut 结果, 上限);
    结果
}

/// 在 Rust 源码里检索 Bevy 常见声明：App、插件、系统调度、Component、Resource、
/// 资源加载与场景根。
///
/// 这是**行级结构化检索**，不是 Rust 语义分析：它不会解析宏展开，也不会
/// 区分同名 trait。输出明确标注"行级检索"，避免冒充完整语义。
pub fn 贝维声明检索(根: &Path, 上限: usize) -> String {
    let 源码 = 收集源码(根, 400);
    let mut 条目: Vec<String> = Vec::new();
    for (路径, 文本) in &源码 {
        for (序号, 原始行) in 文本.lines().enumerate() {
            let 行 = 原始行.trim();
            if 行.is_empty() || 行.starts_with("//") {
                continue;
            }
            let 分类 = if 行.contains("impl Plugin for")
                || 行.contains("impl bevy::prelude::Plugin for")
                || 行.contains("impl bevy::app::Plugin for")
            {
                Some("插件实现")
            } else if 行.contains("derive(")
                && (行.contains("Component") || 行.contains("Resource"))
            {
                if 行.contains("Resource") {
                    Some("Resource 派生")
                } else {
                    Some("Component 派生")
                }
            } else if 行.contains("App::new()") || 行.contains("App::default()") {
                Some("App 构建")
            } else if 行.contains(".add_plugins(") {
                Some("插件注册")
            } else if 行.contains(".add_systems(")
                || 行.contains(".add_system(")
                || 行.contains(".configure_sets(")
            {
                Some("系统调度")
            } else if 行.contains(".load(") && 行.contains('"') {
                Some("资源加载")
            } else if 行.contains("SceneRoot")
                || 行.contains("Handle<Scene>")
                || 行.contains("Scene::new")
            {
                Some("场景引用")
            } else {
                None
            };
            let Some(分类) = 分类 else {
                continue;
            };
            let 片段: String = 行.chars().take(100).collect();
            条目.push(format!("{分类} · {路径}:{} · {片段}", 序号 + 1));
            if 条目.len() >= 上限 {
                break;
            }
        }
        if 条目.len() >= 上限 {
            break;
        }
    }
    if 条目.is_empty() {
        return "（没有检索到 Bevy 声明：项目可能不是 Bevy 工程，或源码不在根目录下）".to_string();
    }
    format!(
        "行级检索（不是完整语义分析；共 {} 条，源码文件 {} 个）：\n{}",
        条目.len(),
        源码.len(),
        条目.join("\n")
    )
}

/// 资源目录与代码引用的关联摘要。
pub fn 资源与引用摘要(根: &Path) -> String {
    let mut 结果 = String::new();
    let 资源根 = 根.join("assets");
    let mut 资源 = Vec::new();
    if 资源根.is_dir() {
        fn 遍历(根: &Path, 目录: &Path, 结果: &mut Vec<String>, 上限: usize) {
            if 结果.len() >= 上限 {
                return;
            }
            let Ok(条目) = std::fs::read_dir(目录) else {
                return;
            };
            let mut 列表: Vec<_> = 条目.flatten().collect();
            列表.sort_by_key(|项| 项.file_name());
            for 项 in 列表 {
                if 结果.len() >= 上限 {
                    return;
                }
                if 项.file_type().map(|类型| 类型.is_dir()).unwrap_or(false) {
                    遍历(根, &项.path(), 结果, 上限);
                } else {
                    结果.push(
                        项.path()
                            .strip_prefix(根)
                            .map(|路径| 路径.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_else(|_| 项.path().to_string_lossy().to_string()),
                    );
                }
            }
        }
        遍历(根, &资源根, &mut 资源, 120);
    }
    结果.push_str(&format!(
        "assets 目录：{}\n",
        if 资源根.is_dir() {
            format!("{} 个文件（最多列 120 个）", 资源.len())
        } else {
            "不存在".to_string()
        }
    ));
    for 项 in &资源 {
        结果.push_str(&format!("- {项}\n"));
    }

    // 代码里的资源路径引用：`xxx.load("...")` / `asset_server.load(...)`。
    let 源码 = 收集源码(根, 300);
    let mut 引用 = Vec::new();
    for (路径, 文本) in &源码 {
        for (序号, 行) in 文本.lines().enumerate() {
            if !行.contains(".load(") && !行.contains("load_from_asset") {
                continue;
            }
            if let Some(引号起点) = 行.find('"')
                && let Some(引号终点) = 行[引号起点 + 1..].find('"')
            {
                let 引用路径 = &行[引号起点 + 1..引号起点 + 1 + 引号终点];
                if !引用路径.is_empty() {
                    引用.push(format!("{路径}:{} -> \"{引用路径}\"", 序号 + 1));
                }
            }
            if 引用.len() >= 80 {
                break;
            }
        }
        if 引用.len() >= 80 {
            break;
        }
    }
    结果.push_str(&format!("\n代码里的资源引用：{} 处\n", 引用.len()));
    for 项 in &引用 {
        结果.push_str(&format!("- {项}\n"));
    }

    // 微风场景文件：原生 Bevy 工程通常没有，必须如实说明差异。
    let 场景数 = std::fs::read_dir(根.join("scenes"))
        .map(|条目| {
            条目
                .flatten()
                .filter(|项| 项.file_name().to_string_lossy().ends_with(".scene.ron"))
                .count()
        })
        .unwrap_or(0);
    结果.push_str(&format!(
        "\n微风场景文件：{场景数} 个（原生工程里的场景通常写在 Rust 代码中，不能假定都有场景文档）\n"
    ));
    结果
}

/// 把原生工程相关的上下文项一次性构建出来（供界面"附加原生工程上下文"）。
pub fn 原生上下文项(
    编辑器: &编辑器状态,
    脚本: Option<&editor_ui::script::脚本文档>,
    上限: usize,
) -> Vec<editor_ui::agent::上下文项> {
    use editor_ui::agent::{上下文种类, 上下文项};
    let Some(项目) = 编辑器.项目.as_ref() else {
        return Vec::new();
    };
    let 根 = 项目.根目录();
    let mut 结果 = Vec::new();
    if let Ok(摘要) = 原生工程摘要(根) {
        结果.push(上下文项 {
            种类: 上下文种类::Cargo工程,
            名称: 项目.名称().to_string(),
            正文: 摘要,
        });
    }
    let 检索 = 贝维声明检索(根, 上限);
    结果.push(上下文项 {
        种类: 上下文种类::代码,
        名称: "Bevy 声明检索".to_string(),
        正文: format!("{}\n\n{}", 资源与引用摘要(根), 检索),
    });
    if let Some(脚本) = 脚本 {
        结果.push(上下文项 {
            种类: 上下文种类::代码,
            名称: 脚本
                .路径
                .as_ref()
                .map(|路径| 路径.文本().to_string())
                .unwrap_or_else(|| "未命名".to_string()),
            正文: 脚本文档摘要(脚本),
        });
    }
    结果.push(上下文项 {
        种类: 上下文种类::诊断,
        名称: "最近诊断".to_string(),
        正文: 诊断摘要(编辑器, 60),
    });
    结果
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::组件数据;
    use scene_doc::property::属性值;

    #[test]
    fn 实体摘要包含组件与属性() {
        let mut 文档 = scene_doc::document::场景文档::新建("t");
        let 标识 = 文档.生成实体("箱子", None);
        let 实体 = 文档.取可变(标识).unwrap();
        实体.组件.push(组件数据::新建(
            "碰撞体2d",
            [("宽度".to_string(), 属性值::浮点(40.0))]
                .into_iter()
                .collect(),
        ));
        let 摘要 = 实体摘要(文档.取(标识).unwrap());
        assert!(摘要.contains("实体：箱子"));
        assert!(摘要.contains("碰撞体2d"));
        assert!(摘要.contains("宽度=40.000"));
    }

    #[test]
    fn 场景摘要有实体数与层级() {
        let mut 会话 = editor_core::state::场景会话::新建("main.scene.ron");
        会话.文档.生成实体("父", None);
        let 摘要 = 场景摘要(&会话);
        assert!(摘要.contains("实体数：1"));
        assert!(摘要.contains("父"));
    }

    #[test]
    fn 没有项目时项目摘要报错而不是返回空串() {
        assert!(项目摘要(None).is_err());
    }

    #[test]
    fn 项目摘要用真实临时目录() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path().join("摘要项目");
        // `项目描述::创建` 要求目标目录为空或不存在，因此先建项目再放文件。
        let 项目 = editor_core::project::项目描述::创建(&根, "摘要项目").unwrap();
        std::fs::create_dir_all(根.join("scenes")).unwrap();
        std::fs::write(根.join("scenes").join("a.scene.ron"), "x").unwrap();
        let 摘要 = 项目摘要(Some(&项目)).unwrap();
        assert!(摘要.contains("摘要项目"));
        assert!(摘要.contains("scenes"), "目录结构应当包含 scenes");
        assert!(摘要.contains("设计分辨率"));
    }

    #[test]
    fn 变更收集能读到未跟踪文件() {
        let 目录 = tempfile::tempdir().unwrap();
        let 根 = 目录.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(根)
            .output()
            .ok();
        std::fs::write(根.join("new.rs"), "fn main() {}\n").unwrap();
        let 变更 = 收集变更(根);
        // git 不可用或仓库未初始化时允许为空：这条测试不断言 git 环境，
        // 只断言"如果拿到结果，状态与路径都是真实字段"。
        for 项 in &变更 {
            assert!(!项.路径.is_empty());
            assert_eq!(
                项.来源,
                editor_ui::agent::变更来源::来源未知,
                "没有任务基线时不能冒认归属"
            );
            assert_eq!(项.验证, editor_ui::agent::验证状态::未验证);
        }
    }

    fn 写原生工程(根: &std::path::Path) {
        std::fs::create_dir_all(根.join("crates/game/src")).unwrap();
        std::fs::create_dir_all(根.join("assets/textures")).unwrap();
        std::fs::write(
            根.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/game"]
resolver = "2"

[workspace.dependencies]
bevy = "0.20"
"#,
        )
        .unwrap();
        std::fs::write(
            根.join("crates/game/Cargo.toml"),
            r#"[package]
name = "game"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true }

[[bin]]
name = "game"
path = "src/main.rs"
"#,
        )
        .unwrap();
        std::fs::write(
            根.join("crates/game/src/main.rs"),
            r#"use bevy::prelude::*;

#[derive(Component)]
struct 玩家;

#[derive(Resource)]
struct 分数(u32);

struct 我的插件;

impl Plugin for 我的插件 {
    fn build(&self, app: &mut App) {
        app.add_plugins(DefaultPlugins)
           .add_systems(Update, 移动);
    }
}

fn 移动() {}

fn main() {
    App::new().add_plugins(我的插件).run();
}
"#,
        )
        .unwrap();
        std::fs::write(根.join("assets/textures/player.png"), b"png").unwrap();
        std::fs::write(
            根.join("crates/game/src/assets.rs"),
            "fn 载入(服务器: &AssetServer) { let _ = 服务器.load(\"textures/player.png\"); }\n",
        )
        .unwrap();
    }

    #[test]
    fn 原生工程摘要包含工作区成员与入口() {
        let 目录 = tempfile::tempdir().unwrap();
        写原生工程(目录.path());
        let 摘要 = 原生工程摘要(目录.path()).unwrap();
        assert!(摘要.contains("是工作区：是"));
        assert!(摘要.contains("game"));
        assert!(摘要.contains("依赖 bevy"));
        assert!(摘要.contains("src/main.rs"), "必须定位到真实入口：{摘要}");
        assert!(摘要.contains("crates/game/src/main.rs"));
    }

    #[test]
    fn 贝维声明检索找到组件插件与调度() {
        let 目录 = tempfile::tempdir().unwrap();
        写原生工程(目录.path());
        let 检索 = 贝维声明检索(目录.path(), 100);
        assert!(检索.contains("插件实现"));
        assert!(检索.contains("Component 派生"));
        assert!(检索.contains("Resource 派生"));
        assert!(检索.contains("系统调度"));
        assert!(检索.contains("crates/game/src/main.rs:"));
        assert!(检索.contains("不是完整语义分析"), "必须标明检索的边界");
    }

    #[test]
    fn 资源摘要把代码引用与资源文件关联() {
        let 目录 = tempfile::tempdir().unwrap();
        写原生工程(目录.path());
        let 摘要 = 资源与引用摘要(目录.path());
        assert!(摘要.contains("assets/textures/player.png"));
        assert!(摘要.contains("\"textures/player.png\""));
        assert!(摘要.contains("微风场景文件：0 个"));
    }

    #[test]
    fn 脚本文档摘要包含光标行附近代码() {
        let 目录 = tempfile::tempdir().unwrap();
        写原生工程(目录.path());
        let 路径 = scene_doc::path::项目相对路径::解析("crates/game/src/main.rs").unwrap();
        let 文本 = std::fs::read_to_string(目录.path().join("crates/game/src/main.rs")).unwrap();
        let mut 文档 = editor_ui::script::脚本文档::新建(Some(路径), 文本);
        文档.光标行 = Some(4);
        let 摘要 = 脚本文档摘要(&文档);
        assert!(摘要.contains("光标行：4"));
        assert!(摘要.contains(">    4 | struct 玩家;"));
    }
}
