//! 中文名：资源选择器网格视图
//! 网格视图的端到端验证：真实项目 + 真实资源注册表 + 无头 egui 逐帧驱动。
//!
//! # 为什么按矩形点，不按「第 n 张卡」点
//!
//! 网格是 `horizontal_wrapped` 排出来的，卡片落在第几行第几列取决于窗口宽度、
//! 字体度量和滚动条出现与否。测试里如果写「点 (120, 80)」，任何一次改卡片尺寸
//! 或改面板内边距都会让它点到隔壁的卡片上 —— 而 egui 不会报错，只会返回另一个
//! 资源的标识，于是这个测试就变成了「碰巧点对了材质」。
//! 所以这里读面板自己登记的 [`选择器卡片几何`]：那是本帧真实矩形。
//!
//! # 三件必须成立的事
//!
//! 1. 点材质卡片 → 返回的动作带的是**注册表里那份稳定标识**，不是路径字符串；
//!    否则文件改名后引用接不回来，而测试照样是绿的。
//! 2. 类别不匹配的卡片**不可选**且点下去不产生任何动作 —— 它要留在原地让用户
//!    看得见（否则用户以为文件没了），但不能被写进材质槽。
//! 3. 卡片几何只在网格视图出现；列表视图清空它，浮层关闭后也不留残影。

use std::collections::BTreeMap;

use editor_core::material::材质资源;
use editor_core::project::项目描述;
use editor_core::state::编辑器状态;
use editor_ui::asset_picker;
use editor_ui::i18n::语言;
use editor_ui::state::{
    UiAction, 界面状态, 资源落点, 资源选择器状态, 选择器卡片几何, 选择器视图
};
use editor_ui::theme::编辑器配色;
use scene_doc::document::实体标识;
use scene_doc::path::项目相对路径;
use scene_doc::property::资源类别;
use uuid::Uuid;

/// 材质资源的相对路径：注册表刷新后应成为类别为「材质」的条目。
const 材质路径: &str = "assets/materials/石板.material.ron";
/// 图片资源的相对路径：用来验证「类别不匹配的卡片不可选」。
const 贴图路径: &str = "assets/贴图/屋顶.png";

fn 取路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("合法路径")
}

/**
 * 建一个同时含材质与图片、注册表已刷新的编辑器状态。
 *
 * 材质用真实写入器 `保存材质` 生成，图片写几个字节的占位内容（选择器只读
 * 注册表元数据，不解码图片）。`TempDir` 必须交给调用方持有：它一析构，
 * 项目根就没了，后续动作里按 UID 回连路径会失败。
 */
fn 建编辑器(用例: &str) -> (tempfile::TempDir, 编辑器状态, Uuid) {
    let 目录 = tempfile::tempdir().expect("创建临时目录");
    let 根 = 目录.path().join(用例);
    项目描述::创建(&根, 用例).expect("创建项目");

    let mut 编辑器 = 编辑器状态::新建();
    编辑器.打开项目(&根).expect("打开项目");

    编辑器
        .项目
        .as_mut()
        .expect("打开项目后应有项目")
        .保存材质(&取路径(材质路径), &材质资源::新建("石板路"))
        .expect("写入材质");
    std::fs::create_dir_all(根.join("assets/贴图")).expect("创建资源目录");
    std::fs::write(根.join(贴图路径), [1u8; 4]).expect("写入图片");

    // 新建的文件先进标识表，再刷新注册表：注册条目里的标识来自同一张表。
    if let Some(项目) = 编辑器.项目.as_mut() {
        项目.同步资源标识().expect("同步资源标识");
    }
    编辑器.刷新资源注册表();

    let 标识 = 编辑器
        .资源注册表
        .取(&取路径(材质路径))
        .expect("材质应被扫描进注册表")
        .唯一标识
        .expect("注册条目应带稳定标识");
    (目录, 编辑器, 标识)
}

/// 给材质字段打开选择器；`视图` 决定列表还是网格。
fn 打开选择器(界面: &mut 界面状态, 视图: 选择器视图) {
    界面.资源选择器 = Some(资源选择器状态 {
        落点: 资源落点::组件字段 {
            实体: 实体标识(Uuid::from_u128(42)),
            组件: "Mesh3d".to_string(),
        },
        属性: "材质".to_string(),
        类别: 资源类别::材质,
        搜索: String::new(),
        // 「全部」页签：让贴图也出现在网格里，才能验证它不可选。
        类别筛选: None,
        视图,
    });
}

/**
 * 无头跑一帧选择器，返回它排进队列的动作。
 *
 * `按下` / `抬起` 各是一帧的指针状态。为什么要点在**两帧**里发：
 * egui 的命中测试在 `begin_pass` 里用**上一帧**登记的 widget 矩形算
 * （`Context::begin_pass` → `hit_test(&prev_pass.widgets, …)`），
 * 同一帧内按下并抬起会被判定成"这一帧才知道按下的是什么"，点击落空。
 * 真人点击天然跨帧，所以这不是绕过缺陷，而是复现真实时序。
 *
 * `输出.textures_delta` 必须显式清空，否则 epaint 在 drop 时 panic。
 */
fn 跑一帧(
    egui上下文: &egui::Context,
    编辑器: &编辑器状态,
    界面: &mut 界面状态,
    指针: Option<egui::Pos2>,
    按下: bool,
    抬起: bool,
) -> Option<UiAction> {
    let mut 事件 = Vec::new();
    if let Some(点) = 指针 {
        事件.push(egui::Event::PointerMoved(点));
        if 按下 {
            事件.push(egui::Event::PointerButton {
                pos: 点,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if 抬起 {
            事件.push(egui::Event::PointerButton {
                pos: 点,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    egui上下文.begin_pass(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        events: 事件,
        ..Default::default()
    });
    let 动作 = asset_picker::绘制(
        egui上下文,
        编辑器,
        界面,
        &BTreeMap::new(),
        编辑器配色::default(),
        语言::中文,
    );
    let mut 输出 = egui上下文.end_pass();
    输出.textures_delta.clear();
    动作
}

/// 一帧静态重绘：指针不动、不按键，只让面板把本帧几何登记出来。
fn 静态帧(
    egui上下文: &egui::Context,
    编辑器: &编辑器状态,
    界面: &mut 界面状态,
) -> Option<UiAction> {
    跑一帧(egui上下文, 编辑器, 界面, None, false, false)
}

/**
 * 在指定矩形上完成一次真实时序的点击，返回最后一帧（抬起帧）产出的动作。
 *
 * 悬停一帧 → 按下（此时上一帧的矩形已登记，命中测试才认得这张卡片）→ 抬起。
 * 中间两帧不应产生动作：选中只在抬起那一帧成立。
 */
fn 点卡片(
    egui上下文: &egui::Context,
    编辑器: &编辑器状态,
    界面: &mut 界面状态,
    矩形: egui::Rect,
) -> Option<UiAction> {
    let 中心 = 矩形.center();
    assert!(
        矩形.is_positive(),
        "卡片矩形应当有面积，否则点不到：{矩形:?}"
    );
    assert_eq!(
        跑一帧(egui上下文, 编辑器, 界面, Some(中心), false, false),
        None,
        "悬停帧不该产生动作"
    );
    assert_eq!(
        跑一帧(egui上下文, 编辑器, 界面, Some(中心), true, false),
        None,
        "只按下不抬起不该产生动作"
    );
    跑一帧(egui上下文, 编辑器, 界面, Some(中心), false, true)
}

/// 先跑一帧登记几何，再返回指定路径那张卡片的矩形。
fn 卡片矩形(
    egui上下文: &egui::Context,
    编辑器: &编辑器状态,
    界面: &mut 界面状态,
    路径: &str,
) -> egui::Rect {
    静态帧(egui上下文, 编辑器, 界面);
    找几何(&界面.选择器卡片几何, 路径).卡片矩形
}

fn 找几何<'a>(几何: &'a [选择器卡片几何], 路径: &str) -> &'a 选择器卡片几何 {
    几何
        .iter()
        .find(|卡片| 卡片.路径 == 路径)
        .unwrap_or_else(|| panic!("网格应登记 {路径} 的卡片几何"))
}

#[test]
fn 网格视图点材质卡片返回注册表里的稳定标识() {
    let (_目录, 编辑器, 材质标识) = 建编辑器("网格选材质");
    let mut 界面 = 界面状态::新建();
    打开选择器(&mut 界面, 选择器视图::网格);
    let egui上下文 = egui::Context::default();

    let 矩形 = 卡片矩形(&egui上下文, &编辑器, &mut 界面, 材质路径);
    let 动作 = 点卡片(&egui上下文, &编辑器, &mut 界面, 矩形);

    assert_eq!(
        动作,
        Some(UiAction::设置资源引用 {
            实体: 实体标识(Uuid::from_u128(42)),
            组件: "Mesh3d".to_string(),
            属性: "材质".to_string(),
            资源标识: 材质标识,
        }),
        "点卡片必须回写落点三件套与注册表里那份稳定标识"
    );
    assert!(
        界面.资源选择器.is_none(),
        "选中后浮层应当关闭，不能留着让用户重复选"
    );
}

#[test]
fn 网格视图类别不匹配的卡片不可选且点击无动作() {
    let (_目录, 编辑器, _材质标识) = 建编辑器("网格不匹配");
    let mut 界面 = 界面状态::新建();
    打开选择器(&mut 界面, 选择器视图::网格);
    let egui上下文 = egui::Context::default();

    // 先跑一帧：把两张卡片的几何与可选标记都登记出来。
    静态帧(&egui上下文, &编辑器, &mut 界面);
    assert!(
        找几何(&界面.选择器卡片几何, 材质路径).可选,
        "类别相符且有标识的材质卡片必须可选"
    );
    assert!(
        !找几何(&界面.选择器卡片几何, 贴图路径).可选,
        "贴图卡片要留在网格里让用户看得见，但类别不匹配就不可选"
    );

    let 贴图矩形 = 找几何(&界面.选择器卡片几何, 贴图路径).卡片矩形;
    let 动作 = 点卡片(&egui上下文, &编辑器, &mut 界面, 贴图矩形);

    assert_eq!(动作, None, "点不可选卡片不能产生任何写文档的动作");
    assert!(
        界面.资源选择器.is_some(),
        "误点不改变选择器状态，浮层保持打开"
    );
}

#[test]
fn 卡片几何只在网格视图出现且随视图切换清空() {
    let (_目录, 编辑器, _材质标识) = 建编辑器("网格几何开关");
    let egui上下文 = egui::Context::default();

    let mut 界面 = 界面状态::新建();
    打开选择器(&mut 界面, 选择器视图::列表);
    静态帧(&egui上下文, &编辑器, &mut 界面);
    assert!(
        界面.选择器卡片几何.is_empty(),
        "列表视图不画卡片，不能留下可点的网格几何"
    );
    assert_eq!(
        界面.资源选择器.as_ref().expect("列表视图不选东西").视图,
        选择器视图::列表,
        "视图状态必须原样写回草稿"
    );

    打开选择器(&mut 界面, 选择器视图::网格);
    静态帧(&egui上下文, &编辑器, &mut 界面);
    assert_eq!(
        界面.选择器卡片几何.len(),
        2,
        "材质与贴图各一张卡片，且只登记本帧画出的这些"
    );
}
