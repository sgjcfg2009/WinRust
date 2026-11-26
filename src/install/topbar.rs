use crate::*;

pub(crate) fn bar_dir() -> PathBuf {
    program_files().join("topbar")
}

pub(crate) const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

pub(crate) fn cfg_path() -> PathBuf {
    bar_dir().join("topbar.cfg")
}

pub(crate) fn od_hidden() -> bool {
    fs::read_to_string(cfg_path()).map_or(true, |c| !c.contains("hide_onedrive=0"))
}

pub(crate) fn set_od_hidden(v: bool) -> Result<(), String> {
    fs::create_dir_all(bar_dir()).and_then(|_| fs::write(cfg_path(), format!("hide_onedrive={}\n", v as u8))).map_err(|e| e.to_string())
}

pub(crate) fn ps_strip() -> String {
    format!(r"{} $t=[string]$(if(Test-Path $p){{Get-Content $p -Raw}}); $t=[regex]::Replace($t,'(?s)# >>> topbar >>>.*?# <<< topbar <<<\r?\n?','');", profile_path())
}
pub(crate) const PS_BLOCK: &str = include_str!("profile.ps1");
pub(crate) const SHELLS: [&str; 2] = ["powershell", "pwsh"];

pub(crate) fn profile_install() -> Result<(), String> {
    shells_run(&format!("{}$b=@'\n{PS_BLOCK}'@; New-Item -Force -ItemType Directory (Split-Path $p) | Out-Null; Set-Content $p ($t.TrimEnd() + [Environment]::NewLine + $b) -Encoding utf8", ps_strip()))
}

pub(crate) fn profile_uninstall() -> Result<(), String> {
    shells_run(&format!("{} if($t.Trim()){{Set-Content $p $t -Encoding utf8}}elseif(Test-Path $p){{Remove-Item $p}}", ps_strip()))
}

pub(crate) fn shells_run(script: &str) -> Result<(), String> {
    let r = run(SHELLS[0], &["-NoProfile", "-Command", script]);
    let _ = run(SHELLS[1], &["-NoProfile", "-Command", script]);
    r
}

pub(crate) fn bar_install() -> Result<(), String> {
    override_windhawk();
    let dst = bar_dir().join("topbar.exe");
    let rsh = bar_dir().join("reshell.exe");
    kill_procs(&["topbar.exe", "reshell.exe"]);
    write_payload("topbar.exe", &dst)?;
    write_payload("reshell.exe", &rsh)?;
    if logon_task("reshell", &rsh) {
        let _ = run("reg", &["delete", RUN_KEY, "/v", "reshell", "/f"]);
    } else {
        run("reg", &["add", RUN_KEY, "/v", "reshell", "/t", "REG_SZ", "/d", &format!("\"{}\"", rsh.display()), "/f"])?;
    }
    let _ = winrust_common::launch::program(&rsh.to_string_lossy());
    run(dst.to_str().unwrap(), &["--restore"])?;
    if register_logon_task("topbar", &dst, "", true).is_ok() && run("schtasks", &["/run", "/tn", "topbar"]).is_ok() {
        let _ = run("reg", &["delete", RUN_KEY, "/v", "topbar", "/f"]);
    } else {
        WARN.lock().unwrap().push("topbar task was not created: it starts via Run, without administrator rights".into());
        run("reg", &["add", RUN_KEY, "/v", "topbar", "/t", "REG_SZ", "/d", &format!("\"{}\"", dst.display()), "/f"])?;
        let _ = winrust_common::launch::program(&dst.to_string_lossy());
    }
    soft("PowerShell profile was not written", profile_install());
    Ok(())
}

pub(crate) fn bar_uninstall() -> Result<(), String> {
    let _ = run("reg", &["delete", RUN_KEY, "/v", "topbar", "/f"]);
    let _ = run("reg", &["delete", RUN_KEY, "/v", "reshell", "/f"]);
    stop_and_delete_task("topbar");
    stop_and_delete_task("reshell");
    kill_procs(&["reshell.exe"]);
    kill_procs(&["topbar.exe"]);
    restore_windows_taskbar();
    soft("PowerShell profile", profile_uninstall());
    rm_dir_checked(&bar_dir())?;
    if let Some(ad) = appdata() {
        for f in [
            "topbar-usage.txt",
            "topbar-tray.txt",
            "topbar-start.txt",
            "topbar-pins.txt",
            "topbar-pin-order.txt",
            "topbar-explorer-pin.txt",
            "topbar-taskbar-state.txt",
        ] {
            rm_file(&ad.join(f));
        }
        rm_dir(&ad.join("topbar-taskband"));
    }
    Ok(())
}
