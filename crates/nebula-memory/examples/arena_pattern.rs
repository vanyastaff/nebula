//! Arena allocation pattern example
//!
//! Demonstrates the arena pattern for bulk allocation and deallocation.

use nebula_memory::arena::{Arena, TypedArena};

fn main() {
    println!("=== Arena Allocation Pattern ===\n");

    // Example 1: Generic arena
    generic_arena_example();

    // Example 2: Typed arena for specific types
    typed_arena_example();
}

fn generic_arena_example() {
    println!("## Generic Arena Example");

    // Create an arena with 1MB capacity
    let arena = Arena::production(1024 * 1024);

    // Allocate various types
    let num = arena.alloc(42_i32);
    let string = arena.alloc_str("Hello, arena!");
    let vec = arena.alloc_slice_copy(&[1, 2, 3, 4, 5]);

    println!("  Allocated integer: {}", num);
    println!("  Allocated string: {}", string);
    println!("  Allocated slice: {:?}", vec);
    println!("  Total allocated: {} bytes\n", arena.allocated());

    // Everything is deallocated when arena is dropped
}

fn typed_arena_example() {
    println!("## Typed Arena Example");

    #[derive(Debug)]
    struct Node {
        value: i32,
        next: Option<&'static Node>,
    }

    // Create a typed arena for Node structures
    let arena = TypedArena::<Node>::production();

    // Build a linked list in the arena
    let node3 = arena.alloc(Node { value: 3, next: None });
    let node2 = arena.alloc(Node { value: 2, next: Some(node3) });
    let node1 = arena.alloc(Node { value: 1, next: Some(node2) });

    // Traverse the list
    print!("  Linked list: ");
    let mut current = Some(node1);
    while let Some(node) = current {
        print!("{} -> ", node.value);
        current = node.next;
    }
    println!("None");

    println!("  Allocated {} nodes\n", arena.len());
}
