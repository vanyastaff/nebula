//! Action component collection for dependency declarations.
//!
//! Provides a structured way for actions to declare their runtime dependencies
//! on credentials and resources. These declarations enable:
//! - Compile-time verification of required dependencies
//! - Runtime dependency resolution and injection
//! - Static analysis of action requirements

use nebula_credential::CredentialRef;
use nebula_resource::ResourceRef;

/// Declares the runtime dependencies required by an action.
///
/// Actions declare their credential and resource requirements through this
/// type. The runtime uses these declarations to:
/// - Verify all dependencies are available before execution
/// - Inject dependencies into the action context
/// - Enable static analysis and validation of workflow configurations
///
/// # Example
///
/// ```rust
/// use nebula_action::ActionComponents;
/// use nebula_credential::CredentialRef;
/// use nebula_resource::ResourceRef;
///
/// struct GithubToken;
/// struct PostgresDb;
///
/// let components = ActionComponents::new()
///     .credential(CredentialRef::of::<GithubToken>())
///     .resource(ResourceRef::of::<PostgresDb>());
///
/// assert_eq!(components.credentials().len(), 1);
/// assert_eq!(components.resources().len(), 1);
/// ```
#[derive(Clone, Debug, Default)]
pub struct ActionComponents {
    credentials: Vec<CredentialRef>,
    resources: Vec<ResourceRef>,
}

impl ActionComponents {
    /// Create an empty component collection.
    pub fn new() -> Self {
        Self {
            credentials: Vec::new(),
            resources: Vec::new(),
        }
    }

    /// Add a credential dependency.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_action::ActionComponents;
    /// use nebula_credential::CredentialRef;
    ///
    /// struct ApiToken;
    ///
    /// let components = ActionComponents::new()
    ///     .credential(CredentialRef::of::<ApiToken>());
    /// ```
    pub fn credential(mut self, cred: CredentialRef) -> Self {
        self.credentials.push(cred);
        self
    }

    /// Add a resource dependency.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_action::ActionComponents;
    /// use nebula_resource::ResourceRef;
    ///
    /// struct DbConnection;
    ///
    /// let components = ActionComponents::new()
    ///     .resource(ResourceRef::of::<DbConnection>());
    /// ```
    pub fn resource(mut self, res: ResourceRef) -> Self {
        self.resources.push(res);
        self
    }

    /// Add multiple credential dependencies.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_action::ActionComponents;
    /// use nebula_credential::CredentialRef;
    ///
    /// struct Token1;
    /// struct Token2;
    ///
    /// let components = ActionComponents::new()
    ///     .with_credentials(vec![
    ///         CredentialRef::of::<Token1>(),
    ///         CredentialRef::of::<Token2>(),
    ///     ]);
    /// ```
    pub fn with_credentials(mut self, creds: Vec<CredentialRef>) -> Self {
        self.credentials.extend(creds);
        self
    }

    /// Add multiple resource dependencies.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_action::ActionComponents;
    /// use nebula_resource::ResourceRef;
    ///
    /// struct Db;
    /// struct Cache;
    ///
    /// let components = ActionComponents::new()
    ///     .with_resources(vec![
    ///         ResourceRef::of::<Db>(),
    ///         ResourceRef::of::<Cache>(),
    ///     ]);
    /// ```
    pub fn with_resources(mut self, resources: Vec<ResourceRef>) -> Self {
        self.resources.extend(resources);
        self
    }

    /// Get the declared credential dependencies.
    pub fn credentials(&self) -> &[CredentialRef] {
        &self.credentials
    }

    /// Get the declared resource dependencies.
    pub fn resources(&self) -> &[ResourceRef] {
        &self.resources
    }

    /// Check if any dependencies are declared.
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty() && self.resources.is_empty()
    }

    /// Count total number of dependencies.
    pub fn len(&self) -> usize {
        self.credentials.len() + self.resources.len()
    }

    /// Consume and split into parts.
    pub fn into_parts(self) -> (Vec<CredentialRef>, Vec<ResourceRef>) {
        (self.credentials, self.resources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCredential;
    struct TestResource;
    struct AnotherCredential;
    struct AnotherResource;

    #[test]
    fn test_empty_components() {
        let components = ActionComponents::new();
        assert!(components.is_empty());
        assert_eq!(components.len(), 0);
        assert_eq!(components.credentials().len(), 0);
        assert_eq!(components.resources().len(), 0);
    }

    #[test]
    fn test_single_credential() {
        let components = ActionComponents::new().credential(CredentialRef::of::<TestCredential>());

        assert!(!components.is_empty());
        assert_eq!(components.len(), 1);
        assert_eq!(components.credentials().len(), 1);
        assert_eq!(components.resources().len(), 0);
    }

    #[test]
    fn test_single_resource() {
        let components = ActionComponents::new().resource(ResourceRef::of::<TestResource>());

        assert!(!components.is_empty());
        assert_eq!(components.len(), 1);
        assert_eq!(components.credentials().len(), 0);
        assert_eq!(components.resources().len(), 1);
    }

    #[test]
    fn test_multiple_dependencies() {
        let components = ActionComponents::new()
            .credential(CredentialRef::of::<TestCredential>())
            .credential(CredentialRef::of::<AnotherCredential>())
            .resource(ResourceRef::of::<TestResource>())
            .resource(ResourceRef::of::<AnotherResource>());

        assert!(!components.is_empty());
        assert_eq!(components.len(), 4);
        assert_eq!(components.credentials().len(), 2);
        assert_eq!(components.resources().len(), 2);
    }

    #[test]
    fn test_batch_add_credentials() {
        let components = ActionComponents::new().with_credentials(vec![
            CredentialRef::of::<TestCredential>(),
            CredentialRef::of::<AnotherCredential>(),
        ]);

        assert_eq!(components.credentials().len(), 2);
    }

    #[test]
    fn test_batch_add_resources() {
        let components = ActionComponents::new().with_resources(vec![
            ResourceRef::of::<TestResource>(),
            ResourceRef::of::<AnotherResource>(),
        ]);

        assert_eq!(components.resources().len(), 2);
    }

    #[test]
    fn test_into_parts() {
        let components = ActionComponents::new()
            .credential(CredentialRef::of::<TestCredential>())
            .resource(ResourceRef::of::<TestResource>());

        let (creds, resources) = components.into_parts();
        assert_eq!(creds.len(), 1);
        assert_eq!(resources.len(), 1);
    }

    #[test]
    fn test_clone() {
        let components = ActionComponents::new()
            .credential(CredentialRef::of::<TestCredential>())
            .resource(ResourceRef::of::<TestResource>());

        let cloned = components.clone();
        assert_eq!(cloned.len(), components.len());
        assert_eq!(cloned.credentials().len(), components.credentials().len());
        assert_eq!(cloned.resources().len(), components.resources().len());
    }

    #[test]
    fn test_default() {
        let components = ActionComponents::default();
        assert!(components.is_empty());
    }

    #[test]
    fn test_builder_chain() {
        let components = ActionComponents::new()
            .credential(CredentialRef::of::<TestCredential>())
            .with_resources(vec![
                ResourceRef::of::<TestResource>(),
                ResourceRef::of::<AnotherResource>(),
            ])
            .credential(CredentialRef::of::<AnotherCredential>());

        assert_eq!(components.credentials().len(), 2);
        assert_eq!(components.resources().len(), 2);
    }
}
