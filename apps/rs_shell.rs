#![windows_subsystem = "windows"]
use std::{
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, Command},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM, RECT},
        UI::{
            Input::KeyboardAndMouse::{MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey},
            WindowsAndMessaging::*,
        },
    },
    core::BOOL,
};

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWinEventHook(
        event_min: u32,
        event_max: u32,
        hmod_win_event_proc: *mut core::ffi::c_void,
        pfn_win_event_proc: Option<unsafe extern "system" fn(isize, u32, HWND, i32, i32, u32, u32)>,
        id_process: u32,
        id_thread: u32,
        dw_flags: u32,
    ) -> isize;
    fn UnhookWinEvent(h_win_event_hook: isize) -> BOOL;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(dw_desired_access: u32, b_inherit_handle: BOOL, dw_process_id: u32) -> HANDLE;
    fn WaitForSingleObject(h_handle: HANDLE, dw_milliseconds: u32) -> u32;
    fn WaitForMultipleObjects(n_count: u32, lp_handles: *const HANDLE, b_wait_all: BOOL, dw_milliseconds: u32) -> u32;
}

const TOPBAR_CLS: [u16; 7] = [b't' as u16, b'o' as u16, b'p' as u16, b'b' as u16, b'a' as u16, b'r' as u16, 0];
const PROGMAN_CLS: [u16; 8] = [b'P' as u16, b'r' as u16, b'o' as u16, b'g' as u16, b'm' as u16, b'a' as u16, b'n' as u16, 0];
const RS_DESKTOP_TITLE: [u16; 11] = [b'r' as u16, b's' as u16, b'-' as u16, b'd' as u16, b'e' as u16, b's' as u16, b'k' as u16, b't' as u16, b'o' as u16, b'p' as u16, 0];

const WINLOGON: &str = r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon";
const NO_WINDOW: u32 = 0x0800_0000;

fn find_ic(s: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    s.as_bytes().windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

fn contains_ic(s: &str, needle: &str) -> bool {
    find_ic(s, needle).is_some()
}

use winrust_common::with_wide;

fn quiet(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(NO_WINDOW);
    c
}

fn reg(args: &[&str]) -> Option<String> {
    let o = Command::new("reg").args(args).creation_flags(NO_WINDOW).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn parse_run(out: &str) -> Vec<String> {
    parse_run_named(out).into_iter().map(|(_, c)| c).collect()
}

fn parse_run_named(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let (n, v) = l.split_once("    REG_EXPAND_SZ").or_else(|| l.split_once("    REG_SZ"))?;
            Some((n.trim().to_string(), v.trim().to_string())).filter(|(_, v)| !v.is_empty())
        })
        .collect()
}

fn parse_disabled(out: &str) -> std::collections::HashSet<String> {
    out.lines()
        .filter_map(|l| {
            let (n, v) = l.split_once("    REG_BINARY")?;
            let first = u8::from_str_radix(v.trim().get(..2)?, 16).ok()?;
            (first & 1 == 1).then(|| n.trim().to_lowercase())
        })
        .collect()
}

const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

fn exe_path(cmd: &str) -> Option<String> {
    let mut c = cmd.trim().to_string();
    while let Some(a) = c.find('%') {
        let b = c[a + 1..].find('%')?;
        let val = std::env::var(&c[a + 1..a + 1 + b]).ok()?;
        c = format!("{}{}{}", &c[..a], val, &c[a + b + 2..]);
    }
    let path = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?.to_string(),
        None => c[..find_ic(&c, ".exe")? + 4].to_string(),
    };
    path.get(path.len().saturating_sub(4)..).is_some_and(|e| e.eq_ignore_ascii_case(".exe")).then_some(path)
}

fn autostart() -> Vec<String> {
    let mut v = vec![];
    let off = |key: String| reg(&["query", &key]).map(|o| parse_disabled(&o)).unwrap_or_default();
    let runs = [
        (r"HKLM\Software\Microsoft\Windows\CurrentVersion\Run", format!(r"HKLM\{APPROVED}\Run")),
        (r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run", format!(r"HKLM\{APPROVED}\Run32")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", format!(r"HKCU\{APPROVED}\Run")),
    ];
    for (key, approved) in runs {
        let disabled = off(approved);
        v.extend(
            reg(&["query", key])
                .map(|o| parse_run_named(&o))
                .unwrap_or_default()
                .into_iter()
                .filter(|(n, _)| !disabled.contains(&n.to_lowercase()))
                .map(|(_, c)| c),
        );
    }
    let dirs = [
        (std::env::var("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\Startup")), format!(r"HKCU\{APPROVED}\StartupFolder")),
        (std::env::var("ProgramData").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\StartUp")), format!(r"HKLM\{APPROVED}\StartupFolder")),
    ];
    for (d, approved) in dirs {
        let Ok(d) = d else { continue };
        let disabled = off(approved);
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name == "desktop.ini" || disabled.contains(&name) {
                continue;
            }
            v.push(format!("\"{}\"", e.path().display()));
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|c| !contains_ic(c, "rs-shell") && !contains_ic(c, r"\topbar\") && seen.insert(c.to_lowercase()));
    v.retain(|c| exe_path(c).is_none_or(|p| std::path::Path::new(&p).exists()));
    v
}

fn dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_default()
}

fn find_exe(name: &str) -> PathBuf {
    let own = dir().join(name);
    if own.exists() {
        return own;
    }
    PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())).join("topbar").join(name)
}

fn bar_hwnd() -> HWND {
    unsafe { FindWindowW(TOPBAR_CLS.as_ptr(), std::ptr::null()) }
}
fn bar_up() -> bool {
    !bar_hwnd().is_null()
}

fn hwnd_proc(h: HWND) -> HANDLE {
    if h.is_null() {
        return ptr::null_mut();
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    if pid == 0 {
        return ptr::null_mut();
    }
    unsafe { OpenProcess(0x0010_0000, 0, pid) }
}

unsafe extern "system" fn on_minimize(_: isize, _: u32, h: HWND, id_obj: i32, id_child: i32, _: u32, _: u32) {
    if id_obj == 0 && id_child == 0 && !h.is_null() {
        unsafe { park(h, 0) };
    }
}

unsafe extern "system" fn park(h: HWND, _: LPARAM) -> BOOL {
    unsafe {
        if IsWindowVisible(h) != 0 && IsIconic(h) != 0 {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(h, &mut r);
            if r.left > -30000 {
                SetWindowPos(h, ptr::null_mut(), -32000, -32000, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        1
    }
}

fn to_bmp(src: &str, dst: &std::path::Path) {
    use windows_sys::{Win32::Graphics::GdiPlus::*, core::GUID};
    const BMP: GUID = GUID { data1: 0x557C_F400, data2: 0x1A04, data3: 0x11D3, data4: [0x9A, 0x73, 0x00, 0x00, 0xF8, 0x1E, 0xF3, 0x2E] };
    unsafe {
        let (mut token, mut input): (usize, GdiplusStartupInput) = (0, std::mem::zeroed());
        input.GdiplusVersion = 1;
        if GdiplusStartup(&mut token, &input, ptr::null_mut()) != 0 {
            return;
        }
        let mut img = ptr::null_mut();
        let loaded = with_wide(src, |w| GdipLoadImageFromFile(w, &mut img));
        if loaded == 0 {
            with_wide(&dst.to_string_lossy(), |w| {
                GdipSaveImageToFile(img, w, &BMP, ptr::null());
            });
            GdipDisposeImage(img);
        }
        GdiplusShutdown(token);
    }
}

fn wallpaper_value() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = with_wide(r"Control Panel\Desktop", |sub| {
        with_wide("WallPaper", |val| unsafe {
            RegGetValueW(HKEY_CURRENT_USER, sub, val, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
        })
    });
    if r != 0 {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.trim().is_empty())
}

fn wallpaper_source() -> Option<PathBuf> {
    let transcoded = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Themes\TranscodedWallpaper"));
    wallpaper_value().map(PathBuf::from).into_iter().chain(transcoded).find(|p| p.is_file())
}

fn wallpaper_bmp() -> Option<PathBuf> {
    let src = wallpaper_source()?;
    if src.extension().is_some_and(|e| e.eq_ignore_ascii_case("bmp")) {
        return Some(src);
    }
    let bmp = bmp_copy()?;
    let modified = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if !bmp.is_file() || modified(&bmp) < modified(&src) {
        let _ = std::fs::create_dir_all(bmp.parent()?);
        let _ = std::fs::remove_file(&bmp);
        to_bmp(&src.to_string_lossy(), &bmp);
    }
    bmp.is_file().then_some(bmp)
}

fn bmp_copy() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|b| PathBuf::from(b).join("rs-shell").join("wallpaper.bmp"))
}

fn set_wallpaper() {
    let Some(path) = wallpaper_bmp() else { return };
    with_wide(&path.to_string_lossy(), |w| unsafe {
        SystemParametersInfoW(0x14, 0, w as _, 0);
    });
}

fn log(msg: &str) {
    winrust_common::log("rs-shell", msg);
}

fn run_shell() {
    winrust_common::signal_shell_ready();
    log("start");
    let mut mm = MINIMIZEDMETRICS { cbSize: size_of::<MINIMIZEDMETRICS>() as u32, iWidth: 0, iHorzGap: 0, iVertGap: 0, iArrange: ARW_HIDE as i32 };
    unsafe { SystemParametersInfoW(SPI_SETMINIMIZEDMETRICS, mm.cbSize, &mut mm as *mut _ as _, 0) };
    let wp = thread::spawn(set_wallpaper);
    let bar_child = if !bar_up() {
        let r = quiet(find_exe("topbar.exe")).spawn();
        log(&format!("topbar spawn -> {:?}", r.as_ref().map(|_| ())));
        r.ok()
    } else {
        None
    };
    let hotkeys = find_exe("reshell.exe");
    if hotkeys.exists() {
        log(&format!("reshell spawn -> {:?}", quiet(&hotkeys).spawn().map(|_| ())));
    }
    let desktop = dir().join("rust-explorer.exe");
    let desktop_up = || unsafe { !FindWindowW(PROGMAN_CLS.as_ptr(), RS_DESKTOP_TITLE.as_ptr()).is_null() };
    log(&format!("desktop exe {} exists={} up={}", desktop.display(), desktop.exists(), desktop_up()));
    if desktop.exists() && !desktop_up() {
        log(&format!("desktop spawn -> {:?}", quiet(&desktop).arg("--desktop").spawn().map(|_| ())));
    }
    thread::spawn(move || {
        let list = autostart();
        log(&format!("autostart: {} entries", list.len()));
        for c in list {
            let r = quiet("cmd").raw_arg(format!("/c start \"\" {c}")).spawn();
            log(&format!("start {c} -> {:?}", r.map(|_| ())));
        }
        let _ = wp.join();
        thread::sleep(Duration::from_secs(4));
        set_wallpaper();
    });
    unsafe { EnumWindows(Some(park), 0) };
    thread::spawn(move || supervise(bar_child));
    hotkey_loop();
}

fn hotkey_loop() {
    unsafe {
        let hook = SetWinEventHook(0x0016, 0x0016, ptr::null_mut(), Some(on_minimize), 0, 0, 0);
        RegisterHotKey(ptr::null_mut(), 1, MOD_WIN | MOD_NOREPEAT, 'D' as u32);
        RegisterHotKey(ptr::null_mut(), 2, MOD_WIN | MOD_NOREPEAT, 'E' as u32);
        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, ptr::null_mut(), 0, 0) > 0 {
            match (m.message == WM_HOTKEY).then_some(m.wParam) {
                Some(1) => winrust_common::desktop::toggle(),
                Some(2) => {
                    let fm = dir().join("rust-explorer.exe");
                    let _ = quiet(if fm.exists() { fm } else { "explorer.exe".into() }).spawn();
                }
                _ => {}
            }
        }
        UnregisterHotKey(ptr::null_mut(), 1);
        UnregisterHotKey(ptr::null_mut(), 2);
        if hook != 0 {
            UnhookWinEvent(hook);
        }
    }
}

fn supervise(mut bar_child: Option<Child>) {
    let bar_exe = find_exe("topbar.exe");
    let (mut starts, mut since) = (0, Instant::now());
    loop {
        if let Some(mut c) = bar_child.take() {
            let _ = c.wait();
        } else {
            let bar = bar_hwnd();
            if !bar.is_null() {
                let h = hwnd_proc(bar);
                if !h.is_null() {
                    unsafe {
                        WaitForSingleObject(h, 0xFFFF_FFFF);
                        CloseHandle(h);
                    }
                    continue;
                }
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        }
        if since.elapsed() > Duration::from_secs(120) {
            (starts, since) = (0, Instant::now());
        }
        starts += 1;
        if starts > 3 {
            let _ = quiet(&bar_exe).arg("--restore").status();
            let _ = quiet("explorer.exe").spawn();
            std::process::exit(0);
        }
        match quiet(&bar_exe).spawn() {
            Ok(c) => bar_child = Some(c),
            Err(_) => {
                let _ = quiet(&bar_exe).arg("--restore").status();
                let _ = quiet("explorer.exe").spawn();
                std::process::exit(0);
            }
        }
    }
}

fn replace_on() -> bool {
    winrust_common::reg::get_dword(r"HKCU\Software\rs-shell", "ReplaceExplorer") == Some(1)
}

fn kill_shell_explorer() {
    winrust_common::proc::kill_shell_explorers();
    log("explorer shell stopped");
}

const AUTOSTART_GRACE: Duration = Duration::from_secs(90);

fn startup_run() -> bool {
    let mut sid = 0;
    unsafe { windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(std::process::id(), &mut sid) };
    let k = format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{sid}");
    ["RunStuffHasBeenRun", "StartupHasBeenRun"].iter().all(|n| winrust_common::reg::exists(&format!(r"{k}\{n}"), None))
}

fn expected_autostart(list: &[String]) -> (Vec<String>, usize) {
    let exes: Vec<String> = list
        .iter()
        .filter_map(|c| {
            if let Some(i) = find_ic(c, "--processstart") {
                let app = c[i + "--processstart".len()..].trim_start().trim_start_matches('"');
                return app.split(['"', ' ']).next().filter(|a| a.get(a.len().saturating_sub(4)..).is_some_and(|e| e.eq_ignore_ascii_case(".exe"))).map(str::to_lowercase);
            }
            exe_path(c).and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().to_lowercase()))
        })
        .collect();
    let untracked = list.len() - exes.len();
    (exes, untracked)
}

fn missing_autostart<'a>(expected: &'a [String], running: &[(u32, String)]) -> Vec<&'a str> {
    expected.iter().filter(|e| !running.iter().any(|(_, n)| n.eq_ignore_ascii_case(e))).map(String::as_str).collect()
}

fn may_end_explorer(markers: bool, since_start: Duration, all_running: bool, untracked: usize) -> bool {
    let settle = Duration::from_secs(if untracked > 0 { 30 } else { 15 });
    markers || since_start >= AUTOSTART_GRACE || (all_running && since_start >= settle)
}

fn start_shell_explorer() {
    let r = winrust_common::proc::start_shell_explorer();
    log(&format!("explorer shell started -> {:?}", r.map(|_| ())));
}

fn overlay() {
    let replace = replace_on();
    log(&format!("overlay start (replace explorer: {replace})"));
    let wp = thread::spawn(wallpaper_bmp);
    winrust_common::wait_shell_ready();
    let bar_exe = find_exe("topbar.exe");
    let has_bar = bar_exe.exists();
    if has_bar && !bar_up() {
        log(&format!("topbar spawn -> {:?}", quiet(&bar_exe).spawn().map(|_| ())));
    }
    let hk = find_exe("reshell.exe");
    if hk.exists() {
        log(&format!("reshell spawn -> {:?}", quiet(&hk).spawn().map(|_| ())));
    }
    let _ = wp.join();
    let desktop = dir().join("rust-explorer.exe");
    let up = || unsafe { !FindWindowW(PROGMAN_CLS.as_ptr(), RS_DESKTOP_TITLE.as_ptr()).is_null() };
    if desktop.exists() && !up() {
        log(&format!("desktop spawn -> {:?}", quiet(&desktop).arg("--desktop").spawn().map(|_| ())));
    }
    if replace {
        thread::spawn(move || overlay_watch(replace, has_bar, desktop, up));
        hotkey_loop();
    } else {
        overlay_watch(replace, has_bar, desktop, up);
    }
}

fn overlay_watch(replace: bool, has_bar: bool, desktop: PathBuf, up: impl Fn() -> bool) {
    let has_desk = desktop.exists();
    let bar_exe = find_exe("topbar.exe");
    let (mut starts, mut misses, mut dmiss, mut stable, mut killed) = (0, 0, 0, 0, false);
    let began = Instant::now();
    let (expected, untracked) = if replace { expected_autostart(&autostart()) } else { (vec![], 0) };
    if replace {
        log(&format!("autostart to wait for: {} exe(s) {:?}, {} other entr(ies)", expected.len(), expected, untracked));
    }
    loop {
        if !replace || killed {
            let bar = if has_bar { bar_hwnd() } else { ptr::null_mut() };
            let desk = if has_desk { unsafe { FindWindowW(PROGMAN_CLS.as_ptr(), RS_DESKTOP_TITLE.as_ptr()) } } else { ptr::null_mut() };
            let mut handles: [HANDLE; 2] = [ptr::null_mut(); 2];
            let mut count = 0u32;
            let hb = hwnd_proc(bar);
            if !hb.is_null() {
                handles[count as usize] = hb;
                count += 1;
            }
            let hd = hwnd_proc(desk);
            if !hd.is_null() {
                handles[count as usize] = hd;
                count += 1;
            }
            if (!has_bar || !bar.is_null()) && (!has_desk || !desk.is_null()) && count > 0 {
                unsafe {
                    WaitForMultipleObjects(count, handles.as_ptr(), 0, 0xFFFF_FFFF);
                    for h in &handles[..count as usize] {
                        CloseHandle(*h);
                    }
                }
                continue;
            }
            for h in &handles[..count as usize] {
                unsafe { CloseHandle(*h) };
            }
        }
        thread::sleep(Duration::from_secs(if !replace || killed { 5 } else { 3 }));
        if replace && !killed {
            stable = if (!has_bar || bar_up()) && (!has_desk || up()) { stable + 1 } else { 0 };
            if stable >= 3 {
                let procs = winrust_common::proc::list();
                let missing = missing_autostart(&expected, &procs);
                let markers = startup_run();
                if may_end_explorer(markers, began.elapsed(), missing.is_empty(), untracked) {
                    log(&format!("autostart done (markers: {markers}, after {} s, not running: {missing:?})", began.elapsed().as_secs()));
                    kill_shell_explorer();
                    killed = true;
                }
            }
        }
        let lent = || winrust_common::settings::explorer_lent() || (replace && killed && !winrust_common::proc::shell_explorers().is_empty());
        dmiss = if has_desk && !up() && !lent() { dmiss + 1 } else { 0 };
        if dmiss >= 2 {
            log(&format!("desktop respawn -> {:?}", quiet(&desktop).arg("--desktop").spawn().map(|_| ())));
            dmiss = 0;
        }
        if !has_bar || bar_up() {
            misses = 0;
            continue;
        }
        misses += 1;
        if misses < 2 {
            continue;
        }
        starts += 1;
        if starts > 3 || quiet(&bar_exe).spawn().is_err() {
            let _ = quiet(&bar_exe).arg("--restore").status();
            if replace {
                start_shell_explorer();
            }
            log("overlay: topbar keeps dying, system shell restored");
            return;
        }
        misses = 0;
    }
}

fn bootstrap() -> bool {
    let Ok(base) = std::env::var("LOCALAPPDATA") else { return false };
    let real = PathBuf::from(base).join(r"plugin-launcher\bin\rs-shell.exe");
    let me = std::env::current_exe().and_then(|p| p.canonicalize()).ok();
    if !real.exists() || real.canonicalize().ok() == me {
        return false;
    }
    let t = Instant::now();
    match quiet(&real).spawn() {
        Ok(mut child) => {
            log(&format!("loader: started {}", real.display()));
            let _ = child.wait();
            t.elapsed() > Duration::from_secs(3)
        }
        Err(e) => {
            log(&format!("loader: cannot start {}: {e}", real.display()));
            false
        }
    }
}

fn main() {
    winrust_common::install_panic_hook("rs-shell");
    let a: Vec<String> = std::env::args().skip(1).collect();
    if !a.is_empty() && a.first().map(String::as_str) != Some("--overlay") {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
    }
    let code = match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => {
            if !bootstrap() {
                run_shell();
            }
            0
        }
        ["--overlay"] => {
            overlay();
            0
        }
        ["--dry-run"] => {
            println!("wallpaper: {:?}", reg(&["query", r"HKCU\Control Panel\Desktop", "/v", "WallPaper"]).map(|o| parse_run(&o)));
            autostart().iter().for_each(|c| println!("autostart: {c}"));
            let bar = find_exe("topbar.exe");
            println!("topbar: {} ({})", bar.display(), if bar.exists() { "found" } else { "MISSING" });
            println!("explorer ran this session's autostart: {}", startup_run());
            0
        }
        ["--enable"] => {
            let exe = std::env::current_exe().unwrap_or_default();
            if !find_exe("topbar.exe").exists() {
                eprintln!("refusing: topbar.exe is not next to {} (no bar = no way to launch anything)", exe.display());
                1
            } else if reg(&["add", WINLOGON, "/v", "Shell", "/t", "REG_SZ", "/d", &exe.to_string_lossy(), "/f"]).is_some() {
                println!("done: you get rs-shell instead of explorer at the next sign-in.\nundo: rs-shell --disable   (or: reg delete \"{WINLOGON}\" /v Shell /f)");
                0
            } else {
                eprintln!("reg add failed");
                1
            }
        }
        ["--disable"] => {
            let _ = reg(&["delete", WINLOGON, "/v", "Shell", "/f"]);
            println!("done: normal shell at the next sign-in");
            0
        }
        _ => {
            eprintln!("usage: rs-shell [--enable | --disable | --dry-run]");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::{exe_path, may_end_explorer, parse_run, to_bmp};
    use std::time::Duration;
    #[test]
    fn converts_a_jpeg_to_bmp() {
        let src = r"C:\Windows\Web\Wallpaper\Windows\img0.jpg";
        if !std::path::Path::new(src).is_file() {
            return;
        }
        let dst = std::env::temp_dir().join("rs-shell-test.bmp");
        let _ = std::fs::remove_file(&dst);
        to_bmp(src, &dst);
        assert_eq!(&std::fs::read(&dst).unwrap()[..2], b"BM");
        let _ = std::fs::remove_file(dst);
    }
    #[test]
    fn ends_explorer_on_markers_or_after_grace() {
        let s = Duration::from_secs;
        assert!(may_end_explorer(true, Duration::ZERO, false, 0));
        assert!(!may_end_explorer(false, s(30), false, 0));
        assert!(may_end_explorer(false, s(90), false, 3));
        assert!(!may_end_explorer(false, s(9), true, 0));
        assert!(may_end_explorer(false, s(15), true, 0));
        assert!(!may_end_explorer(false, s(20), true, 1));
        assert!(may_end_explorer(false, s(30), true, 1));
    }
    #[test]
    fn watches_the_autostart_exes() {
        let list = [r#""C:\a b\Steam.exe" -silent"#.to_string(), r"C:\d\y.exe /background".to_string(), r#""C:\Users\u\Startup\x.lnk""#.to_string()];
        let (exes, untracked) = super::expected_autostart(&list);
        assert_eq!((exes.clone(), untracked), (vec!["steam.exe".to_string(), "y.exe".to_string()], 1));
        let running = [(1, "STEAM.EXE".to_string()), (2, "explorer.exe".to_string())];
        assert_eq!(super::missing_autostart(&exes, &running), ["y.exe"]);
        assert!(super::missing_autostart(&exes, &[(1, "steam.exe".into()), (3, "y.exe".into())]).is_empty());
        let sq = [r#"C:\Users\u\AppData\Local\Discord\Update.exe --processStart Discord.exe"#.to_string(), r#""C:\S\Update.exe" --processStart "slack.exe" --process-start-args "--startup""#.to_string()];
        assert_eq!(super::expected_autostart(&sq).0, ["discord.exe", "slack.exe"]);
    }
    #[test]
    fn finds_the_exe_in_run_commands() {
        assert_eq!(exe_path(r#""C:\a b\x.exe" --hidden"#).as_deref(), Some(r"C:\a b\x.exe"));
        assert_eq!(exe_path(r"C:\d\y.exe /background").as_deref(), Some(r"C:\d\y.exe"));
        assert_eq!(exe_path("script.bat"), None);
        assert!(exe_path(r"%windir%\system32\z.exe").unwrap().to_lowercase().ends_with(r"system32\z.exe"));
    }
    #[test]
    fn parses_reg_query_lines() {
        let out = "\nHKEY_CURRENT_USER\\X\n    Foo    REG_SZ    foo.exe --x\n    Bar    REG_EXPAND_SZ    %APPDATA%\\bar.exe\n    Empty    REG_SZ    \n";
        assert_eq!(parse_run(out), ["foo.exe --x", "%APPDATA%\\bar.exe"]);
        assert_eq!(super::parse_run_named("    Steam Client    REG_SZ    \"C:\\s\\steam.exe\" -silent\n"), [("Steam Client".to_string(), "\"C:\\s\\steam.exe\" -silent".to_string())]);
    }
    #[test]
    fn reads_task_managers_disabled_entries() {
        let out = "\nHKEY_CURRENT_USER\\...\\StartupApproved\\Run\n    Discord    REG_BINARY    030000006A1B2C3D4E5F6071\n    OneDrive    REG_BINARY    020000000000000000000000\n    Spotify Web    REG_BINARY    07000000FFFFFFFF00000000\n    Odd    REG_BINARY    06\n    Junk    REG_BINARY    \n";
        let off = super::parse_disabled(out);
        assert!(off.contains("discord") && off.contains("spotify web"));
        assert!(!off.contains("onedrive") && !off.contains("odd") && !off.contains("junk"));
    }
}
