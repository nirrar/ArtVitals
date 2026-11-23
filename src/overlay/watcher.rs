//! Obserwowanie `config.toml` pod katem recznej edycji.
//!
//! Obserwujemy katalog (nie plik), bo zapis atomowy przez rename podmienia plik
//! i watcher pojedynczego pliku przestalby dzialac. Zdarzenie tylko ustawia flage
//! i budzi UI - samo przeladowanie (i odfiltrowanie wlasnych zapisow) robi `OverlayApp`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{CONFIG_FILE_NAME, config_dir};

pub struct ConfigWatcher {
    _watcher: Option<RecommendedWatcher>,
    changed: Arc<AtomicBool>,
}

impl ConfigWatcher {
    pub fn start(ctx: &egui::Context) -> Self {
        let changed = Arc::new(AtomicBool::new(false));
        let flag = changed.clone();
        let ctx = ctx.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event) => {
                let touches_config = event
                    .paths
                    .iter()
                    .any(|p| p.file_name().is_some_and(|name| name == CONFIG_FILE_NAME));
                if touches_config && (event.kind.is_modify() || event.kind.is_create()) {
                    flag.store(true, Ordering::Relaxed);
                    ctx.request_repaint();
                }
            }
            Err(err) => log::warn!("Blad obserwowania konfiguracji: {err}"),
        });
        let watcher = watcher
            .and_then(|mut w| w.watch(&config_dir(), RecursiveMode::NonRecursive).map(|_| w))
            .map_err(|err| log::error!("Przeladowanie konfiguracji na zywo nie dziala: {err}"))
            .ok();
        Self { _watcher: watcher, changed }
    }

    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::Relaxed)
    }
}
