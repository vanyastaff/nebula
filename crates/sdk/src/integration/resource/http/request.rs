//! Requests as operation intent: the method marker that fixes the effect,
//! and the validated path, query, headers and body.

use std::{fmt, marker::PhantomData, num::NonZeroU32};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use http::{
    HeaderMap, HeaderName, HeaderValue, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, PROXY_AUTHORIZATION},
};
use nebula_resource::{
    ErrorKind,
    call::{Cost, Effect, OperationError},
};
use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};

use super::config::HttpTransport;

pub(super) mod sealed {
    /// Seals [`Method`](super::Method): only the markers of this module
    /// implement it.
    pub trait Sealed {
        /// Whether the request carries an `Idempotency-Key`.
        const KEYED: bool = false;

        /// The request method.
        fn http_method() -> http::Method;
    }
}

/// An HTTP method marker. It fixes the method, the [`Effect`] a
/// [`Request`] declares and its operation key, so the retry safety of a
/// request is a property of its type.
///
/// | Marker | Method | Effect | Operation key |
/// |---|---|---|---|
/// | [`Get`], [`Head`], [`Options`] | `GET`, `HEAD`, `OPTIONS` | `Read` | `http.get`, `http.head`, `http.options` |
/// | [`Put`], [`Delete`] | `PUT`, `DELETE` | `Idempotent` | `http.put`, `http.delete` |
/// | [`Post`], [`Patch`] | `POST`, `PATCH` | `Write` | `http.post`, `http.patch` |
/// | [`Keyed<Post>`](Keyed), [`Keyed<Patch>`](Keyed) | with an `Idempotency-Key` | `Idempotent` | `http.post.keyed`, `http.patch.keyed` |
/// | [`AsWrite<Put>`](AsWrite), [`AsWrite<Delete>`](AsWrite) | a non-idempotent `PUT` / `DELETE` | `Write` | `http.put.as_write`, `http.delete.as_write` |
///
/// `TRACE` and `CONNECT` have no marker.
pub trait Method: sealed::Sealed + Send + 'static {
    /// The method's name.
    const METHOD: &'static str;
    /// What repeating the request does to the provider.
    const EFFECT: Effect;
    /// The [`Operation::KEY`](nebula_resource::call::Operation::KEY) a
    /// request of this marker is journaled under.
    const OPERATION_KEY: &'static str;
}

macro_rules! method_marker {
    ($(#[$doc:meta])* $marker:ident, $method:ident, $effect:ident, $key:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $marker;

        impl sealed::Sealed for $marker {
            fn http_method() -> http::Method {
                http::Method::$method
            }
        }

        impl Method for $marker {
            const METHOD: &'static str = stringify!($method);
            const EFFECT: Effect = Effect::$effect;
            const OPERATION_KEY: &'static str = $key;
        }
    };
}

method_marker!(
    /// `GET`: reads.
    Get, GET, Read, "http.get"
);
method_marker!(
    /// `HEAD`: reads.
    Head, HEAD, Read, "http.head"
);
method_marker!(
    /// `OPTIONS`: reads.
    Options, OPTIONS, Read, "http.options"
);
method_marker!(
    /// `PUT`: idempotent by HTTP semantics; [`Request::as_write`] declares a
    /// provider that breaks them.
    Put, PUT, Idempotent, "http.put"
);
method_marker!(
    /// `DELETE`: idempotent by HTTP semantics; [`Request::as_write`]
    /// declares a provider that breaks them.
    Delete, DELETE, Idempotent, "http.delete"
);
method_marker!(
    /// `POST`: a write; [`Request::idempotency_key`] makes it idempotent.
    Post, POST, Write, "http.post"
);
method_marker!(
    /// `PATCH`: a write; [`Request::idempotency_key`] makes it idempotent.
    Patch, PATCH, Write, "http.patch"
);

/// A `POST` or `PATCH` carrying an `Idempotency-Key`: the provider absorbs a
/// repeat, so the request is `Idempotent`. Built by
/// [`Request::idempotency_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keyed<M>(PhantomData<M>);

/// A `PUT` or `DELETE` whose provider applies a repeat again: a `Write`.
/// Built by [`Request::as_write`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsWrite<M>(PhantomData<M>);

macro_rules! wrapped_marker {
    ($wrapper:ident < $inner:ident >, $effect:ident, $keyed:literal, $key:literal) => {
        impl sealed::Sealed for $wrapper<$inner> {
            const KEYED: bool = $keyed;

            fn http_method() -> http::Method {
                <$inner as sealed::Sealed>::http_method()
            }
        }

        impl Method for $wrapper<$inner> {
            const METHOD: &'static str = <$inner as Method>::METHOD;
            const EFFECT: Effect = Effect::$effect;
            const OPERATION_KEY: &'static str = $key;
        }
    };
}

wrapped_marker!(Keyed<Post>, Idempotent, true, "http.post.keyed");
wrapped_marker!(Keyed<Patch>, Idempotent, true, "http.patch.keyed");
wrapped_marker!(AsWrite<Put>, Write, false, "http.put.as_write");
wrapped_marker!(AsWrite<Delete>, Write, false, "http.delete.as_write");

/// Headers a request may not set: credentials are applied from the unit's
/// pinned slots by [`HttpApi::authorize`](super::HttpApi::authorize) only.
const FORBIDDEN_HEADERS: [HeaderName; 3] = [AUTHORIZATION, PROXY_AUTHORIZATION, COOKIE];

pub(super) const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");

/// One HTTP request, described as data and submitted as an
/// [`Operation`](nebula_resource::call::Operation) on a resource that
/// implements [`HttpApi`](super::HttpApi).
///
/// The path is appended to the transport's base URL (after its mount
/// prefix) and can never leave its origin or prefix: it must start with one
/// `/`, and may not hold `\`, `?`, `#`, control characters or `.` / `..`
/// segments. Query pairs go through [`query`](Self::query). A request never
/// carries credentials of its own: `Authorization`, `Proxy-Authorization`
/// and `Cookie` are refused.
///
/// Defaults: [`Cost::ONE`], one attempt, the transport's response budget.
/// `Debug` shows the method and header names only.
///
/// A request is its own journaled intent: it serializes as
/// `{path, query, headers: [[name, value]], body?: base64, accept,
/// max_bytes, idempotency_key?}` — the cost and the attempt budget are
/// policy, not intent, and are not serialized (a deserialized request has
/// the defaults).
///
/// ```
/// # fn build() -> Result<(), nebula_sdk::integration::resource::OperationError> {
/// use nebula_sdk::integration::resource::http::Request;
///
/// let request = Request::post("/repos/acme/app/issues")?
///     .json(&serde_json::json!({ "title": "flaky test" }))?
///     .idempotency_key("issue-42");
/// assert!(!format!("{request:?}").contains("acme"));
/// # Ok(()) }
/// # build().unwrap();
/// ```
pub struct Request<M: Method> {
    path: String,
    query: Vec<(String, String)>,
    headers: HeaderMap,
    body: Option<Bytes>,
    cost: Cost,
    max_attempts: NonZeroU32,
    accept: Vec<StatusCode>,
    max_bytes: Option<u64>,
    /// The developer part of the provider idempotency key of a
    /// [`Keyed`] request.
    key_part: Option<String>,
    method: PhantomData<fn() -> M>,
}

/// The wire form of a [`Request`]: its intent only.
#[derive(Serialize, Deserialize)]
struct RequestWire {
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    accept: Vec<u16>,
    max_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    idempotency_key: Option<String>,
}

/// `headers` as `[name, value]` pairs, repeated names kept in order.
pub(super) fn header_pairs<E: ser::Error>(headers: &HeaderMap) -> Result<Vec<(String, String)>, E> {
    headers
        .iter()
        .map(|(name, value)| {
            let value = std::str::from_utf8(value.as_bytes())
                .map_err(|_| E::custom("a header value is not UTF-8"))?;
            Ok((name.as_str().to_owned(), value.to_owned()))
        })
        .collect()
}

/// The header map of `[name, value]` pairs.
pub(super) fn header_map<E: de::Error>(pairs: Vec<(String, String)>) -> Result<HeaderMap, E> {
    let mut headers = HeaderMap::with_capacity(pairs.len());
    for (name, value) in pairs {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| E::custom("invalid header name"))?;
        let value = HeaderValue::try_from(value).map_err(|_| E::custom("invalid header value"))?;
        headers.append(name, value);
    }
    Ok(headers)
}

impl<M: Method> Serialize for Request<M> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RequestWire {
            path: self.path.clone(),
            query: self.query.clone(),
            headers: header_pairs(&self.headers)?,
            body: self.body.as_ref().map(|body| STANDARD.encode(body)),
            accept: self.accept.iter().map(StatusCode::as_u16).collect(),
            max_bytes: self.max_bytes,
            idempotency_key: self.key_part.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de, M: Method> Deserialize<'de> for Request<M> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = RequestWire::deserialize(deserializer)?;
        validate_path(&wire.path).map_err(|error| de::Error::custom(error.detail()))?;
        let headers = header_map(wire.headers)?;
        if FORBIDDEN_HEADERS
            .iter()
            .any(|name| headers.contains_key(name))
        {
            return Err(de::Error::custom(
                "credential headers are applied from credential slots only",
            ));
        }
        let body = wire
            .body
            .map(|body| STANDARD.decode(body).map(Bytes::from))
            .transpose()
            .map_err(|_| de::Error::custom("request body is not base64"))?;
        let accept = wire
            .accept
            .into_iter()
            .map(StatusCode::from_u16)
            .collect::<Result<_, _>>()
            .map_err(|_| de::Error::custom("invalid accepted status"))?;
        Ok(Self {
            path: wire.path,
            query: wire.query,
            headers,
            body,
            cost: Cost::ONE,
            max_attempts: NonZeroU32::MIN,
            accept,
            max_bytes: wire.max_bytes,
            key_part: wire.idempotency_key.filter(|_| M::KEYED),
            method: PhantomData,
        })
    }
}

impl<M: Method> fmt::Debug for Request<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        formatter
            .debug_struct("Request")
            .field("method", &M::METHOD)
            .field("path", &"<redacted>")
            .field("query", &"<redacted>")
            .field("headers", &headers)
            .field("body_len", &self.body.as_ref().map(Bytes::len))
            .field("cost", &self.cost)
            .field("max_attempts", &self.max_attempts)
            .finish_non_exhaustive()
    }
}

fn invalid(detail: &'static str) -> OperationError {
    OperationError::new(ErrorKind::Permanent, detail)
}

impl<M: Method> Request<M> {
    fn new(path: &str) -> Result<Self, OperationError> {
        validate_path(path)?;
        Ok(Self {
            path: path.to_owned(),
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: None,
            cost: Cost::ONE,
            max_attempts: NonZeroU32::MIN,
            accept: Vec::new(),
            max_bytes: None,
            key_part: None,
            method: PhantomData,
        })
    }

    /// Appends query pairs; they are percent-encoded.
    #[must_use]
    pub fn query(mut self, pairs: &[(&str, &str)]) -> Self {
        self.query.extend(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        self
    }

    /// Sets a header.
    ///
    /// # Errors
    ///
    /// A permanent error for an invalid name or value, or for
    /// `Authorization`, `Proxy-Authorization` and `Cookie`: credentials come
    /// from the resource's slots through
    /// [`HttpApi::authorize`](super::HttpApi::authorize).
    pub fn header(
        mut self,
        name: &'static str,
        value: impl Into<String>,
    ) -> Result<Self, OperationError> {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| invalid("invalid request header name"))?;
        if FORBIDDEN_HEADERS.contains(&name) {
            return Err(invalid(
                "credential headers are applied from credential slots only",
            ));
        }
        let value = HeaderValue::try_from(value.into())
            .map_err(|_| invalid("invalid request header value"))?;
        self.headers.insert(name, value);
        Ok(self)
    }

    /// Sets the body.
    #[must_use]
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Sets a JSON body and `Content-Type: application/json`.
    ///
    /// # Errors
    ///
    /// A permanent error when `value` does not serialize.
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Result<Self, OperationError> {
        let body =
            serde_json::to_vec(value).map_err(|_| invalid("request body does not serialize"))?;
        self.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        self.body = Some(Bytes::from(body));
        Ok(self)
    }

    /// Sets what one attempt costs against the row's rate limit.
    #[must_use]
    pub fn cost(mut self, cost: Cost) -> Self {
        self.cost = cost;
        self
    }

    /// Sets how many attempts one unit may take. A failed attempt is taken
    /// again only when nothing was sent, when the provider throttled it, or
    /// when the method is replay safe and the failure transient.
    #[must_use]
    pub fn max_attempts(mut self, attempts: NonZeroU32) -> Self {
        self.max_attempts = attempts;
        self
    }

    /// Accepts `status` as an answer: it returns a [`Response`](super::Response)
    /// instead of failing the unit (a `404` that means "absent").
    #[must_use]
    pub fn accept_status(mut self, status: StatusCode) -> Self {
        self.accept.push(status);
        self
    }

    /// Caps the buffered response body at `bytes`; never above the
    /// transport's budget.
    #[must_use]
    pub fn max_bytes(mut self, bytes: u64) -> Self {
        self.max_bytes = Some(bytes);
        self
    }

    pub(super) fn cost_value(&self) -> &Cost {
        &self.cost
    }

    pub(super) fn attempts(&self) -> NonZeroU32 {
        self.max_attempts
    }

    pub(super) fn accepts(&self, status: StatusCode) -> bool {
        self.accept.contains(&status)
    }

    /// The response budget: the request's cap within the transport's.
    pub(super) fn body_budget(&self, transport_budget: u64) -> u64 {
        self.max_bytes
            .map_or(transport_budget, |bytes| bytes.min(transport_budget))
    }

    /// The developer part of a [`Keyed`] request's idempotency key.
    pub(super) fn key_part(&self) -> Option<&str> {
        self.key_part.as_deref()
    }

    /// The outgoing request, without credentials: its URL is the base plus
    /// this path, and must stay under the base's mount prefix.
    pub(super) fn outgoing(
        &self,
        transport: &HttpTransport,
    ) -> Result<reqwest::Request, OperationError> {
        let base = transport.base();
        let prefix = base.path().trim_end_matches('/');
        let mut url: Url = base.clone();
        url.set_path(&format!("{prefix}{}", self.path));
        if !url
            .path()
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
        {
            return Err(invalid("request path leaves the base URL's mount prefix"));
        }
        if !self.query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in &self.query {
                pairs.append_pair(name, value);
            }
        }
        let mut outgoing = reqwest::Request::new(M::http_method(), url);
        *outgoing.headers_mut() = self.headers.clone();
        if let Some(body) = &self.body {
            *outgoing.body_mut() = Some(body.clone().into());
        }
        Ok(outgoing)
    }

    fn retag<N: Method>(self) -> Request<N> {
        Request {
            path: self.path,
            query: self.query,
            headers: self.headers,
            body: self.body,
            cost: self.cost,
            max_attempts: self.max_attempts,
            accept: self.accept,
            max_bytes: self.max_bytes,
            key_part: self.key_part,
            method: PhantomData,
        }
    }

    fn with_idempotency_key(mut self, key: String) -> Self {
        self.key_part = Some(key);
        self
    }
}

impl Request<Get> {
    /// A `GET` of `path`.
    ///
    /// # Errors
    ///
    /// A permanent error when `path` breaks the path rules of [`Request`].
    pub fn get(path: &str) -> Result<Self, OperationError> {
        Self::new(path)
    }
}

impl Request<Head> {
    /// A `HEAD` of `path`.
    ///
    /// # Errors
    ///
    /// A permanent error when `path` breaks the path rules of [`Request`].
    pub fn head(path: &str) -> Result<Self, OperationError> {
        Self::new(path)
    }
}

impl Request<Options> {
    /// An `OPTIONS` of `path`.
    ///
    /// # Errors
    ///
    /// A permanent error when `path` breaks the path rules of [`Request`].
    pub fn options(path: &str) -> Result<Self, OperationError> {
        Self::new(path)
    }
}

impl Request<Post> {
    /// A `POST` to `path`.
    ///
    /// # Errors
    ///
    /// A permanent error when `path` breaks the path rules of [`Request`].
    pub fn post(path: &str) -> Result<Self, OperationError> {
        Self::new(path)
    }

    /// Sends the `POST` with an `Idempotency-Key`: the provider absorbs a
    /// repeat, so the request becomes `Idempotent` and a unit with an
    /// unknown outcome may be retried with the same key.
    ///
    /// `key` is the developer part of the key
    /// ([`Operation::idempotency_key`](nebula_resource::call::Operation::idempotency_key)):
    /// 1 to 256 bytes of visible ASCII, deterministic for the call. The
    /// header carries the unit's derived key
    /// ([`OperationCx::idempotency_key`](nebula_resource::call::OperationCx::idempotency_key)),
    /// never `key` itself. An invalid key refuses the request before any
    /// attempt.
    #[must_use]
    pub fn idempotency_key(self, key: impl Into<String>) -> Request<Keyed<Post>> {
        self.with_idempotency_key(key.into()).retag()
    }
}

impl Request<Patch> {
    /// A `PATCH` of `path`.
    ///
    /// # Errors
    ///
    /// A permanent error when `path` breaks the path rules of [`Request`].
    pub fn patch(path: &str) -> Result<Self, OperationError> {
        Self::new(path)
    }

    /// Sends the `PATCH` with an `Idempotency-Key`, as
    /// [`Request::<Post>::idempotency_key`].
    #[must_use]
    pub fn idempotency_key(self, key: impl Into<String>) -> Request<Keyed<Patch>> {
        self.with_idempotency_key(key.into()).retag()
    }
}

impl Request<Put> {
    /// A `PUT` of `path`.
    ///
    /// # Errors
    ///
    /// A permanent error when `path` breaks the path rules of [`Request`].
    pub fn put(path: &str) -> Result<Self, OperationError> {
        Self::new(path)
    }

    /// Declares that this provider applies a repeated `PUT` again: the
    /// request becomes a `Write`, never retried after it may have been sent.
    #[must_use]
    pub fn as_write(self) -> Request<AsWrite<Put>> {
        self.retag()
    }
}

impl Request<Delete> {
    /// A `DELETE` of `path`.
    ///
    /// # Errors
    ///
    /// A permanent error when `path` breaks the path rules of [`Request`].
    pub fn delete(path: &str) -> Result<Self, OperationError> {
        Self::new(path)
    }

    /// Declares that this provider applies a repeated `DELETE` again, as
    /// [`Request::<Put>::as_write`].
    #[must_use]
    pub fn as_write(self) -> Request<AsWrite<Delete>> {
        self.retag()
    }
}

fn validate_path(path: &str) -> Result<(), OperationError> {
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(invalid("request path must start with a single `/`"));
    }
    if path
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | '?' | '#'))
    {
        return Err(invalid(
            "request path holds a control character, `\\`, `?` or `#`",
        ));
    }
    let dot_segment = path.split('/').any(|segment| {
        let decoded = segment.replace("%2e", ".").replace("%2E", ".");
        decoded == "." || decoded == ".."
    });
    if dot_segment {
        return Err(invalid("request path holds a `.` or `..` segment"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{sealed::Sealed as _, *};

    #[rstest]
    #[case::relative("user")]
    #[case::network_path("//evil.example/user")]
    #[case::backslash("/a\\b")]
    #[case::query("/user?token=1")]
    #[case::fragment("/user#frag")]
    #[case::control("/us\ner")]
    #[case::dot_dot("/v1/../admin")]
    #[case::encoded_dot_dot("/v1/%2e%2E/admin")]
    #[case::dot("/./user")]
    fn bad_paths_are_refused_before_any_attempt(#[case] path: &str) {
        let error = Request::get(path).expect_err("refused");
        assert_eq!(*error.kind(), ErrorKind::Permanent);
    }

    #[rstest]
    #[case("authorization")]
    #[case("Proxy-Authorization")]
    #[case("cookie")]
    fn credential_headers_are_refused(#[case] name: &'static str) {
        let error = Request::get("/user")
            .and_then(|request| request.header(name, "secret"))
            .expect_err("refused");
        assert_eq!(*error.kind(), ErrorKind::Permanent);
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn the_marker_fixes_the_effect() {
        assert_eq!(Get::EFFECT, Effect::Read);
        assert_eq!(Head::EFFECT, Effect::Read);
        assert_eq!(Options::EFFECT, Effect::Read);
        assert_eq!(Put::EFFECT, Effect::Idempotent);
        assert_eq!(Delete::EFFECT, Effect::Idempotent);
        assert_eq!(Post::EFFECT, Effect::Write);
        assert_eq!(Patch::EFFECT, Effect::Write);
        assert_eq!(<Keyed<Post>>::EFFECT, Effect::Idempotent);
        assert_eq!(<Keyed<Patch>>::EFFECT, Effect::Idempotent);
        assert_eq!(<AsWrite<Put>>::EFFECT, Effect::Write);
        assert_eq!(<AsWrite<Delete>>::EFFECT, Effect::Write);
        assert_eq!(<Keyed<Post>>::METHOD, "POST");
        assert_eq!(<AsWrite<Delete>>::METHOD, "DELETE");
    }

    #[test]
    fn the_marker_fixes_the_operation_key() {
        let keys = [
            Get::OPERATION_KEY,
            Head::OPERATION_KEY,
            Options::OPERATION_KEY,
            Put::OPERATION_KEY,
            Delete::OPERATION_KEY,
            Post::OPERATION_KEY,
            Patch::OPERATION_KEY,
            <Keyed<Post>>::OPERATION_KEY,
            <Keyed<Patch>>::OPERATION_KEY,
            <AsWrite<Put>>::OPERATION_KEY,
            <AsWrite<Delete>>::OPERATION_KEY,
        ];
        assert_eq!(
            keys,
            [
                "http.get",
                "http.head",
                "http.options",
                "http.put",
                "http.delete",
                "http.post",
                "http.patch",
                "http.post.keyed",
                "http.patch.keyed",
                "http.put.as_write",
                "http.delete.as_write",
            ]
        );
        const {
            assert!(<Keyed<Post>>::KEYED && <Keyed<Patch>>::KEYED);
            assert!(!Post::KEYED && !<AsWrite<Put>>::KEYED);
        };
    }

    #[test]
    fn a_keyed_request_carries_its_key_part_not_a_header() {
        let keyed = Request::post("/charges")
            .expect("path")
            .idempotency_key("key-1");
        assert_eq!(keyed.key_part(), Some("key-1"));
        assert!(keyed.headers.get(IDEMPOTENCY_KEY).is_none());
        assert_eq!(Request::post("/charges").expect("path").key_part(), None);
    }

    #[test]
    fn a_request_serializes_its_intent_and_round_trips() {
        let request = Request::post("/charges")
            .expect("path")
            .query(&[("dry", "1")])
            .header("x-trace", "t-1")
            .expect("header")
            .header("x-trace", "t-2")
            .expect("repeated header")
            .body(&b"\x00\xff"[..])
            .accept_status(StatusCode::CONFLICT)
            .max_bytes(64)
            .cost(Cost::FREE)
            .max_attempts(NonZeroU32::new(3).expect("three"))
            .idempotency_key("charge-1");
        let json = serde_json::to_value(&request).expect("serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "path": "/charges",
                "query": [["dry", "1"]],
                "headers": [["x-trace", "t-2"]],
                "body": "AP8=",
                "accept": [409],
                "max_bytes": 64,
                "idempotency_key": "charge-1",
            }),
            "policy (cost, attempts) is not intent"
        );
        let back: Request<Keyed<Post>> = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back.path, "/charges");
        assert_eq!(back.body.as_deref(), Some(&b"\x00\xff"[..]));
        assert_eq!(back.key_part(), Some("charge-1"));
        assert!(back.accepts(StatusCode::CONFLICT));
        assert_eq!(back.cost_value(), &Cost::ONE, "the default policy");
        assert_eq!(back.attempts(), NonZeroU32::MIN);
    }

    #[test]
    fn a_deserialized_request_keeps_the_path_and_credential_rules() {
        let wire = |path: &str, header: &str| {
            serde_json::json!({
                "path": path,
                "query": [],
                "headers": [[header, "v"]],
                "accept": [],
                "max_bytes": null,
            })
        };
        assert!(serde_json::from_value::<Request<Get>>(wire("/ok", "x-a")).is_ok());
        assert!(serde_json::from_value::<Request<Get>>(wire("//evil", "x-a")).is_err());
        assert!(serde_json::from_value::<Request<Get>>(wire("/ok", "authorization")).is_err());
    }

    #[test]
    fn debug_shows_the_method_and_header_names_only() {
        let request = Request::put("/users/alice")
            .expect("path")
            .query(&[("token", "q-secret")])
            .header("x-trace", "trace-secret")
            .expect("header")
            .body("body-secret");
        let text = format!("{request:?}");
        assert!(text.contains("PUT") && text.contains("x-trace"), "{text}");
        for secret in ["alice", "q-secret", "trace-secret", "body-secret"] {
            assert!(!text.contains(secret), "{text}");
        }
    }

    #[rstest]
    #[case::root("https://api.example.com", "/user", "https://api.example.com/user")]
    #[case::mount(
        "https://api.example.com/v3",
        "/user",
        "https://api.example.com/v3/user"
    )]
    #[case::mount_slash(
        "https://api.example.com/v3/",
        "/user",
        "https://api.example.com/v3/user"
    )]
    fn the_path_is_appended_under_the_mount_prefix(
        #[case] base: &str,
        #[case] path: &str,
        #[case] expected: &str,
    ) {
        let transport =
            HttpTransport::new(&super::super::HttpConfig::new(base)).expect("transport");
        let outgoing = Request::get(path)
            .expect("path")
            .query(&[("q", "a b")])
            .outgoing(&transport)
            .expect("outgoing");
        assert_eq!(outgoing.url().as_str(), format!("{expected}?q=a+b"));
        assert!(outgoing.headers().get(AUTHORIZATION).is_none());
    }
}
