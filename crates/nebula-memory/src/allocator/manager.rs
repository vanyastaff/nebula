//! Complete allocator manager implementation
//!
//! Provides a central registry for managing multiple allocators
//! and facilitating allocation strategies with runtime switching.

use core::alloc::Layout;
use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::{AllocError, AllocResult, Allocator, ThreadSafeAllocator};

/// Unique identifier for registered allocators
///
/// Uses NonZeroUsize for memory efficiency (allows Option<AllocatorId> to be same size)
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
            NonZeroUsize::new(1).unwrap()
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
    unsafe fn managed_allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>>;

    /// Deallocate memory
    unsafe fn managed_deallocate(&self, ptr: NonNull<u8>, layout: Layout);

    /// Reallocate memory
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
        unsafe { self.allocate(layout) }
    }

    unsafe fn managed_deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { self.deallocate(ptr, layout) }
    }

    unsafe fn managed_reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { self.reallocate(ptr, old_layout, new_layout) }
    }

    fn name(&self) -> &'static str {
        core::any::type_name::<A>()
    }
}

/// Manager for multiple allocators with registry
pub struct AllocatorManager {
    /// Registry of allocators
    #[cfg(feature = "std")]
    allocators:
        std::sync::RwLock<std::collections::HashMap<AllocatorId, Box<dyn ManagedAllocator>>>,

    #[cfg(not(feature = "std"))]
    allocators: spin::RwLock<heapless::FnvIndexMap<AllocatorId, &'static dyn ManagedAllocator, 16>>,

    /// Currently active allocator ID (stored as usize for atomic operations)
    active_allocator: AtomicUsize,

    /// Default fallback allocator ID
    default_allocator: Option<AllocatorId>,
}

impl AllocatorManager {
    /// Creates a new AllocatorManager
    pub fn new() -> Self {
        Self {
            allocators: Default::default(),
            active_allocator: AtomicUsize::new(0),
            default_allocator: None,
        }
    }

    /// Register an allocator and return its ID
    #[cfg(feature = "std")]
    pub fn register<A: ManagedAllocator + 'static>(&self, allocator: A) -> AllocatorId {
        let id = AllocatorId::new();
        let mut registry = self.allocators.write().unwrap();
        registry.insert(id, Box::new(allocator));
        id
    }

    /// Register a static allocator (no_std)
    #[cfg(not(feature = "std"))]
    pub fn register_static(
        &self,
        allocator: &'static dyn ManagedAllocator,
    ) -> Result<AllocatorId, &'static str> {
        let id = AllocatorId::new();
        let mut registry = self.allocators.write();
        registry
            .insert(id, allocator)
            .map_err(|_| "Registry full")?;
        Ok(id)
    }

    /// Set the default allocator (must be already registered)
    pub fn set_default(&mut self, allocator_id: AllocatorId) -> Result<(), &'static str> {
        #[cfg(feature = "std")]
        let exists = self.allocators.read().unwrap().contains_key(&allocator_id);

        #[cfg(not(feature = "std"))]
        let exists = self.allocators.read().contains_key(&allocator_id);

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
    pub fn set_active_allocator(&self, allocator_id: AllocatorId) -> Result<(), &'static str> {
        // Verify allocator exists
        #[cfg(feature = "std")]
        let exists = self.allocators.read().unwrap().contains_key(&allocator_id);

        #[cfg(not(feature = "std"))]
        let exists = self.allocators.read().contains_key(&allocator_id);

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
        #[cfg(feature = "std")]
        {
            let registry = self.allocators.read().unwrap();
            registry.get(&allocator_id).map(|alloc| f(alloc.as_ref()))
        }

        #[cfg(not(feature = "std"))]
        {
            let registry = self.allocators.read();
            registry.get(&allocator_id).map(|&alloc| f(alloc))
        }
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
        #[cfg(feature = "std")]
        {
            let registry = self.allocators.read().unwrap();
            registry
                .iter()
                .map(|(&id, alloc)| (id, alloc.name()))
                .collect()
        }

        #[cfg(not(feature = "std"))]
        {
            let registry = self.allocators.read();
            registry
                .iter()
                .map(|(&id, &alloc)| (id, alloc.name()))
                .collect()
        }
    }

    /// Allocate using the currently active allocator
    pub unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        self.with_active_allocator(|alloc| unsafe { alloc.managed_allocate(layout) })
            .unwrap_or_else(|| Err(AllocError::invalid_layout("no active allocator")))
    }

    /// Deallocate using the currently active allocator
    pub unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.with_active_allocator(|alloc| unsafe { alloc.managed_deallocate(ptr, layout) });
    }

    /// Reallocate using the currently active allocator
    pub unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        self.with_active_allocator(|alloc| unsafe {
            alloc.managed_reallocate(ptr, old_layout, new_layout)
        })
        .unwrap_or_else(|| Err(AllocError::invalid_layout("no active allocator")))
    }
}

/// Global allocator manager singleton
use core::sync::atomic::AtomicBool;

static MANAGER_INIT: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "std")]
static GLOBAL_MANAGER: std::sync::OnceLock<AllocatorManager> = std::sync::OnceLock::new();

#[cfg(not(feature = "std"))]
static GLOBAL_MANAGER: spin::Once<AllocatorManager> = spin::Once::new();

/// Singleton implementation of allocator manager
pub struct GlobalAllocatorManager;

impl GlobalAllocatorManager {
    /// Initializes the global allocator manager
    pub fn init() -> Result<(), &'static str> {
        #[cfg(feature = "std")]
        {
            GLOBAL_MANAGER
                .set(AllocatorManager::new())
                .map_err(|_| "Global allocator manager already initialized")?;
            MANAGER_INIT.store(true, Ordering::SeqCst);
            Ok(())
        }

        #[cfg(not(feature = "std"))]
        {
            if MANAGER_INIT.swap(true, Ordering::SeqCst) {
                return Err("Global allocator manager already initialized");
            }
            GLOBAL_MANAGER.call_once(|| AllocatorManager::new());
            Ok(())
        }
    }

    /// Gets a reference to the global manager
    pub fn get() -> &'static AllocatorManager {
        #[cfg(feature = "std")]
        {
            GLOBAL_MANAGER.get()
                .expect("Global allocator manager not initialized. Call GlobalAllocatorManager::init() first.")
        }

        #[cfg(not(feature = "std"))]
        {
            if !MANAGER_INIT.load(Ordering::SeqCst) {
                panic!(
                    "Global allocator manager not initialized. Call GlobalAllocatorManager::init() first."
                );
            }
            GLOBAL_MANAGER
                .get()
                .expect("Global allocator manager not initialized")
        }
    }

    /// Try to get the global manager without panicking
    pub fn try_get() -> Option<&'static AllocatorManager> {
        #[cfg(feature = "std")]
        {
            GLOBAL_MANAGER.get()
        }

        #[cfg(not(feature = "std"))]
        {
            if MANAGER_INIT.load(Ordering::SeqCst) {
                GLOBAL_MANAGER.get()
            } else {
                None
            }
        }
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
unsafe impl Allocator for AllocatorManager {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        unsafe { self.allocate(layout) }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { self.deallocate(ptr, layout) }
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { self.reallocate(ptr, old_layout, new_layout) }
    }
}

unsafe impl ThreadSafeAllocator for AllocatorManager {}

#[cfg(test)]
mod tests {
    #[cfg(feature = "std")]
    use std::sync::Once;

    use super::*;
    use crate::allocator::system::SystemAllocator;

    #[cfg(feature = "std")]
    static INIT: Once = Once::new();

    #[cfg(feature = "std")]
    fn ensure_global_manager_initialized() {
        INIT.call_once(|| {
            GlobalAllocatorManager::init().expect("Failed to initialize global manager");
        });
    }

    #[test]
    fn test_manager_basic_functionality() {
        let manager = AllocatorManager::new();

        #[cfg(feature = "std")]
        {
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
    }

    #[test]
    fn test_allocator_switching() {
        #[cfg(feature = "std")]
        {
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
    }

    #[test]
    fn test_macros() {
        #[cfg(feature = "std")]
        {
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
}
