//! Wspolna obsluga sterownika PawnIO (`%ProgramFiles%\PawnIO\PawnIOLib.dll`).
//!
//! Kazdy `PawnIo` to osobny uchwyt z zaladowanym jednym modulem (np. `IntelMSR.bin`,
//! `AMDFamily17.bin`, `LpcIO.bin`). Funkcje modulu wywolywane sa po nazwie (`ioctl_*`)
//! z wejsciem i wyjsciem jako tablice u64 - sygnatury z PawnIOLib.h.
//! Wymaga zainstalowanego PawnIO i uprawnien administratora.

use std::ffi::CStr;
use std::path::PathBuf;

use windows::Win32::Foundation::{FreeLibrary, HANDLE, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{HRESULT, HSTRING, PCSTR};

type OpenFn = unsafe extern "system" fn(*mut HANDLE) -> HRESULT;
type LoadFn = unsafe extern "system" fn(HANDLE, *const u8, usize) -> HRESULT;
type ExecuteFn =
    unsafe extern "system" fn(HANDLE, PCSTR, *const u64, usize, *mut u64, usize, *mut usize) -> HRESULT;
type CloseFn = unsafe extern "system" fn(HANDLE) -> HRESULT;

/// E_ACCESSDENIED - typowy przypadek uruchomienia bez admina.
const E_ACCESS_DENIED: u32 = 0x8007_0005;

pub struct PawnIo {
    library: HMODULE,
    handle: HANDLE,
    execute: ExecuteFn,
    close: CloseFn,
}

// Uchwyt PawnIO jest uzywany wylacznie z jednego watku naraz (watek czujnikow).
unsafe impl Send for PawnIo {}

fn pawnio_dll_path() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
        .join("PawnIO")
        .join("PawnIOLib.dll")
}

unsafe fn proc<T>(library: HMODULE, name: &'static CStr) -> Result<T, String> {
    let ptr = unsafe { GetProcAddress(library, PCSTR(name.as_ptr().cast())) };
    match ptr {
        // FARPROC i wskaznik do funkcji maja ten sam rozmiar; sygnatury pochodza z PawnIOLib.h.
        Some(f) => Ok(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, T>(&f) }),
        None => Err(format!("Brak funkcji {} w PawnIOLib.dll", name.to_string_lossy())),
    }
}

impl PawnIo {
    /// Otwiera nowy uchwyt PawnIO i laduje do niego `module` (plik .bin z PawnIO.Modules).
    /// `module_name` sluzy tylko do komunikatow bledow.
    pub fn open(module: &[u8], module_name: &str) -> Result<Self, String> {
        let dll = pawnio_dll_path();
        if !dll.exists() {
            return Err(format!("Nie znaleziono PawnIO ({})", dll.display()));
        }
        let library = unsafe { LoadLibraryW(&HSTRING::from(dll.as_os_str())) }
            .map_err(|err| format!("Nie mozna zaladowac PawnIOLib.dll: {err}"))?;
        match unsafe { Self::init(library, module, module_name) } {
            Ok(pawnio) => Ok(pawnio),
            Err(err) => {
                unsafe {
                    let _ = FreeLibrary(library);
                }
                Err(err)
            }
        }
    }

    unsafe fn init(library: HMODULE, module: &[u8], module_name: &str) -> Result<Self, String> {
        let open: OpenFn = unsafe { proc(library, c"pawnio_open")? };
        let load: LoadFn = unsafe { proc(library, c"pawnio_load")? };
        let execute: ExecuteFn = unsafe { proc(library, c"pawnio_execute")? };
        let close: CloseFn = unsafe { proc(library, c"pawnio_close")? };

        let mut handle = HANDLE::default();
        let hr = unsafe { open(&mut handle) };
        if hr.is_err() {
            return Err(if hr.0 as u32 == E_ACCESS_DENIED {
                "Wymaga uruchomienia jako administrator".to_owned()
            } else {
                format!("pawnio_open: {hr:?}")
            });
        }
        let hr = unsafe { load(handle, module.as_ptr(), module.len()) };
        if hr.is_err() {
            unsafe {
                let _ = close(handle);
            }
            return Err(format!("Modul {module_name} nie zaladowal sie (inny sprzet?): {hr:?}"));
        }
        Ok(Self { library, handle, execute, close })
    }

    /// Wywoluje funkcje modulu; zwraca tyle wartosci, ile modul zapisal (max `output_len`).
    pub fn execute(&self, function: &CStr, input: &[u64], output_len: usize) -> Result<Vec<u64>, String> {
        let mut output = vec![0u64; output_len];
        let mut written = 0usize;
        let hr = unsafe {
            (self.execute)(
                self.handle,
                PCSTR(function.as_ptr().cast()),
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        };
        if hr.is_err() {
            return Err(format!("{}: {hr:?}", function.to_string_lossy()));
        }
        output.truncate(written.min(output_len));
        Ok(output)
    }

    /// Jak `execute`, ale oczekuje dokladnie jednej wartosci wyniku.
    pub fn execute_one(&self, function: &CStr, input: &[u64]) -> Result<u64, String> {
        let out = self.execute(function, input, 1)?;
        out.first().copied().ok_or_else(|| format!("{}: brak wyniku", function.to_string_lossy()))
    }
}

impl Drop for PawnIo {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.close)(self.handle);
            let _ = FreeLibrary(self.library);
        }
    }
}
