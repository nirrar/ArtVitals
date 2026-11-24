//! Odczyty GPU Intel (Arc i zintegrowane) przez Intel Graphics Control Library
//! (`ControlLib.dll` instalowany ze sterownikiem).
//!
//! Uklady struktur i stale wg publicznego naglowka igcl_api.h (github.com/intel/drivers.gpu.control-library,
//! Intel Software License Agreement - tylko do uzytku na platformach Intel, tekst w
//! `assets/IGCL.LICENSE.txt`); sposob uzycia wg LibreHardwareMonitor (MPL-2.0): Interop/IntelGcl.cs,
//! Hardware/Gpu/IntelDiscreteGpu.cs, Hardware/Gpu/IntelGpuGroup.cs.
//! - `ctlPowerTelemetryGet`: temperatura GPU i VRAM, zegary, moc (roznica licznikow energii / czas),
//!   obciazenie (roznica licznika aktywnosci / czas), wentylator,
//! - `ctlEnumFans`/`ctlFanGetState`: obroty i % wentylatora (jak LHM),
//! - `ctlEnumMemoryModules`/`ctlMemoryGetState`: VRAM (tylko pamiec lokalna karty).
//!
//! Moc i obciazenie wymagaja dwoch probek - pierwszy odczyt ich nie ma.
//! Biblioteka ladowana dynamicznie (LoadLibraryW/GetProcAddress). Zaleznosci: crate `windows`.

use std::cell::RefCell;
use std::ffi::c_void;
use std::time::Instant;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

use super::gpu::GpuReading;
use crate::i18n::tr;

const CTL_RESULT_SUCCESS: i32 = 0;
/// CTL_MAKE_VERSION(CTL_IMPL_MAJOR_VERSION = 1, CTL_IMPL_MINOR_VERSION = 1).
const CTL_IMPL_VERSION: u32 = (1 << 16) | 1;
const CTL_INIT_FLAG_USE_LEVEL_ZERO: u32 = 1;
const CTL_DEVICE_TYPE_GRAPHICS: u32 = 1;
const CTL_ADAPTER_PROPERTIES_FLAG_INTEGRATED: u32 = 1;
const CTL_MAX_DEVICE_NAME_LEN: usize = 100;
/// W aktualnym naglowku 108 (LHM ma jeszcze 112 sprzed dodania `num_xe_cores`) - rozmiar calosci ten sam.
const CTL_MAX_RESERVED_SIZE: usize = 108;
const CTL_PSU_COUNT: usize = 5;
const CTL_FAN_COUNT: usize = 5;
const CTL_MEM_LOC_DEVICE: u32 = 1;
const CTL_FAN_SPEED_UNITS_RPM: u32 = 0;
const CTL_FAN_SPEED_UNITS_PERCENT: u32 = 1;
const CTL_UNITS_ANGULAR_SPEED_RPM: u32 = 9;
const CTL_UNITS_PERCENT: u32 = 11;
const INTEL_VENDOR_ID: u32 = 0x8086;
const MAX_DEVICES: u32 = 64;
const MAX_COMPONENTS: u32 = 16;

// ctl_data_type_t
const CTL_DATA_TYPE_INT8: u32 = 0;
const CTL_DATA_TYPE_UINT8: u32 = 1;
const CTL_DATA_TYPE_INT16: u32 = 2;
const CTL_DATA_TYPE_UINT16: u32 = 3;
const CTL_DATA_TYPE_INT32: u32 = 4;
const CTL_DATA_TYPE_UINT32: u32 = 5;
const CTL_DATA_TYPE_INT64: u32 = 6;
const CTL_DATA_TYPE_UINT64: u32 = 7;
const CTL_DATA_TYPE_FLOAT: u32 = 8;
const CTL_DATA_TYPE_DOUBLE: u32 = 9;

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlApplicationId {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlInitArgs {
    size: u32,
    version: u8,
    app_version: u32,
    flags: u32,
    supported_version: u32,
    application_uid: CtlApplicationId,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlFirmwareVersion {
    major_version: u64,
    minor_version: u64,
    build_number: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlAdapterBdf {
    bus: u8,
    device: u8,
    function: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CtlDeviceAdapterProperties {
    size: u32,
    version: u8,
    device_id: *mut c_void,
    device_id_size: u32,
    device_type: u32,
    supported_subfunction_flags: u32,
    driver_version: u64,
    firmware_version: CtlFirmwareVersion,
    pci_vendor_id: u32,
    pci_device_id: u32,
    rev_id: u32,
    num_eus_per_sub_slice: u32,
    num_sub_slices_per_slice: u32,
    num_slices: u32,
    name: [u8; CTL_MAX_DEVICE_NAME_LEN],
    graphics_adapter_properties: u32,
    frequency: u32,
    pci_subsys_id: u16,
    pci_subsys_vendor_id: u16,
    adapter_bdf: CtlAdapterBdf,
    num_xe_cores: u32,
    reserved: [u8; CTL_MAX_RESERVED_SIZE],
}

/// ctl_data_value_t - unia 8 B.
#[repr(C)]
#[derive(Clone, Copy)]
union CtlDataValue {
    data8: i8,
    datau8: u8,
    data16: i16,
    datau16: u16,
    data32: i32,
    datau32: u32,
    data64: i64,
    datau64: u64,
    datafloat: f32,
    datadouble: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CtlTelemetryItem {
    /// C `bool` (1 B w MSVC) - jako u8, bo sterownik moze wpisac dowolny bajt.
    supported: u8,
    units: u32,
    data_type: u32,
    value: CtlDataValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CtlPsuInfo {
    supported: u8,
    psu_type: u32,
    energy_counter: CtlTelemetryItem,
    voltage: CtlTelemetryItem,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CtlPowerTelemetry {
    size: u32,
    version: u8,
    time_stamp: CtlTelemetryItem,
    gpu_energy_counter: CtlTelemetryItem,
    gpu_voltage: CtlTelemetryItem,
    gpu_current_clock_frequency: CtlTelemetryItem,
    gpu_current_temperature: CtlTelemetryItem,
    global_activity_counter: CtlTelemetryItem,
    render_compute_activity_counter: CtlTelemetryItem,
    media_activity_counter: CtlTelemetryItem,
    gpu_power_limited: u8,
    gpu_temperature_limited: u8,
    gpu_current_limited: u8,
    gpu_voltage_limited: u8,
    gpu_utilization_limited: u8,
    vram_energy_counter: CtlTelemetryItem,
    vram_voltage: CtlTelemetryItem,
    vram_current_clock_frequency: CtlTelemetryItem,
    vram_current_effective_frequency: CtlTelemetryItem,
    vram_read_bandwidth_counter: CtlTelemetryItem,
    vram_write_bandwidth_counter: CtlTelemetryItem,
    vram_current_temperature: CtlTelemetryItem,
    vram_power_limited: u8,
    vram_temperature_limited: u8,
    vram_current_limited: u8,
    vram_voltage_limited: u8,
    vram_utilization_limited: u8,
    total_card_energy_counter: CtlTelemetryItem,
    psu: [CtlPsuInfo; CTL_PSU_COUNT],
    fan_speed: [CtlTelemetryItem; CTL_FAN_COUNT],
    // Ponizsze pola tylko dla Version > 0.
    gpu_vr_temp: CtlTelemetryItem,
    vram_vr_temp: CtlTelemetryItem,
    sa_vr_temp: CtlTelemetryItem,
    gpu_effective_clock: CtlTelemetryItem,
    gpu_over_voltage_percent: CtlTelemetryItem,
    gpu_power_percent: CtlTelemetryItem,
    gpu_temperature_percent: CtlTelemetryItem,
    vram_read_bandwidth: CtlTelemetryItem,
    vram_write_bandwidth: CtlTelemetryItem,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlMemProperties {
    size: u32,
    version: u8,
    mem_type: u32,
    location: u32,
    physical_size: u64,
    bus_width: i32,
    num_channels: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlMemState {
    size: u32,
    version: u8,
    free: u64,
    size_bytes: u64,
}

type InitFn = unsafe extern "C" fn(*mut CtlInitArgs, *mut Handle) -> i32;
type CloseFn = unsafe extern "C" fn(Handle) -> i32;
type EnumerateDevicesFn = unsafe extern "C" fn(Handle, *mut u32, *mut Handle) -> i32;
type GetDevicePropertiesFn = unsafe extern "C" fn(Handle, *mut CtlDeviceAdapterProperties) -> i32;
type PowerTelemetryGetFn = unsafe extern "C" fn(Handle, *mut CtlPowerTelemetry) -> i32;
type EnumComponentsFn = unsafe extern "C" fn(Handle, *mut u32, *mut Handle) -> i32;
type MemoryGetPropertiesFn = unsafe extern "C" fn(Handle, *mut CtlMemProperties) -> i32;
type MemoryGetStateFn = unsafe extern "C" fn(Handle, *mut CtlMemState) -> i32;
type FanGetStateFn = unsafe extern "C" fn(Handle, u32, *mut i32) -> i32;

/// Wartosc pozycji telemetrii jako f64 (typ wg `data_type`, jak GetTelemetryValue w LHM).
fn telemetry_value(item: &CtlTelemetryItem) -> Option<f64> {
    if item.supported == 0 {
        return None;
    }
    let v = item.value;
    // SAFETY: odczyt pola unii wskazanego przez `data_type`; kazde pole to zwykle bajty.
    let value = unsafe {
        match item.data_type {
            CTL_DATA_TYPE_INT8 => f64::from(v.data8),
            CTL_DATA_TYPE_UINT8 => f64::from(v.datau8),
            CTL_DATA_TYPE_INT16 => f64::from(v.data16),
            CTL_DATA_TYPE_UINT16 => f64::from(v.datau16),
            CTL_DATA_TYPE_INT32 => f64::from(v.data32),
            CTL_DATA_TYPE_UINT32 => f64::from(v.datau32),
            CTL_DATA_TYPE_INT64 => v.data64 as f64,
            CTL_DATA_TYPE_UINT64 => v.datau64 as f64,
            CTL_DATA_TYPE_FLOAT => f64::from(v.datafloat),
            CTL_DATA_TYPE_DOUBLE => v.datadouble,
            _ => return None,
        }
    };
    value.is_finite().then_some(value)
}

/// Srednia z roznicy licznika: moc (J / s) albo obciazenie (s aktywnosci / s * 100).
/// Ujemna roznica (reset licznika) albo brak czasu - brak wyniku.
fn counter_rate(previous: Option<f64>, current: Option<f64>, elapsed_s: f64) -> Option<f64> {
    let delta = current? - previous?;
    (elapsed_s > 0.0 && delta >= 0.0).then(|| delta / elapsed_s)
}

/// Pierwsze urzadzenie dedykowane, a gdy brak - pierwsze zintegrowane (indeks w `devices`).
fn pick_device(devices: &[(bool, bool)]) -> Option<usize> {
    // (czy karta graficzna Intel, czy zintegrowana)
    let usable = || devices.iter().enumerate().filter(|(_, (ok, _))| *ok);
    usable().find(|(_, (_, integrated))| !integrated).or_else(|| usable().next()).map(|(i, _)| i)
}

#[derive(Default)]
struct Sample {
    timestamp_s: Option<f64>,
    taken: Option<Instant>,
    energy_j: Option<f64>,
    activity_s: Option<f64>,
}

pub struct IntelGpu {
    _library: HMODULE,
    api: Handle,
    close: Option<CloseFn>,
    device: Handle,
    telemetry_get: Option<PowerTelemetryGetFn>,
    fan_get_state: Option<FanGetStateFn>,
    memory_get_state: Option<MemoryGetStateFn>,
    fans: Vec<Handle>,
    /// Moduly pamieci lokalnej karty (CTL_MEM_LOC_DEVICE).
    memory_modules: Vec<Handle>,
    pub name: String,
    pub is_discrete: bool,
    previous: RefCell<Sample>,
}

// Uchwyty IGCL sa uzywane wylacznie z watku czujnikow.
unsafe impl Send for IntelGpu {}

unsafe fn symbol<T>(library: HMODULE, name: &std::ffi::CStr) -> Option<T> {
    unsafe {
        let ptr = GetProcAddress(library, PCSTR(name.as_ptr().cast()))?;
        Some(std::mem::transmute_copy::<_, T>(&ptr))
    }
}

/// Wzorzec "zapytaj o liczbe, potem pobierz uchwyty" (ctlEnumFans, ctlEnumMemoryModules).
unsafe fn enumerate(f: EnumComponentsFn, parent: Handle, limit: u32) -> Vec<Handle> {
    unsafe {
        let mut count = 0u32;
        if f(parent, &mut count, std::ptr::null_mut()) != CTL_RESULT_SUCCESS || count == 0 {
            return Vec::new();
        }
        count = count.min(limit);
        let mut handles = vec![std::ptr::null_mut(); count as usize];
        if f(parent, &mut count, handles.as_mut_ptr()) != CTL_RESULT_SUCCESS {
            return Vec::new();
        }
        handles.truncate(count as usize);
        handles.retain(|h| !h.is_null());
        handles
    }
}

impl IntelGpu {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let library = LoadLibraryW(w!("ControlLib.dll")).map_err(|_| {
                tr(
                    "Brak ControlLib.dll (sterownik Intel Graphics nie jest zainstalowany)",
                    "ControlLib.dll not found (Intel Graphics driver not installed)",
                )
                .to_owned()
            })?;
            let missing = |function: &str| format!("IGCL: {} {function}", tr("brak", "missing"));
            let init: InitFn = symbol(library, c"ctlInit").ok_or_else(|| missing("ctlInit"))?;
            let enumerate_devices: EnumerateDevicesFn =
                symbol(library, c"ctlEnumerateDevices").ok_or_else(|| missing("ctlEnumerateDevices"))?;
            let get_properties: GetDevicePropertiesFn =
                symbol(library, c"ctlGetDeviceProperties").ok_or_else(|| missing("ctlGetDeviceProperties"))?;
            let close: Option<CloseFn> = symbol(library, c"ctlClose");

            // Telemetria wymaga Level Zero (CTL_INIT_FLAG_USE_LEVEL_ZERO, jak LHM i przyklady Intela).
            let mut args = CtlInitArgs {
                size: size_of::<CtlInitArgs>() as u32,
                version: 0,
                app_version: CTL_IMPL_VERSION,
                flags: CTL_INIT_FLAG_USE_LEVEL_ZERO,
                ..Default::default()
            };
            let mut api: Handle = std::ptr::null_mut();
            let status = init(&mut args, &mut api);
            if status != CTL_RESULT_SUCCESS || api.is_null() {
                return Err(format!("ctlInit {} 0x{status:08X})", tr("nieudane (kod", "failed (code")));
            }
            let mut gpu = Self {
                _library: library,
                api,
                close,
                device: std::ptr::null_mut(),
                telemetry_get: symbol(library, c"ctlPowerTelemetryGet"),
                fan_get_state: symbol(library, c"ctlFanGetState"),
                memory_get_state: symbol(library, c"ctlMemoryGetState"),
                fans: Vec::new(),
                memory_modules: Vec::new(),
                name: String::new(),
                is_discrete: false,
                previous: RefCell::new(Sample::default()),
            };

            let devices = enumerate(enumerate_devices, api, MAX_DEVICES);
            if devices.is_empty() {
                return Err(tr("IGCL nie widzi karty Intel", "IGCL sees no Intel card").to_owned());
            }
            let properties: Vec<Option<CtlDeviceAdapterProperties>> = devices
                .iter()
                .map(|&device| {
                    // pDeviceID - bufor na LUID (8 B) jak w przykladach Intela.
                    let mut luid = [0u8; 8];
                    let mut props: CtlDeviceAdapterProperties = std::mem::zeroed();
                    props.size = size_of::<CtlDeviceAdapterProperties>() as u32;
                    props.version = 2;
                    props.device_id = luid.as_mut_ptr().cast();
                    props.device_id_size = luid.len() as u32;
                    let ok = get_properties(device, &mut props) == CTL_RESULT_SUCCESS;
                    props.device_id = std::ptr::null_mut();
                    ok.then_some(props)
                })
                .collect();
            let flags: Vec<(bool, bool)> = properties
                .iter()
                .map(|p| match p {
                    Some(p) => (
                        p.device_type == CTL_DEVICE_TYPE_GRAPHICS && p.pci_vendor_id == INTEL_VENDOR_ID,
                        p.graphics_adapter_properties & CTL_ADAPTER_PROPERTIES_FLAG_INTEGRATED != 0,
                    ),
                    None => (false, false),
                })
                .collect();
            let chosen = pick_device(&flags)
                .ok_or(tr("IGCL nie znalazl karty graficznej Intel", "IGCL found no Intel graphics card"))?;
            let props = properties[chosen]
                .as_ref()
                .ok_or(tr("IGCL: brak wlasciwosci karty", "IGCL: card properties unavailable"))?;
            gpu.device = devices[chosen];
            gpu.is_discrete = !flags[chosen].1;
            gpu.name = {
                let end = props.name.iter().position(|&b| b == 0).unwrap_or(props.name.len());
                let name = String::from_utf8_lossy(&props.name[..end]).trim().to_owned();
                if name.is_empty() { "Intel Graphics".to_owned() } else { name }
            };

            if let Some(enum_fans) = symbol::<EnumComponentsFn>(library, c"ctlEnumFans") {
                gpu.fans = enumerate(enum_fans, gpu.device, MAX_COMPONENTS);
            }
            if let (Some(enum_memory), Some(get_props)) = (
                symbol::<EnumComponentsFn>(library, c"ctlEnumMemoryModules"),
                symbol::<MemoryGetPropertiesFn>(library, c"ctlMemoryGetProperties"),
            ) {
                gpu.memory_modules = enumerate(enum_memory, gpu.device, MAX_COMPONENTS)
                    .into_iter()
                    .filter(|&module| {
                        let mut p = CtlMemProperties { size: size_of::<CtlMemProperties>() as u32, ..Default::default() };
                        get_props(module, &mut p) == CTL_RESULT_SUCCESS && p.location == CTL_MEM_LOC_DEVICE
                    })
                    .collect();
            }
            log::info!(
                "IGCL gotowe: {} ({}, wentylatory: {}, moduly VRAM: {})",
                gpu.name,
                if gpu.is_discrete { "dedykowana" } else { "zintegrowana" },
                gpu.fans.len(),
                gpu.memory_modules.len()
            );
            Ok(gpu)
        }
    }

    fn telemetry(&self) -> Option<CtlPowerTelemetry> {
        let get = self.telemetry_get?;
        let mut t: CtlPowerTelemetry = unsafe { std::mem::zeroed() };
        t.size = size_of::<CtlPowerTelemetry>() as u32;
        t.version = 1;
        (unsafe { get(self.device, &mut t) } == CTL_RESULT_SUCCESS).then_some(t)
    }

    fn fan_state(&self, units: u32) -> Option<f32> {
        let get = self.fan_get_state?;
        let fan = *self.fans.first()?;
        let mut speed = -1i32;
        // -1 = sterownik nie potrafi zmierzyc.
        (unsafe { get(fan, units, &mut speed) } == CTL_RESULT_SUCCESS && speed >= 0).then_some(speed as f32)
    }

    fn vram_mb(&self) -> (Option<f32>, Option<f32>) {
        let Some(get) = self.memory_get_state else {
            return (None, None);
        };
        let (mut total, mut free, mut any) = (0u64, 0u64, false);
        for &module in &self.memory_modules {
            let mut s = CtlMemState { size: size_of::<CtlMemState>() as u32, ..Default::default() };
            if unsafe { get(module, &mut s) } == CTL_RESULT_SUCCESS && s.size_bytes > 0 {
                total += s.size_bytes;
                free += s.free.min(s.size_bytes);
                any = true;
            }
        }
        if !any {
            return (None, None);
        }
        const MB: f32 = 1024.0 * 1024.0;
        (Some((total - free) as f32 / MB), Some(total as f32 / MB))
    }

    pub fn read(&self) -> GpuReading {
        let mut r = GpuReading::default();
        if let Some(t) = self.telemetry() {
            let as_f32 = |item: &CtlTelemetryItem| telemetry_value(item).map(|v| v as f32);
            r.temp_c = as_f32(&t.gpu_current_temperature);
            r.memory_temp_c = as_f32(&t.vram_current_temperature).filter(|&v| v > 0.0);
            r.clock_mhz = as_f32(&t.gpu_current_clock_frequency);
            r.memory_clock_mhz = as_f32(&t.vram_current_clock_frequency).filter(|&v| v > 0.0);
            // Wentylator z telemetrii - jednostka podana w pozycji (RPM albo %).
            let fan = &t.fan_speed[0];
            match fan.units {
                CTL_UNITS_ANGULAR_SPEED_RPM => r.fan_rpm = as_f32(fan).filter(|&v| v >= 0.0),
                CTL_UNITS_PERCENT => r.fan_pct = as_f32(fan).filter(|&v| v >= 0.0),
                _ => {}
            }

            // Moc calej karty, a gdy niedostepna - samego GPU (oba liczniki w J).
            let energy = telemetry_value(&t.total_card_energy_counter)
                .or_else(|| telemetry_value(&t.gpu_energy_counter));
            let activity = telemetry_value(&t.global_activity_counter);
            let now = Instant::now();
            let timestamp = telemetry_value(&t.time_stamp);
            let mut prev = self.previous.borrow_mut();
            // Czas z licznika sterownika (s od 1970); bez niego - zegar monotoniczny programu.
            let elapsed = match (prev.timestamp_s, timestamp) {
                (Some(a), Some(b)) => b - a,
                _ => prev.taken.map_or(0.0, |t| now.duration_since(t).as_secs_f64()),
            };
            r.power_w = counter_rate(prev.energy_j, energy, elapsed).map(|w| w as f32);
            r.load_pct = counter_rate(prev.activity_s, activity, elapsed).map(|a| (a * 100.0).clamp(0.0, 100.0) as f32);
            *prev = Sample { timestamp_s: timestamp, taken: Some(now), energy_j: energy, activity_s: activity };
        }
        if let Some(rpm) = self.fan_state(CTL_FAN_SPEED_UNITS_RPM) {
            r.fan_rpm = Some(rpm);
        }
        if let Some(pct) = self.fan_state(CTL_FAN_SPEED_UNITS_PERCENT) {
            r.fan_pct = Some(pct.min(100.0));
        }
        (r.vram_used_mb, r.vram_total_mb) = self.vram_mb();
        r
    }
}

impl Drop for IntelGpu {
    fn drop(&mut self) {
        if let Some(close) = self.close {
            unsafe { close(self.api) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structs_match_igcl_header_layout() {
        assert_eq!(size_of::<CtlInitArgs>(), 36);
        assert_eq!(size_of::<CtlDeviceAdapterProperties>(), 320);
        assert_eq!(std::mem::offset_of!(CtlDeviceAdapterProperties, driver_version), 32);
        assert_eq!(std::mem::offset_of!(CtlDeviceAdapterProperties, name), 88);
        assert_eq!(std::mem::offset_of!(CtlDeviceAdapterProperties, graphics_adapter_properties), 188);
        assert_eq!(std::mem::offset_of!(CtlDeviceAdapterProperties, num_xe_cores), 204);
        assert_eq!(size_of::<CtlDataValue>(), 8);
        assert_eq!(size_of::<CtlTelemetryItem>(), 24);
        assert_eq!(std::mem::offset_of!(CtlTelemetryItem, value), 16);
        assert_eq!(size_of::<CtlPsuInfo>(), 56);
        assert_eq!(std::mem::offset_of!(CtlPowerTelemetry, time_stamp), 8);
        assert_eq!(std::mem::offset_of!(CtlPowerTelemetry, vram_energy_counter), 208);
        assert_eq!(std::mem::offset_of!(CtlPowerTelemetry, total_card_energy_counter), 384);
        assert_eq!(std::mem::offset_of!(CtlPowerTelemetry, fan_speed), 688);
        assert_eq!(size_of::<CtlPowerTelemetry>(), 1024);
        assert_eq!(size_of::<CtlMemProperties>(), 32);
        assert_eq!(size_of::<CtlMemState>(), 24);
    }

    fn item(data_type: u32, value: CtlDataValue) -> CtlTelemetryItem {
        CtlTelemetryItem { supported: 1, units: 0, data_type, value }
    }

    #[test]
    fn telemetry_values_are_decoded_by_type() {
        assert_eq!(telemetry_value(&item(CTL_DATA_TYPE_DOUBLE, CtlDataValue { datadouble: 61.5 })), Some(61.5));
        assert_eq!(telemetry_value(&item(CTL_DATA_TYPE_FLOAT, CtlDataValue { datafloat: 2.5 })), Some(2.5));
        assert_eq!(telemetry_value(&item(CTL_DATA_TYPE_UINT32, CtlDataValue { datau64: 2400 })), Some(2400.0));
        assert_eq!(telemetry_value(&item(CTL_DATA_TYPE_INT64, CtlDataValue { data64: -5 })), Some(-5.0));
        assert_eq!(telemetry_value(&item(0x4800_FFFF, CtlDataValue { datau64: 1 })), None);
        let mut unsupported = item(CTL_DATA_TYPE_DOUBLE, CtlDataValue { datadouble: 1.0 });
        unsupported.supported = 0;
        assert_eq!(telemetry_value(&unsupported), None);
    }

    #[test]
    fn counters_give_power_and_load() {
        // 150 J w 1.5 s = 100 W; 0.75 s aktywnosci w 1.5 s = 50 %.
        assert_eq!(counter_rate(Some(1000.0), Some(1150.0), 1.5), Some(100.0));
        assert_eq!(counter_rate(Some(10.0), Some(10.75), 1.5), Some(0.5));
        assert_eq!(counter_rate(None, Some(1.0), 1.0), None);
        assert_eq!(counter_rate(Some(5.0), Some(1.0), 1.0), None);
        assert_eq!(counter_rate(Some(1.0), Some(2.0), 0.0), None);
    }

    #[test]
    fn prefers_discrete_arc_over_igpu() {
        assert_eq!(pick_device(&[(true, true), (true, false)]), Some(1));
        assert_eq!(pick_device(&[(false, false), (true, true)]), Some(1));
        assert_eq!(pick_device(&[(false, false)]), None);
        assert_eq!(pick_device(&[]), None);
    }

    /// Na prawdziwym sprzecie: `cargo test -- --ignored --nocapture`; bez sterownika ma zwrocic blad, nie panike.
    #[test]
    #[ignore]
    fn init_on_real_hardware() {
        match IntelGpu::new() {
            Ok(gpu) => eprintln!("{}: {:?}", gpu.name, gpu.read()),
            Err(err) => eprintln!("niedostepne: {err}"),
        }
    }
}
