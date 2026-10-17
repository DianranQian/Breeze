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
    let (mut 编辑器, 临时根) = 准备项目("青岚山谷（修仙 2D 演示）")?;

    let 根 = 新建(&mut 编辑器, "青岚山谷", None)?;
    let 地面 = 新建(&mut 编辑器, "地面", Some(根))?;
    let 景观 = 新建(&mut 编辑器, "景观", Some(根))?;
    let 人物 = 新建(&mut 编辑器, "人物", Some(根))?;

    // ---- 地面：5×3 草地，中央一列山道 ----
    let 格 = 96.0;
    let mut 序号 = 0;
    for 行 in 0..3 {
        for 列 in 0..5 {
            序号 += 1;
            let 路 = 列 == 2;
            let 素材 = if 路 {
                "assets/xiuxian/透明到道路.png"
            } else if 行 == 0 {
                "assets/xiuxian/透明到深色草地.png"
            } else {
                "assets/xiuxian/透明到浅草地.png"
            };
            let 标识 = 新建(
                &mut 编辑器,
                &format!("{} {:02}", if 路 { "山道" } else { "灵草地" }, 序号),
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

    // ---- 景观：崖壁、水潭、沙滩、灵田 ----
    let 景观项: [(&str, (f32, f32), &str, (f32, f32)); 5] = [
        (
            "北崖",
            (-260.0, 210.0),
            "assets/xiuxian/透明到山地.png",
            (192.0, 192.0),
        ),
        (
            "东崖",
            (300.0, 200.0),
            "assets/xiuxian/透明到悬崖.png",
            (160.0, 160.0),
        ),
        (
            "碧水潭",
            (-300.0, -170.0),
            "assets/xiuxian/透明到海水.png",
            (192.0, 192.0),
        ),
        (
            "白沙岸",
            (-160.0, -200.0),
            "assets/xiuxian/透明到沙滩.png",
            (128.0, 128.0),
        ),
        (
            "灵田",
            (260.0, -170.0),
            "assets/xiuxian/透明到耕地.png",
            (160.0, 160.0),
        ),
    ];
    for (名称, 位置, 素材, 尺寸) in 景观项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(景观))?;
        布置精灵(
            &mut 编辑器,
            标识,
            (位置.0, 位置.1, 10.0),
            尺寸,
            素材,
            姿态::平放(),
        )?;
    }

    // ---- 人物：主角与同门弟子（原项目用同一套角色图集，这里取不同帧）----
    let 主角 = 新建(&mut 编辑器, "主角", Some(人物))?;
    布置精灵(
        &mut 编辑器,
        主角,
        (0.0, -30.0, 20.0),
        (128.0, 128.0),
        "assets/xiuxian/protagonist_idle.png",
        姿态::平放(),
    )?;
    let 师姐 = 新建(&mut 编辑器, "同门师姐", Some(人物))?;
    布置精灵(
        &mut 编辑器,
        师姐,
        (140.0, 30.0, 18.0),
        (128.0, 128.0),
        "assets/xiuxian/protagonist_walk.png",
        姿态::平放(),
    )?;

    保存并收尾(
        &mut 编辑器,
        &临时根,
        &项目根,
        "scenes/qinglan_valley.scene.ron",
        Some("主角"),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 3D：TPS Arena 竞技场
// ---------------------------------------------------------------------------

/// 四元数辅助。
struct 姿态;

impl 姿态 {
    /// 不旋转（XY 平面内的精灵/模型，面向 +Z）。
    fn 平放() -> (f32, f32, f32, f32) {
        (0.0, 0.0, 0.0, 1.0)
    }

    /// 绕 X 轴 −90°，把四边形放平到地面（法线朝 +Y）。
    fn 地面() -> (f32, f32, f32, f32) {
        let 半 = std::f32::consts::FRAC_1_SQRT_2;
        (-半, 0.0, 0.0, 半)
    }

    /// 绕 Y 轴 90°，让四边形竖直朝侧面（用于侧墙）。
    fn 侧墙() -> (f32, f32, f32, f32) {
        let 半 = std::f32::consts::FRAC_1_SQRT_2;
        (0.0, 半, 0.0, 半)
    }
}

fn 生成三维(仓库根: &Path) -> 结果<()> {
    let 项目根 = 仓库根.join(三维项目);
    let (mut 编辑器, 临时根) = 准备项目("TPS Arena（3D 演示）")?;

    let 根 = 新建(&mut 编辑器, "TPS 竞技场", None)?;
    let 场地 = 新建(&mut 编辑器, "场地", Some(根))?;
    let 建筑 = 新建(&mut 编辑器, "掩体与建筑", Some(根))?;
    let 角色 = 新建(&mut 编辑器, "角色", Some(根))?;
    let 道具 = 新建(&mut 编辑器, "道具", Some(根))?;

    // ---- 地面：3×3 场地砖（贴图来自 TPS Arena 的 environment/floor.png）----
    let 格 = 8.0_f32;
    let mut 序号 = 0;
    for 行 in 0..3 {
        for 列 in 0..3 {
            序号 += 1;
            let 标识 = 新建(&mut 编辑器, &format!("场地砖 {序号:02}"), Some(场地))?;
            布置精灵(
                &mut 编辑器,
                标识,
                (列 as f32 * 格 - 格, 0.0, 行 as f32 * 格 - 格),
                (格, 格),
                "assets/tps/environment/floor.png",
                姿态::地面(),
            )?;
        }
    }

    // ---- 墙体：后墙与侧墙，贴图来自 environment/wall.png ----
    let 后墙 = 新建(&mut 编辑器, "北侧围墙", Some(建筑))?;
    布置精灵(
        &mut 编辑器,
        后墙,
        (0.0, 4.0, -12.0),
        (24.0, 8.0),
        "assets/tps/environment/wall.png",
        姿态::平放(),
    )?;
    let 侧墙 = 新建(&mut 编辑器, "西侧围墙", Some(建筑))?;
    布置精灵(
        &mut 编辑器,
        侧墙,
        (-12.0, 4.0, 0.0),
        (24.0, 8.0),
        "assets/tps/environment/wall.png",
        姿态::侧墙(),
    )?;

    // ---- 掩体与道具：真实 glTF 模型 ----
    let 模型项: [(&str, (f32, f32, f32), &str); 6] = [
        (
            "长货柜",
            (-7.0, 1.0, 2.0),
            "assets/tps/environment/Container_Long.gltf",
        ),
        (
            "木箱",
            (-3.0, 0.4, 6.0),
            "assets/tps/environment/Crate.gltf",
        ),
        (
            "爆炸桶",
            (3.5, 0.5, 5.0),
            "assets/tps/environment/ExplodingBarrel.gltf",
        ),
        (
            "路障",
            (7.5, 0.5, -4.0),
            "assets/tps/environment/Barrier_Single.gltf",
        ),
        (
            "锥桶",
            (1.0, 0.3, 8.0),
            "assets/tps/environment/TrafficCone.gltf",
        ),
        (
            "沙袋",
            (-8.5, 0.4, -5.0),
            "assets/tps/environment/SackTrench.gltf",
        ),
    ];
    for (名称, 位置, 素材) in 模型项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(道具))?;
        布置模型(&mut 编辑器, 标识, 位置, 素材)?;
    }

    // ---- 角色：三个真实角色模型（Quaternius Toon Shooter Kit，CC0）----
    let 角色项: [(&str, (f32, f32, f32), &str); 3] = [
        (
            "玩家士兵",
            (0.0, 0.0, 5.0),
            "assets/tps/characters/Character_Soldier.gltf",
        ),
        (
            "近战敌人",
            (-4.0, 0.0, -3.0),
            "assets/tps/characters/Character_Enemy.gltf",
        ),
        (
            "远程敌人",
            (4.5, 0.0, -2.0),
            "assets/tps/characters/Character_Hazmat.gltf",
        ),
    ];
    for (名称, 位置, 素材) in 角色项 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(角色))?;
        布置模型(&mut 编辑器, 标识, 位置, 素材)?;
    }

    保存并收尾(
        &mut 编辑器,
        &临时根,
        &项目根,
        "scenes/tps_arena.scene.ron",
        Some("玩家士兵"),
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

/// 装配 glTF 模型实体：只设置变换与「模型」字段。
fn 布置模型(
    编辑器: &mut 编辑器状态,
    标识: 实体标识,
    位置: (f32, f32, f32),
    模型: &str,
) -> 结果<()> {
    编辑器.执行(编辑器命令::设变换 {
        标识,
        变换: Some(变换数据 {
            平移: 位置,
            旋转: 姿态::平放(),
            缩放: (1.0, 1.0, 1.0),
        }),
    })?;

    let 路径 = 项目相对路径::解析(模型)?;
    let 属性 = [
        ("模型".to_string(), 属性值::资源(资源引用值::按路径(路径))),
        ("颜色".to_string(), 属性值::颜色(颜色值::白())),
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

/// 装配贴图精灵：位置/尺寸/贴图/旋转。
#[allow(clippy::too_many_arguments)]
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

    let 路径 = 项目相对路径::解析(素材)?;
    let 属性 = [
        ("贴图".to_string(), 属性值::资源(资源引用值::按路径(路径))),
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
