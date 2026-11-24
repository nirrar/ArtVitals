//! Odczyty GPU NVIDIA: NVML (nvml.dll ze sterownika) + NVAPI dla hotspotu i temperatury pamieci.
//!
//! NVML: temperatura, obciazenie, VRAM, zegar rdzenia i pamieci, pobor mocy, wentylator.
//! NVAPI (`nvapi.rs`): hotspot i temperatura zlacza pamieci - brak ich w publicznym NVML.
//! Obslugiwana jest pierwsza karta (indeks 0). Nie wymaga uprawnien administratora.
//! Karty AMD/Intel nie sa jeszcze obslugiwane.

use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};

use super::nvapi::NvApiThermals;

#[derive(Debug, Default, Clone, Copy)]
pub struct GpuReading {
    pub temp_c: Option<f32>,
    pub hotspot_c: Option<f32>,
    pub memory_temp_c: Option<f32>,
    pub load_pct: Option<f32>,
    pub clock_mhz: Option<f32>,
    pub memory_clock_mhz: Option<f32>,
    pub power_w: Option<f32>,
    pub fan_pct: Option<f32>,
    /// Obroty wentylatora (RPM) - NVAPI/ADL/IGCL, jesli dostepne.
    pub fan_rpm: Option<f32>,
    pub vram_used_mb: Option<f32>,
    pub vram_total_mb: Option<f32>,
}

pub struct GpuSensor {
    nvml: Nvml,
    thermals: Option<NvApiThermals>,
    pub name: String,
    /// Problem z NVAPI do pokazania w oknie konfiguracji (NVML dziala dalej).
    pub thermals_issue: Option<String>,
}

impl GpuSensor {
    pub fn new() -> Result<Self, String> {
        let nvml = Nvml::init().map_err(|err| format!("NVML niedostepne: {err}"))?;
        let name = {
            let device = nvml
                .device_by_index(0)
                .map_err(|err| format!("Brak karty NVIDIA: {err}"))?;
            device.name().unwrap_or_else(|_| "GPU".to_owned())
        };
        log::info!("NVML gotowe: {name}");
        let (thermals, thermals_issue) = match NvApiThermals::new(&name) {
            Ok(thermals) => (Some(thermals), None),
            Err(err) => {
                log::warn!("Hotspot/temperatura pamieci GPU niedostepne: {err}");
                (None, Some(err))
            }
        };
        Ok(Self { nvml, thermals, name, thermals_issue })
    }

    pub fn read(&self) -> GpuReading {
        let Ok(device) = self.nvml.device_by_index(0) else {
            return GpuReading::default();
        };
        let memory = device.memory_info().ok();
        const MB: f32 = 1024.0 * 1024.0;
        let (hotspot_c, memory_temp_c) = self.thermals.as_ref().map(|t| t.read()).unwrap_or_default();
        GpuReading {
            temp_c: device.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32),
            hotspot_c,
            memory_temp_c,
            load_pct: device.utilization_rates().ok().map(|u| u.gpu as f32),
            clock_mhz: device.clock_info(Clock::Graphics).ok().map(|c| c as f32),
            memory_clock_mhz: device.clock_info(Clock::Memory).ok().map(|c| c as f32),
            // NVML podaje moc w miliwatach.
            power_w: device.power_usage().ok().map(|mw| mw as f32 / 1000.0),
            fan_pct: device.fan_speed(0).ok().map(|f| f as f32),
            fan_rpm: None,
            vram_used_mb: memory.as_ref().map(|m| m.used as f32 / MB),
            vram_total_mb: memory.as_ref().map(|m| m.total as f32 / MB),
        }
    }
}
