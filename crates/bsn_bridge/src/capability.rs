//! 官方 BSN 能力边界：一份**可被测试校验的数据**，而不是散落在文档里的说法。
//!
//! # 为什么要把它写成数据
//!
//! 一个"当前版本支持 / 不支持"的清单，如果只写在 Markdown 里，就会随版本升级
//! 悄悄过期，然后被人当成事实引用。这里把它写成 Rust 数据，并加了三道保险：
//!
//! 1. [`目标Bevy版本`] 与 `Cargo.lock` 里真实锁定的版本做**编译期比对**
//!    （见本文件底部测试 `能力清单与锁定版本一致`），升级 Bevy 时测试会立刻失败，
//!    强迫维护者重新核对每一条；
//! 2. 每一条都带 [`能力条目::证据`]，指向具体的源码位置或测试名，
//!    读到条目的人可以直接去验证，而不是相信结论；
//! 3. 测试固定住几条最关键的事实（尤其是"官方 `.bsn` 文件尚未提供"），
//!    任何一条被改掉都必须显式修改测试，改动不可能悄悄发生。
//!
//! # 五个必须区分的层次
//!
//! 任务要求区分五种容易被混为一谈的东西。它们的对应关系是：
//!
//! | 层次 | 本项目的对应物 | 是否存在 |
//! | --- | --- | --- |
//! | Rust 代码里的 `bsn!` / `bsn_list!` | 编译期宏，由 `bevy_scene_macros` 展开 | 存在 |
//! | 官方场景构建与实例化 API | `World::spawn_scene` / `queue_spawn_scene` | 存在 |
//! | 项目现有的 `scene_doc` | `crates/scene_doc`（不依赖 Bevy） | 项目自有 |
//! | 当前场景保存文件的真实格式 | RON 文本，扩展名 `.scene.ron` | 项目自有 |
//! | 官方 `.bsn` 文件格式与加载器 | — | **不存在** |
//!
//! 最后一行是本次接入最重要的结论：Bevy 0.20.0 的 `bevy_scene` 源码里明确写着
//! “Bevy does not currently have support for `.bsn` files”，`bsn!` 宏的语法表里
//! `:"scene.bsn"` 一行标注为“Asset format not yet implemented!”。

/// 本次接入核对所针对的 Bevy 版本。
///
/// 它不是从 `Cargo.toml` 自动推导的，而是**人工核对产出的结论**；
/// 测试会拿它和 `Cargo.lock` 里真实的锁定版本比对，从而在升级时报警。
#[allow(
    non_upper_case_globals,
    reason = "项目约定自定义标识符用中文命名；`Bevy` 是外部固定标识符，保留原名"
)]
pub const 目标Bevy版本: &str = "0.20.0";

/// 一项能力在当前锁定版本下的支持状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum 能力状态 {
    /// 官方已提供，且本适配层已用真实运行验证过可用。
    官方已提供,
    /// 官方提供了底层机制，但要让编辑器用起来还需要项目自行适配。
    需项目适配,
    /// 当前版本官方明确未提供，项目也**没有**自行伪造一个同名的替代品。
    官方未提供,
}

impl 能力状态 {
    /// 稳定文本，用于诊断输出与测试断言。
    pub fn 文本(&self) -> &'static str {
        match self {
            能力状态::官方已提供 => "官方已提供",
            能力状态::需项目适配 => "需项目适配",
            能力状态::官方未提供 => "官方未提供",
        }
    }
}

/// 能力清单里的一个条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct 能力条目 {
    /// 稳定键，英文小写下划线，供界面与测试引用。
    pub 键: &'static str,
    /// 面向开发者的中文描述。
    pub 说明: &'static str,
    /// 当前状态。
    pub 状态: 能力状态,
    /// 证据：具体源码位置或测试名。**不允许为空**，由测试保证。
    pub 证据: &'static str,
}

/// 官方 BSN 能力清单（Bevy 0.20.0）。
///
/// 顺序是刻意安排的：先列"确实能用"的，再列"要自己适配"的，
/// 最后列"官方还没有"的，读起来就是一条边界线。
pub const 能力清单: &[能力条目] = &[
    // ---------- 1. Rust 代码中的 bsn! / bsn_list! ----------
    能力条目 {
        键: "bsn_macro",
        说明: "在 Rust 源码里用 `bsn!` 写编译期场景描述，由宏展开成 `impl Scene`",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene-0.20.0/src/lib.rs:923 `pub use bevy_scene_macros::bsn`；本 crate tests/官方场景原型.rs",
    },
    能力条目 {
        键: "bsn_list_macro",
        说明: "用 `bsn_list!` 生成多个根实体的场景列表，各根实体共享一个名字作用域",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene-0.20.0/src/lib.rs:925；本 crate `BSN宿主::实例化列表`",
    },
    能力条目 {
        键: "bsn_expressions",
        说明: "`bsn!` 的值位置可以放任意 Rust 表达式（`{expr}`）与变量，因此能由运行时数据驱动",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene_macros-0.20.0/src/lib.rs 语法表 `{ 1 + 2 }`；本 crate `模板::带运行时参数模板`",
    },
    能力条目 {
        键: "bsn_name_syntax",
        说明: "`#名字` 语法自动插入 `Name` 组件，并在同一 `bsn!` 作用域内提供实体引用",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene-0.20.0/src/lib.rs 模块文档“Named Entity References”",
    },
    能力条目 {
        键: "bsn_relationships",
        说明: "`Children [...]` 用 `--` 分隔实体构造层级，任意 `RelationshipTarget` 同理",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene-0.20.0/src/lib.rs 模块文档“Entity Hierarchies and Relationships”",
    },
    能力条目 {
        键: "bsn_patching",
        说明: "`组件 { 字段: 值 }` 是**补丁**而非整体替换，未提到的字段保留原值或默认值",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene-0.20.0/src/lib.rs 模块文档“Patching”",
    },
    能力条目 {
        键: "bsn_scene_component",
        说明: "`#[derive(SceneComponent)]` + `#[scene(...)]` 把场景封装成可复用、可传 props 的组件",
        状态: 能力状态::需项目适配,
        证据: "bevy_scene-0.20.0/src/lib.rs 模块文档“Scene Components”；本项目尚未定义任何 SceneComponent",
    },
    // ---------- 2. 官方场景实例化 API ----------
    能力条目 {
        键: "spawn_scene_immediate",
        说明: "`World::spawn_scene(bsn!{..})` 立即解析并实例化，返回根实体；依赖未加载时返回错误",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene-0.20.0/src/spawn.rs:182；本 crate `BSN宿主::实例化`",
    },
    能力条目 {
        键: "spawn_scene_queued",
        说明: "`queue_spawn_scene` 在依赖加载完成后于 `SpawnScene` 调度里实例化，需要跑 App 更新",
        状态: 能力状态::官方已提供,
        证据: "bevy_scene-0.20.0/src/spawn.rs:189 与 src/lib.rs:933 的插件装配",
    },
    能力条目 {
        键: "headless_instantiation",
        说明: "实例化只需要 `TaskPoolPlugin` + `AssetPlugin` + `ScenePlugin`，不需要窗口与 GPU",
        状态: 能力状态::官方已提供,
        证据: "本 crate `BSN宿主::新建` 与全部测试均无窗口运行",
    },
    能力条目 {
        键: "resolved_scene_inspection",
        说明: "`ResolvedSceneRoot::resolve` 给出可检查的中间表示（擦除模板），便于诊断",
        状态: 能力状态::需项目适配,
        证据: "bevy_scene-0.20.0/src/resolved_scene.rs:20；擦除模板无类型化读取接口，本项目改用实例化后回读 ECS",
    },
    // ---------- 3. 官方 .bsn 文件资产格式 ----------
    能力条目 {
        键: "bsn_asset_format",
        说明: "官方 `.bsn` 文件资产格式与 `AssetLoader`",
        状态: 能力状态::官方未提供,
        证据: "bevy_scene-0.20.0/src/lib.rs:852-855 “Bevy does not currently have support for `.bsn` files, but intends to offer a `.bsn` asset format in future releases.”",
    },
    能力条目 {
        键: "bsn_asset_include",
        说明: "`bsn!` 里用 `:\"scene.bsn\"` 引用外部场景文件",
        状态: 能力状态::官方未提供,
        证据: "bevy_scene_macros-0.20.0/src/lib.rs 语法表该行标注“Asset format not yet implemented!”",
    },
    能力条目 {
        键: "bsn_scene_caching",
        说明: "`:scene()` / `:MySceneComp` 复用已缓存的场景函数",
        状态: 能力状态::官方未提供,
        证据: "bevy_scene_macros-0.20.0/src/lib.rs 语法表标注“Caching for scene includes not yet implemented!”",
    },
    能力条目 {
        键: "bsn_asset_hot_reload",
        说明: "`.bsn` 文件热重载",
        状态: 能力状态::官方未提供,
        证据: "依赖 `bsn_asset_format`；格式本身尚不存在，谈不上热重载",
    },
    // ---------- 4. 运行时解析与动态能力 ----------
    能力条目 {
        键: "runtime_string_parsing",
        说明: "在运行时把任意 `.bsn` / `bsn!` 文本解析成场景",
        状态: 能力状态::官方未提供,
        证据: "`bsn!` 是 proc macro，在编译期展开；0.20.0 未提供场景文本解析器",
    },
    能力条目 {
        键: "dynamic_component_registration",
        说明: "把运行时才知道的组件类型动态注册进场景系统",
        状态: 能力状态::需项目适配,
        证据: "组件须在 Rust 类型系统里存在并实现 `Component`；项目只能用编译期已知的组件集合",
    },
    能力条目 {
        键: "component_name_enumeration",
        说明: "按名字枚举实体上已有的组件名",
        状态: 能力状态::需项目适配,
        证据: "`ComponentInfo::name()` 只在 `bevy_utils/debug` 特性开启时返回真实名字，本项目未开启；适配层改用 TypeId 白名单匹配",
    },
    能力条目 {
        键: "identifier_case_heuristic",
        说明: "`bsn!` 用 `char::is_uppercase()` 判断路径是类型还是值，因此中文命名的组件类型不能以 `类型 { 字段: 值 }` 形式书写",
        状态: 能力状态::需项目适配,
        证据: "bevy_macro_utils-0.20.0/src/path_type.rs:30 `last_ident_first_char.is_uppercase()`；中文无大小写→被判为函数值。绕法见 tests/标识符约束.rs",
    },
    // ---------- 5. 项目自有层（对照项，非官方能力） ----------
    能力条目 {
        键: "project_scene_doc",
        说明: "项目的 `scene_doc` 文档模型：稳定 UUID 身份、父指针层级、编辑器元数据、未知字段保留",
        状态: 能力状态::需项目适配,
        证据: "crates/scene_doc/src/document.rs；它与 Bevy 无依赖，因此升级引擎不破坏旧场景",
    },
    能力条目 {
        键: "project_scene_file_format",
        说明: "当前场景文件格式：RON 美化文本，扩展名 `.scene.ron`，`格式版本` 前缀",
        状态: 能力状态::需项目适配,
        证据: "crates/scene_doc/src/storage.rs `默认场景扩展名` 与 `编码场景`",
    },
];

/// 按稳定键查一条能力。
pub fn 查能力(键: &str) -> Option<&'static 能力条目> {
    能力清单.iter().find(|条目| 条目.键 == 键)
}

/// 列出处于某状态的全部能力。
pub fn 按状态筛选(状态: 能力状态) -> Vec<&'static 能力条目> {
    能力清单.iter().filter(|条目| 条目.状态 == 状态).collect()
}

/// 面向用户的边界摘要（一行一句）。
pub fn 边界摘要() -> Vec<String> {
    let mut 结果 = Vec::new();
    for 状态 in [
        能力状态::官方已提供,
        能力状态::需项目适配,
        能力状态::官方未提供,
    ] {
        let 条目 = 按状态筛选(状态);
        结果.push(format!("【{}】共 {} 项：", 状态.文本(), 条目.len()));
        for 项 in 条目 {
            结果.push(format!("  - {}：{}", 项.键, 项.说明));
        }
    }
    结果
}

/// 从 `Cargo.lock` 文本里解析某个包的锁定版本。
///
/// 只做最小解析：找到 `name = "<包名>"` 那一行，然后往下找最近的 `version = "..."`。
/// 不引入 TOML 依赖——这里只需要一个字符串，引入解析器反而增加升级面。
fn 解析锁定版本(清单: &str, 包名: &str) -> Option<String> {
    let 行: Vec<&str> = 清单.lines().collect();
    let 目标 = format!("name = \"{包名}\"");
    for 下标 in 0..行.len() {
        if 行[下标].trim() != 目标 {
            continue;
        }
        // 往后找 version，遇到新的 [[package]] 就停，避免跨包误读。
        for 后续 in &行[下标 + 1..] {
            let 裁剪 = 后续.trim();
            if 裁剪.starts_with("[[") {
                break;
            }
            if let Some(剩余) = 裁剪.strip_prefix("version = ") {
                return Some(剩余.trim().trim_matches('"').to_string());
            }
        }
        return None;
    }
    None
}

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 工作区根目录的 `Cargo.lock`，编译期嵌入。
    const 锁定清单: &str = include_str!("../../../Cargo.lock");

    #[test]
    fn 能力清单与锁定版本一致() {
        let 锁定 = 解析锁定版本(锁定清单, "bevy").expect("Cargo.lock 必须包含 bevy 的锁定版本");
        assert_eq!(
            锁定, 目标Bevy版本,
            "Bevy 版本已升级到 {锁定}，但能力清单仍按 {目标Bevy版本} 编写。\
             请重新核对 bevy_scene 源码与本模块每一条能力的状态与证据，再更新 `目标Bevy版本`。"
        );
    }

    #[test]
    fn 每条能力都有证据且键唯一() {
        let mut 已见 = std::collections::BTreeSet::new();
        for 条目 in 能力清单 {
            assert!(
                !条目.证据.trim().is_empty(),
                "能力 `{}` 缺少证据；没有证据的结论不应进清单",
                条目.键
            );
            assert!(已见.insert(条目.键), "能力键 `{}` 重复", 条目.键);
            // 键必须是英文小写下划线，避免和界面词条键混淆。
            assert!(
                条目
                    .键
                    .chars()
                    .all(|字符| 字符.is_ascii_lowercase() || 字符 == '_'),
                "能力键 `{}` 必须是小写下划线形式",
                条目.键
            );
        }
    }

    /// 本次接入最核心的一条结论：官方 `.bsn` 文件格式**不存在**。
    ///
    /// 这个测试刻意写得"很硬"：它固定住状态值与证据里的关键字符串。
    /// 未来 Bevy 真的发布了 `.bsn` 加载器时，这里会失败，
    /// 从而强制走一遍"是否值得全面采用"的准入评估（见 docs 计划文档）。
    #[test]
    fn 官方_bsn_文件格式在_0_20_0_仍未提供() {
        let 条目 = 查能力("bsn_asset_format").expect("必须记录 .bsn 资产格式的状态");
        assert_eq!(条目.状态, 能力状态::官方未提供);
        assert!(
            条目.证据.contains("does not currently have support"),
            "证据必须引用官方源码里的原话，便于他人复核"
        );

        let 引用条目 = 查能力("bsn_asset_include").expect("必须记录 :\"scene.bsn\" 的状态");
        assert_eq!(引用条目.状态, 能力状态::官方未提供);
        assert!(引用条目.证据.contains("Asset format not yet implemented"));
    }

    #[test]
    fn 五个层次都有对应条目() {
        // 任务要求区分的五种东西，逐一确认清单里能找到。
        for 键 in [
            "bsn_macro",                 // 1. bsn! / bsn_list!
            "spawn_scene_immediate",     // 2. 官方构建与实例化 API
            "project_scene_doc",         // 3. 项目 scene_doc
            "project_scene_file_format", // 4. 当前保存格式
            "bsn_asset_format",          // 5. 官方 .bsn 文件格式
        ] {
            assert!(查能力(键).is_some(), "能力清单缺少 `{键}`");
        }
    }

    #[test]
    fn 边界摘要覆盖全部条目() {
        let 摘要 = 边界摘要();
        let 条目行数 = 摘要
            .iter()
            .filter(|行| 行.trim_start().starts_with("- "))
            .count();
        assert_eq!(条目行数, 能力清单.len(), "摘要必须列出每一条能力");
    }

    #[test]
    fn 版本解析能处理多包清单() {
        let 样例 = "\
[[package]]
name = \"aaa\"
version = \"1.2.3\"

[[package]]
name = \"bevy\"
version = \"9.9.9\"
source = \"registry\"

[[package]]
name = \"bevy_scene\"
version = \"0.0.0\"
";
        assert_eq!(解析锁定版本(样例, "aaa").as_deref(), Some("1.2.3"));
        assert_eq!(解析锁定版本(样例, "bevy").as_deref(), Some("9.9.9"));
        assert_eq!(解析锁定版本(样例, "bevy_scene").as_deref(), Some("0.0.0"));
        assert_eq!(解析锁定版本(样例, "不存在"), None);
    }
}
