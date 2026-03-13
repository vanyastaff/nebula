//! Integration tests for `nebula-system` — platform-gated tests for all five
//! system modules (`memory`, `cpu`, `disk`, `network`, `process`).
//!
//! All tests use `#[cfg(not(miri))]` because `sysinfo` makes system calls that
//! Miri cannot emulate.  Tests that are inherently racy (first-tick CPU usage)
//! are annotated with a comment explaining the non-determinism.

#[cfg(not(miri))]
mod tests {
    use tracing::debug;

    fn init_log() {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter("debug")
            .try_init();
    }

    // ── Memory ────────────────────────────────────────────────────────────────

    #[cfg(feature = "memory")]
    mod memory {
        use super::*;
        use nebula_system::memory::{self, MemoryPressure};

        #[test]
        fn current_returns_sane_values() {
            init_log();
            debug!("test: memory/current_returns_sane_values");

            let info = memory::current();
            debug!("memory info: {:?}", info);

            assert!(info.total > 0, "total memory must be > 0");
            assert!(
                info.available <= info.total,
                "available ({}) must be <= total ({})",
                info.available,
                info.total
            );
            assert!(
                (0.0..=100.0).contains(&info.usage_percent),
                "usage_percent {} must be in [0.0, 100.0]",
                info.usage_percent
            );

            debug!("test passed: current_returns_sane_values");
        }

        #[test]
        fn pressure_does_not_panic() {
            init_log();
            debug!("test: memory/pressure_does_not_panic");

            let p = memory::pressure();
            debug!("memory pressure: {:?}", p);

            // Any variant is valid — just ensure no panic.
            let _ = p;
            debug!("test passed: pressure_does_not_panic");
        }

        #[test]
        fn pressure_is_critical_only_for_critical_variant() {
            init_log();
            debug!("test: memory/pressure_is_critical_only_for_critical_variant");

            assert!(MemoryPressure::Critical.is_critical());
            assert!(!MemoryPressure::High.is_critical());
            assert!(!MemoryPressure::Medium.is_critical());
            assert!(!MemoryPressure::Low.is_critical());

            debug!("test passed: pressure_is_critical_only_for_critical_variant");
        }

        #[test]
        fn pressure_is_concerning_for_high_and_critical() {
            init_log();
            debug!("test: memory/pressure_is_concerning_for_high_and_critical");

            assert!(MemoryPressure::Critical.is_concerning());
            assert!(MemoryPressure::High.is_concerning());
            assert!(!MemoryPressure::Medium.is_concerning());
            assert!(!MemoryPressure::Low.is_concerning());

            debug!("test passed: pressure_is_concerning_for_high_and_critical");
        }
    }

    // ── CPU ───────────────────────────────────────────────────────────────────

    #[cfg(feature = "sysinfo")]
    mod cpu_tests {
        use super::*;
        use nebula_system::cpu;

        #[test]
        fn usage_returns_at_least_one_core() {
            init_log();
            debug!("test: cpu/usage_returns_at_least_one_core");

            let usage = cpu::usage();
            debug!("cpu usage: {:?}", usage);

            // NOTE: First-tick CPU usage values are often 0.0 because sysinfo
            // needs two measurements to compute a delta.  We only assert the
            // structural invariant (≥1 core) to avoid a racy assertion.
            assert!(
                !usage.per_core.is_empty(),
                "per_core must contain at least one entry"
            );

            debug!("test passed: usage_returns_at_least_one_core");
        }

        #[test]
        fn usage_average_in_range() {
            init_log();
            debug!("test: cpu/usage_average_in_range");

            let usage = cpu::usage();
            debug!("cpu average: {}", usage.average);

            assert!(
                (0.0..=100.0).contains(&usage.average),
                "average {} must be in [0.0, 100.0]",
                usage.average
            );

            debug!("test passed: usage_average_in_range");
        }

        #[test]
        fn pressure_does_not_panic() {
            init_log();
            debug!("test: cpu/pressure_does_not_panic");

            let p = cpu::pressure();
            debug!("cpu pressure: {:?}", p);

            let _ = p;
            debug!("test passed: pressure_does_not_panic");
        }

        #[test]
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        fn features_populated_on_x86() {
            init_log();
            debug!("test: cpu/features_populated_on_x86");

            let features = cpu::features();
            debug!("cpu features: {:?}", features);

            // SSE2 has been mandatory on x86-64 since the ABI was defined;
            // every 64-bit x86 processor supports it.
            assert!(features.sse2, "SSE2 must be present on x86-64");

            debug!("test passed: features_populated_on_x86");
        }

        #[test]
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        fn features_returns_default_on_non_x86() {
            init_log();
            debug!("test: cpu/features_returns_default_on_non_x86");

            // On non-x86 platforms the returned struct uses Default values;
            // just verify no panic.
            let features = cpu::features();
            debug!("cpu features (non-x86): {:?}", features);

            debug!("test passed: features_returns_default_on_non_x86");
        }
    }

    // ── Disk ──────────────────────────────────────────────────────────────────

    #[cfg(feature = "disk")]
    mod disk_tests {
        use super::*;
        use nebula_system::disk;

        #[test]
        fn list_returns_at_least_one_disk() {
            init_log();
            debug!("test: disk/list_returns_at_least_one_disk");

            let disks = disk::list();
            debug!("disks: {:?}", disks);

            assert!(!disks.is_empty(), "disk list must not be empty");

            debug!("test passed: list_returns_at_least_one_disk");
        }

        #[test]
        fn disk_info_has_valid_fields() {
            init_log();
            debug!("test: disk/disk_info_has_valid_fields");

            let disks = disk::list();
            for disk_info in &disks {
                debug!("disk info: {:?}", disk_info);

                assert!(
                    disk_info.total_space > 0,
                    "disk '{}' total_space must be > 0",
                    disk_info.mount_point
                );
                assert!(
                    !disk_info.mount_point.is_empty(),
                    "disk mount_point must not be empty"
                );
                assert!(
                    (0.0..=100.0).contains(&disk_info.usage_percent),
                    "disk '{}' usage_percent {} must be in [0.0, 100.0]",
                    disk_info.mount_point,
                    disk_info.usage_percent
                );
            }

            debug!("test passed: disk_info_has_valid_fields");
        }

        #[test]
        fn total_usage_in_range() {
            init_log();
            debug!("test: disk/total_usage_in_range");

            let usage = disk::total_usage();
            debug!("disk total usage: {:?}", usage);

            assert!(
                (0.0..=100.0).contains(&usage.usage_percent),
                "total usage_percent {} must be in [0.0, 100.0]",
                usage.usage_percent
            );

            debug!("test passed: total_usage_in_range");
        }
    }

    // ── Network ───────────────────────────────────────────────────────────────

    #[cfg(feature = "network")]
    mod network_tests {
        use super::*;
        use nebula_system::network;

        #[test]
        fn interfaces_not_empty() {
            init_log();
            debug!("test: network/interfaces_not_empty");

            let ifaces = network::interfaces();
            debug!("network interfaces: {:?}", ifaces);

            // At minimum the loopback interface should be present on all OSes.
            assert!(!ifaces.is_empty(), "network interface list must not be empty");

            debug!("test passed: interfaces_not_empty");
        }

        #[test]
        fn interface_names_non_empty() {
            init_log();
            debug!("test: network/interface_names_non_empty");

            let ifaces = network::interfaces();
            for iface in &ifaces {
                debug!("interface: {:?}", iface);
                assert!(
                    !iface.name.is_empty(),
                    "interface name must not be empty"
                );
            }

            debug!("test passed: interface_names_non_empty");
        }

        // Loopback detection is name-based ("lo" / "lo0") — only asserted on
        // Linux/macOS where the name is conventional.
        #[test]
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        fn loopback_interface_detected_on_unix() {
            init_log();
            debug!("test: network/loopback_interface_detected_on_unix");

            let ifaces = network::interfaces();
            debug!("interfaces: {:?}", ifaces);

            let has_loopback = ifaces.iter().any(|i| i.is_loopback);
            assert!(
                has_loopback,
                "expected at least one loopback interface on Linux/macOS"
            );

            debug!("test passed: loopback_interface_detected_on_unix");
        }
    }

    // ── Process ───────────────────────────────────────────────────────────────

    #[cfg(feature = "process")]
    mod process_tests {
        use super::*;
        use nebula_system::process;

        #[test]
        fn current_returns_own_pid() {
            init_log();
            debug!("test: process/current_returns_own_pid");

            let info = process::current().expect("current() must succeed");
            debug!("process info: {:?}", info);

            assert_eq!(
                info.pid,
                std::process::id(),
                "ProcessInfo.pid must match std::process::id()"
            );

            debug!("test passed: current_returns_own_pid");
        }

        #[test]
        fn current_name_non_empty() {
            init_log();
            debug!("test: process/current_name_non_empty");

            let info = process::current().expect("current() must succeed");
            debug!("process name: {}", info.name);

            assert!(!info.name.is_empty(), "process name must not be empty");

            debug!("test passed: current_name_non_empty");
        }

        #[test]
        fn get_nonexistent_pid_returns_error() {
            init_log();
            debug!("test: process/get_nonexistent_pid_returns_error");

            // PID u32::MAX is virtually guaranteed not to exist.
            let result = process::get_process(u32::MAX);
            debug!("get_process(u32::MAX) result: {:?}", result);

            assert!(
                result.is_err(),
                "get_process(u32::MAX) must return Err for non-existent PID"
            );

            debug!("test passed: get_nonexistent_pid_returns_error");
        }

        /// Documented stub: `cmd` is always empty (see `ProcessInfo` Known Limitations).
        #[test]
        fn cmd_is_empty_stub() {
            init_log();
            debug!("test: process/cmd_is_empty_stub");

            let info = process::current().expect("current() must succeed");
            debug!("process cmd: {:?}", info.cmd);

            // cmd is intentionally not populated (performance constraint);
            // asserting the documented stub behaviour.
            assert!(
                info.cmd.is_empty(),
                "cmd must be empty (documented stub — see ProcessInfo Known Limitations)"
            );

            debug!("test passed: cmd_is_empty_stub");
        }

        /// Documented stub: `environ` is always empty.
        #[test]
        fn environ_is_empty_stub() {
            init_log();
            debug!("test: process/environ_is_empty_stub");

            let info = process::current().expect("current() must succeed");
            debug!("process environ entry count: {}", info.environ.len());

            assert!(
                info.environ.is_empty(),
                "environ must be empty (documented stub — see ProcessInfo Known Limitations)"
            );

            debug!("test passed: environ_is_empty_stub");
        }

        /// On Windows, uid/gid are always None (Unix-only).
        #[test]
        #[cfg(target_os = "windows")]
        fn uid_gid_none_on_windows() {
            init_log();
            debug!("test: process/uid_gid_none_on_windows");

            let info = process::current().expect("current() must succeed");
            debug!("uid: {:?}, gid: {:?}", info.uid, info.gid);

            assert!(info.uid.is_none(), "uid must be None on Windows");
            assert!(info.gid.is_none(), "gid must be None on Windows");

            debug!("test passed: uid_gid_none_on_windows");
        }
    }
}
