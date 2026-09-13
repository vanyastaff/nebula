//! Associated-type schema failures prevent factory construction.

use std::assert_matches;
use std::sync::atomic::{AtomicUsize, Ordering};

use nebula_action::{
    Action, ActionContext, ActionError, ActionMetadataAdmissionError, ActionResult,
    GenericStatelessFactory, MetadataBuildError, StatelessAction,
};
use nebula_schema::{HasSchema, ValidSchema, ValidationError, ValidationReport};
use serde::Deserialize;

static SCHEMA_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Deserialize)]
struct RejectedInput;

impl HasSchema for RejectedInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        SCHEMA_CALLS.fetch_add(1, Ordering::SeqCst);
        Err(ValidationError::builder("test.invalid_schema")
            .message("authored_private_schema_text")
            .build()
            .into())
    }
}

#[derive(Action)]
#[action(key = "test.rejected_schema", name = "Rejected schema", description = "Schema admission probe", input = RejectedInput, output = ())]
struct RejectedAction;

impl StatelessAction for RejectedAction {
    async fn execute(
        &self,
        _: RejectedInput,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<()>, ActionError> {
        Ok(ActionResult::success(()))
    }
}

#[test]
fn input_schema_failure_prevents_factory_construction() {
    SCHEMA_CALLS.store(0, Ordering::SeqCst);
    let Err(error) = GenericStatelessFactory::<RejectedAction>::new() else {
        panic!("invalid schema cannot produce a factory");
    };
    assert_matches!(
        &error,
        ActionMetadataAdmissionError::Schema(MetadataBuildError::Schema(report))
            if report.errors().next().unwrap().code() == "test.invalid_schema"
    );
    assert!(!format!("{error}: {error:?}").contains("authored_private_schema_text"));
    assert_eq!(
        SCHEMA_CALLS.load(Ordering::SeqCst),
        1,
        "factory construction must attempt the input schema exactly once"
    );
}

static OUTPUT_SCHEMA_CALLS: AtomicUsize = AtomicUsize::new(0);

struct RejectedOutput;

impl serde::Serialize for RejectedOutput {
    fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        unreachable!("a rejected output schema is never executed")
    }
}

impl HasSchema for RejectedOutput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        OUTPUT_SCHEMA_CALLS.fetch_add(1, Ordering::SeqCst);
        Err(ValidationError::builder("test.invalid_output_schema")
            .message("authored_private_output_schema_text")
            .build()
            .into())
    }
}

type UnitInput = ();

#[derive(Action)]
#[action(key = "test.rejected_output_schema", name = "Rejected output schema", description = "Output schema admission probe", input = UnitInput, output = RejectedOutput)]
struct OutputRejectedAction;

impl StatelessAction for OutputRejectedAction {
    async fn execute(
        &self,
        (): (),
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<RejectedOutput>, ActionError> {
        Ok(ActionResult::success(RejectedOutput))
    }
}

#[test]
fn output_schema_failure_prevents_factory_construction() {
    OUTPUT_SCHEMA_CALLS.store(0, Ordering::SeqCst);
    let Err(error) = GenericStatelessFactory::<OutputRejectedAction>::new() else {
        panic!("invalid output schema cannot produce a factory");
    };
    assert_matches!(
        &error,
        ActionMetadataAdmissionError::Schema(MetadataBuildError::Schema(report))
            if report.errors().next().unwrap().code() == "test.invalid_output_schema"
    );
    assert!(!format!("{error}: {error:?}").contains("authored_private_output_schema_text"));
    assert_eq!(OUTPUT_SCHEMA_CALLS.load(Ordering::SeqCst), 1);
}
