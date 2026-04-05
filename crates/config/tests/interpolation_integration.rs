//! Integration tests for environment variable interpolation

use nebula_config::{ConfigBuilder, ConfigSource};
use std::io::Write;

#[tokio::test]
async fn toml_with_env_interpolation() {
    unsafe {
        std::env::set_var("NEBULA_INTERP_IT_HOST", "prod.example.com");
        std::env::set_var("NEBULA_INTERP_IT_PORT", "5432");
    }

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    f.write_all(
        b"[server]\nhost = \"${NEBULA_INTERP_IT_HOST}\"\nport = \"${NEBULA_INTERP_IT_PORT}\"\n",
    )
    .unwrap();

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(f.path().to_path_buf()))
        .build()
        .await
        .expect("build with interpolation");

    assert_eq!(
        config.get::<String>("server.host").await.unwrap(),
        "prod.example.com"
    );
    assert_eq!(config.get::<String>("server.port").await.unwrap(), "5432");

    unsafe {
        std::env::remove_var("NEBULA_INTERP_IT_HOST");
        std::env::remove_var("NEBULA_INTERP_IT_PORT");
    }
}

#[tokio::test]
async fn interpolation_disabled_passes_through_literally() {
    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    f.write_all(b"val = \"${SOME_UNSET_VAR}\"\n").unwrap();

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(f.path().to_path_buf()))
        .with_interpolation(false)
        .build()
        .await
        .expect("build without interpolation");

    assert_eq!(
        config.get::<String>("val").await.unwrap(),
        "${SOME_UNSET_VAR}"
    );
}

#[tokio::test]
async fn json_defaults_with_interpolation() {
    unsafe { std::env::set_var("NEBULA_INTERP_IT_NAME", "nebula") };

    let config = ConfigBuilder::new()
        .with_defaults(serde_json::json!({
            "app": {
                "name": "${NEBULA_INTERP_IT_NAME}",
                "version": "1.0"
            }
        }))
        .build()
        .await
        .expect("build from JSON defaults with interpolation");

    assert_eq!(config.get::<String>("app.name").await.unwrap(), "nebula");
    // Non-interpolated value remains unchanged
    assert_eq!(config.get::<String>("app.version").await.unwrap(), "1.0");

    unsafe { std::env::remove_var("NEBULA_INTERP_IT_NAME") };
}

#[tokio::test]
async fn fallback_syntax_works_end_to_end() {
    unsafe { std::env::remove_var("NEBULA_INTERP_IT_MISSING") };

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    f.write_all(b"level = \"${NEBULA_INTERP_IT_MISSING:-info}\"\n")
        .unwrap();

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(f.path().to_path_buf()))
        .build()
        .await
        .expect("build with fallback");

    assert_eq!(config.get::<String>("level").await.unwrap(), "info");
}

#[tokio::test]
async fn recursive_interpolation_not_supported() {
    // ${${VAR}} should NOT be recursively expanded — single pass only.
    unsafe { std::env::set_var("NEBULA_INTERP_IT_INNER", "OUTER") };
    unsafe { std::env::set_var("OUTER", "resolved") };

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    f.write_all(b"val = \"${${NEBULA_INTERP_IT_INNER}}\"\n")
        .unwrap();

    // This should fail because `${NEBULA_INTERP_IT_INNER}` resolves to "OUTER"
    // but the result `${OUTER}` is NOT re-interpolated — so the literal
    // string should contain something unexpected or the parse itself may fail.
    // Since `${` inside another `${…}` is just part of the body, it resolves
    // the inner key literally as `${NEBULA_INTERP_IT_INNER}`.
    let result = ConfigBuilder::new()
        .with_source(ConfigSource::File(f.path().to_path_buf()))
        .build()
        .await;

    // The value should NOT be "resolved" — single pass means no recursion
    if let Ok(config) = result {
        let val = config.get::<String>("val").await.unwrap();
        assert_ne!(val, "resolved", "recursive interpolation should not happen");
    }
    // If it errors, that's also acceptable (malformed reference)

    unsafe {
        std::env::remove_var("NEBULA_INTERP_IT_INNER");
        std::env::remove_var("OUTER");
    }
}
