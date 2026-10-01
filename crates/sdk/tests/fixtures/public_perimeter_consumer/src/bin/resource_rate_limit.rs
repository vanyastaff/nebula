//! An SDK-only integration declaring a rate limit and pacing its provider
//! calls through the managed call facade: one call per provider request, a
//! keyed cost per chat, and the provider's "slow down" classified as a
//! per-key throttle.

use std::{num::NonZeroU32, time::Duration};

use nebula_sdk::integration::resource::{
    Cost, Error, ErrorKind, Operation, OperationCx, OperationError, Provider, Rate, Resident,
    ResidentProvider, ResiliencePolicy, ResourceContext, ResourceHandle, ResourceKey, TeardownCx,
    no_credential_slots, resource_key, retry_after_from_header,
};
use nebula_sdk::prelude::{Deserialize, Serialize};

/// The provider's client: the instance itself, no wrapper.
struct ChatClient;

#[derive(Debug)]
enum ChatError {
    RetryAfter(Duration),
}

/// Classifies the client's answer once: a per-chat slow-down pauses only
/// that chat's key, and the provider applied nothing.
fn classify(outcome: Result<u64, ChatError>) -> Result<u64, OperationError> {
    outcome.map_err(|ChatError::RetryAfter(after)| OperationError::throttled_key(Some(after)))
}

impl ChatClient {
    async fn send(&self, chat_id: i64, text: &str) -> Result<u64, ChatError> {
        std::future::ready(()).await;
        if text.is_empty() {
            return Err(ChatError::RetryAfter(Duration::from_secs(1)));
        }
        Ok(chat_id.unsigned_abs())
    }
}

struct ChatProvider;
no_credential_slots!(ChatProvider);

#[async_trait::async_trait]
impl Provider for ChatProvider {
    type Config = ();
    type Instance = ChatClient;
    type Topology = Resident<Self>;

    fn metadata() -> nebula_sdk::integration::resource::ResourceMetadataDraft {
        nebula_sdk::integration::resource::ResourceMetadataDraft::new(
            Self::key(),
            nebula_sdk::prelude::metadata_name!("ChatProvider"),
            "",
        )
    }

    fn key() -> ResourceKey {
        resource_key!("example.rate-limited-chat")
    }

    fn resilience() -> ResiliencePolicy {
        ResiliencePolicy::new()
            .rate(Rate::per_second(NonZeroU32::MIN.saturating_add(29)))
            .keyed("chat_id", Rate::per_second(NonZeroU32::MIN))
    }

    async fn create(&self, _: &(), _: &ResourceContext) -> Result<Self::Instance, Error> {
        Ok(ChatClient)
    }

    async fn destroy(&self, _: Self::Instance, _: TeardownCx) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for ChatProvider {}

/// Sends one message: one call, booked on the chat's own limit as well as
/// the account's.
#[derive(Serialize, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
struct SendMessage {
    chat_id: i64,
    text: String,
}

impl Operation<ChatProvider> for SendMessage {
    type Output = u64;
    const KEY: &'static str = "chat.send_message";

    async fn run(self, cx: &mut OperationCx<'_, ChatProvider>) -> Result<u64, OperationError> {
        let Self { chat_id, text } = self;
        cx.call(Cost::keyed("chat_id", chat_id), async move |client, ()| {
            classify(client.send(chat_id, &text).await)
        })
        .await
    }
}

/// What action code does with the chat resource's handle.
async fn send(chat: &ResourceHandle<ChatProvider>, chat_id: i64, text: &str) -> Result<u64, Error> {
    Ok(chat
        .submit(SendMessage {
            chat_id,
            text: text.to_owned(),
        })
        .await?)
}

fn main() {
    let _action_code = send;
    let policy = <ChatProvider as Provider>::resilience();
    assert_eq!(policy.keyed_rates().len(), 1, "per-chat limit declared");

    assert_eq!(retry_after_from_header("30"), Some(Duration::from_secs(30)));
    assert_eq!(Cost::keyed("chat_id", 42).permits(), 1);

    assert_eq!(classify(Ok(7)).ok(), Some(7));
    let throttled = classify(Err(ChatError::RetryAfter(Duration::from_secs(3))))
        .expect_err("a slow-down is an error");
    assert_eq!(
        *throttled.kind(),
        ErrorKind::Exhausted {
            retry_after: Some(Duration::from_secs(3))
        }
    );
    assert!(throttled.is_retryable(), "a throttled call applied nothing");
    assert_eq!(throttled.retry_after(), Some(Duration::from_secs(3)));
}
