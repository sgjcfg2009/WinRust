use crate::{app::*, data::*, model::*, ui::*};
use std::collections::{HashMap, VecDeque};
use winapi::shared::windef::POINT;

pub const ROW_H: f64 = 28.0;
pub const NAV_W: f64 = 48.0;
pub const NAV_W_OPEN: f64 = 220.0;
pub const BAND_H: f64 = 52.0;

pub fn series_color(i: usize, dark: bool) -> Rgb {
    let c = [Rgb(40, 168, 234), Rgb(139, 92, 246), Rgb(16, 185, 129), Rgb(236, 72, 153), Rgb(168, 85, 247)][i.min(4)];
    if dark { c } else { c.lerp(Rgb(0, 0, 0), 0.16) }
}

pub fn draw_graph(g: &G, r: R, series: &[(&VecDeque<f32>, Rgb)], ymax: f32, grid: bool) {
    let th = g.th;
    g.fill(r, th.bg.lerp(if th.dark { Rgb(0, 0, 0) } else { Rgb(255, 255, 255) }, 0.12));
    if grid {
        let (cols, rows) = (20, 10);
        let gc = th.border.lerp(th.bg, 0.25);
        for i in 1..cols {
            g.vline(r.l + r.w() * i / cols, r.t, r.b, gc);
        }
        for i in 1..rows {
            g.hline(r.l, r.r, r.t + r.h() * i / rows, gc);
        }
    }
    g.clip(r.inset(1));
    thread_local! { static PTS: std::cell::RefCell<Vec<POINT>> = const { std::cell::RefCell::new(Vec::new()) }; }
    PTS.with_borrow_mut(|pts| {
        for (data, color) in series {
            let n = data.len();
            if n < 2 {
                continue;
            }
            pts.clear();
            for (i, &v) in data.iter().enumerate() {
                pts.push(POINT {
                    x: r.r - ((n - 1 - i) as i32) * r.w() / (CAP as i32 - 1),
                    y: r.b - 1 - ((v / ymax.max(0.001)).clamp(0.0, 1.0) * (r.h() - 2) as f32) as i32,
                });
            }
            let x_end = pts[n - 1].x;
            let x_start = pts[0].x;
            pts.push(POINT { x: x_end, y: r.b });
            pts.push(POINT { x: x_start, y: r.b });
            g.polygon(pts, th.bg.lerp(*color, 0.28));
            g.polyline(&pts[..n], *color, g.px(1.5).max(1));
        }
    });
    g.unclip();
    let first = series.first().map_or(th.border, |s| s.1);
    g.frame(r, th.bg.lerp(first, 0.75));
}

impl App {
    pub fn draw_nav(&self, g: &G, ui: &Ui, lay: &Lay) {
        let th = g.th;
        g.fill(lay.nav, th.nav);
        g.glyph(lay.nav_toggle, '\u{E700}', th.text, false);
        if ui.hit == Hit::NavToggle {
            g.round(lay.nav_toggle.inset(g.px(2.0)), g.px(4.0), Some(th.hover), None);
            g.glyph(lay.nav_toggle, '\u{E700}', th.text, false);
        }
        for (i, r) in lay.nav_items.iter().enumerate() {
            if i + 1 == NAV.len() {
                g.hline(r.l + g.px(10.0), r.r - g.px(10.0), r.t - g.px(4.0), th.border);
            }
            let (page, icon, label) = NAV[i];
            let sel = page == ui.page;
            let hov = ui.hit == Hit::Nav(i);
            if sel || hov {
                g.round(r.inset(g.px(4.0)), g.px(5.0), Some(if sel { th.sel } else { th.hover }), None);
            }
            if sel {
                g.round(R::new(r.l + g.px(4.0), r.t + r.h() / 2 - g.px(8.0), r.l + g.px(7.0), r.t + r.h() / 2 + g.px(8.0)), g.px(2.0), Some(th.accent), None);
            }
            let ir = R::new(r.l, r.t, r.l + g.px(NAV_W), r.b);
            g.glyph(ir, icon, if sel { th.accent } else { th.text }, false);
            if ui.nav_open {
                g.text(R::new(r.l + g.px(NAV_W), r.t, r.r - g.px(8.0), r.b), label, Font::Ui, th.text);
            }
        }
    }

    pub fn draw_band(&self, g: &G, ui: &Ui, _d: &Data, lay: &Lay) {
        let th = g.th;
        g.fill(lay.band, th.bg);
        g.text(lay.title, page_title(ui.page), Font::Title, th.text);
        if ui.page_has_search() && lay.search.w() >= g.px(60.0) {
            g.round(lay.search, g.px(5.0), Some(th.panel), Some(th.border));
            g.glyph(R::new(lay.search.r - g.px(32.0), lay.search.t, lay.search.r - g.px(4.0), lay.search.b), '\u{E721}', th.dim, true);
        }
        for (r, btn, label, glyph) in &lay.buttons {
            let enabled = self.btn_enabled(ui, *btn);
            let hov = enabled && ui.hit == Hit::Button(*btn);
            if hov {
                g.round(*r, g.px(5.0), Some(th.hover), None);
            }
            let c = if enabled { th.text } else { th.dim.lerp(th.bg, 0.35) };
            let gr = R::new(r.l + g.px(8.0), r.t, r.l + g.px(30.0), r.b);
            g.glyph(gr, *glyph, if enabled && *btn == Btn::Efficiency { Rgb(48, 209, 88) } else { c }, true);
            if !label.is_empty() {
                g.text(R::new(r.l + g.px(32.0), r.t, r.r - g.px(8.0), r.b), label, Font::Ui, c);
            }
        }
        g.hline(lay.band.l, lay.band.r, lay.band.b - 1, th.border);
    }

    pub fn draw_table(&self, g: &G, ui: &Ui, d: &Data, lay: &Lay) {
        let th = g.th;
        let page = ui.page;
        let cols = columns(page);
        let xs = &lay.col_x;
        let area = lay.rows_area;

        let sort = ui.sort_of(page);
        for (i, c) in cols.iter().enumerate() {
            let cell = R::new(xs[i], lay.header.t, xs[i + 1], lay.header.b);
            let hov = ui.hit == Hit::Header(i);
            if hov || sort.0 == i {
                g.fill(cell, if sort.0 == i { th.sel } else { th.hover });
            }
            let pad = if cell.w() < g.px(84.0) { g.px(4.0) } else { g.px(8.0) };
            let inner = R::new(cell.l + pad, cell.t + g.px(4.0), cell.r - pad, cell.b - g.px(4.0));
            if let Some(m) = c.metric {
                let val = agg_text(d, m);
                if page == Page::Processes {
                    let top = R::new(inner.l, cell.t + g.px(8.0), inner.r, cell.t + g.px(32.0));
                    g.text_right(top, &val, Font::Big, th.text);
                    g.text_right(R::new(inner.l, cell.t + g.px(32.0), inner.r, cell.b - g.px(4.0)), c.title, Font::Small, th.dim);
                } else {
                    g.text_right(inner, c.title, Font::Semibold, th.text);
                }
            } else if c.align == Align::Right {
                g.text_right(R::new(inner.l, cell.t, inner.r, cell.b - if page == Page::Processes { g.px(4.0) } else { 0 }), c.title, if page == Page::Processes { Font::Small } else { Font::Semibold }, if page == Page::Processes { th.dim } else { th.text });
            } else {
                let lr = R::new(inner.l + if i == 0 { g.px(12.0) } else { 0 }, if page == Page::Processes { cell.b - g.px(28.0) } else { cell.t }, inner.r, cell.b - if page == Page::Processes { g.px(2.0) } else { 0 });
                g.text(lr, c.title, if page == Page::Processes { Font::Small } else { Font::Semibold }, if page == Page::Processes { th.dim } else { th.text });
            }
            if sort.0 == i {
                let ch = if sort.1 { '\u{E70D}' } else { '\u{E70E}' };
                let ar = R::new(cell.l + (cell.w() - g.px(16.0)) / 2, cell.t + g.px(0.0), cell.l + (cell.w() + g.px(16.0)) / 2, cell.t + g.px(12.0));
                if c.metric.is_some() {
                    g.glyph(ar, ch, th.text, true);
                } else {
                    g.glyph(R::new(cell.r - g.px(24.0), cell.t, cell.r - g.px(4.0), cell.t + g.px(14.0)), ch, th.dim, true);
                }
            }
            if i > 0 {
                g.vline(cell.l, lay.header.t, lay.header.b, th.border);
            }
        }
        g.hline(lay.header.l, lay.header.r, lay.header.b - 1, th.border);

        let row_h = g.px(ROW_H);
        g.clip(area);
        let first = (ui.scroll / row_h as f64).floor().max(0.0) as usize;
        let last = ((ui.scroll + area.h() as f64) / row_h as f64).ceil() as usize + 1;
        for (i, row) in ui.rows.iter().enumerate().skip(first).take(last.saturating_sub(first)) {
            let y = area.t + i as i32 * row_h - ui.scroll as i32;
            if row.is_section {
                g.fill(R::new(area.l, y, xs[cols.len()].min(area.r), y + row_h), th.bg);
                let title = row.cells.first().map(String::as_str).unwrap_or("");
                g.text(R::new(xs[0] + g.px(12.0), y + g.px(2.0), xs[cols.len()].min(area.r) - g.px(12.0), y + row_h), title, Font::Semibold, th.text);
                continue;
            }
            let selected = ui.sel.as_deref() == Some(row.key.as_str());
            let hovered = matches!(ui.hit, Hit::Row(j) | Hit::Chevron(j) if j == i);
            let base = if selected { th.sel } else if hovered { th.hover } else { th.bg };
            g.fill(R::new(area.l, y, xs[cols.len()].min(area.r), y + row_h), base);
            for (ci, c) in cols.iter().enumerate() {
                let cell = R::new(xs[ci], y, xs[ci + 1], y + row_h);
                let mut tc = th.text;
                if let Some(Some((v, scale))) = row.heat.get(ci) {
                    let h = heat(&th, *v, *scale);
                    g.fill(cell, if selected { h.lerp(th.text, 0.08) } else if hovered { h.lerp(th.text, 0.04) } else { h });
                    if *v < 0.05 {
                        tc = th.dim;
                    } else if *v / *scale > 0.45 {
                        tc = Rgb(255, 255, 255);
                    }
                }
                let text = row.cells.get(ci).map(String::as_str).unwrap_or("");
                let pad = if cell.w() < g.px(84.0) { g.px(4.0) } else { g.px(8.0) };
                let inner = R::new(cell.l + pad, cell.t, cell.r - pad, cell.b);
                if ci == 0 {
                    let mut x = inner.l + g.px(4.0) + row.depth as i32 * g.px(22.0);
                    if matches!(page, Page::Processes | Page::Users) {
                        if row.expandable {
                            g.glyph(R::new(x - g.px(4.0), y, x + g.px(14.0), y + row_h), if row.expanded { '\u{E70D}' } else { '\u{E76C}' }, th.dim, true);
                        }
                        x += g.px(18.0);
                    }
                    if row.icon_path.is_some() || page == Page::Processes || (page == Page::Users && row.depth > 0) {
                        let sz = g.px(16.0);
                        let ico = row.icon_path.as_deref().map_or(0, |p| d.res.icon(p));
                        g.icon(if ico != 0 { ico } else { d.res.generic_icon }, x, y + (row_h - sz) / 2, sz);
                        x += sz + g.px(8.0);
                    }
                    let badge = if page == Page::Processes && !row.pids.is_empty() {
                        if row.pids.iter().all(|p| d.efficiency.contains(p)) {
                            Some(('\u{E945}', Rgb(74, 222, 128)))
                        } else if row.pids.iter().all(|p| d.suspended.contains(p)) {
                            Some(('\u{E769}', th.dim))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let tr = if badge.is_some() { inner.r - g.px(22.0) } else { inner.r };
                    g.text(R::new(x, y, tr, y + row_h), text, Font::Ui, th.text);
                    if let Some((gl, gc)) = badge {
                        g.glyph(R::new(inner.r - g.px(20.0), y, inner.r, y + row_h), gl, gc, true);
                    }
                } else if c.align == Align::Right {
                    g.text_right(inner, text, Font::Ui, tc);
                } else {
                    g.text(inner, text, Font::Ui, tc);
                }
            }
            for ci in 1..cols.len() {
                if cols[ci].metric.is_some() || cols[ci - 1].metric.is_some() {
                    g.vline(xs[ci], y, y + row_h, th.border);
                }
            }
        }
        g.unclip();

        if ui.rows.is_empty() {
            let msg = if ui.search.is_empty() { "No data" } else { "Nothing found" };
            g.text_center(R::new(area.l, area.t + g.px(30.0), area.r, area.t + g.px(70.0)), msg, Font::Ui, th.dim);
        }

        let content = ui.rows.len() as f64 * row_h as f64;
        if content > area.h() as f64 {
            let track = lay.vbar;
            let thumb_h = ((area.h() as f64 / content) * track.h() as f64).max(g.px(24.0) as f64);
            let max_scroll = content - area.h() as f64;
            let ty = track.t as f64 + (ui.scroll / max_scroll) * (track.h() as f64 - thumb_h);
            let active = ui.drag.is_some() || ui.hit == Hit::VBar;
            let w = if active { g.px(6.0) } else { g.px(3.0) };
            g.round(R::new(track.r - w - g.px(2.0), ty as i32, track.r - g.px(2.0), (ty + thumb_h) as i32), g.px(2.0), Some(th.dim.lerp(th.bg, if active { 0.0 } else { 0.35 })), None);
        }
    }

    pub fn draw_settings(&self, g: &G, ui: &Ui, d: &Data, lay: &Lay) {
        let th = g.th;
        g.clip(lay.body);
        for (r, id) in &lay.setting_cards {
            g.round(*r, g.px(6.0), Some(th.panel), Some(th.border));
            let (title, desc) = setting_text(*id);
            g.text(R::new(r.l + g.px(16.0), r.t + g.px(8.0), r.r - g.px(220.0), r.t + g.px(30.0)), title, Font::Semibold, th.text);
            g.text(R::new(r.l + g.px(16.0), r.t + g.px(30.0), r.r - g.px(220.0), r.b - g.px(6.0)), desc, Font::Small, th.dim);
            let ctl = R::new(r.r - g.px(200.0), r.t, r.r - g.px(16.0), r.b);
            match id {
                SettingId::Theme => {
                    let seg = ["Light", "Dark", "System"];
                    let sw = ctl.w() / 3;
                    for (i, s) in seg.iter().enumerate() {
                        let sr = R::new(ctl.l + i as i32 * sw, r.t + g.px(14.0), ctl.l + (i as i32 + 1) * sw, r.b - g.px(14.0));
                        let on = d.cfg.theme as usize == i;
                        g.round(sr, g.px(4.0), Some(if on { th.accent } else { th.bg }), Some(th.border));
                        g.text_center(sr, s, Font::Small, if on { if th.dark { Rgb(0, 0, 0) } else { Rgb(255, 255, 255) } } else { th.text });
                    }
                }
                _ => {
                    let on = setting_on(*id, d);
                    let enabled = !(*id == SettingId::Etw && !d.is_admin);
                    let t = R::new(ctl.r - g.px(44.0), r.t + r.h() / 2 - g.px(10.0), ctl.r, r.t + r.h() / 2 + g.px(10.0));
                    g.round(t, g.px(10.0), Some(if on { th.accent } else { th.bg }), Some(if on { th.accent } else { th.dim }));
                    let kn = g.px(7.0);
                    let cx = if on { t.r - g.px(11.0) } else { t.l + g.px(11.0) };
                    g.round(R::new(cx - kn, t.t + g.px(10.0) - kn, cx + kn, t.t + g.px(10.0) + kn), kn, Some(if on { if th.dark { Rgb(0, 0, 0) } else { Rgb(255, 255, 255) } } else { th.dim }), None);
                    if !enabled {
                        g.text_right(R::new(ctl.l, r.t, t.l - g.px(10.0), r.b), "administrator needed", Font::Small, th.dim);
                    }
                }
            }
        }
        g.unclip();
        let _ = ui;
    }

    pub fn draw_performance(&self, g: &G, ui: &Ui, d: &Data, lay: &Lay) {
        let th = g.th;
        let items = perf_items(d);
        let sel_idx = items.iter().position(|i| i.kind == ui.perf_sel).unwrap_or(0);
        g.fill(lay.perf_list, th.bg);
        for (i, r) in lay.perf_items.iter().enumerate() {
            let it = &items[i];
            let sel = sel_idx == i;
            let hov = ui.hit == Hit::PerfItem(i);
            if sel || hov {
                g.round(r.inset(g.px(2.0)), g.px(6.0), Some(if sel { th.sel } else { th.hover }), None);
            }
            if sel {
                g.round(R::new(r.l + g.px(2.0), r.t + r.h() / 2 - g.px(10.0), r.l + g.px(5.0), r.t + r.h() / 2 + g.px(10.0)), g.px(2.0), Some(th.accent), None);
            }
            let pad_y = (r.h() / 8).clamp(g.px(4.0), g.px(10.0));
            let thumb_w = (r.w() * 34 / 100).clamp(g.px(44.0), g.px(72.0));
            let thumb = R::new(r.l + g.px(8.0), r.t + pad_y, r.l + g.px(8.0) + thumb_w, r.b - pad_y);
            draw_graph(g, thumb, &it.graphs[0].series, it.graphs[0].ymax, false);
            let tx = thumb.r + g.px(8.0);
            let lh = ((r.h() - pad_y * 2) / 3).max(g.px(13.0));
            g.text(R::new(tx, r.t + pad_y, r.r - g.px(4.0), r.t + pad_y + lh), &it.title, Font::Ui, th.text);
            g.text(R::new(tx, r.t + pad_y + lh, r.r - g.px(4.0), r.t + pad_y + lh * 2), &it.line1, Font::Small, th.dim);
            if !it.line2.is_empty() && r.t + pad_y + lh * 3 <= r.b {
                g.text(R::new(tx, r.t + pad_y + lh * 2, r.r - g.px(4.0), r.b - pad_y / 2), &it.line2, Font::Small, th.dim);
            }
        }
        if ui.perf_drag || ui.hit == Hit::PerfSplit {
            g.fill(R::new(lay.perf_list.r - g.px(1.0), lay.perf_list.t, lay.perf_list.r + g.px(1.0), lay.perf_list.b), th.accent);
        } else {
            g.vline(lay.perf_list.r, lay.perf_list.t, lay.perf_list.b, th.border);
        }

        let m = lay.perf_main;
        let it = &items[sel_idx];
        let hdr_h = (m.h() / 11).clamp(g.px(32.0), g.px(48.0));
        let title_need = g.text_w(&it.big_title, Font::Huge) + g.px(12.0);
        let dev_big = if it.device.is_empty() { 0 } else { g.text_w(&it.device, Font::Big) + g.px(12.0) };
        let dev_font = if title_need + dev_big > m.w() { Font::Ui } else { Font::Big };
        let dev_need = if it.device.is_empty() { 0 } else { g.text_w(&it.device, dev_font) + g.px(10.0) };
        let hsplit = m.l + title_need.min((m.w() - dev_need).max(m.w() * 48 / 100));
        g.text(R::new(m.l, m.t, hsplit, m.t + hdr_h), &it.big_title, Font::Huge, th.text);
        g.text_right(R::new(hsplit, m.t + g.px(6.0), m.r, m.t + hdr_h), &it.device, dev_font, th.text);

        let cols = it.stat_cols.max(1);
        let stat_rows = it.stats.len().div_ceil(cols).max(1) as i32;
        let fact_rows = it.facts.len() as i32;
        let stack_facts = !it.facts.is_empty() && m.w() < g.px(430.0);
        let compact_v = m.h() < g.px(520.0) || stack_facts;
        let stat_row_h = if compact_v { g.px(40.0) } else { g.px(56.0) };
        let fact_row_h = if compact_v { g.px(18.0) } else { g.px(22.0) };
        let axis_h = g.px(22.0);
        let comp_h = if it.composition.is_some() { g.px(54.0) } else { 0 };
        let bottom_need = if stack_facts {
            stat_rows * stat_row_h + fact_rows * fact_row_h + g.px(12.0)
        } else {
            (stat_rows * stat_row_h).max(fact_rows * fact_row_h) + g.px(6.0)
        };
        let max_graph_frac = match it.layout {
            GLayout::Grid => 0.56,
            _ => 0.50,
        };
        let avail_for_graph = m.h() - hdr_h - g.px(6.0) - axis_h - comp_h - bottom_need;
        let block_h = avail_for_graph.clamp(g.px(88.0), (m.h() as f64 * max_graph_frac) as i32);
        let block = R::new(m.l, m.t + hdr_h + g.px(4.0), m.r, m.t + hdr_h + g.px(4.0) + block_h);
        self.draw_graph_block(g, it, block);
        g.text(R::new(m.l, block.b + g.px(2.0), m.l + g.px(180.0), block.b + axis_h), "60 seconds", Font::Small, th.dim);
        g.text_right(R::new(m.r - g.px(100.0), block.b + g.px(2.0), m.r, block.b + axis_h), "0", Font::Small, th.dim);
        let mut y = block.b + axis_h + g.px(4.0);
        if let Some((cap, segs)) = &it.composition {
            g.text(R::new(m.l, y, m.r, y + g.px(16.0)), cap, Font::Small, th.dim);
            let br = R::new(m.l, y + g.px(18.0), m.r, y + g.px(18.0) + g.px(22.0));
            g.fill(br, th.bg.lerp(th.text, 0.06));
            let mut x = br.l;
            for (frac, color) in segs {
                let w = (br.w() as f32 * frac) as i32;
                g.fill(R::new(x, br.t, (x + w).min(br.r), br.b), *color);
                x += w;
            }
            g.frame(br, th.border);
            y = br.b + g.px(10.0);
        }
        let rem_h = (m.b - y).max(g.px(40.0));
        let stats_budget = if stack_facts {
            (rem_h - fact_rows * g.px(16.0) - g.px(6.0)).max(stat_rows * g.px(34.0))
        } else {
            rem_h
        };
        let eff_stat_h = (stats_budget / stat_rows).clamp(g.px(34.0), stat_row_h);
        let gap_x = g.px(10.0);
        let left_pane_w = if it.facts.is_empty() || stack_facts {
            m.w()
        } else {
            (m.w() * 52 / 100).max(g.px(160.0)).min((m.w() - g.px(170.0)).max(m.w() / 2))
        };
        let colw = ((left_pane_w - gap_x * (cols as i32 - 1).max(0)) / cols as i32).max(g.px(48.0));
        let val_font = if eff_stat_h < g.px(44.0) || colw < g.px(92.0) { Font::Semibold } else { Font::Big };
        let (mut bx, mut by) = (m.l, y);
        for (i, (label, value)) in it.stats.iter().enumerate() {
            if i > 0 && i % cols == 0 {
                bx = m.l;
                by += eff_stat_h;
            }
            let lbl_h = g.px(15.0);
            let cell_r = (bx + colw).min(m.l + left_pane_w);
            g.text(R::new(bx, by, cell_r, by + lbl_h), label, Font::Small, th.dim);
            g.text(R::new(bx, by + lbl_h, cell_r, (by + eff_stat_h - g.px(2.0)).min(m.b)), value, val_font, th.text);
            bx += colw + gap_x;
        }
        if !it.facts.is_empty() {
            let (rx, mut ry, fact_avail) = if stack_facts {
                let sy = y + stat_rows * eff_stat_h + g.px(6.0);
                (m.l, sy, (m.b - sy).max(g.px(16.0)))
            } else {
                (m.l + left_pane_w + g.px(12.0), y, rem_h)
            };
            let eff_fact_h = (fact_avail / fact_rows.max(1)).clamp(g.px(15.0), fact_row_h);
            let fw = (m.r - rx).max(g.px(80.0));
            let kw = (fw * 54 / 100).clamp(g.px(64.0), g.px(210.0));
            for (k, v) in &it.facts {
                if ry + eff_fact_h > m.b + g.px(2.0) {
                    break;
                }
                g.text(R::new(rx, ry, rx + kw, ry + eff_fact_h), k, Font::Small, th.dim);
                g.text(R::new(rx + kw + g.px(6.0), ry, m.r, ry + eff_fact_h), v, Font::Small, th.text);
                ry += eff_fact_h;
            }
        }
    }

    fn draw_graph_block(&self, g: &G, it: &PerfItem, r: R) {
        let gap = g.px(10.0);
        let one = |gs: &GraphSpec, rr: R| {
            let lab = g.px(18.0);
            g.text(R::new(rr.l, rr.t, rr.r - g.px(120.0), rr.t + lab), &gs.label, Font::Small, g.th.dim);
            g.text_right(R::new(rr.r - g.px(160.0), rr.t, rr.r, rr.t + lab), &gs.right, Font::Small, g.th.dim);
            draw_graph(g, R::new(rr.l, rr.t + lab + g.px(2.0), rr.r, rr.b), &gs.series, gs.ymax, true);
        };
        match it.layout {
            GLayout::Single => one(&it.graphs[0], r),
            GLayout::Stack => {
                let h1 = (r.h() as f32 * 0.62) as i32;
                one(&it.graphs[0], R::new(r.l, r.t, r.r, r.t + h1 - gap / 2));
                if let Some(g2) = it.graphs.get(1) {
                    one(g2, R::new(r.l, r.t + h1 + gap / 2, r.r, r.b));
                }
            }
            GLayout::Grid => {
                let n = it.graphs.len() as i32;
                let rows = (n + 1) / 2;
                let (cw, ch) = ((r.w() - gap) / 2, (r.h() - gap * (rows - 1)) / rows);
                for (i, gs) in it.graphs.iter().enumerate() {
                    let (cx, cy) = (r.l + (i as i32 % 2) * (cw + gap), r.t + (i as i32 / 2) * (ch + gap));
                    one(gs, R::new(cx, cy, cx + cw, cy + ch));
                }
            }
        }
    }

    pub fn draw_overlay(&self, g: &G, ui: &Ui, client: R) {
        let th = g.th;
        if let Some((msg, _)) = &ui.toast {
            let tw = g.text_w(msg, Font::Ui).min(client.w() - g.px(160.0)) + g.px(40.0);
            let r = R::new(client.l + (client.w() - tw) / 2 + g.px(20.0), client.b - g.px(60.0), client.l + (client.w() + tw) / 2 + g.px(20.0), client.b - g.px(20.0));
            g.round(r, g.px(6.0), Some(th.panel), Some(th.dim.lerp(th.bg, 0.5)));
            g.text_center(r.inset(g.px(8.0)), msg, Font::Ui, th.text);
        }
    }
}

fn agg_text(d: &Data, m: Metric) -> String {
    let v = match m {
        Metric::Cpu => d.cpu_total as f64,
        Metric::Mem => d.mem_pct as f64,
        Metric::Disk => d.sample.disk_active as f64,
        Metric::Net => d.sample.net_util as f64,
        Metric::Gpu => d.sample.gpu_total as f64,
    };
    format!("{v:.0}%")
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SettingId {
    Theme,
    Tray,
    Autostart,
    Etw,
    GpuTemp,
}

pub fn setting_text(id: SettingId) -> (&'static str, &'static str) {
    match id {
        SettingId::Theme => ("Theme", "Light, dark or follow the system"),
        SettingId::Tray => ("Closing the window minimizes to tray", "The program keeps collecting history and applying rules; bring the window back with Ctrl+Alt+T"),
        SettingId::Autostart => ("Start with Windows", "Straight to the tray, no window"),
        SettingId::Etw => ("Per-process network", "'Network' column in Processes (ETW). Needs administrator rights"),
        SettingId::GpuTemp => ("Always read GPU temperature", "Otherwise the temperature is read only on the GPU page. Constant polling can wake a laptop's discrete GPU"),
    }
}

pub fn setting_on(id: SettingId, d: &Data) -> bool {
    match id {
        SettingId::Theme => false,
        SettingId::Tray => d.cfg.to_tray,
        SettingId::Autostart => crate::startup::self_autostart(),
        SettingId::Etw => d.etw.is_some(),
        SettingId::GpuTemp => d.cfg.gpu_temp_always,
    }
}

pub struct GraphSpec<'a> {
    pub label: String,
    pub right: String,
    pub series: Vec<(&'a VecDeque<f32>, Rgb)>,
    pub ymax: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum GLayout {
    Single,
    Stack,
    Grid,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PerfKind {
    Cpu,
    Mem,
    Disk(usize),
    Nic(usize),
    Gpu(usize),
}

pub fn perf_kinds(d: &Data) -> Vec<PerfKind> {
    let mut v = vec![PerfKind::Cpu, PerfKind::Mem];
    v.extend((0..d.disks.len()).map(PerfKind::Disk));
    v.extend((0..d.nics.len()).map(PerfKind::Nic));
    v.extend((0..d.gpus.len()).map(PerfKind::Gpu));
    v
}

pub struct PerfItem<'a> {
    pub kind: PerfKind,
    pub title: String,
    pub line1: String,
    pub line2: String,
    pub graphs: Vec<GraphSpec<'a>>,
    pub layout: GLayout,
    pub big_title: String,
    pub device: String,
    pub stats: Vec<(String, String)>,
    pub stat_cols: usize,
    pub facts: Vec<(String, String)>,
    pub composition: Option<(String, Vec<(f32, Rgb)>)>,
}

fn gb(b: u64) -> String {
    format!("{} GB", dec(b as f64 / 1_073_741_824.0, 1))
}

fn mb_cache(b: u64) -> String {
    if b >= 1 << 20 { format!("{} MB", dec(b as f64 / 1_048_576.0, 1)) } else { format!("{} KB", b >> 10) }
}

pub fn conn_name(adapter: &str) -> &'static str {
    let l = adapter.to_lowercase();
    if l.contains("wi-fi") || l.contains("wireless") || l.contains("wlan") || l.contains("802.11") {
        "Wi-Fi"
    } else if l.contains("bluetooth") {
        "Bluetooth"
    } else if l.contains("vmware") || l.contains("virtual") || l.contains("hyper-v") || l.contains("vethernet") || l.contains("vpn") || l.contains("tap-") {
        "Virtual adapter"
    } else {
        "Ethernet"
    }
}

pub fn perf_items(d: &Data) -> Vec<PerfItem<'_>> {
    let dark = d.cfg.theme != 0 && (d.cfg.theme == 1 || crate::sys::system_dark());
    let col = |i| series_color(i, dark);
    let s = &d.series;
    let last = |q: &VecDeque<f32>| q.back().copied().unwrap_or(0.0);
    let si = &d.sysinfo;
    let speed = si.base_mhz as f32 * d.sample.cpu_perf.max(1.0) / 100.0 / 1000.0;
    let total = d.sys.total_memory();
    let used = d.sys.used_memory();
    let cpu_brand = d.sys.cpus().first().map_or(String::new(), |c| c.brand().to_string());
    let m = &d.meminfo;
    let ms = |x: f64| format!("{} MB/s", dec(x / 1_048_576.0, 1));
    let gspec = |label: &str, right: &str, q, color, ymax| GraphSpec { label: label.into(), right: right.into(), series: vec![(q, color)], ymax };
    let kinds = perf_kinds(d);
    let mut v = vec![];

    let nic_titles: Vec<String> = {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        d.nics
            .iter()
            .map(|n| {
                let c = conn_name(&n.name);
                let k = seen.entry(c).or_insert(0);
                *k += 1;
                if *k > 1 { format!("{c} {k}") } else { c.to_string() }
            })
            .collect()
    };

    for kind in kinds {
        match kind {
            PerfKind::Cpu => v.push(PerfItem {
                kind,
                title: "CPU".into(),
                line1: format!("{:.0}%  {} GHz", last(&s.cpu), dec(speed as f64, 2)),
                line2: String::new(),
                graphs: vec![gspec("% utilization", "100%", &s.cpu, col(0), 100.0)],
                layout: GLayout::Single,
                big_title: "CPU".into(),
                device: cpu_brand.clone(),
                stats: vec![
                    ("Utilization".into(), format!("{:.0}%", last(&s.cpu))),
                    ("Speed".into(), format!("{} GHz", dec(speed as f64, 2))),
                    ("Processes".into(), m.processes.to_string()),
                    ("Up time".into(), fmt_uptime(crate::perf::uptime_secs())),
                ],
                stat_cols: 2,
                facts: vec![
                    ("Base speed:".into(), format!("{} GHz", dec(si.base_mhz as f64 / 1000.0, 2))),
                    ("Cores:".into(), sysinfo::System::physical_core_count().unwrap_or(0).to_string()),
                    ("Logical processors:".into(), d.sys.cpus().len().to_string()),
                    ("Virtualization:".into(), if si.virt { "Enabled" } else { "Disabled" }.into()),
                    ("L3 cache:".into(), mb_cache(si.l3)),
                ],
                composition: None,
            }),

            PerfKind::Mem => {
                let avail = total.saturating_sub(used);
                let mh = &d.hw_mem;
                let kind_txt = mh.kind();
                let reserved = (mh.installed_mb() * 1_048_576).saturating_sub(total);
                let compressed = d.procs.iter().find(|p| p.name.eq_ignore_ascii_case("Memory Compression")).map_or(0, |p| p.mem);
                let cached = if d.sample.mem_modified + d.sample.mem_standby > 0 { d.sample.mem_modified + d.sample.mem_standby } else { m.cache };
                let mut facts: Vec<(String, String)> = vec![];
                if !mh.dimms.is_empty() {
                    facts.push(("Speed:".into(), format!("{} MT/s", mh.speed())));
                    facts.push(("Slots used:".into(), format!("{} of {}", mh.dimms.len(), mh.slots.max(mh.dimms.len() as u32))));
                    facts.push(("Form factor:".into(), mh.form().into()));
                    facts.push(("Hardware reserved:".into(), format!("{} MB", dec(reserved as f64 / 1_048_576.0, 0))));
                }
                let t = total.max(1) as f32;
                let (mut seg, mut parts): (Vec<(f32, Rgb)>, Vec<String>) = (vec![], vec![]);
                seg.push((used as f32 / t, col(1)));
                parts.push(format!("In use {}", gb(used)));
                if d.sample.mem_modified > 0 {
                    seg.push((d.sample.mem_modified as f32 / t, col(1).lerp(Rgb(255, 255, 255), 0.25)));
                    parts.push(format!("Modified {}", gb(d.sample.mem_modified)));
                }
                if d.sample.mem_standby > 0 {
                    seg.push(((d.sample.mem_standby.min(avail)) as f32 / t, col(1).lerp(Rgb(128, 128, 128), 0.6)));
                    parts.push(format!("Standby {}", gb(d.sample.mem_standby)));
                }
                if d.sample.mem_free > 0 {
                    parts.push(format!("Free {}", gb(d.sample.mem_free)));
                }
                v.push(PerfItem {
                    kind,
                    title: "Memory".into(),
                    line1: format!("{}/{} ({:.0}%)", dec(used as f64 / 1_073_741_824.0, 1), gb(total), d.mem_pct),
                    line2: String::new(),
                    graphs: vec![gspec("Memory usage", &gb(total), &s.mem, col(1), 100.0)],
                    layout: GLayout::Single,
                    big_title: "Memory".into(),
                    device: if kind_txt.is_empty() { gb(total) } else { format!("{} {}", gb(total), kind_txt) },
                    stats: vec![
                        (if compressed > 0 { "In use (compressed)" } else { "In use" }.into(), if compressed > 0 { format!("{} ({})", gb(used), gb(compressed)) } else { gb(used) }),
                        ("Available".into(), gb(avail)),
                        ("Committed".into(), format!("{}/{}", dec(m.commit_total as f64 / 1_073_741_824.0, 1), gb(m.commit_limit))),
                        ("Cached".into(), gb(cached)),
                    ],
                    stat_cols: 2,
                    facts,
                    composition: Some((format!("Memory composition: {}", parts.join(" · ")), seg)),
                });
            }

            PerfKind::Disk(i) => {
                let dk = &d.disks[i];
                let info = &dk.info;
                let model = info.model.replace('_', " ");
                let kind_txt = match (info.kind.as_str(), info.bus.as_str()) {
                    ("", "") => "—".to_string(),
                    (k, "") => k.to_string(),
                    ("", b) => b.to_string(),
                    (k, b) => format!("{k} ({b})"),
                };
                let title = format!("Disk {} ({})", dk.number, dk.letters.trim());
                let xfer_max = dk.s_rd.iter().chain(dk.s_wr.iter()).cloned().fold(1.0f32, f32::max) * 1.15;
                let system = dk.letters.to_uppercase().contains("C:");
                v.push(PerfItem {
                    kind,
                    title: title.clone(),
                    line1: if info.kind.is_empty() { "Local disk".into() } else { kind_txt.clone() },
                    line2: format!("{:.0}%", dk.active),
                    graphs: vec![
                        gspec("Active time", "100%", &dk.s_active, col(2), 100.0),
                        GraphSpec { label: "Transfer rate".into(), right: format!("{} MB/s", dec(xfer_max as f64, 0)), series: vec![(&dk.s_rd, col(2)), (&dk.s_wr, col(2).lerp(Rgb(255, 255, 255), 0.45))], ymax: xfer_max },
                    ],
                    layout: GLayout::Stack,
                    big_title: title,
                    device: model,
                    stats: vec![
                        ("Active time".into(), format!("{:.0}%", dk.active)),
                        ("Average response time".into(), format!("{} ms", dec(dk.ms as f64, 1))),
                        ("Read speed".into(), ms(dk.rd)),
                        ("Write speed".into(), ms(dk.wr)),
                    ],
                    stat_cols: 2,
                    facts: vec![
                        ("Capacity:".into(), gb(info.capacity.max(dk.formatted))),
                        ("Formatted:".into(), gb(dk.formatted)),
                        ("System disk:".into(), if system { "Yes" } else { "No" }.into()),
                        ("Type:".into(), kind_txt),
                    ],
                    composition: None,
                });
            }

            PerfKind::Nic(i) => {
                let n = &d.nics[i];
                let title = nic_titles[i].clone();
                let (rx, tx) = (last(&n.s_rx), last(&n.s_tx));
                let ymax = n.s_rx.iter().chain(n.s_tx.iter()).cloned().fold(1000.0f32, f32::max) * 1.15;
                let conn = conn_name(&n.name);
                v.push(PerfItem {
                    kind,
                    title: title.clone(),
                    line1: conn.into(),
                    line2: format!("S: {} R: {} Kbps", tx as i64, rx as i64),
                    graphs: vec![GraphSpec { label: "Throughput".into(), right: format!("{} Kbps", ymax as i64), series: vec![(&n.s_rx, col(3)), (&n.s_tx, col(3).lerp(Rgb(255, 255, 255), 0.4))], ymax }],
                    layout: GLayout::Single,
                    big_title: title,
                    device: n.name.clone(),
                    stats: vec![("Send".into(), format!("{} Kbps", tx as i64)), ("Receive".into(), format!("{} Kbps", rx as i64))],
                    stat_cols: 2,
                    facts: vec![("Adapter name:".into(), n.name.clone()), ("Connection type:".into(), conn.into()), ("Link speed:".into(), format!("{:.0} Mbps", n.bw / 1e6))],
                    composition: None,
                });
            }

            PerfKind::Gpu(i) => {
                let g = &d.gpus[i];
                let a = &g.adapter;
                let vram = a.dedicated.max(g.reg.vram);
                let shared_total = a.shared;
                let temp = g.temp.map(|t| format!("{} °C", dec(t as f64, 0)));
                let name = a.name.clone();
                let title = format!("GPU {i}");
                let vram_mb = (vram >> 20).max(1) as f32;
                v.push(PerfItem {
                    kind,
                    title: title.clone(),
                    line1: name.clone(),
                    line2: match &temp {
                        Some(t) => format!("{:.0}% ({t})", g.util),
                        None => format!("{:.0}%", g.util),
                    },
                    graphs: vec![
                        gspec("3D", "100%", &g.series.g3d, col(4), 100.0),
                        gspec("Video encode", "100%", &g.series.genc, col(4), 100.0),
                        gspec("Video decode", "100%", &g.series.gdec, col(4), 100.0),
                        gspec("Dedicated GPU memory usage", &gb(vram), &g.series.ded, col(4), vram_mb),
                    ],
                    layout: GLayout::Grid,
                    big_title: title,
                    device: name.clone(),
                    stats: vec![
                        ("Utilization".into(), format!("{:.0}%", g.util)),
                        ("Dedicated GPU memory".into(), format!("{}/{}", dec(g.ded_used as f64 / 1_073_741_824.0, 1), gb(vram))),
                        ("Shared GPU memory".into(), format!("{}/{}", dec(g.shr_used as f64 / 1_073_741_824.0, 1), gb(shared_total))),
                        ("GPU temperature".into(), temp.clone().unwrap_or_else(|| "—".into())),
                    ],
                    stat_cols: 2,
                    facts: {
                        let dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
                        vec![
                            ("Driver version:".into(), dash(&g.reg.driver_version)),
                            ("DirectX version:".into(), dash(&a.dx)),
                            ("Physical location:".into(), dash(&a.location)),
                        ]
                    },
                    composition: None,
                });
            }
        }
    }
    v
}

pub fn ru_date(s: &str) -> String {
    let p: Vec<&str> = s.split(['-', '/']).collect();
    if let [m, d, y] = p[..] { format!("{d}.{m}.{y}") } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nic(name: &str) -> NicState {
        NicState { name: name.into(), rx: 0.0, tx: 0.0, bw: 1e9, s_rx: Default::default(), s_tx: Default::default() }
    }

    fn disk(number: u32, letters: &str) -> DiskState {
        DiskState {
            number,
            letters: letters.into(),
            info: crate::hw::DiskInfo { number, model: "TEST_DISK".into(), kind: "HDD".into(), bus: "SATA".into(), capacity: 2 << 40, formatted: 0 },
            formatted: 1 << 40,
            active: 0.0,
            rd: 0.0,
            wr: 0.0,
            ms: 0.0,
            s_active: Default::default(),
            s_rd: Default::default(),
            s_wr: Default::default(),
        }
    }

    fn gpu(name: &str, vendor: u32) -> GpuState {
        GpuState {
            adapter: crate::gpu::Adapter { name: name.into(), vendor, luid: (vendor, 0), dedicated: 4 << 30, shared: 8 << 30, dx: "DirectX 12 (FL 12_1)".into(), location: "PCI-шина 0, устройство 2, функция 0".into() },
            reg: Default::default(),
            util: 0.0,
            ded_used: 0,
            shr_used: 0,
            temp: Some(55.0),
            series: Default::default(),
        }
    }

    #[test]
    fn several_disks_adapters_and_gpus_get_their_own_items() {
        let mut d = Data::new(crate::store::Settings::default());
        d.disks = vec![disk(0, "C:"), disk(1, "D: E:")];
        d.nics = vec![nic("Intel Wi-Fi 6 AX200"), nic("Realtek PCIe GbE Family Controller"), nic("Qualcomm Wireless 802.11ac")];
        d.gpus = vec![gpu("Intel UHD Graphics", 0x8086), gpu("AMD Radeon RX", 0x1002)];
        let items = perf_items(&d);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["ЦП", "Память", "Диск 0 (C:)", "Диск 1 (D: E:)", "Wi-Fi", "Ethernet", "Wi-Fi 2", "GPU 0", "GPU 1"]);
        assert_eq!(perf_kinds(&d).len(), items.len());
        let g1 = items.last().unwrap();
        assert!(g1.facts.iter().any(|f| f.0.contains("DirectX") && f.1.contains("12")));
        assert!(g1.facts.iter().any(|f| f.0.contains("расположение") && f.1.contains("PCI")));
        assert!(g1.line2.contains("55 °C"));
        let d1 = &items[3];
        assert!(d1.facts.iter().any(|f| f.0.contains("Системный") && f.1 == "Нет"));
        assert_eq!(conn_name("Bluetooth Device (Personal Area Network)"), "Bluetooth");
    }

    #[test]
    fn driver_date_is_converted_to_russian_order() {
        assert_eq!(ru_date("10-19-2026"), "19.10.2026");
        assert_eq!(ru_date("garbage"), "garbage");
    }
}
