use std::boxed::Box;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};

use crate::core::{Advertisement, ErrorKind, Link, Notifications, Transport, TransportError};
use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{
    Central, CentralEvent, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::{Stream, StreamExt};
use tokio::select;
use uuid::Uuid;

type Events = Pin<Box<dyn Stream<Item = CentralEvent> + Send>>;

#[derive(Debug)]
pub enum Error {
    Btleplug(btleplug::Error),
    NoAdapter,
    /// Service or characteristic missing on the device.
    NotFound,
    Disconnected,
}

impl From<btleplug::Error> for Error {
    fn from(e: btleplug::Error) -> Self {
        match e {
            btleplug::Error::NotConnected => Self::Disconnected,
            e => Self::Btleplug(e),
        }
    }
}

impl TransportError for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Disconnected => ErrorKind::Disconnected,
            Self::NotFound => ErrorKind::NotFound,
            _ => ErrorKind::Other,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Btleplug(e) => e.fmt(f),
            Self::NoAdapter => f.write_str("no Bluetooth adapter found"),
            Self::NotFound => f.write_str("service or characteristic not found"),
            Self::Disconnected => f.write_str("disconnected"),
        }
    }
}

impl std::error::Error for Error {}

/// btleplug-backed central. Must be used inside a Tokio runtime.
pub struct BtleplugTransport {
    adapter: Adapter,
    events: Option<Events>,
}

impl BtleplugTransport {
    /// Use the system's first Bluetooth adapter.
    pub async fn new() -> Result<Self, Error> {
        let adapter = Manager::new().await?.adapters().await?.into_iter().next().ok_or(Error::NoAdapter)?;
        Ok(Self::with_adapter(adapter))
    }

    pub fn with_adapter(adapter: Adapter) -> Self {
        Self { adapter, events: None }
    }
}

impl Transport for BtleplugTransport {
    type Error = Error;
    type Peer = Peripheral;
    type Link = BtleplugLink;

    async fn scan(&mut self) -> Result<(Peripheral, Advertisement), Error> {
        if self.events.is_none() {
            // Subscribe before scanning so no early report is missed.
            self.events = Some(self.adapter.events().await?);
            self.adapter.start_scan(ScanFilter::default()).await?;
        }
        let events = self.events.as_mut().expect("set above");
        while let Some(event) = events.next().await {
            let (CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id)) = event else { continue };
            // Devices can vanish between the event and the lookup.
            let Ok(peripheral) = self.adapter.peripheral(&id).await else { continue };
            let Ok(Some(props)) = peripheral.properties().await else { continue };
            let mut adv = Advertisement::empty();
            adv.rssi = props.rssi.map(|r| r.clamp(i8::MIN as i16, i8::MAX as i16) as i8);
            if let Some(name) = &props.local_name {
                adv.set_name(name.as_bytes());
            }
            for uuid in &props.services {
                if let Some(short) = short_uuid(uuid) {
                    adv.add_service(short);
                }
            }
            return Ok((peripheral, adv));
        }
        Err(Error::Disconnected)
    }

    async fn connect(&mut self, peer: &Peripheral) -> Result<BtleplugLink, Error> {
        if self.events.take().is_some() {
            self.adapter.stop_scan().await?;
        }
        peer.connect().await?;
        peer.discover_services().await?;
        Ok(BtleplugLink { peripheral: peer.clone(), adapter: self.adapter.clone() })
    }
}

/// `0000xxxx-0000-1000-8000-00805f9b34fb` -> `xxxx`.
fn short_uuid(uuid: &Uuid) -> Option<u16> {
    let v = uuid.as_u128();
    (uuid_from_u16((v >> 96) as u16) == *uuid).then_some((v >> 96) as u16)
}

pub struct BtleplugLink {
    peripheral: Peripheral,
    adapter: Adapter,
}

impl Link for BtleplugLink {
    type Error = Error;
    type Characteristic = Characteristic;
    type Notifications<'a> = BtleplugNotifications;

    async fn characteristic(&self, service: u16, characteristic: u16) -> Result<Characteristic, Error> {
        let (service, characteristic) = (uuid_from_u16(service), uuid_from_u16(characteristic));
        self.peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.service_uuid == service && c.uuid == characteristic)
            .ok_or(Error::NotFound)
    }

    async fn read(&self, c: &Characteristic, buf: &mut [u8]) -> Result<usize, Error> {
        let data = self.peripheral.read(c).await?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }

    async fn write(&self, c: &Characteristic, data: &[u8]) -> Result<(), Error> {
        let kind = if c.properties.contains(CharPropFlags::WRITE) {
            WriteType::WithResponse
        } else {
            WriteType::WithoutResponse
        };
        Ok(self.peripheral.write(c, data, kind).await?)
    }

    async fn subscribe(&self, c: &Characteristic) -> Result<BtleplugNotifications, Error> {
        // Open both streams before enabling notifications so none are missed.
        let values = self.peripheral.notifications().await?;
        let events = self.adapter.events().await?;
        self.peripheral.subscribe(c).await?;
        Ok(BtleplugNotifications {
            values: Mutex::new(values),
            events: Mutex::new(events),
            id: self.peripheral.id(),
            uuid: c.uuid,
        })
    }

    async fn disconnect(&self) -> Result<(), Error> {
        Ok(self.peripheral.disconnect().await?)
    }
}

pub struct BtleplugNotifications {
    // `Mutex` only makes these `Sync` (so `Scale` futures are `Send`, e.g. for `tokio::spawn`);
    // access is always exclusive through `&mut self`, so it never blocks.
    values: Mutex<Pin<Box<dyn Stream<Item = ValueNotification> + Send>>>,
    events: Mutex<Events>,
    id: PeripheralId,
    uuid: Uuid,
}

impl Notifications for BtleplugNotifications {
    type Error = Error;

    /// Cancel-safe: both branches are `Stream::next`.
    async fn next(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let values = self.values.get_mut().unwrap_or_else(PoisonError::into_inner);
        let events = self.events.get_mut().unwrap_or_else(PoisonError::into_inner);
        loop {
            select! {
                v = values.next() => match v {
                    Some(v) if v.uuid == self.uuid => {
                        let n = v.value.len().min(buf.len());
                        buf[..n].copy_from_slice(&v.value[..n]);
                        return Ok(n);
                    }
                    Some(_) => {}
                    None => return Err(Error::Disconnected),
                },
                // Not every platform ends the value stream on disconnect.
                e = events.next() => match e {
                    Some(CentralEvent::DeviceDisconnected(id)) if id == self.id => return Err(Error::Disconnected),
                    Some(_) => {}
                    None => return Err(Error::Disconnected),
                },
            }
        }
    }
}
