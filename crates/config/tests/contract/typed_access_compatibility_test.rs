use super::helpers::fixture_path;
use nebula_config::ConfigBuilder;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct PathContractFixture {
    data: Value,
    typed_access: Vec<TypedAccessCase>,
}

#[derive(Debug, Deserialize)]
struct TypedAccessCase {
    path: String,
    kind: String,
    expected: Value,
}

#[tokio::test]
async fn typed_access_contract_fixture_is_compatible() {
    let fixture_raw = std::fs::read_to_string(fixture_path("compat/path_contract_v1.json"))
        .expect("path fixture should exist");
    let fixture: PathContractFixture =
        serde_json::from_str(&fixture_raw).expect("path fixture must be valid");

    let config = ConfigBuilder::new()
        .with_defaults(fixture.data)
        .build()
        .await
        .expect("config should build from fixture data");

    for case in fixture.typed_access {
        match case.kind.as_str() {
            "u16" => {
                let actual: u16 = config
                    .get(&case.path)
                    .await
                    .expect("u16 access should succeed");
                assert_eq!(serde_json::json!(actual), case.expected);
            }
            "string" => {
                let actual: String = config
                    .get(&case.path)
                    .await
                    .expect("string access should succeed");
                assert_eq!(serde_json::json!(actual), case.expected);
            }
            "bool" => {
                let actual: bool = config
                    .get(&case.path)
                    .await
                    .expect("bool access should succeed");
                assert_eq!(serde_json::json!(actual), case.expected);
            }
            other => panic!("unsupported fixture kind: {other}"),
        }
    }
}
