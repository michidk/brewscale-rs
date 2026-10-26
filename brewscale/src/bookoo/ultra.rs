use super::*;

/// BOOKOO Themis Ultra. Command availability also depends on firmware
/// version (see the vendor spec); this driver cannot know the version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ultra;

impl Detect for Ultra {
    fn detect(adv: &Advertisement) -> Option<Self> {
        (Model::identify(adv) == Some(Model::Ultra)).then_some(Self)
    }
}

impl Protocol for Ultra {
    fn name(&self) -> &'static str {
        "BOOKOO Themis Ultra"
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
            powder_weight: true,
            auto_brew_events: true,
            shutdown: true,
            keep_awake: true,
            max_beep_level: 3,
        }
    }

    fn decode(&self, packet: &[u8]) -> Result<Event, DecodeError> {
        match check(packet)? {
            (TYPE_MEASUREMENT, p) => Ok(Event::Measurement(decode_measurement(p))),
            (TYPE_POWDER, p) => Ok(Event::PowderWeight { weight_cg: signed(p[2], u24(&p[3..6])) }),
            (TYPE_AUTO_BREW, p) => decode_auto_brew(p).map(Event::AutoBrew),
            (t, _) => Err(DecodeError::UnknownType(t)),
        }
    }

    fn encode(&self, command: Command) -> Result<Frame, EncodeError> {
        match command {
            Command::SetPowderWeight { deci_g } if (1..=9999).contains(&deci_g) => {
                let [hi, lo] = deci_g.to_be_bytes();
                Ok(command_frame(0x0D, hi, lo))
            }
            Command::SetPowderWeight { .. } => Err(EncodeError::OutOfRange),
            Command::Shutdown => Ok(command_frame(0x15, 0, 0)),
            Command::KeepAwake => Ok(command_frame(0x25, 0, 0)),
            c => encode_common(c, 3),
        }
    }
}
