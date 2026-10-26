//! Driver tying a [`Protocol`] to a [`Link`].

use core::fmt;

use crate::{
    Command, DecodeError, Detect, EncodeError, ErrorKind, Event, Link, Notifications, Protocol, Transport,
    TransportError,
};

#[derive(Debug)]
pub enum Error<E> {
    /// The link dropped; reconnect from [`find`].
    Disconnected,
    Transport(E),
    /// A notification failed validation (e.g. checksum). Safe to keep reading.
    Decode(DecodeError),
    Encode(EncodeError),
}

impl<E: TransportError> From<E> for Error<E> {
    fn from(e: E) -> Self {
        if e.kind() == ErrorKind::Disconnected {
            Self::Disconnected
        } else {
            Self::Transport(e)
        }
    }
}

impl<E: fmt::Debug> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => f.write_str("disconnected"),
            Self::Transport(e) => write!(f, "transport error: {e:?}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Encode(e) => write!(f, "encode error: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl<E: fmt::Debug> std::error::Error for Error<E> {}

/// Scan until a device protocol `P` recognises is seen. Returns its peer
/// handle and the protocol instance; pass the peer to [`Transport::connect`]
/// and the protocol to [`Scale::attach`]. `P` can be one vendor's protocol or
/// a combined enum (`brewscale::AnyProtocol`).
pub async fn find<P: Detect, T: Transport>(transport: &mut T) -> Result<(T::Peer, P), T::Error> {
    loop {
        let (peer, adv) = transport.scan().await?;
        if let Some(protocol) = P::detect(&adv) {
            return Ok((peer, protocol));
        }
    }
}

/// A connected scale: decoded events out, [`Command`]s in.
pub struct Scale<'l, L: Link, P: Protocol> {
    link: &'l L,
    protocol: P,
    command: L::Characteristic,
    notifications: L::Notifications<'l>,
}

impl<'l, L: Link, P: Protocol> Scale<'l, L, P> {
    /// Discover the protocol's characteristics and subscribe to measurements.
    pub async fn attach(link: &'l L, protocol: P) -> Result<Self, Error<L::Error>> {
        let notify = link.characteristic(protocol.service(), protocol.notify_characteristic()).await?;
        let command = link.characteristic(protocol.service(), protocol.command_characteristic()).await?;
        let notifications = link.subscribe(&notify).await?;
        Ok(Self { link, protocol, command, notifications })
    }

    pub fn protocol(&self) -> &P {
        &self.protocol
    }

    /// Next decoded event. Cancel-safe, so it can be `select`ed against timers
    /// or commands. Packets of unknown type are skipped; a corrupt packet
    /// yields [`Error::Decode`] and the stream stays usable.
    pub async fn next_event(&mut self) -> Result<Event, Error<L::Error>> {
        let mut buf = [0u8; 32];
        loop {
            let n = self.notifications.next(&mut buf).await?;
            match self.protocol.decode(&buf[..n]) {
                Ok(event) => return Ok(event),
                Err(DecodeError::UnknownType(_)) => {}
                Err(e) => return Err(Error::Decode(e)),
            }
        }
    }

    pub async fn send(&self, command: Command) -> Result<(), Error<L::Error>> {
        let frame = self.protocol.encode(command).map_err(Error::Encode)?;
        Ok(self.link.write(&self.command, &frame).await?)
    }

    pub async fn tare(&self) -> Result<(), Error<L::Error>> {
        self.send(Command::Tare).await
    }
}

/// A made-up protocol, to show that nothing here depends on any vendor crate.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{block_on, MockTransport};
    use crate::{Advertisement, Capabilities, EncodeError, Frame};

    #[derive(Clone, Copy)]
    struct Echo;

    impl Protocol for Echo {
        fn name(&self) -> &'static str {
            "Echo"
        }
        fn service(&self) -> u16 {
            0xAAAA
        }
        fn notify_characteristic(&self) -> u16 {
            1
        }
        fn command_characteristic(&self) -> u16 {
            2
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                timer: false,
                powder_weight: false,
                auto_brew_events: false,
                shutdown: false,
                keep_awake: false,
                max_beep_level: 0,
            }
        }
        // packet: [0x01, weight_cg as u16 big endian]
        fn decode(&self, p: &[u8]) -> Result<Event, DecodeError> {
            match p {
                [0x01, hi, lo] => Ok(Event::PowderWeight { weight_cg: u16::from_be_bytes([*hi, *lo]) as i32 }),
                [t, ..] => Err(DecodeError::UnknownType(*t)),
                [] => Err(DecodeError::Length(0)),
            }
        }
        fn encode(&self, c: Command) -> Result<Frame, EncodeError> {
            match c {
                Command::Tare => Ok(Frame::new(b"T")),
                _ => Err(EncodeError::Unsupported),
            }
        }
    }

    impl Detect for Echo {
        fn detect(adv: &Advertisement) -> Option<Self> {
            adv.has_service(0xAAAA).then_some(Echo)
        }
    }

    #[test]
    fn a_new_protocol_plugs_in_without_touching_the_driver() {
        let mut adv = Advertisement::empty();
        adv.add_service(0xAAAA);
        let script: [&[u8]; 2] = [&[0x01, 0x01, 0x2C], &[0x09]];
        let t = MockTransport::new(adv, 0xAAAA, &script);
        block_on(async {
            let mut transport = &t;
            let (peer, echo) = find::<Echo, _>(&mut transport).await.unwrap();
            let link = transport.connect(&peer).await.unwrap();
            let mut scale = Scale::attach(&link, echo).await.unwrap();
            assert_eq!(scale.next_event().await.unwrap(), Event::PowderWeight { weight_cg: 300 });
            scale.tare().await.unwrap();
            assert!(matches!(scale.send(Command::StartTimer).await, Err(Error::Encode(EncodeError::Unsupported))));
            // unknown packet type is skipped, then the script ends: disconnect
            assert!(matches!(scale.next_event().await, Err(Error::Disconnected)));
        });
        assert_eq!(&t.write(0)[..1], b"T");
    }
}
