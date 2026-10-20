//! 中文/日文文本分段：不得刷 `ICU4X data error: No segmentation model for complex script: Chinese/Japanese`。
//!
//! # 这条测试守的是什么
//!
//! 微风用 Bevy 0.20 渲染预览视口里的界面文字，Bevy 的文字排版走
//! `bevy_text → parley → icu_segmenter`。`icu_segmenter` 有两套构造器：
//!
//! * `LineSegmenter::new_dictionary` / `WordSegmenter::new_dictionary` ——
//!   带中日文词典，能对没有空格的中文/日文正确断行；
//! * `LineSegmenter::new_for_non_complex_scripts` —— **不**带任何复杂文字
//!   词典，遇到中文/日文时 `icu_segmenter` 会抛
//!   `DataError::custom("No segmentation model for complex script")`。
//!
//! 选哪一套由 parley 的 `complex-scripts` 特性决定，而这个特性在 Bevy 里只有
//! 打开 `complex_script_segmentation`（根 `Cargo.toml` 的 `bevy` 特性）才会被启用。
//! 特性没打开时，**每一段中文/日文每次重新排版**都会再打一遍警告
//! （不是初始化打一次），界面上只要有会变化的中文字（帧率、计分、改字号、
//! 窗口尺寸变化触发的重排），控制台就会持续刷屏；中文长句也失去按词断行。
//!
//! # 为什么是"另起一个进程再看它的 stderr"
//!
//! 警告不是走 `log` 门面发出来的：`icu_provider` 在**没有**打开自己的
//! `logging` 特性时，把 `log::warn!` 直接换成了 `std::eprintln!`
//! （见 `icu_provider::__private_api` 里的
//! `pub use std::eprintln as warn`）。所以：
//!
//! * 它没有时间戳、没有级别、没有 target，Bevy 的 `RUST_LOG` 过滤器管不到它；
//! * 装一个 `log` 记录器**抓不到**它，只能看真实的标准错误流。
//!
//! 测试进程内 `eprintln!` 会被 libtest 的捕获机制吞掉（只有失败或
//! `--nocapture` 时才显示），所以这里用"父进程 → 子进程"的方式：
//! 父进程用 `--nocapture` 再跑一遍同一个测试函数，把它真实的 stderr 收上来检查。
//!
//! # 为什么测试直接调 parley
//!
//! `bevy_text` 只把 `FontCx` / `LayoutCx` 暴露出来，真正的排版入口没有公开；
//! 而这个缺陷的判定点就在 parley 的断行调用上。这里用**同一个** parley
//! （同一份特性解析结果）复现那条调用路径：缺陷还在就会看到真实警告，
//! 依赖改对了就一条都不出现。
//!
//! dev-dependencies 里的 `parley` **故意不启用** `complex-scripts`，
//! 所以这份测试不会自己把缺陷掩盖掉：特性必须由 Bevy 那边的
//! `complex_script_segmentation` 打开才行。

use std::process::{Command, Stdio};

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, GenericFamily,
    LayoutContext, StyleProperty,
};

/// ICU4X 缺分段数据时输出的固定片段（`icu_provider::DataError` 的 Display 文本）。
const 警告特征: &str = "No segmentation model for complex script";

/// 子进程标记：设了它表示"这次运行只做排版，不再派生下一层进程"。
const 子进程标记: &str = "BREEZE_ICU4X_分段测试子进程";

/// 造一个只含内置中文字体的排版上下文。
///
/// 不依赖系统字体：测试机装没装中文字体不应该改变结论。
fn 造字体上下文() -> FontContext {
    let 路径 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("editor_ui")
        .join("assets")
        .join("fonts")
        .join("NotoSansSC-Subset.ttf");
    let 数据 = std::fs::read(&路径)
        .unwrap_or_else(|错误| panic!("读取内置中文字体 {} 失败：{错误}", 路径.display()));

    let mut 上下文 = FontContext::new();
    let 家族 = 上下文
        .collection
        .register_fonts(parley::fontique::Blob::new(std::sync::Arc::new(数据)), None);
    let 标识 = 家族
        .first()
        .map(|(标识, _)| *标识)
        .expect("内置字体必须至少注册出一个家族");
    上下文
        .collection
        .set_generic_families(GenericFamily::SansSerif, std::iter::once(标识));
    上下文
}

/// 排一次版，返回字形段数量（用来证明"真的排了"，而不是空跑）。
fn 排版一次(字体: &mut FontContext, 排版: &mut LayoutContext<()>, 文本: &str) -> usize {
    let mut 构建器 = 排版.ranged_builder(字体, 文本, 1.0, true);
    构建器.push_default(StyleProperty::FontSize(18.0));
    构建器.push_default(StyleProperty::FontFamily(FontFamily::Single(
        FontFamilyName::Generic(GenericFamily::SansSerif),
    )));

    let mut 布局: parley::Layout<()> = 构建器.build(文本);
    // 断行就是触发 ICU4X 分段的那一步：宽度有限才会真的去找词边界。
    布局.break_all_lines(Some(160.0));
    布局.align(Alignment::Start, AlignmentOptions::default());

    let 段数: usize = 布局.lines().map(|行| 行.items().count()).sum();
    // 防止优化把整段排版消掉。
    std::hint::black_box(布局.width());
    段数
}

/// 中文、日文、英文各反复排版 5 次，返回各自的字形段总数。
fn 反复排版() -> (usize, usize, usize) {
    let mut 字体 = 造字体上下文();
    let mut 排版 = LayoutContext::<()>::new();

    let 中文 = "微风·界面演示：这条文字来自场景文件，改了字号会立刻变。";
    let 日文 = "日本語のテキストです。きょうはいいてんきですね。";
    let 英文 = "Breeze text layout probe: English words are separated by spaces.";

    let (mut 中文段数, mut 日文段数, mut 英文段数) = (0, 0, 0);
    for _ in 0..5 {
        中文段数 += 排版一次(&mut 字体, &mut 排版, 中文);
        日文段数 += 排版一次(&mut 字体, &mut 排版, 日文);
        英文段数 += 排版一次(&mut 字体, &mut 排版, 英文);
    }
    (中文段数, 日文段数, 英文段数)
}

/// 子进程里跑的那一半：只做排版，结果靠断言表达。
fn 子进程运行() {
    let (中文, 日文, 英文) = 反复排版();
    // 排版确实发生了：没有字形段说明字体没解析出来，那样即使没有警告
    // 也说明不了问题（属于假通过）。
    assert!(
        中文 > 0 && 日文 > 0 && 英文 > 0,
        "排版没有产生字形段：中文 {中文}、日文 {日文}、英文 {英文}"
    );
    // 让这条结论在 --nocapture 下留在 stdout 里，便于人工复核。
    println!("排版字形段：中文 {中文}、日文 {日文}、英文 {英文}");
}

/// 中文、日文、英文反复排版：stderr 上不得出现任何 ICU4X 分段缺数据警告。
///
/// 反复排版对应真实场景：界面文字改了字号、改了文本、窗口尺寸变了，
/// 都会重新走一遍断行。缺陷存在时警告条数会随排版次数线性增长。
#[test]
fn 中日英文反复排版都不产生_icu4x_分段警告() {
    if std::env::var_os(子进程标记).is_some() {
        子进程运行();
        return;
    }

    let 可执行文件 = std::env::current_exe().expect("拿不到测试二进制路径");
    let 输出 = Command::new(&可执行文件)
        .arg("中日英文反复排版都不产生_icu4x_分段警告")
        .arg("--exact")
        // 不加 --nocapture 的话，子进程里的 eprintln! 会被 libtest 的捕获吞掉，
        // 父进程就看不到真实的标准错误流了。
        .arg("--nocapture")
        .env(子进程标记, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|错误| panic!("启动子进程 {} 失败：{错误}", 可执行文件.display()));

    let 标准输出 = String::from_utf8_lossy(&输出.stdout);
    let 标准错误 = String::from_utf8_lossy(&输出.stderr);
    assert!(
        输出.status.success(),
        "排版子进程失败（{}）：\n--- stdout ---\n{标准输出}\n--- stderr ---\n{标准错误}",
        输出.status
    );

    let 警告: Vec<&str> = 标准错误
        .lines()
        .filter(|行| 行.contains(警告特征))
        .collect();
    assert!(
        警告.is_empty(),
        "中文/日文排版仍然打出 {} 条 ICU4X 分段缺数据警告（说明 parley 的 \
         `complex-scripts` 特性没打开，即 Bevy 的 `complex_script_segmentation` \
         缺失）。原始 stderr：\n{}",
        警告.len(),
        标准错误
    );
}
