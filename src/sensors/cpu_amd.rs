//! Temperatura i pobor mocy pakietu CPU AMD przez sterownik PawnIO (moduly `AMDFamily*` z PawnIO.Modules).
//!
//! Logika odczytu przeniesiona z LibreHardwareMonitor (MPL-2.0), `Hardware/Cpu/Amd17Cpu.cs`,
//! `Amd10Cpu.cs`, `Amd0FCpu.cs`, oraz linuksowego `drivers/hwmon/k10temp.c`:
//! - Zen (rodziny 17h, 19h, 1Ah), modul `AMDFamily17.bin`: temperatura z rejestru SMN
//!   THM_TCON_CUR_TMP (0x00059800, bity 31:21 * 0.125 C, -49 C przy RANGE_SEL lub TJ_SEL=11b),
//!   dla procesorow z przesunieciem Tctl (1600X/1700X/1800X/2700X/Threadripper 19xx/29xx) Tdie = Tctl - offset;
//!   moc z licznika energii MSR 0xC001029B w jednostkach z MSR 0xC0010299 (jak RAPL Intela).
//! - Rodziny 10h-16h, modul `AMDFamily10.bin`: temperatura z PCI D18F3xA4 (Reported Temperature Control)
//!   albo, dla 15h modele 60h-7Fh, z rejestru SMU 0xD8200CA4. Brak pomiaru mocy.
//! - Rodzina 0Fh (K8), modul `AMDFamily0F.bin`: rejestr THERMTRIP (D18F3xE4) dla rdzeni 0 i 1. Brak mocy.
//!
//! Odczyty przez PCI/SMN sa chronione globalnym mutexem `Global\Access_PCI` (konwencja LHM/HWiNFO),
//! zeby nie kolidowac z innymi programami monitorujacymi.
//! Wymaga: PawnIO, uprawnien administratora. Pliki `.bin` (LGPL-2.1) sa wkompilowane w exe.

use std::ffi::CStr;
use std::time::Instant;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED_0, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateMutexW, MUTEX_MODIFY_STATE, OpenMutexW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::w;

use super::cpu_temp::EnergyMeter;
use super::pawnio::PawnIo;
use crate::i18n::tr;

// Moduly z oficjalnego wydania PawnIO.Modules 0.2.12 (to samo wydanie co IntelMSR.bin), SHA-256:
// AMDFamily17.bin dae74615761b78bdf064dfb3e136252ddcc6fc727d88f14738d0e5800d427a91
// AMDFamily10.bin 6443080b2968474ffbc38aa4356cc56f9664349fa4b917afdb33027d6bb50525
// AMDFamily0F.bin a6e11619e87a97820705a6523714f22d676ce44f902631833d4429b89d509d55
const AMD_FAMILY17_MODULE: &[u8] = include_bytes!("../../assets/AMDFamily17.bin");
const AMD_FAMILY10_MODULE: &[u8] = include_bytes!("../../assets/AMDFamily10.bin");
const AMD_FAMILY0F_MODULE: &[u8] = include_bytes!("../../assets/AMDFamily0F.bin");

// Zen (Amd17Cpu.cs).
const F17H_M01H_THM_TCON_CUR_TMP: u64 = 0x0005_9800;
const F17H_TEMP_RANGE_SEL_MASK: u32 = 0x8_0000;
const F17H_TEMP_TJ_SEL_MASK: u32 = 0x3_0000;
const MSR_PWR_UNIT: u64 = 0xC001_0299;
const MSR_PKG_ENERGY_STAT: u64 = 0xC001_029B;

// Rodziny 10h-16h (Amd10Cpu.cs).
const REPORTED_TEMPERATURE_CONTROL_REGISTER: u64 = 0xA4;
const SMU_REPORTED_TEMP_CTRL_OFFSET: u64 = 0xD820_0CA4;

/// Czas oczekiwania na mutex PCI - jak `Mutexes.WaitPciBus(10)` w LHM.
const PCI_MUTEX_TIMEOUT_MS: u32 = 10;

/// Informacje z CPUID potrzebne do wyboru i konfiguracji modulu.
pub struct AmdCpuInfo<'a> {
    pub family: u32,
    pub model: u32,
    pub brand: &'a str,
    /// CPUID 0x80000007 EDX bit 0 (TS) - cyfrowy czujnik temperatury (tylko K8).
    pub has_thermal_sensor: bool,
    /// Liczba rdzeni z CPUID 0x80000008 ECX[7:0] + 1 (tylko K8).
    pub core_count: u32,
}

enum Kind {
    /// Rodziny 17h/19h/1Ah. `tdie_offset` <= 0 (Tdie = Tctl + offset).
    Zen { tdie_offset: f32 },
    /// Rodziny 10h-16h; `use_smu` dla 15h modele 60h-7Fh.
    Family10 { family: u32, model: u32, use_smu: bool },
    /// Rodzina 0Fh; `offset` z Amd0FCpu.cs, `cores` = 1 albo 2.
    Family0F { offset: f32, cores: u64 },
}

pub struct AmdSensor {
    pawnio: PawnIo,
    kind: Kind,
    pci_mutex: Option<PciMutex>,
    energy: Option<EnergyMeter>,
}

impl AmdSensor {
    pub fn new(info: &AmdCpuInfo) -> Result<Self, String> {
        let (module, name, kind) = match info.family {
            0x17 | 0x19 | 0x1A => {
                (AMD_FAMILY17_MODULE, "AMDFamily17", Kind::Zen { tdie_offset: zen_tdie_offset(info.brand) })
            }
            0x10..=0x16 => {
                let use_smu = info.family == 0x15 && matches!(info.model & 0xF0, 0x60 | 0x70);
                (AMD_FAMILY10_MODULE, "AMDFamily10", Kind::Family10 { family: info.family, model: info.model, use_smu })
            }
            0x0F => {
                if !info.has_thermal_sensor {
                    return Err(tr(
                        "Ten procesor AMD (rodzina 0Fh) nie ma cyfrowego czujnika temperatury",
                        "This AMD CPU (family 0Fh) has no digital temperature sensor",
                    )
                    .to_owned());
                }
                let cores = u64::from(info.core_count.clamp(1, 2));
                (AMD_FAMILY0F_MODULE, "AMDFamily0F", Kind::Family0F { offset: k8_temp_offset(info.model), cores })
            }
            family => {
                return Err(format!(
                    "{} {family:X}h {}",
                    tr("Procesor AMD rodziny", "AMD CPU family"),
                    tr(
                        "nie jest obslugiwany (temperatura i moc niedostepne)",
                        "is not supported (temperature and power unavailable)"
                    )
                ));
            }
        };
        let pawnio = PawnIo::open(module, name)?;
        let pci_mutex = PciMutex::open();
        let mut sensor = Self { pawnio, kind, pci_mutex, energy: None };

        // Probny odczyt - blad (np. odmowa dostepu modulu) trafia do okna "Czujniki".
        sensor.read_raw_temp()?;
        if matches!(sensor.kind, Kind::Zen { .. }) {
            match sensor.read_msr(MSR_PWR_UNIT) {
                Ok(units) => sensor.energy = Some(EnergyMeter::from_power_unit_msr(units)),
                Err(err) => log::warn!("Pobor mocy CPU AMD niedostepny: {err}"),
            }
        }
        log::info!("PawnIO gotowe ({name}), CPU AMD rodzina {:X}h model {:X}h", info.family, info.model);
        Ok(sensor)
    }

    fn read_msr(&self, msr: u64) -> Result<u64, String> {
        self.pawnio
            .execute_one(c"ioctl_read_msr", &[msr])
            .map_err(|err| format!("{} 0x{msr:X} {}: {err}", tr("Odczyt MSR", "MSR"), tr("nieudany", "read failed")))
    }

    fn execute_u32(&self, function: &CStr, input: &[u64]) -> Result<u32, String> {
        // Rejestry PCI/SMN sa 32-bitowe; modul zwraca je w dolnej polowie u64.
        Ok((self.pawnio.execute_one(function, input)? & 0xFFFF_FFFF) as u32)
    }

    /// Surowy odczyt temperatury pakietu w C. `Ok(None)` = mutex PCI zajety (pominieta probka).
    fn read_raw_temp(&self) -> Result<Option<f32>, String> {
        match self.kind {
            Kind::Zen { tdie_offset } => {
                let Some(_guard) = self.lock_pci() else { return Ok(None) };
                let raw = self.execute_u32(c"ioctl_read_smn", &[F17H_M01H_THM_TCON_CUR_TMP])?;
                Ok(Some(decode_zen_tctl(raw) + tdie_offset))
            }
            Kind::Family10 { family, model, use_smu } => {
                let raw = if use_smu {
                    let Some(_guard) = self.lock_pci() else { return Ok(None) };
                    self.execute_u32(c"ioctl_read_smu", &[SMU_REPORTED_TEMP_CTRL_OFFSET])?
                } else {
                    // Indeks procesora (gniazda) 0; LHM nie blokuje tu mutexu PCI.
                    self.execute_u32(c"ioctl_read_miscctl", &[0, REPORTED_TEMPERATURE_CONTROL_REGISTER])?
                };
                Ok(Some(decode_family10_temp(raw, family, model)))
            }
            Kind::Family0F { offset, cores } => {
                let Some(_guard) = self.lock_pci() else { return Ok(None) };
                let mut hottest: Option<f32> = None;
                let mut last_err = None;
                for core in 0..cores {
                    match self.execute_u32(c"ioctl_get_thermtrip", &[0, core]) {
                        Ok(raw) => {
                            let temp = decode_k8_temp(raw, offset);
                            hottest = Some(hottest.map_or(temp, |t| t.max(temp)));
                        }
                        Err(err) => last_err = Some(err),
                    }
                }
                match (hottest, last_err) {
                    (Some(temp), _) => Ok(Some(temp)),
                    (None, Some(err)) => Err(err),
                    (None, None) => Ok(None),
                }
            }
        }
    }

    pub fn read(&mut self) -> Option<f32> {
        let temp = self.read_raw_temp().ok()??;
        (temp > 0.0 && temp < 150.0).then_some(temp)
    }

    /// Sredni pobor mocy pakietu (W) od poprzedniego odczytu; tylko Zen, pierwszy odczyt daje `None`.
    pub fn read_power(&mut self) -> Option<f32> {
        self.energy.as_ref()?;
        let raw = self.read_msr(MSR_PKG_ENERGY_STAT).ok()?;
        self.energy.as_mut()?.sample(raw, Instant::now())
    }

    fn lock_pci(&self) -> Option<PciGuard<'_>> {
        match &self.pci_mutex {
            Some(mutex) => mutex.lock(),
            // Bez mutexu (nie udalo sie go utworzyc/otworzyc) czytamy jak LHM - bez blokady.
            None => Some(PciGuard(None)),
        }
    }
}

/// Tctl z surowej wartosci THM_TCON_CUR_TMP (Amd17Cpu.cs / k10temp `get_raw_temp`).
fn decode_zen_tctl(raw: u32) -> f32 {
    let adjust = raw & F17H_TEMP_RANGE_SEL_MASK != 0 || raw & F17H_TEMP_TJ_SEL_MASK == F17H_TEMP_TJ_SEL_MASK;
    // LHM: (raw >> 21) * 125 * 0.001; dzielenie przez 8 daje to samo bez bledu zaokraglenia.
    let temp = (raw >> 21) as f32 / 8.0;
    if adjust { temp - 49.0 } else { temp }
}

/// Przesuniecie Tdie wzgledem Tctl (tabela `tctl_offset_table` z k10temp, nazwy jak w LHM).
/// Dla pozostalych procesorow Tctl = Tdie - LHM pokazuje wtedy czujnik "Core (Tctl/Tdie)".
fn zen_tdie_offset(brand: &str) -> f32 {
    if ["1600X", "1700X", "1800X"].iter().any(|n| brand.contains(n)) {
        -20.0
    } else if brand.contains("Threadripper 19") || brand.contains("Threadripper 29") {
        -27.0
    } else if brand.contains("2700X") {
        -10.0
    } else {
        0.0
    }
}

/// Temperatura rodzin 10h-16h (Amd10Cpu.cs). LHM porownuje `(value & 0x30000) == 0x3000`, co nigdy
/// nie jest prawda (literowka); stosujemy zamierzony warunek TJ_SEL == 11b, zgodny z k10temp.
fn decode_family10_temp(raw: u32, family: u32, model: u32) -> f32 {
    if (family == 0x15 || family == 0x16) && raw & 0x3_0000 == 0x3_0000 {
        let mask = if family == 0x15 && model & 0xF0 == 0x00 { 0x7FC } else { 0x7FF };
        ((raw >> 21) & mask) as f32 / 8.0 - 49.0
    } else {
        ((raw >> 21) & 0x7FF) as f32 / 8.0
    }
}

/// Przesuniecie temperatury K8 (Amd0FCpu.cs): -49 C, a dla 65 nm AM2+ dodatkowo +21 C.
fn k8_temp_offset(model: u32) -> f32 {
    if model >= 0x69 && !matches!(model, 0xC1 | 0x6C | 0x7C) { -28.0 } else { -49.0 }
}

/// Temperatura rdzenia K8 z rejestru THERMTRIP (bity 23:16).
fn decode_k8_temp(raw: u32, offset: f32) -> f32 {
    ((raw >> 16) & 0xFF) as f32 + offset
}

/// Globalny mutex `Global\Access_PCI` - wspolny z LHM i innymi narzedziami (Mutexes.cs).
struct PciMutex(HANDLE);

// Uchwyt jest uzywany tylko z watku czujnikow.
unsafe impl Send for PciMutex {}

impl PciMutex {
    fn open() -> Option<Self> {
        let name = w!("Global\\Access_PCI");
        let handle = unsafe { CreateMutexW(None, false, name) }
            .or_else(|_| unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE, false, name) });
        match handle {
            Ok(handle) => Some(Self(handle)),
            Err(err) => {
                log::warn!("Mutex Global\\Access_PCI niedostepny, odczyty PCI bez blokady: {err}");
                None
            }
        }
    }

    fn lock(&self) -> Option<PciGuard<'_>> {
        let result = unsafe { WaitForSingleObject(self.0, PCI_MUTEX_TIMEOUT_MS) };
        // Porzucony mutex (inny proces padl) tez oznacza, ze go mamy - jak w LHM.
        (result == WAIT_OBJECT_0 || result == WAIT_ABANDONED_0).then_some(PciGuard(Some(self)))
    }
}

impl Drop for PciMutex {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct PciGuard<'a>(Option<&'a PciMutex>);

impl Drop for PciGuard<'_> {
    fn drop(&mut self) {
        if let Some(mutex) = self.0 {
            unsafe {
                let _ = ReleaseMutex(mutex.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sklada surowy THM_TCON_CUR_TMP: temperatura w krokach 0.125 C na bitach 31:21.
    fn zen_raw(eighths: u32, flags: u32) -> u32 {
        (eighths << 21) | flags
    }

    #[test]
    fn zen_tctl_without_adjust() {
        // 65.5 C = 524 * 0.125
        assert_eq!(decode_zen_tctl(zen_raw(524, 0)), 65.5);
    }

    #[test]
    fn zen_tctl_range_sel_subtracts_49() {
        assert_eq!(decode_zen_tctl(zen_raw(800, F17H_TEMP_RANGE_SEL_MASK)), 51.0);
    }

    #[test]
    fn zen_tctl_tj_sel_both_bits_subtracts_49() {
        assert_eq!(decode_zen_tctl(zen_raw(800, 0x3_0000)), 51.0);
        // Tylko jeden bit TJ_SEL - bez korekty.
        assert_eq!(decode_zen_tctl(zen_raw(800, 0x1_0000)), 100.0);
        assert_eq!(decode_zen_tctl(zen_raw(800, 0x2_0000)), 100.0);
    }

    #[test]
    fn zen_tctl_ignores_low_bits() {
        assert_eq!(decode_zen_tctl(zen_raw(400, 0xFFFF)), 50.0);
    }

    #[test]
    fn zen_tdie_offsets_from_brand() {
        assert_eq!(zen_tdie_offset("AMD Ryzen 7 1800X Eight-Core Processor"), -20.0);
        assert_eq!(zen_tdie_offset("AMD Ryzen 5 1600X Six-Core Processor"), -20.0);
        assert_eq!(zen_tdie_offset("AMD Ryzen 7 2700X Eight-Core Processor"), -10.0);
        assert_eq!(zen_tdie_offset("AMD Ryzen Threadripper 1950X 16-Core Processor"), -27.0);
        assert_eq!(zen_tdie_offset("AMD Ryzen Threadripper 2990WX 32-Core Processor"), -27.0);
        assert_eq!(zen_tdie_offset("AMD Ryzen 7 7800X3D 8-Core Processor"), 0.0);
        assert_eq!(zen_tdie_offset("AMD Ryzen Threadripper 3970X 32-Core Processor"), 0.0);
        assert_eq!(zen_tdie_offset(""), 0.0);
    }

    #[test]
    fn zen_tdie_with_offset_and_range_sel() {
        // 1800X: Tctl = 120 - 49 = 71, Tdie = 51.
        let raw = zen_raw(960, F17H_TEMP_RANGE_SEL_MASK);
        assert_eq!(decode_zen_tctl(raw) + zen_tdie_offset("AMD Ryzen 7 1800X"), 51.0);
    }

    #[test]
    fn family10_plain_value() {
        // 45 C = 360 / 8
        assert_eq!(decode_family10_temp(360 << 21, 0x10, 0x04), 45.0);
        // Na 10h bity TJ_SEL nie maja znaczenia.
        assert_eq!(decode_family10_temp((360 << 21) | 0x3_0000, 0x10, 0x04), 45.0);
    }

    #[test]
    fn family15_tj_sel_subtracts_49_and_masks_model_00() {
        let raw = (803 << 21) | 0x3_0000;
        // Model 30h: pelna maska 0x7FF -> 803/8 - 49
        assert_eq!(decode_family10_temp(raw, 0x15, 0x30), 803.0 / 8.0 - 49.0);
        // Model 02h: maska 0x7FC obcina dwa najmlodsze bity (803 -> 800).
        assert_eq!(decode_family10_temp(raw, 0x15, 0x02), 100.0 - 49.0);
        // Bez TJ_SEL=11b - bez korekty.
        assert_eq!(decode_family10_temp(803 << 21, 0x15, 0x02), 803.0 / 8.0);
    }

    #[test]
    fn k8_offsets_and_decode() {
        assert_eq!(k8_temp_offset(0x43), -49.0);
        assert_eq!(k8_temp_offset(0x6B), -28.0);
        assert_eq!(k8_temp_offset(0x6C), -49.0);
        assert_eq!(k8_temp_offset(0x7C), -49.0);
        assert_eq!(k8_temp_offset(0xC1), -49.0);
        assert_eq!(decode_k8_temp(0x005F_0000, -49.0), 46.0);
        assert_eq!(decode_k8_temp(0xFF5F_FFFF, -49.0), 46.0);
    }
}
