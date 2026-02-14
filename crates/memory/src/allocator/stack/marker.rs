//! Stack marker for position tracking

/// Marker representing a position in the stack allocator
///
/// Can be used to reset the allocator to this position, deallocating
/// all allocations made after the marker was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackMarker {
    pub(super) position: usize,
}
