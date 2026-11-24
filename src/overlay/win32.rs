//! Poprawki okien specyficzne dla Windows.
//!
//! 1. Widocznosc overlaya: winit przy click-through dodaje oknu styl WS_EX_LAYERED, ale nie
//!    wywoluje SetLayeredWindowAttributes - takiego okna system w ogole nie rysuje.
//!    Ustawiamy pelne krycie (LWA_ALPHA = 255); przezroczystosc per piksel z renderera dziala dalej.
//! 2. Ramka: Windows 11 rysuje wokol okien bez ramki cienka szara obwodke, zaokraglone rogi i cien.
//!    Na overlayu na caly ekran wygladalo to jak ramka wokol monitora - wylaczamy przez DWM.
//!    Dodatkowo winit dla okna bez ramki z cieniem przesuwa obszar klienta o 1 px w dol
//!    (WM_NCCALCSIZE, opisany u nich "HACK" dajacy linie 1 px u gory) - na overlayu byla to szara
//!    linia na gornej krawedzi ekranu. Przejmujemy WM_NCCALCSIZE (subclassing okna): caly
//!    prostokat okna jest obszarem klienta, bez przesuniecia.
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
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GWL_EXSTYLE, GetClientRect, GetWindowLongW, GetWindowThreadProcessId, LWA_ALPHA, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowPos, WM_NCCALCSIZE,
    WS_EX_LAYERED,
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
        remove_top_border(hwnd);
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

const OVERLAY_SUBCLASS_ID: usize = 0xA57;

/// Obsluga WM_NCCALCSIZE przed winit: zwrocenie 0 bez zmiany prostokata = caly prostokat okna
/// jest obszarem klienta (bez ramki i bez przesuniecia o 1 px). Reszta komunikatow idzie dalej do winit.
unsafe extern "system" fn overlay_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_NCCALCSIZE && wparam.0 != 0 {
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

unsafe fn remove_top_border(hwnd: HWND) {
    unsafe {
        if !SetWindowSubclass(hwnd, Some(overlay_subclass_proc), OVERLAY_SUBCLASS_ID, 0).as_bool() {
            log::debug!("SetWindowSubclass overlaya nieudane");
            return;
        }
        // SWP_FRAMECHANGED - system ponownie wysyla WM_NCCALCSIZE i przelicza obszar klienta.
        let flags = SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        if let Err(err) = SetWindowPos(hwnd, None, 0, 0, 0, 0, flags) {
            log::debug!("SetWindowPos (przeliczenie ramki overlaya) nieudane: {err}");
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
