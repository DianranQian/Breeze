//! 目标应用进程与窗口的确认（Win32）。
//!
//! # 为什么必须确认窗口
//!
//! "截图有内容"并不能证明截的是**这个应用的主窗口**。验收工具必须先从
//! 操作系统拿到窗口存在性、标题、类名、客户区尺寸与 DPI，才能把
//! "进程在跑但窗口没建起来"、"窗口被最小化成 0×0"、"截到的是另一个程序"
//! 这三种情况与真正的界面缺陷区分开。
//!
//! 这里只用最基础的 `user32` 查询，不抓屏：画面本身由进程内渲染目标回读，
//! 这条路径在 wgpu 应用上可靠，而 GDI 抓屏不可靠（见 `editor_app::capture` 的说明）。

/// 一次窗口查询的结果。
#[derive(Debug, Clone, Default)]
pub struct 窗口信息 {
    /// 窗口句柄（十进制文本，便于跨进程记录）。
    pub 句柄: String,
    /// 窗口标题。
    pub 标题: String,
    /// 窗口类名。
    pub 类名: String,
    /// 是否可见。
    pub 可见: bool,
    /// 客户区尺寸（物理像素）—— 这就是截图应该得到的尺寸。
    pub 客户区: (u32, u32),
    /// 含边框的窗口尺寸（物理像素）。
    pub 窗口矩形: (i32, i32, i32, i32),
    /// 窗口所在显示器的 DPI。
    pub dpi: u32,
}

impl 窗口信息 {
    /// 标题是否像 Bevy Studio 的主窗口。
    pub fn 是编辑器窗口(&self) -> bool {
        self.可见 && self.标题.contains("Bevy Studio")
    }
}

#[cfg(windows)]
mod 平台 {
    use super::窗口信息;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetClientRect, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible,
    };

    /// 枚举回调的上下文。
    struct 枚举上下文 {
        目标进程: u32,
        命中: Vec<isize>,
    }

    unsafe extern "system" fn 枚举回调(句柄: HWND, 参数: LPARAM) -> i32 {
        // SAFETY: `参数` 由本模块传入，指向一个在 `EnumWindows` 返回前
        // 一直存活的 `枚举上下文`。回调里只做查询，不改动系统状态。
        let 上下文 = unsafe { &mut *(参数 as *mut 枚举上下文) };
        let mut 进程号 = 0u32;
        unsafe { GetWindowThreadProcessId(句柄, &mut 进程号) };
        if 进程号 == 上下文.目标进程 {
            上下文.命中.push(句柄 as isize);
        }
        // 返回 1 = 继续枚举。多命中时后面按"可见 + 有标题"挑。
        1
    }

    fn 取标题(句柄: HWND) -> String {
        let mut 缓冲 = [0u16; 512];
        let 长度 = unsafe { GetWindowTextW(句柄, 缓冲.as_mut_ptr(), 缓冲.len() as i32) };
        if 长度 <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&缓冲[..长度 as usize])
    }

    fn 取类名(句柄: HWND) -> String {
        let mut 缓冲 = [0u16; 256];
        let 长度 = unsafe { GetClassNameW(句柄, 缓冲.as_mut_ptr(), 缓冲.len() as i32) };
        if 长度 <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&缓冲[..长度 as usize])
    }

    fn 取客户区(句柄: HWND) -> (u32, u32) {
        let mut 矩形 = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let 成功 = unsafe { GetClientRect(句柄, &mut 矩形) };
        if 成功 == 0 {
            return (0, 0);
        }
        (
            (矩形.right - 矩形.left).max(0) as u32,
            (矩形.bottom - 矩形.top).max(0) as u32,
        )
    }

    fn 取窗口矩形(句柄: HWND) -> (i32, i32, i32, i32) {
        let mut 矩形 = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let 成功 = unsafe { GetWindowRect(句柄, &mut 矩形) };
        if 成功 == 0 {
            return (0, 0, 0, 0);
        }
        (矩形.left, 矩形.top, 矩形.right, 矩形.bottom)
    }

    /// 声明本进程为"每显示器 DPI 感知"。
    ///
    /// # 为什么必须做
    ///
    /// DPI 不感知的进程，`GetClientRect` 返回的是**被 Windows 虚拟化过的**坐标：
    /// 在 150% 缩放的屏幕上，一个 2400×1425 物理像素的窗口会报告成 1600×950。
    /// 于是"截图尺寸"与"操作系统报告的客户区"永远对不上，工具会把一次
    /// 完全正常的采集误判成**采集失败** —— 实测中就是这样发现了这条断言在
    /// 150% 缩放下必然误报。
    ///
    /// 声明感知之后两者都是物理像素，比较才有意义。
    pub fn 提升dpi感知() {
        // 失败不是致命问题（老系统没有这个 API），但那样比较会退化成
        // 用 DPI 折算，调用方已经做了兜底。
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }

    /// 查找属于指定进程的主窗口。
    ///
    /// 选择规则：优先"可见 + 标题含 Bevy Studio"，
    /// 其次任意可见窗口，最后退回第一个枚举到的窗口 ——
    /// 这样即使标题被改动，工具仍然能报告窗口存在，而不是报"没有窗口"。
    pub fn 查找(进程号: u32) -> Option<窗口信息> {
        let mut 上下文 = 枚举上下文 {
            目标进程: 进程号,
            命中: Vec::new(),
        };
        // SAFETY: 回调与参数的生命周期都覆盖整次同步调用。
        let 成功 = unsafe {
            EnumWindows(Some(枚举回调), &mut 上下文 as *mut 枚举上下文 as LPARAM)
        };
        if 成功 == 0 && 上下文.命中.is_empty() {
            return None;
        }

        let mut 候选: Vec<窗口信息> = Vec::new();
        for 值 in &上下文.命中 {
            let 句柄 = *值 as HWND;
            let 信息 = 窗口信息 {
                句柄: format!("{值:#x}"),
                标题: 取标题(句柄),
                类名: 取类名(句柄),
                可见: unsafe { IsWindowVisible(句柄) } != 0,
                客户区: 取客户区(句柄),
                窗口矩形: 取窗口矩形(句柄),
                dpi: unsafe { GetDpiForWindow(句柄) },
            };
            候选.push(信息);
        }

        候选.sort_by_key(|信息| {
            // 排序权重：可见且标题匹配 > 可见 > 其他。
            let 权重 = if 信息.是编辑器窗口() {
                0
            } else if 信息.可见 {
                1
            } else {
                2
            };
            (权重, -(信息.客户区.0 as i64 * 信息.客户区.1 as i64))
        });
        候选.into_iter().next()
    }
}

#[cfg(windows)]
pub use 平台::提升dpi感知;
#[cfg(windows)]
pub use 平台::查找;

#[cfg(not(windows))]
pub use 非windows::查找;

#[cfg(not(windows))]
mod 非windows {
    use super::窗口信息;

    /// 非 Windows 平台不提供窗口确认，返回 `None`。
    ///
    /// 不伪造一个"看起来有窗口"的结果：验收工具会如实报告
    /// "本平台无法确认窗口"，而不是假装通过。
    pub fn 查找(_进程号: u32) -> Option<窗口信息> {
        None
    }
}
