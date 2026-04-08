pub mod actions;
pub mod completion;
pub mod config;
pub mod dev;
pub mod plugin;
pub mod plugin_new;
pub mod replay;
pub mod run;
pub mod validate;

/// Shared exit code constants.
pub mod exit_codes {
    /// Workflow executed but finished with non-success status.
    pub const WORKFLOW_FAILED: u8 = 2;
    /// Workflow validation found errors.
    pub const VALIDATION_FAILED: u8 = 3;
    /// Execution timed out.
    pub const TIMEOUT: u8 = 4;
}
