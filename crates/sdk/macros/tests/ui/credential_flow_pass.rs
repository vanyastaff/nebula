//! Tests for Credential derive macro with FlowProtocol sub-attributes.

use nebula_macros::Credential;
include!("support.rs");

// ── Case 1: OAuth2 flow with #[oauth2(...)] ───────────────────────────────

#[derive(Credential)]
#[credential(
    key = "gh-oauth2",
    name = "GitHub OAuth2",
    description = "Authenticate with GitHub via OAuth2",
    extends = protocols::OAuth2Protocol,
)]
#[oauth2(
    auth_url  = "https://github.com/login/oauth/authorize",
    token_url = "https://github.com/login/oauth/access_token",
    scopes    = ["repo", "user"],
    auth_style = PostBody,
)]
pub struct GithubOauth2;

// ── Case 2: LDAP flow with #[ldap(...)] ──────────────────────────────────

#[derive(Credential)]
#[credential(
    key = "corp-ldap",
    name = "Corporate LDAP",
    description = "LDAP bind for corporate directory",
    extends = protocols::LdapProtocol,
)]
#[ldap(
    tls = Tls,
    timeout_secs = 15,
)]
pub struct CorporateLdap;

fn main() {
    let _desc = GithubOauth2::description();
    assert_eq!(_desc.key, "gh-oauth2");
    assert_eq!(_desc.name, "GitHub OAuth2");

    let _config = GithubOauth2::oauth2_config();
    assert_eq!(_config.auth_url, "https://github.com/login/oauth/authorize");
    assert_eq!(_config.scopes, vec!["repo".to_string(), "user".to_string()]);

    let _ldap_desc = CorporateLdap::description();
    assert_eq!(_ldap_desc.key, "corp-ldap");
}
