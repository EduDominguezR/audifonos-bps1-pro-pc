//! BLE transport for the Bass BP1 Pro ANC.
//!
//! One worker thread owns the radio. It scans, connects, subscribes, and then
//! sits in a `tokio::select!` on three sources: notifications, app requests, and
//! a disconnect signal. Nothing polls, so an idle app burns no measurable CPU.

use baseus_protocol::{Command, DeviceEvent};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;

#[cfg(feature = "mock")]
pub mod mock;

pub mod debug;

#[cfg(windows)]
mod win;

#[cfg(feature = "mock")]
pub use mock::spawn_mock;

/// Turns the raw-traffic log on or off. Off by default: nothing is written.
pub use debug::set_enabled as set_debug;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("no se encontró ningún adaptador Bluetooth")]
    NoAdapter,
    #[error("Bluetooth está apagado o sin permisos")]
    RadioUnavailable,
    #[error("no se encontraron los audífonos")]
    NotFound,
    #[error("el servicio o la característica GATT no existe en el dispositivo")]
    ServiceNotFound,
    #[error("el dispositivo rechazó la escritura ({0})")]
    Rejected(String),
    #[error("se perdió la conexión")]
    Disconnected,
    #[error("{0}")]
    WinRt(String),
}

/// What the worker tells the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A scan started. The UI shows "buscando" without blocking.
    Scanning,
    Connected,
    NotFound,
    Disconnected,
    /// A failed attempt with a message already written for people.
    Failed(String),
    Device(DeviceEvent),
}

/// What the UI asks the worker to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Abandon the current wait and scan again now.
    Rescan,
    Send(Command),
    /// Play the find tone on both buds, then stop it from the worker itself.
    FindFor(std::time::Duration),
    /// Silence the find tone now, if it is playing.
    StopFind,
}

/// Handle to the radio thread. Dropping it closes the request channel and the
/// thread ends after the current wait.
pub struct Worker {
    requests: UnboundedSender<Request>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn request(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    #[cfg(windows)]
    pub fn spawn_real(events: UnboundedSender<Event>) -> Result<Self, TransportError> {
        let (requests, rx) = tokio::sync::mpsc::unbounded_channel();
        let thread = std::thread::Builder::new()
            .name("baseus-ble".to_owned())
            .spawn(move || win::run(events, rx))
            .map_err(|e| TransportError::WinRt(e.to_string()))?;
        Ok(Self {
            requests,
            thread: Some(thread),
        })
    }

    #[cfg(not(windows))]
    pub fn spawn_real(_events: UnboundedSender<Event>) -> Result<Self, TransportError> {
        Err(TransportError::RadioUnavailable)
    }

    /// A worker that reports one failure and then idles. Used when the radio
    /// thread cannot even be created.
    pub fn spawn_failed(events: UnboundedSender<Event>, message: String) -> Self {
        let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let thread = std::thread::Builder::new()
            .name("baseus-idle".to_owned())
            .spawn(move || {
                let _ = events.send(Event::Failed(message));
                while receiver.blocking_recv().is_some() {}
            })
            .ok();
        Self { requests, thread }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the channel is enough: the thread parks on recv and exits.
        // Joining here could block the UI thread for the length of a scan.
        self.thread.take();
    }
}

/// Shared shape of a session outcome, used by both the real and the mock worker.
pub(crate) enum Outcome {
    /// Nothing found: retry quietly from time to time.
    NotFound,
    /// The buds went away: retry with backoff.
    Disconnected,
    /// Something recoverable failed: wait for the user.
    Failed(String),
    /// The app closed its request channel: stop the radio thread.
    Closed,
}

/// Decode one notification and hand it to the UI.
///
/// `last_anc` is needed because `AA 34` acks differ by firmware; see
/// `Bp1ProAnc::resolve_anc_ack`.
pub(crate) fn handle_notification(
    bytes: &[u8],
    events: &UnboundedSender<Event>,
    last_anc: &mut Option<baseus_protocol::AncMode>,
) {
    crate::debug::traffic("RX", bytes);
    let Ok(frame) = baseus_protocol::Frame::decode_notification(bytes) else {
        return;
    };
    let decoded = if frame.cmd == 0x34 {
        Some(DeviceEvent::Anc(
            baseus_protocol::Bp1ProAnc::resolve_anc_ack(&frame.payload, *last_anc),
        ))
    } else {
        baseus_protocol::Bp1ProAnc::decode(&frame).ok()
    };
    if let Some(ref event) = decoded {
        log_invalid_levels(event);
    }
    if let Some(event) = decoded {
        let _ = events.send(Event::Device(event));
    }
}

/// The protocol preserves bytes it does not understand; make them visible in
/// the debug log so a report can show exactly what the firmware sent.
fn log_invalid_levels(event: &DeviceEvent) {
    use baseus_protocol::BatteryLevel;
    match event {
        DeviceEvent::Battery(state) => {
            for (name, level) in [("izquierdo", state.left), ("derecho", state.right)] {
                if let BatteryLevel::Unknown(raw) = level {
                    crate::debug::note(&format!("batería {name}: byte 0x{raw:02X} fuera de 0-100"));
                }
            }
        }
        DeviceEvent::Case(state) => {
            if let BatteryLevel::Unknown(raw) = state.case {
                crate::debug::note(&format!("estuche: byte 0x{raw:02X} fuera de 0-100"));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baseus_protocol::AncMode;

    fn notify(bytes: &[u8], last: Option<AncMode>) -> Option<Event> {
        let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut last = last;
        handle_notification(bytes, &events, &mut last);
        receiver.try_recv().ok()
    }

    #[test]
    fn ack_34_keeps_the_last_commanded_mode() {
        assert_eq!(
            notify(&[0xAA, 0x34, 0x01], Some(AncMode::Transparency)),
            Some(Event::Device(DeviceEvent::Anc(AncMode::Transparency)))
        );
    }

    #[test]
    fn ack_34_with_an_empty_payload_means_off() {
        assert_eq!(
            notify(&[0xAA, 0x34, 0x00], Some(AncMode::Anc)),
            Some(Event::Device(DeviceEvent::Anc(AncMode::Off)))
        );
    }

    #[test]
    fn frames_with_a_bad_magic_are_dropped() {
        assert_eq!(notify(&[0x00, 0x34, 0x01], Some(AncMode::Anc)), None);
    }
}

#[cfg(all(test, feature = "mock"))]
mod mock_tests {
    use std::time::{Duration, Instant};

    use baseus_protocol::{AncMode, Command, DeviceEvent};

    use super::*;

    fn next(receiver: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Event {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(event) = receiver.try_recv() {
                return event;
            }
            assert!(Instant::now() < deadline, "el simulador no respondió");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn mock_connects_reports_battery_and_confirms_anc() {
        let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let worker = spawn_mock(events);

        assert_eq!(next(&mut receiver), Event::Scanning);
        assert_eq!(next(&mut receiver), Event::Connected);

        let mut battery = None;
        for _ in 0..5 {
            if let Event::Device(DeviceEvent::Battery(state)) = next(&mut receiver) {
                battery = Some(state);
            }
        }
        assert_eq!(battery.map(|state| state.left.percent()), Some(Some(84)));

        worker.request(Request::Send(Command::SetAnc {
            mode: AncMode::Anc,
            level: 0x80,
        }));
        let confirmed =
            (0..40).any(|_| next(&mut receiver) == Event::Device(DeviceEvent::Anc(AncMode::Anc)));
        assert!(confirmed, "el simulador no confirmó el ANC");
    }
}
