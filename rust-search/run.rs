use std::path::PathBuf;

const HISTORY_MAX: usize = 50;

pub fn split_cmd(line: &str, exists: impl Fn(&str) -> bool) -> (String, String) {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        let (file, args) = rest.split_once('"').unwrap_or((rest, ""));
        return (file.to_string(), args.trim().to_string());
    }
    let scheme = line.split_once(':').is_some_and(|(s, _)| s.len() > 1 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-+.".contains(c)));
    if scheme || exists(line) {
        return (line.to_string(), String::new());
    }
    let (file, args) = line.split_once(' ').unwrap_or((line, ""));
    (file.to_string(), args.trim().to_string())
}

pub fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) if b > 0 => {
                if let Ok(val) = std::env::var(&after[..b]) {
                    out.push_str(&val);
                    rest = &after[b + 1..];
                } else {
                    out.push('%');
                    rest = after;
                }
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out + rest
}

fn history_file() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("WinRust").join("run-history.txt"))
}

pub fn history() -> Vec<String> {
    history_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

pub fn remember_in(list: &mut Vec<String>, line: &str) {
    let line = line.trim();
    list.retain(|l| !l.eq_ignore_ascii_case(line));
    list.insert(0, line.to_string());
    list.truncate(HISTORY_MAX);
}

pub fn remember(line: &str) {
    let mut h = history();
    remember_in(&mut h, line);
    if let Some(f) = history_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, h.join("\n"));
    }
}

pub const KNOWN: &[(&str, &str)] = &[
    ("cmd", "Command Prompt"),
    ("powershell", "Windows PowerShell"),
    ("regedit", "Registry Editor"),
    ("taskmgr", "Task Manager"),
    ("control", "Control Panel"),
    ("msconfig", "System Configuration, boot"),
    ("services.msc", "Services"),
    ("devmgmt.msc", "Device Manager"),
    ("wt", "Windows Terminal"),
    ("notepad", "Notepad"),
    ("calc", "Calculator"),
    ("mspaint", "Paint"),
    ("diskmgmt.msc", "Disk Management, partitions"),
    ("compmgmt.msc", "Computer Management"),
    ("eventvwr.msc", "Event Viewer, logs"),
    ("gpedit.msc", "Local Group Policy Editor"),
    ("taskschd.msc", "Task Scheduler"),
    ("lusrmgr.msc", "Local Users and Groups"),
    ("perfmon", "Performance Monitor"),
    ("resmon", "Resource Monitor"),
    ("appwiz.cpl", "Programs and Features, uninstall"),
    ("optionalfeatures", "Windows Features"),
    ("ncpa.cpl", "Network Connections, adapters"),
    ("sysdm.cpl", "System Properties, computer name, environment variables"),
    ("powercfg.cpl", "Power Options"),
    ("mmsys.cpl", "Sound, playback devices"),
    ("main.cpl", "Mouse"),
    ("desk.cpl", "Display, resolution"),
    ("timedate.cpl", "Date and Time"),
    ("inetcpl.cpl", "Internet Options, proxy"),
    ("firewall.cpl", "Windows Defender Firewall"),
    ("cleanmgr", "Disk Cleanup"),
    ("dfrgui", "Optimize Drives, defragment"),
    ("msinfo32", "System Information"),
    ("dxdiag", "DirectX Diagnostic Tool, graphics"),
    ("winver", "About Windows, version"),
    ("mstsc", "Remote Desktop Connection"),
    ("osk", "On-Screen Keyboard"),
    ("magnify", "Magnifier"),
    ("charmap", "Character Map"),
    ("snippingtool", "Snipping Tool, screenshot"),
    ("mrt", "Malicious Software Removal Tool"),
    ("%temp%", "Temporary files"),
    ("%appdata%", "AppData folder (Roaming)"),
    ("%localappdata%", "AppData folder (Local)"),
    ("shell:startup", "Startup folder"),
    ("shell:downloads", "Downloads"),
    ("shell:appsfolder", "All apps"),
    ("ms-settings:", "Windows Settings"),
    ("ms-settings:windowsupdate", "Windows Update"),
    ("ms-settings:display", "Settings: display"),
    ("ms-settings:bluetooth", "Settings: Bluetooth and devices"),
    ("ms-settings:network", "Settings: network and internet"),
    ("ms-settings:appsfeatures", "Settings: installed apps"),
    ("ms-settings:personalization", "Settings: personalization"),
    ("ms-settings:sound", "Settings: sound"),
];
const POPULAR: usize = 8;

pub fn describe(line: &str) -> Option<usize> {
    KNOWN.iter().position(|k| k.0.eq_ignore_ascii_case(line.trim()))
}

pub fn programs() -> Vec<String> {
    let mut v: Vec<String> = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .chain(app_paths())
        .filter_map(|n| (n.len() > 4 && n[n.len() - 4..].eq_ignore_ascii_case(".exe")).then(|| n[..n.len() - 4].to_string()))
        .collect();
    v.sort_by_cached_key(|s| s.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

fn app_paths() -> Vec<String> {
    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW};
    let sub: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\0".encode_utf16().collect();
    let mut out = vec![];
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut k: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(root, sub.as_ptr(), 0, KEY_READ, &mut k) } != 0 {
            continue;
        }
        for i in 0.. {
            let (mut buf, mut n) = ([0u16; 260], 260u32);
            if unsafe { RegEnumKeyExW(k, i, buf.as_mut_ptr(), &mut n, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..n as usize]));
        }
        unsafe { RegCloseKey(k) };
    }
    out
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Src {
    Typed,
    History,
    Folder,
    Known(usize),
    Program,
}

pub fn suggest(text: &str, history: &[String], programs: &[String]) -> Vec<(String, Src)> {
    let t = text.trim();
    let add = |s: &str, src: Src, out: &mut Vec<(String, Src)>| {
        if out.len() < 30 && !out.iter().any(|o| o.0.eq_ignore_ascii_case(s)) {
            out.push((s.to_string(), src));
        }
    };
    if t.is_empty() {
        let mut out = vec![];
        for h in history.iter().take(20) {
            add(h, Src::History, &mut out);
        }
        for (i, k) in KNOWN.iter().enumerate().take(POPULAR) {
            if out.len() < 7 {
                add(k.0, Src::Known(i), &mut out);
            }
        }
        return out;
    }
    let low = t.to_lowercase();
    let mut out = vec![(t.to_string(), Src::Typed)];
    for h in history.iter().filter(|h| h.to_lowercase().starts_with(&low)) {
        add(h, Src::History, &mut out);
    }
    for (i, k) in KNOWN.iter().enumerate().filter(|(_, k)| k.0.starts_with(&low)) {
        add(k.0, Src::Known(i), &mut out);
    }
    for p in programs.iter().filter(|p| p.to_lowercase().starts_with(&low)).take(8) {
        add(p, Src::Program, &mut out);
    }
    for h in history.iter().filter(|h| h.to_lowercase().contains(&low)) {
        add(h, Src::History, &mut out);
    }
    if low.chars().count() >= 3 {
        for (i, k) in KNOWN.iter().enumerate().filter(|(_, k)| k.0.contains(&low) || k.1.to_lowercase().contains(&low)) {
            add(k.0, Src::Known(i), &mut out);
        }
    }
    if let Some((dir, prefix)) = t.trim_start_matches('"').rsplit_once('\\') {
        let real = expand(&format!("{dir}\\"));
        if let Ok(rd) = std::fs::read_dir(&real) {
            let pl = prefix.to_lowercase();
            let mut v: Vec<(bool, String)> = rd
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let d = e.file_type().is_ok_and(|t| t.is_dir());
                    n.to_lowercase().starts_with(&pl).then(|| (!d, format!("{dir}\\{n}{}", if d { "\\" } else { "" })))
                })
                .collect();
            v.sort_by_cached_key(|a| (a.0, a.1.to_lowercase()));
            for (_, s) in v {
                add(&s, Src::Folder, &mut out);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_run_box() {
        let none = |_: &str| false;
        assert_eq!(split_cmd("notepad file.txt", none), ("notepad".into(), "file.txt".into()));
        assert_eq!(split_cmd("  cmd  ", none), ("cmd".into(), "".into()));
        assert_eq!(split_cmd(r#""C:\Program Files\a b.exe" -x -y"#, none), (r"C:\Program Files\a b.exe".into(), "-x -y".into()));
        assert_eq!(split_cmd("https://example.com/a b", none), ("https://example.com/a b".into(), "".into()));
        assert_eq!(split_cmd("ms-settings:display", none), ("ms-settings:display".into(), "".into()));
        assert_eq!(split_cmd("shell:startup", none), ("shell:startup".into(), "".into()));
        assert_eq!(split_cmd(r"C:\tools\x.exe y", none), (r"C:\tools\x.exe".into(), "y".into()));
        assert_eq!(split_cmd(r"C:\Program Files", |p| p == r"C:\Program Files"), (r"C:\Program Files".into(), "".into()));
    }

    #[test]
    fn programs_come_from_path_without_exe() {
        let p = programs();
        assert!(p.iter().any(|n| n.eq_ignore_ascii_case("cmd")), "System32 is on PATH");
        assert!(p.iter().all(|n| !n.to_lowercase().ends_with(".exe")));
        assert!(p.windows(2).all(|w| w[0].to_lowercase() < w[1].to_lowercase()), "sorted, no duplicates");
    }

    #[test]
    fn expands_known_variables_only() {
        unsafe { std::env::set_var("RS_RUN_T", "X") };
        assert_eq!(expand("%RS_RUN_T%\\a"), "X\\a");
        assert_eq!(expand("100% %NO_SUCH_VAR_Q% %"), "100% %NO_SUCH_VAR_Q% %");
    }

    #[test]
    fn history_moves_to_top_without_duplicates() {
        let mut h = vec!["cmd".to_string(), "notepad".to_string()];
        remember_in(&mut h, " NOTEPAD ");
        assert_eq!(h, ["NOTEPAD", "cmd"]);
        for i in 0..60 {
            remember_in(&mut h, &i.to_string());
        }
        assert_eq!(h.len(), HISTORY_MAX);
        assert_eq!(h[0], "59");
    }

    #[test]
    fn suggests_text_history_then_folder() {
        assert_eq!(super::suggest(r"c:\no such dir 9q\x", &[], &[])[0].1, Src::Typed);
        let suggest = |t: &str, h: &[String]| super::suggest(t, h, &[]).into_iter().map(|r| r.0).collect::<Vec<_>>();
        let h = vec!["regedit".to_string(), "cmd /k ver".to_string(), "control".to_string()];
        assert_eq!(suggest("", &h), ["regedit", "cmd /k ver", "control", "cmd", "powershell", "taskmgr", "msconfig"]);
        assert_eq!(suggest("co", &h), ["co", "control", "compmgmt.msc"]);
        assert_eq!(suggest("ed", &h)[..2], ["ed", "regedit"]);
        assert_eq!(suggest("Registry", &[]), ["Registry", "regedit"]);
        assert_eq!(super::suggest("chr", &[], &["chrome".into(), "charmap".into()]), [("chr".into(), Src::Typed), ("chrome".into(), Src::Program)]);
        assert_eq!(describe(" REGEDIT "), Some(2));
        assert!(KNOWN.iter().all(|k| k.0 == k.0.to_lowercase()), "prefix matching compares against lowercase");
        let base = std::env::temp_dir().join("rs-run-test-5e2a");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("Beta")).unwrap();
        std::fs::write(base.join("bravo.txt"), "").unwrap();
        std::fs::write(base.join("alpha.txt"), "").unwrap();
        let typed = format!("{}\\b", base.display());
        assert_eq!(suggest(&typed, &[]), [typed.clone(), format!("{}\\Beta\\", base.display()), format!("{}\\bravo.txt", base.display())]);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
