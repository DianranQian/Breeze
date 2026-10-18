# Bevy Studio GUI 自动验收
#
# 一条命令跑完整矩阵：4 个内置工作区 × 3 个内置主题 × 2 种语言，
# 外加"主题编辑器打开"与"窄窗口"两个特殊场景。
#
# 每个场景都会：
#   1. 启动真实应用（独立临时配置目录，不碰真实用户配置）；
#   2. 让应用在指定帧把**渲染目标**回读成 PNG，并写出布局证据 JSON；
#   3. 由 gui_probe 确认窗口、做像素与几何断言，输出判定与报告。
#
# 退出码：任意场景失败则整体非零（0 全部通过 / 2 界面缺陷 / 3 采集失败）。
#
# 用法（仓库根执行）：
#   pwsh tools/gui_acceptance.ps1
#   pwsh tools/gui_acceptance.ps1 -Filter theme     # 只跑名字含 theme 的场景
#   pwsh tools/gui_acceptance.ps1 -Exe path\to\bevy-studio.exe

param(
    [string]$Exe = "",
    [string]$Filter = "",
    [string]$OutRoot = ".verify/runs/acceptance"
)

$ErrorActionPreference = "Stop"
$root = (Get-Location).Path
if ([string]::IsNullOrWhiteSpace($Exe)) {
    $Exe = Join-Path (Split-Path (Split-Path $root -Parent) -Parent) "_build\target\debug\bevy-studio.exe"
    # 兜底：工作区共享 target 目录（见 C:\Spaces\Share\10_Code\10_Bevy\.cargo\config.toml）
    if (-not (Test-Path $Exe)) {
        $Exe = "C:\Spaces\Share\10_Code\10_Bevy\_build\target\debug\bevy-studio.exe"
    }
}
$probe = Join-Path (Split-Path $Exe -Parent) "gui_probe.exe"

if (-not (Test-Path $Exe)) { throw "找不到应用：$Exe（先运行 cargo build -p editor_app）" }
if (-not (Test-Path $probe)) { throw "找不到验收工具：$probe（先运行 cargo build -p gui_probe）" }

$outRoot = Join-Path $root $OutRoot
New-Item -ItemType Directory -Force -Path $outRoot | Out-Null

# 每个场景：名称、工作区、主题、语言、期望明暗、必须可见面板、额外环境变量、窗口尺寸
$scenarios = @(
    @{ Name = "general_zh_dark";    Ws = "general"; Theme = "dark";          Lang = "zh"; Luma = "dark";  Panels = "scene_tree,viewport,inspector,asset_browser" },
    @{ Name = "general_en_dark";    Ws = "general"; Theme = "dark";          Lang = "en"; Luma = "dark";  Panels = "scene_tree,viewport,inspector" },
    @{ Name = "general_zh_light";   Ws = "general"; Theme = "light";         Lang = "zh"; Luma = "light"; Panels = "scene_tree,viewport,inspector" },
    @{ Name = "general_zh_hc";      Ws = "general"; Theme = "high_contrast"; Lang = "zh"; Luma = "dark";  Panels = "scene_tree,viewport,inspector" },
    @{ Name = "scene_zh_dark";      Ws = "scene";   Theme = "dark";          Lang = "zh"; Luma = "dark";  Panels = "scene_tree,viewport,inspector,console" },
    @{ Name = "assets_zh_dark";     Ws = "assets";  Theme = "dark";          Lang = "zh"; Luma = "dark";  Panels = "asset_browser,viewport,inspector" },
    @{ Name = "debug_zh_dark";      Ws = "debug";   Theme = "dark";          Lang = "zh"; Luma = "dark";  Panels = "console,viewport,performance" },
    @{ Name = "theme_editor_zh";    Ws = "general"; Theme = "dark";          Lang = "zh"; Luma = "dark";  Panels = "scene_tree,viewport"; Extra = @{ "BEVY_STUDIO_OPEN_THEME_EDITOR" = "1" } },
    @{ Name = "narrow_en_light";    Ws = "general"; Theme = "light";         Lang = "en"; Luma = "light"; Panels = "scene_tree,viewport,inspector"; Size = "1024x700" }
)

$failed = @()
$passed = 0
foreach ($s in $scenarios) {
    if ($Filter -and ($s.Name -notlike "*$Filter*")) { continue }
    $out = Join-Path $outRoot $s.Name
    $cfg = Join-Path $outRoot ("cfg_" + $s.Name)
    $extra = ""
    if ($s.ContainsKey("Extra")) {
        foreach ($k in $s.Extra.Keys) { $extra += " --env $k=$($s.Extra[$k])" }
    }
    $size = ""
    if ($s.ContainsKey("Size")) { $size = " --window $($s.Size)" }

    $arguments = @(
        "run", "--exe", "`"$Exe`"", "--out", "`"$out`"", "--name", $s.Name,
        "--timeout", "150", "--env", "APPDATA=`"$cfg`"",
        "--env", "BEVY_STUDIO_WORKSPACE=$($s.Ws)",
        "--env", "BEVY_STUDIO_THEME=$($s.Theme)",
        "--env", "BEVY_STUDIO_LANGUAGE=$($s.Lang)",
        "--expect-language", $s.Lang, "--expect-theme", $s.Luma,
        "--expect-workspace", $s.Ws, "--expect-panels", $s.Panels
    ) -join " "
    $output = & cmd /c "`"$probe`" $arguments $extra $size 2>&1"
    $verdict = ($output | Select-String -Pattern "判定：" | Select-Object -First 1).Line
    $code = $LASTEXITCODE
    if ($code -eq 0) {
        $passed++
        Write-Host ("[通过] {0,-20} {1}" -f $s.Name, $verdict)
    } else {
        $failed += $s.Name
        Write-Host ("[失败] {0,-20} {1}" -f $s.Name, $verdict)
        $output | Select-String -Pattern "\[失败\]" | ForEach-Object { Write-Host ("        " + $_.Line) }
    }
}

# ---------------------------------------------------------------------------
# 重启后恢复：同一份配置目录连跑两次，第二次不给任何环境覆盖
# ---------------------------------------------------------------------------
if (-not $Filter -or "restart" -like "*$Filter*") {
    $cfg = Join-Path $outRoot "cfg_restart"
    Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue
    $first = Join-Path $outRoot "restart_first"
    $second = Join-Path $outRoot "restart_second"
    $null = & cmd /c "`"$probe`" run --exe `"$Exe`" --out `"$first`" --name restart_first --timeout 150 --env APPDATA=`"$cfg`" --env BEVY_STUDIO_THEME=light --env BEVY_STUDIO_WORKSPACE=scene --env BEVY_STUDIO_LANGUAGE=en --expect-theme light --expect-workspace scene 2>&1"
    $firstCode = $LASTEXITCODE
    Start-Sleep -Seconds 2
    $output = & cmd /c "`"$probe`" run --exe `"$Exe`" --out `"$second`" --name restart_second --timeout 150 --env APPDATA=`"$cfg`" --expect-theme light --expect-workspace scene --expect-language en 2>&1"
    $code = $LASTEXITCODE
    $verdict = ($output | Select-String -Pattern "判定：" | Select-Object -First 1).Line
    if ($firstCode -eq 0 -and $code -eq 0) {
        $passed++
        Write-Host ("[通过] {0,-20} {1}" -f "restart_restore", $verdict)
    } else {
        $failed += "restart_restore"
        Write-Host ("[失败] {0,-20} {1}" -f "restart_restore", $verdict)
        $output | Select-String -Pattern "\[失败\]" | ForEach-Object { Write-Host ("        " + $_.Line) }
    }
}

# ---------------------------------------------------------------------------
# 场景树交互：真实指针注入场景
#
# 与上面的场景不同，这批场景会**真的移动鼠标并点击**，因此它们验证的是交互
# 本身，而不是"某一帧画得对不对"。
#
# 用仓库内的固定示例工程 `tools/fixtures/tree_menu_project`（5 个实体，
# 行列表占不满视口），因此可复现、不依赖仓库外的临时工程。每个场景都会
# 复制一份到输出目录里跑，不会改动仓库内的固定工程。
#
# 每个场景的期望退出码（0 = 通过；2 = 界面缺陷）单独给出 —— 其中
# `tree_row_menu_negative` **故意期望 2**：它右键名称标签却断言"应该弹出
# 空白菜单"，只有工具真的判失败，才能证明断言在区分两种菜单、而不是空转。
# ---------------------------------------------------------------------------
if (-not $Filter -or "tree" -like "*$Filter*") {
    $fixture = Join-Path $root "tools\fixtures\tree_menu_project"
    if (-not (Test-Path (Join-Path $fixture "project.ron"))) {
        throw "缺少场景树验收用的固定示例工程：$fixture"
    }
    $treeProj = Join-Path $outRoot "fixture_tree_menu"
    Remove-Item -Recurse -Force $treeProj -ErrorAction SilentlyContinue
    Copy-Item -Recurse -Force $fixture $treeProj

    $treeScenarios = @(
        @{ Name = "tree_row_blank_menu";    Args = @("--right-click", "row_blank:容器", "--expect-menu", "row");       Expect = 0 },
        @{ Name = "tree_row_blank_select";  Args = @("--case", "row_blank:容器", "--expect-selection", "容器");        Expect = 0 },
        @{ Name = "tree_scene_blank_menu";  Args = @("--right-click", "scene_blank", "--expect-menu", "blank");        Expect = 0 },
        @{ Name = "tree_collapse_click";    Args = @("--case", "arrow_name:容器", "--expect-state", "scene_rows");     Expect = 0 },
        @{ Name = "tree_row_menu_negative"; Args = @("--right-click", "name:容器", "--expect-menu", "blank");          Expect = 2 }
    )

    foreach ($s in $treeScenarios) {
        if ($Filter -and ($s.Name -notlike "*$Filter*") -and ("tree" -notlike "*$Filter*")) { continue }
        $out = Join-Path $outRoot $s.Name
        $cfg = Join-Path $outRoot ("cfg_" + $s.Name)
        Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue
        # 直接以参数数组调用：项目路径与用例名都可能含中文，走 `cmd /c`
        # 会被 OEM 代码页破坏。
        #
        # 探针会把诊断写到 stderr，而本脚本开着 `ErrorActionPreference = Stop`：
        # 那样一条 stderr 就会被当成终止性错误，整个门禁直接中断。
        # 因此这里临时放宽，把它的输出（含 stderr）收进来用于显示。
        $probeArgs = @(
            "drag", "--exe", $Exe, "--out", $out, "--name", $s.Name,
            "--timeout", "150",
            "--env", "APPDATA=$cfg",
            "--env", "BEVY_STUDIO_OPEN_PROJECT=$treeProj",
            "--frame", "90", "--frame2", "210"
        ) + $s.Args
        $原错误偏好 = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        $output = & $probe @probeArgs 2>&1
        $ErrorActionPreference = $原错误偏好
        $code = $LASTEXITCODE
        $detail = ($output | Select-String -Pattern "菜单断言未通过|选中断言未通过|输入未产生任何变化|失败" | Select-Object -First 1).Line
        if ($code -eq $s.Expect) {
            $passed++
            Write-Host ("[通过] {0,-22} 退出码 {1}（期望 {1}）" -f $s.Name, $code)
        } else {
            $failed += $s.Name
            Write-Host ("[失败] {0,-22} 退出码 {1}（期望 {2}）" -f $s.Name, $code, $s.Expect)
            if ($detail) { Write-Host ("        " + $detail.Trim()) }
        }
    }
}

Write-Host ""
Write-Host ("通过 {0} 个场景，失败 {1} 个" -f $passed, $failed.Count)
if ($failed.Count -gt 0) {
    Write-Host ("失败场景：" + ($failed -join ", "))
    exit 2
}
exit 0
