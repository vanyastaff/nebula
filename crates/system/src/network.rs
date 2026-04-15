//! Network interface information and monitoring
//!
//! # Known Limitations
//!
//! - **Rate tracking** via the `NETWORK_STATS` lazy global may not reflect accurate rates on the
//!   first tick (before any previous snapshot exists), returning `rx_rate = 0.0` and `tx_rate =
//!   0.0` for newly seen interfaces.
//! - **`ip_addresses`** is always empty (`vec![]`); populating it requires additional
//!   platform-specific code beyond sysinfo's network API.
//! - **`is_loopback`** detection is name-based (`"lo"` / `"lo0"`) and may miss renamed loopback
//!   interfaces on non-standard configurations.

use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
    time::Instant,
};

use parking_lot::RwLock;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Network interface information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NetworkInterface {
    /// Interface name (e.g., "eth0", "wlan0", "lo")
    pub name: String,
    /// MAC address
    pub mac_address: Option<String>,
    /// IP addresses (v4 and v6)
    pub ip_addresses: Vec<IpAddress>,
    /// Whether the interface is up
    pub is_up: bool,
    /// Whether the interface is a loopback
    pub is_loopback: bool,
    /// MTU (Maximum Transmission Unit)
    pub mtu: Option<u32>,
    /// Link speed in Mbps (if available)
    pub speed: Option<u64>,
    /// Network statistics
    pub stats: NetworkStats,
}

/// IP address information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IpAddress {
    /// IP address string
    pub address: String,
    /// Network prefix length
    pub prefix_len: u8,
    /// Whether this is IPv6
    pub is_ipv6: bool,
}

/// Network statistics
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NetworkStats {
    /// Bytes received
    pub rx_bytes: u64,
    /// Bytes transmitted
    pub tx_bytes: u64,
    /// Packets received
    pub rx_packets: u64,
    /// Packets transmitted
    pub tx_packets: u64,
    /// Receive errors
    pub rx_errors: u64,
    /// Transmit errors
    pub tx_errors: u64,
    /// Packets dropped on receive
    pub rx_dropped: u64,
    /// Packets dropped on transmit
    pub tx_dropped: u64,
}

/// Network usage over time
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NetworkUsage {
    /// Interface name
    pub interface: String,
    /// Receive rate (bytes/sec)
    pub rx_rate: f64,
    /// Transmit rate (bytes/sec)
    pub tx_rate: f64,
    /// Total received since measurement start
    pub total_rx: u64,
    /// Total transmitted since measurement start
    pub total_tx: u64,
}

// Static storage for network statistics tracking
#[derive(Debug, Clone)]
struct CachedNetworkStats {
    stats: NetworkStats,
    observed_at: Instant,
}

// Static storage for network statistics tracking
static NETWORK_STATS: LazyLock<RwLock<HashMap<String, CachedNetworkStats>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// List all network interfaces
pub fn interfaces() -> Vec<NetworkInterface> {
    #[cfg(feature = "network")]
    {
        use sysinfo::Networks;

        let networks = Networks::new_with_refreshed_list();

        networks
            .iter()
            .map(|(name, network)| {
                let stats = NetworkStats {
                    rx_bytes: network.total_received(),
                    tx_bytes: network.total_transmitted(),
                    rx_packets: network.total_packets_received(),
                    tx_packets: network.total_packets_transmitted(),
                    rx_errors: network.total_errors_on_received(),
                    tx_errors: network.total_errors_on_transmitted(),
                    rx_dropped: 0,
                    tx_dropped: 0,
                };

                NetworkInterface {
                    name: name.to_string(),
                    mac_address: Some(network.mac_address().to_string()),
                    ip_addresses: vec![],
                    is_up: true,
                    is_loopback: name == "lo" || name == "lo0",
                    mtu: None,
                    speed: None,
                    stats,
                }
            })
            .collect()
    }

    #[cfg(not(feature = "network"))]
    {
        Vec::new()
    }
}

/// Get a specific network interface by name
pub fn get_interface(name: &str) -> Option<NetworkInterface> {
    interfaces().into_iter().find(|iface| iface.name == name)
}

/// Get network usage statistics with rate tracking
///
/// On the first call for a given interface, rates will be `0.0` because
/// there is no previous snapshot to compute a delta from.
pub fn usage() -> Vec<NetworkUsage> {
    #[cfg(feature = "network")]
    {
        use sysinfo::Networks;

        let mut networks = Networks::new_with_refreshed_list();
        networks.refresh(false);

        let snapshot: Vec<(String, NetworkStats)> = networks
            .iter()
            .map(|(name, network)| {
                (
                    name.to_string(),
                    NetworkStats {
                        rx_bytes: network.total_received(),
                        tx_bytes: network.total_transmitted(),
                        rx_packets: network.total_packets_received(),
                        tx_packets: network.total_packets_transmitted(),
                        rx_errors: network.total_errors_on_received(),
                        tx_errors: network.total_errors_on_transmitted(),
                        rx_dropped: 0,
                        tx_dropped: 0,
                    },
                )
            })
            .collect();

        let mut stats = NETWORK_STATS.write();
        apply_network_snapshot(&mut stats, snapshot, Instant::now())
    }

    #[cfg(not(feature = "network"))]
    {
        Vec::new()
    }
}

fn apply_network_snapshot(
    cache: &mut HashMap<String, CachedNetworkStats>,
    snapshot: Vec<(String, NetworkStats)>,
    observed_at: Instant,
) -> Vec<NetworkUsage> {
    let mut usage_list = Vec::with_capacity(snapshot.len());
    let mut current_names = HashSet::with_capacity(snapshot.len());

    for (name, current_stats) in snapshot {
        current_names.insert(name.clone());
        let (rx_rate, tx_rate) = if let Some(prev) = cache.get(&name) {
            let elapsed = observed_at
                .saturating_duration_since(prev.observed_at)
                .as_secs_f64();
            if elapsed > 0.0 {
                (
                    current_stats.rx_bytes.saturating_sub(prev.stats.rx_bytes) as f64 / elapsed,
                    current_stats.tx_bytes.saturating_sub(prev.stats.tx_bytes) as f64 / elapsed,
                )
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        usage_list.push(NetworkUsage {
            interface: name.clone(),
            rx_rate,
            tx_rate,
            total_rx: current_stats.rx_bytes,
            total_tx: current_stats.tx_bytes,
        });

        cache.insert(
            name,
            CachedNetworkStats {
                stats: current_stats,
                observed_at,
            },
        );
    }

    // Evict interfaces that disappeared from the current OS snapshot.
    cache.retain(|name, _| current_names.contains(name));

    usage_list
}

/// Get total network statistics across all interfaces
pub fn total_stats() -> NetworkStats {
    let ifaces = interfaces();

    ifaces
        .iter()
        .fold(NetworkStats::default(), |mut acc, iface| {
            acc.rx_bytes += iface.stats.rx_bytes;
            acc.tx_bytes += iface.stats.tx_bytes;
            acc.rx_packets += iface.stats.rx_packets;
            acc.tx_packets += iface.stats.tx_packets;
            acc.rx_errors += iface.stats.rx_errors;
            acc.tx_errors += iface.stats.tx_errors;
            acc.rx_dropped += iface.stats.rx_dropped;
            acc.tx_dropped += iface.stats.tx_dropped;
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_rates_are_normalized_to_seconds() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();

        let first = apply_network_snapshot(
            &mut cache,
            vec![(
                "eth0".to_owned(),
                NetworkStats {
                    rx_bytes: 100,
                    tx_bytes: 50,
                    ..NetworkStats::default()
                },
            )],
            t0,
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].rx_rate, 0.0);
        assert_eq!(first[0].tx_rate, 0.0);

        let second = apply_network_snapshot(
            &mut cache,
            vec![(
                "eth0".to_owned(),
                NetworkStats {
                    rx_bytes: 300,
                    tx_bytes: 250,
                    ..NetworkStats::default()
                },
            )],
            t0 + std::time::Duration::from_secs(2),
        );
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].rx_rate, 100.0);
        assert_eq!(second[0].tx_rate, 100.0);
    }

    #[test]
    fn stale_interfaces_are_evicted_from_cache() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        apply_network_snapshot(
            &mut cache,
            vec![
                (
                    "eth0".to_owned(),
                    NetworkStats {
                        rx_bytes: 1,
                        ..NetworkStats::default()
                    },
                ),
                (
                    "veth123".to_owned(),
                    NetworkStats {
                        rx_bytes: 1,
                        ..NetworkStats::default()
                    },
                ),
            ],
            t0,
        );
        assert_eq!(cache.len(), 2);

        apply_network_snapshot(
            &mut cache,
            vec![(
                "eth0".to_owned(),
                NetworkStats {
                    rx_bytes: 2,
                    ..NetworkStats::default()
                },
            )],
            t0 + std::time::Duration::from_secs(1),
        );

        assert_eq!(cache.len(), 1);
        assert!(cache.contains_key("eth0"));
        assert!(!cache.contains_key("veth123"));
    }
}
