use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

fn services() -> Vec<(&'static str, &'static str)> {
    vec![
        ("DiagTrack", "Connected User Experiences and Telemetry"),
        ("dmwappushservice", "WAP push message routing (telemetry helper)"),
        ("MapsBroker", "downloaded maps"),
        ("RetailDemo", "retail demo mode"),
        ("WerSvc", "Windows Error Reporting"),
        ("Fax", "Fax"),
        ("DPS", "Diagnostic Policy Service (troubleshooters)"),
        ("WdiServiceHost", "diagnostic service host (needs DPS)"),
        ("WdiSystemHost", "diagnostic system host (needs DPS)"),
        ("whesvc", "Windows Health and Optimized Experiences"),
        ("TrkWks", "distributed link tracking"),
        ("WSAIFabricSvc", "Windows AI Fabric (Copilot+/Recall features)"),
    ]
}

fn regs() -> Vec<(&'static str, &'static str, u32, &'static str)> {
    vec![
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection", "AllowTelemetry", 0, "telemetry to the minimum"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection", "DoNotShowFeedbackNotifications", 1, "no feedback pop-ups"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot", "TurnOffWindowsCopilot", 1, "Copilot off"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsAI", "DisableAIDataAnalysis", 1, "Recall: no snapshots of your screen"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableWindowsConsumerFeatures", 1, "no suggested/promoted apps"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableCloudOptimizedContent", 1, "no cloud-driven content"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableSoftLanding", 1, "no Windows tips and tricks"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\AdvertisingInfo", "DisabledByGroupPolicy", 1, "no advertising ID"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System", "EnableActivityFeed", 0, "no activity history (Timeline)"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System", "PublishUserActivities", 0, "activity history stays local"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\GameDVR", "AllowGameDVR", 0, "Game Bar background capture off"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Edge", "StartupBoostEnabled", 0, "Edge does not preload at sign-in"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Edge", "BackgroundModeEnabled", 0, "Edge does not run after it is closed"),
        (r"HKCU\Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions", 1, "no web results in search"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SilentInstalledAppsEnabled", 0, "no apps installed behind your back"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "PreInstalledAppsEnabled", 0, "no pre-installed promoted apps"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "OemPreInstalledAppsEnabled", 0, "no OEM promoted apps"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SystemPaneSuggestionsEnabled", 0, "no ads in Start"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-338388Value", 0, "no suggestions in Start"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-338389Value", 0, "no tips and suggestions"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-353694Value", 0, "no suggestions in Settings"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-353696Value", 0, "no suggestions in Settings"),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "RotatingLockScreenOverlayEnabled", 0, "no ads on the lock screen"),
    ]
}

fn upd_services() -> Vec<(&'static str, &'static str)> {
    vec![
        ("AdobeARMservice", "Adobe Acrobat updater"),
        ("AdobeUpdateService", "Adobe updater"),
        ("AGSService", "Adobe Genuine Software check"),
        ("AGMService", "Adobe Genuine Monitor"),
        ("MozillaMaintenance", "Mozilla maintenance (Firefox still updates itself while it runs)"),
        ("dbupdate", "Dropbox updater"),
        ("dbupdatem", "Dropbox updater"),
        ("ccleaner", "CCleaner updater"),
        ("ccleanerm", "CCleaner updater"),
    ]
}

const UPD_TASKS: &[&str] = &[
    r"\Adobe Acrobat Update Task",
    r"\AdobeGCInvoker",
    r"\Mozilla\Firefox Background Update",
    r"\Mozilla\Firefox Default Browser Agent",
    r"\DropboxUpdateTaskMachine",
    r"\CCleaner Update",
    r"\CCleanerSkipUAC",
    r"\Opera scheduled Autoupdate",
    r"\Opera GX scheduled Autoupdate",
    r"\Opera scheduled assistant Autoupdate",
    r"\AppleSoftwareUpdate",
    r"\OneDrive Standalone Update Task",
    r"\OneDrive Reporting Task",
    r"\ZoomUpdateTask",
];

const WINGET_TASK: &str = r"\WinRust\Update apps";

fn ntfs_regs() -> Vec<(&'static str, &'static str, u32, &'static str)> {
    vec![
        (r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem", "NtfsDisableLastAccessUpdate", 0x8000_0001, "no last-access time written on every read"),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem", "NtfsDisable8dot3NameCreation", 1, "no 8.3 short name created with every new file"),
    ]
}

const DEF_PROCS: &[&str] = &["cargo.exe", "rustc.exe"];

const HOSTS_BEGIN: &str = "# >>> WinRust DNS block >>>";
const HOSTS_END: &str = "# <<< WinRust DNS block <<<";

const BLOCK: &[&str] = &[
    "doubleclick.net", "ad.doubleclick.net", "static.doubleclick.net", "stats.g.doubleclick.net", "googleads.g.doubleclick.net",
    "pubads.g.doubleclick.net", "securepubads.g.doubleclick.net", "cm.g.doubleclick.net", "googleadservices.com", "www.googleadservices.com",
    "pagead2.googlesyndication.com", "tpc.googlesyndication.com", "adservice.google.com", "partner.googleadservices.com",
    "google-analytics.com", "www.google-analytics.com", "ssl.google-analytics.com", "analytics.google.com", "googletagservices.com",
    "www.googletagservices.com", "app-measurement.com", "adnxs.com", "ib.adnxs.com", "secure.adnxs.com", "acdn.adnxs.com",
    "ads.yahoo.com", "analytics.yahoo.com", "advertising.com", "ads.pubmatic.com", "image6.pubmatic.com", "simage2.pubmatic.com",
    "ads.rubiconproject.com", "fastlane.rubiconproject.com", "pixel.rubiconproject.com", "openx.net", "us-ads.openx.net",
    "rtb.openx.net", "criteo.com", "static.criteo.net", "bidder.criteo.com", "dis.criteo.com", "taboola.com", "cdn.taboola.com",
    "trc.taboola.com", "outbrain.com", "widgets.outbrain.com", "log.outbrain.com", "amazon-adsystem.com", "aax.amazon-adsystem.com",
    "c.amazon-adsystem.com", "s.amazon-adsystem.com", "adsrvr.org", "match.adsrvr.org", "insight.adsrvr.org", "casalemedia.com",
    "as.casalemedia.com", "smartadserver.com", "www.smartadserver.com", "moatads.com", "z.moatads.com", "px.moatads.com",
    "scorecardresearch.com", "sb.scorecardresearch.com", "b.scorecardresearch.com", "quantserve.com", "pixel.quantserve.com",
    "quantcount.com", "hotjar.com", "static.hotjar.com", "script.hotjar.com", "vars.hotjar.com", "mouseflow.com", "cdn.mouseflow.com",
    "crazyegg.com", "script.crazyegg.com", "mixpanel.com", "api.mixpanel.com", "cdn.mxpnl.com", "segment.io", "api.segment.io",
    "cdn.segment.com", "amplitude.com", "api.amplitude.com", "api2.amplitude.com", "branch.io", "api2.branch.io", "app.adjust.com",
    "adjust.com", "appsflyer.com", "t.appsflyer.com", "inapps.appsflyer.com", "adcolony.com", "ads30.adcolony.com", "unityads.unity3d.com",
    "auction.unityads.unity3d.com", "applovin.com", "ms.applovin.com", "rt.applovin.com", "mopub.com", "ads.mopub.com", "inmobi.com",
    "chartboost.com", "live.chartboost.com", "vungle.com", "api.vungle.com", "yieldmo.com", "sharethrough.com", "btlr.sharethrough.com",
    "33across.com", "ssc.33across.com", "media.net", "contextual.media.net", "zedo.com", "revcontent.com", "trends.revcontent.com",
    "mgid.com", "servicer.mgid.com", "propellerads.com", "popads.net", "popcash.net", "exoclick.com", "syndication.exoclick.com",
    "adform.net", "track.adform.net", "serving-sys.com", "bs.serving-sys.com", "flashtalking.com", "servedby.flashtalking.com",
    "bluekai.com", "tags.bluekai.com", "krxd.net", "cdn.krxd.net", "demdex.net", "dpm.demdex.net", "everesttech.net",
    "omtrdc.net", "2o7.net", "tapad.com", "addthis.com", "s7.addthis.com", "sharethis.com", "w.sharethis.com", "ads.tiktok.com",
    "analytics.tiktok.com", "ads-api.tiktok.com", "an.facebook.com", "pixel.facebook.com", "ads.linkedin.com", "px.ads.linkedin.com",
    "ads.twitter.com", "static.ads-twitter.com", "ads-api.twitter.com", "analytics.twitter.com", "ads.reddit.com", "events.reddit.com",
    "ads.pinterest.com", "ct.pinterest.com", "an.yandex.ru", "mc.yandex.ru", "adfox.yandex.ru", "ads.adfox.ru", "top-fwz1.mail.ru",
    "ad.mail.ru", "rs.mail.ru", "counter.yadro.ru", "ssp.rambler.ru", "kraken.rambler.ru",
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Group {
    Base,
    Updaters,
    Defender,
    Ntfs,
    Dns,
}

impl Group {
    fn parse(s: &str) -> Option<Group> {
        Some(match s {
            "updaters" => Group::Updaters,
            "defender" => Group::Defender,
            "ntfs" => Group::Ntfs,
            "dns" => Group::Dns,
            _ => return None,
        })
    }
    fn file(self) -> &'static str {
        match self {
            Group::Base => "undo.txt",
            Group::Updaters => "undo-updaters.txt",
            Group::Defender => "undo-defender.txt",
            Group::Ntfs => "undo-ntfs.txt",
            Group::Dns => "undo-dns.txt",
        }
    }
}

fn run(prog: &str, args: &[&str]) -> (bool, String) {
    let args: Vec<String> = if prog == "reg.exe" { winrust_common::reg::user_args(args) } else { args.iter().map(|a| a.to_string()).collect() };
    match Command::new(prog).args(&args).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
        Err(e) => (false, e.to_string()),
    }
}

fn ps(script: &str) -> (bool, String) {
    run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", script])
}

fn psq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
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

#[derive(Debug, PartialEq)]
enum Val {
    Missing,
    Dword(u32),
    Other,
}

fn parse_val(ok: bool, out: &str) -> Val {
    if !ok {
        return Val::Missing;
    }
    match winrust_common::reg::value(out) {
        Some(_) => winrust_common::reg::dword(out).map_or(Val::Other, Val::Dword),
        None => Val::Missing,
    }
}

fn reg_val(key: &str, value: &str) -> Val {
    let (ok, out) = run("reg.exe", &["query", key, "/v", value]);
    parse_val(ok, &out)
}

fn undo_path(g: Group) -> PathBuf {
    PathBuf::from(env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("winrust-debloat").join(g.file())
}

fn hosts_path() -> PathBuf {
    PathBuf::from(env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\drivers\etc\hosts")
}

fn parse_task_names(csv: &str) -> Vec<String> {
    let mut v: Vec<String> = csv.lines().filter_map(|l| l.strip_prefix('"')?.split('"').next().map(str::to_string)).filter(|n| n.starts_with('\\')).collect();
    v.dedup();
    v
}

fn updater_tasks() -> Vec<String> {
    let (_, out) = run("schtasks.exe", &["/query", "/fo", "csv", "/nh"]);
    parse_task_names(&out).into_iter().filter(|n| UPD_TASKS.iter().any(|p| n.to_lowercase().starts_with(&p.to_lowercase()))).collect()
}

fn task_enabled(name: &str) -> Option<bool> {
    let (ok, xml) = run("schtasks.exe", &["/query", "/tn", name, "/xml"]);
    if !ok {
        return None;
    }
    let settings = xml.split("<Settings>").nth(1)?.split("</Settings>").next()?;
    Some(!settings.contains("<Enabled>false</Enabled>"))
}

fn has_winget() -> bool {
    run("where.exe", &["winget"]).0
}

fn defender_paths() -> Vec<String> {
    let mut v = vec![];
    if let Some(home) = env::var_os("USERPROFILE").map(PathBuf::from) {
        for d in [".cargo", ".rustup", r".nuget\packages", r".gradle\caches", r".m2\repository"] {
            v.push(home.join(d));
        }
    }
    if let Some(la) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        v.push(la.join("npm-cache"));
        v.push(la.join(r"pip\cache"));
        v.push(la.join(r"Yarn\Cache"));
        v.push(la.join(r"pnpm\store"));
    }
    if let Some(pf) = env::var_os("ProgramFiles").map(PathBuf::from) {
        v.push(pf.join("Epic Games"));
    }
    v.extend(steam_libraries());
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|p| p.is_dir()).map(|p| p.display().to_string().replace('/', "\\")).filter(|p| seen.insert(p.to_lowercase())).collect()
}

fn steam_libraries() -> Vec<PathBuf> {
    let (_, out) = run("reg.exe", &["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"]);
    let Some(("REG_SZ", root)) = winrust_common::reg::value(&out) else { return vec![] };
    let vdf = fs::read_to_string(Path::new(root).join(r"steamapps\libraryfolders.vdf")).unwrap_or_default();
    let mut libs = vec![PathBuf::from(root)];
    libs.extend(parse_vdf_paths(&vdf).into_iter().map(PathBuf::from));
    libs.into_iter().map(|l| l.join(r"steamapps\common")).collect()
}

fn parse_vdf_paths(vdf: &str) -> Vec<String> {
    vdf.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('"').collect();
            (f.len() >= 4 && f[1] == "path").then(|| f[3].replace(r"\\", r"\"))
        })
        .collect()
}

fn defender_exclusions() -> Result<Vec<String>, String> {
    let (ok, out) = ps("$p=Get-MpPreference -ErrorAction Stop; $p.ExclusionPath; $p.ExclusionProcess");
    if !ok {
        return Err("Microsoft Defender is not active here (another antivirus?): nothing to exclude".into());
    }
    Ok(out.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect())
}

fn hosts_block() -> String {
    let mut s = format!("{HOSTS_BEGIN}\r\n");
    for d in BLOCK {
        s += &format!("0.0.0.0 {d}\r\n");
    }
    s + HOSTS_END + "\r\n"
}

fn strip_block(text: &str) -> String {
    let (Some(a), Some(b)) = (text.find(HOSTS_BEGIN), text.find(HOSTS_END)) else { return text.to_string() };
    if b < a {
        return text.to_string();
    }
    let mut end = b + HOSTS_END.len();
    let rest = &text[end..];
    end += if rest.starts_with("\r\n") { 2 } else { rest.starts_with('\n') as usize };
    let head = text[..a].trim_end_matches(['\r', '\n']);
    let tail = &text[end..];
    if head.is_empty() { tail.to_string() } else { format!("{head}\r\n{tail}") }
}

fn plan(g: Group) {
    println!("{}", "Dry run: nothing is changed. Add --apply (admin) to do it, --undo to revert.\n");
    let (svcs, regs): (Vec<(&str, &str)>, Vec<(&str, &str, u32, &str)>) = match g {
        Group::Base => (services(), regs()),
        Group::Updaters => (upd_services(), vec![]),
        Group::Ntfs => (vec![], ntfs_regs()),
        _ => (vec![], vec![]),
    };
    for (svc, why) in svcs {
        match start_mode(svc) {
            None => println!("{}", format!("  service {0:<18} not present", svc)),
            Some("disabled") => println!("{}", format!("  service {0:<18} already disabled", svc)),
            Some(m) => println!("{}", format!("  service {0:<18} {1} -> disabled   ({2})", svc, m, why)),
        }
    }
    for (key, value, data, why) in regs {
        let state = match reg_val(key, value) {
            Val::Dword(d) if d == data => "already set".to_string(),
            Val::Dword(d) => format!("{d} -> {data}"),
            Val::Missing => format!("(none) -> {data}"),
            Val::Other => "not a DWORD, left alone".into(),
        };
        println!("{}", format!("  policy  {0:<34} {1}   ({2})", value, state, why));
    }
    match g {
        Group::Updaters => {
            for t in updater_tasks() {
                println!("{}", format!("  task    {0}   -> disabled", t));
            }
            let winget = if has_winget() {
                "weekly winget upgrade --all"
            } else {
                "skipped: winget is not installed"
            };
            println!("{}", format!("  task    {0}   {1}", WINGET_TASK, winget));
        }
        Group::Defender => {
            for p in defender_paths() {
                println!("{}", format!("  exclude {0}", p));
            }
            for p in DEF_PROCS {
                println!("{}", format!("  exclude process {0}", p));
            }
        }
        Group::Dns => println!("{}", format!("  hosts   {0} ad/tracker domains -> 0.0.0.0", BLOCK.len())),
        _ => {}
    }
}

struct Log(File);

impl Log {
    fn line(&mut self, s: &str) -> Result<(), String> {
        writeln!(self.0, "{s}").and_then(|_| self.0.flush()).map_err(|e| e.to_string())
    }
}

fn done(ok: bool) -> &'static str {
    if ok {
        "done  "
    } else {
        "FAILED"
    }
}

fn apply_services(list: &[(&str, &str)], log: &mut Log) -> Result<u32, String> {
    let mut failed = 0;
    for (svc, _) in list {
        match start_mode(svc) {
            None => println!("{}", format!("skip   {0}: not present", svc)),
            Some("disabled") => println!("{}", format!("skip   {0}: already disabled", svc)),
            Some(mode) => {
                log.line(&format!("svc|{svc}|{mode}"))?;
                let ok = run("sc.exe", &["config", svc, "start=", "disabled"]).0;
                let _ = run("sc.exe", &["stop", svc]);
                failed += !ok as u32;
                println!("{}", format!("{0}  {1}: {2} -> disabled", done(ok), svc, mode));
            }
        }
    }
    Ok(failed)
}

fn apply_regs(list: &[(&str, &str, u32, &str)], log: &mut Log) -> Result<u32, String> {
    let mut failed = 0;
    for (key, value, data, _) in list {
        let old = match reg_val(key, value) {
            Val::Dword(d) if d == *data => {
                println!("{}", format!("skip   {value}: already {data}"));
                continue;
            }
            Val::Other => {
                println!("{}", format!("skip   {value}: not a DWORD, left alone"));
                continue;
            }
            Val::Dword(d) => d.to_string(),
            Val::Missing => "-".into(),
        };
        log.line(&format!("reg|{key}|{value}|{old}"))?;
        let ok = run("reg.exe", &["add", key, "/v", value, "/t", "REG_DWORD", "/d", &data.to_string(), "/f"]).0;
        failed += !ok as u32;
        println!("{}", format!("{0}  {value} = {data}", done(ok)));
    }
    Ok(failed)
}

fn apply_updaters(log: &mut Log) -> Result<u32, String> {
    let mut failed = apply_services(&upd_services(), log)?;
    for t in updater_tasks() {
        if task_enabled(&t) != Some(true) {
            println!("{}", format!("skip   {t}: already disabled"));
            continue;
        }
        log.line(&format!("task|{t}"))?;
        let ok = run("schtasks.exe", &["/change", "/tn", &t, "/disable"]).0;
        failed += !ok as u32;
        println!("{}", format!("{0}  {t}: disabled", done(ok)));
    }
    if !has_winget() {
        println!("{}", format!("skip   {WINGET_TASK}: winget is not installed (App Installer from the Store)"));
    } else if task_enabled(WINGET_TASK).is_some() {
        println!("{}", format!("skip   {WINGET_TASK}: already there"));
    } else {
        log.line(&format!("newtask|{WINGET_TASK}"))?;
        let user = env::var(winrust_common::reg::SID_ENV).ok().filter(|s| winrust_common::reg::valid_sid(s)).map_or("([Security.Principal.WindowsIdentity]::GetCurrent().Name)".to_string(), |s| psq(&s));
        let script = format!(
            "$a=New-ScheduledTaskAction -Execute 'conhost.exe' -Argument '--headless cmd.exe /c winget upgrade --all --silent --accept-source-agreements --accept-package-agreements --disable-interactivity'; \
             $t=New-ScheduledTaskTrigger -Weekly -DaysOfWeek Sunday -At 12:00; \
             $s=New-ScheduledTaskSettingsSet -StartWhenAvailable -RunOnlyIfNetworkAvailable -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 2); \
             $p=New-ScheduledTaskPrincipal -UserId {user} -LogonType Interactive -RunLevel Highest; \
             Register-ScheduledTask -TaskPath '\\WinRust\\' -TaskName 'Update apps' -Action $a -Trigger $t -Settings $s -Principal $p -Force -ErrorAction Stop | Out-Null"
        );
        let ok = ps(&script).0;
        failed += !ok as u32;
        println!("{}", format!("{0}  {WINGET_TASK}: weekly winget upgrade --all (Sunday 12:00, or at the next start)", done(ok)));
    }
    Ok(failed)
}

fn apply_defender(log: &mut Log) -> Result<u32, String> {
    let have = defender_exclusions()?;
    let paths: Vec<String> = defender_paths().into_iter().filter(|p| !have.contains(&p.to_lowercase())).collect();
    let procs: Vec<&str> = DEF_PROCS.iter().copied().filter(|p| !have.contains(&p.to_lowercase())).collect();
    if paths.is_empty() && procs.is_empty() {
        println!("{}", "skip   nothing new to exclude");
        return Ok(0);
    }
    for p in &paths {
        log.line(&format!("mpx|path|{p}"))?;
    }
    for p in &procs {
        log.line(&format!("mpx|proc|{p}"))?;
    }
    let list = |v: &[&str]| v.iter().map(|s| psq(s)).collect::<Vec<_>>().join(",");
    let mut cmd = "Add-MpPreference -ErrorAction Stop".to_string();
    if !paths.is_empty() {
        cmd += &format!(" -ExclusionPath {}", list(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
    }
    if !procs.is_empty() {
        cmd += &format!(" -ExclusionProcess {}", list(&procs));
    }
    let ok = ps(&cmd).0;
    for p in paths.iter().map(String::as_str).chain(procs.iter().copied()) {
        println!("{}", format!("{0}  exclude {1}", done(ok), p));
    }
    Ok(!ok as u32)
}

fn apply_dns(log: &mut Log) -> Result<u32, String> {
    let path = hosts_path();
    let text = fs::read_to_string(&path).unwrap_or_default();
    if text.contains(HOSTS_BEGIN) {
        println!("{}", "skip   hosts: the block is already there");
        return Ok(0);
    }
    log.line("hosts|block")?;
    let new = format!("{}\r\n\r\n{}", text.trim_end_matches(['\r', '\n']), hosts_block());
    let ok = fs::write(&path, new.trim_start_matches(['\r', '\n'])).is_ok();
    let _ = run("ipconfig.exe", &["/flushdns"]);
    println!("{}", format!("{0}  hosts: {1} domains blocked", done(ok), BLOCK.len()));
    Ok(!ok as u32)
}

fn apply(g: Group) -> Result<(), String> {
    let path = undo_path(g);
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e} (run as admin)", dir.display()))?;
    let f = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut log = Log(f);
    let failed = match g {
        Group::Base => apply_services(&services(), &mut log)? + apply_regs(&regs(), &mut log)?,
        Group::Updaters => apply_updaters(&mut log)?,
        Group::Defender => apply_defender(&mut log)?,
        Group::Ntfs => apply_regs(&ntfs_regs(), &mut log)?,
        Group::Dns => apply_dns(&mut log)?,
    };
    drop(log);
    if fs::metadata(&path).is_ok_and(|m| m.len() == 0) {
        let _ = fs::remove_file(&path);
    }
    println!("{}", format!("\nUndo information saved to {}. Some changes need a reboot or sign-out.", path.display()));
    if failed > 0 { Err(format!("{failed} change(s) failed (run as administrator)")) } else { Ok(()) }
}

fn restored(ok: bool) -> &'static str {
    if ok {
        "restored"
    } else {
        "FAILED  "
    }
}

fn undo(g: Group) -> Result<(), String> {
    let path = undo_path(g);
    let Ok(text) = fs::read_to_string(&path) else {
        println!("{}", "nothing to undo (no undo file)");
        return Ok(());
    };
    let mut failed = 0;
    for line in text.lines().rev() {
        let f: Vec<&str> = line.splitn(4, '|').collect();
        let ok = match f.as_slice() {
            ["svc", name, mode] => run("sc.exe", &["config", name, "start=", mode]).0,
            ["reg", key, value, "-"] => run("reg.exe", &["delete", key, "/v", value, "/f"]).0 || reg_val(key, value) == Val::Missing,
            ["reg", key, value, old] => run("reg.exe", &["add", key, "/v", value, "/t", "REG_DWORD", "/d", old, "/f"]).0,
            ["task", name] => run("schtasks.exe", &["/change", "/tn", name, "/enable"]).0 || task_enabled(name).is_none(),
            ["newtask", name] => run("schtasks.exe", &["/delete", "/tn", name, "/f"]).0 || task_enabled(name).is_none(),
            ["mpx", "path", p] => ps(&format!("Remove-MpPreference -ErrorAction Stop -ExclusionPath {}", psq(p))).0,
            ["mpx", "proc", p] => ps(&format!("Remove-MpPreference -ErrorAction Stop -ExclusionProcess {}", psq(p))).0,
            ["hosts", "block"] => {
                let p = hosts_path();
                let ok = fs::read_to_string(&p).is_ok_and(|t| fs::write(&p, strip_block(&t)).is_ok());
                let _ = run("ipconfig.exe", &["/flushdns"]);
                ok
            }
            _ => continue,
        };
        failed += !ok as u32;
        println!("{}", format!("{0}  {1}", restored(ok), line));
    }
    if failed > 0 {
        return Err(format!("{failed} item(s) not restored; the undo file is kept at {}", path.display()));
    }
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    println!("{}", "\nRestored (services start again after a reboot or by hand).");
    Ok(())
}

fn main() {
    winrust_common::utf8_console();
    let args: Vec<String> = env::args().skip(1).collect();
    let group = match args.iter().find(|a| !a.starts_with("--")) {
        None => Group::Base,
        Some(g) => match Group::parse(g) {
            Some(g) => g,
            None => {
                eprintln!("{}", format!("unknown group '{g}' (updaters, defender, ntfs, dns)"));
                std::process::exit(2);
            }
        },
    };
    let has = |f: &str| args.iter().any(|a| a == f);
    let r = if has("--status") {
        let on = undo_path(group).is_file();
        println!("{}", if on { "on" } else { "off" });
        std::process::exit(!on as i32);
    } else if has("--apply") {
        apply(group)
    } else if has("--undo") {
        undo(group)
    } else {
        plan(group);
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
    fn reads_reg_query_output() {
        let dw = "\r\nHKEY_LOCAL_MACHINE\\X\r\n    AllowTelemetry    REG_DWORD    0x1\r\n\r\n";
        assert_eq!(parse_val(true, dw), Val::Dword(1));
        assert_eq!(parse_val(true, "    V    REG_SZ    hello\r\n"), Val::Other);
        assert_eq!(parse_val(false, "ERROR: The system was unable to find the specified registry key or value."), Val::Missing);
        assert_eq!(parse_val(true, "    NtfsDisableLastAccessUpdate    REG_DWORD    0x80000002\r\n"), Val::Dword(0x8000_0002));
    }

    #[test]
    fn every_registry_item_is_hkcu_or_hklm_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (k, v, _, _) in regs().into_iter().chain(ntfs_regs()) {
            assert!(k.starts_with("HKLM\\") || k.starts_with("HKCU\\"), "{k}");
            assert!(seen.insert((k, v)), "duplicate {k} {v}");
        }
    }

    #[test]
    fn task_names_from_schtasks_csv() {
        let csv = "\r\n\"\\Adobe Acrobat Update Task\",\"21.11.2026 10:00:00\",\"Готово\"\r\n\"\\Mozilla\\Firefox Background Update 308046B0AF4A39CB\",\"N/A\",\"Ready\"\r\n\"\\Mozilla\\Firefox Background Update 308046B0AF4A39CB\",\"N/A\",\"Ready\"\r\nINFO: x\r\n";
        assert_eq!(parse_task_names(csv), vec![r"\Adobe Acrobat Update Task", r"\Mozilla\Firefox Background Update 308046B0AF4A39CB"]);
    }

    #[test]
    fn steam_library_paths() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(parse_vdf_paths(vdf), vec![r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]);
    }

    #[test]
    fn hosts_block_round_trips() {
        let orig = "# Copyright\r\n127.0.0.1 localhost\r\n";
        let with = format!("{}\r\n\r\n{}", orig.trim_end(), hosts_block());
        assert_eq!(strip_block(&with), "# Copyright\r\n127.0.0.1 localhost\r\n");
        let with_tail = format!("{with}10.0.0.1 nas\r\n");
        assert_eq!(strip_block(&with_tail), "# Copyright\r\n127.0.0.1 localhost\r\n10.0.0.1 nas\r\n");
        assert_eq!(strip_block(&hosts_block()), "");
        assert_eq!(strip_block(orig), orig);
    }

    #[test]
    fn block_list_is_clean() {
        let mut seen = std::collections::HashSet::new();
        for d in BLOCK {
            assert!(seen.insert(*d), "duplicate {d}");
            assert!(!d.contains(' ') && d.contains('.'), "{d}");
            for ms in ["microsoft", "windows", "msn.com", "live.com", "bing.com", "office", "skype", "xbox"] {
                assert!(!d.contains(ms), "Microsoft domain {d}: Defender flags it");
            }
        }
    }

    #[test]
    fn groups_parse_and_have_own_files() {
        let gs = ["updaters", "defender", "ntfs", "dns"].map(|g| Group::parse(g).unwrap());
        let files: std::collections::HashSet<_> = gs.iter().map(|g| g.file()).chain([Group::Base.file()]).collect();
        assert_eq!(files.len(), 5);
        assert_eq!(Group::parse("x"), None);
    }
}
