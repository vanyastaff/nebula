use engine::{Action, ActionMetadata, ConnectionCollection, Key, ParameterCollection, ProcessAction};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressCleanAction {
    metadata: ActionMetadata,
}

impl Action for AddressCleanAction {
    fn metadata(&self) -> &ActionMetadata {
        todo!()
    }

    fn inputs(&self) -> Option<&ConnectionCollection> {
        todo!()
    }

    fn outputs(&self) -> Option<&ConnectionCollection> {
        todo!()
    }

    fn parameters(&self) -> Option<&ParameterCollection> {
        todo!()
    }
}

pub struct AddressCleanInput {
    pub address: String,
}

pub struct AddressCleanOutput {}

#[async_trait::async_trait]
impl ProcessAction for AddressCleanAction {
    type Input = AddressCleanInput;
    type Output = AddressCleanOutput;

    async fn execute<C>(
        &self,
        context: &C,
        input: Self::Input,
    ) -> Result<engine::ActionResult<Self::Output>, engine::ActionError>
    where
        C: engine::ProcessContext + Send + Sync,
    {
        let mut request = context.create_request();
        request
            .method("POST")
            .url("https://cleaner.dadata.ru/api/v1/clean/address")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        context
            .get_credential(&Key::new("dadata_api")?)?
            .apply_to_request(&mut request)?;

        let addresses = vec![input.address];
        request.body(serde_json::to_string(&addresses)?);

        let response = request
            .build()?
            .execute_and_parse::<Vec<AddressCleanOutput>>()
            .await?;

        let output = response.into_iter().next()
            .ok_or_else(|| ActionError::Execution {
                message: "DaData returned empty response".to_string(),
            })?;

        Ok(ActionResult::Value(SerializeValue::from(output)))

    }

    fn supports_rollback(&self) -> bool {
        false
    }
}
