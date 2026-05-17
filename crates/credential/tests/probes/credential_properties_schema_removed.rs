//! Probe — ADR-0052 P3: `Credential::properties_schema()` is removed.
//!
//! The `Properties: HasSchema` associated-type bound is the single source of
//! truth; schema is reached via `nebula_schema::schema_of::<C::Properties>()`.
//! There is no `properties_schema()` method on the trait, so this call must
//! fail to compile (`E0599`).

// FQS forces resolution on the `Credential` trait itself, so the only
// reason this fails post-P3 is the removed method — not a missing trait
// import or an inherent-method shadow.
use nebula_credential::{Credential, NoCredential};

fn main() {
    let _ = <NoCredential as Credential>::properties_schema();
}
