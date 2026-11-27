#![windows_subsystem = "windows"]
mod decode;
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering::Relaxed},
        mpsc::{Sender, channel},
    },
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext},
        Input::KeyboardAndMouse::*,
        Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, FOF_ALLOWUNDO, FO_DELETE, HDROP, SHFileOperationW, SHFILEOPSTRUCTW},
        WindowsAndMessaging::*,
    },
};

pub const EXTS: [&str; 11] = ["jpg", "jpeg", "jfif", "png", "bmp", "gif", "tif", "tiff", "webp", "heic", "avif"];
const WM_IMG: u32 = WM_APP + 1;
const BG: [u8; 3] = [24, 22, 22];

static GEN: AtomicU32 = AtomicU32::new(0);
static WORKER: OnceLock<Sender<(isize, u32, PathBuf)>> = OnceLock::new();

#[derive(Default)]
struct App {
    files: Vec<PathBuf>,
    idx: usize,
    img: Option<decode::Img>,
    note: String,
    zoom: Option<f64>,
    off: (f64, f64),
    drag: Option<(i32, i32)>,
    font: isize,
    dpi: i32,
}
thread_local! {
    static APP: RefCell<App> = RefCell::new(App::default());
    static PV_BUF: Cell<(isize, isize, isize, i32, i32)> = const { Cell::new((0, 0, 0, 0, 0)) };
}

fn sc(v: i32, dpi: i32) -> i32 {
    v * dpi.max(96) / 96
}

fn make_font(dpi: i32) -> isize {
    with_wide(winrust_common::style::sans_face(), |face| unsafe {
        CreateFontW(-sc(18, dpi), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face)
    }) as isize
}

fn active_work_area() -> RECT {
    unsafe {
        let mut cur = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cur);
        let mon = MonitorFromPoint(cur, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) != 0 {
            return mi.rcWork;
        }
        let mut wa: RECT = std::mem::zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as *mut _, 0) != 0 {
            return wa;
        }
        RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) }
    }
}

fn with_wide<T>(s: &str, f: impl FnOnce(*const u16) -> T) -> T {
    let mut stack = [0u16; 256];
    let mut len = 0usize;
    for u in s.encode_utf16() {
        if len + 1 >= stack.len() {
            let v: Vec<u16> = s.encode_utf16().chain([0]).collect();
            return f(v.as_ptr());
        }
        stack[len] = u;
        len += 1;
    }
    stack[len] = 0;
    f(stack.as_ptr())
}

fn pt(lp: LPARAM) -> (i32, i32) {
    ((lp & 0xFFFF) as i16 as i32, (lp >> 16) as i16 as i32)
}

fn is_pic(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn worker() -> &'static Sender<(isize, u32, PathBuf)> {
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<(isize, u32, PathBuf)>();
        std::thread::spawn(move || {
            decode::com_init();
            let mut factory = None;
            while let Ok((mut h, mut g, mut path)) = rx.recv() {
                while let Ok((nh, ng, np)) = rx.try_recv() {
                    (h, g, path) = (nh, ng, np);
                }
                if g != GEN.load(Relaxed) {
                    continue;
                }
                let f = match &factory {
                    Some(f) => f,
                    None => match decode::create_factory() {
                        Ok(f) => factory.insert(f),
                        Err(e) => {
                            let msg = Box::into_raw(Box::new((g, Err::<decode::Img, String>(e.message()))));
                            if unsafe { PostMessageW(h as HWND, WM_IMG, 0, msg as isize) } == 0 {
                                unsafe { drop(Box::from_raw(msg)) };
                            }
                            continue;
                        }
                    },
                };
                let r = decode::decode_with(f, &path.to_string_lossy(), BG).map_err(|e| e.message());
                let msg = Box::into_raw(Box::new((g, r)));
                if unsafe { PostMessageW(h as HWND, WM_IMG, 0, msg as isize) } == 0 {
                    unsafe { drop(Box::from_raw(msg)) };
                }
            }
        });
        tx
    })
}

fn open(hwnd: HWND, path: &Path) {
    let Some(dir) = path.parent() else { return };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| is_pic(p)).collect();
    files.sort_by_cached_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    let idx = files.iter().position(|p| p.as_os_str().eq_ignore_ascii_case(path.as_os_str())).unwrap_or_else(|| {
        files.insert(0, path.to_path_buf());
        0
    });
    APP.with_borrow_mut(|a| (a.files, a.idx) = (files, idx));
    load(hwnd);
}

fn load(hwnd: HWND) {
    let Some(path) = APP.with_borrow_mut(|a| {
        (a.zoom, a.off) = (None, (0.0, 0.0));
        a.note = "...".into();
        a.files.get(a.idx).cloned()
    }) else {
        return;
    };
    let g = GEN.fetch_add(1, Relaxed) + 1;
    let _ = worker().send((hwnd as isize, g, path));
    unsafe { InvalidateRect(hwnd, null(), 0) };
    title(hwnd);
}

fn go(hwnd: HWND, to: impl FnOnce(usize, usize) -> usize) {
    let moved = APP.with_borrow_mut(|a| {
        let n = to(a.idx, a.files.len());
        (!a.files.is_empty() && n != a.idx).then(|| a.idx = n).is_some()
    });
    if moved {
        load(hwnd);
    }
}

fn view(a: &App, cw: f64, ch: f64) -> Option<(f64, f64, f64)> {
    let i = a.img.as_ref()?;
    let z = a.zoom.unwrap_or_else(|| (cw / i.w as f64).min(ch / i.h as f64).min(1.0));
    Some((z, (cw - i.w as f64 * z) / 2.0 + a.off.0, (ch - i.h as f64 * z) / 2.0 + a.off.1))
}

fn title(hwnd: HWND) {
    let t = APP.with_borrow(|a| match (a.files.get(a.idx), &a.img) {
        (Some(p), img) => {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let size = img.as_ref().map_or(String::new(), |i| format!("  {}x{}", i.w, i.h));
            format!("{name}  ({}/{}){size}  - Photo viewer", a.idx + 1, a.files.len())
        }
        _ => "Photo viewer".into(),
    });
    with_wide(&t, |w| unsafe { SetWindowTextW(hwnd, w) });
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let (cw, ch) = (rc.right, rc.bottom);
        if cw <= 0 || ch <= 0 {
            EndPaint(hwnd, &ps);
            return;
        }
        let (mut mem_i, mut bmp_i, mut orig_bmp, bw, bh) = PV_BUF.get();
        if mem_i == 0 {
            mem_i = CreateCompatibleDC(dc) as isize;
            SetDCBrushColor(mem_i as HDC, BG[2] as u32 | (BG[1] as u32) << 8 | (BG[0] as u32) << 16);
        }
        let mem = mem_i as HDC;
        if bmp_i == 0 || bw != cw || bh != ch {
            let nb = CreateCompatibleBitmap(dc, cw, ch);
            let prev = SelectObject(mem, nb);
            if bmp_i == 0 {
                orig_bmp = prev as isize;
            } else {
                DeleteObject(prev);
            }
            bmp_i = nb as isize;
            PV_BUF.set((mem_i, bmp_i, orig_bmp, cw, ch));
        }
        FillRect(mem, &rc, GetStockObject(DC_BRUSH as i32) as HBRUSH);
        APP.with_borrow(|a| {
            if let (Some(i), Some((z, x, y))) = (&a.img, view(a, cw as f64, ch as f64)) {
                let mut bi: BITMAPINFO = std::mem::zeroed();
                bi.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: i.w, biHeight: -i.h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..std::mem::zeroed() };
                SetStretchBltMode(mem, if z < 1.0 { HALFTONE } else { COLORONCOLOR } as _);
                SetBrushOrgEx(mem, 0, 0, null_mut());
                StretchDIBits(mem, x as i32, y as i32, (i.w as f64 * z).round() as i32, (i.h as f64 * z).round() as i32, 0, 0, i.w, i.h, i.bgra.as_ptr() as _, &bi, DIB_RGB_COLORS, SRCCOPY);
            } else {
                let prev_font = SelectObject(mem, a.font as _);
                SetBkMode(mem, TRANSPARENT as _);
                SetTextColor(mem, 0x93_8e_8e);
                let pad = sc(16, a.dpi);
                let mut r = RECT { left: rc.left + pad, top: rc.top, right: (rc.right - pad).max(rc.left + pad), bottom: rc.bottom };
                let text = if a.files.is_empty() { "Open a picture with this program, or drop one here" } else { &a.note };
                with_wide(text, |t| {
                    DrawTextW(mem, t, -1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
                });
                SelectObject(mem, prev_font);
            }
        });
        BitBlt(dc, 0, 0, cw, ch, mem, 0, 0, SRCCOPY);
        EndPaint(hwnd, &ps);
    }
}

fn delete_current(hwnd: HWND) {
    let Some(p) = APP.with_borrow(|a| a.files.get(a.idx).cloned()) else { return };
    let mut from: Vec<u16> = p.to_string_lossy().encode_utf16().chain([0, 0]).collect();
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    (op.hwnd, op.wFunc, op.pFrom, op.fFlags) = (hwnd, FO_DELETE, from.as_mut_ptr(), FOF_ALLOWUNDO as u16);
    if unsafe { SHFileOperationW(&mut op) } == 0 && op.fAnyOperationsAborted == 0 {
        APP.with_borrow_mut(|a| {
            a.files.remove(a.idx);
            a.idx = a.idx.min(a.files.len().saturating_sub(1));
            a.img = None;
        });
        load(hwnd);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let d = GetDpiForWindow(hwnd) as i32;
                let dpi = if d > 0 { d } else { GetDpiForSystem() as i32 };
                let f = make_font(dpi);
                APP.with_borrow_mut(|a| {
                    a.dpi = dpi;
                    a.font = f;
                });
                DragAcceptFiles(hwnd, 1);
                0
            }
            WM_DPICHANGED => {
                let dpi = (wp & 0xFFFF) as i32;
                let f = make_font(dpi);
                APP.with_borrow_mut(|a| {
                    if a.font != 0 {
                        DeleteObject(a.font as _);
                    }
                    a.dpi = dpi;
                    a.font = f;
                });
                let r = &*(lp as *const RECT);
                SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            WM_SIZE => {
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_DROPFILES => {
                let d = wp as HDROP;
                let mut buf = [0u16; 1024];
                let n = DragQueryFileW(d, 0, buf.as_mut_ptr(), buf.len() as u32) as usize;
                DragFinish(d);
                open(hwnd, Path::new(&OsString::from_wide(&buf[..n])));
                0
            }
            WM_IMG => {
                let (g, r) = *Box::from_raw(lp as *mut (u32, Result<decode::Img, String>));
                if g == GEN.load(Relaxed) {
                    APP.with_borrow_mut(|a| match r {
                        Ok(i) => (a.img, a.note) = (Some(i), String::new()),
                        Err(e) => (a.img, a.note) = (None, format!("Cannot open this picture: {e}")),
                    });
                    InvalidateRect(hwnd, null(), 0);
                    title(hwnd);
                }
                0
            }
            WM_MOUSEWHEEL => {
                let (mx, my) = pt(lp);
                let mut p = POINT { x: mx, y: my };
                ScreenToClient(hwnd, &mut p);
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let (cw, ch) = (rc.right as f64, rc.bottom as f64);
                let k = if (wp >> 16) as i16 > 0 { 1.15 } else { 1.0 / 1.15 };
                APP.with_borrow_mut(|a| {
                    if let (Some(i), Some((z, x, y))) = (&a.img, view(a, cw, ch)) {
                        let z2 = (z * k).clamp(0.02, 32.0);
                        let (u, v) = ((p.x as f64 - x) / z, (p.y as f64 - y) / z);
                        a.off = (p.x as f64 - u * z2 - (cw - i.w as f64 * z2) / 2.0, p.y as f64 - v * z2 - (ch - i.h as f64 * z2) / 2.0);
                        a.zoom = Some(z2);
                    }
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                SetCapture(hwnd);
                APP.with_borrow_mut(|a| a.drag = Some(pt(lp)));
                0
            }
            WM_MOUSEMOVE => {
                let (x, y) = pt(lp);
                let moved = APP.with_borrow_mut(|a| {
                    let Some((px, py)) = a.drag else { return false };
                    a.off = (a.off.0 + (x - px) as f64, a.off.1 + (y - py) as f64);
                    a.drag = Some((x, y));
                    true
                });
                if moved {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                APP.with_borrow_mut(|a| a.drag = None);
                0
            }
            WM_LBUTTONDBLCLK => {
                APP.with_borrow_mut(|a| (a.zoom, a.off) = (a.zoom.is_none().then_some(1.0), (0.0, 0.0)));
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_KEYDOWN => {
                match wp as u16 {
                    VK_ESCAPE => {
                        DestroyWindow(hwnd);
                    }
                    VK_RIGHT | VK_SPACE | VK_NEXT => go(hwnd, |i, n| (i + 1).min(n.saturating_sub(1))),
                    VK_LEFT | VK_BACK | VK_PRIOR => go(hwnd, |i, _| i.saturating_sub(1)),
                    VK_HOME => go(hwnd, |_, _| 0),
                    VK_END => go(hwnd, |_, n| n.saturating_sub(1)),
                    VK_DELETE => delete_current(hwnd),
                    k if k == b'1' as u16 || k == b'F' as u16 || k == b'0' as u16 => {
                        APP.with_borrow_mut(|a| (a.zoom, a.off) = (if k == b'1' as u16 { Some(1.0) } else { None }, (0.0, 0.0)));
                        InvalidateRect(hwnd, null(), 0);
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                let (mem_i, bmp_i, orig_bmp, ..) = PV_BUF.replace((0, 0, 0, 0, 0));
                if mem_i != 0 {
                    if orig_bmp != 0 {
                        SelectObject(mem_i as _, orig_bmp as _);
                    }
                    if bmp_i != 0 {
                        DeleteObject(bmp_i as _);
                    }
                    DeleteDC(mem_i as _);
                }
                APP.with_borrow_mut(|a| {
                    if a.font != 0 {
                        DeleteObject(a.font as _);
                        a.font = 0;
                    }
                });
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn main() {
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
            SetProcessDPIAware();
        }
        let inst = GetModuleHandleW(null());
        let hwnd = with_wide("WinRustPhoto", |cls| {
            let wc = WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: inst,
                hIcon: null_mut(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: cls,
            };
            RegisterClassW(&wc);
            let wa = active_work_area();
            let (wa_w, wa_h) = ((wa.right - wa.left).max(320), (wa.bottom - wa.top).max(240));
            let win_w = (wa_w * 72 / 100).clamp(wa_w.min(320), wa_w);
            let win_h = (wa_h * 75 / 100).clamp(wa_h.min(240), wa_h);
            let win_x = wa.left + (wa_w - win_w) / 2;
            let win_y = wa.top + (wa_h - win_h) / 2;
            with_wide("Photo viewer", |title| {
                CreateWindowExW(0, cls, title, WS_OVERLAPPEDWINDOW | WS_VISIBLE, win_x, win_y, win_w, win_h, null_mut(), null_mut(), inst, null())
            })
        });
        if let Some(p) = std::env::args_os().nth(1) {
            open(hwnd, Path::new(&p));
        }
        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}
