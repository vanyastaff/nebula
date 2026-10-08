//! Rendering only. Views read the workbench and change purely local UI state directly. Anything
//! that needs the network becomes an `Intent`, which the app runs after the frame is drawn.

pub(crate) mod canvas;
pub(crate) mod catalog;
pub(crate) mod connection;
pub(crate) mod credentials;
pub(crate) mod editor;
pub(crate) mod executions;
pub(crate) mod form;
pub(crate) mod inspector;
pub(crate) mod nav;
pub(crate) mod runs;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod states;
pub(crate) mod status;
pub(crate) mod team;
pub(crate) mod triggers;
pub(crate) mod workflows;

use serde_json::Value;

/// A network operation a view asked for.
pub(crate) enum Intent {
    SignIn,
    /// Opens the built-in demo workspace instead of signing in to a server.
    OpenDemo,
    OpenWorkspace,
    ListWorkflows(usize),
    CreateWorkflow,
    LoadWorkflow(String),
    SaveDraft,
    PublishDraft,
    RunDraft,
    /// Recent runs and the chosen run, read in one request.
    RefreshRuns,
    LoadExecution(String),
    LoadCatalog,
    /// The parameter schema of an action, for the node form.
    LoadSchema(String),
    /// An action's description, for the catalog page.
    LoadActionDetail(String),
    /// The execution history; `more` reads the page after the shown ones.
    LoadExecutions {
        more: bool,
    },
    /// Shows a run on the executions page and reads it.
    OpenExecution(String),
    CancelExecution(String),
    /// Starts a workflow's published version again.
    RerunWorkflow(String),
    LoadCredentials,
    LoadCredentialTypes,
    /// Saves the credential being created.
    CreateCredential,
    DeleteCredential(String),
    TestCredential(String),
    LoadTriggers,
    /// Replaces a workflow's trigger bindings with these.
    SaveTriggers(String, Value),
    /// Registers the webhook of a workflow's trigger.
    RegisterWebhook(String, String),
    LoadProfile,
    SaveProfile,
    LoadTokens,
    CreateToken,
    RevokeToken(String),
    LoadOrgMembers,
    AddOrgMember,
    RemoveOrgMember(String),
    LoadWorkspaceMembers,
    /// Gives a member this workspace role.
    SetWorkspaceMember(String, String),
    RemoveWorkspaceMember(String),
}

pub(crate) type Intents = Vec<Intent>;

#[cfg(test)]
mod a11y_tests;
