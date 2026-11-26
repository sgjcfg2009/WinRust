use std::process::Command;

const NO_WINDOW: u32 = 0x0800_0000;

const PROGRAM_EXTS: [&str; 7] = ["exe", "com", "scr", "bat", "cmd", "msc", "cpl"];

fn ext_of(target: &str) -> &str {
    match target.trim().trim_matches('"').rsplit_once('.') {
        Some((_, ext)) => ext.trim(),
        None => "",
    }
}

pub fn is_program(target: &str) -> bool {
    let t = target.trim().trim_matches('"');
    if t.is_empty() || is_uri(t) {
        return false;
    }
    let ext = ext_of(t);
    PROGRAM_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e))
}

fn is_uri(t: &str) -> bool {
    if t.contains("://") || t.starts_with("::") {
        return true;
    }
    match t.find(':') {
        Some(i) if (1..8).contains(&i) && !(i == 1 && (t[i + 1..].starts_with('\\') || t[i + 1..].starts_with('/'))) => {
            t[..i].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        }
        _ => false,
    }
}

pub fn program(target: &str) -> bool {
    use std::os::windows::process::CommandExt;
    let target = target.trim().trim_matches('"');
    if target.is_empty() {
        return false;
    }
    let uac_on = crate::reg::get_dword(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System", "EnableLUA") != Some(0);
    let ext = ext_of(target);
    if uac_on
        && (ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com") || ext.eq_ignore_ascii_case("scr"))
        && Command::new("runas").args(["/trustlevel:0x20000", target]).creation_flags(NO_WINDOW).spawn().is_ok()
    {
        return true;
    }
    shell_execute(target)
}

pub fn program_args(target: &str, args: &str) -> bool {
    let target = target.trim().trim_matches('"');
    if target.is_empty() {
        return false;
    }
    if args.trim().is_empty() {
        return program(target);
    }
    use std::ptr::null_mut;
    use windows_sys::Win32::{UI::Shell::ShellExecuteW, UI::WindowsAndMessaging::SW_SHOWNORMAL};
    const OPEN: [u16; 5] = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
    crate::with_wide(target, |file| {
        crate::with_wide(args, |par| unsafe {
            ShellExecuteW(null_mut(), OPEN.as_ptr(), file, par, null_mut(), SW_SHOWNORMAL) as isize > 32
        })
    })
}

fn shell_execute(target: &str) -> bool {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{UI::Shell::ShellExecuteW, UI::WindowsAndMessaging::SW_SHOWNORMAL};
    const OPEN: [u16; 5] = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
    crate::with_wide(target, |file| unsafe {
        ShellExecuteW(null_mut(), OPEN.as_ptr(), file, null(), null(), SW_SHOWNORMAL) as isize > 32
    })
}

#[cfg(test)]
mod tests {
    use super::is_program;

    #[test]
    fn programs_are_told_from_shell_targets() {
        for p in ["wt.exe", "taskmgr.exe", r"C:\Windows\System32\taskmgr.exe", "devmgmt.msc", "powercfg.cpl"] {
            assert!(is_program(p), "{p} is a program");
        }
        for p in ["shell:MyComputerFolder", "ms-settings:", "https://example.com", r"C:\Users", "app.lnk", "regedit"] {
            assert!(!is_program(p), "{p} is not a program");
        }
    }
}
