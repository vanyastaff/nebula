---

# nebula-node-registry

## Purpose

`nebula-node-registry` manages the discovery, loading, versioning, and lifecycle of workflow nodes, including support for dynamically loaded plugins and git-based distribution.

## Responsibilities

- Node discovery and registration
- Plugin loading and management
- Version management
- Git-based node distribution
- Node caching and optimization
- Dependency resolution

## Architecture

### Core Components

```rust
pub struct NodeRegistry {
    // Loaded nodes
    nodes: Arc<RwLock<HashMap<String, RegisteredNode>>>,
    
    // Plugin manager
    plugin_manager: Arc<PluginManager>,
    
    // Git integrator
    git_integrator: Arc<GitIntegrator>,
    
    // Node cache
    cache: Arc<NodeCache>,
    
    // Discovery service
    discovery: Arc<DiscoveryService>,
    
    // Dependency resolver
    dependency_resolver: Arc<DependencyResolver>,
    
    // Metrics
    metrics: Arc<RegistryMetrics>,
}

pub struct RegisteredNode {
    pub metadata: NodeMetadata,
    pub factory: Box<dyn NodeFactory>,
    pub source: NodeSource,
    pub loaded_at: DateTime<Utc>,
    pub usage_count: AtomicU64,
}

pub enum NodeSource {
    BuiltIn,
    Plugin { path: PathBuf, manifest: PluginManifest },
    Git { url: String, commit: String },
    Registry { name: String, version: Version },
}
```

### Node Discovery

```rust
pub struct DiscoveryService {
    // Discovery strategies
    strategies: Vec<Box<dyn DiscoveryStrategy>>,
    
    // Discovery cache
    cache: Arc<DiscoveryCache>,
}

#[async_trait]
pub trait DiscoveryStrategy: Send + Sync {
    async fn discover(&self) -> Result<Vec<DiscoveredNode>, Error>;
    fn name(&self) -> &str;
}

pub struct DiscoveredNode {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub location: NodeLocation,
    pub metadata: Option<NodeMetadata>,
}

pub enum NodeLocation {
    Library { path: PathBuf },
    Git { url: String, branch: Option<String> },
    Registry { url: String, package: String },
}

// File system discovery
pub struct FileSystemDiscovery {
    search_paths: Vec<PathBuf>,
    pattern: Regex,
}

#[async_trait]
impl DiscoveryStrategy for FileSystemDiscovery {
    async fn discover(&self) -> Result<Vec<DiscoveredNode>, Error> {
        let mut discovered = Vec::new();
        
        for path in &self.search_paths {
            if !path.exists() {
                continue;
            }
            
            for entry in WalkDir::new(path) {
                let entry = entry?;
                let path = entry.path();
                
                if path.is_file() && self.pattern.is_match(path.to_str().unwrap_or("")) {
                    if let Some(node) = self.analyze_library(path).await? {
                        discovered.push(node);
                    }
                }
            }
        }
        
        Ok(discovered)
    }
}

// Convention-based discovery
pub struct ConventionBasedDiscovery {
    target_dir: PathBuf,
    prefix: String,
}

#[async_trait]
impl DiscoveryStrategy for ConventionBasedDiscovery {
    async fn discover(&self) -> Result<Vec<DiscoveredNode>, Error> {
        let mut discovered = Vec::new();
        let pattern = format!("{}*.{}", self.prefix, LIB_EXTENSION);
        
        for entry in fs::read_dir(&self.target_dir).await? {
            let entry = entry?;
            let path = entry.path();
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
                
            if filename.starts_with(&self.prefix) && filename.ends_with(LIB_EXTENSION) {
                discovered.push(DiscoveredNode {
                    id: extract_node_id(filename),
                    name: extract_node_name(filename),
                    version: Version::parse("0.0.0").unwrap(),
                    location: NodeLocation::Library { path },
                    metadata: None,
                });
            }
        }
        
        Ok(discovered)
    }
}
```

### Plugin Management

```rust
pub struct PluginManager {
    // Loaded plugins
    plugins: Arc<RwLock<HashMap<PluginId, LoadedPlugin>>>,
    
    // Plugin loader
    loader: Arc<PluginLoader>,
    
    // Sandbox for plugins
    sandbox: Arc<PluginSandbox>,
}

pub struct LoadedPlugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub library: Library,
    pub nodes: Vec<String>,
    pub resources: PluginResources,
}

#[derive(Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub license: String,
    pub compatibility: CompatibilityInfo,
    pub nodes: Vec<NodeManifest>,
    pub dependencies: Vec<Dependency>,
}

pub struct PluginLoader {
    validator: Arc<PluginValidator>,
    abi_checker: Arc<AbiChecker>,
}

impl PluginLoader {
    pub async fn load_plugin(&self, path: &Path) -> Result<LoadedPlugin, Error> {
        // Read manifest
        let manifest_path = path.join("plugin.toml");
        let manifest: PluginManifest = toml::from_str(
            &fs::read_to_string(manifest_path).await?
        )?;
        
        // Validate plugin
        self.validator.validate(&manifest, path).await?;
        
        // Check ABI compatibility
        let lib_path = path.join(&format!("lib{}.so", manifest.name));
        self.abi_checker.check_compatibility(&lib_path).await?;
        
        // Load library
        let library = unsafe { Library::new(&lib_path)? };
        
        // Get plugin interface
        let plugin_interface: Symbol<fn() -> PluginInterface> =
            unsafe { library.get(b"plugin_interface")? };
            
        let interface = plugin_interface();
        
        // Verify version
        if interface.abi_version != CURRENT_ABI_VERSION {
            return Err(Error::IncompatibleAbiVersion {
                expected: CURRENT_ABI_VERSION,
                found: interface.abi_version,
            });
        }
        
        // Initialize plugin
        let mut context = PluginContext::new();
        if (interface.init)(&mut context) != 0 {
            return Err(Error::PluginInitializationFailed);
        }
        
        // Register nodes
        let mut registry = NodeRegistryHandle::new();
        (interface.register_nodes)(&mut registry);
        
        Ok(LoadedPlugin {
            id: PluginId::from(&manifest.name),
            manifest,
            library,
            nodes: registry.registered_nodes(),
            resources: PluginResources::default(),
        })
    }
}
```

### Git Integration

```rust
pub struct GitIntegrator {
    work_dir: PathBuf,
    builder: Arc<NodeBuilder>,
    cache: Arc<GitCache>,
}

pub struct GitNodeSource {
    pub url: String,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub path: Option<String>,
    pub build_command: Option<String>,
}

impl GitIntegrator {
    pub async fn install_from_git(
        &self,
        source: GitNodeSource,
    ) -> Result<InstalledNode, Error> {
        // Check cache first
        let cache_key = self.calculate_cache_key(&source);
        if let Some(cached) = self.cache.get(&cache_key).await? {
            return Ok(cached);
        }
        
        // Create work directory
        let work_path = self.work_dir.join(&cache_key);
        fs::create_dir_all(&work_path).await?;
        
        // Clone repository
        let repo = self.clone_or_update(&source, &work_path).await?;
        
        // Checkout specific commit/branch
        if let Some(commit) = &source.commit {
            repo.checkout_commit(commit)?;
        } else if let Some(branch) = &source.branch {
            repo.checkout_branch(branch)?;
        }
        
        // Navigate to path if specified
        let build_path = if let Some(path) = &source.path {
            work_path.join(path)
        } else {
            work_path
        };
        
        // Build node
        let build_output = self.builder
            .build(&build_path, source.build_command.as_deref())
            .await?;
            
        // Find built libraries
        let libraries = self.find_built_libraries(&build_output.target_dir).await?;
        
        // Create installed node
        let installed = InstalledNode {
            id: NodeId::new(),
            source: source.clone(),
            libraries,
            built_at: Utc::now(),
        };
        
        // Cache result
        self.cache.put(&cache_key, &installed).await?;
        
        Ok(installed)
    }
    
    async fn clone_or_update(
        &self,
        source: &GitNodeSource,
        path: &Path,
    ) -> Result<Repository, Error> {
        if path.join(".git").exists() {
            // Update existing repository
            let repo = Repository::open(path)?;
            
            let mut remote = repo.find_remote("origin")?;
            remote.fetch(&[], None, None)?;
            
            Ok(repo)
        } else {
            // Clone new repository
            Ok(Repository::clone(&source.url, path)?)
        }
    }
}
```

### Node Caching

```rust
pub struct NodeCache {
    // Memory cache for hot nodes
    memory_cache: Arc<MemoryCache<String, CachedNode>>,
    
    // Disk cache for compiled nodes
    disk_cache: Arc<DiskCache>,
    
    // Cache statistics
    stats: Arc<CacheStats>,
}

pub struct CachedNode {
    pub factory: Arc<dyn NodeFactory>,
    pub metadata: NodeMetadata,
    pub size: usize,
    pub last_used: Instant,
    pub use_count: u64,
}

impl NodeCache {
    pub async fn get_or_load<F>(
        &self,
        node_id: &str,
        loader: F,
    ) -> Result<Arc<dyn NodeFactory>, Error>
    where
        F: FnOnce() -> Future<Output = Result<Box<dyn NodeFactory>, Error>>,
    {
        // Check memory cache
        if let Some(cached) = self.memory_cache.get(node_id).await {
            self.stats.record_hit(CacheLevel::Memory);
            cached.use_count.fetch_add(1, Ordering::Relaxed);
            return Ok(cached.factory.clone());
        }
        
        // Check disk cache
        if let Some(path) = self.disk_cache.get_path(node_id).await? {
            self.stats.record_hit(CacheLevel::Disk);
            
            let factory = self.load_from_disk(&path).await?;
            self.promote_to_memory(node_id, factory.clone()).await?;
            
            return Ok(factory);
        }
        
        // Load and cache
        self.stats.record_miss();
        
        let factory = Arc::from(loader().await?);
        self.cache_node(node_id, factory.clone()).await?;
        
        Ok(factory)
    }
    
    pub async fn evict_cold_nodes(&self) -> Result<EvictionStats, Error> {
        let mut stats = EvictionStats::default();
        
        // Find cold nodes
        let cold_nodes = self.memory_cache
            .entries()
            .filter(|(_, node)| {
                node.last_used.elapsed() > Duration::from_hours(1) &&
                node.use_count < 10
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
            
        // Evict from memory
        for node_id in cold_nodes {
            if let Some(evicted) = self.memory_cache.remove(&node_id).await {
                stats.evicted_count += 1;
                stats.freed_memory += evicted.size;
                
                // Keep in disk cache
                self.disk_cache.ensure_cached(&node_id).await?;
            }
        }
        
        Ok(stats)
    }
}
```

### Dependency Resolution

```rust
pub struct DependencyResolver {
    registry: Arc<NodeRegistry>,
    version_resolver: Arc<VersionResolver>,
}

pub struct Dependency {
    pub name: String,
    pub version_req: VersionReq,
    pub optional: bool,
    pub features: Vec<String>,
}

impl DependencyResolver {
    pub async fn resolve_dependencies(
        &self,
        node: &NodeManifest,
    ) -> Result<ResolvedDependencies, Error> {
        let mut resolver = DependencyGraph::new();
        
        // Add root node
        resolver.add_node(&node.name, &node.version);
        
        // Resolve recursively
        self.resolve_recursive(&mut resolver, &node.name, &node.dependencies).await?;
        
        // Check for conflicts
        if let Some(conflict) = resolver.find_conflict() {
            return Err(Error::DependencyConflict(conflict));
        }
        
        // Create resolution
        Ok(resolver.create_resolution())
    }
    
    async fn resolve_recursive(
        &self,
        graph: &mut DependencyGraph,
        parent: &str,
        dependencies: &[Dependency],
    ) -> Result<(), Error> {
        for dep in dependencies {
            // Find matching versions
            let versions = self.registry
                .find_node_versions(&dep.name)
                .await?;
                
            let matching = versions
                .into_iter()
                .filter(|v| dep.version_req.matches(v))
                .collect::<Vec<_>>();
                
            if matching.is_empty() && !dep.optional {
                return Err(Error::DependencyNotFound {
                    name: dep.name.clone(),
                    requirement: dep.version_req.clone(),
                });
            }
            
            if let Some(version) = self.version_resolver.select_best(&matching) {
                graph.add_edge(parent, &dep.name, version);
                
                // Load dependency manifest
                let dep_manifest = self.registry
                    .get_node_manifest(&dep.name, version)
                    .await?;
                    
                // Recurse
                self.resolve_recursive(
                    graph,
                    &dep.name,
                    &dep_manifest.dependencies
                ).await?;
            }
        }
        
        Ok(())
    }
}
```

### Registry API

```rust
impl NodeRegistry {
    pub async fn register_node(
        &self,
        factory: Box<dyn NodeFactory>,
        source: NodeSource,
    ) -> Result<(), Error> {
        let metadata = factory.metadata();
        let node_id = metadata.id.clone();
        
        info!("Registering node: {} v{}", metadata.name, metadata.version);
        
        // Check for conflicts
        if let Some(existing) = self.nodes.read().await.get(&node_id) {
            if existing.metadata.version >= metadata.version {
                return Err(Error::NodeAlreadyRegistered {
                    id: node_id,
                    version: existing.metadata.version.clone(),
                });
            }
        }
        
        // Create registered node
        let registered = RegisteredNode {
            metadata: metadata.clone(),
            factory,
            source,
            loaded_at: Utc::now(),
            usage_count: AtomicU64::new(0),
        };
        
        // Register
        self.nodes.write().await.insert(node_id.clone(), registered);
        
        // Update metrics
        self.metrics.nodes_registered.increment();
        
        // Emit event
        self.emit_node_registered_event(&metadata).await?;
        
        Ok(())
    }
    
    pub async fn get_node(&self, node_id: &str) -> Result<Arc<dyn Action>, Error> {
        // Get from registry
        let registered = self.nodes
            .read()
            .await
            .get(node_id)
            .cloned()
            .ok_or(Error::NodeNotFound)?;
            
        // Update usage
        registered.usage_count.fetch_add(1, Ordering::Relaxed);
        
        // Create instance
        let instance = registered.factory.create().await?;
        
        Ok(instance)
    }
    
    pub async fn list_nodes(&self, filter: NodeFilter) -> Vec<NodeMetadata> {
        self.nodes
            .read()
            .await
            .values()
            .filter(|node| filter.matches(&node.metadata))
            .map(|node| node.metadata.clone())
            .collect()
    }
}
```

---

