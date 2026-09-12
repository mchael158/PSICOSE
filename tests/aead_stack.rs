//! seal → PeerLink bytes → open. Feature `aead` only.

mod common;

use psicose::prelude::*;

use common::{established, send};

#[test]
fn sealed_bytes_survive_the_link() {
    let key = [0x11u8; KEY_LEN];
    let nonce = [0x22u8; NONCE_LEN];
    let plain = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

    let mut sealed = [0u8; 32];
    let n = seal_to(&key, &nonce, b"", &plain, &mut sealed).expect("seal");
    assert_eq!(Some(n), sealed_len(plain.len()));

    let cfg = SessionConfig::SECURE;
    let wire = Wire::new();
    let (pump_a, pump_b) = wire.pumps();

    let mut alice = PeerTable::<4>::with(PeerId::from_label(b"alice"), cfg);
    let mut bob = PeerTable::<4>::with(PeerId::from_label(b"bob"), cfg);

    let mut a = match PeerLink::connect(&mut alice, pump_a) {
        Ok(link) => link,
        Err(_) => return,
    };
    let mut b = PeerLink::accept(&bob, pump_b);
    assert!(established(&mut a, &mut alice, &mut b, &mut bob));

    let mut board = [0u8; 64];
    let mut inbox = Defragmenter::new(&mut board);
    assert!(send(
        &mut a,
        &mut alice,
        &mut b,
        &mut bob,
        1,
        &sealed[..n],
        &mut inbox
    ));
    assert_eq!(inbox.as_slice(), &sealed[..n]);

    let mut out = [0u8; 16];
    let m = open_from(&key, &nonce, b"", inbox.as_slice(), &mut out).expect("open");
    assert_eq!(m, plain.len());
    assert_eq!(&out[..m], &plain);
}
