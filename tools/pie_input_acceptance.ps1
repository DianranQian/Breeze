# PIE 键盘路由 + 点击捕获的真实输入验收脚本（R13.1）
#
# 为什么用 PowerShell 而不是 gui_probe：
#   `gui_probe` 能注入鼠标、能截图、能读证据，但**不能注入键盘**。
#   键盘路由是本轮最核心的验收项，只能用操作系统级按键注入。
#
# 本脚本做的事（全部是真实输入，不是模拟资源）：
#   1. 启动微风并等待窗口；用 Alt 破前台锁后 SetForegroundWindow；
#   2. 在 PIE 画面区域**真实点击鼠标**（走 `点击画面 → 请求捕获` 这条路径）；
#   3. 真实按住 ← 800ms，再按住 → 500ms（验证按住、换向）；
#   4. 真实按 Esc，从日志确认已释放；
#   5. 应用在第二帧截图后自行退出。
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/pie_input_acceptance.ps1

param(
    [string]$Exe = "C:\Spaces\Share\10_Code\10_Bevy\_build\target\debug\bevy-studio.exe",
    [string]$Project = ".verify/pie_input/project",
    [string]$Out = ".verify/pie_input/run",
    [int]$CaptureFrame = 240,
    [int]$CaptureFrame2 = 420
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, IntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT p);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    public const uint LEFTDOWN = 0x0002;
    public const uint LEFTUP = 0x0004;
    public const uint KEYUP = 0x0002;
    public const byte VK_MENU = 0x12;
    public const byte VK_LEFT = 0x25;
    public const byte VK_RIGHT = 0x27;
    public const byte VK_ESCAPE = 0x1B;
}
"@

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$projectAbs = (Resolve-Path $Project).Path
$logPath = Join-Path $Out "app.log"
$errPath = Join-Path $Out "app.log.err"

$env:APPDATA = (Join-Path $root "$Out/cfg")
$env:BEVY_STUDIO_OPEN_PROJECT = $projectAbs
$env:BEVY_STUDIO_PIE = "1"
$env:BEVY_STUDIO_LANGUAGE = "zh"
$env:BEVY_STUDIO_CAPTURE_PNG = (Join-Path $root "$Out/shot_captured.png")
$env:BEVY_STUDIO_CAPTURE_JSON = (Join-Path $root "$Out/shot_captured.json")
$env:BEVY_STUDIO_CAPTURE_FRAME = "$CaptureFrame"
$env:BEVY_STUDIO_CAPTURE_PNG2 = (Join-Path $root "$Out/shot_after_esc.png")
$env:BEVY_STUDIO_CAPTURE_JSON2 = (Join-Path $root "$Out/shot_after_esc.json")
$env:BEVY_STUDIO_CAPTURE_FRAME2 = "$CaptureFrame2"
$env:BEVY_STUDIO_CAPTURE_EXIT = "1"

$proc = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $logPath -RedirectStandardError $errPath
Write-Host "已启动 PID=$($proc.Id)，等待窗口…"

$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 120; $i++) {
    Start-Sleep -Milliseconds 500
    $proc.Refresh()
    if ($proc.HasExited) { throw "应用提前退出：$($proc.ExitCode)" }
    $hwnd = $proc.MainWindowHandle
    if ($hwnd -ne [IntPtr]::Zero) { break }
}
if ($hwnd -eq [IntPtr]::Zero) { throw "等不到主窗口" }

# Windows 前台锁：直接 SetForegroundWindow 常常只闪任务栏。
# 先模拟一次 Alt 按下/抬起，让本进程"看起来像用户操作"。
[void][Win]::ShowWindow($hwnd, 9)
[Win]::keybd_event([Win]::VK_MENU, 0, 0, [UIntPtr]::Zero)
[Win]::keybd_event([Win]::VK_MENU, 0, [Win]::KEYUP, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 400
$foreground = [Win]::SetForegroundWindow($hwnd)
Write-Host "窗口=$hwnd 前台=$foreground"

$rect = New-Object Win+RECT
[void][Win]::GetClientRect($hwnd, [ref]$rect)
$origin = New-Object Win+POINT
$origin.X = 0; $origin.Y = 0
[void][Win]::ClientToScreen($hwnd, [ref]$origin)
$dpi = [Win]::GetDpiForWindow($hwnd)
$scale = if ($dpi -gt 0) { $dpi / 96.0 } else { 1.0 }
Write-Host "客户区 $(($rect.Right - $rect.Left))x$(($rect.Bottom - $rect.Top))，原点 ($($origin.X),$($origin.Y))，DPI=$dpi"

# PIE 画面在窗口内的逻辑位置：窗口默认 (640,210)，图像从窗口内 (20,100) 起，
# 尺寸 480×270 → 取图像中心。
$imageLocalX = 640 + 20 + 240
$imageLocalY = 210 + 100 + 135
$clickX = [int]($origin.X + $imageLocalX * $scale)
$clickY = [int]($origin.Y + $imageLocalY * $scale)
Write-Host "点击 PIE 画面（屏幕 $clickX,$clickY）"

Start-Sleep -Seconds 2
[void][Win]::SetCursorPos($clickX, $clickY)
Start-Sleep -Milliseconds 150
[Win]::mouse_event([Win]::LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 90
[Win]::mouse_event([Win]::LEFTUP, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 800

Write-Host "注入：按住 ← 800ms"
[Win]::keybd_event([Win]::VK_LEFT, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 800
[Win]::keybd_event([Win]::VK_LEFT, 0, [Win]::KEYUP, [UIntPtr]::Zero)

Start-Sleep -Milliseconds 400
Write-Host "注入：按住 → 500ms（换向）"
[Win]::keybd_event([Win]::VK_RIGHT, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 500
[Win]::keybd_event([Win]::VK_RIGHT, 0, [Win]::KEYUP, [UIntPtr]::Zero)

Start-Sleep -Milliseconds 900
Write-Host "注入：Esc 释放捕获"
[Win]::keybd_event([Win]::VK_ESCAPE, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 120
[Win]::keybd_event([Win]::VK_ESCAPE, 0, [Win]::KEYUP, [UIntPtr]::Zero)

$proc.WaitForExit(120000) | Out-Null
Write-Host "应用退出码：$($proc.ExitCode)"

Write-Host "--- 焦点与输入轨迹 ---"
Select-String -Path $errPath -Pattern "PIE 输入焦点|PIE 帧" |
    ForEach-Object { $_.Line } |
    Select-Object -First 40
