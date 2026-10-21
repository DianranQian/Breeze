//! 资源文件的操作闭环：重命名/移动与删除。
//!
//! # 为什么这层必须存在（而不是在动作处理器里就地写几行）
//!
//! "把 `assets/a.png` 改成 `assets/b.png`"看起来只是 `fs::rename`。
//! 但在一个有场景引用的项目里，它其实是一个**跨文件事务**：
//!
//! 1. 项目里其它场景可能引用旧路径 —— 不跟着改，下次打开就是断链；
//! 2. 当前打开的场景可能引用旧路径 —— 它还有未保存修改，不能被静默写盘；
//! 3. 场景写到一半失败 —— 必须把已经改过的文件与场景全部退回去，
//!    否则用户得到的是"一半旧路径、一半新路径"的项目。
//!
//! 因此这里把顺序固定下来，并且**任一步失败都回滚**：
//!
//! ```text
//! 校验新路径 → 预读受影响场景（原文留底） → 移动文件
//!   → 写回受影响场景 → 提交当前文档的命令
//! ```
//!
//! 为什么先移动文件再改引用：如果先改内存文档、移动文件失败，
//! 用户会在编辑器里看到"引用已经改了，但文件还在旧名字"，
//! 一保存就把断链写进磁盘。先动文件的话，最坏情况是"文件已改名、
//! 场景还没改"，而这一步失败会立刻把文件移回去。
//!
//! # 删除的规则
//!
//! 删除**不做级联清理**。只要项目里还有任何场景引用它，就拒绝删除并
//! 列出引用位置；有场景读不出来时也拒绝（无法证明没有引用）。
//! 这是成熟引擎的通行做法：宁可让用户先去处理引用，也不要静默毁掉数据。

use std::collections::BTreeMap;

use editor_core::asset_index::项目资源索引;
use editor_core::command::编辑器命令;
use editor_core::state::编辑器状态;
use scene_doc::path::项目相对路径;

/// 一次资源变更的真实结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 资源变更报告 {
    /// 磁盘上被同步改写并保存的场景。
    pub 更新场景: Vec<项目相对路径>,
    /// 打开中的场景是否被改写（走撤销栈，不自动写盘）。
    pub 当前文档已更新: bool,
    /// 当前文档里该资源的引用处数。
    pub 当前文档引用数: usize,
    /// 其它场景里该资源的引用处数。
    pub 其它场景引用数: usize,
}

impl 资源变更报告 {
    /// 一句话摘要（写入控制台）。
    pub fn 摘要(&self) -> String {
        let mut 部分 = Vec::new();
        if !self.更新场景.is_empty() {
            部分.push(format!("改写 {} 个场景", self.更新场景.len()));
        }
        if self.当前文档已更新 {
            部分.push("当前场景已同步（可用 Ctrl+Z 撤销）".to_string());
        }
        if 部分.is_empty() {
            "没有场景引用该资源".to_string()
        } else {
            部分.join("，")
        }
    }
}

/// 重命名/移动一个资源，并同步所有场景引用。
///
/// 返回 `Err` 时保证：**文件仍在旧路径、所有场景仍是原文**。
pub fn 重命名资源(
    编辑器: &mut 编辑器状态,
    旧路径: &项目相对路径,
    新路径: &项目相对路径,
) -> Result<资源变更报告, String> {
    let Some(项目) = 编辑器.项目.clone() else {
        return Err("没有打开的项目".to_string());
    };

    // ---- 校验（不碰磁盘） ----
    if 旧路径 == 新路径 {
        return Err("新旧路径相同，没有需要执行的操作".to_string());
    }
    if !旧路径.文本().starts_with("assets/") {
        return Err(format!("只能在资源目录内移动：{}", 旧路径.文本()));
    }
    if !新路径.文本().starts_with("assets/") {
        return Err(format!("目标必须在资源目录内：{}", 新路径.文本()));
    }
    let 旧绝对 = 项目.绝对路径(旧路径);
    let 新绝对 = 项目.绝对路径(新路径);
    if !旧绝对.is_file() {
        return Err(format!("源资源不存在：{}", 旧路径.文本()));
    }
    if 新绝对.exists() {
        return Err(format!("目标路径已存在：{}", 新路径.文本()));
    }
    if 旧路径.扩展名() != 新路径.扩展名() {
        // 改扩展名会改变资源类别（图片变"其他"），预览与运行时加载都会失效。
        // 真的需要换格式，应当走导入管线而不是重命名。
        return Err("重命名不能改变文件扩展名（会改变资源类型）".to_string());
    }

    // ---- 预读受影响场景 ----
    //
    // 索引同时覆盖"当前内存文档"和"磁盘上的其它场景"：
    // 当前文档里未保存的引用也必须算进去。
    let 索引 =
        项目资源索引::构建(&项目, 编辑器.会话.路径.as_ref(), Some(&编辑器.会话.文档));
    let 当前路径 = 编辑器.会话.路径.clone();
    let 引用列表 = 索引.引用位置(旧路径).to_vec();
    let 当前文档引用数 = 引用列表
        .iter()
        .filter(|引用| Some(&引用.场景) == 当前路径.as_ref())
        .count();
    let 其它场景引用数 = 引用列表.len().saturating_sub(当前文档引用数);

    // 需要改写并写盘的场景（当前场景除外：它走撤销栈，不静默写盘）。
    let 待改写: Vec<项目相对路径> = 引用列表
        .iter()
        .map(|引用| 引用.场景.clone())
        .filter(|场景| Some(场景) != 当前路径.as_ref())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut 原文留底: BTreeMap<项目相对路径, String> = BTreeMap::new();
    let mut 新文档: BTreeMap<项目相对路径, scene_doc::document::场景文档> = BTreeMap::new();
    for 场景 in &待改写 {
        let 绝对 = 项目.绝对路径(场景);
        let 原文 = std::fs::read_to_string(&绝对)
            .map_err(|错误| format!("读取场景 {} 失败：{错误}", 场景.文本()))?;
        let 读取 = 项目
            .读取场景(场景)
            .map_err(|错误| format!("解析场景 {} 失败：{错误}", 场景.文本()))?;
        let mut 文档 = 读取.文档;
        文档.改资源路径(旧路径, 新路径);
        原文留底.insert(场景.clone(), 原文);
        新文档.insert(场景.clone(), 文档);
    }

    // ---- 移动文件 ----
    项目
        .移动资源(旧路径, 新路径)
        .map_err(|错误| format!("移动文件失败：{错误}"))?;

    // ---- 写回场景（失败则逆序回滚） ----
    let mut 已写: Vec<项目相对路径> = Vec::new();
    for (场景, 文档) in &新文档 {
        match 项目.保存场景(场景, 文档) {
            Ok(()) => 已写.push(场景.clone()),
            Err(错误) => {
                // 先还原已经写过的场景，再还原文件与本次失败的场景。
                let 还原错误 = 回滚场景(&项目, &原文留底);
                let _ = 项目.移动资源(新路径, 旧路径);
                let 回滚说明 = 还原错误
                    .map(|额外| format!("；回滚时另有失败：{额外}"))
                    .unwrap_or_default();
                return Err(format!("写回场景 {} 失败：{错误}{回滚说明}", 场景.文本()));
            }
        }
    }

    // ---- 当前文档：走命令，进撤销栈 ----
    let 当前文档已更新 = 当前文档引用数 > 0;
    if 当前文档已更新 {
        let 结果 = 编辑器.执行并记录(编辑器命令::改资源路径 {
            旧路径: 旧路径.clone(),
            新路径: 新路径.clone(),
        });
        if 结果.is_none() {
            // 命令失败意味着当前文档没能同步。此时磁盘已经改完，
            // 必须把文件与场景全部退回去，不能留下"文件新名、文档旧引用"。
            let 还原错误 = 回滚场景(&项目, &原文留底);
            let _ = 项目.移动资源(新路径, 旧路径);
            let 回滚说明 = 还原错误
                .map(|额外| format!("；回滚时另有失败：{额外}"))
                .unwrap_or_default();
            return Err(format!("更新当前场景引用失败，已回滚{回滚说明}"));
        }
    }

    Ok(资源变更报告 {
        更新场景: 已写,
        当前文档已更新,
        当前文档引用数,
        其它场景引用数,
    })
}

/// 删除一个资源；有任何引用时拒绝。
///
/// 返回 `Err` 时文件仍然存在。
pub fn 删除资源(
    编辑器: &mut 编辑器状态,
    路径: &项目相对路径,
) -> Result<资源变更报告, String> {
    let Some(项目) = 编辑器.项目.clone() else {
        return Err("没有打开的项目".to_string());
    };
    if !路径.文本().starts_with("assets/") {
        return Err(format!("只能在资源目录内删除：{}", 路径.文本()));
    }
    let 绝对 = 项目.绝对路径(路径);
    if !绝对.exists() {
        return Err(format!("资源不存在：{}", 路径.文本()));
    }
    if 绝对.is_dir() {
        return Err("暂不支持删除目录资源".to_string());
    }

    let 索引 =
        项目资源索引::构建(&项目, 编辑器.会话.路径.as_ref(), Some(&编辑器.会话.文档));
    if let Some(原因) = 索引.阻止删除原因(路径) {
        return Err(原因);
    }

    let 当前文档引用数 = 索引.引用次数(路径);
    项目
        .删除资源(路径)
        .map_err(|错误| format!("删除文件失败：{错误}"))?;
    Ok(资源变更报告 {
        更新场景: Vec::new(),
        当前文档已更新: false,
        当前文档引用数,
        其它场景引用数: 0,
    })
}

/// 用留底的原文还原场景。返回第一个失败说明（若有）。
fn 回滚场景(
    项目: &editor_core::project::项目描述,
    原文: &BTreeMap<项目相对路径, String>,
) -> Option<String> {
    let mut 失败 = Vec::new();
    for (场景, 文本) in 原文 {
        let 绝对 = 项目.绝对路径(场景);
        if let Err(错误) = std::fs::write(&绝对, 文本) {
            失败.push(format!("{}：{错误}", 场景.文本()));
        }
    }
    (!失败.is_empty()).then(|| 失败.join("；"))
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_core::project::项目描述;
    use scene_doc::document::{场景文档, 组件数据};
    use scene_doc::property::{属性值, 资源引用值};
    use std::collections::BTreeMap as 表;

    fn 路径(文本: &str) -> 项目相对路径 {
        项目相对路径::解析(文本).expect("路径")
    }

    fn 精灵(资源: 项目相对路径) -> 组件数据 {
        let mut 属性 = 表::new();
        属性.insert(
            "texture".to_string(),
            属性值::资源(资源引用值::按路径(资源)),
        );
        组件数据::新建("Sprite", 属性)
    }

    /// 建项目 + 两张图 + 两个场景，其中一个场景是当前打开的场景。
    fn 场景夹具() -> (tempfile::TempDir, 编辑器状态) {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 根 = 目录.path().join("项目");
        let 项目 = 项目描述::创建(&根, "项目").expect("创建项目");
        std::fs::write(根.join("assets").join("房屋.png"), "房屋").expect("写资源");
        std::fs::write(根.join("assets").join("未用.png"), "未用").expect("写资源");

        // 场景甲：当前打开。
        let mut 甲 = 场景文档::新建("甲");
        let 实体 = 甲.生成实体("房屋", None);
        甲.取可变(实体)
            .expect("实体")
            .加组件(精灵(路径("assets/房屋.png")));
        甲.同步资源表();
        let 甲路径 = 路径("scenes/甲.scene.ron");
        项目.保存场景(&甲路径, &甲).expect("保存甲");

        // 场景乙：只在磁盘上。
        let mut 乙 = 场景文档::新建("乙");
        let 实体 = 乙.生成实体("另一间房", None);
        乙.取可变(实体)
            .expect("实体")
            .加组件(精灵(路径("assets/房屋.png")));
        乙.同步资源表();
        项目
            .保存场景(&路径("scenes/乙.scene.ron"), &乙)
            .expect("保存乙");

        let mut 编辑器 = 编辑器状态::新建();
        编辑器.项目 = Some(项目);
        编辑器.会话 = editor_core::state::场景会话 {
            文档: 甲,
            路径: Some(甲路径),
        };
        (目录, 编辑器)
    }

    #[test]
    fn 重命名会同步磁盘场景与当前文档且可撤销() {
        let (_目录, mut 编辑器) = 场景夹具();
        let 旧 = 路径("assets/房屋.png");
        let 新 = 路径("assets/客厅.png");
        let 报告 = 重命名资源(&mut 编辑器, &旧, &新).expect("重命名");

        assert_eq!(报告.更新场景.len(), 1, "只有乙需要写盘");
        assert!(报告.当前文档已更新);
        assert_eq!(报告.当前文档引用数, 1);

        let 项目 = 编辑器.项目.as_ref().expect("项目");
        assert!(项目.绝对路径(&新).is_file(), "新文件应当存在");
        assert!(!项目.绝对路径(&旧).exists(), "旧文件应当已移走");

        // 磁盘场景乙已改写。
        let 乙 = 编辑器
            .项目
            .as_ref()
            .unwrap()
            .读取场景(&路径("scenes/乙.scene.ron"))
            .expect("读乙")
            .文档;
        assert_eq!(
            乙.资源表.keys().cloned().collect::<Vec<_>>(),
            vec![新.clone()]
        );

        // 当前文档也改了，且撤销能退回。
        assert!(编辑器.会话.文档.资源表.contains_key(&新));
        assert!(编辑器.撤销一步());
        assert!(
            编辑器.会话.文档.资源表.contains_key(&旧),
            "撤销应退回旧引用"
        );
    }

    #[test]
    fn 目标已存在时拒绝且不动任何文件() {
        let (_目录, mut 编辑器) = 场景夹具();
        let 错误 = 重命名资源(
            &mut 编辑器,
            &路径("assets/房屋.png"),
            &路径("assets/未用.png"),
        )
        .expect_err("目标存在必须拒绝");
        assert!(错误.contains("已存在"), "错误：{错误}");
        let 项目 = 编辑器.项目.as_ref().unwrap();
        assert!(项目.绝对路径(&路径("assets/房屋.png")).is_file());
    }

    #[test]
    fn 改扩展名会被拒绝() {
        let (_目录, mut 编辑器) = 场景夹具();
        let 错误 = 重命名资源(
            &mut 编辑器,
            &路径("assets/房屋.png"),
            &路径("assets/房屋.jpg"),
        )
        .expect_err("改扩展名必须拒绝");
        assert!(错误.contains("扩展名"), "错误：{错误}");
    }

    #[test]
    fn 有引用时删除被拒绝并列出场景() {
        let (_目录, mut 编辑器) = 场景夹具();
        let 错误 = 删除资源(&mut 编辑器, &路径("assets/房屋.png")).expect_err("有引用必须拒绝删除");
        assert!(错误.contains("场景"), "错误：{错误}");
        assert!(
            编辑器
                .项目
                .as_ref()
                .unwrap()
                .绝对路径(&路径("assets/房屋.png"))
                .is_file()
        );
    }

    #[test]
    fn 无引用时可以删除() {
        let (_目录, mut 编辑器) = 场景夹具();
        删除资源(&mut 编辑器, &路径("assets/未用.png")).expect("无引用应当允许删除");
        assert!(
            !编辑器
                .项目
                .as_ref()
                .unwrap()
                .绝对路径(&路径("assets/未用.png"))
                .exists()
        );
    }
}
