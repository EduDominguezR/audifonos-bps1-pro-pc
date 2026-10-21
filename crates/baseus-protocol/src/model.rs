use thiserror::Error;

use crate::types::{AncMode, BatteryLevel, BatteryState, CaseState, DeviceEvent, EqPreset};
use crate::Frame;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("payload too short for opcode {opcode:#04x}: need {need}, got {got}")]
    PayloadTooShort { opcode: u8, need: usize, got: usize },
    #[error("unknown opcode {0:#04x}")]
    UnknownOpcode(u8),
    #[error("unknown equalizer preset {0:#04x}")]
    UnknownEqPreset(u8),
}

impl DecodeError {
    /// Opcodes we do not decode yet are noise, not failures. The caller logs and
    /// keeps listening instead of dropping the connection.
    pub fn is_ignorable(&self) -> bool {
        matches!(self, Self::UnknownOpcode(_) | Self::UnknownEqPreset(_))
    }
}

/// Decoder for the only hardware-verified model in the upstream project.
pub struct Bp1ProAnc;

impl Bp1ProAnc {
    /// `AA 30` is a periodic keepalive, `AA 24` is a flat "received" ack: neither
    /// carries state, so both fall through to `UnknownOpcode`.
    pub fn decode(frame: &Frame) -> Result<DeviceEvent, DecodeError> {
        match frame.cmd {
            0x02 => Self::decode_battery(&frame.payload),
            0x12 => Self::decode_identity(&frame.payload),
            0x23 => Ok(DeviceEvent::GameMode(
                frame.payload.first().copied().unwrap_or(0) != 0,
            )),
            0x27 => Self::decode_case(&frame.payload),
            0x32 => Ok(DeviceEvent::Anc(AncMode::Transparency)),
            0x33 => Ok(DeviceEvent::Anc(AncMode::Anc)),
            0x42 | 0x43 => {
                let byte = frame.payload.first().copied().unwrap_or(0);
                EqPreset::from_byte(byte)
                    .map(DeviceEvent::Eq)
                    .ok_or(DecodeError::UnknownEqPreset(byte))
            }
            other => Err(DecodeError::UnknownOpcode(other)),
        }
    }

    /// Resolve an `AA 34` cancellation ack.
    ///
    /// Firmware variants disagree here. Some echo the mode in the payload, and
    /// others answer every command, Off included, with a flat `AA 34 01`. A zero
    /// payload is therefore always Off, while a non-zero payload only confirms
    /// whatever was commanded last.
    pub fn resolve_anc_ack(payload: &[u8], last_commanded: Option<AncMode>) -> AncMode {
        if payload.first().copied().unwrap_or(0) == 0 {
            AncMode::Off
        } else {
            last_commanded.unwrap_or(AncMode::Anc)
        }
    }

    fn decode_battery(payload: &[u8]) -> Result<DeviceEvent, DecodeError> {
        // AA 02 left 00 right 01. Bytes 1 and 3 are bud IDs, not charge flags.
        if payload.len() < 4 {
            return Err(DecodeError::PayloadTooShort {
                opcode: 0x02,
                need: 4,
                got: payload.len(),
            });
        }
        Ok(DeviceEvent::Battery(BatteryState {
            left: BatteryLevel::from_bud(payload[0]),
            right: BatteryLevel::from_bud(payload[2]),
        }))
    }

    fn decode_case(payload: &[u8]) -> Result<DeviceEvent, DecodeError> {
        if payload.len() < 2 {
            return Err(DecodeError::PayloadTooShort {
                opcode: 0x27,
                need: 2,
                got: payload.len(),
            });
        }
        Ok(DeviceEvent::Case(CaseState {
            case: BatteryLevel::from_case(payload[0]),
            case_charging: payload[1] != 0,
        }))
    }

    /// `AA 12` repeats the MAC least significant byte first.
    fn decode_identity(payload: &[u8]) -> Result<DeviceEvent, DecodeError> {
        if payload.len() < 6 {
            return Err(DecodeError::PayloadTooShort {
                opcode: 0x12,
                need: 6,
                got: payload.len(),
            });
        }
        let mut mac = [0u8; 6];
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = payload[5 - i];
        }
        Ok(DeviceEvent::Identity(mac))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Frame;

    fn decode(raw: &[u8]) -> Result<DeviceEvent, DecodeError> {
        Bp1ProAnc::decode(&Frame::decode_notification(raw).unwrap())
    }

    #[test]
    fn battery_frame_decodes_both_buds() {
        assert_eq!(
            decode(&[0xAA, 0x02, 0x64, 0x00, 0x64, 0x01]).unwrap(),
            DeviceEvent::Battery(BatteryState {
                left: BatteryLevel::Level(100),
                right: BatteryLevel::Level(100)
            })
        );
    }

    #[test]
    fn battery_frame_with_bud_in_case_reports_the_case_state() {
        assert_eq!(
            decode(&[0xAA, 0x02, 0x00, 0x00, 0x64, 0x01]).unwrap(),
            DeviceEvent::Battery(BatteryState {
                left: BatteryLevel::InCase,
                right: BatteryLevel::Level(100)
            })
        );
    }

    #[test]
    fn battery_keeps_the_left_bud_in_the_left_field() {
        // Frame from the upstream notes: left 100 %, right 90 %.
        assert_eq!(
            decode(&[0xAA, 0x02, 0x64, 0x00, 0x5A, 0x01]).unwrap(),
            DeviceEvent::Battery(BatteryState {
                left: BatteryLevel::Level(100),
                right: BatteryLevel::Level(90)
            })
        );
    }

    #[test]
    fn out_of_range_battery_byte_is_unknown_not_100() {
        assert_eq!(
            decode(&[0xAA, 0x02, 0xE4, 0x00, 0x5A, 0x01]).unwrap(),
            DeviceEvent::Battery(BatteryState {
                left: BatteryLevel::Unknown(0xE4),
                right: BatteryLevel::Level(90)
            })
        );
        assert_eq!(BatteryLevel::Unknown(0xE4).percent(), None);
        assert_eq!(BatteryLevel::Unknown(0xE4).raw(), 0xE4);
    }

    #[test]
    fn case_frame_decodes_percentage_and_charging() {
        assert_eq!(
            decode(&[0xAA, 0x27, 0x32, 0x00]).unwrap(),
            DeviceEvent::Case(CaseState {
                case: BatteryLevel::Level(50),
                case_charging: false
            })
        );
        assert_eq!(
            decode(&[0xAA, 0x27, 0x32, 0x01]).unwrap(),
            DeviceEvent::Case(CaseState {
                case: BatteryLevel::Level(50),
                case_charging: true
            })
        );
        // An empty case is a real 0 %, unlike a bud resting in it.
        assert_eq!(
            decode(&[0xAA, 0x27, 0x00, 0x00]).unwrap(),
            DeviceEvent::Case(CaseState {
                case: BatteryLevel::Level(0),
                case_charging: false
            })
        );
        assert_eq!(
            decode(&[0xAA, 0x27, 0xE4, 0x00]).unwrap(),
            DeviceEvent::Case(CaseState {
                case: BatteryLevel::Unknown(0xE4),
                case_charging: false
            })
        );
    }

    #[test]
    fn game_mode_state_decodes_both_ways() {
        assert_eq!(
            decode(&[0xAA, 0x23, 0x01]).unwrap(),
            DeviceEvent::GameMode(true)
        );
        assert_eq!(
            decode(&[0xAA, 0x23, 0x00]).unwrap(),
            DeviceEvent::GameMode(false)
        );
    }

    #[test]
    fn eq_ack_and_query_response_both_decode() {
        assert_eq!(
            decode(&[0xAA, 0x43, 0x01]).unwrap(),
            DeviceEvent::Eq(EqPreset::BassBoost)
        );
        assert_eq!(
            decode(&[0xAA, 0x42, 0x02]).unwrap(),
            DeviceEvent::Eq(EqPreset::Voice)
        );
    }

    #[test]
    fn unknown_eq_byte_is_ignorable() {
        let error = decode(&[0xAA, 0x43, 0xFF]).unwrap_err();
        assert_eq!(error, DecodeError::UnknownEqPreset(0xFF));
        assert!(error.is_ignorable());
    }

    #[test]
    fn keepalive_and_flat_ack_are_ignorable() {
        for frame in [[0xAA, 0x30, 0x00], [0xAA, 0x24, 0x01], [0xAA, 0x80, 0x01]] {
            let err = decode(&frame).unwrap_err();
            assert!(err.is_ignorable(), "{frame:02X?} should be ignorable");
        }
    }

    #[test]
    fn identity_frame_reverses_the_mac() {
        // AA 12 03 C8 BA CE 01 4A ...  ->  4A:01:CE:BA:C8:03
        let ev = decode(&[
            0xAA, 0x12, 0x03, 0xC8, 0xBA, 0xCE, 0x01, 0x4A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01,
        ])
        .unwrap();
        assert_eq!(
            ev,
            DeviceEvent::Identity([0x4A, 0x01, 0xCE, 0xBA, 0xC8, 0x03])
        );
    }

    #[test]
    fn short_payloads_are_errors_not_panics() {
        assert_eq!(
            decode(&[0xAA, 0x02, 0x64, 0x00, 0x5A]),
            Err(DecodeError::PayloadTooShort {
                opcode: 0x02,
                need: 4,
                got: 3
            })
        );
        assert!(decode(&[0xAA, 0x27, 0x32]).is_err());
        assert!(decode(&[0xAA, 0x12, 0x01]).is_err());
    }

    #[test]
    fn anc_ack_semantics_cover_both_firmware_families() {
        assert_eq!(
            Bp1ProAnc::resolve_anc_ack(&[0x00], Some(AncMode::Anc)),
            AncMode::Off
        );
        assert_eq!(
            Bp1ProAnc::resolve_anc_ack(&[0x01], Some(AncMode::Off)),
            AncMode::Off
        );
        assert_eq!(
            Bp1ProAnc::resolve_anc_ack(&[0x01], Some(AncMode::Transparency)),
            AncMode::Transparency
        );
        assert_eq!(Bp1ProAnc::resolve_anc_ack(&[0x01], None), AncMode::Anc);
    }
}
