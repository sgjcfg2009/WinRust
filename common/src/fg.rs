use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{INPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP, SendInput, SetFocus, VK_MENU},
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
        if GetForegroundWindow() != h {
            keys(&[(VK_MENU, false), (0xE8, false), (0xE8, true)]);
            SetForegroundWindow(h);
            keys(&[(VK_MENU, true)]);
        }
        SetFocus(h);
    }
}

fn keys(k: &[(u16, bool)]) {
    let mut buf: [INPUT; 4] = unsafe { std::mem::zeroed() };
    let n = k.len().min(buf.len());
    for (slot, &(vk, up)) in buf[..n].iter_mut().zip(k) {
        slot.r#type = INPUT_KEYBOARD;
        slot.Anonymous.ki.wVk = vk;
        slot.Anonymous.ki.dwFlags = if up { KEYEVENTF_KEYUP } else { 0 };
    }
    if n > 0 {
        unsafe { SendInput(n as u32, buf.as_ptr(), size_of::<INPUT>() as i32) };
    }
}
