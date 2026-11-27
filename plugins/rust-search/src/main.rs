use std::{
    env,
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    mem::{size_of, ManuallyDrop},
    os::windows::io::FromRawHandle,
    path::PathBuf,
    ptr,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
mod common;
use common::*;
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES},
    Storage::FileSystem::*,
    System::{IO::DeviceIoControl, Ioctl::*, Pipes::*},
};

#[derive(Clone, Copy, Default)]
#[repr(C, packed(2))]
struct Ent {
    parent: u32,
    off: u32,
    len: u16,
}

const _: () = assert!(size_of::<Ent>() == 10);

#[derive(Default)]
struct Index {
    ents: Vec<Ent>,
    arena: Vec<u8>,
    live: usize,
    garbage: usize,
}

const MAGIC: &[u8; 8] = b"RSIDX001";
const REC_MASK: u64 = (1 << 48) - 1;
const MAX_REC: usize = 1 << 28;

impl Index {
    fn new() -> Self {
        Self::default()
    }

    fn len(&self) -> usize {
        self.live
    }

    fn insert(&mut self, frn: u64, parent: u64, name: &str) {
        let rec = (frn & REC_MASK) as usize;
        if rec >= MAX_REC || name.len() > u16::MAX as usize {
            return;
        }
        if rec >= self.ents.len() {
            if rec >= self.ents.capacity() {
                self.ents.reserve_exact(rec + 1 - self.ents.len() + (1 << 16));
            }
            self.ents.resize(rec + 1, Ent::default());
        }
        let old = self.ents[rec].len as usize;
        if old > 0 {
            self.garbage += old;
        } else {
            self.live += 1;
        }
        if self.arena.capacity() - self.arena.len() < name.len() {
            self.arena.reserve_exact(1 << 20);
        }
        let off = self.arena.len() as u32;
        self.arena.extend_from_slice(name.as_bytes());
        self.ents[rec] = Ent { parent: (parent & REC_MASK) as u32, off, len: name.len() as u16 };
        if self.garbage > (2 << 20) && self.garbage * 8 > self.arena.len() {
            self.compact();
        }
    }

    fn remove(&mut self, frn: u64) {
        if let Some(e) = self.ents.get_mut((frn & REC_MASK) as usize) {
            if e.len > 0 {
                self.garbage += e.len as usize;
                e.len = 0;
                self.live -= 1;
            }
        }
    }

    fn name(&self, e: &Ent) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.arena[e.off as usize..e.off as usize + e.len as usize]) }
    }

    fn get(&self, rec: u32) -> Option<(u32, &str)> {
        self.ents.get(rec as usize).filter(|e| e.len > 0).map(|e| (e.parent, self.name(e)))
    }

    fn par_filter_map<T: Send>(&self, f: impl Fn(u32, &str) -> Option<T> + Sync) -> Vec<T> {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
        let chunk = self.ents.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let f = &f;
            let jobs: Vec<_> = self
                .ents
                .chunks(chunk)
                .enumerate()
                .map(|(ci, c)| {
                    s.spawn(move || {
                        let live = c.iter().enumerate().filter(|(_, e)| e.len > 0);
                        live.filter_map(|(i, e)| f((ci * chunk + i) as u32, self.name(e))).collect::<Vec<T>>()
                    })
                })
                .collect();
            jobs.into_iter().flat_map(|j| j.join().unwrap()).collect()
        })
    }

    fn compact(&mut self) {
        let mut arena = Vec::with_capacity(self.arena.len() - self.garbage);
        for e in self.ents.iter_mut().filter(|e| e.len > 0) {
            let s = e.off as usize;
            let off = arena.len() as u32;
            arena.extend_from_slice(&self.arena[s..s + e.len as usize]);
            e.off = off;
        }
        self.arena = arena;
        self.garbage = 0;
    }

    fn write_to(&self, w: &mut impl Write, jid: u64, usn: i64) -> io::Result<()> {
        w.write_all(MAGIC)?;
        for v in [jid, usn as u64, self.ents.len() as u64, self.arena.len() as u64, self.live as u64] {
            w.write_all(&v.to_le_bytes())?;
        }
        for e in &self.ents {
            w.write_all(&e.parent.to_le_bytes())?;
            w.write_all(&e.off.to_le_bytes())?;
            w.write_all(&e.len.to_le_bytes())?;
            w.write_all(&[0, 0])?;
        }
        w.write_all(&self.arena)
    }

    fn read_from(b: &[u8]) -> Option<(Index, u64, i64)> {
        const HEADER: usize = 8 + 5 * 8;
        if b.len() < HEADER || &b[..8] != MAGIC {
            return None;
        }
        let num = |i: usize| u64::from_le_bytes(b[8 + i * 8..16 + i * 8].try_into().unwrap());
        let (jid, usn, n_ents, arena_len, live) = (num(0), num(1) as i64, num(2) as usize, num(3) as usize, num(4) as usize);
        if n_ents > MAX_REC || b.len() != HEADER + n_ents * 12 + arena_len {
            return None;
        }
        let arena = b[HEADER + n_ents * 12..].to_vec();
        let (mut ents, mut seen, mut used) = (Vec::with_capacity(n_ents), 0usize, 0usize);
        for c in b[HEADER..HEADER + n_ents * 12].chunks_exact(12) {
            let e = Ent { parent: u32::from_le_bytes(c[0..4].try_into().unwrap()), off: u32::from_le_bytes(c[4..8].try_into().unwrap()), len: u16::from_le_bytes(c[8..10].try_into().unwrap()) };
            if e.len > 0 {
                let name = arena.get(e.off as usize..e.off as usize + e.len as usize)?;
                std::str::from_utf8(name).ok()?;
                seen += 1;
                used += e.len as usize;
            }
            ents.push(e);
        }
        (seen == live).then(|| (Index { ents, arena, live, garbage: arena_len.saturating_sub(used) }, jid, usn))
    }

    fn shrink(&mut self) {
        self.compact();
        self.ents.shrink_to_fit();
        self.arena.shrink_to_fit();
    }
}
const SHOW: usize = 100;

fn open_volume(drive: char) -> Result<HANDLE, String> {
    let path: Vec<u16> = format!("\\\\.\\{drive}:").encode_utf16().chain([0]).collect();
    let h = unsafe {
        CreateFileW(path.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, ptr::null(), OPEN_EXISTING, 0, ptr::null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(format!(
            "cannot open {drive}: (error {}). Run from an elevated (admin) shell; volume must be NTFS.",
            unsafe { GetLastError() }
        ));
    }
    Ok(h)
}

fn records(b: &[u8], buf: &mut String, mut f: impl FnMut(u64, u64, u32, &str)) {
    let mut off = 8;
    while off + 60 <= b.len() {
        let r = &b[off..];
        let len = u32::from_le_bytes(r[0..4].try_into().unwrap()) as usize;
        if len == 0 || u16::from_le_bytes(r[4..6].try_into().unwrap()) != 2 {
            break;
        }
        let frn = u64::from_le_bytes(r[8..16].try_into().unwrap());
        let parent = u64::from_le_bytes(r[16..24].try_into().unwrap());
        let reason = u32::from_le_bytes(r[40..44].try_into().unwrap());
        let nlen = u16::from_le_bytes(r[56..58].try_into().unwrap()) as usize;
        let noff = u16::from_le_bytes(r[58..60].try_into().unwrap()) as usize;
        buf.clear();
        for c in char::decode_utf16(r[noff..noff + nlen].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]))) {
            buf.push(c.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        f(frn, parent, reason, buf);
        off += len;
    }
}

fn scan(h: HANDLE) -> Result<Index, String> {
    let mut idx = Index::new();
    let mut med = MFT_ENUM_DATA_V0 { StartFileReferenceNumber: 0, LowUsn: 0, HighUsn: i64::MAX };
    let mut buf = vec![0u8; 1 << 20];
    let mut name = String::with_capacity(260);
    loop {
        let mut n = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                h,
                FSCTL_ENUM_USN_DATA,
                &med as *const _ as _,
                size_of::<MFT_ENUM_DATA_V0>() as u32,
                buf.as_mut_ptr() as _,
                buf.len() as u32,
                &mut n,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let e = unsafe { GetLastError() };
            return if e == ERROR_HANDLE_EOF {
                idx.shrink();
                Ok(idx)
            } else { Err(format!("FSCTL_ENUM_USN_DATA failed (error {e})")) };
        }
        let b = &buf[..n as usize];
        med.StartFileReferenceNumber = u64::from_le_bytes(b[..8].try_into().unwrap());
        records(b, &mut name, |frn, parent, _, name| {
            idx.insert(frn, parent, name);
        });
    }
}

fn journal(h: HANDLE) -> Result<USN_JOURNAL_DATA_V0, String> {
    let mut j: USN_JOURNAL_DATA_V0 = unsafe { std::mem::zeroed() };
    let mut n = 0u32;
    let ok = unsafe {
        DeviceIoControl(h, FSCTL_QUERY_USN_JOURNAL, ptr::null(), 0, &mut j as *mut _ as _, size_of::<USN_JOURNAL_DATA_V0>() as u32, &mut n, ptr::null_mut())
    };
    if ok == 0 {
        return Err(format!("USN journal not available (error {})", unsafe { GetLastError() }));
    }
    Ok(j)
}

struct Shared {
    idx: Index,
    jid: u64,
    usn: i64,
    dirty: bool,
}

fn tail(drive: char, sh: &Mutex<Shared>) {
    let h = match open_volume(drive) {
        Ok(h) => h,
        Err(e) => return eprintln!("{e}"),
    };
    let mask = USN_REASON_FILE_CREATE | USN_REASON_FILE_DELETE | USN_REASON_RENAME_NEW_NAME;
    let mut buf = vec![0u8; 1 << 16];
    let (jid, mut usn) = {
        let g = sh.lock().unwrap();
        (g.jid, g.usn)
    };
    let mut name = String::with_capacity(260);
    loop {
        let rd = READ_USN_JOURNAL_DATA_V0 { StartUsn: usn, ReasonMask: mask, ReturnOnlyOnClose: 0, Timeout: 1, BytesToWaitFor: 1, UsnJournalID: jid };
        let mut n = 0u32;
        let ok = unsafe {
            DeviceIoControl(h, FSCTL_READ_USN_JOURNAL, &rd as *const _ as _, size_of::<READ_USN_JOURNAL_DATA_V0>() as u32, buf.as_mut_ptr() as _, buf.len() as u32, &mut n, ptr::null_mut())
        };
        if ok == 0 {
            eprintln!("journal read failed (error {}); rebuilding the index", unsafe { GetLastError() });
            unsafe { CloseHandle(h) };
            return;
        }
        let b = &buf[..n as usize];
        usn = i64::from_le_bytes(b[..8].try_into().unwrap());
        let mut g = sh.lock().unwrap();
        if b.len() > 8 {
            records(b, &mut name, |frn, parent, reason, name| {
                if reason & USN_REASON_FILE_DELETE != 0 {
                    g.idx.remove(frn);
                } else {
                    g.idx.insert(frn, parent, name);
                }
            });
            g.dirty = true;
        }
        g.usn = usn;
    }
}

fn maintain(drive: char, sh: Arc<Mutex<Shared>>) {
    loop {
        tail(drive, &sh);
        thread::sleep(Duration::from_secs(10));
        loop {
            match open_volume(drive).and_then(|h| {
                let r = journal(h).and_then(|j| scan(h).map(|idx| (idx, j)));
                unsafe { CloseHandle(h) };
                r
            }) {
                Ok((idx, j)) => {
                    *sh.lock().unwrap() = Shared { idx, jid: j.UsnJournalID, usn: j.NextUsn, dirty: true };
                    eprintln!("index rebuilt");
                    break;
                }
                Err(e) => {
                    eprintln!("rebuild failed: {e}; retrying in 30s");
                    thread::sleep(Duration::from_secs(30));
                }
            }
        }
    }
}

fn index_path(drive: char) -> Option<PathBuf> {
    Some(PathBuf::from(env::var_os("LOCALAPPDATA")?).join("rs-search").join(format!("index-{drive}.bin")))
}

fn load_saved(drive: char, j: &USN_JOURNAL_DATA_V0) -> Option<(Index, i64)> {
    let (idx, jid, usn) = Index::read_from(&fs::read(index_path(drive)?).ok()?)?;
    (jid == j.UsnJournalID && usn >= j.FirstUsn && usn <= j.NextUsn).then_some((idx, usn))
}

fn save(drive: char, sh: &Mutex<Shared>) {
    let Some(path) = index_path(drive) else { return };
    let mut g = sh.lock().unwrap();
    if !g.dirty {
        return;
    }
    let tmp = path.with_extension("tmp");
    let r = fs::create_dir_all(path.parent().unwrap()).and_then(|_| {
        let mut w = BufWriter::new(File::create(&tmp)?);
        g.idx.write_to(&mut w, g.jid, g.usn)?;
        w.flush()?;
        fs::rename(&tmp, &path)
    });
    match r {
        Ok(()) => g.dirty = false,
        Err(e) => eprintln!("cannot save index to {}: {e}", path.display()),
    }
}

const SAVE_EVERY: Duration = Duration::from_secs(600);

fn full_path(idx: &Index, drive: char, mut rec: u32) -> String {
    let mut parts = Vec::new();
    while let Some((parent, name)) = idx.get(rec) {
        parts.push(name);
        rec = parent;
        if parts.len() > 256 {
            break;
        }
    }
    parts.reverse();
    format!("{drive}:\\{}", parts.join("\\"))
}

fn contains_ci(name: &str, t: &str) -> bool {
    if name.is_ascii() && t.is_ascii() {
        let (h, n) = (name.as_bytes(), t.as_bytes());
        if n.is_empty() {
            return true;
        }
        if n.len() > h.len() {
            return false;
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
            return true;
        }
        false
    } else {
        name.to_lowercase().contains(t)
    }
}

fn ends_with_ci_ascii(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len() && s.as_bytes()[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

fn query(idx: &Index, drive: char, terms: &[String]) -> (Vec<String>, usize) {
    let t0 = &terms[0];
    let t0_ascii = t0.is_ascii();
    let mut hits: Vec<(u8, u8, usize, u32)> = idx.par_filter_map(|rec, n| {
        if !terms.iter().all(|t| contains_ci(n, t)) {
            return None;
        }
        let rank = if n.is_ascii() && t0_ascii {
            let (nb, tb) = (n.as_bytes(), t0.as_bytes());
            if nb.eq_ignore_ascii_case(tb) { 0 } else if nb.len() >= tb.len() && nb[..tb.len()].eq_ignore_ascii_case(tb) { 1 } else { 2 }
        } else {
            let low = n.to_lowercase();
            if low == *t0 { 0 } else if low.starts_with(t0) { 1 } else { 2 }
        };
        Some((!(ends_with_ci_ascii(n, ".lnk") || ends_with_ci_ascii(n, ".exe")) as u8, rank, n.len(), rec))
    });
    let total = hits.len();
    const CAND: usize = 1000;
    if hits.len() > CAND {
        hits.select_nth_unstable(CAND - 1);
        hits.truncate(CAND);
    }
    let mut paths: Vec<(u8, u8, u8, usize, String)> = hits
        .into_iter()
        .map(|(_, rank, _, f)| {
            let p = full_path(idx, drive, f);
            let l = p.to_lowercase();
            let tier = if l.ends_with(".lnk") { if l.contains(START_MENU) { 0 } else { 1 } } else if l.ends_with(".exe") { 2 } else { 3 };
            let junk = JUNK.iter().any(|j| l.contains(j)) as u8;
            (tier, rank, junk, p.len(), p)
        })
        .collect();
    paths.sort_unstable();
    (paths.into_iter().take(SHOW).map(|t| t.4).collect(), total)
}

const START_MENU: &str = "\\start menu\\programs\\";

const JUNK: [&str; 2] = ["\\winsxs\\", "\\windowsapps\\"];

fn render(idx: &Index, drive: char, terms: &[String]) -> String {
    let (paths, total) = query(idx, drive, terms);
    let mut out = paths.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    if total > SHOW {
        out += &format!("... {} more\n", total - SHOW);
    }
    out
}

fn pipe_security(allow: Option<&str>) -> Result<Option<SECURITY_ATTRIBUTES>, String> {
    let Some(sid) = allow else { return Ok(None) };
    let sddl: Vec<u16> = format!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{sid})").encode_utf16().chain([0]).collect();
    let mut sd = ptr::null_mut();
    let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, ptr::null_mut()) };
    if ok == 0 {
        return Err(format!("bad --allow SID (error {})", unsafe { GetLastError() }));
    }
    Ok(Some(SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 }))
}

fn serve(drive: char, allow: Option<String>) -> Result<(), String> {
    let sa = pipe_security(allow.as_deref())?;
    let h = open_volume(drive)?;
    let j = journal(h)?;
    let t = Instant::now();
    let (idx, usn, how) = match load_saved(drive, &j) {
        Some((idx, usn)) => (idx, usn, "loaded"),
        None => (scan(h)?, j.NextUsn, "scanned"),
    };
    unsafe { CloseHandle(h) };
    eprintln!("{how} {} entries in {:?}, serving {}", idx.len(), t.elapsed(), pipe_name(drive));
    let idx = Arc::new(Mutex::new(Shared { idx, jid: j.UsnJournalID, usn, dirty: how == "scanned" }));

    let sh = idx.clone();
    thread::spawn(move || maintain(drive, sh));
    let sh = idx.clone();
    thread::spawn(move || loop {
        save(drive, &sh);
        thread::sleep(SAVE_EVERY);
    });

    let name: Vec<u16> = pipe_name(drive).encode_utf16().chain([0]).collect();
    let pipe = unsafe {
        CreateNamedPipeW(name.as_ptr(), PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_BYTE | PIPE_WAIT, 1, 1 << 16, 1 << 16, 0, sa.as_ref().map_or(ptr::null(), |s| s as *const _))
    };
    if let Some(s) = sa {
        if !s.lpSecurityDescriptor.is_null() {
            unsafe { LocalFree(s.lpSecurityDescriptor as _) };
        }
    }
    if pipe == INVALID_HANDLE_VALUE {
        return Err(format!("cannot create pipe (error {}); is a server already running?", unsafe { GetLastError() }));
    }
    loop {
        let ok = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) };
        if ok != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
            let mut f = ManuallyDrop::new(unsafe { File::from_raw_handle(pipe as _) });
            let mut buf = [0u8; 4096];
            if let Ok(n) = f.read(&mut buf) {
                let terms = split_terms(&String::from_utf8_lossy(&buf[..n]));
                if !terms.is_empty() {
                    let out = render(&idx.lock().unwrap().idx, drive, &terms);
                    let _ = f.write_all(out.as_bytes());
                    let _ = f.sync_all();
                }
            }
        }
        unsafe { DisconnectNamedPipe(pipe) };
    }
}

fn bench(drive: char, args: &[String]) -> Result<(), String> {
    let terms = split_terms(&args.join(" "));
    if terms.is_empty() {
        return Err("usage: rs --bench <term>".into());
    }
    let h = open_volume(drive)?;
    let idx = scan(h);
    unsafe { CloseHandle(h) };
    let idx = idx?;
    for _ in 0..5 {
        let t = Instant::now();
        let (_, total) = query(&idx, drive, &terms);
        println!("query {:?} -> {total} hits in {:?}", terms, t.elapsed());
    }
    Ok(())
}

const TASK: &str = "rs-search";

fn ps(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let o = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("cannot run powershell: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).into_owned())
    }
}

fn install_dir() -> std::path::PathBuf {
    std::path::Path::new(&env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into())).join("rs-search")
}

fn install() -> Result<(), String> {
    let (sid, who) = match env::var(winrust_common::reg::SID_ENV).ok().filter(|s| winrust_common::reg::valid_sid(s)) {
        Some(sid) => (sid, String::new()),
        None => {
            let who = ps("whoami /user /fo csv /nh")?;
            (who.trim().rsplit('"').nth(1).unwrap_or("").to_string(), who)
        }
    };
    if !winrust_common::reg::valid_sid(&sid) {
        return Err(format!("cannot determine your SID from: {who}"));
    }
    let drives = fixed_ntfs_drives();
    if drives.is_empty() {
        return Err("no fixed NTFS drives found".into());
    }
    let _ = ps(&format!("Get-ScheduledTask -TaskName '{TASK}*' -ErrorAction SilentlyContinue | Stop-ScheduledTask; Start-Sleep 1"));
    let dir = install_dir();
    let dst = dir.join("rs.exe");
    let me = env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e} (run as admin)", dir.display()))?;
    if me.canonicalize().ok() != dst.canonicalize().ok() {
        std::fs::copy(&me, &dst).map_err(|e| format!("copy to {}: {e}", dst.display()))?;
    }
    let list = drives.iter().map(|d| format!("'{d}'")).collect::<Vec<_>>().join(",");
    ps(&format!(
        "Unregister-ScheduledTask -TaskName {TASK} -Confirm:$false -ErrorAction SilentlyContinue; \
         $t = New-ScheduledTaskTrigger -AtStartup; \
         $s = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -Priority 4 -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1); \
         foreach ($d in {list}) {{ \
           $a = New-ScheduledTaskAction -Execute '{exe}' -Argument \"-d $d --serve --allow {sid}\"; \
           Register-ScheduledTask -TaskName \"{TASK}-$d\" -Action $a -Trigger $t -Settings $s -User SYSTEM -RunLevel Highest -Force | Out-Null; \
           Start-ScheduledTask -TaskName \"{TASK}-$d\" \
         }}",
        exe = dst.display()
    ))?;
    let names: Vec<String> = drives.iter().map(|d| format!("{d}:")).collect();
    println!("installed to {} for {} and started; runs at every boot. Query with: rs <term>", dst.display(), names.join(" "));
    Ok(())
}

fn uninstall() -> Result<(), String> {
    ps(&format!("Get-ScheduledTask -TaskName '{TASK}*' -ErrorAction SilentlyContinue | ForEach-Object {{ Stop-ScheduledTask -InputObject $_; Unregister-ScheduledTask -InputObject $_ -Confirm:$false }}; Start-Sleep 1"))?;
    let _ = std::fs::remove_dir_all(install_dir());
    let _ = std::fs::remove_dir_all(PathBuf::from(env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\config\systemprofile\AppData\Local\rs-search"));
    println!("uninstalled");
    Ok(())
}

fn split_terms(s: &str) -> Vec<String> {
    s.split_whitespace().map(|t| t.to_lowercase()).collect()
}

fn serve_log(drive: char) {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
    if !env::current_exe().is_ok_and(|e| e.starts_with(install_dir())) {
        return;
    }
    let dir = PathBuf::from(env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into())).join("WinRust");
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join(format!("serve-{drive}.log"));
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::remove_file(&p);
    }
    let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) else { return };
    unsafe { SetStdHandle(STD_ERROR_HANDLE, f.into_raw_handle() as _) };
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    eprintln!("[{t}] serve {drive}: start");
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let (mut drive, mut explicit) = ('C', None);
    if args.first().map(|a| a == "-d").unwrap_or(false) && args.len() >= 2 {
        drive = args[1].chars().next().unwrap_or('C').to_ascii_uppercase();
        explicit = Some(drive);
        args.drain(..2);
    }
    let cmd = args.first().map(String::as_str);
    if cmd == Some("--serve") {
        serve_log(drive);
    }
    if matches!(cmd, Some("--serve" | "--install" | "--uninstall" | "--bench")) {
        let allow = args.iter().position(|a| a == "--allow").and_then(|i| args.get(i + 1)).cloned();
        let r = match cmd {
            Some("--serve") => serve(drive, allow),
            Some("--install") => install(),
            Some("--bench") => bench(drive, &args[1..]),
            _ => uninstall(),
        };
        if let Err(e) = r {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    let q = args.join(" ");
    let terms = split_terms(&q);
    if terms.is_empty() {
        eprintln!("usage: rs [-d X] <term>...   |   rs [-d X] --serve");
        std::process::exit(2);
    }

    let mut answered = false;
    for d in explicit.map_or_else(drive_letters, |d| vec![d]) {
        match ask(d, &q) {
            Some(Ok(out)) => {
                answered = true;
                print!("{out}");
            }
            Some(Err(e)) => {
                eprintln!("cannot reach the {d}: service: {e}");
                std::process::exit(1);
            }
            None => {}
        }
    }
    if answered {
        return;
    }
    for d in explicit.map_or_else(fixed_ntfs_drives, |d| vec![d]) {
        let t = Instant::now();
        let idx = open_volume(d).and_then(scan).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
        eprintln!("no server running; indexed {} entries on {d}: in {:?}", idx.len(), t.elapsed());
        print!("{}", render(&idx, d, &terms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Index {
        let mut idx = Index::new();
        idx.insert(5, 0, "Users");
        idx.insert(6, 5, "Docs");
        idx.insert(7, 6, "Report Final.TXT");
        idx.insert(8, 6, "report");
        idx.insert(10, 0, "WinSxS");
        idx.insert(11, 10, "hosts");
        idx.insert(12, 6, "hosts");
        idx
    }

    #[test]
    fn frn_sequence_bits_are_ignored_and_delete_rename_work() {
        let mut idx = sample();
        idx.insert((3 << 48) | 13, (9 << 48) | 6, "new.txt");
        assert_eq!(full_path(&idx, 'C', 13), "C:\\Users\\Docs\\new.txt");
        idx.insert(13, 6, "renamed.txt");
        idx.remove((3 << 48) | 7);
        assert_eq!(full_path(&idx, 'C', 13), "C:\\Users\\Docs\\renamed.txt");
        assert_eq!(query(&idx, 'C', &["report".into()]).1, 1);
        assert_eq!(idx.len(), 7);
    }

    fn snapshot(idx: &Index) -> Vec<u8> {
        let mut b = Vec::new();
        idx.write_to(&mut b, 42, 1000).unwrap();
        b
    }

    #[test]
    fn snapshot_roundtrips_including_garbage_and_deletes() {
        let mut idx = sample();
        idx.insert(13, 6, "old name");
        idx.insert(13, 6, "new name");
        idx.remove(7);
        let (back, jid, usn) = Index::read_from(&snapshot(&idx)).unwrap();
        assert_eq!((jid, usn, back.len(), back.garbage), (42, 1000, idx.len(), idx.garbage));
        assert_eq!(full_path(&back, 'C', 13), "C:\\Users\\Docs\\new name");
        assert_eq!(query(&back, 'C', &["report".into()]).1, 1);
    }

    #[test]
    fn corrupt_snapshots_are_rejected_not_trusted() {
        let good = snapshot(&sample());
        assert!(Index::read_from(&good).is_some());
        assert!(Index::read_from(&good[..good.len() - 1]).is_none(), "truncated");
        assert!(Index::read_from(&[good.clone(), vec![0]].concat()).is_none(), "trailing bytes");
        let mut bad_magic = good.clone();
        bad_magic[0] ^= 0xff;
        assert!(Index::read_from(&bad_magic).is_none(), "magic");
        let mut bad_utf8 = good.clone();
        *bad_utf8.last_mut().unwrap() = 0xff;
        assert!(Index::read_from(&bad_utf8).is_none(), "utf-8");
        let mut bad_count = good.clone();
        bad_count[8 + 4 * 8] ^= 1;
        assert!(Index::read_from(&bad_count).is_none(), "live count");
        assert!(Index::read_from(&[]).is_none());
    }

    #[test]
    fn start_menu_shortcuts_come_first() {
        let mut idx = sample();
        idx.insert(30, 0, "ProgramData");
        idx.insert(31, 30, "Start Menu");
        idx.insert(32, 31, "Programs");
        idx.insert(33, 32, "My Editor.lnk");
        idx.insert(34, 6, "editor");
        idx.insert(35, 6, "My Editor.lnk");
        let (p, _) = query(&idx, 'C', &["editor".into()]);
        assert_eq!(p[0], "C:\\ProgramData\\Start Menu\\Programs\\My Editor.lnk");
        assert!(p[1].ends_with("My Editor.lnk") && !p[1].contains("Start Menu"), "other shortcuts come right after the apps");
        assert_eq!(p[2], "C:\\Users\\Docs\\editor");
    }

    #[test]
    fn contains_ci_handles_ascii_and_unicode() {
        assert!(contains_ci("Report Final.TXT", "final"));
        assert!(!contains_ci("abc", "abcd"));
        assert!(contains_ci("ÄPFEL.txt", "äpfel"));
        assert!(contains_ci("anything", ""));
    }

    #[test]
    #[ignore]
    fn bench_query_over_1_5m_names() {
        let mut idx = Index::new();
        for i in 1..1_500_000u64 {
            idx.insert(i, 5, &format!("Some_File_Name_{i}.dat"));
        }
        let t = std::time::Instant::now();
        let (_, total) = query(&idx, 'C', &["name_77".into()]);
        eprintln!("query over {} names: {:?}, {total} hits", idx.len(), t.elapsed());
    }

    #[test]
    #[ignore]
    fn bench_snapshot_of_1_5m_names() {
        let mut idx = Index::new();
        for i in 1..1_500_000u64 {
            idx.insert(i, 5, &format!("Some_File_Name_{i}.dat"));
        }
        let (t, b) = (Instant::now(), snapshot(&idx));
        let w = t.elapsed();
        let t = Instant::now();
        assert!(Index::read_from(&b).is_some());
        eprintln!("snapshot {} MB: write {w:?}, load+validate {:?}", b.len() >> 20, t.elapsed());
    }

    #[test]
    fn compaction_keeps_live_names() {
        let mut idx = sample();
        let big = "x".repeat(1000);
        for _ in 0..9000 {
            idx.insert(20, 6, &big);
        }
        assert!(idx.arena.len() < 4 << 20, "arena was not compacted: {}", idx.arena.len());
        assert_eq!(idx.get(20).unwrap().1, big);
        assert_eq!(full_path(&idx, 'C', 12), "C:\\Users\\Docs\\hosts");
    }

    #[test]
    fn os_component_stores_rank_last_within_a_tie() {
        let (p, total) = query(&sample(), 'C', &["hosts".into()]);
        assert_eq!(total, 2);
        assert_eq!(p, vec!["C:\\Users\\Docs\\hosts", "C:\\WinSxS\\hosts"]);
    }

    #[test]
    fn exe_files_rank_first() {
        let mut idx = sample();
        idx.insert(20, 6, "tool.txt");
        idx.insert(21, 0, "tooling.exe");
        let (p, _) = query(&idx, 'C', &["tool".into()]);
        assert_eq!(p[0], "C:\\tooling.exe");
    }

    #[test]
    fn resolves_paths() {
        assert_eq!(full_path(&sample(), 'C', 7), "C:\\Users\\Docs\\Report Final.TXT");
    }

    #[test]
    fn ranks_exact_then_prefix_and_requires_all_terms() {
        let idx = sample();
        let (p, total) = query(&idx, 'C', &["report".into()]);
        assert_eq!(total, 2);
        assert_eq!(p[0], "C:\\Users\\Docs\\report");
        assert_eq!(p[1], "C:\\Users\\Docs\\Report Final.TXT");
        assert_eq!(query(&idx, 'C', &["report".into(), "txt".into()]).1, 1);
        assert_eq!(query(&idx, 'C', &["nope".into()]).1, 0);
    }
}
