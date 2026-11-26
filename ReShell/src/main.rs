mod hotkeys;
mod saver;
mod watchdog;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["saver", m @ ("on" | "off")] => saver::run(*m == "on"),
        ["hotkeys"] | [] => {
            unsafe {
                use windows_sys::Win32::{Foundation::{ERROR_ALREADY_EXISTS, GetLastError}, System::Threading::CreateMutexW};
                let name: Vec<u16> = "Local\\reshell-hotkeys\0".encode_utf16().collect();
                CreateMutexW(std::ptr::null(), 0, name.as_ptr());
                if GetLastError() == ERROR_ALREADY_EXISTS { std::process::exit(0); }
            }
            unsafe { windows_sys::Win32::System::Console::FreeConsole() };
            std::thread::spawn(watchdog::run);
            hotkeys::run()
        }
        _ => {
            eprintln!("usage: reshell saver on|off | hotkeys (default)");
            2
        }
    };
    std::process::exit(code);
}
