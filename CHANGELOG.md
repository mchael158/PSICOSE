# Changelog

## Unreleased

## 0.4.0 — 2026-09-20

### Breaking
- **Zero dependencies, always.** Removed optional features `aead` and
  `embedded-io` (and their crates). `use psicose::…` is only PSICOSE
  types. Implement [`ByteTransport`] on your UART; use [`LinkFace`] for
  demux. Crypto stays in the application if needed.
- Removed `IoTransport` / `IoSource` / `IoSink` and `seal_to` / `open_from`.

### Features
- **`LinkFace`** — demux one physical `ByteTransport` into Pump TX/RX
  ends. Hardware path: UART → your `ByteTransport` → `LinkFace` → `Pump`.
- **ESP32 board examples** (`publish = false`, workspace `boards/`):
  - `boards/esp32-uart` — board-local `ByteTransport` over esp-hal UART1
  - `boards/esp32-wifi` — Wi‑Fi STA / scan / DHCP / TCP → `TcpPipe` →
    `LinkFace` / `Pump` (`esp-radio` + Embassy stay out of psicose)
  - Shared pins in `boards/Cargo.toml`: esp-hal **1.1.2**, esp-radio **0.18**,
    MSRV **1.88** (separate from host MSRV 1.75)
- Host example `tcp_pair` — same TCP + `LinkFace` pattern without a board
  (`--listen` / `--connect` for ESP32 lab).
- Docs: `docs/HARDWARE.md` (+ pt-BR), PROTOCOL §8.2, README hardware-first.
- CI job `esp32-check` (optional / `continue-on-error`).

## 0.3.1 — 2026-09-12

### Features
- **API catalog** — `docs/API.md` / `docs/API.pt-BR.md` list every
  root export and what it does; linked from crate rustdoc, PROTOCOL
  §11.6, and `prelude`.
- **`Node` as framework entry** — identity + neighbor table; `connect` /
  `accept` open `PeerLink`s. Helpers `establish` / `send_message` live in
  the crate so examples do not reinvent P2P.
- **`Wire::copy`** — stop-and-wait A→B through the motor (used by
  `ab_direct`, `jpeg_over_uart`, `sensor_telemetry`).
- Feature `embedded-io`: `IoTransport` / `IoSource` / `IoSink` adapters
  from `embedded-io` **0.6** (`Read` + `Write` + `ReadReady`). Pinned to
  0.6 so MSRV 1.75 keeps resolving (`0.7` needs rustc 1.81+).
  Module path: `psicose::transport::embedded_io`. Wired through crate
  root, `prelude`, `ByteTransport` docs, PROTOCOL §8.1, examples harness
  comments, and `Pump::on(IoTransport, …)` unit smoke test.
- CI covers `--features embedded-io` and `cargo doc --all-features`.
- `PeerLink` honours `Capabilities::WINDOW` after hello (runtime
  `set_window_limit`); stop-and-wait when the bit is absent.

## 0.3.0 — 2026-09-12

### Layout
- Spec moved to `docs/PROTOCOL.md` (+ pt-BR). Example harness to
  `examples/common/link.rs`.
- Example `forum` replaced by `ab_direct` (transport) + `p2p_pair`
  (framework P2P). Crate docs framed as one `no_std` framework with P2P
  in-tree, not a separate package.
- crates.io packaging: `homepage`, docs.rs `all-features` + `docsrs`
  cfg / `doc(cfg)` on `aead`, CI badge on READMEs.
- Public presence: punchier READMEs (when to use / 30s try),
  `SECURITY.md`, `CONTRIBUTING.md`.
- `PeerLink` uses `WindowedPump` (`Wire::link_pumps`). After hello,
  `Capabilities::WINDOW` raises the runtime DATA window to negotiated
  `max_window`; without the bit, DATA stays stop-and-wait (limit 1).

### Transport maturity
- Status: **usable** reliable byte transport (no longer framed as
  experimental-only). Threat model documented in `docs/PROTOCOL.md`: CRC-8 is
  noise detection, not authenticity.
- `IdleBudget::DEFAULT` (1_000_000 idle ticks). `Pump::send_all`,
  `recv_all`, `recv_all_windowed`, and `send_all_windowed` use it so a
  silent peer cannot hang forever. Override with `*_budgeted` /
  `IdleBudget::unbounded` for scripted tests.
- Cleanup carried from unreleased 0.2.x work: `MockFull`, honest
  capability docs, CI lint/doc/`forum`, dead-code cleanup.

### Authenticated encryption (optional)
- Feature `aead`: ChaCha20-Poly1305 (RFC 8439) via RustCrypto,
  `no_std`, stack buffers — `seal` / `open` / `seal_to` / `open_from` /
  `sealed_len`.
- Re-exported at crate root and in `prelude` when the feature is on.
- `SessionConfig::SECURE` = `FORUM` + `ENCRYPTION` (announce only;
  application still calls `seal_to` / `open_from`).
- `Capabilities` docs: bits are announcements; they do not auto-switch
  `PeerLink` internals.
- Integration test `aead_stack` (required-features = `aead`).
- Default build stays **zero dependencies**. Transitive `zeroize` pinned
  `<1.9` so MSRV 1.75 keeps resolving.
- `IdleBudget` implements `Default` (= `DEFAULT`).

## 0.2.3 — 2026-09-11

- `FrameType::Abort = 0x06` on the same 4-byte envelope (`SEQ = 0`).
- Cooperative `Pump` (`rx.poll` then `tx.poll`, no inner loop) and
  `SessionStats` (Copy, stack-only). `Pump::send_all` is the live-pair
  convenience; scripted `stream::send_all` is unchanged.
- `PollOutcome::CrcRejected` / `Aborted`; TX/RX/window abort semantics.
  `PollOutcome::is_closed()` covers FINISH and ABORT.
- P2P layer in the same crate (`p2p` module), still `no_std`/heapless:
  `PeerId` (8 bytes), 12-byte hello (`PeerId` + version + window +
  features), `Capabilities` bits (`WINDOW`/`STREAM`/`FORUM`/
  `COMPRESSION`/`ENCRYPTION`/`FRAGMENTATION`; CRC is not negotiated),
  deterministic handshake (min version, min window, feature
  intersection), `PeerSession` states (`Disconnected`/`Connecting`/
  `Established`/`Closing`/`Aborted`, ≤ 128 bytes), `StreamId` /
  `MessageId` / 7-byte `MessageHeader` / `Fragmenter`. Everything
  travels as payload bytes; the 4-byte frame is unchanged.
- End-to-end P2P: `PeerTable<N>` (`1 ≤ N ≤ 8`) + `PeerLink` over a real
  `Pump` (START + 12-byte hello both ways). `Wire` demuxes one incoming
  stream (ACK/NACK → TX, the rest → RX).
- Public names: `use psicose::prelude::*`. `PeerTable::new(id)`,
  `Wire::new().pumps()`, `Pump::on(tx, rx)`, `PeerLink::connect` /
  `accept`, `link.offer(byte)`, `PeerId::from([u8; 8])`,
  `SessionConfig::offer(window, features)`, `Capabilities::STREAM | …`.
  Spec: `docs/PROTOCOL.md` §11.

## 0.2.2 — 2026-09-10

- Library, tests, and docs no longer call `unwrap` / `expect` / `panic`.
  Missing slots return `Pending` or `Err`; examples use `assert_eq!(…, Ok(…))`.
- Runnable real-world examples: `jpeg_over_uart`, `firmware_flash`,
  `sensor_telemetry`, `radio_windowed`.

## 0.2.1 — 2026-09-10

- Application stream: `SliceSource` / `SliceSink`, `send_all` / `recv_all`
  (and windowed counterparts). The 4-byte frame is the envelope; JPEG,
  file, flash, sensor, and `struct` bytes all go through `ByteSource`.
- `repository` field points at GitHub.

## 0.2.0 — 2026-09-10

- Selective-repeat window: `WindowedSender<_, N>` / `WindowedReceiver<_, N>`
  with `1 ≤ N ≤ 8`, `[Option<T>; N]`, no heap.
- Send window is `[oldest_unacked, oldest_unacked + N)`.
- Type aliases `W8Sender` / `W8Receiver`.

## 0.1.2 — 2026-09-10

- One wire byte per poll (`OutBuf`).
- `Delivered` only after the matching ACK is fully written.
- `FINISH` accepted only at `expected_seq`.
- Closed session ignores late DATA and corrupt frames.
- `offer` returns `Error::NotIdle` instead of a debug-only assert.

## 0.1.1 — 2026-09-10

- Frames valid by construction (`data` / `ack` / `nack` / `start` / `finish`).
- `START` resets the session and waits for ACK.
- `FINISH` waits for ACK.
- Formal ACK/NACK rules on the sender.
- Hostile-wire tests (lost/corrupt DATA, lost ACK/NACK, delayed DATA).

## 0.1.0

- Initial stop-and-wait PSICOSE-1B core (`no_std`, heapless, 4-byte frame).
