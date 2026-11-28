use std::{collections::HashMap, ptr::null_mut};
use windows_sys::Win32::System::Performance::*;

use crate::ui::wide;

struct Counter(PDH_HCOUNTER);

pub struct Perf {
    query: PDH_HQUERY,
    disk_time: Option<Counter>,
    disk_time_a: Option<Counter>,
    disk_read_a: Option<Counter>,
    disk_write_a: Option<Counter>,
    disk_lat_a: Option<Counter>,
    mem_mod: Option<Counter>,
    mem_sb_res: Option<Counter>,
    mem_sb_norm: Option<Counter>,
    mem_sb_core: Option<Counter>,
    mem_free: Option<Counter>,
    gpu_eng: Option<Counter>,
    gpu_ded: Option<Counter>,
    gpu_shr: Option<Counter>,
    net_rx: Option<Counter>,
    net_tx: Option<Counter>,
    net_bw: Option<Counter>,
    cpu_perf: Option<Counter>,
    buf: Vec<u64>,
    name_buf: String,
}

#[derive(Clone, Default, Debug)]
pub struct NetIf {
    pub name: String,
    pub rx: f64,
    pub tx: f64,
    pub bw: f64,
}

#[derive(Clone, Default, Debug)]
pub struct GpuSample {
    pub types: HashMap<String, f32>,
    pub total: f32,
    pub ded: u64,
    pub shr: u64,
}

#[derive(Clone, Default, Debug)]
pub struct DiskSample {
    pub number: u32,
    pub letters: String,
    pub active: f32,
    pub rd: f64,
    pub wr: f64,
    pub ms: f32,
}

#[derive(Clone, Default)]
pub struct Sample {
    pub gpus: HashMap<(u32, i32), GpuSample>,
    pub disks: Vec<DiskSample>,
    pub nics: Vec<NetIf>,
    pub mem_modified: u64,
    pub mem_standby: u64,
    pub mem_free: u64,
    pub disk_active: f32,
    pub gpu_total: f32,
    pub gpu_by_pid: HashMap<u32, f32>,
    pub gpu_ded: u64,
    pub gpu_shr: u64,
    pub net: Option<NetIf>,
    pub net_util: f32,
    pub cpu_perf: f32,
}

pub fn parse_luid(name: &str) -> Option<(u32, i32)> {
    let rest = name.split("luid_0x").nth(1)?;
    let mut it = rest.split('_');
    let high = u32::from_str_radix(it.next()?, 16).ok()?;
    let low = u32::from_str_radix(it.next()?.trim_start_matches("0x"), 16).ok()?;
    Some((low, high as i32))
}

impl Perf {
    pub fn new() -> Option<Perf> {
        unsafe {
            let mut query: PDH_HQUERY = null_mut();
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let add = |path: &str| -> Option<Counter> {
                let mut c: PDH_HCOUNTER = null_mut();
                (PdhAddEnglishCounterW(query, wide(path).as_ptr(), 0, &mut c) == 0).then_some(Counter(c))
            };
            let p = Perf {
                query,
                disk_time: add(r"\PhysicalDisk(_Total)\% Disk Time"),
                disk_time_a: add(r"\PhysicalDisk(*)\% Disk Time"),
                disk_read_a: add(r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
                disk_write_a: add(r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
                disk_lat_a: add(r"\PhysicalDisk(*)\Avg. Disk sec/Transfer"),
                mem_mod: add(r"\Memory\Modified Page List Bytes"),
                mem_sb_res: add(r"\Memory\Standby Cache Reserve Bytes"),
                mem_sb_norm: add(r"\Memory\Standby Cache Normal Priority Bytes"),
                mem_sb_core: add(r"\Memory\Standby Cache Core Bytes"),
                mem_free: add(r"\Memory\Free & Zero Page List Bytes"),
                gpu_eng: add(r"\GPU Engine(*)\Utilization Percentage"),
                gpu_ded: add(r"\GPU Adapter Memory(*)\Dedicated Usage"),
                gpu_shr: add(r"\GPU Adapter Memory(*)\Shared Usage"),
                net_rx: add(r"\Network Interface(*)\Bytes Received/sec"),
                net_tx: add(r"\Network Interface(*)\Bytes Sent/sec"),
                net_bw: add(r"\Network Interface(*)\Current Bandwidth"),
                cpu_perf: add(r"\Processor Information(_Total)\% Processor Performance"),
                buf: Vec::new(),
                name_buf: String::new(),
            };
            PdhCollectQueryData(query);
            Some(p)
        }
    }

    fn value(c: &Option<Counter>) -> f64 {
        let Some(c) = c else { return 0.0 };
        unsafe {
            let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
            if PdhGetFormattedCounterValue(c.0, PDH_FMT_DOUBLE, null_mut(), &mut v) == 0 && v.CStatus == 0 {
                v.Anonymous.doubleValue
            } else {
                0.0
            }
        }
    }

    fn for_each_array(c: &Option<Counter>, buf: &mut Vec<u64>, name_buf: &mut String, mut f: impl FnMut(&str, f64)) {
        let Some(c) = c else { return };
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            let rc = PdhGetFormattedCounterArrayW(c.0, PDH_FMT_DOUBLE, &mut size, &mut count, null_mut());
            if rc != PDH_MORE_DATA || size == 0 {
                return;
            }
            let need = size as usize / 8 + 8;
            if buf.len() < need {
                buf.resize(need, 0);
            }
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c.0, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return;
            }
            for i in 0..count as usize {
                let it = &*items.add(i);
                if it.szName.is_null() || it.FmtValue.CStatus != 0 {
                    continue;
                }
                let mut n = 0;
                while *it.szName.add(n) != 0 {
                    n += 1;
                }
                name_buf.clear();
                for ch in std::char::decode_utf16(std::slice::from_raw_parts(it.szName, n).iter().copied()) {
                    name_buf.push(ch.unwrap_or('\u{FFFD}'));
                }
                f(name_buf, it.FmtValue.Anonymous.doubleValue);
            }
        }
    }

    pub fn sample(&mut self) -> Sample {
        unsafe { PdhCollectQueryData(self.query) };
        let mut s = Sample {
            disk_active: Self::value(&self.disk_time).clamp(0.0, 100.0) as f32,
            cpu_perf: Self::value(&self.cpu_perf) as f32,
            ..Default::default()
        };
        let (mut by_type, mut by_pid_type, mut per_type): (HashMap<String, f64>, HashMap<(u32, String), f64>, HashMap<((u32, i32), String), f64>) = Default::default();
        Self::for_each_array(&self.gpu_eng, &mut self.buf, &mut self.name_buf, |name, v| {
            let pid = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok());
            let ty = name.rsplit("engtype_").next().unwrap_or("");
            if let Some(cur) = by_type.get_mut(ty) {
                *cur += v;
            } else {
                by_type.insert(ty.to_string(), v);
            }
            if let Some(pid) = pid {
                *by_pid_type.entry((pid, ty.to_string())).or_default() += v;
            }
            if let Some(l) = parse_luid(name) {
                *per_type.entry((l, ty.to_string())).or_default() += v;
            }
        });
        s.gpu_total = by_type.values().cloned().fold(0.0, f64::max).min(100.0) as f32;
        for ((pid, _), v) in by_pid_type {
            let e = s.gpu_by_pid.entry(pid).or_insert(0.0);
            *e = e.max(v.min(100.0) as f32);
        }
        Self::for_each_array(&self.gpu_ded, &mut self.buf, &mut self.name_buf, |name, v| {
            s.gpu_ded += v as u64;
            if let Some(l) = parse_luid(name) {
                s.gpus.entry(l).or_default().ded += v as u64;
            }
        });
        Self::for_each_array(&self.gpu_shr, &mut self.buf, &mut self.name_buf, |name, v| {
            s.gpu_shr += v as u64;
            if let Some(l) = parse_luid(name) {
                s.gpus.entry(l).or_default().shr += v as u64;
            }
        });

        for ((l, ty), v) in per_type {
            let g = s.gpus.entry(l).or_default();
            g.types.insert(ty, v.min(100.0) as f32);
            g.total = g.total.max(v.min(100.0) as f32);
        }

        let (mut rd, mut wr, mut lat): (HashMap<String, f64>, HashMap<String, f64>, HashMap<String, f64>) = Default::default();
        Self::for_each_array(&self.disk_read_a, &mut self.buf, &mut self.name_buf, |name, v| {
            rd.insert(name.to_string(), v);
        });
        Self::for_each_array(&self.disk_write_a, &mut self.buf, &mut self.name_buf, |name, v| {
            wr.insert(name.to_string(), v);
        });
        Self::for_each_array(&self.disk_lat_a, &mut self.buf, &mut self.name_buf, |name, v| {
            lat.insert(name.to_string(), v);
        });
        Self::for_each_array(&self.disk_time_a, &mut self.buf, &mut self.name_buf, |name, a| {
            if name == "_Total" {
                return;
            }
            let (num, letters) = name.split_once(' ').unwrap_or((name, ""));
            if let Ok(number) = num.parse::<u32>() {
                s.disks.push(DiskSample {
                    number,
                    letters: letters.to_string(),
                    active: a.clamp(0.0, 100.0) as f32,
                    rd: rd.get(name).copied().unwrap_or(0.0),
                    wr: wr.get(name).copied().unwrap_or(0.0),
                    ms: (lat.get(name).copied().unwrap_or(0.0) * 1000.0) as f32,
                });
            }
        });
        s.disks.sort_by_key(|d| d.number);

        s.mem_modified = Self::value(&self.mem_mod) as u64;
        s.mem_standby = (Self::value(&self.mem_sb_res) + Self::value(&self.mem_sb_norm) + Self::value(&self.mem_sb_core)) as u64;
        s.mem_free = Self::value(&self.mem_free) as u64;

        let (mut tx, mut bw): (HashMap<String, f64>, HashMap<String, f64>) = Default::default();
        Self::for_each_array(&self.net_tx, &mut self.buf, &mut self.name_buf, |name, v| {
            tx.insert(name.to_string(), v);
        });
        Self::for_each_array(&self.net_bw, &mut self.buf, &mut self.name_buf, |name, v| {
            bw.insert(name.to_string(), v);
        });
        let bad = |n: &str| {
            let l = n.to_lowercase();
            l.contains("loopback") || l.contains("isatap") || l.contains("teredo") || l.contains("pseudo") || l.contains("6to4")
        };
        let mut best: Option<NetIf> = None;
        Self::for_each_array(&self.net_rx, &mut self.buf, &mut self.name_buf, |name, r| {
            if bad(name) {
                return;
            }
            let cand = NetIf { rx: r, tx: tx.get(name).copied().unwrap_or(0.0), bw: bw.get(name).copied().unwrap_or(0.0), name: name.to_string() };
            if cand.bw > 0.0 {
                s.nics.push(cand.clone());
            }
            let better = best.as_ref().is_none_or(|b| (cand.rx + cand.tx, cand.bw) > (b.rx + b.tx, b.bw));
            if better && cand.bw > 0.0 {
                best = Some(cand);
            }
        });
        if let Some(b) = &best {
            s.net_util = ((b.rx + b.tx) * 8.0 / b.bw * 100.0).clamp(0.0, 100.0) as f32;
        }
        s.net = best;
        s
    }
}

impl Drop for Perf {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

#[derive(Clone, Default)]
pub struct SysInfo {
    pub base_mhz: u32,
    pub l3: u64,
    pub virt: bool,
}

fn reg_dword(sub: &str, name: &str) -> Option<u32> {
    use winapi::um::winreg::{RegGetValueW, HKEY_LOCAL_MACHINE};
    let (s, n) = (wide(sub), wide(name));
    let (mut v, mut size) = (0u32, 4u32);
    let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, s.as_ptr(), n.as_ptr(), 0x10 , null_mut(), &mut v as *mut u32 as *mut _, &mut size) };
    (rc == 0).then_some(v)
}

fn l3_cache() -> u64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalProcessorInformation(buf: *mut u8, len: *mut u32) -> i32;
    }
    let mut len = 0u32;
    unsafe { GetLogicalProcessorInformation(null_mut(), &mut len) };
    if len == 0 {
        return 0;
    }
    let mut buf = vec![0u8; len as usize];
    if unsafe { GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut len) } == 0 {
        return 0;
    }
    let mut l3 = 0u64;
    for e in buf.as_chunks::<32>().0 {
        if u32::from_le_bytes([e[8], e[9], e[10], e[11]]) == 2 && e[16] == 3 {
            l3 += u32::from_le_bytes([e[20], e[21], e[22], e[23]]) as u64;
        }
    }
    l3
}

fn base_mhz() -> u32 {
    #[link(name = "powrprof")]
    unsafe extern "system" {
        fn CallNtPowerInformation(level: i32, input: *const u8, in_len: u32, output: *mut u8, out_len: u32) -> i32;
    }
    let mut buf = vec![0u8; 24 * 64];
    let rc = unsafe { CallNtPowerInformation(11 , std::ptr::null(), 0, buf.as_mut_ptr(), buf.len() as u32) };
    if rc == 0 {
        let max = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if max > 0 {
            return max;
        }
    }
    reg_dword(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "~MHz").unwrap_or(0)
}

pub fn static_info() -> SysInfo {
    SysInfo {
        base_mhz: base_mhz(),
        l3: l3_cache(),
        virt: unsafe { winapi::um::processthreadsapi::IsProcessorFeaturePresent(21) } != 0 || (std::arch::x86_64::__cpuid(1).ecx >> 31) & 1 == 1,
    }
}

#[derive(Clone, Copy, Default)]
pub struct MemInfo {
    pub commit_total: u64,
    pub commit_limit: u64,
    pub cache: u64,
    pub processes: u32,
}

pub fn mem_info() -> MemInfo {
    use winapi::um::psapi::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
    let mut p: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetPerformanceInfo(&mut p, std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32) };
    if ok == 0 {
        return MemInfo::default();
    }
    let page = p.PageSize as u64;
    MemInfo {
        commit_total: p.CommitTotal as u64 * page,
        commit_limit: p.CommitLimit as u64 * page,
        cache: p.SystemCache as u64 * page,
        processes: p.ProcessCount,
    }
}

pub fn uptime_secs() -> u64 {
    unsafe { winapi::um::sysinfoapi::GetTickCount64() / 1000 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luid_is_parsed_from_counter_instance_names() {
        assert_eq!(parse_luid("pid_4_luid_0x00000000_0x0000E4F8_phys_0_eng_0_engtype_3D"), Some((0xE4F8, 0)));
        assert_eq!(parse_luid("luid_0x00000001_0x0000F4C0_phys_0"), Some((0xF4C0, 1)));
        assert_eq!(parse_luid("no luid here"), None);
    }

    #[test]
    fn pdh_counters_work_and_system_info_is_sane() {
        let mut p = Perf::new().expect("PDH available");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let s = p.sample();
        assert!((0.0..=100.0).contains(&s.disk_active));
        assert!((0.0..=100.0).contains(&s.gpu_total));
        let i = static_info();
        assert!(i.base_mhz > 500 && i.l3 > 0, "base={} l3={}", i.base_mhz, i.l3);
        let m = mem_info();
        assert!(m.processes > 10 && m.commit_limit > 0);
        assert!(uptime_secs() > 10);
        assert!(!s.disks.is_empty(), "PhysicalDisk(*) should return at least one disk");
        assert!(s.disks.iter().any(|d| d.letters.contains("C:")), "{:?}", s.disks);
        assert!(s.mem_free > 0 && s.mem_standby > 0, "mem: free={} standby={}", s.mem_free, s.mem_standby);
    }
}
