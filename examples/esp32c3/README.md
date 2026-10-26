# ESP32-C3 example

Firmware that scans for a supported scale (protocol-agnostic; BOOKOO Themis Mini/Ultra
with the `bookoo` feature), connects, subscribes to
weight notifications, logs each measurement, sends a tare command after the
first one, and reconnects when the link drops (including 10 s of silence).

Stack: `esp-hal` 1.x + `esp-rtos` (embassy) + `esp-radio` BLE controller →
`trouble-host` → `brewscale` with the `trouble` feature (protocols by feature). Logging: `log` over
`esp-println` (UART/USB-serial-JTAG), level from the `ESP_LOG` env var
(default `info`, set in `.cargo/config.toml`).

## Hardware

* An ESP32-C3 board with USB (e.g. ESP32-C3-DevKitM-1 or any C3 dev board).
* A BOOKOO Themis Mini or Ultra, powered on, not connected to a phone app
  (a scale accepts one BLE central at a time).

## Build and flash

```sh
rustup target add riscv32imc-unknown-none-elf   # stable toolchain, Rust 1.95+ (esp-rtos MSRV)
cargo install espflash                          # flasher + serial monitor

cd examples/esp32c3
cargo build --release                           # build only
cargo run --release                             # flash + monitor (runner = espflash flash --monitor)
```

Hold BOOT while pressing RESET if the board does not enter download mode.
Pass the serial port with `ESPFLASH_PORT=/dev/ttyACM0 cargo run --release` if
`espflash` cannot pick one.

## Expected output

```
our address: Address { kind: Random, addr: ... }
scanning for a scale...
found BOOKOO Themis Ultra, connecting
subscribed to weight notifications
0.00 g  flow 0.00 g/s  1234 ms  battery 87%
tare sent
```

Switch the scale off or walk out of range: the log shows `session ended:
disconnected`, then it rescans and reconnects.

## Notes

* The only heap use is `esp_alloc::heap_allocator!(72 KiB)` for the `esp-radio`
  controller driver. `brewscale-*` and `trouble-host` allocate nothing.
* Targets without atomic CAS (C3) get atomics through `esp-hal`, which enables `portable-atomic`'s `unsafe-assume-single-core`;
  a bare `cargo build -p brewscale --features trouble --target riscv32imc-unknown-none-elf`
  needs `RUSTFLAGS="--cfg portable_atomic_unsafe_assume_single_core"`.
* Other chips: change the `esp32c3` feature names in `Cargo.toml`, the target in
  `.cargo/config.toml`, and (for Xtensa chips) use the `esp` toolchain from `espup`.
