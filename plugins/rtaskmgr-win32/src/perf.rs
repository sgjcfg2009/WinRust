use std::{collections::HashMap, ptr::null_mut};
use windows_sys::Win32::System::Performance::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Counter(PDH_HCOUNTER);

pub struct Perf {
    query: PDH_HQUERY,
    disk_time: Option<Counter>,
    disk_read: Option<Counter>,
    disk_write: Option<Counter>,
    disk_lat: Option<Counter>,
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
    pub disk_read: f64,
    pub disk_write: f64,
    pub disk_ms: f32,
    pub gpu_total: f32,
    pub gpu_types: HashMap<String, f32>,
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
                disk_read: add(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec"),
                disk_write: add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec"),
                disk_lat: add(r"\PhysicalDisk(_Total)\Avg. Disk sec/Transfer"),
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

    fn array(c: &Option<Counter>) -> Vec<(String, f64)> {
        let Some(c) = c else { return vec![] };
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            let rc = PdhGetFormattedCounterArrayW(c.0, PDH_FMT_DOUBLE, &mut size, &mut count, null_mut());
            if rc != PDH_MORE_DATA || size == 0 {
                return vec![];
            }
            let mut buf = vec![0u64; size as usize / 8 + 8];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c.0, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return vec![];
            }
            (0..count as usize)
                .filter_map(|i| {
                    let it = &*items.add(i);
                    if it.szName.is_null() || it.FmtValue.CStatus != 0 {
                        return None;
                    }
                    let mut n = 0;
                    while *it.szName.add(n) != 0 {
                        n += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, n));
                    Some((name, it.FmtValue.Anonymous.doubleValue))
                })
                .collect()
        }
    }

    pub fn sample(&mut self) -> Sample {
        unsafe { PdhCollectQueryData(self.query) };
        let mut s = Sample {
            disk_active: Self::value(&self.disk_time).clamp(0.0, 100.0) as f32,
            disk_read: Self::value(&self.disk_read),
            disk_write: Self::value(&self.disk_write),
            disk_ms: (Self::value(&self.disk_lat) * 1000.0) as f32,
            cpu_perf: Self::value(&self.cpu_perf) as f32,
            ..Default::default()
        };
        let (mut by_type, mut by_pid_type, mut per_type): (HashMap<String, f64>, HashMap<(u32, String), f64>, HashMap<((u32, i32), String), f64>) = Default::default();
        for (name, v) in Self::array(&self.gpu_eng) {
            let pid = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok());
            let ty = name.rsplit("engtype_").next().unwrap_or("").to_string();
            *by_type.entry(ty.clone()).or_default() += v;
            if let Some(pid) = pid {
                *by_pid_type.entry((pid, ty.clone())).or_default() += v;
            }
            if let Some(l) = parse_luid(&name) {
                *per_type.entry((l, ty)).or_default() += v;
            }
        }
        s.gpu_total = by_type.values().cloned().fold(0.0, f64::max).min(100.0) as f32;
        s.gpu_types = by_type.iter().map(|(k, v)| (k.clone(), v.min(100.0) as f32)).collect();
        for ((pid, _), v) in by_pid_type {
            let e = s.gpu_by_pid.entry(pid).or_insert(0.0);
            *e = e.max(v.min(100.0) as f32);
        }
        s.gpu_ded = Self::array(&self.gpu_ded).iter().map(|x| x.1).sum::<f64>() as u64;
        s.gpu_shr = Self::array(&self.gpu_shr).iter().map(|x| x.1).sum::<f64>() as u64;

        for ((l, ty), v) in per_type {
            let g = s.gpus.entry(l).or_default();
            g.types.insert(ty, v.min(100.0) as f32);
            g.total = g.total.max(v.min(100.0) as f32);
        }
        for (name, v) in Self::array(&self.gpu_ded) {
            if let Some(l) = parse_luid(&name) {
                s.gpus.entry(l).or_default().ded += v as u64;
            }
        }
        for (name, v) in Self::array(&self.gpu_shr) {
            if let Some(l) = parse_luid(&name) {
                s.gpus.entry(l).or_default().shr += v as u64;
            }
        }

        let (act, rd, wr, lat) = (Self::array(&self.disk_time_a), Self::array(&self.disk_read_a), Self::array(&self.disk_write_a), Self::array(&self.disk_lat_a));
        let (rd, wr, lat): (HashMap<String, f64>, HashMap<String, f64>, HashMap<String, f64>) = (rd.into_iter().collect(), wr.into_iter().collect(), lat.into_iter().collect());
        for (name, a) in act {
            if name == "_Total" {
                continue;
            }
            let (num, letters) = name.split_once(' ').unwrap_or((&name, ""));
            if let Ok(number) = num.parse::<u32>() {
                s.disks.push(DiskSample {
                    number,
                    letters: letters.to_string(),
                    active: a.clamp(0.0, 100.0) as f32,
                    rd: rd.get(&name).copied().unwrap_or(0.0),
                    wr: wr.get(&name).copied().unwrap_or(0.0),
                    ms: (lat.get(&name).copied().unwrap_or(0.0) * 1000.0) as f32,
                });
            }
        }
        s.disks.sort_by_key(|d| d.number);

        s.mem_modified = Self::value(&self.mem_mod) as u64;
        s.mem_standby = (Self::value(&self.mem_sb_res) + Self::value(&self.mem_sb_norm) + Self::value(&self.mem_sb_core)) as u64;
        s.mem_free = Self::value(&self.mem_free) as u64;

        let (rx, tx, bw) = (Self::array(&self.net_rx), Self::array(&self.net_tx), Self::array(&self.net_bw));
        let tx: HashMap<String, f64> = tx.into_iter().collect();
        let bw: HashMap<String, f64> = bw.into_iter().collect();
        let bad = |n: &str| {
            let l = n.to_lowercase();
            l.contains("loopback") || l.contains("isatap") || l.contains("teredo") || l.contains("pseudo") || l.contains("6to4")
        };
        let mut best: Option<NetIf> = None;
        for (name, r) in rx.into_iter().filter(|(n, _)| !bad(n)) {
            let cand = NetIf { rx: r, tx: tx.get(&name).copied().unwrap_or(0.0), bw: bw.get(&name).copied().unwrap_or(0.0), name };
            if cand.bw > 0.0 {
                s.nics.push(cand.clone());
            }
            let better = best.as_ref().is_none_or(|b| (cand.rx + cand.tx, cand.bw) > (b.rx + b.tx, b.bw));
            if better && cand.bw > 0.0 {
                best = Some(cand);
            }
        }
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
    pub sockets: u32,
    pub l1: u64,
    pub l2: u64,
    pub l3: u64,
    pub virt: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub gpu_name: String,
}

fn reg_string(sub: &str, name: &str) -> Option<String> {
    use winapi::um::winreg::{RegGetValueW, HKEY_LOCAL_MACHINE};
    let (s, n) = (wide(sub), wide(name));
    let mut buf = vec![0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, s.as_ptr(), n.as_ptr(), 0x2 , null_mut(), buf.as_mut_ptr() as *mut _, &mut size) };
    (rc == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
}

fn reg_dword(sub: &str, name: &str) -> Option<u32> {
    use winapi::um::winreg::{RegGetValueW, HKEY_LOCAL_MACHINE};
    let (s, n) = (wide(sub), wide(name));
    let (mut v, mut size) = (0u32, 4u32);
    let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, s.as_ptr(), n.as_ptr(), 0x10 , null_mut(), &mut v as *mut u32 as *mut _, &mut size) };
    (rc == 0).then_some(v)
}

fn processor_info() -> (u32, [u64; 3]) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalProcessorInformation(buf: *mut u8, len: *mut u32) -> i32;
    }
    let mut len = 0u32;
    unsafe { GetLogicalProcessorInformation(null_mut(), &mut len) };
    if len == 0 {
        return (1, [0; 3]);
    }
    let mut buf = vec![0u8; len as usize];
    if unsafe { GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut len) } == 0 {
        return (1, [0; 3]);
    }
    let (mut sockets, mut cache) = (0u32, [0u64; 3]);
    for e in buf.as_chunks::<32>().0 {
        match u32::from_le_bytes([e[8], e[9], e[10], e[11]]) {
            3 => sockets += 1,
            2 => {
                let (level, size) = (e[16], u32::from_le_bytes([e[20], e[21], e[22], e[23]]) as u64);
                if (1..=3).contains(&level) {
                    cache[level as usize - 1] += size;
                }
            }
            _ => {}
        }
    }
    (sockets.max(1), cache)
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
    let (sockets, cache) = processor_info();
    let class = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let mut gpus: Vec<String> = (0..8).filter_map(|i| reg_string(&format!(r"{class}\{i:04}"), "DriverDesc")).filter(|n| !n.to_lowercase().contains("basic")).collect();
    gpus.sort_by_key(|n| {
        let l = n.to_lowercase();
        !(l.contains("nvidia") || l.contains("radeon") || l.contains("geforce"))
    });
    SysInfo {
        base_mhz: base_mhz(),
        sockets,
        l1: cache[0],
        l2: cache[1],
        l3: cache[2],
        virt: unsafe { winapi::um::processthreadsapi::IsProcessorFeaturePresent(21) } != 0 || (std::arch::x86_64::__cpuid(1).ecx >> 31) & 1 == 1,
        gpu_name: gpus.into_iter().next().unwrap_or_default(),
    }
}

#[derive(Clone, Copy, Default)]
pub struct MemInfo {
    pub commit_total: u64,
    pub commit_limit: u64,
    pub cache: u64,
    pub paged: u64,
    pub nonpaged: u64,
    pub processes: u32,
    pub threads: u32,
    pub handles: u32,
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
        paged: p.KernelPaged as u64 * page,
        nonpaged: p.KernelNonpaged as u64 * page,
        processes: p.ProcessCount,
        threads: p.ThreadCount,
        handles: p.HandleCount,
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
        let mut p = Perf::new().expect("PDH доступен");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let s = p.sample();
        assert!((0.0..=100.0).contains(&s.disk_active));
        assert!((0.0..=100.0).contains(&s.gpu_total));
        let i = static_info();
        assert!(i.base_mhz > 500 && i.sockets >= 1 && i.l2 > 0, "base={} sockets={} l2={}", i.base_mhz, i.sockets, i.l2);
        let m = mem_info();
        assert!(m.processes > 10 && m.threads > m.processes && m.commit_limit > 0);
        assert!(uptime_secs() > 10);
        assert!(!s.disks.is_empty(), "PhysicalDisk(*) должен вернуть хотя бы один диск");
        assert!(s.disks.iter().any(|d| d.letters.contains("C:")), "{:?}", s.disks);
        assert!(s.mem_free > 0 && s.mem_standby > 0, "состав памяти: free={} standby={}", s.mem_free, s.mem_standby);
        eprintln!("disks={:?}\nnics={:?}\ngpus={:?}\nmem mod={}MB standby={}MB free={}MB", s.disks, s.nics.iter().map(|n| &n.name).collect::<Vec<_>>(), s.gpus, s.mem_modified >> 20, s.mem_standby >> 20, s.mem_free >> 20);
        eprintln!("base={}MHz perf={:.0}% sockets={} L1={}K L2={}K L3={}K virt={} gpu='{}'", i.base_mhz, s.cpu_perf, i.sockets, i.l1 / 1024, i.l2 / 1024, i.l3 / 1024, i.virt, i.gpu_name);
        eprintln!("disk active={:.0}% rd={:.0} wr={:.0} lat={:.2}ms | gpu={:.0}% ded={}MB shr={}MB pids={} | net={:?} util={:.2}%", s.disk_active, s.disk_read, s.disk_write, s.disk_ms, s.gpu_total, s.gpu_ded >> 20, s.gpu_shr >> 20, s.gpu_by_pid.len(), s.net, s.net_util);
        eprintln!("commit={}/{}MB cache={}MB paged={}MB nonpaged={}MB procs={} threads={} handles={}", m.commit_total >> 20, m.commit_limit >> 20, m.cache >> 20, m.paged >> 20, m.nonpaged >> 20, m.processes, m.threads, m.handles);
    }
}
