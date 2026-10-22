//! 资源浏览器面板。
//!
//! # 为什么把"目录树构建"和"绘制"分开
//!
//! 项目资源在磁盘上是**扁平的文件列表**（`Project::资源列表` 返回排序好的
//! 全量列表），而用户需要看到的是**目录树**。这个转换是这个面板里
//! 唯一有实际逻辑的部分，因此做成纯函数，可以脱离 egui 直接测试。

use editor_core::project::{资源条目, 项目描述};
use egui::{Color32, RichText, Sense, Ui};
use scene_doc::path::项目相对路径;
use scene_doc::property::资源类别;

use crate::asset_preview;
use crate::state::{UiAction, 绘制上下文, 资源排序};

/// 资源树节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum 资源节点 {
    /// 目录。
    目录 {
        /// 显示名（不含路径）。
        名称: String,
        /// 完整项目相对路径文本。
        路径: String,
        /// 子节点。
        子项: Vec<资源节点>,
    },
    /// 文件。
    文件 {
        /// 显示名。
        名称: String,
        /// 完整项目相对路径文本。
        路径: String,
        /// 推断出的资源类别。
        类别: 资源类别,
        /// 字节大小。
        大小: u64,
    },
}

impl 资源节点 {
    /// 节点显示名。
    pub fn 名称(&self) -> &str {
        match self {
            资源节点::目录 { 名称, .. } | 资源节点::文件 { 名称, .. } => 名称,
        }
    }

    /// 节点完整路径。
    pub fn 路径(&self) -> &str {
        match self {
            资源节点::目录 { 路径, .. } | 资源节点::文件 { 路径, .. } => 路径,
        }
    }

    /// 递归统计文件数量。
    pub fn 文件数量(&self) -> usize {
        match self {
            资源节点::文件 { .. } => 1,
            资源节点::目录 { 子项, .. } => 子项.iter().map(资源节点::文件数量).sum(),
        }
    }
}

/// 把扁平的资源列表构建成目录树。
pub fn 构建资源树(条目列表: &[资源条目]) -> Vec<资源节点> {
    // 中间表示：路径段 → 子节点表。
    #[derive(Default)]
    struct 中间 {
        子目录: std::collections::BTreeMap<String, 中间>,
        文件: Vec<(String, 资源类别, u64)>,
    }

    let mut 根 = 中间::default();
    for 条目 in 条目列表 {
        let 段列表: Vec<&str> = 条目.路径.文本().split('/').collect();
        if 段列表.is_empty() {
            continue;
        }
        let mut 当前 = &mut 根;
        for 段 in &段列表[..段列表.len() - 1] {
            当前 = 当前.子目录.entry((*段).to_string()).or_default();
        }
        let 文件名 = 段列表[段列表.len() - 1].to_string();
        当前.文件.push((文件名, 条目.类别, 条目.大小));
    }

    fn 转换(节点: 中间, 前缀: &str) -> Vec<资源节点> {
        let mut 结果 = Vec::new();
        for (名称, 子) in 节点.子目录 {
            let 路径 = if 前缀.is_empty() {
                名称.clone()
            } else {
                format!("{前缀}/{名称}")
            };
            结果.push(资源节点::目录 {
                名称,
                路径: 路径.clone(),
                子项: 转换(子, &路径),
            });
        }
        for (名称, 类别, 大小) in 节点.文件 {
            let 路径 = if 前缀.is_empty() {
                名称.clone()
            } else {
                format!("{前缀}/{名称}")
            };
            结果.push(资源节点::文件 {
                名称,
                路径,
                类别,
                大小,
            });
        }
        结果
    }

    // 目录按名排在前，文件按名排在后，与常见文件管理器一致。
    转换(根, "").into_iter().fold(Vec::new(), |mut 累计, 节点| {
        累计.push(节点);
        累计
    })
}

/// 按关键字过滤资源树，保留匹配的文件及其所在目录。
pub fn 过滤资源树(节点列表: &[资源节点], 关键字: &str) -> Vec<资源节点> {
    let 关键字 = 关键字.trim();
    if 关键字.is_empty() {
        return 节点列表.to_vec();
    }

    let mut 结果 = Vec::new();
    for 节点 in 节点列表 {
        match 节点 {
            资源节点::文件 { 名称, .. } => {
                if 名称.contains(关键字) {
                    结果.push(节点.clone());
                }
            }
            资源节点::目录 {
                名称, 路径, 子项
            } => {
                let 过滤后 = 过滤资源树(子项, 关键字);
                // 目录自身名称命中，或子项里有命中，都保留。
                if !过滤后.is_empty() || 名称.contains(关键字) {
                    结果.push(资源节点::目录 {
                        名称: 名称.clone(),
                        路径: 路径.clone(),
                        子项: 过滤后,
                    });
                }
            }
        }
    }
    结果
}

/// 按指定方式排序资源树（递归）。
pub fn 排序资源树(节点列表: &mut [资源节点], 方式: 资源排序) {
    for 节点 in 节点列表.iter_mut() {
        if let 资源节点::目录 { 子项, .. } = 节点 {
            排序资源树(子项, 方式);
        }
    }
    match 方式 {
        资源排序::名称 => 节点列表.sort_by(|左, 右| {
            // 目录始终排在文件之前，这样层级结构不会被排序打散。
            let 左是目录 = matches!(左, 资源节点::目录 { .. });
            let 右是目录 = matches!(右, 资源节点::目录 { .. });
            右是目录
                .cmp(&左是目录)
                .then_with(|| 左.名称().cmp(右.名称()))
        }),
        资源排序::大小 => 节点列表.sort_by(|左, 右| {
            let 左大小 = match 左 {
                资源节点::文件 { 大小, .. } => *大小 as i64,
                资源节点::目录 { .. } => -1,
            };
            let 右大小 = match 右 {
                资源节点::文件 { 大小, .. } => *大小 as i64,
                资源节点::目录 { .. } => -1,
            };
            右大小.cmp(&左大小).then_with(|| 左.名称().cmp(右.名称()))
        }),
        资源排序::类型 => 节点列表.sort_by(|左, 右| {
            let 左键 = match 左 {
                资源节点::文件 { 类别, 名称, .. } => {
                    (0u8, format!("{类别:?}"), 名称.clone())
                }
                资源节点::目录 { 名称, .. } => (1, String::new(), 名称.clone()),
            };
            let 右键 = match 右 {
                资源节点::文件 { 类别, 名称, .. } => {
                    (0u8, format!("{类别:?}"), 名称.clone())
                }
                资源节点::目录 { 名称, .. } => (1, String::new(), 名称.clone()),
            };
            左键.cmp(&右键)
        }),
    }
}

/// 绘制资源浏览器。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let Some(项目) = 上下文.编辑器.项目.as_ref() else {
        ui.add_space(12.0);
        ui.label(
            RichText::new(上下文.语言.取("assets_no_project_hint")).color(上下文.配色.次要文字),
        );
        return;
    };

    // 顶部工具栏
    ui.horizontal(|ui| {
        let mut 关键字 = std::mem::take(&mut 上下文.界面.资源搜索);
        ui.add(
            egui::TextEdit::singleline(&mut 关键字)
                .hint_text(上下文.语言.取("search_assets"))
                .desired_width(140.0),
        );
        上下文.界面.资源搜索 = 关键字;

        egui::ComboBox::from_id_salt("资源排序")
            .selected_text(match 上下文.界面.资源排序 {
                资源排序::名称 => 上下文.语言.取("by_name"),
                资源排序::大小 => 上下文.语言.取("by_size"),
                资源排序::类型 => 上下文.语言.取("by_type"),
            })
            .width(90.0)
            .show_ui(ui, |ui| {
                for (方式, 文本) in [
                    (资源排序::名称, 上下文.语言.取("by_name")),
                    (资源排序::大小, 上下文.语言.取("by_size")),
                    (资源排序::类型, 上下文.语言.取("by_type")),
                ] {
                    if ui
                        .selectable_label(上下文.界面.资源排序 == 方式, 文本)
                        .clicked()
                    {
                        上下文.界面.资源排序 = 方式;
                    }
                }
            });

        if ui
            .small_button(上下文.语言.取("refresh"))
            .on_hover_text(上下文.语言.取("import_assets_hint"))
            .clicked()
        {
            // 「刷新」现在是**真正的重新导入**：扫描磁盘、对比 `imports.ron`、
            // 更新缓存并报告新增/变更/缺失。仅请求重绘的旧实现等于什么也没做。
            上下文.动作.推(UiAction::重新导入资源);
        }
        if ui
            .small_button(上下文.语言.取("check_asset_dependencies"))
            .on_hover_text(上下文.语言.取("check_asset_dependencies_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::检查资源依赖);
        }
        // 引用使用检查：回答"哪些资源没人用、哪些引用已经断了"。
        // 与上面的"检查依赖"分工不同：那只查断链，这里还列出未使用资源。
        if ui
            .small_button(上下文.语言.取("asset_usage_report"))
            .on_hover_text(上下文.语言.取("asset_usage_report_hint"))
            .clicked()
        {
            上下文.动作.推(UiAction::检查资源使用);
        }
    });

    ui.separator();

    // 资源列表 + **场景列表**。
    //
    // # 为什么要把场景并进来
    //
    // `项目描述::资源列表` 只扫描 `assets/`，而场景文档住在 `scenes/`。
    // 用户对"资源浏览器"的期待显然是"项目里能用的东西都在这"，
    // 场景文件当然算资源 —— 它也是本轮预览要覆盖的一类。
    //
    // 合并只发生在这里（界面层），不改 `资源列表` 的语义：
    // 导入清单、依赖检查、使用报告都仍然只针对 `assets/`，
    // 不会因为多显示了几行场景就误报"未导入/未使用"。
    let mut 全部条目 = 项目.资源列表();
    for 场景路径 in 项目.场景列表() {
        if 全部条目.iter().any(|条目| 条目.路径 == 场景路径) {
            continue;
        }
        let 大小 = std::fs::metadata(项目.绝对路径(&场景路径))
            .map(|元数据| 元数据.len())
            .unwrap_or(0);
        全部条目.push(资源条目 {
            路径: 场景路径,
            类别: 资源类别::场景,
            大小,
            是目录: false,
        });
    }
    全部条目.sort_by(|左, 右| 左.路径.cmp(&右.路径));

    let mut 树 = 构建资源树(&全部条目);
    排序资源树(&mut 树, 上下文.界面.资源排序);
    let 树 = 过滤资源树(&树, &上下文.界面.资源搜索);

    // 摘要行
    let 文件总数: usize = 树.iter().map(资源节点::文件数量).sum();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(上下文.语言.填充("asset_files", &[&文件总数]))
                .small()
                .color(上下文.配色.次要文字),
        );
        // 导入状态汇总：只有"需要处理"的数量才值得占位置。
        let 待导入 = 上下文
            .导入状态
            .values()
            .filter(|状态| 状态.需要处理())
            .count();
        if 待导入 > 0 {
            ui.label(
                RichText::new(上下文.语言.填充("import_pending_count", &[&待导入]))
                    .small()
                    .color(上下文.配色.警告),
            )
            .on_hover_text(上下文.语言.取("import_pending_hint"));
        } else if !上下文.导入状态.is_empty() {
            ui.label(
                RichText::new(上下文.语言.取("import_all_uptodate"))
                    .small()
                    .color(上下文.配色.成功),
            );
        }
    });

    // 试听状态条：正在播时给出路径与停止入口。
    // 播放自然结束后由音频系统清空，因此这里不会出现"卡在试听中"的状态。
    if let Some(路径) = 上下文.界面.试听中.clone() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(上下文.语言.填充("audition_now_playing", &[&路径]))
                    .small()
                    .color(上下文.配色.成功),
            );
            if ui.small_button(上下文.语言.取("audition_stop")).clicked() {
                上下文.动作.推(UiAction::停止试听);
            }
        });
    }

    // 预览区。
    //
    // # 为什么不再新建一个"资源预览"面板
    //
    // 预览与资源列表是同一件事的两半：用户选资源就是为了看它。
    // 拆成两个可停靠面板后，用户完全可能把预览关掉/拖到别处，
    // 于是"单击资源看不到东西"这个原始问题会以另一种形式回来。
    // 这里做的是**资源浏览器内部的固定分区**：列表 + 预览，
    // 任何工作区布局下都在一起。
    //
    // 排布方向按可用宽度决定：资源工作区里面板很宽 → 左右分栏；
    // 通用工作区底部只有一条 → 上下分栏。写死一种必然有一边难用。
    let 可用 = ui.available_size();
    let 侧排 = 可用.x >= asset_preview::侧排最小宽度 && 可用.y >= 220.0;
    let 选中 = 上下文.界面.资源选中.clone();
    if 侧排 {
        egui::Panel::right("asset_preview_side")
            .resizable(true)
            .default_size(asset_preview::默认预览宽度)
            .min_size(asset_preview::最小预览宽度)
            .max_size(可用.x * 0.45)
            .show(ui, |ui| {
                绘制预览区(ui, 上下文, 选中.as_ref());
            });
    } else {
        egui::Panel::bottom("asset_preview_bottom")
            .resizable(true)
            .default_size(asset_preview::默认预览高度)
            .min_size(140.0)
            .max_size(可用.y * 0.75)
            .show(ui, |ui| {
                绘制预览区(ui, 上下文, 选中.as_ref());
            });
    }

    if 树.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new(上下文.语言.取("assets_empty_hint")).color(上下文.配色.次要文字));
        return;
    }

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            绘制节点列表(ui, 上下文, &树, 0);
        });
}

/// 绘制资源预览区；没有选中资源时显示可发现性的引导。
fn 绘制预览区(
    ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 选中: Option<&项目相对路径>
) {
    match 选中 {
        Some(路径) => asset_preview::绘制(ui, 上下文, 路径),
        None => {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(上下文.语言.取("asset_preview_title"))
                        .strong()
                        .color(上下文.配色.文字),
                );
            });
            上下文.界面.布局快照.push(crate::state::区域快照 {
                标识: "asset_preview",
                矩形: ui.max_rect(),
            });
            ui.add_space(12.0);
            ui.label(
                RichText::new(上下文.语言.取("asset_preview_idle"))
                    .strong()
                    .color(上下文.配色.次要文字),
            );
            ui.label(
                RichText::new(上下文.语言.取("asset_preview_idle_hint"))
                    .color(上下文.配色.次要文字),
            );
        }
    }
}

/// 递归绘制资源树。
fn 绘制节点列表(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    节点列表: &[资源节点],
    深度: usize,
) {
    for 节点 in 节点列表 {
        match 节点 {
            资源节点::目录 {
                名称, 路径, 子项
            } => {
                let 行 = ui
                    .horizontal(|ui| {
                        ui.add_space(深度 as f32 * 12.0);
                        egui::CollapsingHeader::new(
                            RichText::new(format!("📁 {名称}")).color(上下文.配色.文字),
                        )
                        .default_open(深度 == 0)
                        .id_salt(format!("资源目录-{路径}"))
                        .show(ui, |ui| {
                            绘制节点列表(ui, 上下文, 子项, 深度 + 1);
                        })
                    })
                    .inner;
                // 目录行也进证据：自动验收要先把它点开，才能点到里面的文件。
                // 不记录的话，深目录里的资源在 GUI 上"无法到达"，而这与
                // "预览没实现"在证据上无法区分。
                上下文.界面.资源行几何.push(crate::state::资源行几何 {
                    路径: 路径.clone(),
                    名称: 名称.clone(),
                    行矩形: 行.header_response.rect,
                    名称矩形: Some(行.header_response.rect),
                    选中: false,
                    是目录: true,
                });
                // 目录的展开状态变化本身不需要额外动作（egui 自己保存）。
            }
            资源节点::文件 {
                名称,
                路径,
                类别,
                大小,
            } => {
                绘制文件行(ui, 上下文, 名称, 路径, *类别, *大小, 深度);
            }
        }
    }
}

/// 绘制一个文件行。
fn 绘制文件行(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    名称: &str,
    路径文本: &str,
    类别: 资源类别,
    大小: u64,
    深度: usize,
) {
    let 路径 = match 项目相对路径::解析(路径文本) {
        Ok(路径) => 路径,
        Err(_) => return,
    };

    let 选中 = 上下文.界面.资源选中.as_ref() == Some(&路径);
    let 底色 = if 选中 {
        上下文.配色.选中背景
    } else {
        egui::Color32::TRANSPARENT
    };

    let 行 = egui::Frame::NONE
        .fill(底色)
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            绘制文件行内容(ui, 上下文, &路径, 路径文本, 名称, 类别, 大小, 深度, 选中)
        });

    // 记录真实行矩形与名称矩形：自动验收要按名称中心点单击，
    // 猜坐标在缩进/图标尺寸变化时必然点偏。
    上下文.界面.资源行几何.push(crate::state::资源行几何 {
        路径: 路径文本.to_string(),
        名称: 名称.to_string(),
        行矩形: 行.response.rect,
        名称矩形: 行.inner.名称矩形,
        选中,
        是目录: false,
    });
}

/// 一行的实际内容。返回名称标签的矩形（供证据记录）。
#[allow(clippy::too_many_arguments)]
fn 绘制文件行内容(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    路径: &项目相对路径,
    路径文本: &str,
    名称: &str,
    类别: 资源类别,
    大小: u64,
    深度: usize,
    选中: bool,
) -> 行内矩形 {
    let mut 名称矩形 = None;

    ui.horizontal(|ui| {
        ui.add_space(深度 as f32 * 12.0 + 16.0);

        // 图片显示缩略图，其余显示类别角标。
        if 类别 == 资源类别::图片 {
            if let Some(纹理) = 上下文.缩略图(路径) {
                ui.add(egui::Image::new(纹理).fit_to_exact_size(egui::vec2(16.0, 16.0)));
            } else {
                ui.label(RichText::new("🖼").small());
            }
        } else {
            let 图标 = match 类别 {
                资源类别::音频 => "🔊",
                资源类别::字体 => "🔤",
                资源类别::模型 => "🧊",
                资源类别::材质 => "🎨",
                资源类别::场景 => "🎬",
                资源类别::着色器 => "✨",
                资源类别::图片 => "🖼",
                资源类别::其他 => "📄",
            };
            ui.label(RichText::new(图标).small());
        }

        // 导入状态圆点：绿=新增待导入、黄=已变更、灰=最新、红=缺失。
        //
        // 用颜色 + 悬停文字，而不是在每行写一个词：资源列表要能一口气扫过，
        // 文字标签会把行撑得参差不齐。颜色含义在悬停提示里给出。
        if let Some(状态) = 上下文.导入状态.get(路径文本) {
            let 颜色 = match 状态 {
                editor_core::project::导入状态::新增 => 上下文.配色.成功,
                editor_core::project::导入状态::已变更 => 上下文.配色.警告,
                editor_core::project::导入状态::最新 => 上下文.配色.次要文字,
                editor_core::project::导入状态::缺失 => 上下文.配色.错误,
            };
            let (矩形, 圆点响应) =
                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(矩形.center(), 3.0, 颜色);
            圆点响应.on_hover_text(
                上下文
                    .语言
                    .填充("import_state_tooltip", &[&上下文.语言.取(状态.词条键())]),
            );
        }

        let 文字 = if 选中 {
            RichText::new(名称).color(上下文.配色.文字).strong()
        } else {
            RichText::new(名称).color(上下文.配色.文字)
        };
        let 响应 = ui.add(egui::Label::new(文字).sense(Sense::click_and_drag()));
        名称矩形 = Some(响应.rect);

        响应.clone().on_hover_text(上下文.语言.填充(
            "tooltip_asset_category_and_size",
            &[
                &路径文本,
                &上下文.语言.取(类别.词条键()),
                &人类可读大小(大小),
            ],
        ));

        // 单击 = 选中 + 立刻预览。
        //
        // 这是本轮最核心的交互：用户点一个文件，右边/下面**马上**出现
        // 它的真实内容，不需要先"打开"或记快捷键。选中同时记录在
        // 界面状态里，因此右键菜单、拖放、切换面板都不会把它弄丢。
        if 响应.clicked() {
            上下文.界面.选中资源(路径.clone());
        }

        // 双击：脚本文件送到脚本编辑器；场景文件打开场景；其余资源创建实体。
        if 响应.double_clicked() {
            let 是脚本 = 路径
                .扩展名()
                .map(|扩展| crate::script::脚本语言::按扩展名(&扩展).支持着色())
                .unwrap_or(false);
            if 是脚本 {
                上下文.动作.推(UiAction::打开脚本(路径.clone()));
            } else if 类别 == 资源类别::场景 {
                上下文.动作.推(UiAction::打开场景(路径.clone()));
            } else {
                上下文.动作.推(UiAction::从资源创建实体 {
                    资源: 路径.clone(),
                    父实体: 上下文.主选中(),
                });
            }
        }

        响应.context_menu(|ui| {
            // 音频：试听 / 停止。
            //
            // 放在右键菜单里而不是每行一个按钮：资源列表本来就窄，
            // 每行加两个图标会让文件名被挤到看不见；试听又不是高频操作。
            if 类别 == 资源类别::音频 {
                let 正在播 = 上下文.界面.试听中.as_deref() == Some(路径文本);
                if 正在播 {
                    if ui.button(上下文.语言.取("audition_stop")).clicked() {
                        上下文.动作.推(UiAction::停止试听);
                        ui.close();
                    }
                } else if ui
                    .button(上下文.语言.取("audition_play"))
                    .on_hover_text(上下文.语言.取("audition_play_hint"))
                    .clicked()
                {
                    上下文.动作.推(UiAction::试听资源(路径.clone()));
                    ui.close();
                }
                ui.separator();
            }
            if ui
                .button(上下文.语言.取("asset_find_refs"))
                .on_hover_text(上下文.语言.取("asset_find_refs_hint"))
                .clicked()
            {
                上下文.动作.推(UiAction::请求查找引用(路径.clone()));
                ui.close();
            }
            if ui
                .button(上下文.语言.取("add_this_asset_to_the_scene"))
                .clicked()
            {
                上下文.动作.推(UiAction::从资源创建实体 {
                    资源: 路径.clone(),
                    父实体: 上下文.主选中(),
                });
                ui.close();
            }
            if ui.button(上下文.语言.取("show_in_file_manager")).clicked() {
                上下文.动作.推(UiAction::在资源管理器中显示(路径.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(上下文.语言.取("rename")).clicked() {
                上下文.动作.推(UiAction::请求重命名资源(路径.clone()));
                ui.close();
            }
            if ui.button(上下文.语言.取("delete")).clicked() {
                上下文.动作.推(UiAction::请求删除资源(路径.clone()));
                ui.close();
            }
        });

        // 拖拽：把一个 payload 挂在响应上，供视口/场景树接收。
        // 本期先支持"双击创建"，完整跨面板拖放见路线图。
        let _ = 响应.drag_started();
    });

    行内矩形 { 名称矩形 }
}

/// 行内需要回传的几何。
struct 行内矩形 {
    名称矩形: Option<egui::Rect>,
}

/// 把字节数变成人类可读文本。
pub fn 人类可读大小(字节: u64) -> String {
    const 单位: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut 值 = 字节 as f64;
    let mut 下标 = 0;
    while 值 >= 1024.0 && 下标 + 1 < 单位.len() {
        值 /= 1024.0;
        下标 += 1;
    }
    if 下标 == 0 {
        format!("{字节} {}", 单位[0])
    } else {
        format!("{值:.1} {}", 单位[下标])
    }
}

/// 未使用占位，保持导入完整。
#[allow(dead_code)]
fn _占位(_: &项目描述, _: Color32) {}

#[cfg(test)]
mod 测试 {
    use super::*;

    fn 造条目(路径: &str, 大小: u64) -> 资源条目 {
        let 路径 = 项目相对路径::解析(路径).unwrap();
        let 类别 = 路径
            .扩展名()
            .map(|扩展| 资源类别::由扩展名推断(&扩展))
            .unwrap_or(资源类别::其他);
        资源条目 {
            路径,
            类别,
            大小,
            是目录: false,
        }
    }

    #[test]
    fn 扁平列表能构建出正确的目录树() {
        let 条目 = vec![
            造条目("assets/textures/hero.png", 100),
            造条目("assets/textures/enemy.png", 200),
            造条目("assets/audio/bgm.ogg", 300),
        ];
        let 树 = 构建资源树(&条目);

        // 顶层应当只有 assets 一个目录
        assert_eq!(树.len(), 1);
        let 资源节点::目录 { 名称, 子项, .. } = &树[0] else {
            panic!("顶层应当是目录，实际：{:?}", 树[0]);
        };
        assert_eq!(名称, "assets");

        // assets 下应当有 audio 与 textures 两个子目录
        let 子目录名: Vec<&str> = 子项.iter().map(资源节点::名称).collect();
        assert!(子目录名.contains(&"audio"), "缺少 audio：{子目录名:?}");
        assert!(
            子目录名.contains(&"textures"),
            "缺少 textures：{子目录名:?}"
        );

        assert_eq!(树[0].文件数量(), 3, "应当统计到 3 个文件");
    }

    #[test]
    fn 目录排序把目录排在文件之前() {
        let 条目 = vec![
            造条目("assets/zeta.png", 1),
            造条目("assets/alpha/sub.png", 1),
        ];
        let mut 树 = 构建资源树(&条目);
        排序资源树(&mut 树, 资源排序::名称);

        let 资源节点::目录 { 子项, .. } = &树[0] else {
            panic!("顶层应为目录");
        };
        // alpha 是目录，应当排在 zeta.png 之前
        assert!(
            matches!(&子项[0], 资源节点::目录 { 名称, .. } if 名称 == "alpha"),
            "目录应当排在文件之前，实际顺序：{:?}",
            子项.iter().map(资源节点::名称).collect::<Vec<_>>()
        );
    }

    #[test]
    fn 搜索保留匹配文件及其所在目录() {
        let 条目 = vec![
            造条目("assets/textures/hero.png", 1),
            造条目("assets/textures/enemy.png", 1),
            造条目("assets/audio/bgm.ogg", 1),
        ];
        let 树 = 构建资源树(&条目);
        let 过滤后 = 过滤资源树(&树, "hero");

        // 必须保留 assets/textures 这两层目录，否则结果没有上下文
        let 资源节点::目录 { 子项, .. } = &过滤后[0] else {
            panic!("应当保留顶层目录");
        };
        let 资源节点::目录 {
            名称, 子项: 文件,
        ..
        } = &子项[0]
        else {
            panic!("应当保留 textures 目录");
        };
        assert_eq!(名称, "textures");
        assert_eq!(文件.len(), 1);
        assert_eq!(文件[0].名称(), "hero.png");
    }

    #[test]
    fn 搜索无匹配时返回空() {
        let 树 = 构建资源树(&[造条目("assets/a.png", 1)]);
        assert!(过滤资源树(&树, "不存在").is_empty());
    }

    #[test]
    fn 空关键字返回完整树() {
        let 树 = 构建资源树(&[造条目("assets/a.png", 1)]);
        assert_eq!(过滤资源树(&树, "").len(), 树.len());
    }

    #[test]
    fn 空列表构建出空树() {
        assert!(构建资源树(&[]).is_empty());
    }

    #[test]
    fn 大小显示可读() {
        assert_eq!(人类可读大小(0), "0 B");
        assert_eq!(人类可读大小(512), "512 B");
        assert_eq!(人类可读大小(1024), "1.0 KB");
        assert_eq!(人类可读大小(1536), "1.5 KB");
        assert_eq!(人类可读大小(1024 * 1024), "1.0 MB");
        assert_eq!(人类可读大小(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn 深层嵌套目录能正确构建() {
        let 树 = 构建资源树(&[造条目("assets/a/b/c/d.png", 1)]);
        // 逐层下探到第 5 层
        let mut 当前 = &树[0];
        for 期望名 in ["assets", "a", "b", "c"] {
            let 资源节点::目录 { 名称, 子项, .. } = 当前 else {
                panic!("{期望名} 应当是目录");
            };
            assert_eq!(名称, 期望名);
            当前 = &子项[0];
        }
        assert_eq!(当前.名称(), "d.png");
    }
}
