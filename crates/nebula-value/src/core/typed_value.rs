//! # TypedValue Implementation
//!
//! This module provides the core `TypedValue<T>` wrapper that enables
//! zero-cost abstractions for any type T in the nebula-value system.

use std::fmt;

/// The core zero-cost wrapper for any type T.
///
/// `TypedValue<T>` provides a transparent wrapper around any type T,
/// enabling seamless integration with the nebula-value ecosystem while
/// maintaining zero runtime overhead.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypedValue<T> {
    inner: T,
}

impl<T> TypedValue<T> {
    /// Creates a new `TypedValue<T>` wrapping the given value.
    #[inline(always)]
    pub const fn new(value: T) -> Self {
        Self { inner: value }
    }

    /// Returns a reference to the wrapped value.
    #[inline(always)]
    pub const fn inner(&self) -> &T {
        &self.inner
    }

    /// Returns a mutable reference to the wrapped value.
    #[inline(always)]
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consumes the `TypedValue<T>` and returns the wrapped value.
    #[inline(always)]
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Maps the wrapped value using the provided function.
    #[inline]
    pub fn map<U, F>(self, f: F) -> TypedValue<U>
    where
        F: FnOnce(T) -> U,
    {
        TypedValue::new(f(self.inner))
    }

    /// Tries to map the wrapped value using the provided fallible function.
    #[inline]
    pub fn try_map<U, E, F>(self, f: F) -> Result<TypedValue<U>, E>
    where
        F: FnOnce(T) -> Result<U, E>,
    {
        f(self.inner).map(TypedValue::new)
    }
}

// Default implementation
impl<T: Default> Default for TypedValue<T> {
    #[inline]
    fn default() -> Self {
        Self::new(T::default())
    }
}

// Display implementation
impl<T: fmt::Display> fmt::Display for TypedValue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

// Deref implementation for convenient access
impl<T> std::ops::Deref for TypedValue<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// DerefMut implementation for convenient mutable access
impl<T> std::ops::DerefMut for TypedValue<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

// AsRef implementation
impl<T> AsRef<T> for TypedValue<T> {
    #[inline(always)]
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

// AsMut implementation
impl<T> AsMut<T> for TypedValue<T> {
    #[inline(always)]
    fn as_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

// From implementation for convenient construction
impl<T> From<T> for TypedValue<T> {
    #[inline(always)]
    fn from(value: T) -> Self {
        Self::new(value)
    }
}