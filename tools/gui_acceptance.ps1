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

Write-Host ""
Write-Host ("通过 {0} 个场景，失败 {1} 个" -f $passed, $failed.Count)
if ($failed.Count -gt 0) {
    Write-Host ("失败场景：" + ($failed -join ", "))
    exit 2
}
exit 0
