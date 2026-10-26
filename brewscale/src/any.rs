use crate::core::{
    Advertisement, Capabilities, Command, DecodeError, Detect, EncodeError, Event, Frame, Protocol, Scale,
};

/// A [`Scale`] of whichever protocol was detected.
pub type AnyScale<'l, L> = Scale<'l, L, AnyProtocol>;

/// Declares [`AnyProtocol`]. To add a protocol: create its crate (implement
/// [`Protocol`] + [`Detect`]), add an optional dependency and a feature in
/// `Cargo.toml`, and add one line here.
macro_rules! protocols {
    ($($feature:literal => $Variant:ident($ty:ty)),* $(,)?) => {
        /// Every protocol enabled by Cargo feature, as one [`Protocol`]. With
        /// no protocol feature enabled this type is uninhabited.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum AnyProtocol {
            $(#[cfg(feature = $feature)] $Variant($ty),)*
        }

        // With no protocol enabled the enum is empty and every method is `match *self {}`.
        #[allow(unused_variables)]
        impl Protocol for AnyProtocol {
            fn name(&self) -> &'static str {
                match *self { $(#[cfg(feature = $feature)] Self::$Variant(ref p) => p.name(),)* }
            }
            fn service(&self) -> u16 {
                match *self { $(#[cfg(feature = $feature)] Self::$Variant(ref p) => p.service(),)* }
            }
            fn notify_characteristic(&self) -> u16 {
                match *self { $(#[cfg(feature = $feature)] Self::$Variant(ref p) => p.notify_characteristic(),)* }
            }
            fn command_characteristic(&self) -> u16 {
                match *self { $(#[cfg(feature = $feature)] Self::$Variant(ref p) => p.command_characteristic(),)* }
            }
            fn capabilities(&self) -> Capabilities {
                match *self { $(#[cfg(feature = $feature)] Self::$Variant(ref p) => p.capabilities(),)* }
            }
            fn decode(&self, packet: &[u8]) -> Result<Event, DecodeError> {
                match *self { $(#[cfg(feature = $feature)] Self::$Variant(ref p) => p.decode(packet),)* }
            }
            fn encode(&self, command: Command) -> Result<Frame, EncodeError> {
                match *self { $(#[cfg(feature = $feature)] Self::$Variant(ref p) => p.encode(command),)* }
            }
        }

        impl Detect for AnyProtocol {
            fn detect(adv: &Advertisement) -> Option<Self> {
                let _ = adv;
                $(
                    #[cfg(feature = $feature)]
                    if let Some(p) = <$ty as Detect>::detect(adv) {
                        return Some(Self::$Variant(p));
                    }
                )*
                None
            }
        }
    };
}

protocols! {
    "bookoo" => Bookoo(crate::bookoo::Bookoo),
}
