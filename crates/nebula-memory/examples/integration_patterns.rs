//! Real-world integration patterns for nebula-memory
//!
//! This example demonstrates how to integrate nebula-memory allocators
//! into common application patterns like web servers, compilers, and data processors.

use nebula_memory::allocator::{BumpAllocator, PoolAllocator, TypedAllocator};
use nebula_memory::prelude::*;
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== nebula-memory Integration Patterns ===\n");

    // Pattern 1: Request-scoped allocations (Web Server)
    println!("1. Request-Scoped Allocations (Web Server):");
    request_handler_pattern()?;
    println!();

    // Pattern 2: Arena for AST construction (Compiler)
    println!("2. Arena for AST Construction (Compiler):");
    ast_builder_pattern()?;
    println!();

    // Pattern 3: Object pool for connections (Database)
    println!("3. Object Pool for Connections (Database):");
    connection_pool_pattern()?;
    println!();

    Ok(())
}

/// Pattern 1: Request-scoped memory for web servers
///
/// Each HTTP request gets its own arena allocator that's freed
/// when the request completes, eliminating per-request GC pressure.
fn request_handler_pattern() -> Result<(), Box<dyn std::error::Error>> {
    // Simulated HTTP request
    struct Request {
        path: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    struct Response {
        status: u16,
        body: Vec<u8>,
    }

    fn handle_request(req: Request) -> Result<Response, Box<dyn std::error::Error>> {
        // Create per-request arena (64KB is typical for request processing)
        let arena = BumpAllocator::new(64 * 1024)?;

        // Allocate temporary buffers for request processing
        let buffer = unsafe { arena.alloc_array::<u8>(4096)? };

        // Process request (simulated)
        let response_body = format!("Processed: {} ({} bytes)",
            req.path, req.body.len()).into_bytes();

        Ok(Response {
            status: 200,
            body: response_body,
        })
        // Arena automatically freed here - all temporary allocations cleaned up!
    }

    // Simulate handling requests
    let req = Request {
        path: "/api/users".to_string(),
        headers: HashMap::new(),
        body: vec![1, 2, 3, 4, 5],
    };

    let response = handle_request(req)?;
    println!("✓ Response: {} (status {})", String::from_utf8_lossy(&response.body), response.status);
    println!("✓ All temporary allocations freed automatically");

    Ok(())
}

/// Pattern 2: Arena-based AST for compilers
///
/// AST nodes reference each other within an arena, allowing the entire
/// tree to be freed at once without traversing for cleanup.
fn ast_builder_pattern() -> Result<(), Box<dyn std::error::Error>> {
    // Simulated AST nodes
    #[derive(Debug)]
    enum AstNode<'arena> {
        Literal(i32),
        BinOp {
            left: &'arena AstNode<'arena>,
            right: &'arena AstNode<'arena>,
            op: char,
        },
    }

    struct AstArena {
        allocator: BumpAllocator,
    }

    impl AstArena {
        fn new(capacity: usize) -> Result<Self, AllocError> {
            Ok(Self {
                allocator: BumpAllocator::new(capacity)?,
            })
        }

        fn alloc_node<'a>(&'a self, node: AstNode<'a>) -> Result<&'a AstNode<'a>, AllocError> {
            unsafe {
                let ptr = self.allocator.alloc_init(node)?;
                Ok(&*ptr.as_ptr())
            }
        }
    }

    // Build AST for: (2 + 3) * 4
    let arena = AstArena::new(1024 * 1024)?; // 1MB arena

    let two = arena.alloc_node(AstNode::Literal(2))?;
    let three = arena.alloc_node(AstNode::Literal(3))?;
    let sum = arena.alloc_node(AstNode::BinOp {
        left: two,
        right: three,
        op: '+',
    })?;
    let four = arena.alloc_node(AstNode::Literal(4))?;
    let result = arena.alloc_node(AstNode::BinOp {
        left: sum,
        right: four,
        op: '*',
    })?;

    println!("✓ Built AST: {:?}", result);
    println!("✓ Memory usage: {} / {} bytes",
        arena.allocator.used(), arena.allocator.capacity());
    println!("✓ Zero fragmentation with arena allocation");

    Ok(())
}

/// Pattern 3: Connection pool using object pool allocator
///
/// Pre-allocate a pool of connection objects and reuse them,
/// eliminating allocation overhead for each connection.
fn connection_pool_pattern() -> Result<(), Box<dyn std::error::Error>> {
    use core::sync::atomic::{AtomicU32, Ordering};

    // Simulated database connection
    #[repr(align(8))]
    struct DbConnection {
        id: u32,
        buffer: [u8; 4096],
        _marker: core::marker::PhantomData<()>,
    }

    impl DbConnection {
        fn new(id: u32) -> Self {
            Self {
                id,
                buffer: [0; 4096],
                _marker: core::marker::PhantomData,
            }
        }
    }

    struct ConnectionPool {
        allocator: PoolAllocator,
        next_id: AtomicU32,
    }

    impl ConnectionPool {
        fn new(pool_size: usize) -> Result<Self, AllocError> {
            Ok(Self {
                allocator: PoolAllocator::new(
                    core::mem::size_of::<DbConnection>(),
                    core::mem::align_of::<DbConnection>(),
                    pool_size,
                )?,
                next_id: AtomicU32::new(0),
            })
        }

        fn acquire(&self) -> Result<core::ptr::NonNull<DbConnection>, AllocError> {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);

            unsafe {
                let ptr = self.allocator.alloc_init(DbConnection::new(id))?;
                Ok(ptr)
            }
        }

        fn release(&self, ptr: core::ptr::NonNull<DbConnection>) {
            unsafe {
                self.allocator.dealloc(ptr);
            }
        }
    }

    let pool = ConnectionPool::new(10)?;

    // Acquire connections
    println!("Acquiring connections from pool...");
    let conn1 = pool.acquire()?;
    println!("✓ Acquired connection {}", unsafe { (*conn1.as_ptr()).id });

    let conn2 = pool.acquire()?;
    println!("✓ Acquired connection {}", unsafe { (*conn2.as_ptr()).id });

    // Return connection to pool
    pool.release(conn1);
    println!("✓ Released connection back to pool");

    // Reuse the slot
    let conn3 = pool.acquire()?;
    println!("✓ Reused connection slot (id: {})", unsafe { (*conn3.as_ptr()).id });

    // Cleanup
    pool.release(conn2);
    pool.release(conn3);
    println!("✓ All connections returned to pool");

    Ok(())
}
