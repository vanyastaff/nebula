use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;

/// An ordered collection of parameter definitions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterCollection {
    parameters: Vec<ParameterDef>,
}

impl ParameterCollection {
    /// Create an empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a parameter definition to the collection.
    pub fn add(&mut self, param: ParameterDef) -> &mut Self {
        self.parameters.push(param);
        self
    }

    /// Add a parameter definition (builder-style, consuming).
    #[must_use]
    pub fn with(mut self, param: ParameterDef) -> Self {
        self.parameters.push(param);
        self
    }

    /// Get a parameter by index.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&ParameterDef> {
        self.parameters.get(index)
    }

    /// Get a parameter by its key.
    #[must_use]
    pub fn get_by_key(&self, key: &str) -> Option<&ParameterDef> {
        self.parameters.iter().find(|p| p.key() == key)
    }

    /// Remove and return a parameter by key.
    pub fn remove(&mut self, key: &str) -> Option<ParameterDef> {
        let idx = self.parameters.iter().position(|p| p.key() == key)?;
        Some(self.parameters.remove(idx))
    }

    /// Check whether a parameter with the given key exists.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.parameters.iter().any(|p| p.key() == key)
    }

    /// Iterate over all parameter keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.parameters.iter().map(|p| p.key())
    }

    /// The number of parameters in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Whether the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }

    /// Iterate over all parameter definitions.
    pub fn iter(&self) -> impl Iterator<Item = &ParameterDef> {
        self.parameters.iter()
    }

    /// Iterate mutably over all parameter definitions.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ParameterDef> {
        self.parameters.iter_mut()
    }
}

impl IntoIterator for ParameterCollection {
    type Item = ParameterDef;
    type IntoIter = std::vec::IntoIter<ParameterDef>;

    fn into_iter(self) -> Self::IntoIter {
        self.parameters.into_iter()
    }
}

impl<'a> IntoIterator for &'a ParameterCollection {
    type Item = &'a ParameterDef;
    type IntoIter = std::slice::Iter<'a, ParameterDef>;

    fn into_iter(self) -> Self::IntoIter {
        self.parameters.iter()
    }
}

impl FromIterator<ParameterDef> for ParameterCollection {
    fn from_iter<I: IntoIterator<Item = ParameterDef>>(iter: I) -> Self {
        Self {
            parameters: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    #[test]
    fn new_is_empty() {
        let col = ParameterCollection::new();
        assert!(col.is_empty());
        assert_eq!(col.len(), 0);
    }

    #[test]
    fn add_and_get() {
        let mut col = ParameterCollection::new();
        col.add(ParameterDef::Text(TextParameter::new("host", "Hostname")));
        col.add(ParameterDef::Number(NumberParameter::new("port", "Port")));

        assert_eq!(col.len(), 2);
        assert_eq!(col.get(0).unwrap().key(), "host");
        assert_eq!(col.get(1).unwrap().key(), "port");
        assert!(col.get(2).is_none());
    }

    #[test]
    fn with_builder() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        assert_eq!(col.len(), 2);
    }

    #[test]
    fn get_by_key() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("host", "Host")))
            .with(ParameterDef::Number(NumberParameter::new("port", "Port")));

        assert_eq!(col.get_by_key("port").unwrap().key(), "port");
        assert!(col.get_by_key("missing").is_none());
    }

    #[test]
    fn remove_by_key() {
        let mut col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        let removed = col.remove("a");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().key(), "a");
        assert_eq!(col.len(), 1);
        assert!(col.remove("missing").is_none());
    }

    #[test]
    fn contains() {
        let col =
            ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("host", "Host")));

        assert!(col.contains("host"));
        assert!(!col.contains("port"));
    }

    #[test]
    fn keys_iterator() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        let keys: Vec<&str> = col.keys().collect();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn iter_and_into_iter() {
        let col = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("x", "X")));

        assert_eq!(col.iter().count(), 1);

        let keys: Vec<&str> = (&col).into_iter().map(|p| p.key()).collect();
        assert_eq!(keys, vec!["x"]);

        let owned_keys: Vec<String> = col.into_iter().map(|p| p.key().to_owned()).collect();
        assert_eq!(owned_keys, vec!["x"]);
    }

    #[test]
    fn from_iterator() {
        let defs = vec![
            ParameterDef::Text(TextParameter::new("a", "A")),
            ParameterDef::Text(TextParameter::new("b", "B")),
        ];

        let col: ParameterCollection = defs.into_iter().collect();
        assert_eq!(col.len(), 2);
    }

    #[test]
    fn iter_mut_modifies_in_place() {
        let mut col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        for param in col.iter_mut() {
            param.metadata_mut().required = true;
        }

        assert!(col.get(0).unwrap().is_required());
        assert!(col.get(1).unwrap().is_required());
    }

    #[test]
    fn partial_eq_collections() {
        let a = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("x", "X")));
        let b = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("x", "X")));
        assert_eq!(a, b);

        let c = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("y", "Y")));
        assert_ne!(a, c);
    }

    #[test]
    fn serde_round_trip() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("host", "Host")))
            .with(ParameterDef::Number(NumberParameter::new("port", "Port")));

        let json = serde_json::to_string(&col).unwrap();
        let deserialized: ParameterCollection = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized.get(0).unwrap().key(), "host");
        assert_eq!(deserialized.get(1).unwrap().key(), "port");
    }
}
