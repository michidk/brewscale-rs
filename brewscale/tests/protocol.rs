use brewscale::bookoo::{Bookoo, Mini, Model, Ultra};
use brewscale::*;

fn packet(ty: u8, data: [u8; 17]) -> [u8; 20] {
    let mut p = [0u8; 20];
    p[0] = 0x03;
    p[1] = ty;
    p[2..19].copy_from_slice(&data);
    p[19] = p[..19].iter().fold(0, |a, b| a ^ b);
    p
}

/// 66051 ms, grams, +18.04 g, -2.50 g/s, 87 %, 60.0 min, beep 2, smoothing on.
fn measurement() -> [u8; 20] {
    packet(0x0B, [0x01, 0x02, 0x03, 1, b'+', 0x00, 0x07, 0x0C, b'-', 0x00, 0xFA, 87, 0x02, 0x58, 2, 1, 0])
}

fn expect_measurement(e: Event) -> Measurement {
    match e {
        Event::Measurement(m) => m,
        other => panic!("{other:?}"),
    }
}

#[test]
fn decodes_measurement_on_both_models() {
    let mini = expect_measurement(Mini.decode(&measurement()).unwrap());
    let ultra = expect_measurement(Ultra.decode(&measurement()).unwrap());
    assert_eq!(mini, ultra);
    assert_eq!(
        mini,
        Measurement {
            elapsed_ms: 66051,
            weight_cg: 1804,
            flow_cg_per_s: -250,
            unit: WeightUnit::Gram,
            battery_percent: 87,
            standby_deci_min: 600,
            beep_level: 2,
            flow_smoothing: true,
        }
    );
    assert_eq!(mini.weight_g(), 18.04);
    assert_eq!(mini.flow_g_per_s(), -2.5);
}

#[test]
fn negative_weight_and_ounce_unit() {
    let p = packet(0x0B, [0, 0, 0, 2, b'-', 0, 0x00, 0x64, b'+', 0, 0, 100, 0, 0, 0, 0, 0]);
    let m = expect_measurement(Ultra.decode(&p).unwrap());
    assert_eq!((m.weight_cg, m.unit), (-100, WeightUnit::Ounce));
}

#[test]
fn rejects_corrupt_packets() {
    let good = measurement();
    let mut bad = good;
    bad[8] ^= 1;
    let expected = bad[..19].iter().fold(0, |a, b| a ^ b);
    assert_eq!(Mini.decode(&bad), Err(DecodeError::Checksum { expected, got: good[19] }));
    assert_eq!(Mini.decode(&good[..19]), Err(DecodeError::Length(19)));
    assert_eq!(Mini.decode(&[]), Err(DecodeError::Length(0)));
    let mut hdr = good;
    hdr[0] = 0x04;
    assert_eq!(Mini.decode(&hdr), Err(DecodeError::Header(4)));
}

#[test]
fn ultra_only_packets() {
    let powder = packet(0x0F, [b'+', 0x00, 0x4E, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(Ultra.decode(&powder), Ok(Event::PowderWeight { weight_cg: 20000 }));
    assert_eq!(Mini.decode(&powder), Err(DecodeError::UnknownType(0x0F)));

    let brew = packet(0x0D, [1, 0, 0x03, 0xE8, b'+', 0x00, 0x03, 0xE8, b'+', 0x08, 0xFC, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        Ultra.decode(&brew),
        Ok(Event::AutoBrew(AutoBrewEvent {
            state: AutoBrewState::Started,
            elapsed_ms: 1000,
            weight_cg: 1000,
            result_c: 2300,
        }))
    );
    assert_eq!(Mini.decode(&brew), Err(DecodeError::UnknownType(0x0D)));
    let bad_state = packet(0x0D, [9, 0, 0, 0, b'+', 0, 0, 0, b'+', 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(Ultra.decode(&bad_state).is_err());
}

#[test]
fn encodes_documented_command_frames() {
    // Checksum = XOR of all preceding bytes (vendor spec). Tare and beep also
    // match the AcaiaArduinoBLE ESP32 firmware byte for byte; its start/stop/
    // reset constants do NOT follow the XOR rule, so they are not used here.
    let cases: [(Command, [u8; 6]); 6] = [
        (Command::Tare, [0x03, 0x0A, 0x01, 0x00, 0x00, 0x08]),
        (Command::StartTimer, [0x03, 0x0A, 0x04, 0x00, 0x00, 0x0D]),
        (Command::StopTimer, [0x03, 0x0A, 0x05, 0x00, 0x00, 0x0C]),
        (Command::ResetTimer, [0x03, 0x0A, 0x06, 0x00, 0x00, 0x0F]),
        (Command::SetBeepLevel(1), [0x03, 0x0A, 0x02, 0x00, 0x01, 0x0A]),
        (Command::TareAndStartTimer, [0x03, 0x0A, 0x07, 0x00, 0x00, 0x0E]),
    ];
    for (cmd, bytes) in cases {
        assert_eq!(&*Mini.encode(cmd).unwrap(), &bytes, "{cmd:?}");
        assert_eq!(&*Ultra.encode(cmd).unwrap(), &bytes, "{cmd:?}");
    }
    assert_eq!(&*Mini.encode(Command::SetFlowSmoothing(true)).unwrap(), &[3, 0x0A, 8, 1, 0, 3 ^ 0x0A ^ 8 ^ 1]);
    assert_eq!(&*Ultra.encode(Command::SetAutoOff { minutes: 30 }).unwrap(), &[3, 0x0A, 3, 0, 30, 3 ^ 0x0A ^ 3 ^ 30]);
    assert_eq!(
        &*Ultra.encode(Command::SetPowderWeight { deci_g: 0x0123 }).unwrap(),
        &[3, 0x0A, 0x0D, 1, 0x23, 3 ^ 0x0A ^ 0x0D ^ 1 ^ 0x23]
    );
    assert_eq!(&*Ultra.encode(Command::Shutdown).unwrap(), &[3, 0x0A, 0x15, 0, 0, 3 ^ 0x0A ^ 0x15]);
    assert_eq!(&*Ultra.encode(Command::KeepAwake).unwrap(), &[3, 0x0A, 0x25, 0, 0, 3 ^ 0x0A ^ 0x25]);
}

#[test]
fn command_validation_and_capabilities() {
    assert_eq!(Mini.encode(Command::SetBeepLevel(6)), Err(EncodeError::OutOfRange));
    assert!(Mini.encode(Command::SetBeepLevel(5)).is_ok());
    assert_eq!(Ultra.encode(Command::SetBeepLevel(4)), Err(EncodeError::OutOfRange));
    assert_eq!(Ultra.encode(Command::SetAutoOff { minutes: 4 }), Err(EncodeError::OutOfRange));
    assert_eq!(Ultra.encode(Command::SetAutoOff { minutes: 31 }), Err(EncodeError::OutOfRange));
    assert_eq!(Ultra.encode(Command::SetPowderWeight { deci_g: 0 }), Err(EncodeError::OutOfRange));
    assert_eq!(Ultra.encode(Command::SetPowderWeight { deci_g: 10000 }), Err(EncodeError::OutOfRange));
    for c in [Command::Shutdown, Command::KeepAwake, Command::SetPowderWeight { deci_g: 10 }] {
        assert_eq!(Mini.encode(c), Err(EncodeError::Unsupported));
    }
    assert!(!Mini.capabilities().powder_weight && Ultra.capabilities().powder_weight);
    assert_eq!(Bookoo(Model::Ultra).capabilities(), Ultra.capabilities());
    assert_eq!(Bookoo(Model::Mini).encode(Command::Shutdown), Err(EncodeError::Unsupported));
}

fn adv(name: &str, services: &[u16]) -> Advertisement {
    let mut a = Advertisement::empty();
    a.set_name(name.as_bytes());
    services.iter().for_each(|s| a.add_service(*s));
    a
}

#[test]
fn identifies_models() {
    assert_eq!(Model::identify(&adv("BOOKOO_SC 1234", &[])), Some(Model::Mini));
    assert_eq!(Model::identify(&adv("BOOKOO_SC U 1234", &[])), Some(Model::Ultra));
    assert_eq!(Model::identify(&adv("OOKOO_SC 1234", &[])), Some(Model::Mini), "firmware 1.4.1 truncation");
    assert_eq!(Model::identify(&adv("OOKOO_SC U 1234", &[])), Some(Model::Ultra));
    assert_eq!(Model::identify(&adv("", &[0x0FFE])), Some(Model::Mini));
    assert_eq!(Model::identify(&adv("ACAIA", &[0x180F])), None);
    assert_eq!(Model::identify(&Advertisement::empty()), None);
}

#[test]
fn parses_raw_advertising_data() {
    // flags, complete 16-bit services [0x0FFE, 0x180F], complete name
    let data = [
        2, 0x01, 0x06, 5, 0x03, 0xFE, 0x0F, 0x0F, 0x18, 15, 0x09, b'B', b'O', b'O', b'K', b'O', b'O', b'_', b'S', b'C',
        b' ', b'U', b' ', b'0', b'1',
    ];
    let a = Advertisement::from_ad_bytes(Some(-60), &data);
    assert_eq!(a.name(), b"BOOKOO_SC U 01");
    assert!(a.has_service(0x0FFE) && a.has_service(0x180F) && !a.has_service(1));
    assert_eq!(a.rssi, Some(-60));
    // truncated record and zero length must not panic or invent data
    assert_eq!(Advertisement::from_ad_bytes(None, &[9, 0x09, b'x']).name(), b"");
    assert_eq!(Advertisement::from_ad_bytes(None, &[0, 0x09]).name(), b"");
    assert_eq!(Advertisement::from_ad_bytes(None, &[]).name(), b"");
}
