#![windows_subsystem = "windows"]
#[path = "rust_settings_audio.rs"]
mod audio;
#[path = "rust_settings_bt.rs"]
mod bt;
#[path = "rust_settings_display.rs"]
mod display;
#[path = "rust_settings_ext.rs"]
mod ext;
#[path = "rust_settings_net.rs"]
mod net;
#[path = "rust_settings_power.rs"]
mod power;
#[path = "rust_settings_sys.rs"]
mod sys;
#[path = "rust_settings_wifi.rs"]
mod wifi;
#[path = "rust_settings_wu.rs"]
mod wu;
use std::{
    cell::RefCell,
    ptr::{null, null_mut},
};
use sys::wide;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    NetworkManagement::WiFi::wlan_notification_acm_connection_complete,
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
        SystemInformation::*,
    },
    UI::{HiDpi::GetDpiForSystem, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const STORAGE_SENSE: &str = r"Software\Microsoft\Windows\CurrentVersion\StorageSense\Parameters\StoragePolicy";
const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

enum Row {
    Toggle { label: String, key: &'static str, name: &'static str, on_when: u32, default: u32 },
    Info(String, String),
    Cycle(String, fn() -> String, Op),
    Slider(String, audio::Src),
    Item(String, String, Cmd),
    Arrange(u8),
    Adapter(net::Adapter, bool),
    Header(String),
    Switch(String, fn() -> Option<bool>, Op),
    Wifi(wifi::Net, String),
}

#[derive(Clone, Copy)]
enum Op {
    Res,
    Hz,
    Mute,
    MicMute,
    Orient,
    Scale,
    Hdr,
    Monitor,
    Topology,
    Primary,
    Screen,
    Sleep,
    Plan,
    Radio,
    Hidden,
    Nop,
    OutDev,
    InDev,
    Night,
    Proxy,
    Pac,
    Bt,
    Air,
}

#[derive(Clone)]
enum Cmd {
    Vpn(ext::Vpn),
    CleanTemp,
    EmptyBin,
    Uninstall(String),
    BtPair(String),
    BtUnpair(String),
    BtScan,
    Check,
    Install,
    Wallpaper,
    SyncClock,
}

const CONFIRM_SECS: u32 = 15;
const WM_MOUSELEAVE: u32 = 0x02A3;

struct Page {
    title: String,
    rows: Vec<Row>,
}

#[derive(Default)]
struct App {
    pages: Vec<Page>,
    page: usize,
    dpi: i32,
    font_dpi: i32,
    fonts: [isize; 6],
    mono: bool,
    pending: u32,
    drag: Option<audio::Src>,
    scroll: i32,
    hover: Option<usize>,
    prompt: Option<Prompt>,
    edit: isize,
    brush: isize,
    brush_col: u32,
    mem_dc: isize,
    mem_bmp: isize,
    mem_old_bmp: isize,
    mem_w: i32,
    mem_h: i32,
    tracking: bool,
    msg: Option<String>,
    connecting: Option<(String, String, bool)>,
    ticks: u32,
    pick: Option<Pick>,
    arr: Option<ArrDrag>,
    layout: Vec<display::Mon>,
    armed: Option<String>,
    apps: Option<Vec<ext::App>>,
}

struct ArrDrag {
    idx: usize,
    grab: (i32, i32),
    cur: (i32, i32),
    moved: bool,
}

struct Pick {
    op: Op,
    row: usize,
    items: Vec<String>,
    cur: Option<usize>,
    hover: Option<usize>,
    top: usize,
    t0: std::time::Instant,
}

const PICK_ROW: i32 = 36;
const PICK_MAX: usize = 8;

enum Prompt {
    Ssid,
    Proxy(bool),
    EapUser(wifi::Net),
    EapPass(wifi::Net, String),
    Key(wifi::Net, bool),
}
thread_local! { static APP: RefCell<App> = RefCell::new(App::default()) }

fn is_on(val: Option<u32>, on_when: u32, default: u32) -> bool {
    val.unwrap_or(default) == on_when
}

fn row_at(y: i32, top: i32, h: i32, n: usize) -> Option<usize> {
    if y < top {
        return None;
    }
    let i = ((y - top) / h) as usize;
    (i < n).then_some(i)
}

fn os_name(product: &str, build: u32) -> String {
    if build >= 22000 { product.replace("Windows 10", "Windows 11") } else { product.to_string() }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

const PAGE_KEYS: [&[&str]; 9] = [&["display"], &["sound"], &["power", "battery", "storage"], &["network", "wifi", "ethernet"], &["bluetooth", "airplane"], &["apps"], &["update"], &["personal", "color", "theme"], &["about", "dateandtime", "time"]];

fn page_for(arg: &str) -> usize {
    let a = arg.to_ascii_lowercase();
    PAGE_KEYS.iter().position(|ks| ks.iter().any(|k| a.contains(k))).unwrap_or(0)
}

fn res_text() -> String {
    display::current().map_or("?".into(), |m| format!("{} x {}", m.w, m.h))
}

fn hz_text() -> String {
    display::current().map_or("?".into(), |m| format!("{} Hz", m.hz))
}

fn monitor_text() -> String {
    match display::monitor_info() {
        Some((n, total, name, _)) if total > 1 => format!("{n}/{total} · {name}"),
        Some((_, _, name, _)) => name,
        None => "?".into(),
    }
}

fn topo_text() -> String {
    if display::connected() < 2 {
        return "Single display".into();
    }
    display::topology().map_or("?".into(), |t| topo_name(t).into())
}

fn topo_name(t: display::Topo) -> &'static str {
    use display::Topo::*;
    match t {
        Extend => "Extend",
        Duplicate => "Duplicate",
        PcOnly => "PC screen only",
        SecondOnly => "Second screen only",
    }
}

fn primary_text() -> String {
    match display::monitor_info() {
        Some((_, _, _, true)) => "This is the main display".into(),
        Some(_) => "Make main".into(),
        None => "?".into(),
    }
}

fn scale_text() -> String {
    display::scale().map_or("?".into(), |p| format!("{p}%"))
}

fn hdr_on() -> Option<bool> {
    display::hdr()
}

fn orient_text() -> String {
    display::orientation().and_then(|o| orient_names().get(o as usize).copied()).unwrap_or("?").into()
}

fn orient_names() -> [&'static str; 4] {
    ["Landscape", "Portrait", "Landscape (flipped)", "Portrait (flipped)"]
}

fn listed(op: Op) -> bool {
    matches!(op, Op::Res | Op::Hz | Op::Orient | Op::Scale | Op::Monitor | Op::Plan | Op::Screen | Op::Sleep | Op::OutDev | Op::InDev | Op::Night)
        || (matches!(op, Op::Topology) && display::connected() > 1)
}

fn choices(op: Op) -> Option<(Vec<String>, Option<usize>)> {
    let list = match op {
        Op::Res => {
            let (modes, c) = (display::modes(), display::current()?);
            let r = display::resolutions(&modes);
            (r.iter().map(|(w, h)| format!("{w} x {h}")).collect(), r.iter().position(|&x| x == (c.w, c.h)))
        }
        Op::Hz => {
            let (modes, c) = (display::modes(), display::current()?);
            let r = display::rates_at(&modes, c);
            (r.iter().map(|h| format!("{h} Hz")).collect(), r.iter().position(|&h| h == c.hz))
        }
        Op::Orient => (orient_names().iter().map(|s| s.to_string()).collect(), display::orientation().map(|o| o as usize)),
        Op::Scale => {
            let (v, i) = display::scales()?;
            (v.iter().map(|p| format!("{p}%")).collect(), Some(i))
        }
        Op::Monitor => (display::monitor_names(), Some(display::selected_index())),
        Op::Topology if display::connected() > 1 => {
            let cur = display::topology();
            (display::Topo::ALL.iter().map(|&t| topo_name(t).to_string()).collect(), display::Topo::ALL.iter().position(|&t| Some(t) == cur))
        }
        Op::OutDev | Op::InDev => {
            let d = audio::devices(matches!(op, Op::InDev));
            (d.iter().map(|d| d.name.clone()).collect(), d.iter().position(|d| d.default))
        }
        Op::Night => (night_labels(), ext::NIGHT_STEPS.iter().position(|&k| k == ext::night_kelvin())),
        Op::Plan => power::plans(),
        Op::Screen => power::timeouts(power::Timeout::Screen),
        Op::Sleep => power::timeouts(power::Timeout::Sleep),
        _ => return None,
    };
    (!list.0.is_empty()).then_some(list)
}

unsafe fn choose(hwnd: HWND, op: Op, i: usize) {
    let confirm = || unsafe {
        APP.with_borrow_mut(|a| a.pending = CONFIRM_SECS);
        SetTimer(hwnd, 1, 1000, None);
    };
    let (modes, cur) = (display::modes(), display::current());
    match (op, cur) {
        (Op::Res, Some(c)) => {
            if let Some(&(w, h)) = display::resolutions(&modes).get(i) {
                let m = display::with_res(&modes, c, w, h);
                if m != c && display::apply(m, false) {
                    confirm();
                }
            }
        }
        (Op::Hz, Some(c)) => {
            if let Some(&hz) = display::rates_at(&modes, c).get(i) {
                if hz != c.hz && display::apply(display::Mode { hz, ..c }, false) {
                    confirm();
                }
            }
        }
        (Op::Orient, _) if display::orientation() != Some(i as u32) && display::set_orientation(i as u32, false) => confirm(),
        (Op::Scale, _) => drop(display::set_scale_index(i)),
        (Op::Monitor, _) => display::select_monitor(i),
        (Op::Topology, _) => {
            if display::Topo::ALL.get(i).is_some_and(|&t| display::set_topology_to(t)) {
                confirm();
            }
        }
        (Op::OutDev | Op::InDev, _) => {
            if let Some(d) = audio::devices(matches!(op, Op::InDev)).get(i) {
                audio::set_default(&d.id);
            }
        }
        (Op::Night, _) => ext::set_night(ext::NIGHT_STEPS.get(i).copied().unwrap_or(0)),
        (Op::Plan, _) => power::set_plan(i),
        (Op::Screen, _) => power::set_timeout(power::Timeout::Screen, i),
        (Op::Sleep, _) => power::set_timeout(power::Timeout::Sleep, i),
        _ => {}
    }
}

unsafe fn open_pick(hwnd: HWND, op: Op, row: usize) {
    let Some((items, cur)) = choices(op) else { return };
    let top = cur.unwrap_or(0).saturating_sub(PICK_MAX / 2).min(items.len().saturating_sub(PICK_MAX));
    APP.with_borrow_mut(|a| a.pick = Some(Pick { op, row, items, cur, hover: cur, top, t0: std::time::Instant::now() }));
    unsafe { SetTimer(hwnd, 4, 15, None) };
}

fn pick_geom(a: &App, cw: i32, ch: i32) -> Option<(RECT, usize, bool)> {
    let pk = a.pick.as_ref()?;
    let p = |v: i32| (v as f64 * (a.dpi as f64 / 96.0)) as i32;
    let (cl, cr) = col(cw, a.dpi);
    let scroll = a.scroll.min(max_scroll(a.pages.get(a.page).map_or(0, |pg| pg.rows.len()), ch * 96 / a.dpi.max(1)));
    let row_top = p(ROWS_TOP + ROW_H * pk.row as i32 - scroll);
    let n = pk.items.len().min(PICK_MAX);
    let (w, h) = (p(280).min(cr - cl), p(12 + PICK_ROW * n as i32));
    let left = cr - w - p(6);
    let down = row_top + p(ROW_H) + h + p(12) <= ch || row_top - h < p(12);
    let top = if down { row_top + p(ROW_H) - p(2) } else { row_top - h + p(2) };
    Some((RECT { left, top, right: left + w, bottom: top + h }, n, down))
}

fn pick_item(a: &App, cw: i32, ch: i32, x: i32, y: i32) -> Option<usize> {
    let (r, n, _) = pick_geom(a, cw, ch)?;
    let row = a.dpi * PICK_ROW / 96;
    let pad = a.dpi * 6 / 96;
    (x >= r.left && x < r.right && y >= r.top + pad && y < r.top + pad + row * n as i32).then(|| a.pick.as_ref().unwrap().top + ((y - r.top - pad) / row) as usize)
}

fn night_labels() -> Vec<String> {
    ext::NIGHT_STEPS
        .iter()
        .map(|&k| match k {
            0 => "Off".to_string(),
            5500 => format!("Low · {} K", k),
            4800 => format!("Medium · {} K", k),
            4200 => format!("Strong · {} K", k),
            _ => format!("Maximum · {} K", k),
        })
        .collect()
}

fn out_dev_text() -> String {
    audio::default_name(false)
}

fn in_dev_text() -> String {
    audio::default_name(true)
}

fn bt_on() -> Option<bool> {
    bt::state().bt
}

fn air_on() -> Option<bool> {
    bt::state().air
}

fn screen_text() -> String {
    power::timeout_text(power::Timeout::Screen)
}

fn sleep_text() -> String {
    power::timeout_text(power::Timeout::Sleep)
}

const NET_PAGE: usize = 3;
const BT_PAGE: usize = 4;
const APPS_PAGE: usize = 5;
const WU_PAGE: usize = 6;

thread_local! { static EXPANDED: RefCell<Option<String>> = const { RefCell::new(None) } }
thread_local! { static RADIO: std::cell::Cell<Option<(bool, bool)>> = const { std::cell::Cell::new(None) } }

fn radio_on() -> Option<bool> {
    RADIO.get().filter(|r| r.1).map(|r| r.0)
}

fn sound_muted() -> Option<bool> {
    audio::muted(false)
}

fn mic_muted() -> Option<bool> {
    audio::muted(true)
}

fn bars(q: u32) -> u32 {
    match q {
        0..=19 => 1,
        20..=44 => 2,
        45..=69 => 3,
        _ => 4,
    }
}

fn wifi_note(n: &wifi::Net) -> String {
    use wifi::Sec::*;
    match (n.connected, &n.profile, n.sec) {
        (true, ..) => "Connected  ·  right click: disconnect, forget".into(),
        (_, Some(_), _) => "Saved".into(),
        (_, _, None) if n.eap.is_some() => "Enterprise (802.1X)  ·  user name and password".into(),
        (_, _, None) => "Enterprise with certificates: not supported".into(),
        (_, _, Some(Open | Owe)) => "Open network".into(),
        (_, _, Some(s)) => {
            let kind = match s {
                Wep { .. } => "WEP",
                Wpa { .. } => "WPA",
                Wpa3 => "WPA3",
                _ => "WPA2",
            };
            format!("Secured  ·  {}", kind)
        }
    }
}

fn network_rows() -> Vec<Row> {
    let mut rows = vec![];
    let adapters = net::adapters();
    if !adapters.is_empty() {
        rows.push(Row::Header("Connections".into()));
    }
    for a in adapters {
        let open = EXPANDED.with_borrow(|e| e.as_deref() == Some(a.name.as_str()));
        let details = if open { a.details() } else { vec![] };
        rows.push(Row::Adapter(a, open));
        rows.extend(details.into_iter().map(|(l, v)| Row::Info(l, v)));
    }
    let radio = wifi::radio();
    RADIO.set(radio);
    if let Some((_, hw)) = radio {
        rows.push(Row::Header("Wi-Fi".into()));
        let label = if hw { "Wi-Fi".into() } else { "Wi-Fi is off by the hardware switch".into() };
        rows.push(Row::Switch(label, radio_on, Op::Radio));
        for n in wifi::list().into_iter().take(12) {
            let note = wifi_note(&n);
            rows.push(Row::Wifi(n, note));
        }
        if radio.is_some_and(|r| r.0 && r.1) {
            rows.push(Row::Cycle("Other network…".into(), String::new, Op::Hidden));
        }
    }
    rows.push(Row::Header("VPN".into()));
    let vpns = ext::vpns();
    if vpns.is_empty() {
        rows.push(Row::Info("VPN connections".into(), "none set up".into()));
    }
    for v in vpns {
        let state = if v.on { "Connected" } else { "Disconnected" };
        rows.push(Row::Item(v.name.clone(), state.into(), Cmd::Vpn(v)));
    }
    rows.push(Row::Header("Proxy".into()));
    rows.push(toggle_row("Use a proxy server", ext::INET, "ProxyEnable", 1, 0));
    rows.push(Row::Cycle("Proxy address".into(), ext::proxy_text, Op::Proxy));
    rows.push(Row::Cycle("Setup script (PAC)".into(), ext::pac_text, Op::Pac));
    rows
}

fn toggle_row(label: &str, key: &'static str, name: &'static str, on_when: u32, default: u32) -> Row {
    Row::Toggle { label: label.into(), key, name, on_when, default }
}

fn display_rows() -> Vec<Row> {
    let head = |s: &str| Row::Header(s.into());
    let mut rows = vec![head("Displays")];
    if display::layout().len() > 1 {
        rows.extend((0..4).map(Row::Arrange));
    }
    rows.extend([
        Row::Cycle("Display".into(), monitor_text, Op::Monitor),
        Row::Cycle("Multiple displays".into(), topo_text, Op::Topology),
        Row::Cycle("Main display".into(), primary_text, Op::Primary),
        head("Selected display"),
        Row::Cycle("Resolution".into(), res_text, Op::Res),
        Row::Cycle("Refresh rate".into(), hz_text, Op::Hz),
        Row::Cycle("Scale".into(), scale_text, Op::Scale),
        Row::Cycle("Orientation".into(), orient_text, Op::Orient),
        Row::Switch("HDR".into(), hdr_on, Op::Hdr),
        head("Colour"),
        Row::Cycle("Night light".into(), ext::night_text, Op::Night),
    ]);
    rows
}

fn sound_rows() -> Vec<Row> {
    let head = |s: &str| Row::Header(s.into());
    let mut rows = vec![
        head("Output"),
        Row::Cycle("Output device".into(), out_dev_text, Op::OutDev),
        Row::Slider("Volume".into(), audio::Src::Out),
        Row::Switch("Mute".into(), sound_muted, Op::Mute),
        head("Input"),
        Row::Cycle("Input device".into(), in_dev_text, Op::InDev),
        Row::Slider("Microphone".into(), audio::Src::Mic),
        Row::Switch("Microphone muted".into(), mic_muted, Op::MicMute),
        head("Volume per app"),
    ];
    let apps = audio::apps();
    if apps.is_empty() {
        rows.push(Row::Info("Apps with sound".into(), "none right now".into()));
    }
    rows.extend(apps.into_iter().map(|(pid, name)| Row::Slider(name, audio::Src::App(pid))));
    rows
}

fn power_rows() -> Vec<Row> {
    let head = |s: &str| Row::Header(s.into());
    let bin = ext::bin_bytes();
    [
        vec![
            head("Power"),
            Row::Cycle("Battery".into(), power::battery_text, Op::Nop),
            Row::Cycle("Power plan".into(), power::plan_text, Op::Plan),
            Row::Cycle("Turn screen off after".into(), screen_text, Op::Screen),
            Row::Cycle("Sleep after".into(), sleep_text, Op::Sleep),
            head("Storage"),
            toggle_row("Storage Sense (auto cleanup)", STORAGE_SENSE, "01", 1, 0),
        ],
        power::drives().into_iter().map(|(l, free, total)| Row::Info(format!("Drive {}:", l), format!("{} free of {}", power::size(free), power::size(total)))).collect(),
        vec![
            head("Disk cleanup"),
            Row::Item("Temporary files (older than a day)".into(), ext::temp_text(), Cmd::CleanTemp),
            Row::Item("Recycle Bin".into(), if bin == 0 { "empty".into() } else { power::size(bin) }, Cmd::EmptyBin),
        ],
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn bt_rows() -> Vec<Row> {
    let s = bt::state();
    let head = |x: &str| Row::Header(x.into());
    let label = if s.bt.is_some() { "Bluetooth".into() } else { "Bluetooth: no adapter".into() };
    let mut rows = vec![head("Bluetooth"), Row::Switch(label, bt_on, Op::Bt), Row::Switch("Airplane mode".into(), air_on, Op::Air)];
    if s.bt == Some(true) {
        rows.push(head("Paired devices"));
        if s.paired.is_empty() {
            rows.push(Row::Info("Devices".into(), "none".into()));
        }
        for d in &s.paired {
            let st = if d.connected { "Connected" } else { "Paired" };
            rows.push(Row::Item(d.name.clone(), st.into(), Cmd::BtUnpair(d.id.clone())));
        }
        rows.push(head("Nearby devices"));
        rows.push(Row::Item("Find devices".into(), if s.scanning { "searching…".into() } else { String::new() }, Cmd::BtScan));
        for d in s.nearby.iter().filter(|d| !s.paired.iter().any(|p| p.id == d.id)) {
            rows.push(Row::Item(d.name.clone(), "Pair".into(), Cmd::BtPair(d.id.clone())));
        }
    }
    rows
}

fn apps_rows(list: &[ext::App]) -> Vec<Row> {
    let mut rows = vec![Row::Header(format!("Installed programs: {}", list.len()))];
    rows.extend(list.iter().take(400).map(|a| Row::Item(a.name.clone(), a.version.clone(), Cmd::Uninstall(a.uninstall.clone()))));
    rows
}

fn wu_rows() -> Vec<Row> {
    let s = wu::state();
    let status = match (&s.busy, &s.found) {
        (Some(b), _) => b.clone(),
        (_, None) => "Not checked yet".into(),
        (_, Some(f)) if f.is_empty() => "Up to date".into(),
        (_, Some(f)) => format!("Updates available: {}", f.len()),
    };
    let mut rows = vec![Row::Header("Windows Update".into()), Row::Info("Status".into(), status)];
    if !s.note.is_empty() {
        rows.push(Row::Info("Result".into(), s.note.clone()));
    }
    for title in s.found.iter().flatten().take(30) {
        rows.push(Row::Info("Update".into(), title.clone()));
    }
    if s.busy.is_none() {
        rows.push(Row::Item("Check for updates".into(), String::new(), Cmd::Check));
        if s.found.as_ref().is_some_and(|f| !f.is_empty()) {
            rows.push(Row::Item("Download and install".into(), "needs administrator rights".into(), Cmd::Install));
        }
    }
    rows
}

fn dynamic(page: usize, apps: &mut Option<Vec<ext::App>>) -> Option<Vec<Row>> {
    match page {
        0 => Some(display_rows()),
        1 => Some(sound_rows()),
        2 => Some(power_rows()),
        NET_PAGE => Some(network_rows()),
        BT_PAGE => Some(bt_rows()),
        APPS_PAGE => Some(apps_rows(apps.get_or_insert_with(ext::programs))),
        WU_PAGE => Some(wu_rows()),
        _ => None,
    }
}

fn pages() -> Vec<Page> {
    let reg = |n| sys::string(HKEY_LOCAL_MACHINE, NT, n).unwrap_or_default();
    let build: u32 = reg("CurrentBuild").parse().unwrap_or(0);
    let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    mem.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    let ram = if unsafe { GlobalMemoryStatusEx(&mut mem) } != 0 { gb(mem.ullTotalPhys) } else { "?".into() };
    let head = |s: &str| Row::Header(s.into());
    vec![
        Page { title: "Display".into(), rows: vec![] },
        Page { title: "Sound".into(), rows: vec![] },
        Page { title: "Power & storage".into(), rows: vec![] },
        Page { title: "Network".into(), rows: vec![] },
        Page { title: "Bluetooth".into(), rows: vec![] },
        Page { title: "Apps".into(), rows: vec![] },
        Page { title: "Windows Update".into(), rows: vec![] },
        Page {
            title: "Personalization".into(),
            rows: vec![
                head("Desktop background"),
                Row::Item("Choose a picture…".into(), sys::wallpaper_name(), Cmd::Wallpaper),
                head("Appearance"),
                toggle_row("Dark theme for apps", PERSONALIZE, "AppsUseLightTheme", 0, 1),
                toggle_row("Dark theme for the system", PERSONALIZE, "SystemUsesLightTheme", 0, 1),
                toggle_row("Transparency effects", PERSONALIZE, "EnableTransparency", 1, 1),
                toggle_row("Accent color on title bars", r"Software\Microsoft\Windows\DWM", "ColorPrevalence", 1, 0),
            ],
        },
        Page {
            title: "About".into(),
            rows: vec![
                head("Date & time"),
                Row::Item("Synchronize your clock".into(), "Sync now".into(), Cmd::SyncClock),
                head("Device"),
                Row::Info("Computer name".into(), std::env::var("COMPUTERNAME").unwrap_or_default()),
                Row::Info("Memory".into(), ram),
                head("Windows"),
                Row::Info("Edition".into(), os_name(&reg("ProductName"), build)),
                Row::Info("Version".into(), reg("DisplayVersion")),
                Row::Info("Build".into(), reg("CurrentBuild")),
            ],
        },
    ]
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| ((a >> s & 255) as f32 * (1.0 - t) + (b >> s & 255) as f32 * t) as u32;
    ch(0) | ch(8) << 8 | ch(16) << 16
}

struct Theme {
    bg: u32,
    side: u32,
    card: u32,
    sep: u32,
    hover: u32,
    sel: u32,
    accent: u32,
    fg: u32,
    dim: u32,
    off: u32,
    square: bool,
}

fn theme() -> Theme {
    let p = winrust_common::style::pal();
    let up = |t: f32| mix(p.bg, p.text, t);
    let card = winrust_common::style::card();
    Theme { bg: p.bg, side: up(0.03), card, sep: up(0.1), hover: mix(card, p.text, 0.045), sel: up(0.1), accent: p.accent, fg: p.text, dim: p.dim, off: up(0.2), square: winrust_common::style::style().square() }
}

use winrust_common::style::rgb;

const ICONS: [(&str, u32); 9] = [
    ("\u{E7F4}", rgb(10, 132, 255)),
    ("\u{E767}", rgb(255, 55, 95)),
    ("\u{E7E8}", rgb(48, 209, 88)),
    ("\u{E774}", rgb(50, 173, 230)),
    ("\u{E702}", rgb(0, 122, 255)),
    ("\u{E71D}", rgb(255, 159, 10)),
    ("\u{E895}", rgb(52, 199, 89)),
    ("\u{E771}", rgb(191, 90, 242)),
    ("\u{E946}", rgb(142, 142, 147)),
];

unsafe fn shape(dc: HDC, r: RECT, color: u32, d: i32) {
    use windows_sys::Win32::Graphics::GdiPlus::*;
    unsafe {
        if r.right <= r.left || r.bottom <= r.top {
            return;
        }
        if d <= 0 {
            SetDCBrushColor(dc, color);
            FillRect(dc, &r, GetStockObject(DC_BRUSH as i32) as HBRUSH);
            return;
        }
        let mut g = null_mut();
        if GdipCreateFromHDC(dc, &mut g) != Ok {
            return;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        let (x, y, w, h) = (r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32);
        let d = (d as f32).min(w).min(h);
        let mut path = null_mut();
        GdipCreatePath(FillModeAlternate, &mut path);
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
        let mut br = null_mut();
        GdipCreateSolidFill(0xFF00_0000 | (color & 255) << 16 | (color >> 8 & 255) << 8 | (color >> 16 & 255), &mut br);
        GdipFillPath(g, br as *mut GpBrush, path);
        GdipDeleteBrush(br as *mut GpBrush);
        GdipDeletePath(path);
        GdipDeleteGraphics(g);
    }
}

use winrust_common::with_wide;

unsafe fn text(dc: HDC, font: isize, s: &str, mut r: RECT, color: u32, flags: u32) {
    unsafe {
        SelectObject(dc, font as _);
        SetBkMode(dc, TRANSPARENT as _);
        SetTextColor(dc, color);
        with_wide(s, |p| DrawTextW(dc, p, -1, &mut r, flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS));
    }
}

unsafe fn text_width(dc: HDC, font: isize, s: &str) -> i32 {
    unsafe {
        SelectObject(dc, font as _);
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        with_wide(s, |p| DrawTextW(dc, p, -1, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX));
        r.right
    }
}

const SIDE: i32 = 248;
const ROW_H: i32 = 52;
const ROWS_TOP: i32 = 84;
const NAV_TOP: i32 = 14;
const NAV_H: i32 = 40;
const COL_MAX: i32 = 760;

fn side_w(cw: i32, dpi: i32) -> i32 {
    let p = |v: i32| v * dpi / 96;
    (cw * 26 / 100).clamp(p(150), p(SIDE))
}

fn col(cw: i32, dpi: i32) -> (i32, i32) {
    let p = |v: i32| v * dpi / 96;
    let side = side_w(cw, dpi);
    let avail = (cw - side).max(p(160));
    let w = (avail - p(48)).min(p(COL_MAX)).max(avail - p(16)).max(p(160));
    let l = side + ((avail - w) / 2).max(p(8));
    (l, (l + w).min(cw - p(8)))
}

fn track(cw: i32, dpi: i32) -> (i32, i32) {
    let p = |v: i32| v * dpi / 96;
    let (l, r) = col(cw, dpi);
    let tw = p(220).min((r - l) * 38 / 100).max(p(80));
    (r - p(18) - tw, r - p(18))
}

fn slider_value(x: i32, l: i32, r: i32) -> f32 {
    (((x - l) as f32 / (r - l).max(1) as f32).clamp(0.0, 1.0) * 100.0).round() / 100.0
}

struct Map {
    s: f32,
    cx: f32,
    cy: f32,
    ox: f32,
    oy: f32,
}

impl Map {
    fn dev(&self, x: i32, y: i32) -> (i32, i32) {
        ((self.ox + (x as f32 - self.cx) * self.s) as i32, (self.oy + (y as f32 - self.cy) * self.s) as i32)
    }
    fn desk(&self, x: i32, y: i32) -> (i32, i32) {
        (((x as f32 - self.ox) / self.s + self.cx) as i32, ((y as f32 - self.oy) / self.s + self.cy) as i32)
    }
}

fn arr_map(mons: &[display::Mon], bx: RECT) -> Map {
    let l = mons.iter().map(|m| m.x).min().unwrap_or(0);
    let t = mons.iter().map(|m| m.y).min().unwrap_or(0);
    let r = mons.iter().map(|m| m.x + m.w).max().unwrap_or(1);
    let b = mons.iter().map(|m| m.y + m.h).max().unwrap_or(1);
    let (w, h) = (((r - l) as f32 * 1.8).max(1.0), ((b - t) as f32 * 1.8).max(1.0));
    let s = ((bx.right - bx.left) as f32 / w).min((bx.bottom - bx.top) as f32 / h).max(0.0001);
    Map { s, cx: (l + r) as f32 / 2.0, cy: (t + b) as f32 / 2.0, ox: (bx.left + bx.right) as f32 / 2.0, oy: (bx.top + bx.bottom) as f32 / 2.0 }
}

fn arr_box(a: &App, cw: i32) -> Option<RECT> {
    let p = |v: i32| (v as f64 * (a.dpi as f64 / 96.0)) as i32;
    let i = a.pages.get(a.page)?.rows.iter().position(|r| matches!(r, Row::Arrange(0)))?;
    let (cl, cr) = col(cw, a.dpi);
    let top = p(ROWS_TOP + ROW_H * i as i32 - a.scroll);
    Some(RECT { left: cl + p(16), top: top + p(10), right: cr - p(16), bottom: top + p(ROW_H * 4) - p(10) })
}

fn max_scroll(rows: usize, ch: i32) -> i32 {
    (ROWS_TOP + ROW_H * rows as i32 + 24 - ch).max(0)
}

unsafe fn frame(hwnd: HWND) {
    let t = theme();
    let set = |attr: u32, v: u32| unsafe { windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(hwnd, attr, &v as *const u32 as _, 4) };
    set(20, !winrust_common::style::style().light() as u32);
    set(35, t.side);
    set(36, t.fg);
    set(34, t.sep);
    set(33, if t.square { 1 } else { 2 });
}

fn refresh_fonts() {
    let mono = winrust_common::style::style().mono();
    APP.with_borrow_mut(|a| {
        if a.fonts[0] != 0 && a.mono == mono && a.font_dpi == a.dpi {
            return;
        }
        let font = |h: f32, w: i32, face: &str| unsafe { CreateFontW(-((h * a.dpi as f32 / 96.0).round() as i32), 0, 0, 0, w, 0, 0, 0, 1, 0, 0, 5, 0, wide(face).as_ptr()) as isize };
        let (face, icons) = (winrust_common::style::ui_face(), winrust_common::style::icon_face());
        let new = [font(14.5, 400, face), font(28.0, 600, face), font(16.0, 400, icons), font(12.0, 400, face), font(12.5, 600, face), font(13.0, 400, icons)];
        for f in a.fonts.into_iter().filter(|&f| f != 0) {
            unsafe { DeleteObject(f as _) };
        }
        (a.fonts, a.mono, a.font_dpi) = (new, mono, a.dpi);
    });
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        refresh_fonts();
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let (cw, ch) = (rc.right, rc.bottom);
        if cw <= 0 || ch <= 0 {
            EndPaint(hwnd, &ps);
            return;
        }
        let (mem, cw, ch) = APP.with_borrow_mut(|a| {
            if a.mem_dc == 0 {
                a.mem_dc = CreateCompatibleDC(dc) as isize;
            }
            if a.mem_bmp == 0 || a.mem_w != cw || a.mem_h != ch {
                let nb = CreateCompatibleBitmap(dc, cw, ch);
                let prev = SelectObject(a.mem_dc as HDC, nb);
                if a.mem_bmp != 0 {
                    DeleteObject(prev);
                } else {
                    a.mem_old_bmp = prev as isize;
                }
                a.mem_bmp = nb as isize;
                a.mem_w = cw;
                a.mem_h = ch;
            }
            (a.mem_dc as HDC, cw, ch)
        });
        let t = theme();
        APP.with_borrow(|a| {
            let p = |v: i32| (v as f64 * (a.dpi as f64 / 96.0)) as i32;
            let rd = |v: i32| if t.square { 0 } else { p(v) };
            let inset = |r: RECT, n: i32| RECT { left: r.left + n, top: r.top + n, right: r.right - n, bottom: r.bottom - n };
            let [body, title, icon, small, section, icon_s] = a.fonts;
            let switch = |inner: RECT, r: RECT, on: Option<bool>| {
                let (w, h) = (p(42), p(24));
                let (l, tp) = (inner.right - w, r.top + (r.bottom - r.top - h) / 2);
                let lit = on == Some(true);
                let track = if lit { t.accent } else if on.is_none() { mix(t.card, t.off, 0.5) } else { t.off };
                shape(mem, RECT { left: l, top: tp, right: l + w, bottom: tp + h }, track, rd(h));
                let k = h - p(4);
                let kx = if lit { l + w - k - p(2) } else { l + p(2) };
                shape(mem, RECT { left: kx, top: tp + p(2), right: kx + k, bottom: tp + p(2) + k }, if on.is_none() { mix(t.fg, t.card, 0.5) } else { rgb(255, 255, 255) }, rd(k));
            };
            shape(mem, rc, t.bg, 0);
            let sw = side_w(cw, a.dpi);
            winrust_common::style::backdrop(mem, RECT { left: sw, ..rc }, a.dpi as f32 / 96.0);
            shape(mem, RECT { left: 0, top: 0, right: sw, bottom: ch }, t.side, 0);
            shape(mem, RECT { left: sw, top: 0, right: sw + 1, bottom: ch }, t.sep, 0);
            for (i, pg) in a.pages.iter().enumerate() {
                let r = RECT { left: p(10), top: p(NAV_TOP + NAV_H * i as i32), right: sw - p(10), bottom: p(NAV_TOP + NAV_H * (i as i32 + 1) - 2) };
                let sel = i == a.page;
                if sel {
                    shape(mem, r, t.sel, rd(8));
                }
                let (glyph, tile) = ICONS.get(i).copied().unwrap_or(("", t.dim));
                let s = p(26);
                let tr = RECT { left: r.left + p(8), top: (r.top + r.bottom - s) / 2, right: r.left + p(8) + s, bottom: (r.top + r.bottom + s) / 2 };
                if t.square {
                    text(mem, icon_s, glyph, tr, if sel { t.accent } else { t.dim }, DT_CENTER);
                } else {
                    shape(mem, tr, tile, p(14));
                    text(mem, icon_s, glyph, tr, rgb(255, 255, 255), DT_CENTER);
                }
                text(mem, body, &pg.title, RECT { left: tr.right + p(12), ..r }, if sel { t.fg } else { mix(t.dim, t.fg, 0.55) }, DT_LEFT);
            }
            let Some(pg) = a.pages.get(a.page) else { return };
            let (cl, cr) = col(cw, a.dpi);
            text(mem, title, &pg.title, RECT { left: cl + p(4), top: p(18), right: cr, bottom: p(70) }, t.fg, DT_LEFT);
            let saved = SaveDC(mem);
            IntersectClipRect(mem, sw + 1, p(ROWS_TOP - 8), cw, ch);
            let scroll = a.scroll.min(max_scroll(pg.rows.len(), ch * 96 / a.dpi.max(1)));
            let n = pg.rows.len();
            let is_head = |i: usize| matches!(pg.rows.get(i), Some(Row::Header(_)));
            for (i, row) in pg.rows.iter().enumerate() {
                let y = p(ROWS_TOP + ROW_H * i as i32 - scroll);
                let r = RECT { left: cl, top: y, right: cr, bottom: y + p(ROW_H) };
                if r.bottom < 0 || r.top > ch {
                    continue;
                }
                if let Row::Header(s) = row {
                    text(mem, section, s, RECT { left: cl + p(16), top: r.bottom - p(30), right: cr, bottom: r.bottom - p(8) }, t.dim, DT_LEFT);
                    continue;
                }
                let (first, last) = (i == 0 || is_head(i - 1), i + 1 == n || is_head(i + 1));
                let (rad, mid) = (rd(12), (r.top + r.bottom) / 2);
                shape(mem, RECT { bottom: mid + 1, ..r }, t.card, if first { rad * 2 } else { 0 });
                shape(mem, RECT { top: mid, ..r }, t.card, if last { rad * 2 } else { 0 });
                if first {
                    shape(mem, RECT { top: r.top + rad, bottom: mid + 1, ..r }, t.card, 0);
                }
                if last {
                    shape(mem, RECT { top: mid, bottom: r.bottom - rad, ..r }, t.card, 0);
                }
                if !first {
                    shape(mem, RECT { left: r.left + p(16), top: r.top, right: r.right - p(16), bottom: r.top + 1 }, t.sep, 0);
                }
                if a.hover == Some(i) {
                    shape(mem, inset(r, p(4)), t.hover, rd(9) * 2);
                }
                let inner = RECT { left: r.left + p(16), right: r.right - p(16), ..r };
                match row {
                    Row::Header(_) => {}
                    Row::Toggle { label, key, name, on_when, default } => {
                        text(mem, body, label, inner, t.fg, DT_LEFT);
                        switch(inner, r, Some(is_on(sys::dword(HKEY_CURRENT_USER, key, name), *on_when, *default)));
                    }
                    Row::Switch(label, value, _) => {
                        let v = value();
                        text(mem, body, label, inner, if v.is_some() { t.fg } else { t.dim }, DT_LEFT);
                        switch(inner, r, v);
                    }
                    Row::Info(label, value) => {
                        let lw = text_width(mem, body, label).min((inner.right - inner.left) / 2);
                        text(mem, body, label, RECT { right: inner.left + lw, ..inner }, t.fg, DT_LEFT);
                        text(mem, body, value, RECT { left: inner.left + lw + p(16), ..inner }, t.dim, DT_RIGHT);
                    }
                    Row::Cycle(label, value, op) => {
                        text(mem, body, label, inner, if matches!(op, Op::Hidden) { t.accent } else { t.fg }, DT_LEFT);
                        let chev = !matches!(op, Op::Nop | Op::Hidden) && (listed(*op) || !matches!(op, Op::Topology));
                        let vr = RECT { right: inner.right - if chev { p(22) } else { 0 }, ..inner };
                        let open = a.pick.as_ref().is_some_and(|pk| pk.row == i);
                        text(mem, body, &value(), vr, if open { t.accent } else { t.dim }, DT_RIGHT);
                        if chev {
                            let g = if !listed(*op) { "\u{E76C}" } else if open { "\u{E70E}" } else { "\u{E70D}" };
                            text(mem, icon_s, g, inner, if open { t.accent } else { t.dim }, DT_RIGHT);
                        }
                    }
                    Row::Adapter(ad, open) => {
                        let lit = ad.up && ad.internet != Some(false);
                        let glyph = if ad.wifi { "\u{E701}" } else { "\u{E839}" };
                        text(mem, icon, glyph, RECT { right: inner.left + p(22), ..inner }, if lit { t.accent } else { t.dim }, DT_CENTER);
                        let tl = inner.left + p(34);
                        let st = ad.status();
                        let sw = text_width(mem, small, &st);
                        let right = inner.right - p(22) - sw - p(18);
                        text(mem, body, &ad.name, RECT { left: tl, top: r.top + p(7), right, bottom: r.top + p(29) }, t.fg, DT_LEFT);
                        text(mem, small, &ad.summary(), RECT { left: tl, top: r.top + p(27), right, bottom: r.bottom - p(7) }, t.dim, DT_LEFT);
                        let sr = RECT { right: inner.right - p(22), ..inner };
                        text(mem, small, &st, sr, t.dim, DT_RIGHT);
                        let dot = p(7);
                        let (dx, cy) = (sr.right - sw - p(8) - dot, (r.top + r.bottom) / 2);
                        let dc = match (ad.up, ad.internet) {
                            (false, _) => t.off,
                            (_, Some(true)) => winrust_common::style::GOOD,
                            (_, Some(false)) => winrust_common::style::WARN,
                            _ => t.accent,
                        };
                        shape(mem, RECT { left: dx, top: cy - dot / 2, right: dx + dot, bottom: cy - dot / 2 + dot }, dc, rd(dot));
                        text(mem, icon_s, if *open { "\u{E70D}" } else { "\u{E76C}" }, inner, t.dim, DT_RIGHT);
                    }
                    Row::Wifi(nw, note) => {
                        let busy = a.connecting.as_ref().is_some_and(|c| c.0 == nw.ssid);
                        let (bw, gap, bh) = (p(3), p(2), p(14));
                        let (bx, by) = (inner.left + p(2), (r.top + r.bottom + bh) / 2);
                        for k in 0..4 {
                            let h = bh * (k + 1) / 4;
                            let lit = (k as u32) < bars(nw.quality);
                            let c = if !lit { t.off } else if nw.connected { t.accent } else { t.fg };
                            let x = bx + k * (bw + gap);
                            shape(mem, RECT { left: x, top: by - h, right: x + bw, bottom: by }, c, rd(2));
                        }
                        let tl = inner.left + p(34);
                        text(mem, body, &nw.ssid, RECT { left: tl, top: r.top + p(7), right: inner.right - p(52), bottom: r.top + p(29) }, t.fg, DT_LEFT);
                        let sub = if busy { "Connecting…" } else { note.as_str() };
                        text(mem, small, sub, RECT { left: tl, top: r.top + p(27), right: inner.right - p(52), bottom: r.bottom - p(7) }, if busy || nw.connected { t.accent } else { t.dim }, DT_LEFT);
                        if nw.connected {
                            text(mem, icon_s, "\u{E73E}", inner, t.accent, DT_RIGHT);
                        } else if nw.sec.is_none_or(|s| s.needs_key()) && nw.profile.is_none() {
                            text(mem, icon_s, "\u{E72E}", inner, t.dim, DT_RIGHT);
                        }
                    }
                    Row::Item(label, value, cmd) => {
                        let armed = a.armed.as_deref() == Some(label.as_str());
                        let lw = text_width(mem, body, label).min((inner.right - inner.left) * 3 / 5);
                        text(mem, body, label, RECT { right: inner.left + lw, ..inner }, if armed { winrust_common::style::BAD } else { t.fg }, DT_LEFT);
                        let val_s = if armed { "Click again" } else { value.as_str() };
                        let ink = if armed { winrust_common::style::BAD } else if matches!(cmd, Cmd::Vpn(v) if v.on) || val_s == "Connected" { t.accent } else { t.dim };
                        text(mem, body, val_s, RECT { left: inner.left + lw + p(16), right: inner.right - p(22), ..inner }, ink, DT_RIGHT);
                        text(mem, icon_s, "\u{E76C}", inner, t.dim, DT_RIGHT);
                    }
                    Row::Arrange(k) => {
                        let top = r.top - p(ROW_H * *k as i32);
                        let bx = RECT { left: r.left + p(16), top: top + p(10), right: r.right - p(16), bottom: top + p(ROW_H * 4) - p(10) };
                        let sv = SaveDC(mem);
                        IntersectClipRect(mem, r.left, r.top, r.right, r.bottom);
                        shape(mem, RECT { left: r.left, top, right: r.right, bottom: top + p(ROW_H * 4) }, t.card, 0);
                        let map = arr_map(&a.layout, bx);
                        let sel = display::selected_index();
                        for (j, m) in a.layout.iter().enumerate() {
                            let (x, y) = match &a.arr {
                                Some(d) if d.idx == j => d.cur,
                                _ => (m.x, m.y),
                            };
                            let (x0, y0) = map.dev(x, y);
                            let (x1, y1) = map.dev(x + m.w, y + m.h);
                            let mr = RECT { left: x0, top: y0, right: x1, bottom: y1 };
                            let dragging = a.arr.as_ref().is_some_and(|d| d.idx == j);
                            shape(mem, mr, if j == sel { t.accent } else { t.off }, rd(6));
                            shape(mem, inset(mr, p(2)), if j == sel { mix(t.accent, t.bg, if dragging { 0.0 } else { 0.25 }) } else { mix(t.card, t.fg, 0.1) }, rd(5));
                            text(mem, title, &(j + 1).to_string(), mr, if j == sel { rgb(255, 255, 255) } else { t.fg }, DT_CENTER);
                            if m.primary {
                                shape(mem, RECT { left: x0 + (x1 - x0) / 2 - p(14), top: y1 - p(10), right: x0 + (x1 - x0) / 2 + p(14), bottom: y1 - p(6) }, if j == sel { rgb(255, 255, 255) } else { t.dim }, rd(4));
                            }
                        }
                        RestoreDC(mem, sv);
                    }
                    Row::Slider(label, src) => {
                        text(mem, body, label, inner, t.fg, DT_LEFT);
                        let (l, rt) = track(cw, a.dpi);
                        let v = audio::volume(*src);
                        let (cy, h) = ((r.top + r.bottom) / 2, p(4));
                        shape(mem, RECT { left: l, top: cy - h / 2, right: rt, bottom: cy + h / 2 }, t.off, rd(h));
                        let x = l + ((rt - l) as f32 * v.unwrap_or(0.0)) as i32;
                        shape(mem, RECT { left: l, top: cy - h / 2, right: x, bottom: cy + h / 2 }, t.accent, rd(h));
                        if v.is_some() {
                            shape(mem, RECT { left: x - p(11), top: cy - p(11), right: x + p(11), bottom: cy + p(11) }, t.sep, rd(22));
                            shape(mem, RECT { left: x - p(10), top: cy - p(10), right: x + p(10), bottom: cy + p(10) }, rgb(255, 255, 255), rd(20));
                        }
                        let pct = v.map_or("—".into(), |v| format!("{}%", (v * 100.0).round() as u32));
                        text(mem, body, &pct, RECT { right: l - p(16), ..inner }, t.dim, DT_RIGHT);
                    }
                }
            }
            RestoreDC(mem, saved);
            let (r, e) = bar(cw, ch, a.dpi);
            let rad = rd(14) * 2;
            if a.pending > 0 {
                shape(mem, r, t.accent, rad);
                let msg = format!("Keep this mode? Enter keeps it, Esc or {} s goes back", a.pending);
                text(mem, body, &msg, RECT { left: r.left + p(18), right: r.right - p(18), ..r }, rgb(255, 255, 255), DT_LEFT);
            } else if let Some(pr) = &a.prompt {
                shape(mem, r, t.sep, rad);
                shape(mem, inset(r, 1), t.card, rad);
                shape(mem, inset(e, -p(2)), t.accent, rd(8) * 2);
                shape(mem, e, t.bg, rd(6) * 2);
                let ask = match pr {
                    Prompt::Ssid => "Hidden network name".into(),
                    Prompt::Proxy(false) => "Proxy: host:port (empty: remove)".into(),
                    Prompt::Proxy(true) => "PAC script address (empty: remove)".into(),
                    Prompt::EapUser(n) => format!("User name for \"{}\" (domain\\name)", n.ssid),
                    Prompt::EapPass(n, _) => format!("Password for \"{}\"", n.ssid),
                    Prompt::Key(n, true) => format!("Password for \"{}\" (empty: open)", n.ssid),
                    Prompt::Key(n, false) => format!("Password for \"{}\"", n.ssid),
                };
                let label = a.msg.clone().unwrap_or(ask);
                text(mem, body, &label, RECT { left: r.left + p(18), right: e.left - p(14), ..r }, if a.msg.is_some() { winrust_common::style::BAD } else { t.fg }, DT_LEFT);
            } else if let Some(m) = &a.msg {
                shape(mem, r, t.sep, rad);
                shape(mem, inset(r, 1), t.card, rad);
                let dot = p(8);
                let cy = (r.top + r.bottom) / 2;
                shape(mem, RECT { left: r.left + p(18), top: cy - dot / 2, right: r.left + p(18) + dot, bottom: cy + dot / 2 }, if a.connecting.is_some() { t.accent } else { t.dim }, rd(dot));
                text(mem, body, m, RECT { left: r.left + p(36), right: r.right - p(18), ..r }, t.fg, DT_LEFT);
            }
            if let (Some((pr, n, down)), Some(pk)) = (pick_geom(a, cw, ch), a.pick.as_ref()) {
                let k = (pk.t0.elapsed().as_secs_f32() / 0.18).min(1.0);
                let k = k * k * (3.0 - 2.0 * k);
                let hh = ((pr.bottom - pr.top) as f32 * (0.35 + 0.65 * k)) as i32;
                let r = if down { RECT { bottom: pr.top + hh, ..pr } } else { RECT { top: pr.bottom - hh, ..pr } };
                let rad = rd(12) * 2;
                let light = winrust_common::style::style().light();
                shape(mem, RECT { left: r.left - p(2), top: r.top + p(4), right: r.right + p(2), bottom: r.bottom + p(10) }, mix(t.bg, 0, if light { 0.08 } else { 0.35 }), rad + p(8));
                shape(mem, r, t.sep, rad);
                shape(mem, inset(r, 1), t.card, rad);
                let saved = SaveDC(mem);
                IntersectClipRect(mem, r.left, r.top + p(2), r.right, r.bottom - p(2));
                let off = if down { 0 } else { r.top - pr.top };
                for j in 0..n {
                    let idx = pk.top + j;
                    let ir = RECT { left: pr.left + p(6), top: pr.top + off + p(6 + PICK_ROW * j as i32), right: pr.right - p(6), bottom: pr.top + off + p(6 + PICK_ROW * (j as i32 + 1)) };
                    let ink = |c: u32| mix(t.card, c, k);
                    if pk.hover == Some(idx) {
                        shape(mem, ir, ink(t.hover), rd(8) * 2);
                    }
                    let sel = pk.cur == Some(idx);
                    text(mem, body, &pk.items[idx], RECT { left: ir.left + p(12), right: ir.right - p(36), ..ir }, ink(if sel { t.accent } else { t.fg }), DT_LEFT);
                    if sel {
                        text(mem, icon_s, "\u{E73E}", RECT { right: ir.right - p(12), ..ir }, ink(t.accent), DT_RIGHT);
                    }
                }
                let total = pk.items.len();
                if total > n {
                    let (th, tt) = (r.bottom - r.top - p(16), r.top + p(8));
                    let (a0, a1) = (tt + th * pk.top as i32 / total as i32, tt + th * (pk.top + n) as i32 / total as i32);
                    shape(mem, RECT { left: r.right - p(6), top: a0, right: r.right - p(3), bottom: a1 }, mix(t.card, t.dim, 0.6 * k), rd(3) * 2);
                }
                RestoreDC(mem, saved);
            }
        });
        BitBlt(dc, 0, 0, cw, ch, mem, 0, 0, SRCCOPY);
        EndPaint(hwnd, &ps);
    }
}

enum Act {
    Page(usize),
    Flip(&'static str, &'static str, u32),
    Do(Op),
    Pick(Op, usize),
    Volume(f32, audio::Src),
    Cmd(Cmd),
    Arr(i32, i32),
    Wifi(wifi::Net),
    Adapter(String),
}

fn row_at_scrolled(a: &App, y: i32) -> Option<usize> {
    let p = |v: i32| (v as f64 * (a.dpi as f64 / 96.0)) as i32;
    if y < p(ROWS_TOP - 8) {
        return None;
    }
    let rows = &a.pages.get(a.page)?.rows;
    row_at(y + p(a.scroll), p(ROWS_TOP), p(ROW_H), rows.len())
}

fn click(x: i32, y: i32, cw: i32) -> Option<Act> {
    APP.with_borrow(|a| {
        let p = |v: i32| (v as f64 * (a.dpi as f64 / 96.0)) as i32;
        if x < side_w(cw, a.dpi) {
            return row_at(y, p(NAV_TOP), p(NAV_H), a.pages.len()).map(Act::Page);
        }
        let rows = &a.pages.get(a.page)?.rows;
        let idx = row_at_scrolled(a, y)?;
        match rows.get(idx)? {
            Row::Cycle(_, _, op) if listed(*op) => Some(Act::Pick(*op, idx)),
            Row::Cycle(_, _, op) => Some(Act::Do(*op)),
            Row::Slider(_, src) => {
                let (l, r) = track(cw, a.dpi);
                (x >= l - p(12) && x <= r + p(12)).then(|| Act::Volume(slider_value(x, l, r), *src))
            }
            Row::Item(_, _, cmd) => Some(Act::Cmd(cmd.clone())),
            Row::Arrange(_) => Some(Act::Arr(x, y)),
            Row::Toggle { key, name, on_when, default, .. } => {
                let on = is_on(sys::dword(HKEY_CURRENT_USER, key, name), *on_when, *default);
                Some(Act::Flip(key, name, if on { 1 - on_when } else { *on_when }))
            }
            Row::Wifi(n, _) => Some(Act::Wifi(n.clone())),
            Row::Adapter(ad, _) => Some(Act::Adapter(ad.name.clone())),
            Row::Switch(_, _, op) => Some(Act::Do(*op)),
            Row::Info(..) | Row::Header(_) => None,
        }
    })
}

unsafe fn finish_confirm(hwnd: HWND, keep: bool) {
    unsafe {
        KillTimer(hwnd, 1);
        APP.with_borrow_mut(|a| a.pending = 0);
        match (keep, display::current()) {
            (true, Some(m)) => {
                display::apply(m, true);
            }
            _ => display::revert(),
        }
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe fn display_or_mute(_hwnd: HWND, op: Op) {
    match op {
        Op::Mute => audio::set_mute(false, !audio::muted(false).unwrap_or(false)),
        Op::MicMute => audio::set_mute(true, !audio::muted(true).unwrap_or(false)),
        Op::Primary => drop(display::make_primary()),
        Op::Hdr => {
            if let Some(on) = display::hdr() {
                display::set_hdr(!on);
            }
        }
        Op::Bt => {
            if let Some(on) = bt::state().bt {
                bt::set_bluetooth(!on, ping);
            }
        }
        Op::Air => {
            if let Some(on) = bt::state().air {
                bt::set_airplane(!on, ping);
            }
        }
        Op::Proxy => unsafe { open_prompt(_hwnd, Prompt::Proxy(false)) },
        Op::Pac => unsafe { open_prompt(_hwnd, Prompt::Proxy(true)) },
        _ => {}
    }
}

static MAIN: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
const WM_NOTE: u32 = 0x8000 + 3;
const WM_REFRESH: u32 = 0x8000 + 4;

fn ping() {
    unsafe { PostMessageW(MAIN.load(std::sync::atomic::Ordering::Relaxed) as HWND, WM_REFRESH, 0, 0) };
}

fn note(s: String) {
    let b = Box::into_raw(Box::new(s));
    if unsafe { PostMessageW(MAIN.load(std::sync::atomic::Ordering::Relaxed) as HWND, WM_NOTE, 0, b as isize) } == 0 {
        drop(unsafe { Box::from_raw(b) });
    }
}

unsafe fn refresh_page(hwnd: HWND) {
    unsafe {
        let (page, busy, mut apps) = APP.with_borrow_mut(|a| (a.page, a.pick.is_some() || a.drag.is_some() || a.arr.is_some(), a.apps.take()));
        let (rows, layout) = if busy { (None, None) } else { (dynamic(page, &mut apps), (page == 0).then(display::layout)) };
        APP.with_borrow_mut(|a| {
            a.apps = apps;
            if let Some(l) = layout {
                a.layout = l;
            }
            if let Some(rows) = rows {
                if let Some(p) = a.pages.get_mut(page) {
                    p.rows = rows;
                }
                a.hover = a.hover.filter(|&i| a.pages.get(a.page).is_some_and(|p| i < p.rows.len()));
            }
        });
        clamp_scroll(hwnd, 0);
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe fn goto(hwnd: HWND, i: usize) {
    unsafe {
        close_prompt(hwnd);
        APP.with_borrow_mut(|a| {
            (a.pick, a.armed) = (None, None);
            (a.page, a.scroll, a.hover) = (i, 0, None);
            if i == APPS_PAGE {
                a.apps = None;
            }
        });
        match i {
            NET_PAGE => wifi::scan(),
            BT_PAGE => bt::refresh(ping),
            2 => {
                if ext::TEMP_BYTES.load(std::sync::atomic::Ordering::Relaxed) == u64::MAX {
                    ext::measure_temp(ping);
                }
            }
            _ => {}
        }
        refresh_page(hwnd);
    }
}

fn once_more(label: &str) -> bool {
    APP.with_borrow_mut(|a| {
        let again = a.armed.as_deref() == Some(label);
        a.armed = if again { None } else { Some(label.to_string()) };
        again
    })
}

unsafe fn run_cmd(hwnd: HWND, label: &str, cmd: Cmd) {
    unsafe {
        let say = |m: String| APP.with_borrow_mut(|a| a.msg = Some(m));
        match cmd {
            Cmd::Uninstall(c) => {
                if once_more(label) {
                    say(if ext::uninstall(&c) { format!("Uninstalling \"{}\"", label) } else { format!("Could not start uninstalling \"{}\"", label) });
                }
            }
            Cmd::BtUnpair(id) => {
                if once_more(label) {
                    bt::unpair(id, ping);
                }
            }
            c => {
                APP.with_borrow_mut(|a| a.armed = None);
                match c {
                    Cmd::Vpn(v) => {
                        let name = v.name.clone();
                        if v.on {
                            std::thread::spawn(move || {
                                ext::hang_up(&name);
                                note(format!("VPN \"{}\" disconnected", name));
                            });
                        } else {
                            say(format!("Connecting to \"{}\"…", name));
                            std::thread::spawn(move || note(ext::dial(&name, v.pbk.as_deref()).map_or_else(|e| e, |_| format!("VPN \"{}\" connected", name))));
                        }
                    }
                    Cmd::CleanTemp => {
                        say("Cleaning…".into());
                        ext::clean_temp(note);
                    }
                    Cmd::Wallpaper => {
                        if sys::pick_wallpaper(hwnd).is_ok() {
                            say("Wallpaper changed".into());
                        }
                    }
                    Cmd::SyncClock => {
                        say("Syncing clock…".into());
                        std::thread::spawn(sync_clock);
                    }
                    Cmd::EmptyBin => {
                        if ext::empty_bin(hwnd) {
                            say("Recycle Bin emptied".into());
                        }
                    }
                    Cmd::BtPair(id) => {
                        say("Pairing…".into());
                        bt::pair(id, ping);
                    }
                    Cmd::BtScan => {
                        bt::scan(ping);
                        ping();
                    }
                    Cmd::Check => {
                        wu::check(ping);
                        ping();
                    }
                    Cmd::Install => {
                        wu::install(ping);
                        ping();
                    }
                    Cmd::Uninstall(_) | Cmd::BtUnpair(_) => {}
                }
            }
        }
        refresh_page(hwnd);
    }
}

fn sync_clock() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use windows_sys::Win32::{
        System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
        UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
    };
    const NO_WIN: u32 = 0x0800_0000;
    let run = |prog: &str, args: &[&str]| {
        Command::new(prog)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(NO_WIN)
            .status()
            .is_ok_and(|s| s.success())
    };
    let _ = run("net.exe", &["start", "w32time"]);
    let ok = if run("w32tm.exe", &["/resync", "/force"]) {
        true
    } else {
        let (verb, file, params) = (wide("runas"), wide("cmd.exe"), wide("/c net start w32time >nul 2>&1 & w32tm /resync /force"));
        let mut ei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        ei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        ei.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI;
        ei.lpVerb = verb.as_ptr();
        ei.lpFile = file.as_ptr();
        ei.lpParameters = params.as_ptr();
        ei.nShow = SW_HIDE;
        if unsafe { ShellExecuteExW(&mut ei) } != 0 && !ei.hProcess.is_null() {
            let mut code = 1u32;
            unsafe {
                WaitForSingleObject(ei.hProcess, INFINITE);
                GetExitCodeProcess(ei.hProcess, &mut code);
                CloseHandle(ei.hProcess);
            }
            code == 0
        } else {
            false
        }
    };
    note(if ok {
        "Clock synchronized".into()
    } else {
        "Could not sync clock (needs administrator rights)".into()
    });
}

unsafe fn clamp_scroll(hwnd: HWND, by: i32) {
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        APP.with_borrow_mut(|a| {
            let n = a.pages.get(a.page).map_or(0, |p| p.rows.len());
            a.scroll = (a.scroll + by).clamp(0, max_scroll(n, rc.bottom * 96 / a.dpi.max(1)));
        });
    }
}

fn bar(cw: i32, ch: i32, dpi: i32) -> (RECT, RECT) {
    let p = |v: i32| v * dpi / 96;
    let (cl, cr) = col(cw, dpi);
    let w = (cr - cl).min(p(620));
    let l = cl + (cr - cl - w) / 2;
    let r = RECT { left: l, top: ch - p(78), right: l + w, bottom: ch - p(24) };
    let e = RECT { left: r.left + w * 11 / 20, top: r.top + p(12), right: r.right - p(12), bottom: r.bottom - p(12) };
    (r, e)
}

const WM_EDIT_KEY: u32 = 0x8000 + 2;
const EM_SETSEL: u32 = 0xB1;
const EM_LIMITTEXT: u32 = 0xC5;
const EM_SETMARGINS: u32 = 0xD3;
static EDIT_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

unsafe extern "system" fn edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_KEYDOWN if wp == VK_RETURN as usize || wp == VK_ESCAPE as usize => {
                PostMessageW(GetParent(h), WM_EDIT_KEY, wp, 0);
                0
            }
            WM_CHAR if wp == 13 || wp == 27 => 0,
            _ => CallWindowProcW(std::mem::transmute::<isize, WNDPROC>(EDIT_PROC.load(std::sync::atomic::Ordering::Relaxed)), h, msg, wp, lp),
        }
    }
}

unsafe fn close_prompt(hwnd: HWND) {
    unsafe {
        let ed = APP.with_borrow_mut(|a| {
            a.prompt = None;
            std::mem::take(&mut a.edit)
        });
        if ed != 0 {
            SetWindowTextW(ed as HWND, wide("").as_ptr());
            DestroyWindow(ed as HWND);
            SetFocus(hwnd);
        }
    }
}

unsafe fn open_prompt(hwnd: HWND, pr: Prompt) {
    unsafe {
        close_prompt(hwnd);
        let key = matches!(pr, Prompt::Key(..) | Prompt::EapPass(..));
        let limit = match pr {
            Prompt::Ssid => 32,
            Prompt::Proxy(_) => 300,
            Prompt::EapUser(_) => 64,
            _ => 128,
        };
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let (dpi, font) = APP.with_borrow(|a| (a.dpi, a.fonts[0]));
        let (_, e) = bar(rc.right, rc.bottom, dpi);
        let style = WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32 | if key { ES_PASSWORD as u32 } else { 0 };
        let ed = CreateWindowExW(0, wide("EDIT").as_ptr(), null(), style, e.left, e.top, e.right - e.left, e.bottom - e.top, hwnd, null_mut(), GetModuleHandleW(null()), null());
        if ed.is_null() {
            return;
        }
        SendMessageW(ed, WM_SETFONT, font as usize, 0);
        SendMessageW(ed, EM_LIMITTEXT, limit, 0);
        SendMessageW(ed, EM_SETMARGINS, 3, (6 * dpi / 96 * 0x10001) as isize);
        EDIT_PROC.store(SetWindowLongPtrW(ed, GWLP_WNDPROC, edit_proc as *const () as isize), std::sync::atomic::Ordering::Relaxed);
        SetFocus(ed);
        APP.with_borrow_mut(|a| (a.prompt, a.edit, a.msg) = (Some(pr), ed as isize, None));
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe fn edit_text(ed: isize) -> String {
    unsafe {
        let mut buf = vec![0u16; 320];
        let n = GetWindowTextW(ed as HWND, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let s = String::from_utf16_lossy(&buf[..n]);
        buf.fill(0);
        s
    }
}

unsafe fn start_wait(hwnd: HWND, ssid: String, profile: String, new: bool) {
    unsafe {
        APP.with_borrow_mut(|a| {
            a.msg = Some(format!("Connecting to \"{}\"…", ssid));
            a.connecting = Some((ssid, profile, new));
        });
        SetTimer(hwnd, 3, 30_000, None);
    }
}

unsafe fn add_network(hwnd: HWND, n: &wifi::Net, sec: wifi::Sec, key: &str, hidden: bool) -> bool {
    unsafe {
        match wifi::add_and_connect(&n.ssid, &n.raw, sec, key, hidden, Some(&n.iface)) {
            Ok(p) => {
                close_prompt(hwnd);
                start_wait(hwnd, n.ssid.clone(), p, true);
                true
            }
            Err(e) => {
                APP.with_borrow_mut(|a| a.msg = Some(e));
                false
            }
        }
    }
}

unsafe fn submit_prompt(hwnd: HWND) {
    unsafe {
        let ed = APP.with_borrow(|a| a.edit);
        let mut s = edit_text(ed);
        match APP.with_borrow_mut(|a| a.prompt.take()) {
            Some(Prompt::Ssid) if s.trim().is_empty() || s.len() > 32 => {
                APP.with_borrow_mut(|a| (a.prompt, a.msg) = (Some(Prompt::Ssid), Some("Network name: 1 to 32 bytes".into())));
            }
            Some(Prompt::Ssid) => {
                let n = wifi::Net { raw: s.as_bytes().to_vec(), ssid: s.clone(), quality: 0, connected: false, profile: None, sec: Some(wifi::Sec::Wpa2 { tkip: false }), eap: None, iface: std::mem::zeroed() };
                open_prompt(hwnd, Prompt::Key(n, true));
            }
            Some(Prompt::Proxy(pac)) => {
                let v = s.trim().to_string();
                if if pac { ext::valid_pac(&v) } else { ext::valid_proxy(&v) } {
                    ext::set_proxy(pac, &v);
                    close_prompt(hwnd);
                    refresh_page(hwnd);
                } else {
                    APP.with_borrow_mut(|a| (a.prompt, a.msg) = (Some(Prompt::Proxy(pac)), Some("Not a valid address".into())));
                }
            }
            Some(Prompt::EapUser(n)) if s.trim().is_empty() => APP.with_borrow_mut(|a| (a.prompt, a.msg) = (Some(Prompt::EapUser(n)), Some("Enter the user name".into()))),
            Some(Prompt::EapUser(n)) => open_prompt(hwnd, Prompt::EapPass(n, s.trim().to_string())),
            Some(Prompt::EapPass(n, user)) => match wifi::add_eap_and_connect(&n.ssid, &n.raw, n.eap.unwrap_or("WPA2"), &user, &s, false, Some(&n.iface)) {
                Ok(profile) => {
                    close_prompt(hwnd);
                    start_wait(hwnd, n.ssid.clone(), profile, true);
                }
                Err(e) => {
                    APP.with_borrow_mut(|a| (a.prompt, a.msg) = (Some(Prompt::EapPass(n, user)), Some(e)));
                    SendMessageW(ed as HWND, EM_SETSEL, 0, -1);
                }
            },
            Some(Prompt::Key(n, hidden)) => {
                let sec = if hidden && s.is_empty() { wifi::Sec::Open } else { n.sec.unwrap_or(wifi::Sec::Wpa2 { tkip: false }) };
                if !add_network(hwnd, &n, sec, &s, hidden) {
                    APP.with_borrow_mut(|a| a.prompt = Some(Prompt::Key(n, hidden)));
                    SendMessageW(ed as HWND, EM_SETSEL, 0, -1);
                }
            }
            None => {}
        }
        s.as_bytes_mut().fill(0);
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe fn wifi_click(hwnd: HWND, n: wifi::Net) {
    unsafe {
        close_prompt(hwnd);
        match (&n.profile, n.sec) {
            _ if n.connected => APP.with_borrow_mut(|a| a.msg = Some("Disconnect or forget: right click".into())),
            (Some(p), _) => {
                if wifi::connect(p, &n.iface) {
                    start_wait(hwnd, n.ssid.clone(), p.clone(), false);
                }
            }
            (None, Some(s)) if !s.needs_key() => drop(add_network(hwnd, &n, s, "", false)),
            (None, Some(_)) => open_prompt(hwnd, Prompt::Key(n, false)),
            (None, None) if n.eap.is_some() => open_prompt(hwnd, Prompt::EapUser(n)),
            (None, None) => APP.with_borrow_mut(|a| a.msg = Some("This network signs in with a certificate: not supported".into())),
        }
        SetTimer(hwnd, 2, 1500, None);
    }
}

unsafe fn wifi_menu(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let hit = APP.with_borrow(|a| {
            (x >= side_w(rc.right, a.dpi)).then(|| row_at_scrolled(a, y)).flatten().and_then(|i| match a.pages.get(a.page)?.rows.get(i)? {
                Row::Wifi(n, _) if n.profile.is_some() => Some(n.clone()),
                _ => None,
            })
        });
        let Some(n) = hit else { return };
        let Some(profile) = n.profile.clone() else { return };
        let (on, off, forget) = (wide("Connect"), wide("Disconnect"), wide("Forget network"));
        let m = CreatePopupMenu();
        AppendMenuW(m, MF_STRING, 1, if n.connected { off.as_ptr() } else { on.as_ptr() });
        AppendMenuW(m, MF_STRING, 2, forget.as_ptr());
        let mut pt = POINT { x, y };
        ClientToScreen(hwnd, &mut pt);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(m);
        match cmd {
            1 if n.connected => drop(wifi::disconnect()),
            1 => wifi_click(hwnd, n),
            2 => drop(wifi::forget(&profile)),
            _ => return,
        }
        SetTimer(hwnd, 2, 1500, None);
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == winrust_common::style::msg() {
            winrust_common::style::reload();
            frame(hwnd);
            InvalidateRect(hwnd, null(), 0);
            return 0;
        }
        match msg {
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            0x02E0 => {
                let dpi = (wp & 0xFFFF) as i32;
                if dpi > 0 {
                    APP.with_borrow_mut(|a| a.dpi = dpi);
                    refresh_fonts();
                    let r = &*(lp as *const RECT);
                    SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                    clamp_scroll(hwnd, 0);
                    let ed = APP.with_borrow(|a| a.edit);
                    if ed != 0 {
                        let mut rc: RECT = std::mem::zeroed();
                        GetClientRect(hwnd, &mut rc);
                        let (_, e) = bar(rc.right, rc.bottom, dpi);
                        MoveWindow(ed as HWND, e.left, e.top, e.right - e.left, e.bottom - e.top, 1);
                        SendMessageW(ed as HWND, WM_SETFONT, APP.with_borrow(|a| a.fonts[0]) as usize, 0);
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_SIZE | WM_SETTINGCHANGE => {
                clamp_scroll(hwnd, 0);
                let (ed, dpi) = APP.with_borrow(|a| (a.edit, a.dpi));
                if ed != 0 {
                    let mut rc: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut rc);
                    let (_, e) = bar(rc.right, rc.bottom, dpi);
                    MoveWindow(ed as HWND, e.left, e.top, e.right - e.left, e.bottom - e.top, 1);
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_CTLCOLOREDIT => {
                let t = theme();
                SetTextColor(wp as HDC, t.fg);
                SetBkColor(wp as HDC, t.bg);
                let b = APP.with_borrow_mut(|a| {
                    if a.brush == 0 || a.brush_col != t.bg {
                        if a.brush != 0 {
                            DeleteObject(a.brush as _);
                        }
                        a.brush = CreateSolidBrush(t.bg) as isize;
                        a.brush_col = t.bg;
                    }
                    a.brush
                });
                b as LRESULT
            }
            WM_EDIT_KEY => {
                if wp == VK_RETURN as usize {
                    submit_prompt(hwnd);
                } else {
                    close_prompt(hwnd);
                    APP.with_borrow_mut(|a| a.msg = None);
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            wifi::WM_WIFI => {
                if wp as i32 == wlan_notification_acm_connection_complete && lp != 0 {
                    let (reason, profile) = *Box::from_raw(lp as *mut (u32, String));
                    if let Some((ssid, prof, new)) = APP.with_borrow_mut(|a| a.connecting.take_if(|c| c.1 == profile)) {
                        KillTimer(hwnd, 3);
                        let m = (reason != 0).then(|| {
                            if new {
                                wifi::forget(&prof);
                            }
                            format!("Could not connect to \"{}\": {}", ssid, wifi::reason_text(reason))
                        });
                        APP.with_borrow_mut(|a| a.msg = m);
                    }
                }
                if APP.with_borrow(|a| a.page == NET_PAGE) {
                    refresh_page(hwnd);
                }
                0
            }
            WM_NOTE => {
                let m = *Box::from_raw(lp as *mut String);
                APP.with_borrow_mut(|a| a.msg = Some(m));
                refresh_page(hwnd);
                0
            }
            WM_REFRESH => {
                if let Some(m) = bt::take_note() {
                    APP.with_borrow_mut(|a| a.msg = Some(m));
                }
                refresh_page(hwnd);
                0
            }
            WM_TIMER if wp == 3 => {
                KillTimer(hwnd, 3);
                if let Some((ssid, ..)) = APP.with_borrow_mut(|a| a.connecting.take()) {
                    APP.with_borrow_mut(|a| a.msg = Some(format!("\"{}\" did not answer in 30 s", ssid)));
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_TIMER if wp == 4 => {
                if APP.with_borrow(|a| a.pick.as_ref().is_none_or(|pk| pk.t0.elapsed().as_millis() > 220)) {
                    KillTimer(hwnd, 4);
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEWHEEL if APP.with_borrow(|a| a.pick.is_some()) => {
                let up = (wp >> 16) as i16 > 0;
                APP.with_borrow_mut(|a| {
                    if let Some(pk) = a.pick.as_mut() {
                        let max = pk.items.len().saturating_sub(PICK_MAX);
                        pk.top = if up { pk.top.saturating_sub(1) } else { (pk.top + 1).min(max) };
                    }
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEWHEEL => {
                clamp_scroll(hwnd, -((wp >> 16) as i16 as i32) / 120 * ROW_H);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_TIMER if wp == 2 => {
                if IsIconic(hwnd) == 0 {
                    let (page, n) = APP.with_borrow_mut(|a| {
                        a.ticks += 1;
                        (a.page, a.ticks)
                    });
                    if page == NET_PAGE && n % 8 == 0 {
                        wifi::scan();
                    }
                    if page == BT_PAGE {
                        bt::refresh(ping);
                    }
                    if matches!(page, 0..=4 | 6) {
                        refresh_page(hwnd);
                    }
                }
                0
            }
            WM_TIMER => {
                if APP.with_borrow_mut(|a| (a.pending > 0).then(|| a.pending -= 1).is_some_and(|_| a.pending == 0)) {
                    finish_confirm(hwnd, false);
                } else {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_MOUSEMOVE => {
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let (x, y) = ((lp & 0xFFFF) as i16 as i32, (lp >> 16) as i16 as i32);
                if let Some(src) = APP.with_borrow(|a| a.drag) {
                    let dpi = APP.with_borrow(|a| a.dpi);
                    let (l, r) = track(rc.right, dpi);
                    audio::set_volume(src, slider_value(x, l, r));
                    InvalidateRect(hwnd, null(), 0);
                }
                if APP.with_borrow(|a| a.arr.is_some()) {
                    APP.with_borrow_mut(|a| {
                        let Some(bx) = arr_box(a, rc.right) else { return };
                        let (px, py) = arr_map(&a.layout, bx).desk(x, y);
                        if let Some(d) = a.arr.as_mut() {
                            d.cur = (px - d.grab.0, py - d.grab.1);
                            let m = a.layout[d.idx];
                            d.moved |= (d.cur.0 - m.x).abs() + (d.cur.1 - m.y).abs() > 60;
                        }
                    });
                    InvalidateRect(hwnd, null(), 0);
                }
                if !APP.with_borrow_mut(|a| std::mem::replace(&mut a.tracking, true)) {
                    let mut te = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    TrackMouseEvent(&mut te);
                }
                let over = APP.with_borrow_mut(|a| {
                    let it = pick_item(a, rc.right, rc.bottom, x, y);
                    let pk = a.pick.as_mut()?;
                    let old = pk.hover;
                    pk.hover = it.or(old);
                    Some(old != pk.hover)
                });
                if over.is_some() {
                    if over == Some(true) {
                        InvalidateRect(hwnd, null(), 0);
                    }
                    return 0;
                }
                let changed = APP.with_borrow_mut(|a| {
                    let side = side_w(rc.right, a.dpi);
                    let now = a.pages.get(a.page).and_then(|pg| (x >= side).then(|| row_at_scrolled(a, y)).flatten().filter(|&i| !matches!(pg.rows[i], Row::Info(..) | Row::Header(_) | Row::Arrange(_))));
                    std::mem::replace(&mut a.hover, now) != now
                });
                if changed {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_MOUSELEAVE => {
                if APP.with_borrow_mut(|a| {
                    a.tracking = false;
                    a.hover.take().is_some()
                }) {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                let dropped = APP.with_borrow_mut(|a| {
                    a.drag = None;
                    a.arr.take().map(|d| (d, a.layout.clone()))
                });
                if let Some((d, mons)) = dropped {
                    let m = mons[d.idx];
                    if d.moved {
                        let others: Vec<_> = mons.iter().enumerate().filter(|(j, _)| *j != d.idx).map(|(_, m)| *m).collect();
                        let (x, y) = display::snap(d.cur.0, d.cur.1, m.w, m.h, &others);
                        if (x, y) != (m.x, m.y) && !display::move_monitor(d.idx, x, y) {
                            APP.with_borrow_mut(|a| a.msg = Some("Windows did not accept that arrangement".into()));
                        }
                    } else {
                        display::select_monitor(d.idx);
                    }
                    refresh_page(hwnd);
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let (x, y) = ((lp & 0xFFFF) as i16 as i32, (lp >> 16) as i16 as i32);
                if let Some(picked) = APP.with_borrow_mut(|a| {
                    let it = pick_item(a, rc.right, rc.bottom, x, y);
                    a.pick.take().map(|pk| it.map(|i| (pk.op, i)))
                }) {
                    if let Some((op, i)) = picked {
                        choose(hwnd, op, i);
                    }
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                APP.with_borrow_mut(|a| {
                    if a.prompt.is_none() && a.connecting.is_none() {
                        a.msg = None;
                    }
                });
                match click(x, y, rc.right) {
                    Some(Act::Do(Op::Radio)) => {
                        wifi::set_radio(!RADIO.get().is_some_and(|r| r.0));
                        SetTimer(hwnd, 2, 1500, None);
                    }
                    Some(Act::Do(Op::Hidden)) => open_prompt(hwnd, Prompt::Ssid),
                    Some(Act::Do(op)) => display_or_mute(hwnd, op),
                    Some(Act::Pick(op, row)) => open_pick(hwnd, op, row),
                    Some(Act::Volume(v, src)) => {
                        audio::set_volume(src, v);
                        SetCapture(hwnd);
                        APP.with_borrow_mut(|a| a.drag = Some(src));
                    }
                    Some(Act::Cmd(cmd)) => {
                        let label = APP.with_borrow(|a| row_at_scrolled(a, y).and_then(|i| match a.pages.get(a.page)?.rows.get(i)? {
                            Row::Item(l, ..) => Some(l.clone()),
                            _ => None,
                        }));
                        run_cmd(hwnd, &label.unwrap_or_default(), cmd);
                    }
                    Some(Act::Arr(x, y)) => {
                        let started = APP.with_borrow_mut(|a| {
                            let bx = arr_box(a, rc.right)?;
                            let (px, py) = arr_map(&a.layout, bx).desk(x, y);
                            let i = a.layout.iter().position(|m| px >= m.x && px < m.x + m.w && py >= m.y && py < m.y + m.h)?;
                            let m = a.layout[i];
                            a.arr = Some(ArrDrag { idx: i, grab: (px - m.x, py - m.y), cur: (m.x, m.y), moved: false });
                            Some(())
                        });
                        if started.is_some() {
                            SetCapture(hwnd);
                        }
                    }
                    Some(Act::Page(i)) => goto(hwnd, i),
                    Some(Act::Flip(k, n, v)) => {
                        if sys::set_hkcu_dword(k, n, v) {
                            if k == PERSONALIZE {
                                sys::announce_theme();
                            } else if k == ext::INET {
                                ext::proxy_changed();
                            }
                        }
                    }
                    Some(Act::Wifi(n)) => wifi_click(hwnd, n),
                    Some(Act::Adapter(name)) => {
                        EXPANDED.with_borrow_mut(|e| *e = if e.as_deref() == Some(name.as_str()) { None } else { Some(name) });
                        refresh_page(hwnd);
                    }
                    None => {}
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_RBUTTONUP => {
                wifi_menu(hwnd, (lp & 0xFFFF) as i16 as i32, (lp >> 16) as i16 as i32);
                0
            }
            WM_KEYDOWN => {
                match wp as u16 {
                    VK_ESCAPE | VK_UP | VK_DOWN | VK_RETURN if APP.with_borrow(|a| a.pick.is_some()) => {
                        let key = wp as u16;
                        let go = APP.with_borrow_mut(|a| {
                            let pk = a.pick.as_mut()?;
                            let n = pk.items.len();
                            match key {
                                VK_UP | VK_DOWN => {
                                    let h = pk.hover.map_or(0, |h| if key == VK_UP { h.saturating_sub(1) } else { (h + 1).min(n - 1) });
                                    pk.hover = Some(h);
                                    pk.top = pk.top.min(h).max((h + 1).saturating_sub(PICK_MAX));
                                    None
                                }
                                VK_RETURN => a.pick.take().and_then(|pk| pk.hover.map(|h| (pk.op, h))),
                                _ => {
                                    a.pick = None;
                                    None
                                }
                            }
                        });
                        if let Some((op, i)) = go {
                            choose(hwnd, op, i);
                        }
                        InvalidateRect(hwnd, null(), 0);
                    }
                    VK_ESCAPE => {
                        if APP.with_borrow(|a| a.pending > 0) {
                            finish_confirm(hwnd, false);
                        } else if APP.with_borrow(|a| a.prompt.is_some()) {
                            close_prompt(hwnd);
                            InvalidateRect(hwnd, null(), 0);
                        } else {
                            DestroyWindow(hwnd);
                        }
                    }
                    VK_RETURN if APP.with_borrow(|a| a.pending > 0) => finish_confirm(hwnd, true),
                    k @ (VK_UP | VK_DOWN) => {
                        let to = APP.with_borrow(|a| if k == VK_UP { a.page.saturating_sub(1) } else { (a.page + 1).min(a.pages.len() - 1) });
                        goto(hwnd, to);
                        InvalidateRect(hwnd, null(), 0);
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                APP.with_borrow_mut(|a| {
                    for f in a.fonts.iter_mut() {
                        if *f != 0 {
                            DeleteObject(*f as _);
                            *f = 0;
                        }
                    }
                    if a.brush != 0 {
                        DeleteObject(a.brush as _);
                        a.brush = 0;
                    }
                    if a.mem_dc != 0 {
                        if a.mem_old_bmp != 0 {
                            SelectObject(a.mem_dc as _, a.mem_old_bmp as _);
                            a.mem_old_bmp = 0;
                        }
                        if a.mem_bmp != 0 {
                            DeleteObject(a.mem_bmp as _);
                            a.mem_bmp = 0;
                        }
                        DeleteDC(a.mem_dc as _);
                        a.mem_dc = 0;
                    } else if a.mem_bmp != 0 {
                        DeleteObject(a.mem_bmp as _);
                        a.mem_bmp = 0;
                    }
                });
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn main() {
    winrust_common::install_panic_hook("rust-settings");
    if std::env::args().nth(1).as_deref() == Some("--nightlight") {
        return ext::night_helper();
    }
    audio::init();
    unsafe {
        let (mut token, mut input): (usize, windows_sys::Win32::Graphics::GdiPlus::GdiplusStartupInput) = (0, std::mem::zeroed());
        input.GdiplusVersion = 1;
        windows_sys::Win32::Graphics::GdiPlus::GdiplusStartup(&mut token, &input, null_mut());
        SetProcessDPIAware();
        let dpi = GetDpiForSystem() as i32;
        let start = std::env::args().nth(1).map_or(0, |a| page_for(&a));
        let (pg, lay) = (pages(), display::layout());
        APP.with_borrow_mut(|a| (a.pages, a.page, a.dpi, a.layout) = (pg, start, dpi, lay));
        refresh_fonts();
        let (inst, class) = (GetModuleHandleW(null()), wide("WinRustSettings"));
        let wc = WNDCLASSW { style: 0, lpfnWndProc: Some(proc), cbClsExtra: 0, cbWndExtra: 0, hInstance: inst, hIcon: null_mut(), hCursor: LoadCursorW(null_mut(), IDC_ARROW), hbrBackground: null_mut(), lpszMenuName: null(), lpszClassName: class.as_ptr() };
        RegisterClassW(&wc);
        let mut wa: RECT = std::mem::zeroed();
        let (wa_w, wa_h) = if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as *mut _, 0) != 0 {
            (wa.right - wa.left, wa.bottom - wa.top)
        } else {
            (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
        };
        let win_w = (940 * dpi / 96).min((wa_w - 32).max(600));
        let win_h = (620 * dpi / 96).min((wa_h - 32).max(440));
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("Settings").as_ptr(), WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN, CW_USEDEFAULT, CW_USEDEFAULT, win_w, win_h, null_mut(), null_mut(), inst, null());
        frame(hwnd);
        MAIN.store(hwnd as isize, std::sync::atomic::Ordering::Relaxed);
        wifi::watch(hwnd);
        goto(hwnd, start);
        SetTimer(hwnd, 2, 4000, None);
        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
        windows_sys::Win32::Graphics::GdiPlus::GdiplusShutdown(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_state_follows_value_and_default() {
        assert!(is_on(Some(0), 0, 1));
        assert!(!is_on(Some(1), 0, 1));
        assert!(!is_on(None, 0, 1));
        assert!(!is_on(None, 1, 0));
    }

    #[test]
    fn rows_are_hit_by_position() {
        assert_eq!(row_at(10, 76, 52, 3), None);
        assert_eq!(row_at(76, 76, 52, 3), Some(0));
        assert_eq!(row_at(76 + 52 * 2 + 51, 76, 52, 3), Some(2));
        assert_eq!(row_at(76 + 52 * 3, 76, 52, 3), None);
    }

    #[test]
    fn short_pages_do_not_scroll_and_long_ones_stop_at_the_end() {
        assert_eq!(max_scroll(3, 620), 0);
        assert_eq!(max_scroll(20, 620), ROWS_TOP + ROW_H * 20 + 24 - 620);
    }

    #[test]
    fn every_page_has_its_keys() {
        assert_eq!(pages().len(), PAGE_KEYS.len());
        assert_eq!(page_for("ms-settings:network-status"), 3);
    }

    #[test]
    fn the_arrangement_picture_maps_both_ways_and_keeps_displays_inside() {
        let mons = [display::Mon { x: 0, y: 0, w: 1920, h: 1080, primary: true }, display::Mon { x: 1920, y: 0, w: 1280, h: 1024, primary: false }];
        let bx = RECT { left: 100, top: 20, right: 500, bottom: 180 };
        let m = arr_map(&mons, bx);
        let (x0, y0) = m.dev(0, 0);
        let (x1, y1) = m.dev(3200, 1080);
        assert!(x0 >= bx.left && y0 >= bx.top && x1 <= bx.right && y1 <= bx.bottom);
        let (bx2, by2) = m.dev(1920, 500);
        assert!((m.desk(bx2, by2).0 - 1920).abs() <= 8 && (m.desk(bx2, by2).1 - 500).abs() <= 8);
    }

    #[test]
    fn slider_maps_position_to_whole_percent() {
        assert_eq!(slider_value(100, 100, 300), 0.0);
        assert_eq!(slider_value(200, 100, 300), 0.5);
        assert_eq!(slider_value(900, 100, 300), 1.0);
        assert_eq!(slider_value(-5, 100, 300), 0.0);
    }

    #[test]
    fn names_and_pages() {
        assert_eq!(os_name("Windows 10 Pro", 26100), "Windows 11 Pro");
        assert_eq!(os_name("Windows 10 Pro", 19045), "Windows 10 Pro");
        assert_eq!(gb(16 << 30), "16.0 GB");
        assert_eq!((page_for("ms-settings:about"), page_for("more"), page_for("ms-settings:display"), page_for("")), (8, 0, 0, 0));
        assert_eq!((page_for("ms-settings:sound"), page_for("ms-settings:personalization-colors")), (1, 7));
        assert_eq!((page_for("bluetooth"), page_for("apps"), page_for("update"), page_for("network")), (BT_PAGE, APPS_PAGE, WU_PAGE, NET_PAGE));
        assert_eq!((page_for("ms-settings:powersleep"), page_for("ms-settings:storagesense")), (2, 2));
    }
}
