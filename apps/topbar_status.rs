use super::*;

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

thread_local! {
    static GLYPH_FONTS: RefCell<Vec<(i32, HFONT)>> = const { RefCell::new(Vec::new()) };
}

unsafe fn glyph_font(px: i32) -> HFONT {
    let px = px.max(8);
    GLYPH_FONTS.with_borrow_mut(|cache| {
        if let Some(&(_, f)) = cache.iter().find(|&&(s, _)| s == px) {
            return f;
        }
        winrust_common::ui::ensure_fluent_icons();
        let f = CreateFontW(-px, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(winrust_common::style::icon_face()).as_ptr());
        cache.push((px, f));
        f
    })
}

unsafe fn draw_fluent_glyph(dc: HDC, cx: i32, cy: i32, px: i32, glyph: &str, c: u32) {
    let prev = SelectObject(dc, glyph_font(px));
    let half = (px + sc(8)) / 2;
    crate::text(dc, glyph, RECT { left: cx - half, top: cy - half, right: cx + half, bottom: cy + half }, c, DT_CENTER);
    SelectObject(dc, prev);
}

pub(crate) unsafe fn wifi_glyph(dc: HDC, cx: i32, cy: i32, s: i32, lit: i32, on: u32, _off: u32, mark: u8) {
    let (glyph, col) = match mark {
        1 => ("\u{EB55}", on),
        2 => ("\u{EB55}", WARN),
        _ => (
            match lit {
                3.. => "\u{E701}",
                2 => "\u{EC3E}",
                1 => "\u{EC3D}",
                _ => "\u{EC3C}",
            },
            on,
        ),
    };
    draw_fluent_glyph(dc, cx, cy, s, glyph, col);
}

pub(crate) unsafe fn chevron(dc: HDC, cx: i32, cy: i32, s: i32, deg: f32, c: u32) {
    let norm = ((deg % 360.0) + 360.0) % 360.0;
    let glyph = if (45.0..135.0).contains(&norm) {
        "\u{E76C}"
    } else if (135.0..225.0).contains(&norm) {
        "\u{E70D}"
    } else if (225.0..315.0).contains(&norm) {
        "\u{E76B}"
    } else {
        "\u{E70E}"
    };
    draw_fluent_glyph(dc, cx, cy, (s * 2).max(sc(10)), glyph, c);
}

pub(crate) unsafe fn cable_glyph(dc: HDC, cx: i32, cy: i32, s: i32, c: u32) {
    draw_fluent_glyph(dc, cx, cy, s, "\u{E839}", c);
}

pub(crate) unsafe fn volume_glyph(dc: HDC, cx: i32, cy: i32, s: i32, vol: i32, muted: bool, c: u32) {
    let glyph = if muted {
        "\u{E74F}"
    } else if vol <= 0 {
        "\u{E992}"
    } else if vol < 34 {
        "\u{E993}"
    } else if vol < 67 {
        "\u{E994}"
    } else {
        "\u{E995}"
    };
    draw_fluent_glyph(dc, cx, cy, s, glyph, c);
}

pub(crate) unsafe fn net_icon(dc: HDC, cx: i32, cy: i32, s: i32) {
    let (on, off) = (pal().text, mix(pal().dim, pal().bg, 0.45));
    match net() {
        Link::Wifi(q) => wifi_glyph(dc, cx, cy, s, arcs_lit(q), on, off, 0),
        Link::NoInternet(q) => wifi_glyph(dc, cx, cy, s, arcs_lit(q), mix(on, pal().bg, 0.3), off, 2),
        Link::Offline => wifi_glyph(dc, cx, cy, s, 0, on, mix(pal().dim, pal().bg, 0.2), 1),
        Link::Cable => cable_glyph(dc, cx, cy, s, on),
    }
}
