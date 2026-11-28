use super::*;

pub(crate) unsafe fn screenshot() -> Option<String> {
    let prev = windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(-4isize as _);
    let (x, y, w, h) = (
        GetSystemMetrics(SM_XVIRTUALSCREEN),
        GetSystemMetrics(SM_YVIRTUALSCREEN),
        GetSystemMetrics(SM_CXVIRTUALSCREEN),
        GetSystemMetrics(SM_CYVIRTUALSCREEN),
    );
    if w <= 0 || h <= 0 {
        windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(prev);
        return None;
    }
    let scr = GetDC(null_mut());
    let mem = CreateCompatibleDC(scr);
    let bmp = CreateCompatibleBitmap(scr, w, h);
    let old = SelectObject(mem, bmp);
    BitBlt(mem, 0, 0, w, h, scr, x, y, SRCCOPY | 0x4000_0000);
    SelectObject(mem, old);
    DeleteDC(mem);
    ReleaseDC(null_mut(), scr);
    windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(prev);
    save_bitmap(bmp)
}

pub(crate) unsafe fn save_bitmap(bmp: HBITMAP) -> Option<String> {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    let mut clip = false;
    if OpenClipboard(null_mut()) != 0 {
        EmptyClipboard();
        clip = !SetClipboardData(2, bmp as _).is_null();
        CloseClipboard();
    }
    if !clip {
        DeleteObject(bmp);
    }
    clip.then(String::new)
}

pub(crate) unsafe fn screenshot_notice(bar: HWND) {
    let msg = match screenshot() {
        Some(_) => "Screenshot copied to clipboard",
        None => "Could not take a screenshot",
    };
    KillTimer(bar, 2);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    let cx = if GetMonitorInfoW(MonitorFromWindow(bar, MONITOR_DEFAULTTONEAREST), &mut mi) != 0 {
        (mi.rcMonitor.right - mi.rcMonitor.left) / 2
    } else {
        GetSystemMetrics(SM_CXSCREEN) / 2
    };
    show_tip(msg.to_string(), cx);
    SetTimer(bar, 6, 2500, None);
}
