// Prototype для объединенного Number типа
use rust_decimal::Decimal;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Number {
    /// 64-bit signed integer
    Int(i64),
    /// 64-bit floating point
    Float(f64),
    /// Arbitrary precision decimal
    Decimal(Decimal),
}

impl Number {
    // ==================== Constructors ====================

    pub fn int(value: i64) -> Self {
        Self::Int(value)
    }

    pub fn float(value: f64) -> Self {
        Self::Float(value)
    }

    pub fn decimal(value: Decimal) -> Self {
        Self::Decimal(value)
    }

    // ==================== Type queries ====================

    pub fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    pub fn is_decimal(&self) -> bool {
        matches!(self, Self::Decimal(_))
    }

    pub fn is_finite(&self) -> bool {
        match self {
            Self::Int(_) | Self::Decimal(_) => true,
            Self::Float(f) => f.is_finite(),
        }
    }

    // ==================== Conversions ====================

    /// Convert to i64, with potential loss of precision
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) => {
                if f.is_finite() && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                    Some(*f as i64)
                } else {
                    None
                }
            }
            Self::Decimal(d) => d.to_i64(),
        }
    }

    /// Convert to f64, always succeeds but may lose precision
    pub fn to_f64(&self) -> f64 {
        match self {
            Self::Int(i) => *i as f64,
            Self::Float(f) => *f,
            Self::Decimal(d) => d.to_f64_with_default(f64::NAN),
        }
    }

    /// Convert to Decimal, may lose precision for very large floats
    pub fn to_decimal(&self) -> Option<Decimal> {
        match self {
            Self::Int(i) => Decimal::from_i64(*i),
            Self::Float(f) => Decimal::from_f64(*f),
            Self::Decimal(d) => Some(*d),
        }
    }

    // ==================== Mathematical operations ====================

    pub fn is_positive(&self) -> bool {
        match self {
            Self::Int(i) => *i > 0,
            Self::Float(f) => f.is_finite() && *f > 0.0,
            Self::Decimal(d) => d.is_positive(),
        }
    }

    pub fn is_negative(&self) -> bool {
        match self {
            Self::Int(i) => *i < 0,
            Self::Float(f) => f.is_finite() && *f < 0.0,
            Self::Decimal(d) => d.is_negative(),
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Self::Int(i) => *i == 0,
            Self::Float(f) => *f == 0.0,
            Self::Decimal(d) => d.is_zero(),
        }
    }

    /// Add two numbers, promoting to appropriate precision
    pub fn add(&self, other: &Self) -> Self {
        use Number::*;
        match (self, other) {
            // Same types - preserve type
            (Int(a), Int(b)) => {
                if let Some(result) = a.checked_add(*b) {
                    Int(result)
                } else {
                    // Overflow - promote to decimal
                    let a_dec = Decimal::from_i64(*a).unwrap();
                    let b_dec = Decimal::from_i64(*b).unwrap();
                    Decimal(a_dec + b_dec)
                }
            }
            (Float(a), Float(b)) => Float(a + b),
            (Decimal(a), Decimal(b)) => Decimal(a + b),

            // Mixed types - promote to higher precision
            (Int(a), Float(b)) | (Float(b), Int(a)) => Float(*a as f64 + b),
            (Int(a), Decimal(b)) | (Decimal(b), Int(a)) => {
                let a_dec = Decimal::from_i64(*a).unwrap();
                Decimal(a_dec + b)
            }
            (Float(a), Decimal(b)) | (Decimal(b), Float(a)) => {
                if let Some(a_dec) = Decimal::from_f64(*a) {
                    Decimal(a_dec + b)
                } else {
                    // Can't convert float to decimal, use float
                    Float(a + b.to_f64_with_default(f64::NAN))
                }
            }
        }
    }

    // ==================== JSON serialization strategy ====================

    /// How this number should be serialized to JSON
    pub fn json_strategy(&self) -> JsonNumberStrategy {
        match self {
            Self::Int(_) => JsonNumberStrategy::Number,
            Self::Float(f) => {
                if f.is_finite() {
                    JsonNumberStrategy::Number
                } else {
                    JsonNumberStrategy::String  // NaN, Infinity
                }
            }
            Self::Decimal(_) => JsonNumberStrategy::String, // Preserve precision
        }
    }
}

pub enum JsonNumberStrategy {
    Number,  // Use JSON number
    String,  // Use JSON string
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{}", i),
            Self::Float(fl) => {
                if fl.is_finite() {
                    write!(f, "{}", fl)
                } else if fl.is_nan() {
                    write!(f, "NaN")
                } else if fl.is_infinite() && fl.is_sign_positive() {
                    write!(f, "Infinity")
                } else {
                    write!(f, "-Infinity")
                }
            }
            Self::Decimal(d) => write!(f, "{}", d),
        }
    }
}

// ==================== Пример использования ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_operations() {
        let a = Number::int(42);
        let b = Number::float(3.14);
        let c = Number::decimal(Decimal::new(1234, 2)); // 12.34

        // Type queries
        assert!(a.is_int());
        assert!(b.is_float());
        assert!(c.is_decimal());

        // Conversions
        assert_eq!(a.to_i64(), Some(42));
        assert_eq!(b.to_f64(), 3.14);
        assert_eq!(c.to_decimal(), Some(Decimal::new(1234, 2)));

        // Mathematical operations
        assert!(a.is_positive());
        assert!(!a.is_zero());

        // Addition with type promotion
        let result = a.add(&b); // int + float = float
        assert!(result.is_float());
        assert_eq!(result.to_f64(), 45.14);

        let result2 = a.add(&c); // int + decimal = decimal
        assert!(result2.is_decimal());
    }
}

/*
PROPOSAL ANALYSIS:

✅ Плюсы унифицированного Number:
1. Упрощенный API - один тип для чисел
2. Автоматическое продвижение типов при операциях
3. Более естественная семантика для пользователей
4. Упрощение JSON сериализации
5. Лучше соответствует концепции "Value" как универсального контейнера

❌ Минусы:
1. Breaking change - потребует миграции всего API
2. Сложность реализации арифметических операций
3. Потенциальная потеря производительности (дополнительные проверки типов)
4. Может быть неочевидно, какой тип будет результат операции

🤔 Альтернативы:
1. Оставить как есть - три отдельных типа
2. Добавить Number как alias/wrapper над существующими типами
3. Добавить удобные методы для работы с числами как группой

Рекомендация: Это значительное изменение архитектуры.
Стоит рассмотреть в рамках M1 (спецификация модели данных)
если это действительно улучшит пользовательский опыт.
*/