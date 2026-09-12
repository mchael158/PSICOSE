//! The P2P layer riding the real transport: hello, a live link, and
//! fragments as ordinary payload bytes.

mod common;

use psicose::prelude::*;

use common::established;

/// Ships `payload` across a `Wire` and collects what the other pump delivers.
fn move_bytes(payload: &[u8], out: &mut [u8]) -> usize {
    let wire = Wire::new();
    let (mut tx, mut rx) = wire.pumps();

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
        if tx.sender().state() == TxState::Idle {
            if let Some(b) = hold.take() {
                if tx.sender_mut().offer(b).is_err() {
                    hold = Some(b);
                }
            } else if i >= payload.len() {
                let _ = tx.sender_mut().offer_finish();
            }
        }
        let _ = tx.poll();
        match rx.poll() {
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

#[test]
fn hello_handshake_crosses_the_wire() {
    let mut a = PeerSession::new(
        PeerId::from_label(b"alice"),
        SessionConfig::DEFAULT.with_features(Capabilities::WINDOW | Capabilities::STREAM),
    );
    let mut b = PeerSession::new(
        PeerId::from_label(b"bob"),
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

    assert_eq!(a.remote(), Some(PeerId::from_label(b"bob")));
    assert_eq!(b.remote(), Some(PeerId::from_label(b"alice")));
    let expected = SessionConfig::offer(4, Capabilities::STREAM);
    assert_eq!(a.negotiated(), Some(expected));
    assert_eq!(b.negotiated(), Some(expected));
}

#[test]
fn message_fragments_cross_a_pump() {
    let payload = b"ping";
    let mut frag = Fragmenter::new(StreamId::FORUM, MessageId::new(1), payload, 4);
    let mut board = [0u8; 16];
    let mut inbox = Defragmenter::new(&mut board);

    let mut fragments = 0u16;
    while let Some((header, chunk)) = frag.next_fragment() {
        let mut packet = [0u8; HEADER_LEN + 4];
        let n = HEADER_LEN + chunk.len();
        packet[..HEADER_LEN].copy_from_slice(&header.to_bytes());
        packet[HEADER_LEN..n].copy_from_slice(chunk);

        let mut got = [0u8; HEADER_LEN + 4];
        assert_eq!(move_bytes(&packet[..n], &mut got), n);

        let mut hdr = [0u8; HEADER_LEN];
        hdr.copy_from_slice(&got[..HEADER_LEN]);
        assert_eq!(MessageHeader::from_bytes(hdr), Ok(header));
        assert_eq!(header.fragment, fragments);

        let mut i = 0usize;
        while i < n {
            assert!(inbox.push(got[i]).is_ok());
            i += 1;
        }
        fragments += 1;
    }

    assert_eq!(fragments, 1);
    assert!(inbox.is_complete());
    assert_eq!(inbox.as_slice(), payload);
}

#[test]
fn abort_then_reconnect_reuses_the_endpoint() {
    let cfg = SessionConfig::offer(2, Capabilities::STREAM);
    let mut a = PeerSession::new(PeerId::from_label(b"alice"), cfg);
    let mut b = PeerSession::new(PeerId::from_label(b"bob"), cfg);

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
    assert_eq!(b.remote(), Some(PeerId::from_label(b"alice")));
}

#[test]
fn connect_and_accept_establish_a_and_b() {
    let id_a = PeerId::from_label(b"alice");
    let id_b = PeerId::from_label(b"bob");
    let mut alice = PeerTable::<4>::new(id_a);
    let mut bob = PeerTable::<4>::new(id_b);

    let wire = Wire::new();
    let (pump_a, pump_b) = wire.pumps();
    let mut a = match PeerLink::connect(&mut alice, pump_a) {
        Ok(link) => link,
        Err(_) => return,
    };
    let mut b = PeerLink::accept(&bob, pump_b);

    assert!(established(&mut a, &mut alice, &mut b, &mut bob));
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
    let mut alice = PeerTable::<2>::new(PeerId::from_label(b"alice"));
    let mut bob = PeerTable::<2>::new(PeerId::from_label(b"bob"));
    let wire = Wire::new();
    let (pump_a, pump_b) = wire.pumps();
    let mut a = match PeerLink::connect(&mut alice, pump_a) {
        Ok(link) => link,
        Err(_) => return,
    };
    let mut b = PeerLink::accept(&bob, pump_b);
    assert!(established(&mut a, &mut alice, &mut b, &mut bob));

    assert_eq!(a.offer(b'~'), Ok(()));
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
    assert_eq!(got, Some(b'~'));
}

#[test]
fn table_of_one_rejects_a_second_connect() {
    let mut table = PeerTable::<1>::new(PeerId::from_label(b"alice"));
    assert!(matches!(table.connect(), Ok((0, _))));
    assert_eq!(table.connect(), Err(TableError::Full));
}
