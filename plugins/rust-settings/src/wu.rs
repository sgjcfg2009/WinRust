use std::sync::Mutex;
use windows::{
    Win32::System::{
        Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
        UpdateAgent::*,
    },
    core::BSTR,
};

#[derive(Clone, Default)]
pub struct State {
    pub busy: Option<String>,
    pub found: Option<Vec<String>>,
    pub note: String,
}

static STATE: Mutex<State> = Mutex::new(State { busy: None, found: None, note: String::new() });

pub fn state() -> State {
    STATE.lock().map(|s| s.clone()).unwrap_or_default()
}

fn with<T>(f: impl FnOnce(&mut State) -> T) {
    if let Ok(mut s) = STATE.lock() {
        f(&mut s);
    }
}

fn session() -> windows::core::Result<IUpdateSession> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&UpdateSession, None, CLSCTX_INPROC_SERVER)
    }
}

fn search(s: &IUpdateSession) -> windows::core::Result<IUpdateCollection> {
    unsafe { s.CreateUpdateSearcher()?.Search(&BSTR::from("IsInstalled=0 and IsHidden=0"))?.Updates() }
}

fn titles(c: &IUpdateCollection) -> Vec<String> {
    unsafe { (0..c.Count().unwrap_or(0)).filter_map(|i| c.get_Item(i).ok()).filter_map(|u| u.Title().ok()).map(|t| t.to_string()).collect() }
}

fn err_text(e: &windows::core::Error) -> String {
    if e.code().0 as u32 == 0x8007_0005 {
        "Administrator rights are needed".into()
    } else {
        e.message()
    }
}

fn start(what: &str, done: impl FnOnce() + Send + 'static, job: impl FnOnce() -> String + Send + 'static) {
    let mut busy = true;
    with(|s| {
        busy = s.busy.is_some();
        if !busy {
            s.busy = Some(what.to_string());
        }
    });
    if busy {
        return;
    }
    std::thread::spawn(move || {
        let note = job();
        with(|s| (s.busy, s.note) = (None, note));
        done();
    });
}

pub fn check(done: impl FnOnce() + Send + 'static) {
    start("Checking for updates…", done, || match session().and_then(|s| search(&s)) {
        Ok(c) => {
            let t = titles(&c);
            let note = if t.is_empty() { "You're up to date".into() } else { format!("Updates found: {}", t.len()) };
            with(|s| s.found = Some(t));
            note
        }
        Err(e) => format!("Could not check: {}", err_text(&e)),
    });
}

pub fn install(done: impl FnOnce() + Send + 'static) {
    start("Downloading and installing updates…", done, || {
        let run = || -> windows::core::Result<String> {
            unsafe {
                let s = session()?;
                let found = search(&s)?;
                if found.Count()? == 0 {
                    return Ok("You're up to date".into());
                }
                for i in 0..found.Count()? {
                    let u = found.get_Item(i)?;
                    if !u.EulaAccepted()?.as_bool() {
                        u.AcceptEula()?;
                    }
                }
                let d = s.CreateUpdateDownloader()?;
                d.SetUpdates(&found)?;
                d.Download()?;
                let inst = s.CreateUpdateInstaller()?;
                inst.SetUpdates(&found)?;
                let r = inst.Install()?;
                Ok(if r.RebootRequired()?.as_bool() { "Installed. Restart to finish".into() } else { "Updates installed".into() })
            }
        };
        match run() {
            Ok(m) => {
                with(|s| s.found = Some(vec![]));
                m
            }
            Err(e) => format!("Could not install: {}", err_text(&e)),
        }
    });
}
