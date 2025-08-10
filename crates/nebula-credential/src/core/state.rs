use serde::{Serialize, Deserialize};

/// Trait for credential state types
pub trait CredentialState: Serialize + for<'de> Deserialize<'de> + Send + Sync + Clone {
    /// Version of this state schema
    const VERSION: u16;

    /// Kind identifier for this credential type
    const KIND: &'static str;

    /// Scrub ephemeral data before persistence
    fn scrub_ephemeral(&mut self) {}
}