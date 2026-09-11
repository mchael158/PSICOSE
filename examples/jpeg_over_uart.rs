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
    // Tiny JPEG: SOI + APP0 "JFIF". A real capture is kilobytes; the path is identical.
    let mut camera_ram = [0u8; 20];
    camera_ram[0] = 0xFF;
    camera_ram[1] = 0xD8; // start of image
    camera_ram[2] = 0xFF;
    camera_ram[3] = 0xE0; // APP0
    camera_ram[4] = 0x00;
    camera_ram[5] = 0x10; // APP0 length
    camera_ram[6..11].copy_from_slice(b"JFIF\0");
    camera_ram[11] = 1;
    camera_ram[12] = 1;
    camera_ram[15] = 1;
    camera_ram[17] = 1;

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
