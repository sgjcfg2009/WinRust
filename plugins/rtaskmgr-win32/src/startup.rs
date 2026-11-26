use std::{fs, path::PathBuf, ptr::null_mut};
use winapi::shared::minwindef::HKEY;
use winapi::um::{
    sysinfoapi::GetSystemTimeAsFileTime,
    winnt::{KEY_READ, KEY_SET_VALUE, REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_SZ},
    winreg::*,
};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

struct Source {
    label: &'static str,
    hklm: bool,
    run: Option<&'static str>,
    approved: &'static str,
}

static SOURCES: std::sync::LazyLock<[Source; 5]> = std::sync::LazyLock::new(|| [
    Source { label: "Registry, current user", hklm: false, run: Some(RUN), approved: "Run" },
    Source { label: "Registry, all users", hklm: true, run: Some(RUN), approved: "Run" },
    Source { label: "Registry, all users (32-bit)", hklm: true, run: Some(RUN32), approved: "Run32" },
    Source { label: "Startup folder (user)", hklm: false, run: None, approved: "StartupFolder" },
    Source { label: "Startup folder (all users)", hklm: true, run: None, approved: "StartupFolder" },
]);

pub struct Item {
    pub name: String,
    pub command: String,
    pub source: usize,
    pub enabled: bool,
}

pub const TASK: usize = 5;
pub const SERVICE: usize = 6;

impl Item {
    pub fn source_label(&self) -> &'static str {
        match self.source {
            TASK => "Task Scheduler (at sign-in or boot)",
            SERVICE => "Auto-start service (disabling = manual start)",
            i => SOURCES[i].label,
        }
    }
    pub fn needs_admin(&self) -> bool {
        self.source >= TASK || SOURCES[self.source].hklm
    }
    pub fn exe_name(&self) -> Option<String> {
        let exe = crate::model::exe_from_command(&self.command)?;
        Some(exe.rsplit(['\\', '/']).next()?.to_lowercase())
    }
    pub fn key(&self) -> String {
        format!("{}|{}", self.source, self.name)
    }
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn root(hklm: bool) -> HKEY {
    if hklm { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER }
}

fn open(r: HKEY, path: &str, access: u32) -> Option<HKEY> {
    let mut k: HKEY = null_mut();
    (unsafe { RegOpenKeyExW(r, w(path).as_ptr(), 0, access, &mut k) } == 0).then_some(k)
}

fn from_utf16_bytes(b: &[u8]) -> String {
    let u: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&u).trim_end_matches('\0').to_string()
}

fn string_values(r: HKEY, path: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let Some(k) = open(r, path, KEY_READ) else { return out };
    for i in 0.. {
        let mut name = vec![0u16; 16384];
        let mut nlen = name.len() as u32;
        let mut data = vec![0u8; 32768];
        let mut dlen = data.len() as u32;
        let mut ty = 0u32;
        let rc = unsafe { RegEnumValueW(k, i, name.as_mut_ptr(), &mut nlen, null_mut(), &mut ty, data.as_mut_ptr(), &mut dlen) };
        if rc != 0 {
            break;
        }
        if ty == REG_SZ || ty == REG_EXPAND_SZ {
            out.push((String::from_utf16_lossy(&name[..nlen as usize]), from_utf16_bytes(&data[..dlen as usize])));
        }
    }
    unsafe { RegCloseKey(k) };
    out
}

fn approved_enabled(hklm: bool, kind: &str, name: &str) -> bool {
    let Some(k) = open(root(hklm), &format!(r"{APPROVED}\{kind}"), KEY_READ) else { return true };
    let mut buf = [0u8; 12];
    let mut len = buf.len() as u32;
    let rc = unsafe { RegQueryValueExW(k, w(name).as_ptr(), null_mut(), null_mut(), buf.as_mut_ptr(), &mut len) };
    unsafe { RegCloseKey(k) };
    let flag = (rc == 0 && len != 0).then_some(buf[0]);
    approval_flag_enabled(flag)
}

fn approval_flag_enabled(flag: Option<u8>) -> bool {
    flag.is_none_or(|byte| byte & 1 == 0)
}

fn startup_dir(hklm: bool) -> Option<PathBuf> {
    let var = if hklm { "ProgramData" } else { "APPDATA" };
    std::env::var_os(var).map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
}

pub fn list() -> Vec<Item> {
    let mut out = tasks();
    out.extend(services());
    for (i, s) in SOURCES.iter().enumerate() {
        match s.run {
            Some(path) => {
                for (name, command) in string_values(root(s.hklm), path) {
                    let enabled = approved_enabled(s.hklm, s.approved, &name);
                    out.push(Item { name, command, source: i, enabled });
                }
            }
            None => {
                let Some(d) = startup_dir(s.hklm) else { continue };
                for e in fs::read_dir(d).into_iter().flatten().flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.eq_ignore_ascii_case("desktop.ini") {
                        continue;
                    }
                    let enabled = approved_enabled(s.hklm, s.approved, &name);
                    out.push(Item { name, command: e.path().display().to_string(), source: i, enabled });
                }
            }
        }
    }
    out
}

fn approved_value(enabled: bool, filetime: u64) -> [u8; 12] {
    let mut v = [0u8; 12];
    v[0] = if enabled { 2 } else { 3 };
    if !enabled {
        v[4..].copy_from_slice(&filetime.to_le_bytes());
    }
    v
}

pub fn set_enabled(it: &Item, on: bool) -> Result<(), u32> {
    match it.source {
        TASK => return cmd_ok("schtasks.exe", &["/change", "/tn", &it.name, if on { "/enable" } else { "/disable" }]),
        SERVICE => {
            cmd_ok("sc.exe", &["config", &it.name, "start=", if on { "auto" } else { "demand" }])?;
            if let Some(k) = create(HKEY_LOCAL_MACHINE, SVC_OFF) {
                unsafe {
                    if on {
                        RegDeleteValueW(k, w(&it.name).as_ptr());
                    } else {
                        let one = 1u32;
                        RegSetValueExW(k, w(&it.name).as_ptr(), 0, REG_DWORD, &one as *const u32 as *const u8, 4);
                    }
                    RegCloseKey(k);
                }
            }
            return Ok(());
        }
        _ => {}
    }
    let s = &SOURCES[it.source];
    let mut ft = unsafe { std::mem::zeroed() };
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    let value = approved_value(on, (ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64);
    let mut k: HKEY = null_mut();
    let path = w(&format!(r"{APPROVED}\{}", s.approved));
    let rc = unsafe {
        RegCreateKeyExW(root(s.hklm), path.as_ptr(), 0, null_mut(), 0, KEY_SET_VALUE, null_mut(), &mut k, null_mut())
    };
    if rc != 0 {
        return Err(rc as u32);
    }
    let rc = unsafe { RegSetValueExW(k, w(&it.name).as_ptr(), 0, REG_BINARY, value.as_ptr(), value.len() as u32) };
    unsafe { RegCloseKey(k) };
    if rc == 0 { Ok(()) } else { Err(rc as u32) }
}

pub fn delete(it: &Item) -> Result<(), u32> {
    match it.source {
        TASK => return cmd_ok("schtasks.exe", &["/delete", "/tn", &it.name, "/f"]),
        SERVICE => {
            let r = cmd_ok("sc.exe", &["delete", &it.name]);
            if r.is_ok() && let Some(k) = open(HKEY_LOCAL_MACHINE, SVC_OFF, KEY_SET_VALUE) {
                unsafe {
                    RegDeleteValueW(k, w(&it.name).as_ptr());
                    RegCloseKey(k);
                }
            }
            return r;
        }
        _ => {}
    }
    let s = &SOURCES[it.source];
    if let Some(path) = s.run {
        let Some(k) = open(root(s.hklm), path, KEY_SET_VALUE) else { return Err(5) };
        let rc = unsafe { RegDeleteValueW(k, w(&it.name).as_ptr()) };
        unsafe { RegCloseKey(k) };
        if let Some(ak) = open(root(s.hklm), &format!(r"{APPROVED}\{}", s.approved), KEY_SET_VALUE) {
            unsafe {
                RegDeleteValueW(ak, w(&it.name).as_ptr());
                RegCloseKey(ak);
            }
        }
        return if rc == 0 { Ok(()) } else { Err(rc as u32) };
    }
    if let Err(e) = fs::remove_file(&it.command) {
        return Err(e.raw_os_error().unwrap_or(5) as u32);
    }
    if let Some(ak) = open(root(s.hklm), &format!(r"{APPROVED}\{}", s.approved), KEY_SET_VALUE) {
        unsafe {
            RegDeleteValueW(ak, w(&it.name).as_ptr());
            RegCloseKey(ak);
        }
    }
    Ok(())
}

fn cmd_ok(prog: &str, args: &[&str]) -> Result<(), u32> {
    use std::os::windows::process::CommandExt;
    match std::process::Command::new(prog).args(args).creation_flags(0x0800_0000 ).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => Err(5),
        Err(e) => Err(e.raw_os_error().unwrap_or(1) as u32),
    }
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let rest = &xml[xml.find(&open)? + open.len()..];
    Some(&rest[..rest.find(&format!("</{tag}>"))?])
}

pub fn parse_task(xml: &str) -> Option<(String, bool)> {
    let triggers = tag(xml, "Triggers")?;
    if !triggers.contains("<LogonTrigger") && !triggers.contains("<BootTrigger") {
        return None;
    }
    let exec = tag(xml, "Exec")?;
    let cmd = xml_unescape(tag(exec, "Command")?.trim());
    let args = tag(exec, "Arguments").map(|a| xml_unescape(a.trim())).unwrap_or_default();
    let quoted = if cmd.contains(' ') && !cmd.starts_with('"') { format!("\"{cmd}\"") } else { cmd };
    let enabled = !tag(xml, "Settings").is_some_and(|s| s.contains("<Enabled>false</Enabled>"));
    Some((if args.is_empty() { quoted } else { format!("{quoted} {args}") }, enabled))
}

fn decode_task_file(b: &[u8]) -> String {
    match b {
        [0xFF, 0xFE, rest @ ..] => from_utf16_bytes(rest),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

fn tasks() -> Vec<Item> {
    let Some(root) = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join(r"System32\Tasks")) else { return vec![] };
    let mut out = vec![];
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if dir != root || !e.file_name().eq_ignore_ascii_case("Microsoft") {
                    stack.push(p);
                }
            } else if let Some((command, enabled)) = fs::read(&p).ok().and_then(|b| parse_task(&decode_task_file(&b))) {
                let rel = p.strip_prefix(&root).map(|r| format!("\\{}", r.display())).unwrap_or_default();
                out.push(Item { name: rel, command, source: TASK, enabled });
            }
        }
    }
    out
}

const SERVICES_KEY: &str = r"SYSTEM\CurrentControlSet\Services";
const SVC_OFF: &str = r"SOFTWARE\WinRust\rtaskmgr\StartupSvc";

fn create(r: HKEY, path: &str) -> Option<HKEY> {
    let mut k: HKEY = null_mut();
    (unsafe { RegCreateKeyExW(r, w(path).as_ptr(), 0, null_mut(), 0, KEY_SET_VALUE, null_mut(), &mut k, null_mut()) } == 0).then_some(k)
}

fn dword(k: HKEY, name: &str) -> Option<u32> {
    let mut v = 0u32;
    let mut len = 4u32;
    let mut ty = 0u32;
    let rc = unsafe { RegQueryValueExW(k, w(name).as_ptr(), null_mut(), &mut ty, &mut v as *mut u32 as *mut u8, &mut len) };
    (rc == 0 && ty == REG_DWORD).then_some(v)
}

fn sz(k: HKEY, name: &str) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let mut len = buf.len() as u32;
    let mut ty = 0u32;
    let rc = unsafe { RegQueryValueExW(k, w(name).as_ptr(), null_mut(), &mut ty, buf.as_mut_ptr(), &mut len) };
    (rc == 0 && (ty == REG_SZ || ty == REG_EXPAND_SZ)).then(|| from_utf16_bytes(&buf[..len as usize]))
}

fn subkeys(r: HKEY, path: &str) -> Vec<String> {
    let Some(k) = open(r, path, KEY_READ) else { return vec![] };
    let mut out = vec![];
    for i in 0.. {
        let mut name = [0u16; 256];
        let mut n = name.len() as u32;
        if unsafe { RegEnumKeyExW(k, i, name.as_mut_ptr(), &mut n, null_mut(), null_mut(), null_mut(), null_mut()) } != 0 {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..n as usize]));
    }
    unsafe { RegCloseKey(k) };
    out
}

fn value_names(k: HKEY) -> Vec<String> {
    let mut out = vec![];
    for i in 0.. {
        let mut name = [0u16; 512];
        let mut n = name.len() as u32;
        if unsafe { RegEnumValueW(k, i, name.as_mut_ptr(), &mut n, null_mut(), null_mut(), null_mut(), null_mut()) } != 0 {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..n as usize]));
    }
    out
}

pub fn third_party_image(image: &str) -> bool {
    let i = image.to_lowercase();
    !(i.is_empty() || i.contains("\\windows\\") || i.starts_with("system32") || i.starts_with("\\systemroot") || i.starts_with("%systemroot%") || i.contains("svchost.exe"))
}

fn services() -> Vec<Item> {
    let ours = open(HKEY_LOCAL_MACHINE, SVC_OFF, KEY_READ).map_or(vec![], |k| {
        let v = value_names(k);
        unsafe { RegCloseKey(k) };
        v
    });
    let mut out = vec![];
    for name in subkeys(HKEY_LOCAL_MACHINE, SERVICES_KEY) {
        let Some(k) = open(HKEY_LOCAL_MACHINE, &format!(r"{SERVICES_KEY}\{name}"), KEY_READ) else { continue };
        let (start, ty, image) = (dword(k, "Start"), dword(k, "Type").unwrap_or(0), sz(k, "ImagePath").unwrap_or_default());
        unsafe { RegCloseKey(k) };
        let mine = ours.iter().any(|o| o.eq_ignore_ascii_case(&name));
        if ty & 0x30 == 0 || !third_party_image(&image) || !(start == Some(2) || mine && start == Some(3)) {
            continue;
        }
        out.push(Item { name, command: image, source: SERVICE, enabled: start == Some(2) });
    }
    out
}

#[derive(Default, Debug, PartialEq)]
pub struct BootLog {
    pub boots: Vec<(String, u32, u32)>,
    pub slow: Vec<(String, String, u32)>,
}

fn data_field<'a>(ev: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("<Data Name='{name}'>");
    let rest = &ev[ev.find(&key)? + key.len()..];
    Some(&rest[..rest.find("</Data>")?])
}

pub fn parse_boot_log(xml: &str) -> BootLog {
    let mut log = BootLog::default();
    for ev in xml.split("<Event ").skip(1) {
        let id = tag(ev, "EventID").and_then(|s| s.trim().parse::<u32>().ok());
        let num = |n: &str| data_field(ev, n).and_then(|v| v.trim().parse::<u32>().ok());
        match id {
            Some(100) => {
                let date = ev.split("SystemTime='").nth(1).map(|s| s.chars().take(16).collect::<String>().replace('T', " ")).unwrap_or_default();
                if let Some(b) = num("BootTime") {
                    log.boots.push((date, b, num("MainPathBootTime").unwrap_or(0)));
                }
            }
            Some(101 | 103) => {
                let (Some(name), Some(deg)) = (data_field(ev, "Name"), num("DegradationTime")) else { continue };
                let name = xml_unescape(name).to_lowercase();
                let friendly = data_field(ev, "FriendlyName").map(xml_unescape).unwrap_or_default();
                match log.slow.iter_mut().find(|s| s.0 == name) {
                    Some(s) => s.2 = s.2.max(deg),
                    None => log.slow.push((name, friendly, deg)),
                }
            }
            _ => {}
        }
    }
    log.slow.sort_by(|a, b| b.2.cmp(&a.2));
    log
}

pub fn boot_log() -> &'static BootLog {
    static LOG: std::sync::OnceLock<BootLog> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("wevtutil.exe")
            .args(["qe", "Microsoft-Windows-Diagnostics-Performance/Operational", "/q:*[System[(EventID=100 or EventID=101 or EventID=103)]]", "/c:200", "/rd:true", "/f:xml"])
            .creation_flags(0x0800_0000)
            .output();
        out.map(|o| parse_boot_log(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
    })
}

pub fn boot_impact(it: &Item, log: &BootLog) -> Option<u32> {
    let key = if it.source == SERVICE { it.name.to_lowercase() } else { it.exe_name()? };
    log.slow.iter().find(|s| s.0 == key).map(|s| s.2)
}

const SELF_NAME: &str = "RTaskMgr";

pub fn self_autostart() -> bool {
    string_values(HKEY_CURRENT_USER, RUN).iter().any(|(n, _)| n == SELF_NAME)
}

pub fn set_self_autostart(on: bool, exe: &str) -> bool {
    let Some(k) = open(HKEY_CURRENT_USER, RUN, KEY_SET_VALUE) else { return false };
    let rc = if on {
        let v: Vec<u16> = format!("\"{exe}\" --tray").encode_utf16().chain(Some(0)).collect();
        unsafe { RegSetValueExW(k, w(SELF_NAME).as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32) }
    } else {
        unsafe { RegDeleteValueW(k, w(SELF_NAME).as_ptr()) }
    };
    unsafe { RegCloseKey(k) };
    rc == 0 || !on
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_flags_round_trip() {
        let filetime = 0x0102_0304_0506_0708u64;
        let disabled = approved_value(false, filetime);
        assert_eq!(disabled[..4], [3, 0, 0, 0]);
        assert_eq!(disabled[4..], filetime.to_le_bytes());
        assert!(!approval_flag_enabled(disabled.first().copied()));

        let enabled = approved_value(true, filetime);
        assert_eq!(enabled, [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(approval_flag_enabled(enabled.first().copied()));
        assert!(approval_flag_enabled(None));
    }

    #[test]
    fn task_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?><Task><Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>
            <Settings><Enabled>false</Enabled></Settings><Actions Context="Author"><Exec><Command>C:\Program Files\App\up.exe</Command><Arguments>/silent &amp; /x</Arguments></Exec></Actions></Task>"#;
        assert_eq!(parse_task(xml), Some((r#""C:\Program Files\App\up.exe" /silent & /x"#.to_string(), false)));
        assert_eq!(parse_task(&xml.replace("LogonTrigger", "TimeTrigger")), None, "по расписанию — не автозагрузка");
        assert!(parse_task(&xml.replace("<Settings><Enabled>false</Enabled></Settings>", "<Settings></Settings>")).unwrap().1);
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("<a>й</a>".encode_utf16().flat_map(|c| c.to_le_bytes()));
        assert_eq!(decode_task_file(&bytes), "<a>й</a>");
    }

    #[test]
    fn service_images() {
        assert!(third_party_image(r#""C:\Program Files\Steam\steamservice.exe" /RunAsService"#));
        assert!(!third_party_image(r"C:\WINDOWS\system32\svchost.exe -k netsvcs"));
        assert!(!third_party_image(r"system32\DRIVERS\x.sys"));
        assert!(!third_party_image(r"\SystemRoot\System32\drivers\y.sys"));
        assert!(!third_party_image(""));
    }

    #[test]
    fn boot_log_events() {
        let xml = "<Events><Event xmlns='x'><System><EventID>100</EventID><TimeCreated SystemTime='2026-11-20T08:15:01.1Z'/></System><EventData><Data Name='BootTime'>41234</Data><Data Name='MainPathBootTime'>15000</Data></EventData></Event>\
            <Event xmlns='x'><System><EventID>101</EventID></System><EventData><Data Name='Name'>OneDrive.exe</Data><Data Name='FriendlyName'>Microsoft OneDrive</Data><Data Name='TotalTime'>3000</Data><Data Name='DegradationTime'>2100</Data></EventData></Event>\
            <Event xmlns='x'><System><EventID>101</EventID></System><EventData><Data Name='Name'>onedrive.exe</Data><Data Name='DegradationTime'>900</Data></EventData></Event>\
            <Event xmlns='x'><System><EventID>103</EventID></System><EventData><Data Name='Name'>Steam Client Service</Data><Data Name='FriendlyName'>Steam</Data><Data Name='DegradationTime'>3000</Data></EventData></Event></Events>";
        let log = parse_boot_log(xml);
        assert_eq!(log.boots, vec![("2026-11-20 08:15".to_string(), 41234, 15000)]);
        assert_eq!(log.slow, vec![("steam client service".to_string(), "Steam".to_string(), 3000), ("onedrive.exe".to_string(), "Microsoft OneDrive".to_string(), 2100)]);
        let it = Item { name: "OneDrive".into(), command: r#""C:\Users\a\OneDrive.exe" /background"#.into(), source: 0, enabled: true };
        assert_eq!(boot_impact(&it, &log), Some(2100));
        assert_eq!(parse_boot_log(""), BootLog::default());
    }

    #[test]
    fn utf16_bytes_decode() {
        assert_eq!(from_utf16_bytes(&[0x41, 0, 0x42, 0, 0, 0]), "AB");
    }
}
