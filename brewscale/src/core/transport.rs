//! Backend-facing traits. Async fns in traits: no boxing, no runtime.
#![allow(async_fn_in_trait)]

/// Classification a backend gives its errors so the driver can react
/// (e.g. reconnect) without knowing the BLE stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Disconnected,
    NotFound,
    Other,
}

pub trait TransportError: core::fmt::Debug {
    fn kind(&self) -> ErrorKind;
}

const NAME_MAX: usize = 32;
const SERVICES_MAX: usize = 4;

/// What a scan reported about one device. Parsed and fixed-size, so it works
/// both from raw AD bytes (embedded) and from OS-parsed properties (desktop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advertisement {
    pub rssi: Option<i8>,
    name: [u8; NAME_MAX],
    name_len: u8,
    services: [u16; SERVICES_MAX],
    services_len: u8,
}

impl Advertisement {
    pub const fn empty() -> Self {
        Self { rssi: None, name: [0; NAME_MAX], name_len: 0, services: [0; SERVICES_MAX], services_len: 0 }
    }

    /// Parse raw advertising/scan-response data (length-type-value records).
    /// Malformed trailing records are ignored.
    pub fn from_ad_bytes(rssi: Option<i8>, mut data: &[u8]) -> Self {
        let mut adv = Self { rssi, ..Self::empty() };
        while let [len, rest @ ..] = data {
            let len = *len as usize;
            if len == 0 || len > rest.len() {
                break;
            }
            let (record, next) = rest.split_at(len);
            let (ty, value) = (record[0], &record[1..]);
            match ty {
                0x08 | 0x09 => adv.set_name(value),
                0x02 | 0x03 => value.chunks_exact(2).for_each(|c| adv.add_service(u16::from_le_bytes([c[0], c[1]]))),
                _ => {}
            }
            data = next;
        }
        adv
    }

    pub fn set_name(&mut self, name: &[u8]) {
        let n = name.len().min(NAME_MAX);
        self.name[..n].copy_from_slice(&name[..n]);
        self.name_len = n as u8;
    }

    pub fn add_service(&mut self, uuid16: u16) {
        if (self.services_len as usize) < SERVICES_MAX {
            self.services[self.services_len as usize] = uuid16;
            self.services_len += 1;
        }
    }

    /// Local name bytes (may be truncated to 32 bytes). Empty if not advertised.
    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }

    pub fn has_service(&self, uuid16: u16) -> bool {
        self.services[..self.services_len as usize].contains(&uuid16)
    }
}

/// Central-role BLE adapter: discovery and connection establishment.
pub trait Transport {
    type Error: TransportError;
    /// Opaque device handle (an address on embedded, a platform id on desktop).
    type Peer: Clone;
    type Link: Link<Error = Self::Error>;

    /// Wait for the next advertisement report, starting a scan if needed.
    /// Scanning continues between calls until [`Transport::connect`].
    async fn scan(&mut self) -> Result<(Self::Peer, Advertisement), Self::Error>;

    /// Stop scanning and open a GATT connection.
    async fn connect(&mut self, peer: &Self::Peer) -> Result<Self::Link, Self::Error>;
}

/// An established GATT connection. All methods take `&self` so commands can
/// be sent while a [`Notifications`] stream exists.
pub trait Link {
    type Error: TransportError;
    /// Handle to a discovered characteristic.
    type Characteristic;
    type Notifications<'a>: Notifications<Error = Self::Error>
    where
        Self: 'a;

    /// Discover a service and one of its characteristics by 16-bit UUID.
    async fn characteristic(&self, service: u16, characteristic: u16) -> Result<Self::Characteristic, Self::Error>;
    async fn read(&self, c: &Self::Characteristic, buf: &mut [u8]) -> Result<usize, Self::Error>;
    async fn write(&self, c: &Self::Characteristic, data: &[u8]) -> Result<(), Self::Error>;
    /// Enable notifications on `c` and return the stream of their payloads.
    async fn subscribe(&self, c: &Self::Characteristic) -> Result<Self::Notifications<'_>, Self::Error>;
    async fn disconnect(&self) -> Result<(), Self::Error>;
}

pub trait Notifications {
    type Error: TransportError;
    /// Copy the next payload into `buf` (truncating) and return its length.
    /// Reports [`ErrorKind::Disconnected`] when the link drops.
    ///
    /// **Must be cancel-safe**: callers `select` this against timers and
    /// commands, and a dropped call must not lose a payload.
    async fn next(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
}
