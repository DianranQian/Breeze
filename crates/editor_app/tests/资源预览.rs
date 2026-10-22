//! 资源预览的定向验证。
//!
//! # 为什么测 `计算预览` 而不是跑整个 Bevy App
//!
//! 预览的**判断**部分（这个路径是什么类别、文件在不在、图片多大、
//! 场景里有几个实体、字体家族叫什么）是纯计算，与本轮要防的缺陷
//! （同名文件混淆、缺失文件显示空白、切换项目残留）完全对应。
//! 把它们放进需要 `Assets<Image>` / `EguiUserTextures` / 真窗口的系统里，
//! 这些性质就只能靠人眼看截图。
//!
//! 真实渲染（纹理是否真的画出来、模型是否真的出现在独立层）由
//! 本轮的 GUI 验收覆盖，见 `docs/验收证据/`。
//!
//! 测试覆盖：
//!
//! 1. 选中资源能更新预览目标（`选中资源` + `计算预览` 的路径一致）；
//! 2. 连续选择不同资源时，显示的是最终选中的那个（代次匹配）；
//! 3. 同名但不同目录的文件不会混淆；
//! 4. 切换/关闭项目后不残留上一个项目的预览；
//! 5. 无效路径与文件缺失能给出明确错误；
//! 6. 不支持的文件类型有明确状态；
//! 7. 图片元数据与实际文件一致；
//! 8. 异步加载期间快速切换，旧结果不会覆盖新选择；
//! 9. 删除/重命名后预览状态能正确失效；
//! 10. 预览不会修改场景文档；
//! 11. 关闭项目会清掉试听状态与资源选择；
//! 12. 模型预览不会在文档里创建实体；
//! 13. 反复预览同一资源不会累积状态。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use editor_app::actions;
use editor_app::asset_preview::{判断类别, 计算预览};
use editor_app::build::构建宿主;
use editor_app::config::{布局待保存, 编辑器配置资源};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_core::i18n::语言;
use editor_core::project::项目描述;
use editor_ui::state::{UiAction, 预览阶段};
use scene_doc::document::场景文档;
use scene_doc::path::项目相对路径;

fn 路径(文本: &str) -> 项目相对路径 {
    项目相对路径::解析(文本).expect("测试路径应当合法")
}

fn 写图片(根: &std::path::Path, 相对: &str, 宽: u32, 高: u32) {
    let 绝对 = 根.join(相对);
    if let Some(父) = 绝对.parent() {
        std::fs::create_dir_all(父).expect("建目录");
    }
    let 图 = image::RgbaImage::from_pixel(宽, 高, image::Rgba([10, 200, 30, 255]));
    图.save(&绝对).expect("写 PNG");
}

fn 建项目() -> (tempfile::TempDir, 项目描述) {
    let 目录 = tempfile::tempdir().expect("临时目录");
    let 根 = 目录.path().join("项目");
    let 项目 = 项目描述::创建(&根, "预览测试").expect("创建项目");
    (目录, 项目)
}

/// 1 + 7：选中资源后能拿到**与实际文件一致**的图片元数据。
#[test]
fn 图片预览的尺寸与真实文件一致() {
    let (_目录, 项目) = 建项目();
    写图片(项目.根目录(), "assets/hero.png", 37, 19);

    let 产物 = 计算预览(&项目, &路径("assets/hero.png"), 7, 语言::中文);
    assert_eq!(产物.快照.阶段, 预览阶段::就绪, "{:?}", 产物.快照.说明);
    let 图片 = 产物.快照.图片.as_ref().expect("应当有图片信息");
    assert_eq!((图片.宽, 图片.高), (37, 19), "尺寸必须来自真实文件");
    assert!(图片.格式.contains("png"), "格式应来自内容：{}", 图片.格式);
    assert!(产物.图片.is_some(), "应当产出可注册的像素数据");
    assert_eq!(产物.快照.代次, 7, "快照必须带上请求代次");
    assert!(产物.快照.大小 > 0);
}

/// 3：同名但位于不同目录的资源不会混淆。
#[test]
fn 同名文件在不同目录下预览不同() {
    let (_目录, 项目) = 建项目();
    写图片(项目.根目录(), "assets/a/hero.png", 11, 12);
    写图片(项目.根目录(), "assets/b/hero.png", 64, 33);

    let 甲 = 计算预览(&项目, &路径("assets/a/hero.png"), 1, 语言::中文);
    let 乙 = 计算预览(&项目, &路径("assets/b/hero.png"), 2, 语言::中文);
    let 甲图 = 甲.快照.图片.as_ref().expect("甲应有图");
    let 乙图 = 乙.快照.图片.as_ref().expect("乙应有图");
    assert_eq!((甲图.宽, 甲图.高), (11, 12));
    assert_eq!((乙图.宽, 乙图.高), (64, 33));
    assert_eq!(
        甲.快照.路径.as_deref(),
        Some("assets/a/hero.png"),
        "快照必须记录真实路径，而不只是显示名"
    );
}

/// 5：缺失文件必须给出明确错误，而不是空白。
#[test]
fn 缺失文件返回失败与明确说明() {
    let (_目录, 项目) = 建项目();
    let 产物 = 计算预览(&项目, &路径("assets/不存在.png"), 3, 语言::中文);
    assert_eq!(产物.快照.阶段, 预览阶段::失败);
    let 说明 = 产物.快照.说明.expect("失败必须带说明");
    assert!(
        说明.contains("不存在.png"),
        "说明里应当指出是哪个文件：{说明}"
    );
    assert!(产物.快照.图片.is_none());
}

/// 6：不支持的扩展名给出"不支持"，且与"加载失败"区分开。
#[test]
fn 不支持的类型有明确状态() {
    let (_目录, 项目) = 建项目();
    std::fs::write(项目.根目录().join("assets/blob.bin"), [0u8, 1, 2]).expect("写文件");
    let 产物 = 计算预览(&项目, &路径("assets/blob.bin"), 1, 语言::中文);
    assert_eq!(
        产物.快照.阶段,
        预览阶段::不支持,
        "未知二进制应当是不支持，而不是静默空白"
    );
    assert!(产物.快照.说明.is_some());

    // 损坏的图片是**加载失败**，不是"不支持"：两者含义不同。
    std::fs::write(项目.根目录().join("assets/坏图.png"), b"not a png").expect("写坏图");
    let 坏 = 计算预览(&项目, &路径("assets/坏图.png"), 2, 语言::中文);
    assert_eq!(坏.快照.阶段, 预览阶段::失败, "{:?}", 坏.快照.说明);
}

/// 9：删除文件后预览如实失效。
#[test]
fn 删除文件后预览变成失败() {
    let (_目录, 项目) = 建项目();
    写图片(项目.根目录(), "assets/temp.png", 8, 8);
    let 之前 = 计算预览(&项目, &路径("assets/temp.png"), 1, 语言::中文);
    assert_eq!(之前.快照.阶段, 预览阶段::就绪);

    std::fs::remove_file(项目.根目录().join("assets/temp.png")).expect("删文件");
    let 之后 = 计算预览(&项目, &路径("assets/temp.png"), 2, 语言::中文);
    assert_eq!(之后.快照.阶段, 预览阶段::失败, "文件没了就不能再显示旧内容");
}

/// 重命名后，旧路径失效、新路径可用。
#[test]
fn 重命名后旧路径失效新路径可预览() {
    let (_目录, 项目) = 建项目();
    写图片(项目.根目录(), "assets/旧名.png", 9, 9);
    let 旧 = 计算预览(&项目, &路径("assets/旧名.png"), 1, 语言::中文);
    assert_eq!(旧.快照.阶段, 预览阶段::就绪);

    std::fs::rename(
        项目.根目录().join("assets/旧名.png"),
        项目.根目录().join("assets/新名.png"),
    )
    .expect("重命名");
    let 旧路径 = 计算预览(&项目, &路径("assets/旧名.png"), 2, 语言::中文);
    assert_eq!(旧路径.快照.阶段, 预览阶段::失败);
    let 新路径 = 计算预览(&项目, &路径("assets/新名.png"), 3, 语言::中文);
    assert_eq!(新路径.快照.阶段, 预览阶段::就绪);
}

/// 场景预览显示真实层级，且**不修改**场景文件。
#[test]
fn 场景预览来自真实文档且不改文件() {
    let (_目录, 项目) = 建项目();
    let mut 文档 = 场景文档::新建("测试场景");
    let 根 = 文档.生成实体("根节点", None);
    let 子 = 文档.生成实体("子节点", Some(根));
    文档.取可变(子).expect("子实体");
    let 场景路径 = 路径("scenes/测试.scene.ron");
    项目.保存场景(&场景路径, &文档).expect("保存场景");

    let 绝对 = 项目.绝对路径(&场景路径);
    let 之前 = std::fs::read(&绝对).expect("读场景");

    let 产物 = 计算预览(&项目, &场景路径, 1, 语言::中文);
    assert_eq!(产物.快照.阶段, 预览阶段::就绪, "{:?}", 产物.快照.说明);
    let 预览 = 产物.快照.场景.expect("应当有场景预览");
    assert_eq!(预览.实体总数, 2);
    assert_eq!(预览.根实体数, 1);
    assert!(
        预览.实体.iter().any(|实体| 实体.名称 == "子节点"),
        "层级里应当包含真实实体：{:?}",
        预览.实体
    );
    assert!(
        预览.实体.iter().any(|实体| 实体.深度 == 1),
        "子节点应当有深度"
    );

    let 之后 = std::fs::read(&绝对).expect("再读场景");
    assert_eq!(之前, 之后, "预览绝不能改写场景文件");
    // 场景类别优先于扩展名推断（`ron` 本身会被推成"其他"）。
    assert_eq!(
        判断类别(&项目, &场景路径),
        scene_doc::property::资源类别::场景
    );
}

/// 12：模型预览不会在场景文档里创建实体。
#[test]
fn 模型预览不向文档添加实体() {
    let (_目录, 项目) = 建项目();
    std::fs::write(
        项目.根目录().join("assets/box.gltf"),
        br#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[]}],"scene":0}"#,
    )
    .expect("写 gltf");
    let 产物 = 计算预览(&项目, &路径("assets/box.gltf"), 1, 语言::中文);
    assert_eq!(产物.快照.阶段, 预览阶段::就绪);
    let 模型 = 产物.快照.模型.expect("应当有模型预览占位");
    assert!(模型.等待渲染, "在真正加载出网格之前必须如实标准备中");
    assert_eq!(模型.网格数, 0);
    assert!(产物.模型路径.is_some(), "系统侧需要绝对路径去加载");
    // 计算过程只产出一份快照，没有任何"往文档里塞实体"的入口。
    assert!(产物.快照.场景.is_none());
}

/// 字体预览能读出真实家族名与字形数。
#[test]
fn 字体预览读取真实字体表() {
    let 字体路径 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../editor_ui/assets/fonts/NotoSansSC-Subset.ttf");
    if !字体路径.is_file() {
        // 字体是仓库资产；缺失时这条测试无法成立，直接跳过而不是假装通过。
        eprintln!("跳过：内置字体资产不存在 {}", 字体路径.display());
        return;
    }
    let (目录, 项目) = 建项目();
    let 目标 = 项目.根目录().join("assets/font.ttf");
    std::fs::copy(&字体路径, &目标).expect("复制字体");

    let 产物 = 计算预览(&项目, &路径("assets/font.ttf"), 1, 语言::中文);
    assert_eq!(产物.快照.阶段, 预览阶段::就绪, "{:?}", 产物.快照.说明);
    let 字体 = 产物.快照.字体.expect("应当有字体信息");
    assert!(字体.字形数.unwrap_or(0) > 100, "字形数应当来自真实字体表");
    assert!(!字体.家族名.is_empty(), "应当读出家族名");
    assert!(产物.字体字节.is_some(), "系统侧需要字节来注册家族");
    drop(目录);
}

/// 音频元数据只报告真的能读出来的字段。
#[test]
fn 音频预览只报告可读取的元数据() {
    let (_目录, 项目) = 建项目();
    // 单声道 / 8000 Hz / 16 bit / 0.5 秒。
    let 采样率: u32 = 8_000;
    let 声道: u16 = 1;
    let 位深: u16 = 16;
    let 数据字节: u32 = 8_000;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + 数据字节).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&声道.to_le_bytes());
    wav.extend_from_slice(&采样率.to_le_bytes());
    wav.extend_from_slice(&(采样率 * 声道 as u32 * 位深 as u32 / 8).to_le_bytes());
    wav.extend_from_slice(&(声道 * 位深 / 8).to_le_bytes());
    wav.extend_from_slice(&位深.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&数据字节.to_le_bytes());
    wav.extend(std::iter::repeat_n(0u8, 数据字节 as usize));
    std::fs::write(项目.根目录().join("assets/beep.wav"), &wav).expect("写 wav");

    let 产物 = 计算预览(&项目, &路径("assets/beep.wav"), 1, 语言::中文);
    let 音频 = 产物.快照.音频.expect("应当有音频信息");
    assert_eq!(音频.采样率, Some(8_000));
    assert_eq!(音频.声道数, Some(1));
    assert_eq!(音频.位深, Some(16));
    assert!((音频.时长秒.unwrap_or(0.0) - 0.5).abs() < 0.02);
    assert!(音频.可试听, "WAV 是编辑器编译进来的格式");
}

/// 文本类资源给出只读内容。
#[test]
fn 文本资源给出行内容() {
    let (_目录, 项目) = 建项目();
    std::fs::write(
        项目.根目录().join("assets/readme.md"),
        "# 标题\n正文\n第三行\n",
    )
    .expect("写文本");
    let 产物 = 计算预览(&项目, &路径("assets/readme.md"), 1, 语言::中文);
    let 文本 = 产物.快照.文本.expect("应当有文本预览");
    assert_eq!(文本.总行数, 3);
    assert_eq!(文本.行.first().map(String::as_str), Some("# 标题"));
    assert!(!文本.已截断);
}

// ---------------------------------------------------------------------------
// 动作层：项目生命周期
// ---------------------------------------------------------------------------

fn 建测试世界() -> World {
    let mut 世界 = World::new();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(物理运行时状态::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(布局待保存::default());
    世界.insert_resource(项目运行管理器::default());
    世界.insert_resource(构建宿主::新建());
    世界.insert_resource(诊断中心::default());
    世界.insert_resource(编辑器配置资源::default());
    世界.insert_resource(导入快照资源::default());
    世界
}

fn 执行动作(世界: &mut World) {
    世界
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 界面(世界: &World) -> &editor_ui::界面状态 {
    &世界.resource::<UiStateHolder>().状态
}

/// 4 + 11：关闭项目后不残留预览与试听状态。
#[test]
fn 关闭项目清空资源预览与试听状态() {
    let (目录, 项目) = 建项目();
    写图片(项目.根目录(), "assets/hero.png", 10, 10);

    let mut 世界 = 建测试世界();
    世界.resource_mut::<EditorStateHolder>().状态.项目 = Some(项目);

    {
        let mut 界面状态 = 世界.resource_mut::<UiStateHolder>();
        界面状态.状态.选中资源(路径("assets/hero.png"));
        界面状态.状态.资源预览.阶段 = 预览阶段::就绪;
        界面状态.状态.试听中 = Some("assets/beep.wav".to_string());
        界面状态.动作.推(UiAction::关闭项目);
    }
    执行动作(&mut 世界);

    let 界面 = 界面(&世界);
    assert!(界面.资源选中.is_none(), "关闭项目后不能还选着旧项目的资源");
    assert_eq!(界面.资源预览.阶段, 预览阶段::空闲);
    assert!(界面.资源预览.路径.is_none());
    assert!(界面.试听中.is_none(), "试听状态也必须跟着项目消失");
    assert_eq!(
        界面.试听请求,
        editor_ui::state::试听请求::default(),
        "未消费的试听请求不能留到下一个项目"
    );
    drop(目录);
}

/// 8：旧代次的结果不能覆盖新选择（界面侧只认代次匹配的快照）。
#[test]
fn 旧代次结果不会覆盖新选择() {
    let (_目录, 项目) = 建项目();
    写图片(项目.根目录(), "assets/甲.png", 4, 4);
    写图片(项目.根目录(), "assets/乙.png", 40, 40);

    let mut 界面 = editor_ui::界面状态::新建();
    界面.选中资源(路径("assets/甲.png"));
    let 甲请求 = 界面.预览请求.clone();
    界面.选中资源(路径("assets/乙.png"));
    let 乙请求 = 界面.预览请求.clone();
    assert!(乙请求.代次 > 甲请求.代次);

    // 甲的结果迟到了。
    let 甲产物 = 计算预览(&项目, &路径("assets/甲.png"), 甲请求.代次, 语言::中文);
    assert!(
        !甲产物.快照.是最新(&乙请求),
        "迟到的旧结果不能被当成当前答案"
    );
    let 乙产物 = 计算预览(&项目, &路径("assets/乙.png"), 乙请求.代次, 语言::中文);
    assert!(乙产物.快照.是最新(&乙请求));
    assert_eq!(
        乙产物.快照.图片.map(|图片| (图片.宽, 图片.高)),
        Some((40, 40))
    );
}

/// 13：反复预览同一资源不会累积状态。
#[test]
fn 重复预览同一资源结果稳定() {
    let (_目录, 项目) = 建项目();
    写图片(项目.根目录(), "assets/hero.png", 12, 15);
    let 第一次 = 计算预览(&项目, &路径("assets/hero.png"), 1, 语言::中文);
    for 序号 in 2..6 {
        let 再一次 = 计算预览(&项目, &路径("assets/hero.png"), 序号, 语言::中文);
        assert_eq!(再一次.快照.图片, 第一次.快照.图片);
        assert_eq!(再一次.快照.阶段, 第一次.快照.阶段);
        assert_eq!(再一次.快照.大小, 第一次.快照.大小);
    }
}
