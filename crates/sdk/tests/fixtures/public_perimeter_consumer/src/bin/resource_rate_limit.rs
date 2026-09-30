//! An SDK-only integration declaring a rate limit and pacing its provider
//! calls through the managed call facade: one attempt per provider call, a
//! keyed cost per chat, and the provider's "slow down" reported as a verdict.

use std::{num::NonZeroU32, time::Duration};

use nebula_sdk::integration::resource::{
    Cost, Error, ErrorKind, Lease, Operation, OperationCx, OperationError, Provider, Rate, Resident,
    ResidentProvider, ResiliencePolicy, ResourceContext, ResourceKey, SentState, TeardownCx,
    Verdict, no_credential_slots, resource_key, retry_after_from_header,
};

/// The provider's client: the instance itself, no wrapper.
struct ChatClient;

#[derive(Debug)]
enum ChatError {
    RetryAfter(Duration),
}

impl ChatError {
    /// What the provider said about its limit: a per-chat slow-down.
    fn verdict(&self) -> Verdict {
        match self {
            Self::RetryAfter(after) => Verdict::KeyThrottled {
                retry_after: Some(*after),
            },
        }
    }
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

/// Sends one message: one attempt per provider call, booked on the chat's
/// own limit as well as the account's.
struct SendMessage {
    chat_id: i64,
    text: String,
}

impl Operation<ChatProvider> for SendMessage {
    type Output = u64;

    async fn run(self, cx: &mut OperationCx<'_, ChatProvider>) -> Result<u64, OperationError> {
        let attempt = cx.attempt(Cost::keyed("chat_id", self.chat_id)).await?;
        let outcome = attempt.instance().send(self.chat_id, &self.text).await;
        match outcome {
            Ok(message_id) => {
                attempt.report(Verdict::Pass).await;
                attempt.settle(SentState::Sent);
                Ok(message_id)
            },
            Err(refusal) => {
                // The provider refused and applied nothing: pause the chat,
                // settle the attempt as sent, fail retryably.
                attempt.report(refusal.verdict()).await;
                attempt.settle(SentState::Sent);
                let ChatError::RetryAfter(after) = refusal;
                Err(OperationError::new(
                    ErrorKind::Exhausted {
                        retry_after: Some(after),
                    },
                    "chat provider throttled the message",
                ))
            },
        }
    }
}

/// What action code does with a managed lease of the chat resource.
async fn send(chat: &Lease<ChatProvider>, chat_id: i64, text: &str) -> Result<u64, Error> {
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

    assert_eq!(
        ChatError::RetryAfter(Duration::from_secs(3)).verdict(),
        Verdict::KeyThrottled {
            retry_after: Some(Duration::from_secs(3))
        }
    );
    assert_eq!(retry_after_from_header("30"), Some(Duration::from_secs(30)));
    assert_eq!(Cost::keyed("chat_id", 42).permits(), 1);

    let throttled = OperationError::new(
        ErrorKind::Exhausted {
            retry_after: Some(Duration::from_secs(3)),
        },
        "chat provider throttled the message",
    );
    assert!(throttled.is_retryable(), "a throttled call applied nothing");
    assert_eq!(throttled.retry_after(), Some(Duration::from_secs(3)));
}
