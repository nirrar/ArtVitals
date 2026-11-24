//! Blokada drugiej instancji programu.
//!
//! Pierwsza instancja tworzy nazwany mutex oraz nazwane zdarzenie "pokaz konfiguracje"
//! i czeka na nie w osobnym watku. Kolejne uruchomienie wykrywa istniejacy mutex,
//! ustawia zdarzenie (dzialajaca instancja otwiera okno konfiguracji) i konczy dzialanie.
//! Gdy instancje dzialaja z roznymi uprawnieniami (np. pierwsza jako administrator),
//! system nie pozwala otworzyc obiektow - wtedy pokazujemy komunikat zamiast cicho wyjsc.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
use windows::core::{HSTRING, w};

const MUTEX_NAME: &str = r"Local\ArtVitals.Instance";
const SHOW_EVENT_NAME: &str = r"Local\ArtVitals.ShowSettings";

/// Uchwyty pierwszej instancji - trzymane do konca dzialania programu.
pub struct InstanceGuard {
    _mutex: HANDLE,
    show_event: HANDLE,
}

// Uchwyty obiektow jadra mozna bezpiecznie uzywac z dowolnego watku.
unsafe impl Send for InstanceGuard {}

pub enum Acquire {
    /// To jest pierwsza instancja.
    First(InstanceGuard),
    /// Program juz dziala (dzialajaca instancja zostala poproszona o pokazanie konfiguracji).
    AlreadyRunning,
}

pub fn acquire() -> Acquire {
    // Tylko w buildzie debug: pozwala testowac obok dzialajacej instancji (np. uruchomionej jako admin).
    #[cfg(debug_assertions)]
    if std::env::var_os("CGB_ALLOW_MULTI").is_some() {
        return Acquire::First(InstanceGuard { _mutex: HANDLE::default(), show_event: HANDLE::default() });
    }
    unsafe {
        let mutex = CreateMutexW(None, true, &HSTRING::from(MUTEX_NAME));
        let exists = match &mutex {
            Ok(_) => GetLastError() == ERROR_ALREADY_EXISTS,
            // Brak dostepu = mutex istnieje, ale nalezy do procesu z wyzszymi uprawnieniami.
            Err(_) => true,
        };
        if exists {
            notify_running_instance();
            return Acquire::AlreadyRunning;
        }
        let mutex = mutex.expect("sprawdzone wyzej");
        match CreateEventW(None, false, false, &HSTRING::from(SHOW_EVENT_NAME)) {
            Ok(show_event) => Acquire::First(InstanceGuard { _mutex: mutex, show_event }),
            Err(err) => {
                // Bez zdarzenia blokada nadal dziala, tylko druga instancja nie otworzy okna.
                log::warn!("Nie udalo sie utworzyc zdarzenia instancji: {err}");
                Acquire::First(InstanceGuard { _mutex: mutex, show_event: HANDLE::default() })
            }
        }
    }
}

fn notify_running_instance() {
    let signaled = unsafe {
        OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(SHOW_EVENT_NAME)).and_then(|event| SetEvent(event))
    };
    match signaled {
        Ok(()) => log::info!("Program juz dziala - poproszono go o otwarcie konfiguracji"),
        Err(err) => {
            log::warn!("Program juz dziala, ale nie mozna go powiadomic: {err}");
            unsafe {
                MessageBoxW(
                    None,
                    &HSTRING::from(crate::i18n::tr(
                        "ArtVitals już działa (prawdopodobnie uruchomiony jako administrator).\n\nOtwórz konfigurację skrótem klawiszowym albo z ikony w zasobniku.",
                        "ArtVitals is already running (probably as administrator).\n\nOpen the settings with the keyboard shortcut or from the tray icon.",
                    )),
                    w!("ArtVitals"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
    }
}

impl InstanceGuard {
    /// Uruchamia watek czekajacy na prosby innych instancji; zwraca flage "pokaz konfiguracje".
    pub fn listen(self, ctx: &egui::Context) -> Arc<AtomicBool> {
        let requested = Arc::new(AtomicBool::new(false));
        if self.show_event.is_invalid() {
            return requested;
        }
        let flag = requested.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("single-instance".to_owned())
            .spawn(move || {
                // Watek zyje do konca programu i trzyma uchwyty. InstanceGuard nie zamyka ich
                // przy zwolnieniu, wiec mutex istnieje az do zakonczenia procesu.
                let guard = self;
                loop {
                    let result = unsafe { WaitForSingleObject(guard.show_event, INFINITE) };
                    if result != WAIT_OBJECT_0 {
                        log::warn!("Oczekiwanie na zdarzenie instancji przerwane");
                        break;
                    }
                    flag.store(true, Ordering::Relaxed);
                    ctx.request_repaint();
                }
            });
        if let Err(err) = spawned {
            log::error!("Nie udalo sie uruchomic watku instancji: {err}");
        }
        requested
    }
}
