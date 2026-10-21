# 微风运行时调试示例

这是一个**真实的原生 Bevy 0.20 游戏**，用来验证微风的远程 ECS 检查器：

- 真实 `App` + ECS 世界；
- 9 个实体（1 个玩家 + 8 个敌人）；
- `Transform`、`Name`、自定义组件 `玩家属性`、刻意不注册的 `私有数据`；
- 自定义资源 `调试示例状态`；
- 玩家位置每帧变化，第 90 秒销毁一个敌人，计时结束后正常退出。

## 手动运行

```powershell
cargo run -- --duration-ms 15000
```

没有编辑器注入的 `BREEZE_DEBUG_*` 环境变量时，调试插件完全停用：
不监听端口、不建线程、不改变游戏行为。

## 在微风里运行

1. 用微风打开本目录（含 `Cargo.toml`）。
2. 在「▶ 运行」面板选择 `breeze_debug_demo::breeze_debug_demo`。
3. 点击「构建并运行」。
4. 打开「运行时 ECS」面板：握手后能看到真实的 9 个实体、`Transform` 数值、
   `玩家属性` 字段、`调试示例状态` 资源，以及 `私有数据` 的“不支持检查”。

## 接入自己的游戏

```toml
[dependencies]
bevy_breeze_debug = { package = "breeze_debug_adapter", path = "…/crates/breeze_debug_adapter" }
```

```rust
App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(bevy_breeze_debug::Breeze调试插件::自动())
    .run();
```

需要检查自定义类型时，用 `.注册组件::<T, _>(…)` / `.注册资源::<R, _>(…)`
显式提供只读提取函数；未注册的类型只显示类型名，不伪造字段。

## 安全边界

- 编辑器只监听 `127.0.0.1`；
- 一次性令牌通过进程环境注入，不写进命令行；
- 协议里没有任意文件读写、脚本执行或代码执行命令；
- 适配器只读，不修改游戏世界。
