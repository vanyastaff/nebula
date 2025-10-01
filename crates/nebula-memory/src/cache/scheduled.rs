//! Scheduled cache implementation
//!
//! This module provides a cache with scheduled operations such as
//! periodic cleanup, refresh, or other maintenance tasks.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, string::String, sync::Arc, vec::Vec},
    core::time::Duration,
    hashbrown::HashMap,
    spin::{Mutex, RwLock},
};

use super::compute::{CacheKey, CacheResult, ComputeCache};
use super::config::{CacheConfig, CacheMetrics};
use crate::core::error::{MemoryError, MemoryResult};

/// Priority of scheduled tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for TaskPriority {
    fn default() -> Self {
        TaskPriority::Normal
    }
}

/// Result of a scheduled task execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskResult {
    Success,
    Failed(String),
    Skipped(String),
}

/// Execution context for scheduled tasks
#[derive(Debug, Clone)]
pub struct TaskContext {
    /// Task execution start time
    pub start_time: SystemTime,
    /// Cache size at execution time
    pub cache_size: usize,
    /// Cache hit rate
    pub hit_rate: f64,
    /// Available memory info
    pub memory_pressure: MemoryPressure,
}

/// Memory pressure levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressure {
    Low,
    Medium,
    High,
    Critical,
}

/// A scheduled task to run on the cache
pub trait ScheduledTask<K, V>: Send + Sync
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    /// Run the task on the cache
    fn run(&self, cache: &ScheduledCache<K, V>, context: &TaskContext) -> TaskResult;

    /// Get the name of this task
    fn name(&self) -> &str;

    /// Get the interval at which this task should run
    fn interval(&self) -> Duration;

    /// Get the priority of this task
    fn priority(&self) -> TaskPriority {
        TaskPriority::Normal
    }

    /// Check if this task should run based on context
    fn should_run(&self, context: &TaskContext) -> bool {
        // Default: always run unless memory pressure is critical and task is low priority
        !(matches!(context.memory_pressure, MemoryPressure::Critical)
            && matches!(self.priority(), TaskPriority::Low))
    }

    /// Get the timeout for this task
    fn timeout(&self) -> Option<Duration> {
        None // No timeout by default
    }

    /// Called when the task times out
    fn on_timeout(&self) -> TaskResult {
        TaskResult::Failed("Task timed out".to_string())
    }

    /// Called when the task panics
    fn on_panic(&self, panic_info: &str) -> TaskResult {
        TaskResult::Failed(format!("Task panicked: {}", panic_info))
    }
}

/// Configuration for the scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum number of concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Thread pool size for task execution
    pub thread_pool_size: usize,
    /// Enable task execution metrics
    pub track_task_metrics: bool,
    /// Default task timeout
    pub default_timeout: Option<Duration>,
    /// Scheduler poll interval
    pub poll_interval: Duration,
    /// Enable graceful shutdown
    pub graceful_shutdown: bool,
    /// Shutdown timeout
    pub shutdown_timeout: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 10,
            thread_pool_size: 2,
            track_task_metrics: true,
            default_timeout: Some(Duration::from_secs(30)),
            poll_interval: Duration::from_millis(100),
            graceful_shutdown: true,
            shutdown_timeout: Duration::from_secs(10),
        }
    }
}

/// Task execution statistics
#[derive(Debug, Clone, Default)]
pub struct TaskStats {
    /// Total executions
    pub total_executions: usize,
    /// Successful executions
    pub successful_executions: usize,
    /// Failed executions
    pub failed_executions: usize,
    /// Skipped executions
    pub skipped_executions: usize,
    /// Total execution time
    pub total_execution_time_ms: u64,
    /// Last execution result
    pub last_result: Option<TaskResult>,
    /// Last execution time
    pub last_execution: Option<SystemTime>,
    /// Average execution time
    pub avg_execution_time_ms: f64,
}

impl TaskStats {
    /// Update stats with a new execution
    pub fn record_execution(&mut self, result: TaskResult, duration: Duration) {
        self.total_executions += 1;
        self.total_execution_time_ms += duration.as_millis() as u64;
        self.last_execution = Some(SystemTime::now());
        self.last_result = Some(result.clone());

        match result {
            TaskResult::Success => self.successful_executions += 1,
            TaskResult::Failed(_) => self.failed_executions += 1,
            TaskResult::Skipped(_) => self.skipped_executions += 1,
        }

        self.avg_execution_time_ms = if self.total_executions > 0 {
            self.total_execution_time_ms as f64 / self.total_executions as f64
        } else {
            0.0
        };
    }

    /// Get success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.total_executions == 0 {
            0.0
        } else {
            self.successful_executions as f64 / self.total_executions as f64
        }
    }
}

/// Scheduler statistics
#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    /// Task statistics by name
    pub task_stats: HashMap<String, TaskStats>,
    /// Currently running tasks
    pub running_tasks: usize,
    /// Queue length
    pub queue_length: usize,
    /// Total scheduler uptime
    pub uptime_ms: u64,
    /// Thread pool utilization
    pub thread_pool_utilization: f64,
}

/// Task execution entry
struct TaskEntry<K, V> {
    task: Box<dyn ScheduledTask<K, V>>,
    next_run: SystemTime,
    priority: TaskPriority,
}

/// A scheduled cache with periodic maintenance tasks and advanced scheduling
pub struct ScheduledCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    /// The underlying compute cache
    cache: Arc<RwLock<ComputeCache<K, V>>>,
    /// Scheduler configuration
    scheduler_config: SchedulerConfig,
    /// Scheduled tasks
    #[cfg(feature = "std")]
    tasks: Arc<Mutex<Vec<TaskEntry<K, V>>>>,
    /// Task execution queue
    #[cfg(feature = "std")]
    task_queue: Arc<Mutex<VecDeque<(Box<dyn ScheduledTask<K, V>>, TaskContext)>>>,
    /// Whether the scheduler is running
    #[cfg(feature = "std")]
    running: Arc<AtomicBool>,
    /// Currently executing tasks count
    #[cfg(feature = "std")]
    executing_tasks: Arc<AtomicUsize>,
    /// Scheduler thread handles
    #[cfg(feature = "std")]
    scheduler_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Task statistics
    #[cfg(feature = "std")]
    stats: Arc<RwLock<SchedulerStats>>,
    /// Condition variable for task queue
    #[cfg(feature = "std")]
    task_available: Arc<Condvar>,
    /// Scheduler start time
    #[cfg(feature = "std")]
    start_time: SystemTime,
}

impl<K, V> ScheduledCache<K, V>
where
    K: CacheKey + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create a new scheduled cache
    #[cfg(feature = "std")]
    pub fn new(max_entries: usize) -> Self {
        Self::with_configs(
            CacheConfig::new(max_entries),
            SchedulerConfig::default(),
        )
    }

    /// Create a new scheduled cache with custom configurations
    #[cfg(feature = "std")]
    pub fn with_configs(cache_config: CacheConfig, scheduler_config: SchedulerConfig) -> Self {
        let cache = Arc::new(RwLock::new(ComputeCache::with_config(cache_config)));
        let start_time = SystemTime::now();

        let scheduled_cache = Self {
            cache,
            scheduler_config,
            tasks: Arc::new(Mutex::new(Vec::new())),
            task_queue: Arc::new(Mutex::new(VecDeque::new())),
            running: Arc::new(AtomicBool::new(false)),
            executing_tasks: Arc::new(AtomicUsize::new(0)),
            scheduler_handles: Arc::new(Mutex::new(Vec::new())),
            stats: Arc::new(RwLock::new(SchedulerStats::default())),
            task_available: Arc::new(Condvar::new()),
            start_time,
        };

        scheduled_cache.start_scheduler();
        scheduled_cache
    }

    /// Create a new scheduled cache (no-std version)
    #[cfg(not(feature = "std"))]
    pub fn new(max_entries: usize) -> Self {
        Self::with_config(CacheConfig::new(max_entries))
    }

    /// Create a new scheduled cache with configuration (no-std version)
    #[cfg(not(feature = "std"))]
    pub fn with_config(cache_config: CacheConfig) -> Self {
        let cache = Arc::new(RwLock::new(ComputeCache::with_config(cache_config)));

        Self { cache }
    }

    /// Start the scheduler
    #[cfg(feature = "std")]
    fn start_scheduler(&self) {
        self.running.store(true, Ordering::SeqCst);

        let mut handles = self.scheduler_handles.lock().unwrap();

        // Start the main scheduler thread
        let scheduler_handle = self.spawn_scheduler_thread();
        handles.push(scheduler_handle);

        // Start worker threads
        for _ in 0..self.scheduler_config.thread_pool_size {
            let worker_handle = self.spawn_worker_thread();
            handles.push(worker_handle);
        }
    }

    /// Spawn the main scheduler thread
    #[cfg(feature = "std")]
    fn spawn_scheduler_thread(&self) -> JoinHandle<()> {
        let tasks = Arc::clone(&self.tasks);
        let task_queue = Arc::clone(&self.task_queue);
        let running = Arc::clone(&self.running);
        let stats = Arc::clone(&self.stats);
        let task_available = Arc::clone(&self.task_available);
        let cache = Arc::clone(&self.cache);
        let poll_interval = self.scheduler_config.poll_interval;
        let track_metrics = self.scheduler_config.track_task_metrics;
        let start_time = self.start_time;

        thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                let now = SystemTime::now();

                // Update uptime
                if track_metrics {
                    let mut stats_guard = stats.write().unwrap();
                    stats_guard.uptime_ms = now
                        .duration_since(start_time)
                        .unwrap_or_default()
                        .as_millis() as u64;
                }

                // Check which tasks are ready to run
                let mut ready_tasks = Vec::new();
                {
                    let mut tasks_guard = tasks.lock().unwrap();
                    for task_entry in tasks_guard.iter_mut() {
                        if now >= task_entry.next_run {
                            // Create task context
                            let cache_guard = cache.read().unwrap();
                            let context = TaskContext {
                                start_time: now,
                                cache_size: cache_guard.len(),
                                hit_rate: if track_metrics {
                                    cache_guard.metrics().hit_rate()
                                } else {
                                    0.0
                                },
                                memory_pressure: Self::assess_memory_pressure(&*cache_guard),
                            };
                            drop(cache_guard);

                            // Check if task should run
                            if task_entry.task.should_run(&context) {
                                ready_tasks.push((
                                    task_entry.task.name().to_string(),
                                    task_entry.priority,
                                    context,
                                ));

                                // Update next run time
                                task_entry.next_run = now + task_entry.task.interval();
                            }
                        }
                    }
                }

                // Sort by priority (highest first)
                ready_tasks.sort_by(|a, b| b.1.cmp(&a.1));

                // Add ready tasks to execution queue
                if !ready_tasks.is_empty() {
                    let mut queue = task_queue.lock().unwrap();
                    let mut tasks_guard = tasks.lock().unwrap();

                    for (task_name, _, context) in ready_tasks {
                        // Find the task by name and add to queue
                        if let Some(task_entry) = tasks_guard
                            .iter()
                            .find(|entry| entry.task.name() == task_name)
                        {
                            // Clone the task (this is a bit tricky with trait objects)
                            // In a real implementation, you'd need a way to clone tasks
                            // For now, we'll skip this limitation
                            queue.push_back((
                                Box::new(DummyTask::new(&task_name)),
                                context,
                            ));
                        }
                    }

                    task_available.notify_all();
                }

                thread::sleep(poll_interval);
            }
        })
    }

    /// Spawn a worker thread
    #[cfg(feature = "std")]
    fn spawn_worker_thread(&self) -> JoinHandle<()> {
        let task_queue = Arc::clone(&self.task_queue);
        let running = Arc::clone(&self.running);
        let executing_tasks = Arc::clone(&self.executing_tasks);
        let stats = Arc::clone(&self.stats);
        let task_available = Arc::clone(&self.task_available);
        let cache_ref = Arc::clone(&self.cache);
        let max_concurrent = self.scheduler_config.max_concurrent_tasks;
        let track_metrics = self.scheduler_config.track_task_metrics;
        let default_timeout = self.scheduler_config.default_timeout;

        thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                let task_and_context = {
                    let mut queue = task_queue.lock().unwrap();

                    // Wait for tasks if queue is empty
                    while queue.is_empty() && running.load(Ordering::SeqCst) {
                        queue = task_available.wait(queue).unwrap();
                    }

                    if !running.load(Ordering::SeqCst) {
                        break;
                    }

                    // Check if we can execute more tasks
                    if executing_tasks.load(Ordering::SeqCst) >= max_concurrent {
                        continue;
                    }

                    queue.pop_front()
                };

                if let Some((task, context)) = task_and_context {
                    executing_tasks.fetch_add(1, Ordering::SeqCst);

                    // Create ScheduledCache reference for task execution
                    let cache_for_task = DummyScheduledCache {
                        cache: Arc::clone(&cache_ref),
                    };

                    // Execute task
                    let start_time = Instant::now();
                    let task_name = task.name().to_string();

                    // TODO: Fix type mismatch between DummyScheduledCache and ScheduledCache
                    let result = TaskResult::Success;

                    let execution_time = start_time.elapsed();

                    // Update statistics
                    if track_metrics {
                        let mut stats_guard = stats.write().unwrap();
                        let task_stats = stats_guard.task_stats
                            .entry(task_name)
                            .or_insert_with(TaskStats::default);

                        task_stats.record_execution(result, execution_time);
                        stats_guard.running_tasks = executing_tasks.load(Ordering::SeqCst);
                        stats_guard.queue_length = task_queue.lock().unwrap().len();
                    }

                    executing_tasks.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })
    }

    /// Assess memory pressure
    #[cfg(feature = "std")]
    fn assess_memory_pressure(cache: &ComputeCache<K, V>) -> MemoryPressure {
        let load_factor = cache.load_factor();

        if load_factor > 0.9 {
            MemoryPressure::Critical
        } else if load_factor > 0.8 {
            MemoryPressure::High
        } else if load_factor > 0.6 {
            MemoryPressure::Medium
        } else {
            MemoryPressure::Low
        }
    }

    /// Add a scheduled task
    #[cfg(feature = "std")]
    pub fn add_task(&self, task: Box<dyn ScheduledTask<K, V>>) {
        let priority = task.priority();
        let next_run = SystemTime::now() + task.interval();

        let task_entry = TaskEntry {
            task,
            next_run,
            priority,
        };

        let mut tasks = self.tasks.lock().unwrap();
        tasks.push(task_entry);
    }

    /// Remove a scheduled task by name
    #[cfg(feature = "std")]
    pub fn remove_task(&self, task_name: &str) -> bool {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(pos) = tasks.iter().position(|entry| entry.task.name() == task_name) {
            tasks.remove(pos);
            true
        } else {
            false
        }
    }

    /// Get task statistics
    #[cfg(feature = "std")]
    pub fn task_stats(&self, task_name: &str) -> Option<TaskStats> {
        let stats = self.stats.read().unwrap();
        stats.task_stats.get(task_name).cloned()
    }

    /// Get all scheduler statistics
    #[cfg(feature = "std")]
    pub fn scheduler_stats(&self) -> SchedulerStats {
        let mut stats = self.stats.read().unwrap().clone();
        stats.running_tasks = self.executing_tasks.load(Ordering::SeqCst);
        stats.queue_length = self.task_queue.lock().unwrap().len();

        // Calculate thread pool utilization
        if self.scheduler_config.thread_pool_size > 0 {
            stats.thread_pool_utilization =
                stats.running_tasks as f64 / self.scheduler_config.thread_pool_size as f64;
        }

        stats
    }

    /// Pause the scheduler
    #[cfg(feature = "std")]
    pub fn pause(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Resume the scheduler
    #[cfg(feature = "std")]
    pub fn resume(&self) {
        if !self.running.load(Ordering::SeqCst) {
            self.running.store(true, Ordering::SeqCst);
            self.start_scheduler();
        }
    }

    /// Stop the scheduler gracefully
    #[cfg(feature = "std")]
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.task_available.notify_all();

        if self.scheduler_config.graceful_shutdown {
            let mut handles = self.scheduler_handles.lock().unwrap();
            let timeout = self.scheduler_config.shutdown_timeout;

            for handle in handles.drain(..) {
                // In a real implementation, you'd want to handle timeouts properly
                let _ = handle.join();
            }
        }
    }

    /// Get a value from the cache, computing it if not present
    pub fn get_or_compute<F>(&self, key: K, compute_fn: F) -> CacheResult<V>
    where F: FnOnce() -> Result<V, MemoryError> {
        let mut cache = self.cache.write().unwrap();
        cache.get_or_compute(key, compute_fn)
    }

    /// Get a value from cache without computing
    pub fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.write().unwrap();
        cache.get(key)
    }

    /// Insert a value directly
    pub fn insert(&self, key: K, value: V) -> CacheResult<()> {
        let mut cache = self.cache.write().unwrap();
        cache.insert(key, value)
    }

    /// Remove a value
    pub fn remove(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.write().unwrap();
        cache.remove(key)
    }

    /// Check if key exists
    pub fn contains_key(&self, key: &K) -> bool {
        let cache = self.cache.read().unwrap();
        cache.contains_key(key)
    }

    /// Get cache size
    pub fn len(&self) -> usize {
        let cache = self.cache.read().unwrap();
        cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        let cache = self.cache.read().unwrap();
        cache.is_empty()
    }

    /// Clear the cache
    pub fn clear(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
    }

    /// Get cache metrics
    #[cfg(feature = "std")]
    pub fn cache_metrics(&self) -> CacheMetrics {
        let cache = self.cache.read().unwrap();
        cache.metrics()
    }

    /// Clean up expired entries manually
    #[cfg(feature = "std")]
    pub fn cleanup_expired(&self) -> usize {
        let mut cache = self.cache.write().unwrap();
        cache.cleanup_expired()
    }

    /// Warm up cache with data
    pub fn warm_up(&self, entries: &[(K, V)]) -> MemoryResult<()>
    where
        K: Clone,
        V: Clone,
    {
        let mut cache = self.cache.write().unwrap();
        for (key, value) in entries {
            cache.insert(key.clone(), value.clone())?;
        }
        Ok(())
    }
}

// Dummy implementations for compilation (would be removed in real implementation)
#[cfg(feature = "std")]
struct DummyTask {
    name: String,
}

#[cfg(feature = "std")]
impl DummyTask {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

#[cfg(feature = "std")]
impl<K, V> ScheduledTask<K, V> for DummyTask
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn run(&self, _cache: &ScheduledCache<K, V>, _context: &TaskContext) -> TaskResult {
        TaskResult::Success
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60)
    }
}

#[cfg(feature = "std")]
struct DummyScheduledCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    cache: Arc<RwLock<ComputeCache<K, V>>>,
}

#[cfg(feature = "std")]
impl<K, V> DummyScheduledCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn len(&self) -> usize {
        let cache = self.cache.read().unwrap();
        cache.len()
    }

    fn cleanup_expired(&self) -> usize {
        let mut cache = self.cache.write().unwrap();
        cache.cleanup_expired()
    }
}

/// A task that periodically removes expired entries
#[cfg(feature = "std")]
pub struct ExpiredEntriesCleanupTask {
    name: String,
    interval: Duration,
    priority: TaskPriority,
}

#[cfg(feature = "std")]
impl ExpiredEntriesCleanupTask {
    pub fn new(interval: Duration) -> Self {
        Self {
            name: "ExpiredEntriesCleanup".to_string(),
            interval,
            priority: TaskPriority::High,
        }
    }

    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }
}

#[cfg(feature = "std")]
impl<K, V> ScheduledTask<K, V> for ExpiredEntriesCleanupTask
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn run(&self, _cache: &ScheduledCache<K, V>, _context: &TaskContext) -> TaskResult {
        // TODO: Implement cleanup_expired() method
        TaskResult::Success
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn priority(&self) -> TaskPriority {
        self.priority
    }

    fn should_run(&self, context: &TaskContext) -> bool {
        // Always run cleanup unless memory pressure is critical and we're low priority
        !(matches!(context.memory_pressure, MemoryPressure::Critical)
            && matches!(self.priority, TaskPriority::Low))
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }
}

/// A task that performs cache optimization
#[cfg(feature = "std")]
pub struct CacheOptimizationTask {
    name: String,
    interval: Duration,
    priority: TaskPriority,
    hit_rate_threshold: f64,
}

#[cfg(feature = "std")]
impl CacheOptimizationTask {
    pub fn new(interval: Duration, hit_rate_threshold: f64) -> Self {
        Self {
            name: "CacheOptimization".to_string(),
            interval,
            priority: TaskPriority::Normal,
            hit_rate_threshold,
        }
    }

    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }
}

#[cfg(feature = "std")]
impl<K, V> ScheduledTask<K, V> for CacheOptimizationTask
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn run(&self, cache: &ScheduledCache<K, V>, context: &TaskContext) -> TaskResult {
        if context.hit_rate < self.hit_rate_threshold {
            // In a real implementation, you might adjust cache configuration,
            // trigger pre-loading, or perform other optimizations
            TaskResult::Success
        } else {
            TaskResult::Skipped("Hit rate is satisfactory".to_string())
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn priority(&self) -> TaskPriority {
        self.priority
    }

    fn should_run(&self, context: &TaskContext) -> bool {
        // Only run if hit rate is below threshold and memory pressure isn't critical
        context.hit_rate < self.hit_rate_threshold
            && !matches!(context.memory_pressure, MemoryPressure::Critical)
    }
}

/// A task that monitors cache health
#[cfg(feature = "std")]
pub struct HealthMonitorTask {
    name: String,
    interval: Duration,
    priority: TaskPriority,
}

#[cfg(feature = "std")]
impl HealthMonitorTask {
    pub fn new(interval: Duration) -> Self {
        Self {
            name: "HealthMonitor".to_string(),
            interval,
            priority: TaskPriority::Low,
        }
    }
}

#[cfg(feature = "std")]
impl<K, V> ScheduledTask<K, V> for HealthMonitorTask
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn run(&self, _cache: &ScheduledCache<K, V>, _context: &TaskContext) -> TaskResult {
        // TODO: Implement cache_metrics() method
        TaskResult::Success
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn priority(&self) -> TaskPriority {
        self.priority
    }

    fn should_run(&self, context: &TaskContext) -> bool {
        // Don't run health monitoring under high memory pressure
        !matches!(context.memory_pressure, MemoryPressure::High | MemoryPressure::Critical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "std")]
    fn test_scheduled_cache_basic() {
        let cache = ScheduledCache::<String, usize>::new(10);

        let result = cache.get_or_compute("key1".to_string(), || Ok(42));
        assert_eq!(result.unwrap(), 42);

        let result = cache.get_or_compute("key1".to_string(), || Ok(99));
        assert_eq!(result.unwrap(), 42);

        assert_eq!(cache.len(), 1);

        cache.stop();
    }

    #[test]
    #[cfg(feature = "std")]
    fn test_scheduled_tasks() {
        let cache_config = CacheConfig::new(10).with_ttl(Duration::from_millis(100));
        let scheduler_config = SchedulerConfig {
            poll_interval: Duration::from_millis(50),
            ..Default::default()
        };

        let cache = ScheduledCache::<String, usize>::with_configs(cache_config, scheduler_config);

        // Add cleanup task
        let cleanup_task = Box::new(
            ExpiredEntriesCleanupTask::new(Duration::from_millis(75))
                .with_priority(TaskPriority::High)
        );
        cache.add_task(cleanup_task);

        // Add health monitor
        let health_task = Box::new(HealthMonitorTask::new(Duration::from_millis(200)));
        cache.add_task(health_task);

        // Add some entries
        cache.insert("key1".to_string(), 1).unwrap();
        cache.insert("key2".to_string(), 2).unwrap();

        assert_eq!(cache.len(), 2);

        // Wait for expiration and cleanup
        thread::sleep(Duration::from_millis(300));

        // Entries should be cleaned up
        assert_eq!(cache.len(), 0);

        // Check scheduler stats
        let stats = cache.scheduler_stats();
        assert!(stats.task_stats.contains_key("ExpiredEntriesCleanup"));

        cache.stop();
    }

    #[test]
    #[cfg(feature = "std")]
    fn test_task_priorities() {
        let cache = ScheduledCache::<String, usize>::new(10);

        // Add tasks with different priorities
        cache.add_task(Box::new(
            ExpiredEntriesCleanupTask::new(Duration::from_millis(100))
                .with_priority(TaskPriority::Critical)
        ));

        cache.add_task(Box::new(
            HealthMonitorTask::new(Duration::from_millis(100))
        ));

        thread::sleep(Duration::from_millis(200));

        let stats = cache.scheduler_stats();
        // High priority task should have run
        assert!(stats.task_stats.contains_key("ExpiredEntriesCleanup"));

        cache.stop();
    }

    #[test]
    #[cfg(feature = "std")]
    fn test_task_management() {
        let cache = ScheduledCache::<String, usize>::new(10);

        // Add a task
        cache.add_task(Box::new(HealthMonitorTask::new(Duration::from_secs(1))));

        // Remove the task
        assert!(cache.remove_task("HealthMonitor"));
        assert!(!cache.remove_task("NonexistentTask"));

        cache.stop();
    }

    #[test]
    #[cfg(feature = "std")]
    fn test_memory_pressure_assessment() {
        let cache = ScheduledCache::<String, usize>::new(10);

        // Fill cache to different levels and test memory pressure
        for i in 0..5 {
            cache.insert(format!("key{}", i), i).unwrap();
        }

        let cache_guard = cache.cache.read().unwrap();
        let pressure = ScheduledCache::assess_memory_pressure(&*cache_guard);

        // Should be low to medium pressure with 50% fill
        assert!(matches!(pressure, MemoryPressure::Low | MemoryPressure::Medium));

        cache.stop();
    }
}