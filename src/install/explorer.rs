use crate::*;
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, RRF_RT_REG_DWORD,
    RegCloseKey, RegFlushKey, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};

mod dword_snapshot;
use dword_snapshot::{Dword, backup_and_set, restore};

pub(crate) fn rx_dir() -> PathBuf {
    program_files().join("rust-explorer")
}

pub(crate) const RX_STATE: &str = r"HKLM\SOFTWARE\plugin-launcher\rust-explorer";
pub(crate) const WINLOGON_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon";
pub(crate) const RX_SWITCH: &str = r"HKCU\Software\rs-shell";
pub(crate) const RX_FOLDERS: [&str; 6] = [
    r"HKCU\Software\Classes\Directory\shell\open",
    r"HKCU\Software\Classes\Directory\shell\explore",
    r"HKCU\Software\Classes\Folder\shell\open",
    r"HKCU\Software\Classes\Folder\shell\explore",
    r"HKCU\Software\Classes\Drive\shell\open",
    r"HKCU\Software\Classes\Drive\shell\explore",
];

const RX_SVCS: [(&str, u32); 5] = [("PcaSvc", 4), ("InventorySvc", 4), ("lfsvc", 4), ("cbdhsvc", 4), ("DoSvc", 3)];
const SPLIT_KEY: &str = r"HKLM\SYSTEM\CurrentControlSet\Control";
const SPLIT_VALUE: &str = "SvcHostSplitThresholdInKB";

fn trim_settings() -> impl Iterator<Item = (&'static str, String, &'static str, u32)> {
    RX_SVCS.into_iter()
        .map(|(service, start)| (service, svc_key(service), "Start", start))
        .chain(std::iter::once((SPLIT_VALUE, SPLIT_KEY.into(), SPLIT_VALUE, 0x4000000)))
}

fn trim_subkey(key: &str) -> Vec<u16> {
    wide(key.strip_prefix(r"HKLM\").expect("service settings live under HKLM"))
}

fn trim_registry_error(key: &str, name: &str, code: u32) -> String {
    format!(r"{key}\{name}: {}", std::io::Error::from_raw_os_error(code as i32))
}

fn trim_read((key, name): Dword<'_>) -> Result<Option<u32>, String> {
    let subkey = trim_subkey(key);
    let value_name = wide(name);
    let mut value = 0u32;
    let mut len = size_of::<u32>() as u32;
    let code = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, subkey.as_ptr(), value_name.as_ptr(), RRF_RT_REG_DWORD,
            null_mut(), (&mut value as *mut u32).cast(), &mut len)
    };
    match code {
        ERROR_SUCCESS if len == size_of::<u32>() as u32 => Ok(Some(value)),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
        ERROR_SUCCESS => Err(trim_registry_error(key, name, ERROR_INVALID_DATA)),
        _ => Err(trim_registry_error(key, name, code)),
    }
}

fn trim_write((key, name): Dword<'_>, value: u32) -> Result<(), String> {
    let subkey = trim_subkey(key);
    let value_name = wide(name);
    let mut handle = null_mut();
    let code = unsafe {
        RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey.as_ptr(), 0, KEY_QUERY_VALUE | KEY_SET_VALUE, &mut handle)
    };
    if code != ERROR_SUCCESS {
        return Err(trim_registry_error(key, name, code));
    }
    let bytes = value.to_le_bytes();
    let mut code = unsafe { RegSetValueExW(handle, value_name.as_ptr(), 0, REG_DWORD, bytes.as_ptr(), bytes.len() as u32) };
    if code == ERROR_SUCCESS && key == RX_STATE {
        code = unsafe { RegFlushKey(handle) };
    }
    unsafe { RegCloseKey(handle) };
    if code != ERROR_SUCCESS {
        return Err(trim_registry_error(key, name, code));
    }
    if trim_read((key, name))? != Some(value) {
        return Err(format!("{key}: could not verify the write to {name}"));
    }
    Ok(())
}

const NO_LONGER_TRIMMED: [&str; 2] = ["cbdhsvc", "PcaSvc"];

fn apply_service_trim() {
    for (backup, key, value, new) in trim_settings().filter(|s| !NO_LONGER_TRIMMED.contains(&s.0)) {
        let result = backup_and_set((&key, value), (RX_STATE, backup), new, trim_read, trim_write);
        if matches!(&result, Ok(true)) && value == "Start" && new == 4 {
            let _ = run("sc.exe", &["stop", backup]);
        }
        soft(backup, result.map(|_| ()));
    }
}

fn restore_service_trim() -> Result<(), String> {
    let mut restored = false;
    for (backup, key, value, _) in trim_settings() {
        match restore((&key, value), (RX_STATE, backup), trim_read, trim_write) {
            Ok(saved) => restored |= saved,
            Err(e) => soft(backup, Err(e)),
        }
    }
    if !restored && rx_dir().join("rust-explorer.exe").is_file() {
        WARN.lock().unwrap().push("no service settings were backed up; the current values were left in place".into());
    }
    Ok(())
}

pub(crate) fn rx_folders_bak(i: usize) -> PathBuf {
    rx_dir().join(format!("folders-{i}.reg"))
}

pub(crate) fn rx_lnk() -> PathBuf {
    program_data().join(r"plugin-launcher\rs-overlay.lnk")
}

fn save_windows_desktop_icons() {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory},
            Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx},
            Threading::{OpenProcess, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE},
        },
        UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowThreadProcessId, SendMessageW},
    };
    let Ok(appdata) = env::var("APPDATA") else { return };
    let dst = PathBuf::from(appdata).join(r"rust-explorer\desktop-icons.txt");
    if dst.is_file() {
        return;
    }
    unsafe {
        let pm = FindWindowW(wide("Progman").as_ptr(), wide("Program Manager").as_ptr());
        let mut dv = if !pm.is_null() { FindWindowExW(pm, null_mut(), wide("SHELLDLL_DefView").as_ptr(), null()) } else { null_mut() };
        if dv.is_null() {
            let mut ww = null_mut();
            loop {
                ww = FindWindowExW(null_mut(), ww, wide("WorkerW").as_ptr(), null());
                if ww.is_null() { break; }
                dv = FindWindowExW(ww, null_mut(), wide("SHELLDLL_DefView").as_ptr(), null());
                if !dv.is_null() { break; }
            }
        }
        if dv.is_null() { return; }
        let lv = FindWindowExW(dv, null_mut(), wide("SysListView32").as_ptr(), null());
        if lv.is_null() { return; }
        let n = SendMessageW(lv, 0x1004 , 0, 0) as i32;
        if n <= 0 { return; }
        let mut pid = 0u32;
        GetWindowThreadProcessId(lv, &mut pid);
        let proc = OpenProcess(PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE, 0, pid);
        if proc.is_null() { return; }
        let mem = VirtualAllocEx(proc, null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if !mem.is_null() {
            let mut out = String::new();
            let mut pt = [0i32; 2];
            let mut lvi = [0u8; 88];
            let mut txt = [0u16; 260];
            let mut rw = 0usize;
            for i in 0..n {
                SendMessageW(lv, 0x1010 , i as usize, mem as isize);
                ReadProcessMemory(proc, mem, pt.as_mut_ptr().cast(), 8, &mut rw);
                lvi.fill(0);
                lvi[0..4].copy_from_slice(&1u32.to_le_bytes());
                lvi[4..8].copy_from_slice(&(i as u32).to_le_bytes());
                lvi[24..32].copy_from_slice(&((mem as usize + 128) as u64).to_le_bytes());
                lvi[32..36].copy_from_slice(&250u32.to_le_bytes());
                WriteProcessMemory(proc, mem, lvi.as_ptr().cast(), lvi.len(), &mut rw);
                let ch = (SendMessageW(lv, 0x1073 , i as usize, mem as isize) as usize).min(250);
                if ch > 0 && ReadProcessMemory(proc, (mem as usize + 128) as *const _, txt.as_mut_ptr().cast(), ch * 2, &mut rw) != 0 {
                    let name = String::from_utf16_lossy(&txt[..ch]);
                    if !name.is_empty() {
                        out.push_str(&format!("{}\t{}\t{}\n", pt[0], pt[1], name));
                    }
                }
            }
            if !out.is_empty() {
                if let Some(p) = dst.parent() { let _ = fs::create_dir_all(p); }
                let _ = fs::write(&dst, out);
            }
            VirtualFreeEx(proc, mem, 0, MEM_RELEASE);
        }
        CloseHandle(proc);
    }
}

pub(crate) fn rx_install() -> Result<(), String> {
    override_windhawk();
    save_windows_desktop_icons();
    let fresh = !rx_dir().join("rust-explorer.exe").is_file()
        && trim_read((RX_STATE, "AutoRestartShell"))?.is_none();
    kill_procs(&["rs-shell.exe", "rust-explorer.exe"]);
    let (fm, sh) = (rx_dir().join("rust-explorer.exe"), rx_dir().join("rs-shell.exe"));
    write_payload("rust-explorer.exe", &fm)?;
    write_payload("rs-shell.exe", &sh)?;
    if reg_dword(RX_STATE, "AutoRestartShell").is_none() {
        let v = reg_dword(WINLOGON_KEY, "AutoRestartShell").unwrap_or(1);
        run("reg", &["add", RX_STATE, "/v", "AutoRestartShell", "/t", "REG_DWORD", "/d", &v.to_string(), "/f"])?;
    }
    for (i, k) in RX_FOLDERS.iter().enumerate() {
        let (bak, ours) = (rx_folders_bak(i), out("reg", &["query", &format!(r"{k}\command"), "/ve"]).is_ok_and(|c| c.to_lowercase().contains("rust-explorer")));
        if !bak.is_file() && !ours && run("reg", &["query", k]).is_ok() {
            run("reg", &["export", k, &bak.display().to_string(), "/y"])?;
        }
    }
    for k in RX_FOLDERS {
        let c = format!(r"{k}\command");
        run("reg", &["add", &c, "/ve", "/t", "REG_SZ", "/d", &format!("\"{}\" \"%1\"", fm.display()), "/f"])?;
        run("reg", &["add", &c, "/v", "DelegateExecute", "/t", "REG_SZ", "/d", "", "/f"])?;
    }
    let task_ok = register_logon_task("rs-overlay", &sh, "--overlay", true).is_ok();
    if task_ok {
        let _ = run("reg", &["delete", RUN_KEY, "/v", "rs-overlay", "/f"]);
    } else {
        WARN.lock().unwrap().push("rs-overlay task was not created: it starts via Run, with a Windows startup delay".into());
        run("reg", &["add", RUN_KEY, "/v", "rs-overlay", "/t", "REG_SZ", "/d", &format!("\"{}\" --overlay", sh.display()), "/f"])?;
    }
    run("reg", &["add", RX_SWITCH, "/v", "ReplaceExplorer", "/t", "REG_DWORD", "/d", "1", "/f"])?;
    run("reg", &["add", WINLOGON_KEY, "/v", "AutoRestartShell", "/t", "REG_DWORD", "/d", "0", "/f"])?;
    if fresh {
        apply_service_trim();
    }
    soft("Start menu shortcut 'Rust Explorer' was not created", shortcut(&start_lnk("Rust Explorer"), &fm, ""));
    let _ = run("ie4uinit.exe", &["-show"]);
    let lnk = rx_lnk();
    let _ = fs::create_dir_all(lnk.parent().unwrap());
    soft("overlay is not running now: it starts at the next logon", shortcut(&lnk, &sh, "--overlay"));
    if !(task_ok && run("schtasks", &["/run", "/tn", "rs-overlay"]).is_ok()) {
        let _ = Command::new("explorer.exe").arg(&lnk).spawn();
    }
    Ok(())
}

pub(crate) fn rx_uninstall() -> Result<(), String> {
    let _ = run("reg", &["delete", RUN_KEY, "/v", "rs-overlay", "/f"]);
    stop_and_delete_task("rs-overlay");
    let _ = run("reg", &["delete", RX_SWITCH, "/f"]);
    kill_procs(&["rs-shell.exe"]);
    kill_procs(&["rust-explorer.exe"]);
    soft("service trim", restore_service_trim());
    let back = reg_dword(RX_STATE, "AutoRestartShell").unwrap_or(1);
    if reg_dword(WINLOGON_KEY, "AutoRestartShell") != Some(back) {
        soft("AutoRestartShell", run("reg", &["add", WINLOGON_KEY, "/v", "AutoRestartShell", "/t", "REG_DWORD", "/d", &back.to_string(), "/f"]));
    }
    for (i, k) in RX_FOLDERS.iter().enumerate() {
        let _ = run("reg", &["delete", k, "/f"]);
        let bak = rx_folders_bak(i);
        if bak.is_file() {
            soft(&format!("folder verbs {k} were not restored"), run("reg", &["import", &bak.display().to_string()]));
        }
    }
    if winrust_common::proc::shell_explorers().is_empty() {
        if !winrust_common::launch::program(r"C:\Windows\explorer.exe") {
            let _ = winrust_common::proc::start_shell_explorer();
        }
    }
    rm_file(&rx_lnk());
    rm_file(&start_lnk("Rust Explorer"));
    rm_dir_checked(&rx_dir())?;
    if let Some(ad) = appdata() {
        rm_dir(&ad.join("rust-explorer"));
        for f in ["desktop-icons-hidden.txt", "small-icons", "icon-size"] {
            rm_file(&ad.join("WinRust").join(f));
        }
    }
    if let Some(lad) = local_appdata() {
        rm_dir(&lad.join("rs-shell"));
        rm_dir(&lad.join("rust-explorer"));
    }
    let _ = run("reg", &["delete", RX_STATE, "/f"]);
    Ok(())
}
