//! Action component collection for dependency declarations.
//!
//! Provides a structured way for actions to declare their runtime dependencies
//! on credentials and resources. These declarations enable:
//! - Compile-time verification of required dependencies
//! - Runtime dependency resolution and injection
//! - Static analysis of action requirements

use nebula_credential::core::reference::ErasedCredentialRef;
use nebula_resource::reference::ErasedResourceRef;

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
/// ```rust,ignore
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
    credentials: Vec<ErasedCredentialRef>,
    resources: Vec<ErasedResourceRef>,
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
    /// ```rust,ignore
    /// use nebula_action::ActionComponents;
    /// use nebula_credential::CredentialRef;
    ///
    /// struct ApiToken;
    /// // impl CredentialType for ApiToken { ... }
    ///
    /// let components = ActionComponents::new()
    ///     .credential(CredentialRef::of::<ApiToken>());
    /// ```
    pub fn credential(mut self, cred: impl Into<ErasedCredentialRef>) -> Self {
        self.credentials.push(cred.into());
        self
    }

    /// Add a resource dependency.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_action::ActionComponents;
    /// use nebula_resource::ResourceRef;
    ///
    /// struct DbConnection;
    /// // impl Resource for DbConnection { ... }
    ///
    /// let components = ActionComponents::new()
    ///     .resource(ResourceRef::of::<DbConnection>());
    /// ```
    pub fn resource(mut self, res: impl Into<ErasedResourceRef>) -> Self {
        self.resources.push(res.into());
        self
    }

    /// Add multiple credential dependencies.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_action::ActionComponents;
    /// use nebula_credential::CredentialRef;
    ///
    /// struct Token1;
    /// struct Token2;
    ///
    /// let components = ActionComponents::new()
    ///     .with_credentials(vec![
    ///         CredentialRef::of::<Token1>().into(),
    ///         CredentialRef::of::<Token2>().into(),
    ///     ]);
    /// ```
    pub fn with_credentials(
        mut self,
        creds: impl IntoIterator<Item = impl Into<ErasedCredentialRef>>,
    ) -> Self {
        self.credentials.extend(creds.into_iter().map(Into::into));
        self
    }

    /// Add multiple resource dependencies.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_action::ActionComponents;
    /// use nebula_resource::ResourceRef;
    ///
    /// struct Db;
    /// struct Cache;
    ///
    /// let components = ActionComponents::new()
    ///     .with_resources(vec![
    ///         ResourceRef::of::<Db>().into(),
    ///         ResourceRef::of::<Cache>().into(),
    ///     ]);
    /// ```
    pub fn with_resources(
        mut self,
        resources: impl IntoIterator<Item = impl Into<ErasedResourceRef>>,
    ) -> Self {
        self.resources.extend(resources.into_iter().map(Into::into));
        self
    }

    /// Get the declared credential dependencies.
    pub fn credentials(&self) -> &[ErasedCredentialRef] {
        &self.credentials
    }

    /// Get the declared resource dependencies.
    pub fn resources(&self) -> &[ErasedResourceRef] {
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
    pub fn into_parts(self) -> (Vec<ErasedCredentialRef>, Vec<ErasedResourceRef>) {
        (self.credentials, self.resources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nebula_core::ResourceKey;
    use nebula_credential::CredentialRef;
    use nebula_credential::core::reference::ErasedCredentialRef;
    use nebula_credential::core::result::InitializeResult;
    use nebula_credential::core::{CredentialContext, CredentialDescription};
    use nebula_credential::traits::CredentialType;
    use nebula_parameter::schema::Schema;
    use nebula_resource::ResourceRef;
    use nebula_resource::context::Context;
    use nebula_resource::metadata::ResourceMetadata;
    use nebula_resource::reference::ErasedResourceRef;
    use nebula_resource::resource::Resource;

    struct TestCredential;
    struct TestResource;
    struct AnotherCredential;
    struct AnotherResource;

    #[async_trait]
    impl CredentialType for TestCredential {
        type Input = ();
        type State = nebula_credential::protocols::ApiKeyState;
        fn description() -> CredentialDescription {
            CredentialDescription::builder()
                .key("test_credential")
                .name("Test")
                .description("")
                .properties(Schema::new())
                .build()
                .unwrap()
        }
        async fn initialize(
            &self,
            _: &(),
            _: &mut CredentialContext,
        ) -> Result<InitializeResult<Self::State>, nebula_credential::core::CredentialError>
        {
            unreachable!()
        }
    }

    #[async_trait]
    impl CredentialType for AnotherCredential {
        type Input = ();
        type State = nebula_credential::protocols::ApiKeyState;
        fn description() -> CredentialDescription {
            CredentialDescription::builder()
                .key("another_credential")
                .name("Another")
                .description("")
                .properties(Schema::new())
                .build()
                .unwrap()
        }
        async fn initialize(
            &self,
            _: &(),
            _: &mut CredentialContext,
        ) -> Result<InitializeResult<Self::State>, nebula_credential::core::CredentialError>
        {
            unreachable!()
        }
    }

    struct TestResourceConfig;
    impl nebula_resource::resource::Config for TestResourceConfig {}

    impl Resource for TestResource {
        type Config = TestResourceConfig;
        type Instance = ();
        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("test_resource").unwrap())
        }
        async fn create(
            &self,
            _: &TestResourceConfig,
            _: &Context,
        ) -> nebula_resource::Result<()> {
            Ok(())
        }
    }

    impl Resource for AnotherResource {
        type Config = TestResourceConfig;
        type Instance = ();
        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("another_resource").unwrap())
        }
        async fn create(
            &self,
            _: &TestResourceConfig,
            _: &Context,
        ) -> nebula_resource::Result<()> {
            Ok(())
        }
    }

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
        let components = ActionComponents::new().credential(CredentialRef::<TestCredential>::of());

        assert!(!components.is_empty());
        assert_eq!(components.len(), 1);
        assert_eq!(components.credentials().len(), 1);
        assert_eq!(components.resources().len(), 0);
    }

    #[test]
    fn test_single_resource() {
        let components = ActionComponents::new().resource(ResourceRef::<TestResource>::of());

        assert!(!components.is_empty());
        assert_eq!(components.len(), 1);
        assert_eq!(components.credentials().len(), 0);
        assert_eq!(components.resources().len(), 1);
    }

    #[test]
    fn test_multiple_dependencies() {
        let components = ActionComponents::new()
            .credential(CredentialRef::<TestCredential>::of())
            .credential(CredentialRef::<AnotherCredential>::of())
            .resource(ResourceRef::<TestResource>::of())
            .resource(ResourceRef::<AnotherResource>::of());

        assert!(!components.is_empty());
        assert_eq!(components.len(), 4);
        assert_eq!(components.credentials().len(), 2);
        assert_eq!(components.resources().len(), 2);
    }

    #[test]
    fn test_batch_add_credentials() {
        let creds: Vec<ErasedCredentialRef> = vec![
            CredentialRef::<TestCredential>::of().erase(),
            CredentialRef::<AnotherCredential>::of().erase(),
        ];
        let components = ActionComponents::new().with_credentials(creds);
        assert_eq!(components.credentials().len(), 2);
    }

    #[test]
    fn test_batch_add_resources() {
        let resources: Vec<ErasedResourceRef> = vec![
            ResourceRef::<TestResource>::of().erase(),
            ResourceRef::<AnotherResource>::of().erase(),
        ];
        let components = ActionComponents::new().with_resources(resources);
        assert_eq!(components.resources().len(), 2);
    }

    #[test]
    fn test_into_parts() {
        let components = ActionComponents::new()
            .credential(CredentialRef::<TestCredential>::of())
            .resource(ResourceRef::<TestResource>::of());

        let (creds, resources) = components.into_parts();
        assert_eq!(creds.len(), 1);
        assert_eq!(resources.len(), 1);
    }

    #[test]
    fn test_clone() {
        let components = ActionComponents::new()
            .credential(CredentialRef::<TestCredential>::of())
            .resource(ResourceRef::<TestResource>::of());

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
        let resources: Vec<ErasedResourceRef> = vec![
            ResourceRef::<TestResource>::of().erase(),
            ResourceRef::<AnotherResource>::of().erase(),
        ];
        let components = ActionComponents::new()
            .credential(CredentialRef::<TestCredential>::of())
            .with_resources(resources)
            .credential(CredentialRef::<AnotherCredential>::of());

        assert_eq!(components.credentials().len(), 2);
        assert_eq!(components.resources().len(), 2);
    }
}
