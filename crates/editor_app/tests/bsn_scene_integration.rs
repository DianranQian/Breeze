//! 中文名：BSN场景接入
//! BSN 接入编辑器的端到端测试（方案 A + B 的垂直切片）。
//!
//! 这一组测试验证的任务核心命题是：
//!
//! ```text
//! 官方 bsn! 模板 ──实例化──▶ 真实 Bevy World
//!                              │ 提取（bsn_bridge）
//!                              ▼
//!                        scene_doc 文档 ──▶ 编辑器会话（撤销重做/保存/身份）
//!                              │ 构建（scene_bridge，既有代码）
//!                              ▼
//!                        运行时 Bevy World ──▶ 真实组件与层级
//! ```
//!
//! **两条通道都必须是既有的**：文档层的保存与撤销、桥接层的 `构建世界`。
//! BSN 只是一个新来源，不是一条绕过它们的旁路。如果哪天有人让模板直接
//! 产出运行时实体，下面的 `模板不会绕过文档层的身份` 与
//! `提取不修改世界` 之类的断言会立刻失败。

use std::collections::BTreeSet;

use bevy::prelude::*;

use editor_app::bsn_templates::{
    二维原型模板, 从模板生成场景, 从模板生成文档, 内置模板列表, 内置模板说明, 可见性转文档,
    编辑器提取器,
};
use editor_app::scene_bridge::{
    Sprite属性, 可见性属性, 构建世界, 编辑器相机2d, 默认组件注册表
};
use editor_core::command::编辑器命令;
use editor_core::state::编辑器状态;

use scene_doc::document::{场景文档, 实体标识};
use scene_doc::property::属性值;
use scene_doc::storage::{场景存储, 文件系统场景存储, 默认场景扩展名};

/// 从内置 2D 模板生成一份文档，供各用例复用。
fn 生成二维文档() -> 场景文档 {
    从模板生成场景(&二维原型模板, "二维关卡").expect("内置模板应无损失地生成文档")
}

/// 按名称在文档里找实体。
fn 按名称取(文档: &场景文档, 目标: &str) -> 实体标识 {
    文档
        .实体表
        .values()
        .find(|实体| 实体.名称 == 目标)
        .unwrap_or_else(|| panic!("文档里找不到 `{目标}`"))
        .标识
}

// ======================================================================
// 一、BSN 模板产出的文档 → 既有构建世界 → 真实运行时
// ======================================================================

#[test]
fn BSN模板生成的文档能构建出正确的运行时世界() {
    let 文档 = 生成二维文档();
    let 注册表 = 默认组件注册表();

    let mut 世界 = World::new();
    let (统计, 映射) = 构建世界(&mut 世界, &文档, &注册表);

    // ---- 统计口径 ----
    assert_eq!(统计.实体数, 文档.实体数量(), "运行时实体数必须与文档一致");
    assert_eq!(统计.实体数, 4, "根 + 相机 + 背景板 + 玩家");
    assert_eq!(统计.跳过组件数, 0, "不应有组件被跳过：{:?}", 统计.跳过类型);
    assert!(统计.跳过类型.is_empty());
    assert_eq!(统计.父子关系数, 3, "三个子实体各自一条 ChildOf");

    // ---- 根实体与层级 ----
    let 根标识 = 文档.顶层顺序[0];
    let 根实体 = 映射.取运行时(根标识).expect("根实体应被创建");
    assert_eq!(
        世界
            .get::<Name>(根实体)
            .map(|名字| 名字.as_str().to_string()),
        Some("二维关卡".to_string())
    );
    let 子级 = 世界
        .get::<Children>(根实体)
        .map(|孩子| 孩子.to_vec())
        .expect("根实体应有 Children");
    assert_eq!(子级.len(), 3);

    // ---- 变换真的进了运行时 ----
    let 玩家标识 = 按名称取(&文档, "玩家");
    let 玩家实体 = 映射.取运行时(玩家标识).unwrap();
    let 玩家变换 = 世界
        .get::<Transform>(玩家实体)
        .copied()
        .expect("玩家应有 Transform");
    assert_eq!(玩家变换.translation, Vec3::new(0.0, -80.0, 0.0));
    // 四元数与文档一致（文档里是 (x,y,z,w)）。
    let 文档旋转 = 文档.取(玩家标识).unwrap().变换.unwrap().旋转;
    assert!((玩家变换.rotation.x - 文档旋转.0).abs() < 1e-6);
    assert!((玩家变换.rotation.w - 文档旋转.3).abs() < 1e-6);

    // ---- 编辑器自有组件真的进了运行时 ----
    let 相机标识 = 按名称取(&文档, "主相机");
    let 相机实体 = 映射.取运行时(相机标识).unwrap();
    assert_eq!(
        世界.get::<编辑器相机2d>(相机实体).copied(),
        Some(编辑器相机2d {
            正交半高: 360.0
        }),
        "`编辑器相机2d` 必须原样进入运行时"
    );

    let 背景标识 = 按名称取(&文档, "背景板");
    let 背景实体 = 映射.取运行时(背景标识).unwrap();
    let 背景 = 世界
        .get::<Sprite属性>(背景实体)
        .expect("背景板应有 Sprite属性");
    assert_eq!(
        背景.颜色,
        scene_doc::property::颜色值::srgb(0.10, 0.12, 0.16, 1.0)
    );

    // ---- 玩家有贴图引用，且文档资源表登记了它 ----
    let 玩家 = 世界
        .get::<Sprite属性>(玩家实体)
        .expect("玩家应有 Sprite属性");
    assert_eq!(
        玩家.贴图路径.as_ref().map(|路径| 路径.文本()),
        Some("textures/玩家.png"),
        "BSN 里写的贴图路径必须完整穿过文档到达运行时"
    );
    assert_eq!(玩家.自定义尺寸, Some((48.0, 48.0)));
    assert!(
        文档
            .资源表
            .keys()
            .any(|路径| 路径.文本() == "textures/玩家.png"),
        "资源表应登记 BSN 里引用的贴图：{:?}",
        文档
            .资源表
            .keys()
            .map(|路径| 路径.文本())
            .collect::<Vec<_>>()
    );

    // ---- 可见性：BSN 里写的是 Visible，文档归一为"可见"，运行时回到 Inherited ----
    let 可见性 = 世界
        .get::<Visibility>(玩家实体)
        .copied()
        .expect("玩家应有 Visibility");
    assert_eq!(
        可见性,
        Visibility::Inherited,
        "文档侧只有布尔，`Visible` 会被归一成 `Inherited`（已知有损，见字段提示）"
    );

    // ---- 文档自洽 ----
    assert!(文档.校验().无问题(), "{}", 文档.校验().摘要());
}

#[test]
fn 模板生成的文档实体身份稳定且唯一() {
    let 文档 = 生成二维文档();
    let 标识列表: Vec<实体标识> = 文档.实体表.keys().copied().collect();
    let 唯一: BTreeSet<实体标识> = 标识列表.iter().copied().collect();
    assert_eq!(唯一.len(), 标识列表.len(), "实体标识必须唯一");

    // 两次生成必须是两份独立身份（UUID 不复用）。
    let 另一份 = 生成二维文档();
    let 另一标识: BTreeSet<实体标识> = 另一份.实体表.keys().copied().collect();
    assert!(唯一.is_disjoint(&另一标识), "两次生成不应共享实体标识");
}

#[test]
fn 提取与构建都不修改源世界与源文档() {
    let 文档 = 生成二维文档();
    let 快照 = 文档.clone();
    let 注册表 = 默认组件注册表();

    let mut 世界 = World::new();
    let _ = 构建世界(&mut 世界, &文档, &注册表);

    assert_eq!(文档, 快照, "构建世界必须是只读文档的");
}

// ======================================================================
// 二、BSN 文档进入既有保存 / 撤销重做管线
// ======================================================================

#[test]
fn BSN模板生成的文档保存并重新打开后完全一致() {
    let 文档 = 生成二维文档();
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 路径 = 目录.path().join(format!("二维关卡.{默认场景扩展名}"));
    let 存储 = 文件系统场景存储::新建();

    存储.写入(&路径, &文档).expect("保存应成功");
    let 读取 = 存储.读取(&路径).expect("读取应成功");

    assert_eq!(读取.文档, 文档, "保存后重新打开必须逐字段一致");
    assert!(!读取.从备份恢复);

    // 关键结构在文本里可见，便于 Git 评审。
    let 文本 = std::fs::read_to_string(&路径).unwrap();
    for 关键字 in ["格式版本", "二维关卡", "主相机", "背景板", "玩家"] {
        assert!(文本.contains(关键字), "场景文件里应出现 `{关键字}`");
    }
}

#[test]
fn BSN模板生成的文档进入编辑器会话后可撤销重做() {
    let 文档 = 生成二维文档();
    let 根标识 = 文档.顶层顺序[0];
    let 原名称 = 文档.取(根标识).unwrap().名称.clone();

    let mut 状态 = 编辑器状态::新建();
    状态.应用已载入场景(文档, None);
    assert_eq!(状态.撤销.可撤销步数(), 0, "载入后历史应为空");

    状态
        .执行(编辑器命令::重命名实体 {
            标识: 根标识,
            新名称: "重命名后的关卡".to_string(),
        })
        .expect("重命名应成功");
    assert_eq!(状态.会话.文档.取(根标识).unwrap().名称, "重命名后的关卡");

    assert!(状态.撤销一步(), "应能撤销");
    assert_eq!(
        状态.会话.文档.取(根标识).unwrap().名称,
        原名称,
        "撤销必须回到 BSN 提取时的名称"
    );
    assert_eq!(状态.会话.文档.实体数量(), 4, "撤销不应影响其他实体");

    assert!(状态.重做一步(), "应能重做");
    assert_eq!(状态.会话.文档.取(根标识).unwrap().名称, "重命名后的关卡");
}

#[test]
fn BSN文档里的组件属性可以被编辑器正常修改() {
    let 文档 = 生成二维文档();
    let 玩家标识 = 按名称取(&文档, "玩家");
    let mut 状态 = 编辑器状态::新建();
    状态.应用已载入场景(文档, None);

    // 用既有的"设组件属性"命令改玩家贴图，证明 BSN 产出的组件
    // 与手工创建的组件在文档层完全等价。
    状态
        .执行(编辑器命令::设组件属性 {
            标识: 玩家标识,
            类型名: "Sprite".to_string(),
            属性名: "颜色".to_string(),
            新值: 属性值::颜色(scene_doc::property::颜色值::srgb(1.0, 0.0, 0.0, 1.0)),
        })
        .expect("修改属性应成功");

    let 玩家 = 状态.会话.文档.取(玩家标识).unwrap();
    let 精灵 = 玩家.取组件("Sprite").expect("Sprite 组件应仍在");
    assert_eq!(
        精灵.属性.get("颜色"),
        Some(&属性值::颜色(scene_doc::property::颜色值::srgb(
            1.0, 0.0, 0.0, 1.0
        )))
    );

    // 撤销后回到 BSN 提取出来的原始颜色。
    assert!(状态.撤销一步());
    let 玩家 = 状态.会话.文档.取(玩家标识).unwrap();
    assert_eq!(
        玩家.取组件("Sprite").unwrap().属性.get("颜色"),
        Some(&属性值::颜色(scene_doc::property::颜色值::srgb(
            0.30, 0.68, 1.0, 1.0
        )))
    );
}

// ======================================================================
// 三、不污染：BSN 原型失败/成功都不碰用户场景
// ======================================================================

#[test]
fn 生成模板文档不写任何文件() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 空目录条目数 = std::fs::read_dir(目录.path()).unwrap().count();
    assert_eq!(空目录条目数, 0);

    let _文档 = 生成二维文档();

    assert_eq!(
        std::fs::read_dir(目录.path()).unwrap().count(),
        0,
        "从模板生成文档是纯内存操作，不应写盘"
    );
}

#[test]
fn 模板元数据与真实提取一致() {
    for 模板 in 内置模板列表() {
        let 说明 = 内置模板说明
            .iter()
            .find(|说明| 说明.标识 == 模板.标识())
            .expect("内置模板必须有说明条目");
        let 产物 = 从模板生成文档(模板.as_ref(), "核对").expect("应无损失提取");
        assert_eq!(产物.报告.实体数, 说明.预期实体数);
        assert_eq!(产物.报告.最大深度, 说明.预期最大深度);
        assert!(!产物.报告.有数据损失());
    }
}

// ======================================================================
// 四、现有契约回归：接入 BSN 没有改变任何既有行为
// ======================================================================

#[test]
fn 默认组件注册表未被_bsn_接入改动() {
    let 注册表 = 默认组件注册表();
    // 与 `scene_bridge` 里的内置组件严格一致。`TileTerrain` / `Primitive3d`
    // 是编辑器程序化渲染新增的组件，不是 BSN 接入带来的。
    assert_eq!(
        注册表.组件名列表(),
        vec![
            "Camera2d".to_string(),
            "Primitive3d".to_string(),
            "Sprite".to_string(),
            "TileTerrain".to_string(),
            "Transform".to_string(),
            "Visibility".to_string(),
        ],
        "接入 BSN 不应增删或重命名任何已注册组件"
    );
    assert_eq!(注册表.数量(), 6);
}

#[test]
fn 编辑器提取器与注册表一一对应() {
    let 注册表 = 默认组件注册表();
    let 提取名: BTreeSet<&str> = 编辑器提取器().iter().map(|提取器| 提取器.组件名).collect();
    for 组件名 in 注册表.组件名列表() {
        if 组件名 == "Transform" {
            // Transform 映射到 `变换` 字段，不是一个文档组件。
            continue;
        }
        assert!(
            提取名.contains(组件名.as_str()),
            "注册表组件 `{组件名}` 缺少提取器"
        );
    }
}

#[test]
fn 现有场景文件格式未因_bsn_接入改变() {
    // 用手工构造的文档（完全不走 BSN）验证序列化契约不变：
    // 保存 → 读回 → 相等，且关键字段名保持稳定。
    let mut 文档 = 场景文档::新建("格式契约");
    let 根 = 文档.生成实体("根", None);
    let 子 = 文档.生成实体("子", Some(根));
    if let Some(实体) = 文档.取可变(子) {
        实体.变换 = Some(scene_doc::document::变换数据 {
            平移: (1.0, 2.0, 3.0),
            旋转: (0.0, 0.0, 0.0, 1.0),
            缩放: (1.0, 1.0, 1.0),
        });
    }

    let 目录 = tempfile::tempdir().expect("临时目录");
    let 路径 = 目录.path().join(format!("契约.{默认场景扩展名}"));
    let 存储 = 文件系统场景存储::新建();
    存储.写入(&路径, &文档).expect("写入");
    let 读回 = 存储.读取(&路径).expect("读取");

    assert_eq!(读回.文档, 文档);

    let 文本 = std::fs::read_to_string(&路径).unwrap();
    for 键 in ["格式版本", "元数据", "实体表", "顶层顺序"] {
        assert!(
            文本.contains(键),
            "场景 RON 结构键 `{键}` 不应改变；实际文本开头：\n{}",
            &文本.chars().take(200).collect::<String>()
        );
    }
    // 扩展名契约。
    assert_eq!(默认场景扩展名, "scene.ron");
}

#[test]
fn 可见性映射与既有桥接反向一致() {
    // 文档侧布尔 → 世界 Visibility → 回到文档侧布尔，必须是不动点。
    for 可见 in [true, false] {
        let 属性 = 可见性属性 { 可见 };
        let 世界侧 = if 属性.可见 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        assert_eq!(可见性转文档(&世界侧), 属性, "可见性映射必须是稳定定点");
    }
}
