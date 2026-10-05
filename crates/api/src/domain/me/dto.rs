//! Server compatibility imports and transport-to-domain mappings.

pub use nebula_api_contract::v1::me::*;

#[cfg(test)]
mod tests {
    use super::{CreateTokenResponse, TokenSummary};

    static_assertions::assert_not_impl_any!(CreateTokenResponse: Clone);

    #[test]
    fn create_token_response_debug_redacts_plaintext_pat() {
        const CANARY: &str = "pat_PAT_AUTHORITY_CANARY-0b7f";
        let response = CreateTokenResponse {
            token: CANARY.to_owned(),
            summary: TokenSummary {
                id: "pat_metadata_id".to_owned(),
                name: "CI token".to_owned(),
                scopes: vec!["full_access".to_owned()],
                created_at: "2026-07-21T00:00:00Z".to_owned(),
                last_used_at: None,
                expires_at: None,
            },
        };

        assert!(!format!("{response:?}").contains(CANARY));
        let wire = serde_json::to_value(&response).expect("PAT response serializes");
        assert_eq!(wire["token"], CANARY);
    }
}
