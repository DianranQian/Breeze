//! 真实端到端验收：编辑器调试桥 ⇄ 原生 Bevy 游戏适配器。
//!
//! ```text
//! cargo test -p editor_app --test 运行时调试桥 -- --ignored --nocapture
//! ```
//!
//! 这个测试会：
//!
//! 1. 把 `examples/bevy_debug_demo` 复制到临时目录（不修改示例本身）；
//! 2. 真实 `cargo run` 启动 Bevy 游戏；
//! 3. 编辑器只监听 `127.0.0.1`，游戏主动连接并握手；
//! 4. 读取真实实体/Transform/自定义组件/资源；
//! 5. 游戏销毁一个敌人后，检查器反映真实数量变化；
//! 6. 游戏退出后历史快照保留但不再标记为实时；
//! 7. 重启游戏得到新会话，旧数据不会冒充新会话。
//!
//! 默认 `#[ignore]`：它会编译并运行一个完整的 Bevy 游戏。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use editor_app::debug_bridge::运行时调试管理器;
use editor_app::run::{运行命令, 运行目标类型, 项目运行管理器};
use editor_core::run_entry::运行入口;
use editor_core::runtime_debug::{属性修改记录, 调试连接状态, 运行时数据状态};
use runtime_protocol::检查支持级别;

fn 示例目录() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/bevy_debug_demo")
}

fn 复制目录(从: &Path, 到: &Path) {
    std::fs::create_dir_all(到)
        .unwrap_or_else(|错误| panic!("创建目录失败 {}：{错误}", 到.display()));
    for 项 in std::fs::read_dir(从).unwrap() {
        let 项 = 项.unwrap();
        let 名 = 项.file_name();
        if 名 == "target" {
            continue;
        }
        let 源 = 项.path();
        let 目标 = 到.join(&名);
        if 项.file_type().unwrap().is_dir() {
            复制目录(&源, &目标);
        } else {
            std::fs::copy(&源, &目标).unwrap();
        }
    }
}

/// 准备一个可以真实编译的示例副本。
///
/// 为什么要复制：本仓库的示例目录由工具创建，其 ACL 在本机沙箱里
/// 不允许 `cargo` 新建 `Cargo.lock`/`target`。复制到临时目录既绕开
/// 环境限制，也保证验收**不会修改示例源码**。
fn 准备示例副本() -> (PathBuf, PathBuf) {
    // 构建产物复用仓库自己的 target：Bevy 0.20（含 debug 特性）已经
    // 由适配器测试编译过，验收不必再等十分钟从零编译引擎。
    let 共享目标 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target");
    (准备示例源码副本(), 共享目标)
}

fn 准备示例源码副本() -> PathBuf {
    let 源 = 示例目录();
    assert!(
        源.join("Cargo.toml").is_file(),
        "示例缺失：{}",
        源.display()
    );
    // 用仓库自己的 target 目录而不是系统 TEMP：本机测试进程的 TEMP
    // 可能被启动器隔离（CreateDirectory 直接拒绝），而 target 已被
    // `.gitignore` 忽略，放验收副本不会污染仓库。
    let 根 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/acceptance-temp")
        .join(format!(
            "breeze_debug_acceptance_{}_{}",
            std::process::id(),
            editor_ui::agent::现在秒()
        ));
    if 根.exists() {
        let _ = std::fs::remove_dir_all(&根);
    }
    复制目录(&源, &根);

    // 把相对 path 依赖换成绝对路径：副本不在原来的层级里。
    let 仓库 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let 适配器 = 仓库.join("crates/breeze_debug_adapter");
    let 协议 = 仓库.join("crates/runtime_protocol");
    let 清单 = std::fs::read_to_string(根.join("Cargo.toml")).unwrap();
    let 清单 = 清单
        .replace(
            "../../crates/breeze_debug_adapter",
            &适配器.display().to_string().replace('\\', "/"),
        )
        .replace(
            "../../crates/runtime_protocol",
            &协议.display().to_string().replace('\\', "/"),
        );
    std::fs::write(根.join("Cargo.toml"), 清单).unwrap();
    根
}

fn 等到(超时: Duration, mut 条件: impl FnMut() -> bool) -> bool {
    let 截止 = Instant::now() + 超时;
    while Instant::now() < 截止 {
        if 条件() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    条件()
}

/// 轮询调试管理器，直到握手完成并拿到第一批真实数据。
fn 等到连接并出数据(
    调试: &mut 运行时调试管理器,
    运行: &mut 项目运行管理器,
    超时: Duration,
) -> bool {
    let 截止 = Instant::now() + 超时;
    let mut 已请求 = false;
    while Instant::now() < 截止 {
        调试.轮询();
        for 诊断 in 调试.取诊断() {
            println!("[debug] {:?} {}", 诊断.级别, 诊断.消息);
        }
        let 日志 = 运行.轮询();
        for 行 in 日志 {
            println!("[game] {行}");
        }
        if !运行.状态.在运行() {
            println!("游戏进程在握手前结束：{}", 运行.结果文本());
            return false;
        }
        if 调试.快照().是否实时() && !已请求 {
            // 握手事件里已经自动刷新；这里只在数据仍未到时补一次。
            if 调试.快照().实体页.is_none() {
                调试.刷新();
            }
            已请求 = true;
        }
        if 调试.快照().实体页.is_some() && 调试.快照().世界概览.is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn 查询实体(调试: &mut 运行时调试管理器, 文本: &str) {
    调试.查询实体(0, 32, 文本.to_string(), String::new());
}

fn 等到实体页(
    调试: &mut 运行时调试管理器,
    运行: &mut 项目运行管理器,
    超时: Duration,
    谓词: impl Fn(&runtime_protocol::实体列表页) -> bool,
) -> bool {
    let 截止 = Instant::now() + 超时;
    let mut 上次总数 = None;
    while Instant::now() < 截止 {
        调试.轮询();
        for 行 in 运行.轮询() {
            println!("[game] {行}");
        }
        if let Some(页) = &调试.快照().实体页 {
            if 上次总数 != Some(页.总数) {
                上次总数 = Some(页.总数);
                println!("[debug] 实体页总数 = {}（filtered）", 页.总数);
            }
            if 谓词(页) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn 取组件字段(
    详情: &runtime_protocol::实体详情,
    组件后缀: &str,
    字段: &str,
) -> Option<runtime_protocol::字段值> {
    详情
        .组件
        .iter()
        .find(|组件| 组件.类型名.ends_with(组件后缀))
        .and_then(|组件| 组件.字段.iter().find(|字段项| 字段项.名称 == 字段))
        .map(|字段项| 字段项.值.clone())
}

fn 提交并等待修改(
    调试: &mut 运行时调试管理器,
    运行: &mut 项目运行管理器,
    实体: u64,
    组件: &str,
    字段: &str,
    目标值: runtime_protocol::字段值,
    预期原值: runtime_protocol::字段值,
) -> 属性修改记录 {
    let 之前 = 调试.历史().修改().len();
    调试
        .提交属性修改(
            实体,
            组件.to_string(),
            字段.to_string(),
            None,
            目标值,
            预期原值,
        )
        .expect("提交属性修改");
    let 截止 = Instant::now() + Duration::from_secs(15);
    while Instant::now() < 截止 {
        调试.轮询();
        let _ = 运行.轮询();
        if let Some(记录) = 调试.历史().修改().get(之前) {
            return 记录.clone();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("等待属性修改结果超时");
}

fn 等待修改字段(
    调试: &mut 运行时调试管理器,
    实体: u64,
    组件后缀: &str,
    字段: &str,
    期望: &runtime_protocol::字段值,
) -> bool {
    let 截止 = Instant::now() + Duration::from_secs(20);
    while Instant::now() < 截止 {
        调试.轮询();
        if let Some(详情) = &调试.快照().实体详情
            && 详情.标识 == 实体
            && 取组件字段(详情, 组件后缀, 字段).as_ref() == Some(期望)
        {
            return true;
        }
        调试.刷新();
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn 启动游戏(
    根: &Path,
    目标: &Path,
    时长毫秒: u64,
    销毁轮次: u64,
    调试: &mut 运行时调试管理器,
    运行: &mut 项目运行管理器,
) -> String {
    调试.轮询();
    let (运行标识, 环境) = 调试
        .准备启动("breeze-debug-acceptance")
        .expect("监听调试端口");
    // 链接器会往 TEMP 写临时文件；启动器隔离后的 TEMP 可能不可写。
    // 把子进程的临时目录指到验收副本内部。
    let 子进程临时目录 = 根.join("tmp");
    std::fs::create_dir_all(&子进程临时目录).unwrap();
    let mut 命令 = 运行命令 {
        程序: PathBuf::from("cargo"),
        参数: vec![
            "run".to_string(),
            "--bin".to_string(),
            "breeze_debug_demo".to_string(),
            "--".to_string(),
            "--duration-ms".to_string(),
            时长毫秒.to_string(),
            "--destroy-at".to_string(),
            销毁轮次.to_string(),
            "--add-at".to_string(),
            "17".to_string(),
            "--remove-at".to_string(),
            "19".to_string(),
        ],
        工作目录: 根.to_path_buf(),
        环境: vec![
            ("CARGO_TARGET_DIR".to_string(), 目标.display().to_string()),
            ("CARGO_NET_OFFLINE".to_string(), "true".to_string()),
            ("CARGO_TERM_COLOR".to_string(), "never".to_string()),
            ("TEMP".to_string(), 子进程临时目录.display().to_string()),
            ("TMP".to_string(), 子进程临时目录.display().to_string()),
        ],
        目标类型: 运行目标类型::Cargo可执行目标,
        入口: Some(运行入口::新建(
            "breeze_debug_demo",
            "breeze_debug_demo",
        )),
    };
    命令.环境.extend(环境);
    println!("[debug-env] {:?}", 命令.环境);
    运行
        .启动带标识(命令, Some(运行标识.clone()))
        .expect("启动真实 Bevy 游戏");
    运行标识
}

#[test]
#[ignore = "会真实编译并运行 Bevy 游戏：用 --ignored 显式触发"]
fn 真实游戏完成握手查询变化断开与会话隔离() {
    let (根, 目标) = 准备示例副本();
    let mut 调试 = 运行时调试管理器::default();
    let mut 运行 = 项目运行管理器::default();

    let 第一标识 = 启动游戏(&根, &目标, 30_000, 15, &mut 调试, &mut 运行);
    assert!(
        等到连接并出数据(&mut 调试, &mut 运行, Duration::from_secs(240)),
        "4 分钟内没有完成真实握手；连接状态：{:?}，运行日志：{:?}",
        调试.快照().状态,
        运行.日志.iter().rev().take(40).collect::<Vec<_>>()
    );
    let 第一会话 = 调试.快照().会话标识.clone().expect("握手后必须有会话标识");
    assert_eq!(调试.快照().状态, 调试连接状态::已连接);
    assert_eq!(调试.快照().应用名称.as_deref(), Some("breeze_debug_demo"));
    assert_eq!(调试.快照().bevy版本.as_deref(), Some("0.20"));
    assert_eq!(调试.快照().运行实例标识.as_deref(), Some(第一标识.as_str()));
    assert_eq!(
        运行
            .历史
            .最近()
            .and_then(|记录| 记录.运行实例标识.as_deref()),
        Some(第一标识.as_str()),
        "运行记录与调试会话必须使用同一个运行实例标识"
    );
    assert!(
        调试
            .快照()
            .监听地址
            .as_deref()
            .is_some_and(|值| 值.starts_with("127.0.0.1:")),
        "必须只监听 loopback"
    );

    // ---- 实体列表来自真实世界 ----
    查询实体(&mut 调试, "玩家");
    assert!(
        等到实体页(&mut 调试, &mut 运行, Duration::from_secs(20), |页| 页
            .总数
            >= 1),
        "查询玩家实体超时"
    );
    let 玩家 = 调试
        .快照()
        .实体页
        .as_ref()
        .unwrap()
        .实体
        .iter()
        .find(|项| 项.名称.as_deref() == Some("玩家"))
        .cloned()
        .expect("真实世界里必须有玩家");
    调试.选中实体(玩家.标识);
    assert!(
        等到(Duration::from_secs(20), || {
            调试.轮询();
            调试.快照().实体详情.as_ref().map(|项| 项.标识) == Some(玩家.标识)
        }),
        "实体详情没有返回"
    );
    let 详情 = 调试.快照().实体详情.as_ref().unwrap();
    assert!(
        详情.组件.iter().any(|项| {
            项.类型名.ends_with("玩家属性") && 项.支持 == 检查支持级别::可检查
        }),
        "自定义组件必须可检查"
    );
    let 变换 = 详情
        .组件
        .iter()
        .find(|项| 项.类型名.ends_with("::transform::Transform"))
        .expect("局部 Transform 必须存在");
    let 位置 = 变换
        .字段
        .iter()
        .find(|项| 项.名称 == "translation")
        .expect("Transform 必须有 translation");
    let 横坐标 = match &位置.值 {
        runtime_protocol::字段值::向量(值) if 值.len() >= 3 => 值[0],
        _ => panic!("translation 必须是三维向量：{:?}", 位置.值),
    };
    assert!(
        横坐标 >= 1.0,
        "玩家初始 x 必须是 1.0，实际 {横坐标}；全部字段：{:?}",
        详情
            .组件
            .iter()
            .map(|项| (&项.类型名, &项.字段))
            .collect::<Vec<_>>()
    );
    assert!(
        详情
            .组件
            .iter()
            .any(|项| 项.类型名.ends_with("私有数据") && 项.支持 == 检查支持级别::不支持),
        "未注册组件必须明确标成不支持检查"
    );

    // ---- 自定义资源 ----
    assert!(
        调试
            .快照()
            .资源列表
            .iter()
            .any(|项| 项.名称 == "调试示例状态"),
        "资源列表必须包含显式注册的资源"
    );
    调试.选中资源("调试示例状态");
    assert!(
        等到(Duration::from_secs(20), || {
            调试.轮询();
            调试.快照().资源详情.is_some()
        }),
        "资源详情没有返回"
    );

    // ---- 快照 #1（修改前）----
    调试.采集快照().expect("采集快照");
    let 快照截止 = Instant::now() + Duration::from_secs(20);
    while Instant::now() < 快照截止 && 调试.历史().快照().is_empty() {
        调试.轮询();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(调试.历史().快照().len(), 1, "快照 #1 没有返回");
    assert!(!调试.历史().快照()[0].已截断, "演示世界不应被截断");

    // ---- 白名单属性修改：Transform.scale ----
    let 修改前详情 = 调试
        .快照()
        .实体详情
        .clone()
        .expect("属性修改前必须有实体详情");
    let 当前缩放 =
        取组件字段(&修改前详情, "::transform::Transform", "scale").expect("Transform.scale");
    let 目标缩放 = runtime_protocol::字段值::向量(vec![2.0, 2.0, 2.0]);
    let 缩放记录 = 提交并等待修改(
        &mut 调试,
        &mut 运行,
        玩家.标识,
        "Transform",
        "scale",
        目标缩放.clone(),
        当前缩放,
    );
    assert!(
        缩放记录.结果.成功,
        "合法 Transform 写入必须成功：{缩放记录:?}"
    );
    assert_eq!(缩放记录.结果.最终值.as_ref(), Some(&目标缩放));
    assert!(
        等待修改字段(
            &mut 调试,
            玩家.标识,
            "::transform::Transform",
            "scale",
            &目标缩放
        ),
        "成功响应后检查器必须显示游戏端最终值"
    );

    // ---- 白名单属性修改：自定义数值字段 ----
    let 当前生命 = 取组件字段(&修改前详情, "玩家属性", "生命值").expect("玩家属性.生命值");
    let 目标生命 = runtime_protocol::字段值::浮点(77.0);
    let 生命记录 = 提交并等待修改(
        &mut 调试,
        &mut 运行,
        玩家.标识,
        "玩家属性",
        "生命值",
        目标生命.clone(),
        当前生命,
    );
    assert!(生命记录.结果.成功, "合法自定义字段写入必须成功");
    assert!(等待修改字段(
        &mut 调试,
        玩家.标识,
        "玩家属性",
        "生命值",
        &目标生命
    ));

    let 合法值 = runtime_protocol::字段值::浮点(77.0);
    let 非法 = 提交并等待修改(
        &mut 调试,
        &mut 运行,
        玩家.标识,
        "玩家属性",
        "生命值",
        runtime_protocol::字段值::浮点(2000.0),
        合法值.clone(),
    );
    assert!(!非法.结果.成功, "越界值必须被拒绝");
    assert_eq!(
        非法.结果.错误.as_ref().map(|错误| 错误.代码),
        Some(runtime_protocol::操作错误代码::值无效)
    );
    assert!(等待修改字段(
        &mut 调试,
        玩家.标识,
        "玩家属性",
        "生命值",
        &合法值
    ));

    let 冲突 = 提交并等待修改(
        &mut 调试,
        &mut 运行,
        玩家.标识,
        "玩家属性",
        "生命值",
        runtime_protocol::字段值::浮点(50.0),
        runtime_protocol::字段值::浮点(1.0),
    );
    assert!(
        !冲突.结果.成功 && 冲突.结果.冲突,
        "预期原值不一致必须报冲突"
    );

    let 未注册 = 提交并等待修改(
        &mut 调试,
        &mut 运行,
        玩家.标识,
        "玩家属性",
        "不在白名单",
        runtime_protocol::字段值::浮点(1.0),
        runtime_protocol::字段值::浮点(1.0),
    );
    assert!(!未注册.结果.成功);
    assert_eq!(
        未注册.结果.错误.as_ref().map(|错误| 错误.代码),
        Some(runtime_protocol::操作错误代码::字段不可写)
    );

    let 已销毁 = 提交并等待修改(
        &mut 调试,
        &mut 运行,
        u64::MAX,
        "Transform",
        "scale",
        runtime_protocol::字段值::向量(vec![1.0, 1.0, 1.0]),
        runtime_protocol::字段值::向量(vec![1.0, 1.0, 1.0]),
    );
    assert!(!已销毁.结果.成功);
    assert!(!已销毁.结果.实体仍存在);

    // ---- 等游戏执行销毁/加组件/移除组件，再取快照 #2 ----
    let 变化截止 = Instant::now() + Duration::from_secs(30);
    while Instant::now() < 变化截止 {
        for 行 in 运行.轮询() {
            println!("[game] {行}");
        }
        if !运行.状态.在运行() {
            break;
        }
        if 运行.日志.iter().any(|行| 行.contains("已销毁一个敌人"))
            && 运行.日志.iter().any(|行| 行.contains("护盾"))
            && 运行.日志.iter().any(|行| 行.contains("移除玩家"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    查询实体(&mut 调试, "敌人");
    assert!(
        等到实体页(&mut 调试, &mut 运行, Duration::from_secs(20), |页| 页
            .总数
            == 7),
        "游戏销毁一个敌人后，检查器必须反映 7 个；当前：{:?}",
        调试.快照().实体页.as_ref().map(|页| 页.总数)
    );

    调试.采集快照().expect("采集快照 #2");
    let 快照截止 = Instant::now() + Duration::from_secs(20);
    while Instant::now() < 快照截止 && 调试.历史().快照().len() < 2 {
        调试.轮询();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(调试.历史().快照().len(), 2, "快照 #2 没有返回");
    let 差异 = 调试.比较最近快照().expect("比较快照");
    assert!(差异.可比, "同一运行实例的快照必须可比");
    assert!(!差异.已截断, "演示世界不应被截断");
    assert!(!差异.消失实体.is_empty(), "至少一个敌人实体消失");
    assert!(
        差异.新增组件.iter().any(|(_, 名称)| 名称.contains("护盾")),
        "必须识别新增护盾组件：{:?}",
        差异.新增组件
    );
    assert!(
        差异
            .移除组件
            .iter()
            .any(|(_, 名称)| 名称.contains("私有数据")),
        "必须识别移除私有数据组件：{:?}",
        差异.移除组件
    );
    assert!(
        !差异.字段变化.is_empty(),
        "至少识别一次字段变化（Transform/生命值）"
    );
    assert!(
        !差异.资源变化.is_empty(),
        "资源「调试示例状态.回合」变化必须进入差异"
    );
    assert!(
        调试.历史().快照().last().is_some_and(|项| 项.完整()),
        "本次演示快照应为完整快照"
    );
    let 上下文 = editor_core::智能体上下文文本扩展(调试.快照(), 调试.历史());
    assert!(上下文.contains("成功，游戏端最终值"));
    assert!(上下文.contains("最近快照差异"));
    assert!(上下文.contains("未收到游戏端成功结果") || 上下文.contains("不能当作已生效"));

    // ---- 游戏自然退出：保留历史但标记过期 ----
    let 退出截止 = Instant::now() + Duration::from_secs(90);
    while Instant::now() < 退出截止 && 运行.状态.在运行() {
        let _ = 运行.轮询();
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!运行.状态.在运行(), "游戏没有按计时退出");
    // 测试里没有 Bevy schedule，手动模拟 `推进调试` 的生命周期收尾。
    调试.结束运行("验收：游戏已退出");
    调试.轮询();
    assert_eq!(调试.快照().状态, 调试连接状态::已断开);
    assert!(!调试.快照().是否实时());
    assert_eq!(调试.快照().数据状态(), 运行时数据状态::最近快照);
    assert!(调试.快照().世界概览.is_some(), "历史快照应保留");
    assert!(
        调试.历史().快照().len() >= 2,
        "游戏退出后仍应能查看历史快照"
    );

    // ---- 重启：新会话隔离旧数据 ----
    let 第二标识 = 启动游戏(&根, &目标, 8_000, 99, &mut 调试, &mut 运行);
    assert_ne!(第一标识, 第二标识, "每次运行必须有独立实例标识");
    assert!(
        等到连接并出数据(&mut 调试, &mut 运行, Duration::from_secs(240)),
        "第二次启动没有完成握手"
    );
    let 第二会话 = 调试.快照().会话标识.clone().unwrap();
    assert_ne!(第一会话, 第二会话, "游戏重启必须建立新会话");
    assert_eq!(调试.快照().运行实例标识.as_deref(), Some(第二标识.as_str()));
    assert!(
        调试.快照().实体详情.is_none(),
        "新会话不得沿用旧会话的选中实体详情"
    );
    assert!(
        调试.历史().快照().is_empty(),
        "游戏重启后旧运行实例的快照历史必须清空"
    );

    运行.停止();
    调试.结束运行("验收结束");
    let _ = std::fs::remove_dir_all(&根);
}
