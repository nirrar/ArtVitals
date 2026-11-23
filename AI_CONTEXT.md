# AI_CONTEXT - ArtVitals

Kontekst dla asystentów AI (i ludzi) pracujących nad kodem. Opis funkcji i plan: [README.md](README.md),
historia zmian: [CHANGELOG.md](CHANGELOG.md).

## Projekt w jednym akapicie

Rust, Windows. Przezroczysty overlay (eframe 0.37, renderer **glow**) nad grą w trybie Borderless,
zawsze click-through; konfiguracja w osobnym oknie (drugi viewport egui, tryb immediate).
Czujniki własne, bez zewnętrznych programów: PDH, PawnIO (MSR Intel), NVML + NVAPI, ETW (FPS).
Konfiguracja: `%APPDATA%\ArtVitals\config.toml` (dawniej `CraftGamingBoard` - migracja w `config.rs`).

## Konwencje (obowiązkowe)

- Nazwy w kodzie po angielsku, **komentarze i dokumentacja po polsku**, komentarze tylko przy
  nieoczywistym "dlaczego". Każdy plik zaczyna się nagłówkiem `//!` (co robi, zależności, założenia).
- Zmiany minimalne, bez usuwania istniejących funkcji. Przy zmianie wielu plików - najpierw plan.
- **Nowe zależności (crate'y, pliki binarne) tylko po zgodzie właściciela.** Nowe moduły (features)
  istniejącego crate'a `windows` są OK.
- Nie zgadywać API: sprawdzać źródła crate'ów w `~/.cargo/registry/src/...` (wersje z `Cargo.lock`).
  Nieudokumentowane API (NVAPI, ETW) - tylko na podstawie źródeł LHM/PresentMon, z odnośnikiem w komentarzu.
- Logger (`log::info!` itp.), nie `println!`. Błędy czujników - do `SensorData::issues`, program działa dalej.
- Przed zakończeniem: `cargo clippy --all-targets` bez ostrzeżeń i `cargo test`.
- W UI i kodzie **bez odniesień do systemów/stylistyki Apple** (styl "grupowanych list" opisujemy neutralnie).
- Commit/push tylko na wyraźną prośbę. Gałąź `main`, wersje oznaczane tagami `vX.Y.Z`,
  każda zmiana funkcjonalna = wpis w `CHANGELOG.md` + podbicie wersji w `Cargo.toml` (semver).

## Pułapki, które już kosztowały czas

- **Overlay niewidoczny**: winit przy click-through dodaje `WS_EX_LAYERED` bez `SetLayeredWindowAttributes`
  - naprawiane w `overlay/win32.rs::fix_overlay_window` (szukanie okna po tytule + PID).
- **Szara ramka wokół ekranu**: obwódka/cień DWM Windows 11 - wyłączane atrybutami DWM w tym samym miejscu.
- **egui::Scene** przeciekał transformacją na overlay i rozmywał tekst - podgląd ekranu ma własny
  zoom/pan (`settings.rs`: `PreviewView`, `ViewTransform`). Nie wracać do `Scene`.
- **Zrzuty viewportu immediate nie działają** w eframe 0.37 - rozmyte tło okien to zrzut GDI
  (`win32::capture_window`) + rozmycie CPU (`overlay/blur.rs`).
- Oba okna dzielą jeden `egui::Context` (animacje, id) - id widgetów przez `Id::unique` / `scope_id`.
- Okno konfiguracji: zamknięcie tylko je chowa; program żyje w zasobniku. Druga instancja
  (nazwany mutex) prosi pierwszą o otwarcie okna. Do testów obok działającej instancji:
  build **debug** + zmienna `CGB_ALLOW_MULTI=1` (w release nie działa).
- FPS przez ETW wymaga admina; sesja `ArtVitals-FrameTiming` jest zatrzymywana przy wyjściu
  i przy starcie, jeśli została po awarii.
- PowerShell 5.1 czyta pliki `.ps1` bez BOM jako ANSI - skrypty pomocnicze trzymać w ASCII.

## Model danych

- `Metric` (`metrics.rs`) - katalog; `Device` (Cpu/Gpu/Game) decyduje, co można łączyć na karcie.
  `higher_is_worse()` odwraca progi dla FPS. `ALL` wyznacza indeksy w tablicach `SensorData`.
- `WidgetConfig` (`config.rs`) - karta: metryka główna, `extra: Vec<ExtraMetric>` (tryb `chart`:
  line/separate/value), typ, ikona, stopka, pozycja/rozmiar w punktach ekranu, progi.
  Wysokość karty rośnie automatycznie z dodatkami (`edit_extras`, `extra_rows_height`).
- Stare formaty w `config.toml` (`secondary`, `plot_extra`, `extra = ["..."]`) są wczytywane
  i migrowane w `Config::sanitize`.

## Testowanie UI

Brak testów UI w kodzie. Sprawdzanie: uruchomienie buildu debug, zrzuty ekranu i symulowane
kliknięcia przez WinAPI (PowerShell). Nie zmieniać przy tym trwale konfiguracji użytkownika -
po teście cofnąć (np. „Anuluj” w oknie karty) albo przywrócić plik.

## Sprzęt właściciela (do weryfikacji odczytów)

ASUS TUF Z690-PLUS D4, Intel i5-14600KF, NVIDIA RTX 4070 Ti (NVAPI: hotspot = indeks 1,
pamięć = indeks 7 dla RTX 40), monitor 2560x1440.
