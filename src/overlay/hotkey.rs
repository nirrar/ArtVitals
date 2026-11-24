//! Globalne skroty klawiszowe: pokaz/ukryj overlay w grze oraz otworz/zamknij okno konfiguracji.
//!
//! `GlobalHotKeyManager` musi powstac w watku z petla komunikatow Win32 - czyli
//! w glownym watku eframe (tworzymy go w `OverlayApp::new`).
//! Zdarzenie budzi UI przez `request_repaint`, bo okno click-through bez fokusu
//! nie dostaje innych zdarzen i inaczej skrot zadzialalby dopiero przy kolejnym odswiezeniu.
//! Wymagany jest modyfikator (Ctrl/Alt/Shift), zeby nie zablokowac zwyklego klawisza w grze.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Key, Modifiers};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeySlot {
    /// Pokaz/ukryj overlay w grze.
    Overlay,
    /// Otworz/zamknij okno konfiguracji.
    Settings,
}

impl HotkeySlot {
    fn index(self) -> usize {
        match self {
            HotkeySlot::Overlay => 0,
            HotkeySlot::Settings => 1,
        }
    }
}

pub struct Hotkeys {
    manager: Option<GlobalHotKeyManager>,
    current: [Option<HotKey>; 2],
    /// Identyfikatory wcisnietych skrotow od ostatniego sprawdzenia.
    pressed: Arc<Mutex<Vec<u32>>>,
}

impl Hotkeys {
    pub fn new(ctx: &egui::Context) -> Self {
        let manager = GlobalHotKeyManager::new()
            .map_err(|err| log::error!("Globalne skroty niedostepne: {err}"))
            .ok();
        let pressed = Arc::new(Mutex::new(Vec::new()));
        let queue = pressed.clone();
        let ctx = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                if let Ok(mut queue) = queue.lock() {
                    queue.push(event.id());
                }
                ctx.request_repaint();
            }
        }));
        Self { manager, current: [None, None], pressed }
    }

    /// Rejestruje nowy skrot w danym miejscu. Przy bledzie (np. skrot zajety przez inny
    /// program albo uzyty juz w drugim miejscu) przywraca poprzedni i zwraca opis problemu.
    pub fn set(&mut self, slot: HotkeySlot, text: &str) -> Result<(), String> {
        let hotkey: HotKey = text.parse().map_err(|err| format!("{} \"{text}\": {err}", tr("Niepoprawny skrót", "Invalid shortcut")))?;
        let Some(manager) = &self.manager else {
            return Err(tr("Menedżer skrótów nie działa", "Shortcut manager is not working").to_owned());
        };
        let i = slot.index();
        if self.current[i] == Some(hotkey) {
            return Ok(());
        }
        if self.current[1 - i] == Some(hotkey) {
            return Err(format!(
                "{} \"{text}\" {}",
                tr("Skrót", "Shortcut"),
                tr("jest już używany do innej akcji", "is already used for another action")
            ));
        }
        let previous = self.current[i].take();
        if let Some(previous) = previous {
            let _ = manager.unregister(previous);
        }
        match manager.register(hotkey) {
            Ok(()) => {
                log::info!("Skrot {slot:?}: {text}");
                self.current[i] = Some(hotkey);
                Ok(())
            }
            Err(err) => {
                if let Some(previous) = previous
                    && manager.register(previous).is_ok()
                {
                    self.current[i] = Some(previous);
                }
                log::warn!("Rejestracja skrotu {text} nieudana: {err}");
                Err(format!(
                    "{} \"{text}\" - {}",
                    tr("Nie można ustawić", "Can't set"),
                    tr("skrót zajęty przez inny program", "shortcut is taken by another program")
                ))
            }
        }
    }

    /// Zwraca skroty wcisniete od ostatniego wywolania.
    pub fn take_pressed(&self) -> Vec<HotkeySlot> {
        let ids: Vec<u32> = self.pressed.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        ids.into_iter()
            .filter_map(|id| {
                [HotkeySlot::Overlay, HotkeySlot::Settings]
                    .into_iter()
                    .find(|slot| self.current[slot.index()].is_some_and(|h| h.id() == id))
            })
            .collect()
    }
}

/// Buduje tekst skrotu w formacie rozumianym przez global-hotkey, np. `ctrl+shift+O`.
/// Wymaga co najmniej jednego modyfikatora, zeby nie zablokowac zwyklego klawisza w grze.
pub fn hotkey_string(modifiers: Modifiers, key: Key) -> Option<String> {
    if !(modifiers.ctrl || modifiers.alt || modifiers.shift) {
        return None;
    }
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    let text = format!("{}+{}", parts.join("+"), key.name());
    // Weryfikujemy od razu parserem biblioteki - nie kazda nazwa klawisza egui jest wspierana.
    text.parse::<HotKey>().ok().map(|_| text)
}

/// Rozbija zapisany skrot na modyfikatory i klawisz (do edytora "z klockow").
pub fn split_hotkey(text: &str) -> (Modifiers, Option<Key>) {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            _ => key = Key::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(part)),
        }
    }
    (modifiers, key)
}

/// Klawisze, ktore mozna wybrac w edytorze "z klockow" (tylko obslugiwane przez global-hotkey).
pub fn selectable_keys() -> Vec<Key> {
    let probe = Modifiers { ctrl: true, ..Default::default() };
    Key::ALL
        .iter()
        .copied()
        .filter(|key| !matches!(key, Key::Escape | Key::Enter | Key::Tab | Key::Backspace))
        .filter(|key| hotkey_string(probe, *key).is_some())
        .collect()
}

/// Czytelne czesci skrotu do wyswietlenia jako "klocki", np. ["Ctrl", "Shift", "O"].
pub fn hotkey_parts(text: &str) -> Vec<String> {
    let (modifiers, key) = split_hotkey(text);
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("Ctrl".to_owned());
    }
    if modifiers.alt {
        parts.push("Alt".to_owned());
    }
    if modifiers.shift {
        parts.push("Shift".to_owned());
    }
    if let Some(key) = key {
        parts.push(key.symbol_or_name().to_owned());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_parseable_hotkey_strings() {
        let mods = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(hotkey_string(mods, Key::O).as_deref(), Some("ctrl+shift+O"));
        assert_eq!(hotkey_string(mods, Key::F5).as_deref(), Some("ctrl+shift+F5"));
        assert_eq!(hotkey_string(Modifiers::NONE, Key::O), None);
    }

    #[test]
    fn split_roundtrips_with_builder() {
        let (mods, key) = split_hotkey("ctrl+alt+F7");
        assert!(mods.ctrl && mods.alt && !mods.shift);
        assert_eq!(key, Some(Key::F7));
        assert_eq!(hotkey_string(mods, key.unwrap()).as_deref(), Some("ctrl+alt+F7"));
    }

    #[test]
    fn selectable_keys_contain_common_keys() {
        let keys = selectable_keys();
        assert!(keys.contains(&Key::O) && keys.contains(&Key::F12) && keys.contains(&Key::Home));
        assert!(!keys.contains(&Key::Escape));
    }
}
