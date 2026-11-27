use crate::*;
#[path = "explorer.rs"]
mod explorer;
#[path = "journal.rs"]
mod journal;
#[path = "search.rs"]
mod search;
#[path = "session.rs"]
mod session;
#[path = "taskmgr.rs"]
mod taskmgr;
#[path = "tools.rs"]
mod tools;
#[path = "topbar.rs"]
mod topbar;
pub(crate) use {explorer::*, journal::*, search::*, session::*, taskmgr::*, tools::*, topbar::*};

pub(crate) fn out(prog: &str, args: &[&str]) -> Result<String, String> {
    let owned;
    let mut cmd = Command::new(prog);
    if prog == "reg" {
        owned = i18n::reg::user_args(args);
        cmd.args(&owned);
    } else {
        owned = Vec::new();
        cmd.args(args);
    }
    let o = cmd.creation_flags(0x0800_0000).output().map_err(|e| format!("{prog}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        let joined = if owned.is_empty() { args.join(" ") } else { owned.join(" ") };
        Err(format!("{prog} {joined}: {}{}", String::from_utf8_lossy(&o.stdout).trim(), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

pub(crate) fn run(prog: &str, args: &[&str]) -> Result<(), String> {
    out(prog, args).map(|_| ())
}

pub(crate) static WARN: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(crate) fn soft(what: &str, r: Result<(), String>) {
    if let Err(e) = r {
        log(&format!("WARN {what}: {e}"));
        WARN.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{what}: {e}"));
    }
}

fn ensure_shared_log_dir() {
    let dir = winrust_common::log_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = Command::new("icacls.exe")
        .args([&dir.to_string_lossy().into_owned(), "/grant", "*S-1-5-32-545:(OI)(CI)M", "/Q"])
        .creation_flags(0x0800_0000)
        .output();
}

pub(crate) fn program_data() -> PathBuf {
    PathBuf::from(env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into()))
}

pub(crate) fn start_lnk(name: &str) -> PathBuf {
    program_data().join(format!(r"Microsoft\Windows\Start Menu\Programs\{name}.lnk"))
}

pub(crate) fn shortcut(lnk: &Path, target: &Path, args: &str) -> Result<(), String> {
    run("powershell", &["-NoProfile", "-Command", &format!("$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.Arguments='{args}'; $s.Save()", lnk.display(), target.display())])
}

pub(crate) fn log(s: &str) {
    winrust_common::log("installer", s);
}

fn retry_io(dst: &Path, mut f: impl FnMut() -> std::io::Result<()>) -> Result<(), String> {
    let mut r = f();
    for _ in 0..30 {
        if !matches!(&r, Err(e) if e.raw_os_error() == Some(32) || e.raw_os_error() == Some(5)) {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(200));
        r = f();
    }
    r.map_err(|e| format!("{}: {e}", dst.display()))
}

pub(crate) fn put(src: &Path, dst: &Path) -> Result<(), String> {
    retry_io(dst, || fs::copy(src, dst).map(|_| ()))
}

pub(crate) fn write_payload(name: &str, dst: &Path) -> Result<(), String> {
    if let Some(dir) = dst.parent() {
        if fs::create_dir_all(dir).is_err() {
            grant_admin_full(dir);
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    let disk_src = disk_payload().map(|p| p.join(name)).filter(|p| p.is_file());
    let bytes = PAYLOAD.iter().find(|(m, _)| *m == name).ok_or_else(|| format!("{name} is missing from the build"))?.1;
    let write_once = || -> Result<(), String> {
        if let Some(ref src) = disk_src {
            log(&format!("{name}: from {}", src.display()));
            put(src, dst)
        } else {
            retry_io(dst, || fs::write(dst, bytes))
        }
    };
    if write_once().is_ok() {
        return Ok(());
    }
    clear_readonly(dst);
    grant_admin_full(dst);
    if let Some(dir) = dst.parent() {
        grant_admin_full(dir);
    }
    let _ = unlock_file_holders(dst);
    if write_once().is_ok() {
        return Ok(());
    }
    let _ = move_aside_for_reboot(dst);
    write_once()
}

pub(crate) fn kill_procs(names: &[&str]) {
    let alive = winrust_common::proc::kill_and_wait(names, 4000);
    if !alive.is_empty() {
        soft("kill_procs", Err(format!("still running: {}", alive.join(", "))));
    }
}

pub(crate) fn stop_and_delete_task(name: &str) {
    let _ = run("schtasks", &["/end", "/tn", name]);
    let _ = run("schtasks", &["/delete", "/tn", name, "/f"]);
}

pub(crate) fn stop_and_delete_rs_search_tasks() {
    stop_and_delete_task("rs-search");
    if let Ok(csv) = out("schtasks", &["/query", "/fo", "csv", "/nh"]) {
        for line in csv.lines() {
            let Some(name) = line.trim().strip_prefix('"').and_then(|s| s.split('"').next()) else { continue };
            let trimmed = name.trim_start_matches('\\');
            if trimmed.eq_ignore_ascii_case("rs-search") || trimmed.to_ascii_lowercase().starts_with("rs-search-") {
                stop_and_delete_task(trimmed);
            }
        }
    }
}

fn set_matching_tasks(needles: &[&str], enable: bool) {
    if let Ok(csv) = out("schtasks", &["/query", "/fo", "csv", "/nh"]) {
        for line in csv.lines() {
            let Some(name) = line.trim().strip_prefix('"').and_then(|s| s.split('"').next()) else { continue };
            let low = name.to_ascii_lowercase();
            if needles.iter().any(|n| low.contains(n)) {
                if enable {
                    let _ = run("schtasks", &["/change", "/tn", name, "/enable"]);
                } else {
                    let _ = run("schtasks", &["/end", "/tn", name]);
                    let _ = run("schtasks", &["/change", "/tn", name, "/disable"]);
                }
            }
        }
    }
}

pub(crate) fn override_windhawk() {
    let has_svc = i18n::reg::exists(r"HKLM\SYSTEM\CurrentControlSet\Services\Windhawk", None);
    let has_reg = i18n::reg::exists(r"HKLM\SOFTWARE\Windhawk", None);
    let has_proc = !winrust_common::proc::pids("windhawk.exe").is_empty()
        || !winrust_common::proc::pids("windhawk-tray.exe").is_empty();
    if !has_svc && !has_reg && !has_proc {
        return;
    }
    unsafe {
        let ev = windows_sys::Win32::System::Threading::OpenEventW(
            0x0002,
            0,
            wide(r"Global\WindhawkServiceEmergencyStopEvent").as_ptr(),
        );
        if !ev.is_null() {
            windows_sys::Win32::System::Threading::SetEvent(ev);
            windows_sys::Win32::Foundation::CloseHandle(ev);
        }
    }
    if has_svc {
        let _ = run("sc.exe", &["stop", "Windhawk"]);
        let _ = run("sc.exe", &["config", "Windhawk", "start=", "disabled"]);
    }
    kill_procs(&["windhawk.exe", "windhawk-tray.exe"]);
    set_matching_tasks(&["windhawk"], false);
    let _ = run("reg", &["delete", RUN_KEY, "/v", "Windhawk", "/f"]);
    let _ = run("reg", &["delete", r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run", "/v", "Windhawk", "/f"]);
    for mods_key in [
        r"HKLM\SOFTWARE\Windhawk\Engine\Mods",
        r"HKLM\SOFTWARE\Windhawk\Engine\ModsWritable",
        r"HKCU\SOFTWARE\Windhawk\Engine\Mods",
    ] {
        if let Ok(list) = out("reg", &["query", mods_key]) {
            for line in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let low = line.to_ascii_lowercase();
                if low.contains("taskbar") || low.contains("start-menu") || low.contains("explorer") {
                    let _ = run("reg", &["add", line, "/v", "Disabled", "/t", "REG_DWORD", "/d", "1", "/f"]);
                }
            }
        }
    }
    log("windhawk: stopped and disabled conflicting taskbar hooks");
}

const SAB_KEY: &str = r"HKCU\Software\StartIsBack";
const SAB_STATE: &str = r"HKCU\Software\WinRust\StartAllBack";

pub(crate) fn override_startallback() {
    let has_reg = i18n::reg::exists(SAB_KEY, None)
        || i18n::reg::exists(r"HKLM\SOFTWARE\StartIsBack", None);
    let has_proc = !winrust_common::proc::pids("StartAllBackCfg.exe").is_empty();
    if !has_reg && !has_proc {
        return;
    }
    if reg_dword(SAB_STATE, "DisabledByWinRust").is_none() && reg_dword(SAB_KEY, "Disabled") != Some(1) {
        let prev = reg_dword(SAB_KEY, "Disabled").map_or(u32::MAX, |v| v);
        let _ = run("reg", &["add", SAB_STATE, "/v", "DisabledByWinRust", "/t", "REG_DWORD", "/d", &prev.to_string(), "/f"]);
    }
    kill_procs(&["StartAllBackCfg.exe"]);
    let _ = run("reg", &["add", SAB_KEY, "/v", "Disabled", "/t", "REG_DWORD", "/d", "1", "/f"]);
    set_matching_tasks(&["startallback", "startisback"], false);
    log("startallback: disabled conflicting taskbar hooks");
}

pub(crate) fn restore_startallback() {
    let prev = match reg_dword(SAB_STATE, "DisabledByWinRust") {
        Some(v) => v,
        None if reg_dword(SAB_KEY, "Disabled") == Some(1) => u32::MAX,
        None => return,
    };
    if prev == u32::MAX {
        let _ = run("reg", &["delete", SAB_KEY, "/v", "Disabled", "/f"]);
    } else {
        let _ = run("reg", &["add", SAB_KEY, "/v", "Disabled", "/t", "REG_DWORD", "/d", &prev.to_string(), "/f"]);
    }
    let _ = run("reg", &["delete", SAB_STATE, "/f"]);
    set_matching_tasks(&["startallback", "startisback"], true);
    if !winrust_common::proc::shell_explorers().is_empty() {
        winrust_common::proc::kill_shell_explorers();
        let _ = winrust_common::proc::start_shell_explorer();
    }
    log("startallback: restored");
}

fn delete_on_reboot(path: &Path) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    with_wide(&path.as_os_str().to_string_lossy(), |w| unsafe { MoveFileExW(w, null(), 4) != 0 })
}

fn move_file_win32(src: &Path, dst: &Path) -> Result<(), u32> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        fn GetLastError() -> u32;
    }
    with_wide(&src.as_os_str().to_string_lossy(), |ws| {
        with_wide(&dst.as_os_str().to_string_lossy(), |wd| {
            if unsafe { MoveFileExW(ws, wd, 1 | 8) } != 0 {
                Ok(())
            } else {
                Err(unsafe { GetLastError() })
            }
        })
    })
}

fn clear_readonly(path: &Path) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileAttributesW(name: *const u16, attrs: u32) -> i32;
    }
    with_wide(&path.as_os_str().to_string_lossy(), |w| unsafe { SetFileAttributesW(w, 0x80) });
}

fn unlock_file_holders(path: &Path) -> Vec<String> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    #[derive(Clone, Copy)]
    #[repr(C)]
    struct RmUniqueProcess {
        pid: u32,
        start_time: [u32; 2],
    }
    #[derive(Clone, Copy)]
    #[repr(C)]
    struct RmProcessInfo {
        process: RmUniqueProcess,
        app_name: [u16; 256],
        service_short_name: [u16; 64],
        app_type: u32,
        app_status: u32,
        ts_session_id: u32,
        restartable: i32,
    }
    type StartFn = unsafe extern "system" fn(*mut u32, u32, *mut u16) -> u32;
    type RegFn = unsafe extern "system" fn(u32, u32, *const *const u16, u32, *const core::ffi::c_void, u32, *const *const u16) -> u32;
    type ListFn = unsafe extern "system" fn(u32, *mut u32, *mut u32, *mut RmProcessInfo, *mut u32) -> u32;
    type EndFn = unsafe extern "system" fn(u32) -> u32;

    let mut holders = Vec::new();
    unsafe {
        let lib = LoadLibraryW(wide("rstrtmgr.dll").as_ptr());
        if lib.is_null() {
            return holders;
        }
        let f_start = GetProcAddress(lib, b"RmStartSession\0".as_ptr());
        let f_reg = GetProcAddress(lib, b"RmRegisterResources\0".as_ptr());
        let f_list = GetProcAddress(lib, b"RmGetList\0".as_ptr());
        let f_end = GetProcAddress(lib, b"RmEndSession\0".as_ptr());
        if let (Some(fs_), Some(fr_), Some(fl_), Some(fe_)) = (f_start, f_reg, f_list, f_end) {
            let rm_start: StartFn = std::mem::transmute(fs_);
            let rm_reg: RegFn = std::mem::transmute(fr_);
            let rm_list: ListFn = std::mem::transmute(fl_);
            let rm_end: EndFn = std::mem::transmute(fe_);
            let mut session = 0u32;
            let mut key = [0u16; 64];
            if rm_start(&mut session, 0, key.as_mut_ptr()) == 0 {
                let wp: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
                let ptrs = [wp.as_ptr()];
                if rm_reg(session, 1, ptrs.as_ptr(), 0, null(), 0, null()) == 0 {
                    let (mut needed, mut count, mut reason) = (0u32, 0u32, 0u32);
                    let rc = rm_list(session, &mut needed, &mut count, null_mut(), &mut reason);
                    if rc == 234 && needed > 0 {
                        let mut infos: Vec<RmProcessInfo> = vec![zeroed(); needed as usize];
                        count = needed;
                        if rm_list(session, &mut needed, &mut count, infos.as_mut_ptr(), &mut reason) == 0 {
                            let me = std::process::id();
                            const KEEP: &[&str] = &[
                                "system", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe",
                                "services.exe", "lsass.exe", "svchost.exe", "msmpeng.exe", "explorer.exe",
                            ];
                            for info in infos.iter().take(count as usize) {
                                let pid = info.process.pid;
                                let exe = winrust_common::proc::exe_name(pid).unwrap_or_else(|| {
                                    let n = info.app_name.iter().position(|&c| c == 0).unwrap_or(info.app_name.len());
                                    String::from_utf16_lossy(&info.app_name[..n])
                                });
                                if !exe.is_empty() {
                                    holders.push(format!("{exe} (pid {pid})"));
                                }
                                if pid != me && !KEEP.iter().any(|k| exe.eq_ignore_ascii_case(k)) {
                                    winrust_common::proc::kill(pid);
                                }
                            }
                        }
                    }
                }
                rm_end(session);
            }
        }
        FreeLibrary(lib);
    }
    holders
}

fn grant_admin_full(path: &Path) {
    let p = path.to_string_lossy();
    let _ = run("takeown.exe", &["/F", &p, "/A"]);
    let _ = run("icacls.exe", &[&p, "/remove:d", "*S-1-5-32-544", "/Q"]);
    let _ = run("icacls.exe", &[&p, "/grant", "*S-1-5-32-544:F", "/Q"]);
}

fn move_aside_for_reboot(path: &Path) -> Result<(), String> {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Relaxed);
    let sys_temp = PathBuf::from(env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("Temp");
    let _ = fs::create_dir_all(&sys_temp);
    let staged = sys_temp.join(format!("winrust-del-{}-{n}.old", std::process::id()));
    let e1 = move_file_win32(path, &staged);
    if e1.is_ok() {
        let _ = delete_on_reboot(&staged);
        return Ok(());
    }
    let local_stale = path.with_extension(format!("old.{}.{n}", std::process::id()));
    let e2 = move_file_win32(path, &local_stale);
    if e2.is_ok() {
        let _ = delete_on_reboot(&local_stale);
        return Ok(());
    }
    let holders = unlock_file_holders(path);
    if !holders.is_empty() {
        thread::sleep(std::time::Duration::from_millis(200));
        if fs::remove_file(path).is_ok() {
            return Ok(());
        }
        if move_file_win32(path, &staged).is_ok() {
            let _ = delete_on_reboot(&staged);
            return Ok(());
        }
        if move_file_win32(path, &local_stale).is_ok() {
            let _ = delete_on_reboot(&local_stale);
            return Ok(());
        }
    }
    grant_admin_full(path);
    if let Some(parent) = path.parent() {
        grant_admin_full(parent);
    }
    clear_readonly(path);
    if fs::remove_file(path).is_ok() {
        return Ok(());
    }
    if move_file_win32(path, &staged).is_ok() {
        let _ = delete_on_reboot(&staged);
        return Ok(());
    }
    if move_file_win32(path, &local_stale).is_ok() {
        let _ = delete_on_reboot(&local_stale);
        return Ok(());
    }
    if delete_on_reboot(path) {
        return Ok(());
    }
    let code = e2.err().or(e1.err()).unwrap_or(0);
    let by = if holders.is_empty() { String::new() } else { format!(" [held by: {}]", holders.join(", ")) };
    Err(format!("{}: locked (os error {code}){by}", path.display()))
}

pub(crate) fn rm_file_checked(path: &Path) -> Result<bool, String> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(false);
    }
    clear_readonly(path);
    let mut unlocked = false;
    for attempt in 0..16 {
        match fs::remove_file(path) {
            Ok(()) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) if matches!(e.raw_os_error(), Some(5) | Some(32)) => {
                clear_readonly(path);
                if attempt == 2 && !unlocked {
                    unlocked = true;
                    let _ = unlock_file_holders(path);
                    if e.raw_os_error() == Some(5) {
                        grant_admin_full(path);
                    }
                }
                thread::sleep(std::time::Duration::from_millis(120));
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    move_aside_for_reboot(path)?;
    Ok(true)
}

pub(crate) fn rm_dir_checked(dir: &Path) -> Result<bool, String> {
    if fs::symlink_metadata(dir).is_err() {
        return Ok(false);
    }
    for _ in 0..5 {
        match fs::remove_dir_all(dir) {
            Ok(()) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) if matches!(e.raw_os_error(), Some(5) | Some(32) | Some(145)) => thread::sleep(std::time::Duration::from_millis(120)),
            Err(_) => break,
        }
    }
    let mut deferred = false;
    let mut errs = Vec::new();
    let rd = match fs::read_dir(dir) {
        Ok(rd) => Ok(rd),
        Err(_) => {
            grant_admin_full(dir);
            fs::read_dir(dir)
        }
    };
    match rd {
        Ok(rd) => {
            for e in rd.flatten() {
                let p = e.path();
                let r = if p.is_dir() { rm_dir_checked(&p) } else { rm_file_checked(&p) };
                match r {
                    Ok(d) => deferred |= d,
                    Err(err) => errs.push(err),
                }
            }
        }
        Err(e) => errs.push(format!("{}: {e}", dir.display())),
    }
    if !errs.is_empty() {
        return Err(errs.join("; "));
    }
    clear_readonly(dir);
    for _ in 0..10 {
        match fs::remove_dir(dir) {
            Ok(()) => return Ok(deferred),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(deferred),
            Err(e) if matches!(e.raw_os_error(), Some(5) | Some(32) | Some(145)) => thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => {
                if delete_on_reboot(dir) {
                    return Ok(true);
                }
                return Err(format!("{}: {e}", dir.display()));
            }
        }
    }
    if fs::symlink_metadata(dir).is_ok() {
        if delete_on_reboot(dir) {
            return Ok(true);
        }
        return Err(format!("{}: directory could not be removed", dir.display()));
    }
    Ok(deferred)
}

pub(crate) fn rm_file(path: &Path) {
    if let Err(e) = rm_file_checked(path) {
        soft("rm_file", Err(e));
    }
}

pub(crate) fn rm_dir(dir: &Path) {
    if let Err(e) = rm_dir_checked(dir) {
        soft("rm_dir", Err(e));
    }
}

pub(crate) fn restore_windows_taskbar() {
    use windows_sys::Win32::UI::Shell::{ABM_SETSTATE, ABS_ALWAYSONTOP, APPBARDATA, SHAppBarMessage};
    let saved_state: usize = appdata()
        .and_then(|ad| fs::read_to_string(ad.join("topbar-taskbar-state.txt")).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(ABS_ALWAYSONTOP as usize);
    unsafe {
        let tray = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        let sec = FindWindowW(wide("Shell_SecondaryTrayWnd").as_ptr(), null());
        if !tray.is_null() {
            let mut ab: APPBARDATA = zeroed();
            ab.cbSize = size_of::<APPBARDATA>() as u32;
            ab.hWnd = tray;
            ab.lParam = saved_state as isize;
            SHAppBarMessage(ABM_SETSTATE, &mut ab);
            ShowWindow(tray, SW_SHOWNA);
        }
        if !sec.is_null() {
            ShowWindow(sec, SW_SHOWNA);
        }
        let mut full = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
        SystemParametersInfoW(SPI_SETWORKAREA, 0, &mut full as *mut _ as *mut _, 0);
        PostMessageW(HWND_BROADCAST, WM_SETTINGCHANGE, SPI_SETWORKAREA as usize, 0);
        let tb_msg = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        if tb_msg != 0 {
            SendNotifyMessageW(HWND_BROADCAST, tb_msg, 0, 0);
        }
    }
    if let Some(ad) = appdata() {
        let _ = fs::remove_file(ad.join("topbar-taskbar-state.txt"));
    }
}

pub(crate) fn quiesce_all_winrust() {
    stop_and_delete_rs_search_tasks();
    for task in ["WinRust-update", "rs-overlay", "topbar", "reshell", "rs-launcher", "rtaskmgr"] {
        stop_and_delete_task(task);
    }
    for val in ["rs-overlay", "topbar", "reshell", "rs-launcher", "WinRustNightLight"] {
        let _ = run("reg", &["delete", RUN_KEY, "/v", val, "/f"]);
    }
    kill_procs(&["rs-shell.exe", "reshell.exe"]);
    kill_procs(&[
        "topbar.exe",
        "rust-explorer.exe",
        "rs-launcher.exe",
        "rs.exe",
        "rtaskmgr-win32.exe",
        "rust-settings.exe",
        "photoview.exe",
        "debloat.exe",
        "purge.exe",
    ]);
}

pub(crate) fn verify_uninstalled(i: usize) -> Result<(), String> {
    let p = &PLUGINS[i];
    let mut still = Vec::new();
    if (p.installed)() {
        still.push(format!("{} still reports installed", p.name));
    }
    match p.name {
        "rust-search" => {
            if prog_dir().join("rs.exe").exists() {
                still.push(format!("{} still exists", prog_dir().join("rs.exe").display()));
            }
            if prog_dir().join("rs-launcher.exe").exists() {
                still.push(format!("{} still exists", prog_dir().join("rs-launcher.exe").display()));
            }
            if reg_str(SH_KEY, "Debugger").is_some_and(|v| {
                let l = v.to_ascii_lowercase();
                l.contains("cmd.exe") || l.contains("rs-launcher.exe")
            }) {
                still.push("SearchHost.exe IFEO Debugger still set".into());
            }
        }
        "rtaskmgr-win32" => {
            if tm_installed() {
                still.push("taskmgr.exe IFEO Debugger still points to rtaskmgr".into());
            }
            if tm_dir().join("rtaskmgr-win32.exe").exists() {
                still.push(format!("{} still exists", tm_dir().join("rtaskmgr-win32.exe").display()));
            }
        }
        "topbar" => {
            if bar_dir().join("topbar.exe").exists() {
                still.push(format!("{} still exists", bar_dir().join("topbar.exe").display()));
            }
            if bar_dir().join("reshell.exe").exists() {
                still.push(format!("{} still exists", bar_dir().join("reshell.exe").display()));
            }
        }
        "rust-explorer" => {
            if rx_dir().join("rust-explorer.exe").exists() {
                still.push(format!("{} still exists", rx_dir().join("rust-explorer.exe").display()));
            }
            if rx_dir().join("rs-shell.exe").exists() {
                still.push(format!("{} still exists", rx_dir().join("rs-shell.exe").display()));
            }
            if reg_dword(RX_SWITCH, "ReplaceExplorer") == Some(1) {
                still.push("HKCU\\Software\\rs-shell\\ReplaceExplorer is still 1".into());
            }
        }
        "photoview" => {
            if pv_dir().join("photoview.exe").exists() {
                still.push(format!("{} still exists", pv_dir().join("photoview.exe").display()));
            }
        }
        "rust-settings" => {
            if st_dir().join("rust-settings.exe").exists() {
                still.push(format!("{} still exists", st_dir().join("rust-settings.exe").display()));
            }
        }
        "debloat" => {
            if db_dir().join("debloat.exe").exists() {
                still.push(format!("{} still exists", db_dir().join("debloat.exe").display()));
            }
        }
        "purge" => {
            if pu_dir().join("purge.exe").exists() {
                still.push(format!("{} still exists", pu_dir().join("purge.exe").display()));
            }
        }
        _ => {}
    }
    if still.is_empty() {
        Ok(())
    } else {
        Err(still.join("; "))
    }
}

fn reg_key(key: &str) -> std::borrow::Cow<'_, str> {
    match env::var(i18n::reg::SID_ENV).ok().filter(|s| i18n::reg::valid_sid(s)) {
        Some(sid) => std::borrow::Cow::Owned(i18n::reg::for_user(key, &sid)),
        None => std::borrow::Cow::Borrowed(key),
    }
}

pub(crate) fn reg_dword(key: &str, name: &str) -> Option<u32> {
    i18n::reg::get_dword(&reg_key(key), name)
}

pub(crate) fn reg_str(key: &str, name: &str) -> Option<String> {
    i18n::reg::get_str(&reg_key(key), name)
}

include!(concat!(env!("OUT_DIR"), "/payload.rs"));

pub(crate) fn disk_payload() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let d = exe.parent()?;
    [Some(d.join("payload")), d.ends_with(r"target\release").then(|| d.join(r"..\..\payload"))].into_iter().flatten().find(|p| p.is_dir())
}

pub(crate) fn with_payload<T>(need: &[&str], f: impl FnOnce(&Path) -> Result<T, String>) -> Result<T, String> {
    let d = program_files().join(format!("plugin-launcher-tmp-{}", std::process::id()));
    fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    let disk = disk_payload();
    let unpack = |n: &&str| -> Result<(), String> {
        let bytes = PAYLOAD.iter().find(|(m, _)| m == n).ok_or_else(|| format!("{n} is missing from the build"))?.1;
        match disk.as_ref().map(|p| p.join(n)).filter(|p| p.is_file()) {
            Some(src) => {
                log(&format!("{n}: from {}", src.display()));
                fs::copy(src, d.join(n)).map(|_| ())
            }
            None => fs::write(d.join(n), bytes),
        }
        .map_err(|e| format!("unpacking {n}: {e}"))
    };
    let r = need.iter().try_for_each(unpack).and_then(|_| f(&d));
    rm_dir(&d);
    r
}

pub(crate) fn appdata() -> Option<PathBuf> {
    env::var_os("APPDATA").map(PathBuf::from)
}

pub(crate) fn local_appdata() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

pub(crate) fn program_files() -> PathBuf {
    PathBuf::from(env::var("ProgramFiles").unwrap_or(r"C:\Program Files".into()))
}

fn require_elevated() -> Result<(), String> {
    if winrust_common::proc::elevated() {
        Ok(())
    } else {
        Err("Administrator rights (High integrity) required. Right-click WinRust.exe -> Run as administrator.".into())
    }
}

fn warnings_note(w: &[String]) -> String {
    if w.is_empty() {
        String::new()
    } else {
        format!(" Warnings: {} (see %ProgramData%\\WinRust\\winrust.log).", w.len())
    }
}

pub(crate) fn install_plugin(i: usize) -> Result<String, String> {
    require_elevated()?;
    ensure_shared_log_dir();
    let p = &PLUGINS[i];
    let (inst, uninst) = p.actions.ok_or("installation is not available")?;
    init().inspect_err(|e| log(&format!("{}: ERROR {e}", p.name)))?;
    let was = (p.installed)();
    WARN.lock().unwrap_or_else(|e| e.into_inner()).clear();
    if let Err(e) = inst() {
        log(&format!("{}: ERROR {e}", p.name));
        if was {
            return Err(e);
        }
        let rollback = match uninst() {
            Ok(()) => "automatic rollback completed".to_string(),
            Err(u) => format!("automatic rollback failed: {u}"),
        };
        log(&format!("{}: {rollback}", p.name));
        return Err(format!("{e} ({rollback})"));
    }
    let w = WARN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    log(&format!("{}: {} {}", p.name, if was { "updated" } else { "installed" }, w.join(" | ")));
    let note = warnings_note(&w);
    Ok(format!("{} {}.{note}", p.name, if was { "updated" } else { "installed" }))
}

pub(crate) fn uninstall_plugin(i: usize) -> Result<String, String> {
    require_elevated()?;
    let p = &PLUGINS[i];
    let (_, uninst) = p.actions.ok_or("uninstall is not available")?;
    WARN.lock().unwrap_or_else(|e| e.into_inner()).clear();
    init_soft();
    let res = uninst().and_then(|()| verify_uninstalled(i));
    if let Err(e) = res {
        log(&format!("{}: uninstall ERROR {e}", p.name));
        return Err(e);
    }
    let w = WARN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let any_left = PLUGINS.iter().any(|pl| (pl.installed)());
    if any_left || !w.is_empty() {
        log(&format!("{}: removed {}", p.name, w.join(" | ")));
    }
    let note = warnings_note(&w);
    Ok(format!("{} removed.{note}", p.name))
}

pub(crate) fn rollback_core(hwnd: Option<isize>) -> Result<String, String> {
    require_elevated()?;
    WARN.lock().unwrap_or_else(|e| e.into_inner()).clear();
    init_soft();
    quiesce_all_winrust();

    let targets: Vec<usize> = (0..PLUGINS.len()).rev().filter(|&i| (PLUGINS[i].installed)()).collect();
    if let Some(h) = hwnd {
        *STEPS.lock().unwrap_or_else(|e| e.into_inner()) = targets.iter().map(|&i| (i, false)).collect();
        unsafe { InvalidateRect(h as HWND, null(), 0) };
    }

    let mut errs: Vec<String> = Vec::new();
    let mut warned = false;
    for i in targets {
        if let Some(h) = hwnd {
            JOB.store(i, Relaxed);
            unsafe { InvalidateRect(h as HWND, null(), 0) };
        }
        let r = uninstall_plugin(i);
        match r {
            Ok(_) => {
                warned |= !WARN.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
                if hwnd.is_some() {
                    RESULTS.lock().unwrap_or_else(|e| e.into_inner()).push((i, false, None));
                }
            }
            Err(e) => {
                errs.push(format!("{}: {e}", PLUGINS[i].name));
                if hwnd.is_some() {
                    RESULTS.lock().unwrap_or_else(|e| e.into_inner()).push((i, false, Some(e)));
                }
            }
        }
        refresh();
    }
    if hwnd.is_some() {
        STEPS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    if winrust_common::proc::shell_explorers().is_empty() {
        if !winrust_common::launch::program(r"C:\Windows\explorer.exe") {
            let _ = winrust_common::proc::start_shell_explorer();
        }
    }
    restore_windows_taskbar();
    restore_startallback();

    let remaining: Vec<&str> = PLUGINS.iter().filter(|p| (p.installed)()).map(|p| p.name).collect();
    if !remaining.is_empty() {
        errs.push(format!("still installed: {}", remaining.join(", ")));
    }

    let warnings = WARN.lock().unwrap_or_else(|e| e.into_inner()).join(" | ");
    let note = if warnings.is_empty() && !warned { "" } else {
        if !warnings.is_empty() {
            log(&format!("rollback: {warnings}"));
        }
        " See warnings in %ProgramData%\\WinRust\\winrust.log."
    };
    if errs.is_empty() {
        Ok(format!("All plugins removed.{note}"))
    } else {
        Err(format!("{}{note}", errs.join("; ")))
    }
}

pub(crate) fn rollback() -> Result<String, String> {
    rollback_core(None)
}
