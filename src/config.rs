//! Konfiguracja overlaya zapisywana w `%APPDATA%\ArtVitals\config.toml`.
//!
//! Plik jest jedynym zrodlem prawdy: okno konfiguracji zapisuje do niego zmiany,
//! a reczna edycja pliku jest przeladowywana na zywo (patrz `overlay::watcher`).
//! Po wczytaniu wartosci sa walidowane (`sanitize`), wiec bledny plik nie wywroci aplikacji.
//! Zaleznosci: serde, toml.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 4] = [
        WidgetKind::Graph,
        WidgetKind::Gauge,
        WidgetKind::Bar,
        WidgetKind::Number,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WidgetKind::Graph => "Wykres",
            WidgetKind::Gauge => "Wskaźnik",
            WidgetKind::Bar => "Pasek",
            WidgetKind::Number => "Liczba",
        }
    }

    pub fn default_size(self) -> [f32; 2] {
        match self {
            WidgetKind::Graph => [200.0, 92.0],
            WidgetKind::Gauge => [112.0, 112.0],
            // Naglowek (tytul + wartosc) + pasek + stopka musza sie zmiescic bez nakladania.
            WidgetKind::Bar => [200.0, 80.0],
            WidgetKind::Number => [132.0, 92.0],
        }
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
            ExtraChart::Value => "Tylko wartość",
            ExtraChart::Line => "Na wykresie głównym",
            ExtraChart::Separate => "Osobny wykres",
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
            WidgetIcon::None => "Brak",
            WidgetIcon::Cpu => "Procesor",
            WidgetIcon::Gpu => "Karta graficzna",
            WidgetIcon::Memory => "Pamięć",
            WidgetIcon::Thermometer => "Termometr",
            WidgetIcon::Fire => "Płomień",
            WidgetIcon::Gauge => "Licznik (zegary)",
            WidgetIcon::Lightning => "Błyskawica (moc)",
            WidgetIcon::Fan => "Wentylator",
            WidgetIcon::GameController => "Pad (FPS)",
            WidgetIcon::Timer => "Stoper (czas klatki)",
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
            WidgetFooter::None => "Brak",
            WidgetFooter::Stats => "Min / śr. / max",
            WidgetFooter::VramBar => "Pasek VRAM",
            WidgetFooter::Segments => "Pasek segmentowy",
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
    pub show_title: bool,
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
            show_title: true,
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
        match self.kind {
            WidgetKind::Bar => BAR_EXTRA_ROW_HEIGHT * self.extra.len() as f32,
            WidgetKind::Graph => {
                SEPARATE_CHART_HEIGHT * self.extra.iter().filter(|e| e.chart == ExtraChart::Separate).count() as f32
                    + VALUE_ROW_HEIGHT * self.value_grid_rows() as f32
            }
            _ => 0.0,
        }
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
    pub grid_size: f32,
    pub snap_to_grid: bool,
    /// Magnetyczne przyciaganie do krawedzi ekranu i innych widgetow.
    pub snap_to_edges: bool,
    pub snap_distance: f32,
    pub animations: bool,
    /// Globalny mnoznik krycia calego overlaya 0.1..1.
    pub overlay_opacity: f32,
    pub panel_theme: PanelTheme,
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
            grid_size: 8.0,
            snap_to_grid: true,
            snap_to_edges: true,
            snap_distance: 10.0,
            animations: true,
            overlay_opacity: 1.0,
            panel_theme: PanelTheme::Dark,
            widgets: default_widgets(),
        }
    }
}

/// Nowa karta z sensownymi dodatkami: obciazenie od razu z temperatura,
/// GPU z paskiem VRAM, reszta ze statystykami min/sr/max.
pub fn suggested_widget(id: u64, metric: Metric, kind: WidgetKind, pos: [f32; 2]) -> WidgetConfig {
    let widget = WidgetConfig::new(id, metric, kind, pos);
    let widget = match metric {
        Metric::CpuLoad => widget.with_extra(Metric::CpuTemp, ExtraChart::Line),
        Metric::GpuLoad => widget.with_extra(Metric::GpuTemp, ExtraChart::Line),
        // Karta FPS od razu ze srednia i 1% low (jak w popularnych overlayach).
        Metric::Fps => widget
            .with_extra(Metric::FpsAvg, ExtraChart::Value)
            .with_extra(Metric::FpsLow1, ExtraChart::Value),
        _ => widget,
    };
    let footer = match (metric, kind) {
        (_, WidgetKind::Gauge) => WidgetFooter::None,
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

    pub fn next_widget_id(&self) -> u64 {
        self.widgets.iter().map(|w| w.id).max().unwrap_or(0) + 1
    }

    /// Doprowadza wartosci do sensownych zakresow - plik moze byc edytowany recznie.
    pub fn sanitize(&mut self) {
        self.sample_interval_ms = self.sample_interval_ms.clamp(250, 10_000);
        self.history_seconds = self.history_seconds.clamp(10, 600);
        self.grid_size = finite_or(self.grid_size, 8.0).clamp(2.0, 64.0);
        self.snap_distance = finite_or(self.snap_distance, 10.0).clamp(0.0, 64.0);
        self.overlay_opacity = finite_or(self.overlay_opacity, 1.0).clamp(0.1, 1.0);

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

pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR_NAME)
}

pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE_NAME)
}

/// Wczytuje konfiguracje; przy braku pliku tworzy domyslna, przy bledzie
/// skladni zachowuje kopie zepsutego pliku i startuje z domyslnymi ustawieniami.
/// Kopiuje config.toml ze starego folderu programu (CraftGamingBoard), jesli nowego jeszcze nie ma.
fn migrate_legacy_config(path: &std::path::Path) {
    if path.exists() {
        return;
    }
    let Some(appdata) = std::env::var_os("APPDATA") else { return };
    let legacy = PathBuf::from(appdata).join(LEGACY_APP_DIR_NAME).join(CONFIG_FILE_NAME);
    if !legacy.exists() {
        return;
    }
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
    fn missing_fields_fall_back_to_defaults() {
        let config = Config::from_toml("grid_size = 16.0").unwrap();
        assert_eq!(config.grid_size, 16.0);
        assert_eq!(config.settings_hotkey, Config::default().settings_hotkey);
    }
}
