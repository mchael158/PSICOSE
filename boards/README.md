# Board firmware (ESP32 classic)

Separate Cargo workspace from the host `psicose` crate (MSRV 1.75).

Stock `stable`/`nightly` **cannot** build `xtensa-esp32-none-elf`. You need the
esp-rs toolchain from [espup](https://esp-rs.github.io/book/).

## One-time setup (Windows PowerShell)

```powershell
cargo +stable install espup --locked
espup install
. $HOME\export-esp.ps1          # every new shell (sets PATH / LIBCLANG_PATH)
cargo +stable install espflash --locked   # if you do not have it yet
```

`boards/rust-toolchain.toml` selects channel `esp` automatically when you `cd boards`.

## Build / flash

| Package | Link |
| --- | --- |
| UART | [`esp32-uart`](esp32-uart/) |
| Wi‑Fi / TCP | [`esp32-wifi`](esp32-wifi/) |

```powershell
cd boards
. $HOME\export-esp.ps1
cargo run -p esp32-uart
cargo run -p esp32-wifi   # SSID / PASSWORD / HOST env vars
```

If you see `can't find crate for core` / `xtensa-esp32-none-elf may not be
installed`, the `esp` toolchain is missing or `export-esp.ps1` was not sourced.

Dependency pins: see `[workspace.dependencies]` in [`Cargo.toml`](Cargo.toml).
