use std::{collections::HashMap, ptr::null, sync::Mutex};
use windows_sys::{core::GUID, Win32::System::Diagnostics::Etw::*};

const KERNEL_NETWORK: GUID = GUID { data1: 0x7DD42A49, data2: 0x5329, data3: 0x4832, data4: [0x8D, 0xFD, 0x43, 0xD9, 0x79, 0x15, 0x3A, 0x88] };
const SESSION: &str = "RTaskMgrNet";

static TOTALS: Mutex<Option<HashMap<u32, (u64, u64)>>> = Mutex::new(None);

pub struct Session {
    handle: u64,
}

fn ctl(v: u64) -> CONTROLTRACE_HANDLE {
    CONTROLTRACE_HANDLE { Value: v }
}

fn is_kernel_network(g: &GUID) -> bool {
    (g.data1, g.data2, g.data3, g.data4) == (KERNEL_NETWORK.data1, KERNEL_NETWORK.data2, KERNEL_NETWORK.data3, KERNEL_NETWORK.data4)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn props_buffer(name: &[u16]) -> Vec<u64> {
    let head = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
    let total = head + name.len() * 2;
    let mut buf = vec![0u64; total.div_ceil(8)];
    unsafe {
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        (*p).Wnode.BufferSize = total as u32;
        (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        (*p).Wnode.ClientContext = 1;
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).LoggerNameOffset = head as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (buf.as_mut_ptr() as *mut u8).add(head) as *mut u16, name.len());
    }
    buf
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let r = unsafe { &*rec };
    if !is_kernel_network(&r.EventHeader.ProviderId) || r.UserDataLength < 8 || r.UserData.is_null() {
        return;
    }
    let received = match r.EventHeader.EventDescriptor.Id {
        10 | 26 | 42 | 58 => false,
        11 | 27 | 43 | 59 => true,
        _ => return,
    };
    let d = unsafe { std::slice::from_raw_parts(r.UserData as *const u8, 8) };
    let pid = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
    let size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as u64;
    if let Ok(mut g) = TOTALS.lock() {
        let e = g.get_or_insert_with(HashMap::new).entry(pid).or_insert((0, 0));
        if received { e.0 += size } else { e.1 += size }
    }
}

pub fn start() -> Option<Session> {
    let name = wide(SESSION);
    let session_handle;
    unsafe {
        let mut stale = props_buffer(&name);
        ControlTraceW(ctl(0), name.as_ptr(), stale.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP);

        let mut props = props_buffer(&name);
        let mut handle = ctl(0);
        if StartTraceW(&mut handle, name.as_ptr(), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) != 0 {
            return None;
        }
        if EnableTraceEx2(handle, &KERNEL_NETWORK, EVENT_CONTROL_CODE_ENABLE_PROVIDER, TRACE_LEVEL_INFORMATION as u8, u64::MAX, 0, 0, null()) != 0 {
            ControlTraceW(handle, null(), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP);
            return None;
        }
        session_handle = handle.Value;
    }
    std::thread::spawn(move || unsafe {
        let mut logger = wide(SESSION);
        let mut lf: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
        lf.LoggerName = logger.as_mut_ptr();
        lf.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        lf.Anonymous2.EventRecordCallback = Some(on_event);
        let h = OpenTraceW(&mut lf);
        if h.Value != u64::MAX {
            ProcessTrace(&h, 1, null(), null());
            CloseTrace(h);
        }
    });
    Some(Session { handle: session_handle })
}

pub fn stop(s: Session) {
    let name = wide(SESSION);
    let mut props = props_buffer(&name);
    unsafe { ControlTraceW(ctl(s.handle), null(), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP) };
}

pub fn drain() -> HashMap<u32, (u64, u64)> {
    TOTALS.lock().ok().and_then(|mut g| g.take()).unwrap_or_default()
}
