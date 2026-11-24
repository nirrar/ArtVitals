//! Konfiguracja overlaya zapisywana w `%APPDATA%\ArtVitals\config.toml`.
//!
//! Plik jest jedynym zrodlem prawdy: okno konfiguracji zapisuje do niego zmiany,
//! a reczna edycja pliku jest przeladowywana na zywo (patrz `overlay::watcher`).
//! Po wczytaniu wartosci sa walidowane (`sanitize`), wiec bledny plik nie wywroci aplikacji.
//! Zaleznosci: serde, toml.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::{LanguageSetting, tr};
use crate::metrics::Metric;

pub const APP_DIR_NAME: &str = "ArtVitals";
/// Poprzednia nazwa programu - konfiguracja jest stamtad przenoszona przy pierwszym starcie.
const LEGACY_APP_DIR_NAME: &str = "CraftGamingBoard";
pub const CONFIG_FILE_NAME: &str = "config.toml";

pub const MIN_WIDGET_SIZE: [f32; 2] = [90.0, 60.0];
/// Ile dodatkowych metryk mozna dolozyc do jednej karty.
pub const MAX_EXTRA_METRICS: usize = 4;
/// O tyle rosnie karta "Pasek" na kazda dodatkowa metryke (wiersz z dodatkowym paskiem).
pub const BAR_EXTRA_ROW_HEIGHT: f32 = 16.0;
/// O tyle rosnie karta "Wykres" na kazdy osobny wykres dodatkowej metryki
/// (pasek z etykieta i wartoscia + sam wykres + odstep).
pub const SEPARATE_CHART_HEIGHT: f32 = 40.0;
/// Wysokosc wiersza siatki wartosci pod naglowkiem karty "Wykres".
pub const VALUE_ROW_HEIGHT: f32 = 15.0;
/// Tyle dodatkowych wartosci miesci sie w prawej kolumnie naglowka; wiecej = siatka pod naglowkiem.
pub const HEADER_EXTRAS_MAX: usize = 2;
const MAX_WIDGET_SIZE: f32 = 2000.0;
/// Domyslne granatowe tlo karty.
pub const DEFAULT_CARD_BACKGROUND: [u8; 3] = [12, 22, 38];
/// Maksymalne powiekszenie tekstu kart (w punktach wzgledem tytulu karty).
pub const MAX_FONT_BOOST: u32 = 10;
/// Rozmiar tytulu karty przy skali 1 - od niego liczone jest powiekszenie "+N pt".
const BASE_TEXT_PT: f32 = 11.0;
/// Wysokosc paska z etykieta nad osobnym wykresem (czesc `SEPARATE_CHART_HEIGHT` zajeta tekstem).
pub const SEPARATE_CHART_STRIP: f32 = 12.0;

/// Biezaca skala tekstu kart (bity f32). Globalna, bo rozmiary kart liczone sa w wielu miejscach
/// bez dostepu do `Config`; ustawiana przez `Config::sanitize` i `Config::set_font_boost`.
static TEXT_SCALE_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);

/// Mnoznik czcionek kart wynikajacy z `font_boost`.
pub fn text_scale() -> f32 {
    f32::from_bits(TEXT_SCALE_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn text_scale_for(boost: u32) -> f32 {
    (BASE_TEXT_PT + boost.min(MAX_FONT_BOOST) as f32) / BASE_TEXT_PT
}

fn set_text_scale(scale: f32) {
    // W testach (rownoleglych) globalna skala zostaje 1.0, zeby testy nie wplywaly na siebie.
    if !cfg!(test) {
        TEXT_SCALE_BITS.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    /// Wykres liniowy z wypelnieniem i historia.
    Graph,
    /// Okragly wskaznik (luk).
    Gauge,
    /// Poziomy pasek.
    Bar,
    /// Duza liczba z mini-wykresem pod spodem.
    Number,
    /// Lista wentylatorow i AIO z animowanymi wirnikami (tylko dla metryki `FanSpeed`).
    Fans,
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 5] = [
        WidgetKind::Graph,
        WidgetKind::Gauge,
        WidgetKind::Bar,
        WidgetKind::Number,
        WidgetKind::Fans,
    ];

    /// Czy typ karty pasuje do metryki: wentylatory maja wlasny typ i nie pasuja do innych.
    pub fn fits(self, metric: Metric) -> bool {
        (self == WidgetKind::Fans) == (metric == Metric::FanSpeed)
    }

    /// Typy kart dostepne dla metryki.
    pub fn for_metric(metric: Metric) -> Vec<WidgetKind> {
        Self::ALL.into_iter().filter(|k| k.fits(metric)).collect()
    }

    pub fn label(self) -> &'static str {
        match self {
            WidgetKind::Graph => tr("Wykres", "Graph"),
            WidgetKind::Gauge => tr("Wskaźnik", "Gauge"),
            WidgetKind::Bar => tr("Pasek", "Bar"),
            WidgetKind::Number => tr("Liczba", "Number"),
            WidgetKind::Fans => tr("Wentylatory", "Fans"),
        }
    }

    /// Domyslny rozmiar przy biezacej skali tekstu.
    pub fn default_size(self) -> [f32; 2] {
        self.default_size_at(text_scale())
    }

    /// Domyslny rozmiar przy skali tekstu `ts`. Kazdy wymiar = czesc stala (marginesy, wykres)
    /// plus czesc zajeta tekstem, ktora rosnie z czcionka. Przy `ts` = 1:
    /// Wykres 200x92, Wskaznik 112x112, Pasek 200x80, Liczba 132x92.
    pub fn default_size_at(self, ts: f32) -> [f32; 2] {
        // (stala szer., tekst szer., stala wys., tekst wys.)
        let (fw, tw, fh, th) = match self {
            // Naglowek 33 + stopka 12 to tekst; marginesy, odstepy i wykres stale.
            WidgetKind::Graph => (16.0, 184.0, 47.0, 45.0),
            // Wartosc wpisana w luk - caly wskaznik rosnie z tekstem.
            WidgetKind::Gauge => (16.0, 96.0, 16.0, 96.0),
            // Naglowek (tytul + wartosc) + pasek + stopka musza sie zmiescic bez nakladania.
            WidgetKind::Bar => (16.0, 184.0, 35.0, 45.0),
            WidgetKind::Number => (16.0, 116.0, 16.0, 76.0),
            // Naglowek + kilka wierszy wentylatorow (karta dopasowuje wiersze do wysokosci).
            WidgetKind::Fans => (16.0, 184.0, 16.0, 104.0),
        };
        [fw + tw * ts, fh + th * ts]
    }
}

/// Ikona Phosphor w naglowku karty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetIcon {
    None,
    Cpu,
    Gpu,
    Memory,
    Thermometer,
    Fire,
    Gauge,
    Lightning,
    Fan,
    GameController,
    Timer,
}

/// Jak pokazac dodatkowa metryke na karcie typu "Wykres".
/// Na pozostalych typach kart dodatki maja jeden styl (paski, pierscienie, kolumna).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtraChart {
    /// Sama wartosc z etykieta obok glownej liczby.
    Value,
    /// Dodatkowa linia na wykresie glownej metryki (w swojej skali).
    #[default]
    Line,
    /// Osobny maly wykres pod glownym.
    Separate,
}

impl ExtraChart {
    pub const ALL: [ExtraChart; 3] = [ExtraChart::Line, ExtraChart::Separate, ExtraChart::Value];

    pub fn label(self) -> &'static str {
        match self {
            ExtraChart::Value => tr("Tylko wartość", "Value only"),
            ExtraChart::Line => tr("Na wykresie głównym", "On main graph"),
            ExtraChart::Separate => tr("Osobny wykres", "Separate graph"),
        }
    }
}

/// Dodatkowa metryka karty. W pliku moze byc tez sama nazwa metryki (stary format) -
/// wtedy dostaje domyslny tryb `Line`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ExtraMetricRepr")]
pub struct ExtraMetric {
    pub metric: Metric,
    #[serde(default)]
    pub chart: ExtraChart,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExtraMetricRepr {
    Plain(Metric),
    Full {
        metric: Metric,
        #[serde(default)]
        chart: ExtraChart,
    },
}

impl From<ExtraMetricRepr> for ExtraMetric {
    fn from(repr: ExtraMetricRepr) -> Self {
        match repr {
            ExtraMetricRepr::Plain(metric) => ExtraMetric { metric, chart: ExtraChart::default() },
            ExtraMetricRepr::Full { metric, chart } => ExtraMetric { metric, chart },
        }
    }
}

impl WidgetIcon {
    pub const ALL: [WidgetIcon; 11] = [
        WidgetIcon::None,
        WidgetIcon::Cpu,
        WidgetIcon::Gpu,
        WidgetIcon::Memory,
        WidgetIcon::Thermometer,
        WidgetIcon::Fire,
        WidgetIcon::Gauge,
        WidgetIcon::Lightning,
        WidgetIcon::Fan,
        WidgetIcon::GameController,
        WidgetIcon::Timer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WidgetIcon::None => tr("Brak", "None"),
            WidgetIcon::Cpu => tr("Procesor", "Processor"),
            WidgetIcon::Gpu => tr("Karta graficzna", "Graphics card"),
            WidgetIcon::Memory => tr("Pamięć", "Memory"),
            WidgetIcon::Thermometer => tr("Termometr", "Thermometer"),
            WidgetIcon::Fire => tr("Płomień", "Flame"),
            WidgetIcon::Gauge => tr("Licznik (zegary)", "Gauge (clocks)"),
            WidgetIcon::Lightning => tr("Błyskawica (moc)", "Lightning (power)"),
            WidgetIcon::Fan => tr("Wentylator", "Fan"),
            WidgetIcon::GameController => tr("Pad (FPS)", "Gamepad (FPS)"),
            WidgetIcon::Timer => tr("Stoper (czas klatki)", "Stopwatch (frame time)"),
        }
    }
}

/// Dolny pasek karty z dodatkowa informacja.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetFooter {
    None,
    /// Min / sr. / max metryki glownej z historii.
    Stats,
    /// Pasek zajetosci VRAM z opisem `uzyte / calosc GB`.
    VramBar,
    /// Pasek segmentowy wartosci metryki glownej.
    Segments,
}

impl WidgetFooter {
    pub const ALL: [WidgetFooter; 4] = [
        WidgetFooter::None,
        WidgetFooter::Stats,
        WidgetFooter::VramBar,
        WidgetFooter::Segments,
    ];

    /// Czy stopka pasuje do karty z dana metryka glowna (pasek VRAM tylko na kartach GPU).
    pub fn fits(self, metric: Metric) -> bool {
        self != WidgetFooter::VramBar || metric.device() == crate::metrics::Device::Gpu
    }

    pub fn label(self) -> &'static str {
        match self {
            WidgetFooter::None => tr("Brak", "None"),
            WidgetFooter::Stats => tr("Min / śr. / max", "Min / avg / max"),
            WidgetFooter::VramBar => tr("Pasek VRAM", "VRAM bar"),
            WidgetFooter::Segments => tr("Pasek segmentowy", "Segmented bar"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelTheme {
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetConfig {
    pub id: u64,
    pub metric: Metric,
    /// Dodatkowe metryki z tego samego podzespolu (np. temperatura przy obciazeniu).
    pub extra: Vec<ExtraMetric>,
    /// Stary przelacznik "linie na wykresie" - tylko do odczytu; `false` zamienia sie w tryb `Value`.
    #[serde(rename = "plot_extra", skip_serializing)]
    pub legacy_plot_extra: Option<bool>,
    /// Stary format (jedna "druga wartosc") - tylko do odczytu, migrowany do `extra` w `sanitize`.
    #[serde(rename = "secondary", skip_serializing)]
    pub legacy_secondary: Option<Metric>,
    pub kind: WidgetKind,
    pub icon: WidgetIcon,
    pub footer: WidgetFooter,
    /// Lewy gorny rog w punktach ekranu (logicznych, z uwzglednieniem skalowania DPI).
    pub pos: [f32; 2],
    pub size: [f32; 2],
    /// Przypiety widget nie da sie przesunac ani zmienic jego rozmiaru w oknie konfiguracji.
    pub pinned: bool,
    /// Wlasny tytul; pusty = krotka nazwa metryki.
    pub title: String,
    pub accent: [u8; 3],
    pub warn_threshold: f32,
    pub crit_threshold: f32,
    /// Krycie tla karty 0..1 (0 = tylko sam wykres bez tla).
    pub background_opacity: f32,
    /// Kolor tla karty (RGB); ramka dobierana jest automatycznie.
    pub background_color: [u8; 3],
    pub show_title: bool,
    /// Karta "Wentylatory": identyfikatory ukrytych wentylatorow (`FanReading::id`, `gpu:fan`).
    pub hidden_fans: Vec<String>,
}

impl Default for WidgetConfig {
    fn default() -> Self {
        Self::new(1, Metric::CpuLoad, WidgetKind::Graph, [40.0, 40.0])
    }
}

impl WidgetConfig {
    pub fn new(id: u64, metric: Metric, kind: WidgetKind, pos: [f32; 2]) -> Self {
        let (warn, crit) = metric.default_thresholds();
        Self {
            id,
            metric,
            extra: Vec::new(),
            legacy_plot_extra: None,
            legacy_secondary: None,
            kind,
            icon: default_icon(metric),
            footer: WidgetFooter::None,
            pos,
            size: kind.default_size(),
            pinned: false,
            title: String::new(),
            accent: default_accent(metric),
            warn_threshold: warn,
            crit_threshold: crit,
            background_opacity: 0.82,
            background_color: DEFAULT_CARD_BACKGROUND,
            show_title: true,
            hidden_fans: Vec::new(),
        }
    }

    pub fn with_extra(mut self, metric: Metric, chart: ExtraChart) -> Self {
        self.add_extra(metric, chart);
        self
    }

    pub fn has_extra(&self, metric: Metric) -> bool {
        self.extra.iter().any(|e| e.metric == metric)
    }

    /// Ile miejsca w pionie zajmuja dodatki na tej karcie (wiersze paskow / osobne wykresy).
    fn extra_rows_height(&self) -> f32 {
        self.extra_rows_height_at(text_scale())
    }

    /// Jak `extra_rows_height`, ale przy skali tekstu `ts` (wiersze z tekstem rosna z czcionka).
    fn extra_rows_height_at(&self, ts: f32) -> f32 {
        match self.kind {
            WidgetKind::Bar => BAR_EXTRA_ROW_HEIGHT * ts * self.extra.len() as f32,
            WidgetKind::Graph => {
                let separate_h = SEPARATE_CHART_HEIGHT + SEPARATE_CHART_STRIP * (ts - 1.0);
                separate_h * self.extra.iter().filter(|e| e.chart == ExtraChart::Separate).count() as f32
                    + VALUE_ROW_HEIGHT * ts * self.value_grid_rows() as f32
            }
            _ => 0.0,
        }
    }

    /// Rozmiar "wzorcowy" karty (domyslny + dodatki) przy skali tekstu `ts` -
    /// stosunek dwoch takich rozmiarow mowi, o ile przeskalowac karte po zmianie czcionki.
    fn content_size_at(&self, ts: f32) -> [f32; 2] {
        let [w, h] = self.kind.default_size_at(ts);
        [w, h + self.extra_rows_height_at(ts)]
    }

    /// Ile wierszy (po 2 wartosci) ma siatka wartosci pod naglowkiem karty "Wykres".
    /// Do 2 wartosci mieszcza sie w naglowku - wtedy siatki nie ma.
    pub fn value_grid_rows(&self) -> usize {
        if self.kind != WidgetKind::Graph {
            return 0;
        }
        let shown = self.extra.iter().filter(|e| e.chart != ExtraChart::Separate).count();
        if shown > HEADER_EXTRAS_MAX { shown.div_ceil(2) } else { 0 }
    }

    /// Zmienia dodatki i dopasowuje wysokosc karty, zeby nowe wiersze/wykresy sie zmiescily.
    fn edit_extras(&mut self, change: impl FnOnce(&mut Vec<ExtraMetric>)) {
        let before = self.extra_rows_height();
        change(&mut self.extra);
        let after = self.extra_rows_height();
        self.size[1] = (self.size[1] + after - before).max(MIN_WIDGET_SIZE[1]);
    }

    pub fn add_extra(&mut self, metric: Metric, chart: ExtraChart) {
        if self.accepts_extra(metric) && !self.has_extra(metric) && self.extra.len() < MAX_EXTRA_METRICS {
            self.edit_extras(|extra| extra.push(ExtraMetric { metric, chart }));
        }
    }

    pub fn remove_extra(&mut self, index: usize) {
        if index < self.extra.len() {
            self.edit_extras(|extra| {
                extra.remove(index);
            });
        }
    }

    pub fn set_extra_chart(&mut self, index: usize, chart: ExtraChart) {
        if index < self.extra.len() {
            self.edit_extras(|extra| extra[index].chart = chart);
        }
    }

    /// Zmiana typu karty: nowy domyslny rozmiar + miejsce na dodatki w nowym ukladzie.
    pub fn set_kind(&mut self, kind: WidgetKind) {
        self.kind = kind;
        self.size = kind.default_size();
        self.size[1] += self.extra_rows_height();
    }

    /// Dodatkowa metryka musi byc z tego samego podzespolu co glowna (karta CPU = tylko CPU).
    pub fn accepts_extra(&self, metric: Metric) -> bool {
        metric != self.metric && metric.device() == self.metric.device()
    }

    /// Metryki, ktore jeszcze mozna dolozyc do karty.
    pub fn available_extra(&self) -> Vec<Metric> {
        if self.extra.len() >= MAX_EXTRA_METRICS {
            return Vec::new();
        }
        Metric::ALL
            .into_iter()
            .filter(|m| self.accepts_extra(*m) && !self.has_extra(*m))
            .collect()
    }

    /// Usuwa wszystko, co nie pasuje do metryki glownej (po jej zmianie albo recznej edycji pliku).
    pub fn drop_mismatched(&mut self) {
        let primary = self.metric;
        let mut seen = std::collections::HashSet::new();
        self.edit_extras(|extra| {
            extra.retain(|e| e.metric != primary && e.metric.device() == primary.device() && seen.insert(e.metric));
            extra.truncate(MAX_EXTRA_METRICS);
        });
        if !self.footer.fits(primary) {
            self.footer = WidgetFooter::Stats;
        }
    }

    pub fn with_footer(mut self, footer: WidgetFooter) -> Self {
        self.footer = footer;
        self
    }

    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            self.metric.short_label()
        } else {
            self.title.trim()
        }
    }
}

pub fn default_accent(metric: Metric) -> [u8; 3] {
    match metric {
        Metric::CpuLoad => [72, 166, 255],
        Metric::CpuTemp => [255, 150, 72],
        Metric::GpuLoad => [120, 226, 84],
        Metric::GpuTemp => [255, 104, 112],
        Metric::VramUsage => [184, 132, 255],
        Metric::CpuClock => [100, 220, 210],
        Metric::CpuPower => [255, 214, 10],
        Metric::GpuHotspot => [255, 80, 80],
        Metric::GpuMemTemp => [236, 120, 200],
        Metric::GpuClock => [90, 210, 255],
        Metric::GpuMemClock => [150, 160, 255],
        Metric::GpuPower => [255, 196, 64],
        Metric::GpuFan => [170, 220, 120],
        Metric::Fps | Metric::FpsAvg => [120, 226, 84],
        Metric::FpsLow1 => [255, 196, 64],
        Metric::FpsLow01 => [255, 140, 66],
        Metric::FrameTime => [120, 226, 84],
        Metric::RamUsage | Metric::RamUsed => [64, 200, 255],
        Metric::FanSpeed => [90, 200, 255],
    }
}

pub fn default_icon(metric: Metric) -> WidgetIcon {
    match metric {
        Metric::CpuLoad => WidgetIcon::Cpu,
        Metric::GpuLoad => WidgetIcon::Gpu,
        Metric::VramUsage => WidgetIcon::Memory,
        Metric::CpuTemp | Metric::GpuTemp | Metric::GpuMemTemp => WidgetIcon::Thermometer,
        Metric::GpuHotspot => WidgetIcon::Fire,
        Metric::CpuClock | Metric::GpuClock | Metric::GpuMemClock => WidgetIcon::Gauge,
        Metric::CpuPower | Metric::GpuPower => WidgetIcon::Lightning,
        Metric::GpuFan => WidgetIcon::Fan,
        Metric::Fps | Metric::FpsAvg | Metric::FpsLow1 | Metric::FpsLow01 => WidgetIcon::GameController,
        Metric::FrameTime => WidgetIcon::Timer,
        Metric::RamUsage | Metric::RamUsed => WidgetIcon::Memory,
        Metric::FanSpeed => WidgetIcon::Fan,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Globalny skrot pokazujacy/ukrywajacy overlay w grze, format `ctrl+shift+H`.
    pub overlay_hotkey: String,
    /// Globalny skrot otwierajacy/zamykajacy okno konfiguracji, format `ctrl+shift+O`.
    pub settings_hotkey: String,
    /// Czy okno konfiguracji ma sie otworzyc przy starcie programu.
    pub show_settings_on_start: bool,
    /// Wylacznik calego overlaya bez zamykania programu.
    pub overlay_visible: bool,
    pub sample_interval_ms: u64,
    pub history_seconds: u32,
    /// Z ilu ostatnich sekund liczony jest sredni FPS i 1% / 0.1% low.
    pub fps_stats_seconds: u32,
    pub grid_size: f32,
    pub snap_to_grid: bool,
    /// Magnetyczne przyciaganie do krawedzi ekranu i innych widgetow.
    pub snap_to_edges: bool,
    pub snap_distance: f32,
    pub animations: bool,
    /// Globalny mnoznik krycia calego overlaya 0.1..1.
    pub overlay_opacity: f32,
    /// Powiekszenie tekstu kart 0..=MAX_FONT_BOOST pt (karty rosna razem z tekstem).
    pub font_boost: u32,
    pub panel_theme: PanelTheme,
    /// Plik zrzutu z gry (w katalogu konfiguracji) jako tlo podgladu ekranu; pusty = tlo domyslne.
    pub preview_background: String,
    /// Jezyk interfejsu: auto (wedlug Windows), pl, en.
    pub language: LanguageSetting,
    pub widgets: Vec<WidgetConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            overlay_hotkey: "ctrl+shift+H".to_owned(),
            settings_hotkey: "ctrl+shift+O".to_owned(),
            show_settings_on_start: true,
            overlay_visible: true,
            sample_interval_ms: 1000,
            history_seconds: 60,
            fps_stats_seconds: 10,
            grid_size: 8.0,
            snap_to_grid: true,
            snap_to_edges: true,
            snap_distance: 10.0,
            animations: true,
            overlay_opacity: 1.0,
            font_boost: 0,
            panel_theme: PanelTheme::Dark,
            preview_background: String::new(),
            language: LanguageSetting::Auto,
            widgets: default_widgets(),
        }
    }
}

/// Nowa karta z sensownymi dodatkami: obciazenie od razu z temperatura,
/// GPU z paskiem VRAM, reszta ze statystykami min/sr/max.
pub fn suggested_widget(id: u64, metric: Metric, kind: WidgetKind, pos: [f32; 2]) -> WidgetConfig {
    // Typ musi pasowac do metryki (wentylatory = tylko karta "Wentylatory").
    let kind = if kind.fits(metric) { kind } else { WidgetKind::for_metric(metric)[0] };
    let widget = WidgetConfig::new(id, metric, kind, pos);
    let widget = match metric {
        Metric::CpuLoad => widget.with_extra(Metric::CpuTemp, ExtraChart::Line),
        Metric::GpuLoad => widget.with_extra(Metric::GpuTemp, ExtraChart::Line),
        // Karta FPS od razu ze srednia i 1% low (jak w popularnych overlayach).
        Metric::Fps => widget
            .with_extra(Metric::FpsAvg, ExtraChart::Value)
            .with_extra(Metric::FpsLow1, ExtraChart::Value),
        // Karta RAM: procent zajetosci + ile GB uzyte.
        Metric::RamUsage => widget.with_extra(Metric::RamUsed, ExtraChart::Value),
        _ => widget,
    };
    let footer = match (metric, kind) {
        (_, WidgetKind::Gauge | WidgetKind::Fans) => WidgetFooter::None,
        (Metric::GpuLoad, _) => WidgetFooter::VramBar,
        _ => WidgetFooter::Stats,
    };
    widget.with_footer(footer)
}

/// Domyslny uklad: dwie karty w rzedzie w lewym gornym rogu ekranu.
fn default_widgets() -> Vec<WidgetConfig> {
    let cpu = suggested_widget(1, Metric::CpuLoad, WidgetKind::Graph, [16.0, 16.0]);
    let gpu_x = cpu.pos[0] + cpu.size[0] + 8.0;
    let gpu = suggested_widget(2, Metric::GpuLoad, WidgetKind::Graph, [gpu_x, 16.0]);
    vec![cpu, gpu]
}

impl Config {
    pub fn history_len(&self) -> usize {
        let samples = (self.history_seconds as u64 * 1000) / self.sample_interval_ms.max(1);
        samples.clamp(2, 3600) as usize
    }

    pub fn sensor_timing(&self) -> crate::sensors::SensorTiming {
        crate::sensors::SensorTiming {
            interval_ms: self.sample_interval_ms,
            history_len: self.history_len(),
            fps_window_ms: u64::from(self.fps_stats_seconds) * 1000,
        }
    }

    /// Zmienia powiekszenie tekstu i dopasowuje rozmiar kazdej karty: rosnie tylko o miejsce
    /// zajete przez tekst, wiec wykresy i marginesy zachowuja proporcje, a reczny rozmiar
    /// (wiekszy/mniejszy od domyslnego) zostaje zachowany proporcjonalnie.
    pub fn set_font_boost(&mut self, boost: u32) {
        let boost = boost.min(MAX_FONT_BOOST);
        let (old, new) = (text_scale_for(self.font_boost), text_scale_for(boost));
        for widget in &mut self.widgets {
            let (before, after) = (widget.content_size_at(old), widget.content_size_at(new));
            for (axis, min_size) in MIN_WIDGET_SIZE.into_iter().enumerate() {
                let resized = widget.size[axis] * after[axis] / before[axis];
                widget.size[axis] = finite_or(resized, min_size).clamp(min_size, MAX_WIDGET_SIZE);
            }
        }
        self.font_boost = boost;
        set_text_scale(new);
    }

    /// Ustawia globalny jezyk interfejsu wedlug pola `language` (dziala od razu).
    pub fn apply_language(&self) {
        crate::i18n::set_current(self.language.resolve());
    }

    pub fn next_widget_id(&self) -> u64 {
        self.widgets.iter().map(|w| w.id).max().unwrap_or(0) + 1
    }

    /// Doprowadza wartosci do sensownych zakresow - plik moze byc edytowany recznie.
    pub fn sanitize(&mut self) {
        self.sample_interval_ms = self.sample_interval_ms.clamp(250, 10_000);
        self.history_seconds = self.history_seconds.clamp(10, 600);
        self.fps_stats_seconds = self.fps_stats_seconds.clamp(2, 120);
        self.grid_size = finite_or(self.grid_size, 8.0).clamp(2.0, 64.0);
        self.snap_distance = finite_or(self.snap_distance, 10.0).clamp(0.0, 64.0);
        self.overlay_opacity = finite_or(self.overlay_opacity, 1.0).clamp(0.1, 1.0);
        // Rozmiary kart w pliku sa juz zapisane dla tej czcionki - tylko ustawiamy skale.
        self.font_boost = self.font_boost.min(MAX_FONT_BOOST);
        set_text_scale(text_scale_for(self.font_boost));
        self.apply_language();

        let mut seen_ids = std::collections::HashSet::new();
        let mut next_id = self.next_widget_id();
        for widget in &mut self.widgets {
            // Zduplikowane id psuja zaznaczanie i przeciaganie, wiec nadajemy nowe.
            if !seen_ids.insert(widget.id) {
                widget.id = next_id;
                seen_ids.insert(next_id);
                next_id += 1;
            }
            for (axis, min_size) in MIN_WIDGET_SIZE.into_iter().enumerate() {
                widget.pos[axis] = finite_or(widget.pos[axis], 0.0).clamp(-MAX_WIDGET_SIZE, 10_000.0);
                widget.size[axis] = finite_or(widget.size[axis], min_size).clamp(min_size, MAX_WIDGET_SIZE);
            }
            widget.background_opacity = finite_or(widget.background_opacity, 0.55).clamp(0.0, 1.0);
            widget.warn_threshold = finite_or(widget.warn_threshold, 75.0);
            widget.crit_threshold = finite_or(widget.crit_threshold, 90.0);
            // Krytyczny prog lezy "dalej" niz ostrzezenie: wyzej dla temperatur, nizej dla FPS.
            let reversed = if widget.metric.higher_is_worse() {
                widget.crit_threshold < widget.warn_threshold
            } else {
                widget.crit_threshold > widget.warn_threshold
            };
            if reversed {
                std::mem::swap(&mut widget.warn_threshold, &mut widget.crit_threshold);
            }
            widget.title.truncate(64);
            if !widget.kind.fits(widget.metric) {
                widget.set_kind(WidgetKind::for_metric(widget.metric)[0]);
            }
            // Migracja starego pola `secondary` + porzadki w liscie dodatkowych metryk.
            if let Some(legacy) = widget.legacy_secondary.take()
                && !widget.has_extra(legacy)
            {
                widget.extra.insert(0, ExtraMetric { metric: legacy, chart: ExtraChart::Line });
            }
            if widget.legacy_plot_extra.take() == Some(false) {
                for extra in &mut widget.extra {
                    extra.chart = ExtraChart::Value;
                }
            }
            widget.drop_mismatched();
        }
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        let mut config: Config = toml::from_str(text)?;
        config.sanitize();
        Ok(config)
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// Zmienna srodowiskowa wymuszajaca katalog konfiguracji (testy, kilka profili).
const CONFIG_DIR_ENV: &str = "ARTVITALS_CONFIG_DIR";

/// Katalog konfiguracji i logu. Wersja portable: folder z plikiem exe, jesli da sie w nim pisac
/// (np. pendrive, Pulpit); inaczej (np. Program Files) `%APPDATA%\ArtVitals`.
/// Wyznaczany raz na uruchomienie, zeby config i log nie rozjechaly sie miedzy katalogami.
pub fn config_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(resolve_config_dir).clone()
}

fn appdata_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join(name))
}

fn resolve_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        return PathBuf::from(dir);
    }
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from));
    if let Some(dir) = exe_dir.filter(|dir| is_writable(dir)) {
        return dir;
    }
    appdata_dir(APP_DIR_NAME).unwrap_or_else(|| PathBuf::from("."))
}

/// Czy w katalogu da sie utworzyc plik (probny plik jest od razu usuwany).
fn is_writable(dir: &std::path::Path) -> bool {
    let probe = dir.join(".artvitals-write-test");
    let created = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).is_ok();
    if created {
        let _ = std::fs::remove_file(&probe);
    }
    created
}

pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE_NAME)
}

/// Wczytuje konfiguracje; przy braku pliku tworzy domyslna, przy bledzie
/// skladni zachowuje kopie zepsutego pliku i startuje z domyslnymi ustawieniami.
/// Gdy w katalogu konfiguracji nie ma jeszcze pliku, kopiuje go z poprzedniej lokalizacji:
/// `%APPDATA%\ArtVitals` (wersja sprzed portable), potem `%APPDATA%\CraftGamingBoard` (stara nazwa).
fn migrate_legacy_config(path: &std::path::Path) {
    if path.exists() {
        return;
    }
    let Some(legacy) = [APP_DIR_NAME, LEGACY_APP_DIR_NAME]
        .into_iter()
        .filter_map(appdata_dir)
        .map(|dir| dir.join(CONFIG_FILE_NAME))
        .find(|candidate| candidate.exists() && candidate != path)
    else {
        return;
    };
    let copied = std::fs::create_dir_all(config_dir()).and_then(|_| std::fs::copy(&legacy, path));
    match copied {
        Ok(_) => log::info!("Przeniesiono konfiguracje z {}", legacy.display()),
        Err(err) => log::warn!("Nie udalo sie przeniesc konfiguracji z {}: {err}", legacy.display()),
    }
}

pub fn load_or_create() -> Config {
    let path = config_path();
    migrate_legacy_config(&path);
    match std::fs::read_to_string(&path) {
        Ok(text) => match Config::from_toml(&text) {
            Ok(config) => return config,
            Err(err) => {
                log::error!("Niepoprawny {}: {err}", path.display());
                let backup = path.with_extension("toml.broken");
                if let Err(err) = std::fs::copy(&path, &backup) {
                    log::warn!("Nie udalo sie zrobic kopii {}: {err}", backup.display());
                }
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            log::info!("Brak konfiguracji, tworze domyslna: {}", path.display());
        }
        Err(err) => log::error!("Nie mozna odczytac {}: {err}", path.display()),
    }
    let config = Config::default();
    config.apply_language();
    if let Err(err) = save(&config) {
        log::error!("Nie udalo sie zapisac domyslnej konfiguracji: {err}");
    }
    config
}

/// Zapisuje konfiguracje atomowo (plik tymczasowy + rename), zeby watcher
/// nigdy nie przeczytal polowy pliku. Zwraca zapisana tresc.
pub fn save(config: &Config) -> std::io::Result<String> {
    let text = config.to_toml().map_err(std::io::Error::other)?;
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{CONFIG_FILE_NAME}.tmp"));
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, config_path())?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips_through_toml() {
        let config = Config::default();
        let text = config.to_toml().unwrap();
        assert_eq!(Config::from_toml(&text).unwrap(), config);
    }

    #[test]
    fn sanitize_fixes_bad_values_and_duplicate_ids() {
        let text = r#"
            sample_interval_ms = 1
            overlay_opacity = 7.0
            [[widgets]]
            id = 3
            metric = "gpu_temp"
            kind = "gauge"
            size = [1.0, 99999.0]
            warn_threshold = 90.0
            crit_threshold = 70.0
            [[widgets]]
            id = 3
            metric = "cpu_load"
            kind = "bar"
        "#;
        let config = Config::from_toml(text).unwrap();
        assert_eq!(config.sample_interval_ms, 250);
        assert_eq!(config.overlay_opacity, 1.0);
        assert_eq!(config.widgets[0].size, [MIN_WIDGET_SIZE[0], MAX_WIDGET_SIZE]);
        assert!(config.widgets[0].warn_threshold < config.widgets[0].crit_threshold);
        assert_ne!(config.widgets[0].id, config.widgets[1].id);
    }

    #[test]
    fn legacy_secondary_migrates_to_extra_list() {
        let text = r#"
            [[widgets]]
            id = 1
            metric = "cpu_load"
            kind = "graph"
            secondary = "cpu_temp"
            extra = ["cpu_load", "cpu_temp", "cpu_temp"]
            plot_extra = false
        "#;
        let config = Config::from_toml(text).unwrap();
        assert_eq!(config.widgets[0].extra, vec![ExtraMetric { metric: Metric::CpuTemp, chart: ExtraChart::Value }]);
        assert!(!config.to_toml().unwrap().contains("plot_extra"));
        assert!(!config.to_toml().unwrap().contains("secondary"));
    }

    #[test]
    fn font_boost_grows_cards_by_text_only_and_roundtrips() {
        assert_eq!(WidgetKind::Graph.default_size_at(1.0), [200.0, 92.0]);
        assert_eq!(WidgetKind::Bar.default_size_at(1.0), [200.0, 80.0]);
        let widget = WidgetConfig::new(1, Metric::GpuLoad, WidgetKind::Graph, [0.0, 0.0])
            .with_extra(Metric::GpuClock, ExtraChart::Separate);
        let original = widget.size;
        let mut config = Config { widgets: vec![widget], ..Default::default() };
        config.set_font_boost(99);
        assert_eq!(config.font_boost, MAX_FONT_BOOST);
        let grown = config.widgets[0].size;
        assert!(grown[0] > original[0] && grown[1] > original[1]);
        // Wykres (czesc stala) nie rosnie, wiec karta rosnie wolniej niz czcionka.
        assert!(grown[1] / original[1] < text_scale_for(MAX_FONT_BOOST));
        config.set_font_boost(0);
        assert!((config.widgets[0].size[0] - original[0]).abs() < 0.01);
        assert!((config.widgets[0].size[1] - original[1]).abs() < 0.01);
    }

    #[test]
    fn card_accepts_only_metrics_of_its_device() {
        let cpu = WidgetConfig::new(1, Metric::CpuLoad, WidgetKind::Graph, [0.0, 0.0]);
        assert_eq!(cpu.available_extra(), vec![Metric::CpuTemp, Metric::CpuClock, Metric::CpuPower]);
        let gpu = WidgetConfig::new(2, Metric::GpuTemp, WidgetKind::Bar, [0.0, 0.0]);
        assert!(gpu.available_extra().iter().all(|m| m.device() == crate::metrics::Device::Gpu));
        assert!(gpu.available_extra().contains(&Metric::GpuHotspot));

        let text = r#"
            [[widgets]]
            id = 1
            metric = "cpu_load"
            kind = "graph"
            footer = "vram_bar"
            extra = ["gpu_temp", "cpu_temp", "vram_usage"]
        "#;
        let config = Config::from_toml(text).unwrap();
        assert_eq!(config.widgets[0].extra.len(), 1);
        assert_eq!(config.widgets[0].extra[0].metric, Metric::CpuTemp);
        assert_eq!(config.widgets[0].footer, WidgetFooter::Stats);
    }

    #[test]
    fn extra_metrics_roundtrip_with_chart_mode_and_resize_card() {
        let mut widget = WidgetConfig::new(1, Metric::GpuLoad, WidgetKind::Graph, [0.0, 0.0]);
        let base = widget.size[1];
        widget.add_extra(Metric::GpuClock, ExtraChart::Separate);
        assert_eq!(widget.size[1], base + SEPARATE_CHART_HEIGHT);
        widget.set_extra_chart(0, ExtraChart::Line);
        assert_eq!(widget.size[1], base);

        let mut config = Config { widgets: vec![widget], ..Default::default() };
        config.widgets[0].add_extra(Metric::GpuHotspot, ExtraChart::Separate);
        let back = Config::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(back.widgets[0].extra, config.widgets[0].extra);
    }

    #[test]
    fn many_value_extras_get_their_own_rows() {
        let mut widget = WidgetConfig::new(1, Metric::GpuLoad, WidgetKind::Graph, [0.0, 0.0]);
        let base = widget.size[1];
        widget.add_extra(Metric::GpuTemp, ExtraChart::Value);
        widget.add_extra(Metric::GpuPower, ExtraChart::Line);
        assert_eq!(widget.value_grid_rows(), 0);
        widget.add_extra(Metric::GpuFan, ExtraChart::Value);
        assert_eq!(widget.value_grid_rows(), 2);
        assert_eq!(widget.size[1], base + 2.0 * VALUE_ROW_HEIGHT);
    }

    #[test]
    fn fps_thresholds_are_kept_in_falling_order() {
        let text = r#"
            [[widgets]]
            id = 1
            metric = "fps"
            kind = "number"
            warn_threshold = 30.0
            crit_threshold = 60.0
        "#;
        let config = Config::from_toml(text).unwrap();
        assert_eq!((config.widgets[0].warn_threshold, config.widgets[0].crit_threshold), (60.0, 30.0));
    }

    #[test]
    fn language_field_is_optional_and_roundtrips() {
        // Stary plik bez pola `language` -> auto.
        let old = Config::from_toml("grid_size = 16.0").unwrap();
        assert_eq!(old.language, LanguageSetting::Auto);
        let config = Config::from_toml("language = \"en\"").unwrap();
        assert_eq!(config.language, LanguageSetting::En);
        // sanitize ustawia biezacy jezyk (w testach - dla tego watku).
        assert_eq!(crate::i18n::current(), crate::i18n::Lang::En);
        assert!(config.to_toml().unwrap().contains("language = \"en\""));
        let config = Config::from_toml("language = \"pl\"").unwrap();
        assert_eq!(config.language, LanguageSetting::Pl);
        assert_eq!(crate::i18n::current(), crate::i18n::Lang::Pl);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let config = Config::from_toml("grid_size = 16.0").unwrap();
        assert_eq!(config.grid_size, 16.0);
        assert_eq!(config.settings_hotkey, Config::default().settings_hotkey);
    }
}
