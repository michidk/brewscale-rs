//! [`crate::core::Transport`] on top of [`trouble_host`]: a BLE central
//! that works with any HCI [`Controller`] (e.g. ESP32 via `esp-radio`).
//!
//! `no_std`, no heap. The caller owns the TrouBLE [`Stack`] and must keep its
//! [`Runner`] running next to the application,
//! passing the [`ScanInbox`] as its event handler so scan results reach
//! [`TroubleTransport::scan`]:
//!
//! ```ignore
//! let inbox = ScanInbox::<4>::new();
//! let mut transport = TroubleTransport::new(&stack, &inbox);
//! join(runner.run_with_handler(&inbox), app(&mut transport)).await;
//! ```

use core::future::Future;

use crate::core::{Advertisement, ErrorKind, Link, Notifications, Transport, TransportError};
use bt_hci::cmd::le::{
    LeAddDeviceToFilterAcceptList, LeClearFilterAcceptList, LeCreateConn, LeSetScanEnable, LeSetScanParams,
};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use trouble_host::prelude::*;

/// TrouBLE error wrapped so it can implement [`TransportError`].
#[derive(Debug)]
pub struct Error<E>(pub BleHostError<E>);

impl<E: core::fmt::Debug> TransportError for Error<E> {
    fn kind(&self) -> ErrorKind {
        match &self.0 {
            BleHostError::BleHost(trouble_host::Error::Disconnected) => ErrorKind::Disconnected,
            BleHostError::BleHost(trouble_host::Error::NotFound) => ErrorKind::NotFound,
            _ => ErrorKind::Other,
        }
    }
}

impl<E> From<BleHostError<E>> for Error<E> {
    fn from(e: BleHostError<E>) -> Self {
        Self(e)
    }
}

/// Hand-off from TrouBLE's synchronous advertising callback to async
/// [`Transport::scan`]. Holds up to `N` reports; extra reports are dropped
/// (devices re-advertise, so a scale is still found).
pub struct ScanInbox<const N: usize = 4> {
    reports: Channel<NoopRawMutex, (Address, Advertisement), N>,
}

impl<const N: usize> ScanInbox<N> {
    pub const fn new() -> Self {
        Self { reports: Channel::new() }
    }
}

impl<const N: usize> Default for ScanInbox<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> EventHandler for ScanInbox<N> {
    fn on_adv_reports(&self, mut reports: LeAdvReportsIter<'_>) {
        while let Some(Ok(r)) = reports.next() {
            let adv = Advertisement::from_ad_bytes(Some(r.rssi), r.data);
            let _ = self.reports.try_send((Address { kind: r.addr_kind, addr: r.addr }, adv));
        }
    }
}

/// Central-role [`Transport`]. `N` is the [`ScanInbox`] capacity, `S` the
/// number of GATT services a [`TroubleLink`] can cache.
pub struct TroubleTransport<'s, 'r, C: Controller, P: PacketPool, const N: usize = 4, const S: usize = 4> {
    stack: &'s Stack<'r, C, P>,
    central: Central<'s, C, P>,
    inbox: &'s ScanInbox<N>,
    scan: Option<ScanSession<'s, false>>,
}

impl<'s, 'r, C: Controller, P: PacketPool, const N: usize, const S: usize> TroubleTransport<'s, 'r, C, P, N, S> {
    pub fn new(stack: &'s Stack<'r, C, P>, inbox: &'s ScanInbox<N>) -> Self {
        Self { stack, central: stack.central(), inbox, scan: None }
    }
}

impl<'s, 'r, C, P, const N: usize, const S: usize> Transport for TroubleTransport<'s, 'r, C, P, N, S>
where
    C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdSync<LeSetScanEnable>
        + ControllerCmdSync<LeClearFilterAcceptList>
        + ControllerCmdSync<LeAddDeviceToFilterAcceptList>
        + ControllerCmdAsync<LeCreateConn>,
    P: PacketPool,
    C::Error: core::fmt::Debug,
{
    type Error = Error<C::Error>;
    type Peer = Address;
    type Link = TroubleLink<'s, C, P, S>;

    async fn scan(&mut self) -> Result<(Address, Advertisement), Self::Error> {
        if self.scan.is_none() {
            // Reports from an earlier scan describe devices that may be gone.
            self.inbox.reports.clear();
            let config = ScanConfig { active: true, ..Default::default() };
            self.scan = Some(Scanner::new(&mut self.central).scan(&config).await?);
        }
        Ok(self.inbox.reports.receive().await)
    }

    async fn connect(&mut self, peer: &Address) -> Result<Self::Link, Self::Error> {
        if let Some(scan) = self.scan.take() {
            scan.stop().await;
        }
        let config = ConnectConfig {
            connect_params: Default::default(),
            scan_config: ScanConfig { filter_accept_list: &[*peer], ..Default::default() },
        };
        let conn = self.central.connect(&config).await?;
        let client = GattClient::new(self.stack, &conn).await?;
        Ok(TroubleLink { conn, client })
    }
}

/// GATT client connection.
///
/// TrouBLE only routes ATT responses and notifications while
/// [`GattClient::task`] is polled, so every operation here polls it alongside
/// the real work. Nothing is processed while no `Link`/`Notifications` future
/// is being awaited; incoming PDUs wait in TrouBLE's packet pool meanwhile.
pub struct TroubleLink<'s, C: Controller, P: PacketPool, const S: usize = 4> {
    conn: Connection<'s, P>,
    client: GattClient<'s, C, P, S>,
}

impl<'s, C: Controller, P: PacketPool, const S: usize> TroubleLink<'s, C, P, S> {
    async fn run<T>(&self, op: impl Future<Output = Result<T, BleHostError<C::Error>>>) -> Result<T, Error<C::Error>> {
        match select(self.client.task(), op).await {
            // `task` only returns on failure (the connection ended).
            Either::First(Err(e)) => Err(e.into()),
            Either::First(Ok(())) => Err(BleHostError::BleHost(trouble_host::Error::Disconnected).into()),
            Either::Second(r) => r.map_err(Into::into),
        }
    }
}

impl<'s, C, P, const S: usize> Link for TroubleLink<'s, C, P, S>
where
    C: Controller,
    P: PacketPool,
    C::Error: core::fmt::Debug,
{
    type Error = Error<C::Error>;
    type Characteristic = Characteristic<[u8]>;
    type Notifications<'a>
        = TroubleNotifications<'a, 's, C, P, S>
    where
        Self: 'a;

    async fn characteristic(&self, service: u16, characteristic: u16) -> Result<Self::Characteristic, Self::Error> {
        self.run(async {
            let services = self.client.services_by_uuid(&Uuid::new_short(service)).await?;
            let service = services.first().ok_or(trouble_host::Error::NotFound)?;
            self.client.characteristic_by_uuid(service, &Uuid::new_short(characteristic)).await
        })
        .await
    }

    async fn read(&self, c: &Self::Characteristic, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.run(self.client.read_characteristic(c, buf)).await
    }

    async fn write(&self, c: &Self::Characteristic, data: &[u8]) -> Result<(), Self::Error> {
        if c.props.any(&[CharacteristicProp::Write]) {
            self.run(self.client.write_characteristic(c, data)).await
        } else {
            self.run(self.client.write_characteristic_without_response(c, data)).await
        }
    }

    async fn subscribe(&self, c: &Self::Characteristic) -> Result<Self::Notifications<'_>, Self::Error> {
        let listener = self.run(self.client.subscribe(c, false)).await?;
        Ok(TroubleNotifications { link: self, listener })
    }

    async fn disconnect(&self) -> Result<(), Self::Error> {
        self.conn.disconnect();
        Ok(())
    }
}

pub struct TroubleNotifications<'a, 's, C: Controller, P: PacketPool, const S: usize> {
    link: &'a TroubleLink<'s, C, P, S>,
    listener: NotificationListener<'a, { trouble_host::config::GATT_CLIENT_NOTIFICATION_MTU }>,
}

impl<C, P, const S: usize> Notifications for TroubleNotifications<'_, '_, C, P, S>
where
    C: Controller,
    P: PacketPool,
    C::Error: core::fmt::Debug,
{
    type Error = Error<C::Error>;

    async fn next(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match select(self.link.client.task(), self.listener.next()).await {
            Either::First(Err(e)) => Err(e.into()),
            Either::First(Ok(())) => Err(BleHostError::BleHost(trouble_host::Error::Disconnected).into()),
            Either::Second(n) => {
                let data = n.as_ref();
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                Ok(len)
            }
        }
    }
}
