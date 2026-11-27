use std::{cell::RefCell, collections::HashMap, ptr::null_mut};
use winapi::shared::windef::{HBRUSH, HDC, HFONT, HGDIOBJ, POINT, RECT};
use winapi::um::{wingdi::*, winuser::*};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn c(self) -> u32 {
        self.0 as u32 | (self.1 as u32) << 8 | (self.2 as u32) << 16
    }
    pub fn lerp(self, o: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, o.0), m(self.1, o.1), m(self.2, o.2))
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub bg: Rgb,
    pub nav: Rgb,
    pub panel: Rgb,
    pub text: Rgb,
    pub dim: Rgb,
    pub border: Rgb,
    pub sel: Rgb,
    pub hover: Rgb,
    pub accent: Rgb,
    pub heat0: Rgb,
    pub heat1: Rgb,
}

pub fn theme(dark: bool) -> Theme {
    if dark {
        Theme {
            dark,
            bg: Rgb(0, 0, 0),
            nav: Rgb(8, 8, 8),
            panel: Rgb(16, 16, 16),
            text: Rgb(255, 255, 255),
            dim: Rgb(160, 160, 160),
            border: Rgb(32, 32, 32),
            sel: Rgb(28, 28, 28),
            hover: Rgb(20, 20, 20),
            accent: Rgb(96, 205, 255),
            heat0: Rgb(12, 24, 42),
            heat1: Rgb(12, 112, 204),
        }
    } else {
        Theme {
            dark,
            bg: Rgb(249, 249, 249),
            nav: Rgb(243, 243, 243),
            panel: Rgb(255, 255, 255),
            text: Rgb(27, 27, 27),
            dim: Rgb(97, 97, 97),
            border: Rgb(229, 229, 229),
            sel: Rgb(234, 234, 234),
            hover: Rgb(240, 240, 240),
            accent: Rgb(0, 120, 212),
            heat0: Rgb(234, 243, 252),
            heat1: Rgb(86, 160, 232),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct R {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl R {
    pub fn new(l: i32, t: i32, r: i32, b: i32) -> R {
        R { l, t, r, b }
    }
    pub fn w(&self) -> i32 {
        self.r - self.l
    }
    pub fn h(&self) -> i32 {
        self.b - self.t
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
    pub fn inset(&self, d: i32) -> R {
        R::new(self.l + d, self.t + d, self.r - d, self.b - d)
    }
    pub fn rc(&self) -> RECT {
        RECT { left: self.l, top: self.t, right: self.r, bottom: self.b }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Font {
    Ui,
    Small,
    Semibold,
    Title,
    Big,
    Huge,
    Icons,
    IconsSm,
}

pub struct Fonts {
    f: [HFONT; 8],
    pens: RefCell<HashMap<(i32, u32), isize>>,
}

pub(crate) use winrust_common::wide;

fn mk(face: &str, px: i32, weight: i32) -> HFONT {
    let w = wide(face);
    unsafe { CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, w.as_ptr()) }
}

impl Fonts {
    pub fn new(s: f64) -> Fonts {
        let p = |x: f64| (x * s).round() as i32;
        let sans = winrust_common::style::sans_face();
        let icon = winrust_common::style::icon_face();
        Fonts {
            f: [
                mk(sans, p(13.0), 400),
                mk(sans, p(12.0), 400),
                mk(sans, p(13.0), 600),
                mk(sans, p(22.0), 600),
                mk(sans, p(20.0), 400),
                mk(sans, p(30.0), 300),
                mk(icon, p(16.0), 400),
                mk(icon, p(13.0), 400),
            ],
            pens: RefCell::new(HashMap::new()),
        }
    }
    pub fn get(&self, f: Font) -> HFONT {
        self.f[f as usize]
    }
    fn pen(&self, width: i32, color: u32) -> HGDIOBJ {
        let mut m = self.pens.borrow_mut();
        *m.entry((width, color)).or_insert_with(|| unsafe { CreatePen(PS_SOLID as i32, width, color) } as isize) as HGDIOBJ
    }
    pub fn rebuild(&mut self, s: f64) {
        let mut next = Fonts::new(s);
        std::mem::swap(&mut self.f, &mut next.f);
        std::mem::swap(&mut self.pens, &mut next.pens);
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for f in self.f {
            unsafe { DeleteObject(f as HGDIOBJ) };
        }
        for (_, p) in self.pens.get_mut().drain() {
            unsafe { DeleteObject(p as HGDIOBJ) };
        }
    }
}

fn with_utf16<R2>(s: &str, f: impl FnOnce(&[u16]) -> R2) -> R2 {
    thread_local! { static WBUF: std::cell::RefCell<Vec<u16>> = const { std::cell::RefCell::new(Vec::new()) }; }
    WBUF.with_borrow_mut(|b| {
        b.clear();
        b.extend(s.encode_utf16());
        f(b)
    })
}

pub struct G<'a> {
    pub dc: HDC,
    pub s: f64,
    pub th: Theme,
    pub fonts: &'a Fonts,
}

impl<'a> G<'a> {
    pub fn px(&self, v: f64) -> i32 {
        (v * self.s).round() as i32
    }

    pub fn fill(&self, r: R, c: Rgb) {
        unsafe {
            SetDCBrushColor(self.dc, c.c());
            FillRect(self.dc, &r.rc(), GetStockObject(DC_BRUSH as i32) as HBRUSH);
        }
    }

    pub fn frame(&self, r: R, c: Rgb) {
        unsafe {
            SetDCBrushColor(self.dc, c.c());
            FrameRect(self.dc, &r.rc(), GetStockObject(DC_BRUSH as i32) as HBRUSH);
        }
    }

    pub fn hline(&self, x0: i32, x1: i32, y: i32, c: Rgb) {
        self.fill(R::new(x0, y, x1, y + 1.max(self.px(1.0) / 2)), c)
    }

    pub fn vline(&self, x: i32, y0: i32, y1: i32, c: Rgb) {
        self.fill(R::new(x, y0, x + 1.max(self.px(1.0) / 2), y1), c)
    }

    pub fn round(&self, r: R, radius: i32, fill: Option<Rgb>, border: Option<Rgb>) {
        unsafe {
            let brush: HBRUSH = match fill {
                Some(c) => {
                    SetDCBrushColor(self.dc, c.c());
                    GetStockObject(DC_BRUSH as i32) as HBRUSH
                }
                None => GetStockObject(NULL_BRUSH as i32) as HBRUSH,
            };
            let pen = match border {
                Some(c) => {
                    SetDCPenColor(self.dc, c.c());
                    GetStockObject(DC_PEN as i32)
                }
                None => GetStockObject(NULL_PEN as i32),
            };
            let (ob, op) = (SelectObject(self.dc, brush as HGDIOBJ), SelectObject(self.dc, pen as HGDIOBJ));
            RoundRect(self.dc, r.l, r.t, r.r + 1, r.b + 1, radius * 2, radius * 2);
            SelectObject(self.dc, ob);
            SelectObject(self.dc, op);
        }
    }

    pub fn text_in(&self, r: R, s: &str, f: Font, c: Rgb, flags: u32) {
        if s.is_empty() {
            return;
        }
        with_utf16(s, |w| unsafe {
            let old = SelectObject(self.dc, self.fonts.get(f) as HGDIOBJ);
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, c.c());
            let mut rc = r.rc();
            DrawTextW(self.dc, w.as_ptr(), w.len() as i32, &mut rc, flags | DT_NOPREFIX);
            SelectObject(self.dc, old);
        });
    }

    pub fn text(&self, r: R, s: &str, f: Font, c: Rgb) {
        self.text_in(r, s, f, c, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS)
    }

    pub fn text_right(&self, r: R, s: &str, f: Font, c: Rgb) {
        self.text_in(r, s, f, c, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS)
    }

    pub fn text_center(&self, r: R, s: &str, f: Font, c: Rgb) {
        self.text_in(r, s, f, c, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS)
    }

    pub fn text_w(&self, s: &str, f: Font) -> i32 {
        with_utf16(s, |w| unsafe {
            let old = SelectObject(self.dc, self.fonts.get(f) as HGDIOBJ);
            let mut sz = std::mem::zeroed();
            GetTextExtentPoint32W(self.dc, w.as_ptr(), w.len() as i32, &mut sz);
            SelectObject(self.dc, old);
            sz.cx
        })
    }

    pub fn glyph(&self, r: R, ch: char, c: Rgb, small: bool) {
        let mut b = [0u8; 4];
        self.text_center(r, ch.encode_utf8(&mut b), if small { Font::IconsSm } else { Font::Icons }, c);
    }

    pub fn polyline(&self, pts: &[POINT], c: Rgb, width: i32) {
        if pts.len() < 2 {
            return;
        }
        unsafe {
            if width <= 1 {
                SetDCPenColor(self.dc, c.c());
                let old = SelectObject(self.dc, GetStockObject(DC_PEN as i32));
                Polyline(self.dc, pts.as_ptr(), pts.len() as i32);
                SelectObject(self.dc, old);
            } else {
                let pen = self.fonts.pen(width, c.c());
                let old = SelectObject(self.dc, pen);
                Polyline(self.dc, pts.as_ptr(), pts.len() as i32);
                SelectObject(self.dc, old);
            }
        }
    }

    pub fn polygon(&self, pts: &[POINT], c: Rgb) {
        if pts.len() < 3 {
            return;
        }
        unsafe {
            SetDCBrushColor(self.dc, c.c());
            let (ob, op) = (SelectObject(self.dc, GetStockObject(DC_BRUSH as i32)), SelectObject(self.dc, GetStockObject(NULL_PEN as i32)));
            Polygon(self.dc, pts.as_ptr(), pts.len() as i32);
            SelectObject(self.dc, ob);
            SelectObject(self.dc, op);
        }
    }

    pub fn icon(&self, hicon: isize, x: i32, y: i32, size: i32) {
        if hicon != 0 {
            unsafe { DrawIconEx(self.dc, x, y, hicon as _, size, size, 0, null_mut(), 3 ) };
        }
    }

    pub fn clip(&self, r: R) {
        unsafe {
            SaveDC(self.dc);
            IntersectClipRect(self.dc, r.l, r.t, r.r, r.b);
        }
    }

    pub fn unclip(&self) {
        unsafe { RestoreDC(self.dc, -1) };
    }
}

pub fn heat(th: &Theme, value: f64, scale: f64) -> Rgb {
    th.heat0.lerp(th.heat1, ((value / scale).max(0.0).sqrt()) as f32)
}

pub fn group_thousands(mut n: u64) -> String {
    let mut buf = [0u8; 32];
    let mut idx = buf.len();
    let mut digits = 0u8;
    loop {
        if digits > 0 && digits % 3 == 0 {
            idx -= 1;
            buf[idx] = b',';
        }
        idx -= 1;
        buf[idx] = b'0' + (n % 10) as u8;
        n /= 10;
        digits += 1;
        if n == 0 {
            break;
        }
    }
    unsafe { std::str::from_utf8_unchecked(&buf[idx..]) }.to_string()
}

pub fn fmt_uptime(secs: u64) -> String {
    format!("{}:{:02}:{:02}:{:02}", secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(group_thousands(3572), "3,572");
        assert_eq!(group_thousands(12), "12");
        assert_eq!(fmt_uptime(2 * 86400 + 18 * 3600 + 27 * 60 + 54), "2:18:27:54");
    }

    #[test]
    fn heat_is_monotonic_and_clamped() {
        let t = theme(true);
        assert_eq!(heat(&t, 0.0, 50.0), t.heat0);
        assert_eq!(heat(&t, 500.0, 50.0), t.heat1);
        let (a, b) = (heat(&t, 5.0, 50.0), heat(&t, 25.0, 50.0));
        assert!(b.2 > a.2);
    }

    #[test]
    fn lerp_endpoints() {
        assert_eq!(Rgb(0, 0, 0).lerp(Rgb(100, 200, 50), 0.0), Rgb(0, 0, 0));
        assert_eq!(Rgb(0, 0, 0).lerp(Rgb(100, 200, 50), 1.0), Rgb(100, 200, 50));
    }
}
