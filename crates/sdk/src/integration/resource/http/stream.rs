//! Streamed response bodies: one exchange as one streaming unit.

use std::{fmt, num::NonZeroUsize, time::Instant};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, StatusCode};
use nebula_resource::{
    ErrorKind,
    call::{
        Effect, Lease, OperationCx, OperationError, SentState, StreamOperation, StreamSink,
        Streaming,
    },
};
use tracing::Instrument as _;

use super::{
    auth::HttpApi,
    config::HttpTransport,
    exchange::{admit_head, exchange_span, execute, prepare, settle},
    request::{Method, Request},
};

/// Chunks buffered between the exchange and its reader: past it, the
/// exchange stops reading and TCP pushes back on the provider.
const STREAM_CAPACITY: NonZeroUsize = NonZeroUsize::MIN.saturating_add(7);

/// What the exchange hands its reader: the head once, then body chunks.
enum Frame {
    Head {
        status: StatusCode,
        headers: HeaderMap,
    },
    Chunk(Bytes),
}

/// One request whose body is streamed: the whole exchange is one unit with
/// one attempt.
struct StreamExchange<M: Method> {
    request: Request<M>,
}

fn stopped(detail: &'static str) -> OperationError {
    OperationError::new(ErrorKind::Cancelled, detail)
}

impl<R, M> StreamOperation<R> for StreamExchange<M>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    type Item = Frame;
    type Output = ();
    const KEY: &'static str = M::OPERATION_KEY;
    const EFFECT: Effect = M::EFFECT;

    fn idempotency_key(&self) -> Option<String> {
        self.request.key_part().map(str::to_owned)
    }

    async fn run(
        self,
        cx: &mut OperationCx<'_, R>,
        mut sink: StreamSink<Frame>,
    ) -> Result<(), OperationError> {
        let request = self.request;
        let closing = cx.closing();
        let attempt = cx.attempt(request.cost_value().clone()).await?;
        let span = exchange_span(M::METHOD, Some(1));
        async {
            let transport = attempt.instance().as_ref().clone();
            // No total timeout: the unit's deadline bounds the exchange and
            // the transport's read idle timeout bounds a stall.
            let outgoing = match prepare(&attempt, &transport, &request, None) {
                Ok(outgoing) => outgoing,
                Err(error) => {
                    settle(attempt, SentState::NotSent, &span);
                    return Err(error);
                },
            };
            let head = tokio::select! {
                biased;
                () = closing.closed() => None,
                () = sink.closed() => None,
                head = execute(attempt, &transport, outgoing, &span) => Some(head),
            };
            // Dropping `execute` above drops its attempt unsettled:
            // `MaybeSent`, as the request may be on the wire.
            let Some(head) = head else {
                return Err(stopped("stream stopped before its response head"));
            };
            let (attempt, mut response) = head.map_err(|failure| failure.error)?;
            let status = response.status();
            let attempt = admit_head(attempt, &response, request.accepts(status), &span)
                .await
                .map_err(|failure| failure.error)?;
            let budget = request.body_budget(transport.limits().max_stream_bytes);
            if response
                .content_length()
                .is_some_and(|length| length > budget)
            {
                settle(attempt, SentState::Sent, &span);
                return Err(OperationError::new(
                    ErrorKind::Permanent,
                    "streamed body exceeds its byte budget",
                ));
            }
            settle(attempt, SentState::Sent, &span);
            let headers = std::mem::take(response.headers_mut());
            sink.send(Frame::Head { status, headers }).await?;

            let mut streamed: u64 = 0;
            loop {
                let chunk = tokio::select! {
                    biased;
                    () = closing.closed() => return Err(stopped("lease closing; stream stopped")),
                    () = sink.closed() => return Err(stopped("the stream's consumer is gone")),
                    chunk = response.chunk() => chunk,
                };
                let chunk = match chunk {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => return Ok(()),
                    Err(_) => {
                        return Err(OperationError::new(
                            ErrorKind::Transient,
                            "reading the response stream failed",
                        ));
                    },
                };
                streamed = streamed.saturating_add(chunk.len() as u64);
                if streamed > budget {
                    return Err(OperationError::new(
                        ErrorKind::Permanent,
                        "streamed body exceeds its byte budget",
                    ));
                }
                tokio::select! {
                    biased;
                    () = closing.closed() => return Err(stopped("lease closing; stream stopped")),
                    sent = sink.send(Frame::Chunk(chunk)) => sent?,
                }
            }
        }
        .instrument(span.clone())
        .await
    }
}

/// A response whose body arrives in chunks, from [`open_stream`].
///
/// The whole exchange is one unit: the lease stays held until the body
/// ends, fails, or the stream is dropped or cancelled. `Debug` shows the
/// status and header names only.
pub struct ResponseStream {
    status: StatusCode,
    headers: HeaderMap,
    frames: Streaming<Frame, ()>,
}

impl ResponseStream {
    /// The status.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The headers.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The next body chunk; the unit's error once, after the chunks read
    /// before it; `None` at the end of the body.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe, as [`Streaming::next`].
    pub async fn next(&mut self) -> Option<Result<Bytes, OperationError>> {
        match self.frames.next().await? {
            Ok(Frame::Chunk(chunk)) => Some(Ok(chunk)),
            Ok(Frame::Head { .. }) => Some(Err(OperationError::new(
                ErrorKind::Permanent,
                "response stream repeated its head",
            ))),
            Err(error) => Some(Err(error)),
        }
    }

    /// Stops the exchange: it drops the body and settles `Cancelled`.
    pub fn cancel(&self) {
        self.frames.cancel();
    }
}

impl fmt::Debug for ResponseStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        formatter
            .debug_struct("ResponseStream")
            .field("status", &self.status)
            .field("headers", &headers)
            .finish_non_exhaustive()
    }
}

/// Sends `request` as one streaming unit and returns once the response
/// head arrived, with the body to read in chunks.
///
/// The unit takes one attempt; its sent state is `Sent` once the head
/// arrived. Error statuses are not streamed: they fail as [`send`](super::send)
/// classifies them. A body over the transport's stream budget (or the
/// request's [`max_bytes`](Request::max_bytes)) fails `Permanent`; a body
/// read failure `Transient`; the unit's deadline mid-body `MaybeSent` (a
/// `Write` then has an unknown outcome). The lease closing stops the body
/// (`Cancelled`), and so does dropping or cancelling the stream. At most 8
/// chunks are buffered; past them the exchange stops reading.
///
/// # Errors
///
/// The unit's error when it failed before the head.
pub async fn open_stream<R, M>(
    lease: &Lease<R>,
    request: Request<M>,
) -> Result<ResponseStream, OperationError>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    first_frame(lease.submit_streaming(StreamExchange { request }, STREAM_CAPACITY)).await
}

/// [`open_stream`] with the unit's deadline shortened to `deadline`.
///
/// # Errors
///
/// As [`open_stream`].
pub async fn open_stream_until<R, M>(
    lease: &Lease<R>,
    request: Request<M>,
    deadline: Instant,
) -> Result<ResponseStream, OperationError>
where
    R: HttpApi,
    R::Instance: AsRef<HttpTransport>,
    M: Method,
{
    let frames = lease
        .submit_streaming(StreamExchange { request }, STREAM_CAPACITY)
        .with_deadline(deadline);
    first_frame(frames).await
}

async fn first_frame(mut frames: Streaming<Frame, ()>) -> Result<ResponseStream, OperationError> {
    match frames.next().await {
        Some(Ok(Frame::Head { status, headers })) => Ok(ResponseStream {
            status,
            headers,
            frames,
        }),
        Some(Ok(Frame::Chunk(_))) => Err(OperationError::new(
            ErrorKind::Permanent,
            "response stream sent a chunk before its head",
        )),
        Some(Err(error)) => Err(error),
        None => Err(OperationError::new(
            ErrorKind::Permanent,
            "response stream ended before its head",
        )),
    }
}
