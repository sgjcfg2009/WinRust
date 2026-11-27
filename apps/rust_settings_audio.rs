#![allow(non_snake_case)]
use std::ptr::null;
use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::{
            DEVICE_STATE_ACTIVE, EDataFlow, Endpoints::IAudioEndpointVolume, IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume,
            MMDeviceEnumerator, eCapture, eConsole, eRender,
        },
        System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, STGM_READ},
    },
    core::Interface,
};

pub fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Src {
    Out,
    Mic,
    App(u32),
}

fn enumerator() -> Option<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() }
}

fn default_device(mic: bool) -> Option<IMMDevice> {
    unsafe { enumerator()?.GetDefaultAudioEndpoint(if mic { eCapture } else { eRender }, eConsole).ok() }
}

fn endpoint(mic: bool) -> Option<IAudioEndpointVolume> {
    unsafe { default_device(mic)?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok() }
}

pub struct Dev {
    pub id: String,
    pub name: String,
    pub default: bool,
}

fn dev_id(d: &IMMDevice) -> Option<String> {
    unsafe { d.GetId().ok().and_then(|p| p.to_string().ok()) }
}

fn dev_name(d: &IMMDevice) -> String {
    unsafe {
        let name = d.OpenPropertyStore(STGM_READ).ok().and_then(|s| s.GetValue(&PKEY_Device_FriendlyName).ok()).map(|v| v.to_string()).filter(|s| !s.is_empty());
        name.unwrap_or_else(|| dev_id(d).unwrap_or_default())
    }
}

pub fn devices(mic: bool) -> Vec<Dev> {
    let Some(e) = enumerator() else { return vec![] };
    let flow: EDataFlow = if mic { eCapture } else { eRender };
    let cur = unsafe { e.GetDefaultAudioEndpoint(flow, eConsole).ok() }.and_then(|d| dev_id(&d));
    let Ok(col) = (unsafe { e.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) }) else { return vec![] };
    (0..unsafe { col.GetCount() }.unwrap_or(0)).filter_map(|i| unsafe { col.Item(i).ok() }).filter_map(|d| dev_id(&d).map(|id| Dev { default: Some(&id) == cur.as_ref(), name: dev_name(&d), id })).collect()
}

pub fn default_name(mic: bool) -> String {
    default_device(mic).map_or("—".into(), |d| dev_name(&d))
}

#[path = "audio_policy.rs"]
mod audio_policy;
pub use audio_policy::set_default;

fn sessions() -> Vec<IAudioSessionControl2> {
    unsafe {
        let Some(m) = default_device(false).and_then(|d| d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None).ok()) else { return vec![] };
        let Ok(list) = m.GetSessionEnumerator() else { return vec![] };
        (0..list.GetCount().unwrap_or(0)).filter_map(|i| list.GetSession(i).ok()).filter_map(|s| s.cast::<IAudioSessionControl2>().ok()).collect()
    }
}

fn proc_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 520];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n) != 0;
        CloseHandle(h);
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        let s = path.rsplit('\\').next().unwrap_or(&path);
        ok.then(|| s.strip_suffix(".exe").unwrap_or(s).to_string())
    }
}

pub fn apps() -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = vec![];
    for s in sessions() {
        let pid = unsafe { s.GetProcessId() }.unwrap_or(0);
        let system = unsafe { s.IsSystemSoundsSession() }.0 == 0;
        let pid = if system { 0 } else { pid };
        if out.iter().any(|o| o.0 == pid) {
            continue;
        }
        let name = if pid == 0 { "System sounds".to_string() } else { proc_name(pid).unwrap_or_else(|| format!("PID {pid}")) };
        out.push((pid, name));
    }
    out.sort_by_key(|a| a.1.to_lowercase());
    out
}

fn app_volumes(pid: u32) -> impl Iterator<Item = ISimpleAudioVolume> {
    sessions()
        .into_iter()
        .filter(move |s| {
            let sys = unsafe { s.IsSystemSoundsSession() }.0 == 0;
            if pid == 0 { sys } else { !sys && unsafe { s.GetProcessId() }.ok() == Some(pid) }
        })
        .filter_map(|s| s.cast::<ISimpleAudioVolume>().ok())
}

pub fn volume(src: Src) -> Option<f32> {
    unsafe {
        match src {
            Src::Out | Src::Mic => endpoint(src == Src::Mic)?.GetMasterVolumeLevelScalar().ok(),
            Src::App(pid) => app_volumes(pid).next()?.GetMasterVolume().ok(),
        }
    }
}

pub fn set_volume(src: Src, v: f32) {
    let v = v.clamp(0.0, 1.0);
    unsafe {
        match src {
            Src::Out | Src::Mic => {
                if let Some(e) = endpoint(src == Src::Mic) {
                    let _ = e.SetMasterVolumeLevelScalar(v, null());
                }
            }
            Src::App(pid) => app_volumes(pid).for_each(|a| drop(a.SetMasterVolume(v, null()))),
        }
    }
}

pub fn muted(mic: bool) -> Option<bool> {
    unsafe { Some(endpoint(mic)?.GetMute().ok()?.as_bool()) }
}

pub fn set_mute(mic: bool, m: bool) {
    if let Some(e) = endpoint(mic) {
        let _ = unsafe { e.SetMute(m, null()) };
    }
}
