#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]
use winrust_common as i18n;
mod cli;
mod install;
mod selection;
mod ui;
use {cli::*, install::*, selection::*, ui::*};
use std::{
    cell::Cell,
    env, fs,
    io::Write,
    mem::{size_of, zeroed},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{
        Dwm::DwmSetWindowAttribute,
        Gdi::*,
        GdiPlus::{
            FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen,
            GdipDrawPath, GdipFillPath, GdipFillRectangle, GdipSetPenColor, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdipSetSolidFillColor, GdiplusStartup, GdiplusStartupInput, GpBrush, GpGraphics,
            GpPath, GpPen, GpSolidFill, PixelOffsetModeHalf, SmoothingModeAntiAlias, UnitPixel,
        },
    },
    System::{Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE}, LibraryLoader::GetModuleHandleW},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const BG: u32 = rgb(32, 32, 32);
const PANEL: u32 = rgb(40, 40, 40);
const BORDER: u32 = rgb(62, 62, 62);
const TEXT: u32 = rgb(240, 240, 240);
const DIM: u32 = rgb(160, 160, 160);
const ACCENT: u32 = rgb(118, 185, 0);
const GOOD: u32 = winrust_common::style::GOOD;
const BAD: u32 = winrust_common::style::BAD;
const WM_DONE: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3;

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

struct Plugin {
    name: &'static str,
    desc: &'static str,
    warn: &'static str,
    installed: fn() -> bool,
    actions: Option<(fn() -> Result<(), String>, fn() -> Result<(), String>)>,
    settings: bool,
    bulk: bool,
    glyph: &'static str,
}
const PLUGINS: &[Plugin] = &[
    Plugin {
        name: "rust-search",
        desc: "Fast NTFS file search (Alt+Space). Replaces Windows Search: the WSearch service is disabled and SearchHost is blocked.",
        warn: "The Windows Search service (WSearch) will be DISABLED, SearchHost.exe blocked and the taskbar search box hidden: search in Start, Explorer and Outlook will stop working.",
        installed: || prog_dir().join("rs.exe").is_file(),
        actions: Some((install, uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E721}",
    },
    Plugin {
        name: "rtaskmgr-win32",
        desc: "A task manager in Rust. Replaces the stock one: Ctrl+Shift+Esc and launching taskmgr.exe open it.",
        warn: "The stock Task Manager will be REPLACED: any launch of taskmgr.exe opens rtaskmgr-win32.",
        installed: tm_installed,
        actions: Some((tm_install, tm_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E9D9}",
    },
    Plugin {
        name: "topbar",
        desc: "A Windows 11-style bottom bar instead of the taskbar: windows, clock, menus. The stock taskbar is hidden. Desktop hotkeys (Win+1/2/3, Win+`) are installed with it.",
        warn: "The Windows taskbar will be HIDDEN and replaced by a bar at the bottom of the screen. Uninstalling the plugin brings it back.",
        installed: || bar_dir().join("topbar.exe").is_file(),
        actions: Some((bar_install, bar_uninstall)),
        settings: true,
        bulk: true,
        glyph: "\u{E700}",
    },
    Plugin {
        name: RUST_EXPLORER,
        desc: "A Rust file manager instead of Explorer: folders and drives open in it (Win+E too), and it draws the desktop itself. The regular explorer.exe as the shell is DISABLED.",
        warn: "The REGULAR Explorer will be DISABLED: after install, rust-explorer provides the desktop and folder windows. To save RAM the services InventorySvc and lfsvc (geolocation) are also disabled, DoSvc is set to manual, and svchost services are merged (fully after a reboot); uninstalling the plugin restores them. If something goes wrong: Ctrl+Alt+Del, Task Manager, Run new task, explorer.exe; then uninstall the plugin.",
        installed: || rx_dir().join("rust-explorer.exe").is_file(),
        actions: Some((rx_install, rx_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E8B7}",
    },
    Plugin {
        name: "photoview",
        desc: "Photo viewer: opens instantly, browses the folder with the arrow keys, honors EXIF rotation. Appears in 'Open with' (not set as the default).",
        warn: "The program is added to 'Open with' for jpg, png, bmp, gif, tiff, webp, heic, avif. You make it the default app yourself in Settings.",
        installed: || pv_dir().join("photoview.exe").is_file(),
        actions: Some((pv_install, pv_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E91B}",
    },
    Plugin {
        name: "rust-settings",
        desc: "Settings in Rust: display (resolution, refresh rate), sound (volume), theme, about. Win+I and the bar menus open it instead of Windows Settings, so Explorer is not needed. Other pages open the system ones.",
        warn: "Nothing in the system is changed: a file in Program Files and a 'WinRust Settings' Start menu shortcut. Win+I starts opening this window; uninstall the plugin and it goes back.",
        installed: || st_dir().join("rust-settings.exe").is_file(),
        actions: Some((st_install, st_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E713}",
    },
    Plugin {
        name: "debloat",
        desc: "Disables Windows junk: telemetry, Copilot, Recall, Start menu ads, the Edge background. Reversible changes only, old values are remembered.",
        warn: "Several services will be DISABLED (telemetry, maps, error reporting, diagnostics and troubleshooters, AI Fabric) and registry policies set (Copilot, Recall, ads, Edge background; Edge will show 'managed by your organization'). Old values are saved, uninstall debloat restores everything. No apps are removed.",
        installed: || db_dir().join("debloat.exe").is_file(),
        actions: Some((db_install, db_uninstall)),
        settings: false,
        bulk: false,
        glyph: "\u{EA18}",
    },
    Plugin {
        name: "purge",
        desc: "PERMANENTLY removes the bundled junk like Tiny11 does: Clipchamp, News, Weather, Xbox overlays, Teams, Cortana, Widgets, Skype, Solitaire and more, plus telemetry tasks. There is no undo.",
        warn: "IRREVERSIBLE: apps are removed for all users and from the image (new users will not get them), telemetry tasks are deleted. They come back only from the Store or winget. The Store, Settings, Start, Calculator, Photos and runtimes are not touched. OneDrive and Edge are not included: purge.exe --apply --onedrive --edge. Look at the list first: purge.exe with no arguments changes nothing.",
        installed: || pu_dir().join("purge.exe").is_file(),
        actions: Some((pu_install, pu_uninstall)),
        settings: false,
        bulk: false,
        glyph: "\u{E74D}",
    },
];

static BUSY: AtomicBool = AtomicBool::new(false);
static JOB: AtomicUsize = AtomicUsize::new(usize::MAX);
static INSTALLED: [AtomicBool; PLUGINS.len()] = [const { AtomicBool::new(false) }; PLUGINS.len()];
fn refresh() {
    for (p, f) in PLUGINS.iter().zip(&INSTALLED) {
        f.store((p.installed)(), Relaxed);
    }
}
static SELECTED: [AtomicBool; PLUGINS.len()] = [const { AtomicBool::new(false) }; PLUGINS.len()];
static MSG: Mutex<(String, bool)> = Mutex::new((String::new(), true));
thread_local! { static SETTINGS: Cell<Option<usize>> = const { Cell::new(None) }; }
thread_local! { static HOVER: Cell<Option<usize>> = const { Cell::new(None) }; }

fn elevate_self(args: &[String]) -> bool {
    if winrust_common::proc::elevated() || env::var_os("WINRUST_ELEVATE_TRIED").is_some() {
        return false;
    }
    unsafe { env::set_var("WINRUST_ELEVATE_TRIED", "1") };
    let Ok(exe) = env::current_exe() else { return false };
    let params = args.iter().map(|a| {
        if a.contains(' ') || a.contains('\t') || a.contains('"') {
            format!("\"{}\"", a.replace('"', "\\\""))
        } else {
            a.clone()
        }
    }).collect::<Vec<_>>().join(" ");
    let params_w = (!params.is_empty()).then(|| wide(&params));
    let dir_w = exe.parent().map(|p| wide(&p.to_string_lossy()));
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut core::ffi::c_void, coinit: u32) -> i32;
    }
    let r = unsafe {
        CoInitializeEx(null_mut(), 0x2 | 0x4);
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            null_mut(),
            wide("runas").as_ptr(),
            wide(&exe.to_string_lossy()).as_ptr(),
            params_w.as_ref().map_or(null(), |w| w.as_ptr()),
            dir_w.as_ref().map_or(null(), |w| w.as_ptr()),
            SW_SHOWNORMAL,
        ) as isize
    };
    r > 32
}

fn main() {
    winrust_common::install_panic_hook("installer");
    let args: Vec<String> = env::args().skip(1).collect();
    if elevate_self(&args) {
        return;
    }
    if winrust_common::proc::elevated() {
        winrust_common::proc::enable_debug_privilege();
        unsafe {
            windows_sys::Win32::System::Threading::SetPriorityClass(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                windows_sys::Win32::System::Threading::HIGH_PRIORITY_CLASS,
            );
        }
        if args.is_empty() || matches!(args.first().map(String::as_str), Some("install" | "update")) {
            override_windhawk();
        }
    }
    if !args.is_empty() {
        std::process::exit(cli(&args));
    }
    refresh();
    reset_selection();
    unsafe {
        SetProcessDPIAware();
        winrust_common::ui::ensure_fluent_icons();
        let (mut token, mut input): (usize, GdiplusStartupInput) = (0, zeroed());
        input.GdiplusVersion = 1;
        GdiplusStartup(&mut token, &input, null_mut());
        let hi = GetModuleHandleW(null());
        let cls = wide("WinRust");
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: cls.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);
        let s = ui::scale();
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = RECT { left: 0, top: 0, right: (WIN_W * s).round() as i32, bottom: (WIN_H * s).round() as i32 };
        AdjustWindowRect(&mut r, style, 0);
        let mut wa: RECT = zeroed();
        let has_wa = SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as *mut _, 0) != 0;
        let (wa_l, wa_t, wa_w, wa_h) = if has_wa {
            (wa.left, wa.top, (wa.right - wa.left).max(400), (wa.bottom - wa.top).max(320))
        } else {
            (0, 0, GetSystemMetrics(SM_CXSCREEN).max(400), GetSystemMetrics(SM_CYSCREEN).max(320))
        };
        let win_w = (r.right - r.left).min(wa_w);
        let win_h = (r.bottom - r.top).min(wa_h);
        let x = wa_l + ((wa_w - win_w) / 2).max(0);
        let y = wa_t + ((wa_h - win_h) / 2).max(0);
        let title = wide("WinRust");
        let hwnd = CreateWindowExW(0, cls.as_ptr(), title.as_ptr(), style | WS_VISIBLE, x, y, win_w, win_h, null_mut(), null_mut(), hi, null());
        DwmSetWindowAttribute(hwnd, 20, &1i32 as *const i32 as *const _, 4);
        DwmSetWindowAttribute(hwnd, 33, &2i32 as *const i32 as *const _, 4);
        DwmSetWindowAttribute(hwnd, 35, &ui::HEAD as *const u32 as *const _, 4);
        let mut m: MSG = zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}
