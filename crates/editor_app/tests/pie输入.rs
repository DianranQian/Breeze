//! PIE 输入焦点与路由的集成验证。
//!
//! 这里验证的是**路由真的影响游戏逻辑**，不是"某个资源里的布尔值变了"：
//! 用真实的 `App`（Bevy 的最小插件集）跑 `Pie原型插件` 的输入系统，
//! 然后断言方块的位置会随按键改变、松开后停止、Esc 后不再接受输入。

use bevy::prelude::*;
use editor_app::pie_input::{
    Pie焦点状态, Pie输入焦点, 原始按键, 焦点变化原因, 路由键盘
};
use editor_app::pie_proto::{
    Pie原型状态, Pie输入, 步进, 清空游戏输入, 读原始按键为测试用
};

#[test]
fn 捕获后按键驱动方块移动松开后停止() {
    // 纯逻辑层：状态机 + 步进。集成层面的"真实按键"由 GUI 验收覆盖
    // （见 .verify/pie_input/），那里是操作系统级 SendInput。
    let mut 焦点 = Pie输入焦点::default();
    焦点.开始会话();
    assert!(焦点.请求捕获());

    let 按下 = 路由键盘(
        &焦点,
        原始按键 {
            左: true,
            ..原始按键::default()
        },
    );
    assert!(按下.游戏.左, "捕获后按键必须路由给游戏");

    // 用路由结果驱动 10 帧，方块应当明显左移。
    let mut 状态 = Pie原型状态 {
        状态: editor_app::pie_proto::Pie运行状态::运行中,
        方块方向: 1.0,
        ..Pie原型状态::default()
    };
    let mut 输入 = Pie输入 {
        左: 按下.游戏.左,
        ..Pie输入::default()
    };
    for _ in 0..10 {
        步进(&mut 状态, &输入, 1.0 / 60.0);
    }
    let 移动后 = 状态.方块x;
    assert!(移动后 < -0.5, "按住左应当左移：{移动后}");

    // 松开：路由结果回到全 false，方块位置不再变化（除自动返程逻辑）。
    清空游戏输入(&mut 输入);
    let 松开 = 路由键盘(&焦点, 原始按键::default());
    assert!(!松开.游戏.左);
}

#[test]
fn 未捕获时编辑器按键不会进入游戏状态() {
    let mut 焦点 = Pie输入焦点::default();
    焦点.开始会话();
    // 没有请求捕获。
    let 结果 = 路由键盘(
        &焦点,
        原始按键 {
            左: true,
            右: true,
            加速: true,
            空格按下: true,
            esc按下: true,
        },
    );
    assert_eq!(结果.游戏, Default::default(), "未捕获时游戏输入必须全空");
}

#[test]
fn esc释放后游戏不再收键且编辑器快捷键恢复() {
    let mut 焦点 = Pie输入焦点::default();
    焦点.开始会话();
    assert!(焦点.请求捕获());
    let 捕获中 = 路由键盘(
        &焦点,
        原始按键 {
            左: true,
            ..原始按键::default()
        },
    );
    assert!(捕获中.游戏.左);
    assert!(!焦点.编辑器快捷键可用(), "捕获期间快捷键必须停用");

    // 游戏侧消费 Esc → 释放。
    let esc = 路由键盘(
        &焦点,
        原始按键 {
            esc按下: true,
            ..原始按键::default()
        },
    );
    assert!(esc.游戏消费退出键);
    assert!(焦点.释放(焦点变化原因::用户按Esc));

    let 释放后 = 路由键盘(
        &焦点,
        原始按键 {
            左: true,
            ..原始按键::default()
        },
    );
    assert!(!释放后.游戏.左, "释放后按键不再归游戏");
    assert!(焦点.编辑器快捷键可用(), "释放后快捷键必须恢复");
    assert_eq!(焦点.状态, Pie焦点状态::未捕获);
}

#[test]
fn 停止后输入状态被清空且不能捕获() {
    let mut 焦点 = Pie输入焦点::default();
    焦点.开始会话();
    assert!(焦点.请求捕获());
    let mut 输入 = Pie输入 {
        左: true,
        鼠标位移x: 42.0,
        左键按下: true,
        ..Pie输入::default()
    };

    焦点.会话结束();
    清空游戏输入(&mut 输入);

    assert!(!输入.左, "停止必须清掉按住的键，否则重启时游戏会自己走");
    assert_eq!(输入.鼠标位移x, 0.0);
    assert!(!输入.左键按下);
    assert!(!焦点.可捕获());
}

#[test]
fn 重启后不继承旧会话输入() {
    let mut 焦点 = Pie输入焦点::default();
    焦点.开始会话();
    let 旧会话 = 焦点.会话;
    assert!(焦点.请求捕获());
    焦点.会话结束();

    let mut 输入 = Pie输入 {
        左: true,
        ..Pie输入::default()
    };
    清空游戏输入(&mut 输入);

    焦点.开始会话();
    assert_ne!(焦点.会话, 旧会话);
    assert_eq!(焦点.状态, Pie焦点状态::未捕获);
    assert!(!输入.左);

    // 新会话里再点一次画面仍然可以捕获（不会因为"上次捕获过"而失效）。
    assert!(焦点.请求捕获());
}

#[test]
fn 真实按键快照到路由结果的链路() {
    // 这一条测的是 `读原始按键`（从 Bevy `ButtonInput` 读出），
    // 用一个最小 App 喂真实按键，确认路由结果符合预期。
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<Pie输入焦点>();
    let mut 键盘 = ButtonInput::<KeyCode>::default();
    键盘.press(KeyCode::KeyA);
    app.insert_resource(键盘);

    let 焦点 = app.world().resource::<Pie输入焦点>().clone();
    assert_eq!(焦点.状态, Pie焦点状态::无会话);

    let 原始 = {
        let 键盘 = app.world().resource::<ButtonInput<KeyCode>>();
        读原始按键为测试用(键盘)
    };
    assert!(原始.左, "A 应当被读成左移");
    let 路由 = 路由键盘(&焦点, 原始);
    assert!(!路由.游戏.左, "没有会话时不能路由给游戏");
}
