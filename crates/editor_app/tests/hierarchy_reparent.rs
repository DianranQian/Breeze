//! 中文名：层级拖动
//! 保持世界坐标的层级拖动、防环与撤销重做验证。
//!
//! # 为什么这组测试存在
//!
//! 换父级只改文档里的 `父实体` 与 `次序` 是不够的：实体保存的是**局部**
//! 变换，旧父级与新父级的世界变换不同，直接换父级会让实体在世界空间里
//! 瞬移。这里跑真实的 Bevy 应用：先用 `TransformPlugin` 传播出世界变换，
//! 快照交给动作层反解新局部变换，再从修改后的文档重建一次预览，
//! 断言实体的世界位置、朝向与缩放都与拖动前一致。
//!
//! 防环与撤销重做走同一条动作队列，确保测试覆盖的就是界面推的那条路径。

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use editor_app::actions;
use editor_app::build::构建宿主;
use editor_app::config::{布局待保存, 编辑器配置资源};
use editor_app::diagnostics::诊断中心;
use editor_app::physics_runtime::物理运行时状态;
use editor_app::plugin::{
    EditorStateHolder, UiStateHolder, 导入快照资源, 文件对话框通道, 缩略图缓存, 诊断资源,
};
use editor_app::preview::{同步预览世界变换, 构建预览层级, 预览世界变换快照};
use editor_app::run::项目运行管理器;
use editor_app::scene_bridge::运行时宿主;
use editor_ui::state::UiAction;
use scene_doc::document::{变换数据, 场景文档, 实体标识};

/// 一个只保留变换传播与快照采集的最小应用。
fn 最小应用() -> App {
    let mut 应用 = App::new();
    应用.add_plugins((MinimalPlugins, TransformPlugin));
    应用.init_resource::<预览世界变换快照>();
    应用.add_systems(
        PostUpdate,
        同步预览世界变换.after(bevy::transform::TransformSystems::Propagate),
    );
    应用
}

/// 准备动作队列需要的宿主资源；测试不打开项目，允许项目为空。
fn 装入动作资源(应用: &mut App) {
    let 世界 = 应用.world_mut();
    世界.insert_resource(EditorStateHolder::default());
    世界.insert_resource(UiStateHolder::新建());
    世界.insert_resource(运行时宿主::default());
    世界.insert_resource(物理运行时状态::default());
    世界.insert_resource(文件对话框通道::default());
    世界.insert_resource(诊断资源::default());
    世界.insert_resource(缩略图缓存::default());
    世界.insert_resource(布局待保存::default());
    世界.insert_resource(项目运行管理器::default());
    世界.insert_resource(构建宿主::新建());
    世界.insert_resource(诊断中心::default());
    世界.insert_resource(编辑器配置资源::default());
    世界.insert_resource(导入快照资源::default());
}

/// 在应用里按文档建一次预览层级并传播世界变换。
fn 建预览(应用: &mut App, 文档: &场景文档) {
    {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, 文档);
    }
    应用.world_mut().flush();
    应用.update();
}

fn 推动作(应用: &mut App, 动作: UiAction) {
    应用
        .world_mut()
        .resource_mut::<UiStateHolder>()
        .动作
        .推(动作);
}

fn 执行动作(应用: &mut App) {
    应用
        .world_mut()
        .run_system_once(actions::执行动作队列)
        .expect("执行动作队列");
}

fn 当前文档(应用: &App) -> 场景文档 {
    应用
        .world()
        .resource::<EditorStateHolder>()
        .状态
        .会话
        .文档
        .clone()
}

fn 可撤销步数(应用: &App) -> usize {
    应用
        .world()
        .resource::<EditorStateHolder>()
        .状态
        .可撤销步数()
}

fn 绕_z(角: f32) -> (f32, f32, f32, f32) {
    let 半 = 角 * 0.5;
    (0.0, 0.0, 半.sin(), 半.cos())
}

/// 旧父级与新父级都带旋转与缩放，被拖实体本身带非均匀缩放。
///
/// 这样一来"世界位置保持"必须同时验证平移、朝向与缩放三条通道；
/// 只比较平移时，旋转/缩放算错的实现仍可能蒙混过关。
fn 造文档() -> (场景文档, 实体标识, 实体标识, 实体标识, 实体标识, 实体标识) {
    let mut 文档 = 场景文档::新建("层级拖动");
    let 旧父 = 文档.生成实体("旧父", None);
    let 新父 = 文档.生成实体("新父", None);
    let 实体 = 文档.生成实体("被拖实体", Some(旧父));
    let 新父甲 = 文档.生成实体("新父甲", Some(新父));
    let 新父乙 = 文档.生成实体("新父乙", Some(新父));

    let 旧父变换 = 变换数据 {
        平移: (10.0, 0.0, 0.0),
        旋转: 绕_z(30f32.to_radians()),
        缩放: (2.0, 2.0, 2.0),
    };
    let 新父变换 = 变换数据 {
        平移: (-5.0, 4.0, 0.0),
        旋转: 绕_z(-45f32.to_radians()),
        缩放: (0.5, 0.5, 0.5),
    };
    let 实体变换 = 变换数据 {
        平移: (3.0, 1.0, 0.0),
        旋转: 绕_z(15f32.to_radians()),
        缩放: (1.5, 0.75, 1.0),
    };
    for (标识, 变换) in [
        (旧父, 旧父变换),
        (新父, 新父变换),
        (实体, 实体变换),
        (新父甲, 变换数据::单位()),
        (新父乙, 变换数据::单位()),
    ] {
        if let Some(节点) = 文档.取可变(标识) {
            节点.变换 = Some(变换);
        }
    }
    (文档, 旧父, 新父, 实体, 新父甲, 新父乙)
}

/// 按文档重建预览，返回实体的世界平移、旋转与缩放。
///
/// 用**新应用**重建而不是复用动作测试的应用：后者里的预览层级还是拖动前
/// 建的，直接读会得到旧结果，无法区分"文档写对了"和"预览没更新"。
fn 世界分量(文档: &场景文档, 标识: 实体标识) -> (Vec3, Quat, Vec3) {
    let mut 应用 = 最小应用();
    let 节点表 = {
        let mut 命令 = 应用.world_mut().commands();
        构建预览层级(&mut 命令, 文档)
    };
    应用.world_mut().flush();
    应用.update();
    let 节点 = 节点表[&标识];
    let 全局 = 应用
        .world()
        .entity(节点)
        .get::<GlobalTransform>()
        .expect("传播后必须有 GlobalTransform");
    全局.to_scale_rotation_translation()
}

fn 断言世界一致(实际: (Vec3, Quat, Vec3), 期望: (Vec3, Quat, Vec3), 说明: &str) {
    let (实际平移, 实际旋转, 实际缩放) = 实际;
    let (期望平移, 期望旋转, 期望缩放) = 期望;
    assert!(
        (实际平移 - 期望平移).length() < 1e-3,
        "{说明}：世界平移应为 {期望平移:?}，实际 {实际平移:?}"
    );
    assert!(
        实际旋转.dot(期望旋转).abs() > 1.0 - 1e-3,
        "{说明}：世界朝向应保持不变，期望 {期望旋转:?}，实际 {实际旋转:?}"
    );
    assert!(
        (实际缩放 - 期望缩放).length() < 1e-3,
        "{说明}：世界缩放应为 {期望缩放:?}，实际 {实际缩放:?}"
    );
}

#[test]
fn 层级拖动后世界变换保持不变并追加到新父末尾() {
    let (文档, 旧父, 新父, 实体, 新父甲, 新父乙) = 造文档();
    let 拖动前世界 = 世界分量(&文档, 实体);
    let 原变换 = 文档.取(实体).and_then(|节点| 节点.变换);

    let mut 应用 = 最小应用();
    建预览(&mut 应用, &文档);
    装入动作资源(&mut 应用);
    {
        let 持有 = &mut 应用.world_mut().resource_mut::<EditorStateHolder>().状态;
        持有.应用已载入场景(文档.clone(), None);
    }

    推动作(
        &mut 应用,
        UiAction::更改实体父级 {
            实体,
            新父实体: Some(新父),
        },
    );
    执行动作(&mut 应用);

    let 改后 = 当前文档(&应用);
    assert_eq!(
        改后.取(实体).and_then(|节点| 节点.父实体),
        Some(新父),
        "拖动后父实体应变为新父级"
    );
    assert_eq!(
        改后.取(实体).map(|节点| 节点.次序),
        Some(2),
        "应追加到新父级子列表末尾"
    );
    assert_eq!(
        改后.子实体列表(Some(新父)),
        vec![新父甲, 新父乙, 实体],
        "新父级的子列表应保持原有顺序并把被拖实体追加在末尾"
    );
    assert!(
        改后.校验().无问题(),
        "改后文档必须自洽：{}",
        改后.校验().摘要()
    );
    断言世界一致(世界分量(&改后, 实体), 拖动前世界, "拖动后");

    推动作(&mut 应用, UiAction::撤销);
    执行动作(&mut 应用);
    let 撤销后 = 当前文档(&应用);
    assert_eq!(
        撤销后.取(实体).and_then(|节点| 节点.父实体),
        Some(旧父),
        "撤销后必须回到旧父级"
    );
    assert_eq!(
        撤销后.取(实体).and_then(|节点| 节点.变换),
        原变换,
        "撤销后本地变换必须逐字段复原"
    );
    assert_eq!(
        撤销后.实体表, 文档.实体表,
        "撤销后整份实体表必须与拖动前一致"
    );
    断言世界一致(世界分量(&撤销后, 实体), 拖动前世界, "撤销后");

    推动作(&mut 应用, UiAction::重做);
    执行动作(&mut 应用);
    let 重做后 = 当前文档(&应用);
    assert_eq!(
        重做后.取(实体).and_then(|节点| 节点.父实体),
        Some(新父),
        "重做后应再次挂到新父级"
    );
    assert_eq!(
        重做后.取(实体).map(|节点| 节点.次序),
        Some(2),
        "重做后次序应与首次执行一致"
    );
    断言世界一致(世界分量(&重做后, 实体), 拖动前世界, "重做后");
}

#[test]
fn 拖入自己的后代会被拒绝且文档与历史不变() {
    let (mut 文档, 旧父, _, 实体, _, _) = 造文档();
    let 后代 = 文档.生成实体("后代", Some(实体));
    if let Some(节点) = 文档.取可变(后代) {
        节点.变换 = Some(变换数据 {
            平移: (1.0, 0.0, 0.0),
            ..变换数据::单位()
        });
    }
    let 变更前 = 文档.clone();

    let mut 应用 = 最小应用();
    建预览(&mut 应用, &文档);
    装入动作资源(&mut 应用);
    {
        let 持有 = &mut 应用.world_mut().resource_mut::<EditorStateHolder>().状态;
        持有.应用已载入场景(文档.clone(), None);
    }

    推动作(
        &mut 应用,
        UiAction::更改实体父级 {
            实体,
            新父实体: Some(后代),
        },
    );
    执行动作(&mut 应用);

    let 被拒后 = 当前文档(&应用);
    assert_eq!(
        被拒后.实体表, 变更前.实体表,
        "成环必须被拒绝，文档一个字段都不能改"
    );
    assert_eq!(
        被拒后.取(实体).and_then(|节点| 节点.父实体),
        Some(旧父),
        "被拒后仍应挂在旧父级下"
    );
    assert_eq!(
        被拒后.取(后代).and_then(|节点| 节点.父实体),
        Some(实体),
        "被拒后后代关系不能反转"
    );
    assert_eq!(可撤销步数(&应用), 0, "被拒绝的操作不能进入撤销栈");
    assert!(被拒后.校验().无问题(), "被拒后文档必须自洽");
}

#[test]
fn 拖到自己身上会被拒绝() {
    let (文档, 旧父, _, 实体, _, _) = 造文档();
    let 变更前 = 文档.clone();

    let mut 应用 = 最小应用();
    建预览(&mut 应用, &文档);
    装入动作资源(&mut 应用);
    {
        let 持有 = &mut 应用.world_mut().resource_mut::<EditorStateHolder>().状态;
        持有.应用已载入场景(文档.clone(), None);
    }

    推动作(
        &mut 应用,
        UiAction::更改实体父级 {
            实体,
            新父实体: Some(实体),
        },
    );
    执行动作(&mut 应用);

    let 被拒后 = 当前文档(&应用);
    assert_eq!(被拒后.实体表, 变更前.实体表, "自指必须被拒绝");
    assert_eq!(
        被拒后.取(实体).and_then(|节点| 节点.父实体),
        Some(旧父),
        "被拒后父级不能变"
    );
    assert_eq!(可撤销步数(&应用), 0, "被拒绝的操作不能进入撤销栈");
}
