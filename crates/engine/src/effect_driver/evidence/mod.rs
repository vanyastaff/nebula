//! Exact bounded evidence; only explicitly supported outputs can be replayed.

use std::io::{self, Write};

use nebula_action::{ActionOutput, ActionResult, effect::EffectFailureCode};
use nebula_core::OperationId;
use nebula_storage_port::dto::{FrozenOutcomeEvidence, KnownOutcome, OutcomeEvidenceSource};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::EffectExecutionError;

const MAX_EVIDENCE_BYTES: usize = 1_048_576;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum RecordedOutcome {
    Output {
        operation_id: [u8; 16],
        result: Box<ActionResult<Value>>,
    },
    OutputUnavailable {
        operation_id: [u8; 16],
    },
    Rejected {
        operation_id: [u8; 16],
        code: EffectFailureCode,
    },
}

struct BoundedBytes(Vec<u8>);

impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_EVIDENCE_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("effect evidence limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn supported_result(result: &ActionResult<Value>) -> bool {
    matches!(
        result,
        ActionResult::Success {
            output: ActionOutput::Value(_)
        }
    )
}

#[cfg(test)]
mod tests;

fn encode(outcome: &RecordedOutcome) -> Result<Vec<u8>, EffectExecutionError> {
    let mut bytes = BoundedBytes(Vec::new());
    serde_json::to_writer(&mut bytes, outcome)
        .map_err(|_| EffectExecutionError::InvalidEvidence)?;
    Ok(bytes.0)
}

pub(super) fn applied(
    operation_id: OperationId,
    source: OutcomeEvidenceSource,
    output: Option<Box<ActionResult<Value>>>,
) -> Result<FrozenOutcomeEvidence, EffectExecutionError> {
    let outcome = match output {
        Some(result) if supported_result(&result) => RecordedOutcome::Output {
            operation_id: *operation_id.as_bytes(),
            result,
        },
        _ => RecordedOutcome::OutputUnavailable {
            operation_id: *operation_id.as_bytes(),
        },
    };
    // A known applied effect never becomes retryable merely because its output
    // is oversized or unsupported. The bounded terminal fact remains durable.
    let payload = encode(&outcome).or_else(|_| {
        encode(&RecordedOutcome::OutputUnavailable {
            operation_id: *operation_id.as_bytes(),
        })
    })?;
    FrozenOutcomeEvidence::v1_json(source, KnownOutcome::Succeeded, payload).map_err(Into::into)
}

pub(super) fn rejected(
    operation_id: OperationId,
    source: OutcomeEvidenceSource,
    code: EffectFailureCode,
) -> Result<FrozenOutcomeEvidence, EffectExecutionError> {
    FrozenOutcomeEvidence::v1_json(
        source,
        KnownOutcome::Failed,
        encode(&RecordedOutcome::Rejected {
            operation_id: *operation_id.as_bytes(),
            code,
        })?,
    )
    .map_err(Into::into)
}

pub(super) fn replay(
    operation_id: OperationId,
    evidence: &FrozenOutcomeEvidence,
) -> Result<ActionResult<Value>, EffectExecutionError> {
    evidence
        .validate()
        .map_err(|_| EffectExecutionError::InvalidEvidence)?;
    let outcome: RecordedOutcome = serde_json::from_slice(evidence.payload())
        .map_err(|_| EffectExecutionError::InvalidEvidence)?;
    match (evidence.outcome(), outcome) {
        (
            KnownOutcome::Succeeded,
            RecordedOutcome::Output {
                operation_id: recorded_operation_id,
                result,
            },
        ) if &recorded_operation_id == operation_id.as_bytes() && supported_result(&result) => {
            Ok(*result)
        },
        (
            KnownOutcome::Succeeded,
            RecordedOutcome::OutputUnavailable {
                operation_id: recorded_operation_id,
            },
        ) if &recorded_operation_id == operation_id.as_bytes() => {
            Err(EffectExecutionError::OutputUnavailable { operation_id })
        },
        (
            KnownOutcome::Failed,
            RecordedOutcome::Rejected {
                operation_id: recorded_operation_id,
                code,
            },
        ) if &recorded_operation_id == operation_id.as_bytes() => {
            Err(EffectExecutionError::Rejected { operation_id, code })
        },
        _ => Err(EffectExecutionError::InvalidEvidence),
    }
}
