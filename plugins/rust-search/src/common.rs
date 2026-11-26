#![allow(dead_code)]
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};

const DRIVE_FIXED: u32 = 3;

#[allow(unused_imports)]
pub use winrust_common::search::{ask, drive_letters, pipe_name};

pub fn fixed_ntfs_drives() -> Vec<char> {
    drive_letters()
        .into_iter()
        .filter(|d| {
            let root: Vec<u16> = format!("{d}:\\").encode_utf16().chain([0]).collect();
            let mut fs = [0u16; 16];
            let (ty, ok) = unsafe {
                let ty = GetDriveTypeW(root.as_ptr());
                let null = std::ptr::null_mut();
                (ty, GetVolumeInformationW(root.as_ptr(), null, 0, null as _, null as _, null as _, fs.as_mut_ptr(), 16))
            };
            ty == DRIVE_FIXED && ok != 0 && String::from_utf16_lossy(&fs).trim_end_matches('\0') == "NTFS"
        })
        .collect()
}
