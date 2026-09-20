# PSICOSE — catálogo da API pública

[English](API.md) · [Português (Brasil)](API.pt-BR.md)

O que você obtém com `use psicose::…` (e `use psicose::prelude::*`).
O detalhe está no rustdoc em [docs.rs/psicose](https://docs.rs/psicose);
esta página é o mapa.

## Como importar

```rust
use psicose::prelude::*;           // nomes do dia a dia
use psicose::Node;                 // um item no root da crate
use psicose::p2p::node::Node;      // o mesmo tipo via módulo
```

Features do Cargo: **nenhuma**. A crate tem **sempre zero dependências**.
`use psicose::…` expõe só tipos da PSICOSE.

## Ordem preferida

```text
1. Node / LinkFace   — entrada P2P ou UART no hardware
2. Wire / Pump       — harness no host ou pumps cruas
3. Frame             — só se montar o envelope de 4 bytes à mão
```

Guia de hardware: [HARDWARE.pt-BR.md](HARDWARE.pt-BR.md).

Não monte um P2P externo e passe conexão para o psicose.
`Node` / `PeerLink` / `LinkFace` **são** a pilha.

---

## Motor / P2P (`psicose::…`)

| Nome | O que faz |
| --- | --- |
| [`Node`](https://docs.rs/psicose/latest/psicose/struct.Node.html) | Entrada do framework: `PeerId` local + tabela. `connect` / `accept` abrem links. |
| [`PeerId`](https://docs.rs/psicose/latest/psicose/struct.PeerId.html) | Identidade 8 bytes (`from_label`). Nunca no frame de 4. |
| [`PeerTable`](https://docs.rs/psicose/latest/psicose/struct.PeerTable.html) | Slots de vizinhos (`1 ≤ N ≤ 8`). Prefira via `Node` (`table_mut`). |
| [`PeerEntry`](https://docs.rs/psicose/latest/psicose/struct.PeerEntry.html) | Uma linha da tabela (id remoto + sessão). |
| [`PeerLink`](https://docs.rs/psicose/latest/psicose/struct.PeerLink.html) | Link vivo: pump janelada + máquina do hello. Prefira `Node::{connect,accept}`. |
| [`PeerSession`](https://docs.rs/psicose/latest/psicose/struct.PeerSession.html) | Máquina hello / negotiate / established / closed. |
| [`SessionConfig`](https://docs.rs/psicose/latest/psicose/struct.SessionConfig.html) | Oferta no hello: versão, janela, bits. Presets: `DEFAULT`, `FORUM`, `SECURE`. |
| [`SessionState`](https://docs.rs/psicose/latest/psicose/enum.SessionState.html) | Disconnected → … → Established → Closed. |
| [`Capabilities`](https://docs.rs/psicose/latest/psicose/struct.Capabilities.html) | Bits: `WINDOW`, `STREAM`, `FORUM`, `COMPRESSION`, `ENCRYPTION`, `FRAGMENTATION`. |
| [`Wire`](https://docs.rs/psicose/latest/psicose/type.Wire.html) | Alias de `DuplexWire`: **harness A↔B em memória**, não o frame de 4 bytes. |
| [`DuplexWire`](https://docs.rs/psicose/latest/psicose/struct.DuplexWire.html) | Igual a `Wire`. `pumps()`, `link_pumps()`, `copy(src, sink)`. |
| [`DuplexPort`](https://docs.rs/psicose/latest/psicose/struct.DuplexPort.html) | Metade do duplex (TX ou RX). |
| [`DuplexFull`](https://docs.rs/psicose/latest/psicose/struct.DuplexFull.html) | Ambos os sentidos num peer (avançado). |
| [`establish`](https://docs.rs/psicose/latest/psicose/fn.establish.html) | Faz poll nos dois links até `Established` (helper com loop). |
| [`send_message`](https://docs.rs/psicose/latest/psicose/fn.send_message.html) | Fragmenta e envia um corpo pela sessão estabelecida. |
| [`Fragmenter`](https://docs.rs/psicose/latest/psicose/struct.Fragmenter.html) | Corta blob em `(MessageHeader, pedaço)`. |
| [`Defragmenter`](https://docs.rs/psicose/latest/psicose/struct.Defragmenter.html) | Remonta num buffer do caller (`push`). |
| [`MessageHeader`](https://docs.rs/psicose/latest/psicose/struct.MessageHeader.html) | Cabeçalho de app 7 bytes. |
| [`MessageId`](https://docs.rs/psicose/latest/psicose/struct.MessageId.html) | Contador `u16` de mensagem (não é `SEQ`). |
| [`StreamId`](https://docs.rs/psicose/latest/psicose/struct.StreamId.html) | Id de conversa (`CONTROL` = 0, `FORUM` = 1). |
| [`LinkEvent`](https://docs.rs/psicose/latest/psicose/enum.LinkEvent.html) | Resultado de um `PeerLink::poll`. |
| [`LinkError`](https://docs.rs/psicose/latest/psicose/enum.LinkError.html) | Falha de transporte / hello / tabela. |
| [`WireCopyError`](https://docs.rs/psicose/latest/psicose/enum.WireCopyError.html) | Por que `Wire::copy` parou. |
| [`TableError`](https://docs.rs/psicose/latest/psicose/enum.TableError.html) | Tabela cheia / vazia / slot inválido. |
| [`HandshakeError`](https://docs.rs/psicose/latest/psicose/enum.HandshakeError.html) | Hello inválido / negociação. |
| [`HeaderError`](https://docs.rs/psicose/latest/psicose/enum.HeaderError.html) | Cabeçalho de mensagem inválido. |
| [`DefragError`](https://docs.rs/psicose/latest/psicose/enum.DefragError.html) | Overflow / mismatch no defrag. |
| `HEADER_LEN` | Tamanho do cabeçalho de mensagem (7). |
| `HELLO_LEN` | PeerId + SessionConfig no fio (12). |
| `MAX_PEERS` | Limite de `N` em `PeerTable` / `Node` (8). |
| `PROTOCOL_VERSION` | Byte de versão do hello (1). |

---

## Transporte / pump

| Nome | O que faz |
| --- | --- |
| [`Pump`](https://docs.rs/psicose/latest/psicose/struct.Pump.html) | Sessão stop-and-wait sobre `ByteTransport` (1 DATA em voo). |
| [`WindowedPump`](https://docs.rs/psicose/latest/psicose/struct.WindowedPump.html) | Igual com janela `N ≤ 8`. Usada pelo `PeerLink`. |
| [`PumpEvent`](https://docs.rs/psicose/latest/psicose/enum.PumpEvent.html) | Idle / Sent / Received / Completed / Aborted. |
| [`SessionStats`](https://docs.rs/psicose/latest/psicose/struct.SessionStats.html) | Contadores: ticks, retries, bytes. |
| [`Sender`](https://docs.rs/psicose/latest/psicose/struct.Sender.html) | Máquina TX stop-and-wait. |
| [`Receiver`](https://docs.rs/psicose/latest/psicose/struct.Receiver.html) | Máquina RX stop-and-wait. |
| [`TxState`](https://docs.rs/psicose/latest/psicose/enum.TxState.html) | Idle / enviando / esperando ACK / finished / aborted. |
| [`TxPoll`](https://docs.rs/psicose/latest/psicose/enum.TxPoll.html) | Resultado de um poll TX. |
| [`PollOutcome`](https://docs.rs/psicose/latest/psicose/enum.PollOutcome.html) | Resultado de um poll RX. |
| [`RxState`](https://docs.rs/psicose/latest/psicose/enum.RxState.html) | Estado da sessão RX. |
| [`WindowedSender`](https://docs.rs/psicose/latest/psicose/struct.WindowedSender.html) | TX com `N` em voo. |
| [`WindowedReceiver`](https://docs.rs/psicose/latest/psicose/struct.WindowedReceiver.html) | RX com buffer de reordenação. |
| [`W8Sender`](https://docs.rs/psicose/latest/psicose/type.W8Sender.html) / [`W8Receiver`](https://docs.rs/psicose/latest/psicose/type.W8Receiver.html) | Aliases com `N = 8`. |
| `MAX_WINDOW` | Teto duro (8). |
| [`ByteTransport`](https://docs.rs/psicose/latest/psicose/trait.ByteTransport.html) | Leitura/escrita non-blocking de 1 byte. **Você implementa** no UART/SPI/rádio. |
| [`LinkFace`](https://docs.rs/psicose/latest/psicose/struct.LinkFace.html) | Demux de uma porta física nos extremos TX/RX do Pump (**caminho hardware**). |
| [`FaceTx`](https://docs.rs/psicose/latest/psicose/struct.FaceTx.html) / [`FaceRx`](https://docs.rs/psicose/latest/psicose/struct.FaceRx.html) | Extremos de `LinkFace::split()`. |
| [`FaceError`](https://docs.rs/psicose/latest/psicose/enum.FaceError.html) | Erro da porta ou anel de demux cheio. |
| [`ByteSource`](https://docs.rs/psicose/latest/psicose/trait.ByteSource.html) | App → bytes (`read_byte`). |
| [`ByteSink`](https://docs.rs/psicose/latest/psicose/trait.ByteSink.html) | Bytes → app (`write_byte`). |
| [`RetryPolicy`](https://docs.rs/psicose/latest/psicose/struct.RetryPolicy.html) | Retransmissões por frame antes de falhar. |
| [`IdleBudget`](https://docs.rs/psicose/latest/psicose/struct.IdleBudget.html) | Detector de hang no loop externo. |
| [`Error`](https://docs.rs/psicose/latest/psicose/enum.Error.html) | Erro de transporte / protocolo da crate. |

---

## Frame (envelope de 4 bytes)

| Nome | O que faz |
| --- | --- |
| [`Frame`](https://docs.rs/psicose/latest/psicose/struct.Frame.html) | Codifica/decodifica `TYPE ‖ SEQ ‖ DATA ‖ CRC`. |
| [`FrameType`](https://docs.rs/psicose/latest/psicose/enum.FrameType.html) | DATA, ACK, NACK, START, FINISH, ABORT. |
| [`FrameAssembler`](https://docs.rs/psicose/latest/psicose/struct.FrameAssembler.html) | Monta frame byte a byte. |
| [`FrameError`](https://docs.rs/psicose/latest/psicose/enum.FrameError.html) | Tipo / CRC / semântica inválidos. |
| [`Sequence`](https://docs.rs/psicose/latest/psicose/struct.Sequence.html) | Contador `SEQ` `u8` com wrap. |
| [`crc8`](https://docs.rs/psicose/latest/psicose/fn.crc8.html) | CRC-8 sobre `TYPE‖SEQ‖DATA`. |
| `FRAME_LEN` | Sempre 4. |
| `PAYLOAD_LEN` | Sempre 1 (byte DATA). |
| `CRC8_POLY` | Polinômio. |

---

## Helpers de bytes da aplicação (`stream`)

| Nome | O que faz |
| --- | --- |
| [`SliceSource`](https://docs.rs/psicose/latest/psicose/struct.SliceSource.html) | `&[u8]` como `ByteSource`. |
| [`SliceSink`](https://docs.rs/psicose/latest/psicose/struct.SliceSink.html) | `&mut [u8]` como `ByteSink`. |
| `send_bytes` / `recv_bytes` | Helpers one-shot sobre uma pump. |
| `send_all` / `recv_all` | Drena source / enche sink stop-and-wait. |
| `send_all_windowed` / `recv_all_windowed` | Igual com janela. |
| variantes `*_budgeted` | Igual com `IdleBudget`. |
| `SliceFull` / `StreamError` | Sink cheio / erros de stream. |

---

## Cripto

Não está nesta crate. Sele mensagens da aplicação antes do transporte se
precisar. `Capabilities::ENCRYPTION` / `SessionConfig::SECURE` são só bits
de anúncio no hello.

---

## Módulos (paths; nem todos no prelude)

| Módulo | Papel |
| --- | --- |
| `psicose::prelude` | Reexports para `use psicose::prelude::*`. |
| `psicose::p2p` | Node, links, hello, fragmentação. |
| `psicose::protocol` | Frame, CRC, assembler. |
| `psicose::pump` / `tx` / `rx` / `window` | Máquinas de confiabilidade. |
| `psicose::transport` | `ByteTransport`, `LinkFace`. |
| `psicose::stream` | Slice source/sink + loops send/recv. |
| `psicose::timeout` | Retry / idle. |
| `psicose::error` | `Error`. |
| `psicose::actors` | Scheduler cooperativo multi-link (avançado). |
| `psicose::fault` | Transportes com falha para testes. |

---

## Opções de SessionConfig / Capabilities

**Presets**

| Preset | Significado |
| --- | --- |
| `SessionConfig::DEFAULT` | versão 1, janela 8, `STREAM` |
| `SessionConfig::FORUM` | janela 8 + `STREAM` + `WINDOW` + `FORUM` + `FRAGMENTATION` |
| `SessionConfig::SECURE` | `FORUM` + `ENCRYPTION` (você sela fora da psicose) |
| `SessionConfig::offer(w, caps)` | Janela + bits customizados |

**Bits de capability** (negociado = interseção / min da janela)

| Bit | Significado |
| --- | --- |
| `WINDOW` | Após hello, `PeerLink` pode pipelinar (`max_window`) |
| `STREAM` | Camada stream/mensagem pretendida |
| `FORUM` | Bit de mensagens estilo fórum |
| `FRAGMENTATION` | Espera Fragmenter/Defragmenter |
| `ENCRYPTION` | Peer anuncia cripto na app (ainda deve selar fora) |
| `COMPRESSION` | Bit reservado de anúncio |

---

## Exemplos mínimos

Só transporte:

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

Rodar: `cargo run --example ab_direct`, `cargo run --example p2p_pair`.
Formato do fio: [PROTOCOL.pt-BR.md](PROTOCOL.pt-BR.md).
