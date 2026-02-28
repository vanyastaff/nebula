---

# nebula-binary

## Purpose

`nebula-binary` handles the storage and management of binary data (files, images, documents) with automatic tiering, garbage collection, and efficient streaming.

## Responsibilities

- Binary data storage strategies
- Automatic storage tiering
- Streaming upload/download
- Garbage collection
- Content deduplication
- Compression support

## Architecture

### Core Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BinaryDataLocation {
    /// Small files kept in memory (< 1MB)
    InMemory {
        id: Uuid,
        data: Vec<u8>,
        metadata: BinaryMetadata,
    },
    
    /// Medium files in temporary storage (1-100MB)
    Temp {
        id: Uuid,
        path: PathBuf,
        expires_at: DateTime<Utc>,
        metadata: BinaryMetadata,
    },
    
    /// Large files in object storage (> 100MB)
    Remote {
        id: Uuid,
        storage_type: RemoteStorageType,
        key: String,
        metadata: BinaryMetadata,
    },
    
    /// Generated on-demand (AI images, reports, etc)
    Generated {
        id: Uuid,
        generator: GeneratorType,
        params: serde_json::Value,
        cache_key: Option<String>,
        metadata: BinaryMetadata,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryMetadata {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size: usize,
    pub checksum: String,
    pub created_at: DateTime<Utc>,
    pub accessed_at: DateTime<Utc>,
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteStorageType {
    S3,
    AzureBlob,
    GoogleCloudStorage,
    MinIO,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneratorType {
    AiImage { model: String },
    PdfReport { template: String },
    DataExport { format: ExportFormat },
    Archive { compression: CompressionType },
}
```

### Smart Storage Manager

```rust
pub struct SmartBinaryStorage {
    // Storage tiers
    memory_storage: Arc<MemoryStorage>,
    temp_storage: Arc<TempStorage>,
    remote_storage: Arc<dyn RemoteStorage>,
    
    // Configuration
    config: StorageConfig,
    
    // Metrics
    metrics: Arc<StorageMetrics>,
    
    // Background tasks
    gc_handle: JoinHandle<()>,
    tiering_handle: JoinHandle<()>,
}

pub struct StorageConfig {
    pub memory_threshold: usize,      // Default: 1MB
    pub temp_threshold: usize,        // Default: 100MB
    pub temp_ttl: Duration,          // Default: 24 hours
    pub compression_threshold: usize, // Default: 10MB
    pub deduplication: bool,         // Default: true
    pub gc_interval: Duration,       // Default: 1 hour
}

impl SmartBinaryStorage {
    pub async fn store(
        &self,
        data: BinaryData,
        hints: StorageHints,
    ) -> Result<BinaryHandle, Error> {
        let size = data.size();
        let metadata = self.create_metadata(&data)?;
        
        // Check deduplication
        if self.config.deduplication {
            if let Some(existing) = self.find_duplicate(&metadata.checksum).await? {
                self.metrics.record_dedup_hit();
                return Ok(existing);
            }
        }
        
        // Determine storage location
        let location = match (size, hints) {
            (size, _) if size < self.config.memory_threshold => {
                self.store_in_memory(data, metadata).await?
            }
            
            (size, StorageHints { temp: true, .. }) |
            (size, _) if size < self.config.temp_threshold => {
                self.store_in_temp(data, metadata).await?
            }
            
            _ => {
                self.store_in_remote(data, metadata).await?
            }
        };
        
        Ok(BinaryHandle {
            id: location.id(),
            location,
        })
    }
    
    async fn store_in_memory(
        &self,
        data: BinaryData,
        metadata: BinaryMetadata,
    ) -> Result<BinaryDataLocation, Error> {
        let id = Uuid::new_v4();
        let bytes = data.into_bytes().await?;
        
        self.memory_storage.store(id, bytes.clone()).await?;
        
        Ok(BinaryDataLocation::InMemory {
            id,
            data: bytes,
            metadata,
        })
    }
    
    async fn store_in_temp(
        &self,
        data: BinaryData,
        metadata: BinaryMetadata,
    ) -> Result<BinaryDataLocation, Error> {
        let id = Uuid::new_v4();
        let path = self.temp_storage.create_file(id).await?;
        
        // Stream to file
        let mut file = File::create(&path).await?;
        let mut stream = data.into_stream();
        
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk?).await?;
        }
        
        file.sync_all().await?;
        
        let expires_at = Utc::now() + self.config.temp_ttl;
        
        Ok(BinaryDataLocation::Temp {
            id,
            path,
            expires_at,
            metadata,
        })
    }
    
    async fn store_in_remote(
        &self,
        data: BinaryData,
        metadata: BinaryMetadata,
    ) -> Result<BinaryDataLocation, Error> {
        let id = Uuid::new_v4();
        let key = self.generate_storage_key(&id, &metadata);
        
        // Apply compression if needed
        let data = if metadata.size > self.config.compression_threshold {
            self.compress_data(data).await?
        } else {
            data
        };
        
        self.remote_storage.upload(&key, data).await?;
        
        Ok(BinaryDataLocation::Remote {
            id,
            storage_type: self.remote_storage.storage_type(),
            key,
            metadata,
        })
    }
}
```

### Storage Tiering

```rust
pub struct StorageTiering {
    analyzer: UsageAnalyzer,
    migrator: DataMigrator,
}

impl StorageTiering {
    pub async fn run_tiering_cycle(&self) -> Result<TieringStats, Error> {
        let mut stats = TieringStats::default();
        
        // Analyze usage patterns
        let analysis = self.analyzer.analyze().await?;
        
        // Promote frequently accessed temp files to memory
        for file in analysis.hot_temp_files {
            if file.access_count > 10 && file.size < 1_000_000 {
                self.migrator.promote_to_memory(&file).await?;
                stats.promoted_to_memory += 1;
            }
        }
        
        // Demote cold memory items to temp
        for item in analysis.cold_memory_items {
            if item.last_access.elapsed() > Duration::from_hours(1) {
                self.migrator.demote_to_temp(&item).await?;
                stats.demoted_to_temp += 1;
            }
        }
        
        // Archive old temp files to remote
        for file in analysis.old_temp_files {
            if file.age > Duration::from_days(1) {
                self.migrator.archive_to_remote(&file).await?;
                stats.archived_to_remote += 1;
            }
        }
        
        Ok(stats)
    }
}
```

### Streaming Support

```rust
pub struct StreamingHandler {
    chunk_size: usize,
    buffer_pool: BufferPool,
}

impl StreamingHandler {
    pub async fn upload_stream<S>(
        &self,
        stream: S,
        hints: UploadHints,
    ) -> Result<BinaryHandle, Error>
    where
        S: Stream<Item = Result<Bytes, Error>> + Send,
    {
        let id = Uuid::new_v4();
        let hasher = Sha256::new();
        let mut size = 0;
        
        // Determine storage based on hints
        let storage = match hints.expected_size {
            Some(s) if s < 1_000_000 => StreamStorage::Memory,
            Some(s) if s < 100_000_000 => StreamStorage::Temp,
            _ => StreamStorage::Remote,
        };
        
        match storage {
            StreamStorage::Memory => {
                let mut buffer = Vec::new();
                pin_mut!(stream);
                
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    hasher.update(&chunk);
                    buffer.extend_from_slice(&chunk);
                    size += chunk.len();
                    
                    if size > 1_000_000 {
                        // Switch to temp storage
                        return self.upgrade_to_temp_storage(buffer, stream).await;
                    }
                }
                
                // Store in memory
                Ok(self.finalize_memory_storage(id, buffer, hasher.finalize()))
            }
            
            StreamStorage::Temp => {
                self.stream_to_temp(id, stream, hasher).await
            }
            
            StreamStorage::Remote => {
                self.stream_to_remote(id, stream, hasher).await
            }
        }
    }
    
    pub fn download_stream(
        &self,
        handle: &BinaryHandle,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>, Error> {
        match &handle.location {
            BinaryDataLocation::InMemory { data, .. } => {
                // Convert to stream
                let chunks = data.chunks(self.chunk_size)
                    .map(|chunk| Ok(Bytes::from(chunk.to_vec())))
                    .collect::<Vec<_>>();
                    
                Ok(Box::pin(futures::stream::iter(chunks)))
            }
            
            BinaryDataLocation::Temp { path, .. } => {
                // Stream from file
                Ok(Box::pin(self.stream_from_file(path.clone())))
            }
            
            BinaryDataLocation::Remote { key, .. } => {
                // Stream from remote storage
                Ok(Box::pin(self.remote_storage.download_stream(key)))
            }
            
            BinaryDataLocation::Generated { generator, params, .. } => {
                // Generate on-the-fly
                Ok(Box::pin(self.generate_stream(generator, params)))
            }
        }
    }
}
```

### Garbage Collection

```rust
pub struct GarbageCollector {
    storage: Arc<SmartBinaryStorage>,
    rules: Vec<Box<dyn GcRule>>,
}

#[async_trait]
pub trait GcRule: Send + Sync {
    async fn should_delete(&self, item: &GcItem) -> bool;
    fn name(&self) -> &str;
}

pub struct TtlRule {
    ttl: Duration,
}

#[async_trait]
impl GcRule for TtlRule {
    async fn should_delete(&self, item: &GcItem) -> bool {
        item.last_access.elapsed() > self.ttl
    }
    
    fn name(&self) -> &str {
        "TTL Rule"
    }
}

impl GarbageCollector {
    pub async fn run_gc_cycle(&self) -> Result<GcStats, Error> {
        let mut stats = GcStats::default();
        
        // Scan all storage locations
        let items = self.scan_all_items().await?;
        
        for item in items {
            let mut should_delete = false;
            let mut matched_rule = None;
            
            // Check all rules
            for rule in &self.rules {
                if rule.should_delete(&item).await {
                    should_delete = true;
                    matched_rule = Some(rule.name());
                    break;
                }
            }
            
            if should_delete {
                self.delete_item(&item).await?;
                stats.deleted_count += 1;
                stats.freed_bytes += item.size;
                
                info!(
                    "GC: Deleted {} ({} bytes) - Rule: {}",
                    item.id,
                    item.size,
                    matched_rule.unwrap_or("unknown")
                );
            }
        }
        
        Ok(stats)
    }
}
```

### Deduplication

```rust
pub struct Deduplicator {
    index: Arc<RwLock<HashMap<String, BinaryHandle>>>,
    hasher: Box<dyn Hasher>,
}

impl Deduplicator {
    pub async fn find_duplicate(&self, data: &[u8]) -> Option<BinaryHandle> {
        let hash = self.hasher.hash(data);
        self.index.read().await.get(&hash).cloned()
    }
    
    pub async fn register(&self, hash: String, handle: BinaryHandle) {
        self.index.write().await.insert(hash, handle);
    }
    
    pub async fn dedup_stats(&self) -> DedupStats {
        let index = self.index.read().await;
        
        DedupStats {
            unique_files: index.len(),
            total_references: self.count_references(&index).await,
            space_saved: self.calculate_space_saved(&index).await,
        }
    }
}
```

### Compression

```rust
pub struct CompressionManager {
    strategies: HashMap<String, Box<dyn CompressionStrategy>>,
}

#[async_trait]
pub trait CompressionStrategy: Send + Sync {
    async fn compress(&self, data: Bytes) -> Result<Bytes, Error>;
    async fn decompress(&self, data: Bytes) -> Result<Bytes, Error>;
    fn content_types(&self) -> Vec<String>;
}

pub struct ZstdCompression {
    level: i32,
}

#[async_trait]
impl CompressionStrategy for ZstdCompression {
    async fn compress(&self, data: Bytes) -> Result<Bytes, Error> {
        tokio::task::spawn_blocking(move || {
            let compressed = zstd::compress(&data, self.level)?;
            Ok(Bytes::from(compressed))
        })
        .await?
    }
    
    async fn decompress(&self, data: Bytes) -> Result<Bytes, Error> {
        tokio::task::spawn_blocking(move || {
            let decompressed = zstd::decompress(&data)?;
            Ok(Bytes::from(decompressed))
        })
        .await?
    }
    
    fn content_types(&self) -> Vec<String> {
        vec![
            "text/plain".to_string(),
            "application/json".to_string(),
            "application/xml".to_string(),
        ]
    }
}
```

---

