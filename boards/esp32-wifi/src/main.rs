//! PSICOSE on ESP32 classic — Wi‑Fi TCP link demo.
//!
//! ```text
//! Wi‑Fi STA → embassy-net TcpSocket → TcpPipe (rings) → LinkFace → Pump
//! ```
//!
//! - **`psicose`** stays zero-deps (`psicose::…` only).
//! - **`esp-radio` / Embassy** live only in this board package.
//!
//! ## Setup
//!
//! 1. Set `SSID` / `PASSWORD` (and optionally `HOST`) before build.
//! 2. On the PC (same LAN), run:
//!    `cargo run --example tcp_pair -- --listen 0.0.0.0:19876`
//! 3. Flash this board (`INITIATOR = true` connects to `HOST:PORT`).
//!
//! ```sh
//! cd boards
//! $env:SSID="my-ap"; $env:PASSWORD="secret"; $env:HOST="192.168.1.10"
//! cargo run -p esp32-wifi
//! ```

#![no_std]
#![no_main]

mod pipe;

use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Ipv4Address, Runner, StackResources};
use embassy_time::{Duration, Timer, WithTimeout};
use embedded_io_async::{Read, Write};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
    scan::ScanConfig, sta::StationConfig,
};
use psicose::{LinkFace, PumpEvent, TxState};
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

/// `true` = send `ping` after TCP is up. Host `--listen` should stay receive-only.
const INITIATOR: bool = true;

const PING: &[u8] = b"ping";
const TCP_PORT: u16 = 19876;

const SSID: &str = match option_env!("SSID") {
    Some(s) => s,
    None => "psicose",
};
const PASSWORD: &str = match option_env!("PASSWORD") {
    Some(s) => s,
    None => "psicose",
};
/// IPv4 of the PC running `tcp_pair --listen` (four dotted decimals).
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

#[esp_hal::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().unwrap(),
            )),
    );

    println!("psicose esp32-wifi: starting STA ssid={SSID}");
    let wifi_interface = Interface::station();
    let mut controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

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
    println!("TCP connect {host}:{TCP_PORT} initiator={INITIATOR}");

    let mut rx_buf = [0u8; 1024];
    let mut tx_buf = [0u8; 1024];
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.set_timeout(Some(Duration::from_secs(30)));

    loop {
        match socket.connect((host, TCP_PORT)).await {
            Ok(()) => {
                println!("TCP up — starting PSICOSE pump");
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
    let mut pump = face.pump();

    let mut need_start = INITIATOR;
    let mut out_i = 0usize;
    let mut finish_offered = false;
    let mut got = [0u8; 4];
    let mut got_n = 0usize;
    let mut done_rounds = 0u32;

    loop {
        // Drain PSICOSE → TCP first so `write_byte` keeps accepting.
        while let Some(byte) = pipe::net_pop_tx() {
            if socket.write_all(&[byte]).await.is_err() {
                return;
            }
        }

        // Short TCP read into the RX ring.
        let mut chunk = [0u8; 64];
        match socket
            .read(&mut chunk)
            .with_timeout(Duration::from_millis(2))
            .await
        {
            Ok(Ok(0)) => return,
            Ok(Ok(n)) => {
                for &b in &chunk[..n] {
                    if !pipe::net_push_rx(b) {
                        println!("RX ring full — dropping");
                        break;
                    }
                }
            }
            Ok(Err(_)) => return,
            Err(_timeout) => {}
        }

        if INITIATOR {
            let st = pump.sender().state();
            if matches!(
                st,
                TxState::Idle | TxState::Finished | TxState::Aborted | TxState::Failed
            ) {
                if need_start {
                    if pump.sender_mut().offer_start().is_ok() {
                        need_start = false;
                        out_i = 0;
                        finish_offered = false;
                        got_n = 0;
                    }
                } else if st == TxState::Idle {
                    if out_i < PING.len() {
                        if pump.sender_mut().offer(PING[out_i]).is_ok() {
                            out_i += 1;
                        }
                    } else if !finish_offered {
                        if pump.sender_mut().offer_finish().is_ok() {
                            finish_offered = true;
                        }
                    }
                }
            }
        }

        match pump.poll() {
            Ok(PumpEvent::Received(b)) => {
                if got_n < got.len() {
                    got[got_n] = b;
                    got_n += 1;
                }
                if got_n == PING.len() {
                    println!(
                        "psicose: got {:?}",
                        core::str::from_utf8(&got[..got_n]).unwrap_or("?")
                    );
                    got_n = 0;
                }
            }
            Ok(PumpEvent::Completed) => {
                println!("psicose: session completed");
                done_rounds += 1;
                if done_rounds >= 1 {
                    return;
                }
                if INITIATOR {
                    need_start = true;
                }
            }
            Ok(PumpEvent::Aborted) => {
                println!("psicose: aborted");
                if INITIATOR {
                    need_start = true;
                }
            }
            Ok(_) => {}
            Err(e) => {
                println!("psicose poll err: {e:?}");
                let _ = pump.abort();
                return;
            }
        }
    }
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
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
