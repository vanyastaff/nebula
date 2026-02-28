# Archived From "docs/archive/layers-interaction.md"

### 6. nebula-config → Все крейты

**Паттерн:** Config инжектируется во все крейты через DI

```rust
// nebula-config определяет конфигурации
#[derive(Config)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub timeout: Duration,
}

#[derive(Config)]
pub struct CacheConfig {
    pub backend: CacheBackend,
    pub ttl: Duration,
    pub max_size: usize,
}

// Каждый крейт получает свою конфигурацию
impl DatabaseResource {
    pub fn new(config: DatabaseConfig) -> Self {
        Self { config }
    }
    
    async fn create_instance(&self) -> DatabaseInstance {
        let pool = PgPool::builder()
            .max_connections(self.config.max_connections)
            .connect_timeout(self.config.timeout)
            .build(&self.config.url)
            .await?;
            
        DatabaseInstance { pool }
    }
}

impl CacheResource {
    pub fn new(config: CacheConfig) -> Self {
        let backend = match config.backend {
            CacheBackend::Redis => RedisCache::new(),
            CacheBackend::Memory => MemoryCache::new(config.max_size),
        };
        
        Self { backend, config }
    }
}

// Центральная инициализация
pub struct Application {
    config_manager: ConfigManager,
    resource_manager: ResourceManager,
}

impl Application {
    pub async fn initialize() -> Self {
        // Загружаем конфигурацию
        let config_manager = ConfigManager::from_file("config.toml").await?;
        
        // Создаем resource manager с конфигурацией
        let resource_manager = ResourceManager::new();
        
        // Регистрируем ресурсы с их конфигурациями
        let db_config: DatabaseConfig = config_manager.get()?;
        resource_manager.register(DatabaseResource::new(db_config));
        
        let cache_config: CacheConfig = config_manager.get()?;
        resource_manager.register(CacheResource::new(cache_config));
        
        Self { config_manager, resource_manager }
    }
}
```

