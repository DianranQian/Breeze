//! 中文名：生成演示项目
//! 生成两个可交付的示例项目：`examples/2d_demo` 与 `examples/3d_demo`。
//!
//! # 这两个示例在演示什么
//!
//! 它们不是"把原项目截图摆进编辑器"，而是用编辑器新增的两条**程序化渲染**
//! 路径复刻原项目的画面构成：
//!
//! | 示例 | 原项目 | 复刻方式 |
//! | --- | --- | --- |
//! | 2D | 修仙冒险游戏 | `TileTerrain`：字符网格 → 双网格四角掩码 → 原项目瓦片图集 → 分层合成 |
//! | 3D | TPS Arena | `Primitive3d`：长方体地面/围墙 + 受光材质 + UV 重复；glTF 道具/角色 |
//!
//! 2D 的地形数据与道具由 `tools/build_xiuxian_demo.py` 生成；
//! 本程序只负责把它们装进场景（走编辑器公开 API，结构上不可能写出
//! 编辑器读不回来的文件）。
//!
//! # 用法
//!
//! ```powershell
//! # 先重新生成 2D 素材/数据（图集原样复制，地形是字符网格）
//! python tools/build_xiuxian_demo.py --source <修仙项目>/assets `
//!     --out examples/2d_demo/assets/xiuxian --layout examples/2d_demo/layout.json
//! # 再生成两个场景
//! cargo run -p editor_core --example generate_demo_project
//! ```
//!
//! 生成后可用 `BEVY_STUDIO_OPEN_PROJECT=<仓库>/examples/2d_demo` 打开，
//! 或从编辑器「项目 → 打开示例项目」菜单进入。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use editor_core::command::编辑器命令;
use editor_core::state::编辑器状态;
use scene_doc::document::{变换数据, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 枚举值, 资源引用值, 颜色值};

type 结果<T> = Result<T, Box<dyn std::error::Error>>;

/// 二维示例项目的根目录（相对仓库）。
const 二维项目: &str = "examples/2d_demo";
/// 三维示例项目的根目录（相对仓库）。
const 三维项目: &str = "examples/3d_demo";

fn main() -> 结果<()> {
    let 仓库根 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("定位仓库根目录失败")?
        .to_path_buf();

    生成二维(&仓库根)?;
    生成三维(&仓库根)?;
    println!("\n两个示例项目已生成：");
    println!("  {}", 仓库根.join(二维项目).display());
    println!("  {}", 仓库根.join(三维项目).display());
    Ok(())
}

// ---------------------------------------------------------------------------
// 通用装配
// ---------------------------------------------------------------------------

/// 在临时目录里创建项目骨架。
///
/// `创建项目` 要求目标目录为空，而示例目录里已经放好了素材。
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

fn 新建(
    编辑器: &mut 编辑器状态, 名称: &str, 父: Option<实体标识>
) -> 结果<实体标识> {
    编辑器
        .新建实体(名称.to_string(), 父)
        .ok_or_else(|| format!("新建实体 `{名称}` 失败").into())
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

fn 属性表(
    项: impl IntoIterator<Item = (&'static str, 属性值)>
) -> BTreeMap<String, 属性值> {
    项.into_iter()
        .map(|(键, 值)| (键.to_string(), 值))
        .collect()
}

fn 资源值(路径: &str) -> 结果<属性值> {
    Ok(属性值::资源(资源引用值::按路径(
        项目相对路径::解析(路径)?,
    )))
}

fn 设变换(
    编辑器: &mut 编辑器状态,
    标识: 实体标识,
    平移: (f32, f32, f32),
    旋转: (f32, f32, f32, f32),
) -> 结果<()> {
    编辑器.执行(编辑器命令::设变换 {
        标识,
        变换: Some(变换数据 {
            平移,
            旋转,
            缩放: (1.0, 1.0, 1.0),
        }),
    })?;
    Ok(())
}

fn 加组件(
    编辑器: &mut 编辑器状态, 标识: 实体标识, 组件: 组件数据
) -> 结果<()> {
    if 编辑器.添加组件(标识, 组件) {
        Ok(())
    } else {
        Err(format!("给实体 {标识} 添加组件失败").into())
    }
}

// ---------------------------------------------------------------------------
// 2D：青岚山谷（TileTerrain + 道具精灵）
// ---------------------------------------------------------------------------

fn 生成二维(仓库根: &Path) -> 结果<()> {
    let 项目根 = 仓库根.join(二维项目);
    let (mut 编辑器, 临时根) = 准备项目("青岚山谷（修仙 2D 演示）")?;

    let 布局路径 = 项目根.join("layout.json");
    let 布局文本 = std::fs::read_to_string(&布局路径).map_err(|错误| {
        format!(
            "读取 {} 失败：{错误}\n先运行 tools/build_xiuxian_demo.py 生成 2D 数据",
            布局路径.display()
        )
    })?;
    let 布局: serde_json::Value = serde_json::from_str(&布局文本)?;

    let 根 = 新建(&mut 编辑器, "青岚山谷", None)?;
    let 地形组 = 新建(&mut 编辑器, "地形", Some(根))?;

    // ---- 程序化地形：一个实体 + 参数，不是一个烘焙好的 PNG ----
    let 地形 = 布局.get("terrain").ok_or("layout.json 缺少 terrain")?;
    let 地形标识 = 新建(&mut 编辑器, 取文本(地形, "name")?, Some(地形组))?;
    设变换(
        &mut 编辑器,
        地形标识,
        (
            取数值(地形, "x")? as f32,
            取数值(地形, "y")? as f32,
            取数值(地形, "z")? as f32,
        ),
        (0.0, 0.0, 0.0, 1.0),
    )?;
    let 显示尺寸 = 地形
        .get("display_size")
        .and_then(|值| 值.as_array())
        .and_then(|值| Some((值.first()?.as_f64()? as f32, 值.get(1)?.as_f64()? as f32)))
        .ok_or("terrain.display_size 必须是 [宽, 高]")?;
    加组件(
        &mut 编辑器,
        地形标识,
        组件数据::新建(
            "TileTerrain",
            属性表([
                ("地形文件", 属性值::文本(取文本(地形, "file")?.to_string())),
                ("图集目录", 属性值::文本(取文本(地形, "atlas")?.to_string())),
                ("显示尺寸", 属性值::二维(显示尺寸.0, 显示尺寸.1)),
                (
                    "每格像素",
                    属性值::整数(取数值(地形, "cell_pixels")? as i64),
                ),
                (
                    "变体概率",
                    属性值::浮点(取数值(地形, "variant_chance")? as f32),
                ),
            ]),
        ),
    )?;

    // ---- 道具/角色：按 layout.json 的组路径建树 ----
    let 对象 = 布局
        .get("objects")
        .and_then(|值| 值.as_array())
        .ok_or("layout.json 缺少 objects")?;
    let mut 组表: BTreeMap<String, 实体标识> = BTreeMap::new();
    for 项 in 对象 {
        let 名称 = 取文本(项, "name")?;
        let 组路径 = 项
            .get("group")
            .and_then(|值| 值.as_array())
            .ok_or_else(|| format!("对象 `{名称}` 缺少 group"))?;

        let mut 父 = 根;
        let mut 路径 = String::new();
        for 段 in 组路径 {
            let 段 = 段.as_str().ok_or("group 里的项必须是字符串")?;
            路径.push('/');
            路径.push_str(段);
            父 = match 组表.get(&路径) {
                Some(标识) => *标识,
                None => {
                    let 标识 = 新建(&mut 编辑器, 段, Some(父))?;
                    组表.insert(路径.clone(), 标识);
                    标识
                }
            };
        }

        let 标识 = 新建(&mut 编辑器, 名称, Some(父))?;
        设变换(
            &mut 编辑器,
            标识,
            (
                取数值(项, "x")? as f32,
                取数值(项, "y")? as f32,
                取数值(项, "z")? as f32,
            ),
            (0.0, 0.0, 0.0, 1.0),
        )?;

        let 透明色 = match 项.get("tint").and_then(|值| 值.as_array()) {
            Some(值) if 值.len() >= 3 => 颜色值::srgb(
                值[0].as_f64().unwrap_or(1.0) as f32,
                值[1].as_f64().unwrap_or(1.0) as f32,
                值[2].as_f64().unwrap_or(1.0) as f32,
                1.0,
            ),
            _ => 颜色值::白(),
        };
        加组件(
            &mut 编辑器,
            标识,
            组件数据::新建(
                "Sprite",
                属性表([
                    ("贴图", 资源值(取文本(项, "asset")?)?),
                    ("颜色", 属性值::颜色(透明色)),
                    (
                        "自定义尺寸",
                        属性值::二维(取数值(项, "w")? as f32, 取数值(项, "h")? as f32),
                    ),
                    (
                        "水平翻转",
                        属性值::布尔(
                            项.get("flip_x")
                                .and_then(|值| 值.as_bool())
                                .unwrap_or(false),
                        ),
                    ),
                    ("垂直翻转", 属性值::布尔(false)),
                ]),
            ),
        )?;
    }

    保存并收尾(
        &mut 编辑器,
        &临时根,
        &项目根,
        "scenes/qinglan_valley.scene.ron",
        Some("主角"),
    )
}

// ---------------------------------------------------------------------------
// 3D：TPS Arena（Primitive3d 地面/围墙 + glTF 掩体）
// ---------------------------------------------------------------------------

/// 原 TPS Arena 的竞技场半宽（`resources::竞技场半宽`）。
const 半宽: f32 = 24.0;
/// 原项目围墙高度（`resources::墙高`）。
const 墙高: f32 = 8.0;
/// 原项目墙厚。
const 墙厚: f32 = 0.6;
/// 地面厚度（原项目用扁长方体而不是平面）。
const 地厚: f32 = 0.6;

/// 一条掩体：`(模型, 场景名, 平面中心, 是否绕 Y 轴 90°)`。
///
/// 坐标逐条对应原项目 `src/plugins/level.rs::掩体布局`。原表里还有
/// 视觉包围盒/碰撞盒两列，那是给碰撞与子弹用的；示例只渲染视觉，
/// 因此不需要复制尺寸列 —— 模型自带的节点缩放会给出正确大小。
struct 掩体条目 {
    模型: &'static str,
    名称: &'static str,
    平面中心: (f32, f32),
    旋转90: bool,
}

const 掩体布局: &[掩体条目] = &[
    // 四角货柜组
    掩体条目 {
        模型: "assets/tps/environment/Container_Long.gltf",
        名称: "长货柜·西北",
        平面中心: (-9.0, -7.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Container_Small.gltf",
        名称: "短货柜·东北",
        平面中心: (9.0, -7.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Container_Small.gltf",
        名称: "短货柜·西南",
        平面中心: (-9.0, 7.0),
        旋转90: true,
    },
    掩体条目 {
        模型: "assets/tps/environment/Container_Long.gltf",
        名称: "长货柜·东南",
        平面中心: (9.0, 7.0),
        旋转90: true,
    },
    // 中央主掩体
    掩体条目 {
        模型: "assets/tps/environment/Container_Long.gltf",
        名称: "长货柜·中央",
        平面中心: (0.0, 0.0),
        旋转90: false,
    },
    // 左右沙袋墙
    掩体条目 {
        模型: "assets/tps/environment/SackTrench.gltf",
        名称: "沙袋墙·西",
        平面中心: (-14.0, 0.0),
        旋转90: true,
    },
    掩体条目 {
        模型: "assets/tps/environment/SackTrench.gltf",
        名称: "沙袋墙·东",
        平面中心: (14.0, 0.0),
        旋转90: true,
    },
    // 木箱
    掩体条目 {
        模型: "assets/tps/environment/Crate.gltf",
        名称: "木箱·西北",
        平面中心: (-19.0, -11.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Crate.gltf",
        名称: "木箱·东北",
        平面中心: (19.0, -11.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Crate.gltf",
        名称: "木箱·西南",
        平面中心: (-19.0, 11.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Crate.gltf",
        名称: "木箱·东南",
        平面中心: (19.0, 11.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Crate.gltf",
        名称: "木箱·南西",
        平面中心: (-9.0, -19.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Crate.gltf",
        名称: "木箱·南东",
        平面中心: (9.0, -19.0),
        旋转90: false,
    },
    // 油桶
    掩体条目 {
        模型: "assets/tps/environment/ExplodingBarrel.gltf",
        名称: "油桶·西南",
        平面中心: (-17.0, -17.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/ExplodingBarrel.gltf",
        名称: "油桶·东南",
        平面中心: (17.0, -17.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/ExplodingBarrel.gltf",
        名称: "油桶·西北",
        平面中心: (-17.0, 17.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/ExplodingBarrel.gltf",
        名称: "油桶·东北",
        平面中心: (17.0, 17.0),
        旋转90: false,
    },
    // 铁马
    掩体条目 {
        模型: "assets/tps/environment/Barrier_Single.gltf",
        名称: "铁马·南",
        平面中心: (0.0, -14.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/Barrier_Single.gltf",
        名称: "铁马·北",
        平面中心: (0.0, 14.0),
        旋转90: false,
    },
    // 交通锥
    掩体条目 {
        模型: "assets/tps/environment/TrafficCone.gltf",
        名称: "交通锥·南西",
        平面中心: (-5.0, -14.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/TrafficCone.gltf",
        名称: "交通锥·南东",
        平面中心: (5.0, -14.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/TrafficCone.gltf",
        名称: "交通锥·北西",
        平面中心: (-5.0, 14.0),
        旋转90: false,
    },
    掩体条目 {
        模型: "assets/tps/environment/TrafficCone.gltf",
        名称: "交通锥·北东",
        平面中心: (5.0, 14.0),
        旋转90: false,
    },
];

/// 绕 Y 轴 90° 的四元数 `(x, y, z, w)`。
const 旋转_九十度: (f32, f32, f32, f32) = (0.0, 0.707_106_77, 0.0, 0.707_106_77);

/// 一面墙：`(名称, 中心, 尺寸)`。
type 墙条目 = (&'static str, (f32, f32, f32), (f32, f32, f32));

/// 一名角色：`(名称, 位置, 旋转, 模型路径)`。
type 角色条目 = (
    &'static str,
    (f32, f32, f32),
    (f32, f32, f32, f32),
    &'static str,
);

fn 生成三维(仓库根: &Path) -> 结果<()> {
    let 项目根 = 仓库根.join(三维项目);
    let (mut 编辑器, 临时根) = 准备项目("TPS Arena（3D 演示）")?;

    let 根 = 新建(&mut 编辑器, "TPS 竞技场", None)?;
    let 场地 = 新建(&mut 编辑器, "场地", Some(根))?;
    let 掩体组 = 新建(&mut 编辑器, "掩体与建筑", Some(根))?;
    let 角色组 = 新建(&mut 编辑器, "角色", Some(根))?;

    // ---- 地面：48×48×0.6 的长方体，纹理 6×6 重复 ----
    let 地板 = 新建(&mut 编辑器, "竞技场地板", Some(场地))?;
    设变换(
        &mut 编辑器,
        地板,
        (0.0, -地厚 * 0.5, 0.0),
        (0.0, 0.0, 0.0, 1.0),
    )?;
    加组件(
        &mut 编辑器,
        地板,
        组件数据::新建(
            "Primitive3d",
            属性表([
                ("形状", 属性值::枚举(枚举值::新建("Cuboid", 0))),
                ("尺寸", 属性值::三维(半宽 * 2.0, 地厚, 半宽 * 2.0)),
                ("贴图", 资源值("assets/tps/environment/floor.png")?),
                ("颜色", 属性值::颜色(颜色值::白())),
                ("贴图重复", 属性值::二维(6.0, 6.0)),
                ("受光", 属性值::布尔(true)),
                ("双面", 属性值::布尔(false)),
            ]),
        ),
    )?;

    // ---- 四面围墙：48×8×0.6，纹理沿墙长重复 6 次 ----
    let 墙配置: [墙条目; 4] = [
        ("北墙", (0.0, 墙高 * 0.5, 半宽), (半宽 * 2.0, 墙高, 墙厚)),
        ("南墙", (0.0, 墙高 * 0.5, -半宽), (半宽 * 2.0, 墙高, 墙厚)),
        ("东墙", (半宽, 墙高 * 0.5, 0.0), (墙厚, 墙高, 半宽 * 2.0)),
        ("西墙", (-半宽, 墙高 * 0.5, 0.0), (墙厚, 墙高, 半宽 * 2.0)),
    ];
    for (名称, 位置, 尺寸) in 墙配置 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(场地))?;
        设变换(&mut 编辑器, 标识, 位置, (0.0, 0.0, 0.0, 1.0))?;
        加组件(
            &mut 编辑器,
            标识,
            组件数据::新建(
                "Primitive3d",
                属性表([
                    ("形状", 属性值::枚举(枚举值::新建("Cuboid", 0))),
                    ("尺寸", 属性值::三维(尺寸.0, 尺寸.1, 尺寸.2)),
                    ("贴图", 资源值("assets/tps/environment/wall.png")?),
                    ("颜色", 属性值::颜色(颜色值::白())),
                    ("贴图重复", 属性值::二维(6.0, 1.0)),
                    ("受光", 属性值::布尔(true)),
                    ("双面", 属性值::布尔(false)),
                ]),
            ),
        )?;
    }

    // ---- 23 件掩体：位置与旋转逐条对应原项目 `掩体布局` ----
    for 条目 in 掩体布局 {
        let 标识 = 新建(&mut 编辑器, 条目.名称, Some(掩体组))?;
        let 旋转 = if 条目.旋转90 {
            旋转_九十度
        } else {
            (0.0, 0.0, 0.0, 1.0)
        };
        // 模型原点在底部且 XZ 居中：y=0 就是"底面落在竞技场上"。
        设变换(
            &mut 编辑器,
            标识,
            (条目.平面中心.0, 0.0, 条目.平面中心.1),
            旋转,
        )?;
        加组件(
            &mut 编辑器,
            标识,
            组件数据::新建(
                "Sprite",
                属性表([
                    ("模型", 资源值(条目.模型)?),
                    ("颜色", 属性值::颜色(颜色值::白())),
                    ("水平翻转", 属性值::布尔(false)),
                    ("垂直翻转", 属性值::布尔(false)),
                ]),
            ),
        )?;
    }

    // ---- 角色：都在中央掩体南侧，默认镜头一眼能看到双方 ----
    let 角色配置: [角色条目; 4] = [
        (
            "玩家士兵",
            (0.0, 0.0, 6.0),
            (0.0, 1.0, 0.0, 0.0),
            "assets/tps/characters/Character_Soldier.gltf",
        ),
        (
            "近战敌人",
            (-5.0, 0.0, 2.0),
            (0.0, 0.0, 0.0, 1.0),
            "assets/tps/characters/Character_Enemy.gltf",
        ),
        (
            "远程敌人",
            (5.0, 0.0, 1.0),
            (0.0, 0.0, 0.0, 1.0),
            "assets/tps/characters/Character_Hazmat.gltf",
        ),
        (
            "重装敌人",
            (9.0, 0.0, 3.0),
            (0.0, 0.0, 0.0, 1.0),
            "assets/tps/characters/Character_Hazmat.gltf",
        ),
    ];
    for (名称, 位置, 旋转, 模型) in 角色配置 {
        let 标识 = 新建(&mut 编辑器, 名称, Some(角色组))?;
        设变换(&mut 编辑器, 标识, 位置, 旋转)?;
        加组件(
            &mut 编辑器,
            标识,
            组件数据::新建(
                "Sprite",
                属性表([
                    ("模型", 资源值(模型)?),
                    ("颜色", 属性值::颜色(颜色值::白())),
                    ("水平翻转", 属性值::布尔(false)),
                    ("垂直翻转", 属性值::布尔(false)),
                ]),
            ),
        )?;
    }

    保存并收尾(
        &mut 编辑器,
        &临时根,
        &项目根,
        "scenes/tps_arena.scene.ron",
        Some("玩家士兵"),
    )
}

// ---------------------------------------------------------------------------
// layout.json 读取辅助
// ---------------------------------------------------------------------------

fn 取文本<'a>(值: &'a serde_json::Value, 键: &str) -> 结果<&'a str> {
    值.get(键)
        .and_then(|值| 值.as_str())
        .ok_or_else(|| format!("layout.json 的 `{键}` 缺失或不是字符串").into())
}

fn 取数值(值: &serde_json::Value, 键: &str) -> 结果<f64> {
    值.get(键)
        .and_then(|值| 值.as_f64())
        .ok_or_else(|| format!("layout.json 的 `{键}` 缺失或不是数字").into())
}
