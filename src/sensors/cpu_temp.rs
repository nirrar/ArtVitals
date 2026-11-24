//! Temperatura i pobor mocy pakietu CPU przez sterownik PawnIO - wybor implementacji wg producenta.
//!
//! Producent, rodzina i model z CPUID (`GenuineIntel` / `AuthenticAMD`); AMD obsluguje `cpu_amd.rs`.
//! Intel (modul `IntelMSR`):
//! Temperatura: TjMax z MSR_IA32_TEMPERATURE_TARGET (0x1A2, bity 23:16) minus
//! odleglosc od TjMax z MSR_IA32_PACKAGE_THERM_STATUS (0x1B1, bity 22:16).
//! Moc: przyrost licznika energii MSR_PKG_ENERGY_STATUS (0x611, 32 bity, zawija sie)
//! w jednostkach z MSR_RAPL_POWER_UNIT (0x606, bity 12:8 = 1/2^n J) podzielony przez czas.
//! `EnergyMeter` (ta sama logika licznika) jest wspolny z AMD Zen.
//! Wymaga: zainstalowanego PawnIO (`%ProgramFiles%\PawnIO\PawnIOLib.dll`)
//! oraz uprawnien administratora (urzadzenie sterownika jest dostepne tylko dla adminow).
//! Modul `IntelMSR.bin` (LGPL-2.1, PawnIO.Modules 0.2.12) jest wkompilowany w plik exe.

use std::time::Instant;

use super::cpu_amd::{AmdCpuInfo, AmdSensor};
use crate::i18n::tr;
use super::pawnio::PawnIo;

// PawnIO.Modules 0.2.12, SHA-256 30a5a302283de1e080dfee99e686eca91d7e53e911095cb91a320fb961f1ce12
const INTEL_MSR_MODULE: &[u8] = include_bytes!("../../assets/IntelMSR.bin");

const MSR_IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;
const MSR_IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_RAPL_POWER_UNIT: u64 = 0x606;
const MSR_PKG_ENERGY_STATUS: u64 = 0x611;

pub struct CpuTempSensor {
    backend: Backend,
}

enum Backend {
    Intel(IntelSensor),
    Amd(AmdSensor),
}

impl CpuTempSensor {
    pub fn new() -> Result<Self, String> {
        let id = CpuIdentity::detect().ok_or(tr("Brak instrukcji CPUID (nieobslugiwana architektura)", "No CPUID instruction (unsupported architecture)"))?;
        log::info!("CPU: {} rodzina {:X}h model {:X}h, {}", id.vendor, id.family, id.model, id.brand);
        let backend = match id.vendor.as_str() {
            "GenuineIntel" => Backend::Intel(IntelSensor::new()?),
            "AuthenticAMD" => Backend::Amd(AmdSensor::new(&AmdCpuInfo {
                family: id.family,
                model: id.model,
                brand: &id.brand,
                has_thermal_sensor: id.amd_thermal_sensor,
                core_count: id.amd_core_count,
            })?),
            other => return Err(format!("{} ({other})", tr("Nieobslugiwany producent procesora", "Unsupported CPU vendor"))),
        };
        Ok(Self { backend })
    }

    pub fn read(&mut self) -> Option<f32> {
        match &mut self.backend {
            Backend::Intel(sensor) => sensor.read(),
            Backend::Amd(sensor) => sensor.read(),
        }
    }

    /// Sredni pobor mocy pakietu (W) od poprzedniego odczytu; pierwszy odczyt daje `None`.
    pub fn read_power(&mut self) -> Option<f32> {
        match &mut self.backend {
            Backend::Intel(sensor) => sensor.read_power(),
            Backend::Amd(sensor) => sensor.read_power(),
        }
    }
}

struct IntelSensor {
    pawnio: PawnIo,
    tj_max: f32,
    /// None = brak RAPL.
    energy: Option<EnergyMeter>,
}

impl IntelSensor {
    fn new() -> Result<Self, String> {
        let pawnio = PawnIo::open(INTEL_MSR_MODULE, "IntelMSR")?;
        let mut sensor = Self { pawnio, tj_max: 100.0, energy: None };

        let target = sensor.read_msr(MSR_IA32_TEMPERATURE_TARGET)?;
        let tj_max = ((target >> 16) & 0xFF) as f32;
        // Zerowy TjMax oznacza, ze BIOS go nie raportuje - 100 C to typowa wartosc dla Intela.
        sensor.tj_max = if tj_max > 0.0 { tj_max } else { 100.0 };
        log::info!("PawnIO gotowe, TjMax = {} C", sensor.tj_max);
        match sensor.read_msr(MSR_RAPL_POWER_UNIT) {
            Ok(units) => sensor.energy = Some(EnergyMeter::from_power_unit_msr(units)),
            Err(err) => log::warn!("Pobor mocy CPU niedostepny: {err}"),
        }
        Ok(sensor)
    }

    fn read_msr(&self, msr: u64) -> Result<u64, String> {
        self.pawnio
            .execute_one(c"ioctl_read_msr", &[msr])
            .map_err(|err| format!("{} 0x{msr:X} {}: {err}", tr("Odczyt MSR", "MSR"), tr("nieudany", "read failed")))
    }

    fn read(&mut self) -> Option<f32> {
        let status = self.read_msr(MSR_IA32_PACKAGE_THERM_STATUS).ok()?;
        let delta = ((status >> 16) & 0x7F) as f32;
        let temp = self.tj_max - delta;
        (temp > 0.0 && temp < 150.0).then_some(temp)
    }

    fn read_power(&mut self) -> Option<f32> {
        self.energy.as_ref()?;
        let raw = self.read_msr(MSR_PKG_ENERGY_STATUS).ok()?;
        self.energy.as_mut()?.sample(raw, Instant::now())
    }
}

/// Licznik energii pakietu (RAPL Intela i AMD Zen maja ten sam uklad): 32-bitowy, zawijajacy sie,
/// w jednostkach 1/2^ESU J, gdzie ESU = bity 12:8 rejestru jednostek mocy.
pub(super) struct EnergyMeter {
    unit_j: f64,
    last: Option<(u32, Instant)>,
}

impl EnergyMeter {
    pub(super) fn from_power_unit_msr(units: u64) -> Self {
        Self { unit_j: energy_unit_joules(units), last: None }
    }

    /// Zapisuje probke licznika i zwraca srednia moc od poprzedniej; pierwsza probka daje `None`.
    pub(super) fn sample(&mut self, raw: u64, now: Instant) -> Option<f32> {
        let counter = (raw & 0xFFFF_FFFF) as u32;
        let (last_counter, last_time) = self.last.replace((counter, now))?;
        average_watts(last_counter, counter, self.unit_j, now.duration_since(last_time).as_secs_f64())
    }
}

fn energy_unit_joules(units: u64) -> f64 {
    1.0 / f64::from(1u32 << ((units >> 8) & 0x1F))
}

fn average_watts(last_counter: u32, counter: u32, unit_j: f64, seconds: f64) -> Option<f32> {
    if seconds < 0.05 {
        return None;
    }
    // wrapping_sub obsluguje przepelnienie 32-bitowego licznika.
    let joules = f64::from(counter.wrapping_sub(last_counter)) * unit_j;
    let watts = (joules / seconds) as f32;
    (watts.is_finite() && watts < 1000.0).then_some(watts)
}

/// Producent i identyfikacja procesora z CPUID.
struct CpuIdentity {
    vendor: String,
    family: u32,
    model: u32,
    brand: String,
    /// CPUID 0x80000007 EDX bit 0 (AMD: czujnik temperatury, uzywane dla K8).
    amd_thermal_sensor: bool,
    /// CPUID 0x80000008 ECX[7:0] + 1 (AMD: liczba rdzeni, uzywane dla K8).
    amd_core_count: u32,
}

impl CpuIdentity {
    #[cfg(target_arch = "x86_64")]
    fn detect() -> Option<Self> {
        use std::arch::x86_64::__cpuid;

        let leaf0 = __cpuid(0);
        let vendor = vendor_string(leaf0.ebx, leaf0.edx, leaf0.ecx);
        let (family, model) = if leaf0.eax >= 1 { decode_family_model(__cpuid(1).eax) } else { (0, 0) };
        let max_ext = __cpuid(0x8000_0000).eax;
        let mut brand_bytes = Vec::with_capacity(48);
        if max_ext >= 0x8000_0004 {
            for leaf in 0x8000_0002..=0x8000_0004u32 {
                let r = __cpuid(leaf);
                for reg in [r.eax, r.ebx, r.ecx, r.edx] {
                    brand_bytes.extend_from_slice(&reg.to_le_bytes());
                }
            }
        }
        let brand = String::from_utf8_lossy(&brand_bytes).replace('\0', " ").trim().to_owned();
        let amd_thermal_sensor = max_ext >= 0x8000_0007 && __cpuid(0x8000_0007).edx & 1 != 0;
        let amd_core_count = if max_ext >= 0x8000_0008 { (__cpuid(0x8000_0008).ecx & 0xFF) + 1 } else { 1 };
        Some(Self { vendor, family, model, brand, amd_thermal_sensor, amd_core_count })
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn detect() -> Option<Self> {
        None
    }
}

fn vendor_string(ebx: u32, edx: u32, ecx: u32) -> String {
    let bytes: Vec<u8> = [ebx, edx, ecx].iter().flat_map(|r| r.to_le_bytes()).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Rodzina i model z CPUID(1).EAX wg dokumentacji Intel/AMD: rodzina rozszerzona tylko gdy
/// bazowa = 0Fh, model rozszerzony gdy bazowa rodzina = 06h lub 0Fh.
fn decode_family_model(eax: u32) -> (u32, u32) {
    let base_family = (eax >> 8) & 0xF;
    let base_model = (eax >> 4) & 0xF;
    let family = if base_family == 0xF { base_family + ((eax >> 20) & 0xFF) } else { base_family };
    let model =
        if base_family == 0x6 || base_family == 0xF { (((eax >> 16) & 0xF) << 4) | base_model } else { base_model };
    (family, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_model_decoding() {
        // Ryzen 7 7800X3D: CPUID(1).EAX = 0x00A60F12 -> rodzina 19h, model 61h
        assert_eq!(decode_family_model(0x00A6_0F12), (0x19, 0x61));
        // Ryzen 5 3600: 0x00870F10 -> 17h, 71h
        assert_eq!(decode_family_model(0x0087_0F10), (0x17, 0x71));
        // Ryzen 9000 (Granite Ridge): 0x00B40F40 -> 1Ah, 44h
        assert_eq!(decode_family_model(0x00B4_0F40), (0x1A, 0x44));
        // FX-8350: 0x00600F20 -> 15h, 02h
        assert_eq!(decode_family_model(0x0060_0F20), (0x15, 0x02));
        // i5-14600KF: 0x000B0671 -> 06h, B7h
        assert_eq!(decode_family_model(0x000B_0671), (0x06, 0xB7));
    }

    #[test]
    fn vendor_from_registers() {
        // "AuthenticAMD": EBX="Auth", EDX="enti", ECX="cAMD"
        assert_eq!(vendor_string(0x6874_7541, 0x6974_6E65, 0x444D_4163), "AuthenticAMD");
        assert_eq!(vendor_string(0x756E_6547, 0x4965_6E69, 0x6C65_746E), "GenuineIntel");
    }

    #[test]
    fn energy_unit_from_msr() {
        // Typowe ESU = 14 (AMD Zen) i 14 (Intel) -> 61 uJ; ESU = 16 -> 15.3 uJ.
        assert_eq!(energy_unit_joules(0x000A_0E03), 1.0 / 16384.0);
        assert_eq!(energy_unit_joules(0x0000_1000), 1.0 / 65536.0);
    }

    #[test]
    fn watts_from_counter_delta() {
        let unit = 1.0 / 16384.0;
        // 16384 * 50 jednostek w 1 s = 50 W
        assert_eq!(average_watts(1000, 1000 + 16384 * 50, unit, 1.0), Some(50.0));
        // Za krotki odstep - brak wyniku.
        assert_eq!(average_watts(0, 100, unit, 0.01), None);
        // Nierealna wartosc (>= 1000 W) odrzucana.
        assert_eq!(average_watts(0, u32::MAX, unit, 0.1), None);
    }

    #[test]
    fn watts_across_counter_wrap() {
        let unit = 1.0 / 16384.0;
        // Licznik przeskoczyl z 0xFFFF_F000 przez zero do 0x0000_1000: przyrost 0x2000 jednostek.
        let watts = average_watts(0xFFFF_F000, 0x0000_1000, unit, 0.5).unwrap();
        assert_eq!(watts, (0x2000 as f64 / 16384.0 / 0.5) as f32);
    }

    #[test]
    fn energy_meter_first_sample_is_none() {
        let mut meter = EnergyMeter::from_power_unit_msr(0x0000_0E00);
        let start = Instant::now();
        assert_eq!(meter.sample(0xABCD_0000_0000_0010, start), None);
        // Gorne 32 bity sa ignorowane.
        let later = start + std::time::Duration::from_secs(1);
        assert_eq!(meter.sample(0x1234_0000_0000_4010, later), Some(1.0));
    }
}
