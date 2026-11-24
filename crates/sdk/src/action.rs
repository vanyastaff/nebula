//! Action authoring contracts.
//!
//! Action authors declare the key once as [`Action::KEY`] and return a keyless
//! [`ActionMetadata`] from [`Action::metadata`]. Runtime factories stamp the
//! key, bind the associated [`Action::Input`] and
//! [`Action::Output`] schemas and admit the terminal metadata; that admitted
//! state is intentionally outside the supported SDK surface.
//!
//! # Example
//!
//! ```rust
//! use nebula_sdk::action::{ActionMetadata, metadata_name};
//!
//! let metadata = ActionMetadata::new(metadata_name!("Greet"), "Produces a greeting");
//! let _: ActionMetadata = metadata;
//! ```

pub use nebula_action::{
    Action, ActionEffectContract, ActionMetadata, InputPort, IsolationLevel, JournalProtocol,
    OutputPort, key_of,
};
pub use nebula_core::{ActionKey, Dependencies, action_key};
pub use nebula_metadata::{
    DeprecationNotice, Icon, MetadataError, MetadataName, MetadataVersion, metadata_name,
};
