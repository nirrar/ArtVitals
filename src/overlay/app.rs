//! Glowna aplikacja: overlay (glowny viewport) + okno konfiguracji (drugi viewport).
//!
//! Overlay to przezroczyste okno na caly monitor, zawsze na wierzchu i zawsze
//! click-through - tylko wyswietla karty, nigdy nie przechwytuje myszy ani klawiatury,
//! wiec nie przeszkadza w grze. Cala konfiguracja (przeciaganie, rozmiar, przypinanie,
//! wlasciwosci) odbywa sie w osobnym, zwyklym oknie na pulpicie (`settings.rs`),
//! a zmiany sa widoczne na overlayu na zywo.
//! Okno konfiguracji otwiera/zamyka globalny skrot. Zapis do config.toml jest automatyczny,
//! z opoznieniem, zeby nie pisac pliku przy kazdej klatce przeciagania.

use std::time::{Duration, Instant};

use eframe::egui::{self, CornerRadius, Id, Rect, ThemePreference, ViewportCommand, ViewportId, pos2, vec2};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::hotkey::{HotkeySlot, Hotkeys};
use super::tray::{Tray, TrayCommand};
use crate::single_instance::InstanceGuard;
use super::win32::{LayeredFix, fix_overlay_window};
use super::watcher::ConfigWatcher;
use crate::config::{self, Config, PanelTheme, WidgetConfig, WidgetKind, default_accent, default_icon};
use crate::metrics::Metric;
use crate::sensors::SensorHub;
use crate::widgets::{self, PaintContext, WidgetValues};

const AUTOSAVE_DELAY: Duration = Duration::from_millis(600);

/// Operacje na kartach zbierane w trakcie rysowania UI i wykonywane po nim.
pub(super) enum WidgetAction {
    TogglePin(u64),
    SetKind(u64, WidgetKind),
    SetMetric(u64, Metric),
    Duplicate(u64),
    BringToFront(u64),
    Delete(u64),
    Add(Metric, WidgetKind),
}

pub struct OverlayApp {
    pub(super) config: Config,
    pub(super) sensors: SensorHub,
    pub(super) hotkeys: Hotkeys,
    tray: Option<Tray>,
    /// Ustawiane, gdy ktos uruchomi program drugi raz - wtedy otwieramy okno konfiguracji.
    show_requested: Arc<AtomicBool>,
    /// Ile klatek jeszcze probowac naprawic styl okna overlaya (patrz `win32`).
    layered_fix_attempts: u32,
    watcher: ConfigWatcher,
    pub(super) settings_open: bool,
    pub(super) settings: super::settings::SettingsState,
    dirty_since: Option<Instant>,
    last_saved_text: String,
    /// Komunikat w oknie konfiguracji (tresc, czy blad).
    pub(super) status: Option<(String, bool)>,
    window_fitted: bool,
    /// Obszar monitora w punktach - uklad wspolrzednych kart.
    pub(super) screen: Rect,
}

impl OverlayApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config, instance: InstanceGuard) -> Self {
        let ctx = &cc.egui_ctx;
        crate::icons::install(ctx);
        let show_requested = instance.listen(ctx);
        let tray = Tray::new(ctx);
        let sensors = SensorHub::start(ctx.clone(), config.sample_interval_ms, config.history_len());
        let mut hotkeys = Hotkeys::new(ctx);
        let mut status = None;
        for (slot, text) in [
            (HotkeySlot::Overlay, &config.overlay_hotkey),
            (HotkeySlot::Settings, &config.settings_hotkey),
        ] {
            if let Err(err) = hotkeys.set(slot, text) {
                log::error!("{err}");
                status = Some((err, true));
            }
        }
        apply_style(ctx, config.panel_theme);
        let last_saved_text = config.to_toml().unwrap_or_default();
        Self {
            watcher: ConfigWatcher::start(ctx),
            settings_open: config.show_settings_on_start,
            settings: Default::default(),
            config,
            sensors,
            hotkeys,
            tray,
            show_requested,
            layered_fix_attempts: 600,
            dirty_since: None,
            last_saved_text,
            status,
            window_fitted: false,
            screen: Rect::from_min_size(egui::Pos2::ZERO, vec2(1920.0, 1080.0)),
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn save_now(&mut self) {
        self.dirty_since = None;
        match config::save(&self.config) {
            Ok(text) => self.last_saved_text = text,
            Err(err) => {
                log::error!("Zapis konfiguracji nieudany: {err}");
                self.status = Some((format!("Nie udało się zapisać konfiguracji: {err}"), true));
            }
        }
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        if let Some(since) = self.dirty_since {
            if !self.settings.is_dragging() && since.elapsed() >= AUTOSAVE_DELAY {
                self.save_now();
            } else {
                ctx.request_repaint_after(AUTOSAVE_DELAY);
            }
        }
    }

    /// Przeladowanie po recznej edycji pliku; wlasne zapisy sa pomijane po porownaniu tresci.
    fn reload_if_changed(&mut self, ctx: &egui::Context) {
        if !self.watcher.take_changed() {
            return;
        }
        let Ok(text) = std::fs::read_to_string(config::config_path()) else {
            return;
        };
        if text == self.last_saved_text {
            return;
        }
        match Config::from_toml(&text) {
            Ok(new_config) => {
                log::info!("Przeladowano config.toml po zmianie z zewnatrz");
                self.last_saved_text = text;
                self.apply_config(ctx, new_config);
                self.status = Some(("Wczytano zmiany z config.toml".to_owned(), false));
            }
            Err(err) => {
                log::warn!("config.toml ma blad, pomijam: {err}");
                self.status = Some((format!("Błąd w config.toml: {err}"), true));
            }
        }
    }

    pub(super) fn apply_config(&mut self, ctx: &egui::Context, new_config: Config) {
        for (slot, new, old) in [
            (HotkeySlot::Overlay, &new_config.overlay_hotkey, &self.config.overlay_hotkey),
            (HotkeySlot::Settings, &new_config.settings_hotkey, &self.config.settings_hotkey),
        ] {
            if new != old
                && let Err(err) = self.hotkeys.set(slot, new)
            {
                self.status = Some((err, true));
            }
        }
        if new_config.panel_theme != self.config.panel_theme {
            apply_style(ctx, new_config.panel_theme);
        }
        self.sensors.set_timing(new_config.sample_interval_ms, new_config.history_len());
        self.config = new_config;
        if self.settings.selected.is_some_and(|id| !self.config.widgets.iter().any(|w| w.id == id)) {
            self.settings.selected = None;
        }
    }

    pub(super) fn set_panel_theme(&mut self, ctx: &egui::Context, theme: PanelTheme) {
        self.config.panel_theme = theme;
        apply_style(ctx, theme);
        self.mark_dirty();
    }

    pub(super) fn timing_changed(&mut self) {
        self.sensors.set_timing(self.config.sample_interval_ms, self.config.history_len());
        self.mark_dirty();
    }

    pub(super) fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings.reset_interaction();
        if self.dirty_since.is_some() {
            self.save_now();
        }
    }

    /// Dopasowanie okna overlaya do monitora - rozmiar monitora jest znany dopiero po starcie okna.
    fn fit_window(&mut self, ctx: &egui::Context) {
        let monitor = ctx.input(|i| i.viewport().monitor_size);
        if let Some(size) = monitor {
            self.screen = Rect::from_min_size(egui::Pos2::ZERO, size);
            if !self.window_fitted {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::Pos2::ZERO));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                self.window_fitted = true;
            }
        }
    }

    /// Okno warstwowe bez atrybutow nie jest rysowane, a DWM dorysowuje mu ramke - poprawiamy
    /// to w pierwszych klatkach (patrz `win32`).
    fn fix_overlay_visibility(&mut self, ctx: &egui::Context) {
        if self.layered_fix_attempts == 0 {
            return;
        }
        match fix_overlay_window(crate::OVERLAY_TITLE) {
            LayeredFix::Applied => {
                log::info!("Okno overlaya gotowe (atrybuty warstwy ustawione)");
                self.layered_fix_attempts = 0;
            }
            LayeredFix::NotYet => {
                self.layered_fix_attempts -= 1;
                if self.layered_fix_attempts == 0 {
                    log::warn!("Nie znaleziono warstwowego okna overlaya - poprawka pominieta");
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            LayeredFix::Failed(err) => {
                log::error!("SetLayeredWindowAttributes: {err}");
                self.layered_fix_attempts = 0;
            }
        }
    }

    fn toggle_overlay(&mut self) {
        self.config.overlay_visible = !self.config.overlay_visible;
        self.mark_dirty();
    }

    /// Otwiera okno konfiguracji (albo wyciaga je na wierzch, jesli juz jest otwarte).
    fn open_settings(&mut self, ctx: &egui::Context) {
        self.settings_open = true;
        ctx.send_viewport_cmd_to(super::settings::viewport_id(), ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(super::settings::viewport_id(), ViewportCommand::Focus);
    }

    pub(super) fn quit(&mut self, ctx: &egui::Context) {
        if self.dirty_since.is_some() {
            self.save_now();
        }
        self.sensors.stop();
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
    }

    pub(super) fn apply_action(&mut self, action: WidgetAction) {
        let widgets = &mut self.config.widgets;
        let find = |widgets: &[WidgetConfig], id: u64| widgets.iter().position(|w| w.id == id);
        match action {
            WidgetAction::TogglePin(id) => {
                if let Some(i) = find(widgets, id) {
                    widgets[i].pinned = !widgets[i].pinned;
                }
            }
            WidgetAction::SetKind(id, kind) => {
                if let Some(i) = find(widgets, id) {
                    widgets[i].set_kind(kind);
                }
            }
            WidgetAction::SetMetric(id, metric) => {
                if let Some(i) = find(widgets, id) {
                    change_metric(&mut widgets[i], metric);
                }
            }
            WidgetAction::Duplicate(id) => {
                if let Some(i) = find(widgets, id) {
                    let mut copy = widgets[i].clone();
                    copy.id = self.config.next_widget_id();
                    copy.pinned = false;
                    let moved = Rect::from_min_size(pos2(copy.pos[0] + 16.0, copy.pos[1] + 16.0), copy.size.into());
                    copy.pos = clamp_to_screen(moved, self.screen).min.into();
                    self.settings.selected = Some(copy.id);
                    self.config.widgets.push(copy);
                }
            }
            WidgetAction::BringToFront(id) => {
                if let Some(i) = find(widgets, id) {
                    let widget = widgets.remove(i);
                    widgets.push(widget);
                }
            }
            WidgetAction::Delete(id) => {
                widgets.retain(|w| w.id != id);
                if self.settings.selected == Some(id) {
                    self.settings.selected = None;
                }
            }
            WidgetAction::Add(metric, kind) => {
                let id = self.config.next_widget_id();
                // Nowa karta pod najnizsza istniejaca, zeby nie przykryc innych.
                let y = self
                    .config
                    .widgets
                    .iter()
                    .map(|w| w.pos[1] + w.size[1] + 8.0)
                    .fold(16.0, f32::max);
                let mut widget = config::suggested_widget(id, metric, kind, [16.0, y]);
                let rect = Rect::from_min_size(widget.pos.into(), widget.size.into());
                widget.pos = clamp_to_screen(rect, self.screen).min.into();
                self.config.widgets.push(widget);
                self.settings.selected = Some(id);
            }
        }
        self.mark_dirty();
    }

    /// Rysuje wszystkie karty w ich pozycjach ekranowych (overlay i podglad w oknie konfiguracji).
    pub(super) fn paint_widgets(&self, ctx: &egui::Context, painter: &egui::Painter) {
        for widget in &self.config.widgets {
            let rect = Rect::from_min_size(widget.pos.into(), widget.size.into());
            self.paint_widget_at(ctx, painter, widget, rect, 1.0);
        }
    }

    /// Rysuje jedna karte w dowolnym prostokacie z danym powiekszeniem (podglad w oknie edycji).
    pub(super) fn paint_widget_at(
        &self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        widget: &WidgetConfig,
        rect: Rect,
        zoom: f32,
    ) {
        let data = self.sensors.lock();
        let pc = PaintContext {
            painter,
            data: &data,
            opacity: self.config.overlay_opacity,
            history_len: self.config.history_len(),
            zoom,
        };
        let values = WidgetValues {
            primary: animated(ctx, self.config.animations, widget.id, 0, data.value(widget.metric)),
            extras: widget
                .extra
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    (e.metric, animated(ctx, self.config.animations, widget.id, i as u8 + 1, data.value(e.metric)))
                })
                .collect(),
        };
        widgets::paint_widget(&pc, rect, widget, &values);
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.fit_window(ctx);
        self.fix_overlay_visibility(ctx);
        for slot in self.hotkeys.take_pressed() {
            match slot {
                HotkeySlot::Overlay => self.toggle_overlay(),
                HotkeySlot::Settings => {
                    if self.settings_open {
                        self.close_settings();
                    } else {
                        self.open_settings(ctx);
                    }
                }
            }
        }
        if self.show_requested.swap(false, Ordering::Relaxed) {
            self.open_settings(ctx);
        }
        let commands = self.tray.as_ref().map(|t| t.take_commands()).unwrap_or_default();
        for command in commands {
            match command {
                TrayCommand::OpenSettings => self.open_settings(ctx),
                TrayCommand::ToggleOverlay => {
                    self.toggle_overlay();
                    if let Some(tray) = &mut self.tray {
                        tray.invalidate();
                    }
                }
                TrayCommand::Quit => self.quit(ctx),
            }
        }
        if let Some(tray) = &mut self.tray {
            tray.sync_overlay_visible(self.config.overlay_visible);
        }
        self.reload_if_changed(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.config.overlay_visible {
            self.paint_widgets(&ctx, ui.painter());
        }
        if self.settings_open {
            self.show_settings_window(&ctx);
        }
        self.autosave(&ctx);
    }
}

impl Drop for OverlayApp {
    fn drop(&mut self) {
        self.sensors.stop();
        if self.dirty_since.is_some() {
            self.save_now();
        }
    }
}

fn animated(ctx: &egui::Context, enabled: bool, widget_id: u64, slot: u8, value: Option<f32>) -> Option<f32> {
    let value = value?;
    if !enabled {
        return Some(value);
    }
    Some(ctx.animate_value_with_time(Id::unique(("value", widget_id, slot)), value, 0.35))
}

/// Zmiana metryki: progi zawsze wracaja do domyslnych (inne jednostki), a kolor i ikona
/// tylko wtedy, gdy uzytkownik ich wczesniej nie zmienial.
pub(super) fn change_metric(widget: &mut WidgetConfig, metric: Metric) {
    let old = widget.metric;
    if widget.accent == default_accent(old) {
        widget.accent = default_accent(metric);
    }
    if widget.icon == default_icon(old) {
        widget.icon = default_icon(metric);
    }
    let (warn, crit) = metric.default_thresholds();
    widget.warn_threshold = warn;
    widget.crit_threshold = crit;
    widget.metric = metric;
    // Zmiana podzespolu (np. CPU -> GPU) usuwa dodatki, ktore przestaly pasowac.
    widget.drop_mismatched();
}

pub(super) fn clamp_to_screen(rect: Rect, screen: Rect) -> Rect {
    let size = rect.size().min(screen.size());
    let min = pos2(
        rect.min.x.clamp(screen.min.x, screen.max.x - size.x),
        rect.min.y.clamp(screen.min.y, screen.max.y - size.y),
    );
    Rect::from_min_size(min, size)
}

fn apply_style(ctx: &egui::Context, theme: PanelTheme) {
    ctx.set_theme(match theme {
        PanelTheme::Dark => ThemePreference::Dark,
        PanelTheme::Light => ThemePreference::Light,
    });
    for egui_theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(egui_theme, |style| {
            style.spacing.item_spacing = vec2(8.0, 6.0);
            style.spacing.button_padding = vec2(10.0, 4.0);
            style.spacing.interact_size.y = 24.0;
            style.visuals.window_corner_radius = CornerRadius::same(12);
            style.visuals.menu_corner_radius = CornerRadius::same(8);
            style.visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
            style.visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
            style.visuals.widgets.active.corner_radius = CornerRadius::same(6);
            style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);
            style.animation_time = 0.12;
            let accent = super::ui_kit::palette(theme).accent;
            style.visuals.selection.bg_fill = accent;
            style.visuals.hyperlink_color = accent;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_metric_keeps_custom_accent() {
        let mut widget = WidgetConfig::new(1, Metric::CpuLoad, WidgetKind::Graph, [0.0, 0.0]);
        widget.accent = [1, 2, 3];
        change_metric(&mut widget, Metric::GpuTemp);
        assert_eq!(widget.accent, [1, 2, 3]);
        assert_eq!(widget.icon, default_icon(Metric::GpuTemp));
        assert_eq!((widget.warn_threshold, widget.crit_threshold), Metric::GpuTemp.default_thresholds());
    }

    #[test]
    fn change_metric_to_other_device_drops_extras() {
        let mut widget = config::suggested_widget(1, Metric::CpuLoad, WidgetKind::Graph, [0.0, 0.0]);
        assert_eq!(widget.extra.len(), 1);
        assert_eq!(widget.extra[0].metric, Metric::CpuTemp);
        change_metric(&mut widget, Metric::GpuLoad);
        assert!(widget.extra.is_empty());
    }

    #[test]
    fn clamp_keeps_widget_on_screen() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let rect = Rect::from_min_size(pos2(780.0, -20.0), vec2(100.0, 50.0));
        let clamped = clamp_to_screen(rect, screen);
        assert_eq!(clamped.min, pos2(700.0, 0.0));
    }
}
