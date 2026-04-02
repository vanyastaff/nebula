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
        // SAFETY: restore() is not thread-safe; BumpScope holds &'a BumpAllocator
        // so the borrow checker prevents concurrent use of the allocator while this
        // scope is alive.  Errors here mean the allocator's generation was bumped
        // (via reset()) while allocations were in flight, leaving the allocator in
        // an unrecoverable state — surface this in debug builds.
        let result = self.allocator.restore(self.checkpoint);
        debug_assert!(
            result.is_ok(),
            "BumpScope: checkpoint restore failed on drop — allocator state is corrupt"
        );
    }
}
