use crate::{data::*, draw::*, model::*, startup, store, sys, ui::*};
use native_windows_gui as nwg;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use winapi::shared::windef::{HMENU, HWND};
use winapi::um::winuser::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    RunTask,
    EndTask,
    Efficiency,
    More,
    StartEnable,
    StartDisable,
    StartDelete,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    None,
    NavToggle,
    Nav(usize),
    Button(Btn),
    Header(usize),
    Row(usize),
    Chevron(usize),
    VBar,
    PerfSplit,
    PerfItem(usize),
    Setting(usize),
}

#[derive(Clone)]
pub struct Lay {
    pub client: R,
    pub nav: R,
    pub nav_toggle: R,
    pub nav_items: Vec<R>,
    pub band: R,
    pub title: R,
    pub search: R,
    pub buttons: Vec<(R, Btn, &'static str, char)>,
    pub body: R,
    pub header: R,
    pub rows_area: R,
    pub col_x: Vec<i32>,
    pub vbar: R,
    pub perf_list: R,
    pub perf_items: Vec<R>,
    pub perf_main: R,
    pub setting_cards: Vec<(R, SettingId)>,
}

pub struct Ui {
    pub page: Page,
    pub nav_open: bool,
    pub th: Theme,
    pub rows: Vec<Row>,
    pub sel: Option<String>,
    pub expanded: HashSet<String>,
    pub sorts: HashMap<Page, (usize, bool)>,
    pub search: String,
    pub hit: Hit,
    pub mouse: (i32, i32),
    pub scroll: f64,
    pub drag: Option<(i32, f64)>,
    pub perf_split: Option<f64>,
    pub perf_drag: bool,
    pub perf_sel: PerfKind,
    pub toast: Option<(String, Instant)>,
    pub lay: Option<Lay>,
    pub last_click: Option<(Instant, String)>,
}

impl Ui {
    pub fn sort_of(&self, p: Page) -> (usize, bool) {
        self.sorts.get(&p).copied().unwrap_or_else(|| default_sort(p))
    }
    pub fn page_has_search(&self) -> bool {
        matches!(self.page, Page::Processes | Page::Users | Page::Details | Page::Startup)
    }
    pub fn has_table(&self) -> bool {
        self.page_has_search()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    RunTask,
    EndTask,
    EndTree,
    Efficiency,
    Suspend(bool),
    Prio(u32),
    Affinity(bool),
    OpenDir,
    CopyCmd,
    StartEnable,
    StartDisable,
    StartDelete,
    Export,
    SetTheme(u8),
    ToggleTray,
    ToggleAutostart,
    ToggleEtw,
    ToggleGpuTemp,
    ShowWindow,
    RelaunchAdmin,
    Exit,
    Toggle,
    Go(Page),
}

pub enum MI {
    Item(&'static str, Act),
    Check(&'static str, Act, bool),
    Sep,
    Sub(&'static str, Vec<MI>),
}

pub struct App {
    pub win: nwg::Window,
    pub search: nwg::TextInput,
    #[allow(deprecated)]
    pub _timer: nwg::Timer,
    pub tray: nwg::TrayNotification,
    pub _tray_icon: nwg::Icon,
    pub _icon_big: nwg::Icon,
    pub data: RefCell<Data>,
    pub ui: RefCell<Ui>,
    pub fonts: RefCell<Fonts>,
    pub scale: Cell<f64>,
    pub edit_brush: Cell<isize>,
    pub mem_dc: Cell<isize>,
    pub mem_bmp: Cell<(isize, i32, i32)>,
    pub mem_old_bmp: Cell<isize>,
    pub search_rect: Cell<R>,
    pub last_tick: Cell<Instant>,
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            let brush = self.edit_brush.get();
            if brush != 0 {
                winapi::um::wingdi::DeleteObject(brush as _);
            }
            let dc = self.mem_dc.get();
            if dc != 0 {
                let old = self.mem_old_bmp.get();
                if old != 0 {
                    winapi::um::wingdi::SelectObject(dc as _, old as _);
                }
                let (bmp, _, _) = self.mem_bmp.get();
                if bmp != 0 {
                    winapi::um::wingdi::DeleteObject(bmp as _);
                }
                winapi::um::wingdi::DeleteDC(dc as _);
            }
        }
    }
}

fn page_buttons(p: Page) -> &'static [(Btn, &'static str, char)] {
    const PROCS: [(Btn, &str, char); 4] = [(Btn::RunTask, "Run new task", '\u{E710}'), (Btn::EndTask, "End task", '\u{E711}'), (Btn::Efficiency, "Efficiency mode", '\u{E945}'), (Btn::More, "", '\u{E712}')];
    const PERF: [(Btn, &str, char); 2] = [(Btn::RunTask, "Run new task", '\u{E710}'), (Btn::More, "", '\u{E712}')];
    const USERS: [(Btn, &str, char); 2] = [(Btn::EndTask, "End task", '\u{E711}'), (Btn::More, "", '\u{E712}')];
    const DETAILS: [(Btn, &str, char); 3] = [(Btn::RunTask, "Run new task", '\u{E710}'), (Btn::EndTask, "End task", '\u{E711}'), (Btn::More, "", '\u{E712}')];
    const STARTUP: [(Btn, &str, char); 4] = [(Btn::StartEnable, "Enable", '\u{E73E}'), (Btn::StartDisable, "Disable", '\u{E711}'), (Btn::StartDelete, "Delete", '\u{E74D}'), (Btn::More, "", '\u{E712}')];
    match p {
        Page::Processes => &PROCS,
        Page::Performance => &PERF,
        Page::Users => &USERS,
        Page::Details => &DETAILS,
        Page::Startup => &STARTUP,
        _ => &[],
    }
}

impl App {
    pub fn compute_layout(&self, g: &G, ui: &Ui, d: &Data, w: i32, h: i32) -> Lay {
        let s = g.s;
        let px = |v: f64| (v * s).round() as i32;
        let nav_w = px(if ui.nav_open { NAV_W_OPEN.min((w as f64 / s) * 0.28) } else { NAV_W });
        let nav = R::new(0, 0, nav_w, h);
        let nav_toggle = R::new(0, px(6.0), px(NAV_W), px(6.0 + 44.0));
        let nav_count = NAV.len();
        let nav_step = ((h - px(64.0)) as f64 / nav_count.max(1) as f64).min(42.0 * s).max(28.0 * s);
        let item_h = nav_step.round() as i32;
        let nav_items: Vec<R> = (0..nav_count)
            .map(|i| {
                let y0 = if i + 1 == nav_count { (h - item_h - px(8.0)).max(px(54.0) + i as i32 * item_h) } else { px(54.0) + i as i32 * item_h };
                R::new(0, y0, nav_w, y0 + item_h)
            })
            .collect();
        let band = R::new(nav_w, 0, w, px(BAND_H));
        let title_w = (g.text_w(page_title(ui.page), Font::Title) + px(24.0)).min((band.w() - px(120.0)).max(px(120.0)));
        let title = R::new(band.l + px(16.0), 0, band.l + title_w, band.b);
        let compact_btns = band.w() < px(860.0);
        let mut buttons = vec![];
        let mut right = band.r - px(12.0);
        for &(btn, label, glyph) in page_buttons(ui.page).iter().rev() {
            let lbl = if compact_btns { "" } else { label };
            let bw = if lbl.is_empty() { px(36.0) } else { g.text_w(lbl, Font::Ui) + px(44.0) };
            let r = R::new(right - bw, px(10.0), right, px(10.0 + 32.0));
            buttons.push((r, btn, lbl, glyph));
            right -= bw + px(4.0);
        }
        buttons.reverse();
        let btn_left = buttons.first().map_or(band.r, |b| b.0.l);
        let avail_s = (btn_left - title.r - px(16.0)).max(0);
        let search = if avail_s >= px(90.0) {
            let sw = px(380.0).min(avail_s);
            let sx = title.r + ((avail_s - sw) / 2).max(px(8.0));
            R::new(sx, px(10.0), (sx + sw).min(btn_left - px(8.0)), px(10.0 + 32.0))
        } else {
            R::new(0, 0, 0, 0)
        };
        let body = R::new(nav_w, band.b, w, h);
        let mut lay = Lay {
            client: R::new(0, 0, w, h),
            nav,
            nav_toggle,
            nav_items,
            band,
            title,
            search,
            buttons,
            body,
            header: R::new(0, 0, 0, 0),
            rows_area: R::new(0, 0, 0, 0),
            col_x: vec![],
            vbar: R::new(0, 0, 0, 0),
            perf_list: R::new(0, 0, 0, 0),
            perf_items: vec![],
            perf_main: R::new(0, 0, 0, 0),
            setting_cards: vec![],
        };
        if ui.has_table() {
            let cols = columns(ui.page);
            let hh = px(if ui.page == Page::Processes { 56.0 } else { 36.0 });
            lay.header = R::new(body.l, body.t, body.r, body.t + hh);
            lay.rows_area = R::new(body.l, body.t + hh, body.r, body.b);
            let total_w = body.w().max(1) as f64;
            let fixed_desired: f64 = cols.iter().map(|c| c.w * s).sum();
            let flex_count = cols.iter().filter(|c| c.w == 0.0).count();
            let min_flex_each = (px(180.0) as f64).min(total_w * 0.30);
            let mut x = body.l;
            lay.col_x.push(x);
            if flex_count > 0 && fixed_desired + min_flex_each * flex_count as f64 <= total_w {
                let flex_w = (total_w - fixed_desired) / flex_count as f64;
                for (ci, c) in cols.iter().enumerate() {
                    if ci + 1 == cols.len() {
                        x = body.r;
                    } else {
                        x += (if c.w == 0.0 { flex_w } else { c.w * s }).round() as i32;
                    }
                    lay.col_x.push(x.min(body.r));
                }
            } else {
                let weights: Vec<f64> = cols.iter().map(|c| if c.w == 0.0 { 220.0 } else { c.w }).collect();
                let wsum: f64 = weights.iter().sum::<f64>().max(1.0);
                let mut acc = 0.0;
                for (ci, &wt) in weights.iter().enumerate() {
                    acc += wt;
                    x = if ci + 1 == cols.len() { body.r } else { body.l + (total_w * acc / wsum).round() as i32 };
                    lay.col_x.push(x.min(body.r));
                }
            }
            lay.vbar = lay.rows_area;
        }
        match ui.page {
            Page::Performance => {
                let min_w = px(120.0);
                let max_w = (body.w() - px(240.0)).max(min_w);
                let auto_w = (body.w() * 24 / 100).clamp(px(136.0), px(256.0)).min((body.w() / 3).max(px(120.0)));
                let list_w = ui.perf_split.map_or(auto_w, |v| (v * s).round() as i32).clamp(min_w, max_w);
                lay.perf_list = R::new(body.l, body.t, body.l + list_w, body.b);
                let n = (2 + d.disks.len() + d.nics.len() + d.gpus.len()).max(1) as i32;
                let item_h = px(76.0).min((body.h() - px(16.0)) / n);
                lay.perf_items = (0..n).map(|i| R::new(lay.perf_list.l + px(6.0), body.t + px(8.0) + i * item_h, lay.perf_list.r - px(6.0), body.t + px(8.0) + (i + 1) * item_h)).collect();
                lay.perf_main = R::new(lay.perf_list.r + px(16.0), body.t + px(10.0), body.r - px(16.0), body.b - px(10.0));
            }
            Page::Settings => {
                let ids = [SettingId::Theme, SettingId::Tray, SettingId::Autostart, SettingId::Etw, SettingId::GpuTemp];
                let cw = (body.w() - px(48.0)).min(px(900.0));
                lay.setting_cards = ids.iter().enumerate().map(|(i, id)| (R::new(body.l + px(24.0), body.t + px(16.0) + i as i32 * px(76.0), body.l + px(24.0) + cw, body.t + px(16.0) + i as i32 * px(76.0) + px(66.0)), *id)).collect();
            }
            _ => {}
        }
        lay
    }

    fn content_height(&self, g: &G, ui: &Ui, _lay: &Lay, _d: &Data) -> f64 {
        ui.rows.len() as f64 * g.px(ROW_H) as f64
    }

    pub fn paint(&self, hwnd: HWND) {
        let (Ok(d), Ok(mut ui)) = (self.data.try_borrow(), self.ui.try_borrow_mut()) else {
            unsafe { InvalidateRect(hwnd, null(), 0) };
            return;
        };
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let (w, h) = (rc.right, rc.bottom);
            if w > 0 && h > 0 {
                let mut mem_i = self.mem_dc.get();
                if mem_i == 0 {
                    mem_i = winapi::um::wingdi::CreateCompatibleDC(hdc) as isize;
                    self.mem_dc.set(mem_i);
                }
                let mem = mem_i as winapi::shared::windef::HDC;
                let (mut bmp_i, bw, bh) = self.mem_bmp.get();
                if bmp_i == 0 || bw != w || bh != h {
                    let nb = winapi::um::wingdi::CreateCompatibleBitmap(hdc, w, h);
                    let prev = winapi::um::wingdi::SelectObject(mem, nb as *mut _);
                    if bmp_i != 0 {
                        winapi::um::wingdi::DeleteObject(prev);
                    } else {
                        self.mem_old_bmp.set(prev as isize);
                    }
                    bmp_i = nb as isize;
                    self.mem_bmp.set((bmp_i, w, h));
                }
                let fonts = self.fonts.borrow();
                let g = G { dc: mem, s: self.scale.get(), th: ui.th, fonts: &fonts };
                let lay = self.compute_layout(&g, &ui, &d, w, h);
                let area_h = if ui.has_table() { lay.rows_area.h() } else { lay.body.h() } as f64;
                let max_scroll = (self.content_height(&g, &ui, &lay, &d) - area_h).max(0.0);
                ui.scroll = ui.scroll.clamp(0.0, max_scroll);
                g.fill(lay.client, ui.th.bg);
                match ui.page {
                    Page::Performance => self.draw_performance(&g, &ui, &d, &lay),
                    Page::Settings => self.draw_settings(&g, &ui, &d, &lay),
                    _ => self.draw_table(&g, &ui, &d, &lay),
                }
                self.draw_band(&g, &ui, &d, &lay);
                self.draw_nav(&g, &ui, &lay);
                self.draw_overlay(&g, &ui, lay.client);
                winapi::um::wingdi::BitBlt(hdc, 0, 0, w, h, mem, 0, 0, winapi::um::wingdi::SRCCOPY);
                ui.lay = Some(lay);
            }
            EndPaint(hwnd, &ps);
        }
        drop(ui);
        self.place_search();
    }

    pub fn place_search(&self) {
        let ui = self.ui.borrow();
        let Some(lay) = &ui.lay else { return };
        let s = self.scale.get();
        let show = ui.page_has_search() && lay.search.w() >= (60.0 * s) as i32;
        let r = lay.search;
        if self.search_rect.get() != r || self.search.visible() != show {
            self.search_rect.set(r);
            let px = |v: f64| (v * s).round() as i32;
            if let Some(h) = self.search.handle.hwnd() {
                let eh = px(20.0).max(16);
                unsafe {
                    SetWindowPos(
                        h,
                        null_mut(),
                        r.l + px(10.0),
                        r.t + (r.h() - eh) / 2,
                        (r.w() - px(44.0)).max(px(40.0)),
                        eh,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            self.search.set_visible(show);
        }
    }

    pub fn on_dpi_changed(&self, hwnd: HWND, dpi: u32, rc: &winapi::shared::windef::RECT) {
        let s = (dpi as f64 / 96.0).max(0.5);
        self.scale.set(s);
        self.fonts.borrow_mut().rebuild(s);
        if let Some(sh) = self.search.handle.hwnd() {
            unsafe { SendMessageW(sh, WM_SETFONT, self.fonts.borrow().get(Font::Ui) as usize, 1) };
        }
        unsafe {
            SetWindowPos(hwnd, null_mut(), rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.search_rect.set(R::new(0, 0, 0, 0));
        self.invalidate();
    }

    pub fn invalidate(&self) {
        if let Some(h) = self.win.handle.hwnd() {
            unsafe { InvalidateRect(h, null(), 0) };
        }
    }

    fn hit_test(&self, ui: &Ui, x: i32, y: i32) -> Hit {
        let Some(lay) = &ui.lay else { return Hit::None };
        if lay.nav_toggle.contains(x, y) {
            return Hit::NavToggle;
        }
        if let Some(i) = lay.nav_items.iter().position(|r| r.contains(x, y)) {
            return Hit::Nav(i);
        }
        if lay.band.contains(x, y) {
            return lay.buttons.iter().find(|b| b.0.contains(x, y)).map_or(Hit::None, |b| Hit::Button(b.1));
        }
        let s = self.scale.get();
        match ui.page {
            Page::Performance => {
                if (x - lay.perf_list.r).abs() <= (5.0 * s) as i32 && y >= lay.perf_list.t && y <= lay.perf_list.b {
                    return Hit::PerfSplit;
                }
                lay.perf_items.iter().position(|r| r.contains(x, y)).map_or(Hit::None, Hit::PerfItem)
            }
            Page::Settings => lay.setting_cards.iter().position(|c| c.0.contains(x, y)).map_or(Hit::None, Hit::Setting),
            _ if ui.has_table() => {
                if lay.header.contains(x, y) {
                    let cols = columns(ui.page);
                    return (0..cols.len()).find(|&i| x >= lay.col_x[i] && x < lay.col_x[i + 1]).map_or(Hit::None, Hit::Header);
                }
                if lay.rows_area.contains(x, y) {
                    let content = ui.rows.len() as f64 * (ROW_H * s).round();
                    if content > lay.rows_area.h() as f64 && x >= lay.rows_area.r - (10.0 * s) as i32 {
                        return Hit::VBar;
                    }
                    let i = ((y - lay.rows_area.t) as f64 + ui.scroll) / (ROW_H * s).round();
                    let i = i.floor() as usize;
                    if i < ui.rows.len() {
                        let r = &ui.rows[i];
                        if r.is_section {
                            return Hit::None;
                        }
                        let chev_x = lay.col_x[0] + (12.0 * s) as i32 + r.depth as i32 * (22.0 * s) as i32;
                        if matches!(ui.page, Page::Processes | Page::Users) && r.expandable && x >= chev_x - (4.0 * s) as i32 && x < chev_x + (22.0 * s) as i32 {
                            return Hit::Chevron(i);
                        }
                        return Hit::Row(i);
                    }
                }
                Hit::None
            }
            _ => Hit::None,
        }
    }

    pub fn btn_enabled(&self, ui: &Ui, b: Btn) -> bool {
        let has_sel = ui.sel.is_some();
        match b {
            Btn::RunTask | Btn::More => true,
            Btn::EndTask | Btn::Efficiency | Btn::StartEnable | Btn::StartDisable | Btn::StartDelete => has_sel,
        }
    }

    pub fn on_mouse_move(&self, hwnd: HWND, x: i32, y: i32) {
        let mut ui = self.ui.borrow_mut();
        ui.mouse = (x, y);
        let hit = self.hit_test(&ui, x, y);
        let mut redraw = hit != ui.hit;
        ui.hit = hit;
        let s = self.scale.get();
        if ui.perf_drag
            && let Some((body_l, body_w)) = ui.lay.as_ref().map(|l| (l.body.l, l.body.w()))
        {
            let max_w = ((body_w as f64 / s) - 240.0).max(120.0);
            ui.perf_split = Some(((x - body_l) as f64 / s).clamp(120.0, max_w));
            redraw = true;
        }
        if ui.drag.is_some()
            && let Some((sy, s0)) = ui.drag
            && let Some(area_h) = ui.lay.as_ref().map(|l| l.rows_area.h() as f64)
        {
            let content = ui.rows.len() as f64 * (ROW_H * s).round();
            if content > area_h {
                let thumb = ((area_h / content) * area_h).max(24.0 * s);
                ui.scroll = (s0 + (y - sy) as f64 * (content - area_h) / (area_h - thumb)).clamp(0.0, content - area_h);
                redraw = true;
            }
        }
        drop(ui);
        unsafe {
            let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut t);
        }
        if redraw {
            self.invalidate();
        }
    }

    pub fn on_mouse_leave(&self) {
        let mut ui = self.ui.borrow_mut();
        ui.hit = Hit::None;
        drop(ui);
        self.invalidate();
    }

    pub fn on_lbutton_down(&self, hwnd: HWND, x: i32, y: i32) {
        unsafe { SetFocus(hwnd) };
        let hit = {
            let ui = self.ui.borrow();
            self.hit_test(&ui, x, y)
        };
        match hit {
            Hit::NavToggle => {
                let mut ui = self.ui.borrow_mut();
                ui.nav_open = !ui.nav_open;
            }
            Hit::Nav(i) => self.go_page(NAV[i].0),
            Hit::Button(b) => self.on_button(hwnd, b),
            Hit::Header(i) => {
                {
                    let mut ui = self.ui.borrow_mut();
                    let cols = columns(ui.page);
                    let cur = ui.sort_of(ui.page);
                    let numeric = cols[i].align == Align::Right || cols[i].metric.is_some();
                    let ns = if cur.0 == i { (i, !cur.1) } else { (i, numeric) };
                    let p = ui.page;
                    ui.sorts.insert(p, ns);
                }
                self.rebuild();
            }
            Hit::Chevron(i) => {
                let key = self.ui.borrow().rows.get(i).map(|r| r.key.clone());
                if let Some(k) = key {
                    self.toggle_expand(&k);
                }
            }
            Hit::Row(i) => {
                let mut dbl: Option<String> = None;
                {
                    let mut ui = self.ui.borrow_mut();
                    if let Some(r) = ui.rows.get(i) {
                        let key = r.key.clone();
                        let now = Instant::now();
                        if let Some((t, k)) = &ui.last_click
                            && *k == key && now.duration_since(*t) < Duration::from_millis(450) {
                                dbl = Some(key.clone());
                            }
                        ui.last_click = Some((now, key.clone()));
                        ui.sel = Some(key);
                    }
                }
                if let Some(k) = dbl {
                    self.toggle_expand(&k);
                }
            }
            Hit::VBar => {
                let mut ui = self.ui.borrow_mut();
                ui.drag = Some((y, ui.scroll));
                unsafe { SetCapture(hwnd) };
            }
            Hit::PerfSplit => {
                self.ui.borrow_mut().perf_drag = true;
                unsafe { SetCapture(hwnd) };
            }
            Hit::PerfItem(i) => {
                let kind = perf_kinds(&self.data.borrow()).get(i).copied();
                if let Some(k) = kind {
                    self.ui.borrow_mut().perf_sel = k;
                    self.data.borrow_mut().want_gpu_temp = matches!(k, PerfKind::Gpu(_));
                }
            }
            Hit::Setting(i) => self.on_setting(i, x),
            Hit::None => {}
        }
        self.invalidate();
    }

    pub fn on_lbutton_up(&self) {
        let mut ui = self.ui.borrow_mut();
        let was_drag = ui.drag.take().is_some();
        let was_split = std::mem::replace(&mut ui.perf_drag, false);
        if was_drag || was_split {
            unsafe { ReleaseCapture() };
            drop(ui);
            self.invalidate();
        }
    }

    pub fn on_wheel(&self, delta: i16) {
        let mut ui = self.ui.borrow_mut();
        ui.scroll -= delta as f64 / 120.0 * 3.0 * (ROW_H * self.scale.get()).round();
        drop(ui);
        self.invalidate();
    }

    pub fn on_rbutton_up(&self, hwnd: HWND, x: i32, y: i32) {
        let hit = {
            let ui = self.ui.borrow();
            self.hit_test(&ui, x, y)
        };
        if let Hit::Row(i) | Hit::Chevron(i) = hit {
            {
                let mut ui = self.ui.borrow_mut();
                if let Some(r) = ui.rows.get(i) {
                    ui.sel = Some(r.key.clone());
                }
            }
            self.invalidate();
            let items = self.row_menu();
            let (px, py) = self.to_screen(hwnd, x, y);
            if let Some(a) = self.show_menu(hwnd, &items, px, py) {
                self.do_act(a);
            }
        }
    }

    fn to_screen(&self, hwnd: HWND, x: i32, y: i32) -> (i32, i32) {
        let mut p = winapi::shared::windef::POINT { x, y };
        unsafe { ClientToScreen(hwnd, &mut p) };
        (p.x, p.y)
    }

    pub fn on_key(&self, hwnd: HWND, vk: u32) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
        let (page, n) = {
            let ui = self.ui.borrow();
            (ui.page, ui.rows.len())
        };
        if ctrl && vk == 'F' as u32 && self.ui.borrow().page_has_search() {
            unsafe { SetFocus(self.search.handle.hwnd().unwrap_or(hwnd)) };
            return;
        }
        if !self.ui.borrow().has_table() || n == 0 {
            return;
        }
        let cur = {
            let ui = self.ui.borrow();
            ui.sel.as_ref().and_then(|k| ui.rows.iter().position(|r| &r.key == k))
        };
        let step = ((self.ui.borrow().lay.as_ref().map_or(10, |l| l.rows_area.h()) as f64 / (ROW_H * self.scale.get()).round()) as usize).max(1);
        let target = match vk as i32 {
            VK_DOWN => Some(cur.map_or(0, |c| (c + 1).min(n - 1))),
            VK_UP => Some(cur.map_or(0, |c| c.saturating_sub(1))),
            VK_NEXT => Some(cur.map_or(0, |c| (c + step).min(n - 1))),
            VK_PRIOR => Some(cur.map_or(0, |c| c.saturating_sub(step))),
            VK_HOME => Some(0),
            VK_END => Some(n - 1),
            VK_RIGHT | VK_LEFT => {
                let key = self.ui.borrow().sel.clone();
                if let Some(k) = key {
                    let want_open = vk as i32 == VK_RIGHT;
                    let open = self.ui.borrow().expanded.contains(&k);
                    if want_open != open && matches!(page, Page::Processes | Page::Users) {
                        self.toggle_expand(&k);
                    }
                }
                None
            }
            VK_DELETE => {
                self.do_act(if page == Page::Startup { Act::StartDelete } else { Act::EndTask });
                None
            }
            VK_F5 => {
                self.tick();
                None
            }
            _ => None,
        };
        if let Some(mut t) = target {
            let mut ui = self.ui.borrow_mut();
            let dir = if matches!(vk as i32, VK_UP | VK_PRIOR | VK_END) { -1isize } else { 1isize };
            while ui.rows.get(t).is_some_and(|r| r.is_section) {
                let next = t as isize + dir;
                if next < 0 || next >= n as isize {
                    break;
                }
                t = next as usize;
            }
            if ui.rows.get(t).is_some_and(|r| !r.is_section) {
                ui.sel = Some(ui.rows[t].key.clone());
                if let Some(area_h) = ui.lay.as_ref().map(|l| l.rows_area.h() as f64) {
                    let rh = (ROW_H * self.scale.get()).round();
                    let (top, bot) = (t as f64 * rh, (t as f64 + 1.0) * rh);
                    if top < ui.scroll {
                        ui.scroll = top;
                    } else if bot > ui.scroll + area_h {
                        ui.scroll = bot - area_h;
                    }
                }
            }
        }
        self.invalidate();
    }

    pub fn go_page(&self, p: Page) {
        {
            let mut ui = self.ui.borrow_mut();
            if ui.page == p {
                return;
            }
            ui.page = p;
            ui.scroll = 0.0;
            ui.sel = None;
            ui.search.clear();
            ui.rows.clear();
        }
        self.search.set_text("");
        if p == Page::Startup {
            self.data.borrow_mut().load_startup();
        }
        self.rebuild();
        self.save_settings();
    }

    pub fn rebuild(&self) {
        let d = self.data.borrow();
        let mut ui = self.ui.borrow_mut();
        if !ui.has_table() {
            return;
        }
        let rows = {
            let ctx = BuildCtx { page: ui.page, search: &ui.search, expanded: &ui.expanded, sort: ui.sort_of(ui.page) };
            build_rows(&d, &ctx)
        };
        if let Some(k) = ui.sel.clone()
            && !rows.iter().any(|r| r.key == k) {
                ui.sel = None;
            }
        ui.rows = rows;
    }

    fn toggle_expand(&self, key: &str) {
        {
            let mut ui = self.ui.borrow_mut();
            if !ui.expanded.remove(key) {
                ui.expanded.insert(key.to_string());
            }
        }
        self.rebuild();
    }

    fn hidden(&self) -> bool {
        self.win.handle.hwnd().is_none_or(|h| unsafe { IsWindowVisible(h) == 0 || IsIconic(h) != 0 })
    }

    pub fn on_timer(&self) {
        let every = if self.hidden() { 4900 } else { 900 };
        if self.last_tick.get().elapsed() >= Duration::from_millis(every) {
            self.last_tick.set(Instant::now());
            self.tick();
        }
    }

    pub fn tick(&self) {
        {
            let ui = self.ui.borrow();
            self.data.borrow_mut().want_gpu_temp = ui.page == Page::Performance && matches!(ui.perf_sel, PerfKind::Gpu(_));
        }
        self.data.borrow_mut().refresh();
        if self.hidden() {
            return;
        }
        self.rebuild();
        {
            let mut ui = self.ui.borrow_mut();
            if ui.toast.as_ref().is_some_and(|t| t.1.elapsed() > Duration::from_secs(4)) {
                ui.toast = None;
            }
        }
        self.invalidate();
    }

    pub fn toast(&self, msg: &str) {
        self.ui.borrow_mut().toast = Some((msg.to_string(), Instant::now()));
        self.invalidate();
    }

    pub fn on_search_changed(&self) {
        let t = self.search.text();
        {
            let mut ui = self.ui.borrow_mut();
            ui.search = t;
            ui.scroll = 0.0;
        }
        self.rebuild();
        self.invalidate();
    }

    fn on_button(&self, hwnd: HWND, b: Btn) {
        match b {
            Btn::RunTask => self.do_act(Act::RunTask),
            Btn::EndTask => self.do_act(Act::EndTask),
            Btn::Efficiency => self.do_act(Act::Efficiency),
            Btn::StartEnable => self.do_act(Act::StartEnable),
            Btn::StartDisable => self.do_act(Act::StartDisable),
            Btn::StartDelete => self.do_act(Act::StartDelete),
            Btn::More => {
                let items = self.more_menu();
                let r = self.ui.borrow().lay.as_ref().and_then(|l| l.buttons.iter().find(|x| x.1 == Btn::More).map(|x| x.0));
                if let Some(r) = r {
                    let (x, y) = self.to_screen(hwnd, r.r - 220, r.b + 2);
                    if let Some(a) = self.show_menu(hwnd, &items, x, y) {
                        self.do_act(a);
                    }
                }
            }
        }
    }

    fn on_setting(&self, i: usize, x: i32) {
        let id = self.ui.borrow().lay.as_ref().and_then(|l| l.setting_cards.get(i).map(|c| (c.1, c.0)));
        let Some((id, r)) = id else { return };
        match id {
            SettingId::Theme => {
                let s = self.scale.get();
                let ctl_l = r.r - (200.0 * s) as i32;
                let seg = ((x - ctl_l) / (((200.0 * s) as i32 - (16.0 * s) as i32) / 3).max(1)).clamp(0, 2);
                self.do_act(Act::SetTheme(seg as u8));
            }
            SettingId::Tray => self.do_act(Act::ToggleTray),
            SettingId::Autostart => self.do_act(Act::ToggleAutostart),
            SettingId::Etw => self.do_act(Act::ToggleEtw),
            SettingId::GpuTemp => self.do_act(Act::ToggleGpuTemp),
        }
    }

    fn selected_row(&self) -> Option<Row> {
        let ui = self.ui.borrow();
        ui.sel.as_ref().and_then(|k| ui.rows.iter().find(|r| &r.key == k).cloned())
    }

    fn row_menu(&self) -> Vec<MI> {
        let page = self.ui.borrow().page;
        match page {
            Page::Processes | Page::Users | Page::Details => {
                let mut v = vec![];
                if let Some(r) = self.selected_row()
                    && r.expandable {
                        v.push(MI::Item(if r.expanded { "Collapse" } else { "Expand" }, Act::Toggle));
                        v.push(MI::Sep);
                    }
                v.extend([
                    MI::Item("End task", Act::EndTask),
                    MI::Item("End process tree", Act::EndTree),
                    MI::Item("Efficiency mode", Act::Efficiency),
                    MI::Item("Suspend", Act::Suspend(true)),
                    MI::Item("Resume", Act::Suspend(false)),
                    MI::Sep,
                    MI::Sub("Priority", sys::PRIORITIES.iter().map(|p| MI::Item(p.0, Act::Prio(p.1))).collect()),
                    MI::Sub("CPU cores", vec![MI::Item("All cores", Act::Affinity(false)), MI::Item("Half of the cores only", Act::Affinity(true))]),
                    MI::Sep,
                    MI::Item("Open file location", Act::OpenDir),
                    MI::Item("Copy command line", Act::CopyCmd),
                ]);
                v
            }
            Page::Startup => vec![MI::Item("Enable", Act::StartEnable), MI::Item("Disable", Act::StartDisable), MI::Item("Delete", Act::StartDelete), MI::Sep, MI::Item("Open file location", Act::OpenDir)],
            _ => vec![],
        }
    }

    fn more_menu(&self) -> Vec<MI> {
        let d = self.data.borrow();
        vec![
            MI::Item("Run new task", Act::RunTask),
            MI::Item("Export to CSV", Act::Export),
            MI::Sep,
            MI::Sub("Theme", vec![MI::Check("Light", Act::SetTheme(0), d.cfg.theme == 0), MI::Check("Dark", Act::SetTheme(1), d.cfg.theme == 1), MI::Check("System", Act::SetTheme(2), d.cfg.theme == 2)]),
            MI::Item("Settings", Act::Go(Page::Settings)),
        ]
        .into_iter()
        .chain((!d.is_admin).then_some([MI::Sep, MI::Item("Restart as administrator", Act::RelaunchAdmin)]).into_iter().flatten())
        .collect()
    }

    pub fn tray_menu(&self) -> Vec<MI> {
        let d = self.data.borrow();
        vec![
            MI::Item("Show window", Act::ShowWindow),
            MI::Sep,
            MI::Check("Closing the window minimizes to tray", Act::ToggleTray, d.cfg.to_tray),
            MI::Check("Start with Windows", Act::ToggleAutostart, startup::self_autostart()),
            MI::Check("Per-process network (ETW, administrator)", Act::ToggleEtw, d.etw.is_some()),
            MI::Sep,
        ]
        .into_iter()
        .chain((!d.is_admin).then_some(MI::Item("Restart as administrator", Act::RelaunchAdmin)))
        .chain([MI::Item("Exit", Act::Exit)])
        .collect()
    }

    pub fn show_menu(&self, hwnd: HWND, items: &[MI], x: i32, y: i32) -> Option<Act> {
        if items.is_empty() {
            return None;
        }
        set_menu_theme(self.ui.borrow().th.dark);
        let mut acts: Vec<Act> = vec![];
        let menu = build_menu(items, &mut acts);
        let id = unsafe {
            SetForegroundWindow(hwnd);
            TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, x, y, hwnd, null_mut())
        };
        unsafe { DestroyMenu(menu) };
        (id > 0).then(|| acts.get(id as usize - 1).copied()).flatten()
    }

    pub fn do_act(&self, act: Act) {
        match act {
            Act::ShowWindow => return self.show_window(),
            Act::Exit => return self.quit(),
            Act::RelaunchAdmin => {
                self.save_settings();
                if sys::relaunch_as_admin("") {
                    return self.quit();
                }
                return self.toast("Running as administrator was cancelled");
            }
            Act::Go(p) => return self.go_page(p),
            Act::RunTask => {
                std::thread::spawn(|| unsafe {
                    use winapi::um::libloaderapi::{FreeLibrary, GetProcAddress, LoadLibraryW};
                    let dll: Vec<u16> = "shell32.dll\0".encode_utf16().collect();
                    let sh = LoadLibraryW(dll.as_ptr());
                    if !sh.is_null() {
                        let f = GetProcAddress(sh, 61 as *const i8);
                        if !f.is_null() {
                            type RunFileDlg = unsafe extern "system" fn(winapi::shared::windef::HWND, winapi::shared::windef::HICON, *const u16, *const u16, *const u16, u32);
                            std::mem::transmute::<_, RunFileDlg>(f)(null_mut(), null_mut(), null(), null(), null(), 0);
                        }
                        FreeLibrary(sh);
                    }
                });
                return;
            }
            Act::SetTheme(t) => {
                self.data.borrow_mut().cfg.theme = t;
                self.apply_theme();
                self.save_settings();
                return;
            }
            Act::ToggleTray => {
                let on = {
                    let mut d = self.data.borrow_mut();
                    d.cfg.to_tray = !d.cfg.to_tray;
                    d.cfg.to_tray
                };
                self.toast(if on { "Closing the window will minimize to tray" } else { "Closing the window will quit the program" });
                self.save_settings();
                return;
            }
            Act::ToggleAutostart => {
                let want = !startup::self_autostart();
                let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
                let ok = startup::set_self_autostart(want, &exe);
                self.toast(match (ok, startup::self_autostart()) {
                    (true, true) => "The program will start with Windows (straight to the tray)",
                    (true, false) => "Autostart is disabled",
                    _ => "Could not change autostart",
                });
                return;
            }
            Act::ToggleEtw => {
                let msg = {
                    let mut d = self.data.borrow_mut();
                    if !d.is_admin {
                        "Per-process network needs administrator rights - run the program as administrator".to_string()
                    } else if let Some(s) = d.etw.take() {
                        crate::net::stop(s);
                        d.net.clear();
                        d.cfg.etw = false;
                        "Per-process network is off".into()
                    } else {
                        d.etw = crate::net::start();
                        d.cfg.etw = d.etw.is_some();
                        if d.etw.is_some() { "Per-process network is on ('Network' column)".into() } else { "Could not start ETW".into() }
                    }
                };
                self.toast(&msg);
                self.save_settings();
                return;
            }
            Act::ToggleGpuTemp => {
                let on = {
                    let mut d = self.data.borrow_mut();
                    d.cfg.gpu_temp_always = !d.cfg.gpu_temp_always;
                    d.cfg.gpu_temp_always
                };
                self.toast(if on { "GPU temperature is read continuously" } else { "GPU temperature is read only on the GPU page" });
                self.save_settings();
                return;
            }
            Act::Export => return self.export(),
            _ => {}
        }
        let page = self.ui.borrow().page;
        let Some(row) = self.selected_row() else {
            self.toast("Select a row in the list first");
            return;
        };
        if act == Act::Toggle {
            return self.toggle_expand(&row.key);
        }
        if page == Page::Startup {
            return self.act_startup(&row, act);
        }
        self.act_process(&row, act);
    }

    fn act_process(&self, row: &Row, act: Act) {
        let mut pids: Vec<u32> = row.pids.iter().copied().filter(|&p| p != 0).collect();
        if pids.is_empty() {
            return;
        }
        if matches!(act, Act::EndTask | Act::EndTree | Act::Suspend(true)) {
            let d = self.data.borrow();
            let total = pids.len();
            pids.retain(|&p| d.sys.process(sysinfo::Pid::from_u32(p)).is_none_or(|x| !CRITICAL.contains(&x.name().to_string_lossy().to_lowercase().as_str())));
            if pids.len() < total {
                drop(d);
                self.toast("Critical system processes (csrss, lsass, smss, etc.) are protected and skipped");
                if pids.is_empty() {
                    return;
                }
            }
        }
        if (act == Act::EndTask && pids.len() > 1 || act == Act::EndTree) && !self.confirm(&format!("End {} processes (for a tree, together with their children)?", pids.len())) {
            return;
        }
        let report = |what: &str, ok: usize| {
            self.toast(&if ok == pids.len() { format!("{what}: done ({ok})") } else { format!("{what}: {ok} of {} succeeded - the rest need administrator rights{}", pids.len(), if self.data.borrow().is_admin { "" } else { " (menu '…' → 'Restart as administrator')" }) });
        };
        match act {
            Act::EndTask => {
                let d = self.data.borrow();
                let ok = pids.iter().filter(|&&p| d.sys.process(sysinfo::Pid::from_u32(p)).is_some_and(|x| x.kill())).count();
                drop(d);
                report("Ending", ok);
            }
            Act::EndTree => {
                let d = self.data.borrow();
                let parents: HashMap<u32, u32> = d.procs.iter().map(|p| (p.pid, p.ppid)).collect();
                let all = descendants(&parents, &pids);
                let ok = all.iter().filter(|&&p| d.sys.process(sysinfo::Pid::from_u32(p)).is_some_and(|x| x.kill())).count();
                drop(d);
                self.toast(&format!("Ended {ok} of {} (with children)", all.len()));
            }
            Act::Prio(c) => report("Priority", pids.iter().filter(|&&p| sys::set_priority(p, c)).count()),
            Act::Suspend(on) => {
                let ok: Vec<u32> = pids.iter().copied().filter(|&p| sys::suspend(p, on)).collect();
                {
                    let mut d = self.data.borrow_mut();
                    for p in &ok {
                        if on { d.suspended.insert(*p) } else { d.suspended.remove(p) };
                    }
                }
                report(if on { "Suspending" } else { "Resuming" }, ok.len());
            }
            Act::Efficiency => {
                let on = !pids.iter().all(|p| self.data.borrow().efficiency.contains(p));
                let ok: Vec<u32> = pids.iter().copied().filter(|&p| sys::set_efficiency(p, on)).collect();
                {
                    let mut d = self.data.borrow_mut();
                    for p in &ok {
                        if on { d.efficiency.insert(*p) } else { d.efficiency.remove(p) };
                    }
                }
                report(if on { "Efficiency mode on" } else { "Efficiency mode off" }, ok.len());
            }
            Act::Affinity(half) => report("Core affinity", pids.iter().filter(|&&p| sys::set_affinity(p, half)).count()),
            Act::OpenDir => {
                let d = self.data.borrow();
                let path = pids.iter().find_map(|&p| d.procs.iter().find(|x| x.pid == p).and_then(|x| x.exe.clone()));
                drop(d);
                match path {
                    Some(p) => winrust_common::reveal(&p.replace("\\..\\", "\\")),
                    None => self.toast("File path unavailable (system or protected process)"),
                }
            }
            Act::CopyCmd => {
                let d = self.data.borrow();
                let cmd = process_cmd(&d, pids[0]);
                drop(d);
                if cmd.is_empty() {
                    self.toast("Command line unavailable for this process");
                } else {
                    nwg::Clipboard::set_data_text(&self.win, &cmd);
                    self.toast("Command line copied");
                }
            }
            _ => {}
        }
    }

    fn act_startup(&self, row: &Row, act: Act) {
        if act == Act::StartDelete {
            let name = {
                let d = self.data.borrow();
                d.startup.iter().find(|s| s.key() == row.key).map(|s| s.name.clone())
            };
            let Some(name) = name else { return };
            if !self.confirm(&format!("Permanently delete '{}' from startup?", name)) {
                return;
            }
            let msg = {
                let d = self.data.borrow();
                match d.startup.iter().find(|s| s.key() == row.key) {
                    Some(it) => match startup::delete(it) {
                        Ok(()) => format!("'{}': deleted from startup", it.name),
                        Err(5) => format!("'{}': administrator rights needed", it.name),
                        Err(e) => format!("'{}': Win32 error {e}", it.name),
                    },
                    None => return,
                }
            };
            self.data.borrow_mut().load_startup();
            self.rebuild();
            self.toast(&msg);
            return;
        }
        let on = match act {
            Act::StartEnable => true,
            Act::StartDisable => false,
            Act::OpenDir => {
                if let Some(p) = &row.icon_path { winrust_common::reveal(p); }
                return;
            }
            _ => return,
        };
        let msg = {
            let d = self.data.borrow();
            match d.startup.iter().find(|s| s.key() == row.key) {
                Some(it) => match startup::set_enabled(it, on) {
                    Ok(()) => format!("«{}»: {}", it.name, if on { "enabled" } else { "disabled - will not start with Windows" }),
                    Err(5) => format!("'{}': administrator rights needed", it.name),
                    Err(e) => format!("'{}': Win32 error {e}", it.name),
                },
                None => return,
            }
        };
        self.data.borrow_mut().load_startup();
        self.rebuild();
        self.toast(&msg);
    }

    fn confirm(&self, text: &str) -> bool {
        let p = nwg::MessageParams { title: "Confirmation", content: text, buttons: nwg::MessageButtons::YesNo, icons: nwg::MessageIcons::Warning };
        nwg::modal_message(&self.win, &p) == nwg::MessageChoice::Yes
    }

    fn export(&self) {
        let ui = self.ui.borrow();
        if !ui.has_table() {
            drop(ui);
            return self.toast("Nothing to export on this page");
        }
        let esc = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("\u{feff}");
        out += &columns(ui.page).iter().map(|c| esc(c.title)).collect::<Vec<_>>().join(";");
        for r in &ui.rows {
            out += "\r\n";
            out += &r.cells.iter().map(|c| esc(c)).collect::<Vec<_>>().join(";");
        }
        let (y, mo, d, h, mi, s) = sys::local_time();
        let dir = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).unwrap_or_default().join("Desktop");
        let path = dir.join(format!("rtaskmgr-{}-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}.csv", page_title(ui.page)));
        drop(ui);
        match std::fs::write(&path, out) {
            Ok(()) => self.toast(&format!("Saved: {}", path.display())),
            Err(e) => self.toast(&format!("Could not save CSV: {e}")),
        }
    }

    pub fn apply_theme(&self) {
        let dark = {
            let d = self.data.borrow();
            match d.cfg.theme {
                0 => false,
                1 => true,
                _ => sys::system_dark(),
            }
        };
        let th = theme(dark);
        self.ui.borrow_mut().th = th;
        if let Some(h) = self.win.handle.hwnd() {
            sys::set_dark_titlebar(h, dark);
        }
        unsafe {
            let old = self.edit_brush.replace(winapi::um::wingdi::CreateSolidBrush(th.panel.c()) as isize);
            if old != 0 {
                winapi::um::wingdi::DeleteObject(old as _);
            }
        }
        set_menu_theme(dark);
        self.invalidate();
        if let Some(h) = self.search.handle.hwnd() {
            unsafe { InvalidateRect(h, null(), 1) };
        }
    }

    pub fn save_settings(&self) {
        let ui = self.ui.borrow();
        let mut d = self.data.borrow_mut();
        d.cfg.view = NAV.iter().position(|n| n.0 == ui.page).unwrap_or(0);
        let (sc, sd) = ui.sort_of(Page::Processes);
        d.cfg.sort = sc;
        d.cfg.desc = sd;
        d.cfg.etw = d.etw.is_some();
        let (p, s) = (self.win.position(), self.win.size());
        if p.0 > -10000 && s.0 > 100 {
            d.cfg.pos = p;
            d.cfg.size = s;
        }
        store::save(&d.cfg);
    }

    pub fn show_window(&self) {
        self.win.set_visible(true);
        if let Some(h) = self.win.handle.hwnd() {
            unsafe {
                ShowWindow(h, SW_RESTORE);
                SetForegroundWindow(h);
            }
        }
        self.last_tick.set(Instant::now());
        self.tick();
    }

    pub fn quit(&self) {
        self.save_settings();
        if let Some(h) = self.win.handle.hwnd() {
            unsafe { UnregisterHotKey(h, 1) };
        }
        if let Some(s) = self.data.borrow_mut().etw.take() {
            crate::net::stop(s);
        }
        self.tray.set_visibility(false);
        nwg::stop_thread_dispatch();
    }
}

pub fn descendants(parent_of: &HashMap<u32, u32>, roots: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = roots.to_vec();
    let mut i = 0;
    while i < out.len() {
        let cur = out[i];
        for (&pid, &par) in parent_of {
            if par == cur && !out.contains(&pid) {
                out.push(pid);
            }
        }
        i += 1;
    }
    out
}

fn build_menu(items: &[MI], acts: &mut Vec<Act>) -> HMENU {
    unsafe {
        let m = CreatePopupMenu();
        for it in items {
            match it {
                MI::Item(t, a) => {
                    acts.push(*a);
                    AppendMenuW(m, MF_STRING, acts.len(), wide(t).as_ptr());
                }
                MI::Check(t, a, on) => {
                    acts.push(*a);
                    AppendMenuW(m, MF_STRING | if *on { MF_CHECKED } else { 0 }, acts.len(), wide(t).as_ptr());
                }
                MI::Sep => {
                    AppendMenuW(m, MF_SEPARATOR, 0, null());
                }
                MI::Sub(t, sub) => {
                    let h = build_menu(sub, acts);
                    AppendMenuW(m, MF_POPUP, h as usize, wide(t).as_ptr());
                }
            }
        }
        m
    }
}

pub fn set_menu_theme(dark: bool) {
    use std::sync::OnceLock;
    use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};
    type SetFn = extern "system" fn(i32) -> i32;
    type FlushFn = extern "system" fn();
    static FUNCS: OnceLock<(Option<SetFn>, Option<FlushFn>)> = OnceLock::new();
    let (set, flush) = *FUNCS.get_or_init(|| unsafe {
        let lib = LoadLibraryW(wide("uxtheme.dll").as_ptr());
        if lib.is_null() {
            return (None, None);
        }
        let s = GetProcAddress(lib, 135 as *const i8);
        let f = GetProcAddress(lib, 136 as *const i8);
        (
            (!s.is_null()).then(|| std::mem::transmute::<_, SetFn>(s)),
            (!f.is_null()).then(|| std::mem::transmute::<_, FlushFn>(f)),
        )
    });
    if let Some(f) = set {
        f(if dark { 2 } else { 3 });
    }
    if let Some(f) = flush {
        f();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_collects_all_once() {
        let parents = HashMap::from([(2, 1), (3, 2), (4, 1), (9, 8)]);
        let mut got = descendants(&parents, &[1]);
        got.sort();
        assert_eq!(got, vec![1, 2, 3, 4]);
        assert_eq!(descendants(&parents, &[9]), vec![9]);
    }
}
