//! BSN 宿主：一个**无窗口、无 GPU** 的 Bevy 世界，专门用来实例化官方 BSN 场景。
//!
//! # 为什么需要一个宿主而不是直接 `World::new()`
//!
//! 官方 `World::spawn_scene` 的实现（`bevy_scene-0.20.0/src/spawn.rs:182`）是：
//!
//! ```text
//! let assets = self.resource::<AssetServer>();          // 直接取出，缺了就 panic
//! let mut patch = ScenePatch::load(assets, scene);
//! patch.resolve(assets, self.resource::<Assets<ScenePatch>>())?;
//! patch.spawn(self)
//! ```
//!
//! 也就是说它依赖两个资源：`AssetServer` 与 `Assets<ScenePatch>`。
//! 裸 `World::new()` 会直接 panic。这里把这三个插件装配收在一处，
//! 并在调用官方 API **之前**先检查资源是否齐备 —— 把 panic 变成可处理的错误。
//!
//! # 为什么不需要 `app.update()`
//!
//! `spawn_scene` 是**立即**路径：解析与实例化在同一个调用里完成，
//! 不经过 `SpawnScene` 调度。因此在测试与工具里可以只构造 `App`、
//! 取 `world_mut()` 直接调用，不必跑帧循环。
//!
//! 需要等待资源加载时才需要 `queue_spawn_scene` + 帧循环；本项目当前
//! 的原型场景不含外部资源依赖（见 [`crate::capability`] 的
//! `spawn_scene_queued` 条目），所以没有采用排队路径。

use bevy::app::TaskPoolPlugin;
use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::scene::{ScenePatch, ScenePlugin};

use crate::error::BSN错误;

/// 实例化官方 BSN 场景的宿主世界。
pub struct BSN宿主 {
    应用: App,
}

impl Default for BSN宿主 {
    fn default() -> Self {
        Self::新建()
    }
}

impl BSN宿主 {
    /// 新建一个空宿主。
    ///
    /// 只装配实例化 BSN 所必需的三个插件：
    /// `TaskPoolPlugin`（`AssetPlugin` 依赖它）、`AssetPlugin`（提供 `AssetServer`）、
    /// `ScenePlugin`（注册 `Assets<ScenePatch>` 与场景调度）。
    ///
    /// **刻意不装配** `DefaultPlugins`：那会引入窗口、渲染器与音频，
    /// 在没有显卡的 CI 里跑不起来，而 BSN 的实例化与这些无关。
    pub fn 新建() -> Self {
        let mut 应用 = App::new();
        应用.add_plugins((
            TaskPoolPlugin::default(),
            AssetPlugin::default(),
            ScenePlugin,
        ));
        Self { 应用 }
    }

    /// 只读世界。
    pub fn 世界(&self) -> &World {
        self.应用.world()
    }

    /// 可变世界。用于把场景实例化到调用方自己装配好的世界里。
    pub fn 世界可变(&mut self) -> &mut World {
        self.应用.world_mut()
    }

    /// 立即把一个官方 BSN 场景实例化到宿主世界，返回根实体。
    ///
    /// 若宿主的 `World` 恰好被替换过而缺少依赖资源，返回
    /// [`BSN错误::世界缺少资源`] 而不是让官方 API panic。
    pub fn 实例化<S: Scene>(&mut self, 场景: S) -> Result<Entity, BSN错误> {
        在世界上实例化(self.应用.world_mut(), 场景)
    }

    /// 立即实例化一个 BSN 场景列表，返回每个根实体的标识。
    pub fn 实例化列表<L: SceneList>(&mut self, 列表: L) -> Result<Vec<Entity>, BSN错误> {
        let 世界 = self.应用.world_mut();
        确保依赖就绪(世界)?;
        世界
            .spawn_scene_list(列表)
            .map_err(|来源| BSN错误::实例化失败 {
                来源: format!("{来源}"),
            })
    }

    // ------------------------------------------------------------------
    // 只读查询辅助
    //
    // 下面的方法只服务于"验证 BSN 到底产出了什么"。它们刻意都返回
    // `Option` / 空集合而不是 panic：一个不存在的实体应该让断言失败，
    // 而不是让测试进程异常退出、丢掉后面的检查。
    // ------------------------------------------------------------------

    /// 世界里**活着的**实体数量。
    ///
    /// # 这个数字不能当作"场景里有几个实体"
    ///
    /// 装配插件本身就会留下实体（资产系统与观察者的内部实体），
    /// 所以这里的返回值远大于模板规模。要数场景实体，请数 `Children`
    /// 子树，或者用 [`crate::extract`] 得到的文档实体数。
    /// 底部测试 `场景规模按层级数而不是按世界实体数` 固定住这个认知。
    pub fn 实体总数(&self) -> usize {
        self.应用.world().iter_entities().count()
    }

    /// 实体是否仍然存在。
    pub fn 存在(&self, 实体: Entity) -> bool {
        self.应用.world().get_entity(实体).is_ok()
    }

    /// 读取实体的 `Name` 组件文本。
    pub fn 名称(&self, 实体: Entity) -> Option<String> {
        self.应用
            .world()
            .get::<Name>(实体)
            .map(|名字| 名字.as_str().to_string())
    }

    /// 读取实体的 `Transform`。
    pub fn 变换(&self, 实体: Entity) -> Option<Transform> {
        self.应用.world().get::<Transform>(实体).copied()
    }

    /// 读取实体的子实体列表（Bevy 的 `Children` 是**有序**的）。
    pub fn 子实体(&self, 实体: Entity) -> Vec<Entity> {
        self.应用
            .world()
            .get::<Children>(实体)
            .map(|孩子| 孩子.to_vec())
            .unwrap_or_default()
    }

    /// 读取实体的父实体（Bevy 0.20 用子实体上的 `ChildOf` 表示）。
    pub fn 父实体(&self, 实体: Entity) -> Option<Entity> {
        self.应用
            .world()
            .get::<ChildOf>(实体)
            .map(|父关系| 父关系.parent())
    }

    /// 实体上是否存在某个组件类型。
    pub fn 含组件<T: Component>(&self, 实体: Entity) -> bool {
        self.应用.world().get::<T>(实体).is_some()
    }

    /// 读取实体上的某个组件。
    pub fn 取组件<T: Component>(&self, 实体: Entity) -> Option<&T> {
        self.应用.world().get::<T>(实体)
    }

    /// 实体上挂着的组件个数（含 Bevy 自动补齐的必需组件）。
    pub fn 组件数(&self, 实体: Entity) -> usize {
        self.应用
            .world()
            .inspect_entity(实体)
            .map(|迭代| 迭代.count())
            .unwrap_or(0)
    }
}

/// 在一个**已装配好**的世界里实例化 BSN 场景。
///
/// 这是 `BSN宿主::实例化` 与所有模板共用的入口：先检查依赖资源，
/// 再调用官方 API。这样"缺少资源"永远表现为可处理的错误。
pub fn 在世界上实例化<S: Scene>(世界: &mut World, 场景: S) -> Result<Entity, BSN错误> {
    确保依赖就绪(世界)?;
    世界
        .spawn_scene(场景)
        .map(|根| 根.id())
        .map_err(|来源| BSN错误::实例化失败 {
            来源: format!("{来源}"),
        })
}

/// 检查官方 `spawn_scene` 需要而调用方可能没准备好的资源。
///
/// # 为什么这项检查不能省
///
/// 官方实现里 `self.resource::<AssetServer>()` 是**不可失败**的取值：
/// 资源不存在就直接 panic。对一个"库"来说，把 panic 泄漏给调用方是
/// 不可接受的 —— 编辑器里一次错误的模板调用不应该让整个进程崩掉。
fn 确保依赖就绪(世界: &mut World) -> Result<(), BSN错误> {
    if !世界.contains_resource::<AssetServer>() {
        return Err(BSN错误::世界缺少资源 {
            缺少: "AssetServer",
        });
    }
    if !世界.contains_resource::<Assets<ScenePatch>>() {
        return Err(BSN错误::世界缺少资源 {
            缺少: "Assets<ScenePatch>",
        });
    }
    Ok(())
}

#[cfg(test)]
mod 测试 {
    use super::*;
    use crate::template::{BSN场景模板, 基础关卡模板};

    #[test]
    fn 宿主装配了实例化所需资源() {
        let 宿主 = BSN宿主::新建();
        assert!(宿主.世界().contains_resource::<AssetServer>());
        assert!(宿主.世界().contains_resource::<Assets<ScenePatch>>());
    }

    /// 没有插件的裸世界必须得到错误，而不是 panic。
    ///
    /// 这条测试守住的是"库不该把官方 API 的 panic 泄漏给调用方"。
    #[test]
    fn 裸世界给出错误而不是崩溃() {
        let 结果 = 确保依赖就绪(&mut World::new());
        match 结果 {
            Err(BSN错误::世界缺少资源 { 缺少 }) => assert_eq!(缺少, "AssetServer"),
            其他 => panic!("期望缺少资源的错误，实际：{其他:?}"),
        }
    }

    /// 插件自身会在世界里留下实体（资产系统与观察者的内部实体），
    /// 因此 `实体总数` **不能**当作"场景里有几个实体"来用。
    #[test]
    fn 插件自带实体导致总数不能当场景实体数() {
        let 宿主 = BSN宿主::新建();
        assert!(
            宿主.实体总数() > 3,
            "插件装配后世界里应当已有内部实体；若接近 0 说明插件没装配成功"
        );
    }

    /// 场景规模要从 `Children` 子树数出来，而不是拿世界实体总数去减基线。
    #[test]
    fn 场景规模按层级数而不是按世界实体数() {
        let mut 宿主 = BSN宿主::新建();
        let 根 = 基础关卡模板.实例化(宿主.世界可变()).expect("应实例化成功");

        let 直接子 = 宿主.子实体(根);
        assert_eq!(直接子.len(), 2);
        assert!(直接子.iter().all(|子| 宿主.子实体(*子).is_empty()));
        assert_eq!(1 + 直接子.len(), 3);
    }
}
