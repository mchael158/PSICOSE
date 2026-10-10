//! PSICOSE on ESP32 classic — Wi‑Fi TCP link demo.
//!
//! ```text
//! Wi‑Fi STA → embassy-net TcpSocket → TcpPipe (rings) → LinkFace → Turn
//! ```
//!
//! - **`psicose`** stays zero-deps (`psicose::…` only).
//! - **`esp-radio` / Embassy** live only in this board package.
//!
//! ## Setup
//!
//! 1. Set `SSID` / `PASSWORD` (and optionally `HOST`) before build.
//! 2. On the PC (same LAN), run:
//!    `cargo run --example turn_pair -- --listen 0.0.0.0:19876`
//!    That peer answers `pong`. This board opens with `ping` (`FIRST = true`).
//! 3. Flash this board. It connects to `HOST:PORT`.
//!
//! ```sh
//! cd boards
//! $env:SSID="my-ap"; $env:PASSWORD="secret"; $env:HOST="192.168.1.10"
//! cargo run -p esp32-wifi
//! ```

#![no_std]
#![no_main]

extern crate alloc;

mod pipe;

use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Ipv4Address, Runner, StackResources};
use embassy_time::{Duration, Timer, WithTimeout};
use embedded_io_async::Write;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_radio::wifi::{
    scan::ScanConfig, sta::StationConfig, AuthenticationMethod, Config, ControllerConfig,
    Interface, WifiController,
};
use psicose::{LinkFace, TurnEvent};
use static_cell::StaticCell;

use pipe::TcpPipe;

esp_bootloader_esp_idf::esp_app_desc!();

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: StaticCell<$t> = StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

/// `true` = open with `ping`. The host `turn_pair --listen` answers `pong`.
/// Do not set `true` on both ends: both would `START` at once.
const FIRST: bool = true;

const OURS: &[u8] = if FIRST { b"ping" } else { b"pong" };
const THEIRS: &[u8] = if FIRST { b"pong" } else { b"ping" };
const TCP_PORT: u16 = 19876;

const SSID: &str = match option_env!("SSID") {
    Some(s) => s,
    None => "psicose",
};
const PASSWORD: &str = match option_env!("PASSWORD") {
    Some(s) => s,
    None => "psicose",
};
/// IPv4 of the PC running `turn_pair --listen` (four dotted decimals).
const HOST: &str = match option_env!("HOST") {
    Some(s) => s,
    None => "192.168.1.10",
};

fn parse_host(s: &str) -> Option<Ipv4Address> {
    let mut parts = [0u8; 4];
    let mut i = 0usize;
    for piece in s.split('.') {
        if i >= 4 {
            return None;
        }
        parts[i] = piece.parse().ok()?;
        i += 1;
    }
    if i != 4 {
        return None;
    }
    Some(Ipv4Address::new(parts[0], parts[1], parts[2], parts[3]))
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let software_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, software_interrupt.software_interrupt0);

    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(alloc::string::String::from(PASSWORD))
            .with_auth_method(AuthenticationMethod::Wpa2Personal),
    );

    println!("psicose esp32-wifi: starting STA ssid={SSID}");
    let (mut controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();
    let wifi_interface = interfaces.station;

    let net_config = embassy_net::Config::dhcpv4(Default::default());
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    println!("scan (max 10 APs)…");
    let scan_config = ScanConfig::default().with_max(10);
    match controller.scan_async(&scan_config).await {
        Ok(aps) => {
            for ap in aps {
                println!("  ap: {ap:?}");
            }
        }
        Err(e) => println!("scan failed: {e:?}"),
    }

    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        println!("DHCP: {}", cfg.address);
    }

    let host = parse_host(HOST).unwrap_or_else(|| {
        println!("bad HOST={HOST}, using 192.168.1.10");
        Ipv4Address::new(192, 168, 1, 10)
    });
    println!("TCP connect {host}:{TCP_PORT} first={FIRST}");

    let mut rx_buf = [0u8; 1024];
    let mut tx_buf = [0u8; 1024];
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.set_timeout(Some(Duration::from_secs(30)));

    loop {
        match socket.connect((host, TCP_PORT)).await {
            Ok(()) => {
                println!("TCP up — starting PSICOSE turn");
                run_link(&mut socket).await;
                println!("session ended; reconnecting…");
            }
            Err(e) => {
                println!("TCP connect failed: {e:?}");
                Timer::after(Duration::from_secs(3)).await;
            }
        }
        socket.abort();
        let _ = socket.flush().await;
        Timer::after(Duration::from_secs(1)).await;
    }
}

/// Drive PSICOSE and TCP in one task so the pipe cannot deadlock.
async fn run_link(socket: &mut TcpSocket<'_>) {
    let face = LinkFace::new(TcpPipe);
    let mut turn = face.turn::<8>(FIRST);
    let _ = turn.push(OURS);

    let mut got = [0u8; 4];
    let mut got_n = 0usize;
    let mut yielded = false;
    let mut granted = false;
    // Bytes already taken from TCP that did not fit in the RX ring.
    let mut stash = [0u8; 64];
    let mut stash_n = 0usize;
    let mut stash_i = 0usize;

    loop {
        // Drain PSICOSE → TCP first so `write_byte` keeps accepting.
        if drain_tx(socket).await.is_err() {
            return;
        }

        while stash_i < stash_n {
            if !pipe::net_push_rx(stash[stash_i]) {
                break;
            }
            stash_i += 1;
        }
        if stash_i == stash_n {
            stash_i = 0;
            stash_n = 0;
        }

        // Read only what the ring can hold. A full ring waits: TCP
        // backpressure holds the rest until PSICOSE drains.
        if stash_n == 0 {
            let free = pipe::net_rx_free();
            if free > 0 {
                let mut chunk = [0u8; 64];
                let want = free.min(chunk.len());
                match socket
                    .read(&mut chunk[..want])
                    .with_timeout(Duration::from_millis(2))
                    .await
                {
                    Ok(Ok(0)) => return,
                    Ok(Ok(n)) => {
                        for &b in &chunk[..n] {
                            if pipe::net_push_rx(b) {
                                continue;
                            }
                            stash[stash_n] = b;
                            stash_n += 1;
                        }
                    }
                    Ok(Err(_)) => return,
                    Err(_timeout) => {}
                }
            }
        }

        match turn.poll() {
            Ok(TurnEvent::Received(b)) => {
                if got_n < got.len() {
                    got[got_n] = b;
                    got_n += 1;
                    if got_n == THEIRS.len() {
                        println!(
                            "psicose: got {:?}",
                            core::str::from_utf8(&got[..got_n]).unwrap_or("?")
                        );
                    }
                }
            }
            Ok(TurnEvent::Yielded) => {
                println!("psicose: our turn done");
                yielded = true;
            }
            Ok(TurnEvent::Granted) => {
                println!("psicose: our turn");
                granted = true;
            }
            Ok(TurnEvent::Idle) => {}
            Err(_) => {
                // TX ring full: the byte stayed in the sender. Drain and retry.
                if drain_tx(socket).await.is_err() {
                    return;
                }
                Timer::after(Duration::from_millis(1)).await;
            }
        }

        if yielded && granted && got_n == THEIRS.len() {
            // FINISH ACK may still be sitting in the TX ring.
            if drain_tx(socket).await.is_err() {
                return;
            }
            println!("psicose: exchange done");
            return;
        }
    }
}

async fn drain_tx(socket: &mut TcpSocket<'_>) -> Result<(), ()> {
    while let Some(byte) = pipe::net_pop_tx() {
        if socket.write_all(&[byte]).await.is_err() {
            return Err(());
        }
    }
    Ok(())
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("wifi connection task");
    loop {
        println!("wifi: connecting…");
        match controller.connect_async().await {
            Ok(info) => {
                println!("wifi: connected {info:?}");
                let info = controller.wait_for_disconnect_async().await.ok();
                println!("wifi: disconnected {info:?}");
            }
            Err(e) => println!("wifi: connect failed {e:?}"),
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}
