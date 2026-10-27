# brewscale

[![Crates.io](https://img.shields.io/crates/v/brewscale.svg)](https://crates.io/crates/brewscale) [![Documentation](https://docs.rs/brewscale/badge.svg)](https://docs.rs/brewscale)

Protocol-agnostic BLE coffee-scale library for Rust, on desktop **and**
microcontrollers. Write your application once against one API; add scale
protocols and Bluetooth backends without changing application code.

The browser client is [brewscale-web](https://github.com/michidk/brewscale-web),
a TypeScript implementation of the same protocol and transport boundaries.
Keep the two projects aligned when adding shared scale concepts or supported
protocols.

## Supported models

The current built-in protocol support is **BOOKOO Themis Mini** and **Themis
Ultra**, using the vendor's [open protocol docs](https://github.com/BooKooCode/OpenSource).

## Crate

[`brewscale`](https://crates.io/crates/brewscale) is the only published crate. It contains the vendor-neutral core,
BOOKOO protocol support, the optional desktop `btleplug` backend, and the
optional embedded TrouBLE backend. Most applications only need this one
dependency:

```toml
# Desktop: BOOKOO protocol + btleplug backend
brewscale = { version = "0.1", features = ["btleplug"] }

# Embedded: BOOKOO protocol + TrouBLE backend
brewscale = { version = "0.1", default-features = false, features = ["bookoo", "trouble"] }
```

The `brewscale-core`, `brewscale-bookoo`, and `brewscale-trouble` source
modules are internal implementation modules inside `brewscale`; they are not
separate published packages.

## Protocol-agnostic applications

`AnyProtocol` is generated from the enabled protocol features. The application
never names a vendor; enabling another protocol feature makes its scales work
with no code change.

```rust
use brewscale::*;

let (peer, protocol) = find::<AnyProtocol, _>(&mut transport).await?; // first scale of any enabled protocol
println!("found {}", protocol.name());
let link = transport.connect(&peer).await?;
let mut scale: AnyScale<_> = Scale::attach(&link, protocol).await?;   // discover + subscribe

scale.tare().await?;
loop {
    match scale.next_event().await? {
        Event::Measurement(m) => println!("{:.2} g", m.weight_g()),
        _ => {} // protocol-specific events; Event is #[non_exhaustive]
    }
}
```

`next_event` is cancel-safe: `select` it against a timer or a command channel.
`Error::Disconnected` means the link dropped; start over from `find` to
reconnect. `protocol.capabilities()` tells you what the connected scale
supports (timer, dose weight, ...); unsupported commands return
`EncodeError::Unsupported`.

Full programs: [`brewscale/examples/desktop.rs`](https://github.com/michidk/brewscale/blob/main/brewscale/examples/desktop.rs)
and [`examples/esp32c3/src/bin/scale.rs`](https://github.com/michidk/brewscale/blob/main/examples/esp32c3/src/bin/scale.rs).

## Design

* **Protocols are pure.** A protocol implements `Protocol` (GATT layout,
  `decode`, `encode`, `capabilities`) and `Detect` (recognise its devices from
  an advertisement). No I/O, no allocation, no BLE stack.
* **Transport is a trait.** `Transport` (scan, connect), `Link` (discover,
  read, write, subscribe, disconnect) and `Notifications` use native async fn
  in traits: no boxed futures, no runtime dependency. Backends implement them.
* **One driver.** `Scale` is generic over protocol and link, so protocol tests
  cover every backend and every backend works with every protocol.
* **Stable surface.** `Event`, `Command` and the error enums are
  `#[non_exhaustive]`, so new protocols can add variants without breaking you.

## Adding a protocol

1. Add a protocol module inside `brewscale` and keep it `no_std`.
2. Implement `Protocol` and `Detect` for your type. Reuse the shared `Event`
   and `Command` types; add a variant to them in core if your device needs one.
3. In `brewscale/Cargo.toml` add an optional dependency and a feature
   (`<vendor> = ["dep:brewscale-<vendor>"]`, plus the `std` forwarding line).
4. Add one line to the `protocols!` list in `brewscale/src/any.rs`.
5. Keep the single `brewscale/Cargo.toml` manifest in the release workflows.

[`brewscale/src/core/scale.rs`](https://github.com/michidk/brewscale/blob/main/brewscale/src/core/scale.rs) contains a ten-line
toy protocol test showing the minimum.

## Adding a backend

Implement `Transport`, `Link` and `Notifications` from the core module.
The `trouble` module and [`brewscale/src/btleplug_transport.rs`](https://github.com/michidk/brewscale/blob/main/brewscale/src/btleplug_transport.rs) are the reference backends. `Notifications::next` must be cancel-safe.

## ESP32

See [`examples/esp32c3`](https://github.com/michidk/brewscale/blob/main/examples/esp32c3/README.md) for hardware, flashing and
expected output. Target: **ESP32-C3** (stable Rust, `riscv32imc-unknown-none-elf`).
ESP32-S3 needs the Xtensa toolchain (`espup`) and is not built here, but
The `trouble` feature is chip-independent: any `trouble-host` controller works.

## Features

* `brewscale`: `bookoo` (default), `std`, `mock`, `btleplug` (implies `std`), and `trouble`.

## Testing

```sh
cargo test --workspace                                    # parsers + driver over a mock transport
cargo build -p brewscale --no-default-features --target thumbv7em-none-eabihf   # no std, no alloc
(cd examples/esp32c3 && cargo build --release)            # real firmware
```

CI runs these plus the desktop backend on Linux, Windows and macOS.

## License

MIT
