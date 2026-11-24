//! Odczyty GPU: wybor karty i producenta, wspolny `GpuReading` dla reszty programu.
//!
//! Backendy (biblioteki ze sterownikow, ladowane dynamicznie - bez dodatkowych plikow):
//! - NVIDIA (RTX 20/30/40/50 i starsze): NVML (nvml.dll) - temperatura, obciazenie, VRAM, zegary,
//!   moc, wentylator; NVAPI (`nvapi.rs`) - hotspot, temperatura zlacza pamieci i obroty wentylatora,
//! - AMD Radeon: ADL (`gpu_amd.rs`, atiadlxx.dll),
//! - Intel Arc i zintegrowane: IGCL (`gpu_intel.rs`, ControlLib.dll).
//!
//! Wybor: karta dedykowana ma pierwszenstwo przed zintegrowana; przy kilku dedykowanych wygrywa
//! pierwsza w kolejnosci NVIDIA, AMD, Intel. Kolejne biblioteki ladujemy tylko, gdy wczesniejsze
//! nie znalazly karty dedykowanej. Nie wymaga uprawnien administratora.

use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};

use super::gpu_amd::AmdGpu;
use super::gpu_intel::IntelGpu;
use super::nvapi::{NvApiGpu, ThermalLayout, thermal_layout};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vendor {
    Nvidia,
    Amd,
    Intel,
}

/// Karta NVIDIA: NVML + opcjonalnie NVAPI.
struct NvidiaGpu {
    nvml: Nvml,
    nvapi: Option<NvApiGpu>,
    layout: ThermalLayout,
    nvapi_issue: Option<String>,
    name: String,
}

impl NvidiaGpu {
    fn new() -> Result<Self, String> {
        let nvml = Nvml::init().map_err(|err| format!("NVML niedostepne: {err}"))?;
        let (name, arch, bus_id) = {
            let device = nvml.device_by_index(0).map_err(|err| format!("Brak karty NVIDIA: {err}"))?;
            (
                device.name().unwrap_or_else(|_| "GPU".to_owned()),
                device.architecture().ok(),
                device.pci_info().ok().map(|pci| pci.bus),
            )
        };
        log::info!("NVML gotowe: {name} (architektura {arch:?})");
        let layout = thermal_layout(&name, arch);
        let (nvapi, nvapi_issue) = match NvApiGpu::new(layout, bus_id) {
            Ok(nvapi) => (Some(nvapi), None),
            Err(err) => {
                log::warn!("NVAPI niedostepne (hotspot, temperatura pamieci, RPM): {err}");
                (None, Some(err))
            }
        };
        Ok(Self { nvml, nvapi, layout, nvapi_issue, name })
    }

    fn read(&self) -> GpuReading {
        let Ok(device) = self.nvml.device_by_index(0) else {
            return GpuReading::default();
        };
        let memory = device.memory_info().ok();
        const MB: f32 = 1024.0 * 1024.0;
        let (hotspot_c, memory_temp_c) = self.nvapi.as_ref().map(|n| n.read_thermals()).unwrap_or_default();
        // RPM: NVAPI (jak LHM), a gdy brak - NVML (nvmlDeviceGetFanSpeedRPM, nowsze sterowniki).
        let fan_rpm = self
            .nvapi
            .as_ref()
            .and_then(|n| n.read_fan_rpm())
            .or_else(|| device.fan_speed_rpm(0).ok().map(|rpm| rpm as f32));
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
            fan_rpm,
            vram_used_mb: memory.as_ref().map(|m| m.used as f32 / MB),
            vram_total_mb: memory.as_ref().map(|m| m.total as f32 / MB),
        }
    }
}

enum Backend {
    Nvidia(Box<NvidiaGpu>),
    Amd(AmdGpu),
    Intel(IntelGpu),
}

impl Backend {
    fn vendor(&self) -> Vendor {
        match self {
            Self::Nvidia(_) => Vendor::Nvidia,
            Self::Amd(_) => Vendor::Amd,
            Self::Intel(_) => Vendor::Intel,
        }
    }

    /// Karty NVIDIA obslugiwane przez NVML sa zawsze dedykowane.
    fn is_discrete(&self) -> bool {
        match self {
            Self::Nvidia(_) => true,
            Self::Amd(gpu) => gpu.is_discrete,
            Self::Intel(gpu) => gpu.is_discrete,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Nvidia(gpu) => &gpu.name,
            Self::Amd(gpu) => &gpu.name,
            Self::Intel(gpu) => &gpu.name,
        }
    }

    fn read(&self) -> GpuReading {
        match self {
            Self::Nvidia(gpu) => gpu.read(),
            Self::Amd(gpu) => gpu.read(),
            Self::Intel(gpu) => gpu.read(),
        }
    }

    /// Wyjasnienia braku hotspotu / temperatury pamieci dla okna "Czujniki" (blad, hotspot, pamiec).
    fn thermal_hints(&self) -> (Option<String>, &'static str, &'static str) {
        match self {
            Self::Nvidia(gpu) => (
                gpu.nvapi_issue.clone(),
                if gpu.layout == ThermalLayout::Blackwell {
                    "karty RTX 50 go nie udostepniaja"
                } else {
                    "sterownik go nie podaje"
                },
                "sterownik podaje ja zwykle tylko dla pamieci GDDR6X/GDDR7",
            ),
            Self::Amd(gpu) => (
                None,
                "sterownik AMD go nie podaje dla tej karty",
                if gpu.is_discrete {
                    "sterownik AMD jej nie podaje dla tej karty"
                } else {
                    "karta zintegrowana nie ma wlasnej pamieci"
                },
            ),
            Self::Intel(gpu) => (
                None,
                "sterownik Intel nie podaje osobnego hotspotu - temperatura GPU to najgoretszy czujnik",
                if gpu.is_discrete {
                    "sterownik Intel jej nie podaje dla tej karty"
                } else {
                    "karta zintegrowana nie ma wlasnej pamieci"
                },
            ),
        }
    }
}

/// Wybor karty: pierwsza dedykowana, a gdy brak - pierwsza zintegrowana (kolejnosc wejscia).
fn pick_backend(found: &[(Vendor, bool)]) -> Option<usize> {
    found.iter().position(|(_, discrete)| *discrete).or_else(|| (!found.is_empty()).then_some(0))
}

/// Komunikat dla okna "Czujniki": blad biblioteki albo lista brakujacych odczytow z wyjasnieniem.
fn thermals_issue(
    backend_issue: Option<String>,
    probe: &GpuReading,
    hotspot_hint: &str,
    memory_hint: &str,
) -> Option<String> {
    if backend_issue.is_some() {
        return backend_issue;
    }
    let missing: Vec<String> = [
        probe.hotspot_c.is_none().then(|| format!("brak hotspotu ({hotspot_hint})")),
        probe.memory_temp_c.is_none().then(|| format!("brak temperatury pamieci ({memory_hint})")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if missing.is_empty() {
        return None;
    }
    let mut text = missing.join("; ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    Some(text)
}

pub struct GpuSensor {
    backend: Backend,
    pub name: String,
    /// Problem z hotspotem / temperatura pamieci do pokazania w oknie konfiguracji
    /// (pozostale odczyty dzialaja dalej).
    pub thermals_issue: Option<String>,
}

impl GpuSensor {
    pub fn new() -> Result<Self, String> {
        let mut found: Vec<Backend> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        type Probe = fn() -> Result<Backend, String>;
        let probes: [(&str, Probe); 3] = [
            ("NVIDIA", || NvidiaGpu::new().map(|gpu| Backend::Nvidia(Box::new(gpu)))),
            ("AMD", || AmdGpu::new().map(Backend::Amd)),
            ("Intel", || IntelGpu::new().map(Backend::Intel)),
        ];
        for (vendor, probe) in probes {
            match probe() {
                Ok(backend) => {
                    log::info!(
                        "GPU {vendor}: {} ({})",
                        backend.name(),
                        if backend.is_discrete() { "dedykowana" } else { "zintegrowana" }
                    );
                    let discrete = backend.is_discrete();
                    found.push(backend);
                    if discrete {
                        break;
                    }
                }
                Err(err) => {
                    log::debug!("GPU {vendor} niedostepne: {err}");
                    errors.push(format!("{vendor}: {err}"));
                }
            }
        }

        let flags: Vec<(Vendor, bool)> = found.iter().map(|b| (b.vendor(), b.is_discrete())).collect();
        let Some(index) = pick_backend(&flags) else {
            return Err(format!("Nie znaleziono obslugiwanej karty graficznej ({})", errors.join("; ")));
        };
        let backend = found.swap_remove(index);
        // Pozostale backendy zwalniaja biblioteki w Drop.
        drop(found);

        let probe = backend.read();
        let (issue, hotspot_hint, memory_hint) = backend.thermal_hints();
        let thermals_issue = thermals_issue(issue, &probe, hotspot_hint, memory_hint);
        let name = backend.name().to_owned();
        log::info!("Wybrana karta graficzna: {name} ({:?})", backend.vendor());
        if let Some(issue) = &thermals_issue {
            log::warn!("Hotspot/temperatura pamieci GPU: {issue}");
        }
        Ok(Self { backend, name, thermals_issue })
    }

    pub fn read(&self) -> GpuReading {
        self.backend.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discrete_card_wins_over_integrated() {
        // Kolejnosc wejscia = NVIDIA, AMD, Intel.
        assert_eq!(pick_backend(&[(Vendor::Amd, false), (Vendor::Intel, true)]), Some(1));
        assert_eq!(pick_backend(&[(Vendor::Nvidia, true), (Vendor::Amd, true)]), Some(0));
        assert_eq!(pick_backend(&[(Vendor::Amd, true), (Vendor::Intel, true)]), Some(0));
        assert_eq!(pick_backend(&[(Vendor::Amd, false), (Vendor::Intel, false)]), Some(0));
        assert_eq!(pick_backend(&[]), None);
    }

    #[test]
    fn thermals_issue_lists_missing_values() {
        let full = GpuReading { hotspot_c: Some(70.0), memory_temp_c: Some(60.0), ..Default::default() };
        assert_eq!(thermals_issue(None, &full, "h", "m"), None);

        let no_memory = GpuReading { hotspot_c: Some(70.0), ..Default::default() };
        assert_eq!(thermals_issue(None, &no_memory, "h", "tylko GDDR6X").as_deref(), Some("Brak temperatury pamieci (tylko GDDR6X)"));

        let none = GpuReading::default();
        assert_eq!(
            thermals_issue(None, &none, "x", "y").as_deref(),
            Some("Brak hotspotu (x); brak temperatury pamieci (y)")
        );
        // Blad biblioteki ma pierwszenstwo przed lista brakow.
        assert_eq!(thermals_issue(Some("Brak nvapi64.dll".into()), &none, "x", "y").as_deref(), Some("Brak nvapi64.dll"));
    }

    /// Test na prawdziwym sprzecie: `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn reads_real_gpu() {
        let sensor = GpuSensor::new().expect("brak obslugiwanej karty");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let reading = sensor.read();
        eprintln!("{} / {:?} / {:?}", sensor.name, sensor.thermals_issue, reading);
        assert!(reading.temp_c.is_some());
        assert!(reading.load_pct.is_some());
        assert!(reading.vram_total_mb.is_some());
    }
}
