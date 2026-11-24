//! Katalog metryk, ktore overlay potrafi wyswietlic.
//!
//! Kazda metryka ma stale metadane (nazwa, jednostka, zakres skali, domyslne progi, podzespol).
//! Zakres skali sluzy do normalizacji wykresow i wskaznikow do przedzialu 0..1.
//! Zegary nie maja progow kolorow (wysoki zegar to nic zlego) - patrz `has_thresholds`.
//! Zaleznosci: serde (nazwa metryki zapisywana w config.toml).

use serde::{Deserialize, Serialize};

/// Podzespol, z ktorego pochodzi metryka - karta laczy tylko metryki jednego podzespolu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Cpu,
    Gpu,
    /// Pamiec operacyjna (RAM).
    Memory,
    /// Wentylatory plyty i AIO.
    Cooling,
    /// Klatki mierzonej gry (FPS, czas klatki).
    Game,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    CpuLoad,
    CpuTemp,
    CpuClock,
    CpuPower,
    GpuLoad,
    GpuTemp,
    GpuHotspot,
    GpuMemTemp,
    GpuClock,
    GpuMemClock,
    GpuPower,
    GpuFan,
    VramUsage,
    Fps,
    FpsAvg,
    FpsLow1,
    FpsLow01,
    FrameTime,
    RamUsage,
    RamUsed,
    /// Obroty wentylatorow i AIO - karta "Wentylatory" pokazuje wszystkie, wartosc = najszybszy.
    FanSpeed,
}

impl Metric {
    pub const ALL: [Metric; 21] = [
        Metric::CpuLoad,
        Metric::CpuTemp,
        Metric::CpuClock,
        Metric::CpuPower,
        Metric::GpuLoad,
        Metric::GpuTemp,
        Metric::GpuHotspot,
        Metric::GpuMemTemp,
        Metric::GpuClock,
        Metric::GpuMemClock,
        Metric::GpuPower,
        Metric::GpuFan,
        Metric::VramUsage,
        Metric::Fps,
        Metric::FpsAvg,
        Metric::FpsLow1,
        Metric::FpsLow01,
        Metric::FrameTime,
        Metric::RamUsage,
        Metric::RamUsed,
        Metric::FanSpeed,
    ];

    /// Pozycja w `ALL` - indeks w tablicach odczytow czujnikow.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).expect("kazda metryka jest w ALL")
    }

    pub fn device(self) -> Device {
        match self {
            Metric::CpuLoad | Metric::CpuTemp | Metric::CpuClock | Metric::CpuPower => Device::Cpu,
            Metric::Fps | Metric::FpsAvg | Metric::FpsLow1 | Metric::FpsLow01 | Metric::FrameTime => Device::Game,
            Metric::RamUsage | Metric::RamUsed => Device::Memory,
            Metric::FanSpeed => Device::Cooling,
            _ => Device::Gpu,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Metric::CpuLoad => "Obciążenie CPU",
            Metric::CpuTemp => "Temperatura CPU",
            Metric::CpuClock => "Zegar CPU",
            Metric::CpuPower => "Pobór mocy CPU",
            Metric::GpuLoad => "Obciążenie GPU",
            Metric::GpuTemp => "Temperatura GPU",
            Metric::GpuHotspot => "Hotspot GPU",
            Metric::GpuMemTemp => "Temperatura pamięci GPU",
            Metric::GpuClock => "Zegar GPU",
            Metric::GpuMemClock => "Zegar pamięci GPU",
            Metric::GpuPower => "Pobór mocy GPU",
            Metric::GpuFan => "Wentylator GPU",
            Metric::VramUsage => "Zajętość VRAM",
            Metric::Fps => "FPS",
            Metric::FpsAvg => "Średni FPS",
            Metric::FpsLow1 => "1% low FPS",
            Metric::FpsLow01 => "0.1% low FPS",
            Metric::FrameTime => "Czas klatki",
            Metric::RamUsage => "Zajętość RAM",
            Metric::RamUsed => "Użyta pamięć RAM",
            Metric::FanSpeed => "Wentylatory i AIO",
        }
    }

    /// Krotka etykieta wyswietlana na karcie.
    pub fn short_label(self) -> &'static str {
        match self {
            Metric::CpuLoad => "CPU",
            Metric::CpuTemp => "CPU TEMP",
            Metric::CpuClock => "CPU CLK",
            Metric::CpuPower => "CPU PWR",
            Metric::GpuLoad => "GPU",
            Metric::GpuTemp => "GPU TEMP",
            Metric::GpuHotspot => "HOTSPOT",
            Metric::GpuMemTemp => "VRAM TEMP",
            Metric::GpuClock => "GPU CLK",
            Metric::GpuMemClock => "MEM CLK",
            Metric::GpuPower => "GPU PWR",
            Metric::GpuFan => "FAN",
            Metric::VramUsage => "VRAM",
            Metric::Fps => "FPS",
            Metric::FpsAvg => "AVG",
            Metric::FpsLow1 => "1% LOW",
            Metric::FpsLow01 => "0.1% LOW",
            Metric::FrameTime => "FRAME TIME",
            Metric::RamUsage => "RAM",
            Metric::RamUsed => "RAM",
            Metric::FanSpeed => "FANS",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Metric::CpuTemp | Metric::GpuTemp | Metric::GpuHotspot | Metric::GpuMemTemp => "°C",
            Metric::CpuClock | Metric::GpuClock | Metric::GpuMemClock => " MHz",
            Metric::CpuPower | Metric::GpuPower => " W",
            Metric::Fps | Metric::FpsAvg | Metric::FpsLow1 | Metric::FpsLow01 => "",
            Metric::FrameTime => " ms",
            Metric::RamUsed => " GB",
            Metric::FanSpeed => " RPM",
            _ => "%",
        }
    }

    /// Zakres skali (min, max) uzywany do normalizacji wartosci.
    pub fn range(self) -> (f32, f32) {
        match self {
            // Ponizej ~25 C czujniki praktycznie nie schodza, wiec skala od 20 lepiej wykorzystuje wykres.
            Metric::CpuTemp | Metric::GpuTemp => (20.0, 100.0),
            Metric::GpuHotspot | Metric::GpuMemTemp => (20.0, 110.0),
            Metric::CpuClock => (0.0, 6000.0),
            Metric::GpuClock => (0.0, 3200.0),
            // GDDR6X/GDDR7 raportuja efektywny zegar rzedu 10-16 GHz.
            Metric::GpuMemClock => (0.0, 16000.0),
            Metric::CpuPower => (0.0, 300.0),
            Metric::GpuPower => (0.0, 450.0),
            Metric::Fps | Metric::FpsAvg | Metric::FpsLow1 | Metric::FpsLow01 => (0.0, 240.0),
            Metric::FrameTime => (0.0, 50.0),
            // Skala stala (typowy PC do gier); wykres i tak pokazuje zmiany, procent jest w RamUsage.
            Metric::RamUsed => (0.0, 64.0),
            Metric::FanSpeed => (0.0, 3000.0),
            _ => (0.0, 100.0),
        }
    }

    /// Czy metryka ma progi kolorow (zegary nie maja - wysoki zegar to nic zlego).
    pub fn has_thresholds(self) -> bool {
        !matches!(self, Metric::CpuClock | Metric::GpuClock | Metric::GpuMemClock | Metric::RamUsed | Metric::FanSpeed)
    }

    /// Czy wieksza wartosc jest gorsza (temperatura, obciazenie). Dla FPS jest odwrotnie -
    /// ostrzezenie pojawia sie, gdy wartosc SPADA ponizej progu.
    pub fn higher_is_worse(self) -> bool {
        !matches!(self, Metric::Fps | Metric::FpsAvg | Metric::FpsLow1 | Metric::FpsLow01)
    }

    /// Liczba miejsc po przecinku przy wyswietlaniu.
    pub fn decimals(self) -> usize {
        if matches!(self, Metric::FrameTime | Metric::RamUsed) { 1 } else { 0 }
    }

    /// Domyslne progi (ostrzezenie, krytyczny) w jednostkach metryki.
    /// Dla metryk bez progow - powyzej zakresu, wiec kolor nigdy sie nie zmienia.
    pub fn default_thresholds(self) -> (f32, f32) {
        match self {
            Metric::CpuLoad | Metric::GpuLoad => (75.0, 95.0),
            Metric::CpuTemp => (75.0, 90.0),
            Metric::GpuTemp => (72.0, 83.0),
            Metric::GpuHotspot => (85.0, 100.0),
            Metric::GpuMemTemp => (90.0, 100.0),
            Metric::CpuPower => (150.0, 220.0),
            Metric::GpuPower => (250.0, 320.0),
            Metric::GpuFan => (80.0, 95.0),
            Metric::VramUsage => (80.0, 95.0),
            // FPS: ostrzezenie ponizej pierwszej wartosci, krytyczny ponizej drugiej.
            Metric::Fps | Metric::FpsAvg => (60.0, 30.0),
            Metric::FpsLow1 => (45.0, 25.0),
            Metric::FpsLow01 => (30.0, 20.0),
            Metric::FrameTime => (25.0, 40.0),
            Metric::RamUsage => (80.0, 92.0),
            // Ilosc GB zalezy od wielkosci pamieci - kolor ostrzezenia daje RamUsage.
            Metric::CpuClock | Metric::GpuClock | Metric::GpuMemClock | Metric::RamUsed | Metric::FanSpeed => {
                let (_, max) = self.range();
                (max * 2.0, max * 2.0)
            }
        }
    }

    pub fn normalize(self, value: f32) -> f32 {
        let (min, max) = self.range();
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_position_in_all() {
        for (i, metric) in Metric::ALL.iter().enumerate() {
            assert_eq!(metric.index(), i);
        }
    }

    #[test]
    fn fps_thresholds_point_downwards() {
        let (warn, crit) = Metric::Fps.default_thresholds();
        assert!(!Metric::Fps.higher_is_worse() && crit < warn);
        assert!(Metric::FrameTime.higher_is_worse());
    }

    #[test]
    fn clocks_never_change_color() {
        let (warn, _) = Metric::GpuClock.default_thresholds();
        assert!(warn > Metric::GpuClock.range().1);
        assert!(!Metric::GpuClock.has_thresholds() && Metric::GpuHotspot.has_thresholds());
    }
}
