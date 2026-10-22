//! 存档示例：一个**真实运行**的小游戏循环，演示保存 → 修改 → 重新加载恢复。
//!
//! # 它演示的不是"存档 API 能编译"，而是一条完整玩家路径
//!
//! ```text
//! init     玩家初始状态（等级 3、金币 100、背包 [木剑]）→ 写槽位 slot_1
//! play     模拟玩家游玩：等级 +2、金币 +50、捡到 [药水] → 写 slot_2
//! load     重新读 slot_1 → 状态必须**回到**初始值（证明存档真的隔离了内存）
//! verify   读 slot_2 → 必须包含 play 之后的改动（证明两次存档互不覆盖）
//! corrupt  手动写坏主文件 → 必须从备份恢复并标记来源
//! future   写一份版本过新的存档 → 必须拒绝且不删除文件
//! ```
//!
//! # 为什么状态是 JSON 而不是 Rust struct
//!
//! 存档载荷由**游戏**定义。示例故意用 `serde_json::Value`，说明：
//! 背包、等级、任务这些字段不需要进编辑器核心，也不需要提前设计成
//! 某个共享类型；游戏把结构序列化进去，读回来自己解释。
//!
//! # 用法
//!
//! ```text
//! savegame_demo <存档目录> <场景> [--expect-fail]
//! ```
//!
//! 退出码：0 = 场景通过；1 = 断言失败；2 = 参数错误。

use std::path::PathBuf;

use savegame::{存档, 存档仓库, 存档元数据, 存档格式版本};

fn 主函数() -> Result<(), String> {
    let 参数: Vec<String> = std::env::args().skip(1).collect();
    if 参数.len() < 2 {
        return Err("用法：savegame_demo <存档目录> <init|play|load|verify|corrupt|future>".into());
    }
    let 目录 = PathBuf::from(&参数[0]);
    let 场景 = 参数[1].as_str();
    let 仓库 = 存档仓库::新建(&目录);

    match 场景 {
        "init" => 初始状态(&仓库),
        "play" => 游玩一轮(&仓库),
        "load" => 读回初始槽位并验证(&仓库),
        "verify" => 验证两个槽位的内容互不覆盖(&仓库),
        "corrupt" => 主文件损坏后从备份恢复(&仓库),
        "future" => 版本过新的存档被拒绝(&仓库),
        _ => Err(format!("未知场景：{场景}")),
    }
}

/// 玩家初始状态。
fn 初始载荷() -> serde_json::Value {
    serde_json::json!({
        "等级": 3,
        "金币": 100,
        "背包": ["木剑"],
        "位置": { "x": 12.5, "y": -3.0 },
    })
}

fn 写入(
    仓库: &存档仓库, 槽位: &str, 预览: &str, 时间: u64, 载荷: serde_json::Value
) {
    let 存档 = 存档::新建(
        存档元数据 {
            format_version: 存档格式版本,
            game_version: 1,
            slot: 槽位.to_string(),
            saved_at_unix: 时间,
            play_time_seconds: 0.0,
            preview: 预览.to_string(),
        },
        载荷,
    );
    仓库.写入(&存档).expect("写入存档失败");
}

fn 初始状态(仓库: &存档仓库) -> Result<(), String> {
    写入(仓库, "slot_1", "青岚山谷 · 等级 3", 100, 初始载荷());
    println!("init：已写入 slot_1（等级 3、金币 100）");
    Ok(())
}

fn 游玩一轮(仓库: &存档仓库) -> Result<(), String> {
    // 从 slot_1 读档，模拟"玩家接着玩"。
    let 读取 = 仓库
        .读取("slot_1")
        .map_err(|错误| format!("play 读档失败：{错误}"))?;
    let mut 载荷 = 读取.存档.payload.clone();
    let 等级 = 载荷["等级"].as_u64().unwrap_or(0) + 2;
    let 金币 = 载荷["金币"].as_u64().unwrap_or(0) + 50;
    载荷["等级"] = serde_json::json!(等级);
    载荷["金币"] = serde_json::json!(金币);
    if let Some(背包) = 载荷["背包"].as_array_mut() {
        背包.push(serde_json::json!("药水"));
    }
    写入(
        仓库,
        "slot_2",
        &format!("青岚山谷 · 等级 {等级}"),
        200,
        载荷,
    );
    println!("play：已写入 slot_2（等级 {等级}、金币 {金币}、背包 +药水）");
    Ok(())
}

fn 读回初始槽位并验证(仓库: &存档仓库) -> Result<(), String> {
    let 读取 = 仓库
        .读取("slot_1")
        .map_err(|错误| format!("load 读档失败：{错误}"))?;
    let 载荷 = &读取.存档.payload;
    let 等级 = 载荷["等级"].as_u64().unwrap_or(0);
    let 金币 = 载荷["金币"].as_u64().unwrap_or(0);
    let 背包 = 载荷["背包"]
        .as_array()
        .map(|项| 项.iter().filter_map(|值| 值.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    if 等级 != 3 || 金币 != 100 || 背包 != vec!["木剑"] {
        return Err(format!(
            "load 断言失败：期望等级 3 / 金币 100 / 背包 [木剑]，实际 {等级} / {金币} / {背包:?}"
        ));
    }
    println!("load：slot_1 恢复为初始状态（等级 3、金币 100、背包 [木剑]）");
    Ok(())
}

fn 验证两个槽位的内容互不覆盖(仓库: &存档仓库) -> Result<(), String> {
    let 初始 = 仓库
        .读取("slot_1")
        .map_err(|错误| format!("verify 读 slot_1 失败：{错误}"))?;
    let 游玩后 = 仓库
        .读取("slot_2")
        .map_err(|错误| format!("verify 读 slot_2 失败：{错误}"))?;
    if 初始.存档.payload["等级"].as_u64() != Some(3) {
        return Err("verify：slot_1 被 slot_2 的写入污染了".into());
    }
    if 游玩后.存档.payload["等级"].as_u64() != Some(5) {
        return Err(format!(
            "verify：slot_2 的等级应为 5，实际 {:?}",
            游玩后.存档.payload["等级"]
        ));
    }
    let (列表, 损坏) = 仓库.列表();
    println!(
        "verify：槽位 {} 个、损坏 {} 个、slot_1=等级 3、slot_2=等级 5",
        列表.len(),
        损坏.len()
    );
    Ok(())
}

fn 主文件损坏后从备份恢复(仓库: &存档仓库) -> Result<(), String> {
    // 写两版，让 .bak 里有第一版。
    写入(
        仓库,
        "slot_c",
        "第一版",
        300,
        serde_json::json!({ "版本": 1 }),
    );
    写入(
        仓库,
        "slot_c",
        "第二版",
        400,
        serde_json::json!({ "版本": 2 }),
    );
    let 主文件 = 仓库.槽位路径("slot_c").map_err(|错误| 错误.to_string())?;
    std::fs::write(&主文件, "断电写坏的内容").map_err(|错误| 错误.to_string())?;

    let 读取 = 仓库
        .读取("slot_c")
        .map_err(|错误| format!("corrupt 读档失败：{错误}"))?;
    if !读取.从备份恢复 {
        return Err("corrupt：应当标记为从备份恢复".into());
    }
    if 读取.存档.payload["版本"].as_u64() != Some(1) {
        return Err(format!(
            "corrupt：应当恢复第一版，实际 {:?}",
            读取.存档.payload["版本"]
        ));
    }
    println!(
        "corrupt：主文件损坏，已从 {} 恢复第一版",
        读取.来源路径.display()
    );
    Ok(())
}

fn 版本过新的存档被拒绝(仓库: &存档仓库) -> Result<(), String> {
    let 目录 = 仓库.目录();
    std::fs::create_dir_all(目录).map_err(|错误| 错误.to_string())?;
    let 未来 = 目录.join("slot_future.sav");
    std::fs::write(
        &未来,
        format!(
            "(format_version: {}, slot: \"slot_future\", payload: {{}})",
            存档格式版本 + 1
        ),
    )
    .map_err(|错误| 错误.to_string())?;

    match 仓库.读取("slot_future") {
        Ok(_) => Err("future：版本过新的存档不该被读进来".into()),
        Err(错误) => {
            if !错误.to_string().contains("高于本程序支持") {
                return Err(format!("future：错误信息不够明确：{错误}"));
            }
            if !未来.is_file() {
                return Err("future：拒绝读取不代表可以删除文件".into());
            }
            println!("future：已拒绝版本过新的存档，且未删除文件");
            Ok(())
        }
    }
}

fn main() {
    match 主函数() {
        Ok(()) => {}
        Err(消息) => {
            eprintln!("{消息}");
            std::process::exit(1);
        }
    }
}
