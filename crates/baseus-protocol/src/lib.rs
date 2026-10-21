//! Wire format for the Baseus Bass BP1 Pro ANC earbuds.
//!
//! This crate does no I/O. It turns bytes into [`DeviceEvent`]s and
//! [`Command`]s into bytes. Every constant traces back to a capture in the
//! upstream `baseus-desktop` repository (`docs/protocol/`), see the README for
//! credits and the MIT attribution.

#![forbid(unsafe_code)]

mod commands;
mod framing;
mod model;
mod types;

pub use commands::Command;
pub use framing::{Frame, FrameError, NOTIFY_MAGIC, WRITE_MAGIC};
pub use model::{Bp1ProAnc, DecodeError};
pub use types::{
    AncMode, BatteryLevel, BatteryState, CaseState, DeviceEvent, EqPreset, Side, ANC_DEFAULT_LEVEL,
    ANC_MAX_LEVEL, ANC_MIN_LEVEL,
};
