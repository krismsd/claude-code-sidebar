//! The "peek" window: while the sidebar is collapsed to its strip, hovering a session shows its
//! expanded card in this small borderless window beside the strip (the strip itself is too
//! narrow to draw it). It never takes focus or the mouse: the pointer stays on the strip.
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition};

pub const LABEL: &str = "peek";
/// Width of the peek card (DIP) and the gap to the strip.
const WIDTH: f64 = 320.0;
const GAP: f64 = 6.0;

/// Make the window non-activating (clicks and showing never steal focus from the terminal).
#[cfg(windows)]
pub fn init(app: &AppHandle) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    if let Some(w) = app.get_webview_window(LABEL)
        && let Ok(h) = w.hwnd()
    {
        let h = HWND(h.0);
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(
                h,
                GWL_EXSTYLE,
                ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize,
            );
        }
    }
}

/// Show the peek for `session_id`, level with a strip item whose top is `top` (CSS px in the
/// main window). The peek then reports its height (`resize`) and is kept on screen.
pub fn show(app: &AppHandle, session_id: &str, top: f64, left_edge: bool) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("no main window")?;
    let peek = app.get_webview_window(LABEL).ok_or("no peek window")?;
    let scale = main.scale_factor().map_err(|e| e.to_string())?;
    let pos = main.outer_position().map_err(|e| e.to_string())?;
    let size = main.outer_size().map_err(|e| e.to_string())?;
    let width = (WIDTH * scale) as i32;
    let gap = (GAP * scale) as i32;
    let x = if left_edge {
        pos.x + size.width as i32 + gap
    } else {
        pos.x - width - gap
    };
    let y = pos.y + (top * scale) as i32;
    let _ = peek.set_position(PhysicalPosition::new(x, y));
    let _ = peek.emit_to(LABEL, "peek", session_id);
    show_no_activate(&peek);
    watch_cursor(app);
    Ok(())
}

static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The webview doesn't always report the mouse leaving (a quick flick off the strip), so
/// while the peek is up, check the cursor and hide it once it's no longer over the sidebar.
fn watch_cursor(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            if !peek_visible(&app) {
                break;
            }
            if !cursor_over_main(&app) {
                hide(&app);
                break;
            }
        }
        WATCHING.store(false, Ordering::SeqCst);
    });
}

/// The peek card measured itself: size the window to fit, shifted up if it would run off the
/// bottom of the monitor.
pub fn resize(app: &AppHandle, height: f64) -> Result<(), String> {
    let peek = app.get_webview_window(LABEL).ok_or("no peek window")?;
    let _ = peek.set_size(LogicalSize::new(WIDTH, height.clamp(40.0, 1200.0)));
    if let (Ok(pos), Ok(Some(mon)), Ok(scale)) = (
        peek.outer_position(),
        peek.current_monitor(),
        peek.scale_factor(),
    ) {
        let bottom = mon.position().y + mon.size().height as i32;
        let h = (height * scale) as i32;
        if pos.y + h > bottom - 8 {
            let _ = peek.set_position(PhysicalPosition::new(
                pos.x,
                (bottom - 8 - h).max(mon.position().y),
            ));
        }
    }
    Ok(())
}

/// Hide the peek. It's shown through Win32 (so it never activates), which Tauri doesn't know
/// about, so Tauri's own hide() would be a no-op: hide it the same way.
pub fn hide(app: &AppHandle) {
    let Some(p) = app.get_webview_window(LABEL) else {
        return;
    };
    #[cfg(windows)]
    if let Ok(h) = p.hwnd() {
        use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
        unsafe {
            let _ = ShowWindow(windows::Win32::Foundation::HWND(h.0), SW_HIDE);
        }
        return;
    }
    let _ = p.hide();
}

#[cfg(windows)]
fn peek_visible(app: &AppHandle) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    app.get_webview_window(LABEL)
        .and_then(|w| w.hwnd().ok())
        .is_some_and(|h| unsafe { IsWindowVisible(HWND(h.0)).as_bool() })
}

/// Is the cursor over the sidebar window? (Screen coordinates, straight from Windows.)
#[cfg(windows)]
fn cursor_over_main(app: &AppHandle) -> bool {
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect};
    let Some(h) = app.get_webview_window("main").and_then(|w| w.hwnd().ok()) else {
        return false;
    };
    let (mut pt, mut r) = (POINT::default(), RECT::default());
    unsafe {
        if GetCursorPos(&mut pt).is_err() || GetWindowRect(HWND(h.0), &mut r).is_err() {
            return true; // can't tell: leave it to the webview's own mouse-leave
        }
    }
    pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
}

#[cfg(not(windows))]
fn peek_visible(_: &AppHandle) -> bool {
    false
}

#[cfg(not(windows))]
fn cursor_over_main(_: &AppHandle) -> bool {
    true
}

#[cfg(windows)]
fn show_no_activate(w: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        ShowWindow,
    };
    if let Ok(h) = w.hwnd() {
        let h = HWND(h.0);
        unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                h,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(windows))]
fn show_no_activate(w: &tauri::WebviewWindow) {
    let _ = w.show();
}
