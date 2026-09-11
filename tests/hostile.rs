//! Adversarial wire: drop DATA, corrupt CRC, drop ACK.
//! The sink must see each payload byte exactly once, in order.

mod common;

use core::cell::RefCell;

use psicose::fault::{FaultPolicy, FaultyTransport};
use psicose::protocol::Frame;
use psicose::rx::{Receiver, RxState};
use psicose::timeout::RetryPolicy;
use psicose::tx::{Sender, TxState};

use common::{abort_coop, finish_coop, pump_rx, send_byte_coop, start_coop, End, Wires};

fn policy() -> RetryPolicy {
    RetryPolicy::new(32, 8)
}

#[test]
fn lost_data_is_retransmitted_and_delivered_once() {
    // DATA 0, DATA 1, DATA 2 ← perdido, DATA 3.
    const N: usize = 4;
    let wires = RefCell::new(Wires::new());
    let tx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: true,
        },
        FaultPolicy::drop_every(3),
    );
    let mut sender = Sender::with_policy(tx_end, policy());
    let mut receiver = Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut received = [0u8; N];
    let mut n = 0usize;
    for i in 0..N {
        send_byte_coop(&mut sender, &mut receiver, i as u8, &mut received, &mut n);
    }

    assert_eq!(n, N);
    assert_eq!(received, [0, 1, 2, 3]);
}

#[test]
fn corrupt_data_gets_nack_then_retransmit_then_ack() {
    // DATA 42 → CRC inválido → NACK → DATA 42 de novo → ACK.
    let wires = RefCell::new(Wires::new());
    let tx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: true,
        },
        FaultPolicy::corrupt_first(1),
    );
    let mut sender = Sender::with_policy(tx_end, policy());
    let mut receiver = Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x42, &mut received, &mut n);

    assert_eq!(n, 1);
    assert_eq!(received[0], 0x42);
}

#[test]
fn lost_ack_causes_retransmit_but_not_double_delivery() {
    // DATA 42 → ACK → ACK perdido → DATA 42 de novo →
    // RX reconhece DUPLICATE, não entrega 42 duas vezes, ACK de novo.
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        policy(),
    );
    let rx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: false,
        },
        FaultPolicy::drop_first(1),
    );
    let mut receiver = Receiver::new(rx_end);

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x42, &mut received, &mut n);

    assert_eq!(n, 1, "payload must be delivered exactly once");
    assert_eq!(received[0], 0x42);
}

#[test]
fn wraparound_survives_a_dropped_ack_on_seq_255() {
    const N: usize = 260;
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        policy(),
    );
    // Drop the ACK of DATA 255 (the 256th accepted frame → 256th ACK).
    let rx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: false,
        },
        FaultPolicy {
            drop_first: 0,
            drop_every: 256,
            corrupt_first: 0,
            corrupt_every: 0,
            frame_granularity: true,
        },
    );
    let mut receiver = Receiver::new(rx_end);

    let mut received = [0u8; N];
    let mut n = 0usize;
    for i in 0..N {
        send_byte_coop(
            &mut sender,
            &mut receiver,
            (i % 256) as u8,
            &mut received,
            &mut n,
        );
    }

    assert_eq!(n, N);
    for i in 0..N {
        assert_eq!(received[i], (i % 256) as u8);
    }
}

#[test]
fn lost_nack_is_treated_as_silence_then_retransmit() {
    // DATA corrompido → NACK → NACK perdido → timeout → DATA limpo → ACK.
    let wires = RefCell::new(Wires::new());
    let tx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: true,
        },
        FaultPolicy::corrupt_first(1),
    );
    let mut sender = Sender::with_policy(tx_end, policy());
    let rx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: false,
        },
        FaultPolicy::drop_first(1),
    );
    let mut receiver = Receiver::new(rx_end);

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x77, &mut received, &mut n);

    assert_eq!(n, 1);
    assert_eq!(received[0], 0x77);
}

#[test]
fn delayed_data_is_delivered_once_after_the_hold() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        policy(),
    );
    let rx_end = FaultyTransport::on_write(End {
        wires: &wires,
        is_a: false,
    }, FaultPolicy::none())
    .with_read_hold(8);
    let mut receiver = Receiver::new(rx_end);

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x5A, &mut received, &mut n);

    assert_eq!(n, 1);
    assert_eq!(received[0], 0x5A);
}

#[test]
fn lost_finish_is_retransmitted_and_acked() {
    // DATA ok, FINISH perdido, FINISH de novo, ACK.
    let wires = RefCell::new(Wires::new());
    let tx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: true,
        },
        FaultPolicy::drop_every(2),
    );
    let mut sender = Sender::with_policy(tx_end, policy());
    let mut receiver = Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x01, &mut received, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut received, &mut n);

    assert_eq!(n, 1);
    assert_eq!(received[0], 0x01);
    assert_eq!(receiver.state(), psicose::rx::RxState::Finished);
}

#[test]
fn lost_finish_ack_causes_retransmit_without_extra_delivery() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        policy(),
    );
    let rx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: false,
        },
        FaultPolicy::drop_every(2),
    );
    let mut receiver = Receiver::new(rx_end);

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x02, &mut received, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut received, &mut n);

    assert_eq!(n, 1, "FINISH retry must not deliver payload again");
    assert_eq!(received[0], 0x02);
}

#[test]
fn corrupt_ack_is_ignored_then_retransmit_succeeds() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        policy(),
    );
    let rx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: false,
        },
        FaultPolicy::corrupt_first(1),
    );
    let mut receiver = Receiver::new(rx_end);

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x42, &mut received, &mut n);

    assert_eq!(n, 1);
    assert_eq!(received[0], 0x42);
}

#[test]
fn abort_during_retry_discards_data_and_reopens() {
    let wires = RefCell::new(Wires::new());
    let tx_end = FaultyTransport::on_write(
        End {
            wires: &wires,
            is_a: true,
        },
        FaultPolicy::corrupt_first(1),
    );
    let mut sender = Sender::with_policy(tx_end, policy());
    let mut receiver = Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut received = [0u8; 2];
    let mut n = 0usize;
    assert_eq!(sender.offer(0x33), Ok(()));
    let mut saw_retry = false;
    let mut i = 0usize;
    while i < 10_000 {
        i += 1;
        let _ = sender.poll();
        let _ = pump_rx(&mut receiver, &mut received, &mut n);
        if sender.state() == TxState::Retrying {
            saw_retry = true;
            break;
        }
    }
    assert_eq!(saw_retry, true);
    abort_coop(&mut sender, &mut receiver, &mut received, &mut n);
    assert_eq!(sender.state(), TxState::Aborted);

    start_coop(&mut sender, &mut receiver, &mut received, &mut n);
    send_byte_coop(&mut sender, &mut receiver, 0x44, &mut received, &mut n);
    assert_eq!(received[n - 1], 0x44);
}

#[test]
fn abort_after_finish_is_ignored() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        policy(),
    );
    let mut receiver = Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut received = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x02, &mut received, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut received, &mut n);
    assert_eq!(receiver.state(), RxState::Finished);

    {
        let mut w = wires.borrow_mut();
        for b in Frame::abort().to_bytes() {
            assert_eq!(w.a_to_b.push(b), true);
        }
    }
    let mut i = 0usize;
    while i < 16 {
        i += 1;
        let _ = pump_rx(&mut receiver, &mut received, &mut n);
    }
    assert_eq!(receiver.state(), RxState::Finished);
    assert_eq!(n, 1);
}

#[test]
fn wraparound_after_abort_starts_at_zero() {
    const N: usize = 256;
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::with_policy(
        End {
            wires: &wires,
            is_a: true,
        },
        policy(),
    );
    let mut receiver = Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut received = [0u8; N + 1];
    let mut n = 0usize;
    let mut i = 0usize;
    while i < N {
        send_byte_coop(
            &mut sender,
            &mut receiver,
            (i % 256) as u8,
            &mut received,
            &mut n,
        );
        i += 1;
    }
    assert_eq!(sender.next_seq(), 0);
    abort_coop(&mut sender, &mut receiver, &mut received, &mut n);
    start_coop(&mut sender, &mut receiver, &mut received, &mut n);
    assert_eq!(sender.next_seq(), 0);
    assert_eq!(receiver.expected_seq(), 0);
    send_byte_coop(&mut sender, &mut receiver, 0x7E, &mut received, &mut n);
    assert_eq!(received[n - 1], 0x7E);
    assert_eq!(sender.next_seq(), 1);
}
