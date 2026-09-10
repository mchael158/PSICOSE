# PSICOSE-1B

[![crates.io](https://img.shields.io/crates/v/psicose.svg)](https://crates.io/crates/psicose)
[![docs.rs](https://docs.rs/psicose/badge.svg)](https://docs.rs/psicose)
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
psicose = "0.2"
```

Sem features, sem dependências. MSRV: Rust 1.75.

Docs: [docs.rs/psicose](https://docs.rs/psicose) · spec:
[`PROTOCOL.md`](PROTOCOL.md) ([pt-BR](PROTOCOL.pt-BR.md))

## Por quê

A maioria dos protocolos segura a mensagem inteira na memória. A PSICOSE
não: move um byte de payload por vez. A memória é alguns frames na
stack — 4 bytes de config ou um arquivo de 4 GB.

## Estado (0.2.1 — experimental)

- **protocol** — CRC-8, frame de 4 bytes, validação semântica, wraparound
  de `SEQ`, assembler, `OutBuf`.
- **tx / rx** — stop-and-wait. `START` / `FINISH` esperam ACK.
- **window** — selective-repeat `N ≤ 8`. Aliases: `W8Sender`, `W8Receiver`.
- **transport** — `ByteTransport` / `ByteSource` / `ByteSink`.
- **stream** — qualquer byte pelo envelope: `SliceSource` / `SliceSink`,
  `send_all` / `recv_all` (stop-and-wait e janelado). JPEG, arquivo,
  flash, sensor ou bytes de um `struct` são só um `ByteSource`. O frame
  não é o tipo da aplicação.
- **fault** — `FaultyTransport`.
- **actors** — `System` cooperativo.

Ainda não: SessionId no frame, UART/SPI/CAN/rádio, `File`.

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

## Exemplo

```rust
use psicose::Frame;

let frame = Frame::data(0, 0xAA);
assert_eq!(Frame::from_bytes(frame.to_bytes()).unwrap(), frame);
```

## Testes

```sh
cargo +1.75.0 test
```

## Licença

Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) ou MIT
([LICENSE-MIT](LICENSE-MIT)), à sua escolha.
