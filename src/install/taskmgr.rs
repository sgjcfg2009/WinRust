use crate::*;

pub(crate) const TM_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe";

pub(crate) const TM_STATE: &str = r"HKLM\SOFTWARE\plugin-launcher\rtaskmgr-win32";

pub(crate) fn tm_installed() -> bool {
    reg_str(TM_KEY, "Debugger").is_some_and(|d| d.to_lowercase().contains("rtaskmgr"))
}

pub(crate) fn tm_dir() -> PathBuf {
    program_files().join("rtaskmgr")
}

pub(crate) fn tm_install() -> Result<(), String> {
    if let Some(old) = reg_str(TM_KEY, "Debugger").filter(|_| !tm_installed() && reg_str(TM_STATE, "Debugger").is_none()) {
        run("reg", &["add", TM_STATE, "/v", "Debugger", "/t", "REG_SZ", "/d", &old, "/f"])?;
    }
    let dst = tm_dir().join("rtaskmgr-win32.exe");
    kill_procs(&["rtaskmgr-win32.exe"]);
    write_payload("Task manager.exe", &dst)?;
    write_payload("debloat.exe", &tm_dir().join("debloat.exe"))?;
    soft("Start menu shortcut was not created, search will not find the task manager", shortcut(&start_lnk("Task manager"), &dst, ""));
    soft("rtaskmgr task was not created, launching as administrator will go through UAC", register_manual_task("rtaskmgr", &dst, true));
    run("reg", &["add", TM_KEY, "/v", "Debugger", "/t", "REG_SZ", "/d", &format!("\"{}\"", dst.display()), "/f"])
}

pub(crate) fn tm_uninstall() -> Result<(), String> {
    stop_and_delete_task("rtaskmgr");
    let exe = tm_dir().join("rtaskmgr-win32.exe");
    if exe.is_file() {
        kill_procs(&["rtaskmgr-win32.exe"]);
        soft("undo-speed", out(&exe.display().to_string(), &["--undo-speed"]).map(|_| ()));
    }
    kill_procs(&["rtaskmgr-win32.exe", "debloat.exe"]);
    rm_file(&start_lnk("Task manager"));
    match reg_str(TM_STATE, "Debugger") {
        Some(old) => run("reg", &["add", TM_KEY, "/v", "Debugger", "/t", "REG_SZ", "/d", &old, "/f"])?,
        None if tm_installed() => run("reg", &["delete", TM_KEY, "/v", "Debugger", "/f"])?,
        None => {}
    }
    let _ = run("reg", &["delete", TM_STATE, "/f"]);
    rm_dir_checked(&tm_dir())?;
    if let Some(ad) = appdata() {
        rm_dir(&ad.join("rtaskmgr"));
    }
    if let Some(lad) = local_appdata() {
        rm_dir(&lad.join("rtaskmgr"));
    }
    Ok(())
}
