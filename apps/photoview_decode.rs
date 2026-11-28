use windows::{
    Win32::{Foundation::GENERIC_READ, Graphics::Imaging::*, System::Com::*},
    core::*,
};

pub struct Img {
    pub w: i32,
    pub h: i32,
    pub bgra: Vec<u8>,
}

pub fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

pub fn com_uninit() {
    unsafe {
        CoUninitialize();
    }
}

pub fn create_factory() -> Result<IWICImagingFactory> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

fn transform(orientation: u16) -> i32 {
    match orientation {
        2 => 8,
        3 => 2,
        4 => 16,
        5 => 1 | 8,
        6 => 1,
        7 => 3 | 8,
        8 => 3,
        _ => 0,
    }
}

pub fn decode_with(f: &IWICImagingFactory, path: &str, bg: [u8; 3]) -> Result<Img> {
    unsafe {
        let dec = f.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let frame = dec.GetFrame(0)?;
        let orientation = frame
            .GetMetadataQueryReader()
            .and_then(|q| {
                let mut v = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::default();
                q.GetMetadataByName(w!("/app1/ifd/{ushort=274}"), &mut v)?;
                u16::try_from(&v)
            })
            .unwrap_or(1);
        let fmt = frame.GetPixelFormat().ok();
        let opaque = fmt.is_some_and(|g| {
            g == GUID_WICPixelFormat24bppBGR
                || g == GUID_WICPixelFormat24bppRGB
                || g == GUID_WICPixelFormat32bppBGR
                || g == GUID_WICPixelFormat8bppGray
                || g == GUID_WICPixelFormat16bppGray
        });
        let conv = f.CreateFormatConverter()?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let mut src: IWICBitmapSource = conv.cast()?;
        let t = transform(orientation);
        if t != 0 {
            let r = f.CreateBitmapFlipRotator()?;
            r.Initialize(&src, WICBitmapTransformOptions(t))?;
            src = r.cast()?;
        }
        let (mut w, mut h) = (0u32, 0u32);
        src.GetSize(&mut w, &mut h)?;
        const MAX_DIM: u32 = 4096;
        if w > MAX_DIM || h > MAX_DIM {
            let (nw, nh) = if w >= h {
                (MAX_DIM, ((h as u64 * MAX_DIM as u64) / w as u64).max(1) as u32)
            } else {
                (((w as u64 * MAX_DIM as u64) / h as u64).max(1) as u32, MAX_DIM)
            };
            let scaler = f.CreateBitmapScaler()?;
            scaler.Initialize(&src, nw, nh, WICBitmapInterpolationModeFant)?;
            src = scaler.cast()?;
            w = nw;
            h = nh;
        }
        let byte_len = (w as usize)
            .checked_mul(h as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| Error::from(windows::Win32::Foundation::E_OUTOFMEMORY))?;
        let mut bgra = vec![0u8; byte_len];
        src.CopyPixels(std::ptr::null(), w * 4, &mut bgra)?;
        if !opaque {
            for p in bgra.chunks_exact_mut(4) {
                let a = p[3];
                if a < 255 {
                    let inv = 255 - a as u32;
                    p[0] = (p[0] as u32 + bg[0] as u32 * inv / 255).min(255) as u8;
                    p[1] = (p[1] as u32 + bg[1] as u32 * inv / 255).min(255) as u8;
                    p[2] = (p[2] as u32 + bg[2] as u32 * inv / 255).min(255) as u8;
                    p[3] = 255;
                }
            }
        }
        Ok(Img { w: w as i32, h: h as i32, bgra })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_png_and_composites_alpha_over_background() {
        fn crc(d: &[u8]) -> u32 {
            let mut c = !0u32;
            for &b in d {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, ty: &[u8], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = ty.to_vec();
            body.extend(data);
            out.extend(&body);
            out.extend(crc(&body).to_be_bytes());
        }
        let raw = [0u8, 255, 0, 0, 255, 0, 0, 0, 0];
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        let mut z = vec![0x78, 0x01, 0x01, raw.len() as u8, 0, !(raw.len() as u8), 0xFF];
        z.extend(raw);
        z.extend(((b << 16) | a).to_be_bytes());
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        chunk(&mut png, b"IHDR", &[0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
        chunk(&mut png, b"IDAT", &z);
        chunk(&mut png, b"IEND", &[]);
        let p = std::env::temp_dir().join("photoview-test.png");
        std::fs::write(&p, png).unwrap();
        com_init();
        let img = decode_with(&create_factory().unwrap(), p.to_str().unwrap(), [10, 20, 30]).unwrap();
        let _ = std::fs::remove_file(p);
        assert_eq!((img.w, img.h), (2, 1));
        assert_eq!(img.bgra[..4], [0, 0, 255, 255]);
        assert_eq!(img.bgra[4..8], [10, 20, 30, 255]);
    }
}
