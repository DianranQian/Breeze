//! 选择集：当前在编辑器里被选中的实体集合。
//!
//! # 为什么选择集不是文档的一部分
//!
//! "选中了谁"是**视图状态**，不是数据。把它放进场景文档会带来两个问题：
//! 保存时把临时选中状态写进文件（下次打开莫名其妙有东西是选中的），
//! 以及每次点击都要走撤销栈（用户按 Ctrl+Z 会期望撤销编辑，而不是撤销点选）。
//!
//! 因此选择集独立存放，并且**不参与撤销重做**。但删除实体的命令
//! 需要清理选择集里已经不存在的标识，这一职责在 [`选择集::清理失效项`]。

use std::collections::BTreeSet;

use scene_doc::document::{场景文档, 实体标识};

/// 当前选中的实体集合。
///
/// 用有序集合而非 `Vec`：多选场景下要频繁做"是否已选中"判断，
/// 有序集合让这个判断是 O(log n)，同时保证遍历顺序稳定（不会因
/// 点击顺序不同导致 Inspector 显示顺序抖动）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 选择集 {
    选中: BTreeSet<实体标识>,
    /// 主选中项，Inspector 优先展示它。通常等于最近一次点选的实体。
    主选中: Option<实体标识>,
}

impl 选择集 {
    /// 创建一个空选择集。
    pub fn 新建() -> Self {
        Self::default()
    }

    /// 是否为空。
    pub fn 为空(&self) -> bool {
        self.选中.is_empty()
    }

    /// 选中数量。
    pub fn 数量(&self) -> usize {
        self.选中.len()
    }

    /// 是否选中了指定实体。
    pub fn 包含(&self, 标识: 实体标识) -> bool {
        self.选中.contains(&标识)
    }

    /// 主选中项。空选择集时为 `None`；若主选中项失效则回退到集合里的第一个。
    pub fn 主选中(&self) -> Option<实体标识> {
        match self.主选中 {
            Some(标识) if self.选中.contains(&标识) => Some(标识),
            _ => self.选中.iter().next().copied(),
        }
    }

    /// 所有选中项（有序）。
    pub fn 全部(&self) -> Vec<实体标识> {
        self.选中.iter().copied().collect()
    }

    /// 迭代选中项。
    pub fn 迭代(&self) -> impl Iterator<Item = 实体标识> + '_ {
        self.选中.iter().copied()
    }

    /// 单选：清空后只选中一个。
    pub fn 单选(&mut self, 标识: 实体标识) {
        self.选中.clear();
        self.选中.insert(标识);
        self.主选中 = Some(标识);
    }

    /// 切换某个实体的选中状态（多选点选用）。
    pub fn 切换(&mut self, 标识: 实体标识) {
        if self.选中.remove(&标识) {
            if self.主选中 == Some(标识) {
                self.主选中 = self.选中.iter().next().copied();
            }
        } else {
            self.选中.insert(标识);
            self.主选中 = Some(标识);
        }
    }

    /// 追加选中，不清空已有选择（Shift 范围选择用）。
    pub fn 追加(&mut self, 标识: 实体标识) {
        self.选中.insert(标识);
        self.主选中 = Some(标识);
    }

    /// 清空选择。
    pub fn 清空(&mut self) {
        self.选中.clear();
        self.主选中 = None;
    }

    /// 用给定集合整体替换当前选择。
    pub fn 设为(&mut self, 标识列表: impl IntoIterator<Item = 实体标识>) {
        self.选中 = 标识列表.into_iter().collect();
        self.主选中 = self.选中.iter().next().copied();
    }

    /// 移除指定实体（若被选中）。
    pub fn 移除(&mut self, 标识: 实体标识) {
        if self.选中.remove(&标识) && self.主选中 == Some(标识) {
            self.主选中 = self.选中.iter().next().copied();
        }
    }

    /// 剔除文档中已不存在的实体。
    ///
    /// 必须在每次删除实体之后调用，否则 Inspector 与视口
    /// 会继续尝试访问已消失的实体，表现为"幽灵选中"。
    /// 返回被清理掉的数量。
    pub fn 清理失效项(&mut self, 文档: &场景文档) -> usize {
        let 原数量 = self.选中.len();
        self.选中.retain(|标识| 文档.存在(*标识));
        if let Some(主) = self.主选中
            && !self.选中.contains(&主)
        {
            self.主选中 = self.选中.iter().next().copied();
        }
        if self.选中.is_empty() {
            self.主选中 = None;
        }
        原数量 - self.选中.len()
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use scene_doc::document::实体标识;

    #[test]
    fn 单选会覆盖之前的选择() {
        let mut 选择 = 选择集::新建();
        let 甲 = 实体标识::新建();
        let 乙 = 实体标识::新建();
        选择.单选(甲);
        选择.单选(乙);
        assert_eq!(选择.数量(), 1);
        assert!(选择.包含(乙));
        assert!(!选择.包含(甲));
        assert_eq!(选择.主选中(), Some(乙));
    }

    #[test]
    fn 切换可以取消选中并回退主选中项() {
        let mut 选择 = 选择集::新建();
        let 甲 = 实体标识::新建();
        let 乙 = 实体标识::新建();
        选择.单选(甲);
        选择.切换(乙);
        assert_eq!(选择.数量(), 2);
        assert_eq!(选择.主选中(), Some(乙));

        // 取消主选中项后，主选中应回退到剩余的项，而不是变成 None
        选择.切换(乙);
        assert_eq!(选择.数量(), 1);
        assert_eq!(选择.主选中(), Some(甲));
    }

    #[test]
    fn 清理失效项会剔除不在文档里的实体() {
        let mut 文档 = 场景文档::新建("测试");
        let 存在的 = 文档.生成实体("存在", None);
        let 消失的 = 场景_doc_实体标识();

        let mut 选择 = 选择集::新建();
        选择.设为([存在的, 消失的]);
        assert_eq!(选择.数量(), 2);

        let 清理数 = 选择.清理失效项(&文档);
        assert_eq!(清理数, 1);
        assert_eq!(选择.全部(), vec![存在的]);
    }

    fn 场景_doc_实体标识() -> 实体标识 {
        实体标识::新建()
    }
}
