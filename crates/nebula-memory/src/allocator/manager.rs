//! Complete allocator manager implementation
//!
//! Provides a central registry for managing multiple allocators
//! and facilitating allocation strategies with runtime switching.

use core::alloc::Layout;
use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;

use super::{AllocError, AllocResult, Allocator, ThreadSafeAllocator};

/// Unique identifier for registered allocators
///
/// Uses `NonZeroUsize` for memory efficiency (allows Option<AllocatorId> to be same size)
/// and provides type safety preventing accidental mixing with raw usizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AllocatorId(NonZeroUsize);

impl AllocatorId {
    /// Generate a new unique allocator ID
    ///
    /// IDs are generated atomically and are guaranteed to be unique within the process.
    #[must_use]
    pub fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(1);
        // Start from 1 to ensure NonZeroUsize is always valid
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        // SAFETY: COUNTER starts at 1 and only increments, so id is always non-zero
        // In the extremely unlikely event of overflow, we wrap (but this would take
        // billions of allocator registrations)
        Self(NonZeroUsize::new(id).unwrap_or_else(|| {
            // Overflow protection: restart from 1
            COUNTER.store(1, Ordering::Relaxed);
            // SAFETY: 1 is always non-zero by definition. This is only reached on counter
            // overflow (after ~2^64 allocations), and restarting from 1 is safe.
            unsafe { NonZeroUsize::new_unchecked(1) }
        }))
    }

    /// Get the raw ID value (for internal use only)
    #[inline]
    pub(crate) fn as_usize(self) -> usize {
        self.0.get()
    }
}

impl Default for AllocatorId {
    fn default() -> Self {
        Self::new()
    }
}

/// Type-erased allocator for storage in manager
pub trait ManagedAllocator: Send + Sync {
    /// Allocate memory
    ///
    /// # Safety
    ///
    /// Caller must ensure `layout` has non-zero size and valid alignment.
    /// The returned pointer must be deallocated with the same layout.
    unsafe fn managed_allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>>;

    /// Deallocate memory
    ///
    /// # Safety
    ///
    /// - `ptr` must have been allocated by this allocator with `layout`
    /// - `ptr` must not be used after deallocation
    /// - Must not be called more than once for the same pointer
    unsafe fn managed_deallocate(&self, ptr: NonNull<u8>, layout: Layout);

    /// Reallocate memory
    ///
    /// # Safety
    ///
    /// - `ptr` must have been allocated with `old_layout`
    /// - `old_layout` and `new_layout` must have the same alignment
    /// - `ptr` becomes invalid after this call (use returned pointer instead)
    unsafe fn managed_reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>>;

    /// Get allocator name for debugging
    fn name(&self) -> &'static str;
}

/// Blanket implementation for any thread-safe allocator
impl<A: ThreadSafeAllocator + 'static> ManagedAllocator for A {
    unsafe fn managed_allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Caller's safety requirements are forwarded to the underlying allocator.
        // This is a simple delegation - all preconditions documented in trait apply.
        unsafe { self.allocate(layout) }
    }

    unsafe fn managed_deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: Caller guarantees ptr was allocated with layout. We forward these
        // guarantees to the underlying allocator's deallocate method.
        unsafe { self.deallocate(ptr, layout) }
    }

    unsafe fn managed_reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Caller's invariants (ptr validity, matching old_layout, aligned layouts)
        // are forwarded to the underlying allocator without modification.
        unsafe { self.reallocate(ptr, old_layout, new_layout) }
    }

    fn name(&self) -> &'static str {
        core::any::type_name::<A>()
    }
}

/// Manager for multiple allocators with registry
pub struct AllocatorManager {
    /// Registry of allocators (lock-free concurrent map)
    allocators: DashMap<AllocatorId, Box<dyn ManagedAllocator>>,

    /// Currently active allocator ID (stored as usize for atomic operations)
    active_allocator: AtomicUsize,

    /// Default fallback allocator ID
    default_allocator: Option<AllocatorId>,
}

impl Default for AllocatorManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AllocatorManager {
    /// Creates a new `AllocatorManager`
    #[must_use]
    pub fn new() -> Self {
        Self {
            allocators: Default::default(),
            active_allocator: AtomicUsize::new(0),
            default_allocator: None,
        }
    }

    /// Register an allocator and return its ID
    pub fn register<A: ManagedAllocator + 'static>(&self, allocator: A) -> AllocatorId {
        let id = AllocatorId::new();
        self.allocators.insert(id, Box::new(allocator));
        id
    }

    /// Set the default allocator (must be already registered)
    #[must_use = "operation result must be checked"]
    pub fn set_default(&mut self, allocator_id: AllocatorId) -> Result<(), &'static str> {
        let exists = self.allocators.contains_key(&allocator_id);

        if exists {
            self.default_allocator = Some(allocator_id);
            self.active_allocator
                .store(allocator_id.as_usize(), Ordering::SeqCst);
            Ok(())
        } else {
            Err("Allocator ID not found")
        }
    }

    /// Sets the active allocator
    #[must_use = "operation result must be checked"]
    pub fn set_active_allocator(&self, allocator_id: AllocatorId) -> Result<(), &'static str> {
        // Verify allocator exists
        let exists = self.allocators.contains_key(&allocator_id);

        if exists {
            self.active_allocator
                .store(allocator_id.as_usize(), Ordering::SeqCst);
            Ok(())
        } else {
            Err("Allocator ID not found")
        }
    }

    /// Gets the current active allocator ID
    pub fn get_active_allocator_id(&self) -> Option<AllocatorId> {
        let id_raw = self.active_allocator.load(Ordering::SeqCst);
        NonZeroUsize::new(id_raw).map(AllocatorId)
    }

    /// Get the name of the active allocator
    pub fn get_active_allocator_name(&self) -> &'static str {
        self.get_active_allocator_id()
            .and_then(|id| self.with_allocator_by_id(id, |alloc| alloc.name()))
            .unwrap_or("unknown")
    }

    /// Execute a function with access to specific allocator
    pub fn with_allocator_by_id<F, R>(&self, allocator_id: AllocatorId, f: F) -> Option<R>
    where
        F: FnOnce(&dyn ManagedAllocator) -> R,
    {
        self.allocators
            .get(&allocator_id)
            .map(|alloc| f(alloc.as_ref()))
    }

    /// Execute a function with the active allocator
    pub fn with_active_allocator<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&dyn ManagedAllocator) -> R,
    {
        let id = self.get_active_allocator_id()?;
        self.with_allocator_by_id(id, f)
    }

    /// Resets to the default allocator
    pub fn reset_to_default(&self) {
        if let Some(default_id) = self.default_allocator {
            self.active_allocator
                .store(default_id.as_usize(), Ordering::SeqCst);
        }
    }

    /// Executes a function with a specific allocator temporarily active
    pub fn with_allocator<F, R>(&self, allocator_id: AllocatorId, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let previous = self
            .active_allocator
            .swap(allocator_id.as_usize(), Ordering::SeqCst);
        let result = f();
        self.active_allocator.store(previous, Ordering::SeqCst);
        result
    }

    /// List all registered allocators
    pub fn list_allocators(&self) -> Vec<(AllocatorId, &'static str)> {
        self.allocators
            .iter()
            .map(|entry| (*entry.key(), entry.value().name()))
            .collect()
    }

    /// Allocate using the currently active allocator
    ///
    /// # Safety
    ///
    /// Same requirements as the underlying allocator's `allocate` method:
    /// - `layout` must have non-zero size and valid alignment
    /// - Returned pointer must be deallocated with the same layout
    /// - Caller must ensure an allocator is active before calling
    pub unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: We delegate to the active allocator. Caller's safety requirements
        // are forwarded unchanged. Returns error if no allocator is active.
        self.with_active_allocator(|alloc| unsafe { alloc.managed_allocate(layout) })
            .unwrap_or_else(|| Err(AllocError::invalid_layout("no active allocator")))
    }

    /// Deallocate using the currently active allocator
    ///
    /// # Safety
    ///
    /// - `ptr` must have been allocated by the current active allocator
    /// - `layout` must be the same as used for allocation
    /// - `ptr` must not be used after this call
    /// - Must not be called more than once for the same pointer
    pub unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: Caller guarantees ptr was allocated with layout. We forward to the
        // active allocator. If no allocator is active, this is a silent no-op (defensive).
        self.with_active_allocator(|alloc| unsafe { alloc.managed_deallocate(ptr, layout) });
    }

    /// Reallocate using the currently active allocator
    ///
    /// # Safety
    ///
    /// - `ptr` must have been allocated with `old_layout` by the active allocator
    /// - `old_layout` and `new_layout` must have the same alignment
    /// - `ptr` becomes invalid after this call (use returned pointer instead)
    pub unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Caller's invariants are forwarded to the active allocator.
        // Returns error if no allocator is active.
        self.with_active_allocator(|alloc| unsafe {
            alloc.managed_reallocate(ptr, old_layout, new_layout)
        })
        .unwrap_or_else(|| Err(AllocError::invalid_layout("no active allocator")))
    }
}

/// Global allocator manager singleton
use core::sync::atomic::AtomicBool;

static MANAGER_INIT: AtomicBool = AtomicBool::new(false);

static GLOBAL_MANAGER: std::sync::OnceLock<AllocatorManager> = std::sync::OnceLock::new();

/// Singleton implementation of allocator manager
pub struct GlobalAllocatorManager;

impl GlobalAllocatorManager {
    /// Initializes the global allocator manager
    #[must_use = "initialization result must be checked"]
    pub fn init() -> Result<(), &'static str> {
        GLOBAL_MANAGER
            .set(AllocatorManager::new())
            .map_err(|_| "Global allocator manager already initialized")?;
        MANAGER_INIT.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Gets a reference to the global manager
    pub fn get() -> &'static AllocatorManager {
        GLOBAL_MANAGER.get().expect(
            "Global allocator manager not initialized. Call GlobalAllocatorManager::init() first.",
        )
    }

    /// Try to get the global manager without panicking
    pub fn try_get() -> Option<&'static AllocatorManager> {
        GLOBAL_MANAGER.get()
    }
}

/// Convenience macros for global allocator management
#[macro_export]
macro_rules! with_allocator {
    ($allocator_id:expr, $block:block) => {
        $crate::allocator::manager::GlobalAllocatorManager::get()
            .with_allocator($allocator_id, || $block)
    };
}

#[macro_export]
macro_rules! set_active_allocator {
    ($allocator_id:expr) => {
        $crate::allocator::manager::GlobalAllocatorManager::get()
            .set_active_allocator($allocator_id)
            .expect("Failed to set active allocator")
    };
}

/// Implement Allocator for the manager itself
///
/// # Safety
///
/// This impl is safe because `AllocatorManager` correctly implements the Allocator contract:
/// - All allocations are delegated to registered allocators that uphold memory safety
/// - Active allocator switching is atomic and properly synchronized
/// - No data races can occur in allocation/deallocation paths
/// - Pointers returned are valid and properly aligned (guaranteed by underlying allocators)
unsafe impl Allocator for AllocatorManager {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Simple delegation to our own allocate method, which forwards
        // to the active allocator. Safety requirements are inherited from trait.
        unsafe { self.allocate(layout) }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: Delegation to our deallocate method. Caller must guarantee ptr
        // was allocated with layout, and we forward that guarantee.
        unsafe { self.deallocate(ptr, layout) }
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Delegation to our reallocate method. All caller invariants
        // (ptr validity, layout matching) are preserved.
        unsafe { self.reallocate(ptr, old_layout, new_layout) }
    }
}

/// # Safety
///
/// `AllocatorManager` is thread-safe because:
/// - Registry uses lock-free `DashMap` for concurrent access
/// - Active allocator ID is stored in `AtomicUsize` with proper memory ordering
/// - All registered allocators must be Send + Sync by trait bound
/// - Allocator switching is atomic and properly synchronized (`SeqCst` ordering)
unsafe impl ThreadSafeAllocator for AllocatorManager {}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use super::*;
    use crate::allocator::system::SystemAllocator;

    static INIT: Once = Once::new();

    fn ensure_global_manager_initialized() {
        INIT.call_once(|| {
            GlobalAllocatorManager::init().expect("Failed to initialize global manager");
        });
    }

    #[test]
    fn test_manager_basic_functionality() {
        let manager = AllocatorManager::new();

        let system_alloc = SystemAllocator::new();
        let id = manager.register(system_alloc);

        let mut manager = manager; // Need mut for set_default
        manager.set_default(id).unwrap();

        let layout = Layout::new::<u64>();
        unsafe {
            let ptr = manager.allocate(layout).unwrap();
            manager.deallocate(ptr.cast(), layout);
        }
    }

    #[test]
    fn test_allocator_switching() {
        ensure_global_manager_initialized();
        let manager = GlobalAllocatorManager::get();

        let system1 = SystemAllocator::new();
        let system2 = SystemAllocator::new();

        let id1 = manager.register(system1);
        let id2 = manager.register(system2);

        manager.set_active_allocator(id1).unwrap();
        assert_eq!(manager.get_active_allocator_id(), Some(id1));

        manager.with_allocator(id2, || {
            assert_eq!(manager.get_active_allocator_id(), Some(id2));
        });

        assert_eq!(manager.get_active_allocator_id(), Some(id1));
    }

    #[test]
    fn test_macros() {
        ensure_global_manager_initialized();
        let manager = GlobalAllocatorManager::get();

        let system_alloc = SystemAllocator::new();
        let id = manager.register(system_alloc);

        set_active_allocator!(id);

        with_allocator!(id, {
            // Code using specific allocator
            let layout = Layout::new::<u32>();
            unsafe {
                if let Ok(ptr) = manager.allocate(layout) {
                    manager.deallocate(ptr.cast(), layout);
                }
            }
        });
    }
}
