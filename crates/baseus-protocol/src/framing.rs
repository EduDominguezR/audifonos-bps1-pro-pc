use thiserror::Error;

/// First byte of every device-to-app notification.
pub const NOTIFY_MAGIC: u8 = 0xAA;
/// First byte of every app-to-device write on this model.
pub const WRITE_MAGIC: u8 = 0xBA;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame too short: need 2 bytes, got {0}")]
    TooShort(usize),
    #[error("unexpected magic byte {got:#04x}, expected {expected:#04x}")]
    BadMagic { expected: u8, got: u8 },
}

/// A raw GATT frame: magic byte, command byte, then a command-specific payload.
///
/// There is no length field and no checksum: the PDU size is the frame size.
/// Confirmed against the nRF Connect captures in the upstream repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub cmd: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn decode(buf: &[u8], magic: u8) -> Result<Self, FrameError> {
        if buf.len() < 2 {
            return Err(FrameError::TooShort(buf.len()));
        }
        if buf[0] != magic {
            return Err(FrameError::BadMagic {
                expected: magic,
                got: buf[0],
            });
        }
        Ok(Self {
            cmd: buf[1],
            payload: buf[2..].to_vec(),
        })
    }

    pub fn decode_notification(buf: &[u8]) -> Result<Self, FrameError> {
        Self::decode(buf, NOTIFY_MAGIC)
    }

    pub fn encode(&self, magic: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + self.payload.len());
        out.push(magic);
        out.push(self.cmd);
        out.extend_from_slice(&self.payload);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_anc_transparency() {
        let f = Frame::decode(&[0xAA, 0x32, 0x02, 0xFF], NOTIFY_MAGIC).unwrap();
        assert_eq!(f.cmd, 0x32);
        assert_eq!(f.payload, [0x02, 0xFF]);
    }

    #[test]
    fn decode_battery() {
        let f = Frame::decode(&[0xAA, 0x02, 0x64, 0x00, 0x5A, 0x01], NOTIFY_MAGIC).unwrap();
        assert_eq!(f.cmd, 0x02);
        assert_eq!(f.payload, [0x64, 0x00, 0x5A, 0x01]);
    }

    #[test]
    fn round_trips_through_encode() {
        let original = Frame {
            cmd: 0x02,
            payload: vec![0x64, 0x00, 0x5A, 0x01],
        };
        let decoded = Frame::decode(&original.encode(NOTIFY_MAGIC), NOTIFY_MAGIC).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(
            Frame::decode(&[0x00, 0x30], NOTIFY_MAGIC),
            Err(FrameError::BadMagic {
                expected: NOTIFY_MAGIC,
                got: 0x00
            })
        );
    }

    #[test]
    fn rejects_short_buffer() {
        assert_eq!(
            Frame::decode(&[0xAA], NOTIFY_MAGIC),
            Err(FrameError::TooShort(1))
        );
        assert_eq!(
            Frame::decode(&[], NOTIFY_MAGIC),
            Err(FrameError::TooShort(0))
        );
    }
}
