//! ArtVitals - konfigurowalny overlay statystyk CPU/GPU wyswietlany nad gra.
//!
//! Start: blokada drugiej instancji, logger (plik `overlay.log` w katalogu konfiguracji -
//! obok exe w wersji portable albo `%APPDATA%\ArtVitals`),
//! wczytanie config.toml,
//! utworzenie przezroczystego okna overlaya (zawsze na wierzchu, click-through, bez paska zadan)
//! i ewentualnie okna konfiguracji. Renderer glow - na Windows niezawodnie obsluguje przezroczystosc.
//! Gra musi dzialac w trybie okna bez ramki (Borderless) - ekskluzywny pelny ekran zaslania overlay.

// W wersji release bez okna konsoli; logi trafiaja do pliku.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod icons;
mod metrics;
mod overlay;
mod sensors;
mod single_instance;
mod widgets;

use eframe::egui;

/// Wersja programu z Cargo.toml (historia zmian w CHANGELOG.md).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tytul okna overlaya - po nim `overlay::win32` odnajduje okno do poprawki stylu.
pub const OVERLAY_TITLE: &str = "ArtVitals Overlay";

fn init_logger() {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("art_vitals=info,warn"));
    let dir = config::config_dir();
    let log_file = std::fs::create_dir_all(&dir).and_then(|_| std::fs::File::create(dir.join("overlay.log")));
    match log_file {
        Ok(file) => {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
        // Brak pliku logu nie moze blokowac startu - zostaje stderr.
        Err(err) => eprintln!("Nie mozna utworzyc pliku logu: {err}"),
    }
    builder.init();
}

fn main() -> eframe::Result {
    // Blokada instancji przed loggerem - logger czysci plik logu, a druga instancja
    // nie moze skasowac logu dzialajacej.
    let instance = match single_instance::acquire() {
        single_instance::Acquire::First(guard) => guard,
        single_instance::Acquire::AlreadyRunning => return Ok(()),
    };
    init_logger();
    log::info!("ArtVitals {VERSION} - start");
    let config = config::load_or_create();

    let viewport = egui::ViewportBuilder::default()
        .with_title(OVERLAY_TITLE)
        .with_transparent(true)
        .with_decorations(false)
        .with_always_on_top()
        .with_mouse_passthrough(true)
        .with_taskbar(false)
        .with_resizable(false)
        .with_active(false)
        .with_position([0.0, 0.0])
        // Docelowy rozmiar (caly monitor) ustawiany jest po starcie, gdy znany jest rozmiar monitora.
        .with_inner_size([1920.0, 1080.0]);

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    let result = eframe::run_native(
        "ArtVitals",
        options,
        Box::new(move |cc| Ok(Box::new(overlay::OverlayApp::new(cc, config, instance)))),
    );
    if let Err(err) = &result {
        log::error!("Overlay zakonczyl sie bledem: {err}");
    }
    result
}
