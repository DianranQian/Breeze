//! 场景树面板。
//!
//! # 为什么是"先收集显示行，再绘制"
//!
//! 最直觉的写法是在递归绘制每一行时直接去读文档、判断是否选中。
//! 但那样会同时持有 `&场景文档` 的借用与 `&mut 界面状态` 的借用，
//! 而重命名输入框、折叠箭头都需要改界面状态，借用检查器会拒绝。
//!
//! 这里改成两步：先把整棵树摊平成 [`显示行`] 列表（纯读取，可单独测试），
//! 再拿着这份快照绘制并生成动作。代价是一次 O(n) 的复制，
//! 换来的是清晰的借用边界和**可测的树构建逻辑**。
//!
//! # 一行由三个独立区域组成
//!
//! ```text
//! [缩进][折叠区 18px][类型图标区][名称]              [隐藏][锁定]
//!         ↑可点击        ↑不可折叠          ↑点击选中
//!         ↑只改折叠状态  ↑只表达节点类型
//! ```
//!
//! 三者的宽度都是固定值（见 [`折叠区宽度`] / [`类型图标区宽度`]），
//! 因此**名称的起始横坐标不随节点类型、图标尺寸或折叠状态变化而跳动**。
//!
//! # 为什么不再用字符画箭头与状态开关
//!
//! 整改前这里用的是 `▸` / `▾` / `👁` / `🔒` 等字符。内置中文字体子集里
//! 没有这些字形，egui 于是渲染成**空心方框（tofu）** —— 用户看到的
//! "所有节点左边都是空心正方形"就是这么来的，它看起来像一个节点图标，
//! 实际却是"字体缺字形"的占位符。现在全部改为 `crate::图标` 里的
//! 真实 PNG 图标（与工具栏、资源面板同源同风格）。

use editor_core::state::编辑器状态;
use std::collections::BTreeSet;

use egui::{Color32, RichText, Sense, Ui};
use scene_doc::document::{场景文档, 实体标识};

use crate::icons::图标;
use crate::node_icons::{折叠指示, 节点类型};
use crate::state::{UiAction, 文本编辑目标, 绘制上下文, 选择意图};

/// 折叠区宽度（逻辑像素）。
///
/// 固定值：叶子节点用 `add_space` 占住同样宽度，因此**父子节点的类型图标
/// 与名称始终对齐**。这个值要足够点击（不小于 18），又不能让深层缩进过早
/// 把名称挤出视野。
pub fn 折叠区宽度() -> f32 {
    18.0
}

/// 类型图标区宽度。
///
/// 只比图标本身宽几个像素，保证图标与名称之间有呼吸感，
/// 同时不让"图标列"抢占名称空间。
pub fn 类型图标区宽度(图标尺寸: f32) -> f32 {
    图标尺寸 + 4.0
}

/// 右侧状态开关（隐藏 / 锁定）的按钮宽度。
pub fn 开关宽度() -> f32 {
    18.0
}

/// 单行高度。
///
/// 由"交互尺寸"与"图标尺寸 + 上下留白"的较大者决定：图标变大时行必须
/// 跟着变高，否则图标会被裁切；字号变大时同样成立。
pub fn 行高(ui: &Ui, 上下文: &绘制上下文<'_>) -> f32 {
    let 图标 = 上下文.外观.图标尺寸;
    ui.spacing().interact_size.y.max(图标 + 4.0).max(18.0)
}

/// 行内图标的实际绘制尺寸。
///
/// 夹在行高之内并留 2px 余量：否则主题把 `图标尺寸` 调到 32 时，
/// 图标会溢出 22px 高的行，表现为行与行互相覆盖。
pub fn 图标尺寸(ui: &Ui, 上下文: &绘制上下文<'_>, 行高: f32) -> f32 {
    let _ = ui;
    上下文.外观.图标尺寸.clamp(10.0, 行高 - 2.0)
}

/// 场景树里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 显示行 {
    /// 实体标识。
    pub 标识: 实体标识,
    /// 显示名。
    pub 名称: String,
    /// 缩进层级。
    pub 深度: usize,
    /// 是否有子实体（决定是否显示折叠箭头）。
    pub 有子实体: bool,
    /// 是否处于"编辑器隐藏"状态。
    pub 隐藏: bool,
    /// 是否被锁定。
    pub 锁定: bool,
    /// 是否已被展开。
    pub 展开: bool,
}

/// 把场景文档摊平成场景树显示行。
///
/// 折叠的子树会被跳过，因此返回的行数可能小于实体总数。
/// 这也是这个函数必须独立出来的原因：它是场景树的核心逻辑，
/// 用纯函数表达才能不依赖 egui 直接测试。
pub fn 构建显示行(
    文档: &场景文档,
    折叠: &BTreeSet<scene_doc::document::实体标识>,
) -> Vec<显示行> {
    let mut 结果 = Vec::new();
    // 祖先栈：深度优先遍历是前序的，深度回到某一层时栈自动截断。
    let mut 祖先: Vec<scene_doc::document::实体标识> = Vec::new();
    for (标识, 深度) in 文档.深度优先遍历() {
        祖先.truncate(深度);
        let Some(实体) = 文档.取(标识) else {
            continue;
        };
        // 任一祖先被折叠 → 该节点及整棵子树都不产生显示行。
        // 注意这里**不修改文档**：实体、父子关系、ID 全部原样保留。
        if 祖先.iter().any(|祖先| 折叠.contains(祖先)) {
            continue;
        }
        let 有子实体 = !文档.子实体列表(Some(标识)).is_empty();
        结果.push(显示行 {
            标识,
            名称: 实体.名称.clone(),
            深度,
            有子实体,
            隐藏: 实体.编辑器隐藏,
            锁定: 实体.编辑器锁定,
            展开: 有子实体 && !折叠.contains(&标识),
        });
        祖先.push(标识);
    }
    结果
}

/// 按名称过滤显示行，保留匹配项及其所有祖先。
///
/// 只保留匹配项本身会让结果看起来"孤零零挂在根上"，
/// 用户无法判断它在层级里的位置，因此祖先必须一起保留。
pub fn 按名称过滤(行列表: &[显示行], 关键字: &str) -> Vec<显示行> {
    let 关键字 = 关键字.trim();
    if 关键字.is_empty() {
        return 行列表.to_vec();
    }

    // 先按深度栈求出每一行的祖先链。
    let mut 保留 = vec![false; 行列表.len()];
    let mut 栈: Vec<usize> = Vec::new();
    for (下标, 行) in 行列表.iter().enumerate() {
        while 栈.len() > 行.深度 {
            栈.pop();
        }
        let 命中 = 行.名称.contains(关键字);
        if 命中 {
            保留[下标] = true;
            for 祖先 in &栈 {
                保留[*祖先] = true;
            }
        }
        栈.push(下标);
    }

    行列表
        .iter()
        .zip(保留)
        .filter(|(_, 保留)| *保留)
        .map(|(行, _)| 行.clone())
        .collect()
}

/// 绘制场景树面板。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 编辑器 = 上下文.编辑器;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(crate::场景占位名(编辑器, 上下文.语言))
                .strong()
                .color(上下文.配色.文字),
        );
        if 编辑器.有未保存修改() {
            ui.label(RichText::new("●").color(上下文.配色.警告))
                .on_hover_text(上下文.语言.取("has_unsaved_changes"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("＋")
                .on_hover_text(上下文.语言.取("new_child_entity_under_selection"))
                .clicked()
            {
                let 父 = 上下文.主选中();
                上下文.界面.开始新建实体(父);
            }
            if ui
                .small_button("🗑")
                .on_hover_text(上下文.语言.取("delete_selected_entities"))
                .clicked()
                && let Some(目标) = 上下文.主选中()
            {
                上下文.动作.推(UiAction::删除实体(目标));
            }
        });
    });

    ui.separator();

    // 搜索框：直接绑定界面状态里的字段。
    // egui 的 `TextEdit` 需要 `&mut String`，因此不能在闭包里借用
    // `上下文`（那会与外层的 `&mut 绘制上下文` 冲突）。
    // 这里先把字段"借出来"单独用，绘制结束后再放回去。
    let mut 关键字 = std::mem::take(&mut 上下文.界面.实体搜索);
    ui.horizontal(|ui| {
        ui.label(上下文.语言.取("search"));
        ui.add(
            egui::TextEdit::singleline(&mut 关键字)
                .hint_text(上下文.语言.取("entity_name"))
                .desired_width(f32::INFINITY),
        );
    });
    let 有搜索 = !关键字.trim().is_empty();
    上下文.界面.实体搜索 = 关键字.clone();
    if 有搜索 {
        ui.horizontal(|ui| {
            if ui.small_button(上下文.语言.取("clear_search")).clicked() {
                上下文.界面.实体搜索.clear();
            }
            ui.label(
                RichText::new(上下文.语言.取("hierarchy_filter_ancestors_hint"))
                    .small()
                    .color(上下文.配色.次要文字),
            );
        });
    }

    ui.separator();

    上下文.界面.规范折叠(&编辑器.会话.文档);
    let 折叠 = 上下文.界面.折叠集合().clone();
    let 全部行 = 构建显示行(&编辑器.会话.文档, &折叠);
    上下文.界面.场景行快照 = 全部行
        .iter()
        .map(|行| format!("{}:{}", 行.深度, 行.名称))
        .collect();
    let 显示行列表 = 按名称过滤(&全部行, &关键字);

    if 全部行.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new(上下文.语言.取("hierarchy_empty_hint")).color(上下文.配色.次要文字));
        return;
    }
    if 显示行列表.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                上下文
                    .语言
                    .填充("hierarchy_no_name_match", &[&关键字.trim()]),
            )
            .color(上下文.配色.次要文字),
        );
        return;
    }

    let 主选中 = 编辑器.选择.主选中();

    // 每帧重算：行数会随折叠/过滤变化，复用旧值会让证据里出现过期的箭头。
    上下文.界面.场景行几何.clear();

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // 记录滚动视口：只有落在这里面的行才真的可点击。
            // `clip_rect` 就是 egui 实际会用来裁剪的那一块区域。
            上下文.界面.场景视口矩形 = Some(ui.clip_rect());
            for 行 in &显示行列表 {
                绘制行(ui, 上下文, 行, 主选中);
            }
        });
}

/// 绘制一行。
fn 绘制行(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 行: &显示行, 主选中: Option<实体标识>
) {
    // 正在重命名这一行时，显示输入框而不是标签。
    if let Some(文本编辑目标::实体(目标)) = 上下文.界面.文本编辑.clone()
        && 目标 == 行.标识
    {
        绘制重命名输入(ui, 上下文, 行);
        return;
    }

    let 选中 = 上下文.编辑器.选择.包含(行.标识);
    let 行高 = 行高(ui, 上下文);
    let 图标尺寸 = 图标尺寸(ui, 上下文, 行高);
    // 折叠箭头的矩形要**带出闭包**：自动验收必须按箭头的真实中心点击，
    // 否则点偏了会落到名称标签上（那是"选择"，不是"折叠"），
    // 于是"功能坏了"与"落点错了"无法区分。
    let mut 箭头矩形: Option<egui::Rect> = None;
    let mut 类型图标矩形: Option<egui::Rect> = None;
    let 类型 = match 上下文.编辑器.会话.文档.取(行.标识) {
        Some(实体) => 节点类型::判定(实体, 行.有子实体),
        // 拿不到实体时不猜类型，按"有没有子实体"回退到通用类型。
        None => {
            if 行.有子实体 {
                节点类型::容器
            } else {
                节点类型::普通实体
            }
        }
    };

    // 选中背景覆盖**整行**，而不是只覆盖文字。
    //
    // 用 `Frame` 而不是先画矩形再画控件：`Frame` 的背景先于内容绘制，
    // 因此文字与图标永远在背景之上；且它的矩形天然等于整行（含缩进以外的
    // 全部宽度），不需要我们自己预测行高。`set_min_width` 让背景横跨
    // 实际可用宽度，鼠标扫过时不会出现"只有半个身子高亮"的割裂感。
    let 行背景 = if 选中 {
        上下文.配色.强调淡
    } else {
        egui::Color32::TRANSPARENT
    };
    let 整行 = egui::Frame::NONE
        .fill(行背景)
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(0, 1))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(行.深度 as f32 * ui.spacing().indent);

                // ---- ① 折叠区（固定宽度：所有行对齐）----
                //
                // 展开/折叠指示器与节点类型图标是**两个独立元素**：
                // 前者可点、只改折叠状态；后者不可折叠、只表达类型。
                let 指示 = 折叠指示::判定(行.有子实体, 行.展开);
                match 指示.图标() {
                    Some(图标项) => {
                        let 箭头 = 图标方形按钮(
                            ui,
                            上下文,
                            图标项,
                            图标按钮尺寸 {
                                图标: 图标尺寸,
                                宽: 折叠区宽度(),
                                高: 行高,
                            },
                            上下文.配色.次要文字,
                            true,
                        )
                        .on_hover_text(if 行.展开 {
                            上下文.语言.取("collapse_node")
                        } else {
                            上下文.语言.取("expand_node")
                        });
                        箭头矩形 = Some(箭头.rect);
                        if 箭头.clicked() {
                            // 折叠是界面状态：只改 `界面状态.折叠集合`，
                            // 不产生撤销命令、不写场景文档。
                            上下文.界面.切换折叠(行.标识);
                        }
                    }
                    // 叶子节点：不画可点击的箭头，只占住同宽的位置保证对齐。
                    None => {
                        ui.add_space(折叠区宽度());
                    }
                }

                // ---- ② 节点类型图标（固定宽度：名称起始位置不跳动）----
                //
                // 点击它按既有设计**选择节点**（不折叠、不切换任何开关），
                // 与点击名称完全一致：编辑器里"点这一行"就是选中它。
                let 类型色 = if 行.隐藏 {
                    上下文.配色.次要文字
                } else if 选中 {
                    上下文.配色.强调
                } else {
                    上下文.配色.次要文字
                };
                let 类型按钮 = 图标方形按钮(
                    ui,
                    上下文,
                    类型.图标(),
                    图标按钮尺寸 {
                        图标: 图标尺寸,
                        宽: 类型图标区宽度(图标尺寸),
                        高: 行高,
                    },
                    类型色,
                    true,
                )
                .on_hover_text(上下文.语言.取(类型.名称键()));
                类型图标矩形 = Some(类型按钮.rect);
                if 类型按钮.clicked() {
                    推送选择(上下文, ui, 行.标识);
                }

                // ---- ③ 名称 ----
                //
                // 名称文字颜色要能一眼分辨三种状态：
                // 隐藏（灰暗，提醒它不会出现在视口里）、
                // 选中（强调色，与视口高亮一致）、
                // 普通（正常文字色）。三者必须不同，否则用户无法从场景树
                // 判断某个实体当前处于什么状态。
                let 名称文本 = RichText::new(&行.名称).color(if 行.隐藏 {
                    上下文.配色.次要文字
                } else if 选中 {
                    上下文.配色.强调
                } else {
                    上下文.配色.文字
                });

                let 标签 = ui.add(egui::Label::new(名称文本).sense(Sense::click()));

                // ---- 右侧开关（图标，不再是字体里没有的 emoji）----
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let 锁图标 = if 行.锁定 {
                        图标::锁定
                    } else {
                        图标::未锁定
                    };
                    let 锁色 = if 行.锁定 {
                        上下文.配色.警告
                    } else {
                        上下文.配色.次要文字
                    };
                    if 图标方形按钮(
                        ui,
                        上下文,
                        锁图标,
                        图标按钮尺寸 {
                            图标: 图标尺寸,
                            宽: 开关宽度(),
                            高: 行高,
                        },
                        锁色,
                        true,
                    )
                    .on_hover_text(if 行.锁定 {
                        上下文.语言.取("unlock")
                    } else {
                        上下文.语言.取("lock")
                    })
                    .clicked()
                    {
                        切换实体标记(上下文, 行.标识, 标记项::锁定);
                    }

                    let 眼图标 = if 行.隐藏 {
                        图标::隐藏
                    } else {
                        图标::眼睛
                    };
                    if 图标方形按钮(
                        ui,
                        上下文,
                        眼图标,
                        图标按钮尺寸 {
                            图标: 图标尺寸,
                            宽: 开关宽度(),
                            高: 行高,
                        },
                        上下文.配色.次要文字,
                        true,
                    )
                    .on_hover_text(if 行.隐藏 {
                        上下文.语言.取("show")
                    } else {
                        上下文.语言.取("hide")
                    })
                    .clicked()
                    {
                        切换实体标记(上下文, 行.标识, 标记项::隐藏);
                    }
                });

                标签
            })
        });

    // 整行点击选择。
    //
    // `整行` 现在是 `Frame` 的返回：外层 `.response` 是**整行矩形**（用于
    // 记录证据与右键菜单），`.inner` 是水平布局的返回，而它的 `.inner`
    // 才是名称标签的响应（点击/双击走它，保持原有选择与重命名语义）。
    let 行矩形响应 = 整行.response;
    let 响应 = 整行.inner.inner;

    // 记录这一行的真实几何（整行 + 折叠箭头 + 类型图标），
    // 供自动验收按**箭头中心**点击并按类型图标核对。
    //
    // 记录放在这里而不是绘制中：`行矩形响应.rect` 要等整行布局结束
    // 才会有正确宽度，此时所有子控件的矩形也已经确定。
    上下文.界面.场景行几何.push(crate::state::场景行几何 {
        标识: format!("{:?}", 行.标识),
        名称: 行.名称.clone(),
        深度: 行.深度,
        有子实体: 行.有子实体,
        展开: 行.展开,
        行矩形: 行矩形响应.rect,
        箭头矩形,
        节点类型: 类型.代号(),
        图标: 类型.图标().名称(),
        类型图标矩形,
    });

    if 响应.clicked() {
        推送选择(上下文, ui, 行.标识);
    }
    if 响应.double_clicked() {
        上下文.界面.开始重命名(行.标识, 行.名称.clone());
    }

    // 右键菜单挂在整行上：空白区域右键也能出菜单（原本只有名称上能弹）。
    行矩形响应.context_menu(|ui| {
        if ui.button(上下文.语言.取("rename")).clicked() {
            上下文.界面.开始重命名(行.标识, 行.名称.clone());
            ui.close();
        }
        if ui.button(上下文.语言.取("duplicate")).clicked() {
            上下文.动作.推(UiAction::复制实体(行.标识));
            ui.close();
        }
        if ui.button(上下文.语言.取("new_child_entity_here")).clicked() {
            上下文.界面.开始新建实体(Some(行.标识));
            ui.close();
        }
        ui.separator();
        if ui.button(上下文.语言.取("move_up")).clicked() {
            上下文.动作.推(UiAction::执行命令(
                editor_core::command::编辑器命令::上移实体 { 标识: 行.标识 },
            ));
            ui.close();
        }
        if ui.button(上下文.语言.取("move_down")).clicked() {
            上下文.动作.推(UiAction::执行命令(
                editor_core::command::编辑器命令::下移实体 { 标识: 行.标识 },
            ));
            ui.close();
        }
        if ui.button(上下文.语言.取("move_to_top_level")).clicked() {
            上下文.动作.推(UiAction::执行命令(
                editor_core::command::编辑器命令::重设父子 {
                    标识: 行.标识,
                    新父实体: None,
                    新次序: u32::MAX,
                },
            ));
            ui.close();
        }
        ui.separator();
        if ui.button(上下文.语言.取("delete")).clicked() {
            上下文.动作.推(UiAction::删除实体(行.标识));
            ui.close();
        }
    });

    let _ = 主选中;
}

/// 实体上的布尔标记项。
#[derive(Debug, Clone, Copy)]
enum 标记项 {
    隐藏,
    锁定,
}

/// 图标按钮的尺寸三项。
///
/// 打成一个结构体而不是三个 `f32` 参数：三个连续的同类型参数是最容易
/// 传错顺序的一类接口（`宽` 与 `高` 交换不会有编译错误）。
#[derive(Debug, Clone, Copy)]
pub struct 图标按钮尺寸 {
    /// 图标本身的绘制边长。
    pub 图标: f32,
    /// 按钮（可点击区域）宽度。
    pub 宽: f32,
    /// 按钮（可点击区域）高度。
    pub 高: f32,
}

/// 画一个**尺寸固定**的图标按钮。
///
/// # 为什么不用 `图标库::图标按钮`
///
/// 那个方法让 egui 按图标自然尺寸决定按钮大小，于是不同图标的按钮宽度
/// 会差几个像素 —— 用在工具栏没问题，用在**逐行对齐**的场景树里就会让
/// 类型图标列参差不齐。这里用 `min_size` 把每个按钮钉在
/// `(宽, 行高)` 上，保证列对齐。
///
/// # 图标纹理缺失时的回退
///
/// 资源没打进二进制时退回一个 `?` 文字按钮，而不是画一个空白但仍可点的
/// 区域 —— 后者会让用户以为"这里点不动"。回退仍然占同样宽度，
/// 不会破坏对齐。
fn 图标方形按钮(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    图标项: 图标,
    尺寸: 图标按钮尺寸,
    色调: Color32,
    启用: bool,
) -> egui::Response {
    let 图标按钮尺寸 {
        图标: 边长, 宽, 高
    } = 尺寸;
    let 图片 = 上下文.图标.纹理(ui.ctx(), 图标项).map(|纹理| {
        egui::Image::new(egui::load::SizedTexture::new(
            纹理.id(),
            egui::vec2(边长, 边长),
        ))
        .tint(色调)
    });
    let 按钮 = match 图片 {
        Some(图片) => egui::Button::image(图片),
        None => egui::Button::new(egui::RichText::new("?").color(色调).size(边长 * 0.75)),
    };
    ui.add_enabled(
        启用,
        按钮
            .frame(false)
            .min_size(egui::vec2(宽, 高))
            .corner_radius(egui::CornerRadius::same(3)),
    )
}

/// 按当前修饰键推入一条选择动作。
///
/// 抽出来是因为"点击名称"与"点击类型图标"都要产生**完全相同**的选择语义：
/// 单独实现两遍，迟早会出现"点图标是单选、点名称是切换"这种不一致。
fn 推送选择(上下文: &mut 绘制上下文<'_>, ui: &Ui, 标识: 实体标识) {
    let 修饰 = ui.input(|输入| 输入.modifiers);
    上下文.动作.推(UiAction::选择(if 修饰.command {
        选择意图::切换(标识)
    } else if 修饰.shift {
        选择意图::追加(标识)
    } else {
        选择意图::单选(标识)
    }));
}

/// 切换实体的编辑器标记。
///
/// 走"替换实体"这条命令路径而不是特判一个命令，好处是撤销栈、
/// 保存、资源同步全都自动生效，不需要为标记单独开一条通道。
fn 切换实体标记(上下文: &mut 绘制上下文<'_>, 标识: 实体标识, 项: 标记项) {
    let Some(实体) = 上下文.编辑器.会话.文档.取(标识) else {
        return;
    };
    let mut 新实体 = 实体.clone();
    match 项 {
        标记项::隐藏 => 新实体.编辑器隐藏 = !新实体.编辑器隐藏,
        标记项::锁定 => 新实体.编辑器锁定 = !新实体.编辑器锁定,
    }
    上下文.动作.推(UiAction::执行命令(
        editor_core::command::编辑器命令::替换实体 {
            实体: Box::new(新实体),
        },
    ));
}

/// 绘制重命名输入框。
fn 绘制重命名输入(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 行: &显示行) {
    ui.horizontal(|ui| {
        ui.add_space(行.深度 as f32 * ui.spacing().indent + 14.0);
        let 响应 =
            ui.add(egui::TextEdit::singleline(&mut 上下文.界面.文本缓冲).desired_width(160.0));
        响应.request_focus();

        let 按下回车 = 响应.lost_focus() && ui.input(|输入| 输入.key_pressed(egui::Key::Enter));
        let 按下取消 = ui.input(|输入| 输入.key_pressed(egui::Key::Escape));

        if 按下回车 {
            let 新名称 = 上下文.界面.文本缓冲.trim().to_string();
            if !新名称.is_empty() && 新名称 != 行.名称 {
                上下文.动作.推(UiAction::重命名实体 {
                    标识: 行.标识,
                    新名称,
                });
            }
            上下文.界面.结束编辑();
        } else if 按下取消 {
            上下文.界面.结束编辑();
        }
    });
}

/// 场景树的工具函数：把"在某实体下新建"的目标父实体解析出来。
///
/// 选中实体时作为其子实体插入，未选中时作为顶层实体。这个规则很简单，
/// 但它决定了新节点出现在哪里，是用户最容易感到"怎么跑那儿去了"的地方，
/// 因此单独成函数并加测试。
pub fn 解析新建父实体(编辑器: &编辑器状态) -> Option<实体标识> {
    编辑器.选择.主选中()
}

/// 判断某个实体在当前文档中是否可被选择（未被锁定）。
pub fn 可选(文档: &场景文档, 标识: 实体标识) -> bool {
    文档.取(标识).map(|实体| !实体.编辑器锁定).unwrap_or(false)
}

/// 未使用但保留：让 `Color32` 导入在后续扩展中有意义。
#[allow(dead_code)]
fn _颜色占位() -> Color32 {
    Color32::WHITE
}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 造场景() -> 场景文档 {
        let mut 文档 = 场景文档::新建("树测试");
        let 根 = 文档.生成实体("根", None);
        let 甲 = 文档.生成实体("甲", Some(根));
        let _乙 = 文档.生成实体("乙", Some(根));
        let _孙 = 文档.生成实体("甲的孩子", Some(甲));
        文档
    }

    #[test]
    fn 显示行按深度优先顺序并有正确缩进() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档, &BTreeSet::new());
        let 名称: Vec<&str> = 行.iter().map(|行| 行.名称.as_str()).collect();
        assert_eq!(名称, vec!["根", "甲", "甲的孩子", "乙"]);

        let 深度: Vec<usize> = 行.iter().map(|行| 行.深度).collect();
        assert_eq!(深度, vec![0, 1, 2, 1]);
    }

    #[test]
    fn 有子实体标记正确() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档, &BTreeSet::new());
        let 查 = |名称: &str| 行.iter().find(|行| 行.名称 == 名称).unwrap();
        assert!(查("根").有子实体);
        assert!(查("甲").有子实体);
        assert!(!查("乙").有子实体, "乙没有子实体，不应显示折叠箭头");
        assert!(!查("甲的孩子").有子实体);
    }

    #[test]
    fn 折叠父节点后子节点与深层后代都不可见() {
        let 文档 = 造场景();
        let 根 = 文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == "根")
            .unwrap()
            .标识;
        let 折叠 = BTreeSet::from([根]);
        let 行 = 构建显示行(&文档, &折叠);
        let 名称: Vec<&str> = 行.iter().map(|行| 行.名称.as_str()).collect();
        assert_eq!(名称, vec!["根"], "根折叠后只应剩根自己");
    }

    #[test]
    fn 折叠内层节点只隐藏它自己的后代() {
        let 文档 = 造场景();
        let 甲 = 文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == "甲")
            .unwrap()
            .标识;
        let 折叠 = BTreeSet::from([甲]);
        let 行 = 构建显示行(&文档, &折叠);
        let 名称: Vec<&str> = 行.iter().map(|行| 行.名称.as_str()).collect();
        assert_eq!(名称, vec!["根", "甲", "乙"], "甲的孩子应隐藏，乙仍可见");
    }

    #[test]
    fn 展开后按原有顺序恢复后代() {
        let 文档 = 造场景();
        let 根 = 文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == "根")
            .unwrap()
            .标识;
        let 折叠后 = 构建显示行(&文档, &BTreeSet::from([根]));
        assert_eq!(折叠后.len(), 1);
        let 展开后 = 构建显示行(&文档, &BTreeSet::new());
        let 名称: Vec<&str> = 展开后.iter().map(|行| 行.名称.as_str()).collect();
        assert_eq!(名称, vec!["根", "甲", "甲的孩子", "乙"]);
    }

    #[test]
    fn 折叠集合不会改变场景文档() {
        let 文档 = 造场景();
        let 原数量 = 文档.实体表.len();
        let 根 = 文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == "根")
            .unwrap()
            .标识;
        let 甲 = 文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == "甲")
            .unwrap()
            .标识;
        let 原父 = 文档.取(甲).and_then(|实体| 实体.父实体);
        let _ = 构建显示行(&文档, &BTreeSet::from([根]));
        assert_eq!(文档.实体表.len(), 原数量);
        assert_eq!(文档.取(甲).and_then(|实体| 实体.父实体), 原父);
        assert_eq!(
            文档
                .实体表
                .values()
                .find(|实体| 实体.名称 == "甲")
                .unwrap()
                .标识,
            甲
        );
    }

    #[test]
    fn 删除实体后折叠集合可被清理() {
        let mut 文档 = 造场景();
        let 甲 = 文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == "甲")
            .unwrap()
            .标识;
        let mut 折叠 = BTreeSet::from([甲]);
        文档.实体表.remove(&甲);
        // 清理逻辑与界面状态一致：只保留仍存在的实体。
        折叠.retain(|标识| 文档.取(*标识).is_some());
        assert!(折叠.is_empty());
        let 行 = 构建显示行(&文档, &折叠);
        assert!(!行.is_empty());
    }

    #[test]
    fn 折叠切换是纯界面状态且可重复() {
        let mut 界面 = crate::state::界面状态::新建();
        let 文档 = 造场景();
        let 根 = 文档
            .实体表
            .values()
            .find(|实体| 实体.名称 == "根")
            .unwrap()
            .标识;
        assert!(!界面.已折叠(根));
        界面.切换折叠(根);
        assert!(界面.已折叠(根));
        assert_eq!(构建显示行(&文档, 界面.折叠集合()).len(), 1);
        界面.切换折叠(根);
        assert!(!界面.已折叠(根));
        assert_eq!(构建显示行(&文档, 界面.折叠集合()).len(), 4);
    }

    #[test]
    fn 搜索会保留匹配项及其祖先() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档, &BTreeSet::new());

        let 过滤后 = 按名称过滤(&行, "孩子");
        let 名称: Vec<&str> = 过滤后.iter().map(|行| 行.名称.as_str()).collect();
        assert_eq!(
            名称,
            vec!["根", "甲", "甲的孩子"],
            "匹配项 `甲的孩子` 的祖先 `根` 与 `甲` 都必须保留，\
             否则结果在树里没有上下文"
        );
    }

    #[test]
    fn 空关键字返回全部() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档, &BTreeSet::new());
        assert_eq!(按名称过滤(&行, "").len(), 行.len());
        assert_eq!(按名称过滤(&行, "   ").len(), 行.len());
    }

    #[test]
    fn 无匹配时返回空列表() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档, &BTreeSet::new());
        assert!(按名称过滤(&行, "根本不存在的东西").is_empty());
    }

    #[test]
    fn 锁定实体不可被视口选中() {
        let mut 文档 = 造场景();
        let 行 = 构建显示行(&文档, &BTreeSet::new());
        let 目标 = 行[1].标识;
        assert!(可选(&文档, 目标));
        if let Some(实体) = 文档.取可变(目标) {
            实体.编辑器锁定 = true;
        }
        assert!(!可选(&文档, 目标), "锁定后不应可被选中");
    }

    #[test]
    fn 显示行反映隐藏与锁定标记() {
        let mut 文档 = 造场景();
        let 名 = |文档: &场景文档, 名称: &str| {
            文档
                .实体表
                .values()
                .find(|实体| 实体.名称 == 名称)
                .unwrap()
                .标识
        };
        let 甲 = 名(&文档, "甲");
        if let Some(实体) = 文档.取可变(甲) {
            实体.编辑器隐藏 = true;
            实体.编辑器锁定 = true;
        }
        let 行 = 构建显示行(&文档, &BTreeSet::new());
        let 甲行 = 行.iter().find(|行| 行.标识 == 甲).unwrap();
        assert!(甲行.隐藏 && 甲行.锁定);
    }
}
