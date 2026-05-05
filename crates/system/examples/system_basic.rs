use nebula_system::{self as sys, SystemInfo};

fn main() -> nebula_system::SystemResult<()> {
    // Initialize caches/backends
    sys::init()?;

    // Cached snapshot
    let info = SystemInfo::get();

    println!("=== Nebula System: Basic Info ===");
    println!("Summary:\n{}", sys::summary());
    println!(
        "Snapshot: source={} freshness={:?} observed_at={:?}",
        info.metadata.source, info.metadata.freshness, info.metadata.observed_at
    );

    println!(
        "\nOS: {} {} (kernel {})",
        info.os.name, info.os.version, info.os.kernel_version
    );
    println!("Arch: {}", info.os.arch);

    println!(
        "CPU: {} (cores={}, threads={}, ~{} MHz)",
        info.cpu.brand, info.cpu.cores, info.cpu.threads, info.cpu.frequency_mhz
    );

    println!(
        "Memory: effective_total={:.2} GB, effective_available={:.2} GB, source={:?}, page_size={} B",
        info.memory.effective.total as f64 / (1024.0 * 1024.0 * 1024.0),
        info.memory.effective.available as f64 / (1024.0 * 1024.0 * 1024.0),
        info.memory.effective.source,
        info.memory.page_size,
    );

    Ok(())
}
