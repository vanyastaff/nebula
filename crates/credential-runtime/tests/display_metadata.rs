//! Per-instance display metadata round-trips through the facade.
//!
//! `create` attaches a typed `CredentialDisplay` (name / description / tags),
//! `get` reads it back from the reserved `metadata["display"]` sub-object, and
//! `update` replaces it (cleared fields do not linger). Empty display leaves no
//! residue.

use nebula_credential::CredentialDisplay;
use nebula_credential_runtime::TenantScope;
use nebula_credential_runtime::test_support::in_memory_service;
use serde_json::json;

#[tokio::test]
async fn display_round_trips_through_create_get_and_update() {
    let svc = in_memory_service();
    let scope = TenantScope::new("org1", "ws1");

    let mut tags = std::collections::BTreeMap::new();
    tags.insert("env".to_owned(), "prod".to_owned());
    let display = CredentialDisplay {
        display_name: Some("Prod token".to_owned()),
        description: Some("the production key".to_owned()),
        tags,
    };

    // create() attaches the display to the returned snapshot verbatim.
    let created = svc
        .create(
            &scope,
            "bearer_token",
            json!({ "token": "sk-disp" }),
            display.clone(),
        )
        .await
        .expect("create ok");
    assert_eq!(created.display(), &display);

    // get() reads it back from the reserved `metadata["display"]` sub-object.
    let id = svc.list(&scope).await.expect("list")[0].clone();
    let got = svc.get(&scope, &id).await.expect("get ok");
    assert_eq!(got.display(), &display);

    // update() replaces the display; cleared fields do not linger.
    let next = CredentialDisplay {
        display_name: Some("Renamed".to_owned()),
        ..Default::default()
    };
    svc.update(&scope, &id, json!({ "token": "sk-disp" }), 1, next.clone())
        .await
        .expect("update ok");
    let after = svc.get(&scope, &id).await.expect("get ok");
    assert_eq!(after.display(), &next);
    assert!(after.display().description.is_none());
}

#[tokio::test]
async fn empty_display_is_the_default_on_get() {
    let svc = in_memory_service();
    let scope = TenantScope::new("org1", "ws1");

    svc.create(
        &scope,
        "bearer_token",
        json!({ "token": "sk-nodisp" }),
        CredentialDisplay::default(),
    )
    .await
    .expect("create ok");

    let id = svc.list(&scope).await.expect("list")[0].clone();
    let got = svc.get(&scope, &id).await.expect("get ok");
    assert!(got.display().is_empty());
}
