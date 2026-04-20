pub(crate) mod actions;
pub(crate) mod completion;
pub(crate) mod config;
pub(crate) mod dev;
pub(crate) mod plugin;
pub(crate) mod plugin_new;
pub(crate) mod replay;
pub(crate) mod run;
pub(crate) mod validate;
pub(crate) mod watch;

/// Shared exit code constants.
pub(crate) mod exit_codes {
    /// Workflow executed but finished with non-success status.
    pub(crate) const WORKFLOW_FAILED: u8 = 2;
    /// Workflow validation found errors.
    pub(crate) const VALIDATION_FAILED: u8 = 3;
    /// Execution timed out.
    pub(crate) const TIMEOUT: u8 = 4;
}
