//! Ikona w zasobniku systemowym z menu: konfiguracja, pokaz/ukryj overlay, zamknij program.
//!
//! Ikona musi powstac w watku z petla komunikatow Win32 (glowny watek eframe).
//! Zdarzenia menu i klikniecia ikony trafiaja do kolejki i budza UI przez `request_repaint`.
//! Obrazek ikony jest generowany w kodzie (kafelek z trzema slupkami wykresu).
//! Zaleznosci: tray-icon (menu z muda).

use std::sync::{Arc, Mutex};

use eframe::egui;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    OpenSettings,
    ToggleOverlay,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    overlay_item: CheckMenuItem,
    commands: Arc<Mutex<Vec<TrayCommand>>>,
    shown_overlay_state: Option<bool>,
}

impl Tray {
    pub fn new(ctx: &egui::Context) -> Option<Self> {
        let settings_item = MenuItem::new("Otwórz konfigurację", true, None);
        let overlay_item = CheckMenuItem::new("Overlay widoczny w grze", true, true, None);
        let quit_item = MenuItem::new("Zamknij program", true, None);
        let menu = Menu::new();
        let appended = menu.append_items(&[
            &settings_item,
            &overlay_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ]);
        if let Err(err) = appended {
            log::error!("Menu zasobnika: {err}");
        }

        let icon = TrayIconBuilder::new()
            .with_tooltip(format!("ArtVitals {}", crate::VERSION))
            .with_icon(tray_image())
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build();
        let icon = match icon {
            Ok(icon) => icon,
            Err(err) => {
                log::error!("Nie udalo sie utworzyc ikony w zasobniku: {err}");
                return None;
            }
        };

        let commands = Arc::new(Mutex::new(Vec::new()));
        let ids: [(MenuId, TrayCommand); 3] = [
            (settings_item.id().clone(), TrayCommand::OpenSettings),
            (overlay_item.id().clone(), TrayCommand::ToggleOverlay),
            (quit_item.id().clone(), TrayCommand::Quit),
        ];
        let queue = commands.clone();
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some((_, command)) = ids.iter().find(|(id, _)| *id == event.id)
                && let Ok(mut queue) = queue.lock()
            {
                queue.push(*command);
                menu_ctx.request_repaint();
            }
        }));
        // Lewy klik w ikone otwiera konfiguracje (prawy pokazuje menu).
        let queue = commands.clone();
        let click_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            let open = matches!(
                event,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
            );
            if open && let Ok(mut queue) = queue.lock() {
                queue.push(TrayCommand::OpenSettings);
                click_ctx.request_repaint();
            }
        }));

        Some(Self { _icon: icon, overlay_item, commands, shown_overlay_state: None })
    }

    pub fn take_commands(&self) -> Vec<TrayCommand> {
        self.commands.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }

    /// Synchronizuje "ptaszka" w menu ze stanem overlaya (zmienianym tez skrotem i w oknie).
    pub fn sync_overlay_visible(&mut self, visible: bool) {
        if self.shown_overlay_state != Some(visible) {
            self.overlay_item.set_checked(visible);
            self.shown_overlay_state = Some(visible);
        }
    }

    /// Menu samo przelacza ptaszka po kliknieciu - po obsludze komendy wymuszamy ponowna synchronizacje.
    pub fn invalidate(&mut self) {
        self.shown_overlay_state = None;
    }
}

/// Ikona 32x32: niebieski zaokraglony kafelek z trzema bialymi slupkami.
fn tray_image() -> Icon {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let bars = [(8.0, 18.0), (14.5, 11.0), (21.0, 15.0)];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let coverage = rounded_rect_coverage(fx, fy, 1.0, 1.0, 31.0, 31.0, 7.0);
            if coverage <= 0.0 {
                continue;
            }
            // Pionowy gradient kafelka.
            let t = fy / SIZE as f32;
            let mut color = [
                lerp(64.0, 20.0, t),
                lerp(158.0, 98.0, t),
                lerp(255.0, 214.0, t),
            ];
            for (left, top) in bars {
                let bar = rounded_rect_coverage(fx, fy, left, top, left + 4.0, 25.0, 1.5);
                for c in &mut color {
                    *c = lerp(*c, 255.0, bar);
                }
            }
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i] = color[0] as u8;
            rgba[i + 1] = color[1] as u8;
            rgba[i + 2] = color[2] as u8;
            rgba[i + 3] = (coverage * 255.0) as u8;
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("rozmiar bufora zgodny z wymiarami ikony")
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Wygladzone (antyaliasowane) pokrycie piksela przez zaokraglony prostokat, 0..1.
fn rounded_rect_coverage(x: f32, y: f32, left: f32, top: f32, right: f32, bottom: f32, radius: f32) -> f32 {
    let cx = x.clamp(left + radius, right - radius);
    let cy = y.clamp(top + radius, bottom - radius);
    let distance = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_is_full_inside_and_empty_outside() {
        assert_eq!(rounded_rect_coverage(16.0, 16.0, 1.0, 1.0, 31.0, 31.0, 7.0), 1.0);
        assert_eq!(rounded_rect_coverage(0.0, 0.0, 1.0, 1.0, 31.0, 31.0, 7.0), 0.0);
    }
}
