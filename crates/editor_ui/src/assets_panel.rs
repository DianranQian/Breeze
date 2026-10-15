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

use crate::state::{绘制上下文, 资源排序, UiAction};

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
    转换(根, "")
        .into_iter()
        .fold(Vec::new(), |mut 累计, 节点| {
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
            资源节点::目录 { 名称, 路径, 子项 } => {
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
                资源节点::文件 { 类别, 名称, .. } => (0u8, format!("{类别:?}"), 名称.clone()),
                资源节点::目录 { 名称, .. } => (1, String::new(), 名称.clone()),
            };
            let 右键 = match 右 {
                资源节点::文件 { 类别, 名称, .. } => (0u8, format!("{类别:?}"), 名称.clone()),
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
            RichText::new("尚未打开项目。\n请在「文件 → 打开项目」里选择一个项目目录。")
                .color(上下文.配色.次要文字),
        );
        return;
    };

    // 顶部工具栏
    ui.horizontal(|ui| {
        let mut 关键字 = std::mem::take(&mut 上下文.界面.资源搜索);
        ui.add(
            egui::TextEdit::singleline(&mut 关键字)
                .hint_text("搜索资源")
                .desired_width(140.0),
        );
        上下文.界面.资源搜索 = 关键字;

        egui::ComboBox::from_id_salt("资源排序")
            .selected_text(match 上下文.界面.资源排序 {
                资源排序::名称 => "按名称",
                资源排序::大小 => "按大小",
                资源排序::类型 => "按类型",
            })
            .width(90.0)
            .show_ui(ui, |ui| {
                for (方式, 文本) in [
                    (资源排序::名称, "按名称"),
                    (资源排序::大小, "按大小"),
                    (资源排序::类型, "按类型"),
                ] {
                    if ui
                        .selectable_label(上下文.界面.资源排序 == 方式, 文本)
                        .clicked()
                    {
                        上下文.界面.资源排序 = 方式;
                    }
                }
            });

        if ui.small_button("刷新").clicked() {
            // 资源列表是每次绘制时现场扫描的，因此"刷新"只需要重绘。
            // 这里请求一次重绘，让按钮有即时反馈。
            ui.ctx().request_repaint();
        }
    });

    ui.separator();

    let 全部条目 = 项目.资源列表();
    let mut 树 = 构建资源树(&全部条目);
    排序资源树(&mut 树, 上下文.界面.资源排序);
    let 树 = 过滤资源树(&树, &上下文.界面.资源搜索);

    // 摘要行
    let 文件总数: usize = 树.iter().map(资源节点::文件数量).sum();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} 个资源文件", 文件总数))
                .small()
                .color(上下文.配色.次要文字),
        );
    });

    if 树.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "没有资源。\n把图片、音频等文件放进项目的 assets/ 目录即可被识别。",
            )
            .color(上下文.配色.次要文字),
        );
        return;
    }

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            绘制节点列表(ui, 上下文, &树, 0);
        });
}

/// 递归绘制资源树。
fn 绘制节点列表(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 节点列表: &[资源节点], 深度: usize) {
    for 节点 in 节点列表 {
        match 节点 {
            资源节点::目录 { 名称, 路径, 子项 } => {
                ui.horizontal(|ui| {
                    ui.add_space(深度 as f32 * 12.0);
                    egui::CollapsingHeader::new(
                        RichText::new(format!("📁 {名称}")).color(上下文.配色.文字),
                    )
                    .default_open(深度 == 0)
                    .id_salt(format!("资源目录-{路径}"))
                    .show(ui, |ui| {
                        绘制节点列表(ui, 上下文, 子项, 深度 + 1);
                    });
                });
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

    ui.horizontal(|ui| {
        ui.add_space(深度 as f32 * 12.0 + 16.0);

        // 图片显示缩略图，其余显示类别角标。
        if 类别 == 资源类别::图片 {
            if let Some(纹理) = 上下文.缩略图(&路径) {
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

        let 响应 = ui.add(
            egui::Label::new(RichText::new(名称).color(上下文.配色.文字))
                .sense(Sense::click_and_drag()),
        );

        响应.clone().on_hover_text(format!(
            "{}\n类别：{}\n大小：{}",
            路径文本,
            类别.显示名(),
            人类可读大小(大小)
        ));

        // 双击：场景文件打开场景，图片资源创建精灵。
         if 响应.double_clicked() {
            if 类别 == 资源类别::场景 {
                上下文.动作.推(UiAction::打开场景(路径.clone()));
            } else {
                上下文.动作.推(UiAction::从资源创建实体 {
                    资源: 路径.clone(),
                    父实体: 上下文.主选中(),
                });
            }
        }

        响应.context_menu(|ui| {
            if ui.button("在场景中添加此资源").clicked() {
                上下文.动作.推(UiAction::从资源创建实体 {
                    资源: 路径.clone(),
                    父实体: 上下文.主选中(),
                });
                ui.close();
            }
            if ui.button("在文件管理器中显示").clicked() {
                上下文
                     .动作
                    .推(UiAction::在资源管理器中显示(路径.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("重命名").clicked() {
                上下文.动作.推(UiAction::请求重命名资源(路径.clone()));
                ui.close();
            }
            if ui.button("删除").clicked() {
                上下文.动作.推(UiAction::请求删除资源(路径.clone()));
                ui.close();
            }
        });

        // 拖拽：把一个 payload 挂在响应上，供视口/场景树接收。
        // 本期先支持"双击创建"，完整跨面板拖放见路线图。
        let _ = 响应.drag_started();
    });
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

/// 面板标题。
pub fn 标题() -> &'static str {
    crate::docks::Tab::资源浏览器.标题()
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
        assert!(子目录名.contains(&"textures"), "缺少 textures：{子目录名:?}");

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
        let 资源节点::目录 { 名称, 子项: 文件, .. } = &子项[0] else {
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
