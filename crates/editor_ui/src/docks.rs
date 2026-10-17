//! 停靠布局与面板定义。

use egui_dock::{DockState, NodeIndex};

/// 编辑器里的一个可停靠面板。
///
/// 用枚举而不是字符串标识：这样"新增面板"是一个编译器会强制处理的变更，
/// 不会出现"面板加进布局了但绘制分发忘了加分支"这种只在运行时暴露的问题。
///
/// 派生 `Serialize` / `Deserialize` 是为了把用户的布局持久化到磁盘。
/// 用枚举还有一个额外好处：布局文件里存的是 `场景树` 这样的**变体名**，
/// 而不是依赖声明顺序的索引 —— 将来在枚举中间插入一个新面板，
/// 也不会让旧布局文件里的面板错位成别的面板。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    /// 场景树。
    场景树,
    /// 属性检查器。
    检查器,
    /// 中央视口。
    视口,
    /// 资源浏览器。
    资源浏览器,
    /// 控制台。
    控制台,
    /// 撤销历史。
    历史,
    /// 运行时 ECS 浏览。
    运行时ecs,
    /// 性能统计。
    性能,
}

impl Tab {
    /// 面板标题（按当前语言）。
    ///
    /// 语言作为参数传入而不是读全局状态：标题是纯展示数据，
    /// 保持成纯函数才好写单元测试。
    pub fn 标题(&self, 语言: crate::i18n::语言) -> &'static str {
        语言.取(self.标题键())
    }

    /// 面板标签的词条键。
    ///
    /// 与 [`Tab::菜单键`] 分开：菜单里写的是"资源浏览器"，
    /// 而面板标签为了省宽度只写"资源"，两者的键不同。
    pub fn 标题键(&self) -> &'static str {
        match self {
            Tab::场景树 => "scene_tree",
            Tab::检查器 => "inspector",
            Tab::视口 => "viewport",
            Tab::资源浏览器 => "assets",
            Tab::控制台 => "console",
            Tab::历史 => "history",
            Tab::运行时ecs => "runtime_ecs",
            Tab::性能 => "performance",
        }
    }

    /// 标题的中文原文。
    ///
    /// 它同时是翻译表的键、以及「窗口」菜单里按文本反查面板的依据，
    /// 因此必须与表里的键逐字一致，不要改这里的字符串。
    pub fn 中文标题(&self) -> &'static str {
        match self {
            Tab::场景树 => "场景树",
            Tab::检查器 => "检查器",
            Tab::视口 => "视口",
            Tab::资源浏览器 => "资源",
            Tab::控制台 => "控制台",
            Tab::历史 => "历史",
            Tab::运行时ecs => "运行时 ECS",
            Tab::性能 => "性能",
        }
    }

    /// 该面板在「窗口」菜单里的词条键。
    ///
    /// 菜单定义与"文本 → 面板"的反映射都以它为准，
    /// 避免菜单文案一改就两边对不上。
    pub fn 菜单键(&self) -> &'static str {
        match self {
            Tab::场景树 => "scene_tree",
            Tab::检查器 => "inspector",
            Tab::视口 => "viewport",
            Tab::资源浏览器 => "asset_browser",
            Tab::控制台 => "console",
            Tab::历史 => "history",
            Tab::运行时ecs => "runtime_ecs",
            Tab::性能 => "performance",
        }
    }

    /// 是否可以关闭。
    ///
    /// 视口是编辑器的主体，关掉它用户就没有任何办法编辑场景了，
    /// 因此不允许关闭；其余面板都可以自由关闭再通过"窗口"菜单打开。
    pub fn 可关闭(&self) -> bool {
        !matches!(self, Tab::视口)
    }

    /// 所有面板，顺序即"窗口"菜单里的显示顺序。
    pub fn 全部() -> &'static [Tab] {
        &[
            Tab::场景树,
            Tab::检查器,
            Tab::视口,
            Tab::资源浏览器,
            Tab::控制台,
            Tab::历史,
            Tab::运行时ecs,
            Tab::性能,
        ]
    }
}

/// 默认布局。
///
/// 刻意模仿 Godot 的经典四象限：左侧场景树、右侧检查器、
/// 中央视口、底部资源与控制台。这是绝大多数用户已经形成的肌肉记忆，
/// 第一次打开就能找到东西在哪，不需要先学一套新布局。
pub fn 默认布局() -> DockState<Tab> {
    let mut 布局 = DockState::new(vec![Tab::视口]);
    let 主表面 = 布局.main_surface_mut();

    // 布局结构（自外向内）：
    //
    //   Horizontal(主区 74% | 右列 26%)          ← 检查器/历史/运行时ecs/性能
    //     └ Vertical(上方 72% | 底部 28%)        ← 资源浏览器/控制台
    //         └ Horizontal(视口 78% | 场景树 22%)
    //
    // # 两条必须记住的 egui_dock 语义
    //
    // 1. `fraction` 给的是**新节点**的比例（本次实测确认，见 docs/工作日志.md）。
    //    注意 egui_dock 0.21 的文档写的是"旧节点保留的比例"，与实测相反，
    //    别按文档改 —— 改错方向的表现是某块面板被撑到占满窗口。
    //
    // 2. `split_*` 返回 `[旧节点, 新节点]`，且**旧节点可能被换到别的索引**。
    //    因此后续 split 必须用返回的索引，不能反复写 `NodeIndex::root()`
    //    （它恒等于 `NodeIndex(0)`，只在第一次 split 前是根）。
    //
    // 顺序也必须是这样：先切出整条右列，再在"主区"里切出底部，
    // 最后在"上方"里切出左侧场景树。这样底部区域横跨左+中两列，
    // 与 Godot 一致。
    let [主区, _右列] = 主表面.split_right(
        NodeIndex::root(),
        0.26,
        vec![Tab::检查器, Tab::历史, Tab::运行时ecs, Tab::性能],
    );
    let [上方, _底部] = 主表面.split_below(主区, 0.28, vec![Tab::资源浏览器, Tab::控制台]);
    let [_视口, _场景树] = 主表面.split_left(上方, 0.22, vec![Tab::场景树]);

    布局
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 默认布局包含所有面板且视口不可关闭() {
        let 布局 = 默认布局();
        for 面板 in Tab::全部() {
            assert!(
                布局.find_tab(面板).is_some(),
                "面板 `{}` 应当出现在默认布局里",
                面板.标题(crate::i18n::语言::中文)
            );
        }
        assert!(!Tab::视口.可关闭(), "视口不允许关闭");
        assert!(Tab::控制台.可关闭());
    }

    #[test]
    fn 面板标题互不相同() {
        // 标题重名会让用户无法分辨两个标签页，也会让基于标题的定位失效。
        let mut 标题: Vec<&str> = Tab::全部()
            .iter()
            .map(|面板| 面板.标题(crate::i18n::语言::中文))
            .collect();
        let 原数量 = 标题.len();
        标题.sort_unstable();
        标题.dedup();
        assert_eq!(标题.len(), 原数量, "面板标题必须唯一");
    }
}
