//! Wentylatory plyty glownej i chlodzenia AIO.
//!
//! Wspolny model odczytow dla karty "Wentylatory". Zrodla:
//! - `superio` - uklad Super I/O plyty glownej (Nuvoton NCT67xx/NCT668x, ITE IT86xx/IT87xx)
//!   przez PawnIO + LpcIO (wymaga admina),
//! - `aio` - AIO przez USB HID (NZXT Kraken, Corsair Commander Core/ST).
//!
//! Tylko odczyt - program nigdy nie zmienia predkosci wentylatorow ani pompy.
//!
//! Kontrakt dla reszty programu (NIE zmieniac bez aktualizacji `sensors/mod.rs` i `widgets.rs`):
//! `FanMonitor::new()` -> (monitor, lista problemow), `FanMonitor::read()` -> `FanSnapshot`.
//! Kazdy wentylator ma stabilne `id` (np. `superio:nct6798:fan2`, `aio:kraken-x3:pump`),
//! bo konfiguracja karty zapamietuje, ktore wentylatory pokazac.

mod aio;
mod superio;

/// Rodzaj wentylatora - wplywa na ikone i kolejnosc na karcie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanKind {
    /// Zlacze wentylatora na plycie glownej (CPU_FAN, SYS_FAN...).
    Board,
    /// Pompa AIO.
    Pump,
    /// Wentylator podpiety do AIO / kontrolera.
    AioFan,
}

impl FanKind {
    /// Kolejnosc na liscie: pompy, wentylatory AIO, wentylatory plyty.
    fn order(self) -> u8 {
        match self {
            FanKind::Pump => 0,
            FanKind::AioFan => 1,
            FanKind::Board => 2,
        }
    }
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

pub struct FanMonitor {
    board: Option<superio::SuperIoMonitor>,
    aio: Option<aio::AioMonitor>,
}

impl FanMonitor {
    /// Wykrywa zrodla; brak ktoregos to nie blad krytyczny - opis trafia do listy problemow.
    pub fn new() -> (Self, Vec<(&'static str, String)>) {
        let mut issues = Vec::new();
        let (board, board_issues) = superio::SuperIoMonitor::new();
        issues.extend(board_issues.into_iter().map(|issue| ("Wentylatory płyty głównej", issue)));
        let (aio, aio_issues) = aio::AioMonitor::new();
        issues.extend(aio_issues.into_iter().map(|issue| ("Chłodzenie AIO", issue)));
        (Self { board, aio }, issues)
    }

    pub fn read(&mut self) -> FanSnapshot {
        let mut snapshot = FanSnapshot::default();
        if let Some(aio) = self.aio.as_mut() {
            let (fans, liquid) = aio.read();
            snapshot.fans.extend(fans);
            snapshot.liquid_temp_c = liquid;
        }
        if let Some(board) = self.board.as_mut() {
            snapshot.fans.extend(board.read());
        }
        sort_fans(&mut snapshot.fans);
        snapshot
    }
}

/// Sortowanie stabilne - w obrebie rodzaju zostaje kolejnosc zrodla (kanaly rosnaco).
fn sort_fans(fans: &mut [FanReading]) {
    fans.sort_by_key(|f| f.kind.order());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fan(id: &str, kind: FanKind) -> FanReading {
        FanReading { id: id.to_owned(), name: id.to_owned(), kind, rpm: None, duty_pct: None }
    }

    #[test]
    fn pumps_first_then_aio_fans_then_board_by_channel() {
        let mut fans = vec![
            fan("aio:commander-core:fan1", FanKind::AioFan),
            fan("superio:nct6798:fan1", FanKind::Board),
            fan("aio:commander-core:pump", FanKind::Pump),
            fan("superio:nct6798:fan2", FanKind::Board),
            fan("aio:commander-core:fan2", FanKind::AioFan),
        ];
        sort_fans(&mut fans);
        let ids: Vec<_> = fans.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "aio:commander-core:pump",
                "aio:commander-core:fan1",
                "aio:commander-core:fan2",
                "superio:nct6798:fan1",
                "superio:nct6798:fan2"
            ]
        );
    }
}
