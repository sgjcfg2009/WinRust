use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SMTO_BLOCK, SendMessageTimeoutW, SetForegroundWindow, WM_NULL,
        },
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
            let mut res: usize = 0;
            let alive = ft != 0
                && ft != me
                && SendMessageTimeoutW(fg, WM_NULL, 0, 0, SMTO_ABORTIFHUNG | SMTO_BLOCK, 50, &mut res) != 0;
            let attach = alive && AttachThreadInput(me, ft, 1) != 0;
            SetForegroundWindow(h);
            if attach {
                AttachThreadInput(me, ft, 0);
            }
        }
        SetFocus(h);
    }
}
