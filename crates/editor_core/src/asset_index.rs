//! 项目级资源引用索引：跨全部场景回答"谁在用这个资源"。
//!
//! # 为什么需要跨场景
//!
//! 单个场景的引用视图由 `scene_doc::asset_refs` 提供。但用户删除一个资源时，
//! 危险的不是"当前打开的场景还用不用它"，而是**项目里任何一个场景**还用不用它。
//! 只检查当前场景就允许删除，会让其他场景在下次打开时静默断链 ——
//! 这正是成熟引擎在删除资源前一定要拦住的那类事故。
//!
//! 反过来，"哪些资源没人用"也只能在全项目视角下回答：当前场景没用到，
//! 不代表别的场景没用到。
//!
//! # 为什么放在 `editor_core` 而不是 `editor_app`
//!
//! 这个索引是**项目数据的只读推导**，不做任何文件写入，也不依赖 Bevy。
//! 放在 `editor_core` 后，界面层可以直接用它决定"删除按钮是否可用"、
//! 动作层用它执行守卫，两边用同一份语义，不必各写一遍扫描逻辑。
//!
//! # 当前文档要优先用内存里的那一份
//!
//! 打开中的场景可能**有未保存修改**。如果索引只读磁盘，用户刚把一个精灵的
//! 贴图删掉、还没保存时去删那张图，守卫会误判"还有引用"；
//! 反之刚加上的引用也可能漏掉。因此 [`项目资源索引::构建`] 接受当前文档与它
//! 的路径，用它替换磁盘上的同名场景。

use std::collections::{BTreeMap, BTreeSet};

use scene_doc::asset_refs::{文档资源引用, 资源引用位置};
use scene_doc::document::场景文档;
use scene_doc::path::项目相对路径;

use crate::project::项目描述;

/// 一处跨场景的资源引用。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct 项目引用 {
    /// 引用所在场景（项目相对路径）。
    pub 场景: 项目相对路径,
    /// 场景内的具体位置。
    pub 位置: 资源引用位置,
    /// 引用该资源的实体名称（供界面显示；实体已不存在时为空串）。
    pub 实体名称: String,
}

/// 项目资源引用索引。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 项目资源索引 {
    /// 资源路径 → 全部引用（按场景、实体、组件、属性排序）。
    pub 按资源: BTreeMap<项目相对路径, Vec<项目引用>>,
    /// 读不出来的场景及原因。它们**不会**被当成"没有引用"：
    /// 索引会记住这一点，删除守卫遇到它必须拒绝，而不是乐观放行。
    pub 无法读取的场景: Vec<(项目相对路径, String)>,
}

impl 项目资源索引 {
    /// 构建索引。
    ///
    /// * `当前场景` 与 `当前文档`：打开中的场景优先用内存文档（可为 `None`）。
    /// * 读不出来的场景记进 [`项目资源索引::无法读取的场景`]，不静默忽略。
    pub fn 构建(
        项目: &项目描述,
        当前场景: Option<&项目相对路径>,
        当前文档: Option<&场景文档>,
    ) -> Self {
        let mut 按资源: BTreeMap<项目相对路径, Vec<项目引用>> = BTreeMap::new();
        let mut 无法读取 = Vec::new();

        for 场景路径 in 项目.场景列表() {
            let 文档 = if 当前场景 == Some(&场景路径) {
                match 当前文档 {
                    Some(文档) => 文档.clone(),
                    None => match 项目.读取场景(&场景路径) {
                        Ok(读取) => 读取.文档,
                        Err(错误) => {
                            无法读取.push((场景路径.clone(), 错误.to_string()));
                            continue;
                        }
                    },
                }
            } else {
                match 项目.读取场景(&场景路径) {
                    Ok(读取) => 读取.文档,
                    Err(错误) => {
                        无法读取.push((场景路径.clone(), 错误.to_string()));
                        continue;
                    }
                }
            };

            for 位置 in 文档资源引用::扫描(&文档).按资源 {
                for 引用 in 位置.1 {
                    let 实体名称 = 文档
                        .取(引用.实体)
                        .map(|实体| 实体.名称.clone())
                        .unwrap_or_default();
                    按资源.entry(位置.0.clone()).or_default().push(项目引用 {
                        场景: 场景路径.clone(),
                        位置: 引用,
                        实体名称,
                    });
                }
            }
        }

        for 列表 in 按资源.values_mut() {
            列表.sort();
            列表.dedup();
        }

        Self {
            按资源,
            无法读取的场景: 无法读取,
        }
    }

    /// 某资源在项目里被引用的全部位置。
    pub fn 引用位置(&self, 资源: &项目相对路径) -> &[项目引用] {
        self.按资源.get(资源).map_or(&[], Vec::as_slice)
    }

    /// 某资源在项目里被引用的次数。
    pub fn 引用次数(&self, 资源: &项目相对路径) -> usize {
        self.引用位置(资源).len()
    }

    /// 某资源被哪些场景引用（去重、排序）。
    pub fn 引用场景(&self, 资源: &项目相对路径) -> Vec<&项目相对路径> {
        let mut 集合: BTreeSet<&项目相对路径> = BTreeSet::new();
        for 引用 in self.引用位置(资源) {
            集合.insert(&引用.场景);
        }
        集合.into_iter().collect()
    }

    /// 删除守卫：返回阻止删除的理由。
    ///
    /// 只要还有引用、或有场景读不出来，就**不允许**删除：
    /// * 有引用：删掉会让那些场景断链；
    /// * 有不可读场景：无法证明那里没有引用，宁可拒绝也不能乐观放行。
    ///
    /// 返回 `None` 表示可以安全删除。
    pub fn 阻止删除原因(&self, 资源: &项目相对路径) -> Option<String> {
        let 引用 = self.引用位置(资源);
        if !引用.is_empty() {
            let 场景数 = self.引用场景(资源).len();
            let 位置数 = 引用.len();
            return Some(format!(
                "该资源仍被 {场景数} 个场景中的 {位置数} 处引用：{}",
                self.引用摘要文本(资源)
            ));
        }
        if !self.无法读取的场景.is_empty() {
            let 名称: Vec<&str> = self
                .无法读取的场景
                .iter()
                .map(|(路径, _)| 路径.文本())
                .collect();
            return Some(format!(
                "有 {} 个场景无法读取，无法确认该资源是否被引用：{}",
                名称.len(),
                名称.join("、")
            ));
        }
        None
    }

    /// 一句话引用摘要（供日志与确认对话框）。
    ///
    /// 最多列出 5 处，其余用"等 N 处"概括；完整列表由界面从
    /// [`项目资源索引::引用位置`] 取。
    pub fn 引用摘要文本(&self, 资源: &项目相对路径) -> String {
        const 上限: usize = 5;
        let 引用 = self.引用位置(资源);
        let mut 片段: Vec<String> = 引用
            .iter()
            .take(上限)
            .map(|项| {
                let 名称 = if 项.实体名称.is_empty() {
                    "(已删除实体)".to_string()
                } else {
                    项.实体名称.clone()
                };
                format!(
                    "{} / {} / {}.{}",
                    项.场景.文本(),
                    名称,
                    项.位置.组件,
                    项.位置.属性路径
                )
            })
            .collect();
        if 引用.len() > 上限 {
            片段.push(format!("等 {} 处", 引用.len()));
        }
        片段.join("；")
    }

    /// 项目里**确实存在、但没有任何场景引用**的资源。
    ///
    /// 只统计 `assets/` 下真实存在的文件；不把"引用里出现的路径"混进来，
    /// 因此这个列表可以直接用来清理磁盘空间。
    ///
    /// 有场景读不出来时仍然返回结果，但调用方应提醒用户"可能不完整"。
    pub fn 未被引用的资源(&self, 项目: &项目描述) -> Vec<项目相对路径> {
        let 在用: BTreeSet<&项目相对路径> = self.按资源.keys().collect();
        项目
            .资源列表()
            .into_iter()
            .filter(|条目| !条目.是目录)
            .map(|条目| 条目.路径)
            .filter(|路径| !在用.contains(路径))
            .collect()
    }

    /// 被引用、但磁盘上不存在的资源。
    ///
    /// 与 [`项目描述::检查资源引用`] 的区别：这里以**引用**为入口，
    /// 能回答"这个路径有没有丢"，不依赖场景是否能读；`检查资源引用`
    /// 则是逐场景扫，遇到坏场景会跳过。两者互补，不互相替代。
    pub fn 缺失的资源(&self, 项目: &项目描述) -> Vec<项目相对路径> {
        self.按资源
            .keys()
            .filter(|路径| !项目.绝对路径(路径).is_file())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::{场景文档, 组件数据};
    use scene_doc::property::{属性值, 资源引用值};
    use std::collections::BTreeMap;

    fn 路径(文本: &str) -> 项目相对路径 {
        项目相对路径::解析(文本).expect("路径")
    }

    fn 精灵(资源: 项目相对路径) -> 组件数据 {
        let mut 属性 = BTreeMap::new();
        属性.insert(
            "texture".to_string(),
            属性值::资源(资源引用值::按路径(资源)),
        );
        组件数据::新建("Sprite", 属性)
    }

    fn 建项目() -> (tempfile::TempDir, 项目描述) {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 根 = 目录.path().join("项目");
        let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
        std::fs::write(根.join("assets").join("房屋.png"), b"a").expect("写资源");
        std::fs::write(根.join("assets").join("未用.png"), b"b").expect("写资源");
        (目录, 项目)
    }

    fn 写场景(项目: &项目描述, 名称: &str, 资源: &str) -> 项目相对路径 {
        let 场景路径 = 路径(&format!("scenes/{名称}.scene.ron"));
        let mut 文档 = 场景文档::新建(名称);
        let 实体 = 文档.生成实体("房屋", None);
        文档.取可变(实体).expect("实体").加组件(精灵(路径(资源)));
        文档.同步资源表();
        项目.保存场景(&场景路径, &文档).expect("保存场景");
        场景路径
    }

    #[test]
    fn 跨场景索引能找到全部引用场景() {
        let (_目录, 项目) = 建项目();
        写场景(&项目, "甲", "assets/房屋.png");
        写场景(&项目, "乙", "assets/房屋.png");
        let 索引 = 项目资源索引::构建(&项目, None, None);
        assert_eq!(索引.引用次数(&路径("assets/房屋.png")), 2);
        assert_eq!(索引.引用场景(&路径("assets/房屋.png")).len(), 2);
    }

    #[test]
    fn 当前文档优先于磁盘内容() {
        let (_目录, 项目) = 建项目();
        let 场景 = 写场景(&项目, "甲", "assets/房屋.png");
        // 内存文档把引用改到另一张图（尚未保存）。
        let mut 文档 = 场景文档::新建("甲");
        let 实体 = 文档.生成实体("房屋", None);
        文档
            .取可变(实体)
            .expect("实体")
            .加组件(精灵(路径("assets/未用.png")));
        文档.同步资源表();

        let 索引 = 项目资源索引::构建(&项目, Some(&场景), Some(&文档));
        assert_eq!(索引.引用次数(&路径("assets/房屋.png")), 0);
        assert_eq!(索引.引用次数(&路径("assets/未用.png")), 1);
    }

    #[test]
    fn 有引用时阻止删除并给出位置() {
        let (_目录, 项目) = 建项目();
        写场景(&项目, "甲", "assets/房屋.png");
        let 索引 = 项目资源索引::构建(&项目, None, None);
        let 原因 = 索引
            .阻止删除原因(&路径("assets/房屋.png"))
            .expect("有引用时必须阻止");
        assert!(原因.contains("房屋"), "原因：{原因}");
    }

    #[test]
    fn 无引用时允许删除并列出未使用资源() {
        let (_目录, 项目) = 建项目();
        写场景(&项目, "甲", "assets/房屋.png");
        let 索引 = 项目资源索引::构建(&项目, None, None);
        assert!(索引.阻止删除原因(&路径("assets/未用.png")).is_none());
        let 未用 = 索引.未被引用的资源(&项目);
        assert_eq!(未用, vec![路径("assets/未用.png")]);
    }

    #[test]
    fn 损坏场景会让删除守卫拒绝() {
        let (_目录, 项目) = 建项目();
        写场景(&项目, "甲", "assets/房屋.png");
        // 写一个读不出来的场景文件。
        std::fs::write(项目.根目录().join("scenes/坏.scene.ron"), "{ 这不是 RON")
            .expect("写坏场景");
        let 索引 = 项目资源索引::构建(&项目, None, None);
        assert_eq!(索引.无法读取的场景.len(), 1);
        let 原因 = 索引
            .阻止删除原因(&路径("assets/未用.png"))
            .expect("有不可读场景时必须拒绝");
        assert!(原因.contains("无法读取"), "原因：{原因}");
    }

    #[test]
    fn 缺失资源能被单独列出() {
        let (_目录, 项目) = 建项目();
        写场景(&项目, "甲", "assets/不存在.png");
        let 索引 = 项目资源索引::构建(&项目, None, None);
        assert_eq!(索引.缺失的资源(&项目), vec![路径("assets/不存在.png")]);
    }
}
