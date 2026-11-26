use crate::*;
use std::sync::OnceLock;
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    Security::{GetTokenInformation, LookupAccountNameW, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER, TokenUser},
    System::{
        Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS},
        RemoteDesktop::{
            ProcessIdToSessionId, WTS_CURRENT_SERVER_HANDLE, WTS_INFO_CLASS, WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName,
        },
        Threading::{GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

pub(crate) struct Session {
    pub sid: String,
    pub other: bool,
}

static SESSION: OnceLock<Result<Session, String>> = OnceLock::new();

fn sid_text(sid: *mut core::ffi::c_void) -> String {
    let p = sid as *const u8;
    unsafe {
        let (revision, count) = (*p, *p.add(1) as usize);
        let mut authority = 0u64;
        for i in 0..6 {
            authority = authority << 8 | *p.add(2 + i) as u64;
        }
        let mut s = format!("S-{revision}-{authority}");
        for i in 0..count {
            let o = 8 + i * 4;
            let v = u32::from_le_bytes([*p.add(o), *p.add(o + 1), *p.add(o + 2), *p.add(o + 3)]);
            s.push('-');
            s.push_str(&v.to_string());
        }
        s
    }
}

fn process_sid(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut token = std::ptr::null_mut();
        let ok = OpenProcessToken(h, TOKEN_QUERY, &mut token);
        CloseHandle(h);
        if ok == 0 || token.is_null() {
            return None;
        }
        let mut need = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut need);
        let mut buf = vec![0u8; need.max(1) as usize];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), need, &mut need);
        CloseHandle(token);
        if ok == 0 || need < size_of::<TOKEN_USER>() as u32 {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        Some(sid_text(user.User.Sid))
    }
}

fn wts_user(session: u32) -> Option<String> {
    unsafe fn query(session: u32, class: WTS_INFO_CLASS) -> Option<String> {
        let mut buf: *mut u16 = std::ptr::null_mut();
        let mut len = 0u32;
        if unsafe { WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, session, class, &mut buf, &mut len) } == 0 || buf.is_null() {
            return None;
        }
        let units = unsafe { std::slice::from_raw_parts(buf, (len / 2) as usize) };
        let s = String::from_utf16_lossy(&units[..units.iter().position(|&c| c == 0).unwrap_or(units.len())]);
        unsafe { WTSFreeMemory(buf as *mut core::ffi::c_void) };
        Some(s)
    }
    let name = unsafe { query(session, WTSUserName) }?;
    if name.is_empty() {
        return None;
    }
    let domain = unsafe { query(session, WTSDomainName) }.unwrap_or_default();
    Some(if domain.is_empty() { name } else { format!("{domain}\\{name}") })
}

fn name_sid(account: &str) -> Option<String> {
    let name = wide(account);
    unsafe {
        let mut sid = [0u8; 68];
        let mut len = sid.len() as u32;
        let mut domain = [0u16; 256];
        let mut dlen = domain.len() as u32;
        let mut kind: SID_NAME_USE = 0;
        let ok = LookupAccountNameW(std::ptr::null(), name.as_ptr(), sid.as_mut_ptr().cast(), &mut len, domain.as_mut_ptr(), &mut dlen, &mut kind);
        (ok != 0).then(|| sid_text(sid.as_mut_ptr().cast()))
    }
}

fn shell_sid(session: u32) -> Option<String> {
    const NAMES: [&str; 4] = ["explorer.exe", "topbar.exe", "rust-explorer.exe", "rs-shell.exe"];
    let mut best: Option<(usize, bool, String)> = None;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS as u32, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]);
            if let Some(rank) = NAMES.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
                let mut s = 0u32;
                let same = ProcessIdToSessionId(e.th32ProcessID, &mut s) != 0 && s == session;
                if let Some(sid) = process_sid(e.th32ProcessID) {
                    let better = match &best {
                        None => true,
                        Some((r, bs, _)) => (same, rank) > (*bs, *r) || (same == *bs && rank < *r),
                    };
                    if better {
                        best = Some((rank, same, sid));
                    }
                }
            }
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    best.map(|(_, _, sid)| sid)
}

fn detect() -> Result<Session, String> {
    unsafe {
        let mut session = 0u32;
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
        let sid = wts_user(session).and_then(|a| name_sid(&a)).or_else(|| shell_sid(session));
        let token = process_sid(GetCurrentProcessId()).ok_or_else(|| "could not read the installer's own account SID".to_string())?;
        let sid = sid.ok_or_else(|| "could not determine the signed-in user".to_string())?;
        if matches!(sid.as_str(), "S-1-5-18" | "S-1-5-19" | "S-1-5-20") {
            return Err("the session shell does not run as a normal user (SYSTEM or a service)".into());
        }
        Ok(Session { other: sid != token, sid })
    }
}

pub(crate) fn init() -> Result<&'static Session, String> {
    let s = SESSION.get_or_init(detect).as_ref().map_err(Clone::clone)?;
    if s.other {
        log(&format!("elevated as another account: per-user settings go to {}", s.sid));
        unsafe { env::set_var(i18n::reg::SID_ENV, &s.sid) };
    }
    Ok(s)
}

pub(crate) fn init_soft() {
    if let Err(e) = init() {
        soft("signed-in user unknown, per-user settings are removed from the current account", Err(e));
    }
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub(crate) fn register_task_xml(name: &str, xml: &str) -> Result<(), String> {
    let f = env::temp_dir().join(format!("winrust-task-{}-{}.xml", std::process::id(), name));
    let xml16 = xml.replacen("encoding=\"UTF-8\"", "encoding=\"UTF-16\"", 1);
    let mut bytes = Vec::with_capacity(2 + xml16.len() * 2);
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    for u in xml16.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    fs::write(&f, &bytes).map_err(|e| format!("{}: {e}", f.display()))?;
    let r = run("schtasks", &["/create", "/tn", name, "/xml", &f.to_string_lossy(), "/f"]);
    let _ = fs::remove_file(&f);
    r
}

fn target_sid() -> Option<String> {
    if let Ok(sid) = env::var(i18n::reg::SID_ENV) {
        if i18n::reg::valid_sid(&sid) {
            return Some(sid);
        }
    }
    SESSION.get().and_then(|r| r.as_ref().ok()).map(|s| s.sid.clone())
        .or_else(|| process_sid(unsafe { GetCurrentProcessId() }))
}

pub(crate) fn register_logon_task(name: &str, exe: &Path, args: &str, elevated: bool) -> Result<(), String> {
    let sid = target_sid().ok_or_else(|| "could not determine user SID for task".to_string())?;
    let level = if elevated { "HighestAvailable" } else { "LeastPrivilege" };
    let arg_xml = if args.is_empty() { String::new() } else { format!("<Arguments>{}</Arguments>", xml_esc(args)) };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>{level}</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>4</Priority><Enabled>true</Enabled></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command>{arg_xml}</Exec></Actions>
</Task>"#,
        xml_esc(&exe.to_string_lossy())
    );
    register_task_xml(name, &xml)
}

pub(crate) fn register_manual_task(name: &str, exe: &Path, elevated: bool) -> Result<(), String> {
    let sid = target_sid().ok_or_else(|| "could not determine user SID for task".to_string())?;
    let level = if elevated { "HighestAvailable" } else { "LeastPrivilege" };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Principals><Principal id="Author"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>{level}</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>4</Priority><Enabled>true</Enabled></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command></Exec></Actions>
</Task>"#,
        xml_esc(&exe.to_string_lossy())
    );
    register_task_xml(name, &xml)
}

pub(crate) fn logon_task(name: &str, exe: &Path) -> bool {
    register_logon_task(name, exe, "", true).is_ok()
}

pub(crate) fn profile_path() -> String {
    match env::var(i18n::reg::SID_ENV).ok().filter(|s| i18n::reg::valid_sid(s)) {
        Some(sid) => format!(
            r"$h=(Get-ItemProperty 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}').ProfileImagePath; $d=(Get-ItemProperty 'Registry::HKEY_USERS\{sid}\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders').Personal; $d=[Environment]::ExpandEnvironmentVariables($d.Replace('%USERPROFILE%',$h)); $p=Join-Path $d ($(if($PSVersionTable.PSEdition -eq 'Core'){{'PowerShell'}}else{{'WindowsPowerShell'}}) + '\Microsoft.PowerShell_profile.ps1');"
        ),
        None => "$p=$PROFILE;".into(),
    }
}
