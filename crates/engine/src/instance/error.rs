use thiserror::Error;

#[derive(Debug, Error)]
pub enum InstanceError {
    #[error("Internal error: {0}")]
    InternalError(String),

    /// Ошибка ввода-вывода
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Инстанс с указанными node_id и connection_id не найден
    #[error("Instance not found for node '{node_id}' and connection '{connection_id}'")]
    InstanceNotFound { node_id: String, connection_id: String },

    /// Инстанс с указанными node_id и connection_id уже существует
    #[error("Instance already exists for node '{node_id}' and connection '{connection_id}'")]
    InstanceAlreadyExists { node_id: String, connection_id: String },

    /// Ошибка ленивой инициализации
    #[error("Failed to initialize lazy instance of type '{0}': {1}")]
    LazyInitializationFailed(String, String),

    /// Ленивая инициализация уже выполняется
    #[error("Lazy initialization of instance '{0}' is already in progress")]
    LazyInitializationInProgress(String),

    /// Инициализатор ленивого инстанса уже использован
    #[error("Lazy initializer for instance '{0}' has already been used")]
    LazyInitializerAlreadyUsed(String),
}
