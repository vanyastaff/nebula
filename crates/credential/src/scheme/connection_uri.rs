//! Compound connection URI authentication (postgres://, redis://, etc.).

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{AuthScheme, SecretString};

/// Database / message-broker connection URI, structured.
///
/// Covers database connection strings (`postgres://user:pass@host/db`),
/// cache URIs (`redis://:token@host`), message broker URIs, and any other
/// service where a structured URI is the complete authentication material.
///
/// # Security model (§15.5 §3295 — closes security-lead N4)
///
/// Individual fields exposed via non-secret accessors where they ARE
/// non-secret (host, port, database, username); password remains
/// `SecretString`. The full URL reconstruction returns `SecretString`
/// so logging or serialization paths cannot leak the password component
/// even when the caller forgets redaction.
///
/// Driver injection sites call `.expose_secret()` on the result of
/// [`as_url()`](Self::as_url) exactly once, at the FFI boundary.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::ConnectionUri};
///
/// let uri = ConnectionUri::new(
///     "postgres".into(),
///     "db.example.com".into(),
///     Some(5432),
///     "mydb".into(),
///     "alice".into(),
///     SecretString::new("hunter2"),
/// );
/// assert_eq!(uri.host(), "db.example.com");
/// // Full URL is wrapped in SecretString — leaking it requires expose_secret()
/// let _full = uri.as_url();
/// ```
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop, AuthScheme)]
#[auth_scheme(pattern = ConnectionUri, sensitive)]
pub struct ConnectionUri {
    #[zeroize(skip)]
    scheme: String,
    #[zeroize(skip)]
    host: String,
    #[zeroize(skip)]
    port: Option<u16>,
    #[zeroize(skip)]
    database: String,
    #[zeroize(skip)]
    username: String,
    #[serde(with = "crate::serde_secret")]
    password: SecretString,
}

impl ConnectionUri {
    /// Constructs a structured connection URI.
    ///
    /// Per §15.5 §3295: host / port / database / username are non-secret
    /// (safe to log / display); password is `SecretString` so it never
    /// touches a `Display` / `Debug` / serializer path without redaction.
    #[must_use]
    pub fn new(
        scheme: String,
        host: String,
        port: Option<u16>,
        database: String,
        username: String,
        password: SecretString,
    ) -> Self {
        Self {
            scheme,
            host,
            port,
            database,
            username,
            password,
        }
    }

    /// URI scheme (e.g. `"postgres"`, `"redis"`, `"mongodb"`).
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Host name — safe to log.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Port number, if specified.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Database / namespace identifier — safe to log.
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Username — safe to log (treated as identity, not credential).
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Password as a `SecretString` — caller must `expose_secret()` only at
    /// the FFI boundary, never for logging or serialization.
    pub fn password(&self) -> &SecretString {
        &self.password
    }

    /// Reconstructs the full URL inside `SecretString`.
    ///
    /// Driver injection sites call `.expose_secret()` on the result exactly
    /// once, at the FFI boundary. The wrapper guarantees the URL is never
    /// written to logs / debug output / serializers without an explicit
    /// `expose_secret()` call.
    ///
    /// The `username` and `password` components are percent-encoded per
    /// RFC 3986 §3.2.1 so passwords containing reserved characters
    /// (`@ : / ? #`) cannot break out of the userinfo segment and leak
    /// into hostname or path parsing errors.
    #[must_use]
    pub fn as_url(&self) -> SecretString {
        use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
        let user_enc = utf8_percent_encode(&self.username, NON_ALPHANUMERIC);
        let pass_enc = utf8_percent_encode(self.password.expose_secret(), NON_ALPHANUMERIC);
        let port_part = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        let url = format!(
            "{}://{}:{}@{}{}/{}",
            self.scheme, user_enc, pass_enc, self.host, port_part, self.database,
        );
        SecretString::new(url)
    }
}

impl std::fmt::Debug for ConnectionUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionUri")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthPattern;

    fn sample() -> ConnectionUri {
        ConnectionUri::new(
            "postgres".into(),
            "db.example.com".into(),
            Some(5432),
            "mydb".into(),
            "alice".into(),
            SecretString::new("hunter2"),
        )
    }

    #[test]
    fn pattern_is_connection_uri() {
        assert_eq!(ConnectionUri::pattern(), AuthPattern::ConnectionUri);
    }

    #[test]
    fn debug_redacts_password_only() {
        let uri = sample();
        let debug = format!("{uri:?}");
        assert!(debug.contains("postgres"));
        assert!(debug.contains("db.example.com"));
        assert!(debug.contains("5432"));
        assert!(debug.contains("mydb"));
        assert!(debug.contains("alice"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("hunter2"));
    }

    #[test]
    fn accessors_return_non_secret_fields_directly() {
        let uri = sample();
        assert_eq!(uri.scheme(), "postgres");
        assert_eq!(uri.host(), "db.example.com");
        assert_eq!(uri.port(), Some(5432));
        assert_eq!(uri.database(), "mydb");
        assert_eq!(uri.username(), "alice");
        assert_eq!(uri.password().expose_secret(), "hunter2");
    }

    #[test]
    fn as_url_wraps_full_url_in_secret_string() {
        let uri = sample();
        let full = uri.as_url();
        assert_eq!(
            full.expose_secret(),
            "postgres://alice:hunter2@db.example.com:5432/mydb"
        );
    }

    #[test]
    fn as_url_omits_port_when_absent() {
        let uri = ConnectionUri::new(
            "redis".into(),
            "cache.example.com".into(),
            None,
            "0".into(),
            "default".into(),
            SecretString::new("token123"),
        );
        let full = uri.as_url();
        assert_eq!(
            full.expose_secret(),
            "redis://default:token123@cache.example.com/0"
        );
    }

    #[test]
    fn as_url_percent_encodes_password_special_chars() {
        // Real-world passwords contain `@`, `:`, `/`, `?`, `#`.
        // RFC 3986 §3.2.1 requires percent-encoding for userinfo.
        // Without encoding, password `p@ss:w/d` would produce
        // `u:p@ss:w/d@host` — drivers parse this as host `@host`
        // with the password leaking into hostname error messages.
        let uri = ConnectionUri::new(
            "postgres".into(),
            "host".into(),
            None,
            "db".into(),
            "u".into(),
            SecretString::new("p@ss:w/d"),
        );
        assert_eq!(
            uri.as_url().expose_secret(),
            "postgres://u:p%40ss%3Aw%2Fd@host/db"
        );
    }

    #[test]
    fn as_url_percent_encodes_username_special_chars() {
        // Username also lives in userinfo and must be encoded.
        let uri = ConnectionUri::new(
            "postgres".into(),
            "host".into(),
            None,
            "db".into(),
            "user@corp".into(),
            SecretString::new("plain"),
        );
        assert_eq!(
            uri.as_url().expose_secret(),
            "postgres://user%40corp:plain@host/db"
        );
    }
}
