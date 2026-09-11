//! Trusted typed action ingress with explicit raw data and resolved proof phases.

use std::{any::Any, fmt, sync::Arc};

use nebula_schema::{ResolvedValues, ValidSchema, ValidationReport};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{ActionError, ValidationReason};

/// Input at an erased action boundary, retaining schema resolution provenance.
///
/// Raw JSON is literal serde wire data. Resolved values retain their schema and
/// must match the receiving action's complete declared input contract.
#[derive(Clone)]
pub enum ActionInput {
    /// Unprepared literal serde wire data, never expression shorthand.
    Raw(Value),
    /// Schema-bound resolved data; no preparation is repeated at dispatch.
    Resolved(ResolvedValues),
}

impl fmt::Debug for ActionInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Raw(_) => "ActionInput::Raw(..)",
            Self::Resolved(_) => "ActionInput::Resolved(..)",
        })
    }
}

impl ActionInput {
    /// Prepare raw data or verify an existing proof against the complete schema.
    ///
    /// # Errors
    /// Returns a redacted validation error for invalid raw input or a proof bound
    /// to a different schema. A matching proof is returned unchanged.
    ///
    /// ```
    /// use nebula_action::ActionInput;
    /// use nebula_schema::schema_of;
    ///
    /// let schema = schema_of::<i64>()?;
    /// let values = ActionInput::Raw(serde_json::json!(1.0)).into_resolved(&schema)?;
    /// assert_eq!(values.into_typed_exposing_secrets::<i64>()?, 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[tracing::instrument(name = "action.input.resolve", skip_all, err)]
    pub fn into_resolved(self, schema: &ValidSchema) -> Result<ResolvedValues, ActionError> {
        match self {
            Self::Raw(input) => prepare_values(schema, input),
            Self::Resolved(values) if values.schema().ptr_eq(schema) => Ok(values),
            Self::Resolved(_) => Err(ActionError::validation(
                "input",
                ValidationReason::Other,
                Some("resolved input schema does not match the declared input schema"),
            )),
        }
    }
}

struct ActionInputContractIdentity;

pub(crate) struct ActionInputContract {
    identity: Arc<ActionInputContractIdentity>,
    schema: ValidSchema,
}

impl ActionInputContract {
    pub(crate) fn new(schema: &ValidSchema) -> Self {
        Self {
            identity: Arc::new(ActionInputContractIdentity),
            schema: schema.clone(),
        }
    }

    #[tracing::instrument(name = "action.input.prepare", skip_all, err)]
    pub(crate) fn prepare<T: DeserializeOwned + Send + Sync + 'static>(
        &self,
        input: ActionInput,
    ) -> Result<PreparedActionInput, ActionError> {
        let resolved = input.into_resolved(&self.schema)?;
        let typed = decode_input::<T>(resolved.clone())?;
        Ok(PreparedActionInput {
            contract: Arc::clone(&self.identity),
            resolved,
            typed: Box::new(typed),
        })
    }
}

/// Opaque input admitted against one handle's complete Rust contract.
///
/// Schema validation alone is not sufficient evidence that the declared
/// [`Action::Input`](crate::Action::Input) can be decoded. This token is created
/// only after both schema preparation and typed deserialization succeed. It is
/// bound to the exact receiving handle, even when another handle uses the same
/// Rust input type and schema. The schema-bound resolved proof is retained for
/// durable handoff without re-running authored transforms. Its payload is
/// deliberately inaccessible outside this crate; only the owning adapter may
/// recover it.
pub struct PreparedActionInput {
    contract: Arc<ActionInputContractIdentity>,
    resolved: ResolvedValues,
    typed: Box<dyn Any + Send + Sync>,
}

impl PreparedActionInput {
    /// Schema snapshot carried by this prepared proof.
    #[must_use]
    pub const fn schema(&self) -> &ValidSchema {
        self.resolved.schema()
    }

    pub(crate) fn into_typed<T: 'static>(
        self,
        contract: &ActionInputContract,
    ) -> Result<T, ActionError> {
        self.ensure_contract(contract)?;
        self.typed
            .downcast::<T>()
            .map(|input| *input)
            .map_err(|_| ActionError::fatal("prepared input belongs to another action contract"))
    }

    pub(crate) fn typed_ref<T: 'static>(
        &self,
        contract: &ActionInputContract,
    ) -> Result<&T, ActionError> {
        self.ensure_contract(contract)?;
        self.typed
            .downcast_ref::<T>()
            .ok_or_else(|| ActionError::fatal("prepared input belongs to another action contract"))
    }

    fn ensure_contract(&self, contract: &ActionInputContract) -> Result<(), ActionError> {
        if Arc::ptr_eq(&self.contract, &contract.identity)
            && self.resolved.schema().ptr_eq(&contract.schema)
        {
            Ok(())
        } else {
            Err(ActionError::fatal(
                "prepared input belongs to another action contract",
            ))
        }
    }
}

impl fmt::Debug for PreparedActionInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedActionInput")
            .finish_non_exhaustive()
    }
}

fn prepare_values(schema: &ValidSchema, input: Value) -> Result<ResolvedValues, ActionError> {
    let values = schema.values_from_wire(input).map_err(|_| {
        ActionError::validation(
            "input",
            ValidationReason::WrongType,
            Some("input wire shape is invalid"),
        )
    })?;
    let prepared = schema.validate(values).map_err(|report| {
        ActionError::validation(
            "input",
            validation_reason(&report),
            Some("input schema validation failed"),
        )
    })?;
    prepared.resolve_data().map_err(|_| {
        ActionError::validation(
            "input",
            ValidationReason::Other,
            Some("input data resolution failed"),
        )
    })
}

fn validation_reason(report: &ValidationReport) -> ValidationReason {
    if report.errors().any(|error| error.code() == "required") {
        ValidationReason::MissingField
    } else if report
        .errors()
        .any(|error| matches!(error.code(), "type_mismatch" | "expression.type_mismatch"))
    {
        ValidationReason::WrongType
    } else {
        ValidationReason::Other
    }
}

fn decode_input<T: DeserializeOwned>(resolved: ResolvedValues) -> Result<T, ActionError> {
    // Action implementations are trusted consumers of their declared secret fields.
    // Neither schema diagnostics nor custom serde errors cross this public boundary.
    resolved.into_typed_exposing_secrets().map_err(|_| {
        ActionError::validation(
            "input",
            ValidationReason::WrongType,
            Some("prepared input cannot be decoded as the declared type"),
        )
    })
}
