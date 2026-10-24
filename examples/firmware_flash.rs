//! Host programs an MCU image one page at a time over a single link.
//!
//! Each page is at most 8 bytes (the DATA window). The MCU answers with
//! one status byte; that byte is its turn, so the next page can start.
//! A one-way transfer never flips the turn and stalls after the first page.
//!
//! ```text
//! firmware.bin --page--> LinkFace --UART--> MCU flash
//!              <-- 0x06 --
//! ```
//!
//! Run: `cargo run --example firmware_flash`

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;

use psicose::{ByteSink, ByteSource, ByteTransport, LinkFace, PageEvent};

const PAGE: usize = 8;
const IMAGE: usize = 300;
const RING: usize = 512;

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
    host_to_mcu: RefCell<Ring>,
    mcu_to_host: RefCell<Ring>,
}

struct End<'a> {
    bus: &'a Bus,
    is_host: bool,
}

impl ByteTransport for End<'_> {
    type Error = ();

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        let ok = if self.is_host {
            self.bus.host_to_mcu.borrow_mut().push(byte)
        } else {
            self.bus.mcu_to_host.borrow_mut().push(byte)
        };
        if ok {
            Ok(())
        } else {
            Err(())
        }
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        Ok(if self.is_host {
            self.bus.mcu_to_host.borrow_mut().pop()
        } else {
            self.bus.host_to_mcu.borrow_mut().pop()
        })
    }
}

fn firmware_path() -> PathBuf {
    std::env::temp_dir().join("psicose-firmware-example.bin")
}

fn write_image(path: &std::path::Path) -> Result<[u8; IMAGE], std::io::Error> {
    // Fake Cortex-M vector table (little-endian SP + reset) plus payload.
    let mut image = [0u8; IMAGE];
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

fn read_page(src: &mut FileSource, buf: &mut [u8; PAGE]) -> Result<usize, std::io::Error> {
    let mut n = 0usize;
    while n < PAGE {
        match src.read_byte()? {
            Some(byte) => {
                buf[n] = byte;
                n += 1;
            }
            None => break,
        }
    }
    Ok(n)
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

    let bus = Bus {
        host_to_mcu: RefCell::new(Ring::new()),
        mcu_to_host: RefCell::new(Ring::new()),
    };
    let host_face = LinkFace::new(End {
        bus: &bus,
        is_host: true,
    });
    let mcu_face = LinkFace::new(End {
        bus: &bus,
        is_host: false,
    });
    let mut host = host_face.page::<PAGE>(true);
    let mut mcu = mcu_face.page::<PAGE>(false);

    let mut src = FileSource { file };
    let mut flash = [0xFFu8; IMAGE];
    let mut page = [0u8; PAGE];
    let mut file_done = false;
    let mut in_flight = false;
    let mut sink = FlashSink {
        mem: &mut flash,
        pos: 0,
    };

    for _ in 0..400_000 {
        if host.ready() && !in_flight && !file_done {
            let n = match read_page(&mut src, &mut page) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("firmware_flash read failed: {e}");
                    std::process::exit(1);
                }
            };
            if n == 0 {
                file_done = true;
            } else {
                let queued = host.push_page(&page[..n]);
                if queued != n {
                    eprintln!("firmware_flash: page was not accepted");
                    std::process::exit(1);
                }
                in_flight = true;
            }
        }

        match host.poll() {
            Ok(PageEvent::Status(0x06)) => in_flight = false,
            Ok(_) => {}
            Err(e) => {
                eprintln!("firmware_flash host failed: {e:?}");
                std::process::exit(1);
            }
        }

        match mcu.poll() {
            Ok(PageEvent::Data(byte)) => {
                if sink.write_byte(byte).is_err() {
                    eprintln!("firmware_flash: flash full");
                    std::process::exit(1);
                }
            }
            Ok(PageEvent::PageClosed { .. }) => {
                if !mcu.ack(0x06) {
                    eprintln!("firmware_flash: status was rejected");
                    std::process::exit(1);
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("firmware_flash mcu failed: {e:?}");
                std::process::exit(1);
            }
        }

        if file_done && !in_flight && host.ready() && sink.pos == IMAGE {
            break;
        }
    }

    let programmed = sink.pos;
    {
        let _release = sink;
    }
    let _ = std::fs::remove_file(&path);

    if programmed != IMAGE || flash != expected {
        eprintln!("firmware_flash: image mismatch ({programmed} bytes)");
        std::process::exit(1);
    }
    let sp = u32::from_le_bytes([flash[0], flash[1], flash[2], flash[3]]);
    println!("firmware_flash: programmed {IMAGE} bytes, SP={sp:#010x} (page N={PAGE})");
}
