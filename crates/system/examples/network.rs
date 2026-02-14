use nebula_system::{self as sys};
use std::time::Duration;

fn main() -> nebula_system::SystemResult<()> {
    sys::init()?;

    println!("=== Network ===");

    #[cfg(feature = "network")]
    {
        let ifaces = sys::network::interfaces();
        if ifaces.is_empty() {
            println!("No interfaces.");
        }
        for iface in &ifaces {
            println!(
                "{} up={} loopback={} mac={:?} rx={} tx={}",
                iface.name,
                iface.is_up,
                iface.is_loopback,
                iface.mac_address,
                iface.stats.rx_bytes,
                iface.stats.tx_bytes
            );
        }

        println!("\nSampling usage for 3 seconds...");
        for _ in 0..3 {
            let usage = sys::network::usage();
            for u in &usage {
                println!(
                    "{}: rx_rate={:.0} B/s, tx_rate={:.0} B/s",
                    u.interface, u.rx_rate, u.tx_rate
                );
            }
            std::thread::sleep(Duration::from_secs(1));
        }

        let total = sys::network::total_stats();
        println!("\nTotal rx={} tx={}", total.rx_bytes, total.tx_bytes);
        println!("Online: {}", sys::network::is_online());
    }

    #[cfg(not(feature = "network"))]
    {
        println!(
            "This example requires feature 'network'. Run with:\n  cargo run -p nebula-system --example network --features \"network\""
        );
    }

    Ok(())
}
