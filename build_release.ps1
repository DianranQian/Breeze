# 微风（Breeze）发布打包脚本
#
# 做两件事：
#   1. 构建 release 版编辑器（cargo build --release）；
#   2. 把可执行文件与资源目录收集到 dist/，形成可直接复制分发的绿色版。
#
# 产物布局与 editor_core::asset_root 的资源查找规则一致：
#   dist/bevy-studio.exe
#   dist/bevy-studio.pdb      （存在时一并复制，崩溃报告才能定位符号）
#   dist/assets/              （源目录存在时）
#   dist/examples/            （仅 -WithExamples）
#   dist/build-info.txt
#
# 用法：
#   .\build_release.ps1
#   .\build_release.ps1 -SkipBuild
#   .\build_release.ps1 -OutDir D:\发布\微风
#   .\build_release.ps1 -AssetsDir examples\3d_demo\assets -WithExamples

[CmdletBinding()]
param(
    [string]$OutDir = "dist",
    [string]$AssetsDir = "assets",
    [switch]$SkipBuild,
    [switch]$WithExamples
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
