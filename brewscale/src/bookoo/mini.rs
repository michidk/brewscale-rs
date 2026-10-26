use super::*;

/// BOOKOO Themis Mini.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mini;

impl Detect for Mini {
    fn detect(adv: &Advertisement) -> Option<Self> {
        (Model::identify(adv) == Some(Model::Mini)).then_some(Self)
    }
}

impl Protocol for Mini {
    fn name(&self) -> &'static str {
        "BOOKOO Themis Mini"
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
        Capabilities {
            timer: true,
            powder_weight: false,
            auto_brew_events: false,
            shutdown: false,
            keep_awake: false,
            max_beep_level: 5,
        }
    }

    fn decode(&self, packet: &[u8]) -> Result<Event, DecodeError> {
        match check(packet)? {
            (TYPE_MEASUREMENT, p) => Ok(Event::Measurement(decode_measurement(p))),
            (t, _) => Err(DecodeError::UnknownType(t)),
        }
    }

    fn encode(&self, command: Command) -> Result<Frame, EncodeError> {
        encode_common(command, 5)
    }
}
