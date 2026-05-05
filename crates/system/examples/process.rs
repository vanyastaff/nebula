use nebula_system::{self as sys};

fn main() -> nebula_system::SystemResult<()> {
    sys::init()?;

    println!("=== Processes ===");

    #[cfg(feature = "process")]
    {
        // Current process
        match sys::process::current() {
            Ok(p) => println!(
                "Current PID={} name={} cpu={:?} mem={} threads={:?} uid={:?} gid={:?}",
                p.pid,
                p.name,
                p.cpu_usage,
                sys::utils::format_bytes_usize(p.memory),
                p.thread_count,
                p.uid,
                p.gid
            ),
            Err(e) => println!("Failed to get current process: {e}"),
        }

        // List first 10 processes
        let mut list = sys::process::list();
        list.sort_by_key(|p| p.pid);
        for p in list.into_iter().take(10) {
            println!(
                "PID={} name={} status={:?} cpu={:?} mem={} threads={:?}",
                p.pid,
                p.name,
                p.status,
                p.cpu_usage,
                sys::utils::format_bytes_usize(p.memory),
                p.thread_count
            );
        }

        let stats = sys::process::stats();
        println!(
            "\nStats: total={} running={} sleeping={} total_mem={} total_cpu={:?}",
            stats.total,
            stats.running,
            stats.sleeping,
            sys::utils::format_bytes_usize(stats.total_memory),
            stats.total_cpu
        );
    }

    #[cfg(not(feature = "process"))]
    {
        println!(
            "This example requires feature 'process'. Run with:\n  cargo run -p nebula-system --example process --features \"process\""
        );
    }

    Ok(())
}
