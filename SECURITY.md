# Security

## Threat model

PSICOSE is a **reliable** byte transport, not a secure channel by default.

| Mechanism | Protects against | Does **not** protect against |
| --- | --- | --- |
| CRC-8 on the 4-byte frame | Accidental bit errors / noise | Forgery, injection, replay, eavesdropping |
| ACK / SEQ / retry | Loss and reordering on a cooperative link | A malicious peer that speaks the protocol |
| Feature `aead` (`seal_to` / `open_from`) | Tampering and (with secrecy) eavesdropping of **application** payloads | Misused nonces, leaked keys, traffic analysis of frame timing |

The 4-byte wire frame is unchanged when `aead` is enabled. You seal
application messages **before** they enter `Fragmenter` / `PeerLink` /
`Pump`.

## Using `aead` correctly

- Use a **unique nonce per key** for the lifetime of that key (never reuse).
- Prefer a distinct key per direction or per peer pair when possible.
- Put stable context in AAD when useful (`PeerId`, `MessageId`, version).
- On `open` / `open_from` failure, treat the buffer as untrusted junk.

This crate does **not** implement key exchange, certificates, or HKDF.
You supply key and nonce material from your own provisioning story.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security bugs.

Email the maintainer listed on [crates.io/crates/psicose](https://crates.io/crates/psicose)
(or the commit author on this repository) with:

1. Affected version(s)
2. Description and impact
3. Proof of concept if available

We aim to acknowledge reports promptly and coordinate a fix before any
public disclosure.
