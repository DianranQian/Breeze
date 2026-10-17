//! 示例项目的结构完整性：素材不能悄悄丢、场景不能悄悄变空。
//!
//! # 为什么用"读文本 + 查文件存在"而不是反序列化
//!
//! 这个测试要守住的是**仓库资产**：场景里每一个 `贴图` 引用都必须有对应文件，
//! 且关键对象名称必须还在。用文本扫描实现最直接，也不依赖任何内部 API，
//! 因此就算以后场景格式演进，这条测试仍然能发现"素材路径写错/文件被删"。

use std::path::{Path, PathBuf};

fn 仓库根() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn 项目目录(名: &str) -> PathBuf {
    仓库根().join("examples").join(名)
}

/// 从场景文件里抽出所有 `路径: "..."` 形式的贴图引用。
fn 贴图路径(场景文本: &str) -> Vec<String> {
    let mut 结果 = Vec::new();
    let mut 剩余 = 场景文本;
    while let Some(位置) = 剩余.find("路径: \"") {
        剩余 = &剩余[位置 + "路径: \"".len()..];
        if let Some(结束) = 剩余.find('"') {
            结果.push(剩余[..结束].to_string());
            剩余 = &剩余[结束..];
        } else {
            break;
        }
    }
    结果
}

fn 校验项目(目录名: &str, 场景文件: &str, 期望实体: &[&str]) {
    let 根 = 项目目录(目录名);
    assert!(
        根.join("project.ron").is_file(),
        "{目录名} 缺少 project.ron"
    );
    assert!(根.join("README.md").is_file(), "{目录名} 缺少 README");
    assert!(根.join("ASSETS.md").is_file(), "{目录名} 缺少素材清单");

    let 场景路径 = 根.join("scenes").join(场景文件);
    let 文本 = std::fs::read_to_string(&场景路径)
        .unwrap_or_else(|错误| panic!("读取 {} 失败：{错误}", 场景路径.display()));

    // 场景里必然有资源引用，否则说明生成器丢了贴图字段。
    let 引用 = 贴图路径(&文本);
    assert!(
        !引用.is_empty(),
        "{目录名} 的场景里没有任何贴图引用，示例会退化成白方块"
    );

    // 每个引用都必须能在项目内找到真实文件。
    for 相对 in &引用 {
        let 文件 = 根.join(相对.replace('/', std::path::MAIN_SEPARATOR_STR));
        assert!(
            文件.is_file(),
            "{目录名} 引用了不存在的素材：{相对}（期望在 {}）",
            文件.display()
        );
    }

    // 关键对象名称必须还在（GUI 验收与文档都依赖它们）。
    for 名称 in 期望实体 {
        assert!(
            文本.contains(&format!("名称: \"{名称}\"")),
            "{目录名} 缺少关键对象 `{名称}`"
        );
    }

    // 许可证必须随项目分发。
    assert!(
        根.join("assets").join("DCSS_TILESET_LICENSE.txt").is_file(),
        "{目录名} 缺少素材许可证文件"
    );
}

/// 素材与场景都必须在仓库里，且相对路径可用。
#[test]
fn 二维示例项目素材齐全() {
    校验项目(
        "2d_demo",
        "营地.scene.ron",
        &[
            "玩家",
            "哥布林斥候",
            "兽人战士",
            "狗头人",
            "红枫",
            "祭坛",
            "金币堆",
        ],
    );
}

#[test]
fn 三维示例项目素材齐全() {
    校验项目(
        "3d_demo",
        "圣所.scene.ron",
        &[
            "中心祭坛",
            "左侧古树",
            "右侧古树",
            "后方石碑",
            "主角",
            "哥布林",
            "兽人",
            "悬浮金币",
        ],
    );
}

/// 两个项目的贴图必须都落在项目目录内部：不能引用仓库外或绝对路径。
#[test]
fn 贴图路径都是项目相对路径() {
    for (目录名, 场景) in [
        ("2d_demo", "scenes/营地.scene.ron"),
        ("3d_demo", "scenes/圣所.scene.ron"),
    ] {
        let 文本 = std::fs::read_to_string(项目目录(目录名).join(场景)).expect("读取场景");
        for 相对 in 贴图路径(&文本) {
            assert!(
                !相对.contains(':') && !相对.starts_with('/'),
                "{目录名} 的贴图必须是项目相对路径：{相对}"
            );
            assert!(
                相对.starts_with("assets/"),
                "{目录名} 的贴图必须位于 assets/ 下：{相对}"
            );
        }
    }
}

/// 素材目录里不应出现未被引用的孤儿文件（许可证与说明除外）。
#[test]
fn 素材目录没有孤儿文件() {
    for (目录名, 场景) in [
        ("2d_demo", "scenes/营地.scene.ron"),
        ("3d_demo", "scenes/圣所.scene.ron"),
    ] {
        let 根 = 项目目录(目录名);
        let 文本 = std::fs::read_to_string(根.join(场景)).expect("读取场景");
        let 引用: Vec<String> = 贴图路径(&文本)
            .into_iter()
            .filter_map(|路径| 路径.rsplit('/').next().map(str::to_string))
            .collect();
        let 素材目录 = 根.join("assets").join(目录名);
        for 项 in std::fs::read_dir(&素材目录).expect("读取素材目录") {
            let 项 = 项.expect("目录项");
            let 名 = 项.file_name().to_string_lossy().to_string();
            if !名.ends_with(".png") {
                continue;
            }
            assert!(
                引用.contains(&名),
                "{目录名} 的 {名} 没有被场景引用（要么用上，要么删除）"
            );
        }
    }
}

/// 素材尺寸必须是 PNG 且非空（防止 0 字节文件被当成素材提交）。
#[test]
fn 素材是有效非空文件() {
    fn 校验目录(目录: &Path) {
        for 项 in std::fs::read_dir(目录).expect("读取素材目录") {
            let 项 = 项.expect("目录项");
            let 路径 = 项.path();
            if 路径.extension().and_then(|值| 值.to_str()) != Some("png") {
                continue;
            }
            let 字节 = std::fs::read(&路径).expect("读取素材");
            assert!(
                字节.len() > 64 && 字节.starts_with(&[0x89, b'P', b'N', b'G']),
                "{} 不是有效 PNG",
                路径.display()
            );
        }
    }
    校验目录(&项目目录("2d_demo").join("assets").join("2d_demo"));
    校验目录(&项目目录("3d_demo").join("assets").join("3d_demo"));
}
