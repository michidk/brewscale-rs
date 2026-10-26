//! Vendor-neutral data types and the [`Protocol`] trait.

use core::fmt;

use core::ops::Deref;

use crate::Advertisement;

/// Display unit configured on the scale. Reported weights are always grams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WeightUnit {
    Gram,
    Ounce,
    /// Raw unit byte this crate does not know.
    Unknown(u8),
}

/// Periodic reading. Fixed-point fields avoid floats on small MCUs; the
/// `*_g*` helpers convert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measurement {
    /// Timer value in milliseconds.
    pub elapsed_ms: u32,
    /// Weight in 1/100 g (signed).
    pub weight_cg: i32,
    /// Flow rate in 1/100 g/s (signed).
    pub flow_cg_per_s: i32,
    pub unit: WeightUnit,
    pub battery_percent: u8,
    /// Remaining standby time, in 1/10 minute.
    pub standby_deci_min: u16,
    pub beep_level: u8,
    pub flow_smoothing: bool,
}

impl Measurement {
    pub fn weight_g(&self) -> f32 {
        self.weight_cg as f32 / 100.0
    }
    pub fn flow_g_per_s(&self) -> f32 {
        self.flow_cg_per_s as f32 / 100.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoBrewState {
    Stopped,
    Started,
    Ready,
    ExitReady,
    ExitDone,
}

/// Automatic-mode event with settlement data (Ultra).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoBrewEvent {
    pub state: AutoBrewState,
    pub elapsed_ms: u32,
    /// Weight in 1/100 g.
    pub weight_cg: i32,
    /// Average flow rate (timing mode) or ratio (ratio mode), times 100.
    pub result_c: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    Measurement(Measurement),
    /// Configured dose (Ultra), in 1/100 g.
    PowderWeight {
        weight_cg: i32,
    },
    AutoBrew(AutoBrewEvent),
}

/// Scale-independent commands. Not every scale supports every command; see
/// [`Protocol::capabilities`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    Tare,
    StartTimer,
    StopTimer,
    ResetTimer,
    TareAndStartTimer,
    SetFlowSmoothing(bool),
    SetBeepLevel(u8),
    /// Auto power-off delay, 5..=30 minutes.
    SetAutoOff {
        minutes: u8,
    },
    /// Dose weight in 1/10 g, 1..=9999.
    SetPowderWeight {
        deci_g: u16,
    },
    Shutdown,
    /// Resets the auto power-off countdown.
    KeepAwake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub timer: bool,
    pub powder_weight: bool,
    pub auto_brew_events: bool,
    pub shutdown: bool,
    pub keep_awake: bool,
    pub max_beep_level: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// Not a full 20-byte packet.
    Length(usize),
    /// Wrong product-number byte.
    Header(u8),
    Checksum {
        expected: u8,
        got: u8,
    },
    /// Well-formed packet of a type this driver does not decode.
    UnknownType(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeError {
    Unsupported,
    OutOfRange,
}

macro_rules! impl_error {
    ($t:ty, $($pat:pat => $($arg:expr),+ ;)+) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $($pat => write!(f, $($arg),+),)+ }
            }
        }
        #[cfg(feature = "std")]
        impl std::error::Error for $t {}
    };
}
impl_error!(DecodeError,
    Self::Length(n) => "unexpected packet length {}", n;
    Self::Header(h) => "unexpected header byte {:#04x}", h;
    Self::Checksum { expected, got } => "bad checksum: expected {:#04x}, got {:#04x}", expected, got;
    Self::UnknownType(t) => "unknown packet type {:#04x}", t;
);
impl_error!(EncodeError,
    Self::Unsupported => "command not supported by this scale";
    Self::OutOfRange => "command argument out of range";
);

/// An encoded command, ready to write. Derefs to the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    buf: [u8; 8],
    len: u8,
}

impl Frame {
    pub fn new(bytes: &[u8]) -> Self {
        let mut buf = [0; 8];
        buf[..bytes.len()].copy_from_slice(bytes);
        Self { buf, len: bytes.len() as u8 }
    }
}

impl Deref for Frame {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }
}

/// A scale's wire protocol: GATT layout, packet decoding, command encoding.
/// Implementations must be pure (no I/O, no allocation).
pub trait Protocol {
    /// Human-readable device name, e.g. for logs.
    fn name(&self) -> &'static str;
    /// 16-bit UUID of the primary service.
    fn service(&self) -> u16;
    /// Characteristic that notifies measurements.
    fn notify_characteristic(&self) -> u16;
    /// Characteristic that accepts commands.
    fn command_characteristic(&self) -> u16;
    fn capabilities(&self) -> Capabilities;
    fn decode(&self, packet: &[u8]) -> Result<Event, DecodeError>;
    fn encode(&self, command: Command) -> Result<Frame, EncodeError>;
}

/// A protocol that can recognise its devices from a scan report, so
/// applications can discover scales without naming a vendor.
pub trait Detect: Protocol + Sized {
    /// `Some` if `adv` looks like a device this protocol drives.
    fn detect(adv: &Advertisement) -> Option<Self>;
}
