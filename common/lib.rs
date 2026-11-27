use std::path::PathBuf;

pub mod desktop;
pub mod fg;
pub mod journal;
pub mod launch;
pub mod mem;
pub mod proc;
pub mod reg;
pub mod search;
pub mod settings;
pub mod style;
pub mod ui;

pub fn utf8_console() {
    use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, SetConsoleOutputCP, STD_OUTPUT_HANDLE};
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0u32;
        if !h.is_null() && h != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE && GetConsoleMode(h, &mut mode) != 0 {
            SetConsoleOutputCP(65001);
        }
    }
}

static LOG_FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static LOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static LOG_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

pub fn log_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("WinRust")
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub fn with_wide<T>(s: &str, f: impl FnOnce(*const u16) -> T) -> T {
    let mut stack = [0u16; 256];
    let mut len = 0usize;
    for u in s.encode_utf16() {
        if len + 1 >= stack.len() {
            let v: Vec<u16> = s.encode_utf16().chain([0]).collect();
            return f(v.as_ptr());
        }
        stack[len] = u;
        len += 1;
    }
    stack[len] = 0;
    f(stack.as_ptr())
}

pub fn log(component: &str, msg: &str) {
    use std::io::Write;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(lp: *mut [u16; 8]);
    }
    let mut st = [0u16; 8];
    unsafe { GetLocalTime(&mut st) };
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = LOG_FILE.get_or_init(|| {
        let dir = log_dir();
        if std::fs::create_dir_all(&dir).is_ok() {
            let primary = dir.join("winrust.log");
            if std::fs::OpenOptions::new().create(true).append(true).open(&primary).is_ok() {
                return primary;
            }
        }
        let fb = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("WinRust");
        let _ = std::fs::create_dir_all(&fb);
        fb.join("winrust.log")
    });
    let mut bytes = LOG_BYTES.load(std::sync::atomic::Ordering::Relaxed);
    if bytes == u64::MAX {
        bytes = std::fs::metadata(p).map_or(0, |m| m.len());
    }
    if bytes > 2 * 1024 * 1024 {
        let _ = std::fs::rename(p, p.with_extension("log.old"));
        bytes = 0;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let line = format!("[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] [{component}] {msg}\n", st[0], st[1], st[3], st[4], st[5], st[6]);
        if f.write_all(line.as_bytes()).is_ok() {
            bytes = bytes.saturating_add(line.len() as u64);
        }
    }
    LOG_BYTES.store(bytes, std::sync::atomic::Ordering::Relaxed);
}

pub fn install_panic_hook(component: &'static str) {
    std::panic::set_hook(Box::new(move |info| {
        log(component, &format!("PANIC: {info}\n{}", std::backtrace::Backtrace::force_capture()));
    }));
}

pub fn reveal(path: &str) {
    use std::{os::windows::process::CommandExt, process::Command};
    let rx = std::env::var_os("ProgramFiles").map(|pf| PathBuf::from(pf).join(r"rust-explorer\rust-explorer.exe")).filter(|p| p.is_file());
    let _ = match rx {
        Some(rx) => Command::new(rx).raw_arg(format!("/select,\"{path}\"")).spawn(),
        None => Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn(),
    };
}

pub fn signal_shell_ready() {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::{OpenEventW, SetEvent}};
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    for name in ["Local\\ShellDesktopSwitchEvent", "ShellDesktopSwitchEvent"] {
        let h = with_wide(name, |w| unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, w) });
        if !h.is_null() {
            unsafe {
                SetEvent(h);
                CloseHandle(h);
            }
            return;
        }
    }
}

pub fn wait_shell_ready() {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            StationsAndDesktops::{CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME},
            Threading::{OpenEventW, WaitForSingleObject},
        },
        UI::WindowsAndMessaging::FindWindowW,
    };
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const DESKTOP_READOBJECTS: u32 = 0x0001;

    for name in ["Local\\ShellDesktopSwitchEvent", "ShellDesktopSwitchEvent"] {
        let ev = with_wide(name, |w| unsafe { OpenEventW(SYNCHRONIZE, 0, w) });
        if !ev.is_null() {
            unsafe {
                WaitForSingleObject(ev, 8000);
                CloseHandle(ev);
            }
            return;
        }
    }

    let on_default_desktop = || unsafe {
        let h = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
        if h.is_null() {
            return false;
        }
        let mut buf = [0u16; 64];
        let mut needed = 0u32;
        let ok = GetUserObjectInformationW(h as _, UOI_NAME, buf.as_mut_ptr().cast(), (buf.len() * 2) as u32, &mut needed) != 0;
        CloseDesktop(h);
        if !ok {
            return false;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        n == 7 && buf[..n].iter().zip(b"Default").all(|(&u, &c)| u <= 0x7F && (u as u8).eq_ignore_ascii_case(&c))
    };

    let t0 = Instant::now();
    while !on_default_desktop() && t0.elapsed() < Duration::from_secs(6) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let custom_shell = reg::exists(r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon", Some("Shell"));
    if !custom_shell {
        const PROGMAN: [u16; 8] = [b'P' as u16, b'r' as u16, b'o' as u16, b'g' as u16, b'm' as u16, b'a' as u16, b'n' as u16, 0];
        const PM_TITLE: [u16; 16] = [
            b'P' as u16, b'r' as u16, b'o' as u16, b'g' as u16, b'r' as u16, b'a' as u16, b'm' as u16, b' ' as u16,
            b'M' as u16, b'a' as u16, b'n' as u16, b'a' as u16, b'g' as u16, b'e' as u16, b'r' as u16, 0,
        ];
        const TRAY_CLS: [u16; 14] = [
            b'S' as u16, b'h' as u16, b'e' as u16, b'l' as u16, b'l' as u16, b'_' as u16,
            b'T' as u16, b'r' as u16, b'a' as u16, b'y' as u16, b'W' as u16, b'n' as u16, b'd' as u16, 0,
        ];
        let t1 = Instant::now();
        while unsafe { FindWindowW(TRAY_CLS.as_ptr(), std::ptr::null()).is_null() && FindWindowW(PROGMAN.as_ptr(), PM_TITLE.as_ptr()).is_null() }
            && t1.elapsed() < Duration::from_secs(4)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
