# Przejście pomiaru FPS na PresentMon

Plan zamiany źródła klatek w ArtVitals z własnej sesji ETW (`src/sensors/fps.rs`) na
**Intel PresentMon** uruchamiany jako proces potomny - tak jak robi to CapFrameX.

Status: **propozycja do akceptacji** (nowy plik binarny w paczce = wymaga zgody właściciela, patrz `AI_CONTEXT.md`).

---

## 1. Problem

Pomiar FPS "zacina się": wartości FPS / czas klatki skaczą, a wykres czasu klatki stoi i przeskakuje
porcjami, mimo że gra działa płynnie.

## 2. Diagnoza - sprawdzić PRZED migracją (krok 0)

Najbardziej prawdopodobna przyczyna nie leży w samej metodzie (zdarzenia `Present` z ETW), tylko
w **dostarczaniu zdarzeń w paczkach**:

- Sesja ETW czasu rzeczywistego oddaje zdarzenia dopiero, gdy bufor się zapełni albo minie
  `FlushTimer`. W `properties_buffer()` jest `FlushTimer = 1` (w sekundach), a zdarzeń `Present`
  jest mało bajtowo (kilkanaście kB/s przy 144 FPS), więc bufor praktycznie nigdy się nie zapełnia.
  W efekcie **klatki przychodzą porcjami co ~1 s**.
- `compute_stats` liczy "bieżący FPS" z okna `(teraz - 1 s, teraz]` względem **zegara odczytu**,
  a nie ostatniej odebranej klatki. Gdy paczka jeszcze nie dotarła, w oknie jest za mało klatek
  (FPS spada), a gdy dotrze - nagle wszystkie (FPS wraca). To daje skoki.
- `recent_frames()` odświeża wykres co 33 ms, ale nowe dane pojawiają się raz na sekundę,
  więc wykres stoi i przeskakuje.

**Ważne:** PresentMon też korzysta z sesji ETW czasu rzeczywistego, a jego wyjście czytane przez
potok (pipe) może być dodatkowo buforowane. Sama migracja **nie musi** usunąć zacinania, jeśli
przyczyną jest opisane wyżej dostarczanie w paczkach.

### Szybka poprawka do przetestowania (mała zmiana, bez nowych zależności)

> **Wdrożona 2026-11-24** (pkt 1 i 2, `FlushTimer` = 50 ms, bez zmiany `BufferSize`) -
> czeka na test w grze (pkt 3).

1. W `properties_buffer()`: dodać do `LogFileMode` flagę `EVENT_TRACE_USE_MS_FLUSH_TIMER`
   (`0x10`, Windows 8+) i ustawić `FlushTimer` na np. `50` - wtedy wartość jest w **milisekundach**.
   Opcjonalnie zmniejszyć `BufferSize` (KB), żeby bufory zapełniały się szybciej.
2. W `compute_stats`: liczyć okna względem znacznika **ostatniej klatki** zamiast `now`
   (albo `now` minus znane opóźnienie dostarczenia), żeby opóźnienie nie zaniżało FPS.
3. Test: gra z ograniczeniem FPS (np. 60) - FPS powinien stać równo na 60, a wykres przesuwać
   się płynnie.

Jeśli to usunie zacinanie, migracja na PresentMon jest potrzebna już tylko ze względu na
zasięg (Vulkan/OpenGL, klatki wyświetlone) - patrz niżej.

---

## 3. Co daje PresentMon (powody migracji niezależne od zacinania)

| | Obecnie (własne ETW) | PresentMon |
|---|---|---|
| DX9 / DX10-12 | tak | tak |
| Vulkan / OpenGL (natywna prezentacja) | tylko gdy sterownik prezentuje przez DXGI | tak (DxgKrnl, Win32k) |
| Klatki wyświetlone vs wysłane | tylko wysłane (`MsBetweenPresents`) | oba + zgubione klatki |
| Tryb prezentacji, opóźnienia, GPU busy | nie | tak |
| Zgodność liczb z CapFrameX | przybliżona | 1:1 (to samo źródło) |
| Utrzymanie logiki śledzenia klatek | po naszej stronie | po stronie Intela |

Koszty: dodatkowy plik `PresentMon.exe` w paczce, zarządzanie procesem potomnym, zależność od
formatu CSV konkretnej wersji. Administrator nadal wymagany.

---

## 4. Architektura po zmianie

```
ArtVitals (admin)
 └─ wątek "sensors"
     └─ PresentMonSource::start()
         ├─ CreateProcess(PresentMon.exe, CREATE_NO_WINDOW) w Job Object (KILL_ON_JOB_CLOSE)
         ├─ wątek "presentmon-reader": BufReader(stdout) -> parsowanie wiersza CSV
         │      -> FrameLog::record_frame(pid, czas_klatki_ms, znacznik)
         └─ read() / recent_frames()  - bez zmian dla reszty programu
```

Co zostaje bez zmian:

- `FrameLog` (zmiana tylko wejścia: gotowy czas klatki zamiast znacznika `Present`),
- `compute_stats`, `FpsReading`, wybór procesu (`measured_frames`, `foreground_pid`),
- `sensors/mod.rs`, `metrics.rs`, UI.

Co znika: kod sesji ETW (`properties_buffer`, `on_event`, `StartTraceW` itd.). Zostawić go za
przełącznikiem w konfiguracji jako **źródło zapasowe**, dopóki PresentMon nie zostanie sprawdzony
w kilku grach (zasada: bez usuwania istniejących funkcji).

---

## 5. Plan krok po kroku

### Krok 1 - wybór i przypięcie wersji PresentMon
- Pobrać konkretne wydanie z `github.com/GameTechDev/PresentMon` (licencja MIT), wersja **konsolowa**
  (nie aplikacja z overlayem / usługa).
- Zapisać numer wersji i sumę SHA-256 w kodzie (stała) i w `CHANGELOG.md`.
- Dołożyć `assets/PresentMon.exe` + `assets/PresentMon.LICENSE.txt` (jak przy `IntelMSR.bin`).
- Sprawdzić `PresentMon.exe --help` tej wersji - nazwy opcji i kolumn poniżej są do weryfikacji.

### Krok 2 - uruchamianie procesu
Parametry (do potwierdzenia w `--help`):
- `--output_stdout` - wiersze CSV na standardowe wyjście,
- `--no_console_stats` - bez statystyk w konsoli,
- `--session_name ArtVitals-PresentMon` - własna nazwa sesji (nie koliduje z CapFrameX itp.),
- `--stop_existing_session` - sprząta sesję po awarii poprzedniego uruchomienia,
- `--v1_metrics` - jeśli chcemy klasyczne kolumny (`MsBetweenPresents`, `MsBetweenDisplayChange`).

Szczegóły:
- `std::process::Command` z `CREATE_NO_WINDOW` (`std::os::windows::process::CommandExt`).
- Proces w **Job Object** z `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` - PresentMon zginie razem z
  ArtVitals nawet po awarii (moduł `Win32_System_JobObjects` crate'a `windows` - bez nowych crate'ów).
- Przy zamykaniu (`quit`, `Drop` w `app.rs`) zamiast/oprócz `stop_session()` zatrzymać proces
  i jego sesję ETW.

### Krok 3 - czytanie i parsowanie CSV
- Osobny wątek: `BufReader::new(child.stdout).lines()`.
- Pierwszy wiersz = nagłówek -> mapa `nazwa kolumny -> indeks` (odporne na kolejność kolumn).
- Potrzebne kolumny: `ProcessID`, `Application`, znacznik czasu (`TimeInSeconds` / `CPUStartTime`),
  czas klatki (`MsBetweenPresents` lub `FrameTime`), opcjonalnie `MsBetweenDisplayChange` / `DisplayedTime`.
- Proste `split(',')` wystarczy, jeśli nazwy aplikacji nie zawierają przecinków - inaczej
  dopisać minimalny parser cudzysłowów zamiast nowej zależności (`csv` = nowy crate, wymaga zgody).
- Błędne wiersze pomijać z `log::debug!`, nie przerywać pomiaru.

### Krok 4 - podpięcie pod istniejący model
- Nowa struktura `PresentMonSource` z tym samym API co `FpsMonitor`: `start()`, `read(window_s)`,
  `recent_frames()`.
- Wspólny trait albo enum `FrameSource { Etw(FpsMonitor), PresentMon(PresentMonSource) }`
  w `sensors/mod.rs` - jedna zmiana w miejscu tworzenia (linia z `FpsMonitor::start()`).
- Błąd uruchomienia (brak pliku, brak admina) -> `SensorData::issues`, automatyczny powrót do ETW.

### Krok 5 - konfiguracja
- `config.toml`: `fps_source = "presentmon" | "etw"` (domyślnie `presentmon` po testach).
- Opcja w oknie konfiguracji (sekcja pomiaru FPS) + migracja w `Config::sanitize`.
- Opcjonalnie: wybór "klatki wysłane / wyświetlone" (`MsBetweenPresents` vs `MsBetweenDisplayChange`).

### Krok 6 - opóźnienie i płynność (to, co ma usunąć zacinanie)
- Zmierzyć opóźnienie: różnica między znacznikiem klatki a chwilą odczytu wiersza. Jeśli
  wiersze przychodzą paczkami (bufor potoku / ETW), zastosować poprawkę z rozdziału 2 pkt 2
  (okna liczone od ostatniej klatki).
- Sprawdzić, czy PresentMon opróżnia stdout po każdym wierszu, gdy wyjście idzie do potoku.

### Krok 7 - testy
- Jednostkowe: parser CSV na wierszach z prawdziwego nagrania (nagłówek v1 i v2), wiersze uszkodzone,
  przecinek w nazwie aplikacji.
- Istniejące testy `compute_stats` bez zmian.
- Ręczne, porównanie z CapFrameX na tych samych przebiegach:
  - gra DX11 i DX12 (z ograniczeniem FPS i bez),
  - gra Vulkan (natywna) i OpenGL,
  - przełączanie okien (Alt+Tab) - wybór procesu,
  - zamknięcie / zabicie ArtVitals - czy PresentMon i sesja ETW znikają.
- `cargo clippy --all-targets` bez ostrzeżeń, `cargo test`.

### Krok 8 - dokumentacja i wydanie
- `AI_CONTEXT.md`: sekcja czujników ("ETW (FPS)" -> "PresentMon / ETW"), pułapki z kroku 6.
- `README.md`: licencja PresentMon, wymaganie admina.
- `CHANGELOG.md` + podbicie wersji (zmiana funkcjonalna -> `0.5.0`).

---

## 6. Ryzyka

| Ryzyko | Co z tym zrobić |
|---|---|
| Zacinanie zostaje (dostarczanie w paczkach) | Krok 0 przed migracją; okna liczone od ostatniej klatki |
| Zmiana formatu CSV w nowej wersji | Przypięta wersja + mapowanie kolumn po nagłówku |
| Osierocony proces / sesja ETW po awarii | Job Object + `--stop_existing_session` |
| Antywirus podejrzliwy wobec ukrytego procesu potomnego | Podpisany plik z oficjalnego wydania, sprawdzanie SHA-256 |
| Inne narzędzie (CapFrameX) mierzy równolegle | Własna nazwa sesji; limit sesji ETW w systemie to 64 |
| Większa paczka | ~kilka MB, akceptowalne |

## 7. Decyzje do podjęcia przez właściciela

1. Zgoda na dołączenie `PresentMon.exe` (nowy plik binarny).
2. Czy najpierw wypróbować szybką poprawkę z rozdziału 2 (zalecane - mała zmiana, może wystarczyć).
3. Domyślne źródło po migracji i czy zostawić ETW jako zapasowe (zalecane: tak).
4. Klatki wysłane czy wyświetlone jako domyślny "czas klatki".
