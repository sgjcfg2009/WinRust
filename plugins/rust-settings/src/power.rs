use std::ptr::{null, null_mut};
use windows_sys::{
    Win32::{Foundation::*, Storage::FileSystem::*, System::Power::*},
    core::GUID,
};

const DRIVE_FIXED: u32 = 3;

fn same(a: &GUID, b: &GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}

pub fn size(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    let b = bytes as f64;
    if b >= 1024.0 * GB { format!("{:.2} TB", b / (1024.0 * GB)) } else { format!("{:.1} GB", b / GB) }
}

pub fn battery(on_ac: u8, flag: u8, percent: u8) -> String {
    if flag == 128 || flag == 255 || percent > 100 {
        return "No battery".into();
    }
    let src = if on_ac == 1 { "plugged in" } else { "on battery" };
    format!("{percent}%, {src}")
}

pub fn battery_text() -> String {
    let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return "?".into();
    }
    battery(s.ACLineStatus, s.BatteryFlag, s.BatteryLifePercent)
}

pub fn drives() -> Vec<(char, u64, u64)> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask >> i & 1 == 1)
        .filter_map(|i| {
            let root = [b'A' as u16 + i as u16, b':' as u16, b'\\' as u16, 0];
            if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
                return None;
            }
            let (mut free, mut total) = (0u64, 0u64);
            (unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, null_mut()) } != 0).then_some(((b'A' + i) as char, free, total))
        })
        .collect()
}

fn schemes() -> Vec<GUID> {
    let mut out = vec![];
    for i in 0.. {
        let mut g: GUID = unsafe { std::mem::zeroed() };
        let mut n = size_of::<GUID>() as u32;
        if unsafe { PowerEnumerate(null_mut(), null(), null(), ACCESS_SCHEME, i, &mut g as *mut GUID as *mut u8, &mut n) } != ERROR_SUCCESS {
            break;
        }
        out.push(g);
    }
    out
}

fn active() -> Option<GUID> {
    let mut p: *mut GUID = null_mut();
    if unsafe { PowerGetActiveScheme(null_mut(), &mut p) } != ERROR_SUCCESS || p.is_null() {
        return None;
    }
    let g = unsafe { *p };
    unsafe { LocalFree(p as _) };
    Some(g)
}

fn name_of(g: &GUID) -> String {
    let mut buf = [0u8; 256];
    let mut n = buf.len() as u32;
    let rc = unsafe { PowerReadFriendlyName(null_mut(), g, null(), null(), buf.as_mut_ptr(), &mut n) };
    let v: Vec<u8>;
    let slice = if rc == ERROR_SUCCESS {
        &buf[..(n as usize).min(buf.len())]
    } else if rc == ERROR_MORE_DATA && n >= 2 {
        v = vec![0u8; n as usize];
        let mut v = v;
        if unsafe { PowerReadFriendlyName(null_mut(), g, null(), null(), v.as_mut_ptr(), &mut n) } != ERROR_SUCCESS {
            return "?".into();
        }
        let mut out = String::new();
        for c in char::decode_utf16(v[..(n as usize).min(v.len())].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0)) {
            out.push(c.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        return out;
    } else {
        return "?".into();
    };
    if slice.len() < 2 {
        return "?".into();
    }
    let mut out = String::new();
    for c in char::decode_utf16(slice.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0)) {
        out.push(c.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    out
}

pub fn plan_text() -> String {
    active().map_or("?".into(), |g| name_of(&g))
}

pub fn plans() -> (Vec<String>, Option<usize>) {
    let all = schemes();
    let cur = active().and_then(|c| all.iter().position(|g| same(g, &c)));
    (all.iter().map(name_of).collect(), cur)
}

pub fn set_plan(i: usize) {
    if let Some(g) = schemes().get(i) {
        unsafe { PowerSetActiveScheme(null_mut(), g) };
    }
}

#[derive(Clone, Copy)]
pub enum Timeout {
    Screen,
    Sleep,
}

const VIDEO: GUID = GUID { data1: 0x7516b95f, data2: 0xf776, data3: 0x4464, data4: [0x8c, 0x53, 0x06, 0x16, 0x7f, 0x40, 0xcc, 0x99] };
const VIDEO_IDLE: GUID = GUID { data1: 0x3c0bc021, data2: 0xc8a8, data3: 0x4e07, data4: [0xa9, 0x73, 0x6b, 0x14, 0xcb, 0xcb, 0x2b, 0x7e] };
const SLEEP: GUID = GUID { data1: 0x238c9fa8, data2: 0x0aad, data3: 0x41ed, data4: [0x83, 0xf4, 0x97, 0xbe, 0x24, 0x2c, 0x8f, 0x20] };
const SLEEP_IDLE: GUID = GUID { data1: 0x29f6c1db, data2: 0x86da, data3: 0x48c5, data4: [0x9f, 0xdb, 0xf2, 0xb6, 0x7b, 0x1f, 0x44, 0xda] };
const STEPS: [u32; 10] = [60, 120, 180, 300, 600, 900, 1200, 1800, 3600, 0];

fn ids(t: Timeout) -> (&'static GUID, &'static GUID) {
    match t {
        Timeout::Screen => (&VIDEO, &VIDEO_IDLE),
        Timeout::Sleep => (&SLEEP, &SLEEP_IDLE),
    }
}

pub fn timeout(t: Timeout) -> Option<u32> {
    let (g, (sub, set)) = (active()?, ids(t));
    let mut v = 0u32;
    (unsafe { PowerReadACValueIndex(null_mut(), &g, sub, set, &mut v) } == ERROR_SUCCESS).then_some(v)
}

pub fn timeout_text(t: Timeout) -> String {
    timeout(t).map_or("?".into(), secs_text)
}

fn secs_text(s: u32) -> String {
    match s {
        0 => "Never".into(),
        s if s < 3600 => format!("{} min", s / 60),
        s => format!("{} h", s / 3600),
    }
}

pub fn timeouts(t: Timeout) -> (Vec<String>, Option<usize>) {
    let cur = timeout(t);
    (STEPS.iter().map(|&s| secs_text(s)).collect(), STEPS.iter().position(|&s| Some(s) == cur))
}

pub fn set_timeout(t: Timeout, i: usize) {
    let (Some(g), Some(&v)) = (active(), STEPS.get(i)) else { return };
    let (sub, set) = ids(t);
    unsafe {
        PowerWriteACValueIndex(null_mut(), &g, sub, set, v);
        PowerWriteDCValueIndex(null_mut(), &g, sub, set, v);
        PowerSetActiveScheme(null_mut(), &g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_gb_and_tb() {
        assert_eq!(size(500 << 30), "500.0 GB");
        assert_eq!(size(2 << 40), "2.00 TB");
    }

    #[test]
    fn battery_text_covers_no_battery_and_both_sources() {
        assert_eq!(battery(1, 128, 255), "No battery");
        assert_eq!(battery(0, 0, 73), "73%, on battery");
        assert_eq!(battery(1, 8, 100), "100%, plugged in");
    }

    #[test]
    fn timeout_names() {
        assert_eq!(secs_text(0), "Never");
        assert_eq!(secs_text(300), "5 min");
        assert_eq!(secs_text(3600), "1 h");
        assert_eq!(STEPS.len(), 10);
    }
}
