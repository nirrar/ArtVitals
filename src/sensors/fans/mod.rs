//! Wentylatory plyty glownej i chlodzenia AIO.
//!
//! Wspolny model odczytow dla karty "Wentylatory". Zrodla (do zaimplementowania):
//! - uklad Super I/O plyty glownej (Nuvoton NCT67xx, ITE IT86xx/IT87xx) przez PawnIO + LpcIO,
//! - AIO przez USB HID (NZXT Kraken, Corsair).
//!
//! Kontrakt dla reszty programu (NIE zmieniac bez aktualizacji `sensors/mod.rs` i `widgets.rs`):
//! `FanMonitor::new()` -> (monitor, lista problemow), `FanMonitor::read()` -> `FanSnapshot`.
//! Kazdy wentylator ma stabilne `id` (np. `superio:nct6798:fan2`, `aio:kraken-x53:pump`),
//! bo konfiguracja karty zapamietuje, ktore wentylatory pokazac.

/// Rodzaj wentylatora - wplywa na ikone i kolejnosc na karcie.
// Warianty tworza zrodla (Super I/O, AIO) - do czasu ich dodania nieuzywane.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanKind {
    /// Zlacze wentylatora na plycie glownej (CPU_FAN, SYS_FAN...).
    Board,
    /// Pompa AIO.
    Pump,
    /// Wentylator podpiety do AIO / kontrolera.
    AioFan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanReading {
    /// Stabilny identyfikator (zrodlo:urzadzenie:kanal).
    pub id: String,
    /// Nazwa do wyswietlenia (np. "CPU_FAN", "Pompa", "Wentylator 1").
    pub name: String,
    pub kind: FanKind,
    pub rpm: Option<f32>,
    /// Wypelnienie PWM 0..100, jesli zrodlo je podaje.
    pub duty_pct: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FanSnapshot {
    pub fans: Vec<FanReading>,
    /// Temperatura cieczy AIO (C), jesli jest AIO.
    pub liquid_temp_c: Option<f32>,
}

pub struct FanMonitor {}

impl FanMonitor {
    /// Wykrywa zrodla; brak ktoregos to nie blad krytyczny - opis trafia do listy problemow.
    pub fn new() -> (Self, Vec<(&'static str, String)>) {
        (Self {}, Vec::new())
    }

    pub fn read(&mut self) -> FanSnapshot {
        FanSnapshot::default()
    }
}
