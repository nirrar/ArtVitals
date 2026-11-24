//! Obroty wentylatorow plyty glownej z ukladu Super I/O (Nuvoton NCT67xx/NCT668x, ITE IT86xx/IT87xx).
//!
//! Dostep do portow przez sterownik PawnIO i jego modul `LpcIO.bin` (PawnIO.Modules 0.2.12,
//! LGPL-2.1, wkompilowany w exe; SHA-256 `b3896a1cab0d808fca31fe2ebcae045d59dac690da87b17c858bb8da357eb45e`).
//! Funkcje modulu (z `LpcIO.p`): `ioctl_select_slot` (0 = porty 0x2E/0x2F, 1 = 0x4E/0x4F),
//! `ioctl_find_bars` (po wejsciu w tryb konfiguracji - odblokowuje porty bazowe urzadzen logicznych),
//! `ioctl_pio_inb/outb` (port), `ioctl_superio_inb/inw/outb` (rejestr konfiguracyjny).
//! Stan modulu (wybrany port, BAR-y) jest per uchwyt, dlatego kazdy port LPC ma osobny `PawnIo`.
//!
//! Wykrywanie ukladow i odczyt obrotow przeniesione z LibreHardwareMonitor (MPL-2.0,
//! https://github.com/LibreHardwareMonitor/LibreHardwareMonitor, commit 9e2d677c):
//! `Hardware/Motherboard/Lpc/LpcIO.cs`, `LpcPort.cs`, `Nct677X.cs`, `IT87XX.cs`, `Chip.cs`,
//! domyslne nazwy kanalow z `SuperIOHardware.cs`. Pominiete: Winbond, Fintek, sciezki EC
//! (kontroler Gigabyte, EC ASUS) i cale sterowanie wentylatorami.
//!
//! TYLKO ODCZYT obrotow i wypelnienia PWM. Jedyne zapisy (jak w LHM) dotycza konfiguracji
//! monitorowania, nie predkosci: wejscie/wyjscie z trybu konfiguracji, wybor urzadzenia
//! logicznego, wylaczenie blokady przestrzeni I/O monitora (NCT679x), bit startu monitorowania
//! w NCT668x (0x180) i wybor banku 0 w IT8655E/IT8665E.
//!
//! Dostep do szyny ISA jest chroniony globalnym mutexem `Global\Access_ISABUS.HTP.Method`
//! (ten sam co w LHM, HWiNFO i innych narzedziach), zeby nie mieszac sekwencji port/dane.
//! Wymaga PawnIO i uprawnien administratora; bez nich zwraca opis problemu.

use std::ffi::CStr;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::Security::{
    InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
    SetSecurityDescriptorDacl,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Threading::{
    CreateMutexW, MUTEX_MODIFY_STATE, OpenMutexW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::w;

use super::{FanKind, FanReading};
use crate::i18n::tr;
use crate::sensors::pawnio::PawnIo;

const LPC_IO_MODULE: &[u8] = include_bytes!("../../../assets/LpcIO.bin");

/// Ile czekamy na mutex szyny ISA (jak LHM: 100 ms przy wykrywaniu, 10 ms przy odczycie).
const DETECT_LOCK_TIMEOUT_MS: u32 = 100;
const READ_LOCK_TIMEOUT_MS: u32 = 10;
/// LHM czeka na strone EC w NCT668x do 500 ms (z `Thread.Sleep(1)`). U nas aktywne odpytywanie
/// przez 3 ms - `sleep(1 ms)` na Windows trwa nawet ~16 ms, a odczytow strony jest kilkadziesiat na cykl.
const EC_PAGE_WAIT: Duration = Duration::from_millis(3);

// Rejestry konfiguracyjne Super I/O (LpcIO.cs / LpcPort.cs).
const CONFIGURATION_CONTROL_REGISTER: u8 = 0x02;
const DEVICE_SELECT_REGISTER: u8 = 0x07;
const CHIP_ID_REGISTER: u8 = 0x20;
const CHIP_REVISION_REGISTER: u8 = 0x21;
const IT87_CHIP_VERSION_REGISTER: u8 = 0x22;
const NUVOTON_HARDWARE_MONITOR_IO_SPACE_LOCK: u8 = 0x28;
const LOGICAL_DEVICE_ACTIVATE_REGISTER: u8 = 0x30;
const BASE_ADDRESS_REGISTER: u8 = 0x60;
const ALTERNATE_BASE_ADDRESS_REGISTER: u8 = 0x64;
const WINBOND_NUVOTON_HARDWARE_MONITOR_LDN: u8 = 0x0B;
const IT87_ENVIRONMENT_CONTROLLER_LDN: u8 = 0x04;
const IT8705_GPIO_LDN: u8 = 0x05;
const IT87XX_GPIO_LDN: u8 = 0x07;

// Nct677X.cs
const NCT_ADDRESS_REGISTER_OFFSET: u16 = 0x05;
const NCT_DATA_REGISTER_OFFSET: u16 = 0x06;
const NCT_BANK_SELECT_REGISTER: u8 = 0x4E;
const NCT_EC_SPACE_PAGE_REGISTER_OFFSET: u16 = 0x04;
const NCT_EC_SPACE_INDEX_REGISTER_OFFSET: u16 = 0x05;
const NCT_EC_SPACE_DATA_REGISTER_OFFSET: u16 = 0x06;
const NCT_EC_SPACE_PAGE_SELECT: u8 = 0xFF;
const NUVOTON_VENDOR_ID: u16 = 0x5CA3;
const NCT_FAN_COUNT_REGISTERS: [u16; 7] = [0x4B0, 0x4B2, 0x4B4, 0x4B6, 0x4B8, 0x4BA, 0x4CC];
const NCT_MAX_FAN_COUNT: u16 = 0x1FFF;
const NCT_MIN_FAN_COUNT: u16 = 0x15;
const NCT668X_INIT_REGISTER: u16 = 0x180;
/// Rejestr licznika 13-bit w NCT668x (SYS_FAN7 na niektorych plytach MSI).
const NCT668X_COUNT_REGISTER: u16 = 0x852;
/// Znacznik "brak rejestru" w tablicach LHM dla NCT6687D-R.
const NCT_NO_REGISTER: u16 = 0xFFF;

// IT87XX.cs
const IT87_ADDRESS_REGISTER_OFFSET: u16 = 0x05;
const IT87_DATA_REGISTER_OFFSET: u16 = 0x06;
const IT87_CONFIGURATION_REGISTER: u8 = 0x00;
const IT87_BANK_REGISTER: u8 = 0x06;
const IT87_FAN_TACHOMETER_16BIT_REGISTER: u8 = 0x0C;
const IT87_FAN_TACHOMETER_16BIT_REGISTER_ALT: u8 = 0x0B;
const IT87_FAN_TACHOMETER_DIVISOR_REGISTER: u8 = 0x0B;
const IT87_VENDOR_ID_REGISTER: u8 = 0x58;
const ITE_VENDOR_IDS: [u8; 2] = [0x90, 0x7F];
const IT87_FAN_PWM_CTRL_EXT_REG: [u8; 6] = [0x63, 0x6b, 0x73, 0x7b, 0xa3, 0xab];
const IT87_FAN_TACHOMETER_EXT_REG: [u8; 6] = [0x18, 0x19, 0x1a, 0x81, 0x83, 0x4d];
const IT87_FAN_TACHOMETER_REG: [u8; 6] = [0x0d, 0x0e, 0x0f, 0x80, 0x82, 0x4c];
const IT87_FAN_TACHOMETER_EXT_REG_ALT: [u8; 6] = [0x18, 0x19, 0x1a, 0x81, 0x83, 0x94];
const IT87_FAN_TACHOMETER_REG_ALT: [u8; 6] = [0x0d, 0x0e, 0x0f, 0x80, 0x82, 0x93];

const REGISTER_PORTS: [u16; 2] = [0x2E, 0x4E];

/// Obslugiwane uklady (podzbior `Chip.cs` z LHM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chip {
    Nct610x,
    Nct6771F,
    Nct6776F,
    Nct6779D,
    Nct6791D,
    Nct6792D,
    Nct6792DA,
    Nct6793D,
    Nct6795D,
    Nct6796D,
    /// ID 0xD42A; na niektorych plytach to NCT5585D - dla wentylatorow bez roznicy.
    Nct6796DR,
    Nct6797D,
    Nct6798D,
    /// ID 0xD802; na niektorych plytach to NCT6796D-S - dla wentylatorow bez roznicy.
    Nct6799D,
    Nct6701D,
    Nct6683D,
    Nct6686D,
    Nct6687D,
    /// NCT6687D na plytach MSI serii 800 (AM5/LGA1851) - inny uklad rejestrow.
    Nct6687DR,
    It8613E,
    It8620E,
    It8625E,
    It8628E,
    It8631E,
    It8638E,
    It8655E,
    It8665E,
    It8686E,
    It8688E,
    It8689E,
    It8696E,
    It8705F,
    It8712F,
    It8716F,
    It8718F,
    It8720F,
    It8721F,
    It8726F,
    It8728F,
    It8771E,
    It8772E,
    It8790E,
    /// ID 0x8733 - moze to byc tez IT8791E lub IT8795E.
    It8792E,
    It87952E,
}

/// Uklad Nuvoton po ID i rewizji (switch z `LpcIO.DetectWinbondFintek`, bez Winbond/Fintek).
pub(crate) fn nuvoton_chip(id: u8, revision: u8) -> Option<Chip> {
    use Chip::*;
    let chip = match (id, revision) {
        (0xB4, r) if r & 0xF0 == 0x70 => Nct6771F,
        (0xC3, r) if r & 0xF0 == 0x30 => Nct6776F,
        (0xC4, r) if r & 0xF0 == 0x50 => Nct610x,
        (0xC5, r) if r & 0xF0 == 0x60 => Nct6779D,
        (0xC7, 0x32) => Nct6683D,
        (0xC8, 0x03) => Nct6791D,
        (0xC9, 0x11) => Nct6792D,
        (0xC9, 0x13) => Nct6792DA,
        (0xD1, 0x21) => Nct6793D,
        (0xD3, 0x52) => Nct6795D,
        (0xD4, 0x23) => Nct6796D,
        (0xD4, 0x2A) => Nct6796DR,
        (0xD4, 0x51) => Nct6797D,
        (0xD4, 0x2B) => Nct6798D,
        (0xD4, 0x40 | 0x41) => Nct6686D,
        (0xD5, 0x92) => Nct6687D,
        (0xD8, 0x02) => Nct6799D,
        (0xD8, 0x06) => Nct6701D,
        _ => return None,
    };
    Some(chip)
}

/// Uklad ITE po 16-bitowym ID (`LpcIO.DetectIT87`).
pub(crate) fn ite_chip(id: u16) -> Option<Chip> {
    use Chip::*;
    let chip = match id {
        0x8613 => It8613E,
        0x8620 => It8620E,
        0x8625 => It8625E,
        0x8628 => It8628E,
        0x8631 => It8631E,
        0x8638 => It8638E,
        0x8665 => It8665E,
        0x8655 => It8655E,
        0x8686 => It8686E,
        0x8688 => It8688E,
        0x8689 => It8689E,
        0x8696 => It8696E,
        0x8705 => It8705F,
        0x8712 => It8712F,
        0x8716 => It8716F,
        0x8718 => It8718F,
        0x8720 => It8720F,
        0x8721 => It8721F,
        0x8726 => It8726F,
        0x8728 => It8728F,
        0x8771 => It8771E,
        0x8772 => It8772E,
        0x8790 => It8790E,
        0x8733 => It8792E,
        0x8695 => It87952E,
        _ => return None,
    };
    Some(chip)
}

impl Chip {
    pub(crate) fn display_name(self) -> &'static str {
        use Chip::*;
        match self {
            Nct610x => "Nuvoton NCT6102D/NCT6104D/NCT6106D",
            Nct6771F => "Nuvoton NCT6771F",
            Nct6776F => "Nuvoton NCT6776F",
            Nct6779D => "Nuvoton NCT6779D",
            Nct6791D => "Nuvoton NCT6791D",
            Nct6792D => "Nuvoton NCT6792D",
            Nct6792DA => "Nuvoton NCT6792D-A",
            Nct6793D => "Nuvoton NCT6793D",
            Nct6795D => "Nuvoton NCT6795D",
            Nct6796D => "Nuvoton NCT6796D",
            Nct6796DR => "Nuvoton NCT6796D-R",
            Nct6797D => "Nuvoton NCT6797D",
            Nct6798D => "Nuvoton NCT6798D",
            Nct6799D => "Nuvoton NCT6799D",
            Nct6701D => "Nuvoton NCT6701D",
            Nct6683D => "Nuvoton NCT6683D",
            Nct6686D => "Nuvoton NCT6686D",
            Nct6687D => "Nuvoton NCT6687D",
            Nct6687DR => "Nuvoton NCT6687D-R",
            It8613E => "ITE IT8613E",
            It8620E => "ITE IT8620E",
            It8625E => "ITE IT8625E",
            It8628E => "ITE IT8628E",
            It8631E => "ITE IT8631E",
            It8638E => "ITE IT8638E",
            It8655E => "ITE IT8655E",
            It8665E => "ITE IT8665E",
            It8686E => "ITE IT8686E",
            It8688E => "ITE IT8688E",
            It8689E => "ITE IT8689E",
            It8696E => "ITE IT8696E",
            It8705F => "ITE IT8705F",
            It8712F => "ITE IT8712F",
            It8716F => "ITE IT8716F",
            It8718F => "ITE IT8718F",
            It8720F => "ITE IT8720F",
            It8721F => "ITE IT8721F",
            It8726F => "ITE IT8726F",
            It8728F => "ITE IT8728F",
            It8771E => "ITE IT8771E",
            It8772E => "ITE IT8772E",
            It8790E => "ITE IT8790E",
            It8792E => "ITE IT8791E/IT8792E/IT8795E",
            It87952E => "ITE IT87952E",
        }
    }

    /// Fragment stabilnego id (`superio:<slug>:fanN`) - nie zmieniac, konfiguracja kart go zapamietuje.
    pub(crate) fn slug(self) -> &'static str {
        use Chip::*;
        match self {
            Nct610x => "nct610x",
            Nct6771F => "nct6771",
            Nct6776F => "nct6776",
            Nct6779D => "nct6779",
            Nct6791D => "nct6791",
            Nct6792D => "nct6792",
            Nct6792DA => "nct6792a",
            Nct6793D => "nct6793",
            Nct6795D => "nct6795",
            Nct6796D => "nct6796",
            Nct6796DR => "nct6796r",
            Nct6797D => "nct6797",
            Nct6798D => "nct6798",
            Nct6799D => "nct6799",
            Nct6701D => "nct6701",
            Nct6683D => "nct6683",
            Nct6686D => "nct6686",
            Nct6687D => "nct6687",
            Nct6687DR => "nct6687r",
            It8613E => "it8613",
            It8620E => "it8620",
            It8625E => "it8625",
            It8628E => "it8628",
            It8631E => "it8631",
            It8638E => "it8638",
            It8655E => "it8655",
            It8665E => "it8665",
            It8686E => "it8686",
            It8688E => "it8688",
            It8689E => "it8689",
            It8696E => "it8696",
            It8705F => "it8705",
            It8712F => "it8712",
            It8716F => "it8716",
            It8718F => "it8718",
            It8720F => "it8720",
            It8721F => "it8721",
            It8726F => "it8726",
            It8728F => "it8728",
            It8771E => "it8771",
            It8772E => "it8772",
            It8790E => "it8790",
            It8792E => "it8792",
            It87952E => "it87952",
        }
    }

    fn is_ite(self) -> bool {
        self.slug().starts_with("it")
    }

    /// NCT668x - dostep przez strony EC zamiast bankow.
    fn is_nct668x(self) -> bool {
        matches!(self, Chip::Nct6683D | Chip::Nct6686D | Chip::Nct6687D | Chip::Nct6687DR)
    }

    /// Uklady z blokada przestrzeni I/O monitora, ktora LHM wylacza (`NuvotonDisableIOSpaceLock`).
    fn has_io_space_lock(self) -> bool {
        use Chip::*;
        matches!(
            self,
            Nct6791D
                | Nct6792D
                | Nct6792DA
                | Nct6793D
                | Nct6795D
                | Nct6796D
                | Nct6796DR
                | Nct6797D
                | Nct6798D
                | Nct6799D
                | Nct6701D
        )
    }
}

/// Opis rejestrow wentylatorow ukladu Nuvoton z rodziny NCT677x (dostep przez banki).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Nct677xLayout {
    pub vendor_registers: (u16, u16),
    /// Rejestry 16-bit RPM (starsze uklady) albo None - wtedy liczniki 13-bit `NCT_FAN_COUNT_REGISTERS`.
    pub rpm_registers: Option<Vec<u16>>,
    pub min_rpm: u32,
    pub fan_count: usize,
    pub pwm_out_registers: Vec<u16>,
}

/// Uklad rejestrow z konstruktora `Nct677X` (tylko wentylatory i wyjscia PWM).
pub(crate) fn nct677x_layout(chip: Chip) -> Option<Nct677xLayout> {
    use Chip::*;
    const GENERIC_PWM: [u16; 7] = [0x001, 0x003, 0x011, 0x013, 0x015, 0x017, 0x029];
    const NEW_PWM: [u16; 7] = [0x001, 0x003, 0x011, 0x013, 0x015, 0xA09, 0xB09];
    let vendor = (0x804F, 0x004F);
    let layout = match chip {
        Nct610x => Nct677xLayout {
            vendor_registers: (0x80FE, 0x00FE),
            rpm_registers: Some((0..3).map(|i| 0x030 + (i << 1)).collect()),
            min_rpm: 1_350_000 / 0x1FFF,
            fan_count: 3,
            pwm_out_registers: vec![0x04A, 0x04B, 0x04C],
        },
        Nct6771F | Nct6776F => Nct677xLayout {
            vendor_registers: vendor,
            rpm_registers: Some((0..5).map(|i| 0x656 + (i << 1)).collect()),
            // 16-bit licznik w NCT6771F, 13-bit w NCT6776F.
            min_rpm: if chip == Nct6771F { 1_350_000 / 0xFFFF } else { 1_350_000 / 0x1FFF },
            fan_count: if chip == Nct6771F { 4 } else { 5 },
            pwm_out_registers: GENERIC_PWM[..3].to_vec(),
        },
        Nct6779D => Nct677xLayout {
            vendor_registers: vendor,
            rpm_registers: None,
            min_rpm: 0,
            fan_count: 5,
            pwm_out_registers: GENERIC_PWM[..5].to_vec(),
        },
        Nct6791D | Nct6792D | Nct6792DA | Nct6793D | Nct6795D | Nct6796D => Nct677xLayout {
            vendor_registers: vendor,
            rpm_registers: None,
            min_rpm: 0,
            fan_count: 6,
            pwm_out_registers: GENERIC_PWM[..6].to_vec(),
        },
        Nct6796DR | Nct6701D => Nct677xLayout {
            vendor_registers: vendor,
            rpm_registers: None,
            min_rpm: 0,
            fan_count: 7,
            pwm_out_registers: GENERIC_PWM.to_vec(),
        },
        Nct6797D | Nct6798D | Nct6799D => Nct677xLayout {
            vendor_registers: vendor,
            rpm_registers: None,
            min_rpm: 0,
            fan_count: 7,
            pwm_out_registers: NEW_PWM.to_vec(),
        },
        _ => return None,
    };
    Some(layout)
}

/// Rejestry obrotow (16-bit RPM, licznik 13-bit tylko pod 0x852) i wyjsc PWM NCT668x.
pub(crate) fn nct668x_registers(chip: Chip) -> (Vec<u16>, Vec<u16>) {
    if chip == Chip::Nct6687DR {
        (
            vec![
                0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x15E, 0x15C, 0x15A, 0x158,
                0x156, 0x154,
            ],
            vec![
                0x160,
                0x161,
                0x162,
                0x163,
                0x164,
                0x165,
                0x166,
                0x167,
                NCT_NO_REGISTER,
                0xC93,
                0xE05,
                0xE04,
                0xE03,
                0xE02,
                0xE01,
                0xE00,
            ],
        )
    } else {
        let mut rpm: Vec<u16> = (0..16).map(|i| 0x140 + 2 * i).collect();
        rpm.push(NCT668X_COUNT_REGISTER);
        (rpm, (0x160..=0x167).collect())
    }
}

/// Nazwa kanalu bez opisu na plycie: "Wentylator N" (numeracja od 1).
fn generic_fan_name(index: usize) -> String {
    format!("{} {}", tr("Wentylator", "Fan"), index + 1)
}

/// Kanaly NCT668x zawsze pokazywane wraz z nazwami (domyslna konfiguracja z `SuperIOHardware.cs`).
/// Pozostale kanaly sa dodawane tylko, gdy przy starcie pokazuja obroty > 0.
pub(crate) fn nct668x_default_channels(chip: Chip) -> Vec<(usize, String)> {
    let system = |n: usize| format!("{} {n}", tr("Wentylator systemowy", "System fan"));
    let cpu_fan = || tr("Wentylator CPU", "CPU fan").to_owned();
    let board_pump = || tr("Pompa (złącze płyty)", "Pump (board header)").to_owned();
    match chip {
        Chip::Nct6686D | Chip::Nct6687D => {
            let mut list = vec![(0, cpu_fan()), (1, board_pump())];
            list.extend((2..8).map(|i| (i, system(i - 1))));
            list
        }
        Chip::Nct6687DR => {
            let mut list = vec![
                (0, cpu_fan()),
                (1, board_pump()),
                (2, tr("Wentylator chipsetu", "Chipset fan").to_owned()),
                (3, tr("Wentylator EZ-Connect", "EZ-Connect fan").to_owned()),
            ];
            list.extend((10..16).map(|i| (i, system(i - 9))));
            list
        }
        _ => (0..8).map(|i| (i, generic_fan_name(i))).collect(),
    }
}

/// Uklad rejestrow ITE (konstruktor `IT87XX`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct It87Layout {
    pub fan_count: usize,
    pub control_count: usize,
    pub pwm_ctrl_registers: Vec<u8>,
    pub has_ext_reg: bool,
    pub alt_6th_fan_reg: bool,
    pub requires_bank_select: bool,
}

pub(crate) fn it87_layout(chip: Chip) -> It87Layout {
    use Chip::*;
    let (fan_count, control_count) = match chip {
        It8613E => (5, 4),
        It8625E | It8628E | It8665E | It8689E | It8696E => (6, 6),
        It8686E | It8688E => (6, 5),
        It8631E | It8638E => (2, 2),
        It87952E | It8655E | It8792E | It8705F => (3, 3),
        It8620E => (5, 5),
        _ => (5, 3),
    };
    let pwm_ctrl_registers = match chip {
        It8665E | It8625E => vec![0x15, 0x16, 0x17, 0x1e, 0x1f, 0x92],
        It8792E => vec![0x15, 0x16, 0x17],
        _ => vec![0x15, 0x16, 0x17, 0x7f, 0xa7, 0xaf],
    };
    let has_ext_reg = matches!(
        chip,
        It8721F
            | It8728F
            | It8665E
            | It8686E
            | It8688E
            | It8689E
            | It87952E
            | It8628E
            | It8625E
            | It8620E
            | It8613E
            | It8792E
            | It8655E
            | It8631E
            | It8638E
            | It8696E
            | It8772E
    );
    It87Layout {
        fan_count,
        control_count: control_count.min(pwm_ctrl_registers.len()),
        pwm_ctrl_registers,
        has_ext_reg,
        alt_6th_fan_reg: matches!(chip, It8665E | It8625E),
        requires_bank_select: matches!(chip, It8665E | It8655E),
    }
}

/// Licznik 13-bit Nuvoton -> RPM (`Update13BitFan`): None = odczyt niewiarygodny, 0 = brak obrotow.
pub(crate) fn nct_count_to_rpm(high: u8, low: u8, nct6687d: bool) -> Option<f32> {
    if nct6687d && high == 0xFF && low == 0xF8 {
        return Some(0.0);
    }
    let count = (u16::from(high) << 5) | u16::from(low & 0x1F);
    if count < NCT_MAX_FAN_COUNT { (count >= NCT_MIN_FAN_COUNT).then(|| 1.35e6 / f32::from(count)) } else { Some(0.0) }
}

/// Rejestr 16-bit RPM w NCT6771F/NCT6776F/NCT610x: wartosc ponizej minimum = 0.
pub(crate) fn nct_rpm16(high: u8, low: u8, min_rpm: u32) -> f32 {
    let value = (u32::from(high) << 8) | u32::from(low);
    if value > min_rpm { value as f32 } else { 0.0 }
}

/// Licznik 16-bit ITE -> RPM.
pub(crate) fn it87_rpm16(value: u16) -> Option<f32> {
    if value > 0x3f { Some(if value < 0xffff { 1.35e6 / (f32::from(value) * 2.0) } else { 0.0 }) } else { None }
}

/// Licznik 8-bit ITE (stare IT8705F/IT8712F) -> RPM.
pub(crate) fn it87_rpm8(value: u8, divisor: u32) -> Option<f32> {
    if value > 0 { Some(if value < 0xff { 1.35e6 / (f32::from(value) * divisor as f32) } else { 0.0 }) } else { None }
}

/// Wypelnienie PWM ITE: tryb automatyczny (bit 7) = brak wartosci.
pub(crate) fn it87_duty(control: u8, ext: Option<u8>) -> Option<f32> {
    if control & 0x80 != 0 {
        return None;
    }
    Some(match ext {
        Some(ext) => (f32::from(ext) * 100.0 / 255.0).round(),
        None => (f32::from(control & 0x7F) * 100.0 / 127.0).round(),
    })
}

/// Adres bazowy niepoprawny (`LpcIO.IsInvalidRuntimeBase`).
fn is_invalid_base(address: u16) -> bool {
    address < 0x100 || (address & 0xF007) != 0
}

// ---------------------------------------------------------------------------------------------
// Mutex szyny ISA

/// Globalny mutex wspoldzielony z innymi narzedziami monitorujacymi (LHM: `Mutexes.cs`).
struct IsaBusMutex(Option<HANDLE>);

impl IsaBusMutex {
    fn open() -> Self {
        let name = w!("Global\\Access_ISABUS.HTP.Method");
        unsafe {
            // Pusta lista DACL = dostep dla wszystkich (LHM tworzy go z pelnym dostepem dla "Everyone").
            let mut descriptor = SECURITY_DESCRIPTOR::default();
            let psd = PSECURITY_DESCRIPTOR((&raw mut descriptor).cast());
            // SECURITY_DESCRIPTOR_REVISION = 1
            let sd_ok = InitializeSecurityDescriptor(psd, 1).is_ok()
                && SetSecurityDescriptorDacl(psd, true, None, false).is_ok();
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: psd.0,
                bInheritHandle: false.into(),
            };
            let created = CreateMutexW(sd_ok.then_some(&raw const attributes), false, name);
            let handle = created
                .or_else(|_| OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE, false, name))
                .map_err(|err| log::warn!("Mutex szyny ISA niedostepny ({err}) - odczyt bez synchronizacji"))
                .ok();
            Self(handle)
        }
    }

    /// Jak `Mutexes.WaitMutex` w LHM: brak mutexu = wolna droga, porzucony mutex = przejety.
    fn lock(&self, timeout_ms: u32) -> Option<IsaBusGuard<'_>> {
        let Some(handle) = self.0 else {
            return Some(IsaBusGuard { mutex: self });
        };
        let result = unsafe { WaitForSingleObject(handle, timeout_ms) };
        (result == WAIT_OBJECT_0 || result == WAIT_ABANDONED).then_some(IsaBusGuard { mutex: self })
    }
}

impl Drop for IsaBusMutex {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

struct IsaBusGuard<'a> {
    mutex: &'a IsaBusMutex,
}

impl Drop for IsaBusGuard<'_> {
    fn drop(&mut self) {
        if let Some(handle) = self.mutex.0 {
            unsafe {
                let _ = ReleaseMutex(handle);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Port LPC przez modul LpcIO (odpowiednik `LpcPort.cs` + `PawnIo/LpcIO.cs`)

struct LpcPort {
    io: PawnIo,
    register_port: u16,
}

impl LpcPort {
    fn open(register_port: u16) -> Result<Self, String> {
        let io = PawnIo::open(LPC_IO_MODULE, "LpcIO")?;
        let slot = if register_port == 0x2E { 0 } else { 1 };
        io.execute(c"ioctl_select_slot", &[slot], 0)?;
        Ok(Self { io, register_port })
    }

    fn value_port(&self) -> u16 {
        self.register_port + 1
    }

    fn call(&self, function: &CStr, input: &[u64]) -> Result<(), String> {
        self.io.execute(function, input, 0).map(|_| ())
    }

    fn read_port(&self, port: u16) -> Result<u8, String> {
        Ok(self.io.execute_one(c"ioctl_pio_inb", &[u64::from(port)])? as u8)
    }

    fn write_port(&self, port: u16, value: u8) -> Result<(), String> {
        self.call(c"ioctl_pio_outb", &[u64::from(port), u64::from(value)])
    }

    fn read_byte(&self, register: u8) -> Result<u8, String> {
        Ok(self.io.execute_one(c"ioctl_superio_inb", &[u64::from(register)])? as u8)
    }

    fn read_word(&self, register: u8) -> Result<u16, String> {
        Ok(self.io.execute_one(c"ioctl_superio_inw", &[u64::from(register)])? as u16)
    }

    fn write_byte(&self, register: u8, value: u8) -> Result<(), String> {
        self.call(c"ioctl_superio_outb", &[u64::from(register), u64::from(value)])
    }

    fn find_bars(&self) -> Result<(), String> {
        self.call(c"ioctl_find_bars", &[])
    }

    fn select(&self, logical_device: u8) -> Result<(), String> {
        self.write_byte(DEVICE_SELECT_REGISTER, logical_device)
    }

    fn nuvoton_enter(&self) -> Result<(), String> {
        self.write_port(self.register_port, 0x87)?;
        self.write_port(self.register_port, 0x87)
    }

    fn nuvoton_exit(&self) -> Result<(), String> {
        self.write_port(self.register_port, 0xAA)
    }

    fn nuvoton_disable_io_space_lock(&self) -> Result<(), String> {
        let options = self.read_byte(NUVOTON_HARDWARE_MONITOR_IO_SPACE_LOCK)?;
        if options & 0x10 != 0 {
            self.write_byte(NUVOTON_HARDWARE_MONITOR_IO_SPACE_LOCK, options & !0x10)?;
        }
        Ok(())
    }

    fn it87_enter(&self) -> Result<(), String> {
        let port = self.register_port;
        self.write_port(port, 0x87)?;
        self.write_port(port, 0x01)?;
        self.write_port(port, 0x55)?;
        self.write_port(port, if port == 0x4E { 0xAA } else { 0x55 })
    }

    fn it87_exit(&self) -> Result<(), String> {
        // Drugiego ukladu (0x4E) LHM celowo nie wyprowadza z trybu konfiguracji.
        if self.register_port != 0x4E {
            self.write_port(self.register_port, CONFIGURATION_CONTROL_REGISTER)?;
            self.write_port(self.value_port(), 0x02)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Wykryty uklad

/// Odczyt kanalu: (rpm, wypelnienie PWM w %).
type Sample = (Option<f32>, Option<f32>);

/// Kanal wentylatora pokazywany w programie.
struct Channel {
    /// Indeks w tablicy `Fans` LHM.
    index: usize,
    id: String,
    name: String,
}

enum Backend {
    Nct677x(Nct677xLayout),
    Nct668x { rpm_registers: Vec<u16>, pwm_registers: Vec<u16> },
    It87 { layout: It87Layout, has_16bit: bool, fans_disabled: Vec<bool> },
}

struct SuperIoChip {
    port: LpcPort,
    chip: Chip,
    base: u16,
    backend: Backend,
    channels: Vec<Channel>,
    /// Ostatnie odczyty per kanal - zostaja, gdy mutex jest zajety.
    last: Vec<Sample>,
    error_logged: bool,
}

impl SuperIoChip {
    // --- Nuvoton NCT677x: banki
    fn nct_bank_read(&self, address: u16) -> Result<u8, String> {
        let [bank, register] = address.to_be_bytes();
        let p = &self.port;
        p.write_port(self.base + NCT_ADDRESS_REGISTER_OFFSET, NCT_BANK_SELECT_REGISTER)?;
        p.write_port(self.base + NCT_DATA_REGISTER_OFFSET, bank)?;
        p.write_port(self.base + NCT_ADDRESS_REGISTER_OFFSET, register)?;
        p.read_port(self.base + NCT_DATA_REGISTER_OFFSET)
    }

    // --- Nuvoton NCT668x: strony EC (z oczekiwaniem na wolna strone jak w LHM, ale krocej)
    fn nct_ec_wait_page(&self) -> Result<(), String> {
        let page_port = self.base + NCT_EC_SPACE_PAGE_REGISTER_OFFSET;
        let deadline = Instant::now() + EC_PAGE_WAIT;
        loop {
            if self.port.read_port(page_port)? == NCT_EC_SPACE_PAGE_SELECT {
                return Ok(());
            }
            if Instant::now() >= deadline {
                // Jak LHM: po czasie przejmujemy dostep na sile.
                return self.port.write_port(page_port, NCT_EC_SPACE_PAGE_SELECT);
            }
            std::hint::spin_loop();
        }
    }

    fn nct_ec_read(&self, address: u16) -> Result<u8, String> {
        let [page, index] = address.to_be_bytes();
        let page_port = self.base + NCT_EC_SPACE_PAGE_REGISTER_OFFSET;
        self.nct_ec_wait_page()?;
        self.port.write_port(page_port, page)?;
        self.port.write_port(self.base + NCT_EC_SPACE_INDEX_REGISTER_OFFSET, index)?;
        let value = self.port.read_port(self.base + NCT_EC_SPACE_DATA_REGISTER_OFFSET);
        // Zwolnienie strony dla innych programow - takze po bledzie odczytu.
        let release = self.port.write_port(page_port, NCT_EC_SPACE_PAGE_SELECT);
        let value = value?;
        release?;
        Ok(value)
    }

    fn nct_ec_write(&self, address: u16, data: u8) -> Result<(), String> {
        let [page, index] = address.to_be_bytes();
        let page_port = self.base + NCT_EC_SPACE_PAGE_REGISTER_OFFSET;
        self.nct_ec_wait_page()?;
        self.port.write_port(page_port, page)?;
        self.port.write_port(self.base + NCT_EC_SPACE_INDEX_REGISTER_OFFSET, index)?;
        let written = self.port.write_port(self.base + NCT_EC_SPACE_DATA_REGISTER_OFFSET, data);
        let release = self.port.write_port(page_port, NCT_EC_SPACE_PAGE_SELECT);
        written?;
        release
    }

    fn nct_is_vendor(&self, layout: &Nct677xLayout) -> Result<bool, String> {
        let (high, low) = layout.vendor_registers;
        let id = (u16::from(self.nct_bank_read(high)?) << 8) | u16::from(self.nct_bank_read(low)?);
        Ok(id == NUVOTON_VENDOR_ID)
    }

    // --- ITE
    /// Odczyt z kontrola echa rejestru adresu (`IT87XX.ReadByte`).
    fn it87_read(&self, register: u8) -> Result<(u8, bool), String> {
        let address_port = self.base + IT87_ADDRESS_REGISTER_OFFSET;
        self.port.write_port(address_port, register)?;
        let value = self.port.read_port(self.base + IT87_DATA_REGISTER_OFFSET)?;
        // IT8688E nie oddaje zapisanego adresu przy odczycie zwrotnym.
        let valid = self.port.read_port(address_port)? == register || self.chip == Chip::It8688E;
        Ok((value, valid))
    }

    fn it87_write(&self, register: u8, value: u8) -> Result<(), String> {
        let address_port = self.base + IT87_ADDRESS_REGISTER_OFFSET;
        self.port.write_port(address_port, register)?;
        self.port.write_port(self.base + IT87_DATA_REGISTER_OFFSET, value)?;
        self.port.read_port(address_port).map(|_| ())
    }

    /// Wybor banku 0 (`IT87XX.SelectBank(0)`), wymagany przez IT8655E/IT8665E.
    fn it87_select_bank0(&self) -> Result<(), String> {
        let (value, valid) = self.it87_read(IT87_BANK_REGISTER)?;
        if valid {
            self.it87_write(IT87_BANK_REGISTER, value & 0x9F)?;
        }
        Ok(())
    }

    /// Odczyt wszystkich kanalow ukladu: (rpm, wypelnienie) wg indeksu LHM. Wymaga mutexu szyny ISA.
    fn read_all(&self) -> Result<Vec<Sample>, String> {
        match &self.backend {
            Backend::Nct677x(layout) => self.read_nct677x(layout),
            Backend::Nct668x { rpm_registers, pwm_registers } => self.read_nct668x(rpm_registers, pwm_registers),
            Backend::It87 { layout, has_16bit, fans_disabled } => self.read_it87(layout, *has_16bit, fans_disabled),
        }
    }

    fn read_nct677x(&self, layout: &Nct677xLayout) -> Result<Vec<Sample>, String> {
        // `DisableIOSpaceLock` z LHM: blokada wraca np. po uspieniu - wylaczana, gdy ID producenta znika.
        if self.chip.has_io_space_lock() && !self.nct_is_vendor(layout)? {
            self.port.nuvoton_enter()?;
            let unlocked = self.port.nuvoton_disable_io_space_lock();
            self.port.nuvoton_exit()?;
            unlocked?;
        }
        let mut out = Vec::with_capacity(layout.fan_count);
        for i in 0..layout.fan_count {
            let rpm = match &layout.rpm_registers {
                Some(registers) => {
                    let high = self.nct_bank_read(registers[i])?;
                    let low = self.nct_bank_read(registers[i] + 1)?;
                    Some(nct_rpm16(high, low, layout.min_rpm))
                }
                None => {
                    let high = self.nct_bank_read(NCT_FAN_COUNT_REGISTERS[i])?;
                    let low = self.nct_bank_read(NCT_FAN_COUNT_REGISTERS[i] + 1)?;
                    nct_count_to_rpm(high, low, false)
                }
            };
            let duty = match layout.pwm_out_registers.get(i) {
                Some(&register) => Some(f32::from(self.nct_bank_read(register)?) / 2.55),
                None => None,
            };
            out.push((rpm, duty));
        }
        Ok(out)
    }

    fn read_nct668x(&self, rpm_registers: &[u16], pwm_registers: &[u16]) -> Result<Vec<Sample>, String> {
        let mut out = Vec::with_capacity(rpm_registers.len());
        for (i, &register) in rpm_registers.iter().enumerate() {
            // Pomijamy kanaly, ktorych i tak nie pokazujemy - mniej dostepu do EC.
            if !self.channels.iter().any(|c| c.index == i) && !self.channels.is_empty() {
                out.push((None, None));
                continue;
            }
            let high = self.nct_ec_read(register)?;
            let low = self.nct_ec_read(register + 1)?;
            let rpm = if register == NCT668X_COUNT_REGISTER {
                nct_count_to_rpm(high, low, self.chip == Chip::Nct6687D)
            } else {
                Some(f32::from((u16::from(high) << 8) | u16::from(low)))
            };
            let duty = match pwm_registers.get(i) {
                Some(&pwm) if pwm != NCT_NO_REGISTER => Some((f32::from(self.nct_ec_read(pwm)?) / 2.55).round()),
                _ => None,
            };
            out.push((rpm, duty));
        }
        Ok(out)
    }

    fn read_it87(&self, layout: &It87Layout, has_16bit: bool, fans_disabled: &[bool]) -> Result<Vec<Sample>, String> {
        // Bank moze sie zmienic po wybudzeniu z uspienia - LHM wybiera go przy kazdym odczycie.
        if layout.requires_bank_select {
            self.it87_select_bank0()?;
        }
        let mut out = vec![(None, None); layout.fan_count];
        for (i, slot) in out.iter_mut().enumerate() {
            if has_16bit {
                if fans_disabled[i] {
                    continue;
                }
                let (low_reg, high_reg) = if layout.alt_6th_fan_reg {
                    (IT87_FAN_TACHOMETER_REG_ALT[i], IT87_FAN_TACHOMETER_EXT_REG_ALT[i])
                } else {
                    (IT87_FAN_TACHOMETER_REG[i], IT87_FAN_TACHOMETER_EXT_REG[i])
                };
                let (low, valid) = self.it87_read(low_reg)?;
                if !valid {
                    continue;
                }
                let (high, valid) = self.it87_read(high_reg)?;
                if !valid {
                    continue;
                }
                slot.0 = it87_rpm16(u16::from(low) | (u16::from(high) << 8));
            } else {
                let (value, valid) = self.it87_read(IT87_FAN_TACHOMETER_REG[i])?;
                if !valid {
                    continue;
                }
                let mut divisor = 2;
                if i < 2 {
                    let (divisors, valid) = self.it87_read(IT87_FAN_TACHOMETER_DIVISOR_REGISTER)?;
                    if !valid {
                        continue;
                    }
                    divisor = 1u32 << ((u32::from(divisors) >> (3 * i)) & 0x7);
                }
                slot.0 = it87_rpm8(value, divisor);
            }
        }
        for i in 0..layout.control_count.min(out.len()) {
            let (control, valid) = self.it87_read(layout.pwm_ctrl_registers[i])?;
            if !valid {
                continue;
            }
            let ext = if layout.has_ext_reg && control & 0x80 == 0 {
                match self.it87_read(IT87_FAN_PWM_CTRL_EXT_REG[i])? {
                    (value, true) => Some(value),
                    (_, false) => continue,
                }
            } else {
                None
            };
            out[i].1 = it87_duty(control, ext);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// Wykrywanie (odpowiednik `LpcIO.Detect`)

/// Uklad znaleziony na porcie, jeszcze bez kontroli z konstruktora `Nct677X`/`IT87XX`.
struct Draft {
    chip: Chip,
    base: u16,
    it87_version: u8,
}

enum Detection {
    Found(Draft),
    /// Uklad jest, ale nie obslugujemy go albo nie przeszedl weryfikacji - opis do problemow.
    Rejected(String),
    Nothing,
}

/// Plyta MSI serii 800 (AM5/LGA1851) - tam NCT6687D ma inny uklad rejestrow (`NCT6687DR` w LHM).
fn is_msi_800_series_board() -> bool {
    let manufacturer = read_bios_string(w!("BaseBoardManufacturer")).unwrap_or_default();
    let product = read_bios_string(w!("BaseBoardProduct")).unwrap_or_default().to_uppercase();
    let is_msi = manufacturer.contains("Micro-Star") || manufacturer.to_uppercase().starts_with("MSI");
    is_msi && ["B840", "B850", "B860", "X870", "Z890"].iter().any(|series| product.contains(series))
}

/// Dane SMBIOS plyty z rejestru (`HKLM\HARDWARE\DESCRIPTION\System\BIOS`).
fn read_bios_string(value: windows::core::PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("HARDWARE\\DESCRIPTION\\System\\BIOS"),
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]).trim().to_owned())
}

fn detect_nuvoton(port: &LpcPort) -> Result<Detection, String> {
    port.nuvoton_enter()?;
    let id = port.read_byte(CHIP_ID_REGISTER)?;
    let revision = port.read_byte(CHIP_REVISION_REGISTER)?;
    let Some(mut chip) = nuvoton_chip(id, revision) else {
        if id != 0 && id != 0xFF {
            port.nuvoton_exit()?;
            let report = format!(
                "{} (ID 0x{:04X}) {} 0x{:X}",
                tr("nieobsługiwany układ Winbond/Nuvoton/Fintek", "unsupported Winbond/Nuvoton/Fintek chip"),
                (u16::from(id) << 8) | u16::from(revision),
                tr("na porcie", "on port"),
                port.register_port
            );
            return Ok(Detection::Rejected(report));
        }
        return Ok(Detection::Nothing);
    };
    if chip == Chip::Nct6687D && is_msi_800_series_board() {
        chip = Chip::Nct6687DR;
    }

    port.find_bars()?;
    port.select(WINBOND_NUVOTON_HARDWARE_MONITOR_LDN)?;
    // LHM w tym miejscu wlacza nieaktywne urzadzenie monitora NCT6701D - my tylko czytamy.
    if chip == Chip::Nct6701D && port.read_byte(LOGICAL_DEVICE_ACTIVATE_REGISTER)? == 0 {
        port.nuvoton_exit()?;
        return Ok(Detection::Rejected(format!(
            "{}: {}",
            chip.display_name(),
            tr("monitor sprzętu wyłączony przez BIOS", "hardware monitor disabled by BIOS")
        )));
    }
    let mut address = port.read_word(BASE_ADDRESS_REGISTER)?;
    std::thread::sleep(Duration::from_millis(1));
    let mut verify = port.read_word(BASE_ADDRESS_REGISTER)?;
    // Rodzina D8 moze podawac adres w parze 0x64/0x65.
    if chip == Chip::Nct6701D && address == verify && is_invalid_base(address) {
        address = port.read_word(ALTERNATE_BASE_ADDRESS_REGISTER)?;
        std::thread::sleep(Duration::from_millis(1));
        verify = port.read_word(ALTERNATE_BASE_ADDRESS_REGISTER)?;
    }
    if address == verify && chip.has_io_space_lock() {
        port.nuvoton_disable_io_space_lock()?;
    }
    port.nuvoton_exit()?;

    if address != verify {
        return Ok(Detection::Rejected(format!(
            "{}: {}",
            chip.display_name(),
            tr("weryfikacja adresu nieudana", "address verification failed")
        )));
    }
    // Niektore uklady maja juz dodane przesuniecie 0x05.
    if address & 0x07 == 0x05 {
        address &= 0xFFF8;
    }
    if is_invalid_base(address) {
        return Ok(Detection::Rejected(format!(
            "{}: {} 0x{address:X}",
            chip.display_name(),
            tr("niepoprawny adres", "invalid address")
        )));
    }
    Ok(Detection::Found(Draft { chip, base: address, it87_version: 0 }))
}

fn detect_ite(port: &LpcPort) -> Result<Detection, String> {
    // Na 0x4E uklad bywa juz w trybie konfiguracji - ponowne wejscie psuje IT8792 (ID 0x8883).
    let mut chip_id = 0xFFFF;
    if port.register_port == 0x4E {
        chip_id = port.read_word(CHIP_ID_REGISTER)?;
    }
    if port.register_port != 0x4E || chip_id == 0xFFFF {
        port.it87_enter()?;
        chip_id = port.read_word(CHIP_ID_REGISTER)?;
    }
    let Some(chip) = ite_chip(chip_id) else {
        if chip_id != 0 && chip_id != 0xFFFF {
            port.it87_exit()?;
            let report = format!(
                "{} (ID 0x{chip_id:04X}) {} 0x{:X}",
                tr("nieobsługiwany układ ITE", "unsupported ITE chip"),
                tr("na porcie", "on port"),
                port.register_port
            );
            return Ok(Detection::Rejected(report));
        }
        return Ok(Detection::Nothing);
    };

    port.find_bars()?;
    port.select(IT87_ENVIRONMENT_CONTROLLER_LDN)?;
    let address = port.read_word(BASE_ADDRESS_REGISTER)?;
    std::thread::sleep(Duration::from_millis(1));
    let verify = port.read_word(BASE_ADDRESS_REGISTER)?;
    let version = port.read_byte(IT87_CHIP_VERSION_REGISTER)? & 0x0F;
    // Adres GPIO nie jest nam potrzebny, ale LHM odrzuca uklad z blednym - zachowujemy te weryfikacje.
    let (gpio_ldn, gpio_register) = if chip == Chip::It8705F {
        (IT8705_GPIO_LDN, BASE_ADDRESS_REGISTER)
    } else {
        (IT87XX_GPIO_LDN, BASE_ADDRESS_REGISTER + 2)
    };
    port.select(gpio_ldn)?;
    let gpio_address = port.read_word(gpio_register)?;
    std::thread::sleep(Duration::from_millis(1));
    let gpio_verify = port.read_word(gpio_register)?;
    port.it87_exit()?;

    if address != verify || is_invalid_base(address) {
        return Ok(Detection::Rejected(format!(
            "{}: {} 0x{address:X}",
            chip.display_name(),
            tr("niepoprawny adres", "invalid address")
        )));
    }
    if gpio_address != gpio_verify || is_invalid_base(gpio_address) {
        return Ok(Detection::Rejected(format!(
            "{}: {} 0x{gpio_address:X}",
            chip.display_name(),
            tr("niepoprawny adres GPIO", "invalid GPIO address")
        )));
    }
    Ok(Detection::Found(Draft { chip, base: address, it87_version: version }))
}

/// Starsze rewizje IT8705F/IT8712F nie maja 16-bitowych licznikow.
pub(crate) fn it87_has_16bit(chip: Chip, version: u8) -> bool {
    (chip != Chip::It8705F || version >= 3) && (chip != Chip::It8712F || version >= 8)
}

fn channel(chip: Chip, index: usize, name: String) -> Channel {
    Channel { index, id: format!("superio:{}:fan{}", chip.slug(), index + 1), name }
}

/// Kontrole i inicjalizacja z konstruktorow `Nct677X` / `IT87XX` (z mutexem szyny ISA).
fn build_chip(port: LpcPort, draft: Draft) -> Result<SuperIoChip, String> {
    let Draft { chip, base, it87_version } = draft;
    let name = chip.display_name();
    let mut found = SuperIoChip {
        port,
        chip,
        base,
        backend: Backend::Nct668x { rpm_registers: Vec::new(), pwm_registers: Vec::new() },
        channels: Vec::new(),
        last: Vec::new(),
        error_logged: false,
    };

    if chip.is_ite() {
        let (vendor, valid) = found.it87_read(IT87_VENDOR_ID_REGISTER)?;
        if !valid || !ITE_VENDOR_IDS.contains(&vendor) {
            let issue = tr("nieoczekiwany identyfikator producenta", "unexpected vendor ID");
            return Err(format!("{name}: {issue} 0x{vendor:02X}"));
        }
        // Bit 0x10 rejestru konfiguracji zawsze ustawiony (poza IT8655E/IT8665E).
        let (configuration, valid) = found.it87_read(IT87_CONFIGURATION_REGISTER)?;
        if !valid || (configuration & 0x10 == 0 && chip != Chip::It8655E && chip != Chip::It8665E) {
            return Err(format!("{name}: {}", tr("monitor sprzętu nieaktywny", "hardware monitor inactive")));
        }
        let layout = it87_layout(chip);
        let has_16bit = it87_has_16bit(chip, it87_version);
        let mut fans_disabled = vec![false; layout.fan_count];
        // Kanaly 4-6 bez wlaczonego 16-bitowego licznika sa nieuzywane.
        if has_16bit {
            let (modes, valid) = found.it87_read(IT87_FAN_TACHOMETER_16BIT_REGISTER)?;
            if valid {
                if layout.fan_count >= 5 {
                    fans_disabled[3] = modes & (1 << 4) == 0;
                    fans_disabled[4] = modes & (1 << 5) == 0;
                }
                if layout.fan_count >= 6 {
                    if chip == Chip::It8665E {
                        let (modes, valid) = found.it87_read(IT87_FAN_TACHOMETER_16BIT_REGISTER_ALT)?;
                        if valid {
                            fans_disabled[5] = modes & (1 << 3) == 0;
                        }
                    } else {
                        fans_disabled[5] = modes & (1 << 2) == 0;
                    }
                }
            }
        }
        found.channels = (0..layout.fan_count)
            .filter(|&i| !fans_disabled[i])
            .map(|i| channel(chip, i, generic_fan_name(i)))
            .collect();
        found.backend = Backend::It87 { layout, has_16bit, fans_disabled };
    } else if chip.is_nct668x() {
        // Bit startu monitorowania (jak LHM i sterownik nct6683 w Linuksie) - BIOS zwykle juz go ustawia.
        let init = found.nct_ec_read(NCT668X_INIT_REGISTER)?;
        if init & 0x80 == 0 {
            found.nct_ec_write(NCT668X_INIT_REGISTER, init | 0x80)?;
        }
        let (rpm_registers, pwm_registers) = nct668x_registers(chip);
        found.backend = Backend::Nct668x { rpm_registers, pwm_registers };
        // Pierwszy odczyt wszystkich kanalow (lista kanalow jeszcze pusta), zeby znalezc dodatkowe zlacza.
        let first = found.read_all()?;
        let defaults = nct668x_default_channels(chip);
        let mut channels: Vec<Channel> = defaults.iter().map(|(i, n)| channel(chip, *i, n.clone())).collect();
        for (i, (rpm, _)) in first.iter().enumerate() {
            if !defaults.iter().any(|(d, _)| *d == i) && rpm.is_some_and(|r| r > 0.0) {
                channels.push(channel(chip, i, generic_fan_name(i)));
            }
        }
        channels.sort_by_key(|c| c.index);
        found.channels = channels;
    } else {
        let layout = nct677x_layout(chip)
            .ok_or_else(|| format!("{name}: {}", tr("brak opisu rejestrów", "no register layout")))?;
        if !found.nct_is_vendor(&layout)? {
            return Err(format!("{name}: {}", tr("nieoczekiwany identyfikator producenta", "unexpected vendor ID")));
        }
        found.channels = (0..layout.fan_count).map(|i| channel(chip, i, generic_fan_name(i))).collect();
        found.backend = Backend::Nct677x(layout);
    }
    found.last = vec![(None, None); found.channels.len()];
    Ok(found)
}

/// Jeden port: najpierw Winbond/Nuvoton, potem ITE (kolejnosc jak w `LpcIO.Detect`).
fn detect_port(port: &LpcPort) -> Result<Option<Draft>, String> {
    let mut rejected = None;
    match detect_nuvoton(port)? {
        Detection::Found(draft) => return Ok(Some(draft)),
        Detection::Rejected(report) => rejected = Some(report),
        Detection::Nothing => {}
    }
    match detect_ite(port)? {
        Detection::Found(draft) => Ok(Some(draft)),
        Detection::Rejected(report) => Err(report),
        Detection::Nothing => rejected.map_or(Ok(None), Err),
    }
}

// ---------------------------------------------------------------------------------------------

/// Wentylatory plyty glownej ze wszystkich wykrytych ukladow Super I/O.
pub struct SuperIoMonitor {
    chips: Vec<SuperIoChip>,
    // Po `chips` - mutex zamykany na koncu.
    mutex: IsaBusMutex,
}

impl SuperIoMonitor {
    /// Wykrywa uklady na portach 0x2E i 0x4E. Zwraca monitor (jesli cos znaleziono) i opisy problemow.
    pub fn new() -> (Option<Self>, Vec<String>) {
        let mutex = IsaBusMutex::open();
        let mut ports = Vec::new();
        for register_port in REGISTER_PORTS {
            match LpcPort::open(register_port) {
                Ok(port) => ports.push(port),
                // Brak PawnIO / admina dotyczy obu portow - jeden komunikat wystarczy.
                Err(err) => return (None, vec![err]),
            }
        }
        let Some(guard) = mutex.lock(DETECT_LOCK_TIMEOUT_MS) else {
            let issue = tr(
                "Szyna ISA zajęta przez inny program monitorujący - spróbuj ponownie",
                "ISA bus is busy with another monitoring program - try again",
            );
            return (None, vec![issue.to_owned()]);
        };
        let mut chips = Vec::new();
        let mut reports = Vec::new();
        for port in ports {
            let register_port = port.register_port;
            match detect_port(&port) {
                Ok(Some(draft)) => {
                    let name = draft.chip.display_name();
                    match build_chip(port, draft) {
                        Ok(chip) => {
                            log::info!(
                                "Super I/O: {name} na porcie 0x{register_port:X}, baza 0x{:X}, kanały: {}",
                                chip.base,
                                chip.channels.len()
                            );
                            chips.push(chip);
                        }
                        Err(err) => reports.push(err),
                    }
                }
                Ok(None) => {}
                Err(err) => reports.push(format!("port 0x{register_port:X}: {err}")),
            }
        }
        drop(guard);
        for report in &reports {
            log::warn!("Super I/O: {report}");
        }

        if chips.is_empty() {
            let mut issue = tr(
                "Nie wykryto obsługiwanego układu Super I/O (Nuvoton NCT67xx/NCT668x, ITE IT86xx/IT87xx)",
                "No supported Super I/O chip detected (Nuvoton NCT67xx/NCT668x, ITE IT86xx/IT87xx)",
            )
            .to_owned();
            if !reports.is_empty() {
                issue.push_str(": ");
                issue.push_str(&reports.join("; "));
            }
            return (None, vec![issue]);
        }
        // Przy dwoch ukladach (np. IT8686E + IT8792E) nazwy kanalow musza sie roznic.
        if chips.len() > 1 {
            for chip in &mut chips {
                let short = chip.chip.display_name().rsplit(' ').next().unwrap_or_default().to_owned();
                for ch in &mut chip.channels {
                    ch.name = format!("{} ({short})", ch.name);
                }
            }
        }
        (Some(Self { chips, mutex }), Vec::new())
    }

    pub fn read(&mut self) -> Vec<FanReading> {
        // Szyna zajeta przez inny program - zostaja poprzednie wartosci (jak w LHM).
        if let Some(_guard) = self.mutex.lock(READ_LOCK_TIMEOUT_MS) {
            for chip in &mut self.chips {
                match chip.read_all() {
                    Ok(values) => {
                        chip.error_logged = false;
                        for (slot, ch) in chip.last.iter_mut().zip(&chip.channels) {
                            *slot = values.get(ch.index).copied().unwrap_or((None, None));
                        }
                    }
                    Err(err) => {
                        if !chip.error_logged {
                            log::warn!("Super I/O {}: odczyt nieudany: {err}", chip.chip.display_name());
                            chip.error_logged = true;
                        }
                        chip.last.iter_mut().for_each(|slot| *slot = (None, None));
                    }
                }
            }
        }
        self.chips
            .iter()
            .flat_map(|chip| {
                chip.channels.iter().zip(&chip.last).map(|(ch, &(rpm, duty))| FanReading {
                    id: ch.id.clone(),
                    name: ch.name.clone(),
                    kind: FanKind::Board,
                    rpm,
                    duty_pct: duty,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CHIPS: [Chip; 44] = [
        Chip::Nct610x,
        Chip::Nct6771F,
        Chip::Nct6776F,
        Chip::Nct6779D,
        Chip::Nct6791D,
        Chip::Nct6792D,
        Chip::Nct6792DA,
        Chip::Nct6793D,
        Chip::Nct6795D,
        Chip::Nct6796D,
        Chip::Nct6796DR,
        Chip::Nct6797D,
        Chip::Nct6798D,
        Chip::Nct6799D,
        Chip::Nct6701D,
        Chip::Nct6683D,
        Chip::Nct6686D,
        Chip::Nct6687D,
        Chip::Nct6687DR,
        Chip::It8613E,
        Chip::It8620E,
        Chip::It8625E,
        Chip::It8628E,
        Chip::It8631E,
        Chip::It8638E,
        Chip::It8655E,
        Chip::It8665E,
        Chip::It8686E,
        Chip::It8688E,
        Chip::It8689E,
        Chip::It8696E,
        Chip::It8705F,
        Chip::It8712F,
        Chip::It8716F,
        Chip::It8718F,
        Chip::It8720F,
        Chip::It8721F,
        Chip::It8726F,
        Chip::It8728F,
        Chip::It8771E,
        Chip::It8772E,
        Chip::It8790E,
        Chip::It8792E,
        Chip::It87952E,
    ];

    #[test]
    fn nuvoton_ids_match_lhm_table() {
        assert_eq!(nuvoton_chip(0xD4, 0x2B), Some(Chip::Nct6798D));
        assert_eq!(nuvoton_chip(0xD4, 0x2A), Some(Chip::Nct6796DR));
        assert_eq!(nuvoton_chip(0xD4, 0x41), Some(Chip::Nct6686D));
        assert_eq!(nuvoton_chip(0xD8, 0x02), Some(Chip::Nct6799D));
        assert_eq!(nuvoton_chip(0xD5, 0x92), Some(Chip::Nct6687D));
        assert_eq!(nuvoton_chip(0xC7, 0x32), Some(Chip::Nct6683D));
        // Rewizja maskowana 0xF0 dla starszych ukladow.
        assert_eq!(nuvoton_chip(0xB4, 0x7A), Some(Chip::Nct6771F));
        assert_eq!(nuvoton_chip(0xC5, 0x62), Some(Chip::Nct6779D));
        // Winbond W83627DHG (nieobslugiwany) i brak ukladu.
        assert_eq!(nuvoton_chip(0xA0, 0x20), None);
        assert_eq!(nuvoton_chip(0xFF, 0xFF), None);
    }

    #[test]
    fn ite_ids_match_lhm_table() {
        assert_eq!(ite_chip(0x8686), Some(Chip::It8686E));
        assert_eq!(ite_chip(0x8733), Some(Chip::It8792E));
        assert_eq!(ite_chip(0x8695), Some(Chip::It87952E));
        assert_eq!(ite_chip(0xFFFF), None);
        assert_eq!(ite_chip(0x8883), None);
    }

    #[test]
    fn chip_tables_are_consistent() {
        let mut slugs: Vec<_> = ALL_CHIPS.iter().map(|c| c.slug()).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), ALL_CHIPS.len());
        assert!(ALL_CHIPS.iter().all(|c| c.is_ite() == c.display_name().starts_with("ITE")));
        // Kazdy Nuvoton poza NCT668x ma opis rejestrow.
        assert!(ALL_CHIPS.iter().filter(|c| !c.is_ite() && !c.is_nct668x()).all(|c| nct677x_layout(*c).is_some()));
        // Wyjsc PWM nigdy nie ma wiecej niz rejestrow sterowania (ITE).
        assert!(ALL_CHIPS.iter().filter(|c| c.is_ite()).all(|c| {
            let l = it87_layout(*c);
            l.control_count <= l.pwm_ctrl_registers.len() && l.fan_count <= 6
        }));
    }

    #[test]
    fn nct_layouts_follow_lhm() {
        let l = nct677x_layout(Chip::Nct6798D).unwrap();
        assert_eq!((l.fan_count, l.rpm_registers.is_none()), (7, true));
        assert_eq!(l.pwm_out_registers[5], 0xA09);
        let l = nct677x_layout(Chip::Nct6795D).unwrap();
        assert_eq!(l.fan_count, 6);
        let l = nct677x_layout(Chip::Nct6771F).unwrap();
        assert_eq!((l.fan_count, l.min_rpm), (4, 20));
        assert_eq!(l.rpm_registers.unwrap()[1], 0x658);
        let l = nct677x_layout(Chip::Nct6776F).unwrap();
        assert_eq!((l.fan_count, l.min_rpm), (5, 164));
        let (rpm, pwm) = nct668x_registers(Chip::Nct6687D);
        assert_eq!((rpm.len(), pwm.len(), rpm[16]), (17, 8, 0x852));
        let (rpm, pwm) = nct668x_registers(Chip::Nct6687DR);
        assert_eq!((rpm.len(), pwm.len(), rpm[10]), (16, 16, 0x15E));
        let names = nct668x_default_channels(Chip::Nct6687DR);
        assert_eq!(names.last().map(|(i, _)| *i), Some(15));
    }

    #[test]
    fn nct_13bit_count_conversion() {
        // count = (0x0A << 5) | 0x0E = 334 -> 1.35e6 / 334
        let rpm = nct_count_to_rpm(0x0A, 0x0E, false).unwrap();
        assert!((rpm - 4041.9).abs() < 0.1);
        // Maksymalny licznik = wentylator stoi.
        assert_eq!(nct_count_to_rpm(0xFF, 0x1F, false), Some(0.0));
        // Za maly licznik = odczyt niewiarygodny.
        assert_eq!(nct_count_to_rpm(0x00, 0x05, false), None);
        // Gorne bity mlodszego bajtu sa ignorowane.
        assert_eq!(nct_count_to_rpm(0x0A, 0xEE, false), nct_count_to_rpm(0x0A, 0x0E, false));
        // NCT6687D: 0xFFF8 = brak wentylatora.
        assert_eq!(nct_count_to_rpm(0xFF, 0xF8, true), Some(0.0));
    }

    #[test]
    fn nct_16bit_rpm_threshold() {
        assert_eq!(nct_rpm16(0x04, 0xB0, 164), 1200.0);
        assert_eq!(nct_rpm16(0x00, 0xA0, 164), 0.0);
        assert_eq!(nct_rpm16(0x00, 0x15, 20), 21.0);
    }

    #[test]
    fn it87_conversions() {
        // 16-bit: 1.35e6 / (750 * 2) = 900
        assert_eq!(it87_rpm16(750), Some(900.0));
        assert_eq!(it87_rpm16(0xFFFF), Some(0.0));
        assert_eq!(it87_rpm16(0x3F), None);
        // 8-bit z dzielnikiem 2: 1.35e6 / (150 * 2) = 4500
        assert_eq!(it87_rpm8(150, 2), Some(4500.0));
        assert_eq!(it87_rpm8(0xFF, 2), Some(0.0));
        assert_eq!(it87_rpm8(0, 2), None);
        // Wypelnienie
        assert_eq!(it87_duty(0x80, None), None);
        assert_eq!(it87_duty(0x00, Some(0xFF)), Some(100.0));
        assert_eq!(it87_duty(0x3F, None), Some(50.0));
        assert!(it87_has_16bit(Chip::It8686E, 0));
        assert!(!it87_has_16bit(Chip::It8712F, 7));
        assert!(it87_has_16bit(Chip::It8712F, 8));
    }

    #[test]
    fn it87_layouts_follow_lhm() {
        let l = it87_layout(Chip::It8686E);
        assert_eq!((l.fan_count, l.control_count, l.has_ext_reg), (6, 5, true));
        let l = it87_layout(Chip::It8665E);
        assert!(l.alt_6th_fan_reg && l.requires_bank_select);
        assert_eq!(l.pwm_ctrl_registers[5], 0x92);
        let l = it87_layout(Chip::It8792E);
        assert_eq!((l.fan_count, l.control_count), (3, 3));
        let l = it87_layout(Chip::It8720F);
        assert_eq!((l.fan_count, l.control_count, l.has_ext_reg), (5, 3, false));
    }

    #[test]
    fn invalid_base_addresses() {
        assert!(!is_invalid_base(0x290));
        assert!(!is_invalid_base(0xA40));
        assert!(is_invalid_base(0x0F0));
        assert!(is_invalid_base(0x295));
        assert!(is_invalid_base(0x1290));
    }
}
