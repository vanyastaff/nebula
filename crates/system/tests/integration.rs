//! Integration tests for `nebula-system` — platform-gated tests for all
//! system modules (`memory`, `cpu`, `disk`, `network`, `process`, `info`).
//!
//! All tests use `#[cfg(not(miri))]` because `sysinfo` makes system calls that
//! Miri cannot emulate.  Tests that are inherently racy (first-tick CPU usage)
//! are annotated with a comment explaining the non-determinism.

#[cfg(not(miri))]
mod tests {
    // ── Memory ────────────────────────────────────────────────────────────────

    #[cfg(feature = "sysinfo")]
    mod memory {
        use nebula_system::memory::{self, MemoryPressure};

        #[test]
        fn current_returns_sane_values() {
            let info = memory::current();
            assert!(info.total > 0, "total memory must be > 0");
            assert!(
                info.available <= info.total,
                "available ({}) must be <= total ({})",
                info.available,
                info.total
            );
            let usage_percent = info
                .usage_percent
                .value()
                .copied()
                .expect("usage percent should be available when total memory is available");
            assert!(
                (0.0..=100.0).contains(&usage_percent),
                "usage_percent {usage_percent} must be in [0.0, 100.0]",
            );
            assert!(
                !info.pressure_report.reasons.is_empty(),
                "pressure report must explain the classifier decision"
            );
        }

        #[test]
        fn current_used_equals_total_minus_available() {
            let info = memory::current();
            // Allow ±1 page tolerance for race between total/available reads
            let expected = info.total.saturating_sub(info.available);
            let diff = info.used.abs_diff(expected);
            assert!(
                diff <= 4096,
                "used ({}) should be ~total-available ({expected}), diff={diff}",
                info.used
            );
        }

        #[test]
        fn pressure_does_not_panic() {
            let _ = memory::pressure();
        }

        #[test]
        fn pressure_is_critical_only_for_critical_variant() {
            assert!(MemoryPressure::Critical.is_critical());
            assert!(!MemoryPressure::High.is_critical());
            assert!(!MemoryPressure::Medium.is_critical());
            assert!(!MemoryPressure::Low.is_critical());
        }

        #[test]
        fn pressure_is_concerning_for_high_and_critical() {
            assert!(MemoryPressure::Critical.is_concerning());
            assert!(MemoryPressure::High.is_concerning());
            assert!(!MemoryPressure::Medium.is_concerning());
            assert!(!MemoryPressure::Low.is_concerning());
        }

        #[test]
        fn pressure_ordering() {
            assert!(MemoryPressure::Low < MemoryPressure::Medium);
            assert!(MemoryPressure::Medium < MemoryPressure::High);
            assert!(MemoryPressure::High < MemoryPressure::Critical);
        }
    }

    // ── CPU ───────────────────────────────────────────────────────────────────

    #[cfg(feature = "sysinfo")]
    mod cpu_tests {
        use nebula_system::cpu::{self, CpuPressure};

        #[test]
        fn usage_returns_at_least_one_core() {
            let usage = cpu::usage();
            assert!(
                !usage.per_core.is_empty(),
                "per_core must contain at least one entry"
            );
        }

        #[test]
        fn usage_average_in_range() {
            let usage = cpu::usage();
            assert!(
                (0.0..=100.0).contains(&usage.average),
                "average {} must be in [0.0, 100.0]",
                usage.average
            );
            assert!(
                matches!(
                    usage.sample_status,
                    nebula_system::AvailabilityStatus::Available
                        | nebula_system::AvailabilityStatus::NotSampled
                        | nebula_system::AvailabilityStatus::Stale
                ),
                "cpu usage sample status must describe backend freshness"
            );
        }

        #[test]
        fn pressure_does_not_panic() {
            let _ = cpu::pressure();
        }

        // ── CpuPressure::from_usage boundary values ──

        #[test]
        fn pressure_from_usage_boundaries() {
            assert_eq!(CpuPressure::from_usage(0.0), CpuPressure::Low);
            assert_eq!(CpuPressure::from_usage(50.0), CpuPressure::Low);
            assert_eq!(CpuPressure::from_usage(50.1), CpuPressure::Medium);
            assert_eq!(CpuPressure::from_usage(70.0), CpuPressure::Medium);
            assert_eq!(CpuPressure::from_usage(70.1), CpuPressure::High);
            assert_eq!(CpuPressure::from_usage(85.0), CpuPressure::High);
            assert_eq!(CpuPressure::from_usage(85.1), CpuPressure::Critical);
            assert_eq!(CpuPressure::from_usage(100.0), CpuPressure::Critical);
        }

        #[test]
        fn pressure_is_concerning() {
            assert!(!CpuPressure::Low.is_concerning());
            assert!(!CpuPressure::Medium.is_concerning());
            assert!(CpuPressure::High.is_concerning());
            assert!(CpuPressure::Critical.is_concerning());
        }

        // ── features() caching ──

        #[test]
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        fn features_populated_on_x86() {
            let features = cpu::features();
            assert!(features.sse2, "SSE2 must be present on x86-64");
        }

        #[test]
        fn features_cached_across_calls() {
            let a = cpu::features();
            let b = cpu::features();
            assert_eq!(a.sse, b.sse);
            assert_eq!(a.avx, b.avx);
            assert_eq!(a.avx2, b.avx2);
        }

        // ── topology() ──

        #[test]
        fn topology_sane_values() {
            let topo = cpu::topology();
            assert!(
                topo.cores_per_package >= 1,
                "cores_per_package must be >= 1"
            );
            assert!(topo.threads_per_core >= 1, "threads_per_core must be >= 1");
            assert!(
                !topo.numa_nodes.is_empty(),
                "must have at least one NUMA node"
            );
        }

        #[test]
        fn optimal_thread_count_positive() {
            assert!(cpu::optimal_thread_count() > 0);
        }

        #[test]
        fn cache_info_has_line_size() {
            let cache = cpu::cache_info();
            assert!(cache.line_size > 0, "cache line_size must be > 0");
        }
    }

    // ── Disk ──────────────────────────────────────────────────────────────────

    #[cfg(feature = "disk")]
    mod disk_tests {
        use nebula_system::disk::{self, DiskPressure};

        #[test]
        fn list_returns_at_least_one_disk() {
            let disks = disk::list();
            assert!(!disks.is_empty(), "disk list must not be empty");
        }

        #[test]
        fn disk_info_has_valid_fields() {
            let disks = disk::list();
            for d in &disks {
                assert!(
                    d.total_space > 0,
                    "disk '{}' total_space must be > 0",
                    d.mount_point
                );
                assert!(
                    !d.mount_point.is_empty(),
                    "disk mount_point must not be empty"
                );
                assert!(
                    (0.0..=100.0).contains(&d.usage_percent),
                    "disk '{}' usage_percent {} out of range",
                    d.mount_point,
                    d.usage_percent
                );
            }
        }

        #[test]
        fn total_usage_in_range() {
            let usage = disk::total_usage();
            assert!(
                (0.0..=100.0).contains(&usage.usage_percent),
                "total usage_percent {} out of range",
                usage.usage_percent
            );
        }

        // ── DiskPressure::from_usage boundary values ──

        #[test]
        fn disk_pressure_from_usage_boundaries() {
            assert_eq!(DiskPressure::from_usage(0.0), DiskPressure::Low);
            assert_eq!(DiskPressure::from_usage(50.0), DiskPressure::Low);
            assert_eq!(DiskPressure::from_usage(50.1), DiskPressure::Medium);
            assert_eq!(DiskPressure::from_usage(75.0), DiskPressure::Medium);
            assert_eq!(DiskPressure::from_usage(75.1), DiskPressure::High);
            assert_eq!(DiskPressure::from_usage(90.0), DiskPressure::High);
            assert_eq!(DiskPressure::from_usage(90.1), DiskPressure::Critical);
            assert_eq!(DiskPressure::from_usage(100.0), DiskPressure::Critical);
        }

        #[test]
        fn disk_pressure_is_concerning() {
            assert!(!DiskPressure::Low.is_concerning());
            assert!(!DiskPressure::Medium.is_concerning());
            assert!(DiskPressure::High.is_concerning());
            assert!(DiskPressure::Critical.is_concerning());
        }

        #[test]
        fn missing_mount_pressure_is_unavailable_not_low() {
            let pressure = disk::pressure(Some("__definitely_not_a_mount__"));
            assert!(
                !pressure.is_available(),
                "missing mount must not be reported as Low pressure"
            );
        }

        #[test]
        fn current_path_pressure_is_available() {
            let disk = disk::disk_for_path(".");
            assert!(
                disk.is_available(),
                "disk_for_path('.') should locate the current directory mount"
            );
            let disk = disk.value().expect("available disk value");
            assert!(
                disk.total_space > 0,
                "current directory disk must report total space"
            );

            let pressure = disk::pressure_for_path(".");
            assert!(
                pressure.is_available(),
                "pressure_for_path('.') should locate the current directory mount"
            );
        }

        #[test]
        fn filesystem_info_reports_availability_explicitly() {
            let info = disk::filesystem_info(".");

            #[cfg(unix)]
            assert!(
                info.is_available(),
                "filesystem_info('.') should be available on Unix"
            );

            #[cfg(windows)]
            assert!(
                !info.is_available(),
                "filesystem_info must not fake data on unsupported platforms"
            );
        }

        #[test]
        fn has_enough_space_for_zero_bytes() {
            // Zero bytes required should always succeed on any existing mount point
            let disks = disk::list();
            if let Some(d) = disks.first() {
                assert_eq!(
                    disk::has_enough_space(&d.mount_point, 0).value().copied(),
                    Some(true)
                );
            }
        }

        #[test]
        fn has_enough_space_for_absurd_amount() {
            // u64::MAX bytes should never be available
            let disks = disk::list();
            if let Some(d) = disks.first() {
                assert_eq!(
                    disk::has_enough_space(&d.mount_point, u64::MAX)
                        .value()
                        .copied(),
                    Some(false)
                );
            }
        }
    }

    // ── Network ───────────────────────────────────────────────────────────────

    #[cfg(feature = "network")]
    mod network_tests {
        use nebula_system::network;

        #[test]
        fn interfaces_not_empty() {
            let ifaces = network::interfaces();
            assert!(
                !ifaces.is_empty(),
                "network interface list must not be empty"
            );
        }

        #[test]
        fn interface_names_non_empty() {
            let ifaces = network::interfaces();
            for iface in &ifaces {
                assert!(!iface.name.is_empty(), "interface name must not be empty");
                assert!(
                    iface.ip_addresses.is_available(),
                    "ip address probing must report availability explicitly"
                );
                assert!(
                    !iface.is_up.is_available(),
                    "interface up/down is not implemented and must be explicit"
                );
            }
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        #[test]
        fn loopback_interface_detected_on_unix() {
            let ifaces = network::interfaces();
            assert!(
                ifaces.iter().any(|i| i.is_loopback),
                "expected at least one loopback interface on Linux/macOS"
            );
        }

        #[test]
        fn get_interface_returns_none_for_nonexistent() {
            assert!(network::get_interface("__nonexistent_iface__").is_none());
        }

        #[test]
        fn total_stats_non_negative() {
            let stats = network::total_stats();
            // rx/tx bytes are u64, always >= 0 by type. Just verify we can read them.
            let _ = stats.rx_bytes;
            let _ = stats.tx_bytes;
        }
    }

    // ── Process ───────────────────────────────────────────────────────────────

    #[cfg(feature = "process")]
    mod process_tests {
        use nebula_system::process;

        #[test]
        fn current_returns_own_pid() {
            let info = process::current().expect("current() must succeed");
            assert_eq!(info.pid, std::process::id());
        }

        #[test]
        fn current_name_non_empty() {
            let info = process::current().expect("current() must succeed");
            assert!(!info.name.is_empty(), "process name must not be empty");
            if let Some(thread_count) = info.thread_count.value() {
                assert!(
                    *thread_count >= 1,
                    "available thread_count must be positive"
                );
            } else {
                assert!(
                    !info.thread_count.is_available(),
                    "unknown thread_count must be explicit"
                );
            }
        }

        #[test]
        fn get_nonexistent_pid_returns_error() {
            let result = process::get_process(u32::MAX);
            assert!(result.is_err(), "get_process(u32::MAX) must return Err");
        }

        #[test]
        fn list_is_non_empty() {
            let procs = process::list();
            assert!(!procs.is_empty(), "process list must not be empty");
        }

        #[test]
        fn list_contains_current_process() {
            let my_pid = std::process::id();
            let procs = process::list();
            assert!(
                procs.iter().any(|p| p.pid == my_pid),
                "process list must contain current process (pid={my_pid})"
            );
        }

        #[test]
        fn stats_total_positive() {
            let stats = process::stats();
            assert!(stats.total > 0, "stats.total must be > 0");
        }

        #[test]
        fn find_by_name_finds_current() {
            let info = process::current().expect("current() must succeed");
            let found = process::find_by_name(&info.name);
            assert!(
                found.iter().any(|p| p.pid == info.pid),
                "find_by_name({}) must find current process",
                info.name
            );
        }

        #[test]
        fn children_returns_vec() {
            // May be empty, just ensure no panic
            let _ = process::children(std::process::id());
        }

        #[cfg(target_os = "windows")]
        #[test]
        fn uid_gid_none_on_windows() {
            let info = process::current().expect("current() must succeed");
            assert!(
                !info.uid.is_available(),
                "uid must be marked unavailable/unsupported on Windows"
            );
            assert!(
                !info.gid.is_available(),
                "gid must be marked unavailable/unsupported on Windows"
            );
        }
    }

    // ── ProcessMonitor ───────────────────────────────────────────────────────

    #[cfg(feature = "process")]
    mod process_monitor_tests {
        use nebula_system::process::ProcessMonitor;

        #[test]
        fn monitor_current_process() {
            let mut monitor = ProcessMonitor::new(std::process::id()).expect("monitor own process");
            let sample = monitor.sample();
            assert!(
                sample.is_some(),
                "sample() must return Some for live process"
            );
            let sample = sample.unwrap();
            assert_eq!(sample.pid, std::process::id());
            assert!(sample.memory > 0, "process must use some memory");
        }

        #[test]
        fn monitor_nonexistent_pid_fails() {
            let result = ProcessMonitor::new(u32::MAX);
            assert!(result.is_err(), "monitor for nonexistent PID must fail");
        }

        #[test]
        fn peak_memory_tracks_high_water_mark() {
            let mut monitor = ProcessMonitor::new(std::process::id()).expect("monitor own process");
            // Sample twice — peak should be >= both
            let s1 = monitor.sample().expect("sample 1");
            let s2 = monitor.sample().expect("sample 2");
            assert!(monitor.peak_memory() >= s1.memory);
            assert!(monitor.peak_memory() >= s2.memory);
        }

        #[test]
        fn elapsed_is_positive() {
            let monitor = ProcessMonitor::new(std::process::id()).expect("monitor own process");
            assert!(
                monitor.elapsed() > std::time::Duration::ZERO,
                "elapsed must be > 0"
            );
        }

        #[test]
        fn pid_getter() {
            let monitor = ProcessMonitor::new(std::process::id()).expect("monitor own process");
            assert_eq!(monitor.pid(), std::process::id());
        }
    }

    // ── Info ──────────────────────────────────────────────────────────────────

    #[cfg(feature = "sysinfo")]
    mod info_tests {
        use nebula_system::info::{OsFamily, SystemInfo};

        #[test]
        fn get_returns_consistent_data() {
            let a = SystemInfo::get();
            let b = SystemInfo::get();
            // Same cached snapshot — Arc points to same allocation
            assert_eq!(a.cpu.cores, b.cpu.cores);
            assert_eq!(a.os.name, b.os.name);
            assert_eq!(a.memory.total, b.memory.total);
        }

        #[test]
        fn summary_is_non_empty() {
            let s = nebula_system::summary();
            assert!(!s.is_empty(), "summary must not be empty");
        }

        #[test]
        fn summary_contains_os_name() {
            let info = SystemInfo::get();
            let s = nebula_system::summary();
            assert!(
                s.contains(&info.os.name),
                "summary should contain OS name '{}'",
                info.os.name
            );
        }

        #[test]
        fn os_family_matches_platform() {
            let info = SystemInfo::get();
            match std::env::consts::OS {
                "windows" => assert_eq!(info.os.family, OsFamily::Windows),
                "linux" => assert_eq!(info.os.family, OsFamily::Linux),
                "macos" => assert_eq!(info.os.family, OsFamily::MacOS),
                _ => {}, // Other platforms — just don't panic
            }
        }

        #[test]
        fn init_is_idempotent() {
            nebula_system::init().expect("first init must succeed");
            nebula_system::init().expect("second init must also succeed");
        }

        #[test]
        fn hardware_info_sane() {
            let info = SystemInfo::get();
            assert!(info.hardware.cache_line_size > 0);
            assert!(info.hardware.allocation_granularity > 0);
            assert!(info.hardware.numa_nodes >= 1);
        }
    }

    // ── SystemLoad ───────────────────────────────────────────────────────────

    #[cfg(feature = "sysinfo")]
    mod load_tests {
        use nebula_system::{
            Availability, AvailabilityStatus,
            cpu::CpuPressure,
            info::MemoryCapacitySource,
            load::{self, SystemLoad},
            memory::MemoryPressure,
        };

        #[test]
        fn system_load_values_in_range() {
            let load = load::system_load();
            assert!(
                matches!(
                    load.cpu_sample_status,
                    AvailabilityStatus::Available
                        | AvailabilityStatus::NotSampled
                        | AvailabilityStatus::Stale
                ),
                "CPU sample status should describe freshness"
            );
            if let Some(cpu_usage_percent) = load.cpu_usage_percent.value().copied() {
                assert!(
                    (0.0..=100.0).contains(&cpu_usage_percent),
                    "cpu_usage_percent {cpu_usage_percent} out of range",
                );
            }
            let memory_usage_percent = load
                .memory_usage_percent
                .value()
                .copied()
                .expect("memory usage should be available when sysinfo memory is available");
            assert!(
                (0.0..=100.0).contains(&memory_usage_percent),
                "memory_usage_percent {memory_usage_percent} out of range",
            );
        }

        #[test]
        fn headroom_in_unit_range() {
            let load = load::system_load();
            if let Some(h) = load.headroom().value().copied() {
                assert!(
                    (0.0..=1.0).contains(&h),
                    "headroom {h} must be in [0.0, 1.0]"
                );
            } else {
                assert!(
                    !load.can_accept_work().is_available(),
                    "unavailable headroom should make work admission unavailable"
                );
            }
        }

        #[test]
        fn can_accept_work_returns_availability() {
            let _ = load::system_load().can_accept_work();
        }

        #[test]
        fn unavailable_usage_rejects_work_and_headroom() {
            let load = SystemLoad {
                cpu: CpuPressure::Low,
                memory: MemoryPressure::Low,
                cpu_usage_percent: Availability::not_sampled("first sample"),
                memory_usage_percent: Availability::available(20.0),
                cpu_sample_status: AvailabilityStatus::NotSampled,
                memory_capacity_source: MemoryCapacitySource::Host,
            };

            assert_eq!(
                load.can_accept_work().status,
                AvailabilityStatus::NotSampled
            );
            assert!(!load.headroom().is_available());
        }

        #[test]
        fn critical_pressure_rejects_work() {
            let load = SystemLoad {
                cpu: CpuPressure::Critical,
                memory: MemoryPressure::Low,
                cpu_usage_percent: Availability::available(95.0),
                memory_usage_percent: Availability::available(20.0),
                cpu_sample_status: AvailabilityStatus::Available,
                memory_capacity_source: MemoryCapacitySource::Host,
            };
            assert_eq!(
                load.can_accept_work().value().copied(),
                Some(false),
                "Critical CPU must reject work"
            );

            let load = SystemLoad {
                cpu: CpuPressure::Low,
                memory: MemoryPressure::Critical,
                cpu_usage_percent: Availability::available(10.0),
                memory_usage_percent: Availability::available(95.0),
                cpu_sample_status: AvailabilityStatus::Available,
                memory_capacity_source: MemoryCapacitySource::Host,
            };
            assert_eq!(
                load.can_accept_work().value().copied(),
                Some(false),
                "Critical memory must reject work"
            );
        }

        #[test]
        fn low_pressure_accepts_work() {
            let load = SystemLoad {
                cpu: CpuPressure::Low,
                memory: MemoryPressure::Low,
                cpu_usage_percent: Availability::available(10.0),
                memory_usage_percent: Availability::available(20.0),
                cpu_sample_status: AvailabilityStatus::Available,
                memory_capacity_source: MemoryCapacitySource::Host,
            };
            assert_eq!(load.can_accept_work().value().copied(), Some(true));
        }

        #[test]
        fn headroom_boundary_values() {
            let idle = SystemLoad {
                cpu: CpuPressure::Low,
                memory: MemoryPressure::Low,
                cpu_usage_percent: Availability::available(0.0),
                memory_usage_percent: Availability::available(0.0),
                cpu_sample_status: AvailabilityStatus::Available,
                memory_capacity_source: MemoryCapacitySource::Host,
            };
            assert!((idle.headroom().value().copied().unwrap() - 1.0).abs() < f64::EPSILON);

            let full = SystemLoad {
                cpu: CpuPressure::Critical,
                memory: MemoryPressure::Critical,
                cpu_usage_percent: Availability::available(100.0),
                memory_usage_percent: Availability::available(100.0),
                cpu_sample_status: AvailabilityStatus::Available,
                memory_capacity_source: MemoryCapacitySource::Host,
            };
            assert!(full.headroom().value().copied().unwrap().abs() < f64::EPSILON);
        }
    }
}
