//! Poprawki okien specyficzne dla Windows.
//!
//! 1. Widocznosc overlaya: winit przy click-through dodaje oknu styl WS_EX_LAYERED, ale nie
//!    wywoluje SetLayeredWindowAttributes - takiego okna system w ogole nie rysuje.
//!    Ustawiamy pelne krycie (LWA_ALPHA = 255); przezroczystosc per piksel z renderera dziala dalej.
//! 2. Ramka: Windows 11 rysuje wokol okien bez ramki cienka szara obwodke, zaokraglone rogi i cien.
//!    Na overlayu na caly ekran wygladalo to jak ramka wokol monitora - wylaczamy przez DWM.
//! 3. Zrzut okna konfiguracji (do rozmytego tla): eframe nie obsluguje zrzutow okien
//!    rysowanych w trybie "immediate", wiec kopiujemy piksele z ekranu przez GDI.
//!
//! Okna szukamy po tytule i weryfikujemy, ze naleza do naszego procesu.

use std::ffi::c_void;

use eframe::egui::{Color32, ColorImage};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GWL_EXSTYLE, GetClientRect, GetWindowLongW, GetWindowThreadProcessId, LWA_ALPHA,
    SetLayeredWindowAttributes, WS_EX_LAYERED,
};
use windows::core::HSTRING;

pub enum LayeredFix {
    /// Atrybuty ustawione - overlay jest widoczny.
    Applied,
    /// Okno jeszcze nie istnieje albo nie ma stylu WS_EX_LAYERED - sprobuj w kolejnej klatce.
    NotYet,
    /// Wywolanie WinAPI sie nie powiodlo.
    Failed(String),
}

/// Szuka okna naszego procesu o podanym tytule.
fn find_own_window(title: &str) -> Option<HWND> {
    unsafe {
        let hwnd = FindWindowW(None, &HSTRING::from(title)).ok()?;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        // Okno o tym tytule moze nalezec do innej instancji programu.
        (pid == std::process::id()).then_some(hwnd)
    }
}

pub fn fix_overlay_window(title: &str) -> LayeredFix {
    let Some(hwnd) = find_own_window(title) else {
        return LayeredFix::NotYet;
    };
    unsafe {
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_LAYERED.0 == 0 {
            return LayeredFix::NotYet;
        }
        if let Err(err) = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) {
            return LayeredFix::Failed(err.to_string());
        }
        remove_window_frame(hwnd);
    }
    LayeredFix::Applied
}

/// Wylacza obwodke, zaokraglenia i cien rysowane przez DWM. Bledy sa tylko logowane -
/// starsze Windowsy nie znaja czesci atrybutow, a okno i tak dziala.
unsafe fn remove_window_frame(hwnd: HWND) {
    let border = DWMWA_COLOR_NONE;
    let corners = DWMWCP_DONOTROUND;
    let policy = DWMNCRP_DISABLED;
    let attributes: [(_, *const c_void, u32); 3] = [
        (DWMWA_BORDER_COLOR, &border as *const _ as *const c_void, size_of_val(&border) as u32),
        (DWMWA_WINDOW_CORNER_PREFERENCE, &corners as *const _ as *const c_void, size_of_val(&corners) as u32),
        (DWMWA_NCRENDERING_POLICY, &policy as *const _ as *const c_void, size_of_val(&policy) as u32),
    ];
    for (attribute, value, size) in attributes {
        if let Err(err) = unsafe { DwmSetWindowAttribute(hwnd, attribute, value, size) } {
            log::debug!("DwmSetWindowAttribute({}) nieudane: {err}", attribute.0);
        }
    }
}

/// Kopiuje aktualny wyglad obszaru klienta okna z ekranu.
pub fn capture_window(title: &str) -> Option<ColorImage> {
    let hwnd = find_own_window(title)?;
    unsafe {
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).ok()?;
        let (width, height) = (client.right - client.left, client.bottom - client.top);
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut origin = POINT { x: 0, y: 0 };
        if !ClientToScreen(hwnd, &mut origin).as_bool() {
            return None;
        }

        let screen_dc = GetDC(None);
        let memory_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        let previous = SelectObject(memory_dc, HGDIOBJ(bitmap.0));
        let copied = BitBlt(memory_dc, 0, 0, width, height, Some(screen_dc), origin.x, origin.y, SRCCOPY);

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Ujemna wysokosc = wiersze od gory do dolu, jak w ColorImage.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        SelectObject(memory_dc, previous);
        let lines = if copied.is_ok() {
            GetDIBits(
                memory_dc,
                bitmap,
                0,
                height as u32,
                Some(bgra.as_mut_ptr() as *mut c_void),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory_dc);
        ReleaseDC(None, screen_dc);

        if lines != height {
            log::warn!("Zrzut okna konfiguracji nieudany");
            return None;
        }
        let pixels = bgra.as_chunks::<4>().0.iter().map(|p| Color32::from_rgb(p[2], p[1], p[0])).collect();
        Some(ColorImage::new([width as usize, height as usize], pixels))
    }
}
