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
//! Set [`FIRST`] before flashing. Both boards send, one session at a time
//! ([`psicose::Turn`]): the `true` board opens with `ping`, then the `false`
//! board answers `pong`.
//!
//! - Loopback: `FIRST = true` only works one way (there is no second peer).
//!   Use two boards for the exchange.
//! - Two boards: opposite [`FIRST`], cross TX↔RX, common GND.
//!
//! Do **not** flash `FIRST = true` on both ends: both would `START` at once.
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
use psicose::{ByteTransport, LinkFace, TurnEvent};

esp_bootloader_esp_idf::esp_app_desc!();

/// `true` = open with `ping`. `false` = wait, then answer `pong`.
const FIRST: bool = true;

const OURS: &[u8] = if FIRST { b"ping" } else { b"pong" };
const THEIRS: &[u8] = if FIRST { b"pong" } else { b"ping" };

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

    let uart = match Uart::new(peripherals.UART1, Config::default().with_baudrate(115_200)) {
        Ok(u) => u.with_tx(peripherals.GPIO17).with_rx(peripherals.GPIO16),
        Err(e) => {
            println!("UART1 config failed: {e:?}");
            loop {}
        }
    };

    println!("psicose esp32-uart: UART1 @ 115200 GPIO17/16 first={FIRST}");

    let face = LinkFace::new(EspUart(uart));
    let mut turn = face.turn::<8>(FIRST);
    let _ = turn.push(OURS);

    let mut got = [0u8; 4];
    let mut got_n = 0usize;

    loop {
        match turn.poll() {
            Ok(TurnEvent::Received(b)) => {
                if got_n < got.len() {
                    got[got_n] = b;
                    got_n += 1;
                }
                println!("rx {b:#04x} ({got_n}/{})", THEIRS.len());
                if got_n == THEIRS.len() {
                    println!(
                        "psicose: got {:?}",
                        core::str::from_utf8(&got[..got_n]).unwrap_or("?")
                    );
                    got_n = 0;
                }
            }
            Ok(TurnEvent::Yielded) => println!("psicose: our turn done"),
            Ok(TurnEvent::Granted) => println!("psicose: our turn"),
            Ok(TurnEvent::Idle) => {}
            Err(e) => {
                println!("psicose poll err: {e:?}");
                let _ = turn.abort();
            }
        }
    }
}
