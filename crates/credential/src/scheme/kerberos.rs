//! Kerberos authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use nebula_core::SecretString;

/// Kerberos authentication material (service ticket).
///
/// Produced by: Kerberos/GSSAPI credential flows.
/// Consumed by: Kerberos-protected services (HDFS, MSSQL, AD).
#[derive(Clone, Serialize, Deserialize)]
pub struct KerberosAuth {
    /// Kerberos principal (e.g., `"user@REALM.COM"`).
    pub principal: String,
    /// Kerberos realm (e.g., `"REALM.COM"`).
    pub realm: String,
    /// The service ticket (secret).
    #[serde(with = "nebula_core::serde_secret")]
    service_ticket: SecretString,
    /// When the ticket expires.
    pub expires_at_time: chrono::DateTime<chrono::Utc>,
}

impl KerberosAuth {
    /// Creates a new Kerberos auth with a service ticket.
    pub fn new(
        principal: impl Into<String>,
        realm: impl Into<String>,
        service_ticket: SecretString,
        expires_at_time: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            principal: principal.into(),
            realm: realm.into(),
            service_ticket,
            expires_at_time,
        }
    }

    /// Returns the service ticket secret.
    pub fn service_ticket(&self) -> &SecretString {
        &self.service_ticket
    }
}

impl AuthScheme for KerberosAuth {
    const KIND: &'static str = "kerberos";

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        Some(self.expires_at_time)
    }
}

impl std::fmt::Debug for KerberosAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KerberosAuth")
            .field("principal", &self.principal)
            .field("realm", &self.realm)
            .field("service_ticket", &"[REDACTED]")
            .field("expires_at_time", &self.expires_at_time)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(KerberosAuth::KIND, "kerberos");
    }

    #[test]
    fn debug_redacts_service_ticket() {
        let auth = KerberosAuth::new(
            "user@EXAMPLE.COM",
            "EXAMPLE.COM",
            SecretString::new("ticket-data-base64"),
            chrono::Utc::now() + chrono::Duration::hours(8),
        );
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("ticket-data-base64"));
        assert!(debug.contains("EXAMPLE.COM"));
    }

    #[test]
    fn expires_at_returns_ticket_expiry() {
        let expiry = chrono::Utc::now() + chrono::Duration::hours(8);
        let auth = KerberosAuth::new(
            "user@EXAMPLE.COM",
            "EXAMPLE.COM",
            SecretString::new("ticket"),
            expiry,
        );
        assert_eq!(auth.expires_at(), Some(expiry));
    }
}
