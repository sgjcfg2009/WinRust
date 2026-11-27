use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow},
    },
};

pub fn force(h: HWND) {
    unsafe {
        if SetForegroundWindow(h) != 0 && GetForegroundWindow() == h {
            SetFocus(h);
            return;
        }
        let fg = GetForegroundWindow();
        if !fg.is_null() && fg != h {
            let me = GetCurrentThreadId();
            let ft = GetWindowThreadProcessId(fg, std::ptr::null_mut());
            let attach = ft != 0 && ft != me && AttachThreadInput(me, ft, 1) != 0;
            SetForegroundWindow(h);
            if attach {
                AttachThreadInput(me, ft, 0);
            }
        }
        SetFocus(h);
    }
}
