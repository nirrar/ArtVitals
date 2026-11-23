//! Pomiar klatek (FPS, sredni FPS, 1% / 0.1% low, czas klatki) przez ETW - jak PresentMon.
//!
//! Wlasna sesja ETW czasu rzeczywistego subskrybuje zdarzenia "Present" dostawcow:
//! - Microsoft-Windows-DXGI {CA11C036-0102-4A2D-A6AD-F03CFED5D3C9}: Present_Start (42)
//!   i PresentMultiplaneOverlay_Start (55) - DirectX 10/11/12 (i Vulkan/OpenGL, gdy sterownik
//!   prezentuje przez DXGI); prezentacje z flaga DXGI_PRESENT_TEST sa pomijane (nic nie wyswietlaja),
//! - Microsoft-Windows-D3D9 {783ACA0A-790E-4D7F-8451-AA850511C6B9}: Present_Start (1) - DirectX 9.
//!
//! Identyfikatory wziete z PresentMon (PresentData/ETW/*.h, licencja MIT).
//! Czas klatki = odstep miedzy kolejnymi prezentacjami procesu ("MsBetweenPresents").
//! Mierzony jest proces z aktywnym oknem; gdy nie prezentuje - najaktywniejszy inny proces.
//! Wymaga uprawnien administratora (tworzenie sesji ETW).

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::Etw::{
    CONTROLTRACE_HANDLE, CloseTrace, ControlTraceW, EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_HEADER_FLAG_32_BIT_HEADER,
    EVENT_RECORD, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    EnableTraceEx2, OpenTraceW, PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, PROCESSTRACE_HANDLE,
    ProcessTrace, StartTraceW, TRACE_LEVEL_INFORMATION, WNODE_FLAG_TRACED_GUID,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::{GUID, HSTRING, PWSTR};

const SESSION_NAME: &str = "ArtVitals-FrameTiming";
const DXGI_PROVIDER: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
const D3D9_PROVIDER: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
/// Microsoft_Windows_*_Analytic | Events - jak w PresentMon.
const PRESENT_KEYWORDS: u64 = 0x8000_0000_0000_0002;
const DXGI_PRESENT_START: u16 = 42;
const DXGI_PRESENT_MPO_START: u16 = 55;
const D3D9_PRESENT_START: u16 = 1;
const DXGI_PRESENT_TEST: u32 = 1;

/// Ile ostatnich klatek trafia na wykres czasu klatki.
pub const FRAME_GRAPH_LEN: usize = 240;
/// Odstep dluzszy niz ten nie jest klatka, tylko przerwa (np. gra w tle) - nie wchodzi do statystyk.
const MAX_FRAME_MS: f32 = 1000.0;
/// Okno "biezacego" FPS i czasu klatki.
const CURRENT_WINDOW_S: f64 = 1.0;
/// Proces bez klatek przez tyle sekund przestaje byc mierzony.
const ACTIVE_TIMEOUT_S: f64 = 2.0;
/// Klatki starsze niz tyle sekund sa usuwane (gorna granica okna statystyk).
const MAX_KEEP_S: f64 = 600.0;

#[derive(Default)]
struct ProcessFrames {
    last_present: Option<i64>,
    /// (znacznik QPC, czas klatki w ms)
    frames: VecDeque<(i64, f32)>,
}

#[derive(Default)]
struct FrameLog {
    qpc_frequency: f64,
    processes: HashMap<u32, ProcessFrames>,
}

impl FrameLog {
    fn record_present(&mut self, pid: u32, timestamp: i64) {
        let keep = (MAX_KEEP_S * self.qpc_frequency) as i64;
        let frequency = self.qpc_frequency;
        let process = self.processes.entry(pid).or_default();
        if let Some(last) = process.last_present.replace(timestamp) {
            let ms = ((timestamp - last) as f64 / frequency * 1000.0) as f32;
            if ms > 0.0 && ms < MAX_FRAME_MS {
                process.frames.push_back((timestamp, ms));
            }
        }
        while process.frames.front().is_some_and(|(t, _)| timestamp - t > keep) {
            process.frames.pop_front();
        }
    }
}

/// Wynik pomiaru dla mierzonego procesu.
#[derive(Debug, Default, Clone)]
pub struct FpsReading {
    pub fps: Option<f32>,
    pub avg_fps: Option<f32>,
    pub low1_fps: Option<f32>,
    pub low01_fps: Option<f32>,
    pub frame_time_ms: Option<f32>,
    /// Ostatnie czasy klatek (ms) do wykresu, najstarsza pierwsza.
    pub recent_frames: Vec<f32>,
    /// Nazwa pliku exe mierzonego procesu.
    pub process: Option<String>,
}

pub struct FpsMonitor {
    frame_log: Arc<Mutex<FrameLog>>,
    trace: PROCESSTRACE_HANDLE,
    thread: Option<std::thread::JoinHandle<()>>,
    own_pid: u32,
    names: HashMap<u32, String>,
}

// Uchwyty ETW sa uzywane wylacznie z watku czujnikow (i watku ProcessTrace, ktory sam je zamyka).
unsafe impl Send for FpsMonitor {}

/// Bufor EVENT_TRACE_PROPERTIES z nazwa sesji doklejona na koncu (tak wymaga ETW).
fn properties_buffer() -> Vec<u64> {
    let name_bytes = (SESSION_NAME.encode_utf16().count() + 1) * 2;
    let total = size_of::<EVENT_TRACE_PROPERTIES>() + name_bytes;
    // Bufor u64 - wyrownanie wystarczajace dla struktury.
    let mut buffer = vec![0u64; total.div_ceil(8)];
    let props = buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
    unsafe {
        (*props).Wnode.BufferSize = total as u32;
        (*props).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        // 1 = znaczniki czasu w jednostkach QueryPerformanceCounter.
        (*props).Wnode.ClientContext = 1;
        (*props).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*props).FlushTimer = 1;
        (*props).LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    }
    buffer
}

/// Zatrzymuje sesje ETW pomiaru klatek (wolane tez przy zamykaniu programu - sesja czasu
/// rzeczywistego przezylaby proces, gdyby watek czujnikow nie zdazyl sie zakonczyc).
pub fn stop_session() -> u32 {
    let mut buffer = properties_buffer();
    unsafe {
        ControlTraceW(
            CONTROLTRACE_HANDLE::default(),
            &HSTRING::from(SESSION_NAME),
            buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
            EVENT_TRACE_CONTROL_STOP,
        )
        .0
    }
}

unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    let Some(record) = (unsafe { record.as_ref() }) else { return };
    let header = &record.EventHeader;
    let id = header.EventDescriptor.Id;
    let is_present = if header.ProviderId == DXGI_PROVIDER {
        if id != DXGI_PRESENT_START && id != DXGI_PRESENT_MPO_START {
            return;
        }
        if id == DXGI_PRESENT_START {
            // Dane: wskaznik pIDXGISwapChain (4 lub 8 bajtow, zaleznie od bitowosci gry), potem Flags (u32).
            let pointer_size = if u32::from(header.Flags) & EVENT_HEADER_FLAG_32_BIT_HEADER != 0 { 4 } else { 8 };
            if usize::from(record.UserDataLength) >= pointer_size + 4 && !record.UserData.is_null() {
                let flags = unsafe {
                    std::ptr::read_unaligned((record.UserData as *const u8).add(pointer_size) as *const u32)
                };
                if flags & DXGI_PRESENT_TEST != 0 {
                    return;
                }
            }
        }
        true
    } else {
        header.ProviderId == D3D9_PROVIDER && id == D3D9_PRESENT_START
    };
    if !is_present || record.UserContext.is_null() {
        return;
    }
    let log = unsafe { &*(record.UserContext as *const Mutex<FrameLog>) };
    if let Ok(mut log) = log.lock() {
        log.record_present(header.ProcessId, header.TimeStamp);
    }
}

impl FpsMonitor {
    pub fn start() -> Result<Self, String> {
        let mut frequency = 0i64;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(|err| format!("QPC: {err}"))?;
        let frame_log = Arc::new(Mutex::new(FrameLog { qpc_frequency: frequency as f64, ..Default::default() }));

        let name = HSTRING::from(SESSION_NAME);
        let mut session = CONTROLTRACE_HANDLE::default();
        let mut buffer = properties_buffer();
        let mut status =
            unsafe { StartTraceW(&mut session, &name, buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
        if status == ERROR_ALREADY_EXISTS {
            // Sesja zostala po poprzednim (np. przerwanym) uruchomieniu - zamykamy i zakladamy od nowa.
            stop_session();
            buffer = properties_buffer();
            status = unsafe { StartTraceW(&mut session, &name, buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
        }
        if status == ERROR_ACCESS_DENIED {
            return Err("Wymaga uruchomienia jako administrator".to_owned());
        }
        if status != ERROR_SUCCESS {
            return Err(format!("StartTrace: blad {}", status.0));
        }

        for provider in [DXGI_PROVIDER, D3D9_PROVIDER] {
            let status = unsafe {
                EnableTraceEx2(
                    session,
                    &provider,
                    EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                    TRACE_LEVEL_INFORMATION as u8,
                    PRESENT_KEYWORDS,
                    0,
                    0,
                    None,
                )
            };
            if status != ERROR_SUCCESS {
                stop_session();
                return Err(format!("EnableTraceEx2: blad {}", status.0));
            }
        }

        let mut logger_name: Vec<u16> = SESSION_NAME.encode_utf16().chain(std::iter::once(0)).collect();
        let mut logfile = EVENT_TRACE_LOGFILEW {
            LoggerName: PWSTR(logger_name.as_mut_ptr()),
            // Arc zyje w FpsMonitor dluzej niz watek ProcessTrace (Drop najpierw zatrzymuje sesje).
            Context: Arc::as_ptr(&frame_log) as *mut c_void,
            ..Default::default()
        };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        let trace = unsafe { OpenTraceW(&mut logfile) };
        if trace.Value == u64::MAX {
            stop_session();
            return Err("OpenTrace nieudane".to_owned());
        }

        // ProcessTrace blokuje az do zatrzymania sesji - osobny watek.
        let thread = std::thread::Builder::new()
            .name("fps-etw".to_owned())
            .spawn(move || {
                let status = unsafe { ProcessTrace(&[trace], None, None) };
                if status != ERROR_SUCCESS {
                    log::warn!("ProcessTrace zakonczone bledem {}", status.0);
                }
            })
            .map_err(|err| {
                stop_session();
                format!("Watek ETW: {err}")
            })?;

        log::info!("Pomiar FPS (ETW) uruchomiony");
        Ok(Self { frame_log, trace, thread: Some(thread), own_pid: std::process::id(), names: HashMap::new() })
    }

    /// Pomiar dla aktywnej gry; `window_s` - okno sredniego FPS i lows (jak historia wykresu).
    pub fn read(&mut self, window_s: f64) -> FpsReading {
        let mut now = 0i64;
        if unsafe { QueryPerformanceCounter(&mut now) }.is_err() {
            return FpsReading::default();
        }
        let foreground = foreground_pid();
        let own_pid = self.own_pid;
        let (pid, frames, frequency) = {
            let Ok(mut log) = self.frame_log.lock() else { return FpsReading::default() };
            let frequency = log.qpc_frequency;
            let timeout = (ACTIVE_TIMEOUT_S * frequency) as i64;
            log.processes.retain(|_, p| p.last_present.is_some_and(|t| now - t < (MAX_KEEP_S * frequency) as i64));
            let active = |p: &ProcessFrames| p.last_present.is_some_and(|t| now - t < timeout);
            let pid = foreground
                .filter(|pid| *pid != own_pid && log.processes.get(pid).is_some_and(active))
                .or_else(|| {
                    log.processes
                        .iter()
                        .filter(|(pid, p)| **pid != own_pid && active(p))
                        .max_by_key(|(_, p)| p.frames.iter().filter(|(t, _)| now - t < timeout).count())
                        .map(|(pid, _)| *pid)
                });
            let Some(pid) = pid else { return FpsReading::default() };
            let frames: Vec<(i64, f32)> = log.processes[&pid].frames.iter().copied().collect();
            (pid, frames, frequency)
        };
        let mut reading = compute_stats(&frames, now, frequency, window_s);
        reading.process = Some(self.process_name(pid));
        reading
    }

    fn process_name(&mut self, pid: u32) -> String {
        self.names.entry(pid).or_insert_with(|| process_exe_name(pid).unwrap_or_else(|| format!("PID {pid}"))).clone()
    }
}

impl Drop for FpsMonitor {
    fn drop(&mut self) {
        // Zatrzymanie sesji konczy ProcessTrace w watku ETW.
        stop_session();
        unsafe {
            let _ = CloseTrace(self.trace);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn foreground_pid() -> Option<u32> {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

fn process_exe_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 512];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(handle);
        result.ok()?;
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_owned)
    }
}

/// Statystyki z listy klatek (znacznik QPC, czas klatki ms):
/// - FPS i czas klatki: srednia z ostatniej sekundy,
/// - sredni FPS: 1000 / sredni czas klatki w oknie `window_s`,
/// - 1% / 0.1% low: FPS przy 99. / 99.9. percentylu czasu klatki w tym samym oknie
///   (definicja "percentylowa", jak w MSI Afterburner i CapFrameX "P1").
fn compute_stats(frames: &[(i64, f32)], now: i64, frequency: f64, window_s: f64) -> FpsReading {
    let since = |seconds: f64| now - (seconds * frequency) as i64;
    // Okna (teraz - N s, teraz] - klatka dokladnie na granicy nalezy juz do poprzedniego okna.
    let recent: Vec<f32> = frames.iter().filter(|(t, _)| *t > since(CURRENT_WINDOW_S)).map(|(_, ms)| *ms).collect();
    let mut window: Vec<f32> = frames.iter().filter(|(t, _)| *t > since(window_s)).map(|(_, ms)| *ms).collect();

    let mean = |v: &[f32]| (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32);
    let frame_time_ms = mean(&recent);
    let window_mean = mean(&window);
    window.sort_by(f32::total_cmp);
    let percentile = |p: f32| {
        if window.is_empty() {
            return None;
        }
        let index = ((window.len() as f32 * p).ceil() as usize).clamp(1, window.len()) - 1;
        Some(window[index])
    };
    let to_fps = |ms: Option<f32>| ms.filter(|ms| *ms > 0.0).map(|ms| 1000.0 / ms);

    FpsReading {
        fps: to_fps(frame_time_ms),
        avg_fps: to_fps(window_mean),
        low1_fps: to_fps(percentile(0.99)),
        low01_fps: to_fps(percentile(0.999)),
        frame_time_ms,
        recent_frames: frames.iter().rev().take(FRAME_GRAPH_LEN).rev().map(|(_, ms)| *ms).collect(),
        process: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_match_known_frame_series() {
        let frequency = 1000.0; // 1 tik = 1 ms
        // 1000 klatek po 10 ms (100 FPS) i 20 klatek po 50 ms (zaciecia, ~2% klatek).
        let mut frames = Vec::new();
        let mut t = 0i64;
        for _ in 0..1000 {
            t += 10;
            frames.push((t, 10.0));
        }
        for _ in 0..20 {
            t += 50;
            frames.push((t, 50.0));
        }
        let reading = compute_stats(&frames, t, frequency, 60.0);
        let avg = reading.avg_fps.unwrap();
        assert!((avg - 1000.0 / (11000.0 / 1020.0)).abs() < 0.01);
        // 99. percentyl czasu klatki trafia w zaciecia 50 ms -> 1% low = 20 FPS.
        assert!((reading.low1_fps.unwrap() - 20.0).abs() < 0.01);
        // Ostatnia sekunda to same zaciecia (20 x 50 ms) -> biezacy FPS = 20.
        assert!((reading.fps.unwrap() - 20.0).abs() < 0.01);
        assert!((reading.frame_time_ms.unwrap() - 50.0).abs() < 0.01);
        assert_eq!(reading.recent_frames.len(), FRAME_GRAPH_LEN);
    }

    #[test]
    fn long_gaps_are_not_frames() {
        let mut log = FrameLog { qpc_frequency: 1000.0, ..Default::default() };
        log.record_present(7, 0);
        log.record_present(7, 5000); // 5 s przerwy - gra w tle
        log.record_present(7, 5016);
        assert_eq!(log.processes[&7].frames.len(), 1);
    }
}
