# Security

## What PSICOSE guarantees

| Guarantee | Mechanism |
| --- | --- |
| Detect accidental bit errors on a frame | CRC-8 on `TYPE‖SEQ‖DATA` |
| Deliver payload bytes in order, at most once | SEQ + ACK/NACK + retransmit |
| Bound hangs on a silent peer | `RetryPolicy` + `IdleBudget` |

CRC-8 is **noise detection**, not authentication. An adversary who can
inject or modify bytes on the link can forge a CRC-valid frame.

## What PSICOSE does **not** provide

Confidentiality and authenticity are **outside** this crate. The public
API is only `psicose::…` types with **zero** crypto dependencies.

If the link may be adversarial, seal application messages **before** they
enter `Pump` / `PeerLink` / `LinkFace`, using a crypto library of your
choice in **your** firmware — not inside psicose.

`Capabilities::ENCRYPTION` / `SessionConfig::SECURE` are **advertisement
bits** in the hello only; they do not encrypt anything by themselves.

## Reporting

See the repository security policy / open an issue privately if needed.
