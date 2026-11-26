use std::ffi::c_void;
use std::ptr::{null, null_mut};
use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adapter {
    pub name: String,
    pub vendor: u32,
    pub luid: (u32, i32),
    pub dedicated: u64,
    pub shared: u64,
    pub dx: String,
    pub location: String,
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const IID_IDXGIFACTORY1: Guid = Guid(0x770aae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
const IID_ID3D12DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);

#[link(name = "dxgi")]
unsafe extern "system" {
    fn CreateDXGIFactory1(riid: *const Guid, out: *mut *mut c_void) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn D3DKMTOpenAdapterFromLuid(p: *mut OpenFromLuid) -> i32;
    fn D3DKMTQueryAdapterInfo(p: *mut QueryInfo) -> i32;
    fn D3DKMTCloseAdapter(p: *mut u32) -> i32;
}

#[repr(C)]
struct OpenFromLuid {
    low: u32,
    high: i32,
    handle: u32,
}

#[repr(C)]
struct QueryInfo {
    handle: u32,
    ty: u32,
    data: *mut c_void,
    size: u32,
}

unsafe fn vtbl<T: Copy>(obj: *mut c_void, idx: usize) -> T {
    unsafe {
        let table = *(obj as *const *const usize);
        std::mem::transmute_copy(&*table.add(idx))
    }
}

unsafe fn release(obj: *mut c_void) {
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = vtbl(obj, 2);
        f(obj);
    }
}

fn directx_level(adapter: *mut c_void) -> String {
    unsafe {
        let lib = LoadLibraryW(wide("d3d12.dll").as_ptr());
        if lib.is_null() {
            return "DirectX 11".into();
        }
        let f = GetProcAddress(lib, c"D3D12CreateDevice".as_ptr());
        if f.is_null() {
            return "DirectX 11".into();
        }
        let create: unsafe extern "system" fn(*mut c_void, i32, *const Guid, *mut *mut c_void) -> i32 = std::mem::transmute(f);
        for (fl, name) in [(0xC200, "12_2"), (0xC100, "12_1"), (0xC000, "12_0"), (0xB100, "11_1"), (0xB000, "11_0")] {
            let hr = create(adapter, fl, &IID_ID3D12DEVICE, null_mut());
            if hr == 0 || hr == 1 {
                return format!("DirectX {} (FL {name})", if fl >= 0xC000 { "12" } else { "11" });
            }
        }
        "DirectX 11".into()
    }
}

pub fn temperature(luid: (u32, i32)) -> Option<f32> {
    unsafe {
        let mut open = OpenFromLuid { low: luid.0, high: luid.1, handle: 0 };
        if D3DKMTOpenAdapterFromLuid(&mut open) != 0 {
            return None;
        }
        let mut perf = [0u8; 64];
        let mut q = QueryInfo { handle: open.handle, ty: 62 , data: perf.as_mut_ptr() as *mut c_void, size: perf.len() as u32 };
        let ok = D3DKMTQueryAdapterInfo(&mut q) == 0;
        let mut h = open.handle;
        D3DKMTCloseAdapter(&mut h);
        if !ok {
            return None;
        }
        let t = u32::from_le_bytes([perf[56], perf[57], perf[58], perf[59]]);
        (t > 0 && t < 1500).then_some(t as f32 / 10.0)
    }
}

fn locations() -> Vec<(String, String)> {
    use winapi::shared::guiddef::GUID;
    use winapi::um::setupapi::*;
    let class = GUID { Data1: 0x4d36e968, Data2: 0xe325, Data3: 0x11ce, Data4: [0xbf, 0xc1, 0x08, 0x00, 0x2b, 0xe1, 0x03, 0x18] };
    let mut out = vec![];
    unsafe {
        let set = SetupDiGetClassDevsW(&class, null(), null_mut(), DIGCF_PRESENT);
        if set as isize == -1 {
            return out;
        }
        let mut i = 0;
        loop {
            let mut dev: SP_DEVINFO_DATA = std::mem::zeroed();
            dev.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiEnumDeviceInfo(set, i, &mut dev) == 0 {
                break;
            }
            i += 1;
            let mut prop = |code: u32, buf: &mut [u8]| -> bool {
                let (mut ty, mut need) = (0u32, 0u32);
                SetupDiGetDeviceRegistryPropertyW(set, &mut dev, code, &mut ty, buf.as_mut_ptr(), buf.len() as u32, &mut need) != 0
            };
            let mut desc = [0u8; 512];
            let (mut bus, mut addr) = ([0u8; 4], [0u8; 4]);
            if prop(0 , &mut desc) && prop(0x10 , &mut bus) && prop(0x1C , &mut addr) {
                let w: Vec<u16> = desc.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                let (b, a) = (u32::from_le_bytes(bus), u32::from_le_bytes(addr));
                out.push((String::from_utf16_lossy(&w), format!("PCI bus {b}, device {}, function {}", (a >> 16) & 0xFFFF, a & 0xFFFF)));
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

pub fn adapters() -> Vec<Adapter> {
    let mut out = vec![];
    let locs = locations();
    unsafe {
        let mut factory: *mut c_void = null_mut();
        if CreateDXGIFactory1(&IID_IDXGIFACTORY1, &mut factory) != 0 || factory.is_null() {
            return out;
        }
        let enum1: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 = vtbl(factory, 12);
        let mut i = 0;
        loop {
            let mut ad: *mut c_void = null_mut();
            if enum1(factory, i, &mut ad) != 0 || ad.is_null() {
                break;
            }
            i += 1;
            let mut d = [0u8; 312];
            let get_desc1: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32 = vtbl(ad, 10);
            if get_desc1(ad, d.as_mut_ptr()) == 0 {
                let w: Vec<u16> = d[..256].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                let name = String::from_utf16_lossy(&w);
                let u64_at = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
                let flags = u32::from_le_bytes([d[304], d[305], d[306], d[307]]);
                let dup = out.iter().any(|a: &Adapter| a.name == name && a.dedicated == u64_at(272) && a.shared == u64_at(288));
                if flags & 2 == 0 && !dup && !name.to_lowercase().contains("basic render") {
                    out.push(Adapter {
                        location: locs.iter().find(|l| l.0.eq_ignore_ascii_case(&name)).map(|l| l.1.clone()).unwrap_or_default(),
                        dx: directx_level(ad),
                        vendor: u32::from_le_bytes([d[256], d[257], d[258], d[259]]),
                        luid: (u32::from_le_bytes([d[296], d[297], d[298], d[299]]), i32::from_le_bytes([d[300], d[301], d[302], d[303]])),
                        dedicated: u64_at(272),
                        shared: u64_at(288),
                        name,
                    });
                }
            }
            release(ad);
        }
        release(factory);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_adapters_have_memory_directx_location_and_temperature() {
        if std::env::var_os("CI").is_some() {
            return;
        }
        let a = adapters();
        assert!(!a.is_empty(), "DXGI должен вернуть хотя бы один адаптер");
        for g in &a {
            eprintln!("{g:?} temp={:?}", temperature(g.luid));
            assert!(!g.name.is_empty() && g.dx.starts_with("DirectX"), "{g:?}");
        }
        let nv = a.iter().find(|g| g.vendor == 0x10DE).expect("есть NVIDIA");
        assert!(nv.dedicated > 4 << 30, "выделенная память: {}", nv.dedicated);
        assert!(nv.location.contains("PCI"), "расположение: {:?}", nv.location);
        let t = temperature(nv.luid).expect("температура через D3DKMT");
        assert!((15.0..=110.0).contains(&t), "t={t}");
    }
}
