use crate::*;

pub(crate) use winrust_common::{ui::argb, wide, with_wide};

pub(crate) fn lerp(a: u32, b: u32, t: f32) -> u32 {
    let m = |sh: u32| (((a >> sh & 255) as f32) * (1.0 - t) + ((b >> sh & 255) as f32) * t) as u32;
    m(0) | m(8) << 8 | m(16) << 16
}

pub(crate) type Box4 = [f32; 4];

thread_local! {
    static FONT_CACHE: std::cell::RefCell<std::collections::HashMap<(i32, i32, bool), isize>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

pub(crate) struct Ui {
    dc: HDC,
    g: *mut GpGraphics,
    br: *mut GpSolidFill,
    pen: *mut GpPen,
    s: f32,
}
impl Ui {
    fn px(&self, v: f32) -> i32 {
        (v * self.s).round() as i32
    }
    unsafe fn round_path(&self, x: f32, y: f32, w: f32, h: f32, rad: f32) -> *mut GpPath {
        let mut path: *mut GpPath = null_mut();
        let d = (rad * 2.0).min(h).min(w).max(0.01);
        GdipCreatePath(FillModeAlternate, &mut path);
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
        path
    }
    unsafe fn shape(&self, b: Box4, rad: f32, color: u32) {
        let s = self.s;
        let (x, y, w, h) = (b[0] * s, b[1] * s, (b[2] - b[0]) * s, (b[3] - b[1]) * s);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        GdipSetSolidFillColor(self.br, color);
        if rad <= 0.0 {
            GdipFillRectangle(self.g, self.br as *mut GpBrush, x, y, w, h);
            return;
        }
        let path = self.round_path(x, y, w, h, rad * s);
        GdipFillPath(self.g, self.br as *mut GpBrush, path);
        GdipDeletePath(path);
    }
    unsafe fn round(&self, b: Box4, rad: f32, fill: u32, line: u32) {
        if line == fill {
            self.shape(b, rad, argb(fill, 255));
            return;
        }
        let s = self.s;
        let (x, y, w, h) = (b[0] * s, b[1] * s, (b[2] - b[0]) * s, (b[3] - b[1]) * s);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let path = self.round_path(x + 0.5, y + 0.5, (w - 1.0).max(0.5), (h - 1.0).max(0.5), (rad * s - 0.5).max(0.0));
        GdipSetSolidFillColor(self.br, argb(fill, 255));
        GdipFillPath(self.g, self.br as *mut GpBrush, path);
        GdipSetPenColor(self.pen, argb(line, 255));
        GdipDrawPath(self.g, self.pen, path);
        GdipDeletePath(path);
    }
    unsafe fn font(&self, size: f32, weight: i32, is_icon: bool) -> HFONT {
        let h = -self.px(size);
        FONT_CACHE.with_borrow_mut(|m| {
            *m.entry((h, weight, is_icon)).or_insert_with(|| {
                let face = if is_icon { winrust_common::style::icon_face() } else { winrust_common::style::sans_face() };
                with_wide(face, |w| CreateFontW(h, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, w)) as isize
            }) as HFONT
        })
    }
    unsafe fn text_in(&self, s: &str, b: Box4, size: f32, weight: i32, color: u32, flags: u32, is_icon: bool) {
        let f = self.font(size, weight, is_icon);
        let of = SelectObject(self.dc, f);
        SetTextColor(self.dc, color);
        SetBkMode(self.dc, TRANSPARENT as i32);
        let mut r = RECT { left: self.px(b[0]), top: self.px(b[1]), right: self.px(b[2]), bottom: self.px(b[3]) };
        with_wide(s, |w| DrawTextW(self.dc, w, -1, &mut r, flags | DT_NOPREFIX));
        SelectObject(self.dc, of);
    }
    unsafe fn text(&self, s: &str, b: Box4, size: f32, weight: i32, color: u32, flags: u32) {
        self.text_in(s, b, size, weight, color, flags, false);
    }
    unsafe fn icon(&self, s: &str, b: Box4, size: f32, weight: i32, color: u32, flags: u32) {
        self.text_in(s, b, size, weight, color, flags, true);
    }
    unsafe fn width(&self, s: &str, size: f32, weight: i32) -> f32 {
        let f = self.font(size, weight, false);
        let of = SelectObject(self.dc, f);
        let mut r: RECT = zeroed();
        with_wide(s, |w| DrawTextW(self.dc, w, -1, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX));
        SelectObject(self.dc, of);
        r.right as f32 / self.s
    }
}

pub(crate) const WIN_W: f32 = 800.0;
pub(crate) const WIN_H: f32 = 650.0;
const HEAD_H: f32 = 64.0;
const SIDE_W: f32 = 210.0;
const PAD: f32 = 32.0;
const X0: f32 = SIDE_W + PAD;
const X1: f32 = WIN_W - PAD;
const FOOT_Y: f32 = WIN_H - 68.0;
pub(crate) const BTN_H: f32 = 34.0;
const BTN_Y: f32 = WIN_H - 50.0;
pub(crate) const BTN_W: f32 = 130.0;
pub(crate) const N_READY: usize = {
    let (mut n, mut i) = (0, 0);
    while i < PLUGINS.len() {
        if PLUGINS[i].actions.is_some() && PLUGINS[i].bulk {
            n += 1;
        }
        i += 1;
    }
    n
};
pub(crate) const N_EXTRA: usize = {
    let (mut n, mut i) = (0, 0);
    while i < PLUGINS.len() {
        if PLUGINS[i].actions.is_some() && !PLUGINS[i].bulk {
            n += 1;
        }
        i += 1;
    }
    n
};
pub(crate) const ROW_H: f32 = 34.0;
const LIST_TOP: f32 = 180.0;
const EXTRA_SEC_Y: f32 = LIST_TOP + N_READY as f32 * ROW_H + 12.0;
const EXTRA_TOP: f32 = EXTRA_SEC_Y + 26.0;
const NOTE_Y: f32 = FOOT_Y - 50.0;
const _: () = assert!(EXTRA_TOP + N_EXTRA as f32 * ROW_H <= NOTE_Y, "the custom list runs into the note: make WIN_H taller");

pub(crate) const ID_MAIN: usize = 0;
pub(crate) const ID_ROLLBACK: usize = 1;
pub(crate) const ID_OK: usize = 2;
pub(crate) const ID_CANCEL: usize = 3;
pub(crate) const ID_SWITCH: usize = 4;
pub(crate) const ID_GEAR: usize = 5;
pub(crate) const ID_SHELL: usize = 7;
pub(crate) const ID_ALL: usize = 8;
const ID_BACK: usize = 10;
const ID_EXPRESS: usize = 11;
const ID_CUSTOM: usize = 12;
pub(crate) const ID_CHECK: usize = 100;
pub(crate) const STARTING: usize = usize::MAX - 1;

pub(crate) const HEAD: u32 = rgb(16, 16, 16);
const SIDE: u32 = rgb(24, 24, 24);
const ON_ACCENT: u32 = rgb(12, 12, 12);

thread_local! {
    static PAGE: Cell<u8> = const { Cell::new(1) };
    static CUSTOM: Cell<bool> = const { Cell::new(true) };
    static REMOVING_ALL: Cell<bool> = const { Cell::new(false) };
}
pub(crate) static RESULTS: Mutex<Vec<(usize, bool, Option<String>)>> = Mutex::new(Vec::new());

pub(crate) fn ready() -> impl Iterator<Item = usize> {
    (0..PLUGINS.len()).filter(|&i| PLUGINS[i].actions.is_some() && PLUGINS[i].bulk)
}

pub(crate) fn extras() -> impl Iterator<Item = usize> {
    (0..PLUGINS.len()).filter(|&i| PLUGINS[i].actions.is_some() && !PLUGINS[i].bulk)
}

pub(crate) fn rows() -> impl Iterator<Item = usize> {
    ready().chain(extras())
}

pub(crate) fn chosen() -> impl Iterator<Item = usize> {
    rows().filter(|&i| SELECTED[i].load(Relaxed))
}

pub(crate) fn diff() -> (Vec<usize>, Vec<usize>) {
    let mut buf = [0usize; N_READY + N_EXTRA];
    let mut n = 0;
    for i in rows() {
        buf[n] = i;
        n += 1;
    }
    plan(&buf[..n], |i| INSTALLED[i].load(Relaxed), |i| SELECTED[i].load(Relaxed))
}

pub(crate) fn reset_selection() {
    for i in ready() {
        SELECTED[i].store(selected_by_default(PLUGINS[i].name, INSTALLED[i].load(Relaxed)), Relaxed);
    }
    for i in extras() {
        SELECTED[i].store(INSTALLED[i].load(Relaxed), Relaxed);
    }
}

fn express_adds() -> Vec<usize> {
    rows().filter(|&i| !INSTALLED[i].load(Relaxed) && express_pick(PLUGINS[i].name, PLUGINS[i].bulk, false)).collect()
}

pub(crate) static STEPS: Mutex<Vec<(usize, bool)>> = Mutex::new(Vec::new());

fn primary_box() -> Box4 {
    [X1 - BTN_W, BTN_Y, X1, BTN_Y + BTN_H]
}

fn back_box() -> Box4 {
    let p = primary_box();
    [p[0] - 10.0 - BTN_W, p[1], p[0] - 10.0, p[3]]
}

pub(crate) fn rollback_box(_w: f32) -> Box4 {
    [X0 - 4.0, BTN_Y, X0 + 170.0, BTN_Y + BTN_H]
}

fn card_box(k: usize) -> Box4 {
    let t = 206.0 + k as f32 * 104.0;
    [X0, t, X1, t + 90.0]
}

pub(crate) fn row_box(k: usize, _w: f32) -> Box4 {
    let t = if k < N_READY { LIST_TOP + k as f32 * ROW_H } else { EXTRA_TOP + (k - N_READY) as f32 * ROW_H };
    [X0, t, X1, t + ROW_H]
}

const NAME_X: f32 = X0 + 54.0;
const DESC_X: f32 = X0 + 232.0;

pub(crate) fn gear_box(w: f32) -> Option<(usize, Box4)> {
    let (k, i) = ready().enumerate().find(|&(_, i)| PLUGINS[i].settings)?;
    let r = row_box(k, w);
    Some((i, [DESC_X - 30.0, r[1] + 5.0, DESC_X - 6.0, r[3] - 5.0]))
}

pub(crate) fn all_box(_w: f32) -> Box4 {
    [X1 - 160.0, LIST_TOP - 28.0, X1, LIST_TOP - 6.0]
}

pub(crate) fn switch_box(w: f32) -> Box4 {
    let (d, _, _) = dialog_boxes(w);
    let mid = d[1] + 72.0 + 32.0;
    [d[2] - 24.0 - 16.0 - 40.0, mid - 12.0, d[2] - 24.0 - 16.0, mid + 12.0]
}

pub(crate) fn shell_switch_box(w: f32) -> Box4 {
    let b = switch_box(w);
    [b[0], b[1] + 64.0, b[2], b[3] + 64.0]
}

fn explorer_idx() -> Option<usize> {
    PLUGINS.iter().position(|p| p.name == crate::selection::RUST_EXPLORER)
}

pub(crate) fn dialog_boxes(w: f32) -> (Box4, Box4, Box4) {
    let (dw, dh) = (520.0, 440.0);
    let (l, t) = ((w - dw) / 2.0, (WIN_H - dh) / 2.0);
    let (bt, r) = (t + dh - 20.0 - BTN_H, l + dw - 20.0);
    ([l, t, l + dw, t + dh], [r - BTN_W - 8.0 - BTN_W, bt, r - BTN_W - 8.0, bt + BTN_H], [r - BTN_W, bt, r, bt + BTN_H])
}

pub(crate) fn scale() -> f32 {
    unsafe {
        let dc = GetDC(null_mut());
        let mut s = GetDeviceCaps(dc, LOGPIXELSX as i32) as f32 / 96.0;
        ReleaseDC(null_mut(), dc);
        let mut wa: RECT = zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as *mut _, 0) != 0 {
            let pad = GetSystemMetrics(SM_CXPADDEDBORDER);
            let frame_h = GetSystemMetrics(SM_CYCAPTION) + 2 * (GetSystemMetrics(SM_CYFIXEDFRAME) + pad) + 32;
            let frame_w = 2 * (GetSystemMetrics(SM_CXFIXEDFRAME) + pad) + 48;
            s = s.min(((wa.bottom - wa.top - frame_h) as f32 / WIN_H).max(0.5));
            s = s.min(((wa.right - wa.left - frame_w) as f32 / WIN_W).max(0.5));
        }
        s
    }
}

pub(crate) fn client_scale(cw: i32, ch: i32) -> f32 {
    if cw <= 0 || ch <= 0 {
        return scale();
    }
    (cw as f32 / WIN_W).min(ch as f32 / WIN_H).max(0.5)
}

pub(crate) unsafe fn button(ui: &Ui, b: Box4, label: &str, kind: u8, hot: bool) {
    let (fill, line, fg, wt) = match kind {
        0 => (ACCENT, ACCENT, ON_ACCENT, 700),
        1 => (BG, rgb(120, 120, 120), TEXT, 600),
        2 => (BAD, BAD, rgb(255, 255, 255), 700),
        _ => (rgb(52, 52, 52), rgb(52, 52, 52), lerp(DIM, BG, 0.3), 600),
    };
    let (fill, line) = if hot && kind != 3 { (lerp(fill, rgb(255, 255, 255), 0.12), lerp(line, rgb(255, 255, 255), 0.2)) } else { (fill, line) };
    ui.round(b, 4.0, fill, line);
    ui.text(&label.to_uppercase(), b, 12.5, wt, fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
}

unsafe fn switch(ui: &Ui, b: Box4, on: bool, dim: bool) {
    let track = if on { ACCENT } else { rgb(72, 72, 74) };
    let track = if dim { lerp(track, PANEL, 0.5) } else { track };
    let h = b[3] - b[1];
    ui.round(b, h / 2.0, track, track);
    let k = h - 4.0;
    let x = if on { b[2] - 2.0 - k } else { b[0] + 2.0 };
    ui.round([x, b[1] + 2.0, x + k, b[3] - 2.0], k / 2.0, rgb(255, 255, 255), rgb(255, 255, 255));
}

unsafe fn check(ui: &Ui, b: Box4, on: bool, dim: bool) {
    let c = if dim { lerp(ACCENT, BG, 0.5) } else { ACCENT };
    if on {
        ui.round(b, 2.0, c, c);
        ui.icon("\u{E73E}", b, 11.0, 700, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    } else {
        ui.round(b, 2.0, BG, if dim { BORDER } else { rgb(120, 120, 120) });
    }
}

unsafe fn radio(ui: &Ui, x: f32, y: f32, on: bool) {
    let r = 9.0;
    ui.round([x - r, y - r, x + r, y + r], r, BG, if on { ACCENT } else { rgb(120, 120, 120) });
    if on {
        ui.round([x - 5.0, y - 5.0, x + 5.0, y + 5.0], 5.0, ACCENT, ACCENT);
    }
}

fn tile_color(name: &str) -> u32 {
    match name {
        "rust-search" => rgb(10, 132, 255),
        "rtaskmgr-win32" => rgb(48, 209, 88),
        "topbar" => rgb(94, 92, 230),
        "rust-explorer" => rgb(255, 159, 10),
        "photoview" => rgb(255, 55, 95),
        "debloat" => rgb(100, 210, 255),
        "purge" => rgb(255, 69, 58),
        _ => rgb(142, 142, 147),
    }
}

unsafe fn title(ui: &Ui, t: &str, sub: &str) {
    ui.text(t, [X0, 90.0, X1, 122.0], 22.0, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    ui.text(sub, [X0, 122.0, X1, 144.0], 13.0, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
}

unsafe fn note(ui: &Ui, fallback: &str) {
    let g = MSG.lock().unwrap_or_else(|e| e.into_inner());
    let (m, fg) = if g.0.is_empty() { (fallback, DIM) } else if g.1 { (g.0.as_str(), GOOD) } else { (g.0.as_str(), BAD) };
    ui.text(m, [X0, NOTE_Y, X1, FOOT_Y - 8.0], 12.0, 400, fg, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
}

fn names(v: &[usize]) -> String {
    v.iter().map(|&i| PLUGINS[i].name).collect::<Vec<_>>().join(", ")
}

thread_local! {
    static BACKBUF: Cell<(HDC, HBITMAP, *mut GpGraphics, *mut GpSolidFill, *mut GpPen, i32, i32)> = const { Cell::new((null_mut(), null_mut(), null_mut(), null_mut(), null_mut(), 0, 0)) };
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}

unsafe fn get_backbuf(hdc: HDC, cw: i32, ch: i32) -> (HDC, *mut GpGraphics, *mut GpSolidFill, *mut GpPen) {
    let (mut mem, mut bmp, mut g, mut br, mut pen, bw, bh) = BACKBUF.get();
    if mem.is_null() {
        mem = CreateCompatibleDC(hdc);
    }
    if br.is_null() {
        GdipCreateSolidFill(0, &mut br);
    }
    if pen.is_null() {
        GdipCreatePen1(0, 1.0, UnitPixel, &mut pen);
    }
    if bmp.is_null() || bw != cw || bh != ch {
        if !g.is_null() {
            GdipDeleteGraphics(g);
            g = null_mut();
        }
        let new_bmp = CreateCompatibleBitmap(hdc, cw, ch);
        let prev = SelectObject(mem, new_bmp);
        if !bmp.is_null() {
            DeleteObject(prev);
        }
        bmp = new_bmp;
        GdipCreateFromHDC(mem, &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        BACKBUF.set((mem, bmp, g, br, pen, cw, ch));
    } else if g.is_null() {
        GdipCreateFromHDC(mem, &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        BACKBUF.set((mem, bmp, g, br, pen, cw, ch));
    }
    (mem, g, br, pen)
}

pub(crate) unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let (cw, ch) = (rc.right, rc.bottom);
    if cw <= 0 || ch <= 0 {
        EndPaint(hwnd, &ps);
        return;
    }
    let (mem, g, br, pen) = get_backbuf(hdc, cw, ch);
    let s = client_scale(cw, ch);
    SetDCBrushColor(mem, BG);
    FillRect(mem, &rc, GetStockObject(DC_BRUSH));
    let ui = Ui { dc: mem, g, br, pen, s };
    let w = WIN_W;
    let hover = HOVER.get();
    let hot = |id: usize| hover == Some(id);
    let hair = 1.0 / s;
    let page = PAGE.get();
    let custom = CUSTOM.get();

    ui.shape([0.0, 0.0, w, HEAD_H], 0.0, argb(HEAD, 255));
    ui.shape([0.0, HEAD_H, w, HEAD_H + 3.0], 0.0, argb(ACCENT, 255));
    let mark = [PAD - 6.0, 19.0, PAD + 20.0, 45.0];
    ui.round(mark, 3.0, ACCENT, ACCENT);
    ui.text("W", mark, 16.0, 800, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    ui.text("WinRust", [PAD + 32.0, 0.0, X1, HEAD_H], 20.0, 700, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
    let tw = ui.width("WinRust", 20.0, 700);
    ui.text("Setup", [PAD + 40.0 + tw, 0.0, X1 - 24.0, HEAD_H], 16.0, 300, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);

    ui.shape([0.0, HEAD_H + 3.0, SIDE_W, WIN_H + 1.0], 0.0, argb(SIDE, 255));
    let steps_names = ["Components", "Installing", "Finish"];
    let step_idx = page.saturating_sub(1);
    for (k, label) in steps_names.iter().enumerate() {
        let y = 100.0 + k as f32 * 46.0;
        let cur = k as u8 == step_idx;
        let done = (k as u8) < step_idx;
        if cur {
            ui.shape([0.0, y, 4.0, y + 38.0], 0.0, argb(ACCENT, 255));
            ui.shape([4.0, y, SIDE_W, y + 38.0], 0.0, argb(rgb(34, 34, 34), 255));
        }
        let c = [22.0, y + 8.0, 44.0, y + 30.0];
        if done {
            ui.round(c, 11.0, ACCENT, ACCENT);
            ui.icon("\u{E73E}", c, 10.0, 700, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        } else {
            let line = if cur { ACCENT } else { rgb(80, 80, 80) };
            ui.round(c, 11.0, SIDE, line);
            ui.text(["1", "2", "3"][k], c, 11.0, 600, if cur { TEXT } else { DIM }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        let fg = if cur { TEXT } else if done { lerp(TEXT, DIM, 0.3) } else { DIM };
        ui.text(label, [56.0, y, SIDE_W - 8.0, y + 38.0], 13.5, if cur { 600 } else { 400 }, fg, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }
    ui.text(&format!("v{}", env!("CARGO_PKG_VERSION")), [20.0, WIN_H - 48.0, SIDE_W - 12.0, WIN_H - 24.0], 12.0, 400, lerp(DIM, SIDE, 0.25), DT_LEFT | DT_SINGLELINE | DT_VCENTER);
    ui.shape([SIDE_W, FOOT_Y, w, FOOT_Y + hair], 0.0, argb(BORDER, 255));

    let n_inst = rows().filter(|&i| INSTALLED[i].load(Relaxed)).count();
    match page {
        0 => {
            let sub = if n_inst == 0 {
                "Choose how to install WinRust.".to_string()
            } else {
                format!("{n_inst} components installed. Choose what to do.")
            };
            title(&ui, "Installation options", &sub);
            let admin = winrust_common::proc::elevated();
            let checks = [
                (admin, if admin { "Running as administrator" } else { "Not an administrator: run the installer as administrator" }),
                (true, "All changes can be undone via Remove all"),
            ];
            for (k, (ok, text)) in checks.iter().enumerate() {
                let y = 156.0 + k as f32 * 22.0;
                ui.icon(if *ok { "\u{E73E}" } else { "\u{E711}" }, [X0, y, X0 + 16.0, y + 20.0], 11.0, 700, if *ok { ACCENT } else { BAD }, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text(text, [X0 + 22.0, y, X1, y + 20.0], 12.5, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            let adds = express_adds();
            let express_text = if adds.is_empty() {
                "Everything recommended is already there: the installed components are updated from this version, settings are kept.".to_string()
            } else {
                format!("Installs {}. Explorer and the Windows clean-up are left alone, nothing is removed.", names(&adds))
            };
            let cards = [
                ("Express (recommended)", express_text),
                ("Custom (advanced)", "Pick the components yourself, including the Explorer replacement, the Windows clean-up and removing what is installed.".to_string()),
            ];
            for (k, (head, text)) in cards.iter().enumerate() {
                let b = card_box(k);
                let sel = custom == (k == 1);
                let h = hot(ID_EXPRESS + k);
                ui.round(b, 3.0, if sel { lerp(BG, ACCENT, 0.07) } else if h { rgb(44, 44, 44) } else { PANEL }, if sel { ACCENT } else { BORDER });
                radio(&ui, b[0] + 24.0, b[1] + 26.0, sel);
                ui.text(head, [b[0] + 46.0, b[1] + 14.0, b[2] - 16.0, b[1] + 38.0], 15.0, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                ui.text(text, [b[0] + 46.0, b[1] + 40.0, b[2] - 16.0, b[3] - 8.0], 12.5, 400, DIM, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
            }
            note(&ui, "");
            if n_inst > 0 {
                let rb = rollback_box(w);
                ui.text("Remove WinRust", rb, 13.0, 400, if hot(ID_ROLLBACK) { lerp(BAD, TEXT, 0.3) } else { BAD }, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
            }
            let label = if custom {
                "Next"
            } else if adds.is_empty() && n_inst > 0 {
                "Update"
            } else {
                "Install"
            };
            button(&ui, primary_box(), label, 0, hot(ID_MAIN));
        }
        1 => {
            title(&ui, "Custom installation", "Tick what should be installed. Unticking an installed component removes it.");
            ui.text("Components", [X0, LIST_TOP - 28.0, X1, LIST_TOP - 6.0], 12.0, 700, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
            let all_on = chosen().filter(|&i| PLUGINS[i].bulk).count() == N_READY;
            ui.text(if all_on { "Select none" } else { "Select all" }, all_box(w), 12.5, 600, if hot(ID_ALL) { TEXT } else { ACCENT }, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            if N_EXTRA > 0 {
                ui.text("Extras", [X0, EXTRA_SEC_Y, X1, EXTRA_SEC_Y + 22.0], 12.0, 700, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text("Only when you choose", [X0, EXTRA_SEC_Y, X1, EXTRA_SEC_Y + 22.0], 12.0, 400, lerp(DIM, BG, 0.3), DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
            let (to_rm, to_add) = diff();
            let gear = gear_box(w);
            for (k, i) in rows().enumerate() {
                let p = &PLUGINS[i];
                let r = row_box(k, w);
                let mid = (r[1] + r[3]) / 2.0;
                if hot(ID_CHECK + i) {
                    ui.shape(r, 0.0, argb(rgb(44, 44, 44), 255));
                }
                ui.shape([r[0], r[3] - hair, r[2], r[3]], 0.0, argb(rgb(48, 48, 48), 255));
                check(&ui, [X0 + 8.0, mid - 9.0, X0 + 26.0, mid + 9.0], SELECTED[i].load(Relaxed), locked(i));
                ui.icon(p.glyph, [X0 + 32.0, mid - 10.0, X0 + 50.0, mid + 10.0], 14.0, 400, tile_color(p.name), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                ui.text(p.name, [NAME_X, r[1], DESC_X - 34.0, r[3]], 13.5, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                if let Some((_, gb)) = gear.filter(|&(gi, _)| gi == i) {
                    ui.icon("\u{E713}", gb, 14.0, 400, if hot(ID_GEAR) { TEXT } else { DIM }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                }
                let installed = INSTALLED[i].load(Relaxed);
                let status = if to_rm.contains(&i) {
                    Some(("Will be removed", BAD))
                } else if to_add.contains(&i) {
                    Some(("Will be installed", ACCENT))
                } else if installed {
                    Some((if p.bulk { "Installed" } else { "Applied" }, DIM))
                } else {
                    None
                };
                let mut right = r[2] - 8.0;
                if let Some((st, c)) = status {
                    let sw = ui.width(st, 12.0, 400);
                    ui.text(st, [right - sw - 2.0, r[1], right, r[3]], 12.0, 400, c, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
                    right -= sw + 14.0;
                }
                let desc = p.desc;
                let short = desc.split(". ").next().unwrap_or(desc).trim_end_matches('.');
                ui.text(short, [DESC_X, r[1], right, r[3]], 12.0, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            match hover.filter(|&h| h >= ID_CHECK).map(|h| h - ID_CHECK) {
                Some(i) => ui.text(PLUGINS[i].desc, [X0, NOTE_Y, X1, FOOT_Y - 8.0], 12.0, 400, DIM, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS),
                None => note(&ui, "Point at a component to read what it changes."),
            }
            if n_inst > 0 {
                button(&ui, back_box(), "Uninstall all", 2, hot(ID_ROLLBACK));
            }
            let (label, kind) = match (to_rm.len(), to_add.len()) {
                (0, 0) if n_inst == 0 => ("Nothing selected".to_string(), 3),
                (0, 0) => ("Update".to_string(), 0),
                (0, a) => (format!("Install ({a})"), 0),
                (r, 0) => (format!("Remove ({r})"), 2),
                (r, a) => (format!("Apply +{a} −{r}"), 0),
            };
            button(&ui, primary_box(), &label, kind, hot(ID_MAIN) && kind != 3);
        }
        2 => {
            let all = REMOVING_ALL.get();
            title(
                &ui,
                if all { "Removing WinRust" } else { "Installing WinRust" },
                "Keep the window open until it finishes. The screen may flicker while Windows parts are swapped.",
            );
            let steps = STEPS.lock().unwrap_or_else(|e| e.into_inner());
            let job = JOB.load(Relaxed);
            let pos = steps.iter().position(|&(i, _)| i == job);
            let total = steps.len().max(1);
            let bar = [X0, 186.0, X1, 194.0];
            ui.round(bar, 1.0, rgb(58, 58, 58), rgb(58, 58, 58));
            let (now_text, frac) = if pos.is_none() {
                let t = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()) % 1400) as f32 / 1400.0;
                let (a, b) = ((-0.3 + 1.3 * t).max(0.0), (1.3 * t).min(1.0));
                let bw = bar[2] - bar[0];
                ui.round([bar[0] + bw * a, bar[1], bar[0] + bw * b, bar[3]], 1.0, ACCENT, ACCENT);
                (if all { "Bringing the stock Windows parts back…" } else { "Preparing…" }.to_string(), None)
            } else {
                let q = pos.unwrap_or(0);
                let f = (q as f32 + 0.5) / total as f32;
                ui.round([bar[0], bar[1], bar[0] + (bar[2] - bar[0]) * f, bar[3]], 1.0, ACCENT, ACCENT);
                let (i, install) = steps[q];
                let verb = if install { "Installing" } else { "Removing" };
                (format!("{verb}: {}  ({} / {total})", PLUGINS[i].name, q + 1), Some(f))
            };
            ui.text(&now_text, [X0, 160.0, X1 - 60.0, 182.0], 13.0, 400, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            if let Some(f) = frac {
                ui.text(&format!("{}%", (f * 100.0) as u32), [X1 - 60.0, 160.0, X1, 182.0], 13.0, 600, ACCENT, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
            for (k, &(i, install)) in steps.iter().enumerate() {
                let y = 216.0 + k as f32 * 30.0;
                let (glyph, gc, word) = match pos {
                    Some(q) if k < q => ("\u{E73E}", ACCENT, if install { "Done" } else { "Removed" }),
                    Some(q) if k == q => ("\u{E768}", ACCENT, if install { "Installing…" } else { "Removing…" }),
                    _ => ("\u{E915}", rgb(90, 90, 90), "Queued"),
                };
                ui.icon(glyph, [X0, y, X0 + 18.0, y + 24.0], 11.0, 700, gc, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text(PLUGINS[i].name, [X0 + 26.0, y, X0 + 260.0, y + 24.0], 13.0, 400, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text(word, [X0 + 260.0, y, X1, y + 24.0], 12.5, 400, DIM, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
        }
        _ => {
            let msg_guard = MSG.lock().unwrap_or_else(|e| e.into_inner());
            let (msg, ok) = (msg_guard.0.as_str(), msg_guard.1);
            let all = REMOVING_ALL.get();
            let results = RESULTS.lock().unwrap_or_else(|e| e.into_inner());
            let c = [X0, 92.0, X0 + 44.0, 136.0];
            let col = if ok { ACCENT } else { BAD };
            ui.round(c, 22.0, col, col);
            ui.icon(if ok { "\u{E73E}" } else { "\u{E711}" }, c, 18.0, 700, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let head = match (ok, all) {
                (true, true) => "WinRust has been removed",
                (true, false) => "WinRust setup has finished",
                (false, _) => "Finished, but not everything worked",
            };
            ui.text(head, [c[2] + 16.0, 92.0, X1, 118.0], 21.0, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            let sub = if !ok {
                "The error is shown below. The rest is installed; go back to the options to try again."
            } else if all {
                "Windows Search, the stock Task Manager, the taskbar and Explorer are back."
            } else {
                "The components are already running. No restart is needed."
            };
            ui.text(sub, [c[2] + 16.0, 118.0, X1, 138.0], 12.5, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            if !results.is_empty() {
                ui.text("Component", [X0, 166.0, X0 + 260.0, 188.0], 12.0, 700, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text("Status", [X0 + 260.0, 166.0, X1, 188.0], 12.0, 700, DIM, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
                ui.shape([X0, 190.0, X1, 190.0 + hair], 0.0, argb(BORDER, 255));
            }
            for (k, (i, install, err)) in results.iter().enumerate() {
                let y = 196.0 + k as f32 * 28.0;
                if y + 26.0 > NOTE_Y {
                    break;
                }
                ui.text(PLUGINS[*i].name, [X0, y, X0 + 260.0, y + 26.0], 13.0, 400, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                let (word, c) = match err {
                    Some(_) => ("Failed", BAD),
                    None if *install => ("Installed", ACCENT),
                    None => ("Removed", DIM),
                };
                ui.text(word, [X0 + 260.0, y, X1, y + 26.0], 13.0, 600, c, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
            ui.text(msg, [X0, NOTE_Y, X1, FOOT_Y - 8.0], 12.0, 400, if ok { DIM } else { BAD }, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
            button(&ui, back_box(), "Options", 1, hot(ID_BACK));
            button(&ui, primary_box(), "Close", 0, hot(ID_MAIN));
        }
    }

    DIALOG.with_borrow(|d| {
        if let Some(d) = d {
            ui.shape([0.0, 0.0, w, WIN_H + 1.0], 0.0, argb(0, 160));
            let (db, cancel, okb) = dialog_boxes(w);
            ui.round(db, 8.0, PANEL, BORDER);
            ui.shape([db[0], db[1], db[2], db[1] + 3.0], 0.0, argb(if d.danger { BAD } else { ACCENT }, 255));
            ui.text(&d.title, [db[0] + 24.0, db[1] + 22.0, db[2] - 24.0, db[1] + 50.0], 16.0, 600, TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            ui.text(&d.text, [db[0] + 24.0, db[1] + 60.0, db[2] - 24.0, cancel[1] - 14.0], 12.5, 400, DIM, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
            button(&ui, cancel, "Cancel", 1, hot(ID_CANCEL));
            button(&ui, okb, &d.ok, if d.danger { 2 } else { 0 }, hot(ID_OK));
        }
    });

    if let Some(i) = SETTINGS.get() {
        ui.shape([0.0, 0.0, w, WIN_H + 1.0], 0.0, argb(0, 160));
        let (d, _, closeb) = dialog_boxes(w);
        ui.round(d, 8.0, PANEL, BORDER);
        ui.shape([d[0], d[1], d[2], d[1] + 3.0], 0.0, argb(ACCENT, 255));
        ui.text(&format!("{} settings", PLUGINS[i].name), [d[0] + 24.0, d[1] + 22.0, d[2] - 24.0, d[1] + 50.0], 16.0, 600, TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        let group = [d[0] + 24.0, d[1] + 72.0, d[2] - 24.0, d[1] + 72.0 + 128.0];
        let inner = rgb(56, 56, 56);
        ui.round(group, 3.0, inner, inner);
        ui.shape([group[0] + 16.0, group[1] + 64.0, group[2], group[1] + 64.0 + hair], 0.0, argb(rgb(72, 72, 74), 255));
        let rows = [
            ("Hide the OneDrive icon", "Sync keeps working", od_hidden(), switch_box(w)),
            ("Replace Explorer", "rust-explorer becomes the shell", explorer_idx().is_some_and(|e| SELECTED[e].load(Relaxed)), shell_switch_box(w)),
        ];
        for (k, (name, note, on, sw)) in rows.into_iter().enumerate() {
            let t = group[1] + k as f32 * 64.0;
            ui.text(name, [group[0] + 16.0, t + 12.0, sw[0] - 12.0, t + 32.0], 14.0, 500, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
            ui.text(note, [group[0] + 16.0, t + 32.0, sw[0] - 12.0, t + 52.0], 12.0, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            switch(&ui, sw, on, false);
        }
        button(&ui, closeb, "Done", 0, hot(ID_CANCEL));
    }

    BitBlt(hdc, 0, 0, cw, ch, mem, 0, 0, SRCCOPY);
    EndPaint(hwnd, &ps);
}

#[derive(Clone, Copy)]
pub(crate) enum Act {
    InstallAll,
    Rollback,
}
pub(crate) struct Dialog {
    title: String,
    text: String,
    ok: String,
    danger: bool,
    act: Act,
}
thread_local! { pub(crate) static DIALOG: std::cell::RefCell<Option<Dialog>> = const { std::cell::RefCell::new(None) }; }

pub(crate) unsafe fn hit(hwnd: HWND, lp: isize) -> Option<usize> {
    let (x, y) = ((lp & 0xFFFF) as i16 as f32, ((lp >> 16) & 0xFFFF) as i16 as f32);
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let s = client_scale(rc.right, rc.bottom);
    let w = WIN_W;
    let inside = |b: Box4| x >= b[0] * s && x <= b[2] * s && y >= b[1] * s && y <= b[3] * s;
    if SETTINGS.get().is_some() {
        let (_, _, closeb) = dialog_boxes(w);
        return if inside(switch_box(w)) { Some(ID_SWITCH) } else if inside(shell_switch_box(w)) { Some(ID_SHELL) } else if inside(closeb) { Some(ID_CANCEL) } else { None };
    }
    if DIALOG.with_borrow(|d| d.is_some()) {
        let (_, cancel, ok) = dialog_boxes(w);
        return if inside(ok) { Some(ID_OK) } else if inside(cancel) { Some(ID_CANCEL) } else { None };
    }
    if BUSY.load(Relaxed) {
        return None;
    }
    let n_inst = rows().any(|i| INSTALLED[i].load(Relaxed));
    match PAGE.get() {
        0 => {
            if inside(card_box(0)) {
                return Some(ID_EXPRESS);
            }
            if inside(card_box(1)) {
                return Some(ID_CUSTOM);
            }
            if n_inst && inside(rollback_box(w)) {
                return Some(ID_ROLLBACK);
            }
            inside(primary_box()).then_some(ID_MAIN)
        }
        1 => {
            if gear_box(w).is_some_and(|(_, b)| inside(b)) {
                return Some(ID_GEAR);
            }
            if inside(all_box(w)) {
                return Some(ID_ALL);
            }
            if let Some(i) = rows().enumerate().find_map(|(k, i)| inside(row_box(k, w)).then_some(i)) {
                return Some(ID_CHECK + i);
            }
            if n_inst && inside(back_box()) {
                return Some(ID_ROLLBACK);
            }
            let unchanged = rows().all(|i| INSTALLED[i].load(Relaxed) == SELECTED[i].load(Relaxed));
            (inside(primary_box()) && !(unchanged && !n_inst)).then_some(ID_MAIN)
        }
        3 => {
            if inside(back_box()) {
                return Some(ID_BACK);
            }
            inside(primary_box()).then_some(ID_MAIN)
        }
        _ => None,
    }
}

pub(crate) fn start(hwnd: HWND, which: usize, job: impl FnOnce() -> Result<String, String> + Send + 'static) {
    JOB.store(which, Relaxed);
    BUSY.store(true, Relaxed);
    REMOVING_ALL.set(which == PLUGINS.len());
    RESULTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    MSG.lock().unwrap_or_else(|e| e.into_inner()).0.clear();
    HOVER.set(None);
    PAGE.set(2);
    unsafe {
        SetTimer(hwnd, 1, 40, None);
        InvalidateRect(hwnd, null(), 0);
    }
    let h = hwnd as isize;
    thread::spawn(move || {
        *MSG.lock().unwrap_or_else(|e| e.into_inner()) = match job() {
            Ok(m) => (m, true),
            Err(e) => (format!("Error: {e}"), false),
        };
        refresh();
        BUSY.store(false, Relaxed);
        unsafe { PostMessageW(h as HWND, WM_DONE, 0, 0) };
    });
}

pub(crate) unsafe fn finished(hwnd: HWND) {
    KillTimer(hwnd, 1);
    PAGE.set(3);
    InvalidateRect(hwnd, null(), 0);
}

pub(crate) fn apply(h: isize) -> Result<String, String> {
    let (rm, add) = diff();
    let update = rm.is_empty() && add.is_empty();
    let steps: Vec<(usize, bool)> = if update {
        ready().filter(|&i| INSTALLED[i].load(Relaxed)).map(|i| (i, true)).collect()
    } else {
        rm.iter().map(|&i| (i, false)).chain(add.iter().map(|&i| (i, true))).collect()
    };
    *STEPS.lock().unwrap_or_else(|e| e.into_inner()) = steps.clone();
    let (mut added, mut removed, mut failed, mut warned) = (Vec::new(), Vec::new(), Vec::new(), false);
    for (i, install) in steps {
        JOB.store(i, Relaxed);
        unsafe { InvalidateRect(h as HWND, null(), 0) };
        let r = if install { install_plugin(i) } else { uninstall_plugin(i) };
        match r {
            Ok(_) => {
                warned |= !WARN.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
                if install { added.push(PLUGINS[i].name) } else { removed.push(PLUGINS[i].name) }
                RESULTS.lock().unwrap_or_else(|e| e.into_inner()).push((i, install, None));
            }
            Err(e) => {
                failed.push(format!("{}: {e}", PLUGINS[i].name));
                RESULTS.lock().unwrap_or_else(|e| e.into_inner()).push((i, install, Some(e)));
            }
        }
        refresh();
    }
    STEPS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let mut done = vec![];
    if !added.is_empty() {
        done.push(format!("{} {}", if update { "Updated:" } else { "Installed:" }, added.join(", ")));
    }
    if !removed.is_empty() {
        done.push(format!("Removed: {}", removed.join(", ")));
    }
    let done = done.join(". ");
    if !failed.is_empty() {
        return Err(format!("{}.{}", failed.join("; "), if done.is_empty() { String::new() } else { format!(" {done}.") }));
    }
    let note = if warned { " There are warnings (see %ProgramData%\\WinRust\\winrust.log)." } else { "" };
    Ok(format!("{done}.{note}"))
}

fn locked(i: usize) -> bool {
    PLUGINS[i].name == "purge" && INSTALLED[i].load(Relaxed)
}

pub(crate) unsafe fn ask(hwnd: HWND, d: Dialog) {
    DIALOG.set(Some(d));
    HOVER.set(None);
    InvalidateRect(hwnd, null(), 0);
}

pub(crate) unsafe fn close_dialog(hwnd: HWND, run: bool) {
    let Some(d) = DIALOG.take() else { return };
    HOVER.set(None);
    InvalidateRect(hwnd, null(), 0);
    if !run {
        return;
    }
    match d.act {
        Act::Rollback => {
            let h = hwnd as isize;
            start(hwnd, PLUGINS.len(), move || rollback_core(Some(h)))
        }
        Act::InstallAll => {
            let h = hwnd as isize;
            start(hwnd, STARTING, move || apply(h))
        }
    }
}

pub(crate) unsafe fn click(hwnd: HWND, id: usize) {
    if SETTINGS.get().is_some() {
        if id == ID_SWITCH {
            if let Err(e) = set_od_hidden(!od_hidden()) {
                *MSG.lock().unwrap_or_else(|e| e.into_inner()) = (format!("Error: {e}"), false);
            }
        } else if id == ID_SHELL {
            if let Some(e) = explorer_idx() {
                let on = !SELECTED[e].fetch_xor(true, Relaxed);
                if let Some((other, v)) = linked(PLUGINS[e].name, on) {
                    PLUGINS.iter().position(|p| p.name == other).inspect(|&o| SELECTED[o].store(v, Relaxed));
                }
            }
        } else if id == ID_CANCEL {
            SETTINGS.set(None);
        }
        HOVER.set(None);
        InvalidateRect(hwnd, null(), 0);
        return;
    }
    if id == ID_OK || id == ID_CANCEL {
        return close_dialog(hwnd, id == ID_OK);
    }
    if BUSY.load(Relaxed) {
        return;
    }
    let repaint = || InvalidateRect(hwnd, null(), 0);
    match (PAGE.get(), id) {
        (0, ID_EXPRESS) | (0, ID_CUSTOM) => {
            CUSTOM.set(id == ID_CUSTOM);
            repaint();
        }
        (0, ID_MAIN) if CUSTOM.get() => {
            MSG.lock().unwrap_or_else(|e| e.into_inner()).0.clear();
            PAGE.set(1);
            HOVER.set(None);
            repaint();
        }
        (0, ID_MAIN) => {
            for i in rows() {
                SELECTED[i].store(express_pick(PLUGINS[i].name, PLUGINS[i].bulk, INSTALLED[i].load(Relaxed)), Relaxed);
            }
            let h = hwnd as isize;
            start(hwnd, STARTING, move || apply(h));
        }
        (0 | 1, ID_ROLLBACK) => ask(hwnd, Dialog {
            title: "Remove WinRust?".into(),
            text: "All components will be removed. Windows Search, the stock Task Manager, the taskbar and regular Explorer come back.".into(),
            ok: "Remove all".into(),
            danger: true,
            act: Act::Rollback,
        }),
        (1, ID_GEAR) => {
            SETTINGS.set(PLUGINS.iter().position(|p| p.settings));
            HOVER.set(None);
            repaint();
        }
        (1, ID_ALL) => {
            let on = chosen().filter(|&i| PLUGINS[i].bulk).count() < N_READY;
            ready().for_each(|i| SELECTED[i].store(on, Relaxed));
            repaint();
        }
        (1, i) if i >= ID_CHECK => {
            let i = i - ID_CHECK;
            if !locked(i) {
                let on = !SELECTED[i].fetch_xor(true, Relaxed);
                if let Some((other, v)) = linked(PLUGINS[i].name, on) {
                    PLUGINS.iter().position(|p| p.name == other).inspect(|&o| SELECTED[o].store(v, Relaxed));
                }
            }
            repaint();
        }
        (1, ID_MAIN) if diff() == (vec![], vec![]) => ask(hwnd, Dialog {
            title: "Update WinRust?".into(),
            text: "The installed components will be reinstalled from this version of the installer. Settings are kept.".into(),
            ok: "Update".into(),
            danger: false,
            act: Act::InstallAll,
        }),
        (1, ID_MAIN) => {
            let (rm, add) = diff();
            let mut text = String::new();
            if !add.is_empty() {
                text.push_str(&format!("To be installed: {}.", names(&add)));
            }
            if !rm.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&format!("To be removed: {} (the stock Windows parts come back).", names(&rm)));
            }
            for &i in add.iter().filter(|&&i| PLUGINS[i].name == RUST_EXPLORER || !PLUGINS[i].bulk) {
                text.push_str(&format!("\n\n{}: {}", PLUGINS[i].name, PLUGINS[i].warn));
            }
            let purge = add.iter().any(|&i| PLUGINS[i].name == "purge");
            let (title, ok, danger) = if purge {
                ("Remove apps for good?", "Remove for good", true)
            } else if add.is_empty() {
                ("Remove the selected?", "Remove", true)
            } else if rm.is_empty() {
                ("Install the selected?", "Install", false)
            } else {
                ("Apply the changes?", "Apply", false)
            };
            ask(hwnd, Dialog { title: title.into(), text, ok: ok.into(), danger, act: Act::InstallAll });
        }
        (3, ID_BACK) => {
            reset_selection();
            MSG.lock().unwrap_or_else(|e| e.into_inner()).0.clear();
            PAGE.set(1);
            HOVER.set(None);
            repaint();
        }
        (3, ID_MAIN) => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

pub(crate) unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => return 1,
        WM_MOUSEMOVE => {
            let h = hit(hwnd, lp);
            if h != HOVER.get() {
                HOVER.set(h);
                InvalidateRect(hwnd, null(), 0);
            }
            if !TRACKING.replace(true) {
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut t);
            }
        }
        WM_SETCURSOR if (lp & 0xFFFF) as u32 == HTCLIENT => {
            SetCursor(LoadCursorW(null_mut(), if HOVER.get().is_some() { IDC_HAND } else { IDC_ARROW }));
            return 1;
        }
        WM_MOUSELEAVE => {
            TRACKING.set(false);
            HOVER.set(None);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_LBUTTONUP => {
            if let Some(i) = hit(hwnd, lp) {
                click(hwnd, i);
            }
        }
        WM_KEYDOWN if SETTINGS.get().is_some() && wp as u16 == VK_ESCAPE => {
            SETTINGS.set(None);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_KEYDOWN if DIALOG.with_borrow(|d| d.is_some()) => match wp as u16 {
            VK_ESCAPE => close_dialog(hwnd, false),
            VK_RETURN if DIALOG.with_borrow(|d| d.as_ref().is_some_and(|d| !d.danger)) => close_dialog(hwnd, true),

            _ => {}
        },
        WM_DONE => finished(hwnd),
        0x02E0 => {
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_TIMER if wp == 1 => {
            InvalidateRect(hwnd, null(), 0);
        }
        WM_CLOSE if BUSY.load(Relaxed) => {}
        WM_DESTROY => {
            FONT_CACHE.with_borrow_mut(|m| {
                for (_, f) in m.drain() {
                    DeleteObject(f as _);
                }
            });
            let (mem, bmp, g, br, pen, _, _) = BACKBUF.replace((null_mut(), null_mut(), null_mut(), null_mut(), null_mut(), 0, 0));
            if !pen.is_null() {
                GdipDeletePen(pen);
            }
            if !br.is_null() {
                GdipDeleteBrush(br as *mut GpBrush);
            }
            if !g.is_null() {
                GdipDeleteGraphics(g);
            }
            if !mem.is_null() {
                DeleteDC(mem);
            }
            if !bmp.is_null() {
                DeleteObject(bmp);
            }
            PostQuitMessage(0)
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

