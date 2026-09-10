//! Selective-repeat window (`N ≤ 8`), still `no_std` and heapless.
//!
//! ```text
//! PSICOSE-1B          PSICOSE-W8
//! 1 frame in flight   N frames in flight
//! [slot; 1]           [Option<Slot>; N]
//! heap = 0            heap = 0
//! ```
//!
//! The wire frame does not change. Memory is `[Option<T>; N]` plus the
//! same 4-byte `OutBuf` / assembler. A 4 GB file still never lives here —
//! only up to `N` payload bytes plus a few control bytes.
//!
//! `START` / `FINISH` stay stop-and-wait. Only DATA uses the window.

mod rx;
mod tx;

/// Hard ceiling. Bigger is still heapless; we refuse more RX reorder
/// state than eight payload bytes.
pub const MAX_WINDOW: usize = 8;

pub use rx::WindowedReceiver;
pub use tx::WindowedSender;

/// Window of 8 — the named PSICOSE-W8 sender.
pub type W8Sender<T> = WindowedSender<T, 8>;
/// Window of 8 — the named PSICOSE-W8 receiver.
pub type W8Receiver<T> = WindowedReceiver<T, 8>;

pub(crate) const fn check_window<const N: usize>() {
    assert!(N >= 1, "window N must be at least 1");
    assert!(N <= MAX_WINDOW, "window N must be at most 8");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ByteTransport;

    struct Nop;

    impl ByteTransport for Nop {
        type Error = core::convert::Infallible;
        fn write_byte(&mut self, _byte: u8) -> Result<(), Self::Error> {
            Ok(())
        }
        fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
            Ok(None)
        }
    }

    #[test]
    fn w8_state_fits_in_a_quarter_kilobyte() {
        let tx = core::mem::size_of::<WindowedSender<Nop, 8>>();
        let rx = core::mem::size_of::<WindowedReceiver<Nop, 8>>();
        assert!(tx < 256, "W8 sender is {tx} bytes — must stay a register machine");
        assert!(rx < 256, "W8 receiver is {rx} bytes — must stay a register machine");
    }

    #[test]
    fn n1_is_no_larger_than_a_handful_of_frames() {
        let tx = core::mem::size_of::<WindowedSender<Nop, 1>>();
        assert!(tx < 128, "N=1 sender is {tx} bytes");
    }
}
