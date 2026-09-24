//! An SDK-only integration declaring a rate limit and pacing its client.

use std::{num::NonZeroU32, time::Duration};

use nebula_sdk::integration::resource::{
    Error, Limited, LimitedError, Provider, Rate, Resident, ResidentProvider, ResiliencePolicy,
    ResourceContext, ResourceKey, TeardownCx, Throttle, Verdict, no_credential_slots, on_error,
    resource_key, retry_after_from_header,
};

#[derive(Clone)]
struct ChatClient;

#[derive(Debug)]
enum ChatError {
    RetryAfter(Duration),
}

struct ChatProvider;
no_credential_slots!(ChatProvider);

#[async_trait::async_trait]
impl Provider for ChatProvider {
    type Config = ();
    type Instance = Limited<ChatClient, ChatThrottle>;
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

    async fn create(&self, _: &(), ctx: &ResourceContext) -> Result<Self::Instance, Error> {
        Ok(ctx.limits().wrap(ChatClient, ChatThrottle))
    }

    async fn destroy(&self, _: Self::Instance, _: TeardownCx) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for ChatProvider {}

#[derive(Clone)]
struct ChatThrottle;

impl<T> Throttle<T, ChatError> for ChatThrottle {
    fn check(&self, outcome: &Result<T, ChatError>) -> Verdict {
        match outcome {
            Err(ChatError::RetryAfter(after)) => Verdict::KeyThrottled {
                retry_after: Some(*after),
            },
            Ok(_) => Verdict::Pass,
        }
    }
}

fn main() {
    let _provider = ChatProvider;
    let policy = <ChatProvider as Provider>::resilience();
    assert_eq!(policy.keyed_rates().len(), 1, "per-chat limit declared");

    let refused: Result<(), ChatError> = Err(ChatError::RetryAfter(Duration::from_secs(3)));
    assert_eq!(
        ChatThrottle.check(&refused),
        Verdict::KeyThrottled {
            retry_after: Some(Duration::from_secs(3))
        }
    );
    let error_only = on_error(|error: &ChatError| match error {
        ChatError::RetryAfter(after) => Verdict::Throttled {
            retry_after: Some(*after),
        },
    });
    assert!(matches!(
        Throttle::<(), ChatError>::check(&error_only, &refused),
        Verdict::Throttled { .. }
    ));
    assert_eq!(retry_after_from_header("30"), Some(Duration::from_secs(30)));

    let _client = ChatClient;
    let unlimited: Option<LimitedError<ChatError>> = None;
    assert!(unlimited.is_none());
}
