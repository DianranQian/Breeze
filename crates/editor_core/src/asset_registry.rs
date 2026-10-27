//! 资源注册表：`assets/` 的统一扫描结果与缓存。
//!
//! # 为什么需要一层注册表
//!
//! 资源面板每帧都画。此前它每帧调用 [`项目描述::资源列表`]，而后者会递归
//! 遍历 `assets/` 并对每个文件读一次元数据。资源上千时，这部分开销全部
//! 落在绘制帧里，且项目根本没变的时候也在重复做同样的事。
//!
//! 注册表把"扫描磁盘"从绘制里挪出去：打开项目与用户显式刷新时才扫一次，
//! 面板只读这份快照。扫描结果带**代次**（generation）——每次刷新递增，
//! 界面据此判断缓存的目录树是否还能复用。
//!
//! # 与 UID 表的关系
//!
//! 条目里的 [`注册条目::唯一标识`] 直接读自 [`项目描述::资源标识`]，
//! 也就是 `.breeze/asset_uids.ron` 的内存投影。注册表**不生成**标识：
//! 谁负责分配 UID 仍然只有 [`项目描述::同步资源标识`] 一处，
//! 避免出现"扫描一次就悄悄改写标识表"的第二条通道。
//!
//! # 为什么不进入撤销栈
//!
//! 注册表是磁盘现状的只读推导，不属于场景文档。刷新它不改变任何编辑数据，
//! 因此不该占一个撤销步；面板按代次失效即可。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use scene_doc::path::项目相对路径;
use scene_doc::property::资源类别;
use uuid::Uuid;

use crate::project::{资源条目, 项目描述};

/// 注册表中的一个资源条目。
///
/// 与 [`资源条目`] 的区别只有两点：带上稳定标识与修改时间。
/// 修改时间用于回答"这个文件在两次刷新之间变过没有"，
/// 标识用于让按 UID 绑定的外部状态（选择、引用、缓存）有稳定的锚点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 注册条目 {
    /// 项目相对路径。
    pub 路径: 项目相对路径,
    /// 按扩展名推断的资源类别。
    pub 类别: 资源类别,
    /// 字节大小。
    pub 大小: u64,
    /// 文件最后修改时间；文件系统不提供时为空。
    pub 修改时间: Option<SystemTime>,
    /// 稳定标识；尚未进入 UID 表时为空。
    pub 唯一标识: Option<Uuid>,
}

impl 注册条目 {
    /// 从扫描条目与 UID 表构造注册条目。
    pub fn 新建(条目: 资源条目, 唯一标识: Option<Uuid>) -> Self {
        Self {
            路径: 条目.路径,
            类别: 条目.类别,
            大小: 条目.大小,
            修改时间: 条目.修改时间,
            唯一标识,
        }
    }
}

/// 一次刷新的结果摘要。
///
/// `新增 / 移除 / 变更` 只描述**注册表内容**的差异，不涉及 UID 表：
/// 后者由 [`项目描述::同步资源标识`] 单独负责。分开报告是为了让日志
/// 能如实说明"这次刷新到底发生了什么"，而不是只给出一个总数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct 资源刷新报告 {
    /// 刷新后的条目总数。
    pub 条目数: usize,
    /// 上次刷新后新出现的资源数。
    pub 新增: usize,
    /// 上次刷新后消失的资源数。
    pub 移除: usize,
    /// 路径仍在、但大小/类别/修改时间/UID 发生变化的资源数。
    pub 变更: usize,
    /// 本次刷新后的代次。
    pub 代次: u64,
}

/// `assets/` 的扫描结果缓存。
///
/// 默认状态（未打开项目）代次为 0、条目为空。每次 [`资源注册表::刷新`]
/// 或 [`资源注册表::清空`] 都会推进代次，界面因此不需要额外比较内容，
/// 只比较一个整数就能知道缓存是否过期。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 资源注册表 {
    条目: Vec<注册条目>,
    代次: u64,
    最近项目根: Option<PathBuf>,
}

impl 资源注册表 {
    /// 当前快照中的全部资源（按项目相对路径排序）。
    pub fn 条目(&self) -> &[注册条目] {
        &self.条目
    }

    /// 当前快照的代次。每次刷新或清空都会递增。
    pub fn 代次(&self) -> u64 {
        self.代次
    }

    /// 生成当前快照时对应的项目根目录。
    pub fn 最近项目根(&self) -> Option<&Path> {
        self.最近项目根.as_deref()
    }

    /// 当前快照的条目数量。
    pub fn 条目数(&self) -> usize {
        self.条目.len()
    }

    /// 按路径查找一个条目。
    pub fn 取(&self, 路径: &项目相对路径) -> Option<&注册条目> {
        self.条目.iter().find(|条目| 条目.路径 == *路径)
    }

    /// 按稳定标识查找一个条目。
    ///
    /// 资源选择器只拿到 `UiAction::设置资源引用 { 资源标识 }`，
    /// 路径必须由注册表反查。用标识而不是路径，是因为文件在外部被移动后
    /// 路径会失效，标识仍然指向同一份资源。
    pub fn 按标识(&self, 标识: Uuid) -> Option<&注册条目> {
        self.条目.iter().find(|条目| 条目.唯一标识 == Some(标识))
    }

    /// 重新扫描项目 `assets/` 目录并推进代次。
    ///
    /// 扫描本身复用 [`项目描述::资源列表`]，因此与导入、依赖检查看到的是
    /// 同一套文件枚举规则；这里只额外补上 UID 与修改时间。
    ///
    /// 返回报告供调用方写日志。扫描失败不会发生「整体清空」：
    /// [`项目描述::扫描目录`] 对读不出来的子目录选择跳过，注册表沿用同样的
    /// 语义 —— 一个被占用的目录不该让整个资源浏览器变成空白。
    pub fn 刷新(&mut self, 项目: &项目描述) -> 资源刷新报告 {
        let 旧表: BTreeMap<String, (u64, Option<SystemTime>, 资源类别, Option<Uuid>)> = self
            .条目
            .iter()
            .map(|条目| {
                (
                    条目.路径.文本().to_string(),
                    (条目.大小, 条目.修改时间, 条目.类别, 条目.唯一标识),
                )
            })
            .collect();

        let 新条目: Vec<注册条目> = 项目
            .资源列表()
            .into_iter()
            .map(|条目| {
                let 唯一标识 = 项目.资源标识().取(&条目.路径);
                注册条目::新建(条目, 唯一标识)
            })
            .collect();

        let 新表: BTreeMap<&str, &注册条目> =
            新条目.iter().map(|条目| (条目.路径.文本(), 条目)).collect();
        let 新增 = 新条目
            .iter()
            .filter(|条目| !旧表.contains_key(条目.路径.文本()))
            .count();
        let 移除 = 旧表
            .keys()
            .filter(|路径| !新表.contains_key(路径.as_str()))
            .count();
        let 变更 = 新条目
            .iter()
            .filter(|条目| {
                旧表
                    .get(条目.路径.文本())
                    .is_some_and(|旧| *旧 != (条目.大小, 条目.修改时间, 条目.类别, 条目.唯一标识))
            })
            .count();

        self.条目 = 新条目;
        self.代次 = self.代次.wrapping_add(1);
        self.最近项目根 = Some(项目.根目录().to_path_buf());

        资源刷新报告 {
            条目数: self.条目.len(),
            新增,
            移除,
            变更,
            代次: self.代次,
        }
    }

    /// 清空快照（关闭项目时调用）。
    ///
    /// 代次仍然递增而不是归零：界面的树缓存以「代次 + 项目根」为键，
    /// 归零会让"关闭 A、打开 B"在 B 的首次刷新前短暂命中 A 的旧缓存。
    pub fn 清空(&mut self) {
        let 下一次代次 = self.代次.wrapping_add(1);
        *self = Self {
            代次: 下一次代次,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::project::项目描述;
    use tempfile::TempDir;

    fn 造项目() -> (TempDir, 项目描述) {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 项目 = 项目描述::创建(目录.path().join("项目"), "注册表测试").expect("创建项目");
        项目.建目录("assets/贴图").expect("资源目录");
        std::fs::write(项目.根目录().join("assets/贴图/屋顶.png"), b"png!").expect("写图片");
        std::fs::write(项目.根目录().join("assets/背景.ogg"), b"ogg").expect("写音频");
        std::fs::write(项目.根目录().join("assets/说明.txt"), b"text").expect("写文本");
        (目录, 项目)
    }

    fn 路径(文本: &str) -> 项目相对路径 {
        项目相对路径::解析(文本).expect("路径")
    }

    fn 找<'a>(表: &'a 资源注册表, 文本: &str) -> &'a 注册条目 {
        表.取(&路径(文本))
            .unwrap_or_else(|| panic!("注册表里应存在 {文本}"))
    }

    #[test]
    fn 扫描出正确条目并缓存类别大小修改时间与标识() {
        let (_目录, mut 项目) = 造项目();
        let mut 表 = 资源注册表::default();
        let 报告 = 表.刷新(&mut 项目);

        assert_eq!(报告.条目数, 3, "assets/ 下应有三个文件");
        assert_eq!(报告.新增, 3, "首次刷新全部是新增");
        assert_eq!(报告.移除, 0);
        assert_eq!(报告.变更, 0);
        assert_eq!(报告.代次, 1);
        assert_eq!(表.代次(), 1);
        assert_eq!(表.最近项目根(), Some(项目.根目录()));

        let 图 = 找(&表, "assets/贴图/屋顶.png");
        assert_eq!(图.类别, 资源类别::图片);
        assert_eq!(图.大小, 4);
        assert!(图.修改时间.is_some(), "应当缓存文件修改时间");
        assert_eq!(
            图.唯一标识,
            项目.资源标识().取(&图.路径),
            "注册表的 UID 必须来自同一张标识表"
        );
        assert_eq!(找(&表, "assets/背景.ogg").类别, 资源类别::音频);
        assert_eq!(找(&表, "assets/说明.txt").类别, 资源类别::其他);
    }

    #[test]
    fn 材质文件的复合后缀被识别成材质类别() {
        let (_目录, mut 项目) = 造项目();
        项目.建目录("assets/materials").expect("材质目录");
        std::fs::write(
            项目.根目录().join("assets/materials/石板.material.ron"),
            "(名称: \"石板\")",
        )
        .expect("写材质");

        let mut 表 = 资源注册表::default();
        let 报告 = 表.刷新(&mut 项目);

        assert_eq!(
            报告.条目数, 4,
            "材质文件也在 assets/ 下，注册表不该额外排除它"
        );
        assert_eq!(
            找(&表, "assets/materials/石板.material.ron").类别,
            资源类别::材质,
            "只看最后一段扩展名会把 `.material.ron` 归成「其他」，材质页签永远空"
        );
        assert_eq!(
            项目.材质列表(),
            vec![路径("assets/materials/石板.material.ron")],
            "材质列表与注册表必须共用同一套类别判定"
        );
    }

    #[test]
    fn 修改文件后刷新推进代次并更新内容() {
        let (_目录, mut 项目) = 造项目();
        let mut 表 = 资源注册表::default();
        表.刷新(&mut 项目);
        let 第一次代次 = 表.代次();

        std::fs::write(项目.根目录().join("assets/说明.txt"), [7u8; 14]).expect("改文件");
        let 报告 = 表.刷新(&mut 项目);

        assert_eq!(报告.代次, 第一次代次 + 1, "刷新必须推进代次");
        assert_eq!(报告.条目数, 3, "条目数量不变");
        assert_eq!(报告.新增, 0);
        assert_eq!(报告.移除, 0);
        assert_eq!(报告.变更, 1, "只有被改写的文件算变更");
        let 文本 = 找(&表, "assets/说明.txt");
        assert_eq!(文本.大小, 14);
    }

    #[test]
    fn 新增与删除文件在下一次刷新体现() {
        let (_目录, mut 项目) = 造项目();
        let mut 表 = 资源注册表::default();
        表.刷新(&mut 项目);

        std::fs::write(项目.根目录().join("assets/新增.png"), b"a").expect("写新资源");
        std::fs::remove_file(项目.根目录().join("assets/背景.ogg")).expect("删资源");
        let 报告 = 表.刷新(&mut 项目);

        assert_eq!(报告.条目数, 3);
        assert_eq!(报告.新增, 1);
        assert_eq!(报告.移除, 1);
        assert_eq!(报告.变更, 0);
        assert!(表.取(&路径("assets/新增.png")).is_some());
        assert!(表.取(&路径("assets/背景.ogg")).is_none());
    }

    #[test]
    fn 按标识可以反查路径且未知标识返回空() {
        let (_目录, mut 项目) = 造项目();
        项目.同步资源标识().expect("同步标识");
        let mut 表 = 资源注册表::default();
        表.刷新(&项目);

        let 图 = 找(&表, "assets/贴图/屋顶.png");
        let 标识 = 图.唯一标识.expect("同步后每个资源都应拿到标识");
        assert_eq!(
            表.按标识(标识).map(|条目| 条目.路径.clone()),
            Some(图.路径.clone()),
            "选择器只带标识，必须能反查到路径"
        );
        assert!(表.按标识(Uuid::nil()).is_none(), "未知标识不能命中任何条目");
    }

    #[test]
    fn 清空会丢弃条目并推进代次() {
        let (_目录, mut 项目) = 造项目();
        let mut 表 = 资源注册表::default();
        表.刷新(&mut 项目);
        let 代次 = 表.代次();

        表.清空();

        assert!(表.条目().is_empty());
        assert_eq!(表.最近项目根(), None);
        assert_eq!(表.代次(), 代次 + 1, "清空也要让界面缓存失效");
    }
}
