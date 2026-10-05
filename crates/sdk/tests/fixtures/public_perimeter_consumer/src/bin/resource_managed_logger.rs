//! An SDK-only logger integration authored against the managed call facade:
//! a resource, its operations, and the action-side code that submits them,
//! all through `nebula_sdk::integration::resource`.

use std::{
    num::NonZeroU32,
    sync::{Mutex, PoisonError},
};

use nebula_sdk::integration::resource::{
    Cost, Effect, Error, ErrorKind, Operation, OperationCx, OperationError, PinSlots, Provider,
    Resident, ResidentProvider, ResourceContext, ResourceHandle, ResourceKey,
    ResourceMetadataDraft, SentState, TeardownCx, no_credential_slots, resource_key,
};
use nebula_sdk::prelude::{Deserialize, Serialize};

/// The logger's instance: an in-memory sink with a bounded buffer.
struct LogSink {
    lines: Mutex<Vec<String>>,
    capacity: usize,
}

impl LogSink {
    fn enqueue(&self, line: String) -> Option<usize> {
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.len() >= self.capacity {
            return None;
        }
        lines.push(line);
        Some(lines.len())
    }

    fn written(&self) -> usize {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

struct Logger;
no_credential_slots!(Logger);

#[async_trait::async_trait]
impl Provider for Logger {
    type Config = ();
    type Instance = LogSink;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.managed-logger")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_sdk::prelude::metadata_name!("Logger"),
            "",
        )
    }

    async fn create(&self, _: &(), _: &ResourceContext) -> Result<LogSink, Error> {
        Ok(LogSink {
            lines: Mutex::new(Vec::new()),
            capacity: 1024,
        })
    }

    async fn destroy(&self, _: LogSink, _: TeardownCx) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for Logger {}

/// A line accepted by the sink.
#[derive(Serialize, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
struct Enqueued(usize);

/// Appends one line; appending twice writes it twice.
#[derive(Serialize, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
struct Write {
    line: String,
}

impl Operation<Logger> for Write {
    type Output = Enqueued;
    const KEY: &'static str = "logger.write";

    async fn run(self, cx: &mut OperationCx<'_, Logger>) -> Result<Enqueued, OperationError> {
        let line = self.line;
        cx.call(Cost::FREE, async move |sink, ()| {
            sink.enqueue(line.clone()).map(Enqueued).ok_or_else(|| {
                OperationError::unreachable_as(ErrorKind::Backpressure, "log buffer full")
            })
        })
        .await
    }
}

/// Reports how many lines are written; safe to repeat.
#[derive(Serialize, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
struct Flush;

impl Operation<Logger> for Flush {
    type Output = usize;
    const KEY: &'static str = "logger.flush";
    const EFFECT: Effect = Effect::Idempotent;

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::MIN.saturating_add(1)
    }

    async fn run(self, cx: &mut OperationCx<'_, Logger>) -> Result<usize, OperationError> {
        cx.call(Cost::FREE, async |sink, ()| Ok(sink.written()))
            .await
    }
}

/// What action code does with its resource handle.
async fn log_and_flush(logger: &ResourceHandle<Logger>) -> Result<usize, Error> {
    let Enqueued(_seq) = logger
        .submit(Write {
            line: "hello".to_owned(),
        })
        .await?;
    Ok(logger.submit(Flush).await?)
}

fn main() {
    let _action_code = log_and_flush;
    let (): <Logger as PinSlots>::Pinned = Logger.pin_slots();
    assert_eq!(<Write as Operation<Logger>>::EFFECT, Effect::Write);
    assert!(<Flush as Operation<Logger>>::EFFECT.is_replay_safe());
    assert_eq!(Cost::FREE.permits(), 0);
    let refused = OperationError::unreachable_as(ErrorKind::Backpressure, "log buffer full");
    assert!(refused.is_retryable(), "nothing was sent");
    assert_eq!(refused.sent(), SentState::NotSent);
}
