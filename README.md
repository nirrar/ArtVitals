# ArtVitals

Konfigurowalny overlay statystyk sprzętu i klatek wyświetlany nad grą (Windows).
Część rodziny projektów **ArtCraft** (obok Artcord), pisanych z pomocą AI.

Overlay to przezroczyste okno zawsze na wierzchu, które **nigdy nie przechwytuje myszy ani
klawiatury** - nie przeszkadza w grze i nie wstrzykuje się do jej procesu (bez ryzyka z anty-cheatami).
Cała konfiguracja odbywa się w osobnym oknie, poza grą.

Aktualna wersja: zobacz `Cargo.toml` / [CHANGELOG.md](CHANGELOG.md).

## Co potrafi

- **Karty (widgety)**: wykres z historią, wskaźnik, pasek, duża liczba - w stylu półprzezroczystych
  granatowych kart z ikoną, kolorowaną wartością i stopką (min/śr./max, pasek VRAM, segmenty).
- **Metryki**
  - CPU: obciążenie, zegar, temperatura pakietu, pobór mocy,
  - GPU (NVIDIA): obciążenie, temperatura, hotspot, temperatura pamięci, zegar rdzenia i pamięci,
    pobór mocy, wentylator, zajętość VRAM,
  - pamięć RAM: zajętość (%) i użyte GB,
  - gra: FPS, średni FPS, 1% low, 0.1% low, czas klatki (wykres klatka po klatce).
- **Dodatkowe metryki na karcie** (z tego samego podzespołu) - na wykresie głównym, jako osobny
  wykres albo sama wartość; na pasku jako paski, na wskaźniku jako pierścienie.
- **Progi kolorów** - wartość płynnie przechodzi w żółty i czerwony (dla FPS przy spadku).
- **Okno konfiguracji**: podgląd ekranu z powiększaniem, przeciąganiem i przyciąganiem kart
  (siatka, krawędzie, inne karty), zmiana rozmiaru, przypinanie; wyskakujące okna ustawień nad
  rozmytym tłem z żywym podglądem karty; motyw jasny/ciemny.
- **Globalne skróty** (nagrywane albo układane z klocków): pokaż/ukryj overlay, okno konfiguracji.
- Ikona w zasobniku, blokada drugiej instancji, zapis konfiguracji do `config.toml`
  z przeładowaniem po ręcznej edycji.

## Wymagania

- Windows 10/11, gra w trybie **okna bez ramki (Borderless)** - ekskluzywny pełny ekran zasłania
  każde okno, także overlay.
- Karta NVIDIA (NVML/NVAPI ze sterownika). AMD/Intel GPU - jeszcze nie.
- Procesor Intel + zainstalowany sterownik [PawnIO](https://pawnio.eu) - dla temperatury i mocy CPU.
- **Uruchomienie jako administrator** - potrzebne do temperatury/mocy CPU i pomiaru FPS
  (bez admina reszta działa, a brakujące odczyty są opisane w oknie „Czujniki”).

## Uruchomienie

```powershell
cargo build --release
# prawy przycisk -> "Uruchom jako administrator"
target\release\art_vitals.exe
```

- Pierwsze uruchomienie otwiera okno konfiguracji. Później: skrót klawiszowy albo lewy klik
  w ikonę w zasobniku.
- Konfiguracja i log: `%APPDATA%\ArtVitals\config.toml`, `%APPDATA%\ArtVitals\overlay.log`.

## Budowanie i testy

```powershell
cargo build                     # debug (z oknem konsoli)
cargo build --release           # release (bez konsoli)
cargo test                      # testy jednostkowe
cargo clippy --all-targets      # lint - ma przechodzić bez ostrzeżeń
```

## Architektura (skrót)

| Moduł | Rola |
|---|---|
| `src/main.rs` | start: blokada instancji, logger, okno overlaya (eframe, renderer glow) |
| `src/config.rs` | model `config.toml`, walidacja, migracje starych formatów |
| `src/i18n.rs` | tłumaczenia PL/EN (`tr`), wykrywanie języka Windows |
| `src/metrics.rs` | katalog metryk (nazwy, jednostki, zakresy, progi, podzespół) |
| `src/widgets.rs` | rysowanie kart na Painterze egui |
| `src/icons.rs` | ikony Phosphor (font) |
| `src/sensors/` | wątek odczytów: PDH (CPU), PawnIO (temp/moc CPU), NVML + NVAPI (GPU), ETW (FPS) |
| `src/overlay/app.rs` | aplikacja: overlay, zapis, skróty, zasobnik |
| `src/overlay/settings.rs` | okno konfiguracji (podgląd ekranu, panel, wyskakujące okna) |
| `src/overlay/ui_kit.rs` | kontrolki okna konfiguracji |
| `src/overlay/win32.rs` | poprawki okien Windows (warstwa, ramka DWM, zrzut do rozmycia) |
| `src/single_instance.rs` | blokada drugiej instancji |

Szczegóły dla osób i asystentów AI pracujących nad kodem: [AI_CONTEXT.md](AI_CONTEXT.md).

## Plan

Zrobione:
- [x] Overlay click-through nad grą, karty z wykresami i progami kolorów
- [x] Własne czujniki CPU/GPU (bez zewnętrznych programów w tle)
- [x] Okno konfiguracji poza grą: przeciąganie, przyciąganie, przypinanie, podgląd z powiększaniem
- [x] Dodatkowe metryki na kartach z trybami wyświetlania
- [x] Globalne skróty, ikona w zasobniku, blokada instancji, wersjonowanie
- [x] Pomiar FPS / 1% low / 0.1% low / czasu klatki przez ETW (DirectX 9-12)

Do zrobienia (propozycje):
- [ ] Weryfikacja pomiaru FPS w kilku grach (DX11, DX12, Vulkan) i porównanie z PresentMon
- [ ] FPS dla Vulkan/OpenGL bez DXGI (zdarzenia DxgKrnl)
- [ ] Autostart z Windows (jako administrator, przez Harmonogram zadań)
- [ ] Obsługa GPU AMD/Intel i procesorów AMD
- [x] Pamięć RAM, powiększenie tekstu kart z dopasowaniem rozmiaru
- [ ] Więcej metryk: dyski, sieć, wentylatory płyty głównej i AIO
- [ ] Zapis sesji (benchmark) do CSV z podsumowaniem FPS/lows
- [ ] Profile układu kart per gra
- [ ] Metadane wersji w pliku exe i wydania na GitHubie

## Licencje zewnętrznych zasobów

- Ikony [Phosphor](https://phosphoricons.com) (Bold) - MIT, `assets/Phosphor.LICENSE.txt`
- Moduły `IntelMSR.bin`, `AMDFamily17.bin`, `AMDFamily10.bin`, `AMDFamily0F.bin` z
  [PawnIO.Modules](https://github.com/namazso/PawnIO.Modules) (wydanie 0.2.12) - LGPL-2.1,
  `assets/<moduł>.LICENSE.txt`
- Odczyt temperatury i mocy CPU AMD na podstawie [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) (MPL-2.0)
  i sterownika Linuksa `k10temp`
- Identyfikatory zdarzeń ETW na podstawie [PresentMon](https://github.com/GameTechDev/PresentMon) (MIT),
  odczyt sensorów NVAPI na podstawie [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) (MPL-2.0)
- Odczyty GPU AMD (ADL) i Intel (IGCL) na podstawie [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) (MPL-2.0);
  układy struktur z [AMD Display Library SDK](https://github.com/GPUOpen-LibrariesAndSDKs/display-library) (MIT)
  i [Intel Graphics Control Library](https://github.com/intel/drivers.gpu.control-library) (`igcl_api.h`, MIT)
- Moduł `LpcIO.bin` z PawnIO.Modules 0.2.12 - LGPL-2.1, `assets/LpcIO.LICENSE.txt`
- Crate [hidapi](https://github.com/ruabmbua/hidapi-rs) (MIT, z biblioteką C hidapi na licencji BSD/HIDAPI)
- Odczyt wentylatorów Super I/O (Nuvoton, ITE) i AIO NZXT Kraken na podstawie
  [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) (MPL-2.0),
  protokół Corsair Commander Core wg opisu w dokumentacji [liquidctl](https://github.com/liquidctl/liquidctl)
