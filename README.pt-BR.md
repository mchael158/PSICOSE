# PSICOSE-1B

[![crates.io](https://img.shields.io/crates/v/psicose.svg)](https://crates.io/crates/psicose)
[![docs.rs](https://docs.rs/psicose/badge.svg)](https://docs.rs/psicose)
[![CI](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml/badge.svg)](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/psicose.svg)](https://crates.io/crates/psicose)

[English](README.md) · [Português (Brasil)](README.pt-BR.md)

Protocolo de transporte `no_std`, sem heap, determinístico e orientado a
byte. **O payload é 1 byte. O frame no fio é 4 bytes.** Não é a mesma
coisa.

```text
┌──────┬─────┬─────┬───────┐
│ TYPE │ SEQ │ DATA│  CRC  │   ← frame = 4 bytes (overhead + payload)
└──────┴─────┴─────┴───────┘
                   ▲
                   └── payload da aplicação: exatamente 8 bits
```

Eficiência máxima antes dos ACKs: 1/4 = 25%. Stop-and-wait (DATA+ACK):
1/8 = 12,5%.

```toml
[dependencies]
psicose = "0.3"
# Criptografia autenticada opcional acima do transporte:
# psicose = { version = "0.3", features = ["aead"] }
```

Build padrão: **sem features, sem dependências.** Feature opcional `aead`
adiciona ChaCha20-Poly1305 (RFC 8439). MSRV: Rust 1.75 (`rust-toolchain.toml`).

Docs: [docs.rs/psicose](https://docs.rs/psicose) · spec:
[`docs/PROTOCOL.md`](docs/PROTOCOL.md) ([pt-BR](docs/PROTOCOL.pt-BR.md))

## Por quê

A maioria dos protocolos segura a mensagem inteira na memória. A PSICOSE
não: move um byte de payload por vez. A memória é alguns frames na
stack — 4 bytes de config ou um arquivo de 4 GB.

## Estado (0.3 — transporte utilizável)

Transporte de byte confiável para links embutidos. CRC-8 é **detecção de
ruído**, não autenticação — use `aead` se o link puder ser adversarial.

- **protocol** — CRC-8, frame de 4 bytes, validação semântica, wraparound
  de `SEQ`, assembler, `OutBuf`.
- **tx / rx** — stop-and-wait. `START` / `FINISH` / `ABORT` esperam ACK.
- **pump** — `Pump` cooperativa (`rx.poll` depois `tx.poll`, sem loop
  interno) + `SessionStats` (`bytes_delivered`, `frames_sent`,
  `retries`, `nacks`, `duplicates`, `crc_errors`, `ticks`).
- **window** — selective-repeat `N ≤ 8`. Aliases: `W8Sender`, `W8Receiver`.
- **transport** — `ByteTransport` / `ByteSource` / `ByteSink`.
- **stream** — qualquer byte pelo envelope: `SliceSource` / `SliceSink`,
  `send_all` / `recv_all` (stop-and-wait e janelado). JPEG, arquivo,
  flash, sensor ou bytes de um `struct` são só um `ByteSource`. O frame
  não é o tipo da aplicação.
- **fault** — `FaultyTransport`.
- **actors** — `System` cooperativo.
- **p2p** — mesma crate, mesmo orçamento. `use psicose::prelude::*`. Veja abaixo.

Ainda não: roteamento / gossip / store-and-forward, UART/SPI/CAN/rádio,
`File`.

## Camadas

```text
 bytes da aplicação
        │
        ├─ opcional: aead::seal_to / open_from   (feature = "aead")
        ├─ opcional: Fragmenter / Defragmenter
        ▼
 PeerLink / Pump / Sender|Receiver|Windowed*
        │  DATA 1 byte/frame + CRC-8 + ACK
        ▼
 ByteTransport  (você implementa: UART / SPI / rádio)
```

Você empilha as camadas. Bits de `Capabilities` anunciam intenção; não
ligam sozinhos janela ou AEAD no `PeerLink`.

## API

```rust
use psicose::prelude::*;
```

| Você escreve | Significado |
| --- | --- |
| `PeerId::from_label(b"alice")` | Identidade de 8 bytes (com padding). Nunca no frame de 4 bytes. |
| `PeerTable::<4>::new(id)` | Vizinhos, `1 ≤ N ≤ 8`. Default: janela 8 + `STREAM`. |
| `PeerTable::with(id, SessionConfig::FORUM)` | Hello: `STREAM` + `WINDOW` + `FORUM` + `FRAGMENTATION`. |
| `SessionConfig::SECURE` | `FORUM` + `ENCRYPTION` (anuncia AEAD; ainda chama `seal_to`). |
| `StreamId::FORUM` / `MessageId::new(1)` | Conversa da aplicação. Não é `SEQ`. |
| `Fragmenter` / `Defragmenter` | Corta qualquer blob; remonta num buffer na stack. |
| `seal_to` / `open_from` | Feature `aead`: ciphertext ‖ tag acima do transporte. |
| `SessionConfig::offer(4, features)` | Config do hello. A versão é `PROTOCOL_VERSION`. |
| `Capabilities::*` | Só anúncios. CRC não se negocia. |
| `Wire::new().pumps()` | A↔B na memória. ACK/NACK → TX, DATA/START/FINISH/ABORT → RX. |
| `Pump::on(tx, rx)` | O mesmo envelopamento num UART / SPI / rádio. |
| `PeerLink::connect` / `accept` | START + hello de 12 bytes nos dois sentidos. |
| `link.offer(byte)` | DATA depois de `Established`. |
| `link.poll(&mut table)` | Um passo cooperativo. Nunca entra em loop. |
| `PollOutcome::is_closed()` | FINISH ou ABORT encerrou a transferência. |

O hello no fio tem 12 bytes de payload: `PeerId (8) | ver (1) | janela (1) | features (2)`.
`PeerSession` é só contabilidade (≤ 128 B). `StreamId` / `MessageId` não
são `SEQ`.

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

O mesmo link move bytes A→B. Isso não é um fórum — só payload:

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

Executável: `cargo run --example forum`. Teste: `tests/forum.rs`.

Para outro perfil:
`PeerTable::with(id, SessionConfig::offer(4, Capabilities::STREAM | Capabilities::WINDOW))`.

O envelope de 4 bytes continua só isto:

```rust
use psicose::Frame;

let frame = Frame::data(0, b'A');
assert_eq!(Frame::from_bytes(frame.to_bytes()), Ok(frame));
```

## Inegociáveis

- `#![no_std]`, `#![forbid(unsafe_code)]`
- Sem `Vec`, `String`, `Box`, `Rc`/`Arc`, sem alocador, sem runtime async
- Todo tipo público tem tamanho conhecido em compile time
- Confiabilidade é trabalho do protocolo, nunca da aplicação

## Qualquer dado, não só frame

O frame de 4 bytes é o **envelope**. O dado da aplicação é sempre byte:

```text
JPEG  Arquivo  Flash  Sensor  firmware.bin  o seu struct
  └────────┴───────┴────────┴──────────────┘
                    │
               ByteSource     ← você implementa
                    │ 1 byte por vez
                    ▼
                 PSICOSE      ← nunca possui o blob
                    │
                ByteSink
```

`psicose::File` não está nesta crate. Um JPEG e um arquivo de 4 GB
usam o mesmo caminho: implemente `ByteSource` / `ByteSink` (ou use
`SliceSource` / `SliceSink` quando o buffer já é seu).

## Exemplos

Problemas reais, executáveis. O "UART" e o "rádio" são anéis na
memória; no campo você troca o `End` pelo driver.

```sh
cargo run --example jpeg_over_uart
cargo run --example firmware_flash
cargo run --example sensor_telemetry
cargo run --example radio_windowed
cargo run --example forum
```

| Exemplo | Problema | O que a PSICOSE vê |
| --- | --- | --- |
| `jpeg_over_uart` | RAM de câmera OV2640 → arquivo no host, via UART | bytes JPEG |
| `firmware_flash` | `firmware.bin` no host → flash NOR do MCU, 1 byte programado por vez | bytes do arquivo |
| `sensor_telemetry` | DHT22 + ADC de bateria, struct empacotado no rádio lento | amostra de 8 bytes |
| `radio_windowed` | dump de 300 bytes de EEPROM; o primeiro DATA some | bytes, janela `N=8` |
| `forum` | Alice ↔ Bob: handshake, depois bytes nos dois sentidos | bytes de payload |

`firmware_flash` implementa `ByteSource` em `std::fs::File` — é o
padrão para um binário de 4 GB. A crate continua sem possuir o arquivo.

## Testes

```sh
cargo test
```

Ver `tests/end_to_end.rs`, `tests/hostile.rs`, `tests/windowed.rs`,
`tests/pump.rs`, `tests/p2p.rs` e `tests/forum.rs`.

## Licença

Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) ou MIT
([LICENSE-MIT](LICENSE-MIT)), à sua escolha.
