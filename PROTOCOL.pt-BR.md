# PSICOSE-1B — protocolo formal (0.2.2)

[English](PROTOCOL.md) · [Português (Brasil)](PROTOCOL.pt-BR.md)

Máquina de transporte `no_std`, sem heap. Este arquivo é a especificação.
O código em `src/` é a implementação. Se os dois divergirem, o teste
adversarial em `tests/hostile.rs` decide.

**Estado:** transporte de byte confiável experimental. Ainda não é o
protocolo final.

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

CRC-8: poly `0x07`, init `0x00`, sem reflexão, sem xor-out, sobre
`TYPE || SEQ || DATA`.

### Validade semântica

CRC válido ≠ semanticamente válido. Um `Frame` só existe se **os dois**
valerem.

Frames de controle (`ACK`, `NACK`, `START`, `FINISH`) devem ter
`DATA = 0`. `START` deve ter `SEQ = 0`.

Os construtores públicos são `Frame::data`, `Frame::ack`, `Frame::nack`,
`Frame::start` e `Frame::finish`. Não há `Frame::new` público.
`Frame::from_bytes` rejeita um frame com CRC válido que quebre essas
regras (`FrameError::InvalidSemantics`). Não há caminho `unchecked`.

## 3. Sequência

```
0 → 1 → … → 254 → 255 → 0
```

`previous(0) == 255`. O wraparound faz parte do protocolo, não é erro.

## 4. Sessão (START / DATA / FINISH)

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
- `START` abre uma sessão nova

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
NACK do `expected` atual, `PollOutcome::Rejected`, continuar o poll.

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
      (ou Finished)        ▼
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
         DATA        START       FINISH
           │           │           │
           ▼           ▼           ▼
      ACK / NACK    ACK 0       ACK seq
           │           │           │
           ▼           ▼           ▼
         IDLE        IDLE       FINISHED
```

Um `FINISH` duplicado depois de `FINISHED` é reconfirmado com ACK.
`START` depois de `FINISHED` abre uma sessão nova.

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

Nenhum desses modos pode corromper o fluxo visto pelo `ByteSink`.

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

## 9. Memória de nó (futuro PSICOSE-8)

O protocolo de transporte já é uma máquina de registradores de 8 bits
(`SEQ`, `DATA`, CRC). A identidade de 256 bytes não foi abandonada:

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

Endereço = `u8`, dado = `u8`, memória = 256 bytes. Ainda não é uma VM;
é o teto de estado que o 0.2.2 se recusa a ultrapassar.

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
START / FINISH continuam stop-and-wait (janela vazia para FINISH).
FINISH ainda exige SEQ == expected (sem buracos).
```

`WindowFull` quer dizer: faça poll até um ACK liberar um slot, depois
ofereça de novo.

## 11. Fora deste documento

- SessionId dentro do frame de 4 bytes
- UART / SPI / CAN / rádio
- `psicose::File`

Ordem: janela está aqui → source/sink concretos → fio real.
