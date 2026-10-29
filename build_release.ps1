# 微风（Breeze）发布打包脚本
#
# 做两件事：
#   1. 构建 release 版编辑器（cargo build --release）；
#   2. 把可执行文件与资源目录收集到 dist/，形成可直接复制分发的绿色版。
#
# 加 -ExportGame 时做第三件事（并跳过 dist/ 收集）：
#   把 exe + 工程里可达的场景/资源 + game_manifest.ron 收进一个纯粹的 Game_Build/，
#   双击里面的 exe 直接进游戏，不启动任何编辑器界面。
#   导出本身**不在这里实现**：脚本只调用刚构建出来的 exe 的
#   `--export-game`，包结构、清单字段与依赖闭包都来自 editor_core::package，
#   与编辑器界面的「打包」共用同一份实现。脚本不再解析 project.ron。
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
#   Game_Build/bevy-studio.pdb        （构建目录里有符号时）
#   Game_Build/game_manifest.ron
#   Game_Build/project.ron
#   Game_Build/scenes/                 （全部场景）
#   Game_Build/assets/                 （场景登记过的资源）
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
# 这里还原成真实字符 —— 只用于读编辑器 config.ron 里的"最近项目"，
# 工程清单（project.ron）不再由脚本解析。
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

# 导出交给 exe 自己：`--export-game` 走 editor_core::package::导出可玩包，
# 与编辑器「打包」共用同一份计划、复制与清单逻辑。
# 这里只负责把"相对仓库根的路径"解析成绝对路径，并转达 -ExportWindowed。
# 输出目录的安全判断（不能是工程或它的上级、产物不能在输出目录里）在 Rust 侧。
function 导出游戏包([string]$可执行路径, [string]$工程路径) {
    $目标 = [System.IO.Path]::GetFullPath((ResolvePath $GameOutDir))
    $工程 = [System.IO.Path]::GetFullPath((ResolvePath $工程路径))

    Write-Host "==> 导出可玩包到 $目标"
    $导出参数 = @("--export-game", $工程, $目标)
    if ($ExportWindowed) { $导出参数 += "--windowed" }
    & $可执行路径 @导出参数
    if ($LASTEXITCODE -ne 0) {
        throw "导出可玩包失败（退出码 $LASTEXITCODE）"
    }

    Write-Host ""
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
    # `2>&1` 会把 cargo 的进度（走 stderr）并进管道；Windows PowerShell 5.1
    # 在 ErrorActionPreference=Stop 下把 NativeCommandError 也当终止错误，
    # 于是"Compiling ..."第一行就让脚本停下。这里局部放宽偏好，
    # 构建是否成功仍由 $LASTEXITCODE 判定。
    $原错误偏好 = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $构建行 = @(
            & cargo build --release -p $包名 --bin $目标名 --message-format=json 2>&1 |
                ForEach-Object { "$_" }
        )
        $构建码 = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $原错误偏好
    }
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
