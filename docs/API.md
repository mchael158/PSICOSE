# PSICOSE — public API catalog

[English](API.md) · [Português (Brasil)](API.pt-BR.md)

What you get with `use psicose::…` (and `use psicose::prelude::*`).
Rustdoc on [docs.rs/psicose](https://docs.rs/psicose) is the detailed
source of truth; this page is the map.

## How to import

```rust
use psicose::prelude::*;           // day-to-day names
use psicose::Node;                 // one item from the crate root
use psicose::p2p::node::Node;      // same type via module path
```

Cargo features: **none**. The crate always has **zero dependencies**.
`use psicose::…` exposes only PSICOSE types.

Prefer this order:

```text
1. Node / LinkFace   — P2P or hardware UART entry
2. Wire / Pump       — host harness or raw pumps
3. Frame             — only if you speak the 4-byte envelope yourself
```

Hardware guide: [HARDWARE.md](HARDWARE.md).

Do **not** build an external P2P stack and pass sockets into psicose.
`Node` / `PeerLink` / `LinkFace` **are** the stack.

---

## Motor / P2P (`psicose::…`)

| Name | What it does |
| --- | --- |
| [`Node`](https://docs.rs/psicose/latest/psicose/struct.Node.html) | Framework entry: local `PeerId` + neighbor table. `connect` / `accept` open links. |
| [`PeerId`](https://docs.rs/psicose/latest/psicose/struct.PeerId.html) | 8-byte identity (`from_label`). Never inside the 4-byte frame. |
| [`PeerTable`](https://docs.rs/psicose/latest/psicose/struct.PeerTable.html) | Neighbor slots (`1 ≤ N ≤ 8`). Prefer driving it via `Node` (`table_mut`). |
| [`PeerEntry`](https://docs.rs/psicose/latest/psicose/struct.PeerEntry.html) | One row in the table (remote id + session). |
| [`PeerLink`](https://docs.rs/psicose/latest/psicose/struct.PeerLink.html) | Live link: windowed pump + hello state machine. Prefer `Node::{connect,accept}`. |
| [`PeerSession`](https://docs.rs/psicose/latest/psicose/struct.PeerSession.html) | Hello / negotiate / established / closed register machine. |
| [`SessionConfig`](https://docs.rs/psicose/latest/psicose/struct.SessionConfig.html) | Offered hello: version, max window, capability bits. Presets: `DEFAULT`, `FORUM`, `SECURE`. |
| [`SessionState`](https://docs.rs/psicose/latest/psicose/enum.SessionState.html) | Disconnected → … → Established → Closed. |
| [`Capabilities`](https://docs.rs/psicose/latest/psicose/struct.Capabilities.html) | Bits: `WINDOW`, `STREAM`, `FORUM`, `COMPRESSION`, `ENCRYPTION`, `FRAGMENTATION`. |
| [`Wire`](https://docs.rs/psicose/latest/psicose/type.Wire.html) | Alias of `DuplexWire`: **in-memory A↔B harness**, not the 4-byte frame. |
| [`DuplexWire`](https://docs.rs/psicose/latest/psicose/struct.DuplexWire.html) | Same as `Wire`. `pumps()`, `link_pumps()`, `copy(src, sink)`. |
| [`DuplexPort`](https://docs.rs/psicose/latest/psicose/struct.DuplexPort.html) | One half of the duplex (TX or RX end). |
| [`DuplexFull`](https://docs.rs/psicose/latest/psicose/struct.DuplexFull.html) | Both directions on one peer (advanced). |
| [`establish`](https://docs.rs/psicose/latest/psicose/fn.establish.html) | Poll two links until both `Established` (busy-loop helper). |
| [`send_message`](https://docs.rs/psicose/latest/psicose/fn.send_message.html) | Fragment + send one message body across an established pair. |
| [`Fragmenter`](https://docs.rs/psicose/latest/psicose/struct.Fragmenter.html) | Cut a blob into `(MessageHeader, chunk)` pieces. |
| [`Defragmenter`](https://docs.rs/psicose/latest/psicose/struct.Defragmenter.html) | Rebuild into a caller-owned buffer (`push`). |
| [`MessageHeader`](https://docs.rs/psicose/latest/psicose/struct.MessageHeader.html) | 7-byte app header (stream, id, fragment, flags, len). |
| [`MessageId`](https://docs.rs/psicose/latest/psicose/struct.MessageId.html) | `u16` application message counter (not `SEQ`). |
| [`StreamId`](https://docs.rs/psicose/latest/psicose/struct.StreamId.html) | App conversation id (`CONTROL` = 0, `FORUM` = 1). |
| [`LinkEvent`](https://docs.rs/psicose/latest/psicose/enum.LinkEvent.html) | One `PeerLink::poll` outcome: Idle, Progress, Established, Received, … |
| [`LinkError`](https://docs.rs/psicose/latest/psicose/enum.LinkError.html) | Transport / handshake / table failure on a link. |
| [`WireCopyError`](https://docs.rs/psicose/latest/psicose/enum.WireCopyError.html) | Why `Wire::copy` stopped (source/sink/protocol/stalled). |
| [`TableError`](https://docs.rs/psicose/latest/psicose/enum.TableError.html) | Full / empty / bad slot on `PeerTable`. |
| [`HandshakeError`](https://docs.rs/psicose/latest/psicose/enum.HandshakeError.html) | Bad hello bytes / negotiation. |
| [`HeaderError`](https://docs.rs/psicose/latest/psicose/enum.HeaderError.html) | Bad message header. |
| [`DefragError`](https://docs.rs/psicose/latest/psicose/enum.DefragError.html) | Defragmenter overflow / mismatch. |
| `HEADER_LEN` | Message header size (7). |
| `HELLO_LEN` | PeerId + SessionConfig on the wire (12). |
| `MAX_PEERS` | Upper bound for `PeerTable` / `Node` const `N` (8). |
| `PROTOCOL_VERSION` | Hello version byte (1). |

---

## Transport / pump

| Name | What it does |
| --- | --- |
| [`Pump`](https://docs.rs/psicose/latest/psicose/struct.Pump.html) | Stop-and-wait session over `ByteTransport` (1 DATA in flight). |
| [`WindowedPump`](https://docs.rs/psicose/latest/psicose/struct.WindowedPump.html) | Same with window `N ≤ 8`. Used by `PeerLink`. |
| [`PumpEvent`](https://docs.rs/psicose/latest/psicose/enum.PumpEvent.html) | Idle / Sent / Received / Completed / Aborted. |
| [`SessionStats`](https://docs.rs/psicose/latest/psicose/struct.SessionStats.html) | ticks, retries, bytes (counters). |
| [`Sender`](https://docs.rs/psicose/latest/psicose/struct.Sender.html) | Stop-and-wait TX machine. |
| [`Receiver`](https://docs.rs/psicose/latest/psicose/struct.Receiver.html) | Stop-and-wait RX machine. |
| [`TxState`](https://docs.rs/psicose/latest/psicose/enum.TxState.html) | Idle / sending / waiting ACK / finished / aborted. |
| [`TxPoll`](https://docs.rs/psicose/latest/psicose/enum.TxPoll.html) | Outcome of one TX poll. |
| [`PollOutcome`](https://docs.rs/psicose/latest/psicose/enum.PollOutcome.html) | Outcome of one RX poll. |
| [`RxState`](https://docs.rs/psicose/latest/psicose/enum.RxState.html) | RX session state. |
| [`WindowedSender`](https://docs.rs/psicose/latest/psicose/struct.WindowedSender.html) | TX with `N` in flight. |
| [`WindowedReceiver`](https://docs.rs/psicose/latest/psicose/struct.WindowedReceiver.html) | RX with reorder buffer. |
| [`W8Sender`](https://docs.rs/psicose/latest/psicose/type.W8Sender.html) / [`W8Receiver`](https://docs.rs/psicose/latest/psicose/type.W8Receiver.html) | Aliases with `N = 8`. |
| `MAX_WINDOW` | Hard cap (8). |
| [`ByteTransport`](https://docs.rs/psicose/latest/psicose/trait.ByteTransport.html) | Non-blocking read/write of one byte. **You implement this** for UART/SPI/radio. |
| [`LinkFace`](https://docs.rs/psicose/latest/psicose/struct.LinkFace.html) | Demux one physical port into Pump TX/RX ends (**hardware path**). |
| [`FaceTx`](https://docs.rs/psicose/latest/psicose/struct.FaceTx.html) / [`FaceRx`](https://docs.rs/psicose/latest/psicose/struct.FaceRx.html) | Ends from `LinkFace::split()`. |
| [`FaceError`](https://docs.rs/psicose/latest/psicose/enum.FaceError.html) | Port error or demux ring full. |
| [`ByteSource`](https://docs.rs/psicose/latest/psicose/trait.ByteSource.html) | Application → bytes (`read_byte`). |
| [`ByteSink`](https://docs.rs/psicose/latest/psicose/trait.ByteSink.html) | Bytes → application (`write_byte`). |
| [`RetryPolicy`](https://docs.rs/psicose/latest/psicose/struct.RetryPolicy.html) | How many retransmits per frame before fail. |
| [`IdleBudget`](https://docs.rs/psicose/latest/psicose/struct.IdleBudget.html) | Outer-loop hang detector (`Default` / unbounded). |
| [`Error`](https://docs.rs/psicose/latest/psicose/enum.Error.html) | Crate-wide transport / protocol error. |

---

## Frame (4-byte envelope)

| Name | What it does |
| --- | --- |
| [`Frame`](https://docs.rs/psicose/latest/psicose/struct.Frame.html) | `TYPE ‖ SEQ ‖ DATA ‖ CRC` encode/decode. |
| [`FrameType`](https://docs.rs/psicose/latest/psicose/enum.FrameType.html) | DATA, ACK, NACK, START, FINISH, ABORT. |
| [`FrameAssembler`](https://docs.rs/psicose/latest/psicose/struct.FrameAssembler.html) | Byte-by-byte into a complete frame. |
| [`FrameError`](https://docs.rs/psicose/latest/psicose/enum.FrameError.html) | Bad type / CRC / semantics. |
| [`Sequence`](https://docs.rs/psicose/latest/psicose/struct.Sequence.html) | `u8` SEQ counter with wrap. |
| [`crc8`](https://docs.rs/psicose/latest/psicose/fn.crc8.html) | CRC-8 over `TYPE‖SEQ‖DATA`. |
| `FRAME_LEN` | Always 4. |
| `PAYLOAD_LEN` | Always 1 (DATA byte). |
| `CRC8_POLY` | Polynomial constant. |

---

## Application byte helpers (`stream`)

| Name | What it does |
| --- | --- |
| [`SliceSource`](https://docs.rs/psicose/latest/psicose/struct.SliceSource.html) | `&[u8]` as `ByteSource`. |
| [`SliceSink`](https://docs.rs/psicose/latest/psicose/struct.SliceSink.html) | `&mut [u8]` as `ByteSink`. |
| `send_bytes` / `recv_bytes` | One-shot helpers over a pump. |
| `send_all` / `recv_all` | Drain source / fill sink stop-and-wait. |
| `send_all_windowed` / `recv_all_windowed` | Same with window. |
| `*_budgeted` variants | Same with `IdleBudget`. |
| `SliceFull` / `StreamError` | Sink full / stream errors. |

---

## Crypto

Not in this crate. Seal application messages yourself before the transport
if needed. `Capabilities::ENCRYPTION` / `SessionConfig::SECURE` are hello
advertisement bits only.

---

## Modules (paths, not always in prelude)

| Module | Role |
| --- | --- |
| `psicose::prelude` | Reexports for `use psicose::prelude::*`. |
| `psicose::p2p` | Node, links, hello, fragmentation. |
| `psicose::protocol` | Frame, CRC, assembler. |
| `psicose::pump` / `tx` / `rx` / `window` | Reliability machines. |
| `psicose::transport` | `ByteTransport`, `LinkFace`. |
| `psicose::stream` | Slice source/sink + send/recv loops. |
| `psicose::timeout` | Retry / idle budgets. |
| `psicose::error` | `Error`. |
| `psicose::actors` | Cooperative multi-link scheduler (advanced). |
| `psicose::fault` | Faulty transport wrappers for tests. |

---

## SessionConfig / Capabilities options

**Presets**

| Preset | Meaning |
| --- | --- |
| `SessionConfig::DEFAULT` | version 1, window 8, `STREAM` |
| `SessionConfig::FORUM` | window 8 + `STREAM` + `WINDOW` + `FORUM` + `FRAGMENTATION` |
| `SessionConfig::SECURE` | `FORUM` + `ENCRYPTION` (you seal outside psicose) |
| `SessionConfig::offer(w, caps)` | Custom window + capability bits |

**Capability bits** (negotiated = intersection / min window)

| Bit | Meaning |
| --- | --- |
| `WINDOW` | After hello, `PeerLink` may pipeline (`max_window`) |
| `STREAM` | Stream/message layer intended |
| `FORUM` | App “forum-style” messaging bit |
| `FRAGMENTATION` | Fragmenter/Defragmenter expected |
| `ENCRYPTION` | Peer announces app-level crypto (you must still seal outside) |
| `COMPRESSION` | Reserved announce bit |

---

## Minimal examples

Transport only:

```rust
use psicose::{SliceSink, SliceSource, Wire};

let mut src = SliceSource::new(b"ping");
let mut buf = [0u8; 8];
let mut sink = SliceSink::new(&mut buf);
let n = Wire::new().copy(&mut src, &mut sink).unwrap();
assert_eq!(n, 4);
```

P2P:

```rust
use psicose::prelude::*;

let wire = Wire::new();
let (pump_a, pump_b) = wire.link_pumps();
let mut alice = Node::<4>::new(PeerId::from_label(b"alice"));
let mut bob = Node::<4>::new(PeerId::from_label(b"bob"));
let mut a = alice.connect(pump_a).unwrap();
let mut b = bob.accept(pump_b);
assert!(establish(&mut a, &mut alice, &mut b, &mut bob));
```

Runnable: `cargo run --example ab_direct`, `cargo run --example p2p_pair`.
Wire format: [PROTOCOL.md](PROTOCOL.md).
