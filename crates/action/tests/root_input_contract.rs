//! Typed action catalogs and dispatch preserve the declared root wire shape.

use std::{
    assert_matches,
    error::Error,
    marker::PhantomData,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionHandle, ActionResult, InstanceFactory,
    StatelessAction, TestContextBuilder,
};
use nebula_core::{Dependencies, action_key, node_key};
use nebula_schema::{AuthoredValue, HasSchema, ValidSchema, ValidationReport};
use nebula_workflow::NodeDefinition;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

struct RootEcho<T> {
    calls: Arc<AtomicUsize>,
    marker: PhantomData<fn() -> T>,
}

impl<T> Action for RootEcho<T>
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    type Input = T;
    type Output = T;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("test.root_echo"),
            nebula_action::metadata_name!("Root echo"),
            "Echo the declared root input without coercion",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl<T> StatelessAction for RootEcho<T>
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    async fn execute(
        &self,
        input: T,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<T>, ActionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

fn factory<T>() -> (InstanceFactory<RootEcho<T>>, Arc<AtomicUsize>)
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    let calls = Arc::new(AtomicUsize::new(0));
    let factory = InstanceFactory::new(
        RootEcho::<T>::metadata(),
        RootEcho {
            calls: Arc::clone(&calls),
            marker: PhantomData,
        },
    )
    .unwrap();
    (factory, calls)
}

async fn dispatch<T>(
    factory: &InstanceFactory<RootEcho<T>>,
    wire: Value,
) -> Result<ActionResult<Value>, ActionError>
where
    T: HasSchema + DeserializeOwned + Serialize + Send + Sync + 'static,
{
    let context = TestContextBuilder::new().build();
    let node = NodeDefinition::new(node_key!("echo"), "Echo", "test", "root_echo").unwrap();
    let ActionHandle::Stateless(handle) = factory.instantiate(&node, &context).await? else {
        panic!("stateless factory must produce a stateless handle");
    };
    let input = handle.prepare_input(nebula_action::ActionInput::Raw(wire))?;
    handle.dispatch(input, &context).await
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, nebula_schema::Schema)]
struct UnitInput;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, nebula_schema::Schema)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde object-wire fixture must remain distinct from unit null"
)]
struct EmptyRecordInput {}

fn assert_catalog_roundtrip<T>(input: &T, rejected: Value)
where
    T: HasSchema
        + DeserializeOwned
        + Serialize
        + Send
        + Sync
        + PartialEq
        + std::fmt::Debug
        + 'static,
{
    let (factory, _) = factory::<T>();
    let metadata = factory.metadata();
    let schema = metadata.base().schema();
    let wire = serde_json::to_value(input).unwrap();
    let resolved = schema
        .validate(AuthoredValue::from_data(wire).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(&resolved.into_typed::<T>().unwrap(), input);
    let report = schema
        .validate(AuthoredValue::from_data(rejected).unwrap())
        .unwrap_err();
    assert!(
        report
            .errors()
            .any(|error| { error.code() == "type_mismatch" && error.path().as_str().is_empty() })
    );
}

#[test]
fn unit_catalog_uses_null_not_empty_record() {
    assert_catalog_roundtrip(&(), json!({}));
    assert_catalog_roundtrip(&UnitInput, json!({}));
}

#[test]
fn empty_braced_catalog_uses_object_not_null() {
    assert_catalog_roundtrip(&EmptyRecordInput {}, Value::Null);
}

#[test]
fn scalar_catalogs_reject_other_root_kinds() {
    assert_catalog_roundtrip(&"hello".to_owned(), json!(42));
    assert_catalog_roundtrip(&true, json!("true"));
    assert_catalog_roundtrip(&42_i64, json!(false));
}

#[tokio::test]
async fn direct_unit_dispatch_rejects_supplied_objects_before_execute() {
    let (factory, calls) = factory::<()>();
    for wire in [json!({}), json!({"unused": "supplied-value"})] {
        let error = dispatch(&factory, wire).await.unwrap_err();
        assert_matches!(error, ActionError::Validation { field: "input", .. });
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let output = dispatch(&factory, Value::Null).await.unwrap();
    let ActionResult::Success { output } = output else {
        panic!("successful unit dispatch must retain its null output");
    };
    assert_eq!(output.into_value(), Some(Value::Null));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn direct_scalar_dispatch_preserves_literal_template_looking_strings() {
    let (factory, calls) = factory::<String>();
    let wire = json!("{{ $input.keep_this_as_data }}");
    let output = dispatch(&factory, wire.clone()).await.unwrap();
    let ActionResult::Success { output } = output else {
        panic!("successful scalar dispatch must retain its string output");
    };
    assert_eq!(output.into_value(), Some(wire));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let error = dispatch(&factory, json!({"value": "supplied"}))
        .await
        .unwrap_err();
    assert_matches!(error, ActionError::Validation { field: "input", .. });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn direct_integer_dispatch_uses_schema_normalization_and_range() {
    let (factory, calls) = factory::<i64>();
    let ActionResult::Success { output } = dispatch(&factory, json!(1.0)).await.unwrap() else {
        panic!("an integral JSON number must dispatch as an integer");
    };
    assert_eq!(output.into_value(), Some(json!(1)));
    for wire in [json!(1.5), json!(u64::MAX), json!("1")] {
        let error = dispatch(&factory, wire).await.unwrap_err();
        assert_matches!(error, ActionError::Validation { field: "input", .. });
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[derive(Debug, Serialize)]
struct RejectingInput;

impl HasSchema for RejectingInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        nebula_schema::schema_of::<String>()
    }
}

impl<'de> Deserialize<'de> for RejectingInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let supplied = String::deserialize(deserializer)?;
        Err(serde::de::Error::custom(format!("rejected {supplied}")))
    }
}

#[tokio::test]
async fn direct_dispatch_seals_custom_serde_error_sources() {
    const CANARY: &str = "ACTION_INPUT_SERDE_SECRET_CANARY";
    let (factory, calls) = factory::<RejectingInput>();
    let error = dispatch(&factory, json!(CANARY)).await.unwrap_err();
    assert_matches!(error, ActionError::Validation { field: "input", .. });
    let mut cause: &dyn Error = &error;
    loop {
        assert!(!format!("{cause} {cause:?}").contains(CANARY));
        assert!(cause.downcast_ref::<serde_json::Error>().is_none());
        match cause.source() {
            Some(next) => cause = next,
            None => break,
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
