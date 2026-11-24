//! Action authoring contracts.
//!
//! Action authors declare the key once as [`Component::KEY`] and return a keyless
//! [`Metadata`] from [`Component::metadata`]. Runtime factories stamp the
//! key, bind the associated [`Component::Config`] and
//! [`Action::Output`] schemas and admit the terminal metadata; that admitted
//! state is intentionally outside the supported SDK surface.
//!
//! # Example
//!
//! ```rust
//! use nebula_sdk::action::{Metadata, metadata_name};
//!
//! let metadata = Metadata::new(metadata_name!("Greet"), "Produces a greeting");
//! let _: Metadata = metadata;
//! ```

pub use nebula_action::{
    Action, ActionEffectContract, Component, InputPort, IsolationLevel, JournalProtocol, Metadata,
    OutputPort, key_of, kind,
};
pub use nebula_core::{ActionKey, action_key};
pub use nebula_metadata::{
    DeprecationNotice, Icon, MetadataError, MetadataName, MetadataVersion, metadata_name,
};
