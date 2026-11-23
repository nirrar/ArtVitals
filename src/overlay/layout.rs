//! Przyciaganie widgetow podczas przeciagania i zmiany rozmiaru.
//!
//! Kolejnosc: najpierw magnes do krawedzi/srodkow ekranu i innych widgetow
//! (rowniez "na styk" z odstepem `gap`), a gdy na danej osi nic nie przyciagnelo,
//! zaokraglenie do siatki. Zwracane linie pomocnicze sa rysowane w podgladzie okna konfiguracji.
//! Czysta logika bez UI - latwa do testowania.

use eframe::egui::{Rect, Vec2, pos2};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    Vertical(f32),
    Horizontal(f32),
}

#[derive(Debug, Clone, Copy)]
pub struct SnapSettings {
    pub grid: Option<f32>,
    /// Zasieg magnesu w punktach; `None` wylacza przyciaganie do krawedzi.
    pub edge_distance: Option<f32>,
    /// Odstep miedzy kartami przy przyciaganiu "obok siebie".
    pub gap: f32,
}

/// Najblizsze dopasowanie na jednej osi: (przesuniecie, pozycja linii pomocniczej).
fn best_axis_snap(edges: &[f32], targets: &[(f32, f32)], distance: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for &edge in edges {
        for &(target, guide) in targets {
            let delta = target - edge;
            if delta.abs() <= distance && best.is_none_or(|(d, _)| delta.abs() < d.abs()) {
                best = Some((delta, guide));
            }
        }
    }
    best
}

/// Cele dla jednej osi: (pozycja, gdzie narysowac linie). `horizontal` = os X.
fn axis_targets(others: &[Rect], screen: Rect, gap: f32, horizontal: bool) -> Vec<(f32, f32)> {
    let mut targets = Vec::new();
    let span = |r: &Rect| {
        if horizontal { (r.left(), r.center().x, r.right()) } else { (r.top(), r.center().y, r.bottom()) }
    };
    let (s_min, s_mid, s_max) = span(&screen);
    targets.push((s_min, s_min));
    targets.push((s_mid, s_mid));
    targets.push((s_max, s_max));
    for other in others {
        let (min, mid, max) = span(other);
        targets.push((min, min));
        targets.push((mid, mid));
        targets.push((max, max));
        // Ustawienie obok siebie z odstepem: krawedz ruchomego dotyka krawedzi innego + gap.
        targets.push((max + gap, max));
        targets.push((min - gap, min));
    }
    targets
}

fn round_to(value: f32, grid: f32) -> f32 {
    (value / grid).round() * grid
}

/// Przyciaganie przy przesuwaniu calego prostokata.
pub fn snap_move(moving: Rect, others: &[Rect], screen: Rect, settings: SnapSettings) -> (Rect, Vec<Guide>) {
    let mut offset = Vec2::ZERO;
    let mut guides = Vec::new();

    for horizontal in [true, false] {
        let (min, mid, max) = if horizontal {
            (moving.left(), moving.center().x, moving.right())
        } else {
            (moving.top(), moving.center().y, moving.bottom())
        };
        let mut snapped = false;
        if let Some(distance) = settings.edge_distance {
            let targets = axis_targets(others, screen, settings.gap, horizontal);
            if let Some((delta, guide)) = best_axis_snap(&[min, mid, max], &targets, distance) {
                if horizontal {
                    offset.x = delta;
                    guides.push(Guide::Vertical(guide));
                } else {
                    offset.y = delta;
                    guides.push(Guide::Horizontal(guide));
                }
                snapped = true;
            }
        }
        if !snapped && let Some(grid) = settings.grid {
            let delta = round_to(min, grid) - min;
            if horizontal { offset.x = delta } else { offset.y = delta }
        }
    }
    (moving.translate(offset), guides)
}

/// Przyciaganie przy zmianie rozmiaru (uchwyt w prawym dolnym rogu): ruszaja sie tylko prawa i dolna krawedz.
pub fn snap_resize(
    resizing: Rect,
    min_size: Vec2,
    others: &[Rect],
    screen: Rect,
    settings: SnapSettings,
) -> (Rect, Vec<Guide>) {
    let mut max = resizing.max;
    let mut guides = Vec::new();

    for horizontal in [true, false] {
        let edge = if horizontal { max.x } else { max.y };
        let mut snapped = false;
        if let Some(distance) = settings.edge_distance {
            let targets = axis_targets(others, screen, settings.gap, horizontal);
            if let Some((delta, guide)) = best_axis_snap(&[edge], &targets, distance) {
                if horizontal {
                    max.x += delta;
                    guides.push(Guide::Vertical(guide));
                } else {
                    max.y += delta;
                    guides.push(Guide::Horizontal(guide));
                }
                snapped = true;
            }
        }
        if !snapped && let Some(grid) = settings.grid {
            if horizontal { max.x = round_to(max.x, grid) } else { max.y = round_to(max.y, grid) }
        }
    }
    let max = pos2(max.x.max(resizing.min.x + min_size.x), max.y.max(resizing.min.y + min_size.y));
    (Rect::from_min_max(resizing.min, max), guides)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> SnapSettings {
        SnapSettings { grid: Some(8.0), edge_distance: Some(10.0), gap: 8.0 }
    }

    fn screen() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1920.0, 1080.0))
    }

    #[test]
    fn snaps_to_screen_edge() {
        let moving = Rect::from_min_size(pos2(6.0, 503.0), Vec2::new(100.0, 50.0));
        let (snapped, guides) = snap_move(moving, &[], screen(), settings());
        assert_eq!(snapped.left(), 0.0);
        assert!(guides.contains(&Guide::Vertical(0.0)));
    }

    #[test]
    fn snaps_next_to_other_widget_with_gap() {
        let other = Rect::from_min_size(pos2(100.0, 100.0), Vec2::new(200.0, 90.0));
        let moving = Rect::from_min_size(pos2(311.0, 103.0), Vec2::new(150.0, 90.0));
        let (snapped, _) = snap_move(moving, &[other], screen(), settings());
        assert_eq!(snapped.left(), other.right() + 8.0);
        assert_eq!(snapped.top(), other.top());
    }

    #[test]
    fn falls_back_to_grid_when_nothing_is_near() {
        let moving = Rect::from_min_size(pos2(613.0, 421.0), Vec2::new(100.0, 50.0));
        let no_edges = SnapSettings { edge_distance: None, ..settings() };
        let (snapped, guides) = snap_move(moving, &[], screen(), no_edges);
        assert_eq!(snapped.min, pos2(616.0, 424.0));
        assert!(guides.is_empty());
    }

    #[test]
    fn resize_respects_min_size() {
        let rect = Rect::from_min_max(pos2(500.0, 500.0), pos2(510.0, 505.0));
        let (snapped, _) = snap_resize(rect, Vec2::new(90.0, 60.0), &[], screen(), settings());
        assert!(snapped.width() >= 90.0 && snapped.height() >= 60.0);
    }
}
