# State Management Pattern

## Overview

State management patterns provide consistent approaches for handling resource state, ensuring data consistency, managing concurrent access, and enabling state persistence and recovery.

## State Architecture

```
┌─────────────────────────────────────────┐
│           State Manager                  │
├─────────────────────────────────────────┤
│  • State Store (In-Memory/Persistent)   │
│  • State Transitions                    │
│  • Concurrency Control                  │
│  • Change Notifications                 │
│  • State Snapshots                      │
└─────────────────────────────────────────┘
                    │
    ┌───────────────┼───────────────┐
    ▼               ▼               ▼
┌─────────┐   ┌─────────┐   ┌─────────┐
│Local    │   │Shared   │   │Distributed│
│State    │   │State    │   │State      │
└─────────┘   └─────────┘   └─────────┘
```

## Implementation

### Core State Management

```rust
use std::sync::Arc;
use tokio::sync::{RwLock, watch, broadcast};
use serde::{Serialize, Deserialize};
use std::time::{Duration, Instant};

/// Generic state container
#[derive(Debug, Clone)]
pub struct StateContainer<S: State> {
    /// Current state
    state: Arc<RwLock<S>>,
    
    /// State version
    version: Arc<AtomicU64>,
    
    /// State change broadcaster
    change_notifier: broadcast::Sender<StateChange<S>>,
    
    /// State history
    history: Arc<RwLock<StateHistory<S>>>,
    
    /// Persistence layer
    persistence: Option<Arc<dyn StatePersistence<S>>>,
    
    /// Configuration
    config: StateConfig,
}

/// State trait
pub trait State: Clone + Send + Sync + Serialize + for<'de> Deserialize<'de> {
    /// State identifier
    fn id(&self) -> &str;
    
    /// Validate state
    fn validate(&self) -> Result<()>;
    
    /// Merge with another state
    fn merge(&mut self, other: &Self) -> Result<()>;
    
    /// Get state metadata
    fn metadata(&self) -> StateMetadata;
}

#[derive(Debug, Clone)]
pub struct StateMetadata {
    pub created_at: Instant,
    pub updated_at: Instant,
    pub version: u64,
    pub checksum: Option<String>,
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct StateChange<S: State> {
    pub old_state: S,
    pub new_state: S,
    pub change_type: ChangeType,
    pub timestamp: Instant,
    pub change_id: Uuid,
}

#[derive(Debug, Clone)]
pub enum ChangeType {
    Create,
    Update,
    Delete,
    Merge,
    Rollback,
}

impl<S: State> StateContainer<S> {
    /// Create new state container
    pub fn new(initial_state: S, config: StateConfig) -> Self {
        let (tx, _) = broadcast::channel(config.change_buffer_size);
        
        Self {
            state: Arc::new(RwLock::new(initial_state)),
            version: Arc::new(AtomicU64::new(1)),
            change_notifier: tx,
            history: Arc::new(RwLock::new(StateHistory::new(config.history_size))),
            persistence: None,
            config,
        }
    }
    
    /// Get current state
    pub async fn get(&self) -> S {
        self.state.read().await.clone()
    }
    
    /// Update state
    pub async fn update<F>(&self, updater: F) -> Result<S>
    where
        F: FnOnce(&mut S) -> Result<()>,
    {
        let mut state = self.state.write().await;
        let old_state = state.clone();
        
        // Apply update
        updater(&mut *state)?;
        
        // Validate new state
        state.validate()?;
        
        // Increment version
        let new_version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        
        // Record change
        let change = StateChange {
            old_state: old_state.clone(),
            new_state: state.clone(),
            change_type: ChangeType::Update,
            timestamp: Instant::now(),
            change_id: Uuid::new_v4(),
        };
        
        // Add to history
        self.history.write().await.add(change.clone());
        
        // Notify subscribers
        let _ = self.change_notifier.send(change);
        
        // Persist if configured
        if let Some(ref persistence) = self.persistence {
            persistence.save(&*state, new_version).await?;
        }
        
        Ok(state.clone())
    }
    
    /// Compare and swap
    pub async fn compare_and_swap<F>(&self, expected: &S, updater: F) -> Result<bool>
    where
        F: FnOnce(&mut S) -> Result<()>,
    {
        let mut state = self.state.write().await;
        
        // Check if state matches expected
        if !self.states_equal(&*state, expected) {
            return Ok(false);
        }
        
        // Apply update
        updater(&mut *state)?;
        state.validate()?;
        
        // Update version and notify
        self.version.fetch_add(1, Ordering::SeqCst);
        
        Ok(true)
    }
    
    /// Subscribe to state changes
    pub fn subscribe(&self) -> broadcast::Receiver<StateChange<S>> {
        self.change_notifier.subscribe()
    }
    
    /// Get state version
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }
    
    /// Rollback to previous state
    pub async fn rollback(&self, steps: usize) -> Result<S> {
        let history = self.history.read().await;
        
        if let Some(historical_state) = history.get_previous(steps) {
            let mut state = self.state.write().await;
            let old_state = state.clone();
            
            *state = historical_state.clone();
            
            // Record rollback
            let change = StateChange {
                old_state,
                new_state: state.clone(),
                change_type: ChangeType::Rollback,
                timestamp: Instant::now(),
                change_id: Uuid::new_v4(),
            };
            
            let _ = self.change_notifier.send(change);
            
            Ok(state.clone())
        } else {
            Err(Error::NoHistoricalState)
        }
    }
    
    fn states_equal(&self, a: &S, b: &S) -> bool {
        // Implement state equality check
        serde_json::to_string(a).unwrap() == serde_json::to_string(b).unwrap()
    }
}
```

### Concurrent State Management

```rust
/// Lock-free state management using atomic operations
pub struct LockFreeState<S: AtomicState> {
    state: Arc<AtomicPtr<S>>,
    epoch: Arc<AtomicU64>,
    hazard_pointers: Arc<HazardPointerList>,
}

pub trait AtomicState: Send + Sync {
    /// Atomic compare and swap
    fn cas(&self, expected: &Self, new: Self) -> bool;
    
    /// Clone state atomically
    fn atomic_clone(&self) -> Self;
}

impl<S: AtomicState> LockFreeState<S> {
    /// Update state lock-free
    pub fn update<F>(&self, updater: F) -> Result<()>
    where
        F: Fn(&S) -> S,
    {
        loop {
            // Acquire hazard pointer
            let hp = self.hazard_pointers.acquire();
            
            // Load current state
            let current_ptr = self.state.load(Ordering::Acquire);
            let current = unsafe { &*current_ptr };
            hp.protect(current_ptr);
            
            // Create new state
            let new_state = updater(current);
            let new_ptr = Box::into_raw(Box::new(new_state));
            
            // Try to update
            match self.state.compare_exchange(
                current_ptr,
                new_ptr,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Success - increment epoch
                    self.epoch.fetch_add(1, Ordering::SeqCst);
                    
                    // Schedule old state for deletion
                    self.hazard_pointers.retire(current_ptr);
                    
                    return Ok(());
                }
                Err(_) => {
                    // Failed - cleanup and retry
                    unsafe {
                        Box::from_raw(new_ptr);
                    }
                }
            }
        }
    }
}

/// Multi-version concurrency control (MVCC)
pub struct MVCCState<S: State> {
    /// Version tree
    versions: Arc<RwLock<BTreeMap<u64, VersionedState<S>>>>,
    
    /// Current version
    current_version: Arc<AtomicU64>,
    
    /// Active transactions
    transactions: Arc<RwLock<HashMap<TransactionId, Transaction<S>>>>,
    
    /// Garbage collector
    gc: Arc<GarbageCollector>,
}

#[derive(Clone)]
pub struct VersionedState<S: State> {
    pub version: u64,
    pub state: S,
    pub timestamp: Instant,
    pub transaction_id: Option<TransactionId>,
}

impl<S: State> MVCCState<S> {
    /// Begin transaction
    pub async fn begin_transaction(&self) -> Transaction<S> {
        let tx_id = TransactionId::new();
        let snapshot_version = self.current_version.load(Ordering::SeqCst);
        
        let transaction = Transaction {
            id: tx_id,
            snapshot_version,
            read_set: HashSet::new(),
            write_set: HashMap::new(),
            state: TransactionState::Active,
        };
        
        self.transactions.write().await.insert(tx_id, transaction.clone());
        
        transaction
    }
    
    /// Read in transaction
    pub async fn read(&self, tx: &Transaction<S>, key: &str) -> Result<Option<S>> {
        // Read from write set first
        if let Some(state) = tx.write_set.get(key) {
            return Ok(Some(state.clone()));
        }
        
        // Read from versioned state
        let versions = self.versions.read().await;
        
        // Find visible version
        for (version, versioned_state) in versions.range(..=tx.snapshot_version).rev() {
            if self.is_visible(versioned_state, tx).await {
                return Ok(Some(versioned_state.state.clone()));
            }
        }
        
        Ok(None)
    }
    
    /// Write in transaction
    pub async fn write(&self, tx: &mut Transaction<S>, state: S) -> Result<()> {
        if tx.state != TransactionState::Active {
            return Err(Error::TransactionNotActive);
        }
        
        tx.write_set.insert(state.id().to_string(), state);
        Ok(())
    }
    
    /// Commit transaction
    pub async fn commit(&self, tx: &mut Transaction<S>) -> Result<()> {
        if tx.state != TransactionState::Active {
            return Err(Error::TransactionNotActive);
        }
        
        // Validate read set
        if !self.validate_read_set(tx).await {
            tx.state = TransactionState::Aborted;
            return Err(Error::TransactionConflict);
        }
        
        // Apply write set
        let new_version = self.current_version.fetch_add(1, Ordering::SeqCst) + 1;
        let mut versions = self.versions.write().await;
        
        for (key, state) in &tx.write_set {
            versions.insert(new_version, VersionedState {
                version: new_version,
                state: state.clone(),
                timestamp: Instant::now(),
                transaction_id: Some(tx.id),
            });
        }
        
        tx.state = TransactionState::Committed;
        
        // Trigger garbage collection
        self.gc.schedule_cleanup(tx.snapshot_version).await;
        
        Ok(())
    }
}
```

### Distributed State Management

```rust
/// Distributed state using Raft consensus
pub struct DistributedState<S: State> {
    /// Local state
    local_state: Arc<RwLock<S>>,
    
    /// Raft node
    raft_node: Arc<RaftNode>,
    
    /// State machine
    state_machine: Arc<StateMachine<S>>,
    
    /// Cluster configuration
    cluster_config: ClusterConfig,
}

impl<S: State> DistributedState<S> {
    /// Propose state change
    pub async fn propose(&self, change: StateChange<S>) -> Result<()> {
        // Serialize change
        let data = bincode::serialize(&change)?;
        
        // Propose through Raft
        self.raft_node.propose(data).await?;
        
        // Wait for consensus
        self.wait_for_consensus(change.change_id).await?;
        
        Ok(())
    }
    
    /// Read with consistency level
    pub async fn read(&self, consistency: ConsistencyLevel) -> Result<S> {
        match consistency {
            ConsistencyLevel::Strong => {
                // Read through Raft leader
                self.read_from_leader().await
            }
            ConsistencyLevel::BoundedStaleness(max_lag) => {
                // Check if local state is fresh enough
                let local_state = self.local_state.read().await;
                let lag = self.get_replication_lag().await;
                
                if lag <= max_lag {
                    Ok(local_state.clone())
                } else {
                    self.read_from_leader().await
                }
            }
            ConsistencyLevel::Eventual => {
                // Read from local state
                Ok(self.local_state.read().await.clone())
            }
        }
    }
    
    async fn read_from_leader(&self) -> Result<S> {
        let leader_id = self.raft_node.get_leader().await?;
        
        if leader_id == self.raft_node.id() {
            // We are the leader
            Ok(self.local_state.read().await.clone())
        } else {
            // Forward to leader
            self.forward_read_to_leader(leader_id).await
        }
    }
}

/// CRDT-based state for eventual consistency
pub struct CRDTState<S: CRDT> {
    /// Local CRDT state
    local: Arc<RwLock<S>>,
    
    /// Node ID
    node_id: NodeId,
    
    /// Vector clock
    vector_clock: Arc<RwLock<VectorClock>>,
    
    /// Anti-entropy protocol
    anti_entropy: Arc<AntiEntropy>,
}

pub trait CRDT: Clone + Send + Sync {
    /// Merge with another CRDT
    fn merge(&mut self, other: &Self);
    
    /// Get delta since version
    fn delta(&self, since: &VectorClock) -> Self;
}

impl<S: CRDT> CRDTState<S> {
    /// Update local state
    pub async fn update<F>(&self, updater: F) -> Result<()>
    where
        F: FnOnce(&mut S),
    {
        let mut local = self.local.write().await;
        let mut clock = self.vector_clock.write().await;
        
        // Update state
        updater(&mut *local);
        
        // Increment vector clock
        clock.increment(self.node_id);
        
        // Broadcast delta to peers
        let delta = local.delta(&clock);
        self.anti_entropy.broadcast_delta(delta, clock.clone()).await;
        
        Ok(())
    }
    
    /// Receive update from peer
    pub async fn receive_update(&self, delta: S, peer_clock: VectorClock) {
        let mut local = self.local.write().await;
        let mut clock = self.vector_clock.write().await;
        
        // Merge delta
        local.merge(&delta);
        
        // Update vector clock
        clock.merge(&peer_clock);
    }
}
```

### State Persistence

```rust
/// State persistence trait
#[async_trait]
pub trait StatePersistence<S: State>: Send + Sync {
    /// Save state
    async fn save(&self, state: &S, version: u64) -> Result<()>;
    
    /// Load state
    async fn load(&self) -> Result<Option<(S, u64)>>;
    
    /// Save snapshot
    async fn snapshot(&self, state: &S) -> Result<SnapshotId>;
    
    /// Restore from snapshot
    async fn restore(&self, snapshot_id: &SnapshotId) -> Result<S>;
    
    /// List snapshots
    async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>>;
}

/// File-based persistence
pub struct FilePersistence<S: State> {
    base_path: PathBuf,
    format: SerializationFormat,
    compression: Option<CompressionType>,
    _phantom: PhantomData<S>,
}

#[async_trait]
impl<S: State> StatePersistence<S> for FilePersistence<S> {
    async fn save(&self, state: &S, version: u64) -> Result<()> {
        let file_path = self.base_path.join(format!("state_v{}.dat", version));
        
        // Serialize state
        let data = match self.format {
            SerializationFormat::Json => serde_json::to_vec(state)?,
            SerializationFormat::Bincode => bincode::serialize(state)?,
            SerializationFormat::MessagePack => rmp_serde::to_vec(state)?,
        };
        
        // Compress if configured
        let final_data = if let Some(compression) = &self.compression {
            self.compress(data, compression)?
        } else {
            data
        };
        
        // Write atomically
        let temp_path = file_path.with_extension("tmp");
        tokio::fs::write(&temp_path, final_data).await?;
        tokio::fs::rename(temp_path, file_path).await?;
        
        Ok(())
    }
    
    async fn load(&self) -> Result<Option<(S, u64)>> {
        // Find latest version
        let mut entries = tokio::fs::read_dir(&self.base_path).await?;
        let mut latest_version = 0u64;
        let mut latest_file = None;
        
        while let Some(entry) = entries.next_entry().await? {
            let file_name = entry.file_name();
            if let Some(version) = self.parse_version(&file_name) {
                if version > latest_version {
                    latest_version = version;
                    latest_file = Some(entry.path());
                }
            }
        }
        
        if let Some(file_path) = latest_file {
            let data = tokio::fs::read(file_path).await?;
            
            // Decompress if needed
            let decompressed = if let Some(compression) = &self.compression {
                self.decompress(data, compression)?
            } else {
                data
            };
            
            // Deserialize
            let state = match self.format {
                SerializationFormat::Json => serde_json::from_slice(&decompressed)?,
                SerializationFormat::Bincode => bincode::deserialize(&decompressed)?,
                SerializationFormat::MessagePack => rmp_serde::from_slice(&decompressed)?,
            };
            
            Ok(Some((state, latest_version)))
        } else {
            Ok(None)
        }
    }
}
```

## Configuration

```yaml
state_management:
  # Storage configuration
  storage:
    type: persistent  # memory | persistent | distributed
    path: /var/lib/state
    format: bincode  # json | bincode | msgpack
    compression: zstd # none | gzip | zstd | lz4
    
  # Concurrency configuration
  concurrency:
    model: mvcc  # pessimistic | optimistic | mvcc | lock_free
    max_transactions: 1000
    transaction_timeout: 30s
    
  # History configuration
  history:
    enabled: true
    max_size: 1000
    retention: 24h
    
  # Snapshot configuration
  snapshots:
    enabled: true
    interval: 1h
    max_snapshots: 10
    compression: zstd
    
  # Distributed configuration
  distributed:
    enabled: false
    consensus: raft  # raft | paxos | crdt
    replication_factor: 3
    consistency_level: strong  # strong | bounded | eventual
    
  # Change notification
  notifications:
    buffer_size: 1000
    delivery: at_least_once  # at_most_once | at_least_once | exactly_once
```

## Best Practices

1. **Choose appropriate consistency model** - Strong vs eventual based on requirements
2. **Implement proper validation** - Always validate state transitions
3. **Use versioning** - Track state versions for auditing and rollback
4. **Handle concurrent access** - Use appropriate locking or lock-free structures
5. **Persist critical state** - Don't lose state on crashes
6. **Implement snapshots** - For quick recovery and rollback
7. **Monitor state changes** - Track who changed what and when
8. **Use transactions wisely** - Group related changes
9. **Clean up old state** - Implement garbage collection
10. **Test state transitions** - Ensure all transitions are valid and safe