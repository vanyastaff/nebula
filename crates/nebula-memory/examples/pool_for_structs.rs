//! Object pool pattern for structs
//!
//! Shows how to use PoolAllocator for efficient object reuse.

use nebula_memory::allocator::PoolAllocator;
use std::alloc::Layout;

#[derive(Debug)]
struct Message {
    id: u64,
    data: [u8; 64],
    timestamp: u64,
}

impl Message {
    fn new(id: u64) -> Self {
        Self {
            id,
            data: [0; 64],
            timestamp: 0,
        }
    }
}

fn main() {
    println!("=== Object Pool Pattern for Structs ===\n");

    // Create a pool for Message structures
    // PoolAllocator::for_type::<Message>(capacity) would be ideal, but let's be explicit
    let layout = Layout::new::<Message>();
    let capacity = 1000;

    let pool = PoolAllocator::production(layout.size(), layout.align(), capacity)
        .expect("Failed to create pool");

    println!("Created pool for {} Message objects", capacity);
    println!("Object size: {} bytes", layout.size());
    println!("Initial available: {}\n", pool.available());

    // Simulate message processing workflow
    println!("## Message Processing Simulation");

    let mut allocated_messages = Vec::new();

    // Allocate 10 messages
    for i in 0..10 {
        unsafe {
            let ptr = pool.allocate(layout).expect("Pool exhausted");
            let msg_ptr = ptr.as_ptr() as *mut Message;
            msg_ptr.write(Message::new(i));
            allocated_messages.push(ptr);

            println!("  Allocated message {}", i);
        }
    }

    println!("  Available after allocation: {}", pool.available());

    // Process and return messages to pool
    println!("\n## Returning Messages to Pool");
    for (i, ptr) in allocated_messages.iter().enumerate() {
        unsafe {
            let msg_ptr = ptr.as_ptr() as *mut Message;
            let msg = msg_ptr.read();
            println!("  Processing message {}: id={}", i, msg.id);

            // Return to pool
            pool.deallocate(ptr.as_non_null_ptr(), layout);
        }
    }

    println!("  Available after return: {}", pool.available());

    // Pool objects are reused
    println!("\n## Reusing Pool Objects");
    for i in 100..105 {
        unsafe {
            let ptr = pool.allocate(layout).expect("Pool exhausted");
            let msg_ptr = ptr.as_ptr() as *mut Message;
            msg_ptr.write(Message::new(i));
            println!("  Reused slot for message {}", i);

            // Clean up
            msg_ptr.read();
            pool.deallocate(ptr.as_non_null_ptr(), layout);
        }
    }

    println!("\n✓ All messages processed and pool slots reused efficiently");
}
