# Contributing

## Checks (host)

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo doc --no-deps
```

The `psicose` crate must stay **zero dependencies**. Do not add crates to
`[dependencies]` or optional features that pull third-party code into the
library. HAL/board code belongs under `boards/`, not in `psicose`.

## Hardware examples

Board packages live under `boards/` in a **separate** Cargo workspace
(`boards/Cargo.toml`) so host CI can keep psicose MSRV **1.75**.

Shared pins (ESP32 classic, rustc ≥ **1.88**):

| Crate | Version |
| --- | --- |
| `esp-hal` | 1.1.2 |
| `esp-radio` | 0.18.0 |
| `esp-rtos` | 0.3.0 |
| `esp-alloc` | 0.10.0 |
| `esp-backtrace` | 0.19.0 |
| `esp-println` | 0.17.0 |
| `esp-bootloader-esp-idf` | 0.5.0 |

```sh
cd boards
cargo run -p esp32-uart
cargo run -p esp32-wifi
```

Boards are `publish = false` and are not part of the crates.io package.
