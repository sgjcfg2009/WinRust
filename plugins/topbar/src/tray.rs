use super::*;

pub(crate) unsafe fn find_other(class: &str) -> HWND {
    let mut h = null_mut();
    with_wide(class, |c| loop {
        h = FindWindowExW(null_mut(), h, c, null());
        if h.is_null() || h != HOST.get() {
            return h;
        }
    })
}

fn eq_ascii_ci(buf: &[u16], s: &str) -> bool {
    buf.len() == s.len() && buf.iter().zip(s.bytes()).all(|(&a, b)| a < 128 && (a as u8).eq_ignore_ascii_case(&b))
}

fn starts_with_ascii(buf: &[u16], s: &str) -> bool {
    buf.len() >= s.len() && buf[..s.len()].iter().zip(s.bytes()).all(|(&a, b)| a == b as u16)
}

pub(crate) unsafe fn foreign_taskbar_windows() -> Vec<HWND> {
    let host = HOST.get();
    let mut trays = Vec::new();
    let mut tray_pids = Vec::new();
    for cls in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
        let c = wide(cls);
        let mut h = null_mut();
        loop {
            h = FindWindowExW(null_mut(), h, c.as_ptr(), null());
            if h.is_null() {
                break;
            }
            if h != host {
                trays.push(h);
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, &mut pid);
                if pid != 0 && !tray_pids.contains(&pid) {
                    tray_pids.push(pid);
                }
            }
        }
    }
    struct Ctx<'a> {
        host: HWND,
        own_pid: u32,
        trays: &'a [HWND],
        tray_pids: &'a [u32],
        out: Vec<HWND>,
    }
    unsafe extern "system" fn collect_tb(w: HWND, lp: LPARAM) -> i32 {
        let ctx = &mut *(lp as *mut Ctx);
        if w == ctx.host {
            return 1;
        }
        if ctx.trays.contains(&w) {
            ctx.out.push(w);
            return 1;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, &mut pid);
        if pid == 0 || pid == ctx.own_pid {
            return 1;
        }
        let owner = GetWindow(w, GW_OWNER);
        if !owner.is_null() && ctx.trays.contains(&owner) {
            ctx.out.push(w);
            return 1;
        }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(w, cls.as_mut_ptr(), cls.len() as i32).max(0) as usize;
        let cls = &cls[..n];
        let is_start_cls = eq_ascii_ci(cls, "Start")
            || starts_with_ascii(cls, "StartAllBack")
            || starts_with_ascii(cls, "StartIsBack")
            || (eq_ascii_ci(cls, "Button") && ctx.tray_pids.contains(&pid));
        if is_start_cls {
            ctx.out.push(w);
        }
        1
    }
    let mut ctx = Ctx { host, own_pid: std::process::id(), trays: &trays, tray_pids: &tray_pids, out: Vec::new() };
    EnumWindows(Some(collect_tb), &mut ctx as *mut _ as LPARAM);
    ctx.out
}

pub(crate) unsafe fn announce() {
    let msg = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
    if msg == 0 {
        return;
    }
    SendNotifyMessageW(HWND_BROADCAST, msg, 0, 0);
    const HWND_MESSAGE: HWND = -3isize as HWND;
    const SKIP_CLASSES: &[&str] = &[
        "OleMainThreadWndClass",
        "CicMarshalWndClass",
        "MSCTFIME UI",
        "IME",
        "Default IME",
        "GDI+ Hook Window Class",
        "DDE Server Window",
        "MediaContextNotificationWindow",
        "SystemResourceNotifyWindow",
        "MMDevApiDeviceEnumerator",
        "WorkerW",
        "tooltips_class32",
    ];
    let own_pid = std::process::id();
    let mut w = null_mut();
    let mut cls = [0u16; 128];
    let mut guard = 0usize;
    while guard < 1024 {
        w = FindWindowExW(HWND_MESSAGE, w, null(), null());
        if w.is_null() {
            break;
        }
        guard += 1;
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, &mut pid);
        if pid == 0 || pid == own_pid {
            continue;
        }
        let n = GetClassNameW(w, cls.as_mut_ptr(), cls.len() as i32).max(0) as usize;
        if SKIP_CLASSES.iter().any(|&s| eq_ascii_ci(&cls[..n], s)) {
            continue;
        }
        PostMessageW(w, msg, 0, 0);
    }
    unsafe extern "system" fn enum_top(w: HWND, lp: LPARAM) -> i32 {
        let msg = lp as u32;
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, &mut pid);
        if pid != 0 && pid != std::process::id() {
            PostMessageW(w, msg, 0, 0);
        }
        1
    }
    EnumWindows(Some(enum_top), msg as isize);
}

pub(crate) fn hide_onedrive() -> bool {
    static HIDE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HIDE.get_or_init(|| {
        std::env::current_exe().ok().and_then(|e| std::fs::read_to_string(e.with_file_name("topbar.cfg")).ok()).is_none_or(|c| !c.contains("hide_onedrive=0"))
    })
}

fn skip_tray_exe(exe: &str) -> bool {
    let n = exe.rsplit('\\').next().unwrap_or("");
    n == "explorer.exe" || n == "topbar.exe" || (n == "onedrive.exe" && hide_onedrive())
}

unsafe fn copy_or_extract_icon(raw_icon: HICON, hwnd: HWND, exe: &str) -> HICON {
    let mut ic = if !raw_icon.is_null() { CopyIcon(raw_icon) } else { null_mut() };
    if ic.is_null() && !exe.is_empty() {
        ExtractIconExW(wide(exe).as_ptr(), 0, null_mut(), &mut ic, 1);
    }
    if ic.is_null() && !hwnd.is_null() {
        let mut res = 0usize;
        for kind in [2usize, 0usize, 1usize] {
            if SendMessageTimeoutW(hwnd, WM_GETICON, kind, 0, SMTO_ABORTIFHUNG, 100, &mut res) != 0 && res != 0 {
                ic = CopyIcon(res as HICON);
                if !ic.is_null() { break; }
            }
        }
        if ic.is_null() {
            for idx in [-34i32, -14i32] {
                let h = GetClassLongPtrW(hwnd, idx);
                if h != 0 {
                    ic = CopyIcon(h as HICON);
                    if !ic.is_null() { break; }
                }
            }
        }
    }
    ic
}

pub(crate) unsafe fn import_from_explorer() {
    use windows_sys::Win32::System::{
        Diagnostics::Debug::ReadProcessMemory,
        Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx},
        Threading::{PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE},
    };
    const TB_GETBUTTON: u32 = 0x0417;
    const TB_BUTTONCOUNT: u32 = 0x0418;

    let mut toolbars = Vec::new();
    let exp_tray = find_other("Shell_TrayWnd");
    if !exp_tray.is_null() {
        let notify = FindWindowExW(exp_tray, null_mut(), wide("TrayNotifyWnd").as_ptr(), null());
        if !notify.is_null() {
            let pager = FindWindowExW(notify, null_mut(), wide("SysPager").as_ptr(), null());
            if !pager.is_null() {
                let tb = FindWindowExW(pager, null_mut(), wide("ToolbarWindow32").as_ptr(), null());
                if !tb.is_null() {
                    toolbars.push(tb);
                }
            }
            let tb2 = FindWindowExW(notify, null_mut(), wide("ToolbarWindow32").as_ptr(), null());
            if !tb2.is_null() {
                toolbars.push(tb2);
            }
        }
    }
    let overflow = FindWindowW(wide("NotifyIconOverflowWindow").as_ptr(), null());
    if !overflow.is_null() {
        let tb = FindWindowExW(overflow, null_mut(), wide("ToolbarWindow32").as_ptr(), null());
        if !tb.is_null() {
            toolbars.push(tb);
        }
    }

    for tb in toolbars {
        let mut pid = 0u32;
        GetWindowThreadProcessId(tb, &mut pid);
        if pid == 0 || pid == std::process::id() {
            continue;
        }
        let mut count_res = 0usize;
        if SendMessageTimeoutW(tb, TB_BUTTONCOUNT, 0, 0, SMTO_ABORTIFHUNG, 300, &mut count_res) == 0 || count_res == 0 {
            continue;
        }
        let count = count_res.min(128);
        let proc = OpenProcess(PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE, 0, pid);
        if proc.is_null() {
            continue;
        }
        let remote = VirtualAllocEx(proc, null(), 512, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if !remote.is_null() {
            #[repr(C)]
            struct TbButton64 {
                i_bitmap: i32,
                id_command: i32,
                fs_state: u8,
                fs_style: u8,
                b_reserved: [u8; 6],
                dw_data: usize,
                i_string: isize,
            }
            #[repr(C)]
            struct TrayData64 {
                hwnd: u64,
                uid: u32,
                cb: u32,
                reserved: [u32; 2],
                hicon: u64,
            }
            for idx in 0..count {
                let mut ok = 0usize;
                if SendMessageTimeoutW(tb, TB_GETBUTTON, idx, remote as isize, SMTO_ABORTIFHUNG, 200, &mut ok) == 0 || ok == 0 {
                    continue;
                }
                let mut btn: TbButton64 = zeroed();
                let mut rd = 0usize;
                if ReadProcessMemory(proc, remote, &mut btn as *mut _ as *mut _, size_of::<TbButton64>(), &mut rd) == 0 || btn.dw_data == 0 {
                    continue;
                }
                let mut td: TrayData64 = zeroed();
                if ReadProcessMemory(proc, btn.dw_data as *const _, &mut td as *mut _ as *mut _, size_of::<TrayData64>(), &mut rd) == 0 {
                    continue;
                }
                let hwnd = td.hwnd as usize as HWND;
                if hwnd.is_null() || IsWindow(hwnd) == 0 {
                    continue;
                }
                let exe = exe_of(hwnd);
                if skip_tray_exe(&exe) {
                    continue;
                }
                let mut tip = String::new();
                if btn.i_string > 0x10000 {
                    let mut buf = [0u16; 128];
                    if ReadProcessMemory(proc, btn.i_string as *const _, buf.as_mut_ptr() as *mut _, buf.len() * 2, &mut rd) != 0 {
                        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                        tip = String::from_utf16_lossy(&buf[..n]);
                    }
                }
                let n = exe.rsplit('\\').next().unwrap_or("").to_string();
                TAPPS.with_borrow_mut(|t| {
                    if let Some(existing) = t.iter_mut().find(|x| x.hwnd == hwnd && x.uid == td.uid) {
                        existing.hidden = false;
                        if existing.icon.is_null() {
                            existing.icon = copy_or_extract_icon(td.hicon as usize as HICON, hwnd, &exe);
                        }
                        if existing.tip.is_empty() && !tip.is_empty() {
                            existing.tip = tip;
                        }
                        if existing.cb == 0 && td.cb != 0 {
                            existing.cb = td.cb;
                        }
                    } else {
                        let icon = copy_or_extract_icon(td.hicon as usize as HICON, hwnd, &exe);
                        t.push(TIcon { hwnd, uid: td.uid, guid: [0; 16], icon, cb: td.cb, ver: 0, hidden: false, tip, key: n });
                    }
                });
            }
            VirtualFreeEx(proc, remote, 0, MEM_RELEASE);
        }
        CloseHandle(proc);
    }
}

pub(crate) unsafe fn tray_msg(d: &[u8]) {
    if d.len() < 32 {
        return;
    }
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let (op, flags, uid) = (u(4), u(20), u(16));
    let h = u(12) as i32 as isize as HWND;
    let has_guid = flags & 0x20 != 0 && d.len() >= 960;
    let guid: [u8; 16] = if has_guid { d[944..960].try_into().unwrap() } else { [0; 16] };
    let guid_set = has_guid && guid != [0; 16];
    TAPPS.with_borrow_mut(|t| {
        t.retain(|x| {
            let alive = IsWindow(x.hwnd) != 0;
            if !alive && !x.icon.is_null() {
                DestroyIcon(x.icon);
            }
            alive
        });
        let pos = if guid_set {
            t.iter().position(|x| x.guid == guid || (x.guid == [0; 16] && x.hwnd == h && x.uid == uid))
        } else {
            t.iter().position(|x| x.hwnd == h && x.uid == uid)
        };
        match (op, pos) {
            (2, Some(i)) => {
                let removed = t.remove(i);
                if !removed.icon.is_null() {
                    DestroyIcon(removed.icon);
                }
            }
            (4, Some(i)) if d.len() >= 812 => t[i].ver = u(808),
            (0 | 1, _) => {
                let mut cached_exe: Option<String> = None;
                let i = match pos {
                    Some(i) => i,
                    None => {
                        if h.is_null() || IsWindow(h) == 0 {
                            return;
                        }
                        let exe = exe_of(h);
                        if skip_tray_exe(&exe) {
                            return;
                        }
                        let n = exe.rsplit('\\').next().unwrap_or("").to_string();
                        cached_exe = Some(exe);
                        t.push(TIcon { hwnd: h, uid, guid, icon: null_mut(), cb: 0, ver: 0, hidden: false, tip: String::new(), key: n });
                        t.len() - 1
                    }
                };
                let x = &mut t[i];
                if guid_set {
                    x.guid = guid;
                }
                if flags & 1 != 0 {
                    x.cb = u(24);
                }
                if flags & 2 != 0 || x.icon.is_null() {
                    let raw = if flags & 2 != 0 { u(28) as i32 as isize as HICON } else { null_mut() };
                    let exe = cached_exe.as_deref();
                    let owned_exe;
                    let exe_ref = match exe {
                        Some(s) => s,
                        None => {
                            owned_exe = exe_of(h);
                            &owned_exe
                        }
                    };
                    let ic = copy_or_extract_icon(raw, h, exe_ref);
                    if !ic.is_null() {
                        if !x.icon.is_null() {
                            DestroyIcon(x.icon);
                        }
                        x.icon = ic;
                    }
                }
                if flags & 4 != 0 && d.len() >= 160 {
                    let end = if d.len() >= 288 { 288 } else { 160 };
                    x.tip.clear();
                    x.tip.extend(
                        char::decode_utf16(
                            d[32..end]
                                .chunks_exact(2)
                                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                .take_while(|&c| c != 0),
                        )
                        .map(|r| r.unwrap_or('\u{FFFD}')),
                    );
                }
                if flags & 8 != 0 && d.len() >= 296 && u(292) & 1 != 0 {
                    x.hidden = u(288) & 1 != 0;
                } else if op == 0 {
                    x.hidden = false;
                }
            }
            _ => {}
        }
    });
}

pub(crate) unsafe extern "system" fn tray_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
        let c = &*(lp as *const Cds);
        if c.dw == 1 && c.cb >= 32 {
            tray_msg(std::slice::from_raw_parts(c.p, c.cb as usize));
            sync_trayfly();
            return 1;
        }
        if c.dw == 3 {
            let bar = BAR.load(Relaxed) as HWND;
            let (x, y) = if !bar.is_null() {
                let mut r: RECT = zeroed();
                GetWindowRect(bar, &mut r);
                let l = bar_layout(bar);
                if vert() { (r.left + (r.right - r.left) / 2, r.top + l.apps.0) } else { (r.left + l.apps.0, r.top + (r.bottom - r.top) / 2) }
            } else {
                (0, 0)
            };
            return (((x as u32) & 0xFFFF) | (((y as u32) & 0xFFFF) << 16)) as isize;
        }
        let other = find_other("Shell_TrayWnd");
        if !other.is_null() {
            let mut r = 0usize;
            SendMessageTimeoutW(other, msg, wp, lp, SMTO_ABORTIFHUNG, 2000, &mut r);
            return r as LRESULT;
        }
        return 0;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

thread_local! {
    static LAST_CLICK: Cell<(usize, u32, u32)> = const { Cell::new((0, 0, 0)) };
}

pub(crate) unsafe fn tray_click(t: (HWND, u32, u32, u32), left: bool) {
    let (hwnd, uid, cb, ver) = t;
    if cb == 0 || IsWindow(hwnd) == 0 {
        return;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    AllowSetForegroundWindow(u32::MAX);
    AllowSetForegroundWindow(pid);
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let post = |ev: u32| {
        if ver >= 4 {
            PostMessageW(hwnd, cb, ((pt.x as u32 & 0xffff) | ((pt.y as u32 & 0xffff) << 16)) as usize, (ev | (uid & 0xffff) << 16) as isize);
        } else {
            PostMessageW(hwnd, cb, uid as usize, ev as isize);
        }
    };
    let now = windows_sys::Win32::System::SystemInformation::GetTickCount();
    let prev = if left { LAST_CLICK.replace((hwnd as usize, uid, now)) } else { (0, 0, 0) };
    let double = left && (prev.0, prev.1) == (hwnd as usize, uid) && now.wrapping_sub(prev.2) <= GetDoubleClickTime();
    if double {
        LAST_CLICK.set((0, 0, 0));
    }
    match (left, ver >= 4) {
        (true, v4) => {
            [WM_LBUTTONDOWN, WM_LBUTTONUP].into_iter().for_each(post);
            if v4 { post(0x400); }
            if double { post(WM_LBUTTONDBLCLK); }
        }
        (false, v4) => {
            [WM_RBUTTONDOWN, WM_RBUTTONUP].into_iter().for_each(post);
            if v4 { post(WM_CONTEXTMENU); }
        }
    }
}

pub(crate) fn bar_idx() -> Vec<usize> {
    SHOWN.with_borrow(|shown| {
        TAPPS.with_borrow(|t| t.iter().enumerate().filter(|(_, x)| !x.hidden && shown.as_ref().is_some_and(|s| s.contains(&x.key))).map(|(i, _)| i).collect())
    })
}
pub(crate) fn tray_count() -> usize {
    SHOWN.with_borrow(|shown| {
        TAPPS.with_borrow(|t| t.iter().filter(|x| !x.hidden && shown.as_ref().is_some_and(|s| s.contains(&x.key))).count())
    }) + 1
}

pub(crate) fn over_idx() -> Vec<usize> {
    SHOWN.with_borrow(|shown| {
        TAPPS.with_borrow(|t| t.iter().enumerate().filter(|(_, x)| !x.hidden && !shown.as_ref().is_some_and(|s| s.contains(&x.key))).map(|(i, _)| i).collect())
    })
}

pub(crate) fn toggle_shown(key: &str) {
    let mut cur = SHOWN.with_borrow(|s| s.clone()).unwrap_or_else(|| TAPPS.with_borrow(|t| bar_idx().into_iter().map(|i| t[i].key.clone()).collect()));
    match cur.iter().position(|k| k == key) {
        Some(p) => drop(cur.remove(p)),
        None => cur.push(key.to_string()),
    }
    let _ = std::fs::write(appdata_file("topbar-tray.txt"), cur.join("\n"));
    SHOWN.set(Some(cur));
}
