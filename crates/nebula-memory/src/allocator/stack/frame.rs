//! RAII-based stack frame for automatic restoration

use super::{StackAllocator, StackMarker};

/// RAII helper for stack-based scoped allocation
///
/// This struct automatically restores the stack to a marked position
/// when it goes out of scope, providing exception-safe stack management.
pub struct StackFrame<'a> {
    allocator: &'a StackAllocator,
    marker: StackMarker,
}

impl<'a> StackFrame<'a> {
    /// Creates a new stack frame that will restore to the current position
    /// when dropped
    pub fn new(allocator: &'a StackAllocator) -> Self {
        let marker = allocator.mark();
        Self { allocator, marker }
    }

    /// Gets the underlying allocator
    pub fn allocator(&self) -> &'a StackAllocator {
        self.allocator
    }

    /// Manually restore and consume this frame
    pub fn restore(self) {
        // Drop will handle the restoration
        drop(self);
    }
}

impl<'a> Drop for StackFrame<'a> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.allocator.restore_to_marker(self.marker);
        }
    }
}
