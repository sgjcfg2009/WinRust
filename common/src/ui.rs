use std::{ptr::null_mut, sync::OnceLock};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::{
        Gdi::HDC,
        GdiPlus::{
            FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
            GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawPath,
            GdipFillPath, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GpBrush, GpGraphics, GpPath,
            GpPen, GpSolidFill, PixelOffsetModeHalf, SmoothingModeAntiAlias, UnitPixel,
        },
    },
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA, SystemParametersInfoW},
};

static FLUENT_FONT_DATA: &[u8] = include_bytes!("../assets/Segoe Fluent Icons.ttf");
static FLUENT_FONT_LOADED: OnceLock<bool> = OnceLock::new();

#[link(name = "gdi32")]
unsafe extern "system" {
    fn AddFontMemResourceEx(pb_font: *const core::ffi::c_void, cb_font: u32, pdv: *mut core::ffi::c_void, pc_fonts: *mut u32) -> *mut core::ffi::c_void;
}

pub fn ensure_fluent_icons() -> bool {
    *FLUENT_FONT_LOADED.get_or_init(|| unsafe {
        let mut count = 0u32;
        !AddFontMemResourceEx(FLUENT_FONT_DATA.as_ptr().cast(), FLUENT_FONT_DATA.len() as u32, null_mut(), &mut count).is_null() && count > 0
    })
}

pub mod tokens {
    pub const RADIUS_OVERLAY: i32 = 8;
    pub const RADIUS_CONTROL: i32 = 4;
    pub const GAP_XS: i32 = 4;
    pub const GAP_SM: i32 = 8;
    pub const GAP_MD: i32 = 12;
    pub const GAP_LG: i32 = 16;
    pub const STROKE: i32 = 1;
}

pub mod icons {
    pub const WIFI: &str = "\u{E701}";
    pub const BLUETOOTH: &str = "\u{E702}";
    pub const AIRPLANE: &str = "\u{E709}";
    pub const ENERGY_SAVER: &str = "\u{E945}";
    pub const BRIGHTNESS: &str = "\u{E706}";
    pub const VOLUME_HIGH: &str = "\u{E767}";
    pub const VOLUME_MUTE: &str = "\u{E74F}";
    pub const SPEAKER: &str = "\u{E7F5}";
    pub const HEADPHONES: &str = "\u{E7F6}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
    pub const CHEVRON_LEFT: &str = "\u{E76B}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const SEARCH: &str = "\u{E721}";
    pub const KEYBOARD: &str = "\u{E765}";
    pub const POWER: &str = "\u{E7E8}";
    pub const LOCK: &str = "\u{E72E}";
    pub const SLEEP: &str = "\u{E708}";
    pub const RESTART: &str = "\u{E777}";
    pub const CHECK: &str = "\u{E73E}";
    pub const CLOSE: &str = "\u{E711}";
    pub const USER: &str = "\u{E77B}";
    pub const BATTERY_CHARGE: &str = "\u{E83E}";
    pub const BATTERY: &str = "\u{E83F}";
    pub const EXPLORER: &str = "\u{EC50}";
    pub const TASKMGR: &str = "\u{E9D9}";
    pub const ETHERNET: &str = "\u{E839}";
}

pub fn work_area() -> RECT {
    unsafe {
        let mut wa: RECT = std::mem::zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as *mut _, 0) != 0 && wa.right > wa.left && wa.bottom > wa.top {
            return wa;
        }
        RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN).max(320), bottom: GetSystemMetrics(SM_CYSCREEN).max(240) }
    }
}

pub fn fit_scale(base_w: i32, base_h: i32, dpi_scale: f32, margin_w: i32, margin_h: i32) -> f32 {
    let wa = work_area();
    let aw = ((wa.right - wa.left) - margin_w).max(220) as f32;
    let ah = ((wa.bottom - wa.top) - margin_h).max(200) as f32;
    dpi_scale.min(aw / base_w.max(1) as f32).min(ah / base_h.max(1) as f32).max(0.55)
}

pub const fn argb(c: u32, a: u8) -> u32 {
    (a as u32) << 24 | (c & 255) << 16 | (c & 0xFF00) | (c >> 16 & 255)
}

unsafe fn round_path(x: f32, y: f32, w: f32, h: f32, rad: f32) -> *mut GpPath {
    let mut path: *mut GpPath = null_mut();
    unsafe {
        GdipCreatePath(FillModeAlternate, &mut path);
        let d = (rad * 2.0).min(w).min(h).max(0.01);
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
    }
    path
}

unsafe fn with_graphics(dc: HDC, f: impl FnOnce(*mut GpGraphics)) {
    let mut g: *mut GpGraphics = null_mut();
    if unsafe { GdipCreateFromHDC(dc, &mut g) } != 0 || g.is_null() {
        return;
    }
    unsafe {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        f(g);
        GdipDeleteGraphics(g);
    }
}

pub unsafe fn fill_round(dc: HDC, rc: RECT, rad: f32, color: u32, alpha: u8) {
    let (w, h) = ((rc.right - rc.left) as f32, (rc.bottom - rc.top) as f32);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    unsafe {
        with_graphics(dc, |g| {
            let path = round_path(rc.left as f32, rc.top as f32, w, h, rad);
            let mut br: *mut GpSolidFill = null_mut();
            GdipCreateSolidFill(argb(color, alpha), &mut br);
            GdipFillPath(g, br as *mut GpBrush, path);
            GdipDeleteBrush(br as *mut GpBrush);
            GdipDeletePath(path);
        });
    }
}

pub unsafe fn stroke_round(dc: HDC, rc: RECT, rad: f32, fill: u32, stroke: u32, stroke_w: f32) {
    let (w, h) = ((rc.right - rc.left) as f32, (rc.bottom - rc.top) as f32);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    unsafe {
        with_graphics(dc, |g| {
            let inset = stroke_w * 0.5;
            let path = round_path(rc.left as f32 + inset, rc.top as f32 + inset, (w - stroke_w).max(0.5), (h - stroke_w).max(0.5), (rad - inset).max(0.0));
            let mut br: *mut GpSolidFill = null_mut();
            GdipCreateSolidFill(argb(fill, 255), &mut br);
            GdipFillPath(g, br as *mut GpBrush, path);
            GdipDeleteBrush(br as *mut GpBrush);
            if stroke != fill && stroke_w > 0.0 {
                let mut pen: *mut GpPen = null_mut();
                GdipCreatePen1(argb(stroke, 255), stroke_w, UnitPixel, &mut pen);
                GdipDrawPath(g, pen, path);
                GdipDeletePen(pen);
            }
            GdipDeletePath(path);
        });
    }
}

#[derive(Clone, Copy)]
pub struct LayoutBox {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl LayoutBox {
    pub fn rect(self) -> RECT {
        RECT { left: self.left, top: self.top, right: self.right, bottom: self.bottom }
    }
    pub fn inset(self, dx: i32, dy: i32) -> Self {
        Self { left: self.left + dx, top: self.top + dy, right: self.right - dx, bottom: self.bottom - dy }
    }
    pub fn grid(self, cols: usize, rows: usize, col_gap: i32, row_gap: i32) -> Vec<RECT> {
        let w = (self.right - self.left - col_gap * (cols.saturating_sub(1) as i32)).max(0) / cols.max(1) as i32;
        let h = (self.bottom - self.top - row_gap * (rows.saturating_sub(1) as i32)).max(0) / rows.max(1) as i32;
        let mut out = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                let l = self.left + c as i32 * (w + col_gap);
                let t = self.top + r as i32 * (h + row_gap);
                out.push(RECT { left: l, top: t, right: l + w, bottom: t + h });
            }
        }
        out
    }
}
