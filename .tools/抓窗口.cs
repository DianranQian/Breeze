// 抓取指定窗口的客户区图像并编码为 PNG。
//
// 为什么用 P/Invoke 而不是 System.Drawing：
// 本机 PowerShell 会话里加载不到 System.Drawing 程序集，
// 因此改用 GDI32 手工建 DIB，再用 WIC（Windows Imaging Component）
// 编码 PNG。两者都是系统自带组件，不依赖 .NET 图形库。

using System;
using System.IO;
using System.Runtime.InteropServices;

public static class WindowShot
{
    private const int SRCCOPY = 0x00CC0020;
    private const uint PW_RENDERFULLCONTENT = 0x00000002;
    private const int BI_RGB = 0;

    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    private struct BITMAPINFOHEADER
    {
        public int biSize;
        public int biWidth;
        public int biHeight;
        public short biPlanes;
        public short biBitCount;
        public int biCompression;
        public int biSizeImage;
        public int biXPelsPerMeter;
        public int biYPelsPerMeter;
        public int biClrUsed;
        public int biClrImportant;
    }

    [DllImport("user32.dll")]
    private static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll")]
    private static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);

    [DllImport("user32.dll")]
    private static extern IntPtr GetDC(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

    [DllImport("user32.dll")]
    private static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateCompatibleDC(IntPtr hdc);

    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateDIBSection(
        IntPtr hdc, ref BITMAPINFOHEADER pbmi, uint usage,
        out IntPtr ppvBits, IntPtr hSection, uint offset);

    [DllImport("gdi32.dll")]
    private static extern IntPtr SelectObject(IntPtr hdc, IntPtr h);

    [DllImport("gdi32.dll")]
    private static extern bool DeleteObject(IntPtr ho);

    [DllImport("gdi32.dll")]
    private static extern bool DeleteDC(IntPtr hdc);

    // --- WIC ---
    [DllImport("ole32.dll")]
    private static extern int CoInitializeEx(IntPtr pvReserved, uint dwCoInit);

    [DllImport("ole32.dll")]
    private static extern void CoUninitialize();

    private static Guid CLSID_WICImagingFactory =
        new Guid("CACAF262-9370-4615-A13B-9F5539DA4C0A");

    [ComImport, Guid("ec5ec8a9-c395-4314-9c77-54d7a935ff70"),
     InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IWICImagingFactory
    {
        void CreateDecoderFromFilename(
            [MarshalAs(UnmanagedType.LPWStr)] string filename,
            IntPtr vendor, uint desiredAccess, uint metadataOptions, out IntPtr decoder);

        void CreateEncoder(
            ref Guid containerFormat, IntPtr vendor, out IntPtr encoder);

        void CreateStream(out IntPtr stream);

        void CreateStreamFromFile(
            [MarshalAs(UnmanagedType.LPWStr)] string filename, uint access, out IntPtr stream);

        void InitializeFromFilename(
            [MarshalAs(UnmanagedType.LPWStr)] string filename, uint access);
    }

    public static int Capture(IntPtr hwnd, string outputPath)
    {
        // 确保窗口可见且不在最小化状态
        ShowWindow(hwnd, 5); // SW_SHOW
        SetForegroundWindow(hwnd);

        RECT rect;
        if (!GetClientRect(hwnd, out rect)) return 1;
        int width = rect.Right - rect.Left;
        int height = rect.Bottom - rect.Top;
        if (width <= 0 || height <= 0) return 2;

        IntPtr windowDc = GetDC(hwnd);
        IntPtr memoryDc = CreateCompatibleDC(windowDc);

        var header = new BITMAPINFOHEADER();
        header.biSize = Marshal.SizeOf(typeof(BITMAPINFOHEADER));
        header.biWidth = width;
        header.biHeight = -height; // 负值表示自上而下的行序
        header.biPlanes = 1;
        header.biBitCount = 32;
        header.biCompression = BI_RGB;

        IntPtr bits;
        IntPtr dib = CreateDIBSection(
            windowDc, ref header, 0, out bits, IntPtr.Zero, 0);
        if (dib == IntPtr.Zero || bits == IntPtr.Zero)
        {
            ReleaseDC(hwnd, windowDc);
            DeleteDC(memoryDc);
            return 3;
        }

        IntPtr old = SelectObject(memoryDc, dib);

        // PW_RENDERFULLCONTENT 对 DirectX / Vulkan 渲染的窗口是必需的，
        // 否则抓到的会是全黑或未绘制的内容。
        PrintWindow(hwnd, memoryDc, PW_RENDERFULLCONTENT);

        int result = WriteBmp(bits, width, height, outputPath);

        SelectObject(memoryDc, old);
        DeleteObject(dib);
        DeleteDC(memoryDc);
        ReleaseDC(hwnd, windowDc);
        return result;
    }

    // 写 24 位 BMP。虽然比 PNG 大，但完全无依赖、格式无损，
    // 且不需要在 C# 里声明一整套 WIC COM 接口。
    private static int WriteBmp(IntPtr bits, int width, int height, string path)
    {
        int rowBytes = width * 4;              // 源是 32 位 BGRA
        int outRowBytes = ((width * 3) + 3) / 4 * 4; // BMP 行按 4 字节对齐
        int imageSize = outRowBytes * height;
        int fileSize = 14 + 40 + imageSize;

        byte[] buffer = new byte[fileSize];
        // BITMAPFILEHEADER
        buffer[0] = (byte)'B';
        buffer[1] = (byte)'M';
        BitConverter.GetBytes(fileSize).CopyTo(buffer, 2);
        BitConverter.GetBytes(14 + 40).CopyTo(buffer, 10);
        // BITMAPINFOHEADER
        BitConverter.GetBytes(40).CopyTo(buffer, 14);
        BitConverter.GetBytes(width).CopyTo(buffer, 18);
        BitConverter.GetBytes(height).CopyTo(buffer, 22); // 正值：自下而上
        BitConverter.GetBytes((short)1).CopyTo(buffer, 26);
        BitConverter.GetBytes((short)24).CopyTo(buffer, 28);
        BitConverter.GetBytes(0).CopyTo(buffer, 30);
        BitConverter.GetBytes(imageSize).CopyTo(buffer, 34);

        // 逐行拷贝并做 BGRA -> BGR
        byte[] source = new byte[rowBytes * height];
        Marshal.Copy(bits, source, 0, source.Length);
        for (int y = 0; y < height; y++)
        {
            int srcRow = y * rowBytes;
            // BMP 自下而上，因此倒序写入
            int dstRow = 14 + 40 + (height - 1 - y) * outRowBytes;
            for (int x = 0; x < width; x++)
            {
                buffer[dstRow + x * 3 + 0] = source[srcRow + x * 4 + 0]; // B
                buffer[dstRow + x * 3 + 1] = source[srcRow + x * 4 + 1]; // G
                buffer[dstRow + x * 3 + 2] = source[srcRow + x * 4 + 2]; // R
            }
        }

        File.WriteAllBytes(path, buffer);
        return 0;
    }
}
