//! Camera → host over UART.
//!
//! A cheap CMOS module (OV2640-class) just filled a RAM buffer with a JPEG.
//! The MCU has no heap and must not copy the image. PSICOSE walks the buffer
//! one byte at a time onto a UART; the host reconstructs the file.
//!
//! ```text
//! camera RAM  --ByteSource-->  PSICOSE  --UART-->  host file / RAM
//!    JPEG                         1 B
//! ```
//!
//! Run: `cargo run --example jpeg_over_uart`

#[path = "link.rs"]
mod link;

use psicose::{SliceSink, SliceSource};

fn main() {
    // SOI + APP0 stub. A real capture is kilobytes; the path is identical.
    let camera_ram: [u8; 20] = [
        0xFF, 0xD8, // SOI
        0xFF, 0xE0, // APP0
        0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
        0x00,
    ];

    let mut src = SliceSource::new(&camera_ram);
    let mut host_file = [0u8; 20];
    let mut sink = SliceSink::new(&mut host_file);

    match link::copy_stop_and_wait(&mut src, &mut sink) {
        Ok(n) => {
            assert_eq!(n, 20);
            assert_eq!(sink.written(), &camera_ram[..]);
            assert_eq!(&host_file[0..2], &[0xFF, 0xD8]);
            println!("jpeg_over_uart: {n} bytes, SOI ok, host file matches camera RAM");
        }
        Err(e) => {
            eprintln!("jpeg_over_uart failed: {e:?}");
            std::process::exit(1);
        }
    }
}
