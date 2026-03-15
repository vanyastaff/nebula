//! Poison protection for mutable state that crosses fallible sections.
//!
//! This helper is designed for critical sections where an interrupted mutation
//! would leave the wrapped value in an unknown state.
//!
//! # Example
//!
//! ```rust
//! use nebula_resource::poison::Poison;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let mut protected = Poison::new("pool_state", Vec::<u8>::new());
//! let mut guard = protected.check_and_arm()?;
//! guard.data_mut().push(42);
//! tokio::time::sleep(std::time::Duration::from_millis(1)).await;
//! guard.disarm();
//! # Ok(())
//! # }
//! ```

use chrono::{DateTime, Utc};

/// Wraps a value that must not be reused if a mutation was interrupted.
pub struct Poison<T> {
    what: &'static str,
    state: PoisonState,
    data: T,
}

#[derive(Clone, Copy)]
enum PoisonState {
    Clean,
    Armed,
    Poisoned { at: DateTime<Utc> },
}

impl<T> Poison<T> {
    /// Create a new protected value.
    #[must_use]
    pub fn new(what: &'static str, data: T) -> Self {
        Self {
            what,
            state: PoisonState::Clean,
            data,
        }
    }

    /// Returns a guard that gives `&mut T` access.
    /// Returns `Err(PoisonError)` if already poisoned.
    pub fn check_and_arm(&mut self) -> Result<PoisonGuard<'_, T>, PoisonError> {
        match self.state {
            PoisonState::Clean => {
                self.state = PoisonState::Armed;
                Ok(PoisonGuard(self))
            }
            PoisonState::Armed => {
                let at = Utc::now();
                self.state = PoisonState::Poisoned { at };
                tracing::warn!(
                    what = self.what,
                    poisoned_at = %at,
                    "poisoned state after re-entering an armed critical section"
                );
                Err(PoisonError::Poisoned {
                    what: self.what,
                    at,
                })
            }
            PoisonState::Poisoned { at } => Err(PoisonError::Poisoned {
                what: self.what,
                at,
            }),
        }
    }

    /// True if this value has been poisoned.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        matches!(self.state, PoisonState::Poisoned { .. })
    }

    /// Read access without entering a critical section.
    ///
    /// Unlike [`check_and_arm`](Self::check_and_arm), this does **not** set the
    /// state to `Armed`, so a panic or early return during the read cannot
    /// accidentally poison the value. Use this for read-only observers.
    ///
    /// # Errors
    /// Returns `Err` if the value has already been poisoned.
    pub fn try_read(&self) -> Result<&T, PoisonError> {
        match self.state {
            PoisonState::Poisoned { at } => Err(PoisonError::Poisoned {
                what: self.what,
                at,
            }),
            _ => Ok(&self.data),
        }
    }
}

/// RAII guard. Dropping without calling `disarm()` poisons the value.
pub struct PoisonGuard<'a, T>(&'a mut Poison<T>);

impl<T> PoisonGuard<'_, T> {
    /// Borrow protected data immutably.
    pub fn data(&self) -> &T {
        &self.0.data
    }

    /// Borrow protected data mutably.
    pub fn data_mut(&mut self) -> &mut T {
        &mut self.0.data
    }

    /// Mark the critical section as successfully completed.
    pub fn disarm(self) {
        if matches!(self.0.state, PoisonState::Armed) {
            self.0.state = PoisonState::Clean;
        }
        // `self` drops here; Drop checks for Armed state, which is now
        // Clean, so nothing happens — mem::forget is not needed.
    }
}

impl<T> Drop for PoisonGuard<'_, T> {
    fn drop(&mut self) {
        if matches!(self.0.state, PoisonState::Armed) {
            let at = Utc::now();
            self.0.state = PoisonState::Poisoned { at };
            tracing::warn!(
                what = self.0.what,
                poisoned_at = %at,
                "critical section dropped without disarm; poisoning state"
            );
        }
    }
}

/// Error returned when accessing a poisoned value.
#[derive(Debug, thiserror::Error)]
pub enum PoisonError {
    /// State is poisoned and cannot be reused.
    #[error("state '{what}' was poisoned at {at}")]
    Poisoned {
        /// Name of the protected state.
        what: &'static str,
        /// Timestamp when state was poisoned.
        at: DateTime<Utc>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_can_arm_and_disarm() {
        let mut protected = Poison::new("int", 1_i32);
        let mut guard = protected.check_and_arm().expect("should arm");
        *guard.data_mut() = 2;
        guard.disarm();
        assert!(!protected.is_poisoned());
    }

    #[test]
    fn drop_without_disarm_poisons() {
        let mut protected = Poison::new("int", 1_i32);
        {
            let _guard = protected.check_and_arm().expect("should arm");
        }
        assert!(protected.is_poisoned());
    }

    #[test]
    fn reentrant_arm_poisons() {
        let mut protected = Poison::new("state", 0_i32);
        // The borrow checker prevents a second `check_and_arm` while the first
        // guard is alive (safe Rust). Simulate the re-entrant scenario that can
        // arise through unsafe aliasing (e.g. raw-pointer access in pool code)
        // by setting the state directly — tests are in the same module.
        protected.state = PoisonState::Armed;
        assert!(protected.check_and_arm().is_err()); // temporary dropped here
        assert!(protected.is_poisoned());
    }

    #[test]
    fn poisoned_state_rejects_second_arm() {
        let mut protected = Poison::new("counter", 0_i32);
        {
            let _guard = protected.check_and_arm().expect("should arm");
        }

        let err = match protected.check_and_arm() {
            Ok(_) => panic!("must be poisoned"),
            Err(err) => err,
        };
        match err {
            PoisonError::Poisoned { what, .. } => assert_eq!(what, "counter"),
        }
    }
}
