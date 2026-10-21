//! Simulated earbuds: same request channel, same events, no radio.
//!
//! Enabled with `--mock` so the UI can be exercised and measured on a machine
//! without the hardware. It answers commands the way the real device does,
//! including the flat `0x34` ack quirk.

use std::time::Duration;

use baseus_protocol::{
    AncMode, BatteryLevel, BatteryState, CaseState, Command, DeviceEvent, EqPreset,
};

use crate::{Event, Request, Worker};

pub fn spawn_mock(events: tokio::sync::mpsc::UnboundedSender<Event>) -> Worker {
    let (requests, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let thread = std::thread::Builder::new()
        .name("baseus-mock".to_owned())
        .spawn(move || {
            let mut state = MockState::default();
            let _ = events.send(Event::Scanning);
            std::thread::sleep(Duration::from_millis(600));
            let _ = events.send(Event::Connected);
            for event in state.snapshot() {
                let _ = events.send(Event::Device(event));
            }

            loop {
                let Some(request) = rx.blocking_recv() else {
                    return;
                };
                match request {
                    Request::Rescan => {
                        let _ = events.send(Event::Scanning);
                        std::thread::sleep(Duration::from_millis(400));
                        let _ = events.send(Event::Connected);
                        for event in state.snapshot() {
                            let _ = events.send(Event::Device(event));
                        }
                    }
                    Request::Send(command) => {
                        // Real writes take a few tens of milliseconds; the delay
                        // keeps the loading affordances honest in mock runs.
                        std::thread::sleep(Duration::from_millis(40));
                        for event in state.apply(command) {
                            let _ = events.send(Event::Device(event));
                        }
                    }
                    // The simulated buds have no buzzer; the requests are accepted
                    // so the UI path is identical.
                    Request::FindFor(_) | Request::StopFind => {}
                }
            }
        })
        .expect("mock thread");

    Worker {
        requests,
        thread: Some(thread),
    }
}

struct MockState {
    anc: AncMode,
    level: u8,
    game: bool,
    eq: EqPreset,
    battery: BatteryState,
    case: CaseState,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            anc: AncMode::Off,
            level: 0x68,
            game: false,
            eq: EqPreset::Balanced,
            battery: BatteryState {
                left: BatteryLevel::Level(84),
                right: BatteryLevel::Level(79),
            },
            case: CaseState {
                case: BatteryLevel::Level(62),
                case_charging: false,
            },
        }
    }
}

impl MockState {
    fn snapshot(&self) -> Vec<DeviceEvent> {
        vec![
            DeviceEvent::Battery(self.battery),
            DeviceEvent::Case(self.case),
            DeviceEvent::Anc(self.anc),
            DeviceEvent::GameMode(self.game),
            DeviceEvent::Eq(self.eq),
        ]
    }

    fn apply(&mut self, command: Command) -> Vec<DeviceEvent> {
        match command {
            Command::Handshake | Command::QueryEq => vec![DeviceEvent::Eq(self.eq)],
            Command::SetAnc { mode, level } => {
                self.anc = mode;
                if mode.uses_level() {
                    self.level = level;
                }
                vec![DeviceEvent::Anc(mode)]
            }
            Command::SetEq(preset) => {
                self.eq = preset;
                vec![DeviceEvent::Eq(preset)]
            }
            Command::SetGameMode(on) => {
                self.game = on;
                vec![DeviceEvent::GameMode(on)]
            }
            Command::FindEarbuds { .. } => vec![],
        }
    }
}
