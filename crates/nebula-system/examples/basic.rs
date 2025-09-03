use nebula_system::{self as sys, SystemInfo};

fn main() -> sys::Result<()> {
    // Initialize caches/backends
    sys::init()?;

    // Cached snapshot
    let info = SystemInfo::get();

    println!("=== Nebula System: Basic Info ===");
    println!("Summary:\n{}", sys::summary());

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
        "Memory: total={:.2} GB, available={:.2} GB, page_size={} B",
        info.memory.total as f64 / (1024.0 * 1024.0 * 1024.0),
        info.memory.available as f64 / (1024.0 * 1024.0 * 1024.0),
        info.memory.page_size
    );

    Ok(())
}
