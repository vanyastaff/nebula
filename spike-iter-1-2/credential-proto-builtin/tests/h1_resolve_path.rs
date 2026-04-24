//! H1 (PhantomData + TypeId registry) — runtime resolve path proves out.
//!
//! Validates Strategy §3.2 last paragraph: runtime never holds a vtable
//! pointer to `dyn BitbucketBearer`. Path is:
//!   CredentialKey → registry lookup (Box<dyn AnyCredential>) → downcast to T.

use credential_proto::{CredentialKey, CredentialRegistry};
use credential_proto_builtin::{BitbucketOAuth2, BitbucketPat, OAuth2State, PatState};

#[test]
fn resolve_oauth2_concrete_succeeds() {
    let mut reg = CredentialRegistry::new();
    let key = CredentialKey::new("workspace_a/oauth");
    reg.insert(key.clone(), BitbucketOAuth2);

    let cred = reg.resolve_concrete::<BitbucketOAuth2>(key.as_str());
    assert!(cred.is_some(), "OAuth2 should resolve when registered with matching TypeId");
}

#[test]
fn resolve_with_wrong_typeid_returns_none() {
    let mut reg = CredentialRegistry::new();
    let key = CredentialKey::new("workspace_a/oauth");
    reg.insert(key.clone(), BitbucketOAuth2);

    // Asking for a different concrete type at the same key returns None,
    // not a wrong-type silent success. This is the TypeId guard (in iter-2,
    // preserved via downcast_ref's internal TypeId check).
    let cred = reg.resolve_concrete::<BitbucketPat>(key.as_str());
    assert!(cred.is_none(), "wrong-type lookup must return None, not silently succeed");
}

#[test]
fn resolve_via_any_credential_provides_metadata() {
    let mut reg = CredentialRegistry::new();
    let key = CredentialKey::new("workspace_a/oauth");
    reg.insert(key.clone(), BitbucketOAuth2);

    let any = reg.resolve_any(key.as_str()).expect("registered key resolves");
    assert_eq!(any.credential_key(), "bitbucket_oauth2");
}

#[test]
fn project_state_to_scheme_runs() {
    use credential_proto::Credential;

    let state = OAuth2State {
        access_token: "tok".into(),
        refresh_token: "ref".into(),
    };
    let scheme = BitbucketOAuth2::project(&state);
    assert_eq!(scheme.token, "tok");
}

#[test]
fn project_pat_state_to_bearer_scheme_runs() {
    use credential_proto::Credential;

    let state = PatState { token: "pat".into() };
    let scheme = BitbucketPat::project(&state);
    assert_eq!(scheme.token, "pat");
}
