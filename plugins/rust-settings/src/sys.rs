use std::ptr::null_mut;
use windows_sys::Win32::{Foundation::*, System::Registry::*, UI::WindowsAndMessaging::*};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub fn dword(root: HKEY, key: &str, name: &str) -> Option<u32> {
    let (mut v, mut n) = (0u32, 4u32);
    let rc = unsafe { RegGetValueW(root, wide(key).as_ptr(), wide(name).as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut v as *mut u32 as _, &mut n) };
    (rc == ERROR_SUCCESS).then_some(v)
}

pub fn string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 256];
    let mut n = size_of_val(&buf) as u32;
    let rc = unsafe { RegGetValueW(root, wide(key).as_ptr(), wide(name).as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr() as _, &mut n) };
    (rc == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(n as usize / 2).saturating_sub(1)]))
}

pub fn set_hkcu_dword(key: &str, name: &str, v: u32) -> bool {
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr(), REG_DWORD, &v as *const u32 as _, 4) == ERROR_SUCCESS }
}

pub fn announce_theme() {
    std::thread::spawn(|| {
        let what = wide("ImmersiveColorSet");
        unsafe { SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, what.as_ptr() as isize, SMTO_ABORTIFHUNG, 2000, null_mut()) };
    });
}

pub fn string_long(root: HKEY, key: &str, name: &str) -> Option<String> {
    let mut n = 0u32;
    let (k, v) = (wide(key), wide(name));
    if unsafe { RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut n) } != ERROR_SUCCESS || n < 2 {
        return None;
    }
    let mut buf = vec![0u16; n as usize / 2 + 1];
    let mut n2 = (buf.len() * 2) as u32;
    let rc = unsafe { RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr() as _, &mut n2) };
    (rc == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(n2 as usize / 2).saturating_sub(1)]))
}

pub fn set_hkcu_string(key: &str, name: &str, v: &str) -> bool {
    unsafe {
        if v.is_empty() {
            let rc = RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr());
            return rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND;
        }
        let w = wide(v);
        RegSetKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr(), REG_SZ, w.as_ptr() as _, (w.len() * 2) as u32) == ERROR_SUCCESS
    }
}

pub fn subkeys(root: HKEY, key: &str) -> Vec<String> {
    let mut out = vec![];
    unsafe {
        let mut h: HKEY = null_mut();
        if RegOpenKeyExW(root, wide(key).as_ptr(), 0, KEY_READ, &mut h) != ERROR_SUCCESS {
            return out;
        }
        for i in 0.. {
            let mut buf = [0u16; 256];
            let mut n = buf.len() as u32;
            if RegEnumKeyExW(h, i, buf.as_mut_ptr(), &mut n, null_mut(), null_mut(), null_mut(), null_mut()) != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..n as usize]));
        }
        RegCloseKey(h);
    }
    out
}

pub fn wallpaper_name() -> String {
    let p = string_long(HKEY_CURRENT_USER, r"Control Panel\Desktop", "WallPaper").unwrap_or_default();
    p.rsplit('\\').next().unwrap_or_default().to_string()
}

pub fn pick_wallpaper(owner: HWND) -> Result<(), ()> {
    use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let mut file = [0u16; 1024];
    let filter: Vec<u16> = "Images\0*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.tif;*.tiff;*.webp\0\0".encode_utf16().collect();
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file.as_mut_ptr();
    ofn.nMaxFile = file.len() as u32;
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return Err(());
    }
    let ok = unsafe { SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, file.as_mut_ptr() as _, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE) };
    if ok != 0 { Ok(()) } else { Err(()) }
}
