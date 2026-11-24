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

use std::time::Instant;

use super::pawnio::PawnIo;

const INTEL_MSR_MODULE: &[u8] = include_bytes!("../../assets/IntelMSR.bin");

const MSR_IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;
const MSR_IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_RAPL_POWER_UNIT: u64 = 0x606;
const MSR_PKG_ENERGY_STATUS: u64 = 0x611;

pub struct CpuTempSensor {
    pawnio: PawnIo,
    tj_max: f32,
    /// Ile dzuli to jedna jednostka licznika energii (z MSR_RAPL_POWER_UNIT); None = brak RAPL.
    energy_unit_j: Option<f64>,
    last_energy: Option<(u32, Instant)>,
}

impl CpuTempSensor {
    pub fn new() -> Result<Self, String> {
        let pawnio = PawnIo::open(INTEL_MSR_MODULE, "IntelMSR")?;
        let mut sensor = Self { pawnio, tj_max: 100.0, energy_unit_j: None, last_energy: None };

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
        self.pawnio
            .execute_one(c"ioctl_read_msr", &[msr])
            .map_err(|err| format!("Odczyt MSR 0x{msr:X} nieudany: {err}"))
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
