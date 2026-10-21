use crate::types::{AncMode, EqPreset, Side, ANC_MAX_LEVEL, ANC_MIN_LEVEL};
use crate::WRITE_MAGIC;

/// Every write the app sends to a BP1 Pro ANC.
///
/// The bytes match the captures in the upstream repository, including the
/// fixed `0xFF` strength for Off and Transparency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Sent once after subscribing; the device answers with its identity frame.
    Handshake,
    SetAnc {
        mode: AncMode,
        level: u8,
    },
    SetEq(EqPreset),
    QueryEq,
    SetGameMode(bool),
    FindEarbuds {
        side: Side,
        on: bool,
    },
}

impl Command {
    pub fn encode(self) -> Vec<u8> {
        let (cmd, payload) = match self {
            Self::Handshake => (0x05, vec![0x00]),
            Self::SetAnc { mode, level } => {
                let level = if mode.uses_level() {
                    level.clamp(ANC_MIN_LEVEL, ANC_MAX_LEVEL)
                } else {
                    0xFF
                };
                (0x34, vec![mode.mode_byte(), level])
            }
            Self::SetEq(preset) => (0x43, vec![preset.to_byte()]),
            Self::QueryEq => (0x42, vec![]),
            Self::SetGameMode(on) => (0x24, vec![u8::from(on)]),
            Self::FindEarbuds { side, on } => (0x10, vec![side.index(), u8::from(on)]),
        };
        let mut out = Vec::with_capacity(3 + payload.len());
        out.push(WRITE_MAGIC);
        out.push(cmd);
        out.extend_from_slice(&payload);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anc_off_matches_capture() {
        assert_eq!(
            Command::SetAnc {
                mode: AncMode::Off,
                level: 0x68
            }
            .encode(),
            [0xBA, 0x34, 0x00, 0xFF]
        );
    }

    #[test]
    fn anc_on_keeps_requested_level() {
        assert_eq!(
            Command::SetAnc {
                mode: AncMode::Anc,
                level: 0x68
            }
            .encode(),
            [0xBA, 0x34, 0x01, 0x68]
        );
    }

    #[test]
    fn anc_level_is_clamped_into_the_documented_range() {
        assert_eq!(
            Command::SetAnc {
                mode: AncMode::Anc,
                level: 0
            }
            .encode(),
            [0xBA, 0x34, 0x01, ANC_MIN_LEVEL]
        );
        // 0xFF belongs to Off and Transparency, so a real level never reaches it.
        assert_eq!(
            Command::SetAnc {
                mode: AncMode::Anc,
                level: 0xFF
            }
            .encode()[3],
            ANC_MAX_LEVEL
        );
        assert_ne!(ANC_MAX_LEVEL, 0xFF);
    }

    #[test]
    fn transparency_ignores_level() {
        assert_eq!(
            Command::SetAnc {
                mode: AncMode::Transparency,
                level: 0x10
            }
            .encode(),
            [0xBA, 0x34, 0x02, 0xFF]
        );
    }

    #[test]
    fn eq_commands_match_captures() {
        assert_eq!(
            Command::SetEq(EqPreset::Balanced).encode(),
            [0xBA, 0x43, 0x00]
        );
        assert_eq!(
            Command::SetEq(EqPreset::BassBoost).encode(),
            [0xBA, 0x43, 0x01]
        );
        assert_eq!(Command::SetEq(EqPreset::Voice).encode(), [0xBA, 0x43, 0x02]);
        assert_eq!(Command::SetEq(EqPreset::Clear).encode(), [0xBA, 0x43, 0x03]);
        assert_eq!(Command::QueryEq.encode(), [0xBA, 0x42]);
    }

    #[test]
    fn game_mode_matches_capture() {
        assert_eq!(Command::SetGameMode(true).encode(), [0xBA, 0x24, 0x01]);
        assert_eq!(Command::SetGameMode(false).encode(), [0xBA, 0x24, 0x00]);
    }

    #[test]
    fn find_my_buds_matches_capture() {
        assert_eq!(
            Command::FindEarbuds {
                side: Side::Left,
                on: true
            }
            .encode(),
            [0xBA, 0x10, 0x00, 0x01]
        );
        assert_eq!(
            Command::FindEarbuds {
                side: Side::Right,
                on: false
            }
            .encode(),
            [0xBA, 0x10, 0x01, 0x00]
        );
    }

    #[test]
    fn handshake_matches_capture() {
        assert_eq!(Command::Handshake.encode(), [0xBA, 0x05, 0x00]);
    }
}
