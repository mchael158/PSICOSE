# PSICOSE on hardware

[English](HARDWARE.md) · [Português (Brasil)](HARDWARE.pt-BR.md)

PSICOSE is a **reliable link motor** with **zero crate dependencies**.
On a board you own the HAL; PSICOSE owns frames, ACK/CRC, windowing, and P2P.

## Official paths (ESP32 classic)

### UART

```text
application
     │
     ▼
Pump / Node / PeerLink
     │
     ▼
LinkFace          ← demux one RX stream into Pump TX + RX ends
     │
     ▼
ByteTransport     ← you implement (board-local wrapper around UART)
     │
     ▼
esp-hal UART1     ← boards/esp32-uart only — not in psicose
```

### Wi‑Fi / TCP

```text
application
     │
     ▼
Pump / Node / PeerLink
     │
     ▼
LinkFace
     │
     ▼
ByteTransport     ← board-local TcpPipe (rings) or TcpStream wrapper
     │
     ▼
embassy-net TCP ← Wi‑Fi STA / DHCP / scan live in boards/esp32-wifi
```

| Piece | Where |
| --- | --- |
| `LinkFace`, `Pump`, `Node`, … | Always in `psicose` (zero deps) |
| `ByteTransport` impl | Your firmware / board package |
| `esp-hal` UART | [`boards/esp32-uart`](../boards/esp32-uart/) |
| `esp-radio` + Embassy TCP | [`boards/esp32-wifi`](../boards/esp32-wifi/) |

Host smoke without a board: `cargo run --example tcp_pair`.

In-memory A↔B tests use [`Wire`](https://docs.rs/psicose/latest/psicose/type.Wire.html).

## Pinout (UART example firmware)

| Signal | GPIO |
| --- | --- |
| UART1 TX | 17 |
| UART1 RX | 16 |
| Baud | 115200 |

See [`boards/esp32-uart/README.md`](../boards/esp32-uart/README.md) and
[`boards/esp32-wifi/README.md`](../boards/esp32-wifi/README.md).

## Minimal sketch

```rust,ignore
use psicose::{ByteTransport, LinkFace};

struct MyUart(/* your HAL type */);
impl ByteTransport for MyUart { /* write_byte / read_byte */ }

let face = LinkFace::new(MyUart(/* … */));
let mut pump = face.pump();
loop { let _ = pump.poll(); }
```

Do **not** flash `INITIATOR = true` on both ends of a two-board link.
Set one board to `false` (receive-only). See the board README.

## Out of scope here

BLE, NVS, OTA, other chips — separate board packages.
Crypto is also out of psicose: seal in the application if you need it.
