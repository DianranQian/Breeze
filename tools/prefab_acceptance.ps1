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
#   4. 嵌套：用带内层引用的战机模板建实例后选中**嵌套实例根**，
#      检查器画出「嵌套预制件实例」徽标，且配色等于主题信息色、
#      截图上那个矩形里确实有该色的墨迹（证据由应用自报，像素才是外部事实）；
#   5. 嵌套覆盖的地址域：选中内层模板里的子节点、在检查器拖位置数值框，
#      覆盖条数只能从 0 变到**内层记录**那一份，外层记录恒为 0；
#      Ctrl+S 之后落盘的场景文件里 `覆盖:` 也只出现在内层记录那一段。
#   6. 恢复该项默认值（P4-D）：先造出覆盖并保存，再**重新打开同一份工程**，
#      在检查器里点三枚分量恢复按钮，覆盖条数回到 0、位置写回模板值；
#      Ctrl+S 之后落盘文件里两段记录都不再有 `覆盖:`。
#   7. 组件字段覆盖（P4-D 补链）：夹具里预置一条根节点 `Sprite.水平翻转`
#      覆盖，选中实例根后检查器该字段行画出恢复按钮（探针标识
#      `inspector_revert_field.Sprite.水平翻转`），点击后覆盖条数归零、
#      实体值写回模板默认 false，Ctrl+S 后落盘文件里不再有 `覆盖:`。
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

# 每个场景：名称、fixture 工程、聚焦面板、点击用例、是否重置 2D 视图、拖拽用例、附加断言
$场景 = @(
    # 面板行点击：新建实例 + 自动选中实例根 + 实例记录版本 = 模板版本
    @{ Name = "panel_create"; Fixture = "tools/fixtures/prefab_project"
       Focus = "prefab_browser"; Resize = $false
       Case = "prefab:新兵"
       Expect = @("--expect-selection", "新兵",
                  "--expect-prefab-rows", "2",
                  "--expect-prefab-version", "新兵=1",
                  "--expect-prefab-pending", "新兵=no",
                  "--expect-dirty", "yes") }
    # 待同步箭头：记录里的模板版本从 1 推进到磁盘上的 2，箭头随之消失
    @{ Name = "sync_arrow"; Fixture = "tools/fixtures/prefab_project"
       Focus = "prefab_browser"; Resize = $false
       Case = "sync:敌兵"
       Expect = @("--expect-prefab-version", "敌兵=2",
                  "--expect-prefab-pending", "敌兵=no",
                  "--expect-dirty", "yes") }
    # 实例根拖拽：实体动了、只留一条撤销、放置写回记录，最后落盘
    #
    # 末帧在 Ctrl+S **之后**，所以脏状态应当是 `no`：拖拽产生的未保存修改
    # 已经被这次保存清掉。它同时也证明"保存真的执行了"，免得下面的落盘检查
    # 读到的是拖拽前那一份文件。
    @{ Name = "instance_drag"; Fixture = "tools/fixtures/prefab_project"
       Focus = ""; Resize = $true
       Select = "name:敌兵"
       Drag = "gizmo-drag:move_x:60,press:ctrls"
       Expect = @("--expect-transform", "translate",
                  "--expect-transform-unchanged", "scale",
                  "--expect-undo-delta", "1",
                  "--expect-dirty", "no",
                  "--expect-prefab-placement", "敌兵") }
    # 嵌套徽标：用战机模板建实例（内层核心随之展开），再选中**嵌套实例根**，
    # 检查器必须画出「嵌套预制件实例」那一档徽标，而且用的是主题信息色。
    #
    # 颜色单独断言，是因为「文本对、颜色错」这一档最骗人：徽标退成次要文字色
    # 之后，用户在层级里扫一眼分不出这是内层还是外层，改起来也就改错域。
    @{ Name = "nested_badge"; Fixture = "tools/fixtures/nested_prefab_project"
       Focus = "prefab_browser"; Resize = $false
       Case = "prefab:战机"
       Case3 = "name:引擎座"
       Expect = @("--expect-prefab-rows", "2",
                  "--expect-prefab-nested", "引擎座=yes",
                  "--expect-prefab-overrides", "引擎座=0",
                  "--expect-prefab-badge", "prefab_nested_instance",
                  "--expect-prefab-badge-color", "info") }
    # 嵌套覆盖的地址域：选中**内层模板里的子节点**，在检查器里拖位置数值框，
    # 改动必须记进那一层的内层记录（层内相对地址），外层记录保持 0 条覆盖；
    # 最后 Ctrl+S，落盘的场景文件里也只能在内层记录那一段看到 `覆盖:`。
    #
    # 两条记录都"有覆盖"是布尔断言抓不到的缺陷，所以这里断言的是**条数**：
    # 内层 0→3、外层恒为 0，才说明"最深记录归属 + 层内相对地址"真的接对了。
    # 3 而不是 1：一次位置修改按 平移 / 旋转 / 缩放 三个分量各登记一条，
    # 粒度与 Inspector 的三行编辑一致，将来模板只改缩放时用户覆盖过的平移仍挡住同步。
    @{ Name = "nested_override"; Fixture = "tools/fixtures/nested_prefab_project"
       Focus = "prefab_browser"; Resize = $false
       Case = "prefab:战机"
       Case3 = "name:散热片,drag:inspector_pos_x:48:0,press:ctrls"
       Overrides = $true
       Expect = @("--expect-selection", "散热片",
                  "--expect-prefab-nested", "引擎座=yes",
                  "--expect-prefab-overrides", "引擎座=3",
                  "--expect-prefab-overrides", "战机=0",
                  "--expect-prefab-badge", "prefab_nested_node",
                  "--expect-prefab-badge-color", "info",
                  "--expect-transform", "translate",
                  "--expect-dirty", "no") }
    # 恢复该项默认值（P4-D）：**两遍真机**，因为按钮是拖拽之后才画出来的。
    #
    # 第一遍复刻 nested_override：造出 3 条内层覆盖并 Ctrl+S 落盘。
    # 第二遍重新打开**同一份已保存的工程**：覆盖在加载时就存在，于是
    # 插槽 1 选中子节点之后，第二帧证据里已经有恢复按钮的真实矩形，
    # 插槽 2 点它再 Ctrl+S，覆盖必须回到 0 条。
    #
    # 为什么不能一遍跑完：插槽 2 的坐标一律取自第二帧证据，而第一遍里
    # 按钮要到插槽 2 的拖拽之后才出现，第二帧证据里根本没有它 —— 取不到
    # 矩形只会报 capture_failed。拆成两遍还顺带证明覆盖真的落了盘：
    # 第二遍读到的覆盖来自磁盘，不是内存里残存的那份。
    @{ Name = "nested_revert"; Fixture = "tools/fixtures/nested_prefab_project"
       Focus = "prefab_browser"; Resize = $false
       Case = "prefab:战机"
       Case3 = "name:散热片,drag:inspector_pos_x:48:0,press:ctrls"
       Expect = @("--expect-prefab-overrides", "引擎座=3",
                  "--expect-dirty", "no")
       Second = @{
           Case = "name:散热片"
           # 三个分量各点一次：一次位置拖拽按 平移/旋转/缩放 三个分量各登记
           # 一条覆盖，恢复按钮也按分量出现（与三行编辑一一对应）。
           # 只点平移会留下 2 条覆盖 —— 那正是"恢复只删自己那一项"的证据，
           # 但本场景要断言的是"三条都点完 = 干净"，所以三枚按钮都点。
           # 三枚按钮的矩形都取自第二帧证据；点掉平移不会挪动另外两行。
           Case3 = "click:inspector_revert_pos,click:inspector_revert_rot,click:inspector_revert_scale,press:ctrls"
           Expect = @("--expect-selection", "散热片",
                      "--expect-prefab-overrides", "引擎座=0",
                      "--expect-prefab-overrides", "战机=0",
                      "--expect-transform", "translate",
                      "--expect-undo-delta", "3",
                      "--expect-dirty", "no")
       }
       Revert = $true }
    # 组件字段覆盖（P4-D 补链）：夹具里已经有一条 **根节点** 上的
    # `Sprite.水平翻转` 覆盖（模板默认 false、实例覆盖成 true），
    # 因此不需要先造覆盖 —— 选中实例根，第二帧证据里就该有该行的恢复按钮。
    #
    # 断言认的是探针标识 `inspector_revert_field.<组件>.<属性>`：它由组件注册表
    # 的字段名拼接（与变换三行手写的 `inspector_revert_pos` 那组不是同一套），
    # 点击"取到坐标"本身就证明组件行的标识真的按字段名登记了。
    #
    # 恢复必须做到两件事，缺一不可：覆盖条目归零、实体值写回模板默认 false。
    # 所以落盘核对同时查"不再有 `覆盖:`"与"水平翻转回到 false" ——
    # 只查前者时，"删了覆盖但实体仍是 true"的缺陷会漏过去（下次同步才跳回）。
    @{ Name = "component_revert"; Fixture = "tools/fixtures/component_override_project"
       Focus = ""; Resize = $false
       Case = "name:哨兵"
       Case3 = "click:inspector_revert_field.Sprite.水平翻转,press:ctrls"
       Expect = @("--expect-selection", "哨兵",
                  "--expect-prefab-overrides", "哨兵=0",
                  "--expect-undo-delta", "1",
                  "--expect-dirty", "no")
       RevertComponent = $true }
)

foreach ($s in $场景) {
    if ($Filter -and ($s.Name -notlike "*$Filter*")) { continue }
    $out = Join-Path $outRoot $s.Name
    $cfg = Join-Path $outRoot ("cfg_" + $s.Name)
    $工程 = 准备工程 $s.Fixture $s.Name
    Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue

    $probeArgs = @(
        "drag", "--exe", $Exe, "--out", $out, "--name", $s.Name,
        "--timeout", "180",
        # 默认窗口 1600×950（逻辑）在 150% 缩放下是 2400×1425 物理像素，
        # 比本机 2560×1440 的可用高度还高 —— 底部标签组会落到屏幕外，
        # 点击"送不达"与"面板没反应"就分不开了。显式给一个一定放得下的尺寸。
        "--window", "1280x760",
        # 第三帧必须晚于**插槽 2 的全部注入**：插槽 2 从第二帧证据开始，
        # 依次是"选中 → 拖拽（24 步 × 28ms）→ 每步后 450ms 沉降 → Ctrl+S"，
        # 合计约 2 秒。第三帧一到，应用就会截完图退出，注入被腰斩的表现是
        # 拖拽完全没生效（不是覆盖写错域）。1100 帧在 150～400 FPS 下都留有余量。
        "--env", "APPDATA=$cfg",
        "--env", "BEVY_STUDIO_OPEN_PROJECT=$($工程)",
        "--env", "BEVY_STUDIO_MODE=2d",
        "--env", "BEVY_STUDIO_TOOL=move",
        "--frame", "90", "--frame2", "240", "--frame3", "1100"
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
    # 插槽 2 也可以直接给一条用例串（多个动作以逗号分隔，按第二帧证据取坐标）。
    if ($s.Case3) { $probeArgs += @("--case3", $s.Case3) }
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
    # 嵌套覆盖的落盘核对：内存里的条数对得上还不够，保存后的场景文件里
    # `覆盖:` 只能出现在**内层记录**那一段。
    #
    # 为什么不能只看内存断言：把覆盖写进外层记录，运行时同样"有覆盖"，
    # 但下一次同步会按外层模板的节点地址去找那个内层地址 —— 找不到，
    # 用户改的位置被原样还原。这类静默失效只有读**落盘之后**的记录分段
    # 才分得开，所以这里按模板路径把两段记录切出来各自查。
    if ($code -eq 0 -and $s.Overrides) {
        $场景文件 = Join-Path $工程 "scenes\nested_demo.scene.ron"
        $内层有覆盖 = $null
        $外层无覆盖 = $null
        try {
            $文本 = Get-Content $场景文件 -Raw -Encoding UTF8
            $表 = [regex]::Match($文本, '(?s)预制件实例表: \{(.*?)\r?\n    \},')
            if ($表.Success) {
                # 每条记录从 `        "<uuid>": (` 起，到下一条或表尾止。
                $分段 = [regex]::Split($表.Groups[1].Value, '(?m)^        "[0-9a-fA-F-]+": \(')
                for ($i = 1; $i -lt $分段.Count; $i++) {
                    $段 = $分段[$i]
                    $有覆盖 = $段 -match '(?m)^            覆盖:'
                    if ($段 -match '路径: "prefabs/核心\.prefab\.ron"') { $内层有覆盖 = $有覆盖 }
                    if ($段 -match '路径: "prefabs/战机\.prefab\.ron"') { $外层无覆盖 = -not $有覆盖 }
                }
            }
        } catch { }
        if ($内层有覆盖 -eq $true -and $外层无覆盖 -eq $true) {
            $passed++
            Write-Host ("[通过] {0,-16} 落盘覆盖只在内层记录" -f "override_persist")
        } else {
            $failed += "override_persist"
            Write-Host ("[失败] {0,-16} 落盘覆盖：内层={1} 外层干净={2}（读不到=分段没切中）" -f
                "override_persist", $内层有覆盖, $外层无覆盖)
        }
    }

    # 组件字段覆盖的落盘核对：内存里 `覆盖` 归零还不够，保存后的场景文件
    # 必须同时满足两件事 —— 记录里不再有 `覆盖:`，实体字段回到模板默认 false。
    #
    # 只查其中一件都会漏掉一类真实缺陷：
    # * 只查"没有覆盖:"：删除覆盖但忘记写回实体时，界面显示 true，下次同步才跳回；
    # * 只查"水平翻转=false"：把整条实例记录删掉也能让文件干净，但实例关系没了。
    # 因此额外要求 `预制件实例表:` 仍在。
    if ($code -eq 0 -and $s.RevertComponent) {
        $场景文件 = Join-Path $工程 "scenes\component_demo.scene.ron"
        $无覆盖 = $null
        $回默认 = $null
        $保留记录 = $null
        try {
            $文本 = Get-Content $场景文件 -Raw -Encoding UTF8
            $无覆盖 = -not ($文本 -match '覆盖:')
            $回默认 = $文本 -match '"水平翻转": 布尔\(false\)'
            $保留记录 = $文本 -match '预制件实例表: \{'
        } catch { }
        if ($无覆盖 -eq $true -and $回默认 -eq $true -and $保留记录 -eq $true) {
            $passed++
            Write-Host ("[通过] {0,-16} 落盘无覆盖且回到模板默认值" -f "component_revert_persist")
        } else {
            $failed += "component_revert_persist"
            Write-Host ("[失败] {0,-16} 无覆盖={1} 回到默认={2} 保留记录={3}" -f
                "component_revert_persist", $无覆盖, $回默认, $保留记录)
        }
    }

    # 第二遍（可选）：在**同一份工程**上再启动一次应用。
    #
    # 用途只有一类：第二遍要验证的状态是第一遍**保存到磁盘**的产物
    # （覆盖在加载时就存在），而它不可能在同一遍会话里先用后测 ——
    # 插槽 2 的坐标取自第二帧证据，那时刻还没发生插槽 2 的拖拽。
    if ($code -eq 0 -and $s.Second) {
        $out2 = Join-Path $outRoot ($s.Name + "_second")
        Remove-Item -Recurse -Force $out2 -ErrorAction SilentlyContinue
        $probeArgs2 = @(
            "drag", "--exe", $Exe, "--out", $out2, "--name", $s.Name,
            "--timeout", "180",
            "--window", "1280x760",
            "--env", "APPDATA=$cfg",
            "--env", "BEVY_STUDIO_OPEN_PROJECT=$工程",
            "--env", "BEVY_STUDIO_MODE=2d",
            "--env", "BEVY_STUDIO_TOOL=move",
            "--frame", "90", "--frame2", "240", "--frame3", "1100"
        )
        if ($s.Second.Focus) { $probeArgs2 += @("--env", "BEVY_STUDIO_FOCUS_PANEL=$($s.Second.Focus)") }
        if ($s.Second.Resize) { $probeArgs2 += @("--case2", "click:btn_reset_view_2d", "--case2-delay", "600") }
        if ($s.Second.Case) { $probeArgs2 += @("--case", $s.Second.Case) }
        if ($s.Second.Case3) { $probeArgs2 += @("--case3", $s.Second.Case3) }
        $probeArgs2 += $s.Second.Expect

        $原错误偏好 = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        $output2 = & $probe @probeArgs2 2>&1
        $code2 = $LASTEXITCODE
        if ($code2 -eq 3) {
            Write-Host ("[重试] {0,-16} 注入未送达，重跑一次" -f ($s.Name + "_second"))
            Remove-Item -Recurse -Force $out2 -ErrorAction SilentlyContinue
            $output2 = & $probe @probeArgs2 2>&1
            $code2 = $LASTEXITCODE
        }
        $ErrorActionPreference = $原错误偏好

        if ($code2 -eq 0) {
            $passed++
            Write-Host ("[通过] {0,-16} 退出码 0" -f ($s.Name + "_second"))
        } else {
            $failed += ($s.Name + "_second")
            Write-Host ("[失败] {0,-16} 退出码 {1}" -f ($s.Name + "_second"), $code2)
            $output2 | Select-String -Pattern "未通过|失败|取不到坐标" | Select-Object -First 3 |
                ForEach-Object { Write-Host ("        " + $_.Line.Trim()) }
        }

        # 恢复之后落盘的核对：两段记录都不该再有 `覆盖:`。
        #
        # 内存里条数归零不代表文件干净：保存走的是同一条序列化路径，
        # 若空覆盖表被写回文件，下次加载会把"已恢复"的字段重新当成覆盖。
        if ($code2 -eq 0 -and $s.Revert) {
            $场景文件 = Join-Path $工程 "scenes\nested_demo.scene.ron"
            $内层干净 = $null
            $外层干净 = $null
            try {
                $文本 = Get-Content $场景文件 -Raw -Encoding UTF8
                $表 = [regex]::Match($文本, '(?s)预制件实例表: \{(.*?)\r?\n    \},')
                if ($表.Success) {
                    $分段 = [regex]::Split($表.Groups[1].Value, '(?m)^        "[0-9a-fA-F-]+": \(')
                    for ($i = 1; $i -lt $分段.Count; $i++) {
                        $段 = $分段[$i]
                        $有覆盖 = $段 -match '(?m)^            覆盖:'
                        if ($段 -match '路径: "prefabs/核心\.prefab\.ron"') { $内层干净 = -not $有覆盖 }
                        if ($段 -match '路径: "prefabs/战机\.prefab\.ron"') { $外层干净 = -not $有覆盖 }
                    }
                }
            } catch { }
            if ($内层干净 -eq $true -and $外层干净 -eq $true) {
                $passed++
                Write-Host ("[通过] {0,-16} 恢复后落盘不再有覆盖" -f "revert_persist")
            } else {
                $failed += "revert_persist"
                Write-Host ("[失败] {0,-16} 恢复后落盘：内层干净={1} 外层干净={2}" -f
                    "revert_persist", $内层干净, $外层干净)
            }
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
