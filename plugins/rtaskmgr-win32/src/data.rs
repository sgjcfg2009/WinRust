use crate::{gpu, hw, perf, net, res::ResCache, startup, store, sys};
use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};
use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub const CAP: usize = 60;
const EMA: f32 = 2.0 / 31.0;
pub const CRITICAL: [&str; 8] = ["system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe"];

#[derive(Clone)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub exe: Option<String>,
    pub cpu: f32,
    pub avg: f32,
    pub mem: u64,
    pub disk: f64,
    pub net: f64,
    pub gpu: f32,
    pub title: Option<String>,
    pub user: String,
}

#[derive(Default)]
pub struct Series {
    pub cpu: VecDeque<f32>,
    pub mem: VecDeque<f32>,
    pub disk: VecDeque<f32>,
    pub rd: VecDeque<f32>,
    pub wr: VecDeque<f32>,
    pub rx: VecDeque<f32>,
    pub tx: VecDeque<f32>,
    pub gpu: VecDeque<f32>,
    pub cores: Vec<VecDeque<f32>>,
}

pub fn push(q: &mut VecDeque<f32>, v: f32) {
    if q.len() == CAP {
        q.pop_front();
    }
    q.push_back(v);
}

#[derive(Default)]
pub struct GpuSeries {
    pub total: VecDeque<f32>,
    pub g3d: VecDeque<f32>,
    pub gcopy: VecDeque<f32>,
    pub genc: VecDeque<f32>,
    pub gdec: VecDeque<f32>,
    pub ded: VecDeque<f32>,
    pub shr: VecDeque<f32>,
}

pub struct GpuState {
    pub adapter: gpu::Adapter,
    pub reg: hw::GpuReg,
    pub util: f32,
    pub ded_used: u64,
    pub shr_used: u64,
    pub temp: Option<f32>,
    pub series: GpuSeries,
}

pub struct DiskState {
    pub number: u32,
    pub letters: String,
    pub info: hw::DiskInfo,
    pub formatted: u64,
    pub active: f32,
    pub rd: f64,
    pub wr: f64,
    pub ms: f32,
    pub s_active: VecDeque<f32>,
    pub s_rd: VecDeque<f32>,
    pub s_wr: VecDeque<f32>,
}

pub struct NicState {
    pub name: String,
    pub rx: f64,
    pub tx: f64,
    pub bw: f64,
    pub s_rx: VecDeque<f32>,
    pub s_tx: VecDeque<f32>,
}

pub struct Data {
    pub sys: System,
    nets: Networks,
    pub procs: Vec<Proc>,
    ema: HashMap<u32, f32>,
    pub svcs: Vec<sys::Svc>,
    pub svc_pids: HashMap<u32, Vec<String>>,
    pub startup: Vec<startup::Item>,
    pub series: Series,
    pub cfg: store::Settings,
    pub net: HashMap<u32, f64>,
    pub etw: Option<net::Session>,
    pub perf: Option<perf::Perf>,
    pub sample: perf::Sample,
    pub sysinfo: perf::SysInfo,
    pub meminfo: perf::MemInfo,
    pub res: ResCache,
    pub cpu_total: f32,
    pub mem_pct: f32,
    pub ticks: u32,
    prev: Instant,
    pub is_admin: bool,
    pub efficiency: std::collections::HashSet<u32>,
    pub suspended: std::collections::HashSet<u32>,
    pub disk_total: u64,
    pub hw_mem: hw::MemHw,
    pub gpus: Vec<GpuState>,
    pub disks: Vec<DiskState>,
    pub nics: Vec<NicState>,
    nvml: Option<hw::Nvml>,
    nvml_tried: bool,
    user_cache: HashMap<u32, String>,
    pub want_gpu_temp: bool,
}

pub fn group_key(name: &str) -> String {
    name.to_lowercase()
}

impl Data {
    pub fn new(cfg: store::Settings) -> Data {
        let is_admin = sys::is_admin();
        let mut d = Data::new_inner(cfg, is_admin);
        d.gpus = gpu::adapters()
            .into_iter()
            .map(|a| GpuState { reg: hw::gpu_registry(&a.name), adapter: a, util: 0.0, ded_used: 0, shr_used: 0, temp: None, series: GpuSeries::default() })
            .collect();
        d
    }

    fn new_inner(cfg: store::Settings, is_admin: bool) -> Data {
        Data {
            sys: System::new().expect("sysinfo"),
            nets: Networks::new_with_refreshed_list().expect("сетевые интерфейсы"),
            procs: vec![],
            ema: HashMap::new(),
            svcs: vec![],
            svc_pids: HashMap::new(),
            startup: vec![],
            series: Series::default(),
            etw: if is_admin && cfg.etw { net::start() } else { None },
            cfg,
            net: HashMap::new(),
            perf: perf::Perf::new(),
            sample: perf::Sample::default(),
            sysinfo: perf::static_info(),
            meminfo: perf::MemInfo::default(),
            res: ResCache::new(),
            cpu_total: 0.0,
            mem_pct: 0.0,
            ticks: 0,
            prev: Instant::now(),
            is_admin,
            efficiency: Default::default(),
            suspended: Default::default(),
            disk_total: 0,
            hw_mem: hw::memory_hw(),
            gpus: vec![],
            disks: vec![],
            nics: vec![],
            nvml: None,
            nvml_tried: false,
            user_cache: HashMap::new(),
            want_gpu_temp: false,
        }
    }

    pub fn load_startup(&mut self) {
        self.startup = startup::list();
    }

    pub fn refresh(&mut self) -> Vec<String> {
        let dt = self.prev.elapsed().as_secs_f64().clamp(0.3, 5.0);
        self.prev = Instant::now();
        self.ticks += 1;
        let first = self.ticks == 1;
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let kind = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet);
        self.sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

        self.cpu_total = if first { 0.0 } else { self.sys.global_cpu_usage() };
        let (used, total) = (self.sys.used_memory(), self.sys.total_memory().max(1));
        self.mem_pct = (100.0 * used as f64 / total as f64) as f32;
        let cores = self.sys.cpus().len().max(1) as f32;
        let cur: HashMap<u32, f32> = self.sys.processes().values().map(|p| (p.pid().as_u32(), p.cpu_usage() / cores)).collect();
        self.ema.retain(|k, _| cur.contains_key(k));
        for (k, v) in &cur {
            let e = self.ema.entry(*k).or_insert(*v);
            *e += EMA * (*v - *e);
        }
        if self.etw.is_some() {
            self.net = net::drain().into_iter().map(|(pid, (rx, tx))| (pid, (rx + tx) as f64 / dt)).collect();
        }
        self.sample = self.perf.as_mut().map(|p| p.sample()).unwrap_or_default();
        self.meminfo = perf::mem_info();
        if self.want_gpu_temp || self.cfg.gpu_temp_always {
            for g in self.gpus.iter_mut() {
                g.temp = gpu::temperature(g.adapter.luid).or_else(|| {
                    if g.adapter.vendor == 0x10DE {
                        if !self.nvml_tried {
                            self.nvml_tried = true;
                            self.nvml = hw::Nvml::new();
                        }
                        self.nvml.as_ref().and_then(|n| n.temp()).map(|t| t as f32)
                    } else {
                        None
                    }
                });
            }
        } else {
            for g in self.gpus.iter_mut() {
                g.temp = None;
            }
        }
        if self.ticks % 30 == 1 {
            self.disk_total = disk_capacity();
        }
        if self.ticks % 15 == 1 {
            self.svcs = sys::services();
            self.svc_pids.clear();
            for s in self.svcs.iter().filter(|s| s.pid != 0) {
                self.svc_pids.entry(s.pid).or_default().push(s.name.clone());
            }
        }
        let titles = if self.ticks % 2 == 1 { Some(sys::window_titles()) } else { None };
        let old_titles: HashMap<u32, String> = if titles.is_none() {
            self.procs.iter().filter_map(|p| p.title.clone().map(|t| (p.pid, t))).collect()
        } else {
            HashMap::new()
        };
        let user_cache = &mut self.user_cache;
        let procs: Vec<Proc> = self
            .sys
            .processes()
            .values()
            .map(|p| {
                let pid = p.pid().as_u32();
                let d = p.disk_usage();
                Proc {
                    pid,
                    ppid: p.parent().map_or(0, |x| x.as_u32()),
                    name: p.name().to_string_lossy().into_owned(),
                    exe: p.exe().map(|e| e.display().to_string()).filter(|e| !e.is_empty()),
                    cpu: cur.get(&pid).copied().unwrap_or(0.0),
                    avg: self.ema.get(&pid).copied().unwrap_or(0.0),
                    mem: p.memory(),
                    disk: if first { 0.0 } else { (d.read_bytes + d.written_bytes) as f64 / dt },
                    net: self.net.get(&pid).copied().unwrap_or(0.0),
                    gpu: self.sample.gpu_by_pid.get(&pid).copied().unwrap_or(0.0),
                    user: user_cache.entry(pid).or_insert_with(|| sys::process_user(pid).unwrap_or_else(|| "SYSTEM".into())).clone(),
                    title: match &titles {
                        Some(t) => t.get(&pid).cloned(),
                        None => old_titles.get(&pid).cloned(),
                    },
                }
            })
            .collect();
        self.procs = procs;
        self.user_cache.retain(|k, _| cur.contains_key(k));
        self.efficiency.retain(|p| cur.contains_key(p));
        self.suspended.retain(|p| cur.contains_key(p));

        let (rx, tx) = self.sample.net.as_ref().map_or_else(
            || {
                self.nets.refresh(false);
                let r = self.nets.iter().fold((0u64, 0u64), |a, (_, n)| (a.0 + n.received(), a.1 + n.transmitted()));
                (r.0 as f64 / dt, r.1 as f64 / dt)
            },
            |n| (n.rx, n.tx),
        );
        if !first {
            let s = &mut self.series;
            push(&mut s.cpu, self.cpu_total);
            push(&mut s.mem, self.mem_pct);
            push(&mut s.disk, self.sample.disk_active);
            push(&mut s.rd, (self.sample.disk_read / 1_048_576.0) as f32);
            push(&mut s.wr, (self.sample.disk_write / 1_048_576.0) as f32);
            push(&mut s.rx, (rx * 8.0 / 1000.0) as f32);
            push(&mut s.tx, (tx * 8.0 / 1000.0) as f32);
            push(&mut s.gpu, self.sample.gpu_total);
            s.cores.resize_with(self.sys.cpus().len(), VecDeque::new);
            for (q, c) in s.cores.iter_mut().zip(self.sys.cpus()) {
                push(q, c.usage());
            }
        }

        if !first {
            for g in self.gpus.iter_mut() {
                let smp = self.sample.gpus.get(&g.adapter.luid).cloned().unwrap_or_default();
                let ty = |k: &str| smp.types.get(k).copied().unwrap_or(0.0);
                g.util = smp.total;
                g.ded_used = smp.ded;
                g.shr_used = smp.shr;
                let s = &mut g.series;
                push(&mut s.total, smp.total);
                push(&mut s.g3d, ty("3D"));
                push(&mut s.gcopy, ty("Copy"));
                push(&mut s.genc, ty("VideoEncode"));
                push(&mut s.gdec, ty("VideoDecode"));
                push(&mut s.ded, (smp.ded >> 20) as f32);
                push(&mut s.shr, (smp.shr >> 20) as f32);
            }
            for ds in &self.sample.disks {
                let i = match self.disks.iter().position(|d| d.number == ds.number) {
                    Some(i) => i,
                    None => {
                        self.disks.push(DiskState {
                            number: ds.number,
                            letters: ds.letters.clone(),
                            info: hw::disk_info_for(ds.number),
                            formatted: hw::volumes_total(&ds.letters),
                            active: 0.0,
                            rd: 0.0,
                            wr: 0.0,
                            ms: 0.0,
                            s_active: Default::default(),
                            s_rd: Default::default(),
                            s_wr: Default::default(),
                        });
                        self.disks.len() - 1
                    }
                };
                let d = &mut self.disks[i];
                (d.letters, d.active, d.rd, d.wr, d.ms) = (ds.letters.clone(), ds.active, ds.rd, ds.wr, ds.ms);
                push(&mut d.s_active, ds.active);
                push(&mut d.s_rd, (ds.rd / 1_048_576.0) as f32);
                push(&mut d.s_wr, (ds.wr / 1_048_576.0) as f32);
            }
            self.disks.sort_by_key(|d| d.number);
            for n in &self.sample.nics {
                let i = match self.nics.iter().position(|x| x.name == n.name) {
                    Some(i) => i,
                    None => {
                        self.nics.push(NicState { name: n.name.clone(), rx: 0.0, tx: 0.0, bw: 0.0, s_rx: Default::default(), s_tx: Default::default() });
                        self.nics.len() - 1
                    }
                };
                let x = &mut self.nics[i];
                (x.rx, x.tx, x.bw) = (n.rx, n.tx, n.bw);
                push(&mut x.s_rx, (n.rx * 8.0 / 1000.0) as f32);
                push(&mut x.s_tx, (n.tx * 8.0 / 1000.0) as f32);
            }
            let alive: std::collections::HashSet<&String> = self.sample.nics.iter().map(|n| &n.name).collect();
            self.nics.retain(|n| alive.contains(&n.name));
        }

        vec![]
    }
}

fn disk_capacity() -> u64 {
    use sysinfo::Disks;
    Disks::new_with_refreshed_list()
        .map(|d| (&d).into_iter().find(|x| x.mount_point().to_string_lossy().to_uppercase().starts_with("C:")).map_or(0, |x| x.total_space()))
        .unwrap_or(0)
}

pub fn process_cmd(d: &Data, pid: u32) -> String {
    d.sys.process(Pid::from_u32(pid)).map(|p| p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_keeps_only_cap_points() {
        let mut q = VecDeque::new();
        for i in 0..(CAP + 30) {
            push(&mut q, i as f32);
        }
        assert_eq!(q.len(), CAP);
        assert_eq!(*q.front().unwrap(), 30.0);
    }

    #[test]
    fn refresh_fills_processes_and_series() {
        let mut d = Data::new(store::Settings::default());
        d.refresh();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        d.refresh();
        assert!(d.procs.len() > 20);
        assert!(d.procs.iter().any(|p| p.pid == std::process::id()));
        assert_eq!(d.series.cpu.len(), 1);
        assert!(d.meminfo.processes > 0);
        assert!(!d.hints().is_empty());
    }
}
