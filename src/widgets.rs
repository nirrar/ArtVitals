//! Rysowanie kart widgetow (wykres, wskaznik, pasek, liczba) prosto na Painterze egui.
//!
//! Styl: polprzezroczysta granatowa karta z cienka ramka, duza kolorowa wartosc,
//! opcjonalna druga wartosc z prawej, wykres z gradientowym wypelnieniem i stopka.
//! Wszystkie wymiary i czcionki skaluja sie wspolczynnikiem `scale` liczonym
//! z rozmiaru karty wzgledem domyslnego - mala karta = mniejsze elementy.
//! Kolor wartosci przechodzi plynnie od akcentu przez zolty do czerwieni wg progow widgetu.

use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Mesh, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use crate::config::{self, ExtraChart, SEPARATE_CHART_HEIGHT, WidgetConfig, WidgetFooter, WidgetIcon, WidgetKind};
use crate::metrics::Metric;
use crate::sensors::SensorData;

const CARD_FILL: Color32 = Color32::from_rgb(12, 22, 38);
const CARD_BORDER: Color32 = Color32::from_rgb(70, 98, 136);
const PLOT_FILL: Color32 = Color32::from_rgb(8, 16, 28);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(236, 242, 250);
const TEXT_MUTED: Color32 = Color32::from_rgb(140, 158, 182);
const WARN_COLOR: Color32 = Color32::from_rgb(255, 196, 64);
const CRIT_COLOR: Color32 = Color32::from_rgb(255, 72, 72);

/// Wartosci gotowe do narysowania (ewentualnie juz animowane przez wywolujacego).
pub struct WidgetValues {
    pub primary: Option<f32>,
    /// Dodatkowe metryki karty w kolejnosci z konfiguracji.
    pub extras: Vec<(Metric, Option<f32>)>,
}

pub struct PaintContext<'a> {
    pub painter: &'a Painter,
    pub data: &'a SensorData,
    /// Globalny mnoznik krycia overlaya.
    pub opacity: f32,
    /// Docelowa liczba probek historii (szerokosc osi X wykresu).
    pub history_len: usize,
    /// Powiekszenie: 1.0 na overlayu, mniej w podgladzie okna konfiguracji.
    pub zoom: f32,
}

pub fn accent_color(widget: &WidgetConfig) -> Color32 {
    let [r, g, b] = widget.accent;
    Color32::from_rgb(r, g, b)
}

/// Kolor wartosci zalezny od progow - plynne przejscie zamiast skoku.
pub fn value_color(widget: &WidgetConfig, value: f32) -> Color32 {
    let accent = accent_color(widget);
    // Dla metryk "nizej = gorzej" (FPS) odwracamy znak - wtedy ta sama logika dziala dla obu kierunkow.
    let sign = if widget.metric.higher_is_worse() { 1.0 } else { -1.0 };
    let (value, warn, crit) = (value * sign, widget.warn_threshold * sign, widget.crit_threshold * sign);
    let fade = ((crit - warn) * 0.25).max(1.0);
    if value >= crit {
        CRIT_COLOR
    } else if value >= warn {
        let t = ((value - warn) / (crit - warn).max(0.01)).clamp(0.0, 1.0);
        WARN_COLOR.lerp_to_gamma(CRIT_COLOR, t * t)
    } else if value > warn - fade {
        accent.lerp_to_gamma(WARN_COLOR, (value - (warn - fade)) / fade)
    } else {
        accent
    }
}

fn alpha(color: Color32, opacity: f32) -> Color32 {
    color.gamma_multiply(opacity.clamp(0.0, 1.0))
}

/// Kolor dodatkowej metryki - wg jej domyslnych progow (karta ma progi tylko dla glownej).
fn extra_color(metric: Metric, value: Option<f32>) -> Color32 {
    let Some(v) = value else { return TEXT_MUTED };
    let (warn, crit) = metric.default_thresholds();
    let sign = if metric.higher_is_worse() { 1.0 } else { -1.0 };
    let (v, warn, crit) = (v * sign, warn * sign, crit * sign);
    if v >= crit {
        CRIT_COLOR
    } else if v >= warn {
        WARN_COLOR
    } else {
        TEXT_PRIMARY
    }
}

fn extra_text(metric: Metric, value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{}{}", format_value(metric, Some(v)), metric.unit()),
        None => "—".to_owned(),
    }
}

/// Dodatkowa wartosc do wyswietlenia; `legend` = metryka ma linie na wykresie glownym,
/// wiec przed etykieta rysujemy kropke w kolorze tej linii.
#[derive(Clone, Copy)]
struct ExtraValue {
    metric: Metric,
    value: Option<f32>,
    legend: bool,
}

/// Etykieta (+ kropka legendy) i wartosc w jednej linii; zwraca nic - rysuje od prawej krawedzi.
fn paint_extra_line(pc: &PaintContext<'_>, right: Pos2, extra: ExtraValue, value_size: f32, scale: f32) {
    let op = pc.opacity;
    let value_rect = pc.painter.text(
        right,
        Align2::RIGHT_CENTER,
        extra_text(extra.metric, extra.value),
        FontId::proportional(value_size),
        alpha(extra_color(extra.metric, extra.value), op),
    );
    let label_rect = pc.painter.text(
        pos2(value_rect.left() - 5.0 * scale, right.y),
        Align2::RIGHT_CENTER,
        extra.metric.short_label(),
        FontId::proportional(9.0 * scale),
        alpha(TEXT_MUTED, op),
    );
    if extra.legend {
        pc.painter.circle_filled(
            pos2(label_rect.left() - 5.0 * scale, right.y),
            2.6 * scale,
            alpha(metric_color(extra.metric), op),
        );
    }
}

/// Kolumna dodatkowych metryk wyrownana do prawej: jedna = na wysokosci glownej liczby,
/// kilka = wiersze od gory (tyle, ile sie zmiesci).
fn paint_extras_column(pc: &PaintContext<'_>, area: Rect, extras: &[ExtraValue], value_center_y: f32, scale: f32) {
    match extras {
        [] => {}
        [extra] => paint_extra_line(pc, pos2(area.right(), value_center_y), *extra, 12.0 * scale, scale),
        extras => {
            let line_h = 13.0 * scale;
            let max_lines = ((area.height() / line_h).floor() as usize).max(1);
            for (i, extra) in extras.iter().take(max_lines).enumerate() {
                let y = area.top() + line_h * (i as f32 + 0.5);
                paint_extra_line(pc, pos2(area.right(), y), *extra, 10.5 * scale, scale);
            }
        }
    }
}

/// Siatka wartosci (2 kolumny) pod naglowkiem - gdy dodatkow jest za duzo na kolumne w naglowku.
fn paint_value_grid(pc: &PaintContext<'_>, area: Rect, extras: &[ExtraValue], scale: f32) {
    let row_h = config::VALUE_ROW_HEIGHT * scale;
    let column_gap = 10.0 * scale;
    let column_w = (area.width() - column_gap) / 2.0;
    for (i, extra) in extras.iter().enumerate() {
        let (row, column) = (i / 2, i % 2);
        let left = area.left() + column as f32 * (column_w + column_gap);
        let y = area.top() + row_h * (row as f32 + 0.5);
        if y > area.bottom() {
            break;
        }
        let op = pc.opacity;
        let mut label_left = left;
        if extra.legend {
            pc.painter
                .circle_filled(pos2(left + 2.6 * scale, y), 2.6 * scale, alpha(metric_color(extra.metric), op));
            label_left += 8.0 * scale;
        }
        pc.painter.text(
            pos2(label_left, y),
            Align2::LEFT_CENTER,
            extra.metric.short_label(),
            FontId::proportional(9.0 * scale),
            alpha(TEXT_MUTED, op),
        );
        pc.painter.text(
            pos2(left + column_w, y),
            Align2::RIGHT_CENTER,
            extra_text(extra.metric, extra.value),
            FontId::proportional(10.5 * scale),
            alpha(extra_color(extra.metric, extra.value), op),
        );
    }
}

/// Dodatki pokazywane jako wartosci (bez osobnych wykresow) z informacja o legendzie.
fn value_extras(widget: &WidgetConfig, values: &WidgetValues) -> Vec<ExtraValue> {
    values
        .extras
        .iter()
        .zip(&widget.extra)
        .filter(|(_, extra)| widget.kind != WidgetKind::Graph || extra.chart != ExtraChart::Separate)
        .map(|((metric, value), extra)| ExtraValue {
            metric: *metric,
            value: *value,
            legend: widget.kind == WidgetKind::Graph && extra.chart == ExtraChart::Line,
        })
        .collect()
}

fn format_value(metric: Metric, value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{v:.*}", metric.decimals()),
        None => "—".to_owned(),
    }
}

pub fn paint_widget(pc: &PaintContext<'_>, rect: Rect, widget: &WidgetConfig, values: &WidgetValues) {
    // Skala liczona z rozmiaru logicznego karty, a dopiero potem mnozona przez zoom,
    // zeby podglad w oknie konfiguracji wygladal jak pomniejszony overlay.
    let default = widget.kind.default_size();
    let base_scale = (widget.size[0] / default[0]).min(widget.size[1] / default[1]).clamp(0.6, 3.0);
    let scale = base_scale * pc.zoom;
    let op = pc.opacity;
    let radius = CornerRadius::same((8.0 * scale).round().clamp(1.0, 20.0) as u8);

    if widget.background_opacity > 0.0 {
        let card_alpha = widget.background_opacity * op;
        pc.painter.rect_filled(rect, radius, alpha(CARD_FILL, card_alpha));
        pc.painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, alpha(CARD_BORDER, card_alpha * 0.8)),
            StrokeKind::Inside,
        );
    }

    let inner = rect.shrink(8.0 * scale);
    match widget.kind {
        WidgetKind::Graph => paint_graph_card(pc, inner, widget, values, scale),
        WidgetKind::Number => paint_number_card(pc, inner, widget, values, scale),
        WidgetKind::Gauge => paint_gauge_card(pc, inner, widget, values, scale),
        WidgetKind::Bar => paint_bar_card(pc, inner, widget, values, scale),
    }
}

/// Naglowek: ikona + tytul, pod tytulem duza wartosc z jednostka, z prawej druga wartosc.
/// Zwraca dolna krawedz naglowka.
fn paint_header(
    pc: &PaintContext<'_>,
    area: Rect,
    widget: &WidgetConfig,
    values: &WidgetValues,
    scale: f32,
    show_extras: bool,
) -> f32 {
    let op = pc.opacity;
    let title_size = 11.0 * scale;
    let value_size = 20.0 * scale;
    let header_h = if widget.show_title { title_size + value_size + 2.0 * scale } else { value_size };

    let mut text_left = area.left();
    if widget.icon != WidgetIcon::None {
        let icon_size = (header_h * 0.78).min(26.0 * scale);
        let icon_rect = Rect::from_min_size(
            pos2(area.left(), area.top() + (header_h - icon_size) * 0.5),
            Vec2::splat(icon_size),
        );
        paint_icon(pc.painter, icon_rect, widget.icon, alpha(accent_color(widget), op));
        text_left = icon_rect.right() + 7.0 * scale;
    }

    let mut y = area.top();
    if widget.show_title {
        pc.painter.text(
            pos2(text_left, y),
            Align2::LEFT_TOP,
            widget.display_title(),
            FontId::proportional(title_size),
            alpha(TEXT_PRIMARY, op),
        );
        y += title_size + 2.0 * scale;
    }

    let color = values
        .primary
        .map(|v| value_color(widget, v))
        .unwrap_or(TEXT_MUTED);
    let value_text = format_value(widget.metric, values.primary);
    let value_rect = pc.painter.text(
        pos2(text_left, y),
        Align2::LEFT_TOP,
        &value_text,
        FontId::proportional(value_size),
        alpha(color, op),
    );
    if values.primary.is_some() {
        pc.painter.text(
            pos2(value_rect.right() + 1.0, value_rect.bottom() - 2.0 * scale),
            Align2::LEFT_BOTTOM,
            widget.metric.unit(),
            FontId::proportional(value_size * 0.6),
            alpha(color, op),
        );
    }

    if show_extras && widget.value_grid_rows() == 0 {
        let column = Rect::from_min_max(pos2(area.center().x, area.top()), pos2(area.right(), area.top() + header_h));
        paint_extras_column(pc, column, &value_extras(widget, values), value_rect.center().y, scale);
    }

    area.top() + header_h
}

fn footer_height(widget: &WidgetConfig, scale: f32) -> f32 {
    match widget.footer {
        WidgetFooter::None => 0.0,
        _ => 12.0 * scale,
    }
}

fn paint_graph_card(pc: &PaintContext<'_>, area: Rect, widget: &WidgetConfig, values: &WidgetValues, scale: f32) {
    let mut top = paint_header(pc, area, widget, values, scale, true);
    let footer_h = footer_height(widget, scale);
    let gap = 6.0 * scale;

    // Siatka wartosci pod naglowkiem, gdy dodatkow jest wiecej niz miesci kolumna w naglowku.
    let grid_rows = widget.value_grid_rows();
    if grid_rows > 0 {
        let grid = Rect::from_min_size(
            pos2(area.left(), top + 3.0 * scale),
            vec2(area.width(), config::VALUE_ROW_HEIGHT * scale * grid_rows as f32),
        );
        paint_value_grid(pc, grid, &value_extras(widget, values), scale);
        top = grid.bottom();
    }

    // Osobne wykresy dodatkowych metryk ukladane od dolu, glowny wykres dostaje reszte miejsca.
    let separate: Vec<(Metric, Option<f32>)> = values
        .extras
        .iter()
        .zip(&widget.extra)
        .filter(|(_, extra)| extra.chart == ExtraChart::Separate)
        .map(|(value, _)| *value)
        .collect();
    let row_h = SEPARATE_CHART_HEIGHT * scale;
    let mut bottom = area.bottom() - if footer_h > 0.0 { footer_h + gap } else { 0.0 };
    for (metric, value) in separate.iter().rev() {
        let chart_top = bottom - (row_h - gap);
        if chart_top - gap < top + 14.0 * scale {
            break;
        }
        let rect = Rect::from_min_max(pos2(area.left(), chart_top), pos2(area.right(), bottom));
        paint_separate_chart(pc, rect, *metric, *value, scale);
        bottom = chart_top - gap;
    }

    let plot = Rect::from_min_max(pos2(area.left(), top + gap), pos2(area.right(), bottom));
    if plot.height() > 6.0 {
        paint_history_plot(pc, plot, widget, scale, true);
    }
    if footer_h > 0.0 {
        let footer = Rect::from_min_max(pos2(area.left(), area.bottom() - footer_h), area.max);
        paint_footer(pc, footer, widget, values, scale);
    }
}

fn paint_number_card(pc: &PaintContext<'_>, area: Rect, widget: &WidgetConfig, values: &WidgetValues, scale: f32) {
    let op = pc.opacity;
    let color = values.primary.map(|v| value_color(widget, v)).unwrap_or(TEXT_MUTED);
    let mut top = area.top();
    if widget.show_title {
        let title_size = 13.0 * scale;
        pc.painter.text(
            area.left_top(),
            Align2::LEFT_TOP,
            widget.display_title(),
            FontId::proportional(title_size),
            alpha(accent_color(widget), op),
        );
        top += title_size + 2.0 * scale;
    }
    let footer_h = footer_height(widget, scale);
    let bottom = area.bottom() - if footer_h > 0.0 { footer_h + 3.0 * scale } else { 0.0 };
    let value_area = Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), bottom));
    let value_size = (value_area.height() * 0.95).clamp(10.0, 120.0);
    let value_rect = pc.painter.text(
        value_area.left_center(),
        Align2::LEFT_CENTER,
        format_value(widget.metric, values.primary),
        FontId::proportional(value_size),
        alpha(color, op),
    );
    if values.primary.is_some() {
        pc.painter.text(
            pos2(value_rect.right() + 2.0, value_rect.bottom() - value_size * 0.12),
            Align2::LEFT_BOTTOM,
            widget.metric.unit(),
            FontId::proportional(value_size * 0.4),
            alpha(color, op),
        );
    }
    let column = Rect::from_min_max(pos2(area.center().x, area.top()), pos2(area.right(), bottom));
    paint_extras_column(pc, column, &value_extras(widget, values), area.top() + 7.0 * scale, scale);
    if footer_h > 0.0 {
        let footer = Rect::from_min_max(pos2(area.left(), area.bottom() - footer_h), area.max);
        paint_footer(pc, footer, widget, values, scale);
    }
}

fn paint_gauge_card(pc: &PaintContext<'_>, area: Rect, widget: &WidgetConfig, values: &WidgetValues, scale: f32) {
    let op = pc.opacity;
    let mut top = area.top();
    if widget.show_title {
        pc.painter.text(
            pos2(area.center().x, top),
            Align2::CENTER_TOP,
            widget.display_title(),
            FontId::proportional(11.0 * scale),
            alpha(TEXT_PRIMARY, op),
        );
        top += 13.0 * scale;
    }
    let footer_h = footer_height(widget, scale);
    let bottom = area.bottom() - if footer_h > 0.0 { footer_h + 3.0 * scale } else { 0.0 };
    let gauge_area = Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), bottom));
    let thickness = 7.0 * scale;
    let radius = (gauge_area.width().min(gauge_area.height()) * 0.5 - thickness * 0.5).max(4.0);
    let center = gauge_area.center();

    // Luk 270 stopni z przerwa na dole.
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    paint_arc(pc.painter, center, radius, start, sweep, thickness, alpha(PLOT_FILL, op * 0.9));
    if let Some(v) = values.primary {
        let t = widget.metric.normalize(v);
        if t > 0.0 {
            paint_arc(pc.painter, center, radius, start, sweep * t, thickness, alpha(value_color(widget, v), op));
        }
    }
    let ring_thickness = thickness * 0.55;
    let mut ring_radius = radius - thickness * 0.5 - ring_thickness * 0.5 - 2.5 * scale;
    for (metric, value) in &values.extras {
        if ring_radius < radius * 0.45 {
            break;
        }
        paint_arc(pc.painter, center, ring_radius, start, sweep, ring_thickness, alpha(PLOT_FILL, op * 0.9));
        if let Some(v) = value {
            let t = metric.normalize(*v);
            if t > 0.0 {
                paint_arc(pc.painter, center, ring_radius, start, sweep * t, ring_thickness, alpha(metric_color(*metric), op));
            }
        }
        ring_radius -= ring_thickness + 2.5 * scale;
    }
    let inner_radius = (ring_radius + ring_thickness).min(radius);
    let color = values.primary.map(|v| value_color(widget, v)).unwrap_or(TEXT_MUTED);
    let value_size = inner_radius * 0.62;
    pc.painter.text(
        center,
        Align2::CENTER_CENTER,
        format!(
            "{}{}",
            format_value(widget.metric, values.primary),
            if values.primary.is_some() { widget.metric.unit() } else { "" }
        ),
        FontId::proportional(value_size),
        alpha(color, op),
    );
    if !values.extras.is_empty() {
        let text = values
            .extras
            .iter()
            .map(|(metric, value)| extra_text(*metric, *value))
            .collect::<Vec<_>>()
            .join("  ·  ");
        pc.painter.text(
            pos2(center.x, center.y + value_size * 0.85),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(value_size * 0.4),
            alpha(TEXT_MUTED, op),
        );
    }
    if footer_h > 0.0 {
        let footer = Rect::from_min_max(pos2(area.left(), area.bottom() - footer_h), area.max);
        paint_footer(pc, footer, widget, values, scale);
    }
}

fn paint_bar_card(pc: &PaintContext<'_>, area: Rect, widget: &WidgetConfig, values: &WidgetValues, scale: f32) {
    // Dodatkowe metryki sa tu wierszami z paskami pod glownym - w naglowku bylyby zdublowane.
    let header_bottom = paint_header(pc, area, widget, values, scale, false);
    let footer_h = footer_height(widget, scale);
    let bar_h = 8.0 * scale;
    let bar_top = (header_bottom + 5.0 * scale).min(area.bottom() - bar_h - footer_h);
    let bar = Rect::from_min_size(pos2(area.left(), bar_top), vec2(area.width(), bar_h));
    let fraction = values.primary.map(|v| widget.metric.normalize(v)).unwrap_or(0.0);
    let color = values.primary.map(|v| value_color(widget, v)).unwrap_or(TEXT_MUTED);
    paint_progress(pc.painter, bar, fraction, alpha(color, pc.opacity), pc.opacity);

    // Musi sie zgadzac z config::BAR_EXTRA_ROW_HEIGHT (o tyle rosnie karta na kazdy wiersz).
    let row_h = 13.0 * scale;
    let mut y = bar.bottom() + 4.0 * scale;
    let limit = area.bottom() - if footer_h > 0.0 { footer_h + 3.0 * scale } else { 0.0 };
    for (metric, value) in &values.extras {
        if y + row_h > limit {
            break;
        }
        let row = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), row_h));
        let font = FontId::proportional(9.0 * scale);
        let label = pc.painter.text(
            row.left_center(),
            Align2::LEFT_CENTER,
            metric.short_label(),
            font.clone(),
            alpha(TEXT_MUTED, pc.opacity),
        );
        let text = pc.painter.text(
            row.right_center(),
            Align2::RIGHT_CENTER,
            extra_text(*metric, *value),
            font,
            alpha(extra_color(*metric, *value), pc.opacity),
        );
        let small = Rect::from_min_max(
            pos2(row.left() + (label.width() + 6.0 * scale).max(row.width() * 0.28), row.center().y - 2.5 * scale),
            pos2(text.left() - 6.0 * scale, row.center().y + 2.5 * scale),
        );
        if small.width() > 4.0 {
            let fraction = value.map(|v| metric.normalize(v)).unwrap_or(0.0);
            paint_progress(pc.painter, small, fraction, alpha(metric_color(*metric), pc.opacity), pc.opacity);
        }
        y += row_h;
    }
    if footer_h > 0.0 {
        let footer = Rect::from_min_max(pos2(area.left(), area.bottom() - footer_h), area.max);
        paint_footer(pc, footer, widget, values, scale);
    }
}

fn paint_footer(pc: &PaintContext<'_>, rect: Rect, widget: &WidgetConfig, values: &WidgetValues, scale: f32) {
    let op = pc.opacity;
    let font = FontId::proportional(9.5 * scale);
    match widget.footer {
        WidgetFooter::None => {}
        WidgetFooter::Stats => {
            let Some((min, avg, max)) = pc.data.stats(widget.metric) else {
                return;
            };
            let parts = [("Min", min), ("Śr.", avg), ("Max", max)];
            let slot = rect.width() / parts.len() as f32;
            for (i, (label, value)) in parts.into_iter().enumerate() {
                let x = rect.left() + slot * i as f32;
                let label_rect = pc.painter.text(
                    pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    font.clone(),
                    alpha(TEXT_MUTED, op),
                );
                pc.painter.text(
                    pos2(label_rect.right() + 3.0 * scale, rect.center().y),
                    Align2::LEFT_CENTER,
                    format!("{}{}", format_value(widget.metric, Some(value)), widget.metric.unit()),
                    font.clone(),
                    alpha(value_color(widget, value), op),
                );
            }
        }
        WidgetFooter::VramBar => {
            let label_rect = pc.painter.text(
                rect.left_center(),
                Align2::LEFT_CENTER,
                "VRAM",
                font.clone(),
                alpha(TEXT_MUTED, op),
            );
            let detail = match (pc.data.vram_used_mb, pc.data.vram_total_mb) {
                (Some(used), Some(total)) => format!("{:.1} / {:.0} GB", used / 1024.0, total / 1024.0),
                _ => "—".to_owned(),
            };
            let detail_rect = pc.painter.text(
                rect.right_center(),
                Align2::RIGHT_CENTER,
                detail,
                font,
                alpha(TEXT_PRIMARY, op),
            );
            let bar = Rect::from_min_max(
                pos2(label_rect.right() + 6.0 * scale, rect.center().y - 3.0 * scale),
                pos2(detail_rect.left() - 6.0 * scale, rect.center().y + 3.0 * scale),
            );
            if bar.width() > 4.0 {
                let pct = pc.data.value(Metric::VramUsage);
                let fraction = pct.map(|p| p / 100.0).unwrap_or(0.0);
                let color = pct.map(|p| vram_color(widget, p)).unwrap_or(TEXT_MUTED);
                paint_progress(pc.painter, bar, fraction, alpha(color, op), op);
            }
        }
        WidgetFooter::Segments => {
            let label_rect = pc.painter.text(
                rect.left_center(),
                Align2::LEFT_CENTER,
                widget.metric.short_label(),
                font,
                alpha(TEXT_MUTED, op),
            );
            let fraction = values.primary.map(|v| widget.metric.normalize(v)).unwrap_or(0.0);
            let color = values.primary.map(|v| value_color(widget, v)).unwrap_or(TEXT_MUTED);
            let area = Rect::from_min_max(
                pos2(label_rect.right() + 6.0 * scale, rect.center().y - 3.5 * scale),
                pos2(rect.right(), rect.center().y + 3.5 * scale),
            );
            const SEGMENTS: usize = 12;
            let gap = 2.0 * scale;
            let seg_w = (area.width() - gap * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
            if seg_w > 1.0 {
                let lit = (fraction * SEGMENTS as f32).round() as usize;
                for i in 0..SEGMENTS {
                    let x = area.left() + i as f32 * (seg_w + gap);
                    let seg = Rect::from_min_size(pos2(x, area.top()), vec2(seg_w, area.height()));
                    let fill = if i < lit { alpha(color, op) } else { alpha(PLOT_FILL, op) };
                    pc.painter.rect_filled(seg, CornerRadius::same(1), fill);
                }
            }
        }
    }
}

/// Pasek VRAM uzywa progow metryki VRAM, a nie metryki glownej karty.
fn vram_color(widget: &WidgetConfig, pct: f32) -> Color32 {
    let (warn, crit) = Metric::VramUsage.default_thresholds();
    if pct >= crit {
        CRIT_COLOR
    } else if pct >= warn {
        WARN_COLOR
    } else {
        accent_color(widget)
    }
}

fn paint_progress(painter: &Painter, rect: Rect, fraction: f32, color: Color32, opacity: f32) {
    let radius = CornerRadius::same((rect.height() * 0.5).min(255.0) as u8);
    painter.rect_filled(rect, radius, alpha(PLOT_FILL, opacity));
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let filled = Rect::from_min_size(rect.min, vec2((rect.width() * fraction).max(rect.height()), rect.height()));
        painter.rect_filled(filled, radius, color);
    }
}

/// Wykres historii: ciemne tlo, delikatna siatka, gradientowe wypelnienie pod krzywa i linia z poswiata.
fn paint_history_plot(pc: &PaintContext<'_>, plot: Rect, widget: &WidgetConfig, scale: f32, with_background: bool) {
    let op = pc.opacity;
    let painter = pc.painter.with_clip_rect(plot);
    if with_background {
        painter.rect_filled(plot, CornerRadius::same((3.0 * scale) as u8), alpha(PLOT_FILL, op * 0.85));
        for i in 1..4 {
            let y = plot.top() + plot.height() * i as f32 / 4.0;
            painter.hline(plot.x_range(), y, Stroke::new(1.0, alpha(CARD_BORDER, op * 0.18)));
        }
    }
    // Dodatkowe metryki w trybie "na wykresie glownym" jako cienkie linie (kazda w skali swojej
    // metryki), glowna na wierzchu z wypelnieniem i poswiata.
    for extra in widget.extra.iter().filter(|e| e.chart == ExtraChart::Line) {
        let color = metric_color(extra.metric);
        for segment in history_segments(pc, plot, extra.metric) {
            painter.line(segment, Stroke::new(1.1 * scale, alpha(color, op * 0.85)));
        }
    }
    let accent = accent_color(widget);
    for segment in history_segments(pc, plot, widget.metric) {
        paint_area(&painter, plot, &segment, accent, op);
        painter.line(segment.clone(), Stroke::new(3.5 * scale, alpha(accent, op * 0.18)));
        painter.line(segment, Stroke::new(1.4 * scale, alpha(accent, op)));
    }
}

/// Osobny wykres dodatkowej metryki: u gory pasek z etykieta i wartoscia, pod nim wykres
/// we wlasnej skali - napisy nie nachodza na krzywa.
fn paint_separate_chart(pc: &PaintContext<'_>, rect: Rect, metric: Metric, value: Option<f32>, scale: f32) {
    let op = pc.opacity;
    let strip_h = 12.0 * scale;
    let strip_y = rect.top() + strip_h * 0.5;
    let color = metric_color(metric);
    pc.painter.circle_filled(pos2(rect.left() + 2.6 * scale, strip_y), 2.6 * scale, alpha(color, op));
    pc.painter.text(
        pos2(rect.left() + 8.0 * scale, strip_y),
        Align2::LEFT_CENTER,
        metric.short_label(),
        FontId::proportional(9.0 * scale),
        alpha(TEXT_MUTED, op),
    );
    pc.painter.text(
        pos2(rect.right(), strip_y),
        Align2::RIGHT_CENTER,
        extra_text(metric, value),
        FontId::proportional(10.5 * scale),
        alpha(extra_color(metric, value), op),
    );

    let chart = Rect::from_min_max(pos2(rect.left(), rect.top() + strip_h + 2.0 * scale), rect.max);
    if chart.height() < 4.0 {
        return;
    }
    let painter = pc.painter.with_clip_rect(chart);
    painter.rect_filled(chart, CornerRadius::same((3.0 * scale) as u8), alpha(PLOT_FILL, op * 0.85));
    painter.hline(chart.x_range(), chart.center().y, Stroke::new(1.0, alpha(CARD_BORDER, op * 0.18)));
    for segment in history_segments(pc, chart.shrink2(vec2(0.0, 1.5 * scale)), metric) {
        paint_area(&painter, chart, &segment, color, op * 0.8);
        painter.line(segment, Stroke::new(1.3 * scale, alpha(color, op)));
    }
}

/// Ciagle fragmenty historii metryki jako punkty wykresu (NaN = brak odczytu przerywa krzywa).
/// Os X ma stala dlugosc historii, a nowe probki wjezdzaja od prawej - inaczej po starcie
/// kilka probek rozciagaloby sie na cala szerokosc.
fn history_segments(pc: &PaintContext<'_>, plot: Rect, metric: Metric) -> Vec<Vec<Pos2>> {
    let (history, fixed_capacity) = pc.data.series(metric);
    let capacity = fixed_capacity.unwrap_or(pc.history_len).max(history.len()).max(2);
    let offset = capacity - history.len();
    let step = plot.width() / (capacity - 1) as f32;
    let mut segments = Vec::new();
    let mut current = Vec::new();
    for (i, &v) in history.iter().enumerate() {
        if v.is_finite() {
            let t = metric.normalize(v);
            current.push(pos2(
                plot.left() + step * (offset + i) as f32,
                plot.bottom() - t * (plot.height() - 2.0) - 1.0,
            ));
        } else if !current.is_empty() {
            segments.push(std::mem::take(&mut current));
        }
    }
    segments.push(current);
    segments.retain(|s| s.len() >= 2);
    segments
}

/// Kolor dodatkowej metryki na wykresach/paskach - jej domyslny akcent.
pub fn metric_color(metric: Metric) -> Color32 {
    let [r, g, b] = crate::config::default_accent(metric);
    Color32::from_rgb(r, g, b)
}

fn paint_area(painter: &Painter, plot: Rect, points: &[Pos2], color: Color32, opacity: f32) {
    let mut mesh = Mesh::default();
    let top_alpha = 0.45 * opacity;
    for &p in points {
        // Im wyzej punkt, tym mocniejszy kolor - gradient pionowy niezalezny od wartosci.
        let t = ((plot.bottom() - p.y) / plot.height()).clamp(0.0, 1.0);
        mesh.colored_vertex(p, alpha(color, top_alpha * (0.35 + 0.65 * t)));
        mesh.colored_vertex(pos2(p.x, plot.bottom()), alpha(color, 0.02 * opacity));
    }
    for i in 0..(points.len() as u32 - 1) {
        let (a, b, c, d) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        mesh.add_triangle(a, b, c);
        mesh.add_triangle(b, d, c);
    }
    painter.add(mesh);
}

fn paint_arc(painter: &Painter, center: Pos2, radius: f32, start: f32, sweep: f32, thickness: f32, color: Color32) {
    let steps = ((sweep.abs() * radius / 3.0).ceil() as usize).clamp(8, 160);
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = start + sweep * i as f32 / steps as f32;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    // Zaokraglone konce luku.
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        painter.circle_filled(*first, thickness * 0.5, color);
        painter.circle_filled(*last, thickness * 0.5, color);
    }
    painter.line(points, Stroke::new(thickness, color));
}

/// Ikona Phosphor wysrodkowana w kwadracie `rect`.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: WidgetIcon, color: Color32) {
    if let Some(glyph) = crate::icons::glyph(icon) {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            FontId::new(rect.height(), crate::icons::icon_family()),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_color_respects_thresholds() {
        let widget = WidgetConfig::new(1, Metric::GpuTemp, WidgetKind::Graph, [0.0, 0.0]);
        assert_eq!(value_color(&widget, 20.0), accent_color(&widget));
        assert_eq!(value_color(&widget, widget.crit_threshold + 1.0), CRIT_COLOR);
    }
}
