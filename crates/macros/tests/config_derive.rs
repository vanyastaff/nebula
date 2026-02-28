use nebula_macros::Config;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Config, Clone, Debug, Serialize, Deserialize)]
#[config(source = "env", prefix = "NEBULA_APP")]
#[validator(message = "app config invalid")]
struct AppConfig {
    #[validate(min = 1, max = 65535)]
    port: u16,

    #[validate(required, email)]
    admin_email: Option<String>,

    #[config(key = "NEBULA_SERVICE_NAME")]
    #[validate(min_length = 3)]
    service_name: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            admin_email: None,
            service_name: "nebula".to_string(),
        }
    }
}

fn clear_env() {
    unsafe {
        std::env::remove_var("NEBULA_APP_PORT");
        std::env::remove_var("NEBULA_APP_ADMIN_EMAIL");
        std::env::remove_var("NEBULA_SERVICE_NAME");
    }
}

#[test]
fn config_from_env_loads_and_validates() {
    let _guard = ENV_LOCK.lock().expect("env test lock poisoned");
    clear_env();
    unsafe {
        std::env::set_var("NEBULA_APP_PORT", "9090");
        std::env::set_var("NEBULA_APP_ADMIN_EMAIL", "admin@example.com");
        std::env::set_var("NEBULA_SERVICE_NAME", "gateway");
    }

    let cfg = AppConfig::from_env().expect("config should load");
    assert_eq!(cfg.port, 9090);
    assert_eq!(cfg.admin_email.as_deref(), Some("admin@example.com"));
    assert_eq!(cfg.service_name, "gateway");

    clear_env();
}

#[test]
fn config_from_env_fails_on_validate_rule() {
    let _guard = ENV_LOCK.lock().expect("env test lock poisoned");
    clear_env();
    unsafe {
        std::env::set_var("NEBULA_APP_PORT", "9090");
        std::env::set_var("NEBULA_APP_ADMIN_EMAIL", "bad-email");
        std::env::set_var("NEBULA_SERVICE_NAME", "gateway");
    }

    let err = AppConfig::from_env().expect_err("config should fail validation");
    assert!(err.contains("validation failed"));

    clear_env();
}

#[test]
fn config_from_env_uses_default_when_var_missing() {
    let _guard = ENV_LOCK.lock().expect("env test lock poisoned");
    clear_env();
    unsafe {
        std::env::set_var("NEBULA_APP_ADMIN_EMAIL", "admin@example.com");
    }

    let cfg = AppConfig::from_env().expect("config should load with defaults");
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.service_name, "nebula");
    assert_eq!(cfg.admin_email.as_deref(), Some("admin@example.com"));

    clear_env();
}

#[derive(Config, Clone, Debug, Serialize, Deserialize)]
#[config(
    sources = ["file", "env"],
    path = "target/config_derive_test.json",
    profile_var = "NEBULA_PROFILE",
    prefix = "NEBULA_APP"
)]
struct LayeredConfig {
    #[validate(min = 1, max = 65535)]
    #[config(default = 7777)]
    port: u16,
    #[validate(min_length = 3)]
    mode: String,
}

impl Default for LayeredConfig {
    fn default() -> Self {
        Self {
            port: 7000,
            mode: "default".to_string(),
        }
    }
}

#[derive(Config, Clone, Debug, Serialize, Deserialize)]
#[config(
    sources = ["dotenv", "env"],
    path = "target/config_derive_test.env",
    profile_var = "NEBULA_PROFILE",
    prefix = "NEBULA_APP"
)]
struct DotenvConfig {
    #[validate(min = 1, max = 65535)]
    port: u16,
    #[validate(min_length = 3)]
    mode: String,
}

impl Default for DotenvConfig {
    fn default() -> Self {
        Self {
            port: 7000,
            mode: "default".to_string(),
        }
    }
}

#[test]
fn config_load_with_profile_suffix_and_loader_precedence() {
    let _guard = ENV_LOCK.lock().expect("env test lock poisoned");
    clear_env();
    unsafe {
        std::env::set_var("NEBULA_PROFILE", "dev");
        std::env::set_var("NEBULA_APP_PORT", "9100");
    }

    std::fs::create_dir_all("target").expect("target dir");
    std::fs::write(
        "target/config_derive_test.json",
        r#"{"port": 8000, "mode": "base"}"#,
    )
    .expect("write base json");
    std::fs::write("target/config_derive_test.dev.json", r#"{"mode": "dev"}"#)
        .expect("write dev json");

    let cfg = LayeredConfig::load().expect("layered config should load");
    assert_eq!(cfg.port, 9100);
    assert_eq!(cfg.mode, "dev");

    let _ = std::fs::remove_file("target/config_derive_test.json");
    let _ = std::fs::remove_file("target/config_derive_test.dev.json");
    unsafe {
        std::env::remove_var("NEBULA_PROFILE");
        std::env::remove_var("NEBULA_APP_PORT");
    }
}

#[test]
fn config_load_supports_dotenv_loader_with_profile_suffix() {
    let _guard = ENV_LOCK.lock().expect("env test lock poisoned");
    clear_env();
    unsafe {
        std::env::set_var("NEBULA_PROFILE", "dev");
    }

    std::fs::create_dir_all("target").expect("target dir");
    std::fs::write(
        "target/config_derive_test.env",
        "NEBULA_APP_PORT=8200\nNEBULA_APP_MODE=base\n",
    )
    .expect("write base env");
    std::fs::write("target/config_derive_test.dev.env", "NEBULA_APP_MODE=dev\n")
        .expect("write dev env");

    let cfg = DotenvConfig::load().expect("dotenv config should load");
    assert_eq!(cfg.port, 8200);
    assert_eq!(cfg.mode, "dev");

    let _ = std::fs::remove_file("target/config_derive_test.env");
    let _ = std::fs::remove_file("target/config_derive_test.dev.env");
    unsafe {
        std::env::remove_var("NEBULA_PROFILE");
    }
}

#[derive(Config, Clone, Debug, Serialize, Deserialize)]
#[config(source = "env", prefix = "NEBULA_ALIAS")]
struct AliasConfig {
    #[config(name = "NEBULA_ALIAS_SERVICE", default = "fallback_service")]
    service: String,
}

impl Default for AliasConfig {
    fn default() -> Self {
        Self {
            service: "base".to_string(),
        }
    }
}

#[test]
fn config_field_name_alias_and_default_work() {
    let _guard = ENV_LOCK.lock().expect("env test lock poisoned");
    unsafe {
        std::env::remove_var("NEBULA_ALIAS_SERVICE");
    }

    let cfg = AliasConfig::from_env().expect("alias config should load");
    assert_eq!(cfg.service, "fallback_service");

    unsafe {
        std::env::set_var("NEBULA_ALIAS_SERVICE", "runtime");
    }
    let cfg = AliasConfig::from_env().expect("alias config should load");
    assert_eq!(cfg.service, "runtime");
    unsafe {
        std::env::remove_var("NEBULA_ALIAS_SERVICE");
    }
}
