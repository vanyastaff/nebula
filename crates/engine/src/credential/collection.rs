use std::collections::HashMap;
use crate::credential::{Credential, CredentialError};
use crate::types::Key;

/// A collection for storing and managing credentials.
///
/// This collection provides methods to add, retrieve, and remove credentials
/// with type-safe access. Each credential is uniquely identified by its key.
#[derive(Debug, Clone, Default)]
pub struct CredentialCollection {
    /// Internal storage for credentials, keyed by their unique keys.
    credentials: HashMap<Key, Box<dyn Credential>>,
}

impl CredentialCollection {
    /// Creates a new empty credential collection.
    pub fn new() -> Self {
        Self {
            credentials: HashMap::new(),
        }
    }

    /// Adds a credential to the collection.
    ///
    /// # Arguments
    ///
    /// * `credential` - The credential to add. It must implement the `Credential` trait.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the credential was successfully added.
    /// * `Err(CredentialError::DuplicateKey)` if a credential with the same key already exists.
    pub fn add<C: Credential + 'static>(&mut self, credential: C) -> Result<(), CredentialError> {
        let key = credential.key().clone();

        // Check if a credential with this key already exists
        if self.credentials.contains_key(&key) {
            return Err(CredentialError::DuplicateKey(key));
        }

        // Add the credential to the collection
        self.credentials.insert(key, Box::new(credential));
        Ok(())
    }

    /// Retrieves a reference to a credential by its key.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the credential to retrieve.
    ///
    /// # Returns
    ///
    /// * `Ok(&dyn Credential)` - A reference to the credential.
    /// * `Err(CredentialError::NotFound)` - If no credential with the given key exists.
    pub fn get(&self, key: &Key) -> Result<&dyn Credential, CredentialError> {
        self.credentials
            .get(key)
            .map(|boxed| boxed.as_ref())
            .ok_or_else(|| CredentialError::NotFound(key.clone()))
    }

    /// Retrieves a mutable reference to a credential by its key.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the credential to retrieve.
    ///
    /// # Returns
    ///
    /// * `Ok(&mut dyn Credential)` - A mutable reference to the credential.
    /// * `Err(CredentialError::NotFound)` - If no credential with the given key exists.
    pub fn get_mut(&mut self, key: &Key) -> Result<&mut dyn Credential, CredentialError> {
        self.credentials
            .get_mut(key)
            .map(|boxed| boxed.as_mut())
            .ok_or_else(|| CredentialError::NotFound(key.clone()))
    }

    /// Retrieves a reference to a credential with a specific type.
    ///
    /// This method attempts to downcast the credential to the specified type.
    ///
    /// # Type Parameters
    ///
    /// * `C` - The credential type to downcast to. Must implement the `Credential` trait.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the credential to retrieve.
    ///
    /// # Returns
    ///
    /// * `Ok(&C)` - A reference to the credential of the specified type.
    /// * `Err(CredentialError::NotFound)` - If no credential with the given key exists.
    /// * `Err(CredentialError::InvalidType)` - If the credential cannot be downcast to the specified type.
    pub fn get_as<C: Credential + 'static>(&self, key: &Key) -> Result<&C, CredentialError> {
        let credential = self.get(key)?;

        // Attempt to downcast to the specified type
        credential.downcast_ref::<C>().ok_or_else(|| CredentialError::InvalidType {
            key: key.clone(),
            expected_type: std::any::type_name::<C>().to_string(),
            actual_details: "Credential has different type".to_string(),
        })
    }

    /// Retrieves a mutable reference to a credential with a specific type.
    ///
    /// This method attempts to downcast the credential to the specified type.
    ///
    /// # Type Parameters
    ///
    /// * `C` - The credential type to downcast to. Must implement the `Credential` trait.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the credential to retrieve.
    ///
    /// # Returns
    ///
    /// * `Ok(&mut C)` - A mutable reference to the credential of the specified type.
    /// * `Err(CredentialError::NotFound)` - If no credential with the given key exists.
    /// * `Err(CredentialError::InvalidType)` - If the credential cannot be downcast to the specified type.
    pub fn get_as_mut<C: Credential + 'static>(&mut self, key: &Key) -> Result<&mut C, CredentialError> {
        let credential = self.get_mut(key)?;

        // Attempt to downcast to the specified type
        credential.downcast_mut::<C>().ok_or_else(|| CredentialError::InvalidType {
            key: key.clone(),
            expected_type: std::any::type_name::<C>().to_string(),
            actual_details: "Credential has different type".to_string(),
        })
    }

    /// Removes a credential from the collection.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the credential to remove.
    ///
    /// # Returns
    ///
    /// * `Ok(Box<dyn Credential>)` - The removed credential.
    /// * `Err(CredentialError::NotFound)` - If no credential with the given key exists.
    pub fn remove(&mut self, key: &Key) -> Result<Box<dyn Credential>, CredentialError> {
        self.credentials
            .remove(key)
            .ok_or_else(|| CredentialError::NotFound(key.clone()))
    }

    /// Checks if the collection contains a credential with the given key.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to check.
    ///
    /// # Returns
    ///
    /// * `true` if a credential with the given key exists, `false` otherwise.
    pub fn contains(&self, key: &Key) -> bool {
        self.credentials.contains_key(key)
    }

    /// Returns the number of credentials in the collection.
    ///
    /// # Returns
    ///
    /// * The number of credentials.
    pub fn len(&self) -> usize {
        self.credentials.len()
    }

    /// Checks if the collection is empty.
    ///
    /// # Returns
    ///
    /// * `true` if the collection is empty, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }

    /// Returns a vector of all credential keys in the collection.
    ///
    /// # Returns
    ///
    /// * A vector containing clones of all keys.
    pub fn keys(&self) -> Vec<Key> {
        self.credentials.keys().cloned().collect()
    }

    /// Finds the first credential of the specified type.
    ///
    /// # Type Parameters
    ///
    /// * `C` - The credential type to search for. Must implement the `Credential` trait.
    ///
    /// # Returns
    ///
    /// * `Some(&C)` - A reference to the first credential of the specified type.
    /// * `None` - If no credential of the specified type exists.
    pub fn find_by_type<C: Credential + 'static>(&self) -> Option<&C> {
        for credential in self.credentials.values() {
            if let Some(typed) = credential.as_ref().downcast_ref::<C>() {
                return Some(typed);
            }
        }
        None
    }
    

    /// Clears all credentials from the collection.
    pub fn clear(&mut self) {
        self.credentials.clear();
    }

    /// Applies a function to each credential in the collection.
    ///
    /// # Arguments
    ///
    /// * `f` - The function to apply to each credential.
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&Key, &dyn Credential),
    {
        for (key, credential) in &self.credentials {
            f(key, credential.as_ref());
        }
    }

    /// Applies a mutable function to each credential in the collection.
    ///
    /// # Arguments
    ///
    /// * `f` - The function to apply to each credential.
    pub fn for_each_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(&Key, &mut dyn Credential),
    {
        for (key, credential) in &mut self.credentials {
            f(key, credential.as_mut());
        }
    }
}