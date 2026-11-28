use windows_sys::Win32::{
    Foundation::HWND,
    UI::{
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::{AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, SetForegroundWindow},
    },
};

pub fn force(h: HWND) {
    unsafe {
        let _ = AllowSetForegroundWindow(u32::MAX);
        if SetForegroundWindow(h) == 0 || GetForegroundWindow() != h {
            BringWindowToTop(h);
            SetForegroundWindow(h);
        }
        SetFocus(h);
    }
}
