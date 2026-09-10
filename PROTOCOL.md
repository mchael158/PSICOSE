# PSICOSE-1B — formal protocol (0.2.1)

[English](PROTOCOL.md) · [Português (Brasil)](PROTOCOL.pt-BR.md)

A `no_std`, heapless transport machine. This file is the specification.
The code in `src/` is the implementation. If they diverge, the
adversarial tests in `tests/hostile.rs` decide.

**Status:** experimental reliable byte transport. Not a final protocol.

## 1. Units

| Name | Size | Role |
|------|------|------|
| **payload** | 1 byte (8 bits) | Application data. The only thing `ByteSource` / `ByteSink` see. |
| **frame** | 4 bytes | Unit on the wire. Includes transport overhead. |
| **SEQ** | `u8` | Counter mod 256. Wrap `255 → 0` is intended behavior, not a bug. |

Saying "PSICOSE transmits 1 byte" refers to the **payload**. The
**frame** is always 4 bytes.

```
frame efficiency (before ACK)       = PAYLOAD_LEN / FRAME_LEN = 1/4 = 25%
stop-and-wait efficiency (DATA+ACK) = 1 / 8                   = 12.5%
```

Windowed mode (`window::WindowedSender<_, N>`, `N ≤ 8`, no heap) raises
that ratio. It does not change the frame.

Dogma: a 4 GB file must cross PSICOSE without the protocol ever holding
more than a few bytes of it.

## 2. Frame

```
offset  0        1        2        3
      ┌────────┬────────┬────────┬────────┐
      │  TYPE  │  SEQ   │  DATA  │  CRC   │
      └────────┴────────┴────────┴────────┘
        overhead  overhead  payload  overhead
```

| TYPE | Value | SEQ | DATA |
|------|-------|-----|------|
| Data | `0x01` | byte sequence | application payload |
| Ack | `0x02` | acknowledged sequence | `0x00` |
| Nack | `0x03` | rejected sequence | `0x00` |
| Start | `0x04` | `0x00` | `0x00` |
| Finish | `0x05` | current TX sequence | `0x00` |

CRC-8: poly `0x07`, init `0x00`, no reflection, no xor-out, over
`TYPE || SEQ || DATA`.

### Semantic validity

CRC-valid ≠ semantically valid. A `Frame` exists only if **both** hold.

Control frames (`ACK`, `NACK`, `START`, `FINISH`) must have `DATA = 0`.
`START` must have `SEQ = 0`.

Public constructors are `Frame::data`, `Frame::ack`, `Frame::nack`,
`Frame::start`, and `Frame::finish`. There is no public `Frame::new`.
`Frame::from_bytes` rejects a CRC-valid frame that breaks those rules
(`FrameError::InvalidSemantics`). There is no `unchecked` path.

## 3. Sequence

```
0 → 1 → … → 254 → 255 → 0
```

`previous(0) == 255`. Wraparound is part of the protocol, not an error.

## 4. Session (START / DATA / FINISH)

A transfer is an explicit session:

```
TX                              RX
──                              ──
START  ─────────────────────►   expected_seq = 0
       ◄─────────────────────   ACK 0
DATA 0 ─────────────────────►   deliver, ACK 0
DATA 1 ─────────────────────►   deliver, ACK 1
…                               …
FINISH seq=k ───────────────►   ACK k, then TransferFinished
       ◄─────────────────────   ACK k
```

`START` may arrive mid-stream. The receiver resets `expected_seq` to 0
and ACKs sequence 0. The sender also resets its DATA sequence to 0
after that ACK.

`FINISH` is not fire-and-forget. Its `SEQ` **must** equal the receiver's
`expected_seq` (the next unused DATA sequence). A `FINISH` with any
other `SEQ` is NACKed and does **not** close the session — better to
fail than to drop the tail of a file.

The sender only reaches `Finished` after `ACK(seq of FINISH)`. A lost
`FINISH` is retried. A lost FINISH ACK is retried: the receiver re-ACKs
the same `FINISH` and does not re-open the transfer.

After `Finished`:

- late `DATA` is ignored (not delivered, not NACKed)
- a corrupt frame is ignored (not NACKed)
- a `FINISH` with the closed sequence is re-ACKed
- `START` opens a new session

A session identifier is **not** in the 4-byte frame. A future version
may negotiate one through extra frames after `START`. Do not grow the
frame to add it.

I/O is one byte per poll on both sides. The application sees
`Delivered` only after the matching ACK has been fully written. A frame
is not "on the wire" until all four bytes have left `OutBuf`.

## 5. Stop-and-wait (DATA)

At most **one** DATA frame in flight.

```
TX                              RX
──                              ──
DATA seq=k  ─────────────────►  if CRC/type/semantics fail: NACK expected, do not deliver
                                if seq == expected:  ACK k, deliver, expected++
                                if seq == previous:  ACK k, do NOT deliver   ← critical
                                otherwise:           NACK seq, do not deliver
            ◄─────────────────  ACK / NACK
```

A lost ACK is the case that lies: RX has already advanced. The
retransmit **must not** deliver the byte again. `DuplicateIgnored` +
re-ACK.

Bad CRC, TYPE, or semantics on RX is **not** an application error.
NACK the current `expected`, return `PollOutcome::Rejected`, keep
polling.

### Formal ACK/NACK on TX

| Received while waiting | Action |
|------------------------|--------|
| `ACK(current seq)` | success (advance / session ready / finished) |
| `NACK(current seq)` | retransmit now |
| `ACK(other seq)` | ignore; keep waiting (reset empty ticks) |
| `NACK(other seq)` | ignore; keep waiting (reset empty ticks) |
| invalid frame | ignore; wait for timeout / retransmit |

## 6. State machines

### Sender

```
                 ┌─────────┐
                 │  IDLE   │
                 └────┬────┘
                      │ offer / offer_start / offer_finish
                      ▼
                 ┌─────────┐
                 │ SENDING │
                 └────┬────┘
                      ▼
                ┌───────────┐
                │ WAIT_ACK  │
                └─────┬─────┘
                      │
             ┌────────┼────────┐
             │        │        │
         ACK(cur)  NACK(cur)  TIMEOUT
             │        │        │
             ▼        └────┬───┘
           IDLE            │
      (or Finished)        ▼
                       RETRYING
                           │
                           ▼
                       SENDING
```

Foreign ACK/NACK and invalid frames stay in `WAIT_ACK`.

### Receiver

```
                 ┌─────────┐
                 │  IDLE   │
                 └────┬────┘
                      │ byte
                      ▼
                 ┌────────────┐
                 │ RECEIVING  │  (assembler filling)
                 └─────┬──────┘
                       │ 4th byte
                       ▼
                 ┌────────────┐
                 │ VALIDATING │
                 └─────┬──────┘
           ┌───────────┼───────────┐
           │           │           │
         DATA        START       FINISH
           │           │           │
           ▼           ▼           ▼
      ACK / NACK    ACK 0       ACK seq
           │           │           │
           ▼           ▼           ▼
         IDLE        IDLE       FINISHED
```

A duplicate `FINISH` after `FINISHED` is re-ACKed. `START` after
`FINISHED` opens a new session.

## 7. Fault injection

`psicose::fault::FaultyTransport` wraps any `ByteTransport` and can:

- **DROP DATA** — RX never sees the frame; TX burns ticks and retransmits
- **CORRUPT** — invalid CRC → NACK → retransmit
- **DROP ACK** — TX retransmits; RX recognizes a duplicate and does not
  re-deliver
- **DROP NACK** — treated as silence; TX times out and retransmits
- **DELAY DATA** — reads return `Ok(None)` for N ticks, then the frame
- **DROP FINISH / FINISH ACK** — FINISH is retried until ACKed

None of these modes may corrupt the stream seen by the `ByteSink`.

## 8. ByteSource / ByteSink

These are not the link. They are the **application**. The frame is only
the envelope. PSICOSE does not have a JPEG type, a file type, or a
struct type — those are sequences of bytes.

```
JPEG  File  Flash  Sensor  firmware.bin  [u8] of a struct
  │     │     │       │         │              │
  └─────┴─────┴───────┴─────────┴──────────────┘
                      │
                 ByteSource
                      │ 1 payload byte
                      ▼
                   PSICOSE          ← never owns the blob
                      │ Frame (4 B)
                      ▼
                 ByteTransport
                      │
                 ByteSink
```

`stream::send_all` / `recv_all` drain a source onto a sink. `SliceSource`
/ `SliceSink` borrow a buffer the caller already owns.

`File` is an implementation. The core crate does not include it.

## 9. Node memory (future PSICOSE-8)

The transport protocol is already an 8-bit register machine (`SEQ`,
`DATA`, CRC). The 256-byte identity is not abandoned:

```
PSICOSE NODE
0x00 ───── 0xEF    application scratch
0xF0               TX sequence
0xF1               RX sequence
0xF2               TX retries
0xF3               RX state
0xF4               CRC state
0xF5               timeout ticks
0xF6 ───── 0xFF    reserved
```

Address = `u8`, data = `u8`, memory = 256 bytes. This is not a VM yet.
It is the state ceiling that 0.2.1 refuses to exceed.

## 10. Windowed (`N ≤ 8`)

Selective repeat. The frame does not change. Memory is

```
TX:  [Option<Slot>; N]
RX:  [Option<u8>; N]     ← reorder buffer, at most N payload bytes
```

`1 ≤ N ≤ 8`, checked at compile time. Heap = 0. `std` is not used.

```
TX send window is [oldest_unacked, oldest_unacked+N).
A free slot is not enough: SEQ must stay inside that range.
RX accepts SEQ in [expected, expected+N) and stores the payload.
RX re-ACKs SEQ in [expected-N, expected) without delivering.
RX delivers only the in-order prefix of the buffer.
START / FINISH remain stop-and-wait (window must be empty to FINISH).
FINISH still requires SEQ == expected (no holes).
```

`WindowFull` means: poll until an ACK frees a slot, then offer again.

## 11. Out of scope here

- SessionId inside the 4-byte frame
- UART / SPI / CAN / radio
- `psicose::File`

Order: Windowed is here → concrete sources/sinks → real wire.
