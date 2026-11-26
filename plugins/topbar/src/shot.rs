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
    use windows_sys::Win32::{
        Graphics::GdiPlus::{GdipCreateBitmapFromHBITMAP, GdipDisposeImage, GdipSaveImageToFile, GpBitmap, GpImage},
        System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
    };
    let mut t: SYSTEMTIME = zeroed();
    GetLocalTime(&mut t);
    let name = format!("Screenshot {:04}-{:02}-{:02} {:02}-{:02}-{:02}.png", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let d = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(r"Pictures\Screenshots");
        let _ = std::fs::create_dir_all(&d);
        d
    });
    let mut gb: *mut GpBitmap = null_mut();
    let mut saved = false;
    if GdipCreateBitmapFromHBITMAP(bmp, null_mut(), &mut gb) == 0 && !gb.is_null() {
        let png = windows_sys::core::GUID { data1: 0x557c_f406, data2: 0x1a04, data3: 0x11d3, data4: [0x9a, 0x73, 0x00, 0x00, 0xf8, 0x1e, 0xf3, 0x2e] };
        saved = GdipSaveImageToFile(gb as *mut GpImage, wide(&dir.join(&name).to_string_lossy()).as_ptr(), &png, null()) == 0;
        GdipDisposeImage(gb as *mut GpImage);
    }
    let mut clip = false;
    if OpenClipboard(null_mut()) != 0 {
        EmptyClipboard();
        clip = !SetClipboardData(2, bmp as _).is_null();
        CloseClipboard();
    }
    if !clip {
        DeleteObject(bmp);
    }
    (saved || clip).then_some(name)
}

pub(crate) unsafe fn screenshot_notice(bar: HWND) {
    let msg = match screenshot() {
        Some(n) => format!("Screenshot saved: {n}\nand copied to the clipboard"),
        None => "Could not take a screenshot".to_string(),
    };
    KillTimer(bar, 2);
    show_tip(msg, GetSystemMetrics(SM_CXSCREEN) / 2);
    SetTimer(bar, 6, 2500, None);
}
