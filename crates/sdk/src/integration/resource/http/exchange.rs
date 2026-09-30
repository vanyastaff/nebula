//! One exchange per attempt: credentials last, the request, and the answer
//! classified once — the runtime derives the attempt's sent state, the
//! rate limit's verdict and any re-attempt from that classification.

use std::time::{Duration, Instant};

use bytes::BytesMut;
use http::{HeaderMap, HeaderValue, StatusCode, header::RETRY_AFTER};
use nebula_resource::{
    ErrorKind,
    call::{Attempt, Effect, IdempotencyKey, Operation, OperationCx, OperationError, SentState},
    rate_limit::retry_after_from_header,
};
use tracing::Instrument as _;

use super::{
    auth::{Authorize, HttpApi},
    config::HttpTransport,
    request::{IDEMPOTENCY_KEY, Method, Request, sealed::Sealed},
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

impl Head {
    /// The classified error of a head that is not an answer, with the sent
    /// state it implies (for the exchange span).
    pub(super) fn error(self) -> Option<(OperationError, SentState)> {
        match self {
            Self::Answer => None,
            Self::Throttled(after) => Some((OperationError::throttled(after), SentState::Sent)),
            Self::Transient(detail) => {
                Some((OperationError::interrupted(detail), SentState::MaybeSent))
            },
            Self::Permanent(detail) => Some((OperationError::rejected(detail), SentState::Sent)),
        }
    }
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

/// Records `sent` on the exchange span and hands `error` back.
pub(super) fn failed(
    span: &tracing::Span,
    sent: SentState,
    error: OperationError,
) -> OperationError {
    span.record("sent", sent.as_str());
    error
}

/// A request refused before anything was sent: provably unsent, with the
/// refusal's kind.
fn refused(span: &tracing::Span, error: &OperationError) -> OperationError {
    failed(
        span,
        SentState::NotSent,
        OperationError::unreachable_as(error.kind().clone(), error.detail()),
    )
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

/// The request an attempt sends: built without credentials, a
/// [`Keyed`](super::Keyed) request's `Idempotency-Key` set to the unit's
/// derived `key`, then the pinned `credentials` applied last, bounded by
/// the transport's request timeout and the unit's deadline. A refusal is
/// classified unsent.
pub(super) fn prepare<R, M>(
    transport: &HttpTransport,
    credentials: &R::Pinned,
    key: Option<&IdempotencyKey>,
    request: &Request<M>,
    timeout: Option<Duration>,
    span: &tracing::Span,
) -> Result<reqwest::Request, OperationError>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    let build = || {
        let mut outgoing = request.outgoing(transport)?;
        if <M as Sealed>::KEYED {
            // A unit that declared no key part (a hand-written operation that
            // `send`s a keyed request) has no key to send: refuse rather than
            // send the repeat-absorbing request without one.
            let Some(key) = key else {
                return Err(OperationError::new(
                    ErrorKind::Permanent,
                    "a keyed request needs the unit's idempotency key; declare Operation::idempotency_key",
                ));
            };
            let value = HeaderValue::from_str(key.as_str()).map_err(|_| {
                OperationError::new(ErrorKind::Permanent, "invalid idempotency key header")
            })?;
            outgoing.headers_mut().insert(IDEMPOTENCY_KEY, value);
        }
        let mut headers = HeaderMap::new();
        R::authorize(credentials, &mut Authorize::new(&mut headers))?;
        outgoing.headers_mut().extend(headers);
        *outgoing.timeout_mut() = timeout;
        Ok(outgoing)
    };
    build().map_err(|error| refused(span, &error))
}

/// Sends `outgoing`: unreachable when the connection was never
/// established, interrupted when it failed after.
pub(super) async fn execute(
    transport: &HttpTransport,
    outgoing: reqwest::Request,
    span: &tracing::Span,
) -> Result<reqwest::Response, OperationError> {
    // The error is never formatted: its `Display` carries the URL.
    match transport.client().execute(outgoing).await {
        Ok(response) => {
            span.record("status", response.status().as_u16());
            Ok(response)
        },
        Err(error) if error.is_connect() => Err(failed(
            span,
            SentState::NotSent,
            OperationError::unreachable("could not connect; the request was not sent"),
        )),
        Err(_) => Err(failed(
            span,
            SentState::MaybeSent,
            OperationError::interrupted("the request failed before a response arrived"),
        )),
    }
}

/// Fails an exchange whose head is not an answer, classified.
pub(super) fn admit_head(
    response: &reqwest::Response,
    accepted: bool,
    span: &tracing::Span,
) -> Result<(), OperationError> {
    match classify(response.status(), response.headers(), accepted).error() {
        None => Ok(()),
        Some((error, sent)) => Err(failed(span, sent, error)),
    }
}

/// One exchange on an attempt's `transport`, end to end.
async fn exchange<R, M>(
    transport: &HttpTransport,
    credentials: &R::Pinned,
    key: Option<&IdempotencyKey>,
    request: &Request<M>,
    deadline: Option<Instant>,
    span: &tracing::Span,
) -> Result<Response, OperationError>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    let limits = transport.limits();
    let timeout = deadline.map_or(limits.request_timeout, |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .min(limits.request_timeout)
    });
    let outgoing = prepare::<R, M>(transport, credentials, key, request, Some(timeout), span)?;
    let mut response = execute(transport, outgoing, span).await?;
    let status = response.status();
    admit_head(&response, request.accepts(status), span)?;

    let over_budget = || {
        failed(
            span,
            SentState::Sent,
            OperationError::rejected("response body exceeds its byte budget"),
        )
    };
    let budget = request.body_budget(limits.max_response_bytes);
    if response
        .content_length()
        .is_some_and(|length| length > budget)
    {
        return Err(over_budget());
    }
    let mut body = BytesMut::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let room = budget.saturating_sub(body.len() as u64);
                if chunk.len() as u64 > room {
                    return Err(over_budget());
                }
                body.extend_from_slice(&chunk);
            },
            Ok(None) => break,
            Err(_) => {
                return Err(failed(
                    span,
                    SentState::MaybeSent,
                    OperationError::interrupted("reading the response body failed"),
                ));
            },
        }
    }
    let headers = std::mem::take(response.headers_mut());
    span.record("sent", SentState::Sent.as_str());
    Ok(Response::new(status, headers, body.freeze()))
}

/// Sends `request` on a granted attempt and finishes it
/// ([`Attempt::finish`]): the building block for an operation that makes
/// several different calls in one unit. Inside
/// [`OperationCx::call`](nebula_resource::call::OperationCx::call) use
/// [`Lease::submit`](nebula_resource::call::Lease::submit) of the
/// [`Request`] instead: its [`Operation`] impl is that call.
///
/// The request is built for the attempt's transport, the unit's pinned
/// credentials are applied last through [`HttpApi::authorize`], and the
/// answer is classified with an [`OperationError`] constructor, from which
/// the runtime derives the attempt's sent state and what the rate limit is
/// told:
///
/// | Situation | Classified | Sent | Error kind |
/// |---|---|---|---|
/// | invalid request, `authorize` refused (`None` slot: `CredentialUnavailable`), a [`Keyed`](super::Keyed) request in a unit without an idempotency key | `unreachable_as` | `NotSent` | its kind |
/// | could not connect (DNS, TCP, TLS, connect timeout) | `unreachable` | `NotSent` | `Transient` |
/// | failed after connecting, before the head | `interrupted` | `MaybeSent` | `Transient` |
/// | `2xx`, `3xx`, an accepted status | success | `Sent` | — (a [`Response`]) |
/// | `429`, or `503` with `Retry-After` | `throttled` (the quota pauses) | `Sent` | `Exhausted { retry_after }` |
/// | `408`, `425`, other `5xx` | `interrupted` | `MaybeSent` | `Transient` |
/// | other `4xx` (`401` / `403` included) | `rejected` | `Sent` | `Permanent`, detail the reason phrase |
/// | body over its budget | `rejected` | `Sent` | `Permanent` |
/// | body read failed | `interrupted` | `MaybeSent` | `Transient` |
///
/// The unit's deadline drops an exchange still waiting (`MaybeSent`). A
/// request timeout that fires while the connection is still being
/// established cannot be told apart from a lost request, so it counts as
/// `MaybeSent` too; keep `connect_timeout_ms` below `request_timeout_ms`
/// so a dead host is reported by the connector as `NotSent`.
///
/// # Errors
///
/// As the table says.
pub async fn send<R, M>(
    attempt: Attempt<'_, R>,
    request: &Request<M>,
) -> Result<Response, OperationError>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    let span = exchange_span(M::METHOD, None);
    let result = exchange::<R, M>(
        attempt.instance().as_ref(),
        attempt.credentials(),
        attempt.idempotency_key(),
        request,
        None,
        &span,
    )
    .instrument(span.clone())
    .await;
    attempt.finish(&result).await;
    result
}

/// A buffered request as a unit: one exchange per attempt through
/// [`OperationCx::call`]. Within [`Request::max_attempts`], a failed
/// attempt is taken again when the classification allows it — nothing
/// was sent, the provider throttled it (the next attempt's quota booking
/// waits its `Retry-After` out, within the unit's deadline), or the method
/// is replay safe and the exchange was interrupted — never a `Write` that
/// may have been sent, and never a rejection.
///
/// Its key is the marker's [`Method::OPERATION_KEY`]; a
/// [`Keyed`](super::Keyed) request declares its key as the developer part
/// of the unit's idempotency key, and the `Idempotency-Key` header carries
/// the key the unit derives from it.
impl<R, M> Operation<R> for Request<M>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    type Output = Response;
    const KEY: &'static str = M::OPERATION_KEY;
    const EFFECT: Effect = M::EFFECT;

    fn idempotency_key(&self) -> Option<String> {
        self.key_part().map(str::to_owned)
    }

    fn max_attempts(&self) -> std::num::NonZeroU32 {
        self.attempts()
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<Response, OperationError> {
        let key = cx.idempotency_key().copied();
        let deadline = cx.deadline();
        let cost = self.cost_value().clone();
        let request = self;
        let mut index: u32 = 0;
        cx.call(cost, async move |instance, credentials| {
            index = index.saturating_add(1);
            let span = exchange_span(M::METHOD, Some(index));
            exchange::<R, M>(
                instance.as_ref(),
                credentials,
                key.as_ref(),
                &request,
                Some(deadline),
                &span,
            )
            .instrument(span.clone())
            .await
        })
        .await
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
        assert!(head(status, None).error().is_none());
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

    /// A head's classified error: its kind, detail and implied sent state.
    fn classified(status: u16, retry_after: Option<&'static str>) -> (ErrorKind, SentState) {
        let (error, sent) = head(status, retry_after).error().expect("an error head");
        (error.kind().clone(), sent)
    }

    #[test]
    fn heads_classify_into_throttled_interrupted_and_rejected() {
        let throttled = (
            ErrorKind::Exhausted {
                retry_after: Some(Duration::from_secs(2)),
            },
            SentState::Sent,
        );
        assert_eq!(classified(429, Some("2")), throttled);
        assert_eq!(classified(503, Some("2")), throttled);
        for status in [408, 425, 500, 503] {
            assert_eq!(
                classified(status, None),
                (ErrorKind::Transient, SentState::MaybeSent),
                "{status}: interrupted"
            );
        }
        for status in [400, 401, 404, 422] {
            assert_eq!(
                classified(status, None),
                (ErrorKind::Permanent, SentState::Sent),
                "{status}: rejected"
            );
        }
        let (error, _) = head(404, None).error().expect("an error head");
        assert_eq!(error.detail(), "Not Found", "the reason phrase");
    }

    #[test]
    fn a_response_round_trips_as_its_recorded_form() {
        let mut headers = HeaderMap::new();
        headers.append("x-page", HeaderValue::from_static("1"));
        headers.append("x-page", HeaderValue::from_static("2"));
        let response = Response::new(
            StatusCode::CREATED,
            headers,
            bytes::Bytes::from_static(b"\x00ok"),
        );
        let json = serde_json::to_value(&response).expect("serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "status": 201,
                "headers": [["x-page", "1"], ["x-page", "2"]],
                "body": "AG9r",
            })
        );
        let back: Response = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back.status(), StatusCode::CREATED);
        assert_eq!(back.headers().get_all("x-page").iter().count(), 2);
        assert_eq!(back.body().as_ref(), b"\x00ok");

        let mut opaque = HeaderMap::new();
        opaque.insert(
            "x-bytes",
            HeaderValue::from_bytes(b"\xff").expect("obs-text"),
        );
        let unrecordable = Response::new(StatusCode::OK, opaque, bytes::Bytes::new());
        assert!(
            serde_json::to_value(&unrecordable).is_err(),
            "a non-UTF-8 header value is recorded digest-only"
        );
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
