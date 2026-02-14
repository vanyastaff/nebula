use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::result::ActionResult;

/// Type of interaction requested from a human.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InteractionType {
    /// Binary yes/no approval.
    Approval,
    /// Free-form text or structured data input.
    FormInput,
    /// Choose from a set of options.
    Selection,
}

/// A request for human input, sent from an action to the engine.
///
/// The engine pauses execution and forwards this to the appropriate
/// UI channel (web dashboard, Slack, email, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionRequest {
    /// Unique identifier for this interaction.
    pub interaction_id: String,
    /// What kind of human interaction is needed.
    pub interaction_type: InteractionType,
    /// Message shown to the human (markdown supported).
    pub prompt: String,
    /// Options for `Selection` type, form schema for `FormInput`.
    pub options: Option<serde_json::Value>,
    /// Maximum time to wait for a response.
    pub timeout: Duration,
    /// Additional context passed to the UI.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl InteractionRequest {
    /// Create an approval request.
    pub fn approval(id: impl Into<String>, prompt: impl Into<String>, timeout: Duration) -> Self {
        Self {
            interaction_id: id.into(),
            interaction_type: InteractionType::Approval,
            prompt: prompt.into(),
            options: None,
            timeout,
            metadata: HashMap::new(),
        }
    }

    /// Create a form input request.
    pub fn form(
        id: impl Into<String>,
        prompt: impl Into<String>,
        schema: serde_json::Value,
        timeout: Duration,
    ) -> Self {
        Self {
            interaction_id: id.into(),
            interaction_type: InteractionType::FormInput,
            prompt: prompt.into(),
            options: Some(schema),
            timeout,
            metadata: HashMap::new(),
        }
    }

    /// Create a selection request.
    pub fn selection(
        id: impl Into<String>,
        prompt: impl Into<String>,
        choices: Vec<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            interaction_id: id.into(),
            interaction_type: InteractionType::Selection,
            prompt: prompt.into(),
            options: Some(serde_json::json!(choices)),
            timeout,
            metadata: HashMap::new(),
        }
    }
}

/// Human response to an interaction request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResponse {
    /// The interaction this responds to.
    pub interaction_id: String,
    /// Whether the human approved (for `Approval` type).
    pub approved: Option<bool>,
    /// Human-provided data (form values, selected option, etc.).
    pub data: serde_json::Value,
    /// Who responded.
    pub responder: Option<String>,
}

/// Action that requires human input during workflow execution.
///
/// Used for approval workflows, manual data entry, escalation handling,
/// and any process that requires a human decision.
///
/// The execution flow:
/// 1. `request_interaction` — action determines what input is needed.
/// 2. Engine pauses and delivers the request to the human via configured channels.
/// 3. Human responds (or timeout expires).
/// 4. `process_response` — action processes the response and continues.
///
/// # Type Parameters
///
/// - `Input`: data received from upstream nodes.
/// - `Output`: data produced after human interaction completes.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::*;
/// use nebula_action::interactive::*;
/// use async_trait::async_trait;
///
/// struct DeployApproval {
///     meta: ActionMetadata,
/// }
///
/// #[async_trait]
/// impl InteractiveAction for DeployApproval {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     async fn request_interaction(
///         &self, input: Self::Input, ctx: &ActionContext,
///     ) -> Result<InteractionRequest, ActionError> {
///         ctx.check_cancelled()?;
///         Ok(InteractionRequest::approval(
///             "deploy-approval-1",
///             format!("Approve deployment of {}?", input["service"]),
///             std::time::Duration::from_secs(3600),
///         ))
///     }
///
///     async fn process_response(
///         &self, response: InteractionResponse, _input: Self::Input, ctx: &ActionContext,
///     ) -> Result<ActionResult<Self::Output>, ActionError> {
///         ctx.check_cancelled()?;
///         match response.approved {
///             Some(true) => Ok(ActionResult::success(serde_json::json!({"approved": true}))),
///             _ => Ok(ActionResult::skip("deployment rejected")),
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait InteractiveAction: Action {
    /// Input data received from upstream nodes.
    type Input: Send + Sync + 'static;
    /// Output data produced after interaction completes.
    type Output: Send + Sync + 'static;

    /// Determine what human interaction is needed.
    ///
    /// Called first — the returned `InteractionRequest` is delivered
    /// to the human via the engine's notification system.
    async fn request_interaction(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<InteractionRequest, ActionError>;

    /// Process the human's response and produce output.
    ///
    /// Called after the human responds (or timeout triggers).
    /// On timeout, the engine delivers a response with `approved: None`
    /// and empty data.
    async fn process_response(
        &self,
        response: InteractionResponse,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_request() {
        let req = InteractionRequest::approval("req-1", "Approve this?", Duration::from_secs(300));
        assert_eq!(req.interaction_id, "req-1");
        assert!(matches!(req.interaction_type, InteractionType::Approval));
        assert_eq!(req.timeout, Duration::from_secs(300));
        assert!(req.options.is_none());
    }

    #[test]
    fn form_request() {
        let schema =
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let req = InteractionRequest::form(
            "form-1",
            "Enter details",
            schema.clone(),
            Duration::from_secs(600),
        );
        assert!(matches!(req.interaction_type, InteractionType::FormInput));
        assert_eq!(req.options, Some(schema));
    }

    #[test]
    fn selection_request() {
        let req = InteractionRequest::selection(
            "sel-1",
            "Choose environment",
            vec!["staging".into(), "production".into()],
            Duration::from_secs(120),
        );
        assert!(matches!(req.interaction_type, InteractionType::Selection));
        let opts = req.options.unwrap();
        assert_eq!(opts.as_array().unwrap().len(), 2);
    }

    #[test]
    fn interaction_response() {
        let resp = InteractionResponse {
            interaction_id: "req-1".into(),
            approved: Some(true),
            data: serde_json::json!({}),
            responder: Some("alice".into()),
        };
        assert_eq!(resp.approved, Some(true));
        assert_eq!(resp.responder.as_deref(), Some("alice"));
    }

    #[test]
    fn interaction_request_serialization() {
        let req = InteractionRequest::approval("id-1", "Test?", Duration::from_secs(60));
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["interaction_id"], "id-1");
        assert_eq!(json["prompt"], "Test?");
    }
}
