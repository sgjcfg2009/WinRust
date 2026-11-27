use std::{env, fmt::Write as _, fs, io::Write, path::PathBuf, process::Command};

const JUNK: &[&str] = &[
    "Clipchamp.Clipchamp", "Microsoft.BingNews", "Microsoft.BingWeather", "Microsoft.BingSearch", "Microsoft.Copilot",
    "Microsoft.Windows.Ai.Copilot.Provider", "Microsoft.549981C3F5F10",
    "Microsoft.GamingApp", "Microsoft.XboxGameOverlay", "Microsoft.XboxGamingOverlay", "Microsoft.XboxSpeechToTextOverlay", "Microsoft.Xbox.TCUI",
    "Microsoft.GetHelp", "Microsoft.Getstarted", "Microsoft.MicrosoftOfficeHub", "Microsoft.Office.OneNote", "Microsoft.MicrosoftSolitaireCollection",
    "Microsoft.People", "Microsoft.PowerAutomateDesktop", "Microsoft.Todos", "Microsoft.WindowsAlarms", "microsoft.windowscommunicationsapps",
    "Microsoft.WindowsFeedbackHub", "Microsoft.WindowsMaps", "Microsoft.WindowsSoundRecorder", "Microsoft.YourPhone", "Microsoft.ZuneMusic",
    "Microsoft.ZuneVideo", "MicrosoftCorporationII.MicrosoftFamily", "MicrosoftCorporationII.QuickAssist", "MicrosoftTeams", "MSTeams",
    "Microsoft.OutlookForWindows", "Microsoft.Windows.DevHome", "MicrosoftWindows.Client.WebExperience",
    "Microsoft.SkypeApp", "Microsoft.MixedReality.Portal", "Microsoft.Microsoft3DViewer", "Microsoft.Print3D", "Microsoft.Wallet",
    "SpotifyAB.SpotifyMusic", "Disney.", "Netflix", "TikTok", "Facebook", "Instagram", "LinkedIn", "Twitter", "AmazonVideo", "king.com.", "Duolingo",
];

const PROTECTED: &[&str] = &[
    "Microsoft.WindowsStore", "Microsoft.StorePurchaseApp", "Microsoft.DesktopAppInstaller", "windows.immersivecontrolpanel",
    "Microsoft.Windows.ShellExperienceHost", "Microsoft.Windows.StartMenuExperienceHost", "MicrosoftWindows.Client.", "Microsoft.Windows.",
    "Microsoft.NET", "Microsoft.VCLibs", "Microsoft.UI.Xaml", "Microsoft.WindowsAppRuntime", "Microsoft.SecHealthUI", "Microsoft.AAD.BrokerPlugin",
    "Microsoft.AccountsControl", "Microsoft.LockApp", "Microsoft.XboxIdentityProvider", "Microsoft.GamingServices", "Microsoft.WindowsTerminal",
    "Microsoft.WindowsCalculator", "Microsoft.Paint", "Microsoft.ScreenSketch", "Microsoft.Windows.Photos", "Microsoft.WindowsNotepad",
];

const OVERRIDE_PROTECTED: &[&str] = &[
    "Microsoft.Windows.Ai.Copilot.Provider",
    "Microsoft.Windows.DevHome",
    "MicrosoftWindows.Client.WebExperience",
];

const TASKS: &[&str] = &[
    r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser",
    r"\Microsoft\Windows\Application Experience\ProgramDataUpdater",
    r"\Microsoft\Windows\Application Experience\StartupAppTask",
    r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
    r"\Microsoft\Windows\Customer Experience Improvement Program\UsbCeip",
    r"\Microsoft\Windows\Customer Experience Improvement Program\KernelCeipTask",
    r"\Microsoft\Windows\Feedback\Siuf\DmClient",
    r"\Microsoft\Windows\Feedback\Siuf\DmClientOnScenarioDownload",
    r"\Microsoft\Windows\Windows Error Reporting\QueueReporting",
];

fn starts_ic(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix))
}

fn contains_ic(s: &str, needle: &str) -> bool {
    !needle.is_empty() && s.as_bytes().windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

fn is_junk(name: &str) -> bool {
    let keep = PROTECTED.iter().any(|p| starts_ic(name, p)) && !OVERRIDE_PROTECTED.iter().any(|p| starts_ic(name, p));
    !keep && JUNK.iter().any(|p| starts_ic(name, p) || (p.len() >= 5 && !p.contains('.') && contains_ic(name, p)))
}

enum Item {
    Appx { full: String, name: String },
    Prov { pkg: String, name: String },
}

fn run(prog: &str, args: &[&str]) -> (bool, String) {
    match Command::new(prog).args(args).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
        Err(e) => (false, e.to_string()),
    }
}

fn ps(script: &str) -> (bool, String) {
    run("powershell", &["-NoProfile", "-NonInteractive", "-Command", script])
}

fn parse(out: &str) -> Vec<Item> {
    let mut v = vec![];
    for l in out.lines().map(str::trim) {
        if l.contains('\'') {
            continue;
        }
        let mut it = l.split('|');
        match (it.next(), it.next(), it.next(), it.next(), it.next()) {
            (Some("appx"), Some(full), Some(name), Some(nonremovable), None) if nonremovable != "True" && is_junk(name) => {
                v.push(Item::Appx { full: full.to_string(), name: name.to_string() });
            }
            (Some("prov"), Some(pkg), Some(name), None, None) if is_junk(name) => {
                v.push(Item::Prov { pkg: pkg.to_string(), name: name.to_string() });
            }
            _ => {}
        }
    }
    v
}

fn found() -> Result<Vec<Item>, String> {
    let (ok, out) = ps(r#"Get-AppxPackage -AllUsers | ForEach-Object { "appx|$($_.PackageFullName)|$($_.Name)|$($_.NonRemovable)" }; Get-AppxProvisionedPackage -Online | ForEach-Object { "prov|$($_.PackageName)|$($_.DisplayName)" }"#);
    if !ok && out.trim().is_empty() {
        return Err("cannot list apps (run as administrator)".into());
    }
    Ok(parse(&out))
}

fn existing_tasks() -> Vec<&'static str> {
    let (ok, out) = run("schtasks.exe", &["/Query", "/FO", "CSV", "/NH"]);
    if !ok {
        return vec![];
    }
    let mut present = Vec::new();
    for t in TASKS {
        let found = out.lines().any(|l| {
            l.strip_prefix('"')
                .and_then(|r| r.split('"').next())
                .is_some_and(|name| name.eq_ignore_ascii_case(t))
        });
        if found {
            present.push(*t);
        }
    }
    present
}

fn onedrive_setup() -> Option<PathBuf> {
    let root = PathBuf::from(env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
    ["SysWOW64", "System32"].iter().map(|d| root.join(d).join("OneDriveSetup.exe")).find(|p| p.is_file())
}

fn edge_setup() -> Option<PathBuf> {
    let base = PathBuf::from(env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into())).join(r"Microsoft\Edge\Application");
    let mut vs: Vec<PathBuf> = fs::read_dir(base).ok()?.flatten().map(|e| e.path().join(r"Installer\setup.exe")).filter(|p| p.is_file()).collect();
    vs.sort();
    vs.pop()
}

fn plan(od: bool, edge: bool) {
    println!("Dry run: nothing is removed. --apply REMOVES it FOR GOOD (no undo).\n");
    match found() {
        Ok(items) => {
            for i in &items {
                match i {
                    Item::Appx { name, .. } => println!("  app (installed)    {name}"),
                    Item::Prov { name, .. } => println!("  app (provisioned)  {name}"),
                }
            }
            println!("  {} app entries found", items.len());
        }
        Err(e) => println!("  {e}"),
    }
    for t in existing_tasks() {
        println!("  task               {t}");
    }
    if od {
        println!("  OneDrive           {}", onedrive_setup().map_or("not present".to_string(), |p| p.display().to_string()));
    }
    if edge {
        println!("  Edge               {}", edge_setup().map_or("not present".to_string(), |p| p.display().to_string()));
    }
}

fn apply(od: bool, edge: bool) -> Result<(), String> {
    let dir = PathBuf::from(env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("winrust-purge");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e} (run as admin)", dir.display()))?;
    let mut log = fs::OpenOptions::new().create(true).append(true).open(dir.join("removed.txt")).map_err(|e| e.to_string())?;
    let mut failed = 0u32;
    let mut did = |what: &str, ok: bool| {
        println!("{}  {what}", if ok { "removed" } else { "FAILED " });
        if ok {
            let _ = writeln!(log, "{what}");
        } else {
            failed += 1;
        }
    };
    let items = found()?;
    if !items.is_empty() {
        let mut script = String::with_capacity(items.len() * 96);
        for (idx, i) in items.iter().enumerate() {
            match i {
                Item::Appx { full, .. } => {
                    let _ = writeln!(script, "try {{ Remove-AppxPackage -Package '{full}' -AllUsers -ErrorAction Stop; 'OK|{idx}' }} catch {{ 'ERR|{idx}' }}");
                }
                Item::Prov { pkg, .. } => {
                    let _ = writeln!(script, "try {{ Remove-AppxProvisionedPackage -Online -PackageName '{pkg}' -ErrorAction Stop | Out-Null; 'OK|{idx}' }} catch {{ 'ERR|{idx}' }}");
                }
            }
        }
        let (_, out) = ps(&script);
        let mut ok_flags = vec![false; items.len()];
        for l in out.lines().map(str::trim) {
            if let Some(rest) = l.strip_prefix("OK|")
                && let Ok(idx) = rest.parse::<usize>()
                && let Some(slot) = ok_flags.get_mut(idx)
            {
                *slot = true;
            }
        }
        for (i, ok) in items.iter().zip(ok_flags) {
            match i {
                Item::Appx { name, .. } => did(&format!("app {name}"), ok),
                Item::Prov { name, .. } => did(&format!("provisioned {name}"), ok),
            }
        }
    }
    for t in existing_tasks() {
        did(&format!("task {t}"), run("schtasks.exe", &["/Delete", "/TN", t, "/F"]).0);
    }
    if od {
        let _ = run("taskkill", &["/f", "/im", "OneDrive.exe"]);
        match onedrive_setup() {
            Some(p) => did("OneDrive", run(&p.display().to_string(), &["/uninstall"]).0),
            None => println!("skip     OneDrive: not present"),
        }
    }
    if edge {
        match edge_setup() {
            Some(p) => {
                let _ = run("reg", &["add", r"HKLM\SOFTWARE\Microsoft\EdgeUpdateDev", "/v", "AllowUninstall", "/t", "REG_SZ", "/d", "", "/f"]);
                did("Edge (best effort, Windows may refuse)", run(&p.display().to_string(), &["--uninstall", "--system-level", "--verbose-logging", "--force-uninstall"]).0);
            }
            None => println!("skip     Edge: not present"),
        }
    }
    println!("\nThis cannot be undone. A log of what was removed: {}", dir.join("removed.txt").display());
    if failed > 0 { Err(format!("{failed} item(s) failed (run as administrator)")) } else { Ok(()) }
}

fn main() {
    winrust_common::utf8_console();
    let a: Vec<String> = env::args().skip(1).collect();
    let has = |f: &str| a.iter().any(|x| x == f);
    let r = if has("--ask") {
        plan(has("--onedrive"), has("--edge"));
        print!("\nThis REMOVES the apps above FOR GOOD, there is no undo. Type YES to continue: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let r = if line.trim() == "YES" { apply(has("--onedrive"), has("--edge")) } else { println!("Nothing removed."); Ok(()) };
        println!("\nPress Enter to close.");
        let _ = std::io::stdin().read_line(&mut line);
        r
    } else if has("--apply") {
        apply(has("--onedrive"), has("--edge"))
    } else {
        plan(has("--onedrive"), has("--edge"));
        Ok(())
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_goes_and_the_system_stays() {
        for n in ["Microsoft.BingNews", "Clipchamp.Clipchamp", "Microsoft.ZuneMusic", "MicrosoftTeams", "king.com.CandyCrushSaga", "Microsoft.Windows.DevHome", "MicrosoftWindows.Client.WebExperience", "5319275A.Facebook"] {
            assert!(is_junk(n), "{n}");
        }
        for n in ["Microsoft.WindowsStore", "windows.immersivecontrolpanel", "Microsoft.Windows.ShellExperienceHost", "Microsoft.Windows.StartMenuExperienceHost", "Microsoft.WindowsCalculator", "Microsoft.XboxIdentityProvider", "MicrosoftWindows.Client.CBS", "Microsoft.VCLibs.140.00", "Microsoft.Windows.Photos"] {
            assert!(!is_junk(n), "{n}");
        }
    }

    #[test]
    fn parses_listing_and_skips_nonremovable_and_quotes() {
        let out = "appx|Microsoft.BingNews_1_x64__8wekyb|Microsoft.BingNews|False\nappx|Microsoft.ZuneMusic_1|Microsoft.ZuneMusic|True\nprov|Clipchamp.Clipchamp_1|Clipchamp.Clipchamp\nprov|X'Y|Microsoft.BingNews\nappx|a|Microsoft.WindowsStore|False\n";
        assert_eq!(parse(out).len(), 2);
    }
}
