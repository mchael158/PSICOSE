# Changelog

## Unreleased

- `PeerId::from_label(b"alice")` pads or truncates to 8 bytes. `Display`
  prints the label when it is printable ASCII plus trailing zeros.
- `StreamId::FORUM`, `SessionConfig::FORUM`, and `Defragmenter` (pair of
  `Fragmenter`). Example `forum` and test `tests/forum.rs`: Alice ↔ Bob
  handshake, then payload bytes both ways (`ping` / `pong`). Not a forum.

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
  Spec: `PROTOCOL.md` §11.

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
