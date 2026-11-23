//! Hotspot i temperatura zlacza pamieci GPU NVIDIA przez NVAPI (nvapi64.dll ze sterownika).
//!
//! Tych odczytow nie ma w publicznym NVML. Uzywamy nieudokumentowanej funkcji
//! `NvAPI_GPU_GetThermalSensors` (id 0x65FE3AAD) dokladnie tak jak LibreHardwareMonitor
//! (Interop/NvApi.cs, Hardware/Gpu/NvidiaGpu.cs): maske sensorow wyznaczamy probujac
//! kolejnych bitow, temperatury sa w 1/256 C, a indeksy zaleza od generacji karty.
//! Obslugiwana jest pierwsza karta (jak w NVML). Nie wymaga uprawnien administratora.

use std::ffi::c_void;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

const ID_INITIALIZE: u32 = 0x0150_E828;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
const ID_GET_THERMAL_SENSORS: u32 = 0x65FE_3AAD;
const MAX_PHYSICAL_GPUS: usize = 64;
const NVAPI_OK: i32 = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct NvThermalSensors {
    version: u32,
    mask: u32,
    reserved: [i32; 8],
    temperatures: [i32; 32],
}

type QueryInterfaceFn = unsafe extern "C" fn(u32) -> *mut c_void;
type InitializeFn = unsafe extern "C" fn() -> i32;
type EnumPhysicalGpusFn = unsafe extern "C" fn(*mut *mut c_void, *mut i32) -> i32;
type GetThermalSensorsFn = unsafe extern "C" fn(*mut c_void, *mut NvThermalSensors) -> i32;

pub struct NvApiThermals {
    _library: HMODULE,
    get_thermal_sensors: GetThermalSensorsFn,
    gpu: *mut c_void,
    mask: u32,
    hotspot_index: Option<usize>,
    memory_index: usize,
}

// Uchwyt GPU i wskazniki funkcji sa uzywane wylacznie z watku czujnikow.
unsafe impl Send for NvApiThermals {}

impl NvApiThermals {
    /// `gpu_name` z NVML - wybiera indeksy sensorow wlasciwe dla generacji karty.
    pub fn new(gpu_name: &str) -> Result<Self, String> {
        unsafe {
            let library = LoadLibraryW(w!("nvapi64.dll")).map_err(|err| format!("Brak nvapi64.dll: {err}"))?;
            let query = GetProcAddress(library, PCSTR(c"nvapi_QueryInterface".as_ptr().cast()))
                .ok_or("Brak nvapi_QueryInterface")?;
            let query: QueryInterfaceFn = std::mem::transmute(query);
            let resolve = |id: u32| {
                let ptr = query(id);
                if ptr.is_null() { Err(format!("NVAPI nie zna funkcji 0x{id:08X}")) } else { Ok(ptr) }
            };
            let initialize: InitializeFn = std::mem::transmute(resolve(ID_INITIALIZE)?);
            let enum_gpus: EnumPhysicalGpusFn = std::mem::transmute(resolve(ID_ENUM_PHYSICAL_GPUS)?);
            let get_thermal_sensors: GetThermalSensorsFn = std::mem::transmute(resolve(ID_GET_THERMAL_SENSORS)?);

            if initialize() != NVAPI_OK {
                return Err("NvAPI_Initialize nieudane".to_owned());
            }
            let mut handles = [std::ptr::null_mut::<c_void>(); MAX_PHYSICAL_GPUS];
            let mut count = 0i32;
            if enum_gpus(handles.as_mut_ptr(), &mut count) != NVAPI_OK || count < 1 {
                return Err("NVAPI nie widzi karty NVIDIA".to_owned());
            }

            let mut sensor = Self {
                _library: library,
                get_thermal_sensors,
                gpu: handles[0],
                mask: 0,
                hotspot_index: Some(1),
                memory_index: 9,
            };
            sensor.mask = sensor.probe_mask();
            if sensor.mask == 0 {
                return Err("Karta nie udostepnia sensorow termicznych NVAPI".to_owned());
            }
            // Indeksy jak w LibreHardwareMonitor (NvidiaGpu.cs).
            let name = gpu_name.to_ascii_uppercase();
            if name.contains("RTX 50") || name.contains("BLACKWELL") {
                sensor.hotspot_index = None;
                sensor.memory_index = 2;
            } else if name.contains("RTX 40") {
                sensor.memory_index = 7;
            }
            log::info!("NVAPI gotowe (maska sensorow 0x{:X})", sensor.mask);
            Ok(sensor)
        }
    }

    fn query(&self, mask: u32) -> Option<NvThermalSensors> {
        let mut data = NvThermalSensors {
            // MAKE_NVAPI_VERSION: rozmiar struktury | (wersja << 16).
            version: size_of::<NvThermalSensors>() as u32 | (2 << 16),
            mask,
            reserved: [0; 8],
            temperatures: [0; 32],
        };
        let status = unsafe { (self.get_thermal_sensors)(self.gpu, &mut data) };
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

    /// (hotspot, temperatura pamieci) w C; 0 oznacza brak odczytu.
    pub fn read(&self) -> (Option<f32>, Option<f32>) {
        let Some(data) = self.query(self.mask) else {
            return (None, None);
        };
        let value = |index: usize| {
            let raw = *data.temperatures.get(index)?;
            (raw > 0).then(|| raw as f32 / 256.0)
        };
        (self.hotspot_index.and_then(value), value(self.memory_index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_struct_has_layout_expected_by_nvapi() {
        // 2 x u32 + 8 x i32 + 32 x i32 = 168 bajtow - tak jak w strukturze z LHM.
        assert_eq!(size_of::<NvThermalSensors>(), 168);
    }
}
