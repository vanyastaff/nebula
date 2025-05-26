use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use crate::instance::error::InstanceError;
use crate::instance::resolvable::ResolvableInstance;

#[derive(Debug, Default)]
pub struct InstanceRegistry {
    instances: RwLock<HashMap<(String, String), Arc<dyn ResolvableInstance>>>,
}

impl InstanceRegistry {
    /// Создает новый пустой реестр инстансов
    pub fn new() -> Self {
        Self {
            instances: RwLock::new(HashMap::new()),
        }
    }

    /// Регистрирует инстанс в реестре
    ///
    /// # Аргументы
    ///
    /// * `node_id` - ID узла, создавшего инстанс
    /// * `connection_id` - ID соединения, через которое доступен инстанс
    /// * `instance` - Инстанс для регистрации
    ///
    /// # Возвращает
    ///
    /// * `Ok(())` - если регистрация прошла успешно
    /// * `Err(ActionError)` - если возникла ошибка
    pub fn register<T>(
        &self,
        node_id: impl Into<String>,
        connection_id: impl Into<String>,
        instance: T,
    ) -> Result<(), InstanceError>
    where
        T: ResolvableInstance + 'static,
    {
        let node_id = node_id.into();
        let connection_id = connection_id.into();
        let key = (node_id.clone(), connection_id.clone());

        let mut instances = self.instances.write().map_err(|_| {
            InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
        })?;

        // Проверяем, не существует ли уже инстанс с таким ключом
        if instances.contains_key(&key) {
            return Err(InstanceError::InstanceAlreadyExists {
                node_id,
                connection_id
            });
        }

        // Регистрируем инстанс
        instances.insert(key, Arc::new(instance));
        Ok(())
    }

    /// Регистрирует уже обернутый в Arc инстанс
    pub fn register_arc(
        &self,
        node_id: impl Into<String>,
        connection_id: impl Into<String>,
        instance: Arc<dyn ResolvableInstance>,
    ) -> Result<(), InstanceError> {
        let node_id = node_id.into();
        let connection_id = connection_id.into();
        let key = (node_id.clone(), connection_id.clone());

        let mut instances = self.instances.write().map_err(|_| {
            InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
        })?;

        // Проверяем, не существует ли уже инстанс с таким ключом
        if instances.contains_key(&key) {
            return Err(InstanceError::InstanceAlreadyExists {
                node_id,
                connection_id
            });
        }

        // Регистрируем инстанс
        instances.insert(key, instance);
        Ok(())
    }

    /// Получает инстанс из реестра
    ///
    /// # Аргументы
    ///
    /// * `node_id` - ID узла, создавшего инстанс
    /// * `connection_id` - ID соединения, через которое доступен инстанс
    ///
    /// # Возвращает
    ///
    /// * `Ok(Arc<dyn ResolvableInstance>)` - если инстанс найден
    /// * `Err(InstanceError)` - если инстанс не найден или возникла ошибка
    pub fn get(
        &self,
        node_id: impl AsRef<str>,
        connection_id: impl AsRef<str>,
    ) -> Result<Arc<dyn ResolvableInstance>, InstanceError> {
        let key = (node_id.as_ref().to_string(), connection_id.as_ref().to_string());

        let instances = self.instances.read().map_err(|_| {
            InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
        })?;

        instances.get(&key).cloned().ok_or_else(|| {
            InstanceError::InstanceNotFound {
                node_id: node_id.as_ref().to_string(),
                connection_id: connection_id.as_ref().to_string()
            }
        })
    }

    /// Удаляет инстанс из реестра
    ///
    /// # Аргументы
    ///
    /// * `node_id` - ID узла, создавшего инстанс
    /// * `connection_id` - ID соединения, через которое доступен инстанс
    ///
    /// # Возвращает
    ///
    /// * `Ok(Arc<dyn ResolvableInstance>)` - удаленный инстанс
    /// * `Err(InstanceError)` - если инстанс не найден или возникла ошибка
    pub fn remove(
        &self,
        node_id: impl AsRef<str>,
        connection_id: impl AsRef<str>,
    ) -> Result<Arc<dyn ResolvableInstance>, InstanceError> {
        let key = (node_id.as_ref().to_string(), connection_id.as_ref().to_string());

        let mut instances = self.instances.write().map_err(|_| {
            InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
        })?;

        instances.remove(&key).ok_or_else(|| {
            InstanceError::InstanceNotFound {
                node_id: node_id.as_ref().to_string(),
                connection_id: connection_id.as_ref().to_string()
            }
        })
    }

    /// Очищает реестр, закрывая все инстансы
    pub async fn clear(&self) -> Result<(), InstanceError> {
        let instances = {
            let mut instances_guard = self.instances.write().map_err(|_| {
                InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
            })?;

            std::mem::take(&mut *instances_guard)
        };

        // Закрываем все инстансы
        for (_, instance) in instances {
            if let Err(e) = instance.close().await {
                eprintln!("Error closing instance: {}", e);
                // Продолжаем закрывать остальные инстансы
            }
        }

        Ok(())
    }

    /// Проверяет, существует ли инстанс в реестре
    pub fn contains(
        &self,
        node_id: impl AsRef<str>,
        connection_id: impl AsRef<str>,
    ) -> Result<bool, InstanceError> {
        let key = (node_id.as_ref().to_string(), connection_id.as_ref().to_string());

        let instances = self.instances.read().map_err(|_| {
            InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
        })?;

        Ok(instances.contains_key(&key))
    }

    /// Возвращает количество инстансов в реестре
    pub fn len(&self) -> Result<usize, InstanceError> {
        let instances = self.instances.read().map_err(|_| {
            InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
        })?;

        Ok(instances.len())
    }

    /// Проверяет, пуст ли реестр
    pub fn is_empty(&self) -> Result<bool, InstanceError> {
        let instances = self.instances.read().map_err(|_| {
            InstanceError::InternalError("Failed to acquire lock on instance registry".to_string())
        })?;

        Ok(instances.is_empty())
    }
}