//! Probe: a third-party type that falsely reports `IsRefreshable::VALUE =
//! true` cannot pass itself to a `where C: Refreshable` engine dispatcher.
//!
//! # Context
//!
//! Tech Spec §15.4 splits capabilities into dedicated sub-traits (`Refreshable`,
//! `Revocable`, etc.) so the engine can bind by trait rather than by a
//! runtime const. A plugin author that tries to game the system by
//! hand-implementing `plugin_capability_report::IsRefreshable { VALUE = true }`
//! — without a real `impl Refreshable for ThirdPartyCred` — still cannot
//! reach the engine's refresh dispatcher: the dispatcher binds on the
//! *sub-trait* (`where C: Refreshable`), not on the reporting const.
//!
//! This probe pins that guarantee. The failing line is the call to
//! `RefreshDispatcher::for_credential::<ThirdPartyLiar>()` — E0277
//! because `ThirdPartyLiar` does not implement `Refreshable`, regardless
//! of what `IsRefreshable::VALUE` claims.
//!
//! Closes security-lead findings N1 + N3 + N5 at the type level:
//! - N1: A plugin cannot self-attest refresh capability and reach refresh
//!   machinery without the actual `Refreshable` impl.
//! - N3: The engine's dispatch gate is structural (trait bound), not a
//!   runtime const-bool read that could be spoofed.
//! - N5: The lying `VALUE = true` is harmless at dispatch — the engine
//!   never reads it; the bound fails at monomorphisation.

use std::future::Future;

use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, CredentialState, Refreshable, SecretString,
    contract::plugin_capability_report, error::CredentialError, resolve::ResolveResult,
    scheme::SecretToken,
};
use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};

// ── Stand-in state ──────────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct ThirdPartyState {
    token: String,
}

impl CredentialState for ThirdPartyState {
    const KIND: &'static str = "third_party_state";
    const VERSION: u32 = 1;
}

// ── Third-party credential ─────────────────────────────────────────────────
//
// Hand-rolled (no `#[derive(Credential)]`) so the author can craft arbitrary
// `plugin_capability_report` impls without the macro's parity check
// catching the inconsistency.

struct ThirdPartyLiar;

impl Credential for ThirdPartyLiar {
    type Properties = FieldValues;
    type Scheme = SecretToken;
    type State = ThirdPartyState;

    const KEY: &'static str = "third_party_liar";

    fn metadata() -> CredentialMetadata
    where
        Self: Sized,
    {
        unimplemented!("fixture")
    }

    fn project(state: &ThirdPartyState) -> SecretToken
    where
        Self: Sized,
    {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<ThirdPartyState, ()>, CredentialError>> + Send
    where
        Self: Sized,
    {
        async { unimplemented!("fixture") }
    }
}

// ── Lying capability report ────────────────────────────────────────────────
//
// `IsRefreshable::VALUE = true` with no `impl Refreshable for ThirdPartyLiar`.
// This is the "lie" the engine's const-bool path in pre-§15.4 code would
// have believed. Under §15.4 the engine dispatcher binds on the *sub-trait*,
// not on this const, so the lie is structurally inert.

impl plugin_capability_report::IsInteractive for ThirdPartyLiar {
    const VALUE: bool = false;
}
// Lie: claims refresh capability but has no `impl Refreshable`.
impl plugin_capability_report::IsRefreshable for ThirdPartyLiar {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsRevocable for ThirdPartyLiar {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for ThirdPartyLiar {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for ThirdPartyLiar {
    const VALUE: bool = false;
}

// ── Stub dispatcher — mirrors engine's `RefreshDispatcher::for_credential` ─

struct RefreshDispatcher;

impl RefreshDispatcher {
    /// Stand-in for `nebula_engine::credential::rotation::RefreshDispatcher::for_credential`.
    /// The real engine dispatcher carries the same `where C: Refreshable` bound.
    fn for_credential<C: Refreshable>() -> Self {
        Self
    }
}

fn main() {
    // THE failing line — `ThirdPartyLiar` does not implement `Refreshable`.
    // `IsRefreshable::VALUE = true` is irrelevant: the dispatcher binds on
    // the sub-trait, not the const. E0277.
    let _ = RefreshDispatcher::for_credential::<ThirdPartyLiar>();
}
