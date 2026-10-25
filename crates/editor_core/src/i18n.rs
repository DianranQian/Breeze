//! 界面多语言（简体中文 / English）。
//!
//! # 为什么这个模块在 `editor_core`
//!
//! 依赖方向是 `editor_app → editor_ui → editor_core → scene_doc`，单向。
//! 而需要翻译的文字不只在 `editor_ui`：
//!
//! * [`crate::command::编辑器命令::显示名`] 的名字会显示在「历史」面板里，
//!   它属于 `editor_core`；
//! * 将来 `scene_doc` 的错误文案要本地化时，同样需要更底层的入口。
//!
//! 把翻译入口放在 `editor_core` 之后，上面的每一层都能直接用，
//! 不需要为了翻译而反向依赖界面层。`editor_ui` 只是把它再导出一次，
//! 方便界面代码少写一层路径。
//!
//! # 翻译键为什么是符号键，而不是中文原文
//!
//! 早期版本直接用中文字面量当键（`取("保存场景")`）。它有个隐患：
//! **改动中文措辞就等于换了键**，词表、调用点、测试要同时改，
//! 漏改一处就会静默查不到。
//!
//! 现在改用稳定的 ASCII 符号键：
//!
//! ```ignore
//! ui.label(语言.取("save_scene"));       // → "保存场景" / "Save Scene"
//! ui.label(语言.填充("viewport_2d_entity_count", &[&数量]));
//! ```
//!
//! 用例：
//!
//! * 改中文措辞只动词表一行，调用点不受影响；
//! * 同一个键在中英两种语言下都必须有值 —— 这一点由词表的结构保证
//!   （一条词条同时带中文与英文，不存在"只翻了一半"的状态）；
//! * 键是 ASCII，日志、测试失败信息里都能直接读，不会因为终端编码变乱码。
//!
//! # 缺失翻译的行为
//!
//! `取` 查不到键时**返回键本身**。界面上会直接出现 `menu_file_save_scene`
//! 这样的字符串，一眼就能看出漏了翻译 —— 而不是静默退回中文、
//! 让人误以为已经翻好。同时有一条测试扫描全部调用点，
//! 保证不存在"调用了词表里没有的键"这种情况。

use std::fmt::Display;

/// 界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum 语言 {
    /// 简体中文。
    #[default]
    中文,
    /// English。
    英文,
}

/// 一条翻译词条。
///
/// 一条词条同时携带两种语言，因此**不存在"某个键只翻了一半"的状态** ——
/// 这是结构上的保证，不需要靠人工核对。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct 词条 {
    /// 稳定的 ASCII 符号键。
    pub 键: &'static str,
    /// 简体中文文案。
    pub 中文: &'static str,
    /// 英文文案。
    pub 英文: &'static str,
}

impl 语言 {
    /// 全部可选语言，用于在设置菜单里列出。
    pub const 全部: [语言; 2] = [语言::中文, 语言::英文];

    /// 稳定代号，用于写进配置文件。**不要**改这些字符串，
    /// 否则用户已保存的语言偏好会失效。
    pub fn 代号(self) -> &'static str {
        match self {
            语言::中文 => "zh-CN",
            语言::英文 => "en",
        }
    }

    /// 由代号解析。无法识别时回退到中文。
    ///
    /// 回退而不是报错，是因为语言偏好属于"可有可无的体验设置"：
    /// 一个拼错的代号不应该让编辑器无法启动。
    /// 配置里出现未知语言时，用户在设置菜单里重新选一次即可。
    pub fn 从代号(代号: &str) -> Self {
        match 代号 {
            "en" => 语言::英文,
            _ => 语言::中文,
        }
    }

    /// 在设置菜单里显示的名字。
    ///
    /// 刻意用各语言自己的写法（"简体中文" / "English"）而不是翻译成
    /// 当前语言：用户只看得懂一种语言时，也要能找到自己那一项。
    pub fn 自称(self) -> &'static str {
        match self {
            语言::中文 => "简体中文",
            语言::英文 => "English",
        }
    }

    /// 由语言的自称反查语言。
    pub fn 从自称(自称: &str) -> Option<Self> {
        Self::全部.into_iter().find(|语言| 语言.自称() == 自称)
    }

    /// 该语言在「界面语言」菜单里的词条键。
    ///
    /// 词条内容就是该语言的自称，两种语言下都一样 ——
    /// 于是用户无论当前是哪种语言，都能认出自己那一项。
    pub fn 词条键(self) -> &'static str {
        match self {
            语言::中文 => "language_self_zh_cn",
            语言::英文 => "language_self_en",
        }
    }

    /// 由菜单词条键反查语言。
    ///
    /// 菜单里存的是**键**而不是显示文本，因此这里按键判断，
    /// 与当前语言无关 —— 换成英文界面后依然能正确派发。
    pub fn 从词条键(键: &str) -> Option<Self> {
        Self::全部.into_iter().find(|语言| 语言.词条键() == 键)
    }

    /// 在两种语言间切换。
    pub fn 下一个(self) -> Self {
        match self {
            语言::中文 => 语言::英文,
            语言::英文 => 语言::中文,
        }
    }

    /// 取一条翻译。
    ///
    /// 查不到键时返回键本身（界面上会明显看出是键名），
    /// 见模块头部"缺失翻译的行为"。
    pub fn 取(self, 键: &'static str) -> &'static str {
        match 查(键) {
            Some(词条) => match self {
                语言::中文 => 词条.中文,
                语言::英文 => 词条.英文,
            },
            None => 键,
        }
    }

    /// 取一条翻译并依次填充其中的 `{}` 占位符。
    ///
    /// # 为什么需要这个函数
    ///
    /// `format!` 要求格式串是**字面量** —— 它在编译期解析那个字符串。
    /// 而译文是运行时查表得到的 `&str`，因此
    /// `format!(语言.取("viewport_2d_entity_count"), 数)` **根本编译不过**。
    ///
    /// 所以这里在运行时做替换。占位符统一只用 `{}`：
    /// 需要控制小数位数时由调用方先把值格式化好再传进来，例如
    /// `语言.填充("viewport_zoom", &[&format!("{:.0}", 缩放 * 100.0)])`。
    /// 这样本函数只需处理"顺序替换"，不必自己实现一个格式化器。
    ///
    /// 占位符多于参数时，多出来的原样保留；参数多于占位符时多余的被忽略 ——
    /// 两种情况都不会 panic。译文写错顶多显示难看，不会把编辑器搞崩。
    pub fn 填充(self, 键: &'static str, 参数: &[&dyn Display]) -> String {
        let 模板 = self.取(键);
        let mut 结果 = String::with_capacity(模板.len() + 32);
        let mut 参数迭代 = 参数.iter();
        let mut 剩余 = 模板;

        while let Some(左) = 剩余.find("{}") {
            结果.push_str(&剩余[..左]);
            match 参数迭代.next() {
                Some(值) => 结果.push_str(&值.to_string()),
                None => 结果.push_str("{}"),
            }
            剩余 = &剩余[左 + 2..];
        }
        结果.push_str(剩余);
        结果
    }

    /// 词表里是否存在该键。
    ///
    /// 供测试与"调用点扫描"使用：它能确定地回答
    /// "界面上用到的每一个键都在词表里"。
    pub fn 有键(self, 键: &str) -> bool {
        查(键).is_some()
    }
}

/// 在词表里二分查找。
///
/// 词表按 `键` 的 UTF-8 字节序排好序，由生成脚本保证，
/// `词表按键有序且无重复` 这条测试守着它 —— 顺序错了二分查找会**静默**
/// 查不到，属于必须用测试钉住的一类问题。
fn 查(键: &str) -> Option<&'static 词条> {
    词表
        .binary_search_by(|词条| 词条.键.cmp(键))
        .ok()
        .map(|下标| &词表[下标])
}

/// 词表单独成文件：它有几百条、且由脚本生成，
/// 与这里的逻辑放在一起会把逻辑本身淹掉。
///
/// 文件名用 `table.rs` 而不是中文名：**Rust 的模块文件名必须是 ASCII**，
/// 用中文会直接编译不过。这也符合本项目"文件名用英文、标识符用中文"的约定。
mod table;
pub use table::词表;

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 中文与英文都必须有值 —— 空串会在界面上留下空白。
    #[test]
    fn 两种语言都有非空文案() {
        for 词条 in 词表 {
            assert!(!词条.键.is_empty(), "键不能为空");
            assert!(!词条.中文.trim().is_empty(), "`{}` 缺中文文案", 词条.键);
            assert!(
                !词条.英文.trim().is_empty(),
                "`{}` 缺英文文案 —— 英文模式下会显示成空白",
                词条.键
            );
        }
    }

    /// 词表必须按字符串字节序严格升序，且无重复键。
    ///
    /// 二分查找的前提。顺序错了不会报错，只会静默查不到。
    #[test]
    fn 词表按键有序且无重复() {
        for 窗口 in 词表.windows(2) {
            assert!(
                窗口[0].键 < 窗口[1].键,
                "词表必须按字节序严格升序，但 `{}` 出现在 `{}` 之前或与之相等",
                窗口[0].键,
                窗口[1].键
            );
        }
    }

    /// 键必须是稳定的 ASCII 标识符：小写字母、数字、下划线。
    ///
    /// 这条挡住"顺手把中文当键"的回退 —— 那样又会回到
    /// "改中文措辞等于换键"的老问题。
    #[test]
    fn 键是规范化的小写标识符() {
        for 词条 in 词表 {
            assert!(
                词条.键.is_ascii(),
                "键 `{}` 含非 ASCII 字符；键必须是符号名，不能是中文原文",
                词条.键
            );
            assert!(
                词条
                    .键
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "键 `{}` 只能包含小写字母、数字与下划线",
                词条.键
            );
            assert!(
                !词条.键.starts_with('_') && !词条.键.ends_with('_'),
                "键 `{}` 不应以下划线开头或结尾",
                词条.键
            );
        }
    }

    /// 同一个键在中英两种语言下都要能查到，且结果就是词条里的值。
    #[test]
    fn 两种语言都能查到每一条词条() {
        for 词条 in 词表 {
            assert_eq!(语言::中文.取(词条.键), 词条.中文);
            assert_eq!(语言::英文.取(词条.键), 词条.英文);
            assert!(语言::中文.有键(词条.键));
        }
    }

    /// 中文与英文不应完全相同（除纯符号串外）。
    ///
    /// 完全相同通常意味着"忘了翻译，直接复制了中文"。
    /// 例外是纯符号/占位符串（如 `{标题}`），它们本来就与语言无关。
    #[test]
    fn 英文文案不应与中文完全相同() {
        let mut 疑似未翻译 = Vec::new();
        for 词条 in 词表 {
            if 词条.中文 == 词条.英文 {
                // 纯符号串允许相同
                let 有汉字 = 词条.中文.chars().any(
                    |c| matches!(u32::from(c), 0x3000..=0x303F | 0x4E00..=0x9FFF | 0xFF00..=0xFFEF),
                );
                let 有拉丁字母 = 词条.中文.chars().any(|c| c.is_ascii_alphabetic());
                if 有汉字 && 有拉丁字母 {
                    疑似未翻译.push(词条.键);
                }
            }
        }
        assert!(
            疑似未翻译.is_empty(),
            "以下词条中英完全相同、且含中英混排，疑似未翻译：{疑似未翻译:?}"
        );
    }

    /// 查不到的键返回键本身 —— 界面上会明显看出问题，而不是静默退回中文。
    #[test]
    fn 缺失的键返回键本身() {
        let 不存在的键 = "this_key_does_not_exist";
        assert_eq!(语言::中文.取(不存在的键), 不存在的键);
        assert_eq!(语言::英文.取(不存在的键), 不存在的键);
        assert!(!语言::中文.有键(不存在的键));
    }

    /// 两种语言下 `填充` 都要正确替换占位符。
    #[test]
    fn 填充在两种语言下都替换占位符() {
        for 词条 in 词表 {
            if !词条.中文.contains("{}") {
                continue;
            }
            let 占位符数 = 词条.中文.matches("{}").count();
            assert_eq!(
                词条.英文.matches("{}").count(),
                占位符数,
                "`{}` 的英文文案占位符个数与中文不一致 —— 填充时会错位",
                词条.键
            );
        }
    }

    /// 参数不足时不 panic，且留下可辨认的痕迹。
    #[test]
    fn 填充参数不足时不会崩溃() {
        let 含两占位符 = 词表
            .iter()
            .find(|词条| 词条.中文.matches("{}").count() == 2 && 词条.中文.contains("与"));
        if let Some(词条) = 含两占位符 {
            let 结果 = 语言::中文.填充(词条.键, &[&1]);
            assert!(结果.contains('1'));
        }
    }

    /// 语言代号必须能往返，未知代号回退到中文。
    #[test]
    fn 语言代号往返一致且未知代号回退() {
        for 语言 in 语言::全部 {
            assert_eq!(语言::从代号(语言.代号()), 语言);
        }
        assert_eq!(语言::从代号("zh-CN"), 语言::中文);
        assert_eq!(语言::从代号("klingon"), 语言::中文);
        assert_eq!(语言::从代号(""), 语言::中文);
        assert_eq!(
            语言::从代号("EN"),
            语言::中文,
            "代号区分大小写，未知值回退中文"
        );
    }

    /// 默认语言是中文。
    #[test]
    fn 默认语言是中文() {
        assert_eq!(语言::default(), 语言::中文);
    }

    /// 各语言用自己的写法自称，且能反查。
    #[test]
    fn 语言自称与反查一致() {
        assert_eq!(语言::中文.自称(), "简体中文");
        assert_eq!(语言::英文.自称(), "English");
        for 语言 in 语言::全部 {
            assert_eq!(语言::从自称(语言.自称()), Some(语言));
        }
    }

    /// 连续切换两次回到原语言。
    #[test]
    fn 切换语言可往返() {
        for 语言 in 语言::全部 {
            assert_eq!(语言.下一个().下一个(), 语言);
        }
    }

    /// 关键界面区域必须有词条 —— 这是"覆盖清单"测试。
    ///
    /// # 为什么要有清单
    ///
    /// `词表按键有序且无重复`、`两种语言都有非空文案` 这些只能保证
    /// "表里已有的条目是对的"，不能保证"该有的条目都在"。
    /// 某个区域整块漏了翻译，前面那些测试照样全绿。
    ///
    /// 这里按**界面区域**列出必须存在的键。新增区域时报错会直接给出
    /// 缺哪些键，而不是等用户切到英文才发现有一块还是中文。
    ///
    /// 清单只要求"键存在"，不校验文案措辞 —— 措辞会改，键不会。
    #[test]
    fn 关键界面区域的词条齐备() {
        // (区域名, 该区域必须存在的键)
        let 清单: &[(&str, &[&str])] = &[
            (
                "菜单栏",
                &[
                    "project",
                    "file",
                    "edit",
                    "scene",
                    "entity",
                    "assets",
                    "run_2",
                    "debug",
                    "window",
                    "help",
                    "menu_group_ui_language",
                ],
            ),
            (
                "工具栏",
                &[
                    "run_2",
                    "pause",
                    "resume",
                    "step",
                    "stop_2",
                    "undo_2",
                    "redo_2",
                    "save_scene",
                    "new_entity_2",
                    "delete",
                ],
            ),
            (
                "面板标题",
                &[
                    "scene_tree",
                    "inspector",
                    "viewport",
                    "console",
                    "history",
                    "performance",
                    "runtime_ecs",
                ],
            ),
            (
                "视口",
                &[
                    "viewport_2d_entity_count",
                    "viewport_zoom",
                    "fit_to_window_f",
                    "reset_view",
                    "grid",
                    "bounds",
                ],
            ),
            (
                "场景树与检查器",
                &["search", "entity_name", "inspector_no_entity_selected_hint"],
            ),
            // 检查器的字段名与提示来自组件注册表。
            //
            // 这一段是补上来的：上一轮注册表把**中文当键**存进
            // `属性描述::显示名`，Inspector 执行 `语言.取("可见")` 查不到，
            // 英文模式下字段名仍是中文。当时截图里 Inspector 根本没显示，
            // 我没验证就下了"检查器已覆盖"的结论。
            // 列进清单后，这类"注册表忘了换成键"会在测试阶段直接失败。
            (
                "检查器字段名（组件注册表）",
                &[
                    "transform_field_translation",
                    "transform_field_rotation_quat",
                    "transform_field_scale",
                    "visibility_field_visible",
                    "sprite_field_texture",
                    "sprite_field_tint",
                    "sprite_field_flip_horizontal",
                    "sprite_field_flip_vertical",
                    "sprite_field_custom_size",
                    "camera_field_ortho_half_height",
                ],
            ),
            (
                "检查器字段提示",
                &[
                    "transform_translation_tooltip",
                    "transform_rotation_tooltip",
                    "transform_scale_tooltip",
                    "visibility_field_tooltip",
                    "sprite_field_texture_tooltip",
                    "sprite_field_custom_size_tooltip",
                    "camera_ortho_half_height_tooltip",
                ],
            ),
            (
                "资源类别",
                &[
                    "asset_category_image",
                    "asset_category_audio",
                    "asset_category_font",
                    "asset_category_model",
                    "asset_category_material",
                    "asset_category_shader",
                    "asset_category_other",
                ],
            ),
            ("语言自称", &["language_self_zh_cn", "language_self_en"]),
            // 「关于」窗口。它只在用户主动打开时才出现，最容易被漏翻
            // 而没人发现；这里把整块文案列成清单，漏一条就失败。
            (
                "关于窗口",
                &[
                    "about_bevy_studio",
                    "about_tagline",
                    "about_modes",
                    "about_capabilities",
                    "about_implemented_features",
                    "about_unimplemented_note",
                    "about_ui_stack",
                    "about_scene_format",
                    "tech_stack",
                    "version",
                    "current_scene",
                ],
            ),
        ];

        let mut 缺失 = Vec::new();
        for (区域, 键列表) in 清单 {
            for 键 in *键列表 {
                if !语言::中文.有键(键) {
                    缺失.push(format!("{区域} → {键}"));
                }
            }
        }
        assert!(
            缺失.is_empty(),
            "以下关键界面区域缺少词条（英文模式下这些位置会是中文或键名）：{缺失:#?}"
        );
    }

    /// 文案里不得出现**字面量**的 `\n`（反斜杠 + n）。
    ///
    /// # 这是一条回归测试
    ///
    /// 生成器早先把 TSV 里的转义序列又转义了一次，于是 Rust 源码里成了
    /// `\\n`，运行时渲染出的是**可见的两个字符 `\n`** 而不是换行，
    /// 界面上表现为空状态提示写成 `没有资源。\n把图片拖进来`。
    ///
    /// 这类问题不会让任何其它测试变红（字符串非空、两种语言都有值），
    /// 只有真的看界面才发现，因此专门钉一条。
    #[test]
    fn 文案里不应出现字面量的反斜杠_n() {
        let 字面量 = "\\n"; // Rust 源码里就是两个字符：反斜杠 + n
        let mut 有问题 = Vec::new();
        for 词条 in 词表 {
            for (字段, 值) in [("中文", 词条.中文), ("英文", 词条.英文)] {
                if 值.contains(字面量) {
                    有问题.push(format!("{} 的{}", 词条.键, 字段));
                }
            }
        }
        assert!(
            有问题.is_empty(),
            "以下文案含**可见的** `\\n` 两字符而不是换行，界面会把它们原样显示出来。\
             成因是词表生成时的转义方向（见 tools/gen_messages.py）：{有问题:?}"
        );
    }

    /// 按设计应当多行的文案，必须真的含换行。
    ///
    /// 与上一条互补：上一条禁止"假换行"，这一条保证"该换的地方真换了"。
    #[test]
    fn 多行文案确实含换行() {
        for 键 in [
            "assets_empty_hint",
            "assets_no_project_hint",
            "hierarchy_empty_hint",
            "inspector_no_entity_selected_hint",
            "tooltip_field_name_and_type",
        ] {
            let 中 = 语言::中文.取(键);
            let 英 = 语言::英文.取(键);
            assert!(中.contains('\n'), "`{键}` 的中文应含真实换行，实际：{中:?}");
            assert!(英.contains('\n'), "`{键}` 的英文应含真实换行，实际：{英:?}");
        }
    }

    /// **每一个** `scene_doc` 错误类别都必须在词表里有中英词条。
    ///
    /// # 为什么必须有这条
    ///
    /// `scene_doc` 只输出消息键，不查表；键与词条是否对应，
    /// 只能在这一层核对。漏一条的后果是：英文模式下错误提示直接显示成键名。
    #[test]
    fn 每个场景错误类别都有中英词条() {
        let mut 缺失 = Vec::new();
        for 类别 in scene_doc::error::场景错误类别::全部 {
            let 键 = 类别.消息键();
            if !语言::中文.有键(键) || !语言::英文.有键(键) {
                缺失.push(键);
            }
        }
        assert!(
            缺失.is_empty(),
            "以下场景错误类别没有对应词条（英文模式下会显示键名）：{缺失:?}"
        );
    }

    /// 场景错误词条的中英占位符个数必须一致，否则填充会错位。
    #[test]
    fn 场景错误词条占位符中英一致() {
        for 类别 in scene_doc::error::场景错误类别::全部 {
            let 键 = 类别.消息键();
            let 中 = 语言::中文.取(键);
            let 英 = 语言::英文.取(键);
            assert_eq!(
                中.matches("{}").count(),
                英.matches("{}").count(),
                "`{键}` 中英占位符个数不一致：中={中} 英={英}"
            );
        }
    }
}
