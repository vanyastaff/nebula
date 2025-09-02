use nebula_system::{self as sys};

fn main() -> sys::Result<()> {
    sys::init()?;

    println!("=== Processes ===");

    #[cfg(feature = "process")]
    {
        // Current process
        match sys::process::current() {
            Ok(p) => println!("Current PID={} name={} cpu={:.1}% mem={} KiB", p.pid, p.name, p.cpu_usage, p.memory),
            Err(e) => println!("Failed to get current process: {}", e),
        }

        // List first 10 processes
        let mut list = sys::process::list();
        list.sort_by_key(|p| p.pid);
        for p in list.into_iter().take(10) {
            println!("PID={} name={} status={:?} cpu={:.1}% mem={} KiB", p.pid, p.name, p.status, p.cpu_usage, p.memory);
        }

        let stats = sys::process::stats();
        println!(
            "\nStats: total={} running={} sleeping={} total_mem={} KiB total_cpu={:.1}%",
            stats.total, stats.running, stats.sleeping, stats.total_memory, stats.total_cpu
        );
    }

    #[cfg(not(feature = "process"))]
    {
        println!("This example requires feature 'process'. Run with:\n  cargo run -p nebula-system --example process --features \"process\"");
    }

    Ok(())
}
