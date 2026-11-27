#![windows_subsystem = "windows"]
extern crate winrust_common;
#[path = "rtaskmgr_app.rs"]
mod app;
#[path = "rtaskmgr_data.rs"]
mod data;
#[path = "rtaskmgr_draw.rs"]
mod draw;
#[path = "rtaskmgr_gpu.rs"]
mod gpu;
#[path = "rtaskmgr_hw.rs"]
mod hw;
#[path = "rtaskmgr_model.rs"]
mod model;
#[path = "rtaskmgr_net.rs"]
mod net;
#[path = "rtaskmgr_perf.rs"]
mod perf;
#[path = "rtaskmgr_res.rs"]
mod res;
#[path = "rtaskmgr_startup.rs"]
mod startup;
#[path = "rtaskmgr_store.rs"]
mod store;
#[path = "rtaskmgr_sys.rs"]
mod sys;
#[path = "rtaskmgr_ui.rs"]
mod ui;

use app::*;
use data::*;
use model::*;
use native_windows_gui as nwg;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};
use ui::*;
use winapi::um::winuser::*;

fn net_selftest(out: &str) {
    use std::io::{Read, Write};
    let report = (|| -> Result<String, String> {
        let ses = net::start().ok_or("could not start the ETW session (administrator rights needed)")?;
        std::thread::sleep(Duration::from_millis(500));
        let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let addr = l.local_addr().map_err(|e| e.to_string())?;
        let srv = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 100_000];
            let mut got = 0;
            while got < 200_000 {
                match c.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got += n,
                }
            }
        });
        let mut c = std::net::TcpStream::connect(addr).map_err(|e| e.to_string())?;
        c.write_all(&vec![7u8; 200_000]).map_err(|e| e.to_string())?;
        drop(c);
        let _ = srv.join();
        std::thread::sleep(Duration::from_secs(2));
        let me = std::process::id();
        let all = net::drain();
        net::stop(ses);
        let (rx, tx) = all.get(&me).copied().unwrap_or((0, 0));
        Ok(format!("pid={me} received={rx} sent={tx} processes_in_data={}", all.len()))
    })();
    let _ = std::fs::write(out, report.unwrap_or_else(|e| format!("ERROR: {e}")));
}

struct MutexGuard(winapi::um::winnt::HANDLE);
impl Drop for MutexGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { winapi::um::handleapi::CloseHandle(self.0) };
        }
    }
}

fn focus_existing_taskmgr() {
    let title: Vec<u16> = "Task Manager".encode_utf16().chain([0]).collect();
    let h = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if !h.is_null() {
        unsafe {
            ShowWindow(h, SW_SHOW);
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            SetForegroundWindow(h);
        }
        winrust_common::fg::force(h as _);
    }
}

fn main() {
    if let Some(i) = std::env::args().position(|a| a == "--net-selftest")
        && let Some(out) = std::env::args().nth(i + 1) {
            return net_selftest(&out);
        }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--undo-speed") {
        return;
    }
    if args.first().is_some_and(|a| a == "--real-taskmgr") {
        sys::start_real_taskmgr(&args[1..].join(" "), false);
        return;
    }
    let mtx_name: Vec<u16> = "Local\\rtaskmgr-single".encode_utf16().chain([0]).collect();
    unsafe {
        let existing = winapi::um::synchapi::OpenMutexW(0x0010_0000 , 0, mtx_name.as_ptr());
        if !existing.is_null() {
            winapi::um::handleapi::CloseHandle(existing);
            focus_existing_taskmgr();
            return;
        }
    }
    let tray_flag = store::dir().join("tray.flag");
    if !sys::is_admin() {
        if args.iter().any(|a| a == "--tray") {
            let _ = std::fs::create_dir_all(store::dir());
            let _ = std::fs::write(&tray_flag, "");
        }
        if sys::run_elevated_task() {
            return;
        }
        let _ = std::fs::remove_file(&tray_flag);
    }
    let _single_guard = MutexGuard(unsafe {
        let m = winapi::um::synchapi::CreateMutexW(std::ptr::null_mut(), 0, mtx_name.as_ptr());
        if m.is_null() || winapi::um::errhandlingapi::GetLastError() == 183 {
            if !m.is_null() {
                winapi::um::handleapi::CloseHandle(m);
            }
            focus_existing_taskmgr();
            return;
        }
        m
    });
    let ifeo = sys::taskmgr_args(&args);
    let fallback_args = ifeo.clone().unwrap_or_default();
    std::panic::set_hook(Box::new(move |info| {
        winrust_common::log("rtaskmgr", &format!("PANIC: {info}\n{}", std::backtrace::Backtrace::force_capture()));
        sys::start_real_taskmgr(&fallback_args, true);
    }));
    let hidden = args.iter().any(|a| a == "--tray") || (sys::is_admin() && std::fs::remove_file(&tray_flag).is_ok());
    nwg::enable_visual_styles();
    nwg::init().expect("не удалось инициализировать Win32");
    winrust_common::ui::ensure_fluent_icons();
    let scale = nwg::scale_factor();
    let mut font = nwg::Font::default();
    nwg::Font::builder().family("Segoe UI").size(17).build(&mut font).unwrap();
    nwg::Font::set_global_default(Some(font));

    store::trim_history(10_080);
    let cfg = store::load();
    let startup_asked = ifeo.as_deref().is_some_and(|a| a.to_lowercase().contains("/startup"));
    let page = if startup_asked { Page::Startup } else { NAV.get(cfg.view).map_or(Page::Processes, |n| n.0) };

    let embed = nwg::EmbedResource::load(None).ok();
    let load_icon = |out: &mut nwg::Icon, size: (u32, u32)| {
        let ok = embed.as_ref().is_some_and(|e| nwg::Icon::builder().source_embed(Some(e)).source_embed_id(1).size(Some(size)).build(out).is_ok());
        if !ok {
            nwg::Icon::builder().source_system(Some(nwg::OemIcon::Information)).build(out).unwrap();
        }
    };
    let (mut icon_big, mut tray_icon) = (nwg::Icon::default(), nwg::Icon::default());
    load_icon(&mut icon_big, (32, 32));
    load_icon(&mut tray_icon, (16, 16));

    let mut win = nwg::Window::default();
    let wa = winrust_common::ui::work_area();
    let (ww, wh) = ((wa.right - wa.left).max(600), (wa.bottom - wa.top).max(400));
    let win_w = (cfg.size.0 as i32).clamp(((600.0 * scale) as i32).min(ww), (ww - 20).max(600));
    let win_h = (cfg.size.1 as i32).clamp(((400.0 * scale) as i32).min(wh), (wh - 40).max(400));
    let win_x = cfg.pos.0.clamp(wa.left, (wa.right - win_w).max(wa.left));
    let win_y = cfg.pos.1.clamp(wa.top, (wa.bottom - win_h).max(wa.top));
    nwg::Window::builder()
        .flags(if hidden { nwg::WindowFlags::MAIN_WINDOW } else { nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::VISIBLE })
        .icon(Some(&icon_big))
        .size((win_w, win_h))
        .position((win_x, win_y))
        .title("Task Manager")
        .build(&mut win)
        .unwrap();
    let mut search = nwg::TextInput::default();
    nwg::TextInput::builder()
        .parent(&win)
        .placeholder_text(Some("Type a name, publisher or PID to search"))
        .flags(nwg::TextInputFlags::VISIBLE | nwg::TextInputFlags::TAB_STOP)
        .position((0, 0))
        .size((300, 20))
        .build(&mut search)
        .unwrap();
    #[allow(deprecated)]
    let mut timer = nwg::Timer::default();
    #[allow(deprecated)]
    nwg::Timer::builder().parent(&win).interval(1000).stopped(false).build(&mut timer).unwrap();
    let mut tray = nwg::TrayNotification::default();
    let _ = nwg::TrayNotification::builder().parent(&win).icon(Some(&tray_icon)).tip(Some("Task Manager (Rust)")).build(&mut tray);

    let ui = Ui {
        page,
        nav_open: win_w >= (820.0 * scale) as i32,
        th: theme(true),
        rows: vec![],
        sel: None,
        expanded: HashSet::new(),
        sorts: HashMap::from([(Page::Processes, (cfg.sort.min(6), cfg.desc))]),
        search: String::new(),
        hit: Hit::None,
        mouse: (0, 0),
        scroll: 0.0,
        drag: None,
        perf_split: None,
        perf_drag: false,
        perf_sel: draw::PerfKind::Cpu,
        toast: None,
        lay: None,
        last_click: None,
    };
    let app = Rc::new(App {
        win,
        search,
        _timer: timer,
        tray,
        _tray_icon: tray_icon,
        _icon_big: icon_big,
        data: RefCell::new(Data::new(cfg.clone())),
        ui: RefCell::new(ui),
        fonts: RefCell::new(Fonts::new(scale)),
        scale: Cell::new(scale),
        edit_brush: Cell::new(0),
        mem_dc: Cell::new(0),
        mem_bmp: Cell::new((0, 0, 0)),
        mem_old_bmp: Cell::new(0),
        search_rect: Cell::new(R::new(0, 0, 0, 0)),
        last_tick: Cell::new(Instant::now()),
    });
    if let Some(sh) = app.search.handle.hwnd() {
        unsafe { SendMessageW(sh, WM_SETFONT, app.fonts.borrow().get(Font::Ui) as usize, 1) };
    }
    app.apply_theme();
    {
        let mut d = app.data.borrow_mut();
        if page == Page::Startup {
            d.load_startup();
        }
    }
    app.tick();

    let hwnd = app.win.handle.hwnd().unwrap();
    unsafe { RegisterHotKey(hwnd, 1, (MOD_CONTROL | MOD_ALT) as u32, b'T' as u32) };

    let a = app.clone();
    let _raw = nwg::bind_raw_event_handler(&app.win.handle, 0x10000, move |hwnd, msg, w, l| -> Option<isize> {
        match msg {
            WM_PAINT => {
                a.paint(hwnd);
                Some(0)
            }
            WM_ERASEBKGND => Some(1),
            WM_GETMINMAXINFO => unsafe {
                let mmi = &mut *(l as *mut MINMAXINFO);
                let wa = winrust_common::ui::work_area();
                let (ww, wh) = ((wa.right - wa.left).max(480), (wa.bottom - wa.top).max(360));
                let s = a.scale.get();
                mmi.ptMinTrackSize = winapi::shared::windef::POINT {
                    x: ((620.0 * s) as i32).min((ww * 85 / 100).max(480)),
                    y: ((420.0 * s) as i32).min((wh * 85 / 100).max(360)),
                };
                Some(0)
            },
            0x02E0 => {
                a.on_dpi_changed(hwnd, (w & 0xFFFF) as u32, unsafe { &*(l as *const winapi::shared::windef::RECT) });
                Some(0)
            }
            WM_SIZE => {
                a.invalidate();
                None
            }
            WM_SETCURSOR => {
                if (l & 0xFFFF) == HTCLIENT {
                    let ui = a.ui.borrow();
                    if ui.perf_drag || ui.hit == Hit::PerfSplit {
                        unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE)) };
                        return Some(1);
                    }
                }
                None
            }
            WM_MOUSEMOVE => {
                a.on_mouse_move(hwnd, (l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
                None
            }
            WM_MOUSELEAVE => {
                a.on_mouse_leave();
                None
            }
            WM_LBUTTONDOWN => {
                a.on_lbutton_down(hwnd, (l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
                Some(0)
            }
            WM_LBUTTONUP => {
                a.on_lbutton_up();
                None
            }
            WM_RBUTTONUP => {
                a.on_rbutton_up(hwnd, (l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
                Some(0)
            }
            WM_MOUSEWHEEL => {
                a.on_wheel(((w >> 16) & 0xFFFF) as i16);
                Some(0)
            }
            WM_KEYDOWN => {
                a.on_key(hwnd, w as u32);
                None
            }
            WM_CTLCOLOREDIT => unsafe {
                let th = a.ui.borrow().th;
                winapi::um::wingdi::SetTextColor(w as _, th.text.c());
                winapi::um::wingdi::SetBkColor(w as _, th.panel.c());
                Some(a.edit_brush.get())
            },
            WM_HOTKEY if w == 1 => {
                a.show_window();
                Some(0)
            }
            _ => None,
        }
    })
    .unwrap();

    let a = app.clone();
    let _h = nwg::full_bind_event_handler(&app.win.handle, move |evt, data, handle| {
        use nwg::Event as E;
        match evt {
            E::OnWindowClose => {
                if let nwg::EventData::OnWindowClose(d) = data {
                    let to_tray = a.data.borrow().cfg.to_tray;
                    if to_tray {
                        d.close(false);
                        a.save_settings();
                        a.win.set_visible(false);
                    } else {
                        a.quit();
                    }
                }
            }
            E::OnTimerTick => a.on_timer(),
            E::OnTextInput if handle == a.search.handle => a.on_search_changed(),
            E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) if handle == a.tray.handle => a.show_window(),
            E::OnContextMenu if handle == a.tray.handle => {
                let (x, y) = nwg::GlobalCursor::position();
                let items = a.tray_menu();
                if let Some(h) = a.win.handle.hwnd()
                    && let Some(act) = a.show_menu(h, &items, x, y)
                {
                    a.do_act(act);
                }
            }
            _ => {}
        }
    });
    nwg::dispatch_thread_events();
}
