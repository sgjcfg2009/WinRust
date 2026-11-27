#![windows_subsystem = "windows"]
use std::{cell::RefCell, env, fs, os::windows::fs::MetadataExt, path::{Path, PathBuf}, ptr, sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed}};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::{GetStockObject, InvalidateRect}, System::LibraryLoader::GetModuleHandleW, UI::{Controls::*, Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL}, Shell::*, WindowsAndMessaging::*}};

#[allow(dead_code)]
mod zipx;
#[allow(dead_code)]
mod jobs;
mod shellwin;
mod ui;

static DESKTOP: AtomicBool = AtomicBool::new(false);
fn desk() -> bool { DESKTOP.load(Relaxed) }
const CLR_NONE: isize = 0xFFFF_FFFF;
thread_local! { static WALL: std::cell::Cell<(isize, i32, i32)> = const { std::cell::Cell::new((0, 0, 0)) }; }
fn virt() -> (i32, i32, i32, i32) {
    unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) }
}
fn place_desk_list(list: HWND) {
    let (vx, vy, ..) = virt();
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    unsafe { MoveWindow(list, -vx, -vy + desk_top(), sw, (sh - desk_top() - desk_bottom()).max(0), 1) };
}
fn reload_wall() {
    let (vx, vy, cw, ch) = virt();
    let old = WALL.replace((wall::load(vx, vy, cw, ch), cw, ch)).0;
    if old != 0 { unsafe { windows_sys::Win32::Graphics::Gdi::DeleteObject(old as _) }; }
}
fn place_file() -> PathBuf {
    PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join(r"rust-explorer\window.txt")
}
fn parse_place(s: &str) -> Option<(bool, RECT)> {
    let mut it = s.split_whitespace();
    let max = match it.next()? { "max" => true, "normal" => false, _ => return None };
    let n: Vec<i32> = it.map(str::parse).collect::<Result<_, _>>().ok()?;
    let [left, top, right, bottom] = n[..] else { return None };
    (right - left >= 200 && bottom - top >= 150).then_some((max, RECT { left, top, right, bottom }))
}
fn save_place(h: HWND) {
    let mut wp: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    wp.length = size_of::<WINDOWPLACEMENT>() as u32;
    if unsafe { GetWindowPlacement(h, &mut wp) } == 0 { return; }
    let r = wp.rcNormalPosition;
    let f = place_file();
    let _ = fs::create_dir_all(f.parent().unwrap_or(&f));
    let _ = fs::write(f, format!("{} {} {} {} {}", if wp.showCmd == SW_SHOWMAXIMIZED as u32 { "max" } else { "normal" }, r.left, r.top, r.right, r.bottom));
}
fn saved_place() -> Option<(bool, RECT)> {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect};
    let (max, mut r) = parse_place(&fs::read_to_string(place_file()).ok()?)?;
    unsafe {
        let mon = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) != 0 {
            let wa = mi.rcWork;
            let w = (r.right - r.left).clamp(200, (wa.right - wa.left).max(200));
            let h = (r.bottom - r.top).clamp(150, (wa.bottom - wa.top).max(150));
            r.left = r.left.clamp(wa.left, (wa.right - w).max(wa.left));
            r.top = r.top.clamp(wa.top, (wa.bottom - h).max(wa.top));
            r.right = r.left + w;
            r.bottom = r.top + h;
        }
    }
    Some((max, r))
}
fn work_strips() -> (i32, i32) {
    let mut r: RECT = unsafe { std::mem::zeroed() };
    if unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut _ as *mut _, 0) } == 0 { return (0, 0); }
    (r.top.max(0), (unsafe { GetSystemMetrics(SM_CYSCREEN) } - r.bottom).max(0))
}
fn desk_top() -> i32 { work_strips().0 + ui::sc(8) }
fn desk_bottom() -> i32 {
    let reserved = work_strips().1;
    if reserved > 0 { return reserved + ui::sc(4); }
    let bar_up = unsafe { !FindWindowW(w("topbar").as_ptr(), ptr::null()).is_null() };
    if bar_up && winrust_common::style::style() == winrust_common::style::Style::Dock { ui::sc(52) } else { ui::sc(8) }
}
const THUMB: i32 = 96;
const WM_THUMB: u32 = WM_APP + 2;
static GEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static TX: std::sync::OnceLock<std::sync::mpsc::Sender<(u32, usize, PathBuf)>> = std::sync::OnceLock::new();
const WM_GO: u32 = WM_APP + 1;
static EDIT_OLD: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

struct Row { name: String, dir: bool, size: u64, time: u64, icon: i32, thumb: i32, hdr: bool }

#[derive(Default)]
struct Hist { v: Vec<PathBuf>, pos: usize }
impl Hist {
    fn visit(&mut self, p: PathBuf) {
        if self.v.get(self.pos) == Some(&p) { return; }
        self.v.truncate(self.pos + usize::from(!self.v.is_empty()));
        self.v.push(p);
        self.pos = self.v.len() - 1;
    }
    fn back(&mut self) -> Option<PathBuf> { if self.pos > 0 { self.pos -= 1; Some(self.v[self.pos].clone()) } else { None } }
    fn forward(&mut self) -> Option<PathBuf> { if self.pos + 1 < self.v.len() { self.pos += 1; Some(self.v[self.pos].clone()) } else { None } }
}

#[derive(Clone)]
enum UndoOp {
    Rename { from: PathBuf, to: PathBuf },
    New { path: PathBuf, dir: bool },
}

struct State { list: HWND, tab: HWND, edit: HWND, dir: PathBuf, rows: Vec<Row>, thumbs: bool, himl: isize, tabs: Vec<PathBuf>, active: usize, hist: Hist }
thread_local! {
    static ST: RefCell<State> = RefCell::new(State { list: 0 as _, tab: 0 as _, edit: 0 as _, dir: PathBuf::new(), rows: vec![], thumbs: false, himl: 0, tabs: vec![], active: 0, hist: Hist { v: vec![], pos: 0 } });
    static UNDO: RefCell<Vec<UndoOp>> = const { RefCell::new(Vec::new()) };
    static PENDING_PICK: RefCell<Option<String>> = const { RefCell::new(None) };
}
static SORT_COL: AtomicUsize = AtomicUsize::new(0);
static SORT_ASC: AtomicBool = AtomicBool::new(true);
static SHOW_HIDDEN: AtomicBool = AtomicBool::new(true);
static NAV: AtomicBool = AtomicBool::new(false);
const WM_NAV: u32 = WM_APP + 3;
const WM_JOB: u32 = WM_APP + 5;

fn row(name: String, m: Option<&fs::Metadata>) -> Row {
    Row { name, dir: m.is_some_and(|m| m.is_dir()), size: m.map_or(0, |m| m.len()), time: m.map_or(0, |m| m.last_write_time()), icon: -1, thumb: -2, hdr: false }
}

fn sort_rows(rows: &mut [Row], col: usize, asc: bool) {
    let key = |r: &Row| r.name.rsplit('\\').next().unwrap_or("").to_lowercase();
    rows.sort_by(|a, b| {
        let o = match col { 1 => a.time.cmp(&b.time), 2 => type_text(&a.name, a.dir).cmp(&type_text(&b.name, b.dir)), 3 => a.size.cmp(&b.size), _ => key(a).cmp(&key(b)) };
        (!a.dir).cmp(&!b.dir).then(if asc { o } else { o.reverse() }).then_with(|| key(a).cmp(&key(b)))
    });
    if group_on() { bucket_refresh(); rows.sort_by_cached_key(|r| bucket(r.time)); }
}

const DAY: i64 = 864_000_000_000;
fn bucket_name(i: usize) -> &'static str {
    [
        "Today",
        "Yesterday",
        "Earlier this week",
        "Last week",
        "Earlier this month",
        "Last month",
        "Earlier this year",
        "A long time ago",
    ][i]
}

thread_local! {
    static BCTX: std::cell::Cell<(i64, i64)> = const { std::cell::Cell::new((0, 0)) };
}

fn group_marker() -> PathBuf { PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join(r"rust-explorer\group-by-date") }
static THUMBS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn group_on() -> bool { !desk() && !THUMBS.load(Relaxed) && group_marker().exists() }

fn bucket_refresh() {
    use windows_sys::Win32::{Foundation::FILETIME, Storage::FileSystem::FileTimeToLocalFileTime};
    let now = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() / 100) as i64) + 116_444_736_000_000_000;
    let (utc, mut local) = (FILETIME { dwLowDateTime: now as u32, dwHighDateTime: (now >> 32) as u32 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 });
    let off = if unsafe { FileTimeToLocalFileTime(&utc, &mut local) } != 0 { ((local.dwHighDateTime as i64) << 32 | local.dwLowDateTime as i64) - now } else { 0 };
    BCTX.set(((now + off) / DAY, off));
}

fn bucket(time: u64) -> i32 {
    let (today, off) = BCTX.get();
    match today - (time as i64 + off) / DAY {
        i64::MIN..=0 => 0,
        1 => 1,
        2..=6 => 2,
        7..=13 => 3,
        14..=29 => 4,
        30..=59 => 5,
        60..=364 => 6,
        _ => 7,
    }
}

fn add_headers(rows: &mut Vec<Row>) {
    if !group_on() { return; }
    bucket_refresh();
    let mut out = Vec::with_capacity(rows.len() + 8);
    let mut last = -1;
    for r in rows.drain(..) {
        let b = bucket(r.time);
        if b != last {
            last = b;
            out.push(Row { name: format!("?{}", bucket_name(b as usize)), dir: false, size: 0, time: 0, icon: -1, thumb: -1, hdr: true });
        }
        out.push(r);
    }
    *rows = out;
}

fn type_text(name: &str, dir: bool) -> String {
    if dir { return "File folder".into(); }
    let file = name.rsplit('\\').next().unwrap_or(name);
    match file.rsplit_once('.') { Some((stem, e)) if !stem.is_empty() && !e.is_empty() => format!("{} file", e.to_uppercase()), _ => "File".into() }
}

fn size_text(b: u64) -> String {
    const MB: u64 = 1 << 20;
    if b < MB { format!("{} KB", b.div_ceil(1024)) } else if b < 1 << 30 { format!("{:.1} MB", b as f64 / MB as f64) } else { format!("{:.2} GB", b as f64 / (1u64 << 30) as f64) }
}

fn date_text(ft: u64) -> String {
    use windows_sys::Win32::{Foundation::{FILETIME, SYSTEMTIME}, System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime}};
    if ft == 0 { return String::new(); }
    unsafe {
        let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        let (mut u, mut l): (SYSTEMTIME, SYSTEMTIME) = (std::mem::zeroed(), std::mem::zeroed());
        if FileTimeToSystemTime(&f, &mut u) == 0 || SystemTimeToTzSpecificLocalTime(ptr::null(), &u, &mut l) == 0 { return String::new(); }
        format!("{:04}-{:02}-{:02} {:02}:{:02}", l.wYear, l.wMonth, l.wDay, l.wHour, l.wMinute)
    }
}

fn tool(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env::var_os("ProgramFiles")?).join(name).join(format!("{name}.exe"));
    p.is_file().then_some(p)
}

fn is_pic(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["jpg", "jpeg", "jfif", "png", "bmp", "gif", "tif", "tiff", "webp", "heic", "avif"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn properties(path: &PathBuf) {
    let (f, v) = (w(&path.to_string_lossy()), w("properties"));
    let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    sei.fMask = 0x0C;
    sei.lpVerb = v.as_ptr(); sei.lpFile = f.as_ptr(); sei.nShow = SW_SHOW;
    unsafe { ShellExecuteExW(&mut sei) };
}

fn properties_many(h: HWND, paths: &[PathBuf]) {
    if paths.len() == 1 {
        properties(&paths[0]);
    } else if paths.len() > 1 && !sh::multi_properties(h as _, paths) {
        properties(&paths[0]);
    }
}

pub fn is_cut_row(dir: &Path, name: &str) -> bool {
    if !sh::has_cut() || dir == Path::new(THIS_PC_KEY) || zipx::split_zip(dir).is_some() {
        return false;
    }
    let p = if Path::new(name).is_absolute() { PathBuf::from(name) } else { dir.join(name) };
    sh::is_cut(&p)
}

fn sync_cut_visuals() {
    let (list, dir, thumbs, len) = ST.with_borrow(|st| (st.list, st.dir.clone(), st.thumbs, st.rows.len()));
    if thumbs || desk() {
        for i in 0..len {
            let cut = ST.with_borrow(|st| st.rows.get(i).is_some_and(|r| !r.hdr && is_cut_row(&dir, &r.name)));
            let mut it: LVITEMW = unsafe { std::mem::zeroed() };
            it.stateMask = LVIS_CUT;
            it.state = if cut { LVIS_CUT } else { 0 };
            put(list, LVM_SETITEMSTATE, i, &it as *const _ as isize);
        }
    }
    if !list.is_null() {
        unsafe { InvalidateRect(list, ptr::null(), 0) };
    }
}

fn new_entry(dir: &PathBuf, base: &str, folder: bool) -> Option<String> {
    if zipx::split_zip(dir).is_some() { return None; }
    let (stem, ext) = match base.rsplit_once('.') { Some((s, e)) if !folder => (s.to_string(), format!(".{e}")), _ => (base.to_string(), String::new()) };
    let path = (1..).map(|n| dir.join(if n == 1 { base.to_string() } else { format!("{stem} ({n}){ext}") })).find(|p| !p.exists())?;
    let ok = if folder { fs::create_dir(&path).is_ok() } else { fs::File::create(&path).is_ok() };
    if !ok { return None; }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    UNDO.with_borrow_mut(|u| u.push(UndoOp::New { path, dir: folder }));
    name
}

fn paste(h: HWND, dir: &PathBuf) {
    if zipx::split_zip(dir).is_some() { return; }
    let Some((paths, cut)) = sh::clip_get() else { return };
    let fast = jobs::can_fast_copy(&paths, dir) && !(cut && paths.iter().all(|p| jobs::same_volume(p, dir))) && jobs::writable(dir);
    let sync = !(fast && !jobs::busy());
    if !sync {
        jobs::copy(h as isize, paths, dir.clone(), cut);
    } else {
        let _ = sh::fileop(h, if cut { sh::Op::Move } else { sh::Op::Copy }, &paths, dir);
    }
    if cut {
        sh::clip_clear();
        sync_cut_visuals();
    }
    if sync {
        reload_select_new(h);
    }
}

fn in_zip() -> bool { zipx::split_zip(&cur_dir()).is_some() }

fn confirm_delete(h: HWND, dir: &Path, paths: &[PathBuf], perm: bool) {
    if paths.is_empty() || in_zip() { return; }
    let what = if paths.len() == 1 {
        paths[0].file_name().map_or_else(|| paths[0].to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
    } else {
        format!("{} items", paths.len())
    };
    let msg = if perm {
        format!("Are you sure you want to permanently delete \"{}\"?", what)
    } else {
        format!("Are you sure you want to move \"{}\" to the Recycle Bin?", what)
    };
    let title = if perm {
        "Delete Permanently"
    } else {
        "Delete"
    };
    let def = if perm { MB_DEFBUTTON1 } else { MB_DEFBUTTON2 };
    let ans = unsafe { MessageBoxW(h, w(&msg).as_ptr(), w(title).as_ptr(), MB_YESNO | MB_ICONWARNING | def) };
    if ans == IDYES {
        let _ = sh::fileop(h as _, if perm { sh::Op::PermDelete } else { sh::Op::Delete }, paths, &dir.to_path_buf());
    }
}

fn warn(h: HWND, text: &str) {
    unsafe { MessageBoxW(h, w(text).as_ptr(), w("WinRust Explorer").as_ptr(), MB_ICONWARNING) };
}

fn zip_names(paths: &[PathBuf]) -> Option<(PathBuf, Vec<String>)> {
    let mut arc = None;
    let mut names = vec![];
    for p in paths {
        let (a, inner) = zipx::split_zip(p).filter(|(_, i)| !i.is_empty())?;
        if arc.get_or_insert_with(|| a.clone()) != &a { return None; }
        names.push(inner);
    }
    Some((arc?, names))
}

fn materialize(arc: &Path, names: &[String]) -> Result<Vec<PathBuf>, String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let root = env::temp_dir().join("WinRust-zip");
    for e in fs::read_dir(&root).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > std::time::Duration::from_secs(86_400)) {
            let _ = fs::remove_dir_all(e.path());
        }
    }
    let out = root.join(format!("{}-{}", std::process::id(), N.fetch_add(1, Relaxed)));
    zipx::extract_some(arc, names, &out)?;
    Ok(names.iter().map(|n| out.join(n.rsplit('/').next().unwrap_or(n))).collect())
}

fn real_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    match zip_names(paths) {
        Some((arc, names)) => materialize(&arc, &names),
        None => Ok(paths.to_vec()),
    }
}

const NO_CONSOLE: u32 = 0x0800_0000;

fn spawn_gui(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    let _ = cmd.creation_flags(NO_CONSOLE).spawn();
}

unsafe fn activate(h: HWND, p: PathBuf, is_dir: bool) {
    let p = match zip_names(std::slice::from_ref(&p)) {
        Some((arc, names)) if !is_dir => match materialize(&arc, &names) {
            Ok(mut v) => v.remove(0),
            Err(e) => return warn(h, &e),
        },
        _ => p,
    };
    if is_dir && desk() { spawn_gui(std::process::Command::new(env::current_exe().unwrap_or_default()).arg(p)); }
    else if is_dir { load(h, p) }
    else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) && p.is_file() { load(h, p) }
    else if let Some(exe) = tool("photoview").filter(|_| is_pic(&p)) { spawn_gui(std::process::Command::new(exe).arg(&p)); }
    else {
        let dir = p.parent().map(|d| w(&d.to_string_lossy()));
        unsafe { ShellExecuteW(h, ptr::null(), w(&p.to_string_lossy()).as_ptr(), ptr::null(), dir.as_ref().map_or(ptr::null(), |d| d.as_ptr()), SW_SHOWNORMAL) };
    }
}

fn zip_menu(h: HWND, paths: &[PathBuf]) {
    let items = [
        ui::MenuItem::new(1, "Open", "\u{E8E5}"),
        ui::MenuItem::new(2, "Copy", "\u{E8C8}"),
        ui::MenuItem::new(3, "Extract here", "\u{E8B7}").sep(),
    ];
    let id = unsafe { ui::popup_menu(h, &items) }.unwrap_or(0);
    let Some((arc, names)) = zip_names(paths) else { return };
    match id {
        1 => if let [p] = paths {
            let is_dir = ST.with_borrow(|st| st.rows.iter().any(|r| r.dir && st.dir.join(&r.name) == *p));
            unsafe { activate(h, p.clone(), is_dir) };
        },
        2 => match materialize(&arc, &names) {
            Ok(v) => { let _ = sh::clip_set(h, &v, false); }
            Err(e) => warn(h, &e),
        },
        3 => {
            let (Some(parent), Some(stem)) = (arc.parent(), arc.file_stem()) else { return };
            let out = jobs::free_name(parent, &stem.to_string_lossy(), true);
            std::thread::spawn(move || _ = zipx::extract_some(&arc, &names, &out));
        }
        _ => {}
    }
}

const THIS_PC_KEY: &str = "::THIS_PC::";
const RECYCLE_BIN_KEY: &str = "::RECYCLE_BIN::";
thread_local! { static MENU_DONE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) }; }

fn known_folder_icon(id: &windows::core::GUID) -> i32 {
    use windows_sys::Win32::UI::Shell::SHGFI_PIDL;
    let mut fi: SHFILEINFOW = unsafe { std::mem::zeroed() };
    unsafe {
        if let Ok(pidl) = windows::Win32::UI::Shell::SHGetKnownFolderIDList(id, 0, None) {
            SHGetFileInfoW(pidl as *const u16, 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_PIDL | SHGFI_SYSICONINDEX | SHGFI_SMALLICON);
            windows::Win32::System::Com::CoTaskMemFree(Some(pidl as *const _));
        }
    }
    fi.iIcon
}

fn this_pc_icon() -> i32 {
    known_folder_icon(&windows::Win32::UI::Shell::FOLDERID_ComputerFolder)
}

fn recycle_bin_icon() -> i32 {
    known_folder_icon(&windows::Win32::UI::Shell::FOLDERID_RecycleBinFolder)
}

fn copy_text(h: HWND, s: &str) {
    use windows::Win32::{
        Foundation::{HANDLE, HWND as WHWND},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        },
    };
    let ws = w(s);
    unsafe {
        if OpenClipboard(Some(WHWND(h))).is_ok() {
            let _ = EmptyClipboard();
            if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, ws.len() * 2) {
                let ptr = GlobalLock(hg) as *mut u16;
                if !ptr.is_null() {
                    ptr::copy_nonoverlapping(ws.as_ptr(), ptr, ws.len());
                    let _ = GlobalUnlock(hg);
                    let _ = SetClipboardData(13, Some(HANDLE(hg.0)));
                }
            }
            let _ = CloseClipboard();
        }
    }
}

fn open_terminal(dir: &Path) {
    if std::process::Command::new("wt.exe").arg("-d").arg(dir).spawn().is_err() {
        let _ = std::process::Command::new("cmd.exe").args(["/k", "cd", "/d"]).arg(dir).spawn();
    }
}

fn begin_rename(h: HWND) {
    let (list, ..) = selected();
    let i = put(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
    if i < 0 { return; }
    unsafe {
        SetFocus(list);
        ui::start_rename(h, list, i as usize);
    }
}

pub fn commit_rename(h: HWND, idx: usize, new: &str) {
    ST.with_borrow(|st| {
        let Some(r) = st.rows.get(idx) else { return };
        let old_p = st.dir.join(&r.name);
        let target_name = if desk() {
            let low = r.name.to_lowercase();
            if (low.ends_with(".lnk") || low.ends_with(".url")) && !new.to_lowercase().ends_with(&low[low.len() - 4..]) {
                format!("{new}{}", &r.name[r.name.len() - 4..])
            } else {
                new.to_string()
            }
        } else {
            new.to_string()
        };
        if let Some(parent) = old_p.parent() {
            let new_p = parent.join(target_name);
            if fs::rename(&old_p, &new_p).is_ok() {
                UNDO.with_borrow_mut(|u| u.push(UndoOp::Rename { from: old_p, to: new_p }));
            }
        }
    });
    unsafe { PostMessageW(h, WM_APP, 0, 0) };
}

fn item_menu(h: HWND, dir: &Path, paths: &[PathBuf]) {
    if zipx::split_zip(dir).is_some() { return zip_menu(h, paths); }
    let wh = windows::Win32::Foundation::HWND(h);
    if paths.iter().any(|p| p.to_string_lossy().ends_with(THIS_PC_KEY)) {
        if let Some(verb) = sh::native_known_menu(wh, &windows::Win32::UI::Shell::FOLDERID_ComputerFolder) {
            if verb == "open" {
                if let Ok(me) = env::current_exe() { spawn_gui(&mut std::process::Command::new(me)); }
            } else if verb != "\0" {
                reload(h);
            }
        }
        return;
    }
    if paths.iter().any(|p| p.to_string_lossy().ends_with(RECYCLE_BIN_KEY)) {
        if let Some(verb) = sh::native_known_menu(wh, &windows::Win32::UI::Shell::FOLDERID_RecycleBinFolder) {
            if verb == "open" {
                unsafe { ShellExecuteW(h, w("explore").as_ptr(), w("shell:RecycleBinFolder").as_ptr(), ptr::null(), ptr::null(), SW_SHOWNORMAL) };
            } else if verb != "\0" {
                reload(h);
            }
        }
        return;
    }
    if paths.is_empty() { return; }
    let Ok(first_meta) = fs::metadata(&paths[0]) else { return };
    if paths[1..].iter().any(|p| fs::symlink_metadata(p).is_err()) { return; }
    let single_dir = paths.len() == 1 && first_meta.is_dir();
    let can_runas = paths.len() == 1 && !first_meta.is_dir() && paths[0].extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "exe" | "bat" | "cmd" | "lnk" | "msi" | "ps1" | "com" | "scr" | "msc"));
    let pinned = single_dir && !desk() && ui::pins().contains(&paths[0]);
    let all_archives = paths.iter().all(|p| jobs::is_archive(p));
    let mut items = Vec::with_capacity(20);
    items.push(ui::MenuItem::new(1, "Open", "\u{E8E5}"));
    if can_runas {
        items.push(ui::MenuItem::new(10, "Run as administrator", "\u{E7EF}"));
    }
    if all_archives {
        items.push(ui::MenuItem::new(11, "Extract to folder", "\u{E8B7}").sep());
    }
    items.push(ui::MenuItem::new(12, "Compress to ZIP", "\u{F012}").sep());
    let shell_sm = sh::query_menu(wh, paths).map(|(sm, dyn_items)| {
        for (idx, mut it) in dyn_items.into_iter().enumerate() {
            if idx == 0 { it.sep = true; }
            items.push(it);
        }
        sm
    });
    if single_dir && !desk() {
        items.push(ui::MenuItem::new(2, "Open in new tab", "\u{E8A7}"));
        items.push(ui::MenuItem::new(3, if pinned { "Unpin from Quick access" } else { "Pin to Quick access" }, "\u{E718}"));
    }
    if single_dir {
        items.push(ui::MenuItem::new(14, "Open in Terminal", "\u{E756}"));
    }
    if paths.len() == 1 && is_pic(&paths[0]) {
        items.push(ui::MenuItem::new(15, "Set as desktop background", "\u{E91B}"));
    }
    items.push(ui::MenuItem::new(4, "Copy as path", "\u{E8C8}"));
    items.push(ui::MenuItem::new(5, "Cut", "\u{E8C6}").sep());
    items.push(ui::MenuItem::new(6, "Copy", "\u{E8C8}"));
    items.push(ui::MenuItem::new(7, "Delete", "\u{E74D}").sep().danger());
    if paths.len() == 1 {
        items.push(ui::MenuItem::new(8, "Rename", "\u{E8AC}"));
    }
    items.push(ui::MenuItem::new(9, "Properties", "\u{E946}").sep());
    match unsafe { ui::popup_menu(h, &items) } {
        Some(1) => {
            if single_dir && !desk() { load(h, paths[0].clone()); }
            else { unsafe { for p in paths { activate(h, p.clone(), p.is_dir()); } } }
        }
        Some(2) if single_dir => {
            ST.with_borrow_mut(|st| {
                st.tabs.push(paths[0].clone());
                st.active = st.tabs.len() - 1;
            });
            load(h, paths[0].clone());
        }
        Some(3) if single_dir => ui::toggle_pin(&paths[0]),
        Some(4) => {
            let joined = paths.iter().map(|p| format!("\"{}\"", p.to_string_lossy())).collect::<Vec<_>>().join("\r\n");
            copy_text(h, &joined);
        }
        Some(c @ (5 | 6)) => {
            if sh::clip_set(h, paths, c == 5).is_ok() {
                sync_cut_visuals();
            }
        }
        Some(7) => {
            let perm = unsafe { GetKeyState(0x10) < 0 };
            confirm_delete(h, dir, paths, perm);
            reload(h);
        }
        Some(8) => begin_rename(h),
        Some(9) if !paths.is_empty() => properties_many(h, paths),
        Some(10) if can_runas => unsafe {
            let wd = paths[0].parent().unwrap_or(dir);
            ShellExecuteW(h, w("runas").as_ptr(), w(&paths[0].to_string_lossy()).as_ptr(), ptr::null(), w(&wd.to_string_lossy()).as_ptr(), SW_SHOWNORMAL);
        },
        Some(11) if all_archives => {
            jobs::extract(h as isize, paths.to_vec(), dir.to_path_buf());
        }
        Some(12) => {
            jobs::compress(h as isize, paths.to_vec(), dir.to_path_buf());
        }
        Some(id) if id >= 0x1000 => {
            if let Some(ref sm) = shell_sm {
                sh::invoke(sm, wh, id);
            }
        }
        Some(14) if single_dir => open_terminal(&paths[0]),
        Some(15) if paths.len() == 1 && is_pic(&paths[0]) => unsafe {
            let wp = w(&paths[0].to_string_lossy());
            SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, wp.as_ptr() as *mut _, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE);
    if desk() {
                reload_wall();
                InvalidateRect(h, ptr::null(), 1);
            }
        },
        _ => {}
    }
}

fn icons_marker() -> PathBuf { PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join(r"WinRust\desktop-icons-hidden.txt") }
fn small_marker() -> PathBuf { icons_marker().with_file_name("small-icons") }
fn desk_icon_tier() -> u8 {
    match fs::read_to_string(small_marker()) {
        Ok(s) if s.trim() == "2" => 2,
        Ok(_) => 0,
        Err(_) => 1,
    }
}
fn set_desk_icon_tier(tier: u8) {
    let k = small_marker();
    if tier == 1 {
        let _ = fs::remove_file(&k);
    } else {
        if let Some(d) = k.parent() { let _ = fs::create_dir_all(d); }
        let _ = fs::write(&k, tier.to_string());
    }
}
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn apply_icon_size(list: HWND) {
    let tier = desk_icon_tier();
    let mut fi: SHFILEINFOW = std::mem::zeroed();
    let il = if tier == 2 {
        use windows::Win32::UI::{Controls::IImageList, Shell::{SHGetImageList, SHIL_EXTRALARGE}};
        SHGetImageList::<IImageList>(SHIL_EXTRALARGE as i32)
            .map(|l| windows::core::Interface::into_raw(l) as isize)
            .unwrap_or_else(|_| SHGetFileInfoW(w("C:\\").as_ptr(), 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | SHGFI_LARGEICON) as isize)
    } else {
        let flag = if tier == 0 { SHGFI_SMALLICON } else { SHGFI_LARGEICON };
        SHGetFileInfoW(w("C:\\").as_ptr(), 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | flag) as isize
    };
    put(list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, il);
    let (cx, cy) = match tier { 0 => (76, 64), 2 => (108, 116), _ => (88, 96) };
    put(list, LVM_SETICONSPACING, 0, ((ui::sc(cy) as isize) << 16) | ui::sc(cx) as isize);
}
fn icons_hidden() -> bool { icons_marker().exists() }
fn set_icons_hidden(hide: bool) {
    let m = icons_marker();
    if hide { if let Some(d) = m.parent() { let _ = fs::create_dir_all(d); } let _ = fs::write(&m, "1"); } else { let _ = fs::remove_file(&m); }
    let list = ST.with_borrow(|st| st.list);
    unsafe { ShowWindow(list, if hide { SW_HIDE } else { SW_SHOW }) };
}

fn toggle_group_by_date() {
    let k = group_marker();
    if group_on() {
        let _ = fs::remove_file(&k);
    } else {
        if let Some(d) = k.parent() { let _ = fs::create_dir_all(d); }
        let _ = fs::write(&k, "1");
    }
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn bg_menu(h: HWND) {
    let dir = cur_dir();
    let tier = desk_icon_tier();
    let hidden = icons_hidden();
    let grouped = group_on();
    let col = SORT_COL.load(Relaxed);
    let view_sub = if desk() {
        vec![
            ui::MenuItem::new(0x7001, "Large icons", "").checked(tier == 2),
            ui::MenuItem::new(0x7002, "Medium icons", "").checked(tier == 1),
            ui::MenuItem::new(0x7003, "Small icons", "").checked(tier == 0),
            ui::MenuItem::new(0x7004, "Auto arrange icons", "").sep(),
            ui::MenuItem::new(0x7005, "Show desktop icons", "").checked(!hidden),
        ]
    } else {
        vec![
            ui::MenuItem::new(0x7010, "Large icons", "").checked(THUMBS.load(Relaxed)),
            ui::MenuItem::new(0x7011, "Details", "").checked(!THUMBS.load(Relaxed)),
            ui::MenuItem::new(0x7006, "Group by date", "").sep().checked(grouped),
        ]
    };
    let sort_sub = vec![
        ui::MenuItem::new(0x7007, "Name", "").checked(col == 0),
        ui::MenuItem::new(0x7009, "Date modified", "").checked(col == 1),
        ui::MenuItem::new(0x7012, "Type", "").checked(col == 2),
        ui::MenuItem::new(0x7008, "Size", "").checked(col == 3),
    ];
    let new_sub = vec![
        ui::MenuItem::new(0x700C, "Folder", "\u{E8B7}"),
        ui::MenuItem::new(0x700D, "Text Document", "\u{E8A5}"),
    ];
    let can_paste = sh::clip_get().is_some() && dir != Path::new(THIS_PC_KEY);
    let mut items = vec![
        ui::MenuItem::new(0, "View", "\u{E8FD}").sub(view_sub),
        ui::MenuItem::new(0, "Sort by", "\u{E8CB}").sub(sort_sub),
        ui::MenuItem::new(0x700A, "Refresh", "\u{E72C}"),
        ui::MenuItem::new(0x700B, "Paste", "\u{E77F}").sep().disabled(!can_paste),
        ui::MenuItem::new(0, "New", "\u{E710}").sep().sub(new_sub),
    ];
    if dir != Path::new(THIS_PC_KEY) && !in_zip() {
        items.push(ui::MenuItem::new(0x7013, "Open in Terminal", "\u{E756}").sep());
    }
    if desk() {
        items.push(ui::MenuItem::new(0x700E, "Display settings", "\u{E7F4}").sep());
        items.push(ui::MenuItem::new(0x700F, "Personalize", "\u{E771}"));
    }
    let Some(act) = ui::popup_menu(h, &items) else { return };
    let mut created = None;
    match act {
        0x7001 => { set_desk_icon_tier(2); apply_icon_size(ST.with_borrow(|st| st.list)); let _ = fs::remove_file(desk::pos_file()); }
        0x7002 => { set_desk_icon_tier(1); apply_icon_size(ST.with_borrow(|st| st.list)); let _ = fs::remove_file(desk::pos_file()); }
        0x7003 => { set_desk_icon_tier(0); apply_icon_size(ST.with_borrow(|st| st.list)); let _ = fs::remove_file(desk::pos_file()); }
        0x7004 => { let _ = fs::remove_file(desk::pos_file()); }
        0x7005 => set_icons_hidden(!icons_hidden()),
        0x7006 => toggle_group_by_date(),
        0x7010 if !THUMBS.load(Relaxed) => toggle_view(h),
        0x7011 if THUMBS.load(Relaxed) => toggle_view(h),
        0x7007 => { SORT_COL.store(0, Relaxed); SORT_ASC.store(true, Relaxed); if desk() { let _ = fs::remove_file(desk::pos_file()); } }
        0x7008 => { SORT_COL.store(3, Relaxed); SORT_ASC.store(true, Relaxed); if desk() { let _ = fs::remove_file(desk::pos_file()); } }
        0x7009 => { SORT_COL.store(1, Relaxed); SORT_ASC.store(true, Relaxed); if desk() { let _ = fs::remove_file(desk::pos_file()); } }
        0x7012 => { SORT_COL.store(2, Relaxed); SORT_ASC.store(true, Relaxed); if desk() { let _ = fs::remove_file(desk::pos_file()); } }
        0x700A => {}
        0x700B => { paste(h, &dir); return; }
        0x700C => created = new_entry(&dir, "New folder", true),
        0x700D => created = new_entry(&dir, "New Text Document.txt", false),
        0x7013 => {
            open_terminal(&dir);
            return;
        }
        0x700E | 0x700F => {
            let page = if act == 0x700E { "ms-settings:display" } else { "ms-settings:personalization" };
            let exe = tool("rust-settings").or_else(|| env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("rust-settings.exe"))).filter(|p| p.is_file()));
            if let Some(p) = exe {
                spawn_gui(std::process::Command::new(p).arg(page));
            } else {
                ShellExecuteW(h, ptr::null(), w(page).as_ptr(), ptr::null(), ptr::null(), SW_SHOWNORMAL);
            }
            return;
        }
        _ => {}
    }
    reload(h);
    if let Some(name) = created {
        select_row(&name);
    }
}

unsafe fn new_menu(h: HWND) {
    let dir = cur_dir();
    if dir == Path::new(THIS_PC_KEY) || in_zip() { return; }
    let items = [
        ui::MenuItem::new(1, "Folder", "\u{E8B7}"),
        ui::MenuItem::new(2, "Text Document", "\u{E8A5}"),
    ];
    let created = match unsafe { ui::popup_menu(h, &items) } {
        Some(1) => new_entry(&dir, "New folder", true),
        Some(2) => new_entry(&dir, "New Text Document.txt", false),
        _ => return,
    };
    reload(h);
    if let Some(name) = created {
        select_row(&name);
    }
}

unsafe fn sort_menu(h: HWND) {
    let col = SORT_COL.load(Relaxed);
    let items = [
        ui::MenuItem::new(0, "Name", "").checked(col == 0),
        ui::MenuItem::new(1, "Date modified", "").checked(col == 1),
        ui::MenuItem::new(2, "Type", "").checked(col == 2),
        ui::MenuItem::new(3, "Size", "").checked(col == 3),
    ];
    if let Some(c) = unsafe { ui::popup_menu(h, &items) } {
        if SORT_COL.load(Relaxed) == c { SORT_ASC.store(!SORT_ASC.load(Relaxed), Relaxed); }
        else { SORT_COL.store(c, Relaxed); SORT_ASC.store(true, Relaxed); }
        reload(h);
    }
}

unsafe fn view_menu(h: HWND) {
    let items = [
        ui::MenuItem::new(1, "Large icons", "").checked(THUMBS.load(Relaxed)),
        ui::MenuItem::new(2, "Details", "").checked(!THUMBS.load(Relaxed)),
        ui::MenuItem::new(3, "Group by date", "").sep().checked(group_on()),
        ui::MenuItem::new(4, "Hidden items", "").checked(SHOW_HIDDEN.load(Relaxed)),
    ];
    match unsafe { ui::popup_menu(h, &items) } {
        Some(1) if !THUMBS.load(Relaxed) => toggle_view(h),
        Some(2) if THUMBS.load(Relaxed) => toggle_view(h),
        Some(3) => {
            toggle_group_by_date();
            reload(h);
        }
        Some(4) => {
            SHOW_HIDDEN.store(!SHOW_HIDDEN.load(Relaxed), Relaxed);
            reload(h);
        }
        _ => {}
    }
}

unsafe fn more_menu(h: HWND) {
    let (list, _, paths) = selected();
    let items = [
        ui::MenuItem::new(1, "Select all", "\u{E8B3}"),
        ui::MenuItem::new(2, "Properties", "\u{E946}").disabled(paths.is_empty()),
    ];
    match unsafe { ui::popup_menu(h, &items) } {
        Some(1) => {
            let mut it: LVITEMW = unsafe { std::mem::zeroed() };
            it.stateMask = LVIS_SELECTED;
            it.state = LVIS_SELECTED;
            put(list, LVM_SETITEMSTATE, usize::MAX, &it as *const _ as isize);
        }
        Some(2) if !paths.is_empty() => properties_many(h, &paths),
        _ => {}
    }
}

fn start_target() -> (PathBuf, Option<String>) {
    let mut a = env::args().skip(1);
    let mut p = a.next().unwrap_or_default();
    let pl = p.to_lowercase();
    let select_mode = pl.starts_with("/select,") || pl.starts_with("--select");
    if select_mode {
        p = p.get(8..).unwrap_or("").trim_start_matches(',').trim().to_string();
        if p.is_empty() { p = a.next().unwrap_or_default(); }
    }
    let p = PathBuf::from(p.trim().trim_matches('"'));
    if p.as_os_str().is_empty() { return (PathBuf::from(THIS_PC_KEY), None); }
    match (select_mode || p.is_file(), p.parent()) {
        (true, Some(d)) => (d.to_path_buf(), p.file_name().map(|n| n.to_string_lossy().into_owned())),
        _ => (p, None),
    }
}

fn shell_select(h: isize, p: PathBuf) {
    let h = h as HWND;
    let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else { return };
    let name_s = name.to_string_lossy().into_owned();
    let has_row = ST.with_borrow(|st| st.rows.iter().any(|r| r.name.eq_ignore_ascii_case(&name_s)));
    if !cur_dir().as_os_str().eq_ignore_ascii_case(dir.as_os_str()) || !has_row {
        load(h, dir.to_path_buf());
    }
    select_row(&name_s);
    unsafe {
        if IsIconic(h) != 0 { ShowWindow(h, SW_RESTORE); }
        SetForegroundWindow(h);
        SetFocus(ST.with_borrow(|st| st.list));
    }
}

fn select_row(name: &str) {
    let (list, i) = ST.with_borrow(|st| (st.list, st.rows.iter().position(|r| r.name.eq_ignore_ascii_case(name))));
    let Some(i) = i else { return };
    unsafe {
        let mut it: LVITEMW = std::mem::zeroed();
        it.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
        put(list, LVM_SETITEMSTATE, usize::MAX, &it as *const _ as isize);
        it.state = LVIS_SELECTED | LVIS_FOCUSED;
        put(list, LVM_SETITEMSTATE, i, &it as *const _ as isize);
        put(list, LVM_ENSUREVISIBLE, i, 0);
        SetFocus(list);
    }
}

fn nav(h: HWND, back: bool) {
    let to = ST.with_borrow_mut(|st| if back { st.hist.back() } else { st.hist.forward() });
    if let Some(p) = to {
        NAV.store(true, Relaxed);
        load(h, p);
        NAV.store(false, Relaxed);
    }
}

fn w(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

fn put(h: HWND, msg: u32, wp: usize, lp: isize) -> isize { unsafe { SendMessageW(h, msg, wp, lp) } }

fn wstr(p: *const u16) -> String {
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 { n += 1; }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

pub fn cur_dir() -> PathBuf { ST.with_borrow(|st| st.dir.clone()) }

fn icon_of(p: &PathBuf, dir: bool) -> i32 {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let own = !dir && matches!(ext.as_str(), "exe" | "lnk" | "ico" | "url" | "scr");
    let mut fi: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let attr = if dir { 0x10 } else { 0x80 };
    let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | if own { 0 } else { SHGFI_USEFILEATTRIBUTES };
    unsafe { SHGetFileInfoW(w(&p.to_string_lossy()).as_ptr(), attr, &mut fi, size_of::<SHFILEINFOW>() as u32, flags) };
    fi.iIcon
}

fn shown_name(name: &str, desktop: bool) -> String {
    if name == THIS_PC_KEY { return "This PC".to_string(); }
    if name == RECYCLE_BIN_KEY { return "Recycle Bin".to_string(); }
    if !desktop { return name.to_string(); }
    let file = std::path::Path::new(name).file_name().map_or_else(|| name.to_string(), |n| n.to_string_lossy().into_owned());
    let lower = file.to_lowercase();
    if lower.ends_with(".lnk") || lower.ends_with(".url") { file[..file.len() - 4].to_string() } else { file }
}

fn label(dir: &PathBuf) -> String {
    if dir == Path::new(THIS_PC_KEY) {
        return "This PC".into();
    }
    dir.file_name().map_or_else(|| dir.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

fn rows_of(dir: &PathBuf) -> Option<Vec<Row>> {
    if !desk() && dir == Path::new(THIS_PC_KEY) {
        use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos};
        let mut rows = Vec::new();
        let mut folders = Vec::new();
        for id in [&FOLDERID_Desktop, &FOLDERID_Downloads, &FOLDERID_Documents, &FOLDERID_Pictures, &FOLDERID_Music, &FOLDERID_Videos] {
            if let Some(p) = sh::known_dir(id).filter(|p| p.is_dir()) {
                folders.push(Row { name: p.to_string_lossy().into_owned(), dir: true, size: 0, time: 0, icon: -1, thumb: -2, hdr: false });
            }
        }
        if !folders.is_empty() {
            rows.push(Row { name: format!("\0{} ({})", "Folders", folders.len()), dir: false, size: 0, time: 0, icon: -1, thumb: -2, hdr: true });
            rows.extend(folders);
        }
        let mut drives = Vec::new();
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
            let mask = GetLogicalDrives();
            for i in 0..26u32 {
                if mask & (1 << i) == 0 { continue; }
                let root = format!("{}:\\", (b'A' + i as u8) as char);
                let wr = w(&root);
                let ty = GetDriveTypeW(wr.as_ptr());
                if !matches!(ty, 2 | 3 | 4 | 5 | 6) { continue; }
                let (mut free, mut total) = (0u64, 0u64);
                let _ = GetDiskFreeSpaceExW(wr.as_ptr(), &mut free, &mut total, ptr::null_mut());
                drives.push(Row { name: root, dir: true, size: total, time: 0, icon: -1, thumb: -2, hdr: false });
            }
        }
        if !drives.is_empty() {
            rows.push(Row { name: format!("\0{} ({})", "Devices and drives", drives.len()), dir: false, size: 0, time: 0, icon: -1, thumb: -2, hdr: true });
            rows.extend(drives);
        }
        return Some(rows);
    }
    if !desk() {
        if let Some((arc, inner)) = zipx::split_zip(dir) {
            let mut rows: Vec<Row> = zipx::list_dir(&arc, &inner).ok()?.into_iter().map(|i| Row { name: i.name, dir: i.dir, size: i.size, time: zipx::dos_to_filetime(i.time), icon: -1, thumb: -2, hdr: false }).collect();
            sort_rows(&mut rows, SORT_COL.load(Relaxed), SORT_ASC.load(Relaxed));
            add_headers(&mut rows);
            return Some(rows);
        }
    }
    let mut dirs = vec![dir.clone()];
    if desk() { dirs.extend(sh::desktop_dirs().into_iter().skip(1)); }
    let mut rows = vec![];
    for (n, d) in dirs.iter().enumerate() {
        let Ok(rd) = fs::read_dir(d) else { if n == 0 { return None } else { continue } };
        for e in rd.flatten() {
            if e.file_name().eq_ignore_ascii_case("desktop.ini") { continue; }
            let m = e.metadata().ok();
            if m.as_ref().is_some_and(|m| m.file_attributes() & 2 != 0) && !SHOW_HIDDEN.load(Relaxed) { continue; }
            let name = if n == 0 { e.file_name().to_string_lossy().into_owned() } else { e.path().to_string_lossy().into_owned() };
            rows.push(row(name, m.as_ref()));
        }
    }
    sort_rows(&mut rows, SORT_COL.load(Relaxed), SORT_ASC.load(Relaxed));
    if desk() {
        rows.insert(0, Row { name: THIS_PC_KEY.into(), dir: true, size: 0, time: 0, icon: this_pc_icon(), thumb: -2, hdr: false });
        rows.insert(1, Row { name: RECYCLE_BIN_KEY.into(), dir: true, size: 0, time: 0, icon: recycle_bin_icon(), thumb: -2, hdr: false });
    }
    add_headers(&mut rows);
    Some(rows)
}

static WATCH_WAKE: AtomicUsize = AtomicUsize::new(0);
static WATCH_DIR: std::sync::Mutex<PathBuf> = std::sync::Mutex::new(PathBuf::new());
static WATCH_INIT: std::sync::Once = std::sync::Once::new();
static WATCH_PENDING: AtomicBool = AtomicBool::new(false);

fn watch_set(h: HWND, dir: &Path) {
    use windows_sys::Win32::{
        Storage::FileSystem::{
            FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
            FILE_NOTIFY_CHANGE_SIZE, FindCloseChangeNotification, FindFirstChangeNotificationW,
            FindNextChangeNotification,
        },
        System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject},
    };
    {
        let mut g = WATCH_DIR.lock().unwrap();
        if *g == dir {
            return;
        }
        *g = dir.to_path_buf();
    }
    let hw = h as isize;
    WATCH_INIT.call_once(|| {
        let ev = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
        WATCH_WAKE.store(ev as usize, Relaxed);
        std::thread::spawn(move || unsafe {
            let wake = WATCH_WAKE.load(Relaxed) as HANDLE;
            if wake.is_null() {
                return;
            }
            loop {
                let d = WATCH_DIR.lock().unwrap().clone();
                let ch = if d.as_os_str().is_empty()
                    || d == Path::new(THIS_PC_KEY)
                    || d == Path::new(RECYCLE_BIN_KEY)
                    || zipx::split_zip(&d).is_some()
                    || !d.is_dir()
                {
                    INVALID_HANDLE_VALUE
                } else {
                    FindFirstChangeNotificationW(
                        w(&d.to_string_lossy()).as_ptr(),
                        0,
                        FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE,
                    )
                };
                if ch == INVALID_HANDLE_VALUE || ch.is_null() {
                    WaitForSingleObject(wake, u32::MAX);
                    continue;
                }
                let handles = [wake, ch];
                loop {
                    let r = WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX);
                    if r == 1 {
                        std::thread::sleep(std::time::Duration::from_millis(120));
                        while WaitForSingleObject(ch, 0) == 0 {
                            if FindNextChangeNotification(ch) == 0 {
                                break;
                            }
                        }
                        if !WATCH_PENDING.swap(true, Relaxed) {
                            PostMessageW(hw as HWND, WM_APP, 0, 0);
                        }
                        if FindNextChangeNotification(ch) == 0 {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                FindCloseChangeNotification(ch);
            }
        });
    });
    let wake = WATCH_WAKE.load(Relaxed) as HANDLE;
    if !wake.is_null() {
        unsafe { SetEvent(wake) };
    }
}

fn load(h: HWND, dir: PathBuf) {
    WATCH_PENDING.store(false, Relaxed);
    let Some(rows) = rows_of(&dir) else { return };
    let same_dir = !desk() && cur_dir() == dir;
    if same_dir {
        let unchanged = ST.with_borrow(|st| {
            st.rows.len() == rows.len()
                && st.rows.iter().zip(&rows).all(|(a, b)| a.name == b.name && a.dir == b.dir && a.size == b.size && a.time == b.time && a.hdr == b.hdr)
        });
        if unchanged {
            return;
        }
    }
    if !desk() { unsafe { SetWindowTextW(h, w(&dir.to_string_lossy()).as_ptr()) }; }
    let (list, tab, edit, n) = ST.with_borrow_mut(|st| {
        if let Some(t) = st.tabs.get_mut(st.active) { *t = dir.clone(); }
        st.dir = dir.clone(); st.rows = rows;
        if !NAV.load(Relaxed) { st.hist.visit(dir.clone()); }
        (st.list, st.tab, st.edit, st.rows.len())
    });
    unsafe { SetWindowTextW(edit, w(&dir.to_string_lossy()).as_ptr()) };
    shellwin::navigated(&dir);
    ui::set_search(None);
    if !tab.is_null() { unsafe { InvalidateRect(tab, ptr::null(), 0) }; }
    ui::nav_dirty();
    reset_thumbs();
    if !desk() {
        unsafe { ui::end_rename(false) };
    }
    if !same_dir {
        put(list, LVM_DELETEALLITEMS, 0, 0);
    }
    if !desk() {
        ui::reset_pc();
        unsafe {
            let cur_s = GetWindowLongW(list, GWL_STYLE) as u32;
            let want = if dir == Path::new(THIS_PC_KEY) { cur_s | 0x4000 } else { cur_s & !0x4000 };
            if want != cur_s {
                SetWindowLongW(list, GWL_STYLE, want as i32);
                SetWindowPos(list, ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
            }
            ui::status_dirty(h);
        }
    }
    if desk() {
        desk::fill(list, n);
    } else if dir == Path::new(THIS_PC_KEY) {
        put(list, LVM_SETITEMCOUNT, 0, 0);
    } else {
        put(list, LVM_SETITEMCOUNT, n, if same_dir { 0x0002 } else { 0 });
        unsafe { ui::fit_columns(list) };
    }
    sync_cut_visuals();
    watch_set(h, &dir);
}

fn ask(d: char, q: &str) -> Option<String> {
    use std::{fs::OpenOptions, io::{Read, Write}};
    let mut p = OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\rs-search-{d}")).ok()?;
    p.write_all(q.as_bytes()).ok()?;
    let (mut out, mut chunk) = (Vec::new(), [0u8; 1 << 16]);
    while let Ok(n @ 1..) = p.read(&mut chunk) { out.extend_from_slice(&chunk[..n]); }
    Some(String::from_utf8_lossy(&out).into_owned())
}

fn search(h: HWND, q: &str) {
    let mut rows = vec![];
    let mut more = String::new();
    let mut indexed = false;
    for d in 'A'..='Z' {
        let Some(reply) = ask(d, q) else { continue };
        indexed = true;
        for line in reply.lines() {
            if line.starts_with("... ") { more = format!(", {}", line.trim_start_matches("... ")); continue; }
            let m = fs::metadata(line).ok();
            rows.push(row(line.to_string(), m.as_ref()));
        }
    }
    if !indexed {
        let (local, cut, flipped) = local_search(&cur_dir(), q);
        if flipped { more = format!(" for \u{201C}{}\u{201D}", flip_layout(q)); }
        rows = local;
        if cut { more += ", stopped early: narrow the search"; }
    }
    let n = rows.len();
    let where_ = if indexed { "" } else { " in this folder" };
    let title = if n == 0 {
        format!("search: {} - nothing{}{}", q, where_, "")
    } else {
        format!("search: {} - {} hits{}{}", q, n, where_, more)
    };
    ui::set_search(Some(q.to_string()));
    unsafe { SetWindowTextW(h, w(&title).as_ptr()) };
    let list = ST.with_borrow_mut(|st| { st.rows = rows; st.list });
    reset_thumbs();
    put(list, LVM_DELETEALLITEMS, 0, 0);
    put(list, LVM_SETITEMCOUNT, n, 0);
    unsafe { ui::fit_columns(list) };
}

fn flip_layout(q: &str) -> String {
    const EN: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`QWERTYUIOP{}ASDFGHJKL:\"ZXCVBNM<>~";
    const RU: &str = "йцукенгшщзхъфывапролджэячсмитьбюёЙЦУКЕНГШЩЗХЪФЫВАПРОЛДЖЭЯЧСМИТЬБЮЁ";
    q.chars()
        .map(|c| {
            EN.chars().position(|x| x == c).and_then(|i| RU.chars().nth(i))
                .or_else(|| RU.chars().position(|x| x == c).and_then(|i| EN.chars().nth(i)))
                .unwrap_or(c)
        })
        .collect()
}

fn local_search(dir: &Path, q: &str) -> (Vec<Row>, bool, bool) {
    const LOCAL_MAX: usize = 500;
    const LOCAL_MS: u128 = 1500;
    let (q, t0) = (q.to_lowercase(), std::time::Instant::now());
    let alt = flip_layout(&q);
    let (mut out, mut other, mut queue) = (vec![], vec![], std::collections::VecDeque::from([dir.to_path_buf()]));
    let mut cut = false;
    'walk: while let Some(d) = queue.pop_front() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            if out.len() >= LOCAL_MAX || t0.elapsed().as_millis() > LOCAL_MS { cut = true; break 'walk; }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() && !ft.is_symlink() { queue.push_back(e.path()); }
            let name = e.file_name().to_string_lossy().to_lowercase();
            let hit = |v: &mut Vec<Row>| v.push(row(e.path().to_string_lossy().into_owned(), e.metadata().ok().as_ref()));
            if name.contains(&q) { hit(&mut out); } else if alt != q && other.len() < LOCAL_MAX && name.contains(&alt) { hit(&mut other); }
        }
    }
    if out.is_empty() && !other.is_empty() { (other, cut, true) } else { (out, cut, false) }
}

fn reset_thumbs() {
    GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let himl = ST.with_borrow(|st| st.himl);
    unsafe { ImageList_Remove(himl as _, -1) };
}

fn reload_select_new(h: HWND) {
    let (old, prev_sel) = ST.with_borrow(|st| {
        let old: std::collections::HashSet<String> = st.rows.iter()
            .filter(|r| !r.hdr)
            .map(|r| r.name.to_lowercase())
            .collect();
        let mut prev_sel = std::collections::HashSet::new();
        if !st.list.is_null() {
            let mut idx = put(st.list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
            while idx >= 0 {
                if let Some(r) = st.rows.get(idx as usize).filter(|r| !r.hdr) {
                    prev_sel.insert(r.name.to_lowercase());
                }
                idx = put(st.list, LVM_GETNEXTITEM, idx as usize, LVNI_SELECTED as isize);
            }
        }
        (old, prev_sel)
    });
    reload(h);
    let (list, picks, keep) = ST.with_borrow(|st| {
        let picks: Vec<usize> = st.rows.iter().enumerate()
            .filter(|(_, r)| !r.hdr && !old.contains(&r.name.to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        let keep: Vec<usize> = if picks.is_empty() && !prev_sel.is_empty() {
            st.rows.iter().enumerate()
                .filter(|(_, r)| !r.hdr && prev_sel.contains(&r.name.to_lowercase()))
                .map(|(i, _)| i)
                .collect()
        } else {
            Vec::new()
        };
        (st.list, picks, keep)
    });
    let target = if !picks.is_empty() { &picks } else { &keep };
    if !target.is_empty() && !list.is_null() {
        unsafe {
            let mut it: LVITEMW = std::mem::zeroed();
            it.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
            put(list, LVM_SETITEMSTATE, usize::MAX, &it as *const _ as isize);
            it.state = LVIS_SELECTED | LVIS_FOCUSED;
            for &i in target {
                put(list, LVM_SETITEMSTATE, i, &it as *const _ as isize);
            }
            if !picks.is_empty() {
                put(list, LVM_ENSUREVISIBLE, picks[0], 0);
            }
        }
    }
}

fn reload(h: HWND) { load(h, cur_dir()); }

fn selected() -> (HWND, PathBuf, Vec<PathBuf>) {
    ST.with_borrow(|st| {
        let mut sel = vec![];
        if !desk() && st.dir == Path::new(THIS_PC_KEY) {
            if let Some(r) = ui::pc_selected().and_then(|i| st.rows.get(i)).filter(|r| !r.hdr) {
                sel.push(st.dir.join(&r.name));
            }
            return (st.list, st.dir.clone(), sel);
        }
        let mut i = -1isize;
        loop {
            i = put(st.list, LVM_GETNEXTITEM, i as usize, LVNI_SELECTED as isize);
            let Some(r) = usize::try_from(i).ok().and_then(|i| st.rows.get(i)) else { break };
            if !r.hdr { sel.push(st.dir.join(&r.name)); }
        }
        (st.list, st.dir.clone(), sel)
    })
}

fn col(list: HWND, i: i32, text: &str, width: i32, right: bool) {
    let mut t = w(text);
    let mut c: LVCOLUMNW = unsafe { std::mem::zeroed() };
    c.mask = LVCF_TEXT | LVCF_WIDTH | LVCF_FMT; c.pszText = t.as_mut_ptr(); c.cx = width;
    c.fmt = if right { LVCFMT_RIGHT } else { LVCFMT_LEFT };
    put(list, LVM_INSERTCOLUMNW, i as usize, &c as *const _ as isize);
}

fn tab_new() {
    let tab = ST.with_borrow_mut(|st| { st.tabs.push(st.dir.clone()); st.active = st.tabs.len() - 1; st.tab });
    if !tab.is_null() { unsafe { InvalidateRect(tab, ptr::null(), 0) }; }
}

fn tab_select(h: HWND, i: usize) {
    let dir = ST.with_borrow_mut(|st| { st.active = i; st.tabs[i].clone() });
    load(h, dir);
}

fn tab_close(h: HWND) { let a = ST.with_borrow(|st| st.active); tab_close_at(h, a); }

fn tab_close_at(h: HWND, i: usize) {
    let close_win = ST.with_borrow(|st| st.tabs.len() < 2);
    if close_win {
        unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
        return;
    }
    let (tab, was_active) = ST.with_borrow_mut(|st| {
        if i >= st.tabs.len() { return (st.tab, None); }
        st.tabs.remove(i);
        let before = st.active;
        st.active = if i < before { before - 1 } else { before.min(st.tabs.len() - 1) };
        (st.tab, Some(i == before))
    });
    match was_active {
        Some(true) => { let a = ST.with_borrow(|st| st.active); tab_select(h, a); }
        Some(false) => if !tab.is_null() { unsafe { InvalidateRect(tab, ptr::null(), 0) }; },
        None => {}
    }
}

fn toggle_view(h: HWND) {
    let (list, on) = ST.with_borrow_mut(|st| { st.thumbs = !st.thumbs; (st.list, st.thumbs) });
    THUMBS.store(on, Relaxed);
    ui::row_height(list, !on);
    put(list, LVM_SETVIEW, if on { LV_VIEW_ICON } else { LV_VIEW_DETAILS } as usize, 0);
    reload(h);
}

fn search_focus() {
    let edit = ST.with_borrow(|st| st.edit);
    unsafe {
        SetWindowTextW(edit, w("? ").as_ptr());
        SetFocus(edit);
    }
    put(edit, EM_SETSEL, 2, 2);
}

mod wall {
    use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*, UI::WindowsAndMessaging::{IMAGE_BITMAP, LR_LOADFROMFILE, LoadImageW, SPI_GETDESKWALLPAPER, SystemParametersInfoW}};

    pub fn fill_src(bw: i32, bh: i32, cw: i32, ch: i32) -> (i32, i32, i32, i32) {
        let (sw, sh) = if bw as i64 * ch as i64 > bh as i64 * cw as i64 { (bh * cw / ch.max(1), bh) } else { (bw, bw * ch / cw.max(1)) };
        ((bw - sw) / 2, (bh - sh) / 2, sw, sh)
    }

    fn monitors(vx: i32, vy: i32) -> Vec<RECT> {
        unsafe extern "system" fn each(_: HMONITOR, _: HDC, r: *mut RECT, lp: isize) -> i32 {
            unsafe { (*(lp as *mut Vec<RECT>)).push(*r) };
            1
        }
        let mut v: Vec<RECT> = Vec::new();
        unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(each), &mut v as *mut _ as isize) };
        v.iter().map(|r| RECT { left: r.left - vx, top: r.top - vy, right: r.right - vx, bottom: r.bottom - vy }).collect()
    }

    pub fn load(vx: i32, vy: i32, cw: i32, ch: i32) -> isize {
        let rs_path: Option<Vec<u16>> = std::env::var("LOCALAPPDATA").ok().map(|b| format!(r"{b}\rs-shell\wallpaper.bmp").encode_utf16().chain([0]).collect());
        unsafe {
            let mut src = rs_path.as_ref().map_or(std::ptr::null_mut(), |p| LoadImageW(std::ptr::null_mut(), p.as_ptr(), IMAGE_BITMAP, 0, 0, LR_LOADFROMFILE));
            if src.is_null() {
                let mut wp = [0u16; 260];
                if SystemParametersInfoW(SPI_GETDESKWALLPAPER, wp.len() as u32, wp.as_mut_ptr() as *mut _, 0) != 0 && wp[0] != 0 {
                    use windows_sys::Win32::Graphics::GdiPlus::*;
                    let mut gp: *mut GpBitmap = std::ptr::null_mut();
                    if GdipCreateBitmapFromFile(wp.as_ptr(), &mut gp) == 0 && !gp.is_null() {
                        let mut hb: HBITMAP = std::ptr::null_mut();
                        if GdipCreateHBITMAPFromBitmap(gp, &mut hb, 0xFF00_0000) == 0 && !hb.is_null() {
                            src = hb as _;
                        }
                        GdipDisposeImage(gp as *mut GpImage);
                    }
                }
            }
            if src.is_null() { return 0; }
            let mut bm: BITMAP = std::mem::zeroed();
            GetObjectW(src, size_of::<BITMAP>() as i32, &mut bm as *mut _ as _);
            let screen = GetDC(std::ptr::null_mut());
            let dst = CreateCompatibleBitmap(screen, cw, ch);
            let (sdc, ddc) = (CreateCompatibleDC(screen), CreateCompatibleDC(screen));
            let (o1, o2) = (SelectObject(sdc, src), SelectObject(ddc, dst));
            PatBlt(ddc, 0, 0, cw, ch, BLACKNESS);
            SetStretchBltMode(ddc, HALFTONE);
            let mut mons = monitors(vx, vy);
            if mons.is_empty() { mons.push(RECT { left: 0, top: 0, right: cw, bottom: ch }); }
            for m in mons {
                let (mw, mh) = (m.right - m.left, m.bottom - m.top);
                let (sx, sy, sw, sh) = fill_src(bm.bmWidth, bm.bmHeight, mw, mh);
                StretchBlt(ddc, m.left, m.top, mw, mh, sdc, sx, sy, sw, sh, SRCCOPY);
            }
            SelectObject(sdc, o1); SelectObject(ddc, o2);
            DeleteDC(sdc); DeleteDC(ddc); ReleaseDC(std::ptr::null_mut(), screen); DeleteObject(src);
            dst as isize
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn fill_crops_to_the_screen_aspect() {
            assert_eq!(super::fill_src(1920, 1080, 1920, 1080), (0, 0, 1920, 1080));
            assert_eq!(super::fill_src(1920, 1080, 1080, 1080), (420, 0, 1080, 1080));
            assert_eq!(super::fill_src(1920, 1080, 2560, 1080), (0, 135, 1920, 810));
            assert_eq!(super::fill_src(100, 100, 0, 0).2, 100);
        }
    }

    pub fn paint(dc: HDC, bmp: isize, cw: i32, ch: i32) {
        unsafe {
            let m = CreateCompatibleDC(dc);
            let old = SelectObject(m, bmp as _);
            BitBlt(dc, 0, 0, cw, ch, m, 0, 0, SRCCOPY);
            SelectObject(m, old);
            DeleteDC(m);
        }
    }
}

mod sh {
    use std::{cell::RefCell, ffi::c_void, os::windows::ffi::OsStrExt, path::PathBuf};
    use windows::{core::*, Win32::{Foundation::*, System::{Com::*, DataExchange::RegisterClipboardFormatW, Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock}, Ole::*, SystemServices::MODIFIERKEYS_FLAGS}, UI::{Shell::{Common::ITEMIDLIST, *}, WindowsAndMessaging::*}}};

    #[derive(Clone, Copy)]
    pub enum Op { Copy, Move, Delete, PermDelete }

    pub fn desktop_dirs() -> Vec<PathBuf> {
        [&FOLDERID_Desktop, &FOLDERID_PublicDesktop].iter().filter_map(|id| unsafe {
            let p = SHGetKnownFolderPath(*id, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            s.map(PathBuf::from)
        }).collect()
    }

    pub fn known_dir(id: &GUID) -> Option<PathBuf> {
        unsafe {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            s.map(PathBuf::from)
        }
    }

    pub fn init() {
        unsafe {
            let _ = OleInitialize(None);
            use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
            let dll: Vec<u16> = "uxtheme.dll\0".encode_utf16().collect();
            let hm = LoadLibraryW(dll.as_ptr());
            if !hm.is_null() {
                if let Some(f) = GetProcAddress(hm, 135usize as *const u8) {
                    let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
                    set_mode(2);
                }
                if let Some(f) = GetProcAddress(hm, 136usize as *const u8) {
                    let flush: unsafe extern "system" fn() = std::mem::transmute(f);
                    flush();
                }
            }
        }
    }

    fn wide(p: &PathBuf) -> Vec<u16> { p.as_os_str().encode_wide().chain([0]).collect() }

    pub fn fileop(h: *mut c_void, op: Op, from: &[PathBuf], to: &PathBuf) -> Result<()> {
        unsafe {
            let fo: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            fo.SetOwnerWindow(HWND(h))?;
            fo.SetOperationFlags(match op {
                Op::Delete => FOF_ALLOWUNDO | FOF_NOCONFIRMATION,
                Op::PermDelete => FOF_NOCONFIRMATION,
                _ => FOF_RENAMEONCOLLISION,
            })?;
            let item = |p: &PathBuf| SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide(p).as_ptr()), None);
            let dest = if matches!(op, Op::Delete | Op::PermDelete) { None } else { Some(item(to)?) };
            for p in from {
                let it = item(p)?;
                match (op, &dest) {
                    (Op::Copy, Some(d)) => fo.CopyItem(&it, d, PCWSTR::null(), None)?,
                    (Op::Move, Some(d)) => fo.MoveItem(&it, d, PCWSTR::null(), None)?,
                    _ => fo.DeleteItem(&it, None)?,
                }
            }
            fo.PerformOperations()
        }
    }

    pub fn start_thumbs(main: isize) -> std::sync::mpsc::Sender<(u32, usize, PathBuf)> {
        let (tx, rx) = std::sync::mpsc::channel::<(u32, usize, PathBuf)>();
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let mut wbuf = Vec::with_capacity(260);
            for (era, row, path) in rx {
                if era != super::GEN.load(std::sync::atomic::Ordering::Relaxed) { continue; }
                wbuf.clear();
                wbuf.extend(path.as_os_str().encode_wide());
                wbuf.push(0);
                let Ok(f) = SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(PCWSTR(wbuf.as_ptr()), None) else { continue };
                let Ok(bmp) = f.GetImage(SIZE { cx: super::THUMB, cy: super::THUMB }, SIIGBF_BIGGERSIZEOK) else { continue };
                let padded = pad(bmp.0);
                windows_sys::Win32::Graphics::Gdi::DeleteObject(bmp.0 as _);
                if padded.is_null() { continue; }
                let b = Box::new((era, row, padded as isize));
                let _ = PostMessageW(Some(HWND(main as *mut c_void)), super::WM_THUMB, WPARAM(Box::into_raw(b) as usize), LPARAM(0));
            }
        });
        tx
    }

    unsafe fn pad(src: *mut c_void) -> *mut c_void {
        use windows_sys::Win32::Graphics::Gdi::*;
        unsafe {
            let t = super::THUMB;
            let mut bm: BITMAP = std::mem::zeroed();
            GetObjectW(src as _, size_of::<BITMAP>() as i32, &mut bm as *mut _ as _);
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: t, biHeight: -t, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..std::mem::zeroed() };
            let mut bits = std::ptr::null_mut();
            let dst = CreateDIBSection(std::ptr::null_mut(), &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if dst.is_null() { return std::ptr::null_mut(); }
            let (dc, sdc) = (CreateCompatibleDC(std::ptr::null_mut()), CreateCompatibleDC(std::ptr::null_mut()));
            let (o1, o2) = (SelectObject(dc, dst), SelectObject(sdc, src as _));
            BitBlt(dc, (t - bm.bmWidth) / 2, (t - bm.bmHeight) / 2, bm.bmWidth, bm.bmHeight, sdc, 0, 0, SRCCOPY);
            SelectObject(dc, o1); SelectObject(sdc, o2);
            DeleteDC(dc); DeleteDC(sdc);
            dst
        }
    }

    thread_local! {
        static CUT_STATE: RefCell<(u32, Vec<String>)> = const { RefCell::new((0, Vec::new())) };
    }

    pub fn clip_set(h: *mut c_void, paths: &[PathBuf], cut: bool) -> Result<()> {
        unsafe {
            let data: IDataObject = ui(HWND(h), paths)?;
            let fmt = FORMATETC { cfFormat: RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0 as u32, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
            let hg = GlobalAlloc(GMEM_MOVEABLE, 4)?;
            *(GlobalLock(hg) as *mut u32) = if cut { DROPEFFECT_MOVE.0 } else { DROPEFFECT_COPY.0 };
            let _ = GlobalUnlock(hg);
            let med = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: hg }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
            data.SetData(&fmt, &med, true)?;
            OleSetClipboard(&data)?;
            let seq = windows::Win32::System::DataExchange::GetClipboardSequenceNumber();
            CUT_STATE.with_borrow_mut(|c| {
                c.0 = seq;
                c.1 = if cut { paths.iter().map(|p| p.to_string_lossy().to_lowercase()).collect() } else { Vec::new() };
            });
            Ok(())
        }
    }

    pub fn clip_get() -> Option<(Vec<PathBuf>, bool)> {
        unsafe {
            let o = OleGetClipboard().ok()?;
            let paths = hdrop_paths(&o).ok().filter(|p| !p.is_empty())?;
            let fmt = FORMATETC { cfFormat: RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0 as u32, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
            let cut = o.GetData(&fmt).ok().is_some_and(|mut m| {
                let p = GlobalLock(m.u.hGlobal) as *const u32;
                let v = if p.is_null() { 0 } else { *p };
                let _ = GlobalUnlock(m.u.hGlobal);
                ReleaseStgMedium(&mut m);
                v & DROPEFFECT_MOVE.0 != 0
            });
            Some((paths, cut))
        }
    }

    pub fn clip_clear() {
        unsafe { let _ = OleSetClipboard(None); }
        CUT_STATE.with_borrow_mut(|c| c.1.clear());
    }

    pub fn has_cut() -> bool {
        CUT_STATE.with_borrow(|c| !c.1.is_empty())
    }

    pub fn is_cut(p: &std::path::Path) -> bool {
        CUT_STATE.with_borrow_mut(|c| {
            if c.1.is_empty() { return false; }
            let seq = unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() };
            if seq != c.0 {
                c.1.clear();
                return false;
            }
            let needle = p.to_string_lossy().to_lowercase();
            c.1.iter().any(|x| x == &needle)
        })
    }

    pub fn multi_properties(h: *mut c_void, paths: &[PathBuf]) -> bool {
        unsafe {
            let Ok(data) = ui::<IDataObject>(HWND(h), paths) else { return false };
            SHMultiFileProperties(&data, 0).is_ok()
        }
    }

    struct Pidls(Vec<*mut ITEMIDLIST>);
    impl Drop for Pidls { fn drop(&mut self) { for p in &self.0 { unsafe { CoTaskMemFree(Some(*p as *const c_void)) }; } } }

    fn ui<T: Interface>(h: HWND, paths: &[PathBuf]) -> Result<T> {
        unsafe {
            let mut abs = Pidls(vec![]);
            for p in paths {
                let mut pidl = std::ptr::null_mut();
                SHParseDisplayName(PCWSTR(wide(p).as_ptr()), None, &mut pidl, 0, None)?;
                abs.0.push(pidl);
            }
            let parent: IShellFolder = SHBindToParent(abs.0[0], None)?;
            let kids: Vec<*const ITEMIDLIST> = abs.0.iter().map(|&p| ILFindLastID(p) as *const _).collect();
            parent.GetUIObjectOf(h, &kids, None)
        }
    }

    thread_local! {
        static CM2: RefCell<Option<IContextMenu2>> = const { RefCell::new(None) };
        static CM3: RefCell<Option<IContextMenu3>> = const { RefCell::new(None) };
    }

    unsafe fn run_cm(h: HWND, cm: &IContextMenu, menu: HMENU, base_id: u32) -> (u32, String) {
        unsafe {
            CM2.with_borrow_mut(|c| *c = cm.cast::<IContextMenu2>().ok());
            CM3.with_borrow_mut(|c| *c = cm.cast::<IContextMenu3>().ok());
            let mut pt = POINT::default();
            let id = if GetCursorPos(&mut pt).is_ok() {
                TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, h, None).0 as u32
            } else {
                0
            };
            CM3.with_borrow_mut(|c| *c = None);
            CM2.with_borrow_mut(|c| *c = None);
            if id == 0 {
                return (0, "\0".into());
            }
            if id >= 0x7000 {
                return (id, String::new());
            }
            let cmd = (id - base_id) as usize;
            let mut buf = [0u8; 64];
            let verb = if cm.GetCommandString(cmd, GCS_VERBA, None, PSTR(buf.as_mut_ptr()), 63).is_ok() {
                String::from_utf8_lossy(&buf[..buf.iter().position(|&b| b == 0).unwrap_or(0)]).to_ascii_lowercase()
            } else {
                String::new()
            };
            if matches!(verb.as_str(), "open" | "rename" | "delete" | "cut" | "copy" | "paste") {
                return (id, verb);
            }
            if verb == "display" {
                winrust_common::settings::open("ms-settings:display");
                return (id, String::new());
            }
            if verb == "personalize" {
                winrust_common::settings::open("ms-settings:personalization");
                return (id, String::new());
            }
            let mut ci: CMINVOKECOMMANDINFO = std::mem::zeroed();
            ci.cbSize = size_of::<CMINVOKECOMMANDINFO>() as u32;
            ci.hwnd = h;
            ci.lpVerb = PCSTR(cmd as *const u8);
            ci.nShow = SW_SHOWNORMAL.0;
            let _ = cm.InvokeCommand(&ci);
            (id, String::new())
        }
    }

    pub fn native_known_menu(h: HWND, id: &GUID) -> Option<String> {
        unsafe {
            let pidl = SHGetKnownFolderIDList(id, 0, None).ok()?;
            let res = (|| -> Result<String> {
                let parent: IShellFolder = SHBindToParent(pidl, None)?;
                let child = ILFindLastID(pidl) as *const ITEMIDLIST;
                let cm: IContextMenu = parent.GetUIObjectOf(h, &[child], None)?;
                let menu = CreatePopupMenu()?;
                let out = if cm.QueryContextMenu(menu, 0, 1, 0x6FFF, CMF_NORMAL | CMF_CANRENAME).is_ok() {
                    run_cm(h, &cm, menu, 1).1
                } else {
                    "\0".into()
                };
                let _ = DestroyMenu(menu);
                Ok(out)
            })().ok();
            CoTaskMemFree(Some(pidl as *const c_void));
            res
        }
    }

    pub struct ShellMenu {
        cm: IContextMenu,
        hmenu: HMENU,
    }

    impl Drop for ShellMenu {
        fn drop(&mut self) {
            unsafe { let _ = DestroyMenu(self.hmenu); }
        }
    }

    fn clean_label(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '&' {
                if chars.peek() == Some(&'&') {
                    out.push('&');
                    chars.next();
                }
            } else {
                out.push(c);
            }
        }
        out.split('\t').next().unwrap_or("").trim().to_string()
    }

    unsafe fn read_hmenu(cm: &IContextMenu, cm2: Option<&IContextMenu2>, cm3: Option<&IContextMenu3>, menu: HMENU, top_level: bool) -> Vec<crate::ui::MenuItem> {
        use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, GetMenuItemInfoW, MENUITEMINFOW, MFT_SEPARATOR, MFS_CHECKED, MFS_DISABLED, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU};
        let n = unsafe { GetMenuItemCount(Some(menu)) };
        let mut out = Vec::new();
        let mut pending_sep = false;
        let mut buf = [0u16; 256];
        for pos in 0..n {
            if top_level && out.len() >= 8 {
                break;
            }
            let mut mii: MENUITEMINFOW = unsafe { std::mem::zeroed() };
            mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
            mii.fMask = MIIM_FTYPE | MIIM_ID | MIIM_STATE | MIIM_STRING | MIIM_SUBMENU;
            mii.dwTypeData = windows::core::PWSTR(buf.as_mut_ptr());
            mii.cch = buf.len() as u32;
            if unsafe { GetMenuItemInfoW(menu, pos as u32, true, &mut mii) }.is_err() {
                continue;
            }
            if mii.fType.0 & MFT_SEPARATOR.0 != 0 {
                pending_sep = !out.is_empty();
                continue;
            }
            let raw = String::from_utf16_lossy(&buf[..mii.cch as usize]);
            let label = clean_label(&raw);
            if label.is_empty() {
                continue;
            }
            if !mii.hSubMenu.0.is_null() {
                if top_level {
                    let ll = label.to_ascii_lowercase();
                    if ll.contains("send to") || ll.contains("отправить") || ll.contains("give access") || ll.contains("include in library") || ll.contains("restore previous") {
                        continue;
                    }
                }
                let wp = WPARAM(mii.hSubMenu.0 as usize);
                let lp = LPARAM(pos as isize);
                if let Some(c3) = cm3 {
                    let _ = unsafe { c3.HandleMenuMsg2(0x0117, wp, lp, None) };
                } else if let Some(c2) = cm2 {
                    let _ = unsafe { c2.HandleMenuMsg(0x0117, wp, lp) };
                }
                let sub = unsafe { read_hmenu(cm, cm2, cm3, mii.hSubMenu, false) };
                if sub.is_empty() {
                    continue;
                }
                let mut item = crate::ui::MenuItem::new(0, label, "\u{E712}").sub(sub);
                if pending_sep {
                    item = item.sep();
                    pending_sep = false;
                }
                out.push(item);
                continue;
            }
            let cmd = mii.wID.saturating_sub(1) as usize;
            if top_level {
                let mut vbuf = [0u8; 64];
                if unsafe { cm.GetCommandString(cmd, GCS_VERBA, None, PSTR(vbuf.as_mut_ptr()), 63) }.is_ok() {
                    let verb = String::from_utf8_lossy(&vbuf[..vbuf.iter().position(|&b| b == 0).unwrap_or(0)]).to_ascii_lowercase();
                    if matches!(verb.as_str(), "open" | "opennewprocess" | "opennewtab" | "runas" | "cut" | "copy" | "paste" | "delete" | "rename" | "properties" | "copyaspath" | "pintohome" | "pintostartscreen" | "find" | "explore" | "preview" | "print" | "setwallpaper" | "createshortcut" | "link" | "share" | "includeinlibrary" | "previousversions" | "restore" | "cast" | "compatibility" | "troubleshoot" | "giveaccess" | "sharing") {
                        continue;
                    }
                }
                if out.iter().any(|x: &crate::ui::MenuItem| x.label.eq_ignore_ascii_case(&label)) || out.len() >= 8 {
                    continue;
                }
            }
            let mut item = crate::ui::MenuItem::new(0x1000 + cmd, label, "");
            if mii.fState.0 & MFS_CHECKED.0 != 0 {
                item = item.checked(true);
            }
            if mii.fState.0 & MFS_DISABLED.0 != 0 {
                item = item.disabled(true);
            }
            if pending_sep {
                item = item.sep();
                pending_sep = false;
            }
            out.push(item);
        }
        out
    }

    pub fn query_menu(h: HWND, paths: &[PathBuf]) -> Option<(ShellMenu, Vec<crate::ui::MenuItem>)> {
        unsafe {
            let cm: IContextMenu = ui(h, paths).ok()?;
            let hmenu = CreatePopupMenu().ok()?;
            if cm.QueryContextMenu(hmenu, 0, 1, 0x6FFF, CMF_NORMAL).is_err() {
                let _ = DestroyMenu(hmenu);
                return None;
            }
            let cm2 = cm.cast::<IContextMenu2>().ok();
            let cm3 = cm.cast::<IContextMenu3>().ok();
            let items = read_hmenu(&cm, cm2.as_ref(), cm3.as_ref(), hmenu, true);
            Some((ShellMenu { cm, hmenu }, items))
        }
    }

    pub fn invoke(sm: &ShellMenu, h: HWND, id: usize) {
        if id < 0x1000 {
            return;
        }
        unsafe {
            let mut ci: CMINVOKECOMMANDINFO = std::mem::zeroed();
            ci.cbSize = size_of::<CMINVOKECOMMANDINFO>() as u32;
            ci.hwnd = h;
            ci.lpVerb = PCSTR((id - 0x1000) as *const u8);
            ci.nShow = SW_SHOWNORMAL.0;
            let _ = sm.cm.InvokeCommand(&ci);
        }
    }

    pub fn menu_msg(m: u32, wp: usize, lp: isize) -> bool {
        if CM3.with_borrow(|c| c.as_ref().is_some_and(|c| unsafe { c.HandleMenuMsg2(m, WPARAM(wp), LPARAM(lp), None).is_ok() })) {
            return true;
        }
        CM2.with_borrow(|c| c.as_ref().is_some_and(|c| unsafe { c.HandleMenuMsg(m, WPARAM(wp), LPARAM(lp)).is_ok() }))
    }

    #[cfg(test)]
    pub(crate) fn system_zip_name() -> Option<String> {
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join(r"Microsoft\Windows\SendTo");
        let f = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ZFSendToTarget")))?;
        f.file_stem().map(|s| s.to_string_lossy().into_owned())
    }

    #[cfg(test)]
    unsafe fn hide_system_zip(cm: &IContextMenu, menu: HMENU) {
        use windows::Win32::UI::WindowsAndMessaging::{DeleteMenu, GetMenuItemCount, GetMenuStringW, GetSubMenu, MF_BYPOSITION};
        let Some(name) = system_zip_name() else { return };
        if let Ok(c2) = cm.cast::<IContextMenu2>() { let _ = unsafe { c2.HandleMenuMsg(0x0117, WPARAM(menu.0 as usize), LPARAM(0)) }; }
        unsafe fn strip(m: HMENU, name: &str) { unsafe {
            for i in (0..GetMenuItemCount(Some(m)).max(0)).rev() {
                let mut b = [0u16; 256];
                let n = GetMenuStringW(m, i as u32, Some(&mut b), MF_BYPOSITION);
                if String::from_utf16_lossy(&b[..n.max(0) as usize]).replace('&', "").eq_ignore_ascii_case(name) {
                    let _ = DeleteMenu(m, i as u32, MF_BYPOSITION);
                    continue;
                }
                let sub = GetSubMenu(m, i);
                if !sub.is_invalid() { strip(sub, name); }
            }
        } }
        unsafe { strip(menu, &name) };
    }

    #[cfg(test)]
    pub fn menu_dump(paths: &[PathBuf]) -> Vec<String> {
        use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, GetMenuItemID, GetMenuStringW, GetSubMenu, MF_BYPOSITION};
        fn walk(m: HMENU, depth: usize, out: &mut Vec<String>) {
            unsafe {
                for i in 0..GetMenuItemCount(Some(m)).max(0) as u32 {
                    let mut b = [0u16; 256];
                    let n = GetMenuStringW(m, i, Some(&mut b), MF_BYPOSITION);
                    out.push(format!("{}{} (id {})", "  ".repeat(depth), String::from_utf16_lossy(&b[..n.max(0) as usize]), GetMenuItemID(m, i as i32)));
                    let sub = GetSubMenu(m, i as i32);
                    if !sub.is_invalid() { walk(sub, depth + 1, out); }
                }
            }
        }
        let mut out = vec![];
        unsafe {
            let cm: IContextMenu = ui(HWND::default(), paths).unwrap();
            let menu = CreatePopupMenu().unwrap();
            cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL).ok().unwrap();
            hide_system_zip(&cm, menu);
            walk(menu, 0, &mut out);
            let _ = DestroyMenu(menu);
        }
        out
    }

    #[implement(windows::Win32::System::Ole::IDropSource)]
    struct DropSrc { desk_move: bool }

    impl windows::Win32::System::Ole::IDropSource_Impl for DropSrc_Impl {
        fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> windows::core::HRESULT {
            if escape_pressed.as_bool() {
                return windows::core::HRESULT(0x0004_0101);
            }
            if key_state.0 & 1 == 0 {
                return windows::core::HRESULT(0x0004_0100);
            }
            windows::core::HRESULT(0)
        }
        fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
            if self.desk_move {
                unsafe {
                    let cur = windows_sys::Win32::UI::WindowsAndMessaging::LoadCursorW(std::ptr::null_mut(), windows_sys::Win32::UI::WindowsAndMessaging::IDC_ARROW);
                    windows_sys::Win32::UI::WindowsAndMessaging::SetCursor(cur);
                }
                windows::core::HRESULT(0)
            } else {
                windows::core::HRESULT(0x0004_0102)
            }
        }
    }

    fn drag_bitmap(path: &std::path::Path) -> Option<(isize, i32)> {
        use windows_sys::Win32::Graphics::Gdi::{CreateCompatibleBitmap, CreateCompatibleDC, DC_BRUSH, DeleteDC, DeleteObject, FillRect, GetDC, ReleaseDC, SelectObject, SetDCBrushColor};
        use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SYSICONINDEX, SHGetFileInfoW};
        use windows_sys::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx};
        let sz = crate::ui::sc(48);
        let wp = crate::w(&path.to_string_lossy());
        unsafe {
            let mut fi: SHFILEINFOW = std::mem::zeroed();
            let mut hicon = std::ptr::null_mut();
            if SHGetFileInfoW(wp.as_ptr(), 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | SHGFI_LARGEICON) != 0 {
                if let Ok(il) = SHGetImageList::<windows::Win32::UI::Controls::IImageList>(windows::Win32::UI::Shell::SHIL_EXTRALARGE as i32) {
                    if let Ok(ic) = il.GetIcon(fi.iIcon, 1) {
                        hicon = ic.0;
                    }
                }
            }
            if hicon.is_null() {
                let mut fi2: SHFILEINFOW = std::mem::zeroed();
                if SHGetFileInfoW(wp.as_ptr(), 0, &mut fi2, size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_LARGEICON) != 0 {
                    hicon = fi2.hIcon;
                }
            }
            if hicon.is_null() {
                return None;
            }
            let sdc = GetDC(std::ptr::null_mut());
            let mdc = CreateCompatibleDC(sdc);
            let bmp = CreateCompatibleBitmap(sdc, sz, sz);
            ReleaseDC(std::ptr::null_mut(), sdc);
            if mdc.is_null() || bmp.is_null() {
                if !mdc.is_null() { DeleteDC(mdc); }
                if !bmp.is_null() { DeleteObject(bmp); }
                DestroyIcon(hicon);
                return None;
            }
            let old = SelectObject(mdc, bmp);
            let r = windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: sz, bottom: sz };
            SetDCBrushColor(mdc, 0x00FF_00FF);
            FillRect(mdc, &r, windows_sys::Win32::Graphics::Gdi::GetStockObject(DC_BRUSH as i32) as _);
            DrawIconEx(mdc, 0, 0, hicon, sz, sz, 0, std::ptr::null_mut(), DI_NORMAL);
            SelectObject(mdc, old);
            DeleteDC(mdc);
            DestroyIcon(hicon);
            Some((bmp as isize, sz))
        }
    }

    pub fn drag(h: *mut c_void, paths: &[PathBuf]) -> Result<()> {
        use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HGDIOBJ};
        use windows::Win32::System::Ole::IDropSource;
        use windows::Win32::UI::Shell::{CLSID_DragDropHelper, IDragSourceHelper, SHDRAGIMAGE};
        unsafe {
            let hwnd = HWND(h);
            let data: IDataObject = ui(hwnd, paths)?;
            if let (Some(first), Ok(helper)) = (paths.first(), CoCreateInstance::<_, IDragSourceHelper>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER)) {
                if let Some((bmp, sz)) = drag_bitmap(first) {
                    let shdi = SHDRAGIMAGE {
                        sizeDragImage: SIZE { cx: sz, cy: sz },
                        ptOffset: POINT { x: sz / 2, y: sz / 2 },
                        hbmpDragImage: HBITMAP(bmp as *mut _),
                        crColorKey: COLORREF(0x00FF_00FF),
                    };
                    if helper.InitializeFromBitmap(&shdi, &data).is_err() {
                        let _ = DeleteObject(HGDIOBJ(bmp as *mut _));
                    }
                }
            }
            let src: IDropSource = DropSrc { desk_move: super::desk() }.into();
            SHDoDragDrop(Some(hwnd), &data, Some(&src), DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK)?;
            Ok(())
        }
    }

    fn hdrop_paths(o: &IDataObject) -> Result<Vec<PathBuf>> {
        unsafe {
            let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0 as u32, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
            let mut med = o.GetData(&fmt)?;
            let hd = HDROP(med.u.hGlobal.0);
            let count = DragQueryFileW(hd, u32::MAX, None);
            let mut out = Vec::with_capacity(count as usize);
            let mut buf = Vec::with_capacity(260);
            for i in 0..count {
                let len = DragQueryFileW(hd, i, None) as usize + 1;
                buf.resize(len, 0);
                let n = DragQueryFileW(hd, i, Some(&mut buf)) as usize;
                out.push(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
            }
            ReleaseStgMedium(&mut med);
            Ok(out)
        }
    }

    #[implement(IDropTarget)]
    struct DropT {
        main: isize,
        hwnd: HWND,
        helper: Option<windows::Win32::UI::Shell::IDropTargetHelper>,
    }

    fn effect(k: MODIFIERKEYS_FLAGS) -> DROPEFFECT { if k.0 & 8 != 0 { DROPEFFECT_COPY } else { DROPEFFECT_MOVE } }

    impl IDropTarget_Impl for DropT_Impl {
        fn DragEnter(&self, o: Ref<'_, IDataObject>, k: MODIFIERKEYS_FLAGS, pt: &POINTL, e: *mut DROPEFFECT) -> Result<()> {
            let eff = if super::in_zip() { DROPEFFECT_NONE } else { effect(k) };
            unsafe { *e = eff };
            if let (Some(hp), Some(d)) = (&self.helper, o.as_ref()) {
                let p = POINT { x: pt.x, y: pt.y };
                unsafe { let _ = hp.DragEnter(self.hwnd, d, &p, eff); }
            }
            Ok(())
        }
        fn DragOver(&self, k: MODIFIERKEYS_FLAGS, pt: &POINTL, e: *mut DROPEFFECT) -> Result<()> {
            let eff = if super::in_zip() { DROPEFFECT_NONE } else { effect(k) };
            unsafe { *e = eff };
            if let Some(hp) = &self.helper {
                let p = POINT { x: pt.x, y: pt.y };
                unsafe { let _ = hp.DragOver(&p, eff); }
            }
            Ok(())
        }
        fn DragLeave(&self) -> Result<()> {
            if let Some(hp) = &self.helper {
                unsafe { let _ = hp.DragLeave(); }
            }
            Ok(())
        }
        fn Drop(&self, o: Ref<'_, IDataObject>, k: MODIFIERKEYS_FLAGS, pt: &POINTL, e: *mut DROPEFFECT) -> Result<()> {
            let skip = if super::desk::dragging() {
                super::desk::dropped_here((pt.x, pt.y));
                true
            } else {
                super::in_zip()
            };
            if skip {
                unsafe { *e = DROPEFFECT_NONE };
                if let Some(hp) = &self.helper {
                    unsafe { let _ = hp.DragLeave(); }
                }
                return Ok(());
            }
            let d = o.ok()?;
            let paths = hdrop_paths(d)?;
            let to = super::cur_dir();
            let copy = k.0 & 8 != 0 || paths.first().and_then(|p| p.components().next()) != to.components().next();
            let eff = if copy { DROPEFFECT_COPY } else { DROPEFFECT_MOVE };
            unsafe { *e = eff };
            if let Some(hp) = &self.helper {
                let p = POINT { x: pt.x, y: pt.y };
                unsafe { let _ = hp.Drop(d, &p, eff); }
            }
            let _ = fileop(self.main as *mut c_void, if copy { Op::Copy } else { Op::Move }, &paths, &to);
            unsafe { let _ = PostMessageW(Some(HWND(self.main as *mut c_void)), WM_APP, WPARAM(0), LPARAM(0)); }
            Ok(())
        }
    }

    pub fn register_drop(list: *mut c_void, main: isize) -> Result<()> {
        use windows::Win32::UI::Shell::{CLSID_DragDropHelper, IDropTargetHelper};
        let hwnd = HWND(list);
        let helper = unsafe { CoCreateInstance::<_, IDropTargetHelper>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok() };
        let t: IDropTarget = DropT { main, hwnd, helper }.into();
        unsafe { RegisterDragDrop(hwnd, &t) }
    }
}

mod desk {
    #![allow(unsafe_op_in_unsafe_fn)]
    use super::*;
    use std::{cell::Cell, collections::{HashMap, HashSet}};
    use windows_sys::Win32::Graphics::Gdi::{DC_BRUSH, FillRect, HBRUSH, HDC, ScreenToClient, SetDCBrushColor};

    pub const SPAWN_WORKERW: u32 = 0x052C;
    const KEY: u32 = 0x0001_0001;

    thread_local! {
        static DRAG_FROM: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
        static DROP_AT: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
        static WORKERW: Cell<isize> = const { Cell::new(0) };
        static LIVE: Cell<bool> = const { Cell::new(false) };
        static LIST_ORIG: Cell<isize> = const { Cell::new(0) };
    }

    unsafe fn hit_near_item(list: HWND, x: i32, y: i32) -> Option<(i32, i32)> {
        let mut ht: LVHITTESTINFO = std::mem::zeroed();
        ht.pt = POINT { x, y };
        if put(list, LVM_HITTEST, 0, &mut ht as *mut _ as isize) >= 0 {
            return None;
        }
        let n = put(list, LVM_GETITEMCOUNT, 0, 0) as i32;
        let pad_x = ui::sc(14);
        let pad_y = ui::sc(8);
        let mut best: Option<(i32, i64, i32, i32)> = None;
        for i in 0..n {
            let mut r = RECT { left: LVIR_BOUNDS as i32, top: 0, right: 0, bottom: 0 };
            if put(list, LVM_GETITEMRECT, i as usize, &mut r as *mut _ as isize) == 0 {
                continue;
            }
            if x >= r.left - pad_x && x <= r.right + pad_x && y >= r.top - pad_y && y <= r.bottom + pad_y {
                let mut ir = RECT { left: LVIR_ICON as i32, top: 0, right: 0, bottom: 0 };
                let (cx, cy) = if put(list, LVM_GETITEMRECT, i as usize, &mut ir as *mut _ as isize) != 0 {
                    ((ir.left + ir.right) / 2, (ir.top + ir.bottom) / 2)
                } else {
                    ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
                };
                let d2 = ((x - cx) as i64).pow(2) + ((y - cy) as i64).pow(2);
                if best.is_none_or(|b| d2 < b.1) {
                    best = Some((i, d2, cx, cy));
                }
            }
        }
        best.map(|(_, _, cx, cy)| (cx, cy))
    }

    pub unsafe extern "system" fn list_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let orig: WNDPROC = std::mem::transmute(LIST_ORIG.get());
        let lp = if matches!(m, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN) {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            if let Some((cx, cy)) = hit_near_item(h, x, y) {
                ((cy as u16 as isize) << 16) | (cx as u16 as isize)
            } else {
                lp
            }
        } else {
            lp
        };
        CallWindowProcW(orig, h, m, wp, lp)
    }

    pub unsafe fn hook_list(list: HWND) {
        if LIST_ORIG.get() != 0 {
            return;
        }
        let prev = SetWindowLongPtrW(list, GWLP_WNDPROC, list_proc as *const () as isize);
        if prev != 0 {
            LIST_ORIG.set(prev);
        }
    }

    pub unsafe fn size_workerw(cw: i32, ch: i32) {
        let ww = WORKERW.get() as HWND;
        if !ww.is_null() && IsWindow(ww) != 0 {
            MoveWindow(ww, 0, 0, cw, ch, 1);
        }
    }

    pub fn pos_file() -> PathBuf {
        PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join(r"rust-explorer\desktop-icons.txt")
    }

    fn saved() -> HashMap<String, (i32, i32)> {
        fs::read_to_string(pos_file()).unwrap_or_default().lines().filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.to_string()))
        }).map(|(x, y, n): (i32, i32, String)| (n, (x, y))).collect()
    }

    fn pos(list: HWND, i: usize) -> (i32, i32) {
        let mut p = POINT { x: 0, y: 0 };
        put(list, LVM_GETITEMPOSITION, i, &mut p as *mut _ as isize);
        (p.x, p.y)
    }

    fn set_pos(list: HWND, i: usize, (x, y): (i32, i32)) {
        let p = POINT { x, y };
        put(list, LVM_SETITEMPOSITION32, i, &p as *const _ as isize);
    }

    pub fn save(list: HWND) {
        use std::fmt::Write as _;
        let mut lines = String::new();
        ST.with_borrow(|st| {
            for (i, r) in st.rows.iter().enumerate() {
                let (x, y) = pos(list, i);
                let _ = writeln!(lines, "{x}\t{y}\t{}", r.name);
            }
        });
        let f = pos_file();
        let _ = fs::create_dir_all(f.parent().unwrap_or(&f));
        let _ = fs::write(f, lines);
    }

    struct Grid { cx: i32, cy: i32, cols: i32, rows: i32 }

    impl Grid {
        fn of(list: HWND) -> Grid {
            let s = put(list, LVM_GETITEMSPACING, 0, 0) as u32;
            let (cx, cy) = (((s & 0xFFFF) as i32).max(1), ((s >> 16) as i32).max(1));
            let mut r: RECT = unsafe { std::mem::zeroed() };
            unsafe { GetClientRect(list, &mut r) };
            if r.bottom <= 0 {
                r = unsafe { RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) - desk_top() - desk_bottom() } };
            }
            Grid { cx, cy, cols: (r.right / cx).max(1), rows: (r.bottom / cy).max(1) }
        }
        fn cell(&self, (x, y): (i32, i32)) -> Option<(i32, i32)> {
            let c = ((x + self.cx / 2).div_euclid(self.cx), (y + self.cy / 2).div_euclid(self.cy));
            (c.0 >= 0 && c.1 >= 0 && c.0 < self.cols && c.1 < self.rows).then_some(c)
        }
        fn at(&self, (c, r): (i32, i32)) -> (i32, i32) { (c * self.cx, r * self.cy) }
        fn free(&self, taken: &HashSet<(i32, i32)>, want: Option<(i32, i32)>) -> (i32, i32) {
            let all = (0..self.cols).flat_map(|c| (0..self.rows).map(move |r| (c, r))).filter(|c| !taken.contains(c));
            match want {
                Some((wc, wr)) => all.min_by_key(|&(c, r)| (c - wc).pow(2) + (r - wr).pow(2)),
                None => all.min(),
            }
            .unwrap_or((0, 0))
        }
    }

    pub fn fill(list: HWND, n: usize) {
        for i in 0..n {
            let mut it: LVITEMW = unsafe { std::mem::zeroed() };
            it.mask = LVIF_TEXT | LVIF_IMAGE;
            it.iItem = i as i32;
            it.pszText = usize::MAX as *mut u16;
            it.iImage = -1;
            put(list, LVM_INSERTITEMW, 0, &it as *const _ as isize);
        }
        let (g, old) = (Grid::of(list), saved());
        let mut taken = HashSet::new();
        let mut later = vec![];
        ST.with_borrow(|st| {
            for (i, r) in st.rows.iter().enumerate() {
                match old.get(&r.name).and_then(|&p| g.cell(p)).filter(|c| !taken.contains(c)) {
                    Some(c) => { taken.insert(c); set_pos(list, i, g.at(c)); }
                    None => later.push(i),
                }
            }
        });
        for i in later {
            let c = g.free(&taken, None);
            taken.insert(c);
            set_pos(list, i, g.at(c));
        }
        save(list);
    }

    pub fn drag_start(from: (i32, i32)) { DRAG_FROM.set(Some(from)); DROP_AT.set(None); }
    pub fn dragging() -> bool { DRAG_FROM.get().is_some() }
    pub fn dropped_here(at: (i32, i32)) { DROP_AT.set(Some(at)); }

    pub fn drag_end(h: HWND, list: HWND) -> bool {
        let Some(from) = DRAG_FROM.take() else { return false };
        let at = DROP_AT.take().or_else(|| unsafe {
            let mut c = POINT { x: 0, y: 0 };
            GetCursorPos(&mut c);
            (GetAncestor(WindowFromPoint(c), GA_ROOT) == h).then_some((c.x, c.y))
        });
        let Some(at) = at else { return false };
        let mut pt = POINT { x: at.0, y: at.1 };
        unsafe { ScreenToClient(list, &mut pt) };
        let (_, dir, paths) = selected();
        let mut sel = vec![];
        let mut i = -1isize;
        loop {
            i = put(list, LVM_GETNEXTITEM, i as usize, LVNI_SELECTED as isize);
            if i < 0 { break; }
            sel.push(i as usize);
        }
        let mut ht: LVHITTESTINFO = unsafe { std::mem::zeroed() };
        ht.pt = pt;
        let tgt = put(list, LVM_HITTEST, 0, &mut ht as *mut _ as isize);
        if tgt >= 0 && !sel.contains(&(tgt as usize)) {
            let target = ST.with_borrow(|st| st.rows.get(tgt as usize).filter(|r| r.dir).map(|r| st.dir.join(&r.name)));
            if let Some(t) = target {
                let _ = sh::fileop(h as _, sh::Op::Move, &paths, &t);
                let _ = dir;
                unsafe { PostMessageW(h, WM_APP, 0, 0) };
                return true;
            }
        }
        let g = Grid::of(list);
        let n = put(list, LVM_GETITEMCOUNT, 0, 0).max(0) as usize;
        let mut taken: HashSet<(i32, i32)> = (0..n).filter(|i| !sel.contains(i)).filter_map(|i| g.cell(pos(list, i))).collect();
        let (dx, dy) = (pt.x - from.0, pt.y - from.1);
        for &i in &sel {
            let (x, y) = pos(list, i);
            let want = g.cell((x + dx, y + dy)).unwrap_or_else(|| {
                (((x + dx) / g.cx).clamp(0, g.cols - 1), ((y + dy) / g.cy).clamp(0, g.rows - 1))
            });
            let c = if taken.contains(&want) { g.free(&taken, Some(want)) } else { want };
            taken.insert(c);
            set_pos(list, i, g.at(c));
        }
        save(list);
        true
    }

    pub unsafe fn spawn_workerw(h: HWND) {
        if WORKERW.get() != 0 && IsWindow(WORKERW.get() as HWND) != 0 { return; }
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(h, &mut r);
        let ww = CreateWindowExW(0, w("WorkerW").as_ptr(), ptr::null(), WS_CHILD | WS_VISIBLE, 0, 0, r.right, r.bottom, h, 0 as _, GetModuleHandleW(ptr::null()), ptr::null());
        SetWindowPos(ww, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        WORKERW.set(ww as isize);
    }

    pub unsafe fn check_live(list: HWND) {
        let ww = WORKERW.get() as HWND;
        let live = !ww.is_null() && IsWindow(ww) != 0 && !GetWindow(ww, GW_CHILD).is_null();
        if live == LIVE.replace(live) { return; }
        let ex = GetWindowLongPtrW(list, GWL_EXSTYLE);
        if live {
            SetWindowLongPtrW(list, GWL_EXSTYLE, ex | WS_EX_LAYERED as isize);
            SetLayeredWindowAttributes(list, KEY, 0, LWA_COLORKEY);
        } else {
            SetWindowLongPtrW(list, GWL_EXSTYLE, ex & !(WS_EX_LAYERED as isize));
        }
        InvalidateRect(list, ptr::null(), 1);
    }

    pub unsafe fn erase(dc: HDC, h: HWND) -> bool {
        let (bmp, cw, ch) = WALL.get();
        if LIVE.get() {
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(h, &mut r);
            SetDCBrushColor(dc, KEY);
            FillRect(dc, &r, GetStockObject(DC_BRUSH as i32) as HBRUSH);
        } else if bmp != 0 {
            wall::paint(dc, bmp, cw, ch);
        } else {
            return false;
        }
        true
    }

    pub unsafe extern "system" fn workerw_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if m == WM_ERASEBKGND {
            let (bmp, cw, ch) = WALL.get();
            if bmp != 0 { wall::paint(wp as _, bmp, cw, ch); } else { let mut r: RECT = std::mem::zeroed(); GetClientRect(h, &mut r); FillRect(wp as _, &r, windows_sys::Win32::Graphics::Gdi::GetSysColorBrush(1) as _); }
            return 1;
        }
        DefWindowProcW(h, m, wp, lp)
    }

    #[cfg(test)]
    mod tests {
        use super::Grid;
        use std::collections::HashSet;

        #[test]
        fn grid_cells() {
            let g = Grid { cx: 100, cy: 90, cols: 5, rows: 4 };
            assert_eq!(g.cell((0, 0)), Some((0, 0)));
            assert_eq!(g.cell((149, 44)), Some((1, 0)));
            assert_eq!(g.cell((900, 0)), None);
            let mut taken: HashSet<_> = [(0, 0), (0, 1)].into();
            assert_eq!(g.free(&taken, None), (0, 2));
            taken.insert((2, 2));
            assert_eq!(g.free(&taken, Some((2, 2))), (1, 2));
        }
    }
}

fn drag_log(s: &str) {
    winrust_common::log("rust-explorer", s);
}

unsafe fn delete_word(h: HWND) {
    unsafe {
        let (mut a, mut b) = (0u32, 0u32);
        SendMessageW(h, EM_GETSEL, &mut a as *mut u32 as usize, &mut b as *mut u32 as isize);
        if a == b {
            let n = GetWindowTextLengthW(h) as usize;
            let mut buf = vec![0u16; n + 1];
            GetWindowTextW(h, buf.as_mut_ptr(), n as i32 + 1);
            let mut i = a as usize;
            while i > 0 && buf[i - 1] == b' ' as u16 { i -= 1; }
            while i > 0 && buf[i - 1] != b' ' as u16 { i -= 1; }
            SendMessageW(h, EM_SETSEL, i, a as isize);
        }
        SendMessageW(h, EM_REPLACESEL, 1, w("").as_ptr() as isize);
    }
}

unsafe extern "system" fn edit_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if matches!(wp, 13 | 27) && matches!(m, WM_KEYDOWN | WM_CHAR) {
            if m == WM_KEYDOWN { PostMessageW(GetAncestor(h, GA_ROOT), WM_GO, wp, 0); }
            return 0;
        }
        if m == WM_CHAR && wp == 0x7F { delete_word(h); return 0; }
        CallWindowProcW(std::mem::transmute::<isize, WNDPROC>(EDIT_OLD.load(std::sync::atomic::Ordering::Relaxed)), h, m, wp, lp)
    }
}

const WM_SHUTDOWN_DLG: u32 = WM_APP + 20;
const WM_RESIDENT: u32 = WM_APP + 21;
const WM_DPICHANGED: u32 = 0x02E0;
static SHUTDOWN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

const SHUTDOWN_OPTS: [(&str, &str); 5] = [
    ("Shut down", "Closes all apps and turns off the PC."),
    ("Restart", "Closes all apps, turns off the PC, and then turns it on again."),
    ("Sleep", "The PC stays on but uses low power. Apps stay open so when the PC wakes up, you're back to where you left off."),
    ("Sign out", "Closes all apps and signs you out."),
    ("Switch user", "Switches users without closing apps."),
];

fn run_power_action(idx: usize) {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    match idx {
        0 => spawn_gui(std::process::Command::new("shutdown").args(["/s", "/t", "0"])),
        1 => spawn_gui(std::process::Command::new("shutdown").args(["/r", "/t", "0"])),
        2 => unsafe {
            let lib = LoadLibraryW(w("powrprof.dll").as_ptr());
            if !lib.is_null() {
                if let Some(f) = GetProcAddress(lib, b"SetSuspendState\0".as_ptr()) {
                    let susp: unsafe extern "system" fn(u8, u8, u8) -> u8 = std::mem::transmute(f);
                    susp(0, 0, 0);
                }
                FreeLibrary(lib);
            }
        },
        3 => unsafe {
            #[link(name = "user32")]
            unsafe extern "system" {
                fn ExitWindowsEx(flags: u32, reason: u32) -> i32;
            }
            ExitWindowsEx(0, 0);
        },
        4 => unsafe {
            let lib = LoadLibraryW(w("wtsapi32.dll").as_ptr());
            if !lib.is_null() {
                if let Some(f) = GetProcAddress(lib, b"WTSDisconnectSession\0".as_ptr()) {
                    let disc: unsafe extern "system" fn(isize, u32, i32) -> i32 = std::mem::transmute(f);
                    disc(0, u32::MAX, 0);
                }
                FreeLibrary(lib);
            }
        },
        _ => {}
    }
}

static SHUTDOWN_FONTS: std::sync::Mutex<[isize; 3]> = std::sync::Mutex::new([0; 3]);
static SHUTDOWN_REG: std::sync::Once = std::sync::Once::new();
thread_local! {
    static SHUTDOWN_STATE: std::cell::Cell<(usize, i8)> = const { std::cell::Cell::new((0, -1)) };
}

fn shutdown_rects(cw_full: i32) -> ([RECT; 5], RECT, RECT, RECT) {
    let pad = ui::sc(22);
    let cw = (cw_full - pad * 2).max(ui::sc(180));
    let mut rows = [RECT { left: 0, top: 0, right: 0, bottom: 0 }; 5];
    let mut y = ui::sc(84);
    let rh = ui::sc(32);
    for r in &mut rows {
        *r = ui::rc(pad, y, pad + cw, y + rh);
        y += rh + ui::sc(4);
    }
    let desc = ui::rc(pad, y + ui::sc(6), pad + cw, y + ui::sc(42));
    let bw = ui::sc(92);
    let bh = ui::sc(30);
    let by = y + ui::sc(50);
    let ok = ui::rc(pad + cw - bw * 2 - ui::sc(10), by, pad + cw - bw - ui::sc(10), by + bh);
    let cancel = ui::rc(pad + cw - bw, by, pad + cw, by + bh);
    (rows, desc, ok, cancel)
}

fn shutdown_hit(cw_full: i32, x: i32, y: i32) -> i8 {
    let hit = |r: RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    let (rows, _, ok, cancel) = shutdown_rects(cw_full);
    for (i, r) in rows.iter().enumerate() {
        if hit(*r) { return i as i8; }
    }
    if hit(ok) { return 10; }
    if hit(cancel) { return 11; }
    -1
}

unsafe extern "system" fn shutdown_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    use windows_sys::Win32::Graphics::Gdi::{BeginPaint, DT_CENTER, DT_LEFT, DT_WORDBREAK, DeleteObject, EndPaint, HGDIOBJ, PAINTSTRUCT, SelectObject, SetBkMode, TRANSPARENT};
    unsafe {
        match m {
            WM_CREATE => {
                ui::dark_title(h);
                let _ = windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(h, 33, &2u32 as *const u32 as _, 4);
                *SHUTDOWN_FONTS.lock().unwrap() = [
                    ui::font(20, 600, ui::ui_face()),
                    ui::font(13, 400, ui::ui_face()),
                    ui::font(12, 400, ui::ui_face()),
                ];
                SHUTDOWN_STATE.set((0, -1));
                SetFocus(h);
                0
            }
            WM_GETDLGCODE => 0x0087,
            WM_ACTIVATE => {
                if (wp & 0xFFFF) != 0 {
                    SetFocus(h);
                }
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(h, &mut ps);
                let mut cr: RECT = std::mem::zeroed();
                GetClientRect(h, &mut cr);
                let pal = ui::pal();
                ui::fill(dc, cr, pal.bg, 0);
                let [f20, f13, f12] = *SHUTDOWN_FONTS.lock().unwrap();
                SetBkMode(dc, TRANSPARENT as i32);
                let of = SelectObject(dc, f20 as HGDIOBJ);
                let pad = ui::sc(22);
                let cw = (cr.right - pad * 2).max(ui::sc(180));
                ui::text(dc, "Windows 11", ui::rc(pad, ui::sc(18), pad + cw, ui::sc(46)), pal.text, DT_LEFT);
                SelectObject(dc, f13 as HGDIOBJ);
                ui::text(dc, "What do you want the computer to do?", ui::rc(pad, ui::sc(52), pad + cw, ui::sc(74)), pal.dim, DT_LEFT);
                let (sel, hov) = SHUTDOWN_STATE.get();
                let (rows, desc, ok, cancel) = shutdown_rects(cr.right);
                for (i, &r) in rows.iter().enumerate() {
                    let is_sel = i == sel;
                    let is_hot = hov == i as i8;
                    if is_sel || is_hot {
                        let c = if is_sel {
                            ui::mix(pal.bg, pal.accent, if is_hot { 0.28 } else { 0.20 })
                        } else {
                            ui::mix(pal.bg, pal.text, 0.08)
                        };
                        ui::fill(dc, r, c, ui::sc(8));
                    }
                    if is_sel {
                        ui::fill(dc, ui::rc(r.left, r.top + ui::sc(7), r.left + ui::sc(3), r.bottom - ui::sc(7)), pal.accent, ui::sc(3));
                    }
                    ui::text(dc, SHUTDOWN_OPTS[i].0, ui::rc(r.left + ui::sc(12), r.top, r.right - ui::sc(10), r.bottom), pal.text, DT_LEFT);
                }
                SelectObject(dc, f12 as HGDIOBJ);
                ui::text(dc, SHUTDOWN_OPTS[sel].1, desc, pal.dim, DT_LEFT | DT_WORDBREAK);
                SelectObject(dc, f13 as HGDIOBJ);
                let ok_bg = if hov == 10 { ui::mix(pal.accent, 0x00FF_FFFF, 0.14) } else { pal.accent };
                ui::fill(dc, ok, ok_bg, ui::sc(6));
                ui::text(dc, "OK", ok, winrust_common::style::on_accent(), DT_CENTER);
                let c_bg = ui::mix(pal.bg, pal.text, if hov == 11 { 0.14 } else { 0.08 });
                ui::fill(dc, cancel, c_bg, ui::sc(6));
                ui::text(dc, "Cancel", cancel, pal.text, DT_CENTER);
                SelectObject(dc, of);
                EndPaint(h, &ps);
                0
            }
            WM_MOUSEMOVE => {
                let (x, y) = ((lp & 0xFFFF) as i16 as i32, (lp >> 16) as i16 as i32);
                let mut cr: RECT = std::mem::zeroed();
                GetClientRect(h, &mut cr);
                let (sel, hov) = SHUTDOWN_STATE.get();
                let nh = shutdown_hit(cr.right, x, y);
                if nh != hov {
                    SHUTDOWN_STATE.set((sel, nh));
                    InvalidateRect(h, ptr::null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let (x, y) = ((lp & 0xFFFF) as i16 as i32, (lp >> 16) as i16 as i32);
                let mut cr: RECT = std::mem::zeroed();
                GetClientRect(h, &mut cr);
                let (sel, hov) = SHUTDOWN_STATE.get();
                match shutdown_hit(cr.right, x, y) {
                    i @ 0..=4 => {
                        SHUTDOWN_STATE.set((i as usize, hov));
                        InvalidateRect(h, ptr::null(), 0);
                    }
                    10 => {
                        DestroyWindow(h);
                        run_power_action(sel);
                    }
                    11 => { DestroyWindow(h); }
                    _ => {}
                }
                0
            }
            WM_KEYDOWN => {
                let (sel, hov) = SHUTDOWN_STATE.get();
                match wp as u16 {
                    0x26 | 0x25 => {
                        let ns = if sel == 0 { SHUTDOWN_OPTS.len() - 1 } else { sel - 1 };
                        SHUTDOWN_STATE.set((ns, hov));
                        InvalidateRect(h, ptr::null(), 0);
                    }
                    0x28 | 0x27 | 0x09 => {
                        SHUTDOWN_STATE.set(((sel + 1) % SHUTDOWN_OPTS.len(), hov));
                        InvalidateRect(h, ptr::null(), 0);
                    }
                    0x0D | 0x20 => {
                        DestroyWindow(h);
                        run_power_action(sel);
                    }
                    0x1B => { DestroyWindow(h); }
                    _ => {}
                }
                0
            }
            WM_DPICHANGED => {
                ui::set_scale_dpi((wp & 0xFFFF) as u32);
                for f in std::mem::take(&mut *SHUTDOWN_FONTS.lock().unwrap()) {
                    if f != 0 { DeleteObject(f as HGDIOBJ); }
                }
                *SHUTDOWN_FONTS.lock().unwrap() = [
                    ui::font(20, 600, ui::ui_face()),
                    ui::font(13, 400, ui::ui_face()),
                    ui::font(12, 400, ui::ui_face()),
                ];
                let r = &*(lp as *const RECT);
                SetWindowPos(h, ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                InvalidateRect(h, ptr::null(), 0);
                0
            }
            WM_CLOSE => { DestroyWindow(h); 0 }
            WM_DESTROY => {
                for f in std::mem::take(&mut *SHUTDOWN_FONTS.lock().unwrap()) {
                    if f != 0 { DeleteObject(f as HGDIOBJ); }
                }
                SHUTDOWN_HWND.store(0, Relaxed);
                0
            }
            _ => DefWindowProcW(h, m, wp, lp),
        }
    }
}

fn show_shutdown_dialog(_h: HWND) {
    let cur = SHUTDOWN_HWND.load(Relaxed) as HWND;
    if !cur.is_null() && unsafe { IsWindow(cur) != 0 } {
        unsafe { SetForegroundWindow(cur); }
        return;
    }
    unsafe {
        let inst = GetModuleHandleW(ptr::null());
        let cls = w("RxShutdownDlg");
        SHUTDOWN_REG.call_once(|| {
            let bg = windows_sys::Win32::Graphics::Gdi::CreateSolidBrush(ui::pal().bg);
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(shutdown_proc), hInstance: inst, lpszClassName: cls.as_ptr(), hCursor: LoadCursorW(0 as _, IDC_ARROW), hbrBackground: bg, ..std::mem::zeroed() });
        });
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let wa = winrust_common::ui::work_area();
        let mut r = RECT { left: 0, top: 0, right: ui::sc(420).min(wa.right - wa.left - ui::sc(16)), bottom: ui::sc(362).min(wa.bottom - wa.top - ui::sc(16)) };
        AdjustWindowRectEx(&mut r, style, 0, WS_EX_TOPMOST | WS_EX_DLGMODALFRAME);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let x = wa.left + ((wa.right - wa.left - ww) / 2).max(0);
        let y = wa.top + ((wa.bottom - wa.top - wh) / 2).max(0);
        let dlg = CreateWindowExW(WS_EX_TOPMOST | WS_EX_DLGMODALFRAME, cls.as_ptr(), w("Shut Down Windows").as_ptr(), style | WS_VISIBLE, x, y, ww, wh, 0 as _, 0 as _, inst, ptr::null());
        if !dlg.is_null() {
            use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
            SHUTDOWN_HWND.store(dlg as isize, Relaxed);
            let fg = GetForegroundWindow();
            let f_tid = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, ptr::null_mut()) };
            let m_tid = GetCurrentThreadId();
            if f_tid != 0 && f_tid != m_tid {
                AttachThreadInput(m_tid, f_tid, 1);
            }
            SetForegroundWindow(dlg);
            SetFocus(dlg);
            if f_tid != 0 && f_tid != m_tid {
                AttachThreadInput(m_tid, f_tid, 0);
            }
        }
    }
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if m == winrust_common::style::msg() {
            winrust_common::style::reload();
    if desk() { InvalidateRect(h, ptr::null(), 1); } else { ui::restyle(h); }
            return 0;
        }
        match m {
            WM_CREATE => {
                let inst = GetModuleHandleW(ptr::null());
                let (mut tab, mut edit) = (0 as HWND, 0 as HWND);
                if !desk() {
                    ui::dark_title(h);
                    let big = LoadImageW(inst, 1 as _, IMAGE_ICON, ui::sc(32), ui::sc(32), 0);
                    let sml = LoadImageW(inst, 1 as _, IMAGE_ICON, ui::sc(16), ui::sc(16), 0);
                    if !big.is_null() { put(h, WM_SETICON, 1, big as isize); }
                    if !sml.is_null() { put(h, WM_SETICON, 0, sml as isize); }
                    tab = CreateWindowExW(0, w("RxChrome").as_ptr(), ptr::null(), WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN, 0, 0, 0, 0, h, 0 as _, inst, ptr::null());
                    ui::nav_init(CreateWindowExW(0, w("RxNav").as_ptr(), ptr::null(), WS_CHILD | WS_VISIBLE, 0, 0, 0, 0, h, 0 as _, inst, ptr::null()));
                    edit = CreateWindowExW(0, w("EDIT").as_ptr(), ptr::null(), WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32, 0, 0, 0, 0, tab, 0 as _, inst, ptr::null());
                    put(edit, WM_SETFONT, ui::font(14, 400, ui::ui_face()) as usize, 1);
                    EDIT_OLD.store(SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize), std::sync::atomic::Ordering::Relaxed);
                    SetWindowPos(h, 0 as _, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
                let view = if desk() { LVS_ICON } else { LVS_REPORT };
                let clip = if desk() { 0 } else { WS_CLIPCHILDREN };
                let list = CreateWindowExW(0, w("SysListView32").as_ptr(), ptr::null(), WS_CHILD | WS_VISIBLE | clip | view | if desk() { 0 } else { LVS_OWNERDATA } | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS, 0, 0, 0, 0, h, 0 as _, inst, ptr::null());
                put(list, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize);
                let mut fi: SHFILEINFOW = std::mem::zeroed();
                let il = SHGetFileInfoW(w("C:\\").as_ptr(), 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | SHGFI_SMALLICON);
                put(list, LVM_SETIMAGELIST, LVSIL_SMALL as usize, il as isize);
    if desk() {
                    let big = SHGetFileInfoW(w("C:\\").as_ptr(), 0, &mut fi, size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | SHGFI_LARGEICON);
                    put(list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, big as isize);
                    apply_icon_size(list);
                    let ex = LVS_EX_TRANSPARENTBKGND | 0x0080_0000;
                    put(list, LVM_SETEXTENDEDLISTVIEWSTYLE, ex as usize, ex as isize);
                    for msg in [LVM_SETBKCOLOR, LVM_SETTEXTBKCOLOR] { put(list, msg, 0, CLR_NONE); }
                    put(list, LVM_SETTEXTCOLOR, 0, 0x00FF_FFFF);
                    reload_wall();
                    desk::hook_list(list);
                    put(list, WM_SETFONT, windows_sys::Win32::Graphics::Gdi::CreateFontW(-ui::sc(13), 0, 0, 0, 500, 0, 0, 0, 1, 0, 0, 4, 0, w(ui::ui_face()).as_ptr()) as usize, 1);
                    CreateWindowExW(0, w("SHELLDLL_DefView").as_ptr(), ptr::null(), WS_CHILD, 0, 0, 0, 0, h, 0 as _, inst, ptr::null());
                    SetTimer(h, 7, 2000, None);
                } else {
                    let himl = ImageList_Create(THUMB, THUMB, ILC_COLOR32, 64, 64);
                    put(list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, himl as isize);
                    put(list, LVM_SETICONSPACING, 0, ((THUMB as isize + 34) << 16) | (THUMB as isize + 24));
                    let _ = TX.set(sh::start_thumbs(h as isize));
                    ST.with_borrow_mut(|st| st.himl = himl as isize);
                    col(list, 0, "Name", ui::sc(10), false);
                    col(list, 1, "Date modified", ui::sc(10), false);
                    col(list, 2, "Type", ui::sc(10), false);
                    col(list, 3, "Size", ui::sc(10), true);
                    ui::style_list(list);
                }
                ST.with_borrow_mut(|st| { st.list = list; st.tab = tab; st.edit = edit; st.tabs = vec![PathBuf::new()]; });
                if !desk() { ui::place_edit(tab); }
                let _ = sh::register_drop(list as _, h as isize);
                if desk() && icons_hidden() { ShowWindow(list, SW_HIDE); }
    if desk() { let _ = sh::register_drop(h as _, h as isize); }
                let (start, pick) = if desk() { (sh::desktop_dirs().into_iter().next().unwrap_or_default(), None) } else { start_target() };
                load(h, start);
                if let Some(name) = pick {
                    select_row(&name);
                    PENDING_PICK.set(Some(name));
                }
                if desk() {
                    PostMessageW(h, WM_RESIDENT, 0, 0);
                } else {
                    shellwin::navigated(&cur_dir());
                    shellwin::register(h as isize);
                }
                0
            }
            WM_DPICHANGED if !desk() => { ui::on_dpi_changed(h, wp, lp); 0 }
            WM_RESIDENT if desk() => {
                use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos};
                let mut dirs: Vec<PathBuf> = [&FOLDERID_Desktop, &FOLDERID_Downloads, &FOLDERID_Documents, &FOLDERID_Pictures, &FOLDERID_Music, &FOLDERID_Videos].iter().filter_map(|id| sh::known_dir(id)).collect();
                dirs.extend(ui::pins());
                shellwin::register_resident(&dirs);
                0
            }
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE if desk() => {
                if m == WM_DISPLAYCHANGE {
                    reload_wall();
                    let (vx, vy, vw, vh) = virt();
                    SetWindowPos(h, HWND_BOTTOM, vx, vy, vw, vh, SWP_NOACTIVATE);
                    InvalidateRect(h, ptr::null(), 1);
                } else if wp == SPI_SETWORKAREA as usize {
                    place_desk_list(ST.with_borrow(|st| st.list));
                }
                0
            }
            0x0319 if !desk() && matches!((lp >> 16) & 0x7FF, 1 | 2) => { nav(h, (lp >> 16) & 0x7FF == 1); 1 }
            WM_NCCALCSIZE if wp != 0 && !desk() => {
                if IsZoomed(h) != 0 {
                    let p = &mut *(lp as *mut NCCALCSIZE_PARAMS);
                    let fx = GetSystemMetrics(SM_CXFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER);
                    let fy = GetSystemMetrics(SM_CYFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER);
                    p.rgrc[0].left += fx;
                    p.rgrc[0].top += fy;
                    p.rgrc[0].right -= fx;
                    p.rgrc[0].bottom -= fy;
                }
                0
            }
            WM_NCHITTEST if !desk() => {
                let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
                let mut wr: RECT = std::mem::zeroed();
                GetWindowRect(h, &mut wr);
                let b = ui::sc(6);
                let (l, r, t, bt) = (x < wr.left + b, x >= wr.right - b, y < wr.top + b, y >= wr.bottom - b);
                match (l, r, t, bt) {
                    (true, _, true, _) => HTTOPLEFT as isize,
                    (_, true, true, _) => HTTOPRIGHT as isize,
                    (true, _, _, true) => HTBOTTOMLEFT as isize,
                    (_, true, _, true) => HTBOTTOMRIGHT as isize,
                    (true, _, _, _) => HTLEFT as isize,
                    (_, true, _, _) => HTRIGHT as isize,
                    (_, _, true, _) => HTTOP as isize,
                    (_, _, _, true) => HTBOTTOM as isize,
                    _ => DefWindowProcW(h, m, wp, lp),
                }
            }
            WM_SIZE => {
                let list = ST.with_borrow(|st| st.list);
                let (cw, ch) = ((lp & 0xFFFF) as i32, ((lp >> 16) & 0xFFFF) as i32);
    if desk() {
                    place_desk_list(list);
                    desk::size_workerw(cw, ch);
                } else {
                    ui::layout_children(cw, ch);
                    if ch > 0 {
                        if let Some(name) = PENDING_PICK.take() {
                            select_row(&name);
                        }
                    }
                    InvalidateRect(h, ptr::null(), 0);
                }
                0
            }
            WM_PAINT if !desk() => { ui::paint_status(h); 0 }
            ui::WM_UI => {
                match wp {
                    1 => { let n = ST.with_borrow(|st| st.tabs.len()); if (lp as usize) < n { tab_select(h, lp as usize); } }
                    2 => tab_close_at(h, lp as usize),
                    3 => tab_new(),
                    4 => nav(h, true),
                    5 => nav(h, false),
                    6 => { if let Some(p) = cur_dir().parent() { load(h, p.to_path_buf()) } }
                    7 => reload(h),
                    8 => view_menu(h),
                    9 => search_focus(),
                    16 => new_menu(h),
                    17 => sort_menu(h),
                    18 => more_menu(h),
                    10..=14 => {
                        let (_list, dir, paths) = selected();
                        match wp {
                            10 | 11 if !paths.is_empty() => match real_paths(&paths) { Ok(p) => { if sh::clip_set(h, &p, wp == 10 && !in_zip()).is_ok() { sync_cut_visuals(); } } Err(e) => warn(h, &e) },
                            12 => paste(h, &dir),
                            13 if !in_zip() => begin_rename(h),
                            14 if !paths.is_empty() && !in_zip() => {
                                let perm = GetKeyState(0x10 ) < 0;
                                confirm_delete(h, &dir, &paths, perm);
                                reload(h);
                            }
                            _ => {}
                        }
                    }
                    20 => { if let Some(p) = ui::nav_path(lp as usize) { load(h, p); } }
                    21 => { if let Some(p) = ui::crumb_path(lp as usize) { load(h, p); } }
                    22 => {
                        if let Some(p) = ST.with_borrow(|st| st.rows.get(lp as usize).filter(|r| !r.hdr).map(|r| PathBuf::from(&r.name))) {
                            load(h, p);
                        }
                    }
                    23 => {
                        let (_, dir, paths) = selected();
                        if !paths.is_empty() { item_menu(h, &dir, &paths); } else { bg_menu(h); }
                        MENU_DONE.with(|t| t.set(Some(std::time::Instant::now())));
                    }
                    _ => {}
                }
                0
            }
            WM_ERASEBKGND if desk() && desk::erase(wp as _, h) => 1,
            WM_WINDOWPOSCHANGING if desk() => {
                let wp = &mut *(lp as *mut WINDOWPOS);
                wp.hwndInsertAfter = HWND_BOTTOM;
                wp.flags |= SWP_NOACTIVATE;
                0
            }
            WM_INITMENUPOPUP if sh::menu_msg(m, wp, lp) => 0,
            WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR if sh::menu_msg(m, wp, lp) => 1,
            WM_NOTIFY => {
                let nm = &*(lp as *const NMHDR);
                let list = ST.with_borrow(|st| st.list);
                match nm.code {
                    NM_CUSTOMDRAW if nm.hwndFrom == list => return if desk() { ui::desk_draw(lp) } else { ui::list_draw(lp) },
                    LVN_ITEMCHANGED | LVN_ODSTATECHANGED => ui::status_dirty(h),
                    LVN_ODFINDITEMW if nm.hwndFrom == list => {
                        let f = &*(lp as *const NMLVFINDITEMW);
                        if f.lvfi.flags & (LVFI_STRING | LVFI_PARTIAL) == 0 || f.lvfi.psz.is_null() { return -1; }
                        let mut n = 0;
                        while *f.lvfi.psz.add(n) != 0 { n += 1; }
                        let q = String::from_utf16_lossy(std::slice::from_raw_parts(f.lvfi.psz, n)).to_lowercase();
                        return ST.with_borrow(|st| {
                            let len = st.rows.len();
                            if len == 0 { return -1; }
                            let start = usize::try_from(f.iStart).unwrap_or(0) % len;
                            (0..len).map(|k| (start + k) % len).find(|&i| {
                                !st.rows[i].hdr && {
                                    let mut rc = st.rows[i].name.chars().flat_map(char::to_lowercase);
                                    q.chars().all(|qc| rc.next() == Some(qc))
                                }
                            }).map_or(-1, |i| i as isize)
                        });
                    }
                    LVN_GETDISPINFOW => {
                        let it = &mut (*(lp as *mut NMLVDISPINFOW)).item;
                        ST.with(|c| {
                            let Ok(mut st) = c.try_borrow_mut() else { return };
                            let State { dir, rows, thumbs, .. } = &mut *st;
                            let Some(i) = usize::try_from(it.iItem).ok() else { return };
                            let Some(r) = rows.get_mut(i) else { return };
                            if it.mask & LVIF_TEXT != 0 && it.cchTextMax > 0 {
                                let s = if r.hdr { if it.iSubItem == 0 { r.name[1..].to_string() } else { String::new() } } else { match it.iSubItem { 0 => shown_name(&r.name, desk()), 1 => date_text(r.time), 2 => type_text(&r.name, r.dir), _ if r.dir => String::new(), _ => size_text(r.size) } };
                                let max_c = it.cchTextMax as usize - 1;
                                let mut n = 0usize;
                                for u in s.encode_utf16().take(max_c) {
                                    *it.pszText.add(n) = u;
                                    n += 1;
                                }
                                *it.pszText.add(n) = 0;
                            }
                            if r.hdr { if it.mask & LVIF_IMAGE != 0 { it.iImage = I_IMAGENONE; } return; }
                            if it.mask & LVIF_IMAGE != 0 && *thumbs {
                                if r.thumb == -2 {
                                    r.thumb = -1;
                                    if let Some(tx) = TX.get() { let _ = tx.send((GEN.load(std::sync::atomic::Ordering::Relaxed), i, dir.join(&r.name))); }
                                }
                                it.iImage = if r.thumb >= 0 { r.thumb } else { I_IMAGENONE };
                            } else if it.mask & LVIF_IMAGE != 0 {
                                if r.icon < 0 { r.icon = icon_of(&dir.join(&r.name), r.dir); }
                                it.iImage = r.icon;
                            }
                        });
                    }
                    LVN_ITEMACTIVATE => {
                        let i = (*(lp as *const NMITEMACTIVATE)).iItem;
                        let hit = ST.with_borrow(|st| usize::try_from(i).ok().and_then(|i| st.rows.get(i)).filter(|r| !r.hdr).map(|r| (r.name.clone(), st.dir.join(&r.name), r.dir)));
                        if let Some((name, p, is_dir)) = hit {
                            if name == THIS_PC_KEY {
                                if let Ok(me) = env::current_exe() { spawn_gui(&mut std::process::Command::new(me)); }
                            } else if name == RECYCLE_BIN_KEY {
                                ShellExecuteW(h, w("explore").as_ptr(), w("shell:RecycleBinFolder").as_ptr(), ptr::null(), ptr::null(), SW_SHOWNORMAL);
                            } else {
                                activate(h, p, is_dir);
                            }
                        }
                    }
                    NM_RCLICK if nm.hwndFrom == list => {
                        let (_, dir, paths) = selected();
                        if !paths.is_empty() { item_menu(h, &dir, &paths); } else { bg_menu(h); }
                        MENU_DONE.with(|t| t.set(Some(std::time::Instant::now())));
                    }
                    LVN_COLUMNCLICK => {
                        let c = (*(lp as *const NMLISTVIEW)).iSubItem as usize;
                        let asc = if SORT_COL.load(Relaxed) == c { !SORT_ASC.load(Relaxed) } else { true };
                        SORT_COL.store(c, Relaxed);
                        SORT_ASC.store(asc, Relaxed);
                        let n = ST.with_borrow_mut(|st| {
                            st.rows.retain(|r| !r.hdr);
                            sort_rows(&mut st.rows, c, asc);
                            add_headers(&mut st.rows);
                            st.rows.iter_mut().for_each(|r| r.thumb = -2);
                            st.rows.len()
                        });
                        reset_thumbs();
                        put(list, LVM_REDRAWITEMS, 0, n as isize - 1);
                    }
                    LVN_BEGINDRAG => {
                        let (_, _, paths) = selected();
                        let paths = match real_paths(&paths) { Ok(p) => p, Err(e) => { warn(h, &e); return 0; } };
                        drag_log(&format!("BEGINDRAG desk={} selected={}", desk(), paths.len()));
                        if !paths.is_empty() {
    if desk() { let p = (*(lp as *const NMLISTVIEW)).ptAction; desk::drag_start((p.x, p.y)); }
                            let r = sh::drag(h, &paths);
                            drag_log(&format!("drag finished: {r:?}"));
                            let handled = desk() && desk::drag_end(h, list);
                            drag_log(&format!("moved on the desktop: {handled}"));
                            if !handled { reload(h); }
                        }
                    }
                    LVN_KEYDOWN => {
                        let vk = (*(lp as *const NMLVKEYDOWN)).wVKey as u32;
                        let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
                        let shift = GetKeyState(0x10 ) < 0;
                        if desk() && matches!(vk, 0x08 | 0x46 | 0x4D | 0x54 | 0x57 | 0x09 | 0x4C | 0x72 | 0x73 | 0x52) { return 0; }
                        let (_list, dir, paths) = selected();
                        match vk {
                            0x08 => if let Some(p) = dir.parent() { load(h, p.to_path_buf()) },
                            0x1B if jobs::busy() => { jobs::cancel(); return 0; }
                            0x2E if !paths.is_empty() && !in_zip() => confirm_delete(h, &dir, &paths, shift),
                            0x20 if !desk() => if let Some(p) = paths.first().filter(|p| p.is_file()) { activate(h, p.clone(), false); },
                            0x71 if !in_zip() => { begin_rename(h); return 0; }
                            0x46 if ctrl => { search_focus(); return 0; }
                            0x72 => { search_focus(); return 0; }
                            0x52 if ctrl => {}
                            0x73 | 0x4C if vk == 0x73 || ctrl => {
                                let edit = ST.with_borrow(|st| st.edit);
                                SetFocus(edit);
                                put(edit, EM_SETSEL, 0, -1);
                                return 0;
                            }
                            0x5D if !paths.is_empty() => item_menu(h, &dir, &paths),
                            0x43 | 0x58 if ctrl && !paths.is_empty() => {
                                match real_paths(&paths) { Ok(p) => { if sh::clip_set(h, &p, vk == 0x58 && !in_zip()).is_ok() { sync_cut_visuals(); } } Err(e) => warn(h, &e) }
                                return 0;
                            }
                            0x41 if ctrl => {
                                let mut it: LVITEMW = std::mem::zeroed();
                                it.stateMask = LVIS_SELECTED;
                                it.state = LVIS_SELECTED;
                                put(list, LVM_SETITEMSTATE, usize::MAX, &it as *const _ as isize);
                                return 0;
                            }
                            0x56 if ctrl => { paste(h, &dir); return 0; }
                            0x5A if ctrl => {
                                if let Some(op) = UNDO.with_borrow_mut(|u| u.pop()) {
                                    match op {
                                        UndoOp::Rename { from, to } => { let _ = fs::rename(to, from); }
                                        UndoOp::New { path, dir: true } => { let _ = fs::remove_dir(path); }
                                        UndoOp::New { path, dir: false } => { let _ = fs::remove_file(path); }
                                    }
                                }
                            }
                            0x4E if ctrl => {
                                let created = new_entry(&dir, "New folder", true);
                                reload(h);
                                if let Some(name) = created { select_row(&name); }
                                return 0;
                            }
                            0x48 if ctrl => { SHOW_HIDDEN.fetch_xor(true, Relaxed); }
                            0x74 => {}
                            0x4D if ctrl => { toggle_view(h); return 0; }
                            0x54 if ctrl => tab_new(),
                            0x57 if ctrl => { tab_close(h); return 0; }
                            0x09 if ctrl => {
                                let next = ST.with_borrow(|st| (st.active + 1) % st.tabs.len());
                                tab_select(h, next);
                                return 0;
                            }
                            _ => return 0,
                        }
                        reload(h);
                    }
                    _ => {}
                }
                0
            }
            WM_SETFOCUS => { SetFocus(ST.with_borrow(|st| st.list)); 0 }
            WM_APP => { WATCH_PENDING.store(false, Relaxed); reload_select_new(h); 0 }
            shellwin::WM_SHELL_SELECT => { shell_select(h as isize, *Box::from_raw(lp as *mut PathBuf)); 0 }
            WM_JOB => {
                let err = *Box::from_raw(wp as *mut Option<String>);
                RemovePropW(h, w("WinRustProgress").as_ptr());
                reload_select_new(h);
                ui::status_dirty(h);
                if let Some(e) = err { MessageBoxW(h, w(&e).as_ptr(), w("WinRust Explorer").as_ptr(), MB_ICONWARNING); }
                0
            }
            WM_NAV => {
                match wp {
                    1 => nav(h, true),
                    2 => nav(h, false),
                    3 => {
                        let (_, dir, sel) = selected();
                        if sel.is_empty() {
                            if dir != Path::new(THIS_PC_KEY) { properties(&dir); }
                        } else {
                            properties_many(h, &sel);
                        }
                    }
                    _ => if let Some(p) = cur_dir().parent() { load(h, p.to_path_buf()) },
                }
                0
            }
            WM_THUMB => {
                let (era, row, bmp) = *Box::from_raw(wp as *mut (u32, usize, isize));
                let (list, himl) = ST.with_borrow(|st| (st.list, st.himl));
                if era == GEN.load(std::sync::atomic::Ordering::Relaxed) {
                    let idx = ImageList_Add(himl as _, bmp as _, ptr::null_mut());
                    let ok = idx >= 0 && ST.with_borrow_mut(|st| st.rows.get_mut(row).map(|r| r.thumb = idx).is_some());
                    if ok { put(list, LVM_REDRAWITEMS, row, row as isize); }
                }
                windows_sys::Win32::Graphics::Gdi::DeleteObject(bmp as _);
                0
            }
            WM_GO => {
                let (edit, list) = ST.with_borrow(|st| (st.edit, st.list));
                if wp == 13 {
                    let mut buf = vec![0u16; GetWindowTextLengthW(edit) as usize + 1];
                    let n = GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32) as usize;
                    let p = PathBuf::from(String::from_utf16_lossy(&buf[..n]).trim().trim_matches('"'));
                    let text = String::from_utf16_lossy(&buf[..n]);
                    if let Some(q) = text.trim().strip_prefix('?') {
                        if !q.trim().is_empty() { search(h, q.trim()); SetFocus(list); return 0; }
                    } else if p.is_dir() || zipx::split_zip(&p).is_some() { load(h, p); }
                }
                SetWindowTextW(edit, w(&cur_dir().to_string_lossy()).as_ptr());
                SetFocus(list);
                0
            }
            WM_CONTEXTMENU => {
                let fresh = MENU_DONE.with(|t| t.take()).is_some_and(|t| t.elapsed().as_millis() < 1500);
                if !fresh {
                    if desk() {
                        bg_menu(h);
                    } else if wp as HWND == ST.with_borrow(|st| st.list) {
                        let (_, dir, paths) = selected();
                        if !paths.is_empty() { item_menu(h, &dir, &paths); } else { bg_menu(h); }
                    }
                }
                0
            }
            desk::SPAWN_WORKERW if desk() => { desk::spawn_workerw(h); 0 }
            WM_TIMER if desk() && wp == 7 => { desk::check_live(ST.with_borrow(|st| st.list)); 0 }
            WM_CLOSE if desk() => { PostMessageW(h, WM_SHUTDOWN_DLG, 0, 0); 0 }
            WM_SHUTDOWN_DLG if desk() => { show_shutdown_dialog(h); 0 }
            WM_DESTROY => { if !desk() { save_place(h); } shellwin::revoke(); PostQuitMessage(0); 0 }
            _ => DefWindowProcW(h, m, wp, lp),
        }
    }
}

fn namespace_target() -> Option<String> {
    env::args().skip(1).find_map(|a| {
        let t = a.trim_matches('"').rsplit(',').next().unwrap_or("").to_string();
        let l = t.to_lowercase();
        (l.starts_with("::{") || l.starts_with("shell:")).then_some(t)
    })
}

fn main() {
    winrust_common::install_panic_hook("rust-explorer");
    if let Some(t) = namespace_target() {
        if env::var_os("RX_NS_HOP").is_none() {
            let addr = if t.starts_with("::") { format!("shell:{t}") } else { t };
            unsafe { env::set_var("RX_NS_HOP", "1") };
            let (f, v) = (w(&addr), w("explore"));
            let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
            sei.fMask = 0x0000_0100 | 0x0000_0400;
            sei.lpVerb = v.as_ptr();
            sei.lpFile = f.as_ptr();
            sei.nShow = SW_SHOWNORMAL;
            unsafe { ShellExecuteExW(&mut sei) };
        }
        return;
    }
    sh::init();
    unsafe {
        SetProcessDPIAware();
        ui::init_scale();
        winrust_common::ui::ensure_fluent_icons();
        ui::gdiplus_start();
        InitCommonControlsEx(&INITCOMMONCONTROLSEX { dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_LISTVIEW_CLASSES });
        let inst = GetModuleHandleW(ptr::null());
        let cls = w("RustExplorer");
        let desktop = env::args().nth(1).as_deref() == Some("--desktop");
        let bg = if desktop { windows_sys::Win32::Graphics::Gdi::GetSysColorBrush(1) as _ } else { windows_sys::Win32::Graphics::Gdi::CreateSolidBrush(ui::pal().bg) };
        RegisterClassW(&WNDCLASSW { style: 0x0008, lpfnWndProc: Some(ui::proc), hInstance: inst, lpszClassName: w("RxChrome").as_ptr(), hCursor: LoadCursorW(0 as _, IDC_ARROW), ..std::mem::zeroed() });
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(ui::nav_proc), hInstance: inst, lpszClassName: w("RxNav").as_ptr(), hCursor: LoadCursorW(0 as _, IDC_ARROW), ..std::mem::zeroed() });
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst, lpszClassName: cls.as_ptr(), hCursor: LoadCursorW(0 as _, IDC_ARROW), hIcon: LoadIconW(inst, 1 as _), hbrBackground: bg, ..std::mem::zeroed() });
        if desktop {
            DESKTOP.store(true, Relaxed);
            let pm = w("Progman");
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst, lpszClassName: pm.as_ptr(), hCursor: LoadCursorW(0 as _, IDC_ARROW), hIcon: LoadIconW(inst, 1 as _), hbrBackground: bg, ..std::mem::zeroed() });
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(desk::workerw_proc), hInstance: inst, lpszClassName: w("WorkerW").as_ptr(), ..std::mem::zeroed() });
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(DefWindowProcW), hInstance: inst, lpszClassName: w("SHELLDLL_DefView").as_ptr(), ..std::mem::zeroed() });
            let (vx, vy, vw, vh) = virt();
            let d = CreateWindowExW(WS_EX_TOOLWINDOW, pm.as_ptr(), w("rs-desktop").as_ptr(), WS_POPUP, vx, vy, vw, vh, 0 as _, 0 as _, inst, ptr::null());
            SetWindowPos(d, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        } else {
            let wa = winrust_common::ui::work_area();
            let (sw, sh) = ((wa.right - wa.left).max(600), (wa.bottom - wa.top).max(420));
            let dw = (sw * 76 / 100).clamp(600, ui::sc(1120).min(sw));
            let dh = (sh * 78 / 100).clamp(420, ui::sc(740).min(sh));
            let win = CreateWindowExW(0, cls.as_ptr(), ptr::null(), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, CW_USEDEFAULT, CW_USEDEFAULT, dw, dh, 0 as _, 0 as _, inst, ptr::null());
            match saved_place() {
                Some((max, r)) => {
                    let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
                    wp.length = size_of::<WINDOWPLACEMENT>() as u32;
                    wp.showCmd = if max { SW_SHOWMAXIMIZED } else { SW_SHOWNORMAL } as u32;
                    wp.rcNormalPosition = r;
                    SetWindowPlacement(win, &wp);
                }
                None => { ShowWindow(win, SW_SHOWNORMAL); }
            }
            if let Some(name) = PENDING_PICK.take() {
                select_row(&name);
            }
        }
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, 0 as _, 0, 0) > 0 {
            if msg.message == WM_MOUSEWHEEL && !desk() {
                let nav = ui::nav_hwnd();
                if !nav.is_null() && msg.hwnd != nav {
                    let pt = POINT { x: (msg.lParam & 0xFFFF) as i16 as i32, y: ((msg.lParam >> 16) & 0xFFFF) as i16 as i32 };
                    let mut nr: RECT = std::mem::zeroed();
                    if GetWindowRect(nav, &mut nr) != 0 && pt.x >= nr.left && pt.x < nr.right && pt.y >= nr.top && pt.y < nr.bottom {
                        SendMessageW(nav, WM_MOUSEWHEEL, msg.wParam, msg.lParam);
                        continue;
                    }
                }
            }
            if msg.message == WM_MOUSEWHEEL && desk() && GetKeyState(VK_CONTROL as i32) < 0 {
                let delta = ((msg.wParam >> 16) as u16) as i16;
                if delta != 0 {
                    let cur = desk_icon_tier();
                    let next = if delta > 0 { (cur + 1).min(2) } else { cur.saturating_sub(1) };
                    if next != cur {
                        set_desk_icon_tier(next);
                        let root = GetAncestor(msg.hwnd, GA_ROOT);
                        let list = ST.with_borrow(|st| st.list);
                        apply_icon_size(list);
                        let _ = fs::remove_file(desk::pos_file());
                        reload(root);
                    }
                }
                continue;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == 0x57 && !desk() && GetKeyState(VK_CONTROL as i32) < 0 {
                tab_close(GetAncestor(msg.hwnd, GA_ROOT));
                continue;
            }
            if msg.message == WM_SYSKEYDOWN && !desk() {
                let act = match msg.wParam { 0x25 => 1, 0x27 => 2, 0x0D => 3, 0x26 => 4, _ => 0 };
                if act != 0 { PostMessageW(GetAncestor(msg.hwnd, GA_ROOT), WM_NAV, act, 0); continue; }
            }
            if msg.message == WM_SYSKEYDOWN && msg.wParam == 0x73 && desk() && GetKeyState(0x12) < 0 {
                let root = FindWindowW(w("Progman").as_ptr(), w("rs-desktop").as_ptr());
                if !root.is_null() { PostMessageW(root, WM_SHUTDOWN_DLG, 0, 0); }
                continue;
            }
            let sdlg = SHUTDOWN_HWND.load(Relaxed) as HWND;
            if !sdlg.is_null() && IsDialogMessageW(sdlg, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_place_parses_and_rejects_junk() {
        let (max, r) = super::parse_place("max 10 20 1010 680").unwrap();
        assert!(max);
        assert_eq!((r.left, r.top, r.right, r.bottom), (10, 20, 1010, 680));
        assert!(!super::parse_place("normal -1900 5 -900 700").unwrap().0);
        assert!(super::parse_place("max 0 0 50 50").is_none());
        assert!(super::parse_place("max 0 0 800").is_none());
        assert!(super::parse_place("big 0 0 800 600").is_none());
        assert!(super::parse_place("").is_none());
    }

    #[test]
    fn a_zip_opens_like_a_folder_and_gives_real_files() {
        let d = std::env::temp_dir().join(format!("rx-zipview-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("f.txt"), "hello").unwrap();
        let z = d.join("a.zip");
        let mut w = zipx::Writer::new(fs::File::create(&z).unwrap());
        w.add_prepared(zipx::prepare("inner/f.txt".into(), &d.join("f.txt")).unwrap()).unwrap();
        w.add_prepared(zipx::prepare("top.txt".into(), &d.join("f.txt")).unwrap()).unwrap();
        w.finish().unwrap();
        let names = |dir: &PathBuf| rows_of(dir).unwrap().into_iter().map(|r| format!("{}{}", r.name, if r.dir { "/" } else { "" })).collect::<Vec<_>>();
        assert_eq!(names(&z), ["inner/", "top.txt"]);
        assert_eq!(names(&z.join("inner")), ["f.txt"]);
        assert!(rows_of(&d.join("nothing.zip")).is_none());
        let paths = [z.join("inner").join("f.txt"), z.join("top.txt")];
        let real = real_paths(&paths).unwrap();
        assert!(real.iter().all(|p| p.is_file() && fs::read_to_string(p).unwrap() == "hello"), "{real:?}");
        assert_eq!(real_paths(&[d.join("f.txt")]).unwrap(), [d.join("f.txt")], "ordinary files stay as they are");
        assert!(zip_names(&[z.join("top.txt"), d.join("f.txt")]).is_none(), "mixed selections are not archive selections");
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(real[0].parent().unwrap());
    }

    #[test]
    #[ignore]
    fn system_zip_is_not_in_the_menu() {
        sh::init();
        let f = std::env::temp_dir().join("rx-menu-dump.txt");
        std::fs::write(&f, "x").unwrap();
        let menu = sh::menu_dump(&[f.clone()]);
        let _ = std::fs::remove_file(f);
        println!("{}", menu.join("
"));
        assert!(menu.iter().any(|l| l.contains("Se&nd to") || l.contains("Send to") || l.contains("&Delete") || l.contains("Delete")), "the menu is there");
        if let Some(n) = sh::system_zip_name() { assert!(!menu.iter().any(|l| l.trim().starts_with(&n)), "{n} should be gone"); }
    }

    use super::*;

    fn r(name: &str, dir: bool, size: u64, time: u64) -> Row { Row { name: name.into(), dir, size, time, icon: -1, thumb: -2 , hdr: false } }
    fn names(v: &[Row]) -> Vec<&str> { v.iter().map(|r| r.name.as_str()).collect() }

    #[test]
    fn desktop_rows_show_file_names_without_shortcut_extensions() {
        assert_eq!(shown_name(r"C:\Users\Public\Desktop\Steam.lnk", true), "Steam");
        assert_eq!(shown_name("Notes.txt", true), "Notes.txt");
        assert_eq!(shown_name(r"C:\x\Game.URL", true), "Game");
        assert_eq!(shown_name(r"C:\x\Steam.lnk", false), r"C:\x\Steam.lnk");
    }

    #[test]
    fn folders_first_then_by_the_chosen_column() {
        let mut v = vec![r("b.txt", false, 5, 1), r("A", true, 0, 2), r("a.txt", false, 9, 3)];
        sort_rows(&mut v, 0, true);
        assert_eq!(names(&v), ["A", "a.txt", "b.txt"]);
        sort_rows(&mut v, 3, false);
        assert_eq!(names(&v), ["A", "a.txt", "b.txt"]);
        sort_rows(&mut v, 1, true);
        assert_eq!(names(&v), ["A", "b.txt", "a.txt"]);
    }

    #[test]
    fn search_hit_paths_sort_by_file_name() {
        let mut v = vec![r("C:\\z\\b.txt", false, 1, 1), r("D:\\a\\a.txt", false, 1, 1)];
        sort_rows(&mut v, 0, true);
        assert_eq!(names(&v), ["D:\\a\\a.txt", "C:\\z\\b.txt"]);
    }

    #[test]
    fn history_goes_back_and_forward_and_a_new_visit_drops_the_forward_trail() {
        let mut h = Hist::default();
        for p in ["a", "b", "c"] { h.visit(p.into()); }
        assert_eq!(h.back(), Some("b".into()));
        assert_eq!(h.back(), Some("a".into()));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some("b".into()));
        h.visit("x".into());
        assert_eq!(h.forward(), None);
        assert_eq!(h.back(), Some("b".into()));
        h.visit("x".into()); h.visit("x".into());
        assert_eq!(h.v.len(), 3);
    }

    #[test]
    fn type_and_size_read_like_explorers() {
        assert_eq!(type_text("x", true), "Папка с файлами");
        assert_eq!(type_text("a.txt", false), "Файл TXT");
        assert_eq!(type_text("C:\\d\\b.tar.gz", false), "Файл GZ");
        assert_eq!(type_text(".gitignore", false), "Файл");
        assert_eq!(type_text("Makefile", false), "Файл");
        assert_eq!(size_text(1), "1 KB");
        assert_eq!(size_text(2048), "2 KB");
        assert_eq!(size_text(5 << 20), "5.0 MB");
        assert_eq!(size_text(3 << 30), "3.00 GB");
    }

    #[test]
    fn dates_are_formatted_and_zero_is_blank() {
        assert_eq!(date_text(0), "");
        let t = date_text(132_000_000_000_000_000);
        assert!(t.len() == 16 && t.starts_with("20"), "{t}");
    }
}

#[cfg(test)]
mod search_tests {
    #[test]
    fn local_search_finds_names_below_the_folder() {
        let d = std::env::temp_dir().join(format!("rx-search-test-{}", std::process::id()));
        std::fs::create_dir_all(d.join(r"sub\deep")).unwrap();
        for f in [r"Report.txt", r"sub\notes.md", r"sub\deep\REPORT-2.pdf"] { std::fs::write(d.join(f), "x").unwrap(); }
        let (rows, cut, flipped) = super::local_search(&d, "report");
        let (wrong, _, was_flipped) = super::local_search(&d, "кузщке");
        let mut names: Vec<String> = rows.iter().map(|r| std::path::Path::new(&r.name).file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(names, ["REPORT-2.pdf", "Report.txt"]);
        assert!(!cut && !flipped);
        assert!(was_flipped && wrong.len() == 2);
    }

    #[test]
    fn layout_flip_both_ways() {
        assert_eq!(super::flip_layout("сфкпщ"), "cargo");
        assert_eq!(super::flip_layout("ljrevtyns"), "документы");
        assert_eq!(super::flip_layout("Ыфму"), "Save");
    }
}
