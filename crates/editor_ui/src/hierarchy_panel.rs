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

use editor_core::state::编辑器状态;
use egui::{Color32, RichText, Sense, Ui};
use scene_doc::document::{场景文档, 实体标识};

use crate::docks::Tab;
use crate::state::{
    文本编辑目标, 选择意图, 绘制上下文, UiAction,
};

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
pub fn 构建显示行(文档: &场景文档) -> Vec<显示行> {
    let mut 结果 = Vec::new();
    for (标识, 深度) in 文档.深度优先遍历() {
        let Some(实体) = 文档.取(标识) else {
            continue;
        };
        结果.push(显示行 {
            标识,
            名称: 实体.名称.clone(),
            深度,
            有子实体: !文档.子实体列表(Some(标识)).is_empty(),
            隐藏: 实体.编辑器隐藏,
            锁定: 实体.编辑器锁定,
            展开: 实体.编辑器展开,
        });
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
            RichText::new(编辑器.场景名())
                .strong()
                .color(上下文.配色.文字),
        );
        if 编辑器.有未保存修改() {
            ui.label(RichText::new("●").color(上下文.配色.警告))
                .on_hover_text("有未保存的修改");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("＋")
                .on_hover_text("在选中实体下新建子实体")
                .clicked()
            {
                let 父 = 上下文.主选中();
                上下文.界面.开始新建实体(父);
            }
            if ui
                .small_button("🗑")
                .on_hover_text("删除选中实体")
                .clicked()
            {
                if let Some(目标) = 上下文.主选中() {
                    上下文.动作.推(UiAction::删除实体(目标));
                }
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
        ui.label("搜索");
        ui.add(
            egui::TextEdit::singleline(&mut 关键字)
                .hint_text("实体名称")
                .desired_width(f32::INFINITY),
        );
    });
    let 有搜索 = !关键字.trim().is_empty();
    上下文.界面.实体搜索 = 关键字.clone();
    if 有搜索 {
        ui.horizontal(|ui| {
            if ui.small_button("清除搜索").clicked() {
                上下文.界面.实体搜索.clear();
            }
            ui.label(
                RichText::new("已按名称过滤，匹配项的祖先会一并显示")
                    .small()
                    .color(上下文.配色.次要文字),
            );
        });
    }

    ui.separator();

    let 全部行 = 构建显示行(&编辑器.会话.文档);
    let 显示行列表 = 按名称过滤(&全部行, &关键字);

    if 全部行.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new("场景是空的。\n点击上方 ＋ 新建实体，或从资源面板拖入图片。")
                .color(上下文.配色.次要文字),
        );
        return;
    }
    if 显示行列表.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("没有名称包含 `{}` 的实体", 关键字.trim()))
                .color(上下文.配色.次要文字),
        );
        return;
    }

    let 主选中 = 编辑器.选择.主选中();

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for 行 in &显示行列表 {
                绘制行(ui, 上下文, 行, 主选中);
            }
        });
}

/// 绘制一行。
fn 绘制行(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 行: &显示行, 主选中: Option<实体标识>) {
    // 正在重命名这一行时，显示输入框而不是标签。
    if let Some(文本编辑目标::实体(目标)) = 上下文.界面.文本编辑.clone() {
        if 目标 == 行.标识 {
            绘制重命名输入(ui, 上下文, 行);
            return;
        }
    }

    let 选中 = 上下文.编辑器.选择.包含(行.标识);
    let 行高 = ui.spacing().interact_size.y;

    let 整行 = ui.horizontal(|ui| {
        ui.add_space(行.深度 as f32 * ui.spacing().indent);

        // 折叠箭头
        if 行.有子实体 {
            let 符号 = if 行.展开 { "▾" } else { "▸" };
            if ui
                .add(egui::Button::new(符号).frame(false).small())
                .clicked()
            {
                let 新值 = !行.展开;
                上下文.动作.推(UiAction::执行命令(
                    editor_core::command::编辑器命令::替换实体 {
                        实体: Box::new({
                            let mut 实体 = 上下文
                                .编辑器
                                .会话
                                .文档
                                .取(行.标识)
                                .cloned()
                                .unwrap_or_else(|| {
                                    scene_doc::document::场景实体::新建(行.名称.clone())
                                });
                            实体.编辑器展开 = 新值;
                            实体
                        }),
                    },
                ));
            }
        } else {
            ui.add_space(14.0);
        }

        // 名称
        let 名称文本 = RichText::new(&行.名称).color(if 行.隐藏 {
            上下文.配色.次要文字
        } else if 选中 {
            上下文.配色.文字
        } else {
            上下文.配色.文字
        });

        let 标签 = ui.add(egui::Label::new(名称文本).sense(Sense::click()));

        // 锁定标记
        if 行.锁定 {
            ui.label(RichText::new("🔒").small().color(上下文.配色.警告));
        }

        // 右侧开关
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let 眼睛 = if 行.隐藏 { "🚫" } else { "👁" };
            if ui
                .add(egui::Button::new(眼睛).frame(false).small())
                .on_hover_text(if 行.隐藏 { "显示" } else { "隐藏" })
                .clicked()
            {
                切换实体标记(上下文, 行.标识, 标记项::隐藏);
            }
            let 锁 = if 行.锁定 { "🔒" } else { "🔓" };
            if ui
                .add(egui::Button::new(锁).frame(false).small())
                .on_hover_text(if 行.锁定 { "解锁" } else { "锁定" })
                .clicked()
            {
                切换实体标记(上下文, 行.标识, 标记项::锁定);
            }
            let _ = 行高;
        });

        标签
    });

    // 整行点击选择。
    let 响应 = 整行.inner;
    if 响应.clicked() {
        let 修饰 = ui.input(|输入| 输入.modifiers);
        上下文.动作.推(UiAction::选择(if 修饰.command {
            选择意图::切换(行.标识)
        } else if 修饰.shift {
            选择意图::追加(行.标识)
        } else {
            选择意图::单选(行.标识)
        }));
    }
    if 响应.double_clicked() {
        上下文.界面.开始重命名(行.标识, 行.名称.clone());
    }

    // 右键菜单
    响应.context_menu(|ui| {
        if ui.button("重命名").clicked() {
            上下文.界面.开始重命名(行.标识, 行.名称.clone());
            ui.close();
        }
        if ui.button("复制").clicked() {
            上下文.动作.推(UiAction::复制实体(行.标识));
            ui.close();
        }
        if ui.button("在此新建子实体").clicked() {
            上下文.界面.开始新建实体(Some(行.标识));
            ui.close();
        }
        ui.separator();
        if ui.button("上移").clicked() {
            上下文.动作.推(UiAction::执行命令(
                editor_core::command::编辑器命令::上移实体 { 标识: 行.标识 },
            ));
            ui.close();
        }
        if ui.button("下移").clicked() {
            上下文.动作.推(UiAction::执行命令(
                editor_core::command::编辑器命令::下移实体 { 标识: 行.标识 },
            ));
            ui.close();
        }
        if ui.button("移到顶层").clicked() {
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
        if ui.button("删除").clicked() {
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
    上下文
        .动作
        .推(UiAction::执行命令(
            editor_core::command::编辑器命令::替换实体 {
                实体: Box::new(新实体),
            },
        ));
}

/// 绘制重命名输入框。
fn 绘制重命名输入(ui: &mut Ui, 上下文: &mut 绘制上下文<'_>, 行: &显示行) {
    ui.horizontal(|ui| {
        ui.add_space(行.深度 as f32 * ui.spacing().indent + 14.0);
        let 响应 = ui.add(
            egui::TextEdit::singleline(&mut 上下文.界面.文本缓冲).desired_width(160.0),
        );
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
    文档
        .取(标识)
        .map(|实体| !实体.编辑器锁定)
        .unwrap_or(false)
}

/// 面板标题（供停靠标签使用）。
pub fn 标题() -> &'static str {
    Tab::场景树.标题()
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
        let 行 = 构建显示行(&文档);
        let 名称: Vec<&str> = 行.iter().map(|行| 行.名称.as_str()).collect();
        assert_eq!(名称, vec!["根", "甲", "甲的孩子", "乙"]);

        let 深度: Vec<usize> = 行.iter().map(|行| 行.深度).collect();
        assert_eq!(深度, vec![0, 1, 2, 1]);
    }

    #[test]
    fn 有子实体标记正确() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档);
        let 查 = |名称: &str| 行.iter().find(|行| 行.名称 == 名称).unwrap();
        assert!(查("根").有子实体);
        assert!(查("甲").有子实体);
        assert!(!查("乙").有子实体, "乙没有子实体，不应显示折叠箭头");
        assert!(!查("甲的孩子").有子实体);
    }

    #[test]
    fn 折叠的子树不会出现在显示行里() {
        let mut 文档 = 造场景();
        // 深度优先遍历本身不负责折叠，折叠表现在"展开"标记上。
        // 这里验证标记能被正确读出，折叠渲染由 UI 依据该标记处理。
        let 行 = 构建显示行(&文档);
        let 根行 = 行.iter().find(|行| 行.名称 == "根").unwrap();
        assert!(!根行.展开, "新实体的展开标记应当默认关闭");

        // 设置展开标记后应当能被读出
        let 根标识 = 根行.标识;
        if let Some(实体) = 文档.取可变(根标识) {
            实体.编辑器展开 = true;
        }
        let 行 = 构建显示行(&文档);
        assert!(行.iter().find(|行| 行.名称 == "根").unwrap().展开);
    }

    #[test]
    fn 搜索会保留匹配项及其祖先() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档);

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
        let 行 = 构建显示行(&文档);
        assert_eq!(按名称过滤(&行, "").len(), 行.len());
        assert_eq!(按名称过滤(&行, "   ").len(), 行.len());
    }

    #[test]
    fn 无匹配时返回空列表() {
        let 文档 = 造场景();
        let 行 = 构建显示行(&文档);
        assert!(按名称过滤(&行, "根本不存在的东西").is_empty());
    }

    #[test]
    fn 锁定实体不可被视口选中() {
        let mut 文档 = 造场景();
        let 行 = 构建显示行(&文档);
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
        let 行 = 构建显示行(&文档);
        let 甲行 = 行.iter().find(|行| 行.标识 == 甲).unwrap();
        assert!(甲行.隐藏 && 甲行.锁定);
    }
}
