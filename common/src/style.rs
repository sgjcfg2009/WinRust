use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, Ordering::Relaxed},
    },
};
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW, RegisterWindowMessageW};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Normal,
    Terminal,
    Dock,
    Tiling,
    Road,
    Ruby,
    Win98,
    Win7,
}

pub const ALL: [Style; 8] = [Style::Normal, Style::Terminal, Style::Dock, Style::Tiling, Style::Road, Style::Ruby, Style::Win98, Style::Win7];

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Style::Normal => "normal",
            Style::Terminal => "terminal",
            Style::Dock => "dock",
            Style::Tiling => "tiling",
            Style::Road => "road",
            Style::Ruby => "ruby",
            Style::Win98 => "win98",
            Style::Win7 => "win7",
        }
    }
    pub fn parse(s: &str) -> Style {
        ALL.into_iter().find(|v| v.name().eq_ignore_ascii_case(s.trim())).unwrap_or(Style::Normal)
    }
    pub fn square(self) -> bool {
        matches!(self, Style::Terminal | Style::Win98)
    }
    pub fn beta(self) -> bool {
        matches!(self, Style::Win98 | Style::Win7)
    }
    pub fn mono(self) -> bool {
        self == Style::Terminal
    }
    pub fn light(self) -> bool {
        matches!(self, Style::Road | Style::Win98)
    }
}

static CUR: AtomicU8 = AtomicU8::new(u8::MAX);
static PAL_PTR: AtomicPtr<Pal> = AtomicPtr::new(std::ptr::null_mut());

fn data_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Some(pd) = std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust")) {
            if std::fs::create_dir_all(&pd).is_ok() {
                let probe = pd.join(".probe");
                if std::fs::write(&probe, b"").is_ok() {
                    let _ = std::fs::remove_file(&probe);
                    return Some(pd);
                }
            }
        }
        std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("ProgramData")).map(|p| PathBuf::from(p).join("WinRust"))
    }).as_ref()
}

fn cfg_file(name: &str) -> Option<PathBuf> {
    data_dir().map(|d| d.join(name))
}

fn write_cfg(name: &str, text: &str) {
    if let Some(f) = cfg_file(name) {
        if let Some(parent) = f.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(f, text);
    }
}

fn broadcast() {
    unsafe { PostMessageW(HWND_BROADCAST, msg(), 0, 0) };
}

fn store(s: Style) {
    CUR.store(ALL.iter().position(|&v| v == s).unwrap_or(0) as u8, Relaxed);
    PAL_PTR.store(std::ptr::null_mut(), Relaxed);
}

fn load_style() -> Style {
    let name = std::env::var("WINRUST_STYLE").ok().or_else(|| cfg_file("style.txt").and_then(|f| std::fs::read_to_string(f).ok()));
    Style::parse(&name.unwrap_or_default())
}

pub fn reload() {
    store(load_style());
    HIDE.store(u8::MAX, Relaxed);
    LEFT.store(u8::MAX, Relaxed);
    SIDE.store(u8::MAX, Relaxed);
    OPACITY.store(u8::MAX, Relaxed);
    START_OPACITY.store(u8::MAX, Relaxed);
    SIZE.store(u8::MAX, Relaxed);
    START_SIZE.store(u8::MAX, Relaxed);
    MON.store(u8::MAX, Relaxed);
    COLORS_LOADED.store(false, Relaxed);
    PAL_PTR.store(std::ptr::null_mut(), Relaxed);
}

pub fn style() -> Style {
    let idx = CUR.load(Relaxed);
    if idx == u8::MAX {
        store(load_style());
        ALL[CUR.load(Relaxed) as usize % ALL.len()]
    } else {
        ALL[idx as usize % ALL.len()]
    }
}

pub fn set_style(s: Style) {
    store(s);
    write_cfg("style.txt", s.name());
    broadcast();
}

fn load_bool(slot: &AtomicU8, file: &str) -> bool {
    let v = slot.load(Relaxed);
    if v == u8::MAX {
        let on = cfg_file(file).and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|s| s.trim() == "1") as u8;
        slot.store(on, Relaxed);
        on == 1
    } else {
        v == 1
    }
}

fn store_bool(slot: &AtomicU8, file: &str, on: bool) {
    slot.store(on as u8, Relaxed);
    write_cfg(file, if on { "1" } else { "0" });
    broadcast();
}

fn load_u8(slot: &AtomicU8, file: &str, default: u8, min: u8, max: u8) -> u8 {
    let v = slot.load(Relaxed);
    if v == u8::MAX {
        let loaded = cfg_file(file)
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| s.trim().parse::<u8>().ok())
            .map_or(default, |v| v.clamp(min, max));
        slot.store(loaded, Relaxed);
        loaded
    } else {
        v
    }
}

fn store_u8(slot: &AtomicU8, file: &str, v: u8, min: u8, max: u8) {
    let v = v.clamp(min, max);
    slot.store(v, Relaxed);
    write_cfg(file, &v.to_string());
    broadcast();
}

pub fn autohide() -> bool {
    load_bool(&HIDE, "autohide.txt")
}

static HIDE: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn set_autohide(on: bool) {
    store_bool(&HIDE, "autohide.txt", on);
}

static LEFT: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn align_left() -> bool {
    load_bool(&LEFT, "align-left.txt")
}

pub fn set_align_left(on: bool) {
    store_bool(&LEFT, "align-left.txt", on);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Bottom,
    Left,
    Right,
}

impl Side {
    pub fn parse(s: &str) -> Side {
        let t = s.trim();
        if t.eq_ignore_ascii_case("left") {
            Side::Left
        } else if t.eq_ignore_ascii_case("right") {
            Side::Right
        } else {
            Side::Bottom
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Side::Bottom => "bottom",
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

static SIDE: AtomicU8 = AtomicU8::new(u8::MAX);

fn side_code(s: Side) -> u8 {
    match s {
        Side::Bottom => 0,
        Side::Left => 1,
        Side::Right => 2,
    }
}

pub fn side() -> Side {
    let mut v = SIDE.load(Relaxed);
    if v == u8::MAX {
        let s = cfg_file("side.txt").and_then(|f| std::fs::read_to_string(f).ok()).map_or(Side::Bottom, |s| Side::parse(&s));
        v = side_code(s);
        SIDE.store(v, Relaxed);
    }
    match v {
        1 => Side::Left,
        2 => Side::Right,
        _ => Side::Bottom,
    }
}

pub fn set_side(s: Side) {
    SIDE.store(side_code(s), Relaxed);
    write_cfg("side.txt", s.name());
    broadcast();
}

pub fn msg() -> u32 {
    static M: AtomicU32 = AtomicU32::new(0);
    match M.load(Relaxed) {
        0 => {
            const NAME: [u16; 20] = [
                b'W' as u16, b'i' as u16, b'n' as u16, b'R' as u16, b'u' as u16, b's' as u16, b't' as u16,
                b'S' as u16, b't' as u16, b'y' as u16, b'l' as u16, b'e' as u16,
                b'C' as u16, b'h' as u16, b'a' as u16, b'n' as u16, b'g' as u16, b'e' as u16, b'd' as u16, 0,
            ];
            let m = unsafe { RegisterWindowMessageW(NAME.as_ptr()) };
            M.store(m, Relaxed);
            m
        }
        m => m,
    }
}

#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: u32,
    pub line: u32,
    pub hover: u32,
    pub active: u32,
    pub accent: u32,
    pub text: u32,
    pub dim: u32,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

pub const PALS: [Pal; 8] = [
    Pal { bg: rgb(0, 0, 0), line: rgb(32, 32, 34), hover: rgb(20, 20, 22), active: rgb(28, 28, 30), accent: rgb(96, 205, 255), text: rgb(255, 255, 255), dim: rgb(166, 166, 170) },
    Pal { bg: rgb(8, 11, 9), line: rgb(30, 48, 35), hover: rgb(16, 28, 20), active: rgb(22, 38, 27), accent: rgb(74, 222, 128), text: rgb(204, 232, 212), dim: rgb(98, 132, 108) },
    Pal { bg: rgb(17, 18, 25), line: rgb(40, 42, 56), hover: rgb(29, 31, 42), active: rgb(37, 40, 54), accent: rgb(124, 134, 255), text: rgb(240, 241, 246), dim: rgb(138, 142, 160) },
    Pal { bg: rgb(16, 17, 20), line: rgb(38, 40, 46), hover: rgb(27, 29, 34), active: rgb(35, 37, 43), accent: rgb(245, 182, 82), text: rgb(224, 226, 231), dim: rgb(120, 125, 138) },
    Pal { bg: rgb(247, 245, 240), line: rgb(226, 220, 207), hover: rgb(238, 233, 222), active: rgb(232, 225, 210), accent: rgb(176, 134, 52), text: rgb(28, 26, 22), dim: rgb(124, 117, 104) },
    Pal { bg: rgb(26, 7, 13), line: rgb(72, 22, 36), hover: rgb(46, 13, 24), active: rgb(60, 17, 31), accent: rgb(232, 52, 84), text: rgb(252, 238, 241), dim: rgb(178, 126, 139) },
    Pal { bg: rgb(192, 192, 192), line: rgb(128, 128, 128), hover: rgb(212, 208, 200), active: rgb(160, 160, 160), accent: rgb(0, 0, 128), text: rgb(0, 0, 0), dim: rgb(64, 64, 64) },
    Pal { bg: rgb(31, 58, 96), line: rgb(72, 108, 154), hover: rgb(58, 92, 138), active: rgb(74, 112, 160), accent: rgb(120, 190, 255), text: rgb(255, 255, 255), dim: rgb(176, 198, 226) },
];

pub const GOOD: u32 = rgb(48, 209, 88);
pub const WARN: u32 = rgb(255, 214, 10);
pub const BAD: u32 = rgb(255, 69, 58);

pub fn pal() -> &'static Pal {
    let p = PAL_PTR.load(Relaxed);
    if !p.is_null() {
        return unsafe { &*p };
    }
    let base = &PALS[ALL.iter().position(|&s| s == style()).unwrap_or(0)];
    let resolved: &'static Pal = match custom_colors() {
        None => base,
        Some(o) => {
            static CACHE: Mutex<Vec<(usize, [u32; 7], &'static Pal)>> = Mutex::new(Vec::new());
            let key = base as *const Pal as usize;
            let mut v = CACHE.lock().unwrap();
            if let Some(e) = v.iter().find(|e| e.0 == key && e.1 == o) {
                e.2
            } else {
                let leaked: &'static Pal = Box::leak(Box::new(custom_pal(base, o)));
                v.push((key, o, leaked));
                leaked
            }
        }
    };
    PAL_PTR.store(resolved as *const Pal as *mut Pal, Relaxed);
    resolved
}

fn custom_pal(base: &Pal, o: [u32; 7]) -> Pal {
    let get = |i: usize, d: u32| if o[i] == u32::MAX { d } else { o[i] };
    let bg = get(0, base.bg);
    let text = get(5, base.text);
    let derive = o[0] != u32::MAX || o[5] != u32::MAX;
    Pal {
        bg,
        line: get(1, if derive { mix(bg, text, 0.12) } else { base.line }),
        hover: get(2, if derive { mix(bg, text, 0.07) } else { base.hover }),
        active: get(3, if derive { mix(bg, text, 0.12) } else { base.active }),
        accent: get(4, base.accent),
        text,
        dim: get(6, if derive { mix(bg, text, 0.55) } else { base.dim }),
    }
}

pub const COLOR_NAMES: [&str; 7] = ["bg", "line", "hover", "active", "accent", "text", "dim"];

static COLORS: Mutex<[u32; 7]> = Mutex::new([u32::MAX; 7]);
static COLORS_LOADED: AtomicBool = AtomicBool::new(false);

fn read_colors() -> [u32; 7] {
    parse_colors(&cfg_file("colors.txt").and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default())
}

fn parse_colors(text: &str) -> [u32; 7] {
    let mut o = [u32::MAX; 7];
    for l in text.split(['\n', ';']) {
        if let Some((k, v)) = l.split_once('=') {
            if let (Some(i), Ok(v)) = (COLOR_NAMES.iter().position(|n| *n == k.trim()), u32::from_str_radix(v.trim(), 16)) {
                if v <= 0xFFFFFF { o[i] = rgb((v >> 16) as u8, (v >> 8) as u8, v as u8); }
            }
        }
    }
    o
}

fn colors_text(c: &[u32; 7], sep: &str) -> String {
    c.iter().enumerate().filter(|(_, v)| **v != u32::MAX).map(|(i, v)| format!("{}={:02X}{:02X}{:02X}{sep}", COLOR_NAMES[i], v & 255, (v >> 8) & 255, (v >> 16) & 255)).collect()
}

pub fn custom_colors() -> Option<[u32; 7]> {
    let mut c = COLORS.lock().unwrap();
    if !COLORS_LOADED.load(Relaxed) {
        *c = read_colors();
        COLORS_LOADED.store(true, Relaxed);
    }
    c.iter().any(|v| *v != u32::MAX).then_some(*c)
}

pub fn set_custom_color(i: usize, c: Option<u32>) {
    let mut cur = read_colors();
    cur[i] = c.unwrap_or(u32::MAX);
    save_colors(cur);
}

pub fn reset_custom() {
    save_colors([u32::MAX; 7]);
    set_bar_opacity(100);
}

fn save_colors(c: [u32; 7]) {
    let text = colors_text(&c, "\n");
    if let Some(f) = cfg_file("colors.txt") {
        if let Some(parent) = f.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = if text.is_empty() { std::fs::remove_file(f) } else { std::fs::write(f, text) };
    }
    *COLORS.lock().unwrap() = c;
    COLORS_LOADED.store(true, Relaxed);
    PAL_PTR.store(std::ptr::null_mut(), Relaxed);
    broadcast();
}

static OPACITY: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn bar_opacity() -> u8 {
    load_u8(&OPACITY, "bar-opacity.txt", 100, 0, 100)
}

pub fn set_bar_opacity(v: u8) {
    store_u8(&OPACITY, "bar-opacity.txt", v, 0, 100);
}

static START_OPACITY: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn start_opacity() -> u8 {
    load_u8(&START_OPACITY, "start-opacity.txt", 100, 15, 100)
}

pub fn set_start_opacity(v: u8) {
    store_u8(&START_OPACITY, "start-opacity.txt", v, 15, 100);
}

#[derive(Clone, PartialEq, Debug)]
pub struct Theme {
    pub name: String,
    pub style: Style,
    pub colors: [u32; 7],
    pub opacity: u8,
}

impl Theme {
    fn line(&self) -> String {
        format!("{}\t{}\t{}\t{}", self.name.replace(['\t', '\r', '\n'], " "), self.style.name(), self.opacity, colors_text(&self.colors, ";"))
    }
    fn parse(l: &str) -> Option<Theme> {
        let mut f = l.trim_end_matches('\r').splitn(4, '\t');
        let name = f.next().filter(|n| !n.trim().is_empty())?.to_string();
        let style = Style::parse(f.next()?);
        let opacity = f.next()?.trim().parse::<u8>().ok()?.clamp(0, 100);
        Some(Theme { name, style, colors: parse_colors(f.next().unwrap_or("")), opacity })
    }
    pub fn current(name: &str) -> Theme {
        Theme { name: name.into(), style: style(), colors: read_colors(), opacity: bar_opacity() }
    }
    pub fn apply(&self) {
        set_bar_opacity(self.opacity);
        save_colors(self.colors);
        set_style(self.style);
    }
}

fn themes_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("themes.txt"))
}

pub fn themes() -> Vec<Theme> {
    themes_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default().lines().filter_map(Theme::parse).collect()
}

pub fn save_themes(t: &[Theme]) {
    if let Some(f) = themes_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let mut out = String::with_capacity(t.len() * 48);
        for item in t {
            out.push_str(&item.line());
            out.push('\n');
        }
        let _ = std::fs::write(f, out);
    }
}

pub fn reset_all() {
    set_side(Side::Bottom);
    set_bar_size(1);
    set_start_size(1);
    set_start_opacity(100);
    set_autohide(false);
    set_align_left(false);
    set_alttab_per_monitor(false);
    Theme { name: String::new(), style: Style::Normal, colors: [u32::MAX; 7], opacity: 100 }.apply();
}

pub fn card() -> u32 {
    if style().light() { rgb(255, 255, 255) } else { mix(pal().bg, pal().text, 0.06) }
}

pub fn on_accent() -> u32 {
    if style().light() || style() == Style::Ruby { rgb(255, 255, 255) } else { pal().bg }
}

pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        (x + (y - x) * t.clamp(0.0, 1.0)).round() as u32
    };
    ch(0) | ch(8) << 8 | ch(16) << 16
}

pub unsafe fn backdrop(dc: windows_sys::Win32::Graphics::Gdi::HDC, r: windows_sys::Win32::Foundation::RECT, scale: f32) {
    use windows_sys::Win32::Graphics::GdiPlus::*;
    let ruby = style() == Style::Ruby;
    if !style().light() && !ruby {
        return;
    }
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    if unsafe { GdipCreateFromHDC(dc, &mut g) } != 0 || g.is_null() {
        return;
    }
    unsafe { GdipSetPixelOffsetMode(g, PixelOffsetModeHalf) };
    let argb = |c: u32, a: f32| ((a.clamp(0.0, 1.0) * 255.0) as u32) << 24 | (c & 255) << 16 | (c & 0xFF00) | (c >> 16 & 255);
    if ruby {
        let (x, y, w, h) = (r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1));
        unsafe {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            let mut br: *mut GpLineGradient = std::ptr::null_mut();
            GdipCreateLineBrushI(&Point { X: x, Y: y }, &Point { X: x, Y: y + h }, argb(mix(pal().bg, rgb(150, 18, 42), 0.62), 1.0), argb(mix(pal().bg, rgb(0, 0, 0), 0.25), 1.0), WrapModeTileFlipXY, &mut br);
            GdipFillRectangleI(g, br as *mut GpBrush, x, y, w, h);
            GdipDeleteBrush(br as *mut GpBrush);
            let gh = (h / 3).max(1);
            GdipCreateLineBrushI(&Point { X: x, Y: y }, &Point { X: x, Y: y + gh }, argb(rgb(255, 255, 255), 0.09), argb(rgb(255, 255, 255), 0.0), WrapModeTileFlipXY, &mut br);
            GdipFillRectangleI(g, br as *mut GpBrush, x, y, w, gh);
            GdipDeleteBrush(br as *mut GpBrush);
            GdipDeleteGraphics(g);
        }
        return;
    }
    let (x0, y0, w, h) = (r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32);
    let s = 58.0 * scale.max(0.5);
    let (cols, rows) = ((w / s) as i32 + 2, (h / (s * 0.866)) as i32 + 2);
    let pt = |i: i32, j: i32| {
        let (hx, hy) = (hash(i, j, 1) - 0.5, hash(i, j, 2) - 0.5);
        let off = if j & 1 == 1 { s * 0.5 } else { 0.0 };
        PointF { X: x0 + i as f32 * s - off + hx * s * 0.34, Y: y0 + j as f32 * s * 0.866 + hy * s * 0.3 - s * 0.4 }
    };
    let gold = pal().accent;
    unsafe {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        let mut br: *mut GpSolidFill = std::ptr::null_mut();
        GdipCreateSolidFill(0, &mut br);
        let mut pen: *mut GpPen = std::ptr::null_mut();
        GdipCreatePen1(0, 1.0, UnitPixel, &mut pen);
        for j in 0..rows {
            for i in 0..cols {
                let (a, b, c, d) = if j & 1 == 0 { (pt(i, j), pt(i + 1, j), pt(i, j + 1), pt(i + 1, j + 1)) } else { (pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i + 2, j + 1)) };
                for (k, tri) in [[a, b, c], [b, d, c]].iter().enumerate() {
                    let v = hash(i, j, 3 + k as u32);
                    let shade = if v < 0.5 { mix(pal().bg, rgb(255, 255, 255), 0.25 + v) } else { mix(pal().bg, rgb(226, 214, 190), (v - 0.5) * 0.45) };
                    GdipSetSolidFillColor(br, argb(shade, 1.0));
                    GdipFillPolygon(g, br as *mut GpBrush, tri.as_ptr(), 3, FillModeAlternate);
                    let (edge, wd) = if hash(i, j, 7 + k as u32) > 0.84 { (argb(gold, 0.6), 1.0 * scale) } else { (argb(rgb(255, 255, 255), 0.7), 0.6) };
                    GdipSetPenColor(pen, edge);
                    GdipSetPenWidth(pen, wd);
                    GdipDrawPolygon(g, pen, tri.as_ptr(), 3);
                }
            }
        }
        GdipDeletePen(pen);
        GdipDeleteBrush(br as *mut GpBrush);
        GdipDeleteGraphics(g);
    }
}

fn hash(i: i32, j: i32, salt: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x9E37_79B1) ^ (j as u32).wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

fn font_file(name: &str) -> bool {
    std::env::var_os("WINDIR").is_some_and(|w| PathBuf::from(w).join("Fonts").join(name).is_file())
}

pub fn sans_face() -> &'static str {
    static F: std::sync::OnceLock<&str> = std::sync::OnceLock::new();
    F.get_or_init(|| if font_file("SegUIVar.ttf") { "Segoe UI Variable Text" } else { "Segoe UI" })
}

pub fn ui_face() -> &'static str {
    match style() {
        s if s.mono() => "Consolas",
        Style::Win98 => "Tahoma",
        _ => sans_face(),
    }
}

pub fn icon_face() -> &'static str {
    static F: std::sync::OnceLock<&str> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        if crate::ui::ensure_fluent_icons() || font_file("SegoeIcons.ttf") {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_junk_is_normal() {
        for s in ALL {
            assert_eq!(Style::parse(s.name()), s);
        }
        assert_eq!(Style::parse(" Tiling\r\n"), Style::Tiling);
        assert_eq!(Style::parse(""), Style::Normal);
        assert_eq!(Style::parse("neon"), Style::Normal);
        assert_eq!(PALS.len(), ALL.len());
    }

    #[test]
    fn hash_is_stable_and_in_range() {
        assert_eq!(hash(3, 4, 1), hash(3, 4, 1));
        assert!((0..50).all(|i| (0.0..=1.0).contains(&hash(i, -i, 2))));
        assert_eq!(mix(rgb(0, 0, 0), rgb(200, 100, 50), 0.5), rgb(100, 50, 25));
    }

    #[test]
    fn theme_line_round_trips() {
        let mut c = [u32::MAX; 7];
        c[0] = rgb(1, 2, 3);
        c[4] = rgb(250, 128, 0);
        let t = Theme { name: "Ночь\tтест".into(), style: Style::Ruby, colors: c, opacity: 70 };
        let back = Theme::parse(&t.line()).unwrap();
        assert_eq!(back.name, "Ночь тест");
        assert_eq!((back.style, back.colors, back.opacity), (Style::Ruby, c, 70));
        assert_eq!(Theme::parse("a\tnormal\t5\t").unwrap().opacity, 5);
        assert!(Theme::parse("").is_none() && Theme::parse("x\tnormal").is_none());
        let mut one = [u32::MAX; 7];
        one[0] = rgb(1, 2, 3);
        assert_eq!(parse_colors("bg=010203\nzz=1;text=GG"), one);
    }

    #[test]
    fn side_parses_and_defaults_to_bottom() {
        assert_eq!(Side::parse(" Left
"), Side::Left);
        assert_eq!(Side::parse("right"), Side::Right);
        assert_eq!(Side::parse(""), Side::Bottom);
        assert_eq!(Side::parse("top"), Side::Bottom);
    }
}

static SIZE: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn bar_size() -> u8 {
    load_u8(&SIZE, "bar-size.txt", 1, 0, 2)
}

pub fn set_bar_size(v: u8) {
    store_u8(&SIZE, "bar-size.txt", v, 0, 2);
}

static START_SIZE: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn start_size() -> u8 {
    load_u8(&START_SIZE, "start-size.txt", 1, 0, 2)
}

pub fn set_start_size(v: u8) {
    store_u8(&START_SIZE, "start-size.txt", v, 0, 2);
}

static MON: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn alttab_per_monitor() -> bool {
    load_bool(&MON, "alttab-monitor.txt")
}

pub fn set_alttab_per_monitor(on: bool) {
    store_bool(&MON, "alttab-monitor.txt", on);
}
