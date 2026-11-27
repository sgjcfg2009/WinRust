use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
};
use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

pub fn pipe_name(d: char) -> String {
    format!("\\\\.\\pipe\\rs-search-{d}")
}

pub fn drive_letters() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26).filter(|i| mask & (1 << i) != 0).map(|i| (b'A' + i as u8) as char).collect()
}

pub fn ask(d: char, q: &str) -> Option<io::Result<String>> {
    let mut p = match OpenOptions::new().read(true).write(true).open(pipe_name(d)) {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(e)),
    };
    Some((|| {
        p.write_all(q.as_bytes())?;
        let (mut out, mut chunk) = (Vec::new(), [0u8; 1 << 16]);
        loop {
            match p.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.raw_os_error(), Some(109 | 233)) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    })())
}

pub fn ask_all(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    for d in drive_letters() {
        if let Some(Ok(s)) = ask(d, q) {
            out.extend(s.lines().filter(|l| !l.is_empty() && !l.starts_with("... ")).map(String::from));
        }
    }
    out
}
