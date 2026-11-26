use super::*;
use windows_sys::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipAddPathArc, GdipAddPathLine, GdipClosePathFigure, GdipCreatePath, GdipDeletePath, GdipDrawArc, GdipDrawLine, GdipDrawPath, GdipFillEllipse, GdipSetPenEndCap, GdipSetPenLineJoin, GdipSetPenStartCap, GpPath, LineCapRound, LineJoinRound,
};

pub(crate) unsafe fn endpoint() -> Option<IAudioEndpointVolume> {
    EP.with_borrow_mut(|e| {
        if e.is_none() {
            *e = (|| -> windows::core::Result<IAudioEndpointVolume> {
                let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                en.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)
            })()
            .ok();
        }
        e.clone()
    })
}

pub(crate) unsafe fn read_volume() {
    let v = endpoint().and_then(|e| Some(((e.GetMasterVolumeLevelScalar().ok()? * 100.0).round() as i32, e.GetMute().ok()?.as_bool())));
    if v.is_none() {
        EP.set(None);
    }
    VOL.set(v.unwrap_or((-1, false)));
}

pub(crate) unsafe fn set_volume(v: i32) {
    if let Some(e) = endpoint() {
        let _ = e.SetMasterVolumeLevelScalar(v.clamp(0, 100) as f32 / 100.0, null());
        let _ = e.SetMute(false, null());
        read_volume();
    }
}

pub(crate) unsafe fn add_volume(delta: i32) {
    if let Some(e) = endpoint() {
        let v = (VOL.get().0.max(0) + delta).clamp(0, 100);
        let _ = e.SetMasterVolumeLevelScalar(v as f32 / 100.0, null());
        let _ = e.SetMute(false, null());
        read_volume();
    }
}

pub(crate) unsafe fn toggle_mute() {
    if let Some(e) = endpoint() {
        let _ = e.SetMute(!VOL.get().1, null());
        read_volume();
    }
}

thread_local! {
    static WLAN: Cell<usize> = const { Cell::new(0) };
}

pub(crate) unsafe fn read_wifi() {
    let mut h: HANDLE = WLAN.get() as HANDLE;
    let mut q = -1;
    if h.is_null() {
        let mut ver = 0u32;
        if WlanOpenHandle(2, null(), &mut ver, &mut h) != 0 {
            h = null_mut();
        }
        WLAN.set(h as usize);
    }
    if !h.is_null() {
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
        if WlanEnumInterfaces(h, null(), &mut list) != 0 {
            WlanCloseHandle(h, null());
            WLAN.set(0);
        } else {
            let l = &*list;
            for i in 0..l.dwNumberOfItems as usize {
                let ifc = &*l.InterfaceInfo.as_ptr().add(i);
                let (mut size, mut data) = (0u32, null_mut());
                if ifc.isState == wlan_interface_state_connected && WlanQueryInterface(h, &ifc.InterfaceGuid, wlan_intf_opcode_current_connection, null(), &mut size, &mut data, null_mut()) == 0 {
                    q = (*(data as *const WLAN_CONNECTION_ATTRIBUTES)).wlanAssociationAttributes.wlanSignalQuality as i32;
                    WlanFreeMemory(data);
                }
            }
            WlanFreeMemory(list as *const _);
        }
    }
    WIFI.set(q);
    use windows_sys::Win32::{NetworkManagement::IpHelper::GetNetworkConnectivityHint, Networking::WinSock::{NL_NETWORK_CONNECTIVITY_HINT, NetworkConnectivityLevelHintConstrainedInternetAccess, NetworkConnectivityLevelHintInternetAccess}};
    let mut hint: NL_NETWORK_CONNECTIVITY_HINT = zeroed();
    INET.set(GetNetworkConnectivityHint(&mut hint) != 0 || {
        let lvl = hint.ConnectivityLevel;
        lvl == NetworkConnectivityLevelHintInternetAccess || lvl == NetworkConnectivityLevelHintConstrainedInternetAccess
    });
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Link {
    Wifi(i32),
    NoInternet(i32),
    Cable,
    Offline,
}

pub(crate) fn net() -> Link {
    match (WIFI.get(), INET.get()) {
        (q, true) if q >= 0 => Link::Wifi(q),
        (q, false) if q >= 0 => Link::NoInternet(q),
        (_, true) => Link::Cable,
        _ => Link::Offline,
    }
}

pub(crate) fn arcs_lit(q: i32) -> i32 {
    if q >= 67 { 3 } else if q >= 34 { 2 } else { 1 }
}

#[inline]
fn argb(c: u32) -> u32 {
    0xFF00_0000 | (c & 255) << 16 | (c & 0xFF00) | (c >> 16 & 255)
}

thread_local! {
    static ROUND_PEN: Cell<usize> = const { Cell::new(0) };
}

unsafe fn round_pen(c: u32, w: f32) -> *mut GpPen {
    use windows_sys::Win32::Graphics::GdiPlus::{GdipSetPenColor, GdipSetPenWidth};
    let mut p = ROUND_PEN.get() as *mut GpPen;
    if p.is_null() {
        GdipCreatePen1(argb(c), w, UnitPixel, &mut p);
        if !p.is_null() {
            GdipSetPenStartCap(p, LineCapRound);
            GdipSetPenEndCap(p, LineCapRound);
            GdipSetPenLineJoin(p, LineJoinRound);
            ROUND_PEN.set(p as usize);
        }
    } else {
        GdipSetPenColor(p, argb(c));
        GdipSetPenWidth(p, w);
    }
    p
}

pub(crate) unsafe fn wifi_glyph(dc: HDC, cx: i32, cy: i32, s: i32, lit: i32, on: u32, off: u32, mark: u8) {
    let fg = FRAME_G.get();
    let owned = fg.is_null();
    let g = if !owned {
        fg
    } else {
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
            return;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        g
    };
    let (sf, w) = (s as f32, (s as f32 * 0.12).max(1.6));
    let (bx, by) = (cx as f32, cy as f32 + sf * 0.36);
    let p = round_pen(on, w);
    if !p.is_null() {
        for i in 0..3 {
            let r = sf * (0.30 + 0.25 * i as f32);
            windows_sys::Win32::Graphics::GdiPlus::GdipSetPenColor(p, argb(if i < lit { on } else { off }));
            GdipDrawArc(g, p, bx - r, by - r, 2.0 * r, 2.0 * r, 225.0, 90.0);
        }
    }
    let br = cached_gdip_brush(argb(if lit > 0 { on } else { off }));
    if !br.is_null() {
        GdipFillEllipse(g, br as *mut GpBrush, bx - w * 0.9, by - w * 0.9, w * 1.8, w * 1.8);
    }
    let (mx, my, mr) = (bx + sf * 0.42, by - sf * 0.08, sf * 0.26);
    if mark > 0 {
        let bg = cached_gdip_brush(argb(pal().bg));
        if !bg.is_null() {
            GdipFillEllipse(g, bg as *mut GpBrush, mx - mr, my - mr, 2.0 * mr, 2.0 * mr);
        }
        let c = if mark == 1 { on } else { WARN };
        let p = round_pen(c, (w * 0.8).max(1.4));
        if !p.is_null() {
            let d = mr * 0.5;
            if mark == 1 {
                GdipDrawLine(g, p, mx - d, my - d, mx + d, my + d);
                GdipDrawLine(g, p, mx + d, my - d, mx - d, my + d);
            } else {
                GdipDrawLine(g, p, mx, my - d * 1.2, mx, my + d * 0.2);
                GdipDrawLine(g, p, mx, my + d * 0.95, mx, my + d * 1.0);
            }
        }
    }
    if owned {
        GdipDeleteGraphics(g);
    }
}

pub(crate) unsafe fn chevron(dc: HDC, cx: i32, cy: i32, s: i32, deg: f32, c: u32) {
    let fg = FRAME_G.get();
    let owned = fg.is_null();
    let g = if !owned {
        fg
    } else {
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
            return;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        g
    };
    let p = round_pen(c, (s as f32 * 0.32).max(1.4));
    if !p.is_null() {
        let (sn, cs) = deg.to_radians().sin_cos();
        let pt = |x: f32, y: f32| (cx as f32 + x * cs - y * sn, cy as f32 + x * sn + y * cs);
        let (sf, h) = (s as f32, s as f32 * 0.5);
        let (a, b, d) = (pt(-sf, h), pt(0.0, -h), pt(sf, h));
        GdipDrawLine(g, p, a.0, a.1, b.0, b.1);
        GdipDrawLine(g, p, b.0, b.1, d.0, d.1);
    }
    if owned {
        GdipDeleteGraphics(g);
    }
}

unsafe fn with_gp(dc: HDC, f: impl FnOnce(*mut GpGraphics)) {
    let fg = FRAME_G.get();
    let owned = fg.is_null();
    let g = if !owned {
        fg
    } else {
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
            return;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        g
    };
    f(g);
    if owned {
        GdipDeleteGraphics(g);
    }
}

pub(crate) unsafe fn cable_glyph(dc: HDC, cx: i32, cy: i32, s: i32, c: u32) {
    with_gp(dc, |g| {
        let u = (s as f32 / 16.0).max(0.85);
        let w = (1.3 * u).clamp(1.2, 1.6);
        let p = round_pen(c, w);
        if p.is_null() {
            return;
        }
        let snap = |v: f32| v.round() + 0.5;
        let (fx, fy) = (cx as f32, cy as f32);
        let (ml, mt, mr, mb) = (snap(fx - 6.8 * u), snap(fy - 4.2 * u), snap(fx + 1.4 * u), snap(fy + 2.4 * u));
        let md = (3.2 * u).min(mb - mt);
        let mut mon: *mut GpPath = null_mut();
        GdipCreatePath(FillModeAlternate, &mut mon);
        if !mon.is_null() {
            GdipAddPathLine(mon, snap(fx + 0.2 * u), mt, ml + md * 0.5, mt);
            GdipAddPathArc(mon, ml, mt, md, md, 270.0, -90.0);
            GdipAddPathArc(mon, ml, mb - md, md, md, 180.0, -90.0);
            GdipAddPathLine(mon, ml + md * 0.5, mb, mr, mb);
            GdipDrawPath(g, p, mon);
            GdipDeletePath(mon);
        }
        let sb = snap(fy + 5.0 * u);
        let (l1, l2) = (snap(fx - 3.8 * u), snap(fx - 1.2 * u));
        GdipDrawLine(g, p, l1, mb, l1, sb);
        GdipDrawLine(g, p, l2, mb, l2, sb);
        GdipDrawLine(g, p, snap(fx - 5.0 * u), sb, snap(fx + 0.2 * u), sb);

        let (pl, pt, pw, ph) = (snap(fx + 2.4 * u), snap(fy - 5.2 * u), (4.0 * u).round(), (4.8 * u).round());
        let pd = (2.6 * u).min(pw).min(ph);
        let mut plug: *mut GpPath = null_mut();
        GdipCreatePath(FillModeAlternate, &mut plug);
        if !plug.is_null() {
            GdipAddPathArc(plug, pl, pt, pd, pd, 180.0, 90.0);
            GdipAddPathArc(plug, pl + pw - pd, pt, pd, pd, 270.0, 90.0);
            GdipAddPathArc(plug, pl + pw - pd, pt + ph - pd, pd, pd, 0.0, 90.0);
            GdipAddPathArc(plug, pl, pt + ph - pd, pd, pd, 90.0, 90.0);
            GdipClosePathFigure(plug);
            GdipDrawPath(g, p, plug);
            GdipDeletePath(plug);
        }
        let bar_y = snap(pt + 1.7 * u);
        GdipDrawLine(g, p, pl + 0.3 * u, bar_y, pl + pw - 0.3 * u, bar_y);
        let px_mid = snap(pl + pw * 0.5 - 0.5);
        GdipDrawLine(g, p, px_mid, pt + ph, px_mid, sb);
    });
}

pub(crate) unsafe fn volume_glyph(dc: HDC, cx: i32, cy: i32, s: i32, vol: i32, muted: bool, c: u32) {
    with_gp(dc, |g| {
        let u = (s as f32 / 16.0).max(0.85);
        let w = (1.3 * u).clamp(1.2, 1.6);
        let p = round_pen(c, w);
        if p.is_null() {
            return;
        }
        let snap = |v: f32| v.round() + 0.5;
        let (fx, fy) = (cx as f32, snap(cy as f32));
        let (x0, x1, x2) = (snap(fx - 6.5 * u), snap(fx - 3.5 * u), snap(fx - 0.2 * u));
        let (y_in, y_out) = ((2.4 * u).round().max(2.0), (5.2 * u).round().max(4.0));
        let pts = [
            (x0, fy - y_in),
            (x1, fy - y_in),
            (x2, fy - y_out),
            (x2, fy + y_out),
            (x1, fy + y_in),
            (x0, fy + y_in),
        ];
        for i in 0..pts.len() {
            let (ax, ay) = pts[i];
            let (bx, by) = pts[(i + 1) % pts.len()];
            GdipDrawLine(g, p, ax, ay, bx, by);
        }
        if muted {
            let (mx, d) = (snap(fx + 4.0 * u), (2.2 * u).round().max(2.0));
            GdipDrawLine(g, p, mx - d, fy - d, mx + d, fy + d);
            GdipDrawLine(g, p, mx + d, fy - d, mx - d, fy + d);
        } else {
            let off = mix(pal().dim, pal().bg, 0.45);
            let waves = if vol <= 0 { 0 } else if vol < 34 { 1 } else if vol < 67 { 2 } else { 3 };
            for i in 0..3 {
                let r = (2.6 + 2.1 * i as f32) * u;
                windows_sys::Win32::Graphics::GdiPlus::GdipSetPenColor(p, argb(if i < waves { c } else { off }));
                GdipDrawArc(g, p, x2 - r * 0.35, fy - r, 2.0 * r, 2.0 * r, -48.0, 96.0);
            }
        }
    });
}

pub(crate) unsafe fn net_icon(dc: HDC, cx: i32, cy: i32, s: i32, _f_glyph: HFONT) {
    let (on, off) = (pal().text, mix(pal().dim, pal().bg, 0.45));
    match net() {
        Link::Wifi(q) => wifi_glyph(dc, cx, cy, s, arcs_lit(q), on, off, 0),
        Link::NoInternet(q) => wifi_glyph(dc, cx, cy, s, arcs_lit(q), mix(on, pal().bg, 0.3), off, 2),
        Link::Offline => wifi_glyph(dc, cx, cy, s, 0, on, mix(pal().dim, pal().bg, 0.2), 1),
        Link::Cable => cable_glyph(dc, cx, cy, s, on),
    }
}
