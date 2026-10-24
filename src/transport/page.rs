//! Firmware pages on one [`crate::LinkFace`].
//!
//! A one-way [`crate::WindowTurn`] stalls after the first session: the
//! listener never opens `START`, so the speaker is never granted another
//! turn. [`PageLink`] sends at most `N` bytes, then waits for one status
//! byte. That byte is the listener's turn, and it is what lets the next
//! page start. The 4-byte frame is unchanged.
//!
//! ```text
//! master -- START page[0..N] FINISH -->  listener
//! master <-- START status FINISH ------  listener
//! master -- START page[N..]  FINISH -->  listener
//! ```

use crate::error::Error;
use crate::transport::turn::{TurnEvent, WindowTurn};
use crate::transport::ByteTransport;

/// What one [`PageLink::poll`] observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageEvent {
    /// ACK, retry, or waiting on the peer.
    Idle,
    /// One byte of the page, in order.
    Data(u8),
    /// The page is closed. The listener must [`PageLink::ack`].
    PageClosed {
        /// How many bytes this page carried.
        len: usize,
    },
    /// The listener accepted the page with this status byte.
    Status(u8),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Sending,
    WaitStatus,
    Receiving,
    NeedAck,
    Acking,
}

/// Streams a blob as pages of at most `N` bytes over a windowed turn.
///
/// `N` is the page (and the outbound queue). Use `8` so one page fills the
/// DATA window. The speaker calls [`push_page`](Self::push_page); the
/// listener calls [`ack`](Self::ack) after [`PageEvent::PageClosed`].
pub struct PageLink<Tx, Rx, const N: usize>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    turn: WindowTurn<Tx, Rx, N>,
    phase: Phase,
    filled: usize,
    status: u8,
    have_status: bool,
}

impl<Tx, Rx, const N: usize> PageLink<Tx, Rx, N>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    /// `turn` comes from [`crate::LinkFace::window_turn`]. `first` was
    /// already chosen there: the speaker is the peer that may open `START`.
    pub const fn new(turn: WindowTurn<Tx, Rx, N>) -> Self {
        PageLink {
            turn,
            phase: Phase::Ready,
            filled: 0,
            status: 0,
            have_status: false,
        }
    }

    /// No page is in flight, so the speaker may queue the next one.
    pub const fn ready(&self) -> bool {
        matches!(self.phase, Phase::Ready)
    }

    /// Queue one page. Returns how many bytes were accepted.
    ///
    /// Returns `0` when a page is already in flight, this peer is not the
    /// speaker, or `bytes` is empty. An empty page must not open `START`:
    /// that session would never flip the turn.
    pub fn push_page(&mut self, bytes: &[u8]) -> usize {
        if self.phase != Phase::Ready || !self.turn.speaking() || bytes.is_empty() {
            return 0;
        }
        let n = self.turn.push(bytes);
        if n == 0 {
            return 0;
        }
        self.filled = n;
        self.phase = Phase::Sending;
        n
    }

    /// Answer a closed page with one status byte (the listener's turn).
    pub fn ack(&mut self, status: u8) -> bool {
        if self.phase != Phase::NeedAck {
            return false;
        }
        if self.turn.push(&[status]) != 1 {
            return false;
        }
        self.phase = Phase::Acking;
        true
    }

    /// One cooperative step. Poll both peers until the page is acknowledged.
    pub fn poll(&mut self) -> Result<PageEvent, Error<Tx::Error>> {
        let event = self.turn.poll()?;
        Ok(self.take(event))
    }

    fn take(&mut self, event: TurnEvent) -> PageEvent {
        match self.phase {
            Phase::Sending => {
                if event == TurnEvent::Yielded {
                    self.phase = Phase::WaitStatus;
                    self.filled = 0;
                    self.have_status = false;
                }
                PageEvent::Idle
            }
            Phase::WaitStatus => match event {
                TurnEvent::Received(byte) => {
                    self.status = byte;
                    self.have_status = true;
                    PageEvent::Idle
                }
                TurnEvent::Granted if self.have_status => {
                    let status = self.status;
                    self.have_status = false;
                    self.phase = Phase::Ready;
                    PageEvent::Status(status)
                }
                _ => PageEvent::Idle,
            },
            Phase::Ready | Phase::Receiving => match event {
                TurnEvent::Received(byte) => {
                    self.phase = Phase::Receiving;
                    self.filled += 1;
                    PageEvent::Data(byte)
                }
                TurnEvent::Granted if self.filled > 0 => {
                    let len = self.filled;
                    self.filled = 0;
                    self.phase = Phase::NeedAck;
                    PageEvent::PageClosed { len }
                }
                _ => PageEvent::Idle,
            },
            Phase::NeedAck => PageEvent::Idle,
            Phase::Acking => {
                if event == TurnEvent::Yielded {
                    self.phase = Phase::Ready;
                }
                PageEvent::Idle
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{ByteTransport, LinkFace};
    use core::cell::RefCell;

    const RING: usize = 512;

    struct Ring {
        buf: [u8; RING],
        head: usize,
        len: usize,
    }

    impl Ring {
        const fn new() -> Self {
            Ring {
                buf: [0; RING],
                head: 0,
                len: 0,
            }
        }

        fn push(&mut self, byte: u8) -> bool {
            if self.len == RING {
                return false;
            }
            let tail = (self.head + self.len) % RING;
            self.buf[tail] = byte;
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

    struct Bus {
        ab: RefCell<Ring>,
        ba: RefCell<Ring>,
    }

    struct End<'a> {
        bus: &'a Bus,
        is_a: bool,
    }

    impl ByteTransport for End<'_> {
        type Error = ();

        fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
            let ok = if self.is_a {
                self.bus.ab.borrow_mut().push(byte)
            } else {
                self.bus.ba.borrow_mut().push(byte)
            };
            if ok {
                Ok(())
            } else {
                Err(())
            }
        }

        fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
            Ok(if self.is_a {
                self.bus.ba.borrow_mut().pop()
            } else {
                self.bus.ab.borrow_mut().pop()
            })
        }
    }

    #[test]
    fn a_page_without_status_does_not_open_the_next() {
        let bus = Bus {
            ab: RefCell::new(Ring::new()),
            ba: RefCell::new(Ring::new()),
        };
        let host_face = LinkFace::new(End {
            bus: &bus,
            is_a: true,
        });
        let mcu_face = LinkFace::new(End {
            bus: &bus,
            is_a: false,
        });
        let mut host = host_face.page::<8>(true);
        let mut mcu = mcu_face.page::<8>(false);
        assert_eq!(host.push_page(&[]), 0);
        assert_eq!(host.push_page(&[1, 2, 3, 4]), 4);
        assert_eq!(host.push_page(&[9]), 0);

        let mut closed = false;
        for _ in 0..30_000 {
            let _ = host.poll().unwrap();
            if let PageEvent::PageClosed { len } = mcu.poll().unwrap() {
                assert_eq!(len, 4);
                closed = true;
                break;
            }
        }
        assert!(closed);
        assert!(!host.ready());
        assert_eq!(host.push_page(&[9]), 0);
    }

    #[test]
    fn three_hundred_byte_image_is_programmed_page_by_page() {
        let mut image = [0u8; 300];
        image[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
        image[4..8].copy_from_slice(&0x0800_0201u32.to_le_bytes());
        let mut i = 8usize;
        while i < image.len() {
            image[i] = (i % 251) as u8;
            i += 1;
        }

        let bus = Bus {
            ab: RefCell::new(Ring::new()),
            ba: RefCell::new(Ring::new()),
        };
        let host_face = LinkFace::new(End {
            bus: &bus,
            is_a: true,
        });
        let mcu_face = LinkFace::new(End {
            bus: &bus,
            is_a: false,
        });
        let mut host = host_face.page::<8>(true);
        let mut mcu = mcu_face.page::<8>(false);
        let mut flash = [0xFFu8; 300];
        let mut cursor = 0usize;
        let mut wrote = 0usize;
        let mut acks = 0usize;

        for _ in 0..400_000 {
            if host.ready() && cursor < image.len() {
                let end = core::cmp::min(cursor + 8, image.len());
                let n = host.push_page(&image[cursor..end]);
                assert!(n > 0);
                cursor += n;
            }
            if let PageEvent::Status(0x06) = host.poll().unwrap() {
                acks += 1;
            }
            match mcu.poll().unwrap() {
                PageEvent::Data(byte) => {
                    flash[wrote] = byte;
                    wrote += 1;
                }
                PageEvent::PageClosed { len } => {
                    assert!(len > 0 && len <= 8);
                    assert!(mcu.ack(0x06));
                }
                PageEvent::Idle | PageEvent::Status(_) => {}
            }
            if cursor == image.len() && wrote == image.len() && host.ready() && acks == 38 {
                break;
            }
        }

        assert_eq!(cursor, 300);
        assert_eq!(wrote, 300);
        assert_eq!(acks, 38);
        assert_eq!(flash, image);
        assert!(host.ready());
    }
}
