//! Zestaw kontrolek okna konfiguracji w stylu "grupowanych list":
//! zaokraglone grupy wierszy z cienkimi separatorami, wiersze nawigacyjne ze strzalka,
//! przelaczniki, kontrolki segmentowe i paleta kolorow.
//! Kolory pochodza z `Palette` zaleznej od motywu (jasny/ciemny).

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, Response, RichText, Sense,
    Shadow, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use crate::config::{PanelTheme, WidgetIcon};
use crate::widgets::paint_icon;

pub const ROW_HEIGHT: f32 = 40.0;
const GROUP_RADIUS: u8 = 12;
const ROW_PADDING_X: f32 = 14.0;

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub group: Color32,
    pub sheet: Color32,
    pub separator: Color32,
    pub label: Color32,
    pub secondary: Color32,
    pub accent: Color32,
    pub destructive: Color32,
    pub control_track: Color32,
    pub hover: Color32,
    pub backdrop_tint: Color32,
}

pub fn palette(theme: PanelTheme) -> Palette {
    match theme {
        PanelTheme::Dark => Palette {
            background: Color32::from_rgb(14, 14, 17),
            group: Color32::from_rgb(30, 30, 33),
            sheet: Color32::from_rgb(22, 22, 25),
            separator: Color32::from_white_alpha(18),
            label: Color32::from_rgb(242, 242, 247),
            secondary: Color32::from_rgb(142, 142, 150),
            accent: Color32::from_rgb(10, 132, 255),
            destructive: Color32::from_rgb(255, 69, 58),
            control_track: Color32::from_rgb(58, 58, 62),
            hover: Color32::from_white_alpha(10),
            backdrop_tint: Color32::from_black_alpha(110),
        },
        PanelTheme::Light => Palette {
            background: Color32::from_rgb(242, 242, 247),
            group: Color32::WHITE,
            sheet: Color32::from_rgb(242, 242, 247),
            separator: Color32::from_black_alpha(28),
            label: Color32::from_rgb(10, 10, 12),
            secondary: Color32::from_rgb(110, 110, 118),
            accent: Color32::from_rgb(0, 122, 255),
            destructive: Color32::from_rgb(255, 59, 48),
            control_track: Color32::from_rgb(220, 220, 226),
            hover: Color32::from_black_alpha(10),
            backdrop_tint: Color32::from_white_alpha(90),
        },
    }
}

/// Wielki tytul ekranu.
pub fn large_title(ui: &mut egui::Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).size(26.0).strong().color(pal.label));
}

/// Zaokraglona grupa wierszy z opcjonalnym naglowkiem (male szare wersaliki) i stopka-podpowiedzia.
pub fn group<R>(
    ui: &mut egui::Ui,
    pal: &Palette,
    header: Option<&str>,
    footer: Option<&str>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if let Some(header) = header {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(ROW_PADDING_X);
            ui.label(RichText::new(header.to_uppercase()).size(11.5).color(pal.secondary));
        });
        ui.add_space(2.0);
    }
    let inner = Frame::new()
        .fill(pal.group)
        .corner_radius(CornerRadius::same(GROUP_RADIUS))
        .inner_margin(Margin::symmetric(0, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add_contents(ui)
        })
        .inner;
    if let Some(footer) = footer {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(ROW_PADDING_X);
            ui.label(RichText::new(footer).size(11.5).color(pal.secondary));
        });
    }
    ui.add_space(14.0);
    inner
}

/// Cienka linia miedzy wierszami grupy (wcieta od lewej).
pub fn divider(ui: &mut egui::Ui, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline((rect.left() + ROW_PADDING_X)..=rect.right(), rect.center().y, Stroke::new(1.0, pal.separator));
}

/// Wiersz: etykieta po lewej, kontrolka wyrownana do prawej.
pub fn row<R>(ui: &mut egui::Ui, pal: &Palette, label: &str, add_control: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(vec2(width, ROW_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.add_space(ROW_PADDING_X);
        ui.label(RichText::new(label).color(pal.label));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(ROW_PADDING_X);
            add_control(ui)
        })
        .inner
    })
    .inner
}

/// Kafelek na poczatku wiersza: zaokraglony kwadrat w kolorze akcentu z biala ikona lub znakiem.
pub enum Leading {
    Icon(Color32, WidgetIcon),
    /// Kolo zebate (ustawienia) - ikona Phosphor.
    Gear(Color32),
}

fn paint_leading(painter: &egui::Painter, rect: Rect, leading: &Leading) {
    match leading {
        Leading::Icon(color, icon) => {
            painter.rect_filled(rect, CornerRadius::same(7), *color);
            if *icon != WidgetIcon::None {
                paint_icon(painter, rect.shrink(rect.width() * 0.17), *icon, Color32::WHITE);
            }
        }
        Leading::Gear(color) => {
            painter.rect_filled(rect, CornerRadius::same(7), *color);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                crate::icons::GEAR,
                FontId::new(rect.height() * 0.62, crate::icons::icon_family()),
                Color32::WHITE,
            );
        }
    }
}

/// Klikalny wiersz nawigacyjny: [kafelek] tytul ... szczegol ›
pub fn nav_row(
    ui: &mut egui::Ui,
    pal: &Palette,
    leading: Leading,
    title: &str,
    detail: &str,
    selected: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT + 4.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(8), pal.accent.gamma_multiply(0.18));
    } else if response.hovered() {
        painter.rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(8), pal.hover);
    }
    let tile = Rect::from_min_size(pos2(rect.left() + ROW_PADDING_X, rect.center().y - 14.0), Vec2::splat(28.0));
    paint_leading(painter, tile, &leading);
    let x = tile.right() + 12.0;
    let chevron_x = rect.right() - ROW_PADDING_X;
    painter.text(pos2(chevron_x, rect.center().y), Align2::RIGHT_CENTER, "›", FontId::proportional(20.0), pal.secondary);
    let detail_rect = painter.text(
        pos2(chevron_x - 16.0, rect.center().y),
        Align2::RIGHT_CENTER,
        detail,
        FontId::proportional(13.0),
        pal.secondary,
    );
    let title_clip = Rect::from_min_max(pos2(x, rect.top()), pos2(detail_rect.left() - 8.0, rect.bottom()));
    painter.with_clip_rect(title_clip).text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(14.5),
        pal.label,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Wiersz-przycisk z wysrodkowanym albo lewym tekstem (np. "Dodaj karte", "Usun").
pub fn action_row(ui: &mut egui::Ui, pal: &Palette, text: &str, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(8), pal.hover);
    }
    ui.painter().text(
        pos2(rect.left() + ROW_PADDING_X, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.5),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Przelacznik w ksztalcie kapsulki z animowana galka.
pub fn toggle(ui: &mut egui::Ui, pal: &Palette, on: &mut bool) -> Response {
    let size = vec2(46.0, 28.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.15);
    let track = pal.control_track.lerp_to_gamma(Color32::from_rgb(52, 199, 89), t);
    let radius = CornerRadius::same((size.y * 0.5) as u8);
    ui.painter().rect_filled(rect, radius, track);
    let knob_r = size.y * 0.5 - 2.5;
    let x = egui::lerp((rect.left() + knob_r + 2.5)..=(rect.right() - knob_r - 2.5), t);
    let center = pos2(x, rect.center().y);
    ui.painter().circle_filled(center + vec2(0.0, 1.0), knob_r, Color32::from_black_alpha(40));
    ui.painter().circle_filled(center, knob_r, Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Kontrolka segmentowa na cala szerokosc; zwraca true, gdy zmieniono wybor.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, pal: &Palette, value: &mut T, options: &[(T, &str)]) -> bool {
    let height = 32.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(9), pal.control_track.gamma_multiply(0.7));
    let segment_w = rect.width() / options.len() as f32;
    let mut changed = false;
    for (i, (option, label)) in options.iter().enumerate() {
        let seg = Rect::from_min_size(pos2(rect.left() + segment_w * i as f32, rect.top()), vec2(segment_w, height));
        let response = ui.interact(seg, ui.scope_id().with(("segment", i)), Sense::click());
        let selected = *value == *option;
        if selected {
            let knob = seg.shrink(2.0);
            painter.rect_filled(knob.translate(vec2(0.0, 1.0)), CornerRadius::same(7), Color32::from_black_alpha(25));
            painter.rect_filled(knob, CornerRadius::same(7), pal.group);
        } else if response.hovered() {
            painter.rect_filled(seg.shrink(2.0), CornerRadius::same(7), pal.hover);
        }
        painter.text(
            seg.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(13.0),
            if selected { pal.label } else { pal.secondary },
        );
        if response.clicked() && !selected {
            *value = *option;
            changed = true;
        }
    }
    changed
}

/// Paleta gotowych kolorow + wlasny kolor; zwraca true przy zmianie.
pub fn color_swatches(ui: &mut egui::Ui, pal: &Palette, color: &mut [u8; 3]) -> bool {
    const PRESETS: [[u8; 3]; 8] = [
        [72, 166, 255],
        [120, 226, 84],
        [255, 214, 10],
        [255, 150, 72],
        [255, 104, 112],
        [184, 132, 255],
        [100, 220, 210],
        [236, 236, 240],
    ];
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_space(ROW_PADDING_X);
        for preset in PRESETS {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
            let fill = Color32::from_rgb(preset[0], preset[1], preset[2]);
            ui.painter().circle_filled(rect.center(), 11.0, fill);
            if *color == preset {
                ui.painter().circle_stroke(rect.center(), 13.0, Stroke::new(2.0, pal.label));
            }
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *color = preset;
                changed = true;
            }
        }
        changed |= ui.color_edit_button_srgb(color).on_hover_text(crate::i18n::tr("Własny kolor", "Custom color")).changed();
    });
    ui.add_space(8.0);
    changed
}

/// Rama wyskakujacego okna: zaokraglona, z miekkim cieniem.
pub fn sheet_frame(pal: &Palette) -> Frame {
    Frame::new()
        .fill(pal.sheet)
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin::same(18))
        .stroke(Stroke::new(1.0, pal.separator))
        .shadow(Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(110) })
}

/// Przycisk nacisniety w naglowku wyskakujacego okna.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SheetButton {
    Cancel,
    Confirm,
}

/// Naglowek okna: opcjonalny przycisk anulowania po lewej, tytul na srodku,
/// przycisk zatwierdzenia (pogrubiony) po prawej.
pub fn sheet_header(
    ui: &mut egui::Ui,
    pal: &Palette,
    title: &str,
    cancel: Option<&str>,
    confirm: &str,
) -> Option<SheetButton> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, title, FontId::proportional(17.0), pal.label);
    let mut pressed = None;

    let mut header_button = |text: &str, align: Align2, strong: bool, button: SheetButton| {
        let font = FontId::proportional(if strong { 16.0 } else { 15.5 });
        let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), pal.accent);
        let anchor = if align == Align2::LEFT_CENTER { rect.left_center() } else { rect.right_center() };
        let hit = align.anchor_size(anchor, galley.size()).expand(6.0);
        let response = ui.interact(hit, ui.scope_id().with(("sheet_button", text)), Sense::click());
        let color = if response.hovered() { pal.accent.gamma_multiply(0.8) } else { pal.accent };
        ui.painter().text(anchor, align, text, font, color);
        if strong {
            // Zatwierdzenie wyrozniamy podwojnym napisem z przesunieciem o pol punktu (pseudo-pogrubienie).
            ui.painter().text(anchor + vec2(0.5, 0.0), align, text, FontId::proportional(16.0), color);
        }
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            pressed = Some(button);
        }
    };
    if let Some(cancel) = cancel {
        header_button(cancel, Align2::LEFT_CENTER, false, SheetButton::Cancel);
    }
    header_button(confirm, Align2::RIGHT_CENTER, true, SheetButton::Confirm);
    ui.add_space(12.0);
    pressed
}

/// Ramka podgladu karty (ciemny gradient jak tlo gry), zwraca prostokat wnetrza.
pub fn preview_stage(ui: &mut egui::Ui, height: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut mesh = egui::Mesh::default();
    let top = Color32::from_rgb(44, 72, 112);
    let bottom = Color32::from_rgb(14, 20, 32);
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    ui.painter().add(mesh);
    ui.painter()
        .rect_stroke(rect, CornerRadius::same(14), Stroke::new(1.0, Color32::from_white_alpha(20)), StrokeKind::Inside);
    ui.add_space(14.0);
    rect
}

/// Pigulka z przyciskami (np. kontrola powiekszenia). Zwraca indeks kliknietego przycisku.
pub fn pill_buttons(ui: &mut egui::Ui, pal: &Palette, labels: &[&str]) -> Option<usize> {
    let mut clicked = None;
    Frame::new()
        .fill(pal.group)
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::symmetric(4, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, label) in labels.iter().enumerate() {
                    let button = egui::Button::new(RichText::new(*label).color(pal.label))
                        .fill(Color32::TRANSPARENT)
                        .corner_radius(CornerRadius::same(12))
                        .min_size(vec2(32.0, 26.0));
                    if ui.add(button).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}
