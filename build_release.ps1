# 微风（Breeze）发布打包脚本
#
# 做两件事：
#   1. 构建 release 版编辑器（cargo build --release）；
#   2. 把可执行文件与资源目录收集到 dist/，形成可直接复制分发的绿色版。
#
# 加 -ExportGame 时做第三件事（并跳过 dist/ 收集）：
#   把 exe + 当前工程 + game_manifest.ron 收进一个纯粹的 Game_Build/，
#   双击里面的 exe 直接进游戏，不启动任何编辑器界面。
#
# 产物布局与 editor_core::asset_root 的资源查找规则一致：
#   dist/bevy-studio.exe
#   dist/bevy-studio.pdb      （存在时一并复制，崩溃报告才能定位符号）
#   dist/assets/              （源目录存在时）
#   dist/examples/            （仅 -WithExamples）
#   dist/build-info.txt
#
# -ExportGame 的产物布局（与 editor_app::standalone 的判定规则一致：
# exe 同级有 game_manifest.ron 即以游戏身份启动）：
#   Game_Build/bevy-studio.exe
#   Game_Build/game_manifest.ron
#   Game_Build/project.ron
#   Game_Build/scenes/  assets/  prefabs/  animations/
#   Game_Build/build-info.txt
#
# 用法：
#   .\build_release.ps1
#   .\build_release.ps1 -SkipBuild
#   .\build_release.ps1 -OutDir D:\发布\微风
#   .\build_release.ps1 -AssetsDir examples\3d_demo\assets -WithExamples
#   .\build_release.ps1 -ExportGame
#   .\build_release.ps1 -ExportGame -ExportWindowed   （导出的包窗口模式启动）
#   .\build_release.ps1 -ExportGame -SkipBuild -ProjectDir examples\3d_demo -GameOutDir D:\发布\我的游戏

[CmdletBinding()]
param(
    [string]$OutDir = "dist",
    [string]$AssetsDir = "assets",
    [switch]$SkipBuild,
    [switch]$WithExamples,
    [switch]$ExportGame,
    [switch]$ExportWindowed,
    [string]$ProjectDir = "",
    [string]$GameOutDir = "Game_Build"
)

$ErrorActionPreference = "Stop"

# cargo 以 UTF-8 输出 JSON，而 Windows PowerShell 5.1 默认按控制台代码页解码
# 外部程序输出。仓库路径含中文，解码错位会让 JSON 里的引号被吞掉，
# ConvertFrom-Json 随即报"无效的 JSON 字符串"。这里统一按 UTF-8 读取。
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

$根 = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }
$可执行名称 = "bevy-studio.exe"
$包名 = "editor_app"
$目标名 = "bevy-studio"

function ResolvePath([string]$路径) {
    if ([System.IO.Path]::IsPathRooted($路径)) {
        return [System.IO.Path]::GetFullPath($路径)
    }
    return [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($根, $路径))
}

function TrimPath([string]$路径) {
    return $路径.TrimEnd([char[]]@([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar))
}

# RON（serde 生成）里的字符串是转义过的：Windows 路径写成 "C:\\x"。
# 这里还原成真实字符，避免反斜杠被当成字面量带进清单。
function UnescapeRon([string]$文本) {
    return $文本 -replace '\\\\', '\' -replace '\\"', '"'
}

# 从编辑器配置文件里读"当前打开的工程"，让 -ExportGame 不带参数时
# 导出的就是用户刚刚在编辑器里看着的那个项目。
# 先认 `最近项目`（当前会话打开的那个），再退到 `最近项目列表` 的第一项
# （越靠前越新）；两处都是 Option/Vec，RON 里可能写成 `Some("...")` 或直接 `"..."`。
function 取当前工程 {
    $目录 = if ($env:BEVY_STUDIO_CONFIG_DIR) {
        $env:BEVY_STUDIO_CONFIG_DIR
    }
    elseif ($env:APPDATA) {
        Join-Path $env:APPDATA "BevyStudio"
    }
    else {
        $null
    }
    if (-not $目录) { return $null }
    $配置 = Join-Path $目录 "config.ron"
    if (-not (Test-Path -LiteralPath $配置 -PathType Leaf)) { return $null }
    $文本 = Get-Content -LiteralPath $配置 -Raw -Encoding UTF8
    $候选列表 = @()
    if ($文本 -match '最近项目:\s*(?:Some\(\s*)?"([^"]+)"') {
        $候选列表 += UnescapeRon $Matches[1]
    }
    if ($文本 -match '最近项目列表:\s*\[\s*"([^"]+)"') {
        $候选列表 += UnescapeRon $Matches[1]
    }
    foreach ($候选 in $候选列表) {
        if (Test-Path -LiteralPath $候选 -PathType Container) { return $候选 }
    }
    return $null
}

# 写 game_manifest.ron：UTF-8 无 BOM。
# 带 BOM 时 ron 解析会直接失败，而那份清单是游戏启动的唯一入口，
# 所以这里不走 Set-Content -Encoding UTF8（Windows PowerShell 5.1 会加 BOM）。
function 写文本([string]$路径, [string]$内容) {
    $编码 = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($路径, $内容, $编码)
}

function 导出游戏包([string]$可执行路径, [string]$工程路径) {
    $目标 = TrimPath ([System.IO.Path]::GetFullPath((ResolvePath $GameOutDir)))
    $工程 = TrimPath ([System.IO.Path]::GetFullPath((ResolvePath $工程路径)))

    if ([string]::Equals($目标, $根全, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "游戏包目录不能是仓库根：$目标"
    }
    if ($根全.StartsWith($目标 + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "游戏包目录不能是仓库根的祖先（会清空整个仓库）：$目标"
    }
    if ([string]::Equals($目标, $工程, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "游戏包目录不能就是工程目录本身：$目标"
    }
    if ($工程.StartsWith($目标 + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "工程目录在游戏包目录里面，复制会自我嵌套：工程 $工程 / 输出 $目标"
    }

    $清单路径 = Join-Path $工程 "project.ron"
    if (-not (Test-Path -LiteralPath $清单路径 -PathType Leaf)) {
        throw "$工程 里没有 project.ron，不是微风工程，无法作为游戏导出"
    }

    # 主场景与设计分辨率都从 project.ron 读：导出流程不猜入口，
    # 读不出来就报错 —— 生成一个跑不起来的包比报错更难排查。
    $清单文本 = Get-Content -LiteralPath $清单路径 -Raw -Encoding UTF8
    $名称 = "微风游戏"
    if ($清单文本 -match '名称:\s*"([^"]+)"') { $名称 = UnescapeRon $Matches[1] }
    $设计 = @(1920, 1080)
    if ($清单文本 -match '设计分辨率:\s*\(\s*(\d+)\s*,\s*(\d+)\s*\)') {
        $设计 = @([int]$Matches[1], [int]$Matches[2])
    }
    $主场景 = $null
    if ($清单文本 -match '启动场景:\s*Some\("([^"]+)"\)') {
        $主场景 = (UnescapeRon $Matches[1]) -replace '\\', '/'
    }
    if (-not $主场景) {
        $场景目录 = Join-Path $工程 "scenes"
        $首个 = if (Test-Path -LiteralPath $场景目录 -PathType Container) {
            @(Get-ChildItem -LiteralPath $场景目录 -Recurse -File -Filter "*.scene.ron" | Sort-Object FullName)
        }
        else { @() }
        if ($首个.Count -eq 0) {
            throw "$工程 的 scenes/ 里没有任何场景；请先在编辑器里保存一个场景并设为启动场景"
        }
        $主场景 = ($首个[0].FullName.Substring($工程.Length).TrimStart('\', '/')) -replace '\\', '/'
    }
    $主场景绝对 = Join-Path $工程 ($主场景 -replace '/', [System.IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $主场景绝对 -PathType Leaf)) {
        throw "启动场景不存在：$主场景绝对"
    }

    Write-Host "==> 导出可玩包到 $目标"
    if (Test-Path -LiteralPath $目标) {
        Remove-Item -LiteralPath $目标 -Recurse -Force
    }
    New-Item -ItemType Directory -Path $目标 -Force | Out-Null

    Copy-Item -LiteralPath $可执行路径 -Destination $目标
    $调试符号 = [System.IO.Path]::ChangeExtension($可执行路径, ".pdb")
    if (Test-Path -LiteralPath $调试符号 -PathType Leaf) {
        Copy-Item -LiteralPath $调试符号 -Destination $目标
    }

    Copy-Item -LiteralPath $清单路径 -Destination $目标
    foreach ($子目录 in @("scenes", "assets", "prefabs", "animations")) {
        $源 = Join-Path $工程 $子目录
        if (Test-Path -LiteralPath $源 -PathType Container) {
            Copy-Item -LiteralPath $源 -Destination (Join-Path $目标 $子目录) -Recurse -Force
            Write-Host "    $子目录/"
        }
        elseif ($子目录 -eq "scenes") {
            throw "工程里没有 scenes/ 目录：$工程"
        }
    }

    $全屏 = $true
    if ($ExportWindowed) { $全屏 = $false }
    $清单内容 = @"
(
    格式版本: 1,
    名称: "$($名称 -replace '"', '\"')",
    主场景: "$主场景",
    设计分辨率: ($($设计[0]), $($设计[1])),
    全屏: $($全屏.ToString().ToLower()),
)
"@
    写文本 (Join-Path $目标 "game_manifest.ron") ($清单内容 + "`n")

    $文件 = @(Get-ChildItem -LiteralPath $目标 -Recurse -File)
    $总字节 = [int64]0
    foreach ($项 in $文件) { $总字节 += $项.Length }
    $信息 = New-Object System.Collections.Generic.List[string]
    $信息.Add("# 微风（Breeze）可玩包")
    $信息.Add("# 导出时间：$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
    $信息.Add("# 工程：$工程")
    $信息.Add("# 主场景：$主场景")
    $信息.Add("# 设计分辨率：$($设计[0]) x $($设计[1])")
    $信息.Add("# 全屏：$全屏")
    $信息.Add("# 文件数：$($文件.Count)")
    $信息.Add("# 总大小：$总字节 字节")
    foreach ($项 in ($文件 | Sort-Object FullName)) {
        $相对 = $项.FullName.Substring($目标.Length).TrimStart([char[]]@('\', '/'))
        $信息.Add(("{0}  {1}" -f $相对, $项.Length))
    }
    写文本 (Join-Path $目标 "build-info.txt") (($信息 -join "`n") + "`n")

    Write-Host ""
    Write-Host "==> 可玩包导出完成"
    Write-Host "    目录：$目标"
    Write-Host "    文件：$($文件.Count) 个（$([math]::Round($总字节 / 1MB, 1)) MB）"
    Write-Host "    启动：双击 bevy-studio.exe（同级有 game_manifest.ron，直接进游戏，不开编辑器）"
    Write-Host "    调试：bevy-studio.exe --windowed   退出：Esc"
}

$输出 = ResolvePath $OutDir
$根全 = TrimPath ([System.IO.Path]::GetFullPath($根))
$输出全 = TrimPath ([System.IO.Path]::GetFullPath($输出))

if ([string]::Equals($输出全, $根全, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "输出目录不能是仓库根：$输出"
}
if ($根全.StartsWith($输出全 + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "输出目录不能是仓库根的祖先（会清空整个仓库）：$输出"
}

$可执行 = $null
if (-not $SkipBuild) {
    Write-Host "==> cargo build --release -p $包名 --bin $目标名"
    $构建行 = @(
        & cargo build --release -p $包名 --bin $目标名 --message-format=json 2>&1 |
            ForEach-Object { "$_" }
    )
    $构建码 = $LASTEXITCODE
    if ($构建码 -ne 0) {
        foreach ($行 in $构建行) {
            if (-not $行.TrimStart().StartsWith("{")) {
                Write-Host $行
            }
        }
        foreach ($行 in $构建行) {
            if (-not $行.TrimStart().StartsWith("{")) { continue }
            try { $消息 = $行 | ConvertFrom-Json } catch { continue }
            if ($消息.message -and $消息.message.rendered) {
                Write-Host $消息.message.rendered
            }
        }
        throw "cargo build 失败（退出码 $构建码）"
    }
    foreach ($行 in $构建行) {
        if (-not $行.TrimStart().StartsWith("{")) {
            Write-Host $行
        }
    }
    foreach ($行 in $构建行) {
        if (-not $行.TrimStart().StartsWith("{")) { continue }
        try { $消息 = $行 | ConvertFrom-Json } catch { continue }
        if ($消息.reason -eq "compiler-artifact" -and
            $消息.target.name -eq $目标名 -and
            $消息.executable) {
            $可执行 = $消息.executable
        }
    }
}

if (-not $可执行) {
    $元数据文本 = (& cargo metadata --format-version 1 --no-deps 2>$null) -join "`n"
    if (-not $元数据文本) {
        throw "无法读取 cargo 元数据，找不到 target 目录"
    }
    $元数据 = $元数据文本 | ConvertFrom-Json
    if (-not $元数据.target_directory) {
        throw "cargo 元数据里没有 target_directory"
    }
    $可执行 = [System.IO.Path]::Combine($元数据.target_directory, "release", $可执行名称)
}

if (-not (Test-Path -LiteralPath $可执行 -PathType Leaf)) {
    throw "找不到 release 产物：$可执行`n请先运行：cargo build --release -p $包名 --bin $目标名"
}

if ($ExportGame) {
    $工程 = if ($ProjectDir) { $ProjectDir } else { 取当前工程 }
    if (-not $工程) {
        throw "既没给 -ProjectDir，也没在编辑器配置里找到当前打开的工程；请用 -ProjectDir 指定要导出的微风工程"
    }
    导出游戏包 $可执行 $工程
    return
}

Write-Host "==> 收集到 $输出"
if (Test-Path -LiteralPath $输出) {
    Remove-Item -LiteralPath $输出 -Recurse -Force
}
New-Item -ItemType Directory -Path $输出 -Force | Out-Null

Copy-Item -LiteralPath $可执行 -Destination $输出
$调试符号 = [System.IO.Path]::ChangeExtension($可执行, ".pdb")
if (Test-Path -LiteralPath $调试符号 -PathType Leaf) {
    Copy-Item -LiteralPath $调试符号 -Destination $输出
}

$资源源 = ResolvePath $AssetsDir
if (Test-Path -LiteralPath $资源源 -PathType Container) {
    Copy-Item -LiteralPath $资源源 -Destination (Join-Path $输出 "assets") -Recurse -Force
    Write-Host "    资源目录：$资源源"
}
else {
    Write-Warning "没有找到资源目录 $资源源，跳过（编辑器字体与图标已内嵌在 exe 中）"
}

if ($WithExamples) {
    $示例源 = Join-Path $根 "examples"
    if (Test-Path -LiteralPath $示例源 -PathType Container) {
        Copy-Item -LiteralPath $示例源 -Destination (Join-Path $输出 "examples") -Recurse -Force
        Write-Host "    示例项目：$示例源"
    }
    else {
        Write-Warning "没有找到示例目录 $示例源，跳过"
    }
}

$文件 = @(Get-ChildItem -LiteralPath $输出 -Recurse -File)
$总字节 = [int64]0
foreach ($项 in $文件) {
    $总字节 += $项.Length
}

$清单 = New-Object System.Collections.Generic.List[string]
$清单.Add("# 微风（Breeze）发布包")
$清单.Add("# 打包时间：$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
$清单.Add("# 可执行文件：$可执行名称")
$清单.Add("# 文件数：$($文件.Count)")
$清单.Add("# 总大小：$总字节 字节")
foreach ($项 in ($文件 | Sort-Object FullName)) {
    $相对 = $项.FullName.Substring($输出.Length).TrimStart([char[]]@('\', '/'))
    $清单.Add(("{0}  {1}" -f $相对, $项.Length))
}
Set-Content -LiteralPath (Join-Path $输出 "build-info.txt") -Value $清单 -Encoding UTF8

$总兆 = [math]::Round($总字节 / 1MB, 1)
Write-Host ""
Write-Host "==> 打包完成"
Write-Host "    目录：$输出"
Write-Host "    文件：$($文件.Count) 个（$总兆 MB）"
Write-Host "    启动：双击 $可执行名称；资源按 exe 同级的 assets/ 查找"
