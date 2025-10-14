//! Checkpoint and scoping support for bump allocator

use super::BumpAllocator;

/// Checkpoint for saving/restoring allocator state
#[derive(Debug, Clone, Copy)]
pub struct BumpCheckpoint {
    pub(super) position: usize,
    pub(super) generation: u32,
}

/// RAII guard for automatic checkpoint restoration
///
/// When dropped, automatically restores the allocator to the checkpoint state.
pub struct BumpScope<'a> {
    allocator: &'a BumpAllocator,
    checkpoint: BumpCheckpoint,
}

impl<'a> BumpScope<'a> {
    pub(super) fn new(allocator: &'a BumpAllocator) -> Self {
        Self {
            checkpoint: allocator.checkpoint(),
            allocator,
        }
    }
}

impl Drop for BumpScope<'_> {
    fn drop(&mut self) {
        // Ignore errors during drop - we can't propagate them
        // The restore() method validates the checkpoint internally
        let _ = self.allocator.restore(self.checkpoint);
    }
}
