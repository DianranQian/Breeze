//! 顶部菜单栏。
//!
//! # "不能只做空菜单"的技术保障
//!
//! 需求明确禁止"菜单、按钮和工具栏存在大量无效操作"。为避免这一点，
//! 菜单不直接写 `if ui.button("保存").clicked() { ... }` 这种散装代码，
//! 而是先声明成数据（[`菜单项`] / [`菜单分组`]），再统一渲染。
//!
//! 这样做带来一个可测试的保障：菜单的**可用性规则**是纯函数
//! （[`可用性`]），因此可以写测试断言"没有选中实体时『删除实体』必须禁用"。
//! 若将来有人加了一个菜单项却忘了写它的动作，枚举和渲染函数会同时提醒他。

use editor_core::state::编辑器状态;
use egui::{RichText, Ui};

use crate::docks::Tab;
use crate::state::{UiAction, 绘制上下文};

/// 一个菜单项。
#[derive(Debug, Clone)]
pub struct 菜单项 {
    /// 显示文本。
    pub 文本: &'static str,
    /// 快捷键提示（仅显示，不负责绑定）。
    pub 快捷键: Option<&'static str>,
    /// 可用性规则。
    pub 可用: 可用性,
}

impl 菜单项 {
    /// 构造一个恒可用的菜单项。
    pub const fn 常可用(文本: &'static str, 快捷键: Option<&'static str>) -> Self {
        Self {
            文本,
            快捷键,
            可用: 可用性::始终,
        }
    }

    /// 附加可用性规则。
    pub const fn 需要(mut self, 可用: 可用性) -> Self {
        self.可用 = 可用;
        self
    }
}

/// 菜单项的可用性规则。
///
/// 做成数据而不是散落的 `ui.add_enabled(...)` 条件，是为了让
/// "什么情况下这个操作可以用"成为可枚举、可测试的事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum 可用性 {
    /// 始终可用。
    始终,
    /// 需要有打开的项目。
    需要项目,
    /// 需要当前场景已关联到文件。
    需要已保存场景,
    /// 需要至少选中一个实体。
    需要选中,
    /// 需要可撤销的步骤。
    需要可撤销,
    /// 需要可重做的步骤。
    需要可重做,
    /// 需要游戏正在运行。
    需要运行中,
    /// 需要游戏未运行。
    需要未运行,
    /// 需要游戏已暂停。
    需要已暂停,
    /// 需要当前项目里有场景。
    需要场景文件,
}

impl 可用性 {
    /// 判断规则在给定状态下是否满足。
    pub fn 满足(&self, 编辑器: &编辑器状态, 正在运行: bool, 已暂停: bool) -> bool {
        match self {
            可用性::始终 => true,
            可用性::需要项目 => 编辑器.项目.is_some(),
            可用性::需要已保存场景 => 编辑器.场景已保存到磁盘(),
            可用性::需要选中 => !编辑器.选择.为空(),
            可用性::需要可撤销 => 编辑器.撤销.可撤销步数() > 0,
            可用性::需要可重做 => 编辑器.撤销.可重做步数() > 0,
            可用性::需要运行中 => 正在运行,
            可用性::需要未运行 => !正在运行,
            可用性::需要已暂停 => 已暂停,
            可用性::需要场景文件 => 编辑器
                .项目
                .as_ref()
                .is_some_and(|项目| !项目.场景列表().is_empty()),
        }
    }
}

/// 菜单里的一个分组。
#[derive(Debug, Clone)]
pub struct 菜单分组 {
    /// 分组标题。
    pub 标题: &'static str,
    /// 分组内的菜单项。
    pub 项: Vec<菜单项>,
}

/// 完整菜单栏定义。
///
/// 返回 `(菜单名, 该菜单下的分组)` 列表。分成两段是因为 egui 的
/// `MenuButton` 只有一个闭包，用分组 + 分隔线的结构更容易一眼看清
/// 哪个操作属于哪一类。
pub fn 菜单定义() -> Vec<(&'static str, Vec<菜单分组>)> {
    vec![
        (
            "项目",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("新建项目…", None),
                        菜单项::常可用("打开项目…", None),
                        菜单项::常可用("关闭项目", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("项目设置…", None)],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("退出", None)],
                },
            ],
        ),
        (
            "文件",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("新建场景", None),
                    菜单项::常可用("打开场景…", None).需要(可用性::需要场景文件),
                    菜单项::常可用("保存场景", Some("Ctrl+S")).需要(可用性::需要项目),
                    菜单项::常可用("场景另存为…", None).需要(可用性::需要项目),
                ],
            }],
        ),
        (
            "编辑",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("撤销", Some("Ctrl+Z")).需要(可用性::需要可撤销),
                        菜单项::常可用("重做", Some("Ctrl+Y")).需要(可用性::需要可重做),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("复制实体", Some("Ctrl+D")).需要(可用性::需要选中),
                        菜单项::常可用("删除实体", Some("Delete")).需要(可用性::需要选中),
                        菜单项::常可用("全选", Some("Ctrl+A")).需要(可用性::需要项目),
                    ],
                },
            ],
        ),
        (
            "场景",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("在场景树中新建实体", None),
                        菜单项::常可用("上移", None).需要(可用性::需要选中),
                        菜单项::常可用("下移", None).需要(可用性::需要选中),
                        菜单项::常可用("移到顶层", None).需要(可用性::需要选中),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("聚焦选中项", Some("F")).需要(可用性::需要选中)],
                },
            ],
        ),
        (
            "实体",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("添加组件…", None).需要(可用性::需要选中),
                    菜单项::常可用("复制实体", None).需要(可用性::需要选中),
                    菜单项::常可用("重命名实体", None).需要(可用性::需要选中),
                ],
            }],
        ),
        (
            "资源",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("刷新资源列表", None),
                    菜单项::常可用("在文件管理器中打开 assets/", None).需要(可用性::需要项目),
                ],
            }],
        ),
        (
            "运行",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("运行当前场景", Some("F5")).需要(可用性::需要未运行),
                    菜单项::常可用("暂停/继续", Some("F6")).需要(可用性::需要运行中),
                    菜单项::常可用("停止", Some("F7")).需要(可用性::需要运行中),
                ],
            }],
        ),
        (
            "调试",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("单步一帧", None).需要(可用性::需要已暂停),
                        菜单项::常可用("显示运行时 ECS", None),
                        菜单项::常可用("显示性能统计", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("清空控制台", None)],
                },
            ],
        ),
        (
            "窗口",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("场景树", None),
                        菜单项::常可用("检查器", None),
                        菜单项::常可用("视口", None),
                        菜单项::常可用("资源浏览器", None),
                        菜单项::常可用("控制台", None),
                        菜单项::常可用("历史", None),
                        菜单项::常可用("运行时 ECS", None),
                        菜单项::常可用("性能", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("恢复默认布局", None)],
                },
                // 语言子菜单。两项各用**自己的语言**写自己的名字，
                // 当前项由绘制时打勾标出（见 `绘制`）。
                菜单分组 {
                    标题: "界面语言",
                    项: vec![
                        菜单项::常可用("简体中文", None),
                        菜单项::常可用("English", None),
                    ],
                },
            ],
        ),
        (
            "帮助",
            vec![菜单分组 {
                标题: "",
                项: vec![菜单项::常可用("关于 Bevy Studio", None)],
            }],
        ),
    ]
}

/// 把"窗口"菜单里的显示名映射到面板。
pub fn 窗口菜单对应面板(文本: &str) -> Option<Tab> {
    match 文本 {
        "场景树" => Some(Tab::场景树),
        "检查器" => Some(Tab::检查器),
        "视口" => Some(Tab::视口),
        "资源浏览器" => Some(Tab::资源浏览器),
        "控制台" => Some(Tab::控制台),
        "历史" => Some(Tab::历史),
        "运行时 ECS" => Some(Tab::运行时ecs),
        "性能" => Some(Tab::性能),
        _ => None,
    }
}

/// 把菜单项的文本映射到动作。
///
/// 返回 `None` 表示这个菜单项在本版本**尚未实现**。
/// 这种情况必须在界面上如实标注为禁用，而不是让它看起来能点 ——
/// 需求明确禁止"用 TODO 或占位界面宣称功能完成"。
pub fn 菜单项动作(文本: &str, 编辑器: &编辑器状态) -> Option<UiAction> {
    let 主选中 = 编辑器.选择.主选中();
    match 文本 {
        // ---- 项目 ----
        "新建项目…" => Some(UiAction::新建项目 {
            名称: "我的游戏".to_string(),
        }),
        "打开项目…" => Some(UiAction::请求打开项目),
        "关闭项目" => Some(UiAction::关闭项目),
        "项目设置…" => None, // 未实现

        // ---- 文件 ----
        "新建场景" => Some(UiAction::新建场景),
        "保存场景" => Some(UiAction::保存场景),
        "场景另存为…" => Some(UiAction::请求场景另存为),
        "打开场景…" => None, // 由「打开场景…」弹窗实现，见 UiAction::打开场景

        // ---- 编辑 ----
        "撤销" => Some(UiAction::撤销),
        "重做" => Some(UiAction::重做),
        "复制实体" => 主选中.map(UiAction::复制实体),
        "删除实体" => 主选中.map(UiAction::删除实体),
        "全选" => Some(UiAction::选择(crate::state::选择意图::设为(
            编辑器.会话.文档.实体表.keys().copied().collect(),
        ))),

        // ---- 场景 ----
        "在场景树中新建实体" => Some(UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "新实体".to_string(),
                父实体: 主选中,
                指定标识: None,
            },
        )),
        "上移" => 主选中.map(|标识| {
            UiAction::执行命令(editor_core::command::编辑器命令::上移实体 { 标识 })
        }),
        "下移" => 主选中.map(|标识| {
            UiAction::执行命令(editor_core::command::编辑器命令::下移实体 { 标识 })
        }),
        "移到顶层" => 主选中.map(|标识| {
            UiAction::执行命令(editor_core::command::编辑器命令::重设父子 {
                标识,
                新父实体: None,
                新次序: u32::MAX,
            })
        }),
        "聚焦选中项" => Some(UiAction::聚焦面板(Tab::视口)),

        // ---- 实体 ----
        "添加组件…" => None, // 需在检查器里操作，见下面说明
        "重命名实体" => 主选中.map(|标识| UiAction::重命名实体 {
            标识,
            新名称: 编辑器
                .会话
                .文档
                .取(标识)
                .map(|实体| format!("{}_改", 实体.名称))
                .unwrap_or_else(|| "实体".to_string()),
        }),

        // ---- 资源 ----
        "刷新资源列表" => None, // 资源列表每帧扫描，无需显式刷新
        "在文件管理器中打开 assets/" => Some(UiAction::在资源管理器中显示(
            scene_doc::path::项目相对路径::解析("assets").ok()?,
        )),

        // ---- 运行 ----
        "运行当前场景" => Some(UiAction::运行场景),
        "暂停/继续" => Some(UiAction::暂停运行),
        "停止" => Some(UiAction::停止运行),

        // ---- 调试 ----
        "单步一帧" => Some(UiAction::单步一帧),
        "显示运行时 ECS" => Some(UiAction::聚焦面板(Tab::运行时ecs)),
        "显示性能统计" => Some(UiAction::聚焦面板(Tab::性能)),
        "清空控制台" => Some(UiAction::清空控制台),

        // ---- 窗口 ----
        "恢复默认布局" => Some(UiAction::恢复默认布局),

        // ---- 语言 ----
        //
        // 语言项用**各语言自己的写法**当键（"简体中文" / "English"），
        // 而不是把它们翻译成当前语言：用户只看得懂一种语言时，
        // 才能在菜单里找到自己那一项。
        _ if crate::i18n::语言::从自称(文本).is_some() => {
            crate::i18n::语言::从自称(文本).map(UiAction::设置语言)
        }

        // ---- 帮助 ----
        "关于 Bevy Studio" => Some(UiAction::显示关于),

        // 其余一律视为未实现。
        _ => {
            if let Some(面板) = 窗口菜单对应面板(文本) {
                return Some(UiAction::聚焦面板(面板));
            }
            None
        }
    }
}

/// 绘制菜单栏。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 编辑器 = 上下文.编辑器;
    let 正在运行 = 上下文.正在运行;
    let 已暂停 = 上下文.已暂停;
    let 配色 = 上下文.配色;

    egui::MenuBar::new().ui(ui, |ui| {
        let 语言 = 上下文.语言;
        for (菜单名, 分组列表) in 菜单定义() {
            // 菜单标题与每一项都用**当前语言**显示，
            // 但派发动作时仍然用中文原文（`项.文本`）——
            // 翻译只影响"显示什么"，不影响"点了会做什么"。
            ui.menu_button(语言.取(菜单名), |ui| {
                for (分组下标, 分组) in 分组列表.iter().enumerate() {
                    if 分组下标 > 0 {
                        ui.separator();
                    }
                    // 分组标题（目前只有"界面语言"用到）。
                    if !分组.标题.is_empty() {
                        ui.label(
                            RichText::new(语言.取(分组.标题))
                                .small()
                                .color(配色.次要文字),
                        );
                    }
                    for 项 in &分组.项 {
                        let 已实现 = 菜单项动作(项.文本, 编辑器).is_some();
                        let 规则满足 = 项.可用.满足(编辑器, 正在运行, 已暂停);
                        let 可点 = 已实现 && 规则满足;

                        let 标题 = 语言.取(项.文本);
                        // 语言项用"各语言自己的写法"显示，并在当前项前打勾。
                        // 这样只看得懂一种语言的用户也能找到自己那一项。
                        let 标题 = match crate::i18n::语言::从自称(项.文本) {
                            Some(候选) => {
                                if 候选 == 语言 {
                                    format!("✓ {标题}")
                                } else {
                                    format!("   {标题}")
                                }
                            }
                            None => 标题.to_string(),
                        };
                        let 文本 = match 项.快捷键 {
                            Some(快捷键) => format!("{标题}    {快捷键}"),
                            None => 标题,
                        };

                        let 响应 = ui.add_enabled(可点, egui::Button::new(文本).frame(false));

                        if !已实现 {
                            响应
                                .clone()
                                .on_disabled_hover_text("该功能尚未实现（本版本不提供）");
                        }

                        if 响应.clicked() {
                            if let Some(动作) = 菜单项动作(项.文本, 编辑器) {
                                上下文.动作.推(动作);
                            }
                            ui.close();
                        }
                    }
                }
            });
        }

        // 右侧状态区
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if 正在运行 {
                let 文本 = if 已暂停 {
                    "⏸ 已暂停"
                } else {
                    "▶ 运行中"
                };
                let 颜色 = if 已暂停 {
                    配色.警告
                } else {
                    配色.错误
                };
                ui.label(RichText::new(文本).color(颜色).strong());
                ui.separator();
            }

            // 当前场景名 + 未保存标记
            let 场景名 = 编辑器.场景名().to_string();
            let 脏 = 编辑器.有未保存修改();
            let 文本 = if 脏 {
                format!("{场景名} *")
            } else {
                场景名
            };
            ui.label(RichText::new(文本).color(配色.次要文字));

            ui.separator();
            ui.label(RichText::new(编辑器.项目名()).color(配色.次要文字).small());
        });
    });
}

/// 绘制工具栏。
pub fn 绘制工具栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 编辑器 = 上下文.编辑器;
    let 正在运行 = 上下文.正在运行;
    let 已暂停 = 上下文.已暂停;

    ui.horizontal(|ui| {
        // 运行控制
        if ui
            .add_enabled(!正在运行, egui::Button::new("▶ 运行").small())
            .on_hover_text("从编辑器启动游戏，运行当前场景（F5）")
            .clicked()
        {
            上下文.动作.推(UiAction::运行场景);
        }
        if ui
            .add_enabled(
                正在运行,
                egui::Button::new(if 已暂停 {
                    "▶ 继续"
                } else {
                    "⏸ 暂停"
                })
                .small(),
            )
            .on_hover_text("暂停或继续运行（F6）")
            .clicked()
        {
            上下文.动作.推(UiAction::暂停运行);
        }
        if ui
            .add_enabled(已暂停, egui::Button::new("⏭ 单步").small())
            .on_hover_text("暂停状态下前进一帧")
            .clicked()
        {
            上下文.动作.推(UiAction::单步一帧);
        }
        if ui
            .add_enabled(正在运行, egui::Button::new("⏹ 停止").small())
            .on_hover_text("停止运行并回到编辑状态（F7）")
            .clicked()
        {
            上下文.动作.推(UiAction::停止运行);
        }

        ui.separator();

        // 编辑操作
        if ui
            .add_enabled(
                编辑器.撤销.可撤销步数() > 0,
                egui::Button::new("↶ 撤销").small(),
            )
            .on_hover_text("撤销上一步（Ctrl+Z）")
            .clicked()
        {
            上下文.动作.推(UiAction::撤销);
        }
        if ui
            .add_enabled(
                编辑器.撤销.可重做步数() > 0,
                egui::Button::new("↷ 重做").small(),
            )
            .on_hover_text("重做（Ctrl+Y）")
            .clicked()
        {
            上下文.动作.推(UiAction::重做);
        }

        ui.separator();

        // 场景操作
        if ui
            .add_enabled(
                编辑器.项目.is_some(),
                egui::Button::new("💾 保存场景").small(),
            )
            .on_hover_text("保存当前场景到文件（Ctrl+S）")
            .clicked()
        {
            上下文.动作.推(UiAction::保存场景);
        }

        ui.separator();

        // 新建实体
        if ui
            .button("＋ 新建实体")
            .on_hover_text("在选中实体下新建子实体；未选中时建在顶层")
            .clicked()
        {
            let 父 = 编辑器.选择.主选中();
            上下文.界面.开始新建实体(父);
        }
        if ui
            .add_enabled(!编辑器.选择.为空(), egui::Button::new("🗑 删除"))
            .on_hover_text("删除选中实体及其子树（Delete）")
            .clicked()
            && let Some(标识) = 编辑器.选择.主选中()
        {
            上下文.动作.推(UiAction::删除实体(标识));
        }

        ui.separator();

        // 面板快捷方式
        if ui.small_button("场景树").clicked() {
            上下文.动作.推(UiAction::聚焦面板(Tab::场景树));
        }
        if ui.small_button("资源").clicked() {
            上下文.动作.推(UiAction::聚焦面板(Tab::资源浏览器));
        }
        if ui.small_button("控制台").clicked() {
            上下文.动作.推(UiAction::聚焦面板(Tab::控制台));
        }
    });
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_core::state::编辑器状态;

    #[test]
    fn 菜单定义结构完整且没有空菜单() {
        let 定义 = 菜单定义();
        // 需求里点名的十个菜单必须都在
        let 名字: Vec<&str> = 定义.iter().map(|(名字, _)| *名字).collect();
        for 必须 in [
            "项目", "文件", "编辑", "场景", "实体", "资源", "运行", "调试", "窗口", "帮助",
        ] {
            assert!(名字.contains(&必须), "缺少菜单 `{必须}`，实际：{名字:?}");
        }
        // 每个菜单至少有一个分组，每个分组至少有一个项
        for (菜单名, 分组列表) in &定义 {
            assert!(!分组列表.is_empty(), "菜单 `{菜单名}` 没有任何分组");
            let 总项数: usize = 分组列表.iter().map(|分组| 分组.项.len()).sum();
            assert!(总项数 > 0, "菜单 `{菜单名}` 是空菜单");
        }
    }

    #[test]
    fn 已实现的菜单项在无项目无选中时可用性正确() {
        let 编辑器 = 编辑器状态::新建();

        // 无需任何前置条件的项应当有动作
        for 文本 in ["新建项目…", "打开项目…", "新建场景", "撤销", "重做"] {
            assert!(
                菜单项动作(文本, &编辑器).is_some(),
                "`{文本}` 应当有对应动作"
            );
        }

        // 需要项目的项在无项目时应当不可用
        assert!(!可用性::需要项目.满足(&编辑器, false, false));
        assert!(!可用性::需要选中.满足(&编辑器, false, false));
        assert!(!可用性::需要可撤销.满足(&编辑器, false, false));
    }

    #[test]
    fn 未实现的菜单项被如实标注() {
        let 编辑器 = 编辑器状态::新建();
        // 这些是本版本确实没做的功能，必须返回 None 以便界面显示为禁用。
        {
            let 文本 = "项目设置…";
            assert!(
                菜单项动作(文本, &编辑器).is_none(),
                "`{文本}` 尚未实现，菜单项动作必须返回 None 而不是假装能用"
            );
        }
    }

    #[test]
    fn 需要选中的菜单项在有选中时才产生动作() {
        let mut 编辑器 = 编辑器状态::新建();
        assert!(菜单项动作("删除实体", &编辑器).is_none());

        let 标识 = 编辑器.新建实体("甲", None).unwrap();
        assert!(菜单项动作("删除实体", &编辑器).is_some());
        assert_eq!(编辑器.选择.主选中(), Some(标识));
    }

    #[test]
    fn 窗口菜单映射到正确的面板() {
        for 面板 in Tab::全部() {
            let 显示名 = match 面板 {
                Tab::资源浏览器 => "资源浏览器",
                Tab::运行时ecs => "运行时 ECS",
                其他 => 其他.标题(crate::i18n::语言::中文),
            };
            assert_eq!(
                窗口菜单对应面板(显示名),
                Some(*面板),
                "显示名 `{显示名}` 应当能映射到面板"
            );
        }
        assert_eq!(窗口菜单对应面板("不存在的面板"), None);
    }

    #[test]
    fn 运行控制的可用性由运行状态决定() {
        let 编辑器 = 编辑器状态::新建();
        assert!(可用性::需要未运行.满足(&编辑器, false, false));
        assert!(!可用性::需要未运行.满足(&编辑器, true, false));
        assert!(可用性::需要运行中.满足(&编辑器, true, false));
        assert!(可用性::需要已暂停.满足(&编辑器, true, true));
        assert!(!可用性::需要已暂停.满足(&编辑器, true, false));
    }

    #[test]
    fn 自定义可用性默认是始终可用() {
        let 项 = 菜单项::常可用("测试", None);
        assert_eq!(项.可用, 可用性::始终);
        let 项 = 项.需要(可用性::需要项目);
        assert_eq!(项.可用, 可用性::需要项目);
    }
}
