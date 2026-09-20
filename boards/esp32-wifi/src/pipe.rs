//! Shared RX/TX rings so async Wi‑Fi/TCP can feed a sync [`psicose::ByteTransport`].
//!
//! Embassy/`esp-radio` stay in `main`; PSICOSE only sees this pipe.

use core::cell::RefCell;

use critical_section::Mutex;
use psicose::ByteTransport;

const RING: usize = 512;

#[derive(Clone, Copy)]
struct Ring {
    buf: [u8; RING],
    head: usize,
    tail: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Self {
        Ring {
            buf: [0; RING],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) -> bool {
        if self.len == RING {
            return false;
        }
        self.buf[self.tail] = byte;
        self.tail = (self.tail + 1) % RING;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.buf[self.head];
        self.head = (self.head + 1) % RING;
        self.len -= 1;
        Some(byte)
    }
}

struct Ends {
    /// Bytes from TCP → PSICOSE.
    from_net: Ring,
    /// Bytes from PSICOSE → TCP.
    to_net: Ring,
}

/// Process-wide pipe (one TCP session in the demo).
static PIPE: Mutex<RefCell<Ends>> = Mutex::new(RefCell::new(Ends {
    from_net: Ring::new(),
    to_net: Ring::new(),
}));

/// Push a byte received from the socket into the PSICOSE RX side.
pub fn net_push_rx(byte: u8) -> bool {
    critical_section::with(|cs| PIPE.borrow_ref_mut(cs).from_net.push(byte))
}

/// Pop a byte that PSICOSE wants sent on the socket.
pub fn net_pop_tx() -> Option<u8> {
    critical_section::with(|cs| PIPE.borrow_ref_mut(cs).to_net.pop())
}

/// [`ByteTransport`] face used by [`psicose::LinkFace`].
pub struct TcpPipe;

#[derive(Debug, Clone, Copy)]
pub struct PipeFull;

impl ByteTransport for TcpPipe {
    type Error = PipeFull;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        let ok = critical_section::with(|cs| PIPE.borrow_ref_mut(cs).to_net.push(byte));
        if ok {
            Ok(())
        } else {
            Err(PipeFull)
        }
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        Ok(critical_section::with(|cs| {
            PIPE.borrow_ref_mut(cs).from_net.pop()
        }))
    }
}
