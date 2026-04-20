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

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

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
    let binary = if let Some(p) = locate_counter_binary() {
        p
    } else {
        eprintln!("error: nebula-counter-fixture binary not found next to this example");
        eprintln!();
        eprintln!("build the fixture first:");
        eprintln!("  cargo build -p nebula-plugin-sdk --bin nebula-counter-fixture");
        eprintln!();
        eprintln!("then run the demo again:");
        eprintln!("  cargo run -p nebula-sandbox --example sandbox_demo");
        std::process::exit(1);
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
        let reported = result
            .get("total")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(-1);
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

    // 6. Panic probe. Plugin panics inside `execute`; its process dies; host detects the broken
    //    connection on the retry path. Subsequent calls should succeed against a fresh plugin
    //    process — counter will reset because we lost state.
    println!("--- step 6: plugin panic + auto-respawn ---");
    let before = sandbox.invoke("current", json!({})).await?;
    println!("before panic: current = {before}");
    let t0 = Instant::now();
    match sandbox.invoke("panic", json!({})).await {
        Ok(v) => println!("unexpected ok: {v}"),
        Err(e) => println!("panic call returned: {e} (took {:?})", t0.elapsed()),
    }
    // Next call should spawn a fresh plugin and start from zero.
    let after_panic = sandbox.invoke("increment", json!({ "amount": 7 })).await?;
    println!("after panic (fresh plugin): increment(7) → {after_panic}");
    println!();

    // 7. Timeout probe. Use a separate short-timeout sandbox so we don't affect the main one.
    //    Plugin sleeps longer than the timeout.
    println!("--- step 7: slow action hits per-call timeout ---");
    let fast_binary = locate_counter_binary().unwrap();
    let fast_sandbox = ProcessSandbox::new(
        fast_binary,
        Duration::from_millis(300),
        PluginCapabilities::trusted(),
    );
    let t0 = Instant::now();
    match fast_sandbox.invoke("slow", json!({ "millis": 2000 })).await {
        Ok(v) => println!("unexpected ok: {v}"),
        Err(e) => println!("slow call returned: {e} (took {:?})", t0.elapsed()),
    }
    drop(fast_sandbox);
    println!();

    // 8. Big payload probe. Host `recv_envelope` currently reads byte-at-a-time, so this is where
    //    we'll see if that matters for KB-range responses.
    println!("--- step 8: large payload round-trip ---");
    for kb in [10_u64, 100, 500, 1000] {
        let t0 = Instant::now();
        match sandbox.invoke("big", json!({ "kb": kb })).await {
            Ok(v) => {
                let size = v
                    .get("size_bytes")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let elapsed = t0.elapsed();
                let throughput_kbps = (size as f64) / 1024.0 / elapsed.as_secs_f64();
                println!(
                    "big(kb={kb:>4}) → {size:>7} bytes in {elapsed:>10.3?} ({throughput_kbps:>8.1} KB/s)"
                );
            },
            Err(e) => println!("big(kb={kb}) error: {e}"),
        }
    }
    println!();

    // 9. Latency probe. 100 rapid increments on the cached handle. Measures per-call wall time for
    //    small envelopes round-tripping over the long-lived socket. No respawn should occur during
    //    the loop.
    println!("--- step 9: 100 rapid increments (hot-path latency) ---");
    sandbox.invoke("reset", json!({})).await?;
    let mut latencies: Vec<Duration> = Vec::with_capacity(100);
    let loop_start = Instant::now();
    for _ in 0..100 {
        let t0 = Instant::now();
        sandbox.invoke("increment", json!({ "amount": 1 })).await?;
        latencies.push(t0.elapsed());
    }
    let total = loop_start.elapsed();
    let final_total = sandbox.invoke("current", json!({})).await?;
    latencies.sort();
    let min = latencies.first().copied().unwrap_or_default();
    let max = latencies.last().copied().unwrap_or_default();
    let p50 = latencies[50];
    let p95 = latencies[95];
    let p99 = latencies[99];
    let mean = total / 100;
    println!(
        "100 increments in {total:?} — min={min:?} p50={p50:?} p95={p95:?} p99={p99:?} max={max:?} mean={mean:?}"
    );
    println!("final total: {final_total}");
    println!();

    println!("--- done: plugin process will be killed via kill_on_drop when sandbox drops ---");
    drop(sandbox);

    Ok(())
}
