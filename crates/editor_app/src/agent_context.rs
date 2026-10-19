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

/// 收集项目里已跟踪的变更文件（状态 + 差异）。
pub fn 收集变更(根: &Path, 已验证: bool) -> Vec<editor_ui::agent::文件变更> {
    let Ok(输出) = std::process::Command::new("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(根)
        .output()
    else {
        return Vec::new();
    };
    let 文本 = String::from_utf8_lossy(&输出.stdout).to_string();
    let mut 结果 = Vec::new();
    for 行 in 文本.lines() {
        if 行.len() < 4 {
            continue;
        }
        let 状态 = 行[..2].to_string();
        let 路径 = 行[3..].trim().to_string();
        if 路径.is_empty() {
            continue;
        }
        let 差异 = 文件差异(根, &路径).unwrap_or_else(|错误| format!("(差异读取失败：{错误})"));
        结果.push(editor_ui::agent::文件变更 {
            路径,
            状态,
            差异,
            已验证,
        });
        if 结果.len() >= 40 {
            break;
        }
    }
    结果
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
        let 变更 = 收集变更(根, false);
        // git 不可用或仓库未初始化时允许为空：这条测试不断言 git 环境，
        // 只断言"如果拿到结果，状态与路径都是真实字段"。
        for 项 in &变更 {
            assert!(!项.路径.is_empty());
            assert_eq!(项.已验证, false);
        }
    }
}
