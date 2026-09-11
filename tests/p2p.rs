//! The P2P layer riding the real transport: hello handshake and message
//! fragments crossing a Pump, one wire byte per poll, no heap.

mod common;

use core::cell::RefCell;

use psicose::prelude::*;

use common::{End, Wires};

/// Ships `payload` across a fresh cooperative link and collects what the
/// receiver delivers. This is the transport doing its job; the P2P layer
/// only ever sees bytes in, bytes out.
fn move_bytes(payload: &[u8], out: &mut [u8]) -> usize {
    let wires = RefCell::new(Wires::new());
    let mut pump = Pump::on(
        End {
            wires: &wires,
            is_a: true,
        },
        End {
            wires: &wires,
            is_a: false,
        },
    );

    let mut i = 0usize;
    let mut hold: Option<u8> = None;
    let mut n = 0usize;
    let mut steps = 0usize;
    while steps < 1_000_000 {
        steps += 1;
        if hold.is_none() && i < payload.len() {
            hold = Some(payload[i]);
            i += 1;
        }
        if pump.sender().state() == TxState::Idle {
            if let Some(b) = hold.take() {
                if pump.sender_mut().offer(b).is_err() {
                    hold = Some(b);
                }
            } else if i >= payload.len() {
                let _ = pump.sender_mut().offer_finish();
            }
        }
        match pump.poll() {
            Ok(PumpEvent::Received(b)) => {
                if n < out.len() {
                    out[n] = b;
                    n += 1;
                }
            }
            Ok(PumpEvent::Completed) => return n,
            Ok(_) => {}
            Err(_) => return n,
        }
    }
    n
}

fn handshake_pair<const N: usize>(
    a: &mut PeerLink<DuplexPort<'_>, DuplexPort<'_>>,
    table_a: &mut PeerTable<N>,
    b: &mut PeerLink<DuplexPort<'_>, DuplexPort<'_>>,
    table_b: &mut PeerTable<N>,
) -> bool {
    let mut a_ok = false;
    let mut b_ok = false;
    let mut i = 0usize;
    while i < 100_000 {
        i += 1;
        match a.poll(table_a) {
            Ok(LinkEvent::Established) => a_ok = true,
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        match b.poll(table_b) {
            Ok(LinkEvent::Established) => b_ok = true,
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        if a_ok && b_ok {
            return true;
        }
    }
    false
}

#[test]
fn hello_handshake_crosses_the_wire() {
    let mut a = PeerSession::new(
        PeerId::from([0xAA; 8]),
        SessionConfig::DEFAULT.with_features(Capabilities::WINDOW | Capabilities::STREAM),
    );
    let mut b = PeerSession::new(
        PeerId::from([0xBB; 8]),
        SessionConfig::offer(4, Capabilities::STREAM),
    );

    let hello_a = a.connect();
    let mut wire = [0u8; HELLO_LEN];
    assert_eq!(move_bytes(&hello_a, &mut wire), HELLO_LEN);

    let reply = b.on_hello(&wire);
    assert_eq!(reply, Ok(Some(b.hello_bytes())));
    assert_eq!(b.state(), SessionState::Established);

    let hello_b = b.hello_bytes();
    assert_eq!(move_bytes(&hello_b, &mut wire), HELLO_LEN);
    assert_eq!(a.on_hello(&wire), Ok(None));
    assert_eq!(a.state(), SessionState::Established);

    assert_eq!(a.remote(), Some(PeerId::from([0xBB; 8])));
    assert_eq!(b.remote(), Some(PeerId::from([0xAA; 8])));
    let expected = SessionConfig::offer(4, Capabilities::STREAM);
    assert_eq!(a.negotiated(), Some(expected));
    assert_eq!(b.negotiated(), Some(expected));
}

#[test]
fn forum_post_is_fragmented_shipped_and_rebuilt() {
    let post = [
        0x48, 0x6F, 0x6C, 0xC3, 0xA1, 0x20, 0x6D, 0x75, 0x6E, 0x64, 0x6F,
    ];
    let mut frag = Fragmenter::new(StreamId::new(1), MessageId::new(42), &post, 4);

    let mut rebuilt = [0u8; 16];
    let mut filled = 0usize;
    let mut fragments = 0u16;
    while let Some((header, chunk)) = frag.next_fragment() {
        let mut packet = [0u8; HEADER_LEN + 4];
        packet[..HEADER_LEN].copy_from_slice(&header.to_bytes());
        packet[HEADER_LEN..HEADER_LEN + chunk.len()].copy_from_slice(chunk);

        let mut got = [0u8; HEADER_LEN + 4];
        let n = move_bytes(&packet[..HEADER_LEN + chunk.len()], &mut got);
        assert_eq!(n, HEADER_LEN + chunk.len());

        let mut hdr = [0u8; HEADER_LEN];
        hdr.copy_from_slice(&got[..HEADER_LEN]);
        assert_eq!(MessageHeader::from_bytes(hdr), Ok(header));
        assert_eq!(header.fragment, fragments);

        let len = header.len as usize;
        rebuilt[filled..filled + len].copy_from_slice(&got[HEADER_LEN..HEADER_LEN + len]);
        filled += len;
        fragments += 1;
    }

    assert_eq!(fragments, 3);
    assert_eq!(&rebuilt[..filled], &post);
}

#[test]
fn abort_then_reconnect_reuses_the_endpoint() {
    let cfg = SessionConfig::offer(2, Capabilities::STREAM);
    let mut a = PeerSession::new(PeerId::from([1; 8]), cfg);
    let mut b = PeerSession::new(PeerId::from([2; 8]), cfg);

    let hello = a.connect();
    let mut wire = [0u8; HELLO_LEN];
    assert_eq!(move_bytes(&hello, &mut wire), HELLO_LEN);
    assert!(matches!(b.on_hello(&wire), Ok(Some(_))));
    assert_eq!(b.state(), SessionState::Established);

    a.abort();
    b.abort();
    assert_eq!(a.state(), SessionState::Aborted);
    assert_eq!(b.state(), SessionState::Aborted);

    let hello = a.connect();
    assert_eq!(a.state(), SessionState::Connecting);
    assert_eq!(move_bytes(&hello, &mut wire), HELLO_LEN);
    assert!(matches!(b.on_hello(&wire), Ok(Some(_))));
    assert_eq!(b.state(), SessionState::Established);
    assert_eq!(b.remote(), Some(PeerId::from([1; 8])));
}

#[test]
fn connect_and_accept_establish_a_and_b() {
    let id_a = PeerId::from([0xAA; 8]);
    let id_b = PeerId::from([0xBB; 8]);
    let mut alice = PeerTable::<4>::new(id_a);
    let mut bob = PeerTable::<4>::new(id_b);

    let wire = Wire::new();
    let (pump_a, pump_b) = wire.pumps();
    let connected = PeerLink::connect(&mut alice, pump_a);
    assert!(matches!(connected, Ok(_)));
    let mut a = match connected {
        Ok(link) => link,
        Err(_) => return,
    };
    let mut b = PeerLink::accept(&bob, pump_b);

    assert_eq!(handshake_pair(&mut a, &mut alice, &mut b, &mut bob), true);
    assert_eq!(a.session().state(), SessionState::Established);
    assert_eq!(b.session().state(), SessionState::Established);
    assert_eq!(a.session().remote(), Some(id_b));
    assert_eq!(b.session().remote(), Some(id_a));
    assert_eq!(alice.find(id_b), Some(0));
    assert_eq!(bob.find(id_a), Some(0));
    assert_eq!(alice.established(), 1);
    assert_eq!(bob.established(), 1);
    assert_eq!(a.session().negotiated(), b.session().negotiated());
}

#[test]
fn established_link_delivers_a_data_byte() {
    let mut alice = PeerTable::<2>::new(PeerId::from([1; 8]));
    let mut bob = PeerTable::<2>::new(PeerId::from([2; 8]));
    let wire = Wire::new();
    let (pump_a, pump_b) = wire.pumps();
    let connected = PeerLink::connect(&mut alice, pump_a);
    assert!(matches!(connected, Ok(_)));
    let mut a = match connected {
        Ok(link) => link,
        Err(_) => return,
    };
    let mut b = PeerLink::accept(&bob, pump_b);
    assert_eq!(handshake_pair(&mut a, &mut alice, &mut b, &mut bob), true);

    assert_eq!(a.offer(0x7E), Ok(()));
    let mut got = None;
    let mut i = 0usize;
    while i < 10_000 {
        i += 1;
        let _ = a.poll(&mut alice);
        match b.poll(&mut bob) {
            Ok(LinkEvent::Received(byte)) => {
                got = Some(byte);
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert_eq!(got, Some(0x7E));
}

#[test]
fn table_of_one_rejects_a_second_connect() {
    let mut table = PeerTable::<1>::new(PeerId::from([1; 8]));
    assert!(matches!(table.connect(), Ok((0, _))));
    assert_eq!(table.connect(), Err(TableError::Full));
}
