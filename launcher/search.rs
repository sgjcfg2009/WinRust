use crate::*;

pub(crate) const STATE: &str = r"HKLM\SOFTWARE\plugin-launcher\rust-search";
pub(crate) const SB_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Search";

pub(crate) const SH_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\SearchHost.exe";

pub(crate) fn prog_dir() -> PathBuf {
    program_files().join("rs-search")
}

pub(crate) fn journal() -> Result<Journal, String> {
    let j = Journal::open("rust-search", || {
        if run("reg", &["query", STATE]).is_err() && !prog_dir().join("rs.exe").is_file() {
            return vec![];
        }
        let ws = svc_mode(reg_dword(STATE, "WSearchStart"), reg_dword(STATE, "WSearchDelayed")).unwrap_or("delayed-auto");
        let sb = reg_dword(STATE, "SearchboxMode").unwrap_or(2);
        vec![
            Entry::Svc { name: "WSearch".into(), mode: ws.into() },
            Entry::Reg { key: SH_KEY.into(), name: "Debugger".into(), old: None },
            Entry::Reg { key: SB_KEY.into(), name: "SearchboxTaskbarMode".into(), old: Some(("REG_DWORD".into(), sb.to_string())) },
        ]
    })?;
    let _ = run("reg", &["delete", STATE, "/f"]);
    Ok(j)
}

pub(crate) fn has_wsearch() -> bool {
    run("sc.exe", &["query", "WSearch"]).is_ok()
}

pub(crate) fn install() -> Result<(), String> {
    let j = journal()?;
    with_payload(&["rs.exe", "rs-launcher.exe"], |src| {
        run(src.join("rs.exe").to_str().unwrap(), &["--install"])?;
        let dst = prog_dir().join("rs-launcher.exe");
        let _ = run("taskkill", &["/f", "/im", "rs-launcher.exe"]);
        put(&src.join("rs-launcher.exe"), &dst)?;
        if logon_task("rs-launcher", &dst) {
            let _ = run("reg", &["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "rs-launcher", "/f"]);
        } else {
            run(dst.to_str().unwrap(), &["--autostart"])?;
        }
        let _ = winrust_common::launch::program(&dst.to_string_lossy());
        if has_wsearch() {
            j.set_svc("WSearch", "disabled")?;
            let _ = run("sc.exe", &["stop", "WSearch"]);
        } else {
            WARN.lock().unwrap_or_else(|e| e.into_inner()).push("the Windows Search service (WSearch) does not exist on this system: step skipped".into());
        }
        j.set_reg(SH_KEY, "Debugger", "REG_SZ", &format!("\"{}\" --noop", dst.display()))?;
        let _ = run("taskkill", &["/f", "/im", "SearchHost.exe"]);
        j.set_reg(SB_KEY, "SearchboxTaskbarMode", "REG_DWORD", "0")
    })
}

pub(crate) fn uninstall() -> Result<(), String> {
    stop_and_delete_rs_search_tasks();
    stop_and_delete_task("rs-launcher");
    let _ = run("reg", &["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "rs-launcher", "/f"]);
    kill_procs(&["rs-launcher.exe", "rs.exe"]);
    soft("rust-search journal", journal().and_then(|j| j.undo()));
    if reg_str(SH_KEY, "Debugger").is_some_and(|v| {
        let l = v.to_ascii_lowercase();
        l.contains("cmd.exe") || l.contains("rs-launcher.exe")
    }) {
        run("reg", &["delete", SH_KEY, "/v", "Debugger", "/f"])?;
    }
    let k = svc_key("WSearch");
    if svc_mode(reg_dword(&k, "Start"), reg_dword(&k, "DelayedAutostart")).is_some_and(|m| m.ends_with("auto")) {
        let _ = run("sc.exe", &["start", "WSearch"]);
    }
    rm_dir_checked(&prog_dir())?;
    let sys_rs = PathBuf::from(env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
        .join(r"System32\config\systemprofile\AppData\Local\rs-search");
    rm_dir(&sys_rs);
    if let Some(ad) = appdata() {
        rm_dir(&ad.join("rs-search"));
    }
    if let Some(lad) = local_appdata() {
        rm_dir(&lad.join("rs-search"));
    }
    Ok(())
}
