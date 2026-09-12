//! Dumps raw AAP battery packets and what each entry decodes to.
//!
//! This is the tool the disconnected-component behaviour was verified with. A
//! component the device is not currently talking to - the case, the moment the
//! earbuds come out of it - still appears in the packet, carrying a level byte of
//! 0 and status 0x04. Printing the raw bytes beside the decoded reading is what
//! makes that visible:
//!
//! ```text
//! RAW   04 00 04 00 04 00 03 02 01 64 02 01 04 01 64 02 01 08 01 00 04 01
//!   left  level 100 status Discharging -> Some(100)
//!   right level 100 status Discharging -> Some(100)
//!   case  level 0 status Disconnected -> None
//! ```
//!
//! The arrow is [`Battery::available_level`]: the level as the UI should see it,
//! which is `None` for anything the device is not reporting on.
//!
//! Usage: cargo run --example battery_probe -- <MAC>

use std::time::Duration;

use linuxpods::aap::battery::Battery;
use linuxpods::aap::{Client, is_battery_packet, parse_battery_packet};

/// Long enough for the startup dump plus a few of the periodic reports.
const WINDOW: Duration = Duration::from_secs(25);

fn hex(packet: &[u8]) -> String {
    packet
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One component, raw fields first and the usable reading last.
fn describe(name: &str, battery: Option<Battery>) -> String {
    match battery {
        Some(b) => format!(
            "  {name:<5} level {} status {} -> {:?}",
            b.level,
            b.status,
            b.available_level()
        ),
        None => format!("  {name:<5} absent from this packet"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "linuxpods=debug".into()),
        )
        .init();

    let mac = std::env::args().nth(1).expect("usage: battery_probe <MAC>");

    let mut client = Client::new(&mac)?;
    println!("connecting AAP to {mac}...");
    client.connect().await?;
    client.handshake().await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    client.request_battery_status().await?;
    println!("connected; listening for {WINDOW:?}\n");

    let deadline = tokio::time::Instant::now() + WINDOW;
    let mut seen = 0;

    while let Ok(read) = tokio::time::timeout_at(deadline, client.read_packet()).await {
        match read {
            Ok(packet) if is_battery_packet(&packet) => {
                seen += 1;
                println!("RAW   {}", hex(&packet));
                match parse_battery_packet(&packet) {
                    Ok(info) => {
                        println!("{}", describe("left", info.left));
                        println!("{}", describe("right", info.right));
                        println!("{}\n", describe("case", info.case));
                    }
                    Err(e) => println!("  unparsable: {e}\n"),
                }
            }
            // Everything else on this socket - the startup settings dump, noise
            // control reports - belongs to `noise_probe`.
            Ok(_) => {}
            Err(e) => {
                println!("read failed: {e:#}");
                break;
            }
        }
    }

    if seen == 0 {
        println!("no battery packets arrived - are the AirPods connected?");
    }
    Ok(())
}
