# PSICOSE-1B — protocolo formal (0.3.1)

[English](PROTOCOL.md) · [Português (Brasil)](PROTOCOL.pt-BR.md)

Máquina de transporte `no_std`, sem heap. Este arquivo é a especificação.
O código em `src/` é a implementação. Se os dois divergirem, o teste
adversarial em `tests/hostile.rs` decide.

**Estado:** transporte de byte confiável mais camada P2P no mesmo frame
de 4 bytes. Entrada do framework: `psicose::Node`. Feature `aead`
adiciona ChaCha20-Poly1305 **acima** do transporte. Mapa nome a nome da
API: [API.pt-BR.md](API.pt-BR.md) (English: [API.md](API.md)).

## Modelo de ameaça

| Garantia | Mecanismo |
|----------|-----------|
| Detectar erros acidentais de bit no frame | CRC-8 sobre `TYPE‖SEQ‖DATA` |
| Entregar bytes em ordem, no máximo uma vez | SEQ + ACK/NACK + retransmissão |
| Limitar hang com peer silencioso | `RetryPolicy` (por frame) + `IdleBudget` (loops externos) |
| Confidencialidade / autenticidade vs atacante ativo | **Não** é o CRC. Feature `aead` (`psicose::aead`) nas mensagens da aplicação |

Um adversário que injeta ou altera bytes no link pode forjar um frame
com CRC válido. Trate o CRC só como proteção contra ruído.

```
                    APLICAÇÃO / exemplos
                         │
              ┌──────────▼──────────┐
              │   psicose::Node     │
              │ PeerLink / Session  │
              │ Stream / Message    │
              └──────────┬──────────┘
                         │ bytes de payload
              ┌──────────▼──────────┐
              │      transporte     │
              │  ACK / NACK / CRC   │
              │  START / FINISH /   │
              │  ABORT / Pump       │
              └──────────┬──────────┘
                         │
                   ByteTransport
                         │
          ┌──────────────┼──────────────┐
          │              │              │
        UART           TCP/UDP        Rádio
```

O transporte não sabe o que é peer, stream ou post de fórum. A camada
P2P não aumenta o frame.

## 1. Unidades

| Nome | Tamanho | Papel |
|------|---------|--------|
| **payload** | 1 byte (8 bits) | Dado da aplicação. Única coisa que `ByteSource` / `ByteSink` vê. |
| **frame** | 4 bytes | Unidade no fio. Inclui overhead de transporte. |
| **SEQ** | `u8` | Contador mod 256. Wrap `255 → 0` é comportamento, não erro. |

Dizer "PSICOSE transmite 1 byte" refere-se ao **payload**. O **frame** é
sempre 4 bytes.

```
eficiência de frame (antes de ACK)    = PAYLOAD_LEN / FRAME_LEN = 1/4 = 25%
eficiência stop-and-wait (DATA+ACK)   = 1 / 8                   = 12,5%
```

O modo janelado (`window::WindowedSender<_, N>`, `N ≤ 8`, sem heap)
sobe essa razão. Não muda o frame.

Dogma: um arquivo de 4 GB deve atravessar a PSICOSE sem a PSICOSE jamais
precisar possuir mais que alguns bytes dele em memória.

## 2. Frame

```
offset  0        1        2        3
      ┌────────┬────────┬────────┬────────┐
      │  TYPE  │  SEQ   │  DATA  │  CRC   │
      └────────┴────────┴────────┴────────┘
        overhead  overhead  payload  overhead
```

| TYPE | Valor | SEQ | DATA |
|------|-------|-----|------|
| Data | `0x01` | sequência do byte | payload da aplicação |
| Ack | `0x02` | sequência reconhecida | `0x00` |
| Nack | `0x03` | sequência rejeitada | `0x00` |
| Start | `0x04` | `0x00` | `0x00` |
| Finish | `0x05` | sequência atual do TX | `0x00` |
| Abort | `0x06` | `0x00` | `0x00` |

CRC-8: poly `0x07`, init `0x00`, sem reflexão, sem xor-out, sobre
`TYPE || SEQ || DATA`.

### Validade semântica

CRC válido ≠ semanticamente válido. Um `Frame` só existe se **os dois**
valerem.

Frames de controle (`ACK`, `NACK`, `START`, `FINISH`, `ABORT`) devem ter
`DATA = 0`. `START` e `ABORT` devem ter `SEQ = 0`.

Os construtores públicos são `Frame::data`, `Frame::ack`, `Frame::nack`,
`Frame::start`, `Frame::finish` e `Frame::abort`. Não há `Frame::new` público.
`Frame::from_bytes` rejeita um frame com CRC válido que quebre essas
regras (`FrameError::InvalidSemantics`). Não há caminho `unchecked`.

## 3. Sequência

```
0 → 1 → … → 254 → 255 → 0
```

`previous(0) == 255`. O wraparound faz parte do protocolo, não é erro.

## 4. Sessão (START / DATA / FINISH / ABORT)

Uma transferência é uma sessão explícita:

```
TX                              RX
──                              ──
START  ─────────────────────►   expected_seq = 0
       ◄─────────────────────   ACK 0
DATA 0 ─────────────────────►   entrega, ACK 0
DATA 1 ─────────────────────►   entrega, ACK 1
…                               …
FINISH seq=k ───────────────►   ACK k, depois TransferFinished
       ◄─────────────────────   ACK k
```

`START` pode chegar no meio do fluxo. O receptor zera `expected_seq` e
confirma com ACK 0. Depois desse ACK, o sender também zera a sequência
de DATA.

`FINISH` não é fire-and-forget. O `SEQ` **tem** de ser igual ao
`expected_seq` do receptor (a próxima sequência de DATA não usada). Um
`FINISH` com outro `SEQ` recebe NACK e **não** fecha a sessão — é
melhor falhar do que perder o fim de um arquivo.

O sender só chega em `Finished` depois de `ACK(seq do FINISH)`. Um
`FINISH` perdido é retransmitido. Um ACK de FINISH perdido também: o
receptor reenvia ACK do mesmo `FINISH` e não reabre a transferência.

Depois de `Finished`:

- `DATA` atrasado é ignorado (não entrega, não NACKa)
- frame corrompido é ignorado (não NACKa)
- `FINISH` com a sequência fechada é reconfirmado
- `ABORT` é ignorado (a sessão já está fechada)
- `START` abre uma sessão nova

`ABORT` é um frame de primeira classe (`TYPE = 0x06`). O envelope não
cresce. Não existe cancelamento fora do frame: todo evento da sessão
é o mesmo fluxo `TYPE | SEQ | DATA | CRC`.

```
TX                              RX
──                              ──
ABORT  ─────────────────────►   ACK 0, cancela, descarta DATA pendente
       ◄─────────────────────   ACK 0
```

`ABORT` é confiável como `FINISH`: o sender espera `ACK(0)`. Um `ABORT`
perdido é retransmitido. Um `ABORT` duplicado é reconfirmado.

Semântica:

- `offer_abort` no TX descarta qualquer DATA/START/FINISH em voo,
  inclusive no meio da escrita ou do retry, e envia `ABORT` (`SEQ = 0`).
- Um `ABORT` recebido durante retransmissão ganha: o DATA em voo é
  descartado.
- O RX confirma com ACK 0, trava o estado abortado e zera
  `expected_seq`.
- Depois do abort: DATA atrasado e frames corrompidos são ignorados
  (mesmo latch do FINISH). `START` reabre a sessão em seq 0, inclusive
  no wrap `255 → 0`.
- Um `ABORT` do peer visto no caminho de controle do TX cancela localmente
  sem enviar um segundo `ABORT`. Quem escreve o ACK é o RX do mesmo
  endpoint.

Um identificador de sessão **não** entra no frame de 4 bytes. Uma
versão futura pode negociá-lo com frames extras depois do `START`. Não
aumente o frame para isso.

I/O é um byte por poll dos dois lados. A aplicação só vê `Delivered`
depois que o ACK correspondente foi escrito por completo. Um frame só
está "no fio" quando os quatro bytes saíram do `OutBuf`.

## 5. Stop-and-wait (DATA)

No máximo **um** DATA em voo.

```
TX                              RX
──                              ──
DATA seq=k  ─────────────────►  se CRC/tipo/semântica falha: NACK expected, não entrega
                                se seq == expected:  ACK k, entrega, expected++
                                se seq == previous:  ACK k, NÃO entrega   ← crítico
                                senão:               NACK seq, não entrega
            ◄─────────────────  ACK / NACK
```

O ACK perdido é o caso que mais mente: o RX já avançou. O retransmit
**não** pode entregar o byte de novo. `DuplicateIgnored` + re-ACK.

CRC, TYPE ou semântica inválidos no RX **não** são erro da aplicação.
NACK do `expected` atual, continuar o poll. Falha de CRC devolve
`PollOutcome::CrcRejected`; `SEQ` errado ou TYPE/semântica inválidos
devolvem `PollOutcome::Rejected`. Os dois enviam NACK.

### ACK/NACK formal no TX

| Recebido enquanto espera | Ação |
|--------------------------|------|
| `ACK(seq atual)` | sucesso (avança / sessão pronta / terminou) |
| `NACK(seq atual)` | retransmitir agora |
| `ACK(seq diferente)` | ignorar; continuar esperando (zera ticks vazios) |
| `NACK(seq diferente)` | ignorar; continuar esperando (zera ticks vazios) |
| frame inválido | ignorar; esperar timeout / retransmitir |

## 6. Máquinas de estados

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
         ACK(cur)  NACK(cur)  TIMEOUT / ABORT do peer
             │        │        │
             ▼        └────┬───┘
           IDLE            │
      (Finished/Aborted)   ▼
                       RETRYING
                           │
                           ▼
                       SENDING
```

ACK/NACK de outra sequência e frames inválidos permanecem em
`WAIT_ACK`.

### Receiver

```
                 ┌─────────┐
                 │  IDLE   │
                 └────┬────┘
                      │ byte
                      ▼
                 ┌────────────┐
                 │ RECEIVING  │  (assembler enchendo)
                 └─────┬──────┘
                       │ 4º byte
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

Um `FINISH` duplicado depois de `FINISHED` é reconfirmado com ACK.
`START` depois de `FINISHED` ou `ABORTED` abre uma sessão nova. `ABORT`
depois de `FINISHED` é ignorado.

## 7. Injeção de falhas

`psicose::fault::FaultyTransport` envolve qualquer `ByteTransport` e
pode:

- **DROP DATA** — o RX não vê o frame; o TX estoura ticks e retransmite
- **CORRUPT** — CRC inválido → NACK → retransmit
- **DROP ACK** — o TX retransmite; o RX reconhece duplicate e não
  reentrega
- **DROP NACK** — tratado como silêncio; o TX estoura timeout e
  retransmite
- **DELAY DATA** — leituras devolvem `Ok(None)` por N ticks, depois o
  frame
- **DROP FINISH / ACK de FINISH** — FINISH é retransmitido até o ACK
- **ABORT durante DATA / retry** — o abort ganha; o DATA em voo some
- **ABORT depois de FINISH** — ignorado
- **START depois de ABORT** — sessão nova em seq 0, inclusive `255 → 0`

Nenhum desses modos pode corromper o fluxo visto pelo `ByteSink`.

## 7.1 Pump e SessionStats

A `Pump` é um passo cooperativo: `rx.poll()` e depois `tx.poll()`. Ela
nunca entra em loop dentro de `poll`. `Pump::send_all` é só esse loop
empilhado pelo caller. O `stream::send_all` scriptado ainda espera
ACKs no próprio transporte do sender.

`SessionStats` fica na pump (`Copy`, só stack, sem log):

| Campo | Significado |
|-------|-------------|
| `bytes_delivered` | Bytes de payload que o RX entregou depois de escrever o ACK |
| `frames_sent` | Frames cujos quatro bytes saíram do `OutBuf` do TX |
| `retries` | Vezes que o sender entrou em retransmissão |
| `nacks` | NACKs gerados pelo receptor |
| `duplicates` | DATA duplicado reconfirmado sem entregar |
| `crc_errors` | Rejeições de CRC (também contam em `nacks`) |
| `ticks` | Quantas vezes `Pump::poll` foi chamado |

`PumpEvent` de um passo, do mais alto: `Aborted` > `Completed` >
`Received(u8)` > `Sent` > `Progress` > `Idle`.

Falha de CRC devolve `PollOutcome::CrcRejected`; `SEQ` errado ou
TYPE/semântica inválidos devolvem `PollOutcome::Rejected`. Os dois
enviam NACK.

## 8. ByteSource / ByteSink

Não são o enlace. São a **aplicação**. O frame é só o envelope. A
PSICOSE não tem tipo JPEG, tipo arquivo ou tipo struct — isso tudo é
sequência de bytes.

```
JPEG  Arquivo  Flash  Sensor  firmware.bin  [u8] de um struct
  │       │      │       │         │              │
  └───────┴──────┴───────┴─────────┴──────────────┘
                      │
                 ByteSource
                      │ 1 byte de payload
                      ▼
                   PSICOSE          ← nunca possui o blob
                      │ Frame (4 B)
                      ▼
                 ByteTransport
                      │
                 ByteSink
```

`stream::send_all` / `recv_all` esvaziam uma source numa sink.
`SliceSource` / `SliceSink` emprestam um buffer que já é do caller.

`File` é uma implementação. A crate core não a inclui.

### 8.1 Feature `embedded-io` (opcional)

Não faz parte do formato no fio. Adaptadores em
`psicose::transport::embedded_io`:

- `IoTransport` — `embedded_io::{Read, Write, ReadReady}` → `ByteTransport`
  → `Pump::on` / `WindowedPump::on`
- `IoSource` / `IoSink` — `ByteSource` / `ByteSink` da aplicação

Fixado em **embedded-io 0.6** (MSRV 1.75). Sem a feature, implemente
`ByteTransport` você mesmo (como os examples em memória).

## 9. Memória de nó (futuro PSICOSE-8)

O protocolo de transporte já é uma máquina de registradores de 8 bits
(`SEQ`, `DATA`, CRC). A identidade de 256 bytes não foi abandonada:

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

Endereço = `u8`, dado = `u8`, memória = 256 bytes. Ainda não é uma VM.
`SessionStats` é a forma em software desses registradores.

## 10. Janela (`N ≤ 8`)

Selective repeat. O frame não muda. Memória:

```
TX:  [Option<Slot>; N]
RX:  [Option<u8>; N]     ← reorder, no máximo N bytes de payload
```

`1 ≤ N ≤ 8`, checado em compile time. Heap = 0. Sem `std`.

```
A janela de envio do TX é [oldest_unacked, oldest_unacked+N).
Slot livre não basta: o SEQ tem de ficar dentro dessa faixa.
RX aceita SEQ em [expected, expected+N) e guarda o payload.
RX reconfirma SEQ em [expected-N, expected) sem entregar.
RX entrega só o prefixo em ordem do buffer.
START / FINISH / ABORT continuam stop-and-wait (janela vazia para
FINISH). ABORT esvazia a janela. FINISH ainda exige SEQ == expected
(sem buracos).
```

`WindowFull` quer dizer: faça poll até um ACK liberar um slot, depois
ofereça de novo.

## 11. Camada P2P (módulo `p2p`)

Mesma crate. Mesmo orçamento `no_std` / sem heap / sem `unsafe`.
Identidade, sessões, streams e mensagens são **bytes de payload**. O
frame de 4 bytes não muda e nada dessa camada entra nele.

Comece por `psicose::prelude::*`. `PeerSession` nunca toca um
transporte: produz e consome bytes. O caller os move com `Pump`,
`send_bytes` ou qualquer outra coisa. `PeerSession` cabe em ≤ 128
bytes.

### 11.1 PeerId

Identidade de 64 bits (`[u8; 8]`). `PeerId::from_label(b"alice")`
completa (ou corta) um nome para 8 bytes. Bytes crus continuam
válidos: `PeerId::from([u8; 8])`. Como eles nascem (nome, aleatório,
hash de chave, serial) é assunto da aplicação. O frame físico não
carrega isso.

### 11.2 Hello (12 bytes de payload)

Enviado como DATA comum logo depois do START:

```
┌────────────┬─────────┬────────────┬──────────────┐
│ PeerId (8) │ ver (1) │ janela (1) │ features (2) │
└────────────┴─────────┴────────────┴──────────────┘
```

`SessionConfig` são os 4 últimos bytes: `ver | janela | features_hi |
features_lo`. Versão `0` e janela fora de `1..=8` são rejeitadas
(`HandshakeError`). `max_window` é limitado a `1..=8` no construtor.
`SessionConfig::DEFAULT` é versão 1, janela 8, `STREAM`.
`SessionConfig::offer(4, features)` preenche a versão.

### 11.3 Capabilities (`u16`, big-endian no hello)

CRC **não** é capability. O frame de transporte sempre o carrega.

| Bit | Nome | Significado |
|-----|------|-------------|
| 0 | `WINDOW` | Selective-repeat (`N ≤ 8`). O `PeerLink` sobe o limite de DATA para o `max_window` negociado após o hello. |
| 1 | `STREAM` | Streams lógicos sobre a sessão |
| 2 | `FORUM` | Mensagens da aplicação (não é um fórum) |
| 3 | `COMPRESSION` | **Reservado.** Só anúncio; sem compressão em 0.3.x |
| 4 | `ENCRYPTION` | Com feature `aead`: ChaCha20-Poly1305 acima do transporte. Sem `aead`: só anúncio |
| 5 | `FRAGMENTATION` | Fragmentação de mensagem (`Fragmenter`) |

`COMPRESSION` não altera o payload no fio. `ENCRYPTION` só tem sentido
quando os dois peers compilam com `aead` e selam os dados da aplicação
antes de oferecer bytes à PSICOSE. Bits desconhecidos ficam como estão
e morrem na interseção com um peer que não os liga.

A negociação é determinística e não tem rodada extra: versão mínima,
janela mínima, interseção dos bits. Os dois lados computam o mesmo
resultado. No código: `Capabilities::STREAM | Capabilities::WINDOW`.

### 11.4 Estados de PeerSession

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

| Estado | Contraparte no transporte |
|--------|---------------------------|
| `Disconnected` | Idle / depois do ACK de FINISH |
| `Connecting` | START em voo; nosso hello saiu |
| `Established` | Hellos cruzados; DATA pode fluir |
| `Closing` | FINISH em voo |
| `Aborted` | ABORT enviado ou recebido |

`on_hello` devolve `Ok(Some(reply))` no lado que aceita (precisa
enviar a resposta) e `Ok(None)` quando completa um connect que nós
iniciamos. `record_stats` copia `SessionStats` da pump.

### 11.5 Streams e mensagens

Não misture os contadores:

| Nome | Tamanho | Dono | Significado |
|------|---------|------|-------------|
| `SEQ` | `u8` | transporte | qual frame DATA |
| `StreamId` | `u8` | aplicação | qual conversa |
| `MessageId` | `u16` | aplicação | qual mensagem no stream |
| `fragment` | `u16` | aplicação | qual pedaço dessa mensagem |

O stream 0 é reservado para controle de sessão (`StreamId::CONTROL`).
O stream de dados da aplicação usual desta crate é `StreamId::FORUM`
(1) — um nome reserva, não um produto de fórum. Os outros mapeamentos
continuam política da aplicação.

Cabeçalho de mensagem — 7 bytes de payload por fragmento:

```
┌────────────┬────────────────┬───────────────┬───────────┬─────────┐
│ stream (1) │ message id (2) │ fragmento (2) │ flags (1) │ len (1) │
└────────────┴────────────────┴───────────────┴───────────┴─────────┘
```

Bit 0 das flags = último fragmento. Os demais bits são reservados e
rejeitados (`HeaderError::Flags`). Ids em big-endian.

O `Fragmenter` empresta o payload do caller e devolve
`(cabeçalho, pedaço)` até o último fragmento. O `Defragmenter` escreve
esses bytes de volta num buffer do caller, um byte de payload por vez
(`push`). Tamanho de pedaço `0` vale 1. Payload vazio ainda gera um
fragmento vazio e último, para o receptor ver que a mensagem existe.
Um blob de 4 GB e um post de 11 bytes usam o mesmo iterador.

### 11.6 Node, PeerTable, PeerLink, Wire

**Prefira [`Node`](https://docs.rs/psicose/latest/psicose/struct.Node.html)**
como entrada do framework. Ele é dono de um `PeerTable` e abre `PeerLink`s.

| Chamada | Significado |
|---------|-------------|
| `Node::<N>::new(id)` | Nó vazio. Oferece `SessionConfig::DEFAULT`. |
| `Node::with(id, cfg)` | Igual, config explícito (`FORUM`, `SECURE`, …). |
| `node.connect(pump)` | Reserva slot + START + hello (saída). |
| `node.accept(pump)` | Espera hello e responde (entrada). |
| `establish(&mut a, &mut alice, &mut b, &mut bob)` | Poll até ambos Established. |
| `send_message(...)` | Fragmenta e envia um corpo pelo par. |

`PeerTable<N>` é `[Option<PeerEntry>; N]` com `1 ≤ N ≤ 8`. Sem `Vec`.
Em geral via `node.table_mut()` para `PeerLink::poll`.

| Chamada | Significado |
|---------|-------------|
| `PeerTable::new(id)` | Tabela vazia (baixo nível; prefira `Node`). |
| `PeerTable::with(id, cfg)` | Igual, config explícito. |
| `table.connect()` | Ocupa um slot livre e devolve o hello. |
| `table.accept(hello)` | Instala um hello incoming. |
| `table.find(id)` | Acha o vizinho. |

`PeerLink` é o lado ao vivo: uma `WindowedPump` mais a máquina do hello.
Prefira `Node::{connect,accept}` a `PeerLink::connect/accept`.

```text
connect:  START → hello(12) → espera hello → Established
accept:   espera hello → START → hello(12) → Established
```

`PeerLink::poll(&mut table)` nunca entra em loop. Depois de
`Established`, DATA é payload comum (`LinkEvent::Received`). Use
`link.offer(byte)` — não `pump_mut().sender_mut().offer(byte)`.

Um duplex físico tem **um** fluxo incoming. O sender precisa de
ACK/NACK dele; o receiver precisa de DATA/START/FINISH/ABORT. Dois
leitores no mesmo anel roubam bytes um do outro (o RX come o ACK
que o TX está esperando). `Wire` (`DuplexWire`) demultiplexa frames
completos numa faixa de controle (ACK/NACK → TX) e numa de payload
(o resto, incluindo CRC ruim → RX, para ele poder NACK). O frame de
4 bytes não muda. `Wire` é um **harness em memória**, não o `Frame`
do fio.

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

O mesmo par move bytes A→B. Example de transporte:
`examples/ab_direct.rs` (`Wire::copy`). P2P hello + bytes:
`examples/p2p_pair.rs`, `tests/forum.rs`.

```rust
use psicose::prelude::*;

let cfg = SessionConfig::FORUM;
let wire = Wire::new();
let (pump_a, pump_b) = wire.link_pumps();

let mut alice = Node::<4>::with(PeerId::from_label(b"alice"), cfg);
let mut bob = Node::<4>::with(PeerId::from_label(b"bob"), cfg);

let mut a = match alice.connect(pump_a) {
    Ok(link) => link,
    Err(_) => return,
};
let mut b = bob.accept(pump_b);
assert!(establish(&mut a, &mut alice, &mut b, &mut bob));

let ping = b"ping";
let mut board = [0u8; 32];
let mut inbox = Defragmenter::new(&mut board);
assert!(send_message(&mut a, &mut alice, &mut b, &mut bob, 1, ping, &mut inbox));
```

Um driver UART de verdade faz o mesmo corte: `Pump::on(tx, rx)` ou
`Node` com o seu `ByteTransport` no lugar de `Wire::link_pumps`.

Superfície completa da crate: [API.pt-BR.md](API.pt-BR.md).

### 11.7 Ainda não (aplicações desta camada)

Roteamento, store-and-forward, gossip (`SeenSet<N>`), assinaturas,
hash de conteúdo, backpressure / prioridade. Ficam fora do transporte
e fora do frame de 4 bytes.

## 12. Fora deste documento

- SessionId dentro do frame de 4 bytes
- UART / SPI / CAN / rádio
- `psicose::File`

Ordem: transporte + identidade/sessão/stream P2P estão aqui →
discovery / gossip → fórum.
