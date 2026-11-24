//! Okno konfiguracji (zwykle okno na pulpicie, poza gra).
//!
//! Ekran glowny: po lewej podglad monitora (powiekszanie kolkiem/przyciskami,
//! przesuwanie widoku przeciaganiem tla), po prawej czysta lista w grupach:
//! karty, dodawanie karty, ustawienia, czujniki.
//! Wybranie pozycji (albo dwuklik karty na podgladzie) otwiera wyskakujace okno
//! nad rozmytym tlem; na gorze okna jest zywy podglad konfigurowanej karty.
//! Na podgladzie ekranu karty mozna przeciagac (przyciaganie do siatki, krawedzi
//! ekranu i innych kart), zmieniac im rozmiar uchwytem w rogu i przypinac.
//! Wspolrzedne kart sa zawsze w punktach ekranu - `egui::Scene` sam przelicza powiekszenie.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, Event, FontId, Frame, Id, Key, Margin, Mesh, Modifiers, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, Vec2, ViewportBuilder, ViewportId, pos2, vec2,
};

use super::app::{OverlayApp, WidgetAction, clamp_to_screen};
use super::blur::BackdropBlur;
use super::screenshot;
use super::hotkey::{HotkeySlot, hotkey_parts, hotkey_string, selectable_keys, split_hotkey};
use super::layout::{Guide, SnapSettings, snap_move, snap_resize};
use super::ui_kit::{self, Leading, Palette, SheetButton};
use crate::config::{
    self, Config, ExtraChart, MIN_WIDGET_SIZE, PanelTheme, WidgetConfig, WidgetFooter, WidgetIcon, WidgetKind, default_accent,
    default_icon,
};
use crate::i18n::{LanguageSetting, tr};
use crate::metrics::Metric;
use crate::widgets;

const RESIZE_HANDLE: f32 = 14.0;
const GUIDE_COLOR: Color32 = Color32::from_rgb(0, 214, 255);
const SIDEBAR_WIDTH: f32 = 340.0;
/// Szerokosc wyskakujacego okna: ~40% okna programu, nie mniej niz `SHEET_MIN_WIDTH`.
const SHEET_WIDTH_RATIO: f32 = 0.4;
const SHEET_MIN_WIDTH: f32 = 460.0;
const TOAST_DURATION: Duration = Duration::from_secs(5);
const ZOOM_STEP: f32 = 1.25;
/// Tytul okna konfiguracji - po nim `win32::capture_window` odnajduje okno do zrzutu.
/// Zalezy od jezyka; egui zmienia tytul okna po zmianie jezyka.
fn settings_title() -> &'static str {
    tr("ArtVitals — konfiguracja", "ArtVitals — settings")
}
/// Wysokosc wyskakujacego okna jako czesc wysokosci okna programu: co najmniej 70%,
/// przy dluzszej tresci rosnie do 90%, dalej tresc sie przewija.
const SHEET_MIN_HEIGHT_RATIO: f32 = 0.7;
const SHEET_MAX_HEIGHT_RATIO: f32 = 0.9;
/// Wysokosc naglowka i marginesow okna (poza przewijana trescia).
const SHEET_CHROME_HEIGHT: f32 = 78.0;
/// Podglad karty w oknie ustawien: margines, maks. powiekszenie i granice wysokosci ramki.
const PREVIEW_PADDING: f32 = 24.0;
const PREVIEW_MAX_ZOOM: f32 = 1.6;
const PREVIEW_MIN_HEIGHT: f32 = 170.0;
const PREVIEW_MAX_HEIGHT: f32 = 460.0;

pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("art_vitals_settings")
}

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    Resize,
}

struct DragState {
    id: u64,
    mode: DragMode,
    /// Prostokat karty (w punktach ekranu) w chwili zlapania.
    start: Rect,
    /// Suma przesuniec - przyciaganie liczone od pozycji startowej,
    /// zeby magnes nie "zjadal" drobnych ruchow myszy.
    delta: Vec2,
}

/// Co jest konfigurowane w wyskakujacym oknie.
#[derive(Clone, Copy, PartialEq)]
enum Sheet {
    Widget(u64),
    AddWidget,
    General,
    Sensors,
}

pub struct SettingsState {
    pub selected: Option<u64>,
    drag: Option<DragState>,
    guides: Vec<Guide>,
    recording_hotkey: Option<HotkeySlot>,
    /// Skroty ukladane "z klockow" (modyfikatory + klawisz), osobno dla kazdego skrotu.
    hotkey_drafts: [Option<(Modifiers, Option<Key>)>; 2],
    sheet: Option<Sheet>,
    blur: BackdropBlur,
    /// Stan powiekszenia i przesuniecia podgladu ekranu.
    view: PreviewView,
    add_metric: Metric,
    add_kind: WidgetKind,
    toast: Option<(String, bool, Instant)>,
    /// Prostokat podgladu z poprzedniej klatki - do wyliczenia procentu powiekszenia.
    last_stage: Option<Rect>,
    /// Licznik otwarc okna - nowe id przewijania przy kazdym otwarciu, zeby okno startowalo od gory.
    sheet_generation: u64,
    /// Stan karty z chwili otwarcia okna edycji - "Anuluj" go przywraca.
    edit_snapshot: Option<WidgetConfig>,
    /// Cala konfiguracja z chwili otwarcia ustawien ogolnych - "Anuluj" ja przywraca.
    general_snapshot: Option<Config>,
    /// Wczytany zrzut z gry: (nazwa pliku z konfiguracji, tekstura). Nazwa bez tekstury = blad odczytu.
    background: Option<(String, Option<egui::TextureHandle>)>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            selected: None,
            drag: None,
            guides: Vec::new(),
            recording_hotkey: None,
            hotkey_drafts: [None, None],
            sheet: None,
            blur: BackdropBlur::default(),
            view: PreviewView::default(),
            add_metric: Metric::CpuLoad,
            add_kind: WidgetKind::Graph,
            toast: None,
            last_stage: None,
            sheet_generation: 0,
            edit_snapshot: None,
            general_snapshot: None,
            background: None,
        }
    }
}

impl SettingsState {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn reset_interaction(&mut self) {
        self.drag = None;
        self.guides.clear();
        self.recording_hotkey = None;
        self.sheet = None;
        self.blur.clear();
    }

    fn open_sheet(&mut self, sheet: Sheet) {
        self.sheet = Some(sheet);
        self.sheet_generation += 1;
        self.hotkey_drafts = [None, None];
        self.recording_hotkey = None;
        // Zrzut do rozmycia robimy w nastepnym kroku, gdy klatka jest jeszcze bez okna.
        self.blur.clear();
    }

    fn close_sheet(&mut self) {
        self.sheet = None;
        self.edit_snapshot = None;
        self.general_snapshot = None;
        self.recording_hotkey = None;
        self.blur.clear();
    }
}

impl OverlayApp {
    pub(super) fn show_settings_window(&mut self, ctx: &egui::Context) {
        let builder = ViewportBuilder::default()
            .with_title(settings_title())
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([980.0, 620.0]);
        ctx.show_viewport_immediate(viewport_id(), builder, |ui, _class| {
            if ui.input(|i| i.viewport().close_requested()) {
                self.close_settings();
                return;
            }
            self.settings_ui(ui);
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = ui_kit::palette(self.config.panel_theme);
        if let Some((message, is_error)) = self.status.take() {
            self.settings.toast = Some((message, is_error, Instant::now()));
        }
        if self.settings.sheet.is_none() {
            self.handle_canvas_keyboard(&ctx);
        } else if self.settings.recording_hotkey.is_some() {
            self.handle_hotkey_recording(&ctx);
        }

        let mut actions = Vec::new();
        egui::Panel::right("sidebar")
            .resizable(false)
            .exact_size(SIDEBAR_WIDTH)
            .show_separator_line(false)
            .frame(Frame::new().fill(pal.background).inner_margin(Margin::symmetric(16, 18)))
            .show(ui, |ui| self.sidebar_ui(ui, &pal));
        egui::CentralPanel::default_margins()
            .frame(Frame::new().fill(pal.background).inner_margin(Margin { left: 18, right: 4, top: 18, bottom: 14 }))
            .show(ui, |ui| self.canvas_ui(ui, &pal, &mut actions));

        // Okno: pierwsza klatka po otwarciu jest bez okna (zrzut do rozmycia), potem tlo + okno.
        if let Some(sheet) = self.settings.sheet {
            if !self.settings.blur.is_captured() {
                self.settings.blur.capture(&ctx, settings_title());
            } else {
                self.settings.blur.paint(&ctx, pal.backdrop_tint);
                self.sheet_ui(&ctx, &pal, sheet, &mut actions);
            }
        }

        for action in actions {
            if let WidgetAction::Delete(id) = action
                && self.settings.sheet == Some(Sheet::Widget(id))
            {
                self.settings.close_sheet();
            }
            self.apply_action(action);
        }
    }

    // ---------------------------------------------------------------- klawiatura

    fn handle_canvas_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let Some(id) = self.settings.selected else { return };
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.settings.selected = None;
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Enter)) {
            self.settings.open_sheet(Sheet::Widget(id));
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Delete)) {
            self.apply_action(WidgetAction::Delete(id));
            return;
        }
        let grid = self.config.grid_size;
        let nudge = ctx.input(|i| {
            let step = if i.modifiers.shift { grid } else { 1.0 };
            let mut nudge = Vec2::ZERO;
            if i.key_pressed(Key::ArrowLeft) {
                nudge.x -= step;
            }
            if i.key_pressed(Key::ArrowRight) {
                nudge.x += step;
            }
            if i.key_pressed(Key::ArrowUp) {
                nudge.y -= step;
            }
            if i.key_pressed(Key::ArrowDown) {
                nudge.y += step;
            }
            nudge
        });
        if nudge != Vec2::ZERO {
            let screen = self.screen;
            if let Some(widget) = self.config.widgets.iter_mut().find(|w| w.id == id && !w.pinned) {
                let rect = Rect::from_min_size(widget.pos.into(), widget.size.into()).translate(nudge);
                widget.pos = clamp_to_screen(rect, screen).min.into();
                self.mark_dirty();
            }
        }
    }

    fn handle_hotkey_recording(&mut self, ctx: &egui::Context) {
        let Some(slot) = self.settings.recording_hotkey else { return };
        let captured = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, modifiers)) = captured else { return };
        if key == Key::Escape {
            self.settings.recording_hotkey = None;
            return;
        }
        // Same modyfikatory nie sa zdarzeniem Key, wiec tu zawsze mamy klawisz glowny.
        match hotkey_string(modifiers, key) {
            Some(text) => {
                self.settings.recording_hotkey = None;
                self.apply_hotkey(slot, text);
            }
            None => self.status = Some((tr("Skrót musi zawierać Ctrl, Alt lub Shift", "Shortcut must include Ctrl, Alt or Shift").to_owned(), true)),
        }
    }

    // ---------------------------------------------------------------- panel boczny

    fn sidebar_ui(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        ui.spacing_mut().scroll.floating = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui_kit::large_title(ui, pal, "ArtVitals");
                ui.label(RichText::new(format!("v{}", crate::VERSION)).size(12.0).color(pal.secondary));
            });
            let state = if self.config.overlay_visible {
                tr("Overlay aktywny", "Overlay active")
            } else {
                tr("Overlay ukryty", "Overlay hidden")
            };
            ui.label(
                RichText::new(format!("{state}  ·  {}", hotkey_parts(&self.config.overlay_hotkey).join("+")))
                    .size(13.0)
                    .color(pal.secondary),
            );
            ui.add_space(14.0);

            let mut open = None;
            ui_kit::group(ui, pal, Some(tr("Karty", "Cards")), None, |ui| {
                for widget in &self.config.widgets {
                    let icon = if widget.icon == WidgetIcon::None { default_icon(widget.metric) } else { widget.icon };
                    let detail = format!(
                        "{}{}",
                        if widget.pinned { "🔒 " } else { "" },
                        widget.kind.label()
                    );
                    let response = ui_kit::nav_row(
                        ui,
                        pal,
                        Leading::Icon(widgets::accent_color(widget), icon),
                        widget.display_title(),
                        &detail,
                        self.settings.selected == Some(widget.id),
                    );
                    if response.clicked() {
                        open = Some(Sheet::Widget(widget.id));
                        self.settings.selected = Some(widget.id);
                    }
                    ui_kit::divider(ui, pal);
                }
                if ui_kit::action_row(ui, pal, tr("+  Dodaj kartę", "+  Add card"), pal.accent).clicked() {
                    open = Some(Sheet::AddWidget);
                }
            });

            let mut changed = false;
            ui_kit::group(ui, pal, Some("Overlay"), Some(tr("Przytrzymaj Alt podczas przeciągania, żeby wyłączyć przyciąganie.", "Hold Alt while dragging to disable snapping.")), |ui| {
                changed |= ui_kit::row(ui, pal, tr("Widoczny w grze", "Visible in game"), |ui| ui_kit::toggle(ui, pal, &mut self.config.overlay_visible)).changed();
                ui_kit::divider(ui, pal);
                changed |= ui_kit::row(ui, pal, tr("Przyciąganie do siatki", "Snap to grid"), |ui| ui_kit::toggle(ui, pal, &mut self.config.snap_to_grid)).changed();
                ui_kit::divider(ui, pal);
                changed |= ui_kit::row(ui, pal, tr("Przyciąganie do krawędzi", "Snap to edges"), |ui| ui_kit::toggle(ui, pal, &mut self.config.snap_to_edges)).changed();
            });
            if changed {
                self.mark_dirty();
            }

            let issues = self.sensors.lock().issues.len();
            ui_kit::group(ui, pal, Some(tr("Ustawienia", "Settings")), None, |ui| {
                if ui_kit::nav_row(ui, pal, Leading::Gear(Color32::from_rgb(142, 142, 150)), tr("Ogólne", "General"), "", false).clicked() {
                    open = Some(Sheet::General);
                }
                ui_kit::divider(ui, pal);
                let detail = match issues {
                    0 => tr("Wszystko działa", "All good").to_owned(),
                    1 => "1 problem".to_owned(),
                    n => format!("{n} {}", tr("problemy", "problems")),
                };
                if ui_kit::nav_row(
                    ui,
                    pal,
                    Leading::Icon(Color32::from_rgb(255, 149, 0), WidgetIcon::Thermometer),
                    tr("Czujniki", "Sensors"),
                    &detail,
                    false,
                )
                .clicked()
                {
                    open = Some(Sheet::Sensors);
                }
            });

            ui_kit::group(ui, pal, None, None, |ui| {
                if ui_kit::action_row(ui, pal, tr("Folder konfiguracji", "Config folder"), pal.accent).clicked()
                    && let Err(err) = std::process::Command::new("explorer").arg(config::config_dir()).spawn()
                {
                    self.status = Some((format!("{} {err}", tr("Nie można otworzyć folderu:", "Can't open the folder:")), true));
                }
                ui_kit::divider(ui, pal);
                if ui_kit::action_row(ui, pal, tr("Zamknij program", "Quit"), pal.destructive).clicked() {
                    let ctx = ui.ctx().clone();
                    self.quit(&ctx);
                }
            });

            if let Some(sheet) = open {
                self.settings.open_sheet(sheet);
            }
        });
    }

    // ---------------------------------------------------------------- podglad ekranu

    fn canvas_ui(&mut self, ui: &mut egui::Ui, pal: &Palette, actions: &mut Vec<WidgetAction>) {
        let screen = self.screen;
        let mut zoom_command = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Podgląd ekranu", "Screen preview")).size(20.0).strong().color(pal.label));
            ui.add_space(12.0);
            let percent = format!("{:.0}%", self.preview_zoom() * 100.0);
            zoom_command = ui_kit::pill_buttons(ui, pal, &["−", &percent, "+", tr("Dopasuj", "Fit")]);
            ui.add_space(12.0);
            self.background_menu(ui, pal);
        });
        ui.add_space(10.0);

        let hint_height = 22.0;
        let available = ui.available_rect_before_wrap();
        let stage = Rect::from_min_max(available.min, pos2(available.max.x, available.max.y - hint_height));
        self.settings.last_stage = Some(stage);
        let fit = fit_zoom(stage, screen);
        if self.settings.view.center.is_none() {
            self.settings.view.center = Some(screen.center());
        }
        // Przyciski +/- przyblizaja na zaznaczona karte (albo wszystkie karty) i ustawiaja ja
        // na srodku podgladu - srodek ekranu zwykle jest pusty, a karty siedza w rogach.
        let focus = self
            .config
            .widgets
            .iter()
            .filter(|w| self.settings.selected.is_none_or(|id| id == w.id))
            .map(|w| Rect::from_min_size(w.pos.into(), w.size.into()))
            .reduce(|a, b| a.union(b));
        let view = &mut self.settings.view;
        match zoom_command {
            Some(0) => view.zoom_onto(stage, fit, focus, 1.0 / ZOOM_STEP),
            Some(2) => view.zoom_onto(stage, fit, focus, ZOOM_STEP),
            Some(_) => view.reset(screen),
            None => {}
        }
        // Kolko myszy (lub Ctrl+kolko / gest) nad podgladem powieksza wzgledem kursora.
        let interactive = self.settings.sheet.is_none();
        if interactive && let Some(pointer) = ui.ctx().pointer_hover_pos().filter(|p| stage.contains(*p)) {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll * 0.0015).exp();
            if (factor - 1.0).abs() > f32::EPSILON {
                view.zoom_about(stage, fit, pointer, factor);
            }
        }
        view.clamp_to_screen(stage, fit, screen);
        let transform = view.transform(stage, fit);

        let mut stage_ui = ui.new_child(egui::UiBuilder::new().max_rect(stage));
        stage_ui.set_clip_rect(stage);
        let background = stage_ui.interact(stage, Id::unique("stage_background"), Sense::click_and_drag());
        if background.clicked() {
            self.settings.selected = None;
        }
        if background.double_clicked() {
            self.settings.view.reset(screen);
        }
        if background.dragged() {
            // Przeciaganie tla przesuwa widok.
            if let Some(center) = &mut self.settings.view.center {
                *center -= background.drag_delta() / transform.zoom;
            }
            self.settings.view.clamp_to_screen(stage, fit, screen);
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        let transform = self.settings.view.transform(stage, fit);

        let painter = stage_ui.painter().clone();
        painter.rect_filled(stage, CornerRadius::same(16), Color32::from_rgb(10, 12, 16));
        paint_screen_backdrop(&painter, transform.rect(screen), self.config.grid_size * 4.0 * transform.zoom);
        if let Some(texture) = self.background_texture(ui.ctx()) {
            paint_screenshot(&painter, transform.rect(screen), &texture);
        }
        self.canvas_widgets(&mut stage_ui, &painter, transform, actions);
        painter.rect_stroke(stage, CornerRadius::same(16), Stroke::new(1.0, pal.separator), StrokeKind::Inside);

        ui.allocate_rect(stage, Sense::hover());
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                tr(
                    "Przeciągnij kartę · róg = rozmiar · dwuklik karty = ustawienia · kółko = powiększenie · przeciągnij tło = przesuń widok",
                    "Drag a card · corner = resize · double-click a card = settings · wheel = zoom · drag background = pan",
                ),
            )
            .size(12.0)
            .color(pal.secondary),
        );
        self.paint_toast(ui, stage);
    }

    /// Menu "Tlo" nad podgladem: zrzut z gry z pliku lub schowka, powrot do tla domyslnego.
    fn background_menu(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let dir = config::config_dir();
        let mut stored: Option<Result<String, String>> = None;
        ui.menu_button(RichText::new(tr("Tło: zrzut z gry", "Background: game screenshot")).color(pal.accent), |ui| {
            if ui.button(tr("Wczytaj zrzut z pliku…", "Load screenshot from file…")).clicked() {
                ui.close();
                if let Some(path) = screenshot::pick_image_file() {
                    // Najpierw sprawdzamy, czy obraz sie dekoduje - dopiero potem kopiujemy.
                    stored = Some(screenshot::load_image(&path).and_then(|_| screenshot::store_file(&path, &dir)));
                }
            }
            if ui.button(tr("Wklej ze schowka", "Paste from clipboard"))
                .on_hover_text(tr("Zrzut w grze: PrintScreen albo Win+Shift+S", "In game: PrintScreen or Win+Shift+S")).clicked() {
                ui.close();
                stored = Some(screenshot::clipboard_image().and_then(|image| screenshot::store_image(&image, &dir)));
            }
            if !self.config.preview_background.is_empty() && ui.button(tr("Usuń tło", "Remove background")).clicked() {
                ui.close();
                screenshot::clear(&dir);
                self.config.preview_background.clear();
                self.settings.background = None;
                self.mark_dirty();
            }
        });
        match stored {
            Some(Ok(name)) => {
                self.config.preview_background = name;
                // Ta sama nazwa pliku moze miec nowa zawartosc - wymuszamy ponowne wczytanie.
                self.settings.background = None;
                self.mark_dirty();
                self.status = Some((tr("Wczytano zrzut jako tło podglądu", "Screenshot loaded as preview background").to_owned(), false));
            }
            Some(Err(err)) => self.status = Some((err, true)),
            None => {}
        }
    }

    /// Tekstura zrzutu z gry (wczytywana raz, ponownie po zmianie pliku w konfiguracji).
    fn background_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let name = &self.config.preview_background;
        if name.is_empty() {
            self.settings.background = None;
            return None;
        }
        if self.settings.background.as_ref().is_none_or(|(loaded, _)| loaded != name) {
            let texture = match screenshot::load_image(&config::config_dir().join(name)) {
                Ok(image) => Some(ctx.load_texture("preview_background", image, egui::TextureOptions::LINEAR)),
                Err(err) => {
                    log::warn!("Tlo podgladu: {err}");
                    self.status = Some((err, true));
                    None
                }
            };
            self.settings.background = Some((name.clone(), texture));
        }
        self.settings.background.as_ref().and_then(|(_, texture)| texture.clone())
    }

    fn preview_zoom(&self) -> f32 {
        let stage = self.settings.last_stage.unwrap_or(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0)));
        fit_zoom(stage, self.screen) * self.settings.view.zoom
    }

    /// Karty na podgladzie: obsluga myszy (przeciaganie, rozmiar, menu) i rysowanie w skali.
    fn canvas_widgets(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        transform: ViewTransform,
        actions: &mut Vec<WidgetAction>,
    ) {
        let screen = self.screen;
        let snap = self.snap_settings(ui);
        let rects: Vec<Rect> = self
            .config
            .widgets
            .iter()
            .map(|w| Rect::from_min_size(w.pos.into(), w.size.into()))
            .collect();
        let mut hovered_id = None;

        for index in 0..self.config.widgets.len() {
            let (id, pinned, kind, metric) = {
                let w = &self.config.widgets[index];
                (w.id, w.pinned, w.kind, w.metric)
            };
            let rect = rects[index];
            let view = transform.rect(rect);
            let egui_id = Id::unique(("canvas_widget", id));
            let body = ui.interact(view, egui_id, Sense::click_and_drag());
            let handle_rect = Rect::from_min_size(view.max - Vec2::splat(RESIZE_HANDLE), Vec2::splat(RESIZE_HANDLE));
            let handle = (!pinned).then(|| ui.interact(handle_rect, egui_id.with("resize"), Sense::drag()));
            if body.hovered() || handle.as_ref().is_some_and(|h| h.hovered()) {
                hovered_id = Some(id);
            }

            if body.clicked() || body.drag_started() || body.secondary_clicked() {
                self.settings.selected = Some(id);
            }
            if body.double_clicked() {
                self.settings.open_sheet(Sheet::Widget(id));
            }
            if pinned {
                if body.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
            } else {
                if body.drag_started() {
                    self.settings.drag = Some(DragState { id, mode: DragMode::Move, start: rect, delta: Vec2::ZERO });
                }
                if let Some(handle) = &handle {
                    if handle.drag_started() {
                        self.settings.selected = Some(id);
                        self.settings.drag = Some(DragState { id, mode: DragMode::Resize, start: rect, delta: Vec2::ZERO });
                    }
                    if handle.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
                    }
                }
                if self.settings.drag.as_ref().is_some_and(|d| d.id == id) {
                    let view_delta = body.drag_delta() + handle.as_ref().map(|h| h.drag_delta()).unwrap_or_default();
                    let others: Vec<Rect> = rects
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != index)
                        .map(|(_, r)| *r)
                        .collect();
                    if let Some(drag) = &mut self.settings.drag {
                        drag.delta += view_delta / transform.zoom;
                        let (snapped, guides) = match drag.mode {
                            DragMode::Move => snap_move(drag.start.translate(drag.delta), &others, screen, snap),
                            DragMode::Resize => snap_resize(
                                Rect::from_min_max(drag.start.min, drag.start.max + drag.delta),
                                Vec2::from(MIN_WIDGET_SIZE),
                                &others,
                                screen,
                                snap,
                            ),
                        };
                        let snapped = clamp_to_screen(snapped, screen);
                        let widget = &mut self.config.widgets[index];
                        widget.pos = snapped.min.into();
                        widget.size = snapped.size().into();
                        self.settings.guides = guides;
                        ui.ctx().set_cursor_icon(match drag.mode {
                            DragMode::Move => CursorIcon::Grabbing,
                            DragMode::Resize => CursorIcon::ResizeNwSe,
                        });
                    }
                    if body.drag_stopped() || handle.as_ref().is_some_and(|h| h.drag_stopped()) {
                        self.settings.drag = None;
                        self.settings.guides.clear();
                        self.mark_dirty();
                    }
                } else if body.hovered() && !handle.as_ref().is_some_and(|h| h.hovered()) {
                    ui.ctx().set_cursor_icon(CursorIcon::Grab);
                }
            }

            body.context_menu(|ui| widget_context_menu(ui, id, pinned, kind, metric, actions));
        }

        // Rysowanie po obsludze myszy, zeby karta podazala za kursorem bez opoznienia klatki.
        // Karty rysujemy od razu w docelowej skali - tekst zostaje ostry przy kazdym powiekszeniu.
        let ctx = ui.ctx().clone();
        for widget in &self.config.widgets {
            let view = transform.rect(Rect::from_min_size(widget.pos.into(), widget.size.into()));
            self.paint_widget_at(&ctx, painter, widget, view, transform.zoom);
            paint_selection(
                painter,
                view,
                widgets::accent_color(widget),
                widget.pinned,
                self.settings.selected == Some(widget.id),
                hovered_id == Some(widget.id),
            );
        }
        let stroke = Stroke::new(1.5, GUIDE_COLOR);
        let screen_view = transform.rect(screen);
        for guide in &self.settings.guides {
            match *guide {
                Guide::Vertical(x) => painter.vline(transform.x(x), screen_view.y_range(), stroke),
                Guide::Horizontal(y) => painter.hline(screen_view.x_range(), transform.y(y), stroke),
            };
        }
    }

    fn snap_settings(&self, ui: &egui::Ui) -> SnapSettings {
        let free = ui.input(|i| i.modifiers.alt);
        SnapSettings {
            grid: (self.config.snap_to_grid && !free).then_some(self.config.grid_size),
            edge_distance: (self.config.snap_to_edges && !free).then_some(self.config.snap_distance),
            gap: self.config.grid_size,
        }
    }

    fn paint_toast(&mut self, ui: &egui::Ui, stage: Rect) {
        let Some((message, is_error, since)) = &self.settings.toast else { return };
        if since.elapsed() > TOAST_DURATION {
            self.settings.toast = None;
            return;
        }
        ui.ctx().request_repaint_after(Duration::from_millis(250));
        let painter = ui.painter();
        let font = FontId::proportional(13.5);
        let color = if *is_error { Color32::from_rgb(255, 120, 110) } else { Color32::from_rgb(120, 220, 150) };
        let galley = painter.layout(message.clone(), font, color, (stage.width() - 120.0).max(200.0));
        let size = galley.size() + vec2(32.0, 16.0);
        let rect = Rect::from_center_size(pos2(stage.center().x, stage.bottom() - 20.0 - size.y * 0.5), size);
        painter.rect_filled(rect, CornerRadius::same((size.y * 0.5) as u8), Color32::from_black_alpha(210));
        painter.galley(rect.center() - galley.size() * 0.5, galley, color);
    }

    // ---------------------------------------------------------------- wyskakujace okna

    fn sheet_ui(&mut self, ctx: &egui::Context, pal: &Palette, sheet: Sheet, actions: &mut Vec<WidgetAction>) {
        let window_height = ctx.content_rect().height();
        let window_width = ctx.content_rect().width();
        let sheet_width = (window_width * SHEET_WIDTH_RATIO).max(SHEET_MIN_WIDTH).min(window_width - 80.0);
        let min_height = window_height * SHEET_MIN_HEIGHT_RATIO - SHEET_CHROME_HEIGHT;
        let max_height = (window_height * SHEET_MAX_HEIGHT_RATIO - SHEET_CHROME_HEIGHT).max(200.0);
        let modal = egui::Modal::new(Id::unique("settings_sheet"))
            .frame(ui_kit::sheet_frame(pal))
            .backdrop_color(Color32::TRANSPARENT)
            .show(ctx, |ui| {
                ui.set_width(sheet_width);
                let title = match sheet {
                    Sheet::Widget(id) => self
                        .config
                        .widgets
                        .iter()
                        .find(|w| w.id == id)
                        .map(|w| w.display_title().to_owned())
                        .unwrap_or_default(),
                    Sheet::AddWidget => tr("Nowa karta", "New card").to_owned(),
                    Sheet::General => tr("Ogólne", "General").to_owned(),
                    Sheet::Sensors => tr("Czujniki", "Sensors").to_owned(),
                };
                // Nowa karta: "Anuluj" / "Dodaj"; edycja karty: "Anuluj" (cofa) / "Gotowe" (zatwierdza);
                // ustawienia ogolne tez (zmiany widac na zywo, "Anuluj" przywraca stan z otwarcia);
                // czujniki sa tylko do odczytu, wiec maja samo "Gotowe".
                let (cancel, confirm) = match sheet {
                    Sheet::AddWidget => (Some(tr("Anuluj", "Cancel")), tr("Dodaj", "Add")),
                    Sheet::Widget(_) | Sheet::General => (Some(tr("Anuluj", "Cancel")), tr("Gotowe", "Done")),
                    Sheet::Sensors => (None, tr("Gotowe", "Done")),
                };
                let pressed = ui_kit::sheet_header(ui, pal, &title, cancel, confirm);
                // min_scrolled_height gwarantuje minimum, gdy tresc jest dluzsza niz miejsce w oknie;
                // set_min_height w srodku - gdy tresc jest krotka.
                // Pasek przewijania z wlasnym miejscem - plywajacy zaslanial kontrolki po prawej.
                ui.spacing_mut().scroll.floating = false;
                egui::ScrollArea::vertical()
                    .id_salt(("sheet_scroll", self.settings.sheet_generation))
                    .max_height(max_height)
                    .min_scrolled_height(min_height.min(max_height))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_min_height(min_height.min(max_height));
                    match sheet {
                        Sheet::Widget(id) => self.widget_sheet(ui, pal, id, actions),
                        Sheet::AddWidget => self.add_widget_sheet(ui, pal),
                        Sheet::General => self.general_sheet(ui, pal),
                        Sheet::Sensors => self.sensors_sheet(ui, pal),
                    }
                });
                pressed
            });
        // Esc albo klik obok okna: edycja zostaje (bez utraty zmian), nowa karta nie jest dodawana.
        let pressed = modal.inner;
        let dismissed = pressed.is_none() && modal.should_close();
        match (sheet, pressed) {
            (Sheet::AddWidget, Some(SheetButton::Confirm)) => {
                actions.push(WidgetAction::Add(self.settings.add_metric, self.settings.add_kind));
                self.settings.close_sheet();
            }
            (Sheet::Widget(id), Some(SheetButton::Cancel)) => {
                self.restore_snapshot(id);
                self.settings.close_sheet();
            }
            (Sheet::General, Some(SheetButton::Cancel)) => {
                if let Some(snapshot) = self.settings.general_snapshot.take()
                    && snapshot != self.config
                {
                    // apply_config ponownie rejestruje skroty, ustawia motyw i czestotliwosc odczytow.
                    self.apply_config(ctx, snapshot);
                    self.mark_dirty();
                }
                self.settings.close_sheet();
            }
            (_, Some(_)) => self.settings.close_sheet(),
            (_, None) if dismissed => self.settings.close_sheet(),
            _ => {}
        }
    }

    /// Cofa zmiany karty do stanu z chwili otwarcia okna edycji.
    fn restore_snapshot(&mut self, id: u64) {
        let Some(snapshot) = self.settings.edit_snapshot.take() else { return };
        if snapshot.id != id {
            return;
        }
        if let Some(widget) = self.config.widgets.iter_mut().find(|w| w.id == id)
            && *widget != snapshot
        {
            *widget = snapshot;
            self.mark_dirty();
        }
    }

    fn widget_sheet(&mut self, ui: &mut egui::Ui, pal: &Palette, id: u64, actions: &mut Vec<WidgetAction>) {
        let Some(index) = self.config.widgets.iter().position(|w| w.id == id) else {
            ui.label(tr("Ta karta już nie istnieje.", "This card no longer exists."));
            return;
        };
        if self.settings.edit_snapshot.as_ref().is_none_or(|s| s.id != id) {
            self.settings.edit_snapshot = Some(self.config.widgets[index].clone());
        }

        // Zywy podglad konfigurowanej karty.
        let preview = self.config.widgets[index].clone();
        let stage = ui_kit::preview_stage(ui, preview_stage_height(ui.available_width(), &preview));
        self.paint_preview(ui, stage, &preview);

        let mut changed = false;
        let mut new_metric = None;
        let mut new_kind = preview.kind;
        let mut apply_background_to_all = None;
        let screen = self.screen;
        {
            let w = &mut self.config.widgets[index];

            ui_kit::group(
                ui,
                pal,
                Some(tr("Metryki", "Metrics")),
                Some(match w.kind {
                    WidgetKind::Graph => tr(
                        "Główna metryka jest największa i ma progi kolorów. Każdą dodatkową możesz pokazać na wykresie głównym (linia we własnej skali), na osobnym wykresie albo jako samą wartość.",
                        "The main metric is the largest and has color thresholds. Each extra one can be shown on the main graph (a line in its own scale), on a separate graph, or as a value only.",
                    ),
                    WidgetKind::Bar => tr(
                        "Główna metryka jest największa i ma progi kolorów. Dodatkowe pokazują się jako mniejsze paski pod głównym.",
                        "The main metric is the largest and has color thresholds. Extra metrics are shown as smaller bars below the main one.",
                    ),
                    WidgetKind::Gauge => tr(
                        "Główna metryka jest największa i ma progi kolorów. Dodatkowe pokazują się jako wewnętrzne pierścienie wskaźnika.",
                        "The main metric is the largest and has color thresholds. Extra metrics are shown as inner rings of the gauge.",
                    ),
                    WidgetKind::Number => tr(
                        "Główna metryka jest największa i ma progi kolorów. Dodatkowe pokazują się jako wartości z prawej.",
                        "The main metric is the largest and has color thresholds. Extra metrics are shown as values on the right.",
                    ),
                    WidgetKind::Fans => tr(
                        "Karta pokazuje obroty wszystkich wykrytych wentylatorów, pompy i temperaturę cieczy AIO. Poniżej wybierzesz, które pokazać.",
                        "The card shows the speed of all detected fans and pumps, and the AIO liquid temperature. Choose which ones to show below.",
                    ),
                }),
                |ui| {
                    ui_kit::row(ui, pal, tr("Główna", "Main"), |ui| {
                        egui::ComboBox::from_id_salt("metric").selected_text(w.metric.label()).show_ui(ui, |ui| {
                            for m in Metric::ALL {
                                if ui.selectable_label(m == w.metric, m.label()).clicked() && m != w.metric {
                                    new_metric = Some(m);
                                }
                            }
                        });
                    });
                    let is_graph = w.kind == WidgetKind::Graph;
                    let mut remove = None;
                    let mut set_chart = None;
                    for (i, extra) in w.extra.iter().enumerate() {
                        ui_kit::divider(ui, pal);
                        ui_kit::row(ui, pal, extra.metric.label(), |ui| {
                            let button = egui::Button::new(RichText::new(tr("Usuń", "Remove")).color(pal.destructive))
                                .fill(Color32::TRANSPARENT);
                            if ui.add(button).clicked() {
                                remove = Some(i);
                            }
                            if is_graph {
                                egui::ComboBox::from_id_salt(("extra_chart", i))
                                    .selected_text(extra.chart.label())
                                    .show_ui(ui, |ui| {
                                        for chart in ExtraChart::ALL {
                                            if ui.selectable_label(chart == extra.chart, chart.label()).clicked()
                                                && chart != extra.chart
                                            {
                                                set_chart = Some((i, chart));
                                            }
                                        }
                                    });
                            }
                            // Kolorowa kropka = kolor linii/paska tej metryki na karcie.
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 5.0, widgets::metric_color(extra.metric));
                        });
                    }
                    if let Some(i) = remove {
                        w.remove_extra(i);
                        changed = true;
                    }
                    if let Some((i, chart)) = set_chart {
                        w.set_extra_chart(i, chart);
                        changed = true;
                    }
                    let available = w.available_extra();
                    if !available.is_empty() {
                        ui_kit::divider(ui, pal);
                        padded(ui, |ui| {
                            ui.menu_button(RichText::new(tr("+  Dodaj metrykę", "+  Add metric")).color(pal.accent), |ui| {
                                for metric in available {
                                    if is_graph {
                                        // Na wykresie od razu pytamy, jak pokazac metryke.
                                        ui.menu_button(metric.label(), |ui| {
                                            for chart in ExtraChart::ALL {
                                                if ui.button(chart.label()).clicked() {
                                                    w.add_extra(metric, chart);
                                                    changed = true;
                                                    ui.close();
                                                }
                                            }
                                        });
                                    } else if ui.button(metric.label()).clicked() {
                                        w.add_extra(metric, ExtraChart::default());
                                        changed = true;
                                        ui.close();
                                    }
                                }
                            });
                        });
                    }
                },
            );

            if w.kind == WidgetKind::Fans {
                let data = self.sensors.lock();
                let rows = widgets::fan_rows(&data, None);
                drop(data);
                ui_kit::group(
                    ui,
                    pal,
                    Some(tr("Wentylatory", "Fans")),
                    Some(tr(
                        "Wentylatory płyty wymagają PawnIO i uruchomienia jako administrator. AIO są czytane przez USB - program producenta (NZXT CAM, iCUE) może je blokować.",
                        "Motherboard fans require PawnIO and running as administrator. AIOs are read over USB - the vendor app (NZXT CAM, iCUE) may block them.",
                    )),
                    |ui| {
                        if rows.is_empty() {
                            padded(ui, |ui| {
                                ui.label(RichText::new(tr("Nie wykryto wentylatorów.", "No fans detected.")).color(pal.secondary));
                            });
                        }
                        for (i, row) in rows.iter().enumerate() {
                            if i > 0 {
                                ui_kit::divider(ui, pal);
                            }
                            let mut shown = !w.hidden_fans.contains(&row.id);
                            if ui_kit::row(ui, pal, &row.name, |ui| ui_kit::toggle(ui, pal, &mut shown)).changed() {
                                if shown {
                                    w.hidden_fans.retain(|id| *id != row.id);
                                } else {
                                    w.hidden_fans.push(row.id.clone());
                                }
                                changed = true;
                            }
                        }
                    },
                );
            }

            let kinds = WidgetKind::for_metric(w.metric);
            if kinds.len() > 1 {
                ui_kit::group(ui, pal, Some(tr("Typ karty", "Card type")), None, |ui| {
                    padded(ui, |ui| {
                        let options: Vec<(WidgetKind, &str)> = kinds.iter().map(|k| (*k, k.label())).collect();
                        ui_kit::segmented(ui, pal, &mut new_kind, &options);
                    });
                });
            }

            ui_kit::group(ui, pal, Some(tr("Wygląd", "Appearance")), None, |ui| {
                ui_kit::row(ui, pal, tr("Ikona", "Icon"), |ui| {
                    egui::ComboBox::from_id_salt("icon").selected_text(w.icon.label()).show_ui(ui, |ui| {
                        for icon in WidgetIcon::ALL {
                            changed |= ui.selectable_value(&mut w.icon, icon, icon.label()).changed();
                        }
                    });
                });
                if w.kind != WidgetKind::Fans {
                    ui_kit::divider(ui, pal);
                    ui_kit::row(ui, pal, tr("Stopka", "Footer"), |ui| {
                        egui::ComboBox::from_id_salt("footer").selected_text(w.footer.label()).show_ui(ui, |ui| {
                            for footer in WidgetFooter::ALL.into_iter().filter(|f| f.fits(w.metric)) {
                                changed |= ui.selectable_value(&mut w.footer, footer, footer.label()).changed();
                            }
                        });
                    });
                }
                ui_kit::divider(ui, pal);
                changed |= ui_kit::row(ui, pal, tr("Pokaż tytuł", "Show title"), |ui| ui_kit::toggle(ui, pal, &mut w.show_title)).changed();
                if w.show_title {
                    ui_kit::divider(ui, pal);
                    ui_kit::row(ui, pal, tr("Tytuł", "Title"), |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut w.title)
                                    .hint_text(w.metric.short_label())
                                    .char_limit(64)
                                    .desired_width(180.0),
                            )
                            .changed();
                    });
                }
                ui_kit::divider(ui, pal);
                ui_kit::row(ui, pal, tr("Kolor akcentu", "Accent color"), |ui| {
                    if ui.small_button(tr("Domyślny", "Default")).clicked() {
                        w.accent = default_accent(w.metric);
                        changed = true;
                    }
                });
                changed |= ui_kit::color_swatches(ui, pal, &mut w.accent);
                ui_kit::divider(ui, pal);
                ui_kit::row(ui, pal, tr("Tło karty", "Card background"), |ui| {
                    changed |= ui
                        .add(egui::Slider::new(&mut w.background_opacity, 0.0..=1.0).show_value(false))
                        .changed();
                });
                ui_kit::divider(ui, pal);
                ui_kit::row(ui, pal, tr("Kolor tła", "Background color"), |ui| {
                    if ui.small_button(tr("Wszystkim kartom", "All cards"))
                        .on_hover_text(tr("Ustaw ten kolor i krycie tła na wszystkich kartach", "Apply this background color and opacity to all cards")).clicked() {
                        apply_background_to_all = Some((w.background_color, w.background_opacity));
                    }
                    if ui.small_button(tr("Domyślny", "Default")).clicked() {
                        w.background_color = config::DEFAULT_CARD_BACKGROUND;
                        changed = true;
                    }
                });
                changed |= ui_kit::color_swatches_from(ui, pal, &mut w.background_color, &ui_kit::BACKGROUND_PRESETS);
            });

            let unit = w.metric.unit();
            let (min, max) = w.metric.range();
            // Progi dotycza tylko metryki glownej - nazwa metryki w naglowku, zeby bylo jasne czego.
            let thresholds_header = format!("{} {}", tr("Progi kolorów:", "Color thresholds:"), w.metric.label());
            if w.metric.has_thresholds() {
                let thresholds_footer = format!(
                    "{} {} {} {}",
                    tr("Gdy", "When"),
                    lowercase_first(w.metric.label()),
                    if w.metric.higher_is_worse() {
                        tr("przekroczy wartość", "exceeds")
                    } else {
                        tr("spadnie poniżej", "drops below")
                    },
                    tr(
                        "progu ostrzeżenia, liczba na karcie robi się żółta, a po progu krytycznym czerwona. Dodatkowe metryki mają stałe progi.",
                        "the warning threshold, the number on the card turns yellow, and past the critical threshold red. Extra metrics have fixed thresholds.",
                    ),
                );
                let (warn_label, crit_label) = if w.metric.higher_is_worse() {
                    (tr("Ostrzeżenie (żółty) od", "Warning (yellow) from"), tr("Krytyczny (czerwony) od", "Critical (red) from"))
                } else {
                    (tr("Ostrzeżenie (żółty) poniżej", "Warning (yellow) below"), tr("Krytyczny (czerwony) poniżej", "Critical (red) below"))
                };
                ui_kit::group(ui, pal, Some(&thresholds_header), Some(&thresholds_footer), |ui| {
                    ui_kit::row(ui, pal, warn_label, |ui| {
                        changed |= ui
                            .add(egui::DragValue::new(&mut w.warn_threshold).range(min..=max).speed(0.5).suffix(unit))
                            .changed();
                    });
                    ui_kit::divider(ui, pal);
                    ui_kit::row(ui, pal, crit_label, |ui| {
                        changed |= ui
                            .add(egui::DragValue::new(&mut w.crit_threshold).range(min..=max).speed(0.5).suffix(unit))
                            .changed();
                    });
                });
            } else {
                // Zegary nie maja progow - wysoki zegar to normalna praca, nie ostrzezenie.
                ui_kit::group(ui, pal, Some(&thresholds_header), None, |ui| {
                    padded(ui, |ui| {
                        ui.label(
                            RichText::new(tr(
                                "Zegary nie zmieniają koloru - wysoki zegar to normalna praca, nie ostrzeżenie.",
                                "Clocks don't change color - a high clock is normal operation, not a warning.",
                            ))
                                .color(pal.secondary),
                        );
                    });
                });
            }
            // Krytyczny prog musi lezec "dalej" niz ostrzezenie (wyzej dla temperatur, nizej dla FPS).
            if w.metric.higher_is_worse() {
                w.crit_threshold = w.crit_threshold.max(w.warn_threshold);
            } else {
                w.crit_threshold = w.crit_threshold.min(w.warn_threshold);
            }

            ui_kit::group(
                ui,
                pal,
                Some(tr("Położenie", "Placement")),
                Some(tr("Przypiętej karty nie da się przesunąć ani zmienić jej rozmiaru.", "A pinned card can't be moved or resized.")),
                |ui| {
                    changed |= ui_kit::row(ui, pal, tr("Przypięta", "Pinned"), |ui| ui_kit::toggle(ui, pal, &mut w.pinned)).changed();
                    ui_kit::divider(ui, pal);
                    let enabled = !w.pinned;
                    ui_kit::row(ui, pal, tr("Pozycja", "Position"), |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            changed |= ui.add(egui::DragValue::new(&mut w.pos[1]).prefix("Y ")).changed();
                            changed |= ui.add(egui::DragValue::new(&mut w.pos[0]).prefix("X ")).changed();
                        });
                    });
                    ui_kit::divider(ui, pal);
                    ui_kit::row(ui, pal, tr("Rozmiar", "Size"), |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            changed |= ui
                                .add(egui::DragValue::new(&mut w.size[1]).prefix("H ").range(MIN_WIDGET_SIZE[1]..=2000.0))
                                .changed();
                            changed |= ui
                                .add(egui::DragValue::new(&mut w.size[0]).prefix("W ").range(MIN_WIDGET_SIZE[0]..=2000.0))
                                .changed();
                        });
                    });
                },
            );
            if changed {
                let rect = clamp_to_screen(Rect::from_min_size(w.pos.into(), w.size.into()), screen);
                w.pos = rect.min.into();
            }
        }

        ui_kit::group(ui, pal, None, None, |ui| {
            if ui_kit::action_row(ui, pal, tr("Duplikuj kartę", "Duplicate card"), pal.accent).clicked() {
                actions.push(WidgetAction::Duplicate(id));
            }
            ui_kit::divider(ui, pal);
            if ui_kit::action_row(ui, pal, tr("Przenieś na wierzch", "Bring to front"), pal.accent).clicked() {
                actions.push(WidgetAction::BringToFront(id));
            }
            ui_kit::divider(ui, pal);
            if ui_kit::action_row(ui, pal, tr("Usuń kartę", "Delete card"), pal.destructive).clicked() {
                actions.push(WidgetAction::Delete(id));
            }
        });

        if let Some(m) = new_metric {
            actions.push(WidgetAction::SetMetric(id, m));
        }
        if let Some((color, opacity)) = apply_background_to_all {
            for widget in &mut self.config.widgets {
                widget.background_color = color;
                widget.background_opacity = opacity;
            }
            changed = true;
        }
        if new_kind != preview.kind {
            actions.push(WidgetAction::SetKind(id, new_kind));
        }
        if changed {
            self.mark_dirty();
        }
    }

    fn add_widget_sheet(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let mut sample = config::suggested_widget(0, self.settings.add_metric, self.settings.add_kind, [0.0, 0.0]);
        sample.id = u64::MAX;
        let stage = ui_kit::preview_stage(ui, preview_stage_height(ui.available_width(), &sample));
        self.paint_preview(ui, stage, &sample);

        let kinds = WidgetKind::for_metric(self.settings.add_metric);
        if !kinds.contains(&self.settings.add_kind) {
            self.settings.add_kind = kinds[0];
        }
        if kinds.len() > 1 {
            ui_kit::group(ui, pal, Some(tr("Typ karty", "Card type")), None, |ui| {
                padded(ui, |ui| {
                    let options: Vec<(WidgetKind, &str)> = kinds.iter().map(|k| (*k, k.label())).collect();
                    ui_kit::segmented(ui, pal, &mut self.settings.add_kind, &options);
                });
            });
        }
        ui_kit::group(ui, pal, Some(tr("Co pokazać", "What to show")), None, |ui| {
            for (i, metric) in Metric::ALL.into_iter().enumerate() {
                if i > 0 {
                    ui_kit::divider(ui, pal);
                }
                let [r, g, b] = config::default_accent(metric);
                let selected = self.settings.add_metric == metric;
                let response = ui_kit::nav_row(
                    ui,
                    pal,
                    Leading::Icon(Color32::from_rgb(r, g, b), default_icon(metric)),
                    metric.label(),
                    &if selected { crate::icons::CHECK.to_string() } else { String::new() },
                    selected,
                );
                if response.clicked() {
                    self.settings.add_metric = metric;
                }
            }
        });
    }

    fn general_sheet(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let ctx = ui.ctx().clone();
        if self.settings.general_snapshot.is_none() {
            self.settings.general_snapshot = Some(self.config.clone());
        }
        let mut changed = false;
        let mut timing = false;

        self.hotkey_group(ui, pal, HotkeySlot::Overlay);
        self.hotkey_group(ui, pal, HotkeySlot::Settings);

        ui_kit::group(ui, pal, Some("Overlay"), None, |ui| {
            ui_kit::row(ui, pal, tr("Krycie", "Opacity"), |ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut self.config.overlay_opacity, 0.1..=1.0).show_value(false))
                    .changed();
            });
            ui_kit::divider(ui, pal);
            ui_kit::row(ui, pal, tr("Powiększenie tekstu", "Text size boost"), |ui| {
                // Karty rosna razem z tekstem (o miejsce zajete przez napisy), wykresy zostaja.
                let mut boost = self.config.font_boost;
                if ui
                    .add(egui::Slider::new(&mut boost, 0..=config::MAX_FONT_BOOST).prefix("+").suffix(" pt"))
                    .changed()
                {
                    self.config.set_font_boost(boost);
                    changed = true;
                }
            });
            ui_kit::divider(ui, pal);
            changed |= ui_kit::row(ui, pal, tr("Płynne animacje", "Smooth animations"), |ui| ui_kit::toggle(ui, pal, &mut self.config.animations)).changed();
        });

        ui_kit::group(ui, pal, Some(tr("Dane", "Data")), None, |ui| {
            ui_kit::row(ui, pal, tr("Odświeżanie", "Refresh interval"), |ui| {
                timing |= ui
                    .add(egui::Slider::new(&mut self.config.sample_interval_ms, 250..=5000).suffix(" ms").step_by(50.0))
                    .changed();
            });
            ui_kit::divider(ui, pal);
            ui_kit::row(ui, pal, tr("Historia wykresu", "Graph history"), |ui| {
                timing |= ui.add(egui::Slider::new(&mut self.config.history_seconds, 10..=600).suffix(" s")).changed();
            });
        });

        ui_kit::group(
            ui,
            pal,
            Some(tr("Pomiar FPS", "FPS measurement")),
            Some(tr(
                "Średni FPS oraz 1% i 0.1% low są liczone z ostatnich N sekund gry. Krótsze okno szybciej reaguje na zacięcia i ich koniec, dłuższe daje stabilniejszy wynik (jak w benchmarkach).",
                "Average FPS and 1% / 0.1% lows are calculated from the last N seconds of gameplay. A shorter window reacts faster to stutters and their end, a longer one gives a steadier result (like in benchmarks).",
            )),
            |ui| {
                ui_kit::row(ui, pal, tr("Okno średniej i lows", "Average & lows window"), |ui| {
                    timing |= ui
                        .add(egui::Slider::new(&mut self.config.fps_stats_seconds, 2..=120).suffix(" s"))
                        .changed();
                });
            },
        );

        ui_kit::group(ui, pal, Some(tr("Układanie kart", "Card layout")), None, |ui| {
            ui_kit::row(ui, pal, tr("Rozmiar siatki", "Grid size"), |ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut self.config.grid_size, 2.0..=64.0).suffix(" pt").step_by(1.0))
                    .changed();
            });
            ui_kit::divider(ui, pal);
            ui_kit::row(ui, pal, tr("Zasięg przyciągania", "Snap distance"), |ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut self.config.snap_distance, 0.0..=64.0).suffix(" pt").step_by(1.0))
                    .changed();
            });
        });

        let mut theme = self.config.panel_theme;
        ui_kit::group(ui, pal, Some(tr("Okno konfiguracji", "Settings window")), None, |ui| {
            changed |= ui_kit::row(ui, pal, tr("Otwieraj przy starcie", "Open on startup"), |ui| {
                ui_kit::toggle(ui, pal, &mut self.config.show_settings_on_start)
            })
            .changed();
            ui_kit::divider(ui, pal);
            padded(ui, |ui| {
                ui_kit::segmented(
                    ui,
                    pal,
                    &mut theme,
                    &[(PanelTheme::Dark, tr("Ciemny", "Dark")), (PanelTheme::Light, tr("Jasny", "Light"))],
                );
            });
        });

        // Jezyk: naglowek w obu jezykach, zeby dalo sie go znalezc po pomylkowej zmianie.
        let mut language = self.config.language;
        ui_kit::group(ui, pal, Some("Język / Language"), None, |ui| {
            padded(ui, |ui| {
                let options: Vec<(LanguageSetting, &str)> = LanguageSetting::ALL.iter().map(|l| (*l, l.label())).collect();
                ui_kit::segmented(ui, pal, &mut language, &options);
            });
        });
        if language != self.config.language {
            self.config.language = language;
            self.config.apply_language();
            changed = true;
        }

        if theme != self.config.panel_theme {
            self.set_panel_theme(&ctx, theme);
        }
        if timing {
            self.timing_changed();
        }
        if changed {
            self.mark_dirty();
        }
    }

    /// Sekcja jednego skrotu: aktualne klocki, nagrywanie i ukladanie z klockow.
    fn hotkey_group(&mut self, ui: &mut egui::Ui, pal: &Palette, slot: HotkeySlot) {
        let (title, footer) = match slot {
            HotkeySlot::Overlay => (
                tr("Pokaż / ukryj overlay w grze", "Show / hide overlay in game"),
                tr("Działa także w grze. Musi zawierać Ctrl, Alt lub Shift.", "Works in game too. Must include Ctrl, Alt or Shift."),
            ),
            HotkeySlot::Settings => (
                tr("Okno konfiguracji", "Settings window"),
                tr("Otwiera i zamyka to okno. Musi zawierać Ctrl, Alt lub Shift.", "Opens and closes this window. Must include Ctrl, Alt or Shift."),
            ),
        };
        let current = self.hotkey_text(slot).to_owned();
        let index = slot_index(slot);
        let draft = &mut self.settings.hotkey_drafts[index];
        if draft.is_none() {
            *draft = Some(split_hotkey(&current));
        }
        let mut apply = None;

        ui_kit::group(ui, pal, Some(title), Some(footer), |ui| {
            ui_kit::row(ui, pal, tr("Aktualny skrót", "Current shortcut"), |ui| {
                keycaps(ui, pal, &hotkey_parts(&current));
            });
            ui_kit::divider(ui, pal);
            ui_kit::row(ui, pal, tr("Nagraj z klawiatury", "Record from keyboard"), |ui| {
                let recording = self.settings.recording_hotkey == Some(slot);
                let text = if recording {
                    RichText::new(tr("Naciśnij skrót…  (Esc = anuluj)", "Press a shortcut…  (Esc = cancel)")).color(pal.accent)
                } else {
                    RichText::new(tr("Nagraj", "Record")).color(pal.accent)
                };
                if ui.button(text).clicked() {
                    self.settings.recording_hotkey = if recording { None } else { Some(slot) };
                }
            });
            ui_kit::divider(ui, pal);
            padded(ui, |ui| {
                ui.label(RichText::new(tr("Ułóż z klocków", "Build from keys")).color(pal.label));
                ui.add_space(6.0);
                let Some((modifiers, key)) = &mut self.settings.hotkey_drafts[index] else { return };
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    keycap_toggle(ui, pal, "Ctrl", &mut modifiers.ctrl);
                    keycap_toggle(ui, pal, "Alt", &mut modifiers.alt);
                    keycap_toggle(ui, pal, "Shift", &mut modifiers.shift);
                    ui.label(RichText::new("+").size(16.0).color(pal.secondary));
                    let selected = key.map(|k| k.symbol_or_name().to_owned()).unwrap_or_else(|| tr("Klawisz", "Key").to_owned());
                    egui::ComboBox::from_id_salt(("hotkey_key", index))
                        .selected_text(selected)
                        .height(320.0)
                        .show_ui(ui, |ui| {
                            for candidate in selectable_keys() {
                                ui.selectable_value(key, Some(candidate), candidate.symbol_or_name());
                            }
                        });
                });
                ui.add_space(8.0);
                let built = key.and_then(|k| hotkey_string(*modifiers, k));
                ui.horizontal(|ui| {
                    match &built {
                        Some(text) => keycaps(ui, pal, &hotkey_parts(text)),
                        None => {
                            ui.label(RichText::new(tr("Wybierz co najmniej jeden z Ctrl/Alt/Shift i klawisz", "Pick at least one of Ctrl/Alt/Shift and a key")).color(pal.secondary));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_apply = built.as_ref().is_some_and(|text| *text != current);
                        if ui.add_enabled(can_apply, egui::Button::new(tr("Ustaw skrót", "Set shortcut"))).clicked() {
                            apply = built.clone();
                        }
                    });
                });
            });
        });

        if let Some(text) = apply {
            self.apply_hotkey(slot, text);
        }
    }

    fn hotkey_text(&self, slot: HotkeySlot) -> &str {
        match slot {
            HotkeySlot::Overlay => &self.config.overlay_hotkey,
            HotkeySlot::Settings => &self.config.settings_hotkey,
        }
    }

    fn apply_hotkey(&mut self, slot: HotkeySlot, text: String) {
        match self.hotkeys.set(slot, &text) {
            Ok(()) => {
                self.status = Some((format!("{} {}", tr("Nowy skrót:", "New shortcut:"), hotkey_parts(&text).join(" + ")), false));
                self.settings.hotkey_drafts[slot_index(slot)] = Some(split_hotkey(&text));
                match slot {
                    HotkeySlot::Overlay => self.config.overlay_hotkey = text,
                    HotkeySlot::Settings => self.config.settings_hotkey = text,
                }
                self.mark_dirty();
            }
            Err(err) => self.status = Some((err, true)),
        }
    }

    fn sensors_sheet(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let data = self.sensors.lock();
        let footer = format!(
            "{}{}",
            data.gpu_name.as_deref().unwrap_or(tr("Brak karty NVIDIA", "No NVIDIA card")),
            match &data.fps_process {
                Some(process) => format!(
                    "  ·  {} {process} ({} {} s)",
                    tr("FPS mierzone dla:", "FPS measured for:"),
                    tr("średnia i lows z ostatnich", "average and lows from the last"),
                    self.config.fps_stats_seconds
                ),
                None => format!("  ·  {}", tr("FPS: brak aktywnej gry (DirectX 9-12)", "FPS: no active game (DirectX 9-12)")),
            }
        );
        ui_kit::group(ui, pal, Some(tr("Odczyty na żywo", "Live readings")), Some(&footer), |ui| {
            for (i, metric) in Metric::ALL.into_iter().enumerate() {
                if i > 0 {
                    ui_kit::divider(ui, pal);
                }
                ui_kit::row(ui, pal, metric.label(), |ui| match data.value(metric) {
                    Some(v) => ui.label(RichText::new(format!("{v:.*}{}", metric.decimals(), metric.unit())).color(pal.secondary)),
                    None => ui.label(RichText::new("—").color(pal.secondary)),
                });
            }
        });
        if !data.issues.is_empty() {
            ui_kit::group(
                ui,
                pal,
                Some(tr("Problemy", "Issues")),
                Some(tr("Temperatura CPU wymaga uruchomienia programu jako administrator.", "CPU temperature requires running the program as administrator.")),
                |ui| {
                    for (i, (source, issue)) in data.issues.iter().enumerate() {
                        if i > 0 {
                            ui_kit::divider(ui, pal);
                        }
                        padded(ui, |ui| {
                            ui.label(RichText::new(*source).strong().color(pal.label));
                            ui.label(RichText::new(issue).color(Color32::from_rgb(255, 159, 10)));
                        });
                    }
                },
            );
        }
    }

    /// Rysuje karte wysrodkowana w ramce podgladu, jak najwieksza (do 160%).
    fn paint_preview(&self, ui: &egui::Ui, stage: Rect, widget: &WidgetConfig) {
        let inner = stage.shrink(PREVIEW_PADDING);
        let zoom = (inner.width() / widget.size[0]).min(inner.height() / widget.size[1]).min(PREVIEW_MAX_ZOOM);
        let rect = Rect::from_center_size(stage.center(), Vec2::from(widget.size) * zoom);
        let painter = ui.painter().with_clip_rect(stage);
        self.paint_widget_at(ui.ctx(), &painter, widget, rect, zoom);
    }
}

/// "Obciazenie CPU" -> "obciazenie CPU" (skroty jak CPU/GPU/VRAM zostaja wielkimi literami,
/// takze na poczatku: "CPU load" zostaje bez zmian).
fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if !chars.clone().next().is_some_and(char::is_uppercase) => first.to_lowercase().chain(chars).collect(),
        _ => text.to_owned(),
    }
}

/// Wysokosc ramki podgladu dopasowana do karty: karta pokazana w powiekszeniu do 160%
/// (ograniczonym szerokoscia okna), ramka rosnie razem z karta, gdy dochodza metryki.
fn preview_stage_height(available_width: f32, widget: &WidgetConfig) -> f32 {
    let zoom = ((available_width - 2.0 * PREVIEW_PADDING) / widget.size[0]).min(PREVIEW_MAX_ZOOM);
    (widget.size[1] * zoom + 2.0 * PREVIEW_PADDING).clamp(PREVIEW_MIN_HEIGHT, PREVIEW_MAX_HEIGHT)
}

fn padded(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add_contents(ui);
    });
}

/// Powiekszenie, przy ktorym caly ekran miesci sie w podgladzie (z marginesem).
fn fit_zoom(stage: Rect, screen: Rect) -> f32 {
    ((stage.width() / screen.width()).min(stage.height() / screen.height()) * 0.96).max(0.01)
}

/// Stan podgladu: powiekszenie wzgledem "dopasuj" i punkt ekranu w srodku podgladu.
struct PreviewView {
    zoom: f32,
    center: Option<Pos2>,
}

impl Default for PreviewView {
    fn default() -> Self {
        Self { zoom: 1.0, center: None }
    }
}

impl PreviewView {
    fn reset(&mut self, screen: Rect) {
        self.zoom = 1.0;
        self.center = Some(screen.center());
    }

    /// Powiekszenie wzgledem punktu `anchor` (w pikselach podgladu) - punkt pod kursorem zostaje na miejscu.
    fn zoom_about(&mut self, stage: Rect, fit: f32, anchor: Pos2, factor: f32) {
        let Some(center) = self.center else { return };
        let old = fit * self.zoom;
        let world = center + (anchor - stage.center()) / old;
        self.zoom = (self.zoom * factor).clamp(0.5, 8.0);
        let new = fit * self.zoom;
        self.center = Some(world - (anchor - stage.center()) / new);
    }

    /// Zmiana powiekszenia z wycentrowaniem widoku na `focus` (prostokat w punktach ekranu).
    fn zoom_onto(&mut self, stage: Rect, fit: f32, focus: Option<Rect>, factor: f32) {
        match focus {
            Some(focus) => {
                self.zoom = (self.zoom * factor).clamp(0.5, 8.0);
                self.center = Some(focus.center());
            }
            None => self.zoom_about(stage, fit, stage.center(), factor),
        }
    }

    /// Nie pozwala przewinac widoku poza monitor: na osi, na ktorej ekran miesci sie w calosci,
    /// trzyma go na srodku, a na pozostalych - krawedz ekranu przy krawedzi podgladu.
    fn clamp_to_screen(&mut self, stage: Rect, fit: f32, screen: Rect) {
        let Some(center) = &mut self.center else { return };
        let half_view = stage.size() / (fit * self.zoom) * 0.5;
        for axis in 0..2 {
            let (min, max) = (screen.min[axis] + half_view[axis], screen.max[axis] - half_view[axis]);
            center[axis] = if min > max { screen.center()[axis] } else { center[axis].clamp(min, max) };
        }
    }

    fn transform(&self, stage: Rect, fit: f32) -> ViewTransform {
        ViewTransform { origin: stage.center(), center: self.center.unwrap_or_default(), zoom: fit * self.zoom }
    }
}

/// Przeliczenie wspolrzednych ekranu na piksele podgladu.
#[derive(Clone, Copy)]
struct ViewTransform {
    origin: Pos2,
    center: Pos2,
    zoom: f32,
}

impl ViewTransform {
    fn x(&self, x: f32) -> f32 {
        self.origin.x + (x - self.center.x) * self.zoom
    }

    fn y(&self, y: f32) -> f32 {
        self.origin.y + (y - self.center.y) * self.zoom
    }

    fn rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(pos2(self.x(r.min.x), self.y(r.min.y)), pos2(self.x(r.max.x), self.y(r.max.y)))
    }
}

fn slot_index(slot: HotkeySlot) -> usize {
    match slot {
        HotkeySlot::Overlay => 0,
        HotkeySlot::Settings => 1,
    }
}

/// Skrot narysowany jako rzad klockow-klawiszy: [Ctrl] + [Shift] + [O].
fn keycaps(ui: &mut egui::Ui, pal: &Palette, parts: &[String]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        // W wierszu wyrownanym do prawej elementy ukladaja sie od prawej - odwracamy kolejnosc,
        // zeby skrot zawsze czytal sie "Ctrl + Shift + O".
        let ordered: Vec<&String> = if ui.layout().prefer_right_to_left() {
            parts.iter().rev().collect()
        } else {
            parts.iter().collect()
        };
        for (i, part) in ordered.into_iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("+").color(pal.secondary));
            }
            keycap(ui, pal, part, false);
        }
    });
}

fn keycap(ui: &mut egui::Ui, pal: &Palette, text: &str, active: bool) -> egui::Response {
    let font = FontId::proportional(13.0);
    let color = if active { Color32::WHITE } else { pal.label };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let size = vec2((galley.size().x + 18.0).max(30.0), 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let fill = if active { pal.accent } else { pal.group };
    // Dolna krawedz ciemniejsza - klocek wyglada jak klawisz.
    painter.rect_filled(rect.translate(vec2(0.0, 2.0)), CornerRadius::same(7), pal.separator.gamma_multiply(3.0));
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, pal.separator), StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() * 0.5, galley, color);
    response
}

/// Klocek-modyfikator do wlaczania/wylaczania klikiem.
fn keycap_toggle(ui: &mut egui::Ui, pal: &Palette, text: &str, on: &mut bool) {
    if keycap(ui, pal, text, *on).on_hover_cursor(CursorIcon::PointingHand).clicked() {
        *on = !*on;
    }
}

fn widget_context_menu(
    ui: &mut egui::Ui,
    id: u64,
    pinned: bool,
    kind: WidgetKind,
    metric: Metric,
    actions: &mut Vec<WidgetAction>,
) {
    if ui.button(if pinned { tr("Odepnij", "Unpin") } else { tr("Przypnij", "Pin") }).clicked() {
        actions.push(WidgetAction::TogglePin(id));
        ui.close();
    }
    ui.menu_button(tr("Typ karty", "Card type"), |ui| {
        for option in WidgetKind::for_metric(metric) {
            if ui.selectable_label(option == kind, option.label()).clicked() {
                actions.push(WidgetAction::SetKind(id, option));
                ui.close();
            }
        }
    });
    ui.menu_button(tr("Metryka", "Metric"), |ui| {
        for option in Metric::ALL {
            if ui.selectable_label(option == metric, option.label()).clicked() {
                actions.push(WidgetAction::SetMetric(id, option));
                ui.close();
            }
        }
    });
    ui.separator();
    if ui.button(tr("Duplikuj", "Duplicate")).clicked() {
        actions.push(WidgetAction::Duplicate(id));
        ui.close();
    }
    if ui.button(tr("Na wierzch", "Bring to front")).clicked() {
        actions.push(WidgetAction::BringToFront(id));
        ui.close();
    }
    if ui.button(tr("Usuń", "Delete")).clicked() {
        actions.push(WidgetAction::Delete(id));
        ui.close();
    }
}

/// Tlo monitora na podgladzie: ciemny gradient udajacy ekran gry i kropki siatki.
/// Zrzut z gry w prostokacie ekranu - wypelnia go w calosci (przy innych proporcjach przyciety
/// po srodku), lekko przyciemniony, zeby karty pozostaly czytelne.
fn paint_screenshot(painter: &egui::Painter, screen: Rect, texture: &egui::TextureHandle) {
    let [w, h] = texture.size().map(|v| v as f32);
    let (screen_aspect, image_aspect) = (screen.width() / screen.height(), w / h);
    let uv = if image_aspect > screen_aspect {
        let span = screen_aspect / image_aspect;
        Rect::from_min_max(pos2((1.0 - span) * 0.5, 0.0), pos2((1.0 + span) * 0.5, 1.0))
    } else {
        let span = image_aspect / screen_aspect;
        Rect::from_min_max(pos2(0.0, (1.0 - span) * 0.5), pos2(1.0, (1.0 + span) * 0.5))
    };
    painter.image(texture.id(), screen, uv, Color32::from_gray(225));
}

fn paint_screen_backdrop(painter: &egui::Painter, screen: Rect, dot_spacing: f32) {
    let mut mesh = Mesh::default();
    let top = Color32::from_rgb(40, 66, 102);
    let bottom = Color32::from_rgb(14, 20, 32);
    mesh.colored_vertex(screen.left_top(), top);
    mesh.colored_vertex(screen.right_top(), top);
    mesh.colored_vertex(screen.left_bottom(), bottom);
    mesh.colored_vertex(screen.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(mesh);

    let dot = Color32::from_white_alpha(20);
    let mut y = screen.top();
    while y <= screen.bottom() {
        let mut x = screen.left();
        while x <= screen.right() {
            painter.circle_filled(pos2(x, y), 1.2, dot);
            x += dot_spacing.max(8.0);
        }
        y += dot_spacing.max(8.0);
    }
    painter.rect_stroke(screen, CornerRadius::same(6), Stroke::new(2.0, Color32::from_white_alpha(50)), StrokeKind::Outside);
}

fn paint_selection(painter: &egui::Painter, rect: Rect, accent: Color32, pinned: bool, selected: bool, hovered: bool) {
    let radius = CornerRadius::same(9);
    if selected {
        painter.rect_stroke(rect.expand(3.0), radius, Stroke::new(2.5, accent), StrokeKind::Outside);
    } else if hovered {
        painter.rect_stroke(rect.expand(2.0), radius, Stroke::new(1.5, Color32::from_white_alpha(150)), StrokeKind::Outside);
    }

    if pinned {
        paint_pin_badge(painter, pos2(rect.right() - 10.0, rect.top() + 10.0), accent);
    } else if selected || hovered {
        // Uchwyt zmiany rozmiaru: trzy ukosne kreski w prawym dolnym rogu.
        let corner = rect.max - vec2(3.0, 3.0);
        let stroke = Stroke::new(1.8, Color32::from_white_alpha(200));
        for i in 1..=3 {
            let d = i as f32 * 3.5;
            painter.line_segment([corner - vec2(d, 0.0), corner - vec2(0.0, d)], stroke);
        }
    }
}

/// Znaczek przypiecia (klodka) w rogu karty.
fn paint_pin_badge(painter: &egui::Painter, center: Pos2, color: Color32) {
    painter.circle_filled(center, 8.0, Color32::from_black_alpha(190));
    let body = Rect::from_center_size(center + vec2(0.0, 1.5), vec2(7.0, 5.0));
    painter.rect_filled(body, CornerRadius::same(1), color);
    let shackle = Rect::from_center_size(center + vec2(0.0, -2.0), vec2(4.5, 5.5));
    painter.rect_stroke(shackle, CornerRadius::same(2), Stroke::new(1.3, color), StrokeKind::Middle);
}
