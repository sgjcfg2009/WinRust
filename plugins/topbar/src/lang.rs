use super::*;

pub(crate) unsafe fn next_lang(bar: HWND, back: bool) {
    let mut buf: [HKL; 32] = [null_mut(); 32];
    let n = GetKeyboardLayoutList(32, buf.as_mut_ptr()).clamp(0, 32) as usize;
    if n < 2 {
        return;
    }
    let l = &buf[..n];
    let own = own_foreground();
    let cur = GetKeyboardLayout(if own { 0 } else { GetWindowThreadProcessId(app_window(), null_mut()) });
    let i = l.iter().position(|&k| k == cur);
    let next = l[match (i, back) { (Some(i), false) => (i + 1) % n, (Some(i), true) => (i + n - 1) % n, (None, _) => 0 }];
    if own {
        ActivateKeyboardLayout(next, 0);
        InvalidateRect(bar, null(), 0);
    } else {
        set_lang(bar, next);
    }
}

pub(crate) unsafe fn set_lang(bar: HWND, next: HKL) {
    let w = app_window();
    let tid = GetWindowThreadProcessId(w, null_mut());
    let cur = GetKeyboardLayout(tid);
    let mut gi: GUITHREADINFO = zeroed();
    gi.cbSize = size_of::<GUITHREADINFO>() as u32;
    let target = if GetGUIThreadInfo(tid, &mut gi) != 0 && !gi.hwndFocus.is_null() { gi.hwndFocus } else { w };
    PostMessageW(target, WM_INPUTLANGCHANGEREQUEST, 0, next as isize);
    LANG_TRY.set((tid, cur as usize, next as usize));
    SetTimer(bar, 7, 250, None);
}

thread_local! {
    pub(crate) static SIG: Cell<u64> = const { Cell::new(0) };
}
