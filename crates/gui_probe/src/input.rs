//! Windows 真实鼠标输入注入。
//!
//! # 为什么用 `SendInput` 而不是消息投递
//!
//! `PostMessage(WM_MOUSEMOVE)` 只把消息塞进队列，winit 的鼠标捕获、
//! `SetCapture`、光标位置都拿不到，egui 里的拖动会表现为"按下了但不动"。
//! `SendInput` 走系统输入栈，与真人操作同一条路径，因此才能验证拖拽。
//!
//! # 坐标
//!
//! 本模块只接受**屏幕物理像素**。调用方负责：
//! 1. 逻辑坐标 × `pixels_per_point` → 客户区物理坐标；
//! 2. 加上客户区在屏幕上的原点 → 屏幕物理坐标。
//!
//! 这里再换算成 `SendInput` 要求的 0..=65535 虚拟桌面归一化坐标。
//! 换算用整个虚拟桌面（`MOUSEEVENTF_VIRTUALDESK`），多显示器也成立。

use std::thread::sleep;
use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// 一次拖拽的轨迹（屏幕物理像素）。
#[derive(Debug, Clone, Copy)]
pub struct 拖拽 {
    /// 起点。
    pub 从: (i32, i32),
    /// 终点。
    pub 到: (i32, i32),
    /// 中间移动步数。
    pub 步数: u32,
    /// 每一步之间的等待。
    pub 步间隔: Duration,
}

/// 注入结果：每一步都成功才算成功。
#[derive(Debug, Clone, Copy, Default)]
pub struct 注入结果 {
    /// 移动事件是否全部送达。
    pub 移动成功: bool,
    /// 按下是否成功。
    pub 按下成功: bool,
    /// 抬起是否成功。
    pub 抬起成功: bool,
    /// 中间移动的步数。
    pub 移动步数: u32,
}

impl 注入结果 {
    /// 整条链路是否完整。
    pub fn 完整(&self) -> bool {
        self.移动成功 && self.按下成功 && self.抬起成功
    }

    /// 供报告使用的一句话。
    pub fn 摘要(&self) -> String {
        format!(
            "move={} down={} up={} steps={}",
            self.移动成功, self.按下成功, self.抬起成功, self.移动步数
        )
    }
}

/// 执行一次按下—移动—抬起。
pub fn 执行拖拽(参数: 拖拽) -> 注入结果 {
    let mut 结果 = 注入结果::default();

    结果.移动成功 = 移动到(参数.从);
    sleep(Duration::from_millis(120));
    结果.按下成功 = 按下();
    sleep(Duration::from_millis(120));

    let 步数 = 参数.步数.max(1);
    let mut 全部移动 = 结果.移动成功;
    for 序号 in 1..=步数 {
        let 比例 = 序号 as f32 / 步数 as f32;
        let x = 参数.从.0 + ((参数.到.0 - 参数.从.0) as f32 * 比例).round() as i32;
        let y = 参数.从.1 + ((参数.到.1 - 参数.从.1) as f32 * 比例).round() as i32;
        if !移动到((x, y)) {
            全部移动 = false;
        }
        结果.移动步数 += 1;
        sleep(参数.步间隔);
    }
    结果.移动成功 = 全部移动;

    sleep(Duration::from_millis(120));
    结果.抬起成功 = 抬起();
    // 抬起后仍停在该位置：真实用户松手后指针也不会瞬移。
    let _ = 移动到(参数.到);
    结果
}

/// 移动指针到屏幕物理坐标。
pub fn 移动到(坐标: (i32, i32)) -> bool {
    发送鼠标(坐标, MOUSEEVENTF_MOVE)
}

/// 左键按下。
pub fn 按下() -> bool {
    发送鼠标(取当前指针(), MOUSEEVENTF_LEFTDOWN)
}

/// 左键抬起。
pub fn 抬起() -> bool {
    发送鼠标(取当前指针(), MOUSEEVENTF_LEFTUP)
}

/// 在当前指针位置单击一次。
pub fn 单击() -> bool {
    按下() && {
        sleep(Duration::from_millis(60));
        抬起()
    }
}

/// 读取当前指针位置（屏幕物理像素）。
pub fn 取当前指针() -> (i32, i32) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut 点 = POINT { x: 0, y: 0 };
        // SAFETY: 传出的指针指向本栈帧里的 POINT，调用期间有效。
        unsafe {
            GetCursorPos(&mut 点);
        }
        (点.x, 点.y)
    }
    #[cfg(not(windows))]
    {
        (0, 0)
    }
}

#[cfg(windows)]
fn 发送鼠标(坐标: (i32, i32), 额外标志: u32) -> bool {
    // SAFETY: 只读取系统度量；失败时退回 1920×1080 的主屏假设。
    let (左, 上, 宽, 高) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    };
    // 归一化到 0..=65535，并带上 VIRTUALDESK。
    let 归一 = |值: i32, 原点: i32, 尺寸: i32| -> i32 {
        let 相对 = (值 - 原点).clamp(0, 尺寸.saturating_sub(1));
        ((相对 as i64 * 65535) / 尺寸.saturating_sub(1).max(1) as i64) as i32
    };
    let 输入 = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 归一(坐标.0, 左, 宽),
                dy: 归一(坐标.1, 上, 高),
                mouseData: 0,
                dwFlags: MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | 额外标志,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: 输入数组只在本调用内使用；SendInput 会复制数据。
    let 已送 = unsafe { SendInput(1, &输入, std::mem::size_of::<INPUT>() as i32) };
    已送 == 1
}

#[cfg(not(windows))]
fn 发送鼠标(_坐标: (i32, i32), _额外标志: u32) -> bool {
    false
}
