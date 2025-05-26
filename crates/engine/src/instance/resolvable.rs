use std::fmt::Debug;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use crate::instance::error::InstanceError;

#[async_trait]
pub trait ResolvableInstance: Debug + Send + Sync {
    /// Возвращает тип этого инстанса
    fn type_name(&self) -> &str;

    /// Возвращает значение свойства из этого инстанса
    async fn get_property(&self, path: &str) -> Option<Value>;

    /// Возвращает этот инстанс как общее значение Value
    async fn as_value(&self) -> Value;

    /// Закрывает ресурсы, связанные с этим инстансом
    async fn close(&self) -> Result<(), InstanceError> {
        Ok(()) // По умолчанию ничего не делаем
    }

    /// Клонирует этот инстанс
    fn clone_instance(&self) -> Arc<dyn ResolvableInstance>;
}

pub trait InstanceType: ResolvableInstance {}

// Макрос для упрощения реализации InstanceType для конкретных типов
#[macro_export]
macro_rules! impl_instance_type {
    ($type:ty) => {
        impl InstanceType for $type {}
    };
}