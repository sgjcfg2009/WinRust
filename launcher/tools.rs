use crate::*;

pub(crate) fn pv_dir() -> PathBuf {
    program_files().join("photoview")
}

pub(crate) fn st_dir() -> PathBuf {
    program_files().join("rust-settings")
}

pub(crate) fn pu_dir() -> PathBuf {
    program_files().join("purge")
}

pub(crate) fn db_dir() -> PathBuf {
    program_files().join("debloat")
}

pub(crate) fn tool_install(exe: &str, dir: &Path, lnk: Option<&str>) -> Result<(), String> {
    kill_procs(&[exe]);
    let dst = dir.join(exe);
    write_payload(exe, &dst)?;
    if let Some(n) = lnk {
        soft(&format!("Start menu shortcut '{n}' was not created, search will not find it"), shortcut(&start_lnk(n), &dst, ""));
    }
    Ok(())
}

pub(crate) fn tool_remove(exe: &str, dir: &Path, lnk: Option<&str>) -> Result<(), String> {
    if let Some(n) = lnk {
        rm_file(&start_lnk(n));
    }
    kill_procs(&[exe]);
    rm_dir_checked(dir).map(|_| ())
}

pub(crate) fn st_install() -> Result<(), String> {
    tool_install("rust-settings.exe", &st_dir(), Some("WinRust Settings"))
}

pub(crate) fn st_uninstall() -> Result<(), String> {
    tool_remove("rust-settings.exe", &st_dir(), Some("WinRust Settings"))?;
    let _ = run("reg", &["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "WinRustNightLight", "/f"]);
    let _ = run("reg", &["delete", r"HKCU\Software\WinRust\NightLight", "/f"]);
    Ok(())
}

pub(crate) const PV_EXTS: [&str; 11] = ["jpg", "jpeg", "jfif", "png", "bmp", "gif", "tif", "tiff", "webp", "heic", "avif"];
pub(crate) const PV_PROG: &str = "WinRustPhoto";
pub(crate) const PV_APP: &str = "WinRust Photo Viewer";

pub(crate) fn pv_install() -> Result<(), String> {
    tool_install("photoview.exe", &pv_dir(), Some("Photo viewer"))?;
    let exe = pv_dir().join("photoview.exe").display().to_string().replace('\\', r"\\");
    let mut reg = format!(
        "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\Classes\\{PV_PROG}]\r\n@=\"Picture\"\r\n\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\Classes\\{PV_PROG}\\shell\\open\\command]\r\n@=\"\\\"{exe}\\\" \\\"%1\\\"\"\r\n\r\n\
         [HKEY_LOCAL_MACHINE\\SOFTWARE\\WinRust\\PhotoView\\Capabilities]\r\n\"ApplicationName\"=\"{PV_APP}\"\r\n\"ApplicationDescription\"=\"Fast picture viewer\"\r\n\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\WinRust\\PhotoView\\Capabilities\\FileAssociations]\r\n"
    );
    for e in PV_EXTS {
        reg += &format!("\".{e}\"=\"{PV_PROG}\"\r\n");
    }
    reg += &format!("\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\RegisteredApplications]\r\n\"{PV_APP}\"=\"SOFTWARE\\\\WinRust\\\\PhotoView\\\\Capabilities\"\r\n");
    for e in PV_EXTS {
        reg += &format!("\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\Classes\\.{e}\\OpenWithProgids]\r\n\"{PV_PROG}\"=hex(0):\r\n");
    }
    let f = pv_dir().join("register.reg");
    fs::write(&f, reg).map_err(|e| format!("{}: {e}", f.display()))?;
    let r = run("reg", &["import", &f.display().to_string()]);
    let _ = fs::remove_file(&f);
    r
}

pub(crate) fn pv_uninstall() -> Result<(), String> {
    for e in PV_EXTS {
        let _ = run("reg", &["delete", &format!(r"HKLM\SOFTWARE\Classes\.{e}\OpenWithProgids"), "/v", PV_PROG, "/f"]);
    }
    let _ = run("reg", &["delete", r"HKLM\SOFTWARE\RegisteredApplications", "/v", PV_APP, "/f"]);
    let _ = run("reg", &["delete", &format!(r"HKLM\SOFTWARE\Classes\{PV_PROG}"), "/f"]);
    let _ = run("reg", &["delete", r"HKLM\SOFTWARE\WinRust\PhotoView", "/f"]);
    tool_remove("photoview.exe", &pv_dir(), Some("Photo viewer"))
}

pub(crate) fn db_install() -> Result<(), String> {
    tool_install("debloat.exe", &db_dir(), None)?;
    out(&db_dir().join("debloat.exe").display().to_string(), &["--apply"]).map(|_| ())
}

pub(crate) fn db_uninstall() -> Result<(), String> {
    let exe = db_dir().join("debloat.exe");
    if exe.is_file() {
        soft("debloat --undo", out(&exe.display().to_string(), &["--undo"]).map(|_| ()));
    }
    tool_remove("debloat.exe", &db_dir(), None)
}

pub(crate) fn pu_install() -> Result<(), String> {
    tool_install("purge.exe", &pu_dir(), None)?;
    out(&pu_dir().join("purge.exe").display().to_string(), &["--apply"]).map(|_| ())
}

pub(crate) fn pu_uninstall() -> Result<(), String> {
    tool_remove("purge.exe", &pu_dir(), None)
}
