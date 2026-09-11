# PSICOSE-1B — formal protocol (0.2.3)

[English](PROTOCOL.md) · [Português (Brasil)](PROTOCOL.pt-BR.md)

A `no_std`, heapless transport machine. This file is the specification.
The code in `src/` is the implementation. If they diverge, the
adversarial tests in `tests/hostile.rs` decide.

**Status:** experimental reliable byte transport plus a P2P layer that
rides the same 4-byte frame as payload. Not a final protocol.

```
                    APPLICATION
                         │
              ┌──────────▼──────────┐
              │     p2p (here)      │
              │ PeerId / Session    │
              │ Stream / Message    │
              └──────────┬──────────┘
                         │ payload bytes
              ┌──────────▼──────────┐
              │      transport      │
              │  ACK / NACK / CRC   │
              │  START / FINISH /   │
              │  ABORT / Pump       │
              └──────────┬──────────┘
                         │
                   ByteTransport
                         │
          ┌──────────────┼──────────────┐
          │              │              │
        UART           TCP/UDP        Radio
```

The transport does not know what a peer, a stream, or a forum post is.
The P2P layer does not grow the frame.

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
| Abort | `0x06` | `0x00` | `0x00` |

CRC-8: poly `0x07`, init `0x00`, no reflection, no xor-out, over
`TYPE || SEQ || DATA`.

### Semantic validity

CRC-valid ≠ semantically valid. A `Frame` exists only if **both** hold.

Control frames (`ACK`, `NACK`, `START`, `FINISH`, `ABORT`) must have
`DATA = 0`. `START` and `ABORT` must have `SEQ = 0`.

Public constructors are `Frame::data`, `Frame::ack`, `Frame::nack`,
`Frame::start`, `Frame::finish`, and `Frame::abort`. There is no public `Frame::new`.
`Frame::from_bytes` rejects a CRC-valid frame that breaks those rules
(`FrameError::InvalidSemantics`). There is no `unchecked` path.

## 3. Sequence

```
0 → 1 → … → 254 → 255 → 0
```

`previous(0) == 255`. Wraparound is part of the protocol, not an error.

## 4. Session (START / DATA / FINISH / ABORT)

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
- `ABORT` is ignored (the session is already closed)
- `START` opens a new session

`ABORT` is a first-class frame (`TYPE = 0x06`). It does not grow the
envelope. There is no out-of-band cancel: every session event is the
same `TYPE | SEQ | DATA | CRC` stream.

```
TX                              RX
──                              ──
ABORT  ─────────────────────►   ACK 0, cancel, discard pending DATA
       ◄─────────────────────   ACK 0
```

`ABORT` is reliable like `FINISH`: the sender waits for `ACK(0)`. A lost
`ABORT` is retried. A duplicate `ABORT` is re-ACKed.

Semantics:

- TX `offer_abort` drops any in-flight DATA/START/FINISH, including a
  frame mid-write or mid-retry, and sends `ABORT` (`SEQ = 0`).
- An `ABORT` received while TX is retransmitting wins: DATA in flight
  is discarded.
- RX ACKs `0`, latches aborted, resets `expected_seq` to 0.
- After abort: late DATA and corrupt frames are ignored (same latch as
  FINISH). `START` reopens the session at seq 0, including wrap
  `255 → 0`.
- A peer `ABORT` observed on the TX control path locally cancels
  without sending a second `ABORT`. The RX side of the same endpoint
  is the one that writes the ACK.

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
NACK the current `expected`, keep polling. CRC failures return
`PollOutcome::CrcRejected`; a wrong `SEQ` or bad TYPE/semantics
returns `PollOutcome::Rejected`. Both send a NACK.

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
                      │ offer / offer_start / offer_finish / offer_abort
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
         ACK(cur)  NACK(cur)  TIMEOUT / peer ABORT
             │        │        │
             ▼        └────┬───┘
           IDLE            │
      (Finished/Aborted)   ▼
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
         DATA        START       FINISH      ABORT
           │           │           │           │
           ▼           ▼           ▼           ▼
      ACK / NACK    ACK 0       ACK seq      ACK 0
           │           │           │           │
           ▼           ▼           ▼           ▼
         IDLE        IDLE       FINISHED    ABORTED
```

A duplicate `FINISH` after `FINISHED` is re-ACKed. `START` after
`FINISHED` or `ABORTED` opens a new session. `ABORT` after `FINISHED`
is ignored.

## 7. Fault injection

`psicose::fault::FaultyTransport` wraps any `ByteTransport` and can:

- **DROP DATA** — RX never sees the frame; TX burns ticks and retransmits
- **CORRUPT** — invalid CRC → NACK → retransmit
- **DROP ACK** — TX retransmits; RX recognizes a duplicate and does not
  re-deliver
- **DROP NACK** — treated as silence; TX times out and retransmits
- **DELAY DATA** — reads return `Ok(None)` for N ticks, then the frame
- **DROP FINISH / FINISH ACK** — FINISH is retried until ACKed
- **ABORT during DATA / retry** — abort wins; DATA in flight is dropped
- **ABORT after FINISH** — ignored
- **START after ABORT** — new session at seq 0, including `255 → 0`

None of these modes may corrupt the stream seen by the `ByteSink`.

## 7.1 Pump and SessionStats

`Pump` is one cooperative step: `rx.poll()` then `tx.poll()`. It never
loops inside `poll`. `Pump::send_all` is only that loop stacked by the
caller. Scripted `stream::send_all` still expects ACKs on the sender's
own transport.

`SessionStats` lives on the pump (`Copy`, stack-only, no logging):

| Field | Meaning |
|-------|---------|
| `bytes_delivered` | Payload bytes the RX delivered after writing the ACK |
| `frames_sent` | Frames whose four bytes fully left the TX `OutBuf` |
| `retries` | Times the sender entered retransmission |
| `nacks` | NACKs generated by the receiver |
| `duplicates` | Duplicate DATA re-ACKed without delivering |
| `crc_errors` | CRC rejects (also counted in `nacks`) |
| `ticks` | How many times `Pump::poll` was called |

`PumpEvent` of one step, highest first: `Aborted` > `Completed` >
`Received(u8)` > `Sent` > `Progress` > `Idle`.

CRC failures return `PollOutcome::CrcRejected`; a wrong `SEQ` or bad
TYPE/semantics return `PollOutcome::Rejected`. Both send a NACK.

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
0xF0               TX_SEQ
0xF1               RX_SEQ
0xF2               BYTES_LO
0xF3               BYTES_HI
0xF4               RETRIES
0xF5               NACKS
0xF6               CRC_ERRORS
0xF7               DUPLICATES
0xF8 ───── 0xFF    reserved
```

Address = `u8`, data = `u8`, memory = 256 bytes. This is not a VM yet.
`SessionStats` is the software form of those registers.

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
START / FINISH / ABORT remain stop-and-wait (window must be empty to
FINISH). ABORT clears the window. FINISH still requires SEQ == expected
(no holes).
```

`WindowFull` means: poll until an ACK frees a slot, then offer again.

## 11. P2P layer (`p2p` module)

Same crate. Same `no_std` / heapless / no-`unsafe` budget. Identity,
sessions, streams, and messages are **payload bytes**. The 4-byte frame
does not change and nothing of this layer enters it.

Start from `psicose::prelude::*`. `PeerSession` never touches a
transport: it produces and consumes bytes. The caller moves them with
`Pump`, `send_bytes`, or anything else. `PeerSession` is ≤ 128 bytes.

### 11.1 PeerId

64-bit identity (`[u8; 8]`). `PeerId::from_label(b"alice")` pads (or
truncates) a name to 8 bytes. Raw bytes still work:
`PeerId::from([u8; 8])`. How they are generated (name, random, hash of
a key, serial) is the application's business. The wire frame does not
carry it.

### 11.2 Hello (12 payload bytes)

Sent as ordinary DATA right after START:

```
┌────────────┬─────────┬────────────┬──────────────┐
│ PeerId (8) │ ver (1) │ window (1) │ features (2) │
└────────────┴─────────┴────────────┴──────────────┘
```

`SessionConfig` is the last 4 bytes: `ver | window | features_hi |
features_lo`. Version `0` and window outside `1..=8` are rejected
(`HandshakeError`). `max_window` is clamped into `1..=8` on construct.
`SessionConfig::DEFAULT` is version 1, window 8, `STREAM`.
`SessionConfig::offer(4, features)` fills the version for you.

### 11.3 Capabilities (`u16`, big-endian in the hello)

CRC is **not** a capability. The transport frame always carries it.

| Bit | Name | Meaning |
|-----|------|---------|
| 0 | `WINDOW` | Selective-repeat (`N ≤ 8`) |
| 1 | `STREAM` | Logical streams over the session |
| 2 | `FORUM` | Application messages (not a forum product) |
| 3 | `COMPRESSION` | Payload compression (above the transport) |
| 4 | `ENCRYPTION` | Payload encryption (above the transport) |
| 5 | `FRAGMENTATION` | Message fragmentation (`Fragmenter`) |

Unknown bits are kept as-is and die in the intersection with a peer
that does not set them.

Negotiation is deterministic and has no extra round: min version, min
window, intersection of feature bits. Both ends compute the same
result. In code: `Capabilities::STREAM | Capabilities::WINDOW`.

### 11.4 PeerSession states

```
Disconnected ──connect()──► Connecting ──on_hello()──► Established
     ▲                                                     │
     │                                                close() / abort()
     └── closed() ◄── Closing ◄────────────────────────────┤
                                                           ▼
                                                        Aborted
                                                           │
                                                      connect()
                                                           ▼
                                                      Connecting
```

| State | Transport counterpart |
|-------|------------------------|
| `Disconnected` | Idle / after FINISH ACK |
| `Connecting` | START in flight; our hello left |
| `Established` | Hellos crossed; DATA may flow |
| `Closing` | FINISH in flight |
| `Aborted` | ABORT sent or received |

`on_hello` returns `Ok(Some(reply))` on the accepting side (must send
the reply) and `Ok(None)` when it completes a connect we initiated.
`record_stats` copies `SessionStats` from the pump.

### 11.5 Streams and messages

Do not mix the counters:

| Name | Size | Owner | Meaning |
|------|------|-------|---------|
| `SEQ` | `u8` | transport | which DATA frame |
| `StreamId` | `u8` | application | which conversation |
| `MessageId` | `u16` | application | which message in the stream |
| `fragment` | `u16` | application | which piece of that message |

Stream 0 is reserved for session control (`StreamId::CONTROL`). This
crate's usual application-data stream is `StreamId::FORUM` (1) — a
stand-in name, not a forum product. Other mappings remain application
policy.

Message header — 7 payload bytes per fragment:

```
┌────────────┬────────────────┬──────────────┬───────────┬─────────┐
│ stream (1) │ message id (2) │ fragment (2) │ flags (1) │ len (1) │
└────────────┴────────────────┴──────────────┴───────────┴─────────┘
```

Flags bit 0 = last fragment. All other bits are reserved and rejected
(`HeaderError::Flags`). Ids are big-endian.

`Fragmenter` borrows the caller's payload and yields `(header, chunk)`
until the last fragment. `Defragmenter` writes those bytes back into a
caller-owned buffer, one payload byte at a time (`push`). Chunk size
`0` is treated as 1. An empty payload still yields one empty last
fragment so the receiver sees the message exist. A 4 GB blob and an
11-byte post use the same iterator.

### 11.6 PeerTable, PeerLink, Wire

`PeerTable<N>` is `[Option<PeerEntry>; N]` with `1 ≤ N ≤ 8`. No `Vec`.

| Call | Meaning |
|------|---------|
| `PeerTable::new(id)` | Empty table. Offers `SessionConfig::DEFAULT` (version 1, window 8, `STREAM`). |
| `PeerTable::with(id, cfg)` | Same, explicit config. `SessionConfig::FORUM` or `SessionConfig::offer(4, features)`. |
| `table.connect()` | Takes a free slot, returns the hello. |
| `table.accept(hello)` | Installs an incoming hello. |
| `table.find(id)` | Locates a neighbor. |

`PeerLink` is the live side: one `Pump` plus the hello state machine.

```text
connect:  START → hello(12) → wait hello → Established
accept:   wait hello → START → hello(12) → Established
```

`PeerLink::poll(&mut table)` never loops. After `Established`, DATA is
ordinary payload (`LinkEvent::Received`). Call `link.offer(byte)` —
not `pump_mut().sender_mut().offer(byte)`.

One physical duplex has **one** incoming stream. The sender needs
ACK/NACK from it; the receiver needs DATA/START/FINISH/ABORT. Two
readers on the same ring steal each other's bytes (the RX eats the
ACK the TX is waiting for). `Wire` (`DuplexWire`) demuxes complete
frames into a control lane (ACK/NACK → TX) and a payload lane
(everything else, including a CRC miss → RX so it can NACK). The
4-byte frame does not change.

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

The same pair moves bytes A→B. Not a forum. Runnable:
`examples/forum.rs`. Test: `tests/forum.rs`.

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

A real UART driver does the same split: `Pump::on(tx, rx)`.

### 11.7 Not yet (applications of this layer)

Routing, store-and-forward, gossip (`SeenSet<N>`), signatures, content
hash, backpressure / priority. They stay out of the transport and out
of the 4-byte frame.

## 12. Out of scope here

- SessionId inside the 4-byte frame
- UART / SPI / CAN / radio
- `psicose::File`

Order: transport + P2P identity/session/stream are here → discovery /
gossip → forum.
