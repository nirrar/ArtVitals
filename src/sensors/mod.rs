//! Zbieranie odczytow czujnikow w osobnym watku.
//!
//! Watek co `interval_ms` odczytuje wszystkie zrodla, dopisuje probki do historii
//! (bufor kolowy o dlugosci `history_len`) i budzi UI przez `request_repaint`.
//! UI tylko czyta wspolny stan pod krotko trzymanym Mutexem.
//! Zrodla, ktore sie nie zainicjalizowaly (np. temp. CPU bez admina), daja `None`
//! a powod trafia do `SensorData::issues`, zeby pokazac go w oknie konfiguracji.

mod cpu_load;
mod cpu_temp;
pub mod fps;
mod gpu;
mod nvapi;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::metrics::Metric;

#[derive(Default)]
pub struct SensorData {
    pub current: [Option<f32>; Metric::ALL.len()],
    /// Historia probek; brak odczytu zapisywany jako NaN, zeby wykres pokazal przerwe.
    pub history: [VecDeque<f32>; Metric::ALL.len()],
    pub vram_used_mb: Option<f32>,
    pub vram_total_mb: Option<f32>,
    pub gpu_name: Option<String>,
    /// Czasy ostatnich klatek (ms), klatka po klatce - wykres czasu klatki jest dokladniejszy
    /// niz probki co sekunde.
    pub frame_times: VecDeque<f32>,
    /// Plik exe gry, ktorej klatki sa mierzone.
    pub fps_process: Option<String>,
    /// Problemy z inicjalizacja zrodel: (zrodlo, opis).
    pub issues: Vec<(&'static str, String)>,
}

impl SensorData {
    pub fn value(&self, metric: Metric) -> Option<f32> {
        self.current[metric.index()]
    }

    pub fn history(&self, metric: Metric) -> &VecDeque<f32> {
        &self.history[metric.index()]
    }

    /// Dane do wykresu: (probki, stala dlugosc osi X). Czas klatki - klatka po klatce,
    /// pozostale metryki - probki co interwal odczytu (dlugosc osi X wg historii).
    pub fn series(&self, metric: Metric) -> (&VecDeque<f32>, Option<usize>) {
        if metric == Metric::FrameTime && !self.frame_times.is_empty() {
            (&self.frame_times, Some(fps::FRAME_GRAPH_LEN))
        } else {
            (self.history(metric), None)
        }
    }

    /// (min, srednia, max) z poprawnych probek historii.
    pub fn stats(&self, metric: Metric) -> Option<(f32, f32, f32)> {
        let mut count = 0usize;
        let (mut min, mut max, mut sum) = (f32::MAX, f32::MIN, 0.0f32);
        for &v in self.series(metric).0.iter().filter(|v| v.is_finite()) {
            min = min.min(v);
            max = max.max(v);
            sum += v;
            count += 1;
        }
        (count > 0).then(|| (min, sum / count as f32, max))
    }

    fn push(&mut self, metric: Metric, value: Option<f32>, history_len: usize) {
        let i = metric.index();
        self.current[i] = value;
        let history = &mut self.history[i];
        history.push_back(value.unwrap_or(f32::NAN));
        while history.len() > history_len {
            history.pop_front();
        }
    }
}

#[derive(Clone)]
pub struct SensorHub {
    data: Arc<Mutex<SensorData>>,
    interval_ms: Arc<AtomicU64>,
    history_len: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl SensorHub {
    pub fn start(ctx: eframe::egui::Context, interval_ms: u64, history_len: usize) -> Self {
        let hub = Self {
            data: Arc::new(Mutex::new(SensorData::default())),
            interval_ms: Arc::new(AtomicU64::new(interval_ms)),
            history_len: Arc::new(AtomicUsize::new(history_len)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let worker = hub.clone();
        let spawned = std::thread::Builder::new()
            .name("sensors".to_owned())
            .spawn(move || worker.run(ctx));
        if let Err(err) = spawned {
            log::error!("Nie udalo sie uruchomic watku czujnikow: {err}");
        }
        hub
    }

    pub fn lock(&self) -> MutexGuard<'_, SensorData> {
        // Zatruty mutex (panika w watku) nie powinien zabic UI - dane i tak sa tylko do odczytu.
        self.data.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_timing(&self, interval_ms: u64, history_len: usize) {
        self.interval_ms.store(interval_ms, Ordering::Relaxed);
        self.history_len.store(history_len, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn run(self, ctx: eframe::egui::Context) {
        let mut issues = Vec::new();
        let mut cpu_load = cpu_load::CpuLoadSensor::new()
            .map_err(|err| issues.push(("Obciążenie CPU", err)))
            .ok();
        let mut cpu_temp = cpu_temp::CpuTempSensor::new()
            .map_err(|err| issues.push(("Temperatura i moc CPU", err)))
            .ok();
        let gpu = gpu::GpuSensor::new().map_err(|err| issues.push(("GPU", err))).ok();
        if let Some(issue) = gpu.as_ref().and_then(|g| g.thermals_issue.clone()) {
            issues.push(("Hotspot i temperatura pamięci GPU", issue));
        }
        let mut fps_monitor = fps::FpsMonitor::start().map_err(|err| issues.push(("Pomiar FPS", err))).ok();
        for (source, issue) in &issues {
            log::warn!("{source}: {issue}");
        }
        {
            let mut data = self.lock();
            data.issues = issues;
            data.gpu_name = gpu.as_ref().map(|g| g.name.clone());
        }

        while !self.stop.load(Ordering::Relaxed) {
            let started = Instant::now();
            let cpu = cpu_load.as_mut().map(|s| s.read()).unwrap_or_default();
            let temp = cpu_temp.as_mut().and_then(|s| s.read());
            let cpu_power = cpu_temp.as_mut().and_then(|s| s.read_power());
            let gpu_reading = gpu.as_ref().map(|g| g.read()).unwrap_or_default();
            // Sredni FPS i lows liczone w tym samym oknie co historia wykresow.
            let window_s = self.history_len.load(Ordering::Relaxed) as f64
                * self.interval_ms.load(Ordering::Relaxed) as f64
                / 1000.0;
            let frames = fps_monitor.as_mut().map(|m| m.read(window_s)).unwrap_or_default();
            let vram_pct = match (gpu_reading.vram_used_mb, gpu_reading.vram_total_mb) {
                (Some(used), Some(total)) if total > 0.0 => Some(used / total * 100.0),
                _ => None,
            };

            {
                let history_len = self.history_len.load(Ordering::Relaxed).max(2);
                let mut data = self.lock();
                let g = &gpu_reading;
                let values = [
                    (Metric::CpuLoad, cpu.load_pct),
                    (Metric::CpuTemp, temp),
                    (Metric::CpuClock, cpu.clock_mhz),
                    (Metric::CpuPower, cpu_power),
                    (Metric::GpuLoad, g.load_pct),
                    (Metric::GpuTemp, g.temp_c),
                    (Metric::GpuHotspot, g.hotspot_c),
                    (Metric::GpuMemTemp, g.memory_temp_c),
                    (Metric::GpuClock, g.clock_mhz),
                    (Metric::GpuMemClock, g.memory_clock_mhz),
                    (Metric::GpuPower, g.power_w),
                    (Metric::GpuFan, g.fan_pct),
                    (Metric::VramUsage, vram_pct),
                    (Metric::Fps, frames.fps),
                    (Metric::FpsAvg, frames.avg_fps),
                    (Metric::FpsLow1, frames.low1_fps),
                    (Metric::FpsLow01, frames.low01_fps),
                    (Metric::FrameTime, frames.frame_time_ms),
                ];
                for (metric, value) in values {
                    data.push(metric, value, history_len);
                }
                data.vram_used_mb = gpu_reading.vram_used_mb;
                data.vram_total_mb = gpu_reading.vram_total_mb;
                data.frame_times = frames.recent_frames.into_iter().collect();
                data.fps_process = frames.process;
            }
            ctx.request_repaint();

            let interval = Duration::from_millis(self.interval_ms.load(Ordering::Relaxed));
            if let Some(rest) = interval.checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_trimmed_and_stats_skip_gaps() {
        let mut data = SensorData::default();
        for v in [Some(10.0), None, Some(30.0), Some(20.0)] {
            data.push(Metric::CpuLoad, v, 3);
        }
        assert_eq!(data.history(Metric::CpuLoad).len(), 3);
        assert_eq!(data.stats(Metric::CpuLoad), Some((20.0, 25.0, 30.0)));
        assert_eq!(data.value(Metric::CpuLoad), Some(20.0));
    }
}
