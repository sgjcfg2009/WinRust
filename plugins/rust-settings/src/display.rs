use std::{
    cell::Cell,
    ptr::{null, null_mut},
};
use windows_sys::Win32::Graphics::Gdi::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mode {
    pub w: u32,
    pub h: u32,
    pub hz: u32,
}

thread_local! {
    static SEL: Cell<usize> = const { Cell::new(0) };
    static PREV_TOPO: Cell<Option<i32>> = const { Cell::new(None) };
}

type Name = [u16; 32];

fn attached() -> Vec<(Name, bool)> {
    let mut out = vec![];
    for i in 0.. {
        let mut dd: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        dd.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        if unsafe { EnumDisplayDevicesW(null(), i, &mut dd, 0) } == 0 {
            break;
        }
        if dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0 {
            out.push((dd.DeviceName, dd.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0));
        }
    }
    out
}

fn selected() -> Option<(Name, bool)> {
    let a = attached();
    let n = a.len();
    a.into_iter().nth(SEL.get() % n.max(1))
}

fn dev() -> Option<Name> {
    selected().filter(|s| !s.1).map(|s| s.0)
}

fn dev_ptr(n: &Option<Name>) -> *const u16 {
    n.as_ref().map_or(null(), |n| n.as_ptr())
}

fn name_text(n: &[u16]) -> String {
    String::from_utf16_lossy(n).trim_end_matches('\0').to_string()
}

pub fn monitor_info() -> Option<(usize, usize, String, bool)> {
    let total = attached().len();
    let (name, primary) = selected()?;
    let label = selected_path().and_then(|p| friendly_name(&p)).unwrap_or_default();
    let fallback = || if primary && total == 1 { "Built-in display".to_string() } else { format!("Display {}", name_text(&name).trim_start_matches(r"\\.\DISPLAY")) };
    Some((SEL.get() % total.max(1) + 1, total, if label.is_empty() { fallback() } else { label }, primary))
}

pub fn monitor_names() -> Vec<String> {
    let keep = SEL.get();
    let v = (0..attached().len()).map(|i| {
        SEL.set(i);
        monitor_info().map_or(format!("{}", i + 1), |m| format!("{} · {}", i + 1, m.2))
    }).collect();
    SEL.set(keep);
    v
}

pub fn selected_index() -> usize {
    SEL.get() % attached().len().max(1)
}

pub fn select_monitor(i: usize) {
    SEL.set(i);
}

fn blank() -> DEVMODEW {
    let mut d: DEVMODEW = unsafe { std::mem::zeroed() };
    d.dmSize = size_of::<DEVMODEW>() as u16;
    d
}

fn mode_of(d: &DEVMODEW) -> Mode {
    Mode { w: d.dmPelsWidth, h: d.dmPelsHeight, hz: d.dmDisplayFrequency }
}

fn current_dm() -> Option<DEVMODEW> {
    let mut d = blank();
    (unsafe { EnumDisplaySettingsW(dev_ptr(&dev()), ENUM_CURRENT_SETTINGS, &mut d) } != 0).then_some(d)
}

pub fn current() -> Option<Mode> {
    current_dm().map(|d| mode_of(&d))
}

pub fn modes() -> Vec<Mode> {
    let (mut out, name) = (vec![], dev());
    for i in 0.. {
        let mut d = blank();
        if unsafe { EnumDisplaySettingsW(dev_ptr(&name), i, &mut d) } == 0 {
            break;
        }
        let m = mode_of(&d);
        if d.dmBitsPerPel == 32 && m.hz > 1 && !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

fn pick_hz(avail: &[u32], want: u32) -> u32 {
    if avail.contains(&want) { want } else { avail.iter().copied().max().unwrap_or(want) }
}

fn rates(modes: &[Mode], w: u32, h: u32) -> Vec<u32> {
    let mut v: Vec<u32> = modes.iter().filter(|m| (m.w, m.h) == (w, h)).map(|m| m.hz).collect();
    v.sort_unstable();
    v
}

pub fn resolutions(modes: &[Mode]) -> Vec<(u32, u32)> {
    let mut res: Vec<(u32, u32)> = modes.iter().map(|m| (m.w, m.h)).collect();
    res.sort_unstable_by_key(|&(w, h)| std::cmp::Reverse((w as u64 * h as u64, w)));
    res.dedup();
    res
}

pub fn with_res(modes: &[Mode], cur: Mode, w: u32, h: u32) -> Mode {
    Mode { w, h, hz: pick_hz(&rates(modes, w, h), cur.hz) }
}

pub fn rates_at(modes: &[Mode], cur: Mode) -> Vec<u32> {
    rates(modes, cur.w, cur.h)
}

pub fn orientation() -> Option<u32> {
    current_dm().map(|d| unsafe { d.Anonymous1.Anonymous2.dmDisplayOrientation })
}

pub fn set_orientation(o: u32, save: bool) -> bool {
    let Some(mut d) = current_dm() else { return false };
    if (unsafe { d.Anonymous1.Anonymous2.dmDisplayOrientation } ^ o) & 1 == 1 {
        (d.dmPelsWidth, d.dmPelsHeight) = (d.dmPelsHeight, d.dmPelsWidth);
    }
    d.Anonymous1.Anonymous2.dmDisplayOrientation = o % 4;
    d.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYORIENTATION | DM_POSITION;
    change(&d, save)
}

fn change(d: &DEVMODEW, save: bool) -> bool {
    let name = dev_ptr(&dev());
    let flags = if save { CDS_UPDATEREGISTRY } else { CDS_FULLSCREEN };
    unsafe {
        if ChangeDisplaySettingsExW(name, d, null_mut(), CDS_TEST, null()) == DISP_CHANGE_SUCCESSFUL {
            return ChangeDisplaySettingsExW(name, d, null_mut(), flags, null()) == DISP_CHANGE_SUCCESSFUL;
        }
        if d.dmFields & DM_DISPLAYFIXEDOUTPUT != 0 {
            let mut d2 = *d;
            d2.dmFields &= !DM_DISPLAYFIXEDOUTPUT;
            d2.Anonymous1.Anonymous2.dmDisplayFixedOutput = DMDFO_DEFAULT;
            if ChangeDisplaySettingsExW(name, &d2, null_mut(), CDS_TEST, null()) == DISP_CHANGE_SUCCESSFUL {
                return ChangeDisplaySettingsExW(name, &d2, null_mut(), flags, null()) == DISP_CHANGE_SUCCESSFUL;
            }
        }
        false
    }
}

fn force_stretch(save: bool) {
    let want = selected().map(|s| s.0);
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != 0 {
            return;
        }
        let mut paths = vec![std::mem::zeroed::<DISPLAYCONFIG_PATH_INFO>(); np as usize];
        let mut modes = vec![std::mem::zeroed::<DISPLAYCONFIG_MODE_INFO>(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), null_mut()) != 0 {
            return;
        }
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        let mut changed = false;
        for p in &mut paths {
            let matches = match want {
                Some(w) => {
                    let mut n: DISPLAYCONFIG_SOURCE_DEVICE_NAME = std::mem::zeroed();
                    n.header = header(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(), p, false);
                    DisplayConfigGetDeviceInfo(&mut n.header) == 0 && n.viewGdiDeviceName == w
                }
                None => true,
            };
            if matches && p.targetInfo.scaling != DISPLAYCONFIG_SCALING_STRETCHED {
                p.targetInfo.scaling = DISPLAYCONFIG_SCALING_STRETCHED;
                changed = true;
            }
        }
        if changed {
            let flags = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES | if save { SDC_SAVE_TO_DATABASE } else { 0 };
            SetDisplayConfig(paths.len() as u32, paths.as_ptr(), modes.len() as u32, modes.as_ptr(), flags);
        }
    }
}

fn apply_via_display_config(m: Mode, save: bool) -> bool {
    let want = selected().map(|s| s.0);
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != 0 {
            return false;
        }
        let mut paths = vec![std::mem::zeroed::<DISPLAYCONFIG_PATH_INFO>(); np as usize];
        let mut modes = vec![std::mem::zeroed::<DISPLAYCONFIG_MODE_INFO>(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), null_mut()) != 0 {
            return false;
        }
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        let mut touched = false;
        for p in &mut paths {
            let matches = match want {
                Some(w) => {
                    let mut n: DISPLAYCONFIG_SOURCE_DEVICE_NAME = std::mem::zeroed();
                    n.header = header(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(), p, false);
                    DisplayConfigGetDeviceInfo(&mut n.header) == 0 && n.viewGdiDeviceName == w
                }
                None => true,
            };
            if !matches {
                continue;
            }
            let sidx = p.sourceInfo.Anonymous.modeInfoIdx as usize;
            if let Some(sm) = modes.get_mut(sidx).filter(|mi| mi.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) {
                sm.Anonymous.sourceMode.width = m.w;
                sm.Anonymous.sourceMode.height = m.h;
                p.targetInfo.scaling = DISPLAYCONFIG_SCALING_STRETCHED;
                if m.hz > 0 {
                    p.targetInfo.refreshRate.Numerator = m.hz;
                    p.targetInfo.refreshRate.Denominator = 1;
                }
                touched = true;
            }
        }
        if !touched {
            return false;
        }
        let base_flags = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES | if save { SDC_SAVE_TO_DATABASE } else { 0 };
        if SetDisplayConfig(paths.len() as u32, paths.as_ptr(), modes.len() as u32, modes.as_ptr(), base_flags) == 0 {
            return true;
        }
        for p in &mut paths {
            if p.targetInfo.scaling == DISPLAYCONFIG_SCALING_STRETCHED {
                p.targetInfo.scaling = DISPLAYCONFIG_SCALING_ASPECTRATIOCENTEREDMAX;
            }
        }
        SetDisplayConfig(paths.len() as u32, paths.as_ptr(), modes.len() as u32, modes.as_ptr(), base_flags) == 0
    }
}

pub fn apply(m: Mode, save: bool) -> bool {
    if save {
        PREV_TOPO.set(None);
    }
    if apply_via_display_config(m, save) {
        return true;
    }
    let Some(mut d) = current_dm() else { return false };
    d.dmPelsWidth = m.w;
    d.dmPelsHeight = m.h;
    d.dmDisplayFrequency = m.hz;
    d.dmBitsPerPel = 32;
    d.Anonymous1.Anonymous2.dmDisplayFixedOutput = DMDFO_STRETCH;
    d.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_BITSPERPEL | DM_DISPLAYORIENTATION | DM_POSITION | DM_DISPLAYFIXEDOUTPUT;
    if !change(&d, save) {
        return false;
    }
    force_stretch(save);
    true
}

pub fn revert() {
    unsafe { ChangeDisplaySettingsExW(null(), null(), null_mut(), 0, null()) };
    if let Some(t) = PREV_TOPO.take() {
        set_topology(t);
    }
}

pub fn make_primary() -> bool {
    let Some((name, primary)) = selected() else { return false };
    if primary {
        return false;
    }
    let get = |n: &Name| {
        let mut d = blank();
        (unsafe { EnumDisplaySettingsW(n.as_ptr(), ENUM_CURRENT_SETTINGS, &mut d) } != 0).then_some(d)
    };
    let Some(me) = get(&name) else { return false };
    let (ox, oy) = unsafe { (me.Anonymous1.Anonymous2.dmPosition.x, me.Anonymous1.Anonymous2.dmPosition.y) };
    let mut moves = vec![];
    for (n, _) in attached() {
        let Some(mut d) = get(&n) else { return false };
        unsafe {
            d.Anonymous1.Anonymous2.dmPosition.x -= ox;
            d.Anonymous1.Anonymous2.dmPosition.y -= oy;
        }
        d.dmFields = DM_POSITION;
        let flags = CDS_UPDATEREGISTRY | CDS_NORESET | if n == name { CDS_SET_PRIMARY } else { 0 };
        if unsafe { ChangeDisplaySettingsExW(n.as_ptr(), &d, null_mut(), CDS_TEST, null()) } != DISP_CHANGE_SUCCESSFUL {
            return false;
        }
        moves.push((n, d, flags));
    }
    for (n, d, flags) in &moves {
        unsafe { ChangeDisplaySettingsExW(n.as_ptr(), d, null_mut(), *flags, null()) };
    }
    unsafe { ChangeDisplaySettingsExW(null(), null(), null_mut(), 0, null()) == DISP_CHANGE_SUCCESSFUL }
}

use windows_sys::Win32::Devices::Display::*;

const SCALES: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

fn header(ty: i32, size: usize, p: &DISPLAYCONFIG_PATH_INFO, target: bool) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    let (adapter, id) = if target { (p.targetInfo.adapterId, p.targetInfo.id) } else { (p.sourceInfo.adapterId, p.sourceInfo.id) };
    DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: ty, size: size as u32, adapterId: adapter, id }
}

fn active_paths() -> Vec<DISPLAYCONFIG_PATH_INFO> {
    paths_of(QDC_ONLY_ACTIVE_PATHS).map_or(vec![], |p| p.0)
}

fn paths_of(flags: u32) -> Option<(Vec<DISPLAYCONFIG_PATH_INFO>, i32)> {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(flags, &mut np, &mut nm) != 0 {
            return None;
        }
        let mut paths = vec![std::mem::zeroed::<DISPLAYCONFIG_PATH_INFO>(); np as usize];
        let mut modes = vec![std::mem::zeroed::<DISPLAYCONFIG_MODE_INFO>(); nm as usize];
        let mut topo: DISPLAYCONFIG_TOPOLOGY_ID = 0;
        let topo_ptr = if flags == QDC_DATABASE_CURRENT { &mut topo as *mut _ } else { null_mut() };
        if QueryDisplayConfig(flags, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), topo_ptr) != 0 {
            return None;
        }
        paths.truncate(np as usize);
        Some((paths, topo))
    }
}

fn selected_path() -> Option<DISPLAYCONFIG_PATH_INFO> {
    let want = selected()?.0;
    active_paths().into_iter().find(|p| unsafe {
        let mut n: DISPLAYCONFIG_SOURCE_DEVICE_NAME = std::mem::zeroed();
        n.header = header(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(), p, false);
        DisplayConfigGetDeviceInfo(&mut n.header) == 0 && n.viewGdiDeviceName == want
    })
}

#[repr(C)]
struct TargetName {
    h: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    flags: u32,
    technology: i32,
    manufacturer: u16,
    product: u16,
    connector: u32,
    friendly: [u16; 64],
    path: [u16; 128],
}

fn friendly_name(p: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut t: TargetName = unsafe { std::mem::zeroed() };
    t.h = header(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, size_of::<TargetName>(), p, true);
    (unsafe { DisplayConfigGetDeviceInfo(&mut t.h) } == 0).then(|| name_text(&t.friendly)).filter(|s| !s.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Topo {
    Extend,
    Duplicate,
    PcOnly,
    SecondOnly,
}

impl Topo {
    pub const ALL: [Topo; 4] = [Topo::Extend, Topo::Duplicate, Topo::PcOnly, Topo::SecondOnly];
    fn id(self) -> i32 {
        match self {
            Topo::PcOnly => 1,
            Topo::Duplicate => 2,
            Topo::Extend => 4,
            Topo::SecondOnly => 8,
        }
    }
    fn of(id: i32) -> Option<Topo> {
        Topo::ALL.into_iter().find(|t| t.id() == id)
    }
}

pub fn topology() -> Option<Topo> {
    Topo::of(paths_of(QDC_DATABASE_CURRENT)?.1)
}

pub fn connected() -> usize {
    let Some((paths, _)) = paths_of(QDC_ALL_PATHS) else { return 0 };
    let mut ids: Vec<(u32, i32, u32)> = paths.iter().filter(|p| p.targetInfo.targetAvailable != 0).map(|p| (p.targetInfo.adapterId.LowPart, p.targetInfo.adapterId.HighPart, p.targetInfo.id)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

fn set_topology(id: i32) -> bool {
    unsafe { SetDisplayConfig(0, null(), 0, null(), SDC_APPLY | id as u32) == 0 }
}

pub fn set_topology_to(t: Topo) -> bool {
    let (Some(cur), true) = (topology(), connected() > 1) else { return false };
    if cur == t {
        return false;
    }
    let ok = set_topology(t.id());
    if ok {
        PREV_TOPO.set(Some(cur.id()));
    }
    ok
}

#[repr(C)]
struct DpiGet {
    h: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    min: i32,
    cur: i32,
    max: i32,
}

#[repr(C)]
struct DpiSet {
    h: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    rel: i32,
}

fn dpi_steps(p: &DISPLAYCONFIG_PATH_INFO) -> Option<(i32, i32, i32)> {
    let mut g = DpiGet { h: header(-3, size_of::<DpiGet>(), p, false), min: 0, cur: 0, max: 0 };
    (unsafe { DisplayConfigGetDeviceInfo(&mut g.h) } == 0).then_some((g.min, g.cur, g.max))
}

fn percent(min: i32, rel: i32) -> Option<u32> {
    SCALES.get(usize::try_from(rel - min).ok()?).copied()
}

pub fn scale() -> Option<u32> {
    let (min, cur, _) = dpi_steps(&selected_path()?)?;
    percent(min, cur)
}

pub fn scales() -> Option<(Vec<u32>, usize)> {
    let (min, cur, max) = dpi_steps(&selected_path()?)?;
    Some(((min..=max).filter_map(|r| percent(min, r)).collect(), (cur - min).max(0) as usize))
}

pub fn set_scale_index(i: usize) -> bool {
    let Some(p) = selected_path() else { return false };
    let Some((min, _, max)) = dpi_steps(&p) else { return false };
    let mut s = DpiSet { h: header(-4, size_of::<DpiSet>(), &p, false), rel: (min + i as i32).min(max) };
    unsafe { DisplayConfigSetDeviceInfo(&mut s.h) == 0 }
}

#[repr(C)]
struct ColorGet {
    h: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    value: u32,
    encoding: i32,
    bits: u32,
}

#[repr(C)]
struct ColorSet {
    h: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    enable: u32,
}

pub fn hdr() -> Option<bool> {
    let p = selected_path()?;
    let mut g = ColorGet { h: header(DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, size_of::<ColorGet>(), &p, true), value: 0, encoding: 0, bits: 0 };
    if unsafe { DisplayConfigGetDeviceInfo(&mut g.h) } != 0 {
        return None;
    }
    hdr_state(g.value)
}

fn hdr_state(value: u32) -> Option<bool> {
    (value & 1 != 0 && value & 8 == 0).then_some(value & 2 != 0)
}

pub fn set_hdr(on: bool) -> bool {
    let Some(p) = selected_path() else { return false };
    let mut s = ColorSet { h: header(DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE, size_of::<ColorSet>(), &p, true), enable: on as u32 };
    unsafe { DisplayConfigSetDeviceInfo(&mut s.h) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_steps_are_relative_to_the_recommended_one() {
        assert_eq!(percent(-1, -1), Some(100));
        assert_eq!(percent(-1, 0), Some(125));
        assert_eq!(percent(-1, 3), Some(200));
        assert_eq!(percent(-1, 99), None);
    }

    #[test]
    fn screen_modes_map_to_windows_ids() {
        for t in Topo::ALL {
            assert_eq!(Topo::of(t.id()), Some(t));
        }
        assert_eq!(Topo::of(0), None);
    }

    #[test]
    fn hdr_needs_support_and_no_force_disable() {
        assert_eq!(hdr_state(0), None);
        assert_eq!(hdr_state(1), Some(false));
        assert_eq!(hdr_state(3), Some(true));
        assert_eq!(hdr_state(1 | 8), None);
    }

    #[test]
    #[ignore = "reads this machine's display; run with --ignored --nocapture"]
    fn live_primary_display() {
        println!("scale {:?}, hdr {:?}, monitor {:?}, topology {:?}, connected {}", scale(), hdr(), monitor_info(), topology(), connected());
    }

    const fn m(w: u32, h: u32, hz: u32) -> Mode {
        Mode { w, h, hz }
    }
    const ALL: [Mode; 5] = [m(1920, 1080, 60), m(1920, 1080, 144), m(1280, 720, 60), m(2560, 1440, 60), m(2560, 1440, 120)];

    #[test]
    fn resolutions_largest_first_keeping_the_rate() {
        assert_eq!(resolutions(&ALL), vec![(2560, 1440), (1920, 1080), (1280, 720)]);
        assert_eq!(with_res(&ALL, m(2560, 1440, 120), 1920, 1080), m(1920, 1080, 144));
        assert_eq!(with_res(&ALL, m(1920, 1080, 60), 2560, 1440), m(2560, 1440, 60));
        assert!(resolutions(&[]).is_empty());
    }

    #[test]
    fn refresh_rates_of_the_current_resolution() {
        assert_eq!(rates_at(&ALL, m(1920, 1080, 60)), vec![60, 144]);
        assert_eq!(rates_at(&ALL, m(1280, 720, 60)), vec![60]);
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mon {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub primary: bool,
}

pub fn layout() -> Vec<Mon> {
    attached()
        .into_iter()
        .filter_map(|(n, primary)| {
            let mut d = blank();
            (unsafe { EnumDisplaySettingsW(n.as_ptr(), ENUM_CURRENT_SETTINGS, &mut d) } != 0).then(|| {
                let p = unsafe { d.Anonymous1.Anonymous2.dmPosition };
                Mon { x: p.x, y: p.y, w: d.dmPelsWidth as i32, h: d.dmPelsHeight as i32, primary }
            })
        })
        .collect()
}

fn overlaps(a: (i32, i32, i32, i32), b: &Mon) -> bool {
    a.0 < b.x + b.w && a.0 + a.2 > b.x && a.1 < b.y + b.h && a.1 + a.3 > b.y
}

pub fn snap(x: i32, y: i32, w: i32, h: i32, others: &[Mon]) -> (i32, i32) {
    let mut best: Option<((i32, i32), i64)> = None;
    for o in others {
        let along_y = y.clamp(o.y - h + 1, o.y + o.h - 1);
        let along_x = x.clamp(o.x - w + 1, o.x + o.w - 1);
        for c in [(o.x + o.w, along_y), (o.x - w, along_y), (along_x, o.y + o.h), (along_x, o.y - h)] {
            if others.iter().any(|m| overlaps((c.0, c.1, w, h), m)) {
                continue;
            }
            let d = (c.0 - x) as i64 * (c.0 - x) as i64 + (c.1 - y) as i64 * (c.1 - y) as i64;
            if best.is_none_or(|b| d < b.1) {
                best = Some((c, d));
            }
        }
    }
    best.map_or((x, y), |b| b.0)
}

pub fn move_monitor(i: usize, x: i32, y: i32) -> bool {
    let att = attached();
    let mut mons = layout();
    if i >= mons.len() || mons.len() != att.len() {
        return false;
    }
    (mons[i].x, mons[i].y) = (x, y);
    let Some(p) = mons.iter().find(|m| m.primary).copied() else { return false };
    let mut moves = vec![];
    for ((n, _), m) in att.iter().zip(&mons) {
        let mut d = blank();
        if unsafe { EnumDisplaySettingsW(n.as_ptr(), ENUM_CURRENT_SETTINGS, &mut d) } == 0 {
            return false;
        }
        d.Anonymous1.Anonymous2.dmPosition.x = m.x - p.x;
        d.Anonymous1.Anonymous2.dmPosition.y = m.y - p.y;
        d.dmFields = DM_POSITION;
        if unsafe { ChangeDisplaySettingsExW(n.as_ptr(), &d, null_mut(), CDS_TEST, null()) } != DISP_CHANGE_SUCCESSFUL {
            return false;
        }
        moves.push((*n, d, m.primary));
    }
    for (n, d, primary) in &moves {
        unsafe { ChangeDisplaySettingsExW(n.as_ptr(), d, null_mut(), CDS_UPDATEREGISTRY | CDS_NORESET | if *primary { CDS_SET_PRIMARY } else { 0 }, null()) };
    }
    unsafe { ChangeDisplaySettingsExW(null(), null(), null_mut(), 0, null()) == DISP_CHANGE_SUCCESSFUL }
}

#[cfg(test)]
mod arrange_tests {
    use super::*;

    fn m(x: i32, y: i32, w: i32, h: i32) -> Mon {
        Mon { x, y, w, h, primary: false }
    }

    #[test]
    fn a_dropped_display_lands_flush_against_the_nearest_edge() {
        let main = [m(0, 0, 1920, 1080)];
        assert_eq!(snap(1950, 100, 1280, 720, &main), (1920, 100));
        assert_eq!(snap(-1300, -50, 1280, 720, &main), (-1280, -50));
        assert_eq!(snap(300, 1200, 1280, 720, &main), (300, 1080));
        assert_eq!(snap(100, 5000, 1280, 720, &main), (100, 1080));
        let r = snap(1000, 400, 800, 600, &main);
        assert!(!overlaps((r.0, r.1, 800, 600), &main[0]));
    }
}
