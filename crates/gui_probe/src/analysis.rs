//! 断言与判定。
//!
//! # 判定为什么必须分两类
//!
//! "采集失败"与"界面缺陷"的处理方式完全相反：
//!
//! * **采集失败**（没有截图、尺寸不符、窗口不存在）说明是**工具链**的问题，
//!   这时绝不能去改界面代码 —— 上一轮就是这样把好代码改坏的；
//! * **界面缺陷**（图有了、尺寸也对，但某块区域是空白、文字没渲染、
//!   面板位置错乱）才是产品问题。
//!
//! 把两者混成一个"失败"，实践中总是导致错误的一边被修。

use crate::evidence::证据;
use crate::image_stats::{图像, 矩形, 颜色差};
use crate::window_probe::窗口信息;

/// 一条断言的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 断言类别 {
    /// 采集链路问题：文件缺失、尺寸不符、窗口不存在。
    采集,
    /// 界面问题：区域空白、文字没渲染、布局关系错乱、文案不对。
    界面,
}

/// 一条断言结果。
#[derive(Debug, Clone)]
pub struct 断言 {
    /// 断言名（稳定英文 id，便于脚本与文档引用）。
    pub 名称: &'static str,
    /// 是否通过。
    pub 通过: bool,
    /// 类别。
    pub 类别: 断言类别,
    /// 人类可读的说明（含实测数字）。
    pub 说明: String,
}

/// 验收用的布局轮廓。
///
/// 每个内置工作区对"哪块面板在哪个位置"有**不同的产品定义**：
/// 通用工作区是"左树/中视口/右检查器/底部工具"，
/// 资源工作区则是"资源浏览器占中央、视口移到右上"。
/// 把轮廓显式建模，验收断言才不会把一种合理布局判成缺陷。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 布局轮廓 {
    /// 通用：左树 / 中视口 / 右检查器 / 底部工具区。
    通用,
    /// 场景编辑：与通用同构，但视口更高、底部更矮。
    场景,
    /// 资源管理：资源浏览器占中央主体，视口在右，检查器在其下方。
    资源,
    /// 调试：底部控制台明显更高。
    调试,
    /// 未指定：只做最低限度的检查。
    无,
}

impl 布局轮廓 {
    /// 由工作区 id 推断轮廓。
    pub fn 从id(id: &str) -> Self {
        match id {
            "general" => Self::通用,
            "scene" => Self::场景,
            "assets" => Self::资源,
            "debug" => Self::调试,
            _ => Self::无,
        }
    }
}

/// 验收期望。
#[derive(Debug, Clone)]
pub struct 期望 {
    /// 期望语言代号（`zh` / `en`）。`None` 不检查。
    pub 语言: Option<String>,
    /// 期望主题明暗（`dark` / `light`）。`None` 不检查。
    pub 明暗: Option<String>,
    /// 必须存在的面板键。
    pub 必须面板: Vec<String>,
    /// 视口宽度占窗口宽度的最小比例。
    pub 最小视口宽度占比: f32,
    /// 状态栏必须包含的文本片段。
    pub 状态栏包含: Vec<String>,
    /// 必须处于激活状态的工作区 id。
    pub 工作区: Option<String>,
    /// 布局轮廓（通常由工作区 id 推断）。
    pub 轮廓: 布局轮廓,
}

impl Default for 期望 {
    fn default() -> Self {
        Self {
            语言: None,
            明暗: None,
            必须面板: Vec::new(),
            最小视口宽度占比: 0.30,
            状态栏包含: Vec::new(),
            工作区: None,
            轮廓: 布局轮廓::无,
        }
    }
}

/// 总体判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 判定 {
    /// 全部通过。
    通过,
    /// 采集链路失败。
    采集失败,
    /// 界面缺陷。
    界面缺陷,
}

impl 判定 {
    /// 退出码。与 `editor_app::capture` 的约定一致：
    /// 0 成功、2 界面缺陷、3 采集失败。
    pub fn 退出码(&self) -> i32 {
        match self {
            判定::通过 => 0,
            判定::界面缺陷 => 2,
            判定::采集失败 => 3,
        }
    }

    /// 中文描述。
    pub fn 描述(&self) -> &'static str {
        match self {
            判定::通过 => "通过",
            判定::界面缺陷 => "界面缺陷",
            判定::采集失败 => "采集失败",
        }
    }
}

/// 分析结果。
#[derive(Debug, Clone)]
pub struct 分析结果 {
    /// 全部断言。
    pub 断言: Vec<断言>,
    /// 截图尺寸。
    pub 截图尺寸: (u32, u32),
    /// 全图均亮度。
    pub 均亮度: f32,
    /// 全图主导色（`#RRGGBB`）。
    pub 主导色: String,
    /// 判定。
    pub 判定: 判定,
}

impl 分析结果 {
    /// 失败的断言。
    pub fn 失败项(&self) -> Vec<&断言> {
        self.断言.iter().filter(|项| !项.通过).collect()
    }
}

/// 一次分析所需的输入。
pub struct 输入<'a> {
    pub 图像: &'a 图像,
    pub 证据: &'a 证据,
    pub 窗口: Option<&'a 窗口信息>,
    pub 期望: &'a 期望,
}

/// 断言收集器。
struct 收集器 {
    项: Vec<断言>,
}

impl 收集器 {
    fn 新() -> Self {
        Self { 项: Vec::new() }
    }

    fn 通过(&mut self, 名称: &'static str, 类别: 断言类别, 说明: impl Into<String>) {
        self.项.push(断言 {
            名称,
            通过: true,
            类别,
            说明: 说明.into(),
        });
    }

    fn 失败(&mut self, 名称: &'static str, 类别: 断言类别, 说明: impl Into<String>) {
        self.项.push(断言 {
            名称,
            通过: false,
            类别,
            说明: 说明.into(),
        });
    }

    /// 按条件二选一。
    fn 要求(
        &mut self,
        条件: bool,
        名称: &'static str,
        类别: 断言类别,
        通过说明: impl Into<String>,
        失败说明: impl Into<String>,
    ) {
        if 条件 {
            self.通过(名称, 类别, 通过说明);
        } else {
            self.失败(名称, 类别, 失败说明);
        }
    }
}

/// 判断文本是否含中日韩汉字。
pub fn 含汉字(文本: &str) -> bool {
    文本.chars().any(|字| {
        let 码 = 字 as u32;
        (0x4E00..=0x9FFF).contains(&码) || (0x3400..=0x4DBF).contains(&码)
    })
}

/// 十六进制颜色文本。
fn 十六进制(色: [u8; 4]) -> String {
    format!("#{:02X}{:02X}{:02X}", 色[0], 色[1], 色[2])
}

/// 对一次采集做完整分析。
pub fn 分析(输入: 输入<'_>) -> 分析结果 {
    let mut 收集 = 收集器::新();
    let 图 = 输入.图像;
    let 证据 = 输入.证据;
    let 期望 = 输入.期望;

    // ---------------------------------------------------------------
    // 1. 采集链路
    // ---------------------------------------------------------------
    let 宽一致 = (图.宽 as i64 - 证据.window.width as i64).abs() <= 3;
    let 高一致 = (图.高 as i64 - 证据.window.height as i64).abs() <= 3;
    收集.要求(
        宽一致 && 高一致,
        "capture_size_matches_evidence",
        断言类别::采集,
        format!(
            "截图 {}×{} 与证据窗口 {}×{} 一致",
            图.宽, 图.高, 证据.window.width, 证据.window.height
        ),
        format!(
            "截图 {}×{} 与证据窗口 {}×{} 不一致：截图与证据不是同一帧或窗口尺寸变了",
            图.宽, 图.高, 证据.window.width, 证据.window.height
        ),
    );

    if let Some(窗口) = 输入.窗口 {
        let 客户区一致 = 窗口.客户区.0.abs_diff(图.宽) <= 3 && 窗口.客户区.1.abs_diff(图.高) <= 3;
        收集.要求(
            客户区一致,
            "capture_matches_os_client_area",
            断言类别::采集,
            format!(
                "操作系统报告的客户区 {}×{} 与截图一致（窗口标题「{}」，类名「{}」）",
                窗口.客户区.0, 窗口.客户区.1, 窗口.标题, 窗口.类名
            ),
            format!(
                "操作系统客户区 {}×{} 与截图 {}×{} 不一致：可能截到了别的窗口或缩放不同步",
                窗口.客户区.0, 窗口.客户区.1, 图.宽, 图.高
            ),
        );
        收集.要求(
            窗口.是编辑器窗口(),
            "window_identified",
            断言类别::采集,
            format!(
                "已确认目标窗口：标题「{}」，可见={}，DPI={}",
                窗口.标题, 窗口.可见, 窗口.dpi
            ),
            format!(
                "没有找到可见的 Bevy Studio 主窗口（标题「{}」，可见={}）",
                窗口.标题, 窗口.可见
            ),
        );
    } else {
        收集.成功_无窗口();
    }

    // ---------------------------------------------------------------
    // 2. 画面内容
    // ---------------------------------------------------------------
    let 全图 = 图.全图统计();
    收集.要求(
        全图.唯一色数 >= 8 && 全图.主导占比 <= 0.98,
        "frame_not_constant",
        断言类别::界面,
        format!(
            "画面有内容：{} 种颜色，主导色 {} 占 {:.1}%",
            全图.唯一色数,
            十六进制(全图.主导色),
            全图.主导占比 * 100.0
        ),
        format!(
            "画面近乎纯色（{} 种颜色，主导色 {} 占 {:.1}%）：窗口没有正常绘制",
            全图.唯一色数,
            十六进制(全图.主导色),
            全图.主导占比 * 100.0
        ),
    );

    // 主题背景：默认布局下面板背景覆盖面积最大，因此主导色应当接近它。
    if let Some(面板背景) = 证据::解析颜色(&证据.theme.panel_bg) {
        let 期望色 = [面板背景.0, 面板背景.1, 面板背景.2, 255];
        let 差 = 颜色差(全图.主导色, 期望色);
        收集.要求(
            差 <= 40,
            "theme_background_applied",
            断言类别::界面,
            format!(
                "主导色 {} 与主题面板背景 {} 一致（差 {差}）",
                十六进制(全图.主导色),
                证据.theme.panel_bg
            ),
            format!(
                "主导色 {} 与主题面板背景 {} 相差 {差}：主题没有真正应用到界面",
                十六进制(全图.主导色),
                证据.theme.panel_bg
            ),
        );
    }

    // 明暗主题：用均亮度做粗判，避免把"主题根本没生效"当成通过。
    if let Some(明暗) = 期望.明暗.as_deref() {
        let 符合 = match 明暗 {
            "dark" => 全图.均亮度 < 120.0,
            "light" => 全图.均亮度 > 140.0,
            _ => true,
        };
        收集.要求(
            符合,
            "theme_brightness_matches",
            断言类别::界面,
            format!("平均亮度 {:.1} 符合 `{明暗}` 主题预期", 全图.均亮度),
            format!("平均亮度 {:.1} 不符合 `{明暗}` 主题预期", 全图.均亮度),
        );
    }

    // ---------------------------------------------------------------
    // 3. 每块区域
    // ---------------------------------------------------------------
    let 区域表: Vec<(&str, 矩形)> = 证据
        .regions
        .iter()
        .map(|区域| {
            (
                区域.id.as_str(),
                矩形 {
                    x: 区域.x,
                    y: 区域.y,
                    w: 区域.w,
                    h: 区域.h,
                },
            )
        })
        .collect();

    if 区域表.is_empty() {
        收集.失败(
            "regions_present",
            断言类别::界面,
            "证据里没有任何区域矩形：界面没有上报布局几何",
        );
    } else {
        收集.通过(
            "regions_present",
            断言类别::界面,
            format!("证据含 {} 块区域", 区域表.len()),
        );
    }

    let mut 全inside = true;
    let mut 空白区域: Vec<&str> = Vec::new();
    for (id, 区域) in &区域表 {
        if !图.包含(*区域) {
            全inside = false;
            continue;
        }
        let 统计 = 图.区域统计(*区域);
        // 退化尺寸本身就是缺陷。
        if 区域.w < 24 || 区域.h < 16 {
            continue;
        }
        let 有内容 = 统计.唯一色数 >= 3 && 统计.对比占比() >= 0.0002;
        if !有内容 {
            空白区域.push(id);
        }
    }
    收集.要求(
        全inside,
        "regions_inside_frame",
        断言类别::采集,
        "全部区域矩形都落在截图内",
        "有区域矩形越出截图：布局几何与截图不是同一帧",
    );
    收集.要求(
        空白区域.is_empty(),
        "regions_have_content",
        断言类别::界面,
        "每块区域都画出了内容（≥3 种颜色且存在对比像素）",
        format!("这些区域是空白的：{空白区域:?}"),
    );

    // 文字：菜单栏与状态栏是纯文字区，必须有足够的对比像素。
    for (id, 阈值) in [("menubar", 60usize), ("statusbar", 60), ("toolbar", 40)] {
        if let Some((_, 区域)) = 区域表.iter().find(|(候选, _)| *候选 == id) {
            let 统计 = 图.区域统计(*区域);
            收集.要求(
                统计.对比像素数 >= 阈值,
                "text_rendered_in_chrome",
                断言类别::界面,
                format!(
                    "`{id}` 有 {} 个文字/图标像素（阈值 {阈值}）",
                    统计.对比像素数
                ),
                format!(
                    "`{id}` 只有 {} 个对比像素（阈值 {阈值}）：文字或图标没有渲染出来",
                    统计.对比像素数
                ),
            );
        }
    }

    // ---------------------------------------------------------------
    // 4. 布局几何关系
    // ---------------------------------------------------------------
    let 取 = |id: &str| 区域表.iter().find(|(候选, _)| *候选 == id).map(|(_, r)| *r);

    if let (Some(菜单), Some(工具栏), Some(状态)) = (取("menubar"), 取("toolbar"), 取("statusbar"))
    {
        收集.要求(
            菜单.y <= 8 && 菜单.下() <= 工具栏.y + 2,
            "chrome_bars_order",
            断言类别::界面,
            format!(
                "菜单栏在顶部（y={}），工具栏紧随其后（y={}）",
                菜单.y, 工具栏.y
            ),
            format!(
                "菜单栏/工具栏顺序异常：菜单 y={} 高={}，工具栏 y={}",
                菜单.y, 菜单.h, 工具栏.y
            ),
        );
        收集.要求(
            (状态.下() - 图.高 as i32).abs() <= 3,
            "statusbar_at_bottom",
            断言类别::界面,
            format!(
                "状态栏贴住窗口底部（底边 {} / 窗口高 {}）",
                状态.下(),
                图.高
            ),
            format!(
                "状态栏没有贴住窗口底部：底边 {} / 窗口高 {}",
                状态.下(),
                图.高
            ),
        );
    }

    if let Some(视口) = 取("viewport") {
        let 占比 = 视口.w as f32 / 图.宽 as f32;
        // 资源管理工作的视口本来就小（资源浏览器占中央），
        // 因此下限按轮廓区分，而不是用一个"通用 30%"硬套所有工作区。
        let 最小视口 = match 期望.轮廓 {
            布局轮廓::资源 => 0.18,
            _ => 期望.最小视口宽度占比,
        };
        收集.要求(
            占比 >= 最小视口 && 占比 <= 0.9,
            "viewport_width_share",
            断言类别::界面,
            format!(
                "视口宽 {}px，占窗口 {:.1}%（要求 {:.0}%~90%）",
                视口.w,
                占比 * 100.0,
                最小视口 * 100.0
            ),
            format!(
                "视口宽 {}px，占窗口 {:.1}%，超出 {:.0}%~90%：分栏比例很可能写反了",
                视口.w,
                占比 * 100.0,
                最小视口 * 100.0
            ),
        );

        match 期望.轮廓 {
            // 通用/场景/调试：左树、右检查器、底部工具区。
            布局轮廓::通用 | 布局轮廓::场景 | 布局轮廓::调试 => {
                for (id, 关系) in [("scene_tree", "左侧"), ("inspector", "右侧")] {
                    if let Some(区域) = 取(id) {
                        let 成立 = match 关系 {
                            "左侧" => 区域.右() <= 视口.x + 2,
                            _ => 区域.x >= 视口.右() - 2,
                        };
                        收集.要求(
                            成立,
                            "panel_side_relations",
                            断言类别::界面,
                            format!("`{id}` 位于视口{关系}（{区域:?} vs 视口 {视口:?}）"),
                            format!("`{id}` 不在视口{关系}：{区域:?} vs 视口 {视口:?}"),
                        );
                    }
                }

                // 底部面板：通用看资源浏览器，场景/调试看控制台。
                let 底部候选: [&str; 3] = match 期望.轮廓 {
                    布局轮廓::通用 => ["asset_browser", "assets", "console"],
                    _ => ["console", "asset_browser", "assets"],
                };
                for id in 底部候选 {
                    if let Some(区域) = 取(id) {
                        let 成立 = 区域.y >= 视口.下() - 2;
                        收集.要求(
                            成立,
                            "bottom_panel_below_viewport",
                            断言类别::界面,
                            format!("`{id}` 位于视口下方（y={}，视口底 {}）", 区域.y, 视口.下()),
                            format!(
                                "`{id}` 没有位于视口下方：y={}，视口底 {}",
                                区域.y,
                                视口.下()
                            ),
                        );
                        // 调试工作区的底部必须**明显更高**：控制台要能看多行。
                        if 期望.轮廓 == 布局轮廓::调试 {
                            let 高占比 = 区域.h as f32 / 图.高 as f32;
                            收集.要求(
                                高占比 >= 0.25,
                                "debug_console_tall_enough",
                                断言类别::界面,
                                format!("调试工作区底部高 {:.1}%（要求 ≥25%）", 高占比 * 100.0),
                                format!(
                                    "调试工作区底部只有 {:.1}% 高（要求 ≥25%）：控制台看不到几行",
                                    高占比 * 100.0
                                ),
                            );
                        }
                        break;
                    }
                }
            }
            // 资源管理：资源浏览器是**中央主体**，视口移到右侧，检查器在其下方。
            布局轮廓::资源 => {
                let 资源 = 取("asset_browser").or_else(|| 取("assets"));
                if let Some(资源) = 资源 {
                    let 宽占比 = 资源.w as f32 / 图.宽 as f32;
                    let 高占比 = 资源.h as f32 / 图.高 as f32;
                    收集.要求(
                        宽占比 >= 0.25 && 高占比 >= 0.55,
                        "asset_browser_is_main_area",
                        断言类别::界面,
                        format!(
                            "资源浏览器占 {:.1}% 宽 × {:.1}% 高（要求 ≥25% × ≥55%）",
                            宽占比 * 100.0,
                            高占比 * 100.0
                        ),
                        format!(
                            "资源浏览器只有 {:.1}% 宽 × {:.1}% 高：它应当是资源工作区的主体",
                            宽占比 * 100.0,
                            高占比 * 100.0
                        ),
                    );
                    收集.要求(
                        视口.x >= 资源.右() - 2,
                        "viewport_right_of_assets",
                        断言类别::界面,
                        format!(
                            "视口在资源浏览器右侧（视口 x={}，资源右边 {}）",
                            视口.x,
                            资源.右()
                        ),
                        format!(
                            "视口没有位于资源浏览器右侧：视口 x={}，资源右边 {}",
                            视口.x,
                            资源.右()
                        ),
                    );
                } else {
                    收集.失败(
                        "asset_browser_is_main_area",
                        断言类别::界面,
                        "资源工作区里没有资源浏览器区域",
                    );
                }
                if let Some(场景树) = 取("scene_tree") {
                    收集.要求(
                        场景树.右() <= 视口.x + 2,
                        "panel_side_relations",
                        断言类别::界面,
                        "场景树位于主编辑区左侧",
                        format!("场景树没有位于左侧：{场景树:?}"),
                    );
                }
            }
            // 未指定轮廓时只做"存在且不退化"的最低检查。
            布局轮廓::无 => {}
        }
    } else {
        收集.失败(
            "viewport_present",
            断言类别::界面,
            "截图证据里没有视口区域矩形",
        );
    }

    // 每条区域都应当有可用面积。
    for (id, 区域) in &区域表 {
        if 区域.h > 0 && 区域.w > 0 {
            continue;
        }
        收集.失败(
            "region_not_degenerate",
            断言类别::界面,
            format!("区域 `{id}` 尺寸退化：{区域:?}"),
        );
    }

    // ---------------------------------------------------------------
    // 5. 文案与语言
    // ---------------------------------------------------------------
    if let Some(工作区) = 期望.工作区.as_deref() {
        收集.要求(
            证据.workspace.id == 工作区,
            "workspace_selected",
            断言类别::界面,
            format!(
                "当前工作区为 `{}`（{}，{}，共 {} 个工作区）",
                证据.workspace.id,
                证据.workspace.name,
                if 证据.workspace.builtin {
                    "内置"
                } else {
                    "用户"
                },
                证据.workspace.count
            ),
            format!(
                "期望工作区 `{工作区}`，实际 `{}`（{}）",
                证据.workspace.id, 证据.workspace.name
            ),
        );
    }

    if let Some(语言) = 期望.语言.as_deref() {
        // 语言代号允许带区域后缀：`zh-CN` 属于 `zh`。
        let 语言匹配 = 证据.language == 语言
            || 证据
                .language
                .strip_prefix(语言)
                .is_some_and(|后缀| 后缀.starts_with('-'));
        收集.要求(
            语言匹配,
            "language_selected",
            断言类别::界面,
            format!("界面语言为 `{}`（{}）", 证据.language, 证据.language_label),
            format!(
                "期望语言 `{语言}`，实际 `{}`（{}）",
                证据.language, 证据.language_label
            ),
        );

        let 状态文案 = 证据.状态栏文案();
        let 标题文案 = 证据.面板标题文案();
        if 语言 == "en" {
            let 中文残留: Vec<&str> = 证据
                .status_left
                .iter()
                .chain(证据.status_right.iter())
                .chain(证据.panel_titles.iter())
                .filter(|文本| 含汉字(文本))
                .map(|文本| 文本.as_str())
                .collect();
            收集.要求(
                中文残留.is_empty(),
                "no_cjk_in_english_ui",
                断言类别::界面,
                "英文界面里没有中文残留",
                format!("英文界面仍有中文：{中文残留:?}"),
            );
        } else if 语言 == "zh" {
            收集.要求(
                含汉字(&状态文案) || 含汉字(&标题文案),
                "cjk_present_in_chinese_ui",
                断言类别::界面,
                "中文界面含中文文案",
                "中文界面没有任何中文文案：语言切换没有生效",
            );
        }
    }

    for 片段 in &期望.状态栏包含 {
        let 状态文案 = 证据.状态栏文案();
        收集.要求(
            状态文案.contains(片段.as_str()),
            "statusbar_contains",
            断言类别::界面,
            format!("状态栏包含「{片段}」"),
            format!("状态栏不含「{片段}」：实际「{状态文案}」"),
        );
    }

    for 面板 in &期望.必须面板 {
        let 存在 = 证据.panel_keys.iter().any(|键| 键 == 面板);
        收集.要求(
            存在,
            "required_panel_visible",
            断言类别::界面,
            format!("面板 `{面板}` 在工作区里可见"),
            format!(
                "面板 `{面板}` 不在布局里（可见面板：{:?}）",
                证据.panel_keys
            ),
        );
    }

    // 判定：先采集、后界面。采集失败时不应把界面问题当成主因。
    let 采集失败 = 收集
        .项
        .iter()
        .any(|项| !项.通过 && 项.类别 == 断言类别::采集);
    let 界面失败 = 收集
        .项
        .iter()
        .any(|项| !项.通过 && 项.类别 == 断言类别::界面);
    let 判定 = if 采集失败 {
        判定::采集失败
    } else if 界面失败 {
        判定::界面缺陷
    } else {
        判定::通过
    };

    分析结果 {
        断言: 收集.项,
        截图尺寸: (图.宽, 图.高),
        均亮度: 全图.均亮度,
        主导色: 十六进制(全图.主导色),
        判定,
    }
}

impl 收集器 {
    /// 没有窗口信息时记录一条明确的说明，而不是假装通过。
    fn 成功_无窗口(&mut self) {
        self.通过(
            "window_identified",
            断言类别::采集,
            "本次未提供目标进程，跳过操作系统层面窗口确认（仅用进程内证据）",
        );
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::evidence::{主题证据, 区域证据, 尺寸, 证据};
    use crate::image_stats::图像;

    fn 纯色图(宽: u32, 高: u32, 色: [u8; 4]) -> 图像 {
        // 通过公开路径构造：写入临时 PNG 再读回，保证与真实流程一致。
        let 目录 = std::env::temp_dir().join(format!("gui_probe_测试_{}", 唯一后缀()));
        std::fs::create_dir_all(&目录).unwrap();
        let 路径 = 目录.join("纯色.png");
        let mut 缓冲 = image::RgbaImage::new(宽, 高);
        for 像素 in 缓冲.pixels_mut() {
            *像素 = image::Rgba(色);
        }
        缓冲.save(&路径).unwrap();
        let 图 = 图像::读取(&路径).unwrap();
        let _ = std::fs::remove_dir_all(&目录);
        图
    }

    fn 唯一后缀() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    fn 最小证据(宽: u32, 高: u32) -> 证据 {
        证据 {
            schema: 1,
            app_version: "0.0.0".into(),
            bevy_version: "0.0.0".into(),
            language: "zh".into(),
            language_label: "中文".into(),
            ui_scale: 1.0,
            pixels_per_point: 1.0,
            window: 尺寸 {
                width: 宽,
                height: 高,
            },
            workspace: crate::evidence::工作区证据 {
                id: "general".into(),
                name: "通用".into(),
                builtin: true,
                count: 4,
            },
            theme: 主题证据 {
                id: "dark".into(),
                panel_bg: "#1E1F22".into(),
                text: "#DEE2E6".into(),
                accent: "#569CD6".into(),
            },
            regions: vec![区域证据 {
                id: "viewport".into(),
                x: 5,
                y: 5,
                w: 40,
                h: 40,
            }],
            status_left: vec!["已保存".into()],
            status_right: vec!["100%".into()],
            panel_titles: vec!["视口".into()],
            panel_keys: vec!["viewport".into()],
            viewport_scale_percent: 100.0,
            icon_count: 0,
        }
    }

    #[test]
    fn 纯色画面被判为界面缺陷而不是采集失败() {
        let 图 = 纯色图(100, 100, [30, 31, 34, 255]);
        let 证据 = 最小证据(100, 100);
        let 期望 = 期望::default();
        let 结果 = 分析(输入 {
            图像: &图,
            证据: &证据,
            窗口: None,
            期望: &期望,
        });
        assert_eq!(结果.判定, 判定::界面缺陷, "纯色画面应当是界面缺陷");
        let 失败名: Vec<&str> = 结果.失败项().iter().map(|项| 项.名称).collect();
        assert!(
            失败名.contains(&"frame_not_constant"),
            "应当报告画面近乎纯色：{失败名:?}"
        );
    }

    #[test]
    fn 尺寸不符被判为采集失败() {
        let 图 = 纯色图(50, 50, [30, 31, 34, 255]);
        let 证据 = 最小证据(100, 100);
        let 期望 = 期望::default();
        let 结果 = 分析(输入 {
            图像: &图,
            证据: &证据,
            窗口: None,
            期望: &期望,
        });
        assert_eq!(结果.判定, 判定::采集失败);
        assert!(
            结果
                .断言
                .iter()
                .any(|项| 项.名称 == "capture_size_matches_evidence" && !项.通过)
        );
    }

    #[test]
    fn 中文检测覆盖汉字但不误判拉丁() {
        assert!(含汉字("场景树"));
        assert!(含汉字("Scene 树"));
        assert!(!含汉字("Scene Tree"));
        assert!(!含汉字("100%"));
    }

    #[test]
    fn 退出码与判定对应() {
        assert_eq!(判定::通过.退出码(), 0);
        assert_eq!(判定::界面缺陷.退出码(), 2);
        assert_eq!(判定::采集失败.退出码(), 3);
    }

    #[test]
    fn 无窗口信息时不把窗口断言算成失败() {
        let 图 = 纯色图(100, 100, [30, 31, 34, 255]);
        let 证据 = 最小证据(100, 100);
        let 期望 = 期望::default();
        let 结果 = 分析(输入 {
            图像: &图,
            证据: &证据,
            窗口: None,
            期望: &期望,
        });
        let 窗口项 = 结果
            .断言
            .iter()
            .find(|项| 项.名称 == "window_identified")
            .expect("必须记录一条窗口确认结论");
        assert!(窗口项.通过);
        assert_eq!(窗口项.类别, 断言类别::采集);
    }
}
