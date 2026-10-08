//! Public configuration diagnostics must not reveal connection secrets.

use nebula_core::ArtifactSetDigest;
use nebula_worker_bin::config::WorkerConfig;

#[test]
fn worker_config_debug_redacts_the_entire_database_url() {
    let config = WorkerConfig {
        artifact_set_digest: ArtifactSetDigest::from_bytes([0x71; 32]),
        database_url: Some(
            "postgres://diagnostic-user:sentinel-password@private-host/private-db?options=sentinel-options"
                .to_owned(),
        ),
        db_path: "worker.db".to_owned(),
        processor_id: [0x42; 16],
    };

    for rendered in [format!("{config:?}"), format!("{config:#?}")] {
        for sensitive in [
            "diagnostic-user",
            "sentinel-password",
            "private-host",
            "private-db",
            "sentinel-options",
        ] {
            assert!(
                !rendered.contains(sensitive),
                "worker configuration diagnostics exposed connection data"
            );
        }
        assert!(rendered.contains("database_url"));
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains("worker.db"));
        assert!(rendered.contains("processor_id"));
    }
}
