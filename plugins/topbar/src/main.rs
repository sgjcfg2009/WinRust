#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]
use std::{
    cell::{Cell, RefCell},
    mem::{size_of, zeroed},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    ptr::{null, null_mut},
    thread,
    sync::Mutex,
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicIsize, Ordering::Relaxed},
};
extern crate winrust_common;
use winrust_common as i18n;
use winrust_common::style::{Style, card, on_accent, pal, rgb, style, ui_face};
mod clip;
mod fly;
mod quick;
mod tray;
mod status;
mod run;
mod shot;
mod lang;
mod settings;
mod thumbs;
use {tray::*, status::*, run::*, shot::*, lang::*, settings::*};

pub(crate) unsafe fn own_foreground() -> bool {
    let fg = GetForegroundWindow();
    if fg.is_null() {
        return false;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(fg, &mut pid);
    pid == std::process::id()
}
use windows::{
    Win32::{
        Media::Audio::{Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender},
        System::Com::{CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile, STGM},
        UI::Shell::{IShellLinkW, ShellLink},
    },
    core::{Interface, PCWSTR},
};
use windows_sys::Win32::{
    Foundation::*,
    Globalization::{GetDateFormatW, GetTimeFormatW},
    Graphics::{
        Dwm::DwmGetWindowAttribute,
        Gdi::*,
        GdiPlus::{
            FillModeAlternate, GdipAddPathArc, GdipClosePathFigure,
            GdipCreateBitmapFromHICON, GdipCreateBitmapFromScan0, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush,
            GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDisposeImage, GdipDrawImageRectI, GdipDrawPath, GdipDrawRectangle, GdipFillPath, GdipFillRectangle,
            GdipGetImageGraphicsContext, GdipResetClip, GdipSetInterpolationMode, GdipSetPixelOffsetMode, GdipSetSmoothingMode,
            GdiplusStartup, GdiplusStartupInput, GpBitmap, GpBrush, GpGraphics, GpImage, GpPath, GpPen, GpSolidFill, InterpolationModeHighQualityBicubic, PixelOffsetModeHalf,
            SmoothingModeAntiAlias, UnitPixel,
        },
    },
    NetworkManagement::WiFi::*,
    System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
        Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
        SystemInformation::{GetLocalTime, GetTickCount64},
        Threading::{OpenProcess, QueryFullProcessImageNameW, TerminateProcess},
    },
    UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

fn rr(v: i32) -> i32 {
    if style().square() { 0 } else { sc(v) }
}
fn at_top() -> bool {
    style() == Style::Tiling && !vert()
}
fn vert() -> bool {
    i18n::style::side() != i18n::style::Side::Bottom
}
fn dock() -> bool {
    style() == Style::Dock && !vert()
}
unsafe fn bar_x() -> i32 {
    if i18n::style::side() == i18n::style::Side::Left { -bar_off() } else { GetSystemMetrics(SM_CXSCREEN) - bar_h() + bar_off() }
}
unsafe fn bar_pos() -> (i32, i32) {
    if vert() { (bar_x(), 0) } else { (bar_x_w().0, bar_y()) }
}
unsafe fn side_x(w: i32) -> i32 {
    if i18n::style::side() == i18n::style::Side::Left { bar_h() + sc(8) } else { GetSystemMetrics(SM_CXSCREEN) - bar_h() - sc(8) - w }
}
unsafe fn axis_len() -> i32 {
    GetSystemMetrics(if vert() { SM_CYSCREEN } else { SM_CXSCREEN })
}
fn dock_gap() -> i32 {
    if dock() { sc(8) } else { 0 }
}
unsafe fn bar_x_w() -> (i32, i32) {
    let cx = GetSystemMetrics(SM_CXSCREEN);
    if dock() {
        let w = (cx * 3 / 5).max(sc(720)).min(cx);
        ((cx - w) / 2, w)
    } else {
        (0, cx)
    }
}
unsafe fn bar_y() -> i32 {
    if at_top() { -bar_off() } else { bar_top() + bar_off() }
}
unsafe fn pop_y(h: i32) -> i32 {
    if vert() { GetSystemMetrics(SM_CYSCREEN) - sc(12) - h } else if at_top() { bar_h() + sc(12) } else { bar_top() - sc(12) - h }
}
const GOOD: u32 = winrust_common::style::GOOD;
const BAD: u32 = winrust_common::style::BAD;
const WARN: u32 = winrust_common::style::WARN;
const CALLBACK: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const NO_WINDOW: u32 = 0x0800_0000;
const BAR_H: i32 = 48;
const NONE: i32 = -1;
const CLOCK: i32 = -4;
const WIFI_I: i32 = -7;
const VOL_I: i32 = -8;
const BAT_I: i32 = -9;
const TAPP: i32 = -1000;
const SHOWDESK: i32 = -13;
const GEAR: i32 = -12;
const GEAR_X: (i32, i32) = (6, 36);
const WIN_START: u32 = WM_APP + 10;
const ALT_TAB: u32 = WM_APP + 11;
const ALT_UP: u32 = WM_APP + 12;
static SWITCHING: AtomicBool = AtomicBool::new(false);
static FULLSCREEN: AtomicBool = AtomicBool::new(false);
static DESK_HIDE: AtomicBool = AtomicBool::new(false);
static WIN_ALONE: AtomicBool = AtomicBool::new(false);
static BAR: AtomicIsize = AtomicIsize::new(0);
const WIN_NUM: u32 = WM_APP + 15;
const WIN_X: u32 = WM_APP + 17;
const WIN_TAB: u32 = WM_APP + 19;
const NEXT_LANG: u32 = WM_APP + 16;
const VOLKEY: u32 = WM_APP + 14;
const SNIP: u32 = WM_APP + 13;
static EXPLORER_UP: AtomicBool = AtomicBool::new(true);

struct Pin {
    lnk: Vec<u16>,
    target: String,
    args: String,
    icon: HICON,
}
struct Btn {
    pin: Option<usize>,
    wins: Vec<HWND>,
    icon: HICON,
}

struct TIcon {
    hwnd: HWND,
    uid: u32,
    guid: [u8; 16],
    icon: HICON,
    cb: u32,
    ver: u32,
    hidden: bool,
    tip: String,
    key: String,
}
const WM_COPYDATA: u32 = 0x4A;
#[repr(C)]
struct Cds {
    dw: usize,
    cb: u32,
    p: *const u8,
}
static USAGE: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

fn appdata_file(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join(name)
}
fn uses(name: &str) -> u32 {
    USAGE
        .lock()
        .unwrap()
        .get_or_insert_with(|| std::fs::read_to_string(appdata_file("topbar-usage.txt")).unwrap_or_default().lines().filter_map(|l| l.rsplit_once('\t')).filter_map(|(k, n)| Some((k.to_string(), n.parse().ok()?))).collect())
        .get(name)
        .copied()
        .unwrap_or(0)
}
fn bump(path: &str) {
    bump_name(&std::path::Path::new(path).file_stem().unwrap_or_default().to_string_lossy());
}
fn bump_name(name: &str) {
    let name = name.to_lowercase();
    let mut g = USAGE.lock().unwrap();
    let m = g.get_or_insert_with(|| {
        std::fs::read_to_string(appdata_file("topbar-usage.txt"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.rsplit_once('\t'))
            .filter_map(|(k, n)| Some((k.to_string(), n.parse().ok()?)))
            .collect()
    });
    *m.entry(name).or_insert(0) += 1;
    let _ = std::fs::write(appdata_file("topbar-usage.txt"), m.iter().map(|(k, n)| format!("{k}\t{n}\n")).collect::<String>());
}
fn lnk_uses(lnk: &std::path::Path) -> u32 {
    uses(&lnk.file_stem().unwrap_or_default().to_string_lossy().to_lowercase())
}

thread_local! {
    static HOST: Cell<HWND> = const { Cell::new(null_mut()) };
    static TIPWND: Cell<HWND> = const { Cell::new(null_mut()) };
    static SWWND: Cell<HWND> = const { Cell::new(null_mut()) };
    static SW: RefCell<(Vec<HWND>, usize)> = const { RefCell::new((Vec::new(), 0)) };
    static TIP: RefCell<String> = const { RefCell::new(String::new()) };
    static PINS: RefCell<Vec<Pin>> = const { RefCell::new(Vec::new()) };
    static BTNS: RefCell<Vec<Btn>> = const { RefCell::new(Vec::new()) };
    static TAPPS: RefCell<Vec<TIcon>> = const { RefCell::new(Vec::new()) };
    static SHOWN: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    static TRAYFLY: Cell<HWND> = const { Cell::new(null_mut()) };
    static TRAY_KEEP: Cell<bool> = const { Cell::new(false) };
    static FHOT: Cell<i32> = const { Cell::new(-1) };
    static HOT: Cell<i32> = const { Cell::new(NONE) };
    static ORDER: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
    static BAR_P: Cell<f32> = const { Cell::new(0.0) };
    static DESK_T: Cell<u64> = const { Cell::new(0) };
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
    static OLD_STATE: Cell<usize> = const { Cell::new(ABS_ALWAYSONTOP as usize) };
    static BAT: Cell<(i32, bool)> = const { Cell::new((-1, false)) };
    static VOL: Cell<(i32, bool)> = const { Cell::new((-1, false)) };
    static WIFI: Cell<i32> = const { Cell::new(-1) };
    static INET: Cell<bool> = const { Cell::new(true) };
    static TICK: Cell<u32> = const { Cell::new(0) };
    static EP: RefCell<Option<IAudioEndpointVolume>> = const { RefCell::new(None) };
    static TB_MSG: Cell<u32> = const { Cell::new(0) };
    static SH_MSG: Cell<u32> = const { Cell::new(0) };
    static EXP_TRAY: Cell<HWND> = const { Cell::new(null_mut()) };
    static APP_FG: Cell<HWND> = const { Cell::new(null_mut()) };
    static LANG_TRY: Cell<(u32, usize, usize)> = const { Cell::new((0, 0, 0)) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
fn with_wide<R>(s: &str, f: impl FnOnce(*const u16) -> R) -> R {
    thread_local! {
        static WIDE_TMP: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    }
    WIDE_TMP.with_borrow_mut(|b| {
        b.clear();
        b.extend(s.encode_utf16());
        b.push(0);
        f(b.as_ptr())
    })
}
fn eq_ascii(buf: &[u16], s: &str) -> bool {
    buf.len() == s.len() && buf.iter().zip(s.bytes()).all(|(&a, b)| a == b as u16)
}
fn sc(v: i32) -> i32 {
    (v as f32 * SCALE.get()) as i32
}
fn small_bar() -> bool {
    i18n::style::bar_size() == 0
}

fn bar_h() -> i32 {
    let d = [-14, 0, 8][i18n::style::bar_size() as usize % 3];
    if vert() { sc(60 + d) } else { sc(BAR_H + d) }
}
unsafe fn bar_top() -> i32 {
    if at_top() { 0 } else { GetSystemMetrics(SM_CYSCREEN) - bar_h() - dock_gap() }
}

const FCELL: i32 = 40;

fn fly_size() -> (usize, i32, i32) {
    let n = over_idx().len().max(1);
    let cols = n.min(5);
    (cols, sc(FCELL) * cols as i32 + sc(16), sc(FCELL) * n.div_ceil(cols) as i32 + sc(16))
}

unsafe fn paint_trayfly(hwnd: HWND, wdc: HDC, w: i32, h: i32) {
    if w <= 0 || h <= 0 {
        return;
    }
    thread_local! {
        static TF_BUF: RefCell<LayeredBuf> = const { RefCell::new(LayeredBuf::new()) };
    }
    let r = RECT { left: 0, top: 0, right: w, bottom: h };
    let sop = i18n::style::start_opacity();
    let ulw = sop < 100;
    let bg_a = if ulw { ((255 * sop as u32) / 100).max(1) as u8 } else { 255 };
    let (dc, _, _) = TF_BUF.with_borrow_mut(|b| b.begin(wdc, w, h));
    let rad = rr(16);
    fill_argb(dc, r, pal().bg, bg_a, rad);
    let (cols, _, _) = fly_size();
    let over = over_idx();
    TAPPS.with_borrow(|t| {
        if over.is_empty() {
            let of = SelectObject(dc, tip_font());
            text(dc, "No icons", r, pal().dim, DT_CENTER);
            SelectObject(dc, of);
            return;
        }
        for (n, ti) in over.iter().map(|&i| &t[i]).enumerate() {
            let (x, y) = (sc(8) + (n % cols) as i32 * sc(FCELL), sc(8) + (n / cols) as i32 * sc(FCELL));
            if FHOT.get() == n as i32 {
                let cell = RECT { left: x, top: y, right: x + sc(FCELL), bottom: y + sc(FCELL) };
                if ulw {
                    fill_argb(dc, cell, pal().hover, ((bg_a as u32) + 45).min(220) as u8, rr(8));
                } else {
                    fill(dc, cell, mix(pal().bg, pal().text, 0.12), rr(8));
                    fill(dc, RECT { left: cell.left + 1, top: cell.top + 1, right: cell.right - 1, bottom: cell.bottom - 1 }, pal().hover, rr(6));
                }
            }
            if !ti.icon.is_null() {
                let sz = sc(20);
                draw_icon(dc, x + (sc(FCELL) - sz) / 2, y + (sc(FCELL) - sz) / 2, sz, ti.icon);
            }
        }
    });
    LayeredBuf::present(hwnd, w, h, dc);
}

unsafe fn trayfly_pos(fw: i32, fh: i32) -> (i32, i32) {
    let scr = GetSystemMetrics(SM_CXSCREEN);
    let (bx, bw) = bar_x_w();
    let cx = bx + layout(if vert() { axis_len() } else { bw }).apps.0 + sc(12);
    if vert() {
        (side_x(fw), (cx - fh / 2).clamp(sc(12), GetSystemMetrics(SM_CYSCREEN) - fh - sc(12)))
    } else {
        ((cx - fw / 2).clamp(sc(12), scr - fw - sc(12)), pop_y(fh))
    }
}

unsafe fn hide_trayfly(w: HWND) {
    hide_tip(BAR.load(Relaxed) as HWND);
    ShowWindow(w, SW_HIDE);
    let bar = BAR.load(Relaxed) as HWND;
    if !bar.is_null() {
        InvalidateRect(bar, null(), 0);
    }
}

pub(crate) unsafe fn sync_trayfly() {
    let w = TRAYFLY.get();
    if !w.is_null() && IsWindowVisible(w) != 0 {
        if over_idx().is_empty() {
            hide_trayfly(w);
        } else {
            let (_, fw, fh) = fly_size();
            let (x, y) = trayfly_pos(fw, fh);
            SetWindowPos(w, HWND_TOPMOST, x, y, fw, fh, SWP_NOACTIVATE);
            let sdc = GetDC(null_mut());
            paint_trayfly(w, sdc, fw, fh);
            ReleaseDC(null_mut(), sdc);
        }
    }
    let bar = BAR.load(Relaxed) as HWND;
    if !bar.is_null() {
        InvalidateRect(bar, null(), 0);
    }
}

unsafe fn fly_toggle() {
    let w = TRAYFLY.get();
    if IsWindowVisible(w) != 0 {
        hide_trayfly(w);
        return;
    }
    let (_, fw, fh) = fly_size();
    FHOT.set(-1);
    let (x, y) = trayfly_pos(fw, fh);
    SetWindowPos(w, HWND_TOPMOST, x, y, fw, fh, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    let sdc = GetDC(null_mut());
    paint_trayfly(w, sdc, fw, fh);
    ReleaseDC(null_mut(), sdc);
    InvalidateRect(BAR.load(Relaxed) as HWND, null(), 0);
    force_fg(w);
}

unsafe extern "system" fn fly_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let cell_at = |lp: LPARAM| -> i32 {
        let (x, y) = ((lp & 0xffff) as i16 as i32 - sc(8), ((lp >> 16) & 0xffff) as i16 as i32 - sc(8));
        let (cols, _, _) = fly_size();
        let n = over_idx().len() as i32;
        if x < 0 || y < 0 || x >= sc(FCELL) * cols as i32 {
            return -1;
        }
        let i = (y / sc(FCELL)) * cols as i32 + x / sc(FCELL);
        if i < n { i } else { -1 }
    };
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let wdc = BeginPaint(hwnd, &mut ps);
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            paint_trayfly(hwnd, wdc, r.right, r.bottom);
            EndPaint(hwnd, &ps);
            0
        }
        WM_MOUSEMOVE => {
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            let hit = cell_at(lp);
            if FHOT.replace(hit) != hit {
                InvalidateRect(hwnd, null(), 0);
                if hit >= 0 {
                    let tip = over_idx().get(hit as usize).and_then(|&k| TAPPS.with_borrow(|t| t.get(k).map(|x| x.tip.clone()))).unwrap_or_default();
                    if !tip.is_empty() {
                        let mut wr: RECT = zeroed();
                        GetWindowRect(hwnd, &mut wr);
                        let (cols, _, _) = fly_size();
                        let cx = wr.left + sc(8) + (hit % cols as i32) * sc(FCELL) + sc(FCELL) / 2;
                        show_tip(tip, cx);
                    } else {
                        hide_tip(BAR.load(Relaxed) as HWND);
                    }
                } else {
                    hide_tip(BAR.load(Relaxed) as HWND);
                }
            }
            0
        }
        WM_MOUSELEAVE => {
            FHOT.set(-1);
            hide_tip(BAR.load(Relaxed) as HWND);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_RBUTTONDOWN => 0,
        WM_LBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN => {
            let i = cell_at(lp);
            let ti = over_idx().get(i.max(0) as usize).and_then(|&k| TAPPS.with_borrow(|t| t.get(k).map(|x| ((x.hwnd, x.uid, x.cb, x.ver), x.key.clone()))));
            if let (true, Some((t, key))) = (i >= 0, ti) {
                if msg == WM_MBUTTONDOWN {
                    toggle_shown(&key);
                    InvalidateRect(hwnd, null(), 0);
                } else {
                    hide_tip(BAR.load(Relaxed) as HWND);
                    AllowSetForegroundWindow(u32::MAX);
                    if msg == WM_RBUTTONUP {
                        TRAY_KEEP.set(true);
                        SetTimer(hwnd, 2, 120, None);
                    }
                    REVEAL_UNTIL.set(now_ms() + 600);
                    tray_click(t, msg == WM_LBUTTONDOWN);
                    if msg == WM_LBUTTONDOWN {
                        TRAY_KEEP.set(false);
                        hide_trayfly(hwnd);
                    }
                }
            }
            0
        }
        WM_ACTIVATE if wp & 0xffff == 0 => {
            if !TRAY_KEEP.get() {
                hide_trayfly(hwnd);
            } else {
                hide_tip(BAR.load(Relaxed) as HWND);
            }
            0
        }
        WM_TIMER if wp == 2 => {
            let esc = GetAsyncKeyState(VK_ESCAPE as i32) < 0;
            let outside = if GetAsyncKeyState(VK_LBUTTON as i32) < 0 {
                let mut pt: POINT = zeroed();
                let mut r: RECT = zeroed();
                GetCursorPos(&mut pt);
                GetWindowRect(hwnd, &mut r);
                pt.x < r.left || pt.x >= r.right || pt.y < r.top || pt.y >= r.bottom
            } else {
                false
            };
            if esc || outside {
                TRAY_KEEP.set(false);
                KillTimer(hwnd, 2);
                hide_trayfly(hwnd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn tip_font() -> HFONT {
    thread_local! {
        static TIP_FONT: Cell<(i32, HFONT)> = const { Cell::new((0, null_mut())) };
    }
    let px = sc(12);
    let (cpx, mut f) = TIP_FONT.get();
    if cpx != px || f.is_null() {
        if !f.is_null() {
            DeleteObject(f);
        }
        f = CreateFontW(-px, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(ui_face()).as_ptr());
        TIP_FONT.set((px, f));
    }
    f
}

unsafe extern "system" fn tip_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg != WM_PAINT {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(hwnd, &mut ps);
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    if r.right > 0 && r.bottom > 0 {
        thread_local! {
            static TIP_BUF: RefCell<LayeredBuf> = const { RefCell::new(LayeredBuf::new()) };
        }
        let (dc, _, _) = TIP_BUF.with_borrow_mut(|b| b.begin(wdc, r.right, r.bottom));
        let rad = rr(8);
        fill(dc, r, pal().bg, rad);
        stroke_round(dc, r, pal().line, 255, rad, 1.0);
        let of = SelectObject(dc, tip_font());
        let tr = RECT { left: sc(8), top: sc(5), right: r.right - sc(8), bottom: r.bottom - sc(5) };
        TIP.with_borrow(|s| text_box(dc, s, tr, pal().text, DT_LEFT | DT_NOPREFIX | DT_WORDBREAK));
        SelectObject(dc, of);
        EndPaint(hwnd, &ps);
        LayeredBuf::present(hwnd, r.right, r.bottom, dc);
    } else {
        EndPaint(hwnd, &ps);
    }
    0
}

unsafe fn tip_text(h: i32) -> String {
    match h {
        0 => "Start".into(),
        TAPP => "Hidden icons".into(),
        GEAR => "Style".into(),
        SHOWDESK => "Show desktop".into(),
        CLOCK => {
            let mut t: SYSTEMTIME = zeroed();
            GetLocalTime(&mut t);
            let mut b = [0u16; 80];
            let n = GetDateFormatW(0x0400, 2, &t, null(), b.as_mut_ptr(), 80).max(1) as usize - 1;
            String::from_utf16_lossy(&b[..n])
        }
        WIFI_I => match net() {
            Link::Wifi(q) => format!("Wi‑Fi · signal {q}%"),
            Link::NoInternet(_) => "Wi‑Fi connected, no internet".into(),
            Link::Cable => "Internet over a cable".into(),
            Link::Offline => "Not connected".into(),
        },
        VOL_I => vol_tip(),
        BAT_I => match BAT.get() {
            (p, ac) if p >= 0 => format!("Battery {p}%{}", if ac { " · plugged in" } else { "" }),
            _ => String::new(),
        },
        t if t > TAPP && t < TAPP + 100 => bar_idx().get((t - TAPP - 1) as usize).and_then(|&i| TAPPS.with_borrow(|t| t.get(i).map(|i| i.tip.clone()))).unwrap_or_default(),
        i if i >= 1 => {
            let (wins, lnk) = BTNS.with_borrow(|b| b.get(i as usize).map(|b| (b.wins.clone(), b.pin.and_then(|p| PINS.with_borrow(|p2| p2.get(p).map(|x| wstr_of(&x.lnk))))))).unwrap_or_default();
            let title = |w: HWND| {
                let mut b = [0u16; 256];
                let n = GetWindowTextW(w, b.as_mut_ptr(), 256).max(0) as usize;
                String::from_utf16_lossy(&b[..n])
            };
            match (wins.first(), lnk) {
                (Some(&w), _) if wins.len() > 1 => format!("{}  (+{})", title(w), wins.len() - 1),
                (Some(&w), _) => title(w),
                (None, Some(l)) => std::path::Path::new(&l).file_stem().unwrap_or_default().to_string_lossy().into(),
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
}

fn vol_tip() -> String {
    match VOL.get() {
        (v, false) if v >= 0 => format!("Volume {v}%"),
        (_, true) => "Muted".into(),
        _ => "Volume".into(),
    }
}

unsafe fn show_tip(s: String, cx: i32) {
    let tw = TIPWND.get();
    if s.is_empty() || tw.is_null() {
        return;
    }
    let (dc, f) = (GetDC(tw), tip_font());
    let of = SelectObject(dc, f);
    let mut r = RECT { left: 0, top: 0, right: sc(280), bottom: 0 };
    with_wide(&s, |ws| DrawTextW(dc, ws, -1, &mut r, DT_CALCRECT | DT_NOPREFIX | DT_WORDBREAK));
    SelectObject(dc, of);
    ReleaseDC(tw, dc);
    let (w, h) = (r.right + sc(16), r.bottom + sc(10));
    TIP.set(s);
    if vert() {
        let y = (cx - h / 2).clamp(sc(4), GetSystemMetrics(SM_CYSCREEN) - h - sc(4));
        SetWindowPos(tw, HWND_TOPMOST, side_x(w), y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        InvalidateRect(tw, null(), 0);
        return;
    }
    let x = (cx - w / 2).clamp(sc(4), GetSystemMetrics(SM_CXSCREEN) - w - sc(4));
    let tf = TRAYFLY.get();
    let y = if !tf.is_null() && IsWindowVisible(tf) != 0 {
        let mut wr: RECT = zeroed();
        GetWindowRect(tf, &mut wr);
        (wr.top - h - sc(6)).max(sc(4))
    } else if at_top() {
        bar_h() - bar_off() + sc(6)
    } else {
        bar_top() + bar_off() - h - sc(6)
    };
    SetWindowPos(tw, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    InvalidateRect(tw, null(), 0);
}

thread_local! {
    static SW_MON: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
    static SW_CARDS: RefCell<(Vec<RECT>, Vec<isize>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

unsafe fn alt_tab(back: bool) {
    let sw = SWWND.get();
    if IsWindowVisible(sw) == 0 {
        let mut v: Vec<HWND> = Vec::new();
        EnumWindows(Some(collect), &mut v as *mut _ as LPARAM);
        let mut mon = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
        if i18n::style::alttab_per_monitor() && !v.is_empty() {
            let of = |w: HWND| MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST);
            let m0 = of(v[0]);
            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(m0, &mut mi) != 0 {
                let same: Vec<HWND> = v.iter().copied().filter(|&w| of(w) == m0).collect();
                if same.len() > 1 {
                    v = same;
                    mon = mi.rcMonitor;
                }
            }
        }
        let n = v.len();
        if n == 0 {
            return SWITCHING.store(false, Relaxed);
        }
        SW.set((v, if n == 1 { 0 } else if back { n - 1 } else { 1 }));
        SW_MON.set(mon);
        sw_layout(sw);
    } else {
        SW.with_borrow_mut(|(v, i)| *i = if back { (*i + v.len() - 1) % v.len() } else { (*i + 1) % v.len() });
    }
    InvalidateRect(sw, null(), 0);
}

unsafe fn sw_layout(sw: HWND) {
    let mon = SW_MON.get();
    let (scr, sch) = (mon.right - mon.left, mon.bottom - mon.top);
    let pad = sc(thumbs::GAP) + sc(4);
    let wins = SW.with_borrow(|(v, _)| v.clone());
    let (mut rects, tw, th) = thumbs::grid(wins.len(), scr * 9 / 10 - 2 * pad, sch * 8 / 10 - 2 * pad, sc(thumbs::CARD_W), sc(thumbs::CARD_H), sc(thumbs::GAP));
    rects.iter_mut().for_each(|r| *r = RECT { left: r.left + pad, top: r.top + pad, right: r.right + pad, bottom: r.bottom + pad });
    let (w, h) = (tw + 2 * pad, th + 2 * pad);
    SW_CARDS.with_borrow_mut(|(_, ids)| thumbs::unregister(sw, ids));
    SetWindowPos(sw, HWND_TOPMOST, mon.left + (scr - w) / 2, mon.top + (sch - h) / 2, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    let ids = thumbs::register(sw, &wins, &rects);
    SW_CARDS.set((rects, ids));
}

unsafe fn alt_close() {
    let w = SW.with_borrow_mut(|(v, i)| {
        let w = v.get(*i).copied();
        if w.is_some() {
            v.remove(*i);
            *i = (*i).min(v.len().saturating_sub(1));
        }
        w
    });
    if let Some(w) = w {
        PostMessageW(w, WM_CLOSE, 0, 0);
    }
    if SW.with_borrow(|(v, _)| v.is_empty()) {
        alt_done(true);
        SWITCHING.store(false, Relaxed);
    } else {
        sw_layout(SWWND.get());
        InvalidateRect(SWWND.get(), null(), 0);
    }
}

unsafe fn alt_done(cancel: bool) {
    ShowWindow(SWWND.get(), SW_HIDE);
    SW_CARDS.with_borrow_mut(|(r, ids)| {
        thumbs::unregister(SWWND.get(), ids);
        r.clear();
    });
    let (v, i) = SW.take();
    if let Some(&w) = v.get(i).filter(|_| !cancel) {
        if IsIconic(w) != 0 {
            ShowWindow(w, SW_RESTORE);
        }
        force_fg(w);
    }
}

unsafe extern "system" fn sw_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16) as i16 as i32);
    let at = || SW_CARDS.with_borrow(|(r, _)| thumbs::card_at(r, x, y));
    match msg {
        WM_PAINT => SW.with_borrow(|(v, sel)| SW_CARDS.with_borrow(|(r, ids)| thumbs::paint(hwnd, v, r, ids, Some(*sel)))),
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
        WM_LBUTTONUP | WM_MBUTTONUP => {
            let Some(k) = at() else { return 0 };
            SW.with_borrow_mut(|(_, i)| *i = k);
            let c = SW_CARDS.with_borrow(|(r, _)| thumbs::close_rect(&r[k]));
            if msg == WM_MBUTTONUP || (x >= c.left && x < c.right && y >= c.top && y < c.bottom) {
                alt_close();
            } else {
                SWITCHING.store(false, Relaxed);
                if GetAsyncKeyState(VK_MENU as i32) < 0 {
                    keys(&[0xE8]);
                }
                alt_done(false);
            }
        }
        WM_MOUSEWHEEL => alt_tab((wp >> 16) as i16 > 0),
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

unsafe fn hide_tip(bar: HWND) {
    KillTimer(bar, 2);
    ShowWindow(TIPWND.get(), SW_HIDE);
}

unsafe fn set_state(hwnd: HWND, state: usize) {
    let mut ab: APPBARDATA = zeroed();
    ab.cbSize = size_of::<APPBARDATA>() as u32;
    ab.hWnd = hwnd;
    ab.lParam = state as isize;
    SHAppBarMessage(ABM_SETSTATE, &mut ab);
}

unsafe fn hide_taskbar() {
    set_state(find_other("Shell_TrayWnd"), ABS_AUTOHIDE as usize);
    for t in foreign_taskbar_windows() {
        ShowWindow(t, SW_HIDE);
    }
}

fn taskbar_state_file() -> PathBuf {
    appdata_file("topbar-taskbar-state.txt")
}

fn saved_taskbar_state() -> Option<usize> {
    std::fs::read_to_string(taskbar_state_file()).ok()?.trim().parse().ok()
}

fn forget_taskbar_state() {
    let _ = std::fs::remove_file(taskbar_state_file());
}

unsafe fn remember_taskbar_state() -> usize {
    if let Some(s) = saved_taskbar_state() {
        return s;
    }
    let tb = find_other("Shell_TrayWnd");
    if tb.is_null() {
        return ABS_ALWAYSONTOP as usize;
    }
    let mut ab: APPBARDATA = zeroed();
    ab.cbSize = size_of::<APPBARDATA>() as u32;
    ab.hWnd = tb;
    let s = SHAppBarMessage(ABM_GETSTATE, &mut ab);
    let _ = std::fs::write(taskbar_state_file(), s.to_string());
    s
}

unsafe fn show_taskbar(state: usize) {
    set_state(find_other("Shell_TrayWnd"), state);
    for t in foreign_taskbar_windows() {
        ShowWindow(t, SW_SHOWNA);
    }
}

unsafe fn hide_taskbar_if_shown() {
    if foreign_taskbar_windows().into_iter().any(|t| IsWindowVisible(t) != 0) {
        hide_taskbar();
    }
}

unsafe fn place(hwnd: HWND) {
    apply_opacity(hwnd);
    let (cx, cy) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    let mut ab: APPBARDATA = zeroed();
    ab.cbSize = size_of::<APPBARDATA>() as u32;
    ab.hWnd = hwnd;
    let strip = if dock() || i18n::style::autohide() { 0 } else { bar_h() };
    if vert() {
        let left = i18n::style::side() == i18n::style::Side::Left;
        ab.uEdge = if left { ABE_LEFT } else { ABE_RIGHT };
        ab.rc = if left { RECT { left: 0, top: 0, right: strip, bottom: cy } } else { RECT { left: cx - strip, top: 0, right: cx, bottom: cy } };
        SHAppBarMessage(ABM_QUERYPOS, &mut ab);
        if left { ab.rc.left = sane_edge(ab.rc.left, cx) } else { ab.rc.right = cx - sane_edge(cx - ab.rc.right, cx) }
        if left { ab.rc.right = ab.rc.left + strip } else { ab.rc.left = ab.rc.right - strip }
    } else if at_top() {
        ab.uEdge = ABE_TOP;
        ab.rc = RECT { left: 0, top: 0, right: cx, bottom: strip };
        SHAppBarMessage(ABM_QUERYPOS, &mut ab);
        ab.rc.top = sane_edge(ab.rc.top, cy);
        ab.rc.bottom = ab.rc.top + strip;
    } else {
        ab.uEdge = ABE_BOTTOM;
        ab.rc = RECT { left: 0, top: cy - strip, right: cx, bottom: cy };
        SHAppBarMessage(ABM_QUERYPOS, &mut ab);
        ab.rc.bottom = cy - sane_edge(cy - ab.rc.bottom, cy);
        ab.rc.top = ab.rc.bottom - strip;
    }
    SHAppBarMessage(ABM_SETPOS, &mut ab);
    {
        set_work_area(if vert() { if ab.uEdge == ABE_LEFT { RECT { left: ab.rc.right, top: 0, right: cx, bottom: cy } } else { RECT { left: 0, top: 0, right: ab.rc.left, bottom: cy } } } else if at_top() { RECT { left: 0, top: ab.rc.bottom, right: cx, bottom: cy } } else { RECT { left: 0, top: 0, right: cx, bottom: ab.rc.top } });
    }
    let (x, w) = bar_x_w();
    if dock() && i18n::style::bar_opacity() >= 100 {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED as isize));
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_LAYERED as isize));
    }
    SetWindowRgn(hwnd, null_mut(), 1);
    if vert() {
        SetWindowPos(hwnd, HWND_TOPMOST, bar_x(), 0, bar_h(), cy, SWP_NOACTIVATE);
    } else {
        SetWindowPos(hwnd, HWND_TOPMOST, x, bar_y(), w, bar_h(), SWP_NOACTIVATE);
    }
    InvalidateRect(hwnd, null(), 0);
}

fn sane_edge(offset: i32, size: i32) -> i32 {
    if (0..=size / 4).contains(&offset) { offset } else { 0 }
}

unsafe fn set_work_area(mut want: RECT) {
    let mut cur: RECT = zeroed();
    SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut cur as *mut _ as *mut _, 0);
    if (cur.left, cur.top, cur.right, cur.bottom) != (want.left, want.top, want.right, want.bottom) {
        SystemParametersInfoW(SPI_SETWORKAREA, 0, &mut want as *mut _ as *mut _, 0);
        PostMessageW(HWND_BROADCAST, WM_SETTINGCHANGE, SPI_SETWORKAREA as usize, 0);
        EnumWindows(Some(fit_maximized), &want as *const RECT as isize);
    }
}

unsafe extern "system" fn fit_maximized(h: HWND, lp: isize) -> i32 {
    let want = &*(lp as *const RECT);
    if IsWindowVisible(h) == 0 || IsZoomed(h) == 0 || MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) != MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) {
        return 1;
    }
    let mut r: RECT = zeroed();
    GetWindowRect(h, &mut r);
    let f =GetSystemMetrics(SM_CXFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER);
    let n = RECT { left: r.left.max(want.left - f), top: r.top.max(want.top - f), right: r.right.min(want.right + f), bottom: r.bottom.min(want.bottom + f) };
    if (n.left, n.top, n.right, n.bottom) != (r.left, r.top, r.right, r.bottom) && n.right > n.left && n.bottom > n.top {
        SetWindowPos(h, null_mut(), n.left, n.top, n.right - n.left, n.bottom - n.top, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    1
}

thread_local! {
    static EXE_CACHE: RefCell<HashMap<usize, (u32, String)>> = RefCell::new(HashMap::new());
}

unsafe fn exe_of(h: HWND) -> String {
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, &mut pid);
    if let Some(e) = EXE_CACHE.with_borrow(|c| c.get(&(h as usize)).filter(|(p, e)| *p == pid && !e.is_empty()).map(|(_, e)| e.clone())) {
        return e;
    }
    let p = OpenProcess(0x1000, 0, pid);
    if p.is_null() {
        return String::new();
    }
    let (mut b, mut n) = ([0u16; 520], 520u32);
    let ok = QueryFullProcessImageNameW(p, 0, b.as_mut_ptr(), &mut n);
    CloseHandle(p);
    let exe = if ok == 0 { String::new() } else { String::from_utf16_lossy(&b[..n as usize]).to_lowercase() };
    EXE_CACHE.with_borrow_mut(|c| {
        if c.len() > 512 {
            c.clear();
        }
        c.insert(h as usize, (pid, exe.clone()));
    });
    exe
}

unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> i32 {
    let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
    let mut cloaked = 0u32;
    DwmGetWindowAttribute(h, 14, &mut cloaked as *mut _ as *mut _, 4);
    let mut cls = [0u16; 32];
    let n = GetClassNameW(h, cls.as_mut_ptr(), 32).max(0) as usize;
    let ours = eq_ascii(&cls[..n], "topbar")
        || eq_ascii(&cls[..n], "topbar_fly")
        || eq_ascii(&cls[..n], "Progman")
        || eq_ascii(&cls[..n], "WorkerW")
        || eq_ascii(&cls[..n], "Shell_TrayWnd")
        || eq_ascii(&cls[..n], "Shell_SecondaryTrayWnd");
    if IsWindowVisible(h) != 0 && GetWindow(h, GW_OWNER).is_null() && cloaked == 0 && !ours && GetWindowTextLengthW(h) > 0 && (ex & WS_EX_TOOLWINDOW == 0 || ex & WS_EX_APPWINDOW != 0) {
        (*(lp as *mut Vec<HWND>)).push(h);
    }
    1
}

unsafe fn resolve_cmd(s: &str) -> String {
    use windows_sys::Win32::Storage::FileSystem::SearchPathW;
    let e = expand_env(s);
    if e.contains(['\\', '/', ':']) {
        return e;
    }
    let mut b = [0u16; 520];
    let n = SearchPathW(null(), wide(&e).as_ptr(), wide(".exe").as_ptr(), b.len() as u32, b.as_mut_ptr(), null_mut()) as usize;
    if n > 0 && n < b.len() { String::from_utf16_lossy(&b[..n]) } else { e }
}

fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        if let Some(b) = after.find('%') {
            let var = &after[..b];
            match std::env::var(var) {
                Ok(v) => out.push_str(&v),
                Err(_) => {
                    out.push('%');
                    out.push_str(var);
                    out.push('%');
                }
            }
            rest = &after[b + 1..];
        } else {
            out.push('%');
            rest = after;
            break;
        }
    }
    out.push_str(rest);
    out
}

pub(crate) unsafe fn lnk_target(lnk: &[u16]) -> String {
    (|| -> Option<String> {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        sl.cast::<IPersistFile>().ok()?.Load(PCWSTR(lnk.as_ptr()), STGM(0)).ok()?;
        let mut b = [0u16; 520];
        sl.GetPath(&mut b, null_mut(), 0).ok()?;
        let path = String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0)?]);
        if !path.is_empty() {
            return Some(path.to_lowercase());
        }
        let mut idx = 0;
        sl.GetIconLocation(&mut b, &mut idx).ok()?;
        let icon = String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0)?]);
        let icon = expand_env(&icon).to_lowercase();
        icon.ends_with(".exe").then_some(icon)
    })()
    .unwrap_or_default()
}

unsafe fn lnk_icon(lnk: &[u16], target: &str) -> HICON {
    lnk_icon_sz(lnk, target, icon_px())
}

unsafe fn lnk_icon_sz(lnk: &[u16], target: &str, sz: i32) -> HICON {
    if !target.is_empty() && std::path::Path::new(target).is_file() {
        if sz > 24 {
            let ic = item_icon(target, sz);
            if !ic.is_null() {
                return ic;
            }
        }
        if target.to_ascii_lowercase().ends_with(".exe") {
            let mut big: HICON = null_mut();
            let mut sml: HICON = null_mut();
            ExtractIconExW(wide(target).as_ptr(), 0, &mut big, &mut sml, 1);
            let icon = if sz > 20 && !big.is_null() {
                if !sml.is_null() { DestroyIcon(sml); }
                big
            } else if !sml.is_null() {
                if !big.is_null() { DestroyIcon(big); }
                sml
            } else {
                big
            };
            if !icon.is_null() {
                return icon;
            }
        }
        let ic = item_icon(target, sz);
        if !ic.is_null() {
            return ic;
        }
    }
    let ic = item_icon(&resolve_cmd(&wstr_of(lnk)), sz);
    if !ic.is_null() {
        return ic;
    }
    let mut sfi: SHFILEINFOW = zeroed();
    SHGetFileInfoW(lnk.as_ptr(), 0, &mut sfi, size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | if sz > 20 { SHGFI_LARGEICON } else { SHGFI_SMALLICON });
    sfi.hIcon
}

fn icon_px() -> i32 {
    sc(if small_bar() { 18 } else { 24 })
}

unsafe fn item_icon(path: &str, px: i32) -> HICON {
    use windows::Win32::{
        Foundation::SIZE,
        UI::Shell::{IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY},
    };
    let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide(path).as_ptr()), None) else { return null_mut() };
    let Ok(f) = item.cast::<IShellItemImageFactory>() else { return null_mut() };
    let Ok(bmp) = f.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY) else { return null_mut() };
    let color = bmp.0 as HBITMAP;
    let mask = CreateBitmap(px, px, 1, 1, null());
    let mut ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let ic = CreateIconIndirect(&mut ii);
    DeleteObject(mask);
    DeleteObject(color);
    ic
}

thread_local! {
    static EXE_ICONS: RefCell<HashMap<String, HICON>> = RefCell::new(HashMap::new());
}

pub(crate) unsafe fn window_own_icon(h: HWND) -> HICON {
    for which in [1usize, 2, 0] {
        let mut r = 0usize;
        SendMessageTimeoutW(h, WM_GETICON, which, 0, SMTO_ABORTIFHUNG, 40, &mut r);
        if r != 0 {
            return r as HICON;
        }
    }
    let c = GetClassLongPtrW(h, GCLP_HICON);
    let hc = (if c != 0 { c } else { GetClassLongPtrW(h, GCLP_HICONSM) }) as HICON;
    if !hc.is_null() { hc } else { null_mut() }
}

pub(crate) unsafe fn aumid_icon(aumid: &str, px: i32) -> HICON {
    if aumid.is_empty() {
        return null_mut();
    }
    let id = format!(r"shell:AppsFolder\{aumid}");
    item_icon(&id, px)
}

unsafe fn cache_icon(key: String, make: impl FnOnce() -> HICON) -> HICON {
    if let Some(ic) = EXE_ICONS.with_borrow(|m| m.get(&key).copied()) {
        return ic;
    }
    let ic = make();
    if !ic.is_null() {
        EXE_ICONS.with_borrow_mut(|m| { m.insert(key, ic); });
    }
    ic
}

unsafe fn win_icon_with_aumid(w: HWND, aumid: &str) -> (HICON, bool) {
    let exe = exe_of(w);
    if exe.ends_with("applicationframehost.exe") {
        let ic = store_icon(w);
        if !ic.is_null() {
            return (ic, false);
        }
    }
    let own = window_own_icon(w);
    if !own.is_null() {
        return (own, false);
    }
    if !aumid.is_empty() {
        let ic = cache_icon(format!("aumid:{aumid}"), || aumid_icon(aumid, icon_px()));
        if !ic.is_null() {
            return (ic, false);
        }
    }
    if !exe.is_empty() && !is_host(&exe) {
        let ic = cache_icon(exe.clone(), || {
            let s = item_icon(&exe, icon_px());
            if !s.is_null() {
                return s;
            }
            let mut big: HICON = null_mut();
            ExtractIconExW(wide(&exe).as_ptr(), 0, &mut big, null_mut(), 1);
            big
        });
        if !ic.is_null() {
            return (ic, false);
        }
    }
    (LoadIconW(null_mut(), IDI_APPLICATION), true)
}

fn is_host(exe: &str) -> bool {
    const HOSTS: [&str; 8] = ["applicationframehost.exe", "javaw.exe", "java.exe", "pythonw.exe", "python.exe", "mmc.exe", "rundll32.exe", "dllhost.exe"];
    HOSTS.iter().any(|h| exe.rsplit('\\').next() == Some(*h))
}

unsafe fn store_icon(frame: HWND) -> HICON {
    use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
    let core = FindWindowExW(frame, null_mut(), wide("Windows.UI.Core.CoreWindow").as_ptr(), null());
    if core.is_null() {
        return null_mut();
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(core, &mut pid);
    let p = OpenProcess(windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if p.is_null() {
        return null_mut();
    }
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    let ok = GetApplicationUserModelId(p, &mut len, buf.as_mut_ptr()) == 0;
    CloseHandle(p);
    if !ok {
        return null_mut();
    }
    let id = format!(r"shell:AppsFolder\{}", wstr_of(&buf));
    cache_icon(id.clone(), || item_icon(&id, icon_px()))
}

fn msix_family(target: &str) -> Option<(String, String)> {
    let l = target.to_lowercase();
    let (_, tail) = l.split_once(r"\windowsapps\")?;
    let (folder, rest) = tail.split_once('\\')?;
    let mut it = folder.split('_');
    let (p0, _, _, _, p4) = (it.next()?, it.next()?, it.next()?, it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    Some((format!("{p0}_{p4}"), rest.to_string()))
}

unsafe fn fix_msix_pin(lnk: &[u16], target: &str) -> Option<String> {
    use windows_sys::Win32::Storage::Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily};
    if target.is_empty() || std::path::Path::new(target).exists() {
        return None;
    }
    let (family, rest) = msix_family(target)?;
    let fam = wide(&family);
    let (mut count, mut len) = (0u32, 0u32);
    GetPackagesByPackageFamily(fam.as_ptr(), &mut count, null_mut(), &mut len, null_mut());
    if count == 0 {
        return None;
    }
    let (mut names, mut buf) = (vec![null_mut::<u16>(); count as usize], vec![0u16; len as usize]);
    if GetPackagesByPackageFamily(fam.as_ptr(), &mut count, names.as_mut_ptr(), &mut len, buf.as_mut_ptr()) != 0 {
        return None;
    }
    let mut path = [0u16; 520];
    let mut plen = path.len() as u32;
    if GetPackagePathByFullName(names[0], &mut plen, path.as_mut_ptr()) != 0 {
        return None;
    }
    let new = std::path::Path::new(&wstr_of(&path)).join(&rest);
    if !new.exists() {
        return None;
    }
    let new = new.to_string_lossy().to_lowercase();
    let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
    let pf = sl.cast::<IPersistFile>().ok()?;
    pf.Load(PCWSTR(lnk.as_ptr()), STGM(0)).ok()?;
    sl.SetPath(PCWSTR(wide(&new).as_ptr())).ok()?;
    pf.Save(PCWSTR(lnk.as_ptr()), true).ok()?;
    Some(new)
}

fn pin_order_file() -> PathBuf {
    appdata_file("topbar-pin-order.txt")
}

unsafe fn save_pin_order() {
    let text: String = PINS.with_borrow(|p| p.iter().map(|x| format!("{}
", std::path::Path::new(&wstr_of(&x.lnk)).file_name().unwrap_or_default().to_string_lossy())).collect());
    let _ = std::fs::write(pin_order_file(), text);
}

unsafe fn pin_key(lnk: &[u16]) -> (String, String) {
    let target = lnk_target(lnk);
    if !target.is_empty() {
        return (target, lnk_args(lnk).to_lowercase());
    }
    let id = lnk_aumid(lnk).to_lowercase();
    (if id.is_empty() { id } else { format!("aumid:{id}") }, String::new())
}

const AUMID_KEY: windows::Win32::Foundation::PROPERTYKEY = windows::Win32::Foundation::PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
    pid: 5,
};

unsafe fn lnk_aumid(lnk: &[u16]) -> String {
    use windows::Win32::{System::Com::{CoTaskMemFree, StructuredStorage::PropVariantToStringAlloc}, UI::Shell::PropertiesSystem::IPropertyStore};
    (|| -> Option<String> {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        sl.cast::<IPersistFile>().ok()?.Load(PCWSTR(lnk.as_ptr()), STGM(0)).ok()?;
        let ps: IPropertyStore = sl.cast().ok()?;
        let v = ps.GetValue(&AUMID_KEY).ok()?;
        let p = PropVariantToStringAlloc(&v).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    })()
    .unwrap_or_default()
}

unsafe fn win_aumid(h: HWND) -> String {
    use windows::Win32::{Foundation::HWND as WHWND, System::Com::{CoTaskMemFree, StructuredStorage::PropVariantToStringAlloc}, UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow}};
    (|| -> Option<String> {
        let ps: IPropertyStore = SHGetPropertyStoreForWindow(WHWND(h)).ok()?;
        let p = PropVariantToStringAlloc(&ps.GetValue(&AUMID_KEY).ok()?).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    })()
    .unwrap_or_default()
    .to_lowercase()
}

fn unpinned_file() -> PathBuf {
    appdata_file(r"topbar-taskband\unpinned.txt")
}

fn key_line(k: &(String, String)) -> String {
    format!("{}|{}", k.0, k.1)
}

unsafe fn taskband_links() -> Vec<((String, String), PathBuf)> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW};
    let mut n = 0u32;
    let (key, val) = (wide(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Taskband"), wide("FavoritesResolve"));
    if RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_BINARY, null_mut(), null_mut(), &mut n) != 0 || n == 0 {
        return vec![];
    }
    let mut blob = vec![0u8; n as usize];
    if RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_BINARY, null_mut(), blob.as_mut_ptr().cast(), &mut n) != 0 {
        return vec![];
    }
    const HEAD: [u8; 20] = [0x4C, 0, 0, 0, 0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
    let starts: Vec<usize> = (0..blob.len().saturating_sub(HEAD.len())).filter(|&i| blob[i..i + HEAD.len()] == HEAD).collect();
    let dir = appdata_file("topbar-taskband");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::read_dir(&dir).into_iter().flatten().flatten().filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk"))).for_each(|e| {
        let _ = std::fs::remove_file(e.path());
    });
    let tmp = dir.join("tmp.lnk");
    let gone: Vec<String> = std::fs::read_to_string(unpinned_file()).unwrap_or_default().lines().map(str::to_string).collect();
    let mut out = vec![];
    for (k, &a) in starts.iter().enumerate() {
        let b = starts.get(k + 1).copied().unwrap_or(blob.len());
        if std::fs::write(&tmp, &blob[a..b]).is_err() {
            continue;
        }
        let key = pin_key(&wide(&tmp.to_string_lossy()));
        if key.0.is_empty() || gone.contains(&key_line(&key)) {
            continue;
        }
        if key.0.ends_with(".lnk") {
            let real = PathBuf::from(&key.0);
            let rk = pin_key(&wide(&real.to_string_lossy()));
            if real.is_file() && !rk.0.is_empty() && !gone.contains(&key_line(&rk)) {
                out.push((rk, real));
            }
            continue;
        }
        let base = key.0.strip_prefix("aumid:").unwrap_or_else(|| std::path::Path::new(&key.0).file_stem().and_then(|s| s.to_str()).unwrap_or(""));
        let stem: String = base.chars().map(|c| if c.is_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(40).collect();
        let h = key_line(&key).bytes().fold(5381u32, |h, c| h.wrapping_mul(33) ^ c as u32);
        let file = dir.join(format!("{stem}-{h:08x}.lnk"));
        if std::fs::copy(&tmp, &file).is_ok() {
            out.push((key, file));
        }
    }
    let _ = std::fs::remove_file(tmp);
    out
}

unsafe fn load_pins() {
    let Some(appdata) = std::env::var_os("APPDATA") else { return };
    let dir = PathBuf::from(appdata).join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");
    let mut files: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk"))).collect();
    files.sort();
    files.sort_by_key(|f| std::cmp::Reverse(lnk_uses(f)));
    let fav = taskband_links();
    let key_of = |f: &PathBuf| pin_key(&wide(&f.to_string_lossy()));
    let mut keys: Vec<_> = files.iter().map(key_of).collect();
    for (k, f) in &fav {
        if !keys.contains(k) {
            keys.push(k.clone());
            files.push(f.clone());
        }
    }
    let at: HashMap<PathBuf, usize> = files.iter().cloned().zip(&keys).map(|(f, k)| (f, fav.iter().position(|(fk, _)| fk == k).unwrap_or(usize::MAX))).collect();
    files.sort_by_key(|f| at[f]);
    let order: Vec<String> = std::fs::read_to_string(pin_order_file()).unwrap_or_default().lines().map(|l| l.to_lowercase()).collect();
    files.sort_by_key(|f| order.iter().position(|n| *n == f.file_name().unwrap_or_default().to_string_lossy().to_lowercase()).unwrap_or(usize::MAX));
    for f in files {
        let lnk = wide(&f.to_string_lossy());
        let target = lnk_target(&lnk);
        let target = fix_msix_pin(&lnk, &target).unwrap_or(target);
        let icon = lnk_icon(&lnk, &target);
        let args = lnk_args(&lnk);
        PINS.with_borrow_mut(|p| p.push(Pin { lnk, target, args, icon }));
    }
    let marker = appdata_file("topbar-explorer-pin.txt");
    let rx = std::env::var_os("ProgramFiles").map(|pf| PathBuf::from(pf).join(r"rust-explorer\rust-explorer.exe"));
    if let Some(rx) = rx.filter(|rx| rx.is_file() && !marker.exists()) {
        pin_exe(&rx.to_string_lossy(), "");
        let _ = std::fs::write(marker, "1");
    }
}

fn refresh_buttons() {
    let mut wins: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(collect), &mut wins as *mut _ as LPARAM) };
    let mut btns = vec![Btn { pin: None, wins: vec![], icon: null_mut() }];
    PINS.with_borrow(|pins| btns.extend((0..pins.len()).map(|i| Btn { pin: Some(i), wins: vec![], icon: null_mut() })));
    let mut loose: Vec<HWND> = Vec::new();
    for w in wins {
        let exe = unsafe { exe_of(w) };
        let pin = PINS.with_borrow(|p| p.iter().position(|p| !p.target.is_empty() && p.target == exe));
        match pin {
            Some(i) => btns[1 + i].wins.push(w),
            None => loose.push(w),
        }
    }
    let mut keys: Vec<String> = vec![String::new(); btns.len()];
    ORDER.with_borrow_mut(|o| {
        o.retain(|w| loose.contains(w));
        for &w in &loose {
            if !o.contains(&w) {
                o.push(w);
            }
        }
        for &w in o.iter() {
            let exe = unsafe { exe_of(w) };
            let (aumid, icon) = unsafe { win_meta(w) };
            let key = format!("{exe}|{aumid}");
            let same = (!exe.is_empty() && !is_host(&exe)).then(|| keys.iter().position(|k| *k == key)).flatten();
            match same {
                Some(i) => btns[i].wins.push(w),
                None => {
                    btns.push(Btn { pin: None, wins: vec![w], icon });
                    keys.push(key);
                }
            }
        }
    });
    BTNS.set(btns);
}

thread_local! {
    static META: RefCell<HashMap<usize, (u32, u64, String, HICON, bool)>> = RefCell::new(HashMap::new());
}
const META_RETRY_MS: u64 = 500;

unsafe fn win_meta(w: HWND) -> (String, HICON) {
    let mut pid = 0u32;
    GetWindowThreadProcessId(w, &mut pid);
    let now = now_ms();
    if let Some((aumid, ic, fallback, t)) = META.with_borrow(|m| m.get(&(w as usize)).filter(|e| e.0 == pid).map(|e| (e.2.clone(), e.3, e.4, e.1))) {
        if !fallback || now.saturating_sub(t) < META_RETRY_MS {
            return (aumid, ic);
        }
        let (new_ic, new_fb) = win_icon_with_aumid(w, &aumid);
        let (keep_ic, keep_fb) = if !new_fb && !new_ic.is_null() { (new_ic, false) } else { (ic, fallback) };
        META.with_borrow_mut(|m| { m.insert(w as usize, (pid, now, aumid.clone(), keep_ic, keep_fb)); });
        return (aumid, keep_ic);
    }
    let aumid = win_aumid(w);
    let (ic, fb) = win_icon_with_aumid(w, &aumid);
    META.with_borrow_mut(|m| {
        if m.len() > 256 {
            m.retain(|&hw, _| IsWindow(hw as HWND) != 0);
        }
        m.insert(w as usize, (pid, now, aumid.clone(), ic, fb));
    });
    (aumid, ic)
}

unsafe fn hide_stub(w: HWND) {
    let mut r: RECT = zeroed();
    if IsIconic(w) != 0 && GetWindowRect(w, &mut r) != 0 && r.left > -30000 {
        SetWindowPos(w, null_mut(), -32000, -32000, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn refresh() {
    refresh_windows();
    refresh_status();
}

fn refresh_windows() {
    refresh_buttons();
    BTNS.with_borrow(|b| b.iter().flat_map(|b| &b.wins).for_each(|&w| unsafe { hide_stub(w) }));
}

unsafe fn prune_tapps() {
    let mut changed = false;
    TAPPS.with_borrow_mut(|t| {
        let before = t.len();
        t.retain(|x| {
            let alive = IsWindow(x.hwnd) != 0;
            if !alive && !x.icon.is_null() {
                DestroyIcon(x.icon);
            }
            alive
        });
        changed = t.len() != before;
    });
    if changed {
        sync_trayfly();
    }
}

fn refresh_status() {
    let n = TICK.get();
    TICK.set(n + 1);
    unsafe {
        let mut p: SYSTEM_POWER_STATUS = zeroed();
        let ok = GetSystemPowerStatus(&mut p) != 0 && p.BatteryFlag & 128 == 0 && p.BatteryLifePercent <= 100;
        BAT.set(if ok { (p.BatteryLifePercent as i32, p.ACLineStatus == 1) } else { (-1, false) });
        read_volume();
        let up = !find_other("Shell_TrayWnd").is_null();
        if EXPLORER_UP.swap(up, Relaxed) && !up {
            place(BAR.load(Relaxed) as HWND);
            announce();
        }
        if n < 16 && n % 4 == 2 && TAPPS.with_borrow(|t| t.is_empty()) {
            announce();
        }
        if n % 3 == 0 {
            read_wifi();
        }
    }
    unsafe {
        prune_tapps();
        SetWindowPos(HOST.get(), HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

struct Layout {
    x0: i32,
    bw: i32,
    gap: i32,
    apps: (i32, i32),
    sys: (i32, i32),
    clock: (i32, i32),
}

fn slot_x(l: &Layout, i: i32) -> i32 {
    l.x0 + i * (l.bw + l.gap)
}

fn layout(w: i32) -> Layout {
    let small = small_bar();
    let gap = sc(if small { 2 } else { 4 });
    let clock = (w - sc(if small { 84 } else { 94 }), w - sc(if small { 6 } else { 8 }));
    let bat_w = if BAT.get().0 >= 0 { if small { 28 } else { 32 } } else { 0 };
    let sys_w = if small { 56 + bat_w } else { 64 + bat_w };
    let sys = (clock.0 - sc(4) - sc(sys_w), clock.0 - sc(4));
    let tray_step = sc(if small { 20 } else { 24 });
    let apps_r = sys.0 - sc(4);
    let apps = (apps_r - tray_step * tray_count() as i32, apps_r);
    let left_pad = if vert() { sc(GEAR_X.0 + GEAR_X.1 + 6) } else { sc(8) };
    let avail = (apps.0 - sc(8) - left_pad).max(sc(16));
    let n = BTNS.with_borrow(|b| b.len()) as i32;
    let bw = (((avail + gap) / n.max(1)) - gap).clamp(sc(16), sc(40 + [-10, 0, 8][i18n::style::bar_size() as usize % 3]));
    let total_w = n * (bw + gap) - gap;
    let right_limit = (apps.0 - sc(8) - total_w).max(left_pad);
    let centered = (w - total_w) / 2;
    let x0 = if i18n::style::align_left() { left_pad } else { centered }.min(right_limit).max(left_pad);
    Layout { x0, bw, gap, apps, sys, clock }
}

unsafe fn hit(hwnd: HWND, x: i32, y: i32) -> i32 {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let (x, l) = if vert() { (y, layout(r.bottom)) } else { (x, layout(r.right)) };
    if vert() && x < sc(GEAR_X.0 + GEAR_X.1) {
        return GEAR;
    }
    if x >= l.clock.1 {
        return SHOWDESK;
    }
    if x >= l.clock.0 {
        return CLOCK;
    }
    if x >= l.sys.0 {
        let d = x - l.sys.0;
        let small = small_bar();
        return if d < sc(if small { 28 } else { 32 }) {
            WIFI_I
        } else if d < sc(if small { 56 } else { 64 }) {
            VOL_I
        } else {
            BAT_I
        };
    }
    if x >= l.apps.0 && x < l.apps.1 {
        return TAPP + (x - l.apps.0) / sc(if small_bar() { 20 } else { 24 });
    }
    if x >= l.apps.1 {
        return NONE;
    }
    let n = BTNS.with_borrow(|b| b.len()) as i32;
    let d = x - l.x0;
    if d < 0 {
        return NONE;
    }
    let i = d / (l.bw + l.gap);
    if d % (l.bw + l.gap) >= l.bw || i >= n { NONE } else { i }
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        (x + (y - x) * t) as u32
    };
    ch(0) | ch(8) << 8 | ch(16) << 16
}

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

unsafe fn now_ms() -> u64 {
    GetTickCount64()
}

unsafe fn glow(dc: HDC, on: bool, r: RECT, to: u32, rad: i32) {
    if on {
        fill(dc, r, to, rad);
    }
}

fn bar_off() -> i32 {
    if BAR_P.get() < 0.5 { bar_h() + dock_gap() } else { 0 }
}

unsafe fn bar_step(hwnd: HWND) {
    let target = if FULLSCREEN.load(Relaxed) || (i18n::style::autohide() && DESK_HIDE.load(Relaxed)) { 0.0 } else { 1.0 };
    BAR_P.set(target);
    if target > 0.0 {
        if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let (px, py) = bar_pos();
        SetWindowPos(hwnd, HWND_TOPMOST, px, py, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
    } else {
        ShowWindow(hwnd, SW_HIDE);
    }
}

unsafe fn trayfly_open() -> bool {
    let w = TRAYFLY.get();
    !w.is_null() && IsWindowVisible(w) != 0
}

unsafe fn fullscreen_now(bar: HWND) -> bool {
    if fly::any_open() || trayfly_open() || now_ms() < REVEAL_UNTIL.get() {
        return false;
    }
    let fg = GetForegroundWindow();
    if fg.is_null() || fg == bar || IsWindowVisible(fg) == 0 || IsIconic(fg) != 0 {
        return false;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(fg, &mut pid);
    if pid == std::process::id() {
        return false;
    }
    let mut cls = [0u16; 64];
    let n = GetClassNameW(fg, cls.as_mut_ptr(), 64).max(0) as usize;
    if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "RustExplorer", "#32768"].iter().any(|s| eq_ascii(&cls[..n], s)) {
        return false;
    }
    let mut r: RECT = zeroed();
    GetWindowRect(fg, &mut r);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &mut mi);
    let m = mi.rcMonitor;
    let caption = GetWindowLongW(fg, GWL_STYLE) as u32 & WS_CAPTION == WS_CAPTION;
    (m.left, m.top) == (0, 0) && covers_monitor(r, m, caption, IsZoomed(fg) != 0)
}

fn covers_monitor(r: RECT, m: RECT, caption: bool, zoomed: bool) -> bool {
    if caption {
        return !zoomed && (r.left, r.top, r.right, r.bottom) == (m.left, m.top, m.right, m.bottom);
    }
    r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
}

unsafe fn desktop_now() -> bool {
    BTNS.with_borrow(|b| b.iter().flat_map(|b| &b.wins).all(|&w| IsIconic(w) != 0))
}

unsafe fn fs_update(hwnd: HWND) {
    if UI_GUARD.get() {
        return;
    }
    let fs = fullscreen_now(hwnd);
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    let now = now_ms();
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let over = if vert() {
        if i18n::style::side() == i18n::style::Side::Left { pt.x <= 0 || (pt.x < bar_h() && BAR_P.get() > 0.0) } else { pt.x >= sw - 1 || (pt.x >= sw - bar_h() && BAR_P.get() > 0.0) }
    } else if at_top() { pt.y <= 0 || (pt.y < bar_h() && BAR_P.get() > 0.0) } else { pt.y >= sh - 1 || (pt.y >= sh - bar_h() - dock_gap() && BAR_P.get() > 0.0) };
    if fs || !(i18n::style::autohide() || desktop_now() || dock_covered()) || fly::any_open() || trayfly_open() || over {
        DESK_T.set(now);
    }
    let desk = !fs && now.saturating_sub(DESK_T.get()) >= 2000;
    if DESK_HIDE.swap(desk, Relaxed) != desk {
        bar_step(hwnd);
    }
    if FULLSCREEN.swap(fs, Relaxed) != fs {
        if fs {
            fly::close();
            hide_tip(hwnd);
            thumbs::hide_preview();
        }
        bar_step(hwnd);
    }
    let poll = !fs && (i18n::style::autohide() || dock());
    if FS_POLL.replace(poll) != poll {
        if poll {
            SetTimer(hwnd, 3, 250, None);
        } else {
            KillTimer(hwnd, 3);
        }
    }
}

thread_local! {
    static FS_POLL: Cell<bool> = const { Cell::new(true) };
    static UI_GUARD: Cell<bool> = const { Cell::new(false) };
    static REVEAL_UNTIL: Cell<u64> = const { Cell::new(0) };
}

unsafe fn dock_covered() -> bool {
    if !dock() {
        return false;
    }
    let fg = GetForegroundWindow();
    if fg.is_null() || IsIconic(fg) != 0 || IsZoomed(fg) != 0 || fg == BAR.load(Relaxed) as HWND || fly::any_open() {
        return false;
    }
    let mut r: RECT = zeroed();
    GetWindowRect(fg, &mut r);
    let (x, w) = bar_x_w();
    let top = GetSystemMetrics(SM_CYSCREEN) - bar_h() - dock_gap();
    r.bottom > top && r.right > x && r.left < x + w
}

unsafe fn force_fg(hw: HWND) {
    winrust_common::fg::force(hw);
}

unsafe fn cached_gdi_brush(c: u32) -> HBRUSH {
    thread_local! {
        static GDI_BRUSHES: RefCell<HashMap<u32, isize>> = RefCell::new(HashMap::new());
    }
    GDI_BRUSHES.with_borrow_mut(|m| {
        if let Some(&b) = m.get(&c) {
            return b as HBRUSH;
        }
        if m.len() >= 64 {
            for &b in m.values() {
                DeleteObject(b as HBRUSH);
            }
            m.clear();
        }
        let b = CreateSolidBrush(c);
        m.insert(c, b as isize);
        b
    })
}

unsafe fn cached_gdip_brush(argb: u32) -> *mut GpSolidFill {
    thread_local! {
        static GDIP_BRUSHES: RefCell<HashMap<u32, isize>> = RefCell::new(HashMap::new());
    }
    GDIP_BRUSHES.with_borrow_mut(|m| {
        if let Some(&b) = m.get(&argb) {
            return b as *mut GpSolidFill;
        }
        if m.len() >= 64 {
            for &b in m.values() {
                GdipDeleteBrush(b as *mut GpBrush);
            }
            m.clear();
        }
        let mut br: *mut GpSolidFill = null_mut();
        GdipCreateSolidFill(argb, &mut br);
        if !br.is_null() {
            m.insert(argb, br as isize);
        }
        br
    })
}

#[inline]
fn argb(c: u32, a: u8) -> u32 {
    ((a as u32) << 24) | ((c & 255) << 16) | (c & 0xFF00) | ((c >> 16) & 255)
}

unsafe fn rounded_path(x: f32, y: f32, w: f32, h: f32, rad: f32) -> *mut GpPath {
    let d = rad.min(w).min(h);
    if d <= 0.0 || w <= 0.0 || h <= 0.0 {
        return null_mut();
    }
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    if !path.is_null() {
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
    }
    path
}

unsafe fn create_dib32(dc: HDC, w: i32, h: i32) -> (HBITMAP, *mut std::ffi::c_void) {
    let mut bi: BITMAPINFO = zeroed();
    bi.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..zeroed()
    };
    let mut bits: *mut std::ffi::c_void = null_mut();
    let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    (bmp, bits)
}

unsafe fn init_gp_surface(bits: *mut std::ffi::c_void, w: i32, h: i32) -> (*mut GpBitmap, *mut GpGraphics) {
    let mut gp_bmp: *mut GpBitmap = null_mut();
    let mut gp_g: *mut GpGraphics = null_mut();
    if !bits.is_null() && GdipCreateBitmapFromScan0(w, h, w * 4, 0x000E_200B, bits as *const u8, &mut gp_bmp) == 0 && !gp_bmp.is_null() {
        if GdipGetImageGraphicsContext(gp_bmp as *mut GpImage, &mut gp_g) == 0 && !gp_g.is_null() {
            GdipSetSmoothingMode(gp_g, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(gp_g, PixelOffsetModeHalf);
            GdipSetInterpolationMode(gp_g, InterpolationModeHighQualityBicubic);
            return (gp_bmp, gp_g);
        }
        GdipDisposeImage(gp_bmp as *mut GpImage);
    }
    (null_mut(), null_mut())
}

unsafe fn fill(dc: HDC, r: RECT, c: u32, rad: i32) {
    fill_argb(dc, r, c, 255, rad);
}

pub(crate) unsafe fn fill_argb(dc: HDC, r: RECT, c: u32, a: u8, rad: i32) {
    let rad = if style().square() { 0 } else { rad };
    let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let fg = FRAME_G.get();
    let owned = fg.is_null();
    let g = if !owned {
        fg
    } else if rad <= 0 && a == 255 {
        let b = cached_gdi_brush(c);
        FillRect(dc, &r, b);
        return;
    } else {
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
            let b = cached_gdi_brush(c);
            let (ob, op) = (SelectObject(dc, b), SelectObject(dc, GetStockObject(NULL_PEN)));
            RoundRect(dc, r.left, r.top, r.right + 1, r.bottom + 1, rad, rad);
            SelectObject(dc, ob);
            SelectObject(dc, op);
            return;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        g
    };
    let br = cached_gdip_brush(argb(c, a));
    if !br.is_null() {
        if rad <= 0 {
            GdipFillRectangle(g, br as *mut GpBrush, r.left as f32, r.top as f32, w, h);
        } else {
            let path = rounded_path(r.left as f32, r.top as f32, w, h, rad as f32);
            if !path.is_null() {
                GdipFillPath(g, br as *mut GpBrush, path);
                GdipDeletePath(path);
            }
        }
    }
    if owned {
        GdipDeleteGraphics(g);
    }
}

pub(crate) unsafe fn stroke_round(dc: HDC, r: RECT, c: u32, a: u8, rad: i32, width: f32) {
    let rad = if style().square() { 0 } else { rad };
    let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
    if w <= width || h <= width {
        return;
    }
    let fg = FRAME_G.get();
    let owned = fg.is_null();
    let g = if !owned {
        fg
    } else {
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
            return;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        g
    };
    let mut pen: *mut GpPen = null_mut();
    if GdipCreatePen1(argb(c, a), width, UnitPixel, &mut pen) == 0 && !pen.is_null() {
        let half = 0.5 * width;
        let (x, y, sw, sh) = (r.left as f32 + half, r.top as f32 + half, w - width, h - width);
        if rad <= 0 {
            GdipDrawRectangle(g, pen, x, y, sw, sh);
        } else {
            let path = rounded_path(x, y, sw, sh, (rad as f32 - width).max(1.0));
            if !path.is_null() {
                GdipDrawPath(g, pen, path);
                GdipDeletePath(path);
            }
        }
        GdipDeletePen(pen);
    }
    if owned {
        GdipDeleteGraphics(g);
    }
}

pub(crate) unsafe fn draw_icon(dc: HDC, x: i32, y: i32, sz: i32, ic: HICON) {
    if ic.is_null() || sz <= 0 {
        return;
    }
    let fg = FRAME_G.get();
    let owned = fg.is_null();
    let g = if !owned {
        fg
    } else {
        let mut gp: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(dc, &mut gp) != 0 || gp.is_null() {
            DrawIconEx(dc, x, y, ic, sz, sz, 0, null_mut(), DI_NORMAL);
            return;
        }
        GdipSetSmoothingMode(gp, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(gp, PixelOffsetModeHalf);
        GdipSetInterpolationMode(gp, InterpolationModeHighQualityBicubic);
        gp
    };
    let mut drawn = false;
    let mut ii: ICONINFO = zeroed();
    if GetIconInfo(ic, &mut ii) != 0 {
        if !ii.hbmColor.is_null() {
            let mut bm: BITMAP = zeroed();
            if GetObjectW(ii.hbmColor, size_of::<BITMAP>() as i32, &mut bm as *mut _ as *mut _) != 0
                && bm.bmBitsPixel == 32
                && bm.bmWidth > 0
                && bm.bmHeight > 0
            {
                let (bw, bh) = (bm.bmWidth, bm.bmHeight);
                let mut bi: BITMAPINFO = zeroed();
                bi.bmiHeader = BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: bw,
                    biHeight: -bh,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..zeroed()
                };
                thread_local! {
                    static ICON_PX: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
                }
                ICON_PX.with_borrow_mut(|px| {
                    let len = (bw * bh) as usize;
                    px.resize(len, 0);
                    if GetDIBits(dc, ii.hbmColor, 0, bh as u32, px.as_mut_ptr().cast(), &mut bi, DIB_RGB_COLORS) != 0 {
                        let has_alpha = px.iter().any(|&p| (p >> 24) != 0);
                        if has_alpha {
                            let straight = px.iter().any(|&p| {
                                let a = p >> 24;
                                ((p >> 16) & 255) > a || ((p >> 8) & 255) > a || (p & 255) > a
                            });
                            if straight {
                                for p in px.iter_mut() {
                                    let a = *p >> 24;
                                    if a == 0 {
                                        *p = 0;
                                    } else if a < 255 {
                                        let r = (((*p >> 16) & 255) * a + 127) / 255;
                                        let g_ = (((*p >> 8) & 255) * a + 127) / 255;
                                        let b = ((*p & 255) * a + 127) / 255;
                                        *p = (a << 24) | (r << 16) | (g_ << 8) | b;
                                    }
                                }
                            }
                            let mut bmp: *mut GpBitmap = null_mut();
                            if GdipCreateBitmapFromScan0(bw, bh, bw * 4, 0x000E_200B, px.as_ptr() as *const u8, &mut bmp) == 0 && !bmp.is_null() {
                                GdipDrawImageRectI(g, bmp as *mut GpImage, x, y, sz, sz);
                                GdipDisposeImage(bmp as *mut GpImage);
                                drawn = true;
                            }
                        }
                    }
                });
            }
            DeleteObject(ii.hbmColor);
        }
        if !ii.hbmMask.is_null() {
            DeleteObject(ii.hbmMask);
        }
    }
    if !drawn {
        let mut bmp: *mut GpBitmap = null_mut();
        if GdipCreateBitmapFromHICON(ic, &mut bmp) == 0 && !bmp.is_null() {
            GdipDrawImageRectI(g, bmp as *mut GpImage, x, y, sz, sz);
            GdipDisposeImage(bmp as *mut GpImage);
        } else {
            if owned {
                GdipDeleteGraphics(g);
            }
            DrawIconEx(dc, x, y, ic, sz, sz, 0, null_mut(), DI_NORMAL);
            return;
        }
    }
    if owned {
        GdipDeleteGraphics(g);
    }
}

pub(crate) unsafe fn text_box(dc: HDC, s: &str, mut r: RECT, c: u32, dt_flags: u32) {
    if s.is_empty() {
        return;
    }
    let (dst_bits, dst_w, dst_h) = FRAME_BITS.get();
    if !dst_bits.is_null() {
        let (rw, rh) = ((r.right - r.left).max(0), (r.bottom - r.top).max(0));
        if rw == 0 || rh == 0 {
            return;
        }
        thread_local! {
            static TEXT_BUF: std::cell::Cell<(HDC, HBITMAP, *mut std::ffi::c_void, i32, i32)> = const { std::cell::Cell::new((null_mut(), null_mut(), null_mut(), 0, 0)) };
        }
        let (mut mem, mut bmp, mut src_bits, bw, bh) = TEXT_BUF.get();
        if mem.is_null() {
            mem = CreateCompatibleDC(dc);
        }
        if bmp.is_null() || rw > bw || rh > bh {
            let nw = rw.max(bw).max(64);
            let nh = rh.max(bh).max(32);
            let (nb, nb_bits) = create_dib32(dc, nw, nh);
            let prev = SelectObject(mem, nb);
            if !bmp.is_null() {
                DeleteObject(prev);
            }
            bmp = nb;
            src_bits = nb_bits;
            TEXT_BUF.set((mem, bmp, src_bits, nw, nh));
        }
        let stride = TEXT_BUF.get().3;
        if !bmp.is_null() && !src_bits.is_null() {
            let src_mut = std::slice::from_raw_parts_mut(src_bits as *mut u32, (stride * rh) as usize);
            src_mut.fill(0);
            let font = GetCurrentObject(dc, OBJ_FONT as u32);
            let of = SelectObject(mem, font);
            SetBkMode(mem, TRANSPARENT as i32);
            SetTextColor(mem, rgb(255, 255, 255));
            let mut lr = RECT { left: 0, top: 0, right: rw, bottom: rh };
            with_wide(s, |ws| DrawTextW(mem, ws, -1, &mut lr, dt_flags));
            GdiFlush();
            let (cr, cg, cb) = ((c & 255) as u32, ((c >> 8) & 255) as u32, ((c >> 16) & 255) as u32);
            let src = std::slice::from_raw_parts(src_bits as *const u32, (stride * rh) as usize);
            let dst = std::slice::from_raw_parts_mut(dst_bits, (dst_w * dst_h) as usize);
            for yy in 0..rh {
                let dy = r.top + yy;
                if dy < 0 || dy >= dst_h { continue; }
                for xx in 0..rw {
                    let dx = r.left + xx;
                    if dx < 0 || dx >= dst_w { continue; }
                    let p = src[(yy * stride + xx) as usize];
                    let a = (((p & 255) * 29 + ((p >> 8) & 255) * 150 + ((p >> 16) & 255) * 77) >> 8).min(255);
                    if a == 0 { continue; }
                    let inv = 255 - a;
                    let idx = (dy * dst_w + dx) as usize;
                    let dp = dst[idx];
                    let (da, dr, dg, db) = ((dp >> 24) & 255, (dp >> 16) & 255, (dp >> 8) & 255, dp & 255);
                    let oa = a + (da * inv + 127) / 255;
                    let or_ = (cr * a + dr * inv + 127) / 255;
                    let og_ = (cg * a + dg * inv + 127) / 255;
                    let ob_ = (cb * a + db * inv + 127) / 255;
                    dst[idx] = (oa << 24) | (or_ << 16) | (og_ << 8) | ob_;
                }
            }
            SelectObject(mem, of);
        }
        return;
    }
    SetTextColor(dc, c);
    with_wide(s, |ws| DrawTextW(dc, ws, -1, &mut r, dt_flags));
}

unsafe fn text(dc: HDC, s: &str, r: RECT, c: u32, flags: u32) {
    text_box(dc, s, r, c, flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
}

const WINRUST_PROGRESS: [u16; 16] = [b'W' as u16, b'i' as u16, b'n' as u16, b'R' as u16, b'u' as u16, b's' as u16, b't' as u16, b'P' as u16, b'r' as u16, b'o' as u16, b'g' as u16, b'r' as u16, b'e' as u16, b's' as u16, b's' as u16, 0];
const WINRUST_BADGE: [u16; 13] = [b'W' as u16, b'i' as u16, b'n' as u16, b'R' as u16, b'u' as u16, b's' as u16, b't' as u16, b'B' as u16, b'a' as u16, b'd' as u16, b'g' as u16, b'e' as u16, 0];

fn parse_u16_badge(s: &[u16]) -> Option<u32> {
    let mut i = 0;
    while i < s.len() && s[i] == b' ' as u16 {
        i += 1;
    }
    let close = match s.get(i).copied() {
        Some(c) if c == b'(' as u16 => b')' as u16,
        Some(c) if c == b'[' as u16 => b']' as u16,
        _ => return None,
    };
    let rest = &s[i + 1..];
    let end = rest.iter().position(|&c| c == close)?;
    let mut val = 0u32;
    let mut any = false;
    for &c in &rest[..end] {
        if c == b' ' as u16 {
            continue;
        }
        if !(b'0' as u16..=b'9' as u16).contains(&c) {
            return None;
        }
        any = true;
        val = val.checked_mul(10)?.checked_add((c - b'0' as u16) as u32)?;
    }
    any.then_some(val)
}

unsafe fn win_progress(wins: &[HWND]) -> Option<u8> {
    for &w in wins {
        let v = GetPropW(w, WINRUST_PROGRESS.as_ptr()) as usize;
        if (1..=101).contains(&v) {
            return Some((v - 1) as u8);
        }
    }
    None
}

unsafe fn win_badge(wins: &[HWND]) -> u32 {
    let mut best = 0u32;
    let mut buf = [0u16; 64];
    for &w in wins {
        let v = GetPropW(w, WINRUST_BADGE.as_ptr()) as usize;
        if v > 0 {
            best = best.max(v as u32);
        }
        let n = GetWindowTextW(w, buf.as_mut_ptr(), 64).max(0) as usize;
        if let Some(num) = parse_u16_badge(&buf[..n]) {
            best = best.max(num);
        }
    }
    best
}

unsafe fn icon_of(h: HWND) -> HICON {
    let own = window_own_icon(h);
    if !own.is_null() {
        return own;
    }
    let aumid = win_aumid(h);
    if !aumid.is_empty() {
        let ic = cache_icon(format!("aumid:{aumid}"), || aumid_icon(&aumid, icon_px()));
        if !ic.is_null() {
            return ic;
        }
    }
    let exe = exe_of(h);
    if !exe.is_empty() && !is_host(&exe) {
        let ic = cache_icon(exe.clone(), || {
            let mut big: HICON = null_mut();
            ExtractIconExW(wide(&exe).as_ptr(), 0, &mut big, null_mut(), 1);
            big
        });
        if !ic.is_null() {
            return ic;
        }
    }
    null_mut()
}

unsafe fn app_window() -> HWND {
    let fg = GetForegroundWindow();
    let mut pid = 0u32;
    GetWindowThreadProcessId(fg, &mut pid);
    if !fg.is_null() && pid != std::process::id() {
        APP_FG.set(fg);
        return fg;
    }
    let last = APP_FG.get();
    if IsWindow(last) != 0 { last } else { fg }
}

thread_local! {
    static WIN_DIRTY: Cell<bool> = const { Cell::new(false) };
}

unsafe fn repaint_if_changed(hwnd: HWND) {
    let last = SIG.get();
    let s = view_sig();
    if s != last {
        SIG.set(s);
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe extern "system" fn win_event(_: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK, ev: u32, w: HWND, id: i32, child: i32, _: u32, _: u32) {
    if id != 0 || child != 0 || w.is_null() {
        return;
    }
    let bar = BAR.load(Relaxed) as HWND;
    if bar.is_null() {
        return;
    }
    let known = || BTNS.with_borrow(|b| b.iter().any(|b| b.wins.contains(&w)));
    let top = || GetAncestor(w, GA_PARENT) == GetDesktopWindow();
    let relevant = match ev {
        0x0003 => {
            fs_update(bar);
            true
        }
        0x8001 => known(),
        0x800C => !known() && top(),
        _ => top() || known(),
    };
    if relevant && !WIN_DIRTY.replace(true) {
        SetTimer(bar, 8, 30, None);
    }
}

unsafe fn view_sig() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    BTNS.with_borrow(|b| {
        for b in b {
            (b.pin, b.icon as usize, b.wins.len()).hash(&mut h);
            for &w in &b.wins {
                (w as usize, IsIconic(w)).hash(&mut h);
            }
        }
    });
    let mut t: SYSTEMTIME = zeroed();
    GetLocalTime(&mut t);
    let (mut a, mut b) = ([0u16; 40], [0u16; 40]);
    let n = GetTimeFormatW(0x0400, 0, &t, null(), a.as_mut_ptr(), 40).max(1) as usize - 1;
    let m = GetDateFormatW(0x0400, 1, &t, null(), b.as_mut_ptr(), 40).max(1) as usize - 1;
    (GetForegroundWindow() as usize, BAT.get(), VOL.get(), WIFI.get(), INET.get(), EXPLORER_UP.load(Relaxed), &a[..n], &b[..m]).hash(&mut h);
    TAPPS.with_borrow(|t| t.iter().for_each(|x| (x.hwnd as usize, x.uid, x.icon as usize, x.hidden, &x.tip).hash(&mut h)));
    h.finish()
}

unsafe fn clock_texts() -> (String, String) {
    let mut t: SYSTEMTIME = zeroed();
    GetLocalTime(&mut t);
    let (mut a, mut b) = ([0u16; 40], [0u16; 40]);
    let n = GetTimeFormatW(0x0400, 0, &t, null(), a.as_mut_ptr(), 40).max(1) as usize - 1;
    let m = GetDateFormatW(0x0400, 1, &t, null(), b.as_mut_ptr(), 40).max(1) as usize - 1;
    (String::from_utf16_lossy(&a[..n]), String::from_utf16_lossy(&b[..m]))
}

unsafe fn draw_win11_logo(dc: HDC, cx: i32, cy: i32, t: i32, g: i32) {
    use windows_sys::Win32::Graphics::GdiPlus::{GdipCreateFromHDC, GdipCreateLineBrushI, GdipDeleteGraphics, GdipFillRectangleI, GpLineGradient, Point, SmoothingModeAntiAlias, WrapModeTileFlipXY};
    let total = 2 * t + g;
    let (x0, y0) = (cx - total / 2, cy - total / 2);
    let fg = FRAME_G.get();
    let owned = fg.is_null();
    let mut gp: *mut GpGraphics = if !owned { fg } else { null_mut() };
    if owned && (GdipCreateFromHDC(dc, &mut gp) != 0 || gp.is_null()) {
        gp = null_mut();
    }
    if !gp.is_null() {
        GdipSetSmoothingMode(gp, SmoothingModeAntiAlias);
        let mut br: *mut GpLineGradient = null_mut();
        GdipCreateLineBrushI(&Point { X: x0, Y: y0 }, &Point { X: x0 + total, Y: y0 + total }, argb(rgb(112, 212, 255), 255), argb(rgb(0, 120, 212), 255), WrapModeTileFlipXY, &mut br);
        if !br.is_null() {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (px, py) = (x0 + dx * (t + g), y0 + dy * (t + g));
                GdipFillRectangleI(gp, br as *mut GpBrush, px, py, t, t);
            }
            GdipDeleteBrush(br as *mut GpBrush);
            if owned { GdipDeleteGraphics(gp); }
            return;
        }
        if owned { GdipDeleteGraphics(gp); }
    }
    let tint = [rgb(108, 206, 255), rgb(64, 182, 246), rgb(44, 166, 238), rgb(0, 120, 212)];
    for (k, (dx, dy)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
        let (px, py) = (x0 + dx * (t + g), y0 + dy * (t + g));
        fill(dc, RECT { left: px, top: py, right: px + t, bottom: py + t }, tint[k], 0);
    }
}

unsafe fn draw_battery(dc: HDC, x: i32, by: i32, bw: i32, bh: i32, pct: i32, ac: bool, bg: u32, rad_scale: i32) {
    let col = if ac { GOOD } else if pct <= 15 { BAD } else { pal().text };
    fill(dc, RECT { left: x, top: by, right: x + bw, bottom: by + bh }, col, sc(2 + rad_scale));
    fill(dc, RECT { left: x + sc(1), top: by + sc(1), right: x + bw - sc(1), bottom: by + bh - sc(1) }, bg, sc(1 + rad_scale));
    let fw = ((bw - sc(4)) * pct / 100).max(sc(1));
    fill(dc, RECT { left: x + sc(2), top: by + sc(2), right: x + sc(2) + fw, bottom: by + bh - sc(2) }, col, sc(rad_scale));
    fill(dc, RECT { left: x + bw + sc(1), top: by + sc(3), right: x + bw + sc(3), bottom: by + bh - sc(3) }, col, sc(1));
}

unsafe fn paint_vert(dc: HDC, cr: RECT, fonts: [HFONT; 4]) {
    let [_f_txt, _f_small, _f_glyph, _f_bold] = fonts;
    let l = layout(cr.bottom);
    let fg = GetForegroundWindow();
    let (lft, rgt) = (sc(6), cr.right - sc(6));
    let cx = (lft + rgt) / 2;
    let left = i18n::style::side() == i18n::style::Side::Left;
    let hot = HOT.get();
    let active_bg = mix(pal().active, pal().accent, 0.12);
    let cell = |y: i32, len: i32| RECT { left: lft, top: y, right: rgt, bottom: y + len };
    let dim = mix(pal().dim, pal().text, 0.5);

    let gr = cell(sc(GEAR_X.0), sc(GEAR_X.1));
    glow(dc, hot == GEAR, gr, pal().hover, sc(8));
    SelectObject(dc, fonts[2]);
    text(dc, "\u{E713}", gr, if hot == GEAR { pal().text } else { dim }, DT_CENTER);

    BTNS.with_borrow(|btns| {
        for (i, b) in btns.iter().enumerate() {
            let y = slot_x(&l, i as i32);
            let r = cell(y, l.bw);
            let mid = y + l.bw / 2;
            let act = b.wins.contains(&fg);
            let is_hot = hot == i as i32;
            if i != 0 && (is_hot || act) {
                fill(dc, r, if act { active_bg } else { pal().hover }, sc(8));
            }
            if i == 0 {
                let t = ((sc(8) as f32) * if is_hot { 1.12 } else { 1.0 }) as i32;
                let g = sc(1).max(1);
                let tint = [mix(pal().accent, pal().text, 0.35), mix(pal().accent, pal().text, 0.15), mix(pal().accent, pal().text, 0.15), pal().accent];
                for (k, (dx, dy)) in [(-1, -1), (0, -1), (-1, 0), (0, 0)].into_iter().enumerate() {
                    let (px, py) = (cx + if dx < 0 { -t - g } else { g }, mid + if dy < 0 { -t - g } else { g });
                    fill(dc, RECT { left: px, top: py, right: px + t, bottom: py + t }, tint[k], sc(3));
                }
                continue;
            }
            let ic = match b.pin {
                Some(p) => PINS.with_borrow(|pins| pins.get(p).map_or(null_mut(), |p| p.icon)),
                None => b.icon,
            };
            let ic = if ic.is_null() { b.wins.first().map_or(ic, |&w| icon_of(w)) } else { ic };
            let isz = icon_px().min(l.bw - sc(6));
            draw_icon(dc, cx - isz / 2, mid - isz / 2, isz, ic);
            if !b.wins.is_empty() {
                let min = b.wins.iter().all(|&w| IsIconic(w) != 0);
                let pl = sc(if act { 18 } else { 6 });
                let col = if act { pal().accent } else if min { mix(pal().dim, pal().bg, 0.45) } else { pal().dim };
                let px = if left { sc(1) } else { cr.right - sc(4) };
                fill(dc, RECT { left: px, top: mid - pl / 2, right: px + sc(3), bottom: mid + pl / 2 }, col, sc(3));
            }
        }
    });

    fill(dc, RECT { left: cx - sc(9), top: l.apps.0 - sc(5), right: cx + sc(9), bottom: l.apps.0 - sc(5) + 1 }, pal().line, 0);

    glow(dc, hot == TAPP, cell(l.apps.0, sc(24)), pal().hover, sc(8));
    let open_tf = IsWindowVisible(TRAYFLY.get()) != 0;
    let turn = if open_tf { 1.0 } else { 0.0 };
    chevron(dc, cx, l.apps.0 + sc(12), sc(5), if left { 90.0 } else { 270.0 } + 180.0 * turn, if open_tf { pal().accent } else { pal().text });
    SelectObject(dc, fonts[0]);
    let bar = bar_idx();
    TAPPS.with_borrow(|t| {
        for (k, &ix) in bar.iter().enumerate() {
            let (i, ti) = (k + 1, &t[ix]);
            let y = l.apps.0 + i as i32 * sc(24);
            glow(dc, hot == TAPP + i as i32, cell(y, sc(24)), pal().hover, sc(8));
            draw_icon(dc, cx - sc(8), y + sc(4), sc(16), ti.icon);
        }
    });

    let grp = cell(l.sys.0, l.sys.1 - l.sys.0);
    let sys_hot = matches!(hot, WIFI_I | VOL_I | BAT_I) || fly::is_open(fly::Kind::Quick);
    glow(dc, sys_hot, grp, pal().hover, sc(8));
    net_icon(dc, cx, l.sys.0 + sc(16), sc(17));
    let (vol, muted) = VOL.get();
    volume_glyph(dc, cx, l.sys.0 + sc(48), sc(16), vol, muted, if muted { BAD } else { pal().text });
    let (pct, ac) = BAT.get();
    if pct >= 0 {
        let (bw, bh) = (sc(22), sc(11));
        let (bx, by) = (cx - bw / 2 - sc(1), l.sys.0 + sc(69));
        draw_battery(dc, bx, by, bw, bh, pct, ac, if sys_hot { pal().hover } else { pal().bg }, 2);
        SelectObject(dc, fonts[1]);
        text(dc, &format!("{pct}%"), cell(l.sys.0 + sc(84), sc(18)), pal().text, DT_CENTER);
    }

    let cb = cell(l.clock.0, l.clock.1 - l.clock.0);
    glow(dc, hot == CLOCK || fly::is_open(fly::Kind::Cal), cb, pal().hover, sc(8));
    let (tm, dt) = clock_texts();
    let mid = (cb.top + cb.bottom) / 2;
    SelectObject(dc, fonts[3]);
    text(dc, &tm, RECT { left: 0, top: cb.top, right: cr.right, bottom: mid + sc(1) }, pal().text, DT_CENTER);
    SelectObject(dc, fonts[1]);
    text(dc, &dt, RECT { left: 0, top: mid - sc(1), right: cr.right, bottom: cb.bottom }, mix(pal().dim, pal().text, 0.35), DT_CENTER);
}

thread_local! {
    static BG: Cell<(HDC, HBITMAP, u64)> = const { Cell::new((null_mut(), null_mut(), 0)) };
    static BAR_BUF: Cell<(HDC, HBITMAP, *mut std::ffi::c_void, i32, i32, bool)> = const { Cell::new((null_mut(), null_mut(), null_mut(), 0, 0, false)) };
    static BAR_GP: Cell<(*mut GpBitmap, *mut GpGraphics, *mut std::ffi::c_void, i32, i32)> = const { Cell::new((null_mut(), null_mut(), null_mut(), 0, 0)) };
    static FONTS: Cell<([HFONT; 4], u64)> = const { Cell::new(([null_mut(); 4], 0)) };
    pub(crate) static FRAME_G: Cell<*mut GpGraphics> = const { Cell::new(null_mut()) };
    pub(crate) static FRAME_BITS: Cell<(*mut u32, i32, i32)> = const { Cell::new((null_mut(), 0, 0)) };
}

pub(crate) struct LayeredBuf {
    pub dc: HDC,
    pub bmp: HBITMAP,
    pub bits: *mut std::ffi::c_void,
    pub gp_bmp: *mut GpBitmap,
    pub gp_g: *mut GpGraphics,
    pub w: i32,
    pub h: i32,
}

impl LayeredBuf {
    pub const fn new() -> Self {
        Self { dc: null_mut(), bmp: null_mut(), bits: null_mut(), gp_bmp: null_mut(), gp_g: null_mut(), w: 0, h: 0 }
    }

    pub unsafe fn begin(&mut self, wdc: HDC, w: i32, h: i32) -> (HDC, *mut u32, *mut GpGraphics) {
        if self.dc.is_null() {
            self.dc = CreateCompatibleDC(wdc);
        }
        if self.bmp.is_null() || self.w != w || self.h != h {
            if !self.gp_g.is_null() {
                GdipDeleteGraphics(self.gp_g);
                self.gp_g = null_mut();
            }
            if !self.gp_bmp.is_null() {
                GdipDisposeImage(self.gp_bmp as *mut GpImage);
                self.gp_bmp = null_mut();
            }
            let (nb, new_bits) = create_dib32(wdc, w, h);
            let prev = SelectObject(self.dc, nb);
            if !self.bmp.is_null() {
                DeleteObject(prev);
            }
            self.bmp = nb;
            self.bits = new_bits;
            self.w = w;
            self.h = h;
            let (gp_bmp, gp_g) = init_gp_surface(self.bits, w, h);
            self.gp_bmp = gp_bmp;
            self.gp_g = gp_g;
        } else if !self.gp_g.is_null() {
            GdipResetClip(self.gp_g);
        }
        if !self.bits.is_null() {
            std::slice::from_raw_parts_mut(self.bits as *mut u32, (w * h) as usize).fill(0);
            FRAME_BITS.set((self.bits as *mut u32, w, h));
        } else {
            FRAME_BITS.set((null_mut(), 0, 0));
        }
        FRAME_G.set(self.gp_g);
        (self.dc, self.bits as *mut u32, self.gp_g)
    }

    pub unsafe fn present(hwnd: HWND, w: i32, h: i32, dc: HDC) {
        let fg = FRAME_G.replace(null_mut());
        if !fg.is_null() {
            GdipResetClip(fg);
        }
        FRAME_BITS.set((null_mut(), 0, 0));
        GdiFlush();
        let mut wr: RECT = zeroed();
        GetWindowRect(hwnd, &mut wr);
        let mut pt_dst = POINT { x: wr.left, y: wr.top };
        let mut sz = SIZE { cx: w, cy: h };
        let mut pt_src = POINT { x: 0, y: 0 };
        let mut bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        UpdateLayeredWindow(hwnd, null_mut(), &mut pt_dst, &mut sz, dc, &mut pt_src, 0, &mut bf, ULW_ALPHA);
    }
}

fn key_of(v: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish() | 1
}

unsafe fn bg_layer(wdc: HDC, cr: RECT) -> HDC {
    let h = cr.bottom;
    let p = pal();
    let key = key_of((cr.right, h, vert(), at_top(), i18n::style::side() == i18n::style::Side::Left, style().light(), std::mem::discriminant(&style()), [p.bg, p.text, p.line, p.accent], SCALE.get().to_bits()));
    let (mut dc, mut bmp, k) = BG.get();
    if k == key && !dc.is_null() {
        return dc;
    }
    if dc.is_null() {
        dc = CreateCompatibleDC(wdc);
    }
    let new_bmp = CreateCompatibleBitmap(wdc, cr.right, h);
    let prev = SelectObject(dc, new_bmp);
    if !bmp.is_null() {
        DeleteObject(prev);
    }
    bmp = new_bmp;
    BG.set((dc, bmp, key));
    fill(dc, cr, pal().bg, 0);
    winrust_common::style::backdrop(dc, cr, sc(100) as f32 / 100.0);
    if vert() {
        let x = if i18n::style::side() == i18n::style::Side::Left { cr.right - 1 } else { 0 };
        fill(dc, RECT { left: x, top: 0, right: x + 1, bottom: h }, pal().line, 0);
    }
    let edge = if at_top() { h - 1 } else { 0 };
    if !vert() {
        fill(dc, RECT { left: 0, top: edge, right: cr.right, bottom: edge + 1 }, pal().line, 0);
    }
    dc
}

unsafe fn bar_fonts() -> [HFONT; 4] {
    let small = small_bar();
    let key = key_of((SCALE.get().to_bits(), small, ui_face(), winrust_common::style::icon_face()));
    let (fonts, k) = FONTS.get();
    if k == key {
        return fonts;
    }
    fonts.iter().filter(|f| !f.is_null()).for_each(|&f| {
        DeleteObject(f);
    });
    let (face, gface) = (wide(ui_face()), wide(winrust_common::style::icon_face()));
    let font = |f: &[u16], px: i32, wt: i32| CreateFontW(-sc(px), 0, 0, 0, wt, 0, 0, 0, 1, 0, 0, 5, 0, f.as_ptr());
    let (t_px, s_px, g_px) = if small { (12, 11, 14) } else { (13, 12, 16) };
    let fonts = [font(&face, t_px, 400), font(&face, s_px, 400), font(&gface, g_px, 400), font(&face, t_px, 600)];
    FONTS.set((fonts, key));
    fonts
}

unsafe fn bar_backbuf(wdc: HDC, w: i32, h: i32, ulw: bool) -> (HDC, *mut std::ffi::c_void) {
    let (mut dc, mut bmp, mut bits, bw, bh, bulw) = BAR_BUF.get();
    if dc.is_null() {
        dc = CreateCompatibleDC(wdc);
    }
    if bmp.is_null() || bw != w || bh != h || bulw != ulw {
        let (new_bmp, new_bits) = if ulw {
            create_dib32(wdc, w, h)
        } else {
            (CreateCompatibleBitmap(wdc, w, h), null_mut())
        };
        let prev = SelectObject(dc, new_bmp);
        if !bmp.is_null() {
            DeleteObject(prev);
        }
        bmp = new_bmp;
        bits = new_bits;
        BAR_BUF.set((dc, bmp, bits, w, h, ulw));
    }
    (dc, bits)
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(hwnd, &mut ps);
    let mut cr: RECT = zeroed();
    GetClientRect(hwnd, &mut cr);
    let h = cr.bottom;
    if cr.right <= 0 || h <= 0 {
        EndPaint(hwnd, &ps);
        return;
    }
    let op = i18n::style::bar_opacity();
    let is_dock = dock();
    let ulw = op < 100 || is_dock;
    let (dc, bits) = bar_backbuf(wdc, cr.right, h, ulw);
    let bg_a = ((255 * op as u32) / 100).max(1) as u32;
    if ulw && !bits.is_null() {
        let buf = std::slice::from_raw_parts_mut(bits as *mut u32, (cr.right * h) as usize);
        if is_dock {
            buf.fill(0);
        } else {
            let c = pal().bg;
            let (r, g, b) = (((c & 255) * bg_a / 255), (((c >> 8) & 255) * bg_a / 255), (((c >> 16) & 255) * bg_a / 255));
            buf.fill((bg_a << 24) | (r << 16) | (g << 8) | b);
        }
        FRAME_BITS.set((bits as *mut u32, cr.right, h));
        let (mut gp_bmp, mut fg_g, gbits, gw, gh) = BAR_GP.get();
        if gp_bmp.is_null() || fg_g.is_null() || gbits != bits || gw != cr.right || gh != h {
            if !fg_g.is_null() {
                GdipDeleteGraphics(fg_g);
            }
            if !gp_bmp.is_null() {
                GdipDisposeImage(gp_bmp as *mut GpImage);
            }
            (gp_bmp, fg_g) = init_gp_surface(bits, cr.right, h);
            BAR_GP.set((gp_bmp, fg_g, bits, cr.right, h));
        } else {
            GdipResetClip(fg_g);
        }
        FRAME_G.set(fg_g);
        if is_dock {
            fill_argb(dc, cr, pal().bg, bg_a as u8, h);
            stroke_round(dc, cr, pal().line, bg_a as u8, h, 1.0);
        }
    } else {
        FRAME_BITS.set((null_mut(), 0, 0));
        BitBlt(dc, 0, 0, cr.right, h, bg_layer(wdc, cr), 0, 0, SRCCOPY);
    }
    SetBkMode(dc, TRANSPARENT as i32);
    let fonts = bar_fonts();
    let [f_txt, _f_small, _f_glyph, _f_bold] = fonts;
    let of = SelectObject(dc, f_txt);

    'horiz: {

    if vert() {
        paint_vert(dc, cr, fonts);
        break 'horiz;
    }
    let l = layout(cr.right);
    let fg = GetForegroundWindow();
    let small = small_bar();
    let (top, bot) = if small { (sc(2), h - sc(2)) } else { (sc(4), h - sc(4)) };
    let mid = (top + bot) / 2;
    let cell = |x: i32, w: i32| RECT { left: x, top, right: x + w, bottom: bot };
    let hot = HOT.get();

    BTNS.with_borrow(|btns| {
        for (i, b) in btns.iter().enumerate() {
            let x = slot_x(&l, i as i32);
            let r = RECT { left: x, top, right: x + l.bw, bottom: bot };
            let act = b.wins.contains(&fg);
            let ta = if act { 1.0 } else { 0.0 };
            if i == 0 {
                let t = sc(if small { 7 } else { 9 });
                let g = sc(1).max(1);
                draw_win11_logo(dc, (r.left + r.right) / 2, mid, t, g);
                continue;
            }
            let ic = match b.pin {
                Some(p) => PINS.with_borrow(|pins| pins.get(p).map_or(null_mut(), |p| p.icon)),
                None => b.icon,
            };
            let ic = if ic.is_null() { b.wins.first().map_or(ic, |&w| icon_of(w)) } else { ic };
            let isz = icon_px().min(l.bw - sc(6));
            draw_icon(dc, x + (l.bw - isz) / 2, mid - isz / 2 - sc(1), isz, ic);
            if let Some(pct) = win_progress(&b.wins) {
                let pb_l = x + sc(4);
                let pb_r = x + l.bw - sc(4);
                let pb_t = bot - sc(4);
                fill(dc, RECT { left: pb_l, top: pb_t, right: pb_r, bottom: bot - sc(1) }, mix(pal().bg, pal().text, 0.16), sc(3));
                let fw = ((pb_r - pb_l) * pct as i32 / 100).max(sc(4));
                fill(dc, RECT { left: pb_l, top: pb_t, right: (pb_l + fw).min(pb_r), bottom: bot - sc(1) }, GOOD, sc(3));
            } else if !b.wins.is_empty() {
                let min = b.wins.iter().all(|&w| IsIconic(w) != 0);
                let pw = sc(6) + (sc(10) as f32 * ta) as i32;
                let py = bot - sc(3);
                let col = if act { pal().accent } else if min { mix(pal().dim, pal().bg, 0.45) } else { pal().dim };
                fill(dc, RECT { left: x + (l.bw - pw) / 2, top: py, right: x + (l.bw + pw) / 2, bottom: py + sc(3) }, col, sc(3));
                if b.wins.len() > 1 {
                    fill(dc, RECT { left: x + (l.bw + pw) / 2 + sc(2), top: py, right: x + (l.bw + pw) / 2 + sc(5), bottom: py + sc(3) }, col, sc(3));
                }
            }
            let badge = win_badge(&b.wins);
            if badge > 0 {
                let btxt = if badge > 99 { "99+".to_string() } else { badge.to_string() };
                let bw_p = sc(if btxt.len() >= 3 { 20 } else if btxt.len() == 2 { 16 } else { 13 });
                let bh_p = sc(13);
                let br = RECT { left: (r.right - bw_p - sc(1)).max(r.left), top: r.top + sc(1), right: r.right - sc(1), bottom: r.top + sc(1) + bh_p };
                fill(dc, br, rgb(232, 17, 35), bh_p);
                SelectObject(dc, fonts[1]);
                text(dc, &btxt, br, rgb(255, 255, 255), DT_CENTER);
                SelectObject(dc, fonts[0]);
            }
        }
    });

    let tw = sc(if small { 20 } else { 24 });
    let t_ico = sc(if small { 14 } else { 16 });
    glow(dc, hot == TAPP, cell(l.apps.0, tw), pal().hover, rr(8));
    SelectObject(dc, fonts[2]);
    let open_tf = IsWindowVisible(TRAYFLY.get()) != 0;
    let turn = if open_tf { 1.0 } else { 0.0 };
    let cc = cell(l.apps.0, tw);
    chevron(dc, (cc.left + cc.right) / 2, mid, sc(if small { 5 } else { 6 }), if at_top() { 180.0 } else { 0.0 } + 180.0 * turn, if open_tf { pal().accent } else { pal().text });
    SelectObject(dc, fonts[0]);
    let bar = bar_idx();
    TAPPS.with_borrow(|t| {
        for (k, &ix) in bar.iter().enumerate() {
            let (i, ti) = (k + 1, &t[ix]);
            let x = l.apps.0 + i as i32 * tw;
            glow(dc, hot == TAPP + i as i32, cell(x, tw), pal().hover, rr(8));
            draw_icon(dc, x + (tw - t_ico) / 2, mid - t_ico / 2, t_ico, ti.icon);
        }
    });

    let grp = RECT { left: l.sys.0, top, right: l.sys.1, bottom: bot };
    let sys_hot = matches!(hot, WIFI_I | VOL_I | BAT_I) || fly::is_open(fly::Kind::Quick);
    glow(dc, sys_hot, grp, pal().hover, rr(8));
    let mut x = grp.left + sc(if small { 7 } else { 8 });
    net_icon(dc, x + sc(if small { 9 } else { 10 }), mid, sc(if small { 16 } else { 18 }));
    x += sc(if small { 25 } else { 28 });
    let (vol, muted) = VOL.get();
    let vw = sc(if small { 20 } else { 24 });
    volume_glyph(dc, x + vw / 2, mid, sc(if small { 16 } else { 18 }), vol, muted, if muted { BAD } else { pal().text });
    x += sc(if small { 26 } else { 30 });
    let (pct, ac) = BAT.get();
    if pct >= 0 {
        let (bw, bh) = (sc(if small { 18 } else { 20 }), sc(if small { 10 } else { 11 }));
        let by = mid - bh / 2;
        draw_battery(dc, x, by, bw, bh, pct, ac, if sys_hot { pal().hover } else { pal().bg }, 1);
    }

    let cb = cell(l.clock.0, l.clock.1 - l.clock.0);
    glow(dc, hot == CLOCK || fly::is_open(fly::Kind::Cal), cb, pal().hover, rr(8));
    let (tm, dt) = clock_texts();
    let ch = sc(if small { 13 } else { 15 });
    SelectObject(dc, fonts[1]);
    text(dc, &tm, RECT { left: cb.left + sc(4), top: mid - ch, right: cb.right - sc(6), bottom: mid }, pal().text, DT_RIGHT);
    text(dc, &dt, RECT { left: cb.left + sc(4), top: mid, right: cb.right - sc(6), bottom: mid + ch }, mix(pal().dim, pal().text, 0.7), DT_RIGHT);
    }

    if ulw && !bits.is_null() {
        EndPaint(hwnd, &ps);
        LayeredBuf::present(hwnd, cr.right, h, dc);
    } else {
        FRAME_G.set(null_mut());
        FRAME_BITS.set((null_mut(), 0, 0));
        BitBlt(wdc, 0, 0, cr.right, h, dc, 0, 0, SRCCOPY);
        EndPaint(hwnd, &ps);
    }
    SelectObject(dc, of);
}

unsafe fn keys(vks: &[u16]) {
    let send = |vk: u16, up: bool| {
        let mut i: INPUT = zeroed();
        i.r#type = INPUT_KEYBOARD;
        i.Anonymous.ki.wVk = vk;
        let ext = if matches!(vk, 0x25..=0x28 | 0x2C | 0x5B | 0x5C) { 0x0001 } else { 0 };
        i.Anonymous.ki.dwFlags = ext | if up { KEYEVENTF_KEYUP } else { 0 };
        SendInput(1, &i, size_of::<INPUT>() as i32);
    };
    vks.iter().for_each(|&v| send(v, false));
    vks.iter().rev().for_each(|&v| send(v, true));
}

thread_local! {
    static VDESKS: std::cell::RefCell<(usize, Vec<Vec<HWND>>)> = std::cell::RefCell::new((0, vec![vec![]]));
}

pub(crate) fn vdesk_info() -> (usize, usize) {
    VDESKS.with(|v| {
        let d = v.borrow();
        (d.0 + 1, d.1.len())
    })
}

unsafe fn vdesk_switch(desks: &mut Vec<Vec<HWND>>, cur: &mut usize, next: usize) {
    if *cur == next || next >= desks.len() {
        return;
    }
    let mut vis: Vec<HWND> = Vec::new();
    EnumWindows(Some(collect), &mut vis as *mut Vec<HWND> as LPARAM);
    let mut hidden = Vec::new();
    for w in vis {
        if IsWindow(w) != 0 {
            ShowWindow(w, SW_HIDE);
            if IsWindowVisible(w) == 0 {
                hidden.push(w);
            }
        }
    }
    desks[*cur] = hidden;
    *cur = next;
    let mut fg: HWND = null_mut();
    for w in desks[*cur].drain(..) {
        if IsWindow(w) != 0 {
            ShowWindow(w, 8);
            if fg.is_null() && IsIconic(w) == 0 {
                fg = w;
            }
        }
    }
    if !fg.is_null() {
        force_fg(fg);
    }
}

pub(crate) unsafe fn vdesk_action(act: u8) {
    if EXPLORER_UP.load(Relaxed) {
        match act {
            0 => keys(&[VK_CONTROL, VK_LWIN, VK_LEFT]),
            1 => keys(&[VK_CONTROL, VK_LWIN, b'D' as u16]),
            2 => keys(&[VK_CONTROL, VK_LWIN, VK_RIGHT]),
            _ => keys(&[VK_CONTROL, VK_LWIN, VK_F4]),
        }
        return;
    }
    VDESKS.with(|v| {
        let mut d = v.borrow_mut();
        let d = &mut *d;
        let (cur, desks) = (&mut d.0, &mut d.1);
        match act {
            0 if desks.len() > 1 => {
                let next = (*cur + desks.len() - 1) % desks.len();
                vdesk_switch(desks, cur, next);
            }
            1 => {
                let next = desks.len();
                desks.push(Vec::new());
                vdesk_switch(desks, cur, next);
            }
            2 if desks.len() > 1 => {
                let next = (*cur + 1) % desks.len();
                vdesk_switch(desks, cur, next);
            }
            3 if desks.len() > 1 => {
                let old = *cur;
                desks.remove(old);
                *cur = old.saturating_sub(1);
                let mut fg: HWND = null_mut();
                for w in desks[*cur].drain(..) {
                    if IsWindow(w) != 0 {
                        ShowWindow(w, 8);
                        if fg.is_null() && IsIconic(w) == 0 {
                            fg = w;
                        }
                    }
                }
                if !fg.is_null() {
                    force_fg(fg);
                }
            }
            _ => {}
        }
    });
    refresh_windows();
    let bar = BAR.load(Relaxed) as HWND;
    if !bar.is_null() {
        InvalidateRect(bar, null(), 0);
    }
}

unsafe fn vdesk_restore_all() {
    VDESKS.with(|v| {
        let mut d = v.borrow_mut();
        for list in &mut d.1 {
            for w in list.drain(..) {
                ShowWindow(w, 8);
            }
        }
        d.0 = 0;
        d.1.truncate(1);
    });
}

unsafe fn close_window(w: HWND) {
    if PostMessageW(w, WM_CLOSE, 0, 0) == 0 {
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, &mut pid);
        ShellExecuteW(null_mut(), wide("runas").as_ptr(), wide("taskkill.exe").as_ptr(), wide(&format!("/PID {pid}")).as_ptr(), null(), SW_HIDE);
    }
}

unsafe fn msix_app_id(lnk: &str) -> Option<String> {
    let (family, _) = msix_family(&lnk_target(&wide(lnk)))?;
    let prefix = format!("{family}!").to_lowercase();
    fly::apps_folder().into_iter().map(|a| a.1).find(|id| id.to_lowercase().starts_with(&prefix))
}

unsafe fn activate_app(aumid: &str) -> bool {
    use windows::Win32::UI::Shell::{ACTIVATEOPTIONS, ApplicationActivationManager, IApplicationActivationManager};
    CoCreateInstance::<_, IApplicationActivationManager>(&ApplicationActivationManager, None, CLSCTX_ALL)
        .and_then(|m| m.ActivateApplication(PCWSTR(wide(aumid).as_ptr()), PCWSTR::null(), ACTIVATEOPTIONS(0)))
        .is_ok()
}

unsafe fn open(path: &[u16]) {
    let n = path.iter().position(|&c| c == 0).unwrap_or(path.len());
    let p = String::from_utf16_lossy(&path[..n]);
    if p.to_lowercase().ends_with(".lnk") {
        bump(&p);
    }
    if p.to_lowercase().starts_with("ms-settings:") {
        return winrust_common::settings::open(&p);
    }
    let aumid = if p.to_lowercase().starts_with(r"shell:appsfolder\") { Some(p[r"shell:appsfolder\".len()..].to_string()) } else { msix_app_id(&p) };
    if aumid.is_some_and(|id| activate_app(&id)) {
        return;
    }
    if winrust_common::launch::is_program(&p) {
        winrust_common::launch::program(&p);
        return;
    }
    let rx = std::env::var_os("ProgramFiles").map(|pf| PathBuf::from(pf).join(r"rust-explorer\rust-explorer.exe")).filter(|x| x.is_file());
    let open_rx = |arg: Option<&str>| {
        let Some(ref exe) = rx else { return false };
        let mut cmd = Command::new(exe);
        if let Some(a) = arg {
            cmd.arg(a);
        }
        cmd.spawn().is_ok()
    };
    if p.eq_ignore_ascii_case("shell:MyComputerFolder") && open_rx(None) {
        return;
    }
    if std::path::Path::new(&p).is_dir() && open_rx(Some(&p)) {
        return;
    }
    if p.to_lowercase().ends_with(".lnk") {
        let t = lnk_target(path);
        if !t.is_empty() && std::path::Path::new(&t).is_dir() && open_rx(Some(&t)) {
            return;
        }
        if !t.is_empty() && winrust_common::launch::is_program(&t) && msix_family(&t).is_none() {
            let args = lnk_args(path);
            if winrust_common::launch::program_args(&t, &args) {
                return;
            }
        }
    }
    if Command::new("explorer.exe").arg(&p).spawn().is_err() {
        ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(&p).as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

fn wstr_of(w: &[u16]) -> String {
    String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
}

fn windows_count(n: usize) -> String {
    format!("{n} {}", if n == 1 { "window" } else { "windows" })
}

unsafe fn app_menu(hwnd: HWND, h: i32) {
    #[derive(Clone, Copy)]
    enum A {
        Win(HWND),
        Open,
        Pin,
        Left,
        Right,
        Start,
        Close,
        Kill,
    }
    let (wins, pin, bicon) = BTNS.with_borrow(|b| b.get(h as usize).map(|b| (b.wins.clone(), b.pin, b.icon)).unwrap_or((vec![], None, null_mut())));
    let exe = wins.first().map(|&w| exe_of(w)).unwrap_or_default();
    let (lnk, picon) = pin.and_then(|p| PINS.with_borrow(|p2| p2.get(p).map(|x| (wstr_of(&x.lnk), x.icon)))).unwrap_or_default();
    let launch = if lnk.is_empty() { exe.clone() } else { lnk.clone() };
    let stem = std::path::Path::new(&launch).file_stem().unwrap_or_default().to_string_lossy().to_string();
    let name: String = stem.chars().take(1).flat_map(char::to_uppercase).chain(stem.chars().skip(1)).collect();
    let sub = match (wins.len(), pin.is_some()) {
        (0, _) => "Pinned · not running".to_string(),
        (n, true) => format!("Pinned · {}", windows_count(n)),
        (n, false) => format!("Running · {}", windows_count(n)),
    };
    let icon = if picon.is_null() { bicon } else { picon };
    let mut items: Vec<(fly::MItem, A)> = Vec::new();
    if wins.len() > 1 {
        for &w in wins.iter().take(6) {
            let mut b = [0u16; 256];
            let n = GetWindowTextW(w, b.as_mut_ptr(), 256).max(0) as usize;
            items.push((fly::MItem { icon: icon_of(w), ..fly::MItem::new(String::from_utf16_lossy(&b[..n])) }, A::Win(w)));
        }
    }
    if !launch.is_empty() {
        let it = fly::MItem::new(if wins.is_empty() { "Open" } else { "New window" });
        items.push((if items.is_empty() { it } else { it.sep() }, A::Open));
    }
    if pin.is_some() {
        items.push((fly::MItem::new("Unpin from the bar"), A::Pin));
    } else if !exe.is_empty() {
        items.push((fly::MItem::new("Pin to the bar").glyph("\u{E718}"), A::Pin));
    }
    if let Some(p) = pin.filter(|_| !lnk.is_empty()) {
        let n = PINS.with_borrow(|p| p.len());
        if p > 0 {
            items.push((fly::MItem::new(if vert() { "Move up" } else { "Move left" }).glyph(if vert() { "\u{E74A}" } else { "\u{E72B}" }), A::Left));
        }
        if p + 1 < n {
            items.push((fly::MItem::new(if vert() { "Move down" } else { "Move right" }).glyph(if vert() { "\u{E74B}" } else { "\u{E72A}" }), A::Right));
        }
    }
    if !launch.is_empty() {
        let s = fly::start_pinned(&launch);
        items.push((fly::MItem::new(if s { "Remove from Start" } else { "Add to Start" }).glyph("\u{E8A9}"), A::Start));
    }
    if !wins.is_empty() {
        items.push((fly::MItem::new(if wins.len() > 1 { "Close all windows" } else { "Close window" }).glyph("\u{E8BB}").hint("Alt+F4").sep(), A::Close));
        items.push((fly::MItem::new("End process").glyph("\u{E711}").danger(), A::Kill));
    }
    let acts: Vec<A> = items.iter().map(|x| x.1).collect();
    let items = items.into_iter().map(|x| x.0).collect();
    let first_pid = wins.first().map_or(0, |&w| { let mut pid = 0; GetWindowThreadProcessId(w, &mut pid); pid });
    fly::menu(tip_cx(&bar_layout(hwnd), h), Some((icon, "", name, sub)), items, move |i| {
        match acts.get(i).copied() {
            Some(A::Win(w)) => {
                if IsIconic(w) != 0 {
                    ShowWindow(w, SW_RESTORE);
                }
                force_fg(w);
            }
            Some(A::Open) => open(&wide(&launch)),
            Some(A::Pin) => {
                match PINS.with_borrow(|p| p.iter().position(|x| !lnk.is_empty() && wstr_of(&x.lnk) == lnk)) {
                    Some(p) => unpin(p),
                    None if lnk.is_empty() => pin_exe(&exe, &proc_args(first_pid)),
                    None => {}
                }
                refresh_buttons();
            }
            Some(a @ (A::Left | A::Right)) => {
                if let Some(p) = PINS.with_borrow(|p| p.iter().position(|x| wstr_of(&x.lnk) == lnk)) {
                    let to = if matches!(a, A::Left) { p.wrapping_sub(1) } else { p + 1 };
                    if PINS.with_borrow_mut(|v| (to < v.len()).then(|| v.swap(p, to))).is_some() {
                        save_pin_order();
                        refresh_buttons();
                    }
                }
            }
            Some(A::Start) => fly::start_toggle(&launch),
            Some(A::Close) => wins.iter().for_each(|&w| close_window(w)),
            Some(A::Kill) => {
                let mut pids: Vec<u32> = wins
                    .iter()
                    .map(|&w| {
                        let mut pid = 0u32;
                        GetWindowThreadProcessId(w, &mut pid);
                        pid
                    })
                    .collect();
                pids.sort_unstable();
                pids.dedup();
                for pid in pids {
                    let p = OpenProcess(1, 0, pid);
                    if p.is_null() || TerminateProcess(p, 1) == 0 {
                        ShellExecuteW(null_mut(), wide("runas").as_ptr(), wide("taskkill.exe").as_ptr(), wide(&format!("/F /T /PID {pid}")).as_ptr(), null(), SW_HIDE);
                    }
                    if !p.is_null() {
                        CloseHandle(p);
                    }
                }
            }
            None => {}
        }
        InvalidateRect(BAR.load(Relaxed) as HWND, null(), 0);
    });
}

fn pin_dir() -> PathBuf {
    appdata_file(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar")
}

unsafe fn unpin(i: usize) {
    let Some(p) = PINS.with_borrow_mut(|p| (i < p.len()).then(|| p.remove(i))) else { return };
    let n = p.lnk.iter().position(|&c| c == 0).unwrap_or(p.lnk.len());
    let key = pin_key(&p.lnk);
    if !key.0.is_empty() {
        let mut t = std::fs::read_to_string(unpinned_file()).unwrap_or_default();
        t.push_str(&key_line(&key));
        t.push('\n');
        let _ = std::fs::create_dir_all(appdata_file("topbar-taskband"));
        let _ = std::fs::write(unpinned_file(), t);
    }
    let _ = std::fs::remove_file(String::from_utf16_lossy(&p.lnk[..n]));
    DestroyIcon(p.icon);
}

unsafe fn lnk_args(lnk: &[u16]) -> String {
    (|| -> Option<String> {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        sl.cast::<IPersistFile>().ok()?.Load(PCWSTR(lnk.as_ptr()), STGM(0)).ok()?;
        let mut b = [0u16; 2048];
        sl.GetArguments(&mut b).ok()?;
        Some(wstr_of(&b))
    })()
    .unwrap_or_default()
}

fn proc_args(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    let cl = winrust_common::proc::cmdline(pid).map(|c| c.trim().to_string()).unwrap_or_else(|| {
        let script = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8;(Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').CommandLine");
        let out = Command::new("powershell.exe").args(["-NoProfile", "-Command", &script]).creation_flags(NO_WINDOW).output();
        out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    });
    let rest = match cl.strip_prefix('"') {
        Some(r) => r.split_once('"').map_or("", |x| x.1),
        None => cl.split_once(' ').map_or("", |x| x.1),
    };
    rest.trim().to_string()
}

unsafe fn pin_exe(path: &str, args: &str) {
    let is_lnk = path.to_lowercase().ends_with(".lnk");
    let exe = if is_lnk { lnk_target(&wide(path)) } else { path.to_lowercase() };
    let args = if is_lnk { lnk_args(&wide(path)) } else { args.to_string() };
    let exe = exe.as_str();
    if exe.is_empty() || PINS.with_borrow(|p| p.iter().any(|p| p.target == exe && p.args == args)) {
        return;
    }
    let stem = std::path::Path::new(if is_lnk { path } else { exe }).file_stem().unwrap_or_default().to_string_lossy().to_string();
    let name: String = stem.chars().enumerate().map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c }).collect();
    let _ = std::fs::create_dir_all(pin_dir());
    let Some(file) = (1..).map(|n| pin_dir().join(if n == 1 { format!("{name}.lnk") } else { format!("{name} ({n}).lnk") })).find(|p| !p.exists()) else { return };
    let lnk = wide(&file.to_string_lossy());
    let ok = if is_lnk && std::fs::copy(path, &file).is_ok() {
        Some(())
    } else {
        (|| -> Option<()> {
            let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            sl.SetPath(PCWSTR(wide(exe).as_ptr())).ok()?;
            if !args.is_empty() {
                sl.SetArguments(PCWSTR(wide(&args).as_ptr())).ok()?;
            }
            sl.cast::<IPersistFile>().ok()?.Save(PCWSTR(lnk.as_ptr()), true).ok()
        })()
    };
    if ok.is_some() {
        let line = key_line(&(exe.to_string(), args.to_lowercase()));
        if let Ok(t) = std::fs::read_to_string(unpinned_file()) {
            let _ = std::fs::write(unpinned_file(), t.lines().filter(|l| *l != line).map(|l| format!("{l}\n")).collect::<String>());
        }
        let icon = lnk_icon(&lnk, exe);
        PINS.with_borrow_mut(|p| p.push(Pin { lnk, target: exe.to_string(), args, icon }));
    }
}

unsafe fn on_drop(hwnd: HWND, hd: HDROP) {
    let mut pt: POINT = zeroed();
    DragQueryPoint(hd, &mut pt);
    let n = DragQueryFileW(hd, u32::MAX, null_mut(), 0);
    let files: Vec<String> = (0..n)
        .map(|i| {
            let len = DragQueryFileW(hd, i, null_mut(), 0) as usize;
            let mut b = vec![0u16; len + 1];
            DragQueryFileW(hd, i, b.as_mut_ptr(), b.len() as u32);
            String::from_utf16_lossy(&b[..len])
        })
        .collect();
    DragFinish(hd);
    let h = hit(hwnd, pt.x, pt.y);
    if h == 0 {
        let added: Vec<&String> = files.iter().filter(|f| fly::start_pin(f)).collect();
        let name = |p: &str| std::path::Path::new(p).file_stem().unwrap_or_default().to_string_lossy().to_string();
        if !fly::is_open(fly::Kind::Start) {
            fly::toggle(fly::Kind::Start);
        }
        fly::note(&match added.len() {
            0 => "Already in Start".to_string(),
            1 => format!("Added to Start: {}", name(added[0])),
            n => format!("Added {n} items to Start"),
        });
    } else {
        files.iter().for_each(|f| pin_exe(f, ""));
        refresh_buttons();
    }
    InvalidateRect(hwnd, null(), 0);
}

pub(crate) fn purge_installed() -> bool {
    std::env::var_os("ProgramFiles").is_some_and(|pf| PathBuf::from(pf).join(r"purge\purge.exe").is_file())
}

unsafe fn open_purge() {
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        let exe = PathBuf::from(pf).join(r"purge\purge.exe");
        ShellExecuteW(null_mut(), wide("runas").as_ptr(), wide(&exe.to_string_lossy()).as_ptr(), wide("--ask").as_ptr(), null(), SW_SHOWNORMAL);
    }
}

pub(crate) unsafe fn open_taskmgr() {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let exe = dir
        .as_ref()
        .and_then(|d| ["rtaskmgr-win32.exe", "Task manager.exe"].into_iter().map(|n| d.join(n)).find(|p| p.is_file()))
        .or_else(|| std::env::var_os("ProgramFiles").map(|pf| PathBuf::from(pf).join(r"rtaskmgr\rtaskmgr-win32.exe")).filter(|p| p.is_file()));
    if let Some(p) = exe {
        open(&wide(&p.to_string_lossy()));
    } else {
        let sys = format!(r"{}\System32\taskmgr.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
        open(&wide(&sys));
    }
}

unsafe extern "system" fn find_launcher(h: HWND, lp: LPARAM) -> i32 {
    let mut cls = [0u16; 32];
    let n = GetClassNameW(h, cls.as_mut_ptr(), 32).max(0) as usize;
    if String::from_utf16_lossy(&cls[..n]) == "rs_launcher" {
        *(lp as *mut HWND) = h;
        return 0;
    }
    1
}

unsafe fn start_menu() {
    let bar = BAR.load(Relaxed) as HWND;
    let tools: [(&str, &str); 15] = [
        ("Apps and Features", "ms-settings:appsfeatures"),
        ("Power Options", "powercfg.cpl"),
        ("Event Viewer", "eventvwr.msc"),
        ("System", "ms-settings:about"),
        ("Device Manager", "devmgmt.msc"),
        ("Network Connections", "ms-settings:network"),
        ("Disk Management", "diskmgmt.msc"),
        ("Computer Management", "compmgmt.msc"),
        ("Terminal", "!term"),
        ("Task Manager", "!taskmgr"),
        ("Settings", "ms-settings:"),
        ("File Explorer", "shell:MyComputerFolder"),
        ("Run", "!run"),
        ("Desktop", "!desktop"),
        ("Task View", "!taskview"),
    ];
    let power = [("Lock", "!lock"), ("Sleep", "!sleep"), ("Restart", "!restart"), ("Shut down", "!shutdown")];
    let mut items = Vec::new();
    let mut targets = Vec::new();
    for (i, (en, to)) in tools.iter().chain(power.iter()).enumerate() {
        let it = fly::MItem::new(*en);
        items.push(if i == tools.len() { it.sep() } else { it });
        targets.push(*to);
    }
    let head = (null_mut(), "\u{E700}", "Start".to_string(), String::new());
    fly::menu(tip_cx(&bar_layout(bar), 0), Some(head), items, move |i| {
        let Some(&to) = targets.get(i) else { return };
        let quiet = |c: &mut Command| { let _ = c.creation_flags(NO_WINDOW).spawn(); };
        match to {
            "!term" => {
                let wt = std::env::var_os("LOCALAPPDATA").map(|b| PathBuf::from(b).join(r"Microsoft\WindowsApps\wt.exe")).filter(|p| p.is_file());
                open(&wide(if wt.is_some() { "wt.exe" } else { "powershell.exe" }));
            }
            "!run" => run_box(),
            "!taskmgr" => open_taskmgr(),
            "!desktop" => winrust_common::desktop::toggle(),
            "!taskview" => if EXPLORER_UP.load(Relaxed) { keys(&[VK_LWIN, VK_TAB]); } else { fly::toggle(fly::Kind::Desktops); },
            "!lock" => lock_workstation(),
            "!sleep" => fly::sleep_now(),
            "!restart" => quiet(Command::new("shutdown").args(["/r", "/t", "0"])),
            "!shutdown" => quiet(Command::new("shutdown").args(["/s", "/t", "0"])),
            _ => open(&wide(to)),
        }
    });
}

unsafe extern "system" fn snap_proc(_: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK, _: u32, w: HWND, id: i32, _: i32, _: u32, _: u32) {
    if GetAsyncKeyState(VK_LBUTTON as i32) >= 0 {
        return;
    }
    if id != 0 || w.is_null() || IsWindowVisible(w) == 0 || GetWindowLongW(w, GWL_STYLE) as u32 & WS_THICKFRAME == 0 || IsZoomed(w) != 0 {
        return;
    }
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(mon, &mut mi) == 0 {
        return;
    }
    let (m, a) = (mi.rcMonitor, mi.rcWork);
    let half = (a.right - a.left) / 2;
    if pt.y <= m.top + 1 {
        ShowWindow(w, SW_MAXIMIZE);
    } else if pt.x <= m.left + 1 {
        SetWindowPos(w, null_mut(), a.left, a.top, half, a.bottom - a.top, SWP_NOZORDER | SWP_NOACTIVATE);
    } else if pt.x >= m.right - 2 {
        SetWindowPos(w, null_mut(), a.left + half, a.top, half, a.bottom - a.top, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

unsafe fn win_num(n: usize) {
    let Some((wins, pin)) = BTNS.with_borrow(|b| b.get(n + 1).map(|b| (b.wins.clone(), b.pin))) else { return };
    if wins.is_empty() {
        if let Some(l) = pin.and_then(|p| PINS.with_borrow(|p2| p2.get(p).map(|x| x.lnk.clone()))) {
            open(&l);
        }
        return;
    }
    let i = wins.iter().position(|&w| w == GetForegroundWindow());
    let w = i.map_or(wins[0], |i| wins[(i + 1) % wins.len()]);
    if wins.len() == 1 && i.is_some() {
        ShowWindow(w, SW_MINIMIZE);
    } else {
        if IsIconic(w) != 0 {
            ShowWindow(w, SW_RESTORE);
        }
        force_fg(w);
    }
}

unsafe fn click(h: i32, left: bool, mid: bool) {
    match h {
        WIFI_I => fly::toggle(if left { fly::Kind::Quick } else { fly::Kind::Wifi }),
        VOL_I | BAT_I => fly::toggle(fly::Kind::Quick),
        CLOCK if left => fly::toggle(fly::Kind::Cal),
        CLOCK => {
            let items = vec![
                fly::MItem::new("Adjust date and time").glyph("\u{E823}"),
                fly::MItem::new("Notification settings").glyph("\u{EA8F}"),
            ];
            fly::menu(tip_cx(&bar_layout(BAR.load(Relaxed) as HWND), CLOCK), None, items, |i| {
                if i == 0 {
                    winrust_common::settings::open("ms-settings:dateandtime");
                } else {
                    open(&wide("ms-settings:notifications"));
                }
            });
        }
        SHOWDESK if left => winrust_common::desktop::toggle(),
        GEAR => style_menu(),
        TAPP..=-900 => {
            let slot = (h - TAPP) as usize;
            if slot == 0 {
                if left {
                    fly_toggle();
                }
            } else if mid {
                if let Some(k) = bar_idx().get(slot - 1).and_then(|&i| TAPPS.with_borrow(|t| t.get(i).map(|x| x.key.clone()))) {
                    toggle_shown(&k);
                }
            } else {
                let ti = bar_idx().get(slot - 1).and_then(|&i| TAPPS.with_borrow(|t| t.get(i).map(|x| (x.hwnd, x.uid, x.cb, x.ver))));
                REVEAL_UNTIL.set(now_ms() + 600);
                ti.into_iter().for_each(|ti| tray_click(ti, left));
            }
        }
        0 if left => fly::toggle(fly::Kind::Start),
        0 => start_menu(),
        1.. => {
            let (wins, lnk) = BTNS.with_borrow(|b| b.get(h as usize).map(|b| (b.wins.clone(), b.pin.and_then(|p| PINS.with_borrow(|p2| p2.get(p).map(|x| x.lnk.clone()))))).unwrap_or_default());
            match (wins.first(), left) {
                (None, true) => lnk.iter().for_each(|l| {
                    PINS.with_borrow_mut(|p| p.iter_mut().filter(|x| x.lnk == *l).for_each(|x| x.target = fix_msix_pin(l, &x.target).unwrap_or(std::mem::take(&mut x.target))));
                    open(l)
                }),
                (Some(&w), false) => close_window(w),
                (Some(_), true) if wins.len() > 1 => {
                    let fg = GetForegroundWindow();
                    let next = match wins.iter().position(|&w| w == fg) {
                        Some(i) => wins[(i + 1) % wins.len()],
                        None => wins[0],
                    };
                    if IsIconic(next) != 0 {
                        ShowWindow(next, SW_RESTORE);
                    }
                    force_fg(next);
                }
                (Some(&w), true) => {
                    let fg = GetForegroundWindow();
                    if wins.contains(&fg) && IsIconic(fg) == 0 {
                        ShowWindow(fg, SW_MINIMIZE);
                    } else {
                        if IsIconic(w) != 0 {
                            ShowWindow(w, SW_RESTORE);
                        }
                        force_fg(w);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

#[repr(C)]
struct ShellHookInfo {
    hwnd: HWND,
    rc: RECT,
}

unsafe fn min_rect(info: *mut ShellHookInfo, bar: HWND) {
    let (w, l) = ((*info).hwnd, bar_layout(bar));
    let exe = exe_of(w);
    let i = BTNS.with_borrow(|b| b.iter().position(|b| b.wins.contains(&w)).or_else(|| b.iter().position(|b| !exe.is_empty() && b.wins.first().is_some_and(|&x| exe_of(x) == exe))));
    let x = match i {
        Some(i) => slot_x(&l, i as i32),
        None => (l.x0 + l.apps.0) / 2,
    };
    if vert() {
        let left = bar_x() + sc(4);
        (*info).rc = RECT { left, top: x, right: left + bar_h() - sc(8), bottom: x + l.bw };
        return;
    }
    let (top, x) = (bar_top() + sc(4), x + bar_x_w().0);
    (*info).rc = RECT { left: x, top, right: x + l.bw, bottom: top + bar_h() - sc(8) };
}

unsafe fn tip_cx(l: &Layout, h: i32) -> i32 {
    (if vert() { 0 } else { bar_x_w().0 }) + match h {
        GEAR => if vert() { sc(GEAR_X.0 + GEAR_X.1 / 2) } else { sc(120) },
        CLOCK => (l.clock.0 + l.clock.1) / 2,
        SHOWDESK => l.clock.1 + sc(3),
        WIFI_I => l.sys.0 + sc(18),
        VOL_I => l.sys.0 + sc(46),
        BAT_I => l.sys.0 + sc(76),
        t if (TAPP..TAPP + 100).contains(&t) => l.apps.0 + (t - TAPP) * sc(24) + sc(12),
        i => slot_x(l, i) + l.bw / 2,
    }
}

unsafe fn bar_layout(hwnd: HWND) -> Layout {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    layout(if vert() { r.bottom } else { r.right })
}

unsafe fn appbar_new(hwnd: HWND) {
    let mut ab: APPBARDATA = zeroed();
    ab.cbSize = size_of::<APPBARDATA>() as u32;
    ab.hWnd = hwnd;
    ab.uCallbackMessage = CALLBACK;
    SHAppBarMessage(ABM_NEW, &mut ab);
    EXP_TRAY.set(find_other("Shell_TrayWnd"));
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg != 0 && msg == SH_MSG.get() {
        if wp == 5 && lp != 0 {
            min_rect(lp as *mut ShellHookInfo, hwnd);
            return 1;
        }
        return 0;
    }
    if msg != 0 && msg == TB_MSG.get() {
        let t = find_other("Shell_TrayWnd");
        if !t.is_null() && t != EXP_TRAY.get() {
            appbar_new(hwnd);
            hide_taskbar();
            place(hwnd);
        }
        return 0;
    }
    match msg {
        WM_PAINT => paint(hwnd),
        0x02E0 => {
            let dpi = ((wp >> 16) & 0xFFFF) as f32;
            if dpi > 0.0 {
                SCALE.set(dpi / 96.0);
                place(hwnd);
                InvalidateRect(hwnd, null(), 0);
            }
        }
        WM_ERASEBKGND => return 1,
        WM_HOTKEY if wp == 99 => {
            fly::toggle(fly::Kind::Widgets);
        }
        WM_TIMER if wp == 2 => {
            KillTimer(hwnd, 2);
            let h = HOT.get();
            if h != NONE && !fly::any_open() {
                let cx = tip_cx(&bar_layout(hwnd), h);
                if !thumbs::show_preview(h, cx) {
                    thumbs::hide_preview();
                    show_tip(tip_text(h), cx);
                }
            }
        }
        WM_TIMER if wp == 6 => hide_tip(hwnd),
        WM_TIMER if wp == 3 => fs_update(hwnd),
        WM_TIMER if wp == 7 => {
            KillTimer(hwnd, 7);
            let (tid, before, next) = LANG_TRY.get();
            if GetKeyboardLayout(tid) as usize == before && !EXPLORER_UP.load(Relaxed) {
                PostMessageW(app_window(), WM_INPUTLANGCHANGEREQUEST, 0, next as isize);
            } else if GetKeyboardLayout(tid) as usize == before {
                let w = app_window();
                if GetForegroundWindow() != w {
                    force_fg(w);
                }
                keys(&[VK_LWIN, VK_SPACE]);
            }
            InvalidateRect(hwnd, null(), 0);
        }
        WIN_START => {
            UI_GUARD.set(true);
            fly::toggle(fly::Kind::Start);
            UI_GUARD.set(false);
            fs_update(hwnd);
        }
        ALT_TAB => alt_tab(wp == 1),
        WIN_NUM => win_num(wp),
        WIN_X => {
            UI_GUARD.set(true);
            start_menu();
            UI_GUARD.set(false);
            fs_update(hwnd);
        }
        WIN_TAB => {
            UI_GUARD.set(true);
            if wp == 0 { fly::toggle(fly::Kind::Desktops); } else { vdesk_action((wp - 1) as u8); }
            UI_GUARD.set(false);
            fs_update(hwnd);
        }
        NEXT_LANG => next_lang(hwnd, wp == 1),
        ALT_UP if wp == 2 => alt_close(),
        ALT_UP => alt_done(wp == 1),
        SNIP if wp == 1 => screenshot_notice(hwnd),
        SNIP => clip::start(),
        VOLKEY => {
            if wp == 2 { toggle_mute() } else { add_volume(if wp == 0 { 4 } else { -4 }) }
            KillTimer(hwnd, 2);
            show_tip(vol_tip(), tip_cx(&bar_layout(hwnd), VOL_I));
            SetTimer(hwnd, 6, 1200, None);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_DROPFILES => on_drop(hwnd, wp as HDROP),
        WM_TIMER if wp == 8 => {
            KillTimer(hwnd, 8);
            WIN_DIRTY.set(false);
            hide_taskbar_if_shown();
            prune_tapps();
            refresh_windows();
            repaint_if_changed(hwnd);
        }
        WM_TIMER => {
            let was_exp = EXPLORER_UP.load(Relaxed);
            let tb = find_other("Shell_TrayWnd");
            if !tb.is_null() && IsWindowVisible(tb) != 0 {
                hide_taskbar();
            }
            refresh_status();
            let now_exp = EXPLORER_UP.load(Relaxed);
            if was_exp && !now_exp {
                let host = HOST.get();
                if !host.is_null() {
                    SetWindowPos(host, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                announce();
            }
            fs_update(hwnd);
            repaint_if_changed(hwnd);
        }
        WM_MOUSEMOVE => {
            let h = hit(hwnd, (lp & 0xffff) as i16 as i32, (lp >> 16) as i16 as i32);
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            if HOT.replace(h) != h {
                hide_tip(hwnd);
                if h != NONE {
                    SetTimer(hwnd, 2, 450, None);
                }
                InvalidateRect(hwnd, null(), 0);
            }
        }
        WM_MOUSELEAVE => {
            hide_tip(hwnd);
            HOT.set(-1);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_MOUSEWHEEL => {
            add_volume(if (wp >> 16) as i16 > 0 { 2 } else { -2 });
            KillTimer(hwnd, 2);
            if !fly::any_open() {
                show_tip(vol_tip(), tip_cx(&bar_layout(hwnd), VOL_I));
                SetTimer(hwnd, 6, 1200, None);
            }
            InvalidateRect(hwnd, null(), 0);
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            let x = (lp & 0xffff) as i16 as i32;
            let h = hit(hwnd, x, (lp >> 16) as i16 as i32);
            let x = if vert() { (lp >> 16) as i16 as i32 } else { x };
            hide_tip(hwnd);
            thumbs::hide_preview();
            UI_GUARD.set(true);
            if h == NONE && msg == WM_RBUTTONDOWN {
                fly::context_menu(x);
            } else if h >= 1 && msg == WM_RBUTTONDOWN {
                app_menu(hwnd, h);
            } else if !(msg == WM_RBUTTONDOWN && (TAPP..=-900).contains(&h)) {
                click(h, msg == WM_LBUTTONDOWN, msg == WM_MBUTTONDOWN);
            }
            UI_GUARD.set(false);
            fs_update(hwnd);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_RBUTTONUP => {
            let h = hit(hwnd, (lp & 0xffff) as i16 as i32, (lp >> 16) as i16 as i32);
            if (TAPP..=-900).contains(&h) {
                UI_GUARD.set(true);
                click(h, false, false);
                UI_GUARD.set(false);
                fs_update(hwnd);
                InvalidateRect(hwnd, null(), 0);
            }
        }
        CALLBACK => {
            if wp == ABN_FULLSCREENAPP as usize {
                fs_update(hwnd);
            } else if wp == ABN_POSCHANGED as usize {
                place(hwnd);
            }
        }
        WM_DISPLAYCHANGE => place(hwnd),
        WM_DESTROY => {
            vdesk_restore_all();
            let mut ab: APPBARDATA = zeroed();
            ab.cbSize = size_of::<APPBARDATA>() as u32;
            ab.hWnd = hwnd;
            SHAppBarMessage(ABM_REMOVE, &mut ab);
            set_work_area(RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) });
            show_taskbar(OLD_STATE.get());
            forget_taskbar_state();
            DestroyWindow(HOST.replace(null_mut()));
            announce();
            PostQuitMessage(0);
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

pub(crate) unsafe fn lock_workstation() {
    let u32_mod = GetModuleHandleW(wide("user32.dll").as_ptr());
    if !u32_mod.is_null() {
        if let Some(p) = GetProcAddress(u32_mod, b"LockWorkStation\0".as_ptr()) {
            std::mem::transmute::<_, unsafe extern "system" fn() -> i32>(p)();
        }
    }
}

unsafe fn enable_dpi_aware() {
    let u32_mod = GetModuleHandleW(wide("user32.dll").as_ptr());
    if !u32_mod.is_null() {
        if let Some(p) = GetProcAddress(u32_mod, b"SetProcessDpiAwarenessContext\0".as_ptr()) {
            let f: unsafe extern "system" fn(isize) -> i32 = std::mem::transmute(p);
            if f(-4) != 0 {
                return;
            }
        }
    }
    SetProcessDPIAware();
}

fn main() {
    winrust_common::install_panic_hook("topbar");
    unsafe {
        if std::env::args().any(|a| a == "--restore") {
            show_taskbar(saved_taskbar_state().unwrap_or(ABS_ALWAYSONTOP as usize));
            forget_taskbar_state();
            set_work_area(RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) });
            announce();
            return;
        }
        if std::env::args().any(|a| a == "--shot") {
            enable_dpi_aware();
        winrust_common::ui::ensure_fluent_icons();
            let (mut tok, mut inp): (usize, GdiplusStartupInput) = (0, zeroed());
            inp.GdiplusVersion = 1;
            GdiplusStartup(&mut tok, &inp, null_mut());
            screenshot();
            return;
        }
        if let Some(p) = std::env::args().skip_while(|a| a != "--preview").nth(1) {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            winrust_common::ui::ensure_fluent_icons();
            enable_dpi_aware();
            let dc = GetDC(null_mut());
            SCALE.set(GetDeviceCaps(dc, LOGPIXELSX as i32) as f32 / 96.0);
            ReleaseDC(null_mut(), dc);
            let (mut tok, mut inp): (usize, GdiplusStartupInput) = (0, zeroed());
            inp.GdiplusVersion = 1;
            GdiplusStartup(&mut tok, &inp, null_mut());
            fly::init(GetModuleHandleW(null()));
            fly::PREVIEW.store(true, Relaxed);
            fly::toggle(if p == "wifi" { fly::Kind::Wifi } else { fly::Kind::Quick });
            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            return;
        }
        let m = windows_sys::Win32::System::Threading::CreateMutexW(null(), 0, wide("Local\\topbar-single").as_ptr());
        if m.is_null() || GetLastError() == ERROR_ALREADY_EXISTS || !FindWindowW(wide("topbar").as_ptr(), null()).is_null() {
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let mut mm: MINIMIZEDMETRICS = zeroed();
        mm.cbSize = size_of::<MINIMIZEDMETRICS>() as u32;
        if SystemParametersInfoW(SPI_GETMINIMIZEDMETRICS, mm.cbSize, &mut mm as *mut _ as *mut _, 0) != 0 {
            mm.iArrange |= ARW_HIDE as i32;
            SystemParametersInfoW(SPI_SETMINIMIZEDMETRICS, mm.cbSize, &mut mm as *mut _ as *mut _, 0);
        }
        let (mut gtok, mut gin): (usize, GdiplusStartupInput) = (0, zeroed());
        gin.GdiplusVersion = 1;
        GdiplusStartup(&mut gtok, &gin, null_mut());
        TB_MSG.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
        SH_MSG.set(RegisterWindowMessageW(wide("SHELLHOOK").as_ptr()));
        enable_dpi_aware();
        winrust_common::ui::ensure_fluent_icons();
        let dc = GetDC(null_mut());
        SCALE.set(GetDeviceCaps(dc, LOGPIXELSX as i32) as f32 / 96.0);
        ReleaseDC(null_mut(), dc);
        SHOWN.set(std::fs::read_to_string(appdata_file("topbar-tray.txt")).ok().map(|s| s.lines().map(str::to_string).collect()));
        load_pins();
        fly::init(GetModuleHandleW(null()));
        thread::spawn(fly::load_apps);
        thread::spawn(winrust_common::mem::auto_loop);
        let hi = GetModuleHandleW(null());
        let class = wide("topbar");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: class.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED, class.as_ptr(), class.as_ptr(), WS_POPUP, 0, 0, GetSystemMetrics(SM_CXSCREEN), bar_h(), null_mut(), null_mut(), hi, null());
        winrust_common::wait_shell_ready();
        let tc = wide("Shell_TrayWnd");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(tray_proc), hInstance: hi, lpszClassName: tc.as_ptr(), ..zeroed() });
        HOST.set(CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE, tc.as_ptr(), null(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), hi, null()));
        SetWindowPos(HOST.get(), HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let tnc = wide("TrayNotifyWnd");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(DefWindowProcW), hInstance: hi, lpszClassName: tnc.as_ptr(), ..zeroed() });
        CreateWindowExW(0, tnc.as_ptr(), null(), WS_CHILD, 0, 0, 0, 0, HOST.get(), null_mut(), hi, null());
        ChangeWindowMessageFilterEx(HOST.get(), WM_COPYDATA, MSGFLT_ALLOW, null_mut());
        ChangeWindowMessageFilterEx(HOST.get(), 0x49 , MSGFLT_ALLOW, null_mut());
        DragAcceptFiles(hwnd, 1);
        for m in [CALLBACK, TB_MSG.get(), WM_DROPFILES, 0x49, WM_COPYDATA] {
            ChangeWindowMessageFilterEx(hwnd, m, MSGFLT_ALLOW, null_mut());
        }
        let fc = wide("topbar_trayfly");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(fly_proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: fc.as_ptr(), ..zeroed() });
        TRAYFLY.set(CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED, fc.as_ptr(), null(), WS_POPUP, 0, 0, 1, 1, null_mut(), null_mut(), hi, null()));
        let tipc = wide("topbar_tip");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(tip_proc), hInstance: hi, lpszClassName: tipc.as_ptr(), ..zeroed() });
        TIPWND.set(CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED, tipc.as_ptr(), null(), WS_POPUP, 0, 0, 1, 1, null_mut(), null_mut(), hi, null()));
        let swc = wide("topbar_alttab");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(sw_proc), hInstance: hi, lpszClassName: swc.as_ptr(), ..zeroed() });
        SWWND.set(CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED, swc.as_ptr(), null(), WS_POPUP, 0, 0, 1, 1, null_mut(), null_mut(), hi, null()));
        let pvc = wide("topbar_preview");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(thumbs::pv_proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: pvc.as_ptr(), ..zeroed() });
        thumbs::PVWND.set(CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED, pvc.as_ptr(), null(), WS_POPUP, 0, 0, 1, 1, null_mut(), null_mut(), hi, null()));
        OLD_STATE.set(remember_taskbar_state());
        appbar_new(hwnd);
        if find_other("Shell_TrayWnd").is_null() {
            if let Some(f) = GetProcAddress(GetModuleHandleW(wide("user32.dll").as_ptr()), b"SetTaskmanWindow\0".as_ptr()) {
                std::mem::transmute::<_, unsafe extern "system" fn(HWND) -> i32>(f)(hwnd);
            }
            RegisterShellHookWindow(hwnd);
        }
        hide_taskbar();
        place(hwnd);
        refresh();
        import_from_explorer();
        announce();
        SetTimer(hwnd, 1, 1000, None);
        BAR.store(hwnd as isize, Relaxed);
        RegisterHotKey(hwnd, 99, (MOD_WIN | MOD_NOREPEAT) as u32, 0x57);
        bar_step(hwnd);
        fs_update(hwnd);
        windows_sys::Win32::UI::Accessibility::SetWinEventHook(0x000B, 0x000B, null_mut(), Some(snap_proc), 0, 0, 0);
        for (a, b) in [(0x0003, 0x0003), (0x0016, 0x0017), (0x8000, 0x8003), (0x800C, 0x800C), (0x8017, 0x8018)] {
            windows_sys::Win32::UI::Accessibility::SetWinEventHook(a, b, null_mut(), Some(win_event), 0, 0, 2);
        }
        thread::spawn(|| {
            let mut hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), GetModuleHandleW(null()), 0);
            SetTimer(null_mut(), 0, 30_000, None);
            let mut m: MSG = zeroed();
            while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
                if m.message == WM_TIMER {
                    let new = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), GetModuleHandleW(null()), 0);
                    if !new.is_null() {
                        if !hook.is_null() {
                            UnhookWindowsHookEx(hook);
                        }
                        hook = new;
                    }
                }
            }
        });
        let mut m: MSG = zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_far_off_appbar_edge_goes_back_to_the_screen_edge() {
        assert_eq!(super::sane_edge(48, 1080), 48);
        assert_eq!(super::sane_edge(540, 1080), 0);
        assert_eq!(super::sane_edge(-20, 1080), 0);
    }

    #[test]
    fn borderless_games_count_as_fullscreen() {
        use super::{RECT, covers_monitor};
        let m = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        assert!(covers_monitor(m, m, false, false));
        assert!(covers_monitor(m, m, true, false));
        assert!(!covers_monitor(RECT { left: -8, top: -8, right: 1928, bottom: 1088 }, m, true, true));
        assert!(!covers_monitor(m, m, true, true));
        assert!(!covers_monitor(RECT { left: 0, top: 0, right: 1920, bottom: 1032 }, m, false, false));
    }

    #[test]
    fn network_icon_state() {
        use super::{INET, Link, WIFI, arcs_lit, net};
        let at = |q: i32, inet: bool| {
            WIFI.set(q);
            INET.set(inet);
            net()
        };
        assert!(at(80, true) == Link::Wifi(80));
        assert!(at(80, false) == Link::NoInternet(80));
        assert!(at(-1, true) == Link::Cable);
        assert!(at(-1, false) == Link::Offline);
        assert_eq!((arcs_lit(0), arcs_lit(40), arcs_lit(100)), (1, 2, 3));
    }

    #[test]
    fn mix_blends() {
        assert_eq!(super::mix(super::rgb(0, 0, 0), super::rgb(200, 100, 50), 0.0), super::rgb(0, 0, 0));
        assert_eq!(super::mix(super::rgb(0, 0, 0), super::rgb(200, 100, 50), 1.0), super::rgb(200, 100, 50));
        assert_eq!(super::mix(super::rgb(0, 0, 0), super::rgb(200, 100, 50), 0.5), super::rgb(100, 50, 25));
    }

    #[test]
    fn msix_family_from_package_path() {
        let f = super::msix_family(r"C:\Program Files\WindowsApps\Claude_5.11492.5.0_x64__pzs8sxrjxfjjc\app\claude.exe");
        assert_eq!(f, Some(("claude_pzs8sxrjxfjjc".into(), r"app\claude.exe".into())));
        assert_eq!(super::msix_family(r"C:\Program Files\Steam\steam.exe"), None);
    }

    #[test]
    fn stale_claude_pin_resolves_to_installed_version() {
        let old = r"c:\program files\windowsapps\claude_5.11492.5.0_x64__pzs8sxrjxfjjc\app\claude.exe";
        let Some((fam, _)) = super::msix_family(old) else { panic!() };
        let mut n = 0u32;
        let mut len = 0u32;
        unsafe { windows_sys::Win32::Storage::Packaging::Appx::GetPackagesByPackageFamily(super::wide(&fam).as_ptr(), &mut n, std::ptr::null_mut(), &mut len, std::ptr::null_mut()) };
        if n == 0 {
            return;
        }
        let tmp = std::env::temp_dir().join("topbar-msix-test.lnk");
        let _ = std::fs::remove_file(&tmp);
        unsafe {
            use windows::Win32::System::Com::*;
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let sl: windows::Win32::UI::Shell::IShellLinkW = CoCreateInstance(&windows::Win32::UI::Shell::ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            sl.SetPath(windows::core::PCWSTR(super::wide(old).as_ptr())).unwrap();
            windows::core::Interface::cast::<IPersistFile>(&sl).unwrap().Save(windows::core::PCWSTR(super::wide(&tmp.to_string_lossy()).as_ptr()), true).unwrap();
            let lnk = super::wide(&tmp.to_string_lossy());
            let new = super::fix_msix_pin(&lnk, old).expect("re-pointed");
            assert!(std::path::Path::new(&new).exists() && new.ends_with(r"\app\claude.exe"), "{new}");
            assert_eq!(super::lnk_target(&lnk), new);
        }
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod env_tests {
    #[test]
    fn expands_known_variables_only() {
        let windir = std::env::var("windir").unwrap();
        assert_eq!(super::expand_env(r"%windir%\explorer.exe"), format!(r"{windir}\explorer.exe"));
        assert_eq!(super::expand_env(r"%no_such_var_x%\a.exe"), r"%no_such_var_x%\a.exe");
        assert_eq!(super::expand_env("100% sure"), "100% sure");
    }
}
