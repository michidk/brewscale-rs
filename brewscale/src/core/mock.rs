//! In-memory [`Transport`] for driving the drivers without Bluetooth.
//! `no_std`, no alloc, no executor: use [`block_on`] for the always-ready futures.

use core::cell::RefCell;
use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

use crate::{Advertisement, ErrorKind, Link, Notifications, Transport, TransportError};

#[derive(Debug, PartialEq, Eq)]
pub enum MockError {
    Disconnected,
    NotFound,
}

impl TransportError for MockError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Disconnected => ErrorKind::Disconnected,
            Self::NotFound => ErrorKind::NotFound,
        }
    }
}

/// Poll a future that never waits on anything but the mock.
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    match f.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("mock future pending"),
    }
}

const MAX_WRITES: usize = 8;

/// Scripted device: advertises `adv`, delivers `notifications` in order, then
/// reports a disconnect. Writes are recorded.
pub struct MockTransport<'a> {
    pub adv: Advertisement,
    /// The only service the device exposes.
    pub service: u16,
    pub notifications: &'a [&'a [u8]],
    writes: RefCell<([[u8; 8]; MAX_WRITES], usize)>,
}

impl<'a> MockTransport<'a> {
    pub fn new(adv: Advertisement, service: u16, notifications: &'a [&'a [u8]]) -> Self {
        Self { adv, service, notifications, writes: RefCell::new(([[0; 8]; MAX_WRITES], 0)) }
    }

    /// Number of writes, and the `i`th written frame (zero-padded to 8 bytes).
    pub fn write_count(&self) -> usize {
        self.writes.borrow().1
    }
    pub fn write(&self, i: usize) -> [u8; 8] {
        self.writes.borrow().0[i]
    }
}

pub struct MockLink<'t>(&'t MockTransport<'t>);
pub struct MockNotifications<'t> {
    t: &'t MockTransport<'t>,
    next: usize,
}

impl<'t> Transport for &'t MockTransport<'t> {
    type Error = MockError;
    type Peer = ();
    type Link = MockLink<'t>;

    async fn scan(&mut self) -> Result<((), Advertisement), MockError> {
        Ok(((), self.adv))
    }

    async fn connect(&mut self, _: &()) -> Result<Self::Link, MockError> {
        // Deref'ing yields the full `'t`; the lint's suggested auto-deref would shorten it.
        #[allow(clippy::explicit_auto_deref)]
        let t: &'t MockTransport<'t> = *self;
        Ok(MockLink(t))
    }
}

impl<'t> Link for MockLink<'t> {
    type Error = MockError;
    type Characteristic = u16;
    type Notifications<'n>
        = MockNotifications<'t>
    where
        Self: 'n;

    async fn characteristic(&self, service: u16, c: u16) -> Result<u16, MockError> {
        if service == self.0.service {
            Ok(c)
        } else {
            Err(MockError::NotFound)
        }
    }
    async fn read(&self, _: &u16, _: &mut [u8]) -> Result<usize, MockError> {
        Ok(0)
    }
    async fn write(&self, _: &u16, data: &[u8]) -> Result<(), MockError> {
        let mut w = self.0.writes.borrow_mut();
        let i = w.1;
        w.0[i][..data.len()].copy_from_slice(data);
        w.1 += 1;
        Ok(())
    }
    async fn subscribe(&self, _: &u16) -> Result<MockNotifications<'t>, MockError> {
        Ok(MockNotifications { t: self.0, next: 0 })
    }
    async fn disconnect(&self) -> Result<(), MockError> {
        Ok(())
    }
}

impl Notifications for MockNotifications<'_> {
    type Error = MockError;
    async fn next(&mut self, buf: &mut [u8]) -> Result<usize, MockError> {
        let p = self.t.notifications.get(self.next).ok_or(MockError::Disconnected)?;
        self.next += 1;
        let n = p.len().min(buf.len());
        buf[..n].copy_from_slice(&p[..n]);
        Ok(n)
    }
}
