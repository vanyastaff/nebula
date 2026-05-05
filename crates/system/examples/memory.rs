use nebula_system::{self as sys};

fn main() -> nebula_system::SystemResult<()> {
    sys::init()?;

    let mem = sys::memory::current();

    // Helper to format bytes (usize) from memory module
    fn fmt(b: usize) -> String {
        sys::memory::format_bytes(b)
    }

    println!("=== Memory Info ===");
    println!("total      : {}", fmt(mem.total));
    println!("available  : {}", fmt(mem.available));
    println!("used       : {}", fmt(mem.used));
    println!("usage      : {:?}", mem.usage_percent);
    println!("pressure   : {:?}", mem.pressure);
    println!("source     : {:?}", mem.capacity_source);
    println!("evidence   : {:?}", mem.pressure_report.reasons);

    println!("\nTip: High/Critical pressure may indicate you should reduce in-memory caches.");

    Ok(())
}
