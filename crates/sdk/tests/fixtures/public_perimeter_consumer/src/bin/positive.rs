use nebula_sdk::{
    integration::credential::{TestFailureCode, TestResult},
    prelude::{ActionBuilder, WorkflowBuilder, action_key},
};

fn main() {
    let metadata = ActionBuilder::new(action_key!("example.perimeter"), "Perimeter action")
        .with_description("Uses only the supported SDK authoring surface")
        .build();
    let workflow = WorkflowBuilder::new("public_perimeter")
        .add_node("invoke", "example", "perimeter")
        .build()
        .expect("the supported builder must accept one valid node");
    let result = TestResult::Failed {
        code: TestFailureCode::AuthenticationRejected,
    };

    assert_eq!(metadata.base.name, "Perimeter action");
    assert_eq!(workflow.nodes.len(), 1);
    assert_eq!(
        result.failure_code(),
        Some(TestFailureCode::AuthenticationRejected)
    );
}
