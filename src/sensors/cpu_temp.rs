//! Temperatura i pobor mocy pakietu CPU (Intel) przez sterownik PawnIO i jego modul `IntelMSR`.
//!
//! Temperatura: TjMax z MSR_IA32_TEMPERATURE_TARGET (0x1A2, bity 23:16) minus
//! odleglosc od TjMax z MSR_IA32_PACKAGE_THERM_STATUS (0x1B1, bity 22:16).
//! Moc: przyrost licznika energii MSR_PKG_ENERGY_STATUS (0x611, 32 bity, zawija sie)
//! w jednostkach z MSR_RAPL_POWER_UNIT (0x606, bity 12:8 = 1/2^n J) podzielony przez czas.
//! Wymaga: zainstalowanego PawnIO (`%ProgramFiles%\PawnIO\PawnIOLib.dll`)
//! oraz uprawnien administratora (urzadzenie sterownika jest dostepne tylko dla adminow).
//! Modul `IntelMSR.bin` (LGPL-2.1, PawnIO.Modules) jest wkompilowany w plik exe.
//! Procesory AMD nie sa jeszcze obslugiwane (inny modul i inne rejestry).

use std::ffi::CString;
use std::path::PathBuf;
use std::time::Instant;

use windows::Win32::Foundation::{FreeLibrary, HANDLE, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{HRESULT, HSTRING, PCSTR};

const INTEL_MSR_MODULE: &[u8] = include_bytes!("../../assets/IntelMSR.bin");

const MSR_IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;
const MSR_IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_RAPL_POWER_UNIT: u64 = 0x606;
const MSR_PKG_ENERGY_STATUS: u64 = 0x611;

type OpenFn = unsafe extern "system" fn(*mut HANDLE) -> HRESULT;
type LoadFn = unsafe extern "system" fn(HANDLE, *const u8, usize) -> HRESULT;
type ExecuteFn =
    unsafe extern "system" fn(HANDLE, PCSTR, *const u64, usize, *mut u64, usize, *mut usize) -> HRESULT;
type CloseFn = unsafe extern "system" fn(HANDLE) -> HRESULT;

pub struct CpuTempSensor {
    library: HMODULE,
    handle: HANDLE,
    execute: ExecuteFn,
    close: CloseFn,
    read_msr_name: CString,
    tj_max: f32,
    /// Ile dzuli to jedna jednostka licznika energii (z MSR_RAPL_POWER_UNIT); None = brak RAPL.
    energy_unit_j: Option<f64>,
    last_energy: Option<(u32, Instant)>,
}

// Uchwyt PawnIO jest uzywany wylacznie z watku czujnikow.
unsafe impl Send for CpuTempSensor {}

fn pawnio_dll_path() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
        .join("PawnIO")
        .join("PawnIOLib.dll")
}

unsafe fn proc<T>(library: HMODULE, name: &'static [u8]) -> Result<T, String> {
    debug_assert!(name.ends_with(b"\0"));
    let ptr = unsafe { GetProcAddress(library, PCSTR(name.as_ptr())) };
    match ptr {
        // FARPROC i wskaznik do funkcji maja ten sam rozmiar; sygnatury pochodza z PawnIOLib.h.
        Some(f) => Ok(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, T>(&f) }),
        None => Err(format!(
            "Brak funkcji {} w PawnIOLib.dll",
            String::from_utf8_lossy(&name[..name.len() - 1])
        )),
    }
}

impl CpuTempSensor {
    pub fn new() -> Result<Self, String> {
        let dll = pawnio_dll_path();
        if !dll.exists() {
            return Err(format!("Nie znaleziono PawnIO ({})", dll.display()));
        }
        let library = unsafe { LoadLibraryW(&HSTRING::from(dll.as_os_str())) }
            .map_err(|err| format!("Nie mozna zaladowac PawnIOLib.dll: {err}"))?;

        match unsafe { Self::init(library) } {
            Ok(sensor) => Ok(sensor),
            Err(err) => {
                unsafe {
                    let _ = FreeLibrary(library);
                }
                Err(err)
            }
        }
    }

    unsafe fn init(library: HMODULE) -> Result<Self, String> {
        let open: OpenFn = unsafe { proc(library, b"pawnio_open\0")? };
        let load: LoadFn = unsafe { proc(library, b"pawnio_load\0")? };
        let execute: ExecuteFn = unsafe { proc(library, b"pawnio_execute\0")? };
        let close: CloseFn = unsafe { proc(library, b"pawnio_close\0")? };

        let mut handle = HANDLE::default();
        let hr = unsafe { open(&mut handle) };
        if hr.is_err() {
            // E_ACCESSDENIED to typowy przypadek uruchomienia bez admina.
            return Err(if hr.0 as u32 == 0x8007_0005 {
                "Wymaga uruchomienia jako administrator".to_owned()
            } else {
                format!("pawnio_open: {hr:?}")
            });
        }

        let hr = unsafe { load(handle, INTEL_MSR_MODULE.as_ptr(), INTEL_MSR_MODULE.len()) };
        if hr.is_err() {
            unsafe {
                let _ = close(handle);
            }
            return Err(format!("Modul IntelMSR nie zaladowal sie (procesor inny niz Intel?): {hr:?}"));
        }

        let mut sensor = Self {
            library,
            handle,
            execute,
            close,
            read_msr_name: CString::new("ioctl_read_msr").expect("stala bez bajtow zerowych"),
            tj_max: 100.0,
            energy_unit_j: None,
            last_energy: None,
        };

        let target = sensor.read_msr(MSR_IA32_TEMPERATURE_TARGET)?;
        let tj_max = ((target >> 16) & 0xFF) as f32;
        // Zerowy TjMax oznacza, ze BIOS go nie raportuje - 100 C to typowa wartosc dla Intela.
        sensor.tj_max = if tj_max > 0.0 { tj_max } else { 100.0 };
        log::info!("PawnIO gotowe, TjMax = {} C", sensor.tj_max);
        match sensor.read_msr(MSR_RAPL_POWER_UNIT) {
            Ok(units) => sensor.energy_unit_j = Some(1.0 / f64::from(1u32 << ((units >> 8) & 0x1F))),
            Err(err) => log::warn!("Pobor mocy CPU niedostepny: {err}"),
        }
        Ok(sensor)
    }

    fn read_msr(&self, msr: u64) -> Result<u64, String> {
        let input = [msr];
        let mut output = [0u64; 1];
        let mut written = 0usize;
        let hr = unsafe {
            (self.execute)(
                self.handle,
                PCSTR(self.read_msr_name.as_ptr() as *const u8),
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        };
        if hr.is_err() || written != 1 {
            return Err(format!("Odczyt MSR 0x{msr:X} nieudany: {hr:?}"));
        }
        Ok(output[0])
    }

    pub fn read(&mut self) -> Option<f32> {
        let status = self.read_msr(MSR_IA32_PACKAGE_THERM_STATUS).ok()?;
        let delta = ((status >> 16) & 0x7F) as f32;
        let temp = self.tj_max - delta;
        (temp > 0.0 && temp < 150.0).then_some(temp)
    }

    /// Sredni pobor mocy pakietu (W) od poprzedniego odczytu; pierwszy odczyt daje `None`.
    pub fn read_power(&mut self) -> Option<f32> {
        let unit = self.energy_unit_j?;
        let counter = (self.read_msr(MSR_PKG_ENERGY_STATUS).ok()? & 0xFFFF_FFFF) as u32;
        let now = Instant::now();
        let previous = self.last_energy.replace((counter, now));
        let (last_counter, last_time) = previous?;
        let seconds = now.duration_since(last_time).as_secs_f64();
        if seconds < 0.05 {
            return None;
        }
        // wrapping_sub obsluguje przepelnienie 32-bitowego licznika.
        let joules = f64::from(counter.wrapping_sub(last_counter)) * unit;
        let watts = (joules / seconds) as f32;
        (watts.is_finite() && watts < 1000.0).then_some(watts)
    }
}

impl Drop for CpuTempSensor {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.close)(self.handle);
            let _ = FreeLibrary(self.library);
        }
    }
}
