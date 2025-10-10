//! Nebula Validator v2.0 - Modern Type-Safe Validation Architecture
//! 
//! Основные принципы:
//! - Type-safe где возможно (compile-time validation)
//! - Zero-cost abstractions
//! - Composable validators
//! - Async support
//! - Extensible through traits

// ============================================================================
// CORE TRAITS - Основа системы
// ============================================================================

/// Базовый trait для всех валидаторов
/// Generic T - тип входных данных
pub trait TypedValidator {
    /// Тип входных данных
    type Input: ?Sized;
    
    /// Тип выходных данных (может быть refined type)
    type Output;
    
    /// Тип ошибки
    type Error: std::error::Error;
    
    /// Синхронная валидация
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error>;
    
    /// Метаданные валидатора
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::default()
    }
}

/// Async версия валидатора
#[async_trait::async_trait]
pub trait AsyncValidator {
    type Input: ?Sized;
    type Output;
    type Error: std::error::Error;
    
    async fn validate_async(&self, input: &Self::Input) -> Result<Self::Output, Self::Error>;
}

/// Метаданные валидатора для introspection
#[derive(Debug, Clone)]
pub struct ValidatorMetadata {
    pub name: String,
    pub description: Option<String>,
    pub complexity: ValidationComplexity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationComplexity {
    Constant,    // O(1) - простая проверка
    Linear,      // O(n) - зависит от размера входа
    Expensive,   // O(n²) или async операции
}

// ============================================================================
// REFINED TYPES - Типы с гарантиями
// ============================================================================

/// Refined type - значение, прошедшее валидацию
/// Гарантирует на уровне типов, что значение валидно
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refined<T, V> {
    value: T,
    _validator: std::marker::PhantomData<V>,
}

impl<T, V> Refined<T, V>
where
    V: TypedValidator<Input = T, Output = T>,
{
    /// Создание refined type с проверкой
    pub fn new(value: T, validator: &V) -> Result<Self, V::Error> {
        validator.validate(&value)?;
        Ok(Self {
            value,
            _validator: std::marker::PhantomData,
        })
    }
    
    /// Небезопасное создание без проверки (для оптимизации)
    /// SAFETY: Вызывающий должен гарантировать валидность
    pub unsafe fn new_unchecked(value: T) -> Self {
        Self {
            value,
            _validator: std::marker::PhantomData,
        }
    }
    
    /// Извлечение значения
    pub fn into_inner(self) -> T {
        self.value
    }
    
    /// Ссылка на значение
    pub fn get(&self) -> &T {
        &self.value
    }
}

// ============================================================================
// VALIDATION STATE - Type-state pattern
// ============================================================================

/// Type-state pattern для builder'ов параметров
pub struct Unvalidated;
pub struct Validated<V>(std::marker::PhantomData<V>);

/// Параметр с type-state
pub struct Parameter<T, S = Unvalidated> {
    value: T,
    _state: std::marker::PhantomData<S>,
}

impl<T> Parameter<T, Unvalidated> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            _state: std::marker::PhantomData,
        }
    }
    
    /// Валидация переводит в состояние Validated
    pub fn validate<V>(self, validator: &V) -> Result<Parameter<T, Validated<V>>, V::Error>
    where
        V: TypedValidator<Input = T, Output = T>,
    {
        validator.validate(&self.value)?;
        Ok(Parameter {
            value: self.value,
            _state: std::marker::PhantomData,
        })
    }
}

impl<T, V> Parameter<T, Validated<V>> {
    /// Безопасное извлечение - гарантировано валидно
    pub fn unwrap(self) -> T {
        self.value
    }
}

// ============================================================================
// COMBINATORS - Композиция валидаторов
// ============================================================================

/// Extension trait для композиции
pub trait ValidatorExt: TypedValidator + Sized {
    /// AND combinator - оба должны пройти
    fn and<V>(self, other: V) -> And<Self, V>
    where
        V: TypedValidator<Input = Self::Input, Output = Self::Output, Error = Self::Error>,
    {
        And::new(self, other)
    }
    
    /// OR combinator - хотя бы один должен пройти
    fn or<V>(self, other: V) -> Or<Self, V>
    where
        V: TypedValidator<Input = Self::Input, Output = Self::Output, Error = Self::Error>,
    {
        Or::new(self, other)
    }
    
    /// NOT combinator - должен НЕ пройти
    fn not(self) -> Not<Self> {
        Not::new(self)
    }
    
    /// Map output type
    fn map<F, O>(self, f: F) -> Map<Self, F>
    where
        F: Fn(Self::Output) -> O,
    {
        Map::new(self, f)
    }
    
    /// Conditional validation
    fn when<C>(self, condition: C) -> When<Self, C>
    where
        C: Fn(&Self::Input) -> bool,
    {
        When::new(self, condition)
    }
    
    /// Кэширование результатов
    fn cached(self) -> Cached<Self>
    where
        Self::Input: std::hash::Hash + Eq,
        Self::Output: Clone,
        Self::Error: Clone,
    {
        Cached::new(self)
    }
}

// Автоматически реализуем для всех валидаторов
impl<T: TypedValidator> ValidatorExt for T {}

/// AND combinator
pub struct And<L, R> {
    left: L,
    right: R,
}

impl<L, R> And<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

impl<L, R> TypedValidator for And<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Output = L::Output, Error = L::Error>,
{
    type Input = L::Input;
    type Output = L::Output;
    type Error = L::Error;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let output = self.left.validate(input)?;
        self.right.validate(input)?;
        Ok(output)
    }
}

/// OR combinator
pub struct Or<L, R> {
    left: L,
    right: R,
}

impl<L, R> Or<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

impl<L, R> TypedValidator for Or<L, R>
where
    L: TypedValidator,
    R: TypedValidator<Input = L::Input, Output = L::Output, Error = L::Error>,
{
    type Input = L::Input;
    type Output = L::Output;
    type Error = L::Error;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        self.left.validate(input)
            .or_else(|_| self.right.validate(input))
    }
}

/// NOT combinator
pub struct Not<V> {
    inner: V,
}

impl<V> Not<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

/// Map combinator
pub struct Map<V, F> {
    validator: V,
    mapper: F,
}

impl<V, F> Map<V, F> {
    pub fn new(validator: V, mapper: F) -> Self {
        Self { validator, mapper }
    }
}

impl<V, F, O> TypedValidator for Map<V, F>
where
    V: TypedValidator,
    F: Fn(V::Output) -> O,
{
    type Input = V::Input;
    type Output = O;
    type Error = V::Error;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let output = self.validator.validate(input)?;
        Ok((self.mapper)(output))
    }
}

/// Conditional validator
pub struct When<V, C> {
    validator: V,
    condition: C,
}

impl<V, C> When<V, C> {
    pub fn new(validator: V, condition: C) -> Self {
        Self { validator, condition }
    }
}

impl<V, C> TypedValidator for When<V, C>
where
    V: TypedValidator,
    C: Fn(&V::Input) -> bool,
{
    type Input = V::Input;
    type Output = V::Output;
    type Error = V::Error;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if (self.condition)(input) {
            self.validator.validate(input)
        } else {
            // NOTE: This is a design limitation of the TypedValidator trait.
            // When the condition is false, we skip validation, but we have no way
            // to produce an Output value since we never called the inner validator.
            //
            // Possible solutions:
            // 1. Change trait to allow Option<Output>
            // 2. Require Output: Default
            // 3. Use a different approach like Optional<V> wrapper that wraps the result
            // 4. Remove When combinator from this architecture
            //
            // For now, this is a conceptual example showing the limitation.
            panic!(
                "When combinator with false condition has no way to produce Output. \
                 This is a design limitation documented in validator_arch.rs"
            )
        }
    }
}

/// Cached validator
pub struct Cached<V> {
    validator: V,
    cache: std::sync::RwLock<std::collections::HashMap<u64, CacheEntry<V>>>,
}

struct CacheEntry<V: TypedValidator> {
    result: Result<V::Output, V::Error>,
}

impl<V> Cached<V>
where
    V: TypedValidator,
    V::Input: std::hash::Hash,
    V::Output: Clone,
    V::Error: Clone,
{
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            cache: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

// ============================================================================
// CONCRETE VALIDATORS - Примеры реализаций
// ============================================================================

/// String length validator
pub struct MinLength {
    pub min: usize,
}

impl TypedValidator for MinLength {
    type Input = str;
    type Output = ();
    type Error = ValidationError;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.len() >= self.min {
            Ok(())
        } else {
            Err(ValidationError::new(
                "min_length",
                format!("String must be at least {} characters", self.min),
            ))
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "MinLength".to_string(),
            description: Some(format!("Minimum length: {}", self.min)),
            complexity: ValidationComplexity::Constant,
        }
    }
}

/// Range validator для чисел
pub struct InRange<T> {
    pub min: T,
    pub max: T,
}

impl<T> TypedValidator for InRange<T>
where
    T: PartialOrd + std::fmt::Display + Copy,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if *input >= self.min && *input <= self.max {
            Ok(())
        } else {
            Err(ValidationError::new(
                "in_range",
                format!("Value must be between {} and {}", self.min, self.max),
            ))
        }
    }
}

/// Regex validator
pub struct MatchesRegex {
    pub pattern: regex::Regex,
}

impl TypedValidator for MatchesRegex {
    type Input = str;
    type Output = ();
    type Error = ValidationError;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if self.pattern.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::new(
                "regex",
                format!("String must match pattern: {}", self.pattern.as_str()),
            ))
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "MatchesRegex".to_string(),
            description: Some(format!("Pattern: {}", self.pattern.as_str())),
            complexity: ValidationComplexity::Linear,
        }
    }
}

// ============================================================================
// ERROR HANDLING - Улучшенная система ошибок
// ============================================================================

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
    pub field: Option<String>,
    pub params: std::collections::HashMap<String, String>,
    pub nested: Vec<ValidationError>,
}

impl ValidationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
            params: std::collections::HashMap::new(),
            nested: Vec::new(),
        }
    }
    
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
    
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }
    
    pub fn with_nested(mut self, errors: Vec<ValidationError>) -> Self {
        self.nested = errors;
        self
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(field) = &self.field {
            write!(f, "{}: {}", field, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for ValidationError {}

// ============================================================================
// USAGE EXAMPLES
// ============================================================================

#[cfg(test)]
mod examples {
    use super::*;
    
    #[test]
    fn example_basic_validation() {
        let validator = MinLength { min: 5 };
        
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }
    
    #[test]
    fn example_composition() {
        let validator = MinLength { min: 5 }
            .and(MatchesRegex { 
                pattern: regex::Regex::new(r"^\w+$").unwrap() 
            });
        
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err()); // too short
        assert!(validator.validate("hello world").is_err()); // space not allowed
    }
    
    #[test]
    fn example_refined_types() {
        let validator = MinLength { min: 5 };
        
        // Создаем refined type
        let validated = Refined::new("hello".to_string(), &validator).unwrap();
        
        // Теперь компилятор знает, что это валидная строка!
        let value = validated.into_inner();
        assert_eq!(value, "hello");
    }
    
    #[test]
    fn example_type_state() {
        let param = Parameter::new("hello".to_string());
        
        let validator = MinLength { min: 5 };
        let validated = param.validate(&validator).unwrap();
        
        // Можем безопасно unwrap - тип гарантирует валидность
        let value = validated.unwrap();
        assert_eq!(value, "hello");
    }
}
