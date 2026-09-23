use nebula_credential::{
    CredentialDisplay, CredentialService, OAuth2Credential, TenantScope, UserInput,
};

fn bypass(service: &CredentialService, scope: &TenantScope, input: UserInput) {
    let _ = service.create(scope, "api_key", serde_json::json!({}), CredentialDisplay::default());
    let _ = service.update(scope, "id", None, None, CredentialDisplay::default());
    let _ = service.delete(scope, "id");
    let _ = service.test(scope, "id");
    let _ = service.refresh(scope, "id");
    let _ = service.revoke(scope, "id");
    let _ = service.resolve(scope, "oauth2", serde_json::json!({}));
    let _ = service.continue_resolve(scope, "oauth2", "pending", input);
    let _ = service.scheme_factory::<OAuth2Credential>(scope, nebula_credential::CredentialId::new());
}

fn main() {
    let _ = bypass;
}
