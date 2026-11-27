use std::{fs, io, path::Path};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::DELETE, System::Registry::*};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Root {
    Hkcu,
    Hklm,
}
impl Root {
    fn h(self) -> HKEY {
        if self == Root::Hkcu { HKEY_CURRENT_USER } else { HKEY_LOCAL_MACHINE }
    }
    fn name(self) -> &'static str {
        if self == Root::Hkcu { "HKEY_CURRENT_USER" } else { "HKEY_LOCAL_MACHINE" }
    }
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub root: Root,
    pub key: String,
    pub value: Option<String>,
}
impl Hit {
    pub fn describe(&self) -> String {
        let r = if self.root == Root::Hkcu { "HKCU" } else { "HKLM" };
        match &self.value {
            Some(v) => format!("{r}\\{}  \u{00B7}  {}", self.key, if v.is_empty() { "(Default)" } else { v }),
            None => format!("{r}\\{}  (whole entry)", self.key),
        }
    }
}

#[derive(Default)]
pub struct Report {
    pub removed: usize,
    pub denied: usize,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn boundary_after(c: Option<char>) -> bool {
    c.is_none_or(|c| matches!(c, '\\' | '"' | '\'' | ',' | ';' | ' ' | '\n' | '/'))
}

fn contains_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() {
        return Some(0);
    }
    if n.len() > h.len() {
        return None;
    }
    'at: for i in 0..=h.len() - n.len() {
        if h[i].to_ascii_lowercase() != n[0] {
            continue;
        }
        for j in 1..n.len() {
            if h[i + j].to_ascii_lowercase() != n[j] {
                continue 'at;
            }
        }
        return Some(i);
    }
    None
}

fn refs(hay: &str, needle: &str) -> bool {
    if hay.is_ascii() && needle.is_ascii() {
        let mut from = 0;
        while let Some(i) = contains_ci(&hay[from..], needle) {
            let (s, e) = (from + i, from + i + needle.len());
            let before_ok = hay[..s].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
            if before_ok && boundary_after(hay[e..].chars().next()) {
                return true;
            }
            from = e;
        }
        return false;
    }
    let h = hay.to_lowercase();
    let mut from = 0;
    while let Some(i) = h[from..].find(needle) {
        let (s, e) = (from + i, from + i + needle.len());
        let before_ok = h[..s].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        if before_ok && boundary_after(h[e..].chars().next()) {
            return true;
        }
        from = e;
    }
    false
}

fn under(hay: &str, needle: &str) -> bool {
    let trimmed = hay.trim_start_matches([' ', '"']);
    if trimmed.is_ascii() && needle.is_ascii() {
        return trimmed.len() >= needle.len()
            && trimmed.as_bytes()[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
            && boundary_after(trimmed[needle.len()..].chars().next());
    }
    let h = trimmed.to_lowercase();
    h.starts_with(needle) && boundary_after(h[needle.len()..].chars().next())
}

struct Val {
    name: String,
    ty: u32,
    data: Vec<u8>,
}
impl Val {
    fn text(&self) -> Option<String> {
        matches!(self.ty, REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ).then(|| {
            let u: Vec<u16> = self.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&u).trim_end_matches('\0').replace('\0', "\n")
        })
    }
}

struct Scratch {
    name: Vec<u16>,
    data: Vec<u8>,
    sub: [u16; 256],
}

impl Scratch {
    fn new() -> Self {
        Self { name: vec![0u16; 16384], data: vec![0u8; 1024], sub: [0u16; 256] }
    }
}

unsafe fn open(parent: HKEY, sub: &str, write: bool) -> Option<HKEY> {
    let mut k: HKEY = std::ptr::null_mut();
    let acc = if write { KEY_READ | KEY_SET_VALUE | DELETE } else { KEY_READ };
    (RegOpenKeyExW(parent, wide(sub).as_ptr(), 0, acc, &mut k) == ERROR_SUCCESS).then_some(k)
}

unsafe fn read_values(key: HKEY, sc: &mut Scratch) -> Vec<Val> {
    let (mut out, mut i) = (Vec::new(), 0u32);
    loop {
        let (mut nlen, mut dlen, mut ty) = (sc.name.len() as u32, sc.data.len() as u32, 0u32);
        match RegEnumValueW(key, i, sc.name.as_mut_ptr(), &mut nlen, std::ptr::null(), &mut ty, sc.data.as_mut_ptr(), &mut dlen) {
            ERROR_SUCCESS => {
                out.push(Val { name: String::from_utf16_lossy(&sc.name[..nlen as usize]), ty, data: sc.data[..dlen as usize].to_vec() });
                i += 1;
            }
            ERROR_MORE_DATA => sc.data.resize((dlen as usize).max(sc.data.len() * 2), 0),
            _ => break,
        }
    }
    out
}

unsafe fn subkeys(key: HKEY, sc: &mut Scratch) -> Vec<String> {
    let (mut out, mut i) = (Vec::new(), 0u32);
    loop {
        let mut len = sc.sub.len() as u32;
        if RegEnumKeyExW(key, i, sc.sub.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&sc.sub[..len as usize]));
        i += 1;
    }
    out
}

const OWNER_VALUES: [&str; 5] = ["installlocation", "uninstallstring", "quietuninstallstring", "", "path"];
fn owns(vals: &[Val], needle: &str) -> bool {
    vals.iter().any(|v| OWNER_VALUES.iter().any(|&o| o.eq_ignore_ascii_case(&v.name)) && v.text().is_some_and(|t| !t.is_empty() && under(&t, needle)))
}

unsafe fn walk(root: Root, key: HKEY, path: &str, needle: &str, depth: u32, hits: &mut Vec<Hit>, sc: &mut Scratch) {
    for v in read_values(key, sc) {
        if refs(&v.name, needle) || v.text().is_some_and(|t| refs(&t, needle)) {
            hits.push(Hit { root, key: path.into(), value: Some(v.name) });
        }
    }
    let leaf = path.rsplit('\\').next().unwrap_or("");
    let container = leaf.eq_ignore_ascii_case("uninstall") || leaf.eq_ignore_ascii_case("app paths");
    for s in subkeys(key, sc) {
        let Some(child) = open(key, &s, false) else { continue };
        let child_path = format!("{path}\\{s}");
        if container && owns(&read_values(child, sc), needle) {
            hits.push(Hit { root, key: child_path, value: None });
        } else if depth < 40 {
            walk(root, child, &child_path, needle, depth + 1, hits, sc);
        }
        RegCloseKey(child);
    }
}

pub fn scan(path: &str) -> Vec<Hit> {
    let needle = path.trim_end_matches('\\').to_lowercase();
    let mut hits = Vec::new();
    let mut sc = Scratch::new();
    for (root, sub) in [(Root::Hkcu, "Software"), (Root::Hklm, "SOFTWARE")] {
        unsafe {
            if let Some(k) = open(root.h(), sub, false) {
                walk(root, k, sub, &needle, 0, &mut hits, &mut sc);
                RegCloseKey(k);
            }
        }
    }
    hits
}

fn reg_line(v: &Val) -> String {
    let name = if v.name.is_empty() { "@".into() } else { format!("\"{}\"", v.name.replace('\\', "\\\\").replace('"', "\\\"")) };
    let hex = || v.data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(",");
    let data = match v.ty {
        REG_DWORD if v.data.len() == 4 => format!("dword:{:08x}", u32::from_le_bytes(v.data[..4].try_into().unwrap())),
        REG_BINARY => format!("hex:{}", hex()),
        t => format!("hex({t:x}):{}", hex()),
    };
    let note = v.text().map(|t| format!("; {}\r\n", t.replace('\n', " | "))).unwrap_or_default();
    format!("{note}{name}={data}\r\n")
}

unsafe fn dump_tree(root: Root, key: HKEY, path: &str, out: &mut String, depth: u32, sc: &mut Scratch) {
    out.push_str(&format!("\r\n[{}\\{path}]\r\n", root.name()));
    for v in read_values(key, sc) {
        out.push_str(&reg_line(&v));
    }
    for s in subkeys(key, sc) {
        if let Some(c) = open(key, &s, false) {
            if depth < 40 {
                dump_tree(root, c, &format!("{path}\\{s}"), out, depth + 1, sc);
            }
            RegCloseKey(c);
        }
    }
}

pub fn backup(hits: &[Hit], file: &Path) -> io::Result<()> {
    let mut out = String::from("Windows Registry Editor Version 5.00\r\n");
    let mut last_header = String::new();
    let mut sc = Scratch::new();
    for h in hits {
        unsafe {
            let Some(k) = open(h.root.h(), &h.key, false) else { continue };
            match &h.value {
                None => {
                    dump_tree(h.root, k, &h.key, &mut out, 0, &mut sc);
                    last_header.clear();
                }
                Some(name) => {
                    if let Some(v) = read_values(k, &mut sc).into_iter().find(|v| v.name == *name) {
                        let header = format!("[{}\\{}]", h.root.name(), h.key);
                        if header != last_header {
                            out.push_str(&format!("\r\n{header}\r\n"));
                            last_header = header;
                        }
                        out.push_str(&reg_line(&v));
                    }
                }
            }
            RegCloseKey(k);
        }
    }
    if let Some(d) = file.parent() {
        fs::create_dir_all(d)?;
    }
    let mut b = vec![0xFF, 0xFE];
    b.extend(out.encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(file, b)
}

pub fn delete(hits: &[Hit]) -> Report {
    let mut r = Report::default();
    for h in hits {
        let ok = unsafe {
            match &h.value {
                None => RegDeleteTreeW(h.root.h(), wide(&h.key).as_ptr()) == ERROR_SUCCESS,
                Some(v) => match open(h.root.h(), &h.key, true) {
                    Some(k) => {
                        let ok = RegDeleteValueW(k, wide(v).as_ptr()) == ERROR_SUCCESS;
                        RegCloseKey(k);
                        ok
                    }
                    None => false,
                },
            }
        };
        if ok { r.removed += 1 } else { r.denied += 1 }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    #[test]
    #[ignore = "writes to the Windows registry; run only in a disposable Windows VM"]
    fn scan_backup_delete_restore_in_the_real_registry() {
        let p = "C:\\rs-sweep-test-9f3a\\App";
        let reg = |args: &[&str]| std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output().unwrap().status.success();
        let base = "HKCU\\Software\\rs-sweep-test-9f3a";
        let _ = reg(&["delete", base, "/f"]);
        assert!(reg(&["add", &format!("{base}\\Run"), "/v", "x", "/d", &format!("\"{p}\\a.exe\" /s"), "/f"]));
        assert!(reg(&["add", &format!("{base}\\Run"), "/v", "keep", "/d", "C:\\rs-sweep-test-9f3aXYZ\\a.exe", "/f"]));
        assert!(reg(&["add", &format!("{base}\\Run"), "/v", "n", "/t", "REG_DWORD", "/d", "7", "/f"]));
        assert!(reg(&["add", &format!("{base}\\Uninstall\\Prod"), "/v", "InstallLocation", "/d", p, "/f"]));
        assert!(reg(&["add", &format!("{base}\\Uninstall\\Prod\\Sub"), "/v", "Size", "/t", "REG_DWORD", "/d", "5", "/f"]));
        assert!(reg(&["add", &format!("{base}\\Uninstall\\Other"), "/v", "DisplayIcon", "/d", &format!("{p}\\icon.ico"), "/f"]));
        assert!(reg(&["add", &format!("{base}\\Uninstall\\Other"), "/v", "InstallLocation", "/d", "C:\\Elsewhere", "/f"]));

        let hits: Vec<Hit> = scan(p).into_iter().filter(|h| h.key.contains("rs-sweep-test-9f3a")).collect();
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(hits.iter().any(|h| h.key.ends_with("Uninstall\\Prod") && h.value.is_none()));
        assert!(hits.iter().any(|h| h.key.ends_with("Uninstall\\Other") && h.value.as_deref() == Some("DisplayIcon")));

        let bak = std::env::temp_dir().join("rs-sweep-test-9f3a.reg");
        backup(&hits, &bak).unwrap();
        let r = delete(&hits);
        assert_eq!((r.removed, r.denied), (3, 0));
        assert!(!reg(&["query", &format!("{base}\\Run"), "/v", "x"]), "value must be gone");
        assert!(reg(&["query", &format!("{base}\\Run"), "/v", "keep"]), "look-alike path must stay");
        assert!(!reg(&["query", &format!("{base}\\Uninstall\\Prod")]), "uninstall key must be gone");
        assert!(reg(&["query", &format!("{base}\\Uninstall\\Other"), "/v", "InstallLocation"]), "other program's entry must stay");

        assert!(reg(&["import", bak.to_str().unwrap()]), "backup must import");
        assert!(reg(&["query", &format!("{base}\\Run"), "/v", "x"]), "value restored");
        assert!(reg(&["query", &format!("{base}\\Uninstall\\Prod\\Sub"), "/v", "Size"]), "whole tree restored");
        assert!(reg(&["query", &format!("{base}\\Uninstall\\Other"), "/v", "DisplayIcon"]), "icon value restored");
        assert_eq!(scan(p).into_iter().filter(|h| h.key.contains("rs-sweep-test-9f3a")).count(), 3, "restored exactly");
        let _ = fs::remove_file(bak);
        assert!(reg(&["delete", base, "/f"]));
    }

    #[test]
    fn matches_whole_paths_only() {
        let n = "c:\\tools\\foo";
        assert!(refs("C:\\Tools\\Foo", n));
        assert!(refs("\"C:\\Tools\\Foo\\bar.exe\" /x", n));
        assert!(refs("C:\\Tools\\Foo,0", n));
        assert!(!refs("C:\\Tools\\Foobar\\x.exe", n));
        assert!(!refs("xc:\\tools\\foo", n));
        assert!(under("\"C:\\Tools\\Foo\\u.exe\" /S", n));
        assert!(under("C:\\Tools\\Foo", n));
        assert!(!under("C:\\Tools\\Foobar", n));
        assert!(!under("C:\\Tools", n));
    }
}
