use super::*;

pub(crate) fn run_dialog() {
    std::thread::spawn(|| unsafe {
        let sh = LoadLibraryW(wide("shell32.dll").as_ptr());
        if !sh.is_null() {
            if let Some(f) = GetProcAddress(sh, 61 as *const u8) {
                type RunFileDlg = unsafe extern "system" fn(HWND, HICON, *const u16, *const u16, *const u16, u32);
                std::mem::transmute::<_, RunFileDlg>(f)(null_mut(), null_mut(), null(), null(), null(), 0);
            }
            FreeLibrary(sh);
        }
    });
}

pub(crate) unsafe fn run_box() {
    let mut h: HWND = null_mut();
    EnumWindows(Some(find_launcher), &mut h as *mut _ as LPARAM);
    if h.is_null() {
        return run_dialog();
    }
    let mut pid = 0;
    GetWindowThreadProcessId(h, &mut pid);
    AllowSetForegroundWindow(pid);
    PostMessageW(h, WM_HOTKEY, 2, 0);
}

pub(crate) unsafe extern "system" fn kb_hook(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if n >= 0 {
        let k = &*(l as *const KBDLLHOOKSTRUCT);
        let down = w == WM_KEYDOWN as usize || w == WM_SYSKEYDOWN as usize;
        let bar = BAR.load(Relaxed) as HWND;
        let exp_up = EXPLORER_UP.load(Relaxed);
        if !exp_up {
            let vol = match k.vkCode as u16 {
                VK_VOLUME_UP => Some(0),
                VK_VOLUME_DOWN => Some(1),
                VK_VOLUME_MUTE => Some(2),
                _ => None,
            };
            if let Some(v) = vol {
                if down {
                    PostMessageW(bar, VOLKEY, v, 0);
                }
                return 1;
            }
            if k.vkCode == VK_SNAPSHOT as u32 && k.flags & LLKHF_INJECTED != 0 {
                if down {
                    PostMessageW(bar, SNIP, 1, 0);
                }
                return 1;
            }
        }
        if k.flags & LLKHF_INJECTED == 0 {
            let held = |vk: u16| GetAsyncKeyState(vk as i32) < 0;
            let vk = k.vkCode;
            if matches!(vk, 0x2C | 0x53 | 0x31..=0x39 | 0x20 | 0x52 | 0x09 | 0x58) {
                let win = held(VK_LWIN) || held(VK_RWIN);
                let snip_key = !exp_up && vk == VK_SNAPSHOT as u32;
                let win_shift_s = vk == b'S' as u32 && down && win && held(VK_SHIFT);
                if snip_key || win_shift_s {
                    if down {
                        WIN_ALONE.store(false, Relaxed);
                        if win_shift_s {
                            keys(&[0xE8]);
                        }
                        PostMessageW(bar, SNIP, snip_key as usize, 0);
                    }
                    return 1;
                }
                if win && !held(VK_CONTROL) && !held(VK_MENU) {
                    let no_shift = !held(VK_SHIFT);
                    if (0x31..=0x39).contains(&vk) && no_shift {
                        if down {
                            WIN_ALONE.store(false, Relaxed);
                            keys(&[0xE8]);
                            PostMessageW(bar, WIN_NUM, vk as usize - 0x31, 0);
                        }
                        return 1;
                    }
                    if vk == VK_SPACE as u32 && (!exp_up || own_foreground()) {
                        if down {
                            WIN_ALONE.store(false, Relaxed);
                            keys(&[0xE8]);
                            PostMessageW(bar, NEXT_LANG, held(VK_SHIFT) as usize, 0);
                        }
                        return 1;
                    }
                    if no_shift && matches!(vk as u16, 0x52 | 0x09 | 0x58) {
                        if down {
                            WIN_ALONE.store(false, Relaxed);
                            keys(&[0xE8]);
                            match vk as u16 {
                                0x52 => run_box(),
                                0x09 => { PostMessageW(bar, WIN_TAB, 0, 0); }
                                _ => { PostMessageW(bar, WIN_X, 0, 0); }
                            }
                        }
                        return 1;
                    }
                }
            }
            if vk == VK_TAB as u32 && k.flags & LLKHF_ALTDOWN != 0 && !held(VK_CONTROL) {
                if down {
                    SWITCHING.store(true, Relaxed);
                    PostMessageW(bar, ALT_TAB, held(VK_SHIFT) as usize, 0);
                }
                return 1;
            }
            if SWITCHING.load(Relaxed) {
                if vk == VK_ESCAPE as u32 && down {
                    SWITCHING.store(false, Relaxed);
                    PostMessageW(bar, ALT_UP, 1, 0);
                    return 1;
                }
                if matches!(vk as u16, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_DELETE | VK_RETURN) {
                    if down {
                        match vk as u16 {
                            VK_LEFT | VK_UP => { PostMessageW(bar, ALT_TAB, 1, 0); }
                            VK_RIGHT | VK_DOWN => { PostMessageW(bar, ALT_TAB, 0, 0); }
                            VK_DELETE => { PostMessageW(bar, ALT_UP, 2, 0); }
                            _ => {
                                SWITCHING.store(false, Relaxed);
                                PostMessageW(bar, ALT_UP, 0, 0);
                                keys(&[0xE8]);
                            }
                        }
                    }
                    return 1;
                }
                if matches!(vk as u16, VK_LMENU | VK_RMENU | VK_MENU) && !down {
                    SWITCHING.store(false, Relaxed);
                    keys(&[0xE8]);
                    PostMessageW(bar, ALT_UP, 0, 0);
                }
            }
            if vk == VK_LWIN as u32 || vk == VK_RWIN as u32 {
                if down {
                    WIN_ALONE.store(true, Relaxed);
                } else if WIN_ALONE.swap(false, Relaxed) {
                    keys(&[0xE8]);
                    PostMessageW(bar, WIN_START, 0, 0);
                }
            } else if down {
                WIN_ALONE.store(false, Relaxed);
            }
        }
    }
    CallNextHookEx(null_mut(), n, w, l)
}

