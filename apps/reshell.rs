#![windows_subsystem = "windows"]
#[path = "reshell_hotkeys.rs"]
mod hotkeys;
#[path = "reshell_saver.rs"]
mod saver;
#[path = "reshell_watchdog.rs"]
mod watchdog;

fn attach_console() {
    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let code = match (args.next().as_deref(), args.next().as_deref(), args.next()) {
        (Some("saver"), Some(m @ ("on" | "off")), None) => {
            attach_console();
            saver::run(m == "on")
        }
        (Some("hotkeys") | None, None, None) => {
            unsafe {
                use windows_sys::Win32::{Foundation::{ERROR_ALREADY_EXISTS, GetLastError}, System::Threading::CreateMutexW};
                const NAME: [u16; 22] = [
                    b'L' as u16, b'o' as u16, b'c' as u16, b'a' as u16, b'l' as u16, b'\\' as u16,
                    b'r' as u16, b'e' as u16, b's' as u16, b'h' as u16, b'e' as u16, b'l' as u16, b'l' as u16, b'-' as u16,
                    b'h' as u16, b'o' as u16, b't' as u16, b'k' as u16, b'e' as u16, b'y' as u16, b's' as u16, 0,
                ];
                CreateMutexW(std::ptr::null(), 0, NAME.as_ptr());
                if GetLastError() == ERROR_ALREADY_EXISTS { std::process::exit(0); }
            }
            std::thread::spawn(watchdog::run);
            hotkeys::run()
        }
        _ => {
            attach_console();
            eprintln!("usage: reshell saver on|off | hotkeys (default)");
            2
        }
    };
    std::process::exit(code);
}
