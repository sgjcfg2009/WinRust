use super::*;
use std::sync::Mutex;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicI32, Ordering::Relaxed},
};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Quick,
    Wifi,
    Cal,
    Start,
    Widgets,
    Desktops,
    Menu,
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    VolSlide,
    BrightSlide,
    Mute,
    AudioOut,
    AudioPick(usize),
    QuickBack,
    WifiSwitch,
    Tile(u8),
    WifiMore,
    Net(usize),
    NetOff,
    Ctx(u8),
    Settings(&'static str),
    Menu(u8),
    CalPrev,
    CalNext,
    CalToday,
    WidgetTool(u8),
    Desk(u8),
    App(usize),
    StartAll,
    StartCtx(u8),
    Power(u8),
}

struct AudioDev {
    id: String,
    name: String,
    default: bool,
}

#[path = "audio_policy.rs"]
mod audio_policy;
fn set_audio_device(id: &str) {
    let _ = audio_policy::set_default(id);
}

fn audio_devices() -> Vec<AudioDev> {
    use windows::Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::{DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender},
        System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ},
    };
    unsafe {
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return vec![] };
        let cur = en.GetDefaultAudioEndpoint(eRender, eConsole).ok().and_then(|d| d.GetId().ok().and_then(|p| p.to_string().ok()));
        let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return vec![] };
        (0..col.GetCount().unwrap_or(0))
            .filter_map(|i| col.Item(i).ok())
            .filter_map(|d| {
                let id = d.GetId().ok().and_then(|p| p.to_string().ok())?;
                let name = d.OpenPropertyStore(STGM_READ).ok().and_then(|s| s.GetValue(&PKEY_Device_FriendlyName).ok()).map(|v| v.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());
                Some(AudioDev { default: Some(&id) == cur.as_ref(), name, id })
            })
            .collect()
    }
}

fn refresh_audio_devices() {
    AUDIO_DEVS.set(audio_devices());
}

struct Net {
    ssid: String,
    q: i32,
    secure: bool,
    profile: Option<String>,
    connected: bool,
    auth: i32,
    g: windows_sys::core::GUID,
}

struct Conn {
    ssid: String,
    auth: i32,
    q: i32,
    rx: u32,
    tx: u32,
    phy: &'static str,
}

fn auth_name(a: i32) -> &'static str {
    match a {
        1 => "Open",
        2 => "WEP",
        3 => "WPA",
        4 => "WPA‑Personal",
        6 => "WPA2",
        7 => "WPA2‑Personal",
        8 => "WPA3",
        9 => "WPA3‑Personal",
        _ => "Secured",
    }
}

fn phy_name(t: i32) -> &'static str {
    match t {
        7 => "Wi‑Fi 4",
        8 => "Wi‑Fi 5",
        10 => "Wi‑Fi 6",
        11 => "Wi‑Fi 7",
        4..=6 => "802.11a/b/g",
        _ => "Wi‑Fi",
    }
}

static APPS: Mutex<Vec<(String, Vec<u16>)>> = Mutex::new(Vec::new());
static START_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
pub static PREVIEW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static BRIGHT: AtomicI32 = AtomicI32::new(-1);

thread_local! {
    static FLY: Cell<Option<Kind>> = const { Cell::new(None) };
    static FH: Cell<HWND> = const { Cell::new(null_mut()) };
    static OPENING: Cell<bool> = const { Cell::new(false) };
    static CAL: Cell<(i32, i32)> = const { Cell::new((2026, 1)) };
    static NETS: RefCell<Vec<Net>> = const { RefCell::new(Vec::new()) };
    static CONN: RefCell<Option<Conn>> = const { RefCell::new(None) };
    static PASS: RefCell<Option<(usize, String)>> = const { RefCell::new(None) };
    static NOTE: RefCell<String> = const { RefCell::new(String::new()) };
    static CONNECTING: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
    static CTX: RefCell<Option<(String, i32, i32, u64)>> = const { RefCell::new(None) };
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static SCROLL: Cell<usize> = const { Cell::new(0) };
    static SELALL: Cell<bool> = const { Cell::new(false) };
    static ICONS: RefCell<HashMap<(Vec<u16>, i32), HICON>> = RefCell::new(HashMap::new());
    static AUDIO_DEVS: RefCell<Vec<AudioDev>> = const { RefCell::new(Vec::new()) };
    static RECTS_BUF: RefCell<Vec<(RECT, Act)>> = const { RefCell::new(Vec::new()) };
    static PAINT_RECTS: RefCell<Vec<(RECT, Act)>> = const { RefCell::new(Vec::new()) };
    static SEL: Cell<usize> = const { Cell::new(0) };
    static OVER: Cell<Option<Act>> = const { Cell::new(None) };
    static DRAG: Cell<u8> = const { Cell::new(0) };
    static MENU_X: Cell<i32> = const { Cell::new(0) };
    static MENU_SCROLL: Cell<i32> = const { Cell::new(0) };
    static CONFIRM: Cell<i32> = const { Cell::new(-1) };
    static FIN: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    static T0: Cell<u64> = const { Cell::new(0) };
    static JUST: RefCell<(String, u64)> = const { RefCell::new((String::new(), 0)) };
    static QUICK_SUB: Cell<u8> = const { Cell::new(0) };
    static START_ALL: Cell<bool> = const { Cell::new(false) };
    static START_CTX: RefCell<Option<(usize, i32, i32)>> = const { RefCell::new(None) };
    static DEACT_START_MS: Cell<u64> = const { Cell::new(0) };
    static FLY_LBUF: RefCell<LayeredBuf> = const { RefCell::new(LayeredBuf::new()) };
    static FLY_FONTS: Cell<(u32, [HFONT; 6])> = const { Cell::new((0, [null_mut(); 6])) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

fn month_name(m: i32) -> &'static str {
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ][(m.clamp(1, 12) - 1) as usize]
}

fn month_title(y: i32, m: i32) -> String {
    format!("{} {y}", month_name(m))
}

fn weekday_initials() -> [&'static str; 7] {
    [
        "Mo",
        "Tu",
        "We",
        "Th",
        "Fr",
        "Sa",
        "Su",
    ]
}

fn start_program_dirs() -> Vec<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Programs, FOLDERID_PublicDesktop, IShellItem, KF_FLAG_DEFAULT, SHGetKnownFolderItem, SIGDN_FILESYSPATH},
    };
    let mut dirs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |p: PathBuf| {
        if p.is_dir() && seen.insert(p.to_string_lossy().to_lowercase()) {
            dirs.push(p);
        }
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        for fid in [&FOLDERID_CommonPrograms, &FOLDERID_Programs, &FOLDERID_Desktop, &FOLDERID_PublicDesktop] {
            if let Ok(item) = SHGetKnownFolderItem::<IShellItem>(fid, KF_FLAG_DEFAULT, None) {
                if let Ok(pw) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                    if let Ok(s) = pw.to_string() {
                        push(PathBuf::from(s));
                    }
                    CoTaskMemFree(Some(pw.0 as *const _));
                }
            }
        }
    }
    for (var, sub) in [("ProgramData", r"Microsoft\Windows\Start Menu\Programs"), ("APPDATA", r"Microsoft\Windows\Start Menu\Programs")] {
        if let Some(b) = std::env::var_os(var) {
            push(PathBuf::from(b).join(sub));
        }
    }
    dirs
}

fn norm_key(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

fn is_junk_exe(name_lower: &str) -> bool {
    const JUNK: &[&str] = &[
        "uninstall", "unins", "setup", "update", "updater", "crash", "helper", "handler",
        "redist", "vcredist", "dxsetup", "elevate", "createdump", "webengine", "notification",
        "daemon", "reporter", "broker", "svc", "service", "stub", "overlay", "driverquery",
        "browsercore", "imagingdevices", "proxy", "webview", "diag", "troubleshoot", "oalinst",
    ];
    name_lower.is_empty() || JUNK.iter().any(|k| name_lower.contains(k))
}

fn is_gui_exe(path: &std::path::Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut dos = [0u8; 64];
    if f.read_exact(&mut dos).is_err() || &dos[0..2] != b"MZ" {
        return false;
    }
    let pe_off = u32::from_le_bytes([dos[60], dos[61], dos[62], dos[63]]) as u64;
    if f.seek(SeekFrom::Start(pe_off)).is_err() {
        return false;
    }
    let mut pe = [0u8; 96];
    if f.read_exact(&mut pe).is_err() || &pe[0..4] != b"PE\0\0" {
        return false;
    }
    let sub = u16::from_le_bytes([pe[92], pe[93]]);
    sub == 2
}

fn scan_program_roots(v: &mut Vec<(String, Vec<u16>)>, have: &mut std::collections::HashSet<String>, have_exes: &mut std::collections::HashSet<String>) {
    const SKIP_DIRS: &[&str] = &[
        "windowsapps", "common files", "windows nt", "reference assemblies", "msbuild",
        "microsoft.net", "uninstall information", "windows defender", "windows mail",
        "windows media player", "windows photo viewer", "windows powershell",
        "windows security", "windows sidebar", "internet explorer", "modifymodules", "winrust",
    ];
    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_root = |p: PathBuf| {
        if p.is_dir() && seen.insert(p.to_string_lossy().to_ascii_lowercase()) {
            roots.push(p);
        }
    };
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(b) = std::env::var_os(var) {
            push_root(PathBuf::from(b));
        }
    }
    if let Some(b) = std::env::var_os("LOCALAPPDATA") {
        push_root(PathBuf::from(b).join("Programs"));
    }
    for r in roots {
        for e in std::fs::read_dir(&r).into_iter().flatten().flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let dn = dir.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
            if SKIP_DIRS.iter().any(|s| dn == *s) {
                continue;
            }
            let mut candidates: Vec<(String, PathBuf)> = Vec::new();
            for fe in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = fe.path();
                if p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
                    let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    let nl = stem.to_ascii_lowercase();
                    if !is_junk_exe(&nl) && is_gui_exe(&p) {
                        candidates.push((stem, p));
                    }
                }
            }
            if candidates.is_empty() {
                continue;
            }
            let pick = if let Some(main_i) = candidates.iter().position(|(s, _)| s.eq_ignore_ascii_case(&dn)) {
                candidates.swap_remove(main_i)
            } else if candidates.len() == 1 {
                candidates.swap_remove(0)
            } else {
                continue;
            };
            let (stem, p) = pick;
            let p_low = p.to_string_lossy().to_ascii_lowercase();
            let nk = norm_key(&stem);
            if !have_exes.contains(&p_low) && !have_exes.contains(&nk) && have.insert(nk.clone()) {
                have_exes.insert(p_low);
                have_exes.insert(nk);
                v.push((stem, wide(&p.to_string_lossy())));
            }
        }
    }
}

fn start_apps_watcher(dirs: &[PathBuf]) {
    static WATCHING: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if dirs.is_empty() || WATCHING.set(()).is_err() {
        return;
    }
    let watch_dirs = dirs.to_vec();
    std::thread::spawn(move || unsafe {
        use windows_sys::Win32::{
            Storage::FileSystem::{
                FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
                FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
            },
            System::Threading::WaitForMultipleObjects,
        };
        let mask = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE;
        let handles: Vec<HANDLE> = watch_dirs
            .iter()
            .filter_map(|d| {
                let h = FindFirstChangeNotificationW(wide(&d.to_string_lossy()).as_ptr(), 1, mask);
                (h != INVALID_HANDLE_VALUE && !h.is_null()).then_some(h)
            })
            .take(64)
            .collect();
        if handles.is_empty() {
            return;
        }
        loop {
            let r = WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, u32::MAX);
            let idx = r as usize;
            if idx >= handles.len() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = FindNextChangeNotification(handles[idx]);
            load_apps();
        }
        for h in handles {
            FindCloseChangeNotification(h);
        }
    });
}

pub fn load_apps() {
    fn walk(dir: &std::path::Path, depth: u8, out: &mut Vec<(String, Vec<u16>)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 5 {
                walk(&p, depth + 1, out);
            } else if p.extension().is_some_and(|x| {
                x.eq_ignore_ascii_case("lnk") || x.eq_ignore_ascii_case("url") || x.eq_ignore_ascii_case("exe") || x.eq_ignore_ascii_case("appref-ms")
            }) {
                let name = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if !name.is_empty() && !name.to_lowercase().contains("uninstall") {
                    out.push((name, wide(&p.to_string_lossy())));
                }
            }
        }
    }
    let dirs = start_program_dirs();
    start_apps_watcher(&dirs);
    let mut v = Vec::new();
    for d in &dirs {
        walk(d, 0, &mut v);
    }
    let mut have: std::collections::HashSet<String> = std::collections::HashSet::with_capacity(v.len() * 2);
    let mut have_exes: std::collections::HashSet<String> = std::collections::HashSet::with_capacity(v.len() * 2);
    for (name, wpath) in &v {
        have.insert(norm_key(name));
        let p_str = String::from_utf16_lossy(&wpath[..wpath.len().saturating_sub(1)]);
        let tgt = if p_str.to_ascii_lowercase().ends_with(".lnk") {
            unsafe { lnk_target(wpath) }
        } else {
            p_str
        };
        if !tgt.is_empty() {
            let tl = tgt.to_ascii_lowercase();
            if let Some(stem) = std::path::Path::new(&tl).file_stem().and_then(|s| s.to_str()) {
                have_exes.insert(norm_key(stem));
            }
            have_exes.insert(tl);
        }
    }
    for (name, id) in apps_folder() {
        let id_l = id.to_ascii_lowercase();
        let nl = name.to_ascii_lowercase();
        let sys_junk = id_l.contains("\\windows\\system32\\")
            || id_l.contains("\\windows\\syswow64\\")
            || id_l.contains("\\windows\\winsxs\\")
            || id_l.contains("\\winrust\\")
            || id_l.starts_with("microsoft.uev.")
            || is_junk_exe(&nl)
            || nl.contains("controlhost")
            || nl.contains("copilotupdate");
        if sys_junk {
            continue;
        }
        let nk = norm_key(&name);
        let exe_stem = std::path::Path::new(&id_l).file_stem().and_then(|s| s.to_str()).map(norm_key).unwrap_or_default();
        if !have_exes.contains(&id_l) && (exe_stem.is_empty() || !have_exes.contains(&exe_stem)) && have.insert(nk) {
            if !exe_stem.is_empty() {
                have_exes.insert(exe_stem);
            }
            have_exes.insert(id_l);
            v.push((name, wide(&format!(r"shell:AppsFolder\{id}"))));
        }
    }
    scan_program_roots(&mut v, &mut have, &mut have_exes);
    let sys_root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    for (name, target) in [
        ("Control Panel", "shell:::{26EE0668-A00A-44D7-9371-BEB064C98683}".to_string()),
        ("Command Prompt (cmd)", sys_root.join(r"System32\cmd.exe").to_string_lossy().into_owned()),
        ("Windows PowerShell", sys_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe").to_string_lossy().into_owned()),
        ("Registry Editor", sys_root.join("regedit.exe").to_string_lossy().into_owned()),
        ("Task Manager", sys_root.join(r"System32\Taskmgr.exe").to_string_lossy().into_owned()),
        ("Settings", "ms-settings:".to_string()),
    ] {
        if (target.starts_with("ms-") || target.starts_with("shell:") || PathBuf::from(&target).is_file()) && have.insert(name.to_lowercase()) {
            v.push((name.to_string(), wide(&target)));
        }
    }
    v.sort_by_cached_key(|a| a.0.to_lowercase());
    v.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    *APPS.lock().unwrap_or_else(|e| e.into_inner()) = v;
    let hw = START_HWND.load(Relaxed);
    if hw != 0 {
        unsafe { InvalidateRect(hw as HWND, null(), 0) };
    }
}

pub fn apps_folder() -> Vec<(String, String)> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING},
    };
    let mut out = vec![];
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return out };
        let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut items = [None];
            let mut got = 0u32;
            if en.Next(&mut items, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(it) = items[0].take() else { break };
            let name = |k: SIGDN| {
                it.GetDisplayName(k).ok().map(|p| {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const _));
                    s
                })
            };
            if let (Some(n), Some(id)) = (name(SIGDN_NORMALDISPLAY), name(SIGDN_PARENTRELATIVEPARSING)) {
                out.push((n, id));
            }
        }
    }
    out
}

const EN_KEYS: &[u8; 33] = b"qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
const RU_KEYS: [char; 33] = [
    'й', 'ц', 'у', 'к', 'е', 'н', 'г', 'ш', 'щ', 'з', 'х', 'ъ',
    'ф', 'ы', 'в', 'а', 'п', 'р', 'о', 'л', 'д', 'ж', 'э',
    'я', 'ч', 'с', 'м', 'и', 'т', 'ь', 'б', 'ю', 'ё',
];

fn flip_layout(q: &str) -> String {
    q.chars()
        .map(|c| {
            if c.is_ascii() {
                EN_KEYS.iter().position(|&x| x == c as u8).map(|i| RU_KEYS[i]).unwrap_or(c)
            } else {
                RU_KEYS.iter().position(|&x| x == c).map(|i| EN_KEYS[i] as char).unwrap_or(c)
            }
        })
        .collect()
}

fn score(name: &str, q: &str) -> Option<u8> {
    if q.is_empty() {
        return None;
    }
    if name.starts_with(q) {
        return Some(0);
    }
    let words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    if words.clone().any(|w| w.starts_with(q)) {
        return Some(1);
    }
    let mut q_chars = q.chars();
    let q0 = q_chars.next()?;
    if q_chars.clone().next().is_some() {
        let mut init_chars = words.filter_map(|w| w.chars().next());
        if init_chars.next() == Some(q0) && q_chars.zip(init_chars.by_ref()).all(|(a, b)| a == b) {
            let q_len = q.chars().count();
            let w_len = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).count();
            if w_len >= q_len {
                return Some(2);
            }
        }
    }
    name.contains(q).then_some(3)
}

static PINNED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn pin_file() -> PathBuf {
    appdata_file("topbar-start.txt")
}

fn pinned_load() {
    *PINNED.lock().unwrap_or_else(|e| e.into_inner()) = std::fs::read_to_string(pin_file()).map(|s| s.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default();
}

pub fn start_pinned(path: &str) -> bool {
    PINNED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|p| p.eq_ignore_ascii_case(path))
}

unsafe fn on_files(hw: HWND, hd: HDROP) {
    let n = DragQueryFileW(hd, u32::MAX, null_mut(), 0);
    let mut added = vec![];
    for i in 0..n {
        let len = DragQueryFileW(hd, i, null_mut(), 0) as usize;
        let mut b = vec![0u16; len + 1];
        DragQueryFileW(hd, i, b.as_mut_ptr(), b.len() as u32);
        let path = String::from_utf16_lossy(&b[..len]);
        if start_pin(&path) {
            added.push(std::path::Path::new(&path).file_stem().unwrap_or_default().to_string_lossy().to_string());
        }
    }
    DragFinish(hd);
    note(&match added.len() {
        0 => "Already in Start".to_string(),
        1 => format!("Added to Start: {}", added[0]),
        n => format!("Added {n} items to Start"),
    });
    KillTimer(hw, 6);
    InvalidateRect(hw, null(), 0);
    force_fg(hw);
}

pub fn start_pin(path: &str) -> bool {
    if start_pinned(path) {
        return false;
    }
    start_toggle(path);
    JUST.set((path.to_string(), unsafe { now_ms() }));
    true
}

pub fn is_open(k: Kind) -> bool {
    FLY.get() == Some(k)
}

pub fn any_open() -> bool {
    FLY.get().is_some()
}

pub fn note(s: &str) {
    NOTE.set(s.to_string());
}

pub fn start_toggle(path: &str) {
    let mut p = PINNED.lock().unwrap_or_else(|e| e.into_inner());
    match p.iter().position(|x| x.eq_ignore_ascii_case(path)) {
        Some(i) => drop(p.remove(i)),
        None => p.push(path.to_string()),
    }
    let _ = std::fs::write(pin_file(), p.join("\n"));
}

fn filtered() -> Vec<usize> {
    let q = QUERY.with_borrow(|q| q.to_lowercase());
    let mut apps = APPS.lock().unwrap_or_else(|e| e.into_inner());
    if q.is_empty() {
        if START_ALL.get() {
            return (0..apps.len()).collect();
        }
        let pinned = PINNED.lock().unwrap_or_else(|e| e.into_inner());
        return pinned
            .iter()
            .map(|p| {
                let w = wide(p);
                apps.iter().position(|a| a.1 == w).unwrap_or_else(|| {
                    let stem = std::path::Path::new(p).file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let name: String = stem.chars().take(1).flat_map(char::to_uppercase).chain(stem.chars().skip(1)).collect();
                    apps.push((name, w));
                    apps.len() - 1
                })
            })
            .collect();
    }
    let fq = flip_layout(&q);
    let mut v: Vec<(u8, u32, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let n = a.0.to_lowercase();
            let s = score(&n, &q).or_else(|| score(&n, &fq).map(|s| s + 4))?;
            Some((s, u32::MAX - uses(&n), i))
        })
        .collect();
    v.sort_unstable();
    v.into_iter().map(|x| x.2).collect()
}

fn lcd_io(code: u32, input: Option<[u8; 3]>) -> Option<[u8; 3]> {
    use windows_sys::Win32::{
        Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
        System::IO::DeviceIoControl,
    };
    unsafe {
        let h = CreateFileW(wide(r"\\.\LCD").as_ptr(), 0xC000_0000, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let (mut out, mut n) = ([0u8; 3], 0u32);
        let ok = match input {
            Some(i) => DeviceIoControl(h, code, i.as_ptr().cast(), 3, null_mut(), 0, &mut n, null_mut()),
            None => DeviceIoControl(h, code, null(), 0, out.as_mut_ptr().cast(), 3, &mut n, null_mut()),
        } != 0;
        CloseHandle(h);
        ok.then_some(out)
    }
}

fn read_brightness() {
    std::thread::spawn(|| {
        let ac = unsafe {
            let mut s: windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS = zeroed();
            windows_sys::Win32::System::Power::GetSystemPowerStatus(&mut s) != 0 && s.ACLineStatus == 1
        };
        let v = lcd_io(0x23_0498, None).map(|b| if ac { b[1] } else { b[2] } as i32).filter(|v| *v <= 100).or_else(|| {
            let o = Command::new("powershell").args(["-NoProfile", "-Command", "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue).CurrentBrightness"]).creation_flags(NO_WINDOW).output();
            o.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        });
        BRIGHT.store(v.unwrap_or(-1), Relaxed);
    });
}

static BRIGHT_REQ: (Mutex<Option<i32>>, std::sync::Condvar) = (Mutex::new(None), std::sync::Condvar::new());
static BRIGHT_WORKER: std::sync::OnceLock<()> = std::sync::OnceLock::new();

fn set_monitor_brightness_dxva2(b: u32) -> bool {
    use windows_sys::Win32::{
        Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
        System::LibraryLoader::{GetProcAddress, LoadLibraryW},
    };
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PhysMon {
        h: HANDLE,
        desc: [u16; 128],
    }
    unsafe {
        let lib = LoadLibraryW(wide("dxva2.dll").as_ptr());
        if lib.is_null() {
            return false;
        }
        let get_n = GetProcAddress(lib, b"GetNumberOfPhysicalMonitorsFromHMONITOR\0".as_ptr());
        let get_m = GetProcAddress(lib, b"GetPhysicalMonitorsFromHMONITOR\0".as_ptr());
        let set_b = GetProcAddress(lib, b"SetMonitorBrightness\0".as_ptr());
        let dst_m = GetProcAddress(lib, b"DestroyPhysicalMonitors\0".as_ptr());
        let mut ok = false;
        if let (Some(gn), Some(gm), Some(sb), Some(dm)) = (get_n, get_m, set_b, dst_m) {
            let gn: unsafe extern "system" fn(HMONITOR, *mut u32) -> i32 = std::mem::transmute(gn);
            let gm: unsafe extern "system" fn(HMONITOR, u32, *mut PhysMon) -> i32 = std::mem::transmute(gm);
            let sb: unsafe extern "system" fn(HANDLE, u32) -> i32 = std::mem::transmute(sb);
            let dm: unsafe extern "system" fn(u32, *const PhysMon) -> i32 = std::mem::transmute(dm);
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut cnt = 0u32;
            if gn(mon, &mut cnt) != 0 && cnt > 0 {
                let mut mons = vec![PhysMon { h: null_mut(), desc: [0; 128] }; cnt as usize];
                if gm(mon, cnt, mons.as_mut_ptr()) != 0 {
                    for pm in &mons {
                        if sb(pm.h, b) != 0 {
                            ok = true;
                        }
                    }
                    dm(cnt, mons.as_ptr());
                }
            }
        }
        FreeLibrary(lib);
        ok
    }
}

fn set_brightness(v: i32) {
    BRIGHT_WORKER.get_or_init(|| {
        std::thread::spawn(|| {
            let (lock, cv) = &BRIGHT_REQ;
            let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let Some(val) = g.take() else {
                    g = cv.wait(g).unwrap_or_else(|e| e.into_inner());
                    continue;
                };
                drop(g);
                let b = val.clamp(0, 100) as u8;
                if lcd_io(0x23_049C, Some([3, b, b])).is_none() && !set_monitor_brightness_dxva2(b as u32) {
                    let c = format!("Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=1;Brightness={val}}}");
                    let _ = Command::new("powershell").args(["-NoProfile", "-Command", &c]).creation_flags(NO_WINDOW).output();
                }
                g = lock.lock().unwrap_or_else(|e| e.into_inner());
            }
        });
    });
    *BRIGHT_REQ.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
    BRIGHT_REQ.1.notify_one();
}

unsafe fn for_each_wlan(mut f: impl FnMut(HANDLE, &windows_sys::core::GUID)) {
    let (mut h, mut ver) = (null_mut(), 0u32);
    if WlanOpenHandle(2, null(), &mut ver, &mut h) != 0 {
        return;
    }
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    if WlanEnumInterfaces(h, null(), &mut list) == 0 {
        let l = &*list;
        for i in 0..l.dwNumberOfItems as usize {
            f(h, &(*l.InterfaceInfo.as_ptr().add(i)).InterfaceGuid);
        }
        WlanFreeMemory(list as *const _);
    }
    WlanCloseHandle(h, null());
}

unsafe fn with_iface<T>(g: &windows_sys::core::GUID, f: impl FnOnce(HANDLE, &windows_sys::core::GUID) -> T) -> Option<T> {
    let (mut h, mut ver) = (null_mut(), 0u32);
    if WlanOpenHandle(2, null(), &mut ver, &mut h) != 0 {
        return None;
    }
    let r = f(h, g);
    WlanCloseHandle(h, null());
    Some(r)
}

unsafe fn read_one(h: HANDLE, g: &windows_sys::core::GUID) -> (Vec<Net>, Option<Conn>) {
    {
        let mut out: Vec<Net> = Vec::new();
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = null_mut();
        if WlanGetAvailableNetworkList(h, g, 0, null(), &mut list) == 0 {
            let l = &*list;
            for i in 0..l.dwNumberOfItems as usize {
                let n = &*l.Network.as_ptr().add(i);
                let ssid = String::from_utf8_lossy(&n.dot11Ssid.ucSSID[..n.dot11Ssid.uSSIDLength as usize]).to_string();
                if ssid.is_empty() {
                    continue;
                }
                let pn = String::from_utf16_lossy(&n.strProfileName[..n.strProfileName.iter().position(|&c| c == 0).unwrap_or(0)]);
                let net = Net { ssid, q: n.wlanSignalQuality as i32, secure: n.bSecurityEnabled != 0, profile: (n.dwFlags & 2 != 0 && !pn.is_empty()).then_some(pn), connected: n.dwFlags & 1 != 0, auth: n.dot11DefaultAuthAlgorithm as i32, g: *g };
                match out.iter_mut().find(|o| o.ssid == net.ssid) {
                    Some(o) if o.q < net.q => *o = net,
                    Some(_) => {}
                    None => out.push(net),
                }
            }
            WlanFreeMemory(list as *const _);
        }
        out.sort_by_key(|n| (!n.connected, -n.q));
        let (mut size, mut data) = (0u32, null_mut());
        let conn = (WlanQueryInterface(h, g, wlan_intf_opcode_current_connection, null(), &mut size, &mut data, null_mut()) == 0).then(|| {
            let at = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
            let a = &at.wlanAssociationAttributes;
            let ssid = String::from_utf8_lossy(&a.dot11Ssid.ucSSID[..a.dot11Ssid.uSSIDLength as usize]).to_string();
            let c = Conn { ssid, auth: at.wlanSecurityAttributes.dot11AuthAlgorithm as i32, q: a.wlanSignalQuality as i32, rx: a.ulRxRate / 1000, tx: a.ulTxRate / 1000, phy: phy_name(a.dot11PhyType as i32) };
            WlanFreeMemory(data);
            c
        });
        if let Some(c) = &conn {
            match out.iter().position(|n| n.ssid == c.ssid) {
                Some(i) => {
                    out[i].connected = true;
                    out.swap(0, i);
                }
                None => out.insert(0, Net { ssid: c.ssid.clone(), q: c.q, secure: c.auth != 1, profile: Some(c.ssid.clone()), connected: true, auth: c.auth, g: *g }),
            }
        }
        (out, conn)
    }
}

unsafe fn read_nets() {
    let mut nets: Vec<Net> = vec![];
    let mut conn: Option<Conn> = None;
    for_each_wlan(|h, g| {
        let (o, c) = read_one(h, g);
        for n in o {
            match nets.iter_mut().find(|x| x.ssid == n.ssid) {
                Some(x) if (n.connected && !x.connected) || (!x.connected && x.q < n.q) => *x = n,
                Some(_) => {}
                None => nets.push(n),
            }
        }
        if conn.is_none() {
            conn = c;
        }
    });
    nets.sort_by_key(|n| (!n.connected, -n.q));
    nets.truncate(7);
    NETS.set(nets);
    CONN.set(conn);
    let on = NETS.with_borrow(|n| n.iter().find(|n| n.connected).map(|n| n.ssid.clone()));
    if let Some((ssid, t0)) = CONNECTING.with_borrow(|c| c.clone()) {
        if on.as_deref() == Some(ssid.as_str()) {
            CONNECTING.set(None);
            NOTE.set(String::new());
        } else if now_ms().saturating_sub(t0) > 25_000 {
            CONNECTING.set(None);
            NOTE.set(format!("Could not connect to “{ssid}”"));
        }
    }
}

unsafe fn start_connecting(ssid: &str) {
    CONNECTING.set(Some((ssid.to_string(), now_ms())));
    NOTE.set(String::new());
}

unsafe fn forget(ssid: &str) {
    let Some(p) = NETS.with_borrow(|n| n.iter().find(|n| n.ssid == ssid).and_then(|n| n.profile.clone())) else { return };
    let w = wide(&p);
    let mut ok = false;
    for_each_wlan(|h, g| ok |= WlanDeleteProfile(h, g, w.as_ptr(), null()) == 0);
    NOTE.set(if ok { format!("Forgot “{ssid}”") } else { format!("Could not forget “{ssid}”") });
    read_nets();
}

unsafe fn wifi_connect(profile: &str, g: &windows_sys::core::GUID) {
    let p = wide(profile);
    with_iface(g, |h, g| {
        let params = WLAN_CONNECTION_PARAMETERS { wlanConnectionMode: wlan_connection_mode_profile, strProfile: p.as_ptr(), pDot11Ssid: null_mut(), pDesiredBssidList: null_mut(), dot11BssType: dot11_BSS_type_any, dwFlags: 0 };
        WlanConnect(h, g, &params, null());
    });
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

unsafe fn wifi_join(n: &Net, pass: &str) {
    let (auth, key) = match n.auth {
        1 => ("open", String::new()),
        4 => ("WPAPSK", pass.into()),
        7 => ("WPA2PSK", pass.into()),
        _ => ("WPA3SAE", pass.into()),
    };
    let sec = if auth == "open" {
        "<authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption>".to_string()
    } else {
        format!("<authEncryption><authentication>{auth}</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption><sharedKey><keyType>passphrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>", esc(&key))
    };
    let name = esc(&n.ssid);
    let xml = format!("<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{name}</name><SSIDConfig><SSID><name>{name}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security>{sec}</security></MSM></WLANProfile>");
    let x = wide(&xml);
    let mut reason = 0u32;
    let ok = with_iface(&n.g, |h, g| WlanSetProfile(h, g, 0, x.as_ptr(), null(), 1, null_mut(), &mut reason) == 0).unwrap_or(false);
    if ok {
        wifi_connect(&n.ssid, &n.g);
        start_connecting(&n.ssid);
    } else {
        NOTE.set("Could not save the network (wrong password format?)".into());
    }
}

fn start_rows() -> usize {
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let max_h = ((sh - bar_h() - sc(24)) as f32 / SCALE.get()).floor() as i32;
    let cap = match i18n::style::start_size() { 0 => 6, 2 => 12, _ => 9 };
    ((max_h - 136) / 40).clamp(5, cap) as usize
}

fn max_menu_h() -> i32 {
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    (((sh - bar_h() - sc(24)) as f32 / SCALE.get()).floor() as i32).max(200)
}

fn quick_base_size() -> (i32, i32) {
    let n = sliders();
    let sl_h = if n == 2 { 86 } else { 42 };
    let foot_top = 188 + sl_h + 10;
    (304, foot_top + 44)
}

fn quick_fit_scale() -> f32 {
    let (bw, bh) = quick_base_size();
    let s = SCALE.get();
    unsafe {
        let mut wa: RECT = zeroed();
        let (aw, ah) = if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as *mut _, 0) != 0 {
            ((wa.right - wa.left).max(240), (wa.bottom - wa.top).max(220))
        } else {
            (GetSystemMetrics(SM_CXSCREEN).max(240), GetSystemMetrics(SM_CYSCREEN).max(220))
        };
        let max_w = (aw - sc(16)).max(220) as f32;
        let max_h = (ah - sc(64)).max(200) as f32;
        s.min(max_w / bw as f32).min(max_h / bh as f32).max(0.55)
    }
}

fn qsc(v: i32, qs: f32) -> i32 {
    (v as f32 * qs).round() as i32
}

fn qr(l: i32, t: i32, rr: i32, b: i32, qs: f32) -> RECT {
    RECT { left: qsc(l, qs), top: qsc(t, qs), right: qsc(rr, qs), bottom: qsc(b, qs) }
}

fn size(k: Kind) -> (i32, i32) {
    match k {
        Kind::Quick => quick_base_size(),
        Kind::Cal => (330, 340),
        Kind::Wifi => (360, wifi_rows().2),
        Kind::Start => {
            let sw = match i18n::style::start_size() { 0 => 480, 2 => 640, _ => 560 };
            (sw, 136 + start_rows() as i32 * 40)
        }
        Kind::Widgets => (380, 404),
        Kind::Desktops => (336, 114),
        Kind::Menu => (MW, menu_rows().1.min(max_menu_h())),
    }
}

fn wifi_rows() -> (Vec<(i32, i32)>, Option<i32>, i32) {
    NETS.with_borrow(|n| {
        let (mut y, mut rows, mut label) = (64, vec![], None);
        for net in n.iter() {
            if net.connected {
                rows.push((y, y + 60));
                y += 74;
            } else {
                if label.is_none() {
                    label = Some(y);
                    y += 26;
                }
                rows.push((y, y + 46));
                y += 46;
            }
        }
        (rows, label, y + if n.is_empty() { 70 } else { 10 } + 44)
    })
}

fn sliders() -> i32 {
    if BRIGHT.load(Relaxed) >= 0 { 2 } else { 1 }
}

const CTX_W: i32 = 196;
const CTX_ROW: i32 = 34;

fn ctx_items(ssid: &str) -> Vec<(&'static str, u8)> {
    NETS.with_borrow(|n| {
        let Some(n) = n.iter().find(|n| n.ssid == ssid) else { return vec![] };
        let mut v = vec![if n.connected { ("Disconnect", 1) } else { ("Connect", 0) }];
        if n.profile.is_some() {
            v.push(("Forget network", 2));
        }
        v.push(("Known networks…", 3));
        v
    })
}

fn ctx_rects() -> Vec<(RECT, Act)> {
    CTX.with_borrow(|c| match c {
        Some((ssid, x, y, _)) => (0..ctx_items(ssid).len() as i32).map(|j| (RECT { left: x + sc(6), top: y + sc(6 + CTX_ROW * j), right: x + sc(CTX_W - 6), bottom: y + sc(6 + CTX_ROW * (j + 1)) }, Act::Ctx(j as u8))).collect(),
        None => vec![],
    })
}

fn r(l: i32, t: i32, rr: i32, b: i32) -> RECT {
    RECT { left: sc(l), top: sc(t), right: sc(rr), bottom: sc(b) }
}

fn rects_into(k: Kind, v: &mut Vec<(RECT, Act)>) {
    v.clear();
    let (w, h) = size(k);
    match k {
        Kind::Quick => {
            let (w, _h) = quick_base_size();
            let qs = quick_fit_scale();
            let sub = QUICK_SUB.get();
            if sub == 1 {
                v.push((qr(12, 12, 44, 44, qs), Act::QuickBack));
                let dev_n = AUDIO_DEVS.with_borrow(|d| d.len()).min(5);
                for i in 0..dev_n {
                    let y = 56 + i as i32 * 40;
                    v.push((qr(12, y, w - 12, y + 36, qs), Act::AudioPick(i)));
                }
                let (_, h) = quick_base_size();
                v.push((qr(12, h - 40, w - 12, h - 6, qs), Act::Settings("ms-settings:sound")));
                return;
            }
            if sub == 2 {
                v.push((qr(12, 12, 44, 44, qs), Act::QuickBack));
                v.push((qr(w - 58, 16, w - 16, 40, qs), Act::WifiSwitch));
                let (_, h) = quick_base_size();
                if PASS.with_borrow(|p| p.is_none()) {
                    let max_n = ((h - 104) / 44).max(1) as usize;
                    let cnt = NETS.with_borrow(|n| n.len()).min(max_n);
                    for i in 0..cnt {
                        let y = 56 + i as i32 * 44;
                        v.push((qr(12, y, w - 12, y + 40, qs), Act::Net(i)));
                    }
                }
                v.push((qr(12, h - 40, w - 12, h - 6, qs), Act::Settings("ms-settings:network-wifi")));
                return;
            }
            let btn_w = (w - 32 - 12) / 2;
            for i in 0..4u8 {
                let (tl, tt) = (16 + (btn_w + 12) * (i as i32 % 2), 16 + 80 * (i as i32 / 2));
                if i == 0 {
                    let mid = tl + btn_w / 2;
                    v.push((qr(tl, tt, mid, tt + 48, qs), Act::Tile(0)));
                    v.push((qr(mid, tt, tl + btn_w, tt + 48, qs), Act::WifiMore));
                } else {
                    v.push((qr(tl, tt, tl + btn_w, tt + 48, qs), Act::Tile(i)));
                }
            }
            let n = sliders();
            let mut y = 188;
            if BRIGHT.load(Relaxed) >= 0 {
                v.push((qr(52, y, w - 18, y + 40, qs), Act::BrightSlide));
                y += 44;
            }
            v.push((qr(14, y, 48, y + 40, qs), Act::Mute));
            v.push((qr(52, y, w - 58, y + 40, qs), Act::VolSlide));
            v.push((qr(w - 54, y + 2, w - 14, y + 38, qs), Act::AudioOut));
            let sl_h = if n == 2 { 86 } else { 42 };
            let foot_top = 188 + sl_h + 10;
            v.push((qr(w - 48, foot_top + 5, w - 12, foot_top + 39, qs), Act::Settings("ms-settings:")));
        }
        Kind::Cal => {
            v.push((r(w - 84, 10, w - 52, 42), Act::CalPrev));
            v.push((r(w - 48, 10, w - 16, 42), Act::CalNext));
            v.push((r(16, 10, w - 100, 42), Act::CalToday));
        }
        Kind::Wifi => {
            let rows = wifi_rows().0;
            v.extend(rows.iter().enumerate().map(|(i, &(t, b))| (r(12, t, w - 12, b), Act::Net(i))));
            if let Some(i) = NETS.with_borrow(|n| n.iter().position(|n| n.connected)) {
                let (t, b) = rows[i];
                v.push((r(w - 12 - 108, t + 14, w - 24, b - 14), Act::NetOff));
            }
            v.push((r(w - 58, 18, w - 16, 42), Act::WifiSwitch));
            v.push((r(12, h - 44, w - 12, h - 8), Act::Settings("ms-settings:network-wifi")));
            v.extend(ctx_rects());
        }
        Kind::Widgets => {
            for i in 0..3u8 {
                let x = 16 + i as i32 * 120;
                v.push((r(x, 308, x + 108, 384), Act::WidgetTool(i)));
            }
        }
        Kind::Desktops => {
            for i in 0..4u8 {
                let x = 14 + i as i32 * 78;
                v.push((r(x, 42, x + 70, 100), Act::Desk(i)));
            }
        }
        Kind::Menu => {
            let ms = MENU_SCROLL.get();
            let top_clip = if MHEAD.with_borrow(|hd| hd.is_some()) { MHEAD_H + 4 } else { 3 };
            MITEMS.with_borrow(|m| {
                for (i, (t, b)) in menu_rows().0.into_iter().enumerate() {
                    if m[i].head { continue; }
                    let (t2, b2) = (t - ms, b - ms);
                    if b2 <= top_clip || t2 >= h { continue; }
                    v.push((r(4, t2, w - 4, b2), Act::Menu(i as u8)));
                }
            });
        }
        Kind::Start => {
            let f = filtered();
            let s = SCROLL.get();
            let q_empty = QUERY.with_borrow(|q| q.is_empty());
            if q_empty {
                v.push((r(w - 114, 68, w - 22, 94), Act::StartAll));
            }
            if !q_empty || START_ALL.get() {
                let list_top = if q_empty { 100 } else { 74 };
                let max_r = ((h - 68 - list_top) / 38).max(1) as usize;
                v.extend((0..max_r).filter(|i| s + i < f.len()).map(|i| (r(16, list_top + 38 * i as i32, w - 16, list_top + 38 * (i as i32 + 1)), Act::App(s + i))));
            } else {
                let cols = 6usize;
                let cw = (w - 44) / cols as i32;
                let ch = 78;
                let max_rows = ((h - 68 - 102) / ch).clamp(1, 3) as usize;
                let cap = (cols * max_rows).min(f.len());
                for i in 0..cap {
                    let (col, row) = (i % cols, i / cols);
                    let x = 22 + col as i32 * cw;
                    let y = 102 + row as i32 * ch;
                    v.push((r(x, y, x + cw, y + ch), Act::App(i)));
                }
            }
            v.extend((0..4u8).map(|i| {
                let x = w - 24 - 4 * 34 + 34 * i as i32;
                (r(x, h - 46, x + 30, h - 14), Act::Power(i))
            }));
            if let Some((idx, mx, my)) = START_CTX.with_borrow(|c| *c) {
                let n = start_ctx_items(idx).len() as i32;
                for j in 0..n {
                    v.push((RECT { left: mx + sc(4), top: my + sc(6 + 32 * j), right: mx + sc(216), bottom: my + sc(6 + 32 * (j + 1)) }, Act::StartCtx(j as u8)));
                }
            }
        }
    }
}

fn act_at(k: Kind, x: i32, y: i32) -> Option<(RECT, Act)> {
    RECTS_BUF.with_borrow_mut(|v| {
        rects_into(k, v);
        v.iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).copied()
    })
}

pub fn init(hi: HINSTANCE) {
    pinned_load();
    unsafe {
        let class = wide("topbar_fly");
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: class.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);
    }
}

pub struct MItem {
    pub icon: HICON,
    pub glyph: &'static str,
    pub label: String,
    pub hint: &'static str,
    pub on: bool,
    pub danger: bool,
    pub sep: bool,
    pub head: bool,
    pub beta: bool,
    pub swatch: u32,
}

impl MItem {
    pub fn head(label: impl Into<String>) -> Self {
        MItem { head: true, ..MItem::new(label) }
    }
    pub fn swatch(mut self, c: u32) -> Self {
        self.swatch = c;
        self
    }
    pub fn glyph(mut self, g: &'static str) -> Self {
        self.glyph = g;
        self
    }
    pub fn new(label: impl Into<String>) -> Self {
        MItem { icon: null_mut(), glyph: "", label: label.into(), hint: "", on: false, danger: false, sep: false, head: false, beta: false, swatch: u32::MAX }
    }
    pub fn hint(mut self, h: &'static str) -> Self {
        self.hint = h;
        self
    }
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn sep(mut self) -> Self {
        self.sep = true;
        self
    }
}

type Pick = Box<dyn FnOnce(usize)>;
thread_local! {
    static MITEMS: RefCell<Vec<MItem>> = const { RefCell::new(Vec::new()) };
    static MHEAD: RefCell<Option<(HICON, &'static str, String, String)>> = const { RefCell::new(None) };
    static MPICK: RefCell<Option<Pick>> = const { RefCell::new(None) };
}
const MW: i32 = 224;
const MROW: i32 = 28;
const MHEAD_H: i32 = 44;

pub fn menu(cx: i32, head: Option<(HICON, &'static str, String, String)>, items: Vec<MItem>, pick: impl FnOnce(usize) + 'static) {
    if is_open(Kind::Menu) {
        close();
    }
    MITEMS.set(items);
    MHEAD.set(head);
    MPICK.set(Some(Box::new(pick)));
    MENU_SCROLL.set(0);
    MENU_X.set(if vert() { cx } else { cx - sc(MW) / 2 });
    toggle(Kind::Menu);
}

fn menu_rows() -> (Vec<(i32, i32)>, i32) {
    let mut y = if MHEAD.with_borrow(|h| h.is_some()) { MHEAD_H + 6 } else { 4 };
    let rows = MITEMS.with_borrow(|m| {
        m.iter()
            .map(|it| {
                if it.sep {
                    y += 6;
                }
                let h = if it.head { 22 } else { MROW };
                y += h;
                (y - h, y)
            })
            .collect()
    });
    (rows, y + 4)
}

pub fn context_menu(x: i32) {
    let pu = crate::purge_installed();
    let mut items = vec![
        MItem::new("Taskbar settings").glyph("\u{E713}"),
        MItem::new("Windows Settings").glyph("\u{E770}").hint("Win+I"),
        MItem::new("Personalization").glyph("\u{E771}"),
        MItem::new("Task Manager").glyph("\u{E9D9}").hint("Ctrl+Shift+Esc").sep(),
    ];
    if pu {
        items.push(MItem::new("Remove Windows junk (irreversible)").glyph("\u{E74D}").danger().sep());
    }
    menu(x, None, items, move |i| unsafe {
        match i {
            0 => crate::settings::style_menu(),
            1 => open(&wide("ms-settings:")),
            2 => open(&wide("ms-settings:personalization")),
            3 => crate::open_taskmgr(),
            4 if pu => crate::open_purge(),
            _ => {}
        }
    });
}

pub fn close() {
    let h = FH.get();
    FLY.set(None);
    START_HWND.store(0, Relaxed);
    if !h.is_null() && unsafe { IsWindow(h) != 0 } {
        unsafe {
            KillTimer(h, 1);
            KillTimer(h, 2);
            ShowWindow(h, SW_HIDE);
        }
    }
}

pub fn toggle(k: Kind) {
    if FLY.get() == Some(k) && unsafe { IsWindowVisible(FH.get()) } != 0 {
        return close();
    }
    if k == Kind::Start && unsafe { now_ms() }.saturating_sub(DEACT_START_MS.get()) < 220 {
        return;
    }
    FLY.set(Some(k));
    if k != Kind::Start {
        START_HWND.store(0, Relaxed);
    }
    unsafe {
        CTX.set(None);
        QUERY.set(String::new());
        SELALL.set(false);
        SCROLL.set(0);
        SEL.set(0);
        PASS.set(None);
        NOTE.set(String::new());
        CONFIRM.set(-1);
        OVER.set(None);
        let mut t: SYSTEMTIME = zeroed();
        GetLocalTime(&mut t);
        CAL.set((t.wYear as i32, t.wMonth as i32));
        match k {
            Kind::Wifi => {
                for_each_wlan(|h, g| _ = WlanScan(h, g, null(), null(), null()));
                read_nets();
            }
            Kind::Quick => {
                QUICK_SUB.set(0);
                read_volume();
                read_brightness();
                read_nets();
                refresh_audio_devices();
                crate::quick::refresh();
            }
            Kind::Start => {
                START_ALL.set(false);
                START_CTX.set(None);
                if APPS.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                    std::thread::spawn(load_apps);
                }
            }
            _ => {}
        }
        let (bw, bh) = size(k);
        let s_eff = if k == Kind::Quick { quick_fit_scale() } else { winrust_common::ui::fit_scale(bw, bh, SCALE.get(), sc(16), sc(64)) };
        let (mut w, mut h) = ((bw as f32 * s_eff).round() as i32, (bh as f32 * s_eff).round() as i32);
        let wa = winrust_common::ui::work_area();
        let (sw, sh) = ((wa.right - wa.left).max(240), (wa.bottom - wa.top).max(220));
        w = w.min((sw - sc(24)).max(sc(220)));
        h = h.min((sh - sc(24)).max(sc(200)));
        let x = if vert() {
            side_x(w).clamp(wa.left + sc(4), (wa.right - w - sc(4)).max(wa.left + sc(4)))
        } else {
            match k {
                Kind::Start | Kind::Desktops => wa.left + (sw - w) / 2,
                Kind::Widgets => wa.left + sc(12),
                Kind::Menu => MENU_X.get().clamp(wa.left + sc(12), (wa.right - w - sc(12)).max(wa.left + sc(12))),
                _ => (wa.right - w - sc(12)).max(wa.left + sc(12)),
            }
        };
        let class = wide("topbar_fly");
        let fy = if vert() && k == Kind::Menu {
            (MENU_X.get() - h / 2).clamp(wa.top + sc(12), (wa.bottom - h - sc(12)).max(wa.top + sc(12)))
        } else {
            pop_y(h).clamp(wa.top + sc(12), (wa.bottom - h - sc(12)).max(wa.top + sc(12)))
        };
        let mut hw = FH.get();
        if hw.is_null() || IsWindow(hw) == 0 {
            let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED;
            hw = CreateWindowExW(ex, class.as_ptr(), null(), WS_POPUP, x, fy, w, h, null_mut(), null_mut(), GetModuleHandleW(null()), null());
            for m in [WM_DROPFILES, 0x49, WM_COPYDATA] {
                ChangeWindowMessageFilterEx(hw, m, MSGFLT_ALLOW, null_mut());
            }
            FH.set(hw);
        }
        DragAcceptFiles(hw, (k == Kind::Start) as i32);
        START_HWND.store(if k == Kind::Start { hw as isize } else { 0 }, Relaxed);
        FIN.set((x, fy));
        T0.set(now_ms());
        OPENING.set(true);
        SetWindowPos(hw, null_mut(), x, fy, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        let sdc = GetDC(null_mut());
        paint_layer(hw, k, sdc);
        ReleaseDC(null_mut(), sdc);
        ValidateRect(hw, null());
        SetWindowPos(hw, HWND_TOPMOST, x, fy, w, h, SWP_SHOWWINDOW | SWP_NOACTIVATE);
        if crate::FULLSCREEN.load(Relaxed) {
            SetForegroundWindow(hw);
            SetFocus(hw);
        } else {
            force_fg(hw);
        }
        OPENING.set(false);
        SetTimer(hw, 1, 1000, None);
    }
}

unsafe fn slide(a: Act, x: i32, rect: RECT) {
    let f = ((x - rect.left - sc(8)) as f32 / (rect.right - rect.left - sc(16)) as f32).clamp(0.0, 1.0);
    if a == Act::VolSlide {
        set_volume((f * 100.0).round() as i32);
    } else {
        BRIGHT.store(((f * 100.0) as i32).max(1), Relaxed);
    }
}

unsafe fn wifi_radio(on: bool) {
    crate::quick::set_wifi(on, move || unsafe { wifi_radio_wlan(on) });
}

unsafe fn wifi_radio_wlan(on: bool) {
    for_each_wlan(|h, g| {
        let s = WLAN_PHY_RADIO_STATE { dwPhyIndex: 0, dot11SoftwareRadioState: if on { dot11_radio_state_on } else { dot11_radio_state_off }, dot11HardwareRadioState: dot11_radio_state_on };
        WlanSetInterface(h, g, wlan_intf_opcode_radio_state, size_of::<WLAN_PHY_RADIO_STATE>() as u32, &s as *const _ as *const _, null());
    });
}

unsafe fn launch(i: usize) -> bool {
    let Some(app) = filtered().get(i).and_then(|&ai| APPS.lock().unwrap_or_else(|e| e.into_inner()).get(ai).cloned()) else { return false };
    if wstr_of(&app.1).starts_with("shell:") {
        bump_name(&app.0);
    }
    open(&app.1);
    true
}

fn start_ctx_items(i: usize) -> Vec<(&'static str, &'static str, u8)> {
    let Some(lnk) = filtered().get(i).and_then(|&ai| APPS.lock().unwrap_or_else(|e| e.into_inner()).get(ai).map(|a| a.1.clone())) else { return vec![] };
    let path = wstr_of(&lnk);
    let file = !path.starts_with("shell:");
    let pin_label = if start_pinned(&path) { "Remove from Start" } else { "Add to Start" };
    let mut v = vec![("\u{E8E5}", "Open", 1)];
    if file {
        v.push(("\u{E7EF}", "Run as administrator", 2));
        v.push(("\u{E8DA}", "Open file location", 3));
    }
    v.push(("\u{E718}", pin_label, 4));
    v
}

unsafe fn app_menu(hw: HWND, i: usize, x: i32, y: i32) {
    let items = start_ctx_items(i);
    if items.is_empty() {
        return;
    }
    let mut cr: RECT = zeroed();
    GetClientRect(hw, &mut cr);
    let (pw, ph) = (cr.right.max(sc(240)), cr.bottom.max(sc(200)));
    let mh = sc(12 + 32 * items.len() as i32);
    let mx = x.min(pw - sc(224)).max(sc(8));
    let my = y.min(ph - mh - sc(8)).max(sc(8));
    START_CTX.set(Some((i, mx, my)));
}

unsafe fn run_act(a: Act, rect: RECT, x: i32) {
    match a {
        Act::VolSlide | Act::BrightSlide => {
            DRAG.set(if a == Act::VolSlide { 1 } else { 2 });
            SetCapture(FH.get());
            slide(a, x, rect);
        }
        Act::WifiMore => {
            if FLY.get() == Some(Kind::Quick) {
                for_each_wlan(|h, g| _ = WlanScan(h, g, null(), null(), null()));
                read_nets();
                PASS.set(None);
                QUICK_SUB.set(2);
            } else {
                toggle(Kind::Wifi);
            }
        }
        Act::QuickBack => {
            PASS.set(None);
            QUICK_SUB.set(0);
        }
        Act::WifiSwitch => wifi_radio(WIFI.get() < 0 && NETS.with_borrow(|n| n.is_empty())),
        Act::Tile(i) => match i {
            0 => wifi_radio(WIFI.get() < 0),
            1 => match crate::quick::bluetooth() {
                Some(on) => crate::quick::set(false, !on),
                None => {
                    crate::quick::refresh();
                    NOTE.set("No Bluetooth state, opening Settings".into());
                    open(&wide("ms-settings:bluetooth"));
                    close();
                }
            },
            2 => crate::quick::set(true, !crate::quick::airplane().unwrap_or(false)),
            _ => crate::quick::set_energy_saver(!crate::quick::energy_saver()),
        },
        Act::Mute => toggle_mute(),
        Act::AudioOut => {
            refresh_audio_devices();
            QUICK_SUB.set(1);
        }
        Act::AudioPick(i) => {
            let id = AUDIO_DEVS.with_borrow(|devs| devs.get(i).map(|d| d.id.clone()));
            if let Some(id) = id {
                set_audio_device(&id);
                refresh_audio_devices();
                EP.set(None);
                read_volume();
                refresh_status();
                InvalidateRect(BAR.load(Relaxed) as HWND, null(), 0);
            }
        }
        Act::Menu(i) => {
            close();
            if let Some(pick) = MPICK.take() {
                pick(i as usize);
            }
        }
        Act::StartCtx(j) => {
            let Some((i, _, _)) = START_CTX.take() else { return };
            let Some(cmd) = start_ctx_items(i).get(j as usize).map(|x| x.2) else { return };
            let Some(lnk) = filtered().get(i).and_then(|&ai| APPS.lock().unwrap_or_else(|e| e.into_inner()).get(ai).map(|a| a.1.clone())) else { return };
            let path = wstr_of(&lnk);
            match cmd {
                1 => {
                    if launch(i) {
                        close();
                    }
                }
                2 => {
                    let _ = ShellExecuteW(null_mut(), wide("runas").as_ptr(), lnk.as_ptr(), null(), null(), SW_SHOWNORMAL);
                    close();
                }
                3 => {
                    let p = String::from_utf16_lossy(&lnk[..lnk.len().saturating_sub(1)]);
                    winrust_common::reveal(&p);
                    close();
                }
                4 => start_toggle(&path),
                _ => {}
            }
        }
        Act::Power(p) => {
            if p >= 2 && CONFIRM.get() != p as i32 {
                CONFIRM.set(p as i32);
                return;
            }
            close();
            match p {
                0 => crate::lock_workstation(),
                1 => { std::thread::spawn(|| sleep_now()); }
                2 => { let _ = Command::new("shutdown").args(["/r", "/t", "0"]).creation_flags(NO_WINDOW).spawn(); }
                _ => { let _ = Command::new("shutdown").args(["/s", "/t", "0"]).creation_flags(NO_WINDOW).spawn(); }
            }
        }
        Act::Settings(u) => {
            open(&wide(u));
            close();
        }
        Act::CalPrev | Act::CalNext => {
            let (mut y, mut m) = CAL.get();
            m += if a == Act::CalNext { 1 } else { -1 };
            (y, m) = if m < 1 { (y - 1, 12) } else if m > 12 { (y + 1, 1) } else { (y, m) };
            CAL.set((y, m));
        }
        Act::CalToday => {
            let mut t: SYSTEMTIME = zeroed();
            GetLocalTime(&mut t);
            CAL.set((t.wYear as i32, t.wMonth as i32));
        }
        Act::Net(i) => {
            let n = NETS.with_borrow(|n| n.get(i).map(|n| (n.connected, n.profile.clone(), n.auth, n.secure, n.ssid.clone(), n.g)));
            let Some((connected, profile, auth, secure, ssid, iface)) = n else { return };
            if connected {
            } else if let Some(p) = profile {
                wifi_connect(&p, &iface);
                start_connecting(&ssid);
            } else if !secure {
                NETS.with_borrow(|l| wifi_join(&l[i], ""));
            } else if matches!(auth, 4 | 7 | 8 | 9) {
                PASS.set(Some((i, String::new())));
            } else {
                open(&wide("ms-settings:network-wifi"));
                close();
            }
        }
        Act::NetOff => {
            for_each_wlan(|h, g| _ = WlanDisconnect(h, g, null()));
            NOTE.set("Disconnected".into());
        }
        Act::Ctx(j) => {
            let Some((ssid, ..)) = CTX.take() else { return };
            match ctx_items(&ssid).get(j as usize).map(|i| i.1) {
                Some(0) => {
                    if let Some(i) = NETS.with_borrow(|n| n.iter().position(|n| n.ssid == ssid)) {
                        run_act(Act::Net(i), rect, x);
                    }
                }
                Some(1) => run_act(Act::NetOff, rect, x),
                Some(2) => forget(&ssid),
                Some(3) => {
                    open(&wide("ms-settings:network-wifisettings"));
                    close();
                }
                _ => {}
            }
        }
        Act::App(i) => {
            if launch(i) {
                close();
            }
        }
        Act::StartAll => {
            START_ALL.set(!START_ALL.get());
            SCROLL.set(0);
            SEL.set(0);
        }
        Act::WidgetTool(i) => {
            close();
            match i {
                0 => open(&wide("shell:MyComputerFolder")),
                1 => crate::open_taskmgr(),
                _ => open(&wide("ms-settings:")),
            }
        }
        Act::Desk(i) => {
            close();
            vdesk_action(i);
        }
    }
}

pub fn sleep_now() {
    use windows_sys::Win32::{
        Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES},
        System::{Power::SetSuspendState, Threading::{GetCurrentProcess, OpenProcessToken}},
    };
    unsafe {
        let mut tok = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut tok) != 0 {
            let mut tp: TOKEN_PRIVILEGES = zeroed();
            tp.PrivilegeCount = 1;
            tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            if LookupPrivilegeValueW(null(), wide("SeShutdownPrivilege").as_ptr(), &mut tp.Privileges[0].Luid) != 0 {
                AdjustTokenPrivileges(tok, 0, &tp, 0, null_mut(), null_mut());
            }
            CloseHandle(tok);
        }
        SetSuspendState(false, false, false);
    }
}

unsafe fn fill_a(rc: RECT, c: u32, a: u8, rad: i32) {
    let g = FRAME_G.get();
    if g.is_null() {
        return;
    }
    let rad = if style().square() { 0 } else { rad };
    let (wd, ht) = ((rc.right - rc.left) as f32, (rc.bottom - rc.top) as f32);
    if wd <= 0.0 || ht <= 0.0 {
        return;
    }
    let argb = ((a as u32) << 24) | ((c & 255) << 16) | (c & 0xFF00) | ((c >> 16) & 255);
    let br = crate::cached_gdip_brush(argb);
    if !br.is_null() {
        if rad <= 0 {
            GdipFillRectangle(g, br as *mut GpBrush, rc.left as f32, rc.top as f32, wd, ht);
        } else {
            let d = (rad as f32).min(wd).min(ht);
            let (x, y) = (rc.left as f32, rc.top as f32);
            let mut path: *mut GpPath = null_mut();
            GdipCreatePath(FillModeAlternate, &mut path);
            if !path.is_null() {
                GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
                GdipAddPathArc(path, x + wd - d, y, d, d, 270.0, 90.0);
                GdipAddPathArc(path, x + wd - d, y + ht - d, d, d, 0.0, 90.0);
                GdipAddPathArc(path, x, y + ht - d, d, d, 90.0, 90.0);
                GdipClosePathFigure(path);
                GdipFillPath(g, br as *mut GpBrush, path);
                GdipDeletePath(path);
            }
        }
    }
}

unsafe fn stroke_a(rc: RECT, c: u32, a: u8, rad: i32) {
    use windows_sys::Win32::Graphics::GdiPlus::GdipDrawPath;
    let g = FRAME_G.get();
    if g.is_null() {
        return;
    }
    let rad = if style().square() { 0 } else { rad };
    let (wd, ht) = ((rc.right - rc.left) as f32 - 1.0, (rc.bottom - rc.top) as f32 - 1.0);
    if wd <= 0.0 || ht <= 0.0 {
        return;
    }
    let argb = ((a as u32) << 24) | ((c & 255) << 16) | (c & 0xFF00) | ((c >> 16) & 255);
    let mut pen: *mut GpPen = null_mut();
    if GdipCreatePen1(argb, 1.0, UnitPixel, &mut pen) != 0 || pen.is_null() {
        return;
    }
    let (x, y) = (rc.left as f32 + 0.5, rc.top as f32 + 0.5);
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    if !path.is_null() {
        if rad <= 1 {
            GdipAddPathArc(path, x, y, 0.01, 0.01, 180.0, 90.0);
            GdipAddPathArc(path, x + wd, y, 0.01, 0.01, 270.0, 90.0);
            GdipAddPathArc(path, x + wd, y + ht, 0.01, 0.01, 0.0, 90.0);
            GdipAddPathArc(path, x, y + ht, 0.01, 0.01, 90.0, 90.0);
        } else {
            let d = ((rad as f32) - 1.0).min(wd).min(ht);
            GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
            GdipAddPathArc(path, x + wd - d, y, d, d, 270.0, 90.0);
            GdipAddPathArc(path, x + wd - d, y + ht - d, d, d, 0.0, 90.0);
            GdipAddPathArc(path, x, y + ht - d, d, d, 90.0, 90.0);
        }
        GdipClosePathFigure(path);
        GdipDrawPath(g, pen, path);
        GdipDeletePath(path);
    }
    GdipDeletePen(pen);
}

unsafe fn track(dc: HDC, rc: RECT, frac: f32, ulw: bool) {
    let (l, rr, m, th) = (rc.left + sc(10), rc.right - sc(10), (rc.top + rc.bottom) / 2, sc(2).max(2));
    let rail = RECT { left: l, top: m - th, right: rr, bottom: m + th };
    if ulw {
        fill_a(rail, pal().text, 42, 2 * th);
    } else {
        fill(dc, rail, mix(card(), pal().text, 0.16), 2 * th);
    }
    let x = l + ((rr - l) as f32 * frac.clamp(0.0, 1.0)) as i32;
    fill(dc, RECT { left: l, top: m - th, right: x.max(l + 2 * th), bottom: m + th }, pal().accent, 2 * th);
    let kr = sc(9).max(6);
    if ulw {
        fill_a(RECT { left: x - kr - 1, top: m - kr - 1, right: x + kr + 1, bottom: m + kr + 1 }, pal().text, 38, 2 * kr + 2);
    } else {
        fill(dc, RECT { left: x - kr - 1, top: m - kr - 1, right: x + kr + 1, bottom: m + kr + 1 }, mix(card(), pal().text, 0.14), 2 * kr + 2);
    }
    fill(dc, RECT { left: x - kr, top: m - kr, right: x + kr, bottom: m + kr }, if style().light() { rgb(255, 255, 255) } else { rgb(69, 69, 69) }, 2 * kr);
    let ir = sc(5).max(3);
    fill(dc, RECT { left: x - ir, top: m - ir, right: x + ir, bottom: m + ir }, pal().accent, 2 * ir);
}

unsafe fn surface(dc: HDC, rc: RECT, base: u32, rad: i32) {
    let light = style().light();
    if light {
        fill(dc, RECT { top: rc.top + sc(1), bottom: rc.bottom + sc(2), ..rc }, mix(pal().bg, pal().dim, 0.14), rad);
    }
    fill(dc, rc, if light { mix(pal().bg, pal().line, 0.9) } else { mix(base, pal().text, 0.10) }, rad);
    fill(dc, RECT { left: rc.left + 1, top: rc.top + 1, right: rc.right - 1, bottom: rc.bottom - 1 }, base, (rad - 2).max(0));
}

fn inset(rc: RECT, d: i32) -> RECT {
    RECT { left: rc.left + d, top: rc.top + d, right: rc.right - d, bottom: rc.bottom - d }
}

unsafe fn user_display_name() -> String {
    thread_local! { static NAME: std::cell::OnceCell<String> = const { std::cell::OnceCell::new() }; }
    NAME.with(|c| {
        c.get_or_init(|| {
            let m = LoadLibraryW(wide("secur32.dll").as_ptr());
            if !m.is_null() {
                let mut res = None;
                if let Some(p) = GetProcAddress(m, b"GetUserNameExW\0".as_ptr()) {
                    let f: unsafe extern "system" fn(i32, *mut u16, *mut u32) -> u8 = std::mem::transmute(p);
                    let mut buf = [0u16; 128];
                    let mut sz = buf.len() as u32;
                    if f(3, buf.as_mut_ptr(), &mut sz) != 0 && sz > 1 {
                        let s = String::from_utf16_lossy(&buf[..sz as usize - 1]).trim().to_string();
                        if !s.is_empty() {
                            res = Some(s);
                        }
                    }
                }
                FreeLibrary(m);
                if let Some(s) = res {
                    return s;
                }
            }
            std::env::var("USERNAME").unwrap_or_default()
        })
        .clone()
    })
}

unsafe fn user_avatar() -> *mut GpBitmap {
    thread_local! { static AVATAR: Cell<(isize, bool)> = const { Cell::new((0, false)) }; }
    let (ptr, tried) = AVATAR.get();
    if tried {
        return ptr as *mut GpBitmap;
    }
    let mut bmp: *mut GpBitmap = null_mut();
    let m = LoadLibraryW(wide("shell32.dll").as_ptr());
    if !m.is_null() {
        if let Some(p) = GetProcAddress(m, 261usize as *const u8) {
            let f: unsafe extern "system" fn(*const u16, u32, *mut u16, u32) -> i32 = std::mem::transmute(p);
            let mut buf = [0u16; 512];
            if f(null(), 0x8000_0000, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
                if len > 0 {
                    use windows_sys::Win32::Graphics::GdiPlus::GdipCreateBitmapFromFile;
                    let mut b: *mut GpBitmap = null_mut();
                    if GdipCreateBitmapFromFile(buf.as_ptr(), &mut b) == 0 && !b.is_null() {
                        bmp = b;
                    }
                }
            }
        }
        FreeLibrary(m);
    }
    AVATAR.set((bmp as isize, true));
    bmp
}

unsafe fn paint(hw: HWND, k: Kind) {
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(hw, &mut ps);
    paint_layer(hw, k, wdc);
    EndPaint(hw, &ps);
}

unsafe fn paint_layer(hw: HWND, k: Kind, wdc: HDC) {
    let mut cr: RECT = zeroed();
    GetClientRect(hw, &mut cr);
    if cr.right <= 0 || cr.bottom <= 0 {
        return;
    }
    BUSY.set(false);
    let sop = i18n::style::start_opacity();
    let ulw = sop < 100;
    let bg_a = if ulw { ((255 * sop as u32) / 100).max(1) as u8 } else { 255 };
    let (dc, _, _g) = FLY_LBUF.with_borrow_mut(|b| b.begin(wdc, cr.right, cr.bottom));
    let rad = rr(16);
    fill_a(cr, pal().bg, bg_a, rad);
    SetBkMode(dc, TRANSPARENT as i32);
    let fs_ = if k == Kind::Quick { quick_fit_scale() } else { SCALE.get() };
    let fs_bits = fs_.to_bits();
    let fonts = {
        let (cached_bits, mut f_arr) = FLY_FONTS.get();
        if cached_bits != fs_bits || f_arr[0].is_null() {
            for f in f_arr {
                if !f.is_null() {
                    DeleteObject(f);
                }
            }
            let (face, gface) = (wide(ui_face()), wide(winrust_common::style::icon_face()));
            let font = |f: &[u16], px: i32, wt: i32| CreateFontW(-((px as f32 * fs_).round() as i32), 0, 0, 0, wt, 0, 0, 0, 1, 0, 0, 5, 0, f.as_ptr());
            f_arr = [
                font(&face, 13, 400),
                font(&face, 12, 400),
                font(&gface, 16, 400),
                font(&face, 15, 600),
                font(&face, 13, 600),
                font(&gface, 11, 400),
            ];
            FLY_FONTS.set((fs_bits, f_arr));
        }
        f_arr
    };
    let [f_txt, f_small, f_glyph, f_title, f_semi, f_gs] = fonts;
    let mut rs = PAINT_RECTS.take();
    let card = card();
    let of = SelectObject(dc, f_txt);
    let (w, h) = size(k);
    let over = OVER.get();
    let hot = |a: Act| over == Some(a);
    let g = |a: Act, rc: RECT, _from: u32, to: u32, rad: i32| glow(dc, hot(a), rc, to, rad);
    let (vol, muted) = VOL.get();

    match k {
        Kind::Quick => 'quick_done: {
            use winrust_common::ui::icons;
            let (w, h) = quick_base_size();
            let qs = fs_;
            rects_into(k, &mut rs);
            let sub_page = QUICK_SUB.get();
            let hov_a = ((bg_a as u32) + 40).min(230) as u8;
            if sub_page == 1 {
                let back_rc = qr(12, 12, 44, 44, qs);
                if ulw {
                    if hot(Act::QuickBack) {
                        fill_a(back_rc, mix(pal().bg, pal().text, 0.12), hov_a, qsc(6, qs));
                    }
                } else {
                    g(Act::QuickBack, back_rc, pal().bg, mix(pal().bg, pal().text, 0.08), qsc(6, qs));
                }
                SelectObject(dc, f_glyph);
                text(dc, icons::CHEVRON_LEFT, back_rc, pal().text, DT_CENTER);
                SelectObject(dc, f_title);
                text(dc, "Select a sound output", qr(50, 12, w - 16, 44, qs), pal().text, DT_LEFT);
                AUDIO_DEVS.with_borrow(|devs| {
                    for (i, d) in devs.iter().enumerate().take(5) {
                        let y = 56 + i as i32 * 40;
                        let row = qr(12, y, w - 12, y + 36, qs);
                        if ulw {
                            if hot(Act::AudioPick(i)) || d.default {
                                fill_a(row, mix(pal().bg, pal().text, if hot(Act::AudioPick(i)) { 0.14 } else { 0.08 }), hov_a, qsc(6, qs));
                            }
                        } else {
                            let base = if d.default { mix(pal().bg, pal().text, 0.06) } else { pal().bg };
                            g(Act::AudioPick(i), row, base, mix(pal().bg, pal().text, 0.09), qsc(6, qs));
                        }
                        if d.default {
                            fill(dc, qr(14, y + 8, 17, y + 28, qs), pal().accent, qsc(3, qs));
                        }
                        let low = d.name.to_lowercase();
                        let dg = if low.contains("headphone") || low.contains("headset") || low.contains("tai nghe") { icons::HEADPHONES } else { icons::SPEAKER };
                        SelectObject(dc, f_glyph);
                        text(dc, dg, qr(22, y, 48, y + 36, qs), if d.default { pal().accent } else { pal().text }, DT_CENTER);
                        SelectObject(dc, if d.default { f_semi } else { f_txt });
                        text(dc, &d.name, qr(52, y, w - 20, y + 36, qs), if d.default { pal().accent } else { pal().text }, DT_LEFT);
                    }
                });
                fill_a(qr(16, h - 44, w - 16, h - 43, qs), pal().text, if ulw { 24 } else { 20 }, 0);
                let more_rc = qr(12, h - 40, w - 12, h - 6, qs);
                if ulw {
                    if hot(Act::Settings("ms-settings:sound")) {
                        fill_a(more_rc, mix(pal().bg, pal().text, 0.12), hov_a, qsc(6, qs));
                    }
                } else {
                    g(Act::Settings("ms-settings:sound"), more_rc, pal().bg, mix(pal().bg, pal().text, 0.08), qsc(6, qs));
                }
                SelectObject(dc, f_txt);
                text(dc, "More volume settings", RECT { left: more_rc.left + qsc(8, qs), ..more_rc }, pal().text, DT_LEFT);
                break 'quick_done;
            }
            if sub_page == 2 {
                let on = NETS.with_borrow(|n| !n.is_empty()) || WIFI.get() >= 0;
                let back_rc = qr(12, 12, 44, 44, qs);
                if ulw {
                    if hot(Act::QuickBack) {
                        fill_a(back_rc, mix(pal().bg, pal().text, 0.12), hov_a, qsc(6, qs));
                    }
                } else {
                    g(Act::QuickBack, back_rc, pal().bg, mix(pal().bg, pal().text, 0.08), qsc(6, qs));
                }
                SelectObject(dc, f_glyph);
                text(dc, icons::CHEVRON_LEFT, back_rc, pal().text, DT_CENTER);
                SelectObject(dc, f_title);
                text(dc, "Wi‑Fi", qr(50, 12, w - 72, 44, qs), pal().text, DT_LEFT);
                let sw = qr(w - 58, 16, w - 16, 40, qs);
                let ts = if on { 1.0 } else { 0.0 };
                let swh = sw.bottom - sw.top;
                fill(dc, sw, mix(mix(card, pal().text, 0.18), pal().accent, ts), swh);
                let kr = swh / 2 - qsc(2, qs);
                let kx = sw.left + swh / 2 + ((sw.right - sw.left - swh) as f32 * ts) as i32;
                let ky = (sw.top + sw.bottom) / 2;
                fill(dc, RECT { left: kx - kr, top: ky - kr, right: kx + kr, bottom: ky + kr }, rgb(255, 255, 255), 2 * kr);
                if let Some((i, p)) = PASS.with_borrow(|p| p.clone()) {
                    let name = NETS.with_borrow(|n| n.get(i).map(|n| n.ssid.clone()).unwrap_or_default());
                    SelectObject(dc, f_txt);
                    text(dc, &format!("Password for “{name}”"), qr(18, 62, w - 18, 84, qs), pal().text, DT_LEFT);
                    if ulw {
                        fill_a(qr(16, 90, w - 16, 128, qs), mix(pal().bg, pal().text, 0.08), hov_a, qsc(8, qs));
                        stroke_a(qr(16, 90, w - 16, 128, qs), pal().accent, 220, qsc(8, qs));
                    } else {
                        fill(dc, qr(16, 90, w - 16, 128, qs), pal().accent, qsc(8, qs));
                        fill(dc, qr(18, 92, w - 18, 126, qs), card, qsc(6, qs));
                    }
                    let dots = "•".repeat(p.chars().count());
                    text(dc, &dots, qr(30, 90, w - 30, 128, qs), pal().text, DT_LEFT);
                    SelectObject(dc, f_small);
                    text(dc, "Enter — connect · Esc — cancel", qr(18, 136, w - 18, 156, qs), pal().dim, DT_LEFT);
                } else {
                    let max_n = ((h - 104) / 44).max(1) as usize;
                    let state = net();
                    let online = !matches!(state, Link::NoInternet(_));
                    NETS.with_borrow(|nets| {
                        if nets.is_empty() {
                            SelectObject(dc, f_txt);
                            text(dc, if on { "No networks nearby" } else { "Turn Wi‑Fi on to see networks" }, qr(18, 70, w - 18, 110, qs), pal().dim, DT_LEFT);
                        }
                        for (i, n) in nets.iter().enumerate().take(max_n) {
                            let y = 56 + i as i32 * 44;
                            let rc = qr(12, y, w - 12, y + 40, qs);
                            if ulw {
                                if n.connected || hot(Act::Net(i)) {
                                    fill_a(rc, mix(pal().bg, pal().text, if hot(Act::Net(i)) { 0.14 } else { 0.08 }), hov_a, qsc(6, qs));
                                }
                            } else {
                                let base = if n.connected { mix(pal().bg, pal().text, 0.06) } else { pal().bg };
                                g(Act::Net(i), rc, base, mix(pal().bg, pal().text, 0.09), qsc(6, qs));
                            }
                            wifi_glyph(dc, rc.left + qsc(20, qs), (rc.top + rc.bottom) / 2 - qsc(1, qs), qsc(14, qs), arcs_lit(n.q), if n.connected { pal().accent } else { pal().text }, mix(pal().dim, card, 0.55), if n.connected && !online { 2 } else { 0 });
                            SelectObject(dc, if n.connected { f_semi } else { f_txt });
                            text(dc, &n.ssid, RECT { left: rc.left + qsc(40, qs), right: rc.right - qsc(44, qs), ..rc }, if n.connected { pal().accent } else { pal().text }, DT_LEFT);
                            if n.connected {
                                SelectObject(dc, f_glyph);
                                text(dc, icons::CHECK, RECT { left: rc.right - qsc(36, qs), right: rc.right - qsc(8, qs), ..rc }, pal().accent, DT_CENTER);
                            } else if n.secure {
                                SelectObject(dc, f_gs);
                                text(dc, icons::LOCK, RECT { left: rc.right - qsc(36, qs), right: rc.right - qsc(8, qs), ..rc }, pal().dim, DT_CENTER);
                            }
                        }
                    });
                }
                fill_a(qr(16, h - 44, w - 16, h - 43, qs), pal().text, if ulw { 24 } else { 20 }, 0);
                let more_rc = qr(12, h - 40, w - 12, h - 6, qs);
                if ulw {
                    if hot(Act::Settings("ms-settings:network-wifi")) {
                        fill_a(more_rc, mix(pal().bg, pal().text, 0.12), hov_a, qsc(6, qs));
                    }
                } else {
                    g(Act::Settings("ms-settings:network-wifi"), more_rc, pal().bg, mix(pal().bg, pal().text, 0.08), qsc(6, qs));
                }
                SelectObject(dc, f_txt);
                text(dc, "More Wi‑Fi settings", RECT { left: more_rc.left + qsc(8, qs), ..more_rc }, pal().text, DT_LEFT);
                break 'quick_done;
            }
            let q = WIFI.get();
            let ssid = NETS.with_borrow(|n| n.iter().find(|n| n.connected).map(|n| n.ssid.clone()));
            let wsub = match (q >= 0, ssid) {
                (true, Some(s)) => s,
                _ => "Wi‑Fi".into(),
            };
            let (bt, air, eco) = (crate::quick::bluetooth(), crate::quick::airplane(), crate::quick::energy_saver());
            let states = [q >= 0, bt == Some(true), air == Some(true), eco];
            let tiles = [
                (icons::WIFI, wsub.as_str()),
                (icons::BLUETOOTH, "Bluetooth"),
                (icons::AIRPLANE, "Airplane mode"),
                (icons::ENERGY_SAVER, "Energy saver"),
            ];
            let btn_w = (w - 32 - 12) / 2;
            for (i, (glyph, label)) in tiles.iter().enumerate() {
                let (tl, tt) = (16 + (btn_w + 12) * (i as i32 % 2), 16 + 80 * (i as i32 / 2));
                let full = qr(tl, tt, tl + btn_w, tt + 48, qs);
                let on = states[i];
                let base = if on { pal().accent } else { card };
                let hover_bg = if on { mix(pal().accent, rgb(255, 255, 255), 0.12) } else { mix(card, pal().text, 0.06) };
                let ink = if on { on_accent() } else { pal().text };
                if ulw {
                    let tile_a = if on { ((bg_a as u32) + 55).min(240) as u8 } else { ((bg_a as u32) + 25).min(215) as u8 };
                    let fill_col = if on { base } else { mix(pal().bg, pal().text, 0.08) };
                    fill_a(full, fill_col, tile_a, qsc(8, qs));
                    if !on {
                        stroke_a(full, pal().text, 28, qsc(8, qs));
                    }
                    if i == 0 {
                        let mid = tl + btn_w / 2;
                        let left_rc = qr(tl + 1, tt + 1, mid, tt + 47, qs);
                        let right_rc = qr(mid + 1, tt + 1, tl + btn_w - 1, tt + 47, qs);
                        if hot(Act::Tile(0)) {
                            fill_a(left_rc, if on { rgb(255, 255, 255) } else { pal().text }, if on { 32 } else { 22 }, qsc(7, qs));
                        }
                        if hot(Act::WifiMore) {
                            fill_a(right_rc, if on { rgb(255, 255, 255) } else { pal().text }, if on { 32 } else { 22 }, qsc(7, qs));
                        }
                        fill_a(qr(mid, tt + 1, mid + 1, tt + 47, qs), if on { on_accent() } else { pal().text }, if on { 70 } else { 30 }, 0);
                        wifi_glyph(dc, (left_rc.left + left_rc.right) / 2, (left_rc.top + left_rc.bottom) / 2 - qsc(1, qs), qsc(15, qs), if q >= 0 { arcs_lit(q) } else { 0 }, ink, mix(ink, base, 0.55), if q < 0 { 1 } else { 0 });
                        SelectObject(dc, f_gs);
                        text(dc, icons::CHEVRON_RIGHT, right_rc, ink, DT_CENTER);
                    } else {
                        if hot(Act::Tile(i as u8)) {
                            fill_a(inset(full, 1), if on { rgb(255, 255, 255) } else { pal().text }, if on { 32 } else { 22 }, qsc(7, qs));
                        }
                        SelectObject(dc, f_glyph);
                        text(dc, glyph, full, ink, DT_CENTER);
                    }
                } else {
                    surface(dc, full, base, qsc(8, qs));
                    if i == 0 {
                        let mid = tl + btn_w / 2;
                        let left_rc = qr(tl + 1, tt + 1, mid, tt + 47, qs);
                        let right_rc = qr(mid + 1, tt + 1, tl + btn_w - 1, tt + 47, qs);
                        g(Act::Tile(0), left_rc, base, hover_bg, qsc(7, qs));
                        g(Act::WifiMore, right_rc, base, hover_bg, qsc(7, qs));
                        fill(dc, qr(mid, tt + 1, mid + 1, tt + 47, qs), if on { mix(on_accent(), pal().accent, 0.55) } else { mix(card, pal().text, 0.12) }, 0);
                        wifi_glyph(dc, (left_rc.left + left_rc.right) / 2, (left_rc.top + left_rc.bottom) / 2 - qsc(1, qs), qsc(15, qs), if q >= 0 { arcs_lit(q) } else { 0 }, ink, mix(ink, base, 0.55), if q < 0 { 1 } else { 0 });
                        SelectObject(dc, f_gs);
                        text(dc, icons::CHEVRON_RIGHT, right_rc, ink, DT_CENTER);
                    } else {
                        g(Act::Tile(i as u8), inset(full, 1), base, hover_bg, qsc(7, qs));
                        SelectObject(dc, f_glyph);
                        text(dc, glyph, full, ink, DT_CENTER);
                    }
                }
                SelectObject(dc, f_small);
                text(dc, label, qr(tl, tt + 51, tl + btn_w, tt + 73, qs), pal().text, DT_CENTER);
            }
            fill_a(qr(16, 178, w - 16, 179, qs), pal().text, if ulw { 24 } else { 20 }, 0);
            let n = sliders();
            let mut y = 188;
            let b = BRIGHT.load(Relaxed);
            if b >= 0 {
                SelectObject(dc, f_glyph);
                text(dc, icons::BRIGHTNESS, qr(14, y, 48, y + 40, qs), mix(pal().dim, pal().text, 0.5), DT_CENTER);
                if let Some((rc, _)) = rs.iter().find(|(_, a)| *a == Act::BrightSlide) {
                    track(dc, *rc, b as f32 / 100.0, ulw);
                }
                y += 44;
            }
            if ulw {
                if hot(Act::Mute) {
                    fill_a(qr(14, y + 2, 48, y + 38, qs), mix(pal().bg, pal().text, 0.12), hov_a, qsc(6, qs));
                }
            } else {
                g(Act::Mute, qr(14, y + 2, 48, y + 38, qs), pal().bg, mix(pal().bg, pal().text, 0.08), qsc(6, qs));
            }
            let mrc = qr(14, y, 48, y + 40, qs);
            volume_glyph(dc, (mrc.left + mrc.right) / 2, (mrc.top + mrc.bottom) / 2, qsc(16, qs), vol, muted, if muted { BAD } else { pal().text });
            if let Some((rc, _)) = rs.iter().find(|(_, a)| *a == Act::VolSlide) {
                track(dc, *rc, if muted || vol < 0 { 0.0 } else { vol as f32 / 100.0 }, ulw);
            }
            if let Some(&(out_rc, _)) = rs.iter().find(|(_, a)| *a == Act::AudioOut) {
                if ulw {
                    if hot(Act::AudioOut) {
                        fill_a(out_rc, mix(pal().bg, pal().text, 0.12), hov_a, qsc(6, qs));
                    }
                } else {
                    g(Act::AudioOut, out_rc, pal().bg, mix(pal().bg, pal().text, 0.08), qsc(6, qs));
                }
                let dev_glyph = AUDIO_DEVS.with_borrow(|devs| {
                    let is_hp = devs.iter().find(|d| d.default).is_some_and(|d| {
                        let l = d.name.to_lowercase();
                        l.contains("headphone") || l.contains("headset") || l.contains("tai nghe")
                    });
                    if is_hp { icons::HEADPHONES } else { icons::SPEAKER }
                });
                SelectObject(dc, f_glyph);
                text(dc, dev_glyph, RECT { left: out_rc.left + qsc(2, qs), right: out_rc.right - qsc(12, qs), ..out_rc }, pal().text, DT_CENTER);
                SelectObject(dc, f_gs);
                text(dc, icons::CHEVRON_RIGHT, RECT { left: out_rc.right - qsc(16, qs), right: out_rc.right - qsc(2, qs), ..out_rc }, pal().dim, DT_CENTER);
            }
            let sl_h = if n == 2 { 86 } else { 42 };
            let foot_top = 188 + sl_h + 10;
            let (pct, ac) = BAT.get();
            if pct >= 0 {
                SelectObject(dc, f_glyph);
                text(dc, if ac { icons::BATTERY_CHARGE } else { icons::BATTERY }, qr(14, foot_top + 5, 40, foot_top + 39, qs), if ac { GOOD } else if pct <= 15 { BAD } else { pal().text }, DT_CENTER);
                SelectObject(dc, f_txt);
                text(dc, &format!("{pct}%"), qr(42, foot_top + 5, 90, foot_top + 39, qs), pal().text, DT_LEFT);
            } else {
                SelectObject(dc, f_small);
                text(dc, &if muted { "0%".into() } else if vol < 0 { String::new() } else { format!("{vol}%") }, qr(18, foot_top + 5, 76, foot_top + 39, qs), pal().dim, DT_LEFT);
            }
            if let Some(&(gear, _)) = rs.iter().find(|(_, a)| matches!(a, Act::Settings(_))) {
                if ulw {
                    if hot(Act::Settings("ms-settings:")) {
                        fill_a(gear, mix(pal().bg, pal().text, 0.12), hov_a, qsc(6, qs));
                    }
                } else {
                    g(Act::Settings("ms-settings:"), gear, pal().bg, mix(pal().bg, pal().text, 0.08), qsc(6, qs));
                }
                SelectObject(dc, f_glyph);
                text(dc, icons::SETTINGS, gear, pal().text, DT_CENTER);
            }
        }
        Kind::Cal => {
            let (y, m) = CAL.get();
            SelectObject(dc, f_title);
            text(dc, &month_title(y, m), r(16, 10, w - 100, 42), pal().text, DT_LEFT);
            SelectObject(dc, f_glyph);
            text(dc, "\u{E70E}", r(w - 84, 10, w - 52, 42), pal().text, DT_CENTER);
            text(dc, "\u{E70D}", r(w - 48, 10, w - 16, 42), pal().text, DT_CENTER);
            SelectObject(dc, f_small);
            for (i, d) in weekday_initials().iter().enumerate() {
                text(dc, d, r(18 + 42 * i as i32, 54, 60 + 42 * i as i32, 78), pal().dim, DT_CENTER);
            }
            let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
            let yy = if m < 3 { y - 1 } else { y };
            let first = ((yy + yy / 4 - yy / 100 + yy / 400 + t[m as usize - 1] + 1) % 7 + 6) % 7;
            let dim = match m {
                2 => if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 },
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            let mut now: SYSTEMTIME = zeroed();
            GetLocalTime(&mut now);
            SelectObject(dc, f_txt);
            for d in 1..=dim {
                let (col, row) = ((first + d - 1) % 7, (first + d - 1) / 7);
                let cell = r(18 + 42 * col, 84 + 40 * row, 60 + 42 * col, 120 + 40 * row);
                let today = now.wYear as i32 == y && now.wMonth as i32 == m && now.wDay as i32 == d;
                if today {
                    fill(dc, cell, pal().accent, sc(18));
                }
                text(dc, &d.to_string(), cell, if today { pal().bg } else { pal().text }, DT_CENTER);
            }
        }
        Kind::Wifi => {
            let on = NETS.with_borrow(|n| !n.is_empty()) || WIFI.get() >= 0;
            let state = net();
            let note = NOTE.with_borrow(|n| n.clone());
            let sub = if !note.is_empty() {
                Some((note, pal().dim))
            } else {
                match state {
                    Link::Wifi(_) => None,
                    Link::NoInternet(_) => Some(("Connected, no internet".to_string(), WARN)),
                    Link::Cable => Some(("Online over a cable".into(), pal().dim)),
                    Link::Offline if on => Some(("Not connected".into(), pal().dim)),
                    Link::Offline => Some(("Turned off".into(), pal().dim)),
                }
            };
            SelectObject(dc, f_title);
            match &sub {
                Some((s, c)) => {
                    text(dc, "Wi‑Fi", r(18, 13, w - 76, 33), pal().text, DT_LEFT);
                    SelectObject(dc, f_small);
                    text(dc, s, r(18, 33, w - 76, 51), *c, DT_LEFT);
                }
                None => text(dc, "Wi‑Fi", r(18, 18, w - 76, 42), pal().text, DT_LEFT),
            }
            let sw = r(w - 58, 18, w - 16, 42);
            let ts = if on { 1.0 } else { 0.0 };
            let swh = sw.bottom - sw.top;
            fill(dc, sw, mix(mix(card, pal().text, 0.18), pal().accent, ts), swh);
            let kr = swh / 2 - sc(2);
            let kx = sw.left + swh / 2 + ((sw.right - sw.left - swh) as f32 * ts) as i32;
            let ky = (sw.top + sw.bottom) / 2;
            fill(dc, RECT { left: kx - kr, top: ky - kr, right: kx + kr, bottom: ky + kr }, rgb(255, 255, 255), 2 * kr);
            rects_into(k, &mut rs);
            let pass = PASS.with_borrow(|p| p.clone());
            if let Some((i, p)) = pass {
                let name = NETS.with_borrow(|n| n.get(i).map(|n| n.ssid.clone()).unwrap_or_default());
                SelectObject(dc, f_txt);
                text(dc, &format!("Password for “{name}”"), r(18, 62, w - 18, 84), pal().text, DT_LEFT);
                fill(dc, r(16, 90, w - 16, 128), pal().accent, sc(10));
                fill(dc, r(18, 92, w - 18, 126), card, sc(8));
                let dots = "•".repeat(p.chars().count());
                text(dc, &dots, r(30, 90, w - 30, 128), pal().text, DT_LEFT);
                if now_ms() / 530 % 2 == 0 {
                    let mut tr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    let wd = wide(&dots);
                    DrawTextW(dc, wd.as_ptr(), -1, &mut tr, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
                    let cx = sc(30) + tr.right - tr.left + sc(2);
                    fill(dc, RECT { left: cx, top: sc(100), right: cx + sc(2), bottom: sc(118) }, pal().accent, 0);
                }
                BUSY.set(true);
                SelectObject(dc, f_small);
                text(dc, "Enter — connect · Esc — cancel", r(18, 136, w - 18, 156), pal().dim, DT_LEFT);
            } else {
                let (rows, label, _) = wifi_rows();
                let conn = CONN.with_borrow(|c| c.as_ref().map(|c| (c.q, c.phy, c.rx, c.tx)));
                let joining = CONNECTING.with_borrow(|c| c.as_ref().map(|c| c.0.clone()));
                let age = now_ms().saturating_sub(T0.get()) as f32;
                let now = now_ms();
                NETS.with_borrow(|nets| {
                    if nets.is_empty() {
                        SelectObject(dc, f_txt);
                        text(dc, if on { "No networks nearby" } else { "Turn Wi‑Fi on to see networks" }, r(18, 70, w - 18, 110), pal().dim, DT_LEFT);
                    }
                    if let Some(ly) = label {
                        SelectObject(dc, f_small);
                        text(dc, "Other networks", r(20, ly + 2, w - 16, ly + 22), pal().dim, DT_LEFT);
                    }
                    let first_other = nets.iter().position(|n| !n.connected);
                    let last_other = nets.iter().rposition(|n| !n.connected);
                    if let (Some(a), Some(b)) = (first_other, last_other) {
                        surface(dc, r(12, rows[a].0, w - 12, rows[b].1), card, sc(14));
                    }
                    for (i, n) in nets.iter().enumerate() {
                        let rc = rs[i].0;
                        let t = ease(((age - 40.0 * i as f32) / 220.0).clamp(0.0, 1.0));
                        if t < 1.0 {
                            BUSY.set(true);
                        }
                        let ink = |c: u32| mix(card, c, t);
                        let busy = joining.as_deref() == Some(n.ssid.as_str());
                        if n.connected {
                            surface(dc, rc, card, sc(14));
                        } else if last_other != Some(i) {
                            fill(dc, RECT { left: rc.left + sc(48), top: rc.bottom - 1, right: rc.right - sc(14), bottom: rc.bottom }, mix(card, pal().text, 0.08), 0);
                        }
                        let open = hot(Act::Net(i)) || (n.connected && hot(Act::NetOff));
                        g(Act::Net(i), RECT { left: rc.left + sc(4), top: rc.top + sc(4), right: rc.right - sc(4), bottom: rc.bottom - sc(4) }, card, mix(card, pal().text, 0.045), sc(10));
                        let lit = if busy { BUSY.set(true); (now / 230 % 4) as i32 } else { arcs_lit(n.q) };
                        let online = !matches!(state, Link::NoInternet(_));
                        let cy = (rc.top + rc.bottom) / 2;
                        let arc = if n.connected || busy { pal().accent } else { pal().text };
                        wifi_glyph(dc, rc.left + sc(26), cy - sc(1), sc(16), lit, ink(arc), ink(mix(pal().dim, card, 0.55)), if n.connected && !online { 2 } else { 0 });
                        let saved = n.profile.is_some();
                        let (sub, sub_c) = if busy {
                            (format!("{}{}", "Connecting", &"...."[..(now / 400 % 4) as usize]), pal().accent)
                        } else if n.connected {
                            let q = conn.map(|c| c.0).unwrap_or(n.q);
                            match conn {
                                _ if !online => ("Connected, no internet".into(), WARN),
                                Some((_, _, rx, tx)) if open => (format!("↓{rx} ↑{tx} {} · {}", "Mbps", auth_name(n.auth)), pal().dim),
                                Some((_, phy, ..)) => (format!("{} · {phy} · {q}%", "Connected"), pal().dim),
                                None => (format!("{} · {q}%", "Connected"), pal().dim),
                            }
                        } else if !n.secure {
                            ("Open network".into(), WARN)
                        } else if saved {
                            ("Saved".into(), pal().dim)
                        } else {
                            (String::new(), pal().dim)
                        };
                        let right = rc.right - sc(118);
                        SelectObject(dc, if n.connected { f_semi } else { f_txt });
                        if sub.is_empty() {
                            text(dc, &n.ssid, RECT { left: rc.left + sc(48), right, ..rc }, ink(pal().text), DT_LEFT);
                        } else {
                            text(dc, &n.ssid, RECT { left: rc.left + sc(48), top: cy - sc(19), right, bottom: cy + sc(1) }, ink(pal().text), DT_LEFT);
                            SelectObject(dc, f_small);
                            text(dc, &sub, RECT { left: rc.left + sc(48), top: cy, right, bottom: cy + sc(18) }, ink(sub_c), DT_LEFT);
                        }
                        if busy {
                            let (l, rr) = (rc.left + sc(48), rc.right - sc(16));
                            let span = rr - l;
                            let p = (now % 1300) as f32 / 1300.0;
                            let a = l + ((span as f32) * (ease(p) * 1.3 - 0.3)) as i32;
                            let b = a + span * 3 / 10;
                            fill(dc, RECT { left: l, top: rc.bottom - sc(5), right: rr, bottom: rc.bottom - sc(3) }, mix(card, pal().accent, 0.18), sc(2));
                            fill(dc, RECT { left: a.max(l), top: rc.bottom - sc(5), right: b.min(rr), bottom: rc.bottom - sc(3) }, pal().accent, sc(2));
                        }
                        if n.connected {
                            if open {
                                if let Some(&(ob, _)) = rs.iter().find(|(_, a)| *a == Act::NetOff) {
                                    g(Act::NetOff, ob, card, mix(card, BAD, 0.14), ob.bottom - ob.top);
                                    SelectObject(dc, f_semi);
                                    text(dc, "Disconnect", ob, if hot(Act::NetOff) { BAD } else { pal().dim }, DT_CENTER);
                                }
                            } else {
                                SelectObject(dc, f_glyph);
                                text(dc, "\u{E73E}", RECT { left: rc.right - sc(44), right: rc.right - sc(14), ..rc }, ink(pal().accent), DT_CENTER);
                            }
                        } else if open && !busy {
                            SelectObject(dc, f_semi);
                            let action = if saved || !n.secure { "Connect" } else { "Enter password" };
                            text(dc, action, RECT { left: rc.right - sc(130), right: rc.right - sc(16), ..rc }, pal().accent, DT_RIGHT);
                        } else if n.secure && !busy {
                            SelectObject(dc, f_gs);
                            text(dc, "\u{E72E}", RECT { left: rc.right - sc(42), right: rc.right - sc(14), ..rc }, ink(mix(pal().dim, card, 0.35)), DT_CENTER);
                        }
                    }
                });
            }
            if let Some(&(lr, la)) = rs.iter().find(|(_, a)| matches!(a, Act::Settings(_))) {
                g(la, lr, pal().bg, mix(pal().bg, pal().text, 0.05), sc(10));
                SelectObject(dc, f_txt);
                text(dc, "Network settings", RECT { left: lr.left + sc(8), ..lr }, pal().accent, DT_LEFT);
                SelectObject(dc, f_gs);
                text(dc, "\u{E76C}", RECT { left: lr.right - sc(34), ..lr }, pal().dim, DT_CENTER);
            }
            if let Some((ssid, mx, my, t0)) = CTX.with_borrow(|c| c.clone()) {
                let items = ctx_items(&ssid);
                let t = ease((now_ms().saturating_sub(t0) as f32 / 150.0).min(1.0));
                if t < 1.0 {
                    BUSY.set(true);
                }
                let full = sc(12 + CTX_ROW * items.len() as i32);
                let mr = RECT { left: mx, top: my, right: mx + sc(CTX_W), bottom: my + (full as f32 * (0.4 + 0.6 * t)) as i32 };
                fill(dc, RECT { left: mr.left - 1, top: mr.top + sc(2), right: mr.right + 1, bottom: mr.bottom + sc(4) }, mix(pal().bg, 0, if style().light() { 0.12 } else { 0.5 }), sc(14));
                surface(dc, mr, card, sc(12));
                for ((ir, a), (label, what)) in ctx_rects().iter().zip(&items) {
                    if ir.bottom > mr.bottom - sc(4) {
                        break;
                    }
                    g(*a, *ir, card, if *what == 2 { mix(card, BAD, 0.12) } else { mix(card, pal().text, 0.06) }, sc(8));
                    SelectObject(dc, f_txt);
                    text(dc, label, RECT { left: ir.left + sc(12), ..*ir }, if *what == 2 { BAD } else { pal().text }, DT_LEFT);
                }
            }
        }
        Kind::Widgets => {
            use windows_sys::Win32::{Storage::FileSystem::GetDiskFreeSpaceExW, System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX}};
            SelectObject(dc, f_title);
            text(dc, "Widgets", r(20, 16, w - 20, 44), pal().text, DT_LEFT);
            SelectObject(dc, f_small);
            text(dc, &clock_texts().1, r(20, 18, w - 20, 44), pal().dim, DT_RIGHT);

            surface(dc, r(16, 56, w - 16, 174), card, sc(16));
            let mut ms: MEMORYSTATUSEX = zeroed();
            ms.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
            GlobalMemoryStatusEx(&mut ms);
            let total_gb = ms.ullTotalPhys as f64 / 1_073_741_824.0;
            let used_gb = (ms.ullTotalPhys.saturating_sub(ms.ullAvailPhys)) as f64 / 1_073_741_824.0;
            let mem_pct = ms.dwMemoryLoad as f32 / 100.0;
            SelectObject(dc, f_semi);
            text(dc, "Memory (RAM)", r(28, 68, w - 28, 90), pal().text, DT_LEFT);
            text(dc, &format!("{}%", ms.dwMemoryLoad), r(28, 68, w - 28, 90), pal().accent, DT_RIGHT);
            SelectObject(dc, f_small);
            text(dc, &format!("{used_gb:.1} / {total_gb:.1} GB"), r(28, 92, w - 28, 112), pal().dim, DT_LEFT);
            let mb = r(28, 122, w - 28, 130);
            fill(dc, mb, mix(card, pal().text, 0.12), sc(8));
            fill(dc, RECT { right: mb.left + ((mb.right - mb.left) as f32 * mem_pct).max(sc(6) as f32) as i32, ..mb }, pal().accent, sc(8));
            let (vol_p, vol_m) = VOL.get();
            text(dc, &format!("{}  {}%", "Volume:", if vol_m { 0 } else { vol_p }), r(28, 142, w - 28, 162), pal().dim, DT_LEFT);

            surface(dc, r(16, 188, w - 16, 294), card, sc(16));
            const C_DRIVE: [u16; 4] = [b'C' as u16, b':' as u16, b'\\' as u16, 0];
            let (mut free_b, mut tot_b) = (0u64, 0u64);
            GetDiskFreeSpaceExW(C_DRIVE.as_ptr(), &mut free_b, &mut tot_b, null_mut());
            let tot_d = (tot_b as f64 / 1_073_741_824.0).max(0.1);
            let free_d = free_b as f64 / 1_073_741_824.0;
            let used_d = (tot_d - free_d).max(0.0);
            let d_pct = (used_d / tot_d).clamp(0.0, 1.0) as f32;
            SelectObject(dc, f_semi);
            text(dc, "Drive C:", r(28, 200, w - 28, 222), pal().text, DT_LEFT);
            text(dc, &format!("{:.0}%", d_pct * 100.0), r(28, 200, w - 28, 222), if d_pct > 0.9 { BAD } else { GOOD }, DT_RIGHT);
            SelectObject(dc, f_small);
            text(dc, &format!("{free_d:.0} GB free of {tot_d:.0} GB"), r(28, 224, w - 28, 244), pal().dim, DT_LEFT);
            let db = r(28, 256, w - 28, 264);
            fill(dc, db, mix(card, pal().text, 0.12), sc(8));
            fill(dc, RECT { right: db.left + ((db.right - db.left) as f32 * d_pct).max(sc(6) as f32) as i32, ..db }, if d_pct > 0.9 { BAD } else { GOOD }, sc(8));

            let tools = [
                ("\u{EC50}", "Explorer"),
                ("\u{E9D9}", "Tasks"),
                ("\u{E713}", "Settings"),
            ];
            rects_into(k, &mut rs);
            for &(rc, a) in &rs {
                let Act::WidgetTool(i) = a else { continue };
                let (glyph, lbl) = tools[i as usize];
                surface(dc, rc, card, sc(14));
                g(a, inset(rc, 1), card, mix(card, pal().text, 0.06), sc(12));
                SelectObject(dc, f_glyph);
                text(dc, glyph, RECT { top: rc.top + sc(10), bottom: rc.top + sc(44), ..rc }, pal().accent, DT_CENTER);
                SelectObject(dc, f_small);
                text(dc, lbl, RECT { top: rc.top + sc(46), bottom: rc.bottom - sc(6), ..rc }, pal().text, DT_CENTER);
            }
        }
        Kind::Desktops => {
            let card = i18n::style::card();
            let (cur, total) = vdesk_info();
            let title = if total > 1 { format!("Desktop {cur} / {total}") } else { "Desktops".into() };
            SelectObject(dc, f_txt);
            text(dc, &title, r(16, 12, w - 16, 34), pal().text, DT_LEFT);
            SelectObject(dc, f_small);
            text(dc, "Win+Tab", r(16, 14, w - 16, 34), pal().dim, DT_RIGHT);
            let btns = [
                ("\u{E72B}", "Prev"),
                ("\u{E710}", "New"),
                ("\u{E72A}", "Next"),
                ("\u{E711}", "Close"),
            ];
            rects_into(k, &mut rs);
            for &(rc, a) in &rs {
                let Act::Desk(i) = a else { continue };
                let (glyph, lbl) = btns[i as usize];
                surface(dc, rc, card, sc(12));
                g(a, inset(rc, 1), card, pal().hover, sc(10));
                SelectObject(dc, f_glyph);
                text(dc, glyph, RECT { top: rc.top + sc(6), bottom: rc.top + sc(34), ..rc }, if i == 1 { pal().accent } else if i == 3 { BAD } else { pal().text }, DT_CENTER);
                SelectObject(dc, f_small);
                text(dc, lbl, RECT { top: rc.top + sc(34), bottom: rc.bottom - sc(4), ..rc }, pal().text, DT_CENTER);
            }
        }
        Kind::Menu => {
            let ms = MENU_SCROLL.get();
            let has_head = MHEAD.with_borrow(|hd| hd.is_some());
            let top_clip = if has_head { MHEAD_H + 4 } else { 3 };
            rects_into(k, &mut rs);
            MITEMS.with_borrow(|items| {
                SelectObject(dc, f_small);
                for (it, (t, b)) in items.iter().zip(menu_rows().0) {
                    let (t, b) = (t - ms, b - ms);
                    if b <= top_clip || t >= h { continue; }
                    if it.head {
                        if it.sep {
                            fill(dc, r(10, t - 3, w - 10, t - 2), pal().line, 0);
                        }
                        text(dc, &it.label, r(14, t + 4, w - 10, b), pal().dim, DT_LEFT);
                    }
                }
                for (rc, a) in rs.iter() {
                    let Act::Menu(i) = *a else { continue };
                    let it = &items[i as usize];
                    if it.sep {
                        fill(dc, RECT { left: rc.left + sc(6), top: rc.top - sc(3), right: rc.right - sc(6), bottom: rc.top - sc(3) + 1 }, pal().line, 0);
                    }
                    let col = if it.danger { BAD } else { pal().text };
                    g(*a, *rc, pal().bg, if it.danger { mix(pal().bg, BAD, 0.22) } else { pal().hover }, rr(4));
                    let mut lx = rc.left + sc(10);
                    if !it.icon.is_null() {
                        draw_icon(dc, lx, (rc.top + rc.bottom) / 2 - sc(8), sc(16), it.icon);
                        lx += sc(22);
                    } else if !it.glyph.is_empty() {
                        SelectObject(dc, f_glyph);
                        text(dc, it.glyph, RECT { left: lx, top: rc.top, right: lx + sc(16), bottom: rc.bottom }, if it.on { pal().accent } else { col }, DT_CENTER);
                        lx += sc(22);
                    }
                    if it.swatch != u32::MAX {
                        let (cy, d) = ((rc.top + rc.bottom) / 2, sc(8));
                        fill(dc, RECT { left: lx, top: cy - d, right: lx + 2 * d, bottom: cy + d }, pal().line, 2 * d);
                        fill(dc, RECT { left: lx + 1, top: cy - d + 1, right: lx + 2 * d - 1, bottom: cy + d - 1 }, it.swatch, 2 * d);
                        lx += sc(24);
                    }
                    let mut rx = rc.right - sc(10);
                    if it.on {
                        let (cy, d) = ((rc.top + rc.bottom) / 2, sc(4));
                        fill(dc, RECT { left: rx - 2 * d, top: cy - d, right: rx, bottom: cy + d }, pal().accent, 2 * d);
                        rx -= sc(16);
                    }
                    SelectObject(dc, f_small);
                    if it.beta {
                        text(dc, "BETA", RECT { left: lx, right: rx, ..*rc }, rgb(255, 59, 48), DT_RIGHT);
                    }
                    if !it.hint.is_empty() {
                        text(dc, it.hint, RECT { left: lx, right: rx, ..*rc }, mix(pal().dim, pal().bg, 0.25), DT_RIGHT);
                    }
                    SelectObject(dc, f_txt);
                    text(dc, &it.label, RECT { left: lx, right: rx - match it.hint.chars().count() { 0 => 0, 1..=2 => sc(18), _ => sc(84) }, ..*rc }, if it.on { pal().accent } else { col }, DT_LEFT);
                }
            });
            MHEAD.with_borrow(|hd| {
                if let Some((ic, glyph, title, sub)) = hd {
                    fill(dc, r(1, 1, w - 1, MHEAD_H + 3), pal().bg, (rad - 1).max(0));
                    fill(dc, r(1, (MHEAD_H / 2).max(1), w - 1, MHEAD_H + 3), pal().bg, 0);
                    let tile = r(10, 8, 38, 36);
                    fill(dc, tile, mix(pal().bg, pal().accent, 0.16), sc(8));
                    if !ic.is_null() {
                        draw_icon(dc, tile.left + sc(4), tile.top + sc(4), sc(20), *ic);
                    } else {
                        SelectObject(dc, f_glyph);
                        text(dc, glyph, tile, pal().accent, DT_CENTER);
                    }
                    SelectObject(dc, f_title);
                    text(dc, title, r(46, 6, w - 10, 24), pal().text, DT_LEFT);
                    SelectObject(dc, f_small);
                    text(dc, sub, r(46, 22, w - 10, 38), pal().dim, DT_LEFT);
                    fill(dc, r(10, MHEAD_H + 2, w - 10, MHEAD_H + 3), pal().line, 0);
                }
            });
        }
        Kind::Start => {
            use winrust_common::ui::icons;
            let sr = r(24, 20, w - 24, 54);
            if ulw {
                let a = (bg_a / 2).max(24);
                fill_a(sr, mix(pal().bg, pal().text, 0.08), a, sc(8));
                stroke_a(sr, pal().text, 32, sc(8));
            } else {
                surface(dc, sr, card, sc(8));
            }
            SelectObject(dc, f_glyph);
            text(dc, icons::SEARCH, r(32, 20, 58, 54), pal().dim, DT_CENTER);
            SelectObject(dc, f_txt);
            let q = QUERY.with_borrow(|q| q.clone());
            if q.is_empty() {
                text(dc, "Type here to search", r(64, 20, w - 32, 54), pal().dim, DT_LEFT);
            } else {
                text(dc, &q, r(64, 20, w - 32, 54), if SELALL.get() { pal().accent } else { pal().text }, DT_LEFT);
            }
            let f = filtered();
            let q_empty = q.is_empty();
            let all_mode = START_ALL.get();
            if q_empty {
                SelectObject(dc, f_semi);
                text(dc, if all_mode { "All apps" } else { "Pinned" }, r(28, 68, w - 130, 94), pal().text, DT_LEFT);
                let ab = r(w - 114, 68, w - 24, 94);
                if ulw {
                    let a = if hot(Act::StartAll) { (bg_a as u32 + 45).min(210) as u8 } else { (bg_a / 2).max(24) };
                    fill_a(ab, mix(pal().bg, pal().text, if hot(Act::StartAll) { 0.14 } else { 0.08 }), a, sc(6));
                    stroke_a(ab, pal().text, 32, sc(6));
                } else {
                    surface(dc, ab, card, sc(6));
                    g(Act::StartAll, inset(ab, 1), card, mix(card, pal().text, 0.08), sc(5));
                }
                SelectObject(dc, f_small);
                text(dc, if all_mode { "‹ Back" } else { "All apps ›" }, ab, pal().text, DT_CENTER);
            }
            if f.is_empty() {
                SelectObject(dc, f_txt);
                if q_empty {
                    text(dc, "No pinned apps yet", r(28, 118, w - 28, 146), pal().dim, DT_CENTER);
                    SelectObject(dc, f_small);
                    text(dc, "Open 'All apps ›' and right-click an app → Add to Start", r(28, 148, w - 28, 174), mix(pal().dim, pal().bg, 0.25), DT_CENTER);
                } else {
                    text(dc, "No apps found", r(28, 92, w - 28, 128), pal().dim, DT_CENTER);
                }
            }
            rects_into(k, &mut rs);
            let apps = APPS.lock().unwrap_or_else(|e| e.into_inner());
            let grid_mode = q_empty && !all_mode;
            let just_info = JUST.with_borrow(|j| {
                let age = now_ms().saturating_sub(j.1) as f32 / 1200.0;
                if age < 1.0 && !j.0.is_empty() { Some((wide(&j.0), (1.0 - age) * (1.0 - age))) } else { None }
            });
            let icon_sz = if grid_mode { sc(32) } else { icon_px() };
            for (rc, a) in &rs {
                if let Act::App(i) = *a {
                    let Some(app) = f.get(i).and_then(|&ai| apps.get(ai)) else { continue };
                    let pulse = just_info.as_ref().filter(|(jw, _)| app.1 == *jw).map_or(0.0, |(_, p)| *p);
                    let sel = i == SEL.get() && !grid_mode;
                    if ulw {
                        let hov = hot(*a);
                        if pulse > 0.0 || sel || hov {
                            BUSY.set(BUSY.get() || pulse > 0.0);
                            let base_a = ((255 * sop as u32) / 100 + if sel || hov { 55 } else { 30 }).min(230) as u8;
                            let col = if pulse > 0.0 {
                                mix(pal().bg, pal().accent, 0.45 * pulse)
                            } else if sel {
                                mix(pal().hover, pal().accent, if hov { 0.22 } else { 0.14 })
                            } else {
                                mix(pal().bg, pal().text, 0.14)
                            };
                            fill_a(*rc, col, base_a, sc(6));
                        }
                    } else {
                        if pulse > 0.0 || sel {
                            BUSY.set(BUSY.get() || pulse > 0.0);
                            fill(dc, *rc, mix(if sel { mix(pal().bg, pal().accent, 0.14) } else { pal().bg }, pal().accent, 0.45 * pulse), sc(6));
                        }
                        g(*a, *rc, if sel { mix(pal().bg, pal().accent, 0.14) } else { pal().bg }, if sel { mix(pal().hover, pal().accent, 0.18) } else { pal().hover }, sc(6));
                    }
                    let ic = ICONS.with_borrow_mut(|m| {
                        let key = (app.1.clone(), icon_sz);
                        if let Some(&h) = m.get(&key) {
                            h
                        } else {
                            let t = lnk_target(&app.1);
                            let h = lnk_icon_sz(&app.1, &t, icon_sz);
                            m.insert(key, h);
                            h
                        }
                    });
                    if grid_mode {
                        if !ic.is_null() {
                            draw_icon(dc, (rc.left + rc.right - icon_sz) / 2, rc.top + sc(10), icon_sz, ic);
                        }
                        SelectObject(dc, f_small);
                        text(dc, &app.0, RECT { left: rc.left + sc(4), top: rc.top + sc(48), right: rc.right - sc(4), bottom: rc.bottom - sc(4) }, pal().text, DT_CENTER);
                    } else {
                        if sel {
                            fill(dc, RECT { left: rc.left, top: rc.top + sc(9), right: rc.left + sc(3), bottom: rc.bottom - sc(9) }, pal().accent, sc(3));
                        }
                        if !ic.is_null() {
                            draw_icon(dc, rc.left + sc(13), (rc.top + rc.bottom - icon_sz) / 2, icon_sz, ic);
                        }
                        let store = wstr_of(&app.1).starts_with("shell:");
                        SelectObject(dc, f_txt);
                        text(dc, &app.0, RECT { left: rc.left + sc(52), top: rc.top, right: rc.right - sc(110), bottom: rc.bottom }, pal().text, DT_LEFT);
                        SelectObject(dc, f_small);
                        let kind = if sel && !q.is_empty() { "Enter ↵" } else if store { "App" } else { "" };
                        text(dc, kind, RECT { left: rc.right - sc(110), top: rc.top, right: rc.right - sc(14), bottom: rc.bottom }, pal().dim, DT_RIGHT);
                    }
                }
            }
            drop(apps);
            let av = r(24, h - 46, 54, h - 16);
            let fg_av = FRAME_G.get();
            let av_bmp = user_avatar();
            if !fg_av.is_null() && !av_bmp.is_null() {
                use windows_sys::Win32::Graphics::GdiPlus::{
                    GdipCreateBitmapFromScan0, GdipCreateTexture, GdipDeleteBrush, GdipDisposeImage, GdipDrawImageRectI,
                    GdipFillEllipse, GdipGetImageGraphicsContext, GdipSetInterpolationMode, GdipTranslateTextureTransform,
                    GpTexture, InterpolationModeHighQualityBicubic, WrapModeClamp,
                };
                let (aw, ah) = ((av.right - av.left).max(1), (av.bottom - av.top).max(1));
                thread_local! {
                    static AV_SCALED: Cell<(i32, i32, isize)> = const { Cell::new((0, 0, 0)) };
                }
                let (cw, ch, mut scaled_i) = AV_SCALED.get();
                if cw != aw || ch != ah || scaled_i == 0 {
                    if scaled_i != 0 {
                        GdipDisposeImage(scaled_i as *mut GpImage);
                    }
                    let mut scaled: *mut GpBitmap = null_mut();
                    if GdipCreateBitmapFromScan0(aw, ah, 0, 0x000E_200B, null(), &mut scaled) == 0 && !scaled.is_null() {
                        let mut sg: *mut GpGraphics = null_mut();
                        if GdipGetImageGraphicsContext(scaled as *mut GpImage, &mut sg) == 0 && !sg.is_null() {
                            GdipSetInterpolationMode(sg, InterpolationModeHighQualityBicubic);
                            GdipDrawImageRectI(sg, av_bmp as *mut GpImage, 0, 0, aw, ah);
                            GdipDeleteGraphics(sg);
                        }
                        scaled_i = scaled as isize;
                    } else {
                        scaled_i = 0;
                    }
                    AV_SCALED.set((aw, ah, scaled_i));
                }
                if scaled_i != 0 {
                    let mut tx: *mut GpTexture = null_mut();
                    if GdipCreateTexture(scaled_i as *mut GpImage, WrapModeClamp, &mut tx) == 0 && !tx.is_null() {
                        GdipTranslateTextureTransform(tx, av.left as f32, av.top as f32, 0);
                        GdipFillEllipse(fg_av, tx as *mut GpBrush, av.left as f32, av.top as f32, aw as f32, ah as f32);
                        GdipDeleteBrush(tx as *mut GpBrush);
                    }
                }
            } else {
                fill(dc, av, mix(card, pal().accent, 0.2), sc(30));
                SelectObject(dc, f_glyph);
                text(dc, icons::USER, av, pal().text, DT_CENTER);
            }
            SelectObject(dc, f_small);
            let note = NOTE.with_borrow(|n| n.clone());
            let hint = match CONFIRM.get() {
                2 => "Click again to restart".to_string(),
                3 => "Click again to shut down".to_string(),
                _ if !note.is_empty() => note,
                _ => user_display_name(),
            };
            text(dc, &hint, r(62, h - 50, w - 156, h - 12), pal().text, DT_LEFT);
            for (rc, a) in &rs {
                if let Act::Power(p) = *a {
                    let armed = CONFIRM.get() == p as i32;
                    if ulw {
                        if armed || hot(*a) {
                            let pa = ((bg_a as u32) + 55).min(240) as u8;
                            fill_a(*rc, if armed { BAD } else { mix(pal().bg, pal().text, 0.14) }, pa, sc(6));
                        }
                    } else {
                        glow(dc, armed || hot(*a), *rc, if armed { BAD } else { pal().hover }, sc(6));
                    }
                    SelectObject(dc, f_glyph);
                    text(dc, [icons::LOCK, icons::SLEEP, icons::RESTART, icons::POWER][p as usize], *rc, if armed { rgb(255, 255, 255) } else { pal().text }, DT_CENTER);
                }
            }
            if let Some((idx, mx, my)) = START_CTX.with_borrow(|c| *c) {
                let items = start_ctx_items(idx);
                let mr = RECT { left: mx, top: my, right: mx + sc(220), bottom: my + sc(12 + 32 * items.len() as i32) };
                fill(dc, RECT { left: mr.left - 1, top: mr.top + sc(2), right: mr.right + 1, bottom: mr.bottom + sc(4) }, mix(pal().bg, 0, if style().light() { 0.12 } else { 0.5 }), sc(10));
                surface(dc, mr, card, sc(8));
                for (j, (glyph, label, _)) in items.iter().enumerate() {
                    let ir = RECT { left: mx + sc(4), top: my + sc(6 + 32 * j as i32), right: mx + sc(216), bottom: my + sc(6 + 32 * (j as i32 + 1)) };
                    g(Act::StartCtx(j as u8), ir, card, mix(card, pal().text, 0.08), sc(6));
                    SelectObject(dc, f_glyph);
                    text(dc, glyph, RECT { left: ir.left + sc(6), right: ir.left + sc(28), ..ir }, pal().dim, DT_CENTER);
                    SelectObject(dc, f_txt);
                    text(dc, label, RECT { left: ir.left + sc(32), right: ir.right - sc(8), ..ir }, pal().text, DT_LEFT);
                }
            }
        }
    }

    PAINT_RECTS.set(rs);
    SelectObject(dc, of);
    LayeredBuf::present(hw, cr.right, cr.bottom, dc);
    if BUSY.get() {
        SetTimer(hw, 2, 16, None);
    } else {
        KillTimer(hw, 2);
    }
}

unsafe extern "system" fn proc(hw: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if hw != FH.get() {
        return DefWindowProcW(hw, msg, wp, lp);
    }
    let Some(k) = FLY.get() else { return DefWindowProcW(hw, msg, wp, lp) };
    let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16 & 0xffff) as i16 as i32);
    match msg {
        WM_PAINT => paint(hw, k),
        0x02E0 => {
            let dpi = ((wp >> 16) & 0xFFFF) as f32;
            if dpi > 0.0 {
                SCALE.set(dpi / 96.0);
                let (w, h) = (sc(size(k).0), sc(size(k).1));
                let r = &*(lp as *const RECT);
                FIN.set((r.left, r.top));
                SetWindowPos(hw, null_mut(), r.left, r.top, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_ERASEBKGND => return 1,
        WM_ACTIVATE if wp & 0xffff == 0 && !PREVIEW.load(Relaxed) => {
            if OPENING.get() {
                return 0;
            }
            if now_ms().saturating_sub(T0.get()) < 150 {
                SetWindowPos(hw, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                force_fg(hw);
                return 0;
            }
            if k == Kind::Start {
                DEACT_START_MS.set(now_ms());
            }
            close();
        }
        WM_DROPFILES => on_files(hw, wp as HDROP),
        WM_TIMER if wp == 2 => {
            InvalidateRect(hw, null(), 0);
        }
        WM_TIMER => {
            if k == Kind::Wifi && PASS.with_borrow(|p| p.is_none()) {
                read_nets();
                let (w, h) = (sc(size(k).0), sc(size(k).1));
                let (x, _) = FIN.get();
                let y = pop_y(h);
                FIN.set((x, y));
                SetWindowPos(hw, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_MOUSEMOVE => {
            if DRAG.get() != 0 {
                let want = if DRAG.get() == 1 { Act::VolSlide } else { Act::BrightSlide };
                let found = RECTS_BUF.with_borrow_mut(|v| {
                    rects_into(k, v);
                    v.iter().find(|(_, a)| *a == want).copied()
                });
                if let Some((rc, a)) = found {
                    slide(a, x, rc);
                }
            }
            let o = act_at(k, x, y).map(|a| a.1);
            if OVER.replace(o) != o || DRAG.get() != 0 {
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_LBUTTONDOWN => {
            let hit = act_at(k, x, y);
            if START_CTX.with_borrow(|c| c.is_some()) && !matches!(hit, Some((_, Act::StartCtx(_)))) {
                START_CTX.set(None);
            } else if CTX.with_borrow(|c| c.is_some()) && !matches!(hit, Some((_, Act::Ctx(_)))) {
                CTX.set(None);
            } else if let Some((rc, a)) = hit {
                run_act(a, rc, x);
            }
            if FLY.get().is_some() {
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_RBUTTONUP => {
            match (k, act_at(k, x, y)) {
                (Kind::Start, Some((_, Act::App(i)))) => app_menu(hw, i, x, y),
                (Kind::Wifi, Some((_, Act::Net(_) | Act::NetOff))) if PASS.with_borrow(|p| p.is_none()) => {
                    let i = if let Some((_, Act::Net(i))) = act_at(k, x, y) { i } else { NETS.with_borrow(|n| n.iter().position(|n| n.connected)).unwrap_or(0) };
                    if let Some(ssid) = NETS.with_borrow(|n| n.get(i).map(|n| n.ssid.clone())) {
                        let mut cr: RECT = zeroed();
                        GetClientRect(hw, &mut cr);
                        let (pw, ph) = (cr.right.max(sc(220)), cr.bottom.max(sc(160)));
                        let mh = sc(12 + CTX_ROW * ctx_items(&ssid).len() as i32);
                        CTX.set(Some((ssid, x.min(pw - sc(CTX_W) - sc(8)).max(sc(8)), y.min(ph - mh - sc(8)).max(sc(8)), now_ms())));
                    }
                }
                _ => {}
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_LBUTTONUP => {
            let d = DRAG.replace(0);
            if d != 0 {
                ReleaseCapture();
                if d == 2 {
                    set_brightness(BRIGHT.load(Relaxed));
                }
            }
        }
        WM_MOUSEWHEEL => {
            let up = (wp >> 16) as i16 > 0;
            match k {
                Kind::Quick => add_volume(if up { 2 } else { -2 }),
                Kind::Start => SCROLL.set(if up { SCROLL.get().saturating_sub(2) } else { (SCROLL.get() + 2).min(filtered().len().saturating_sub(start_rows())) }),
                Kind::Menu => {
                    let max_s = (menu_rows().1 - size(Kind::Menu).1).max(0);
                    MENU_SCROLL.set(if up { (MENU_SCROLL.get() - 40).max(0) } else { (MENU_SCROLL.get() + 40).min(max_s) });
                }
                _ => {}
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_KEYDOWN if k == Kind::Start && (wp == VK_DOWN as usize || wp == VK_UP as usize) => {
            let n = filtered().len();
            if n > 0 {
                let s = if wp == VK_DOWN as usize { (SEL.get() + 1).min(n - 1) } else { SEL.get().saturating_sub(1) };
                SEL.set(s);
                SCROLL.set(SCROLL.get().min(s).max((s + 1).saturating_sub(start_rows())));
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_CHAR => {
            let c = wp as u32;
            if c == 27 {
                if START_CTX.with_borrow(|c| c.is_some()) {
                    START_CTX.set(None);
                } else if CTX.with_borrow(|c| c.is_some()) {
                    CTX.set(None);
                } else if PASS.with_borrow(|p| p.is_some()) {
                    PASS.set(None);
                } else if k == Kind::Quick && QUICK_SUB.get() != 0 {
                    QUICK_SUB.set(0);
                } else if k == Kind::Start && QUERY.with_borrow(|q| !q.is_empty()) {
                    QUERY.set(String::new());
                    SCROLL.set(0);
                    SEL.set(0);
                } else {
                    close();
                    return 0;
                }
            } else if (k == Kind::Wifi || (k == Kind::Quick && QUICK_SUB.get() == 2)) && PASS.with_borrow(|p| p.is_some()) {
                if c == 13 {
                    if let Some((i, p)) = PASS.take() {
                        NETS.with_borrow(|n| wifi_join(&n[i], &p));
                    }
                } else if let Some(p) = PASS.with_borrow_mut(|p| p.as_mut().map(|p| &mut p.1 as *mut String)) {
                    let p = &mut *p;
                    if c == 8 {
                        p.pop();
                    } else if let Some(ch) = char::from_u32(c).filter(|_| c >= 32) {
                        p.push(ch);
                    }
                }
            } else if k == Kind::Start {
                if c == 13 {
                    if launch(SEL.get()) {
                        close();
                        return 0;
                    }
                } else {
                    QUERY.with_borrow_mut(|q| {
                        let all = SELALL.replace(false);
                        if c == 1 {
                            SELALL.set(!q.is_empty());
                        } else if c == 127 {
                            if all { q.clear() } else { let t = q.trim_end().len(); q.truncate(q[..t].rfind(' ').map_or(0, |i| i + 1)); }
                        } else if c == 8 {
                            if all { q.clear() } else { q.pop(); }
                        } else if let Some(ch) = char::from_u32(c).filter(|_| c >= 32) {
                            if all { q.clear() }
                            q.push(ch);
                        }
                    });
                    SCROLL.set(0);
                    SEL.set(0);
                }
            }
            InvalidateRect(hw, null(), 0);
        }
        _ => return DefWindowProcW(hw, msg, wp, lp),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{flip_layout, score, init, is_open, toggle, close, Kind, FLY, FH, GetModuleHandleW, SetProcessDPIAware, null, null_mut, DestroyWindow, IsWindow};

    #[test]
    fn layout_flip_both_ways() {
        assert_eq!(flip_layout("сркщьу"), "chrome");
        assert_eq!(flip_layout("ntktuhfv"), "телеграм");
        assert_eq!(flip_layout("vs code"), "мы сщву");
    }

    #[test]
    fn win_button_always_opens_start() {
        unsafe {
            SetProcessDPIAware();
            init(GetModuleHandleW(null()));
            toggle(Kind::Start);
            assert!(is_open(Kind::Start), "the first press opens Start");
            toggle(Kind::Start);
            assert!(!is_open(Kind::Start), "the second press closes it");
            toggle(Kind::Start);
            assert!(is_open(Kind::Start), "a press right after the close opens it again");
            let live = FH.get();
            assert!(!live.is_null() && IsWindow(live) != 0);
            DestroyWindow(live);
            close();
            FLY.set(Some(Kind::Start));
            FH.set(null_mut());
            toggle(Kind::Start);
            assert!(is_open(Kind::Start) && IsWindow(FH.get()) != 0, "the press opens a real panel");
            close();
        }
    }

    #[test]
    fn ranking() {
        assert_eq!(score("visual studio code", "vis"), Some(0));
        assert_eq!(score("visual studio code", "cod"), Some(1));
        assert_eq!(score("visual studio code", "vsc"), Some(2));
        assert_eq!(score("calculator", "cul"), Some(3));
        assert_eq!(score("calculator", "xyz"), None);
        assert_eq!(score("calculator", ""), None);
        assert_eq!(score("v", "v"), Some(0));
    }
}
