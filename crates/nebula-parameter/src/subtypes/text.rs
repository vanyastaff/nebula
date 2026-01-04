//! Text parameter subtypes for semantic validation and UI hints.
//!
//! This module defines subtypes that provide semantic meaning to text parameters,
//! enabling appropriate validation, transformation, and UI rendering.
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::core::subtype::TextSubtype;
//!
//! // Generic text
//! let subtype = TextSubtype::Generic;
//!
//! // Email with validation
//! let subtype = TextSubtype::Email;
//!
//! // Code with language specification
//! let subtype = TextSubtype::code_with_language(CodeLanguage::Rust);
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;

/// Semantic subtype for text parameters.
///
/// Subtypes provide hints for validation, transformation, and UI rendering.
/// They follow the pattern used by Blender's RNA system and Unreal Engine's UPROPERTY.
///
/// # Categories
///
/// - **Generic**: Basic text types (Generic, SingleLine, MultiLine, RichText)
/// - **Code**: Programming languages and formats (Code, Json, Xml, Yaml, etc.)
/// - **Web**: URLs, emails, domains (Email, Url, Hostname, etc.)
/// - **Files**: File paths and directories (FilePath, DirectoryPath, etc.)
/// - **Network**: IP addresses, MAC addresses (IpAddress, MacAddress, etc.)
/// - **Identifiers**: UUIDs, slugs, usernames (Uuid, Slug, Username, etc.)
/// - **Localization**: Locale, currency, country codes (Locale, CurrencyCode, etc.)
/// - **DateTime**: Date, time, duration, timezone (Date, Time, DateTime, etc.)
/// - **Queries**: XPath, CSS selectors, JSONPath (XPath, CssSelector, etc.)
/// - **Version Control**: Git refs, commits (GitRef, GitCommitSha, etc.)
/// - **DevOps**: Docker images, K8s resources (DockerImage, K8sResourceName, etc.)
/// - **Security**: Passwords, API keys, tokens (Secret, etc.)
/// - **Custom**: User-defined subtypes (Custom)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSubtype {
    // =========================================================================
    // Generic Text (4 variants)
    // =========================================================================
    /// Generic text with no specific format.
    ///
    /// # Use Cases
    /// - General-purpose text input
    /// - No validation required
    /// - Default subtype
    ///
    /// # UI Hint
    /// Single-line or multi-line text input depending on context.
    Generic,

    /// Single-line text (no newlines allowed).
    ///
    /// # Use Cases
    /// - Names, titles, labels
    /// - Short descriptive text
    /// - Form fields that should not wrap
    ///
    /// # UI Hint
    /// Single-line text input, newlines trimmed/rejected.
    SingleLine,

    /// Multi-line text (newlines allowed).
    ///
    /// # Use Cases
    /// - Descriptions, comments, notes
    /// - Documentation text
    /// - Long-form content
    ///
    /// # UI Hint
    /// Textarea or multi-line editor.
    MultiLine,

    /// Rich text with formatting (HTML/Markdown).
    ///
    /// # Use Cases
    /// - Blog posts, articles
    /// - Formatted documentation
    /// - Email bodies
    ///
    /// # UI Hint
    /// Rich text editor (WYSIWYG or Markdown).
    RichText,

    // =========================================================================
    // Code and Structured Data (15 variants)
    // =========================================================================
    /// Source code (language unspecified).
    ///
    /// # Use Cases
    /// - Generic code snippets
    /// - Scripts without specific language
    ///
    /// # UI Hint
    /// Code editor with syntax highlighting (auto-detect or plain).
    Code,

    /// Source code with specific language.
    ///
    /// # Use Cases
    /// - Language-specific validation
    /// - Syntax highlighting
    /// - Language-aware formatting
    ///
    /// # UI Hint
    /// Code editor with language-specific syntax highlighting.
    CodeWithLanguage(CodeLanguage),

    /// JSON data.
    ///
    /// # Use Cases
    /// - Configuration files
    /// - API payloads
    /// - Structured data
    ///
    /// # Validation
    /// Must be valid JSON.
    ///
    /// # UI Hint
    /// JSON editor with validation and formatting.
    Json,

    /// XML data.
    ///
    /// # Use Cases
    /// - XML documents
    /// - SOAP payloads
    /// - Legacy formats
    ///
    /// # Validation
    /// Must be well-formed XML.
    ///
    /// # UI Hint
    /// XML editor with validation.
    Xml,

    /// YAML data.
    ///
    /// # Use Cases
    /// - Configuration files
    /// - Docker compose
    /// - Kubernetes manifests
    ///
    /// # Validation
    /// Must be valid YAML.
    ///
    /// # UI Hint
    /// YAML editor with validation.
    Yaml,

    /// TOML data.
    ///
    /// # Use Cases
    /// - Rust Cargo.toml
    /// - Configuration files
    ///
    /// # Validation
    /// Must be valid TOML.
    ///
    /// # UI Hint
    /// TOML editor with validation.
    Toml,

    /// SQL query.
    ///
    /// # Use Cases
    /// - Database queries
    /// - SQL snippets
    ///
    /// # UI Hint
    /// SQL editor with syntax highlighting.
    SqlQuery,

    /// Regular expression pattern.
    ///
    /// # Use Cases
    /// - Pattern matching
    /// - Validation rules
    /// - Search/replace
    ///
    /// # Validation
    /// Must be valid regex syntax.
    ///
    /// # UI Hint
    /// Regex editor with syntax highlighting and testing.
    Regex,

    /// Template string (e.g., Handlebars, Jinja2).
    ///
    /// # Use Cases
    /// - Email templates
    /// - Document generation
    /// - Dynamic content
    ///
    /// # UI Hint
    /// Template editor with variable highlighting.
    Template,

    /// CSS stylesheet.
    ///
    /// # Use Cases
    /// - Custom styling
    /// - Theme definitions
    ///
    /// # Validation
    /// Valid CSS syntax.
    ///
    /// # UI Hint
    /// CSS editor with syntax highlighting.
    Css,

    /// HTML markup.
    ///
    /// # Use Cases
    /// - Web content
    /// - Email bodies
    ///
    /// # Validation
    /// Well-formed HTML.
    ///
    /// # UI Hint
    /// HTML editor or WYSIWYG.
    Html,

    /// Markdown text.
    ///
    /// # Use Cases
    /// - Documentation
    /// - README files
    /// - Formatted text
    ///
    /// # UI Hint
    /// Markdown editor with preview.
    Markdown,

    /// GraphQL query.
    ///
    /// # Use Cases
    /// - GraphQL API queries
    /// - Schema definitions
    ///
    /// # UI Hint
    /// GraphQL editor with validation.
    GraphQL,

    /// Shell command or script.
    ///
    /// # Use Cases
    /// - Command execution
    /// - Shell scripts
    ///
    /// # UI Hint
    /// Shell editor with syntax highlighting.
    Shell,

    /// Environment variable value.
    ///
    /// # Use Cases
    /// - Configuration
    /// - Deployment settings
    ///
    /// # UI Hint
    /// Plain text, possibly with variable expansion preview.
    EnvVar,

    // =========================================================================
    // Web and Network (10 variants)
    // =========================================================================
    /// Email address.
    ///
    /// # Use Cases
    /// - User registration
    /// - Contact information
    /// - Notifications
    ///
    /// # Validation
    /// Valid email format (RFC 5322).
    ///
    /// # UI Hint
    /// Email input with validation.
    ///
    /// # Example
    /// ```text
    /// user@example.com
    /// ```
    Email,

    /// URL (absolute or relative).
    ///
    /// # Use Cases
    /// - Links
    /// - API endpoints
    /// - Resources
    ///
    /// # Validation
    /// Valid URL format.
    ///
    /// # UI Hint
    /// URL input with validation and preview.
    Url,

    /// Absolute URL (must include scheme).
    ///
    /// # Use Cases
    /// - External links
    /// - API endpoints
    ///
    /// # Validation
    /// Must start with scheme (http://, https://, etc.).
    ///
    /// # Example
    /// ```text
    /// https://example.com/path
    /// ```
    UrlAbsolute,

    /// Relative URL (no scheme).
    ///
    /// # Use Cases
    /// - Internal links
    /// - Path references
    ///
    /// # Example
    /// ```text
    /// /path/to/resource
    /// ../relative/path
    /// ```
    UrlRelative,

    /// Hostname or domain name.
    ///
    /// # Use Cases
    /// - Server configuration
    /// - DNS records
    ///
    /// # Validation
    /// Valid hostname format (RFC 1123).
    ///
    /// # Example
    /// ```text
    /// example.com
    /// subdomain.example.com
    /// ```
    Hostname,

    /// Domain name.
    ///
    /// # Use Cases
    /// - Domain configuration
    /// - DNS settings
    ///
    /// # Example
    /// ```text
    /// example.com
    /// ```
    DomainName,

    /// IP address (v4 or v6).
    ///
    /// # Use Cases
    /// - Network configuration
    /// - Server addresses
    ///
    /// # Validation
    /// Valid IPv4 or IPv6 format.
    IpAddress,

    /// IPv4 address.
    ///
    /// # Validation
    /// Valid IPv4 format (dotted decimal).
    ///
    /// # Example
    /// ```text
    /// 192.168.1.1
    /// ```
    IpV4Address,

    /// IPv6 address.
    ///
    /// # Validation
    /// Valid IPv6 format (colon-separated hex).
    ///
    /// # Example
    /// ```text
    /// 2001:0db8::1
    /// ```
    IpV6Address,

    /// MAC address.
    ///
    /// # Use Cases
    /// - Network hardware
    /// - Device identification
    ///
    /// # Validation
    /// Valid MAC format (colon or hyphen separated).
    ///
    /// # Example
    /// ```text
    /// 00:1A:2B:3C:4D:5E
    /// ```
    MacAddress,

    // =========================================================================
    // File System (5 variants)
    // =========================================================================
    /// File path (absolute or relative).
    ///
    /// # Use Cases
    /// - File selection
    /// - Path configuration
    ///
    /// # UI Hint
    /// File picker dialog.
    FilePath,

    /// Absolute file path.
    ///
    /// # Use Cases
    /// - System paths
    /// - Deployment paths
    ///
    /// # Example
    /// ```text
    /// /usr/local/bin/app
    /// C:\Program Files\App\
    /// ```
    FilePathAbsolute,

    /// Relative file path.
    ///
    /// # Use Cases
    /// - Project-relative paths
    /// - Working directory references
    ///
    /// # Example
    /// ```text
    /// ./config/app.json
    /// ../data/input.csv
    /// ```
    FilePathRelative,

    /// Directory path.
    ///
    /// # Use Cases
    /// - Folder selection
    /// - Output directories
    ///
    /// # UI Hint
    /// Directory picker dialog.
    DirectoryPath,

    /// File extension.
    ///
    /// # Use Cases
    /// - File type filtering
    /// - Extension validation
    ///
    /// # Example
    /// ```text
    /// .json
    /// .txt
    /// ```
    FileExtension,

    // =========================================================================
    // Identifiers and Names (8 variants)
    // =========================================================================
    /// UUID (Universally Unique Identifier).
    ///
    /// # Use Cases
    /// - Unique identifiers
    /// - Primary keys
    /// - Session IDs
    ///
    /// # Validation
    /// Valid UUID format (RFC 4122).
    ///
    /// # Example
    /// ```text
    /// 550e8400-e29b-41d4-a716-446655440000
    /// ```
    Uuid,

    /// URL-friendly slug.
    ///
    /// # Use Cases
    /// - URL segments
    /// - SEO-friendly identifiers
    ///
    /// # Validation
    /// Lowercase letters, numbers, hyphens only.
    ///
    /// # Example
    /// ```text
    /// my-blog-post
    /// product-name-123
    /// ```
    Slug,

    /// Username.
    ///
    /// # Use Cases
    /// - User accounts
    /// - Login credentials
    ///
    /// # Validation
    /// Alphanumeric, underscores, hyphens typically.
    ///
    /// # Example
    /// ```text
    /// john_doe
    /// user123
    /// ```
    Username,

    /// Password or secret.
    ///
    /// # Use Cases
    /// - Authentication
    /// - API keys
    /// - Sensitive data
    ///
    /// # UI Hint
    /// Password input (masked).
    Secret,

    /// MIME type.
    ///
    /// # Use Cases
    /// - Content type specification
    /// - File type identification
    ///
    /// # Validation
    /// Valid MIME type format.
    ///
    /// # Example
    /// ```text
    /// application/json
    /// text/html
    /// ```
    MimeType,

    /// Phone number.
    ///
    /// # Use Cases
    /// - Contact information
    /// - SMS notifications
    ///
    /// # Validation
    /// Valid phone number format (E.164 recommended).
    ///
    /// # Example
    /// ```text
    /// +1-555-123-4567
    /// ```
    PhoneNumber,

    /// Credit card number.
    ///
    /// # Use Cases
    /// - Payment processing
    ///
    /// # Validation
    /// Luhn algorithm validation.
    ///
    /// # Security
    /// Should be transmitted securely (HTTPS, encryption).
    ///
    /// # UI Hint
    /// Masked input, formatted with spaces.
    CreditCard,

    /// Hex color code.
    ///
    /// # Use Cases
    /// - Color selection
    /// - Styling
    ///
    /// # Validation
    /// Valid hex color format (#RGB or #RRGGBB).
    ///
    /// # Example
    /// ```text
    /// #FF5733
    /// #F00
    /// ```
    HexColor,

    // =========================================================================
    // Date and Time (5 variants)
    // =========================================================================
    /// Date (without time).
    ///
    /// # Use Cases
    /// - Birthdays
    /// - Deadlines
    /// - Event dates
    ///
    /// # Format
    /// ISO 8601 date (YYYY-MM-DD) recommended.
    ///
    /// # Example
    /// ```text
    /// 2024-03-15
    /// ```
    Date,

    /// Time (without date).
    ///
    /// # Use Cases
    /// - Scheduling
    /// - Time of day
    ///
    /// # Format
    /// ISO 8601 time (HH:MM:SS) recommended.
    ///
    /// # Example
    /// ```text
    /// 14:30:00
    /// ```
    Time,

    /// Date and time.
    ///
    /// # Use Cases
    /// - Timestamps
    /// - Event scheduling
    ///
    /// # Format
    /// ISO 8601 datetime recommended.
    ///
    /// # Example
    /// ```text
    /// 2024-03-15T14:30:00Z
    /// ```
    DateTime,

    /// Duration or time span.
    ///
    /// # Use Cases
    /// - Timeouts
    /// - Intervals
    /// - Session length
    ///
    /// # Format
    /// ISO 8601 duration or human-readable format.
    ///
    /// # Example
    /// ```text
    /// PT1H30M (1 hour 30 minutes)
    /// 1h 30m
    /// ```
    Duration,

    /// Timezone identifier.
    ///
    /// # Use Cases
    /// - Timezone selection
    /// - Time conversion
    ///
    /// # Format
    /// IANA timezone database name.
    ///
    /// # Example
    /// ```text
    /// America/New_York
    /// Europe/London
    /// ```
    Timezone,

    // =========================================================================
    // Localization (4 variants)
    // =========================================================================
    /// Locale identifier.
    ///
    /// # Use Cases
    /// - Internationalization
    /// - Language selection
    ///
    /// # Format
    /// BCP 47 language tag.
    ///
    /// # Example
    /// ```text
    /// en-US
    /// fr-FR
    /// zh-CN
    /// ```
    Locale,

    /// Currency code.
    ///
    /// # Use Cases
    /// - Financial applications
    /// - E-commerce
    ///
    /// # Format
    /// ISO 4217 currency code.
    ///
    /// # Example
    /// ```text
    /// USD
    /// EUR
    /// JPY
    /// ```
    CurrencyCode,

    /// Country code.
    ///
    /// # Use Cases
    /// - Geographic data
    /// - Shipping addresses
    ///
    /// # Format
    /// ISO 3166-1 alpha-2 code.
    ///
    /// # Example
    /// ```text
    /// US
    /// GB
    /// FR
    /// ```
    CountryCode,

    /// Language code.
    ///
    /// # Use Cases
    /// - Language selection
    /// - Content localization
    ///
    /// # Format
    /// ISO 639-1 language code.
    ///
    /// # Example
    /// ```text
    /// en
    /// fr
    /// de
    /// ```
    LanguageCode,

    // =========================================================================
    // Query Languages (5 variants)
    // =========================================================================
    /// CSS selector.
    ///
    /// # Use Cases
    /// - Web scraping
    /// - DOM querying
    ///
    /// # Example
    /// ```text
    /// div.container > p.text
    /// ```
    CssSelector,

    /// XPath expression.
    ///
    /// # Use Cases
    /// - XML querying
    /// - Web scraping
    ///
    /// # Example
    /// ```text
    /// //div[@class='container']/p
    /// ```
    XPath,

    /// JSONPath expression.
    ///
    /// # Use Cases
    /// - JSON querying
    /// - Data extraction
    ///
    /// # Example
    /// ```text
    /// $.store.book[*].author
    /// ```
    JsonPath,

    /// JMESPath expression.
    ///
    /// # Use Cases
    /// - JSON querying (AWS CLI, etc.)
    /// - Data transformation
    ///
    /// # Example
    /// ```text
    /// people[?age > `20`].name
    /// ```
    JmesPath,

    /// Cron expression.
    ///
    /// # Use Cases
    /// - Job scheduling
    /// - Periodic tasks
    ///
    /// # Example
    /// ```text
    /// 0 0 * * * (daily at midnight)
    /// */5 * * * * (every 5 minutes)
    /// ```
    CronExpression,

    // =========================================================================
    // Version Control and DevOps (5 variants)
    // =========================================================================
    /// Semantic version.
    ///
    /// # Use Cases
    /// - Software versioning
    /// - Dependency management
    ///
    /// # Format
    /// SemVer 2.0 (MAJOR.MINOR.PATCH).
    ///
    /// # Example
    /// ```text
    /// 1.2.3
    /// 2.0.0-beta.1
    /// ```
    SemVer,

    /// Git reference (branch, tag, commit).
    ///
    /// # Use Cases
    /// - Repository references
    /// - CI/CD pipelines
    ///
    /// # Example
    /// ```text
    /// main
    /// refs/heads/feature-branch
    /// v1.0.0
    /// ```
    GitRef,

    /// Git commit SHA.
    ///
    /// # Use Cases
    /// - Specific commit references
    /// - Deployment tracking
    ///
    /// # Example
    /// ```text
    /// a3b2c1d4e5f6
    /// ```
    GitCommitSha,

    /// Docker image reference.
    ///
    /// # Use Cases
    /// - Container deployment
    /// - Image management
    ///
    /// # Example
    /// ```text
    /// nginx:latest
    /// myregistry.com/myapp:1.2.3
    /// ```
    DockerImage,

    /// Kubernetes resource name.
    ///
    /// # Use Cases
    /// - K8s deployments
    /// - Resource management
    ///
    /// # Validation
    /// Valid K8s resource name (RFC 1123 subdomain).
    ///
    /// # Example
    /// ```text
    /// my-deployment
    /// app-service-prod
    /// ```
    K8sResourceName,

    // =========================================================================
    // Custom (1 variant)
    // =========================================================================
    /// Custom subtype defined by the user.
    ///
    /// # Use Cases
    /// - Domain-specific types
    /// - Application-specific validation
    ///
    /// # Example
    /// ```rust
    /// TextSubtype::Custom("invoice_number".into())
    /// TextSubtype::Custom("sku".into())
    /// ```
    Custom(String),
}

impl TextSubtype {
    /// Create a Code subtype with specified language.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::subtype::{TextSubtype, CodeLanguage};
    ///
    /// let subtype = TextSubtype::code_with_language(CodeLanguage::Rust);
    /// ```
    #[must_use]
    pub fn code_with_language(language: CodeLanguage) -> Self {
        Self::CodeWithLanguage(language)
    }

    /// Create a Custom subtype.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::subtype::TextSubtype;
    ///
    /// let subtype = TextSubtype::custom("invoice_number");
    /// ```
    #[must_use]
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }

    /// Check if this subtype represents code.
    ///
    /// Returns `true` for `Code`, `CodeWithLanguage`, and structured data formats.
    #[must_use]
    pub fn is_code(&self) -> bool {
        matches!(
            self,
            Self::Code
                | Self::CodeWithLanguage(_)
                | Self::Json
                | Self::Xml
                | Self::Yaml
                | Self::Toml
                | Self::SqlQuery
                | Self::Css
                | Self::Html
                | Self::Markdown
                | Self::GraphQL
                | Self::Shell
        )
    }

    /// Check if this subtype requires special security handling.
    #[must_use]
    pub fn is_sensitive(&self) -> bool {
        matches!(self, Self::Secret | Self::CreditCard)
    }

    /// Check if this subtype represents a structured format (JSON, XML, YAML, etc.).
    #[must_use]
    pub fn is_structured(&self) -> bool {
        matches!(self, Self::Json | Self::Xml | Self::Yaml | Self::Toml)
    }

    /// Check if this subtype represents a file path.
    #[must_use]
    pub fn is_file_path(&self) -> bool {
        matches!(
            self,
            Self::FilePath | Self::FilePathAbsolute | Self::FilePathRelative | Self::DirectoryPath
        )
    }

    /// Get the MIME type hint for this subtype, if applicable.
    ///
    /// Returns the recommended MIME type for content of this subtype.
    #[must_use]
    pub fn mime_type_hint(&self) -> Option<&'static str> {
        match self {
            Self::Json => Some("application/json"),
            Self::Xml => Some("application/xml"),
            Self::Yaml => Some("application/yaml"),
            Self::Toml => Some("application/toml"),
            Self::Html => Some("text/html"),
            Self::Css => Some("text/css"),
            Self::Markdown => Some("text/markdown"),
            Self::Regex => Some("text/plain"),
            Self::SqlQuery => Some("application/sql"),
            _ => None,
        }
    }
}

impl Default for TextSubtype {
    fn default() -> Self {
        Self::Generic
    }
}

impl fmt::Display for TextSubtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generic => write!(f, "generic"),
            Self::SingleLine => write!(f, "single_line"),
            Self::MultiLine => write!(f, "multi_line"),
            Self::RichText => write!(f, "rich_text"),
            Self::Code => write!(f, "code"),
            Self::CodeWithLanguage(lang) => write!(f, "code({})", lang),
            Self::Json => write!(f, "json"),
            Self::Xml => write!(f, "xml"),
            Self::Yaml => write!(f, "yaml"),
            Self::Toml => write!(f, "toml"),
            Self::SqlQuery => write!(f, "sql"),
            Self::Regex => write!(f, "regex"),
            Self::Template => write!(f, "template"),
            Self::Css => write!(f, "css"),
            Self::Html => write!(f, "html"),
            Self::Markdown => write!(f, "markdown"),
            Self::GraphQL => write!(f, "graphql"),
            Self::Shell => write!(f, "shell"),
            Self::EnvVar => write!(f, "env_var"),
            Self::Email => write!(f, "email"),
            Self::Url => write!(f, "url"),
            Self::UrlAbsolute => write!(f, "url_absolute"),
            Self::UrlRelative => write!(f, "url_relative"),
            Self::Hostname => write!(f, "hostname"),
            Self::DomainName => write!(f, "domain"),
            Self::IpAddress => write!(f, "ip_address"),
            Self::IpV4Address => write!(f, "ipv4"),
            Self::IpV6Address => write!(f, "ipv6"),
            Self::MacAddress => write!(f, "mac_address"),
            Self::FilePath => write!(f, "file_path"),
            Self::FilePathAbsolute => write!(f, "file_path_absolute"),
            Self::FilePathRelative => write!(f, "file_path_relative"),
            Self::DirectoryPath => write!(f, "directory_path"),
            Self::FileExtension => write!(f, "file_extension"),
            Self::Uuid => write!(f, "uuid"),
            Self::Slug => write!(f, "slug"),
            Self::Username => write!(f, "username"),
            Self::Secret => write!(f, "secret"),
            Self::MimeType => write!(f, "mime_type"),
            Self::PhoneNumber => write!(f, "phone_number"),
            Self::CreditCard => write!(f, "credit_card"),
            Self::HexColor => write!(f, "hex_color"),
            Self::Date => write!(f, "date"),
            Self::Time => write!(f, "time"),
            Self::DateTime => write!(f, "datetime"),
            Self::Duration => write!(f, "duration"),
            Self::Timezone => write!(f, "timezone"),
            Self::Locale => write!(f, "locale"),
            Self::CurrencyCode => write!(f, "currency_code"),
            Self::CountryCode => write!(f, "country_code"),
            Self::LanguageCode => write!(f, "language_code"),
            Self::CssSelector => write!(f, "css_selector"),
            Self::XPath => write!(f, "xpath"),
            Self::JsonPath => write!(f, "jsonpath"),
            Self::JmesPath => write!(f, "jmespath"),
            Self::CronExpression => write!(f, "cron"),
            Self::SemVer => write!(f, "semver"),
            Self::GitRef => write!(f, "git_ref"),
            Self::GitCommitSha => write!(f, "git_sha"),
            Self::DockerImage => write!(f, "docker_image"),
            Self::K8sResourceName => write!(f, "k8s_resource"),
            Self::Custom(name) => write!(f, "custom({})", name),
        }
    }
}

// =============================================================================
// CodeLanguage
// =============================================================================

/// Programming language for code subtypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeLanguage {
    /// Rust
    Rust,
    /// Python
    Python,
    /// JavaScript
    JavaScript,
    /// TypeScript
    TypeScript,
    /// Go
    Go,
    /// Java
    Java,
    /// C
    C,
    /// C++
    #[serde(rename = "cpp")]
    Cpp,
    /// C#
    #[serde(rename = "csharp")]
    CSharp,
    /// Ruby
    Ruby,
    /// PHP
    PHP,
    /// Swift
    Swift,
    /// Kotlin
    Kotlin,
    /// Scala
    Scala,
    /// Haskell
    Haskell,
    /// Elixir
    Elixir,
    /// Erlang
    Erlang,
    /// Clojure
    Clojure,
    /// Lua
    Lua,
    /// R
    R,
    /// MATLAB
    Matlab,
    /// SQL
    Sql,
    /// Shell (Bash, sh, zsh)
    Shell,
    /// PowerShell
    PowerShell,
    /// Perl
    Perl,
    /// Dart
    Dart,
    /// Zig
    Zig,
    /// V
    V,
    /// Nim
    Nim,
    /// Crystal
    Crystal,
    /// OCaml
    OCaml,
    /// F#
    #[serde(rename = "fsharp")]
    FSharp,
    /// Fortran
    Fortran,
    /// COBOL
    Cobol,
    /// Assembly
    Assembly,
    /// WebAssembly (WAT)
    WebAssembly,
    /// Custom language
    Custom(String),
}

impl CodeLanguage {
    /// Get file extension for this language.
    #[must_use]
    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::Python => "py",
            Self::JavaScript => "js",
            Self::TypeScript => "ts",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "cs",
            Self::Ruby => "rb",
            Self::PHP => "php",
            Self::Swift => "swift",
            Self::Kotlin => "kt",
            Self::Scala => "scala",
            Self::Haskell => "hs",
            Self::Elixir => "ex",
            Self::Erlang => "erl",
            Self::Clojure => "clj",
            Self::Lua => "lua",
            Self::R => "r",
            Self::Matlab => "m",
            Self::Sql => "sql",
            Self::Shell => "sh",
            Self::PowerShell => "ps1",
            Self::Perl => "pl",
            Self::Dart => "dart",
            Self::Zig => "zig",
            Self::V => "v",
            Self::Nim => "nim",
            Self::Crystal => "cr",
            Self::OCaml => "ml",
            Self::FSharp => "fs",
            Self::Fortran => "f90",
            Self::Cobol => "cob",
            Self::Assembly => "asm",
            Self::WebAssembly => "wat",
            Self::Custom(_) => "txt",
        }
    }

    /// Get language name as string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "c++",
            Self::CSharp => "c#",
            Self::Ruby => "ruby",
            Self::PHP => "php",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Scala => "scala",
            Self::Haskell => "haskell",
            Self::Elixir => "elixir",
            Self::Erlang => "erlang",
            Self::Clojure => "clojure",
            Self::Lua => "lua",
            Self::R => "r",
            Self::Matlab => "matlab",
            Self::Sql => "sql",
            Self::Shell => "shell",
            Self::PowerShell => "powershell",
            Self::Perl => "perl",
            Self::Dart => "dart",
            Self::Zig => "zig",
            Self::V => "v",
            Self::Nim => "nim",
            Self::Crystal => "crystal",
            Self::OCaml => "ocaml",
            Self::FSharp => "f#",
            Self::Fortran => "fortran",
            Self::Cobol => "cobol",
            Self::Assembly => "assembly",
            Self::WebAssembly => "webassembly",
            Self::Custom(name) => name,
        }
    }
}

impl fmt::Display for CodeLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        assert_eq!(TextSubtype::default(), TextSubtype::Generic);
    }

    #[test]
    fn test_is_code() {
        assert!(TextSubtype::Code.is_code());
        assert!(TextSubtype::CodeWithLanguage(CodeLanguage::Rust).is_code());
        assert!(TextSubtype::Json.is_code());
        assert!(TextSubtype::Xml.is_code());
        assert!(!TextSubtype::Email.is_code());
        assert!(!TextSubtype::Generic.is_code());
    }

    #[test]
    fn test_is_sensitive() {
        assert!(TextSubtype::Secret.is_sensitive());
        assert!(TextSubtype::CreditCard.is_sensitive());
        assert!(!TextSubtype::Email.is_sensitive());
        assert!(!TextSubtype::Generic.is_sensitive());
    }

    #[test]
    fn test_is_structured() {
        assert!(TextSubtype::Json.is_structured());
        assert!(TextSubtype::Xml.is_structured());
        assert!(TextSubtype::Yaml.is_structured());
        assert!(TextSubtype::Toml.is_structured());
        assert!(!TextSubtype::Code.is_structured());
        assert!(!TextSubtype::Email.is_structured());
    }

    #[test]
    fn test_is_file_path() {
        assert!(TextSubtype::FilePath.is_file_path());
        assert!(TextSubtype::FilePathAbsolute.is_file_path());
        assert!(TextSubtype::FilePathRelative.is_file_path());
        assert!(TextSubtype::DirectoryPath.is_file_path());
        assert!(!TextSubtype::Generic.is_file_path());
    }

    #[test]
    fn test_mime_type_hint() {
        assert_eq!(TextSubtype::Json.mime_type_hint(), Some("application/json"));
        assert_eq!(TextSubtype::Xml.mime_type_hint(), Some("application/xml"));
        assert_eq!(TextSubtype::Yaml.mime_type_hint(), Some("application/yaml"));
        assert_eq!(TextSubtype::Html.mime_type_hint(), Some("text/html"));
        assert_eq!(TextSubtype::Email.mime_type_hint(), None);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", TextSubtype::Generic), "generic");
        assert_eq!(format!("{}", TextSubtype::Email), "email");
        assert_eq!(format!("{}", TextSubtype::Json), "json");
        assert_eq!(
            format!("{}", TextSubtype::CodeWithLanguage(CodeLanguage::Rust)),
            "code(rust)"
        );
        assert_eq!(
            format!("{}", TextSubtype::Custom("invoice".into())),
            "custom(invoice)"
        );
    }

    #[test]
    fn test_serialization() {
        let subtype = TextSubtype::Email;
        let json = serde_json::to_string(&subtype).unwrap();
        assert_eq!(json, "\"email\"");

        let deserialized: TextSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, TextSubtype::Email);
    }

    #[test]
    fn test_code_with_language_serialization() {
        let subtype = TextSubtype::CodeWithLanguage(CodeLanguage::Rust);
        let json = serde_json::to_string(&subtype).unwrap();
        let deserialized: TextSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, subtype);
    }

    #[test]
    fn test_custom_serialization() {
        let subtype = TextSubtype::Custom("invoice_number".into());
        let json = serde_json::to_string(&subtype).unwrap();
        let deserialized: TextSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, subtype);
    }

    #[test]
    fn test_code_language_file_extension() {
        assert_eq!(CodeLanguage::Rust.file_extension(), "rs");
        assert_eq!(CodeLanguage::Python.file_extension(), "py");
        assert_eq!(CodeLanguage::JavaScript.file_extension(), "js");
        assert_eq!(CodeLanguage::TypeScript.file_extension(), "ts");
    }

    #[test]
    fn test_code_language_as_str() {
        assert_eq!(CodeLanguage::Rust.as_str(), "rust");
        assert_eq!(CodeLanguage::Python.as_str(), "python");
        assert_eq!(CodeLanguage::CSharp.as_str(), "c#");
        assert_eq!(CodeLanguage::Cpp.as_str(), "c++");
    }

    #[test]
    fn test_code_language_display() {
        assert_eq!(format!("{}", CodeLanguage::Rust), "rust");
        assert_eq!(format!("{}", CodeLanguage::Python), "python");
        assert_eq!(format!("{}", CodeLanguage::JavaScript), "javascript");
    }

    #[test]
    fn test_code_language_serialization() {
        let lang = CodeLanguage::Rust;
        let json = serde_json::to_string(&lang).unwrap();
        assert_eq!(json, "\"rust\"");

        let deserialized: CodeLanguage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CodeLanguage::Rust);
    }

    #[test]
    fn test_all_variants_are_unique() {
        // Ensure Display produces unique strings for all variants
        use std::collections::HashSet;

        let variants = vec![
            TextSubtype::Generic,
            TextSubtype::SingleLine,
            TextSubtype::MultiLine,
            TextSubtype::RichText,
            TextSubtype::Code,
            TextSubtype::Json,
            TextSubtype::Xml,
            TextSubtype::Email,
            TextSubtype::Url,
            TextSubtype::FilePath,
            TextSubtype::Uuid,
            TextSubtype::Date,
            TextSubtype::Locale,
            TextSubtype::CssSelector,
            TextSubtype::SemVer,
        ];

        let strings: HashSet<String> = variants.iter().map(ToString::to_string).collect();
        assert_eq!(strings.len(), variants.len());
    }

    #[test]
    fn test_clone() {
        let subtype = TextSubtype::Email;
        let cloned = subtype.clone();
        assert_eq!(subtype, cloned);
    }

    #[test]
    fn test_eq() {
        assert_eq!(TextSubtype::Email, TextSubtype::Email);
        assert_ne!(TextSubtype::Email, TextSubtype::Url);
        assert_eq!(
            TextSubtype::CodeWithLanguage(CodeLanguage::Rust),
            TextSubtype::CodeWithLanguage(CodeLanguage::Rust)
        );
        assert_ne!(
            TextSubtype::CodeWithLanguage(CodeLanguage::Rust),
            TextSubtype::CodeWithLanguage(CodeLanguage::Python)
        );
    }
}
