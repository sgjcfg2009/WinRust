use std::{
    collections::HashMap,
    ptr::{null, null_mut},
    sync::{mpsc, Arc, Mutex},
};
use winapi::um::{
    shellapi::ExtractIconExW,
    winver::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
};

#[derive(Clone, Default)]
pub struct Res {
    pub icon: isize,
    pub desc: String,
    pub company: String,
}

pub struct ResCache {
    map: Arc<Mutex<HashMap<String, Option<Res>>>>,
    tx: mpsc::Sender<String>,
    pub generic_icon: isize,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn version_string(buf: &[u8], lang: &str, key: &str) -> Option<String> {
    let q = wide(&format!(r"\StringFileInfo\{lang}\{key}"));
    let (mut p, mut len) = (null_mut(), 0u32);
    let ok = unsafe { VerQueryValueW(buf.as_ptr() as *const _, q.as_ptr(), &mut p, &mut len) };
    if ok == 0 || p.is_null() || len == 0 {
        return None;
    }
    let s = unsafe { std::slice::from_raw_parts(p as *const u16, len as usize) };
    let s = String::from_utf16_lossy(s).trim_end_matches('\0').trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn load(path: &str) -> Res {
    let mut r = Res::default();
    let w = wide(path);
    unsafe {
        let mut small = null_mut();
        if ExtractIconExW(w.as_ptr(), 0, null_mut(), &mut small, 1) > 0 {
            r.icon = small as isize;
        }
        let mut h = 0u32;
        let size = GetFileVersionInfoSizeW(w.as_ptr(), &mut h);
        if size > 0 {
            let mut buf = vec![0u8; size as usize];
            if GetFileVersionInfoW(w.as_ptr(), 0, size, buf.as_mut_ptr() as *mut _) != 0 {
                let (mut p, mut len) = (null_mut(), 0u32);
                let q = wide(r"\VarFileInfo\Translation");
                let mut langs: Vec<String> = vec![];
                if VerQueryValueW(buf.as_ptr() as *const _, q.as_ptr(), &mut p, &mut len) != 0 && len >= 4 {
                    let pairs = std::slice::from_raw_parts(p as *const u16, (len / 2) as usize);
                    for c in pairs.as_chunks::<2>().0 {
                        langs.push(format!("{:04x}{:04x}", c[0], c[1]));
                    }
                }
                langs.push("040904b0".into());
                langs.push("040904e4".into());
                for l in &langs {
                    if r.desc.is_empty() {
                        r.desc = version_string(&buf, l, "FileDescription").unwrap_or_default();
                    }
                    if r.company.is_empty() {
                        r.company = version_string(&buf, l, "CompanyName").unwrap_or_default();
                    }
                }
            }
        }
    }
    let _ = null::<u8>();
    let bad = |s: &str| s.chars().any(|c| c.is_control() || c == '\u{FFFD}');
    if bad(&r.desc) {
        r.desc.clear();
    }
    if bad(&r.company) {
        r.company.clear();
    }
    r
}

impl ResCache {
    pub fn new() -> Self {
        let map: Arc<Mutex<HashMap<String, Option<Res>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel::<String>();
        let m = map.clone();
        std::thread::spawn(move || {
            for path in rx {
                let r = load(&path);
                if let Ok(mut g) = m.lock() {
                    g.insert(path.to_lowercase(), Some(r));
                }
            }
        });
        let generic_icon = unsafe {
            let mut small = null_mut();
            let w = wide("shell32.dll");
            if ExtractIconExW(w.as_ptr(), 2, null_mut(), &mut small, 1) > 0 { small as isize } else { 0 }
        };
        Self { map, tx, generic_icon }
    }

    pub fn get(&self, path: &str) -> Option<Res> {
        let key = path.to_lowercase();
        let Ok(mut g) = self.map.lock() else { return None };
        match g.get(&key) {
            Some(v) => v.clone(),
            None => {
                g.insert(key, None);
                let _ = self.tx.send(path.to_string());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_description_and_icon_of_a_system_file() {
        let r = load(r"C:\Windows\System32\notepad.exe");
        assert!(!r.desc.is_empty(), "описание файла должно читаться");
        assert!(r.company.to_lowercase().contains("microsoft"));
        assert_ne!(r.icon, 0, "иконка должна извлекаться");
    }

    #[test]
    fn missing_file_gives_empty_resources() {
        let r = load(r"C:\no\such\file.exe");
        assert!(r.desc.is_empty() && r.icon == 0);
    }
}
