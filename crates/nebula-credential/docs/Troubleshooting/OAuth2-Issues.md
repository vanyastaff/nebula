---
title: OAuth2 and Authentication Protocol Issues
description: Troubleshooting guide for OAuth2, SAML, JWT, LDAP, and other authentication protocol errors
tags: [troubleshooting, oauth2, saml, jwt, ldap, authentication, protocols]
related:
  - "[[Common-Errors]]"
  - "[[../Examples/OAuth2-Flow]]"
  - "[[../Examples/OAuth2-GitHub]]"
  - "[[../Examples/SAML-Authentication]]"
  - "[[../Examples/JWT-Validation]]"
  - "[[../Examples/LDAP-Authentication]]"
  - "[[Debugging-Checklist]]"
status: published
version: 1.0.0
---

# OAuth2 and Authentication Protocol Issues

Comprehensive troubleshooting for OAuth2, SAML, JWT, LDAP, and other authentication protocol errors in `nebula-credential`.

---

## Quick Reference

| Protocol | Common Error | Quick Fix | Detailed Section |
|----------|--------------|-----------|------------------|
| OAuth2 | `invalid_grant` | Restart auth flow | [§1.1](#11-invalid_grant) |
| OAuth2 | `State mismatch` | Check state storage | [§1.4](#14-statemismatch) |
| SAML | `SignatureVerificationFailed` | Verify IdP cert | [§2.1](#21-signatureverificationfailed) |
| JWT | `TokenValidationFailed` | Check signing key | [§3.1](#31-tokenvalidationfailed) |
| LDAP | `BindFailed` | Verify DN template | [§4.1](#41-bindfailed) |
| Kerberos | `CredentialAcquisitionFailed` | Check KDC connectivity | [§5.1](#51-credentialacquisitionfailed) |

---

## 1. OAuth2 Errors

### 1.1 `invalid_grant`

**Error Response**:
```json
{
  "error": "invalid_grant",
  "error_description": "Authorization code expired",
  "error_uri": "https://oauth.provider.com/docs/errors#invalid_grant"
}
```

**Common Causes**:
1. Authorization code expired (typically 10 minutes)
2. Authorization code already used
3. Authorization code revoked
4. Code verifier mismatch (PKCE)

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

match oauth2_credential.exchange_code(&code, &state).await {
    Err(CredentialError::OAuth2(OAuth2Error::TokenExchangeFailed(error_response))) => {
        eprintln!("OAuth2 Error: {}", error_response.error);
        eprintln!("Description: {}",
            error_response.error_description.as_deref().unwrap_or("N/A"));
        
        if error_response.error == "invalid_grant" {
            if let Some(desc) = &error_response.error_description {
                if desc.contains("expired") {
                    eprintln!("⚠️  Authorization code expired");
                    eprintln!("   Time since code received: {:?}", elapsed_time);
                    eprintln!("   Typical expiry: 10 minutes");
                } else if desc.contains("used") || desc.contains("redeemed") {
                    eprintln!("⚠️  Code already used - possible replay attack or duplicate request");
                } else if desc.contains("verifier") || desc.contains("PKCE") {
                    eprintln!("⚠️  PKCE code_verifier mismatch");
                    eprintln!("   Verify code_verifier matches code_challenge");
                }
            }
        }
    }
    Ok(tokens) => println!("Token exchange successful"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

**Solutions**:

1. **Expired code**: Restart OAuth2 flow

```rust
use nebula_credential::prelude::*;

pub async fn retry_oauth2_flow(
    config: OAuth2Config,
    ctx: &CredentialContext,
) -> Result<OAuth2Credential, CredentialError> {
    eprintln!("Restarting OAuth2 flow...");
    
    // Initialize new flow
    let flow = OAuth2Flow::new(config.clone());
    
    // Generate authorization URL with PKCE
    let (auth_url, state, pkce) = if config.pkce_enabled {
        let flow_with_pkce = flow.with_pkce();
        let (url, state, pkce) = flow_with_pkce.generate_authorization_url();
        (url, state, Some(pkce))
    } else {
        let (url, state) = flow.generate_authorization_url();
        (url, state, None)
    };
    
    eprintln!("Authorization URL: {auth_url}");
    eprintln!("State: {state}");
    
    // User navigates to URL, authorizes, redirected back with code
    // ... (implementation-specific)
    
    // Exchange code (with fresh code, within expiry window)
    let tokens = exchange_code(&code, &state, pkce.as_ref(), &config).await?;
    
    Ok(tokens.to_credential(config))
}
```

2. **Code reuse**: Ensure single-use code handling

```rust
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct CodeTracker {
    used_codes: Arc<Mutex<HashSet<String>>>,
}

impl CodeTracker {
    pub async fn mark_used(&self, code: &str) -> Result<(), OAuth2Error> {
        let mut used = self.used_codes.lock().await;
        
        if used.contains(code) {
            return Err(OAuth2Error::TokenExchangeFailed(OAuth2ErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Code already used".to_string()),
                error_uri: None,
            }));
        }
        
        used.insert(code.to_string());
        Ok(())
    }
}
```

3. **PKCE mismatch**: Verify code_verifier persistence

```rust
use nebula_credential::prelude::*;

// Store PKCE verifier during authorization
pub async fn start_oauth2_with_pkce(
    config: OAuth2Config,
    state_store: &mut StateStore,
) -> Result<String, OAuth2Error> {
    let pkce = PkceChallenge::generate();
    
    // Build authorization URL with code_challenge
    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method={}",
        config.authorization_endpoint,
        config.client_id.expose(),
        urlencoding::encode(&config.redirect_uri),
        config.scopes.join("%20"),
        state.expose(),
        pkce.challenge,
        pkce.method.as_str()
    );
    
    // Store verifier for later use
    state_store.save_pkce_verifier(&state, pkce.verifier).await?;
    
    Ok(auth_url)
}

// Retrieve verifier during token exchange
pub async fn exchange_code_with_pkce(
    code: &str,
    state: &str,
    state_store: &StateStore,
    config: &OAuth2Config,
) -> Result<OAuth2TokenResponse, OAuth2Error> {
    // Retrieve stored verifier
    let verifier = state_store.get_pkce_verifier(state).await?
        .ok_or_else(|| OAuth2Error::TokenExchangeFailed(OAuth2ErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("PKCE verifier not found".to_string()),
            error_uri: None,
        }))?;
    
    // Exchange code with verifier
    let response = reqwest::Client::new()
        .post(config.token_endpoint.clone())
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &config.redirect_uri),
            ("client_id", config.client_id.expose()),
            ("code_verifier", verifier.expose()),
        ])
        .send()
        .await
        .map_err(|e| OAuth2Error::NetworkError(e.to_string()))?;
    
    // ... handle response
}
```

---

### 1.2 `invalid_client`

**Error Response**:
```json
{
  "error": "invalid_client",
  "error_description": "Client authentication failed"
}
```

**Common Causes**:
- Wrong client_id
- Wrong client_secret
- Client credentials not in correct format (Basic Auth vs form parameters)
- Client not configured for this grant type

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let config = OAuth2Config {
    client_id: SecretString::new("my-client-id"),
    client_secret: Some(SecretString::new("my-client-secret")),
    // ...
};

eprintln!("Client ID: {}", config.client_id.expose());
eprintln!("Client Secret present: {}", config.client_secret.is_some());
eprintln!("Token endpoint: {}", config.token_endpoint);

// Test client credentials
match test_client_credentials(&config).await {
    Ok(_) => eprintln!("✓ Client credentials valid"),
    Err(e) => eprintln!("✗ Client credentials invalid: {e}"),
}
```

**Solutions**:

1. **Verify credentials**: Check OAuth2 app configuration in provider dashboard

2. **Correct authentication method**:

```rust
use nebula_credential::prelude::*;
use base64::Engine;

pub async fn exchange_code_basic_auth(
    code: &str,
    config: &OAuth2Config,
) -> Result<OAuth2TokenResponse, OAuth2Error> {
    let client = reqwest::Client::new();
    
    // Method 1: HTTP Basic Authentication (recommended)
    let credentials = format!(
        "{}:{}",
        config.client_id.expose(),
        config.client_secret.as_ref().unwrap().expose()
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
    
    let response = client
        .post(config.token_endpoint.clone())
        .header("Authorization", format!("Basic {encoded}"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &config.redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| OAuth2Error::NetworkError(e.to_string()))?;
    
    // ... parse response
}

pub async fn exchange_code_form_params(
    code: &str,
    config: &OAuth2Config,
) -> Result<OAuth2TokenResponse, OAuth2Error> {
    let client = reqwest::Client::new();
    
    // Method 2: Form parameters (some providers prefer this)
    let response = client
        .post(config.token_endpoint.clone())
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &config.redirect_uri),
            ("client_id", config.client_id.expose()),
            ("client_secret", config.client_secret.as_ref().unwrap().expose()),
        ])
        .send()
        .await
        .map_err(|e| OAuth2Error::NetworkError(e.to_string()))?;
    
    // ... parse response
}
```

---

### 1.3 `unauthorized_client`

**Error**: Client not authorized for this grant type

**Solutions**:

1. Update OAuth2 app configuration to enable Authorization Code grant
2. For public clients (mobile/SPA), use Authorization Code + PKCE
3. For confidential clients (server), use client_secret

---

### 1.4 `StateMismatch`

**Error**: `OAuth2Error::StateMismatch`

**Cause**: CSRF protection detected state parameter mismatch

**Common Reasons**:
- State not persisted correctly
- State expired
- Multiple concurrent flows
- CSRF attack attempt

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

pub async fn handle_oauth2_callback(
    code: &str,
    returned_state: &str,
    state_store: &StateStore,
) -> Result<OAuth2Credential, OAuth2Error> {
    // Retrieve stored state
    let stored_state = state_store.get_state(returned_state).await?;
    
    match stored_state {
        None => {
            eprintln!("⚠️  State not found in storage");
            eprintln!("   Returned state: {returned_state}");
            eprintln!("   Possible causes:");
            eprintln!("   - State expired");
            eprintln!("   - Storage cleared");
            eprintln!("   - Wrong state_store instance");
            return Err(OAuth2Error::StateMismatch);
        }
        Some(state) if state.expose() != returned_state => {
            eprintln!("⚠️  SECURITY: State mismatch detected!");
            eprintln!("   Expected: {}", state.expose());
            eprintln!("   Received: {returned_state}");
            eprintln!("   Possible CSRF attack - aborting");
            return Err(OAuth2Error::StateMismatch);
        }
        Some(_) => {
            eprintln!("✓ State validated successfully");
        }
    }
    
    // Proceed with token exchange
    // ...
}
```

**Solutions**:

1. **Use persistent storage** for state:

```rust
use nebula_credential::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc, Duration};

pub struct PersistentStateStore {
    states: Arc<RwLock<HashMap<String, StateEntry>>>,
}

struct StateEntry {
    state: SecretString,
    pkce_verifier: Option<SecretString>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl PersistentStateStore {
    pub fn new() -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    pub async fn save_state(
        &self,
        state: SecretString,
        pkce_verifier: Option<SecretString>,
    ) -> Result<(), OAuth2Error> {
        let mut states = self.states.write().await;
        
        let entry = StateEntry {
            state: state.clone(),
            pkce_verifier,
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(10),
        };
        
        states.insert(state.expose().to_string(), entry);
        
        Ok(())
    }
    
    pub async fn get_state(&self, state: &str) -> Result<Option<SecretString>, OAuth2Error> {
        let states = self.states.read().await;
        
        if let Some(entry) = states.get(state) {
            // Check expiration
            if Utc::now() > entry.expires_at {
                eprintln!("State expired");
                return Ok(None);
            }
            
            Ok(Some(entry.state.clone()))
        } else {
            Ok(None)
        }
    }
    
    pub async fn cleanup_expired(&self) {
        let mut states = self.states.write().await;
        let now = Utc::now();
        
        states.retain(|_, entry| now <= entry.expires_at);
    }
}
```

2. **Generate cryptographically secure state**:

```rust
use nebula_credential::prelude::*;
use rand::RngCore;

pub fn generate_secure_state() -> SecretString {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    
    let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    SecretString::new(state)
}
```

---

### 1.5 `RefreshFailed`

**Error**: Token refresh failed

**Common Causes**:
- Refresh token expired
- Refresh token revoked
- Refresh token single-use and already consumed
- Client credentials changed

**Solution**: Re-authenticate

```rust
use nebula_credential::prelude::*;

pub async fn refresh_or_reauthenticate(
    credential: &OAuth2Credential,
    config: &OAuth2Config,
    ctx: &CredentialContext,
) -> Result<OAuth2Credential, CredentialError> {
    // Attempt refresh
    match credential.refresh(&credential).await {
        Ok(refreshed) => {
            eprintln!("✓ Token refreshed successfully");
            Ok(refreshed)
        }
        Err(CredentialError::OAuth2(OAuth2Error::RefreshFailed(error))) => {
            eprintln!("✗ Refresh failed: {}", error.error);
            
            if error.error == "invalid_grant" {
                eprintln!("Refresh token expired or revoked, re-authentication required");
                
                // Restart OAuth2 flow
                retry_oauth2_flow(config.clone(), ctx).await
            } else {
                Err(CredentialError::OAuth2(OAuth2Error::RefreshFailed(error)))
            }
        }
        Err(e) => Err(e),
    }
}
```

---

## 2. SAML Errors

### 2.1 `SignatureVerificationFailed`

**Error**: `SamlError::SignatureVerificationFailed`

**Common Causes**:
- Wrong IdP certificate
- Certificate expired
- XML signature tampered
- Clock skew between SP and IdP

**Diagnosis**:

```rust
use nebula_credential::prelude::*;
use openssl::x509::X509;

let config = SamlConfig {
    idp_certificate: r#"-----BEGIN CERTIFICATE-----
MIIDXTCCAkWgAwIBAgIJAKZ...
-----END CERTIFICATE-----"#.to_string(),
    // ...
};

// Verify certificate validity
let cert = X509::from_pem(config.idp_certificate.as_bytes())
    .map_err(|e| SamlError::InvalidCertificate(e.to_string()))?;

let not_before = cert.not_before();
let not_after = cert.not_after();
let now = Utc::now();

eprintln!("IdP Certificate:");
eprintln!("  Subject: {}", cert.subject_name());
eprintln!("  Issuer: {}", cert.issuer_name());
eprintln!("  Valid from: {not_before}");
eprintln!("  Valid until: {not_after}");

if now < not_before {
    eprintln!("⚠️  Certificate not yet valid");
} else if now > not_after {
    eprintln!("⚠️  Certificate expired");
} else {
    eprintln!("✓ Certificate valid");
}
```

**Solutions**:

1. **Update IdP certificate** from metadata:

```rust
use nebula_credential::prelude::*;

pub async fn fetch_idp_metadata(
    metadata_url: &str,
) -> Result<SamlConfig, SamlError> {
    // Fetch IdP metadata XML
    let metadata_xml = reqwest::get(metadata_url)
        .await
        .map_err(|e| SamlError::XmlParseError(e.to_string()))?
        .text()
        .await
        .map_err(|e| SamlError::XmlParseError(e.to_string()))?;
    
    // Parse metadata (simplified)
    let doc = roxmltree::Document::parse(&metadata_xml)
        .map_err(|e| SamlError::XmlParseError(e.to_string()))?;
    
    // Extract certificate
    let cert_node = doc.descendants()
        .find(|n| n.tag_name().name() == "X509Certificate")
        .ok_or_else(|| SamlError::InvalidCertificate("No certificate in metadata".to_string()))?;
    
    let cert_b64 = cert_node.text()
        .ok_or_else(|| SamlError::InvalidCertificate("Empty certificate".to_string()))?;
    
    let cert_pem = format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----",
        cert_b64
    );
    
    // Extract SSO URL
    let sso_url = doc.descendants()
        .find(|n| {
            n.tag_name().name() == "SingleSignOnService" &&
            n.attribute("Binding") == Some("urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST")
        })
        .and_then(|n| n.attribute("Location"))
        .ok_or_else(|| SamlError::XmlParseError("No SSO URL in metadata".to_string()))?;
    
    Ok(SamlConfig {
        idp_sso_url: sso_url.to_string(),
        idp_certificate: cert_pem,
        // ... other fields
    })
}
```

2. **Allow clock skew**:

```rust
use nebula_credential::prelude::*;

pub fn verify_saml_assertion_with_skew(
    assertion: &SamlAssertion,
    clock_skew_seconds: i64,
) -> Result<(), SamlError> {
    let now = Utc::now();
    let skew = chrono::Duration::seconds(clock_skew_seconds);
    
    // Check NotBefore with skew
    if let Some(not_before) = assertion.not_before {
        if now + skew < not_before {
            return Err(SamlError::VerificationFailed(
                format!("Assertion not yet valid (now: {now}, not_before: {not_before})")
            ));
        }
    }
    
    // Check NotOnOrAfter with skew
    if let Some(not_on_or_after) = assertion.not_on_or_after {
        if now - skew >= not_on_or_after {
            return Err(SamlError::VerificationFailed(
                format!("Assertion expired (now: {now}, not_on_or_after: {not_on_or_after})")
            ));
        }
    }
    
    Ok(())
}
```

---

### 2.2 `MissingAssertion`

**Error**: `SamlError::MissingAssertion`

**Cause**: SAML response does not contain assertion

**Solution**: Check SAML response status:

```rust
use nebula_credential::prelude::*;

pub fn parse_saml_response(
    response_xml: &str,
) -> Result<SamlAssertion, SamlError> {
    let doc = roxmltree::Document::parse(response_xml)
        .map_err(|e| SamlError::XmlParseError(e.to_string()))?;
    
    // Check for Status
    if let Some(status_code) = doc.descendants()
        .find(|n| n.tag_name().name() == "StatusCode")
        .and_then(|n| n.attribute("Value"))
    {
        if status_code != "urn:oasis:names:tc:SAML:2.0:status:Success" {
            eprintln!("⚠️  SAML authentication failed");
            eprintln!("   Status: {status_code}");
            
            // Check for StatusMessage
            if let Some(msg) = doc.descendants()
                .find(|n| n.tag_name().name() == "StatusMessage")
                .and_then(|n| n.text())
            {
                eprintln!("   Message: {msg}");
            }
            
            return Err(SamlError::VerificationFailed(status_code.to_string()));
        }
    }
    
    // Extract assertion
    // ...
}
```

---

## 3. JWT Errors

### 3.1 `TokenValidationFailed`

**Error**: `JwtError::TokenValidationFailed`

**Common Causes**:
- Wrong signing key
- Token expired
- Wrong algorithm
- Invalid issuer/audience

**Diagnosis**:

```rust
use nebula_credential::prelude::*;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation, Algorithm};

pub fn diagnose_jwt(token: &str, public_key_pem: &str) -> Result<(), JwtError> {
    // Decode header without verification
    let header = decode_header(token)
        .map_err(|e| JwtError::DecodeFailed(e.to_string()))?;
    
    eprintln!("JWT Header:");
    eprintln!("  Algorithm: {:?}", header.alg);
    eprintln!("  Type: {:?}", header.typ);
    eprintln!("  Key ID: {:?}", header.kid);
    
    // Decode payload without verification
    let unverified = jsonwebtoken::dangerous_insecure_decode::<Claims>(token)
        .map_err(|e| JwtError::DecodeFailed(e.to_string()))?;
    
    eprintln!("\nJWT Claims:");
    eprintln!("  Subject: {}", unverified.claims.sub);
    eprintln!("  Issuer: {:?}", unverified.claims.iss);
    eprintln!("  Audience: {:?}", unverified.claims.aud);
    eprintln!("  Issued At: {}", unverified.claims.iat);
    eprintln!("  Expires: {}", unverified.claims.exp);
    
    let now = Utc::now().timestamp() as u64;
    if unverified.claims.exp < now {
        eprintln!("  ⚠️  Token expired {} seconds ago", now - unverified.claims.exp);
    }
    
    // Verify signature
    let decoding_key = DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
        .map_err(|e| JwtError::InvalidPublicKey(e.to_string()))?;
    
    let mut validation = Validation::new(header.alg);
    validation.validate_exp = false; // Check manually above
    
    match decode::<Claims>(token, &decoding_key, &validation) {
        Ok(_) => eprintln!("\n✓ Signature valid"),
        Err(e) => eprintln!("\n✗ Signature invalid: {e}"),
    }
    
    Ok(())
}
```

**Solutions**:

1. **Correct signing key**:

```rust
use nebula_credential::prelude::*;
use jsonwebtoken::{decode, DecodingKey, Validation};

pub fn validate_jwt_with_jwks(
    token: &str,
    jwks_url: &str,
) -> Result<Claims, JwtError> {
    // Decode header to get kid
    let header = decode_header(token)
        .map_err(|e| JwtError::DecodeFailed(e.to_string()))?;
    
    let kid = header.kid.ok_or_else(|| {
        JwtError::DecodeFailed("No kid in JWT header".to_string())
    })?;
    
    // Fetch JWKS
    let jwks: serde_json::Value = reqwest::blocking::get(jwks_url)
        .map_err(|e| JwtError::TokenValidationFailed(e.to_string()))?
        .json()
        .map_err(|e| JwtError::TokenValidationFailed(e.to_string()))?;
    
    // Find matching key
    let key = jwks["keys"]
        .as_array()
        .ok_or_else(|| JwtError::InvalidPublicKey("No keys in JWKS".to_string()))?
        .iter()
        .find(|k| k["kid"] == kid)
        .ok_or_else(|| JwtError::InvalidPublicKey(format!("Key {kid} not found")))?;
    
    // Convert to DecodingKey (simplified for RSA)
    let n = key["n"].as_str().ok_or_else(|| JwtError::InvalidPublicKey("No n".to_string()))?;
    let e = key["e"].as_str().ok_or_else(|| JwtError::InvalidPublicKey("No e".to_string()))?;
    
    let decoding_key = DecodingKey::from_rsa_components(n, e)
        .map_err(|e| JwtError::InvalidPublicKey(e.to_string()))?;
    
    // Validate
    let validation = Validation::new(header.alg);
    let token_data = decode::<Claims>(token, &decoding_key, &validation)
        .map_err(|e| JwtError::TokenValidationFailed(e.to_string()))?;
    
    Ok(token_data.claims)
}
```

2. **Handle expired tokens**:

```rust
use nebula_credential::prelude::*;

pub async fn get_valid_jwt(
    credential: &JwtCredential,
) -> Result<String, JwtError> {
    // Check if current token is expired
    if credential.is_expired() {
        eprintln!("JWT expired, refreshing...");
        
        // Refresh if refresh token available
        if let Some(refresh_token) = &credential.refresh_token {
            let refreshed = credential.refresh_token_flow(refresh_token).await?;
            Ok(refreshed.access_token.expose().to_string())
        } else {
            Err(JwtError::RefreshTokenExpired)
        }
    } else {
        Ok(credential.access_token.expose().to_string())
    }
}
```

---

## 4. LDAP Errors

### 4.1 `BindFailed`

**Error**: `LdapError::BindFailed`

**Common Causes**:
- Wrong DN format
- Wrong password
- User not found
- LDAP server unreachable

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let config = LdapConfig {
    url: "ldaps://ldap.example.com".to_string(),
    base_dn: "dc=example,dc=com".to_string(),
    bind_dn_template: Some("cn={username},ou=users,dc=example,dc=com".to_string()),
    domain: None,
    use_tls: true,
    ca_cert: None,
    timeout: Duration::from_secs(10),
};

let username = "alice";
let bind_dn = config.bind_dn_template
    .as_ref()
    .unwrap()
    .replace("{username}", username);

eprintln!("LDAP Configuration:");
eprintln!("  URL: {}", config.url);
eprintln!("  Base DN: {}", config.base_dn);
eprintln!("  Bind DN: {bind_dn}");
eprintln!("  TLS: {}", config.use_tls);

// Test connection
match test_ldap_connection(&config).await {
    Ok(_) => eprintln!("✓ LDAP server reachable"),
    Err(e) => eprintln!("✗ LDAP server unreachable: {e}"),
}
```

**Solutions**:

1. **Correct DN template**:

```rust
use nebula_credential::prelude::*;

pub async fn ldap_bind_with_search(
    config: &LdapConfig,
    username: &str,
    password: &SecretString,
) -> Result<LdapUserInfo, LdapError> {
    // Option 1: Use bind DN template
    if let Some(template) = &config.bind_dn_template {
        let bind_dn = template.replace("{username}", username);
        
        match ldap_bind(&config.url, &bind_dn, password).await {
            Ok(conn) => return ldap_get_user_info(conn, &bind_dn).await,
            Err(e) => eprintln!("Bind with template failed: {e}"),
        }
    }
    
    // Option 2: Search for user first
    let search_dn = format!("cn={username},{}", config.base_dn);
    
    match ldap_bind(&config.url, &search_dn, password).await {
        Ok(conn) => ldap_get_user_info(conn, &search_dn).await,
        Err(e) => Err(LdapError::BindFailed(e.to_string())),
    }
}
```

2. **Active Directory UPN format**:

```rust
use nebula_credential::prelude::*;

pub async fn ldap_bind_upn(
    config: &LdapConfig,
    username: &str,
    password: &SecretString,
) -> Result<LdapUserInfo, LdapError> {
    // Active Directory accepts username@domain format
    let upn = if let Some(domain) = &config.domain {
        format!("{}@{}", username, domain)
    } else {
        username.to_string()
    };
    
    eprintln!("Binding as UPN: {upn}");
    
    ldap_bind(&config.url, &upn, password).await
}
```

---

### 4.2 `TlsNegotiationFailed`

**Error**: `LdapError::TlsNegotiationFailed`

**Cause**: LDAPS connection failed

**Solutions**:

1. **Provide CA certificate**:

```rust
use nebula_credential::prelude::*;

let config = LdapConfig {
    url: "ldaps://ldap.example.com".to_string(),
    use_tls: true,
    ca_cert: Some(r#"-----BEGIN CERTIFICATE-----
MIIDrzCCApegAwIBAgIQCDvgVpBCRrGhdWrJWZHHSjANBgkqhkiG9w0BAQUFADBh
...
-----END CERTIFICATE-----"#.to_string()),
    // ...
};
```

2. **Use STARTTLS instead of LDAPS**:

```rust
// ldap://  (port 389) + STARTTLS
let config = LdapConfig {
    url: "ldap://ldap.example.com".to_string(),
    use_tls: true, // Enable STARTTLS
    // ...
};
```

---

## 5. Kerberos Errors

### 5.1 `CredentialAcquisitionFailed`

**Error**: `KerberosError::CredentialAcquisitionFailed`

**Common Causes**:
- KDC unreachable
- Wrong principal name
- Password expired
- Clock skew

**Solutions**:

1. **Verify KDC connectivity**:

```bash
# Test KDC reachability
nc -zv kdc.example.com 88

# Check Kerberos configuration
cat /etc/krb5.conf
```

2. **Synchronize clocks**:

```bash
# Kerberos requires clock synchronization (< 5 minutes skew)
ntpdate pool.ntp.org
```

3. **Test kinit**:

```bash
# Acquire TGT manually
kinit user@EXAMPLE.COM
klist
```

---

## Related Documentation

- [[Common-Errors]] - All error types catalog
- [[../Examples/OAuth2-Flow]] - OAuth2 implementation guide
- [[../Examples/SAML-Authentication]] - SAML implementation
- [[../Examples/JWT-Validation]] - JWT validation guide
- [[../Examples/LDAP-Authentication]] - LDAP integration
- [[Debugging-Checklist]] - Systematic debugging

---

## Summary

This guide covers troubleshooting for:

✅ **OAuth2 errors** - Token exchange, refresh, state management  
✅ **SAML errors** - Signature verification, assertions  
✅ **JWT errors** - Token validation, expiration  
✅ **LDAP errors** - Bind failures, TLS issues  
✅ **Kerberos errors** - Ticket acquisition  

Each protocol section includes diagnosis code, solutions, and prevention strategies.
