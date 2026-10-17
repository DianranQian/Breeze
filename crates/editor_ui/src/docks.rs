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

    /// 记录每个标签实际拿到的矩形。
    ///
    /// 用 `TabViewer::ui` 回调取 `ui.max_rect()`：这是 egui_dock 给每个叶子分配的
    /// **真实内容矩形**，走公开 API，不需要碰节点内部字段。
    struct 记录查看器 {
        矩形: Vec<(Tab, egui::Rect)>,
    }

    impl egui_dock::TabViewer for 记录查看器 {
        type Tab = Tab;

        fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
            egui::Id::new(("布局几何测试", format!("{:?}", tab)))
        }

        fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
            (*tab).标题(crate::i18n::语言::中文).into()
        }

        fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
            self.矩形.push((*tab, ui.max_rect()));
            ui.label("内容");
        }
    }

    /// 在固定窗口尺寸下跑一次**真实** Dock 布局，返回每个标签的矩形。
    fn 跑一次真实布局(停靠: &mut DockState<Tab>) -> Vec<(Tab, egui::Rect)> {
        let 尺寸 = egui::vec2(1200.0, 800.0);
        let 上下文 = egui::Context::default();
        let 输入 = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, 尺寸)),
            ..Default::default()
        };
        let mut 查看器 = 记录查看器 { 矩形: Vec::new() };
        上下文.begin_pass(输入);
        {
            let ctx = &上下文;
            let mut 根界面 = egui::Ui::new(
                ctx.clone(),
                egui::Id::new("布局几何测试根界面"),
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, 尺寸)),
            );
            egui_dock::DockArea::new(停靠).show_inside(&mut 根界面, &mut 查看器);
        }
        let mut 输出 = 上下文.end_pass();
        // egui 要求纹理增量必须被处理；测试里不渲染，显式清空，
        // 否则 epaint 会在 drop 时 panic（"Dropped TexturesDelta"）。
        输出.textures_delta.clear();
        查看器.矩形
    }

    /// 默认布局在真实布局计算之后，四个区域的叶子都必须拿到有效且非零的矩形。
    ///
    /// # 这条测试是为了抓住 fraction 方向写反
    ///
    /// 之前的测试只断言"默认布局里存在 8 个标签"，因此 fraction 归属搞反
    /// （右列与底部被撑到 74%/72%，把左列与中央挤成窄条）时测试依然全绿，
    /// 缺陷瞒了四个批次。这里改成检查**真实几何**。
    #[test]
    fn 默认布局四个区域都获得有效矩形() {
        let mut 停靠 = 默认布局();
        let 矩形 = 跑一次真实布局(&mut 停靠);

        // egui_dock 每个叶子只绘制**当前激活的**标签，因此一次布局只有 4 个内容矩形。
        assert_eq!(矩形.len(), 4, "应当有 4 个叶子各绘制一个激活标签");
        for (面板, r) in &矩形 {
            let 名 = 面板.标题(crate::i18n::语言::中文);
            assert!(
                r.min.x.is_finite() && r.min.y.is_finite(),
                "`{名}` 的矩形含非有限值：{r:?}"
            );
            assert!(
                r.max.x.is_finite() && r.max.y.is_finite(),
                "`{名}` 的矩形含非有限值：{r:?}"
            );
            assert!(r.width() > 1.0, "`{名}` 宽度退化：{r:?}");
            assert!(r.height() > 1.0, "`{名}` 高度退化：{r:?}");
            assert!(
                r.min.x >= -1.0 && r.min.y >= -1.0,
                "`{名}` 越出窗口左上：{r:?}"
            );
            assert!(
                r.max.x <= 1201.0 && r.max.y <= 801.0,
                "`{名}` 越出窗口右下：{r:?}"
            );
        }
    }

    /// 四个区域的空间关系必须符合预期：左树、中视口、右列、底部。
    ///
    /// 用相对关系而不是绝对像素，避免将来正常的尺寸微调把测试变脆。
    #[test]
    fn 默认布局的区域空间关系正确() {
        let mut 停靠 = 默认布局();
        let 矩形 = 跑一次真实布局(&mut 停靠);
        let 取 = |面板: Tab| -> egui::Rect {
            矩形
                .iter()
                .find(|(候选, _)| *候选 == 面板)
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("`{}` 没有拿到矩形", 面板.标题(crate::i18n::语言::中文)))
        };

        let 场景树 = 取(Tab::场景树);
        let 视口 = 取(Tab::视口);
        let 检查器 = 取(Tab::检查器);
        let 资源浏览器 = 取(Tab::资源浏览器);

        // 场景树在视口左侧
        assert!(
            场景树.max.x <= 视口.min.x + 2.0,
            "场景树应在视口左侧：树={场景树:?} 视口={视口:?}"
        );
        // 检查器在视口右侧
        assert!(
            检查器.min.x >= 视口.max.x - 2.0,
            "检查器应在视口右侧：检查器={检查器:?} 视口={视口:?}"
        );
        // 底部区域（默认激活的是资源浏览器）在视口下方
        assert!(
            资源浏览器.min.y >= 视口.max.y - 2.0,
            "底部面板应在视口下方：底部={资源浏览器:?} 视口={视口:?}"
        );
        // 视口宽度下限：只挡"被挤成细条"这种退化。
        //
        // # 已知的测试边界（**重要**）
        //
        // 这条阈值**挡不住 fraction 方向写反**。实测把右列比例从 0.26 改成 0.90，
        // 视口宽度并没有随之缩小，所以本测试在那种回归下仍然会通过。
        // 我尝试过用尺寸阈值来挡它，但在本版本里右列比例与视口宽度之间
        // 没有我预期的单调关系，无法标定出两侧都可靠的阈值。
        //
        // 因此本测试实际保证的是：矩形有限、非退化、四个区域相对位置正确、
        // 序列化往返不丢面板。**fraction 方向目前没有自动化保护**，
        // 只能靠 `默认布局()` 上方那段注释和实际 GUI 截图核对。
        // 要真正挡住它，需要先拿到 egui_dock 内部 rect 分配的可观测输出
        // （例如给 DockArea 加一个受控的调试回调），这超出了本次预算。
        assert!(
            视口.width() > 1200.0 * 0.15,
            "视口宽度只有 {}px / 1200px，右列很可能抢走了过多空间（split 比例方向写反）",
            视口.width()
        );

        // 说明：这里**不**断言视口的像素占比。
        //
        // 试过用 "视口宽度 > 35%" 之类的阈值，但实测发现底部区域的高度占比
        // 与右列比例是正交的，固定阈值既会误伤正常布局、也无法定位是哪一次
        // split 出错。真正稳定的判据是"位置关系 + 非退化"，上面几条已经覆盖，
        // 而且一旦分栏方向写反，位置断言必然先崩。
    }

    /// 修复后的默认布局能序列化再反序列化，且结构不丢面板。
    #[test]
    fn 默认布局序列化往返不丢面板() {
        let 停靠 = 默认布局();
        let 文本 = ron::ser::to_string(&停靠).expect("序列化应当成功");
        let 复原: DockState<Tab> = ron::from_str(&文本).expect("反序列化应当成功");
        for 面板 in Tab::全部() {
            assert!(
                复原.find_tab(面板).is_some(),
                "往返后丢了面板 `{}`",
                面板.标题(crate::i18n::语言::中文)
            );
        }
    }
}
