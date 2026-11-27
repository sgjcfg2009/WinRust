#![allow(non_snake_case)]
use std::ffi::c_void;
use windows::{
    Win32::{
        Media::Audio::{eCommunications, eConsole, eMultimedia},
        System::Com::{CLSCTX_ALL, CoCreateInstance},
    },
    core::{GUID, HRESULT, PCWSTR},
};

#[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows::core::IUnknown {
    fn GetMixFormat(&self, dev: PCWSTR, fmt: *mut *mut c_void) -> HRESULT;
    fn GetDeviceFormat(&self, dev: PCWSTR, default: i32, fmt: *mut *mut c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, dev: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, dev: PCWSTR, a: *mut c_void, b: *mut c_void) -> HRESULT;
    fn GetProcessingPeriod(&self, dev: PCWSTR, default: i32, a: *mut i64, b: *mut i64) -> HRESULT;
    fn SetProcessingPeriod(&self, dev: PCWSTR, a: *mut i64) -> HRESULT;
    fn GetShareMode(&self, dev: PCWSTR, m: *mut c_void) -> HRESULT;
    fn SetShareMode(&self, dev: PCWSTR, m: *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, dev: PCWSTR, fx: i32, key: *const c_void, v: *mut c_void) -> HRESULT;
    fn SetPropertyValue(&self, dev: PCWSTR, fx: i32, key: *const c_void, v: *mut c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, dev: PCWSTR, role: i32) -> HRESULT;
    fn SetEndpointVisibility(&self, dev: PCWSTR, visible: i32) -> HRESULT;
}

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

pub fn set_default(id: &str) -> bool {
    unsafe {
        let Ok(pc) = CoCreateInstance::<_, IPolicyConfig>(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) else { return false };
        winrust_common::with_wide(id, |w| {
            [eConsole, eMultimedia, eCommunications].iter().all(|r| pc.SetDefaultEndpoint(PCWSTR(w), r.0).is_ok())
        })
    }
}
