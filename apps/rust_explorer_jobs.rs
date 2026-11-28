use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering::{Acquire, Relaxed, Release}},
    },
    time::Instant,
};

struct Progress {
    pub what: String,
    pub files: AtomicU64,
    pub total_files: AtomicU64,
    pub bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    cancel: AtomicBool,
    started: Instant,
}

impl Progress {
    fn new(what: String) -> Arc<Progress> {
        Arc::new(Progress { what, files: 0.into(), total_files: 0.into(), bytes: 0.into(), total_bytes: 0.into(), cancel: false.into(), started: Instant::now() })
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Acquire)
    }
}

static JOB: Mutex<Option<Arc<Progress>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static TICK_LOCK: Mutex<bool> = Mutex::new(false);
static TICK_CV: Condvar = Condvar::new();

pub fn status_text() -> Option<String> {
    let p = JOB.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let (b, tb, f, tf) = (p.bytes.load(Relaxed), p.total_bytes.load(Relaxed), p.files.load(Relaxed), p.total_files.load(Relaxed));
    let secs = p.started.elapsed().as_secs_f64().max(0.1);
    let mut s = String::with_capacity(p.what.len() + 64);
    s.push_str(&p.what);
    if tf > 0 {
        let _ = write!(s, "  \u{00B7}  {f} of {tf} files");
    }
    if tb > 0 {
        let _ = write!(
            s,
            "  \u{00B7}  {} of {} ({}%)  \u{00B7}  {}/s",
            super::size_text(b),
            super::size_text(tb),
            b * 100 / tb.max(1),
            super::size_text((b as f64 / secs) as u64)
        );
    }
    s.push('\u{2026}');
    Some(s)
}

pub fn busy() -> bool {
    RUNNING.load(Acquire)
}

pub fn progress_pct() -> Option<u8> {
    let p = JOB.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let (b, tb, f, tf) = (p.bytes.load(Relaxed), p.total_bytes.load(Relaxed), p.files.load(Relaxed), p.total_files.load(Relaxed));
    if tb > 0 {
        Some(((b * 100) / tb.max(1)).min(100) as u8)
    } else if tf > 0 {
        Some(((f * 100) / tf.max(1)).min(100) as u8)
    } else {
        Some(0)
    }
}

pub fn cancel() {
    if let Some(p) = JOB.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        p.cancel.store(true, Release);
    }
}

fn spawn(h: isize, what: String, f: impl FnOnce(&Progress) -> Result<(), String> + Send + 'static) -> bool {
    if RUNNING.swap(true, Acquire) {
        return false;
    }
    let p = Progress::new(what);
    *JOB.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
    *TICK_LOCK.lock().unwrap_or_else(|e| e.into_inner()) = true;
    let ticker = std::thread::spawn(move || {
        let mut g = TICK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        while *g {
            unsafe { super::ui::status_dirty(h as _) };
            let (ng, _) = TICK_CV.wait_timeout(g, std::time::Duration::from_millis(200)).unwrap_or_else(|e| e.into_inner());
            g = ng;
        }
    });
    std::thread::spawn(move || {
        let r = f(&p);
        *JOB.lock().unwrap_or_else(|e| e.into_inner()) = None;
        RUNNING.store(false, Release);
        *TICK_LOCK.lock().unwrap_or_else(|e| e.into_inner()) = false;
        TICK_CV.notify_one();
        let _ = ticker.join();
        let msg = Box::new(r.err());
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(h as _, super::WM_JOB, Box::into_raw(msg) as usize, 0) };
    });
    true
}

type Entry = (PathBuf, PathBuf, Option<u64>);

fn walk(src: &Path, rel: PathBuf, out: &mut Vec<Entry>) -> std::io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.is_dir() {
        out.push((src.to_path_buf(), rel.clone(), None));
        for e in fs::read_dir(src)? {
            let e = e?;
            walk(&e.path(), rel.join(e.file_name()), out)?;
        }
    } else if !(md.file_type().is_symlink() && fs::metadata(src).is_ok_and(|m| m.is_dir())) {
        let len = if md.file_type().is_symlink() { fs::metadata(src).map_or(md.len(), |m| m.len()) } else { md.len() };
        out.push((src.to_path_buf(), rel, Some(len)));
    }
    Ok(())
}

pub fn free_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let mut p = dir.join(name);
    if fs::symlink_metadata(&p).is_err() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !is_dir && !s.is_empty() => (s, e),
        _ => (name, ""),
    };
    let mut buf = String::with_capacity(name.len() + 8);
    for n in 2u32.. {
        buf.clear();
        if ext.is_empty() {
            let _ = write!(buf, "{stem} ({n})");
        } else {
            let _ = write!(buf, "{stem} ({n}).{ext}");
        }
        p.set_file_name(&buf);
        if fs::symlink_metadata(&p).is_err() {
            return p;
        }
    }
    unreachable!()
}

pub fn can_fast_copy(from: &[PathBuf], to: &Path) -> bool {
    to.is_dir() && to.has_root() && !from.is_empty() && from.iter().all(|p| p.has_root() && fs::symlink_metadata(p).is_ok() && !to.starts_with(p))
}

pub fn writable(dir: &Path) -> bool {
    let t = dir.join(format!(".winrust-{}.tmp", std::process::id()));
    let ok = fs::File::create(&t).is_ok();
    let _ = fs::remove_file(&t);
    ok
}

pub fn same_volume(a: &Path, b: &Path) -> bool {
    a.components().next().zip(b.components().next()).is_some_and(|(x, y)| x.as_os_str().eq_ignore_ascii_case(y.as_os_str()))
}

fn seek_penalty(p: &Path) -> bool {
    use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::{IO::DeviceIoControl, Ioctl::*}};
    let Some(first) = std::os::windows::ffi::OsStrExt::encode_wide(p.as_os_str()).next() else { return false };
    if !(first as u8 as char).is_ascii_alphabetic() || first > 0x7F {
        return false;
    }
    let dev = [b'\\' as u16, b'\\' as u16, b'.' as u16, b'\\' as u16, first, b':' as u16, 0];
    unsafe {
        let h = CreateFileW(dev.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE {
            return false;
        }
        let q = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceSeekPenaltyProperty, QueryType: PropertyStandardQuery, AdditionalParameters: [0] };
        let mut d: DEVICE_SEEK_PENALTY_DESCRIPTOR = std::mem::zeroed();
        let mut n = 0u32;
        let ok = DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, &q as *const _ as _, size_of::<STORAGE_PROPERTY_QUERY>() as u32, &mut d as *mut _ as _, size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32, &mut n, std::ptr::null_mut()) != 0;
        CloseHandle(h);
        ok && d.IncursSeekPenalty
    }
}

const BIG: u64 = 256 << 20;

fn fill_wide_path(p: &Path, buf: &mut Vec<u16>) {
    use std::os::windows::ffi::OsStrExt;
    buf.clear();
    buf.extend(p.as_os_str().encode_wide());
    if buf.len() >= 240 && p.is_absolute() && !buf.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]) {
        let mut prefixed = Vec::with_capacity(buf.len() + 8);
        if buf.starts_with(&[b'\\' as u16, b'\\' as u16]) {
            prefixed.extend([b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16, b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16]);
            prefixed.extend_from_slice(&buf[2..]);
        } else {
            prefixed.extend([b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]);
            prefixed.extend_from_slice(buf);
        }
        *buf = prefixed;
    }
    buf.push(0);
}

fn copy_file(src: &Path, dst: &Path, size: u64, p: &Progress, ws: &mut Vec<u16>, wd: &mut Vec<u16>) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{COPY_FILE_FAIL_IF_EXISTS, COPY_FILE_NO_BUFFERING, CopyFileExW};
    unsafe extern "system" fn cb(_total: i64, done: i64, _st: i64, _sd: i64, _id: u32, _r: u32, _s: *mut core::ffi::c_void, _d: *mut core::ffi::c_void, data: *const core::ffi::c_void) -> u32 {
        let (prog, last) = unsafe { &mut *(data as *mut (&Progress, u64)) };
        prog.bytes.fetch_add((done as u64).saturating_sub(*last), Relaxed);
        *last = done as u64;
        u32::from(prog.cancelled())
    }
    fill_wide_path(src, ws);
    fill_wide_path(dst, wd);
    let mut state: (&Progress, u64) = (p, 0);
    let flags = COPY_FILE_FAIL_IF_EXISTS | if size >= BIG { COPY_FILE_NO_BUFFERING } else { 0 };
    let ok = unsafe { CopyFileExW(ws.as_ptr(), wd.as_ptr(), Some(cb), &mut state as *mut _ as _, std::ptr::null_mut(), flags) } != 0;
    p.bytes.fetch_add(size.saturating_sub(state.1), Relaxed);
    if ok { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

pub fn copy(h: isize, from: Vec<PathBuf>, to: PathBuf, then_delete: bool) -> bool {
    let what = if then_delete { "Moving" } else { "Copying" }.to_string();
    spawn(h, what, move |p| copy_now(&from, &to, then_delete, p))
}

fn copy_now(from: &[PathBuf], to: &Path, then_delete: bool, p: &Progress) -> Result<(), String> {
    let mut entries = vec![];
    for src in from {
        let name = src.file_name().ok_or_else(|| format!("{}: no name", src.display()))?.to_string_lossy();
        let top = free_name(to, &name, src.is_dir());
        let top_name = top.file_name().ok_or_else(|| format!("{}: no name", src.display()))?;
        walk(src, PathBuf::from(top_name), &mut entries).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    let (mut tf, mut tb) = (0u64, 0u64);
    let mut files: Vec<&Entry> = Vec::with_capacity(entries.len());
    for e in &entries {
        if let Some(sz) = e.2 {
            tf += 1;
            tb += sz;
            files.push(e);
        } else {
            let d = to.join(&e.1);
            fs::create_dir_all(&d).map_err(|err| format!("{}: {err}", d.display()))?;
        }
    }
    p.total_files.store(tf, Relaxed);
    p.total_bytes.store(tb, Relaxed);
    files.sort_by_key(|e| std::cmp::Reverse(e.2));
    let threads = if seek_penalty(&from[0]) || seek_penalty(to) { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8) };
    let next = AtomicU64::new(0);
    let errors: Mutex<Vec<String>> = Mutex::new(vec![]);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let (mut ws, mut wd, mut dst) = (Vec::with_capacity(260), Vec::with_capacity(260), PathBuf::with_capacity(260));
                while !p.cancelled() {
                    let Some(e) = files.get(next.fetch_add(1, Relaxed) as usize) else { break };
                    dst.clear();
                    dst.push(to);
                    dst.push(&e.1);
                    let Some(sz) = e.2 else { continue };
                    match copy_file(&e.0, &dst, sz, p, &mut ws, &mut wd) {
                        Ok(()) => _ = p.files.fetch_add(1, Relaxed),
                        Err(err) if !p.cancelled() => errors.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{}: {err}", e.0.display())),
                        Err(_) => {}
                    }
                }
            });
        }
    });
    if p.cancelled() {
        return Err("Cancelled".to_string());
    }
    let errors = errors.into_inner().unwrap_or_else(|e| e.into_inner());
    if !errors.is_empty() {
        let more = if errors.len() > 5 { format!("\n... and {} more", errors.len() - 5) } else { String::new() };
        let kept = if then_delete { "\nNothing was deleted from the source." } else { "" };
        return Err(format!("{} file(s) were not copied:\n{}{more}{kept}", errors.len(), errors[..errors.len().min(5)].join("\n")));
    }
    if then_delete {
        for src in from {
            let r = if fs::symlink_metadata(src).is_ok_and(|m| m.is_dir()) { fs::remove_dir_all(src) } else { fs::remove_file(src) };
            r.map_err(|e| format!("Copied, but {} could not be removed: {e}", src.display()))?;
        }
    }
    Ok(())
}

const PAR_MAX: u64 = 64 << 20;

fn zip_name(rel: &Path) -> String {
    let s = rel.to_string_lossy();
    if s.contains('\\') { s.replace('\\', "/") } else { s.into_owned() }
}

fn zip_now(from: &[PathBuf], dest: &Path, p: &Progress) -> Result<(), String> {
    use crate::zipx::{self, Prepared, Writer};
    let mut entries = vec![];
    for src in from {
        let name = src.file_name().ok_or_else(|| format!("{}: no name", src.display()))?;
        walk(src, PathBuf::from(name), &mut entries).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    entries.retain(|e| e.0 != dest);
    let file = fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut w = Writer::new(std::io::BufWriter::with_capacity(1 << 20, file));
    let err = |e: &dyn std::fmt::Display, path: &Path| format!("{}: {e}", path.display());
    let (mut tf, mut tb) = (0u64, 0u64);
    let mut small = Vec::new();
    let mut big = Vec::new();
    for e in &entries {
        match e.2 {
            None => w.add_dir(&zip_name(&e.1), zipx::dos_time(&e.0)).map_err(|x| err(&x, &e.0))?,
            Some(sz) => {
                tf += 1;
                tb += sz;
                if sz <= PAR_MAX { small.push(e); } else { big.push(e); }
            }
        }
    }
    p.total_files.store(tf, Relaxed);
    p.total_bytes.store(tb, Relaxed);
    if !small.is_empty() {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(small.len());
        let next = AtomicU64::new(0);
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Prepared, String>>(threads * 2);
        std::thread::scope(|s| {
            for _ in 0..threads {
                let tx = tx.clone();
                let (small, next) = (&small, &next);
                s.spawn(move || {
                    while let Some(e) = small.get(next.fetch_add(1, Relaxed) as usize) {
                        if p.cancelled() {
                            break;
                        }
                        let r = zipx::prepare(zip_name(&e.1), &e.0).map_err(|x| format!("{}: {x}", e.0.display()));
                        if tx.send(r).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);
            for r in rx {
                if p.cancelled() {
                    return Err("Cancelled".to_string());
                }
                let f = r?;
                let n = f.uncompressed_size();
                w.add_prepared(f).map_err(|e| e.to_string())?;
                p.bytes.fetch_add(n, Relaxed);
                p.files.fetch_add(1, Relaxed);
            }
            Ok(())
        })?;
    }
    for e in big {
        w.add_stream(zip_name(&e.1), &e.0, |n| { p.bytes.fetch_add(n, Relaxed); !p.cancelled() }).map_err(|x| err(&x, &e.0))?;
        p.files.fetch_add(1, Relaxed);
    }
    w.finish().map_err(|e| err(&e, dest))?;
    Ok(())
}

pub fn compress(h: isize, paths: Vec<PathBuf>, dir: PathBuf) -> bool {
    let base = if paths.len() == 1 {
        paths[0].file_stem().or_else(|| paths[0].file_name()).unwrap_or_default().to_string_lossy().into_owned()
    } else {
        dir.file_name().unwrap_or_default().to_string_lossy().into_owned()
    };
    let stem = if base.is_empty() { "Archive".to_string() } else { base };
    let out_zip = free_name(&dir, &format!("{stem}.zip"), false);
    let what = format!("Compressing {}", out_zip.file_name().unwrap_or_default().to_string_lossy());
    spawn(h, what, move |p| zip_now(&paths, &out_zip, p).inspect_err(|_| _ = fs::remove_file(&out_zip)).or_else(|e| if p.cancelled() { Ok(()) } else { Err(e) }))
}

pub fn is_archive(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("7z"))
}

pub fn extract(h: isize, archives: Vec<PathBuf>, dir: PathBuf) -> bool {
    let what = format!("Extracting {}", archives.iter().map(|a| a.file_name().unwrap_or_default().to_string_lossy()).collect::<Vec<_>>().join(", "));
    spawn(h, what, move |p| {
        p.total_files.store(archives.len() as u64, Relaxed);
        for a in &archives {
            let stem = a.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let out = free_name(&dir, &stem, true);
            let r = extract_with(a, &out, p);
            if r.is_err() && p.cancelled() {
                let _ = fs::remove_dir_all(&out);
                return Ok(());
            }
            r.map_err(|e| format!("{}: {e}", a.display()))?;
            p.files.fetch_add(1, Relaxed);
        }
        Ok(())
    })
}

#[cfg(test)]
fn extract_one(archive: &Path, out: &Path) -> Result<(), String> {
    extract_with(archive, out, &Progress::new(String::new()))
}

fn extract_with(archive: &Path, out: &Path, p: &Progress) -> Result<(), String> {
    let is7z = archive.extension().is_some_and(|e| e.eq_ignore_ascii_case("7z"));
    if is7z {
        return sevenz_rust2::decompress_file(archive, out).map_err(|e| e.to_string());
    }
    crate::zipx::extract(archive, out, |t| _ = p.total_bytes.fetch_add(t, Relaxed), |n| { p.bytes.fetch_add(n, Relaxed); !p.cancelled() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rx-jobs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn tree(root: &Path) {
        fs::create_dir_all(root.join("src/a/b")).unwrap();
        fs::write(root.join("src/one.txt"), "hello").unwrap();
        fs::write(root.join("src/a/two.bin"), vec![7u8; 300_000]).unwrap();
        fs::write(root.join("src/a/b/empty"), "").unwrap();
    }

    #[test]
    fn free_names() {
        let d = tmp("names");
        fs::write(d.join("x.txt"), "").unwrap();
        fs::create_dir(d.join("dir.v2")).unwrap();
        assert_eq!(free_name(&d, "x.txt", false), d.join("x (2).txt"));
        assert_eq!(free_name(&d, "y.txt", false), d.join("y.txt"));
        assert_eq!(free_name(&d, "dir.v2", true), d.join("dir.v2 (2)"));
        assert_eq!(free_name(&d, ".gitignore", false), d.join(".gitignore"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copies_a_tree_and_moves_it() {
        let d = tmp("copy");
        tree(&d);
        fs::create_dir(d.join("dst")).unwrap();
        let p = Progress::new(String::new());
        copy_now(&[d.join("src")], &d.join("dst"), false, &p).unwrap();
        assert_eq!(fs::read_to_string(d.join("dst/src/one.txt")).unwrap(), "hello");
        assert_eq!(fs::read(d.join("dst/src/a/two.bin")).unwrap().len(), 300_000);
        assert!(d.join("dst/src/a/b/empty").is_file());
        assert_eq!((p.files.load(Relaxed), p.total_files.load(Relaxed)), (3, 3));
        assert_eq!(p.bytes.load(Relaxed), p.total_bytes.load(Relaxed));
        copy_now(&[d.join("src")], &d.join("dst"), true, &Progress::new(String::new())).unwrap();
        assert!(d.join("dst/src (2)/a/two.bin").is_file() && !d.join("src").exists());
        assert!(!can_fast_copy(&[d.join("dst")], &d.join("dst/src")), "into itself");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_round_trip() {
        let d = tmp("zip");
        tree(&d);
        let z = d.join("out.zip");
        zip_now(&[d.join("src")], &z, &Progress::new(String::new())).unwrap();
        extract_one(&z, &d.join("back")).unwrap();
        assert_eq!(fs::read_to_string(d.join("back/src/one.txt")).unwrap(), "hello");
        assert_eq!(fs::read(d.join("back/src/a/two.bin")).unwrap(), vec![7u8; 300_000]);
        assert!(d.join("back/src/a/b/empty").is_file());
        assert!(extract_one(&d.join("src/one.txt"), &d.join("bad")).is_err(), "not an archive");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_agrees_with_other_implementations() {
        use std::io::{Read, Write};
        let d = tmp("xzip");
        tree(&d);
        let z = d.join("ours.zip");
        zip_now(&[d.join("src")], &z, &Progress::new(String::new())).unwrap();
        let mut a = zip::ZipArchive::new(fs::File::open(&z).unwrap()).unwrap();
        let mut two = vec![];
        a.by_name("src/a/two.bin").unwrap().read_to_end(&mut two).unwrap();
        assert_eq!(two, vec![7u8; 300_000]);
        assert!(a.by_name("src/a/b/").is_ok() && a.by_name("src/one.txt").is_ok());
        let t = std::process::Command::new("tar").arg("-xf").arg(&z).arg("-C").arg(&d).current_dir(&d).status();
        if let Ok(t) = t {
            assert!(t.success(), "tar could not read our zip");
            assert_eq!(fs::read(d.join("src/a/two.bin")).unwrap().len(), 300_000);
        }
        let theirs = d.join("theirs.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&theirs).unwrap());
        let (def, sto) = (zip::write::SimpleFileOptions::default(), zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored));
        w.start_file("a/x.txt", def).unwrap();
        w.write_all(&b"hello hello hello ".repeat(5000)).unwrap();
        w.start_file("raw.bin", sto).unwrap();
        w.write_all(&[1, 2, 3]).unwrap();
        w.add_directory("empty/", def).unwrap();
        w.finish().unwrap();
        extract_one(&theirs, &d.join("theirs")).unwrap();
        assert_eq!(fs::read(d.join("theirs/a/x.txt")).unwrap().len(), 90_000);
        assert_eq!(fs::read(d.join("theirs/raw.bin")).unwrap(), [1, 2, 3]);
        assert!(d.join("theirs/empty").is_dir());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unpacks_7z() {
        let d = tmp("7z");
        tree(&d);
        let a = d.join("t.7z");
        sevenz_rust2::compress_to_path(d.join("src"), &a).unwrap();
        extract_one(&a, &d.join("back")).unwrap();
        let found: Vec<PathBuf> = {
            let mut v = vec![];
            walk(&d.join("back"), PathBuf::new(), &mut v).unwrap();
            v.into_iter().filter(|e| e.2.is_some()).map(|e| e.1).collect()
        };
        assert_eq!(found.len(), 3, "{found:?}");
        let two = found.iter().find(|p| p.ends_with("two.bin")).unwrap();
        assert_eq!(fs::read(d.join("back").join(two)).unwrap(), vec![7u8; 300_000]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    #[ignore]
    fn live_zip() {
        let Some(src) = std::env::var_os("RX_LIVE").map(PathBuf::from) else { return };
        let d = tmp("live");
        let z = d.join("live.zip");
        let t = Instant::now();
        zip_now(&[src.clone()], &z, &Progress::new(String::new())).unwrap();
        let mut files = vec![];
        walk(&src, PathBuf::new(), &mut files).unwrap();
        let raw: u64 = files.iter().filter_map(|e| e.2).sum();
        println!("LIVE packed {} files, {} -> {} bytes in {:?}", files.iter().filter(|e| e.2.is_some()).count(), raw, fs::metadata(&z).unwrap().len(), t.elapsed());
        let name = src.file_name().unwrap();
        let (a, b, c) = (d.join("tar"), d.join("ps"), d.join("ours"));
        for o in [&a, &b] {
            fs::create_dir_all(o).unwrap();
        }
        let t = Instant::now();
        assert!(std::process::Command::new("tar").arg("-xf").arg(&z).arg("-C").arg(&a).status().unwrap().success());
        println!("LIVE tar unpacked in {:?}", t.elapsed());
        let t = Instant::now();
        let ps = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &format!("Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force", z.display(), b.display())]).status().unwrap();
        println!("LIVE Expand-Archive {} in {:?}", ps.success(), t.elapsed());
        assert!(ps.success());
        let t = Instant::now();
        extract_one(&z, &c).unwrap();
        println!("LIVE ours unpacked in {:?}", t.elapsed());
        let mut bad = 0;
        for (path, rel, size) in &files {
            if size.is_none() {
                continue;
            }
            let want = fs::read(path).unwrap();
            for root in [&a, &b, &c] {
                let got = fs::read(root.join(name).join(rel.strip_prefix(name).unwrap_or(rel)));
                let got = got.or_else(|_| fs::read(root.join(rel)));
                if got.as_deref().ok() != Some(&want[..]) {
                    bad += 1;
                    println!("LIVE MISMATCH {} in {}", rel.display(), root.display());
                }
            }
        }
        println!("LIVE compared {} files x 3 unpackers, {bad} mismatches", files.iter().filter(|e| e.2.is_some()).count());
        let _ = fs::remove_dir_all(&d);
        assert_eq!(bad, 0);
    }

    #[test]
    #[ignore]
    fn bench() {
        let Some(root) = std::env::var_os("RX_BENCH").map(PathBuf::from) else { return };
        let small = root.join("small");
        fs::create_dir_all(&small).unwrap();
        for i in 0..5000 {
            fs::write(small.join(format!("f{i}.bin")), vec![(i % 251) as u8; 4096 + (i * 37) % 60_000]).unwrap();
        }
        let big = root.join("big");
        fs::create_dir_all(&big).unwrap();
        let chunk: Vec<u8> = (0..1u32 << 20).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let mut f = fs::File::create(big.join("1gb.bin")).unwrap();
        for _ in 0..1024 {
            std::io::Write::write_all(&mut f, &chunk).unwrap();
        }
        drop(f);
        super::super::sh::init();
        for src in [&small, &big] {
            let dst_old = root.join("out-old");
            let dst_new = root.join("out-new");
            let _ = fs::remove_dir_all(&dst_old);
            let _ = fs::remove_dir_all(&dst_new);
            fs::create_dir_all(&dst_old).unwrap();
            fs::create_dir_all(&dst_new).unwrap();
            let t = Instant::now();
            super::super::sh::fileop(std::ptr::null_mut(), super::super::sh::Op::Copy, &[src.clone()], &dst_old).unwrap();
            let old = t.elapsed();
            let t = Instant::now();
            copy_now(&[src.clone()], &dst_new, false, &Progress::new(String::new())).unwrap();
            let new = t.elapsed();
            println!("BENCH copy {}: IFileOperation {:.2} s, ours {:.2} s", src.file_name().unwrap().to_string_lossy(), old.as_secs_f64(), new.as_secs_f64());
        }
        let t = Instant::now();
        zip_now(&[small.clone()], &root.join("ours.zip"), &Progress::new(String::new())).unwrap();
        println!("BENCH zip small: ours {:.2} s", t.elapsed().as_secs_f64());
    }

    #[test]
    fn volumes_and_names() {
        assert!(same_volume(Path::new(r"C:\a"), Path::new(r"c:\b\c")));
        assert!(!same_volume(Path::new(r"C:\a"), Path::new(r"D:\a")));
        assert_eq!(zip_name(Path::new(r"a\b\c.txt")), "a/b/c.txt");
        assert!(is_archive(Path::new("x.ZIP")) && is_archive(Path::new("y.7z")) && !is_archive(Path::new("z.rar")));
    }
}
