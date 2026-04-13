//! Sandbox demo: drives the counter fixture plugin through `ProcessSandbox`
//! and prints each round-trip, proving the plugin process is long-lived
//! across multiple invocations.
//!
//! Build and run:
//!
//! ```bash
//! cargo build -p nebula-plugin-sdk --bin nebula-counter-fixture
//! cargo run -p nebula-sandbox --example sandbox_demo
//! ```
//!
//! Expected output: the counter accumulates across five `increment` calls
//! without ever going back to zero. If you see the counter reset between
//! calls, the long-lived `PluginHandle` is broken.

use std::{path::PathBuf, time::Duration};

use nebula_sandbox::{ProcessSandbox, capabilities::PluginCapabilities};
use serde_json::json;

fn locate_counter_binary() -> Option<PathBuf> {
    // Example binaries and [[bin]] targets live side-by-side in
    // `target/<profile>/` — the [[bin]] targets land in the profile root,
    // examples land in `target/<profile>/examples/`. We're running as an
    // example, so we're in the `examples/` subdirectory and need to go up
    // one level to find the fixture bin.
    let self_exe = std::env::current_exe().ok()?;
    let examples_dir = self_exe.parent()?;
    let profile_dir = examples_dir.parent()?;

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let candidate = profile_dir.join(format!("nebula-counter-fixture{ext}"));
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let binary = match locate_counter_binary() {
        Some(p) => p,
        None => {
            eprintln!("error: nebula-counter-fixture binary not found next to this example");
            eprintln!();
            eprintln!("build the fixture first:");
            eprintln!("  cargo build -p nebula-plugin-sdk --bin nebula-counter-fixture");
            eprintln!();
            eprintln!("then run the demo again:");
            eprintln!("  cargo run -p nebula-sandbox --example sandbox_demo");
            std::process::exit(1);
        }
    };

    println!("=== nebula-sandbox slice-1c demo ===");
    println!("plugin binary: {}", binary.display());
    println!();

    let sandbox = ProcessSandbox::new(
        binary,
        Duration::from_secs(5),
        PluginCapabilities::trusted(),
    );

    // 1. Metadata handshake + first round-trip. This spawns the plugin, reads the handshake line
    //    from its stdout, dials the announced UDS / Named Pipe, and caches the PluginHandle.
    println!("--- step 1: metadata ---");
    let meta_envelope = sandbox.get_metadata().await?;
    println!("got metadata envelope:");
    println!("{meta_envelope:#?}");
    println!();

    // 2. Five increment calls with different amounts. Each call reuses the cached PluginHandle — no
    //    respawn. Running total accumulates.
    println!("--- step 2: 5 sequential increment calls (same plugin process) ---");
    let amounts = [10_i64, 20, 30, 5, 100];
    let mut running = 0_i64;
    for (i, amount) in amounts.iter().enumerate() {
        let result = sandbox
            .invoke("increment", json!({ "amount": amount }))
            .await?;
        running += amount;
        let reported = result.get("total").and_then(|v| v.as_i64()).unwrap_or(-1);
        println!(
            "call {} — increment({:>3}) → total={:>4} (expected {:>4}) {}",
            i + 1,
            amount,
            reported,
            running,
            if reported == running {
                "OK"
            } else {
                "MISMATCH"
            }
        );
    }
    println!();

    // 3. Query current. Another round-trip over the same cached connection.
    println!("--- step 3: current ---");
    let current = sandbox.invoke("current", json!({})).await?;
    println!("current → {current}");
    println!();

    // 4. Reset, then query again.
    println!("--- step 4: reset + current ---");
    let reset = sandbox.invoke("reset", json!({})).await?;
    println!("reset → {reset}");
    let post_reset = sandbox.invoke("current", json!({})).await?;
    println!("current → {post_reset}");
    println!();

    // 5. Trigger an error envelope by calling an unknown action. The plugin returns
    //    ActionResultError which `invoke` converts to ActionError::fatal.
    println!("--- step 5: unknown action (expected error) ---");
    match sandbox.invoke("does_not_exist", json!({})).await {
        Ok(v) => println!("unexpected ok: {v}"),
        Err(e) => println!("error (expected): {e}"),
    }
    println!();

    println!("--- done: plugin process will be killed via kill_on_drop when sandbox drops ---");
    drop(sandbox);

    Ok(())
}
