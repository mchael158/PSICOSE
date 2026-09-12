# PSICOSE-1B

[![crates.io](https://img.shields.io/crates/v/psicose.svg)](https://crates.io/crates/psicose)
[![docs.rs](https://docs.rs/psicose/badge.svg)](https://docs.rs/psicose/latest/psicose/)
[![CI](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml/badge.svg)](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml)
[![no_std](https://img.shields.io/badge/no__std-yes-brightgreen.svg)](https://docs.rs/psicose)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/mchael158/PSICOSE)
[![license](https://img.shields.io/crates/l/psicose.svg)](https://crates.io/crates/psicose)

[English](README.md) · [Português (Brasil)](README.pt-BR.md)

**`no_std` framework for tiny reliable links** — wire + pump + window + **P2P**,
constant stack RAM, zero default deps.

Payload is **1 byte**. Wire frame is **4 bytes**. P2P (`Node`, `PeerLink`, hello,
fragmentation) is **in this crate**, on the same frame — not a bolt-on package.
Examples call into this motor; they do not invent an external P2P and pass
connections in.

```text
┌──────┬─────┬─────┬───────┐
│ TYPE │ SEQ │ DATA│  CRC  │   ← frame = 4 bytes
└──────┴─────┴─────┴───────┘
                   ▲
                   └── application payload: exactly 8 bits
```

Best-case efficiency before ACKs: 1/4 = 25%. Stop-and-wait (DATA+ACK): 1/8 = 12.5%.

```toml
[dependencies]
psicose = "0.3"
# Optional authenticated encryption above the transport:
# psicose = { version = "0.3", features = ["aead"] }
# UART/SPI via embedded-io 0.6:
# psicose = { version = "0.3", features = ["embedded-io"] }
```

| | |
| --- | --- |
| Default build | **framework core (incl. P2P), zero dependencies** |
| Optional | `aead`, `embedded-io` (0.6 — MSRV 1.75) |
| MSRV | Rust 1.75 |
| Safety | `#![no_std]` · `#![forbid(unsafe_code)]` · no heap |
| Docs | [docs.rs/psicose](https://docs.rs/psicose/latest/psicose/) |
| Spec | [`docs/PROTOCOL.md`](docs/PROTOCOL.md) · [pt-BR](docs/PROTOCOL.pt-BR.md) |
| API map | [`docs/API.md`](docs/API.md) · [pt-BR](docs/API.pt-BR.md) |

## Try in 30 seconds

```sh
cargo run --example ab_direct    # motor: Wire::copy
cargo run --example p2p_pair     # motor: Node + PeerLink
cargo run --example jpeg_over_uart
cargo test
```

## When to use

- UART / SPI / radio where RAM is scarce and you cannot buffer the whole message
- Firmware flash, camera JPEG, sensor samples, EEPROM dumps — anything as bytes
- You want stop-and-wait or selective-repeat (`N ≤ 8`) with CRC + retry on the stack
- Optional ChaCha20-Poly1305 (`aead`) when the link may be adversarial

## When not to use

- You need high throughput / large frames (by design: 1 DATA byte per frame)
- You need routing, mesh, gossip, or store-and-forward (not in this crate)
- You only need a raw UART driver — implement `ByteTransport`; PSICOSE sits above it

## Why it exists

Most transfer stacks hold the whole message in memory. PSICOSE never does:
memory is a few frames on the stack whether you send 4 bytes of config or a
multi-gigabyte file. Reliability (ACK, NACK, SEQ, CRC-8, tick retry) is the
protocol's job — not yours.

CRC-8 is **noise detection**, not authentication. Enable feature `aead` and
call `seal_to` / `open_from` when an attacker may be on the wire. See
[`SECURITY.md`](SECURITY.md).

## Status (0.3 — usable)

- **protocol** — CRC-8, 4-byte frame, `SEQ` wraparound, assembler
- **tx / rx** — stop-and-wait; `START` / `FINISH` / `ABORT` wait for ACK
- **pump** — cooperative `Pump` + `SessionStats` (no inner loop)
- **window** — selective-repeat `1 ≤ N ≤ 8` (`W8Sender` / `W8Receiver`)
- **stream** — `SliceSource` / `SliceSink`, `send_all` / `recv_all`
- **p2p** — `Node`, `PeerLink`, hello, fragmentation (`prelude`)
- **aead** (optional) — ChaCha20-Poly1305 above the transport
- **embedded-io** (optional) — `IoTransport` / `IoSource` / `IoSink` from
  `embedded-io` 0.6 (`Read` + `Write` + `ReadReady`)

Not in this crate: concrete UART/SPI drivers, `File`, routing / gossip.

## Layers (one framework)

```text
 exemplos / application
        │
        ▼
 psicose::Node                              ← motor entry
        ├─ PeerLink / PeerTable / hello
        ├─ Fragmenter / Defragmenter
        ├─ optional aead::seal_to / open_from
        ▼
 Wire::copy / Pump / WindowedPump
        │  DATA 1 byte/frame + CRC-8 + ACK
        ▼
 ByteTransport
        ├─ you implement (UART / SPI / radio)
        └─ or IoTransport::new(port)           (feature = "embedded-io")
```

Use only the transport (`Wire::copy` / `Pump`) or climb to P2P (`Node`) —
same crate, same frame. `Capabilities::WINDOW` is honoured by `PeerLink`
after hello.

## API sketch

```rust
use psicose::prelude::*;
```

| You write | Meaning |
| --- | --- |
| `PeerId::from_label(b"alice")` | 8-byte identity (padded). Never in the 4-byte frame. |
| `Node::<4>::new(id)` | Framework entry: identity + neighbor table. |
| `Node::with(id, SessionConfig::FORUM)` | Hello: stream + window + forum + fragmentation. |
| `SessionConfig::SECURE` | `FORUM` + `ENCRYPTION` (still call `seal_to` yourself). |
| `Fragmenter` / `Defragmenter` | Cut any blob; rebuild into a stack buffer. |
| `seal_to` / `open_from` | Feature `aead`: ciphertext ‖ tag above the transport. |
| `Wire::new().copy(src, sink)` | Motor stop-and-wait A→B (examples use this). |
| `Wire::new().link_pumps()` | In-memory A↔B for `PeerLink` (windowed). |
| `Pump::on(tx, rx)` | Same wrapping on a real UART / SPI / radio. |
| `IoTransport::new(port)` | Feature `embedded-io`: wrap `Read+Write+ReadReady`. |
| `node.connect` / `node.accept` | START + 12-byte hello both ways. |
| `establish` / `send_message` | Cooperative helpers inside the crate. |

```rust
use psicose::prelude::*;

let wire = Wire::new();
let (pump_a, pump_b) = wire.link_pumps();

let mut alice = Node::<4>::new(PeerId::from_label(b"alice"));
let mut bob = Node::<4>::new(PeerId::from_label(b"bob"));

let mut a = match alice.connect(pump_a) {
    Ok(link) => link,
    Err(_) => return,
};
let mut b = bob.accept(pump_b);
assert!(establish(&mut a, &mut alice, &mut b, &mut bob));
```

Runnable: `cargo run --example p2p_pair`. Transport-only: `ab_direct`.
Tests: `tests/forum.rs`.

The 4-byte envelope alone:

```rust
use psicose::Frame;

let frame = Frame::data(0, b'A');
assert_eq!(Frame::from_bytes(frame.to_bytes()), Ok(frame));
```

## Any data, not just “messages”

```text
JPEG  File  Flash  Sensor  firmware.bin  your struct
  └───────┴───────┴────────┴──────────────┘
                    │
               ByteSource
                    │ 1 byte at a time
                    ▼
                 PSICOSE      ← never owns the blob
```

## Examples

In-memory stand-ins; swap `End` for your driver.

```sh
cargo run --example jpeg_over_uart
cargo run --example firmware_flash
cargo run --example sensor_telemetry
cargo run --example radio_windowed
cargo run --example ab_direct
cargo run --example p2p_pair
```

| Example | Layer | What PSICOSE sees |
| --- | --- | --- |
| `ab_direct` | transport (`Pump`) | `ping` / `pong` bytes |
| `p2p_pair` | motor P2P (`Node` + `PeerLink`) | hello + bytes fragmentados |
| `jpeg_over_uart` | transport | JPEG bytes |
| `firmware_flash` | transport | file bytes |
| `sensor_telemetry` | transport | 8-byte sample |
| `radio_windowed` | windowed transport | bytes, window `N=8` |

## Non-negotiables

- `#![no_std]`, `#![forbid(unsafe_code)]`
- No `Vec`, `String`, `Box`, `Rc`/`Arc`, no allocator, no async runtime
- Every public type has a size known at compile time
- Reliability is the protocol's job, never the application's

## Contributing / security

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to run tests and open PRs
- [`SECURITY.md`](SECURITY.md) — threat model and how to report issues
- [`CHANGELOG.md`](CHANGELOG.md)

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
