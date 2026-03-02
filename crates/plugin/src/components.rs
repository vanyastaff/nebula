//! Plugin component collection in ref style: credentials, resources, actions.
//!
//! Components are declared with [`CredentialRef`], [`ResourceRef`], and [`ActionRef`] only.

use nebula_action::ActionRef;
use nebula_credential::CredentialRef;
use nebula_resource::ResourceRef;

/// Declares the runtime components of a plugin, in ref style only.
///
/// Plugins declare credentials, resources, and action types via refs (same style
/// as [`ActionComponents`](nebula_action::ActionComponents)).
///
/// # Example
///
/// ```rust,ignore
/// use nebula_plugin::PluginComponents;
/// use nebula_credential::CredentialRef;
/// use nebula_resource::ResourceRef;
/// use nebula_action::ActionRef;
///
/// struct GithubToken;
/// struct PostgresDb;
/// struct HttpRequestAction;
///
/// let mut components = PluginComponents::new();
/// components
///     .credential(CredentialRef::of::<GithubToken>())
///     .resource(ResourceRef::of::<PostgresDb>())
///     .action(ActionRef::of::<HttpRequestAction>());
/// ```
#[derive(Clone, Default)]
pub struct PluginComponents {
    credentials: Vec<CredentialRef>,
    resources: Vec<ResourceRef>,
    actions: Vec<ActionRef>,
}

impl PluginComponents {
    /// Create an empty component collection.
    pub fn new() -> Self {
        Self {
            credentials: Vec::new(),
            resources: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// Declare a required credential (ref style).
    pub fn credential(&mut self, cred: CredentialRef) -> &mut Self {
        self.credentials.push(cred);
        self
    }

    /// Declare a required resource (ref style).
    pub fn resource(&mut self, res: ResourceRef) -> &mut Self {
        self.resources.push(res);
        self
    }

    /// Declare an action type this plugin provides (ref style).
    pub fn action(&mut self, action: ActionRef) -> &mut Self {
        self.actions.push(action);
        self
    }

    /// Credential refs declared by this plugin.
    pub fn credentials(&self) -> &[CredentialRef] {
        &self.credentials
    }

    /// Resource refs declared by this plugin.
    pub fn resources(&self) -> &[ResourceRef] {
        &self.resources
    }

    /// Action refs declared by this plugin.
    pub fn actions(&self) -> &[ActionRef] {
        &self.actions
    }

    /// Consume and split into parts: credentials, resources, actions.
    pub fn into_parts(self) -> (Vec<CredentialRef>, Vec<ResourceRef>, Vec<ActionRef>) {
        (self.credentials, self.resources, self.actions)
    }
}

impl std::fmt::Debug for PluginComponents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginComponents")
            .field("credentials", &self.credentials.len())
            .field("resources", &self.resources.len())
            .field("actions", &self.actions.len())
            .finish()
    }
}
