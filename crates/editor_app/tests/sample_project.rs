//! 中文名：示例项目
//! 示例项目的结构完整性：素材不能悄悄丢、场景不能悄悄退化成静态图。
//!
//! # 为什么用"读文本 + 查文件存在"
//!
//! 这个测试守住的是**仓库里的示例资产与场景**：场景里的每个资源引用都要有
//! 真实文件，关键对象名称必须还在，两个示例必须真的使用程序化渲染组件
//! （`TileTerrain` / `Primitive3d`）而不是退回一堆预拼好的 PNG。
//!
//! 用文本扫描实现最直接，也不依赖任何内部 API，因此场景格式演进时它仍然
//! 能发现"路径写错 / 文件被删 / 生成器退化成静态图"。

use std::path::{Path, PathBuf};

fn 仓库根() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn 项目目录(名: &str) -> PathBuf {
    仓库根().join("examples").join(名)
}

fn 场景文本(目录名: &str, 场景文件: &str) -> String {
    let 路径 = 项目目录(目录名).join("scenes").join(场景文件);
    std::fs::read_to_string(&路径)
        .unwrap_or_else(|错误| panic!("读取 {} 失败：{错误}", 路径.display()))
}

/// 从场景文件里抽出所有 `路径: "..."` 形式的资源引用。
fn 资源路径(场景文本: &str) -> Vec<String> {
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

/// 从场景文件里抽出某个文本字段的值（例如 `图集目录`）。
fn 文本字段(场景文本: &str, 字段: &str) -> Option<String> {
    let 标记 = format!("\"{字段}\": 文本(\"");
    let 位置 = 场景文本.find(&标记)?;
    let 剩余 = &场景文本[位置 + 标记.len()..];
    let 结束 = 剩余.find('"')?;
    Some(剩余[..结束].to_string())
}

/// 递归收集某个扩展名的文件。
fn 收集文件(目录: &Path, 扩展名: &str) -> Vec<PathBuf> {
    let mut 结果 = Vec::new();
    let Ok(项) = std::fs::read_dir(目录) else {
        return 结果;
    };
    for 项 in 项.flatten() {
        let 路径 = 项.path();
        if 路径.is_dir() {
            结果.extend(收集文件(&路径, 扩展名));
        } else if 路径.extension().and_then(|值| 值.to_str()) == Some(扩展名) {
            结果.push(路径);
        }
    }
    结果
}

fn 校验项目(目录名: &str, 场景文件: &str, 期望实体: &[&str]) -> String {
    let 根 = 项目目录(目录名);
    assert!(
        根.join("project.ron").is_file(),
        "{目录名} 缺少 project.ron"
    );
    assert!(根.join("README.md").is_file(), "{目录名} 缺少 README");
    assert!(根.join("ASSETS.md").is_file(), "{目录名} 缺少素材清单");

    let 文本 = 场景文本(目录名, 场景文件);

    let 引用 = 资源路径(&文本);
    assert!(
        !引用.is_empty(),
        "{目录名} 的场景里没有任何资源引用，示例会退化成白方块"
    );
    for 相对 in &引用 {
        let 文件 = 根.join(相对.replace('/', std::path::MAIN_SEPARATOR_STR));
        assert!(
            文件.is_file(),
            "{目录名} 引用了不存在的素材：{相对}（期望在 {}）",
            文件.display()
        );
    }

    for 名称 in 期望实体 {
        assert!(
            文本.contains(&format!("名称: \"{名称}\"")),
            "{目录名} 缺少关键对象 `{名称}`"
        );
    }

    assert!(根.join("assets").exists(), "{目录名} 缺少 assets 目录");
    文本
}

/// 2D 示例：地形必须是 `TileTerrain` 程序化渲染，道具/角色是精灵。
#[test]
fn 二维示例使用程序化瓦片地形() {
    let 文本 = 校验项目(
        "2d_demo",
        "qinglan_valley.scene.ron",
        &[
            "主角",
            "同门师姐",
            "采药人",
            "村民",
            "青岚山谷地形",
            "青溪村·东屋",
            "古樟 01",
        ],
    );

    assert!(
        文本.contains("类型名: \"TileTerrain\""),
        "2D 示例必须用 TileTerrain 在加载时程序化合成地面，而不是烘焙 PNG"
    );
    let 地形文件 = 文本字段(&文本, "地形文件").expect("TileTerrain 缺少地形文件字段");
    let 图集目录 = 文本字段(&文本, "图集目录").expect("TileTerrain 缺少图集目录字段");
    let 根 = 项目目录("2d_demo");
    assert!(
        根.join(地形文件.replace('/', std::path::MAIN_SEPARATOR_STR))
            .is_file(),
        "地形数据文件不存在：{地形文件}"
    );
    assert!(
        根.join(图集目录.replace('/', std::path::MAIN_SEPARATOR_STR))
            .is_dir(),
        "图集目录不存在：{图集目录}"
    );

    // 地形数据里存的是字符网格，不是一张图。
    let 地形文本 =
        std::fs::read_to_string(根.join(地形文件.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .expect("读取地形数据");
    assert!(地形文本.contains("宽度: 48"), "地形数据宽度应为 48");
    assert!(地形文本.contains("地表:"), "地形数据缺少地表网格");
    assert!(
        !地形文本.contains(".png"),
        "地形数据不应引用预烘焙的地面图片"
    );
}

/// 3D 示例：地面/围墙必须是受光的 `Primitive3d`，掩体是真实 glTF。
#[test]
fn 三维示例使用受光几何体与真实模型() {
    let 文本 = 校验项目(
        "3d_demo",
        "tps_arena.scene.ron",
        &[
            "玩家士兵",
            "近战敌人",
            "远程敌人",
            "重装敌人",
            "竞技场地板",
            "北墙",
            "南墙",
            "长货柜·中央",
            "木箱·西北",
            "交通锥·南西",
        ],
    );

    assert!(
        文本.contains("类型名: \"Primitive3d\""),
        "3D 示例的地面/围墙必须是实体几何体，而不是 unlit 四边形"
    );
    assert!(文本.contains("\"形状\":"), "Primitive3d 缺少形状字段");
    assert!(
        文本.contains("\"贴图重复\":"),
        "Primitive3d 缺少 UV 重复字段"
    );
    assert!(文本.contains("\"受光\": 布尔(true)"), "地面/围墙必须受光");

    // 23 件掩体 + 4 名角色，共 27 个 Sprite（模型）实体。
    let 精灵数 = 文本.matches("类型名: \"Sprite\"").count();
    assert_eq!(精灵数, 27, "3D 示例应有 23 件掩体 + 4 名角色");
    // 1 块地面 + 4 面围墙。
    let 几何数 = 文本.matches("类型名: \"Primitive3d\"").count();
    assert_eq!(几何数, 5, "3D 示例应有 1 块地面 + 4 面围墙");

    assert!(
        项目目录("3d_demo")
            .join("assets/tps/environment/Container_Small.gltf")
            .is_file(),
        "缺少短货柜模型（原项目四角货柜组要用）"
    );
}

/// 两个项目的资源引用必须都落在项目目录内部：不能引用仓库外或绝对路径。
#[test]
fn 贴图路径都是项目相对路径() {
    for (目录名, 场景) in [
        ("2d_demo", "qinglan_valley.scene.ron"),
        ("3d_demo", "tps_arena.scene.ron"),
    ] {
        for 相对 in 资源路径(&场景文本(目录名, 场景)) {
            assert!(
                !相对.contains(':') && !相对.starts_with('/'),
                "{目录名} 的资源必须是项目相对路径：{相对}"
            );
            assert!(
                相对.starts_with("assets/"),
                "{目录名} 的资源必须位于 assets/ 下：{相对}"
            );
        }
    }
}

/// 素材目录里不应出现未被使用的孤儿 PNG。
///
/// 2D 的瓦片图集是"整目录被 TileTerrain 使用"，不存在逐张 `路径:` 引用；
/// 因此把场景里声明的 `图集目录` 整体视为已使用，其余 PNG 必须被显式引用。
#[test]
fn 素材目录没有孤儿文件() {
    let 根 = 项目目录("2d_demo");
    let 文本 = 场景文本("2d_demo", "qinglan_valley.scene.ron");
    let 图集目录 = 文本字段(&文本, "图集目录").expect("TileTerrain 缺少图集目录");
    let 图集根 = 根.join(图集目录.replace('/', std::path::MAIN_SEPARATOR_STR));

    let 被引用: Vec<PathBuf> = 资源路径(&文本)
        .into_iter()
        .map(|路径| 根.join(路径.replace('/', std::path::MAIN_SEPARATOR_STR)))
        .collect();

    for 图片 in 收集文件(&根.join("assets/xiuxian"), "png") {
        let 在图集目录 = 图片.starts_with(&图集根);
        assert!(
            在图集目录 || 被引用.iter().any(|路径| 路径 == &图片),
            "2d_demo 的 {} 没有被场景使用（要么引用，要么删除）",
            图片.display()
        );
    }

    // 3D：所有 PNG 都必须被资源引用覆盖。
    let 根 = 项目目录("3d_demo");
    let 文本 = 场景文本("3d_demo", "tps_arena.scene.ron");
    let 被引用: Vec<PathBuf> = 资源路径(&文本)
        .into_iter()
        .map(|路径| 根.join(路径.replace('/', std::path::MAIN_SEPARATOR_STR)))
        .collect();
    for 图片 in 收集文件(&根.join("assets/tps"), "png") {
        assert!(
            被引用.iter().any(|路径| 路径 == &图片),
            "3d_demo 的 {} 没有被场景引用（要么用上，要么删除）",
            图片.display()
        );
    }
}

/// 素材必须是有效非空 PNG（防止 0 字节文件被当成素材提交）。
#[test]
fn 素材是有效非空文件() {
    for (目录名, 素材根) in [
        ("2d_demo", "assets/xiuxian"),
        ("3d_demo", "assets/tps/environment"),
    ] {
        for 图片 in 收集文件(&项目目录(目录名).join(素材根), "png") {
            let 字节 = std::fs::read(&图片).expect("读取素材");
            assert!(
                字节.len() > 64 && 字节.starts_with(&[0x89, b'P', b'N', b'G']),
                "{} 不是有效 PNG",
                图片.display()
            );
        }
    }
}
