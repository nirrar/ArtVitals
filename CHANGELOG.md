# Historia zmian - ArtVitals

Wersjonowanie semantyczne (`MAJOR.MINOR.PATCH`), numer w `Cargo.toml`:
- **PATCH** - poprawki błędów bez nowych funkcji,
- **MINOR** - nowe funkcje zgodne wstecz (stary `config.toml` dalej działa),
- **MAJOR** - zmiany niezgodne wstecz (np. format konfiguracji wymagający ręcznej migracji).

Wersja jest widoczna w oknie konfiguracji (przy tytule), w podpowiedzi ikony w zasobniku
i w pierwszej linii `%APPDATA%\ArtVitals\overlay.log`.

## [0.3.0] - 2026-11-23

### Dodane
- Nowe metryki GPU: hotspot, temperatura pamięci (NVAPI, jak w LibreHardwareMonitor), zegar rdzenia,
  zegar pamięci, pobór mocy, wentylator.
- Nowe metryki CPU: zegar (jak w Menedżerze zadań) i pobór mocy pakietu (liczniki energii RAPL, wymaga admina).
- Przy dodawaniu metryki do wykresu wybór: na wykresie głównym (linia we własnej skali), osobny wykres
  albo tylko wartość; tryb można zmienić później przy każdej metryce.
- Nowe ikony: płomień (hotspot), licznik (zegary), błyskawica (moc), wentylator.
- Przycisk „Anuluj” w ustawieniach ogólnych - przywraca stan z otwarcia okna (także skróty i motyw).

### Zmienione
- Nowa nazwa: **ArtVitals** (wcześniej CraftGamingBoard). Konfiguracja jest automatycznie
  przenoszona z `%APPDATA%\CraftGamingBoard` do `%APPDATA%\ArtVitals`.
- Czytelniejsza karta „Wykres”: więcej niż 2 dodatkowe wartości trafiają do siatki pod nagłówkiem
  (kropka = metryka ma linię na wykresie), osobne wykresy mają etykietę i wartość w pasku nad wykresem
  i większe odstępy.
- Podgląd w oknie ustawień karty rośnie razem z kartą (dodatkowe metryki nie ściskają podglądu).
- Okno nowej karty ma przyciski „Anuluj” / „Dodaj” - „Dodaj” faktycznie dodaje kartę.
- Okno edycji karty ma „Anuluj” (cofa zmiany od otwarcia okna) i „Gotowe” (zatwierdza).
- Zegary nie mają progów kolorów (wysoki zegar to normalna praca).
- Dodatkowe metryki zapisywane jako `[[widgets.extra]]` z polem `chart`; stary format
  (`extra = ["cpu_temp"]`, `plot_extra`) jest wczytywany i przenoszony automatycznie.

### Naprawione
- Trzecia i kolejna dodatkowa wartość nie była widoczna na karcie (kolumna w nagłówku mieści 2).
- Etykiety i wartości osobnych wykresów nachodziły na krzywą.

## [0.2.0] - 2026-11-23

### Dodane
- Ikony Phosphor (Bold, licencja MIT) na kartach i w oknie konfiguracji.
- Ikona w zasobniku systemowym: lewy klik otwiera konfigurację, menu pod prawym
  (konfiguracja, pokaż/ukryj overlay, zamknij program).
- Blokada drugiej instancji - ponowne uruchomienie otwiera okno konfiguracji działającego programu.
- Dowolna liczba dodatkowych metryk na karcie (tylko z tego samego podzespołu: CPU albo GPU),
  we wszystkich typach kart: linie na wykresie, paski, pierścienie wskaźnika, kolumna wartości.
- Drugi globalny skrót (pokaż/ukryj overlay w grze); skróty można nagrać albo ułożyć z klocków.
- Okno konfiguracji w stylu grupowanych list z wyskakującymi oknami nad rozmytym tłem
  i żywym podglądem konfigurowanej karty.
- Powiększanie i przesuwanie podglądu ekranu.

### Zmienione
- Dodatkowa metryka zawsze ma etykietę (np. „CPU TEMP”).
- Progi kolorów w ustawieniach karty opisują, której metryki dotyczą (np. „Progi kolorów: Obciążenie CPU”).
- Okno ustawień ma ~40% szerokości i co najmniej 70% wysokości okna programu.
- Wyższa domyślna karta „Pasek” - pasek nie nachodzi już na wartość.
- Stary klucz `secondary` w `config.toml` jest automatycznie przenoszony do listy `extra`.

### Naprawione
- Overlay był niewidoczny (styl okna warstwowego bez atrybutów).
- Szara ramka i cień wokół ekranu przy działającym overlayu.
- Druga instancja kasowała plik logu pierwszej.
- Wyskakujące okno otwierało się przewinięte; pasek przewijania zasłaniał kontrolki.

## [0.1.0] - 2026-11-23

### Dodane
- Pierwsza wersja: przezroczysty overlay click-through nad grą, karty CPU/GPU
  (wykres, wskaźnik, pasek, liczba), własne czujniki (PDH, PawnIO, NVML),
  okno konfiguracji z przeciąganiem i przyciąganiem kart, zapis do `config.toml`
  z przeładowaniem na żywo.
