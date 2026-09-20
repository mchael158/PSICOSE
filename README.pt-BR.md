# PSICOSE-1B

[![crates.io](https://img.shields.io/crates/v/psicose.svg)](https://crates.io/crates/psicose)
[![docs.rs](https://docs.rs/psicose/badge.svg)](https://docs.rs/psicose/latest/psicose/)
[![CI](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml/badge.svg)](https://github.com/mchael158/PSICOSE/actions/workflows/ci.yml)
[![no_std](https://img.shields.io/badge/no__std-yes-brightgreen.svg)](https://docs.rs/psicose)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/mchael158/PSICOSE)
[![license](https://img.shields.io/crates/l/psicose.svg)](https://crates.io/crates/psicose)

[English](README.md) · [Português (Brasil)](README.pt-BR.md)

**Framework `no_std` para links confiáveis** — fio + pump + janela + **P2P**,
RAM constante na stack, zero deps no build padrão.

Payload = **1 byte**. Frame = **4 bytes**. P2P (`Node`, `PeerLink`, hello,
fragmentação) está **nesta crate**, no mesmo frame — não é um pacote à parte.
Os exemplos chamam este motor; não inventam P2P externo nem passam conexão.

```text
┌──────┬─────┬─────┬───────┐
│ TYPE │ SEQ │ DATA│  CRC  │   ← frame = 4 bytes
└──────┴─────┴─────┴───────┘
                   ▲
                   └── payload da aplicação: exatamente 8 bits
```

Eficiência máxima antes dos ACKs: 1/4 = 25%. Stop-and-wait (DATA+ACK): 1/8 = 12,5%.

```toml
[dependencies]
psicose = "0.3"   # zero dependências — API só psicose::…
```

| | |
| --- | --- |
| Dependências | **nenhuma** (sempre) |
| MSRV | Rust 1.75 |
| Segurança | `#![no_std]` · `#![forbid(unsafe_code)]` · sem heap |
| Docs | [docs.rs/psicose](https://docs.rs/psicose/latest/psicose/) |
| Spec | [`docs/PROTOCOL.md`](docs/PROTOCOL.md) · [pt-BR](docs/PROTOCOL.pt-BR.md) |
| Mapa da API | [`docs/API.md`](docs/API.md) · [pt-BR](docs/API.pt-BR.md) |
| Hardware | [`docs/HARDWARE.md`](docs/HARDWARE.md) · [pt-BR](docs/HARDWARE.pt-BR.md) |

## No hardware (ESP32)

```text
UART1 → seu ByteTransport → LinkFace → Pump / Node
Wi‑Fi TCP → TcpPipe / TcpStream → LinkFace → Pump / Node
```

Firmware de exemplo (não publicado):

- UART: [`boards/esp32-uart`](boards/esp32-uart/) — TX=GPIO17, RX=GPIO16 @ 115200
- Wi‑Fi/TCP: [`boards/esp32-wifi`](boards/esp32-wifi/) — STA + DHCP + TCP → PSICOSE

`esp-hal` / `esp-radio` vivem **só** nesses pacotes de board.

```sh
cd boards && cargo run -p esp32-uart
# ou: cd boards && cargo run -p esp32-wifi   # precisa SSID/PASSWORD/HOST
```

Smoke TCP no host (sem placa): `cargo run --example tcp_pair`.

`Wire` é **harness de teste em memória**, não o frame no fio.

## Experimente em 30 segundos (host)

```sh
cargo run --example ab_direct    # motor: Wire::copy
cargo run --example p2p_pair     # motor: Node + PeerLink
cargo run --example tcp_pair     # motor: TCP + LinkFace
cargo run --example jpeg_over_uart
cargo test
```

## Quando usar

- UART / SPI / rádio com pouca RAM — sem bufferar a mensagem inteira
- Flash de firmware, JPEG de câmera, telemetria, dump de EEPROM
- Stop-and-wait ou selective-repeat (`N ≤ 8`) com CRC + retry na stack
- Cripto opcional **fora** da psicose se o link puder ser adversarial

## Quando não usar

- Precisa de alto throughput / frames grandes (por desenho: 1 DATA byte/frame)
- Precisa de roteamento, mesh ou gossip (fora desta crate)
- Só precisa do driver UART — implemente `ByteTransport`; a PSICOSE fica acima

## Por que existe

A maioria dos stacks segura a mensagem inteira na memória. A PSICOSE não:
a memória é alguns frames na stack, sejam 4 bytes de config ou um arquivo
enorme. Confiabilidade (ACK, NACK, SEQ, CRC-8, retry) é do protocolo.

CRC-8 é **detecção de ruído**, não autenticação. Sele mensagens na **sua**
aplicação antes do transporte se houver atacante no fio. Veja [`SECURITY.md`](SECURITY.md).

## Estado (0.4 — utilizável)

- **protocol** — CRC-8, frame 4 bytes, wraparound de `SEQ`, assembler
- **tx / rx** — stop-and-wait; `START` / `FINISH` / `ABORT` esperam ACK
- **pump** — `Pump` cooperativa + `SessionStats` (sem loop interno)
- **window** — selective-repeat `1 ≤ N ≤ 8`
- **stream** — `SliceSource` / `SliceSink`, `send_all` / `recv_all`
- **p2p** — `Node`, `PeerLink`, hello, fragmentação (`prelude`)
- **LinkFace** — demux de uma porta física em TX/RX do Pump (caminho hardware)
- **dependências** — **nenhuma**

Fora da crate: HALs de SoC (`esp-hal` fica em `boards/`), `File`, roteamento / gossip, cripto.

## Camadas (um framework)

```text
 exemplos / aplicação / boards/esp32-uart | boards/esp32-wifi
        │
        ▼
 psicose::Node                              ← entrada do motor
        ├─ PeerLink / PeerTable / hello
        ├─ Fragmenter / Defragmenter
        ▼
 LinkFace / Wire::copy / Pump / WindowedPump
        │  DATA 1 byte/frame + CRC-8 + ACK
        ▼
 ByteTransport
        ├─ você implementa no UART / SPI / rádio
        └─ Wire                             (só harness de teste)
```

Hardware (`LinkFace` + seu `ByteTransport`) ou P2P (`Node`) — mesma crate,
mesmo frame. `Wire` é para testes no host. `Capabilities::WINDOW` é honrado
pelo `PeerLink` após o hello.

## API (resumo)

```rust
use psicose::prelude::*;
```

| Você escreve | Significado |
| --- | --- |
| `PeerId::from_label(b"alice")` | Identidade 8 bytes. Nunca no frame de 4. |
| `Node::<4>::new(id)` | Entrada do framework: identidade + tabela. |
| `Node::with(id, SessionConfig::FORUM)` | Hello com bits de app. |
| `SessionConfig::SECURE` | `FORUM` + `ENCRYPTION` (anuncia cripto da app; você sela). |
| `Fragmenter` / `Defragmenter` | Corta / remonta blob na stack. |
| `Wire::new().copy(src, sink)` | Harness host stop-and-wait A→B (testes). |
| `LinkFace::new(port).pump()` | Hardware: demux de um UART nos extremos do Pump. |
| `Wire::new().link_pumps()` | A↔B em memória para `PeerLink` (janelado). |
| `Pump::on(tx, rx)` | Mesmo envelopamento no hardware. |
| `node.connect` / `node.accept` | START + hello 12 bytes. |
| `establish` / `send_message` | Helpers cooperativos **dentro** da crate. |

```rust
use psicose::prelude::*;

let wire = Wire::new();
let (pump_a, pump_b) = wire.link_pumps();

let mut alice = Node::<4>::new(PeerId::from_label(b"alice"));
let mut bob = Node::<4>::new(PeerId::from_label(b"bob"));

let mut a = match alice.connect(pump_a) {
    Ok(link) => link,
    Err(_) => return,
};
let mut b = bob.accept(pump_b);
assert!(establish(&mut a, &mut alice, &mut b, &mut bob));
```

Rodar: `cargo run --example p2p_pair`. Só transporte: `ab_direct`.
Testes: `tests/forum.rs`.

## Exemplos

```sh
cargo run --example jpeg_over_uart
cargo run --example firmware_flash
cargo run --example sensor_telemetry
cargo run --example radio_windowed
cargo run --example ab_direct
cargo run --example p2p_pair
```

| Example | Camada | O que a PSICOSE vê |
| --- | --- | --- |
| `ab_direct` | transporte (`Pump`) | bytes `ping` / `pong` |
| `p2p_pair` | motor P2P (`Node` + `PeerLink`) | hello + bytes fragmentados |
| `jpeg_over_uart` | transporte | bytes JPEG |
| `firmware_flash` | transporte | bytes do arquivo |
| `sensor_telemetry` | transporte | 8 bytes |
| `radio_windowed` | transporte janelado | bytes, janela 8 |

## Não negociável

- `#![no_std]`, `#![forbid(unsafe_code)]`
- Sem `Vec` / `String` / `Box` / allocator / async runtime
- Todo tipo público tem tamanho conhecido em compile time

## Contribuir / segurança

- [`CONTRIBUTING.md`](CONTRIBUTING.md)
- [`SECURITY.md`](SECURITY.md)
- [`CHANGELOG.md`](CHANGELOG.md)

## Licença

Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) ou MIT ([LICENSE-MIT](LICENSE-MIT)),
à sua escolha.
