use std::fmt::{Debug, Formatter};
use std::sync::{Arc, Mutex, RwLock};
use futures::future::BoxFuture;
use async_trait::async_trait;
use serde_json::{json, Value};
use crate::instance::error::InstanceError;
use crate::instance::resolvable::ResolvableInstance;

#[derive(Debug, Clone, PartialEq)]
enum LazyStatus {
    /// Еще не инициализирован
    NotInitialized,
    /// В процессе инициализации
    Initializing,
    /// Инициализирован успешно
    Initialized,
    /// Произошла ошибка при инициализации
    Failed(String),
}

/// Обертка для ленивой инициализации инстанса
pub struct LazyInstance<T: ResolvableInstance + 'static> {
    /// Инициализированное значение или None, если еще не инициализировано
    instance: RwLock<Option<Arc<T>>>,

    /// Функция инициализации, которая будет вызвана при первом обращении
    initializer: Mutex<Option<Box<dyn (FnOnce() -> BoxFuture<'static, Result<T, InstanceError>>) + Send + Sync>>>,

    /// Статус инициализации
    status: RwLock<LazyStatus>,

    /// Тип инстанса (для отладки)
    type_name: &'static str,
}

impl<T: ResolvableInstance + 'static> LazyInstance<T> {
    /// Создает новый ленивый инстанс с функцией инициализации
    pub fn new<F, Fut>(initializer: F, type_name: &'static str) -> Self
    where
        F: FnOnce() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, InstanceError>> + Send + 'static,
    {
        Self {
            instance: RwLock::new(None),
            initializer: Mutex::new(Some(Box::new(move || Box::pin(initializer())))),
            status: RwLock::new(LazyStatus::NotInitialized),
            type_name,
        }
    }

    /// Получает или инициализирует инстанс
    pub async fn get(&self) -> Result<Arc<T>, InstanceError> {
        // Сначала проверяем, есть ли уже готовый инстанс
        {
            let instance_guard = self.instance.read().unwrap();
            if let Some(instance) = &*instance_guard {
                return Ok(Arc::clone(instance));
            }

            // Проверяем статус, чтобы не пытаться инициализировать дважды при ошибке
            let status = self.status.read().unwrap();
            match &*status {
                LazyStatus::Initialized => unreachable!("Instance should be set if initialized"),
                LazyStatus::Failed(err) => return Err(InstanceError::LazyInitializationFailed(
                    self.type_name.to_string(),
                    err.clone()
                )),
                _ => {} // Продолжаем инициализацию
            }
        }

        // Блокируем для инициализации
        let mut status = self.status.write().unwrap();

        // Повторная проверка после получения блокировки
        match &*status {
            LazyStatus::Initialized => {
                // Кто-то успел инициализировать пока мы ждали блокировку
                let instance_guard = self.instance.read().unwrap();
                return Ok(Arc::clone(instance_guard.as_ref().unwrap()));
            }
            LazyStatus::Initializing => {
                return Err(InstanceError::LazyInitializationInProgress(
                    self.type_name.to_string()
                ));
            }
            LazyStatus::Failed(err) => {
                return Err(InstanceError::LazyInitializationFailed(
                    self.type_name.to_string(),
                    err.clone()
                ));
            }
            LazyStatus::NotInitialized => {
                // Продолжаем инициализацию
                *status = LazyStatus::Initializing;
            }
        }

        // Извлекаем инициализатор
        let initializer = {
            let mut initializer_guard = self.initializer.lock().unwrap();
            initializer_guard.take().ok_or(InstanceError::LazyInitializerAlreadyUsed(
                self.type_name.to_string()
            ))?
        };

        // Выполняем инициализацию
        match initializer().await {
            Ok(instance) => {
                let instance_arc = Arc::new(instance);
                {
                    let mut instance_guard = self.instance.write().unwrap();
                    *instance_guard = Some(Arc::clone(&instance_arc));
                }
                *status = LazyStatus::Initialized;
                Ok(instance_arc)
            }
            Err(e) => {
                *status = LazyStatus::Failed(e.to_string());
                Err(e)
            }
        }
    }

    /// Проверяет, был ли уже инициализирован инстанс
    pub fn is_initialized(&self) -> bool {
        let status = self.status.read().unwrap();
        matches!(*status, LazyStatus::Initialized)
    }

    /// Проверяет, произошла ли ошибка при инициализации
    pub fn has_failed(&self) -> bool {
        let status = self.status.read().unwrap();
        matches!(*status, LazyStatus::Failed(_))
    }

    /// Получает ошибку инициализации, если она есть
    pub fn get_error(&self) -> Option<String> {
        let status = self.status.read().unwrap();
        match &*status {
            LazyStatus::Failed(err) => Some(err.clone()),
            _ => None
        }
    }
}

impl<T: ResolvableInstance + 'static> Debug for LazyInstance<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let status = self.status.read().unwrap();
        let instance_status = match &*status {
            LazyStatus::NotInitialized => "not initialized",
            LazyStatus::Initializing => "initializing",
            LazyStatus::Initialized => "initialized",
            LazyStatus::Failed(_) => "failed",
        };

        f.debug_struct("LazyInstance")
            .field("type_name", &self.type_name)
            .field("status", &instance_status)
            .finish()
    }
}

#[async_trait]
impl<T: ResolvableInstance + 'static> ResolvableInstance for LazyInstance<T> {
    fn type_name(&self) -> &str {
        self.type_name
    }

    async fn get_property(&self, path: &str) -> Option<Value> {
        match self.get().await {
            Ok(instance) => instance.get_property(path).await,
            Err(_) => None
        }
    }

    async fn as_value(&self) -> Value {
        match self.get().await {
            Ok(instance) => instance.as_value().await,
            Err(e) => json!({
                "type": "lazy_instance",
                "status": "error",
                "error": e.to_string()
            })
        }
    }

    async fn close(&self) -> Result<(), InstanceError> {
        let instance_guard = self.instance.read().unwrap();
        if let Some(instance) = &*instance_guard {
            instance.close().await
        } else {
            Ok(())
        }
    }

    fn clone_instance(&self) -> Arc<dyn ResolvableInstance> {
        // LazyInstance не поддерживает клонирование,
        // так как инициализатор может быть вызван только раз
        Arc::new(DummyInstance::new(self.type_name))
    }
}

/// Placeholder для LazyInstance, когда клонирование невозможно
#[derive(Debug)]
struct DummyInstance {
    type_name: &'static str,
}

impl DummyInstance {
    fn new(type_name: &'static str) -> Self {
        Self { type_name }
    }
}

#[async_trait]
impl ResolvableInstance for DummyInstance {
    fn type_name(&self) -> &str {
        self.type_name
    }

    async fn get_property(&self, _path: &str) -> Option<Value> {
        None
    }

    async fn as_value(&self) -> Value {
        json!({
            "type": "lazy_instance_placeholder",
            "original_type": self.type_name,
            "status": "unavailable",
            "error": "This is a placeholder for a lazy instance that cannot be cloned"
        })
    }

    fn clone_instance(&self) -> Arc<dyn ResolvableInstance> {
        Arc::new(self.clone())
    }
}

impl Clone for DummyInstance {
    fn clone(&self) -> Self {
        Self { type_name: self.type_name }
    }
}