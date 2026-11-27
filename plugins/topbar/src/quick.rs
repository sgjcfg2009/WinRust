use std::sync::atomic::{AtomicBool, AtomicI8, Ordering::Relaxed};
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows_sys::{
    Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
    core::GUID,
};

static BT: AtomicI8 = AtomicI8::new(-2);
static AIR: AtomicI8 = AtomicI8::new(-2);
static BUSY: AtomicBool = AtomicBool::new(false);

fn radios() -> Vec<Radio> {
    unsafe { let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED); }
    let _ = Radio::RequestAccessAsync().and_then(|a| a.join());
    Radio::GetRadiosAsync().and_then(|a| a.join()).map(|v| v.into_iter().collect()).unwrap_or_default()
}

pub fn refresh() {
    if BUSY.swap(true, Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        let rs = radios();
        let (mut has_bt, mut bt_on, mut all_off) = (false, false, true);
        for r in &rs {
            let is_on = r.State().is_ok_and(|s| s == RadioState::On);
            if is_on {
                all_off = false;
            }
            if r.Kind().is_ok_and(|k| k == RadioKind::Bluetooth) {
                has_bt = true;
                bt_on |= is_on;
            }
        }
        BT.store(if !has_bt { -1 } else { bt_on as i8 }, Relaxed);
        AIR.store(if rs.is_empty() { -1 } else { all_off as i8 }, Relaxed);
        BUSY.store(false, Relaxed);
    });
}

pub fn bluetooth() -> Option<bool> {
    match BT.load(Relaxed) {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub fn airplane() -> Option<bool> {
    match AIR.load(Relaxed) {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub fn set(air: bool, on: bool) {
    if air {
        AIR.store(on as i8, Relaxed);
    } else {
        BT.store(on as i8, Relaxed);
    }
    std::thread::spawn(move || {
        for r in radios() {
            let kind = r.Kind().ok();
            let (hit, state) = if air { (on || matches!(kind, Some(RadioKind::WiFi | RadioKind::Bluetooth)), if on { RadioState::Off } else { RadioState::On }) } else { (kind == Some(RadioKind::Bluetooth), if on { RadioState::On } else { RadioState::Off }) };
            if hit {
                let _ = r.SetStateAsync(state).and_then(|a| a.join());
            }
        }
        BUSY.store(false, Relaxed);
        refresh();
    });
}

pub fn set_wifi(on: bool, fallback: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let state = if on { RadioState::On } else { RadioState::Off };
        let done = radios().iter().filter(|r| r.Kind().is_ok_and(|k| k == RadioKind::WiFi)).fold(None, |acc: Option<bool>, r| {
            let ok = r.SetStateAsync(state).and_then(|a| a.join()).is_ok_and(|s| s == windows::Devices::Radios::RadioAccessStatus::Allowed);
            Some(acc.unwrap_or(true) && ok)
        });
        if done != Some(true) {
            fallback();
        }
        refresh();
    });
}

const EFFICIENCY: GUID = GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a);
const BALANCED: GUID = GUID::from_u128(0);

type PowerGetFn = unsafe extern "system" fn(*mut GUID) -> u32;
type PowerSetFn = unsafe extern "system" fn(GUID) -> u32;

fn powrprof_fns() -> (Option<PowerGetFn>, Option<PowerSetFn>) {
    static FNS: std::sync::OnceLock<(Option<PowerGetFn>, Option<PowerSetFn>)> = std::sync::OnceLock::new();
    *FNS.get_or_init(|| unsafe {
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        const DLL: [u16; 13] = [b'p' as u16, b'o' as u16, b'w' as u16, b'r' as u16, b'p' as u16, b'r' as u16, b'o' as u16, b'f' as u16, b'.' as u16, b'd' as u16, b'l' as u16, b'l' as u16, 0];
        let h = LoadLibraryW(DLL.as_ptr());
        if h.is_null() {
            return (None, None);
        }
        let get = GetProcAddress(h, b"PowerGetActualOverlayScheme\0".as_ptr()).map(|f| std::mem::transmute(f));
        let set = GetProcAddress(h, b"PowerSetActiveOverlayScheme\0".as_ptr()).map(|f| std::mem::transmute(f));
        (get, set)
    })
}

fn overlay() -> Option<GUID> {
    let f = powrprof_fns().0?;
    let mut g = BALANCED;
    (unsafe { f(&mut g) } == 0).then_some(g)
}

pub fn energy_saver() -> bool {
    let eco = overlay().is_some_and(|g| (g.data1, g.data2, g.data3, g.data4) == (EFFICIENCY.data1, EFFICIENCY.data2, EFFICIENCY.data3, EFFICIENCY.data4));
    let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    eco || (unsafe { GetSystemPowerStatus(&mut s) } != 0 && s.SystemStatusFlag == 1)
}

pub fn set_energy_saver(on: bool) {
    if let Some(f) = powrprof_fns().1 {
        unsafe { f(if on { EFFICIENCY } else { BALANCED }) };
    }
}
