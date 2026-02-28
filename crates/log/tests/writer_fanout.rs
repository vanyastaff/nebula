#[cfg(feature = "file")]
use nebula_log::Rolling;
use nebula_log::{Config, DestinationFailurePolicy, LogError, WriterConfig, init_with};

#[test]
fn multi_writer_policy_defaults_to_best_effort() {
    let config = WriterConfig::Multi {
        policy: DestinationFailurePolicy::default(),
        writers: vec![WriterConfig::Stderr, WriterConfig::Stdout],
    };

    match config {
        WriterConfig::Multi { policy, .. } => {
            assert_eq!(policy, DestinationFailurePolicy::BestEffort);
        }
        _ => panic!("unexpected writer config"),
    }
}

#[test]
fn multi_writer_can_use_fail_fast_policy() {
    let config = WriterConfig::Multi {
        policy: DestinationFailurePolicy::FailFast,
        writers: vec![WriterConfig::Stderr, WriterConfig::Stdout],
    };

    match config {
        WriterConfig::Multi { policy, .. } => {
            assert_eq!(policy, DestinationFailurePolicy::FailFast);
        }
        _ => panic!("unexpected writer config"),
    }
}

#[test]
fn multi_writer_can_use_primary_with_fallback_policy() {
    let config = WriterConfig::Multi {
        policy: DestinationFailurePolicy::PrimaryWithFallback,
        writers: vec![WriterConfig::Stderr, WriterConfig::Stdout],
    };

    match config {
        WriterConfig::Multi { policy, .. } => {
            assert_eq!(policy, DestinationFailurePolicy::PrimaryWithFallback);
        }
        _ => panic!("unexpected writer config"),
    }
}

#[test]
fn multi_writer_requires_at_least_one_destination() {
    let config = Config {
        writer: WriterConfig::Multi {
            policy: DestinationFailurePolicy::BestEffort,
            writers: vec![],
        },
        ..Config::default()
    };

    let result = init_with(config);
    match result {
        Err(LogError::Config(msg)) => assert!(msg.contains("at least one writer")),
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("expected config error"),
    }
}

#[cfg(feature = "file")]
#[test]
fn size_rolling_creates_rotated_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("rolling.log");

    let config = Config {
        writer: WriterConfig::File {
            path: path.clone(),
            rolling: Some(Rolling::Size(1)),
            non_blocking: false,
        },
        ..Config::default()
    };

    let _guard = init_with(config).expect("init with rolling size");
    let payload = "x".repeat(1024 * 1024 + 64);
    nebula_log::info!(payload = %payload, "seed file");
    nebula_log::info!("trigger rolling");

    let rotated = std::path::PathBuf::from(format!("{}.1", path.display()));
    assert!(path.exists());
    assert!(rotated.exists());
}
