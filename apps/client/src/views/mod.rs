//! Rendering only. Views read the workbench and change purely local UI state directly. Anything
//! that needs the network becomes an `Intent`, which the app runs after the frame is drawn.

pub(crate) mod canvas;
pub(crate) mod connection;
pub(crate) mod editor;
pub(crate) mod form;
pub(crate) mod inspector;
pub(crate) mod navigator;
pub(crate) mod runs;
pub(crate) mod shell;

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
    LoadRecentRuns,
    /// Recent runs and the chosen run, read in one request.
    RefreshRuns,
    LoadExecution(String),
    LoadCatalog,
    /// The parameter schema of an action, for the node form.
    LoadSchema(String),
}

pub(crate) type Intents = Vec<Intent>;
