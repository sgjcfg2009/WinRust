#![allow(dead_code)]
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};

const DRIVE_FIXED: u32 = 3;

#[allow(unused_imports)]
pub use winrust_common::search::{ask, drive_letters, pipe_name};

pub fn fixed_ntfs_drives() -> Vec<char> {
    const NTFS: [u16; 5] = [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0];
    drive_letters()
        .into_iter()
        .filter(|&d| {
            let root = [d as u16, b':' as u16, b'\\' as u16, 0];
            let mut fs = [0u16; 16];
            let (ty, ok) = unsafe {
                let ty = GetDriveTypeW(root.as_ptr());
                let null = std::ptr::null_mut();
                (ty, GetVolumeInformationW(root.as_ptr(), null, 0, null as _, null as _, null as _, fs.as_mut_ptr(), 16))
            };
            ty == DRIVE_FIXED && ok != 0 && fs[..5] == NTFS
        })
        .collect()
}
