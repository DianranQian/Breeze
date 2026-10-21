//! 打开旧场景时补全资源稳定标识（R11）。
//!
//! ```text
//! 旧场景（资源引用没有 UID） → 打开 → 内存文档补上 UID、标记未保存
//! → 保存 → 重新打开 → 不再标脏（已经补齐过）
//! ```

use editor_core::project::项目描述;
use editor_core::state::编辑器状态;
use scene_doc::document::场景文档;
use scene_doc::path::项目相对路径;
use scene_doc::property::属性值;

fn 路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("路径")
}

/// 建项目 + 一个"旧格式"场景：资源表里有引用，但没有 UID。
fn 建旧场景() -> (tempfile::TempDir, 编辑器状态, 项目相对路径) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    std::fs::write(根.join("assets").join("房屋.png"), "x").expect("写资源");

    let mut 文档 = 场景文档::新建("旧场景");
    let 实体 = 文档.生成实体("房屋", None);
    let mut 属性 = std::collections::BTreeMap::new();
    属性.insert(
        "texture".to_string(),
        属性值::资源(scene_doc::property::资源引用值::按路径(路径(
            "assets/房屋.png",
        ))),
    );
    文档
        .取可变(实体)
        .expect("实体")
        .加组件(scene_doc::document::组件数据::新建("Sprite", 属性));
    文档.同步资源表();
    let 场景路径 = 路径("scenes/旧.scene.ron");
    项目.保存场景(&场景路径, &文档).expect("保存场景");

    let mut 状态 = 编辑器状态::新建();
    状态.项目 = Some(项目);
    (目录, 状态, 场景路径)
}

#[test]
fn 打开旧场景会补全标识并标记未保存() {
    let (_目录, mut 状态, 场景路径) = 建旧场景();
    状态.打开场景(&场景路径).expect("打开场景");

    let 引用 = 状态
        .会话
        .文档
        .资源表
        .get(&路径("assets/房屋.png"))
        .expect("资源表条目");
    assert!(引用.唯一标识.is_some(), "打开后内存文档应当有 UID");
    assert!(状态.有未保存修改(), "补 UID 是真实变化，必须标脏");
    assert!(
        状态
            .日志列表()
            .iter()
            .any(|条目| 条目.渲染(editor_core::i18n::语言::中文).contains("补全稳定标识")),
        "必须告诉用户发生了什么"
    );
}

#[test]
fn 保存后重新打开不再标脏() {
    let (_目录, mut 状态, 场景路径) = 建旧场景();
    状态.打开场景(&场景路径).expect("打开");
    assert!(状态.有未保存修改());

    状态.保存场景().expect("保存");
    assert!(!状态.有未保存修改(), "保存后应当干净");

    // 重新打开：UID 已经在文件里，不应该再改动文档。
    状态.打开场景(&场景路径).expect("再次打开");
    assert!(
        !状态.有未保存修改(),
        "已经带 UID 的场景再打开不应被标脏"
    );
}

#[test]
fn 打开从不引用资源的场景不会被误标脏() {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
    let mut 文档 = 场景文档::新建("空场景");
    文档.生成实体("只有节点", None);
    let 场景路径 = 路径("scenes/空.scene.ron");
    项目.保存场景(&场景路径, &文档).expect("保存");

    let mut 状态 = 编辑器状态::新建();
    状态.项目 = Some(项目);
    状态.打开场景(&场景路径).expect("打开");
    assert!(!状态.有未保存修改(), "没有资源就没有 UID 要补，不该标脏");
}
