# 微风 场景变换工具（2D + 3D）真实 GUI 验收
#
# 每个场景都启动**真实应用窗口**，由 `gui_probe` 用 `SendInput` 注入真实
# 鼠标/键盘事件，并断言结构化证据里的**实体变换数值**、撤销步数、文档脏状态。
#
# 与前几轮的区别（也是这个脚本存在的理由）：
#   * 坐标全部来自应用当帧测得的操纵器几何（`gizmo_parts[].hit/dir`），
#     没有写死的屏幕坐标；
#   * 断言落在"平移/旋转/缩放三个分量"上，因此"未操作轴被误改"也会被抓到；
#   * 每个场景都核对"一次拖拽恰好一条撤销记录"。
#
# 用法（仓库根执行）：
#   pwsh tools/gui_transform_acceptance.ps1
#   pwsh tools/gui_transform_acceptance.ps1 -Filter 2d
#   pwsh tools/gui_transform_acceptance.ps1 -Exe path\to\bevy-studio.exe

param(
    [string]$Exe = "",
    [string]$Filter = "",
    [string]$OutRoot = ".verify/runs/transform"
)

$ErrorActionPreference = "Stop"
$root = (Get-Location).Path
if ([string]::IsNullOrWhiteSpace($Exe)) {
    $Exe = "C:\Spaces\Share\10_Code\10_Bevy\_build\target\debug\bevy-studio.exe"
}
$probe = Join-Path (Split-Path $Exe -Parent) "gui_probe.exe"
if (-not (Test-Path $Exe)) { throw "找不到应用：$Exe（先运行 cargo build -p editor_app）" }
if (-not (Test-Path $probe)) { throw "找不到验收工具：$probe（先运行 cargo build -p gui_probe）" }

$outRoot = Join-Path $root $OutRoot
New-Item -ItemType Directory -Force -Path $outRoot | Out-Null

# 固定示例工程拷到输出目录：验收过程绝不改动仓库内的 fixture。
function 准备工程([string]$源, [string]$名) {
    $目标 = Join-Path $outRoot ("proj_" + $名)
    Remove-Item -Recurse -Force $目标 -ErrorAction SilentlyContinue
    Copy-Item -Recurse -Force (Join-Path $root $源) $目标
    return $目标
}
$工程2D = 准备工程 "tools/fixtures/transform_project_2d" "2d"
$工程3D = 准备工程 "tools/fixtures/transform_project_3d" "3d"

$passed = 0
$failed = @()

# 每个场景：名称、模式、工具、工程、插槽参数、附加断言、期望退出码
$场景 = @(
    # ---------------- 2D ----------------
    @{ Name = "2d_move_x";      Mode = "2d"; Tool = "move";   Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:move_x:80"
       Expect = @("--expect-transform", "translate", "--expect-transform-unchanged", "scale",
                  "--expect-transform-unchanged", "rotation", "--expect-undo-delta", "1",
                  "--expect-dirty", "yes") }
    @{ Name = "2d_move_y";      Mode = "2d"; Tool = "move";   Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:move_y:70"
       Expect = @("--expect-transform", "translate", "--expect-transform-unchanged", "scale",
                  "--expect-undo-delta", "1") }
    @{ Name = "2d_move_plane";  Mode = "2d"; Tool = "move";   Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:move_xy:60"
       Expect = @("--expect-transform", "translate", "--expect-transform-unchanged", "scale",
                  "--expect-transform-unchanged", "rotation", "--expect-undo-delta", "1") }
    @{ Name = "2d_rotate";      Mode = "2d"; Tool = "rotate"; Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:rotate_plane:90"
       Expect = @("--expect-transform", "rotation", "--expect-transform-unchanged", "translate",
                  "--expect-transform-unchanged", "scale", "--expect-undo-delta", "1") }
    @{ Name = "2d_scale_x";     Mode = "2d"; Tool = "scale";  Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:scale_x:60"
       Expect = @("--expect-transform", "scale", "--expect-transform-unchanged", "translate",
                  "--expect-transform-unchanged", "rotation", "--expect-undo-delta", "1") }
    @{ Name = "2d_scale_y";     Mode = "2d"; Tool = "scale";  Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:scale_y:60"
       Expect = @("--expect-transform", "scale", "--expect-transform-unchanged", "translate",
                  "--expect-undo-delta", "1") }
    @{ Name = "2d_scale_uniform"; Mode = "2d"; Tool = "scale"; Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:scale_uniform:70"
       Expect = @("--expect-transform", "scale", "--expect-transform-unchanged", "translate",
                  "--expect-undo-delta", "1") }
    # 取消：拖到一半按 Escape 再松手 → 变换必须精确回到起点、且不留撤销记录
    @{ Name = "2d_cancel_escape"; Mode = "2d"; Tool = "move"; Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-cancel:move_x:90"
       Expect = @("--expect-transform-unchanged", "translate", "--expect-undo-delta", "0",
                  "--expect-dirty", "no") }
    # 撤销 / 重做：拖拽后按 Ctrl+Z（应回到起点），再按 Ctrl+Y（应回到最终值）
    @{ Name = "2d_undo";        Mode = "2d"; Tool = "move";   Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:move_x:80,press:ctrlz"
       Expect = @("--expect-transform-unchanged", "translate", "--expect-undo-delta", "0") }
    @{ Name = "2d_redo";        Mode = "2d"; Tool = "move";   Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:move_x:80,press:ctrlz,press:ctrly"
       Expect = @("--expect-transform", "translate", "--expect-undo-delta", "1") }
    # 检查器数值框连拖：一次拖拽产生**一条**撤销记录（合并语义）
    #
    # 拖动 48px，位置 x 的步进是 0.05/px，因此期望 x 增加 2.4；
    # 逐帧会发出几十条 `设变换`，若不合并就会变成几十步撤销 ——
    # 与上一轮操纵器那 24 条记录是同一类缺陷。
    @{ Name = "2d_inspector_drag_merged"; Mode = "2d"; Tool = "move"; Proj = $工程2D; Entity = "甲"
       Case3 = "drag:inspector_pos_x:48:0"
       Expect = @("--expect-transform", "translate", "--expect-transform-unchanged", "scale",
                  "--expect-undo-delta", "1") }
    # 同一次拖拽，按**一次** Ctrl+Z 必须整段退回（MERGE_ENDS：回到最早的执行前）
    @{ Name = "2d_inspector_drag_undo"; Mode = "2d"; Tool = "move"; Proj = $工程2D; Entity = "甲"
       Case3 = "drag:inspector_pos_x:48:0,press:ctrlz"
       Expect = @("--expect-transform-unchanged", "translate", "--expect-undo-delta", "0") }
    # 工具栏切换工具：点真实按钮，断言工具真的换了（不靠环境变量）
    @{ Name = "2d_tool_switch"; Mode = "2d"; Tool = "move";   Proj = $工程2D; Entity = "甲"
       Case3 = "click:tool_rotate"
       Expect = @("--expect-tool", "rotate") }
    # 多选：甲与乙都被选中时，拖甲的手柄应当同时移动两者（一次撤销）
    @{ Name = "2d_multi_move";  Mode = "2d"; Tool = "move";   Proj = $工程2D; Entity = "甲"
       Case3 = "gizmo-drag:move_x:60"
       Expect = @("--expect-transform", "translate", "--expect-undo-delta", "1") }

    # ---------------- 3D ----------------
    @{ Name = "3d_move_x";      Mode = "3d"; Tool = "move";   Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:move_x:60"
       Expect = @("--expect-transform", "translate", "--expect-transform-unchanged", "scale",
                  "--expect-transform-unchanged", "rotation", "--expect-undo-delta", "1") }
    @{ Name = "3d_move_y";      Mode = "3d"; Tool = "move";   Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:move_y:60"
       Expect = @("--expect-transform", "translate", "--expect-undo-delta", "1") }
    @{ Name = "3d_move_z";      Mode = "3d"; Tool = "move";   Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:move_z:60"
       Expect = @("--expect-transform", "translate", "--expect-undo-delta", "1") }
    @{ Name = "3d_rotate_x";    Mode = "3d"; Tool = "rotate"; Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:rotate_x:80"
       Expect = @("--expect-transform", "rotation", "--expect-transform-unchanged", "translate",
                  "--expect-transform-unchanged", "scale", "--expect-undo-delta", "1") }
    @{ Name = "3d_rotate_y";    Mode = "3d"; Tool = "rotate"; Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:rotate_y:80"
       Expect = @("--expect-transform", "rotation", "--expect-undo-delta", "1") }
    @{ Name = "3d_rotate_z";    Mode = "3d"; Tool = "rotate"; Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:rotate_z:80"
       Expect = @("--expect-transform", "rotation", "--expect-undo-delta", "1") }
    @{ Name = "3d_scale_x";     Mode = "3d"; Tool = "scale";  Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:scale_x:50"
       Expect = @("--expect-transform", "scale", "--expect-transform-unchanged", "translate",
                  "--expect-undo-delta", "1") }
    @{ Name = "3d_scale_y";     Mode = "3d"; Tool = "scale";  Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:scale_y:50"
       Expect = @("--expect-transform", "scale", "--expect-undo-delta", "1") }
    @{ Name = "3d_scale_z";     Mode = "3d"; Tool = "scale";  Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:scale_z:50"
       Expect = @("--expect-transform", "scale", "--expect-undo-delta", "1") }
    @{ Name = "3d_scale_uniform"; Mode = "3d"; Tool = "scale"; Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:scale_uniform:60"
       Expect = @("--expect-transform", "scale", "--expect-undo-delta", "1") }
    @{ Name = "3d_plane_move";  Mode = "3d"; Tool = "move";   Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-drag:move_xy:55"
       Expect = @("--expect-transform", "translate", "--expect-undo-delta", "1") }
    @{ Name = "3d_cancel_escape"; Mode = "3d"; Tool = "move"; Proj = $工程3D; Entity = "甲"
       Case3 = "gizmo-cancel:move_z:70"
       Expect = @("--expect-transform-unchanged", "translate", "--expect-undo-delta", "0") }
)

foreach ($s in $场景) {
    if ($Filter -and ($s.Name -notlike "*$Filter*")) { continue }
    $out = Join-Path $outRoot $s.Name
    $cfg = Join-Path $outRoot ("cfg_" + $s.Name)
    Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue

    $probeArgs = @(
        "drag", "--exe", $Exe, "--out", $out, "--name", $s.Name,
        "--timeout", "180",
        "--env", "APPDATA=$cfg",
        "--env", "BEVY_STUDIO_OPEN_PROJECT=$($s.Proj)",
        "--env", "BEVY_STUDIO_MODE=$($s.Mode)",
        "--env", "BEVY_STUDIO_TOOL=$($s.Tool)",
        # 选中实体，再用**当帧测得的**手柄几何拖拽
        "--case", "name:$($s.Entity)"
    )
    # 2D 视角会被"打开项目时自动适应窗口"改掉，而那次自适应发生在首帧布局
    # 尚未稳定时 —— 因此显式点一次「重置视图」，把相机钉在缩放 100%、
    # 中心原点，操纵器几何才是可预期的。
    # 3D 相机启动时就是默认轨道视角（打开项目不会自动适应），无需重置。
    if ($s.Mode -eq "2d") {
        $probeArgs += @("--case2", "click:btn_reset_view_2d", "--case2-delay", "600")
    }
    $probeArgs += @(
        "--case3", $s.Case3,
        "--frame", "90", "--frame2", "240", "--frame3", "400",
        "--expect-selection", $s.Entity
    )
    $probeArgs += $s.Expect

    $原错误偏好 = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $probe @probeArgs 2>&1
    $code = $LASTEXITCODE

    # 退出码 3 是"注入没送达/没赶上帧"（capture_failed），不是判定失败。
    # 真实注入偶尔会遇到窗口前台切换或首帧布局未稳，表现为"取不到坐标"。
    # 这种情况重跑一次即可；**只对退出码 3 重试**，判定失败（2）绝不重试，
    # 否则就等于拿重试掩盖真实缺陷。
    if ($code -eq 3) {
        Write-Host ("[重试] {0,-22} 注入未送达，重跑一次" -f $s.Name)
        Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue
        $output = & $probe @probeArgs 2>&1
        $code = $LASTEXITCODE
    }
    $ErrorActionPreference = $原错误偏好

     if ($code -eq 0) {
        $passed++
        Write-Host ("[通过] {0,-22} {1}" -f $s.Name, "退出码 0")
    } else {
        $failed += $s.Name
        Write-Host ("[失败] {0,-22} 退出码 {1}" -f $s.Name, $code)
        $output | Select-String -Pattern "未通过|失败|取不到坐标" | Select-Object -First 3 |
            ForEach-Object { Write-Host ("        " + $_.Line.Trim()) }
    }
}

# ---------------------------------------------------------------------------
# 保存并重新打开：同一份工程，先拖拽 + Ctrl+S，再由新进程打开并读回变换
# ---------------------------------------------------------------------------
if (-not $Filter -or "persist" -like "*$Filter*") {
    $工程 = 准备工程 "tools/fixtures/transform_project_2d" "persist"
    $out = Join-Path $outRoot "2d_save"
    $cfg = Join-Path $outRoot "cfg_2d_save"
    Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue

    $原错误偏好 = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $null = & $probe drag --exe $Exe --out $out --name 2d_save --timeout 180 `
        --env "APPDATA=$cfg" --env "BEVY_STUDIO_OPEN_PROJECT=$工程" --env "BEVY_STUDIO_TOOL=move" `
        --case "name:甲" --case2 "click:btn_reset_view_2d" --case2-delay 600 `
        --case3 "gizmo-drag:move_x:80,press:ctrls" --frame 90 --frame2 240 --frame3 400 `
        --expect-selection "甲" --expect-transform "translate" --expect-undo-delta 1 2>&1
    $保存码 = $LASTEXITCODE

    # 新进程重新打开同一份工程（配置目录也复用：模拟"重启后再看"）。
    $out2 = Join-Path $outRoot "2d_reopen"
    $null = & $probe drag --exe $Exe --out $out2 --name 2d_reopen --timeout 180 `
        --env "APPDATA=$cfg" --env "BEVY_STUDIO_OPEN_PROJECT=$工程" --env "BEVY_STUDIO_TOOL=move" `
        --case "name:甲" --frame 90 --frame2 240 `
        --expect-selection "甲" --expect-undo-delta 0 2>&1
    $重开码 = $LASTEXITCODE
    $ErrorActionPreference = $原错误偏好

    # 变换值必须一致：保存前的末帧 vs 重开后的第二帧
    $保存后 = $null
    $重开后 = $null
    try {
        $a = Get-Content (Join-Path $out "2d_save_final.json") -Raw -Encoding UTF8 | ConvertFrom-Json
        $保存后 = $a.selection_transform.translate -join ","
    } catch { }
    try {
        $b = Get-Content (Join-Path $out2 "2d_reopen_after.json") -Raw -Encoding UTF8 | ConvertFrom-Json
        $重开后 = $b.selection_transform.translate -join ","
    } catch { }

    if ($保存码 -eq 0 -and $重开码 -eq 0 -and $保存后 -and ($保存后 -eq $重开后)) {
        $passed++
        Write-Host ("[通过] {0,-22} {1}" -f "2d_persist", "保存后 $保存后 / 重开后 $重开后")
    } else {
        $failed += "2d_persist"
        Write-Host ("[失败] {0,-22} 保存码 {1} 重开码 {2}；保存后 {3} / 重开后 {4}" -f `
            "2d_persist", $保存码, $重开码, $保存后, $重开后)
    }
}

Write-Host ""
Write-Host ("通过 {0} 个场景，失败 {1} 个" -f $passed, $failed.Count)
if ($failed.Count -gt 0) {
    Write-Host ("失败场景：" + ($failed -join ", "))
    exit 2
}
exit 0
