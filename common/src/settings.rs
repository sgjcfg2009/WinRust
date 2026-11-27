use crate::proc;
use std::{os::windows::process::CommandExt, process::{Command, Stdio}, thread::sleep, time::Duration};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::{
        Input::KeyboardAndMouse::keybd_event,
        WindowsAndMessaging::{EnumWindows, FindWindowExW, GetShellWindow, GetWindow, GetWindowThreadProcessId, IsHungAppWindow, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow},
    },
};

const SETTINGS: &str = "SystemSettings.exe";
const ENROLL: &str = "CredentialEnrollmentManager.exe";

fn pid_of(h: HWND) -> u32 {
    let mut p = 0;
    unsafe { GetWindowThreadProcessId(h, &mut p) };
    p
}

fn shell_pid() -> Option<u32> {
    let h = unsafe { GetShellWindow() };
    (!h.is_null()).then(|| pid_of(h))
}

fn explorer_shell() -> bool {
    shell_pid().is_some_and(|p| proc::exe_name(p).is_some_and(|n| n.eq_ignore_ascii_case("explorer.exe")))
}

fn explorer_tray() -> bool {
    let Some(sp) = shell_pid() else { return false };
    crate::with_wide("Shell_TrayWnd", |cls| {
        let mut t = std::ptr::null_mut();
        loop {
            t = unsafe { FindWindowExW(std::ptr::null_mut(), t, cls, std::ptr::null()) };
            if t.is_null() {
                return false;
            }
            if pid_of(t) == sp {
                return true;
            }
        }
    })
}

fn find_core_window(pid: u32, mut f: impl FnMut(HWND) -> bool) -> bool {
    crate::with_wide("Windows.UI.Core.CoreWindow", |cls| {
        let mut c = std::ptr::null_mut();
        loop {
            c = unsafe { FindWindowExW(std::ptr::null_mut(), c, cls, std::ptr::null()) };
            if c.is_null() {
                return false;
            }
            if pid_of(c) == pid && f(c) {
                return true;
            }
        }
    })
}

fn raise() {
    let Some(sp) = proc::pids(SETTINGS).first().copied() else { return };
    find_core_window(sp, |c| unsafe {
        let mut o = GetWindow(c, 4);
        if o.is_null() {
            o = c;
        }
        keybd_event(0x12, 0, 0, 0);
        keybd_event(0x12, 0, 2, 0);
        ShowWindow(o, 9);
        SetForegroundWindow(o);
        true
    });
}

unsafe extern "system" fn hung_cb(h: HWND, l: LPARAM) -> i32 {
    let d = unsafe { &mut *(l as *mut (u32, bool)) };
    if pid_of(h) == d.0 && unsafe { IsWindowVisible(h) != 0 && IsHungAppWindow(h) != 0 } {
        d.1 = true;
    }
    1
}

fn hung(pid: u32) -> bool {
    let mut d = (pid, false);
    unsafe { EnumWindows(Some(hung_cb), &mut d as *mut _ as LPARAM) };
    d.1
}

fn seen(h: HWND) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    let mut cloaked = 0u32;
    unsafe { IsWindowVisible(h) != 0 && (IsIconic(h) != 0 || DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as _, 4) != 0 || cloaked == 0) }
}

fn showing(pid: u32) -> bool {
    find_core_window(pid, |c| {
        let o = unsafe { GetWindow(c, 4) };
        seen(c) || (!o.is_null() && seen(o))
    })
}

fn stale(pid: u32) -> bool {
    hung(pid) || !showing(pid)
}

fn any_hung() -> bool {
    proc::pids(SETTINGS).into_iter().any(hung)
}

fn kill_stale() {
    proc::pids(SETTINGS).into_iter().filter(|&p| stale(p)).for_each(proc::kill);
}

fn kill_hung() {
    proc::pids(SETTINGS).into_iter().filter(|&p| hung(p)).for_each(proc::kill);
}

fn wait(n: u32, mut done: impl FnMut() -> bool) {
    for _ in 0..n {
        if done() {
            return;
        }
        sleep(Duration::from_millis(100));
    }
}

fn mark_autostart_done() {
    use windows_sys::Win32::System::{Registry::*, RemoteDesktop::ProcessIdToSessionId};
    let mut sid = 0u32;
    unsafe { ProcessIdToSessionId(std::process::id(), &mut sid) };
    for name in ["RunStuffHasBeenRun", "StartupHasBeenRun"] {
        let key = format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{sid}\{name}");
        crate::with_wide(&key, |kw| {
            let mut h: HKEY = std::ptr::null_mut();
            if unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, kw, 0, std::ptr::null(), REG_OPTION_VOLATILE, KEY_WRITE, std::ptr::null(), &mut h, std::ptr::null_mut()) } == 0 {
                unsafe { RegCloseKey(h) };
            }
        });
    }
}

fn has_desktop(p: u32) -> bool {
    proc::cmdline(p).is_some_and(|c| c.contains("--desktop"))
}

const LENT: &str = "Local\\WinRust-ExplorerLent";

pub fn explorer_lent() -> bool {
    use windows_sys::Win32::{Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError}, System::Threading::OpenMutexW};
    let h = crate::with_wide(LENT, |w| unsafe { OpenMutexW(0x0010_0000, 0, w) });
    if h.is_null() {
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    unsafe { CloseHandle(h) };
    true
}

fn run(uri: &str) {
    kill_stale();
    let mut started = false;
    let mut lent = std::ptr::null_mut();
    if !explorer_shell() {
        lent = crate::with_wide(LENT, |w| unsafe { windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, w) });
        proc::pids("rust-explorer.exe").into_iter().filter(|&p| has_desktop(p)).for_each(proc::kill);
        proc::pids("ApplicationFrameHost.exe").into_iter().for_each(proc::kill);
        mark_autostart_done();
        let _ = proc::start_shell_explorer();
        wait(200, explorer_shell);
        wait(150, explorer_tray);
        sleep(Duration::from_secs(1));
        started = true;
    }
    for _ in 0..2 {
        let _ = Command::new(r"C:\Windows\explorer.exe").arg(uri).status();
        wait(80, || !proc::pids(SETTINGS).is_empty());
        sleep(Duration::from_secs(1));
        raise();
        sleep(Duration::from_secs(3));
        raise();
        if !any_hung() {
            break;
        }
        kill_hung();
    }
    if started {
        let (mut bad, mut shown, mut waited) = (0, false, 0);
        loop {
            let enrolling = !proc::pids(ENROLL).is_empty();
            if !enrolling && (bad >= 5 || proc::pids(SETTINGS).is_empty() || (!shown && waited >= 30)) {
                break;
            }
            sleep(Duration::from_secs(2));
            waited += 1;
            let on_screen = proc::pids(SETTINGS).into_iter().any(showing);
            shown |= on_screen;
            bad = if !enrolling && (any_hung() || (shown && !on_screen)) { bad + 1 } else { 0 };
        }
        kill_stale();
        proc::kill_shell_explorers();
        let rx = std::path::PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default()).join(r"rust-explorer\rust-explorer.exe");
        if rx.is_file() && !proc::pids("rust-explorer.exe").into_iter().any(has_desktop) {
            let _ = Command::new(rx).arg("--desktop").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        }
    }
    if !lent.is_null() {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(lent) };
    }
}

fn own_page(uri: &str) -> Option<&'static str> {
    let trimmed = uri.trim();
    if trimmed.len() > 64 || !trimmed.is_ascii() {
        return None;
    }
    let mut buf = [0u8; 64];
    for (dst, &src) in buf.iter_mut().zip(trimmed.as_bytes()) {
        *dst = src.to_ascii_lowercase();
    }
    let lower = std::str::from_utf8(&buf[..trimmed.len()]).ok()?;
    match lower {
        "ms-settings:" | "ms-settings:display" | "ms-settings:nightlight" => Some("display"),
        "ms-settings:sound" => Some("sound"),
        "ms-settings:personalization" | "ms-settings:colors" | "ms-settings:personalization-colors" | "ms-settings:themes" => Some("personalization"),
        "ms-settings:powersleep" | "ms-settings:storagesense" | "ms-settings:batterysaver" => Some("power"),
        "ms-settings:network" | "ms-settings:network-status" | "ms-settings:network-wifi" | "ms-settings:network-wifisettings" | "ms-settings:network-vpn" | "ms-settings:network-proxy" => Some("network"),
        "ms-settings:bluetooth" | "ms-settings:network-airplanemode" => Some("bluetooth"),
        "ms-settings:appsfeatures" => Some("apps"),
        "ms-settings:windowsupdate" => Some("update"),
        "ms-settings:about" | "ms-settings:dateandtime" | "ms-settings:time" => Some("about"),
        _ => None,
    }
}

fn own_exe() -> Option<std::path::PathBuf> {
    let exe = std::path::PathBuf::from(std::env::var_os("ProgramFiles")?).join("rust-settings").join("rust-settings.exe");
    exe.is_file().then_some(exe)
}

pub fn open(uri: &str) {
    if let (Some(page), Some(exe)) = (own_page(uri), own_exe()) {
        if Command::new(exe).arg(page).stdin(Stdio::null()).creation_flags(0x0800_0000).spawn().is_ok() {
            return;
        }
    }
    let uri = uri.to_string();
    std::thread::spawn(move || run(&uri));
}

#[cfg(test)]
mod tests {
    use super::own_page;

    #[test]
    fn lent_marker_lives_as_long_as_its_handle() {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::CreateMutexW};
        let h = crate::with_wide(super::LENT, |w| unsafe { CreateMutexW(std::ptr::null(), 0, w) });
        assert!(!h.is_null());
        assert!(super::explorer_lent());
        unsafe { CloseHandle(h) };
        assert!(!super::explorer_lent(), "gone with the last handle (no Settings page is open during the test)");
    }

    #[test]
    fn only_exact_addresses_go_to_rust_settings() {
        assert_eq!(own_page("ms-settings:"), Some("display"));
        assert_eq!(own_page("MS-Settings:Sound"), Some("sound"));
        assert_eq!(own_page("ms-settings:themes"), Some("personalization"));
        assert_eq!(own_page("ms-settings:powersleep"), Some("power"));
        assert_eq!(own_page("ms-settings:network-status"), Some("network"));
        assert_eq!(own_page("ms-settings:personalization-background"), None);
        assert_eq!(own_page("ms-settings:network-wifi"), Some("network"));
        assert_eq!(own_page("ms-settings:bluetooth"), Some("bluetooth"));
        assert_eq!(own_page("ms-settings:windowsupdate"), Some("update"));
        assert_eq!(own_page("ms-settings:easeofaccess"), None);
        assert_eq!(own_page("ms-settings:sound; calc"), None);
    }
}
