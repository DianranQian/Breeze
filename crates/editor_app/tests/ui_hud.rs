//! 中文名：游戏内 UI 与 HUD
//! 界面组件的端到端验收：文档数据 → 运行时 Bevy UI 树 → 停止丢弃。
//!
//! # 为什么这些用例要同时看"运行时"与"文档"
//!
//! 界面组件有两个最容易"看起来接上了、实际没通"的接缝：
//!
//! 1. 运行时必须真的生成 `Node` / `Text` / `ImageNode`，而不是只把属性
//!    塞进实体（那样"运行"后屏幕上什么也不会有）；
//! 2. 界面树是**投影产物**，不能回写场景文档 —— 停止运行后文档里的实体与
//!    组件数量必须和启动前一致。
//!
//! 另外界面坐标是设计分辨率下的像素，这里用 1280×720（而不是默认的
//! 1920×1080）构建，专门守住"运行入口把项目设计分辨率传下去"这条线。

use bevy::prelude::*;
use bevy::ui::widget::ImageNode;

use editor_app::scene_bridge::{全部内置组件注册表, 运行时宿主};
use editor_app::ui_components::{
    界面图片属性, 界面图片组件名, 界面文本组件名, 界面组件名列表, 界面面板组件名,
};
use editor_app::ui_data::收集界面项;
use editor_app::ui_runtime::{运行界面来源, 运行界面根};
use scene_doc::document::{场景文档, 实体标识, 组件数据};
use scene_doc::path::项目相对路径;
use scene_doc::property::{属性值, 资源引用值, 颜色值};

fn 挂组件(
    文档: &mut 场景文档,
    名称: &str,
    组件名: &str,
    字段: &[(&str, 属性值)],
) -> 实体标识 {
    let 标识 = 文档.生成实体(名称, None);
    let 属性: std::collections::BTreeMap<String, 属性值> = 字段
        .iter()
        .map(|(键, 值)| ((*键).to_string(), 值.clone()))
        .collect();
    if let Some(实体) = 文档.取可变(标识) {
        实体.加组件(组件数据::新建(组件名, 属性));
    }
    标识
}

/// 面板 + 文本 + 图片的最小 HUD 场景。
fn 建界面场景() -> (场景文档, 实体标识, 实体标识, 实体标识) {
    let mut 文档 = 场景文档::新建("HUD");
    let 面板 = 挂组件(
        &mut 文档,
        "状态栏",
        界面面板组件名,
        &[
            ("宽度", 属性值::浮点(320.0)),
            ("高度", 属性值::浮点(180.0)),
            ("偏移X", 属性值::浮点(20.0)),
            ("偏移Y", 属性值::浮点(30.0)),
            (
                "锚点",
                属性值::枚举(scene_doc::property::枚举值::新建("BottomRight", 3)),
            ),
        ],
    );
    let 文本 = 挂组件(
        &mut 文档,
        "标题",
        界面文本组件名,
        &[
            ("文本", 属性值::文本("你好，Breeze".into())),
            ("字号", 属性值::浮点(24.0)),
        ],
    );
    let 图片 = 挂组件(
        &mut 文档,
        "徽标",
        界面图片组件名,
        &[
            ("宽度", 属性值::浮点(64.0)),
            ("高度", 属性值::浮点(64.0)),
            ("着色", 属性值::颜色(颜色值::srgb(0.0, 1.0, 0.0, 1.0))),
        ],
    );
    (文档, 面板, 文本, 图片)
}

#[test]
fn 运行时启动后界面落成真实节点() {
    let (文档, 面板, 文本, 图片) = 建界面场景();
    let 注册表 = 全部内置组件注册表();
    let 启动前组件数: usize = 文档.实体表.values().map(|实体| 实体.组件.len()).sum();

    let mut 宿主 = 运行时宿主::default();
    宿主
        .启动以设计分辨率(&文档, &注册表, (1280, 720))
        .expect("有实体的场景必须能启动");

    assert_eq!(宿主.上次统计.实体数, 3, "界面实体仍按文档实体计数");
    assert_eq!(宿主.上次统计.界面节点数, 3, "三个界面元素都要落成节点");
    assert!(
        !宿主.上次统计.跳过类型.contains(&界面图片组件名.to_string()),
        "UiImage 已注册并支持，不应出现在跳过列表"
    );

    let 世界 = 宿主.世界.as_mut().expect("运行中必须有世界");
    let 根 = 世界
        .query_filtered::<Entity, With<运行界面根>>()
        .iter(世界)
        .next()
        .expect("界面树必须有根节点");
    let 容器: Vec<Entity> = 世界
        .query_filtered::<Entity, With<运行界面来源>>()
        .iter(世界)
        .collect();
    assert_eq!(容器.len(), 3);
    for 实体 in &容器 {
        assert_eq!(
            世界.get::<ChildOf>(*实体).map(|父| 父.parent()),
            Some(根),
            "每个界面元素都必须挂到界面根下"
        );
    }

    // 右下锚点：1280-320-20 = 940，720-180-30 = 510。
    let 面板容器 = 容器
        .iter()
        .copied()
        .find(|实体| 世界.get::<运行界面来源>(*实体).unwrap().0 == 面板)
        .expect("面板必须有对应容器");
    let 面板布局 = 世界.get::<Node>(面板容器).unwrap();
    assert_eq!(面板布局.left, Val::Px(940.0));
    assert_eq!(面板布局.top, Val::Px(510.0));
    assert_eq!(面板布局.width, Val::Px(320.0));
    assert_eq!(面板布局.height, Val::Px(180.0));

    // 文本落成 Text 子节点。
    let 文本节点: Vec<Entity> = 世界
        .query_filtered::<Entity, (With<Text>, With<TextFont>)>()
        .iter(世界)
        .collect();
    assert_eq!(文本节点.len(), 1);
    assert_eq!(
        世界.get::<Text>(文本节点[0]).unwrap().0,
        "你好，Breeze",
        "文本内容必须原样进入运行时"
    );
    let 文本来源 = 世界
        .get::<ChildOf>(文本节点[0])
        .map(|父| 父.parent())
        .expect("文本必须挂在界面容器下");
    assert_eq!(世界.get::<运行界面来源>(文本来源).unwrap().0, 文本);

    // 图片落成 ImageNode，着色生效。
    let 图片容器 = 容器
        .iter()
        .copied()
        .find(|实体| 世界.get::<运行界面来源>(*实体).unwrap().0 == 图片)
        .expect("图片必须有对应容器");
    let 图片节点 = 世界
        .get::<ImageNode>(图片容器)
        .expect("图片容器必须带 ImageNode");
    let 颜色 = 图片节点.color.to_srgba();
    assert!(
        (颜色.red).abs() < 1e-6 && (颜色.green - 1.0).abs() < 1e-6,
        "着色必须写到图片节点上，实际 {颜色:?}"
    );

    // 停止：运行时世界整体丢弃，文档一点没动。
    宿主.停止();
    assert!(宿主.世界.is_none(), "停止后运行时世界必须消失");
    let 启动后组件数: usize = 文档.实体表.values().map(|实体| 实体.组件.len()).sum();
    assert_eq!(启动后组件数, 启动前组件数, "界面投影不得回写文档");
    assert_eq!(文档.实体表.len(), 3, "界面投影不得往文档里加实体");
}

#[test]
fn 预览与运行时读到同一份界面投影() {
    let (文档, 面板, 文本, 图片) = 建界面场景();
    let 项 = 收集界面项(&文档);
    assert_eq!(项.len(), 3);
    let 按来源 = |标识: 实体标识| 项.iter().find(|项| 项.来源 == 标识).cloned().unwrap();
    assert_eq!(按来源(面板).尺寸, (320.0, 180.0));
    assert_eq!(
        按来源(面板).锚点,
        editor_core::ui_layout::界面锚点::右下,
        "面板的锚点必须被共享投影读出来"
    );
    assert!(按来源(文本).内容.is_some());
    assert!(按来源(图片).内容.is_some());
}

#[test]
fn 图片组件在检查器里可编辑() {
    let 注册表 = 全部内置组件注册表();
    assert!(界面组件名列表().contains(&界面图片组件名));
    let 类型 = 注册表.要求(界面图片组件名).expect("UiImage 必须已注册");

    let 字段: Vec<&str> = 类型
        .字段描述()
        .iter()
        .map(|描述| 描述.字段名.as_str())
        .collect();
    assert_eq!(
        字段,
        vec!["锚点", "宽度", "高度", "偏移X", "偏移Y", "贴图", "着色"],
        "Inspector 字段是用户能改的全部属性"
    );
    let 贴图 = 类型
        .字段描述()
        .iter()
        .find(|描述| 描述.字段名 == "贴图")
        .expect("必须有贴图字段");
    assert!(
        matches!(
            贴图.种类,
            scene_doc::registry::属性种类::资源 {
                类别: scene_doc::property::资源类别::图片
            }
        ),
        "贴图字段必须走图片资源选择器"
    );

    // 默认值 → 实例化 → 导出，字段必须一一对上。
    let mut 实例 = 类型.默认实例();
    let mut 属性 = std::collections::BTreeMap::new();
    for 描述 in 类型.字段描述() {
        属性.insert(描述.字段名.clone(), 描述.默认值.clone());
    }
    let 路径 = 项目相对路径::解析("assets/ui/logo.png").unwrap();
    属性.insert(
        "贴图".to_string(),
        属性值::资源(资源引用值::按路径(路径.clone())),
    );
    assert!(类型.实例化(实例.as_mut(), &属性));
    let 值 = 实例.downcast_ref::<界面图片属性>().unwrap();
    assert_eq!(值.贴图路径.as_ref(), Some(&路径));
    let 导出 = 类型.导出(实例.as_ref()).unwrap();
    assert_eq!(
        导出
            .get("贴图")
            .and_then(属性值::取资源)
            .map(|引用| &引用.路径),
        Some(&路径)
    );
}
