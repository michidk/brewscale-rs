//! Connect to any supported scale (protocol-agnostic), log weight measurements,
//! tare once, and reconnect whenever the link drops. ESP32-C3, TrouBLE host + esp-radio controller.
#![no_std]
#![no_main]

use brewscale::{find, AnyProtocol, Error, Event, Link, Protocol, Scale, Transport, TransportError};
use brewscale::trouble::{ScanInbox, TroubleTransport};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use log::{info, warn};
use trouble_host::prelude::*;

esp_bootloader_esp_idf::esp_app_desc!();

/// A scale streams ~10 packets/s; silence this long means a dead link.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(10);

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // Only the esp-radio controller driver allocates; brewscale and TrouBLE do not.
    esp_alloc::heap_allocator!(size: 72 * 1024);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);

    // Static random address derived from the chip's factory MAC (top two bits set).
    let mut addr: [u8; 6] = esp_hal::efuse::base_mac_address().as_bytes().try_into().unwrap();
    addr.reverse();
    addr[5] |= 0xC0;
    let address = Address::random(addr);
    info!("our address: {:?}", address);

    // 1 connection; L2CAP channels: signalling + ATT + 1 spare.
    let mut resources: HostResources<DefaultPacketPool, 1, 3> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address).build();
    let mut runner = stack.runner();
    let inbox = ScanInbox::<4>::new();
    let mut transport: TroubleTransport<_, _, 4, 4> = TroubleTransport::new(&stack, &inbox);

    let app = async {
        loop {
            match session(&mut transport).await {
                Ok(()) => {}
                Err(e) => warn!("session ended: {e}"),
            }
            Timer::after(Duration::from_secs(2)).await;
        }
    };
    // The host runner must run alongside the app; it also feeds scan results to `inbox`.
    let (host, ()) = join(runner.run_with_handler(&inbox), app).await;
    panic!("BLE host stopped: {:?}", host.err());
}

/// One find -> connect -> stream cycle. Returns when the link is lost.
async fn session<T: Transport>(transport: &mut T) -> Result<(), Error<T::Error>> {
    info!("scanning for a scale...");
    let (peer, protocol) = find::<AnyProtocol, _>(transport).await?;
    info!("found {}, connecting", protocol.name());
    let link = transport.connect(&peer).await?;
    let result = stream(&link, protocol).await;
    let _ = link.disconnect().await;
    result
}

async fn stream<L: Link>(link: &L, protocol: AnyProtocol) -> Result<(), Error<L::Error>>
where
    L::Error: TransportError,
{
    let mut scale = Scale::attach(link, protocol).await?;
    info!("subscribed to weight notifications");
    let mut tared = false;
    loop {
        match select(scale.next_event(), Timer::after(SILENCE_TIMEOUT)).await {
            Either::First(Ok(Event::Measurement(m))) => {
                info!(
                    "{:.2} g  flow {:.2} g/s  {} ms  battery {}%",
                    m.weight_g(),
                    m.flow_g_per_s(),
                    m.elapsed_ms,
                    m.battery_percent
                );
                if !tared {
                    scale.tare().await?;
                    tared = true;
                    info!("tare sent");
                }
            }
            Either::First(Ok(other)) => info!("event: {other:?}"),
            // A corrupt packet is not fatal; keep streaming.
            Either::First(Err(Error::Decode(e))) => warn!("bad packet: {e}"),
            Either::First(Err(e)) => return Err(e),
            Either::Second(()) => return Err(Error::Disconnected),
        }
    }
}
