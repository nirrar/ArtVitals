//! Odczyty GPU AMD Radeon przez ADL (AMD Display Library, `atiadlxx.dll` instalowany ze sterownikiem).
//!
//! Port podejscia LibreHardwareMonitor (MPL-2.0): Hardware/Gpu/AmdGpu.cs, Hardware/Gpu/AmdGpuGroup.cs,
//! Interop/AtiAdlxx.cs. Uklady struktur sprawdzone z naglowkami ADL SDK (GPUOpen display-library, MIT,
//! include/adl_structures.h, adl_defines.h) - m.in. `ADLPMLogStartOutput` ma 64 B (w LHM brakuje
//! pola `ulReserved[14]`).
//!
//! Zrodla odczytow (kolejnosc jak w LHM, nowsze nadpisuja starsze):
//! - Overdrive 5: temperatura, wentylator (RPM i %), zegary i obciazenie,
//! - Overdrive 6: pobor mocy, Overdrive N (7): temperatury krawedzi, pamieci i hotspotu,
//! - Overdrive 8 i nowsze (Vega/RDNA): log PM (`ADL2_Adapter_PMLog_Start` - pamiec aktualizowana
//!   przez sterownik) albo `ADL2_New_QueryPMLogData_Get`,
//! - VRAM: `ADL2_Adapter_MemoryInfoX4_Get` (calkowita) i `ADL2_Adapter_DedicatedVRAMUsage_Get` (zajeta).
//!
//! Biblioteka ladowana dynamicznie (LoadLibraryW/GetProcAddress), brak funkcji = brak odczytu.
//! Zaleznosci: crate `windows`. Nie wymaga uprawnien administratora.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ffi::c_void;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

use super::gpu::GpuReading;

const ADL_OK: i32 = 0;
const ADL_TRUE: i32 = 1;
const ADL_MAX_PATH: usize = 256;
const ADL_PMLOG_MAX_SENSORS: usize = 256;
const ATI_VENDOR_ID: u32 = 0x1002;
const FAN_SPEED_TYPE_PERCENT: i32 = 1;
const FAN_SPEED_TYPE_RPM: i32 = 2;
/// Okres probkowania logu PM (ms) - jak w LHM.
const PMLOG_SAMPLE_RATE_MS: u32 = 1000;

// Identyfikatory sensorow ADL_PMLOG_* (adl_defines.h).
const PMLOG_CLK_GFXCLK: u16 = 1;
const PMLOG_CLK_MEMCLK: u16 = 2;
const PMLOG_TEMPERATURE_EDGE: u16 = 8;
const PMLOG_TEMPERATURE_MEM: u16 = 9;
const PMLOG_FAN_RPM: u16 = 14;
const PMLOG_FAN_PERCENTAGE: u16 = 15;
const PMLOG_INFO_ACTIVITY_GFX: u16 = 19;
const PMLOG_ASIC_POWER: u16 = 23;
const PMLOG_TEMPERATURE_HOTSPOT: u16 = 27;
const PMLOG_TEMPERATURE_GFX: u16 = 28;
const PMLOG_TEMPERATURE_HOTSPOT_GCD: u16 = 50;
const PMLOG_BOARD_POWER: u16 = 73;

// Typy temperatur ADL2_OverdriveN_Temperature_Get (odpowiadaja ADL_PMLOG_TEMPERATURE_*).
const ODN_TEMP_EDGE: i32 = 1;
const ODN_TEMP_MEM: i32 = 2;
const ODN_TEMP_HOTSPOT: i32 = 7;
const OD6_POWER_TOTAL: i32 = 0;

// Rodziny ASIC (ADLGcnInfo::ASICFamilyId, jak GCNFamilies w LHM / amdgpu_drm.h).
const FAMILY_AI: i32 = 141;
const FAMILY_NV3: i32 = 145;

#[repr(C)]
#[derive(Clone, Copy)]
struct AdlAdapterInfo {
    size: i32,
    adapter_index: i32,
    udid: [u8; ADL_MAX_PATH],
    bus_number: i32,
    device_number: i32,
    function_number: i32,
    vendor_id: i32,
    adapter_name: [u8; ADL_MAX_PATH],
    display_name: [u8; ADL_MAX_PATH],
    present: i32,
    exist: i32,
    driver_path: [u8; ADL_MAX_PATH],
    driver_path_ext: [u8; ADL_MAX_PATH],
    pnp_string: [u8; ADL_MAX_PATH],
    os_display_index: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlPmActivity {
    size: i32,
    engine_clock: i32,
    memory_clock: i32,
    vddc: i32,
    activity_percent: i32,
    current_performance_level: i32,
    current_bus_speed: i32,
    current_bus_lanes: i32,
    maximum_bus_lanes: i32,
    reserved: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlTemperature {
    size: i32,
    temperature: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlFanSpeedValue {
    size: i32,
    speed_type: i32,
    fan_speed: i32,
    flags: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlParameterRange {
    min: i32,
    max: i32,
    step: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlOdParameters {
    size: i32,
    number_of_performance_levels: i32,
    activity_reporting_supported: i32,
    discrete_performance_levels: i32,
    reserved: i32,
    engine_clock: AdlParameterRange,
    memory_clock: AdlParameterRange,
    vddc: AdlParameterRange,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlOd6Capabilities {
    capabilities: i32,
    supported_states: i32,
    number_of_performance_levels: i32,
    engine_clock_range: AdlParameterRange,
    memory_clock_range: AdlParameterRange,
    ext_value: i32,
    ext_mask: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlSingleSensorData {
    supported: i32,
    value: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdlPmLogDataOutput {
    size: i32,
    sensors: [AdlSingleSensorData; ADL_PMLOG_MAX_SENSORS],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdlGcnInfo {
    cu_count: i32,
    tex_count: i32,
    rop_count: i32,
    asic_family_id: i32,
    asic_revision_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdlPmLogSupportInfo {
    sensors: [u16; ADL_PMLOG_MAX_SENSORS],
    reserved: [i32; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdlPmLogStartInput {
    sensors: [u16; ADL_PMLOG_MAX_SENSORS],
    /// `unsigned long` - 32 bity na Windows.
    sample_rate: u32,
    reserved: [i32; 15],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdlPmLogStartOutput {
    logging_address: *mut c_void,
    reserved: [i32; 14],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdlPmLogData {
    version: u32,
    active_sample_rate: u32,
    /// FILETIME (100 ns od 1601-01-01 UTC) ostatniej aktualizacji.
    last_updated: u64,
    /// Pary (id sensora, wartosc), zakonczone id = 0.
    values: [[u32; 2]; ADL_PMLOG_MAX_SENSORS],
    reserved: [u32; 256],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdlMemoryInfoX4 {
    memory_size: i64,
    memory_type: [u8; ADL_MAX_PATH],
    memory_bandwidth: i64,
    hyper_memory_size: i64,
    invisible_memory_size: i64,
    visible_memory_size: i64,
    vram_vendor_rev_id: i64,
    memory_bandwidth_x2: i64,
    memory_bit_rate_x2: i64,
}

type Context = *mut c_void;
type MallocCallback = unsafe extern "system" fn(i32) -> *mut c_void;
type MainControlCreateFn = unsafe extern "C" fn(MallocCallback, i32, *mut Context) -> i32;
type MainControlDestroyFn = unsafe extern "C" fn(Context) -> i32;
type NumberOfAdaptersFn = unsafe extern "C" fn(Context, *mut i32) -> i32;
type AdapterInfoGetFn = unsafe extern "C" fn(Context, *mut AdlAdapterInfo, i32) -> i32;
type AdapterActiveGetFn = unsafe extern "C" fn(Context, i32, *mut i32) -> i32;
type GcnAsicInfoGetFn = unsafe extern "C" fn(Context, i32, *mut AdlGcnInfo) -> i32;
type OverdriveCapsFn = unsafe extern "C" fn(Context, i32, *mut i32, *mut i32, *mut i32) -> i32;
type Od6CapabilitiesGetFn = unsafe extern "C" fn(Context, i32, *mut AdlOd6Capabilities) -> i32;
type Od5ParametersGetFn = unsafe extern "C" fn(Context, i32, *mut AdlOdParameters) -> i32;
type Od5TemperatureGetFn = unsafe extern "C" fn(Context, i32, i32, *mut AdlTemperature) -> i32;
type Od5FanSpeedGetFn = unsafe extern "C" fn(Context, i32, i32, *mut AdlFanSpeedValue) -> i32;
type Od5CurrentActivityGetFn = unsafe extern "C" fn(Context, i32, *mut AdlPmActivity) -> i32;
type Od6CurrentPowerGetFn = unsafe extern "C" fn(Context, i32, i32, *mut i32) -> i32;
type OdnTemperatureGetFn = unsafe extern "C" fn(Context, i32, i32, *mut i32) -> i32;
type QueryPmLogDataGetFn = unsafe extern "C" fn(Context, i32, *mut AdlPmLogDataOutput) -> i32;
type PmLogDeviceCreateFn = unsafe extern "C" fn(Context, i32, *mut u32) -> i32;
type PmLogDeviceDestroyFn = unsafe extern "C" fn(Context, u32) -> i32;
type PmLogSupportGetFn = unsafe extern "C" fn(Context, i32, *mut AdlPmLogSupportInfo) -> i32;
type PmLogStartFn =
    unsafe extern "C" fn(Context, i32, *mut AdlPmLogStartInput, *mut AdlPmLogStartOutput, u32) -> i32;
type PmLogStopFn = unsafe extern "C" fn(Context, i32, u32) -> i32;
type MemoryInfoX4GetFn = unsafe extern "C" fn(Context, i32, *mut AdlMemoryInfoX4) -> i32;
type DedicatedVramUsageGetFn = unsafe extern "C" fn(Context, i32, *mut i32) -> i32;

/// Naglowek przed bloku zwracanym ADL - przechowuje rozmiar potrzebny do zwolnienia.
const ALLOC_HEADER: usize = 16;

/// Alokator przekazywany do ADL2_Main_Control_Create (ADL_MAIN_MALLOC_CALLBACK, __stdcall).
/// Zadna z uzywanych tu funkcji nie zwraca pamieci przez ten callback, ale ADL wymaga go zawsze;
/// pamiec z niego zwalnia `adl_free`.
unsafe extern "system" fn adl_alloc(size: i32) -> *mut c_void {
    let Ok(size) = usize::try_from(size) else {
        return std::ptr::null_mut();
    };
    let Ok(layout) = Layout::from_size_align(size + ALLOC_HEADER, ALLOC_HEADER) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let base = alloc_zeroed(layout);
        if base.is_null() {
            return std::ptr::null_mut();
        }
        base.cast::<usize>().write(size);
        base.add(ALLOC_HEADER).cast()
    }
}

/// Zwalnia blok przydzielony przez `adl_alloc`.
#[cfg_attr(not(test), allow(dead_code))]
unsafe fn adl_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let base = ptr.cast::<u8>().sub(ALLOC_HEADER);
        let size = base.cast::<usize>().read();
        dealloc(base, Layout::from_size_align_unchecked(size + ALLOC_HEADER, ALLOC_HEADER));
    }
}

/// Tekst z tablicy `char[]` zakonczonej zerem.
fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

/// Vendor ID z UDID w formacie Windows ("PCI_VEN_1002&DEV_..."); pole iVendorID z ADL jest
/// na Windows bledne (komentarz w LHM AtiAdlxx.cs).
fn vendor_from_udid(udid: &str) -> Option<u32> {
    let start = udid.to_ascii_uppercase().find("PCI_VEN_")? + "PCI_VEN_".len();
    let hex: String = udid[start..].chars().take_while(|c| c.is_ascii_hexdigit()).take(4).collect();
    u32::from_str_radix(&hex, 16).ok()
}

/// Rodziny APU (zintegrowane): Trinity, Kaveri, Carrizo, Raven, Van Gogh, Rembrandt, Phoenix,
/// Raphael, GC 11.5 (Strix), Mendocino - numery jak w LHM GCNFamilies.
fn is_apu_family(family: i32) -> bool {
    matches!(family, 105 | 125 | 135 | 142 | 144 | 146 | 148 | 149 | 150 | 151)
}

/// Kandydat z enumeracji ADL (jeden fizyczny GPU ma zwykle kilka indeksow - po jednym na wyjscie).
#[derive(Debug, Clone)]
struct Candidate {
    adapter_index: i32,
    bus: i32,
    device: i32,
    vendor: Option<u32>,
    has_udid: bool,
    active: bool,
    family: i32,
    pmlog_sensors: usize,
}

/// Wybor adaptera: tylko AMD z UDID, duplikaty (ta sama szyna/urzadzenie) zwijane do wpisu
/// z logiem PM, potem aktywnego (jak grupowanie w LHM AmdGpuGroup.cs); karta dedykowana ma
/// pierwszenstwo przed APU. Zwraca indeks w `candidates`.
fn pick_adapter(candidates: &[Candidate]) -> Option<usize> {
    let mut groups: Vec<usize> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        if !c.has_udid || c.vendor != Some(ATI_VENDOR_ID) {
            continue;
        }
        let score = |c: &Candidate| (c.pmlog_sensors > 0, c.active);
        match groups.iter_mut().find(|g| candidates[**g].bus == c.bus && candidates[**g].device == c.device) {
            Some(best) if score(c) > score(&candidates[*best]) => *best = i,
            Some(_) => {}
            None => groups.push(i),
        }
    }
    groups
        .iter()
        .copied()
        .find(|&i| !is_apu_family(candidates[i].family))
        .or_else(|| groups.first().copied())
}

/// Czy dane logu PM sa swieze: znacznik czasu w +-48 h od teraz i rozsadny okres probkowania
/// (ta sama kontrola co w LHM AmdGpu.GetAdlSensor).
fn pmlog_data_valid(last_updated: u64, sample_rate: u32, now_filetime: u64) -> bool {
    const HOURS_48: u64 = 48 * 3600 * 10_000_000;
    last_updated != 0
        && sample_rate <= 86_400_000
        && last_updated <= now_filetime.saturating_add(HOURS_48)
        && last_updated >= now_filetime.saturating_sub(HOURS_48)
}

fn now_filetime() -> u64 {
    // FILETIME liczy od 1601-01-01; roznica do epoki Unix to 11 644 473 600 s.
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (unix.as_secs() + 11_644_473_600) * 10_000_000 + u64::from(unix.subsec_nanos() / 100)
}

/// Wartosc sensora z par (id, wartosc) logu PM; lista konczy sie id = 0.
fn pmlog_value(values: &[[u32; 2]], sensor: u16) -> Option<u32> {
    values
        .iter()
        .take_while(|pair| pair[0] != 0)
        .find(|pair| pair[0] == u32::from(sensor))
        .map(|pair| pair[1])
}

/// Temperatura OverdriveN w C. Bez sensora czesc kart zwraca np. 54000 C - zakres +-256 C jak w LHM.
fn odn_temperature(raw: i32, scale: f64) -> Option<f32> {
    let max = (256.0 / scale) as i32;
    let min = (-256.0 / scale) as i32;
    (min..=max).contains(&raw).then(|| (f64::from(raw) * scale) as f32)
}

struct Functions {
    main_control_destroy: Option<MainControlDestroyFn>,
    od5_temperature: Option<Od5TemperatureGetFn>,
    od5_fan_speed: Option<Od5FanSpeedGetFn>,
    od5_activity: Option<Od5CurrentActivityGetFn>,
    od6_power: Option<Od6CurrentPowerGetFn>,
    odn_temperature: Option<OdnTemperatureGetFn>,
    query_pmlog: Option<QueryPmLogDataGetFn>,
    pmlog_stop: Option<PmLogStopFn>,
    pmlog_device_destroy: Option<PmLogDeviceDestroyFn>,
    dedicated_vram_usage: Option<DedicatedVramUsageGetFn>,
}

pub struct AmdGpu {
    _library: HMODULE,
    context: Context,
    functions: Functions,
    adapter_index: i32,
    family: i32,
    pub name: String,
    pub is_discrete: bool,
    overdrive_supported: bool,
    overdrive_level: i32,
    /// Uchwyt urzadzenia logu PM (0 = brak) i lista sensorow, ktore sterownik loguje.
    pmlog_device: u32,
    pmlog_address: *const AdlPmLogData,
    pmlog_supported: Vec<u16>,
    vram_total_mb: Option<f32>,
}

// Kontekst ADL i wskazniki sa uzywane wylacznie z watku czujnikow.
unsafe impl Send for AmdGpu {}

unsafe fn symbol<T>(library: HMODULE, name: &std::ffi::CStr) -> Option<T> {
    unsafe {
        let ptr = GetProcAddress(library, PCSTR(name.as_ptr().cast()))?;
        Some(std::mem::transmute_copy::<_, T>(&ptr))
    }
}

impl AmdGpu {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let library = LoadLibraryW(w!("atiadlxx.dll"))
                .map_err(|_| "Brak atiadlxx.dll (sterownik AMD nie jest zainstalowany)".to_owned())?;
            let create: MainControlCreateFn =
                symbol(library, c"ADL2_Main_Control_Create").ok_or("ADL: brak ADL2_Main_Control_Create")?;
            let number_of_adapters: NumberOfAdaptersFn = symbol(library, c"ADL2_Adapter_NumberOfAdapters_Get")
                .ok_or("ADL: brak ADL2_Adapter_NumberOfAdapters_Get")?;
            let adapter_info_get: AdapterInfoGetFn =
                symbol(library, c"ADL2_Adapter_AdapterInfo_Get").ok_or("ADL: brak ADL2_Adapter_AdapterInfo_Get")?;
            let functions = Functions {
                main_control_destroy: symbol(library, c"ADL2_Main_Control_Destroy"),
                od5_temperature: symbol(library, c"ADL2_Overdrive5_Temperature_Get"),
                od5_fan_speed: symbol(library, c"ADL2_Overdrive5_FanSpeed_Get"),
                od5_activity: symbol(library, c"ADL2_Overdrive5_CurrentActivity_Get"),
                od6_power: symbol(library, c"ADL2_Overdrive6_CurrentPower_Get"),
                odn_temperature: symbol(library, c"ADL2_OverdriveN_Temperature_Get"),
                query_pmlog: symbol(library, c"ADL2_New_QueryPMLogData_Get"),
                pmlog_stop: symbol(library, c"ADL2_Adapter_PMLog_Stop"),
                pmlog_device_destroy: symbol(library, c"ADL2_Device_PMLog_Device_Destroy"),
                dedicated_vram_usage: symbol(library, c"ADL2_Adapter_DedicatedVRAMUsage_Get"),
            };

            let mut context: Context = std::ptr::null_mut();
            // 1 = tylko adaptery podlaczone/aktywne (jak LHM).
            let status = create(adl_alloc, 1, &mut context);
            if status != ADL_OK || context.is_null() {
                return Err(format!("ADL2_Main_Control_Create nieudane (kod {status})"));
            }
            // Od tej chwili kontekst zwalnia Drop - takze przy bledzie ponizej.
            let mut gpu = Self {
                _library: library,
                context,
                functions,
                adapter_index: -1,
                family: 0,
                name: String::new(),
                is_discrete: false,
                overdrive_supported: false,
                overdrive_level: -1,
                pmlog_device: 0,
                pmlog_address: std::ptr::null(),
                pmlog_supported: Vec::new(),
                vram_total_mb: None,
            };

            let mut count = 0i32;
            if number_of_adapters(context, &mut count) != ADL_OK || count <= 0 {
                return Err("ADL nie widzi zadnej karty AMD".to_owned());
            }
            let count = count.min(256) as usize;
            let mut infos: Vec<AdlAdapterInfo> = vec![std::mem::zeroed(); count];
            let bytes = (count * size_of::<AdlAdapterInfo>()) as i32;
            if adapter_info_get(context, infos.as_mut_ptr(), bytes) != ADL_OK {
                return Err("ADL2_Adapter_AdapterInfo_Get nieudane".to_owned());
            }

            let active_get: Option<AdapterActiveGetFn> = symbol(library, c"ADL2_Adapter_Active_Get");
            let gcn_info_get: Option<GcnAsicInfoGetFn> = symbol(library, c"ADL2_GcnAsicInfo_Get");
            let pmlog_create: Option<PmLogDeviceCreateFn> = symbol(library, c"ADL2_Device_PMLog_Device_Create");
            let pmlog_support: Option<PmLogSupportGetFn> = symbol(library, c"ADL2_Adapter_PMLog_Support_Get");

            let candidates: Vec<Candidate> = infos
                .iter()
                .map(|info| {
                    let index = info.adapter_index;
                    let udid = c_string(&info.udid);
                    let mut active = 0i32;
                    let active = active_get.is_some_and(|f| f(context, index, &mut active) == ADL_OK && active != 0);
                    let mut gcn = AdlGcnInfo::default();
                    if let Some(f) = gcn_info_get {
                        f(context, index, &mut gcn);
                    }
                    let pmlog_sensors = if gcn.asic_family_id >= FAMILY_AI {
                        gpu.pmlog_support_list(index, pmlog_create, pmlog_support).map_or(0, |(device, list)| {
                            gpu.destroy_pmlog_device(device);
                            list.len()
                        })
                    } else {
                        0
                    };
                    Candidate {
                        adapter_index: index,
                        bus: info.bus_number,
                        device: info.device_number,
                        vendor: vendor_from_udid(&udid),
                        has_udid: !udid.is_empty(),
                        active,
                        family: gcn.asic_family_id,
                        pmlog_sensors,
                    }
                })
                .collect();
            let chosen = pick_adapter(&candidates).ok_or("ADL nie znalazl karty AMD")?;
            let candidate = &candidates[chosen];
            gpu.adapter_index = candidate.adapter_index;
            gpu.family = candidate.family;
            gpu.is_discrete = !is_apu_family(candidate.family);
            gpu.name = c_string(&infos[chosen].adapter_name);
            if gpu.name.is_empty() {
                gpu.name = "AMD Radeon".to_owned();
            }

            gpu.detect_overdrive(library);
            if candidate.family >= FAMILY_AI {
                gpu.start_pmlog(library, pmlog_create, pmlog_support);
            }
            if let Some(f) = symbol::<MemoryInfoX4GetFn>(library, c"ADL2_Adapter_MemoryInfoX4_Get") {
                let mut info: AdlMemoryInfoX4 = std::mem::zeroed();
                if f(context, gpu.adapter_index, &mut info) == ADL_OK && info.memory_size > 0 {
                    gpu.vram_total_mb = Some(info.memory_size as f32 / (1024.0 * 1024.0));
                }
            }
            log::info!(
                "ADL gotowe: {} (adapter {}, rodzina {}, Overdrive {}{}, log PM: {} sensorow)",
                gpu.name,
                gpu.adapter_index,
                gpu.family,
                gpu.overdrive_level,
                if gpu.overdrive_supported { "" } else { " - niedostepny" },
                gpu.pmlog_supported.len()
            );
            Ok(gpu)
        }
    }

    /// Lista sensorow logu PM; zwraca utworzone urzadzenie (do zniszczenia przez wywolujacego).
    unsafe fn pmlog_support_list(
        &self,
        adapter_index: i32,
        create: Option<PmLogDeviceCreateFn>,
        support: Option<PmLogSupportGetFn>,
    ) -> Option<(u32, Vec<u16>)> {
        let (create, support) = (create?, support?);
        let mut device = 0u32;
        unsafe {
            if create(self.context, adapter_index, &mut device) != ADL_OK {
                return None;
            }
            let mut info: AdlPmLogSupportInfo = std::mem::zeroed();
            if support(self.context, adapter_index, &mut info) != ADL_OK {
                self.destroy_pmlog_device(device);
                return None;
            }
            let list: Vec<u16> = info.sensors.iter().copied().take_while(|&s| s != 0).collect();
            Some((device, list))
        }
    }

    fn destroy_pmlog_device(&self, device: u32) {
        if let (Some(destroy), true) = (self.functions.pmlog_device_destroy, device != 0) {
            unsafe { destroy(self.context, device) };
        }
    }

    /// Poziom Overdrive: ADL2_Overdrive_Caps, a gdy brak - OD6, potem OD5 (jak w LHM).
    unsafe fn detect_overdrive(&mut self, library: HMODULE) {
        unsafe {
            let (mut supported, mut enabled, mut version) = (0i32, 0i32, 0i32);
            if let Some(caps) = symbol::<OverdriveCapsFn>(library, c"ADL2_Overdrive_Caps")
                && caps(self.context, self.adapter_index, &mut supported, &mut enabled, &mut version) == ADL_OK
            {
                self.overdrive_supported = supported == ADL_TRUE;
                self.overdrive_level = version;
                return;
            }
            if let Some(od6) = symbol::<Od6CapabilitiesGetFn>(library, c"ADL2_Overdrive6_Capabilities_Get") {
                let mut caps = AdlOd6Capabilities::default();
                if od6(self.context, self.adapter_index, &mut caps) == ADL_OK && caps.capabilities > 0 {
                    self.overdrive_supported = true;
                    self.overdrive_level = 6;
                    return;
                }
            }
            if let Some(od5) = symbol::<Od5ParametersGetFn>(library, c"ADL2_Overdrive5_ODParameters_Get") {
                let mut params = AdlOdParameters { size: size_of::<AdlOdParameters>() as i32, ..Default::default() };
                if od5(self.context, self.adapter_index, &mut params) == ADL_OK
                    && params.activity_reporting_supported > 0
                {
                    self.overdrive_supported = true;
                    self.overdrive_level = 5;
                    return;
                }
            }
            self.overdrive_level = -1;
        }
    }

    /// Start logu PM: sterownik co `PMLOG_SAMPLE_RATE_MS` zapisuje wartosci pod zwrocony adres.
    unsafe fn start_pmlog(
        &mut self,
        library: HMODULE,
        create: Option<PmLogDeviceCreateFn>,
        support: Option<PmLogSupportGetFn>,
    ) {
        unsafe {
            let Some(start) = symbol::<PmLogStartFn>(library, c"ADL2_Adapter_PMLog_Start") else {
                return;
            };
            let Some((device, list)) = self.pmlog_support_list(self.adapter_index, create, support) else {
                return;
            };
            let mut input: AdlPmLogStartInput = std::mem::zeroed();
            // Lista konczy sie zerem (ADL_SENSOR_MAXTYPES) - ostatni element zostaje 0.
            for (slot, sensor) in input.sensors.iter_mut().zip(list.iter().take(ADL_PMLOG_MAX_SENSORS - 1)) {
                *slot = *sensor;
            }
            input.sample_rate = PMLOG_SAMPLE_RATE_MS;
            let mut output: AdlPmLogStartOutput = std::mem::zeroed();
            if start(self.context, self.adapter_index, &mut input, &mut output, device) == ADL_OK
                && !output.logging_address.is_null()
            {
                self.pmlog_device = device;
                self.pmlog_address = output.logging_address.cast();
                self.pmlog_supported = list;
            } else {
                self.destroy_pmlog_device(device);
            }
        }
    }

    fn od5_fan(&self, speed_type: i32) -> Option<f32> {
        let f = self.functions.od5_fan_speed?;
        let mut value = AdlFanSpeedValue { size: size_of::<AdlFanSpeedValue>() as i32, speed_type, ..Default::default() };
        let ok = unsafe { f(self.context, self.adapter_index, 0, &mut value) } == ADL_OK;
        (ok && value.fan_speed >= 0).then_some(value.fan_speed as f32)
    }

    fn odn_temp(&self, kind: i32, scale: f64) -> Option<f32> {
        let f = self.functions.odn_temperature?;
        let mut raw = 0i32;
        let ok = unsafe { f(self.context, self.adapter_index, kind, &mut raw) } == ADL_OK;
        if ok { odn_temperature(raw, scale) } else { None }
    }

    /// Odczyty Overdrive 5/6/N (karty sprzed Vegi i jako baza dla nowszych - kolejnosc z LHM).
    fn read_legacy(&self, r: &mut GpuReading) {
        if let Some(f) = self.functions.od5_temperature {
            let mut t = AdlTemperature { size: size_of::<AdlTemperature>() as i32, ..Default::default() };
            if unsafe { f(self.context, self.adapter_index, 0, &mut t) } == ADL_OK {
                // OD5 podaje tysieczne czesci stopnia.
                r.temp_c = Some(t.temperature as f32 * 0.001);
            }
        }
        r.fan_rpm = self.od5_fan(FAN_SPEED_TYPE_RPM);
        r.fan_pct = self.od5_fan(FAN_SPEED_TYPE_PERCENT);
        if let Some(f) = self.functions.od5_activity {
            let mut a = AdlPmActivity { size: size_of::<AdlPmActivity>() as i32, ..Default::default() };
            if unsafe { f(self.context, self.adapter_index, &mut a) } == ADL_OK {
                // Zegary OD5 w jednostkach 10 kHz.
                r.clock_mhz = (a.engine_clock > 0).then_some(a.engine_clock as f32 * 0.01);
                r.memory_clock_mhz = (a.memory_clock > 0).then_some(a.memory_clock as f32 * 0.01);
                r.load_pct = Some(a.activity_percent.clamp(0, 100) as f32);
            }
        }
        if self.overdrive_level >= 6
            && let Some(f) = self.functions.od6_power
        {
            let mut power = 0i32;
            if unsafe { f(self.context, self.adapter_index, OD6_POWER_TOTAL, &mut power) } == ADL_OK {
                // Wartosc w formacie stalopozycyjnym 24.8 (W).
                r.power_w = Some((power >> 8) as f32);
            }
        }
        if self.overdrive_level >= 7 {
            if let Some(edge) = self.odn_temp(ODN_TEMP_EDGE, 0.001) {
                r.temp_c = Some(edge);
            }
            r.memory_temp_c = self.odn_temp(ODN_TEMP_MEM, 1.0).or(r.memory_temp_c);
            r.hotspot_c = self.odn_temp(ODN_TEMP_HOTSPOT, 1.0).or(r.hotspot_c);
        }
    }

    /// Odczyty Overdrive 8+: log PM (gdy swiezy), a w drugiej kolejnosci New_QueryPMLogData_Get.
    fn read_pmlog(&self, r: &mut GpuReading) {
        let od8 = self.functions.query_pmlog.and_then(|f| {
            let mut out: AdlPmLogDataOutput = unsafe { std::mem::zeroed() };
            (unsafe { f(self.context, self.adapter_index, &mut out) } == ADL_OK).then_some(out)
        });
        let pmlog = (!self.pmlog_address.is_null())
            .then(|| unsafe {
                // Pamiec zapisywana przez sterownik - odczyt volatile; adres nieznany, wiec sprawdzamy wyrownanie.
                if self.pmlog_address.is_aligned() {
                    std::ptr::read_volatile(self.pmlog_address)
                } else {
                    std::ptr::read_unaligned(self.pmlog_address)
                }
            })
            .filter(|d| pmlog_data_valid(d.last_updated, d.active_sample_rate, now_filetime()));
        let get = |sensor: u16| -> Option<f32> {
            let from_pmlog = pmlog
                .as_ref()
                .filter(|_| self.pmlog_supported.contains(&sensor))
                .and_then(|d| pmlog_value(&d.values, sensor))
                .map(|v| v as f32);
            from_pmlog.or_else(|| {
                let s = od8.as_ref()?.sensors.get(usize::from(sensor))?;
                (s.supported != 0).then_some(s.value as f32)
            })
        };

        r.clock_mhz = get(PMLOG_CLK_GFXCLK).or(r.clock_mhz);
        r.memory_clock_mhz = get(PMLOG_CLK_MEMCLK).or(r.memory_clock_mhz);
        r.temp_c = get(PMLOG_TEMPERATURE_EDGE).or_else(|| get(PMLOG_TEMPERATURE_GFX)).or(r.temp_c);
        r.memory_temp_c = get(PMLOG_TEMPERATURE_MEM).or(r.memory_temp_c);
        // HOTSPOT_GCD to rezerwa dla kart chipletowych (RX 7000) - LHM czyta tylko HOTSPOT.
        r.hotspot_c =
            get(PMLOG_TEMPERATURE_HOTSPOT).or_else(|| get(PMLOG_TEMPERATURE_HOTSPOT_GCD)).or(r.hotspot_c);
        r.fan_rpm = get(PMLOG_FAN_RPM).or(r.fan_rpm);
        r.fan_pct = get(PMLOG_FAN_PERCENTAGE).or(r.fan_pct);
        r.load_pct = get(PMLOG_INFO_ACTIVITY_GFX).map(|v| v.clamp(0.0, 100.0)).or(r.load_pct);
        // RDNA3+ (NV3): moc calej karty z BOARD_POWER; starsze - ASIC_POWER, a gdy brak - BOARD_POWER.
        let power = if self.family >= FAMILY_NV3 {
            get(PMLOG_BOARD_POWER)
        } else {
            get(PMLOG_ASIC_POWER).or_else(|| get(PMLOG_BOARD_POWER))
        };
        r.power_w = power.or(r.power_w);
    }

    pub fn read(&self) -> GpuReading {
        let mut r = GpuReading::default();
        if self.overdrive_supported {
            self.read_legacy(&mut r);
        }
        if self.overdrive_level >= 8 || !self.overdrive_supported {
            self.read_pmlog(&mut r);
        }
        if let Some(f) = self.functions.dedicated_vram_usage {
            let mut used_mb = 0i32;
            if unsafe { f(self.context, self.adapter_index, &mut used_mb) } == ADL_OK && used_mb >= 0 {
                r.vram_used_mb = Some(used_mb as f32);
            }
        }
        r.vram_total_mb = self.vram_total_mb;
        r
    }
}

impl Drop for AmdGpu {
    fn drop(&mut self) {
        unsafe {
            if self.pmlog_device != 0
                && let Some(stop) = self.functions.pmlog_stop
            {
                stop(self.context, self.adapter_index, self.pmlog_device);
            }
            self.destroy_pmlog_device(self.pmlog_device);
            if let Some(destroy) = self.functions.main_control_destroy {
                destroy(self.context);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structs_match_adl_sdk_layout() {
        assert_eq!(size_of::<AdlAdapterInfo>(), 1572);
        assert_eq!(size_of::<AdlPmActivity>(), 40);
        assert_eq!(size_of::<AdlTemperature>(), 8);
        assert_eq!(size_of::<AdlFanSpeedValue>(), 16);
        assert_eq!(size_of::<AdlOdParameters>(), 56);
        assert_eq!(size_of::<AdlOd6Capabilities>(), 44);
        assert_eq!(size_of::<AdlPmLogDataOutput>(), 4 + 256 * 8);
        assert_eq!(size_of::<AdlGcnInfo>(), 20);
        assert_eq!(size_of::<AdlPmLogSupportInfo>(), 576);
        assert_eq!(size_of::<AdlPmLogStartInput>(), 576);
        assert_eq!(size_of::<AdlPmLogStartOutput>(), 64);
        assert_eq!(size_of::<AdlPmLogData>(), 3088);
        assert_eq!(std::mem::offset_of!(AdlPmLogData, values), 16);
        assert_eq!(size_of::<AdlMemoryInfoX4>(), 320);
        assert_eq!(std::mem::offset_of!(AdlAdapterInfo, adapter_name), 280);
        assert_eq!(std::mem::offset_of!(AdlAdapterInfo, pnp_string), 1312);
    }

    #[test]
    fn allocator_round_trip() {
        unsafe {
            let ptr = adl_alloc(100);
            assert!(!ptr.is_null());
            assert_eq!(ptr as usize % ALLOC_HEADER, 0);
            std::ptr::write_bytes(ptr.cast::<u8>(), 0xAB, 100);
            adl_free(ptr);
            assert!(adl_alloc(-1).is_null());
            adl_free(std::ptr::null_mut());
        }
    }

    #[test]
    fn vendor_is_parsed_from_windows_udid() {
        assert_eq!(vendor_from_udid("PCI_VEN_1002&DEV_744C&SUBSYS_0E3B1002&REV_C8_6&1"), Some(0x1002));
        assert_eq!(vendor_from_udid("pci_ven_10de&dev_2782"), Some(0x10DE));
        assert_eq!(vendor_from_udid("0:0:1002:0:0"), None);
        assert_eq!(vendor_from_udid(""), None);
    }

    #[test]
    fn c_string_stops_at_nul() {
        let mut buf = [0u8; 16];
        buf[..13].copy_from_slice(b"AMD Radeon RX");
        assert_eq!(c_string(&buf), "AMD Radeon RX");
        assert_eq!(c_string(b"abc"), "abc");
    }

    fn candidate(index: i32, bus: i32, family: i32, active: bool, pmlog: usize) -> Candidate {
        Candidate {
            adapter_index: index,
            bus,
            device: 0,
            vendor: Some(ATI_VENDOR_ID),
            has_udid: true,
            active,
            family,
            pmlog_sensors: pmlog,
        }
    }

    #[test]
    fn picks_discrete_card_over_apu_and_collapses_duplicates() {
        let mut other_vendor = candidate(0, 1, 145, true, 30);
        other_vendor.vendor = Some(0x10DE);
        let list = vec![
            other_vendor,
            candidate(1, 5, 148, true, 20),  // APU Phoenix
            candidate(2, 3, 145, false, 0),  // RX 7000, nieaktywne wyjscie
            candidate(3, 3, 145, true, 40),  // ten sam GPU, aktywny z logiem PM
        ];
        assert_eq!(pick_adapter(&list), Some(3));
    }

    #[test]
    fn falls_back_to_apu_and_skips_entries_without_udid() {
        let mut no_udid = candidate(0, 2, 143, true, 0);
        no_udid.has_udid = false;
        let list = vec![no_udid, candidate(1, 5, 146, false, 0), candidate(2, 5, 146, true, 0)];
        assert_eq!(pick_adapter(&list), Some(2));
        assert_eq!(pick_adapter(&[]), None);
    }

    #[test]
    fn apu_families() {
        assert!(is_apu_family(142)); // Raven
        assert!(is_apu_family(148)); // Phoenix
        assert!(!is_apu_family(143)); // Navi 1x/2x
        assert!(!is_apu_family(145)); // Navi 3x
        assert!(!is_apu_family(0));
    }

    #[test]
    fn pmlog_helpers() {
        let mut values = [[0u32; 2]; ADL_PMLOG_MAX_SENSORS];
        values[0] = [u32::from(PMLOG_CLK_GFXCLK), 2500];
        values[1] = [u32::from(PMLOG_TEMPERATURE_HOTSPOT), 78];
        values[3] = [u32::from(PMLOG_FAN_RPM), 1200]; // za terminatorem - ignorowane
        assert_eq!(pmlog_value(&values, PMLOG_TEMPERATURE_HOTSPOT), Some(78));
        assert_eq!(pmlog_value(&values, PMLOG_CLK_GFXCLK), Some(2500));
        assert_eq!(pmlog_value(&values, PMLOG_FAN_RPM), None);

        let now = 133_000_000_000_000_000u64;
        assert!(pmlog_data_valid(now - 10_000_000, 1000, now));
        assert!(!pmlog_data_valid(0, 1000, now));
        assert!(!pmlog_data_valid(now, 90_000_000, now));
        assert!(!pmlog_data_valid(now - 49 * 3600 * 10_000_000, 1000, now));
        assert!(now_filetime() > now);
    }

    #[test]
    fn odn_temperature_range() {
        assert_eq!(odn_temperature(65_000, 0.001), Some(65.0));
        assert_eq!(odn_temperature(70, 1.0), Some(70.0));
        assert_eq!(odn_temperature(54_000, 1.0), None);
        assert_eq!(odn_temperature(-300, 1.0), None);
    }

    /// Na prawdziwym sprzecie: `cargo test -- --ignored --nocapture`; bez sterownika ma zwrocic blad, nie panike.
    #[test]
    #[ignore]
    fn init_on_real_hardware() {
        match AmdGpu::new() {
            Ok(gpu) => eprintln!("{}: {:?}", gpu.name, gpu.read()),
            Err(err) => eprintln!("niedostepne: {err}"),
        }
    }
}
