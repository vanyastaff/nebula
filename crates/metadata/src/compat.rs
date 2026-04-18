//! Version-compatibility helpers shared across metadata families.
//!
//! For v1 the only family that versions its metadata is
//! [`ActionMetadata`](../../nebula_action/struct.ActionMetadata.html); the
//! canonical compatibility rules live in `nebula-action` because they
//! reference action-specific fields (ports, isolation level).
//!
//! This module reserves the name and is expected to grow as credentials
//! and resources gain their own versioned evolution stories.
