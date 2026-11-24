//! Chlodzenia AIO przez USB HID: obroty pompy i wentylatorow oraz temperatura cieczy (TYLKO ODCZYT).
//!
//! Zaleznosc: crate `hidapi` (na Windows backend C hidapi - urzadzenia otwierane wspoldzielone,
//! FILE_SHARE_READ | FILE_SHARE_WRITE, wiec NZXT CAM / iCUE nie sa blokowane).
//!
//! Obslugiwane:
//! - NZXT Kraken X42/X52/X62/X72 (PID 0x170E) - urzadzenie samo wysyla raport stanu 0x04,
//!   nic nie wysylamy. Na podstawie LibreHardwareMonitor `Hardware/Controller/Nzxt/KrakenV2.cs` (MPL-2.0).
//! - NZXT Kraken X53/X63/X73 (0x2007, 0x2014), Z53/Z63/Z73 (0x3008), Kraken 2023 (0x300E),
//!   Kraken 2023 Elite (0x300C), Kraken Elite v2 (0x3012) - zapytanie 0x74 0x01, odpowiedz 0x75 0x01.
//!   Na podstawie LHM `KrakenV3.cs` i `NzxtGroup.cs` (MPL-2.0, commit 9e2d677c); uklad bajtow zgodny
//!   z opisem protokolu liquidctl `docs/developer/protocol/nzxt_x3-z3-2023.md`.
//!   UWAGA: LHM przy starcie ustawia pompe na 60% i wentylatory na 40% - tego celowo NIE robimy.
//! - Corsair Commander Core (0x0C1C), Commander ST (0x0C32) - kontrolery AIO iCUE Elite Capellix /
//!   Elite LCD, oraz Commander Core XT (0x0C2A, bez pompy). LHM ich nie obsluguje; implementacja
//!   wlasna wylacznie na podstawie opisu protokolu liquidctl `docs/developer/protocol/commander_core.md`
//!   (bez kodu liquidctl - GPL). Odczyt wymaga "wybudzenia" urzadzenia i ponownego uspienia (powrot
//!   do trybu sprzetowego), dlatego pomijamy go, gdy dziala iCUE (jego sesja by sie rozjechala).
//!
//! Nieobslugiwane: Corsair Hydro Platinum / Pro XT / Elite RGB (brak otwartej dokumentacji
//! protokolu poza kodem GPL), NZXT Kraken M22 / Kraken 2024 RGB / Plus (brak w LHM).
//!
//! Odczyt nie blokuje watku czujnikow dluzej niz ~45 ms na urzadzenie (limity czasu odczytu HID);
//! przy braku odpowiedzi zostaja ostatnie dobre wartosci, po 3 s bez danych - brak odczytu.

use std::time::{Duration, Instant};

use hidapi::{DeviceInfo, HidApi, HidDevice};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

use super::{FanKind, FanReading};
use crate::i18n::tr;

const NZXT_VID: u16 = 0x1E71;
const CORSAIR_VID: u16 = 0x1B1C;

/// Limit czasu jednego odczytu urzadzenia w petli czujnikow.
const READ_BUDGET: Duration = Duration::from_millis(45);
/// Limit przy wykrywaniu (raz, przy starcie).
const INIT_BUDGET: Duration = Duration::from_millis(300);
/// Po tylu sekundach bez nowych danych wartosci sa traktowane jako nieaktualne.
const STALE_AFTER: Duration = Duration::from_secs(3);
/// Tyle kolejnych bledow wejscia/wyjscia i urzadzenie jest pomijane do restartu programu.
const MAX_IO_ERRORS: u32 = 5;
/// Co ile sprawdzamy, czy dziala iCUE (przy Commander Core).
const ICUE_CHECK_EVERY: Duration = Duration::from_secs(5);
/// Procesy iCUE - gdy dzialaja, nie rozmawiamy z Commander Core.
const ICUE_PROCESSES: [&str; 2] = ["iCUE.exe", "Corsair.Service.exe"];

/// Bufor odczytu: Commander Core z firmware 1.x ma raporty 1024 B (+ ewentualny bajt ID raportu).
const READ_BUFFER_LEN: usize = 1025;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Model {
    /// Kraken X42/X52/X62/X72.
    KrakenX2,
    /// Kraken X53/X63/X73 - bez wentylatorow na kontrolerze.
    KrakenX3,
    KrakenZ3,
    Kraken2023,
    Kraken2023Elite,
    KrakenEliteV2,
    CommanderCore,
    CommanderCoreXt,
    CommanderSt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    KrakenV2,
    KrakenV3 { has_fans: bool },
    Commander { has_pump: bool },
}

/// Model po VID/PID (tabela PID z LHM `NzxtGroup.cs` i z opisu protokolu Commander Core w liquidctl).
pub(crate) fn model_for(vendor_id: u16, product_id: u16) -> Option<Model> {
    use Model::*;
    let model = match (vendor_id, product_id) {
        (NZXT_VID, 0x170E) => KrakenX2,
        (NZXT_VID, 0x2007 | 0x2014) => KrakenX3,
        (NZXT_VID, 0x3008) => KrakenZ3,
        (NZXT_VID, 0x300C) => Kraken2023Elite,
        (NZXT_VID, 0x300E) => Kraken2023,
        (NZXT_VID, 0x3012) => KrakenEliteV2,
        (CORSAIR_VID, 0x0C1C) => CommanderCore,
        (CORSAIR_VID, 0x0C2A) => CommanderCoreXt,
        (CORSAIR_VID, 0x0C32) => CommanderSt,
        _ => return None,
    };
    Some(model)
}

impl Model {
    /// Fragment stabilnego id (`aio:<slug>:pump`) - nie zmieniac, konfiguracja kart go zapamietuje.
    pub(crate) fn slug(self) -> &'static str {
        use Model::*;
        match self {
            KrakenX2 => "kraken-x2",
            KrakenX3 => "kraken-x3",
            KrakenZ3 => "kraken-z3",
            Kraken2023 => "kraken-2023",
            Kraken2023Elite => "kraken-2023-elite",
            KrakenEliteV2 => "kraken-elite-v2",
            CommanderCore => "commander-core",
            CommanderCoreXt => "commander-core-xt",
            CommanderSt => "commander-st",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        use Model::*;
        match self {
            KrakenX2 => "NZXT Kraken X42/X52/X62/X72",
            KrakenX3 => "NZXT Kraken X53/X63/X73",
            KrakenZ3 => "NZXT Kraken Z53/Z63/Z73",
            Kraken2023 => "NZXT Kraken 2023",
            Kraken2023Elite => "NZXT Kraken 2023 Elite",
            KrakenEliteV2 => "NZXT Kraken Elite v2",
            CommanderCore => "Corsair Commander Core",
            CommanderCoreXt => "Corsair Commander Core XT",
            CommanderSt => "Corsair Commander ST",
        }
    }

    fn protocol(self) -> Protocol {
        use Model::*;
        match self {
            KrakenX2 => Protocol::KrakenV2,
            KrakenX3 => Protocol::KrakenV3 { has_fans: false },
            KrakenZ3 | Kraken2023 | Kraken2023Elite | KrakenEliteV2 => Protocol::KrakenV3 { has_fans: true },
            CommanderCore | CommanderSt => Protocol::Commander { has_pump: true },
            CommanderCoreXt => Protocol::Commander { has_pump: false },
        }
    }
}

/// Jeden odczyt stanu AIO.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AioStatus {
    pub liquid_c: Option<f32>,
    pub pump_rpm: Option<f32>,
    pub pump_duty: Option<f32>,
    /// (rpm, wypelnienie) kolejnych kanalow wentylatorow.
    pub fans: Vec<(Option<f32>, Option<f32>)>,
}

fn plausible_liquid(temp: f32) -> Option<f32> {
    (temp > 0.0 && temp < 100.0).then_some(temp)
}

// ---------------------------------------------------------------------------------------------
// Dekodowanie raportow (czyste funkcje - testy jednostkowe)

/// Kraken X3/Z3/2023: odpowiedz 0x75 0x01 na zapytanie 0x74 0x01 (`KrakenV3.Update`).
pub(crate) fn parse_kraken_v3(report: &[u8], has_fans: bool) -> Option<AioStatus> {
    if report.len() < 26 || report[0] != 0x75 || report[1] != 0x01 {
        return None;
    }
    let mut status = AioStatus {
        liquid_c: plausible_liquid(f32::from(report[15]) + f32::from(report[16]) / 10.0),
        pump_rpm: Some(f32::from(u16::from_le_bytes([report[17], report[18]]))),
        pump_duty: Some(f32::from(report[19])),
        fans: Vec::new(),
    };
    if has_fans {
        status.fans.push((Some(f32::from(u16::from_le_bytes([report[23], report[24]]))), Some(f32::from(report[25]))));
    }
    Some(status)
}

/// Kraken X42-X72: raport stanu 0x04 wysylany przez urzadzenie (`KrakenV2.Update`).
pub(crate) fn parse_kraken_v2(report: &[u8]) -> Option<AioStatus> {
    // Raport z zerowa temperatura czasem sie zdarza - LHM go pomija.
    if report.len() < 7 || report[0] != 0x04 || report[1] == 0x00 {
        return None;
    }
    Some(AioStatus {
        liquid_c: plausible_liquid(f32::from(report[1]) + f32::from(report[2]) / 10.0),
        pump_rpm: Some(f32::from(u16::from_be_bytes([report[5], report[6]]))),
        pump_duty: None,
        fans: vec![(Some(f32::from(u16::from_be_bytes([report[3], report[4]]))), None)],
    })
}

// Commander Core: polecenia (bajt 0x08 przed poleceniem dokladany w `commander_transact`).
const CC_WAKE: [u8; 4] = [0x01, 0x03, 0x00, 0x02];
const CC_SLEEP: [u8; 4] = [0x01, 0x03, 0x00, 0x01];
const CC_READ_INITIAL: [u8; 3] = [0x08, 0x00, 0x01];
const CC_CLOSE_ENDPOINT: [u8; 3] = [0x05, 0x01, 0x00];
const CC_OPEN_ENDPOINT: u8 = 0x0d;
/// Tryby (endpointy) i typy danych w odpowiedzi.
const CC_MODE_SPEEDS: (u8, [u8; 2]) = (0x17, [0x06, 0x00]);
const CC_MODE_CONNECTED: (u8, [u8; 2]) = (0x1a, [0x09, 0x00]);
const CC_MODE_TEMPERATURES: (u8, [u8; 2]) = (0x21, [0x10, 0x00]);
const CC_CONNECTED: u8 = 0x07;

/// Dane z odpowiedzi "Read Initial": [0x00, 0x08, 0x00, typ (2 B), dane...].
pub(crate) fn commander_read_payload(response: &[u8], data_type: [u8; 2]) -> Option<&[u8]> {
    (response.len() >= 6 && response[..3] == [0x00, 0x08, 0x00] && response[3..5] == data_type).then(|| &response[5..])
}

/// Predkosci: [liczba, u16 LE...].
pub(crate) fn commander_parse_speeds(data: &[u8]) -> Option<Vec<u16>> {
    let count = usize::from(*data.first()?);
    if count == 0 || count > 8 || data.len() < 1 + 2 * count {
        return None;
    }
    let (pairs, _) = data[1..1 + 2 * count].as_chunks::<2>();
    Some(pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect())
}

/// Temperatury: [liczba, (stan: 0 = podlaczony, u16 LE w 0.1 C)...].
pub(crate) fn commander_parse_temperatures(data: &[u8]) -> Option<Vec<Option<f32>>> {
    let count = usize::from(*data.first()?);
    if count == 0 || count > 8 || data.len() < 1 + 3 * count {
        return None;
    }
    let (sensors, _) = data[1..1 + 3 * count].as_chunks::<3>();
    Some(
        sensors
            .iter()
            .map(|&[state, lo, hi]| (state == 0x00).then(|| f32::from(u16::from_le_bytes([lo, hi])) / 10.0))
            .collect(),
    )
}

/// Podlaczone urzadzenia: [liczba, stan (0x07 = podlaczony)...].
pub(crate) fn commander_parse_connected(data: &[u8]) -> Option<Vec<bool>> {
    let count = usize::from(*data.first()?);
    if count == 0 || count > 8 || data.len() < 1 + count {
        return None;
    }
    Some(data[1..1 + count].iter().map(|&s| s == CC_CONNECTED).collect())
}

/// Podzial listy predkosci na pompe i wentylatory (Core/ST: pierwsza pozycja to port AIO).
pub(crate) fn commander_status(speeds: &[u16], temperatures: Option<&[Option<f32>]>, has_pump: bool) -> AioStatus {
    let (pump, fans) = if has_pump { (speeds.first(), speeds.get(1..).unwrap_or(&[])) } else { (None, speeds) };
    AioStatus {
        // Pierwszy czujnik temperatury w Core/ST to czujnik cieczy na porcie AIO.
        liquid_c: if has_pump {
            temperatures.and_then(|t| t.first().copied().flatten()).and_then(plausible_liquid)
        } else {
            None
        },
        pump_rpm: pump.map(|&rpm| f32::from(rpm)),
        pump_duty: None,
        fans: fans.iter().map(|&rpm| (Some(f32::from(rpm)), None)).collect(),
    }
}

// ---------------------------------------------------------------------------------------------
// Komunikacja

fn remaining_ms(deadline: Instant) -> i32 {
    let left = deadline.saturating_duration_since(Instant::now()).as_millis();
    left.min(1000) as i32
}

/// Zbiera raporty, ktore juz czekaja w buforze (bez czekania).
fn drain(dev: &HidDevice, buf: &mut [u8], mut on_report: impl FnMut(&[u8])) -> Result<(), String> {
    // Windows trzyma do ~32 raportow na uchwyt - limit tylko na wszelki wypadek.
    for _ in 0..64 {
        let n = dev.read_timeout(buf, 0).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        on_report(&buf[..n]);
    }
    Ok(())
}

/// Kraken X3/Z3/2023: zapytanie o stan i czekanie na odpowiedz do `deadline`.
fn poll_kraken_v3(dev: &HidDevice, has_fans: bool, deadline: Instant) -> Result<Option<AioStatus>, String> {
    let mut buf = [0u8; 64];
    let mut latest = None;
    // Odpowiedzi spoznione z poprzedniego cyklu (albo na zapytania NZXT CAM) tez sa dobre.
    drain(dev, &mut buf, |r| {
        if let Some(s) = parse_kraken_v3(r, has_fans) {
            latest = Some(s);
        }
    })?;
    dev.write(&[0x74, 0x01]).map_err(|e| e.to_string())?;
    loop {
        let wait = remaining_ms(deadline);
        if wait <= 0 {
            return Ok(latest);
        }
        let n = dev.read_timeout(&mut buf, wait).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(latest);
        }
        if let Some(s) = parse_kraken_v3(&buf[..n], has_fans) {
            return Ok(Some(s));
        }
    }
}

/// Kraken X42-X72: urzadzenie samo wysyla raporty stanu - bierzemy najnowszy.
fn poll_kraken_v2(dev: &HidDevice, deadline: Instant) -> Result<Option<AioStatus>, String> {
    let mut buf = [0u8; 64];
    let mut latest = None;
    drain(dev, &mut buf, |r| {
        if let Some(s) = parse_kraken_v2(r) {
            latest = Some(s);
        }
    })?;
    while latest.is_none() {
        let wait = remaining_ms(deadline);
        if wait <= 0 {
            break;
        }
        let n = dev.read_timeout(&mut buf, wait).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        latest = parse_kraken_v2(&buf[..n]);
    }
    Ok(latest)
}

/// Jedno polecenie Commander Core: zapis [ID raportu 0x00, 0x08, polecenie...] i odpowiedz
/// [0x00, polecenie, 0x00 (status), dane...].
fn commander_transact(dev: &HidDevice, command: &[u8], deadline: Instant) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; READ_BUFFER_LEN];
    // Stare odpowiedzi (np. po przekroczeniu czasu w poprzednim cyklu) nie moga sie pomylic z nowymi.
    drain(dev, &mut buf, |_| {})?;
    let mut packet = Vec::with_capacity(command.len() + 2);
    packet.extend_from_slice(&[0x00, 0x08]);
    packet.extend_from_slice(command);
    dev.write(&packet).map_err(|e| e.to_string())?;
    loop {
        let wait = remaining_ms(deadline);
        if wait <= 0 {
            return Err(format!("{} 0x{:02X}", tr("brak odpowiedzi na polecenie", "no response to command"), command[0]));
        }
        let n = dev.read_timeout(&mut buf, wait).map_err(|e| e.to_string())?;
        if n == 0 {
            continue;
        }
        let response = &buf[..n];
        if n >= 3 && response[0] == 0x00 && response[1] == command[0] {
            if response[2] != 0x00 {
                return Err(format!(
                    "{} 0x{:02X} {} 0x{:02X})",
                    tr("polecenie", "command"),
                    command[0],
                    tr("odrzucone (status", "rejected (status"),
                    response[2]
                ));
            }
            return Ok(response.to_vec());
        }
    }
}

/// Otwarcie endpointu w danym trybie, odczyt poczatkowy i zamkniecie (opis protokolu Commander Core).
fn commander_read_mode(dev: &HidDevice, mode: (u8, [u8; 2]), deadline: Instant) -> Result<Vec<u8>, String> {
    commander_transact(dev, &[CC_OPEN_ENDPOINT, 0x00, mode.0], deadline)?;
    let response = commander_transact(dev, &CC_READ_INITIAL, deadline);
    // Zamkniecie endpointu takze po nieudanym odczycie.
    let closed = commander_transact(dev, &CC_CLOSE_ENDPOINT, deadline);
    let response = response?;
    closed?;
    commander_read_payload(&response, mode.1)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| {
            format!("{} 0x{:02X}", tr("nieoczekiwana odpowiedz w trybie", "unexpected response in mode"), mode.0)
        })
}

/// Pelny odczyt Commander Core: wybudzenie, predkosci (+ temperatury), uspienie (powrot do trybu sprzetowego).
fn poll_commander(dev: &HidDevice, has_pump: bool, deadline: Instant) -> Result<AioStatus, String> {
    commander_transact(dev, &CC_WAKE, deadline)?;
    let result = (|| {
        let speeds = commander_read_mode(dev, CC_MODE_SPEEDS, deadline)?;
        let speeds = commander_parse_speeds(&speeds).ok_or(tr("niepoprawna lista predkosci", "invalid speed list"))?;
        let temperatures = if has_pump {
            let data = commander_read_mode(dev, CC_MODE_TEMPERATURES, deadline)?;
            commander_parse_temperatures(&data)
        } else {
            None
        };
        Ok::<_, String>(commander_status(&speeds, temperatures.as_deref(), has_pump))
    })();
    // Uspienie zawsze (z malym zapasem czasu), zeby urzadzenie wrocilo do swoich krzywych sprzetowych.
    let slept = commander_transact(dev, &CC_SLEEP, deadline.max(Instant::now() + Duration::from_millis(15)));
    let status = result?;
    slept?;
    Ok(status)
}

/// Ktore porty wentylatorow sa podlaczone (Core/ST: pozycja 0 to port AIO).
fn commander_connected_fans(dev: &HidDevice, has_pump: bool, deadline: Instant) -> Result<Vec<usize>, String> {
    commander_transact(dev, &CC_WAKE, deadline)?;
    let data = commander_read_mode(dev, CC_MODE_CONNECTED, deadline);
    let slept = commander_transact(dev, &CC_SLEEP, deadline.max(Instant::now() + Duration::from_millis(15)));
    let data = data?;
    slept?;
    let connected = commander_parse_connected(&data)
        .ok_or(tr("niepoprawna lista podlaczonych urzadzen", "invalid connected device list"))?;
    let skip = usize::from(has_pump);
    Ok(connected.iter().skip(skip).enumerate().filter(|(_, c)| **c).map(|(i, _)| i).collect())
}

/// Czy dziala ktorys z procesow (porownanie nazw bez wielkosci liter).
fn is_process_running(names: &[&str]) -> bool {
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if names.iter().any(|n| n.eq_ignore_ascii_case(&exe)) {
                found = true;
                break;
            }
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
        found
    }
}

// ---------------------------------------------------------------------------------------------

struct AioDevice {
    dev: HidDevice,
    model: Model,
    /// Np. `aio:kraken-x3` (z sufiksem `-2` dla drugiego takiego samego urzadzenia).
    id_prefix: String,
    /// Sufiks nazwy, gdy jest kilka AIO.
    name_suffix: String,
    /// Kanaly wentylatorow do pokazania (indeksy w `AioStatus::fans`).
    fan_channels: Vec<usize>,
    status: AioStatus,
    updated: Option<Instant>,
    io_errors: u32,
    disabled: bool,
    error_logged: bool,
    icue_checked: Option<Instant>,
    icue_running: bool,
}

impl AioDevice {
    fn poll(&mut self, deadline: Instant) -> Result<Option<AioStatus>, String> {
        match self.model.protocol() {
            Protocol::KrakenV2 => poll_kraken_v2(&self.dev, deadline),
            Protocol::KrakenV3 { has_fans } => poll_kraken_v3(&self.dev, has_fans, deadline),
            Protocol::Commander { has_pump } => {
                let due = self.icue_checked.is_none_or(|t| t.elapsed() >= ICUE_CHECK_EVERY);
                if due {
                    self.icue_checked = Some(Instant::now());
                    self.icue_running = is_process_running(&ICUE_PROCESSES);
                }
                if self.icue_running {
                    return Ok(None);
                }
                poll_commander(&self.dev, has_pump, deadline).map(Some)
            }
        }
    }

    fn readings(&self, out: &mut Vec<FanReading>) {
        let fresh = self.updated.is_some_and(|t| t.elapsed() < STALE_AFTER);
        let value = |v: Option<f32>| if fresh { v } else { None };
        if matches!(
            self.model.protocol(),
            Protocol::KrakenV2 | Protocol::KrakenV3 { .. } | Protocol::Commander { has_pump: true }
        ) {
            out.push(FanReading {
                id: format!("{}:pump", self.id_prefix),
                name: format!("{}{}", tr("Pompa", "Pump"), self.name_suffix),
                kind: FanKind::Pump,
                rpm: value(self.status.pump_rpm),
                duty_pct: value(self.status.pump_duty),
            });
        }
        let aggregated = !matches!(self.model.protocol(), Protocol::Commander { .. });
        for &channel in &self.fan_channels {
            let (rpm, duty) = self.status.fans.get(channel).copied().unwrap_or((None, None));
            let (id, name) = if aggregated {
                // Kraken podaje jeden wspolny odczyt dla wszystkich wentylatorow.
                let name = format!("{}{}", tr("Wentylatory AIO", "AIO fans"), self.name_suffix);
                (format!("{}:fans", self.id_prefix), name)
            } else {
                (
                    format!("{}:fan{}", self.id_prefix, channel + 1),
                    format!("{} {}{}", tr("Wentylator AIO", "AIO fan"), channel + 1, self.name_suffix),
                )
            };
            out.push(FanReading { id, name, kind: FanKind::AioFan, rpm: value(rpm), duty_pct: value(duty) });
        }
    }
}

/// Wybiera po jednym interfejsie HID na urzadzenie (preferowany interfejs producenta, usage page >= 0xFF00).
fn pick_devices(api: &HidApi) -> Vec<(&DeviceInfo, Model)> {
    let mut picked: Vec<(&DeviceInfo, Model)> = Vec::new();
    for info in api.device_list() {
        let Some(model) = model_for(info.vendor_id(), info.product_id()) else {
            continue;
        };
        // Ten sam model i numer seryjny = kolejny interfejs/kolekcja tego samego urzadzenia.
        let same = picked.iter().position(|(p, m)| *m == model && p.serial_number() == info.serial_number());
        let vendor_page = |i: &DeviceInfo| i.usage_page() >= 0xFF00;
        match same {
            Some(idx) => {
                let current = picked[idx].0;
                let better = (vendor_page(info) && !vendor_page(current))
                    || (vendor_page(info) == vendor_page(current)
                        && info.interface_number() < current.interface_number());
                if better {
                    picked[idx].0 = info;
                }
            }
            None => picked.push((info, model)),
        }
    }
    picked
}

/// Wszystkie wykryte AIO.
pub struct AioMonitor {
    devices: Vec<AioDevice>,
    // Po `devices` - kontekst hidapi zwalniany na koncu.
    _api: HidApi,
}

impl AioMonitor {
    pub fn new() -> (Option<Self>, Vec<String>) {
        let api = match HidApi::new() {
            Ok(api) => api,
            Err(err) => {
                let issue = tr("Nie można odczytać listy urządzeń USB HID:", "Can't read the USB HID device list:");
                return (None, vec![format!("{issue} {err}")]);
            }
        };
        let mut issues = Vec::new();
        let mut devices: Vec<AioDevice> = Vec::new();
        let candidates = pick_devices(&api);
        let has_commander = candidates.iter().any(|(_, m)| matches!(m.protocol(), Protocol::Commander { .. }));
        let icue_running = has_commander && is_process_running(&ICUE_PROCESSES);

        for (info, model) in candidates {
            let name = model.display_name();
            if matches!(model.protocol(), Protocol::Commander { .. }) && icue_running {
                issues.push(format!(
                    "{name}: {}",
                    tr(
                        "działa Corsair iCUE - odczyt pominięty, żeby nie zakłócać jego pracy (zamknij iCUE i uruchom program ponownie)",
                        "Corsair iCUE is running - reading skipped to avoid interfering with it (close iCUE and restart the app)",
                    )
                ));
                continue;
            }
            let dev = match api.open_path(info.path()) {
                Ok(dev) => dev,
                Err(err) => {
                    issues.push(format!(
                        "{name}: {} {err}",
                        tr(
                            "nie można otworzyć urządzenia (zajęte przez inny program?):",
                            "can't open the device (in use by another program?):",
                        )
                    ));
                    continue;
                }
            };
            let duplicates = devices.iter().filter(|d| d.model == model).count();
            let id_prefix = if duplicates == 0 {
                format!("aio:{}", model.slug())
            } else {
                format!("aio:{}-{}", model.slug(), duplicates + 1)
            };
            let mut device = AioDevice {
                dev,
                model,
                id_prefix,
                name_suffix: String::new(),
                fan_channels: Vec::new(),
                status: AioStatus::default(),
                updated: None,
                io_errors: 0,
                disabled: false,
                error_logged: false,
                icue_checked: Some(Instant::now()),
                icue_running: false,
            };
            match device.init() {
                Ok(()) => {
                    log::info!("AIO: {name} ({}), kanały wentylatorów: {:?}", device.id_prefix, device.fan_channels);
                    devices.push(device);
                }
                Err(err) => issues.push(format!("{name}: {err}")),
            }
        }

        if devices.len() > 1 {
            for (i, device) in devices.iter_mut().enumerate() {
                device.name_suffix = format!(" ({} #{})", device.model.display_name(), i + 1);
            }
        }
        if devices.is_empty() {
            if issues.is_empty() {
                issues.push(
                    tr(
                        "Nie wykryto obsługiwanego AIO (NZXT Kraken X/Z/2023/Elite, Corsair Commander Core/ST)",
                        "No supported AIO detected (NZXT Kraken X/Z/2023/Elite, Corsair Commander Core/ST)",
                    )
                    .to_owned(),
                );
            }
            return (None, issues);
        }
        (Some(Self { devices, _api: api }), issues)
    }

    /// Odczyt wszystkich AIO: (pompy i wentylatory, temperatura cieczy pierwszego AIO, ktore ja podaje).
    pub fn read(&mut self) -> (Vec<FanReading>, Option<f32>) {
        let mut readings = Vec::new();
        let mut liquid = None;
        for device in &mut self.devices {
            if !device.disabled {
                let deadline = Instant::now() + READ_BUDGET;
                match device.poll(deadline) {
                    Ok(Some(status)) => {
                        device.status = status;
                        device.updated = Some(Instant::now());
                        device.io_errors = 0;
                        device.error_logged = false;
                    }
                    // Brak odpowiedzi w limicie czasu - zostaja ostatnie wartosci.
                    Ok(None) => {}
                    Err(err) => {
                        device.io_errors += 1;
                        if !device.error_logged {
                            log::warn!("AIO {}: odczyt nieudany: {err}", device.model.display_name());
                            device.error_logged = true;
                        }
                        if device.io_errors >= MAX_IO_ERRORS {
                            log::warn!("AIO {}: zbyt wiele błędów - urządzenie pominięte", device.model.display_name());
                            device.disabled = true;
                        }
                    }
                }
            }
            device.readings(&mut readings);
            if liquid.is_none() && device.updated.is_some_and(|t| t.elapsed() < STALE_AFTER) {
                liquid = device.status.liquid_c;
            }
        }
        (readings, liquid)
    }
}

impl AioDevice {
    /// Pierwszy odczyt przy starcie - urzadzenie, ktore nie odpowiada, jest pomijane.
    fn init(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + INIT_BUDGET;
        match self.model.protocol() {
            Protocol::KrakenV2 => {
                // Urzadzenie nadaje samo; brak raportu w limicie to jeszcze nie blad.
                if let Some(status) = poll_kraken_v2(&self.dev, deadline)? {
                    self.status = status;
                    self.updated = Some(Instant::now());
                }
                self.fan_channels = vec![0];
            }
            Protocol::KrakenV3 { has_fans } => {
                let status = poll_kraken_v3(&self.dev, has_fans, deadline)?
                    .ok_or(tr(
                        "brak odpowiedzi (czy działa NZXT CAM? zamknij go i uruchom program ponownie)",
                        "no response (is NZXT CAM running? close it and restart the app)",
                    ))?;
                self.status = status;
                self.updated = Some(Instant::now());
                self.fan_channels = if has_fans { vec![0] } else { Vec::new() };
            }
            Protocol::Commander { has_pump } => {
                let status = poll_commander(&self.dev, has_pump, deadline)
                    .map_err(|e| format!("{} {e}", tr("brak poprawnej odpowiedzi:", "no valid response:")))?;
                let all: Vec<usize> = (0..status.fans.len()).collect();
                self.fan_channels = commander_connected_fans(&self.dev, has_pump, Instant::now() + INIT_BUDGET)
                    .map(|c| c.into_iter().filter(|i| *i < status.fans.len()).collect())
                    .unwrap_or_else(|err| {
                        log::warn!(
                            "AIO {}: lista podłączonych wentylatorów niedostępna ({err})",
                            self.model.display_name()
                        );
                        all
                    });
                self.status = status;
                self.updated = Some(Instant::now());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raport 0x75 0x01 zbudowany wg ukladu bajtow z LHM `KrakenV3.Update` / opisu protokolu.
    fn kraken_v3_report() -> [u8; 64] {
        let mut r = [0u8; 64];
        r[0] = 0x75;
        r[1] = 0x01;
        r[15] = 31; // ciecz 31.4 C
        r[16] = 4;
        r[17..19].copy_from_slice(&2100u16.to_le_bytes());
        r[19] = 70;
        r[23..25].copy_from_slice(&1250u16.to_le_bytes());
        r[25] = 45;
        r
    }

    #[test]
    fn kraken_v3_status_decoding() {
        let report = kraken_v3_report();
        let s = parse_kraken_v3(&report, true).unwrap();
        assert!((s.liquid_c.unwrap() - 31.4).abs() < 1e-4);
        assert_eq!((s.pump_rpm, s.pump_duty), (Some(2100.0), Some(70.0)));
        assert_eq!(s.fans, vec![(Some(1250.0), Some(45.0))]);
        // X53/X63/X73 bez wentylatorow.
        assert!(parse_kraken_v3(&report, false).unwrap().fans.is_empty());
        // Inny raport (np. odpowiedz na zapytanie o firmware 0x11 0x01) i za krotki raport.
        let mut other = report;
        other[0] = 0x11;
        assert_eq!(parse_kraken_v3(&other, true), None);
        assert_eq!(parse_kraken_v3(&report[..20], true), None);
    }

    #[test]
    fn kraken_v2_status_decoding() {
        // [0x04, temp, temp/10, fan BE, pump BE, ...]
        let report = [0x04, 29, 7, 0x03, 0x84, 0x0A, 0x28, 0, 0, 0];
        let s = parse_kraken_v2(&report).unwrap();
        assert!((s.liquid_c.unwrap() - 29.7).abs() < 1e-4);
        assert_eq!(s.pump_rpm, Some(2600.0));
        assert_eq!(s.fans, vec![(Some(900.0), None)]);
        assert_eq!(parse_kraken_v2(&[0x04, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(parse_kraken_v2(&[0x05, 29, 7, 0, 0, 0, 0]), None);
    }

    #[test]
    fn commander_core_decoding() {
        // Odpowiedz na odczyt predkosci: naglowek, typ 06 00, 7 pozycji (pompa + 6 wentylatorow).
        let mut speeds = vec![0x00, 0x08, 0x00, 0x06, 0x00, 7];
        for rpm in [2356u16, 810, 791, 0, 0, 0, 0] {
            speeds.extend_from_slice(&rpm.to_le_bytes());
        }
        let data = commander_read_payload(&speeds, CC_MODE_SPEEDS.1).unwrap();
        let parsed = commander_parse_speeds(data).unwrap();
        assert_eq!(parsed, vec![2356, 810, 791, 0, 0, 0, 0]);
        // Zly typ danych.
        assert_eq!(commander_read_payload(&speeds, CC_MODE_TEMPERATURES.1), None);

        // Temperatury: 3b:01 = 315 = 31.5 C (przyklad z opisu protokolu), drugi czujnik odlaczony.
        let temps = [0x00, 0x08, 0x00, 0x10, 0x00, 2, 0x00, 0x3b, 0x01, 0x01, 0x00, 0x00];
        let data = commander_read_payload(&temps, CC_MODE_TEMPERATURES.1).unwrap();
        let parsed_temps = commander_parse_temperatures(data).unwrap();
        assert_eq!(parsed_temps, vec![Some(31.5), None]);

        let status = commander_status(&parsed, Some(&parsed_temps), true);
        assert_eq!(status.pump_rpm, Some(2356.0));
        assert_eq!(status.fans.len(), 6);
        assert_eq!(status.fans[0], (Some(810.0), None));
        assert_eq!(status.liquid_c, Some(31.5));
        // Core XT: bez pompy i bez temperatury cieczy.
        let xt = commander_status(&parsed[..6], Some(&parsed_temps), false);
        assert_eq!((xt.pump_rpm, xt.liquid_c, xt.fans.len()), (None, None, 6));

        let connected = [0x00, 0x08, 0x00, 0x09, 0x00, 7, 0x07, 0x07, 0x07, 0x01, 0x01, 0x01, 0x01];
        let data = commander_read_payload(&connected, CC_MODE_CONNECTED.1).unwrap();
        assert_eq!(commander_parse_connected(data).unwrap(), vec![true, true, true, false, false, false, false]);
        // Za krotkie dane.
        assert_eq!(commander_parse_speeds(&[7, 0x10]), None);
    }

    #[test]
    fn model_table() {
        assert_eq!(model_for(0x1E71, 0x2007), Some(Model::KrakenX3));
        assert_eq!(model_for(0x1E71, 0x2014), Some(Model::KrakenX3));
        assert_eq!(model_for(0x1E71, 0x3008), Some(Model::KrakenZ3));
        assert_eq!(model_for(0x1E71, 0x170E), Some(Model::KrakenX2));
        assert_eq!(model_for(0x1B1C, 0x0C1C), Some(Model::CommanderCore));
        assert_eq!(model_for(0x1B1C, 0x0C2A), Some(Model::CommanderCoreXt));
        // Grid V3 (0x1711) to nie AIO.
        assert_eq!(model_for(0x1E71, 0x1711), None);
        assert_eq!(model_for(0x1234, 0x2007), None);
    }
}
