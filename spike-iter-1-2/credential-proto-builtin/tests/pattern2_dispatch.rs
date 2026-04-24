//! Pattern 2 end-to-end dispatch (iter-2 orchestrator step 3).
//!
//! Load-bearing test for Q3 ergonomics. Flow:
//!   1. Registry holds `Box<dyn AnyCredential>` (erased concrete credential).
//!   2. Engine / test harness also holds the matching `State` per credential
//!      (in production: pulled from storage layer, decrypted).
//!   3. Action is declared with `CredentialRef<dyn BitbucketBearerPhantom>`
//!      — the phantom NAMES a capability, it does NOT hold the value.
//!   4. At invocation, the engine resolves credential+state, projects the
//!      scheme via `C::project(&state)`, and hands the action an
//!      `&BearerScheme` (the concrete Scheme, not a dyn).
//!   5. Action body uses the scheme for HTTP Authorization.
//!
//! The question this answers: can H1 (TypeId registry) support this flow
//! WITHOUT the action enumerating every concrete credential type?
//!
//! Answer (validated below): NO under pure H1. The engine-side `resolve +
//! project` requires ONE of:
//!   (a) Per-capability object-safe projection trait on `AnyCredential`
//!       (`project_as_bearer(&dyn AnyCredential, &dyn AnyState) -> Option<BearerScheme>`).
//!   (b) Macro-generated per-credential-type dispatch table keyed by
//!       TypeId (H2 shape).
//!   (c) Engine performs `Credential::project` at state-decryption time
//!       (before handing anything to the action), storing the Scheme in
//!       the resolved context. Action never touches Credential.
//!
//! The spike models path (c) as the most realistic — engine has the type
//! info (it just decrypted the state using the credential's typed State),
//! so projection happens there with full type knowledge. The action receives
//! a Scheme reference.
//!
//! This validates the Strategy §3.5 mechanism (i) direction at a sketch
//! level: Resource's `AcceptedAuth: SchemeInjector` is the narrow
//! object-safe trait through which the Scheme flows to the action body.

use credential_proto::{Credential, CredentialKey, CredentialRegistry};
use credential_proto_builtin::{
    BearerScheme, BitbucketOAuth2, BitbucketPat, OAuth2State, PatState,
};

// ─── SchemeInjector — narrow object-safe trait (Strategy §3.5 direction) ───
//
// `dyn SchemeInjector` is well-formed (no assoc types on method signatures).
// Each concrete Scheme opts in; the phantom shim is NOT needed here because
// Scheme's trait `AuthScheme` has no assoc types — it's the Credential trait
// that required the shim, not the Scheme trait.

trait SchemeInjector {
    /// Pretend this sets an HTTP Authorization header. Real impl would take
    /// `&mut http::Request`; spike returns the header string for test.
    fn inject(&self) -> String;
}

impl SchemeInjector for BearerScheme {
    fn inject(&self) -> String {
        format!("Authorization: Bearer {}", self.token)
    }
}

// ─── Resolved credential context — what the engine hands the action ────────
//
// Iter-2 finding: the ACTION does NOT receive a `&dyn AnyCredential` and
// downcast. It receives a projected scheme reference. The phantom trait in
// the action struct declaration is purely a TYPE-CHECKING signal — it binds
// "the credential slot expects BearerScheme-producing credentials". At
// invocation, the engine has already done the projection.

struct ResolvedBearer(BearerScheme);

impl ResolvedBearer {
    fn inject(&self) -> String {
        <BearerScheme as SchemeInjector>::inject(&self.0)
    }
}

// ─── Engine-side resolve+project (generic over concrete credential type) ───
//
// In production, this function is MACRO-GENERATED per (credential-type,
// state-storage) pair. For the spike, hand-write it generic over C.
//
// KEY CONSTRAINT: the where-clause `C::Scheme = BearerScheme` is how the
// engine guarantees "this credential produces the right scheme for the
// target resource." Without it, a SlackOAuth2Credential (also Scheme=
// BearerScheme) could be registered at a Bitbucket slot at runtime, and
// only the macro-emitted where-clause bound on registration catches it.

fn resolve_as_bearer<C>(
    registry: &CredentialRegistry,
    key: &str,
    state: &C::State,
) -> Option<ResolvedBearer>
where
    C: Credential<Scheme = BearerScheme>,
{
    // The registry confirms the concrete type matches C (via downcast_ref).
    // Not strictly needed in this sketch (we already have typed state
    // passed in) — but demonstrates the TypeId safety path is still viable.
    let _cred = registry.resolve_concrete::<C>(key)?;
    Some(ResolvedBearer(C::project(state)))
}

// ─── Test: the Pattern 2 flow end-to-end ────────────────────────────────────

#[test]
fn pattern2_oauth2_resolves_and_injects() {
    let mut reg = CredentialRegistry::new();
    let key = CredentialKey::new("ws_a/oauth");
    reg.insert(key.clone(), BitbucketOAuth2);

    let state = OAuth2State {
        access_token: "oauth_tok".into(),
        refresh_token: "oauth_refresh".into(),
    };
    let resolved = resolve_as_bearer::<BitbucketOAuth2>(&reg, key.as_str(), &state)
        .expect("OAuth2 resolves as bearer");
    assert_eq!(resolved.inject(), "Authorization: Bearer oauth_tok");
}

#[test]
fn pattern2_pat_resolves_and_injects() {
    let mut reg = CredentialRegistry::new();
    let key = CredentialKey::new("ws_a/pat");
    reg.insert(key.clone(), BitbucketPat);

    let state = PatState { token: "pat_tok".into() };
    let resolved = resolve_as_bearer::<BitbucketPat>(&reg, key.as_str(), &state)
        .expect("PAT resolves as bearer");
    assert_eq!(resolved.inject(), "Authorization: Bearer pat_tok");
}

// AppPassword CANNOT compile into resolve_as_bearer because its Scheme is
// BasicScheme, not BearerScheme. The where-clause `C::Scheme = BearerScheme`
// rejects it at compile time. This is §3.5 mechanism (i) cross-check in
// action — verified by compile_fail example in examples/ directory.
