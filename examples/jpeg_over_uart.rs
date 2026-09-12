//! Camera → host over UART through the **PSICOSE motor**.
//!
//! ```text
//! camera RAM  --SliceSource-->  Wire::copy  --UART path-->  host
//! ```
//!
//! Run: `cargo run --example jpeg_over_uart`

use psicose::{SliceSink, SliceSource, Wire};

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

    match Wire::new().copy(&mut src, &mut sink) {
        Ok(n) => {
            assert_eq!(n, 20);
            assert_eq!(sink.written(), &camera_ram[..]);
            assert_eq!(&host_file[0..2], &[0xFF, 0xD8]);
            println!("jpeg_over_uart: {n} bytes, SOI ok, host file matches camera RAM");
        }
        Err(e) => {
            eprintln!("jpeg_over_uart failed: {e}");
            std::process::exit(1);
        }
    }
}
