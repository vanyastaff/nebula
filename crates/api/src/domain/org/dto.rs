//! Server compatibility imports and transport-to-domain mappings.

pub use nebula_api_contract::v1::org::*;

#[cfg(test)]
mod tests {
    use super::{CreateServiceAccountResponse, ServiceAccountSummary};

    static_assertions::assert_not_impl_any!(CreateServiceAccountResponse: Clone);

    #[test]
    fn create_service_account_response_debug_redacts_bearer_key() {
        const CANARY: &str = "nbl_sa_SERVICE_ACCOUNT_AUTHORITY_CANARY";
        let response = CreateServiceAccountResponse {
            account: ServiceAccountSummary {
                id: "svc_test".to_owned(),
                name: "automation".to_owned(),
                scopes: vec!["workflows:run".to_owned()],
                created_at: "2026-07-21T00:00:00Z".to_owned(),
            },
            key: CANARY.to_owned(),
        };

        let debug = format!("{response:?}");
        assert!(debug.contains("CreateServiceAccountResponse"));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(CANARY));
    }
}
