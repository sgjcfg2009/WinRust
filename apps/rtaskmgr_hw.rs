use std::ptr::null_mut;
use winapi::shared::minwindef::HMODULE;
use winapi::um::{
    fileapi::{CreateFileW, GetDiskFreeSpaceExW, OPEN_EXISTING},
    handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
    ioapiset::DeviceIoControl,
    libloaderapi::{FreeLibrary, GetProcAddress, LoadLibraryW},
    winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE},
};

use crate::ui::wide;

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Dimm {
    pub size_mb: u64,
    pub form: &'static str,
    pub kind: &'static str,
    pub speed: u16,
    pub cfg_speed: u16,
    pub locator: String,
    pub mfr: String,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct MemHw {
    pub dimms: Vec<Dimm>,
    pub slots: u32,
}

impl MemHw {
    pub fn installed_mb(&self) -> u64 {
        self.dimms.iter().map(|d| d.size_mb).sum()
    }
    pub fn kind(&self) -> &'static str {
        self.dimms.first().map_or("", |d| d.kind)
    }
    pub fn speed(&self) -> u16 {
        self.dimms.iter().map(|d| if d.cfg_speed > 0 { d.cfg_speed } else { d.speed }).max().unwrap_or(0)
    }
    pub fn form(&self) -> &'static str {
        self.dimms.first().map_or("", |d| d.form)
    }
}

fn form_name(v: u8) -> &'static str {
    match v {
        0x09 => "DIMM",
        0x0C => "RIMM",
        0x0D => "SODIMM",
        0x0F => "FB-DIMM",
        0x03 => "SIMM",
        0x10 => "Die",
        _ => "—",
    }
}

fn kind_name(v: u8) -> &'static str {
    match v {
        0x13 => "DDR",
        0x14 => "DDR2",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        _ => "",
    }
}

pub fn parse_smbios(data: &[u8]) -> MemHw {
    let mut hw = MemHw::default();
    let mut i = 0;
    while i + 4 <= data.len() {
        let (ty, l) = (data[i], data[i + 1] as usize);
        if l < 4 || i + l > data.len() {
            break;
        }
        let f = &data[i..i + l];
        let start = i + l;
        let mut j = start;
        while j + 1 < data.len() && !(data[j] == 0 && data[j + 1] == 0) {
            j += 1;
        }
        let strings: Vec<&[u8]> = if j > start { data[start..j].split(|&b| b == 0).collect() } else { vec![] };
        let s = |n: u8| -> String { if n == 0 { String::new() } else { strings.get(n as usize - 1).map(|b| String::from_utf8_lossy(b).trim().to_string()).unwrap_or_default() } };
        let u16_at = |o: usize| if l >= o + 2 { u16::from_le_bytes([f[o], f[o + 1]]) } else { 0 };
        match ty {
            16 => hw.slots += u16_at(0x0D) as u32,
            17 => {
                let raw = u16_at(0x0C);
                let size_mb = match raw {
                    0 | 0xFFFF => 0,
                    0x7FFF => if l >= 0x20 { u32::from_le_bytes([f[0x1C], f[0x1D], f[0x1E], f[0x1F]]) as u64 } else { 0 },
                    r if r & 0x8000 != 0 => (r & 0x7FFF) as u64 / 1024,
                    r => r as u64,
                };
                if size_mb > 0 {
                    hw.dimms.push(Dimm {
                        size_mb,
                        form: form_name(f.get(0x0E).copied().unwrap_or(0)),
                        kind: kind_name(f.get(0x12).copied().unwrap_or(0)),
                        speed: u16_at(0x15),
                        cfg_speed: u16_at(0x20),
                        locator: s(f.get(0x10).copied().unwrap_or(0)),
                        mfr: s(f.get(0x17).copied().unwrap_or(0)),
                    });
                }
            }
            127 => break,
            _ => {}
        }
        i = j + 2;
    }
    hw
}

pub fn memory_hw() -> MemHw {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemFirmwareTable(sig: u32, id: u32, buf: *mut u8, size: u32) -> u32;
    }
    let sig = u32::from_be_bytes(*b"RSMB");
    let size = unsafe { GetSystemFirmwareTable(sig, 0, null_mut(), 0) };
    if size < 8 {
        return MemHw::default();
    }
    let mut buf = vec![0u8; size as usize];
    if unsafe { GetSystemFirmwareTable(sig, 0, buf.as_mut_ptr(), size) } == 0 {
        return MemHw::default();
    }
    let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    parse_smbios(&buf[8..(8 + len).min(buf.len())])
}

#[derive(Clone, Default, Debug)]
pub struct DiskInfo {
    pub number: u32,
    pub model: String,
    pub kind: String,
    pub bus: String,
    pub capacity: u64,
    pub formatted: u64,
}

fn open_device(path: &str) -> Option<winapi::um::winnt::HANDLE> {
    let h = unsafe { CreateFileW(wide(path).as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null_mut(), OPEN_EXISTING, 0, null_mut()) };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

fn ioctl(h: winapi::um::winnt::HANDLE, code: u32, input: &[u8], out: &mut [u8]) -> bool {
    let mut ret = 0u32;
    unsafe { DeviceIoControl(h, code, input.as_ptr() as *mut _, input.len() as u32, out.as_mut_ptr() as *mut _, out.len() as u32, &mut ret, null_mut()) != 0 }
}

fn cstr_at(buf: &[u8], off: u32) -> String {
    if off == 0 || off as usize >= buf.len() {
        return String::new();
    }
    let s = &buf[off as usize..];
    let n = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..n]).trim().to_string()
}

pub fn volumes_total(letters: &str) -> u64 {
    use winapi::shared::ntdef::ULARGE_INTEGER;
    let mut sum = 0u64;
    for l in letters.split_whitespace() {
        unsafe {
            let (mut a, mut b, mut c): (ULARGE_INTEGER, ULARGE_INTEGER, ULARGE_INTEGER) = (std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed());
            if GetDiskFreeSpaceExW(wide(&format!(r"{l}\")).as_ptr(), &mut a, &mut b, &mut c) != 0 {
                sum += *b.QuadPart();
            }
        }
    }
    sum
}

#[cfg(test)]
pub fn disk_info() -> DiskInfo {
    let mut number = 0;
    if let Some(v) = open_device(r"\\.\C:") {
        let mut out = [0u8; 64];
        if ioctl(v, 0x0056_0000 , &[], &mut out) {
            number = u32::from_le_bytes([out[8], out[9], out[10], out[11]]);
        }
        unsafe { CloseHandle(v) };
    }
    disk_info_for(number)
}

pub fn disk_info_for(number: u32) -> DiskInfo {
    let mut d = DiskInfo { number, ..Default::default() };
    if let Some(h) = open_device(&format!(r"\\.\PhysicalDrive{}", d.number)) {
        let mut out = [0u8; 1024];
        let query = |prop: u32, buf: &mut [u8; 1024]| -> bool {
            let mut q = [0u8; 12];
            q[..4].copy_from_slice(&prop.to_le_bytes());
            ioctl(h, 0x002D_1400, &q, buf)
        };
        if query(0, &mut out) {
            let off = |o: usize| u32::from_le_bytes([out[o], out[o + 1], out[o + 2], out[o + 3]]);
            let (vendor, product) = (cstr_at(&out, off(12)), cstr_at(&out, off(16)));
            d.model = if vendor.is_empty() || product.to_lowercase().starts_with(&vendor.to_lowercase()) { product } else { format!("{vendor} {product}") };
            d.bus = match off(28) {
                17 => "NVMe",
                11 => "SATA",
                7 => "USB",
                8 => "RAID",
                3 => "ATA",
                1 => "SCSI",
                _ => "",
            }
            .into();
        }
        if query(7, &mut out) {
            d.kind = if out[8] == 0 { "SSD" } else { "HDD" }.into();
        }
        let mut geo = [0u8; 128];
        if ioctl(h, 0x0007_00A0, &[], &mut geo) {
            d.capacity = i64::from_le_bytes([geo[24], geo[25], geo[26], geo[27], geo[28], geo[29], geo[30], geo[31]]).max(0) as u64;
        }
        unsafe { CloseHandle(h) };
    }
    unsafe {
        let mut total: winapi::shared::ntdef::ULARGE_INTEGER = std::mem::zeroed();
        let sys_root = format!("{}\\", std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()));
        if GetDiskFreeSpaceExW(wide(&sys_root).as_ptr(), null_mut(), &mut total, null_mut()) != 0 {
            d.formatted = *total.QuadPart();
        }
    }
    d
}

#[derive(Clone, Default, Debug)]
pub struct GpuReg {
    pub driver_version: String,
    pub vram: u64,
}

pub fn gpu_registry(name: &str) -> GpuReg {
    use winapi::um::winreg::{RegGetValueW, HKEY_LOCAL_MACHINE};
    let class = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let mut buf = [0u16; 256];
    let mut get_str = |sub: &str, val: &str| -> Option<String> {
        let (s, v) = (wide(sub), wide(val));
        let mut size = (buf.len() * 2) as u32;
        let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, s.as_ptr(), v.as_ptr(), 0x2, null_mut(), buf.as_mut_ptr() as *mut _, &mut size) };
        (rc == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    };
    for i in 0..8 {
        let sub = format!(r"{class}\{i:04}");
        if get_str(&sub, "DriverDesc").as_deref() == Some(name) {
            let (s, v) = (wide(&sub), wide("HardwareInformation.qwMemorySize"));
            let (mut q, mut size) = (0u64, 8u32);
            let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, s.as_ptr(), v.as_ptr(), 0xFFFF, null_mut(), &mut q as *mut u64 as *mut _, &mut size) };
            return GpuReg { driver_version: get_str(&sub, "DriverVersion").unwrap_or_default(), vram: if rc == 0 { q } else { 0 } };
        }
    }
    GpuReg::default()
}

pub struct Nvml {
    lib: HMODULE,
    device: usize,
    get_temp: unsafe extern "C" fn(usize, u32, *mut u32) -> i32,
    shutdown: Option<unsafe extern "C" fn() -> i32>,
}

impl Nvml {
    pub fn new() -> Option<Nvml> {
        unsafe {
            let mut lib = LoadLibraryW(wide("nvml.dll").as_ptr());
            if lib.is_null() {
                let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
                let p = format!(r"{pf}\NVIDIA Corporation\NVSMI\nvml.dll");
                lib = LoadLibraryW(wide(&p).as_ptr());
            }
            if lib.is_null() {
                return None;
            }
            let init = GetProcAddress(lib, c"nvmlInit_v2".as_ptr());
            let by_index = GetProcAddress(lib, c"nvmlDeviceGetHandleByIndex_v2".as_ptr());
            let temp = GetProcAddress(lib, c"nvmlDeviceGetTemperature".as_ptr());
            let shut = GetProcAddress(lib, c"nvmlShutdown".as_ptr());
            if init.is_null() || by_index.is_null() || temp.is_null() {
                FreeLibrary(lib);
                return None;
            }
            let init: unsafe extern "C" fn() -> i32 = std::mem::transmute(init);
            let by_index: unsafe extern "C" fn(u32, *mut usize) -> i32 = std::mem::transmute(by_index);
            let shutdown: Option<unsafe extern "C" fn() -> i32> = (!shut.is_null()).then(|| std::mem::transmute(shut));
            if init() != 0 {
                FreeLibrary(lib);
                return None;
            }
            let mut device = 0usize;
            if by_index(0, &mut device) != 0 {
                if let Some(f) = shutdown {
                    f();
                }
                FreeLibrary(lib);
                return None;
            }
            Some(Nvml {
                lib,
                device,
                get_temp: std::mem::transmute::<winapi::shared::minwindef::FARPROC, unsafe extern "C" fn(usize, u32, *mut u32) -> i32>(temp),
                shutdown,
            })
        }
    }

    pub fn temp(&self) -> Option<u32> {
        let mut t = 0u32;
        (unsafe { (self.get_temp)(self.device, 0, &mut t) } == 0).then_some(t)
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe {
            if let Some(f) = self.shutdown {
                f();
            }
            FreeLibrary(self.lib);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<u8> {
        let mut t = vec![];
        let mut a = vec![0u8; 0x17];
        a[0] = 16;
        a[1] = 0x17;
        a[0x0D] = 2;
        t.extend(a);
        t.extend([0, 0]);
        let mut m = vec![0u8; 0x22];
        m[0] = 17;
        m[1] = 0x22;
        m[0x0C] = 0x00;
        m[0x0D] = 0x40;
        m[0x0E] = 0x0D;
        m[0x10] = 1;
        m[0x12] = 0x22;
        m[0x15] = 0xE0;
        m[0x16] = 0x15;
        m[0x17] = 2;
        m[0x20] = 0xE0;
        m[0x21] = 0x15;
        t.extend(m);
        t.extend(b"DIMM A\0Samsung\0\0");
        let mut e = vec![0u8; 0x22];
        e[0] = 17;
        e[1] = 0x22;
        t.extend(e);
        t.extend([0, 0]);
        t.extend([127, 4, 0, 0, 0, 0]);
        t
    }

    #[test]
    fn smbios_parser_reads_modules_slots_and_strings() {
        let hw = parse_smbios(&table());
        assert_eq!(hw.slots, 2);
        assert_eq!(hw.dimms.len(), 1, "пустой слот не считается модулем");
        let d = &hw.dimms[0];
        assert_eq!((d.size_mb, d.form, d.kind, d.speed, d.cfg_speed), (16384, "SODIMM", "DDR5", 5600, 5600));
        assert_eq!((d.locator.as_str(), d.mfr.as_str()), ("DIMM A", "Samsung"));
        assert_eq!(hw.installed_mb(), 16384);
    }

    #[test]
    fn smbios_parser_survives_garbage() {
        assert_eq!(parse_smbios(&[]), MemHw::default());
        assert_eq!(parse_smbios(&[17, 200, 0, 0, 1, 2, 3]), MemHw::default());
    }

    #[test]
    fn real_hardware_is_readable() {
        if std::env::var_os("CI").is_some() {
            return;
        }
        let m = memory_hw();
        assert!(m.installed_mb() >= 1024, "память из SMBIOS: {m:?}");
        assert!(m.slots >= m.dimms.len() as u32);
        let d = disk_info();
        assert!(d.capacity > 10_000_000_000 && d.formatted > 10_000_000_000, "{d:?}");
        assert!(!d.model.is_empty(), "{d:?}");
        assert!(d.kind == "SSD" || d.kind == "HDD", "{d:?}");
        eprintln!("RAM: {:?}\nDISK: {:?}", m, d);
        let g = gpu_registry("");
        eprintln!("GPU reg: {:?}", g);
        if let Some(n) = Nvml::new() {
            let t = n.temp().expect("температура");
            assert!((15..=110).contains(&t), "t={t}");
            eprintln!("GPU temp: {t} °C");
        }
    }
}
