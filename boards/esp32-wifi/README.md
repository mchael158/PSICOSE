# PSICOSE on ESP32 (classic) — Wi‑Fi / TCP demo

Firmware example: reliable byte link over **TCP on Wi‑Fi** using the PSICOSE motor.

```text
Wi‑Fi STA → embassy-net TcpSocket → TcpPipe → LinkFace → Turn
```

- **`psicose`** has **zero** crate dependencies. API is only `psicose::…`.
- **`esp-radio` / Embassy / embassy-net`** are dependencies of **this board only** (`publish = false`).

Scan + STA connect + DHCP happen **outside** psicose. After TCP is up, the board
feeds bytes through a ring pipe into [`LinkFace`](https://docs.rs/psicose/latest/psicose/struct.LinkFace.html).

## Roles

| Side | `Turn` | How |
| --- | --- | --- |
| ESP32 | `FIRST = true` | connects to `HOST:19876`, opens with `ping` |
| PC | answers | `cargo run --example turn_pair -- --listen 0.0.0.0:19876` (`pong`) |

Do **not** open with `ping` on both ends. A full RX ring waits; bytes are not dropped.

## Build / flash

Needs [espup](https://esp-rs.github.io/book/) + `espflash`, same as `boards/esp32-uart`.

```powershell
# once per shell
. $HOME\export-esp.ps1

cd boards
$env:SSID = "my-ap"
$env:PASSWORD = "secret"
$env:HOST = "192.168.1.10"   # PC LAN IP
cargo run -p esp32-wifi
```

On the PC (same Wi‑Fi), start the listener **before** the board connects:

```sh
cargo run --example turn_pair -- --listen 0.0.0.0:19876
```

You should see `ping` on the PC and `pong` on the board (ACK/CRC through the motor).

## Host-only smoke (no board)

```sh
cargo run --example tcp_pair
```

Two localhost TCP peers exercise the same `ByteTransport` + `LinkFace` pattern.

## Versions

Pins come from [`boards/Cargo.toml`](../Cargo.toml) `[workspace.dependencies]`
(esp-hal **1.1.2**, esp-radio **0.18**, MSRV **1.88**). Do not bump to
esp-hal 1.2 / esp-radio 1.0-beta unless the toolchain is rustc **1.95+**.

```sh
cd boards
cargo run -p esp32-wifi
```

If `cargo check` fails after a crates.io bump, align with the current
[esp-hal `embassy_dhcp`](https://github.com/esp-rs/esp-hal/tree/main/examples/wifi/embassy_dhcp) example.
