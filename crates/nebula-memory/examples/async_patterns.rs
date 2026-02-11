//! Modern Async Patterns with nebula-memory
//!
//! This example demonstrates best practices for async Rust programming using:
//! - CancellationToken for graceful shutdown
//! - Structured concurrency with JoinSet
//! - Comprehensive tracing
//! - Timeout handling
//! - Error handling with context

use std::time::Duration;

use nebula_memory::async_support::{AsyncArena, AsyncPool};
use nebula_memory::pool::Poolable;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

/// Example poolable object
#[derive(Debug, Clone)]
struct WorkItem {
    id: usize,
    data: Vec<u8>,
}

impl WorkItem {
    fn new(id: usize) -> Self {
        Self {
            id,
            data: vec![0; 1024],
        }
    }

    fn process(&mut self) {
        // Simulate work
        self.data[0] = self.id as u8;
    }
}

impl Poolable for WorkItem {
    fn reset(&mut self) {
        self.data.fill(0);
    }

    fn memory_usage(&self) -> usize {
        std::mem::size_of::<Self>() + self.data.capacity()
    }
}

/// Pattern 1: Graceful Shutdown with CancellationToken
async fn pattern_graceful_shutdown() -> anyhow::Result<()> {
    println!("\n=== Pattern 1: Graceful Shutdown ===\n");

    // Create shutdown token
    let shutdown = CancellationToken::new();

    // Create pool with shutdown support
    let pool = AsyncPool::new(10, || WorkItem::new(0), shutdown.clone());

    // Spawn worker tasks
    let mut tasks = Vec::new();
    for i in 0..5 {
        let pool = pool.clone_handle();
        let shutdown = shutdown.clone();

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Respect shutdown signal
                    _ = shutdown.cancelled() => {
                        println!("Worker {i} shutting down gracefully");
                        break;
                    }

                    // Do work
                    result = pool.acquire() => {
                        if let Ok(mut item) = result {
                            item.process();
                            sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        });

        tasks.push(task);
    }

    // Let workers run for a bit
    sleep(Duration::from_millis(500)).await;

    // Initiate graceful shutdown
    println!("Initiating shutdown...");
    pool.shutdown();

    // Wait for all workers to finish
    for task in tasks {
        task.await?;
    }

    println!("All workers shut down gracefully\n");

    Ok(())
}

/// Pattern 2: Structured Concurrency with JoinSet
async fn pattern_structured_concurrency() -> anyhow::Result<()> {
    println!("\n=== Pattern 2: Structured Concurrency ===\n");

    let shutdown = CancellationToken::new();
    let arena = AsyncArena::new(shutdown.clone());

    // Use JoinSet for structured concurrency
    let mut set = JoinSet::new();

    // Spawn multiple tasks
    for i in 0..10 {
        let arena = arena.clone_handle();
        set.spawn(async move {
            // Allocate in arena
            let handle = arena.alloc(i * 10).await?;

            // Process
            let result = handle.modify(|v| *v += 1).await;

            // Read result
            let value = handle.read(|v| *v).await;

            Ok::<_, anyhow::Error>(value)
        });
    }

    // Collect all results
    let mut results = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(value)) => {
                println!("Task completed with value: {}", value);
                results.push(value);
            }
            Ok(Err(e)) => eprintln!("Task failed: {}", e),
            Err(e) => eprintln!("Join error: {}", e),
        }
    }

    println!("\nProcessed {} tasks successfully", results.len());

    shutdown.cancel();

    Ok(())
}

/// Pattern 3: Timeout Handling
async fn pattern_timeout_handling() -> anyhow::Result<()> {
    println!("\n=== Pattern 3: Timeout Handling ===\n");

    let shutdown = CancellationToken::new();

    // Create pool with custom timeout
    let pool = AsyncPool::new(5, || WorkItem::new(0), shutdown.clone())
        .with_timeout(Duration::from_secs(5));

    // Create arena with timeout
    let arena = AsyncArena::new(shutdown.clone())
        .with_timeout(Duration::from_secs(5));

    // Demonstrate timeout operations
    let handle = arena.alloc(42).await?;

    // Try to read with custom timeout
    match handle.try_read(|v| *v, Duration::from_millis(100)).await {
        Ok(value) => println!("Read succeeded: {}", value),
        Err(e) => eprintln!("Read timed out: {}", e),
    }

    // Pool acquire with timeout
    match pool.acquire().await {
        Ok(item) => println!("Acquired item with timeout"),
        Err(e) => eprintln!("Acquire failed: {}", e),
    }

    shutdown.cancel();
    pool.drain(Duration::from_millis(50)).await;

    Ok(())
}

/// Pattern 4: Error Handling with Context
async fn pattern_error_handling() -> anyhow::Result<()> {
    println!("\n=== Pattern 4: Error Handling ===\n");

    let shutdown = CancellationToken::new();
    let pool = AsyncPool::new(2, || WorkItem::new(0), shutdown.clone());

    // Demonstrate proper error handling
    let result = async {
        // Try to acquire
        let item1 = pool.acquire().await?;
        let item2 = pool.acquire().await?;

        // This should fail (pool exhausted)
        let item3 = pool.try_acquire().await
            .ok_or_else(|| anyhow::anyhow!("Pool exhausted as expected"))?;

        Ok::<_, anyhow::Error>(())
    }
    .await;

    match result {
        Ok(_) => println!("All acquisitions succeeded"),
        Err(e) => println!("Expected error: {}", e),
    }

    shutdown.cancel();
    pool.drain(Duration::from_millis(50)).await;

    Ok(())
}

/// Pattern 5: Concurrent Work with Backpressure
async fn pattern_concurrent_with_backpressure() -> anyhow::Result<()> {
    println!("\n=== Pattern 5: Concurrent Work with Backpressure ===\n");

    let shutdown = CancellationToken::new();
    let pool = AsyncPool::new(5, || WorkItem::new(0), shutdown.clone());

    let mut set = JoinSet::new();

    // Spawn many tasks - pool's semaphore provides backpressure
    for i in 0..20 {
        let pool = pool.clone_handle();
        set.spawn(async move {
            // Acquire will wait if pool is busy (backpressure)
            let mut item = pool.acquire().await?;

            println!("Processing item {}", i);
            item.process();

            // Simulate variable work duration
            sleep(Duration::from_millis(50 + (i % 3) * 50)).await;

            Ok::<_, anyhow::Error>(item.id)
        });
    }

    let mut completed = 0;
    while let Some(res) = set.join_next().await {
        if res.is_ok() {
            completed += 1;
        }
    }

    println!("\nCompleted {} tasks with automatic backpressure", completed);

    shutdown.cancel();
    pool.drain(Duration::from_millis(50)).await;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Modern Async Patterns with nebula-memory ===");

    // Run all patterns
    pattern_graceful_shutdown().await?;
    pattern_structured_concurrency().await?;
    pattern_timeout_handling().await?;
    pattern_error_handling().await?;
    pattern_concurrent_with_backpressure().await?;

    println!("\n=== All patterns demonstrated successfully ===\n");

    Ok(())
}
