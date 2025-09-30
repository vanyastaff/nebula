use super::Float;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Hashable wrapper for Float that can be used in HashMap/HashSet
///
/// This wrapper implements Eq and Hash by:
/// - Treating all NaN values as equal
/// - Normalizing +0.0 and -0.0 to the same value
/// - Using bit representation for hashing
///
/// **Use with caution**: This violates IEEE 754 semantics where NaN != NaN.
/// Only use when you need to store floats in hash-based collections.
#[derive(Debug, Clone, Copy)]
pub struct HashableFloat(Float);

impl HashableFloat {
    /// Create a new hashable float
    pub const fn new(value: Float) -> Self {
        Self(value)
    }

    /// Create from f64
    pub const fn from_f64(value: f64) -> Self {
        Self(Float::new(value))
    }

    /// Get the inner Float
    pub const fn inner(&self) -> Float {
        self.0
    }

    /// Get the f64 value
    pub const fn value(&self) -> f64 {
        self.0.value()
    }
}

impl PartialEq for HashableFloat {
    fn eq(&self, other: &Self) -> bool {
        // NaN == NaN for hashing purposes
        if self.0.is_nan() && other.0.is_nan() {
            return true;
        }

        // +0.0 == -0.0
        if self.value() == 0.0 && other.value() == 0.0 {
            return true;
        }

        self.0 == other.0
    }
}

// Eq can be implemented because we defined equality semantics
impl Eq for HashableFloat {}

impl PartialOrd for HashableFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HashableFloat {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Hash for HashableFloat {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.0.is_nan() {
            // All NaN values hash to the same value
            f64::NAN.to_bits().hash(state);
        } else if self.value() == 0.0 {
            // +0.0 and -0.0 hash to the same value
            0.0f64.to_bits().hash(state);
        } else {
            self.0.to_bits().hash(state);
        }
    }
}

impl fmt::Display for HashableFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Float> for HashableFloat {
    fn from(f: Float) -> Self {
        Self(f)
    }
}

impl From<f64> for HashableFloat {
    fn from(v: f64) -> Self {
        Self(Float::new(v))
    }
}

impl From<HashableFloat> for Float {
    fn from(h: HashableFloat) -> Self {
        h.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn test_nan_equality() {
        let nan1 = HashableFloat::from_f64(f64::NAN);
        let nan2 = HashableFloat::from_f64(f64::NAN);

        // HashableFloat: NaN == NaN
        assert_eq!(nan1, nan2);
    }

    #[test]
    fn test_zero_equality() {
        let pos_zero = HashableFloat::from_f64(0.0);
        let neg_zero = HashableFloat::from_f64(-0.0);

        // +0.0 == -0.0
        assert_eq!(pos_zero, neg_zero);
    }

    #[test]
    fn test_hash_consistency() {
        use std::collections::hash_map::DefaultHasher;

        let hash_fn = |f: HashableFloat| {
            let mut hasher = DefaultHasher::new();
            f.hash(&mut hasher);
            hasher.finish()
        };

        // NaN values hash the same
        assert_eq!(
            hash_fn(HashableFloat::from_f64(f64::NAN)),
            hash_fn(HashableFloat::from_f64(f64::NAN))
        );

        // +0.0 and -0.0 hash the same
        assert_eq!(
            hash_fn(HashableFloat::from_f64(0.0)),
            hash_fn(HashableFloat::from_f64(-0.0))
        );

        // Normal values hash consistently
        let f = HashableFloat::from_f64(3.14);
        assert_eq!(hash_fn(f), hash_fn(f));
    }

    #[test]
    fn test_hashmap_usage() {
        let mut map = HashMap::new();

        // Can use HashableFloat as key
        map.insert(HashableFloat::from_f64(3.14), "pi");
        map.insert(HashableFloat::from_f64(2.71), "e");
        map.insert(HashableFloat::from_f64(f64::NAN), "nan");

        assert_eq!(map.get(&HashableFloat::from_f64(3.14)), Some(&"pi"));
        assert_eq!(map.get(&HashableFloat::from_f64(f64::NAN)), Some(&"nan"));

        // All NaN values map to the same entry
        assert_eq!(map.get(&HashableFloat::from_f64(f64::NAN)), Some(&"nan"));
    }

    #[test]
    fn test_hashset_usage() {
        let mut set = HashSet::new();

        set.insert(HashableFloat::from_f64(3.14));
        set.insert(HashableFloat::from_f64(2.71));
        set.insert(HashableFloat::from_f64(f64::NAN));
        set.insert(HashableFloat::from_f64(f64::NAN)); // Duplicate

        // Only 3 unique values (second NaN is duplicate)
        assert_eq!(set.len(), 3);
        assert!(set.contains(&HashableFloat::from_f64(3.14)));
        assert!(set.contains(&HashableFloat::from_f64(f64::NAN)));
    }

    #[test]
    fn test_ordering() {
        let mut values = vec![
            HashableFloat::from_f64(f64::NAN),
            HashableFloat::from_f64(3.14),
            HashableFloat::from_f64(f64::NEG_INFINITY),
            HashableFloat::from_f64(0.0),
            HashableFloat::from_f64(f64::INFINITY),
        ];

        values.sort();

        // Order: -Infinity < 0 < 3.14 < +Infinity < NaN
        assert!(values[0].inner().is_negative_infinity());
        assert_eq!(values[1].value(), 0.0);
        assert_eq!(values[2].value(), 3.14);
        assert!(values[3].inner().is_positive_infinity());
        assert!(values[4].inner().is_nan());
    }
}