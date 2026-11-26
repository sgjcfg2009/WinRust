use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::Command,
};

const SERVICES: &[(&str, &str)] = &[
    ("DiagTrack", "Connected User Experiences and Telemetry"),
    ("dmwappushservice", "WAP push message routing (telemetry helper)"),
    ("SysMain", "Superfetch/prefetch (little use on an SSD)"),
    ("MapsBroker", "downloaded maps"),
    ("RetailDemo", "retail demo mode"),
    ("WerSvc", "Windows Error Reporting"),
    ("Fax", "Fax"),
];
const WSEARCH: (&str, &str) = ("WSearch", "Windows Search indexer (replaced by rs)");

const REGS: &[(&str, &str, u32, &str)] = &[
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection", "AllowTelemetry", 0, "telemetry to the minimum"),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Dsh", "AllowNewsAndInterests", 0, "Widgets off"),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableWindowsConsumerFeatures", 1, "no suggested/promoted apps"),
    (r"HKCU\Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions", 1, "no web results in Start search"),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot", "TurnOffWindowsCopilot", 1, "Copilot off"),
];

fn run(prog: &str, args: &[&str]) -> (bool, String) {
    match Command::new(prog).args(args).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
        Err(e) => (false, e.to_string()),
    }
}

fn start_mode(svc: &str) -> Option<&'static str> {
    let (ok, out) = run("sc.exe", &["qc", svc]);
    let line = out.lines().find(|l| ok && l.contains("START_TYPE"))?;
    Some(if line.contains("DISABLED") {
        "disabled"
    } else if line.contains("DEMAND_START") {
        "demand"
    } else if line.contains("AUTO_START") {
        if line.contains("DELAYED") { "delayed-auto" } else { "auto" }
    } else {
        return None;
    })
}

fn reg_exists(key: &str, value: &str) -> bool {
    run("reg.exe", &["query", key, "/v", value]).0
}

fn undo_path() -> PathBuf {
    PathBuf::from(env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("rs-debloat").join("undo.txt")
}

fn services(with_wsearch: bool) -> Vec<(&'static str, &'static str)> {
    let mut v = SERVICES.to_vec();
    if with_wsearch {
        v.push(WSEARCH);
    }
    v
}

fn plan(with_wsearch: bool) {
    println!("Dry run: nothing is changed. Add --apply (admin) to do it, --undo to revert.\n");
    for (svc, why) in services(with_wsearch) {
        match start_mode(svc) {
            None => println!("  service {svc:<18} not present"),
            Some("disabled") => println!("  service {svc:<18} already disabled"),
            Some(m) => println!("  service {svc:<18} {m} -> disabled   ({why})"),
        }
    }
    for (key, value, data, why) in REGS {
        let state = if reg_exists(key, value) { "already set, left alone" } else { "will set" };
        println!("  policy  {value:<32} = {data}  {state}   ({why})");
    }
    if !with_wsearch {
        println!("\nWindows Search is not included; add --with-wsearch once rs has replaced it for you.");
    }
}

fn apply(with_wsearch: bool) -> Result<(), String> {
    let path = undo_path();
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("cannot create {}: {e} (run as admin)", path.parent().unwrap().display()))?;
    let mut log = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    for (svc, _) in services(with_wsearch) {
        match start_mode(svc) {
            None => println!("skip   {svc}: not present"),
            Some("disabled") => println!("skip   {svc}: already disabled"),
            Some(mode) => {
                writeln!(log, "svc|{svc}|{mode}").map_err(|e| e.to_string())?;
                let (ok, _) = run("sc.exe", &["config", svc, "start=", "disabled"]);
                let _ = run("sc.exe", &["stop", svc]);
                println!("{}  {svc}: {mode} -> disabled", if ok { "done  " } else { "FAILED" });
            }
        }
    }
    for (key, value, data, _) in REGS {
        if reg_exists(key, value) {
            println!("skip   {value}: already set");
            continue;
        }
        writeln!(log, "reg|{key}|{value}").map_err(|e| e.to_string())?;
        let (ok, _) = run("reg.exe", &["add", key, "/v", value, "/t", "REG_DWORD", "/d", &data.to_string(), "/f"]);
        println!("{}  {value} = {data}", if ok { "done  " } else { "FAILED" });
    }
    println!("\nUndo information saved to {}. Some changes need a reboot or sign-out.", path.display());
    Ok(())
}

fn undo() -> Result<(), String> {
    let path = undo_path();
    let text = fs::read_to_string(&path).map_err(|_| "nothing to undo (no undo file)".to_string())?;
    for line in text.lines() {
        let f: Vec<&str> = line.split('|').collect();
        let ok = match f.as_slice() {
            ["svc", name, mode] => run("sc.exe", &["config", name, "start=", mode]).0,
            ["reg", key, value] => run("reg.exe", &["delete", key, "/v", value, "/f"]).0,
            _ => continue,
        };
        println!("{}  {line}", if ok { "restored" } else { "FAILED  " });
    }
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    println!("\nServices were restored to their old start mode (start them manually or reboot).");
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let ws = args.iter().any(|a| a == "--with-wsearch");
    let r = if args.iter().any(|a| a == "--apply") {
        apply(ws)
    } else if args.iter().any(|a| a == "--undo") {
        undo()
    } else {
        plan(ws);
        Ok(())
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
