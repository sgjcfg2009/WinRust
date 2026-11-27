use crate::sys::{self, wide};
use std::{
    path::Path,
    ptr::{null, null_mut},
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::{Duration, SystemTime},
};
use windows_sys::Win32::{
    Foundation::*,
    NetworkManagement::Rras::*,
    Networking::WinInet::{INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW},
    System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
};

fn utf16(s: &[u16]) -> String {
    String::from_utf16_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())])
}

pub const INET: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

pub fn proxy_text() -> String {
    sys::string_long(HKEY_CURRENT_USER, INET, "ProxyServer").filter(|s| !s.is_empty()).unwrap_or_else(|| "not set".into())
}

pub fn pac_text() -> String {
    sys::string_long(HKEY_CURRENT_USER, INET, "AutoConfigURL").filter(|s| !s.is_empty()).unwrap_or_else(|| "not set".into())
}

pub fn valid_proxy(s: &str) -> bool {
    s.len() <= 200 && !s.chars().any(|c| c.is_control() || c == ' ' || c == '"' || c == '<' || c == '>')
}

pub fn valid_pac(s: &str) -> bool {
    s.is_empty() || (s.len() <= 400 && ["http://", "https://", "file://"].iter().any(|p| s.to_ascii_lowercase().starts_with(p)) && !s.chars().any(|c| c.is_control() || c == ' '))
}

pub fn set_proxy(pac: bool, v: &str) -> bool {
    let ok = sys::set_hkcu_string(INET, if pac { "AutoConfigURL" } else { "ProxyServer" }, v);
    if ok && !pac && !v.is_empty() {
        sys::set_hkcu_dword(INET, "ProxyEnable", 1);
    }
    proxy_changed();
    ok
}

pub fn proxy_changed() {
    unsafe {
        InternetSetOptionW(null(), INTERNET_OPTION_SETTINGS_CHANGED, null(), 0);
        InternetSetOptionW(null(), INTERNET_OPTION_REFRESH, null(), 0);
    }
}

#[derive(Clone, Debug)]
pub struct Vpn {
    pub name: String,
    pub pbk: Option<String>,
    pub on: bool,
}

fn user_pbk() -> Option<String> {
    let p = std::path::PathBuf::from(std::env::var_os("APPDATA")?).join(r"Microsoft\Network\Connections\Pbk\rasphone.pbk");
    p.is_file().then(|| p.to_string_lossy().into_owned())
}

fn entries(pbk: Option<&str>) -> Vec<String> {
    let w = pbk.map(wide);
    let (mut cb, mut n) = (size_of::<RASENTRYNAMEW>() as u32, 0u32);
    let mut v: Vec<RASENTRYNAMEW> = vec![unsafe { std::mem::zeroed() }; 1];
    v[0].dwSize = cb;
    let ptr = |w: &Option<Vec<u16>>| w.as_ref().map_or(null(), |w| w.as_ptr());
    let mut rc = unsafe { RasEnumEntriesW(null(), ptr(&w), v.as_mut_ptr(), &mut cb, &mut n) };
    if rc == ERROR_BUFFER_TOO_SMALL {
        v = vec![unsafe { std::mem::zeroed() }; cb as usize / size_of::<RASENTRYNAMEW>() + 1];
        v.iter_mut().for_each(|e| e.dwSize = size_of::<RASENTRYNAMEW>() as u32);
        rc = unsafe { RasEnumEntriesW(null(), ptr(&w), v.as_mut_ptr(), &mut cb, &mut n) };
    }
    if rc != 0 {
        return vec![];
    }
    v.iter().take(n as usize).map(|e| utf16(&e.szEntryName)).collect()
}

fn active() -> Vec<(String, HRASCONN)> {
    let (mut cb, mut n) = (size_of::<RASCONNW>() as u32, 0u32);
    let mut v: Vec<RASCONNW> = vec![unsafe { std::mem::zeroed() }; 1];
    v[0].dwSize = cb;
    let mut rc = unsafe { RasEnumConnectionsW(v.as_mut_ptr(), &mut cb, &mut n) };
    if rc == ERROR_BUFFER_TOO_SMALL {
        v = vec![unsafe { std::mem::zeroed() }; cb as usize / size_of::<RASCONNW>() + 1];
        v.iter_mut().for_each(|e| e.dwSize = size_of::<RASCONNW>() as u32);
        rc = unsafe { RasEnumConnectionsW(v.as_mut_ptr(), &mut cb, &mut n) };
    }
    if rc != 0 {
        return vec![];
    }
    v.iter().take(n as usize).map(|c| (utf16(&c.szEntryName), c.hrasconn)).collect()
}

pub fn vpns() -> Vec<Vpn> {
    let on = active();
    let mut out: Vec<Vpn> = vec![];
    let upbk = user_pbk();
    let unames = upbk.as_deref().map_or(vec![], |p| entries(Some(p)));
    for (pbk, names) in [(None, entries(None)), (upbk, unames)] {
        for name in names {
            if !out.iter().any(|v| v.name == name) {
                out.push(Vpn { on: on.iter().any(|c| c.0 == name), name, pbk: pbk.clone() });
            }
        }
    }
    out
}

pub fn ras_error(code: u32) -> String {
    let mut buf = [0u16; 256];
    if unsafe { RasGetErrorStringW(code, buf.as_mut_ptr(), buf.len() as u32) } == 0 {
        let s = utf16(&buf);
        if !s.is_empty() {
            return s;
        }
    }
    format!("code {}", code)
}

pub fn dial(name: &str, pbk: Option<&str>) -> Result<(), String> {
    unsafe {
        let mut p: RASDIALPARAMSW = std::mem::zeroed();
        p.dwSize = size_of::<RASDIALPARAMSW>() as u32;
        for (d, s) in p.szEntryName.iter_mut().zip(name.encode_utf16().take(256)) {
            *d = s;
        }
        let w = pbk.map(wide);
        let wp = w.as_ref().map_or(null(), |w| w.as_ptr());
        let mut saved = 0;
        let rc = RasGetEntryDialParamsW(wp, &mut p, &mut saved);
        if rc != 0 {
            return Err(ras_error(rc));
        }
        let mut h: HRASCONN = null_mut();
        let rc = RasDialW(null(), wp, &p, 0, null(), &mut h);
        p.szPassword.fill(0);
        if rc != 0 {
            if !h.is_null() {
                RasHangUpW(h);
            }
            return Err(if rc == ERROR_AUTHENTICATION_FAILURE as u32 || saved == 0 { "No saved password: enter it once in Windows' VPN settings".into() } else { ras_error(rc) });
        }
        Ok(())
    }
}

pub fn hang_up(name: &str) -> bool {
    let Some((_, h)) = active().into_iter().find(|c| c.0 == name) else { return false };
    unsafe { RasHangUpW(h) == 0 }
}

pub static TEMP_BYTES: AtomicU64 = AtomicU64::new(u64::MAX);

fn old_enough(m: &std::fs::Metadata, min_age: Duration) -> bool {
    m.modified().ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|a| a >= min_age)
}

fn reparse(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0
}

pub fn clean_dir(dir: &Path, min_age: Duration) -> (u64, u32) {
    let (mut bytes, mut files) = (0u64, 0u32);
    let Ok(rd) = std::fs::read_dir(dir) else { return (0, 0) };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
        if reparse(&m) {
            continue;
        }
        if m.is_dir() {
            let (b, f) = clean_dir(&p, min_age);
            (bytes, files) = (bytes + b, files + f);
            let _ = std::fs::remove_dir(&p);
        } else if old_enough(&m, min_age) && std::fs::remove_file(&p).is_ok() {
            (bytes, files) = (bytes + m.len(), files + 1);
        }
    }
    (bytes, files)
}

pub fn dir_bytes(dir: &Path, min_age: Duration) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .filter_map(|e| Some((e.path(), std::fs::symlink_metadata(e.path()).ok()?)))
        .filter(|(_, m)| !reparse(m))
        .map(|(p, m)| if m.is_dir() { dir_bytes(&p, min_age) } else if old_enough(&m, min_age) { m.len() } else { 0 })
        .sum()
}

const TEMP_AGE: Duration = Duration::from_secs(24 * 3600);

fn temp_dir() -> std::path::PathBuf {
    std::env::temp_dir()
}

pub fn measure_temp(done: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        TEMP_BYTES.store(dir_bytes(&temp_dir(), TEMP_AGE), Relaxed);
        done();
    });
}

pub fn clean_temp(done: impl FnOnce(String) + Send + 'static) {
    std::thread::spawn(move || {
        let (b, n) = clean_dir(&temp_dir(), TEMP_AGE);
        TEMP_BYTES.store(dir_bytes(&temp_dir(), TEMP_AGE), Relaxed);
        done(format!("Files deleted: {} ({})", n, crate::power::size(b)));
    });
}

pub fn temp_text() -> String {
    match TEMP_BYTES.load(Relaxed) {
        u64::MAX => "measuring…".into(),
        b => crate::power::size(b),
    }
}

pub fn bin_bytes() -> u64 {
    use windows_sys::Win32::UI::Shell::*;
    let mut i: SHQUERYRBINFO = unsafe { std::mem::zeroed() };
    i.cbSize = size_of::<SHQUERYRBINFO>() as u32;
    if unsafe { SHQueryRecycleBinW(null(), &mut i) } == 0 { i.i64Size.max(0) as u64 } else { 0 }
}

pub fn empty_bin(hwnd: HWND) -> bool {
    use windows_sys::Win32::UI::Shell::*;
    unsafe { SHEmptyRecycleBinW(hwnd, null(), SHERB_NOSOUND) == 0 }
}

const NIGHT: &str = r"Software\WinRust\NightLight";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub const NIGHT_STEPS: [u32; 5] = [0, 5500, 4800, 4200, 3600];

pub fn night_kelvin() -> u32 {
    sys::dword(HKEY_CURRENT_USER, NIGHT, "Kelvin").unwrap_or(0)
}

pub fn night_text() -> String {
    match night_kelvin() {
        0 => "Off".into(),
        k => format!("{k} K"),
    }
}

pub fn set_night(k: u32) {
    sys::set_hkcu_dword(NIGHT, "Kelvin", k);
    if k > 0 {
        if let Ok(exe) = std::env::current_exe() {
            sys::set_hkcu_string(RUN, "WinRustNightLight", &format!("\"{}\" --nightlight", exe.display()));
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new(exe).arg("--nightlight").creation_flags(0x0800_0000).spawn();
        }
    } else {
        sys::set_hkcu_string(RUN, "WinRustNightLight", "");
    }
}

pub fn gains(kelvin: u32) -> [f32; 3] {
    let t = kelvin.clamp(1000, 6500) as f32 / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.698_73 * (t - 60.0).powf(-0.133_204_76) };
    let g = if t <= 66.0 { 99.470_8 * t.ln() - 161.119_57 } else { 288.122_17 * (t - 60.0).powf(-0.075_514_85) };
    let b = if t >= 66.0 { 255.0 } else if t <= 19.0 { 0.0 } else { 138.517_73 * (t - 10.0).ln() - 305.044_8 };
    [r, g, b].map(|c| (c / 255.0).clamp(0.0, 1.0))
}

fn ramp(k: u32) -> [[u16; 256]; 3] {
    let g = if k == 0 { [1.0; 3] } else { gains(k) };
    let mut r = [[0u16; 256]; 3];
    for (c, ch) in r.iter_mut().enumerate() {
        for (i, v) in ch.iter_mut().enumerate() {
            *v = (i as f32 * 257.0 * g[c]).round() as u16;
        }
    }
    r
}

fn apply_ramp(r: &[[u16; 256]; 3]) {
    use windows_sys::Win32::{Graphics::Gdi::*, UI::ColorSystem::SetDeviceGammaRamp};
    let disp = wide("DISPLAY");
    for i in 0.. {
        let mut dd: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        dd.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        if unsafe { EnumDisplayDevicesW(null(), i, &mut dd, 0) } == 0 {
            break;
        }
        if dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        unsafe {
            let dc = CreateDCW(disp.as_ptr(), dd.DeviceName.as_ptr(), null(), null());
            if !dc.is_null() {
                SetDeviceGammaRamp(dc, r.as_ptr() as _);
                DeleteDC(dc);
            }
        }
    }
}

pub fn night_helper() {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    unsafe {
        let m = CreateMutexW(null(), 0, wide("WinRustNightLight").as_ptr());
        if m.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
    }
    let mut cached: Option<(u32, [[u16; 256]; 3])> = None;
    loop {
        let k = night_kelvin();
        let r = match cached {
            Some((ck, ref cr)) if ck == k => cr,
            _ => {
                cached = Some((k, ramp(k)));
                &cached.as_ref().unwrap().1
            }
        };
        apply_ramp(r);
        if k == 0 {
            return;
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

#[derive(Clone)]
pub struct App {
    pub name: String,
    pub version: String,
    pub uninstall: String,
}

pub fn programs() -> Vec<App> {
    let base = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let wow = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    let mut out: Vec<App> = vec![];
    for (root, path) in [(HKEY_LOCAL_MACHINE, base), (HKEY_LOCAL_MACHINE, wow), (HKEY_CURRENT_USER, base)] {
        for sub in sys::subkeys(root, path) {
            let key = format!(r"{path}\{sub}");
            let Some(name) = sys::string_long(root, &key, "DisplayName").filter(|n| !n.trim().is_empty()) else { continue };
            let Some(cmd) = sys::string_long(root, &key, "UninstallString").filter(|c| !c.trim().is_empty()) else { continue };
            if sys::dword(root, &key, "SystemComponent") == Some(1) || sys::string_long(root, &key, "ParentKeyName").is_some() {
                continue;
            }
            if !out.iter().any(|a| a.name == name) {
                out.push(App { version: sys::string_long(root, &key, "DisplayVersion").unwrap_or_default(), name, uninstall: cmd });
            }
        }
    }
    out.sort_by_cached_key(|a| a.name.to_lowercase());
    out
}

pub fn uninstall(cmd: &str) -> bool {
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let mut line = wide(cmd);
        if CreateProcessW(null(), line.as_mut_ptr(), null(), null(), 0, 0, null(), null(), &si, &mut pi) == 0 {
            return false;
        }
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_and_pac_addresses_are_checked() {
        assert!(valid_proxy("") && valid_proxy("proxy.corp:8080") && valid_proxy("http=a:1;https=b:2"));
        assert!(!valid_proxy("a b") && !valid_proxy("a\"b"));
        assert!(valid_pac("") && valid_pac("http://x/proxy.pac") && valid_pac("FILE://C:/p.pac"));
        assert!(!valid_pac("ftp://x") && !valid_pac("proxy.pac"));
    }

    #[test]
    fn warmer_light_cuts_blue_first() {
        let (w, n) = (gains(6500), gains(3600));
        assert!(w.iter().all(|&g| g > 0.95));
        assert!(n[0] > 0.99 && n[2] < n[1] && n[1] < n[0] && n[2] > 0.4);
        let r = ramp(3600);
        assert_eq!(r[0][0], 0);
        assert_eq!(r[0][255], 65535);
        assert!(r[2][255] < r[0][255]);
        assert_eq!(ramp(0)[2][255], 65535);
    }

    #[test]
    fn cleaning_removes_old_files_and_empty_folders_and_counts_them() {
        let d = std::env::temp_dir().join(format!("rs-clean-test-{}", std::process::id()));
        std::fs::create_dir_all(d.join("sub/deeper")).unwrap();
        std::fs::write(d.join("a.tmp"), [0u8; 10]).unwrap();
        std::fs::write(d.join("sub/deeper/b.tmp"), [0u8; 5]).unwrap();
        assert_eq!(dir_bytes(&d, Duration::from_secs(3600)), 0);
        assert_eq!(dir_bytes(&d, Duration::ZERO), 15);
        assert_eq!(clean_dir(&d, Duration::ZERO), (15, 2));
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        std::fs::remove_dir(&d).unwrap();
    }
}

#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn print_system() {
        println!("vpns: {:?}", vpns());
        println!("proxy: {} | pac: {}", proxy_text(), pac_text());
        println!("programs: {}", programs().len());
        println!("bin: {} bytes, night: {}", bin_bytes(), night_text());
    }
}
