# Bevy Studio —— 原生 Cargo 工程 + AI Agent 模式的 GUI 自动验收
#
# 这两件事都是"看起来简单、实际只能靠真实界面确认"的：
#
# | 场景 | 只跑单测会漏掉什么 |
# | --- | --- |
# | 原生 Cargo 工程打开 | `project.ron` 缺失时真实打开链路是否还能建出工作区 |
# | `BEVY_STUDIO_MODE=agent` | 模式枚举切了、**布局**却被"修复"逻辑换成通用布局 |
#
# 因此这里的每个场景都启动真实应用、抓渲染目标、读布局证据再做断言，
# 并且对"点击模式栏切换"用真实 SendInput 注入（不是伪造状态）。
#
# 用法（仓库根执行）：
#   pwsh tools/gui_agent_acceptance.ps1
#   pwsh tools/gui_agent_acceptance.ps1 -Exe path\to\bevy-studio.exe
#
# # 不要与其他 GUI 验收并行跑
#
# 点击类场景靠 SendInput 打真实鼠标事件，落点由"目标窗口的客户区原点 +
# 逻辑坐标"算出。同期再有另一个验收在启动/关闭窗口时，前台焦点与窗口枚举
# 都会被抢，表现为 `received_pointer=false`、截图采集失败 —— 那是**环境冲突**，
# 不是界面缺陷。实测过一次并行运行：同一份代码单独跑 11/11 通过，
# 并行跑却随机挂 2~3 项。串行执行是本脚本的前提。
#
# 退出码：0 全部通过 / 2 界面缺陷 / 3 采集失败 / 4 用法错误 / 1 脚本自身失败。

param(
    [string]$Exe = "",
    [string]$OutRoot = ".verify/runs/agent_acceptance"
)

$ErrorActionPreference = "Stop"
$root = (Get-Location).Path

if ([string]::IsNullOrWhiteSpace($Exe)) {
    $candidates = @(
        (Join-Path (Split-Path (Split-Path $root -Parent) -Parent) "_build\target\debug\bevy-studio.exe"),
        (Join-Path $root "target\debug\bevy-studio.exe"),
        "C:\Spaces\Share\10_Code\10_Bevy\_build\target\debug\bevy-studio.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { $Exe = $c; break }
    }
}
$probe = Join-Path (Split-Path $Exe -Parent) "gui_probe.exe"
if (-not (Test-Path $Exe)) { throw "找不到应用：$Exe（先运行 cargo build -p editor_app）" }
if (-not (Test-Path $probe)) { throw "找不到验收工具：$probe（先运行 cargo build -p gui_probe）" }

$outRoot = Join-Path $root $OutRoot
if (Test-Path $outRoot) { Remove-Item -Recurse -Force $outRoot }
New-Item -ItemType Directory -Force -Path $outRoot | Out-Null

# 每个"会话"用独立配置目录：场景之间只有明确要测"重启恢复"时才共享。
$cfgAgent = Join-Path $outRoot "cfg_agent"
$cfgModes = Join-Path $outRoot "cfg_modes"
$cfgNative = Join-Path $outRoot "cfg_native"
foreach ($d in @($cfgAgent, $cfgModes, $cfgNative)) {
    New-Item -ItemType Directory -Force -Path $d | Out-Null
}

$script:失败 = 0
$script:通过 = 0

function 记录([string]$名, [int]$码) {
    if ($码 -eq 0) {
        $script:通过++
        Write-Host "[通过] $名"
    } else {
        $script:失败++
        Write-Host "[失败] $名（退出码 $码）" -ForegroundColor Red
    }
}

# 启动一次并直接判定（run：单帧采集）。
function 启动判定([string]$名, [string]$配置目录, [string[]]$额外) {
    $参数 = @("run", "--exe", $Exe, "--out", $outRoot, "--name", $名,
        "--window", "1600x1000", "--frame", "150", "--timeout", "180") +
        @("--env", "APPDATA=$配置目录") + $额外
    & $probe @参数 2>&1 | ForEach-Object { Write-Host "    $_" }
    记录 $名 $LASTEXITCODE
}

# 点击一次区域再判定：证据来自"点击之后"的第二帧截图。
function 点击后判定([string]$名, [string]$配置目录, [string]$用例, [string[]]$期望) {
    & $probe drag --exe $Exe --out $outRoot --name $名 --case $用例 --timeout 180 `
        --env "APPDATA=$配置目录" 2>&1 | ForEach-Object { Write-Host "    $_" }
    $码 = $LASTEXITCODE
    if ($码 -ne 0) { 记录 $名 $码; return }
    # drag 自己只校验拖拽/点击是否生效；布局断言交给 analyze 读证据。
    & $probe analyze --png (Join-Path $outRoot "${名}_after.png") `
        --json (Join-Path $outRoot "${名}_after.json") --name $名 @期望 2>&1 |
        ForEach-Object { Write-Host "    $_" }
    $码 = $LASTEXITCODE
    # drag 会话本身若判定失败（输入没送达），也不能算通过。
    $drag报告 = Join-Path $outRoot "$名.drag.json"
    if ($码 -eq 0 -and (Test-Path $drag报告)) {
        $判定 = (Get-Content $drag报告 -Raw | ConvertFrom-Json)
        if ($判定.PSObject.Properties.Name -contains "ok" -and -not $判定.ok) { $码 = 2 }
    }
    记录 $名 $码
}

Write-Host "== 场景 1：BEVY_STUDIO_MODE=agent 启动即进入智能体布局 =="
启动判定 "agent_start" $cfgAgent @(
    "--env", "BEVY_STUDIO_MODE=agent",
    "--expect-mode", "agent",
    "--expect-workspace", "general",
    "--expect-panels", "agent_chat,agent_sessions,agent_review,console"
)

Write-Host "== 场景 2：重启后仍恢复智能体布局（存档往返） =="
启动判定 "agent_restart" $cfgAgent @(
    "--expect-mode", "agent",
    "--expect-workspace", "general",
    "--expect-panels", "agent_chat,agent_sessions,agent_review,console"
)

Write-Host "== 场景 3：普通启动仍是 2D 默认模式 =="
启动判定 "default_2d" $cfgModes @(
    "--env", "BEVY_STUDIO_MODE=2d",
    "--expect-mode", "2d",
    "--expect-panels", "viewport,scene_tree,inspector,console"
)

Write-Host "== 场景 4：点击模式栏 2D→AI Agent，布局随之切换 =="
点击后判定 "click_to_agent" $cfgModes "click:mode_agent" @(
    "--expect-mode", "agent",
    "--expect-panels", "agent_chat,agent_sessions,agent_review,console"
)

Write-Host "== 场景 5：AI Agent→3D，专用布局让位给 3D 布局 =="
点击后判定 "click_to_3d" $cfgModes "click:mode_3d" @(
    "--expect-mode", "3d",
    "--expect-panels", "viewport,scene_tree,inspector,console"
)

Write-Host "== 场景 6：3D→代码模式 =="
点击后判定 "click_to_script" $cfgModes "click:mode_script" @(
    "--expect-mode", "script",
    "--expect-panels", "script,console"
)

Write-Host "== 场景 7：代码→AI Agent，回到智能体布局 =="
点击后判定 "click_back_agent" $cfgModes "click:mode_agent" @(
    "--expect-mode", "agent",
    "--expect-panels", "agent_chat,agent_sessions,agent_review,console"
)

Write-Host "== 场景 8：打开无微风配置的原生 Cargo 工程 =="
$固件 = Join-Path $outRoot "native_fixture"
New-Item -ItemType Directory -Force -Path (Join-Path $固件 "src"), (Join-Path $固件 "assets/textures") | Out-Null
@"
[package]
name = "native_fixture_game"
version = "0.1.0"
edition = "2024"

[dependencies]
bevy = "0.20"
"@ | Set-Content (Join-Path $固件 "Cargo.toml") -Encoding utf8
@"
fn main() {
    println!("native fixture");
}
"@ | Set-Content (Join-Path $固件 "src/main.rs") -Encoding utf8
Set-Content (Join-Path $固件 "assets/textures/hero.png") -Value "placeholder" -Encoding ascii

if ((Test-Path (Join-Path $固件 "project.ron")) -or -not (Test-Path (Join-Path $固件 "Cargo.toml"))) {
    Write-Host "固件准备失败：打开前必须只有 Cargo.toml" -ForegroundColor Red
    exit 1
}
$清单指纹前 = (Get-FileHash (Join-Path $固件 "Cargo.toml") -Algorithm SHA256).Hash

启动判定 "native_open" $cfgNative @(
    "--env", "BEVY_STUDIO_OPEN_PROJECT=$固件",
    "--expect-mode", "2d",
    "--expect-panels", "viewport,asset_browser,console,scene_tree"
)

$清单指纹后 = (Get-FileHash (Join-Path $固件 "Cargo.toml") -Algorithm SHA256).Hash
if ($清单指纹前 -ne $清单指纹后) {
    Write-Host "[失败] native_manifest_untouched：打开项目改动了用户的 Cargo.toml" -ForegroundColor Red
    $script:失败++
} else {
    Write-Host "[通过] native_manifest_untouched"
    $script:通过++
}
if (Test-Path (Join-Path $固件 "project.ron")) {
    Write-Host "[失败] native_no_project_ron：打开原生工程生成了 project.ron" -ForegroundColor Red
    $script:失败++
} else {
    Write-Host "[通过] native_no_project_ron"
    $script:通过++
}

Write-Host "== 场景 9：原生工程里浏览并打开 Rust 源文件 =="
启动判定 "native_open_source" $cfgNative @(
    "--env", "BEVY_STUDIO_OPEN_PROJECT=$固件",
    "--env", "BEVY_STUDIO_OPEN_SCRIPT=src/main.rs",
    "--expect-mode", "script",
    "--expect-panels", "script,console"
)

Write-Host ""
Write-Host "通过 $script:通过 项，失败 $script:失败 项"
if ($script:失败 -gt 0) { exit 2 }
exit 0
