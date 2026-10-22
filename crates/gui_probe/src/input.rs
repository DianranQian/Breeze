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
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
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
    /// 按下后是否先做 2px 预热移动。
    pub 预热: bool,
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
    let mut 结果 = 注入结果 {
        移动成功: 移动到(参数.从),
        ..Default::default()
    };

    sleep(Duration::from_millis(120));
    结果.按下成功 = 按下();
    sleep(Duration::from_millis(120));

    // 预热：先移动 2px 再回来。
    //
    // # 为什么需要
    //
    // 真实拖拽的第一次移动发生在按下之后；如果按下与第一段移动之间没有任何
    // 中间状态，egui 的拖动判定偶尔会把它当成"按下即松开"或忽略位移。
    // 2px 的往返不会触发任何有效命中变化，但会稳定地产生 press→move 序列。
    if 参数.预热 {
        let 预热点 = (参数.从.0 + 2, 参数.从.1 + 1);
        let _ = 移动到(预热点);
        sleep(Duration::from_millis(40));
        let _ = 移动到(参数.从);
        sleep(Duration::from_millis(40));
    }

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

/// 按下并抬起一次键盘按键（Windows 虚拟键码）。
///
/// # 为什么需要它
///
/// "按 Escape 取消拖拽"是一条真实存在的交互契约，而它**只能**用真实键盘
/// 事件验证：egui 的快捷键读取的是 `RawInput` 里的键盘事件，
/// 直接改应用内部状态既不真实、也测不到"快捷键有没有接上"。
///
/// 返回是否两次 `SendInput` 都送达。
/// 是否是"扩展键"（方向键、Home/End、Insert/Delete 等）。
///
/// Windows 的 `SendInput` 对这类键要求带 `KEYEVENTF_EXTENDEDKEY`，
/// 否则它们会被当成小键盘上的同名键，目标窗口收得到 `WM_KEYDOWN`
/// 但键码不对 —— 表现是"注入成功、应用毫无反应"。
///
/// 这是实测踩出来的：Esc（普通键）能生效，方向键（扩展键）完全没反应。
pub fn 是扩展键(虚拟键: u16) -> bool {
    matches!(
        虚拟键,
        0x21 | 0x22 | 0x23 | 0x24 | 0x25 | 0x26 | 0x27 | 0x28 | 0x2D | 0x2E | 0x90 | 0x91
    )
}

/// 发送一次键盘事件，自动补扩展键标志。
fn 发送键盘_自动(虚拟键码: u16, 标志: u32) -> bool {
    let 标志 = if 是扩展键(虚拟键码) {
        标志 | KEYEVENTF_EXTENDEDKEY
    } else {
        标志
    };
    发送键盘(虚拟键码, 标志)
}

/// 按下键盘（不松开）。配合 [`抬起键盘`] 实现"按住 N 毫秒"。
///
/// `执行按键` 只能表达"按一下"，而游戏输入验证需要"按住"——
/// 按住与按一下在 Bevy 里是 `pressed()` 与 `just_pressed()` 的区别，
/// 用按一下去验证持续移动会得到"动了一帧"的假结果。
pub fn 按下键盘(虚拟键: u16) -> bool {
    发送键盘_自动(虚拟键, 0)
}

/// 抬起键盘。
pub fn 抬起键盘(虚拟键: u16) -> bool {
    发送键盘_自动(虚拟键, KEYEVENTF_KEYUP)
}

pub fn 执行按键(虚拟键: u16) -> bool {
    let 按下 = 发送键盘_自动(虚拟键, 0);
    sleep(Duration::from_millis(60));
    let 抬起 = 发送键盘_自动(虚拟键, KEYEVENTF_KEYUP);
    sleep(Duration::from_millis(120));
    按下 && 抬起
}

/// 按下并抬起一次**组合键**（例如 Ctrl+Z）。
///
/// 顺序必须是"修饰键按下 → 主键按下 → 主键抬起 → 修饰键抬起"：
/// 先松开修饰键会让主键被当成独立按键（Ctrl+Z 变成 Z）。
pub fn 执行组合键(修饰键: u16, 主键: u16) -> bool {
    let 甲 = 发送键盘(修饰键, 0);
    sleep(Duration::from_millis(40));
    let 乙 = 发送键盘(主键, 0);
    sleep(Duration::from_millis(60));
    let 丙 = 发送键盘(主键, KEYEVENTF_KEYUP);
    sleep(Duration::from_millis(40));
    let 丁 = 发送键盘(修饰键, KEYEVENTF_KEYUP);
    sleep(Duration::from_millis(150));
    甲 && 乙 && 丙 && 丁
}

/// 发送一次键盘事件。
#[cfg(windows)]
fn 发送键盘(虚拟键码: u16, 标志: u32) -> bool {
    let 输入 = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 虚拟键码 as VIRTUAL_KEY,
                wScan: 0,
                dwFlags: 标志,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: `INPUT` 由本函数完整初始化；`SendInput` 只读它，
    // 且大小由 `size_of::<INPUT>()` 精确给出。
    let 已发送 = unsafe { SendInput(1, &输入, std::mem::size_of::<INPUT>() as i32) };
    已发送 == 1
}

#[cfg(not(windows))]
fn 发送键盘(_虚拟键码: u16, _标志: u32) -> bool {
    false
}

/// 在指定屏幕坐标完成一次**真实右键单击**。
///
/// 与左键同一套 `SendInput` 路径，只是换成 `RIGHTDOWN`/`RIGHTUP`。
/// 用于验证右键菜单这类"浮层"交互：菜单不会改变布局矩形，
/// 因此只能靠截图确认它真的弹出来了。
pub fn 执行右键单击(坐标: (i32, i32)) -> 注入结果 {
    let mut 结果 = 注入结果 {
        移动成功: 移动到(坐标),
        按下成功: false,
        抬起成功: false,
        移动步数: 0,
    };
    sleep(Duration::from_millis(130));
    结果.按下成功 = 发送鼠标(取当前指针(), MOUSEEVENTF_RIGHTDOWN);
    sleep(Duration::from_millis(80));
    结果.抬起成功 = 发送鼠标(取当前指针(), MOUSEEVENTF_RIGHTUP);
    sleep(Duration::from_millis(150));
    结果
}

/// 在指定屏幕坐标完成一次**真实单击**。
///
/// # 为什么"点一下"不能用拖拽流程代替（这是按钮点击验收的关键）
///
/// 用 [`执行拖拽`] 去"点"一个按钮会失败，而且是**静默失败**：
/// 拖拽流程按下后要等待预热、24 段移动、抬起，整个过程约 1.0 秒；
/// 而 egui 的 `InputOptions::max_click_duration` 默认 **0.8 秒** ——
/// 超过它就不再算点击（egui 源码里的原话：
/// "If the pointer is down for longer than this it will no longer register as a click"）。
///
/// 结果是：坐标完全正确、指针事件也确实到达了应用，但 `Response::clicked()`
/// 永远是 `false`。上一轮"Run 按钮点了没反应"就是这样被误判成产品缺陷的 ——
/// 断点不在产品里，在这条注入路径里。
///
/// 因此真实单击必须是：移动 → 按下 → **短暂停顿** → 抬起，全程远小于 0.8 秒。
pub fn 执行单击(坐标: (i32, i32)) -> 注入结果 {
    let mut 结果 = 注入结果 {
        移动成功: 移动到(坐标),
        按下成功: false,
        抬起成功: false,
        移动步数: 0,
    };
    // 指针必须先落在目标上，再按下：egui 的命中判定用的是上一帧的指针位置。
    sleep(Duration::from_millis(130));
    结果.按下成功 = 按下();
    // 80ms 的"按住"足够长到被真实计入，又远小于 egui 的 0.8s 点击上限。
    sleep(Duration::from_millis(80));
    结果.抬起成功 = 抬起();
    sleep(Duration::from_millis(80));
    结果
}

/// 在指定屏幕坐标发送滚轮。
///
/// # 为什么必须真的发滚轮，而不是改"滚动位置"
///
/// 场景树的滚动位置是 egui `ScrollArea` 的内部状态，外部既读不到也写不了。
/// 唯一能证明"滚动发生了"的办法是把 `WM_MOUSEWHEEL` 的等价输入交给系统输入栈，
/// 再对比滚动前后应用自己记录的行矩形 —— 行的 y 变了，就是真的滚了。
///
/// `格数` 为正表示向上滚（内容下移），为负表示向下滚。Windows 的一个
/// "刻度"是 `WHEEL_DELTA = 120`。
///
/// # 为什么分多次发
///
/// egui 对单个滚轮事件有**平滑滚动**处理：一次发 600 会被当成一次大步进，
/// 中间帧的滚动位置不连续，且不同版本的手感不同。分 5 次发 120 更接近
/// 真实滚轮，也让"滚动确实发生了"的中间态可见。
pub fn 滚动(坐标: (i32, i32), 格数: i32) -> bool {
    if !移动到(坐标) {
        return false;
    }
    sleep(Duration::from_millis(120));
    let 方向 = if 格数 >= 0 { 1 } else { -1 };
    let 次数 = 格数.abs().max(1);
    let mut 全部成功 = true;
    for _ in 0..次数 {
        if !发送滚轮(坐标, 方向 * WHEEL_DELTA) {
            全部成功 = false;
        }
        sleep(Duration::from_millis(60));
    }
    sleep(Duration::from_millis(160));
    全部成功
}

/// 滚轮的一"格"。
const WHEEL_DELTA: i32 = 120;

#[cfg(windows)]
fn 发送滚轮(坐标: (i32, i32), 增量: i32) -> bool {
    // 与 发送鼠标 相同的归一化：`MOUSEEVENTF_VIRTUALDESK` 让多显示器也成立。
    let (左, 上, 宽, 高) = 屏幕度量();
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
                // `mouseData` 是**无符号**字段，负数要按 16 位补码传。
                mouseData: 增量 as u32,
                dwFlags: MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | MOUSEEVENTF_WHEEL,
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
fn 发送滚轮(_坐标: (i32, i32), _增量: i32) -> bool {
    false
}

/// 取虚拟桌面度量（左、上、宽、高）。
#[cfg(windows)]
fn 屏幕度量() -> (i32, i32, i32, i32) {
    // SAFETY: 只读取系统度量；失败时退回 1920×1080 的主屏假设。
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
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
