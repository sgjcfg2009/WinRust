use super::*;
use windows_sys::Win32::{
    Foundation::SIZE,
    Graphics::Dwm::{DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties},
};

pub(crate) const CARD_W: i32 = 220;
pub(crate) const CARD_H: i32 = 160;
pub(crate) const GAP: i32 = 10;
const TITLE: i32 = 28;

pub(crate) fn grid(n: usize, aw: i32, ah: i32, cw: i32, ch: i32, gap: i32) -> (Vec<RECT>, i32, i32) {
    if n == 0 {
        return (vec![], 0, 0);
    }
    let mut s = 1.0f32;
    let (w, h, rows, cols) = loop {
        let (w, h) = ((cw as f32 * s) as i32, (ch as f32 * s) as i32);
        let fit = ((aw + gap) / (w + gap)).max(1) as usize;
        let rows = n.div_ceil(fit);
        if rows as i32 * (h + gap) - gap <= ah || s < 0.35 {
            break (w, h, rows, n.div_ceil(rows));
        }
        s *= 0.9;
    };
    let (tw, th) = (cols as i32 * (w + gap) - gap, rows as i32 * (h + gap) - gap);
    let mut out = Vec::with_capacity(n);
    for r in 0..rows {
        let count = cols.min(n - r * cols);
        let x0 = (tw - (count as i32 * (w + gap) - gap)) / 2;
        for c in 0..count {
            let (x, y) = (x0 + c as i32 * (w + gap), r as i32 * (h + gap));
            out.push(RECT { left: x, top: y, right: x + w, bottom: y + h });
        }
    }
    (out, tw, th)
}

pub(crate) fn close_rect(r: &RECT) -> RECT {
    let s = sc(TITLE) - sc(8);
    RECT { left: r.right - s - sc(4), top: r.top + sc(4), right: r.right - sc(4), bottom: r.top + sc(4) + s }
}

fn preview_area(r: &RECT) -> RECT {
    RECT { left: r.left + sc(8), top: r.top + sc(TITLE), right: r.right - sc(8), bottom: r.bottom - sc(8) }
}

pub(crate) fn card_at(rects: &[RECT], x: i32, y: i32) -> Option<usize> {
    rects.iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

thread_local! {
    static CARD_ICONS: RefCell<HashMap<usize, Vec<HICON>>> = RefCell::new(HashMap::new());
}

unsafe fn load_card_icon(w: HWND) -> HICON {
    let window_icon = window_own_icon(w);
    if !window_icon.is_null() {
        return CopyIcon(window_icon);
    }
    let aumid = win_aumid(w);
    if !aumid.is_empty() {
        let ic = aumid_icon(&aumid, sc(48));
        if !ic.is_null() {
            return ic;
        }
    }
    let exe = exe_of(w);
    let shell_icon = if exe.is_empty() || is_host(&exe) { null_mut() } else { item_icon(&exe, sc(48)) };
    if !shell_icon.is_null() {
        return shell_icon;
    }
    let fallback = icon_of(w);
    if fallback.is_null() { null_mut() } else { CopyIcon(fallback) }
}

pub(crate) unsafe fn register(host: HWND, wins: &[HWND], rects: &[RECT]) -> Vec<isize> {
    let icons = wins.iter().map(|&w| load_card_icon(w)).collect();
    CARD_ICONS.with_borrow_mut(|all| { all.insert(host as usize, icons); });
    wins.iter().zip(rects).map(|(&w, r)| {
        let mut id = 0isize;
        if IsIconic(w) != 0 || DwmRegisterThumbnail(host, w, &mut id) != 0 {
            return 0;
        }
        let mut src = SIZE { cx: 0, cy: 0 };
        DwmQueryThumbnailSourceSize(id, &mut src);
        let a = preview_area(r);
        let (aw, ah) = ((a.right - a.left) as f32, (a.bottom - a.top) as f32);
        let k = if src.cx > 0 && src.cy > 0 { (aw / src.cx as f32).min(ah / src.cy as f32).min(1.0) } else { 1.0 };
        let (dw, dh) = ((src.cx as f32 * k) as i32, (src.cy as f32 * k) as i32);
        let (dx, dy) = (a.left + (aw as i32 - dw) / 2, a.top + (ah as i32 - dh) / 2);
        let p = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT { left: dx, top: dy, right: dx + dw, bottom: dy + dh },
            rcSource: zeroed(),
            opacity: 255,
            fVisible: 1,
            fSourceClientAreaOnly: 0,
        };
        DwmUpdateThumbnailProperties(id, &p);
        id
    }).collect()
}

pub(crate) unsafe fn unregister(host: HWND, ids: &mut Vec<isize>) {
    if let Some(icons) = CARD_ICONS.with_borrow_mut(|all| all.remove(&(host as usize))) {
        for icon in icons { if !icon.is_null() { DestroyIcon(icon); } }
    }
    ids.drain(..).filter(|&id| id != 0).for_each(|id| {
        DwmUnregisterThumbnail(id);
    });
}

pub(crate) unsafe fn paint(host: HWND, wins: &[HWND], rects: &[RECT], ids: &[isize], sel: Option<usize>) {
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(host, &mut ps);
    let mut r: RECT = zeroed();
    GetClientRect(host, &mut r);
    if r.right <= 0 || r.bottom <= 0 {
        EndPaint(host, &ps);
        return;
    }
    thread_local! {
        static PV_BUF: RefCell<LayeredBuf> = const { RefCell::new(LayeredBuf::new()) };
        static PV_FONT: std::cell::Cell<(i32, HFONT)> = const { std::cell::Cell::new((0, null_mut())) };
    }
    let (mem, _, _) = PV_BUF.with_borrow_mut(|b| b.begin(wdc, r.right, r.bottom));
    let fh = sc(12);
    let (cfh, mut f) = PV_FONT.get();
    if cfh != fh || f.is_null() {
        if !f.is_null() {
            DeleteObject(f);
        }
        f = CreateFontW(-fh, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(ui_face()).as_ptr());
        PV_FONT.set((fh, f));
    }
    let of = SelectObject(mem, f);
    SetBkMode(mem, TRANSPARENT as i32);
    let fallback_ic = LoadIconW(null_mut(), IDI_APPLICATION);
    let mut title_buf = String::with_capacity(64);
    let mut b = [0u16; 256];
    CARD_ICONS.with_borrow(|all| {
        let icons = all.get(&(host as usize));
        for (k, (&w, c)) in wins.iter().zip(rects).enumerate() {
            let on = sel == Some(k);
            if on {
                fill(mem, *c, pal().accent, sc(12));
            } else {
                stroke_round(mem, *c, pal().line, 255, sc(12), 1.0);
            }
            fill(mem, RECT { left: c.left + sc(2), top: c.top + sc(2), right: c.right - sc(2), bottom: c.bottom - sc(2) }, if on { pal().active } else { pal().hover }, sc(10));
            let ic = icons.and_then(|v| v.get(k)).copied().filter(|h| !h.is_null()).unwrap_or(fallback_ic);
            let t = sc(TITLE);
            draw_icon(mem, c.left + sc(8), c.top + (t - sc(16)) / 2, sc(16), ic);
            let n = GetWindowTextW(w, b.as_mut_ptr(), 256).max(0) as usize;
            title_buf.clear();
            title_buf.extend(char::decode_utf16(b[..n].iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')));
            let x = close_rect(c);
            text(mem, &title_buf, RECT { left: c.left + sc(30), top: c.top, right: x.left - sc(4), bottom: c.top + t }, pal().text, 0);
            if on {
                text(mem, "\u{2715}", x, pal().text, DT_CENTER);
            }
            if ids.get(k).is_none_or(|&id| id == 0) {
                let a = preview_area(c);
                let s = sc(48).min(a.right - a.left).min(a.bottom - a.top).max(sc(16));
                draw_icon(mem, (a.left + a.right - s) / 2, (a.top + a.bottom - s) / 2, s, ic);
            }
        }
    });
    SelectObject(mem, of);
    EndPaint(host, &ps);
    LayeredBuf::present(host, r.right, r.bottom, mem);
}

thread_local! {
    pub(crate) static PVWND: Cell<HWND> = const { Cell::new(null_mut()) };
    static PV: RefCell<(Vec<HWND>, Vec<RECT>, Vec<isize>, Option<usize>)> = const { RefCell::new((Vec::new(), Vec::new(), Vec::new(), None)) };
    static PV_BTN: Cell<i32> = const { Cell::new(NONE) };
    static PV_AWAY: Cell<u64> = const { Cell::new(0) };
}

pub(crate) unsafe fn show_preview(btn: i32, cx: i32) -> bool {
    let pv = PVWND.get();
    if pv.is_null() || btn <= 0 {
        return false;
    }
    let wins: Vec<HWND> = BTNS.with_borrow(|b| b.get(btn as usize).map(|b| b.wins.iter().copied().filter(|&w| IsWindow(w) != 0).collect()).unwrap_or_default());
    if wins.is_empty() {
        return false;
    }
    PV_BTN.set(btn);
    PV_AWAY.set(0);
    layout(pv, wins, cx);
    SetTimer(pv, 1, 150, None);
    true
}

unsafe fn layout(pv: HWND, wins: Vec<HWND>, cx: i32) {
    let (scw, sch) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    let (rects, w, h) = grid(wins.len(), scw - sc(16), sch / 2, sc(CARD_W), sc(CARD_H), sc(GAP));
    let old = PV.take();
    let mut ids = old.2;
    unregister(pv, &mut ids);
    if vert() {
        let y = (cx - h / 2).clamp(sc(4), sch - h - sc(4));
        SetWindowPos(pv, HWND_TOPMOST, side_x(w), y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    } else {
        let x = (cx - w / 2).clamp(sc(4), scw - w - sc(4));
        SetWindowPos(pv, HWND_TOPMOST, x, if at_top() { bar_h() - bar_off() + sc(6) } else { bar_top() + bar_off() - h - sc(6) }, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
    let ids = register(pv, &wins, &rects);
    PV.set((wins, rects, ids, None));
    InvalidateRect(pv, null(), 0);
}

pub(crate) unsafe fn hide_preview() {
    let pv = PVWND.get();
    if pv.is_null() || IsWindowVisible(pv) == 0 {
        return;
    }
    KillTimer(pv, 1);
    ShowWindow(pv, SW_HIDE);
    let (_, _, mut ids, _) = PV.take();
    unregister(pv, &mut ids);
    PV_BTN.set(NONE);
}

pub(crate) unsafe extern "system" fn pv_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16) as i16 as i32);
    match msg {
        WM_PAINT => PV.with_borrow(|(w, r, ids, sel)| paint(hwnd, w, r, ids, *sel)),
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
        WM_MOUSEMOVE => {
            let k = PV.with_borrow(|p| card_at(&p.1, x, y));
            if PV.with_borrow_mut(|p| std::mem::replace(&mut p.3, k)) != k {
                InvalidateRect(hwnd, null(), 0);
            }
        }
        WM_LBUTTONUP | WM_MBUTTONUP => {
            let Some((k, w, r)) = PV.with_borrow(|p| card_at(&p.1, x, y).map(|k| (k, p.0[k], p.1[k]))) else { return 0 };
            let c = close_rect(&r);
            if msg == WM_MBUTTONUP || (x >= c.left && x < c.right && y >= c.top && y < c.bottom) {
                PostMessageW(w, WM_CLOSE, 0, 0);
                let mut rest = PV.with_borrow(|p| p.0.clone());
                rest.remove(k);
                if rest.is_empty() {
                    hide_preview();
                } else {
                    let mut wr: RECT = zeroed();
                    GetWindowRect(hwnd, &mut wr);
                    layout(hwnd, rest, if vert() { (wr.top + wr.bottom) / 2 } else { (wr.left + wr.right) / 2 });
                }
            } else {
                hide_preview();
                if IsIconic(w) != 0 {
                    ShowWindow(w, SW_RESTORE);
                }
                force_fg(w);
            }
        }
        WM_TIMER if wp == 1 => {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            let mut wr: RECT = zeroed();
            GetWindowRect(hwnd, &mut wr);
            let inside = pt.x >= wr.left && pt.x < wr.right && pt.y >= wr.top && pt.y < wr.bottom;
            if inside || HOT.get() == PV_BTN.get() {
                PV_AWAY.set(0);
            } else if PV_AWAY.get() == 0 {
                PV_AWAY.set(now_ms());
            } else if now_ms().saturating_sub(PV_AWAY.get()) > 300 {
                hide_preview();
            }
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::grid;

    #[test]
    fn closing_a_popup_releases_its_icon_copies() {
        unsafe {
            use super::*;
            let host = 1usize as HWND;
            let icon = CopyIcon(LoadIconW(null_mut(), IDI_APPLICATION));
            assert!(!icon.is_null());
            CARD_ICONS.with_borrow_mut(|all| { all.insert(host as usize, vec![icon]); });
            unregister(host, &mut Vec::new());
            assert!(CARD_ICONS.with_borrow(|all| !all.contains_key(&(host as usize))));
            let mut info: ICONINFO = zeroed();
            assert_eq!(GetIconInfo(icon, &mut info), 0);
        }
    }

    #[test]
    fn grid_balances_rows_and_centers_the_last_one() {
        let (r, tw, th) = grid(5, 350, 1000, 100, 50, 10);
        assert_eq!((tw, th), (320, 110));
        assert_eq!(r[0].left, 0);
        assert_eq!(r[3].left, 55);
        assert_eq!(r[4].top, 60);
    }

    #[test]
    fn grid_shrinks_cards_when_they_do_not_fit() {
        let (r, _, th) = grid(40, 500, 200, 100, 50, 10);
        assert_eq!(r.len(), 40);
        assert!(th <= 200, "{th}");
        assert!(r[0].right < 100);
    }

    #[test]
    fn grid_of_nothing_is_empty() {
        assert_eq!(grid(0, 100, 100, 10, 10, 1).0.len(), 0);
    }
}
