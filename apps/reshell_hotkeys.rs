use std::sync::{
    OnceLock,
    atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering::Relaxed},
    mpsc::{Sender, channel},
};
use windows_sys::Win32::{Foundation::*, System::{LibraryLoader::GetModuleHandleW, Registry::*}, UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*}};

static PREV: AtomicUsize = AtomicUsize::new(usize::MAX);
static HOOK: AtomicIsize = AtomicIsize::new(0);
static HELD: AtomicU32 = AtomicU32::new(0);
static TOPBAR: [u16; 7] = [b't' as u16, b'o' as u16, b'p' as u16, b'b' as u16, b'a' as u16, b'r' as u16, 0];

enum Work {
    Action(fn()),
    Desk(usize),
}

fn worker() -> &'static Sender<Work> {
    static TX: OnceLock<Sender<Work>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel::<Work>();
        std::thread::spawn(move || {
            while let Ok(w) = rx.recv() {
                match w {
                    Work::Action(f) => f(),
                    Work::Desk(id) => go(if id == usize::MAX { PREV.load(Relaxed) } else { id }),
                }
            }
        });
        tx
    })
}

const VD_KEY: [u16; 67] = [
    b'S' as u16, b'O' as u16, b'F' as u16, b'T' as u16, b'W' as u16, b'A' as u16, b'R' as u16, b'E' as u16, b'\\' as u16,
    b'M' as u16, b'i' as u16, b'c' as u16, b'r' as u16, b'o' as u16, b's' as u16, b'o' as u16, b'f' as u16, b't' as u16, b'\\' as u16,
    b'W' as u16, b'i' as u16, b'n' as u16, b'd' as u16, b'o' as u16, b'w' as u16, b's' as u16, b'\\' as u16,
    b'C' as u16, b'u' as u16, b'r' as u16, b'r' as u16, b'e' as u16, b'n' as u16, b't' as u16,
    b'V' as u16, b'e' as u16, b'r' as u16, b's' as u16, b'i' as u16, b'o' as u16, b'n' as u16, b'\\' as u16,
    b'E' as u16, b'x' as u16, b'p' as u16, b'l' as u16, b'o' as u16, b'r' as u16, b'e' as u16, b'r' as u16, b'\\' as u16,
    b'V' as u16, b'i' as u16, b'r' as u16, b't' as u16, b'u' as u16, b'a' as u16, b'l' as u16,
    b'D' as u16, b'e' as u16, b's' as u16, b'k' as u16, b't' as u16, b'o' as u16, b'p' as u16, b's' as u16, 0,
];
const VD_IDS: [u16; 18] = [
    b'V' as u16, b'i' as u16, b'r' as u16, b't' as u16, b'u' as u16, b'a' as u16, b'l' as u16,
    b'D' as u16, b'e' as u16, b's' as u16, b'k' as u16, b't' as u16, b'o' as u16, b'p' as u16, b'I' as u16, b'D' as u16, b's' as u16, 0,
];
const VD_CUR: [u16; 22] = [
    b'C' as u16, b'u' as u16, b'r' as u16, b'r' as u16, b'e' as u16, b'n' as u16, b't' as u16,
    b'V' as u16, b'i' as u16, b'r' as u16, b't' as u16, b'u' as u16, b'a' as u16, b'l' as u16,
    b'D' as u16, b'e' as u16, b's' as u16, b'k' as u16, b't' as u16, b'o' as u16, b'p' as u16, 0,
];

fn desktops() -> Option<(usize, usize)> {
    let mut ids = [0u8; 1024];
    let mut ids_len = ids.len() as u32;
    let mut cur = [0u8; 16];
    let mut cur_len = cur.len() as u32;
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, VD_KEY.as_ptr(), VD_IDS.as_ptr(), RRF_RT_REG_BINARY, std::ptr::null_mut(), ids.as_mut_ptr().cast(), &mut ids_len) != 0 {
            return None;
        }
        if RegGetValueW(HKEY_CURRENT_USER, VD_KEY.as_ptr(), VD_CUR.as_ptr(), RRF_RT_REG_BINARY, std::ptr::null_mut(), cur.as_mut_ptr().cast(), &mut cur_len) != 0 || cur_len != 16 {
            return None;
        }
    }
    index_in(&ids[..ids_len as usize], &cur)
}

fn index_in(ids: &[u8], cur: &[u8]) -> Option<(usize, usize)> {
    let i = ids.chunks_exact(16).position(|c| c == cur)?;
    Some((i, ids.len() / 16))
}

fn tap(vk: u16, up: bool) -> INPUT {
    let mut i: INPUT = unsafe { std::mem::zeroed() };
    i.r#type = INPUT_KEYBOARD;
    i.Anonymous.ki = KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 };
    i
}

fn send(keys: &[INPUT]) {
    unsafe { SendInput(keys.len() as u32, keys.as_ptr(), size_of::<INPUT>() as i32) };
}

fn open_files() {
    use std::{path::PathBuf, process::{Command, Stdio}};
    let mine = std::env::var_os("LOCALAPPDATA").map(|b| PathBuf::from(b).join(r"plugin-launcher\bin\rust-explorer.exe"));
    let beside = std::env::current_exe().ok().map(|e| e.with_file_name("rust-explorer.exe"));
    let installed = std::env::var_os("ProgramFiles").map(|b| PathBuf::from(b).join(r"rust-explorer\rust-explorer.exe"));
    let exe = [installed, mine, beside].into_iter().flatten().filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p))).max().map(|(_, p)| p).unwrap_or_else(|| "explorer.exe".into());
    let _ = Command::new(exe).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

fn open_settings() {
    winrust_common::settings::open("ms-settings:");
}

fn win_action(vk: u32) -> Option<fn()> {
    match vk {
        0x45 => Some(open_files),
        0x44 => Some(winrust_common::desktop::toggle),
        0x49 => Some(open_settings),
        _ => None,
    }
}

fn go(target: usize) {
    let Some((cur, count)) = desktops() else { return };
    if target == cur || target >= count { return }
    PREV.store(cur, Relaxed);
    let key = if target > cur { VK_RIGHT } else { VK_LEFT };
    for _ in 0..cur.abs_diff(target) {
        send(&[tap(VK_CONTROL, false), tap(key, false), tap(key, true), tap(VK_CONTROL, true)]);
    }
}

unsafe extern "system" fn hook(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if n >= 0 {
        let k = unsafe { &*(l as *const KBDLLHOOKSTRUCT) };
        let is_action = matches!(k.vkCode, 0x44 | 0x45 | 0x49);
        let is_desk = matches!(k.vkCode, 0x31..=0x33 | 0xC0);
        if is_action || is_desk {
            if (w == WM_KEYUP as usize || w == WM_SYSKEYUP as usize) && HELD.load(Relaxed) == k.vkCode {
                HELD.store(0, Relaxed);
            }
            let win = unsafe { (GetAsyncKeyState(VK_LWIN as i32) | GetAsyncKeyState(VK_RWIN as i32)) as u16 & 0x8000 != 0 };
            if win {
                if let (Some(act), true) = (win_action(k.vkCode), k.flags & LLKHF_INJECTED == 0) {
                    if w == WM_KEYDOWN as usize && HELD.swap(k.vkCode, Relaxed) != k.vkCode {
                        send(&[tap(0xE8, false), tap(0xE8, true)]);
                        let _ = worker().send(Work::Action(act));
                    }
                    return 1;
                }
                if is_desk && k.flags & LLKHF_INJECTED == 0 {
                    let id = if k.vkCode == 0xC0 {
                        Some(usize::MAX)
                    } else {
                        let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000 != 0 };
                        let bar = if ctrl { false } else { unsafe { !FindWindowW(TOPBAR.as_ptr(), std::ptr::null()).is_null() } };
                        (!bar || ctrl).then(|| k.vkCode as usize - 0x31)
                    };
                    if let Some(id) = id {
                        if w == WM_KEYDOWN as usize {
                            send(&[tap(0xE8, false), tap(0xE8, true)]);
                            let _ = worker().send(Work::Desk(id));
                        }
                        return 1;
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(HOOK.load(Relaxed) as _, n, w, l) }
}

pub fn run() -> i32 {
    let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), GetModuleHandleW(std::ptr::null()), 0) };
    if h.is_null() { eprintln!("hook failed"); return 1 }
    HOOK.store(h as isize, Relaxed);
    let mut m: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut m, std::ptr::null_mut(), 0, 0) } > 0 {}
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_index() {
        let ids = [[1u8; 16], [2; 16], [3; 16]].concat();
        assert_eq!(super::index_in(&ids, &[2; 16]), Some((1, 3)));
        assert_eq!(super::index_in(&ids, &[9; 16]), None);
    }
}
