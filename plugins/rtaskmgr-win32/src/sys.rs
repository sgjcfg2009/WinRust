use std::{collections::HashMap, ptr::null_mut};
use winapi::um::{
    errhandlingapi::GetLastError,
    handleapi::CloseHandle,
    processthreadsapi::{OpenProcess, SetPriorityClass},
    winbase::{
        ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetProcessAffinityMask, HIGH_PRIORITY_CLASS,
        IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    },
    winnt::{HANDLE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, SERVICE_WIN32},
    winsvc::*,
};

use crate::ui::wide;

unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }
}

pub struct Svc {
    pub name: String,
    pub display: String,
    pub pid: u32,
    pub running: bool,
}

pub fn services() -> Vec<Svc> {
    let mut out = vec![];
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ENUMERATE_SERVICE);
        if scm.is_null() {
            return out;
        }
        let (mut need, mut cnt, mut resume) = (0u32, 0u32, 0u32);
        let enumerate = |buf: *mut u8, len: u32, need: &mut u32, cnt: &mut u32, resume: &mut u32| {
            EnumServicesStatusExW(
                scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_STATE_ALL, buf, len, need, cnt, resume, std::ptr::null(),
            )
        };
        enumerate(null_mut(), 0, &mut need, &mut cnt, &mut resume);
        let mut buf = vec![0u64; need as usize / 8 + 1024];
        resume = 0;
        if enumerate(buf.as_mut_ptr() as *mut u8, (buf.len() * 8) as u32, &mut need, &mut cnt, &mut resume) != 0 {
            let arr = buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW;
            for i in 0..cnt as usize {
                let e = &*arr.add(i);
                out.push(Svc {
                    name: from_wide(e.lpServiceName),
                    display: from_wide(e.lpDisplayName),
                    pid: e.ServiceStatusProcess.dwProcessId,
                    running: e.ServiceStatusProcess.dwCurrentState == SERVICE_RUNNING,
                });
            }
        }
        CloseServiceHandle(scm);
    }
    out
}

pub const PRIORITIES: &[(&str, u32)] = &[
    ("Realtime", 0x0000_0100),
    ("High", HIGH_PRIORITY_CLASS),
    ("Above normal", ABOVE_NORMAL_PRIORITY_CLASS),
    ("Normal", NORMAL_PRIORITY_CLASS),
    ("Below normal", BELOW_NORMAL_PRIORITY_CLASS),
    ("Low", IDLE_PRIORITY_CLASS),
];

fn with_proc<T>(pid: u32, access: u32, f: impl FnOnce(HANDLE) -> T) -> Option<T> {
    unsafe {
        let h = OpenProcess(access, 0, pid);
        if h.is_null() {
            return None;
        }
        let r = f(h);
        CloseHandle(h);
        Some(r)
    }
}

pub fn set_priority(pid: u32, class: u32) -> bool {
    with_proc(pid, PROCESS_SET_INFORMATION, |h| unsafe { SetPriorityClass(h, class) != 0 }).unwrap_or(false)
}

pub fn set_affinity(pid: u32, half: bool) -> bool {
    with_proc(pid, PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION, |h| unsafe {
        let (mut pm, mut sm) = (0usize, 0usize);
        if GetProcessAffinityMask(h, &mut pm, &mut sm) == 0 {
            return false;
        }
        let cores = sm.count_ones();
        let mask = if half && cores > 1 { first_n_bits(sm, cores / 2) } else { sm };
        SetProcessAffinityMask64(h, mask) != 0
    })
    .unwrap_or(false)
}

fn first_n_bits(mask: usize, n: u32) -> usize {
    let (mut out, mut left) = (0usize, n);
    for b in 0..usize::BITS {
        if left == 0 {
            break;
        }
        if mask >> b & 1 == 1 {
            out |= 1 << b;
            left -= 1;
        }
    }
    out
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "SetProcessAffinityMask"]
    fn SetProcessAffinityMask64(h: HANDLE, mask: usize) -> i32;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn IsUserAnAdmin() -> i32;
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(h: HANDLE) -> i32;
    fn NtResumeProcess(h: HANDLE) -> i32;
}

pub fn suspend(pid: u32, on: bool) -> bool {
    with_proc(pid, 0x0800 , |h| unsafe {
        (if on { NtSuspendProcess(h) } else { NtResumeProcess(h) }) >= 0
    })
    .unwrap_or(false)
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

pub fn run_elevated_task() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("schtasks").args(["/run", "/tn", "rtaskmgr"]).creation_flags(0x0800_0000).status().is_ok_and(|s| s.success())
}

pub fn relaunch_as_admin(args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let (verb, path, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(args));
    unsafe { winapi::um::shellapi::ShellExecuteW(null_mut(), verb.as_ptr(), path.as_ptr(), params.as_ptr(), std::ptr::null(), 1) as usize > 32 }
}

pub fn taskmgr_args(args: &[String]) -> Option<String> {
    let first = args.first()?;
    first.to_lowercase().trim_matches('"').ends_with("taskmgr.exe").then(|| args[1..].join(" "))
}

fn spawn_past_ifeo(exe: &str, cmdline: &str) -> Result<HANDLE, u32> {
    use winapi::um::{debugapi::DebugActiveProcessStop, processthreadsapi::*, winbase::{DEBUG_ONLY_THIS_PROCESS, DebugSetProcessKillOnExit}};
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let mut cmd = wide(cmdline);
        let ok = CreateProcessW(wide(exe).as_ptr(), cmd.as_mut_ptr(), null_mut(), null_mut(), 0, DEBUG_ONLY_THIS_PROCESS, null_mut(), null_mut(), &mut si, &mut pi);
        if ok == 0 {
            return Err(GetLastError());
        }
        DebugSetProcessKillOnExit(0);
        DebugActiveProcessStop(pi.dwProcessId);
        CloseHandle(pi.hThread);
        Ok(pi.hProcess)
    }
}

pub fn start_real_taskmgr(args: &str, elevate: bool) -> bool {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let exe = format!(r"{root}\System32\Taskmgr.exe");
    match spawn_past_ifeo(&exe, &format!("\"{exe}\" {args}")) {
        Ok(h) => {
            unsafe { CloseHandle(h) };
            true
        }
        Err(740) if elevate => relaunch_as_admin(&format!("--real-taskmgr {args}")),
        Err(_) => false,
    }
}

pub fn system_dark() -> bool {
    let (key, val) = (wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), wide("AppsUseLightTheme"));
    let (mut data, mut size) = (1u32, 4u32);
    let rc = unsafe {
        winapi::um::winreg::RegGetValueW(
            winapi::um::winreg::HKEY_CURRENT_USER,
            key.as_ptr(),
            val.as_ptr(),
            0x10,
            null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    rc == 0 && data == 0
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(h: winapi::shared::windef::HWND, attr: u32, v: *const std::ffi::c_void, size: u32) -> i32;
}

pub fn set_dark_titlebar(h: winapi::shared::windef::HWND, dark: bool) {
    let v = dark as i32;
    unsafe { DwmSetWindowAttribute(h, 20 , &v as *const i32 as *const _, 4) };
}

pub fn window_titles() -> HashMap<u32, String> {
    use winapi::shared::{minwindef::{BOOL, LPARAM}, windef::HWND};
    use winapi::um::winuser::{EnumWindows, GetWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, GW_OWNER};
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        unsafe {
            let map = &mut *(l as *mut HashMap<u32, String>);
            if IsWindowVisible(h) == 0 || !GetWindow(h, GW_OWNER).is_null() {
                return 1;
            }
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(h, buf.as_mut_ptr(), 256);
            if n > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, &mut pid);
                map.entry(pid).or_insert_with(|| String::from_utf16_lossy(&buf[..n as usize]));
            }
        }
        1
    }
    let mut map: HashMap<u32, String> = HashMap::new();
    unsafe { EnumWindows(Some(cb), &mut map as *mut _ as LPARAM) };
    map
}

pub fn set_efficiency(pid: u32, on: bool) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetProcessInformation(h: HANDLE, class: i32, info: *const std::ffi::c_void, size: u32) -> i32;
    }
    #[repr(C)]
    struct Throttle {
        version: u32,
        control: u32,
        state: u32,
    }
    with_proc(pid, PROCESS_SET_INFORMATION, |h| unsafe {
        let t = Throttle { version: 1, control: 1 , state: on as u32 };
        let a = SetProcessInformation(h, 4 , &t as *const _ as *const _, std::mem::size_of::<Throttle>() as u32) != 0;
        let b = SetPriorityClass(h, if on { IDLE_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS }) != 0;
        a || b
    })
    .unwrap_or(false)
}

pub fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    unsafe {
        let mut t: winapi::um::minwinbase::SYSTEMTIME = std::mem::zeroed();
        winapi::um::sysinfoapi::GetLocalTime(&mut t);
        (t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taskmgr_args_from_ifeo() {
        let a = |v: &[&str]| taskmgr_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&["C:\\WINDOWS\\system32\\taskmgr.exe", "/0", "/startup"]).as_deref(), Some("/0 /startup"));
        assert_eq!(a(&["\"C:\\Windows\\System32\\Taskmgr.exe\""]).as_deref(), Some(""));
        assert_eq!(a(&["--tray"]), None);
        assert_eq!(a(&[]), None);
    }

    #[test]
    fn debug_spawn_is_released_and_runs_to_its_end() {
        use winapi::um::{processthreadsapi::GetExitCodeProcess, synchapi::WaitForSingleObject};
        let cmd = std::env::var("ComSpec").unwrap();
        let h = spawn_past_ifeo(&cmd, &format!("\"{cmd}\" /c exit 7")).expect("spawn");
        let mut code = 0u32;
        unsafe {
            assert_eq!(WaitForSingleObject(h, 10_000), 0, "a process we did not let go would hang here");
            GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
        }
        assert_eq!(code, 7);
    }
    #[test]
    fn first_bits() {
        assert_eq!(first_n_bits(0b1111_0000, 2), 0b0011_0000);
        assert_eq!(first_n_bits(0b1111, 4), 0b1111);
        assert_eq!(first_n_bits(usize::MAX, 40), (1usize << 40) - 1);
    }
}

pub fn process_user(pid: u32) -> Option<String> {
    use winapi::um::{processthreadsapi::OpenProcessToken, securitybaseapi::GetTokenInformation, winbase::LookupAccountSidW, winnt::{TOKEN_QUERY, TOKEN_USER}};
    with_proc(pid, PROCESS_QUERY_LIMITED_INFORMATION, |h| unsafe {
        let mut tok: HANDLE = null_mut();
        if OpenProcessToken(h, TOKEN_QUERY, &mut tok) == 0 {
            return None;
        }
        let mut buf = [0u64; 64];
        let mut need = 0u32;
        let ok = GetTokenInformation(tok, 1 , buf.as_mut_ptr() as *mut _, size_of_val(&buf) as u32, &mut need) != 0;
        CloseHandle(tok);
        if !ok {
            return None;
        }
        let sid = (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid;
        let (mut nl, mut dl, mut ty) = (256u32, 256u32, 0u32);
        let (mut name, mut dom) = ([0u16; 256], [0u16; 256]);
        if LookupAccountSidW(null_mut(), sid, name.as_mut_ptr(), &mut nl, dom.as_mut_ptr(), &mut dl, &mut ty) == 0 {
            return None;
        }
        let (n, dm) = (String::from_utf16_lossy(&name[..nl as usize]), String::from_utf16_lossy(&dom[..dl as usize]));
        static LOCAL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let local = LOCAL.get_or_init(|| std::env::var("COMPUTERNAME").unwrap_or_default());
        Some(if dm.is_empty() || dm.eq_ignore_ascii_case(local) || dm.eq_ignore_ascii_case("NT AUTHORITY") { n } else { format!("{dm}\\{n}") })
    })
    .flatten()
}

pub type CpuTimes = (i64, i64, i64);

pub fn cpu_times() -> Option<CpuTimes> {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQuerySystemInformation(class: u32, info: *mut std::ffi::c_void, len: u32, ret: *mut u32) -> i32;
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Spi {
        idle: i64,
        kernel: i64,
        user: i64,
        dpc: i64,
        interrupt: i64,
        count: u32,
    }
    let mut buf = vec![Spi::default(); 1024];
    let mut ret = 0u32;
    let st = unsafe { NtQuerySystemInformation(8 , buf.as_mut_ptr() as *mut _, (buf.len() * size_of::<Spi>()) as u32, &mut ret) };
    if st < 0 {
        return None;
    }
    let n = ret as usize / size_of::<Spi>();
    Some(buf[..n].iter().fold((0, 0, 0), |a, c| (a.0 + c.kernel + c.user, a.1 + c.dpc, a.2 + c.interrupt)))
}

pub fn dpc_share(a: CpuTimes, b: CpuTimes) -> (f32, f32) {
    let total = (b.0 - a.0) as f64;
    if total <= 0.0 {
        return (0.0, 0.0);
    }
    (((b.1 - a.1) as f64 / total * 100.0) as f32, ((b.2 - a.2) as f64 / total * 100.0) as f32)
}

pub fn hung_windows() -> Vec<(u32, String)> {
    use winapi::shared::{minwindef::{BOOL, LPARAM}, windef::HWND};
    use winapi::um::winuser::{EnumWindows, GetWindow, GetWindowTextW, GetWindowThreadProcessId, IsHungAppWindow, IsWindowVisible, GW_OWNER};
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        unsafe {
            let v = &mut *(l as *mut Vec<(u32, String)>);
            if IsWindowVisible(h) != 0 && GetWindow(h, GW_OWNER).is_null() && IsHungAppWindow(h) != 0 {
                let mut buf = [0u16; 256];
                let n = GetWindowTextW(h, buf.as_mut_ptr(), 256).max(0) as usize;
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, &mut pid);
                if n > 0 && !v.iter().any(|x| x.0 == pid) {
                    v.push((pid, String::from_utf16_lossy(&buf[..n])));
                }
            }
        }
        1
    }
    let mut v: Vec<(u32, String)> = vec![];
    unsafe { EnumWindows(Some(cb), &mut v as *mut _ as LPARAM) };
    v
}

#[cfg(test)]
mod diag_tests {
    use super::*;

    #[test]
    fn dpc_share_and_times() {
        assert_eq!(dpc_share((0, 0, 0), (1000, 20, 10)), (2.0, 1.0));
        assert_eq!(dpc_share((5, 0, 0), (5, 1, 1)), (0.0, 0.0));
        let t = cpu_times().expect("NtQuerySystemInformation(8)");
        assert!(t.0 > 0 && t.1 >= 0 && t.2 >= 0);
        let _ = hung_windows();
    }
}
