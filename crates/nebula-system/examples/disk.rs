use nebula_system::{self as sys};

fn main() -> sys::Result<()> {
    sys::init()?;

    println!("=== Disks ===");

    #[cfg(feature = "disk")]
    {
        let disks = sys::disk::list();
        if disks.is_empty() {
            println!("No disks detected.");
        }
        for d in disks {
            println!(
                "{} [{}] total={} avail={} used={} ({:.1}%) removable={} type={:?}",
                d.mount_point,
                d.filesystem,
                sys::disk::format_bytes(d.total_space),
                sys::disk::format_bytes(d.available_space),
                sys::disk::format_bytes(d.used_space),
                d.usage_percent,
                d.is_removable,
                d.disk_type
            );
        }

        let total = sys::disk::total_usage();
        println!(
            "\nTotal: total={} used={} avail={} ({:.1}% used)",
            sys::disk::format_bytes(total.total_space),
            sys::disk::format_bytes(total.used_space),
            sys::disk::format_bytes(total.available_space),
            total.usage_percent
        );
    }

    #[cfg(not(feature = "disk"))]
    {
        println!(
            "This example requires feature 'disk'. Run with:\n  cargo run -p nebula-system --example disk --features \"disk\""
        );
    }

    Ok(())
}
