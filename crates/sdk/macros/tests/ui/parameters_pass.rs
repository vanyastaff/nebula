//! Tests for the Parameters derive macro - successful cases.

use nebula_macros::Parameters;
include!("support.rs");

/// Database connection parameters.
#[derive(Parameters)]
pub struct DatabaseConfig {
    #[param(description = "Database host", required, default = "localhost")]
    host: String,
    
    #[param(description = "Port number", default = 5432)]
    port: u16,
    
    #[param(description = "Database name", required)]
    database: String,
    
    #[param(description = "Username", required)]
    username: String,
    
    #[param(description = "Password", secret, required)]
    password: String,
    
    #[param(description = "Enable SSL", default = true)]
    ssl: bool,
}

/// HTTP request parameters.
#[derive(Parameters)]
pub struct HttpRequestConfig {
    #[param(description = "Request URL", required, validation = "url")]
    url: String,
    
    #[param(
        description = "HTTP method",
        options = ["GET", "POST", "PUT", "DELETE", "PATCH"],
        default = "GET"
    )]
    method: String,
    
    #[param(description = "Timeout in seconds", default = 30)]
    timeout: u32,
    
    #[param(description = "Follow redirects", default = true)]
    follow_redirects: bool,
}

fn main() {
    let _db_config = DatabaseConfig {
        host: "localhost".to_string(),
        port: 5432,
        database: "nebula".to_string(),
        username: "user".to_string(),
        password: "secret".to_string(),
        ssl: false,
    };

    let _params = DatabaseConfig::parameters();
    let _count = DatabaseConfig::param_count();

    let _http_config = HttpRequestConfig {
        url: "https://example.com".to_string(),
        method: "GET".to_string(),
        timeout: 30,
        follow_redirects: true,
    };
}
