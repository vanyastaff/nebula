//! Action authoring contracts.
//!
//! Action authors return an [`ActionMetadataDraft`] from [`Action::metadata`].
//! Runtime factories bind the associated [`Action::Input`] and
//! [`Action::Output`] schemas and admit the terminal metadata; that admitted
//! state is intentionally outside the supported SDK surface.
//!
//! # Example
//!
//! ```rust
//! use nebula_sdk::action::{ActionMetadataDraft, action_key, metadata_name};
//!
//! let draft = ActionMetadataDraft::new(
//!     action_key!("example.greet"),
//!     metadata_name!("Greet"),
//!     "Produces a greeting",
//! );
//! let _: ActionMetadataDraft = draft;
//! ```

pub use nebula_action::{
    Action, ActionEffectContract, ActionMetadataDraft, CheckpointPolicy, InputPort, IsolationLevel,
    OutputPort,
};
pub use nebula_core::{ActionKey, Dependencies, action_key};
pub use nebula_metadata::{
    DeprecationNotice, Icon, MetadataError, MetadataName, MetadataVersion, metadata_name,
};
