//! Cooperative [`Pump`]: incremental progress, no internal loop.

mod common;

use core::cell::RefCell;

use psicose::tx::TxState;
use psicose::{Error, IdleBudget, Pump, PumpEvent, SliceSource, StreamError};

use common::{End, Wires};

fn pair(wires: &RefCell<Wires>) -> Pump<End<'_>, End<'_>> {
    Pump::on(End { wires, is_a: true }, End { wires, is_a: false })
}

fn drive_until<F>(pump: &mut Pump<End<'_>, End<'_>>, mut done: F) -> PumpEvent
where
    F: FnMut(PumpEvent) -> bool,
{
    let mut last = PumpEvent::Idle;
    let mut i = 0usize;
    while i < 100_000 {
        i += 1;
        match pump.poll() {
            Ok(ev) => {
                last = ev;
                if done(ev) {
                    return ev;
                }
            }
            Err(_) => return last,
        }
    }
    last
}

#[test]
fn send_all_is_just_a_loop_over_poll() {
    let wires = RefCell::new(Wires::new());
    let mut pump = pair(&wires);
    let payload = [0x50, 0x53, 0x49];
    let mut src = SliceSource::new(&payload);
    assert_eq!(pump.send_all(&mut src), Ok(3));
    assert_eq!(pump.sender().state(), TxState::Finished);
    assert_eq!(pump.stats().bytes_delivered, 3);
    assert!(pump.stats().frames_sent >= 4);
    assert!(pump.stats().ticks > 0);
}

#[test]
fn send_all_budgeted_returns_when_peer_is_silent() {
    // TX writes into a ring nobody drains; RX reads an empty ring.
    // Progress/retransmit must not reset the outer idle budget.
    let wires = RefCell::new(Wires::new());
    let mut pump = Pump::on(
        End {
            wires: &wires,
            is_a: true,
        },
        End {
            wires: &wires,
            is_a: true,
        },
    );
    let payload = [0xAB];
    let mut src = SliceSource::new(&payload);
    let err = pump.send_all_budgeted(&mut src, IdleBudget::new(32));
    assert!(matches!(
        err,
        Err(StreamError::Protocol(Error::IdleBudgetExhausted))
    ));
}

#[test]
fn poll_returns_immediately_and_makes_progress() {
    let wires = RefCell::new(Wires::new());
    let mut pump = pair(&wires);
    assert_eq!(pump.sender_mut().offer(0x7E), Ok(()));
    let first = pump.poll();
    assert!(matches!(
        first,
        Ok(PumpEvent::Idle) | Ok(PumpEvent::Progress)
    ));
    let ev = drive_until(&mut pump, |e| matches!(e, PumpEvent::Received(0x7E)));
    assert_eq!(ev, PumpEvent::Received(0x7E));
    assert_eq!(pump.stats().bytes_delivered, 1);
}

#[test]
fn abort_during_data_cancels_and_is_reusable() {
    let wires = RefCell::new(Wires::new());
    let mut pump = pair(&wires);
    assert_eq!(pump.sender_mut().offer(0x11), Ok(()));
    let _ = drive_until(&mut pump, |e| {
        matches!(e, PumpEvent::Progress | PumpEvent::Received(_))
    });
    assert_eq!(pump.abort(), Ok(()));
    let ev = drive_until(&mut pump, |e| matches!(e, PumpEvent::Aborted));
    assert_eq!(ev, PumpEvent::Aborted);
    assert_eq!(pump.sender().state(), TxState::Aborted);

    assert_eq!(pump.sender_mut().offer_start(), Ok(()));
    let mut i = 0usize;
    while i < 100_000 && pump.sender().state() != TxState::Idle {
        i += 1;
        let _ = pump.poll();
    }
    assert_eq!(pump.sender().state(), TxState::Idle);
    assert_eq!(pump.sender_mut().offer(0x22), Ok(()));
    let ev = drive_until(&mut pump, |e| matches!(e, PumpEvent::Received(0x22)));
    assert_eq!(ev, PumpEvent::Received(0x22));
}

#[test]
fn start_after_abort_resets_seq_to_zero() {
    let wires = RefCell::new(Wires::new());
    let mut pump = pair(&wires);
    assert_eq!(pump.sender_mut().offer(0x01), Ok(()));
    let _ = drive_until(&mut pump, |e| {
        matches!(e, PumpEvent::Sent | PumpEvent::Received(_))
    });
    assert_eq!(pump.abort(), Ok(()));
    let _ = drive_until(&mut pump, |e| matches!(e, PumpEvent::Aborted));

    assert_eq!(pump.sender_mut().offer_start(), Ok(()));
    let mut i = 0usize;
    while i < 100_000 && pump.sender().state() != TxState::Idle {
        i += 1;
        let _ = pump.poll();
    }
    assert_eq!(pump.sender().state(), TxState::Idle);
    assert_eq!(pump.sender().next_seq(), 0);
    assert_eq!(pump.receiver().expected_seq(), 0);
}
