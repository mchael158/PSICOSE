//! Field node sends a packed weather sample on the **hardware path**.
//!
//! ```text
//! sample [u8; 8] --> LinkFace::window_turn --> gateway sample
//! ```
//!
//! Same entry a UART board uses (`WindowTurn`, window of 8). The byte
//! pipe below stands in for the wire. `Wire` is not this path.
//!
//! Run: `cargo run --example sensor_telemetry`

use core::cell::RefCell;

use psicose::{ByteTransport, LinkFace, TurnEvent};

/// 23.1 °C, 47% RH, 3710 mV, flags 0.
const SAMPLE: [u8; 8] = [0xA5, 0xE7, 0x00, 0x2F, 0x7E, 0x0E, 0x00, 0x1D];
/// Gateway answer: 22.0 °C, 40% RH, 3600 mV, flags 1.
const ANSWER: [u8; 8] = [0xA5, 0xDC, 0x00, 0x28, 0x10, 0x0E, 0x01, 0x4E];

fn unpack(bytes: &[u8; 8]) -> Option<(i16, u8, u16, u8)> {
    if bytes[0] != 0xA5 {
        return None;
    }
    let mut xor = 0u8;
    let mut i = 0usize;
    while i < 7 {
        xor ^= bytes[i];
        i += 1;
    }
    if xor != bytes[7] {
        return None;
    }
    let temp = i16::from_le_bytes([bytes[1], bytes[2]]);
    let humidity = bytes[3];
    let battery = u16::from_le_bytes([bytes[4], bytes[5]]);
    Some((temp, humidity, battery, bytes[6]))
}

const RING: usize = 256;

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
        let byte = if self.is_a {
            self.bus.ba.borrow_mut().pop()
        } else {
            self.bus.ab.borrow_mut().pop()
        };
        Ok(byte)
    }
}

fn main() {
    let bus = Bus {
        ab: RefCell::new(Ring::new()),
        ba: RefCell::new(Ring::new()),
    };
    let node = LinkFace::new(End {
        bus: &bus,
        is_a: true,
    });
    let gateway = LinkFace::new(End {
        bus: &bus,
        is_a: false,
    });
    let mut node = node.window_turn::<8>(true);
    let mut gateway = gateway.window_turn::<8>(false);
    assert_eq!(node.push(&SAMPLE), 8);
    assert_eq!(gateway.push(&ANSWER), 8);

    let mut got_node = [0u8; 8];
    let mut got_gw = [0u8; 8];
    let mut nn = 0usize;
    let mut ng = 0usize;
    let mut node_yielded = false;

    for _ in 0..40_000 {
        match node.poll() {
            Ok(TurnEvent::Received(b)) => {
                got_node[nn] = b;
                nn += 1;
            }
            Ok(TurnEvent::Yielded) => node_yielded = true,
            Ok(_) => {}
            Err(e) => {
                eprintln!("sensor_telemetry node: {e:?}");
                std::process::exit(1);
            }
        }
        match gateway.poll() {
            Ok(TurnEvent::Received(b)) => {
                got_gw[ng] = b;
                ng += 1;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("sensor_telemetry gateway: {e:?}");
                std::process::exit(1);
            }
        }
        if node_yielded && nn == 8 && ng == 8 {
            break;
        }
    }

    if ng != 8 || nn != 8 {
        eprintln!("sensor_telemetry stalled node={nn} gateway={ng}");
        std::process::exit(1);
    }
    let (temp, rh, mv, flags) = unpack(&got_gw).unwrap_or_else(|| {
        eprintln!("gateway rejected sample");
        std::process::exit(1);
    });
    println!(
        "sensor_telemetry: 8 B via window_turn  T={}°C  RH={rh}%  batt={mv} mV  flags={flags:#04x}",
        temp as f32 / 10.0
    );
}
