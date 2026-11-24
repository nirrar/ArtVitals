//! Hotspot, temperatura zlacza pamieci i obroty wentylatora GPU NVIDIA przez NVAPI
//! (nvapi64.dll ze sterownika).
//!
//! Tych odczytow nie ma w publicznym NVML. Uzywamy nieudokumentowanych funkcji dokladnie tak jak
//! LibreHardwareMonitor (MPL-2.0; Interop/NvApi.cs, Hardware/Gpu/NvidiaGpu.cs):
//! - `NvAPI_GPU_GetThermalSensors` (0x65FE3AAD): maske sensorow wyznaczamy probujac kolejnych
//!   bitow, temperatury sa w 1/256 C, a indeksy zaleza od generacji karty (`ThermalLayout`),
//! - `NvAPI_GPU_ClientFanCoolersGetStatus` (0x35AED5E8) - obroty wentylatorow (RTX 20 i nowsze),
//!   z rezerwa `NvAPI_GPU_GetTachReading` (0x5F608315) dla starszych kart,
//! - `NvAPI_GPU_GetBusId` (0x1BE0B8E5) - dopasowanie uchwytu NVAPI do karty wybranej w NVML.
//!
//! Zaleznosci: crate `windows` (LoadLibraryW/GetProcAddress), `nvml-wrapper` (tylko typ architektury).
//! Nie wymaga uprawnien administratora.

use std::ffi::c_void;

use nvml_wrapper::enums::device::DeviceArchitecture;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

const ID_INITIALIZE: u32 = 0x0150_E828;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
const ID_GET_THERMAL_SENSORS: u32 = 0x65FE_3AAD;
const ID_GET_BUS_ID: u32 = 0x1BE0_B8E5;
const ID_FAN_COOLERS_GET_STATUS: u32 = 0x35AE_D5E8;
const ID_GET_TACH_READING: u32 = 0x5F60_8315;
const MAX_PHYSICAL_GPUS: usize = 64;
const MAX_FAN_COOLERS_STATUS_ITEMS: usize = 32;
const NVAPI_OK: i32 = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct NvThermalSensors {
    version: u32,
    mask: u32,
    reserved: [i32; 8],
    temperatures: [i32; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NvFanCoolersStatusItem {
    cooler_id: u32,
    current_rpm: u32,
    current_min_level: u32,
    current_max_level: u32,
    current_level: u32,
    reserved: [u32; 8],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NvFanCoolersStatus {
    version: u32,
    count: u32,
    reserved: [u64; 4],
    items: [NvFanCoolersStatusItem; MAX_FAN_COOLERS_STATUS_ITEMS],
}

type QueryInterfaceFn = unsafe extern "C" fn(u32) -> *mut c_void;
type InitializeFn = unsafe extern "C" fn() -> i32;
type EnumPhysicalGpusFn = unsafe extern "C" fn(*mut *mut c_void, *mut i32) -> i32;
type GetThermalSensorsFn = unsafe extern "C" fn(*mut c_void, *mut NvThermalSensors) -> i32;
type GetBusIdFn = unsafe extern "C" fn(*mut c_void, *mut u32) -> i32;
type FanCoolersGetStatusFn = unsafe extern "C" fn(*mut c_void, *mut NvFanCoolersStatus) -> i32;
type GetTachReadingFn = unsafe extern "C" fn(*mut c_void, *mut i32) -> i32;

/// MAKE_NVAPI_VERSION: rozmiar struktury | (wersja << 16).
const fn nvapi_version<T>(version: u32) -> u32 {
    size_of::<T>() as u32 | (version << 16)
}

/// Ktore indeksy `NvAPI_GPU_GetThermalSensors` oznaczaja hotspot i pamiec (jak w LHM NvidiaGpu.cs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalLayout {
    /// Turing (RTX 20), Ampere (RTX 30) i starsze: hotspot = 1, pamiec = 9.
    Default,
    /// Ada (RTX 40): hotspot = 1, pamiec = 7.
    Ada,
    /// Blackwell (RTX 50): brak hotspotu, pamiec = 2.
    Blackwell,
}

impl ThermalLayout {
    fn hotspot_index(self) -> Option<usize> {
        match self {
            Self::Default | Self::Ada => Some(1),
            Self::Blackwell => None,
        }
    }

    fn memory_index(self) -> usize {
        match self {
            Self::Default => 9,
            Self::Ada => 7,
            Self::Blackwell => 2,
        }
    }
}

/// Uklad sensorow dla karty. Najpierw reguly nazw z LHM (prefiks "NVIDIA GeForce RTX 50/40",
/// "Blackwell" w nazwie), potem architektura z NVML - obejmuje karty bez GeForce w nazwie
/// (np. "NVIDIA RTX 4000 Ada Generation"), ktorych LHM nie rozpoznaje. Zwykle `contains("RTX 50")`
/// bylo bledne: pasowalo tez do "RTX 5000 Ada".
pub fn thermal_layout(name: &str, arch: Option<DeviceArchitecture>) -> ThermalLayout {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("NVIDIA GEFORCE RTX 50") || upper.contains("BLACKWELL") {
        return ThermalLayout::Blackwell;
    }
    if upper.starts_with("NVIDIA GEFORCE RTX 40") {
        return ThermalLayout::Ada;
    }
    match arch {
        Some(DeviceArchitecture::Blackwell) => ThermalLayout::Blackwell,
        Some(DeviceArchitecture::Ada) => ThermalLayout::Ada,
        _ => ThermalLayout::Default,
    }
}

pub struct NvApiGpu {
    _library: HMODULE,
    get_thermal_sensors: Option<GetThermalSensorsFn>,
    fan_coolers_get_status: Option<FanCoolersGetStatusFn>,
    get_tach_reading: Option<GetTachReadingFn>,
    gpu: *mut c_void,
    /// 0 = karta nie udostepnia sensorow termicznych NVAPI.
    mask: u32,
    layout: ThermalLayout,
}

// Uchwyt GPU i wskazniki funkcji sa uzywane wylacznie z watku czujnikow.
unsafe impl Send for NvApiGpu {}

impl NvApiGpu {
    /// `bus_id` z NVML (PCI bus) - wybiera ten sam fizyczny GPU, co NVML; bez niego pierwszy uchwyt.
    pub fn new(layout: ThermalLayout, bus_id: Option<u32>) -> Result<Self, String> {
        unsafe {
            let library = LoadLibraryW(w!("nvapi64.dll")).map_err(|err| format!("Brak nvapi64.dll: {err}"))?;
            let query = GetProcAddress(library, PCSTR(c"nvapi_QueryInterface".as_ptr().cast()))
                .ok_or("Brak nvapi_QueryInterface")?;
            let query: QueryInterfaceFn = std::mem::transmute(query);
            let resolve = |id: u32| {
                let ptr = query(id);
                (!ptr.is_null()).then_some(ptr)
            };
            let required = |id: u32| resolve(id).ok_or_else(|| format!("NVAPI nie zna funkcji 0x{id:08X}"));
            let initialize: InitializeFn = std::mem::transmute(required(ID_INITIALIZE)?);
            let enum_gpus: EnumPhysicalGpusFn = std::mem::transmute(required(ID_ENUM_PHYSICAL_GPUS)?);
            // Pozostale funkcje sa opcjonalne - brak jednej nie wylacza pozostalych odczytow.
            let get_thermal_sensors =
                resolve(ID_GET_THERMAL_SENSORS).map(|p| std::mem::transmute::<*mut c_void, GetThermalSensorsFn>(p));
            let get_bus_id = resolve(ID_GET_BUS_ID).map(|p| std::mem::transmute::<*mut c_void, GetBusIdFn>(p));
            let fan_coolers_get_status = resolve(ID_FAN_COOLERS_GET_STATUS)
                .map(|p| std::mem::transmute::<*mut c_void, FanCoolersGetStatusFn>(p));
            let get_tach_reading =
                resolve(ID_GET_TACH_READING).map(|p| std::mem::transmute::<*mut c_void, GetTachReadingFn>(p));

            if initialize() != NVAPI_OK {
                return Err("NvAPI_Initialize nieudane".to_owned());
            }
            let mut handles = [std::ptr::null_mut::<c_void>(); MAX_PHYSICAL_GPUS];
            let mut count = 0i32;
            if enum_gpus(handles.as_mut_ptr(), &mut count) != NVAPI_OK || count < 1 {
                return Err("NVAPI nie widzi karty NVIDIA".to_owned());
            }
            let handles = &handles[..(count as usize).min(MAX_PHYSICAL_GPUS)];

            // Indeksy NVAPI i NVML moga wskazywac rozne karty - dopasowanie po numerze szyny PCI (jak LHM).
            let gpu = match (bus_id, get_bus_id) {
                (Some(wanted), Some(get_bus_id)) => handles
                    .iter()
                    .copied()
                    .find(|&handle| {
                        let mut bus = u32::MAX;
                        get_bus_id(handle, &mut bus) == NVAPI_OK && bus == wanted
                    })
                    .unwrap_or(handles[0]),
                _ => handles[0],
            };

            let mut sensor = Self {
                _library: library,
                get_thermal_sensors,
                fan_coolers_get_status,
                get_tach_reading,
                gpu,
                mask: 0,
                layout,
            };
            sensor.mask = sensor.probe_mask();
            log::info!("NVAPI gotowe (maska sensorow 0x{:X}, uklad {:?})", sensor.mask, layout);
            Ok(sensor)
        }
    }

    fn query(&self, mask: u32) -> Option<NvThermalSensors> {
        let get_thermal_sensors = self.get_thermal_sensors?;
        let mut data = NvThermalSensors {
            version: nvapi_version::<NvThermalSensors>(2),
            mask,
            reserved: [0; 8],
            temperatures: [0; 32],
        };
        let status = unsafe { get_thermal_sensors(self.gpu, &mut data) };
        (status == NVAPI_OK).then_some(data)
    }

    /// Najwieksza dzialajaca maska: kolejne bity az do pierwszego bledu (jak w LHM).
    fn probe_mask(&self) -> u32 {
        for bit in 0..32 {
            if self.query(1u32 << bit).is_none() {
                return (1u32 << bit).wrapping_sub(1);
            }
        }
        u32::MAX
    }

    /// (hotspot, temperatura pamieci) w C; 0 oznacza brak sensora (LHM aktywuje tylko != 0).
    pub fn read_thermals(&self) -> (Option<f32>, Option<f32>) {
        if self.mask == 0 {
            return (None, None);
        }
        let Some(data) = self.query(self.mask) else {
            return (None, None);
        };
        let value = |index: usize| {
            let raw = *data.temperatures.get(index)?;
            (raw > 0).then(|| raw as f32 / 256.0)
        };
        (self.layout.hotspot_index().and_then(value), value(self.layout.memory_index()))
    }

    /// Obroty pierwszego wentylatora: ClientFanCoolersGetStatus, a gdy niedostepne - GetTachReading
    /// (kolejnosc jak w LHM). 0 RPM jest poprawnym odczytem (tryb pasywny).
    pub fn read_fan_rpm(&self) -> Option<f32> {
        if let Some(get_status) = self.fan_coolers_get_status {
            // Struktura ~1.7 KB - zerowana jak w LHM (tablica Items alokowana pusta).
            let mut status: NvFanCoolersStatus = unsafe { std::mem::zeroed() };
            status.version = nvapi_version::<NvFanCoolersStatus>(1);
            let result = unsafe { get_status(self.gpu, &mut status) };
            if result == NVAPI_OK && status.count > 0 {
                return Some(status.items[0].current_rpm as f32);
            }
        }
        let get_tach = self.get_tach_reading?;
        let mut rpm = 0i32;
        let result = unsafe { get_tach(self.gpu, &mut rpm) };
        (result == NVAPI_OK && rpm >= 0).then_some(rpm as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_struct_has_layout_expected_by_nvapi() {
        // 2 x u32 + 8 x i32 + 32 x i32 = 168 bajtow - tak jak w strukturze z LHM.
        assert_eq!(size_of::<NvThermalSensors>(), 168);
        assert_eq!(nvapi_version::<NvThermalSensors>(2), 168 | (2 << 16));
    }

    #[test]
    fn fan_coolers_struct_has_layout_expected_by_nvapi() {
        // Element: 5 x u32 + 8 x u32 = 52 B; calosc: 8 + 4 x u64 + 32 x 52 = 1704 B (Pack=8 w LHM).
        assert_eq!(size_of::<NvFanCoolersStatusItem>(), 52);
        assert_eq!(size_of::<NvFanCoolersStatus>(), 1704);
        assert_eq!(std::mem::offset_of!(NvFanCoolersStatus, items), 40);
    }

    #[test]
    fn layout_follows_lhm_name_rules() {
        assert_eq!(thermal_layout("NVIDIA GeForce RTX 4070 Ti", None), ThermalLayout::Ada);
        assert_eq!(thermal_layout("NVIDIA GeForce RTX 4060 Laptop GPU", None), ThermalLayout::Ada);
        assert_eq!(thermal_layout("NVIDIA GeForce RTX 5090", None), ThermalLayout::Blackwell);
        assert_eq!(thermal_layout("NVIDIA RTX PRO 6000 Blackwell Workstation Edition", None), ThermalLayout::Blackwell);
        assert_eq!(thermal_layout("NVIDIA GeForce RTX 3080", None), ThermalLayout::Default);
        assert_eq!(thermal_layout("NVIDIA GeForce RTX 2070 SUPER", None), ThermalLayout::Default);
    }

    #[test]
    fn layout_uses_architecture_when_name_is_not_geforce() {
        // "RTX 5000 Ada" zawiera "RTX 50" - nie moze trafic do ukladu Blackwell.
        let ada = Some(DeviceArchitecture::Ada);
        assert_eq!(thermal_layout("NVIDIA RTX 5000 Ada Generation", ada), ThermalLayout::Ada);
        assert_eq!(thermal_layout("NVIDIA RTX 5000 Ada Generation", None), ThermalLayout::Default);
        assert_eq!(thermal_layout("NVIDIA RTX A4000", Some(DeviceArchitecture::Ampere)), ThermalLayout::Default);
        assert_eq!(thermal_layout("NVIDIA GeForce RTX 3090", Some(DeviceArchitecture::Ampere)), ThermalLayout::Default);
        assert_eq!(thermal_layout("Unknown", Some(DeviceArchitecture::Blackwell)), ThermalLayout::Blackwell);
    }
}
