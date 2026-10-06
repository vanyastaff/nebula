//! Execution history: query-string parsing, the opaque cursor token, and the
//! port-to-wire projection.
//!
//! The cursor token is base64url (no padding) of a versioned JSON object, per
//! the API-wide cursor rule (spec §05). It carries no tenant: the scope comes
//! from the request, so a token replayed in another tenant can only page that
//! tenant's own history.

use std::str::FromStr;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, SecondsFormat, Utc};
use nebula_core::{ExecutionId, WorkflowId};
use nebula_storage_port::{
    ExecutionHistoryCursor, ExecutionHistoryPage, ExecutionHistoryPageSize, ExecutionHistoryQuery,
    ExecutionListingStatus, ExecutionStatusSet, MicrosInstant,
};
use serde::{Deserialize, Serialize};

use super::dto::{
    ExecutionHistoryParams, ExecutionStatus, ExecutionSummary, ListExecutionsResponse,
};
use crate::error::{ApiError, ValidationFieldError};

/// Translate query parameters into a store query.
///
/// `route_workflow` is the workflow a workflow-scoped route fixes; a
/// `workflow_id` parameter must then be absent or name the same workflow.
///
/// # Errors
/// [`ApiError::Validation`] naming the offending parameter, with the accepted
/// form in `expected`.
pub(crate) fn history_query(
    params: &ExecutionHistoryParams,
    route_workflow: Option<&str>,
) -> Result<ExecutionHistoryQuery, ApiError> {
    let mut query = ExecutionHistoryQuery::new()
        .with_page_size(page_size(params.limit)?)
        .with_statuses(
            params
                .status
                .as_deref()
                .map_or(Ok(ExecutionStatusSet::ALL), status_set)?,
        );

    let route_workflow = route_workflow.map(workflow_id).transpose()?;
    let filter_workflow = params.workflow_id.as_deref().map(workflow_id).transpose()?;
    match (route_workflow, filter_workflow) {
        (Some(route), Some(filter)) if route != filter => {
            return Err(invalid(
                "conflicting_workflow_id",
                "workflow_id differs from the workflow in the path",
                &route.to_string(),
                &filter.to_string(),
            ));
        },
        (route, filter) => {
            if let Some(workflow) = route.or(filter) {
                query = query.with_workflow(workflow.to_string());
            }
        },
    }

    let after = params
        .created_after
        .as_deref()
        .map(|raw| instant("created_after", raw))
        .transpose()?;
    let before = params
        .created_before
        .as_deref()
        .map(|raw| instant("created_before", raw))
        .transpose()?;
    if let (Some(after), Some(before)) = (after, before)
        && after >= before
    {
        return Err(invalid(
            "empty_time_range",
            "created_after must be earlier than created_before",
            "created_after < created_before",
            "created_after >= created_before",
        ));
    }
    if let Some(after) = after {
        query = query.with_created_after(after);
    }
    if let Some(before) = before {
        query = query.with_created_before(before);
    }

    if let Some(token) = params.cursor.as_deref() {
        query = query.with_cursor(token.parse::<CursorToken>()?.0);
    }
    Ok(query)
}

/// Project a store page onto the wire.
///
/// # Errors
/// [`ApiError::Internal`] if the next cursor cannot be encoded.
pub(crate) fn history_response(
    page: ExecutionHistoryPage,
) -> Result<ListExecutionsResponse, ApiError> {
    let next_cursor = page
        .next_cursor
        .as_ref()
        .map(CursorToken::encode)
        .transpose()?;
    Ok(ListExecutionsResponse {
        has_more: next_cursor.is_some(),
        next_cursor,
        items: page
            .items
            .into_iter()
            .map(|summary| ExecutionSummary {
                id: summary.id,
                workflow_id: summary.workflow_id,
                status: wire_status(summary.status),
                created_at: rfc3339(summary.created_at),
                started_at: summary.started_at.map(rfc3339),
                finished_at: summary.finished_at.map(rfc3339),
                updated_at: rfc3339(summary.updated_at),
            })
            .collect(),
    })
}

/// The opaque wire form of an [`ExecutionHistoryCursor`].
///
/// Parsed with [`FromStr`]; produced by [`CursorToken::encode`]. Any token
/// this endpoint did not issue — wrong version, unknown field, not an
/// execution id, oversized — is rejected with one fixed problem that never
/// echoes the token.
struct CursorToken(ExecutionHistoryCursor);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorPayload {
    v: u8,
    /// Creation instant, microseconds since the epoch.
    t: i64,
    id: String,
}

impl CursorToken {
    /// Version tag of the payload; any other version is rejected, so the
    /// encoding can change without misreading old tokens.
    const VERSION: u8 = 1;

    /// Upper bound on a token: an `exe_<ULID>` id and a timestamp fit
    /// comfortably; anything longer was not issued here.
    const MAX_LEN: usize = 256;

    fn encode(cursor: &ExecutionHistoryCursor) -> Result<String, ApiError> {
        let payload = CursorPayload {
            v: Self::VERSION,
            t: cursor.created_at().as_micros(),
            id: cursor.id().to_owned(),
        };
        let json = serde_json::to_vec(&payload)
            .map_err(|_| ApiError::Internal("execution history cursor did not encode".into()))?;
        Ok(URL_SAFE_NO_PAD.encode(json))
    }
}

impl FromStr for CursorToken {
    type Err = ApiError;

    fn from_str(token: &str) -> Result<Self, Self::Err> {
        let reject = || {
            invalid(
                "invalid_cursor",
                "cursor was not issued by this endpoint",
                "next_cursor from a previous page",
                "<opaque>",
            )
        };
        if token.len() > Self::MAX_LEN {
            return Err(reject());
        }
        let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| reject())?;
        let payload: CursorPayload = serde_json::from_slice(&bytes).map_err(|_| reject())?;
        if payload.v != Self::VERSION || ExecutionId::parse(&payload.id).is_err() {
            return Err(reject());
        }
        let created_at = MicrosInstant::from_micros(payload.t).ok_or_else(reject)?;
        Ok(Self(ExecutionHistoryCursor::new(created_at, payload.id)))
    }
}

fn workflow_id(raw: &str) -> Result<WorkflowId, ApiError> {
    WorkflowId::parse(raw).map_err(|_| {
        invalid(
            "invalid_workflow_id",
            "workflow_id is not a workflow identifier",
            "wf_<ULID>",
            raw,
        )
    })
}

/// `limit` defaults to 20, is capped at 100, and rejects 0.
fn page_size(limit: Option<u32>) -> Result<ExecutionHistoryPageSize, ApiError> {
    let Some(limit) = limit else {
        return Ok(ExecutionHistoryPageSize::default());
    };
    let capped = u8::try_from(limit).map_or(ExecutionHistoryPageSize::MAX.get(), |limit| {
        limit.min(ExecutionHistoryPageSize::MAX.get())
    });
    ExecutionHistoryPageSize::new(capped).map_err(|_| {
        invalid(
            "invalid_limit",
            "limit must be at least 1",
            "integer 1..=100",
            &limit.to_string(),
        )
    })
}

/// A comma-separated status list; an empty list means every status.
fn status_set(raw: &str) -> Result<ExecutionStatusSet, ApiError> {
    let statuses = raw
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            token.parse::<ExecutionListingStatus>().map_err(|_| {
                invalid(
                    "invalid_status",
                    "status lists an unknown execution status",
                    &ExecutionStatusSet::ALL
                        .iter()
                        .map(ExecutionListingStatus::as_str)
                        .collect::<Vec<_>>()
                        .join(","),
                    token,
                )
            })
        })
        .collect::<Result<ExecutionStatusSet, ApiError>>()?;
    Ok(if statuses.is_empty() {
        ExecutionStatusSet::ALL
    } else {
        statuses
    })
}

fn instant(parameter: &'static str, raw: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|instant| instant.with_timezone(&Utc))
        .map_err(|_| {
            invalid(
                "invalid_timestamp",
                &format!("{parameter} is not an RFC 3339 timestamp"),
                "RFC 3339, e.g. 2026-10-05T12:00:00Z",
                raw,
            )
        })
}

/// Exhaustive by design: a new listing status must get a wire spelling.
const fn wire_status(status: ExecutionListingStatus) -> ExecutionStatus {
    match status {
        ExecutionListingStatus::Created => ExecutionStatus::Created,
        ExecutionListingStatus::Running => ExecutionStatus::Running,
        ExecutionListingStatus::Paused => ExecutionStatus::Paused,
        ExecutionListingStatus::Cancelling => ExecutionStatus::Cancelling,
        ExecutionListingStatus::Completed => ExecutionStatus::Completed,
        ExecutionListingStatus::Failed => ExecutionStatus::Failed,
        ExecutionListingStatus::Cancelled => ExecutionStatus::Cancelled,
        ExecutionListingStatus::TimedOut => ExecutionStatus::TimedOut,
    }
}

fn rfc3339(instant: MicrosInstant) -> String {
    instant
        .to_datetime()
        .to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn invalid(code: &str, detail: &str, expected: &str, actual: &str) -> ApiError {
    ApiError::Validation {
        detail: detail.to_owned(),
        errors: vec![ValidationFieldError {
            code: code.to_owned(),
            detail: detail.to_owned(),
            pointer: None,
            path: None,
            expected: Some(expected.to_owned()),
            actual: Some(actual.to_owned()),
            remediation: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> ExecutionHistoryParams {
        ExecutionHistoryParams::default()
    }

    fn error_code(error: ApiError) -> String {
        match error {
            ApiError::Validation { errors, .. } => errors[0].code.clone(),
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[test]
    fn cursor_tokens_round_trip() {
        let cursor = ExecutionHistoryCursor::new(
            MicrosInstant::from_micros(1_759_665_600_123_456).unwrap(),
            ExecutionId::new().to_string(),
        );
        let token = CursorToken::encode(&cursor).unwrap();
        assert_eq!(token.parse::<CursorToken>().unwrap().0, cursor);
    }

    #[test]
    fn foreign_or_tampered_tokens_are_rejected() {
        let wrong_version =
            URL_SAFE_NO_PAD.encode(format!(r#"{{"v":2,"t":1,"id":"{}"}}"#, ExecutionId::new()));
        let not_an_execution = URL_SAFE_NO_PAD.encode(r#"{"v":1,"t":1,"id":"x"}"#);
        let extra_field = URL_SAFE_NO_PAD.encode(format!(
            r#"{{"v":1,"t":1,"id":"{}","s":"ws"}}"#,
            ExecutionId::new()
        ));
        for token in [
            "not base64!".to_owned(),
            wrong_version,
            not_an_execution,
            extra_field,
            "A".repeat(CursorToken::MAX_LEN + 1),
        ] {
            let Err(error) = token.parse::<CursorToken>() else {
                panic!("{token} must be rejected");
            };
            assert_eq!(error_code(error), "invalid_cursor");
        }
    }

    #[test]
    fn limit_defaults_caps_and_rejects_zero() {
        assert_eq!(page_size(None).unwrap().get(), 20);
        assert_eq!(page_size(Some(5000)).unwrap().get(), 100);
        assert_eq!(page_size(Some(u32::MAX)).unwrap().get(), 100);
        assert_eq!(error_code(page_size(Some(0)).unwrap_err()), "invalid_limit");
    }

    #[test]
    fn status_list_parses_names_the_bad_token_and_empty_means_all() {
        assert_eq!(
            status_set("failed, timed_out,,").unwrap(),
            ExecutionStatusSet::from([
                ExecutionListingStatus::Failed,
                ExecutionListingStatus::TimedOut
            ])
        );
        assert_eq!(status_set(" , ").unwrap(), ExecutionStatusSet::ALL);
        match status_set("failed,exploded").unwrap_err() {
            ApiError::Validation { errors, .. } => {
                assert_eq!(errors[0].code, "invalid_status");
                assert_eq!(errors[0].actual.as_deref(), Some("exploded"));
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn timestamps_must_be_rfc3339() {
        let mut bad = params();
        bad.created_after = Some("yesterday".into());
        assert_eq!(
            error_code(history_query(&bad, None).unwrap_err()),
            "invalid_timestamp"
        );

        let mut good = params();
        good.created_after = Some("2026-10-05T12:00:00+02:00".into());
        let query = history_query(&good, None).unwrap();
        assert_eq!(
            query.created_after(),
            Some(MicrosInstant::floor(
                "2026-10-05T10:00:00Z".parse().unwrap()
            ))
        );
    }

    #[test]
    fn an_empty_creation_range_is_rejected() {
        let mut range = params();
        range.created_after = Some("2026-10-05T12:00:00Z".into());
        range.created_before = Some("2026-10-05T12:00:00Z".into());
        assert_eq!(
            error_code(history_query(&range, None).unwrap_err()),
            "empty_time_range"
        );
    }

    #[test]
    fn workflow_filter_must_agree_with_the_route() {
        let route = WorkflowId::new().to_string();
        let mut same = params();
        same.workflow_id = Some(route.clone());
        assert_eq!(
            history_query(&same, Some(&route)).unwrap().workflow_id(),
            Some(route.as_str())
        );

        let mut other = params();
        other.workflow_id = Some(WorkflowId::new().to_string());
        assert_eq!(
            error_code(history_query(&other, Some(&route)).unwrap_err()),
            "conflicting_workflow_id"
        );

        let mut malformed = params();
        malformed.workflow_id = Some("not-a-workflow".into());
        assert_eq!(
            error_code(history_query(&malformed, None).unwrap_err()),
            "invalid_workflow_id"
        );
    }

    #[test]
    fn every_listing_status_has_the_same_wire_name() {
        for status in ExecutionListingStatus::ALL {
            assert_eq!(
                serde_json::to_value(wire_status(status)).unwrap(),
                serde_json::json!(status.as_str())
            );
        }
    }
}
