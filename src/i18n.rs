//! Tlumaczenia interfejsu: polski i angielski.
//!
//! Teksty sa wpisane w miejscu uzycia parami `tr("polski", "English")` - bez plikow z kluczami.
//! Biezacy jezyk jest globalny (atomik, jak `config::text_scale()`), bo napisy powstaja w wielu
//! miejscach bez dostepu do `Config`. Ustawia go `Config::sanitize` / `Config::apply_language`
//! na podstawie pola `language` (auto = jezyk interfejsu Windows: polski -> PL, reszta -> EN).
//! Zaleznosci: serde (wybor jezyka w config.toml), windows (Win32_Globalization).

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

/// Jezyk, w ktorym wyswietlany jest interfejs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Pl,
    En,
}

/// Wybor jezyka zapisywany w config.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageSetting {
    /// Wedlug jezyka interfejsu Windows.
    #[default]
    Auto,
    Pl,
    En,
}

impl LanguageSetting {
    pub const ALL: [LanguageSetting; 3] = [LanguageSetting::Auto, LanguageSetting::Pl, LanguageSetting::En];

    pub fn label(self) -> &'static str {
        match self {
            LanguageSetting::Auto => "Auto (system)",
            // Nazwy jezykow zawsze we wlasnym jezyku, zeby dalo sie wrocic z obcego.
            LanguageSetting::Pl => "Polski",
            LanguageSetting::En => "English",
        }
    }

    /// Jezyk wynikajacy z ustawienia (auto = wykrycie z systemu).
    pub fn resolve(self) -> Lang {
        match self {
            LanguageSetting::Auto => system_lang(),
            LanguageSetting::Pl => Lang::Pl,
            LanguageSetting::En => Lang::En,
        }
    }
}

/// Glowny identyfikator jezyka polskiego (LANG_POLISH) w LANGID.
const LANG_POLISH: u16 = 0x15;

/// Polski system -> polski, kazdy inny -> angielski. Dolne 10 bitow LANGID to jezyk glowny.
pub fn lang_from_langid(langid: u16) -> Lang {
    if langid & 0x3FF == LANG_POLISH { Lang::Pl } else { Lang::En }
}

/// Jezyk interfejsu uzytkownika Windows.
pub fn system_lang() -> Lang {
    // SAFETY: funkcja bez argumentow, zwraca LANGID.
    let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    lang_from_langid(langid)
}

fn encode(lang: Lang) -> u8 {
    match lang {
        Lang::Pl => 0,
        Lang::En => 1,
    }
}

fn decode(value: u8) -> Lang {
    if value == 1 { Lang::En } else { Lang::Pl }
}

// W programie jezyk jest globalny. W testach (rownoleglych) - osobny dla kazdego watku,
// zeby test przelaczajacy jezyk nie zmienial napisow w innych testach.
#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    static CURRENT: AtomicU8 = const { AtomicU8::new(0) };
}

/// Biezacy jezyk interfejsu.
pub fn current() -> Lang {
    #[cfg(not(test))]
    let value = CURRENT.load(Ordering::Relaxed);
    #[cfg(test)]
    let value = CURRENT.with(|c| c.load(Ordering::Relaxed));
    decode(value)
}

pub fn set_current(lang: Lang) {
    #[cfg(not(test))]
    CURRENT.store(encode(lang), Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.store(encode(lang), Ordering::Relaxed));
}

/// Tekst w biezacym jezyku. Uzycie: `tr("Krycie", "Opacity")`; dla tekstow dynamicznych
/// tlumaczy sie szablon albo kawalki, np. `format!("{} {n}", tr("Karta", "Card"))`.
pub fn tr(pl: &'static str, en: &'static str) -> &'static str {
    match current() {
        Lang::Pl => pl,
        Lang::En => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn langid_maps_polish_only_to_polish() {
        assert_eq!(lang_from_langid(0x0415), Lang::Pl); // pl-PL
        assert_eq!(lang_from_langid(0x0409), Lang::En); // en-US
        assert_eq!(lang_from_langid(0x0809), Lang::En); // en-GB
        assert_eq!(lang_from_langid(0x0407), Lang::En); // de-DE
        assert_eq!(lang_from_langid(0x0000), Lang::En);
    }

    #[test]
    fn tr_follows_current_language() {
        set_current(Lang::Pl);
        assert_eq!(tr("Krycie", "Opacity"), "Krycie");
        set_current(Lang::En);
        assert_eq!(tr("Krycie", "Opacity"), "Opacity");
        assert_eq!(crate::metrics::Metric::CpuTemp.label(), "CPU temperature");
        set_current(Lang::Pl);
        assert_eq!(crate::metrics::Metric::CpuTemp.label(), "Temperatura CPU");
    }

    #[test]
    fn explicit_setting_resolves_without_system() {
        assert_eq!(LanguageSetting::Pl.resolve(), Lang::Pl);
        assert_eq!(LanguageSetting::En.resolve(), Lang::En);
    }
}
