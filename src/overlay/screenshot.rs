//! Zrzut z gry jako tlo podgladu ekranu w oknie konfiguracji.
//!
//! - wybor pliku: systemowe okno IFileOpenDialog (PNG, JPG, BMP),
//! - schowek: obraz CF_DIB (PrintScreen, Win+Shift+S),
//! - dekodowanie plikow: WIC (Windows Imaging Component) - bez dodatkowych bibliotek.
//!
//! Obraz jest zapisywany w katalogu konfiguracji (`preview-background.*`), zeby wersja portable
//! przenosila go razem z ustawieniami. Zaleznosci: windows (Com, Imaging, Shell, DataExchange, Memory).

use std::path::{Path, PathBuf};

use eframe::egui::ColorImage;
use windows::Win32::Foundation::{GENERIC_READ, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnLoad,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_DIB;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::core::{HSTRING, w};

use crate::i18n::{Lang, current, tr};

/// Nazwa pliku tla (bez rozszerzenia) w katalogu konfiguracji.
const BACKGROUND_STEM: &str = "preview-background";
/// Wiekszy obraz nie zmiesci sie w teksturze GPU (limit egui/OpenGL) - taki zrzut odrzucamy.
const MAX_IMAGE_SIDE: usize = 8192;

/// COM w watku UI - winit zwykle juz go zainicjalizowal (wtedy wynik jest ignorowany).
fn ensure_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// Systemowe okno wyboru obrazu; `None` = anulowano.
pub fn pick_image_file() -> Option<PathBuf> {
    ensure_com();
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let (filter_name, title) = match current() {
            Lang::Pl => (w!("Obrazy (PNG, JPG, BMP)"), w!("Zrzut ekranu z gry")),
            Lang::En => (w!("Images (PNG, JPG, BMP)"), w!("Game screenshot")),
        };
        let filters = [COMDLG_FILTERSPEC { pszName: filter_name, pszSpec: w!("*.png;*.jpg;*.jpeg;*.bmp") }];
        let _ = dialog.SetFileTypes(&filters);
        let _ = dialog.SetTitle(title);
        // Wlascicielem jest okno konfiguracji (aktywne w chwili klikniecia) - dialog jest modalny.
        let owner = GetForegroundWindow();
        dialog.Show(Some(owner).filter(|h: &HWND| !h.is_invalid())).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path.map(PathBuf::from)
    }
}

/// Dekoduje plik obrazu (PNG/JPG/BMP/...) przez WIC do RGBA.
pub fn load_image(path: &Path) -> Result<ColorImage, String> {
    ensure_com();
    let fail = |err: windows::core::Error| format!("{} {}: {err}", tr("Nie można odczytać obrazu", "Cannot read image"), path.display());
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(fail)?;
        let decoder = factory
            .CreateDecoderFromFilename(
                &HSTRING::from(path.as_os_str()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnLoad,
            )
            .map_err(fail)?;
        let frame = decoder.GetFrame(0).map_err(fail)?;
        let converter = factory.CreateFormatConverter().map_err(fail)?;
        converter
            .Initialize(&frame, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(fail)?;
        let (mut width, mut height) = (0u32, 0u32);
        converter.GetSize(&mut width, &mut height).map_err(fail)?;
        check_size(width as usize, height as usize)?;
        let stride = width * 4;
        let mut pixels = vec![0u8; (stride * height) as usize];
        converter.CopyPixels(std::ptr::null(), stride, &mut pixels).map_err(fail)?;
        Ok(ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &pixels))
    }
}

fn check_size(width: usize, height: usize) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err(tr("Obraz jest pusty", "The image is empty").to_owned());
    }
    if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(format!(
            "{} ({width}x{height}, max {MAX_IMAGE_SIDE} px)",
            tr("Obraz jest za duży", "The image is too large")
        ));
    }
    Ok(())
}

/// Obraz ze schowka (format CF_DIB - PrintScreen, Win+Shift+S, wiekszosc programow).
pub fn clipboard_image() -> Result<ColorImage, String> {
    unsafe {
        OpenClipboard(None).map_err(|err| format!("{} {err}", tr("Schowek jest zajęty:", "The clipboard is busy:")))?;
        let result = (|| {
            let handle: HANDLE = GetClipboardData(u32::from(CF_DIB.0))
                .map_err(|_| {
                    tr(
                        "W schowku nie ma obrazu - zrób zrzut (PrintScreen lub Win+Shift+S)",
                        "No image in the clipboard - take a screenshot (PrintScreen or Win+Shift+S)",
                    )
                    .to_owned()
                })?;
            let global = HGLOBAL(handle.0);
            let size = GlobalSize(global);
            let data = GlobalLock(global) as *const u8;
            if data.is_null() {
                return Err(tr("Nie można odczytać schowka", "Cannot read the clipboard").to_owned());
            }
            let bytes = std::slice::from_raw_parts(data, size).to_vec();
            let _ = GlobalUnlock(global);
            parse_dib(&bytes)
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Dekoduje DIB (BITMAPINFOHEADER + piksele) 24/32 bpp, BI_RGB lub BI_BITFIELDS.
fn parse_dib(bytes: &[u8]) -> Result<ColorImage, String> {
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let bad = || tr("Nieobsługiwany format obrazu w schowku", "Unsupported image format in the clipboard").to_owned();
    let header_size = u32_at(0).ok_or_else(bad)? as usize;
    let width = u32_at(4).ok_or_else(bad)? as i32;
    let height = u32_at(8).ok_or_else(bad)? as i32;
    let bit_count = u16_at(14).ok_or_else(bad)?;
    let compression = u32_at(16).ok_or_else(bad)?;
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    if header_size < 40 || width <= 0 || height == 0 || !matches!(bit_count, 24 | 32) {
        return Err(bad());
    }
    if compression != BI_RGB && compression != BI_BITFIELDS {
        return Err(bad());
    }
    // Przy BI_BITFIELDS z naglowkiem 40 B za naglowkiem sa 3 maski kolorow (zakladamy standardowe BGR).
    let masks = if compression == BI_BITFIELDS && header_size == 40 { 12 } else { 0 };
    let offset = header_size + masks;
    let (width, top_down) = (width as usize, height < 0);
    let height = height.unsigned_abs() as usize;
    check_size(width, height)?;
    let bytes_per_pixel = usize::from(bit_count / 8);
    let stride = (width * bytes_per_pixel).div_ceil(4) * 4;
    if bytes.len() < offset + stride * height {
        return Err(bad());
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row = if top_down { y } else { height - 1 - y };
        let start = offset + row * stride;
        for x in 0..width {
            let p = start + x * bytes_per_pixel;
            // DIB przechowuje BGR(A); kanal alfa w zrzutach bywa zerowy - obraz zawsze nieprzezroczysty.
            rgba.extend_from_slice(&[bytes[p + 2], bytes[p + 1], bytes[p], 255]);
        }
    }
    Ok(ColorImage::from_rgba_unmultiplied([width, height], &rgba))
}

/// Zapisuje obraz jako 32-bitowy BMP (bez kompresji - nie wymaga kodera).
fn write_bmp(image: &ColorImage, path: &Path) -> std::io::Result<()> {
    let [width, height] = image.size;
    let pixel_bytes = width * height * 4;
    let mut out = Vec::with_capacity(54 + pixel_bytes);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    // Ujemna wysokosc = wiersze od gory.
    out.extend_from_slice(&(-(height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for pixel in &image.pixels {
        let [r, g, b, _] = pixel.to_srgba_unmultiplied();
        out.extend_from_slice(&[b, g, r, 255]);
    }
    std::fs::write(path, out)
}

/// Usuwa poprzednie pliki tla (dowolne rozszerzenie).
fn remove_backgrounds(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_stem().is_some_and(|stem| stem == BACKGROUND_STEM) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Kopiuje wybrany plik do katalogu konfiguracji; zwraca nazwe pliku do zapisania w konfiguracji.
pub fn store_file(source: &Path, dir: &Path) -> Result<String, String> {
    let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let name = format!("{BACKGROUND_STEM}.{extension}");
    std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    remove_backgrounds(dir);
    std::fs::copy(source, dir.join(&name)).map_err(|err| format!("{} {err}", tr("Nie można zapisać tła:", "Cannot save the background:")))?;
    Ok(name)
}

/// Zapisuje obraz ze schowka w katalogu konfiguracji; zwraca nazwe pliku.
pub fn store_image(image: &ColorImage, dir: &Path) -> Result<String, String> {
    let name = format!("{BACKGROUND_STEM}.bmp");
    std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    remove_backgrounds(dir);
    write_bmp(image, &dir.join(&name)).map_err(|err| format!("{} {err}", tr("Nie można zapisać tła:", "Cannot save the background:")))?;
    Ok(name)
}

/// Usuwa zapisane tlo.
pub fn clear(dir: &Path) {
    remove_backgrounds(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DIB 2x2, 24 bpp, od dolu: dolny wiersz czerwony/zielony, gorny niebieski/bialy.
    fn sample_dib() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0u8; 24]);
        // Wiersz 0 w pliku = dolny: BGR czerwony, BGR zielony + 2 B wyrownania.
        dib.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
        dib.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]);
        dib
    }

    #[test]
    fn dib_bottom_up_is_flipped_and_converted_to_rgb() {
        let image = parse_dib(&sample_dib()).unwrap();
        assert_eq!(image.size, [2, 2]);
        assert_eq!(image.pixels[0].to_srgba_unmultiplied(), [0, 0, 255, 255]); // lewy gorny = niebieski
        assert_eq!(image.pixels[2].to_srgba_unmultiplied(), [255, 0, 0, 255]); // lewy dolny = czerwony
    }

    #[test]
    fn bmp_roundtrips_through_dib_parser() {
        let image = parse_dib(&sample_dib()).unwrap();
        let dir = std::env::temp_dir().join(format!("artvitals-bmp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bmp");
        write_bmp(&image, &path).unwrap();
        let file = std::fs::read(&path).unwrap();
        // Plik BMP = 14 B naglowka pliku + DIB.
        let back = parse_dib(&file[14..]).unwrap();
        assert_eq!(back.pixels, image.pixels);
        // Ten sam plik przez WIC (sciezka "Wczytaj zrzut z pliku").
        let decoded = load_image(&path).unwrap();
        assert_eq!(decoded.size, [2, 2]);
        assert_eq!(decoded.pixels, image.pixels);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_unsupported_dib() {
        let mut dib = sample_dib();
        dib[14] = 8; // 8 bpp z paleta
        assert!(parse_dib(&dib).is_err());
    }
}
