//! Infrastructure traits for storage and locking

mod credential;
mod lock;
mod storage;

pub use credential::{Credential, InteractiveCredential};
pub use lock::{DistributedLock, LockError, LockGuard};
pub use storage::{StateStore, StateVersion};

