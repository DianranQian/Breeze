//! 生成两个可交付的演示项目：`examples/2d_demo` 与 `examples/3d_demo`。
//!
//! # 为什么用编辑器 API 生成，而不是手写 RON
//!
//! 场景文件是嵌套 RON（`Option`、枚举、资源引用），手写极易写出"语法可以、
//! 语义不对"的文件，而错误表现为"打开后场景是空的"。
//! 走 `编辑器状态` 的公开 API 则**结构上不可能写出编辑器读不回来的文件**。
//!
//! # 素材
//!
//! 精灵来自 Dungeon Crawl Stone Soup 的 32×32 图块集（CC0，见
//! `examples/*/assets/DCSS_TILESET_LICENSE.txt` 与 `examples/*/README.md`）。
//! 本生成器只引用**相对项目路径**（`assets/2d_demo/*.png`），
//! 因此项目可以整体移动/复制到任何机器上打开。
//!
//! # 用法
//!
//! ```powershell
//! cargo run -p editor_core --example 生成演示项目
//! ```
//!
//! 生成后可用 `BEVY_STUDIO_OPEN_PROJECT=<仓库>/examples/2d_demo` 打开，
//! 或从编辑器「项目 → 打开示例项目」菜单进入。

use std::path::{Path, PathBuf};

use editor_core::command::编辑器命令;
use editor_core::state::编辑器状态;
use scene_doc::document::{变换数据, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 资源引用值, 颜色值};

type 结果<T> = Result<T, Box<dyn std::error::Error>>;

/// 一组装饰/单位的条目：(名称, 位置, 素材, 尺寸)。
type 条目 = (&'static str, (f32, f32), &'static str, (f32, f32));

/// 二维演示项目的根目录（相对仓库）。
const 二维项目: &str = "examples/2d_demo";
/// 三维演示项目的根目录（相对仓库）。
const 三维项目: &str = "examples/3d_demo";

fn main() -> 结果<()> {
    let 仓库根 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("定位仓库根目录失败")?
        .to_path_buf();

    生成二维(&仓库根)?;
    生成三维(&仓库根)?;
    println!("\n两个演示项目已生成：");
    println!("  {}", 仓库根.join(二维项目).display());
    println!("  {}", 仓库根.join(三维项目).display());
    Ok(())
}

/// 在临时目录里创建项目骨架。
///
/// # 为什么不在目标目录直接创建
///
/// `创建项目` 要求目标目录为空，而示例项目目录里已经放好了素材
/// （`assets/`）。因此先在临时目录生成项目与场景，再把
/// `project.ron` 与 `scenes/` 复制到最终目录 —— 素材原样保留。
fn 准备项目(名称: &str) -> 结果<(编辑器状态, PathBuf)> {
    let 临时 = std::env::temp_dir().join(format!(
        "bevy_studio_demo_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|值| 值.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&临时);
    let mut 编辑器 = 编辑器状态::新建();
    编辑器.创建项目(&临时, 名称)?;
    Ok((编辑器, 临时))
}

/// 把临时项目里的清单与场景复制到最终目录。
fn 复制到最终目录(临时根: &Path, 最终根: &Path) -> 结果<()> {
    std::fs::create_dir_all(最终根.join("scenes"))?;
    std::fs::copy(临时根.join("project.ron"), 最终根.join("project.ron"))?;
    for 项 in std::fs::read_dir(临时根.join("scenes"))? {
        let 项 = 项?;
        if 项.path().is_file() {
            std::fs::copy(项.path(), 最终根.join("scenes").join(项.file_name()))?;
        }
    }
    let _ = std::fs::remove_dir_all(临时根);
    Ok(())
}

fn 保存并收尾(
    编辑器: &mut 编辑器状态,
    临时根: &Path,
    最终根: &Path,
    场景相对: &str,
    选中名称: Option<&str>,
) -> 结果<()> {
    if let Some(名称) = 选中名称
        && let Some(标识) = 编辑器
            .会话
            .文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == 名称)
            .map(|实体| 实体.标识)
    {
        编辑器.选择.单选(标识);
    }
    let 场景路径 = 项目相对路径::解析(场景相对)?;
    编辑器.保存场景到(&场景路径)?;
    let 项目 = 编辑器.项目.as_mut().ok_or("项目未被创建")?;
    项目.清单可变().启动场景 = Some(场景路径.clone());
    项目.保存清单()?;
    复制到最终目录(临时根, 最终根)?;
    println!(
        "  实体 {} 个：{}",
        编辑器.会话.文档.实体表.len(),
        编辑器
            .会话
            .文档
            .实体表
            .values()
            .map(|实体| 实体.名称.as_str())
            .collect::<Vec<_>>()
            .join("、")
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 2D：林间营地（俯视角）
// ---------------------------------------------------------------------------

fn 生成二维(仓库根: &Path) -> 结果<()> {
    let 项目根 = 仓库根.join(二维项目);
    let (mut 编辑器, 临时根) = 准备项目("林间营地（2D 演示）")?;

    let 根 = 新建(&mut 编辑器, "林间营地", None)?;
    let 地面 = 新建(&mut 编辑器, "地面", Some(根))?;
    let 装饰 = 新建(&mut 编辑器, "装饰", Some(根))?;
    let 敌人 = 新建(&mut 编辑器, "敌人", Some(根))?;
    let 道具 = 新建(&mut 编辑器, "道具", Some(根))?;

    // ---- 地面：5×3 草地 + 中间一条土路 ----
    let 格 = 96.0;
    let mut 序号 = 0;
    for 行 in 0..3 {
        for 列 in 0..5 {
            序号 += 1;
            let 路 = 列 == 2;
            let 素材 = if 路 {
                "assets/2d_demo/floor_dirt.png"
            } else {
                "assets/2d_demo/floor_grass.png"
            };
            let 标识 = 新建(
                &mut 编辑器,
                &format!("{} {:02}", if 路 { "土路" } else { "草地" }, 序号),
                Some(地面),
            )?;
            布置精灵(
                &mut 编辑器,
                标识,
                (列 as f32 * 格 - 2.0 * 格, 行 as f32 * 格 - 格, -20.0),
                (格, 格),
                素材,
                姿态::平放(),
            )?;
        }
    }

    // ---- 装饰：树、雕像、祭坛 ----
    let 装饰项: [条目; 5] = [
        (
            "红枫",
            (-360.0, 150.0),
            "assets/2d_demo/decor_tree_red.png",
            (128.0, 128.0),
        ),
        (
            "金枫",
            (330.0, 170.0),
            "assets/2d_demo/decor_tree_yellow.png",
            (128.0, 128.0),
        ),
        (
            "石碑",
            (-330.0, -160.0),
            "assets/2d_demo/decor_statue.png",
            (80.0, 80.0),
        ),
        (
            "祭坛",
            (0.0, 150.0),
            "assets/2d_demo/decor_altar.png",
            (96.0, 96.0),
        ),
        (
            "远树",
            (390.0, -150.0),
            "assets/2d_demo/decor_tree_red.png",
            (96.0, 96.0),
        ),
    ];
    for (名称, 位置, 素材, 尺寸) in 装饰项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(装饰))?;
        布置精灵(
            &mut 编辑器,
            标识,
            (位置.0, 位置.1, 10.0),
            尺寸,
            素材,
            姿态::平放(),
        )?;
    }

    // ---- 敌人 ----
    let 敌人项: [(&'static str, (f32, f32), &'static str); 3] = [
        (
            "哥布林斥候",
            (-190.0, -110.0),
            "assets/2d_demo/monster_goblin.png",
        ),
        (
            "兽人战士",
            (200.0, -120.0),
            "assets/2d_demo/monster_orc.png",
        ),
        (
            "狗头人",
            (-60.0, -190.0),
            "assets/2d_demo/monster_kobold.png",
        ),
    ];
    for (名称, 位置, 素材) in 敌人项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(敌人))?;
        布置精灵(
            &mut 编辑器,
            标识,
            (位置.0, 位置.1, 0.0),
            (88.0, 88.0),
            素材,
            姿态::平放(),
        )?;
    }

    // ---- 道具 ----
    let 道具项: [条目; 3] = [
        (
            "金币堆",
            (150.0, 30.0),
            "assets/2d_demo/item_gold.png",
            (56.0, 56.0),
        ),
        (
            "蓝色药水",
            (210.0, 60.0),
            "assets/2d_demo/item_potion_blue.png",
            (48.0, 48.0),
        ),
        (
            "黑色药水",
            (250.0, 20.0),
            "assets/2d_demo/item_potion_black.png",
            (48.0, 48.0),
        ),
    ];
    for (名称, 位置, 素材, 尺寸) in 道具项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(道具))?;
        布置精灵(
            &mut 编辑器,
            标识,
            (位置.0, 位置.1, 5.0),
            尺寸,
            素材,
            姿态::平放(),
        )?;
    }

    // ---- 玩家 ----
    let 玩家 = 新建(&mut 编辑器, "玩家", Some(根))?;
    布置精灵(
        &mut 编辑器,
        玩家,
        (0.0, -20.0, 20.0),
        (104.0, 104.0),
        "assets/2d_demo/player_human_m.png",
        姿态::平放(),
    )?;

    保存并收尾(
        &mut 编辑器,
        &临时根,
        &项目根,
        "scenes/营地.scene.ron",
        Some("玩家"),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 3D：浮空圣所（空间布局演示）
// ---------------------------------------------------------------------------

/// 四元数辅助。
struct 姿态;

impl 姿态 {
    /// 不旋转（XY 平面内的精灵，面向 +Z）。
    fn 平放() -> (f32, f32, f32, f32) {
        (0.0, 0.0, 0.0, 1.0)
    }

    /// 绕 X 轴 −90°，把 XY 平面里的四边形放平到地面（法线朝 +Y）。
    fn 地面() -> (f32, f32, f32, f32) {
        let 半 = std::f32::consts::FRAC_1_SQRT_2;
        (-半, 0.0, 0.0, 半)
    }
}

fn 生成三维(仓库根: &Path) -> 结果<()> {
    let 项目根 = 仓库根.join(三维项目);
    let (mut 编辑器, 临时根) = 准备项目("浮空圣所（3D 演示）")?;

    let 根 = 新建(&mut 编辑器, "浮空圣所", None)?;
    let 地基 = 新建(&mut 编辑器, "地基", Some(根))?;
    let 建筑 = 新建(&mut 编辑器, "建筑", Some(根))?;
    let 单位 = 新建(&mut 编辑器, "单位", Some(根))?;
    let 拾取物 = 新建(&mut 编辑器, "拾取物", Some(根))?;

    // ---- 地基：3×3 四边形平铺在 y=0 平面上，两种材质交替 ----
    let 格 = 2.0_f32;
    let mut 序号 = 0;
    for 行 in 0..3 {
        for 列 in 0..3 {
            序号 += 1;
            let 棋格 = (行 + 列) % 2 == 0;
            let 素材 = if 棋格 {
                "assets/3d_demo/tile_floor_a.png"
            } else {
                "assets/3d_demo/tile_floor_b.png"
            };
            let 标识 = 新建(&mut 编辑器, &format!("地砖 {序号:02}"), Some(地基))?;
            布置精灵(
                &mut 编辑器,
                标识,
                (列 as f32 * 格 - 格, -0.05, 行 as f32 * 格 - 格),
                (格, 格),
                素材,
                姿态::地面(),
            )?;
        }
    }

    // ---- 建筑：中轴上的祭坛与两侧的树 ----
    let 建筑项: [(&'static str, (f32, f32, f32), &'static str, (f32, f32)); 4] = [
        (
            "中心祭坛",
            (0.0, 1.0, 0.0),
            "assets/3d_demo/prop_altar.png",
            (2.0, 2.0),
        ),
        (
            "左侧古树",
            (-4.2, 1.6, -1.5),
            "assets/3d_demo/prop_tree.png",
            (3.0, 3.0),
        ),
        (
            "右侧古树",
            (4.2, 1.6, -1.5),
            "assets/3d_demo/prop_tree_2.png",
            (3.0, 3.0),
        ),
        (
            "后方石碑",
            (0.0, 1.4, -4.5),
            "assets/3d_demo/prop_statue.png",
            (2.4, 2.4),
        ),
    ];
    for (名称, 位置, 素材, 尺寸) in 建筑项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(建筑))?;
        布置精灵(&mut 编辑器, 标识, 位置, 尺寸, 素材, 姿态::平放())?;
    }

    // ---- 单位：三个不同深度与左右位置的角色 ----
    let 单位项: [(&str, (f32, f32, f32), &str); 3] = [
        ("主角", (0.0, 0.9, 1.8), "assets/3d_demo/unit_player.png"),
        (
            "哥布林",
            (-2.6, 0.7, -0.6),
            "assets/3d_demo/unit_goblin.png",
        ),
        ("兽人", (2.8, 0.8, 2.6), "assets/3d_demo/unit_orc.png"),
    ];
    for (名称, 位置, 素材) in 单位项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(单位))?;
        布置精灵(&mut 编辑器, 标识, 位置, (1.4, 1.4), 素材, 姿态::平放())?;
    }

    // ---- 拾取物：不同高度（检验 Y 轴与拾取）----
    let 拾取项: [(&str, (f32, f32, f32), &str); 2] = [
        (
            "悬浮金币",
            (1.4, 1.8, 0.6),
            "assets/3d_demo/pickup_gold.png",
        ),
        (
            "悬浮药水",
            (-1.6, 2.1, 2.2),
            "assets/3d_demo/pickup_potion.png",
        ),
    ];
    for (名称, 位置, 素材) in 拾取项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(拾取物))?;
        布置精灵(&mut 编辑器, 标识, 位置, (0.7, 0.7), 素材, 姿态::平放())?;
    }

    保存并收尾(
        &mut 编辑器,
        &临时根,
        &项目根,
        "scenes/圣所.scene.ron",
        Some("中心祭坛"),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 通用装配
// ---------------------------------------------------------------------------

fn 新建(
    编辑器: &mut 编辑器状态, 名称: &str, 父: Option<实体标识>
) -> 结果<实体标识> {
    编辑器
        .新建实体(名称.to_string(), 父)
        .ok_or_else(|| format!("新建实体 `{名称}` 失败").into())
}

/// 装配一个带贴图的 Sprite（位置/尺寸/贴图/旋转）。
///
/// 位置是三元的：2D 项目把 z 当作图层顺序（地面 −20、装饰 10、玩家 20），
/// 3D 项目则真的用 y/z 表达空间位置。数据模型本来就是三元变换，
/// 两个项目共用同一个装配函数正是为了证明这一点。
fn 布置精灵(
    编辑器: &mut 编辑器状态,
    标识: 实体标识,
    位置: (f32, f32, f32),
    尺寸: (f32, f32),
    素材: &str,
    旋转: (f32, f32, f32, f32),
) -> 结果<()> {
    编辑器.执行(编辑器命令::设变换 {
        标识,
        变换: Some(变换数据 {
            平移: 位置,
            旋转,
            缩放: (1.0, 1.0, 1.0),
        }),
    })?;

    let 贴图路径 = 项目相对路径::解析(素材)?;
    let 属性 = [
        (
            "贴图".to_string(),
            属性值::资源(资源引用值::按路径(贴图路径)),
        ),
        ("颜色".to_string(), 属性值::颜色(颜色值::白())),
        ("自定义尺寸".to_string(), 属性值::二维(尺寸.0, 尺寸.1)),
        ("水平翻转".to_string(), 属性值::布尔(false)),
        ("垂直翻转".to_string(), 属性值::布尔(false)),
    ]
    .into_iter()
    .collect();

    if !编辑器.添加组件(标识, 组件数据::新建("Sprite", 属性)) {
        return Err(format!("给实体 {标识} 添加 Sprite 组件失败").into());
    }
    Ok(())
}
