//! Screenshot eines Bildschirmbereichs per GDI (Ersatz für `mss`).

use crate::matcher::Image;
use crate::win::*;
use std::ffi::c_void;
use std::mem::size_of;

/// Liefert den Bereich als BGR-Bild.
pub fn grab(left: i32, top: i32, width: i32, height: i32) -> Result<Image, String> {
    if width <= 0 || height <= 0 {
        return Err("Wardogs-Fenster hat keine gültige Größe.".into());
    }
    let (w, h) = (width as usize, height as usize);
    unsafe {
        let screen = GetDC(0);
        if screen == 0 {
            return Err("Bildschirm konnte nicht gelesen werden.".into());
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, width, height);
        if mem == 0 || bmp == 0 {
            if bmp != 0 {
                DeleteObject(bmp);
            }
            if mem != 0 {
                DeleteDC(mem);
            }
            ReleaseDC(0, screen);
            return Err("Screenshot-Puffer konnte nicht angelegt werden.".into());
        }
        let old = SelectObject(mem, bmp);
        let copied = BitBlt(mem, 0, 0, width, height, screen, left, top, SRCCOPY | CAPTUREBLT);
        // Die Bitmap darf für GetDIBits in keinem DC ausgewählt sein
        SelectObject(mem, old);

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negativ = von oben nach unten
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            bmiColors: [0],
        };
        let mut bgra = vec![0u8; w * h * 4];
        let lines = GetDIBits(screen, bmp, 0, height as u32, bgra.as_mut_ptr() as *mut c_void, &mut info, 0);

        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(0, screen);

        if copied == 0 || lines == 0 {
            return Err("Screenshot fehlgeschlagen.".into());
        }

        let mut data = vec![0u8; w * h * 3];
        for (src, dst) in bgra.chunks_exact(4).zip(data.chunks_exact_mut(3)) {
            dst[0] = src[0];
            dst[1] = src[1];
            dst[2] = src[2];
        }
        Ok(Image { w, h, data })
    }
}
