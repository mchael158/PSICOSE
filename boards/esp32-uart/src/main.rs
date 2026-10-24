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
//! ([`psicose::WindowTurn`]): the `true` board opens with an 8-byte sample,
//! then the `false` board answers with its own sample. DATA uses the
//! window of 8, so the sample is not one round trip per byte.
//!
//! - Loopback: `FIRST = true` only works one way (there is no second peer).
//!   Use two boards for the exchange.
//! - Two boards: opposite [`FIRST`], cross TX↔RX, common GND.
//! - RS-485: set [`RS485`] so frames leave as a burst and GPIO4 is the DE pin.
//!   The session is stop-and-wait ([`psicose::Turn`]). Do not use the window
//!   on one pair — the peer's ACK would collide with the next DATA frame.
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
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::{
    main,
    uart::{Config, Uart},
};
use esp_println::println;
use psicose::{ByteTransport, HalfDuplex, LineDirection, LinkFace, TurnEvent};

esp_bootloader_esp_idf::esp_app_desc!();

/// `true` = open with [`SAMPLE_A`]. `false` = wait, then answer [`SAMPLE_B`].
const FIRST: bool = true;

/// `true` = one pair, GPIO4 drives DE, frames go out as a 4-byte burst.
const RS485: bool = false;

/// 23.1 °C, 47% RH, 3710 mV, flags 0. Magic `0xA5`, trailing xor.
const SAMPLE_A: [u8; 8] = [0xA5, 0xE7, 0x00, 0x2F, 0x7E, 0x0E, 0x00, 0x1D];
/// 22.0 °C, 40% RH, 3600 mV, flags 1.
const SAMPLE_B: [u8; 8] = [0xA5, 0xDC, 0x00, 0x28, 0x10, 0x0E, 0x01, 0x4E];

const OURS: &[u8] = if FIRST { &SAMPLE_A } else { &SAMPLE_B };
const THEIRS: &[u8] = if FIRST { &SAMPLE_B } else { &SAMPLE_A };

/// Thin ESP32 UART → [`ByteTransport`]. Lives in the **board**, not in psicose.
struct EspUart(Uart<'static, esp_hal::Blocking>);

#[derive(Debug)]
struct UartIoError;

/// RS-485 driver enable. High while a frame is leaving the UART.
struct DePin(Output<'static>);

impl LineDirection for DePin {
    fn transmit(&mut self) {
        self.0.set_high();
    }

    fn receive(&mut self) {
        self.0.set_low();
    }
}

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

    println!("psicose esp32-uart: UART1 @ 115200 GPIO17/16 first={FIRST} rs485={RS485}");

    if RS485 {
        let de = Output::new(peripherals.GPIO4, Level::Low, OutputConfig::default());
        let face = LinkFace::new(HalfDuplex::new(EspUart(uart), DePin(de)));
        let mut turn = face.turn::<8>(FIRST);
        let _ = turn.push(OURS);
        poll_forever(&mut turn);
    } else {
        let face = LinkFace::new(EspUart(uart));
        let mut turn = face.window_turn::<8>(FIRST);
        let _ = turn.push(OURS);
        poll_forever(&mut turn);
    }
}

fn poll_forever<T: PollTurn>(turn: &mut T) -> ! {
    let mut got = [0u8; 8];
    let mut got_n = 0usize;
    loop {
        match turn.step() {
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
                let _ = turn.cancel();
            }
        }
    }
}

trait PollTurn {
    type Error: core::fmt::Debug;
    fn step(&mut self) -> Result<TurnEvent, Self::Error>;
    fn cancel(&mut self) -> Result<(), Self::Error>;
}

impl<'a> PollTurn for psicose::WindowTurn<psicose::FaceTx<'a, EspUart>, psicose::FaceRx<'a, EspUart>, 8>
{
    type Error = psicose::Error<psicose::FaceError<UartIoError>>;

    fn step(&mut self) -> Result<TurnEvent, Self::Error> {
        self.poll()
    }

    fn cancel(&mut self) -> Result<(), Self::Error> {
        self.abort()
    }
}

impl<'a> PollTurn
    for psicose::Turn<
        psicose::FaceTx<'a, HalfDuplex<EspUart, DePin>>,
        psicose::FaceRx<'a, HalfDuplex<EspUart, DePin>>,
        8,
    >
{
    type Error = psicose::Error<psicose::FaceError<UartIoError>>;

    fn step(&mut self) -> Result<TurnEvent, Self::Error> {
        self.poll()
    }

    fn cancel(&mut self) -> Result<(), Self::Error> {
        self.abort()
    }
}
