# Changelog

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
