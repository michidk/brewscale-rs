//! BOOKOO Themis Mini/Ultra protocol drivers. Wire format:
//! <https://github.com/BooKooCode/OpenSource> (`bookoo_mini_scale`, `bookoo_ultra_scale`).
//!
//! Service `0x0FFE`; notify `0xFF11`; write `0xFF12`. Every packet is
//! `[0x03, type, data.., xor]` where `xor` is the XOR of all preceding bytes.

mod mini;
mod ultra;

pub use mini::Mini;
pub use ultra::Ultra;

use crate::core::{
    Advertisement, AutoBrewEvent, AutoBrewState, Capabilities, Command, DecodeError, Detect, EncodeError, Event, Frame,
    Measurement, Protocol, WeightUnit,
};

pub const SERVICE: u16 = 0x0FFE;
pub const WEIGHT_CHARACTERISTIC: u16 = 0xFF11;
pub const COMMAND_CHARACTERISTIC: u16 = 0xFF12;

const PRODUCT: u8 = 0x03;
const PACKET_LEN: usize = 20;
const TYPE_COMMAND: u8 = 0x0A;
const TYPE_MEASUREMENT: u8 = 0x0B;
const TYPE_AUTO_BREW: u8 = 0x0D;
const TYPE_POWDER: u8 = 0x0F;
/// ASCII '-': the only sign byte value meaning negative.
const MINUS: u8 = 0x2D;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Mini,
    Ultra,
}

impl Model {
    /// Identify a scale from its advertised name: `BOOKOO_SC xxxx` (Mini) or
    /// `BOOKOO_SC U xxxx` (Ultra). Matches from `OOKOO_SC` because Mini
    /// firmware 1.4.1 reportedly advertises a name missing its leading `B`
    /// (decentespresso/decaid#972), and falls back to the `0x0FFE` service.
    pub fn identify(adv: &Advertisement) -> Option<Self> {
        const NEEDLE: &[u8] = b"OOKOO_SC";
        let name = adv.name();
        match name.windows(NEEDLE.len()).position(|w| w == NEEDLE) {
            Some(i) => {
                let rest = &name[i + NEEDLE.len()..];
                Some(if rest.starts_with(b" U ") || rest == b" U" { Self::Ultra } else { Self::Mini })
            }
            // Service match alone can't tell models apart; Mini commands are
            // the safe subset.
            None if adv.has_service(SERVICE) => Some(Self::Mini),
            None => None,
        }
    }
}

/// Protocol selected at runtime, e.g. from [`Model::identify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bookoo(pub Model);

macro_rules! dispatch {
    ($self:ident, $p:ident => $e:expr) => {
        match $self.0 {
            Model::Mini => {
                let $p = Mini;
                $e
            }
            Model::Ultra => {
                let $p = Ultra;
                $e
            }
        }
    };
}

impl Detect for Bookoo {
    fn detect(adv: &Advertisement) -> Option<Self> {
        Model::identify(adv).map(Self)
    }
}

impl Protocol for Bookoo {
    fn name(&self) -> &'static str {
        dispatch!(self, p => p.name())
    }
    fn service(&self) -> u16 {
        SERVICE
    }
    fn notify_characteristic(&self) -> u16 {
        WEIGHT_CHARACTERISTIC
    }
    fn command_characteristic(&self) -> u16 {
        COMMAND_CHARACTERISTIC
    }
    fn capabilities(&self) -> Capabilities {
        dispatch!(self, p => p.capabilities())
    }
    fn decode(&self, packet: &[u8]) -> Result<Event, DecodeError> {
        dispatch!(self, p => p.decode(packet))
    }
    fn encode(&self, command: Command) -> Result<Frame, EncodeError> {
        dispatch!(self, p => p.encode(command))
    }
}

fn xor(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |a, b| a ^ b)
}

fn u24(b: &[u8]) -> u32 {
    u32::from_be_bytes([0, b[0], b[1], b[2]])
}

/// Sign byte + big-endian magnitude -> signed value.
fn signed(sign: u8, magnitude: u32) -> i32 {
    let m = magnitude as i32;
    if sign == MINUS {
        -m
    } else {
        m
    }
}

/// Validate length, header and checksum; returns `(type, packet)`.
fn check(packet: &[u8]) -> Result<(u8, &[u8; PACKET_LEN]), DecodeError> {
    let p: &[u8; PACKET_LEN] = packet.try_into().map_err(|_| DecodeError::Length(packet.len()))?;
    if p[0] != PRODUCT {
        return Err(DecodeError::Header(p[0]));
    }
    let expected = xor(&p[..PACKET_LEN - 1]);
    if expected != p[PACKET_LEN - 1] {
        return Err(DecodeError::Checksum { expected, got: p[PACKET_LEN - 1] });
    }
    Ok((p[1], p))
}

fn decode_measurement(p: &[u8; PACKET_LEN]) -> Measurement {
    Measurement {
        elapsed_ms: u24(&p[2..5]),
        unit: match p[5] {
            1 => WeightUnit::Gram,
            2 => WeightUnit::Ounce,
            u => WeightUnit::Unknown(u),
        },
        weight_cg: signed(p[6], u24(&p[7..10])),
        flow_cg_per_s: signed(p[10], u16::from_be_bytes([p[11], p[12]]) as u32),
        battery_percent: p[13],
        standby_deci_min: u16::from_be_bytes([p[14], p[15]]),
        beep_level: p[16],
        flow_smoothing: p[17] != 0,
    }
}

fn decode_auto_brew(p: &[u8; PACKET_LEN]) -> Result<AutoBrewEvent, DecodeError> {
    let state = match p[2] {
        0 => AutoBrewState::Stopped,
        1 => AutoBrewState::Started,
        2 => AutoBrewState::Ready,
        3 => AutoBrewState::ExitReady,
        4 => AutoBrewState::ExitDone,
        _ => return Err(DecodeError::UnknownType(TYPE_AUTO_BREW)),
    };
    Ok(AutoBrewEvent {
        state,
        elapsed_ms: u24(&p[3..6]),
        weight_cg: signed(p[6], u24(&p[7..10])),
        result_c: signed(p[10], u16::from_be_bytes([p[11], p[12]]) as u32),
    })
}

fn command_frame(d1: u8, d2: u8, d3: u8) -> Frame {
    let mut f = [PRODUCT, TYPE_COMMAND, d1, d2, d3, 0];
    f[5] = xor(&f[..5]);
    Frame::new(&f)
}

/// Encoding for commands both models accept. `max_beep` differs per model.
fn encode_common(command: Command, max_beep: u8) -> Result<Frame, EncodeError> {
    Ok(match command {
        Command::Tare => command_frame(0x01, 0, 0),
        Command::SetBeepLevel(l) if l <= max_beep => command_frame(0x02, 0, l),
        Command::SetAutoOff { minutes } if (5..=30).contains(&minutes) => command_frame(0x03, 0, minutes),
        Command::StartTimer => command_frame(0x04, 0, 0),
        Command::StopTimer => command_frame(0x05, 0, 0),
        Command::ResetTimer => command_frame(0x06, 0, 0),
        Command::TareAndStartTimer => command_frame(0x07, 0, 0),
        Command::SetFlowSmoothing(on) => command_frame(0x08, on as u8, 0),
        Command::SetBeepLevel(_) | Command::SetAutoOff { .. } => return Err(EncodeError::OutOfRange),
        // Ultra-only or future commands.
        _ => return Err(EncodeError::Unsupported),
    })
}
