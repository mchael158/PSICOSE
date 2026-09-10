//! EEPROM dump over a lossy radio, window of 8.
//!
//! A remote logger has 300 bytes of calibration + event log in EEPROM.
//! The uplink drops the first DATA frame (fading). Selective-repeat
//! (`N = 8`) keeps 8 frames in flight; the lost slot is retried. The
//! gateway still sees a clean, in-order dump.
//!
//! ```text
//! EEPROM  -->  WindowedSender<_, 8>  --lossy radio-->  WindowedReceiver
//! ```
//!
//! Run: `cargo run --example radio_windowed`

#[path = "link.rs"]
mod link;

use core::cell::RefCell;

use psicose::error::Error;
use psicose::fault::{FaultPolicy, FaultyTransport};
use psicose::rx::PollOutcome;
use psicose::timeout::RetryPolicy;
use psicose::tx::TxPoll;
use psicose::window::{WindowedReceiver, WindowedSender};
use psicose::{ByteSink, ByteSource, SliceSink, SliceSource};

fn main() {
    let mut eeprom = [0u8; 300];
    let mut i = 0usize;
    while i < eeprom.len() {
        eeprom[i] = (i % 256) as u8;
        i += 1;
    }

    let wires = RefCell::new(link::Wires::new());
    let (tx_end, rx_end) = link::pair(&wires);

    // First DATA frame vanishes. The protocol must retransmit it.
    let mut tx = WindowedSender::<_, 8>::with_policy(
        FaultyTransport::on_write(tx_end, FaultPolicy::drop_first(1)),
        RetryPolicy::new(128, 16),
    );
    let mut rx = WindowedReceiver::<_, 8>::new(rx_end);

    let mut src = SliceSource::new(&eeprom);
    let mut host = [0u8; 300];
    let mut sink = SliceSink::new(&mut host);

    let mut hold: Option<u8> = None;
    let mut exhausted = false;
    let mut finish_offered = false;
    let mut n = 0usize;
    let mut ok = false;

    let mut steps = 0usize;
    while steps < 2_000_000 {
        steps += 1;
        if hold.is_none() && !exhausted {
            match src.read_byte() {
                Ok(Some(b)) => hold = Some(b),
                Ok(None) => exhausted = true,
                Err(_) => break,
            }
        }
        if let Some(b) = hold {
            match tx.offer(b) {
                Ok(()) => hold = None,
                Err(Error::WindowFull) | Err(Error::NotIdle) => {}
                Err(_) => break,
            }
        } else if exhausted && !finish_offered {
            match tx.offer_finish() {
                Ok(()) => finish_offered = true,
                Err(Error::NotIdle) => {}
                Err(_) => break,
            }
        }

        match tx.poll() {
            Ok(TxPoll::TransferDone) => {}
            Ok(_) => {}
            Err(_) => break,
        }

        match rx.poll() {
            Ok(PollOutcome::Delivered(byte)) => {
                if sink.write_byte(byte).is_err() {
                    break;
                }
                n = n.saturating_add(1);
            }
            Ok(PollOutcome::TransferFinished) => {
                ok = true;
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    if ok && n == 300 && sink.written() == &eeprom[..] {
        println!("radio_windowed: 300 B dump ok after a dropped frame (N=8)");
    } else {
        eprintln!("radio_windowed failed: ok={ok} n={n}");
        std::process::exit(1);
    }
}
