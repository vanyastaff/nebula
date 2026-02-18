//! Infrastructure traits for storage, locking, and rotation

mod credential;
mod lock;
mod rotation;
mod storage;
mod testable;

pub use credential::{
    CredentialResource, CredentialType, FlowProtocol, InteractiveCredential, Refreshable,
    Revocable, StaticProtocol,
};
pub use lock::{DistributedLock, LockError, LockGuard};
pub use rotation::RotatableCredential;
pub use storage::{StateStore, StateVersion, StorageProvider};
pub use testable::TestableCredential;
