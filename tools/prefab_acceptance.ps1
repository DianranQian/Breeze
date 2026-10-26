# 微风 预制件（P3）交互闭环真实 GUI 验收
#
# 每个场景都启动**真实应用窗口**，由 `gui_probe` 用 `SendInput` 注入真实
# 鼠标/键盘事件，断言落在**结构化证据**上而不是截图观感：
#
#   1. 预制件面板点击模板行 → 场景里出现实例、新实例被选中、实例记录版本正确；
#   2. 场景树「待同步」箭头点击 → 实例记录的模板版本推进、蓝箭头消失；
#   3. 拖拽实例根操纵器 → 实体变换改变、恰好一条撤销记录、
#      **记录.放置**被写回（P3 之前只改运行时实体，保存后位置会丢），
#      并且 Ctrl+S 之后场景文件里的放置值确实变了。
#
# 坐标全部来自应用当帧测得的真实矩形（`prefab_rows[].row`、
# `scene_rows[].prefab_sync`、`gizmo_parts[].hit`），没有写死屏幕坐标；
# 断言认的是"实例记录/文档"的事实，因此"点到了复制图标"这类误判会被抓住。
#
# 已知边界：`gui_probe` 目前只能断言右键菜单的种类与菜单项，**无法点击菜单项**，
# 所以「脱钩节点回收/删除」这条 P3 冲突处理路径只能由集成测试覆盖，
# 本脚本不假装验证它。
#
# 用法（仓库根执行）：
#   powershell -ExecutionPolicy Bypass -File tools/prefab_acceptance.ps1
#   powershell -ExecutionPolicy Bypass -File tools/prefab_acceptance.ps1 -Filter sync
#   powershell -ExecutionPolicy Bypass -File tools/prefab_acceptance.ps1 -Exe target\debug\bevy-studio.exe

param(
    [string]$Exe = "",
    [string]$Filter = "",
    [string]$OutRoot = "target/verify/prefab"
)

$ErrorActionPreference = "Stop"
$root = (Get-Location).Path
if ([string]::IsNullOrWhiteSpace($Exe)) {
    $Exe = Join-Path $root "target\debug\bevy-studio.exe"
}
$probe = Join-Path (Split-Path $Exe -Parent) "gui_probe.exe"
if (-not (Test-Path $Exe)) { throw "找不到应用：$Exe（先运行 cargo build -p editor_app）" }
if (-not (Test-Path $probe)) { throw "找不到验收工具：$probe（先运行 cargo build -p gui_probe）" }

$outRoot = Join-Path $root $OutRoot
New-Item -ItemType Directory -Force -Path $outRoot | Out-Null

# 固定示例工程拷到输出目录：验收过程绝不改动仓库内的 fixture。
#
# 工程内容刻意做成"磁盘模板 v2、场景实例记录 v1"：应用一打开就有
# 一个真实的「待同步」实例，箭头不需要先手工造出来。
function 准备工程([string]$源, [string]$名) {
    $目标 = Join-Path $outRoot ("proj_" + $名)
    Remove-Item -Recurse -Force $目标 -ErrorAction SilentlyContinue
    Copy-Item -Recurse -Force (Join-Path $root $源) $目标
    return $目标
}

$passed = 0
$failed = @()

# 每个场景：名称、聚焦面板、点击用例、是否重置 2D 视图、拖拽用例、附加断言
$场景 = @(
    # 面板行点击：新建实例 + 自动选中实例根 + 实例记录版本 = 模板版本
    @{ Name = "panel_create"; Focus = "prefab_browser"; Resize = $false
       Case = "prefab:新兵"
       Expect = @("--expect-selection", "新兵",
                  "--expect-prefab-rows", "2",
                  "--expect-prefab-version", "新兵=1",
                  "--expect-prefab-pending", "新兵=no",
                  "--expect-dirty", "yes") }
    # 待同步箭头：记录里的模板版本从 1 推进到磁盘上的 2，箭头随之消失
    @{ Name = "sync_arrow"; Focus = "prefab_browser"; Resize = $false
       Case = "sync:敌兵"
       Expect = @("--expect-prefab-version", "敌兵=2",
                  "--expect-prefab-pending", "敌兵=no",
                  "--expect-dirty", "yes") }
    # 实例根拖拽：实体动了、只留一条撤销、放置写回记录，最后落盘
    #
    # 末帧在 Ctrl+S **之后**，所以脏状态应当是 `no`：拖拽产生的未保存修改
    # 已经被这次保存清掉。它同时也证明"保存真的执行了"，免得下面的落盘检查
    # 读到的是拖拽前那一份文件。
    @{ Name = "instance_drag"; Focus = ""; Resize = $true
       Select = "name:敌兵"
       Drag = "gizmo-drag:move_x:60,press:ctrls"
       Expect = @("--expect-transform", "translate",
                  "--expect-transform-unchanged", "scale",
                  "--expect-undo-delta", "1",
                  "--expect-dirty", "no",
                  "--expect-prefab-placement", "敌兵") }
)

foreach ($s in $场景) {
    if ($Filter -and ($s.Name -notlike "*$Filter*")) { continue }
    $out = Join-Path $outRoot $s.Name
    $cfg = Join-Path $outRoot ("cfg_" + $s.Name)
    $工程 = 准备工程 "tools/fixtures/prefab_project" $s.Name
    Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue

    $probeArgs = @(
        "drag", "--exe", $Exe, "--out", $out, "--name", $s.Name,
        "--timeout", "180",
        # 默认窗口 1600×950（逻辑）在 150% 缩放下是 2400×1425 物理像素，
        # 比本机 2560×1440 的可用高度还高 —— 底部标签组会落到屏幕外，
        # 点击"送不达"与"面板没反应"就分不开了。显式给一个一定放得下的尺寸。
        "--window", "1280x760",
        "--env", "APPDATA=$cfg",
        "--env", "BEVY_STUDIO_OPEN_PROJECT=$($工程)",
        "--env", "BEVY_STUDIO_MODE=2d",
        "--env", "BEVY_STUDIO_TOOL=move",
        "--frame", "90", "--frame2", "240", "--frame3", "400"
    )
    # 预制件面板默认是底部标签组里的**隐藏标签**，不聚焦就根本不会被绘制，
    # 证据里自然也没有行矩形。用启动环境变量把它切到前台。
    if ($s.Focus) { $probeArgs += @("--env", "BEVY_STUDIO_FOCUS_PANEL=$($s.Focus)") }
    if ($s.Select) { $probeArgs += @("--case", $s.Select) }
    # 打开工程会触发"2D 视角自动适应"，而那次自适应发生在首帧布局未稳时；
    # 显式点一次「重置视图」，操纵器几何才可预期（与变换验收同一套做法）。
    if ($s.Resize) { $probeArgs += @("--case2", "click:btn_reset_view_2d", "--case2-delay", "600") }
    if ($s.Case) { $probeArgs += @("--case", $s.Case) }
    if ($s.Drag) { $probeArgs += @("--case3", $s.Drag) }
    $probeArgs += $s.Expect

    $原错误偏好 = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $probe @probeArgs 2>&1
    $code = $LASTEXITCODE

    # 退出码 3 是"注入没送达/没赶上帧"（capture_failed），不是判定失败。
    # 只对 3 重试一次；判定失败（2）绝不重试，否则等于拿重试掩盖真实缺陷。
    if ($code -eq 3) {
        Write-Host ("[重试] {0,-16} 注入未送达，重跑一次" -f $s.Name)
        Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue
        $output = & $probe @probeArgs 2>&1
        $code = $LASTEXITCODE
    }
    $ErrorActionPreference = $原错误偏好

    if ($code -eq 0) {
        $passed++
        Write-Host ("[通过] {0,-16} 退出码 0" -f $s.Name)
    } else {
        $failed += $s.Name
        Write-Host ("[失败] {0,-16} 退出码 {1}" -f $s.Name, $code)
        $output | Select-String -Pattern "未通过|失败|取不到坐标" | Select-Object -First 3 |
            ForEach-Object { Write-Host ("        " + $_.Line.Trim()) }
    }

    # 拖拽场景额外核对"落盘"：内存里的放置对得上还不够，
    # 保存后的场景文件里 `放置.平移` 必须也是拖拽后的值。
    if ($code -eq 0 -and $s.Drag) {
        $场景文件 = Join-Path $工程 "scenes\prefab_demo.scene.ron"
        $放置 = $null
        try {
            $文本 = Get-Content $场景文件 -Raw -Encoding UTF8
            $匹配 = [regex]::Match($文本, '放置:\s*\(\s*平移:\s*\(([^)]*)\)')
            if ($匹配.Success) {
                $放置 = $匹配.Groups[1].Value -split ',' |
                    ForEach-Object { [double]($_.Trim()) }
            }
        } catch { }
        if ($放置 -and $放置.Count -eq 3 -and $放置[0] -gt 1.0 -and
            [math]::Abs($放置[1]) -lt 0.01 -and [math]::Abs($放置[2]) -lt 0.01) {
            $passed++
            Write-Host ("[通过] {0,-16} 落盘放置 = {1}" -f "drag_persist", ($放置 -join ", "))
        } else {
            $failed += "drag_persist"
            Write-Host ("[失败] {0,-16} 落盘放置 = {1}" -f "drag_persist",
                ($(if ($放置) { $放置 -join ", " } else { "<未读取到>" })))
        }
    }
}

Write-Host ""
Write-Host ("通过 {0} 项，失败 {1} 项" -f $passed, $failed.Count)
if ($failed.Count -gt 0) {
    Write-Host ("失败项：" + ($failed -join ", "))
    exit 2
}
exit 0
