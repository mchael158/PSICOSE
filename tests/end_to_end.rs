//! Real [`Sender`] + [`Receiver`] on one thread, heapless link.
//! 300 bytes forces `SEQ` wraparound (255 → 0) on both ends.

mod common;

use core::cell::RefCell;

use psicose::tx::Sender;
use psicose::ByteSource;

use common::{abort_coop, finish_coop, send_byte_coop, start_coop, End, Wires};

#[test]
fn delivers_bytes_in_order_across_a_sequence_wraparound() {
    const N: usize = 300;
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut receiver = psicose::rx::Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

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

    finish_coop(&mut sender, &mut receiver, &mut received, &mut n);

    assert_eq!(n, N);
    for i in 0..N {
        assert_eq!(received[i], (i % 256) as u8);
    }
}

#[test]
fn single_byte_round_trip() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut receiver = psicose::rx::Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut got = [0u8; 1];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x99, &mut got, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut got, &mut n);
    assert_eq!(n, 1);
    assert_eq!(got[0], 0x99);
}

#[test]
fn start_resets_the_session_mid_stream() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut receiver = psicose::rx::Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut got = [0u8; 4];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0xA0, &mut got, &mut n);
    send_byte_coop(&mut sender, &mut receiver, 0xA1, &mut got, &mut n);
    assert_eq!(receiver.expected_seq(), 2);

    start_coop(&mut sender, &mut receiver, &mut got, &mut n);
    assert_eq!(receiver.expected_seq(), 0);
    assert_eq!(sender.next_seq(), 0);

    send_byte_coop(&mut sender, &mut receiver, 0xB0, &mut got, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut got, &mut n);

    assert_eq!(n, 3);
    assert_eq!(&got[..3], &[0xA0, 0xA1, 0xB0]);
}

#[test]
fn start_after_finish_opens_a_new_session() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut receiver = psicose::rx::Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut got = [0u8; 2];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x11, &mut got, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut got, &mut n);
    assert_eq!(receiver.state(), psicose::rx::RxState::Finished);

    start_coop(&mut sender, &mut receiver, &mut got, &mut n);
    send_byte_coop(&mut sender, &mut receiver, 0x22, &mut got, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut got, &mut n);

    assert_eq!(n, 2);
    assert_eq!(&got[..2], &[0x11, 0x22]);
}

#[test]
fn ten_thousand_bytes_wrap_the_sequence_repeatedly() {
    const N: usize = 10_000;
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut receiver = psicose::rx::Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

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
    finish_coop(&mut sender, &mut receiver, &mut received, &mut n);

    assert_eq!(n, N);
    for i in 0..N {
        assert_eq!(received[i], (i % 256) as u8);
    }
}

#[test]
fn any_blob_is_just_bytes_through_a_source() {
    // JPEG SOI + a packed config. PSICOSE does not know either type.
    let blob = [
        0xFF, 0xD8, 0xFF, 0xE0, // JPEG-looking prefix
        0x01, 0x00, 0x1E, 0x00, // "struct" fields as bytes
    ];
    let mut src = psicose::SliceSource::new(&blob);

    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut receiver = psicose::rx::Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut got = [0u8; 8];
    let mut n = 0usize;
    while let Ok(Some(byte)) = src.read_byte() {
        send_byte_coop(&mut sender, &mut receiver, byte, &mut got, &mut n);
    }
    finish_coop(&mut sender, &mut receiver, &mut got, &mut n);

    assert_eq!(src.remaining(), 0);
    assert_eq!(&got[..n], &blob);
}

#[test]
fn abort_during_data_then_new_session() {
    let wires = RefCell::new(Wires::new());
    let mut sender = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut receiver = psicose::rx::Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut got = [0u8; 2];
    let mut n = 0usize;
    send_byte_coop(&mut sender, &mut receiver, 0x10, &mut got, &mut n);
    abort_coop(&mut sender, &mut receiver, &mut got, &mut n);
    assert_eq!(sender.state(), psicose::tx::TxState::Aborted);

    start_coop(&mut sender, &mut receiver, &mut got, &mut n);
    send_byte_coop(&mut sender, &mut receiver, 0x20, &mut got, &mut n);
    finish_coop(&mut sender, &mut receiver, &mut got, &mut n);

    assert_eq!(got[n - 1], 0x20);
    assert_eq!(receiver.expected_seq(), 1);
}
