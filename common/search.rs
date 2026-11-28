use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
};
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

pub fn pipe_name(d: char) -> String {
    format!("\\\\.\\pipe\\rs-search-{d}")
}

pub fn drive_letters() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26).filter(|i| mask & (1 << i) != 0).map(|i| (b'A' + i as u8) as char).collect()
}

fn ask_with(d: char, q: &str, chunk: &mut [u8]) -> Option<io::Result<String>> {
    let mut p = match OpenOptions::new().read(true).write(true).open(pipe_name(d)) {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(e)),
    };
    Some((|| {
        p.write_all(q.as_bytes())?;
        let mut out = Vec::new();
        loop {
            match p.read(chunk) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.raw_os_error(), Some(109 | 233)) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    })())
}

pub fn ask(d: char, q: &str) -> Option<io::Result<String>> {
    let mut chunk = [0u8; 1 << 16];
    ask_with(d, q, &mut chunk)
}

pub fn ask_all(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 1 << 16];
    for d in drive_letters() {
        let root = [d as u16, b':' as u16, b'\\' as u16, 0];
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
            continue;
        }
        if let Some(Ok(s)) = ask_with(d, q, &mut chunk) {
            out.extend(s.lines().filter(|l| !l.is_empty() && !l.starts_with("... ")).map(String::from));
        }
    }
    out
}
