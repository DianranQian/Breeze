//! 中文名：瓦片地图面板
//! 瓦片地图面板：开发者导入图集、定义瓦片集、在图层上刷瓦片。
//!
//! # 工作流（与 Godot 的 TileSet / TileMapLayer 对齐）
//!
//! ```text
//! 资源浏览器选中一张图片
//!         │  「从选中资源导入」
//!         ▼
//! 瓦片集（.tileset.ron）：瓦片尺寸、图集源、双网格规则、变体表
//!         │  地图引用它
//!         ▼
//! 瓦片地图（.tilemap.ron）：图层 + 单元格
//!         │
//!         ▼
//! 面板画布 / 视口预览（加载时按规则烘焙）
//! ```
//!
//! 编辑器**不认识任何具体图片名或材质名**：调色板完全由瓦片集里的
//! 图集源生成，双网格掩码表也写在瓦片集里。这样同一套面板可以服务
//! 任何像素风项目，而不只是示例里的修仙素材。
//!
//! # 与场景的关系
//!
//! 地图与瓦片集是场景之外的文件，修改不进场景撤销栈。面板只在用户点
//! 「绑定到组件」时把两个路径写回 `TileTerrain` 组件，场景因此才变脏。

use std::collections::BTreeMap;

use editor_core::tilemap::{瓦片地图, 瓦片格};
use editor_core::tileset::{图集源, 瓦片集};
use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use scene_doc::document::实体标识;
use scene_doc::property::属性值;

use crate::state::{UiAction, 属性编辑, 瓦片地图待确认, 绘制上下文};
use crate::theme::编辑器配色;

/// 面板入口。
pub fn 绘制(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 配色 = 上下文.配色;
    let 纹理 = 上下文.瓦片图集;
    let 选中 = 上下文.主选中();
    let 选中名 = 选中
        .and_then(|标识| 上下文.编辑器.会话.文档.取(标识))
        .map(|实体| 实体.名称.clone());
    let 组件地图 = 选中.and_then(|标识| 组件文本(上下文, 标识, "地图文件"));
    let 组件集 = 选中.and_then(|标识| 组件文本(上下文, 标识, "瓦片集"));
    let 资源选中 = 上下文
        .界面
        .资源选中
        .as_ref()
        .map(|路径| 路径.文本().to_string());
    let 有项目 = 上下文.有项目();

    let mut 动作列表: Vec<UiAction> = Vec::new();

    // ---- 跟随选择：只在没有未保存修改时自动切换，避免吞掉用户工作 ----
    {
        let 编辑 = &mut 上下文.界面.瓦片地图;
        if let Some(标识) = 选中
            && 编辑.实体 != Some(标识)
            && !编辑.脏
            && !编辑.瓦片集脏
        {
            编辑.实体 = Some(标识);
            编辑.地图文件 = 组件地图.clone().unwrap_or_default();
            编辑.瓦片集文件 = 组件集.clone().unwrap_or_default();
            编辑.地图 = None;
            编辑.瓦片集 = None;
            编辑.错误 = None;
            编辑.请求中 = None;
            编辑.待确认 = None;
            编辑.图层 = 0;
            编辑.笔刷 = None;
            编辑.撤销栈.clear();
            编辑.重做栈.clear();
            if !编辑.地图文件.trim().is_empty() {
                动作列表.push(UiAction::载入瓦片地图 {
                    实体: 标识,
                    地图文件: 编辑.地图文件.clone(),
                });
                编辑.请求中 = Some((标识, 编辑.地图文件.clone()));
            }
        }
    }

    ui.horizontal_wrapped(|ui| {
        ui.heading(语言.取("tilemap_title"));
        match (编辑实体(上下文), &选中名) {
            (Some(_), Some(名称)) => {
                ui.label(RichText::new(名称).color(配色.次要文字));
            }
            _ => {
                ui.label(RichText::new(语言.取("tilemap_entity_none")).color(配色.次要文字));
            }
        }
        let 编辑 = &mut 上下文.界面.瓦片地图;
        if 编辑.脏 {
            ui.label(RichText::new(语言.取("tilemap_unsaved")).color(配色.警告));
        } else if 编辑.地图.is_some() {
            ui.label(RichText::new(语言.取("tilemap_saved")).color(配色.成功));
        }
    });

    if !有项目 {
        ui.label(RichText::new(语言.取("tilemap_no_project")).color(配色.次要文字));
        return;
    }

    // ---- 文件与实体绑定 ----
    {
        let 编辑 = &mut 上下文.界面.瓦片地图;
        egui::Grid::new("tilemap_files")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label(语言.取("tilemap_map_file"));
                ui.add(
                    egui::TextEdit::singleline(&mut 编辑.地图文件)
                        .hint_text(语言.取("tilemap_map_file_hint"))
                        .desired_width(f32::INFINITY),
                );
                ui.end_row();
                ui.label(语言.取("tilemap_tileset_file"));
                ui.add(
                    egui::TextEdit::singleline(&mut 编辑.瓦片集文件)
                        .hint_text(语言.取("tilemap_tileset_file_hint"))
                        .desired_width(f32::INFINITY),
                );
                ui.end_row();
            });

        ui.horizontal_wrapped(|ui| {
            if ui.button(语言.取("tilemap_load")).clicked()
                && let Some(实体) = 编辑.实体
                && !编辑.地图文件.trim().is_empty()
            {
                if 编辑.脏 || 编辑.瓦片集脏 {
                    编辑.待确认 = Some(瓦片地图待确认::载入 {
                        实体,
                        地图文件: 编辑.地图文件.clone(),
                    });
                } else {
                    动作列表.push(UiAction::载入瓦片地图 {
                        实体,
                        地图文件: 编辑.地图文件.clone(),
                    });
                    编辑.请求中 = Some((实体, 编辑.地图文件.clone()));
                }
            }

            if ui
                .add_enabled(编辑.可保存(), egui::Button::new(语言.取("tilemap_save")))
                .clicked()
                && let (Some(实体), Some(地图)) = (编辑.实体, 编辑.地图.as_ref())
            {
                动作列表.push(UiAction::保存瓦片地图 {
                    实体,
                    地图文件: 编辑.地图文件.clone(),
                    文本: 地图.序列化(),
                });
            }

            if ui.button(语言.取("tilemap_create")).clicked() {
                新建地图(编辑, &语言);
            }

            if ui
                .add_enabled(
                    编辑.实体.is_some(),
                    egui::Button::new(语言.取("tilemap_bind")),
                )
                .on_hover_text(语言.取("tilemap_bind_hint"))
                .clicked()
                && let Some(实体) = 编辑.实体
            {
                动作列表.push(UiAction::修改属性(属性编辑 {
                    实体,
                    组件: "TileTerrain".to_string(),
                    属性: "地图文件".to_string(),
                    新值: 属性值::文本(编辑.地图文件.clone()),
                }));
                动作列表.push(UiAction::修改属性(属性编辑 {
                    实体,
                    组件: "TileTerrain".to_string(),
                    属性: "瓦片集".to_string(),
                    新值: 属性值::文本(编辑.瓦片集文件.clone()),
                }));
            }
        });

        if let Some(错误) = &编辑.错误 {
            ui.label(RichText::new(错误).color(配色.错误));
        }
        if let Some((实体, 文件)) = &编辑.请求中 {
            ui.label(RichText::new(format!("{} · {}", 文件, 实体)).color(配色.次要文字));
        }

        // 未保存确认。
        if let Some(待确认) = 编辑.待确认.clone() {
            ui.separator();
            ui.label(RichText::new(语言.取("tilemap_discard_hint")).color(配色.警告));
            ui.horizontal(|ui| {
                if ui.button(语言.取("tilemap_discard_confirm")).clicked() {
                    编辑.待确认 = None;
                    let 瓦片地图待确认::载入 {
                        实体, 地图文件
                    } = 待确认;
                    动作列表.push(UiAction::载入瓦片地图 {
                        实体,
                        地图文件: 地图文件.clone(),
                    });
                    编辑.请求中 = Some((实体, 地图文件));
                }
                if ui.button(语言.取("tilemap_cancel")).clicked() {
                    编辑.待确认 = None;
                }
            });
        }
    }

    绘制瓦片集编辑(ui, 上下文, &mut 动作列表, 资源选中.as_deref(), 配色);

    绘制调色板(ui, 上下文, 配色, 纹理);

    绘制图层(ui, 上下文);

    绘制画布(ui, 上下文, 配色, 纹理);

    for 动作 in 动作列表 {
        上下文.动作.推(动作);
    }
}

/// 当前面板绑定的实体。
fn 编辑实体(上下文: &绘制上下文<'_>) -> Option<实体标识> {
    上下文.界面.瓦片地图.实体
}

/// 读一个组件文本属性。
fn 组件文本(
    上下文: &绘制上下文<'_>, 实体: 实体标识, 属性: &str
) -> Option<String> {
    上下文
        .编辑器
        .会话
        .文档
        .取(实体)?
        .取组件("TileTerrain")?
        .属性
        .get(属性)
        .and_then(属性值::取文本)
        .map(str::to_string)
}

/// 新建一张空地图。路径必须由用户先填好，编辑器不猜目录。
fn 新建地图(编辑: &mut crate::state::瓦片地图编辑, 语言: &crate::i18n::语言) {
    if 编辑.地图文件.trim().is_empty() {
        编辑.错误 = Some(语言.取("tilemap_need_map_path").to_string());
        return;
    }
    if 编辑.瓦片集文件.trim().is_empty() {
        编辑.错误 = Some(语言.取("tilemap_need_tileset_path").to_string());
        return;
    }

    let 宽高 = 编辑.待应用尺寸;
    let mut 地图 = 瓦片地图::新建(宽高.0, 宽高.1);
    地图.瓦片集 = 编辑.瓦片集文件.clone();

    if 编辑.瓦片集.is_none() {
        let 名称 = 文件主名(&编辑.瓦片集文件);
        编辑.瓦片集 = Some(瓦片集::新建(名称));
        编辑.瓦片集脏 = true;
    }

    编辑.地图 = Some(地图);
    编辑.脏 = true;
    编辑.错误 = None;
    编辑.图层 = 0;
    编辑.笔刷 = None;
    编辑.撤销栈.clear();
    编辑.重做栈.clear();
}

/// 文件主名（去掉目录与扩展名）。
fn 文件主名(路径: &str) -> String {
    let 文件 = 路径.rsplit('/').next().unwrap_or(路径);
    文件
        .split('.')
        .next()
        .filter(|名| !名.is_empty())
        .unwrap_or(文件)
        .to_string()
}

// ---------------------------------------------------------------------------
// 瓦片集编辑
// ---------------------------------------------------------------------------

/// 绘制瓦片集编辑器：尺寸、双网格规则、图集源、掩码表。
fn 绘制瓦片集编辑(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    动作列表: &mut Vec<UiAction>,
    资源选中: Option<&str>,
    配色: 编辑器配色,
) {
    let 语言 = 上下文.语言;
    let 编辑 = &mut 上下文.界面.瓦片地图;

    ui.separator();
    egui::CollapsingHeader::new(语言.取("tilemap_tileset"))
        .default_open(true)
        .show(ui, |ui| {
            let Some(集) = 编辑.瓦片集.as_mut() else {
                ui.label(RichText::new(语言.取("tilemap_empty_hint")).color(配色.次要文字));
                return;
            };

            let mut 集脏 = false;

            egui::Grid::new("tilemap_tileset_header")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(语言.取("tilemap_tileset_name"));
                    if ui.text_edit_singleline(&mut 集.名称).changed() {
                        集脏 = true;
                    }
                    ui.end_row();

                    ui.label(语言.取("tilemap_tile_width"));
                    if ui
                        .add(egui::DragValue::new(&mut 集.瓦片宽).range(1..=1024))
                        .changed()
                    {
                        集脏 = true;
                    }
                    ui.end_row();

                    ui.label(语言.取("tilemap_tile_height"));
                    if ui
                        .add(egui::DragValue::new(&mut 集.瓦片高).range(1..=1024))
                        .changed()
                    {
                        集脏 = true;
                    }
                    ui.end_row();

                    ui.label(语言.取("tilemap_margin"));
                    if ui
                        .add(egui::DragValue::new(&mut 集.边距).range(0..=64))
                        .changed()
                    {
                        集脏 = true;
                    }
                    ui.end_row();

                    ui.label(语言.取("tilemap_spacing"));
                    if ui
                        .add(egui::DragValue::new(&mut 集.间距).range(0..=64))
                        .changed()
                    {
                        集脏 = true;
                    }
                    ui.end_row();
                });

            if ui
                .checkbox(&mut 集.双网格.启用, 语言.取("tilemap_dual_grid"))
                .changed()
            {
                集脏 = true;
            }

            ui.horizontal(|ui| {
                ui.label(语言.取("tilemap_corner_bits"));
                for 位 in &mut 集.双网格.角位权 {
                    if ui.add(egui::DragValue::new(位).range(0..=15)).changed() {
                        集脏 = true;
                    }
                }
                if ui.button(语言.取("tilemap_mask_edit")).clicked() {
                    编辑.掩码编辑中 = true;
                    编辑.掩码草稿源 = None;
                    编辑.掩码草稿 = 掩码文本(&集.双网格.掩码表);
                }
            });

            // 变体表。
            let mut 有变体 = 集.双网格.变体.is_some();
            if ui
                .checkbox(&mut 有变体, 语言.取("tilemap_variant"))
                .changed()
            {
                if 有变体 {
                    集.双网格.变体 = Some(editor_core::tileset::变体表::default());
                } else {
                    集.双网格.变体 = None;
                }
                集脏 = true;
            }
            if let Some(变体) = 集.双网格.变体.as_mut() {
                egui::Grid::new("tilemap_variant_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(语言.取("tilemap_variant_image"));
                        if ui.text_edit_singleline(&mut 变体.图片).changed() {
                            集脏 = true;
                        }
                        ui.end_row();
                        ui.label(语言.取("tilemap_variant_cols"));
                        if ui
                            .add(egui::DragValue::new(&mut 变体.列).range(1..=256))
                            .changed()
                        {
                            集脏 = true;
                        }
                        ui.end_row();
                        ui.label(语言.取("tilemap_variant_rows"));
                        if ui
                            .add(egui::DragValue::new(&mut 变体.行).range(1..=256))
                            .changed()
                        {
                            集脏 = true;
                        }
                        ui.end_row();
                    });
            }

            ui.separator();
            ui.label(RichText::new(语言.取("tilemap_sources")).strong());
            let mut 删除源 = None;
            // 先取出每个源生效的掩码表：下面循环里 `源` 是可变借用，
            // 不能再回头调用 `集.源的掩码表()`（那需要整集的不可变借用）。
            let 掩码表们: Vec<[u16; 16]> = (0..集.图集源.len())
                .map(|序号| *集.源的掩码表(序号))
                .collect();
            for 序号 in 0..集.图集源.len() {
                let 源 = &mut 集.图集源[序号];
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}", 序号 + 1));
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut 源.名称)
                                    .hint_text(语言.取("tilemap_source_name"))
                                    .desired_width(100.0),
                            )
                            .changed()
                        {
                            集脏 = true;
                        }
                        if ui
                            .checkbox(&mut 源.双网格, 语言.取("tilemap_source_dual"))
                            .changed()
                        {
                            集脏 = true;
                        }
                        ui.label(语言.取("tilemap_source_order"));
                        if ui
                            .add(egui::DragValue::new(&mut 源.绘制顺序).range(-99..=99))
                            .changed()
                        {
                            集脏 = true;
                        }
                        if ui.button(语言.取("tilemap_source_remove")).clicked() {
                            删除源 = Some(序号);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(语言.取("tilemap_source_image"));
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut 源.图片)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed()
                        {
                            集脏 = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(语言.取("tilemap_source_variant_row"));
                        let mut 有行 = 源.变体行.is_some();
                        if ui.checkbox(&mut 有行, "").changed() {
                            源.变体行 = if 有行 { Some(0) } else { None };
                            集脏 = true;
                        }
                        if let Some(行) = 源.变体行.as_mut()
                            && ui.add(egui::DragValue::new(行).range(0..=255)).changed()
                        {
                            集脏 = true;
                        }
                        if ui.button(语言.取("tilemap_mask_edit")).clicked() {
                            编辑.掩码编辑中 = true;
                            编辑.掩码草稿源 = Some(序号);
                            编辑.掩码草稿 = 掩码文本(&掩码表们[序号]);
                        }
                    });
                });
            }

            if let Some(序号) = 删除源 {
                集.图集源.remove(序号);
                if 编辑.笔刷.map(|格| 格.源 as usize) == Some(序号) {
                    编辑.笔刷 = None;
                }
                集脏 = true;
            }

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        资源选中.is_some(),
                        egui::Button::new(语言.取("tilemap_import_source")),
                    )
                    .on_hover_text(语言.取("tilemap_import_hint"))
                    .clicked()
                    && let Some(图片) = 资源选中
                {
                    let 名称 = if 编辑.导入源名称.trim().is_empty() {
                        文件主名(图片)
                    } else {
                        编辑.导入源名称.trim().to_string()
                    };
                    let 顺序 = 集.图集源.len() as i32;
                    let 双网格 = 集.双网格.启用;
                    集.图集源.push(图集源 {
                        名称,
                        图片: 图片.to_string(),
                        列: 0,
                        行: 0,
                        双网格,
                        绘制顺序: 顺序,
                        变体行: None,
                        掩码表: None,
                    });
                    集脏 = true;
                }
                ui.add(
                    egui::TextEdit::singleline(&mut 编辑.导入源名称)
                        .hint_text(语言.取("tilemap_source_name"))
                        .desired_width(120.0),
                );
                if 资源选中.is_none() {
                    ui.label(
                        RichText::new(语言.取("tilemap_import_no_selection")).color(配色.次要文字),
                    );
                }
            });

            // 掩码表草稿编辑器。
            if 编辑.掩码编辑中 {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    let 目标 = match 编辑.掩码草稿源 {
                        Some(序号) => 集
                            .图集源
                            .get(序号)
                            .map(|源| 源.显示名())
                            .unwrap_or_default(),
                        None => 语言.取("tilemap_dual_grid").to_string(),
                    };
                    ui.label(format!("{} · {}", 语言.取("tilemap_mask_table"), 目标));
                    ui.add(egui::TextEdit::singleline(&mut 编辑.掩码草稿).desired_width(360.0));
                    if ui.button(语言.取("tilemap_mask_apply")).clicked() {
                        match 解析掩码(&编辑.掩码草稿) {
                            Ok(表) => {
                                match 编辑.掩码草稿源 {
                                    Some(序号) => {
                                        if let Some(源) = 集.图集源.get_mut(序号) {
                                            源.掩码表 = Some(表);
                                        }
                                    }
                                    None => 集.双网格.掩码表 = 表,
                                }
                                编辑.掩码编辑中 = false;
                                集脏 = true;
                            }
                            Err(原因) => 编辑.错误 = Some(原因),
                        }
                    }
                    if ui.button(语言.取("tilemap_cancel")).clicked() {
                        编辑.掩码编辑中 = false;
                    }
                    if 编辑.掩码草稿源.is_some()
                        && ui.button(语言.取("tilemap_mask_use_global")).clicked()
                        && let Some(序号) = 编辑.掩码草稿源
                        && let Some(源) = 集.图集源.get_mut(序号)
                    {
                        源.掩码表 = None;
                        编辑.掩码编辑中 = false;
                        集脏 = true;
                    }
                });
            }

            if 集脏 {
                编辑.瓦片集脏 = true;
            }

            ui.horizontal(|ui| {
                let 可保存 =
                    编辑.瓦片集.is_some() && 编辑.瓦片集脏 && !编辑.瓦片集文件.trim().is_empty();
                if ui
                    .add_enabled(可保存, egui::Button::new(语言.取("tilemap_save_tileset")))
                    .clicked()
                    && let Some(集) = 编辑.瓦片集.as_ref()
                {
                    动作列表.push(UiAction::保存瓦片集 {
                        瓦片集文件: 编辑.瓦片集文件.clone(),
                        文本: 集.序列化(),
                    });
                }
                if 编辑.瓦片集脏 {
                    ui.label(RichText::new(语言.取("tilemap_unsaved")).color(配色.警告));
                }
            });
        });
}

/// 掩码表 → 逗号分隔文本。
fn 掩码文本(表: &[u16; 16]) -> String {
    表.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// 解析 16 项掩码表。
fn 解析掩码(文本: &str) -> Result<[u16; 16], String> {
    let 项: Vec<&str> = 文本
        .split(|字: char| 字 == ',' || 字.is_whitespace())
        .filter(|项| !项.is_empty())
        .collect();
    if 项.len() != 16 {
        return Err(format!("掩码表需要 16 项，当前 {} 项", 项.len()));
    }
    let mut 表 = [0u16; 16];
    for (序号, 项) in 项.iter().enumerate() {
        表[序号] = 项
            .parse::<u16>()
            .map_err(|_| format!("掩码表第 {序号} 项 `{项}` 不是 0..65535 的整数"))?;
    }
    Ok(表)
}

// ---------------------------------------------------------------------------
// 调色板
// ---------------------------------------------------------------------------

/// 绘制调色板：每个图集源的每一张瓦片都可点选。
fn 绘制调色板(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    配色: 编辑器配色,
    纹理: &BTreeMap<String, egui::TextureHandle>,
) {
    let 语言 = 上下文.语言;
    let 编辑 = &mut 上下文.界面.瓦片地图;
    let Some(集) = 编辑.瓦片集.as_ref() else {
        return;
    };

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(语言.取("tilemap_palette")).strong());
        if ui
            .selectable_label(编辑.笔刷.is_none(), 语言.取("tilemap_eraser"))
            .clicked()
        {
            编辑.笔刷 = None;
        }
    });

    let mut 新笔刷 = 编辑.笔刷;
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (源序号, 源) in 集.图集源.iter().enumerate() {
                let 图 = 纹理.get(&源.图片);
                let (列, 行) = 图集网格(集, 源, 图);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(源.显示名()).color(配色.次要文字).small());
                    for 瓦片序号 in 0..列.saturating_mul(行) {
                        let 选中 =
                            新笔刷 == Some(瓦片格::新建(源序号 as u16, 瓦片序号 as u16));
                        let (矩形, 响应) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                        let 目标 = 矩形.shrink(1.0);
                        画瓦片(ui.painter(), 图, 集, 源, 瓦片序号, 目标, 源序号);
                        if 选中 {
                            ui.painter().rect_stroke(
                                矩形,
                                2.0,
                                egui::Stroke::new(2.0, 配色.强调),
                                egui::StrokeKind::Inside,
                            );
                        } else if 响应.hovered() {
                            ui.painter().rect_stroke(
                                矩形,
                                2.0,
                                egui::Stroke::new(1.0, 配色.边框强),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if 响应.clicked() {
                            新笔刷 = Some(瓦片格::新建(源序号 as u16, 瓦片序号 as u16));
                        }
                    }
                });
            }
        });
    编辑.笔刷 = 新笔刷;
}

/// 图集源的实际列数与行数（优先用瓦片集里的显式值，否则按纹理尺寸算）。
fn 图集网格(集: &瓦片集, 源: &图集源, 图: Option<&egui::TextureHandle>) -> (u32, u32) {
    let 尺寸 = 图.map(|图| 图.size_vec2()).unwrap_or_default();
    let 列 = 源.计算列数(尺寸.x.max(1.0) as u32, 集.瓦片宽, 集.边距, 集.间距);
    let 行 = 源.计算行数(尺寸.y.max(1.0) as u32, 集.瓦片高, 集.边距, 集.间距);
    (列.max(1), 行.max(1))
}

/// 画一张瓦片；没有纹理时退回按源编号上色。
fn 画瓦片(
    painter: &egui::Painter,
    图: Option<&egui::TextureHandle>,
    集: &瓦片集,
    源: &图集源,
    瓦片序号: u32,
    目标: Rect,
    源序号: usize,
) {
    if let Some(图) = 图
        && let Some(uv) = 瓦片uv(图, 集, 源, 瓦片序号)
    {
        painter.image(图.id(), 目标, uv, Color32::WHITE);
    } else {
        painter.rect_filled(目标, 0.0, 源颜色(源序号));
    }
}

/// 一张瓦片在整张图集纹理里的 UV 矩形。
fn 瓦片uv(
    图: &egui::TextureHandle, 集: &瓦片集, 源: &图集源, 瓦片序号: u32
) -> Option<Rect> {
    let 尺寸 = 图.size_vec2();
    if 尺寸.x <= 0.0 || 尺寸.y <= 0.0 {
        return None;
    }
    let 列 = 源.计算列数(尺寸.x as u32, 集.瓦片宽, 集.边距, 集.间距);
    let 行 = 源.计算行数(尺寸.y as u32, 集.瓦片高, 集.边距, 集.间距);
    if 列 == 0 || 行 == 0 || 瓦片序号 >= 列 * 行 {
        return None;
    }
    let 列号 = 瓦片序号 % 列;
    let 行号 = 瓦片序号 / 列;
    let x0 = (集.边距 + 列号 * (集.瓦片宽 + 集.间距)) as f32 / 尺寸.x;
    let y0 = (集.边距 + 行号 * (集.瓦片高 + 集.间距)) as f32 / 尺寸.y;
    let x1 = x0 + 集.瓦片宽 as f32 / 尺寸.x;
    let y1 = y0 + 集.瓦片高 as f32 / 尺寸.y;
    Some(Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)))
}

/// 没有纹理时的源颜色。
fn 源颜色(序号: usize) -> Color32 {
    const 色: [Color32; 8] = [
        Color32::from_rgb(94, 129, 84),
        Color32::from_rgb(70, 105, 130),
        Color32::from_rgb(160, 140, 92),
        Color32::from_rgb(120, 100, 140),
        Color32::from_rgb(150, 96, 84),
        Color32::from_rgb(96, 132, 132),
        Color32::from_rgb(132, 120, 72),
        Color32::from_rgb(100, 100, 110),
    ];
    色[序号 % 色.len()]
}

// ---------------------------------------------------------------------------
// 图层
// ---------------------------------------------------------------------------

/// 图层操作，先收集再执行，避免在遍历时改 `Vec`。
enum 图层操作 {
    选择(usize),
    上移(usize),
    下移(usize),
    删除(usize),
    新增(String, bool),
}

/// 绘制图层列表与尺寸设置。
fn 绘制图层(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>) {
    let 语言 = 上下文.语言;
    let 编辑 = &mut 上下文.界面.瓦片地图;
    let Some(地图) = 编辑.地图.as_mut() else {
        return;
    };

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(语言.取("tilemap_layers")).strong());
        ui.label(语言.取("tilemap_size"));
        ui.add(egui::DragValue::new(&mut 编辑.待应用尺寸.0).range(1..=1024));
        ui.add(egui::DragValue::new(&mut 编辑.待应用尺寸.1).range(1..=1024));
        if ui.button(语言.取("tilemap_apply_size")).clicked() {
            match 地图.调整尺寸(编辑.待应用尺寸.0, 编辑.待应用尺寸.1) {
                Ok(()) => {
                    编辑.脏 = true;
                    编辑.错误 = None;
                    if 编辑.图层 >= 地图.图层.len() {
                        编辑.图层 = 0;
                    }
                }
                Err(原因) => 编辑.错误 = Some(原因),
            }
        }
    });

    let mut 操作: Option<图层操作> = None;
    for (序号, 层) in 地图.图层.iter().enumerate() {
        ui.horizontal(|ui| {
            let 标签 = format!(
                "{} {}{}",
                序号 + 1,
                层.名称,
                if 层.双网格 {
                    语言.取("tilemap_layer_dual_suffix")
                } else {
                    ""
                }
            );
            if ui.selectable_label(编辑.图层 == 序号, 标签).clicked() {
                操作 = Some(图层操作::选择(序号));
            }
            if ui
                .small_button("↑")
                .on_hover_text(语言.取("tilemap_layer_up"))
                .clicked()
            {
                操作 = Some(图层操作::上移(序号));
            }
            if ui
                .small_button("↓")
                .on_hover_text(语言.取("tilemap_layer_down"))
                .clicked()
            {
                操作 = Some(图层操作::下移(序号));
            }
            if ui
                .add_enabled(地图.图层.len() > 1, egui::Button::new("×").small())
                .on_hover_text(语言.取("tilemap_layer_delete"))
                .clicked()
            {
                操作 = Some(图层操作::删除(序号));
            }
        });
    }

    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut 编辑.新图层名)
                .hint_text(语言.取("tilemap_layer_name"))
                .desired_width(120.0),
        );
        ui.checkbox(&mut 编辑.新图层双网格, 语言.取("tilemap_layer_dual"));
        if ui.button(语言.取("tilemap_layer_add")).clicked() {
            let 名称 = if 编辑.新图层名.trim().is_empty() {
                语言.填充("tilemap_layer_default", &[&(地图.图层.len() + 1)])
            } else {
                编辑.新图层名.trim().to_string()
            };
            操作 = Some(图层操作::新增(名称, 编辑.新图层双网格));
        }
    });

    if let Some(操作) = 操作 {
        match 操作 {
            图层操作::选择(序号) => 编辑.图层 = 序号,
            图层操作::上移(序号) => {
                if 地图.移动图层(序号, -1) {
                    编辑.图层 = 序号.saturating_sub(1);
                    编辑.脏 = true;
                }
            }
            图层操作::下移(序号) => {
                if 地图.移动图层(序号, 1) {
                    编辑.图层 = 序号 + 1;
                    编辑.脏 = true;
                }
            }
            图层操作::删除(序号) => {
                if 地图.删除图层(序号) {
                    编辑.脏 = true;
                    if 编辑.图层 >= 地图.图层.len() {
                        编辑.图层 = 0;
                    }
                }
            }
            图层操作::新增(名称, 双网格) => match 地图.新增图层(名称, 双网格)
            {
                Ok(序号) => {
                    编辑.图层 = 序号;
                    编辑.脏 = true;
                    编辑.新图层名.clear();
                }
                Err(原因) => 编辑.错误 = Some(原因),
            },
        }
    }

    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                !编辑.撤销栈.is_empty(),
                egui::Button::new(语言.取("tilemap_undo")),
            )
            .clicked()
        {
            编辑.撤销();
        }
        if ui
            .add_enabled(
                !编辑.重做栈.is_empty(),
                egui::Button::new(语言.取("tilemap_redo")),
            )
            .clicked()
        {
            编辑.重做();
        }
        ui.label(语言.取("tilemap_brush_radius"));
        ui.add(egui::DragValue::new(&mut 编辑.笔刷半径).range(0..=16));
        ui.label(语言.取("tilemap_zoom"));
        ui.add(egui::DragValue::new(&mut 编辑.缩放).range(4.0..=64.0));
        ui.checkbox(&mut 编辑.显示网格, 语言.取("tilemap_show_grid"));
    });
}

// ---------------------------------------------------------------------------
// 画布
// ---------------------------------------------------------------------------

/// 画布交互结果。
struct 画布交互 {
    点击: Option<(usize, usize)>,
    擦除: bool,
    按住: bool,
}

/// 绘制地图画布：双网格图层按四角掩码合成，普通图层直接贴瓦片。
fn 绘制画布(
    ui: &mut Ui,
    上下文: &mut 绘制上下文<'_>,
    配色: 编辑器配色,
    纹理: &BTreeMap<String, egui::TextureHandle>,
) {
    let 编辑 = &mut 上下文.界面.瓦片地图;
    let (Some(地图), Some(集)) = (编辑.地图.as_ref(), 编辑.瓦片集.as_ref()) else {
        return;
    };

    // 闭包里只读地图/瓦片集，交互结果带出来后再改编辑状态：
    // 这样不会在 `&地图`/`&集` 还活着时又要 `&mut 编辑`。
    let 缩放 = 编辑.缩放.clamp(4.0, 64.0);
    let 显示网格 = 编辑.显示网格;

    ui.separator();
    let 交互 = egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let 尺寸 = vec2(地图.宽 as f32 * 缩放, 地图.高 as f32 * 缩放);
            let (画布, 响应) = ui.allocate_exact_size(尺寸, Sense::click_and_drag());
            let 画布 = Rect::from_min_size(画布.min, 尺寸);
            let painter = ui.painter_at(画布);
            painter.rect_filled(画布, 0.0, 配色.视口背景);

            for 层 in &地图.图层 {
                if 层.双网格 && 集.双网格.启用 {
                    画双网格层(&painter, 地图, 集, 层, 画布, 缩放, 纹理, 配色);
                } else {
                    for y in 0..地图.高 {
                        for x in 0..地图.宽 {
                            let Some(格) = 层.取格(x, y) else {
                                continue;
                            };
                            let Some(源) = 集.图集源.get(格.源 as usize) else {
                                continue;
                            };
                            let 目标 = Rect::from_min_size(
                                pos2(画布.left() + x as f32 * 缩放, 画布.top() + y as f32 * 缩放),
                                vec2(缩放, 缩放),
                            );
                            画瓦片(
                                &painter,
                                纹理.get(&源.图片),
                                集,
                                源,
                                格.瓦片 as u32,
                                目标,
                                格.源 as usize,
                            );
                        }
                    }
                }
            }

            if 显示网格 {
                for x in 0..=地图.宽 {
                    let x = 画布.left() + x as f32 * 缩放;
                    painter.line_segment(
                        [pos2(x, 画布.top()), pos2(x, 画布.bottom())],
                        egui::Stroke::new(1.0, 配色.网格次.gamma_multiply(0.5)),
                    );
                }
                for y in 0..=地图.高 {
                    let y = 画布.top() + y as f32 * 缩放;
                    painter.line_segment(
                        [pos2(画布.left(), y), pos2(画布.right(), y)],
                        egui::Stroke::new(1.0, 配色.网格次.gamma_multiply(0.5)),
                    );
                }
            }

            let 擦除 = 响应.secondary_clicked() || 响应.dragged_by(egui::PointerButton::Secondary);
            let 按住 = 响应.clicked()
                || 响应.secondary_clicked()
                || 响应.dragged()
                || 响应.dragged_by(egui::PointerButton::Secondary);
            let mut 点击 = None;
            if 按住
                && let Some(位置) = 响应.hover_pos()
                && 画布.contains(位置)
            {
                let x = ((位置.x - 画布.left()) / 缩放).floor() as i32;
                let y = ((位置.y - 画布.top()) / 缩放).floor() as i32;
                if x >= 0 && y >= 0 && (x as usize) < 地图.宽 && (y as usize) < 地图.高 {
                    点击 = Some((x as usize, y as usize));
                }
            }

            if let Some((x, y)) = 点击 {
                let 目标 = Rect::from_min_size(
                    pos2(画布.left() + x as f32 * 缩放, 画布.top() + y as f32 * 缩放),
                    vec2(缩放, 缩放),
                );
                painter.rect_stroke(
                    目标,
                    0.0,
                    egui::Stroke::new(2.0, 配色.强调),
                    egui::StrokeKind::Inside,
                );
            }

            画布交互 {
                点击, 擦除, 按住
            }
        })
        .inner;

    if let Some((x, y)) = 交互.点击 {
        编辑.开始绘制();
        if 交互.擦除 {
            编辑.应用笔刷值(x as i32, y as i32, None);
        } else {
            编辑.应用笔刷(x as i32, y as i32);
        }
    }
    if !交互.按住 {
        编辑.结束绘制();
    }
}

/// 画一个双网格图层。
#[allow(clippy::too_many_arguments)]
fn 画双网格层(
    painter: &egui::Painter,
    地图: &瓦片地图,
    集: &瓦片集,
    层: &editor_core::tilemap::地图图层,
    画布: Rect,
    缩放: f32,
    纹理: &BTreeMap<String, egui::TextureHandle>,
    配色: 编辑器配色,
) {
    for vy in 0..=地图.高 as i32 {
        for vx in 0..=地图.宽 as i32 {
            let 四角 = [(vx - 1, vy - 1), (vx, vy - 1), (vx, vy), (vx - 1, vy)];
            // 只测试真正出现在四角里的源：不在四角的源掩码必然是 0。
            let mut 源列表: Vec<u16> = Vec::new();
            for (x, y) in 四角 {
                let x = x.clamp(0, 地图.宽 as i32 - 1) as usize;
                let y = y.clamp(0, 地图.高 as i32 - 1) as usize;
                if let Some(格) = 层.取格(x, y)
                    && !源列表.contains(&格.源)
                {
                    源列表.push(格.源);
                }
            }
            源列表.sort_by_key(|源| {
                (
                    集.图集源
                        .get(*源 as usize)
                        .map(|源| 源.绘制顺序)
                        .unwrap_or(0),
                    *源,
                )
            });

            for 源序号 in 源列表 {
                let Some(源) = 集.图集源.get(源序号 as usize) else {
                    continue;
                };
                if !源.双网格 {
                    continue;
                }
                let mut mask = 0u8;
                for (位, (x, y)) in 集.双网格.角位权.iter().zip(四角) {
                    let x = x.clamp(0, 地图.宽 as i32 - 1) as usize;
                    let y = y.clamp(0, 地图.高 as i32 - 1) as usize;
                    if 层.取格(x, y).map(|格| 格.源) == Some(源序号) {
                        mask |= 位;
                    }
                }
                if mask == 0 {
                    continue;
                }
                let 瓦片序号 = 集.源的掩码表(源序号 as usize)[mask as usize] as u32;
                let 目标 = Rect::from_min_size(
                    pos2(
                        画布.left() + (vx as f32 - 0.5) * 缩放,
                        画布.top() + (vy as f32 - 0.5) * 缩放,
                    ),
                    vec2(缩放, 缩放),
                );
                画瓦片(
                    painter,
                    纹理.get(&源.图片),
                    集,
                    源,
                    瓦片序号,
                    目标,
                    源序号 as usize,
                );
                if mask == 15 && 源.变体行.is_some() && 集.双网格.变体.is_some() {
                    painter.rect_stroke(
                        目标,
                        0.0,
                        egui::Stroke::new(1.0, 配色.强调淡),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
    }
}
