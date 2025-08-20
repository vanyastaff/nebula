//! Network information and monitoring

use std::collections::HashMap;
use once_cell::sync::Lazy;
use parking_lot::RwLock;

#[cfg(feature = "serde")]
use serde::{Serialize, Deserialize};

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

/// Connection information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Connection {
    /// Local address
    pub local_addr: String,
    /// Local port
    pub local_port: u16,
    /// Remote address
    pub remote_addr: String,
    /// Remote port
    pub remote_port: u16,
    /// Connection state
    pub state: ConnectionState,
    /// Protocol (TCP/UDP)
    pub protocol: Protocol,
    /// Process ID using this connection
    pub pid: Option<u32>,
}

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ConnectionState {
    Established,
    Listen,
    TimeWait,
    CloseWait,
    SynSent,
    SynReceived,
    Closing,
    Closed,
    Unknown,
}

/// Network protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Protocol {
    Tcp,
    Udp,
    Tcp6,
    Udp6,
    Other,
}

// Static storage for network statistics tracking
static NETWORK_STATS: Lazy<RwLock<HashMap<String, NetworkStats>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// List all network interfaces
pub fn interfaces() -> Vec<NetworkInterface> {
    #[cfg(feature = "network")]
    {
        use sysinfo::Networks;

        // sysinfo 0.37: use Networks helper instead of System::networks()
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
                    rx_dropped: 0, // Not available in sysinfo
                    tx_dropped: 0, // Not available in sysinfo
                };

                NetworkInterface {
                    name: name.clone(),
                    mac_address: Some(network.mac_address().to_string()),
                    ip_addresses: vec![], // Would need additional platform-specific code
                    is_up: true, // Assume up if in the list
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

/// Get specific network interface
pub fn get_interface(name: &str) -> Option<NetworkInterface> {
    interfaces()
        .into_iter()
        .find(|iface| iface.name == name)
}

/// Get network usage statistics
pub fn usage() -> Vec<NetworkUsage> {
    #[cfg(feature = "network")]
    {
        use sysinfo::Networks;

        // sysinfo 0.37: use Networks helper and refresh it
        let mut networks = Networks::new_with_refreshed_list();
        networks.refresh(false);

        let mut stats = NETWORK_STATS.write();
        let mut usage_list = Vec::new();

        for (name, network) in networks.iter() {
            let current_stats = NetworkStats {
                rx_bytes: network.total_received(),
                tx_bytes: network.total_transmitted(),
                rx_packets: network.total_packets_received(),
                tx_packets: network.total_packets_transmitted(),
                rx_errors: network.total_errors_on_received(),
                tx_errors: network.total_errors_on_transmitted(),
                rx_dropped: 0,
                tx_dropped: 0,
            };

            // Calculate rates if we have previous stats
            if let Some(prev_stats) = stats.get(name) {
                let rx_diff = current_stats.rx_bytes.saturating_sub(prev_stats.rx_bytes);
                let tx_diff = current_stats.tx_bytes.saturating_sub(prev_stats.tx_bytes);

                usage_list.push(NetworkUsage {
                    interface: name.clone(),
                    rx_rate: rx_diff as f64,
                    tx_rate: tx_diff as f64,
                    total_rx: current_stats.rx_bytes,
                    total_tx: current_stats.tx_bytes,
                });
            } else {
                usage_list.push(NetworkUsage {
                    interface: name.clone(),
                    rx_rate: 0.0,
                    tx_rate: 0.0,
                    total_rx: current_stats.rx_bytes,
                    total_tx: current_stats.tx_bytes,
                });
            }

            // Update stats cache
            stats.insert(name.clone(), current_stats);
        }

        usage_list
    }

    #[cfg(not(feature = "network"))]
    {
        Vec::new()
    }
}

/// Get active network connections
pub fn connections() -> Vec<Connection> {
    // This would require platform-specific implementation
    // or additional dependencies like netstat2
    Vec::new()
}

/// Get connections for a specific process
pub fn connections_for_process(pid: u32) -> Vec<Connection> {
    connections()
        .into_iter()
        .filter(|conn| conn.pid == Some(pid))
        .collect()
}

/// Network configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NetworkConfig {
    /// Hostname
    pub hostname: String,
    /// DNS servers
    pub dns_servers: Vec<String>,
    /// Default gateway
    pub gateway: Option<String>,
    /// Domain name
    pub domain: Option<String>,
}

/// Get network configuration
pub fn config() -> NetworkConfig {
    use crate::info::SystemInfo;

    let info = SystemInfo::get();

    NetworkConfig {
        hostname: info.os.hostname.clone(),
        dns_servers: detect_dns_servers(),
        gateway: detect_gateway(),
        domain: None,
    }
}

fn detect_dns_servers() -> Vec<String> {
    #[cfg(unix)]
    {
        use std::fs;

        if let Ok(content) = fs::read_to_string("/etc/resolv.conf") {
            return content
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if line.starts_with("nameserver") {
                        line.split_whitespace().nth(1).map(String::from)
                    } else {
                        None
                    }
                })
                .collect();
        }
    }

    #[cfg(windows)]
    {
        // Would need Windows-specific implementation
    }

    Vec::new()
}

fn detect_gateway() -> Option<String> {
    #[cfg(unix)]
    {
        use std::process::Command;

        // Try to get default gateway using ip command
        if let Ok(output) = Command::new("ip")
            .args(&["route", "show", "default"])
            .output()
        {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                // Parse: "default via 192.168.1.1 dev eth0"
                for line in stdout.lines() {
                    if line.starts_with("default") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 3 && parts[1] == "via" {
                            return Some(parts[2].to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Check network connectivity
pub fn is_online() -> bool {
    // Simple check: see if we have any non-loopback interfaces with IP addresses
    interfaces()
        .iter()
        .any(|iface| !iface.is_loopback && iface.is_up)
}

/// Get total network statistics across all interfaces
pub fn total_stats() -> NetworkStats {
    let interfaces = interfaces();

    interfaces.iter().fold(NetworkStats::default(), |mut acc, iface| {
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