# Historia zmian - ArtVitals

Wersjonowanie semantyczne (`MAJOR.MINOR.PATCH`), numer w `Cargo.toml`:
- **PATCH** - poprawki błędów bez nowych funkcji,
- **MINOR** - nowe funkcje zgodne wstecz (stary `config.toml` dalej działa),
- **MAJOR** - zmiany niezgodne wstecz (np. format konfiguracji wymagający ręcznej migracji).

Wersja jest widoczna w oknie konfiguracji (przy tytule), w podpowiedzi ikony w zasobniku
i w pierwszej linii `overlay.log` (obok exe w wersji portable albo w `%APPDATA%\ArtVitals`).

## [0.6.0] - 2026-11-24
### Dodane
- Licencja MPL-2.0 (plik `LICENSE`) i repozytorium https://github.com/nirrar/ArtVitals.
- Zegary od 1 GHz wyświetlane w GHz (np. 2.73 GHz, 10.7 GHz), poniżej 1 GHz dalej w MHz.
- Karta „Wentylatory”: własna nazwa każdego wentylatora (np. „Pompa AIO” dla złącza płyty)
  i bieżące obroty przy każdym wentylatorze na liście w ustawieniach karty.
- Tło podglądu ekranu w oknie konfiguracji ze zrzutu z gry (z pliku PNG/JPG/BMP albo ze schowka po
  PrintScreen / Win+Shift+S) - łatwiej ustawić karty względem HUD-u gry. Zrzut jest zapisywany obok konfiguracji.
- Kolor tła kart: gotowe ciemne odcienie lub własny kolor, przycisk „Wszystkim kartom”
  (kolor i krycie tła na wszystkich kartach); ramka karty dobierana do koloru.
- Wersja portable: `config.toml` i `overlay.log` w folderze z plikiem exe, jeśli da się w nim
  zapisywać (inaczej, np. w Program Files, dalej `%APPDATA%\ArtVitals`). Przy pierwszym starcie
  konfiguracja jest kopiowana z `%APPDATA%\ArtVitals`. Zmienna `ARTVITALS_CONFIG_DIR` wymusza katalog.
- Pomiar FPS gier Vulkan i OpenGL: zdarzenia jądra grafiki (DxgKrnl Present_Info) dla procesów,
  które nie prezentują przez DXGI/D3D9 (bez podwójnego liczenia klatek gier DirectX).
- Karta „Wentylatory”: obroty pomp, wentylatorów AIO, płyty głównej i GPU z animowanym wirnikiem
  (szybkość obrotu zależna od RPM), wypełnienie PWM, temperatura cieczy AIO; wybór pokazywanych
  wentylatorów w ustawieniach karty.
- Temperatura i pobór mocy procesorów AMD (wymaga PawnIO i administratora, jak dla Intela):
  - Ryzen / Threadripper / EPYC (Zen, rodziny 17h, 19h, 1Ah) - temperatura pakietu „Tctl/Tdie”
    z rejestru SMN THM_TCON_CUR_TMP (z korektą -49 °C i przesunięciem Tdie dla 1600X/1700X/1800X/
    2700X/Threadripper 1000/2000, jak LibreHardwareMonitor) oraz pobór mocy pakietu z liczników RAPL AMD.
  - Starsze AMD (rodziny 10h-16h: Phenom, Athlon II, FX, APU A-series; oraz 0Fh - Athlon 64) -
    tylko temperatura (rejestry PCI / SMU, jak LibreHardwareMonitor i `k10temp`).
  - Nieobsługiwany producent lub rodzina daje czytelny komunikat w oknie „Czujniki”.
- Producent i rodzina procesora rozpoznawane przez CPUID; Intel działa bez zmian.
- Nowe moduły PawnIO.Modules 0.2.12 (to samo wydanie co `IntelMSR.bin`, LGPL-2.1), SHA-256:
  - `AMDFamily17.bin` - `dae74615761b78bdf064dfb3e136252ddcc6fc727d88f14738d0e5800d427a91`
  - `AMDFamily10.bin` - `6443080b2968474ffbc38aa4356cc56f9664349fa4b917afdb33027d6bb50525`
  - `AMDFamily0F.bin` - `a6e11619e87a97820705a6523714f22d676ce44f902631833d4429b89d509d55`
  - (`IntelMSR.bin` - `30a5a302283de1e080dfee99e686eca91d7e53e911095cb91a320fb961f1ce12`)
- Karty graficzne AMD Radeon (ADL ze sterownika, `atiadlxx.dll`): temperatura, hotspot,
  temperatura pamięci, obciążenie, zegary rdzenia i pamięci, pobór mocy, wentylator (% i RPM), VRAM.
- Karty graficzne Intel Arc i zintegrowane (Intel Graphics Control Library, `ControlLib.dll`):
  temperatura GPU i pamięci, obciążenie, zegary, pobór mocy, wentylator, VRAM (karty z własną pamięcią).
- Obroty wentylatora GPU NVIDIA (RPM) przez NVAPI, z rezerwą w NVML.
- Automatyczny wybór karty: dedykowana przed zintegrowaną, przy kilku dedykowanych kolejność
  NVIDIA, AMD, Intel. Okno „Czujniki” wyjaśnia, którego odczytu (hotspot / pamięć) brakuje i dlaczego.
- Interfejs po angielsku. Język wybierany automatycznie z języka interfejsu Windows (polski system
  -> polski, każdy inny -> angielski) albo ręcznie: Ogólne -> „Język / Language” (Auto (system) /
  Polski / English), zmiana działa od razu (okno konfiguracji, menu zasobnika, napisy na kartach).
  Nowe pole `language` w `config.toml` (`auto` / `pl` / `en`, domyślnie `auto`) - stare pliki
  wczytują się bez zmian.
- Przetłumaczone także komunikaty czujników w oknie „Czujniki” (CPU, PawnIO, FPS) i okno
  „program już działa”, a także komunikaty czujników GPU i wentylatorów.
- Odczyt wentylatorów płyty głównej z układu Super I/O (tylko odczyt, bez sterowania): Nuvoton
  NCT6771F-NCT6799D, NCT6701D, NCT6102D/6104D/6106D, NCT6683D/NCT6686D/NCT6687D (także wariant
  płyt MSI serii 800) oraz ITE IT86xx/IT87xx, porty 0x2E i 0x4E. Wykrywanie i przeliczanie obrotów
  przeniesione z LibreHardwareMonitor (MPL-2.0); dostęp do szyny ISA synchronizowany wspólnym
  mutexem `Global\Access_ISABUS.HTP.Method`, jak w innych narzędziach. Wymaga PawnIO i administratora.
- Moduł `assets/LpcIO.bin` z PawnIO.Modules 0.2.12 (LGPL-2.1, `assets/LpcIO.LICENSE.txt`),
  SHA-256 `b3896a1cab0d808fca31fe2ebcae045d59dac690da87b17c858bb8da357eb45e`
  (paczka `release_0_2_12.zip`: `09698897ce19b92f1043a2e322cfcf0b1fe823e0b576971b178a65244fb0af9d`).
- Odczyt chłodzeń AIO przez USB HID (pompa, wentylatory, temperatura cieczy - tylko odczyt):
  NZXT Kraken X42/X52/X62/X72, X53/X63/X73, Z53/Z63/Z73, Kraken 2023 / 2023 Elite / Elite v2
  (na podstawie LibreHardwareMonitor, bez ustawiania pompy przy starcie) oraz Corsair Commander
  Core / Commander ST (Elite Capellix, Elite LCD) i Commander Core XT - pomijane, gdy działa iCUE.
  Urządzenia otwierane współdzielone, odczyt z limitem czasu ~45 ms na urządzenie.
- Nowa zależność: crate `hidapi` 2.6.7 (MIT).

### Naprawione
- Krótka etykieta użytej pamięci RAM to „RAM GB” (wcześniej dwa razy „RAM” na jednej karcie).
- Indeksy sensorów NVAPI dobierane jak w LibreHardwareMonitor (prefiks „NVIDIA GeForce RTX 40/50”),
  a dla kart bez „GeForce” w nazwie - wg architektury z NVML. Wcześniej np. „RTX 5000 Ada”
  dostawała układ kart RTX 50 (brak hotspotu, zła temperatura pamięci).
- Przy kilku kartach NVIDIA odczyty NVAPI pochodzą z tej samej karty co NVML (dopasowanie po szynie PCI).
- Opis progów kolorów karty FPS: „fPS” -> „FPS” (skróty na początku nazwy metryki zostają
  wielkimi literami).

## [0.5.0] - 2026-11-24
### Dodane
- Powiększenie tekstu kart (Ogólne → Overlay → „Powiększenie tekstu”, +0 do +10 pt względem
  tytułu karty, reszta napisów proporcjonalnie). Karty rosną razem z tekstem - tylko o miejsce
  zajęte przez napisy, więc wykresy i marginesy zachowują proporcje; ręcznie zmieniony rozmiar
  karty jest przeliczany proporcjonalnie.
- Pamięć RAM: metryki „Zajętość RAM” (%) i „Użyta pamięć RAM” (GB). Nowa karta RAM od razu
  pokazuje oba odczyty. Nie wymaga administratora.

## [0.4.1] - 2026-11-23

### Zmienione
- Średni FPS i 1% / 0.1% low liczone z osobnego, krótszego okna (domyślnie 10 s, ustawienie
  „Okno średniej i lows” w Ogólnych, 2-120 s) zamiast z całej historii wykresu (60 s) -
  przy 60 s wartości wyglądały na zamrożone, bo pojedyncza sekunda prawie ich nie zmienia.
- Wykres czasu klatki odświeżany ~30 razy na sekundę (gdy gra wyświetla klatki) zamiast raz
  na interwał odczytu - przesuwa się płynnie zamiast skokami.

### Naprawione
- Zacinanie pomiaru FPS: ETW oddawało zdarzenia klatek paczkami co 1 s, przez co FPS skakał,
  a wykres czasu klatki stał i przeskakiwał. Sesja ETW opróżnia teraz bufory co 50 ms,
  a bieżący FPS / lows są liczone od ostatniej odebranej klatki zamiast od chwili odczytu.
- Wysokie zużycie CPU przy otwartym oknie konfiguracji (35-70% rdzenia): animacje wartości
  przerysowywały oba okna z częstotliwością monitora. Animacje są teraz ograniczone do ~30 klatek/s.
- Szara linia 1 px na górnej krawędzi ekranu przy działającym programie (przesunięcie obszaru okna
  bez ramki w winit) - okno overlaya ma teraz obszar dokładnie równy ekranowi.

## [0.4.0] - 2026-11-23

### Dodane
- Pomiar klatek przez ETW (jak PresentMon): FPS, średni FPS, 1% low, 0.1% low i czas klatki
  dla gry z aktywnym oknem. Obsługuje DirectX 9-12 (Vulkan/OpenGL, gdy sterownik prezentuje
  przez DXGI). Wymaga uruchomienia jako administrator.
- Wykres czasu klatki rysowany klatka po klatce (ostatnie 240 klatek), nie z próbek co sekundę.
- Karta FPS od razu ze średnią i 1% low; okno „Czujniki” pokazuje, który proces jest mierzony.
- Ikony: pad (FPS) i stoper (czas klatki).

### Zmienione
- Progi kolorów dla FPS działają w dół („poniżej” zamiast „od”) - kolor ostrzegawczy przy spadku.
- Czas klatki z jednym miejscem po przecinku (np. 6.9 ms).

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
