# PSICOSE on ESP32 (classic) — UART demo

Firmware example: reliable byte link over **UART1** using the PSICOSE motor.

```text
UART1 → ByteTransport (board-local) → LinkFace → WindowTurn
```

- **`psicose`** has **zero** crate dependencies. Its API is only `psicose::…`.
- **`esp-hal`** is a dependency of **this board package only** (`publish = false`).

## Hardware

| Signal | GPIO | Notes |
| --- | --- | --- |
| UART1 TX | **17** | |
| UART1 RX | **16** | |
| Baud | 115200 | 8N1 |

### Two boards

Flash **opposite** [`FIRST`](src/main.rs) values. Both send one 8-byte sample
at a time, DATA window of 8:

| Board | `FIRST` | Wiring |
| --- | --- | --- |
| A | `true` | GPIO17 → B GPIO16 |
| B | `false` | GPIO17 → A GPIO16 |
| both | | common GND |

If both are `true`, both open `START` at once and the wire corrupts.

## Toolchain

Stock Rust **cannot** target ESP32 classic (Xtensa). Install espup once:

```powershell
cargo +stable install espup --locked
espup install
. $HOME\export-esp.ps1
```

Then:

```powershell
cd boards
. $HOME\export-esp.ps1
cargo run -p esp32-uart
```

Needs [espup](https://esp-rs.github.io/book/) + `espflash`. Dependency pins
live in [`boards/Cargo.toml`](../Cargo.toml).
