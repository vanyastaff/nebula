//! Requests as operation intent: the method marker that fixes the effect,
//! and the validated path, query, headers and body.

use std::{fmt, marker::PhantomData, num::NonZeroU32};

use bytes::Bytes;
use http::{
    HeaderMap, HeaderName, HeaderValue, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, PROXY_AUTHORIZATION},
};
use nebula_resource::{
    ErrorKind,
    call::{Cost, Effect, OpError},
};
use reqwest::Url;
use serde::Serialize;

use super::config::HttpTransport;

pub(super) mod sealed {
    /// Seals [`Method`](super::Method): only the markers of this module
    /// implement it.
    pub trait Sealed {
        /// The request method.
        fn http_method() -> http::Method;
    }
}

/// An HTTP method marker. It fixes the method and the [`Effect`] a
/// [`Request`] declares, so the retry safety of a request is a property of
/// its type.
///
/// | Marker | Method | Effect |
/// |---|---|---|
/// | [`Get`], [`Head`], [`Options`] | `GET`, `HEAD`, `OPTIONS` | `Read` |
/// | [`Put`], [`Delete`] | `PUT`, `DELETE` | `Idempotent` |
/// | [`Post`], [`Patch`] | `POST`, `PATCH` | `Write` |
/// | [`Keyed<Post>`](Keyed), [`Keyed<Patch>`](Keyed) | with an `Idempotency-Key` | `Idempotent` |
/// | [`AsWrite<Put>`](AsWrite), [`AsWrite<Delete>`](AsWrite) | a non-idempotent `PUT` / `DELETE` | `Write` |
///
/// `TRACE` and `CONNECT` have no marker.
pub trait Method: sealed::Sealed + Send + 'static {
    /// The method's name.
    const METHOD: &'static str;
    /// What repeating the request does to the provider.
    const EFFECT: Effect;
}

macro_rules! method_marker {
    ($(#[$doc:meta])* $marker:ident, $method:ident, $effect:ident) => {
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
        }
    };
}

method_marker!(
    /// `GET`: reads.
    Get, GET, Read
);
method_marker!(
    /// `HEAD`: reads.
    Head, HEAD, Read
);
method_marker!(
    /// `OPTIONS`: reads.
    Options, OPTIONS, Read
);
method_marker!(
    /// `PUT`: idempotent by HTTP semantics; [`Request::as_write`] declares a
    /// provider that breaks them.
    Put, PUT, Idempotent
);
method_marker!(
    /// `DELETE`: idempotent by HTTP semantics; [`Request::as_write`]
    /// declares a provider that breaks them.
    Delete, DELETE, Idempotent
);
method_marker!(
    /// `POST`: a write; [`Request::idempotency_key`] makes it idempotent.
    Post, POST, Write
);
method_marker!(
    /// `PATCH`: a write; [`Request::idempotency_key`] makes it idempotent.
    Patch, PATCH, Write
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
    ($wrapper:ident < $inner:ident >, $effect:ident) => {
        impl sealed::Sealed for $wrapper<$inner> {
            fn http_method() -> http::Method {
                <$inner as sealed::Sealed>::http_method()
            }
        }

        impl Method for $wrapper<$inner> {
            const METHOD: &'static str = <$inner as Method>::METHOD;
            const EFFECT: Effect = Effect::$effect;
        }
    };
}

wrapped_marker!(Keyed<Post>, Idempotent);
wrapped_marker!(Keyed<Patch>, Idempotent);
wrapped_marker!(AsWrite<Put>, Write);
wrapped_marker!(AsWrite<Delete>, Write);

/// Headers a request may not set: credentials are applied from the unit's
/// pinned slots by [`HttpApi::authorize`](super::HttpApi::authorize) only.
const FORBIDDEN_HEADERS: [HeaderName; 3] = [AUTHORIZATION, PROXY_AUTHORIZATION, COOKIE];

const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");

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
/// ```
/// # fn build() -> Result<(), nebula_sdk::integration::resource::OpError> {
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
    /// A defect found by a builder that cannot fail (an invalid
    /// idempotency key): the request is refused before any attempt.
    defect: Option<&'static str>,
    method: PhantomData<fn() -> M>,
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

fn invalid(detail: &'static str) -> OpError {
    OpError::new(ErrorKind::Permanent, detail)
}

impl<M: Method> Request<M> {
    fn new(path: &str) -> Result<Self, OpError> {
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
            defect: None,
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
    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Result<Self, OpError> {
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
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Result<Self, OpError> {
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

    /// The defect a builder recorded, refused before any attempt.
    pub(super) fn check(&self) -> Result<(), OpError> {
        self.defect.map_or(Ok(()), |detail| Err(invalid(detail)))
    }

    /// The outgoing request, without credentials: its URL is the base plus
    /// this path, and must stay under the base's mount prefix.
    pub(super) fn outgoing(&self, transport: &HttpTransport) -> Result<reqwest::Request, OpError> {
        self.check()?;
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
            defect: self.defect,
            method: PhantomData,
        }
    }

    fn with_idempotency_key(mut self, key: String) -> Self {
        match HeaderValue::try_from(key) {
            Ok(value) if !value.is_empty() => {
                self.headers.insert(IDEMPOTENCY_KEY, value);
            },
            _ => self.defect = Some("invalid idempotency key"),
        }
        self
    }
}

macro_rules! constructor {
    ($marker:ident, $name:ident, $doc:literal) => {
        impl Request<$marker> {
            #[doc = $doc]
            ///
            /// # Errors
            ///
            /// A permanent error when `path` breaks the path rules of
            /// [`Request`].
            pub fn $name(path: &str) -> Result<Self, OpError> {
                Self::new(path)
            }
        }
    };
}

constructor!(Get, get, "A `GET` of `path`.");
constructor!(Head, head, "A `HEAD` of `path`.");
constructor!(Options, options, "An `OPTIONS` of `path`.");
constructor!(Put, put, "A `PUT` of `path`.");
constructor!(Delete, delete, "A `DELETE` of `path`.");
constructor!(Post, post, "A `POST` to `path`.");
constructor!(Patch, patch, "A `PATCH` of `path`.");

impl Request<Post> {
    /// Sends the `POST` with an `Idempotency-Key`: the provider absorbs a
    /// repeat, so the request becomes `Idempotent` and a unit with an
    /// unknown outcome may be retried with the same key. An empty or invalid
    /// key refuses the request before any attempt.
    #[must_use]
    pub fn idempotency_key(self, key: impl Into<String>) -> Request<Keyed<Post>> {
        self.with_idempotency_key(key.into()).retag()
    }
}

impl Request<Patch> {
    /// Sends the `PATCH` with an `Idempotency-Key`, as
    /// [`Request::<Post>::idempotency_key`].
    #[must_use]
    pub fn idempotency_key(self, key: impl Into<String>) -> Request<Keyed<Patch>> {
        self.with_idempotency_key(key.into()).retag()
    }
}

impl Request<Put> {
    /// Declares that this provider applies a repeated `PUT` again: the
    /// request becomes a `Write`, never retried after it may have been sent.
    #[must_use]
    pub fn as_write(self) -> Request<AsWrite<Put>> {
        self.retag()
    }
}

impl Request<Delete> {
    /// Declares that this provider applies a repeated `DELETE` again, as
    /// [`Request::<Put>::as_write`].
    #[must_use]
    pub fn as_write(self) -> Request<AsWrite<Delete>> {
        self.retag()
    }
}

fn validate_path(path: &str) -> Result<(), OpError> {
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

    use super::*;

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
    fn an_invalid_idempotency_key_refuses_the_request() {
        let keyed = Request::post("/charges").expect("path").idempotency_key("");
        assert_eq!(
            *keyed.check().expect_err("refused").kind(),
            ErrorKind::Permanent
        );
        let keyed = Request::post("/charges")
            .expect("path")
            .idempotency_key("key-1");
        keyed.check().expect("valid key");
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
