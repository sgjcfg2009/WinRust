use std::{
    ffi::c_void,
    os::windows::process::CommandExt,
    process::Command,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES},
    System::{
        Diagnostics::{
            Debug::ReadProcessMemory,
            ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS},
        },
        Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, TerminateProcess, WaitForSingleObject},
    },
};

const _: () = assert!(size_of::<usize>() == 8, "the PEB offsets below are x64");

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(h: *mut c_void, class: u32, out: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

fn for_each_proc(mut f: impl FnMut(u32, &[u16])) {
    unsafe {
        let s = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if s == INVALID_HANDLE_VALUE {
            return;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(s, &mut e) != 0;
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            f(e.th32ProcessID, &e.szExeFile[..n]);
            ok = Process32NextW(s, &mut e) != 0;
        }
        CloseHandle(s);
    }
}

pub fn list() -> Vec<(u32, String)> {
    let mut v = Vec::with_capacity(256);
    for_each_proc(|pid, exe| v.push((pid, String::from_utf16_lossy(exe))));
    v
}

pub fn exe_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::System::Threading::{PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let (mut b, mut n) = ([0u16; 520], 520u32);
        let ok = QueryFullProcessImageNameW(h, 0, b.as_mut_ptr(), &mut n) != 0;
        CloseHandle(h);
        if !ok {
            return None;
        }
        let slice = &b[..n as usize];
        let start = slice.iter().rposition(|&c| c == b'\\' as u16).map_or(0, |i| i + 1);
        Some(String::from_utf16_lossy(&slice[start..]))
    }
}

fn eq_exe_name(w: &[u16], s: &str) -> bool {
    if s.is_ascii() {
        let b = s.as_bytes();
        w.len() == b.len() && w.iter().zip(b).all(|(&u, &c)| u <= 0x7F && (u as u8).eq_ignore_ascii_case(&c))
    } else {
        String::from_utf16_lossy(w).eq_ignore_ascii_case(s)
    }
}

pub fn pids(name: &str) -> Vec<u32> {
    let mut v = Vec::new();
    for_each_proc(|pid, exe| {
        if eq_exe_name(exe, name) {
            v.push(pid);
        }
    });
    v
}

pub fn cmdline(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(0x0410, 0, pid);
        if h.is_null() {
            return None;
        }
        let read = |addr: usize, buf: *mut c_void, n: usize| ReadProcessMemory(h, addr as *const c_void, buf, n, null_mut()) != 0;
        let r = (|| {
            let mut pbi = [0usize; 6];
            if NtQueryInformationProcess(h, 0, pbi.as_mut_ptr().cast(), 48, null_mut()) != 0 {
                return None;
            }
            let mut params = 0usize;
            if !read(pbi[1] + 0x20, (&mut params as *mut usize).cast(), 8) {
                return None;
            }
            let mut us = [0u8; 16];
            if !read(params + 0x70, us.as_mut_ptr().cast(), 16) {
                return None;
            }
            let len = u16::from_le_bytes([us[0], us[1]]) as usize;
            let buf = usize::from_le_bytes(us[8..16].try_into().ok()?);
            let mut w = vec![0u16; len / 2];
            if !read(buf, w.as_mut_ptr().cast(), len) {
                return None;
            }
            Some(String::from_utf16_lossy(&w))
        })();
        CloseHandle(h);
        r
    }
}

pub fn enable_debug_privilege() {
    const PRIVS: [&str; 4] = [
        "SeDebugPrivilege",
        "SeTakeOwnershipPrivilege",
        "SeBackupPrivilege",
        "SeRestorePrivilege",
    ];
    unsafe {
        let mut tok = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut tok) == 0 || tok.is_null() {
            return;
        }
        for name in PRIVS {
            let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
            tp.PrivilegeCount = 1;
            tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            if crate::with_wide(name, |w| LookupPrivilegeValueW(null(), w, &mut tp.Privileges[0].Luid)) != 0 {
                AdjustTokenPrivileges(tok, 0, &tp, 0, null_mut(), null_mut());
            }
        }
        CloseHandle(tok);
    }
}

pub fn kill(pid: u32) {
    const ACCESS: u32 = 0x0010_0001;
    unsafe {
        let mut h = OpenProcess(ACCESS, 0, pid);
        if h.is_null() {
            enable_debug_privilege();
            h = OpenProcess(ACCESS, 0, pid);
        }
        if !h.is_null() {
            TerminateProcess(h, 1);
            WaitForSingleObject(h, 1500);
            CloseHandle(h);
        }
    }
}

pub fn kill_and_wait(names: &[&str], timeout_ms: u32) -> Vec<String> {
    enable_debug_privilege();
    let me = unsafe { GetCurrentProcessId() };
    const ACCESS: u32 = 0x0010_0001;
    let mut handles = Vec::new();
    for_each_proc(|pid, exe| {
        if pid != me && names.iter().any(|n| eq_exe_name(exe, n)) {
            unsafe {
                let h = OpenProcess(ACCESS, 0, pid);
                if !h.is_null() {
                    TerminateProcess(h, 1);
                    handles.push(h);
                }
            }
        }
    });
    if handles.is_empty() {
        return Vec::new();
    }
    let mut any_timeout = false;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    for h in handles {
        let rem = deadline.saturating_duration_since(Instant::now()).as_millis().min(u32::MAX as u128) as u32;
        unsafe {
            if WaitForSingleObject(h, rem) != 0 {
                any_timeout = true;
            }
            CloseHandle(h);
        }
    }
    if !any_timeout {
        return Vec::new();
    }
    let mut survivors = Vec::new();
    for_each_proc(|pid, exe| {
        if pid != me && names.iter().any(|n| eq_exe_name(exe, n)) {
            survivors.push(pid);
        }
    });
    if !survivors.is_empty() {
        for pid in survivors {
            let _ = Command::new("taskkill.exe").args(["/F", "/T", "/PID", &pid.to_string()]).creation_flags(0x0800_0000).output();
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let mut alive = Vec::new();
    for_each_proc(|pid, exe| {
        if pid != me && names.iter().any(|n| eq_exe_name(exe, n)) {
            alive.push(String::from_utf16_lossy(exe));
        }
    });
    alive.sort();
    alive.dedup();
    alive
}

pub fn kill_names(names: &[&str]) {
    let _ = kill_and_wait(names, 3000);
}

fn ends_with_ignore_ascii_case(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn is_shell_cmdline(c: &str) -> bool {
    let mut s = c.trim_end();
    if ends_with_ignore_ascii_case(s, "/nouaccheck") {
        let r = &s[..s.len() - 11];
        if !r.ends_with(char::is_whitespace) {
            return false;
        }
        s = r.trim_end();
    }
    let s = s.strip_suffix('"').unwrap_or(s);
    ends_with_ignore_ascii_case(s, "explorer.exe")
}

pub fn shell_explorers() -> Vec<u32> {
    pids("explorer.exe").into_iter().filter(|&p| cmdline(p).is_some_and(|c| is_shell_cmdline(&c))).collect()
}

pub fn kill_shell_explorers() {
    shell_explorers().into_iter().for_each(kill);
}

pub fn elevated() -> bool {
    use windows_sys::Win32::{
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation, TokenElevationType, TokenIntegrityLevel},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn IsUserAnAdmin() -> i32;
    }
    let (mut t, mut e, mut n) = (null_mut(), TOKEN_ELEVATION { TokenIsElevated: 0 }, 0u32);
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut t) == 0 || t.is_null() {
            return IsUserAnAdmin() != 0;
        }
        let ok = GetTokenInformation(t, TokenElevation, &mut e as *mut _ as _, size_of::<TOKEN_ELEVATION>() as u32, &mut n);
        let (mut et, mut etn) = (0u32, 0u32);
        let default_admin = GetTokenInformation(t, TokenElevationType, &mut et as *mut _ as _, 4, &mut etn) != 0 && et == 1 && IsUserAnAdmin() != 0;
        if default_admin {
            CloseHandle(t);
            return true;
        }
        if ok == 0 || e.TokenIsElevated == 0 {
            CloseHandle(t);
            return false;
        }
        let mut need = 0u32;
        GetTokenInformation(t, TokenIntegrityLevel, null_mut(), 0, &mut need);
        let mut high = true;
        if need >= size_of::<*const u8>() as u32 {
            let mut stack = [0u8; 128];
            let mut heap = Vec::new();
            let buf: &mut [u8] = if (need as usize) <= stack.len() {
                &mut stack[..need as usize]
            } else {
                heap.resize(need as usize, 0);
                &mut heap
            };
            if GetTokenInformation(t, TokenIntegrityLevel, buf.as_mut_ptr().cast(), need, &mut need) != 0 {
                let sid = *(buf.as_ptr() as *const *const u8);
                if !sid.is_null() {
                    let count = *sid.add(1) as usize;
                    if count >= 1 {
                        let o = 8 + (count - 1) * 4;
                        let rid = u32::from_le_bytes([*sid.add(o), *sid.add(o + 1), *sid.add(o + 2), *sid.add(o + 3)]);
                        high = rid >= 0x3000;
                    }
                }
            }
        }
        CloseHandle(t);
        high
    }
}

pub fn start_shell_explorer() -> std::io::Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};
    let uac_on = crate::reg::get_dword(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System", "EnableLUA") != Some(0);
    if elevated() && uac_on {
        Command::new("runas.exe").args(["/trustlevel:0x20000", r"C:\Windows\explorer.exe"]).creation_flags(0x0800_0000).status().map(|_| ())
    } else {
        Command::new(r"C:\Windows\explorer.exe").spawn().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::is_shell_cmdline;

    #[test]
    fn only_a_bare_explorer_is_the_shell() {
        assert!(is_shell_cmdline(r"C:\Windows\explorer.exe"));
        assert!(is_shell_cmdline(r#""C:\Windows\explorer.exe""#));
        assert!(is_shell_cmdline(r#""C:\Windows\EXPLORER.EXE" /NoUACCheck "#));
        assert!(!is_shell_cmdline(r"C:\Windows\explorer.exe C:\Users"));
        assert!(!is_shell_cmdline(r"C:\Windows\explorer.exe /factory,{5BD95610-9434-43C2-886C-57852CC8A120} -Embedding"));
        assert!(!is_shell_cmdline(r"C:\Windows\explorer.exe ms-settings:"));
    }

    #[test]
    fn lists_own_process() {
        let me = std::process::id();
        assert!(super::list().iter().any(|(p, _)| *p == me));
        assert!(super::cmdline(me).is_some_and(|c| !c.is_empty()));
        let name = super::exe_name(me).unwrap();
        assert!(name.to_lowercase().ends_with(".exe") && !name.contains('\\'), "{name}");
        assert!(super::list().iter().any(|(p, n)| *p == me && n.eq_ignore_ascii_case(&name)));
    }
}
