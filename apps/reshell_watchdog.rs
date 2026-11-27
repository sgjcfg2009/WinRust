use std::{os::windows::process::CommandExt, process::Command, thread, time::{Duration, Instant}};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{INFINITE, OpenProcess, WaitForSingleObject},
    UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, IsWindowVisible},
};

const SYNCHRONIZE: u32 = 0x0010_0000;
const TOPBAR: [u16; 7] = [b't' as u16, b'o' as u16, b'p' as u16, b'b' as u16, b'a' as u16, b'r' as u16, 0];
const TRAY: [u16; 14] = [b'S' as u16, b'h' as u16, b'e' as u16, b'l' as u16, b'l' as u16, b'_' as u16, b'T' as u16, b'r' as u16, b'a' as u16, b'y' as u16, b'W' as u16, b'n' as u16, b'd' as u16, 0];

const MAX_RESTARTS: u32 = 3;
const STABLE_CHECKS: u32 = 12;

fn broken(bar_up: bool, tray_present: bool, tray_visible: bool) -> bool {
    !bar_up && tray_present && !tray_visible
}

#[derive(Debug, PartialEq, Eq)]
enum Act {
    Nothing,
    Recover,
}

#[derive(Default)]
struct Guard {
    strikes: u32,
    restarts: u32,
    stable: u32,
}

impl Guard {
    fn step(&mut self, bar_up: bool, broke: bool) -> Act {
        self.stable = if bar_up { self.stable + 1 } else { 0 };
        if self.stable >= STABLE_CHECKS {
            self.restarts = 0;
        }
        self.strikes = if broke { self.strikes + 1 } else { 0 };
        if self.strikes == 2 && self.restarts < MAX_RESTARTS {
            self.restarts += 1;
            self.strikes = 0;
            return Act::Recover;
        }
        Act::Nothing
    }
}

pub fn run() {
    let mut g = Guard::default();
    loop {
        let bar = unsafe { FindWindowW(TOPBAR.as_ptr(), std::ptr::null()) };
        if !bar.is_null() {
            g.strikes = 0;
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(bar, &mut pid) };
            if pid != 0 {
                let h = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
                if !h.is_null() {
                    let up_since = Instant::now();
                    unsafe {
                        WaitForSingleObject(h, INFINITE);
                        CloseHandle(h);
                    }
                    if up_since.elapsed() >= Duration::from_secs(60) {
                        g.restarts = 0;
                    }
                    continue;
                }
            }
            thread::sleep(Duration::from_secs(5));
            g.step(true, false);
            continue;
        }
        thread::sleep(Duration::from_secs(5));
        let bar = unsafe { FindWindowW(TOPBAR.as_ptr(), std::ptr::null()) };
        let tray = unsafe { FindWindowW(TRAY.as_ptr(), std::ptr::null()) };
        let broke = broken(!bar.is_null(), !tray.is_null(), !tray.is_null() && unsafe { IsWindowVisible(tray) } != 0);
        if g.step(!bar.is_null(), broke) == Act::Recover {
            if let Ok(exe) = std::env::current_exe() {
                let topbar = exe.with_file_name("topbar.exe");
                let _ = Command::new(&topbar).arg("--restore").creation_flags(0x0800_0000).status();
                let _ = Command::new(&topbar).creation_flags(0x0800_0000).spawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_bar_gone_and_taskbar_hidden_is_broken() {
        assert!(broken(false, true, false));
        assert!(!broken(true, true, false));
        assert!(!broken(false, true, true));
        assert!(!broken(false, false, false));
    }

    #[test]
    fn recovers_after_two_broken_checks() {
        let mut g = Guard::default();
        assert_eq!(g.step(false, true), Act::Nothing);
        assert_eq!(g.step(false, true), Act::Recover);
    }

    #[test]
    fn a_bar_that_keeps_crashing_is_given_up() {
        let mut g = Guard::default();
        let mut recovered = 0;
        for _ in 0..40 {
            if g.step(false, true) == Act::Recover {
                recovered += 1;
            }
        }
        assert_eq!(recovered, MAX_RESTARTS);
    }

    #[test]
    fn a_stable_bar_gives_the_tries_back() {
        let mut g = Guard::default();
        for _ in 0..4 {
            g.step(false, true);
        }
        assert_eq!(g.restarts, 2);
        for _ in 0..STABLE_CHECKS {
            g.step(true, false);
        }
        assert_eq!(g.restarts, 0);
    }
}
