//! 场景文档里"谁引用了哪个资源"的**只读**查询。
//!
//! # 为什么单独一个文件而不是塞回 `document.rs`
//!
//! `document.rs` 已经很长，负责文档的增删改、校验与序列化。
//! "把文档里的资源引用摊平成一张表"是一个只读视图：
//! 它不改变文档，只回答编辑器的三个问题：
//!
//! 1. **删除这个资源安全吗**：还有哪个实体、哪个组件的哪个字段指向它？
//! 2. **重命名后要改哪里**：同一份答案，用来在改名后确认没有漏改。
//! 3. **哪些资源没人用**：把全项目的引用并起来，与磁盘上的资源列表相减。
//!
//! 这三个问题都要求"到底是谁引用"可追溯，而不是只给一个计数。
//! 因此这里返回的是带位置的引用列表。
//!
//! # 与 `document.rs` 里私有函数的关系
//!
//! `document.rs` 里有一个私有的 `收集资源引用`，只保留路径与唯一标识，
//! 因为它服务的是"资源表完整性"（不需要知道引用来自哪个实体）。
//! 这里需要实体与字段，所以递归规则相同、落点信息更全。
//! `文档资源引用与资源表路径集合一致` 这条测试用来防止两处规则漂移。

use std::collections::BTreeMap;

use crate::document::{场景文档, 实体标识};
use crate::path::项目相对路径;
use crate::property::属性值;

/// 一个具体位置的资源引用。
///
/// "位置"必须精确到**组件 + 属性路径**，否则用户看到"某个实体引用了这张图"
/// 仍然不知道去改哪个字段。属性路径用 `.` 连接嵌套结构，
/// 列表元素写成 `[0]`，例如 `texture`、`路径[2].图标`。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct 资源引用位置 {
    /// 引用所在实体。
    pub 实体: 实体标识,
    /// 引用所在组件类型名（例如 `Sprite`）。
    pub 组件: String,
    /// 组件内的属性路径（例如 `texture` 或 `路径[1]`）。
    pub 属性路径: String,
}

/// 一份文档内的资源引用视图。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 文档资源引用 {
    /// 资源路径 → 引用位置（同一位置只出现一次，按实体/组件/路径排序）。
    pub 按资源: BTreeMap<项目相对路径, Vec<资源引用位置>>,
}

impl 文档资源引用 {
    /// 扫描整份文档，得到资源路径到引用位置的映射。
    ///
    /// 复杂度 O(组件字段数)。只读，不修改文档，也不改变资源表。
    pub fn 扫描(文档: &场景文档) -> Self {
        let mut 按资源: BTreeMap<项目相对路径, Vec<资源引用位置>> = BTreeMap::new();
        // 显式按 key 排序遍历，保证以后实体表换成 HashMap 时结果顺序也不漂移。
        let 标识列表: Vec<实体标识> = 文档.实体表.keys().copied().collect();
        for 标识 in 标识列表 {
            let Some(实体) = 文档.实体表.get(&标识) else {
                continue;
            };
            for 组件 in &实体.组件 {
                for (属性名, 值) in &组件.属性 {
                    let mut 前缀 = String::new();
                    收集(值, 属性名, &mut 前缀, &mut |资源, 路径| {
                        按资源.entry(资源.clone()).or_default().push(资源引用位置 {
                            实体: 标识,
                            组件: 组件.类型名.clone(),
                            属性路径: 路径.to_string(),
                        });
                    });
                }
            }
        }
        for 位置列表 in 按资源.values_mut() {
            位置列表.sort();
            位置列表.dedup();
        }
        Self { 按资源 }
    }

    /// 某资源在这份文档里被引用的次数（位置数，不是实体数）。
    pub fn 引用次数(&self, 资源: &项目相对路径) -> usize {
        self.按资源.get(资源).map_or(0, Vec::len)
    }

    /// 某资源在这份文档里被哪些位置引用。
    pub fn 引用位置(&self, 资源: &项目相对路径) -> &[资源引用位置] {
        self.按资源.get(资源).map_or(&[], Vec::as_slice)
    }

    /// 文档里出现的全部资源路径。
    pub fn 全部路径(&self) -> Vec<&项目相对路径> {
        self.按资源.keys().collect()
    }

    /// 文档引用了、但资源表里没有登记的路径。
    ///
    /// 正常流程下（保存前跑 `同步资源表`）这里应当为空；
    /// 非空说明有人绕过了文档 API 直接改了组件属性。
    pub fn 未登记(&self, 文档: &场景文档) -> Vec<项目相对路径> {
        self.按资源
            .keys()
            .filter(|路径| !文档.资源表.contains_key(*路径))
            .cloned()
            .collect()
    }
}

/// 递归收集一个属性值里的资源引用，并把当前属性路径交给 `输出`。
///
/// `前缀` 在递归中增长、返回前恢复原长，因此同一棵树里可以复用同一个
/// `String`，不必为每一层分配新串。列表元素写成 `[下标]`。
fn 收集(
    值: &属性值, 段: &str, 前缀: &mut String, 输出: &mut impl FnMut(&项目相对路径, &str)
) {
    let 原长 = 前缀.len();
    if !前缀.is_empty() && !段.starts_with('[') {
        前缀.push('.');
    }
    前缀.push_str(段);

    match 值 {
        属性值::资源(引用) => 输出(&引用.路径, 前缀),
        属性值::列表(列表) => {
            for (下标, 项) in 列表.iter().enumerate() {
                let 段 = format!("[{下标}]");
                收集(项, &段, 前缀, 输出);
            }
        }
        属性值::结构(字段) => {
            for (键, 项) in 字段 {
                收集(项, 键, 前缀, 输出);
            }
        }
        属性值::枚举(枚举) => {
            if let Some(负载) = &枚举.负载 {
                收集(负载, "", 前缀, 输出);
            }
        }
        _ => {}
    }

    前缀.truncate(原长);
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::document::组件数据;
    use crate::property::{属性值, 资源引用值};
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

    fn 带资源的文档() -> 场景文档 {
        let mut 文档 = 场景文档::新建("引用");
        let 甲 = 文档.生成实体("房屋", None);
        let 乙 = 文档.生成实体("树", None);
        let 资源 = 路径("assets/xiuxian/房屋.png");
        文档.取可变(甲).expect("甲").加组件(精灵(资源.clone()));
        文档.取可变(乙).expect("乙").加组件(精灵(资源));
        // 列表：路径必须能指到具体元素。
        let mut 属性 = BTreeMap::new();
        属性.insert(
            "点".to_string(),
            属性值::列表(vec![
                属性值::空,
                属性值::资源(资源引用值::按路径(路径("assets/路标.png"))),
            ]),
        );
        文档
            .取可变(甲)
            .expect("甲")
            .加组件(组件数据::新建("路径点", 属性));
        文档.同步资源表();
        文档
    }

    #[test]
    fn 同一个资源被两个实体引用时都被列出() {
        let 文档 = 带资源的文档();
        let 引用 = 文档资源引用::扫描(&文档);
        let 房屋 = 路径("assets/xiuxian/房屋.png");
        assert_eq!(引用.引用次数(&房屋), 2, "两个 Sprite 各一处");
        let 位置 = 引用.引用位置(&房屋);
        assert!(
            位置
                .iter()
                .any(|项| 项.组件 == "Sprite" && 项.属性路径 == "texture")
        );
    }

    #[test]
    fn 列表里的引用带下标位置() {
        let 文档 = 带资源的文档();
        let 引用 = 文档资源引用::扫描(&文档);
        let 路标 = 路径("assets/路标.png");
        let 位置 = 引用.引用位置(&路标);
        assert_eq!(位置.len(), 1);
        assert_eq!(位置[0].属性路径, "点[1]", "必须指到列表里的具体一项");
    }

    #[test]
    fn 不存在的引用不会被报告() {
        let 文档 = 带资源的文档();
        let 引用 = 文档资源引用::扫描(&文档);
        assert_eq!(引用.引用次数(&路径("assets/没有.png")), 0);
        assert!(引用.引用位置(&路径("assets/没有.png")).is_empty());
    }

    #[test]
    fn 文档资源引用与资源表路径集合一致() {
        // 防止本文件与 `document.rs` 里的私有收集规则漂移。
        let 文档 = 带资源的文档();
        let 引用 = 文档资源引用::扫描(&文档);
        let 从视图: std::collections::BTreeSet<项目相对路径> =
            引用.全部路径().into_iter().cloned().collect();
        let 从资源表: std::collections::BTreeSet<项目相对路径> =
            文档.资源表.keys().cloned().collect();
        assert_eq!(从视图, 从资源表, "两份实现必须看到同一批资源");
        assert!(引用.未登记(&文档).is_empty(), "资源表应当包含全部引用");
    }
}
