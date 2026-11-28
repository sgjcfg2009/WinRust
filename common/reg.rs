pub fn value(out: &str) -> Option<(&str, &str)> {
    out.lines().find_map(|l| {
        let mut f = l.strip_prefix("    ")?.splitn(3, "    ");
        let (_, ty) = (f.next()?, f.next()?);
        ty.starts_with("REG_").then(|| (ty, f.next().unwrap_or("").trim_end()))
    })
}

fn split_root(path: &str) -> Option<(windows_sys::Win32::System::Registry::HKEY, &str)> {
    use windows_sys::Win32::System::Registry::*;
    let (root, sub) = path.split_once('\\').unwrap_or((path, ""));
    let hkey = if root.eq_ignore_ascii_case("HKLM") || root.eq_ignore_ascii_case("HKEY_LOCAL_MACHINE") {
        HKEY_LOCAL_MACHINE
    } else if root.eq_ignore_ascii_case("HKCU") || root.eq_ignore_ascii_case("HKEY_CURRENT_USER") {
        HKEY_CURRENT_USER
    } else if root.eq_ignore_ascii_case("HKU") || root.eq_ignore_ascii_case("HKEY_USERS") {
        HKEY_USERS
    } else if root.eq_ignore_ascii_case("HKCR") || root.eq_ignore_ascii_case("HKEY_CLASSES_ROOT") {
        HKEY_CLASSES_ROOT
    } else {
        return None;
    };
    Some((hkey, sub))
}

pub fn exists(path: &str, name: Option<&str>) -> bool {
    use windows_sys::Win32::System::Registry::*;
    let Some((root, sub)) = split_root(path) else { return false };
    crate::with_wide(sub, |sub_p| unsafe {
        let mut k = std::ptr::null_mut();
        if RegOpenKeyExW(root, sub_p, 0, KEY_READ, &mut k) != 0 {
            return false;
        }
        let ok = match name {
            None => true,
            Some(n) => crate::with_wide(n, |name_p| {
                let (mut ty, mut len) = (0u32, 0u32);
                RegQueryValueExW(k, name_p, std::ptr::null(), &mut ty, std::ptr::null_mut(), &mut len) == 0
            }),
        };
        RegCloseKey(k);
        ok
    })
}

pub fn get_dword(path: &str, name: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::*;
    let (root, sub) = split_root(path)?;
    crate::with_wide(sub, |sub_p| {
        crate::with_wide(name, |name_p| unsafe {
            let mut k = std::ptr::null_mut();
            if RegOpenKeyExW(root, sub_p, 0, KEY_READ, &mut k) != 0 {
                return None;
            }
            let (mut ty, mut val, mut len) = (0u32, 0u32, 4u32);
            let ok = RegQueryValueExW(k, name_p, std::ptr::null(), &mut ty, (&mut val as *mut u32).cast(), &mut len) == 0;
            RegCloseKey(k);
            (ok && ty == 4 && len == 4).then_some(val)
        })
    })
}

pub fn get_str(path: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::*;
    let (root, sub) = split_root(path)?;
    crate::with_wide(sub, |sub_p| {
        crate::with_wide(name, |name_p| unsafe {
            let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
            let mut stack = [0u16; 260];
            let mut len = (stack.len() * 2) as u32;
            let rc = RegGetValueW(root, sub_p, name_p, flags, std::ptr::null_mut(), stack.as_mut_ptr().cast(), &mut len);
            if rc == 0 {
                let mut used = (len as usize / 2).min(stack.len());
                while used > 0 && stack[used - 1] == 0 {
                    used -= 1;
                }
                return Some(String::from_utf16_lossy(&stack[..used]));
            }
            if rc != 234 {
                return None;
            }
            let mut heap = vec![0u16; (len as usize + 1) / 2 + 16];
            let mut hlen = (heap.len() * 2) as u32;
            if RegGetValueW(root, sub_p, name_p, flags, std::ptr::null_mut(), heap.as_mut_ptr().cast(), &mut hlen) != 0 {
                return None;
            }
            let mut used = (hlen as usize / 2).min(heap.len());
            while used > 0 && heap[used - 1] == 0 {
                used -= 1;
            }
            Some(String::from_utf16_lossy(&heap[..used]))
        })
    })
}

pub fn dword(out: &str) -> Option<u32> {
    match value(out)? {
        ("REG_DWORD", d) => u32::from_str_radix(d.trim_start_matches("0x"), 16).ok(),
        _ => None,
    }
}

pub const SID_ENV: &str = "WINRUST_SID";

pub fn valid_sid(s: &str) -> bool {
    s.starts_with("S-1-") && s.len() > 4 && s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
}

pub fn for_user(arg: &str, sid: &str) -> String {
    let rest = ["HKCU", "HKEY_CURRENT_USER"].iter().find_map(|p| arg.strip_prefix(p).filter(|r| r.is_empty() || r.starts_with('\\')));
    rest.map_or_else(|| arg.to_string(), |r| format!("HKU\\{sid}{r}"))
}

pub fn user_args(args: &[&str]) -> Vec<String> {
    let sid = std::env::var(SID_ENV).ok().filter(|s| valid_sid(s));
    args.iter().map(|a| sid.as_deref().map_or_else(|| a.to_string(), |s| for_user(a, s))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aims_hkcu_at_the_given_user() {
        assert_eq!(for_user(r"HKCU\Software\X", "S-1-5-21-1-2-3-1001"), r"HKU\S-1-5-21-1-2-3-1001\Software\X");
        assert_eq!(for_user("HKEY_CURRENT_USER", "S-1-5-21-9"), r"HKU\S-1-5-21-9");
        assert_eq!(for_user(r"HKLM\SOFTWARE\X", "S-1-5-21-9"), r"HKLM\SOFTWARE\X");
        assert_eq!(for_user(r"HKCUX\a", "S-1-5-21-9"), r"HKCUX\a");
        assert_eq!(for_user("/v", "S-1-5-21-9"), "/v");
        assert!(valid_sid("S-1-5-21-1-2-3-1001") && !valid_sid("S-1-5-x; calc") && !valid_sid(""));
    }

    #[test]
    fn reads_reg_query_output() {
        let key = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\REG_X\r\n";
        assert_eq!(dword(&format!("{key}    AllowTelemetry    REG_DWORD    0x1a\r\n\r\n")), Some(26));
        assert_eq!(value(&format!("{key}    Debugger    REG_SZ    \"C:\\Program Files\\x.exe\" /e\r\n")), Some(("REG_SZ", "\"C:\\Program Files\\x.exe\" /e")));
        assert_eq!(value("    (Default)    REG_SZ    \r\n"), Some(("REG_SZ", "")));
        assert_eq!(dword("    V    REG_SZ    0x1\r\n"), None);
        assert_eq!(value("ERROR: The system was unable to find the specified registry key or value."), None);
    }

    #[test]
    fn native_read() {
        assert!(exists(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", Some("ProductName")));
        assert!(exists(r"HKLM\SOFTWARE\Microsoft", None));
        assert!(!exists(r"HKLM\SOFTWARE\no-such-key-winrust", None));
        assert!(!exists(r"BAD\x", None));
        assert!(get_dword(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentMajorVersionNumber").is_some_and(|v| v >= 10));
        assert_eq!(get_dword(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "ProductName"), None, "REG_SZ is not a DWORD");
    }
}
