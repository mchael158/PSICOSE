# PSICOSE no hardware

[English](HARDWARE.md) · [Português (Brasil)](HARDWARE.pt-BR.md)

A PSICOSE é um **motor de link confiável** com **zero dependências de crate**.
No board **você é dono** do HAL; a PSICOSE **é dona** do frame, ACK/CRC, janela e P2P.

## Caminhos oficiais (ESP32 clássico)

### UART

```text
aplicação
     │
     ▼
Pump / Node / PeerLink
     │
     ▼
LinkFace          ← demux de um RX em extremos TX + RX do Pump
     │
     ▼
ByteTransport     ← você implementa (wrapper local em torno do UART)
     │
     ▼
esp-hal UART1     ← só em boards/esp32-uart — não entra na psicose
```

### Wi‑Fi / TCP

```text
aplicação
     │
     ▼
Pump / Node / PeerLink
     │
     ▼
LinkFace
     │
     ▼
ByteTransport     ← TcpPipe (anéis) ou wrapper de TcpStream no board
     │
     ▼
embassy-net TCP ← STA / DHCP / scan ficam em boards/esp32-wifi
```

| Peça | Onde |
| --- | --- |
| `LinkFace`, `Pump`, `Node`, … | Sempre em `psicose` (zero deps) |
| Impl de `ByteTransport` | Seu firmware / pacote de board |
| `esp-hal` UART | [`boards/esp32-uart`](../boards/esp32-uart/) |
| `esp-radio` + Embassy TCP | [`boards/esp32-wifi`](../boards/esp32-wifi/) |

Smoke no host sem placa: `cargo run --example tcp_pair`.

Testes A↔B em memória usam [`Wire`](https://docs.rs/psicose/latest/psicose/type.Wire.html).

## Pinout (firmware UART de exemplo)

| Sinal | GPIO |
| --- | --- |
| UART1 TX | 17 |
| UART1 RX | 16 |
| Baud | 115200 |

Veja [`boards/esp32-uart/README.md`](../boards/esp32-uart/README.md) e
[`boards/esp32-wifi/README.md`](../boards/esp32-wifi/README.md).

## Esboço mínimo

```rust,ignore
use psicose::{ByteTransport, LinkFace};

struct MeuUart(/* seu tipo HAL */);
impl ByteTransport for MeuUart { /* write_byte / read_byte */ }

let face = LinkFace::new(MeuUart(/* … */));
let mut turn = face.turn::<8>(true); // o outro par: false
let _ = turn.push(b"ping");
loop { let _ = turn.poll(); }
```

Grave `FIRST` em exatamente um par. O outro responde no turno dele
(`psicose::Turn`). UART: `FIRST` oposto nos dois boards. Wi-Fi: o board
abre com `ping`, e `turn_pair --listen` responde `pong`. Anel RX cheio
espera. Veja os READMEs dos boards.

## Fora de escopo aqui

BLE, NVS, OTA, outros chips — pacotes de board separados.
Cripto também fica fora da psicose: sele na aplicação se precisar.
