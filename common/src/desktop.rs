use std::sync::Mutex;
use windows_sys::{
    core::BOOL,
    Win32::{Foundation::{HWND, LPARAM}, Graphics::Dwm::DwmGetWindowAttribute, UI::WindowsAndMessaging::*},
};

static HIDDEN: Mutex<Vec<isize>> = Mutex::new(Vec::new());

pub fn toggle() {
    let mut hid = HIDDEN.lock().unwrap();
    unsafe {
        let mut v: Vec<isize> = vec![];
        EnumWindows(Some(collect), &mut v as *mut _ as LPARAM);
        if !v.is_empty() {
            v.iter().for_each(|&h| { ShowWindow(h as HWND, SW_MINIMIZE); });
            *hid = v;
        } else {
            hid.iter().for_each(|&h| { if IsWindow(h as HWND) != 0 { ShowWindow(h as HWND, SW_RESTORE); } });
            hid.clear();
        }
    }
}

unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
    unsafe {
        if IsWindowVisible(h) == 0 || IsIconic(h) != 0 {
            return 1;
        }
        if GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
            return 1;
        }
        if !GetWindow(h, GW_OWNER).is_null() || GetWindowTextLengthW(h) <= 0 {
            return 1;
        }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(h, 14, &mut cloaked as *mut _ as _, 4);
        if cloaked == 0 && !is_desktop(h) {
            (*(lp as *mut Vec<isize>)).push(h as isize);
        }
        1
    }
}

fn is_desktop(h: HWND) -> bool {
    const PROGMAN: [u16; 7] = [b'P' as u16, b'r' as u16, b'o' as u16, b'g' as u16, b'm' as u16, b'a' as u16, b'n' as u16];
    const WORKERW: [u16; 7] = [b'W' as u16, b'o' as u16, b'r' as u16, b'k' as u16, b'e' as u16, b'r' as u16, b'W' as u16];
    let mut cls = [0u16; 32];
    let n = unsafe { GetClassNameW(h, cls.as_mut_ptr(), cls.len() as i32) } as usize;
    cls[..n] == PROGMAN || cls[..n] == WORKERW
}

#[cfg(test)]
fn is_desktop_class(c: &str) -> bool {
    matches!(c, "Progman" | "WorkerW")
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_classes() {
        assert!(super::is_desktop_class("Progman"));
        assert!(!super::is_desktop_class("Notepad"));
    }
}
