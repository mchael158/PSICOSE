# PSICOSE-1B

[![crates.io](https://img.shields.io/crates/v/psicose.svg)](https://crates.io/crates/psicose)
[![docs.rs](https://docs.rs/psicose/badge.svg)](https://docs.rs/psicose)
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
psicose = "0.2"
```

No features, no dependencies. MSRV: Rust 1.75.

Docs: [docs.rs/psicose](https://docs.rs/psicose) · spec: [`PROTOCOL.md`](PROTOCOL.md)
([pt-BR](PROTOCOL.pt-BR.md))

## Why

Most transfer protocols hold the whole message in memory. PSICOSE never
does: it moves one payload byte at a time. Memory is a few frames on the
stack, whether you send 4 bytes of config or a 4 GB file.

## Status (0.2.0 — experimental)

- **protocol** — CRC-8, 4-byte frame, semantic validation, `SEQ`
  wraparound (`255 → 0`), assembler, `OutBuf` (one wire byte per poll).
- **tx / rx** — stop-and-wait. `START` / `FINISH` wait for ACK.
- **window** — selective-repeat `WindowedSender<_, N>` /
  `WindowedReceiver<_, N>`, `1 ≤ N ≤ 8`. Aliases: `W8Sender`, `W8Receiver`.
- **transport** — `ByteTransport` / `ByteSource` / `ByteSink`.
- **fault** — `FaultyTransport` for adversarial tests.
- **actors** — cooperative `System` for N links on one thread.

Not yet: SessionId in the 4-byte frame, UART/SPI/CAN/radio, `File`.

## Non-negotiables

- `#![no_std]`, `#![forbid(unsafe_code)]`
- No `Vec`, `String`, `Box`, `Rc`/`Arc`, no allocator, no async runtime
- Every public type has a size known at compile time
- Reliability is the protocol's job, never the application's

## Example

```rust
use psicose::Frame;

let frame = Frame::data(0, 0xAA);
assert_eq!(Frame::from_bytes(frame.to_bytes()).unwrap(), frame);
```

## Tests

```sh
cargo +1.75.0 test
```

See `tests/end_to_end.rs`, `tests/hostile.rs`, and `tests/windowed.rs`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
