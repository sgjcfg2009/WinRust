use std::{
    cell::{Cell, RefCell},
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, Ordering::Relaxed},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

static BUSY: AtomicBool = AtomicBool::new(false);

struct St {
    orig: HBITMAP,
    dim: HBITMAP,
    w: i32,
    h: i32,
    from: Option<(i32, i32)>,
    to: (i32, i32),
}

thread_local! {
    static ST: RefCell<Option<St>> = const { RefCell::new(None) };
    static MEM_DC: Cell<HDC> = const { Cell::new(null_mut()) };
}

fn rect(a: (i32, i32), b: (i32, i32)) -> RECT {
    RECT { left: a.0.min(b.0), top: a.1.min(b.1), right: a.0.max(b.0), bottom: a.1.max(b.1) }
}

fn sel() -> Option<RECT> {
    ST.with_borrow(|s| s.as_ref().and_then(|s| s.from.map(|f| rect(f, s.to))))
}

pub fn start() {
    if BUSY.swap(true, Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        unsafe { run() };
        BUSY.store(false, Relaxed);
    });
}

unsafe fn run() {
    windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(-4isize as _);
    let (vx, vy, w, h) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
    if w <= 0 || h <= 0 {
        return;
    }
    let scr = GetDC(null_mut());
    let (mem, dd, one) = (CreateCompatibleDC(scr), CreateCompatibleDC(scr), CreateCompatibleDC(scr));
    let (orig, dim, black) = (CreateCompatibleBitmap(scr, w, h), CreateCompatibleBitmap(scr, w, h), CreateCompatibleBitmap(scr, 1, 1));
    let (o1, o2, o3) = (SelectObject(mem, orig), SelectObject(dd, dim), SelectObject(one, black));
    BitBlt(mem, 0, 0, w, h, scr, vx, vy, SRCCOPY | 0x4000_0000);
    BitBlt(dd, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    PatBlt(one, 0, 0, 1, 1, BLACKNESS);
    AlphaBlend(dd, 0, 0, w, h, one, 0, 0, 1, 1, BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 110, AlphaFormat: 0 });
    SelectObject(mem, o1);
    SelectObject(dd, o2);
    SelectObject(one, o3);
    for d in [mem, dd, one] {
        DeleteDC(d);
    }
    DeleteObject(black);
    ReleaseDC(null_mut(), scr);
    ST.set(Some(St { orig, dim, w, h, from: None, to: (0, 0) }));

    static REG: std::sync::Once = std::sync::Once::new();
    static CLS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let cls = CLS.get_or_init(|| super::wide("WinRustClip"));
    let hi = GetModuleHandleW(null());
    REG.call_once(|| {
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(proc);
        wc.hInstance = hi;
        wc.hCursor = LoadCursorW(null_mut(), IDC_CROSS);
        wc.lpszClassName = cls.as_ptr();
        RegisterClassW(&wc);
    });
    let hw = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, cls.as_ptr(), null(), WS_POPUP | WS_VISIBLE, vx, vy, w, h, null_mut(), null_mut(), hi, null());
    if !hw.is_null() {
        super::force_fg(hw);
        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    let dc = MEM_DC.replace(null_mut());
    if !dc.is_null() {
        DeleteDC(dc);
    }
    DeleteObject(orig);
    DeleteObject(dim);
    ST.set(None);
}

unsafe fn paint(hw: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hw, &mut ps);
    let p = ps.rcPaint;
    let mut mem = MEM_DC.get();
    if mem.is_null() {
        mem = CreateCompatibleDC(hdc);
        MEM_DC.set(mem);
    }
    let (dim, orig) = ST.with_borrow(|s| s.as_ref().map(|s| (s.dim, s.orig))).unwrap_or((null_mut(), null_mut()));
    let old = SelectObject(mem, dim);
    BitBlt(hdc, p.left, p.top, p.right - p.left, p.bottom - p.top, mem, p.left, p.top, SRCCOPY);
    if let Some(r) = sel() {
        let (l, t, rr, b) = (r.left.max(p.left), r.top.max(p.top), r.right.min(p.right), r.bottom.min(p.bottom));
        SelectObject(mem, orig);
        if rr > l && b > t {
            BitBlt(hdc, l, t, rr - l, b - t, mem, l, t, SRCCOPY);
        }
        FrameRect(hdc, &RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 }, GetStockObject(WHITE_BRUSH) as HBRUSH);
    }
    SelectObject(mem, old);
    EndPaint(hw, &ps);
}

unsafe fn dirty(hw: HWND, before: Option<RECT>, now: Option<RECT>) {
    let r = match (before, now) {
        (Some(a), Some(b)) => RECT { left: a.left.min(b.left), top: a.top.min(b.top), right: a.right.max(b.right), bottom: a.bottom.max(b.bottom) },
        (Some(a), None) | (None, Some(a)) => a,
        _ => return,
    };
    InvalidateRect(hw, &RECT { left: r.left - 3, top: r.top - 3, right: r.right + 3, bottom: r.bottom + 3 }, 0);
}

unsafe fn finish(hw: HWND) {
    let (r, orig) = match (sel(), ST.with_borrow(|s| s.as_ref().map(|s| s.orig))) {
        (Some(r), Some(o)) => (r, o),
        _ => {
            DestroyWindow(hw);
            return;
        }
    };
    let (rw, rh) = (r.right - r.left, r.bottom - r.top);
    if rw < 3 || rh < 3 {
        DestroyWindow(hw);
        return;
    }
    let scr = GetDC(null_mut());
    let (src, dst) = (CreateCompatibleDC(scr), CreateCompatibleDC(scr));
    let piece = CreateCompatibleBitmap(scr, rw, rh);
    let (o1, o2) = (SelectObject(src, orig), SelectObject(dst, piece));
    BitBlt(dst, 0, 0, rw, rh, src, r.left, r.top, SRCCOPY);
    SelectObject(src, o1);
    SelectObject(dst, o2);
    DeleteDC(src);
    DeleteDC(dst);
    ReleaseDC(null_mut(), scr);
    DestroyWindow(hw);
    let _ = super::save_bitmap(piece);
}

unsafe extern "system" fn proc(hw: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let pt = ((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32);
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hw);
            0
        }
        WM_LBUTTONDOWN => {
            ST.with_borrow_mut(|s| {
                if let Some(s) = s {
                    s.from = Some(pt);
                    s.to = pt;
                }
            });
            SetCapture(hw);
            0
        }
        WM_MOUSEMOVE => {
            if wp & 1 != 0 {
                let mut before = None;
                let mut after = None;
                ST.with_borrow_mut(|s| {
                    if let Some(s) = s {
                        if let Some(f) = s.from {
                            before = Some(rect(f, s.to));
                            s.to = (pt.0.clamp(0, s.w), pt.1.clamp(0, s.h));
                            after = Some(rect(f, s.to));
                        }
                    }
                });
                if let (Some(a), Some(b)) = (before, after) {
                    if (a.left, a.top, a.right, a.bottom) != (b.left, b.top, b.right, b.bottom) {
                        dirty(hw, before, after);
                    }
                }
            }
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            finish(hw);
            0
        }
        WM_RBUTTONDOWN => {
            DestroyWindow(hw);
            0
        }
        WM_KEYDOWN if wp == VK_ESCAPE as usize => {
            DestroyWindow(hw);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hw, msg, wp, lp),
    }
}
