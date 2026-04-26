//! Probe - `dyn Credential` rejected at compile time. See parent
//! driver `compile_fail_dyn_credential_const_key.rs`.
//!
//! Two errors fire here. Both are blockers for `dyn Credential`:
//!
//! - **`E0191`** at the bare `dyn Credential` site - the associated
//!   types (`Input`/`Scheme`/`State`) must be specified for the trait
//!   object to be well-formed as a Rust type.
//! - **`E0038`** at the fully-qualified site - even with the assoc
//!   types specified, `const KEY` is not dyn-compatible (no method
//!   carrier; the dyn-vtable cannot project an associated const). This
//!   second site demonstrates that the const-KEY block fires
//!   independently of the assoc-type set.
//!
//! Together these capture the structural reason the phantom-shim
//! pattern (ADR-0035 1) exists: `dyn Credential` is doubly blocked,
//! and any `dyn`-position use must route through a phantom trait that
//! has neither `const KEY` nor unspecified associated types.

use nebula_credential::Credential;

fn _take_bare(_c: &dyn Credential) {}

fn _take_typed<I, S, St>(
    _c: &dyn Credential<Input = I, Scheme = S, State = St>,
) where
    I: nebula_schema::HasSchema + Send + Sync + 'static,
    S: nebula_credential::AuthScheme,
    St: nebula_credential::CredentialState,
{
}

fn main() {}
