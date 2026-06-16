//! Extract real application icons from executable paths and return them as
//! PNG `data:` URLs the UI can render directly.

#[cfg(windows)]
pub fn app_icon_data_url(path: &str) -> Option<String> {
    windows_impl::app_icon_data_url(path)
}

#[cfg(not(windows))]
pub fn app_icon_data_url(_path: &str) -> Option<String> {
    None
}

/// Minimal standard base64 encoder (no external dependency).
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(windows)]
mod windows_impl {
    use std::ffi::c_void;
    use std::io::Cursor;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    use super::base64_encode;

    pub fn app_icon_data_url(path: &str) -> Option<String> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut info = SHFILEINFOW::default();
        let result = unsafe {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            )
        };
        if result == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let rgba = unsafe { icon_to_rgba(info) };
        unsafe {
            let _ = DestroyIcon(info.hIcon);
        }
        let (width, height, pixels) = rgba?;
        let img = image::RgbaImage::from_raw(width, height, pixels)?;
        let mut cursor = Cursor::new(Vec::new());
        img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
        Some(format!(
            "data:image/png;base64,{}",
            base64_encode(&cursor.into_inner())
        ))
    }

    /// Returns (width, height, RGBA bytes) for the icon's color bitmap.
    unsafe fn icon_to_rgba(info: SHFILEINFOW) -> Option<(u32, u32, Vec<u8>)> {
        let mut icon_info = ICONINFO::default();
        GetIconInfo(info.hIcon, &mut icon_info).ok()?;
        let color = icon_info.hbmColor;
        let mask = icon_info.hbmMask;

        let cleanup = |c: windows::Win32::Graphics::Gdi::HBITMAP,
                       m: windows::Win32::Graphics::Gdi::HBITMAP| {
            use windows::Win32::Graphics::Gdi::DeleteObject;
            if !c.is_invalid() {
                let _ = DeleteObject(c);
            }
            if !m.is_invalid() {
                let _ = DeleteObject(m);
            }
        };

        if color.is_invalid() {
            cleanup(color, mask);
            return None;
        }

        let mut bmp = BITMAP::default();
        let written = GetObjectW(
            HGDIOBJ(color.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut _ as *mut c_void),
        );
        if written == 0 || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
            cleanup(color, mask);
            return None;
        }
        let width = bmp.bmWidth;
        let height = bmp.bmHeight;

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width;
        // Negative height => top-down rows.
        bmi.bmiHeader.biHeight = -height;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let mut buffer = vec![0u8; (width * height * 4) as usize];
        let hdc = GetDC(HWND(std::ptr::null_mut()));
        let lines = GetDIBits(
            hdc,
            color,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(HWND(std::ptr::null_mut()), hdc);
        cleanup(color, mask);

        if lines == 0 {
            return None;
        }

        // GetDIBits returns BGRA. Convert to RGBA.
        let mut any_alpha = false;
        for px in buffer.chunks_exact_mut(4) {
            px.swap(0, 2);
            if px[3] != 0 {
                any_alpha = true;
            }
        }
        // Legacy icons report zero alpha across the board; treat them as opaque.
        if !any_alpha {
            for px in buffer.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }

        Some((width as u32, height as u32, buffer))
    }
}
