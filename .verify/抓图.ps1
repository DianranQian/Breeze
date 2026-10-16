# 启动编辑器（可选打开示例项目）并抓取窗口像素，转成 PNG。
#
# 为什么需要这个脚本：编辑器窗口用 Vulkan 渲染，普通的屏幕截图
# 会因为窗口未置顶而抓到别的程序；`PrintWindow(PW_RENDERFULLCONTENT)`
# 能按窗口句柄抓到内容，因此这里走 GDI。
#
# 用法：
#   .\抓图.ps1                    # 直接启动
#   .\抓图.ps1 -项目 .verify\示例项目 -输出 proj.png

param(
    [string]$项目 = "",
    [string]$输出 = "shot.png",
    [int]$等待秒 = 16,
    [string]$窗口标题 = "bevy-studio"
)

$ErrorActionPreference = "Stop"
$仓库 = Split-Path -Parent $MyInvocation.MyCommand.Path
$可执行 = "C:\Spaces\Share\10_Code\10_Bevy\_build\target\debug\bevy-studio.exe"

if (-not (Test-Path $可执行)) { throw "找不到可执行文件：$可执行" }

Get-Process -Name $窗口标题 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400

if ($项目 -ne "") { $env:BEVY_STUDIO_OPEN_PROJECT = (Resolve-Path $项目).Path }
$日志 = Join-Path $env:TEMP "bevy-studio-$([guid]::NewGuid().ToString('N').Substring(0,6)).err"
$进程 = Start-Process -FilePath $可执行 -RedirectStandardError $日志 -PassThru
Write-Host "PID=$($进程.Id)  日志=$日志"

Start-Sleep -Seconds $等待秒
$进程.Refresh()
if ($进程.HasExited) { throw "编辑器已退出，退出码=$($进程.ExitCode)（日志见 $日志）" }

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class 窗口抓图 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr h);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr h, int w, int ht);
  [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr h, IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr h);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr h);
  [DllImport("gdi32.dll")] public static extern int GetDIBits(IntPtr dc, IntPtr bmp, uint s, uint n, byte[] bits, ref BIH bi, uint u);
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l,t,r,b; }
  [StructLayout(LayoutKind.Sequential)] public struct BIH {
    public int biSize; public int biWidth; public int biHeight; public short biPlanes; public short biBitCount;
    public int biCompression; public int biSizeImage; public int biXPelsPerMeter; public int biYPelsPerMeter;
    public int biClrUsed; public int biClrImportant; }
  public static string 抓(IntPtr hwnd, string 路径) {
    RECT r; GetWindowRect(hwnd, out r);
    int w = r.r-r.l, ht = r.b-r.t;
    if (w <= 0 || ht <= 0) return "窗口尺寸为 0";
    IntPtr wdc = GetDC(hwnd);
    IntPtr mdc = CreateCompatibleDC(wdc);
    IntPtr bmp = CreateCompatibleBitmap(wdc, w, ht);
    IntPtr old = SelectObject(mdc, bmp);
    // PW_RENDERFULLCONTENT：对 DirectX/Vulkan 渲染的窗口必需
    PrintWindow(hwnd, mdc, 2);
    var bi = new BIH();
    bi.biSize = 40; bi.biWidth = w; bi.biHeight = -ht;
    bi.biPlanes = 1; bi.biBitCount = 32; bi.biCompression = 0;
    byte[] buf = new byte[w*ht*4];
    GetDIBits(mdc, bmp, 0, (uint)ht, buf, ref bi, 0);
    System.IO.File.WriteAllBytes(路径 + ".raw", buf);
    System.IO.File.WriteAllText(路径 + ".size", w + " " + ht);
    SelectObject(mdc, old); DeleteObject(bmp); DeleteDC(mdc); ReleaseDC(hwnd, wdc);
    return w + "x" + ht;
  }
}
"@

$句柄 = (Get-Process -Id $进程.Id).MainWindowHandle
[void][窗口抓图]::SetForegroundWindow($句柄)
Start-Sleep -Milliseconds 1200
$路径 = Join-Path $仓库 ([System.IO.Path]::GetFileNameWithoutExtension($输出))
Write-Host "抓取: $([窗口抓图]::抓($句柄, $路径))"

Stop-Process -Id $进程.Id -Force
Remove-Item Env:\BEVY_STUDIO_OPEN_PROJECT -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 700

# 用 Pillow 把 DIB 转成 PNG。这里有两个坑：
#   1. GetDIBits 传负高度 => 已是自上而下，不要再翻转；
#   2. 32 位 BI_RGB 的 DIB 通道顺序是 BGRA，不是 RGBA。
#      按 RGBA 解码会让红蓝对调，从而把渲染正常的画面误判成
#      "红蓝通道 bug" —— 实测中确实因此白查了很久。
$py = @"
from PIL import Image
raw = open(r"$路径.raw","rb").read()
w,h = [int(x) for x in open(r"$路径.size").read().split()]
_im = Image.frombytes("RGBA",(w,h),raw[:w*h*4])
_b,_g,_r,_a = _im.split()
Image.merge("RGB",(_r,_g,_b)).save(r"$输出")
print("PNG:", r"$输出")
"@
$py | Set-Content "$env:TEMP\_conv.py" -Encoding UTF8
python "$env:TEMP\_conv.py"
Remove-Item "$路径.raw","$路径.size" -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "=== 日志要点 ==="
$文本 = [System.IO.File]::ReadAllText($日志, [System.Text.Encoding]::UTF8)
($文本 -split "`n") | Where-Object { $_ -match "字体|项目|场景|预览|WARN|ERROR|panick" } | Select-Object -First 10
