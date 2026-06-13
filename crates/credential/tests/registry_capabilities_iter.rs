//! Registry capability detection coverage (Tech Spec §15.8).
//!
//! Per Tech Spec §15.8 (closes security-lead N6) capability discovery
//! reads the bitflag set computed by `compute_capabilities::<C>()` from
//! per-credential `plugin_capability_report::IsX::VALUE` constants. This
//! integration probe registers a static credential and a richly-capable
//! one, then asserts:
//!
//! 1. `capabilities_of(key)` returns the exact bitflag set declared by the type's IsX impls — no
//!    self-attestation surface.
//! 2. `iter_compatible(required)` filters by `bitflags::contains` semantics — empty set yields
//!    every entry; multi-flag set ANDs.
//! 3. Filtered iterator excludes entries that miss at least one required flag — load-bearing for
//!    operator-UI / discovery code that picks "which credentials can refresh + revoke".

use nebula_core::auth::{
    AuthPattern, AuthScheme, EgressShape, RefreshStrategyKind, SchemeFamily, SensitiveScheme,
};
use nebula_credential::{
    Capabilities, Credential, CredentialContext, CredentialMetadata, CredentialRegistry,
    CredentialState, Refreshable, Revocable, SecretString,
    contract::plugin_capability_report,
    error::CredentialError,
    resolve::{RefreshOutcome, ResolveResult},
    scheme::SecretToken,
};
use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};

// ── Static probe credential — zero capabilities ────────────────────

pub struct StaticProbe;

impl Credential for StaticProbe {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "probe.static";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("probe.static"))
            .name("StaticProbe")
            .description("zero-capability probe credential")
            .schema(nebula_credential::schema_of::<Self::Properties>())
            .pattern(AuthPattern::SecretToken)
            .build()
            .expect("StaticProbe metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new("static-probe"),
        )))
    }
}

impl plugin_capability_report::IsInteractive for StaticProbe {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for StaticProbe {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for StaticProbe {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for StaticProbe {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for StaticProbe {
    const VALUE: bool = false;
}

// ── Active-refresh probe scheme + family ────────────────────────────
//
// A `Refreshable` credential is sound only on a scheme whose family declares an
// active wire-intrinsic refresh class (F3 containment law, enforced at
// registration). `SecretToken`'s family is `Static`-only, so the refreshable
// probe carries its own minimal scheme whose family permits `RefreshToken`.

pub struct ProbeRefreshFamily;

impl SchemeFamily for ProbeRefreshFamily {
    const EGRESS: &'static [EgressShape] = &[EgressShape::InlineSecret];
    fn refresh_classes() -> &'static [RefreshStrategyKind] {
        &[RefreshStrategyKind::RefreshToken]
    }
    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }
}

#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct ProbeToken(String);

impl AuthScheme for ProbeToken {
    type Family = ProbeRefreshFamily;
    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }
}

impl SensitiveScheme for ProbeToken {}

impl CredentialState for ProbeToken {
    const KIND: &'static str = "probe.refreshable.token";
    const VERSION: u32 = 1;
}

// ── Refreshable + Revocable probe credential ────────────────────────

pub struct RefreshableProbe;

impl Credential for RefreshableProbe {
    type Properties = ();
    type Scheme = ProbeToken;
    type State = ProbeToken;

    const KEY: &'static str = "probe.refreshable";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("probe.refreshable"))
            .name("RefreshableProbe")
            .description("refreshable + revocable probe credential")
            .schema(nebula_credential::schema_of::<Self::Properties>())
            .pattern(AuthPattern::OAuth2)
            .build()
            .expect("RefreshableProbe metadata is valid")
    }

    fn project(state: &ProbeToken) -> ProbeToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<ProbeToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(ProbeToken(
            "refreshable-probe".to_owned(),
        )))
    }
}

impl Refreshable for RefreshableProbe {
    async fn refresh(
        _state: &mut ProbeToken,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        Ok(RefreshOutcome::Refreshed)
    }
}

impl Revocable for RefreshableProbe {
    async fn revoke(
        _state: &mut ProbeToken,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }
}

impl plugin_capability_report::IsInteractive for RefreshableProbe {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for RefreshableProbe {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsRevocable for RefreshableProbe {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsTestable for RefreshableProbe {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for RefreshableProbe {
    const VALUE: bool = false;
}

// ── Probe assertions ────────────────────────────────────────────────

#[test]
fn capabilities_of_reports_static_probe_as_empty() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(StaticProbe, env!("CARGO_CRATE_NAME"))
        .expect("static-probe registration must succeed");

    let caps = registry
        .capabilities_of("probe.static")
        .expect("registered key must have a capability set");
    assert_eq!(
        caps,
        Capabilities::empty(),
        "static probe declares no capabilities; bitflag set must be empty"
    );
}

#[test]
fn capabilities_of_reports_refreshable_probe_as_refreshable_plus_revocable() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(RefreshableProbe, env!("CARGO_CRATE_NAME"))
        .expect("refreshable-probe registration must succeed");

    let caps = registry
        .capabilities_of("probe.refreshable")
        .expect("registered key must have a capability set");
    assert_eq!(
        caps,
        Capabilities::REFRESHABLE | Capabilities::REVOCABLE,
        "refreshable probe declares Refreshable + Revocable; bitflag set must reflect that"
    );
}

#[test]
fn capabilities_of_returns_none_for_unknown_key() {
    let registry = CredentialRegistry::new();
    assert!(registry.capabilities_of("nonexistent.key").is_none());
}

#[test]
fn iter_compatible_with_empty_filter_returns_every_entry() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(StaticProbe, env!("CARGO_CRATE_NAME"))
        .expect("static registration");
    registry
        .register(RefreshableProbe, env!("CARGO_CRATE_NAME"))
        .expect("refreshable registration");

    let mut keys: Vec<&str> = registry
        .iter_compatible(Capabilities::empty())
        .map(|(k, _)| k)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["probe.refreshable", "probe.static"]);
}

#[test]
fn iter_compatible_with_refreshable_filter_excludes_static_probe() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(StaticProbe, env!("CARGO_CRATE_NAME"))
        .expect("static registration");
    registry
        .register(RefreshableProbe, env!("CARGO_CRATE_NAME"))
        .expect("refreshable registration");

    let matched: Vec<&str> = registry
        .iter_compatible(Capabilities::REFRESHABLE)
        .map(|(k, _)| k)
        .collect();

    assert_eq!(
        matched,
        vec!["probe.refreshable"],
        "Refreshable filter must include refreshable probe and exclude the static one"
    );
}

#[test]
fn iter_compatible_with_anded_filter_requires_every_flag() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(StaticProbe, env!("CARGO_CRATE_NAME"))
        .expect("static registration");
    registry
        .register(RefreshableProbe, env!("CARGO_CRATE_NAME"))
        .expect("refreshable registration");

    // RefreshableProbe declares REFRESHABLE + REVOCABLE; the filter
    // matches it because both required flags are present.
    let combined: Vec<&str> = registry
        .iter_compatible(Capabilities::REFRESHABLE | Capabilities::REVOCABLE)
        .map(|(k, _)| k)
        .collect();
    assert_eq!(combined, vec!["probe.refreshable"]);

    // Asking for a flag the refreshable probe does NOT declare (TESTABLE)
    // returns nothing.
    let none: Vec<&str> = registry
        .iter_compatible(Capabilities::REFRESHABLE | Capabilities::TESTABLE)
        .map(|(k, _)| k)
        .collect();
    assert!(
        none.is_empty(),
        "AND filter requires every flag; missing TESTABLE must exclude every entry"
    );
}
