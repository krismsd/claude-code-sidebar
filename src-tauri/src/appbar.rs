//! Window placement (SPEC §3.9, M9): docked as a Windows AppBar on the left or right edge of a
//! chosen monitor (the shell reserves the space, like the taskbar), or floating as a normal
//! window that reserves nothing. Collapsing just narrows it. Re-docks on shell notifications,
//! display changes and DPI changes.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    ABE_LEFT, ABE_RIGHT, ABM_ACTIVATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
    ABM_WINDOWPOSCHANGED, ABN_FULLSCREENAPP, ABN_POSCHANGED, APPBARDATA, DefSubclassProc,
    SHAppBarMessage, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, HWND_NOTOPMOST, HWND_TOPMOST, MONITORINFOF_PRIMARY, PostMessageW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, WM_ACTIVATE, WM_APP, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_EXITSIZEMOVE, WM_SETTINGCHANGE, WM_WINDOWPOSCHANGED,
};
use windows::core::BOOL;

const CALLBACK_MSG: u32 = WM_APP + 0x100;
const REDOCK_MSG: u32 = WM_APP + 0x101;
const SUBCLASS_ID: usize = 0xCC51;

/// Where and how the sidebar sits. Sizes are device-independent pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub left_edge: bool,
    pub width: i32,
    /// 0 = primary monitor, n = the nth monitor counting from the left.
    pub monitor: usize,
    pub floating: bool,
    /// Collapsed to the slim strip: `width` is the strip width, in either mode.
    pub collapsed: bool,
    /// Floating: last position and size (physical pixels), if known.
    pub float_rect: Option<(i32, i32, i32, i32)>,
}

static PLACEMENT: Mutex<Option<Placement>> = Mutex::new(None);
/// Registered with the shell as an AppBar right now.
static REGISTERED: AtomicBool = AtomicBool::new(false);
/// A full-screen app is covering the sidebar (ABN_FULLSCREENAPP).
static COVERED: AtomicBool = AtomicBool::new(false);
/// Floating window rect after the user moved/resized it (physical pixels).
static MOVED_TO: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

pub fn covered() -> bool {
    COVERED.load(Ordering::Relaxed)
}

/// The floating window's position after a move/resize, to save (taken once).
pub fn take_moved() -> Option<(i32, i32, i32, i32)> {
    MOVED_TO.lock().unwrap().take()
}

fn placement() -> Placement {
    PLACEMENT.lock().unwrap().clone().unwrap_or(Placement {
        left_edge: false,
        width: 320,
        monitor: 0,
        floating: false,
        collapsed: false,
        float_rect: None,
    })
}

fn data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: CALLBACK_MSG,
        uEdge: if placement().left_edge {
            ABE_LEFT
        } else {
            ABE_RIGHT
        },
        ..Default::default()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Monitor {
    /// 1-based, counting from the left (what `monitor = n` in config means).
    pub number: usize,
    pub name: String,
    pub primary: bool,
    /// Physical pixels.
    pub width: i32,
    pub height: i32,
    pub scale_percent: u32,
    #[serde(skip)]
    rect: (i32, i32, i32, i32),
}

/// All monitors, left to right.
pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(out.0 as *mut Vec<Monitor>);
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(h, &mut mi as *mut _ as *mut MONITORINFO).as_bool() {
                let r = mi.monitorInfo.rcMonitor;
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let name = String::from_utf16_lossy(&mi.szDevice)
                    .trim_end_matches('\0')
                    .to_string();
                list.push(Monitor {
                    number: 0,
                    name: name.trim_start_matches(r"\\.\").to_string(),
                    primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                    width: r.right - r.left,
                    height: r.bottom - r.top,
                    scale_percent: dx * 100 / 96,
                    rect: (r.left, r.top, r.right, r.bottom),
                });
            }
            BOOL(1)
        }
    }
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut _ as isize));
    }
    list.sort_by_key(|m| (m.rect.0, m.rect.1));
    for (i, m) in list.iter_mut().enumerate() {
        m.number = i + 1;
    }
    list
}

/// The configured monitor, falling back to the primary one (e.g. after unplugging).
fn target_monitor(n: usize) -> Option<Monitor> {
    let list = monitors();
    let chosen = if n > 0 {
        list.iter().find(|m| m.number == n)
    } else {
        None
    };
    chosen
        .or_else(|| list.iter().find(|m| m.primary))
        .or(list.first())
        .cloned()
}

/// Take over the window: subclass it and apply `p`. Call once, after the window exists.
pub fn attach(hwnd: HWND, p: Placement) {
    *PLACEMENT.lock().unwrap() = Some(p);
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0);
    }
    apply(hwnd);
}

/// Change placement at runtime (config reload, collapse, mode switch). Safe from any thread:
/// the work happens on the window's thread.
pub fn configure(hwnd: HWND, p: Placement) {
    *PLACEMENT.lock().unwrap() = Some(p);
    unsafe {
        let _ = PostMessageW(Some(hwnd), REDOCK_MSG, WPARAM(0), LPARAM(0));
    }
}

fn apply(hwnd: HWND) {
    let p = placement();
    let Some(mon) = target_monitor(p.monitor) else {
        return;
    };
    let width = p.width * mon.scale_percent as i32 / 100;
    unsafe {
        if p.floating {
            if REGISTERED.swap(false, Ordering::Relaxed) {
                let mut abd = data(hwnd);
                SHAppBarMessage(ABM_REMOVE, &mut abd);
            }
            // Saved position if it's still on a screen; otherwise near the chosen monitor's edge.
            let (l, t, r, b) = mon.rect;
            let (x, y, w, h) = match p.float_rect {
                Some((x, y, w, h)) if monitors().iter().any(|m| x >= m.rect.0 && x < m.rect.2) => {
                    (x, y, w, h)
                }
                _ => (
                    if p.left_edge { l + 40 } else { r - width - 40 },
                    t + 40,
                    width,
                    (b - t) * 3 / 4,
                ),
            };
            // Collapsed: the strip width; otherwise whatever width the user last resized it to.
            let w = if p.collapsed { width } else { w };
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        } else {
            if !REGISTERED.swap(true, Ordering::Relaxed) {
                let mut abd = data(hwnd);
                SHAppBarMessage(ABM_NEW, &mut abd);
            }
            dock(hwnd, &p, &mon, width);
        }
    }
}

fn dock(hwnd: HWND, p: &Placement, mon: &Monitor, width: i32) {
    let fit = |rc: &mut RECT| {
        if p.left_edge {
            rc.right = rc.left + width;
        } else {
            rc.left = rc.right - width;
        }
    };
    unsafe {
        let mut abd = data(hwnd);
        let (l, t, r, b) = mon.rect;
        abd.rc = RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        };
        fit(&mut abd.rc);
        SHAppBarMessage(ABM_QUERYPOS, &mut abd);
        // The shell may have moved our outer edge in (e.g. another AppBar); keep our width.
        fit(&mut abd.rc);
        SHAppBarMessage(ABM_SETPOS, &mut abd);
        let RECT {
            left,
            top,
            right,
            bottom,
        } = abd.rc;
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            left,
            top,
            right - left,
            bottom - top,
            SWP_NOACTIVATE,
        );
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _ref: usize,
) -> LRESULT {
    unsafe {
        let docked = REGISTERED.load(Ordering::Relaxed);
        match msg {
            CALLBACK_MSG if docked => match wparam.0 as u32 {
                ABN_POSCHANGED => apply(hwnd),
                ABN_FULLSCREENAPP => {
                    // Step out of the way of full-screen apps (video, presentations).
                    COVERED.store(lparam.0 != 0, Ordering::Relaxed);
                    let z = if lparam.0 != 0 {
                        HWND_NOTOPMOST
                    } else {
                        HWND_TOPMOST
                    };
                    let _ = SetWindowPos(
                        hwnd,
                        Some(z),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                _ => {}
            },
            // Let the default handling (Tauri resizing for the new DPI etc.) finish, then re-dock.
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE if docked => {
                let _ = PostMessageW(Some(hwnd), REDOCK_MSG, WPARAM(0), LPARAM(0));
            }
            REDOCK_MSG => apply(hwnd),
            // Floating: remember where the user put it.
            WM_EXITSIZEMOVE if !docked => {
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_ok() {
                    *MOVED_TO.lock().unwrap() =
                        Some((r.left, r.top, r.right - r.left, r.bottom - r.top));
                }
            }
            WM_ACTIVATE if docked => {
                let mut abd = data(hwnd);
                SHAppBarMessage(ABM_ACTIVATE, &mut abd);
            }
            WM_WINDOWPOSCHANGED if docked => {
                let mut abd = data(hwnd);
                SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut abd);
            }
            WM_DESTROY if docked => {
                let mut abd = data(hwnd);
                SHAppBarMessage(ABM_REMOVE, &mut abd);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}
