use std::{fs, os::windows::process::CommandExt, path::PathBuf, process::Command};
use windows_sys::Win32::Graphics::Gdi::*;

const NO_WINDOW: u32 = 0x0800_0000;

const NAME: &str = "SuperSaver";
const SAVER: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";
const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
const BRIGHTNESS: &str = "aded5e82-b909-4619-9949-f5d71dac0bcb";
const SUB_VIDEO: &str = "7516b95f-f776-4464-8c53-06167f40cc99";
const LAUNCHERS: &[&str] = &["steam.exe", "steamwebhelper.exe", "Discord.exe", "EpicGamesLauncher.exe", "Battle.net.exe", "RiotClientServices.exe", "GalaxyClient.exe", "EADesktop.exe", "Origin.exe"];

fn hz_file() -> PathBuf {
    let dir = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("reshell");
    let _ = fs::create_dir_all(&dir);
    dir.join("saver_hz.txt")
}

fn devmode() -> DEVMODEA {
    let mut dm: DEVMODEA = unsafe { std::mem::zeroed() };
    dm.dmSize = size_of::<DEVMODEA>() as u16;
    unsafe { EnumDisplaySettingsA(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut dm) };
    dm
}

fn get_hz() -> u32 { devmode().dmDisplayFrequency }

fn set_hz(hz: u32) -> bool {
    let mut dm = devmode();
    dm.dmDisplayFrequency = hz;
    dm.dmFields = DM_DISPLAYFREQUENCY;
    unsafe { ChangeDisplaySettingsA(&dm, CDS_TEST) == DISP_CHANGE_SUCCESSFUL && ChangeDisplaySettingsA(&dm, 0) == DISP_CHANGE_SUCCESSFUL }
}

fn powercfg(args: &[&str]) -> String {
    Command::new("powercfg").args(args).creation_flags(NO_WINDOW).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn find_scheme() -> Option<String> {
    powercfg(&["/l"]).lines().find(|l| l.contains(NAME)).and_then(guid_in)
}

fn guid_in(s: &str) -> Option<String> {
    let i = s.find("GUID:")? + 5;
    s[i..].split_whitespace().next().map(str::to_owned)
}

pub fn run(on: bool) -> i32 {
    let existing = find_scheme();
    let hz_path = hz_file();
    if !on {
        powercfg(&["/setactive", BALANCED]);
        if let Some(g) = existing { powercfg(&["/d", &g]); }
        if let Some(hz) = fs::read_to_string(&hz_path).ok().and_then(|s| s.trim().parse().ok()) {
            if set_hz(hz) { let _ = fs::remove_file(&hz_path); }
        }
        return 0;
    }
    let cur = get_hz();
    if cur != 60 {
        if !hz_path.exists() { let _ = fs::write(&hz_path, cur.to_string()); }
        if !set_hz(60) { eprintln!("60 Hz not supported on this display"); }
    }
    let _ = winrust_common::proc::kill_and_wait(LAUNCHERS, 2000);

    let g = match existing.or_else(|| guid_in(&powercfg(&["/duplicatescheme", SAVER]))) {
        Some(g) => g,
        None => { eprintln!("could not create power scheme"); return 1; }
    };
    powercfg(&["/changename", &g, NAME, "Max battery life"]);
    for (sub, set, v) in [
        ("SUB_PROCESSOR", "PROCTHROTTLEMAX", "40"), ("SUB_PROCESSOR", "PERFBOOSTMODE", "0"), ("SUB_PROCESSOR", "SYSCOOLPOL", "0"),
        ("SUB_VIDEO", "VIDEOIDLE", "60"), ("SUB_SLEEP", "STANDBYIDLE", "600"), (SUB_VIDEO, BRIGHTNESS, "20"),
    ] {
        powercfg(&["/setdcvalueindex", &g, sub, set, v]);
    }
    powercfg(&["/setactive", &g]);
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_guid() {
        assert_eq!(super::guid_in("Power Scheme GUID: abc-123  (SuperSaver)").as_deref(), Some("abc-123"));
        assert_eq!(super::guid_in("nothing"), None);
    }
}
