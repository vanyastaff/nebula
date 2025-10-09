//! Thread-local arena implementation for maximum performance

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use super::{Arena, ArenaConfig};
use crate::core::error::MemoryError;

thread_local! {
    static LOCAL_ARENA: RefCell<LocalArena> = RefCell::new(LocalArena::new());
}

/// Thread-local arena for fast allocations without synchronization
pub struct LocalArena {
    arena: Arena,
    generation: Cell<u64>,
}

impl LocalArena {
    /// Creates new local arena with default config
    pub fn new() -> Self {
        Self::with_config(ArenaConfig::default())
    }

    /// Creates new local arena with custom config
    pub fn with_config(config: ArenaConfig) -> Self {
        Self {
            arena: Arena::new(config),
            generation: Cell::new(0),
        }
    }

    /// Gets current generation counter
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// Allocates raw memory in arena
    pub fn alloc_bytes(&self, size: usize, align: usize) -> Result<NonNull<u8>, MemoryError> {
        self.arena
            .alloc_bytes_aligned(size, align)
            .map(|ptr| unsafe { NonNull::new_unchecked(ptr) })
    }

    /// Allocates and initializes a value
    pub fn alloc<T>(&self, value: T) -> Result<LocalRef<T>, MemoryError> {
        Ok(LocalRef {
            ptr: self.arena.alloc(value)?.into(),
            generation: self.generation.get(),
            _phantom: PhantomData,
        })
    }

    /// Allocates uninitialized memory
    pub fn alloc_uninit<T>(&self) -> Result<LocalRefMut<MaybeUninit<T>>, MemoryError> {
        Ok(LocalRefMut {
            ptr: self.arena.alloc_uninit::<T>()?.into(),
            generation: self.generation.get(),
            _phantom: PhantomData,
        })
    }

    /// Allocates and copies a slice
    pub fn alloc_slice<T: Copy>(&self, slice: &[T]) -> Result<LocalRef<[T]>, MemoryError> {
        Ok(LocalRef {
            ptr: self.arena.alloc_slice(slice)?.into(),
            generation: self.generation.get(),
            _phantom: PhantomData,
        })
    }

    /// Allocates a string
    pub fn alloc_str(&self, s: &str) -> Result<LocalRef<str>, MemoryError> {
        Ok(LocalRef {
            ptr: self.arena.alloc_str(s)?.into(),
            generation: self.generation.get(),
            _phantom: PhantomData,
        })
    }

    /// Resets the arena and increments generation
    pub fn reset(&mut self) {
        self.arena.reset();
        self.generation.set(self.generation.get().wrapping_add(1));
    }

    /// Gets arena statistics
    pub fn stats(&self) -> &super::ArenaStats {
        self.arena.stats()
    }
}

/// Immutable reference to arena-allocated value
pub struct LocalRef<T: ?Sized> {
    ptr: NonNull<T>,
    generation: u64,
    _phantom: PhantomData<T>,
}

impl<T: ?Sized> LocalRef<T> {
    /// Checks if reference is still valid
    pub fn is_valid(&self) -> bool {
        LOCAL_ARENA.with(|arena| arena.borrow().generation() == self.generation)
    }

    /// Gets reference if valid
    pub fn get(&self) -> &T {
        assert!(self.is_valid(), "LocalRef used after arena reset");
        unsafe { self.ptr.as_ref() }
    }

    /// Tries to get reference
    pub fn try_get(&self) -> Option<&T> {
        self.is_valid().then(|| unsafe { self.ptr.as_ref() })
    }
}

impl<T: ?Sized> std::ops::Deref for LocalRef<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.get()
    }
}

/// Mutable reference to arena-allocated value
pub struct LocalRefMut<T: ?Sized> {
    ptr: NonNull<T>,
    generation: u64,
    _phantom: PhantomData<T>,
}

impl<T: ?Sized> LocalRefMut<T> {
    /// Checks if reference is still valid
    pub fn is_valid(&self) -> bool {
        LOCAL_ARENA.with(|arena| arena.borrow().generation() == self.generation)
    }

    /// Gets reference if valid
    pub fn get(&self) -> &T {
        assert!(self.is_valid(), "LocalRefMut used after arena reset");
        unsafe { self.ptr.as_ref() }
    }

    /// Gets mutable reference if valid
    pub fn get_mut(&mut self) -> &mut T {
        assert!(self.is_valid(), "LocalRefMut used after arena reset");
        unsafe { self.ptr.as_mut() }
    }

    /// Tries to get reference
    pub fn try_get(&self) -> Option<&T> {
        self.is_valid().then(|| unsafe { self.ptr.as_ref() })
    }

    /// Tries to get mutable reference
    pub fn try_get_mut(&mut self) -> Option<&mut T> {
        self.is_valid().then(|| unsafe { self.ptr.as_mut() })
    }
}

impl<T: ?Sized> std::ops::Deref for LocalRefMut<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T: ?Sized> std::ops::DerefMut for LocalRefMut<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T> LocalRefMut<MaybeUninit<T>> {
    /// Initializes the value
    pub fn init(mut self, value: T) -> LocalRefMut<T> {
        unsafe { self.ptr.as_mut().write(value) };

        LocalRefMut {
            ptr: self.ptr.cast(),
            generation: self.generation,
            _phantom: PhantomData,
        }
    }
}

/// Executes closure with thread-local arena
pub fn with_local_arena<F, R>(f: F) -> R
where
    F: FnOnce(&LocalArena) -> R,
{
    LOCAL_ARENA.with(|arena| f(&arena.borrow()))
}

/// Executes closure with mutable thread-local arena
pub fn with_local_arena_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut LocalArena) -> R,
{
    LOCAL_ARENA.with(|arena| f(&mut arena.borrow_mut()))
}

/// Allocates value in thread-local arena
pub fn alloc_local<T>(value: T) -> Result<LocalRef<T>, MemoryError> {
    with_local_arena(|arena| arena.alloc(value))
}

/// Resets thread-local arena
pub fn reset_local_arena() {
    with_local_arena_mut(|arena| arena.reset());
}

/// Gets reference to the thread-local arena
pub fn local_arena() -> &'static LocalArena {
    LOCAL_ARENA.with(|arena| unsafe {
        // Создаем статическую ссылку на thread-local арену
        // Это безопасно, поскольку thread_local гарантирует,
        // что значение существует до конца работы потока
        std::mem::transmute::<&LocalArena, &'static LocalArena>(&arena.borrow())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_allocation() {
        let x = alloc_local(42).unwrap();
        assert_eq!(*x, 42);
    }

    #[test]
    fn generation_check() {
        let x = alloc_local(100).unwrap();
        assert!(x.is_valid());

        reset_local_arena();
        assert!(!x.is_valid());
    }

    #[test]
    fn thread_isolation() {
        use std::thread;

        let handle = thread::spawn(|| {
            let x = alloc_local(123).unwrap();
            assert_eq!(*x, 123);
        });

        handle.join().unwrap();

        let y = alloc_local(456).unwrap();
        assert_eq!(*y, 456);
    }
}
