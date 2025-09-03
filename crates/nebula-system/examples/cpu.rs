use nebula_system::{self as sys};
use std::time::Duration;

fn main() -> sys::Result<()> {
    sys::init()?;

    println!("=== CPU Overview ===");
    let features = sys::cpu::features();
    let cache = sys::cpu::cache_info();
    let topo = sys::cpu::topology();

    println!(
        "Features: sse={}, sse2={}, avx={}, avx2={}, avx512={}",
        features.sse, features.sse2, features.avx, features.avx2, features.avx512
    );
    println!(
        "Cache: L1d={:?}, L1i={:?}, L2={:?}, L3={:?}, line={}B",
        cache.l1_data, cache.l1_instruction, cache.l2, cache.l3, cache.line_size
    );
    println!(
        "Topology: packages={}, cores_per_package={}, threads_per_core={}, numa_nodes={}",
        topo.packages,
        topo.cores_per_package,
        topo.threads_per_core,
        topo.numa_nodes.len()
    );

    println!("\nSampling CPU usage 5 times (500ms interval)...");
    for _ in 0..5 {
        let u = sys::cpu::usage();
        println!(
            "avg={:.1}%  peak={:.1}%  cores_under_pressure={}",
            u.average, u.peak, u.cores_under_pressure
        );
        std::thread::sleep(Duration::from_millis(500));
    }

    println!("\nPressure level: {:?}", sys::cpu::pressure());
    println!(
        "Suggested worker threads: {}",
        sys::cpu::optimal_thread_count()
    );

    Ok(())
}
