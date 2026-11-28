use std::{ffi::c_void, path::PathBuf, sync::OnceLock, thread, time::Duration};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LUID},
    Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES},
    System::{
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtSetSystemInformation(class: u32, info: *const c_void, len: u32) -> i32;
}

const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
const MEMORY_PURGE_STANDBY_LIST: u32 = 4;
const PAGE: u64 = 4096;

#[repr(C)]
#[derive(Default)]
struct MemoryLists {
    zero: usize,
    free: usize,
    modified: usize,
    modified_no_write: usize,
    bad: usize,
    standby_by_priority: [usize; 8],
    repurposed_by_priority: [usize; 8],
    modified_pagefile: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Lists {
    free: u64,
    standby: u64,
}

fn enable_privilege(name: &str) -> bool {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut tok) == 0 {
            return false;
        }
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        let mut ok = crate::with_wide(name, |w| LookupPrivilegeValueW(std::ptr::null(), w, &mut luid)) != 0;
        if ok {
            let tp = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [windows_sys::Win32::Security::LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }] };
            ok = AdjustTokenPrivileges(tok, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0 && windows_sys::Win32::Foundation::GetLastError() == 0;
        }
        CloseHandle(tok);
        ok
    }
}

fn ensure_profile_privilege() -> bool {
    static PRIV_OK: OnceLock<bool> = OnceLock::new();
    *PRIV_OK.get_or_init(|| enable_privilege("SeProfileSingleProcessPrivilege"))
}

fn lists() -> Option<Lists> {
    if !ensure_profile_privilege() {
        return None;
    }
    let mut m = MemoryLists::default();
    let st = unsafe { NtQuerySystemInformation(SYSTEM_MEMORY_LIST_INFORMATION, &mut m as *mut _ as *mut c_void, size_of::<MemoryLists>() as u32, std::ptr::null_mut()) };
    (st >= 0).then(|| Lists { free: (m.zero + m.free) as u64 * PAGE, standby: m.standby_by_priority.iter().sum::<usize>() as u64 * PAGE })
}

fn purge_standby() -> Result<(), i32> {
    if !ensure_profile_privilege() {
        return Err(0xC000_0061u32 as i32);
    }
    let cmd = MEMORY_PURGE_STANDBY_LIST;
    let st = unsafe { NtSetSystemInformation(SYSTEM_MEMORY_LIST_INFORMATION, &cmd as *const u32 as *const c_void, 4) };
    if st >= 0 { Ok(()) } else { Err(st) }
}

fn total_ram() -> u64 {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    unsafe { GlobalMemoryStatusEx(&mut m) };
    m.ullTotalPhys
}

fn should_purge(l: Lists, total: u64) -> bool {
    l.free < (512 << 20).max(total / 20) && l.standby > 1 << 30
}

fn auto_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join(r"WinRust\autopurge.txt"))
}

fn auto_on() -> bool {
    static AUTO_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
    AUTO_FILE.get_or_init(auto_file).as_ref().is_some_and(|f| f.is_file())
}

pub fn auto_loop() {
    use windows_sys::Win32::{
        Storage::FileSystem::{FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME},
        System::{
            Memory::{CreateMemoryResourceNotification, LowMemoryResourceNotification, QueryMemoryResourceNotification},
            Threading::{INFINITE, WaitForMultipleObjects, WaitForSingleObject},
        },
    };
    let total = total_ram();
    let mut last: Option<std::time::Instant> = None;
    let mem_ev = unsafe { CreateMemoryResourceNotification(LowMemoryResourceNotification) };
    let dir_ev = auto_file()
        .and_then(|f| f.parent().map(|p| p.to_path_buf()))
        .map(|d| {
            let _ = std::fs::create_dir_all(&d);
            crate::with_wide(&d.to_string_lossy(), |w| unsafe { FindFirstChangeNotificationW(w, 0, FILE_NOTIFY_CHANGE_FILE_NAME) })
        })
        .unwrap_or(windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE);
    if mem_ev.is_null() {
        loop {
            thread::sleep(Duration::from_secs(30));
            if last.is_some_and(|t| t.elapsed() < Duration::from_secs(180)) || !auto_on() {
                continue;
            }
            if lists().is_some_and(|l| should_purge(l, total)) && purge_standby().is_ok() {
                last = Some(std::time::Instant::now());
            }
        }
    }
    loop {
        if !auto_on() {
            if dir_ev != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                unsafe {
                    WaitForSingleObject(dir_ev, INFINITE);
                    FindNextChangeNotification(dir_ev);
                }
            } else {
                thread::sleep(Duration::from_secs(60));
            }
            continue;
        }
        let mut low = 0;
        let signaled = unsafe { QueryMemoryResourceNotification(mem_ev, &mut low) != 0 && low != 0 };
        if signaled && last.is_none_or(|t| t.elapsed() >= Duration::from_secs(180)) {
            if lists().is_some_and(|l| should_purge(l, total)) && purge_standby().is_ok() {
                last = Some(std::time::Instant::now());
            }
            thread::sleep(Duration::from_secs(180));
            continue;
        }
        if dir_ev != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            let handles = [mem_ev, dir_ev];
            let r = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            if r == 1 {
                unsafe { FindNextChangeNotification(dir_ev) };
            }
        } else {
            unsafe { WaitForSingleObject(mem_ev, INFINITE) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purge_rule() {
        let gb = 1u64 << 30;
        assert!(should_purge(Lists { free: 100 << 20, standby: 4 * gb }, 16 * gb));
        assert!(!should_purge(Lists { free: 2 * gb, standby: 4 * gb }, 16 * gb), "enough free memory");
        assert!(!should_purge(Lists { free: 100 << 20, standby: 500 << 20 }, 16 * gb), "nothing worth dropping");
        assert!(should_purge(Lists { free: 1 * gb, standby: 8 * gb }, 64 * gb), "5% of 64 GB is 3.2 GB");
    }

    #[test]
    fn struct_matches_the_kernel_layout() {
        assert_eq!(size_of::<MemoryLists>(), 22 * size_of::<usize>());
        assert!(total_ram() > 0);
    }
}
