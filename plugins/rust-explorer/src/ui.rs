
#![allow(unsafe_op_in_unsafe_fn)]
use super::*;
use std::{cell::Cell, collections::HashMap, path::Path};
use windows_sys::Win32::{Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*}, System::{LibraryLoader::{GetProcAddress, LoadLibraryW}, SystemInformation::GetTickCount64}, UI::Input::KeyboardAndMouse::{GetFocus, ReleaseCapture, TRACKMOUSEEVENT, TME_LEAVE, TrackMouseEvent}};

pub use winrust_common::style::{pal, ui_face};
pub fn listbg() -> u32 { if winrust_common::style::style().light() { winrust_common::style::card() } else { mix(pal().bg, pal().text, 0.015) } }
pub fn toolbar_bg() -> u32 { if winrust_common::style::style().light() { mix(pal().bg, 0x00FF_FFFF, 0.5) } else { mix(pal().bg, pal().text, 0.045) } }
pub fn cmd_bg() -> u32 { if winrust_common::style::style().light() { pal().bg } else { mix(pal().bg, pal().text, 0.022) } }
pub fn pill() -> u32 { if winrust_common::style::style().light() { mix(pal().bg, pal().text, 0.06) } else { mix(pal().bg, pal().text, 0.065) } }
pub fn off() -> u32 { mix(pal().dim, pal().bg, 0.45) }

pub const CHROME_H: i32 = 122;
pub const STATUS_H: i32 = 26;
pub const WM_UI: u32 = WM_APP + 4;
const WM_MOUSELEAVE: u32 = 0x02A3;

const NONE: i32 = -1;
const BACK: i32 = 1;
const FWD: i32 = 2;
const UP: i32 = 3;
const RELOAD: i32 = 4;
const SEARCH: i32 = 5;
const VIEW: i32 = 6;
const PLUS: i32 = 7;
const NEW_BTN: i32 = 8;
const SORT_BTN: i32 = 9;
const CMD: i32 = 10;
const MORE_BTN: i32 = 15;
const WIN_MIN: i32 = 16;
const WIN_MAX: i32 = 17;
const WIN_CLOSE: i32 = 18;
const CMD_GLYPHS: [&str; 5] = ["\u{E8C6}", "\u{E8C8}", "\u{E77F}", "\u{E8AC}", "\u{E74D}"];
const TAB: i32 = 100;
const CLOSE: i32 = 200;
const CRUMB: i32 = 400;

thread_local! {
    static CRUMBS: RefCell<Vec<(RECT, PathBuf)>> = const { RefCell::new(Vec::new()) };
    static SEARCHING: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn set_search(q: Option<String>) { SEARCHING.set(q); }

pub fn crumb_path(i: usize) -> Option<PathBuf> { CRUMBS.with_borrow(|c| c.get(i).map(|x| x.1.clone())) }

fn crumb_at(x: i32, y: i32) -> Option<usize> { CRUMBS.with_borrow(|c| c.iter().position(|(r, _)| inside(r, x, y))) }

fn crumbs_of(dir: &Path) -> Vec<(String, PathBuf)> {
    if dir == Path::new(crate::THIS_PC_KEY) {
        return vec![("This PC".into(), dir.to_path_buf())];
    }
    let mut acc = PathBuf::new();
    dir.components().filter_map(|c| {
        acc.push(c.as_os_str());
        match c {
            std::path::Component::RootDir => None,
            std::path::Component::Prefix(p) => { acc.push("\\"); Some((p.as_os_str().to_string_lossy().into_owned(), acc.clone())) }
            _ => Some((c.as_os_str().to_string_lossy().into_owned(), acc.clone())),
        }
    }).collect()
}

thread_local! {
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
    static HV: RefCell<HashMap<i32, (f32, u64)>> = RefCell::new(HashMap::new());
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static HOT: Cell<i32> = const { Cell::new(NONE) };
    static PRESS: Cell<(i32, u64)> = const { Cell::new((i32::MIN, 0)) };
    static BRUSH: Cell<isize> = const { Cell::new(0) };
}

pub fn init_scale() {
    unsafe {
        let dc = GetDC(ptr::null_mut());
        SCALE.set(GetDeviceCaps(dc, LOGPIXELSX as i32) as f32 / 96.0);
        ReleaseDC(ptr::null_mut(), dc);
    }
}

pub fn sc(v: i32) -> i32 { (v as f32 * SCALE.get()).round() as i32 }

fn now() -> u64 { unsafe { GetTickCount64() } }

fn ease(t: f32) -> f32 { t * t * (3.0 - 2.0 * t) }

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        (x + (y - x) * t) as u32
    };
    ch(0) | ch(8) << 8 | ch(16) << 16
}

fn hv(key: i32, on: bool) -> f32 {
    let t = now();
    let target = if on { 1.0 } else { 0.0 };
    let v = HV.with_borrow_mut(|m| {
        let e = m.entry(key).or_insert((0.0, t));
        let dt = t.saturating_sub(e.1).min(50) as f32;
        e.1 = t;
        e.0 = if e.0 < target { (e.0 + dt / 120.0).min(target) } else { (e.0 - dt / 200.0).max(target) };
        e.0
    });
    if v != target { BUSY.set(true); }
    ease(v)
}

fn press_t(key: i32) -> f32 {
    let (k, t0) = PRESS.get();
    let t = now().saturating_sub(t0) as f32 / 320.0;
    if k != key || t >= 1.0 { return 0.0; }
    BUSY.set(true);
    (1.0 - t) * (1.0 - t)
}

pub fn font(px: i32, weight: i32, face: &str) -> isize {
    unsafe { CreateFontW(-sc(px), 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, w(face).as_ptr()) as isize }
}

pub fn dark_title(h: HWND) {
    unsafe {
        let set = |attr: u32, v: i32| { DwmSetWindowAttribute(h, attr, &v as *const _ as _, 4); };
        set(20, !winrust_common::style::style().light() as i32);
        set(33, if winrust_common::style::style().square() { 1 } else { 2 });
        set(35, pal().bg as i32);
        set(36, pal().text as i32);
        set(34, pal().line as i32);
    }
}

pub fn dark_mode() {
    unsafe {
        let m = LoadLibraryW(w("uxtheme.dll").as_ptr());
        if m.is_null() { return; }
        if let Some(f) = GetProcAddress(m, 135usize as *const u8) { std::mem::transmute::<_, extern "system" fn(i32) -> i32>(f)(2); }
        if let Some(f) = GetProcAddress(m, 136usize as *const u8) { std::mem::transmute::<_, extern "system" fn()>(f)(); }
    }
}

pub unsafe fn restyle(h: HWND) {
    unsafe extern "system" fn each(c: HWND, _: LPARAM) -> i32 {
        let mut cls = [0u16; 32];
        let n = GetClassNameW(c, cls.as_mut_ptr(), 32).max(0) as usize;
        if String::from_utf16_lossy(&cls[..n]) == "SysListView32" {
            style_list(c);
        }
        1
    }
    EnumChildWindows(h, Some(each), 0);
    let (a, b) = FONTS.replace((0, 0));
    for f in [a, b].into_iter().filter(|&f| f != 0) {
        DeleteObject(f as HGDIOBJ);
    }
    for f in CHROME_FONTS.replace([0; 3]).into_iter().filter(|&f| f != 0) { DeleteObject(f as HGDIOBJ); }
    for f in NAV_FONTS.replace([0; 4]).into_iter().filter(|&f| f != 0) { DeleteObject(f as HGDIOBJ); }
    for f in PC_FONTS.replace([0; 4]).into_iter().filter(|&f| f != 0) { DeleteObject(f as HGDIOBJ); }
    for f in MENU_FONTS.replace([0; 2]).into_iter().filter(|&f| f != 0) { DeleteObject(f as HGDIOBJ); }
    for f in HDR_FONTS.replace([0; 2]).into_iter().filter(|&f| f != 0) { DeleteObject(f as HGDIOBJ); }
    for f in [STATUS_FONT.replace(0), LIST_ICO_FONT.replace(0), EMPTY_FONT.replace(0)].into_iter().filter(|&f| f != 0) {
        DeleteObject(f as HGDIOBJ);
    }
    PC_INFO.with_borrow_mut(|m| m.clear());
    dark_title(h);
    let old = SetClassLongPtrW(h, GCLP_HBRBACKGROUND, CreateSolidBrush(pal().bg) as isize);
    DeleteObject(old as HGDIOBJ);
    RedrawWindow(h, ptr::null(), ptr::null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME);
}

pub fn style_list(list: HWND) {
    unsafe {
        let light = winrust_common::style::style().light();
        SetWindowTheme(list, w(if light { "Explorer" } else { "DarkMode_Explorer" }).as_ptr(), ptr::null());
        let hdr = SendMessageW(list, LVM_GETHEADER, 0, 0) as HWND;
        SetWindowTheme(hdr, w(if light { "ItemsView" } else { "DarkMode_ItemsView" }).as_ptr(), ptr::null());
        for (m, c) in [(LVM_SETBKCOLOR, listbg()), (LVM_SETTEXTBKCOLOR, listbg()), (LVM_SETTEXTCOLOR, pal().text)] { SendMessageW(list, m, 0, c as isize); }
        let old_f = SendMessageW(list, WM_GETFONT, 0, 0);
        SendMessageW(list, WM_SETFONT, font(14, 400, ui_face()) as usize, 1);
        if old_f != 0 { DeleteObject(old_f as HGDIOBJ); }
        row_height(list, true);
        SetWindowSubclass(list, Some(list_sub), 1, 0);
    }
}

pub const ROW_H: i32 = 36;

pub fn row_height(list: HWND, on: bool) {
    unsafe {
        let il = if on { ImageList_Create(1, sc(ROW_H), ILC_COLOR32, 1, 1) as isize } else { 0 };
        let old = SendMessageW(list, LVM_SETIMAGELIST, LVSIL_STATE as usize, il);
        if old != 0 { ImageList_Destroy(old as _); }
    }
}

struct BackBuf {
    dc: HDC,
    bmp: HBITMAP,
    w: i32,
    h: i32,
}

impl BackBuf {
    const fn new() -> Self {
        Self { dc: ptr::null_mut(), bmp: ptr::null_mut(), w: 0, h: 0 }
    }
    unsafe fn get(&mut self, wdc: HDC, w: i32, h: i32) -> HDC {
        let (w, h) = (w.max(1), h.max(1));
        if self.dc.is_null() {
            self.dc = CreateCompatibleDC(wdc);
        }
        if self.bmp.is_null() || self.w != w || self.h != h {
            let nb = CreateCompatibleBitmap(wdc, w, h);
            let prev = SelectObject(self.dc, nb);
            if !self.bmp.is_null() {
                DeleteObject(prev);
            }
            self.bmp = nb;
            self.w = w;
            self.h = h;
        }
        self.dc
    }
}

thread_local! {
    static ROWHOT: Cell<isize> = const { Cell::new(-1) };
    static FONTS: Cell<(isize, isize)> = const { Cell::new((0, 0)) };
    static PC_SEL: Cell<isize> = const { Cell::new(-1) };
    static PC_SCROLL: Cell<i32> = const { Cell::new(0) };
    static PC_COLLAPSED: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::from(["Folders".to_string()]));
    static CHROME_BUF: RefCell<BackBuf> = const { RefCell::new(BackBuf::new()) };
    static NAV_BUF: RefCell<BackBuf> = const { RefCell::new(BackBuf::new()) };
    static PC_BUF: RefCell<BackBuf> = const { RefCell::new(BackBuf::new()) };
    static ICON_BUF: RefCell<BackBuf> = const { RefCell::new(BackBuf::new()) };
    static CHROME_FONTS: Cell<[isize; 3]> = const { Cell::new([0; 3]) };
    static NAV_FONTS: Cell<[isize; 4]> = const { Cell::new([0; 4]) };
    static PC_FONTS: Cell<[isize; 4]> = const { Cell::new([0; 4]) };
    static MENU_FONTS: Cell<[isize; 2]> = const { Cell::new([0; 2]) };
    static HDR_FONTS: Cell<[isize; 2]> = const { Cell::new([0; 2]) };
    static STATUS_FONT: Cell<isize> = const { Cell::new(0) };
    static LIST_ICO_FONT: Cell<isize> = const { Cell::new(0) };
    static EMPTY_FONT: Cell<isize> = const { Cell::new(0) };
    static WBUF: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    static IL_XL: Cell<isize> = const { Cell::new(0) };
    static PC_INFO: RefCell<HashMap<String, (String, Option<(u64, u64)>, i32)>> = RefCell::new(HashMap::new());
}

fn pc_sec_key(name: &str) -> String {
    name.trim_start_matches('\0').split(" (").next().unwrap_or("").to_string()
}

pub fn pc_selected() -> Option<usize> {
    usize::try_from(PC_SEL.get()).ok()
}

pub fn reset_pc() {
    PC_SEL.set(-1);
    PC_SCROLL.set(0);
    ROWHOT.set(-1);
}

fn friendly_date(ft: u64) -> String {
    let s = date_text(ft);
    let mut now: windows_sys::Win32::Foundation::FILETIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime(&mut now) };
    let now = (now.dwHighDateTime as u64) << 32 | now.dwLowDateTime as u64;
    let (today, yest) = (date_text(now), date_text(now.saturating_sub(864_000_000_000)));
    match s.get(..10) {
        Some(d) if s.len() >= 16 && today.get(..10) == Some(d) => format!("Today, {}", &s[11..]),
        Some(d) if s.len() >= 16 && yest.get(..10) == Some(d) => format!("Yesterday, {}", &s[11..]),
        _ => s,
    }
}

fn items_text(k: usize) -> String {
    format!("{k} item{}", if k == 1 { "" } else { "s" })
}

pub unsafe fn list_draw(lp: LPARAM) -> LRESULT {
    let cd = &*(lp as *const NMLVCUSTOMDRAW);
    let list = cd.nmcd.hdr.hwndFrom;
    if ST.with(|c| c.try_borrow().map_or(true, |s| s.thumbs)) { return CDRF_DODEFAULT as isize; }
    let dc = cd.nmcd.hdc;
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(list, &mut cr);
    match cd.nmcd.dwDrawStage {
        CDDS_PREPAINT => (CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT) as isize,
        CDDS_POSTPAINT => {
            let cnt = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) as usize;
            if cnt == 0 {
                let hdr = SendMessageW(list, LVM_GETHEADER, 0, 0) as HWND;
                let mut hr: RECT = std::mem::zeroed();
                if !hdr.is_null() { GetClientRect(hdr, &mut hr); }
                SetDCBrushColor(dc, listbg());
                FillRect(dc, &rc(0, hr.bottom, cr.right, cr.bottom), GetStockObject(DC_BRUSH as i32) as HBRUSH);
                let mut ef = EMPTY_FONT.get();
                if ef == 0 {
                    ef = font(40, 400, winrust_common::style::icon_face());
                    EMPTY_FONT.set(ef);
                }
                if FONTS.get().0 == 0 { FONTS.set((font(14, 400, ui_face()), font(13, 400, ui_face()))); }
                SetBkMode(dc, TRANSPARENT as i32);
                let mid = cr.bottom / 3;
                let of = SelectObject(dc, ef as HFONT);
                text(dc, "\u{E8B7}", rc(0, mid - sc(40), cr.right, mid + sc(10)), mix(listbg(), pal().dim, 0.5), DT_CENTER);
                SelectObject(dc, FONTS.get().0 as HFONT);
                let msg = if SEARCHING.with_borrow(|s| s.is_some()) { "Nothing found" } else { "This folder is empty" };
                text(dc, msg, rc(0, mid + sc(14), cr.right, mid + sc(40)), pal().dim, DT_CENTER);
                SelectObject(dc, of);
            } else {
                let mut lr = RECT { left: LVIR_BOUNDS as i32, ..std::mem::zeroed() };
                if SendMessageW(list, LVM_GETITEMRECT, cnt - 1, &mut lr as *mut _ as isize) != 0 && lr.bottom < cr.bottom {
                    SetDCBrushColor(dc, listbg());
                    FillRect(dc, &rc(0, lr.bottom, cr.right, cr.bottom), GetStockObject(DC_BRUSH as i32) as HBRUSH);
                }
            }
            CDRF_DODEFAULT as isize
        }
        CDDS_ITEMPREPAINT => {
            let i = cd.nmcd.dwItemSpec;
            let row = ST.with(|c| {
                let mut st = c.try_borrow_mut().ok()?;
                let State { dir, rows, .. } = &mut *st;
                let r = rows.get_mut(i)?;
                if r.icon < 0 && !r.hdr { r.icon = icon_of(&dir.join(&r.name), r.dir); }
                Some((r.name.clone(), r.dir, r.time, r.size, r.icon, r.hdr))
            });
            let Some((name, is_dir, time, size, icon, hdr)) = row else { return CDRF_DODEFAULT as isize };
            let mut r = RECT { left: LVIR_BOUNDS as i32, ..std::mem::zeroed() };
            SendMessageW(list, LVM_GETITEMRECT, i, &mut r as *mut _ as isize);
            let row_r = rc(0, r.top, cr.right, r.bottom);
            SetDCBrushColor(dc, listbg());
            FillRect(dc, &row_r, GetStockObject(DC_BRUSH as i32) as HBRUSH);
            if hdr {
                let x0 = r.left;
                if FONTS.get().0 == 0 { FONTS.set((font(14, 400, ui_face()), font(13, 400, ui_face()))); }
                SetBkMode(dc, TRANSPARENT as i32);
                let of = SelectObject(dc, FONTS.get().0 as HFONT);
                text(dc, &name[1..], rc(x0 + sc(16), r.top + sc(8), cr.right, r.bottom), pal().accent, DT_LEFT);
                fill(dc, rc(sc(16), r.bottom - 1, cr.right - sc(16), r.bottom), pal().line, 0);
                SelectObject(dc, of);
                return CDRF_SKIPDEFAULT as isize;
            }
            let sel = SendMessageW(list, LVM_GETITEMSTATE, i, LVIS_SELECTED as isize) as u32 & LVIS_SELECTED != 0;
            let hot = ROWHOT.get() == i as isize;
            let focus = GetFocus() == list;
            let pill = rc(sc(6), r.top + sc(2), cr.right - sc(6), r.bottom - sc(2));
            let bg = match (sel, focus) {
                (true, true) => mix(listbg(), pal().accent, if hot { 0.24 } else { 0.18 }),
                (true, false) => mix(listbg(), pal().text, 0.10),
                _ if hot => mix(listbg(), pal().text, 0.055),
                _ => listbg(),
            };
            if bg != listbg() { fill(dc, pill, bg, sc(8)); }
            if sel { fill(dc, rc(pill.left, pill.top + sc(7), pill.left + sc(3), pill.bottom - sc(7)), if focus { pal().accent } else { pal().dim }, sc(4)); }
            let cw: Vec<i32> = (0..4).map(|k| SendMessageW(list, LVM_GETCOLUMNWIDTH, k, 0) as i32).collect();
            let x: Vec<i32> = (0..4).map(|k| r.left + cw[..k].iter().sum::<i32>()).collect();
            let himl = SendMessageW(list, LVM_GETIMAGELIST, LVSIL_SMALL as usize, 0);
            let (mut iw, mut ih) = (sc(16), sc(16));
            if himl != 0 {
                ImageList_GetIconSize(himl as _, &mut iw, &mut ih);
            }
            let cut = ST.with(|c| c.try_borrow().ok().is_some_and(|st| crate::is_cut_row(&st.dir, &name)));
            if himl != 0 && icon >= 0 {
                let ix = x[0] + sc(16);
                let iy = (r.top + r.bottom - ih) / 2;
                if cut {
                    ICON_BUF.with_borrow_mut(|b| {
                        let sdc = b.get(dc, iw, ih);
                        SetDCBrushColor(sdc, bg);
                        FillRect(sdc, &rc(0, 0, iw, ih), GetStockObject(DC_BRUSH as i32) as HBRUSH);
                        ImageList_Draw(himl as _, icon, sdc, 0, 0, ILD_TRANSPARENT);
                        GdiAlphaBlend(dc, ix, iy, iw, ih, sdc, 0, 0, iw, ih, BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 110, AlphaFormat: 0 });
                    });
                } else {
                    ImageList_Draw(himl as _, icon, dc, ix, iy, ILD_TRANSPARENT);
                }
            } else {
                let mut fg = LIST_ICO_FONT.get();
                if fg == 0 {
                    fg = font(15, 400, winrust_common::style::icon_face());
                    LIST_ICO_FONT.set(fg);
                }
                let old_f = SelectObject(dc, fg as HFONT);
                SetBkMode(dc, TRANSPARENT as i32);
                let gcol = if is_dir { 0x001E_B7F7 } else { pal().dim };
                let gcol = if cut { mix(bg, gcol, 0.45) } else { gcol };
                text(dc, if is_dir { "\u{E8B7}" } else { "\u{E8A5}" }, rc(x[0] + sc(14), r.top, x[0] + sc(14) + iw + sc(4), r.bottom), gcol, DT_CENTER);
                SelectObject(dc, old_f);
            }
            if FONTS.get().0 == 0 { FONTS.set((font(14, 400, ui_face()), font(13, 400, ui_face()))); }
            let (f_name, f_det) = FONTS.get();
            SetBkMode(dc, TRANSPARENT as i32);
            let of = SelectObject(dc, f_name as HFONT);
            let p = Path::new(&name);
            let is_this_pc = ST.with(|c| c.try_borrow().is_ok_and(|s| s.dir == Path::new(crate::THIS_PC_KEY)));
            let hit_of_search = p.is_absolute() && !is_this_pc;
            let drive_info = if is_this_pc && name.len() == 3 && name.ends_with(":\\") {
                let letter = name.chars().next().unwrap_or('C');
                let mut vol = [0u16; 64];
                let ok = windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW(w(&name).as_ptr(), vol.as_mut_ptr(), 64, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), 0);
                let vname = if ok != 0 { wstr(vol.as_ptr()) } else { String::new() };
                let base = if vname.is_empty() { "Local Disk" } else { &vname };
                let (mut free, mut total) = (0u64, 0u64);
                let has_cap = windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(w(&name).as_ptr(), &mut free, &mut total, ptr::null_mut()) != 0 && total > 0;
                Some((format!("{base} ({letter}:)"), has_cap.then_some((free, total))))
            } else {
                None
            };
            let shown = if let Some((dn, _)) = &drive_info {
                dn.clone()
            } else if p.is_absolute() {
                p.file_name().map_or(name.clone(), |n| n.to_string_lossy().into_owned())
            } else {
                name.clone()
            };
            text(dc, &shown, rc(x[0] + sc(16) + iw + sc(12), r.top, x[1] - sc(8), r.bottom), pal().text, DT_LEFT);
            SelectObject(dc, f_det as HFONT);
            let dim = if sel { mix(pal().dim, pal().text, 0.45) } else { pal().dim };
            if let Some((_, Some((free, total)))) = drive_info {
                let used = 1.0 - free as f32 / total as f32;
                let bar = rc(x[1] + sc(10), (r.top + r.bottom) / 2 - sc(4), (x[2] - sc(12)).max(x[1] + sc(50)), (r.top + r.bottom) / 2 + sc(4));
                fill(dc, bar, mix(listbg(), pal().text, 0.14), sc(4));
                let fw = ((bar.right - bar.left) as f32 * used) as i32;
                fill(dc, rc(bar.left, bar.top, bar.left + fw.max(sc(4)), bar.bottom), if used > 0.9 { winrust_common::style::BAD } else { pal().accent }, sc(4));
                text(dc, &format!("{} free of {}", fmt_cap(free), fmt_cap(total)), rc(x[2] + sc(10), r.top, x[3] - sc(8), r.bottom), dim, DT_LEFT);
            } else {
                text(dc, &friendly_date(time), rc(x[1] + sc(10), r.top, x[2] - sc(8), r.bottom), dim, DT_LEFT);
                let kind = if hit_of_search { p.parent().map_or(String::new(), |d| d.to_string_lossy().into_owned()) } else { type_text(&name, is_dir) };
                text(dc, &kind, rc(x[2] + sc(10), r.top, x[3] - sc(8), r.bottom), dim, DT_LEFT | DT_PATH_ELLIPSIS);
                if !is_dir { text(dc, &size_text(size), rc(x[3], r.top, x[3] + cw[3] - sc(14), r.bottom), dim, DT_RIGHT); }
            }
            SelectObject(dc, of);
            CDRF_SKIPDEFAULT as isize
        }
        _ => CDRF_DODEFAULT as isize,
    }
}

pub unsafe fn fit_columns(list: HWND) {
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(list, &mut cr);
    let w = (cr.right - sc(4)).max(0);
    if w <= 0 { return; }
    let (c1, c2, c3) = if w >= sc(560) {
        let c1 = (w * 20 / 100).clamp(sc(120), sc(170));
        let c2 = (w * 17 / 100).clamp(sc(100), sc(150));
        let c3 = (w * 12 / 100).clamp(sc(72), sc(105));
        (c1, c2, c3)
    } else {
        (w * 22 / 100, w * 18 / 100, w * 14 / 100)
    };
    let c0 = (w - c1 - c2 - c3).max(sc(80).min(w));
    let sum = c0 + c1 + c2 + c3;
    let c0 = if sum > w { (c0 - (sum - w)).max(0) } else { c0 };
    SendMessageW(list, WM_SETREDRAW, 0, 0);
    SendMessageW(list, LVM_SETCOLUMNWIDTH, 0, c0 as isize);
    SendMessageW(list, LVM_SETCOLUMNWIDTH, 1, c1 as isize);
    SendMessageW(list, LVM_SETCOLUMNWIDTH, 2, c2 as isize);
    SendMessageW(list, LVM_SETCOLUMNWIDTH, 3, c3 as isize);
    if ST.with(|c| c.try_borrow().is_ok_and(|s| !s.thumbs)) {
        ShowScrollBar(list, 0, 0);
    }
    SendMessageW(list, WM_SETREDRAW, 1, 0);
}

unsafe fn redraw_row(list: HWND, i: isize) {
    if i < 0 { return; }
    let mut r = RECT { left: LVIR_BOUNDS as i32, ..std::mem::zeroed() };
    if SendMessageW(list, LVM_GETITEMRECT, i as usize, &mut r as *mut _ as isize) != 0 {
        let mut cr: RECT = std::mem::zeroed();
        GetClientRect(list, &mut cr);
        InvalidateRect(list, &rc(0, r.top, cr.right, r.bottom), 0);
    }
}

fn this_pc_layout(cw: i32, rows: &[Row]) -> (Vec<(RECT, usize, bool)>, i32) {
    let cols = ((cw - sc(32)) / sc(260)).clamp(1, 4);
    let gap = sc(12);
    let tw = ((cw - sc(32) - (cols - 1) * gap) / cols).max(sc(180));
    let th = sc(68);
    let mut out = Vec::with_capacity(rows.len());
    let mut y = sc(10);
    let mut col = 0;
    let mut in_grid = false;
    let mut hidden = false;
    PC_COLLAPSED.with_borrow(|collapsed| {
        for (i, r) in rows.iter().enumerate() {
            if r.hdr {
                if in_grid && col > 0 {
                    y += th + sc(8);
                }
                if i > 0 {
                    y += sc(8);
                }
                out.push((rc(sc(16), y, (cw - sc(16)).max(sc(100)), y + sc(32)), i, true));
                y += sc(36);
                col = 0;
                in_grid = false;
                hidden = collapsed.contains(&pc_sec_key(&r.name));
            } else if !hidden {
                let x = sc(16) + col * (tw + gap);
                out.push((rc(x, y, x + tw, y + th), i, false));
                col += 1;
                if col >= cols {
                    col = 0;
                    y += th + sc(8);
                }
                in_grid = true;
            }
        }
    });
    if in_grid && col > 0 {
        y += th + sc(8);
    }
    (out, y + sc(12))
}

fn pc_hit(h: HWND, x: i32, y: i32) -> Option<(usize, bool)> {
    let mut cr: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(h, &mut cr) };
    let scroll = PC_SCROLL.get();
    ST.with(|c| {
        c.try_borrow().ok().and_then(|s| {
            this_pc_layout(cr.right, &s.rows)
                .0
                .into_iter()
                .find(|(r, _, _)| x >= r.left && x < r.right && y + scroll >= r.top && y + scroll < r.bottom)
                .map(|(_, i, hdr)| (i, hdr))
        })
    })
}

fn pc_max_scroll(h: HWND) -> i32 {
    let mut cr: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(h, &mut cr) };
    ST.with(|c| c.try_borrow().map_or(0, |s| (this_pc_layout(cr.right, &s.rows).1 - cr.bottom).max(0)))
}

unsafe fn paint_this_pc(list: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let wdc = BeginPaint(list, &mut ps);
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(list, &mut cr);
    let dc = PC_BUF.with_borrow_mut(|b| b.get(wdc, cr.right, cr.bottom));
    SetDCBrushColor(dc, listbg());
    FillRect(dc, &cr, GetStockObject(DC_BRUSH as i32) as HBRUSH);
    SetBkMode(dc, TRANSPARENT as i32);
    let mut pf = PC_FONTS.get();
    if pf[0] == 0 {
        pf = [
            font(14, 600, ui_face()),
            font(13, 400, ui_face()),
            font(12, 400, ui_face()),
            font(12, 400, winrust_common::style::icon_face()),
        ];
        PC_FONTS.set(pf);
    }
    let [f_hdr, f_txt, f_sub, f_ico] = [pf[0] as HFONT, pf[1] as HFONT, pf[2] as HFONT, pf[3] as HFONT];
    let of = SelectObject(dc, f_txt);
    let hot = ROWHOT.get();
    let sel = PC_SEL.get();
    let mut il_xl = IL_XL.get();
    if il_xl == 0 {
        il_xl = windows::Win32::UI::Shell::SHGetImageList::<windows::Win32::UI::Controls::IImageList>(windows::Win32::UI::Shell::SHIL_EXTRALARGE as i32)
            .ok()
            .map(|l| windows::core::Interface::as_raw(&l) as isize)
            .unwrap_or(0);
        IL_XL.set(il_xl);
    }
    let isz = sc(44);
    ST.with(|c| {
        let Ok(st) = c.try_borrow() else { return };
        let (items, tot_h) = this_pc_layout(cr.right, &st.rows);
        let max_s = (tot_h - cr.bottom).max(0);
        let scroll = PC_SCROLL.get().clamp(0, max_s);
        PC_SCROLL.set(scroll);
        for (r0, idx, is_hdr) in items {
            let r = rc(r0.left, r0.top - scroll, r0.right, r0.bottom - scroll);
            if r.bottom < 0 || r.top > cr.bottom {
                continue;
            }
            let row = &st.rows[idx];
            if is_hdr {
                if hot == idx as isize {
                    fill(dc, r, mix(listbg(), pal().text, 0.05), sc(6));
                }
                let collapsed = PC_COLLAPSED.with_borrow(|c| c.contains(&pc_sec_key(&row.name)));
                SelectObject(dc, f_ico);
                text(dc, if collapsed { "\u{E76C}" } else { "\u{E70D}" }, rc(r.left + sc(4), r.top, r.left + sc(22), r.bottom), pal().dim, DT_CENTER);
                SelectObject(dc, f_hdr);
                text(dc, &row.name[1..], rc(r.left + sc(26), r.top, r.right - sc(8), r.bottom), pal().text, DT_LEFT);
                continue;
            }
            let is_hot = hot == idx as isize;
            let is_sel = sel == idx as isize;
            if is_sel || is_hot {
                fill(dc, r, if is_sel { mix(listbg(), pal().accent, 0.20) } else { mix(listbg(), pal().text, 0.06) }, sc(8));
            }
            let (title, cap, ico_idx) = PC_INFO.with_borrow_mut(|m| {
                m.entry(row.name.clone()).or_insert_with(|| {
                    let mut fi: SHFILEINFOW = std::mem::zeroed();
                    let ic = if SHGetFileInfoW(w(&row.name).as_ptr(), 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX) != 0 { fi.iIcon } else { -1 };
                    if row.name.len() == 3 && row.name.ends_with(":\\") {
                        let letter = row.name.chars().next().unwrap_or('C');
                        let mut vol = [0u16; 64];
                        let ok = windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW(w(&row.name).as_ptr(), vol.as_mut_ptr(), 64, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), 0);
                        let vname = if ok != 0 { wstr(vol.as_ptr()) } else { String::new() };
                        let base = if vname.is_empty() { "Local Disk" } else { &vname };
                        let (mut free, mut total) = (0u64, 0u64);
                        let cap = (windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(w(&row.name).as_ptr(), &mut free, &mut total, ptr::null_mut()) != 0 && total > 0).then_some((free, total));
                        (format!("{base} ({letter}:)"), cap, ic)
                    } else {
                        let title = Path::new(&row.name).file_name().map_or_else(|| row.name.clone(), |n| n.to_string_lossy().into_owned());
                        (title, None, ic)
                    }
                }).clone()
            });
            if il_xl != 0 && ico_idx >= 0 {
                ImageList_Draw(il_xl as _, ico_idx, dc, r.left + sc(10), (r.top + r.bottom - isz) / 2, ILD_TRANSPARENT);
            }
            let tx = r.left + sc(16) + isz;
            if let Some((free, total)) = cap {
                SelectObject(dc, f_txt);
                text(dc, &title, rc(tx, r.top + sc(6), r.right - sc(10), r.top + sc(24)), pal().text, DT_LEFT);
                let used = 1.0 - free as f32 / total as f32;
                let bar = rc(tx, r.top + sc(28), r.right - sc(14), r.top + sc(38));
                fill(dc, bar, mix(listbg(), pal().text, 0.16), sc(4));
                let fw = ((bar.right - bar.left) as f32 * used) as i32;
                fill(dc, rc(bar.left, bar.top, bar.left + fw.max(sc(4)), bar.bottom), if used > 0.9 { winrust_common::style::BAD } else { pal().accent }, sc(4));
                SelectObject(dc, f_sub);
                text(dc, &format!("{} free of {}", fmt_cap(free), fmt_cap(total)), rc(tx, r.top + sc(41), r.right - sc(10), r.bottom - sc(4)), pal().dim, DT_LEFT);
            } else {
                SelectObject(dc, f_txt);
                text(dc, &title, rc(tx, r.top, r.right - sc(10), r.bottom), pal().text, DT_LEFT);
            }
        }
        if max_s > 0 && cr.bottom > sc(24) {
            let tot = tot_h.max(1);
            let th = ((cr.bottom as f32 * cr.bottom as f32 / tot as f32) as i32).clamp(sc(24), cr.bottom - sc(8));
            let ty = ((cr.bottom - th - sc(8)) as f32 * (scroll as f32 / max_s as f32)) as i32 + sc(4);
            fill(dc, rc(cr.right - sc(5), ty, cr.right - sc(2), ty + th), mix(listbg(), pal().text, 0.28), sc(3));
        }
    });
    BitBlt(wdc, 0, 0, cr.right, cr.bottom, dc, 0, 0, SRCCOPY);
    SelectObject(dc, of);
    EndPaint(list, &ps);
}

unsafe fn edge_hit(h: HWND, lp: LPARAM) -> bool {
    let p = GetParent(h);
    if p.is_null() || IsZoomed(p) != 0 { return false; }
    let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
    let mut wr: RECT = std::mem::zeroed();
    GetWindowRect(p, &mut wr);
    let b = sc(6);
    x < wr.left + b || x >= wr.right - b || y < wr.top + b || y >= wr.bottom - b
}

unsafe extern "system" fn list_sub(h: HWND, m: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if m == WM_NCHITTEST && !crate::desk() && edge_hit(h, lp) { return HTTRANSPARENT as isize; }
    let is_pc = ST.with(|c| c.try_borrow().is_ok_and(|s| s.dir == Path::new(crate::THIS_PC_KEY)));
    if is_pc {
        let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
        match m {
            WM_ERASEBKGND => return 1,
            WM_PAINT => { paint_this_pc(h); return 0; }
            WM_MOUSEMOVE => {
                let hit = pc_hit(h, x, y).map_or(-1, |(i, _)| i as isize);
                if ROWHOT.replace(hit) != hit {
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    TrackMouseEvent(&mut tme);
                    InvalidateRect(h, ptr::null(), 0);
                }
                return 0;
            }
            WM_MOUSELEAVE => { ROWHOT.set(-1); InvalidateRect(h, ptr::null(), 0); return 0; }
            WM_LBUTTONDOWN => {
                SetFocus(h);
                match pc_hit(h, x, y) {
                    Some((idx, true)) => {
                        let key = ST.with(|c| c.try_borrow().ok().and_then(|s| s.rows.get(idx).map(|r| pc_sec_key(&r.name))));
                        if let Some(k) = key {
                            PC_COLLAPSED.with_borrow_mut(|c| {
                                if !c.remove(&k) { c.insert(k); }
                            });
                            let next_scroll = PC_SCROLL.get().clamp(0, pc_max_scroll(h));
                            PC_SCROLL.set(next_scroll);
                            let sel = PC_SEL.get();
                            if sel >= 0 {
                                let mut cr: RECT = std::mem::zeroed();
                                GetClientRect(h, &mut cr);
                                let still = ST.with(|c| c.try_borrow().ok().is_some_and(|s| this_pc_layout(cr.right, &s.rows).0.iter().any(|(_, i, hdr)| !hdr && *i as isize == sel)));
                                if !still { PC_SEL.set(-1); }
                            }
                        }
                    }
                    Some((idx, false)) => { PC_SEL.set(idx as isize); }
                    None => { PC_SEL.set(-1); }
                }
                InvalidateRect(h, ptr::null(), 0);
                status_dirty(GetParent(h));
                return 0;
            }
            WM_LBUTTONDBLCLK => {
                if let Some((i, false)) = pc_hit(h, x, y) {
                    PostMessageW(GetParent(h), WM_UI, 22, i as isize);
                }
                return 0;
            }
            WM_RBUTTONDOWN => {
                SetFocus(h);
                match pc_hit(h, x, y) {
                    Some((idx, false)) => { PC_SEL.set(idx as isize); }
                    _ => { PC_SEL.set(-1); }
                }
                InvalidateRect(h, ptr::null(), 0);
                status_dirty(GetParent(h));
                return 0;
            }
            WM_RBUTTONUP => {
                match pc_hit(h, x, y) {
                    Some((idx, false)) => { PC_SEL.set(idx as isize); }
                    _ => { PC_SEL.set(-1); }
                }
                InvalidateRect(h, ptr::null(), 0);
                status_dirty(GetParent(h));
                PostMessageW(GetParent(h), WM_UI, 23, 0);
                return 0;
            }
            WM_CONTEXTMENU => return 0,
            WM_MOUSEWHEEL => {
                let delta = ((wp >> 16) as u16) as i16 as i32;
                let step = (-delta * sc(48)) / 120;
                let next = (PC_SCROLL.get() + step).clamp(0, pc_max_scroll(h));
                if PC_SCROLL.replace(next) != next {
                    InvalidateRect(h, ptr::null(), 0);
                }
                return 0;
            }
            WM_SIZE => {
                let next = PC_SCROLL.get().clamp(0, pc_max_scroll(h));
                PC_SCROLL.set(next);
            }
            WM_KEYDOWN if wp == 0x0D => {
                let s = PC_SEL.get();
                if s >= 0 {
                    PostMessageW(GetParent(h), WM_UI, 22, s);
                }
                return 0;
            }
            _ => {}
        }
    }
    if m == WM_CONTEXTMENU { return 0; }
    if m == WM_ERASEBKGND && ST.with(|c| c.try_borrow().map_or(false, |s| !s.thumbs)) {
        let mut cr: RECT = std::mem::zeroed();
        GetClientRect(h, &mut cr);
        let dc = wp as HDC;
        SetDCBrushColor(dc, listbg());
        FillRect(dc, &cr, GetStockObject(DC_BRUSH as i32) as HBRUSH);
        return 1;
    }
    if m == WM_MOUSEMOVE {
        let mut ht: LVHITTESTINFO = std::mem::zeroed();
        ht.pt = POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 };
        let i = SendMessageW(h, LVM_HITTEST, 0, &mut ht as *mut _ as isize);
        let old = ROWHOT.replace(i);
        if old != i {
            redraw_row(h, old);
            redraw_row(h, i);
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
        }
    } else if m == WM_MOUSELEAVE {
        redraw_row(h, ROWHOT.replace(-1));
    } else if m == WM_SETFOCUS || m == WM_KILLFOCUS {
        InvalidateRect(h, ptr::null(), 0);
    } else if m == WM_SIZE && !crate::desk() {
        let r = DefSubclassProc(h, m, wp, lp);
        fit_columns(h);
        return r;
    }
    if m == WM_NOTIFY {
        let nm = &*(lp as *const NMCUSTOMDRAW);
        let hdr = SendMessageW(h, LVM_GETHEADER, 0, 0) as HWND;
        if nm.hdr.hwndFrom == hdr && nm.hdr.code == NM_CUSTOMDRAW {
            return match nm.dwDrawStage {
                CDDS_PREPAINT => {
                    SetDCBrushColor(nm.hdc, listbg());
                    FillRect(nm.hdc, &nm.rc, GetStockObject(DC_BRUSH as i32) as HBRUSH);
                    CDRF_NOTIFYITEMDRAW as isize
                }
                CDDS_ITEMPREPAINT => {
                    let (dc, r, i) = (nm.hdc, nm.rc, nm.dwItemSpec);
                    SetDCBrushColor(dc, listbg());
                    FillRect(dc, &r, GetStockObject(DC_BRUSH as i32) as HBRUSH);
                    fill(dc, rc(r.right - 1, r.top + sc(10), r.right, r.bottom - sc(10)), mix(listbg(), pal().text, 0.05), 0);
                    let mut buf = [0u16; 128];
                    let mut it: HDITEMW = std::mem::zeroed();
                    it.mask = HDI_TEXT;
                    it.pszText = buf.as_mut_ptr();
                    it.cchTextMax = 128;
                    SendMessageW(hdr, HDM_GETITEMW, i, &mut it as *mut _ as isize);
                    let sorted = SORT_COL.load(Relaxed) == i;
                    let arrow = if !sorted { "" } else if SORT_ASC.load(Relaxed) { "  \u{2191}" } else { "  \u{2193}" };
                    let mut hf = HDR_FONTS.get();
                    if hf[0] == 0 {
                        hf = [font(13, 400, ui_face()), font(13, 600, ui_face())];
                        HDR_FONTS.set(hf);
                    }
                    let f = if sorted { hf[1] } else { hf[0] } as HFONT;
                    let of = SelectObject(dc, f);
                    SetBkMode(dc, TRANSPARENT as i32);
                    let align = if i == 3 { DT_RIGHT } else { DT_LEFT };
                    text(dc, &format!("{}{arrow}", wstr(buf.as_ptr())), rc(r.left + sc(10), r.top, r.right - sc(12), r.bottom), if sorted { pal().text } else { pal().dim }, align);
                    SelectObject(dc, of);
                    CDRF_SKIPDEFAULT as isize
                }
                _ => 0,
            };
        }
    }
    DefSubclassProc(h, m, wp, lp)
}

struct Layout { tabs: Vec<RECT>, close: Vec<RECT>, plus: RECT, cap_min: RECT, cap_max: RECT, cap_close: RECT, back: RECT, fwd: RECT, up: RECT, reload: RECT, pill: RECT, search: RECT, new_btn: RECT, cmds: Vec<RECT>, sort_btn: RECT, view: RECT, more_btn: RECT }

fn rc(l: i32, t: i32, r: i32, b: i32) -> RECT { RECT { left: l, top: t, right: r, bottom: b } }

fn layout(cw: i32, n: usize) -> Layout {
    let cap_w = sc(44);
    let cap_close = rc(cw - cap_w, 0, cw, sc(36));
    let cap_max = rc(cw - cap_w * 2, 0, cw - cap_w, sc(36));
    let cap_min = rc(cw - cap_w * 3, 0, cw - cap_w * 2, sc(36));
    let (ty0, ty1) = (sc(6), sc(38));
    let tw = ((cw - cap_w * 3 - sc(52)) / n.max(1) as i32 - sc(4)).clamp(sc(96), sc(230));
    let tabs: Vec<RECT> = (0..n as i32).map(|i| { let l = sc(8) + i * (tw + sc(4)); rc(l, ty0, l + tw, ty1) }).collect();
    let close = tabs.iter().map(|t| rc(t.right - sc(28), (t.top + t.bottom) / 2 - sc(10), t.right - sc(8), (t.top + t.bottom) / 2 + sc(10))).collect();
    let px = tabs.last().map_or(sc(8), |t| t.right + sc(4));
    let plus = rc(px, ty0 + sc(3), px + sc(28), ty1 - sc(3));

    let (ny0, ny1, bs) = (sc(43), sc(75), sc(32));
    let nav_btn = |i: i32| rc(sc(10) + i * (bs + sc(2)), ny0, sc(10) + i * (bs + sc(2)) + bs, ny1);
    let reload = nav_btn(3);
    let sw = if cw >= sc(820) { sc(220) } else { bs };
    let search = rc(cw - sc(12) - sw, ny0, cw - sc(12), ny1);
    let pill = rc(reload.right + sc(10), ny0, search.left - sc(10), ny1);

    let (cy0, cy1, cbw) = (sc(85), sc(117), sc(34));
    let wide_cmd = cw >= sc(640);
    let nw = if wide_cmd { sc(82) } else { cbw };
    let new_btn = rc(sc(10), cy0, sc(10) + nw, cy1);
    let cmd_x0 = new_btn.right + sc(12);
    let sw_btn = if wide_cmd { sc(82) } else { cbw };
    let vw_btn = if wide_cmd { sc(82) } else { cbw };
    let more_btn = rc(cw - sc(12) - cbw, cy0, cw - sc(12), cy1);
    let view = rc(more_btn.left - sc(4) - vw_btn, cy0, more_btn.left - sc(4), cy1);
    let sort_btn = rc(view.left - sc(4) - sw_btn, cy0, view.left - sc(4), cy1);
    let cmds: Vec<RECT> = (0..5)
        .map(|i| {
            let l = cmd_x0 + i * (cbw + sc(4));
            rc(l, cy0, l + cbw, cy1)
        })
        .filter(|r| r.right + sc(8) <= sort_btn.left)
        .collect();

    Layout { tabs, close, plus, cap_min, cap_max, cap_close, back: nav_btn(0), fwd: nav_btn(1), up: nav_btn(2), reload, pill, search, new_btn, cmds, sort_btn, view, more_btn }
}

fn inside(r: &RECT, x: i32, y: i32) -> bool { x >= r.left && x < r.right && y >= r.top && y < r.bottom }

fn hit(l: &Layout, x: i32, y: i32) -> i32 {
    for (k, r) in [(WIN_CLOSE, &l.cap_close), (WIN_MAX, &l.cap_max), (WIN_MIN, &l.cap_min)] {
        if inside(r, x, y) { return k; }
    }
    for (i, c) in l.close.iter().enumerate() { if inside(c, x, y) && l.tabs.len() > 1 { return CLOSE + i as i32; } }
    for (i, t) in l.tabs.iter().enumerate() { if inside(t, x, y) { return TAB + i as i32; } }
    for (k, r) in [(BACK, &l.back), (FWD, &l.fwd), (UP, &l.up), (RELOAD, &l.reload), (SEARCH, &l.search), (NEW_BTN, &l.new_btn), (SORT_BTN, &l.sort_btn), (VIEW, &l.view), (MORE_BTN, &l.more_btn), (PLUS, &l.plus)] {
        if inside(r, x, y) { return k; }
    }
    for (i, r) in l.cmds.iter().enumerate() { if inside(r, x, y) { return CMD + i as i32; } }
    NONE
}

pub fn place_edit(chrome: HWND) {
    unsafe {
        let edit = ST.with(|c| c.try_borrow().map_or(ptr::null_mut(), |s| s.edit));
        if edit.is_null() { return; }
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(chrome, &mut r);
        let p = layout(r.right, 1).pill;
        let eh = sc(20);
        MoveWindow(edit, p.left + sc(40), (p.top + p.bottom - eh) / 2, (p.right - p.left - sc(40) - sc(16)).max(0), eh, 1);
        if GetFocus() != edit { ShowWindow(edit, SW_HIDE); }
    }
}

pub fn gdiplus_start() {
    use windows_sys::Win32::Graphics::GdiPlus::{GdiplusStartup, GdiplusStartupInput};
    unsafe {
        let (mut tok, mut inp): (usize, GdiplusStartupInput) = (0, std::mem::zeroed());
        inp.GdiplusVersion = 1;
        GdiplusStartup(&mut tok, &inp, ptr::null_mut());
    }
}

unsafe fn fill(dc: HDC, r: RECT, c: u32, rad: i32) {
    let rad = if winrust_common::style::style().square() { 0 } else { rad };
    use windows_sys::Win32::Graphics::GdiPlus::*;
    let (wd, ht) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
    if wd <= 0.0 || ht <= 0.0 { return; }
    let mut g: *mut GpGraphics = ptr::null_mut();
    if rad <= 0 || GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
        SetDCBrushColor(dc, c);
        let b = GetStockObject(DC_BRUSH as i32) as HBRUSH;
        if rad <= 0 {
            FillRect(dc, &r, b);
        } else {
            let (ob, op) = (SelectObject(dc, b as HGDIOBJ), SelectObject(dc, GetStockObject(NULL_PEN)));
            RoundRect(dc, r.left, r.top, r.right + 1, r.bottom + 1, rad, rad);
            SelectObject(dc, ob);
            SelectObject(dc, op);
        }
        return;
    }
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
    GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
    let (x, y, d) = (r.left as f32, r.top as f32, (rad as f32).min(wd).min(ht));
    let mut path: *mut GpPath = ptr::null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(path, x + wd - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(path, x + wd - d, y + ht - d, d, d, 0.0, 90.0);
    GdipAddPathArc(path, x, y + ht - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    let mut br: *mut GpSolidFill = ptr::null_mut();
    GdipCreateSolidFill(0xFF00_0000 | (c & 255) << 16 | (c & 0xFF00) | (c >> 16 & 255), &mut br);
    GdipFillPath(g, br as *mut GpBrush, path);
    GdipDeleteBrush(br as *mut GpBrush);
    GdipDeletePath(path);
    GdipDeleteGraphics(g);
}

unsafe fn text(dc: HDC, s: &str, mut r: RECT, c: u32, flags: u32) {
    if s.is_empty() { return; }
    SetTextColor(dc, c);
    WBUF.with_borrow_mut(|b| {
        b.clear();
        b.extend(s.encode_utf16());
        DrawTextW(dc, b.as_ptr(), b.len() as i32, &mut r, flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
    });
}

unsafe fn glow(dc: HDC, key: i32, r: RECT, base: u32, to: u32, rad: i32, on: bool) {
    let (t, p) = (hv(key, on), press_t(key));
    if t > 0.01 || p > 0.0 { fill(dc, r, mix(mix(base, to, t), pal().accent, 0.45 * p), rad); }
}

unsafe fn paint_chrome(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let wdc = BeginPaint(hwnd, &mut ps);
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut cr);
    let snap = ST.with(|c| c.try_borrow().ok().map(|s| (s.tabs.iter().map(label).collect::<Vec<_>>(), s.active, s.hist.pos > 0, s.hist.pos + 1 < s.hist.v.len(), s.thumbs, s.edit, s.dir.clone())));
    let Some((labels, active, can_back, can_fwd, thumbs, edit, dir)) = snap else { EndPaint(hwnd, &ps); return };
    BUSY.set(false);
    let dc = CHROME_BUF.with_borrow_mut(|b| b.get(wdc, cr.right, cr.bottom));
    SetDCBrushColor(dc, pal().bg);
    FillRect(dc, &cr, GetStockObject(DC_BRUSH as i32) as HBRUSH);
    winrust_common::style::backdrop(dc, cr, sc(100) as f32 / 100.0);
    let tb = toolbar_bg();
    let cb = cmd_bg();
    let row2 = rc(0, sc(38), cr.right, sc(80));
    let row3 = rc(0, sc(80), cr.right, cr.bottom);
    fill(dc, row2, tb, 0);
    fill(dc, row3, cb, 0);
    SetBkMode(dc, TRANSPARENT as i32);
    let l = layout(cr.right, labels.len());
    let hot = HOT.get();
    let mut cf = CHROME_FONTS.get();
    if cf[0] == 0 {
        cf = [font(13, 400, ui_face()), font(15, 400, winrust_common::style::icon_face()), font(12, 400, winrust_common::style::icon_face())];
        CHROME_FONTS.set(cf);
    }
    let [f_txt, f_glyph, f_small] = [cf[0] as HFONT, cf[1] as HFONT, cf[2] as HFONT];
    let of = SelectObject(dc, f_txt);

    for (i, t) in l.tabs.iter().enumerate() {
        let on = i == active;
        let (th, ta) = (hv(TAB + i as i32, hot == TAB + i as i32 || hot == CLOSE + i as i32), hv(1000 + i as i32, on));
        let p = press_t(TAB + i as i32);
        let tab_col = if on { tb } else { mix(mix(pal().bg, pal().hover, th), pal().accent, 0.20 * p) };
        if on || th > 0.01 || p > 0.0 {
            fill(dc, *t, tab_col, sc(14));
            if on {
                fill(dc, rc(t.left, t.bottom - sc(8), t.right, t.bottom), tab_col, 0);
            }
        } else if i + 1 != active && i + 1 < l.tabs.len() {
            fill(dc, rc(t.right + sc(1), t.top + sc(7), t.right + sc(2), t.bottom - sc(7)), mix(pal().bg, pal().text, 0.10), 0);
        }
        SelectObject(dc, f_small);
        text(dc, "\u{E8B7}", rc(t.left + sc(12), t.top, t.left + sc(28), t.bottom), 0x001E_B7F7, DT_CENTER);
        SelectObject(dc, f_txt);
        text(dc, &labels[i], rc(t.left + sc(32), t.top, t.right - sc(if labels.len() > 1 { 32 } else { 12 }), t.bottom), mix(pal().dim, pal().text, ta.max(th)), DT_LEFT);
        if labels.len() > 1 && (on || th > 0.05) {
            let c = l.close[i];
            glow(dc, CLOSE + i as i32, c, tab_col, pal().hover, sc(8), hot == CLOSE + i as i32);
            SelectObject(dc, f_small);
            text(dc, "\u{E711}", c, mix(pal().dim, pal().text, if hot == CLOSE + i as i32 { 1.0 } else { 0.45 }), DT_CENTER);
        }
    }
    glow(dc, PLUS, l.plus, pal().bg, pal().hover, sc(8), hot == PLUS);
    SelectObject(dc, f_small);
    text(dc, "\u{E710}", l.plus, pal().dim, DT_CENTER);
    let zoomed = IsZoomed(GetParent(hwnd)) != 0;
    for (k, r, g, red) in [(WIN_MIN, l.cap_min, "\u{E921}", false), (WIN_MAX, l.cap_max, if zoomed { "\u{E923}" } else { "\u{E922}" }, false), (WIN_CLOSE, l.cap_close, "\u{E8BB}", true)] {
        if red {
            glow(dc, k, r, pal().bg, 0x0023_11E8, 0, hot == k);
        } else {
            glow(dc, k, r, pal().bg, pal().hover, 0, hot == k);
        }
        text(dc, g, r, if red && hot == k { 0x00FF_FFFF } else { pal().text }, DT_CENTER);
    }

    SelectObject(dc, f_glyph);
    for (k, r, g, enabled) in [(BACK, l.back, "\u{E72B}", can_back), (FWD, l.fwd, "\u{E72A}", can_fwd), (UP, l.up, "\u{E74A}", true), (RELOAD, l.reload, "\u{E72C}", true)] {
        if enabled { glow(dc, k, r, tb, pal().hover, sc(8), hot == k); }
        text(dc, g, r, if enabled { pal().text } else { off() }, DT_CENTER);
    }

    let s_hov = hv(SEARCH, hot == SEARCH);
    fill(dc, l.search, mix(pal().line, pal().accent, s_hov * 0.5), sc(10));
    fill(dc, rc(l.search.left + 1, l.search.top + 1, l.search.right - 1, l.search.bottom - 1), mix(pill(), pal().hover, s_hov * 0.4), sc(8));
    if l.search.right - l.search.left > sc(80) {
        SelectObject(dc, f_txt);
        let cur_name = label(&dir);
        let stxt = format!("Search {}", cur_name);
        text(dc, &stxt, rc(l.search.left + sc(12), l.search.top, l.search.right - sc(32), l.search.bottom), pal().dim, DT_LEFT);
        SelectObject(dc, f_glyph);
        text(dc, "\u{E721}", rc(l.search.right - sc(32), l.search.top, l.search.right - sc(4), l.search.bottom), pal().dim, DT_CENTER);
    } else {
        SelectObject(dc, f_glyph);
        text(dc, "\u{E721}", l.search, pal().text, DT_CENTER);
    }

    fill(dc, rc(0, sc(80), cr.right, sc(81)), mix(pal().bg, pal().text, 0.06), 0);
    glow(dc, NEW_BTN, l.new_btn, cb, pal().hover, sc(8), hot == NEW_BTN);
    if l.new_btn.right - l.new_btn.left > sc(50) {
        SelectObject(dc, f_glyph);
        text(dc, "\u{E710}", rc(l.new_btn.left + sc(6), l.new_btn.top, l.new_btn.left + sc(26), l.new_btn.bottom), pal().accent, DT_CENTER);
        SelectObject(dc, f_txt);
        text(dc, "New", rc(l.new_btn.left + sc(28), l.new_btn.top, l.new_btn.right - sc(18), l.new_btn.bottom), pal().text, DT_LEFT);
        SelectObject(dc, f_small);
        text(dc, "\u{E70D}", rc(l.new_btn.right - sc(18), l.new_btn.top, l.new_btn.right - sc(4), l.new_btn.bottom), pal().dim, DT_CENTER);
    } else {
        SelectObject(dc, f_glyph);
        text(dc, "\u{E710}", l.new_btn, pal().accent, DT_CENTER);
    }
    fill(dc, rc(l.new_btn.right + sc(5), l.new_btn.top + sc(6), l.new_btn.right + sc(6), l.new_btn.bottom - sc(6)), mix(pal().bg, pal().text, 0.10), 0);
    SelectObject(dc, f_glyph);
    for (i, r) in l.cmds.iter().enumerate() {
        glow(dc, CMD + i as i32, *r, cb, pal().hover, sc(8), hot == CMD + i as i32);
        text(dc, CMD_GLYPHS[i], *r, pal().text, DT_CENTER);
    }
    glow(dc, SORT_BTN, l.sort_btn, cb, pal().hover, sc(8), hot == SORT_BTN);
    if l.sort_btn.right - l.sort_btn.left > sc(50) {
        SelectObject(dc, f_glyph);
        text(dc, "\u{E8CB}", rc(l.sort_btn.left + sc(6), l.sort_btn.top, l.sort_btn.left + sc(28), l.sort_btn.bottom), pal().text, DT_CENTER);
        SelectObject(dc, f_txt);
        text(dc, "Sort", rc(l.sort_btn.left + sc(30), l.sort_btn.top, l.sort_btn.right - sc(18), l.sort_btn.bottom), pal().text, DT_LEFT);
        SelectObject(dc, f_small);
        text(dc, "\u{E70D}", rc(l.sort_btn.right - sc(18), l.sort_btn.top, l.sort_btn.right - sc(4), l.sort_btn.bottom), pal().dim, DT_CENTER);
    } else {
        SelectObject(dc, f_glyph);
        text(dc, "\u{E8CB}", l.sort_btn, pal().text, DT_CENTER);
    }
    glow(dc, VIEW, l.view, cb, pal().hover, sc(8), hot == VIEW);
    if l.view.right - l.view.left > sc(50) {
        SelectObject(dc, f_glyph);
        text(dc, if thumbs { "\u{E8FD}" } else { "\u{E80A}" }, rc(l.view.left + sc(6), l.view.top, l.view.left + sc(28), l.view.bottom), pal().text, DT_CENTER);
        SelectObject(dc, f_txt);
        text(dc, "View", rc(l.view.left + sc(30), l.view.top, l.view.right - sc(18), l.view.bottom), pal().text, DT_LEFT);
        SelectObject(dc, f_small);
        text(dc, "\u{E70D}", rc(l.view.right - sc(18), l.view.top, l.view.right - sc(4), l.view.bottom), pal().dim, DT_CENTER);
    } else {
        SelectObject(dc, f_glyph);
        text(dc, if thumbs { "\u{E8FD}" } else { "\u{E80A}" }, l.view, pal().text, DT_CENTER);
    }
    glow(dc, MORE_BTN, l.more_btn, cb, pal().hover, sc(8), hot == MORE_BTN);
    SelectObject(dc, f_glyph);
    text(dc, "\u{E712}", l.more_btn, pal().text, DT_CENTER);

    let focused = !edit.is_null() && GetFocus() == edit;
    let f = hv(2000, focused);
    fill(dc, l.pill, mix(pal().line, pal().accent, f), sc(10));
    fill(dc, rc(l.pill.left + 1, l.pill.top + 1, l.pill.right - 1, l.pill.bottom - 1), pill(), sc(8));
    SelectObject(dc, f_glyph);
    let searching = SEARCHING.with_borrow(|s| s.clone());
    text(dc, if searching.is_some() { "\u{E721}" } else { "\u{E838}" }, rc(l.pill.left + sc(10), l.pill.top, l.pill.left + sc(38), l.pill.bottom), mix(pal().dim, pal().accent, f), DT_CENTER);
    let mut crumbs = vec![];
    if !focused {
        let (x0, x1) = (l.pill.left + sc(40), l.pill.right - sc(12));
        let (t, b) = (l.pill.top + sc(5), l.pill.bottom - sc(5));
        if let Some(q) = &searching {
            SelectObject(dc, f_txt);
            text(dc, &format!("Results for \u{201C}{q}\u{201D}"), rc(x0 + sc(6), l.pill.top, x1, l.pill.bottom), pal().accent, DT_LEFT);
        } else {
            let parts = crumbs_of(&dir);
            let width = |s: &str| { let mut r = rc(0, 0, 0, 0); DrawTextW(dc, w(s).as_ptr(), -1, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX); r.right + sc(16) };
            let chev = sc(18);
            SelectObject(dc, f_txt);
            let ws: Vec<i32> = parts.iter().map(|p| width(&p.0)).collect();
            let mut first = 0;
            while first + 1 < parts.len() && ws[first..].iter().sum::<i32>() + chev * (parts.len() - first) as i32 + width("\u{2026}") > x1 - x0 { first += 1; }
            let mut shown: Vec<(String, PathBuf, i32)> = vec![];
            if first > 0 { shown.push(("\u{2026}".into(), parts[first - 1].1.clone(), width("\u{2026}"))); }
            shown.extend(parts[first..].iter().zip(&ws[first..]).map(|(p, &wd)| (p.0.clone(), p.1.clone(), wd)));
            let mut x = x0;
            let last = shown.len().saturating_sub(1);
            for (i, (label, path, wd)) in shown.into_iter().enumerate() {
                let r = rc(x, t, (x + wd).min(x1), b);
                glow(dc, CRUMB + i as i32, r, pill(), pal().hover, sc(8), hot == CRUMB + i as i32);
                SelectObject(dc, f_txt);
                text(dc, &label, r, if i == last { pal().text } else { mix(pal().dim, pal().text, if hot == CRUMB + i as i32 { 1.0 } else { 0.35 }) }, DT_CENTER);
                crumbs.push((r, path));
                x = r.right;
                if i < last {
                    SelectObject(dc, f_small);
                    text(dc, "\u{E76C}", rc(x, t, x + chev, b), mix(pal().dim, pill(), 0.3), DT_CENTER);
                    x += chev;
                }
            }
        }
    }
    CRUMBS.set(crumbs);

    fill(dc, rc(0, cr.bottom - 1, cr.right, cr.bottom), pal().line, 0);
    BitBlt(wdc, 0, 0, cr.right, cr.bottom, dc, 0, 0, SRCCOPY);
    SelectObject(dc, of);
    if BUSY.get() { SetTimer(hwnd, 4, 16, None); } else { KillTimer(hwnd, 4); }
    EndPaint(hwnd, &ps);
}

pub unsafe fn paint_status(h: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(h, &mut ps);
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(h, &mut cr);
    let strip = rc(0, cr.bottom - sc(STATUS_H), cr.right, cr.bottom);
    SetDCBrushColor(dc, pal().bg);
    FillRect(dc, &strip, GetStockObject(DC_BRUSH as i32) as HBRUSH);
    winrust_common::style::backdrop(dc, strip, sc(100) as f32 / 100.0);
    fill(dc, rc(0, strip.top, cr.right, strip.top + 1), pal().line, 0);
    let snap = ST.with(|c| {
        c.try_borrow().ok().map(|s| {
            let (mut n, mut bytes) = (0usize, 0u64);
            if s.dir == Path::new(crate::THIS_PC_KEY) {
                if let Some(r) = usize::try_from(PC_SEL.get()).ok().and_then(|i| s.rows.get(i)).filter(|r| !r.hdr) {
                    n = 1;
                    if !r.dir { bytes += r.size; }
                }
            } else {
                let mut i = -1isize;
                loop {
                    i = SendMessageW(s.list, LVM_GETNEXTITEM, i as usize, LVNI_SELECTED as isize);
                    let Some(r) = usize::try_from(i).ok().and_then(|i| s.rows.get(i)) else { break };
                    if r.hdr { continue; }
                    n += 1;
                    if !r.dir { bytes += r.size; }
                }
            }
            (s.rows.iter().filter(|r| !r.hdr).count(), n, bytes)
        })
    });
    if let Some((n, sel, bytes)) = snap {
        if let Some(pct) = crate::jobs::progress_pct() {
            SetPropW(h, w("WinRustProgress").as_ptr(), (pct as usize + 1) as HANDLE);
        } else {
            RemovePropW(h, w("WinRustProgress").as_ptr());
        }
        let selected = |s: usize| format!("{} selected", s);
        let left = match (sel, bytes) {
            (0, _) => items_text(n),
            (s, 0) => format!("{}  \u{00B7}  {}", items_text(n), selected(s)),
            (s, b) => format!("{}  \u{00B7}  {}  \u{00B7}  {}", items_text(n), selected(s), size_text(b)),
        };
        let left = crate::jobs::status_text().unwrap_or(left);
        let by = [
            "Name",
            "Date",
            "Type",
            "Size",
        ][SORT_COL.load(Relaxed).min(3)];
        let right = format!("{by} {}", if SORT_ASC.load(Relaxed) { "\u{2191}" } else { "\u{2193}" });
        let mut sf = STATUS_FONT.get();
        if sf == 0 {
            sf = font(12, 400, ui_face());
            STATUS_FONT.set(sf);
        }
        let of = SelectObject(dc, sf as HFONT);
        SetBkMode(dc, TRANSPARENT as i32);
        text(dc, &left, rc(strip.left + sc(14), strip.top, strip.right / 2, strip.bottom), pal().dim, DT_LEFT);
        text(dc, &right, rc(strip.right / 2, strip.top, strip.right - sc(14), strip.bottom), pal().dim, DT_RIGHT);
        SelectObject(dc, of);
    }
    EndPaint(h, &ps);
}

pub unsafe fn status_dirty(h: HWND) {
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(h, &mut cr);
    let strip = rc(0, cr.bottom - sc(STATUS_H), cr.right, cr.bottom);
    InvalidateRect(h, &strip, 0);
}

pub unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
    let layout_now = || {
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(h, &mut r);
        layout(r.right, ST.with(|c| c.try_borrow().map_or(1, |s| s.tabs.len())))
    };
    match m {
        WM_NCHITTEST if !crate::desk() && edge_hit(h, lp) => return HTTRANSPARENT as isize,
        WM_PAINT => paint_chrome(h),
        WM_ERASEBKGND => return 1,
        WM_SIZE => { place_edit(h); InvalidateRect(h, ptr::null(), 0); }
        WM_TIMER if wp == 4 => { InvalidateRect(h, ptr::null(), 0); }
        WM_MOUSEMOVE => {
            let k = match hit(&layout_now(), x, y) { NONE => crumb_at(x, y).map_or(NONE, |i| CRUMB + i as i32), k => k };
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            if HOT.replace(k) != k { InvalidateRect(h, ptr::null(), 0); }
        }
        WM_MOUSELEAVE => {
            HOT.set(NONE);
            InvalidateRect(h, ptr::null(), 0);
        }
        WM_LBUTTONDBLCLK => {
            let l = layout_now();
            if y < sc(40) && hit(&l, x, y) == NONE {
                let p = GetParent(h);
                SendMessageW(p, WM_SYSCOMMAND, if IsZoomed(p) != 0 { SC_RESTORE } else { SC_MAXIMIZE } as usize, 0);
            }
        }
        WM_LBUTTONDOWN | WM_MBUTTONDOWN => {
            let l = layout_now();
            let k = hit(&l, x, y);
            if k == WIN_MIN || k == WIN_MAX || k == WIN_CLOSE {
                let p = GetParent(h);
                let cmd = match k {
                    WIN_MIN => SC_MINIMIZE,
                    WIN_MAX => if IsZoomed(p) != 0 { SC_RESTORE } else { SC_MAXIMIZE },
                    _ => SC_CLOSE,
                };
                PostMessageW(p, WM_SYSCOMMAND, cmd as usize, 0);
                return 0;
            }
            if k == NONE {
                if y < sc(40) && m == WM_LBUTTONDOWN {
                    ReleaseCapture();
                    SendMessageW(GetParent(h), WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
                    return 0;
                }
                if let Some(i) = crumb_at(x, y) {
                    PRESS.set((CRUMB + i as i32, now()));
                    PostMessageW(GetParent(h), WM_UI, 21, i as isize);
                } else if inside(&l.pill, x, y) {
                    let edit = ST.with(|c| c.try_borrow().map_or(ptr::null_mut(), |s| s.edit));
                    SetFocus(edit);
                    SendMessageW(edit, EM_SETSEL, 0, -1);
                }
                InvalidateRect(h, ptr::null(), 0);
                return 0;
            }
            PRESS.set((k, now()));
            InvalidateRect(h, ptr::null(), 0);
            let (act, idx) = match k {
                k if (CMD..CMD + 5).contains(&k) => (k as usize - CMD as usize + 10, 0), BACK => (4, 0), FWD => (5, 0), UP => (6, 0), RELOAD => (7, 0), SEARCH => (9, 0), VIEW => (8, 0), PLUS => (3, 0), NEW_BTN => (16, 0), SORT_BTN => (17, 0), MORE_BTN => (18, 0),
                k if k >= CLOSE => (2, (k - CLOSE) as isize),
                k => (if m == WM_MBUTTONDOWN { 2 } else { 1 }, (k - TAB) as isize),
            };
            PostMessageW(GetParent(h), WM_UI, act, idx);
        }
        WM_COMMAND if (wp >> 16) == 0x0100 || (wp >> 16) == 0x0200 => {
            ShowWindow(lp as HWND, if (wp >> 16) == 0x0100 { SW_SHOW } else { SW_HIDE });
            InvalidateRect(h, ptr::null(), 0);
        }
        WM_CTLCOLOREDIT => {
            let dc = wp as HDC;
            SetTextColor(dc, pal().text);
            SetBkColor(dc, pill());
            if BRUSH.get() == 0 { BRUSH.set(CreateSolidBrush(pill()) as isize); }
            return BRUSH.get();
        }
        WM_RBUTTONUP => return 0,
        WM_CONTEXTMENU => {
            let edit = ST.with(|c| c.try_borrow().map_or(ptr::null_mut(), |s| s.edit));
            if wp as HWND != edit { return 0; }
            return DefWindowProcW(h, m, wp, lp);
        }
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}

#[derive(Clone)]
pub struct MenuItem {
    pub id: usize,
    pub label: String,
    pub glyph: &'static str,
    pub sep: bool,
    pub danger: bool,
    pub checked: bool,
    pub disabled: bool,
    pub sub: Vec<MenuItem>,
}

impl MenuItem {
    pub fn new(id: usize, label: impl Into<String>, glyph: &'static str) -> Self {
        Self { id, label: label.into(), glyph, sep: false, danger: false, checked: false, disabled: false, sub: Vec::new() }
    }
    pub fn sep(mut self) -> Self { self.sep = true; self }
    pub fn danger(mut self) -> Self { self.danger = true; self }
    pub fn checked(mut self, on: bool) -> Self { self.checked = on; self }
    pub fn disabled(mut self, off: bool) -> Self { self.disabled = off; self }
    pub fn sub(mut self, items: Vec<MenuItem>) -> Self { self.sub = items; self }
}

struct MenuState<'a> {
    items: &'a [MenuItem],
    ys: Vec<(i32, i32)>,
    tot_h: i32,
    scroll: i32,
    hot: isize,
    picked: Option<usize>,
    done: bool,
    sub_open: bool,
}

fn menu_fonts() -> (HFONT, HFONT) {
    let mut mf = MENU_FONTS.get();
    if mf[0] == 0 {
        mf = [font(13, 400, ui_face()), font(13, 400, winrust_common::style::icon_face())];
        MENU_FONTS.set(mf);
    }
    (mf[0] as HFONT, mf[1] as HFONT)
}

unsafe extern "system" fn menu_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_NCCREATE {
        let cs = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(h, GWLP_USERDATA, cs.lpCreateParams as isize);
        return 1;
    }
    let ptr = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut MenuState<'_>;
    if ptr.is_null() { return DefWindowProcW(h, m, wp, lp); }
    let st = &mut *ptr;
    match m {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let wdc = BeginPaint(h, &mut ps);
            let mut cr: RECT = std::mem::zeroed();
            GetClientRect(h, &mut cr);
            thread_local! {
                static MENU_BUF: RefCell<BackBuf> = const { RefCell::new(BackBuf::new()) };
            }
            let dc = MENU_BUF.with_borrow_mut(|b| b.get(wdc, cr.right, cr.bottom));
            let bg_col = if winrust_common::style::style().light() { winrust_common::style::card() } else { mix(pal().bg, pal().text, 0.05) };
            SetDCBrushColor(dc, bg_col);
            FillRect(dc, &cr, GetStockObject(DC_BRUSH as i32) as HBRUSH);
            SetBkMode(dc, TRANSPARENT as i32);
            let (f_txt, f_ico) = menu_fonts();
            let of = SelectObject(dc, f_txt);
            let max_s = (st.tot_h - cr.bottom).max(0);
            st.scroll = st.scroll.clamp(0, max_s);
            for (i, it) in st.items.iter().enumerate() {
                let (y0, y1) = (st.ys[i].0 - st.scroll, st.ys[i].1 - st.scroll);
                if y1 < 0 || y0 > cr.bottom {
                    continue;
                }
                if it.sep {
                    fill(dc, rc(sc(8), y0 - sc(3), cr.right - sc(8), y0 - sc(2)), mix(bg_col, pal().text, 0.10), 0);
                }
                let r = rc(sc(4), y0, cr.right - sc(4), y1);
                if st.hot == i as isize && !it.disabled {
                    fill(dc, r, mix(bg_col, pal().text, 0.09), sc(6));
                }
                let fg = if it.disabled { off() } else if it.danger { winrust_common::style::BAD } else { pal().text };
                let ico_c = if it.disabled { off() } else if it.danger { fg } else { mix(pal().dim, pal().text, 0.7) };
                SelectObject(dc, f_ico);
                let g = if it.checked && it.glyph.is_empty() { "\u{E73E}" } else { it.glyph };
                text(dc, g, rc(r.left + sc(6), r.top, r.left + sc(26), r.bottom), if it.checked { pal().accent } else { ico_c }, DT_CENTER);
                SelectObject(dc, f_txt);
                let right_pad = if it.sub.is_empty() { sc(8) } else { sc(22) };
                text(dc, &it.label, rc(r.left + sc(30), r.top, r.right - right_pad, r.bottom), fg, DT_LEFT);
                if !it.sub.is_empty() {
                    SelectObject(dc, f_ico);
                    text(dc, "\u{E76C}", rc(r.right - sc(20), r.top, r.right - sc(4), r.bottom), pal().dim, DT_CENTER);
                }
            }
            if max_s > 0 && cr.bottom > sc(16) {
                let th = ((cr.bottom as f32 * cr.bottom as f32 / st.tot_h.max(1) as f32) as i32).clamp(sc(18), cr.bottom - sc(6));
                let ty = ((cr.bottom - th - sc(6)) as f32 * (st.scroll as f32 / max_s as f32)) as i32 + sc(3);
                fill(dc, rc(cr.right - sc(4), ty, cr.right - sc(1), ty + th), mix(bg_col, pal().text, 0.28), sc(2));
            }
            BitBlt(wdc, 0, 0, cr.right, cr.bottom, dc, 0, 0, SRCCOPY);
            SelectObject(dc, of);
            EndPaint(h, &ps);
            0
        }
        WM_MOUSEMOVE => {
            let y = ((lp >> 16) & 0xFFFF) as i16 as i32 + st.scroll;
            let hit = st.ys.iter().position(|&(a, b)| y >= a && y < b).map_or(-1, |i| i as isize);
            if st.hot != hit {
                st.hot = hit;
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                InvalidateRect(h, ptr::null(), 0);
                if hit >= 0 && !st.items[hit as usize].sub.is_empty() && !st.items[hit as usize].disabled {
                    SetTimer(h, 9, 220, None);
                } else {
                    KillTimer(h, 9);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            let mut cr: RECT = std::mem::zeroed();
            GetClientRect(h, &mut cr);
            let max_s = (st.tot_h - cr.bottom).max(0);
            if max_s > 0 {
                let delta = ((wp >> 16) as u16) as i16 as i32;
                let step = (-delta * sc(36)) / 120;
                let next = (st.scroll + step).clamp(0, max_s);
                if st.scroll != next {
                    st.scroll = next;
                    InvalidateRect(h, ptr::null(), 0);
                }
            }
            0
        }
        WM_TIMER if wp == 9 => {
            KillTimer(h, 9);
            if st.hot >= 0 && !st.sub_open && !st.items[st.hot as usize].sub.is_empty() && !st.items[st.hot as usize].disabled {
                open_submenu(h, st, st.hot as usize);
            }
            0
        }
        WM_MOUSELEAVE => {
            KillTimer(h, 9);
            if st.hot != -1 { st.hot = -1; InvalidateRect(h, ptr::null(), 0); }
            0
        }
        WM_LBUTTONUP => {
            KillTimer(h, 9);
            let y = ((lp >> 16) & 0xFFFF) as i16 as i32 + st.scroll;
            if let Some(i) = st.ys.iter().position(|&(a, b)| y >= a && y < b) {
                if !st.items[i].disabled {
                    if !st.items[i].sub.is_empty() {
                        open_submenu(h, st, i);
                        return 0;
                    }
                    st.picked = Some(st.items[i].id);
                    st.done = true;
                }
            } else {
                st.done = true;
            }
            0
        }
        WM_KEYDOWN => {
            match wp as u16 {
                0x1B => st.done = true,
                0x27 if st.hot >= 0 && !st.items[st.hot as usize].sub.is_empty() && !st.items[st.hot as usize].disabled => {
                    open_submenu(h, st, st.hot as usize);
                }
                0x0D if st.hot >= 0 && !st.items[st.hot as usize].disabled => {
                    if !st.items[st.hot as usize].sub.is_empty() {
                        open_submenu(h, st, st.hot as usize);
                    } else {
                        st.picked = Some(st.items[st.hot as usize].id);
                        st.done = true;
                    }
                }
                0x26 if !st.items.is_empty() => {
                    st.hot = if st.hot <= 0 { st.items.len() as isize - 1 } else { st.hot - 1 };
                    InvalidateRect(h, ptr::null(), 0);
                }
                0x28 if !st.items.is_empty() => {
                    st.hot = if st.hot < 0 || st.hot + 1 >= st.items.len() as isize { 0 } else { st.hot + 1 };
                    InvalidateRect(h, ptr::null(), 0);
                }
                _ => {}
            }
            0
        }
        WM_KILLFOCUS | WM_ACTIVATE if !st.sub_open && (m == WM_KILLFOCUS || (wp & 0xFFFF) == 0) => {
            st.done = true;
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

unsafe fn open_submenu(h: HWND, st: &mut MenuState<'_>, idx: usize) {
    let mut wr: RECT = std::mem::zeroed();
    GetWindowRect(h, &mut wr);
    let y0 = st.ys[idx].0 - st.scroll;
    st.sub_open = true;
    let res = popup_menu_at(h, &st.items[idx].sub, Some((wr.right - sc(4), wr.top + y0 - sc(4))), Some(wr.left + sc(4)));
    st.sub_open = false;
    if res.is_some() {
        st.picked = res;
        st.done = true;
    } else {
        SetFocus(h);
    }
}

pub unsafe fn popup_menu(owner: HWND, items: &[MenuItem]) -> Option<usize> {
    popup_menu_at(owner, items, None, None)
}

pub unsafe fn popup_menu_at(owner: HWND, items: &[MenuItem], pos: Option<(i32, i32)>, flip_x: Option<i32>) -> Option<usize> {
    if items.is_empty() { return None; }
    static REG: std::sync::Once = std::sync::Once::new();
    let cls = w("WinRustExplorerMenu");
    REG.call_once(|| {
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(menu_proc);
        wc.lpszClassName = cls.as_ptr();
        wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
        RegisterClassExW(&wc);
    });
    let mut pt: POINT = std::mem::zeroed();
    if let Some((px, py)) = pos {
        pt = POINT { x: px, y: py };
    } else {
        GetCursorPos(&mut pt);
    }
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(mon, &mut mi);
    let wa = mi.rcWork;
    let wa_w = (wa.right - wa.left - sc(8)).max(sc(160));
    let wa_h = (wa.bottom - wa.top - sc(8)).max(sc(120));

    let dc = GetDC(ptr::null_mut());
    let (f, _) = menu_fonts();
    let of = SelectObject(dc, f);
    let mut max_w = sc(168);
    for it in items {
        let mut r = rc(0, 0, 0, 0);
        DrawTextW(dc, w(&it.label).as_ptr(), -1, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        let extra = if it.sub.is_empty() { sc(54) } else { sc(70) };
        max_w = max_w.max(r.right + extra);
    }
    SelectObject(dc, of);
    ReleaseDC(ptr::null_mut(), dc);

    let mut row_h = sc(26);
    let mut gap = sc(1);
    let mut sep_h = sc(5);
    let calc_ys = |rh: i32, gp: i32, sh: i32| {
        let mut y = sc(4);
        let mut ys = Vec::with_capacity(items.len());
        for it in items {
            if it.sep { y += sh; }
            ys.push((y, y + rh));
            y += rh + gp;
        }
        (ys, y + sc(4))
    };
    let (mut ys, mut tot_h) = calc_ys(row_h, gap, sep_h);
    if tot_h > wa_h {
        row_h = sc(22);
        gap = 0;
        sep_h = sc(3);
        (ys, tot_h) = calc_ys(row_h, gap, sep_h);
    }
    let mw = max_w.min(sc(300)).min(wa_w);
    let mh = tot_h.min(wa_h);
    let x = if let Some(fx) = flip_x {
        if pt.x + mw <= wa.right - sc(4) { pt.x } else { (fx - mw).max(wa.left + sc(4)) }
    } else {
        pt.x.min(wa.right - mw - sc(4)).max(wa.left + sc(4))
    };
    let y = if pt.y + mh <= wa.bottom - sc(4) {
        pt.y.max(wa.top + sc(4))
    } else if pt.y - mh >= wa.top + sc(4) {
        pt.y - mh
    } else {
        (wa.bottom - mh - sc(4)).max(wa.top + sc(4))
    };

    let mut st = MenuState { items, ys, tot_h, scroll: 0, hot: -1, picked: None, done: false, sub_open: false };
    let hw = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        cls.as_ptr(),
        ptr::null(),
        WS_POPUP | WS_CLIPSIBLINGS,
        x, y, mw, mh,
        owner,
        ptr::null_mut(),
        ptr::null_mut(),
        &mut st as *mut _ as *mut _,
    );
    if hw.is_null() { return None; }
    let rad = if winrust_common::style::style().square() { 1 } else { 2 };
    DwmSetWindowAttribute(hw, 33, &rad as *const _ as _, 4);
    let bdr = mix(pal().bg, pal().text, 0.14) as i32;
    DwmSetWindowAttribute(hw, 34, &bdr as *const _ as _, 4);
    ShowWindow(hw, SW_SHOW);
    SetFocus(hw);
    let mut msg: MSG = std::mem::zeroed();
    while !st.done && GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    DestroyWindow(hw);
    st.picked
}

pub const NAV_W: i32 = 232;
const NAV_ROW: i32 = 34;
const NAV_HEAD: i32 = 32;

struct NavItem { title: String, path: PathBuf, glyph: &'static str }

fn nav_h(it: &NavItem) -> i32 {
    sc(if it.path.as_os_str().is_empty() { NAV_HEAD } else { NAV_ROW })
}

thread_local! {
    static NAV: RefCell<Vec<NavItem>> = const { RefCell::new(Vec::new()) };
    static NAVWND: Cell<isize> = const { Cell::new(0) };
    static NAVHOT: Cell<i32> = const { Cell::new(-1) };
    static NAVSCROLL: Cell<i32> = const { Cell::new(0) };
}

pub fn nav_hwnd() -> HWND { NAVWND.get() as HWND }

fn nav_total_h() -> i32 {
    sc(16) + NAV.with_borrow(|n| n.iter().map(nav_h).sum::<i32>())
}

unsafe fn nav_max_scroll(h: HWND) -> i32 {
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(h, &mut cr);
    (nav_total_h() - cr.bottom).max(0)
}

fn fmt_cap(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let v = b as f64;
    if v >= TB { format!("{:.1} TB", v / TB) }
    else if v >= 10.0 * GB { format!("{:.0} GB", v / GB) }
    else if v >= GB { format!("{:.1} GB", v / GB) }
    else if v >= MB { format!("{:.0} MB", v / MB) }
    else { format!("{:.0} KB", (v / KB).max(1.0)) }
}

fn pins_file() -> Option<PathBuf> { env::var_os("APPDATA").map(|a| PathBuf::from(a).join("WinRust").join("rust-explorer-pins.txt")) }

pub fn pins() -> Vec<PathBuf> {
    pins_file().and_then(|f| fs::read_to_string(f).ok()).map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()).unwrap_or_default()
}

pub fn toggle_pin(p: &Path) {
    let mut v = pins();
    if let Some(i) = v.iter().position(|x| x == p) { v.remove(i); } else { v.push(p.to_path_buf()); }
    if let Some(f) = pins_file() {
        if let Some(d) = f.parent() { let _ = fs::create_dir_all(d); }
        let _ = fs::write(f, v.iter().map(|x| x.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n"));
    }
    let h = NAVWND.get() as HWND;
    if !h.is_null() { nav_init(h); nav_dirty(); }
}

pub fn nav_init(h: HWND) {
    NAVWND.set(h as isize);
    let home = PathBuf::from(env::var_os("USERPROFILE").unwrap_or_default());
    let head = |lbl: &str| NavItem { title: lbl.into(), path: PathBuf::new(), glyph: "" };
    let mut v = vec![head("Quick access"), NavItem { title: "Home".into(), path: home.clone(), glyph: "\u{E80F}" }];
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos};
    let known = [
        ("Desktop", &FOLDERID_Desktop, "\u{E7F4}"),
        ("Downloads", &FOLDERID_Downloads, "\u{E896}"),
        ("Documents", &FOLDERID_Documents, "\u{E8A5}"),
        ("Pictures", &FOLDERID_Pictures, "\u{E91B}"),
        ("Music", &FOLDERID_Music, "\u{E8D6}"),
        ("Videos", &FOLDERID_Videos, "\u{E714}"),
    ];
    for (lbl, id, g) in known {
        let Some(p) = crate::sh::known_dir(id).filter(|p| p.is_dir()) else { continue };
        v.push(NavItem { title: lbl.into(), path: p, glyph: g });
    }
    for p in pins() {
        if p.is_dir() { v.push(NavItem { title: p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned()), path: p, glyph: "\u{E718}" }); }
    }
    v.push(NavItem { title: "This PC".into(), path: PathBuf::from(crate::THIS_PC_KEY), glyph: "\u{E7F4}" });
    unsafe {
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
        let mask = GetLogicalDrives();
        for i in 0..26u32 {
            if mask & (1 << i) == 0 { continue; }
            let letter = (b'A' + i as u8) as char;
            let root = format!("{letter}:\\");
            let ty = GetDriveTypeW(w(&root).as_ptr());
            if ty == 4 {
                v.push(NavItem { title: format!("Network Drive ({}:)", letter), path: PathBuf::from(root), glyph: "\u{E8CE}" });
                continue;
            }
            if !matches!(ty, 2 | 3 | 5 | 6) { continue; }
            let mut label = [0u16; 64];
            let ok = GetVolumeInformationW(w(&root).as_ptr(), label.as_mut_ptr(), 64, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), 0);
            let name = if ok != 0 { wstr(label.as_ptr()) } else { String::new() };
            let base = if name.is_empty() { "Local Disk" } else { &name };
            v.push(NavItem { title: format!("{base} ({letter}:)"), path: PathBuf::from(root), glyph: if ty == 5 { "\u{E958}" } else { "\u{EDA2}" } });
        }
    }
    NAV.with_borrow_mut(|n| *n = v);
}

pub fn nav_path(i: usize) -> Option<PathBuf> {
    NAV.with_borrow(|n| n.get(i).map(|x| x.path.clone()).filter(|p| !p.as_os_str().is_empty()))
}

pub fn nav_dirty() {
    let h = NAVWND.get() as HWND;
    if !h.is_null() { unsafe { InvalidateRect(h, ptr::null(), 0) }; }
}

pub unsafe fn layout_children(cw: i32, ch: i32) {
    let (chrome, list) = ST.with(|c| c.try_borrow().map_or((ptr::null_mut(), ptr::null_mut()), |s| (s.tab, s.list)));
    let nav = NAVWND.get() as HWND;
    let (top, status) = (sc(CHROME_H), sc(STATUS_H));
    let nw = if cw >= sc(920) { sc(NAV_W) } else if cw >= sc(640) { sc(180) } else { 0 };
    let h = (ch - top - status).max(0);
    MoveWindow(chrome, 0, 0, cw, top, 1);
    MoveWindow(nav, 0, top, nw, h, 1);
    MoveWindow(list, nw, top, cw - nw, h, 1);
    fit_columns(list);
}

fn nav_hit(y: i32) -> Option<usize> {
    let mut top = sc(8) - NAVSCROLL.get();
    NAV.with_borrow(|n| {
        for (i, it) in n.iter().enumerate() {
            let hgt = nav_h(it);
            if y >= top && y < top + hgt { return (!it.path.as_os_str().is_empty()).then_some(i); }
            top += hgt;
        }
        None
    })
}

unsafe fn paint_nav(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let wdc = BeginPaint(hwnd, &mut ps);
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut cr);
    BUSY.set(false);
    let dc = NAV_BUF.with_borrow_mut(|b| b.get(wdc, cr.right, cr.bottom));
    SetDCBrushColor(dc, pal().bg);
    FillRect(dc, &cr, GetStockObject(DC_BRUSH as i32) as HBRUSH);
    winrust_common::style::backdrop(dc, cr, sc(100) as f32 / 100.0);
    SetBkMode(dc, TRANSPARENT as i32);
    let mut nf = NAV_FONTS.get();
    if nf[0] == 0 {
        nf = [font(13, 400, ui_face()), font(12, 600, ui_face()), font(16, 400, winrust_common::style::icon_face()), font(11, 400, ui_face())];
        NAV_FONTS.set(nf);
    }
    let [f_txt, f_head, f_glyph, _f_small] = [nf[0] as HFONT, nf[1] as HFONT, nf[2] as HFONT, nf[3] as HFONT];
    let of = SelectObject(dc, f_txt);
    let cur = ST.with(|c| c.try_borrow().ok().map(|s| s.dir.to_string_lossy().to_lowercase())).unwrap_or_default();
    let hot = NAVHOT.get();
    let max_s = (nav_total_h() - cr.bottom).max(0);
    let scroll = NAVSCROLL.get().clamp(0, max_s);
    NAVSCROLL.set(scroll);
    NAV.with_borrow(|n| {
        let here = if cur == crate::THIS_PC_KEY.to_lowercase() {
            n.iter().position(|it| it.path == Path::new(crate::THIS_PC_KEY))
        } else {
            n.iter().enumerate().filter(|(_, it)| !it.path.as_os_str().is_empty() && it.path != Path::new(crate::THIS_PC_KEY) && cur.starts_with(&it.path.to_string_lossy().to_lowercase())).max_by_key(|(_, it)| it.path.as_os_str().len()).map(|(i, _)| i)
        };
        let mut y = sc(8) - scroll;
        for (i, it) in n.iter().enumerate() {
            if it.path.as_os_str().is_empty() {
                if i > 0 {
                    fill(dc, rc(sc(14), y + sc(6), cr.right - sc(14), y + sc(7)), mix(pal().bg, pal().text, 0.08), 0);
                }
                SelectObject(dc, f_head);
                text(dc, &it.title, rc(sc(16), y + sc(8), cr.right - sc(8), y + sc(NAV_HEAD)), pal().dim, DT_LEFT);
                y += sc(NAV_HEAD);
                continue;
            }
            let hgt = nav_h(it);
            let r = rc(sc(8), y + sc(1), cr.right - sc(8), y + hgt - sc(1));
            let (th, ta) = (hv(5000 + i as i32, hot == i as i32), hv(6000 + i as i32, here == Some(i)));
            let p = press_t(5000 + i as i32);
            if th.max(ta) > 0.01 || p > 0.0 { fill(dc, r, mix(mix(mix(pal().bg, pal().hover, th), pal().active, ta), pal().accent, 0.3 * p), sc(8)); }
            if ta > 0.01 { fill(dc, rc(r.left, y + sc(8), r.left + sc(3), y + hgt - sc(8)), mix(pal().active, pal().accent, ta), sc(3)); }
            let line = rc(r.left, y, r.right, y + sc(NAV_ROW));
            let ico_col = match it.glyph {
                "\u{E80F}" => 0x00FF_CD60,
                "\u{E7F4}" => 0x00FF_C24C,
                "\u{E896}" => 0x00F8_BD38,
                "\u{E8A5}" => 0x0024_BFFB,
                "\u{E91B}" => 0x0099_D334,
                "\u{E8D6}" => 0x00B6_72F4,
                "\u{E714}" => 0x00FA_8BA7,
                "\u{E718}" => 0x001E_B7F7,
                _ => mix(pal().dim, pal().accent, ta),
            };
            SelectObject(dc, f_glyph);
            text(dc, it.glyph, rc(r.left + sc(10), line.top, r.left + sc(38), line.bottom), ico_col, DT_CENTER);
            SelectObject(dc, f_txt);
            text(dc, &it.title, rc(r.left + sc(44), line.top, r.right - sc(8), line.bottom), mix(pal().dim, pal().text, ta.max(th).max(0.45)), DT_LEFT);
            y += hgt;
        }
    });
    if max_s > 0 && cr.bottom > sc(24) {
        let tot = nav_total_h().max(1);
        let th = ((cr.bottom as f32 * cr.bottom as f32 / tot as f32) as i32).clamp(sc(24), cr.bottom - sc(8));
        let ty = ((cr.bottom - th - sc(8)) as f32 * (scroll as f32 / max_s as f32)) as i32 + sc(4);
        fill(dc, rc(cr.right - sc(5), ty, cr.right - sc(2), ty + th), mix(pal().bg, pal().text, 0.28), sc(3));
    }
    fill(dc, rc(cr.right - 1, 0, cr.right, cr.bottom), mix(pal().bg, pal().text, 0.06), 0);
    BitBlt(wdc, 0, 0, cr.right, cr.bottom, dc, 0, 0, SRCCOPY);
    SelectObject(dc, of);
    if BUSY.get() { SetTimer(hwnd, 4, 16, None); } else { KillTimer(hwnd, 4); }
    EndPaint(hwnd, &ps);
}

pub unsafe extern "system" fn nav_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_NCHITTEST && !crate::desk() && edge_hit(h, lp) { return HTTRANSPARENT as isize; }
    let y = ((lp >> 16) & 0xFFFF) as i16 as i32;
    match m {
        WM_PAINT => paint_nav(h),
        WM_ERASEBKGND => return 1,
        WM_TIMER if wp == 4 => { InvalidateRect(h, ptr::null(), 0); }
        WM_MOUSEMOVE => {
            let k = nav_hit(y).map_or(-1, |i| i as i32);
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            if NAVHOT.replace(k) != k { InvalidateRect(h, ptr::null(), 0); }
        }
        WM_MOUSELEAVE => { NAVHOT.set(-1); InvalidateRect(h, ptr::null(), 0); }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) as u16) as i16 as i32;
            let step = (-delta * sc(48)) / 120;
            let next = (NAVSCROLL.get() + step).clamp(0, nav_max_scroll(h));
            if NAVSCROLL.replace(next) != next { InvalidateRect(h, ptr::null(), 0); }
        }
        WM_SIZE => {
            let next = NAVSCROLL.get().clamp(0, nav_max_scroll(h));
            if NAVSCROLL.replace(next) != next { InvalidateRect(h, ptr::null(), 0); }
        }
        WM_LBUTTONDOWN => {
            if let Some(i) = nav_hit(y) {
                PRESS.set((5000 + i as i32, now()));
                InvalidateRect(h, ptr::null(), 0);
                PostMessageW(GetParent(h), WM_UI, 20, i as isize);
            }
        }
        WM_RBUTTONUP => {
            if let Some(p) = nav_hit(y).and_then(nav_path).filter(|p| pins().contains(p)) {
                let items = [MenuItem::new(1, "Unpin from Quick access", "\u{E77A}")];
                if popup_menu(h, &items) == Some(1) { toggle_pin(&p); }
            }
        }
        WM_CONTEXTMENU => return 0,
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}
