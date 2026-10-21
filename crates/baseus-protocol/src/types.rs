/// Which earbud a command targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// Bud ID byte used by the `BA 10` find-my-buds command.
    pub fn index(self) -> u8 {
        match self {
            Self::Left => 0x00,
            Self::Right => 0x01,
        }
    }
}

/// Noise control mode. The wire values are the payload of `BA 34 mode level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AncMode {
    #[default]
    Off,
    Anc,
    Transparency,
}

impl AncMode {
    pub fn mode_byte(self) -> u8 {
        match self {
            Self::Off => 0x00,
            Self::Anc => 0x01,
            Self::Transparency => 0x02,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Apagado",
            Self::Anc => "Cancelación",
            Self::Transparency => "Transparencia",
        }
    }

    /// Only active cancellation honours the strength byte.
    pub fn uses_level(self) -> bool {
        self == Self::Anc
    }
}

/// Lowest strength byte the firmware accepts for active cancellation.
pub const ANC_MIN_LEVEL: u8 = 0x10;
/// Strength byte the official app sends when no strength is chosen.
pub const ANC_DEFAULT_LEVEL: u8 = 0x68;
/// Highest usable strength byte. `0xFF` is reserved for Off and Transparency,
/// where the firmware expects "no strength" rather than a real level.
pub const ANC_MAX_LEVEL: u8 = 0xFE;

/// Equalizer presets, in the same order the official app lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EqPreset {
    #[default]
    Balanced,
    BassBoost,
    Voice,
    Clear,
}

impl EqPreset {
    pub const ALL: [Self; 4] = [Self::Balanced, Self::BassBoost, Self::Voice, Self::Clear];

    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Balanced),
            1 => Some(Self::BassBoost),
            2 => Some(Self::Voice),
            3 => Some(Self::Clear),
            _ => None,
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            Self::Balanced => 0,
            Self::BassBoost => 1,
            Self::Voice => 2,
            Self::Clear => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Balanced => "Equilibrado",
            Self::BassBoost => "Graves",
            Self::Voice => "Voz",
            Self::Clear => "Claro",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Balanced => "Sonido parejo para todo",
            Self::BassBoost => "Más graves, más cuerpo",
            Self::Voice => "Voces y podcasts al frente",
            Self::Clear => "Agudos abiertos, más detalle",
        }
    }
}

/// A battery value exactly as the firmware reported it.
///
/// Not every byte is a percentage: a bud seated in the case answers 0, and some
/// firmware revisions answer with bytes above 100. Clamping those to 100 hides
/// the problem, so the raw byte is preserved instead of guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryLevel {
    /// A real percentage between 1 and 100.
    Level(u8),
    /// A bud in the case reports 0: that is a state, not an empty battery.
    InCase,
    /// A byte no firmware revision documents. The raw value is kept so the
    /// debug log can show it.
    Unknown(u8),
}

impl BatteryLevel {
    /// Byte 0 from a bud means "seated in the case".
    pub fn from_bud(byte: u8) -> Self {
        match byte {
            0 => Self::InCase,
            1..=100 => Self::Level(byte),
            other => Self::Unknown(other),
        }
    }

    /// A case at 0 % is an empty case, not a bud in the case.
    pub fn from_case(byte: u8) -> Self {
        match byte {
            0..=100 => Self::Level(byte),
            other => Self::Unknown(other),
        }
    }

    pub fn percent(self) -> Option<u8> {
        match self {
            Self::Level(value) => Some(value),
            Self::InCase | Self::Unknown(_) => None,
        }
    }

    /// Raw byte, including the invalid one, for the debug log.
    pub fn raw(self) -> u8 {
        match self {
            Self::Level(value) => value,
            Self::InCase => 0,
            Self::Unknown(value) => value,
        }
    }
}

/// Bud battery report (`AA 02`). A bud seated in the case reports 0 here; the
/// frame carries no charging flag, which is why [`BatteryState`] has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryState {
    pub left: BatteryLevel,
    pub right: BatteryLevel,
}

/// Case battery report (`AA 27`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaseState {
    pub case: BatteryLevel,
    pub case_charging: bool,
}

/// Everything the device can tell the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    Battery(BatteryState),
    Case(CaseState),
    Anc(AncMode),
    GameMode(bool),
    Eq(EqPreset),
    /// MAC address from the `AA 12` identity frame, in normal order.
    Identity([u8; 6]),
}
