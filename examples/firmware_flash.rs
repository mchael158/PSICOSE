//! Host streams `firmware.bin` into MCU flash, one byte at a time.
//!
//! A 4 GB image would use this same `FileSource`. PSICOSE never holds more
//! than the current payload byte. On the device, `FlashSink` is a NOR/NAND
//! program of address `base + pos`.
//!
//! ```text
//! host disk  --read(1)-->  PSICOSE  --UART-->  MCU flash program(1)
//! firmware.bin
//! ```
//!
//! Run: `cargo run --example firmware_flash`

#[path = "common/link.rs"]
mod link;

use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;

use psicose::{ByteSink, ByteSource};

struct FileSource {
    file: File,
}

impl ByteSource for FileSource {
    type Error = std::io::Error;

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        let mut buf = [0u8; 1];
        match self.file.read(&mut buf) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(buf[0])),
            Err(e) => Err(e),
        }
    }
}

/// Simulated NOR flash: program one byte at `base + pos`.
struct FlashSink<'a> {
    mem: &'a mut [u8],
    pos: usize,
}

impl ByteSink for FlashSink<'_> {
    type Error = ();

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        if self.pos >= self.mem.len() {
            return Err(());
        }
        self.mem[self.pos] = byte;
        self.pos += 1;
        Ok(())
    }
}

fn firmware_path() -> PathBuf {
    std::env::temp_dir().join("psicose-firmware-example.bin")
}

fn write_image(path: &std::path::Path) -> Result<[u8; 300], std::io::Error> {
    // Fake Cortex-M vector table (little-endian SP + reset) plus payload.
    let mut image = [0u8; 300];
    image[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    image[4..8].copy_from_slice(&0x0800_0201u32.to_le_bytes());
    let mut i = 8usize;
    while i < image.len() {
        image[i] = (i % 251) as u8;
        i += 1;
    }
    let mut f = File::create(path)?;
    f.write_all(&image)?;
    Ok(image)
}

fn main() {
    let path = firmware_path();
    let expected = match write_image(&path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("could not create firmware.bin: {e}");
            std::process::exit(1);
        }
    };

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("could not open firmware.bin: {e}");
            std::process::exit(1);
        }
    };

    let mut src = FileSource { file };
    let mut flash = [0xFFu8; 300];
    let mut sink = FlashSink {
        mem: &mut flash,
        pos: 0,
    };

    match link::copy_windowed::<_, _, 8>(&mut src, &mut sink) {
        Ok(n) => {
            assert_eq!(n, 300);
            assert_eq!(flash, expected);
            let sp = u32::from_le_bytes([flash[0], flash[1], flash[2], flash[3]]);
            println!("firmware_flash: programmed {n} bytes, SP={sp:#010x} (window N=8)");
        }
        Err(e) => {
            eprintln!("firmware_flash failed: {e:?}");
            std::process::exit(1);
        }
    }

    let _ = std::fs::remove_file(&path);
}
