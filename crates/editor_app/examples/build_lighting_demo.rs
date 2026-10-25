//! 中文名：构建灯光演示项目
//! 生成一个"带灯光的三维演示项目"，供 GUI 验收与手工试用。
//!
//! 运行：
//!
//! ```powershell
//! cargo run -p editor_app --example build_lighting_demo
//! # 可选：指定输出目录
//! cargo run -p editor_app --example build_lighting_demo -- .verify/runs/lighting_demo
//! ```
//!
//! # 为什么用代码生成而不是往仓库里塞一个 `.scene.ron`
//!
//! 场景文件里全是 UUID 与属性表，手写既容易写错、也没法保证和当前
//! 序列化格式一致。用真实 API 构造文档再保存，得到的文件**一定是**编辑器
//! 自己能读回来的格式；改动字段时这个生成器会跟着编译期检查一起更新。
//!
//! 它同时是"三维灯光"这个功能的固定夹具：GUI 验收用它打开一个有方向光与
//! 点光的场景，截图里才能看到真实的明暗关系，而不是靠默认光"看起来还行"。

use editor_core::project::项目描述;
use scene_doc::document::{变换数据, 场景文档, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 枚举值, 颜色值};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let 目标 = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".verify/runs/lighting_demo".to_string());
    let 根 = std::path::PathBuf::from(&目标);
    // 重复生成时先清空：保证每次得到的项目内容完全一致。
    if 根.exists() {
        std::fs::remove_dir_all(&根)?;
    }
    if let Some(父) = 根.parent() {
        std::fs::create_dir_all(父)?;
    }
    let mut 项目 = 项目描述::创建(&根, "灯光演示")?;

    let mut 文档 = 场景文档::新建("lights");
    加网格实体(
        &mut 文档,
        "地面",
        (0.0, 0.0, 0.0),
        "Cuboid",
        (20.0, 0.4, 20.0),
        颜色值::srgb(0.55, 0.55, 0.58, 1.0),
    );
    加网格实体(
        &mut 文档,
        "箱子",
        (2.0, 1.2, 0.0),
        "Cuboid",
        (1.6, 2.4, 1.6),
        颜色值::srgb(0.85, 0.7, 0.45, 1.0),
    );
    加网格实体(
        &mut 文档,
        "矮墙",
        (-3.0, 1.0, 1.5),
        "Cuboid",
        (0.5, 2.0, 4.0),
        颜色值::srgb(0.6, 0.75, 0.8, 1.0),
    );

    // 方向光：暖白色，从右上前方斜照下来。
    //
    // 光的方向由实体旋转决定（Bevy 的方向光沿自身 -Z 照射），
    // 因此这里给出"绕 Y 转 -35°、再绕 X 转 -50°"的旋转：
    // 箱子左侧留出阴影，右侧被照亮，一眼能看出光是从右上来的。
    let 主光 = 文档.生成实体("主光（方向光）", None);
    if let Some(实体) = 文档.取可变(主光) {
        实体.变换 = Some(变换数据 {
            平移: (6.0, 10.0, 6.0),
            旋转: 欧拉角转四元数(-35.0, -50.0, 0.0),
            缩放: (1.0, 1.0, 1.0),
        });
        实体.加组件(组件数据::新建(
            "方向光",
            [
                (
                    "颜色".to_string(),
                    属性值::颜色(颜色值::srgb(1.0, 0.94, 0.85, 1.0)),
                ),
                ("照度".to_string(), 属性值::浮点(12_000.0)),
                ("投射阴影".to_string(), 属性值::布尔(true)),
            ]
            .into_iter()
            .collect(),
        ));
    }

    // 点光：偏红的补光，放在箱子右前方，用来验证"距离衰减"。
    let 补光 = 文档.生成实体("补光（点光）", None);
    if let Some(实体) = 文档.取可变(补光) {
        实体.变换 = Some(变换数据 {
            平移: (4.5, 2.0, 3.0),
            ..变换数据::单位()
        });
        实体.加组件(组件数据::新建(
            "点光",
            [
                (
                    "颜色".to_string(),
                    属性值::颜色(颜色值::srgb(1.0, 0.55, 0.45, 1.0)),
                ),
                ("强度".to_string(), 属性值::浮点(2_600.0)),
                ("半径".to_string(), 属性值::浮点(14.0)),
            ]
            .into_iter()
            .collect(),
        ));
    }

    let 场景路径 = 项目相对路径::解析("scenes/lights.scene.ron")?;
    项目.保存场景(&场景路径, &文档)?;
    项目.清单可变().启动场景 = Some(场景路径);
    项目.保存清单()?;

    println!("已生成灯光演示项目：{}", 根.display());
    println!(
        "打开方式：$env:BEVY_STUDIO_OPEN_PROJECT=\"{}\"; $env:BEVY_STUDIO_MODE=\"3d\"",
        根.display()
    );
    Ok(())
}

/// 加一个 `Primitive3d` 网格实体。
fn 加网格实体(
    文档: &mut 场景文档,
    名称: &str,
    位置: (f32, f32, f32),
    形状: &str,
    尺寸: (f32, f32, f32),
    颜色: 颜色值,
) {
    let 标识 = 文档.生成实体(名称, None);
    let Some(实体) = 文档.取可变(标识) else {
        return;
    };
    实体.变换 = Some(变换数据 {
        平移: 位置,
        ..变换数据::单位()
    });
    实体.加组件(组件数据::新建(
        "Primitive3d",
        [
            ("形状".to_string(), 属性值::枚举(枚举值::新建(形状, 0))),
            ("尺寸".to_string(), 属性值::三维(尺寸.0, 尺寸.1, 尺寸.2)),
            ("颜色".to_string(), 属性值::颜色(颜色)),
            ("受光".to_string(), 属性值::布尔(true)),
            ("双面".to_string(), 属性值::布尔(false)),
            ("贴图重复".to_string(), 属性值::二维(1.0, 1.0)),
        ]
        .into_iter()
        .collect(),
    ));
}

/// 欧拉角（度，Y→X→Z 顺序）转四元数。
///
/// 示例里手写四元数极易算错方向，而"光照方向不对"在截图里很难判断，
/// 所以这里用公式算：先绕 Y 偏航、再绕 X 俯仰。
fn 欧拉角转四元数(偏航: f32, 俯仰: f32, 滚转: f32) -> (f32, f32, f32, f32) {
    let (偏航, 俯仰, 滚转) = (
        偏航.to_radians() * 0.5,
        俯仰.to_radians() * 0.5,
        滚转.to_radians() * 0.5,
    );
    let (sy, cy) = 偏航.sin_cos();
    let (sx, cx) = 俯仰.sin_cos();
    let (sz, cz) = 滚转.sin_cos();
    let w = cy * cx * cz + sy * sx * sz;
    let x = cy * sx * cz + sy * cx * sz;
    let y = sy * cx * cz - cy * sx * sz;
    let z = cy * cx * sz - sy * sx * cz;
    (x, y, z, w)
}
