//! WinRT GATT client for the BP1 Pro ANC.
//!
//! The thread that calls [`run`] owns every WinRT call. Async operations are
//! waited on with `join()`, which blocks that thread only, never the UI.
//!
//! Discovery is deliberately unfiltered: these buds put their service UUID in
//! the scan response, and matching after the event arrives is what the upstream
//! captures show working. The name is a fallback for firmware that omits the
//! UUID during the first second of advertising.

use std::time::Duration;

use baseus_protocol::{AncMode, Side};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use windows::core::{IInspectable, GUID};
use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
    BluetoothLEScanningMode,
};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue,
    GattCommunicationStatus, GattDeviceService, GattValueChangedEventArgs, GattWriteOption,
};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothLEDevice};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

use crate::{handle_notification, Event, Outcome, Request, TransportError};

const SCAN_TIMEOUT: Duration = Duration::from_secs(15);
/// Shorter scan used by the quiet retries after "not found".
const RETRY_SCAN: Duration = Duration::from_secs(3);
/// How often the worker tries again on its own while the buds are missing.
const NOT_FOUND_RETRY: Duration = Duration::from_secs(4);
/// Stand-in deadline when no find tone is playing.
const IDLE_SLEEP: Duration = Duration::from_secs(24 * 60 * 60);
const ADVERTISED_NAME: &str = "Bass BP1 Pro";
const CONNECT_BACKOFF_START: Duration = Duration::from_secs(2);
const CONNECT_BACKOFF_MAX: Duration = Duration::from_secs(30);

fn service_uuid() -> GUID {
    GUID::from_u128(0x5352_7aa4_29f7_ae11_4e74_9973_3478_2568)
}

fn write_uuid() -> GUID {
    GUID::from_u128(0xee68_4b1a_1e9b_ed3e_ee55_f894_667e_92ac)
}

fn notify_uuid() -> GUID {
    GUID::from_u128(0x654b_749c_e37f_ae1f_ebab_40ca_133e_3690)
}

impl From<windows::core::Error> for TransportError {
    fn from(value: windows::core::Error) -> Self {
        Self::WinRt(value.message())
    }
}

/// Keeps the worker thread inside an MTA for as long as it makes WinRT calls.
///
/// Without this the first activation (`BluetoothLEAdvertisementWatcher::new`)
/// fails with `CO_E_NOTINITIALIZED`: the apartment of the UI thread does not
/// apply to the thread that owns the radio.
struct Apartment;

impl Apartment {
    fn init() -> Option<Self> {
        // SAFETY: `RoUninitialize` runs in the same thread when this drops.
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Some(Self),
            Err(_) => None,
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful `RoInitialize` above.
        unsafe { RoUninitialize() };
    }
}

pub fn run(events: UnboundedSender<Event>, mut requests: UnboundedReceiver<Request>) {
    let _apartment = Apartment::init();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = events.send(Event::Failed(format!(
                "No se pudo iniciar el servicio de conexión: {error}"
            )));
            return;
        }
    };

    runtime.block_on(async move {
        let mut backoff = CONNECT_BACKOFF_START;
        // Only explicit attempts announce "buscando"; a quiet retry leaves the
        // status screen untouched so it does not flicker.
        let mut announce = true;
        let mut notified = false;
        let mut scan_timeout = SCAN_TIMEOUT;

        loop {
            if announce {
                let _ = events.send(Event::Scanning);
            }

            let (connected, outcome) = session(&events, &mut requests, scan_timeout).await;
            if connected {
                // A working link resets the wait, even if it drops right away.
                backoff = CONNECT_BACKOFF_START;
            }

            match outcome {
                Outcome::NotFound => {
                    if !notified {
                        let _ = events.send(Event::NotFound);
                        notified = true;
                    }
                    // Try again on our own every 4 s with a 3 s scan. A click
                    // on the button cuts the wait short.
                    let pause = NOT_FOUND_RETRY.saturating_sub(RETRY_SCAN);
                    match tokio::time::timeout(pause, wait_for_rescan(&mut requests)).await {
                        Ok(true) => {
                            announce = true;
                            notified = false;
                            scan_timeout = SCAN_TIMEOUT;
                        }
                        Ok(false) => return,
                        Err(_) => {
                            announce = false;
                            scan_timeout = RETRY_SCAN;
                        }
                    }
                }
                Outcome::Disconnected => {
                    let _ = events.send(Event::Disconnected);
                    notified = false;
                    announce = true;
                    scan_timeout = SCAN_TIMEOUT;
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(CONNECT_BACKOFF_MAX);
                }
                Outcome::Failed(message) => {
                    let _ = events.send(Event::Failed(message));
                    notified = false;
                    if !wait_for_rescan(&mut requests).await {
                        return;
                    }
                    backoff = CONNECT_BACKOFF_START;
                    announce = true;
                    scan_timeout = SCAN_TIMEOUT;
                }
                Outcome::Closed => return,
            }
        }
    });
}

/// Returns true when the user asked for a new scan, false when the app is closing.
async fn wait_for_rescan(requests: &mut UnboundedReceiver<Request>) -> bool {
    loop {
        match requests.recv().await {
            Some(Request::Rescan) => return true,
            Some(_) => continue,
            None => return false,
        }
    }
}

/// Keeps only the newest equalizer and ANC write, preserving the order of every
/// other request. A burst of preset taps should not replay stale states.
fn coalesce_sends(requests: Vec<Request>) -> Vec<Request> {
    let mut out = Vec::with_capacity(requests.len());
    for request in requests {
        if matches!(&request, Request::Send(baseus_protocol::Command::SetEq(_))) {
            out.retain(|queued| {
                !matches!(queued, Request::Send(baseus_protocol::Command::SetEq(_)))
            });
        }
        if matches!(
            &request,
            Request::Send(baseus_protocol::Command::SetAnc { .. })
        ) {
            out.retain(|queued| {
                !matches!(
                    queued,
                    Request::Send(baseus_protocol::Command::SetAnc { .. })
                )
            });
        }
        out.push(request);
    }
    out
}

async fn session(
    events: &UnboundedSender<Event>,
    requests: &mut UnboundedReceiver<Request>,
    scan_timeout: Duration,
) -> (bool, Outcome) {
    let address = match find_earbuds(scan_timeout) {
        Ok(Some(address)) => address,
        Ok(None) => return (false, Outcome::NotFound),
        Err(error) => return (false, Outcome::Failed(friendly(&error))),
    };

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (gone_tx, mut gone_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let connection = match Connection::open(address, notify_tx, gone_tx) {
        Ok(connection) => connection,
        Err(TransportError::NotFound) => return (false, Outcome::NotFound),
        Err(error) => return (false, Outcome::Failed(friendly(&error))),
    };

    let _ = events.send(Event::Connected);

    // Handshake then a preset query: the device answers both, which gives the UI
    // real state without polling for it.
    if let Err(error) = connection
        .write(&baseus_protocol::Command::Handshake.encode())
        .and_then(|()| connection.write(&baseus_protocol::Command::QueryEq.encode()))
    {
        return (true, Outcome::Failed(friendly(&error)));
    }

    let mut last_anc: Option<AncMode> = None;
    let mut find_deadline: Option<tokio::time::Instant> = None;
    loop {
        let deadline = find_deadline.unwrap_or_else(|| tokio::time::Instant::now() + IDLE_SLEEP);
        tokio::select! {
            Some(bytes) = notify_rx.recv() => {
                handle_notification(&bytes, events, &mut last_anc);
            }
            request = requests.recv() => match request {
                Some(Request::Rescan) => {
                    // Leaving the session drops the link, so silence the tone first.
                    let _ = write_find(&connection, false);
                    return (true, Outcome::Disconnected);
                }
                Some(Request::Send(command)) => {
                    let coalesce = matches!(
                        &command,
                        baseus_protocol::Command::SetEq(_)
                            | baseus_protocol::Command::SetAnc { .. }
                    );
                    let mut batch = vec![Request::Send(command)];
                    if coalesce {
                        while let Ok(next) = requests.try_recv() {
                            batch.push(next);
                        }
                    }
                    for request in coalesce_sends(batch) {
                        match request {
                            Request::Rescan => {
                                let _ = write_find(&connection, false);
                                return (true, Outcome::Disconnected);
                            }
                            Request::Send(command) => {
                                if let baseus_protocol::Command::SetAnc { mode, .. } = command {
                                    last_anc = Some(mode);
                                }
                                if let Err(error) = connection.write(&command.encode()) {
                                    return (true, Outcome::Failed(friendly(&error)));
                                }
                            }
                            Request::FindFor(duration) => {
                                if let Err(error) = write_find(&connection, true) {
                                    return (true, Outcome::Failed(friendly(&error)));
                                }
                                find_deadline =
                                    Some(tokio::time::Instant::now() + duration);
                            }
                            Request::StopFind => {
                                let _ = write_find(&connection, false);
                                find_deadline = None;
                            }
                        }
                    }
                }
                Some(Request::FindFor(duration)) => {
                    if let Err(error) = write_find(&connection, true) {
                        return (true, Outcome::Failed(friendly(&error)));
                    }
                    find_deadline = Some(tokio::time::Instant::now() + duration);
                }
                Some(Request::StopFind) => {
                    let _ = write_find(&connection, false);
                    find_deadline = None;
                }
                None => {
                    let _ = write_find(&connection, false);
                    return (true, Outcome::Closed);
                }
            },
            _ = tokio::time::sleep_until(deadline) => {
                if find_deadline.take().is_some() {
                    let _ = write_find(&connection, false);
                }
            }
            _ = gone_rx.recv() => {
                let _ = write_find(&connection, false);
                return (true, Outcome::Disconnected);
            }
            else => {
                let _ = write_find(&connection, false);
                return (true, Outcome::Disconnected);
            }
        }
    }
}

/// Sends the find-tone switch to both buds, so neither keeps ringing alone.
fn write_find(connection: &Connection, on: bool) -> Result<(), TransportError> {
    for side in [Side::Left, Side::Right] {
        connection.write(&baseus_protocol::Command::FindEarbuds { side, on }.encode())?;
    }
    Ok(())
}

/// Buds already connected to Windows stop advertising, so ask the device
/// enumerator first and skip the scan when their GATT service is present.
fn find_connected_device() -> Result<Option<u64>, TransportError> {
    let found = (|| -> windows::core::Result<Option<u64>> {
        let selector = GattDeviceService::GetDeviceSelectorFromUuid(service_uuid())?;
        let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;
        if devices.Size()? == 0 {
            return Ok(None);
        }
        let id = devices.GetAt(0)?.Id()?;
        let device = BluetoothLEDevice::FromIdAsync(&id)?.join()?;
        Ok(Some(device.BluetoothAddress()?))
    })();
    Ok(found.unwrap_or(None))
}

fn find_earbuds(scan_timeout: Duration) -> Result<Option<u64>, TransportError> {
    if let Some(address) = find_connected_device()? {
        return Ok(Some(address));
    }

    // Creating or starting the watcher fails when the radio is off or policy
    // blocks it. Windows returns several different HRESULTs for that, so the
    // type, not the message, is what the UI sees.
    let watcher =
        BluetoothLEAdvertisementWatcher::new().map_err(|_| TransportError::RadioUnavailable)?;
    watcher
        .SetScanningMode(BluetoothLEScanningMode::Active)
        .map_err(|_| TransportError::RadioUnavailable)?;

    let (found_tx, found_rx) = std::sync::mpsc::channel::<u64>();
    let handler = TypedEventHandler::<
        BluetoothLEAdvertisementWatcher,
        BluetoothLEAdvertisementReceivedEventArgs,
    >::new(move |_sender, args| {
        let Ok(args) = args.ok() else {
            return Ok(());
        };
        if let Ok(advertisement) = args.Advertisement() {
            let by_name = advertisement
                .LocalName()
                .map(|name| name.to_string().eq_ignore_ascii_case(ADVERTISED_NAME))
                .unwrap_or(false);
            let by_service = (|| {
                let uuids = advertisement.ServiceUuids().ok()?;
                let size = uuids.Size().ok()?;
                for i in 0..size {
                    if uuids.GetAt(i).ok()? == service_uuid() {
                        return Some(true);
                    }
                }
                Some(false)
            })()
            .unwrap_or(false);
            if by_name || by_service {
                if let Ok(address) = args.BluetoothAddress() {
                    let _ = found_tx.send(address);
                }
            }
        }
        Ok(())
    });

    let token = watcher.Received(&handler)?;
    watcher
        .Start()
        .map_err(|_| TransportError::RadioUnavailable)?;
    let address = found_rx.recv_timeout(scan_timeout).ok();
    let _ = watcher.Stop();
    let _ = watcher.RemoveReceived(token);
    Ok(address)
}

/// A live GATT session. Dropping it unsubscribes and closes the device.
struct Connection {
    device: BluetoothLEDevice,
    write: GattCharacteristic,
    notify: GattCharacteristic,
    notify_token: i64,
    status_token: i64,
}

impl Connection {
    fn open(
        address: u64,
        notifications: UnboundedSender<Vec<u8>>,
        gone: UnboundedSender<()>,
    ) -> Result<Self, TransportError> {
        let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.join()?;

        let services = device.GetGattServicesForUuidAsync(service_uuid())?.join()?;
        if services.Status()? != GattCommunicationStatus::Success {
            return Err(TransportError::ServiceNotFound);
        }
        let services = services.Services()?;
        if services.Size()? == 0 {
            return Err(TransportError::ServiceNotFound);
        }
        let service = services.GetAt(0)?;

        let notify = characteristic(service.clone(), notify_uuid())?;
        let write = characteristic(service, write_uuid())?;

        let value_handler = TypedEventHandler::<GattCharacteristic, GattValueChangedEventArgs>::new(
            move |_sender, args| {
                let Ok(args) = args.ok() else {
                    return Ok(());
                };
                if let Ok(buffer) = args.CharacteristicValue() {
                    if let Ok(reader) = DataReader::FromBuffer(&buffer) {
                        let length = reader.UnconsumedBufferLength().unwrap_or(0) as usize;
                        let mut bytes = vec![0u8; length];
                        if reader.ReadBytes(&mut bytes).is_ok() {
                            let _ = notifications.send(bytes);
                        }
                    }
                }
                Ok(())
            },
        );
        let notify_token = notify.ValueChanged(&value_handler)?;

        let status_handler =
            TypedEventHandler::<BluetoothLEDevice, IInspectable>::new(move |sender, _| {
                let Ok(sender) = sender.ok() else {
                    return Ok(());
                };
                let disconnected = sender
                    .ConnectionStatus()
                    .map(|status| status == BluetoothConnectionStatus::Disconnected)
                    .unwrap_or(false);
                if disconnected {
                    let _ = gone.send(());
                }
                Ok(())
            });
        let status_token = device.ConnectionStatusChanged(&status_handler)?;

        let subscribed = notify
            .WriteClientCharacteristicConfigurationDescriptorAsync(
                GattClientCharacteristicConfigurationDescriptorValue::Notify,
            )?
            .join()?;
        if subscribed != GattCommunicationStatus::Success {
            return Err(TransportError::ServiceNotFound);
        }

        Ok(Self {
            device,
            write,
            notify,
            notify_token,
            status_token,
        })
    }

    fn write(&self, bytes: &[u8]) -> Result<(), TransportError> {
        crate::debug::traffic("TX", bytes);
        let writer = DataWriter::new()?;
        writer.WriteBytes(bytes)?;
        let buffer = writer.DetachBuffer()?;
        let status = self
            .write
            .WriteValueWithOptionAsync(&buffer, GattWriteOption::WriteWithResponse)?
            .join()?;
        match status {
            GattCommunicationStatus::Success => Ok(()),
            other => Err(TransportError::Rejected(format!("{other:?}"))),
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.notify.RemoveValueChanged(self.notify_token);
        let _ = self.device.RemoveConnectionStatusChanged(self.status_token);
        let _ = self
            .notify
            .WriteClientCharacteristicConfigurationDescriptorAsync(
                GattClientCharacteristicConfigurationDescriptorValue::None,
            );
        let _ = self.device.Close();
    }
}

fn characteristic(
    service: windows::Devices::Bluetooth::GenericAttributeProfile::GattDeviceService,
    uuid: GUID,
) -> Result<GattCharacteristic, TransportError> {
    let result = service.GetCharacteristicsForUuidAsync(uuid)?.join()?;
    if result.Status()? != GattCommunicationStatus::Success {
        return Err(TransportError::ServiceNotFound);
    }
    let characteristics = result.Characteristics()?;
    if characteristics.Size()? == 0 {
        return Err(TransportError::ServiceNotFound);
    }
    Ok(characteristics.GetAt(0)?)
}

/// Turn the Windows error codes into something a person can act on.
fn friendly(error: &TransportError) -> String {
    match error {
        TransportError::NoAdapter => {
            "Windows no encuentra el adaptador Bluetooth. Revisa que esté encendido.".to_owned()
        }
        TransportError::RadioUnavailable => {
            "El Bluetooth está apagado o Windows no permite usarlo. Enciéndelo en \
             Configuración y vuelve a intentarlo."
                .to_owned()
        }
        TransportError::ServiceNotFound => {
            "Los audífonos se conectaron, pero no exponen el servicio esperado. Puede que \
             este modelo no sea compatible."
                .to_owned()
        }
        TransportError::Rejected(_) => {
            "Los audífonos rechazaron el cambio. Prueba otra vez.".to_owned()
        }
        TransportError::NotFound => "No se encontraron los audífonos.".to_owned(),
        TransportError::Disconnected => "Se perdió la conexión.".to_owned(),
        TransportError::WinRt(raw) => {
            let lower = raw.to_lowercase();
            if lower.contains("radio") || lower.contains("0x800710df") {
                "El Bluetooth está apagado o Windows no permite usarlo. Enciéndelo en \
                 Configuración y vuelve a intentarlo."
                    .to_owned()
            } else if lower.contains("0x80070490") {
                "Windows no encuentra el adaptador Bluetooth. Revisa que esté encendido.".to_owned()
            } else if lower.contains("0x80070005") {
                "Windows bloqueó el acceso al Bluetooth. Revisa los permisos en Configuración."
                    .to_owned()
            } else {
                "No se pudo completar la conexión. Vuelve a intentarlo en unos segundos.".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_radio_that_is_not_ready_asks_the_user_to_turn_bluetooth_on() {
        let message = friendly(&TransportError::WinRt(
            "El dispositivo no está listo para su uso. (0x800710DF)".to_owned(),
        ));
        assert!(message.contains("Bluetooth está apagado"), "{message}");
    }

    #[test]
    fn a_missing_device_says_so_instead_of_blaming_the_radio() {
        let message = friendly(&TransportError::NotFound);
        assert!(message.contains("No se encontraron"), "{message}");
    }

    #[test]
    fn unknown_failures_stay_generic() {
        let message = friendly(&TransportError::WinRt("0x80004005".to_owned()));
        assert!(message.contains("No se pudo completar"), "{message}");
    }

    #[test]
    fn coalesce_keeps_only_the_newest_eq_and_anc_writes() {
        let eq = |preset| Request::Send(baseus_protocol::Command::SetEq(preset));
        let anc = |mode| Request::Send(baseus_protocol::Command::SetAnc { mode, level: 0x68 });
        let find = Request::FindFor(std::time::Duration::from_secs(8));
        let input = vec![
            eq(baseus_protocol::EqPreset::Balanced),
            find,
            eq(baseus_protocol::EqPreset::BassBoost),
            anc(AncMode::Off),
            Request::StopFind,
            anc(AncMode::Anc),
        ];
        assert_eq!(
            coalesce_sends(input),
            vec![
                find,
                eq(baseus_protocol::EqPreset::BassBoost),
                Request::StopFind,
                anc(AncMode::Anc),
            ]
        );
    }
}
