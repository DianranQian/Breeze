//! 中文字体支持。
//!
//! # 为什么需要这个模块
//!
//! `egui` 自带四种字体，**全都不含汉字**：
//!
//! | 字体 | 大小 | 覆盖 |
//! | --- | --- | --- |
//! | `Hack-Regular.ttf` | 0.30 MB | 拉丁（等宽） |
//! | `Ubuntu-Light.ttf` | 0.34 MB | 拉丁（比例） |
//! | `NotoEmoji-Regular.ttf` | 0.40 MB | Emoji |
//! | `emoji-icon-font.ttf` | 0.31 MB | 图标 |
//!
//! 而本编辑器的界面**几乎全是中文**。不做处理的话，所有菜单、面板标题、
//! 日志都会渲染成豆腐块（`□□□`），编辑器等于不可用。
//!
//! 这个缺陷的特点是：**编译通过、clippy 零警告、单元测试全绿，
//! 只有真正把窗口打开看才会发现。**
//!
//! # 为什么把字体作为资产打包，而不是读系统字体
//!
//! 最初的实现是扫描系统字体目录（`msyh.ttc` → `Deng.ttf` → `simhei.ttf` …）。
//! 它省下了仓库体积，但代价很实在：
//!
//! * **不确定**：换一台机器、换一个 Windows 版本（或精简版系统、
//!   Linux 容器、CI 环境）就可能一个都找不到，界面直接变方框；
//! * **代码多**：需要维护一份跨平台候选表、集合索引、
//!   以及一整套"找不到怎么办"的错误处理；
//! * **难以测试**：测试结果依赖运行时的机器状态，
//!   换台机器就可能是另一个结果。
//!
//! 改成打包资产之后这三点都消失了：**字体是程序的一部分**，
//! 在哪台机器上渲染结果都一样，测试也是确定的。
//!
//! 代价是仓库与二进制各多出约 2.2 MB。这是有意的取舍，
//! 详见 `crates/editor_ui/assets/fonts/README.md`。
//!
//! # 为什么用子集而不是完整字体
//!
//! 完整字体 16.95 MB，静态实例 10.11 MB，而界面用不到那么多字。
//! 打包的是 **GB2312 全集（6763 字）+ 源码用到的全部字符 +
//! ASCII + 标点**，约 2.2 MB。
//!
//! 为什么不止于"源码里出现的字"（那样只要 0.32 MB）：
//! **实体名、场景名、日志里的路径都是用户输入的，字符不可预知。**
//! 场景树是核心面板，用户给实体起中文名是日常操作，
//! 只打包源码字符就会让这些名字变成方框。
//!
//! # 为什么中文字体作为"后备"而不是"主字体"
//!
//! `epaint` 的查找规则是：从家族列表第一项开始找，找不到该字形就顺延。
//! 因此把中文字体**追加**到默认字体之后，就能同时得到：
//!
//! * 拉丁字母用 `Ubuntu-Light` / `Hack` 渲染（拉丁字形更精致）；
//! * 汉字自动落到中文字体上。
//!
//! 反过来的话，界面上大量的 `Sprite`、`Transform`、`Vec3`
//! 也会用中文字体渲染，观感明显变差。

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use egui::epaint::text::{FontData, FontDefinitions, FontFamily};

/// 打包进二进制的中文字体。
///
/// 用 `include_bytes!` 而不是运行时读文件：这样字体是**程序的一部分**，
/// 无论从哪个工作目录启动、无论可执行文件被复制到哪里，都能找到它。
/// 运行时读文件则要考虑相对路径、安装目录、被移动等情况，
/// 每一种都可能让界面变成方框。
const 内置字体字节: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Subset.ttf");

/// 内置字体在 `FontDefinitions` 里的键名。
pub const 内置字体键: &str = "bevy_studio_内置中文字体";

/// 内置字体的展示名称（用于日志）。
pub const 内置字体名称: &str = "Noto Sans SC（内置子集）";

/// 允许用户用环境变量追加一个后备字体。
///
/// 用途：内置子集覆盖 GB2312 的 6763 字，但不含 Unicode 扩展区的
/// 生僻字（罕见人名、地名用字）。需要这些字时，指向一个完整字体即可，
/// 不必改代码或重新编译。
pub const 后备字体环境变量: &str = "BEVY_STUDIO_FALLBACK_FONT";
/// 后备字体为 `.ttc` 集合时，用于指定字面序号。
pub const 后备字体索引环境变量: &str = "BEVY_STUDIO_FALLBACK_FONT_INDEX";

/// 字体安装结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 字体安装结果 {
    /// 内置字体是否已装入（正常情况恒为 `true`）。
    pub 内置已装入: bool,
    /// 追加的后备字体路径（若用户设置了环境变量且文件存在）。
    pub 后备字体: Option<PathBuf>,
}

/// 获取内置字体的字节。
pub fn 内置字体() -> &'static [u8] {
    内置字体字节
}

/// 内置字体的字节数（用于日志与测试）。
pub fn 内置字体大小() -> usize {
    内置字体字节.len()
}

/// 读取用户指定的后备字体。文件不存在时返回 `None`。
///
/// 刻意不让它成为错误：后备字体是可选增强，缺失不应该妨碍启动。
pub fn 读取后备字体() -> Option<(PathBuf, u32, Vec<u8>)> {
    let 路径文本 = std::env::var(后备字体环境变量).ok()?;
    if 路径文本.trim().is_empty() {
        return None;
    }
    let 路径 = PathBuf::from(路径文本);
    let 字节 = std::fs::read(&路径).ok()?;
    let 索引 = std::env::var(后备字体索引环境变量)
        .ok()
        .and_then(|值| 值.parse::<u32>().ok())
        .unwrap_or(0);
    Some((路径, 索引, 字节))
}

/// 构造字体定义：默认字体 + 内置中文子集（+ 可选后备字体）。
///
/// 中文字体一律**追加在末尾**，因此不会挤掉默认的拉丁字体。
pub fn 构造字体定义(后备: Option<(PathBuf, u32, Vec<u8>)>) -> FontDefinitions {
    let mut 定义 = FontDefinitions::default();

    定义.font_data.insert(
        内置字体键.to_string(),
        Arc::new(FontData {
            font: Cow::Borrowed(内置字体字节),
            // 子集是静态字体，不是字体集合，因此字面序号固定为 0。
            index: 0,
            tweak: Default::default(),
        }),
    );

    let mut 家族列表 = vec![内置字体键.to_string()];

    // 可选的后备字体：接在内置字体**之后**，只在内置字体缺字时才用到。
    if let Some((路径, 索引, 字节)) = 后备 {
        let 键 = "bevy_studio_后备字体".to_string();
        定义.font_data.insert(
            键.clone(),
            Arc::new(FontData {
                font: Cow::Owned(字节),
                index: 索引,
                tweak: Default::default(),
            }),
        );
        let _ = 路径;
        家族列表.push(键);
    }

    for 家族 in [FontFamily::Proportional, FontFamily::Monospace] {
        定义
            .families
            .entry(家族)
            .or_default()
            .extend(家族列表.iter().cloned());
    }

    定义
}

/// 把中文字体安装到 egui 上下文。返回安装结果。
///
/// 只在启动时调用一次：`set_fonts` 会让 egui 重建字体图集并重新排版，
/// 重复调用会白白付出这个代价。
pub fn 安装中文字体(上下文: &egui::Context) -> 字体安装结果 {
    let 后备 = 读取后备字体();
    let 后备路径 = 后备.as_ref().map(|(路径, _, _)| 路径.clone());

    上下文.set_fonts(构造字体定义(后备));

    字体安装结果 {
        内置已装入: true,
        后备字体: 后备路径,
    }
}

/// 用于验证字体覆盖的代表性文本。
///
/// 取自界面上**真实出现**的字符串，覆盖菜单名、面板名、
/// 检查器字段、日志文案。
pub const 界面代表性中文: &[&str] = &[
    "项目",
    "文件",
    "编辑",
    "场景",
    "实体",
    "资源",
    "运行",
    "调试",
    "窗口",
    "帮助",
    "场景树",
    "检查器",
    "视口",
    "资源浏览器",
    "控制台",
    "历史",
    "性能",
    "保存场景",
    "撤销重做",
    "新建实体",
    "删除组件",
    "位置",
    "旋转",
    "缩放",
    "平移",
    "可见性",
    "自定义尺寸",
    "水平翻转",
    "未选中任何实体",
    "已保存",
    "未保存修改",
    "预览渲染尚未就绪",
];

/// 用 `epaint` 检查一组字体定义能否渲染给定的全部文本。
///
/// 返回**缺少字形**的那些文本。
///
/// 用 `has_glyphs` 而不是"试着排版一下"：后者对缺失字形会
/// 静默画出豆腐块，看不出问题。`has_glyphs` 直接查字体链里
/// 有没有这个字形，才是可靠的判据。
pub fn 检查覆盖(定义: &FontDefinitions, 文本列表: &[&str]) -> Vec<String> {
    let mut 字体 =
        egui::epaint::text::Fonts::new(egui::epaint::text::TextOptions::default(), 定义.clone());
    let 字体标识 = egui::FontId::proportional(14.0);

    文本列表
        .iter()
        .filter(|文本| !字体.has_glyphs(&字体标识, 文本))
        .map(|文本| (*文本).to_string())
        .collect()
}

/// 检查一段文本里哪些**字符**缺少字形。
///
/// 与 [`检查覆盖`] 的区别：它按单字报告，用于把
/// "哪几个字没覆盖"说清楚，而不是只说"这句不行"。
pub fn 缺少字形的字符(定义: &FontDefinitions, 文本: &str) -> Vec<char> {
    let mut 字体 =
        egui::epaint::text::Fonts::new(egui::epaint::text::TextOptions::default(), 定义.clone());
    let 字体标识 = egui::FontId::proportional(14.0);

    let mut 缺失: Vec<char> = 文本
        .chars()
        .filter(|字符| !字体.has_glyph(&字体标识, *字符))
        .collect();
    缺失.sort_unstable();
    缺失.dedup();
    缺失
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 先证明问题真实存在：egui 默认字体**不含**汉字。
    ///
    /// 这条是"问题存在"的证据。若哪天 egui 上游内置了 CJK 字体，
    /// 它会失败 —— 那时本模块可以大幅简化。
    #[test]
    fn egui_默认字体不含中文字形() {
        let 默认 = FontDefinitions::default();
        let 缺失 = 检查覆盖(&默认, &["场景树", "检查器"]);

        assert!(
            !缺失.is_empty(),
            "egui 默认字体竟然覆盖了汉字（缺失列表为空）。\
             这说明上游已内置 CJK 字体，本模块的存在前提需要重新评估。"
        );
    }

    /// 内置字体必须覆盖界面上的全部代表性中文。
    #[test]
    fn 内置字体覆盖界面代表性中文() {
        let 定义 = 构造字体定义(None);
        let 缺失 = 检查覆盖(&定义, 界面代表性中文);

        assert!(
            缺失.is_empty(),
            "内置字体缺少以下中文的字形：{缺失:?}\n\
             解决办法：运行 crates/editor_ui/assets/fonts/重新生成子集.py 重建字体子集。"
        );
    }

    /// **核心维护性测试**：内置字体必须覆盖界面源码里出现的每一个汉字。
    ///
    /// # 为什么这条必须有
    ///
    /// 字体子集是"按字符集生成"的：它只会包含生成时收集到的字。
    /// 因此**新增一句界面文案，如果里面有子集外的字，它就会显示成方框** ——
    /// 而且编译、clippy、其它测试全都不会报错。
    ///
    /// 这条测试把界面源码全文读进来逐字检查，把那个"只有看窗口才发现"
    /// 的问题变成了一条会在提交前失败的测试。
    #[test]
    fn 内置字体覆盖界面源码里的全部汉字() {
        let 定义 = 构造字体定义(None);

        // 收集 editor_ui 全部源码（界面文案都在这里）。
        let 界面目录 = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut 全文 = String::new();
        let mut 文件数 = 0usize;
        for 条目 in 遍历_rs文件(&界面目录) {
            if let Ok(内容) = std::fs::read_to_string(&条目) {
                全文.push_str(&内容);
                文件数 += 1;
            }
        }
        assert!(文件数 > 0, "应当能读到界面源码，实际读到 {文件数} 个文件");

        // 只挑汉字来检查：ASCII、标点、Emoji 由默认字体或内置标点覆盖，
        // 这里关心的是"中文文案有没有漏字"。
        let 中文: String = 全文
            .chars()
            .filter(|字符| {
                matches!(u32::from(*字符), 0x3000..=0x303F | 0x4E00..=0x9FFF | 0xFF00..=0xFFEF)
            })
            .collect();

        let 缺失 = 缺少字形的字符(&定义, &中文);
        assert!(
            缺失.is_empty(),
            "界面源码里出现了内置字体不含的字符：{}\n\
             共 {} 个不同字符，扫描了 {文件数} 个源文件。\n\
             解决办法：运行 crates/editor_ui/assets/fonts/重新生成子集.py 重建字体子集。",
            缺失.iter().collect::<String>(),
            缺失.len()
        );
    }

    /// 内置字体必须覆盖用户可能输入的**常用**汉字。
    ///
    /// 场景树显示的是用户自己起的实体名，字符不可预知。
    /// 这里抽查一批典型的游戏开发常用字（多为 GB2312 内），
    /// 确认它们都有字形 —— 否则用户起个中文名就会看到方框。
    #[test]
    fn 内置字体覆盖用户可能输入的常用汉字() {
        let 定义 = 构造字体定义(None);
        // 一组典型的实体/场景命名用字
        let 用户用字 = "勇者之剑森林小径玩家敌人子弹关卡主菜单生命值攻击力防御\
                        金币宝箱传送门沙漠雪山地牢城堡龙王公主骑士法师盗贼\
                        火焰冰冻雷电毒药治疗复活升级经验装备武器护甲头盔\
                        左边右边上边下边前后中间开始结束暂停继续重试胜利失败";
        let 缺失 = 缺少字形的字符(&定义, 用户用字);
        assert!(
            缺失.is_empty(),
            "以下常用汉字缺字形，用户起中文名会看到方框：{:?}",
            缺失
        );
    }

    /// 拉丁字符仍由默认字体覆盖（内置字体只是追加，没有挤掉它们）。
    #[test]
    fn 拉丁字符仍被覆盖() {
        let 定义 = 构造字体定义(None);
        let 缺失 = 检查覆盖(
            &定义,
            &["Sprite", "Transform", "Vec3", "Visibility", "Camera2d"],
        );
        assert!(缺失.is_empty(), "拉丁字符应当被覆盖，缺失：{缺失:?}");
    }

    /// 内置中文字体必须只**追加**，不能挤掉默认字体的顺序。
    ///
    /// 否则 `Sprite`、`Transform` 这类高频英文标识符会改用中文字体渲染，
    /// 观感明显变差。
    #[test]
    fn 中文字体只追加不替换默认字体() {
        let 定义 = 构造字体定义(None);
        let 默认 = FontDefinitions::default();

        for 家族 in [FontFamily::Proportional, FontFamily::Monospace] {
            let 默认列表 = 默认.families.get(&家族).cloned().unwrap_or_default();
            let 实际列表 = 定义.families.get(&家族).cloned().unwrap_or_default();

            assert_eq!(
                实际列表.len(),
                默认列表.len() + 1,
                "{家族:?} 家族应当只是在默认列表末尾追加了一项：\
                 默认 {} 项，实际 {} 项",
                默认列表.len(),
                实际列表.len()
            );
            assert_eq!(
                &实际列表[..默认列表.len()],
                默认列表.as_slice(),
                "{家族:?} 家族原有的字体顺序被改动了"
            );
            assert_eq!(
                实际列表.last().map(String::as_str),
                Some(内置字体键),
                "追加的应当是内置中文字体"
            );
        }
    }

    /// 内置字体资源必须真的存在且大小合理。
    ///
    /// 防止有人误删或误替换成空文件（那样界面会静默变方框）。
    #[test]
    fn 内置字体大小合理() {
        let 大小 = 内置字体大小();
        assert!(
            大小 > 1_000_000,
            "内置字体只有 {大小} 字节，太小了 —— 很可能被替换成了残缺文件"
        );
        assert!(
            大小 < 8_000_000,
            "内置字体有 {大小} 字节，超出预期（子集应当约 2.2 MB）"
        );
    }

    /// 内置字体必须是合法的 TrueType/OpenType 文件。
    #[test]
    fn 内置字体文件头合法() {
        let 字节 = 内置字体();
        assert!(字节.len() > 12, "字体文件太短");
        // sfnt 版本标签：0x00010000（TrueType）或 "OTTO"（CFF）
        let 标签 = &字节[0..4];
        let 合法 = 标签 == [0x00, 0x01, 0x00, 0x00] || 标签 == b"OTTO" || 标签 == b"true";
        assert!(
            合法,
            "字体文件头不是已知的 sfnt 标签：{标签:?}。\
             文件可能被损坏或根本不是字体。"
        );
    }

    /// 后备字体缺失时不应报错，只是不追加。
    #[test]
    fn 后备字体缺失时静默跳过() {
        // 不依赖环境变量状态：直接验证"读取函数在路径不存在时返回 None"
        // 这一行为（通过构造一个必然不存在的路径来间接确认）。
        let 不存在的 = PathBuf::from("C:/绝对不存在的后备字体_测试用.ttf");
        assert!(!不存在的.is_file());
        // `读取后备字体` 读取的是环境变量，这里主要确认它不会 panic。
        let _ = 读取后备字体();

        // 没有后备字体时，构造出的定义依然可用且覆盖中文。
        let 定义 = 构造字体定义(None);
        assert!(检查覆盖(&定义, &["场景树"]).is_empty());
    }

    /// 追加后备字体时，它必须排在内置字体**之后**。
    ///
    /// 顺序决定优先级：内置子集优先，后备字体只在内置缺字时才被用到。
    #[test]
    fn 后备字体排在内置字体之后() {
        let 假后备 = (PathBuf::from("C:/fake-fallback.ttf"), 2u32, vec![0u8; 16]);
        let 定义 = 构造字体定义(Some(假后备));

        for 家族 in [FontFamily::Proportional, FontFamily::Monospace] {
            let 列表 = 定义.families.get(&家族).cloned().unwrap_or_default();
            let 内置位置 = 列表.iter().position(|项| 项 == 内置字体键);
            let 后备位置 = 列表.iter().position(|项| 项 == "bevy_studio_后备字体");
            assert!(内置位置.is_some(), "{家族:?} 家族应含内置字体");
            assert!(后备位置.is_some(), "{家族:?} 家族应含后备字体");
            assert!(
                内置位置 < 后备位置,
                "内置字体必须排在后备字体之前，否则用户的后备字体会\
                 抢走本应由内置字体渲染的字"
            );
        }

        // 集合索引也要被保留
        let 数据 = 定义.font_data.get("bevy_studio_后备字体").unwrap();
        assert_eq!(数据.index, 2, "后备字体的集合字面序号必须被传递");
    }

    /// 递归收集目录下的 `.rs` 文件。
    fn 遍历_rs文件(目录: &std::path::Path) -> Vec<PathBuf> {
        let mut 结果 = Vec::new();
        let mut 栈 = vec![目录.to_path_buf()];
        while let Some(当前) = 栈.pop() {
            let Ok(读取) = std::fs::read_dir(&当前) else {
                continue;
            };
            for 条目 in 读取.flatten() {
                let 路径 = 条目.path();
                if 路径.is_dir() {
                    栈.push(路径);
                } else if 路径.extension().and_then(|扩展| 扩展.to_str()) == Some("rs") {
                    结果.push(路径);
                }
            }
        }
        结果
    }
}
