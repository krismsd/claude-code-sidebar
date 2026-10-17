//! Keep desktop icons where the user put them. Whenever the sidebar changes the reserved screen
//! space (AppBar on/off, width, edge, monitor), Explorer lays the whole desktop out again and,
//! with "Align to grid", icons end up scattered. We snapshot every icon's position just before
//! such a change and put them back afterwards, through the desktop's shell view
//! (IShellWindows → IShellBrowser → IFolderView), the same way desktop-layout tools do.
//!
//! COM objects are apartment-bound, so a snapshot is taken and restored on one worker thread.
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, IServiceProvider,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IEnumIDList, IFolderView, IShellBrowser, IShellWindows, SID_STopLevelBrowser, SVGIO_ALLVIEW,
    SVSI_POSITIONITEM, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};
use windows::core::Interface;

/// How long Explorer takes to re-lay out the desktop after a work-area change: restore after
/// the first delay, and once more after the second in case it reflowed late.
const SETTLE: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1200)];

/// Icon positions (desktop list-view coordinates) with the item IDs they belong to.
struct Snapshot {
    items: Vec<(*mut ITEMIDLIST, POINT)>,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        for (pidl, _) in &self.items {
            unsafe { CoTaskMemFree(Some(*pidl as _)) };
        }
    }
}

fn desktop_view() -> windows::core::Result<IFolderView> {
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
        let location = VARIANT::from(0i32); // CSIDL_DESKTOP
        let mut hwnd = 0i32;
        let dispatch = windows.FindWindowSW(
            &location,
            &VARIANT::default(),
            SWC_DESKTOP,
            &mut hwnd,
            SWFO_NEEDDISPATCH,
        )?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        browser.QueryActiveShellView()?.cast()
    }
}

fn snapshot(view: &IFolderView) -> windows::core::Result<Snapshot> {
    let mut snap = Snapshot { items: Vec::new() };
    unsafe {
        let items: IEnumIDList = view.Items(SVGIO_ALLVIEW)?;
        loop {
            let mut pidl = [std::ptr::null_mut::<ITEMIDLIST>()];
            let mut fetched = 0u32;
            if items.Next(&mut pidl, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            match view.GetItemPosition(pidl[0]) {
                Ok(pt) => snap.items.push((pidl[0], pt)),
                Err(_) => CoTaskMemFree(Some(pidl[0] as _)),
            }
        }
    }
    Ok(snap)
}

/// Put icons back, except ones whose old spot is now under the sidebar (`avoid`, screen
/// coordinates), which keep the place Explorer gave them.
fn restore(view: &IFolderView, snap: &Snapshot, avoid: Option<RECT>) -> (usize, usize) {
    // Desktop list-view coordinates start at the virtual screen's top-left corner.
    let (ox, oy) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        )
    };
    let under = |p: &POINT| {
        avoid.is_some_and(|r| {
            let (x, y) = (p.x + ox, p.y + oy);
            x >= r.left - 80 && x < r.right && y >= r.top && y < r.bottom
        })
    };
    let (mut moved, mut kept_off) = (0, 0);
    for (pidl, pt) in snap.items.iter() {
        if under(pt) {
            kept_off += 1;
            continue;
        }
        let before = unsafe { view.GetItemPosition(*pidl) }.ok();
        // Explorer snaps icons onto its grid (a few pixels) after a move: close enough.
        if before.is_some_and(|c| (c.x - pt.x).abs() <= 12 && (c.y - pt.y).abs() <= 12) {
            continue;
        }
        moved += 1;
        let ids = [*pidl as *const ITEMIDLIST];
        unsafe {
            let _ = view.SelectAndPositionItems(
                1,
                ids.as_ptr(),
                Some(pt as *const POINT),
                SVSI_POSITIONITEM.0 as u32,
            );
        }
    }
    (moved, kept_off)
}

/// Run `change` (which alters the reserved screen space) with desktop icons kept in place.
/// `change` runs on the calling thread once the snapshot is taken; the restore happens in the
/// background. `avoid` gives the sidebar's rect after the change, if it reserves space.
pub fn keep_icons<F, A>(change: F, avoid: A)
where
    F: FnOnce(),
    A: Fn() -> Option<RECT> + Send + 'static,
{
    let done = run(change, avoid);
    drop(done);
}

/// Like `keep_icons`, but waits for the restore to finish (used when quitting, so the icons
/// are back before the process exits).
pub fn keep_icons_blocking<F, A>(change: F, avoid: A)
where
    F: FnOnce(),
    A: Fn() -> Option<RECT> + Send + 'static,
{
    let done = run(change, avoid);
    let _ = done.recv_timeout(Duration::from_secs(6));
}

fn run<F, A>(change: F, avoid: A) -> mpsc::Receiver<()>
where
    F: FnOnce(),
    A: Fn() -> Option<RECT> + Send + 'static,
{
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let snap = desktop_view().and_then(|v| snapshot(&v).map(|s| (v, s)));
        if let Err(e) = &snap {
            log::warn!("desktop icons: couldn't read positions: {e}");
        }
        let _ = ready_tx.send(());
        if go_rx.recv().is_ok()
            && let Ok((view, snap)) = &snap
        {
            for (i, delay) in SETTLE.iter().enumerate() {
                std::thread::sleep(*delay);
                let (moved, kept_off) = restore(view, snap, avoid());
                log::info!(
                    "desktop icons: pass {}: {} of {} put back, {} left (would be under the sidebar)",
                    i + 1,
                    moved,
                    snap.items.len(),
                    kept_off
                );
            }
        }
        drop(snap);
        if com {
            CoUninitialize();
        }
        let _ = done_tx.send(());
    });
    // Never let the snapshot hold up the sidebar for long.
    let _ = ready_rx.recv_timeout(Duration::from_secs(2));
    change();
    let _ = go_tx.send(());
    done_rx
}
