//! One exchange per granted attempt: credentials last, the request, the
//! classified answer, the settled attempt — and the unit's attempt loop.

use std::time::{Duration, Instant};

use bytes::BytesMut;
use http::{HeaderMap, StatusCode, header::RETRY_AFTER};
use nebula_resource::{
    ErrorKind,
    call::{Attempt, Effect, OpCx, OpError, Operation, SentState},
    rate_limit::{Verdict, retry_after_from_header},
};
use tracing::Instrument as _;

use super::{
    auth::{Authorize, HttpApi},
    config::HttpTransport,
    request::{Method, Request},
    response::Response,
};

/// What a response head says, before any body is read.
pub(super) enum Head {
    /// An answer to hand back: `2xx`, `3xx`, or an accepted status.
    Answer,
    /// The provider asked to slow down (`429`, or `503` with `Retry-After`).
    Throttled(Option<Duration>),
    /// A failure worth another attempt (`408`, `425`, `5xx`).
    Transient(&'static str),
    /// A failure another attempt repeats (other `4xx`).
    Permanent(&'static str),
}

/// Classifies a response head. Redirects are answers: following one would
/// be a new unit, so the caller decides.
pub(super) fn classify(status: StatusCode, headers: &HeaderMap, accepted: bool) -> Head {
    if accepted || status.is_success() || status.is_redirection() {
        return Head::Answer;
    }
    let retry_after = headers
        .get(RETRY_AFTER)
        .map(|value| value.to_str().ok().and_then(retry_after_from_header));
    match status.as_u16() {
        429 => Head::Throttled(retry_after.flatten()),
        503 => match retry_after {
            Some(after) => Head::Throttled(after),
            None => Head::Transient("service unavailable"),
        },
        408 | 425 | 500..=599 => Head::Transient(
            status
                .canonical_reason()
                .unwrap_or("provider failed the request"),
        ),
        400..=499 => Head::Permanent(
            status
                .canonical_reason()
                .unwrap_or("provider refused the request"),
        ),
        _ => Head::Permanent("unexpected response status"),
    }
}

/// A failed attempt: its error and what it settled.
pub(super) struct Failure {
    pub(super) error: OpError,
    pub(super) sent: SentState,
}

/// Settles `attempt` and records the state on the exchange span.
pub(super) fn settle<R>(attempt: Attempt<'_, R>, sent: SentState, span: &tracing::Span)
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
{
    span.record("sent", sent.as_str());
    attempt.settle(sent);
}

fn fail<R>(
    attempt: Attempt<'_, R>,
    sent: SentState,
    error: OpError,
    span: &tracing::Span,
) -> Failure
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
{
    settle(attempt, sent, span);
    Failure { error, sent }
}

/// The exchange span: method, attempt number, status and sent state — never
/// a URL or a header value.
pub(super) fn exchange_span(method: &'static str, index: Option<u32>) -> tracing::Span {
    tracing::info_span!(
        "nebula.sdk.http.attempt",
        method,
        attempt = index,
        status = tracing::field::Empty,
        sent = tracing::field::Empty,
    )
}

/// The request a granted attempt sends: built without credentials, then
/// the pinned credentials applied last, bounded by the transport's request
/// timeout and the unit's deadline.
pub(super) fn prepare<R, M>(
    attempt: &Attempt<'_, R>,
    transport: &HttpTransport,
    request: &Request<M>,
    timeout: Option<Duration>,
) -> Result<reqwest::Request, OpError>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    let mut outgoing = request.outgoing(transport)?;
    let mut credentials = HeaderMap::new();
    R::authorize(attempt.slots(), &mut Authorize::new(&mut credentials))?;
    outgoing.headers_mut().extend(credentials);
    *outgoing.timeout_mut() = timeout;
    Ok(outgoing)
}

/// Sends `request`, failing the attempt `NotSent` when the connection was
/// never established and `MaybeSent` when it failed after.
pub(super) async fn execute<'a, R>(
    attempt: Attempt<'a, R>,
    transport: &HttpTransport,
    outgoing: reqwest::Request,
    span: &tracing::Span,
) -> Result<(Attempt<'a, R>, reqwest::Response), Failure>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
{
    // The error is never formatted: its `Display` carries the URL.
    match transport.client().execute(outgoing).await {
        Ok(response) => {
            span.record("status", response.status().as_u16());
            Ok((attempt, response))
        },
        Err(error) if error.is_connect() => Err(fail(
            attempt,
            SentState::NotSent,
            OpError::new(
                ErrorKind::Transient,
                "could not connect; the request was not sent",
            ),
            span,
        )),
        Err(_) => Err(fail(
            attempt,
            SentState::MaybeSent,
            OpError::new(
                ErrorKind::Transient,
                "the request failed before a response arrived",
            ),
            span,
        )),
    }
}

/// Reports the head to the rate limit and fails the attempt unless it is an
/// answer.
pub(super) async fn admit_head<'a, R>(
    attempt: Attempt<'a, R>,
    response: &reqwest::Response,
    accepted: bool,
    span: &tracing::Span,
) -> Result<Attempt<'a, R>, Failure>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
{
    let (verdict, error) = match classify(response.status(), response.headers(), accepted) {
        Head::Answer => (Verdict::Pass, None),
        Head::Throttled(retry_after) => (
            Verdict::Throttled { retry_after },
            Some(OpError::new(
                ErrorKind::Exhausted { retry_after },
                "provider throttled the request",
            )),
        ),
        Head::Transient(detail) => (
            Verdict::Pass,
            Some(OpError::new(ErrorKind::Transient, detail)),
        ),
        Head::Permanent(detail) => (
            Verdict::Pass,
            Some(OpError::new(ErrorKind::Permanent, detail)),
        ),
    };
    attempt.report(verdict).await;
    match error {
        None => Ok(attempt),
        Some(error) => Err(fail(attempt, SentState::Sent, error, span)),
    }
}

/// One granted attempt, end to end.
async fn exchange<R, M>(
    attempt: Attempt<'_, R>,
    request: &Request<M>,
    deadline: Option<Instant>,
    span: &tracing::Span,
) -> Result<Response, Failure>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    let transport = attempt.instance().as_ref().clone();
    let limits = transport.limits();
    let timeout = deadline.map_or(limits.request_timeout, |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .min(limits.request_timeout)
    });
    let outgoing = match prepare(&attempt, &transport, request, Some(timeout)) {
        Ok(outgoing) => outgoing,
        Err(error) => return Err(fail(attempt, SentState::NotSent, error, span)),
    };
    let (attempt, mut response) = execute(attempt, &transport, outgoing, span).await?;
    let status = response.status();
    let attempt = admit_head(attempt, &response, request.accepts(status), span).await?;

    let budget = request.body_budget(limits.max_response_bytes);
    if response
        .content_length()
        .is_some_and(|length| length > budget)
    {
        return Err(fail(
            attempt,
            SentState::Sent,
            OpError::new(
                ErrorKind::Permanent,
                "response body exceeds its byte budget",
            ),
            span,
        ));
    }
    let mut body = BytesMut::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let room = budget.saturating_sub(body.len() as u64);
                if chunk.len() as u64 > room {
                    return Err(fail(
                        attempt,
                        SentState::Sent,
                        OpError::new(
                            ErrorKind::Permanent,
                            "response body exceeds its byte budget",
                        ),
                        span,
                    ));
                }
                body.extend_from_slice(&chunk);
            },
            Ok(None) => break,
            Err(_) => {
                return Err(fail(
                    attempt,
                    SentState::Sent,
                    OpError::new(ErrorKind::Transient, "reading the response body failed"),
                    span,
                ));
            },
        }
    }
    let headers = std::mem::take(response.headers_mut());
    settle(attempt, SentState::Sent, span);
    Ok(Response::new(status, headers, body.freeze()))
}

/// Sends `request` on a granted attempt and settles it: the building block
/// for an operation that makes several different calls in one unit.
///
/// The request is built for the attempt's transport, the unit's pinned
/// credentials are applied last through [`HttpApi::authorize`], and the
/// answer is classified:
///
/// | Situation | Settled | Error kind |
/// |---|---|---|
/// | invalid request, `authorize` refused (`None` slot: `CredentialUnavailable`) | `NotSent` | its kind |
/// | could not connect (DNS, TCP, TLS, connect timeout) | `NotSent` | `Transient` |
/// | failed after connecting, before the head | `MaybeSent` | `Transient` |
/// | `2xx`, `3xx`, an accepted status | `Sent` | — (a [`Response`]) |
/// | `429`, or `503` with `Retry-After` (reported `Throttled`) | `Sent` | `Exhausted { retry_after }` |
/// | `408`, `425`, other `5xx` | `Sent` | `Transient` |
/// | other `4xx` (`401` / `403` included) | `Sent` | `Permanent`, detail the reason phrase |
/// | body over its budget / body read failed | `Sent` | `Permanent` / `Transient` |
///
/// Every answer other than a throttle is reported [`Verdict::Pass`]. The
/// unit's deadline drops an exchange still waiting (`MaybeSent`). A request
/// timeout that fires while the connection is still being established
/// cannot be told apart from a lost request, so it counts as `MaybeSent`
/// too; keep `connect_timeout_ms` below `request_timeout_ms` so a dead
/// host is reported by the connector as `NotSent`.
///
/// # Errors
///
/// As the table says.
pub async fn send<R, M>(attempt: Attempt<'_, R>, request: &Request<M>) -> Result<Response, OpError>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    let span = exchange_span(M::METHOD, None);
    exchange(attempt, request, None, &span)
        .instrument(span.clone())
        .await
        .map_err(|failure| failure.error)
}

/// How long to wait before another attempt, or `None` when the failure
/// must end the unit: only a throttle, a transient failure that sent
/// nothing, or a transient failure of a replay-safe method is retried.
pub(super) fn retry_wait(failure: &Failure, effect: Effect) -> Option<Duration> {
    match failure.error.kind() {
        ErrorKind::Exhausted { retry_after } => Some(retry_after.unwrap_or(Duration::ZERO)),
        ErrorKind::Transient if failure.sent == SentState::NotSent || effect.is_replay_safe() => {
            Some(Duration::ZERO)
        },
        _ => None,
    }
}

/// Waits `wait` before the next attempt when it ends before `deadline`;
/// `false` when it does not.
pub(super) async fn wait_for_retry(wait: Duration, deadline: Instant) -> bool {
    if wait.is_zero() {
        return true;
    }
    if Instant::now()
        .checked_add(wait)
        .is_none_or(|at| at >= deadline)
    {
        return false;
    }
    tokio::time::sleep(wait).await;
    true
}

/// A buffered request as a unit: one [`send`] per attempt. Within
/// [`Request::max_attempts`], a failed attempt is taken again only when it
/// sent nothing, when the provider throttled it (after its `Retry-After`,
/// if that ends before the unit's deadline), or when the method is replay
/// safe and the failure transient — never a `Write` that may have been
/// sent.
impl<R, M> Operation<R> for Request<M>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    type Output = Response;
    const EFFECT: Effect = M::EFFECT;

    fn max_attempts(&self) -> std::num::NonZeroU32 {
        self.attempts()
    }

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<Response, OpError> {
        self.check()?;
        let deadline = cx.deadline();
        loop {
            let index = cx.attempts().saturating_add(1);
            let attempt = cx.attempt(self.cost_value().clone()).await?;
            let span = exchange_span(M::METHOD, Some(index));
            let failure = match exchange(attempt, &self, Some(deadline), &span)
                .instrument(span.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(failure) => failure,
            };
            if cx.attempts() >= self.attempts().get() {
                return Err(failure.error);
            }
            let Some(wait) = retry_wait(&failure, M::EFFECT) else {
                return Err(failure.error);
            };
            if !wait_for_retry(wait, deadline).await {
                return Err(failure.error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;
    use rstest::rstest;

    use super::*;

    fn head(status: u16, retry_after: Option<&'static str>) -> Head {
        let mut headers = HeaderMap::new();
        if let Some(value) = retry_after {
            headers.insert(RETRY_AFTER, HeaderValue::from_static(value));
        }
        classify(
            StatusCode::from_u16(status).expect("status"),
            &headers,
            false,
        )
    }

    #[rstest]
    #[case(200)]
    #[case(204)]
    #[case(301)]
    #[case(307)]
    fn successes_and_redirects_are_answers(#[case] status: u16) {
        assert!(matches!(head(status, None), Head::Answer));
    }

    #[test]
    fn throttles_carry_the_providers_hint() {
        assert!(matches!(
            head(429, Some("2")),
            Head::Throttled(Some(after)) if after == Duration::from_secs(2)
        ));
        assert!(matches!(head(429, None), Head::Throttled(None)));
        assert!(matches!(
            head(503, Some("7")),
            Head::Throttled(Some(after)) if after == Duration::from_secs(7)
        ));
        assert!(matches!(head(503, None), Head::Transient(_)));
    }

    #[rstest]
    #[case(408)]
    #[case(425)]
    #[case(500)]
    #[case(502)]
    #[case(504)]
    fn timeouts_and_server_errors_are_transient(#[case] status: u16) {
        assert!(matches!(head(status, None), Head::Transient(_)));
    }

    #[rstest]
    #[case(400, "Bad Request")]
    #[case(401, "Unauthorized")]
    #[case(403, "Forbidden")]
    #[case(404, "Not Found")]
    #[case(422, "Unprocessable Entity")]
    fn other_client_errors_are_permanent_with_the_reason(
        #[case] status: u16,
        #[case] reason: &str,
    ) {
        assert!(matches!(head(status, None), Head::Permanent(detail) if detail == reason));
    }

    #[test]
    fn an_accepted_status_is_an_answer() {
        let status = StatusCode::NOT_FOUND;
        assert!(matches!(
            classify(status, &HeaderMap::new(), true),
            Head::Answer
        ));
    }

    fn failure(kind: ErrorKind, sent: SentState) -> Failure {
        Failure {
            error: OpError::new(kind, "test"),
            sent,
        }
    }

    #[rstest]
    #[case::not_sent_write(
        ErrorKind::Transient,
        SentState::NotSent,
        Effect::Write,
        Some(Duration::ZERO)
    )]
    #[case::maybe_sent_write(ErrorKind::Transient, SentState::MaybeSent, Effect::Write, None)]
    #[case::sent_write(ErrorKind::Transient, SentState::Sent, Effect::Write, None)]
    #[case::maybe_sent_read(
        ErrorKind::Transient,
        SentState::MaybeSent,
        Effect::Read,
        Some(Duration::ZERO)
    )]
    #[case::sent_idempotent(
        ErrorKind::Transient,
        SentState::Sent,
        Effect::Idempotent,
        Some(Duration::ZERO)
    )]
    #[case::throttled_write(
        ErrorKind::Exhausted { retry_after: Some(Duration::from_secs(2)) },
        SentState::Sent,
        Effect::Write,
        Some(Duration::from_secs(2))
    )]
    #[case::permanent_read(ErrorKind::Permanent, SentState::Sent, Effect::Read, None)]
    #[case::permanent_not_sent(ErrorKind::Permanent, SentState::NotSent, Effect::Read, None)]
    fn only_safe_failures_are_retried_in_the_unit(
        #[case] kind: ErrorKind,
        #[case] sent: SentState,
        #[case] effect: Effect,
        #[case] expected: Option<Duration>,
    ) {
        assert_eq!(retry_wait(&failure(kind, sent), effect), expected);
    }

    #[test]
    fn a_response_prints_no_body_or_header_value() {
        let mut headers = HeaderMap::new();
        headers.insert("x-token", HeaderValue::from_static("header-secret"));
        let response = Response::new(
            StatusCode::OK,
            headers,
            bytes::Bytes::from_static(b"body-secret"),
        );
        let text = format!("{response:?}");
        assert!(text.contains("x-token") && text.contains("200"), "{text}");
        assert!(!text.contains("header-secret") && !text.contains("body-secret"));
        assert_eq!(
            response
                .json::<serde_json::Value>()
                .expect_err("not json")
                .detail(),
            "response body is not the expected JSON"
        );
    }
}
