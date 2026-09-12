//! Field node sends a packed weather sample over a slow radio.
//!
//! The node has a DHT22 + ADC battery sense. RAM is tens of bytes. The
//! sample is a packed struct, not a JSON string. PSICOSE does not know
//! about fields — it only ships the 8 bytes. The gateway rebuilds the
//! struct and prints engineering units.
//!
//! ```text
//! DHT22 / ADC  -->  [u8; 8]  -->  PSICOSE  -->  gateway
//!                   packed
//! ```
//!
//! Run: `cargo run --example sensor_telemetry`

#[path = "common/link.rs"]
mod link;

use psicose::{SliceSink, SliceSource};

/// On-wire layout (little-endian), 8 bytes:
/// `magic | temp_c_x10_le | humidity | battery_mv_le | flags | xor`
fn pack_sample(temp_c_x10: i16, humidity: u8, battery_mv: u16, flags: u8) -> [u8; 8] {
    let t = temp_c_x10.to_le_bytes();
    let b = battery_mv.to_le_bytes();
    let mut frame = [0xA5, t[0], t[1], humidity, b[0], b[1], flags, 0];
    let mut xor = 0u8;
    let mut i = 0usize;
    while i < 7 {
        xor ^= frame[i];
        i += 1;
    }
    frame[7] = xor;
    frame
}

fn unpack_sample(bytes: &[u8; 8]) -> Option<(i16, u8, u16, u8)> {
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
    let flags = bytes[6];
    Some((temp, humidity, battery, flags))
}

fn main() {
    // 23.1 °C, 47 % RH, 3.710 V, flag "heater off".
    let sample = pack_sample(231, 47, 3710, 0x00);

    let mut src = SliceSource::new(&sample);
    let mut radio_rx = [0u8; 8];
    let mut sink = SliceSink::new(&mut radio_rx);

    match link::copy_stop_and_wait(&mut src, &mut sink) {
        Ok(n) => {
            assert_eq!(n, 8);
            assert_eq!(sink.written(), &sample[..]);
            match unpack_sample(&radio_rx) {
                Some((temp, rh, mv, flags)) => {
                    println!(
                        "sensor_telemetry: {n} B  T={}°C  RH={rh}%  batt={} mV  flags={flags:#04x}",
                        temp as f32 / 10.0,
                        mv
                    );
                }
                None => {
                    eprintln!("gateway rejected sample (magic/xor)");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("sensor_telemetry failed: {e:?}");
            std::process::exit(1);
        }
    }
}
