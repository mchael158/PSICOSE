# PSICOSE-1B

[![crates.io](https://img.shields.io/crates/v/psicose.svg)](https://crates.io/crates/psicose)
[![docs.rs](https://docs.rs/psicose/badge.svg)](https://docs.rs/psicose)
[![CI](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml/badge.svg)](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/psicose.svg)](https://crates.io/crates/psicose)

[English](README.md) · [Português (Brasil)](README.pt-BR.md)

A `no_std`, heapless, deterministic, byte-oriented transport protocol.
**Payload is 1 byte. The wire frame is 4 bytes.** Those are not the same
thing.

```text
┌──────┬─────┬─────┬───────┐
│ TYPE │ SEQ │ DATA│  CRC  │   ← frame = 4 bytes (overhead + payload)
└──────┴─────┴─────┴───────┘
                   ▲
                   └── application payload: exactly 8 bits
```

Best-case efficiency before ACKs: 1/4 = 25%. Stop-and-wait (DATA+ACK):
1/8 = 12.5%.

```toml
[dependencies]
psicose = "0.3"
# Optional authenticated encryption above the transport:
# psicose = { version = "0.3", features = ["aead"] }
```

Default build: **no features, no dependencies.** Optional feature `aead`
adds ChaCha20-Poly1305 (RFC 8439). MSRV: Rust 1.75 (`rust-toolchain.toml`).

Docs: [docs.rs/psicose](https://docs.rs/psicose) · spec: [`docs/PROTOCOL.md`](docs/PROTOCOL.md)
([pt-BR](docs/PROTOCOL.pt-BR.md))

## Why

Most transfer protocols hold the whole message in memory. PSICOSE never
does: it moves one payload byte at a time. Memory is a few frames on the
stack, whether you send 4 bytes of config or a 4 GB file.

## Status (0.3 — usable transport)

Reliable byte transport for embedded links. CRC-8 is **noise detection**,
not authentication — enable `aead` when the link may be adversarial.

- **protocol** — CRC-8, 4-byte frame, semantic validation, `SEQ`
  wraparound (`255 → 0`), assembler, `OutBuf` (one wire byte per poll).
- **tx / rx** — stop-and-wait. `START` / `FINISH` / `ABORT` wait for ACK.
- **pump** — cooperative `Pump` (`rx.poll` then `tx.poll`, no inner
  loop) + `SessionStats` (`bytes_delivered`, `frames_sent`, `retries`,
  `nacks`, `duplicates`, `crc_errors`, `ticks`).
- **window** — selective-repeat `WindowedSender<_, N>` /
  `WindowedReceiver<_, N>`, `1 ≤ N ≤ 8`. Aliases: `W8Sender`, `W8Receiver`.
- **transport** — `ByteTransport` / `ByteSource` / `ByteSink`.
- **stream** — any bytes through the envelope: `SliceSource` /
  `SliceSink`, `send_all` / `recv_all` (stop-and-wait and windowed).
  JPEG, a file, flash, a sensor, or `struct` bytes are all just a
  `ByteSource`. The frame is not the application type.
- **fault** — `FaultyTransport` for adversarial tests.
- **actors** — cooperative `System` for N links on one thread.
- **p2p** — same crate, same budget. `use psicose::prelude::*`. See below.

Not yet: routing / gossip / store-and-forward, UART/SPI/CAN/radio,
`File`.

## Layers

```text
 application bytes
        │
        ├─ optional: aead::seal_to / open_from   (feature = "aead")
        ├─ optional: Fragmenter / Defragmenter
        ▼
 PeerLink / Pump / Sender|Receiver|Windowed*
        │  DATA 1 byte/frame + CRC-8 + ACK
        ▼
 ByteTransport  (you implement: UART / SPI / radio)
```

Compose the layers yourself. Capabilities bits announce intent; they do
not auto-switch `PeerLink` to windowed or AEAD.

## API

```rust
use psicose::prelude::*;
```

| You write | Meaning |
| --- | --- |
| `PeerId::from_label(b"alice")` | 8-byte identity (padded). Never in the 4-byte frame. |
| `PeerTable::<4>::new(id)` | Neighbors, `1 ≤ N ≤ 8`. Default: window 8 + `STREAM`. |
| `PeerTable::with(id, SessionConfig::FORUM)` | Hello: `STREAM` + `WINDOW` + `FORUM` + `FRAGMENTATION`. |
| `SessionConfig::SECURE` | `FORUM` + `ENCRYPTION` (announce AEAD; still call `seal_to` yourself). |
| `StreamId::FORUM` / `MessageId::new(1)` | Application conversation. Not `SEQ`. |
| `Fragmenter` / `Defragmenter` | Cut any blob; rebuild into a stack buffer. |
| `seal_to` / `open_from` | Feature `aead`: ciphertext ‖ tag above the transport. |
| `SessionConfig::offer(4, features)` | Hello config. Version is `PROTOCOL_VERSION`. |
| `Capabilities::*` | Announcements only. CRC is not negotiated. |
| `Wire::new().pumps()` | In-memory A↔B. ACK/NACK → TX, DATA/START/FINISH/ABORT → RX. |
| `Pump::on(tx, rx)` | Same wrapping on a UART / SPI / radio. |
| `PeerLink::connect` / `accept` | START + 12-byte hello both ways. |
| `link.offer(byte)` | DATA after `Established`. |
| `link.poll(&mut table)` | One cooperative step. Never loops. |
| `PollOutcome::is_closed()` | FINISH or ABORT ended the transfer. |

Wire hello is 12 payload bytes: `PeerId (8) | ver (1) | window (1) | features (2)`.
`PeerSession` is bookkeeping only (≤ 128 B). `StreamId` / `MessageId` are
not `SEQ`.

```rust
use psicose::prelude::*;

let wire = Wire::new();
let (pump_a, pump_b) = wire.pumps();

let mut alice = PeerTable::<4>::new(PeerId::from_label(b"alice"));
let mut bob = PeerTable::<4>::new(PeerId::from_label(b"bob"));

let mut a = match PeerLink::connect(&mut alice, pump_a) {
    Ok(link) => link,
    Err(_) => return,
};
let mut b = PeerLink::accept(&bob, pump_b);
let _ = (a.poll(&mut alice), b.poll(&mut bob));
```

The same link moves bytes A→B. This is not a forum — only payload:

```rust
use psicose::prelude::*;

let cfg = SessionConfig::FORUM;
let wire = Wire::new();
let (pump_a, pump_b) = wire.pumps();

let mut alice = PeerTable::<4>::with(PeerId::from_label(b"alice"), cfg);
let mut bob = PeerTable::<4>::with(PeerId::from_label(b"bob"), cfg);

let mut a = match PeerLink::connect(&mut alice, pump_a) {
    Ok(link) => link,
    Err(_) => return,
};
let mut b = PeerLink::accept(&bob, pump_b);
let _ = (a.poll(&mut alice), b.poll(&mut bob));

let ping = b"ping";
let mut frag = Fragmenter::new(StreamId::FORUM, MessageId::new(1), ping, 4);
let mut board = [0u8; 32];
let mut inbox = Defragmenter::new(&mut board);
```

Runnable: `cargo run --example forum`. Test: `tests/forum.rs`.

Override a custom offer with
`PeerTable::with(id, SessionConfig::offer(4, Capabilities::STREAM | Capabilities::WINDOW))`.

The 4-byte envelope is still just this:

```rust
use psicose::Frame;

let frame = Frame::data(0, b'A');
assert_eq!(Frame::from_bytes(frame.to_bytes()), Ok(frame));
```

## Non-negotiables

- `#![no_std]`, `#![forbid(unsafe_code)]`
- No `Vec`, `String`, `Box`, `Rc`/`Arc`, no allocator, no async runtime
- Every public type has a size known at compile time
- Reliability is the protocol's job, never the application's

## Any data, not just frames

The 4-byte frame is the **envelope**. Application data is always bytes:

```text
JPEG  File  Flash  Sensor  firmware.bin  your struct
  └───────┴───────┴────────┴──────────────┘
                    │
               ByteSource     ← you implement this
                    │ 1 byte at a time
                    ▼
                 PSICOSE      ← never owns the blob
                    │
                ByteSink
```

`psicose::File` is not in this crate. A JPEG and a 4 GB file use the
same path: implement `ByteSource` / `ByteSink` (or use `SliceSource` /
`SliceSink` when the caller already holds the buffer).

## Examples

Runnable stand-ins for problems this crate is meant to solve. The "UART"
and "radio" are in-memory rings; swap `End` for your driver.

```sh
cargo run --example jpeg_over_uart
cargo run --example firmware_flash
cargo run --example sensor_telemetry
cargo run --example radio_windowed
cargo run --example forum
```

| Example | Real problem | What PSICOSE sees |
| --- | --- | --- |
| `jpeg_over_uart` | OV2640-class camera RAM → host file over UART | JPEG bytes |
| `firmware_flash` | Host `firmware.bin` → MCU NOR flash, 1 byte programmed at a time | file bytes |
| `sensor_telemetry` | DHT22 + battery ADC packed struct over a slow radio | 8-byte sample |
| `radio_windowed` | 300-byte EEPROM dump; first DATA frame dropped | bytes, window `N=8` |
| `forum` | Alice ↔ Bob: handshake, then bytes both ways | payload bytes |

`firmware_flash` implements `ByteSource` on `std::fs::File` — that is the
pattern for a 4 GB image. The crate still never owns the file.

## Tests

```sh
cargo test
```

See `tests/end_to_end.rs`, `tests/hostile.rs`, `tests/windowed.rs`,
`tests/pump.rs`, `tests/p2p.rs`, and `tests/forum.rs`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
