//! 统一诊断中心的**应用层**：把真实构建输出、运行日志与 Agent 结果
//! 汇进同一个 [`诊断集合`]，并给界面一份只读快照。
//!
//! # 数据流
//!
//! ```text
//!   cargo --message-format=json ──→ 构建宿主报告 ──┐
//!   cargo run 的 stdout/stderr ──→ 运行管理器 ─────┼→ 诊断中心 ──→ 诊断快照 ──→ 界面
//!   Agent 任务结果 ──────────────→ 智能体状态 ─────┘
//! ```
//!
//! # 三条必须守住的纪律
//!
//! 1. **每个任务都有身份**：开始一次构建/运行就开一个新任务，
//!    界面默认只显示"当前任务"的诊断（[`诊断集合::当前视图`]）。
//!    这样用户改完代码重新构建时，不会看到上一轮的错误继续躺在列表里。
//! 2. **同一条诊断只进一次**：构建报告每帧都可能被读到，
//!    但报告里的诊断是**累积**的。这里按报告修订号增量采集，
//!    并且集合自己按内容去重（双重保险：转发链路将来再加一层也不会重复）。
//! 3. **普通 stderr 不是错误**：运行日志只有带明确错误标记的行
//!    （panic / `ERROR` / `error[E….` / 资源加载失败）才变成诊断，
//!    其余仍然是控制台日志。宁可漏判也不制造假错误。

use bevy::prelude::*;
use editor_core::build::{构建报告, 构建状态, 构建配置};
use editor_core::diagnostics::{
    从构建诊断, 任务标识, 任务类别, 解析运行日志, 诊断严重级别, 诊断条目, 诊断来源, 诊断集合,
};

/// 诊断快照类型在核心层定义（界面层也要读它）。
///
/// 这里 `pub use` 而不是 `use`：`editor_app::diagnostics::诊断快照`
/// 是对外路径，界面层与测试都按这个名字引用。
pub use editor_core::diagnostics::诊断快照;

/// 诊断中心资源。
#[derive(Resource)]
pub struct 诊断中心 {
    /// 诊断集合（权威数据）。
    pub 集合: 诊断集合,
    /// 界面同步用的修订号。
    修订: u64,
    /// 上一次看到的构建报告修订号（增量采集用）。
    上次构建修订: Option<u64>,
    /// 上一次看到的构建状态（判断"新的一次构建开始了"）。
    上次构建状态: 构建状态,
    /// 当前构建任务。
    当前构建任务: Option<任务标识>,
    /// 上一次看到的运行序号（运行历史里的序号，变化即"新的一次运行开始了"）。
    上次运行序号: Option<u64>,
    /// 当前运行任务。
    当前运行任务: Option<任务标识>,
    /// 上一次看到的 Agent 任务摘要（避免同一条结果反复入库）。
    上次智能体摘要: Option<String>,
    /// 等待下一行补全消息的 panic 诊断编号（Rust 的 panic 输出是两行）。
    待续诊断: Option<u64>,
}

impl Default for 诊断中心 {
    fn default() -> Self {
        Self {
            集合: 诊断集合::default(),
            修订: 0,
            上次构建修订: None,
            上次构建状态: 构建状态::空闲,
            当前构建任务: None,
            上次运行序号: None,
            当前运行任务: None,
            上次智能体摘要: None,
            待续诊断: None,
        }
    }
}

impl 诊断中心 {
    /// 修订号。
    pub fn 修订(&self) -> u64 {
        self.修订
    }

    fn 标记变化(&mut self) {
        self.修订 = self.修订.wrapping_add(1);
    }

    /// 换项目时清空。旧项目的诊断不属于新项目。
    pub fn 清空(&mut self) {
        self.集合.清空();
        self.上次构建修订 = None;
        self.上次构建状态 = 构建状态::空闲;
        self.当前构建任务 = None;
        self.上次运行序号 = None;
        self.当前运行任务 = None;
        self.上次智能体摘要 = None;
        self.待续诊断 = None;
        self.标记变化();
    }

    /// 让界面重新读一次快照。
    ///
    /// 诊断集合被**外部**改动时（例如"清空当前结果"直接操作 `集合`），
    /// 修订号不会自己跳，界面就会停在旧快照上。这个入口是那种情形的出口。
    pub fn 标记界面变化(&mut self) {
        self.标记变化();
    }

    /// 采集一次构建报告。
    ///
    /// `报告修订` 是 [`crate::build::构建宿主::修订`]：只有它变化时才做解析，
    /// 否则每帧都遍历一遍诊断列表。
    ///
    /// 开始新一次构建（状态从非运行中 → 运行中）会开启新的任务标识。
    pub fn 采集构建(
        &mut self, 报告: &构建报告, 配置: Option<&构建配置>, 报告修订: u64
    ) {
        let 之前状态 = self.上次构建状态;
        let 新构建 = 报告.状态 == 构建状态::运行中 && 之前状态 != 构建状态::运行中;
        if 新构建 {
            self.当前构建任务 = Some(self.集合.开启任务(任务类别::构建));
            self.标记变化();
        }
        self.上次构建状态 = 报告.状态;

        if self.上次构建修订 == Some(报告修订) {
            return;
        }
        self.上次构建修订 = Some(报告修订);

        // 构建刚刚结束：留一条历史记录（成功也算，"上次成功过"是重要信息）。
        if 之前状态 == 构建状态::运行中
            && 报告.状态.已结束()
            && let Some(任务) = self.当前构建任务
        {
            self.集合.记录构建(editor_core::diagnostics::构建记录 {
                任务,
                命令行: 报告.命令行.clone(),
                成功: 报告.状态 == 构建状态::成功,
                耗时毫秒: 报告.耗时毫秒,
                错误数: 报告.错误数(),
                警告数: 报告.警告数(),
                时间: 现在秒(),
            });
            self.标记变化();
        }

        let Some(任务) = self.当前构建任务 else {
            // 没有任务归属时也要能记录：例如"项目没有 Cargo.toml"是在
            // 开任务之前就失败的。这类诊断永远属于"当前结果"。
            let 来源 = 来源由子命令(配置);
            self.记录无任务诊断(报告, 来源);
            return;
        };
        let 来源 = 来源由子命令(配置);
        let 包 = 配置.and_then(|配置| 配置.包名.clone());
        let 目标 = 配置.and_then(|配置| 配置.二进制名.clone());
        let 时间 = 现在秒();
        let mut 变化 = false;
        for 诊断 in &报告.诊断 {
            变化 |= self.集合.追加(从构建诊断(
                来源,
                诊断,
                任务,
                包.clone(),
                目标.clone(),
                时间,
            ));
        }
        // 构建失败但没有任何结构化诊断时，失败原因（stderr 尾部）本身就是
        // 一条诊断 —— 否则用户看到"构建失败"，列表却是空的。
        if 报告.状态 == 构建状态::失败
            && 报告.诊断.is_empty()
            && let Some(摘要) = 报告.摘要文本()
        {
            let mut 条目 = 诊断条目::新建(来源, 诊断严重级别::错误, 摘要);
            条目.任务 = Some(任务);
            条目.发生时间 = 时间;
            条目.原始日志 = Some(摘要.to_string());
            变化 |= self.集合.追加(条目);
        }
        if 变化 {
            self.标记变化();
        }
    }

    fn 记录无任务诊断(&mut self, 报告: &构建报告, 来源: 诊断来源) {
        if 报告.诊断.is_empty() {
            if let Some(摘要) = 报告.摘要文本() {
                let mut 条目 = 诊断条目::新建(来源, 诊断严重级别::错误, 摘要);
                条目.发生时间 = 现在秒();
                条目.原始日志 = Some(摘要.to_string());
                if self.集合.追加(条目) {
                    self.标记变化();
                }
            }
            return;
        }
        let 时间 = 现在秒();
        let mut 变化 = false;
        for 诊断 in &报告.诊断 {
            // 没有任务时用 `None`：这类诊断在界面里始终属于当前结果。
            let mut 条目 =
                诊断条目::新建(来源, 诊断严重级别::从构建级别(诊断.级别), 诊断.消息.clone());
            条目.代码 = 诊断.代码.clone();
            条目.文件 = 诊断.位置.as_ref().and_then(|位置| 位置.文件.clone());
            条目.行 = 诊断.位置.as_ref().map(|位置| 位置.行).unwrap_or(0);
            条目.列 = 诊断.位置.as_ref().map(|位置| 位置.列).unwrap_or(0);
            条目.发生时间 = 时间;
            变化 |= self.集合.追加(条目);
        }
        if 变化 {
            self.标记变化();
        }
    }

    /// 采集一行运行日志。
    ///
    /// `运行序号` 是 [`editor_core::run_entry::运行历史`] 里最近记录的序号；
    /// 它变化就表示"又跑了一次"，于是开一个新的运行任务。
    pub fn 采集运行日志(&mut self, 行: &str, 运行序号: Option<u64>) {
        if 运行序号 != self.上次运行序号 {
            self.上次运行序号 = 运行序号;
            if 运行序号.is_some() {
                self.当前运行任务 = Some(self.集合.开启任务(任务类别::运行));
                self.标记变化();
            }
        }
        let Some(任务) = self.当前运行任务 else {
            return;
        };
        let 解析结果 = 解析运行日志(行, 诊断来源::运行, 任务, 现在秒());
        if 解析结果.is_none()
            && let Some(编号) = self.待续诊断.take()
        {
            // 续行是子进程的原始输出，可能带 `[stderr]` 前缀：
            // 拼进消息前去掉它，诊断详情里不该出现传输层标记。
            let 续行 = 行
                .trim()
                .strip_prefix("[stderr]")
                .or_else(|| 行.trim().strip_prefix("[stdout]"))
                .unwrap_or_else(|| 行.trim())
                .trim();
            if 续行.is_empty() {
                // 空行不结束续行状态：panic 输出之间可能夹着空行。
                self.待续诊断 = Some(编号);
            } else if self.集合.追加消息(编号, 续行) {
                self.标记变化();
            }
            return;
        }
        let Some(条目) = 解析结果 else {
            return;
        };
        // 位置后面直接是冒号 = 原因在下一行（Rust 的 panic 就是两行）。
        let 等待续行 = 条目.代码.as_deref() == Some("panic") && 条目.消息.trim_end().ends_with(':');
        if self.集合.追加(条目) {
            self.标记变化();
        }
        if 等待续行 {
            self.待续诊断 = self
                .集合
                .条目()
                .iter()
                .rev()
                .find(|条目| 条目.任务 == Some(任务) && 条目.代码.as_deref() == Some("panic"))
                .map(|条目| 条目.编号);
        }
    }

    /// 采集 Agent 任务的真实结果。
    ///
    /// 只在**失败/环境阻塞/达到上限**时记一条错误；成功不产生诊断
    /// （"修好了"不是问题）。
    pub fn 采集智能体(&mut self, 摘要: &str, 成功: bool) {
        let 摘要 = 摘要.trim();
        if 摘要.is_empty() {
            return;
        }
        if 成功 {
            // 成功结果不产生诊断，但要清掉"上一次失败"的去重记忆，
            // 让下一轮失败仍然能被记录。
            return;
        }
        if self.上次智能体摘要.as_deref() == Some(摘要) {
            return;
        }
        self.上次智能体摘要 = Some(摘要.to_string());
        let 任务 = self.集合.开启任务(任务类别::智能体);
        let mut 条目 = 诊断条目::新建(诊断来源::智能体, 诊断严重级别::错误, 摘要);
        条目.任务 = Some(任务);
        条目.发生时间 = 现在秒();
        if self.集合.追加(条目) {
            self.标记变化();
        }
    }

    /// 生成界面快照。
    pub fn 快照(&self) -> 诊断快照 {
        self.集合.生成快照()
    }
}

/// 诊断快照资源。
///
/// 快照类型本身在核心层（[`editor_core::diagnostics::诊断快照`]）：
/// 界面层也要读它，而 `editor_ui` 不依赖 `editor_app`。
#[derive(Resource, Default)]
pub struct 诊断快照资源 {
    /// 快照。
    pub 快照: 诊断快照,
    /// 上次同步的修订号。
    pub 修订: u64,
}

/// 由构建配置推出诊断来源。
fn 来源由子命令(配置: Option<&构建配置>) -> 诊断来源 {
    match 配置.map(|配置| 配置.子命令) {
        Some(editor_core::build::构建子命令::测试) => 诊断来源::测试,
        Some(editor_core::build::构建子命令::检查) => 诊断来源::检查,
        _ => 诊断来源::构建,
    }
}

/// 当前 UNIX 秒。
fn 现在秒() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|时长| 时长.as_secs())
        .unwrap_or(0)
}

/// 系统：把诊断中心的当前状态同步成界面快照。
///
/// 修订号没变就不克隆：`全部条目` 可能有上千条带字符串的诊断，
/// 每帧克隆在 60 FPS 下会变成稳定的每秒几 MB 分配。
pub fn 同步诊断快照(诊断: Res<诊断中心>, mut 快照: ResMut<诊断快照资源>) {
    if 快照.修订 == 诊断.修订() {
        return;
    }
    快照.快照 = 诊断.快照();
    快照.修订 = 诊断.修订();
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use editor_core::build::{
        代码位置, 构建报告, 构建状态, 构建诊断, 诊断级别
    };
    use std::path::PathBuf;

    fn 报告(状态: 构建状态, 诊断: Vec<构建诊断>) -> 构建报告 {
        let mut 报告 = 构建报告::开始("cargo build");
        报告.状态 = 状态;
        报告.诊断 = 诊断;
        报告
    }

    fn 编译错误() -> 构建诊断 {
        构建诊断 {
            级别: 诊断级别::错误,
            消息: "mismatched types".into(),
            代码: Some("E0308".into()),
            位置: Some(代码位置 {
                文件: Some(PathBuf::from("src/main.rs")),
                行: 7,
                列: 5,
            }),
        }
    }

    #[test]
    fn 构建诊断进入当前结果并能跳转() {
        let mut 中心 = 诊断中心::default();
        中心.采集构建(&报告(构建状态::运行中, vec![编译错误()]), None, 1);
        中心.采集构建(&报告(构建状态::失败, vec![编译错误()]), None, 2);
        let 快照 = 中心.快照();
        assert_eq!(快照.错误数, 1);
        assert_eq!(快照.条目.len(), 1);
        let 条目 = &快照.条目[0];
        assert_eq!(条目.代码.as_deref(), Some("E0308"));
        assert!(条目.可跳转());
        assert!(条目.任务.is_some(), "构建诊断必须归属任务");
    }

    #[test]
    fn 同一条诊断不会因为多次采集而重复() {
        let mut 中心 = 诊断中心::default();
        let 报告 = 报告(构建状态::失败, vec![编译错误()]);
        中心.采集构建(&报告, None, 1);
        中心.采集构建(&报告, None, 2);
        中心.采集构建(&报告, None, 3);
        assert_eq!(中心.快照().条目.len(), 1);
    }

    #[test]
    fn 新一次构建会开新任务且旧诊断不再是当前结果() {
        let mut 中心 = 诊断中心::default();
        中心.采集构建(&报告(构建状态::运行中, vec![编译错误()]), None, 1);
        中心.采集构建(&报告(构建状态::失败, vec![编译错误()]), None, 2);
        assert_eq!(中心.快照().错误数, 1);

        // 第二次构建开始（还没有任何诊断）。
        中心.采集构建(&报告(构建状态::运行中, vec![]), None, 3);
        let 快照 = 中心.快照();
        assert_eq!(快照.错误数, 0, "旧构建的错误不能混进新构建的结果");
        assert_eq!(快照.全部条目.len(), 1, "历史诊断仍然保留");
        assert_ne!(快照.构建任务.unwrap().序号, 1);
    }

    #[test]
    fn 构建失败但没有结构化诊断时给出可读原因() {
        let mut 报告 = 构建报告::开始("cargo build");
        报告.完成(false, 10);
        报告.摘要 = Some("error: could not find `Cargo.toml`".into());
        let mut 中心 = 诊断中心::default();
        中心.采集构建(&报告, None, 1);
        let 快照 = 中心.快照();
        assert_eq!(快照.错误数, 1);
        assert!(快照.条目[0].消息.contains("Cargo.toml"));
    }

    #[test]
    fn 运行日志里的panic成为可跳转诊断() {
        let mut 中心 = 诊断中心::default();
        中心.采集运行日志("thread 'main' panicked at src/main.rs:42:5: boom", Some(1));
        let 快照 = 中心.快照();
        assert_eq!(快照.错误数, 1);
        assert_eq!(快照.条目[0].行, 42);
        assert_eq!(快照.运行任务.unwrap().序号, 1);
    }

    #[test]
    fn 普通运行日志不产生诊断() {
        let mut 中心 = 诊断中心::default();
        for 行 in [
            "Compiling my_game v0.1.0",
            "Finished dev profile",
            "hello",
            "[stderr] warning: unused variable",
        ] {
            中心.采集运行日志(行, Some(1));
        }
        assert!(中心.快照().条目.is_empty());
    }

    #[test]
    fn 第二次运行会把上一次的运行时错误移出当前结果() {
        let mut 中心 = 诊断中心::default();
        中心.采集运行日志("thread 'main' panicked at src/main.rs:1:1: 第一次", Some(1));
        assert_eq!(中心.快照().错误数, 1);
        中心.采集运行日志("hello from run 2", Some(2));
        assert_eq!(
            中心.快照().错误数,
            0,
            "第二次运行还没有错误时，当前结果必须是空的"
        );
        assert_eq!(中心.快照().全部条目.len(), 1);
    }

    #[test]
    fn 智能体失败会记录成功不记录() {
        let mut 中心 = 诊断中心::default();
        中心.采集智能体("修复第 1 轮失败：error[E0308]", false);
        assert_eq!(中心.快照().错误数, 1);
        let 之前 = 中心.修订();
        中心.采集智能体("构建通过（退出码 0）", true);
        assert_eq!(中心.修订(), 之前, "成功结果不产生诊断");
        中心.采集智能体("修复第 1 轮失败：error[E0308]", false);
        assert_eq!(中心.快照().条目.len(), 1, "同一条失败不重复记录");
    }

    #[test]
    fn 清空后没有诊断() {
        let mut 中心 = 诊断中心::default();
        中心.采集构建(&报告(构建状态::失败, vec![编译错误()]), None, 1);
        中心.清空();
        let 快照 = 中心.快照();
        assert!(快照.条目.is_empty());
        assert_eq!(快照.错误数, 0);
    }
}
