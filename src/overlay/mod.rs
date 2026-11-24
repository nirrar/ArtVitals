//! Warstwa okien: overlay nad gra (`app`), okno konfiguracji (`settings`, kontrolki w `ui_kit`,
//! rozmyte tlo pod wyskakujacymi oknami w `blur`), ikona w zasobniku (`tray`),
//! logika przyciagania kart (`layout`), globalny skrot (`hotkey`) i obserwator pliku (`watcher`).

mod app;
mod blur;
mod hotkey;
mod layout;
mod screenshot;
mod settings;
mod tray;
mod ui_kit;
mod watcher;
mod win32;

pub use app::OverlayApp;
