//! NUMA (Non-Uniform Memory Access) support
//!
//! This module provides abstractions and utilities for working with NUMA
//! architectures, allowing memory to be allocated and accessed efficiently on
//! multi-socket systems.

use std::io;
#[cfg(feature = "numa-aware")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "numa-aware")]
use numa;

/// NUMA node identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NumaNodeId(pub usize);

/// NUMA topology information
#[derive(Debug)]
pub struct NumaTopology {
    /// Number of NUMA nodes in the system
    pub node_count: usize,
    /// CPU IDs for each NUMA node
    pub node_cpus: Vec<Vec<usize>>,
    /// Memory size for each NUMA node
    pub node_memory: Vec<usize>,
    /// Distance matrix between nodes (higher = more distant)
    pub distance_matrix: Vec<Vec<usize>>,
}

/// NUMA allocation policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumaPolicy {
    /// Allocate on the local node of the current thread
    Local,
    /// Allocate on the specified node
    Node(NumaNodeId),
    /// Interleave allocation across all nodes
    Interleave,
    /// Interleave across specified nodes
    InterleaveNodes(Vec<NumaNodeId>),
    /// Preferred node, but allow fallback
    Preferred(NumaNodeId),
}

/// NUMA memory manager to handle NUMA-aware memory allocations
#[cfg(feature = "numa-aware")]
pub struct NumaMemoryManager {
    node_count: usize,
    current_node: AtomicUsize,
    topology: Option<NumaTopology>,
    enabled: bool,
}

#[cfg(feature = "numa-aware")]
impl NumaMemoryManager {
    /// Create a new NUMA memory manager
    pub fn new() -> io::Result<Self> {
        let node_count = detect_numa_nodes()?;
        let topology = if node_count > 1 { Some(detect_numa_topology()?) } else { None };

        Ok(Self {
            node_count,
            current_node: AtomicUsize::new(0),
            topology,
            enabled: node_count > 1,
        })
    }

    /// Check if NUMA is available and enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the number of NUMA nodes
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Get the NUMA topology
    pub fn topology(&self) -> Option<&NumaTopology> {
        self.topology.as_ref()
    }

    /// Get the current node for the calling thread
    pub fn current_node(&self) -> NumaNodeId {
        #[cfg(target_os = "linux")]
        {
            if let Ok(node) = crate::platform::linux::get_current_numa_node() {
                return NumaNodeId(node);
            }
        }

        // Fallback to round-robin
        let node = self.current_node.fetch_add(1, Ordering::Relaxed) % self.node_count;
        self.current_node.store(node, Ordering::Relaxed);
        NumaNodeId(node)
    }

    /// Allocate memory with NUMA policy
    pub fn allocate(&self, size: usize, policy: NumaPolicy) -> io::Result<*mut u8> {
        if !self.enabled {
            // Fall back to regular allocation
            let layout = std::alloc::Layout::from_size_align(size, 64)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "NUMA allocation failed"));
            }

            return Ok(ptr);
        }

        match policy {
            NumaPolicy::Local => self.allocate_on_node(size, self.current_node()),
            NumaPolicy::Node(node) => self.allocate_on_node(size, node),
            NumaPolicy::Interleave => self.allocate_interleaved(size, None),
            NumaPolicy::InterleaveNodes(nodes) => self.allocate_interleaved(size, Some(&nodes)),
            NumaPolicy::Preferred(node) => self.allocate_preferred(size, node),
        }
    }

    /// Deallocate NUMA memory
    pub unsafe fn deallocate(&self, ptr: *mut u8, size: usize) {
        if !self.enabled {
            // Regular deallocation
            let layout = std::alloc::Layout::from_size_align(size, 64)
                .unwrap_or_else(|_| std::alloc::Layout::from_size_align(size, 8).unwrap());

            std::alloc::dealloc(ptr, layout);
            return;
        }

        #[cfg(target_os = "linux")]
        {
            libc::free(ptr as *mut libc::c_void);
            return;
        }

        // Fallback for non-Linux
        let layout = std::alloc::Layout::from_size_align(size, 64)
            .unwrap_or_else(|_| std::alloc::Layout::from_size_align(size, 8).unwrap());

        std::alloc::dealloc(ptr, layout);
    }

    /// Allocate memory on a specific NUMA node
    fn allocate_on_node(&self, size: usize, node: NumaNodeId) -> io::Result<*mut u8> {
        #[cfg(target_os = "linux")]
        {
            use libc::c_void;

            if !numa::numa_available() {
                return Err(io::Error::new(io::ErrorKind::Other, "NUMA not available"));
            }

            // Temporarily set the memory allocation policy to this node
            let prev_mask = numa::numa_get_membind();

            let node_mask = numa::numa_allocate_nodemask();
            numa::numa_bitmask_clearall(node_mask);
            numa::numa_bitmask_setbit(node_mask, node.0);
            numa::numa_bind(node_mask);

            // Allocate memory
            let ptr = unsafe { libc::malloc(size) as *mut u8 };

            // Restore previous policy
            numa::numa_set_membind(prev_mask);
            numa::numa_free_nodemask(node_mask);

            if ptr.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "NUMA allocation failed"));
            }

            Ok(ptr)
        }

        #[cfg(not(target_os = "linux"))]
        {
            // Fallback for non-Linux
            let layout = std::alloc::Layout::from_size_align(size, 64)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "NUMA allocation failed"));
            }

            Ok(ptr)
        }
    }

    /// Allocate memory interleaved across NUMA nodes
    fn allocate_interleaved(
        &self,
        size: usize,
        nodes: Option<&[NumaNodeId]>,
    ) -> io::Result<*mut u8> {
        #[cfg(target_os = "linux")]
        {
            use libc::c_void;

            if !numa::numa_available() {
                return Err(io::Error::new(io::ErrorKind::Other, "NUMA not available"));
            }

            // Set interleave policy
            let prev_mask = numa::numa_get_interleave_mask();

            let node_mask = numa::numa_allocate_nodemask();
            if let Some(nodes) = nodes {
                // Interleave across specified nodes
                numa::numa_bitmask_clearall(node_mask);
                for node in nodes {
                    numa::numa_bitmask_setbit(node_mask, node.0);
                }
            } else {
                // Interleave across all nodes
                numa::numa_bitmask_setall(node_mask);
            }

            numa::numa_set_interleave_mask(node_mask);

            // Allocate memory
            let ptr = unsafe { libc::malloc(size) as *mut u8 };

            // Restore previous policy
            numa::numa_set_interleave_mask(prev_mask);
            numa::numa_free_nodemask(node_mask);

            if ptr.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "NUMA allocation failed"));
            }

            Ok(ptr)
        }

        #[cfg(not(target_os = "linux"))]
        {
            // Fallback for non-Linux
            let layout = std::alloc::Layout::from_size_align(size, 64)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "NUMA allocation failed"));
            }

            Ok(ptr)
        }
    }

    /// Allocate memory with preferred NUMA node
    fn allocate_preferred(&self, size: usize, node: NumaNodeId) -> io::Result<*mut u8> {
        #[cfg(target_os = "linux")]
        {
            use libc::c_void;

            if !numa::numa_available() {
                return Err(io::Error::new(io::ErrorKind::Other, "NUMA not available"));
            }

            // Set preferred policy
            let prev_mode = numa::numa_preferred();
            numa::numa_set_preferred(node.0);

            // Allocate memory
            let ptr = unsafe { libc::malloc(size) as *mut u8 };

            // Restore previous policy
            numa::numa_set_preferred(prev_mode);

            if ptr.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "NUMA allocation failed"));
            }

            Ok(ptr)
        }

        #[cfg(not(target_os = "linux"))]
        {
            // Fallback for non-Linux
            let layout = std::alloc::Layout::from_size_align(size, 64)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "NUMA allocation failed"));
            }

            Ok(ptr)
        }
    }
}

/// Detect number of NUMA nodes in the system
pub fn detect_numa_nodes() -> io::Result<usize> {
    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::get_numa_node_count()
    }

    #[cfg(windows)]
    {
        if let Some(get_numa_node_count) = crate::platform::windows::get_numa_node_count_fn() {
            let count = get_numa_node_count();
            if count > 0 {
                return Ok(count);
            }
        }

        // Return 1 if NUMA is not detected or not supported
        Ok(1)
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    {
        // Most other platforms either don't support NUMA or don't expose it
        Ok(1)
    }
}

/// Detect NUMA topology
pub fn detect_numa_topology() -> io::Result<NumaTopology> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        use std::path::Path;

        let node_count = crate::platform::linux::get_numa_node_count()?;
        let mut node_cpus = Vec::with_capacity(node_count);
        let mut node_memory = Vec::with_capacity(node_count);
        let mut distance_matrix = Vec::with_capacity(node_count);

        for node in 0..node_count {
            // Get CPUs for this node
            let cpus_path = format!("/sys/devices/system/node/node{}/cpulist", node);
            if let Ok(cpulist) = fs::read_to_string(&cpus_path) {
                let cpus = parse_cpu_list(&cpulist.trim());
                node_cpus.push(cpus);
            } else {
                node_cpus.push(Vec::new());
            }

            // Get memory for this node
            let mem_path = format!("/sys/devices/system/node/node{}/meminfo", node);
            if let Ok(meminfo) = fs::read_to_string(&mem_path) {
                let mut memory = 0;
                for line in meminfo.lines() {
                    if line.starts_with("Node") && line.contains("MemTotal") {
                        if let Some(value) = line.split_whitespace().nth(3) {
                            if let Ok(kb) = value.parse::<usize>() {
                                memory = kb * 1024; // Convert KB to bytes
                                break;
                            }
                        }
                    }
                }
                node_memory.push(memory);
            } else {
                node_memory.push(0);
            }

            // Get distance matrix
            let dist_path = format!("/sys/devices/system/node/node{}/distance", node);
            if let Ok(distances) = fs::read_to_string(&dist_path) {
                let node_distances: Vec<usize> =
                    distances.split_whitespace().filter_map(|s| s.parse::<usize>().ok()).collect();
                distance_matrix.push(node_distances);
            } else {
                // If distance file doesn't exist, use a simple distance model
                let mut node_distances = Vec::with_capacity(node_count);
                for i in 0..node_count {
                    if i == node {
                        node_distances.push(10); // Local distance
                    } else {
                        node_distances.push(20); // Remote distance
                    }
                }
                distance_matrix.push(node_distances);
            }
        }

        Ok(NumaTopology { node_count, node_cpus, node_memory, distance_matrix })
    }

    #[cfg(not(target_os = "linux"))]
    {
        // For non-Linux platforms, create a simple NUMA topology
        let node_count = detect_numa_nodes()?;
        let cpu_count = num_cpus::get();
        let memory = crate::platform::get_total_memory();

        let cpus_per_node = cpu_count / node_count;
        let memory_per_node = memory / node_count;

        let mut node_cpus = Vec::with_capacity(node_count);
        let mut node_memory = Vec::with_capacity(node_count);
        let mut distance_matrix = Vec::with_capacity(node_count);

        for node in 0..node_count {
            // Assign CPUs to nodes
            let mut cpus = Vec::new();
            for cpu in 0..cpus_per_node {
                cpus.push(node * cpus_per_node + cpu);
            }
            node_cpus.push(cpus);

            // Assign memory to nodes
            node_memory.push(memory_per_node);

            // Create distance matrix
            let mut node_distances = Vec::with_capacity(node_count);
            for i in 0..node_count {
                if i == node {
                    node_distances.push(10); // Local distance
                } else {
                    node_distances.push(20); // Remote distance
                }
            }
            distance_matrix.push(node_distances);
        }

        Ok(NumaTopology { node_count, node_cpus, node_memory, distance_matrix })
    }
}

/// Parse CPU list from Linux sysfs
fn parse_cpu_list(cpulist: &str) -> Vec<usize> {
    let mut cpus = Vec::new();

    for part in cpulist.split(',') {
        if part.contains('-') {
            let range: Vec<&str> = part.split('-').collect();
            if range.len() == 2 {
                if let (Ok(start), Ok(end)) = (range[0].parse::<usize>(), range[1].parse::<usize>())
                {
                    for cpu in start..=end {
                        cpus.push(cpu);
                    }
                }
            }
        } else {
            if let Ok(cpu) = part.parse::<usize>() {
                cpus.push(cpu);
            }
        }
    }

    cpus
}

#[cfg(target_os = "linux")]
pub fn get_current_numa_node() -> io::Result<usize> {
    use std::fs;

    let cpu_id = get_current_cpu()?;

    for node in 0..64 {
        // Arbitrary upper limit
        let cpulist_path = format!("/sys/devices/system/node/node{}/cpulist", node);
        if !std::path::Path::new(&cpulist_path).exists() {
            continue;
        }

        if let Ok(cpulist) = fs::read_to_string(&cpulist_path) {
            let cpus = parse_cpu_list(&cpulist.trim());
            if cpus.contains(&cpu_id) {
                return Ok(node);
            }
        }
    }

    // Default to node 0 if we can't determine it
    Ok(0)
}

#[cfg(target_os = "linux")]
fn get_current_cpu() -> io::Result<usize> {
    use std::fs;

    // Try to get current CPU from /proc/self/stat
    if let Ok(content) = fs::read_to_string("/proc/self/stat") {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() >= 39 {
            if let Ok(cpu) = parts[38].parse::<usize>() {
                return Ok(cpu);
            }
        }
    }

    // Fallback: use CPU 0
    Ok(0)
}
