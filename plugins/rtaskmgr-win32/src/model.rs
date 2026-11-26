use crate::{data::*, ui::*};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Page {
    Processes,
    Performance,
    Startup,
    Users,
    Details,
    Settings,
}

pub static NAV: std::sync::LazyLock<[(Page, char, &'static str); 6]> = std::sync::LazyLock::new(|| [
    (Page::Processes, '\u{E9D9}', "Processes"),
    (Page::Performance, '\u{E9D2}', "Performance"),
    (Page::Startup, '\u{EC4A}', "Startup"),
    (Page::Users, '\u{E77B}', "Users"),
    (Page::Details, '\u{EA37}', "Details"),
    (Page::Settings, '\u{E713}', "Settings"),
]);

pub fn page_title(p: Page) -> &'static str {
    NAV.iter().find(|n| n.0 == p).map_or("", |n| n.2)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    Right,
}

#[derive(Clone, Copy)]
pub struct Col {
    pub title: &'static str,
    pub w: f64,
    pub align: Align,
    pub metric: Option<Metric>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Metric {
    Cpu,
    Mem,
    Disk,
    Net,
    Gpu,
}

const fn col(title: &'static str, w: f64, align: Align) -> Col {
    Col { title, w, align, metric: None }
}
const fn mcol(title: &'static str, m: Metric) -> Col {
    Col { title, w: 84.0, align: Align::Right, metric: Some(m) }
}

pub fn columns(p: Page) -> Vec<Col> {
    use Align::*;
    use Metric::*;
    match p {
        Page::Processes => vec![
            col("Name", 0.0, Left),
            mcol("CPU", Cpu),
            mcol("Memory", Mem),
            mcol("Disk", Disk),
            mcol("Network", Net),
            mcol("GPU", Gpu),
        ],
        Page::Users => vec![
            col("User", 270.0, Left),
            col("Processes", 112.0, Right),
            mcol("CPU", Cpu),
            mcol("Memory", Mem),
            mcol("Disk", Disk),
            mcol("Network", Net),
            mcol("GPU", Gpu),
        ],
        Page::Details => vec![col("Name", 220.0, Left), col("PID", 70.0, Right), col("Status", 100.0, Left), col("CPU", 70.0, Right), col("Memory", 110.0, Right)],
        Page::Startup => vec![col("Name", 240.0, Left), col("Publisher", 180.0, Left), col("Status", 100.0, Left), col("Slows boot by", 150.0, Right), col("Command", 0.0, Left)],
        _ => vec![],
    }
}

pub fn default_sort(p: Page) -> (usize, bool) {
    match p {
        Page::Processes => (1, true),
        Page::Users => (2, true),
        Page::Details => (3, true),
        _ => (0, false),
    }
}

#[derive(Clone, Default)]
pub struct Row {
    pub key: String,
    pub depth: u8,
    pub is_section: bool,
    pub expandable: bool,
    pub expanded: bool,
    pub icon_path: Option<String>,
    pub cells: Vec<String>,
    pub heat: Vec<Option<(f64, f64)>>,
    pub sortv: Vec<f64>,
    pub tip: String,
    pub pids: Vec<u32>,
    pub aux: String,
}

pub fn dec(v: f64, digits: usize) -> String {
    format!("{:.*}", digits, v)
}

pub fn fmt_cpu(v: f32) -> String {
    if v < 0.05 { "0%".into() } else { format!("{}%", dec(v as f64, 1)) }
}

pub fn fmt_mem_ru(b: u64) -> String {
    let mb = b as f64 / 1_048_576.0;
    if mb >= 1000.0 { format!("{} MB", group_thousands(mb as u64)) } else if mb >= 100.0 { format!("{} MB", dec(mb, 0)) } else { format!("{} MB", dec(mb, 1)) }
}

pub fn fmt_disk_ru(bps: f64) -> String {
    let mb = bps / 1_048_576.0;
    if mb < 0.05 { "0 MB/s".into() } else { format!("{} MB/s", dec(mb, 1)) }
}

pub fn fmt_net_ru(bps: f64, known: bool) -> String {
    if !known {
        return "—".into();
    }
    let mbit = bps * 8.0 / 1e6;
    if mbit < 0.05 { "0 Mbps".into() } else { format!("{} Mbps", dec(mbit, 1)) }
}

fn friendly(d: &Data, p: &Proc) -> String {
    if let Some(r) = p.exe.as_deref().and_then(|e| d.res.get(e))
        && !r.desc.is_empty() {
            return r.desc;
        }
    p.name.strip_suffix(".exe").or_else(|| p.name.strip_suffix(".EXE")).unwrap_or(&p.name).to_string()
}

fn svc_display(d: &Data, pid: u32) -> Option<String> {
    let name = d.svc_pids.get(&pid)?.first()?.clone();
    Some(d.svcs.iter().find(|s| s.name == name).map_or(name, |s| s.display.clone()))
}

fn child_name(d: &Data, p: &Proc) -> String {
    if p.name.eq_ignore_ascii_case("svchost.exe")
        && let Some(s) = svc_display(d, p.pid) {
            return format!("Service host: {s}");
        }
    if let Some(t) = &p.title {
        return t.clone();
    }
    friendly(d, p)
}

fn proc_row(d: &Data, key: String, depth: u8, name: String, members: &[&Proc], expandable: bool, expanded: bool, with_status: bool) -> Row {
    let cpu: f32 = members.iter().map(|p| p.cpu).sum();
    let mem: u64 = members.iter().map(|p| p.mem).sum();
    let disk: f64 = members.iter().map(|p| p.disk).sum();
    let net: f64 = members.iter().map(|p| p.net).sum();
    let gpu: f32 = members.iter().map(|p| p.gpu).fold(0.0, f32::max);
    let etw = d.etw.is_some();
    let gpu_s = if gpu >= 0.05 { format!("{}%", dec(gpu as f64, 0)) } else { "0%".into() };
    let mut cells = vec![name];
    let mut heat = vec![None];
    let mut sortv = vec![f64::NAN];
    if with_status {
        let status = if members.iter().all(|p| d.suspended.contains(&p.pid)) {
            "Suspended"
        } else if members.iter().all(|p| d.efficiency.contains(&p.pid)) {
            "Efficiency"
        } else {
            ""
        };
        cells.push(status.into());
        heat.push(None);
        sortv.push(f64::NAN);
    }
    cells.extend([fmt_cpu(cpu), fmt_mem_ru(mem), fmt_disk_ru(disk), fmt_net_ru(net, etw), gpu_s]);
    heat.extend([Some((cpu as f64, 40.0)), Some((mem as f64, 1.5e9)), Some((disk, 5e6)), Some((net, 5e6)), Some((gpu as f64, 40.0))]);
    sortv.extend([cpu as f64, mem as f64, disk, net, gpu as f64]);
    Row {
        key,
        depth,
        is_section: false,
        expandable,
        expanded,
        icon_path: members.iter().find_map(|p| p.exe.clone()),
        cells,
        heat,
        sortv,
        tip: String::new(),
        pids: members.iter().map(|p| p.pid).collect(),
        aux: String::new(),
    }
}

pub struct BuildCtx<'a> {
    pub page: Page,
    pub search: &'a str,
    pub expanded: &'a HashSet<String>,
    pub sort: (usize, bool),
}

fn matches(d: &Data, p: &Proc, q: &str) -> bool {
    p.name.to_lowercase().contains(q)
        || p.pid.to_string() == q
        || friendly(d, p).to_lowercase().contains(q)
        || p.exe.as_deref().and_then(|e| d.res.get(e)).is_some_and(|r| r.company.to_lowercase().contains(q))
        || p.title.as_deref().is_some_and(|t| t.to_lowercase().contains(q))
}

pub fn build_rows(d: &Data, c: &BuildCtx) -> Vec<Row> {
    let q = c.search.trim().to_lowercase();
    let mut rows: Vec<Row> = match c.page {
        Page::Processes => {
            let mut groups: HashMap<String, Vec<&Proc>> = HashMap::new();
            for p in &d.procs {
                groups.entry(group_key(&p.name)).or_default().push(p);
            }
            let mut apps: Vec<(Row, Vec<Row>)> = vec![];
            let mut bg: Vec<(Row, Vec<Row>)> = vec![];
            for (key, mut members) in groups {
                if !q.is_empty() && !members.iter().any(|p| matches(d, p, &q)) {
                    continue;
                }
                members.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
                let is_app = members.iter().any(|p| p.title.is_some());
                let multi = members.len() > 1;
                let base = if members[0].name.eq_ignore_ascii_case("svchost.exe") { "Service host".to_string() } else { friendly(d, members[0]) };
                let name = if multi { format!("{base} ({})", members.len()) } else if members[0].name.eq_ignore_ascii_case("svchost.exe") { child_name(d, members[0]) } else { base };
                let expanded = multi && c.expanded.contains(&key);
                let head = proc_row(d, key.clone(), 0, name, &members, multi, expanded, false);
                let kids: Vec<Row> = if expanded {
                    members.iter().map(|p| proc_row(d, format!("{key}#{}", p.pid), 1, child_name(d, p), &[p], false, false, false)).collect()
                } else {
                    vec![]
                };
                if is_app {
                    apps.push((head, kids));
                } else {
                    bg.push((head, kids));
                }
            }
            sort_pairs(&mut apps, c.sort);
            sort_pairs(&mut bg, c.sort);
            let mut res = Vec::with_capacity(apps.len() + bg.len() + 2);
            if !apps.is_empty() {
                res.push(Row {
                    key: "__sec_apps__".into(),
                    is_section: true,
                    cells: vec![format!("Apps ({})", apps.len())],
                    heat: vec![None; 7],
                    sortv: vec![f64::NAN; 7],
                    ..Default::default()
                });
                res.extend(apps.into_iter().flat_map(|(h, k)| std::iter::once(h).chain(k)));
            }
            if !bg.is_empty() {
                res.push(Row {
                    key: "__sec_bg__".into(),
                    is_section: true,
                    cells: vec![format!("Background processes ({})", bg.len())],
                    heat: vec![None; 7],
                    sortv: vec![f64::NAN; 7],
                    ..Default::default()
                });
                res.extend(bg.into_iter().flat_map(|(h, k)| std::iter::once(h).chain(k)));
            }
            res
        }
        Page::Users => {
            let mut groups: HashMap<String, Vec<&Proc>> = HashMap::new();
            for p in &d.procs {
                groups.entry(p.user.clone()).or_default().push(p);
            }
            let mut out: Vec<(Row, Vec<Row>)> = vec![];
            for (user, mut members) in groups {
                if !q.is_empty() && !user.to_lowercase().contains(&q) && !members.iter().any(|p| matches(d, p, &q)) {
                    continue;
                }
                members.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
                let key = format!("user:{user}");
                let expanded = c.expanded.contains(&key);
                let mut head = proc_row(d, key.clone(), 0, user.clone(), &members, true, expanded, true);
                head.cells[1] = members.len().to_string();
                head.sortv[1] = members.len() as f64;
                head.icon_path = None;
                head.tip = String::new();
                let kids: Vec<Row> = if expanded { members.iter().map(|p| proc_row(d, format!("{key}#{}", p.pid), 1, child_name(d, p), &[p], false, false, true)).collect() } else { vec![] };
                out.push((head, kids));
            }
            sort_pairs(&mut out, c.sort);
            out.into_iter().flat_map(|(h, k)| std::iter::once(h).chain(k)).collect()
        }
        Page::Details => d
            .procs
            .iter()
            .filter(|p| q.is_empty() || matches(d, p, &q))
            .map(|p| Row {
                key: p.pid.to_string(),
                icon_path: p.exe.clone(),
                cells: vec![p.name.clone(), p.pid.to_string(), (if d.suspended.contains(&p.pid) { "Suspended" } else { "Running" }).into(), fmt_cpu(p.cpu), fmt_mem_ru(p.mem)],
                sortv: vec![f64::NAN, p.pid as f64, f64::NAN, p.cpu as f64, p.mem as f64],
                pids: vec![p.pid],
                heat: vec![None; 5],
                ..Default::default()
            })
            .collect(),
        Page::Startup => d
            .startup
            .iter()
            .filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.command.to_lowercase().contains(&q))
            .map(|s| {
                let exe = exe_from_command(&s.command);
                let company = exe.as_deref().and_then(|e| d.res.get(e)).map(|r| r.company).unwrap_or_default();
                let impact = if d.is_admin { crate::startup::boot_impact(s, crate::startup::boot_log()) } else { None };
                let impact_txt = match impact {
                    Some(ms) => format!("+{} s", dec(ms as f64 / 1000.0, 1)),
                    None if d.is_admin => "—".into(),
                    None => "administrator needed".into(),
                };
                Row {
                    key: s.key(),
                    icon_path: exe,
                    cells: vec![s.name.clone(), company, (if s.enabled { "Enabled" } else { "Disabled" }).into(), impact_txt, s.command.clone()],
                    sortv: vec![f64::NAN, f64::NAN, f64::NAN, impact.unwrap_or(0) as f64, f64::NAN],
                    heat: vec![None; 5],
                    tip: format!("Source: {}.{}", s.source_label(), if s.needs_admin() && !d.is_admin { " Administrator rights are needed to change it." } else { "" }),
                    ..Default::default()
                }
            })
            .collect(),
        _ => vec![],
    };
    if !matches!(c.page, Page::Processes | Page::Users) {
        let (ci, desc) = c.sort;
        rows.sort_by(|a, b| {
            let o = cmp_cells(a, b, ci);
            if desc { o.reverse() } else { o }
        });
    }
    rows
}

fn cmp_cells(a: &Row, b: &Row, ci: usize) -> std::cmp::Ordering {
    let (x, y) = (a.sortv.get(ci).copied().unwrap_or(f64::NAN), b.sortv.get(ci).copied().unwrap_or(f64::NAN));
    if x.is_nan() || y.is_nan() {
        a.cells.get(ci).map(|s| s.to_lowercase()).cmp(&b.cells.get(ci).map(|s| s.to_lowercase()))
    } else {
        x.total_cmp(&y)
    }
}

fn sort_pairs(v: &mut [(Row, Vec<Row>)], sort: (usize, bool)) {
    let (ci, desc) = sort;
    let ord = |a: &Row, b: &Row| {
        let o = cmp_cells(a, b, ci);
        if desc { o.reverse() } else { o }
    };
    v.sort_by(|a, b| ord(&a.0, &b.0));
    for (_, kids) in v.iter_mut() {
        kids.sort_by(|a, b| ord(a, b));
    }
}

pub fn exe_from_command(cmd: &str) -> Option<String> {
    let c = cmd.trim();
    let path = if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let l = c.to_lowercase();
        let end = l.find(".exe").map(|i| i + 4).unwrap_or(c.len());
        c[..end].to_string()
    };
    let path = path.replace("%windir%", &std::env::var("WINDIR").unwrap_or_default());
    (!path.is_empty() && path.contains('\\')).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_extraction() {
        assert_eq!(exe_from_command(r#""C:\Program Files\X\x.exe" --silent"#).as_deref(), Some(r"C:\Program Files\X\x.exe"));
        assert_eq!(exe_from_command(r"C:\Tools\a.exe -k b").as_deref(), Some(r"C:\Tools\a.exe"));
        assert_eq!(exe_from_command("cmd /c foo"), None);
    }

    #[test]
    fn number_formats_use_comma() {
        assert_eq!(fmt_cpu(2.04), "2,0%");
        assert_eq!(fmt_cpu(0.01), "0%");
        assert_eq!(fmt_mem_ru(3_746_000_000), "3 572 МБ");
        assert_eq!(fmt_mem_ru(73 * 1_048_576 + 100_000), "73,1 МБ");
        assert_eq!(fmt_disk_ru(0.0), "0 МБ/с");
        assert_eq!(fmt_net_ru(1.0, false), "—");
    }

    #[test]
    fn every_page_with_table_has_valid_default_sort() {
        for p in [Page::Processes, Page::Users, Page::Details, Page::Startup] {
            assert!(default_sort(p).0 < columns(p).len(), "{p:?}");
        }
    }

    #[test]
    fn users_page_groups_processes_by_account() {
        let mut d = Data::new(crate::store::Settings::default());
        d.refresh();
        let me = d.procs.iter().find(|p| p.pid == std::process::id()).expect("свой процесс").user.clone();
        assert!(!me.is_empty() && me != "СИСТЕМА", "свой токен должен читаться: {me}");
        let mut exp = HashSet::new();
        let rows = build_rows(&d, &BuildCtx { page: Page::Users, search: "", expanded: &exp, sort: default_sort(Page::Users) });
        assert!(rows.iter().all(|r| r.depth == 0 && r.expandable));
        let mine = rows.iter().find(|r| r.cells[0] == me).expect("строка текущего пользователя");
        assert!(mine.pids.contains(&std::process::id()));
        exp.insert(mine.key.clone());
        let rows2 = build_rows(&d, &BuildCtx { page: Page::Users, search: "", expanded: &exp, sort: default_sort(Page::Users) });
        assert!(rows2.iter().filter(|r| r.depth == 1).count() == mine.pids.len());
    }

    #[test]
    fn processes_rows_group_and_expand() {
        let mut d = Data::new(crate::store::Settings::default());
        d.refresh();
        let exp: HashSet<String> = HashSet::new();
        let ctx = BuildCtx { page: Page::Processes, search: "", expanded: &exp, sort: default_sort(Page::Processes) };
        let rows = build_rows(&d, &ctx);
        assert!(!rows.is_empty() && rows.iter().all(|r| r.depth == 0));
        let multi = rows.iter().find(|r| r.expandable).expect("есть группа из нескольких процессов");
        let mut exp2 = HashSet::new();
        exp2.insert(multi.key.clone());
        let ctx2 = BuildCtx { page: Page::Processes, search: "", expanded: &exp2, sort: default_sort(Page::Processes) };
        let rows2 = build_rows(&d, &ctx2);
        assert!(rows2.len() > rows.len());
        assert!(rows2.iter().any(|r| r.depth == 1));
        let pid = std::process::id().to_string();
        let ctx3 = BuildCtx { page: Page::Processes, search: &pid, expanded: &exp, sort: default_sort(Page::Processes) };
        assert!(build_rows(&d, &ctx3).iter().any(|r| r.pids.contains(&std::process::id())));
    }
}
