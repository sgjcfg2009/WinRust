#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]
mod regsweep;
mod run;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{
        Dwm::DwmSetWindowAttribute,
        Gdi::*,
        GdiPlus::{
            FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateBitmapFromHICON, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush,
            GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDisposeImage, GdipDrawImageRectI, GdipDrawPath, GdipFillPath, GdipSetInterpolationMode, GdipSetSmoothingMode, GdiplusStartup,
            GdiplusStartupInput, GpBitmap, GpBrush, GpGraphics, GpImage, GpPath, GpPen, GpSolidFill, InterpolationModeHighQualityBicubic, SmoothingModeAntiAlias, UnitPixel,
        },
    },
    Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::*,
        Shell::{
            FO_DELETE, FOF_ALLOWUNDO, FOF_NOERRORUI, FOF_SILENT, SHFILEINFOW, SHFILEOPSTRUCTW, SHFileOperationW, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, ShellExecuteW,
        },
        WindowsAndMessaging::*,
    },
};

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetImageList(i_image_list: i32, riid: *const windows_sys::core::GUID, ppv_obj: *mut *mut core::ffi::c_void) -> i32;
}

type Rgb = (u8, u8, u8);
use winrust_common::style::{pal, style, ui_face};
fn cr(c: u32) -> Rgb {
    (c as u8, (c >> 8) as u8, (c >> 16) as u8)
}
fn mixc(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| (((a >> sh) & 255) as f32 + ((((b >> sh) & 255) as f32) - ((a >> sh) & 255) as f32) * t) as u32;
    ch(0) | ch(8) << 8 | ch(16) << 16
}
fn c_bg() -> Rgb {
    cr(pal().bg)
}
fn c_border() -> Rgb {
    cr(pal().line)
}
fn c_sel() -> Rgb {
    cr(pal().active)
}
fn c_accent() -> Rgb {
    cr(pal().accent)
}
fn c_text() -> Rgb {
    cr(pal().text)
}
fn c_dim() -> Rgb {
    cr(pal().dim)
}
fn c_faint() -> Rgb {
    cr(mixc(pal().dim, pal().bg, 0.4))
}
fn c_menu_bg() -> Rgb {
    cr(mixc(pal().bg, pal().line, 0.5))
}
fn c_menu_hot() -> Rgb {
    cr(pal().active)
}

const W: i32 = 680;
const INPUT_H: i32 = 64;
const ROW_H: i32 = 52;
const PAD: i32 = 8;
const FOOT_H: i32 = 34;
const MAX_ROWS: usize = 7;
const RADIUS: i32 = 14;

const T_DEBOUNCE: usize = 1;
const T_CARET: usize = 3;

const F_INPUT: usize = 0;
const F_NAME: usize = 1;
const F_PATH: usize = 2;
const F_ICON: usize = 3;
const F_HINT: usize = 4;

#[derive(Default)]
struct St {
    scale: f32,
    fonts: [isize; 5],
    mem_dc: isize,
    mem_bmp: isize,
    mem_old_bmp: isize,
    mem_bits: isize,
    mem_gp: isize,
    mem_w: i32,
    mem_h: i32,
    win_h: i32,
    text: Vec<u16>,
    results: Vec<String>,
    top: usize,
    sel: usize,
    sel_f: f32,
    height: f32,
    th: f32,
    ta: f32,
    x: i32,
    y: i32,
    caret: bool,
    mouse: (i32, i32),
    icons: HashMap<String, isize>,
    rename: Option<String>,
    saved: Vec<u16>,
    busy: bool,
    menu: Option<MenuSt>,
    purge: Option<Purge>,
    run: bool,
    run_src: Vec<run::Src>,
    programs: Vec<String>,
    programs_loading: bool,
    run_history: Vec<String>,
}
thread_local!(static ST: RefCell<St> = RefCell::new(St::default()));

fn st<R>(f: impl FnOnce(&mut St) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

use winrust_common::wide;
fn rgb(c: Rgb) -> u32 {
    c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16
}
fn argb(c: Rgb) -> u32 {
    0xff00_0000 | (c.0 as u32) << 16 | (c.1 as u32) << 8 | c.2 as u32
}

fn ask(q: &str) -> Vec<String> {
    winrust_common::search::ask_all(q)
}

fn is_app(path: &str) -> bool {
    let l = path.to_lowercase();
    l.ends_with(".lnk") && l.contains("\\start menu\\programs\\")
}

fn labels(path: &str) -> (String, String) {
    let (dir, file) = path.rsplit_once('\\').unwrap_or(("", path));
    if is_app(path) {
        (
            file.trim_end_matches(".lnk").trim_end_matches(".LNK").to_string(),
            "Application".into(),
        )
    } else {
        (file.to_string(), dir.to_string())
    }
}

fn run_labels(line: &str, src: Option<run::Src>) -> (String, String) {
    let desc = |i: usize| run::KNOWN[i].1;
    let known = run::describe(line).map(desc);
    match src {
        Some(run::Src::Typed) => (line.to_string(), known.unwrap_or("Run").into()),
        Some(run::Src::Folder) => {
            let (dir, name) = line.trim_end_matches('\\').rsplit_once('\\').unwrap_or(("", line));
            (format!("{name}{}", if line.ends_with('\\') { "\\" } else { "" }), dir.to_string())
        }
        Some(run::Src::Known(i)) => (line.to_string(), desc(i).into()),
        Some(run::Src::Program) => (line.to_string(), "Program".into()),
        _ => (line.to_string(), known.map_or("Recent".into(), |d| format!("{}  \u{00B7}  {d}", "Recent"))),
    }
}

fn target_height(s: &St) -> f32 {
    let p = |v: i32| v as f32 * s.scale;
    let rows = s.results.len().min(MAX_ROWS).max(1) as i32;
    let base = if s.text.is_empty() && s.results.is_empty() { p(INPUT_H) } else { p(INPUT_H + PAD + rows * ROW_H + if s.results.is_empty() { PAD } else { FOOT_H }) };
    if s.purge.is_some() {
        p(INPUT_H + PAD + PANEL_H)
    } else if s.menu.is_some() {
        base.max(p(INPUT_H + PAD + MENU_PAD * 2 + menu_len() as i32 * MENU_ITEM_H + 6))
    } else {
        base
    }
}

const WM_RESULTS: u32 = WM_APP + 8;
const WM_PROGRAMS: u32 = WM_APP + 9;
static QUERY_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ANSWER: std::sync::Mutex<Option<(u64, Vec<String>)>> = std::sync::Mutex::new(None);
static PROGRAMS_DATA: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

fn spawn_programs_load(h: isize) {
    let should = st(|s| {
        if s.programs_loading {
            false
        } else {
            s.programs_loading = true;
            true
        }
    });
    if should {
        std::thread::spawn(move || {
            let p = run::programs();
            *PROGRAMS_DATA.lock().unwrap() = Some(p);
            unsafe { PostMessageW(h as HWND, WM_PROGRAMS, 0, 0) };
        });
    }
}

unsafe fn sync_window(h: HWND) {
    let (x, y, w, hgt, resize) = st(|s| {
        s.height = s.th;
        s.sel_f = (s.sel.saturating_sub(s.top)) as f32;
        let w = (W as f32 * s.scale) as i32;
        let hgt = s.height.round() as i32;
        let resize = hgt != s.win_h;
        if resize {
            s.win_h = hgt;
        }
        (s.x, s.y, w, hgt, resize)
    });
    if resize {
        SetWindowPos(h, HWND_TOPMOST, x, y, w, hgt, SWP_NOACTIVATE);
    }
    paint(h);
}

unsafe fn show(h: HWND, run: bool) {
    if run && st(|s| s.programs.is_empty()) {
        spawn_programs_load(h as isize);
    }
    let (x, y, max_w, max_h) = {
        let mut pt: POINT = zeroed();
        GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let mw = (mi.rcWork.right - mi.rcWork.left - 16).max(320);
        let mh = (mi.rcWork.bottom - mi.rcWork.top - 16).max(160);
        let w = st(|s| ((W as f32 * s.scale) as i32).min(mw));
        (
            mi.rcWork.left + (mi.rcWork.right - mi.rcWork.left - w) / 2,
            mi.rcWork.top + (mi.rcWork.bottom - mi.rcWork.top) / 5,
            mw,
            mh,
        )
    };
    let (w, hgt) = st(|s| {
        s.text.clear();
        s.run = run;
        if run && s.run_history.is_empty() {
            s.run_history = run::history();
        }
        (s.results, s.run_src) = if run { run::suggest("", &s.run_history, &[]).into_iter().unzip() } else { (vec![], vec![]) };
        (s.rename, s.menu) = (None, None);
        if matches!(s.purge, Some(Purge::Confirm { .. })) {
            s.purge = None;
        }
        (s.top, s.sel, s.sel_f, s.caret) = (0, 0, 0.0, true);
        s.th = target_height(s).min(max_h as f32);
        (s.height, s.ta, s.x, s.y) = (s.th, 1.0, x, y);
        let w = ((W as f32 * s.scale) as i32).min(max_w);
        let hgt = (s.height.round() as i32).min(max_h);
        s.win_h = hgt;
        (w, hgt)
    });
    SetWindowPos(h, HWND_TOPMOST, x, y, w, hgt, SWP_SHOWWINDOW);
    paint(h);
    winrust_common::fg::force(h);
    SetTimer(h, T_CARET, 530, None);
}

unsafe fn hide(h: HWND) {
    st(|s| s.ta = 0.0);
    KillTimer(h, T_CARET);
    ShowWindow(h, SW_HIDE);
}

unsafe fn refresh(h: HWND) {
    use std::sync::atomic::Ordering::SeqCst;
    let q = st(|s| String::from_utf16_lossy(&s.text));
    let seq = QUERY_SEQ.fetch_add(1, SeqCst) + 1;
    if q.trim().is_empty() {
        return apply_results(h, vec![]);
    }
    let hw = h as isize;
    std::thread::spawn(move || {
        let res = ask(&q);
        if QUERY_SEQ.load(SeqCst) == seq {
            *ANSWER.lock().unwrap() = Some((seq, res));
            unsafe { PostMessageW(hw as HWND, WM_RESULTS, 0, 0) };
        }
    });
}

unsafe fn apply_results(h: HWND, res: Vec<String>) {
    st(|s| {
        s.results = res;
        s.menu = None;
        (s.top, s.sel, s.sel_f) = (0, 0, 0.0);
        s.th = target_height(s);
    });
    sync_window(h);
}

unsafe fn open(h: HWND, reveal: bool) {
    if st(|s| s.run) {
        return run_selected(h, reveal && GetKeyState(VK_SHIFT as i32) < 0);
    }
    let Some(p) = st(|s| if s.rename.is_some() { None } else { s.results.get(s.sel).cloned() }) else { return };
    hide(h);
    if reveal {
        winrust_common::reveal(&p);
    } else if std::path::Path::new(&p).is_dir() && open_in_rust_explorer(&p) {
    } else {
        ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(&p).as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

fn rust_explorer_exe() -> Option<std::path::PathBuf> {
    std::env::var_os("ProgramFiles")
        .map(|pf| std::path::PathBuf::from(pf).join(r"rust-explorer\rust-explorer.exe"))
        .filter(|p| p.is_file())
}

fn open_in_rust_explorer(dir: &str) -> bool {
    rust_explorer_exe().is_some_and(|rx| std::process::Command::new(rx).arg(dir).spawn().is_ok())
}

fn remember_run(line: &str) {
    run::remember(line);
    st(|s| run::remember_in(&mut s.run_history, line));
}

unsafe fn run_selected(h: HWND, admin: bool) {
    use windows_sys::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW, ShellExecuteExW};
    let Some(line) = st(|s| s.results.get(s.sel).cloned()) else { return };
    let (file, args) = run::split_cmd(&run::expand(&line), |p| std::path::Path::new(p).exists());
    hide(h);
    if !admin && args.is_empty() {
        if file.to_ascii_lowercase().starts_with("ms-settings:") {
            winrust_common::settings::open(&file);
            remember_run(&line);
            return;
        }
        if file.eq_ignore_ascii_case("shell:MyComputerFolder") {
            if let Some(rx) = rust_explorer_exe() {
                if std::process::Command::new(rx).spawn().is_ok() {
                    remember_run(&line);
                    return;
                }
            }
        }
        if std::path::Path::new(&file).is_dir() && open_in_rust_explorer(&file) {
            remember_run(&line);
            return;
        }
    }
    let (verb, file_w, args_w, dir_w) = (wide(if admin { "runas" } else { "open" }), wide(&file), wide(&args), wide(&std::env::var("USERPROFILE").unwrap_or_default()));
    let mut ei: SHELLEXECUTEINFOW = zeroed();
    ei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    ei.fMask = SEE_MASK_FLAG_NO_UI;
    ei.lpVerb = verb.as_ptr();
    ei.lpFile = file_w.as_ptr();
    ei.lpParameters = if args.is_empty() { null() } else { args_w.as_ptr() };
    ei.lpDirectory = dir_w.as_ptr();
    ei.nShow = SW_SHOWNORMAL;
    if ShellExecuteExW(&mut ei) != 0 {
        remember_run(&line);
    } else if GetLastError() != ERROR_CANCELLED {
        note(&format!("Windows cannot find '{}'. Make sure you typed the name correctly, and then try again.", file), MB_ICONERROR);
    }
}

fn move_sel(delta: isize) {
    st(|s| {
        if s.results.is_empty() || s.rename.is_some() {
            return;
        }
        s.sel = (s.sel as isize + delta).clamp(0, s.results.len() as isize - 1) as usize;
        if s.sel >= s.top + MAX_ROWS {
            s.top = s.sel + 1 - MAX_ROWS;
        }
        if s.sel < s.top {
            s.top = s.sel;
        }
    });
}

fn row_at(y: i32) -> Option<usize> {
    st(|s| {
        let rel = y as f32 - (INPUT_H + PAD) as f32 * s.scale;
        let row = (rel / (ROW_H as f32 * s.scale)).floor();
        (s.purge.is_none() && rel >= 0.0 && (row as usize) < MAX_ROWS && s.top + (row as usize) < s.results.len()).then(|| s.top + row as usize)
    })
}

unsafe fn on_char(h: HWND, c: u16) {
    if c >= 0x20 && c != 0x7f && st(|s| s.menu.is_none() && s.purge.is_none()) {
        st(|s| {
            if s.text.len() < 200 {
                s.text.push(c);
            }
        });
        text_changed(h);
    }
}

unsafe fn clip_copy(h: HWND, cut: bool) {
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
    };
    let len = st(|s| s.text.len());
    if len == 0 || OpenClipboard(h) == 0 {
        return;
    }
    EmptyClipboard();
    let hg = GlobalAlloc(GMEM_MOVEABLE, (len + 1) * 2);
    if !hg.is_null() {
        let p = GlobalLock(hg) as *mut u16;
        if !p.is_null() {
            st(|s| std::ptr::copy_nonoverlapping(s.text.as_ptr(), p, len));
            *p.add(len) = 0;
            GlobalUnlock(hg);
            if SetClipboardData(13, hg).is_null() {
                windows_sys::Win32::Foundation::GlobalFree(hg);
            }
        } else {
            windows_sys::Win32::Foundation::GlobalFree(hg);
        }
    }
    CloseClipboard();
    if cut {
        st(|s| {
            s.text.clear();
            s.caret = true;
        });
        text_changed(h);
    }
}

unsafe fn clip_paste(h: HWND) {
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalUnlock},
    };
    if st(|s| s.menu.is_some() || s.purge.is_some()) || OpenClipboard(h) == 0 {
        return;
    }
    let hd = GetClipboardData(13);
    let mut pasted = false;
    if !hd.is_null() {
        let p = GlobalLock(hd) as *const u16;
        if !p.is_null() {
            st(|s| {
                let mut i = 0usize;
                while s.text.len() < 200 {
                    let c = *p.add(i);
                    if c == 0 {
                        break;
                    }
                    if c >= 0x20 {
                        s.text.push(c);
                        pasted = true;
                    }
                    i += 1;
                }
                s.caret = true;
            });
            GlobalUnlock(hd);
        }
    }
    CloseClipboard();
    if pasted {
        text_changed(h);
    }
}

unsafe fn backspace(h: HWND, word: bool) {
    st(|s| {
        let drop_one = |t: &mut Vec<u16>| {
            if let Some(c) = t.pop() {
                if (0xDC00..0xE000).contains(&c) {
                    t.pop();
                }
            }
        };
        drop_one(&mut s.text);
        if word {
            while s.text.last().is_some_and(|c| *c != 0x20) {
                drop_one(&mut s.text);
            }
        }
        s.caret = true;
    });
    text_changed(h);
}

unsafe fn text_changed(h: HWND) {
    QUERY_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if st(|s| s.rename.is_some()) {
    } else if st(|s| s.run) {
        let (res, src): (Vec<_>, Vec<_>) = st(|s| run::suggest(&String::from_utf16_lossy(&s.text), &s.run_history, &s.programs)).into_iter().unzip();
        apply_results(h, res);
        st(|s| s.run_src = src);
    } else if st(|s| s.text.is_empty()) {
        KillTimer(h, T_DEBOUNCE);
        st(|s| {
            s.results.clear();
            (s.top, s.sel, s.sel_f) = (0, 0, 0.0);
            s.th = target_height(s);
        });
        sync_window(h);
    } else {
        SetTimer(h, T_DEBOUNCE, 70, None);
    }
    InvalidateRect(h, null(), 0);
}

unsafe fn apply_style(main: HWND) {
    let scale = st(|s| s.scale);
    let font = |px: i32, weight: i32, face: &str| {
        CreateFontW(-((px as f32 * scale).round() as i32), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as _, 0, 0, CLEARTYPE_QUALITY as _, 0, wide(face).as_ptr()) as isize
    };
    let face = ui_face();
    let fonts = [font(22, 400, face), font(16, 600, face), font(13, 400, face), font(20, 400, winrust_common::style::icon_face()), font(12, 400, face)];
    for f in st(|s| std::mem::replace(&mut s.fonts, fonts)) {
        if f != 0 {
            DeleteObject(f as _);
        }
    }
    let dark = !style().light() as i32;
    DwmSetWindowAttribute(main, 20, &dark as *const _ as _, 4);
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == winrust_common::style::msg() {
        winrust_common::style::reload();
        apply_style(h);
        InvalidateRect(h, null(), 0);
        return 0;
    }
    match m {
        WM_HOTKEY => {
            let run = wp == 2;
            if IsWindowVisible(h) != 0 && st(|s| s.ta > 0.0 && s.run == run) { hide(h) } else { show(h, run) }
            0
        }
        WM_ACTIVATE => {
            if (wp & 0xffff) as u32 == WA_INACTIVE && st(|s| s.ta > 0.0 && !s.busy && !matches!(s.purge, Some(Purge::Scanning(_) | Purge::Working(_)))) {
                hide(h);
            }
            0
        }
        WM_TIMER => {
            match wp {
                T_DEBOUNCE => {
                    KillTimer(h, T_DEBOUNCE);
                    refresh(h);
                }
                T_CARET => {
                    st(|s| s.caret = !s.caret);
                    InvalidateRect(h, null(), 0);
                }
                _ => {}
            }
            0
        }
        WM_CHAR => {
            on_char(h, wp as u16);
            0
        }
        WM_PURGE => {
            let p = *Box::from_raw(lp as *mut Purge);
            set_purge(h, Some(p));
            0
        }
        WM_KEYDOWN => {
            if st(|s| s.purge.is_some()) {
                let idle = st(|s| matches!(s.purge, Some(Purge::Confirm { .. } | Purge::Done { .. })));
                match wp as u16 {
                    VK_ESCAPE if idle => close_purge(h),
                    VK_ESCAPE => hide(h),
                    VK_RETURN => purge_press(h, st(|s| match &s.purge {
                        Some(Purge::Confirm { hot, .. } | Purge::Done { hot, .. }) => *hot,
                        _ => 0,
                    })),
                    VK_LEFT | VK_RIGHT | VK_TAB => {
                        let n = st(|s| purge_buttons(s).len());
                        st(|s| {
                            if let Some(Purge::Confirm { hot, .. } | Purge::Done { hot, .. }) = s.purge.as_mut() {
                                *hot = if n > 1 { 1 - *hot } else { 0 };
                            }
                        });
                        InvalidateRect(h, null(), 0);
                    }
                    _ => {}
                }
                return 0;
            }
            if st(|s| s.menu.is_some()) {
                match wp as u16 {
                    VK_ESCAPE => close_menu(h),
                    VK_RETURN => run_menu(h, st(|s| s.menu.as_ref().map_or(0, |m| m.hot))),
                    k @ (VK_UP | VK_DOWN) => {
                        st(|s| {
                            if let Some(m) = s.menu.as_mut() {
                                m.hot = if k == VK_DOWN { (m.hot + 1) % menu_len() } else { (m.hot + menu_len() - 1) % menu_len() };
                            }
                        });
                        InvalidateRect(h, null(), 0);
                    }
                    _ => {}
                }
                return 0;
            }
            let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
            match wp as u16 {
                VK_ESCAPE => {
                    if st(|s| s.rename.is_some()) { end_rename(h, None) } else { hide(h) }
                }
                VK_RETURN => {
                    if st(|s| s.rename.is_some()) { finish_rename(h) } else { open(h, ctrl) }
                }
                VK_DOWN => move_sel(1),
                VK_UP => move_sel(-1),
                VK_NEXT => move_sel(MAX_ROWS as isize),
                VK_PRIOR => move_sel(-(MAX_ROWS as isize)),
                VK_BACK => backspace(h, ctrl),
                0x43 if ctrl => clip_copy(h, false),
                0x58 if ctrl => clip_copy(h, true),
                0x56 if ctrl => clip_paste(h),
                VK_TAB if st(|s| s.run) => {
                    if let Some(r) = st(|s| s.results.get(s.sel).cloned()) {
                        st(|s| (s.text, s.caret) = (r.encode_utf16().collect(), true));
                        text_changed(h);
                    }
                }
                _ => return 0,
            }
            sync_window(h);
            0
        }
        WM_MOUSEMOVE => {
            let pos = ((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32);
            if st(|s| s.purge.is_some()) {
                if let Some(i) = purge_hit(pos.0, pos.1) {
                    let changed = st(|s| match s.purge.as_mut() {
                        Some(Purge::Confirm { hot, .. } | Purge::Done { hot, .. }) => std::mem::replace(hot, i) != i,
                        _ => false,
                    });
                    if changed {
                        InvalidateRect(h, null(), 0);
                    }
                }
                return 0;
            }
            if let Some(hit) = menu_hit(pos.0, pos.1) {
                if let Some(i) = hit {
                    if st(|s| s.menu.as_mut().is_some_and(|m| std::mem::replace(&mut m.hot, i) != i)) {
                        InvalidateRect(h, null(), 0);
                    }
                }
                return 0;
            }
            let moved = st(|s| {
                let far = (pos.0 - s.mouse.0).abs() + (pos.1 - s.mouse.1).abs() >= 8;
                if far {
                    s.mouse = pos;
                }
                far
            });
            if let Some(i) = moved.then(|| row_at(pos.1)).flatten() {
                if st(|s| std::mem::replace(&mut s.sel, i)) != i {
                    sync_window(h);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            if st(|s| s.menu.is_some() || s.purge.is_some()) {
                return 0;
            }
            move_sel(if ((wp >> 16) as i16) > 0 { -1 } else { 1 });
            sync_window(h);
            0
        }
        WM_LBUTTONUP => {
            if st(|s| s.purge.is_some()) {
                if let Some(i) = purge_hit((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32) {
                    purge_press(h, i);
                }
                return 0;
            }
            match menu_hit((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32) {
                Some(Some(i)) => return { run_menu(h, i); 0 },
                Some(None) => return { close_menu(h); 0 },
                None => {}
            }
            if let Some(i) = row_at(((lp >> 16) & 0xffff) as i16 as i32) {
                st(|s| s.sel = i);
                open(h, false);
            }
            0
        }
        WM_RBUTTONUP => {
            let (x, y) = ((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32);
            match row_at(y) {
                Some(i) if st(|s| s.rename.is_none() && s.purge.is_none() && !s.run) => open_menu(h, i, x, y),
                _ => close_menu(h),
            }
            0
        }
        WM_RESULTS => {
            let a = ANSWER.lock().unwrap().take();
            if let Some((seq, res)) = a {
                if seq == QUERY_SEQ.load(std::sync::atomic::Ordering::SeqCst) && st(|s| s.ta > 0.0 && s.rename.is_none()) {
                    apply_results(h, res);
                }
            }
            0
        }
        WM_PROGRAMS => {
            if let Some(p) = PROGRAMS_DATA.lock().unwrap().take() {
                st(|s| {
                    s.programs = p;
                    s.programs_loading = false;
                });
                if st(|s| s.ta > 0.0 && s.run) {
                    text_changed(h);
                }
            }
            0
        }
        WM_ERASEBKGND => 1,
        0x02E0 => {
            let dpi = (wp & 0xFFFF) as f32;
            if dpi > 0.0 {
                st(|s| {
                    s.scale = dpi / 96.0;
                    s.th = target_height(s);
                });
                apply_style(h);
                sync_window(h);
            }
            0
        }
        WM_PAINT => {
            paint(h);
            0
        }
        WM_DESTROY => {
            UnregisterHotKey(h, 1);
            st(|s| {
                for f in s.fonts.iter_mut() {
                    if *f != 0 {
                        DeleteObject(*f as _);
                        *f = 0;
                    }
                }
                for (_, img) in s.icons.drain() {
                    if img != 0 {
                        GdipDisposeImage(img as _);
                    }
                }
                if s.mem_gp != 0 {
                    GdipDeleteGraphics(s.mem_gp as _);
                    s.mem_gp = 0;
                }
                if s.mem_dc != 0 {
                    if s.mem_old_bmp != 0 {
                        SelectObject(s.mem_dc as _, s.mem_old_bmp as _);
                        s.mem_old_bmp = 0;
                    }
                    if s.mem_bmp != 0 {
                        DeleteObject(s.mem_bmp as _);
                        s.mem_bmp = 0;
                    }
                    DeleteDC(s.mem_dc as _);
                    s.mem_dc = 0;
                } else if s.mem_bmp != 0 {
                    DeleteObject(s.mem_bmp as _);
                    s.mem_bmp = 0;
                }
            });
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

fn menu_labels() -> [&'static str; 7] {
    [
        "Open",
        "Run as administrator",
        "Show in folder",
        "Add to Start",
        "Rename",
        "Move to Recycle Bin",
        "Delete forever + clean registry",
    ]
}

fn menu_len() -> usize {
    7
}

fn start_file() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("topbar-start.txt")
}

fn start_pins() -> Vec<String> {
    fs::read_to_string(start_file()).map(|s| s.lines().filter(|l| !l.is_empty()).map(String::from).collect()).unwrap_or_default()
}

fn start_pinned(p: &str) -> bool {
    start_pins().iter().any(|x| x.eq_ignore_ascii_case(p))
}

fn start_toggle(p: &str) {
    let mut v = start_pins();
    match v.iter().position(|x| x.eq_ignore_ascii_case(p)) {
        Some(i) => drop(v.remove(i)),
        None => v.push(p.to_string()),
    }
    let _ = fs::write(start_file(), v.join("\n"));
}
const MENU_W: i32 = 270;
const MENU_ITEM_H: i32 = 32;
const MENU_PAD: i32 = 6;
const DANGER: Rgb = (248, 113, 113);

struct MenuSt {
    x: i32,
    y: i32,
    hot: usize,
    path: String,
    pinned: bool,
}

unsafe fn ask_box(h: HWND, text: &str, flags: u32) -> i32 {
    st(|s| s.busy = true);
    let r = MessageBoxW(h, wide(text).as_ptr(), wide("rs launcher").as_ptr(), flags | MB_TOPMOST | MB_SETFOREGROUND);
    st(|s| s.busy = false);
    r
}

fn protected(p: &str) -> bool {
    let l = p.trim_end_matches('\\').to_lowercase();
    if !l.contains('\\') {
        return true;
    }
    let var = |k: &str| std::env::var(k).ok().map(|v| v.trim_end_matches('\\').to_lowercase());
    let win = var("SystemRoot");
    if win.as_ref().is_some_and(|w| l == *w || l.starts_with(&format!("{w}\\"))) {
        return true;
    }
    let profile = var("USERPROFILE");
    let users = profile.as_ref().and_then(|p| p.rsplit_once('\\').map(|(d, _)| d.to_string()));
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramData"].iter().filter_map(|k| var(k)).chain(profile).chain(users).any(|x| l == x)
}

fn menu_rect(s: &St, m: &MenuSt) -> (i32, i32, i32, i32) {
    let p = |v: i32| (v as f32 * s.scale).round() as i32;
    let (w, h) = (p(MENU_W), p(MENU_PAD) * 2 + menu_len() as i32 * p(MENU_ITEM_H));
    let l = m.x.min(p(W) - w - p(6)).max(p(6));
    let t = m.y.min(s.height.round() as i32 - h - p(6)).max(0);
    (l, t, l + w, t + h)
}

fn menu_hit(x: i32, y: i32) -> Option<Option<usize>> {
    st(|s| {
        let m = s.menu.as_ref()?;
        let (l, t, r, b) = menu_rect(s, m);
        let p = |v: i32| (v as f32 * s.scale).round() as i32;
        Some((x >= l && x < r && y >= t && y < b).then(|| (((y - t - p(MENU_PAD)) / p(MENU_ITEM_H)).max(0) as usize).min(menu_len() - 1)))
    })
}

unsafe fn open_menu(h: HWND, row: usize, x: i32, y: i32) {
    let path = st(|s| s.results.get(row).cloned());
    if let Some(path) = path {
        let pinned = start_pinned(&path);
        st(|s| {
            s.sel = row;
            s.menu = Some(MenuSt { x, y, hot: 0, path, pinned });
            s.th = target_height(s);
        });
        sync_window(h);
    }
}

unsafe fn close_menu(h: HWND) {
    st(|s| {
        if s.menu.take().is_some() {
            s.th = target_height(s);
        }
    });
    sync_window(h);
}

unsafe fn run_menu(h: HWND, i: usize) {
    let Some(p) = st(|s| s.menu.as_ref().map(|m| m.path.clone())) else { return };
    close_menu(h);
    match i {
        0 => open(h, false),
        1 => {
            hide(h);
            ShellExecuteW(null_mut(), wide("runas").as_ptr(), wide(&p).as_ptr(), null(), null(), SW_SHOWNORMAL);
        }
        2 => open(h, true),
        3 => start_toggle(&p),
        4 => begin_rename(h, &p),
        _ => remove(h, &p, i == 6),
    }
}

unsafe fn begin_rename(h: HWND, p: &str) {
    if protected(p) {
        ask_box(h, "This is a protected system location.", MB_OK | MB_ICONWARNING);
        return;
    }
    KillTimer(h, T_DEBOUNCE);
    st(|s| {
        let name = p.rsplit('\\').next().unwrap_or(p);
        s.saved = std::mem::replace(&mut s.text, name.encode_utf16().collect());
        s.rename = Some(p.to_string());
        s.caret = true;
    });
    InvalidateRect(h, null(), 0);
}

unsafe fn end_rename(h: HWND, renamed_to: Option<String>) {
    st(|s| {
        if let (Some(old), Some(new)) = (s.rename.take(), renamed_to) {
            if let Some(r) = s.results.iter_mut().find(|r| **r == old) {
                *r = new;
            }
        }
        s.text = std::mem::take(&mut s.saved);
    });
    InvalidateRect(h, null(), 0);
}

unsafe fn finish_rename(h: HWND) {
    let Some(old) = st(|s| s.rename.clone()) else { return };
    let new = st(|s| String::from_utf16_lossy(&s.text)).trim().to_string();
    let (dir, cur) = old.rsplit_once('\\').unwrap_or(("", &old));
    if new.is_empty() || new.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        ask_box(h, "A name cannot be empty or contain \\ / : * ? \" < > |", MB_OK | MB_ICONWARNING);
        return;
    }
    let dst = format!("{dir}\\{new}");
    if new != cur {
        if let Err(e) = fs::rename(&old, &dst) {
            ask_box(h, &format!("Cannot rename:\n{e}"), MB_OK | MB_ICONERROR);
            return;
        }
    }
    end_rename(h, Some(dst));
}

fn recycle(p: &str) -> Result<(), String> {
    let from: Vec<u16> = p.encode_utf16().chain([0, 0]).collect();
    let mut op: SHFILEOPSTRUCTW = unsafe { zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO | FOF_SILENT | FOF_NOERRORUI) as u16;
    let rc = unsafe { SHFileOperationW(&mut op) };
    let aborted = op.fAnyOperationsAborted;
    if rc != 0 || aborted != 0 { Err(format!("Recycle Bin refused it (code {rc})")) } else { Ok(()) }
}

fn note(text: &str, flags: u32) {
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide("rs launcher").as_ptr(), flags | MB_TOPMOST | MB_SETFOREGROUND) };
}

unsafe fn remove(h: HWND, p: &str, purge: bool) {
    if protected(p) {
        ask_box(h, "This is a protected system location.", MB_OK | MB_ICONWARNING);
        return;
    }
    if purge {
        return begin_purge(h, p);
    }
    hide(h);
    let p = p.to_string();
    std::thread::spawn(move || {
        if let Err(e) = recycle(&p) {
            note(&format!("Cannot delete:\n{e}"), MB_OK | MB_ICONERROR);
        }
    });
}

const WM_PURGE: u32 = WM_APP + 7;
const PANEL_H: i32 = 300;
const LIST_MAX: usize = 4;

enum Purge {
    Scanning(String),
    Confirm { path: String, files: u64, bytes: u64, hits: Vec<regsweep::Hit>, hot: usize },
    Working(String),
    Done { path: String, title: String, lines: Vec<String>, ok: bool, backup: Option<String>, hot: usize },
}

fn post(h: isize, p: Purge) {
    unsafe { PostMessageW(h as HWND, WM_PURGE, 0, Box::into_raw(Box::new(p)) as isize) };
}

fn measure(p: &std::path::Path) -> (u64, u64) {
    use std::os::windows::fs::MetadataExt;
    let Ok(m) = fs::symlink_metadata(p) else { return (0, 0) };
    if m.file_attributes() & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != FILE_ATTRIBUTE_DIRECTORY {
        return (1, m.len());
    }
    fs::read_dir(p).map_or((0, 0), |rd| rd.flatten().map(|e| measure(&e.path())).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1)))
}

fn wipe(p: &std::path::Path, failed: &mut Vec<String>) {
    use std::os::windows::fs::MetadataExt;
    let Ok(m) = fs::symlink_metadata(p) else { return };
    let attrs = m.file_attributes();
    let (dir, link) = (attrs & FILE_ATTRIBUTE_DIRECTORY != 0, attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0);
    let before = failed.len();
    if dir && !link {
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                wipe(&e.path(), failed);
            }
        }
        if failed.len() > before {
            return;
        }
    }
    if !link && m.permissions().readonly() {
        let mut perm = m.permissions();
        perm.set_readonly(false);
        let _ = fs::set_permissions(p, perm);
    }
    if let Err(e) = if dir { fs::remove_dir(p) } else { fs::remove_file(p) } {
        failed.push(format!("{}  ({})", p.display(), e.kind()));
    }
}

fn size_text(b: u64) -> String {
    let (v, u) = match b {
        0..1024 => return format!("{b} B"),
        ..1_048_576 => (b as f64 / 1024.0, "KB"),
        ..1_073_741_824 => (b as f64 / 1_048_576.0, "MB"),
        _ => (b as f64 / 1_073_741_824.0, "GB"),
    };
    format!("{v:.1} {u}")
}

fn count_text(n: usize, en: &str) -> String {
    format!("{n} {en}{}", if n == 1 { "" } else { "s" })
}

fn files_text(n: usize) -> String {
    count_text(n, "file")
}

fn items_text(n: usize) -> String {
    count_text(n, "item")
}

fn entries_text(n: usize) -> String {
    count_text(n, "registry entry")
}

fn results_text(n: usize) -> String {
    count_text(n, "result")
}

fn backup_path(p: &str) -> std::path::PathBuf {
    let mut t = unsafe { zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
    let t: windows_sys::Win32::Foundation::SYSTEMTIME = t;
    let name: String = p.rsplit('\\').next().unwrap_or("item").chars().map(|c| if c.is_alphanumeric() || " .-_".contains(c) { c } else { '_' }).take(60).collect();
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    std::path::Path::new(&base)
        .join(r"rs-search\registry-backups")
        .join(format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}  {name}.reg", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond))
}

unsafe fn set_purge(h: HWND, p: Option<Purge>) {
    st(|s| {
        s.menu = None;
        s.purge = p;
        s.th = target_height(s);
    });
    sync_window(h);
}

unsafe fn begin_purge(h: HWND, p: &str) {
    set_purge(h, Some(Purge::Scanning(p.into())));
    let (hw, path) = (h as isize, p.to_string());
    std::thread::spawn(move || {
        let (files, bytes) = measure(std::path::Path::new(&path));
        let hits = regsweep::scan(&path);
        post(hw, Purge::Confirm { path, files, bytes, hits, hot: 0 });
    });
}

unsafe fn run_purge(h: HWND) {
    let Some((path, hits)) = st(|s| match s.purge.take() {
        Some(Purge::Confirm { path, hits, .. }) => Some((path, hits)),
        other => {
            s.purge = other;
            None
        }
    }) else {
        return;
    };
    set_purge(h, Some(Purge::Working(path.clone())));
    let hw = h as isize;
    std::thread::spawn(move || {
        let (files, bytes) = measure(std::path::Path::new(&path));
        let mut failed = Vec::new();
        wipe(std::path::Path::new(&path), &mut failed);
        let done = |title: &str, lines: Vec<String>, ok: bool, backup: Option<String>| Purge::Done { path: path.clone(), title: title.into(), lines, ok, backup, hot: 0 };
        if !failed.is_empty() {
            let mut lines = vec![format!("{} could not be removed (in use or access denied):", items_text(failed.len()))];
            lines.extend(failed.iter().take(3).cloned());
            if failed.len() > 3 {
                lines.push(format!("... and {} more", failed.len() - 3));
            }
            lines.push("The registry was not touched. Close the program using them and try again.".into());
            return post(hw, done("Some files are still there", lines, false, None));
        }
        let mut lines = vec![format!("{} removed ({})", files_text(files as usize), size_text(bytes))];
        if hits.is_empty() {
            lines.push("No registry entries pointed to it.".into());
            return post(hw, done("Deleted", lines, true, None));
        }
        let bak = backup_path(&path);
        if let Err(e) = regsweep::backup(&hits, &bak) {
            lines.push(format!("Registry left as is: the backup could not be written ({}).", e));
            return post(hw, done("Deleted, registry kept", lines, false, None));
        }
        let r = regsweep::delete(&hits);
        lines.push(format!("{} removed", entries_text(r.removed)));
        if r.denied > 0 {
            lines.push(format!("{} need administrator rights and were left in place", entries_text(r.denied)));
        }
        post(hw, done("Deleted", lines, r.denied == 0, Some(bak.display().to_string())));
    });
}

unsafe fn purge_press(h: HWND, i: usize) {
    let action = st(|s| match &s.purge {
        Some(Purge::Confirm { .. }) => Some(if i == 1 { 1 } else { 0 }),
        Some(Purge::Done { backup, .. }) => Some(if i == 1 && backup.is_some() { 2 } else { 3 }),
        _ => None,
    });
    match action {
        Some(1) => run_purge(h),
        Some(2) => {
            let b = st(|s| match &s.purge {
                Some(Purge::Done { backup, .. }) => backup.clone(),
                _ => None,
            });
            if let Some(b) = b {
                hide(h);
                winrust_common::reveal(&b);
            }
        }
        Some(_) => close_purge(h),
        None => {}
    }
}

unsafe fn close_purge(h: HWND) {
    st(|s| {
        if let Some(Purge::Done { path, .. }) = &s.purge {
            if fs::symlink_metadata(path).is_err() {
                let p = path.clone();
                s.results.retain(|r| *r != p);
                s.sel = s.sel.min(s.results.len().saturating_sub(1));
                s.top = s.top.min(s.sel);
            }
        }
    });
    set_purge(h, None);
}

fn purge_buttons(s: &St) -> Vec<(i32, i32, i32, i32)> {
    let p = |v: i32| (v as f32 * s.scale).round() as i32;
    let (w, bottom) = (p(W), p(INPUT_H + PAD + PANEL_H));
    let (bw, bh) = (p(150), p(36));
    let (t, b) = (bottom - p(20) - bh, bottom - p(20));
    let right = (w - p(22) - bw, t, w - p(22), b);
    let left = (right.0 - p(10) - p(110), t, right.0 - p(10), b);
    match &s.purge {
        Some(Purge::Confirm { .. }) => vec![left, right],
        Some(Purge::Done { backup: Some(_), .. }) => vec![left, right],
        Some(Purge::Done { .. }) => vec![(w - p(22) - p(110), t, w - p(22), b)],
        _ => vec![],
    }
}

fn purge_hit(x: i32, y: i32) -> Option<usize> {
    st(|s| purge_buttons(s).iter().position(|&(l, t, r, b)| x >= l && x < r && y >= t && y < b))
}

struct View {
    scale: f32,
    fonts: [isize; 5],
    text: Vec<u16>,
    rows: Vec<(String, String, isize)>,
    total: usize,
    sel_f: f32,
    caret: bool,
    renaming: bool,
    menu: Option<(i32, i32, usize)>,
    menu_pinned: bool,
    purge: Option<PView>,
    run: bool,
}

struct PView {
    glyph: (&'static str, Rgb),
    title: (String, Rgb),
    path: String,
    lines: Vec<(String, Rgb, bool)>,
    note: String,
    buttons: Vec<((i32, i32, i32, i32), &'static str, bool, bool)>,
}

fn purge_view(s: &St) -> Option<PView> {
    let rects = purge_buttons(s);
    let btn = |labels: &[(&'static str, bool)], hot: usize| rects.iter().zip(labels).enumerate().map(|(i, (r, (l, d)))| (*r, *l, *d, i == hot)).collect::<Vec<_>>();
    Some(match s.purge.as_ref()? {
        Purge::Scanning(path) => PView {
            glyph: ("\u{E721}", c_dim()),
            title: ("Checking what will be removed\u{2026}".into(), c_text()),
            path: path.clone(),
            lines: vec![("Counting files and searching the registry. Nothing is deleted yet.".into(), c_dim(), false)],
            note: String::new(),
            buttons: vec![],
        },
        Purge::Working(path) => PView {
            glyph: ("\u{E74D}", DANGER),
            title: ("Deleting\u{2026}".into(), c_text()),
            path: path.clone(),
            lines: vec![("Esc hides this card; the result waits for the next Alt+Space.".into(), c_dim(), false)],
            note: String::new(),
            buttons: vec![],
        },
        Purge::Confirm { path, files, bytes, hits, hot } => {
            let mut lines = vec![(format!("{}  \u{00B7}  {}", files_text(*files as usize), size_text(*bytes)), c_text(), false)];
            if hits.is_empty() {
                lines.push(("No registry entries point to it.".into(), c_dim(), false));
            } else {
                lines.push((format!("{} point to it and will be removed:", entries_text(hits.len())), c_dim(), false));
                lines.extend(hits.iter().take(LIST_MAX).map(|h| (h.describe(), c_faint(), true)));
                if hits.len() > LIST_MAX {
                    lines.push((format!("+ {} more", hits.len() - LIST_MAX), c_faint(), true));
                }
            }
            PView {
                glyph: ("\u{E74D}", DANGER),
                title: ("Delete forever?".into(), DANGER),
                path: path.clone(),
                lines,
                note: if hits.is_empty() {
                    "Files skip the Recycle Bin and cannot be restored.".into()
                } else {
                    "Files skip the Recycle Bin. Registry entries are saved to a .reg backup first.".into()
                },
                buttons: btn(&[
                    ("Cancel", false),
                    ("Delete forever", true),
                ], *hot),
            }
        }
        Purge::Done { path, title, lines, ok, backup, hot } => PView {
            glyph: if *ok { ("\u{E73E}", c_accent()) } else { ("\u{E7BA}", DANGER) },
            title: (title.clone(), if *ok { c_text() } else { DANGER }),
            path: path.clone(),
            lines: lines.iter().enumerate().map(|(i, l)| (l.clone(), if i == 0 { c_text() } else { c_dim() }, false)).collect(),
            note: backup.as_ref().map_or(String::new(), |b| format!("Backup (double-click to restore): {}", b)),
            buttons: if backup.is_some() {
                btn(&[
                    ("Close", false),
                    ("Show backup", false),
                ], *hot)
            } else {
                btn(&[("Close", false)], 0)
            },
        },
    })
}

unsafe fn draw_purge(dc: HDC, g: *mut GpGraphics, bits: *mut u32, stride: i32, hgt: i32, w: i32, v: &View, pv: &PView) {
    let s = v.scale;
    let p = |x: i32| (x as f32 * s).round() as i32;
    let top = p(INPUT_H + PAD);
    for ((l, t, r, b), _, danger, hot) in &pv.buttons {
        let (rad, l, t, bw, bh) = (8.0 * s, *l as f32, *t as f32, (*r - *l) as f32, (*b - *t) as f32);
        if *danger {
            round_rect(g, l, t, bw, bh, rad, DANGER);
            if !*hot {
                round_rect(g, l + 1.5 * s, t + 1.5 * s, bw - 3.0 * s, bh - 3.0 * s, rad - 1.5 * s, c_bg());
            }
        } else {
            round_rect(g, l, t, bw, bh, rad, if *hot { c_menu_hot() } else { c_menu_bg() });
        }
    }
    let left = p(58);
    text(dc, bits, stride, hgt, v.fonts[F_ICON], pv.glyph.1, RECT { left: p(22), top: top + p(12), right: p(52), bottom: top + p(40) }, pv.glyph.0, DT_SINGLELINE | DT_VCENTER);
    text(dc, bits, stride, hgt, v.fonts[F_NAME], pv.title.1, RECT { left, top: top + p(12), right: w - p(22), bottom: top + p(40) }, &pv.title.0, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    text(dc, bits, stride, hgt, v.fonts[F_PATH], c_dim(), RECT { left, top: top + p(40), right: w - p(22), bottom: top + p(60) }, &pv.path, DT_SINGLELINE | DT_PATH_ELLIPSIS);
    let mut y = top + p(74);
    for (line, c, small) in &pv.lines {
        let h_line = p(if *small { 18 } else { 22 });
        let (font, indent) = if *small { (v.fonts[F_HINT], p(12)) } else { (v.fonts[F_PATH], 0) };
        text(dc, bits, stride, hgt, font, *c, RECT { left: left + indent, top: y, right: w - p(22), bottom: y + h_line }, line, DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS);
        y += h_line;
    }
    let bottom = top + p(PANEL_H);
    if !pv.note.is_empty() {
        text(dc, bits, stride, hgt, v.fonts[F_HINT], c_faint(), RECT { left: p(22), top: bottom - p(20 + 36 + 30), right: w - p(22), bottom: bottom - p(20 + 36 + 8) }, &pv.note, DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS);
    }
    for ((l, t, r, b), label, danger, hot) in &pv.buttons {
        let c = if *danger && *hot { (20, 20, 24) } else if *danger { DANGER } else { c_text() };
        text(dc, bits, stride, hgt, v.fonts[F_PATH], c, RECT { left: *l, top: *t, right: *r, bottom: *b }, label, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    }
}

unsafe fn icon_path(p: &str) -> String {
    use windows_sys::Win32::Storage::FileSystem::SearchPathW;
    let e = run::expand(p);
    if e.contains(['\\', '/', ':']) {
        return e;
    }
    let mut b = [0u16; 520];
    let n = SearchPathW(null(), wide(&e).as_ptr(), wide(".exe").as_ptr(), b.len() as u32, b.as_mut_ptr(), null_mut()) as usize;
    if n > 0 && n < b.len() { String::from_utf16_lossy(&b[..n]) } else { e }
}

unsafe fn extract_hires_icon(path: &[u16], scale: f32) -> isize {
    const IID_IIMAGELIST: windows_sys::core::GUID = windows_sys::core::GUID { data1: 0x46EB_5926, data2: 0x582E, data3: 0x4017, data4: [0x9F, 0xDF, 0xE8, 0x99, 0x8D, 0xAA, 0x09, 0x50] };
    let mut fi: SHFILEINFOW = zeroed();
    if SHGetFileInfoW(path.as_ptr(), 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX) == 0 {
        if SHGetFileInfoW(path.as_ptr(), 0x80, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES) == 0 {
            return 0;
        }
    }
    let list_id = if scale > 1.75 { 4 } else { 2 };
    let mut il: *mut core::ffi::c_void = null_mut();
    if SHGetImageList(list_id, &IID_IIMAGELIST, &mut il) != 0 || il.is_null() {
        return 0;
    }
    let vtbl = *(il as *const *const *const core::ffi::c_void);
    let get_icon: unsafe extern "system" fn(*mut core::ffi::c_void, i32, u32, *mut HICON) -> i32 = std::mem::transmute(*vtbl.add(10));
    let release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32 = std::mem::transmute(*vtbl.add(2));
    let mut hicon: HICON = null_mut();
    let hr = get_icon(il, fi.iIcon, 1, &mut hicon);
    release(il);
    if hr != 0 || hicon.is_null() {
        return 0;
    }
    let mut bmp: *mut GpBitmap = null_mut();
    GdipCreateBitmapFromHICON(hicon, &mut bmp);
    DestroyIcon(hicon);
    bmp as isize
}

unsafe fn ensure_icons() {
    st(|s| {
        if s.icons.len() > 512 {
            for (_, i) in s.icons.drain() {
                if i != 0 {
                    GdipDisposeImage(i as _);
                }
            }
        }
    });
    let (scale, need): (f32, Vec<String>) = st(|s| (s.scale, s.results.iter().skip(s.top).take(MAX_ROWS).filter(|p| !s.icons.contains_key(*p)).cloned().collect()));
    for p in need {
        let path = wide(&icon_path(&p));
        let bmp = extract_hires_icon(&path, scale);
        st(|s| {
            s.icons.insert(p, bmp);
        });
    }
}

unsafe fn fix_alpha(bits: *mut u32, stride: i32, hgt: i32, r: RECT) {
    let (x0, x1) = (r.left.max(0), r.right.min(stride));
    let (y0, y1) = (r.top.max(0), r.bottom.min(hgt));
    for y in y0..y1 {
        let row = bits.add((y * stride) as usize);
        for x in x0..x1 {
            *row.add(x as usize) |= 0xFF00_0000;
        }
    }
}

unsafe fn paint(h: HWND) {
    ensure_icons();
    let v = st(|s| View {
        scale: s.scale,
        fonts: s.fonts,
        text: s.text.clone(),
        rows: s.results.iter().enumerate().skip(s.top).take(MAX_ROWS).map(|(i, p)| {
            let (a, b) = if s.run { run_labels(p, s.run_src.get(i).copied()) } else { labels(p) };
            (a, b, s.icons.get(p).copied().unwrap_or(0))
        }).collect(),
        total: s.results.len(),
        sel_f: s.sel_f,
        caret: s.caret,
        renaming: s.rename.is_some(),
        menu: s.menu.as_ref().map(|m| {
            let (l, t, ..) = menu_rect(s, m);
            (l, t, m.hot)
        }),
        menu_pinned: s.menu.as_ref().is_some_and(|m| m.pinned),
        purge: purge_view(s),
        run: s.run,
    });
    let mut ps: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(h, &mut ps);
    let (x, y, w, hgt) = st(|s| (s.x, s.y, (W as f32 * s.scale) as i32, s.win_h.max(1)));
    let (mem, bmp, bits, gp) = st(|s| {
        if s.mem_dc == 0 {
            s.mem_dc = CreateCompatibleDC(dc) as isize;
        }
        if s.mem_bmp == 0 || w > s.mem_w || hgt > s.mem_h {
            if s.mem_gp != 0 {
                GdipDeleteGraphics(s.mem_gp as _);
                s.mem_gp = 0;
            }
            if s.mem_bmp != 0 {
                DeleteObject(s.mem_bmp as _);
                s.mem_bmp = 0;
                s.mem_bits = 0;
            }
            s.mem_w = s.mem_w.max(w);
            s.mem_h = s.mem_h.max(hgt);
            let mut bi: BITMAPINFO = zeroed();
            bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = s.mem_w;
            bi.bmiHeader.biHeight = -s.mem_h;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut pbits: *mut core::ffi::c_void = null_mut();
            let b = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut pbits, null_mut(), 0);
            s.mem_bmp = b as isize;
            s.mem_bits = pbits as isize;
            let prev = SelectObject(s.mem_dc as _, b as _);
            if s.mem_old_bmp == 0 {
                s.mem_old_bmp = prev as isize;
            }
            let mut g: *mut GpGraphics = null_mut();
            GdipCreateFromHDC(s.mem_dc as _, &mut g);
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            GdipSetInterpolationMode(g, InterpolationModeHighQualityBicubic);
            s.mem_gp = g as isize;
        }
        (s.mem_dc as HDC, s.mem_bmp as HBITMAP, s.mem_bits as *mut u32, s.mem_gp as *mut GpGraphics)
    });
    let _ = bmp;
    let stride = st(|s| s.mem_w);
    std::ptr::write_bytes(bits, 0, (stride * hgt) as usize);
    draw(mem, gp, bits, stride, w, hgt, &v);
    let mut pt_dst = POINT { x, y };
    let mut sz = SIZE { cx: w, cy: hgt };
    let mut pt_src = POINT { x: 0, y: 0 };
    let bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    UpdateLayeredWindow(h, dc, &mut pt_dst, &mut sz, mem, &mut pt_src, 0, &bf, ULW_ALPHA);
    EndPaint(h, &ps);
}

unsafe fn fill(dc: HDC, l: i32, t: i32, r: i32, b: i32, c: Rgb) {
    SetDCBrushColor(dc, rgb(c));
    FillRect(dc, &RECT { left: l, top: t, right: r, bottom: b }, GetStockObject(DC_BRUSH) as _);
}

unsafe fn make_round_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> *mut GpPath {
    let r = if style().square() { 0.5 } else { r };
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    let d = r * 2.0;
    GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    path
}

unsafe fn round_rect(g: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb) {
    let path = make_round_path(x, y, w, h, r);
    let mut b: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(argb(c), &mut b);
    GdipFillPath(g, b as *mut GpBrush, path);
    GdipDeleteBrush(b as *mut GpBrush);
    GdipDeletePath(path);
}

unsafe fn stroke_round_rect(g: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb) {
    let path = make_round_path(x, y, w, h, r);
    let mut pen: *mut GpPen = null_mut();
    GdipCreatePen1(argb(c), 1.0, UnitPixel, &mut pen);
    GdipDrawPath(g, pen, path);
    GdipDeletePen(pen);
    GdipDeletePath(path);
}

unsafe fn text(dc: HDC, bits: *mut u32, stride: i32, hgt: i32, font: isize, c: Rgb, mut r: RECT, s: &str, flags: u32) {
    thread_local! { static BUF: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) }; }
    let old = SelectObject(dc, font as _);
    SetTextColor(dc, rgb(c));
    BUF.with_borrow_mut(|w| {
        w.clear();
        w.extend(s.encode_utf16());
        DrawTextW(dc, w.as_ptr(), w.len() as i32, &mut r, flags | DT_NOPREFIX);
    });
    SelectObject(dc, old);
    fix_alpha(bits, stride, hgt, r);
}

unsafe fn draw(dc: HDC, g: *mut GpGraphics, bits: *mut u32, stride: i32, w: i32, hgt: i32, v: &View) {
    let s = v.scale;
    let p = |x: i32| (x as f32 * s).round() as i32;
    SetBkMode(dc, TRANSPARENT as i32);
    round_rect(g, 0.5, 0.5, (w - 1) as f32, (hgt - 1) as f32, RADIUS as f32 * s, c_bg());
    stroke_round_rect(g, 0.5, 0.5, (w - 1) as f32, (hgt - 1) as f32, RADIUS as f32 * s, c_border());

    let (rows_top, row_h) = (p(INPUT_H + PAD), ROW_H as f32 * s);
    let nrows = v.rows.len();

    if nrows > 0 && v.purge.is_none() {
        let y = rows_top as f32 + v.sel_f * row_h;
        round_rect(g, p(PAD) as f32, y + 2.0 * s, (w - 2 * p(PAD)) as f32, row_h - 4.0 * s, 10.0 * s, c_sel());
        round_rect(g, p(PAD + 4) as f32, y + row_h / 2.0 - 10.0 * s, 3.0 * s, 20.0 * s, 1.5 * s, c_accent());
    }

    let mid = p(INPUT_H) / 2;
    let (glyph, gc) = if v.run { ("\u{E756}", c_accent()) } else { ("\u{E721}", c_dim()) };
    text(dc, bits, stride, hgt, v.fonts[F_ICON], gc, RECT { left: p(22), top: mid - p(14), right: p(52), bottom: mid + p(14) }, glyph, DT_SINGLELINE | DT_VCENTER);
    let area = RECT { left: p(58), top: mid - p(18), right: w - p(24), bottom: mid + p(18) };
    let shown = String::from_utf16_lossy(&v.text);
    if v.text.is_empty() {
        let hint = if v.run { "Type a program, folder, document or website" } else { "Search files and apps" };
        text(dc, bits, stride, hgt, v.fonts[F_INPUT], c_faint(), area, hint, DT_SINGLELINE | DT_VCENTER);
    } else {
        let mut size: SIZE = zeroed();
        let old = SelectObject(dc, v.fonts[F_INPUT] as _);
        GetTextExtentPoint32W(dc, v.text.as_ptr(), v.text.len() as i32, &mut size);
        SelectObject(dc, old);
        let overflow = (size.cx - (area.right - area.left - p(4))).max(0);
        let saved = SaveDC(dc);
        IntersectClipRect(dc, area.left, area.top, area.right, area.bottom);
        text(dc, bits, stride, hgt, v.fonts[F_INPUT], c_text(), RECT { left: area.left - overflow, ..area }, &shown, DT_SINGLELINE | DT_VCENTER | DT_NOCLIP);
        if v.caret {
            let cr_rc = RECT { left: area.left - overflow + size.cx + p(2), top: mid - p(13), right: area.left - overflow + size.cx + p(4), bottom: mid + p(13) };
            fill(dc, cr_rc.left, cr_rc.top, cr_rc.right, cr_rc.bottom, c_accent());
            fix_alpha(bits, stride, hgt, cr_rc);
        }
        RestoreDC(dc, saved);
    }
    if v.text.is_empty() && v.caret {
        let cr_rc = RECT { left: p(58), top: mid - p(13), right: p(58) + p(2), bottom: mid + p(13) };
        fill(dc, cr_rc.left, cr_rc.top, cr_rc.right, cr_rc.bottom, c_accent());
        fix_alpha(bits, stride, hgt, cr_rc);
    }

    if !v.text.is_empty() || v.purge.is_some() || nrows > 0 {
        let b_rc = RECT { left: p(PAD + 6), top: p(INPUT_H) - 1, right: w - p(PAD + 6), bottom: p(INPUT_H) };
        fill(dc, b_rc.left, b_rc.top, b_rc.right, b_rc.bottom, c_border());
        fix_alpha(bits, stride, hgt, b_rc);
    }
    if let Some(pv) = &v.purge {
        return draw_purge(dc, g, bits, stride, hgt, w, v, pv);
    }
    if nrows == 0 {
        if !v.text.is_empty() {
            text(dc, bits, stride, hgt, v.fonts[F_NAME], c_dim(), RECT { left: 0, top: rows_top, right: w, bottom: rows_top + row_h as i32 }, "No results", DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        }
        return;
    }

    for (i, (name, sub, icon)) in v.rows.iter().enumerate() {
        let y0 = rows_top + (i as f32 * row_h) as i32;
        let isz = p(28);
        if *icon != 0 {
            GdipDrawImageRectI(g, *icon as *mut GpImage, p(PAD + 16), y0 + (row_h as i32 - isz) / 2, isz, isz);
        }
        let (left, right) = (p(PAD + 16 + 28 + 14), w - p(PAD + 14));
        text(dc, bits, stride, hgt, v.fonts[F_NAME], c_text(), RECT { left, top: y0 + p(7), right, bottom: y0 + p(29) }, name, DT_SINGLELINE | DT_END_ELLIPSIS);
        text(dc, bits, stride, hgt, v.fonts[F_PATH], c_dim(), RECT { left, top: y0 + p(29), right, bottom: y0 + p(47) }, sub, DT_SINGLELINE | DT_END_ELLIPSIS);
    }
    let foot_y = rows_top + (nrows as f32 * row_h) as i32;
    let hint: std::borrow::Cow<'_, str> = if v.run {
        "\u{21B5} run     Ctrl+Shift+\u{21B5} as administrator     Tab complete     Esc close".into()
    } else if v.renaming {
        "Type the new name     \u{21B5} rename     Esc cancel".into()
    } else if v.total > nrows {
        format!("\u{2191}\u{2193} select     \u{21B5} open     Ctrl+\u{21B5} folder     Right-click menu     Esc close   \u{2022}   {}", results_text(v.total)).into()
    } else {
        "\u{2191}\u{2193} select     \u{21B5} open     Ctrl+\u{21B5} folder     Right-click menu     Esc close".into()
    };
    text(
        dc,
        bits,
        stride,
        hgt,
        v.fonts[F_HINT],
        c_faint(),
        RECT { left: p(PAD + 16), top: foot_y + p(4), right: w - p(PAD + 14), bottom: foot_y + p(FOOT_H) },
        &hint,
        DT_SINGLELINE | DT_VCENTER,
    );

    if let Some((l, t, hot)) = v.menu {
        let (mw, pad, ih) = (p(MENU_W), p(MENU_PAD), p(MENU_ITEM_H));
        let mh = pad * 2 + menu_len() as i32 * ih;
        round_rect(g, l as f32, t as f32, mw as f32, mh as f32, 10.0 * s, c_border());
        round_rect(g, (l + 1) as f32, (t + 1) as f32, (mw - 2) as f32, (mh - 2) as f32, 9.0 * s, c_menu_bg());
        round_rect(g, (l + pad) as f32, (t + pad + hot as i32 * ih) as f32, (mw - 2 * pad) as f32, ih as f32, 7.0 * s, c_menu_hot());
        for (i, label) in menu_labels().iter().enumerate() {
            let label = if i == 3 && v.menu_pinned { "Remove from Start" } else { label };
            let y0 = t + pad + i as i32 * ih;
            let c = if i == menu_len() - 1 { DANGER } else { c_text() };
            text(dc, bits, stride, hgt, v.fonts[F_PATH], c, RECT { left: l + pad + p(12), top: y0, right: l + mw - pad, bottom: y0 + ih }, label, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        }
    }
}

fn kill_search_host() {
    winrust_common::proc::pids("SearchHost.exe").into_iter().for_each(winrust_common::proc::kill);
}

fn search_host_guard() {
    use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
    unsafe extern "system" fn created(_: HWINEVENTHOOK, _: u32, w: HWND, id: i32, child: i32, _: u32, _: u32) {
        thread_local! { static LAST: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
        if id != 0 || child != 0 || w.is_null() || unsafe { GetAncestor(w, GA_ROOT) } != w {
            return;
        }
        let mut pid = 0u32;
        if unsafe { GetWindowThreadProcessId(w, &mut pid) } == 0 || pid == 0 || LAST.replace(pid) == pid {
            return;
        }
        if winrust_common::proc::exe_name(pid).is_some_and(|n| n.eq_ignore_ascii_case("SearchHost.exe")) {
            winrust_common::proc::kill(pid);
        }
    }
    kill_search_host();
    unsafe {
        SetWinEventHook(0x8000, 0x8000, null_mut(), Some(created), 0, 0, 0x0002);
        let mut m: MSG = zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {}
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--noop") {
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--autostart") {
        use std::os::windows::process::CommandExt;
        let exe = std::env::current_exe().unwrap();
        let ok = std::process::Command::new("reg")
            .args(winrust_common::reg::user_args(&["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "rs-launcher", "/t", "REG_SZ", "/d", &format!("\"{}\"", exe.display()), "/f"]))
            .creation_flags(0x0800_0000)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        std::process::exit(if ok { 0 } else { 1 });
    }
    unsafe {
        SetProcessDPIAware();
        let dc = GetDC(null_mut());
        let scale = GetDeviceCaps(dc, LOGPIXELSX as i32) as f32 / 96.0;
        ReleaseDC(null_mut(), dc);

        let (mut token, mut input): (usize, GdiplusStartupInput) = (0, zeroed());
        input.GdiplusVersion = 1;
        GdiplusStartup(&mut token, &input, null_mut());

        let hi = GetModuleHandleW(null());
        let cls = wide("rs_launcher");
        RegisterClassW(&WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hi,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: cls.as_ptr(),
        });
        let main = CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST, cls.as_ptr(), wide("rs launcher").as_ptr(), WS_POPUP, 0, 0, 10, 10, null_mut(), null_mut(), hi, null());
        if main.is_null() {
            let msg = wide(&format!("cannot create window (error {})", GetLastError()));
            MessageBoxW(null_mut(), msg.as_ptr(), wide("rs launcher").as_ptr(), MB_ICONERROR);
            return;
        }

        st(|s| {
            s.scale = scale;
            s.run_history = run::history();
        });
        apply_style(main);
        spawn_programs_load(main as isize);

        if RegisterHotKey(main, 1, MOD_ALT | MOD_NOREPEAT, VK_SPACE as u32) == 0 {
            MessageBoxW(null_mut(), wide("Alt+Space is taken (is the launcher already running?)").as_ptr(), wide("rs launcher").as_ptr(), MB_ICONERROR);
            return;
        }
        let blocked = winrust_common::reg::exists(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\SearchHost.exe", Some("Debugger"));
        if blocked {
            std::thread::spawn(|| search_host_guard());
        }
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        windows_sys::Win32::Graphics::GdiPlus::GdiplusShutdown(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    #[test]
    fn wipe_unlocks_read_only_and_never_follows_junctions() {
        let base = std::env::temp_dir().join("rs-wipe-test-7c1d");
        let _ = fs::remove_dir_all(&base);
        let (doomed, outside) = (base.join("doomed"), base.join("outside"));
        fs::create_dir_all(doomed.join("sub")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let ro = doomed.join("sub").join("ro.txt");
        fs::write(&ro, "x").unwrap();
        let mut perm = fs::metadata(&ro).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&ro, perm).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let link = doomed.join("link");
        assert!(std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&outside).creation_flags(0x0800_0000).output().unwrap().status.success());

        assert_eq!(measure(&doomed), (2, 1), "ro.txt + the junction itself, target not counted");
        let mut failed = Vec::new();
        wipe(&doomed, &mut failed);
        assert!(failed.is_empty(), "{failed:?}");
        assert!(!doomed.exists());
        assert!(outside.join("keep.txt").exists(), "junction target must survive");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn sizes() {
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(5 * 1_048_576), "5.0 MB");
    }
}
