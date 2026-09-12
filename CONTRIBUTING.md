# Contributing

Thanks for looking at PSICOSE. Small, focused PRs are welcome.

## Before you open a PR

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features aead -- -D warnings
cargo clippy --all-targets --features embedded-io -- -D warnings
cargo test
cargo test --features aead
cargo test --features embedded-io
cargo doc --no-deps --all-features
```

MSRV is **Rust 1.75** (`rust-toolchain.toml`). Keep the default build
**zero dependencies** unless you are touching optional features (`aead`,
`embedded-io`).

## Design constraints (please keep)

- `#![no_std]`, `#![forbid(unsafe_code)]`, no heap
- Wire frame stays 4 bytes; DATA payload stays 1 byte
- No homemade cryptography — AEAD is RustCrypto ChaCha20-Poly1305 only
- Capabilities bits announce; they must not silently change `PeerLink` internals
- Prefer honest docs over marketing demos

## Spec

Wire and semantics live in [`docs/PROTOCOL.md`](docs/PROTOCOL.md)
([pt-BR](docs/PROTOCOL.pt-BR.md)). If code and spec disagree, fix one of
them in the same PR and add a test when you can.

## Issues

Bug reports with a minimal repro (`cargo test` or a tiny example) help most.
Feature requests: say what hardware / link constraint you are hitting.
