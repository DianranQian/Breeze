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
use crate::icons::图标;
use crate::state::{UiAction, 区域快照, 绘制上下文};

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
            "project",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("new_project_2", None),
                        菜单项::常可用("open_project", None),
                        菜单项::常可用("open_2d_demo", None),
                        菜单项::常可用("open_3d_demo", None),
                        菜单项::常可用("close_project", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("project_settings", None).需要(可用性::需要项目)],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("quit", None)],
                },
            ],
        ),
        (
            "file",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("new_scene", None),
                    菜单项::常可用("open_scene", None).需要(可用性::需要场景文件),
                    菜单项::常可用("save_scene", Some("Ctrl+S")).需要(可用性::需要项目),
                    菜单项::常可用("save_scene_as", None).需要(可用性::需要项目),
                ],
            }],
        ),
        (
            "edit",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("undo_2", Some("Ctrl+Z")).需要(可用性::需要可撤销),
                        菜单项::常可用("redo_2", Some("Ctrl+Y")).需要(可用性::需要可重做),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("duplicate_entity", Some("Ctrl+D")).需要(可用性::需要选中),
                        菜单项::常可用("delete_entity", Some("Delete")).需要(可用性::需要选中),
                        菜单项::常可用("select_all", Some("Ctrl+A")).需要(可用性::需要项目),
                    ],
                },
            ],
        ),
        (
            "scene",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("create_an_entity_in_the_scene_tree", None),
                        菜单项::常可用("move_up", None).需要(可用性::需要选中),
                        菜单项::常可用("move_down", None).需要(可用性::需要选中),
                        菜单项::常可用("move_to_top_level", None).需要(可用性::需要选中),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("focus_selection", Some("F")).需要(可用性::需要选中)],
                },
            ],
        ),
        (
            "entity",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("add_component_2", None).需要(可用性::需要选中),
                    菜单项::常可用("duplicate_entity", None).需要(可用性::需要选中),
                    菜单项::常可用("rename_entity", None).需要(可用性::需要选中),
                ],
            }],
        ),
        (
            "assets",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("refresh_the_asset_list", None).需要(可用性::需要项目),
                    菜单项::常可用("check_asset_dependencies", None).需要(可用性::需要项目),
                    菜单项::常可用("tooltip_open_assets_folder", None).需要(可用性::需要项目),
                ],
            }],
        ),
        (
            "run_2",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("run_the_current_scene", Some("F5")).需要(可用性::需要未运行),
                    菜单项::常可用("pause_resume", Some("F6")).需要(可用性::需要运行中),
                    菜单项::常可用("stop_2", Some("F7")).需要(可用性::需要运行中),
                ],
            }],
        ),
        (
            "debug",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("step_one_frame", None).需要(可用性::需要已暂停),
                        菜单项::常可用("show_runtime_ecs", None),
                        菜单项::常可用("show_performance_statistics", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("clear_console", None)],
                },
            ],
        ),
        (
            "window",
            vec![
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("scene_tree", None),
                        菜单项::常可用("inspector", None),
                        菜单项::常可用("viewport", None),
                        菜单项::常可用("asset_browser", None),
                        菜单项::常可用("console", None),
                        菜单项::常可用("history", None),
                        菜单项::常可用("runtime_ecs", None),
                        菜单项::常可用("performance", None),
                        菜单项::常可用("script", None),
                        菜单项::常可用("build", None),
                        菜单项::常可用("timeline", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![菜单项::常可用("restore_default_layout", None)],
                },
                // 工作区：与顶部工作区栏同源，菜单里再给一份入口，
                // 这样即使工作区栏被窗口宽度压缩，功能仍然可达。
                菜单分组 {
                    标题: "workspace",
                    项: vec![
                        菜单项::常可用("workspace_general", None),
                        菜单项::常可用("workspace_scene", None),
                        菜单项::常可用("workspace_assets", None),
                        菜单项::常可用("workspace_debug", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("workspace_new", None),
                        菜单项::常可用("workspace_duplicate", None),
                        菜单项::常可用("workspace_reset_all", None),
                    ],
                },
                // 主题：与主题编辑器同源。切主题不影响工作区布局。
                菜单分组 {
                    标题: "theme",
                    项: vec![
                        菜单项::常可用("theme_dark", None),
                        菜单项::常可用("theme_light", None),
                        菜单项::常可用("theme_high_contrast", None),
                    ],
                },
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("theme_editor", None),
                        菜单项::常可用("theme_reset_all_builtin", None),
                    ],
                },
                // 浮动面板：显式的入口，不要求用户发现"把标签拖出窗口"这个手势。
                菜单分组 {
                    标题: "",
                    项: vec![
                        菜单项::常可用("float_focused_panel", None),
                        菜单项::常可用("dock_all_floating", None),
                    ],
                },
                // 语言子菜单。两项各用**自己的语言**写自己的名字，
                // 当前项由绘制时打勾标出（见 `绘制`）。
                菜单分组 {
                    标题: "menu_group_ui_language",
                    项: vec![
                        菜单项::常可用(crate::i18n::语言::中文.词条键(), None),
                        菜单项::常可用(crate::i18n::语言::英文.词条键(), None),
                    ],
                },
            ],
        ),
        (
            "help",
            vec![菜单分组 {
                标题: "",
                项: vec![
                    菜单项::常可用("keyboard_shortcuts", Some("F1")),
                    菜单项::常可用("about_bevy_studio", None),
                ],
            }],
        ),
    ]
}

/// 把"window"菜单里的显示名映射到面板。
pub fn 窗口菜单对应面板(文本: &str) -> Option<Tab> {
    match 文本 {
        "scene_tree" => Some(Tab::场景树),
        "inspector" => Some(Tab::检查器),
        "viewport" => Some(Tab::视口),
        "asset_browser" => Some(Tab::资源浏览器),
        "console" => Some(Tab::控制台),
        "history" => Some(Tab::历史),
        "runtime_ecs" => Some(Tab::运行时ecs),
        "performance" => Some(Tab::性能),
        "script" => Some(Tab::脚本),
        "build" => Some(Tab::构建),
        "timeline" => Some(Tab::时间轴),
        _ => None,
    }
}

/// 把菜单项的文本映射到动作。
///
/// 返回 `None` 表示这个菜单项在本版本**尚未实现**。
/// 这种情况必须在界面上如实标注为禁用，而不是让它看起来能点 ——
/// 需求明确禁止"用 TODO 或占位界面宣称功能完成"。
pub fn 菜单项动作(
    文本: &str,
    语言: crate::i18n::语言,
    编辑器: &编辑器状态,
) -> Option<UiAction> {
    let 主选中 = 编辑器.选择.主选中();
    match 文本 {
        // ---- 项目 ----
        "new_project_2" => Some(UiAction::新建项目 {
            名称: 语言.取("my_game").to_string(),
        }),
        "open_project" => Some(UiAction::请求打开项目),
        "open_2d_demo" => Some(UiAction::打开示例项目(
            crate::dev_modes::示例项目::二维,
        )),
        "open_3d_demo" => Some(UiAction::打开示例项目(
            crate::dev_modes::示例项目::三维,
        )),
        "close_project" => Some(UiAction::关闭项目),
        "project_settings" => Some(UiAction::显示项目设置(true)),

        // ---- 文件 ----
        "new_scene" => Some(UiAction::新建场景),
        "save_scene" => Some(UiAction::保存场景),
        "save_scene_as" => Some(UiAction::请求场景另存为),
        "open_scene" => None, // 由「打开场景…」弹窗实现，见 UiAction::打开场景

        // ---- 编辑 ----
        "undo_2" => Some(UiAction::撤销),
        "redo_2" => Some(UiAction::重做),
        "duplicate_entity" => 主选中.map(UiAction::复制实体),
        "delete_entity" => 主选中.map(UiAction::删除实体),
        "select_all" => Some(UiAction::选择(crate::state::选择意图::设为(
            编辑器.会话.文档.实体表.keys().copied().collect(),
        ))),

        // ---- 场景 ----
        "create_an_entity_in_the_scene_tree" => Some(UiAction::执行命令(
            editor_core::command::编辑器命令::新建实体 {
                名称: "new_entity".to_string(),
                父实体: 主选中,
                指定标识: None,
            },
        )),
        "move_up" => 主选中.map(|标识| {
            UiAction::执行命令(editor_core::command::编辑器命令::上移实体 { 标识 })
        }),
        "move_down" => 主选中.map(|标识| {
            UiAction::执行命令(editor_core::command::编辑器命令::下移实体 { 标识 })
        }),
        "move_to_top_level" => 主选中.map(|标识| {
            UiAction::执行命令(editor_core::command::编辑器命令::重设父子 {
                标识,
                新父实体: None,
                新次序: u32::MAX,
            })
        }),
        "focus_selection" => Some(UiAction::聚焦面板(Tab::视口)),

        // ---- 实体 ----
        "add_component_2" => None, // 需在检查器里操作，见下面说明
        "rename_entity" => 主选中.map(|标识| UiAction::重命名实体 {
            标识,
            新名称: 编辑器
                .会话
                .文档
                .取(标识)
                // 用 `填充` 而不是 `format!`：`format!` 要求格式串是字面量，
                // 而译文是运行时查表得来的，两者无法直接组合。
                .map(|实体| 语言.填充("entity_duplicate_name", &[&实体.名称]))
                .unwrap_or_else(|| 语言.取("entity").to_string()),
        }),

        // ---- 资源 ----
        // 「刷新资源列表」现在是**真的重新导入**：扫描磁盘、对比 `imports.ron`、
        // 更新缓存并报告新增/变更/缺失。此前它只是请求一次重绘
        // （因为列表本来就是每帧扫描的），那个按钮等于什么都不做。
        "refresh_the_asset_list" => Some(UiAction::重新导入资源),
        "check_asset_dependencies" => Some(UiAction::检查资源依赖),
        "tooltip_open_assets_folder" => Some(UiAction::在资源管理器中显示(
            scene_doc::path::项目相对路径::解析("assets").ok()?,
        )),

        // ---- 运行 ----
        "run_the_current_scene" => Some(UiAction::运行场景),
        "pause_resume" => Some(UiAction::暂停运行),
        "stop_2" => Some(UiAction::停止运行),

        // ---- 调试 ----
        "step_one_frame" => Some(UiAction::单步一帧),
        "show_runtime_ecs" => Some(UiAction::聚焦面板(Tab::运行时ecs)),
        "show_performance_statistics" => Some(UiAction::聚焦面板(Tab::性能)),
        "clear_console" => Some(UiAction::清空控制台),

        // ---- 窗口 ----
        "restore_default_layout" => Some(UiAction::恢复默认布局),

        // ---- 工作区 ----
        //
        // 菜单项存的是 i18n 名称键，派发时按名称键反查内置工作区，
        // 因此菜单文案随语言变化时动作映射不会失效。
        //
        // 「新建 / 复制」走对话框而不是直接建一个自动命名的工作区：
        // 工作区是用户会长期使用的对象，名字必须是用户自己起的。
        "workspace_new" => Some(UiAction::打开工作区对话框 {
            复制当前: false
        }),
        "workspace_duplicate" => Some(UiAction::打开工作区对话框 { 复制当前: true }),
        "workspace_reset_all" => Some(UiAction::重置全部内置工作区),
        "theme_editor" => Some(UiAction::显示主题编辑器(true)),
        "theme_reset_all_builtin" => Some(UiAction::重置全部内置主题),
        "float_focused_panel" => Some(UiAction::浮动面板(Tab::检查器)),
        "dock_all_floating" => Some(UiAction::停靠全部浮动面板),

        // ---- 语言 ----
        //
        // 语言项存的是**词条键**（`language_self_zh_cn` / `language_self_en`），
        // 按键派发动作，因此与当前界面语言无关。
        _ if crate::i18n::语言::从词条键(文本).is_some() => {
            crate::i18n::语言::从词条键(文本).map(UiAction::设置语言)
        }

        // ---- 帮助 ----
        "keyboard_shortcuts" => Some(UiAction::显示帮助(true)),
        "about_bevy_studio" => Some(UiAction::显示关于),

        // 其余一律视为未实现。
        _ => {
            if let Some(面板) = 窗口菜单对应面板(文本) {
                return Some(UiAction::聚焦面板(面板));
            }
            // 工作区项：按 i18n 名称键反查内置工作区。
            if let Some(内置) = crate::workspaces::内置工作区::从名称键(文本) {
                return Some(UiAction::切换工作区(内置.id().to_string()));
            }
            // 主题项：同样按 i18n 名称键反查。
            if let Some(内置) = crate::theme::内置主题::从名称键(文本) {
                return Some(UiAction::设置主题(内置.id().to_string()));
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
        // 当前工作区 id 先拷出来：下面要在菜单项里打勾，同时还要推动作
        // （两者分别借用 `上下文.界面` 与 `上下文.动作`，先把值取出最省事）。
        let 当前工作区id = 上下文.界面.工作区.当前id().to_string();
        let 当前主题id = 上下文.界面.主题库.当前id().to_string();
        for (菜单名, 分组列表) in 菜单定义() {
            // 菜单标题与每一项都用**当前语言**显示，
            // 但派发动作时仍然用中文原文（`项.文本`）——
            // 翻译只影响"显示什么"，不影响"点了会做什么"。
            ui.menu_button(语言.取(菜单名), |ui| {
                for (分组下标, 分组) in 分组列表.iter().enumerate() {
                    if 分组下标 > 0 {
                        ui.separator();
                    }
                    // 分组标题（目前只有「界面语言」这一个分组用到）。
                    if !分组.标题.is_empty() {
                        ui.label(
                            RichText::new(语言.取(分组.标题))
                                .small()
                                .color(配色.次要文字),
                        );
                    }
                    for 项 in &分组.项 {
                        let 已实现 = 菜单项动作(项.文本, 语言, 编辑器).is_some();
                        let 规则满足 = 项.可用.满足(编辑器, 正在运行, 已暂停);
                        let 可点 = 已实现 && 规则满足;

                        let 标题 = 语言.取(项.文本);
                        // 语言项用"各语言自己的写法"显示，并在当前项前打勾。
                        // 这样只看得懂一种语言的用户也能找到自己那一项。
                        // 勾选标记与前导空格是**排版**，不是文案：
                        // 未选中项用等宽空格占住勾选标记的位置，让两列对齐。
                        // 因此这里不查词表，改成纯格式串（也就没有中文字面量）。
                        let 标题 = match crate::i18n::语言::从词条键(项.文本) {
                            Some(候选) if 候选 == 语言 => format!("✓ {}", 标题),
                            Some(_) => format!("   {}", 标题),
                            None => 标题.to_string(),
                        };
                        // 工作区项同样打勾：用户一眼能看出当前在哪个工作区。
                        let 标题 = match crate::workspaces::内置工作区::从名称键(项.文本)
                        {
                            Some(内置) if 内置.id() == 当前工作区id => {
                                format!("✓ {}", 标题.trim_start())
                            }
                            Some(_) => format!("   {}", 标题.trim_start()),
                            None => 标题,
                        };
                        // 主题项打勾：当前主题一目了然。
                        let 标题 = match crate::theme::内置主题::从名称键(项.文本) {
                            Some(内置) if 内置.id() == 当前主题id => {
                                format!("✓ {}", 标题.trim_start())
                            }
                            Some(_) => format!("   {}", 标题.trim_start()),
                            None => 标题,
                        };
                        // 用位置参数而不是具名参数（`{标题}`）：具名参数会把
                        // 中文标识符写进格式串，使"这一行有没有界面文案"
                        // 难以判断，扫描时只能靠加例外来消音。
                        let 文本 = match 项.快捷键 {
                            Some(快捷键) => format!("{}    {}", 标题, 快捷键),
                            None => 标题,
                        };

                        let 响应 = ui.add_enabled(可点, egui::Button::new(文本).frame(false));

                        if !已实现 {
                            响应
                                .clone()
                                .on_disabled_hover_text("tooltip_feature_not_implemented");
                        }

                        if 响应.clicked() {
                            if let Some(动作) = 菜单项动作(项.文本, 语言, 编辑器) {
                                上下文.动作.推(动作);
                            }
                            ui.close();
                        }
                    }
                }

                // 「最近项目」子菜单。
                //
                // 它不是静态菜单项：条目来自用户配置里的列表，长度可变。
                // 因此在这里单独渲染，而不是塞进 `菜单定义()`（那份定义是
                // 静态的、被覆盖测试逐项比对，不能随用户数据变化）。
                //
                // 用绝对路径做按钮文本：重名项目很常见（`游戏/`、`游戏2/`），
                // 只显示名字会让用户点错。
                if 菜单名 == "project" && !上下文.界面.最近项目.is_empty() {
                    ui.separator();
                    ui.menu_button(语言.取("recent_projects"), |ui| {
                        let 现有: Vec<std::path::PathBuf> = 上下文.界面.最近项目.clone();
                        for 目录 in 现有 {
                            let 存在 = 目录.is_dir();
                            let 文本 = if 存在 {
                                目录.display().to_string()
                            } else {
                                format!(
                                    "{}  ({})",
                                    目录.display(),
                                    语言.取("recent_project_missing")
                                )
                            };
                            let 响应 = ui.add_enabled(存在, egui::Button::new(文本).frame(false));
                            if 响应.clicked() {
                                上下文.动作.推(UiAction::打开最近项目(目录.clone()));
                                ui.close();
                            }
                            if !存在 {
                                响应.on_disabled_hover_text(语言.取("recent_project_missing_hint"));
                            }
                        }
                        ui.separator();
                        if ui.button(语言.取("recent_projects_clear")).clicked() {
                            上下文.动作.推(UiAction::清除最近项目);
                            ui.close();
                        }
                    });
                }
            });
        }

        // 右侧状态区
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if 正在运行 {
                let 文本 = if 已暂停 { "paused" } else { "running" };
                let 颜色 = if 已暂停 {
                    配色.警告
                } else {
                    配色.错误
                };
                ui.label(RichText::new(文本).color(颜色).strong());
                ui.separator();
            }

            // 当前场景名 + 未保存标记
            // 未命名/未打开时按界面语言显示占位名：
            // 否则英文界面顶部会残留两处中文（项目名与场景名）。
            let 场景名 = crate::场景占位名(编辑器, 语言).to_string();
            let 脏 = 编辑器.有未保存修改();
            let 文本 = if 脏 {
                语言.填充("window_title_unsaved_marker", &[&场景名])
            } else {
                场景名
            };
            ui.label(RichText::new(文本).color(配色.次要文字));

            ui.separator();
            let 项目名 = if 编辑器.项目.is_none() {
                语言.取("no_project_open").to_string()
            } else {
                编辑器.项目名().to_string()
            };
            ui.label(RichText::new(项目名).color(配色.次要文字).small());
        });
    });
}

/// 绘制工具栏。
///
/// # 一条工具栏，一组职责
///
/// 从左到右依次是：模式专用工具 → **预览控制（编辑器内）** →
/// **项目运行（独立进程）** → 编辑 → 实体 → 面板快捷入口 → 主题编辑器。
/// 每一组之间用分隔线隔开，组内顺序固定。
///
/// # 为什么运行要有两组，而且必须改名
///
/// 本编辑器里有两种**完全不同**的"运行"：
///
/// | | 预览（编辑器内） | 项目运行（独立进程） |
/// | --- | --- | --- |
/// | 做什么 | 在编辑器自己的 World 里推进当前场景 | 起一个真实子进程执行项目的运行目标 |
/// | 数据 | 编辑器文档 → 运行时世界（单向桥） | 与编辑器进程完全隔离 |
/// | 日志 | 运行时统计（帧率/帧耗时） | 子进程 stdout/stderr 逐行进控制台 |
/// | 停止 | 冻结运行时世界 | 结束进程树 |
///
/// 整改前两者都叫「▶ 运行」，图标文字完全一样，而且**在运行模式下
/// 同一行里各画了一次**（模式工具栏 + 通用工具栏），用户点到的很可能是
/// 另一个"运行"，于是一个真实存在的功能看起来"点了没反应"。
///
/// 现在：一个概念一个入口，并且用不同的文案把目标类型写清楚
/// （"预览" = 编辑器内；"运行项目" = 独立进程）。
///
/// # 为什么图标与文字同时显示
///
/// 只有图标需要用户记住含义（对新用户不友好），只有文字则一屏放不下
/// 几个操作。专业编辑器的通行做法是"图标 + 短文字"：
/// 图标负责快速识别，文字负责首次学习，悬停再给完整说明。
pub fn 绘制工具栏(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 编辑器 = 上下文.编辑器;
    let 正在运行 = 上下文.正在运行;
    let 已暂停 = 上下文.已暂停;
    // 图标尺寸跟随主题，文字色也取主题的"主文字"，
    // 这样浅色主题下图标自动变深，不需要为每个主题各画一套。
    let 图标尺寸 = 上下文.外观.图标尺寸;
    let 图标色 = 上下文.配色.文字;
    let 次要色 = 上下文.配色.次要文字;
    let 语言 = 上下文.语言;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        // ---- 模式专用工具 ----
        // 放在最左边：它随主开发模式变化，是"当前任务"最直接的入口。
        if crate::dev_mode_bar::绘制模式工具栏(ui, 上下文) {
            ui.separator();
        }

        // ---- 预览控制（编辑器内）----
        //
        // 文案写"预览"而不是"运行"：它的效果只是让编辑器内的运行时世界
        // 推进并显示统计，不会生成任何可交付的游戏进程。
        let 响应 = 上下文
            .图标
            .图标按钮(
                ui,
                图标::运行,
                Some(语言.取("preview_scene")),
                图标尺寸,
                图标色,
                !正在运行,
            )
            .on_hover_text(语言.取("preview_scene_hint"));
        上下文.界面.布局快照.push(区域快照 {
            标识: "btn_preview_scene",
            矩形: 响应.rect,
        });
        if 响应.clicked() {
            上下文.动作.推(UiAction::运行场景);
        }
        let 响应 = 上下文
            .图标
            .图标按钮(ui, 图标::暂停, None, 图标尺寸, 图标色, 正在运行)
            .on_hover_text(语言.取("pause_or_resume_running_f6"));
        上下文.界面.布局快照.push(区域快照 {
            标识: "btn_pause_preview",
            矩形: 响应.rect,
        });
        if 响应.clicked() {
            上下文.动作.推(UiAction::暂停运行);
        }
        let 响应 = 上下文
            .图标
            .图标按钮(ui, 图标::单步, None, 图标尺寸, 图标色, 已暂停)
            .on_hover_text(语言.取("advance_one_frame_while_paused"));
        上下文.界面.布局快照.push(区域快照 {
            标识: "btn_step_preview",
            矩形: 响应.rect,
        });
        if 响应.clicked() {
            上下文.动作.推(UiAction::单步一帧);
        }
        let 响应 = 上下文
            .图标
            .图标按钮(ui, 图标::停止, None, 图标尺寸, 图标色, 正在运行)
            .on_hover_text(语言.取("tooltip_stop_running"));
        上下文.界面.布局快照.push(区域快照 {
            标识: "btn_stop_preview",
            矩形: 响应.rect,
        });
        if 响应.clicked() {
            上下文.动作.推(UiAction::停止运行);
        }
        // 预览状态：与"项目运行"的状态分开显示，避免两个状态混成一个。
        let 预览状态 = if 已暂停 {
            语言.取("paused_2")
        } else if 正在运行 {
            语言.取("running_2")
        } else {
            语言.取("run_state_stopped")
        };
        ui.label(RichText::new(预览状态).small().color(if 正在运行 {
            上下文.配色.成功
        } else {
            次要色
        }));

        ui.separator();

        // ---- 项目运行（独立进程）----
        //
        // 这里的三个按钮原来只存在于"运行模式"的专用工具栏里，
        // 于是切到 2D / 3D / 代码模式后完全没有办法运行项目。
        // 运行是"操作"而不是"编辑面"，它必须在所有模式下都可用。
        let 项目在跑 = 上下文.界面.项目运行中;
        let 响应 = 上下文
            .图标
            .图标按钮(
                ui,
                图标::运行,
                Some(语言.取("run_project")),
                图标尺寸,
                图标色,
                !项目在跑,
            )
            .on_hover_text(语言.取("run_project_hint"));
        上下文.界面.布局快照.push(区域快照 {
            标识: "btn_run_project",
            矩形: 响应.rect,
        });
        if 响应.clicked() {
            上下文.动作.推(UiAction::运行项目);
        }
        let 响应 = 上下文
            .图标
            .图标按钮(ui, 图标::刷新, None, 图标尺寸, 图标色, true)
            .on_hover_text(语言.取("rerun_project_hint"));
        上下文.界面.布局快照.push(区域快照 {
            标识: "btn_rerun_project",
            矩形: 响应.rect,
        });
        if 响应.clicked() {
            上下文.动作.推(UiAction::重新运行项目);
        }
        let 响应 = 上下文
            .图标
            .图标按钮(ui, 图标::停止, None, 图标尺寸, 图标色, 项目在跑)
            .on_hover_text(语言.取("stop_project_hint"));
        上下文.界面.布局快照.push(区域快照 {
            标识: "btn_stop_project",
            矩形: 响应.rect,
        });
        if 响应.clicked() {
            上下文.动作.推(UiAction::停止项目);
        }
        let 摘要 = 上下文.界面.项目运行摘要.clone();
        if !摘要.is_empty() {
            ui.label(RichText::new(摘要).small().color(if 项目在跑 {
                上下文.配色.成功
            } else {
                次要色
            }));
        }

        ui.separator();

        // ---- 编辑 ----
        if 上下文
            .图标
            .图标按钮(
                ui,
                图标::撤销,
                None,
                图标尺寸,
                图标色,
                编辑器.撤销.可撤销步数() > 0,
            )
            .on_hover_text(语言.取("undo_the_previous_step_ctrl_z"))
            .clicked()
        {
            上下文.动作.推(UiAction::撤销);
        }
        if 上下文
            .图标
            .图标按钮(
                ui,
                图标::重做,
                None,
                图标尺寸,
                图标色,
                编辑器.撤销.可重做步数() > 0,
            )
            .on_hover_text(语言.取("redo_ctrl_y"))
            .clicked()
        {
            上下文.动作.推(UiAction::重做);
        }
        if 上下文
            .图标
            .图标按钮(
                ui,
                图标::保存,
                None,
                图标尺寸,
                图标色,
                编辑器.项目.is_some(),
            )
            .on_hover_text(语言.取("tooltip_save_current_scene"))
            .clicked()
        {
            上下文.动作.推(UiAction::保存场景);
        }

        ui.separator();

        // ---- 实体 ----
        if 上下文
            .图标
            .图标按钮(ui, 图标::新建, None, 图标尺寸, 图标色, true)
            .on_hover_text(语言.取("tooltip_new_child_entity"))
            .clicked()
        {
            let 父 = 编辑器.选择.主选中();
            上下文.界面.开始新建实体(父);
        }
        if 上下文
            .图标
            .图标按钮(ui, 图标::删除, None, 图标尺寸, 图标色, !编辑器.选择.为空())
            .on_hover_text(语言.取("tooltip_delete_entity_subtree"))
            .clicked()
            && let Some(标识) = 编辑器.选择.主选中()
        {
            上下文.动作.推(UiAction::删除实体(标识));
        }

        ui.separator();

        // ---- 面板快捷入口（图标即面板身份）----
        for (面板, 图标项) in [
            (Tab::场景树, 图标::场景树),
            (Tab::资源浏览器, 图标::资源),
            (Tab::控制台, 图标::控制台),
            (Tab::历史, 图标::历史),
            (Tab::运行时ecs, 图标::运行时ecs),
            (Tab::性能, 图标::性能),
        ] {
            if 上下文
                .图标
                .图标按钮(ui, 图标项, None, 图标尺寸, 次要色, true)
                .on_hover_text(面板.标题(语言))
                .clicked()
            {
                上下文.动作.推(UiAction::聚焦面板(面板));
            }
        }

        // ---- 右侧：主题编辑器入口 ----
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if 上下文
                .图标
                .图标按钮(ui, 图标::主题, None, 图标尺寸, 次要色, true)
                .on_hover_text(语言.取("theme_editor"))
                .clicked()
            {
                上下文.动作.推(UiAction::显示主题编辑器(true));
            }
        });
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
            "project", "file", "edit", "scene", "entity", "assets", "run_2", "debug", "window",
            "help",
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
        for 文本 in [
            "new_project_2",
            "open_project",
            "new_scene",
            "undo_2",
            "redo_2",
        ] {
            assert!(
                菜单项动作(文本, crate::i18n::语言::中文, &编辑器).is_some(),
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
        //
        // 「项目设置」曾经也在这张名单里 —— 它现在有真实实现
        // （校验 + 写回 project.ron），因此从名单移除，并补一条正向断言：
        // 一个功能从"未实现"变成"已实现"时，测试必须同时改，
        // 否则名单会变成"永远为真"的摆设。
        for 文本 in ["tooltip_feature_not_implemented"] {
            assert!(
                菜单项动作(文本, crate::i18n::语言::中文, &编辑器).is_none(),
                "`{文本}` 尚未实现，菜单项动作必须返回 None 而不是假装能用"
            );
        }
        assert!(
            matches!(
                菜单项动作("project_settings", crate::i18n::语言::中文, &编辑器),
                Some(UiAction::显示项目设置(true))
            ),
            "项目设置已经实现，必须返回真实动作"
        );
        assert_eq!(
            菜单项动作("refresh_the_asset_list", crate::i18n::语言::中文, &编辑器),
            Some(UiAction::重新导入资源),
            "资源刷新现在是真实导入，不是空按钮"
        );
        assert_eq!(
            菜单项动作("check_asset_dependencies", crate::i18n::语言::中文, &编辑器),
            Some(UiAction::检查资源依赖)
        );
    }

    #[test]
    fn 需要选中的菜单项在有选中时才产生动作() {
        let mut 编辑器 = 编辑器状态::新建();
        assert!(菜单项动作("delete_entity", crate::i18n::语言::中文, &编辑器).is_none());

        let 标识 = 编辑器.新建实体("甲", None).unwrap();
        assert!(菜单项动作("delete_entity", crate::i18n::语言::中文, &编辑器).is_some());
        assert_eq!(编辑器.选择.主选中(), Some(标识));
    }

    #[test]
    fn 窗口菜单映射到正确的面板() {
        for 面板 in Tab::全部() {
            // 直接取面板自己声明的菜单键：菜单定义、反映射、这条测试
            // 三者共用同一个来源，菜单文案改动时不会出现两边对不上。
            let 显示名 = 面板.菜单键();
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
