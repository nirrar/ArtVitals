//! Obciazenie i zegar CPU z licznikow wydajnosci Windows (PDH).
//!
//! Obciazenie: `\Processor Information(_Total)\% Processor Utility` - ten sam licznik co
//! Menedzer zadan (uwzglednia boost, wiec bywa >100% - przycinamy).
//! Zegar: `Processor Frequency` (bazowy, MHz) x `% Processor Performance` / 100 - tak samo
//! liczy "Szybkosc" Menedzer zadan (uwzglednia turbo).
//! Liczniki sa roznicowe: pierwszy odczyt po otwarciu zapytania nie ma wartosci.
//! Nie wymaga uprawnien administratora.

use windows::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery,
    PdhCollectQueryData, PdhGetFormattedCounterValue, PdhOpenQueryW,
};
use windows::core::{PCWSTR, w};

const ERROR_SUCCESS: u32 = 0;

#[derive(Debug, Default, Clone, Copy)]
pub struct CpuLoadReading {
    pub load_pct: Option<f32>,
    pub clock_mhz: Option<f32>,
}

pub struct CpuLoadSensor {
    query: PDH_HQUERY,
    utility: PDH_HCOUNTER,
    /// Liczniki zegara sa opcjonalne - brak ich nie wylacza odczytu obciazenia.
    frequency: Option<PDH_HCOUNTER>,
    performance: Option<PDH_HCOUNTER>,
}

// Uchwyty PDH sa uzywane wylacznie z watku czujnikow, do ktorego sensor zostaje przeniesiony.
unsafe impl Send for CpuLoadSensor {}

impl CpuLoadSensor {
    pub fn new() -> Result<Self, String> {
        let mut query = PDH_HQUERY::default();
        unsafe {
            let status = PdhOpenQueryW(None, 0, &mut query);
            if status != ERROR_SUCCESS {
                return Err(format!("PdhOpenQueryW: 0x{status:08X}"));
            }
            let utility = match add_counter(query, w!("\\Processor Information(_Total)\\% Processor Utility")) {
                Ok(counter) => counter,
                Err(status) => {
                    PdhCloseQuery(query);
                    return Err(format!("PdhAddEnglishCounterW: 0x{status:08X}"));
                }
            };
            let frequency = add_counter(query, w!("\\Processor Information(_Total)\\Processor Frequency")).ok();
            let performance =
                add_counter(query, w!("\\Processor Information(_Total)\\% Processor Performance")).ok();
            if frequency.is_none() || performance.is_none() {
                log::warn!("Liczniki zegara CPU niedostepne - zegar CPU nie bedzie pokazywany");
            }
            // Pierwsza probka bazowa - wartosci pojawia sie przy kolejnym odczycie.
            PdhCollectQueryData(query);
            Ok(Self { query, utility, frequency, performance })
        }
    }

    pub fn read(&mut self) -> CpuLoadReading {
        unsafe {
            if PdhCollectQueryData(self.query) != ERROR_SUCCESS {
                return CpuLoadReading::default();
            }
        }
        let load_pct = read_counter(self.utility).map(|v| v.clamp(0.0, 100.0));
        let clock_mhz = match (self.frequency.and_then(read_counter), self.performance.and_then(read_counter)) {
            (Some(base), Some(perf)) if base > 0.0 => Some(base * perf / 100.0),
            _ => None,
        };
        CpuLoadReading { load_pct, clock_mhz }
    }
}

unsafe fn add_counter(query: PDH_HQUERY, path: PCWSTR) -> Result<PDH_HCOUNTER, u32> {
    let mut counter = PDH_HCOUNTER::default();
    let status = unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) };
    if status == ERROR_SUCCESS { Ok(counter) } else { Err(status) }
}

fn read_counter(counter: PDH_HCOUNTER) -> Option<f32> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status = unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut value) };
    if status != ERROR_SUCCESS {
        return None;
    }
    let v = unsafe { value.Anonymous.doubleValue } as f32;
    v.is_finite().then_some(v)
}

impl Drop for CpuLoadSensor {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
