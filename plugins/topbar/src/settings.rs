use super::*;

thread_local! {
    static SET_HWND: Cell<HWND> = const { Cell::new(null_mut()) };
    static SET_OPENING: Cell<bool> = const { Cell::new(false) };
    static SET_T0: Cell<u64> = const { Cell::new(0) };
    static SET_HOT: Cell<i32> = const { Cell::new(-1) };
    static SET_RESET: Cell<bool> = const { Cell::new(false) };
    static SET_LBUF: RefCell<LayeredBuf> = const { RefCell::new(LayeredBuf::new()) };
    static SET_FONTS: Cell<(u32, [HFONT; 4])> = const { Cell::new((0, [null_mut(); 4])) };
}

const SW: i32 = 544;
const SH: i32 = 216;

pub(crate) unsafe fn style_menu() {
    let cur = SET_HWND.get();
    if !cur.is_null() && IsWindow(cur) != 0 {
        DestroyWindow(cur);
        SET_HWND.set(null_mut());
        return;
    }
    SET_HOT.set(-1);
    SET_RESET.set(false);
    let hi = GetModuleHandleW(null());
    static CLS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let cls = CLS.get_or_init(|| wide("topbar_settings_panel"));
    static REG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !REG.swap(true, Relaxed) {
        let wc = WNDCLASSW { lpfnWndProc: Some(set_proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: cls.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);
    }
    let s = winrust_common::ui::fit_scale(SW, SH, SCALE.get(), sc(16), sc(64));
    let (w, h) = ((SW as f32 * s).round() as i32, (SH as f32 * s).round() as i32);
    let wa = winrust_common::ui::work_area();
    let x = ((wa.right + wa.left - w) / 2).max(wa.left + sc(8));
    let y = if at_top() { bar_h() + sc(12) } else { (bar_top() - sc(12) - h).max(wa.top + sc(8)) };
    let hw = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED, cls.as_ptr(), null(), WS_POPUP, x, y, w, h, null_mut(), null_mut(), hi, null());
    SET_HWND.set(hw);
    SET_T0.set(now_ms());
    SET_OPENING.set(true);
    SetWindowPos(hw, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
    force_fg(hw);
    SetWindowPos(hw, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    SET_OPENING.set(false);
}

fn sscale(cw: i32, ch: i32) -> f32 {
    if cw <= 0 || ch <= 0 {
        winrust_common::ui::fit_scale(SW, SH, SCALE.get(), sc(16), sc(64))
    } else {
        (cw as f32 / SW as f32).min(ch as f32 / SH as f32).max(0.5)
    }
}

fn sr(l: i32, t: i32, r_: i32, b: i32, s: f32) -> RECT {
    RECT { left: (l as f32 * s).round() as i32, top: (t as f32 * s).round() as i32, right: (r_ as f32 * s).round() as i32, bottom: (b as f32 * s).round() as i32 }
}

const POS_IDS: i32 = 10;
const BSIZE_IDS: i32 = 20;
const BOP_IDS: i32 = 30;
const SSIZE_IDS: i32 = 40;
const SOP_IDS: i32 = 50;
const BEH_IDS: i32 = 60;
const ACT_COLORS: i32 = 90;
const ACT_RESET: i32 = 91;

const BAR_OPS: [u8; 7] = [100, 85, 70, 50, 30, 15, 0];
const BAR_OP_LBLS: [&str; 7] = ["100%", "85%", "70%", "50%", "30%", "15%", "0%"];
const START_OPS: [u8; 5] = [100, 85, 70, 50, 30];
const START_OP_LBLS: [&str; 5] = ["100%", "85%", "70%", "50%", "30%"];

fn for_each_set_item(s: f32, mut f: impl FnMut(RECT, i32, &'static str, bool, bool)) {
    use i18n::style::{self as st, Side};
    f(sr(SW - 160, 10, SW - 88, 34, s), ACT_COLORS, "Colors…", false, false);
    let armed = SET_RESET.get();
    f(sr(SW - 82, 10, SW - 14, 34, s), ACT_RESET, if armed { "Confirm?" } else { "Reset" }, armed, true);

    let pos_lbls = ["Bottom", "Left", "Right"];
    let sides = [Side::Bottom, Side::Left, Side::Right];
    let cur_side = st::side();
    for (k, &lbl) in pos_lbls.iter().enumerate() {
        let x = 108 + k as i32 * 64;
        f(sr(x, 54, x + 60, 80, s), POS_IDS + k as i32, lbl, cur_side == sides[k], false);
    }
    let sz_lbls = ["S", "M", "L"];
    let cur_bsz = st::bar_size();
    for (k, &lbl) in sz_lbls.iter().enumerate() {
        let x = 382 + k as i32 * 50;
        f(sr(x, 54, x + 46, 80, s), BSIZE_IDS + k as i32, lbl, cur_bsz == k as u8, false);
    }

    let cur_bop = st::bar_opacity();
    for (k, (&op, &lbl)) in BAR_OPS.iter().zip(&BAR_OP_LBLS).enumerate() {
        let x = 108 + k as i32 * 60;
        f(sr(x, 94, x + 56, 120, s), BOP_IDS + k as i32, lbl, cur_bop == op, false);
    }

    let cur_ssz = st::start_size();
    for (k, &lbl) in sz_lbls.iter().enumerate() {
        let x = 108 + k as i32 * 44;
        f(sr(x, 134, x + 40, 160, s), SSIZE_IDS + k as i32, lbl, cur_ssz == k as u8, false);
    }
    let cur_sop = st::start_opacity();
    for (k, (&op, &lbl)) in START_OPS.iter().zip(&START_OP_LBLS).enumerate() {
        let x = 304 + k as i32 * 46;
        f(sr(x, 134, x + 42, 160, s), SOP_IDS + k as i32, lbl, cur_sop == op, false);
    }

    let behs = [
        ("Auto-hide", st::autohide()),
        ("Align left", st::align_left()),
        ("Alt+Tab monitor", st::alttab_per_monitor()),
    ];
    for (k, (lbl, on)) in behs.iter().enumerate() {
        let x = 108 + k as i32 * 142;
        f(sr(x, 174, x + 136, 200, s), BEH_IDS + k as i32, lbl, *on, false);
    }
}

fn set_hit(s: f32, x: i32, y: i32) -> Option<i32> {
    let mut hit = None;
    for_each_set_item(s, |r, id, _, _, _| {
        if hit.is_none() && x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            hit = Some(id);
        }
    });
    hit
}

unsafe fn set_paint(hw: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(hw, &mut ps);
    let mut cr: RECT = zeroed();
    GetClientRect(hw, &mut cr);
    if cr.right <= 0 || cr.bottom <= 0 {
        EndPaint(hw, &ps);
        return;
    }
    let s = sscale(cr.right, cr.bottom);
    let (dc, _, _) = SET_LBUF.with_borrow_mut(|b| b.begin(wdc, cr.right, cr.bottom));
    let border_col = if style().light() { mix(pal().bg, pal().line, 0.9) } else { mix(pal().bg, pal().text, 0.14) };
    let rad_w = rr(16);
    fill(dc, cr, pal().bg, rad_w);
    stroke_round(dc, cr, border_col, 255, rad_w, 1.0);
    SetBkMode(dc, TRANSPARENT as i32);

    let s_bits = s.to_bits();
    let [f_title, f_lbl, f_btn, f_glyph] = {
        let (cb, mut fa) = SET_FONTS.get();
        if cb != s_bits || fa[0].is_null() {
            for f in fa {
                if !f.is_null() {
                    DeleteObject(f);
                }
            }
            let (face, gface) = (wide(ui_face()), wide(winrust_common::style::icon_face()));
            let mk = |f: &[u16], px: i32, wt: i32| CreateFontW(-((px as f32 * s).round() as i32), 0, 0, 0, wt, 0, 0, 0, 1, 0, 0, 5, 0, f.as_ptr());
            fa = [mk(&face, 14, 600), mk(&face, 12, 400), mk(&face, 12, 600), mk(&gface, 15, 400)];
            SET_FONTS.set((s_bits, fa));
        }
        fa
    };
    let of = SelectObject(dc, f_glyph);
    text(dc, "\u{E713}", sr(14, 10, 36, 34, s), pal().accent, DT_CENTER);
    SelectObject(dc, f_title);
    text(dc, "Bar settings", sr(42, 10, SW - 170, 34, s), pal().text, DT_LEFT);
    fill(dc, sr(0, 44, SW, 45, s), mix(pal().bg, pal().text, 0.08), 0);

    SelectObject(dc, f_lbl);
    let labels = [
        (sr(14, 54, 104, 80, s), "Position"),
        (sr(318, 54, 378, 80, s), "Size"),
        (sr(14, 94, 104, 120, s), "Bar opacity"),
        (sr(14, 134, 104, 160, s), "Start menu"),
        (sr(248, 134, 300, 160, s), "Opacity"),
        (sr(14, 174, 104, 200, s), "Behavior"),
    ];
    for (rc, lbl) in labels {
        text(dc, lbl, rc, pal().dim, DT_LEFT);
    }

    let hot = SET_HOT.get();
    let rad = (4.0 * s).round() as i32;
    let card = i18n::style::card();
    let card_border = mix(card, pal().text, 0.12);
    for_each_set_item(s, |rc, id, lbl, active, danger| {
        let is_hot = hot == id;
        let bg = if active {
            if danger { BAD } else { pal().accent }
        } else if is_hot {
            mix(card, pal().text, 0.08)
        } else {
            card
        };
        fill(dc, rc, bg, rad);
        if !active {
            stroke_round(dc, rc, card_border, 255, rad, 1.0);
        }
        let fg = if active {
            if danger { rgb(255, 255, 255) } else { i18n::style::on_accent() }
        } else if danger {
            BAD
        } else {
            pal().text
        };
        SelectObject(dc, if active { f_btn } else { f_lbl });
        text(dc, lbl, rc, fg, DT_CENTER);
    });

    SelectObject(dc, of);
    EndPaint(hw, &ps);
    LayeredBuf::present(hw, cr.right, cr.bottom, dc);
}

unsafe extern "system" fn set_proc(hw: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    use i18n::style::{self as st, Side};
    match msg {
        WM_PAINT => set_paint(hw),
        WM_ERASEBKGND => return 1,
        WM_ACTIVATE if wp & 0xffff == 0 => {
            if SET_OPENING.get() {
                return 0;
            }
            if now_ms().saturating_sub(SET_T0.get()) < 150 {
                SetWindowPos(hw, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                force_fg(hw);
                return 0;
            }
            SET_HWND.set(null_mut());
            DestroyWindow(hw);
        }
        WM_KEYDOWN if wp == VK_ESCAPE as usize => {
            SET_HWND.set(null_mut());
            DestroyWindow(hw);
        }
        WM_MOUSEMOVE => {
            let mut cr: RECT = zeroed();
            GetClientRect(hw, &mut cr);
            let s = sscale(cr.right, cr.bottom);
            let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16 & 0xffff) as i16 as i32);
            let h = set_hit(s, x, y).unwrap_or(-1);
            if SET_HOT.replace(h) != h {
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_LBUTTONDOWN => {
            let mut cr: RECT = zeroed();
            GetClientRect(hw, &mut cr);
            let s = sscale(cr.right, cr.bottom);
            let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16 & 0xffff) as i16 as i32);
            if let Some(id) = set_hit(s, x, y) {
                let mut moved = false;
                match id {
                    x if (POS_IDS..POS_IDS + 3).contains(&x) => {
                        st::set_side([Side::Bottom, Side::Left, Side::Right][(x - POS_IDS) as usize]);
                        moved = true;
                    }
                    x if (BSIZE_IDS..BSIZE_IDS + 3).contains(&x) => {
                        st::set_bar_size((x - BSIZE_IDS) as u8);
                        moved = true;
                    }
                    x if (BOP_IDS..BOP_IDS + 7).contains(&x) => {
                        st::set_bar_opacity(BAR_OPS[(x - BOP_IDS) as usize]);
                    }
                    x if (SSIZE_IDS..SSIZE_IDS + 3).contains(&x) => {
                        st::set_start_size((x - SSIZE_IDS) as u8);
                    }
                    x if (SOP_IDS..SOP_IDS + 5).contains(&x) => {
                        st::set_start_opacity(START_OPS[(x - SOP_IDS) as usize]);
                    }
                    x if x == BEH_IDS => {
                        st::set_autohide(!st::autohide());
                        moved = true;
                    }
                    x if x == BEH_IDS + 1 => {
                        st::set_align_left(!st::align_left());
                    }
                    x if x == BEH_IDS + 2 => {
                        st::set_alttab_per_monitor(!st::alttab_per_monitor());
                    }
                    ACT_COLORS => {
                        SET_HWND.set(null_mut());
                        DestroyWindow(hw);
                        colors_page();
                        return 0;
                    }
                    ACT_RESET => {
                        if SET_RESET.get() {
                            st::reset_all();
                            SET_RESET.set(false);
                            moved = true;
                        } else {
                            SET_RESET.set(true);
                            InvalidateRect(hw, null(), 0);
                            return 0;
                        }
                    }
                    _ => {}
                }
                if id != ACT_RESET {
                    SET_RESET.set(false);
                }
                relook(moved);
                InvalidateRect(hw, null(), 0);
            }
        }
        _ => return DefWindowProcW(hw, msg, wp, lp),
    }
    0
}

pub(crate) unsafe fn gear_page(glyph: &'static str, title: &str, sub: &str, items: Vec<fly::MItem>, pick: impl FnOnce(usize) + 'static) {
    let bar = BAR.load(Relaxed) as HWND;
    fly::menu(tip_cx(&bar_layout(bar), GEAR), Some((null_mut(), glyph, title.to_string(), sub.to_string())), items, pick);
}

pub(crate) fn back() -> fly::MItem {
    fly::MItem::new("‹   Back")
}

pub(crate) unsafe fn relook(moved: bool) {
    let bar = BAR.load(Relaxed) as HWND;
    place(bar);
    if moved {
        bar_step(bar);
    }
    apply_opacity(bar);
    InvalidateRect(bar, null(), 0);
}

pub(crate) unsafe fn apply_opacity(hwnd: HWND) {
    let op = i18n::style::bar_opacity();
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED as isize));
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_LAYERED as isize));
    if op >= 100 {
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
    }
}

pub(crate) unsafe fn colors_page() {
    use windows_sys::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
    const PCT: [u8; 7] = [100, 85, 70, 50, 30, 15, 0];
    let colors = [
        "Background",
        "Lines and borders",
        "Hover",
        "Pressed and selected",
        "Accent",
        "Text",
        "Secondary text",
    ];
    let cur = i18n::style::custom_colors().unwrap_or([u32::MAX; 7]);
    let p = i18n::style::pal();
    let now = [p.bg, p.line, p.hover, p.active, p.accent, p.text, p.dim];
    let mut items = vec![back(), fly::MItem::head("Colors (dot on the right: changed by you)").sep()];
    items.extend(colors.iter().enumerate().map(|(i, n)| fly::MItem::new(*n).swatch(now[i]).on(cur[i] != u32::MAX)));
    items.push(fly::MItem::head("Bar opacity"));
    items.extend(PCT.iter().map(|&v| fly::MItem::new(match v {
        100 => "100 %, solid".to_string(),
        0 => "0 %, completely transparent".to_string(),
        _ => format!("{v} %"),
    }).on(i18n::style::bar_opacity() == v)));
    items.push(fly::MItem::new("Back to the theme's colors").danger().sep());
    let (c0, o0) = (2, 3 + colors.len());
    gear_page("\u{E790}", "Colors and transparency", "On top of the chosen theme", items, move |i| {
        let bar = BAR.load(Relaxed) as HWND;
        if i == 0 {
            return style_menu();
        } else if (c0..c0 + colors.len()).contains(&i) {
            static CUST: std::sync::Mutex<[u32; 16]> = std::sync::Mutex::new([0xFFFFFF; 16]);
            let mut cust = CUST.lock().unwrap();
            let mut cc: CHOOSECOLORW = zeroed();
            cc.lStructSize = size_of::<CHOOSECOLORW>() as u32;
            cc.hwndOwner = bar;
            cc.rgbResult = now[i - c0];
            cc.lpCustColors = cust.as_mut_ptr();
            cc.Flags = CC_RGBINIT | CC_FULLOPEN;
            if ChooseColorW(&mut cc) != 0 {
                i18n::style::set_custom_color(i - c0, Some(cc.rgbResult));
            }
        } else if let Some(&v) = i.checked_sub(o0).and_then(|k| PCT.get(k)) {
            i18n::style::set_bar_opacity(v);
        } else {
            i18n::style::reset_custom();
        }
        apply_opacity(bar);
        InvalidateRect(bar, null(), 0);
        colors_page();
    });
}
