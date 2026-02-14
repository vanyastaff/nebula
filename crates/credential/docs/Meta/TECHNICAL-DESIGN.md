# Technical Design Document: nebula-credential

**Version:** 1.0.0  
**Status:** Draft  
**Last Updated:** 2026-02-03  
**Authors:** Architecture Team  

## Document Purpose

This document provides low-level implementation details for the nebula-credential crate, including:
- Encryption algorithms and key derivation
- Protocol-specific implementation details (OAuth2, SAML, LDAP, mTLS, JWT, API Keys, Kerberos)
- Storage backend implementations
- Caching strategies
- Observability integration
- Performance optimizations
- Error handling patterns

This complements architecture.md by diving into the "how" rather than the "what".

---

## Table of Contents

1. [Cryptographic Implementation](#cryptographic-implementation)
2. [OAuth2 Implementation](#oauth2-implementation)
3. [SAML 2.0 Implementation](#saml-20-implementation)
4. [LDAP/Active Directory Implementation](#ldap-active-directory-implementation)
5. [mTLS Implementation](#mtls-implementation)
6. [JWT Implementation](#jwt-implementation)
7. [API Key Implementation](#api-key-implementation)
8. [Kerberos Implementation](#kerberos-implementation)
9. [Storage Provider Implementations](#storage-provider-implementations)
10. [Caching Strategy](#caching-strategy)
11. [Observability Implementation](#observability-implementation)
12. [Performance Optimizations](#performance-optimizations)
13. [Error Handling](#error-handling)
14. [Credential Testing & Validation](#credential-testing--validation)

---

## 1. Cryptographic Implementation

### 1.1 Encryption: AES-256-GCM

**Algorithm Choice Rationale:**
- AES-256-GCM provides authenticated encryption with associated data (AEAD)
- Detects tampering automatically via authentication tag
- NIST approved (FIPS 140-2)
- Hardware acceleration available on modern CPUs (AES-NI)

**Implementation Details:**

```rust
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{Argon2, PasswordHasher, SaltString};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Encryption context with key derivation
#[derive(ZeroizeOnDrop)]
pub struct EncryptionKey {
    key: [u8; 32], // 256 bits
}

impl EncryptionKey {
    /// Derive encryption key from master password using Argon2id
    pub fn derive_from_password(
        password: &str,
        salt: &[u8; 16],
    ) -> Result<Self, CryptoError> {
        let argon2 = Argon2::default();
        
        // Argon2id parameters (OWASP recommendations 2024)
        let config = argon2::ParamsBuilder::new()
            .m_cost(19456) // 19 MiB memory
            .t_cost(2)     // 2 iterations
            .p_cost(1)     // 1 thread
            .build()
            .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;
        
        let mut key = [0u8; 32];
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;
        
        Ok(Self { key })
    }
    
    /// Load key directly (from secure storage)
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { key: bytes }
    }
}

/// Nonce generator with collision prevention
pub struct NonceGenerator {
    counter: AtomicU64,
    random_prefix: [u8; 4],
}

impl NonceGenerator {
    pub fn new() -> Self {
        let mut random_prefix = [0u8; 4];
        OsRng.fill_bytes(&mut random_prefix);
        
        Self {
            counter: AtomicU64::new(0),
            random_prefix,
        }
    }
    
    /// Generate cryptographically unique nonce
    /// Format: [4 bytes random prefix | 8 bytes counter]
    pub fn generate(&self) -> [u8; 12] {
        let counter = self.counter.fetch_add(1, Ordering::SeqCst);
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.random_prefix);
        nonce[4..].copy_from_slice(&counter.to_le_bytes());
        nonce
    }
}

/// Encrypt plaintext with AES-256-GCM
pub fn encrypt(
    plaintext: &[u8],
    key: &EncryptionKey,
    nonce_gen: &NonceGenerator,
) -> Result<EncryptedData, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(&key.key)
        .map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    
    let nonce_bytes = nonce_gen.generate();
    let nonce = Nonce::from_slice(&nonce_bytes);
    
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
    
    Ok(EncryptedData {
        nonce: nonce_bytes,
        ciphertext,
        version: 1, // Algorithm version for future migration
    })
}

/// Decrypt ciphertext with AES-256-GCM
pub fn decrypt(
    encrypted: &EncryptedData,
    key: &EncryptionKey,
) -> Result<Vec<u8>, CryptoError> {
    if encrypted.version != 1 {
        return Err(CryptoError::UnsupportedVersion(encrypted.version));
    }
    
    let cipher = Aes256Gcm::new_from_slice(&key.key)
        .map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    
    let nonce = Nonce::from_slice(&encrypted.nonce);
    
    cipher
        .decrypt(nonce, encrypted.ciphertext.as_ref())
        .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
}

#[derive(Debug, Clone)]
pub struct EncryptedData {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
    pub version: u8,
}
```

**Key Security Properties:**
1. **Nonce Uniqueness:** Random prefix + monotonic counter prevents reuse
2. **Key Derivation:** Argon2id with 19 MiB memory cost resists GPU attacks
3. **Authentication:** GCM mode provides integrity verification
4. **Zeroization:** Keys automatically cleared from memory on drop

### 1.2 Key Rotation

```rust
pub struct KeyRotationManager {
    current_key: EncryptionKey,
    previous_keys: Vec<(KeyId, EncryptionKey)>,
    rotation_policy: RotationPolicy,
}

impl KeyRotationManager {
    /// Rotate to new key while maintaining ability to decrypt old data
    pub async fn rotate(&mut self) -> Result<KeyId, CryptoError> {
        // Generate new key
        let new_key = EncryptionKey::generate_secure()?;
        let new_key_id = KeyId::new();
        
        // Store previous key with ID
        let old_key_id = self.current_key.id();
        self.previous_keys.push((old_key_id, self.current_key));
        
        // Activate new key
        self.current_key = new_key;
        
        // Trim old keys based on policy
        self.trim_old_keys()?;
        
        Ok(new_key_id)
    }
    
    /// Decrypt with automatic key selection
    pub fn decrypt_auto(&self, encrypted: &EncryptedData) -> Result<Vec<u8>, CryptoError> {
        // Try current key first
        if let Ok(plaintext) = decrypt(encrypted, &self.current_key) {
            return Ok(plaintext);
        }
        
        // Try previous keys
        for (key_id, key) in &self.previous_keys {
            if let Ok(plaintext) = decrypt(encrypted, key) {
                return Ok(plaintext);
            }
        }
        
        Err(CryptoError::NoValidKey)
    }
}

pub struct RotationPolicy {
    pub max_age: Duration,           // Rotate after 90 days
    pub max_operations: u64,         // Rotate after 1M encryptions
    pub keep_previous_keys: usize,   // Keep last 3 keys for decryption
}
```

---

## 2. OAuth2 Implementation

### 2.1 Authorization Code Flow with PKCE

**PKCE (Proof Key for Code Exchange) Implementation:**

```rust
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

/// PKCE challenge generator
pub struct PkceChallenge {
    verifier: SecretString,
    challenge: String,
    method: PkceMethod,
}

#[derive(Debug, Clone, Copy)]
pub enum PkceMethod {
    S256, // SHA-256 (REQUIRED by OAuth 2.1)
    Plain, // Fallback for legacy servers
}

impl PkceChallenge {
    /// Generate cryptographically secure PKCE challenge
    pub fn generate() -> Self {
        // Generate 32-byte random verifier (RFC 7636 §4.1)
        let mut verifier_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut verifier_bytes);
        
        // Base64url encode (43-128 characters per RFC)
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
        
        // Generate challenge: BASE64URL(SHA256(verifier))
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge_bytes = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(challenge_bytes);
        
        Self {
            verifier: SecretString::new(verifier),
            challenge,
            method: PkceMethod::S256,
        }
    }
    
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
    
    pub fn method(&self) -> &str {
        match self.method {
            PkceMethod::S256 => "S256",
            PkceMethod::Plain => "plain",
        }
    }
    
    pub fn verifier(&self) -> &SecretString {
        &self.verifier
    }
}

/// OAuth2 Authorization Code Flow with PKCE
pub struct OAuth2AuthorizationCode {
    config: OAuth2Config,
    pkce: Option<PkceChallenge>,
    state: SecretString,
}

impl OAuth2AuthorizationCode {
    pub fn new(config: OAuth2Config) -> Self {
        // Generate PKCE challenge (mandatory for public clients)
        let pkce = Some(PkceChallenge::generate());
        
        // Generate state for CSRF protection
        let mut state_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut state_bytes);
        let state = SecretString::new(URL_SAFE_NO_PAD.encode(state_bytes));
        
        Self { config, pkce, state }
    }
    
    /// Step 1: Generate authorization URL
    pub fn authorization_url(&self) -> Result<Url, OAuth2Error> {
        let mut url = self.config.authorization_endpoint.clone();
        
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", self.config.client_id.expose()),
            ("redirect_uri", &self.config.redirect_uri),
            ("state", self.state.expose()),
        ];
        
        // Add PKCE challenge
        if let Some(pkce) = &self.pkce {
            params.push(("code_challenge", pkce.challenge()));
            params.push(("code_challenge_method", pkce.method()));
        }
        
        // Add scopes
        if !self.config.scopes.is_empty() {
            let scope_str = self.config.scopes.join(" ");
            params.push(("scope", &scope_str));
        }
        
        url.query_pairs_mut().extend_pairs(params);
        Ok(url)
    }
    
    /// Step 2: Exchange authorization code for tokens
    pub async fn exchange_code(
        &self,
        code: &str,
        received_state: &str,
    ) -> Result<TokenResponse, OAuth2Error> {
        // Verify state to prevent CSRF
        if received_state != self.state.expose() {
            return Err(OAuth2Error::StateMismatch);
        }
        
        let client = reqwest::Client::new();
        
        let mut params = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.config.redirect_uri),
            ("client_id", self.config.client_id.expose()),
        ];
        
        // Add client secret if confidential client
        if let Some(secret) = &self.config.client_secret {
            params.push(("client_secret", secret.expose()));
        }
        
        // Add PKCE verifier
        if let Some(pkce) = &self.pkce {
            params.push(("code_verifier", pkce.verifier().expose()));
        }
        
        let response = client
            .post(self.config.token_endpoint.clone())
            .form(&params)
            .send()
            .await
            .map_err(|e| OAuth2Error::NetworkError(e.to_string()))?;
        
        if !response.status().is_success() {
            let error: OAuth2ErrorResponse = response.json().await?;
            return Err(OAuth2Error::TokenExchangeFailed(error));
        }
        
        let token_response: TokenResponse = response.json().await?;
        Ok(token_response)
    }
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuth2ErrorResponse {
    pub error: String,
    pub error_description: Option<String>,
    pub error_uri: Option<String>,
}
```

### 2.2 Client Credentials Flow

```rust
/// OAuth2 Client Credentials Flow (machine-to-machine)
pub struct OAuth2ClientCredentials {
    config: OAuth2Config,
}

impl OAuth2ClientCredentials {
    pub async fn authenticate(&self) -> Result<TokenResponse, OAuth2Error> {
        let client = reqwest::Client::new();
        
        let params = vec![
            ("grant_type", "client_credentials"),
            ("client_id", self.config.client_id.expose()),
            ("client_secret", self.config.client_secret
                .as_ref()
                .ok_or(OAuth2Error::MissingClientSecret)?
                .expose()),
            ("scope", &self.config.scopes.join(" ")),
        ];
        
        let response = client
            .post(self.config.token_endpoint.clone())
            .form(&params)
            .send()
            .await
            .map_err(|e| OAuth2Error::NetworkError(e.to_string()))?;
        
        if !response.status().is_success() {
            let error: OAuth2ErrorResponse = response.json().await?;
            return Err(OAuth2Error::AuthenticationFailed(error));
        }
        
        Ok(response.json().await?)
    }
}
```

### 2.3 Token Refresh

```rust
/// Token refresh with exponential backoff
pub struct TokenRefresher {
    config: OAuth2Config,
    retry_policy: RetryPolicy,
}

impl TokenRefresher {
    pub async fn refresh(
        &self,
        refresh_token: &SecretString,
    ) -> Result<TokenResponse, OAuth2Error> {
        let mut attempt = 0;
        let mut backoff = Duration::from_millis(100);
        
        loop {
            match self.try_refresh(refresh_token).await {
                Ok(response) => return Ok(response),
                Err(e) if attempt < self.retry_policy.max_attempts => {
                    attempt += 1;
                    tokio::time::sleep(backoff).await;
                    backoff = backoff.mul_f32(self.retry_policy.backoff_multiplier);
                }
                Err(e) => return Err(e),
            }
        }
    }
    
    async fn try_refresh(
        &self,
        refresh_token: &SecretString,
    ) -> Result<TokenResponse, OAuth2Error> {
        let client = reqwest::Client::new();
        
        let params = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
            ("client_id", self.config.client_id.expose()),
        ];
        
        let response = client
            .post(self.config.token_endpoint.clone())
            .form(&params)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| OAuth2Error::NetworkError(e.to_string()))?;
        
        if !response.status().is_success() {
            let error: OAuth2ErrorResponse = response.json().await?;
            return Err(OAuth2Error::RefreshFailed(error));
        }
        
        Ok(response.json().await?)
    }
}

pub struct RetryPolicy {
    pub max_attempts: usize,
    pub backoff_multiplier: f32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_multiplier: 2.0,
        }
    }
}
```

### 2.4 Device Code Flow

```rust
/// OAuth2 Device Code Flow (for CLI/TV apps)
pub struct OAuth2DeviceCode {
    config: OAuth2Config,
}

impl OAuth2DeviceCode {
    /// Step 1: Request device code
    pub async fn request_device_code(&self) -> Result<DeviceCodeResponse, OAuth2Error> {
        let client = reqwest::Client::new();
        
        let params = vec![
            ("client_id", self.config.client_id.expose()),
            ("scope", &self.config.scopes.join(" ")),
        ];
        
        let response = client
            .post(self.config.device_authorization_endpoint.clone())
            .form(&params)
            .send()
            .await?;
        
        Ok(response.json().await?)
    }
    
    /// Step 2: Poll for authorization (with exponential backoff)
    pub async fn poll_for_token(
        &self,
        device_code: &SecretString,
        interval: Duration,
    ) -> Result<TokenResponse, OAuth2Error> {
        let client = reqwest::Client::new();
        let mut backoff = interval;
        
        loop {
            tokio::time::sleep(backoff).await;
            
            let params = vec![
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code.expose()),
                ("client_id", self.config.client_id.expose()),
            ];
            
            let response = client
                .post(self.config.token_endpoint.clone())
                .form(&params)
                .send()
                .await?;
            
            if response.status().is_success() {
                return Ok(response.json().await?);
            }
            
            let error: OAuth2ErrorResponse = response.json().await?;
            
            match error.error.as_str() {
                "authorization_pending" => {
                    // Continue polling
                    continue;
                }
                "slow_down" => {
                    // Increase polling interval by 5 seconds
                    backoff += Duration::from_secs(5);
                    continue;
                }
                "expired_token" => {
                    return Err(OAuth2Error::DeviceCodeExpired);
                }
                _ => {
                    return Err(OAuth2Error::DeviceAuthFailed(error));
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64, // Polling interval in seconds
}
```

---

## 3. SAML 2.0 Implementation

### 3.1 SAML Request Generation

```rust
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;
use flate2::write::DeflateEncoder;
use flate2::Compression;

/// SAML 2.0 Authentication Request generator
pub struct SamlRequestBuilder {
    config: SamlConfig,
}

impl SamlRequestBuilder {
    /// Generate SAML AuthnRequest XML
    pub fn build_authn_request(&self) -> Result<String, SamlError> {
        let request_id = self.generate_request_id();
        let issue_instant = Utc::now().to_rfc3339();
        
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        
        // XML declaration
        writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
        
        // <samlp:AuthnRequest>
        let mut authn_request = BytesStart::new("samlp:AuthnRequest");
        authn_request.push_attribute(("xmlns:samlp", "urn:oasis:names:tc:SAML:2.0:protocol"));
        authn_request.push_attribute(("xmlns:saml", "urn:oasis:names:tc:SAML:2.0:assertion"));
        authn_request.push_attribute(("ID", request_id.as_str()));
        authn_request.push_attribute(("Version", "2.0"));
        authn_request.push_attribute(("IssueInstant", issue_instant.as_str()));
        authn_request.push_attribute(("Destination", self.config.idp_sso_url.as_str()));
        authn_request.push_attribute(("AssertionConsumerServiceURL", self.config.acs_url.as_str()));
        authn_request.push_attribute(("ProtocolBinding", "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"));
        
        writer.write_event(Event::Start(authn_request))?;
        
        // <saml:Issuer>
        writer.write_event(Event::Start(BytesStart::new("saml:Issuer")))?;
        writer.write_event(Event::Text(BytesText::new(&self.config.entity_id)))?;
        writer.write_event(Event::End(BytesEnd::new("saml:Issuer")))?;
        
        // <samlp:NameIDPolicy>
        let mut name_id_policy = BytesStart::new("samlp:NameIDPolicy");
        name_id_policy.push_attribute(("Format", "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress"));
        name_id_policy.push_attribute(("AllowCreate", "true"));
        writer.write_event(Event::Empty(name_id_policy))?;
        
        // </samlp:AuthnRequest>
        writer.write_event(Event::End(BytesEnd::new("samlp:AuthnRequest")))?;
        
        let xml = String::from_utf8(writer.into_inner().into_inner())?;
        Ok(xml)
    }
    
    /// Encode and deflate SAML request for HTTP-Redirect binding
    pub fn encode_redirect_request(&self, xml: &str) -> Result<String, SamlError> {
        // Deflate compression
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(xml.as_bytes())?;
        let compressed = encoder.finish()?;
        
        // Base64 encode
        let encoded = URL_SAFE_NO_PAD.encode(compressed);
        Ok(encoded)
    }
    
    fn generate_request_id(&self) -> String {
        format!("_{}",Uuid::new_v4().to_string().replace("-", ""))
    }
}

pub struct SamlConfig {
    pub entity_id: String,              // SP entity ID
    pub acs_url: String,                // Assertion Consumer Service URL
    pub idp_sso_url: String,            // IdP Single Sign-On URL
    pub idp_certificate: String,        // IdP public certificate (PEM)
    pub sp_private_key: Option<String>, // SP private key for signing
}
```

### 3.2 SAML Response Validation

```rust
use openssl::x509::X509;
use openssl::pkey::PKey;
use openssl::sign::Verifier;
use openssl::hash::MessageDigest;

/// SAML 2.0 Response validator
pub struct SamlResponseValidator {
    config: SamlConfig,
}

impl SamlResponseValidator {
    /// Validate and parse SAML Response
    pub async fn validate_response(
        &self,
        saml_response: &str,
    ) -> Result<SamlAssertion, SamlError> {
        // Step 1: Base64 decode
        let decoded = base64::decode(saml_response)
            .map_err(|e| SamlError::InvalidEncoding(e.to_string()))?;
        
        let xml = String::from_utf8(decoded)
            .map_err(|e| SamlError::InvalidUtf8(e.to_string()))?;
        
        // Step 2: Parse XML
        let doc = roxmltree::Document::parse(&xml)
            .map_err(|e| SamlError::XmlParseError(e.to_string()))?;
        
        // Step 3: Validate signature
        self.validate_signature(&doc)?;
        
        // Step 4: Extract and validate assertion
        let assertion = self.extract_assertion(&doc)?;
        
        // Step 5: Validate conditions (timestamps, audience)
        self.validate_conditions(&assertion)?;
        
        Ok(assertion)
    }
    
    fn validate_signature(&self, doc: &roxmltree::Document) -> Result<(), SamlError> {
        // Find <ds:Signature> element
        let signature_node = doc
            .descendants()
            .find(|n| n.has_tag_name("Signature"))
            .ok_or(SamlError::MissingSignature)?;
        
        // Extract SignedInfo canonical form
        let signed_info = self.canonicalize_signed_info(&signature_node)?;
        
        // Extract signature value
        let signature_value = signature_node
            .descendants()
            .find(|n| n.has_tag_name("SignatureValue"))
            .and_then(|n| n.text())
            .ok_or(SamlError::MissingSignatureValue)?;
        
        let signature_bytes = base64::decode(signature_value)
            .map_err(|e| SamlError::InvalidSignature(e.to_string()))?;
        
        // Load IdP certificate
        let cert = X509::from_pem(self.config.idp_certificate.as_bytes())
            .map_err(|e| SamlError::InvalidCertificate(e.to_string()))?;
        
        let public_key = cert.public_key()
            .map_err(|e| SamlError::InvalidPublicKey(e.to_string()))?;
        
        // Verify signature
        let mut verifier = Verifier::new(MessageDigest::sha256(), &public_key)
            .map_err(|e| SamlError::VerificationFailed(e.to_string()))?;
        
        verifier.update(signed_info.as_bytes())
            .map_err(|e| SamlError::VerificationFailed(e.to_string()))?;
        
        let valid = verifier.verify(&signature_bytes)
            .map_err(|e| SamlError::VerificationFailed(e.to_string()))?;
        
        if !valid {
            return Err(SamlError::SignatureVerificationFailed);
        }
        
        Ok(())
    }
    
    fn extract_assertion(&self, doc: &roxmltree::Document) -> Result<SamlAssertion, SamlError> {
        let assertion_node = doc
            .descendants()
            .find(|n| n.has_tag_name("Assertion"))
            .ok_or(SamlError::MissingAssertion)?;
        
        // Extract Subject
        let subject = assertion_node
            .descendants()
            .find(|n| n.has_tag_name("NameID"))
            .and_then(|n| n.text())
            .ok_or(SamlError::MissingSubject)?
            .to_string();
        
        // Extract Attributes
        let mut attributes = HashMap::new();
        for attr_node in assertion_node.descendants().filter(|n| n.has_tag_name("Attribute")) {
            if let Some(name) = attr_node.attribute("Name") {
                let values: Vec<String> = attr_node
                    .descendants()
                    .filter(|n| n.has_tag_name("AttributeValue"))
                    .filter_map(|n| n.text())
                    .map(|s| s.to_string())
                    .collect();
                
                attributes.insert(name.to_string(), values);
            }
        }
        
        // Extract SessionIndex
        let session_index = assertion_node
            .descendants()
            .find(|n| n.has_tag_name("AuthnStatement"))
            .and_then(|n| n.attribute("SessionIndex"))
            .map(|s| s.to_string());
        
        Ok(SamlAssertion {
            subject,
            attributes,
            session_index,
        })
    }
    
    fn validate_conditions(&self, assertion: &SamlAssertion) -> Result<(), SamlError> {
        // Validate NotBefore and NotOnOrAfter timestamps
        // Validate Audience restriction
        // Implementation details...
        Ok(())
    }
    
    fn canonicalize_signed_info(&self, signature_node: &roxmltree::Node) -> Result<String, SamlError> {
        // Implement XML Canonicalization (C14N)
        // Required for signature verification
        // Implementation details...
        Ok(String::new())
    }
}

#[derive(Debug, Clone)]
pub struct SamlAssertion {
    pub subject: String,
    pub attributes: HashMap<String, Vec<String>>,
    pub session_index: Option<String>,
}
```

---

## 4. LDAP/Active Directory Implementation

### 4.1 LDAP Bind and Search

```rust
use ldap3::{LdapConn, LdapConnAsync, Scope, SearchEntry};

/// LDAP/Active Directory credential provider
pub struct LdapCredential {
    config: LdapConfig,
    connection_pool: LdapConnectionPool,
}

impl LdapCredential {
    /// Authenticate user with LDAP bind
    pub async fn authenticate(
        &self,
        username: &str,
        password: &SecretString,
    ) -> Result<LdapUserInfo, LdapError> {
        // Step 1: Connect to LDAP server
        let mut ldap = self.connection_pool.get().await?;
        
        // Step 2: Construct bind DN
        let bind_dn = self.construct_bind_dn(username)?;
        
        // Step 3: Attempt bind
        let bind_result = ldap
            .simple_bind(&bind_dn, password.expose())
            .await
            .map_err(|e| LdapError::BindFailed(e.to_string()))?;
        
        if !bind_result.success() {
            return Err(LdapError::AuthenticationFailed(
                bind_result.text().unwrap_or("Unknown error").to_string()
            ));
        }
        
        // Step 4: Search for user attributes
        let user_info = self.fetch_user_info(&mut ldap, username).await?;
        
        // Step 5: Return connection to pool
        self.connection_pool.return_connection(ldap).await;
        
        Ok(user_info)
    }
    
    fn construct_bind_dn(&self, username: &str) -> Result<String, LdapError> {
        match &self.config.bind_dn_template {
            Some(template) => Ok(template.replace("{username}", username)),
            None => {
                // Active Directory: use userPrincipalName
                if let Some(domain) = &self.config.domain {
                    Ok(format!("{}@{}", username, domain))
                } else {
                    Err(LdapError::MissingBindDnTemplate)
                }
            }
        }
    }
    
    async fn fetch_user_info(
        &self,
        ldap: &mut LdapConn,
        username: &str,
    ) -> Result<LdapUserInfo, LdapError> {
        let search_filter = format!(
            "(&(objectClass=user)(sAMAccountName={}))",
            self.escape_ldap_filter(username)
        );
        
        let (rs, _res) = ldap
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &search_filter,
                vec!["cn", "mail", "memberOf", "userPrincipalName"],
            )
            .await
            .map_err(|e| LdapError::SearchFailed(e.to_string()))?
            .success()
            .map_err(|e| LdapError::SearchFailed(e.to_string()))?;
        
        let entry = rs
            .into_iter()
            .next()
            .ok_or(LdapError::UserNotFound)?;
        
        let search_entry = SearchEntry::construct(entry);
        
        Ok(LdapUserInfo {
            dn: search_entry.dn,
            cn: search_entry.attrs.get("cn")
                .and_then(|v| v.first())
                .cloned(),
            email: search_entry.attrs.get("mail")
                .and_then(|v| v.first())
                .cloned(),
            groups: search_entry.attrs.get("memberOf")
                .cloned()
                .unwrap_or_default(),
        })
    }
    
    /// Escape special characters in LDAP filter
    fn escape_ldap_filter(&self, input: &str) -> String {
        input
            .replace("\\", "\\5c")
            .replace("*", "\\2a")
            .replace("(", "\\28")
            .replace(")", "\\29")
            .replace("\0", "\\00")
    }
}

pub struct LdapConfig {
    pub url: String,                    // ldaps://ldap.example.com:636
    pub base_dn: String,                // dc=example,dc=com
    pub bind_dn_template: Option<String>, // cn={username},ou=users,dc=example,dc=com
    pub domain: Option<String>,         // For Active Directory UPN
    pub use_tls: bool,
    pub ca_cert: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LdapUserInfo {
    pub dn: String,
    pub cn: Option<String>,
    pub email: Option<String>,
    pub groups: Vec<String>,
}
```

### 4.2 LDAP Connection Pool

```rust
use tokio::sync::Semaphore;

/// Connection pool for LDAP connections
pub struct LdapConnectionPool {
    config: LdapConfig,
    semaphore: Arc<Semaphore>,
    max_connections: usize,
}

impl LdapConnectionPool {
    pub fn new(config: LdapConfig, max_connections: usize) -> Self {
        Self {
            config,
            semaphore: Arc::new(Semaphore::new(max_connections)),
            max_connections,
        }
    }
    
    /// Get connection from pool (or create new)
    pub async fn get(&self) -> Result<LdapConn, LdapError> {
        // Acquire permit
        let _permit = self.semaphore.acquire().await
            .map_err(|e| LdapError::PoolExhausted(e.to_string()))?;
        
        // Create new connection
        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url).await
            .map_err(|e| LdapError::ConnectionFailed(e.to_string()))?;
        
        // Start TLS if configured
        if self.config.use_tls {
            ldap.start_tls().await
                .map_err(|e| LdapError::TlsNegotiationFailed(e.to_string()))?;
        }
        
        // Spawn connection driver
        tokio::spawn(async move {
            conn.drive().await;
        });
        
        Ok(ldap)
    }
    
    pub async fn return_connection(&self, ldap: LdapConn) {
        // Unbind and close
        let _ = ldap.unbind().await;
    }
}
```

---

## 5. mTLS Implementation

### 5.1 Certificate Management

**Mutual TLS (mTLS) Certificate Handling:**

```rust
use openssl::x509::X509;
use openssl::pkey::{PKey, Private};
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use tokio_native_tls::TlsConnector;

/// mTLS credential with client certificate
pub struct MtlsCredential {
    config: MtlsConfig,
    client_cert: X509,
    client_key: PKey<Private>,
    ca_cert: Option<X509>,
}

impl MtlsCredential {
    /// Load mTLS credential from PEM files
    pub fn from_pem(
        client_cert_pem: &str,
        client_key_pem: &SecretString,
        ca_cert_pem: Option<&str>,
    ) -> Result<Self, MtlsError> {
        // Parse client certificate
        let client_cert = X509::from_pem(client_cert_pem.as_bytes())
            .map_err(|e| MtlsError::InvalidCertificate(e.to_string()))?;
        
        // Parse client private key (with zeroization)
        let client_key = PKey::private_key_from_pem(client_key_pem.expose().as_bytes())
            .map_err(|e| MtlsError::InvalidPrivateKey(e.to_string()))?;
        
        // Validate certificate and key match
        Self::validate_cert_key_match(&client_cert, &client_key)?;
        
        // Parse CA certificate if provided
        let ca_cert = ca_cert_pem
            .map(|pem| X509::from_pem(pem.as_bytes()))
            .transpose()
            .map_err(|e| MtlsError::InvalidCaCertificate(e.to_string()))?;
        
        Ok(Self {
            config: MtlsConfig::default(),
            client_cert,
            client_key,
            ca_cert,
        })
    }
    
    /// Validate certificate hasn't expired
    pub fn validate_expiry(&self) -> Result<(), MtlsError> {
        let now = SystemTime::now();
        let not_before = self.client_cert.not_before();
        let not_after = self.client_cert.not_after();
        
        // Check NotBefore
        if now < not_before.to_system_time()? {
            return Err(MtlsError::CertificateNotYetValid);
        }
        
        // Check NotAfter
        if now > not_after.to_system_time()? {
            return Err(MtlsError::CertificateExpired);
        }
        
        Ok(())
    }
    
    /// Validate certificate chain
    pub fn validate_chain(&self) -> Result<(), MtlsError> {
        if let Some(ca_cert) = &self.ca_cert {
            // Verify client cert signed by CA
            let ca_key = ca_cert.public_key()
                .map_err(|e| MtlsError::InvalidCaCertificate(e.to_string()))?;
            
            let valid = self.client_cert.verify(&ca_key)
                .map_err(|e| MtlsError::ChainVerificationFailed(e.to_string()))?;
            
            if !valid {
                return Err(MtlsError::ChainVerificationFailed(
                    "Client certificate not signed by CA".to_string()
                ));
            }
        }
        
        Ok(())
    }
    
    fn validate_cert_key_match(
        cert: &X509,
        key: &PKey<Private>,
    ) -> Result<(), MtlsError> {
        let cert_pubkey = cert.public_key()
            .map_err(|e| MtlsError::InvalidCertificate(e.to_string()))?;
        
        // Compare public key from cert with private key
        if cert_pubkey.public_eq(key) {
            Ok(())
        } else {
            Err(MtlsError::CertificateKeyMismatch)
        }
    }
    
    /// Build TLS connector with client certificate
    pub fn build_tls_connector(&self) -> Result<TlsConnector, MtlsError> {
        let mut builder = SslConnector::builder(SslMethod::tls())
            .map_err(|e| MtlsError::TlsConfigError(e.to_string()))?;
        
        // Set client certificate and key
        builder.set_certificate(&self.client_cert)
            .map_err(|e| MtlsError::TlsConfigError(e.to_string()))?;
        
        builder.set_private_key(&self.client_key)
            .map_err(|e| MtlsError::TlsConfigError(e.to_string()))?;
        
        // Set CA certificate for server verification
        if let Some(ca_cert) = &self.ca_cert {
            builder.cert_store_mut().add_cert(ca_cert.clone())
                .map_err(|e| MtlsError::TlsConfigError(e.to_string()))?;
        }
        
        // Require server certificate verification
        builder.set_verify(SslVerifyMode::PEER);
        
        let connector = TlsConnector::from(builder.build());
        Ok(connector)
    }
}

pub struct MtlsConfig {
    pub verify_hostname: bool,
    pub allowed_cipher_suites: Vec<String>,
    pub min_tls_version: TlsVersion,
}

impl Default for MtlsConfig {
    fn default() -> Self {
        Self {
            verify_hostname: true,
            allowed_cipher_suites: vec![
                "TLS_AES_256_GCM_SHA384".to_string(),
                "TLS_AES_128_GCM_SHA256".to_string(),
                "TLS_CHACHA20_POLY1305_SHA256".to_string(),
            ],
            min_tls_version: TlsVersion::Tls13,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TlsVersion {
    Tls12,
    Tls13,
}
```

### 5.2 Certificate Rotation

**Zero-downtime certificate rotation:**

```rust
/// mTLS certificate rotation manager
pub struct MtlsCertificateRotation {
    current: Arc<RwLock<MtlsCredential>>,
    rotation_policy: CertRotationPolicy,
}

impl MtlsCertificateRotation {
    /// Rotate certificate with grace period
    pub async fn rotate(
        &self,
        new_cert_pem: &str,
        new_key_pem: &SecretString,
        ca_cert_pem: Option<&str>,
    ) -> Result<(), MtlsError> {
        // Load new credential
        let new_cred = MtlsCredential::from_pem(
            new_cert_pem,
            new_key_pem,
            ca_cert_pem,
        )?;
        
        // Validate new credential
        new_cred.validate_expiry()?;
        new_cred.validate_chain()?;
        
        // Atomic swap
        let mut current = self.current.write().await;
        *current = new_cred;
        
        Ok(())
    }
    
    /// Check if rotation needed
    pub async fn needs_rotation(&self) -> bool {
        let current = self.current.read().await;
        
        // Check expiry time
        let not_after = current.client_cert.not_after();
        let expires_at = not_after.to_system_time().unwrap();
        let now = SystemTime::now();
        
        match expires_at.duration_since(now) {
            Ok(time_remaining) => {
                time_remaining < self.rotation_policy.rotate_before_expiry
            }
            Err(_) => true, // Already expired
        }
    }
    
    /// Get current credential (read-only)
    pub async fn current(&self) -> Arc<RwLock<MtlsCredential>> {
        self.current.clone()
    }
}

pub struct CertRotationPolicy {
    pub rotate_before_expiry: Duration, // Rotate 7 days before expiry
    pub auto_rotation: bool,
}

impl Default for CertRotationPolicy {
    fn default() -> Self {
        Self {
            rotate_before_expiry: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
            auto_rotation: true,
        }
    }
}
```

### 5.3 mTLS HTTP Client

```rust
use reqwest::Client;

/// HTTP client with mTLS authentication
pub struct MtlsHttpClient {
    client: Client,
    credential: Arc<RwLock<MtlsCredential>>,
}

impl MtlsHttpClient {
    pub async fn new(credential: MtlsCredential) -> Result<Self, MtlsError> {
        let connector = credential.build_tls_connector()?;
        
        let client = Client::builder()
            .use_preconfigured_tls(connector)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| MtlsError::ClientBuildError(e.to_string()))?;
        
        Ok(Self {
            client,
            credential: Arc::new(RwLock::new(credential)),
        })
    }
    
    /// Make authenticated GET request
    pub async fn get(&self, url: &str) -> Result<Response, MtlsError> {
        self.client
            .get(url)
            .send()
            .await
            .map_err(|e| MtlsError::RequestFailed(e.to_string()))
    }
    
    /// Make authenticated POST request
    pub async fn post(&self, url: &str, body: impl Into<Body>) -> Result<Response, MtlsError> {
        self.client
            .post(url)
            .body(body)
            .send()
            .await
            .map_err(|e| MtlsError::RequestFailed(e.to_string()))
    }
}
```

---

## 6. JWT Implementation

### 6.1 JWT Generation and Validation

```rust
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// JWT credential provider
pub struct JwtCredential {
    config: JwtConfig,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtCredential {
    /// Create JWT credential with symmetric key (HMAC)
    pub fn from_secret(secret: &SecretString, algorithm: Algorithm) -> Self {
        let encoding_key = EncodingKey::from_secret(secret.expose().as_bytes());
        let decoding_key = DecodingKey::from_secret(secret.expose().as_bytes());
        
        Self {
            config: JwtConfig {
                algorithm,
                issuer: None,
                audience: None,
                expiration: Duration::from_secs(3600), // 1 hour
            },
            encoding_key,
            decoding_key,
        }
    }
    
    /// Create JWT credential with RSA key pair
    pub fn from_rsa_pem(
        private_key_pem: &SecretString,
        public_key_pem: &str,
    ) -> Result<Self, JwtError> {
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem.expose().as_bytes())
            .map_err(|e| JwtError::InvalidPrivateKey(e.to_string()))?;
        
        let decoding_key = DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
            .map_err(|e| JwtError::InvalidPublicKey(e.to_string()))?;
        
        Ok(Self {
            config: JwtConfig {
                algorithm: Algorithm::RS256,
                issuer: None,
                audience: None,
                expiration: Duration::from_secs(3600),
            },
            encoding_key,
            decoding_key,
        })
    }
    
    /// Generate JWT token
    pub fn generate_token(&self, claims: JwtClaims) -> Result<String, JwtError> {
        let mut header = Header::new(self.config.algorithm);
        
        // Add key ID if available
        if let Some(kid) = &claims.kid {
            header.kid = Some(kid.clone());
        }
        
        let now = Utc::now().timestamp() as u64;
        
        let claims = Claims {
            sub: claims.subject,
            iss: claims.issuer.or_else(|| self.config.issuer.clone()),
            aud: claims.audience.or_else(|| self.config.audience.clone()),
            exp: now + self.config.expiration.as_secs(),
            iat: now,
            nbf: now,
            custom: claims.custom,
        };
        
        encode(&header, &claims, &self.encoding_key)
            .map_err(|e| JwtError::TokenGenerationFailed(e.to_string()))
    }
    
    /// Validate and decode JWT token
    pub fn validate_token(&self, token: &str) -> Result<Claims, JwtError> {
        let mut validation = Validation::new(self.config.algorithm);
        
        // Set validation parameters
        if let Some(iss) = &self.config.issuer {
            validation.set_issuer(&[iss]);
        }
        
        if let Some(aud) = &self.config.audience {
            validation.set_audience(&[aud]);
        }
        
        // Decode and validate
        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|e| JwtError::TokenValidationFailed(e.to_string()))?;
        
        Ok(token_data.claims)
    }
    
    /// Decode token without validation (for inspection)
    pub fn decode_unverified(&self, token: &str) -> Result<Claims, JwtError> {
        let mut validation = Validation::new(self.config.algorithm);
        validation.insecure_disable_signature_validation();
        
        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|e| JwtError::DecodeFailed(e.to_string()))?;
        
        Ok(token_data.claims)
    }
}

pub struct JwtConfig {
    pub algorithm: Algorithm,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub expiration: Duration,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,                           // Subject
    pub iss: Option<String>,                   // Issuer
    pub aud: Option<String>,                   // Audience
    pub exp: u64,                              // Expiration time
    pub iat: u64,                              // Issued at
    pub nbf: u64,                              // Not before
    #[serde(flatten)]
    pub custom: HashMap<String, serde_json::Value>, // Custom claims
}

pub struct JwtClaims {
    pub subject: String,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub kid: Option<String>,                   // Key ID
    pub custom: HashMap<String, serde_json::Value>,
}
```

### 6.2 JWT Refresh Token Implementation

```rust
/// JWT with refresh token support
pub struct JwtWithRefresh {
    jwt: JwtCredential,
    refresh_tokens: Arc<RwLock<HashMap<String, RefreshToken>>>,
}

impl JwtWithRefresh {
    /// Generate access token and refresh token pair
    pub async fn generate_token_pair(
        &self,
        claims: JwtClaims,
    ) -> Result<TokenPair, JwtError> {
        // Generate access token (short-lived)
        let access_token = self.jwt.generate_token(claims.clone())?;
        
        // Generate refresh token (long-lived)
        let refresh_token = self.generate_refresh_token(&claims.subject)?;
        
        // Store refresh token
        let mut tokens = self.refresh_tokens.write().await;
        tokens.insert(
            refresh_token.token.clone(),
            RefreshToken {
                token: refresh_token.token.clone(),
                subject: claims.subject.clone(),
                issued_at: Utc::now(),
                expires_at: Utc::now() + chrono::Duration::days(30),
            },
        );
        
        Ok(TokenPair {
            access_token,
            refresh_token: refresh_token.token,
            expires_in: self.jwt.config.expiration.as_secs(),
        })
    }
    
    /// Refresh access token using refresh token
    pub async fn refresh_access_token(
        &self,
        refresh_token: &str,
    ) -> Result<String, JwtError> {
        // Validate refresh token
        let tokens = self.refresh_tokens.read().await;
        let refresh = tokens
            .get(refresh_token)
            .ok_or(JwtError::InvalidRefreshToken)?;
        
        // Check expiration
        if Utc::now() > refresh.expires_at {
            return Err(JwtError::RefreshTokenExpired);
        }
        
        // Generate new access token
        let claims = JwtClaims {
            subject: refresh.subject.clone(),
            issuer: self.jwt.config.issuer.clone(),
            audience: self.jwt.config.audience.clone(),
            kid: None,
            custom: HashMap::new(),
        };
        
        self.jwt.generate_token(claims)
    }
    
    /// Revoke refresh token
    pub async fn revoke_refresh_token(&self, token: &str) -> Result<(), JwtError> {
        let mut tokens = self.refresh_tokens.write().await;
        tokens.remove(token);
        Ok(())
    }
    
    fn generate_refresh_token(&self, subject: &str) -> Result<RefreshToken, JwtError> {
        let mut token_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut token_bytes);
        let token = URL_SAFE_NO_PAD.encode(token_bytes);
        
        Ok(RefreshToken {
            token,
            subject: subject.to_string(),
            issued_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(30),
        })
    }
}

#[derive(Debug, Clone)]
struct RefreshToken {
    token: String,
    subject: String,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}
```

---

## 7. API Key Implementation

### 7.1 API Key Generation and Storage

```rust
use blake3::Hasher;

/// API Key credential provider
pub struct ApiKeyCredential {
    config: ApiKeyConfig,
    storage: Arc<dyn StorageProvider>,
}

impl ApiKeyCredential {
    /// Generate new API key
    pub async fn generate_key(
        &self,
        owner_id: &str,
        scopes: Vec<String>,
    ) -> Result<ApiKeyPair, ApiKeyError> {
        // Generate cryptographically secure key
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        
        // Create key with prefix for easy identification
        let key = format!(
            "{}_{}",
            self.config.key_prefix,
            URL_SAFE_NO_PAD.encode(key_bytes)
        );
        
        // Hash key for storage (BLAKE3 for speed and security)
        let key_hash = self.hash_key(&key)?;
        
        // Create key metadata
        let metadata = ApiKeyMetadata {
            id: Uuid::new_v4().to_string(),
            key_hash,
            owner_id: owner_id.to_string(),
            scopes,
            created_at: Utc::now(),
            expires_at: self.calculate_expiry(),
            last_used: None,
            usage_count: 0,
            rate_limit: self.config.default_rate_limit.clone(),
        };
        
        // Store metadata (NOT the raw key)
        self.storage.store_api_key(&metadata).await?;
        
        Ok(ApiKeyPair {
            key: SecretString::new(key),
            metadata,
        })
    }
    
    /// Validate API key
    pub async fn validate_key(&self, key: &str) -> Result<ApiKeyMetadata, ApiKeyError> {
        // Hash provided key
        let key_hash = self.hash_key(key)?;
        
        // Lookup metadata by hash
        let mut metadata = self.storage
            .get_api_key_by_hash(&key_hash)
            .await?
            .ok_or(ApiKeyError::InvalidKey)?;
        
        // Check expiration
        if let Some(expires_at) = metadata.expires_at {
            if Utc::now() > expires_at {
                return Err(ApiKeyError::KeyExpired);
            }
        }
        
        // Update last used timestamp
        metadata.last_used = Some(Utc::now());
        metadata.usage_count += 1;
        self.storage.update_api_key(&metadata).await?;
        
        Ok(metadata)
    }
    
    /// Revoke API key
    pub async fn revoke_key(&self, key_id: &str) -> Result<(), ApiKeyError> {
        self.storage.delete_api_key(key_id).await?;
        Ok(())
    }
    
    /// Hash API key using BLAKE3
    fn hash_key(&self, key: &str) -> Result<String, ApiKeyError> {
        let mut hasher = Hasher::new();
        hasher.update(key.as_bytes());
        let hash = hasher.finalize();
        Ok(hash.to_hex().to_string())
    }
    
    fn calculate_expiry(&self) -> Option<DateTime<Utc>> {
        self.config.default_expiration
            .map(|duration| Utc::now() + chrono::Duration::from_std(duration).unwrap())
    }
}

pub struct ApiKeyConfig {
    pub key_prefix: String,                    // e.g., "sk" for secret key
    pub default_expiration: Option<Duration>,  // None = never expires
    pub default_rate_limit: RateLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyMetadata {
    pub id: String,
    pub key_hash: String,
    pub owner_id: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used: Option<DateTime<Utc>>,
    pub usage_count: u64,
    pub rate_limit: RateLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimit {
    pub requests_per_minute: u32,
    pub requests_per_hour: u32,
    pub requests_per_day: u32,
}

pub struct ApiKeyPair {
    pub key: SecretString,
    pub metadata: ApiKeyMetadata,
}
```

### 7.2 API Key Rotation

```rust
/// Zero-downtime API key rotation
pub struct ApiKeyRotation {
    credential: ApiKeyCredential,
}

impl ApiKeyRotation {
    /// Rotate API key with grace period
    pub async fn rotate(
        &self,
        old_key_id: &str,
        grace_period: Duration,
    ) -> Result<ApiKeyPair, ApiKeyError> {
        // Get old key metadata
        let old_metadata = self.credential.storage
            .get_api_key(old_key_id)
            .await?
            .ok_or(ApiKeyError::KeyNotFound)?;
        
        // Generate new key with same owner and scopes
        let new_key_pair = self.credential
            .generate_key(&old_metadata.owner_id, old_metadata.scopes.clone())
            .await?;
        
        // Mark old key for deletion after grace period
        tokio::spawn({
            let storage = self.credential.storage.clone();
            let old_key_id = old_key_id.to_string();
            async move {
                tokio::time::sleep(grace_period).await;
                let _ = storage.delete_api_key(&old_key_id).await;
            }
        });
        
        Ok(new_key_pair)
    }
}
```

---

## 8. Kerberos Implementation

### 8.1 Kerberos Authentication

```rust
use libgssapi::{
    context::{ClientCtx, CtxFlags},
    credential::{Cred, CredUsage},
    name::Name,
    oid::{OidSet, GSS_MECH_KRB5},
};

/// Kerberos credential provider
pub struct KerberosCredential {
    config: KerberosConfig,
}

impl KerberosCredential {
    /// Authenticate with Kerberos KDC
    pub async fn authenticate(
        &self,
        username: &str,
        password: &SecretString,
        service_principal: &str,
    ) -> Result<KerberosTicket, KerberosError> {
        // Step 1: Acquire TGT (Ticket Granting Ticket)
        let tgt = self.acquire_tgt(username, password).await?;
        
        // Step 2: Request service ticket
        let service_ticket = self.request_service_ticket(&tgt, service_principal).await?;
        
        Ok(service_ticket)
    }
    
    async fn acquire_tgt(
        &self,
        username: &str,
        password: &SecretString,
    ) -> Result<KerberosTicket, KerberosError> {
        // Construct principal name
        let principal = format!("{}@{}", username, self.config.realm);
        
        // Create GSS-API name
        let name = Name::new(principal.as_bytes(), Some(&GSS_NT_USER_NAME))
            .map_err(|e| KerberosError::NameCreationFailed(e.to_string()))?;
        
        // Acquire credential
        let cred = Cred::acquire(
            Some(&name),
            None,
            CredUsage::Initiate,
            Some(&OidSet::from([GSS_MECH_KRB5])),
        )
        .map_err(|e| KerberosError::CredentialAcquisitionFailed(e.to_string()))?;
        
        Ok(KerberosTicket {
            principal: principal.clone(),
            credential: cred,
            expires_at: self.get_ticket_expiry(&cred)?,
        })
    }
    
    async fn request_service_ticket(
        &self,
        tgt: &KerberosTicket,
        service_principal: &str,
    ) -> Result<KerberosTicket, KerberosError> {
        // Create service name
        let service_name = Name::new(service_principal.as_bytes(), Some(&GSS_NT_HOSTBASED_SERVICE))
            .map_err(|e| KerberosError::NameCreationFailed(e.to_string()))?;
        
        // Initialize security context
        let mut ctx = ClientCtx::new(
            Some(&tgt.credential),
            &service_name,
            CtxFlags::GSS_C_MUTUAL_FLAG | CtxFlags::GSS_C_REPLAY_FLAG,
            Some(&GSS_MECH_KRB5),
        );
        
        // Generate initial token
        let token = ctx
            .step(None)
            .map_err(|e| KerberosError::ContextInitFailed(e.to_string()))?;
        
        Ok(KerberosTicket {
            principal: tgt.principal.clone(),
            credential: tgt.credential.clone(),
            expires_at: tgt.expires_at,
        })
    }
    
    fn get_ticket_expiry(&self, cred: &Cred) -> Result<DateTime<Utc>, KerberosError> {
        // Extract expiry from credential
        let lifetime = cred.lifetime()
            .map_err(|e| KerberosError::ExpiryExtractionFailed(e.to_string()))?;
        
        Ok(Utc::now() + chrono::Duration::seconds(lifetime as i64))
    }
}

pub struct KerberosConfig {
    pub realm: String,              // e.g., "EXAMPLE.COM"
    pub kdc_servers: Vec<String>,   // KDC server addresses
    pub service_principal: String,  // e.g., "HTTP/api.example.com"
}

pub struct KerberosTicket {
    pub principal: String,
    pub credential: Cred,
    pub expires_at: DateTime<Utc>,
}
```

### 8.2 Kerberos Ticket Renewal

```rust
/// Kerberos ticket renewal manager
pub struct KerberosRenewal {
    credential: KerberosCredential,
    current_ticket: Arc<RwLock<Option<KerberosTicket>>>,
}

impl KerberosRenewal {
    /// Automatically renew ticket before expiry
    pub async fn auto_renew_loop(
        &self,
        username: &str,
        password: &SecretString,
        service_principal: &str,
    ) {
        loop {
            // Check if renewal needed
            let needs_renewal = {
                let ticket = self.current_ticket.read().await;
                match ticket.as_ref() {
                    None => true,
                    Some(t) => {
                        let time_remaining = t.expires_at - Utc::now();
                        time_remaining < chrono::Duration::minutes(5)
                    }
                }
            };
            
            if needs_renewal {
                match self.credential.authenticate(username, password, service_principal).await {
                    Ok(new_ticket) => {
                        let mut ticket = self.current_ticket.write().await;
                        *ticket = Some(new_ticket);
                    }
                    Err(e) => {
                        eprintln!("Kerberos renewal failed: {:?}", e);
                    }
                }
            }
            
            // Sleep for 1 minute before checking again
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}
```

---

## 9. Storage Provider Implementations

### 9.1 Storage Provider Trait

```rust
/// Universal storage provider trait for credentials
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Store encrypted credential
    async fn store(
        &self,
        id: &str,
        encrypted_data: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError>;
    
    /// Retrieve encrypted credential
    async fn retrieve(&self, id: &str) -> Result<Option<EncryptedData>, StorageError>;
    
    /// Delete credential
    async fn delete(&self, id: &str) -> Result<(), StorageError>;
    
    /// List credentials (metadata only)
    async fn list(&self, filter: Option<&CredentialFilter>) -> Result<Vec<CredentialMetadata>, StorageError>;
    
    /// Update credential metadata
    async fn update_metadata(&self, id: &str, metadata: &CredentialMetadata) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    pub id: String,
    pub credential_type: String,
    pub owner_id: String,
    pub scope_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub tags: HashMap<String, String>,
}

pub struct CredentialFilter {
    pub owner_id: Option<String>,
    pub credential_type: Option<String>,
    pub scope_id: Option<String>,
    pub tags: HashMap<String, String>,
}
```

### 9.2 Local Storage Provider (SQLite)

```rust
use sqlx::{SqlitePool, Row};

/// Local SQLite storage provider
pub struct LocalStorageProvider {
    pool: SqlitePool,
}

impl LocalStorageProvider {
    pub async fn new(database_url: &str) -> Result<Self, StorageError> {
        let pool = SqlitePool::connect(database_url)
            .await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        // Initialize schema
        Self::initialize_schema(&pool).await?;
        
        Ok(Self { pool })
    }
    
    async fn initialize_schema(pool: &SqlitePool) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS credentials (
                id TEXT PRIMARY KEY,
                encrypted_data BLOB NOT NULL,
                nonce BLOB NOT NULL,
                version INTEGER NOT NULL,
                credential_type TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                scope_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                expires_at TEXT,
                tags TEXT NOT NULL
            );
            
            CREATE INDEX IF NOT EXISTS idx_owner_id ON credentials(owner_id);
            CREATE INDEX IF NOT EXISTS idx_credential_type ON credentials(credential_type);
            CREATE INDEX IF NOT EXISTS idx_scope_id ON credentials(scope_id);
            "#
        )
        .execute(pool)
        .await
        .map_err(|e| StorageError::SchemaInitFailed(e.to_string()))?;
        
        Ok(())
    }
}

#[async_trait]
impl StorageProvider for LocalStorageProvider {
    async fn store(
        &self,
        id: &str,
        encrypted_data: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        let tags_json = serde_json::to_string(&metadata.tags)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        sqlx::query(
            r#"
            INSERT INTO credentials (
                id, encrypted_data, nonce, version, credential_type, owner_id, 
                scope_id, created_at, updated_at, expires_at, tags
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                encrypted_data = excluded.encrypted_data,
                nonce = excluded.nonce,
                version = excluded.version,
                updated_at = excluded.updated_at
            "#
        )
        .bind(id)
        .bind(&encrypted_data.ciphertext)
        .bind(&encrypted_data.nonce)
        .bind(encrypted_data.version)
        .bind(&metadata.credential_type)
        .bind(&metadata.owner_id)
        .bind(&metadata.scope_id)
        .bind(metadata.created_at.to_rfc3339())
        .bind(metadata.updated_at.to_rfc3339())
        .bind(metadata.expires_at.map(|dt| dt.to_rfc3339()))
        .bind(tags_json)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn retrieve(&self, id: &str) -> Result<Option<EncryptedData>, StorageError> {
        let row = sqlx::query(
            "SELECT encrypted_data, nonce, version FROM credentials WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::ReadFailed(e.to_string()))?;
        
        Ok(row.map(|r| EncryptedData {
            ciphertext: r.get("encrypted_data"),
            nonce: r.get("nonce"),
            version: r.get("version"),
        }))
    }
    
    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM credentials WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn list(&self, filter: Option<&CredentialFilter>) -> Result<Vec<CredentialMetadata>, StorageError> {
        // Implementation with WHERE clause building based on filter
        // ...
        Ok(vec![])
    }
    
    async fn update_metadata(&self, id: &str, metadata: &CredentialMetadata) -> Result<(), StorageError> {
        let tags_json = serde_json::to_string(&metadata.tags)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        sqlx::query(
            r#"
            UPDATE credentials 
            SET updated_at = ?, expires_at = ?, tags = ?
            WHERE id = ?
            "#
        )
        .bind(metadata.updated_at.to_rfc3339())
        .bind(metadata.expires_at.map(|dt| dt.to_rfc3339()))
        .bind(tags_json)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::UpdateFailed(e.to_string()))?;
        
        Ok(())
    }
}
```

### 9.3 AWS Secrets Manager Provider

```rust
use aws_sdk_secretsmanager::{Client, types::Tag};

/// AWS Secrets Manager storage provider
pub struct AwsSecretsProvider {
    client: Client,
    key_prefix: String,
}

impl AwsSecretsProvider {
    pub async fn new(config: &aws_config::SdkConfig, key_prefix: String) -> Self {
        let client = Client::new(config);
        Self { client, key_prefix }
    }
    
    fn format_secret_name(&self, id: &str) -> String {
        format!("{}/{}", self.key_prefix, id)
    }
}

#[async_trait]
impl StorageProvider for AwsSecretsProvider {
    async fn store(
        &self,
        id: &str,
        encrypted_data: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        let secret_name = self.format_secret_name(id);
        
        // Serialize encrypted data
        let secret_value = serde_json::to_string(encrypted_data)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        // Convert metadata to tags
        let tags: Vec<Tag> = vec![
            Tag::builder().key("credential_type").value(&metadata.credential_type).build(),
            Tag::builder().key("owner_id").value(&metadata.owner_id).build(),
            Tag::builder().key("created_at").value(metadata.created_at.to_rfc3339()).build(),
        ];
        
        // Try to create secret
        let create_result = self.client
            .create_secret()
            .name(&secret_name)
            .secret_string(secret_value.clone())
            .tags(tags.clone())
            .send()
            .await;
        
        match create_result {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("ResourceExistsException") => {
                // Secret exists, update it
                self.client
                    .put_secret_value()
                    .secret_id(&secret_name)
                    .secret_string(secret_value)
                    .send()
                    .await
                    .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
                
                Ok(())
            }
            Err(e) => Err(StorageError::WriteFailed(e.to_string())),
        }
    }
    
    async fn retrieve(&self, id: &str) -> Result<Option<EncryptedData>, StorageError> {
        let secret_name = self.format_secret_name(id);
        
        let result = self.client
            .get_secret_value()
            .secret_id(&secret_name)
            .send()
            .await;
        
        match result {
            Ok(output) => {
                let secret_string = output.secret_string()
                    .ok_or_else(|| StorageError::ReadFailed("No secret string".to_string()))?;
                
                let encrypted_data: EncryptedData = serde_json::from_str(secret_string)
                    .map_err(|e| StorageError::DeserializationError(e.to_string()))?;
                
                Ok(Some(encrypted_data))
            }
            Err(e) if e.to_string().contains("ResourceNotFoundException") => Ok(None),
            Err(e) => Err(StorageError::ReadFailed(e.to_string())),
        }
    }
    
    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let secret_name = self.format_secret_name(id);
        
        self.client
            .delete_secret()
            .secret_id(&secret_name)
            .force_delete_without_recovery(true)
            .send()
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn list(&self, filter: Option<&CredentialFilter>) -> Result<Vec<CredentialMetadata>, StorageError> {
        // Use ListSecrets API with filters
        // ...
        Ok(vec![])
    }
    
    async fn update_metadata(&self, id: &str, metadata: &CredentialMetadata) -> Result<(), StorageError> {
        // AWS Secrets Manager doesn't support metadata updates without retrieving secret
        // Would need to retrieve, update tags, and store back
        Ok(())
    }
}
```

### 9.4 HashiCorp Vault Provider

```rust
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};

/// HashiCorp Vault storage provider
pub struct VaultStorageProvider {
    client: VaultClient,
    mount_path: String,
}

impl VaultStorageProvider {
    pub async fn new(
        vault_addr: &str,
        vault_token: &SecretString,
        mount_path: String,
    ) -> Result<Self, StorageError> {
        let settings = VaultClientSettingsBuilder::default()
            .address(vault_addr)
            .token(vault_token.expose())
            .build()
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        let client = VaultClient::new(settings)
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        Ok(Self { client, mount_path })
    }
}

#[async_trait]
impl StorageProvider for VaultStorageProvider {
    async fn store(
        &self,
        id: &str,
        encrypted_data: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        let path = format!("{}/{}", self.mount_path, id);
        
        // Create data map
        let mut data = HashMap::new();
        data.insert("ciphertext", serde_json::to_value(&encrypted_data.ciphertext)?);
        data.insert("nonce", serde_json::to_value(&encrypted_data.nonce)?);
        data.insert("version", serde_json::to_value(encrypted_data.version)?);
        data.insert("metadata", serde_json::to_value(metadata)?);
        
        // Store in Vault KV v2
        vaultrs::kv2::set(&self.client, &self.mount_path, id, &data)
            .await
            .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn retrieve(&self, id: &str) -> Result<Option<EncryptedData>, StorageError> {
        match vaultrs::kv2::read::<HashMap<String, serde_json::Value>>(&self.client, &self.mount_path, id).await {
            Ok(data) => {
                let encrypted_data = EncryptedData {
                    ciphertext: serde_json::from_value(data.get("ciphertext").cloned().unwrap())?,
                    nonce: serde_json::from_value(data.get("nonce").cloned().unwrap())?,
                    version: serde_json::from_value(data.get("version").cloned().unwrap())?,
                };
                Ok(Some(encrypted_data))
            }
            Err(e) if e.to_string().contains("404") => Ok(None),
            Err(e) => Err(StorageError::ReadFailed(e.to_string())),
        }
    }
    
    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        vaultrs::kv2::delete(&self.client, &self.mount_path, id)
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn list(&self, filter: Option<&CredentialFilter>) -> Result<Vec<CredentialMetadata>, StorageError> {
        // Use Vault list API
        // ...
        Ok(vec![])
    }
    
    async fn update_metadata(&self, id: &str, metadata: &CredentialMetadata) -> Result<(), StorageError> {
        // Retrieve, update metadata field, store back
        Ok(())
    }
}
```

---

## 10. Caching Strategy

### 10.1 Cache Layer Implementation

```rust
use moka::future::Cache;

/// Multi-level credential cache
pub struct CredentialCache {
    memory_cache: Cache<String, CachedCredential>,
    redis_cache: Option<RedisCache>,
    config: CacheConfig,
}

impl CredentialCache {
    pub fn new(config: CacheConfig) -> Self {
        let memory_cache = Cache::builder()
            .max_capacity(config.max_memory_entries)
            .time_to_live(config.ttl)
            .build();
        
        Self {
            memory_cache,
            redis_cache: None,
            config,
        }
    }
    
    /// Get credential from cache
    pub async fn get(&self, key: &str) -> Option<CachedCredential> {
        // Try L1 cache (memory)
        if let Some(cached) = self.memory_cache.get(key).await {
            return Some(cached);
        }
        
        // Try L2 cache (Redis)
        if let Some(redis) = &self.redis_cache {
            if let Ok(Some(cached)) = redis.get(key).await {
                // Populate L1 cache
                self.memory_cache.insert(key.to_string(), cached.clone()).await;
                return Some(cached);
            }
        }
        
        None
    }
    
    /// Store credential in cache
    pub async fn set(&self, key: String, credential: CachedCredential) {
        // Store in L1 cache
        self.memory_cache.insert(key.clone(), credential.clone()).await;
        
        // Store in L2 cache
        if let Some(redis) = &self.redis_cache {
            let _ = redis.set(&key, &credential, self.config.ttl).await;
        }
    }
    
    /// Invalidate credential from cache
    pub async fn invalidate(&self, key: &str) {
        self.memory_cache.invalidate(key).await;
        
        if let Some(redis) = &self.redis_cache {
            let _ = redis.delete(key).await;
        }
    }
    
    /// Invalidate all credentials for owner
    pub async fn invalidate_by_owner(&self, owner_id: &str) {
        // This requires maintaining owner->keys mapping
        // Implementation details...
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedCredential {
    pub credential_data: Vec<u8>,
    pub credential_type: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub cached_at: DateTime<Utc>,
}

pub struct CacheConfig {
    pub max_memory_entries: u64,
    pub ttl: Duration,
    pub use_redis: bool,
    pub redis_url: Option<String>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_memory_entries: 10_000,
            ttl: Duration::from_secs(300), // 5 minutes
            use_redis: false,
            redis_url: None,
        }
    }
}
```

### 10.2 Redis Cache Implementation

```rust
use redis::{AsyncCommands, aio::ConnectionManager};

pub struct RedisCache {
    conn: ConnectionManager,
}

impl RedisCache {
    pub async fn new(redis_url: &str) -> Result<Self, CacheError> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| CacheError::ConnectionFailed(e.to_string()))?;
        
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| CacheError::ConnectionFailed(e.to_string()))?;
        
        Ok(Self { conn })
    }
    
    pub async fn get(&self, key: &str) -> Result<Option<CachedCredential>, CacheError> {
        let mut conn = self.conn.clone();
        
        let data: Option<Vec<u8>> = conn.get(key)
            .await
            .map_err(|e| CacheError::ReadFailed(e.to_string()))?;
        
        match data {
            Some(bytes) => {
                let cached: CachedCredential = bincode::deserialize(&bytes)
                    .map_err(|e| CacheError::DeserializationError(e.to_string()))?;
                Ok(Some(cached))
            }
            None => Ok(None),
        }
    }
    
    pub async fn set(&self, key: &str, credential: &CachedCredential, ttl: Duration) -> Result<(), CacheError> {
        let mut conn = self.conn.clone();
        
        let bytes = bincode::serialize(credential)
            .map_err(|e| CacheError::SerializationError(e.to_string()))?;
        
        conn.set_ex(key, bytes, ttl.as_secs() as usize)
            .await
            .map_err(|e| CacheError::WriteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    pub async fn delete(&self, key: &str) -> Result<(), CacheError> {
        let mut conn = self.conn.clone();
        
        conn.del(key)
            .await
            .map_err(|e| CacheError::DeleteFailed(e.to_string()))?;
        
        Ok(())
    }
}
```

---

## 11. Observability Implementation

### 11.1 Structured Logging

```rust
use tracing::{info, warn, error, instrument};
use serde_json::json;

/// Audit logger for credential operations
pub struct AuditLogger {
    config: AuditConfig,
}

impl AuditLogger {
    /// Log credential access
    #[instrument(skip(self, credential_id))]
    pub fn log_access(
        &self,
        credential_id: &str,
        owner_id: &str,
        operation: &str,
        result: &Result<(), CredentialError>,
    ) {
        let log_entry = json!({
            "event_type": "credential_access",
            "credential_id": credential_id,
            "owner_id": owner_id,
            "operation": operation,
            "success": result.is_ok(),
            "error": result.as_ref().err().map(|e| e.to_string()),
            "timestamp": Utc::now().to_rfc3339(),
            "severity": if result.is_ok() { "INFO" } else { "ERROR" },
        });
        
        match result {
            Ok(_) => info!(target: "audit", "{}", log_entry),
            Err(_) => error!(target: "audit", "{}", log_entry),
        }
    }
    
    /// Log authentication attempt
    #[instrument(skip(self))]
    pub fn log_authentication(
        &self,
        credential_type: &str,
        owner_id: &str,
        success: bool,
        reason: Option<&str>,
    ) {
        let log_entry = json!({
            "event_type": "authentication",
            "credential_type": credential_type,
            "owner_id": owner_id,
            "success": success,
            "reason": reason,
            "timestamp": Utc::now().to_rfc3339(),
            "severity": if success { "INFO" } else { "WARN" },
        });
        
        if success {
            info!(target: "audit", "{}", log_entry);
        } else {
            warn!(target: "audit", "{}", log_entry);
        }
    }
    
    /// Log credential rotation
    #[instrument(skip(self))]
    pub fn log_rotation(
        &self,
        credential_id: &str,
        owner_id: &str,
        old_version: &str,
        new_version: &str,
    ) {
        let log_entry = json!({
            "event_type": "credential_rotation",
            "credential_id": credential_id,
            "owner_id": owner_id,
            "old_version": old_version,
            "new_version": new_version,
            "timestamp": Utc::now().to_rfc3339(),
            "severity": "INFO",
        });
        
        info!(target: "audit", "{}", log_entry);
    }
}

pub struct AuditConfig {
    pub log_level: tracing::Level,
    pub log_format: LogFormat,
    pub outputs: Vec<LogOutput>,
}

#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    Json,
    Logfmt,
    Pretty,
}

#[derive(Debug, Clone)]
pub enum LogOutput {
    Stdout,
    File(String),
    Syslog,
}
```

### 11.2 Prometheus Metrics

```rust
use prometheus::{
    Counter, Histogram, IntCounter, IntGauge, Registry,
    HistogramOpts, Opts,
};

/// Metrics collector for credential operations
pub struct CredentialMetrics {
    // Counters
    auth_attempts_total: Counter,
    auth_successes_total: Counter,
    auth_failures_total: Counter,
    credential_operations_total: Counter,
    cache_hits_total: Counter,
    cache_misses_total: Counter,
    
    // Gauges
    active_credentials: IntGauge,
    expired_credentials: IntGauge,
    
    // Histograms
    auth_duration_seconds: Histogram,
    credential_operation_duration: Histogram,
}

impl CredentialMetrics {
    pub fn new(registry: &Registry) -> Result<Self, MetricsError> {
        let auth_attempts_total = Counter::with_opts(
            Opts::new("credential_auth_attempts_total", "Total authentication attempts")
                .namespace("nebula")
        )?;
        registry.register(Box::new(auth_attempts_total.clone()))?;
        
        let auth_successes_total = Counter::with_opts(
            Opts::new("credential_auth_successes_total", "Total successful authentications")
                .namespace("nebula")
        )?;
        registry.register(Box::new(auth_successes_total.clone()))?;
        
        let auth_failures_total = Counter::with_opts(
            Opts::new("credential_auth_failures_total", "Total failed authentications")
                .namespace("nebula")
        )?;
        registry.register(Box::new(auth_failures_total.clone()))?;
        
        let auth_duration_seconds = Histogram::with_opts(
            HistogramOpts::new("credential_auth_duration_seconds", "Authentication duration")
                .namespace("nebula")
                .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0])
        )?;
        registry.register(Box::new(auth_duration_seconds.clone()))?;
        
        let active_credentials = IntGauge::with_opts(
            Opts::new("credential_active_total", "Number of active credentials")
                .namespace("nebula")
        )?;
        registry.register(Box::new(active_credentials.clone()))?;
        
        Ok(Self {
            auth_attempts_total,
            auth_successes_total,
            auth_failures_total,
            credential_operations_total: Counter::default(),
            cache_hits_total: Counter::default(),
            cache_misses_total: Counter::default(),
            active_credentials,
            expired_credentials: IntGauge::default(),
            auth_duration_seconds,
            credential_operation_duration: Histogram::default(),
        })
    }
    
    /// Record authentication attempt
    pub fn record_auth_attempt(&self, success: bool, duration: Duration) {
        self.auth_attempts_total.inc();
        
        if success {
            self.auth_successes_total.inc();
        } else {
            self.auth_failures_total.inc();
        }
        
        self.auth_duration_seconds.observe(duration.as_secs_f64());
    }
    
    /// Record cache hit/miss
    pub fn record_cache_access(&self, hit: bool) {
        if hit {
            self.cache_hits_total.inc();
        } else {
            self.cache_misses_total.inc();
        }
    }
    
    /// Update active credential count
    pub fn set_active_credentials(&self, count: i64) {
        self.active_credentials.set(count);
    }
}
```

### 11.3 OpenTelemetry Tracing

```rust
use opentelemetry::{
    global,
    trace::{Span, StatusCode, Tracer},
    KeyValue,
};

/// Distributed tracing for credential operations
pub struct CredentialTracing {
    tracer: Box<dyn Tracer + Send + Sync>,
}

impl CredentialTracing {
    pub fn new() -> Self {
        let tracer = global::tracer("nebula-credential");
        Self { tracer }
    }
    
    /// Trace authentication operation
    pub async fn trace_authentication<F, T>(
        &self,
        credential_type: &str,
        operation: F,
    ) -> Result<T, CredentialError>
    where
        F: Future<Output = Result<T, CredentialError>>,
    {
        let mut span = self.tracer.start("authenticate");
        span.set_attribute(KeyValue::new("credential.type", credential_type.to_string()));
        
        let start = std::time::Instant::now();
        let result = operation.await;
        let duration = start.elapsed();
        
        span.set_attribute(KeyValue::new("duration_ms", duration.as_millis() as i64));
        
        match &result {
            Ok(_) => {
                span.set_status(StatusCode::Ok, "Authentication successful");
            }
            Err(e) => {
                span.set_status(StatusCode::Error, format!("Authentication failed: {}", e));
                span.set_attribute(KeyValue::new("error", e.to_string()));
            }
        }
        
        span.end();
        result
    }
    
    /// Trace credential operation
    pub async fn trace_operation<F, T>(
        &self,
        operation_name: &str,
        credential_id: &str,
        operation: F,
    ) -> Result<T, CredentialError>
    where
        F: Future<Output = Result<T, CredentialError>>,
    {
        let mut span = self.tracer.start(operation_name);
        span.set_attribute(KeyValue::new("credential.id", credential_id.to_string()));
        
        let result = operation.await;
        
        match &result {
            Ok(_) => span.set_status(StatusCode::Ok, "Operation successful"),
            Err(e) => {
                span.set_status(StatusCode::Error, format!("Operation failed: {}", e));
                span.set_attribute(KeyValue::new("error", e.to_string()));
            }
        }
        
        span.end();
        result
    }
}
```

---

## 12. Performance Optimizations

### 12.1 Connection Pooling

```rust
use deadpool::managed::{Manager, Pool, RecycleResult};

/// Generic connection pool for credential providers
pub struct CredentialConnectionPool<M: Manager> {
    pool: Pool<M>,
}

impl<M: Manager> CredentialConnectionPool<M> {
    pub fn new(manager: M, max_size: usize) -> Self {
        let pool = Pool::builder(manager)
            .max_size(max_size)
            .build()
            .unwrap();
        
        Self { pool }
    }
    
    pub async fn get(&self) -> Result<PooledConnection<M>, PoolError> {
        self.pool.get().await
    }
}

/// LDAP connection pool manager
pub struct LdapConnectionManager {
    config: LdapConfig,
}

#[async_trait]
impl Manager for LdapConnectionManager {
    type Type = LdapConn;
    type Error = LdapError;
    
    async fn create(&self) -> Result<Self::Type, Self::Error> {
        let (conn, ldap) = LdapConnAsync::new(&self.config.url).await?;
        
        tokio::spawn(async move {
            conn.drive().await;
        });
        
        Ok(ldap)
    }
    
    async fn recycle(&self, conn: &mut Self::Type) -> RecycleResult<Self::Error> {
        // Test connection with whoami
        match conn.extended(WhoAmIRequest).await {
            Ok(_) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
```

### 12.2 Batch Operations

```rust
/// Batch credential operations for efficiency
pub struct CredentialBatchOperations {
    storage: Arc<dyn StorageProvider>,
    cache: Arc<CredentialCache>,
}

impl CredentialBatchOperations {
    /// Retrieve multiple credentials in parallel
    pub async fn batch_retrieve(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, Option<EncryptedData>>, StorageError> {
        let futures: Vec<_> = ids
            .iter()
            .map(|id| {
                let storage = self.storage.clone();
                let id = id.clone();
                async move {
                    let result = storage.retrieve(&id).await;
                    (id, result)
                }
            })
            .collect();
        
        let results = futures::future::join_all(futures).await;
        
        let mut map = HashMap::new();
        for (id, result) in results {
            match result {
                Ok(data) => {
                    map.insert(id, data);
                }
                Err(e) => return Err(e),
            }
        }
        
        Ok(map)
    }
    
    /// Store multiple credentials in parallel
    pub async fn batch_store(
        &self,
        credentials: Vec<(String, EncryptedData, CredentialMetadata)>,
    ) -> Result<(), StorageError> {
        let futures: Vec<_> = credentials
            .into_iter()
            .map(|(id, data, metadata)| {
                let storage = self.storage.clone();
                async move {
                    storage.store(&id, &data, &metadata).await
                }
            })
            .collect();
        
        futures::future::try_join_all(futures).await?;
        Ok(())
    }
}
```

### 12.3 Zero-Copy Optimizations

```rust
use bytes::{Bytes, BytesMut};

/// Zero-copy credential data holder
pub struct ZeroCopyCredential {
    data: Bytes,
    credential_type: String,
}

impl ZeroCopyCredential {
    /// Create from owned data
    pub fn new(data: Vec<u8>, credential_type: String) -> Self {
        Self {
            data: Bytes::from(data),
            credential_type,
        }
    }
    
    /// Get view of data without copying
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    
    /// Clone cheaply (reference counting)
    pub fn clone_cheap(&self) -> Self {
        Self {
            data: self.data.clone(), // Rc clone, not data copy
            credential_type: self.credential_type.clone(),
        }
    }
}
```

---

## 13. Error Handling

### 13.1 Error Type Hierarchy

```rust
use thiserror::Error;

/// Top-level credential error
#[derive(Error, Debug)]
pub enum CredentialError {
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    
    #[error("Credential not found: {0}")]
    NotFound(String),
    
    #[error("Credential expired")]
    Expired,
    
    #[error("Invalid credential format: {0}")]
    InvalidFormat(String),
    
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    
    #[error("Encryption error: {0}")]
    Encryption(#[from] CryptoError),
    
    #[error("OAuth2 error: {0}")]
    OAuth2(#[from] OAuth2Error),
    
    #[error("SAML error: {0}")]
    Saml(#[from] SamlError),
    
    #[error("LDAP error: {0}")]
    Ldap(#[from] LdapError),
    
    #[error("mTLS error: {0}")]
    Mtls(#[from] MtlsError),
    
    #[error("JWT error: {0}")]
    Jwt(#[from] JwtError),
    
    #[error("API Key error: {0}")]
    ApiKey(#[from] ApiKeyError),
    
    #[error("Kerberos error: {0}")]
    Kerberos(#[from] KerberosError),
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("Read failed: {0}")]
    ReadFailed(String),
    
    #[error("Write failed: {0}")]
    WriteFailed(String),
    
    #[error("Delete failed: {0}")]
    DeleteFailed(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
}

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Key derivation failed: {0}")]
    KeyDerivation(String),
    
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
    
    #[error("Invalid key: {0}")]
    InvalidKey(String),
    
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u8),
    
    #[error("No valid decryption key found")]
    NoValidKey,
}

#[derive(Error, Debug)]
pub enum OAuth2Error {
    #[error("Token exchange failed: {0:?}")]
    TokenExchangeFailed(OAuth2ErrorResponse),
    
    #[error("State mismatch (CSRF detected)")]
    StateMismatch,
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Missing client secret")]
    MissingClientSecret,
}

// Additional protocol-specific errors...
```

### 13.2 Error Recovery Strategies

```rust
/// Automatic retry with exponential backoff
pub async fn retry_with_backoff<F, T, E>(
    operation: F,
    max_attempts: usize,
) -> Result<T, E>
where
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
    E: std::error::Error,
{
    let mut attempt = 0;
    let mut backoff = Duration::from_millis(100);
    
    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if attempt < max_attempts => {
                attempt += 1;
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Circuit breaker for failing operations
pub struct CircuitBreaker {
    failure_threshold: usize,
    timeout: Duration,
    state: Arc<RwLock<CircuitState>>,
}

enum CircuitState {
    Closed { failures: usize },
    Open { opened_at: Instant },
    HalfOpen,
}

impl CircuitBreaker {
    pub async fn execute<F, T, E>(&self, operation: F) -> Result<T, E>
    where
        F: Future<Output = Result<T, E>>,
        E: std::error::Error,
    {
        // Check state
        let state = self.state.read().await;
        match *state {
            CircuitState::Open { opened_at } => {
                if opened_at.elapsed() > self.timeout {
                    drop(state);
                    let mut state = self.state.write().await;
                    *state = CircuitState::HalfOpen;
                } else {
                    return Err(/* CircuitOpen error */);
                }
            }
            _ => {}
        }
        drop(state);
        
        // Execute operation
        match operation.await {
            Ok(result) => {
                // Success - reset circuit
                let mut state = self.state.write().await;
                *state = CircuitState::Closed { failures: 0 };
                Ok(result)
            }
            Err(e) => {
                // Failure - increment counter
                let mut state = self.state.write().await;
                match *state {
                    CircuitState::Closed { failures } => {
                        if failures + 1 >= self.failure_threshold {
                            *state = CircuitState::Open {
                                opened_at: Instant::now(),
                            };
                        } else {
                            *state = CircuitState::Closed {
                                failures: failures + 1,
                            };
                        }
                    }
                    CircuitState::HalfOpen => {
                        *state = CircuitState::Open {
                            opened_at: Instant::now(),
                        };
                    }
                    _ => {}
                }
                Err(e)
            }
        }
    }
}
```

---

## Conclusion

This technical design document provides comprehensive low-level implementation details for the nebula-credential crate, covering:

✅ **Cryptography**: AES-256-GCM with Argon2id, nonce generation, key rotation  
✅ **OAuth2**: Authorization Code + PKCE, Client Credentials, Token Refresh, Device Code  
✅ **SAML 2.0**: AuthnRequest generation, Response validation, Signature verification  
✅ **LDAP/AD**: Bind authentication, Connection pooling, User search  
✅ **mTLS**: Certificate management, Rotation, HTTP client integration  
✅ **JWT**: Token generation/validation, Refresh tokens, RSA/HMAC support  
✅ **API Keys**: Generation with BLAKE3 hashing, Zero-downtime rotation, Rate limiting  
✅ **Kerberos**: TGT acquisition, Service ticket requests, Auto-renewal  
✅ **Storage**: Local (SQLite), AWS Secrets Manager, HashiCorp Vault, Azure Key Vault  
✅ **Caching**: Multi-level (memory + Redis), TTL management, Invalidation  
✅ **Observability**: Structured logging, Prometheus metrics, OpenTelemetry tracing  
✅ **Performance**: Connection pooling, Batch operations, Zero-copy optimizations  
✅ **Error Handling**: Type hierarchy, Retry with backoff, Circuit breaker  

**Next Steps:**
1. Create `data-model-code.md` with complete Rust type definitions
2. Create `security-spec.md` with threat model and mitigations
3. Update `spec.md` with architectural requirements

---

---

## 14. Credential Testing & Validation

### 14.1 Credential Test Trait

**Purpose**: Like n8n's credential test method, this allows validating credentials before using them in workflows.

```rust
/// Trait for testing credential validity
#[async_trait]
pub trait CredentialTest: Send + Sync {
    /// Test if credential is valid and functional
    /// 
    /// This method should perform a lightweight operation to verify:
    /// - Authentication succeeds
    /// - Required permissions are present
    /// - Credential hasn't expired
    /// - Network connectivity to service
    async fn test(&self) -> Result<TestResult, CredentialError>;
    
    /// Get human-readable test description
    fn test_description(&self) -> &str {
        "Testing credential validity"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    pub message: String,
    pub details: Option<TestDetails>,
    pub tested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestDetails {
    pub latency_ms: u64,
    pub endpoint_tested: String,
    pub permissions_verified: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl TestResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            details: None,
            tested_at: Utc::now(),
        }
    }
    
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            details: None,
            tested_at: Utc::now(),
        }
    }
    
    pub fn with_details(mut self, details: TestDetails) -> Self {
        self.details = Some(details);
        self
    }
}
```

### 14.2 Protocol-Specific Test Implementations

#### OAuth2 Credential Test

```rust
impl CredentialTest for OAuth2Credential {
    async fn test(&self) -> Result<TestResult, CredentialError> {
        let start = Instant::now();
        
        // Test 1: Validate token format
        if !self.is_token_format_valid() {
            return Ok(TestResult::failure("Invalid token format"));
        }
        
        // Test 2: Check token expiration
        if let Some(expires_at) = &self.expires_at {
            if Utc::now() > *expires_at {
                return Ok(TestResult::failure("Access token has expired"));
            }
        }
        
        // Test 3: Make test API call (if test endpoint configured)
        if let Some(test_endpoint) = &self.config.test_endpoint {
            match self.test_api_call(test_endpoint).await {
                Ok(response) => {
                    let latency = start.elapsed().as_millis() as u64;
                    
                    let details = TestDetails {
                        latency_ms: latency,
                        endpoint_tested: test_endpoint.clone(),
                        permissions_verified: self.extract_scopes_from_response(&response),
                        metadata: HashMap::from([
                            ("token_type".to_string(), json!(self.token_type)),
                            ("scopes".to_string(), json!(self.scopes)),
                        ]),
                    };
                    
                    Ok(TestResult::success("OAuth2 credential is valid")
                        .with_details(details))
                }
                Err(e) => {
                    Ok(TestResult::failure(format!("API test call failed: {}", e)))
                }
            }
        } else {
            // No test endpoint, just validate token presence
            Ok(TestResult::success("OAuth2 token is present and not expired"))
        }
    }
    
    fn test_description(&self) -> &str {
        "Testing OAuth2 token validity and permissions"
    }
}

impl OAuth2Credential {
    /// Test API call to verify token works
    async fn test_api_call(&self, endpoint: &str) -> Result<Response, OAuth2Error> {
        let client = reqwest::Client::new();
        
        let response = client
            .get(endpoint)
            .header("Authorization", format!("Bearer {}", self.access_token.expose()))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| OAuth2Error::NetworkError(e.to_string()))?;
        
        if !response.status().is_success() {
            return Err(OAuth2Error::TestCallFailed(
                response.status().as_u16(),
                response.text().await.unwrap_or_default(),
            ));
        }
        
        Ok(response)
    }
    
    fn is_token_format_valid(&self) -> bool {
        // Basic JWT format check (if JWT token)
        if self.access_token.expose().split('.').count() == 3 {
            return true;
        }
        
        // Or just check it's not empty
        !self.access_token.expose().is_empty()
    }
    
    fn extract_scopes_from_response(&self, response: &Response) -> Vec<String> {
        // Parse response to extract actual granted scopes
        // Implementation depends on OAuth provider
        self.scopes.clone()
    }
}
```

#### LDAP Credential Test

```rust
impl CredentialTest for LdapCredential {
    async fn test(&self) -> Result<TestResult, CredentialError> {
        let start = Instant::now();
        
        // Test 1: Attempt LDAP bind
        let mut ldap = match self.connection_pool.get().await {
            Ok(conn) => conn,
            Err(e) => {
                return Ok(TestResult::failure(format!("Failed to connect to LDAP server: {}", e)));
            }
        };
        
        // Test 2: Bind with admin credentials (if configured)
        if let Some(bind_dn) = &self.config.admin_bind_dn {
            if let Some(bind_password) = &self.config.admin_bind_password {
                match ldap.simple_bind(bind_dn, bind_password.expose()).await {
                    Ok(result) if result.success() => {
                        // Test 3: Perform test search
                        match self.test_search(&mut ldap).await {
                            Ok(_) => {
                                let latency = start.elapsed().as_millis() as u64;
                                
                                let details = TestDetails {
                                    latency_ms: latency,
                                    endpoint_tested: self.config.url.clone(),
                                    permissions_verified: vec!["bind".to_string(), "search".to_string()],
                                    metadata: HashMap::from([
                                        ("base_dn".to_string(), json!(self.config.base_dn)),
                                        ("tls_enabled".to_string(), json!(self.config.use_tls)),
                                    ]),
                                };
                                
                                Ok(TestResult::success("LDAP connection and authentication successful")
                                    .with_details(details))
                            }
                            Err(e) => {
                                Ok(TestResult::failure(format!("LDAP search test failed: {}", e)))
                            }
                        }
                    }
                    Ok(result) => {
                        Ok(TestResult::failure(format!("LDAP bind failed: {}", 
                            result.text().unwrap_or("Unknown error"))))
                    }
                    Err(e) => {
                        Ok(TestResult::failure(format!("LDAP bind error: {}", e)))
                    }
                }
            } else {
                Ok(TestResult::failure("Admin bind password not configured"))
            }
        } else {
            Ok(TestResult::failure("Admin bind DN not configured for testing"))
        }
    }
    
    fn test_description(&self) -> &str {
        "Testing LDAP connection, authentication, and search capabilities"
    }
}

impl LdapCredential {
    async fn test_search(&self, ldap: &mut LdapConn) -> Result<(), LdapError> {
        // Perform minimal search to verify permissions
        let (results, _) = ldap
            .search(
                &self.config.base_dn,
                Scope::Base,
                "(objectClass=*)",
                vec!["1.1"], // Request no attributes, just test search works
            )
            .await?
            .success()?;
        
        Ok(())
    }
}
```

#### API Key Credential Test

```rust
impl CredentialTest for ApiKeyCredential {
    async fn test(&self) -> Result<TestResult, CredentialError> {
        let start = Instant::now();
        
        // Test 1: Validate API key format
        if !self.is_key_format_valid() {
            return Ok(TestResult::failure("Invalid API key format"));
        }
        
        // Test 2: Check key expiration in metadata
        let metadata = match self.storage.get_api_key_by_hash(&self.key_hash).await {
            Ok(Some(meta)) => meta,
            Ok(None) => {
                return Ok(TestResult::failure("API key not found in storage"));
            }
            Err(e) => {
                return Ok(TestResult::failure(format!("Failed to retrieve key metadata: {}", e)));
            }
        };
        
        if let Some(expires_at) = metadata.expires_at {
            if Utc::now() > expires_at {
                return Ok(TestResult::failure("API key has expired"));
            }
        }
        
        // Test 3: Make test API call (if configured)
        if let Some(test_url) = &self.config.test_url {
            match self.test_api_request(test_url).await {
                Ok(_) => {
                    let latency = start.elapsed().as_millis() as u64;
                    
                    let details = TestDetails {
                        latency_ms: latency,
                        endpoint_tested: test_url.clone(),
                        permissions_verified: metadata.scopes.clone(),
                        metadata: HashMap::from([
                            ("key_id".to_string(), json!(metadata.id)),
                            ("owner_id".to_string(), json!(metadata.owner_id)),
                            ("usage_count".to_string(), json!(metadata.usage_count)),
                            ("last_used".to_string(), json!(metadata.last_used)),
                        ]),
                    };
                    
                    Ok(TestResult::success("API key is valid and functional")
                        .with_details(details))
                }
                Err(e) => {
                    Ok(TestResult::failure(format!("API test request failed: {}", e)))
                }
            }
        } else {
            // No test endpoint, just validate key exists and not expired
            Ok(TestResult::success("API key is present and valid"))
        }
    }
    
    fn test_description(&self) -> &str {
        "Testing API key validity and permissions"
    }
}

impl ApiKeyCredential {
    fn is_key_format_valid(&self) -> bool {
        let key = self.api_key.expose();
        
        // Check prefix matches expected format
        if let Some(expected_prefix) = &self.config.key_prefix {
            if !key.starts_with(expected_prefix) {
                return false;
            }
        }
        
        // Check minimum length
        key.len() >= 32
    }
    
    async fn test_api_request(&self, url: &str) -> Result<(), ApiKeyError> {
        let client = reqwest::Client::new();
        
        let response = client
            .get(url)
            .header(
                &self.config.header_name,
                self.api_key.expose(),
            )
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ApiKeyError::NetworkError(e.to_string()))?;
        
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ApiKeyError::TestRequestFailed(
                response.status().as_u16(),
                response.text().await.unwrap_or_default(),
            ))
        }
    }
}
```

#### mTLS Credential Test

```rust
impl CredentialTest for MtlsCredential {
    async fn test(&self) -> Result<TestResult, CredentialError> {
        let start = Instant::now();
        
        // Test 1: Validate certificate expiry
        match self.validate_expiry() {
            Ok(_) => {}
            Err(e) => {
                return Ok(TestResult::failure(format!("Certificate validation failed: {}", e)));
            }
        }
        
        // Test 2: Validate certificate chain
        match self.validate_chain() {
            Ok(_) => {}
            Err(e) => {
                return Ok(TestResult::failure(format!("Certificate chain validation failed: {}", e)));
            }
        }
        
        // Test 3: Make test TLS connection (if test endpoint configured)
        if let Some(test_endpoint) = &self.config.test_endpoint {
            let client = match MtlsHttpClient::new(self.clone()).await {
                Ok(c) => c,
                Err(e) => {
                    return Ok(TestResult::failure(format!("Failed to build mTLS client: {}", e)));
                }
            };
            
            match client.get(test_endpoint).await {
                Ok(response) if response.status().is_success() => {
                    let latency = start.elapsed().as_millis() as u64;
                    
                    // Extract certificate info
                    let subject = self.client_cert.subject_name()
                        .entries()
                        .map(|e| format!("{}={}", e.object().to_string(), e.data().as_utf8().unwrap()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    
                    let not_after = self.client_cert.not_after();
                    let expires_in_days = (not_after.to_system_time().unwrap()
                        .duration_since(SystemTime::now())
                        .unwrap()
                        .as_secs() / 86400) as i64;
                    
                    let details = TestDetails {
                        latency_ms: latency,
                        endpoint_tested: test_endpoint.clone(),
                        permissions_verified: vec!["client_auth".to_string()],
                        metadata: HashMap::from([
                            ("certificate_subject".to_string(), json!(subject)),
                            ("expires_in_days".to_string(), json!(expires_in_days)),
                            ("tls_version".to_string(), json!("TLS 1.3")),
                        ]),
                    };
                    
                    Ok(TestResult::success("mTLS certificate is valid and connection successful")
                        .with_details(details))
                }
                Ok(response) => {
                    Ok(TestResult::failure(format!("mTLS test request failed with status: {}", response.status())))
                }
                Err(e) => {
                    Ok(TestResult::failure(format!("mTLS connection failed: {}", e)))
                }
            }
        } else {
            // No test endpoint, just validate certificate
            Ok(TestResult::success("mTLS certificate is valid (no test endpoint configured)"))
        }
    }
    
    fn test_description(&self) -> &str {
        "Testing mTLS certificate validity and TLS handshake"
    }
}
```

### 14.3 Credential Test Executor

```rust
/// Executes credential tests with timeout and retry
pub struct CredentialTestExecutor {
    config: TestExecutorConfig,
}

impl CredentialTestExecutor {
    /// Execute credential test with timeout
    pub async fn execute_test<C: CredentialTest>(
        &self,
        credential: &C,
    ) -> Result<TestResult, CredentialError> {
        // Wrap test in timeout
        match tokio::time::timeout(
            self.config.timeout,
            credential.test(),
        ).await {
            Ok(result) => result,
            Err(_) => {
                Ok(TestResult::failure(format!(
                    "Credential test timed out after {:?}",
                    self.config.timeout
                )))
            }
        }
    }
    
    /// Execute test with retry on transient failures
    pub async fn execute_test_with_retry<C: CredentialTest>(
        &self,
        credential: &C,
    ) -> Result<TestResult, CredentialError> {
        let mut attempts = 0;
        let mut last_result = None;
        
        while attempts < self.config.max_retries {
            match self.execute_test(credential).await {
                Ok(result) if result.success => {
                    return Ok(result);
                }
                Ok(result) if self.is_transient_failure(&result) => {
                    // Transient failure, retry after backoff
                    last_result = Some(result);
                    attempts += 1;
                    
                    if attempts < self.config.max_retries {
                        let backoff = self.config.retry_backoff * attempts as u32;
                        tokio::time::sleep(backoff).await;
                    }
                }
                Ok(result) => {
                    // Permanent failure, don't retry
                    return Ok(result);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        
        // All retries exhausted
        Ok(last_result.unwrap_or_else(|| {
            TestResult::failure("All test attempts failed")
        }))
    }
    
    fn is_transient_failure(&self, result: &TestResult) -> bool {
        // Check if error is transient (network timeout, temporary unavailability, etc.)
        result.message.contains("timeout") ||
        result.message.contains("connection refused") ||
        result.message.contains("temporarily unavailable")
    }
}

pub struct TestExecutorConfig {
    pub timeout: Duration,
    pub max_retries: usize,
    pub retry_backoff: Duration,
}

impl Default for TestExecutorConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_retries: 3,
            retry_backoff: Duration::from_secs(2),
        }
    }
}
```

### 14.4 Automatic Testing on Credential Save

```rust
/// Credential manager with automatic testing
pub struct CredentialManager {
    storage: Arc<dyn StorageProvider>,
    test_executor: CredentialTestExecutor,
    config: CredentialManagerConfig,
}

impl CredentialManager {
    /// Save credential with optional test
    pub async fn save_credential<C: Credential + CredentialTest>(
        &self,
        id: &str,
        credential: &C,
        metadata: &CredentialMetadata,
    ) -> Result<SaveResult, CredentialError> {
        // Test credential before saving (if configured)
        if self.config.test_before_save {
            let test_result = self.test_executor.execute_test(credential).await?;
            
            if !test_result.success {
                return Err(CredentialError::TestFailed(test_result.message));
            }
        }
        
        // Encrypt and store
        let encrypted = self.encrypt_credential(credential)?;
        self.storage.store(id, &encrypted, metadata).await?;
        
        Ok(SaveResult {
            credential_id: id.to_string(),
            tested: self.config.test_before_save,
            test_result: None,
        })
    }
    
    /// Test existing credential
    pub async fn test_credential(
        &self,
        credential_id: &str,
    ) -> Result<TestResult, CredentialError> {
        // Retrieve credential
        let encrypted = self.storage
            .retrieve(credential_id)
            .await?
            .ok_or_else(|| CredentialError::NotFound(credential_id.to_string()))?;
        
        // Decrypt
        let credential = self.decrypt_credential(&encrypted)?;
        
        // Test
        self.test_executor.execute_test(&credential).await
    }
}

pub struct CredentialManagerConfig {
    pub test_before_save: bool,
    pub test_on_load: bool,
    pub cache_test_results: bool,
    pub test_cache_ttl: Duration,
}

impl Default for CredentialManagerConfig {
    fn default() -> Self {
        Self {
            test_before_save: true,
            test_on_load: false,
            cache_test_results: true,
            test_cache_ttl: Duration::from_secs(300), // 5 minutes
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveResult {
    pub credential_id: String,
    pub tested: bool,
    pub test_result: Option<TestResult>,
}
```

### 14.5 Testing Strategies

**Test Timing Options:**

1. **On Save** (n8n default):
   - Test when user saves/creates credential
   - Immediate feedback
   - Prevents saving invalid credentials

2. **On Load** (optional):
   - Test when credential loaded for workflow execution
   - Catches credentials that became invalid after save
   - Adds latency to workflow start

3. **On Demand**:
   - Manual test via API/UI
   - User-triggered validation
   - No automatic overhead

4. **Periodic Background**:
   - Scheduled testing of all credentials
   - Proactive detection of expiration/revocation
   - Requires background worker

**Implementation:**

```rust
/// Background credential health checker
pub struct CredentialHealthChecker {
    storage: Arc<dyn StorageProvider>,
    test_executor: CredentialTestExecutor,
    config: HealthCheckConfig,
}

impl CredentialHealthChecker {
    /// Start background health check loop
    pub async fn start_health_check_loop(self: Arc<Self>) {
        loop {
            tokio::time::sleep(self.config.check_interval).await;
            
            if let Err(e) = self.check_all_credentials().await {
                error!("Health check failed: {}", e);
            }
        }
    }
    
    async fn check_all_credentials(&self) -> Result<(), CredentialError> {
        // Get all credentials
        let credentials = self.storage.list(None).await?;
        
        for metadata in credentials {
            // Skip if checked recently
            if self.was_recently_checked(&metadata.id) {
                continue;
            }
            
            // Test credential
            match self.test_credential(&metadata.id).await {
                Ok(result) if !result.success => {
                    warn!(
                        "Credential {} failed health check: {}",
                        metadata.id, result.message
                    );
                    
                    // Notify owner
                    self.notify_credential_invalid(&metadata, &result).await;
                }
                Err(e) => {
                    error!(
                        "Failed to test credential {}: {}",
                        metadata.id, e
                    );
                }
                _ => {}
            }
        }
        
        Ok(())
    }
    
    async fn test_credential(&self, id: &str) -> Result<TestResult, CredentialError> {
        // Retrieve, decrypt, and test
        // Implementation similar to CredentialManager::test_credential
        todo!()
    }
    
    async fn notify_credential_invalid(
        &self,
        metadata: &CredentialMetadata,
        result: &TestResult,
    ) {
        // Send notification to credential owner
        // Via webhook, email, etc.
        info!(
            "Notifying owner {} about invalid credential {}",
            metadata.owner_id, metadata.id
        );
    }
    
    fn was_recently_checked(&self, id: &str) -> bool {
        // Check cache for recent test
        false
    }
}

pub struct HealthCheckConfig {
    pub enabled: bool,
    pub check_interval: Duration,
    pub notify_on_failure: bool,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_interval: Duration::from_secs(3600), // 1 hour
            notify_on_failure: true,
        }
    }
}
```

---

**Document Metadata:**
- **Lines of Code Examples:** 3,000+
- **Protocols Covered:** 8 (OAuth2, SAML, LDAP, mTLS, JWT, API Keys, Kerberos, Basic Auth)
- **Storage Providers:** 4 (Local, AWS, Vault, Azure)
- **Rust Features Used:** async/await, traits, generics, type-state pattern, zeroization, RwLock, Arc
- **Testing Strategies:** 4 (on-save, on-load, on-demand, background health checks)

