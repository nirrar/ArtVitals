//! Ikony Phosphor (wariant Bold) jako font wkompilowany w program.
//!
//! Zrodlo: paczka `@phosphor-icons/web` 2.1.2, licencja MIT (assets/Phosphor.LICENSE.txt).
//! Font jest rejestrowany w egui jako osobna rodzina `ICON_FAMILY`, a ikona to pojedynczy
//! znak z prywatnego obszaru Unicode - kody wziete z `src/bold/style.css` tej paczki.

use eframe::egui::{self, FontData, FontFamily, FontInsert, FontPriority, epaint::text::InsertFontFamily};

use crate::config::WidgetIcon;

const PHOSPHOR_BOLD: &[u8] = include_bytes!("../assets/Phosphor-Bold.ttf");
pub const ICON_FAMILY: &str = "phosphor";

const CPU: char = '\u{e610}';
const GRAPHICS_CARD: char = '\u{e612}';
const MEMORY: char = '\u{e9c4}';
const THERMOMETER: char = '\u{e5cc}';
const FIRE: char = '\u{e242}';
const GAUGE: char = '\u{e628}';
const LIGHTNING: char = '\u{e2de}';
const FAN: char = '\u{e9f2}';
const GAME_CONTROLLER: char = '\u{e26e}';
const TIMER: char = '\u{e492}';
pub const GEAR: char = '\u{e272}';
pub const CHECK: char = '\u{e182}';

/// Rejestruje font z ikonami - wywolywane raz przy starcie aplikacji.
pub fn install(ctx: &egui::Context) {
    ctx.add_font(FontInsert {
        name: ICON_FAMILY.to_owned(),
        data: FontData::from_static(PHOSPHOR_BOLD),
        families: vec![
            InsertFontFamily { family: icon_family(), priority: FontPriority::Highest },
            // Zapasowo dla zwyklego tekstu - ikony (np. CHECK) mozna wtedy wstawiac w napisy.
            InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Lowest },
        ],
    });
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name(ICON_FAMILY.into())
}

pub fn glyph(icon: WidgetIcon) -> Option<char> {
    match icon {
        WidgetIcon::None => None,
        WidgetIcon::Cpu => Some(CPU),
        WidgetIcon::Gpu => Some(GRAPHICS_CARD),
        WidgetIcon::Memory => Some(MEMORY),
        WidgetIcon::Thermometer => Some(THERMOMETER),
        WidgetIcon::Fire => Some(FIRE),
        WidgetIcon::Gauge => Some(GAUGE),
        WidgetIcon::Lightning => Some(LIGHTNING),
        WidgetIcon::Fan => Some(FAN),
        WidgetIcon::GameController => Some(GAME_CONTROLLER),
        WidgetIcon::Timer => Some(TIMER),
    }
}
