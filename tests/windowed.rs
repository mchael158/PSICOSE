//! Windowed sender + receiver on a heapless cooperative link.
//! No `Vec`, no threads, no allocator.

mod common;

use core::cell::RefCell;

use psicose::error::Error;
use psicose::rx::PollOutcome;
use psicose::timeout::RetryPolicy;
use psicose::transport::ByteTransport;
use psicose::tx::TxPoll;
use psicose::window::{WindowedReceiver, WindowedSender};

use common::{End, Wires};

fn pump<T: ByteTransport, const N: usize>(
    rx: &mut WindowedReceiver<T, N>,
    out: &mut [u8],
    filled: &mut usize,
) -> bool {
    match rx.poll() {
        Ok(PollOutcome::Delivered(byte)) => {
            if *filled < out.len() {
                out[*filled] = byte;
                *filled += 1;
            }
            false
        }
        Ok(PollOutcome::TransferFinished) => true,
        Ok(_) | Err(_) => false,
    }
}

fn transfer<const N: usize>(src: &[u8], out: &mut [u8]) -> usize {
    let wires = RefCell::new(Wires::new());
    let mut sender = WindowedSender::<_, N>::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        RetryPolicy::new(128, 16),
    );
    let mut receiver = WindowedReceiver::<_, N>::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut offered = 0usize;
    let mut filled = 0usize;
    let mut finish_offered = false;
    let cap = src.len().saturating_mul(128).saturating_add(2048);

    for _ in 0..cap {
        if offered < src.len() {
            match sender.offer(src[offered]) {
                Ok(()) => offered += 1,
                Err(Error::WindowFull) => {}
                Err(_) => break,
            }
        } else if !finish_offered {
            match sender.offer_finish() {
                Ok(()) => finish_offered = true,
                Err(Error::NotIdle) => {}
                Err(_) => break,
            }
        }

        match sender.poll() {
            Ok(TxPoll::TransferDone) => return filled,
            Ok(_) => {}
            Err(_) => break,
        }
        for _ in 0..8 {
            if pump(&mut receiver, out, &mut filled) {
                break;
            }
        }
    }
    filled
}

#[test]
fn w8_delivers_in_order_across_wraparound() {
    const K: usize = 300;
    let mut src = [0u8; K];
    let mut out = [0u8; K];
    for (i, b) in src.iter_mut().enumerate() {
        *b = (i % 256) as u8;
    }
    let n = transfer::<8>(&src, &mut out);
    assert_eq!(n, K);
    assert_eq!(out, src);
}

#[test]
fn n1_matches_stop_and_wait_single_byte() {
    let src = [0x99u8];
    let mut out = [0u8; 1];
    let n = transfer::<1>(&src, &mut out);
    assert_eq!(n, 1);
    assert_eq!(out[0], 0x99);
}

#[test]
fn w4_survives_lost_data_inside_the_window() {
    use psicose::fault::{FaultPolicy, FaultyTransport};

    const K: usize = 16;
    let mut src = [0u8; K];
    let mut out = [0u8; K];
    for (i, b) in src.iter_mut().enumerate() {
        *b = i as u8;
    }

    let wires = RefCell::new(Wires::new());
    let mut sender = WindowedSender::<_, 4>::with_policy(
        FaultyTransport::on_write(
            End {
                wires: &wires,
                is_a: true,
            },
            FaultPolicy::drop_first(1),
        ),
        RetryPolicy::new(128, 16),
    );
    let mut receiver = WindowedReceiver::<_, 4>::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut offered = 0usize;
    let mut filled = 0usize;
    let mut finish_offered = false;

    for _ in 0..16384 {
        if offered < K {
            match sender.offer(src[offered]) {
                Ok(()) => offered += 1,
                Err(Error::WindowFull) => {}
                Err(_) => break,
            }
        } else if !finish_offered && sender.offer_finish().is_ok() {
            finish_offered = true;
        }
        match sender.poll() {
            Ok(TxPoll::TransferDone) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        for _ in 0..8 {
            if pump(&mut receiver, &mut out, &mut filled) {
                break;
            }
        }
    }

    assert_eq!(filled, K);
    assert_eq!(out, src);
}
