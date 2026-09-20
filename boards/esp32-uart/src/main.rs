//! PSICOSE on ESP32 classic — UART1 link demo.
//!
//! Path: `UART1 → ByteTransport (local) → LinkFace → Pump`
//!
//! The board package may depend on `esp-hal`. The **psicose** crate itself
//! has **zero** dependencies — only `psicose::` types appear in the motor API.
//!
//! Pinout (DevKit-style):
//! - UART1 TX = GPIO17
//! - UART1 RX = GPIO16
//! - Baud = 115200
//!
//! ## Roles
//!
//! Set [`INITIATOR`] before flashing:
//! - `true`  — sends `ping` (loopback: wire GPIO17↔GPIO16 on the same board)
//! - `false` — receive-only peer (second board; cross TX↔RX + GND)
//!
//! Do **not** flash `INITIATOR = true` on both ends of a two-board link:
//! both would START/DATA at once and corrupt the shared TX wire.
//!
//! ```sh
//! cd boards
//! cargo run -p esp32-uart
//! ```

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    main,
    uart::{Config, Uart},
};
use esp_println::println;
use psicose::{ByteTransport, LinkFace, PumpEvent, TxState};

esp_bootloader_esp_idf::esp_app_desc!();

/// `true` = send `ping`. `false` = only receive (second board).
const INITIATOR: bool = true;

const PING: &[u8] = b"ping";

/// Thin ESP32 UART → [`ByteTransport`]. Lives in the **board**, not in psicose.
struct EspUart(Uart<'static, esp_hal::Blocking>);

#[derive(Debug)]
struct UartIoError;

impl ByteTransport for EspUart {
    type Error = UartIoError;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        // Trait allows blocking until the FIFO accepts the byte.
        self.0.write(&[byte]).map(|_| ()).map_err(|_| UartIoError)
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        if !self.0.read_ready() {
            return Ok(None);
        }
        let mut buf = [0u8; 1];
        match self.0.read(&mut buf) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(buf[0])),
            Err(_) => Err(UartIoError),
        }
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let uart = match Uart::new(
        peripherals.UART1,
        Config::default().with_baudrate(115_200),
    ) {
        Ok(u) => u.with_tx(peripherals.GPIO17).with_rx(peripherals.GPIO16),
        Err(e) => {
            println!("UART1 config failed: {e:?}");
            loop {}
        }
    };

    println!(
        "psicose esp32-uart: UART1 @ 115200 GPIO17/16 initiator={INITIATOR}"
    );

    let face = LinkFace::new(EspUart(uart));
    let mut pump = face.pump();

    // After FINISH/ABORT the sender must `offer_start` before DATA again.
    let mut need_start = INITIATOR;
    let mut out_i = 0usize;
    let mut finish_offered = false;
    let mut got = [0u8; 4];
    let mut got_n = 0usize;

    loop {
        if INITIATOR {
            let st = pump.sender().state();
            if matches!(
                st,
                TxState::Idle | TxState::Finished | TxState::Aborted | TxState::Failed
            ) {
                if need_start {
                    if pump.sender_mut().offer_start().is_ok() {
                        need_start = false;
                        out_i = 0;
                        finish_offered = false;
                        got_n = 0;
                    }
                } else if st == TxState::Idle {
                    if out_i < PING.len() {
                        if pump.sender_mut().offer(PING[out_i]).is_ok() {
                            out_i += 1;
                        }
                    } else if !finish_offered {
                        if pump.sender_mut().offer_finish().is_ok() {
                            finish_offered = true;
                        }
                    }
                }
            }
        }

        match pump.poll() {
            Ok(PumpEvent::Received(b)) => {
                if got_n < got.len() {
                    got[got_n] = b;
                    got_n += 1;
                }
                println!("rx {b:#04x} ({got_n}/{})", PING.len());
                if got_n == PING.len() {
                    println!(
                        "psicose: got {:?}",
                        core::str::from_utf8(&got[..got_n]).unwrap_or("?")
                    );
                    got_n = 0;
                }
            }
            Ok(PumpEvent::Completed) => {
                println!("psicose: session completed");
                if INITIATOR {
                    need_start = true;
                }
            }
            Ok(PumpEvent::Aborted) => {
                println!("psicose: aborted");
                if INITIATOR {
                    need_start = true;
                }
            }
            Ok(_) => {}
            Err(e) => {
                println!("psicose poll err: {e:?}");
                let _ = pump.abort();
                if INITIATOR {
                    need_start = true;
                }
            }
        }
    }
}
