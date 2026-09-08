//! Backend identities shared by gate policy and runtime-authority evidence.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Ord, PartialOrd, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Backend {
    InMemory,
    Sqlite,
    Postgresql,
}

impl From<Backend> for &'static str {
    fn from(backend: Backend) -> Self {
        match backend {
            Backend::InMemory => "in-memory",
            Backend::Sqlite => "sqlite",
            Backend::Postgresql => "postgresql",
        }
    }
}
