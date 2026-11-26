use std::{collections::HashMap, env, process::Command, thread};
use sysinfo::{ProcessesToUpdate, System, MINIMUM_CPU_UPDATE_INTERVAL};

fn services() -> HashMap<u32, String> {
    let out = Command::new("tasklist").args(["/svc", "/fo", "csv", "/nh"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim().trim_matches('"').split("\",\"").collect();
            (f.len() == 3 && f[2] != "N/A").then(|| Some((f[1].parse().ok()?, f[2].to_string()))).flatten()
        })
        .collect()
}

struct Row {
    label: String,
    count: usize,
    pid: u32,
    cpu: f32,
    mem: u64,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut sys = System::new().expect("cannot read system information");
    sys.refresh_processes(ProcessesToUpdate::All, true);

    if args.first().map(String::as_str) == Some("--kill") {
        let Some(target) = args.get(1) else { return eprintln!("usage: rs-top --kill <pid|name>") };
        let pid = target.parse::<u32>().ok();
        let victims: Vec<_> = sys
            .processes()
            .values()
            .filter(|p| match pid {
                Some(id) => p.pid().as_u32() == id,
                None => p.name().to_string_lossy().eq_ignore_ascii_case(target),
            })
            .collect();
        if victims.is_empty() {
            return println!("no process matches {target}");
        }
        for p in victims {
            let ok = p.kill();
            println!("{} {} (pid {})", if ok { "killed" } else { "cannot kill (needs admin?)" }, p.name().to_string_lossy(), p.pid());
        }
        return;
    }

    thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.refresh_memory();
    let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
    let svc = services();

    let mut rows: HashMap<String, Row> = HashMap::new();
    for (pid, p) in sys.processes() {
        let name = p.name().to_string_lossy().into_owned();
        let key = if name.eq_ignore_ascii_case("svchost.exe") { format!("{name} {pid}") } else { name.clone() };
        let label = match svc.get(&pid.as_u32()) {
            Some(s) if key != name => format!("{name} [{s}]"),
            _ => name,
        };
        let r = rows.entry(key).or_insert(Row { label, count: 0, pid: pid.as_u32(), cpu: 0.0, mem: 0 });
        r.count += 1;
        r.cpu += p.cpu_usage() / cores;
        r.mem += p.memory();
    }
    let mut rows: Vec<Row> = rows.into_values().collect();
    if args.iter().any(|a| a == "--cpu") {
        rows.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    } else {
        rows.sort_by_key(|r| std::cmp::Reverse(r.mem));
    }
    let n = args.iter().find_map(|a| a.parse::<usize>().ok()).unwrap_or(15);

    let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
    println!("RAM {:.1} / {:.1} GB used\n", gb(sys.used_memory()), gb(sys.total_memory()));
    println!("{:>5} {:>8}  {}", "CPU%", "MEM MB", "PROCESS");
    for r in rows.iter().take(n) {
        let who = if r.count == 1 { format!("{} (pid {})", r.label, r.pid) } else { format!("{} x{}", r.label, r.count) };
        println!("{:>5.1} {:>8}  {}", r.cpu, r.mem >> 20, who);
    }
}
