//! 中文名：组件字段覆盖夹具
//! 校验 `tools/fixtures/component_override_project` 的手写 RON 真的是一份
//! "带组件字段覆盖的预制件实例"，并且这份覆盖能被恢复流程清干净。
//!
//! # 为什么夹具需要一条 cargo 测试
//!
//! 这份夹具的唯一消费者是 `tools/prefab_acceptance.ps1` 的真实 GUI 用例，
//! 而那条用例只在手工/发版时跑。夹具写错一个字段名（例如把覆盖键的组件
//! 写成模板名），GUI 那边只会表现为"按钮没出现、点击取不到坐标"，
//! 排查方向会被误导到界面层。把同一份文件先在数据层读一遍，夹具自身的
//! 错误就在 `cargo test` 里当场变成失败，而不是伪装成 UI 缺陷。
//!
//! # 为什么顺带跑一次恢复
//!
//! GUI 用例断言的是"点恢复按钮之后覆盖条数归零、实体值回到模板默认值"。
//! 这里用同一对参数（组件 + 属性名）在内存里走一遍 [`恢复属性默认值`]，
//! 证明这份手写覆盖**确实可恢复**：若覆盖键与模板字段对不上，
//! 删除数会是 0，GUI 用例也就失去意义。

use std::path::PathBuf;

use scene_doc::document::场景文档;
use scene_doc::prefab::{实体来源, 覆盖状态, 预制件文档};
use scene_doc::prefab_nested::{定位属性覆盖, 恢复属性默认值};
use scene_doc::property::属性值;
use scene_doc::storage::{解码场景, 解码预制件};

const 夹具目录: &str = "../../tools/fixtures/component_override_project";

fn 夹具路径(相对: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(夹具目录)
        .join(相对)
}

fn 读文本(相对: &str) -> (String, PathBuf) {
    let 路径 = 夹具路径(相对);
    let 文本 = std::fs::read_to_string(&路径)
        .unwrap_or_else(|错误| panic!("读不到夹具 {}：{错误}", 路径.display()));
    (文本, 路径)
}

fn 读场景() -> 场景文档 {
    let (文本, 路径) = 读文本("scenes/component_demo.scene.ron");
    解码场景(&文本, &路径).expect("场景夹具应当能解码")
}

fn 读模板() -> 预制件文档 {
    let (文本, 路径) = 读文本("prefabs/哨兵.prefab.ron");
    解码预制件(&文本, &路径).expect("模板夹具应当能解码")
}

/// 模板根上 `水平翻转` 的出厂值。恢复流程要写回的正是这个值。
fn 模板默认翻转(模板: &预制件文档) -> bool {
    let 根 = 模板
        .场景
        .取(模板.头.根)
        .expect("模板头里的根标识必须能在模板场景里找到");
    match 根
        .取组件("Sprite")
        .and_then(|组件| 组件.属性.get("水平翻转"))
    {
        Some(属性值::布尔(值)) => *值,
        other => panic!("模板根的 Sprite.水平翻转 必须是布尔，实际 {other:?}"),
    }
}

fn 实体翻转(文档: &场景文档, 标识: scene_doc::document::实体标识) -> bool {
    match 文档
        .取(标识)
        .and_then(|实体| 实体.取组件("Sprite"))
        .and_then(|组件| 组件.属性.get("水平翻转"))
    {
        Some(属性值::布尔(值)) => *值,
        other => panic!("实体 Sprite.水平翻转 必须是布尔，实际 {other:?}"),
    }
}

#[test]
fn 组件覆盖夹具是一条可恢复的根字段覆盖() {
    let 文档 = 读场景();
    let 模板 = 读模板();

    assert_eq!(文档.元数据.场景名, "component_demo");
    // 模板默认 false、实例覆盖成 true：两者不同，恢复才有可观测的效果。
    assert!(!模板默认翻转(&模板), "模板默认必须是 false");
    assert_eq!(模板.头.版本, 1, "实例记录的版本必须与模板版本一致");

    let 哨兵 = 文档
        .实体表
        .values()
        .find(|实体| 实体.名称 == "哨兵")
        .expect("场景里必须有一个名为「哨兵」的实体")
        .标识;
    match 文档.取(哨兵).map(|实体| &实体.来源) {
        Some(实体来源::预制件实例根 { 实例根 }) => assert_eq!(*实例根, 哨兵),
        other => panic!("哨兵必须是预制件实例根，实际 {other:?}"),
    }
    assert!(实体翻转(&文档, 哨兵), "实体上的值应当是覆盖后的 true");

    let 记录 = 文档
        .预制件实例表
        .get(&哨兵)
        .expect("实例根必须有对应的预制件实例记录");
    assert_eq!(记录.模板.标识, 模板.头.标识);
    assert_eq!(记录.覆盖.条目.len(), 1, "夹具只应带一条字段覆盖");
    assert!(!记录.待同步(), "实例不应处于待同步状态");

    let 落点 = 定位属性覆盖(&文档, 哨兵, "Sprite", "水平翻转")
        .expect("实例根的 Sprite.水平翻转 应当能定位到覆盖");
    assert_eq!(落点.记录键, 哨兵);
    assert_eq!(落点.状态, 覆盖状态::直接覆盖);
    assert!(
        落点.键.节点.是根(),
        "覆盖面应当是实例根（场景 RON 里没有 `节点:` 段）"
    );

    // 恢复：删掉这条覆盖并把模板默认值写回实体。
    let mut 文档 = 文档;
    let 报告 =
        恢复属性默认值(&mut 文档, 哨兵, "Sprite", "水平翻转", &模板).expect("恢复应当返回报告");
    assert_eq!(报告.删除覆盖, 1, "应当恰好删掉夹具里那一条覆盖");
    assert!(报告.写回实体, "模板有默认值时必须写回实体");

    assert!(文档.预制件实例表[&哨兵].覆盖.是空(), "恢复后覆盖表必须为空");
    assert!(
        !实体翻转(&文档, 哨兵),
        "恢复后实体值必须回到模板默认的 false"
    );
    assert_eq!(
        定位属性覆盖(&文档, 哨兵, "Sprite", "水平翻转").map(|落点| 落点.状态),
        Some(覆盖状态::继承),
        "恢复后这一项应当是「继承」而不是「直接覆盖」"
    );
}
